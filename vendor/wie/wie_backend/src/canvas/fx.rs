//! ★ A host EFFECT LIBRARY for `draw_line` (2026-10-14, hero2's 「악마화」 effect renewal).
//!
//! A host block that knows this emulator asks for whole pre-drawn effects instead of lines: it arms the mode with
//! `setAlpha(FX_TAG | 1)`, then every `drawLine(x1, y1, x2, y2)` draws frame `y2 − y1` of effect `x2 − x1` with its
//! anchor at `(x1, y1)` (the differences are what the call carries — the Graphics translation moves both ends alike),
//! and `setAlpha(FX_TAG | 0)` puts plain lines back. The paint event disarms it like the stroke, so it never outlives a
//! frame. The game itself never draws a line (hero2's image has no `drawLine` reference word), so nothing of the game
//! can reach this path. ★★ 2026-10-18 `setAlpha(FX_TAG | 2)` arms them at twice the size: every part's offset and sprite
//! doubled about the anchor (hero2's 야수조련 목걸이 doubles a skill's reach, the picture follows).
//!
//! The library is DATA the host registers once (`set_fx_library`) — a PNG atlas cut into sprites (straight RGBA, an
//! anchor each) and effects made of frames made of parts `(sprite, dx, dy, alpha, flags)` — so the pictures can be
//! redrawn without touching a guest byte. An effect or frame the library does not have draws nothing; the host draw
//! alpha (`DRAW_ALPHA_TAG`) scales every part.
//!
//! ```text
//! "H2FX" · u32 version (1 | 2) · u32 png_len · png
//! u32 sprites · { u16 x · u16 y · u16 w · u16 h · i16 ax · i16 ay } × sprites      (the atlas rect and the anchor in it)
//! u32 effects · { u8 name_len · name · u32 frames · { u16 parts · { u16 sprite · i16 dx · i16 dy · u8 alpha · u8 flags } × parts } × frames } × effects
//! ★ version 2 (2026-10-15, the HD twins — see below):
//! u32 hd_png_len · hd png (8-bit grey = colour indices) · u32 hd ·
//!   { u16 sprite · u8 scale · u8 colours · u16 x · u16 y · u16 w · u16 h · i16 ax · i16 ay · [u8; 4] × colours } × hd
//! ```
//! (all little-endian; `flags` bit 0 = mirrored about the anchor column · bit 1 = additive · ★★ 2026-10-18 bit 2 = the
//! sprite at twice its size about its anchor — a twin at twice its own scale — so an effect can be a bigger copy of
//! another without a pixel of its own.)
//!
//! ★★ HD (2026-10-15, the user: *"이펙트와 외형 전부 업스케일링"*). A sprite may carry a TWIN drawn at `scale` sprite px
//! per guest px (the reference art is the game at 3×) — indexed colour (index 0 = transparent, the rest a palette of
//! straight RGBA per sprite) so the twins cost a byte a pixel. The guest-resolution draw never uses a twin: the 1× sprite
//! is what the framebuffer gets, exactly as before. The twins are for the HD MODE (`set_hd_mode`): under it an effect draw
//! RECORDS `(effect, age, x, y, alpha)` instead of touching the framebuffer, and the host composites the twins at the
//! WINDOW's resolution over the finished frame (`compose_hd`). The order the game paints in is kept by a LAYER MAP: every
//! guest draw since the last recorded effect ends a group (`hd_touch`), the recorder compares the framebuffer with its
//! snapshot and marks the pixels the guest painted since with the new group's number; a recorded effect of group `g` is
//! composited only over pixels whose layer is `≤ g` — what the game drew after it (a monster in front of the demon)
//! stays in front, what it drew before (the tiles, a monster behind) goes under. The paint event hands the record over
//! (`hd_finish`) with the frame it belongs to and starts a new one.

use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// `setAlpha` arguments whose upper 16 bits are this arm (`| 1` · `| 2` at twice the size) or disarm (`| 0`) the effect lines.
pub const FX_TAG: u32 = 0x4835_0000;

pub const FX_FLIP_X: u8 = 1;
pub const FX_ADD: u8 = 2;
pub const FX_X2: u8 = 4;
/// A 4-direction claw part. With a draw scale of 2 (야수조련 목걸이) the part is not doubled about the feet: its
/// 1× window is stretched onto the doubled hit window, so the tile next to the body stays put and the far tile
/// lands on the far tile. Bits 4..5 are the direction (0 up · 1 down · 2 left · 3 right). Ignored at scale 1.
pub const FX_WIDE_WINDOW: u8 = 8;
/// A lava piece of the necklace's doubled ring. `dx`/`dy` pack the ring tile and the 1× pixel offset
/// ([`lava_unpack`]). Drawn at the ring tile's top-left, each source pixel is moved out from the beast's tile edge
/// (8 px from the tile centre) onto the doubled ring, whose far edge is the outer edge of the radius-4 tiles.
pub const FX_LAVA_RADIAL: u8 = 64;
/// A claw part. At scale 2 a side swing grows about the chest (20px above the feet). An up or down swing
/// ([`FX_CLAW_UP`] · [`FX_CLAW_DOWN`]) doubles about its own centre and that centre is pulled onto the character,
/// so the slash stays where it leaves the body. Ignored at scale 1.
pub const FX_FROM_CHEST: u8 = 128;
/// Bits 4..5, with [`FX_FROM_CHEST`]: 1 = the up claw, 2 = the down claw. 0 = a side claw.
pub const FX_CLAW_UP: u8 = 16;
pub const FX_CLAW_DOWN: u8 = 32;
pub const CLAW_CHEST_Y: i32 = -20;

/// 0 = plain lines · 1 = effects · 2 = effects at twice the size.
static FX_SCALE: AtomicU8 = AtomicU8::new(0);

pub fn set_fx_lines(on: bool) {
    set_fx_scale(u8::from(on));
}

/// The arm from `setAlpha(FX_TAG | v)`: 0 off, 2 twice the size, anything else 1×.
pub fn set_fx_scale(v: u8) {
    FX_SCALE.store(if v == 0 || v == 2 { v } else { 1 }, Ordering::Relaxed);
}

