mod fx;
mod lbmp;

pub use self::fx::{
    FX_ADD, FX_FLIP_X, FX_TAG, FX_X2, FxEffect, FxHd, FxLibrary, FxPart, FxSprite, HdDraw, HdFrame, HdView, compose_hd, fx_library, fx_lines,
    fx_scale, hd_finish, hd_mode, hd_record, hd_reset, hd_touch, set_fx_library, set_fx_lines, set_fx_scale, set_hd_mode,
};

use alloc::{borrow::Cow, boxed::Box, string::ToString, vec, vec::Vec};
use core::mem::size_of;

use ab_glyph::{Font, FontRef, ScaleFont};
use bytemuck::{Pod, cast_slice, pod_collect_to_vec};
use image::ImageReader;
use num_traits::{Num, Zero};

use wie_util::{Result, WieError};

use self::lbmp::decode_lbmp;

lazy_static::lazy_static! {
    static ref FONT: FontRef<'static> = FontRef::try_from_slice(include_bytes!("../../fonts/neodgm.ttf")).unwrap();
}

/// ★ A host draw alpha for image blits (2026-10-02): every `Canvas::draw` scales its source's
/// alpha by this (255 = off, the default — the paths below are then exactly what they were).
/// The WIPI `Graphics.setAlpha(I)V` is a stub in this tree (the game's own calls — a boot fade,
/// 0..255 — change nothing), so a caller that KNOWS this emulator asks for it with a tagged value,
/// `DRAW_ALPHA_TAG | alpha`: hero2's host block draws 일섬's afterimages with the game's own sprite
/// routine at a fraction of their alpha, then puts 255 back. Process-wide on purpose — the sprite
/// routine draws through whatever `Graphics` the game holds; the paint event resets it, so a
/// value can never outlive a frame.
static DRAW_ALPHA: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);

/// `setAlpha` arguments whose upper 24 bits are this are the host draw alpha (the low byte).
pub const DRAW_ALPHA_TAG: u32 = 0x4832_4100;

pub fn set_draw_alpha(alpha: u8) {
    DRAW_ALPHA.store(alpha, core::sync::atomic::Ordering::Relaxed);
}

pub fn draw_alpha() -> u8 {
    DRAW_ALPHA.load(core::sync::atomic::Ordering::Relaxed)
}

/// `color` with its alpha scaled by the host draw alpha (exact at 255).
fn with_draw_alpha(color: Color) -> Color {
    let a = draw_alpha();
    if a == 255 {
        return color;
    }
    Color {
        a: ((u16::from(color.a) * u16::from(a) + 127) / 255) as u8,
        ..color
    }
}

/// ★ A host STROKE for `draw_line` (2026-10-08 (3), hero2's awakening pictures — 개화's floor, 천공의검's tears). While it
/// is set, a line is a capsule with round caps whose half-width goes from `w0` at (x1, y1) to `w1` at (x2, y2) — a taper —
/// in quarter pixels, anti-aliased at the edge; SOFT, the coverage falls off linearly from the axis instead (a glow).
/// `setAlpha(STROKE_TAG | w0 << 8 | w1)` or `setAlpha(STROKE_SOFT_TAG | …)`; `STROKE_TAG | 0` puts the 1-px line back.
/// The game itself never draws a line (hero2's image has no reference word for `drawLine` — the host block brings its own),
/// and the paint event resets it like the draw alpha, so it can never outlive a frame.
static STROKE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// `setAlpha` arguments whose upper 16 bits are one of these set the host stroke (the low 16 bits: `w0 << 8 | w1`).
pub const STROKE_TAG: u32 = 0x4833_0000;
pub const STROKE_SOFT_TAG: u32 = 0x4834_0000;
const STROKE_SOFT: u32 = 1 << 16;

/// `widths` = `w0 << 8 | w1` in quarter pixels (0 = the plain 1-px line, whatever `soft` says).
pub fn set_stroke(widths: u16, soft: bool) {
    let v = if widths == 0 {
        0
    } else {
        u32::from(widths) | if soft { STROKE_SOFT } else { 0 }
    };
    STROKE.store(v, core::sync::atomic::Ordering::Relaxed);
}

fn stroke() -> u32 {
    STROKE.load(core::sync::atomic::Ordering::Relaxed)
}

pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub trait Image: Send {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn bytes_per_pixel(&self) -> u32;
    fn get_pixel(&self, x: i32, y: i32) -> Color;
    fn raw(&self) -> Cow<'_, [u8]>;
    fn colors(&self) -> Vec<Color>;

    /// Copy `px` pixels of RAW data starting at pixel `(x, y)` into `out`, which is
    /// `px * bytes_per_pixel()` long. `false` = unsupported or out of bounds, and the
    /// caller must use the per-pixel path.
    ///
    /// ★ WHY THIS EXISTS (2026-09-09). `raw()` copies the WHOLE image, which is the
    /// wrong shape for a blit: hero2's average `draw` is ~495 px out of a much larger
    /// sheet. And the per-pixel path is worse — for a guest-backed buffer every
    /// `get_pixel` is a `dyn` dispatch into `ArmCore::read_bytes`, i.e. a core-mutex
    /// lock plus a page-table walk PER PIXEL. Measured on hero2: ~24 K drawn px/frame,
    /// so ~48 K locked guest accesses per frame counting the destination writes.
    /// A row at a time turns that into a handful per row.
    fn raw_row(&self, x: i32, y: i32, px: usize, out: &mut [u8]) -> bool {
        let _ = (x, y, px, out);
        false
    }
}

pub trait ImageBuffer: Send {
    fn put_pixel(&mut self, x: i32, y: i32, color: Color);
    fn put_pixels(&mut self, x: i32, y: i32, width: u32, colors: &[Color]);
    fn xor_pixel(&mut self, x: i32, y: i32, color: Color);

    /// Fill the half-open rect [x0, x1) x [y0, y1) with a solid colour. The caller
    /// has ALREADY clamped the bounds to the buffer; the default falls back to
    /// per-pixel puts, concrete buffers can overwrite whole row spans at once.
    ///
    /// ★ Added for wie_android's heavy-map lag: `Canvas::fill_rect` used to go
    /// through `put_pixel` for every pixel (bounds check + colour conversion each),
    /// which made the game's full-screen fills (map transitions, the blackout-event
    /// darkening overlays) cost tens of millions of per-pixel calls per second.
    fn fill_span(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_pixel(x, y, color);
            }
        }
    }

    /// Copy one row of RAW pixel data (same pixel format as this buffer) to
    /// `(x, y)`. Returns `false` when the buffer cannot take raw rows (the caller
    /// then uses the per-pixel path). Bounds must already be clamped.
    fn put_row_raw(&mut self, _x: i32, _y: i32, _data: &[u8]) -> bool {
        false
    }

    /// Read `out.len() / bytes_per_pixel()` pixels of RAW data at `(x, y)`. The
    /// destination half of [`Image::raw_row`]: a partially transparent blit has to read
    /// the background, and doing that per pixel is what this avoids.
    fn get_row_raw(&self, x: i32, y: i32, out: &mut [u8]) -> bool {
        let _ = (x, y, out);
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub trait Canvas: Send {
    fn image(&self) -> &dyn Image;
    fn set_xor_mode(&mut self, xor_mode: bool);
    fn copy_area(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, w: u32, h: u32, clip: Clip);
    fn draw(&mut self, dx: i32, dy: i32, w: u32, h: u32, src: &dyn Image, sx: i32, sy: i32, clip: Clip);
    fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color);
    fn draw_text(&mut self, string: &str, x: i32, y: i32, text_alignment: TextAlignment, color: Color);
    fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip);
    fn draw_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip);
    fn draw_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip);
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip);
    fn fill_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip);
    fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip);
    fn put_pixel(&mut self, x: i32, y: i32, color: Color);
}

pub trait PixelType: Send {
    type DataType: Copy + Pod + Num + Send;
    fn from_color(color: Color) -> Self::DataType;
    fn to_color(raw: Self::DataType) -> Color;
    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType;
}

pub struct Rgb332Pixel;

impl PixelType for Rgb332Pixel {
    type DataType = u8;

    fn from_color(color: Color) -> Self::DataType {
        let r = ((color.r as u16 * 7 + 127) / 255) as u8;
        let g = ((color.g as u16 * 7 + 127) / 255) as u8;
        let b = ((color.b as u16 * 3 + 127) / 255) as u8;

        (r << 5) | (g << 2) | b
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = (raw >> 5) & 0x7;
        let g = (raw >> 2) & 0x7;
        let b = raw & 0x3;

        Color {
            a: 0xff,
            r: r * 36,
            g: g * 36,
            b: b * 85,
        }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct Rgb565Pixel;

impl PixelType for Rgb565Pixel {
    type DataType = u16;

    fn from_color(color: Color) -> Self::DataType {
        let r = (color.r as u16) >> 3;
        let g = (color.g as u16) >> 2;
        let b = (color.b as u16) >> 3;

        (r << 11) | (g << 5) | b
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = ((raw >> 11) & 0x1f) as u8;
        let g = ((raw >> 5) & 0x3f) as u8;
        let b = (raw & 0x1f) as u8;

        let r = ((r as u32 * 255 + 15) / 31) as u8;
        let g = ((g as u32 * 255 + 31) / 63) as u8;
        let b = ((b as u32 * 255 + 15) / 31) as u8;

        Color { a: 0xff, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct Rgb8Pixel;

impl PixelType for Rgb8Pixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let b = (raw & 0xff) as u8;

        Color { a: 0xff, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct ArgbPixel;

impl PixelType for ArgbPixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.a as u32) << 24) | ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let a = ((raw >> 24) & 0xff) as u8;
        let r = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let b = (raw & 0xff) as u8;

        Color { a, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        let source = ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32;

        (raw ^ source) | 0xff000000
    }
}

pub struct AbgrPixel;

impl PixelType for AbgrPixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.a as u32) << 24) | ((color.b as u32) << 16) | ((color.g as u32) << 8) | color.r as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let a = ((raw >> 24) & 0xff) as u8;
        let b = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let r = (raw & 0xff) as u8;

        Color { a, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        let source = ((color.b as u32) << 16) | ((color.g as u32) << 8) | color.r as u32;

        (raw ^ source) | 0xff000000
    }
}

pub struct VecImageBuffer<T>
where
    T: PixelType,
{
    width: u32,
    height: u32,
    data: Vec<T::DataType>,
}

impl<T> VecImageBuffer<T>
where
    T: PixelType,
{
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![T::DataType::zero(); (width * height) as usize],
        }
    }

    pub fn from_raw(width: u32, height: u32, raw: Vec<T::DataType>) -> Self {
        Self { width, height, data: raw }
    }
}

impl<T> Image for VecImageBuffer<T>
where
    T: PixelType + 'static,
{
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn bytes_per_pixel(&self) -> u32 {
        size_of::<T::DataType>() as u32
    }

    fn get_pixel(&self, x: i32, y: i32) -> Color {
        let raw = self.data[((y as u32) * self.width + (x as u32)) as usize];

        T::to_color(raw)
    }

    fn raw(&self) -> Cow<'_, [u8]> {
        cast_slice(&self.data).into()
    }

    fn colors(&self) -> Vec<Color> {
        self.data.iter().map(|&x| T::to_color(x)).collect()
    }

    /// A slice copy. Host-backed, so this is not about avoiding guest reads - it is so
    /// that a blit whose DESTINATION is guest-backed can still take the row path when
    /// the source happens to be a host buffer (a decoded PNG, say).
    fn raw_row(&self, x: i32, y: i32, px: usize, out: &mut [u8]) -> bool {
        let bpp = size_of::<T::DataType>();
        if x < 0 || y < 0 || out.len() != px * bpp {
            return false;
        }
        if (y as u32) >= self.height || (x as u32).saturating_add(px as u32) > self.width {
            return false;
        }
        let start = ((y as u32) * self.width + (x as u32)) as usize;
        out.copy_from_slice(bytemuck::cast_slice(&self.data[start..start + px]));
        true
    }
}

