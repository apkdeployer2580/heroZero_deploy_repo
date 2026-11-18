use alloc::{boxed::Box, vec, vec::Vec};

use bytemuck::{Pod, Zeroable, cast_slice_mut};

use wipi_types::wipic::{WIPICFramebuffer, WIPICIndirectPtr, WIPICWord};

use wie_backend::canvas::{
    ArgbPixel, Canvas, Clip, Color, Image, ImageBufferCanvas, PixelType, Rgb8Pixel, Rgb565Pixel, TextAlignment, VecImageBuffer,
};
use wie_util::Result;

use crate::context::WIPICContext;

/// Where a row band's starting pixels come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BandInit {
    /// Read the band back from guest memory — what the framebuffer holds right now.
    Read,
    /// Start blank. Only legal when the caller provably overwrites every pixel of
    /// the band before the canvas drops; see [`FrameBuffer::canvas_rows_uninit`].
    Blank,
}

pub struct FrameBuffer(pub WIPICFramebuffer);

impl FrameBuffer {
    pub fn empty() -> Self {
        Self(WIPICFramebuffer {
            width: 0,
            height: 0,
            bpl: 0,
            bpp: 0,
            buf: WIPICIndirectPtr(0),
        })
    }

    pub fn new(context: &mut dyn WIPICContext, width: WIPICWord, height: WIPICWord, bpp: WIPICWord) -> Result<Self> {
        let bytes_per_pixel = bpp / 8;

        let buf = context.alloc(width * height * bytes_per_pixel)?;

        Ok(Self(WIPICFramebuffer {
            width,
            height,
            bpl: width * bytes_per_pixel,
            bpp: bytes_per_pixel * 8,
            buf,
        }))
    }

    pub fn from_image(context: &mut dyn WIPICContext, image: &dyn Image) -> Result<Self> {
        let buf = context.alloc(image.width() * image.height() * image.bytes_per_pixel())?;

        context.write_bytes(context.data_ptr(buf)?, &image.raw())?;

        Ok(Self(WIPICFramebuffer {
            width: image.width(),
            height: image.height(),
            bpl: image.width() * image.bytes_per_pixel(),
            bpp: image.bytes_per_pixel() * 8,
            buf,
        }))
    }

    /// Bytes per line, derived the same way `new`/`from_image` lay the buffer out.
    fn line_bytes(&self) -> u32 {
        self.0.width * self.0.bpp / 8
    }

    /// Clamp a requested row range `[y, y + h)` to the framebuffer: returns
    /// `(first_row, row_count)`.
    fn clamp_rows(&self, y: i32, h: i32) -> (u32, u32) {
        let y0 = y.clamp(0, self.0.height as i32) as u32;
        let y1 = (y as i64 + h.max(0) as i64).clamp(y0 as i64, self.0.height as i64) as u32;
        (y0, y1 - y0)
    }

    /// The whole framebuffer's bytes. Production code goes through the row-banded
    /// paths below; kept (with `write`) as the equivalence tests' ground truth.
    #[allow(dead_code)]
    pub fn data(&self, context: &dyn WIPICContext) -> Result<Vec<u8>> {
        self.data_rows(context, 0, self.0.height)
    }

    /// Read only rows `[y0, y0 + rows)` — ONE contiguous guest-memory read.
    ///
    /// ★ This is the core of the heavy-map lag fix (wie_android, 2026-08-06): every
    /// draw call used to round-trip the WHOLE framebuffer through guest memory
    /// (~240 KB read + ~240 KB write for a 496x240 screen), so a frame's cost
    /// scaled with the CALL COUNT x the FULL framebuffer size instead of with the
    /// pixels actually drawn. Sprite-dense maps and full-screen effects multiplied
    /// hundreds of such round-trips per frame.
    fn data_rows(&self, context: &dyn WIPICContext, y0: u32, rows: u32) -> Result<Vec<u8>> {
        let size = self.line_bytes() * rows;
        let mut buf = vec![0; size as usize];
        if size > 0 {
            context.read_bytes(context.data_ptr(self.0.buf)? + y0 * self.line_bytes(), &mut buf)?;
        }

        Ok(buf)
    }