pub fn fx_lines() -> bool {
    fx_scale() != 0
}

pub fn fx_scale() -> u8 {
    FX_SCALE.load(Ordering::Relaxed)
}

/// Scale 2 and [`FX_WIDE_WINDOW`]: stretch this part's 1× window onto the doubled one.
pub fn wide_window_on(flags: u8, scale: u8) -> bool {
    scale >= 2 && flags & FX_WIDE_WINDOW != 0
}

pub fn lava_radial_on(flags: u8) -> bool {
    flags & FX_LAVA_RADIAL != 0
}

/// Extra screen offset that turns a scale-`s` draw about the feet into one about the chest. `(0, 0)` at scale 1.
pub fn chest_shift(flags: u8, scale: i32) -> (i32, i32) {
    if scale <= 1 || flags & FX_FROM_CHEST == 0 {
        (0, 0)
    } else {
        (0, (1 - scale) * CLAW_CHEST_Y)
    }
}

fn claw_vertical(flags: u8) -> bool {
    flags & (FX_CLAW_UP | FX_CLAW_DOWN) != 0
}

/// The translation added after a uniform scale about the feet. Side claws drop to the chest. Up and down claws
/// double about the slash's own centre (`mass_x`/`mass_y`, feet-relative) and that centre's x is pulled onto the
/// character, so the picture stays where the 1× slash leaves the body.
pub fn claw_shift(flags: u8, scale: i32, spr: &FxSprite, dx: i32, dy: i32) -> (i32, i32) {
    if scale <= 1 || flags & FX_FROM_CHEST == 0 {
        return (0, 0);
    }
    if claw_vertical(flags) {
        let cx = spr.mass_x + dx;
        let cy = spr.mass_y + dy;
        return (-scale * cx, (1 - scale) * cy);
    }
    chest_shift(flags, scale)
}

/// Centre of the pixels at alpha ≥ 80, in guest px from the anchor. `(0, 0)` when the sprite is empty.
fn sprite_mass(px: &[[u8; 4]], w: i32, ax: i32, ay: i32) -> (i32, i32) {
    if w <= 0 {
        return (0, 0);
    }
    let mut n = 0i64;
    let mut sx = 0i64;
    let mut sy = 0i64;
    for (i, p) in px.iter().enumerate() {
        if p[3] < 80 {
            continue;
        }
        let i = i as i32;
        sx += i64::from(i % w - ax);
        sy += i64::from(i / w - ay);
        n += 1;
    }
    if n == 0 {
        return (0, 0);
    }
    (div_round(sx, n), div_round(sy, n))
}

fn div_round(s: i64, n: i64) -> i32 {
    let half = n / 2;
    let s = if s >= 0 { s + half } else { s - half };
    (s / n) as i32
}

pub fn wide_dir(flags: u8) -> u8 {
    (flags >> 4) & 3
}

/// One axis of a window, in guest px from the feet: the 1× span `[s0, s1]` maps onto the doubled span `[d0, d1]`.
struct WinAxis {
    s0: f32,
    s1: f32,
    d0: f32,
    d1: f32,
}

fn map_axis(v: f32, a: WinAxis) -> f32 {
    a.d0 + (v - a.s0) * (a.d1 - a.d0) / (a.s1 - a.s0)
}

fn unmap_axis(v: f32, a: WinAxis) -> f32 {
    a.s0 + (v - a.d0) * (a.s1 - a.s0) / (a.d1 - a.d0)
}

/// The claw windows in guest px from the feet (`DemonPatch::FEET` (8, 10), a tile is 16 px). A stage-0/1 window is
/// 3 tiles forward by ±2 aside; the necklace doubles it to 6 by ±4. The inner edge (the first tile out from the body)
/// is the same in both, so a uniform ×2 about the feet — which moves that edge out by a tile — does not match.
fn wide_axes(dir: u8) -> (WinAxis, WinAxis) {
    let side_x = WinAxis {
        s0: -40.0,
        s1: 40.0,
        d0: -72.0,
        d1: 72.0,
    };
    let side_y = WinAxis {
        s0: -42.0,
        s1: 38.0,
        d0: -74.0,
        d1: 70.0,
    };
    match dir {
        0 => (
            side_x,
            WinAxis {
                s0: -58.0,
                s1: -10.0,
                d0: -106.0,
                d1: -10.0,
            },
        ),
        1 => (
            side_x,
            WinAxis {
                s0: 6.0,
                s1: 54.0,
                d0: 6.0,
                d1: 102.0,
            },
        ),
        2 => (
            WinAxis {
                s0: -56.0,
                s1: -8.0,
                d0: -104.0,
                d1: -8.0,
            },
            side_y,
        ),
        _ => (
            WinAxis {
                s0: 8.0,
                s1: 56.0,
                d0: 8.0,
                d1: 104.0,
            },
            side_y,
        ),
    }
}

/// A feet-relative point on the 1× four-direction window, moved onto the doubled window. `dir` is [`wide_dir`].
pub fn wide_window(dir: u8, x: f32, y: f32) -> (f32, f32) {
    let (h, v) = wide_axes(dir);
    (map_axis(x, h), map_axis(y, v))
}

/// The inverse of [`wide_window`].
pub fn wide_window_inv(dir: u8, x: f32, y: f32) -> (f32, f32) {
    let (h, v) = wide_axes(dir);
    (unmap_axis(x, h), unmap_axis(y, v))
}

/// Sprite pixel holding the feet-relative point `fx, fy` (the part's own dx, dy included), or `None` outside it.
pub fn wide_sprite_px(dir: u8, flip: bool, ax: i32, ay: i32, dx: i32, dy: i32, w: i32, h: i32, fx: f32, fy: f32) -> Option<(i32, i32)> {
    let (sx, sy) = wide_window_inv(dir, fx, fy);
    let rel_x = sx - dx as f32;
    let rel_y = sy - dy as f32;
    let px = if flip { ax as f32 - rel_x } else { ax as f32 + rel_x };
    let py = ay as f32 + rel_y;
    let (ix, iy) = (px.floor() as i32, py.floor() as i32);
    (0..w).contains(&ix).then_some(())?;
    (0..h).contains(&iy).then_some(())?;
    Some((ix, iy))
}