impl<T> ImageBuffer for VecImageBuffer<T>
where
    T: PixelType + 'static,
{
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }

        let raw = T::from_color(color);

        self.data[((y as u32) * self.width + (x as u32)) as usize] = raw;
    }

    fn put_pixels(&mut self, x: i32, y: i32, width: u32, colors: &[Color]) {
        for (i, color) in colors.iter().enumerate() {
            let x = x + (i as i32 % (width as i32));
            let y = y + (i as i32 / (width as i32));

            if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
                continue;
            }

            let raw = T::from_color(*color);

            self.data[((y as u32) * self.width + (x as u32)) as usize] = raw;
        }
    }

    fn xor_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }

        let offset = ((y as u32) * self.width + (x as u32)) as usize;
        self.data[offset] = T::xor_color(self.data[offset], color);
    }

    fn fill_span(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        // Bounds are pre-clamped by `Canvas::fill_rect`; guard anyway so a misuse
        // cannot panic on the slice index.
        let (w, h) = (self.width as i32, self.height as i32);
        let (x0, x1) = (x0.clamp(0, w), x1.clamp(0, w));
        let (y0, y1) = (y0.clamp(0, h), y1.clamp(0, h));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let raw = T::from_color(color);
        for y in y0..y1 {
            let start = (y as u32 * self.width + x0 as u32) as usize;
            self.data[start..start + (x1 - x0) as usize].fill(raw);
        }
    }

    fn put_row_raw(&mut self, x: i32, y: i32, data: &[u8]) -> bool {
        let pixel_size = size_of::<T::DataType>();
        let count = data.len() / pixel_size;
        if x < 0 || y < 0 || (y as u32) >= self.height || data.len() % pixel_size != 0 || (x as u32).saturating_add(count as u32) > self.width {
            // A mis-sized row is a caller bug; report "unsupported" so the caller's
            // per-pixel fallback (which clamps) runs instead of corrupting memory.
            return false;
        }
        let start = ((y as u32) * self.width + (x as u32)) as usize;
        let dst: &mut [u8] = bytemuck::cast_slice_mut(&mut self.data[start..start + count]);
        dst.copy_from_slice(data);
        true
    }

    fn get_row_raw(&self, x: i32, y: i32, out: &mut [u8]) -> bool {
        let pixel_size = size_of::<T::DataType>();
        let count = out.len() / pixel_size;
        if x < 0 || y < 0 || (y as u32) >= self.height || out.len() % pixel_size != 0 || (x as u32).saturating_add(count as u32) > self.width {
            return false;
        }
        let start = ((y as u32) * self.width + (x as u32)) as usize;
        let src: &[u8] = bytemuck::cast_slice(&self.data[start..start + count]);
        out.copy_from_slice(src);
        true
    }
}

pub struct ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    image_buffer: T,
    xor_mode: bool,
}