    /// Read rows `[y0, y0 + rows)` straight into a PIXEL vector — one allocation,
    /// one copy.
    ///
    /// ★ `image_rows`/`canvas_rows` used to call `data_rows` (allocate a `Vec<u8>`,
    /// read the band into it) and then `pod_collect_to_vec` (allocate a `Vec<u16>`
    /// and copy the very same bytes AGAIN). At 496x240x16bpp that redundant copy was
    /// ~238 KB per band, paid on every single draw call, for nothing: guest and host
    /// are both little-endian and `VecImageBuffer::raw` hands the pixel vector back
    /// out as bytes unchanged, so the pixel vector's bytes ARE the guest bytes.
    ///
    /// Reading into `cast_slice_mut` of an already-typed vector is what makes that
    /// safe without alignment guesswork: a `Vec<P>` is aligned for `P` by
    /// construction and casting DOWN to `u8` can never fail — unlike `cast_vec`,
    /// which would have to reinterpret a byte vector (alignment 1) as `Vec<u16>` and
    /// therefore cannot be used here at all.
    fn pixel_rows<P>(&self, context: &dyn WIPICContext, y0: u32, rows: u32, init: BandInit) -> Result<Vec<P>>
    where
        P: Pod + Zeroable,
    {
        // Callers pick `P` from `self.0.bpp`, so `size_of::<P>() == bpp / 8` and the
        // byte length below equals the `line_bytes() * rows` the old path read.
        let count = self.0.width as usize * rows as usize;
        let mut pixels = vec![P::zeroed(); count];
        if count > 0 && init == BandInit::Read {
            let bytes: &mut [u8] = cast_slice_mut(&mut pixels);
            context.read_bytes(context.data_ptr(self.0.buf)? + y0 * self.line_bytes(), bytes)?;
        }

        Ok(pixels)
    }

    pub fn image(&self, context: &mut dyn WIPICContext) -> Result<Box<dyn Image>> {
        Ok(self.image_rows(context, 0, self.0.height as i32)?.0)
    }

    /// The framebuffer's rows covering `[y, y + h)` as an image, plus the row the
    /// image starts at (subtract it from source-y coordinates). Rows outside the
    /// framebuffer are clipped away.
    pub fn image_rows(&self, context: &mut dyn WIPICContext, y: i32, h: i32) -> Result<(Box<dyn Image>, i32)> {
        let (y0, rows) = self.clamp_rows(y, h);

        let image: Box<dyn Image> = match self.0.bpp {
            16 => Box::new(VecImageBuffer::<Rgb565Pixel>::from_raw(
                self.0.width as _,
                rows as _,
                self.pixel_rows::<u16>(context, y0, rows, BandInit::Read)?,
            )),
            32 => Box::new(VecImageBuffer::<ArgbPixel>::from_raw(
                self.0.width as _,
                rows as _,
                self.pixel_rows::<u32>(context, y0, rows, BandInit::Read)?,
            )),
            _ => unimplemented!("Unsupported pixel format: {}", self.0.bpp),
        };
        Ok((image, y0 as i32))
    }

    /// A canvas over the WHOLE framebuffer. Prefer [`canvas_rows`] wherever the
    /// touched row range is known — this one reads and writes back every row.
    pub fn canvas<'a>(&'a self, context: &'a mut dyn WIPICContext) -> Result<FramebufferCanvas<'a>> {
        self.canvas_rows(context, 0, self.0.height as i32)
    }

    /// A canvas that materialises ONLY rows `[y, y + h)` of the framebuffer and
    /// writes only them back. Callers keep using ABSOLUTE coordinates — the canvas
    /// translates internally, so a band canvas draws exactly what a full canvas
    /// would, as long as the band covers the op's touched rows.
    pub fn canvas_rows<'a>(&'a self, context: &'a mut dyn WIPICContext, y: i32, h: i32) -> Result<FramebufferCanvas<'a>> {
        self.canvas_rows_with(context, y, h, BandInit::Read)
    }