/// `dx`/`dy` of an [`FX_LAVA_RADIAL`] part → (ring tile, 1× pixel offset). Pixel offsets sit in −500..499.
pub fn lava_unpack(v: i32) -> (i32, i32) {
    (v.div_euclid(1000), v.rem_euclid(1000) - 500)
}

/// A point relative to the beast's tile top-left, moved onto the doubled lava ring. Inside the beast's own tile
/// (8 px from its centre) it stays; the outer edge of the radius-2 ring (40 px) lands on the outer edge of the
/// radius-4 ring (72 px). A uniform ×2 about the centre would move the inner edge out by a tile and the outer
/// edge half a tile past the hits.
pub fn lava_radial(x: f32, y: f32) -> (f32, f32) {
    let (dx, dy) = (x - 8.0, y - 8.0);
    let r = (dx * dx + dy * dy).sqrt();
    if r <= 8.0 {
        return (x, y);
    }
    let s = (r * 2.0 - 8.0) / r;
    (8.0 + dx * s, 8.0 + dy * s)
}

/// The inverse of [`lava_radial`].
pub fn lava_radial_inv(x: f32, y: f32) -> (f32, f32) {
    let (dx, dy) = (x - 8.0, y - 8.0);
    let rp = (dx * dx + dy * dy).sqrt();
    if rp <= 8.0 {
        return (x, y);
    }
    let s = (rp + 8.0) * 0.5 / rp;
    (8.0 + dx * s, 8.0 + dy * s)
}

/// Sprite pixel under a destination pixel whose centre is `(sx, sy)` on screen. The part is drawn at the ring
/// tile's top-left `(x, y)`.
pub fn lava_sprite_px(x: i32, y: i32, dx: i32, dy: i32, ax: i32, ay: i32, w: i32, h: i32, flip: bool, sx: f32, sy: f32) -> Option<(i32, i32)> {
    let (ring_dx, pix_dx) = lava_unpack(dx);
    let (ring_dy, pix_dy) = lava_unpack(dy);
    let (wx, wy) = lava_radial_inv(sx - x as f32 + ring_dx as f32 * 16.0, sy - y as f32 + ring_dy as f32 * 16.0);
    let rel_x = wx - ring_dx as f32 * 16.0 - pix_dx as f32;
    let rel_y = wy - ring_dy as f32 * 16.0 - pix_dy as f32;
    let px = if flip { ax as f32 - rel_x } else { ax as f32 + rel_x };
    let py = ay as f32 + rel_y;
    let (ix, iy) = (px.floor() as i32, py.floor() as i32);
    (0..w).contains(&ix).then_some(())?;
    (0..h).contains(&iy).then_some(())?;
    Some((ix, iy))
}

fn lava_screen(x: i32, y: i32, ring_dx: i32, ring_dy: i32, wx: f32, wy: f32) -> (f32, f32) {
    let (mx, my) = lava_radial(wx, wy);
    (x as f32 + mx - ring_dx as f32 * 16.0, y as f32 + my - ring_dy as f32 * 16.0)
}

/// Guest-pixel box (inclusive) of a lava part stretched onto the doubled ring, drawn at the ring tile's top-left.
pub fn lava_dest_box(x: i32, y: i32, dx: i32, dy: i32, ax: i32, ay: i32, w: i32, h: i32, flip: bool) -> (i32, i32, i32, i32) {
    let (ring_dx, pix_dx) = lava_unpack(dx);
    let (ring_dy, pix_dy) = lava_unpack(dy);
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for sx in [0.0, w as f32] {
        for sy in [0.0, h as f32] {
            let rel_x = if flip { ax as f32 - sx } else { sx - ax as f32 };
            let rel_y = sy - ay as f32;
            let (sx, sy) = lava_screen(
                x,
                y,
                ring_dx,
                ring_dy,
                ring_dx as f32 * 16.0 + pix_dx as f32 + rel_x,
                ring_dy as f32 * 16.0 + pix_dy as f32 + rel_y,
            );
            min_x = min_x.min(sx);
            min_y = min_y.min(sy);
            max_x = max_x.max(sx);
            max_y = max_y.max(sy);
        }
    }
    (min_x.floor() as i32, min_y.floor() as i32, max_x.ceil() as i32, max_y.ceil() as i32)
}

/// Guest-pixel box (inclusive) of a part stretched onto the doubled window, around the feet at `(x, y)`.
pub fn wide_dest_box(x: i32, y: i32, ax: i32, ay: i32, dx: i32, dy: i32, w: i32, h: i32, flip: bool, dir: u8) -> (i32, i32, i32, i32) {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for sx in [0.0, w as f32] {
        for sy in [0.0, h as f32] {
            let rel_x = if flip { ax as f32 - sx } else { sx - ax as f32 };
            let rel_y = sy - ay as f32;
            let (mx, my) = wide_window(dir, dx as f32 + rel_x, dy as f32 + rel_y);
            min_x = min_x.min(mx);
            min_y = min_y.min(my);
            max_x = max_x.max(mx);
            max_y = max_y.max(my);
        }
    }
    (
        x + min_x.floor() as i32,
        y + min_y.floor() as i32,
        x + max_x.ceil() as i32,
        y + max_y.ceil() as i32,
    )
}

/// ★ HD: a sprite's twin at `scale` sprite px per guest px — indexed colour (0 = transparent) over `pal` (straight RGBA).
pub struct FxHd {
    pub scale: u8,
    pub w: u32,
    pub h: u32,
    /// The anchor in TWIN pixels (the 1× anchor × scale, as the generator wrote it).
    pub ax: i32,
    pub ay: i32,
    pub idx: Vec<u8>,
    pub pal: Vec<[u8; 4]>,
}