impl<T> ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    pub fn new(image_buffer: T) -> Self {
        Self {
            image_buffer,
            xor_mode: false,
        }
    }

    pub fn into_inner(self) -> T {
        self.image_buffer
    }

    fn blend_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.compose_pixel(x, y, color, true);
    }

    fn compose_pixel(&mut self, x: i32, y: i32, color: Color, blend: bool) {
        if x < 0 || y < 0 || (x as u32) >= self.image_buffer.width() || (y as u32) >= self.image_buffer.height() {
            return;
        }

        if self.xor_mode {
            if color.a == 0 {
                return;
            }

            let color = if blend && color.a < 255 {
                Color {
                    a: 255,
                    r: ((color.r as u16 * color.a as u16 + 127) / 255) as u8,
                    g: ((color.g as u16 * color.a as u16 + 127) / 255) as u8,
                    b: ((color.b as u16 * color.a as u16 + 127) / 255) as u8,
                }
            } else {
                color
            };
            self.image_buffer.xor_pixel(x, y, color);
            return;
        }

        if !blend {
            self.image_buffer.put_pixel(x, y, color);
            return;
        }

        let bg = self.image_buffer.get_pixel(x, y);
        let factor = color.a as f32 / 255.0;

        let computed_color = Color {
            a: 0xff,
            r: (color.r as f32 * factor + bg.r as f32 * (1.0 - factor)) as u8,
            g: (color.g as f32 * factor + bg.g as f32 * (1.0 - factor)) as u8,
            b: (color.b as f32 * factor + bg.b as f32 * (1.0 - factor)) as u8,
        };

        self.image_buffer.put_pixel(x, y, computed_color);
    }

    fn plot(&mut self, x: i32, y: i32, color: Color, clip: &Clip) {
        if x < 0 || y < 0 || (x as u32) >= self.image_buffer.width() || (y as u32) >= self.image_buffer.height() {
            return;
        }
        if x < clip.x || x >= clip.x + clip.width as i32 || y < clip.y || y >= clip.y + clip.height as i32 {
            return;
        }
        self.put_pixel(x, y, color);
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_arc(&mut self, cx: f32, cy: f32, a: f32, b: f32, start_deg: f32, sweep_deg: f32, color: Color, clip: &Clip) {
        let radius = a.max(b).max(1.0);
        let sweep_rad = sweep_deg.to_radians();
        let start_rad = start_deg.to_radians();
        let steps = ((sweep_rad.abs() * radius).ceil() as i32 * 2).max(1);

        for i in 0..=steps {
            let theta = start_rad + sweep_rad * (i as f32) / (steps as f32);
            let px = (cx + a * theta.cos()).round() as i32;
            let py = (cy - b * theta.sin()).round() as i32;
            self.plot(px, py, color, clip);
        }
    }

    /// The host stroke (`STROKE`): every pixel whose centre lies within the half-width of the segment — the half-width
    /// interpolated along it, round caps — blended at `color.a` × its coverage. Hard: `clamp(hw + ½ − d, 0, 1)` (an
    /// anti-aliased edge, a tip of width 0 still shows its axis at half); soft: `max(0, 1 − d / (hw + ½))`. Row by row,
    /// each row only where the axis's slab can reach it.
    fn draw_stroke(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color, stroke: u32) {
        if color.a == 0 {
            return;
        }
        let soft = stroke & STROKE_SOFT != 0;
        let w0 = ((stroke >> 8) & 0xff) as f32 / 4.0;
        let w1 = (stroke & 0xff) as f32 / 4.0;
        let (ax, ay) = (x1 as f32 + 0.5, y1 as f32 + 0.5);
        let (dx, dy) = ((x2 - x1) as f32, (y2 - y1) as f32);
        let len2 = dx * dx + dy * dy;
        let reach = w0.max(w1) + 1.0;
        let (width, height) = (self.image_buffer.width() as i32, self.image_buffer.height() as i32);
        let x_lo = ((ax.min(ax + dx) - reach) as i32 - 1).max(0);
        let x_hi = ((ax.max(ax + dx) + reach) as i32 + 1).min(width - 1);
        let y_lo = ((ay.min(ay + dy) - reach) as i32 - 1).max(0);
        let y_hi = ((ay.max(ay + dy) + reach) as i32 + 1).min(height - 1);
        if x_lo > x_hi || y_lo > y_hi {
            return;
        }
        let len = if len2 > 0.0 { len2.sqrt() } else { 0.0 };
        for y in y_lo..=y_hi {
            let py = y as f32 + 0.5;
            let (mut xa, mut xb) = (x_lo, x_hi);
            if dy.abs() > 0.01 {
                let xc = ax + dx * (py - ay) / dy;
                let half = reach * len / dy.abs() + 1.0;
                xa = xa.max((xc - half) as i32 - 1);
                xb = xb.min((xc + half) as i32 + 1);
            }
            for x in xa..=xb {
                let px = x as f32 + 0.5;
                let t = if len2 > 0.0 {
                    (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (qx, qy) = (ax + dx * t - px, ay + dy * t - py);
                let d = (qx * qx + qy * qy).sqrt();
                let hw = w0 + (w1 - w0) * t;
                let cover = if soft { 1.0 - d / (hw + 0.5) } else { (hw + 0.5 - d).min(1.0) };
                if cover <= 0.0 {
                    continue;
                }
                let a = (color.a as f32 * cover + 0.5) as u8;
                if a != 0 {
                    self.blend_pixel(x, y, Color { a, ..color });
                }
            }
        }
    }
}

impl<T> ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    /// The effect library (`fx`): every part of frame `age` of `effect` — its sprite's anchor on `(x + dx, y + dy)`, the
    /// part's alpha × the host draw alpha, mirrored about the anchor column / added instead of blended / twice the size
    /// about the anchor by its flags —
    /// clipped to the buffer. A 4-byte buffer goes a row at a time through the raw row calls (one read, one write a row);
    /// anything else pixel by pixel.
    fn draw_fx(&mut self, x: i32, y: i32, effect: i32, age: i32) {
        if let Some(lib) = fx_library() {
            self.draw_fx_scaled(&lib, x, y, effect, age, fx::fx_scale().max(1));
        }
    }

    #[cfg(test)]
    fn draw_fx_from(&mut self, lib: &FxLibrary, x: i32, y: i32, effect: i32, age: i32) {
        self.draw_fx_scaled(lib, x, y, effect, age, 1);
    }

    /// ★★ 2026-10-18 `scale` 2 = every part's offset and sprite doubled about the anchor (`FX_TAG | 2`).
    fn draw_fx_scaled(&mut self, lib: &FxLibrary, x: i32, y: i32, effect: i32, age: i32, scale: u8) {
        let Some(frame) = lib.frame(effect, age) else {
            return;
        };
        let ga = u32::from(draw_alpha());
        if ga == 0 || self.xor_mode {
            return;
        }
        // ★★ HD mode (2026-10-15): the draw is RECORDED with the framebuffer's layer map and composited by the host at the
        //   window's resolution (`fx::compose_hd`) — the guest framebuffer gets nothing of it.
        if hd_mode() {
            let (w, h, bpp) = (self.image_buffer.width(), self.image_buffer.height(), self.image_buffer.bytes_per_pixel());
            let raw = self.image_buffer.raw();
            fx::hd_record(&raw, w, h, bpp, effect, age, x, y, scale, ga as u8);
            return;
        }
        let s = i32::from(scale.max(1));
        let (bw, bh) = (self.image_buffer.width() as i32, self.image_buffer.height() as i32);
        let raw = self.image_buffer.bytes_per_pixel() == 4;
        let mut row: Vec<u8> = Vec::new();
        for part in frame {
            let spr = &lib.sprites[part.sprite as usize];
            let a = (u32::from(part.alpha) * ga + 127) / 255;
            if a == 0 || spr.w == 0 || spr.h == 0 {
                continue;
            }
            if fx::lava_radial_on(part.flags) {
                self.draw_fx_lava(x, y, part, spr, a);
                continue;
            }
            if fx::wide_window_on(part.flags, scale) {
                self.draw_fx_wide(x, y, part, spr, a);
                continue;
            }
            let flip = part.flags & FX_FLIP_X != 0;
            let add = part.flags & FX_ADD != 0;
            let m = if part.flags & FX_X2 != 0 { 2 * s } else { s };
            let (cx, cy) = fx::claw_shift(part.flags, s, spr, i32::from(part.dx), i32::from(part.dy));
            let (ox, oy) = (
                x.saturating_add(i32::from(part.dx) * s + cx),
                y.saturating_add(i32::from(part.dy) * s + cy),
            );
            let (w, h) = (spr.w as i32, spr.h as i32);
            let (x_lo, x_hi) = if flip {
                (ox + (spr.ax - (w - 1)) * m, ox + spr.ax * m + m - 1)
            } else {
                (ox - spr.ax * m, ox + (w - spr.ax) * m - 1)
            };
            let (cx0, cx1) = (x_lo.max(0), x_hi.min(bw - 1));
            if cx0 > cx1 {
                continue;
            }
            let n = (cx1 - cx0 + 1) as usize;
            let col = |dx: i32| -> usize {
                (if flip {
                    spr.ax - (dx - ox).div_euclid(m)
                } else {
                    spr.ax + (dx - ox).div_euclid(m)
                }) as usize
            };
            for dy in (oy - spr.ay * m).max(0)..=(oy + (h - spr.ay) * m - 1).min(bh - 1) {
                let sy = spr.ay + (dy - oy).div_euclid(m);
                let src = &spr.px[(sy * w) as usize..((sy + 1) * w) as usize];
                if raw {
                    row.resize(n * 4, 0);
                    if self.image_buffer.get_row_raw(cx0, dy, &mut row) {
                        // Byte order: [b, g, r, a] (the ARGB pixel stored little-endian — `draw`'s raw path).
                        for (i, d) in row.chunks_exact_mut(4).enumerate() {
                            let p = src[col(cx0 + i as i32)];
                            let sa = u32::from(p[3]) * a / 255;
                            if sa == 0 {
                                continue;
                            }
                            fx_mix(&mut d[2], p[0], sa, add);
                            fx_mix(&mut d[1], p[1], sa, add);
                            fx_mix(&mut d[0], p[2], sa, add);
                            d[3] = 0xff;
                        }
                        if self.image_buffer.put_row_raw(cx0, dy, &row) {
                            continue;
                        }
                    }
                }
                for dx in cx0..=cx1 {
                    let p = src[col(dx)];
                    let sa = u32::from(p[3]) * a / 255;
                    if sa == 0 {
                        continue;
                    }
                    let mut bg = self.image_buffer.get_pixel(dx, dy);
                    fx_mix(&mut bg.r, p[0], sa, add);
                    fx_mix(&mut bg.g, p[1], sa, add);
                    fx_mix(&mut bg.b, p[2], sa, add);
                    bg.a = 0xff;
                    self.image_buffer.put_pixel(dx, dy, bg);
                }
            }
        }
    }

    /// [`fx::FX_LAVA_RADIAL`]: the piece's pixels, drawn at this ring tile, stretched out to the doubled ring.
    fn draw_fx_lava(&mut self, x: i32, y: i32, part: &fx::FxPart, spr: &fx::FxSprite, alpha: u32) {
        let flip = part.flags & FX_FLIP_X != 0;
        let add = part.flags & FX_ADD != 0;
        let (bw, bh) = (self.image_buffer.width() as i32, self.image_buffer.height() as i32);
        let (x0, y0, x1, y1) = fx::lava_dest_box(
            x,
            y,
            i32::from(part.dx),
            i32::from(part.dy),
            spr.ax,
            spr.ay,
            spr.w as i32,
            spr.h as i32,
            flip,
        );
        let (x0, x1) = (x0.max(0), x1.min(bw - 1));
        let (y0, y1) = (y0.max(0), y1.min(bh - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        let (w, h) = (spr.w as i32, spr.h as i32);
        for py in y0..=y1 {
            for px in x0..=x1 {
                let Some((ix, iy)) = fx::lava_sprite_px(
                    x,
                    y,
                    i32::from(part.dx),
                    i32::from(part.dy),
                    spr.ax,
                    spr.ay,
                    w,
                    h,
                    flip,
                    px as f32 + 0.5,
                    py as f32 + 0.5,
                ) else {
                    continue;
                };
                let p = spr.px[(iy * w + ix) as usize];
                let sa = u32::from(p[3]) * alpha / 255;
                if sa == 0 {
                    continue;
                }
                let mut bg = self.image_buffer.get_pixel(px, py);
                fx_mix(&mut bg.r, p[0], sa, add);
                fx_mix(&mut bg.g, p[1], sa, add);
                fx_mix(&mut bg.b, p[2], sa, add);
                bg.a = 0xff;
                self.image_buffer.put_pixel(px, py, bg);
            }
        }
    }

    /// [`fx::FX_WIDE_WINDOW`] at scale 2: each source pixel is painted where the doubled hit window puts it.
    fn draw_fx_wide(&mut self, x: i32, y: i32, part: &fx::FxPart, spr: &fx::FxSprite, alpha: u32) {
        let flip = part.flags & FX_FLIP_X != 0;
        let add = part.flags & FX_ADD != 0;
        let dir = fx::wide_dir(part.flags);
        let (bw, bh) = (self.image_buffer.width() as i32, self.image_buffer.height() as i32);
        let (x0, y0, x1, y1) = fx::wide_dest_box(
            x,
            y,
            spr.ax,
            spr.ay,
            i32::from(part.dx),
            i32::from(part.dy),
            spr.w as i32,
            spr.h as i32,
            flip,
            dir,
        );
        let (x0, x1) = (x0.max(0), x1.min(bw - 1));
        let (y0, y1) = (y0.max(0), y1.min(bh - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        let (w, h) = (spr.w as i32, spr.h as i32);
        for py in y0..=y1 {
            for px in x0..=x1 {
                let feet_x = px as f32 + 0.5 - x as f32;
                let feet_y = py as f32 + 0.5 - y as f32;
                let Some((ix, iy)) = fx::wide_sprite_px(dir, flip, spr.ax, spr.ay, i32::from(part.dx), i32::from(part.dy), w, h, feet_x, feet_y)
                else {
                    continue;
                };
                let p = spr.px[(iy * w + ix) as usize];
                let sa = u32::from(p[3]) * alpha / 255;
                if sa == 0 {
                    continue;
                }
                let mut bg = self.image_buffer.get_pixel(px, py);
                fx_mix(&mut bg.r, p[0], sa, add);
                fx_mix(&mut bg.g, p[1], sa, add);
                fx_mix(&mut bg.b, p[2], sa, add);
                bg.a = 0xff;
                self.image_buffer.put_pixel(px, py, bg);
            }
        }
    }
}

/// One channel of an effect pixel over the background: `s` at coverage `sa` (0..=255) blended, or added and clamped.
fn fx_mix(d: &mut u8, s: u8, sa: u32, add: bool) {
    let (dv, sv) = (u32::from(*d), u32::from(s));
    *d = if add {
        (dv + sv * sa / 255).min(255) as u8
    } else {
        ((sv * sa + dv * (255 - sa) + 127) / 255) as u8
    };
}

/// Whether the point lies within the pie sweep starting at `start_deg` spanning
/// `sweep_deg` degrees (counterclockwise for positive, clockwise for negative).
/// Angles follow the J2ME/WIPI convention: 0° is 3 o'clock, positive is CCW.
fn point_in_sweep(px: f32, py: f32, cx: f32, cy: f32, start_deg: f32, sweep_deg: f32) -> bool {
    if sweep_deg.abs() >= 360.0 {
        return true;
    }
    if sweep_deg == 0.0 {
        return false;
    }

    let dx = px - cx;
    let dy = py - cy;
    if dx * dx + dy * dy < 1.0 {
        return true; // apex of the pie
    }

    // screen y grows downward, so negate dy to match the math convention
    let angle = (-dy).atan2(dx).to_degrees();
    let (start, sweep) = if sweep_deg > 0.0 {
        (start_deg, sweep_deg)
    } else {
        (start_deg + sweep_deg, -sweep_deg)
    };

    (angle - start).rem_euclid(360.0) <= sweep
}

#[allow(clippy::too_many_arguments)]
impl<T> Canvas for ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    fn image(&self) -> &dyn Image {
        &self.image_buffer
    }

    fn set_xor_mode(&mut self, xor_mode: bool) {
        self.xor_mode = xor_mode;
    }

    fn copy_area(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, w: u32, h: u32, clip: Clip) {
        fx::hd_touch();
        let image_width = (self.image_buffer.width() as i64).min(i32::MAX as i64);
        let image_height = (self.image_buffer.height() as i64).min(i32::MAX as i64);

        let x_start = (dx as i64).max(clip.x as i64).max(0);
        let y_start = (dy as i64).max(clip.y as i64).max(0);
        let x_end = (dx as i64 + w as i64).min(clip.x as i64 + clip.width as i64).min(image_width);
        let y_end = (dy as i64 + h as i64).min(clip.y as i64 + clip.height as i64).min(image_height);

        if x_start >= x_end || y_start >= y_end {
            return;
        }

        wie_util::stats::add_canvas_copy(((x_end - x_start).max(0) * (y_end - y_start).max(0)) as u64);
        let mut pixels = Vec::with_capacity(((x_end - x_start) * (y_end - y_start)) as usize);
        for y_dst in y_start..y_end {
            for x_dst in x_start..x_end {
                let x_src = sx as i64 + x_dst - dx as i64;
                let y_src = sy as i64 + y_dst - dy as i64;

                if x_src < 0 || y_src < 0 || x_src >= image_width || y_src >= image_height {
                    continue;
                }

                pixels.push((x_dst as i32, y_dst as i32, self.image_buffer.get_pixel(x_src as i32, y_src as i32)));
            }
        }

        for (x, y, color) in pixels {
            self.image_buffer.put_pixel(x, y, color);
        }
    }

    fn draw(&mut self, dx: i32, dy: i32, w: u32, h: u32, src: &dyn Image, sx: i32, sy: i32, clip: Clip) {
        fx::hd_touch();
        // Intersect all four constraints ONCE (src bounds, dst bounds, clip, the
        // requested w x h) instead of re-testing them per pixel. `x`/`y` below are
        // offsets into the requested region, exactly as in the old per-pixel loop.
        let x_lo = 0i64.max(-(sx as i64)).max(-(dx as i64)).max(clip.x as i64 - dx as i64);
        let y_lo = 0i64.max(-(sy as i64)).max(-(dy as i64)).max(clip.y as i64 - dy as i64);
        let x_hi = (w as i64)
            .min(src.width() as i64 - sx as i64)
            .min(self.image_buffer.width() as i64 - dx as i64)
            .min(clip.x as i64 + clip.width as i64 - dx as i64);
        let y_hi = (h as i64)
            .min(src.height() as i64 - sy as i64)
            .min(self.image_buffer.height() as i64 - dy as i64)
            .min(clip.y as i64 + clip.height as i64 - dy as i64);
        wie_util::stats::add_canvas_draw(((x_hi - x_lo).max(0) * (y_hi - y_lo).max(0)) as u64);
        if x_lo >= x_hi || y_lo >= y_hi {
            return;
        }
        let (x_lo, x_hi, y_lo, y_hi) = (x_lo as i32, x_hi as i32, y_lo as i32, y_hi as i32);

        // ★ Same-format 16bpp blit = raw row copies. RGB565 carries no alpha (its
        // `to_color` always yields a = 255), so the per-pixel blend was a straight
        // overwrite spelled expensively: get_pixel + two colour conversions + an
        // f32 blend per pixel. This is the hot path of the game's world rendering —
        // every MC_grpDrawImage / MC_grpCopyFrameBuffer of the map tiles and the
        // per-frame offscreen -> screen present go through here.
        let alpha = draw_alpha();
        if alpha == 255 && !self.xor_mode && src.bytes_per_pixel() == 2 && self.image_buffer.bytes_per_pixel() == 2 {
            let raw = src.raw();
            let src_w = src.width() as usize;
            let row_px = (x_hi - x_lo) as usize;
            let mut ok = true;
            for y in y_lo..y_hi {
                let start = (((sy + y) as usize) * src_w + (sx + x_lo) as usize) * 2;
                ok &= self.image_buffer.put_row_raw(dx + x_lo, dy + y, &raw[start..start + row_px * 2]);
            }
            if ok {
                return;
            }
            // A buffer that cannot take raw rows falls through to the exact path.
        }

        // ★★ SAME-FORMAT 32bpp (ARGB) BLIT, ROW AT A TIME (2026-09-09).
        //
        // This is hero2's world-rendering path and it had no fast route at all: the
        // 16bpp branch above cannot take it (hero2's images are 4 B/px - `Image::
        // createImage` makes ARGB), so every drawn pixel went through `src.get_pixel`
        // + `put_pixel`, and for a guest-backed buffer each of those is a `dyn`
        // dispatch into `ArmCore` that takes the core mutex and walks the page table.
        // Measured: ~24 K drawn px/frame, so ~48 K locked guest accesses per frame.
        //
        // Unlike RGB565, ARGB carries alpha, so a blanket raw copy would be WRONG.
        // The row is therefore classified first:
        //   * every pixel opaque  -> a raw row copy IS the per-pixel result (the
        //     per-pixel path's `a == 255` arm is a straight `put_pixel(color)`, and
        //     `from_color(to_color(raw)) == raw`), so copy it;
        //   * otherwise           -> read the destination row once, compose in HOST
        //     memory with the exact same f32 arithmetic and `as u8` truncation as
        //     `compose_pixel`, write it back once.
        // Either way the output is bit-identical to the per-pixel path - which the
        // probe's frame dumps are the standing check on.
        //
        // Any unsupported/out-of-range row makes the whole call fall through to the
        // per-pixel path rather than half-drawing.
        if !self.xor_mode && src.bytes_per_pixel() == 4 && self.image_buffer.bytes_per_pixel() == 4 {
            let row_px = (x_hi - x_lo) as usize;
            let mut src_row = vec![0u8; row_px * 4];
            let mut dst_row = vec![0u8; row_px * 4];
            let mut ok = true;
            for y in y_lo..y_hi {
                if !src.raw_row(sx + x_lo, sy + y, row_px, &mut src_row) {
                    ok = false;
                    break;
                }
                // Byte order: `ArgbPixel::from_color` packs (a<<24)|(r<<16)|(g<<8)|b
                // into a u32, stored little-endian, so a pixel's bytes are [b, g, r, a].
                if alpha != 255 {
                    // The host draw alpha: the row is composed, never copied.
                    for p in src_row.chunks_exact_mut(4) {
                        p[3] = ((p[3] as u16 * alpha as u16) / 255) as u8;
                    }
                }
                if src_row.chunks_exact(4).all(|p| p[3] == 0xff) {
                    if !self.image_buffer.put_row_raw(dx + x_lo, dy + y, &src_row) {
                        ok = false;
                        break;
                    }
                    wie_util::stats::add_canvas_row(0);
                    continue;
                }
                if !self.image_buffer.get_row_raw(dx + x_lo, dy + y, &mut dst_row) {
                    ok = false;
                    break;
                }
                for (s_px, d_px) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
                    let a = s_px[3];
                    if a == 0 {
                        // Blend factor 0 leaves the background exactly - skip.
                        continue;
                    }
                    if a == 255 {
                        d_px.copy_from_slice(s_px);
                        continue;
                    }
                    let factor = a as f32 / 255.0;
                    let mix = |sc: u8, bc: u8| (sc as f32 * factor + bc as f32 * (1.0 - factor)) as u8;
                    d_px[0] = mix(s_px[0], d_px[0]); // b
                    d_px[1] = mix(s_px[1], d_px[1]); // g
                    d_px[2] = mix(s_px[2], d_px[2]); // r
                    d_px[3] = 0xff; // compose_pixel always writes a = 0xff
                }
                if !self.image_buffer.put_row_raw(dx + x_lo, dy + y, &dst_row) {
                    ok = false;
                    break;
                }
                wie_util::stats::add_canvas_row(1);
            }
            if ok {
                return;
            }
            wie_util::stats::add_canvas_row(2);
            // Fall through. A row that was already written is overwritten with the
            // same value by the per-pixel path, so a partial fast path is harmless.
        }

        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                let mut color = src.get_pixel(sx + x, sy + y);
                if alpha != 255 {
                    color.a = ((color.a as u16 * alpha as u16) / 255) as u8;
                }
                if self.xor_mode {
                    self.blend_pixel(dx + x, dy + y, color);
                } else if color.a == 255 {
                    // Blend factor 1.0 — the result IS the source pixel; skip the
                    // background read and the f32 mix.
                    self.image_buffer.put_pixel(dx + x, dy + y, color);
                } else if color.a != 0 {
                    self.blend_pixel(dx + x, dy + y, color);
                }
                // a == 0: blend factor 0 leaves the background exactly — skip.
            }
        }
    }

    fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color) {
        // ★ 2026-10-14: under the effect lines (`FX_TAG`) a line is a library effect — frame `y2 − y1` of effect `x2 − x1`
        //   anchored at (x1, y1); off, nothing below changes.
        if fx_lines() {
            self.draw_fx(x1, y1, x2.wrapping_sub(x1), y2.wrapping_sub(y1));
            return;
        }
        fx::hd_touch();
        // ★ 2026-10-08: the host draw alpha scales lines too (hero2's 천공의검 picture) — 255 leaves this exact.
        let color = with_draw_alpha(color);
        // ★ 2026-10-08 (3): under a host stroke the line is a tapered capsule (see `STROKE`); off, this is exact.
        let stroke = stroke();
        if stroke != 0 {
            self.draw_stroke(x1, y1, x2, y2, color, stroke);
            return;
        }
        if x1 == x2 && y1 == y2 {
            self.blend_pixel(x1 as _, y1 as _, color);
            return;
        }

        // bresenham's line drawing
        let dx = (x2 - x1).abs();
        let dy = (y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;

        let mut x = x1;
        let mut y = y1;

        while x != x2 || y != y2 {
            self.blend_pixel(x as _, y as _, color);

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn draw_text(&mut self, string: &str, x: i32, y: i32, text_alignment: TextAlignment, color: Color) {
        fx::hd_touch();
        let size = 10.0; // TODO
        let font = FONT.as_scaled(FONT.pt_to_px_scale(size).unwrap());

        let total_width = string.chars().map(|c| font.h_advance(font.scaled_glyph(c).id)).sum::<f32>();
        let x = match text_alignment {
            TextAlignment::Left => x,
            TextAlignment::Center => x - (total_width / 2.0) as i32,
            TextAlignment::Right => x - total_width as i32,
        };

        let mut position = 0.0;
        for c in string.chars() {
            if c.is_control() {
                continue;
            }

            let glyph = font.scaled_glyph(c);
            let h_advance = font.h_advance(glyph.id);

            if let Some(outlined_glyph) = font.outline_glyph(glyph) {
                outlined_glyph.draw(|glyph_x: u32, glyph_y, c| {
                    let bounds = outlined_glyph.px_bounds();
                    self.blend_pixel(
                        x + (glyph_x as f32 + bounds.min.x + position) as i32,
                        y + (glyph_y as f32 + bounds.min.y + size) as i32,
                        Color {
                            a: (c * 255.0) as u8,
                            r: color.r,
                            g: color.g,
                            b: color.b,
                        },
                    )
                });
            }

            position += h_advance;
        }
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        fx::hd_touch();
        // TODO use put_pixels
        for x in x..x + (w as i32) {
            if x < 0 || x >= self.image_buffer.width() as i32 {
                continue;
            }
            if x < clip.x || x >= clip.x + clip.width as i32 {
                continue;
            }
            if y < 0 || y >= self.image_buffer.height() as i32 {
                continue;
            }
            if y < clip.y || y >= clip.y + clip.height as i32 {
                continue;
            }

            self.put_pixel(x, y, color);
            self.put_pixel(x, y + (h as i32) - 1, color);
        }
        for y in y..y + (h as i32) {
            if x < 0 || x >= self.image_buffer.width() as i32 {
                continue;
            }
            if x < clip.x || x >= clip.x + clip.width as i32 {
                continue;
            }
            if y < 0 || y >= self.image_buffer.height() as i32 {
                continue;
            }
            if y < clip.y || y >= clip.y + clip.height as i32 {
                continue;
            }

            self.put_pixel(x, y, color);
            self.put_pixel(x + (w as i32) - 1, y, color);
        }
    }

    fn draw_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        fx::hd_touch();
        if w == 0 || h == 0 {
            return;
        }

        let a = (w as f32 - 1.0) / 2.0;
        let b = (h as f32 - 1.0) / 2.0;
        let cx = x as f32 + a;
        let cy = y as f32 + b;

        self.stroke_arc(cx, cy, a, b, start_angle as f32, arc_angle as f32, color, &clip);
    }

    fn draw_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        fx::hd_touch();
        if w == 0 || h == 0 {
            return;
        }
        if arc_width == 0 || arc_height == 0 {
            return self.draw_rect(x, y, w, h, color, clip);
        }

        let rx = (arc_width.min(w) as f32 / 2.0).max(0.5);
        let ry = (arc_height.min(h) as f32 / 2.0).max(0.5);
        let rxi = rx.round() as i32;
        let ryi = ry.round() as i32;

        let left = x;
        let right = x + w as i32 - 1;
        let top = y;
        let bottom = y + h as i32 - 1;

        for px in (left + rxi)..=(right - rxi) {
            self.plot(px, top, color, &clip);
            self.plot(px, bottom, color, &clip);
        }
        for py in (top + ryi)..=(bottom - ryi) {
            self.plot(left, py, color, &clip);
            self.plot(right, py, color, &clip);
        }

        self.stroke_arc(right as f32 - rx, top as f32 + ry, rx, ry, 0.0, 90.0, color, &clip);
        self.stroke_arc(left as f32 + rx, top as f32 + ry, rx, ry, 90.0, 90.0, color, &clip);
        self.stroke_arc(left as f32 + rx, bottom as f32 - ry, rx, ry, 180.0, 90.0, color, &clip);
        self.stroke_arc(right as f32 - rx, bottom as f32 - ry, rx, ry, 270.0, 90.0, color, &clip);
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        fx::hd_touch();
        // Clamp ONCE against the buffer and the clip, then fill row spans. The old
        // loop ran put_pixel (bounds check + colour conversion) for every pixel,
        // which made full-screen fills — map transitions, darkening overlays — one
        // of the most expensive single guest calls.
        let x0 = (x as i64).max(clip.x as i64).max(0).min(self.image_buffer.width() as i64) as i32;
        let y0 = (y as i64).max(clip.y as i64).max(0).min(self.image_buffer.height() as i64) as i32;
        let x1 = (x as i64 + w as i64)
            .min(clip.x as i64 + clip.width as i64)
            .min(self.image_buffer.width() as i64)
            .max(x0 as i64) as i32;
        let y1 = (y as i64 + h as i64)
            .min(clip.y as i64 + clip.height as i64)
            .min(self.image_buffer.height() as i64)
            .max(y0 as i64) as i32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        wie_util::stats::add_canvas_fill(((x1 - x0) as i64 * (y1 - y0) as i64) as u64);
        // ★ 2026-10-08: under a host draw alpha (hero2's 천공의검 flash) the fill blends pixel by pixel; at 255 the
        //   span path below is untouched.
        if !self.xor_mode && draw_alpha() != 255 {
            let color = with_draw_alpha(color);
            for y in y0..y1 {
                for x in x0..x1 {
                    self.blend_pixel(x, y, color);
                }
            }
            return;
        }
        if self.xor_mode {
            for y in y0..y1 {
                for x in x0..x1 {
                    self.put_pixel(x, y, color);
                }
            }
        } else {
            self.image_buffer.fill_span(x0, y0, x1, y1, color);
        }
    }

    fn fill_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        fx::hd_touch();
        if w == 0 || h == 0 {
            return;
        }

        let a = (w as f32 - 1.0) / 2.0;
        let b = (h as f32 - 1.0) / 2.0;
        let cx = x as f32 + a;
        let cy = y as f32 + b;
        let da = a.max(0.5);
        let db = b.max(0.5);
        let start = start_angle as f32;
        let sweep = arc_angle as f32;

        for py in y..y + h as i32 {
            for px in x..x + w as i32 {
                let nx = (px as f32 - cx) / da;
                let ny = (py as f32 - cy) / db;
                if nx * nx + ny * ny > 1.0 {
                    continue;
                }
                if !point_in_sweep(px as f32, py as f32, cx, cy, start, sweep) {
                    continue;
                }
                self.plot(px, py, color, &clip);
            }
        }
    }

    fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        fx::hd_touch();
        if w == 0 || h == 0 {
            return;
        }
        if arc_width == 0 || arc_height == 0 {
            return self.fill_rect(x, y, w, h, color, clip);
        }

        let rx = (arc_width.min(w) as f32 / 2.0).max(0.5);
        let ry = (arc_height.min(h) as f32 / 2.0).max(0.5);

        let left_center = x as f32 + rx;
        let right_center = (x + w as i32 - 1) as f32 - rx;
        let top_center = y as f32 + ry;
        let bottom_center = (y + h as i32 - 1) as f32 - ry;

        for py in y..y + h as i32 {
            let cy = if (py as f32) < top_center {
                top_center
            } else if (py as f32) > bottom_center {
                bottom_center
            } else {
                py as f32
            };
            for px in x..x + w as i32 {
                let cx = if (px as f32) < left_center {
                    left_center
                } else if (px as f32) > right_center {
                    right_center
                } else {
                    px as f32
                };

                let nx = (px as f32 - cx) / rx;
                let ny = (py as f32 - cy) / ry;
                if nx * nx + ny * ny <= 1.0 {
                    self.plot(px, py, color, &clip);
                }
            }
        }
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        fx::hd_touch();
        self.compose_pixel(x, y, color, false);
    }
}