    /// [`canvas_rows`] that does NOT read the band back first.
    ///
    /// ★ The band is written back WHOLE on drop, so every pixel the caller does not
    /// draw is written as zero. This is therefore only legal when the caller
    /// provably covers the entire band — see the guard in `graphics::fill_rect`,
    /// the one op that does. Measured on the real game: the field draws a
    /// full-screen `MC_grpFillRect` every frame, so the skipped read alone was
    /// 238 KB of guest memory pulled in and thrown away 25 times a second.
    pub fn canvas_rows_uninit<'a>(&'a self, context: &'a mut dyn WIPICContext, y: i32, h: i32) -> Result<FramebufferCanvas<'a>> {
        self.canvas_rows_with(context, y, h, BandInit::Blank)
    }

    fn canvas_rows_with<'a>(&'a self, context: &'a mut dyn WIPICContext, y: i32, h: i32, init: BandInit) -> Result<FramebufferCanvas<'a>> {
        let (y0, rows) = self.clamp_rows(y, h);

        let canvas: Box<dyn Canvas> = match self.0.bpp {
            16 => Box::new(ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::from_raw(
                self.0.width as _,
                rows as _,
                self.pixel_rows::<u16>(context, y0, rows, init)?,
            ))),
            32 => Box::new(ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::from_raw(
                self.0.width as _,
                rows as _,
                self.pixel_rows::<u32>(context, y0, rows, init)?,
            ))),
            _ => unimplemented!("Unsupported pixel format: {}", self.0.bpp),
        };

        Ok(FramebufferCanvas {
            framebuffer: self,
            context,
            canvas,
            band_y0: y0 as i32,
            band_rows: rows,
        })
    }

    /// True when a `fill_rect(x, .., w, ..)` provably repaints every pixel of the
    /// band it is given, so the band does not have to be read back first.
    ///
    /// `graphics::fill_rect` passes the rect ITSELF as the clip, and the band is
    /// exactly `clamp_rows(y, h)`, so vertical coverage is automatic and only the
    /// horizontal span has to be checked. Deliberately conservative: `i64` maths so
    /// no add can wrap, and anything that does not span the full framebuffer width
    /// falls back to the reading path.
    pub fn fill_covers_band(&self, x: i32, w: i32) -> bool {
        w > 0 && x <= 0 && (x as i64 + w as i64) >= self.0.width as i64
    }

    /// See [`data`]: retained for the tests' seeding; production writes are banded.
    #[allow(dead_code)]
    pub fn write(&self, context: &mut dyn WIPICContext, data: &[u8]) -> Result<()> {
        context.write_bytes(context.data_ptr(self.0.buf)?, data)
    }

    fn write_rows(&self, context: &mut dyn WIPICContext, y0: u32, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        context.write_bytes(context.data_ptr(self.0.buf)? + y0 * self.line_bytes(), data)
    }

    /// Fill `[x, x+w) x [y, y+h)` by writing ONLY those pixels into guest memory.
    /// `Ok(false)` = this path does not apply (not 16bpp) and the caller must fall
    /// back to the canvas.
    ///
    /// ── WHY (2026-08-13, `MIGRATION_LGT.md` §2.24) ───────────────────────────────
    /// The banded canvas is still a whole-band round trip: read `width x h` pixels out
    /// of guest memory, draw, write `width x h` back — plus one heap allocation. That
    /// is the right shape for `drawImage`, which needs the pixels under it, but a
    /// FILL needs nothing under it. And Hero3's panel screens (SLOT, 네트워크 창고, and
    /// every server message box) are drawn as a dither of small rects: **1,650
    /// `MC_grpFillRect` calls per frame**, device-measured. At 496x240x16bpp each one
    /// was ~1 KB read + ~1 KB written + ~1 KB zeroed for the ~16 bytes it actually
    /// changed, which is why those screens ran at 5 fps on the S9 while the field ran
    /// fine.
    ///
    /// Byte-for-byte equivalent to the canvas path, deliberately including its edge
    /// cases: the clamp arithmetic below mirrors `ImageBufferCanvas::fill_rect`
    /// (which receives the rect as its own clip) and `clamp_rows` — hence `w as u32`
    /// but `h.max(0)`, which are NOT symmetric. The colour is written straight
    /// through because for 16bpp `Rgb565Pixel::from_color(to_color(p)) == p` for all
    /// 65536 values (exhaustively checked), so the canvas' conversion round trip is
    /// the identity.
    pub fn fill_rect_direct(&self, context: &mut dyn WIPICContext, x: i32, y: i32, w: i32, h: i32, pixel: WIPICWord) -> Result<bool> {
        if self.0.bpp != 16 {
            return Ok(false);
        }
        let (width, height) = (self.0.width as i64, self.0.height as i64);
        let x0 = (x as i64).max(0).min(width);
        let y0 = (y as i64).max(0).min(height);
        let x1 = (x as i64 + (w as u32) as i64).min(width).max(x0);
        let y1 = (y as i64 + h.max(0) as i64).min(height).max(y0);
        if x0 >= x1 || y0 >= y1 {
            return Ok(true);
        }

        // One row of the fill colour, reused for every row. Sized for the widest
        // framebuffer this game has (496 px = 992 B) without touching the heap.
        const STACK_BYTES: usize = 2048;
        let run_bytes = (x1 - x0) as usize * 2;
        let mut stack = [0u8; STACK_BYTES];
        let mut heap: Vec<u8>;
        let row: &mut [u8] = if run_bytes <= STACK_BYTES {
            &mut stack[..run_bytes]
        } else {
            heap = vec![0u8; run_bytes];
            &mut heap
        };
        let le = (pixel as u16).to_le_bytes();
        for cell in row.chunks_exact_mut(2) {
            cell.copy_from_slice(&le);
        }

        let base = context.data_ptr(self.0.buf)?;
        let line = self.line_bytes();
        for row_y in y0..y1 {
            context.write_bytes(base + row_y as u32 * line + x0 as u32 * 2, row)?;
        }

        Ok(true)
    }

    pub fn pixel_to_color(&self, pixel: WIPICWord) -> Color {
        match self.0.bpp {
            16 => Rgb565Pixel::to_color(pixel as u16),
            _ => Rgb8Pixel::to_color(pixel),
        }
    }
}