pub struct FxSprite {
    pub w: u32,
    pub h: u32,
    pub ax: i32,
    pub ay: i32,
    /// The bright pixels' centre, in guest px from the anchor (the slash, not the faint dust).
    pub mass_x: i32,
    pub mass_y: i32,
    /// Straight RGBA, row-major.
    pub px: Vec<[u8; 4]>,
    /// ★ HD: the twin, when the library carries one (version 2).
    pub hd: Option<FxHd>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxPart {
    pub sprite: u16,
    pub dx: i16,
    pub dy: i16,
    pub alpha: u8,
    pub flags: u8,
}

pub struct FxEffect {
    pub name: String,
    pub frames: Vec<Vec<FxPart>>,
}

pub struct FxLibrary {
    pub sprites: Vec<FxSprite>,
    pub effects: Vec<FxEffect>,
}

static FX_LIB: spin::RwLock<Option<Arc<FxLibrary>>> = spin::RwLock::new(None);

/// Register the library (the last one wins). `Ok((sprites, effects))`, or why the bytes were refused (nothing changes).
pub fn set_fx_library(bytes: &[u8]) -> core::result::Result<(usize, usize), &'static str> {
    let lib = FxLibrary::parse(bytes)?;
    let counts = (lib.sprites.len(), lib.effects.len());
    *FX_LIB.write() = Some(Arc::new(lib));
    Ok(counts)
}

pub fn fx_library() -> Option<Arc<FxLibrary>> {
    FX_LIB.read().clone()
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> core::result::Result<&'a [u8], &'static str> {
        let end = self.at.checked_add(n).ok_or("fx: length overflow")?;
        let s = self.b.get(self.at..end).ok_or("fx: truncated")?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> core::result::Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> core::result::Result<u16, &'static str> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn i16(&mut self) -> core::result::Result<i16, &'static str> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> core::result::Result<u32, &'static str> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

impl FxLibrary {
    pub fn parse(bytes: &[u8]) -> core::result::Result<Self, &'static str> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(4)? != b"H2FX" {
            return Err("fx: not an H2FX library");
        }
        let version = r.u32()?;
        if version != 1 && version != 2 {
            return Err("fx: unknown version");
        }
        let png_len = r.u32()? as usize;
        let png = r.take(png_len)?;
        let atlas = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .map_err(|_| "fx: the atlas does not decode")?
            .to_rgba8();
        let (aw, ah) = (atlas.width(), atlas.height());
        let n = r.u32()? as usize;
        if n > 0xffff {
            return Err("fx: too many sprites");
        }
        let mut sprites = Vec::with_capacity(n);
        for _ in 0..n {
            let (x, y, w, h) = (r.u16()? as u32, r.u16()? as u32, r.u16()? as u32, r.u16()? as u32);
            let (ax, ay) = (r.i16()? as i32, r.i16()? as i32);
            if x + w > aw || y + h > ah {
                return Err("fx: a sprite lies outside the atlas");
            }
            let mut px = Vec::with_capacity((w * h) as usize);
            for yy in y..y + h {
                for xx in x..x + w {
                    px.push(atlas.get_pixel(xx, yy).0);
                }
            }
            let (mass_x, mass_y) = sprite_mass(&px, w as i32, ax, ay);
            sprites.push(FxSprite { w, h, ax, ay, mass_x, mass_y, px, hd: None });
        }
        let ne = r.u32()? as usize;
        if ne > 0xffff {
            return Err("fx: too many effects");
        }
        let mut effects = Vec::with_capacity(ne);
        for _ in 0..ne {
            let nl = r.u8()? as usize;
            let name = String::from_utf8_lossy(r.take(nl)?).into_owned();
            let nf = r.u32()? as usize;
            if nf > 0x1_0000 {
                return Err("fx: too many frames");
            }
            let mut frames = Vec::with_capacity(nf);
            for _ in 0..nf {
                let np = r.u16()? as usize;
                let mut parts = Vec::with_capacity(np);
                for _ in 0..np {
                    let part = FxPart {
                        sprite: r.u16()?,
                        dx: r.i16()?,
                        dy: r.i16()?,
                        alpha: r.u8()?,
                        flags: r.u8()?,
                    };
                    if part.sprite as usize >= sprites.len() {
                        return Err("fx: a part names a sprite past the list");
                    }
                    parts.push(part);
                }
                frames.push(parts);
            }
            effects.push(FxEffect { name, frames });
        }
        if version == 2 {
            // ★ HD: the twins — an 8-bit grey PNG of colour indices and a palette per twin
            let hd_png_len = r.u32()? as usize;
            let hd_png = r.take(hd_png_len)?;
            let hd_atlas = if hd_png_len == 0 {
                None
            } else {
                Some(
                    image::load_from_memory_with_format(hd_png, image::ImageFormat::Png)
                        .map_err(|_| "fx: the HD atlas does not decode")?
                        .to_luma8(),
                )
            };
            let nh = r.u32()? as usize;
            if nh > sprites.len() {
                return Err("fx: more twins than sprites");
            }
            for _ in 0..nh {
                let sprite = r.u16()? as usize;
                let scale = r.u8()?;
                let ncol = r.u8()? as usize;
                let (x, y, w, h) = (r.u16()? as u32, r.u16()? as u32, r.u16()? as u32, r.u16()? as u32);
                let (ax, ay) = (r.i16()? as i32, r.i16()? as i32);
                let mut pal = Vec::with_capacity(ncol + 1);
                pal.push([0, 0, 0, 0]);
                for _ in 0..ncol {
                    let c = r.take(4)?;
                    pal.push([c[0], c[1], c[2], c[3]]);
                }
                let Some(atlas) = hd_atlas.as_ref() else {
                    return Err("fx: a twin without an HD atlas");
                };
                if sprite >= sprites.len() || scale == 0 {
                    return Err("fx: a twin names no sprite or has no scale");
                }
                if x + w > atlas.width() || y + h > atlas.height() {
                    return Err("fx: a twin lies outside the HD atlas");
                }
                let mut idx = Vec::with_capacity((w * h) as usize);
                for yy in y..y + h {
                    for xx in x..x + w {
                        let v = atlas.get_pixel(xx, yy).0[0];
                        if usize::from(v) > ncol {
                            return Err("fx: a twin's pixel names a colour past its palette");
                        }
                        idx.push(v);
                    }
                }
                sprites[sprite].hd = Some(FxHd {
                    scale,
                    w,
                    h,
                    ax,
                    ay,
                    idx,
                    pal,
                });
            }
        }
        if r.at != bytes.len() {
            return Err("fx: trailing bytes");
        }
        Ok(Self { sprites, effects })
    }

    /// The parts of frame `age` of `effect` (`None` = nothing to draw).
    pub fn frame(&self, effect: i32, age: i32) -> Option<&[FxPart]> {
        if effect < 0 || age < 0 {
            return None;
        }
        self.effects.get(effect as usize)?.frames.get(age as usize).map(|f| f.as_slice())
    }

    /// How many sprites carry a twin, and the twins' pixels in all.
    pub fn hd_stats(&self) -> (usize, usize) {
        let twins: Vec<&FxHd> = self.sprites.iter().filter_map(|s| s.hd.as_ref()).collect();
        (twins.len(), twins.iter().map(|t| t.idx.len()).sum())
    }
}