pub struct Clip {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Clip {
    pub fn intersect(&self, other: &Clip) -> Clip {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let width = (self.x + (self.width as i32)).min(other.x + (other.width as i32)) - x;
        let height = (self.y + (self.height as i32)).min(other.y + (other.height as i32)) - y;

        Clip {
            x,
            y,
            width: width as _,
            height: height as _,
        }
    }
}

pub fn decode_image(data: &[u8]) -> Result<Box<dyn Image>> {
    extern crate std; // XXX

    use std::io::Cursor;

    if data[0] == b'L' && data[1] == b'B' && data[2] == b'M' && data[3] == b'P' {
        return decode_lbmp(data);
    }

    let image = ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .map_err(|x| WieError::FatalError(x.to_string()))?
        .decode()
        .map_err(|x| WieError::FatalError(x.to_string()))?;
    let rgba = image.into_rgba8();

    let data = rgba.pixels().flat_map(|x| [x.0[2], x.0[1], x.0[0], x.0[3]]).collect::<Vec<_>>();

    Ok(Box::new(VecImageBuffer::<ArgbPixel>::from_raw(
        rgba.width(),
        rgba.height(),
        pod_collect_to_vec(&data),
    )) as Box<_>)
}

pub fn string_width(string: &str, pt_size: f32) -> f32 {
    let font = FONT.as_scaled(FONT.pt_to_px_scale(pt_size).unwrap());

    string.chars().map(|c| font.h_advance(font.scaled_glyph(c).id)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use wie_util::Result;

    use crate::canvas::{Clip, Image, ImageBufferCanvas};

    use super::{ArgbPixel, Canvas, Color, Rgb332Pixel, STROKE_SOFT, VecImageBuffer};

    #[test]
    fn test_canvas() -> Result<()> {
        let image_buffer = VecImageBuffer::<ArgbPixel>::new(10, 10);
        let mut canvas = ImageBufferCanvas::new(image_buffer);

        let clip = Clip {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        canvas.fill_rect(0, 0, 10, 10, Color { r: 0, g: 0, b: 0, a: 255 }, clip);

        let image_buffer = canvas.into_inner();
        let raw = image_buffer.raw();

        assert_eq!(raw.len(), 10 * 10 * 4);
        for i in 0..10 * 10 {
            assert_eq!(raw[i * 4], 0);
            assert_eq!(raw[i * 4 + 1], 0);
            assert_eq!(raw[i * 4 + 2], 0);
            assert_eq!(raw[i * 4 + 3], 255);
        }

        Ok(())
    }

    fn full_clip(size: u32) -> Clip {
        Clip {
            x: 0,
            y: 0,
            width: size,
            height: size,
        }
    }

    const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    const BACKGROUND: Color = Color {
        a: 255,
        r: 0x12,
        g: 0x34,
        b: 0x56,
    };

    const XOR_COLOR: Color = Color {
        a: 255,
        r: 0xf0,
        g: 0x0f,
        b: 0xaa,
    };

    fn is_set(image: &impl Image, x: i32, y: i32) -> bool {
        let c = image.get_pixel(x, y);
        c.r != 0 || c.g != 0 || c.b != 0
    }

    fn assert_color(image: &dyn Image, x: i32, y: i32, expected: Color) {
        let actual = image.get_pixel(x, y);
        assert_eq!((actual.a, actual.r, actual.g, actual.b), (expected.a, expected.r, expected.g, expected.b));
    }

    #[test]
    fn test_xor_mode_fill_rect_toggles_pixels() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 4));

        canvas.fill_rect(0, 0, 4, 4, BACKGROUND, full_clip(4));
        canvas.set_xor_mode(true);
        canvas.fill_rect(1, 1, 2, 2, XOR_COLOR, full_clip(4));

        assert_color(
            canvas.image(),
            1,
            1,
            Color {
                a: 255,
                r: BACKGROUND.r ^ XOR_COLOR.r,
                g: BACKGROUND.g ^ XOR_COLOR.g,
                b: BACKGROUND.b ^ XOR_COLOR.b,
            },
        );
        assert_color(canvas.image(), 0, 0, BACKGROUND);

        canvas.fill_rect(1, 1, 2, 2, XOR_COLOR, full_clip(4));

        assert_color(canvas.image(), 1, 1, BACKGROUND);
    }