/// A view over a horizontal band of a [`FrameBuffer`], written back on drop.
///
/// Implements [`Canvas`] in the FRAMEBUFFER's coordinate space: every y (and the
/// clip) is translated into the band before delegating, so call sites are
/// identical whether the canvas covers the whole framebuffer or one row.
pub struct FramebufferCanvas<'a> {
    framebuffer: &'a FrameBuffer,
    context: &'a mut dyn WIPICContext,
    canvas: Box<dyn Canvas>,
    band_y0: i32,
    band_rows: u32,
}

impl FramebufferCanvas<'_> {
    fn translate_clip(&self, clip: Clip) -> Clip {
        Clip {
            x: clip.x,
            y: clip.y - self.band_y0,
            width: clip.width,
            height: clip.height,
        }
    }
}

impl Drop for FramebufferCanvas<'_> {
    fn drop(&mut self) {
        if self.band_rows > 0 {
            self.framebuffer
                .write_rows(self.context, self.band_y0 as u32, &self.canvas.image().raw())
                .unwrap()
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl Canvas for FramebufferCanvas<'_> {
    fn image(&self) -> &dyn Image {
        // The band's image. Only meaningful to callers that made a full canvas.
        self.canvas.image()
    }

    fn set_xor_mode(&mut self, xor_mode: bool) {
        self.canvas.set_xor_mode(xor_mode)
    }

    fn copy_area(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, w: u32, h: u32, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.copy_area(dx, dy - self.band_y0, sx, sy - self.band_y0, w, h, clip)
    }

    fn draw(&mut self, dx: i32, dy: i32, w: u32, h: u32, src: &dyn Image, sx: i32, sy: i32, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.draw(dx, dy - self.band_y0, w, h, src, sx, sy, clip)
    }

    fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color) {
        self.canvas.draw_line(x1, y1 - self.band_y0, x2, y2 - self.band_y0, color)
    }

    fn draw_text(&mut self, string: &str, x: i32, y: i32, text_alignment: TextAlignment, color: Color) {
        self.canvas.draw_text(string, x, y - self.band_y0, text_alignment, color)
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.draw_rect(x, y - self.band_y0, w, h, color, clip)
    }

    fn draw_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.draw_arc(x, y - self.band_y0, w, h, start_angle, arc_angle, color, clip)
    }

    fn draw_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.draw_round_rect(x, y - self.band_y0, w, h, arc_width, arc_height, color, clip)
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.fill_rect(x, y - self.band_y0, w, h, color, clip)
    }

    fn fill_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.fill_arc(x, y - self.band_y0, w, h, start_angle, arc_angle, color, clip)
    }

    fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        let clip = self.translate_clip(clip);
        self.canvas.fill_round_rect(x, y - self.band_y0, w, h, arc_width, arc_height, color, clip)
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.canvas.put_pixel(x, y - self.band_y0, color)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use wie_backend::canvas::{Canvas, Clip, Color};

    use crate::context::test::TestContext;

    use super::FrameBuffer;

    const W: u32 = 32;
    const H: u32 = 24;
    const RED: Color = Color { a: 255, r: 255, g: 0, b: 0 };

    /// A framebuffer pre-seeded with a deterministic 16bpp pattern.
    fn seeded_fb(context: &mut TestContext) -> FrameBuffer {
        let fb = FrameBuffer::new(context, W, H, 16).unwrap();
        let pattern: Vec<u8> = (0..W * H * 2).map(|i| (i * 37 % 251) as u8).collect();
        fb.write(context, &pattern).unwrap();
        fb
    }

    fn clip(x: i32, y: i32, w: u32, h: u32) -> Clip {
        Clip { x, y, width: w, height: h }
    }

    /// Every op done through a ROW-BAND canvas must leave the framebuffer byte-
    /// identical to the same op done through a FULL canvas — the band is an
    /// optimisation, not a semantic.
    #[test]
    fn band_canvas_matches_full_canvas() {
        type Op = fn(&mut dyn Canvas);
        // (band_y, band_h, op) — each op uses ABSOLUTE coordinates and stays
        // within the band it declares, which is the call sites' contract.
        let cases: &[(i32, i32, Op)] = &[
            (5, 1, |c| c.put_pixel(3, 5, RED)),
            (4, 6, |c| c.fill_rect(2, 4, 10, 6, RED, clip(2, 4, 10, 6))),
            (-3, 9, |c| c.fill_rect(-2, -3, 8, 9, RED, clip(-2, -3, 8, 9))),
            (10, 40, |c| c.fill_rect(20, 10, 40, 40, RED, clip(20, 10, 40, 40))),
            (2, 7, |c| c.draw_line(1, 8, 12, 2, RED)),
            (6, 8, |c| c.draw_rect(3, 6, 9, 8, RED, clip(3, 6, 9, 8))),
            (8, 10, |c| c.fill_arc(4, 8, 12, 10, 0, 360, RED, clip(4, 8, 12, 10))),
        ];

        for (i, (band_y, band_h, op)) in cases.iter().enumerate() {
            let mut ctx_full = TestContext::new();
            let fb_full = seeded_fb(&mut ctx_full);
            {
                let mut canvas = fb_full.canvas(&mut ctx_full).unwrap();
                op(&mut canvas);
            }
            let expected = fb_full.data(&ctx_full).unwrap();

            let mut ctx_band = TestContext::new();
            let fb_band = seeded_fb(&mut ctx_band);
            {
                let mut canvas = fb_band.canvas_rows(&mut ctx_band, *band_y, *band_h).unwrap();
                op(&mut canvas);
            }
            let actual = fb_band.data(&ctx_band).unwrap();

            assert_eq!(expected, actual, "case {i}: band canvas diverged from full canvas");
        }
    }

    /// The read-skipping band must be byte-identical to the reading band for every
    /// fill the guard admits. If this ever diverges, the guard is admitting a fill
    /// that does NOT cover its band and the skipped rows get written back as black.
    #[test]
    fn uninit_band_matches_read_band_for_covering_fills() {
        // (x, y, w, h) — all span the full framebuffer width, which is what
        // `fill_covers_band` requires. Includes a negative origin and an
        // over-long span, both of which the game emits.
        let cases: &[(i32, i32, i32, i32)] = &[
            (0, 0, W as i32, H as i32),
            (0, 5, W as i32, 4),
            (-6, 3, W as i32 + 12, 6),
            (0, -4, W as i32, 10),
            (0, H as i32 - 2, W as i32, 9),
        ];

        for (i, (x, y, w, h)) in cases.iter().enumerate() {
            let mut ctx_read = TestContext::new();
            let fb_read = seeded_fb(&mut ctx_read);
            assert!(fb_read.fill_covers_band(*x, *w), "case {i}: guard rejected a covering fill");
            {
                let mut canvas = fb_read.canvas_rows(&mut ctx_read, *y, *h).unwrap();
                canvas.fill_rect(*x, *y, *w as u32, *h as u32, RED, clip(*x, *y, *w as u32, *h as u32));
            }
            let expected = fb_read.data(&ctx_read).unwrap();

            let mut ctx_skip = TestContext::new();
            let fb_skip = seeded_fb(&mut ctx_skip);
            {
                let mut canvas = fb_skip.canvas_rows_uninit(&mut ctx_skip, *y, *h).unwrap();
                canvas.fill_rect(*x, *y, *w as u32, *h as u32, RED, clip(*x, *y, *w as u32, *h as u32));
            }
            let actual = fb_skip.data(&ctx_skip).unwrap();

            assert_eq!(expected, actual, "case {i}: read-skipping band diverged from the reading band");
        }
    }

    /// `fill_rect_direct` must leave the framebuffer byte-identical to the banded
    /// canvas for EVERY rect, including the degenerate ones — it is the only path
    /// `graphics::fill_rect` takes at 16bpp, so a divergence here is a wrong pixel on
    /// the device, not a slow one. The cases cover what the game emits (small dither
    /// rects, the full-screen clear) plus the clamp corners: negative origins, spans
    /// past both edges, zero and negative extents.
    #[test]
    fn direct_fill_matches_band_canvas() {
        // A colour whose 565 value is not symmetric, so a byte-order slip shows up.
        const PIXEL: u32 = 0xf81f;
        let cases: &[(i32, i32, i32, i32)] = &[
            (2, 4, 10, 6),
            (0, 0, W as i32, H as i32),
            (3, 7, 1, 1),
            (5, 9, 2, 1),
            (-4, -3, 8, 9),
            (W as i32 - 3, H as i32 - 2, 10, 10),
            (20, 10, 40, 40),
            (0, 5, W as i32 + 12, 4),
            (4, 6, 0, 5),
            (4, 6, 5, 0),
            (4, 6, -5, 5),
            (4, 6, 5, -5),
            (W as i32 + 4, 2, 5, 5),
            (2, H as i32 + 4, 5, 5),
        ];

        for (i, (x, y, w, h)) in cases.iter().enumerate() {
            let mut ctx_canvas = TestContext::new();
            let fb_canvas = seeded_fb(&mut ctx_canvas);
            {
                let color = fb_canvas.pixel_to_color(PIXEL);
                let mut canvas = fb_canvas.canvas_rows(&mut ctx_canvas, *y, *h).unwrap();
                canvas.fill_rect(*x, *y, *w as u32, *h as u32, color, clip(*x, *y, *w as u32, *h as u32));
            }
            let expected = fb_canvas.data(&ctx_canvas).unwrap();

            let mut ctx_direct = TestContext::new();
            let fb_direct = seeded_fb(&mut ctx_direct);
            assert!(
                fb_direct.fill_rect_direct(&mut ctx_direct, *x, *y, *w, *h, PIXEL).unwrap(),
                "case {i}: the direct path declined a 16bpp fill"
            );
            let actual = fb_direct.data(&ctx_direct).unwrap();

            assert_eq!(expected, actual, "case {i}: direct fill diverged from the band canvas");
        }
    }

    /// The guard must reject anything that leaves part of a row untouched — those
    /// fills MUST keep reading the band back.
    #[test]
    fn fill_covers_band_rejects_partial_spans() {
        let mut ctx = TestContext::new();
        let fb = seeded_fb(&mut ctx);

        assert!(!fb.fill_covers_band(1, W as i32), "starts inside the buffer");
        assert!(!fb.fill_covers_band(0, W as i32 - 1), "stops short of the right edge");
        assert!(!fb.fill_covers_band(-1, W as i32), "shifted left, so the right edge is uncovered");
        assert!(!fb.fill_covers_band(0, 0), "empty");
        assert!(!fb.fill_covers_band(0, -5), "negative width");
        assert!(!fb.fill_covers_band(i32::MIN, i32::MAX), "must not wrap into a false positive");
        assert!(fb.fill_covers_band(0, W as i32), "exact cover");
    }

    /// A blit through row-banded SOURCE + DESTINATION must equal the full-canvas
    /// blit, with the source-y adjusted by the returned band origin — the exact
    /// shape `draw_image` / `copy_frame_buffer` / `copy_area` use.
    #[test]
    fn banded_blit_matches_full_blit() {
        // (dx, dy, w, h, sx, sy)
        let cases: &[(i32, i32, i32, i32, i32, i32)] = &[(5, 6, 10, 8, 2, 3), (0, -2, 12, 9, -1, 4), (25, 18, 20, 20, 10, 10)];
        for (i, (dx, dy, w, h, sx, sy)) in cases.iter().enumerate() {
            let mut ctx_full = TestContext::new();
            let src_full = seeded_fb(&mut ctx_full);
            let dst_full = FrameBuffer::new(&mut ctx_full, W, H, 16).unwrap();
            {
                let src_image = src_full.image(&mut ctx_full).unwrap();
                let mut canvas = dst_full.canvas(&mut ctx_full).unwrap();
                canvas.draw(
                    *dx,
                    *dy,
                    *w as u32,
                    *h as u32,
                    &*src_image,
                    *sx,
                    *sy,
                    clip(*dx, *dy, *w as u32, *h as u32),
                );
            }
            let expected = dst_full.data(&ctx_full).unwrap();

            let mut ctx_band = TestContext::new();
            let src_band = seeded_fb(&mut ctx_band);
            let dst_band = FrameBuffer::new(&mut ctx_band, W, H, 16).unwrap();
            {
                let (src_image, src_y0) = src_band.image_rows(&mut ctx_band, *sy, *h).unwrap();
                let mut canvas = dst_band.canvas_rows(&mut ctx_band, *dy, *h).unwrap();
                canvas.draw(
                    *dx,
                    *dy,
                    *w as u32,
                    *h as u32,
                    &*src_image,
                    *sx,
                    *sy - src_y0,
                    clip(*dx, *dy, *w as u32, *h as u32),
                );
            }
            let actual = dst_band.data(&ctx_band).unwrap();

            assert_eq!(expected, actual, "case {i}: banded blit diverged from full blit");
        }
    }

    /// A band fully outside the framebuffer materialises zero rows, draws nothing
    /// and writes nothing back.
    #[test]
    fn out_of_range_band_is_a_noop() {
        let mut context = TestContext::new();
        let fb = seeded_fb(&mut context);
        let before = fb.data(&context).unwrap();
        {
            let mut canvas = fb.canvas_rows(&mut context, H as i32 + 5, 10).unwrap();
            canvas.fill_rect(0, H as i32 + 5, 8, 4, RED, clip(0, H as i32 + 5, 8, 4));
            canvas.put_pixel(1, H as i32 + 6, RED);
        }
        assert_eq!(before, fb.data(&context).unwrap());
    }
}