// ── ★★ HD mode: the record and the layer map ───────────────────────────────────────────────────────────────────────

/// One recorded effect draw: frame `age` of `effect` anchored at guest `(x, y)` at `scale` × its size (1 · 2), under the
/// host draw alpha `alpha`, in paint-order group `group` (composited over the guest pixels whose layer is `≤ group`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HdDraw {
    pub effect: i32,
    pub age: i32,
    pub x: i32,
    pub y: i32,
    pub scale: u8,
    pub alpha: u8,
    pub group: u8,
}

/// The effects a paint recorded instead of drawing, with the LAYER MAP of its framebuffer: `layer[y * w + x]` = the
/// group after which the guest last painted that pixel (0 = before any recorded draw).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HdFrame {
    pub w: u32,
    pub h: u32,
    pub layer: Vec<u8>,
    pub draws: Vec<HdDraw>,
}

struct HdRecorder {
    w: u32,
    h: u32,
    bpp: u32,
    group: u8,
    /// The framebuffer's raw bytes as they were when the current group began.
    snap: Vec<u8>,
    layer: Vec<u8>,
    draws: Vec<HdDraw>,
}

static HD_MODE: AtomicBool = AtomicBool::new(false);
static HD_DIRTY: AtomicBool = AtomicBool::new(false);
static HD_REC: spin::Mutex<Option<HdRecorder>> = spin::Mutex::new(None);

/// A paint may record this many effect draws at most (a runaway recorder is dropped rather than grown).
const HD_DRAWS_MAX: usize = 4096;

/// HD mode on: effect draws are recorded for `compose_hd`, the guest framebuffer gets none of them.
pub fn set_hd_mode(on: bool) {
    HD_MODE.store(on, Ordering::Relaxed);
    if !on {
        *HD_REC.lock() = None;
    }
}

pub fn hd_mode() -> bool {
    HD_MODE.load(Ordering::Relaxed)
}

/// The guest painted something (every canvas primitive but the effect lines says so): the next recorded draw begins a
/// new group.
#[inline]
pub fn hd_touch() {
    if HD_MODE.load(Ordering::Relaxed) {
        HD_DIRTY.store(true, Ordering::Relaxed);
    }
}

/// The paint event starts: whatever an unfinished paint left is dropped.
pub fn hd_reset() {
    if HD_MODE.load(Ordering::Relaxed) {
        *HD_REC.lock() = None;
        HD_DIRTY.store(false, Ordering::Relaxed);
    }
}

/// Mark the pixels of `now` that differ from `snap` with `group` (bpp bytes a pixel), then keep `now` as the snapshot.
fn hd_advance(rec: &mut HdRecorder, now: &[u8], group: u8) {
    let bpp = rec.bpp as usize;
    if bpp == 0 || now.len() != rec.snap.len() {
        return;
    }
    for (i, (a, b)) in now.chunks_exact(bpp).zip(rec.snap.chunks_exact(bpp)).enumerate() {
        if a != b {
            if let Some(l) = rec.layer.get_mut(i) {
                *l = group;
            }
        }
    }
    rec.snap.copy_from_slice(now);
}

/// Record an effect draw under HD mode. `raw` = the framebuffer's bytes now (`w × h`, `bpp` a pixel).
#[allow(clippy::too_many_arguments)]
pub fn hd_record(raw: &[u8], w: u32, h: u32, bpp: u32, effect: i32, age: i32, x: i32, y: i32, scale: u8, alpha: u8) {
    let mut guard = HD_REC.lock();
    let fresh = match guard.as_ref() {
        Some(r) => r.w != w || r.h != h || r.bpp != bpp || r.snap.len() != raw.len() || r.draws.len() >= HD_DRAWS_MAX,
        None => true,
    };
    if fresh {
        *guard = Some(HdRecorder {
            w,
            h,
            bpp,
            group: 0,
            snap: raw.to_vec(),
            layer: vec![0; (w * h) as usize],
            draws: Vec::with_capacity(64),
        });
        HD_DIRTY.store(false, Ordering::Relaxed);
    } else if HD_DIRTY.swap(false, Ordering::Relaxed) {
        let rec = guard.as_mut().expect("the recorder is there");
        let group = rec.group.saturating_add(1).min(254);
        hd_advance(rec, raw, group);
        rec.group = group;
    }
    let rec = guard.as_mut().expect("the recorder is there");
    rec.draws.push(HdDraw {
        effect,
        age,
        x,
        y,
        scale,
        alpha,
        group: rec.group,
    });
}

/// The paint is done: the record with its layer map (the pixels the guest painted after the last recorded draw get the
/// group past the last), or `None` when nothing was recorded. The recorder starts over.
pub fn hd_finish(raw: &[u8], w: u32, h: u32, bpp: u32) -> Option<HdFrame> {
    let mut rec = HD_REC.lock().take()?;
    HD_DIRTY.store(false, Ordering::Relaxed);
    if rec.w != w || rec.h != h || rec.bpp != bpp || rec.draws.is_empty() {
        return None;
    }
    let last = rec.group.saturating_add(1).min(255);
    hd_advance(&mut rec, raw, last);
    Some(HdFrame {
        w,
        h,
        layer: rec.layer,
        draws: rec.draws,
    })
}