    #[test]
    fn test_xor_mode_transparent_source_is_noop() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(1, 1));
        canvas.fill_rect(0, 0, 1, 1, BACKGROUND, full_clip(1));
        canvas.set_xor_mode(true);

        let src = VecImageBuffer::<ArgbPixel>::from_raw(1, 1, vec![0x00ff0000]);
        canvas.draw(0, 0, 1, 1, &src, 0, 0, full_clip(1));

        assert_color(canvas.image(), 0, 0, BACKGROUND);
    }

    #[test]
    fn test_xor_mode_rgb332_toggles_native_pixel() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<Rgb332Pixel>::new(1, 1));
        let background = Color { a: 255, r: 0, g: 0, b: 0 };
        let source = Color { a: 255, r: 19, g: 0, b: 0 };

        canvas.fill_rect(0, 0, 1, 1, background, full_clip(1));
        canvas.set_xor_mode(true);
        canvas.fill_rect(0, 0, 1, 1, source, full_clip(1));
        canvas.fill_rect(0, 0, 1, 1, source, full_clip(1));

        assert_color(canvas.image(), 0, 0, background);
    }

    #[test]
    fn test_copy_area_uses_source_snapshot_for_overlap() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 1));
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let black = Color { a: 255, r: 0, g: 0, b: 0 };

        canvas.put_pixel(0, 0, red);
        canvas.put_pixel(1, 0, green);
        canvas.put_pixel(2, 0, blue);
        canvas.put_pixel(3, 0, black);

        canvas.copy_area(1, 0, 0, 0, 3, 1, full_clip(4));

        assert_color(canvas.image(), 0, 0, red);
        assert_color(canvas.image(), 1, 0, red);
        assert_color(canvas.image(), 2, 0, green);
        assert_color(canvas.image(), 3, 0, blue);
    }

    #[test]
    fn test_copy_area_respects_clip() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 1));
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let black = Color { a: 255, r: 0, g: 0, b: 0 };
        let clip = Clip {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
        };

        canvas.put_pixel(0, 0, red);
        canvas.put_pixel(1, 0, green);
        canvas.put_pixel(2, 0, blue);
        canvas.put_pixel(3, 0, black);

        canvas.copy_area(1, 0, 0, 0, 3, 1, clip);

        assert_color(canvas.image(), 0, 0, red);
        assert_color(canvas.image(), 1, 0, green);
        assert_color(canvas.image(), 2, 0, green);
        assert_color(canvas.image(), 3, 0, black);
    }

    #[test]
    fn test_fill_arc_full_circle() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.fill_arc(0, 0, 32, 32, 0, 360, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 16), "center should be filled");
        assert!(!is_set(&image, 0, 0), "corner should be outside the ellipse");
        assert!(!is_set(&image, 31, 31), "corner should be outside the ellipse");
    }

    #[test]
    fn test_fill_arc_quadrant() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        // 0..90 deg sweeps from 3 o'clock CCW to 12 o'clock => upper-right quadrant
        canvas.fill_arc(0, 0, 32, 32, 0, 90, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 24, 8), "upper-right quadrant should be filled");
        assert!(!is_set(&image, 8, 8), "upper-left quadrant should be empty");
        assert!(!is_set(&image, 8, 24), "lower-left quadrant should be empty");
        assert!(!is_set(&image, 24, 24), "lower-right quadrant should be empty");
    }

    #[test]
    fn test_fill_arc_negative_sweep_is_clockwise() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        // 0..-90 deg sweeps CW from 3 o'clock to 6 o'clock => lower-right quadrant
        canvas.fill_arc(0, 0, 32, 32, 0, -90, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 24, 24), "lower-right quadrant should be filled");
        assert!(!is_set(&image, 24, 8), "upper-right quadrant should be empty");
    }

    #[test]
    fn test_draw_arc_endpoint() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_arc(0, 0, 32, 32, 0, 360, WHITE, full_clip(32));
        let image = canvas.into_inner();

        // bounding box 32x32 => center (15.5, 15.5), radius 15.5; nominal endpoints round to:
        assert!(is_set(&image, 31, 16), "3 o'clock (0 deg) point");
        assert!(is_set(&image, 16, 0), "12 o'clock (90 deg) point");
        assert!(is_set(&image, 0, 16), "9 o'clock (180 deg) point");
        assert!(is_set(&image, 16, 31), "6 o'clock (270 deg) point");
        assert!(!is_set(&image, 16, 16), "arc outline must not fill the interior");
    }

    #[test]
    fn test_draw_round_rect_outline() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_round_rect(0, 0, 32, 32, 16, 16, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 0), "top straight edge");
        assert!(is_set(&image, 16, 31), "bottom straight edge");
        assert!(is_set(&image, 0, 16), "left straight edge");
        assert!(is_set(&image, 31, 16), "right straight edge");
        assert!(!is_set(&image, 0, 0), "rounded corner cut away");
        assert!(!is_set(&image, 16, 16), "outline must not fill the interior");
    }

    #[test]
    fn test_fill_arc_degenerate_width() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(8, 8));
        canvas.fill_arc(3, 0, 1, 8, 0, 360, WHITE, full_clip(8));
        let image = canvas.into_inner();

        for y in 0..8 {
            assert!(is_set(&image, 3, y), "degenerate 1px-wide column should be fully filled (y={y})");
        }
    }

    #[test]
    fn test_fill_round_rect_corners_empty() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.fill_round_rect(0, 0, 32, 32, 16, 16, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 16), "center should be filled");
        assert!(is_set(&image, 16, 0), "straight top edge should be filled");
        assert!(!is_set(&image, 0, 0), "rounded corner should be empty");
        assert!(!is_set(&image, 31, 0), "rounded corner should be empty");
    }

    /// The row-span fast path of `fill_rect` must be pixel-identical to the old
    /// per-pixel loop, including clip intersection and out-of-bounds clamping.
    #[test]
    fn test_fill_rect_fast_path_matches_per_pixel() {
        use super::Rgb565Pixel;

        let cases: &[(i32, i32, u32, u32, Clip)] = &[
            (2, 3, 5, 4, full_clip(16)),
            (-3, -2, 8, 6, full_clip(16)),
            (10, 12, 20, 20, full_clip(16)),
            (
                0,
                0,
                16,
                16,
                Clip {
                    x: 4,
                    y: 5,
                    width: 6,
                    height: 3,
                },
            ),
        ];
        for (x, y, w, h, clip) in cases {
            let mut fast = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            fast.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            fast.fill_rect(
                *x,
                *y,
                *w,
                *h,
                WHITE,
                Clip {
                    x: clip.x,
                    y: clip.y,
                    width: clip.width,
                    height: clip.height,
                },
            );

            // Reference: the old shape — put_pixel per pixel with inline tests.
            let mut reference = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            reference.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            for py in *y..*y + (*h as i32) {
                for px in *x..*x + (*w as i32) {
                    if px < 0 || py < 0 || px >= 16 || py >= 16 {
                        continue;
                    }
                    if px < clip.x || px >= clip.x + clip.width as i32 || py < clip.y || py >= clip.y + clip.height as i32 {
                        continue;
                    }
                    reference.put_pixel(px, py, WHITE);
                }
            }

            assert_eq!(
                fast.image().raw(),
                reference.image().raw(),
                "fill_rect({x}, {y}, {w}, {h}) diverged from the per-pixel reference"
            );
        }
    }

    /// The 16bpp raw-row-copy fast path of `draw` must place exactly the source
    /// pixels an opaque per-pixel blend would have placed, and touch nothing else —
    /// including with negative offsets and a clip.
    #[test]
    fn test_draw_565_row_copy_matches_per_pixel() {
        use super::{PixelType, Rgb565Pixel};

        // Source: a deterministic gradient.
        let mut src_data = alloc::vec::Vec::new();
        for i in 0..8 * 8u32 {
            src_data.push((i * 2557 % 65536) as u16);
        }
        let src = VecImageBuffer::<Rgb565Pixel>::from_raw(8, 8, src_data);

        let cases: &[(i32, i32, i32, i32, Clip)] = &[
            (3, 4, 0, 0, full_clip(16)),
            (-2, -3, 0, 0, full_clip(16)),
            (14, 14, 0, 0, full_clip(16)),
            (0, 0, -2, 3, full_clip(16)),
            (
                2,
                2,
                1,
                1,
                Clip {
                    x: 3,
                    y: 3,
                    width: 4,
                    height: 2,
                },
            ),
        ];
        for (dx, dy, sx, sy, clip) in cases {
            let mut fast = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            fast.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            fast.draw(
                *dx,
                *dy,
                8,
                8,
                &src,
                *sx,
                *sy,
                Clip {
                    x: clip.x,
                    y: clip.y,
                    width: clip.width,
                    height: clip.height,
                },
            );

            let mut reference = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            reference.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            for y in 0..8i32 {
                for x in 0..8i32 {
                    if sx + x < 0 || sy + y < 0 || sx + x >= 8 || sy + y >= 8 {
                        continue;
                    }
                    if dx + x < 0 || dy + y < 0 || dx + x >= 16 || dy + y >= 16 {
                        continue;
                    }
                    if dx + x < clip.x || dx + x >= clip.x + clip.width as i32 || dy + y < clip.y || dy + y >= clip.y + clip.height as i32 {
                        continue;
                    }
                    // The old path: blend of an opaque colour = plain overwrite.
                    reference.put_pixel(dx + x, dy + y, src.get_pixel(sx + x, sy + y));
                }
            }

            assert_eq!(
                fast.image().raw(),
                reference.image().raw(),
                "draw({dx}, {dy}, src {sx}, {sy}) diverged from the per-pixel reference"
            );
        }
        // The pixel round-trip the row copy skips must be the identity, or the two
        // paths could not be equal: every 565 value survives to_color -> from_color.
        for v in 0..=u16::MAX {
            assert_eq!(Rgb565Pixel::from_color(Rgb565Pixel::to_color(v)), v);
        }
    }

    /// Transparent (a == 0) source pixels leave the destination pixel alone, and
    /// opaque ones land exactly — the two blend fast paths.
    #[test]
    fn test_draw_alpha_fast_paths() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(2, 1));
        canvas.fill_rect(0, 0, 2, 1, BACKGROUND, full_clip(2));

        let src = VecImageBuffer::<ArgbPixel>::from_raw(2, 1, vec![0x00123456, 0xffabcdef]);
        canvas.draw(0, 0, 2, 1, &src, 0, 0, full_clip(2));

        assert_color(canvas.image(), 0, 0, BACKGROUND);
        assert_color(
            canvas.image(),
            1,
            0,
            Color {
                a: 255,
                r: 0xab,
                g: 0xcd,
                b: 0xef,
            },
        );
    }

    #[test]
    fn test_arc_respects_clip() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        let clip = Clip {
            x: 0,
            y: 0,
            width: 16,
            height: 32,
        };
        canvas.fill_arc(0, 0, 32, 32, 0, 360, WHITE, clip);
        let image = canvas.into_inner();

        assert!(is_set(&image, 8, 16), "inside clip should be filled");
        assert!(!is_set(&image, 24, 16), "outside clip must not be filled");
    }

    /// The host stroke, called directly (the process-wide switch is left alone — tests run in parallel): a hard taper from
    /// 4 px to 0 is solid near its start, thin at its end and nothing past it; a soft one is brightest on its axis and
    /// fades out sideways; a zero-length soft stroke is a round dot.
    #[test]
    fn test_host_stroke_tapers_and_glows() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(64, 32));
        canvas.draw_stroke(4, 16, 54, 16, WHITE, (16 << 8) | 0);
        let image = canvas.into_inner();
        for y in 13..=19 {
            assert!(is_set(&image, 6, y), "the blunt start is 4 px each side of the axis (row {y})");
        }
        assert!(!is_set(&image, 6, 11) && !is_set(&image, 6, 21), "and no wider");
        assert!(is_set(&image, 52, 16) && !is_set(&image, 52, 13), "the sharp end is the axis alone");
        assert!(!is_set(&image, 58, 16), "nothing past the tip");

        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_stroke(4, 16, 28, 16, WHITE, STROKE_SOFT | (24 << 8) | 24);
        let image = canvas.into_inner();
        let (axis, side, edge) = (image.get_pixel(16, 16).r, image.get_pixel(16, 19).r, image.get_pixel(16, 23).r);
        assert!(axis > side && side > edge && edge == 0, "soft: {axis} > {side} > {edge} = 0");

        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_stroke(16, 16, 16, 16, WHITE, STROKE_SOFT | (20 << 8) | 20);
        let image = canvas.into_inner();
        assert!(is_set(&image, 16, 16) && is_set(&image, 19, 16) && is_set(&image, 16, 13), "a dot");
        assert_eq!(image.get_pixel(19, 16).r, image.get_pixel(13, 16).r, "round: the same both sides");
        assert!(!is_set(&image, 23, 16), "radius 5");
    }

    /// An H2FX blob: a 3×2 atlas (row 0 = red · green · blue opaque, row 1 = white at alpha 128 · two clear), sprite 0 =
    /// the top row anchored on its middle pixel, sprite 1 = the half-white pixel; effect "e" = frame 0 [sprite 0 at (0,0)],
    /// frame 1 [sprite 0 mirrored at (0,1) · sprite 1 added at (−2,0) alpha 255].
    /// ★★ HD (2026-10-15): the v1 blob with a version-2 tail — sprite 0's twin at 2× (6 × 2, three colours, two columns each).
    fn fx_blob_v2() -> alloc::vec::Vec<u8> {
        fx_blob_v2_idx(&[1, 1, 2, 2, 3, 3, 1, 1, 2, 2, 3, 3])
    }

    /// `fx_blob_v2` with the twin's 6 × 2 colour indices given (0 = transparent · 1 red · 2 green · 3 blue).
    fn fx_blob_v2_idx(idx: &[u8; 12]) -> alloc::vec::Vec<u8> {
        use image::{ImageEncoder, codecs::png::PngEncoder};
        let mut b = fx_blob();
        b[4] = 2;
        let mut png = alloc::vec::Vec::new();
        PngEncoder::new(&mut png).write_image(idx, 6, 2, image::ExtendedColorType::L8).unwrap();
        b.extend_from_slice(&(png.len() as u32).to_le_bytes());
        b.extend_from_slice(&png);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(2);
        b.push(3);
        for v in [0u16, 0, 6, 2] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [2i16, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for c in [[255u8, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            b.extend_from_slice(&c);
        }
        b
    }

    /// The HD tests and the draw test share the process-wide HD mode.
    static HD_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// ★★ HD: a version-2 library parses with its twin (and refuses a twin past the atlas); under HD mode an effect draw is
    /// RECORDED, not drawn, and the record's layer map tells the guest's draws apart by when they came.
    #[test]
    fn test_fx_hd_record_keeps_the_paint_order() {
        let _guard = HD_TEST_LOCK.lock();
        let lib = super::FxLibrary::parse(&fx_blob_v2()).expect("parses");
        assert_eq!(lib.hd_stats(), (1, 12));
        let t = lib.sprites[0].hd.as_ref().unwrap();
        assert_eq!((t.scale, t.w, t.h, t.ax, t.ay, t.pal.len(), t.idx[2]), (2, 6, 2, 2, 0, 4, 2));
        assert!(lib.sprites[1].hd.is_none());
        let mut bad = fx_blob_v2();
        let n = bad.len();
        bad[n - 12 - 8 - 4] = 9; // the twin's width past the atlas
        assert!(super::FxLibrary::parse(&bad).is_err());
        assert!(super::FxLibrary::parse(&fx_blob()).expect("v1 parses").sprites[0].hd.is_none());

        super::set_hd_mode(true);
        super::hd_reset();
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(8, 4));
        let black = Color { a: 255, r: 0, g: 0, b: 0 };
        let white = Color {
            a: 255,
            r: 255,
            g: 255,
            b: 255,
        };
        canvas.fill_rect(0, 0, 8, 4, black, full_clip(8));
        canvas.draw_fx_from(&lib, 4, 1, 0, 0); //           group 0
        assert_color(canvas.image(), 4, 1, black); //       nothing drawn
        canvas.fill_rect(2, 1, 1, 1, white, full_clip(8)); // the guest paints over (2, 1): group 1 from here
        canvas.draw_fx_from(&lib, 0, 0, 0, 1); //           group 1
        canvas.draw_fx_from(&lib, 7, 3, 0, 0); //           group 1 (no guest draw between)
        canvas.fill_rect(5, 2, 1, 1, white, full_clip(8)); // after the last: the final group
        let raw = canvas.image().raw();
        let hd = super::hd_finish(&raw, 8, 4, 4).expect("a record");
        assert_eq!(hd.draws.len(), 3);
        assert_eq!(
            hd.draws
                .iter()
                .map(|d| (d.effect, d.age, d.x, d.y, d.group))
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![(0, 0, 4, 1, 0), (0, 1, 0, 0, 1), (0, 0, 7, 3, 1)]
        );
        assert_eq!(hd.layer[1 * 8 + 2], 1, "painted between the first and the second draw");
        assert_eq!(hd.layer[2 * 8 + 5], 2, "painted after the last draw");
        assert_eq!(hd.layer.iter().filter(|l| **l == 0).count(), 30);
        assert!(super::hd_finish(&raw, 8, 4, 4).is_none(), "taken");
        super::set_hd_mode(false);
        canvas.draw_fx_from(&lib, 4, 1, 0, 0);
        assert_color(canvas.image(), 3, 1, Color { a: 255, r: 255, g: 0, b: 0 });
    }

    /// ★★ HD: the compositor puts the twin on the window at the window's scale (nearest), only over guest pixels whose layer
    /// is at or under the draw's group; a sprite without a twin goes on at 1×; a flipped part mirrors about the anchor.
    #[test]
    fn test_fx_hd_compose_at_the_window_scale() {
        let lib = super::FxLibrary::parse(&fx_blob_v2()).expect("parses");
        let (w, h, k) = (8u32, 4u32, 2u32);
        let (ww, wh) = (w * k, h * k);
        let view = super::HdView {
            rect: (0, 0, ww, wh),
            src_w: w,
            src_y0: 0,
            src_h: h,
        };
        let black = 0xff00_0000u32;
        let mut hd = super::HdFrame {
            w,
            h,
            layer: alloc::vec![0; (w * h) as usize],
            draws: alloc::vec![super::HdDraw {
                effect: 0,
                age: 0,
                x: 4,
                y: 1,
                scale: 1,
                alpha: 255,
                group: 0
            }],
        };
        let mut dst = alloc::vec![black; (ww * wh) as usize];
        super::compose_hd(&lib, &hd, &mut dst, ww, wh, &view);
        let at = |dst: &[u32], x: u32, y: u32| dst[(y * ww + x) as usize] & 0xff_ffff;
        // the twin's six columns are three guest px wide: (3..4) red · (4..5) green · (5..6) blue → window x 6..8 · 8..10 · 10..12, rows 2..4
        assert_eq!(
            (
                at(&dst, 6, 2),
                at(&dst, 7, 3),
                at(&dst, 8, 2),
                at(&dst, 9, 3),
                at(&dst, 10, 2),
                at(&dst, 11, 3)
            ),
            (0xff0000, 0xff0000, 0x00ff00, 0x00ff00, 0x0000ff, 0x0000ff)
        );
        assert_eq!(
            (at(&dst, 5, 2), at(&dst, 12, 2), at(&dst, 6, 1), at(&dst, 6, 4)),
            (0, 0, 0, 0),
            "nothing outside"
        );
        // the guest painted over (4, 1) after the draw: the green column stays the window's
        hd.layer[1 * 8 + 4] = 1;
        let mut dst = alloc::vec![black; (ww * wh) as usize];
        super::compose_hd(&lib, &hd, &mut dst, ww, wh, &view);
        assert_eq!(
            (at(&dst, 6, 2), at(&dst, 8, 2), at(&dst, 9, 3), at(&dst, 10, 2)),
            (0xff0000, 0, 0, 0x0000ff)
        );
        // frame 1: the flipped twin one row down, and the 1× white (no twin) added at half alpha two px left
        hd.layer[1 * 8 + 4] = 0;
        hd.draws[0].age = 1;
        let mut dst = alloc::vec![black; (ww * wh) as usize];
        super::compose_hd(&lib, &hd, &mut dst, ww, wh, &view);
        assert_eq!(
            (at(&dst, 6, 4), at(&dst, 8, 5), at(&dst, 10, 4)),
            (0x0000ff, 0x00ff00, 0xff0000),
            "mirrored about the anchor column"
        );
        assert_eq!(at(&dst, 4, 2), 0x808080, "the 1× sprite at the window's scale, half white added on black");
        assert_eq!(at(&dst, 5, 3), 0x808080);
    }

    /// ★★ 2026-10-16: a twin denser than the window (scale 2 on a 1× window) is averaged over the 2×2 twin px each window px
    /// covers — premultiplied: red ×3 + green = (191, 63, 0); a transparent sample thins the alpha (blue at 3/4 over black).
    #[test]
    fn test_fx_hd_compose_averages_a_denser_twin() {
        let lib = super::FxLibrary::parse(&fx_blob_v2_idx(&[1, 2, 2, 2, 0, 3, 1, 1, 2, 2, 3, 3])).expect("parses");
        let (w, h) = (8u32, 4u32);
        let view = super::HdView {
            rect: (0, 0, w, h),
            src_w: w,
            src_y0: 0,
            src_h: h,
        };
        let hd = super::HdFrame {
            w,
            h,
            layer: alloc::vec![0; (w * h) as usize],
            draws: alloc::vec![super::HdDraw {
                effect: 0,
                age: 0,
                x: 4,
                y: 1,
                scale: 1,
                alpha: 255,
                group: 0
            }],
        };
        let mut dst = alloc::vec![0xff00_0000u32; (w * h) as usize];
        super::compose_hd(&lib, &hd, &mut dst, w, h, &view);
        let at = |x: u32, y: u32| dst[(y * w + x) as usize] & 0xff_ffff;
        assert_eq!(
            (at(3, 1), at(4, 1), at(5, 1)),
            (0xbf3f00, 0x00ff00, 0x0000bf),
            "mixed · solid · a quarter transparent"
        );
        assert_eq!((at(2, 1), at(6, 1), at(3, 0), at(3, 2)), (0, 0, 0, 0), "nothing outside");
        // the same twin on a 2× window takes the one centre sample, as before
        let view2 = super::HdView {
            rect: (0, 0, 2 * w, 2 * h),
            src_w: w,
            src_y0: 0,
            src_h: h,
        };
        let mut dst2 = alloc::vec![0xff00_0000u32; (4 * w * h) as usize];
        super::compose_hd(&lib, &hd, &mut dst2, 2 * w, 2 * h, &view2);
        let at2 = |x: u32, y: u32| dst2[(y * 2 * w + x) as usize] & 0xff_ffff;
        assert_eq!(
            (at2(6, 2), at2(7, 2), at2(6, 3), at2(8, 2), at2(9, 2)),
            (0xff0000, 0x00ff00, 0xff0000, 0x00ff00, 0x00ff00)
        );
    }

    fn fx_blob() -> alloc::vec::Vec<u8> {
        use image::{ImageEncoder, codecs::png::PngEncoder};
        let px: [u8; 24] = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut png = alloc::vec::Vec::new();
        PngEncoder::new(&mut png).write_image(&px, 3, 2, image::ExtendedColorType::Rgba8).unwrap();
        let mut b = alloc::vec::Vec::new();
        b.extend_from_slice(b"H2FX");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(png.len() as u32).to_le_bytes());
        b.extend_from_slice(&png);
        b.extend_from_slice(&2u32.to_le_bytes());
        for v in [0u16, 0, 3, 1, 1, 0, 0, 1, 1, 1, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(1);
        b.push(b'e');
        b.extend_from_slice(&2u32.to_le_bytes());
        let part = |b: &mut alloc::vec::Vec<u8>, s: u16, dx: i16, dy: i16, a: u8, f: u8| {
            b.extend_from_slice(&s.to_le_bytes());
            b.extend_from_slice(&dx.to_le_bytes());
            b.extend_from_slice(&dy.to_le_bytes());
            b.push(a);
            b.push(f);
        };
        b.extend_from_slice(&1u16.to_le_bytes());
        part(&mut b, 0, 0, 0, 255, 0);
        b.extend_from_slice(&2u16.to_le_bytes());
        part(&mut b, 0, 0, 1, 255, super::FX_FLIP_X);
        part(&mut b, 1, -2, 0, 255, super::FX_ADD);
        b
    }

    /// The effect library: the blob parses (and refuses a truncated or retagged one); frame 0 puts the three colours on
    /// either side of the anchor; frame 1 mirrors them one row down and adds half white; an age or effect past the library
    /// draws nothing.
    #[test]
    fn test_fx_library_draws_frames() {
        let _guard = HD_TEST_LOCK.lock();
        let blob = fx_blob();
        let lib = super::FxLibrary::parse(&blob).expect("parses");
        assert_eq!(
            (
                lib.sprites.len(),
                lib.effects.len(),
                lib.effects[0].name.as_str(),
                lib.effects[0].frames.len()
            ),
            (2, 1, "e", 2)
        );
        assert!(super::FxLibrary::parse(&blob[..blob.len() - 1]).is_err());
        let mut bad = blob.clone();
        bad[0] = b'X';
        assert!(super::FxLibrary::parse(&bad).is_err());

        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(8, 4));
        canvas.fill_rect(0, 0, 8, 4, Color { a: 255, r: 0, g: 0, b: 0 }, full_clip(8));
        canvas.draw_fx_from(&lib, 4, 1, 0, 0);
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        assert_color(canvas.image(), 3, 1, red);
        assert_color(canvas.image(), 4, 1, green);
        assert_color(canvas.image(), 5, 1, blue);
        canvas.draw_fx_from(&lib, 4, 1, 0, 1);
        assert_color(canvas.image(), 3, 2, blue);
        assert_color(canvas.image(), 5, 2, red);
        let added = canvas.image().get_pixel(2, 1);
        assert_eq!((added.r, added.g, added.b), (128, 128, 128), "half white added on black");
        canvas.draw_fx_from(&lib, 0, 0, 0, 2);
        canvas.draw_fx_from(&lib, 0, 0, 1, 0);
        canvas.draw_fx_from(&lib, 0, 0, -1, 0);
        assert_color(canvas.image(), 0, 0, Color { a: 255, r: 0, g: 0, b: 0 });
        // clipped at the edge: the anchor at x 0 keeps green · blue, drops red
        canvas.draw_fx_from(&lib, 0, 3, 0, 0);
        assert_color(canvas.image(), 0, 3, green);
        assert_color(canvas.image(), 1, 3, blue);
    }

    /// ★★ 2026-10-21 사방 할퀴기 + 야수조련 목걸이: scale 2 stretches the 1× window onto the doubled hit window. The pixel
    /// sitting on the inner edge (8 px forward of the feet) stays there; a uniform ×2 about the feet would move it to 16.
    #[test]
    fn test_wide_window_keeps_the_inner_tile_and_reaches_the_doubled_far_edge() {
        let (right, up, down, left) = (3u8, 0, 1, 2);
        assert_eq!(super::fx::wide_window(right, 8.0, -2.0), (8.0, -2.0));
        assert_eq!(super::fx::wide_window(right, 56.0, 38.0), (104.0, 70.0));
        assert_eq!(super::fx::wide_window(left, -56.0, -42.0), (-104.0, -74.0));
        assert_eq!(super::fx::wide_window(left, -8.0, 38.0), (-8.0, 70.0));
        assert_eq!(super::fx::wide_window(up, 0.0, -58.0).1, -106.0);
        assert_eq!(super::fx::wide_window(up, 0.0, -10.0).1, -10.0);
        assert_eq!(super::fx::wide_window(down, -40.0, 6.0), (-72.0, 6.0));
        assert_eq!(super::fx::wide_window(down, 40.0, 54.0), (72.0, 102.0));
        assert_eq!(super::fx::wide_window_inv(right, 104.0, 70.0), (56.0, 38.0));

        use image::{ImageEncoder, codecs::png::PngEncoder};
        let px = [255u8, 0, 0, 255];
        let mut png = alloc::vec::Vec::new();
        PngEncoder::new(&mut png).write_image(&px, 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        let mut b = alloc::vec::Vec::new();
        b.extend_from_slice(b"H2FX");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(png.len() as u32).to_le_bytes());
        b.extend_from_slice(&png);
        b.extend_from_slice(&1u32.to_le_bytes());
        for v in [0u16, 0, 1, 1, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(1);
        b.push(b'e');
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&8i16.to_le_bytes());
        b.extend_from_slice(&0i16.to_le_bytes());
        b.push(255);
        b.push(super::fx::FX_WIDE_WINDOW | (right << 4));
        let lib = super::FxLibrary::parse(&b).unwrap();
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(48, 80));
        canvas.fill_rect(0, 0, 48, 80, Color { a: 255, r: 0, g: 0, b: 0 }, full_clip(48));
        canvas.draw_fx_scaled(&lib, 10, 40, 0, 0, 2);
        let mut xs = alloc::vec::Vec::new();
        for y in 0..80 {
            for x in 0..48 {
                let p = canvas.image().get_pixel(x, y);
                if p.r > 200 {
                    xs.push(x);
                }
            }
        }
        assert!(!xs.is_empty(), "the stretched pixel is drawn");
        let mid = xs.iter().sum::<i32>() / xs.len() as i32;
        assert!(
            mid < 23,
            "the inner-edge pixel stays near x 19, not the uniform ×2 at x 27 (got {mid}, {xs:?})"
        );
    }

    /// ★★ 2026-10-21 용암불 + 야수조련 목걸이: the beast's tile edge stays, and the radius-2 ring's outer edge lands on the
    /// radius-4 ring. A uniform ×2 about the centre would move the inner edge out to 16 px and the outer edge to 80 px.
    #[test]
    fn test_lava_radial_keeps_the_beast_tile_and_reaches_radius_4() {
        assert_eq!(super::fx::lava_radial(8.0, 0.0), (8.0, 0.0));
        assert_eq!(super::fx::lava_radial(8.0, -32.0), (8.0, -64.0));
        assert_eq!(super::fx::lava_radial_inv(8.0, -64.0), (8.0, -32.0));
        assert_eq!(super::fx::lava_unpack(2 * 1000 + 500), (2, 0));
        assert_eq!(super::fx::lava_unpack(-2 * 1000 + 495), (-2, -5));
    }

    /// ★★ 2026-10-21 a necklace claw grows about the chest: scale 2 drops the picture 20px so the swing still leaves the body.
    /// An up or down slash instead doubles about its own centre and that centre's x is pulled onto the character.
    #[test]
    fn test_chest_shift_only_applies_when_the_claw_is_enlarged() {
        assert_eq!(super::fx::chest_shift(super::fx::FX_FROM_CHEST, 1), (0, 0));
        assert_eq!(super::fx::chest_shift(0, 2), (0, 0));
        assert_eq!(super::fx::chest_shift(super::fx::FX_FROM_CHEST, 2), (0, 20));
        let spr = super::fx::FxSprite {
            w: 1,
            h: 1,
            ax: 0,
            ay: 0,
            mass_x: -16,
            mass_y: -36,
            px: alloc::vec![[0, 0, 0, 0]],
            hd: None,
        };
        let up = super::fx::FX_FROM_CHEST | super::fx::FX_CLAW_UP;
        assert_eq!(super::fx::claw_shift(up, 1, &spr, 0, 0), (0, 0));
        // scale 2: x' = 2*(x - cx) so the centre lands on the character; y' keeps the centre
        assert_eq!(super::fx::claw_shift(up, 2, &spr, 0, 0), (32, 36));
        assert_eq!(super::fx::claw_shift(super::fx::FX_FROM_CHEST, 2, &spr, 0, 0), (0, 20));
    }

    /// `fx_blob` with frame 0's part at twice the size and frame 1's mirrored part too (`at` = `fx_blob().len()`).
    fn fx_blob_x2(mut b: alloc::vec::Vec<u8>, at: usize) -> alloc::vec::Vec<u8> {
        b[at - 19] = super::FX_X2;
        b[at - 9] = super::FX_FLIP_X | super::FX_X2;
        b
    }

    /// ★★ 2026-10-18 FX_X2: a part twice its size about its anchor — each sprite px a 2 × 2 block, the anchor px's block
    /// starting on the anchor (mirrored: the columns reversed about it); the compositor samples a twin at half its scale.
    #[test]
    fn test_fx_part_at_twice_the_size() {
        let _guard = HD_TEST_LOCK.lock();
        let lib = super::FxLibrary::parse(&fx_blob_x2(fx_blob(), fx_blob().len())).expect("parses");
        let black = Color { a: 255, r: 0, g: 0, b: 0 };
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(12, 6));
        canvas.fill_rect(0, 0, 12, 6, black, full_clip(12));
        canvas.draw_fx_from(&lib, 6, 2, 0, 0);
        for (x, c) in [(4, red), (5, red), (6, green), (7, green), (8, blue), (9, blue), (3, black), (10, black)] {
            assert_color(canvas.image(), x, 2, c);
        }
        assert_color(canvas.image(), 5, 3, red);
        assert_color(canvas.image(), 9, 3, blue);
        assert_color(canvas.image(), 6, 1, black);
        assert_color(canvas.image(), 6, 4, black);
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(12, 6));
        canvas.fill_rect(0, 0, 12, 6, black, full_clip(12));
        canvas.draw_fx_from(&lib, 6, 0, 0, 1);
        for (x, c) in [(4, blue), (5, blue), (6, green), (7, green), (8, red), (9, red)] {
            assert_color(canvas.image(), x, 1, c);
            assert_color(canvas.image(), x, 2, c);
        }
        assert_color(canvas.image(), 6, 3, black);
        let added = canvas.image().get_pixel(4, 0);
        assert_eq!((added.r, added.g, added.b), (128, 128, 128), "the plain part beside it as before");

        // the twin (scale 2) at twice the size is one twin px a guest px: guest x 2..4 red · 4..6 green · 6..8 blue, rows 1..3
        let lib = super::FxLibrary::parse(&fx_blob_x2(fx_blob_v2(), fx_blob().len())).expect("parses");
        let (w, h, k) = (8u32, 4u32, 2u32);
        let (ww, wh) = (w * k, h * k);
        let view = super::HdView {
            rect: (0, 0, ww, wh),
            src_w: w,
            src_y0: 0,
            src_h: h,
        };
        let hd = super::HdFrame {
            w,
            h,
            layer: alloc::vec![0; (w * h) as usize],
            draws: alloc::vec![super::HdDraw {
                effect: 0,
                age: 0,
                x: 4,
                y: 1,
                scale: 1,
                alpha: 255,
                group: 0
            }],
        };
        let mut dst = alloc::vec![0xff00_0000u32; (ww * wh) as usize];
        super::compose_hd(&lib, &hd, &mut dst, ww, wh, &view);
        let at = |x: u32, y: u32| dst[(y * ww + x) as usize] & 0xff_ffff;
        assert_eq!(
            (at(4, 2), at(7, 5), at(8, 2), at(11, 5), at(12, 2), at(15, 5)),
            (0xff0000, 0xff0000, 0x00ff00, 0x00ff00, 0x0000ff, 0x0000ff)
        );
        assert_eq!((at(3, 2), at(4, 1), at(4, 6)), (0, 0, 0), "nothing outside");
    }

    /// ★★ 2026-10-18 `FX_TAG | 2`: the whole effect at twice the size about the anchor — a part's offset doubled with its
    /// sprite (frame 1: the mirrored part one row down → two rows down, two px tall; the white two px left → four, 2 × 2);
    /// the record keeps the scale for the compositor; `| 1` and `| 0` as before, anything else 1×.
    #[test]
    fn test_fx_lines_armed_at_twice_the_size() {
        let _guard = HD_TEST_LOCK.lock();
        let lib = super::FxLibrary::parse(&fx_blob()).expect("parses");
        let black = Color { a: 255, r: 0, g: 0, b: 0 };
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(12, 6));
        canvas.fill_rect(0, 0, 12, 6, black, full_clip(12));
        canvas.draw_fx_scaled(&lib, 6, 0, 0, 1, 2);
        for y in [2, 3] {
            assert_color(canvas.image(), 4, y, blue);
            assert_color(canvas.image(), 9, y, red);
        }
        assert_color(canvas.image(), 4, 1, black);
        for (x, y) in [(2, 0), (3, 1)] {
            let p = canvas.image().get_pixel(x, y);
            assert_eq!((p.r, p.g, p.b), (128, 128, 128), "the white part four px left, 2 × 2");
        }
        assert_color(canvas.image(), 4, 0, black);

        super::set_hd_mode(true);
        super::hd_reset();
        canvas.draw_fx_scaled(&lib, 6, 0, 0, 1, 2);
        let raw = canvas.image().raw();
        let hd = super::hd_finish(&raw, 12, 6, 4).expect("a record");
        assert_eq!((hd.draws[0].scale, hd.draws[0].x, hd.draws[0].effect), (2, 6, 0));
        super::set_hd_mode(false);
        let (ww, wh) = (24u32, 12u32);
        let view = super::HdView {
            rect: (0, 0, ww, wh),
            src_w: 12,
            src_y0: 0,
            src_h: 6,
        };
        let mut dst = alloc::vec![0xff00_0000u32; (ww * wh) as usize];
        super::compose_hd(&lib, &hd, &mut dst, ww, wh, &view);
        let at = |x: u32, y: u32| dst[(y * ww + x) as usize] & 0xff_ffff;
        assert_eq!(
            (at(8, 4), at(9, 7), at(19, 4), at(8, 3)),
            (0x0000ff, 0x0000ff, 0xff0000, 0),
            "the compositor doubles it alike"
        );

        for (v, want) in [(1u8, 1u8), (2, 2), (0, 0), (7, 1)] {
            super::set_fx_scale(v);
            assert_eq!(super::fx_scale(), want);
        }
        super::set_fx_lines(false);
        assert!(!super::fx_lines());
    }
}