/// Where the guest frame sits in the window: the window rect it fills, the guest frame's width, and the rows shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HdView {
    pub rect: (i32, i32, u32, u32),
    pub src_w: u32,
    pub src_y0: u32,
    pub src_h: u32,
}

#[inline]
fn hd_mix(d: u32, shift: u32, s: u8, sa: u32, add: bool) -> u32 {
    let dv = (d >> shift) & 0xff;
    let sv = u32::from(s);
    let v = if add {
        (dv + sv * sa / 255).min(255)
    } else {
        (sv * sa + dv * (255 - sa) + 127) / 255
    };
    (d & !(0xff << shift)) | (v << shift)
}

/// One [`FX_LAVA_RADIAL`] part. The guest point under each window pixel is mapped back through [`lava_radial_inv`].
fn compose_lava(
    spr: &FxSprite,
    part: &FxPart,
    alpha: u32,
    flip: bool,
    add: bool,
    draw: &HdDraw,
    layer: &[u8],
    hd_w: i32,
    hd_h: i32,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    view: &HdView,
) {
    let (rx, ry, rw, rh) = view.rect;
    if rw == 0 || rh == 0 {
        return;
    }
    let gpx = view.src_w as f32 / rw as f32;
    let gpy = view.src_h as f32 / rh as f32;
    let (gx0, gy0, gx1, gy1) = lava_dest_box(
        draw.x,
        draw.y,
        i32::from(part.dx),
        i32::from(part.dy),
        spr.ax,
        spr.ay,
        spr.w as i32,
        spr.h as i32,
        flip,
    );
    let wx0 = (rx as f32 + gx0 as f32 / gpx).floor() as i32;
    let wx1 = (rx as f32 + (gx1 as f32 + 1.0) / gpx).ceil() as i32;
    let src_y0 = view.src_y0 as i32;
    let wy0 = (ry as f32 + (gy0 - src_y0) as f32 / gpy).floor() as i32;
    let wy1 = (ry as f32 + (gy1 - src_y0) as f32 / gpy).ceil() as i32;
    let (wx0, wx1) = (wx0.max(rx.max(0)), wx1.min((rx + rw as i32).min(dst_w as i32)));
    let (wy0, wy1) = (wy0.max(ry.max(0)), wy1.min((ry + rh as i32).min(dst_h as i32)));
    if wx0 >= wx1 || wy0 >= wy1 {
        return;
    }
    let (ring_dx, pix_dx) = lava_unpack(i32::from(part.dx));
    let (ring_dy, pix_dy) = lava_unpack(i32::from(part.dy));
    let (ts, tax, tay) = match &spr.hd {
        Some(t) => (f32::from(t.scale), t.ax as f32, t.ay as f32),
        None => (1.0, spr.ax as f32, spr.ay as f32),
    };
    for wy in wy0..wy1 {
        let v = view.src_y0 as f32 + (wy - ry) as f32 * gpy + gpy * 0.5;
        let gy = v.floor() as i32;
        let drow = (wy as u32 * dst_w) as usize;
        for wx in wx0..wx1 {
            let u = (wx - rx) as f32 * gpx + gpx * 0.5;
            let gx = u.floor() as i32;
            if gx < 0 || gy < 0 || gx >= hd_w || gy >= hd_h || layer[(gy * hd_w + gx) as usize] > draw.group {
                continue;
            }
            let (sx, sy) = lava_radial_inv(u - draw.x as f32 + ring_dx as f32 * 16.0, v - draw.y as f32 + ring_dy as f32 * 16.0);
            let rel_x = sx - ring_dx as f32 * 16.0 - pix_dx as f32;
            let rel_y = sy - ring_dy as f32 * 16.0 - pix_dy as f32;
            let (tx, ty) = if spr.hd.is_some() {
                let tx = if flip { tax - rel_x * ts } else { tax + rel_x * ts };
                (tx.floor() as i32, (tay + rel_y * ts).floor() as i32)
            } else {
                let px = if flip { spr.ax as f32 - rel_x } else { spr.ax as f32 + rel_x };
                (px.floor() as i32, (spr.ay as f32 + rel_y).floor() as i32)
            };
            let p: [u8; 4] = if let Some(t) = &spr.hd {
                if !(0..t.w as i32).contains(&tx) || !(0..t.h as i32).contains(&ty) {
                    continue;
                }
                t.pal[usize::from(t.idx[(ty as u32 * t.w + tx as u32) as usize])]
            } else if !(0..spr.w as i32).contains(&tx) || !(0..spr.h as i32).contains(&ty) {
                continue;
            } else {
                spr.px[(ty as u32 * spr.w + tx as u32) as usize]
            };
            let sa = u32::from(p[3]) * alpha / 255;
            if sa == 0 {
                continue;
            }
            let di = drow + wx as usize;
            let mut d = dst[di];
            d = hd_mix(d, 16, p[0], sa, add);
            d = hd_mix(d, 8, p[1], sa, add);
            d = hd_mix(d, 0, p[2], sa, add);
            dst[di] = d | 0xff00_0000;
        }
    }
}

/// One [`FX_WIDE_WINDOW`] part at scale 2. The guest point under each window pixel is mapped back onto the 1× sprite
/// (or its twin) through [`wide_window_inv`].
fn compose_wide(
    spr: &FxSprite,
    part: &FxPart,
    alpha: u32,
    flip: bool,
    add: bool,
    draw: &HdDraw,
    layer: &[u8],
    hd_w: i32,
    hd_h: i32,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    view: &HdView,
) {
    let (rx, ry, rw, rh) = view.rect;
    if rw == 0 || rh == 0 {
        return;
    }
    let gpx = view.src_w as f32 / rw as f32;
    let gpy = view.src_h as f32 / rh as f32;
    let dir = wide_dir(part.flags);
    let (gx0, gy0, gx1, gy1) = wide_dest_box(
        draw.x,
        draw.y,
        spr.ax,
        spr.ay,
        i32::from(part.dx),
        i32::from(part.dy),
        spr.w as i32,
        spr.h as i32,
        flip,
        dir,
    );
    let wx0 = (rx as f32 + gx0 as f32 / gpx).floor() as i32;
    let wx1 = (rx as f32 + (gx1 as f32 + 1.0) / gpx).ceil() as i32;
    let src_y0 = view.src_y0 as i32;
    let wy0 = (ry as f32 + (gy0 - src_y0) as f32 / gpy).floor() as i32;
    let wy1 = (ry as f32 + (gy1 - src_y0) as f32 / gpy).ceil() as i32;
    let (wx0, wx1) = (wx0.max(rx.max(0)), wx1.min((rx + rw as i32).min(dst_w as i32)));
    let (wy0, wy1) = (wy0.max(ry.max(0)), wy1.min((ry + rh as i32).min(dst_h as i32)));
    if wx0 >= wx1 || wy0 >= wy1 {
        return;
    }
    let (ts, tax, tay) = match &spr.hd {
        Some(t) => (f32::from(t.scale), t.ax as f32, t.ay as f32),
        None => (1.0, spr.ax as f32, spr.ay as f32),
    };
    for wy in wy0..wy1 {
        let v = view.src_y0 as f32 + (wy - ry) as f32 * gpy + gpy * 0.5;
        let gy = v.floor() as i32;
        let drow = (wy as u32 * dst_w) as usize;
        for wx in wx0..wx1 {
            let u = (wx - rx) as f32 * gpx + gpx * 0.5;
            let gx = u.floor() as i32;
            if gx < 0 || gy < 0 || gx >= hd_w || gy >= hd_h || layer[(gy * hd_w + gx) as usize] > draw.group {
                continue;
            }
            let (fx, fy) = (u - draw.x as f32, v - draw.y as f32);
            let (sx, sy) = wide_window_inv(dir, fx, fy);
            let rel_x = sx - f32::from(part.dx);
            let rel_y = sy - f32::from(part.dy);
            let (tx, ty) = if spr.hd.is_some() {
                let tx = if flip { tax - rel_x * ts } else { tax + rel_x * ts };
                let ty = tay + rel_y * ts;
                (tx.floor() as i32, ty.floor() as i32)
            } else {
                let px = if flip { spr.ax as f32 - rel_x } else { spr.ax as f32 + rel_x };
                (px.floor() as i32, (spr.ay as f32 + rel_y).floor() as i32)
            };
            let p: [u8; 4] = if let Some(t) = &spr.hd {
                if !(0..t.w as i32).contains(&tx) || !(0..t.h as i32).contains(&ty) {
                    continue;
                }
                t.pal[usize::from(t.idx[(ty as u32 * t.w + tx as u32) as usize])]
            } else {
                if !(0..spr.w as i32).contains(&tx) || !(0..spr.h as i32).contains(&ty) {
                    continue;
                }
                spr.px[(ty as u32 * spr.w + tx as u32) as usize]
            };
            let sa = u32::from(p[3]) * alpha / 255;
            if sa == 0 {
                continue;
            }
            let di = drow + wx as usize;
            let mut d = dst[di];
            d = hd_mix(d, 16, p[0], sa, add);
            d = hd_mix(d, 8, p[1], sa, add);
            d = hd_mix(d, 0, p[2], sa, add);
            dst[di] = d | 0xff00_0000;
        }
    }
}

/// Composite the recorded effects over the window (`dst`, `0xAARRGGBB`, `dst_w × dst_h`) in which the guest frame of the
/// record was blitted as `view` says. A twin is sampled nearest-neighbour at the window's scale; a sprite without one at
/// 1×. Every part is blended as the guest draw would blend it (`fx_mix`), over window pixels whose guest pixel's layer is
/// `≤` the draw's group.
///
/// ★★ 2026-10-16 (the 6× demon body, the user: *"악의 발동했을때 캐릭터 외형 더 업스케일링"*): a twin DENSER than the window
/// (more than one twin px per window px — the 6× body on a 4.9× phone) is averaged over 2×2 samples at the window pixel's
/// quarter points (premultiplied — a transparent sample thins the alpha), so it shrinks smoothly instead of dropping every
/// fifth row. A twin at or under the window's density takes the one centre sample as before, to the bit.
pub fn compose_hd(lib: &FxLibrary, hd: &HdFrame, dst: &mut [u32], dst_w: u32, dst_h: u32, view: &HdView) {
    let (rx, ry, rw, rh) = view.rect;
    if rw == 0 || rh == 0 || view.src_w == 0 || view.src_h == 0 || hd.w == 0 || hd.h == 0 || hd.layer.len() != (hd.w * hd.h) as usize {
        return;
    }
    if dst.len() < (dst_w * dst_h) as usize {
        return;
    }
    // the window rect clipped to the window
    let x_lo = rx.max(0);
    let y_lo = ry.max(0);
    let x_hi = (rx + rw as i32).min(dst_w as i32);
    let y_hi = (ry + rh as i32).min(dst_h as i32);
    if x_lo >= x_hi || y_lo >= y_hi {
        return;
    }
    // guest px per window px
    let gpx = view.src_w as f32 / rw as f32;
    let gpy = view.src_h as f32 / rh as f32;
    let mut cols: Vec<(i32, i32, i32)> = Vec::new(); // (the two sample columns or −1, guest x)
    let mut rows: Vec<(i32, i32, i32)> = Vec::new(); // (the two sample rows or −1, guest y)
    for draw in &hd.draws {
        let Some(frame) = lib.frame(draw.effect, draw.age) else { continue };
        let ga = u32::from(draw.alpha);
        if ga == 0 {
            continue;
        }
        for part in frame {
            let spr = &lib.sprites[part.sprite as usize];
            let a = (u32::from(part.alpha) * ga + 127) / 255;
            if a == 0 {
                continue;
            }
            let flip = part.flags & FX_FLIP_X != 0;
            let add = part.flags & FX_ADD != 0;
            if lava_radial_on(part.flags) {
                compose_lava(
                    spr,
                    part,
                    a,
                    flip,
                    add,
                    draw,
                    &hd.layer,
                    hd.w as i32,
                    hd.h as i32,
                    dst,
                    dst_w,
                    dst_h,
                    view,
                );
                continue;
            }
            if wide_window_on(part.flags, draw.scale) {
                compose_wide(
                    spr,
                    part,
                    a,
                    flip,
                    add,
                    draw,
                    &hd.layer,
                    hd.w as i32,
                    hd.h as i32,
                    dst,
                    dst_w,
                    dst_h,
                    view,
                );
                continue;
            }
            let s = i32::from(draw.scale.max(1));
            let (cx, cy) = claw_shift(part.flags, s, spr, i32::from(part.dx), i32::from(part.dy));
            let (ox, oy) = (
                draw.x.saturating_add(i32::from(part.dx) * s + cx) as f32,
                draw.y.saturating_add(i32::from(part.dy) * s + cy) as f32,
            );
            // the twin or the 1× sprite (k = its px per guest px — half of that for a part at twice the size)
            let (k, sw, sh, ax, ay) = match &spr.hd {
                Some(t) => (f32::from(t.scale), t.w as i32, t.h as i32, t.ax as f32, t.ay as f32),
                None => (1.0, spr.w as i32, spr.h as i32, spr.ax as f32, spr.ay as f32),
            };
            let k = (if part.flags & FX_X2 != 0 { k / 2.0 } else { k }) / s as f32;
            if sw == 0 || sh == 0 {
                continue;
            }
            let dense = spr.hd.is_some() && k * gpx > 1.0;
            let offs: [f32; 2] = if dense { [0.25, 0.75] } else { [0.5, 0.5] };
            // the guest-space extent (float) and its window columns / rows
            let (gx0, gx1) = if flip {
                (ox + (ax - sw as f32 + 1.0) / k, ox + (ax + 1.0) / k)
            } else {
                (ox - ax / k, ox + (sw as f32 - ax) / k)
            };
            let (gy0, gy1) = (oy - ay / k, oy + (sh as f32 - ay) / k);
            let wx0 = (rx as f32 + gx0 / gpx).floor() as i32;
            let wx1 = (rx as f32 + gx1 / gpx).ceil() as i32;
            let wy0 = (ry as f32 + (gy0 - view.src_y0 as f32) / gpy).floor() as i32;
            let wy1 = (ry as f32 + (gy1 - view.src_y0 as f32) / gpy).ceil() as i32;
            let (wx0, wx1) = (wx0.max(x_lo), wx1.min(x_hi));
            let (wy0, wy1) = (wy0.max(y_lo), wy1.min(y_hi));
            if wx0 >= wx1 || wy0 >= wy1 {
                continue;
            }
            cols.clear();
            for wx in wx0..wx1 {
                let gx = ((wx - rx) as f32 * gpx + gpx * 0.5).floor() as i32; // the window pixel's centre in guest px
                let inside = (0..hd.w as i32).contains(&gx);
                let mut cs = [-1i32; 2];
                for (n, o) in offs.iter().enumerate() {
                    let u = (wx - rx) as f32 * gpx + gpx * o;
                    let t = (u - ox) * k;
                    let c = if flip { (ax - t).ceil() as i32 } else { (t + ax).floor() as i32 };
                    cs[n] = if inside && (0..sw).contains(&c) { c } else { -1 };
                }
                cols.push((cs[0], cs[1], gx));
            }
            rows.clear();
            for wy in wy0..wy1 {
                let gy = (view.src_y0 as f32 + (wy - ry) as f32 * gpy + gpy * 0.5).floor() as i32;
                let inside = (0..hd.h as i32).contains(&gy);
                let mut rs = [-1i32; 2];
                for (n, o) in offs.iter().enumerate() {
                    let v = view.src_y0 as f32 + (wy - ry) as f32 * gpy + gpy * o;
                    let r = ((v - oy) * k + ay).floor() as i32;
                    rs[n] = if inside && (0..sh).contains(&r) { r } else { -1 };
                }
                rows.push((rs[0], rs[1], gy));
            }
            for (j, &(r0, r1, gy)) in rows.iter().enumerate() {
                if r0 < 0 && r1 < 0 {
                    continue;
                }
                let wy = wy0 + j as i32;
                let drow = (wy as u32 * dst_w) as usize;
                let lrow = (gy as u32 * hd.w) as usize;
                for (i, &(c0, c1, gx)) in cols.iter().enumerate() {
                    if c0 < 0 && c1 < 0 {
                        continue;
                    }
                    if hd.layer[lrow + gx as usize] > draw.group {
                        continue;
                    }
                    let p: [u8; 4] = match &spr.hd {
                        Some(t) if dense => {
                            // 2×2 premultiplied: the colour by alpha weight, the alpha by the four
                            let mut acc = [0u32; 4];
                            for r in [r0, r1] {
                                for c in [c0, c1] {
                                    if r < 0 || c < 0 {
                                        continue;
                                    }
                                    let q = t.pal[usize::from(t.idx[(r * sw + c) as usize])];
                                    let qa = u32::from(q[3]);
                                    acc[0] += u32::from(q[0]) * qa;
                                    acc[1] += u32::from(q[1]) * qa;
                                    acc[2] += u32::from(q[2]) * qa;
                                    acc[3] += qa;
                                }
                            }
                            if acc[3] == 0 {
                                continue;
                            }
                            [
                                (acc[0] / acc[3]) as u8,
                                (acc[1] / acc[3]) as u8,
                                (acc[2] / acc[3]) as u8,
                                ((acc[3] + 2) / 4) as u8,
                            ]
                        }
                        Some(t) => t.pal[usize::from(t.idx[(r0 * sw + c0) as usize])],
                        None => spr.px[(r0 * sw + c0) as usize],
                    };
                    let sa = u32::from(p[3]) * a / 255;
                    if sa == 0 {
                        continue;
                    }
                    let di = drow + (wx0 + i as i32) as usize;
                    let mut d = dst[di];
                    d = hd_mix(d, 16, p[0], sa, add);
                    d = hd_mix(d, 8, p[1], sa, add);
                    d = hd_mix(d, 0, p[2], sa, add);
                    dst[di] = d | 0xff00_0000;
                }
            }
        }
    }
}
