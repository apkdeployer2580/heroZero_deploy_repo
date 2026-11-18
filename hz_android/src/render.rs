//! Drawing into the window buffer: the scaled game picture, then the HUD over it. Pure functions
//! over `&mut [u32]` (softbuffer's 0x00RRGGBB — no alpha channel, so translucency is blended by
//! hand) so they are tested on the host.

use std::collections::HashSet;

use wie_backend::KeyCode;

use crate::{
    font,
    hud::{self, Action, Control, Face, Rect, Shape},
    hud_view::{HvBuff, HvExp, HvGeom, HvIcon, HvStats, hv_draw_badges, hv_draw_buffs, hv_draw_exp, hv_draw_gl, hv_draw_status},
    minimap_view::{mm_draw_panel, mm_tile_px, mm_view_origin},
    padhud::{Badge, FacePos, PadPos},
    placement::CtrlId,
    touch::Hud,
};

/// Nearest-neighbour source column for each destination column of a `dst_w`-wide blit. Built once
/// per layout, not per frame.
pub fn column_map(dst_w: u32, src_w: u32) -> Vec<u32> {
    (0..dst_w)
        .map(|x| ((u64::from(x) * u64::from(src_w)) / u64::from(dst_w.max(1))) as u32)
        .collect()
}

#[allow(clippy::too_many_arguments)]
/// Clear the window to black and draw `src` (`src_w` x `src_h`, 0xAARRGGBB) scaled into `game`.
/// Each destination row is built once and copied down for every row that maps to the same source
/// row — at an integer scale of 6 that is one conversion per six rows.
pub fn blit_game(dst: &mut [u32], win_w: u32, win_h: u32, game: Rect, src: &[u32], src_w: u32, src_h: u32, cols: &[u32]) {
    let (ww, wh) = (win_w as i32, win_h as i32);
    let x0 = game.x.max(0);
    let x1 = (game.x + game.w).min(ww);
    let (y0, y1) = (game.y.max(0), (game.y + game.h).min(wh));
    if src.len() < (src_w * src_h) as usize || game.w <= 0 || game.h <= 0 || cols.len() != game.w as usize || x0 >= x1 || y0 >= y1 {
        dst.fill(0);
        return;
    }
    // Black around the picture only: the picture's own pixels are all written below (2026-10-27 —
    // clearing the whole window first was a second pass over 2.5 M pixels a present on the S25).
    dst[..(y0 * ww) as usize].fill(0);
    dst[(y1 * ww) as usize..].fill(0);
    for y in y0..y1 {
        let row = (y * ww) as usize;
        dst[row..row + x0 as usize].fill(0);
        dst[row + x1 as usize..row + ww as usize].fill(0);
    }
    let mut last_sy = u32::MAX;
    let mut prev_row: Option<usize> = None;
    for dy in y0..y1 {
        let sy = ((u64::from((dy - game.y) as u32) * u64::from(src_h)) / u64::from(game.h as u32)) as u32;
        let row_start = (dy * ww) as usize;
        if sy == last_sy
            && let Some(prev) = prev_row
        {
            dst.copy_within(prev + x0 as usize..prev + x1 as usize, row_start + x0 as usize);
        } else {
            let src_row = &src[(sy * src_w) as usize..((sy + 1) * src_w) as usize];
            for dx in x0..x1 {
                dst[row_start + dx as usize] = src_row[cols[(dx - game.x) as usize] as usize] & 0x00ff_ffff;
            }
            last_sy = sy;
        }
        prev_row = Some(row_start);
    }
}

/// `dst` blended toward `color` by `alpha`/255: per channel `(d·(255−a) + c·a) / 255`, floored.
///
/// ★ 2026-10-27: red and blue in one word, green in another, and the division done as
/// `(x + 1 + (x >> 8)) >> 8` — equal to `x / 255` for every `x` a channel can reach (≤ 255·255;
/// checked exhaustively, and two million random blends against the per-channel form), so every
/// pixel is what it was. Every translucent HUD pixel goes through here, every present.
#[inline]
fn blend(dst: u32, color: u32, alpha: u32) -> u32 {
    let inv = 255 - alpha;
    let rb = (dst & 0x00ff_00ff) * inv + (color & 0x00ff_00ff) * alpha;
    let g = (dst & 0x0000_ff00) * inv + (color & 0x0000_ff00) * alpha;
    let rb = ((rb + 0x0001_0001 + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let g = ((g + 0x0000_0100 + ((g >> 8) & 0x0000_ff00)) >> 8) & 0x0000_ff00;
    rb | g
}

/// Which round shape a [`RoundMask`] is (its coverage function).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RoundKind {
    Disc,
    Soft,
    Ring,
}

/// Sub-pixel steps a round shape's centre, radius and parameter are taken at (1/16 px).
const ROUND_PHASES: f64 = 16.0;

/// `(kind, centre x, centre y, radius, parameter)` in [`ROUND_PHASES`] — the centre relative to
/// the mask's corner, so a shape that only moves by whole pixels keeps its mask.
type RoundKey = (RoundKind, i32, i32, i32, i32);

/// A round shape's coverage (0..=255 in up to two channels) over its box, and the runs of pixels
/// it covers at all, `(row, from, to)`.
///
/// ★ 2026-10-27: every disc, soft disc and ring of the HUD used to be rasterised for each present
/// — a `sqrt` and float math for every pixel of its bounding box. On the S25's 2340×1080 window
/// (host bench `frame_cost`) the stick alone was 1.0 ms and OK 0.8 ms of a 3.4 ms HUD, every
/// game frame and every touch move. Its pixels only change with its size and sub-pixel phase
/// (hero2 cached the same keys as layers, #4), so the coverage is made once per those and drawn
/// as an integer blend over the covered runs only.
struct RoundMask {
    w: i32,
    a: Vec<u8>,
    b: Vec<u8>,
    runs: Vec<(i32, i32, i32)>,
}

static ROUND_MASKS: std::sync::Mutex<Option<std::collections::HashMap<RoundKey, std::sync::Arc<RoundMask>>>> = std::sync::Mutex::new(None);

/// The mask for `key` (see [`Canvas::round`]), from the cache or made now. `cover` is the shape's
/// coverage at a distance from the centre; the key's kind names it.
fn round_mask(key: RoundKey, pad: f64, cover: impl Fn(f64, f64, f64) -> (f64, f64)) -> std::sync::Arc<RoundMask> {
    if let Some(m) = ROUND_MASKS.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&key).cloned())) {
        return m;
    }
    let (_, qx, qy, qr, qp) = key;
    let (cx, cy, r, p) = (f64::from(qx) / ROUND_PHASES, f64::from(qy) / ROUND_PHASES, f64::from(qr) / ROUND_PHASES, f64::from(qp) / ROUND_PHASES);
    let w = (cx + r + pad).ceil() as i32 + 1;
    let h = (cy + r + pad).ceil() as i32 + 1;
    let n = (w.max(0) * h.max(0)) as usize;
    let (mut a, mut b, mut runs) = (vec![0u8; n], vec![0u8; n], Vec::new());
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    for y in 0..h {
        let mut run: Option<i32> = None;
        for x in 0..w {
            let d = ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - cy).powi(2)).sqrt();
            let (ca, cb) = cover(d, r, p);
            let i = (y * w + x) as usize;
            (a[i], b[i]) = (byte(ca), byte(cb));
            match (a[i] > 0 || b[i] > 0, run) {
                (true, None) => run = Some(x),
                (false, Some(from)) => {
                    runs.push((y, from, x));
                    run = None;
                }
                _ => {}
            }
        }
        if let Some(from) = run {
            runs.push((y, from, w));
        }
    }
    let m = std::sync::Arc::new(RoundMask { w, a, b, runs });
    if let Ok(mut g) = ROUND_MASKS.lock() {
        let map = g.get_or_insert_with(std::collections::HashMap::new);
        // The stick's thumb follows a finger at any phase: a bound, not a working set.
        if map.len() >= 512 {
            map.clear();
        }
        map.insert(key, m.clone());
    }
    m
}

struct Canvas<'a> {
    px: &'a mut [u32],
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, color: u32, alpha: u32) {
        if alpha > 0 && x >= 0 && y >= 0 && x < self.w && y < self.h {
            let i = (y * self.w + x) as usize;
            self.px[i] = blend(self.px[i], color, alpha.min(255));
        }
    }

    /// An anti-aliased disc (coverage from the radial distance — at control sizes as good as
    /// supersampling and far cheaper), with an optional anti-aliased outline `ring_w` wide.
    #[allow(clippy::too_many_arguments)]
    fn disc(&mut self, cx: f64, cy: f64, r: f64, fill: u32, fill_a: u32, ring: u32, ring_a: u32, ring_w: f64) {
        if r <= 0.0 {
            return;
        }
        let cover = |d: f64, r: f64, ring_w: f64| {
            let outer = (r + 0.5 - d).clamp(0.0, 1.0);
            if outer <= 0.0 {
                return (0.0, 0.0);
            }
            let inner = (r - ring_w + 0.5 - d).clamp(0.0, 1.0);
            (inner, if ring_w > 0.0 && outer > inner { outer - inner } else { 0.0 })
        };
        self.round(RoundKind::Disc, cx, cy, r, ring_w, 1.0, cover, (fill, fill_a), (ring, ring_a));
    }

    /// hero2's `blend_disc_soft`: a disc whose fill fades out over its last `feather` px
    /// (smoothstepped), so a key melts into the game instead of ending on a hard arc.
    fn disc_soft(&mut self, cx: f64, cy: f64, r: f64, color: u32, a: u32, feather: f64) {
        if r <= 0.0 || a == 0 {
            return;
        }
        let cover = |d: f64, r: f64, f: f64| {
            let cov = ((r - d) / f).clamp(0.0, 1.0);
            (cov * cov * (3.0 - 2.0 * cov), 0.0)
        };
        self.round(RoundKind::Soft, cx, cy, r, feather.max(0.9), 1.0, cover, (color, a), (0, 0));
    }

    /// hero2's `blend_ring_soft`: an anti-aliased ring `thick` px wide centred on radius `r`.
    fn ring_soft(&mut self, cx: f64, cy: f64, r: f64, thick: f64, color: u32, a: u32) {
        if r <= 1.0 || a == 0 {
            return;
        }
        let half = (thick / 2.0).max(0.6);
        let cover = |d: f64, r: f64, half: f64| ((1.0 - ((d - r).abs() - half).max(0.0)).clamp(0.0, 1.0), 0.0);
        self.round(RoundKind::Ring, cx, cy, r, half, half + 1.5, cover, (color, a), (0, 0));
    }

    /// A round shape centred on (`cx`, `cy`) out to `r` + `pad`, in up to two colours whose
    /// coverage `cover(distance, r, p)` gives for a pixel's centre — through [`round_mask`]: the
    /// coverage is worked out once per shape size and sub-pixel phase, and drawn from its runs.
    #[allow(clippy::too_many_arguments)]
    fn round(&mut self, kind: RoundKind, cx: f64, cy: f64, r: f64, p: f64, pad: f64, cover: impl Fn(f64, f64, f64) -> (f64, f64), first: (u32, u32), second: (u32, u32)) {
        let (ox, oy) = ((cx - r - pad).floor(), (cy - r - pad).floor());
        let q = |v: f64| (v * ROUND_PHASES).round() as i32;
        let key = (kind, q(cx - ox), q(cy - oy), q(r), q(p));
        let m = round_mask(key, pad, cover);
        let (ox, oy) = (ox as i32, oy as i32);
        for &(row, from, to) in &m.runs {
            let y = oy + row;
            if y < 0 || y >= self.h {
                continue;
            }
            for lx in from.max(-ox)..to.min(self.w - ox) {
                let i = (row * m.w + lx) as usize;
                let (a, b) = (u32::from(m.a[i]), u32::from(m.b[i]));
                let at = (y * self.w + ox + lx) as usize;
                if a > 0 && first.1 > 0 {
                    self.px[at] = blend(self.px[at], first.0, first.1 * a / 255);
                }
                if b > 0 && second.1 > 0 {
                    self.px[at] = blend(self.px[at], second.0, second.1 * b / 255);
                }
            }
        }
    }

    fn rect(&mut self, b: Rect, fill: u32, fill_a: u32, ring: u32, ring_a: u32, ring_w: i32) {
        for y in b.y.max(0)..(b.y + b.h).min(self.h) {
            for x in b.x.max(0)..(b.x + b.w).min(self.w) {
                let edge = x < b.x + ring_w || y < b.y + ring_w || x >= b.x + b.w - ring_w || y >= b.y + b.h - ring_w;
                if edge {
                    self.put(x, y, ring, ring_a);
                } else {
                    self.put(x, y, fill, fill_a);
                }
            }
        }
    }

    fn text_centred(&mut self, text: &str, cx: i32, cy: i32, scale: u32, color: u32, alpha: u32) {
        let x = cx - font::text_width(text, scale) as i32 / 2;
        self.text_at(text, x, cy, scale, color, alpha);
    }

    /// `text` from `x`, centred on the line `cy`.
    fn text_at(&mut self, text: &str, x: i32, cy: i32, scale: u32, color: u32, alpha: u32) {
        let y = cy - (font::GLYPH_H * scale) as i32 / 2;
        let mut pts = Vec::new();
        font::for_each_pixel(text, x, y, scale, |px, py| pts.push((px, py)));
        for (px, py) in pts {
            self.put(px, py, color, alpha);
        }
    }

    /// `r` in `color`, opaque.
    fn fill(&mut self, r: Rect, color: u32) {
        let b = r.clipped(&Rect::new(0, 0, self.w, self.h));
        for y in b.y..b.bottom() {
            let row = (y * self.w) as usize;
            self.px[row + b.x as usize..row + b.right() as usize].fill(color);
        }
    }

    /// The largest text scale at which `text` fits `w` x `h` with a margin.
    fn fit_scale(text: &str, w: i32, h: i32) -> u32 {
        let by_h = (h as u32 * 45 / 100) / font::GLYPH_H;
        let by_w = (w as u32 * 85 / 100) / font::text_width(text, 1).max(1);
        by_h.min(by_w).max(1)
    }
}

const WHITE: u32 = 0x00ff_ffff;
const RED: u32 = 0x00e0_3030;

/// hero2's key colours (`overlay.rs`): a slate face, a steel rim, the pressed blue; the arrows a
/// little bluer; OK's gold.
const BTN: u32 = 0x002c_3040;
const BTN_DPAD: u32 = 0x0026_3548;
const BTN_PRESSED: u32 = 0x004a_6cff;
const BORDER: u32 = 0x0070_80a0;
const GOLD: u32 = 0x00e0_9a3c;

/// hero2's `draw_round_face`, the one round key: the inscribed disc of `r`, translucent, its fill
/// fading over the last ~9 % of the radius, and an anti-aliased rim. Returns its centre and radius.
fn round_face(c: &mut Canvas, r: Rect, fill: u32, rim: u32, fill_a: u32, rim_a: u32) -> (f64, f64, f64) {
    let rad = f64::from(r.w.min(r.h)) / 2.0;
    let (cx, cy) = (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
    if rad >= 1.0 {
        c.disc_soft(cx, cy, rad, fill, fill_a, (rad * 0.09).max(1.2));
        let t = (rad / 13.0).clamp(1.4, 4.0);
        c.ring_soft(cx, cy, rad - t / 2.0, t, rim, rim_a);
    }
    (cx, cy, rad)
}

fn draw_control(c: &mut Canvas, ctl: &Control, on: bool) {
    match ctl.face {
        Face::Hidden => return,
        Face::Hex(ink) => {
            if let Some(k) = ctl.action.key() {
                draw_hex(c, ink, k, if on { BTN_PRESSED } else { BTN_DPAD });
            }
            return;
        }
        // hero2's page button (`draw_mask_button_spanned` with `QUICKPAGE_ICON_SPAN`): its icon
        // smaller than the minimap toggle's — the player's 「페이지 아이콘이 너무 크고」 (2026-08-06).
        // The PC's cell keeps the cell's face.
        Face::Icon(icon) => {
            let mut b = ctl.shape.bounds();
            if let Shape::Box(r) = ctl.shape {
                c.rect(r, if on { BTN_PRESSED } else { BTN }, 92, BORDER, 165, (r.h / 18).max(2));
                // Under the PC cell's key letters (`draw_cell_key`, top-left).
                b = Rect::new(r.x, r.y + r.h / 5, r.w, r.h - r.h / 5);
            }
            draw_mask_icon(c, b, icon, QUICKPAGE_ICON_SPAN, on, matches!(ctl.shape, Shape::Circle { .. }));
            return;
        }
        // The one key that matters most: hero2's ACTION disc, no label, a gold rim and ring.
        Face::Action => {
            let (cx, cy, rad) = round_face(c, ctl.shape.bounds(), if on { BTN_PRESSED } else { BTN }, GOLD, 95, 180);
            c.ring_soft(cx, cy, rad * 0.72, (rad / 16.0).max(1.4), GOLD, 140);
            return;
        }
        Face::Plain => {}
    }
    let fill = if on { BTN_PRESSED } else { BTN };
    match ctl.shape {
        Shape::Circle { .. } => {
            round_face(c, ctl.shape.bounds(), fill, BORDER, 92, 165);
        }
        Shape::Box(b) | Shape::Wedge { square: b, .. } => c.rect(b, fill, 92, BORDER, 165, (b.h / 18).max(2)),
    }
    if !ctl.label.is_empty() {
        let b = ctl.shape.bounds();
        let scale = Canvas::fit_scale(ctl.label, b.w * 70 / 100, b.h * 70 / 100);
        let (cx, cy) = b.centre();
        c.text_centred(ctl.label, cx, cy, scale, WHITE, 235);
    }
}

/// A window's arrow (hero2's `draw_hex_face` + `draw_hex_glyph`): its rounded hexagon in `ink`,
/// translucent with a rim band, and a squat rounded triangle pointing its way in the middle.
fn draw_hex(c: &mut Canvas, ink: Rect, key: KeyCode, fill: u32) {
    if ink.w < 4 || ink.h < 4 {
        return;
    }
    hex_face(c, ink, key, fill, BORDER, 92, 165);
    // The glyph: 0.19 of the arrow's length, base 2.3 × that wide, 1.65 × tall.
    let along = if matches!(key, KeyCode::UP | KeyCode::DOWN) { ink.h } else { ink.w };
    let g = f64::from(along) * 0.19;
    if g < 3.0 {
        return;
    }
    let (hb, tip, base) = (g * 1.15, g, g * 0.65);
    let side = (hb * 2.0 + 4.0).ceil() as i32;
    let m = f64::from(side) / 2.0;
    let (v, dir) = match key {
        KeyCode::UP => ([(m, m - tip), (m + hb, m + base), (m - hb, m + base)], 0),
        KeyCode::DOWN => ([(m, m + tip), (m - hb, m - base), (m + hb, m - base)], 1),
        KeyCode::LEFT => ([(m - tip, m), (m + base, m - hb), (m + base, m + hb)], 2),
        _ => ([(m + tip, m), (m - base, m + hb), (m - base, m - hb)], 3),
    };
    let (cx, cy) = ink.centre();
    let bbox = Rect::new(cx - side / 2, cy - side / 2, side, side);
    convex_face(c, bbox, &v, (2, dir), (g * 0.22).max(1.0), 0.0, WHITE, WHITE, 215, 0);
}

/// An arrow's rounded hexagon alone (hero2's `draw_hex_face`), in `ink`, with a rim band.
fn hex_face(c: &mut Canvas, ink: Rect, key: KeyCode, fill: u32, rim: u32, fill_a: u32, rim_a: u32) {
    if ink.w < 4 || ink.h < 4 {
        return;
    }
    let short = f64::from(ink.w.min(ink.h));
    let round_r = (short * 0.10).clamp(1.5, 16.0);
    let t = (short / 13.0).clamp(1.4, 4.0);
    // The mask is the same for UP and DOWN (and LEFT and RIGHT): the key carries the axis.
    let axis = u8::from(matches!(key, KeyCode::UP | KeyCode::DOWN));
    let verts = hud::hexagon(Rect::new(0, 0, ink.w, ink.h), key);
    convex_face(c, ink, &verts, (1, axis), round_r, t, fill, rim, fill_a, rim_a);
}

/// `(kind, variant, w, h, round_r × 10, rim_t × 10)`: everything a mask's pixels depend on.
type FaceKey = (u8, u8, i32, i32, u32, u32);

/// Rounded-convex-polygon masks (hero2's `FACE_MASKS`): each pixel `cover << 8 | rim × cover`. A
/// handful — two hexagon axes and four glyphs per arrow size — so the arrows cost one blend a pixel.
static FACE_MASKS: std::sync::Mutex<Option<std::collections::HashMap<FaceKey, std::sync::Arc<Vec<u16>>>>> = std::sync::Mutex::new(None);

/// hero2's `draw_convex_face`: the convex polygon `verts` (relative to `bbox`), its corners rounded
/// by `round_r`, anti-aliased, with a rim band `rim_t` wide just inside the outline.
#[allow(clippy::too_many_arguments)]
fn convex_face(c: &mut Canvas, bbox: Rect, verts: &[(f64, f64)], kind: (u8, u8), round_r: f64, rim_t: f64, fill: u32, rim: u32, fill_a: u32, rim_a: u32) {
    if verts.len() < 3 || bbox.is_empty() {
        return;
    }
    let key: FaceKey = (kind.0, kind.1, bbox.w, bbox.h, (round_r * 10.0) as u32, (rim_t * 10.0) as u32);
    let cached = FACE_MASKS.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&key).cloned()));
    let mask = cached.unwrap_or_else(|| {
        let m = std::sync::Arc::new(convex_mask(bbox.w, bbox.h, verts, round_r, rim_t));
        if let Ok(mut g) = FACE_MASKS.lock() {
            let map = g.get_or_insert_with(std::collections::HashMap::new);
            if map.len() >= 48 {
                map.clear();
            }
            map.insert(key, m.clone());
        }
        m
    });
    for py in 0..bbox.h {
        for px in 0..bbox.w {
            let m = mask[(py * bbox.w + px) as usize];
            let cover = u32::from(m >> 8);
            if cover == 0 {
                continue;
            }
            let (x, y) = (bbox.x + px, bbox.y + py);
            c.put(x, y, fill, fill_a * cover / 255);
            let edge = u32::from(m & 0xff);
            if rim_a > 0 && edge > 0 {
                c.put(x, y, rim, rim_a * edge / 255);
            }
        }
    }
}

/// The distance-field mask behind [`convex_face`] (hero2's `convex_mask`): the polygon eroded by
/// `round_r` (each edge pushed in along its normal, neighbours re-intersected), the exact distance
/// to that, less `round_r` — which rounds every corner by exactly `round_r`.
fn convex_mask(w: i32, h: i32, verts: &[(f64, f64)], round_r: f64, rim_t: f64) -> Vec<u16> {
    let n = verts.len();
    let mut out = vec![0u16; (w.max(0) * h.max(0)) as usize];
    let area2: f64 = (0..n).map(|i| verts[i].0 * verts[(i + 1) % n].1 - verts[(i + 1) % n].0 * verts[i].1).sum();
    if area2.abs() < 1e-6 {
        return out;
    }
    let wind = area2.signum();
    let (cx, cy) = verts.iter().fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (cx, cy) = (cx / n as f64, cy / n as f64);
    let r_in = (0..n)
        .map(|i| {
            let ((ax, ay), (bx, by)) = (verts[i], verts[(i + 1) % n]);
            let (ex, ey) = (bx - ax, by - ay);
            ((ex * (cy - ay) - ey * (cx - ax)) / (ex * ex + ey * ey).sqrt().max(1e-9)).abs()
        })
        .fold(f64::INFINITY, f64::min);
    let round_r = round_r.min(r_in * 0.9).max(0.0);
    // The eroded polygon: each edge line `nx·x + ny·y = c` moved in by `round_r`.
    let lines: Vec<(f64, f64, f64)> = (0..n)
        .map(|i| {
            let ((ax, ay), (bx, by)) = (verts[i], verts[(i + 1) % n]);
            let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt().max(1e-9);
            let (nx, ny) = (-(by - ay) / len * wind, (bx - ax) / len * wind);
            (nx, ny, nx * ax + ny * ay + round_r)
        })
        .collect();
    let v: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let ((n1x, n1y, c1), (n2x, n2y, c2)) = (lines[(i + n - 1) % n], lines[i]);
            let det = n1x * n2y - n2x * n1y;
            if det.abs() < 1e-9 { verts[i] } else { ((c1 * n2y - c2 * n1y) / det, (n1x * c2 - n2x * c1) / det) }
        })
        .collect();
    let aa = 1.2f64;
    for py in 0..h {
        let fy = f64::from(py) + 0.5;
        for px in 0..w {
            let fx = f64::from(px) + 0.5;
            let mut best = f64::INFINITY;
            let mut inside = true;
            for i in 0..n {
                let (ax, ay) = v[i];
                let (ex, ey) = (v[(i + 1) % n].0 - ax, v[(i + 1) % n].1 - ay);
                let (wx, wy) = (fx - ax, fy - ay);
                let t = ((wx * ex + wy * ey) / (ex * ex + ey * ey).max(1e-9)).clamp(0.0, 1.0);
                best = best.min((wx - ex * t).powi(2) + (wy - ey * t).powi(2));
                if (ex * wy - ey * wx) * wind < 0.0 {
                    inside = false;
                }
            }
            let d = round_r - best.sqrt() * if inside { -1.0 } else { 1.0 };
            if d < -aa {
                continue;
            }
            let cover = ((d + aa) / (2.0 * aa)).clamp(0.0, 1.0);
            let edge = if rim_t > 0.0 && d < rim_t { 1.0 - (d / rim_t).clamp(0.0, 1.0) } else { 0.0 };
            out[(py * w + px) as usize] = (((cover * 255.0).round() as u16) << 8) | (edge * cover * 255.0).round() as u16;
        }
    }
    out
}

/// hero2's stick: a dark navy base and a pale steel thumb.
const STICK_BASE: u32 = 0x001a_2436;
const STICK_THUMB: u32 = 0x009f_b6d4;

/// The top tabs' colours (hero2's, hero3's measured): gold rim, near-black face at 170.
const TAB_RIM: u32 = 0x00b9_8323;
const TAB_FACE: u32 = 0x000a_0c12;
const TAB_FACE_A: u32 = 170;
/// The longest label a tab carries (`SAVE`): every tab letters at the size that fits it.
const TAB_LABEL_CHARS: i32 = 4;

/// A top tab (Phase 7, hero2's `draw_top_tab`): flush with the window's top edge, square at the
/// top and round at the bottom, a gold rim on the three sides that are not the screen's edge, and
/// the label in the HUD's 5×7 font laid twice one pixel apart (bold). Anti-aliased on the two arcs
/// only, so a thin rim stays crisp. What it draws is smaller than what it answers to
/// ([`hud::top_tab_hit`]).
fn draw_top_tab(c: &mut Canvas, r: Rect, text: &str, on: bool) {
    if r.is_empty() {
        return;
    }
    let t = (r.w / 28).max(1);
    let rad = (r.w / 10).max(t + 1).min(r.w / 2).min(r.h);
    let (tf, rf) = (f64::from(t), f64::from(rad));
    let ccy = f64::from(r.bottom()) - rf;
    let (ccl, ccr) = (f64::from(r.x) + rf, f64::from(r.right()) - rf);
    let face_a = if on { 235 } else { TAB_FACE_A };
    for y in r.y.max(0)..r.bottom().min(c.h) {
        let py = f64::from(y) + 0.5;
        let corner_row = y + rad >= r.bottom();
        for x in r.x.max(0)..r.right().min(c.w) {
            // (how much of the pixel is inside the tab, how much of that is face rather than rim)
            let (cov, face_k) = if corner_row && (x < r.x + rad || x + rad >= r.right()) {
                let cx = if x < r.x + rad { ccl } else { ccr };
                let d = ((f64::from(x) + 0.5 - cx).powi(2) + (py - ccy).powi(2)).sqrt();
                ((rf - d + 0.5).clamp(0.0, 1.0), ((rf - tf) - d + 0.5).clamp(0.0, 1.0))
            } else {
                let edge = (x - r.x).min(r.right() - 1 - x).min(r.bottom() - 1 - y);
                (1.0, if edge < t { 0.0 } else { 1.0 })
            };
            if cov <= 0.0 {
                continue;
            }
            if face_k > 0.0 {
                c.put(x, y, TAB_FACE, (f64::from(face_a) * cov * face_k).round() as u32);
            }
            if face_k < 1.0 {
                c.put(x, y, TAB_RIM, (255.0 * cov * (1.0 - face_k)).round() as u32);
            }
        }
    }
    let inner_h = (r.h - t).max(1);
    let glyph_h = font::GLYPH_H as i32;
    // A run of n glyphs is 6n - 1 cells wide (+1 px for the bold pass).
    let s = ((r.w - t * 2 - 5) / (TAB_LABEL_CHARS * 6 - 1)).min(inner_h * 3 / 5 / glyph_h).max(1);
    let bold = i32::from(s >= 2);
    let tw = font::text_width(text, s as u32) as i32 + bold;
    let (x, y) = (r.x + (r.w - tw) / 2, r.y + (inner_h - glyph_h * s) / 2);
    let mut pts = Vec::new();
    for dx in 0..=bold {
        font::for_each_pixel(text, x + dx, y, s as u32, |px, py| pts.push((px, py)));
    }
    for (px, py) in pts {
        c.put(px, py, WHITE, 255);
    }
}

/// The game's own minimap window's colours (menu 8 / `*`, probe 2026-10-23).
const GAME_MM_WALL: u32 = 0x005f_5f3f;
const GAME_MM_FLOOR: u32 = 0x00bf_bfdf;
const GAME_MM_EXIT: u32 = 0x0039_7dff;
const GAME_MM_BAND: u32 = 0x009b_040d;
const GAME_MM_PLAYER: u32 = 0x00ff_0000;

/// The world map popup's chrome (hero2's window: a dark edge, a bevel, a black title band).
const WM_CHROME_EDGE: u32 = 0x003d_2f1b;
const WM_CHROME_BEVEL: u32 = 0x009b_8a71;

/// The world map (Phase 7-2, hero2's `draw_worldmap`, which is hero5's StateMap::DrawWorldMap in
/// hero2's chrome): the field dimmed behind it, the chrome and its black title band with the round
/// back button, the picture (or a flat sea) through the camera with hero5's edge shading, the
/// ships' dashed lanes and the roads under the studs, the player's place green, the cursor's place
/// under the hopping pin over a breathing stud.
fn draw_world(c: &mut Canvas, hud: &Hud, w: &crate::touch::World) {
    use crate::worldmap_view::*;
    let (sw, sh) = (c.w.max(0) as u32, c.h.max(0) as u32);
    let p = hud.world_panel();
    if p.view.w == 0 || p.view.h == 0 {
        return;
    }
    for v in c.px.iter_mut() {
        let x = *v;
        *v = (((x >> 16) & 0xff) * 4 / 10) << 16 | (((x >> 8) & 0xff) * 4 / 10) << 8 | ((x & 0xff) * 4 / 10);
    }
    let full = WmBox { x: 0, y: 0, w: sw, h: sh };
    let f = p.frame;
    let e = WM_EDGE / 2;
    wm_fill_a(c.px, sw, sh, full, f, WM_CHROME_EDGE, 255);
    wm_fill_a(c.px, sw, sh, full, WmBox { x: f.x + e, y: f.y + e, w: f.w.saturating_sub(2 * e), h: f.h.saturating_sub(2 * e) }, WM_CHROME_BEVEL, 255);
    wm_fill_a(c.px, sw, sh, full, p.band, 0, 255);

    let (iw, ih) = wm_picture_size();
    match wm_picture() {
        Some(pic) => wm_blit(c.px, sw, sh, &pic.px, pic.w, pic.h, w.cam, p.view),
        None => wm_fill_a(c.px, sw, sh, full, p.view, WM_FLAT_SEA, 255),
    }
    wm_vignette(c.px, sw, sh, p.view, 35 * p.unit);

    let at = |i: usize| wm_screen_raw(w.cam, WM_PLACES[i].x, WM_PLACES[i].y, iw, ih, p.view);
    for &(a, b, qx, qy) in WM_SEA_LANES {
        let bend = wm_screen_raw(w.cam, qx, qy, iw, ih, p.view);
        wm_draw_sea_lane(c.px, sw, sh, p.view, at(a), bend, at(b), p.unit);
    }
    for &(a, b) in WM_ROADS {
        let ((x0, y0), (x1, y1)) = (at(a), at(b));
        if wm_road_is_dirt(a, b) {
            // The painting already carries the actual land connections as winding tracks.
            // Straight opaque strokes over it cut across mountains and hide that artwork.
            // Keep the graph visible when the optional background asset is missing.
            if wm_picture().is_none() {
                wm_draw_dirt_road(c.px, sw, sh, p.view, x0, y0, x1, y1, p.unit);
            }
        } else {
            wm_draw_road(c.px, sw, sh, p.view, x0, y0, x1, y1, p.unit);
        }
    }
    // The game-style minimap inset (7-5), drawn last: the names keep off it.
    let minimap = hud.world_minimap().and_then(|(m, player)| wm_inset(p.view, m.w, m.h, p.unit).map(|inset| (m, player, inset)));
    let inset = minimap.map(|(_, _, (panel, ..))| panel);
    // The studs, the cursor's last so nothing covers it.
    let margin = (16 * p.unit) as i32;
    let inside = |(x, y): (i32, i32)| {
        x >= p.view.x as i32 - margin && y >= p.view.y as i32 - margin && x <= (p.view.x + p.view.w) as i32 + margin && y <= (p.view.y + p.view.h) as i32 + margin
    };
    for i in 0..WM_PLACES.len() {
        let (x, y) = at(i);
        if !inside((x, y)) {
            continue;
        }
        if i != w.cursor {
            let kind = if Some(i) == w.here { WmStud::Here } else { WmStud::Plain };
            wm_draw_stud(c.px, sw, sh, p.view, x, y, p.unit, kind);
        }
        // The name (7-3), in the game's own font, in hero5's half-transparent box under the stud,
        // pushed back inside the window at its edges.
        if let Some(l) = hud.label(i)
            && let Some(b) = wm_name_box(x, y, l.w * p.unit, p.unit, p.view, inset)
        {
            wm_fill_a(c.px, sw, sh, p.view, b, 0, 150);
            let ty = b.y + b.h.saturating_sub(l.h * p.unit) / 2;
            draw_label(c, l, (b.x + 4 * p.unit) as i32, ty as i32, p.unit, 0x00ff_fff4, p.view);
        }
    }
    // Where the quest under way leads (7-6): the game's 「!」 beside that place's stud.
    if let Some(q) = hud.quest_place() {
        let (qx, qy) = at(q);
        if inside((qx, qy)) {
            let u = p.unit as i32;
            wm_draw_quest(c.px, sw, sh, p.view, qx + 7 * u, qy - 7 * u, 5.0 * p.unit as f32);
        }
    }
    let (x, y) = at(w.cursor);
    // hero5's stud breathes: a triangle wave over ~1 s at 30 frames a second.
    let phase = w.frame % 32;
    let glow = if phase < 16 { phase * 16 } else { (31 - phase) * 16 };
    wm_draw_stud(c.px, sw, sh, p.view, x, y, p.unit, WmStud::Cursor { glow, here: Some(w.cursor) == w.here });
    wm_draw_pin(c.px, sw, sh, p.view, x, y, p.unit, wm_pin_lift(w.frame));

    // The game's own minimap window, for the cursor's place (7-5): its red title band with the
    // name, walls olive, floor lavender, exits blue, the player red — the menu-8 window's colours
    // (probe 2026-10-23; `g.a(dd)` draws 0x397DFF exits and a 0xFF0000 player).
    if let Some((m, player, (panel, band, (ox, oy), s))) = minimap {
        wm_fill_a(c.px, sw, sh, full, WmBox { x: panel.x.saturating_sub(2), y: panel.y.saturating_sub(2), w: panel.w + 4, h: panel.h + 4 }, 0, 160);
        wm_fill_a(c.px, sw, sh, full, panel, GAME_MM_WALL, 255);
        wm_fill_a(c.px, sw, sh, full, band, GAME_MM_BAND, 255);
        for ty in 0..u32::from(m.h) {
            for tx in 0..u32::from(m.w) {
                let colour = match m.cells[(ty * u32::from(m.w) + tx) as usize] {
                    hz_java::state::MM_FLOOR => GAME_MM_FLOOR,
                    hz_java::state::MM_EXIT => GAME_MM_EXIT,
                    _ => continue,
                };
                wm_fill_a(c.px, sw, sh, full, WmBox { x: ox + tx * s, y: oy + ty * s, w: s, h: s }, colour, 255);
            }
        }
        if let Some((px_, py_)) = player {
            let (cx, cy) = (ox as f32 + px_ * s as f32, oy as f32 + py_ * s as f32);
            wm_disc(c.px, sw, sh, full, cx as i32, cy as i32, (s as f32 * 0.5).max(1.5 * p.unit as f32), GAME_MM_PLAYER, 255);
        }
        // The quest's guide tile in this map (7-6): the game window's yellow 「!」, anchored as that
        // window anchors it — `g.a(dd)` draws `rpg/e.p` TOP|LEFT at (x0 + h·f + 1, y0 + i·f), so it
        // hangs right and down from the tile, over the door that runs on from it (S25 2026-10-23:
        // centred on the tile it sat beside the door). Not at its size: the game's 12 px icon is
        // three of its 4 px tiles, which at the inset's tiles would cover a room — ~1.6 tiles here.
        if let Some((gx, gy)) = hud.guide_tile(m.id) {
            let d = (s * 8 / 5).max(6 * p.unit);
            let (x0, y0) = (ox + u32::from(gx) * s + s / 4, oy + u32::from(gy) * s);
            wm_draw_quest(c.px, sw, sh, p.view, (x0 + d / 2) as i32, (y0 + d / 2) as i32, d as f32 / 2.0);
        }
        // The band's name: the game window's own title for the map the player is on
        // (「고아원-2층A」), the place's name for a place's map read from its file.
        if let Some(l) = m.title.as_ref().or(hud.label(w.cursor)) {
            let ls = (band.h * 7 / 10 / l.h.max(1)).max(1);
            let ty = band.y + band.h.saturating_sub(l.h * ls) / 2;
            draw_label(c, l, (band.x + band.w.saturating_sub(l.w * ls) / 2) as i32, ty as i32, ls, 0x00ff_ffff, band);
        }
    }

    // The title band: the title on a lighter plate fading out to the right (hero5's), the back
    // button at the right end.
    if let Some(t) = hud.label(WM_PLACES.len()) {
        let s = ((p.band.h * 6 / 10) / t.h.max(1)).max(1);
        let plate = WmBox { x: p.band.x, y: p.band.y, w: t.w * s + 12 * p.unit + p.band.h, h: p.band.h };
        for dx in 0..plate.w {
            let a = 55 - 55 * dx / plate.w.max(1);
            wm_fill_a(c.px, sw, sh, full, WmBox { x: plate.x + dx, y: plate.y, w: 1, h: plate.h }, 0x00ff_ffff, a);
        }
        let ty = p.band.y + p.band.h.saturating_sub(t.h * s) / 2;
        draw_label(c, t, (p.band.x + 6 * p.unit) as i32, ty as i32, s, 0x00f4_f0e0, full);
    }
    wm_draw_back(c.px, sw, sh, full, p.back, false);
}

/// A label the game's font lettered (white ink, alpha = coverage) at `scale` window px per game
/// pixel, `color`, with a dark shadow half a pixel down-right, clipped to `clip`.
fn draw_label(c: &mut Canvas, l: &hz_java::Icon, x: i32, y: i32, scale: u32, color: u32, clip: crate::worldmap_view::WmBox) {
    let s = scale.max(1) as i32;
    let shadow = (s / 2).max(1);
    let (cx0, cy0, cx1, cy1) = (clip.x as i32, clip.y as i32, (clip.x + clip.w) as i32, (clip.y + clip.h) as i32);
    for (pass, (dx, dy, ink)) in [(shadow, shadow, 0x0010_1218u32), (0, 0, color)].into_iter().enumerate() {
        for py in 0..l.h as i32 {
            for px in 0..l.w as i32 {
                let a = l.px[(py * l.w as i32 + px) as usize] >> 24;
                if a == 0 {
                    continue;
                }
                let a = if pass == 0 { a * 3 / 4 } else { a };
                for yy in 0..s {
                    for xx in 0..s {
                        let (sx, sy) = (x + px * s + xx + dx, y + py * s + yy + dy);
                        if sx >= cx0 && sy >= cy0 && sx < cx1 && sy < cy1 {
                            c.put(sx, sy, ink, a);
                        }
                    }
                }
            }
        }
    }
}

/// hero2's `draw_gear`: a round key with a cog — eight square teeth, a white wheel, its hollow
/// and its hub.
fn draw_gear(c: &mut Canvas, g: Rect, on: bool) {
    round_face(c, g, if on { BTN_PRESSED } else { BTN }, BORDER, 92, 165);
    let (cx, cy) = g.centre();
    let r = (g.w.min(g.h) / 2 - g.w / 7).max(6);
    let tooth = (r / 3).max(3);
    for (dx, dy) in [(0, -10), (7, -7), (10, 0), (7, 7), (0, 10), (-7, 7), (-10, 0), (-7, -7)] {
        let (tx, ty) = (cx + dx * r / 10 - tooth / 2, cy + dy * r / 10 - tooth / 2);
        c.rect(Rect::new(tx, ty, tooth, tooth), WHITE, 255, WHITE, 255, 0);
    }
    let (fx, fy, fr) = (f64::from(cx), f64::from(cy), f64::from(r));
    c.disc(fx, fy, fr, WHITE, 255, WHITE, 0, 0.0);
    c.disc(fx, fy, fr * 11.0 / 20.0, BTN, 255, WHITE, 0, 0.0);
    c.disc(fx, fy, fr / 4.0, WHITE, 255, WHITE, 0, 0.0);
}

/// The whole HUD for the current state, over the picture already in `dst`.
pub fn draw_hud(dst: &mut [u32], win_w: u32, win_h: u32, hud: &Hud) {
    let mut c = Canvas {
        px: dst,
        w: win_w as i32,
        h: win_h as i32,
    };
    let l = hud.layout();
    let held: HashSet<KeyCode> = hud.held();

    // The minimap (Phase 6-5), under the controls: hero2's panel, or the button that brings it back
    // (hero2's folded map). Under them, unlike hero2's: the panel is no placement obstacle here and
    // a control the player put on it wins the touch (`hud::hit`), so it must show whole.
    if let Some((map, marks, r)) = hud.minimap_shown() {
        draw_minimap(&mut c, map, marks, r, hud.guide_tile(map.id));
    } else if let Some(r) = hud.minimap_restore_shown() {
        draw_mask_button(&mut c, r, crate::ui_icons::Icon::MinimapToggle);
    }

    let stats = hud.stats_shown();
    // The layout editor and the pad screen draw the controls themselves, as they are being set up.
    if hud.editor().is_none() && !hud.remap_open() {
        // hero2's order: the EXP bar under the controls — it sits between the stick and OK, no
        // obstacle to them, so a control the player puts over it stays whole…
        if let Some(st) = stats.as_ref().filter(|_| hud.bar_off()) {
            draw_exp(&mut c, hud, st);
        }
        // …and the 창고 question's answer line with it: clear of the controls on a phone, under
        // them where a squarer window has no room between the arrows and the plates.
        if hud.answer_hint_shown() {
            draw_answer_hint(&mut c, hud);
        }
        // hero2's stick: at rest only a hint where the thumb goes (a faint disc and its thumb, no
        // arrows — the player read the old placeholder as a real control), brighter under a finger.
        if let Some(st) = l.stick {
            let (base, rim, thumb) = if hud.joy().is_some() { (55, 90, 105) } else { (18, 34, 40) };
            let ((cx, cy), (tx, ty)) = match hud.joy() {
                Some(j) => {
                    let (dx, dy) = (j.tx - j.cx, j.ty - j.cy);
                    let (d, max) = ((dx * dx + dy * dy).sqrt(), st.base_r * (1.0 - hud::THUMB));
                    let k = if d > max { max / d } else { 1.0 };
                    ((j.cx, j.cy), (j.cx + dx * k, j.cy + dy * k))
                }
                None => {
                    let (cx, cy) = st.seat.centre();
                    let at = (f64::from(cx), f64::from(cy));
                    (at, at)
                }
            };
            c.disc(cx, cy, st.base_r, STICK_BASE, base, BORDER, rim, 2.8);
            c.disc(tx, ty, st.base_r * hud::THUMB, STICK_THUMB, thumb, STICK_THUMB, 0, 0.0);
        }
        let quick = hud.quick_shown();
        for ctl in &l.controls {
            let on = matches!(ctl.action, Action::Key(k) if held.contains(&k));
            draw_hud_control(&mut c, ctl, on, quick, l.screen.desktop);
        }
        // …the status block (Phase 6-1) over them, then the top tabs (Phase 7: SAVE, and MAP once
        // the world map exists) and the gear.
        if let Some(st) = stats.as_ref() {
            draw_status(&mut c, hud, st);
        }
        for (r, tab) in hud.tabs_shown() {
            draw_top_tab(&mut c, *r, tab.label(), false);
        }
        if let Some(g) = l.gear {
            draw_gear(&mut c, g, hud.settings_open());
        }
    }

    if hud.settings_open() {
        draw_settings(&mut c, hud);
    }

    // A host window (the PC's dialogs): over the settings, under the notices' place.
    if let Some(w) = hud.host() {
        draw_host(&mut c, hud, w);
    }

    if hud.remap_open() {
        draw_remap(&mut c, hud);
    }

    if let Some(ed) = hud.editor() {
        draw_editor(&mut c, hud, ed);
    }

    // The gamepad badges over the controls (after the panel and the editor: `badges()` is empty
    // in the editor, and a panel's row must not be read through a badge).
    if !hud.settings_open() {
        for (b, pos) in hud.badges() {
            draw_badge(&mut c, b, pos);
        }
    }

    // The world map (Phase 7-2), modal over everything but the tabs (MAP closes it) and the notice.
    if let Some(w) = hud.world() {
        draw_world(&mut c, hud, w);
        // A window's arrows over it (A3, hero2's `draw_menu_dpad`): the cursor walks the roads.
        let held = hud.world_held();
        for ctl in hud.world_arrows() {
            draw_control(&mut c, &ctl, matches!(ctl.action, Action::Key(k) if held.contains(&k)));
        }
        for (r, tab) in hud.tabs_shown() {
            draw_top_tab(&mut c, *r, tab.label(), *tab == hud::Tab::Map);
        }
    }

    // The notices, last: they must be readable over everything — except a host panel, which
    // they covered (the settings' CLOSE row, S9 2026-10-22): its rows or its hint line say what
    // a notice would, and the stack keeps its clocks meanwhile.
    if !hud.settings_open() && !hud.remap_open() && hud.host().is_none() {
        draw_notices(&mut c, hud);
    }
    // The game stopped: over everything, for good.
    if let Some(text) = hud.stuck() {
        draw_fatal(&mut c, hud, text);
    }
}

// ------------------------------------------------------------- the settings window (B2) ----

/// This game's window, as its menu `bh` draws it (S9 2026-10-26, 4.5 window px a guest px): a black
/// edge two guest px wide round a parchment body in four tones a guest px each; row plates dark
/// brown in a brown line with parchment letters, the cursor's row dark red in a brighter red line
/// with white letters; the slot screen's gold (its title, 「LV」) for a word to notice and its grey
/// (a slot not picked) for one that is not.
const WIN_EDGE: u32 = 0x0000_0000;
const WIN_PARCHMENT: [u32; 4] = [0x00c0_b890, 0x00c5_bd97, 0x00cc_c4a1, 0x00d0_c8a8];
const WIN_ROW: u32 = 0x0036_2e23;
const WIN_ROW_LINE: u32 = 0x006b_3b00;
const WIN_ROW_INK: u32 = 0x00c0_b890;
const WIN_CURSOR: u32 = 0x0066_0000;
const WIN_CURSOR_LINE: u32 = 0x0081_0000;
const WIN_GOLD: u32 = 0x00de_be7b;
const WIN_GREY: u32 = 0x007f_7f7f;

/// One guest pixel in window px, whole (the window's lines are this thick).
fn guest_unit(s: &hud::Screen) -> i32 {
    ((f64::from(s.game.h) / f64::from(s.guest_h.max(1))).round() as i32).max(1)
}

/// This game's parchment over `r`: a tone for each guest-px cell counted from `origin`, mixed as
/// the menu's is (the lightest twice as often as the middle two, the base as often as the rest),
/// running three cells up a diagonal as its grain does.
fn parchment(c: &mut Canvas, r: Rect, origin: (i32, i32), u: i32) {
    let b = r.clipped(&Rect::new(0, 0, c.w, c.h));
    if b.is_empty() {
        return;
    }
    let cell = |v: i32, o: i32| (v - o).div_euclid(u);
    for cy in cell(b.y, origin.1)..=cell(b.bottom() - 1, origin.1) {
        for cx in cell(b.x, origin.0)..=cell(b.right() - 1, origin.0) {
            let k = ((cx + cy) as u32).wrapping_mul(0x9e37_79b9) ^ ((cx - cy).div_euclid(6) as u32).wrapping_mul(0x85eb_ca6b);
            let k = (k ^ (k >> 15)).wrapping_mul(0x2c1b_3c6d);
            let tone = match (k >> 16) % 12 {
                0..=5 => 0,
                6 | 7 => 1,
                8 | 9 => 2,
                _ => 3,
            };
            c.fill(Rect::new(origin.0 + cx * u, origin.1 + cy * u, u, u).clipped(&b), WIN_PARCHMENT[tone]);
        }
    }
}

/// The settings window (B2): hero2's — the screen behind dimmed to 40 %, 74 % × 72 % centred, the
/// title with 닫기 at its right, five rows at a time and a scrollbar past them, each row its name on
/// the left and its state on the right, a changed state in gold — in this game's own window: its
/// edge, its parchment, its row plates and its font (hero3's way: the game's own look, not a host
/// panel's).
fn draw_settings(c: &mut Canvas, hud: &Hud) {
    let s = *hud.screen();
    for v in c.px.iter_mut() {
        let x = *v;
        *v = (((x >> 16) & 0xff) * 4 / 10) << 16 | (((x >> 8) & 0xff) * 4 / 10) << 8 | ((x & 0xff) * 4 / 10);
    }
    let g = hud::settings_geom(&s);
    let u = guest_unit(&s);
    c.fill(g.panel, WIN_EDGE);
    let body = g.panel.grown(-2 * u);
    let origin = (body.x, body.y);
    parchment(c, body, origin, u);
    // The rows; then the parchment again over what a scrolled plate puts outside the list.
    let pressed = hud.settings_pressed();
    for (r, a, can) in hud.settings_rows() {
        let (name, state, st) = hud.settings_words(a, can);
        draw_win_row(c, hud, r, u, (name, state), st, can && pressed == Some(a));
    }
    parchment(c, Rect::new(body.x, body.y, body.w, g.view.y - body.y), origin, u);
    parchment(c, Rect::new(body.x, g.view.bottom(), body.w, body.bottom() - g.view.bottom()), origin, u);
    // The title in the game's black ink, level with 닫기, centred on the rows' span (not on the
    // window, so it does not drift towards 닫기) — half the header high, as hero2's and hero3's
    // titles are, so past the game picture's own scale.
    let close = hud::settings_close(&s);
    let (tx, ty) = (g.x0 + g.bw / 2, close.centre().1);
    match hud.text("SETTINGS") {
        Some(t) => {
            let k = (f64::from(g.view.y - body.y) * 0.45 / f64::from(t.h.max(1))).min(f64::from(g.bw / 2) / f64::from(t.w.max(1)));
            let (dw, dh) = ((f64::from(t.w) * k).round() as i32, (f64::from(t.h) * k).round() as i32);
            blit_line(c, t, tx - dw / 2, ty - dh / 2, k, WIN_EDGE, 255);
        }
        None => draw_line(c, hud, "SETTINGS", tx, ty, g.bw * 6 / 10, close.h, WIN_EDGE, 255),
    }
    draw_win_row(c, hud, close, u, ("CLOSE", ""), hud::RowState::Normal, false);
    // The scrollbar while there are rows to scroll to, in the margin right of the plates: a groove
    // in the plates' brown, the thumb in the cursor's red.
    let max = hud::settings_scroll_max(&s, hud.settings_list().len());
    if max > 0 {
        let right = g.x0 + g.bw;
        let gw = (2 * u).max(4);
        let groove = Rect::new(right + (body.right() - right) / 2 - gw / 2, g.view.y, gw, g.view.h);
        c.fill(groove, WIN_ROW);
        let thumb = (g.view.h * g.view.h / (g.view.h + max)).max(4 * u);
        let at = g.view.y + (g.view.h - thumb) * hud.settings_scroll().clamp(0, max) / max;
        c.fill(Rect::new(groove.x, at, gw, thumb), WIN_CURSOR);
    }
}

/// A row plate of this game's window: its line round its fill — the cursor's red while a finger
/// rests on it — the name at the left and the state at the right at ONE size (a bigger name over a
/// smaller state reads as two controls, hero2), or the name centred when there is no state (닫기).
fn draw_win_row(c: &mut Canvas, hud: &Hud, r: Rect, u: i32, (name, state): (&str, &str), st: hud::RowState, cursor: bool) {
    let (line, fill, ink) = if cursor { (WIN_CURSOR_LINE, WIN_CURSOR, WHITE) } else { (WIN_ROW_LINE, WIN_ROW, WIN_ROW_INK) };
    c.fill(r, line);
    c.fill(r.grown(-u), fill);
    let name_ink = if st == hud::RowState::Disabled { WIN_GREY } else { ink };
    let state_ink = if st == hud::RowState::Active { WIN_GOLD } else { WIN_GREY };
    let (cx, cy) = r.centre();
    if state.is_empty() {
        draw_line(c, hud, name, cx, cy, r.w, r.h * 7 / 10, name_ink, 255);
        return;
    }
    let pad = (r.w / 24).clamp(4, 40);
    let inner = r.w - 2 * pad;
    match (hud.text(name), hud.text(state)) {
        (Some(n), Some(v)) => {
            // The pair and a font cell of daylight across the plate, 70 % of its height, at most
            // the game picture's own scale (`line_scale`).
            let game = f64::from(hud.screen().game.h) / f64::from(hud.screen().guest_h.max(1));
            let k = game
                .min(f64::from(inner) / f64::from((n.w + v.w + n.h).max(1)))
                .min(f64::from(r.h) * 0.7 / f64::from(n.h.max(v.h).max(1)));
            let top = |ic: &hz_java::Icon| cy - (f64::from(ic.h) * k).round() as i32 / 2;
            blit_line(c, n, r.x + pad, top(n), k, name_ink, 255);
            blit_line(c, v, r.right() - pad - (f64::from(v.w) * k).round() as i32, top(v), k, state_ink, 255);
        }
        // Not lettered yet (before the title): the host's 5×7 ASCII, the pair at one scale.
        _ => {
            let both = font::text_width(name, 1) + font::text_width(state, 1) + 2 * font::GLYPH_H;
            let scale = ((inner as u32 * 85 / 100) / both.max(1)).min((r.h as u32 * 45 / 100) / font::GLYPH_H).max(1);
            c.text_at(name, r.x + pad, cy, scale, name_ink, 255);
            c.text_at(state, r.right() - pad - font::text_width(state, scale) as i32, cy, scale, state_ink, 255);
        }
    }
}

// ------------------------------------------------------ the host windows (the PC's) ----

use crate::hostwin::{HostWin, Text};

/// The PC's quick bar: the cell's key in its top-left corner, white on a dark pad.
fn draw_cell_key(c: &mut Canvas, b: Rect, label: &str) {
    if label.is_empty() {
        return;
    }
    let scale = (b.h as u32 / 26).max(1);
    let (tw, th) = (font::text_width(label, scale) as i32, (font::GLYPH_H * scale) as i32);
    let pad = (b.h / 14).max(2);
    let r = Rect::new(b.x + pad, b.y + pad, tw + pad, th + pad);
    c.rect(r, CD_VEIL, 170, CD_VEIL, 0, 0);
    c.text_at(label, r.x + pad / 2, r.y + r.h / 2, scale, WHITE, 240);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// The picture a host line has, if the game's font lettered it.
fn host_icon<'a>(hud: &'a Hud, t: &Text) -> Option<&'a hz_java::Icon> {
    match t {
        Text::Line(k) => hud.text(k),
        Text::Dyn { id, .. } => hud.text_id(*id),
        Text::Ascii(_) => None,
    }
}

/// `t` with its left, centre or right at `x`, centred on the line `cy`, within `w × h`: the game's
/// font at the picture's scale or less once lettered, else the 5×7 font (`Text::fallback`).
#[allow(clippy::too_many_arguments)]
fn draw_text(c: &mut Canvas, hud: &Hud, t: &Text, x: i32, cy: i32, w: i32, h: i32, align: Align, ink: u32) {
    let w = w.max(1);
    match host_icon(hud, t) {
        Some(icon) => {
            let k = line_scale(hud.screen(), icon, w, h * 8 / 10);
            let dw = (f64::from(icon.w) * k).round() as i32;
            let dh = (f64::from(icon.h) * k).round() as i32;
            let x0 = match align {
                Align::Left => x,
                Align::Centre => x - dw / 2,
                Align::Right => x - dw,
            };
            blit_line(c, icon, x0, cy - dh / 2, k, ink, 255);
        }
        None => {
            let s = t.fallback();
            // Up to 60 % of the row (`fit_scale` reads 45 %), never past the picture's own scale.
            let scale = Canvas::fit_scale(&s, w, h + h / 3).min((f64::from(hud.screen().game.h) / f64::from(hud.screen().guest_h.max(1))).round().max(1.0) as u32);
            let tw = font::text_width(&s, scale) as i32;
            let x0 = match align {
                Align::Left => x,
                Align::Centre => x - tw / 2,
                Align::Right => x - tw,
            };
            c.text_at(&s, x0, cy, scale, ink, 255);
        }
    }
}

/// A host window (`crate::hostwin`, the PC's dialogs) in the settings window's look: the screen
/// dimmed, the panel, the parchment, the title and 닫기 — and its rows: a plate for one that acts
/// (the cursor's in the game's red), the text in the game's black ink for one that does not, a
/// value at the right in the plates' brown.
fn draw_host(c: &mut Canvas, hud: &Hud, win: &HostWin) {
    let s = *hud.screen();
    for v in c.px.iter_mut() {
        let x = *v;
        *v = (((x >> 16) & 0xff) * 4 / 10) << 16 | (((x >> 8) & 0xff) * 4 / 10) << 8 | ((x & 0xff) * 4 / 10);
    }
    let g = hud::host_geom(&s);
    let u = guest_unit(&s);
    c.fill(g.panel, WIN_EDGE);
    let body = g.panel.grown(-2 * u);
    let origin = (body.x, body.y);
    parchment(c, body, origin, u);
    let pressed = hud.host_pressed();
    let pad = (g.bw / 40).clamp(4, 40);
    for (i, r) in hud::host_rows(&s, win.rows.len(), win.scroll) {
        let row = &win.rows[i];
        let (cx, cy) = r.centre();
        match &row.act {
            Some(_) => {
                let cursor = win.cursor == Some(i) || pressed == Some(i);
                let (line, fill, ink) = if cursor { (WIN_CURSOR_LINE, WIN_CURSOR, WHITE) } else { (WIN_ROW_LINE, WIN_ROW, WIN_ROW_INK) };
                c.fill(r, line);
                c.fill(r.grown(-u), fill);
                match &row.value {
                    None => draw_text(c, hud, &row.text, cx, cy, r.w - 2 * pad, r.h * 7 / 10, Align::Centre, ink),
                    Some(v) => {
                        draw_text(c, hud, &row.text, r.x + pad, cy, r.w * 58 / 100, r.h * 7 / 10, Align::Left, ink);
                        draw_text(c, hud, v, r.right() - pad, cy, r.w * 38 / 100, r.h * 7 / 10, Align::Right, if cursor { WHITE } else { WIN_GOLD });
                    }
                }
            }
            None => match &row.value {
                None => draw_text(c, hud, &row.text, r.x + pad, cy, r.w - 2 * pad, r.h * 8 / 10, Align::Left, WIN_EDGE),
                Some(v) => {
                    draw_text(c, hud, &row.text, r.x + pad, cy, r.w * 55 / 100, r.h * 8 / 10, Align::Left, WIN_EDGE);
                    draw_text(c, hud, v, r.right() - pad, cy, r.w * 42 / 100, r.h * 8 / 10, Align::Right, WIN_ROW_LINE);
                }
            },
        }
    }
    parchment(c, Rect::new(body.x, body.y, body.w, g.view.y - body.y), origin, u);
    parchment(c, Rect::new(body.x, g.view.bottom(), body.w, body.bottom() - g.view.bottom()), origin, u);
    let close = hud::settings_close(&s);
    let (tx, ty) = (g.x0 + g.bw / 2, close.centre().1);
    draw_text(c, hud, &win.title, tx, ty, g.bw * 6 / 10, (g.view.y - body.y) * 55 / 100, Align::Centre, WIN_EDGE);
    draw_win_row(c, hud, close, u, ("CLOSE", ""), hud::RowState::Normal, false);
    let max = hud::host_scroll_max(&s, win.rows.len());
    if max > 0 {
        let right = g.x0 + g.bw;
        let gw = (2 * u).max(4);
        let groove = Rect::new(right + (body.right() - right) / 2 - gw / 2, g.view.y, gw, g.view.h);
        c.fill(groove, WIN_ROW);
        let thumb = (g.view.h * g.view.h / (g.view.h + max)).max(4 * u);
        let at = g.view.y + (g.view.h - thumb) * win.scroll.clamp(0, max) / max;
        c.fill(Rect::new(groove.x, at, gw, thumb), WIN_CURSOR);
    }
}

/// The pad screen's lights (hero2's): gold = a control you can give a button (the action disc's);
/// the badges' warm gold = the one picked, waiting or just changed; blue = what the last bare press
/// fires.
const PM_OFFER: u32 = GOLD;
const PM_SEL: u32 = BADGE_LIT;
const PM_PROBE: u32 = 0x006c_d0ff;

/// The pad screen (B6, hero2's `draw_padmap`): the field HUD as it is played — lighter dimmed than
/// the layout editor, this is the game with its buttons lit, not a blueprint of it — every control a
/// pad button can be given ringed and badged with the DRAFT's button, and this game's window with
/// the two lines, the cells and 취소 · 저장 · 기본값.
fn draw_remap(c: &mut Canvas, hud: &Hud) {
    let Some(r) = hud.remap() else { return };
    let now = std::time::Instant::now();
    let sc = hud.remap_scene();
    c.rect(hud.screen().window(), 0, 115, 0, 0, 0);
    // The HUD by its own painters (the plates show the slots once read).
    let quick = hud.quick_bar();
    for ctl in &sc.layout.controls {
        draw_hud_control(c, ctl, false, quick, false);
    }
    for (r, tab) in &sc.layout.tabs {
        draw_top_tab(c, *r, tab.label(), false);
    }
    let lit = |row: usize| {
        if r.sel == Some(row) {
            PM_SEL
        } else if r.probe == Some(row) {
            PM_PROBE
        } else {
            PM_OFFER
        }
    };
    let pulse = hud.remap_pulse_at(now);
    let breath = pulse.unwrap_or(0.0);
    for t in sc.targets.iter().filter(|t| t.shape != hud::TargetShape::Chip) {
        let colour = lit(t.row);
        let offer = colour == PM_OFFER;
        let sel = r.sel == Some(t.row);
        let b = t.rect;
        if t.shape == hud::TargetShape::Round {
            let rad = f64::from(b.w.min(b.h)) / 2.0;
            let (cx, cy) = (f64::from(b.x) + f64::from(b.w) / 2.0, f64::from(b.y) + f64::from(b.h) / 2.0);
            let th = (rad / 7.0).max(2.5);
            if sel {
                c.disc_soft(cx, cy, rad, PM_SEL, 60 + (70.0 * breath) as u32, (rad * 0.08).max(1.0));
                if pulse.is_some() {
                    c.ring_soft(cx, cy, rad + th * (1.4 + 1.6 * breath), th * 0.8, PM_SEL, (220.0 * (1.0 - 0.7 * breath)) as u32);
                }
            }
            let (thick, a) = if offer { (th, 215) } else { (th * 1.5, 255) };
            c.ring_soft(cx, cy, rad + thick * 0.15, thick, colour, a);
        } else {
            let th = (b.w.min(b.h) / 10).max(3);
            if sel {
                c.rect(b, PM_SEL, 50 + (70.0 * breath) as u32, 0, 0, 0);
            }
            let t2 = if offer { th } else { th * 3 / 2 };
            c.rect(b.grown(t2 / 2), 0, 0, colour, if offer { 215 } else { 255 }, t2);
        }
    }
    // The badges over everything, as in play.
    let badges = hud.remap_badges(&sc);
    let s = *hud.screen();
    for (t, pos) in sc.targets.iter().zip(&badges) {
        if t.shape != hud::TargetShape::Chip
            && let (Some(pos), Some(b)) = (pos, crate::padhud::badge_at(t.rect, s.w, s.h))
        {
            draw_badge(c, b, *pos);
        }
    }
    draw_remap_panel(c, hud, &sc, &badges, &lit);
}

/// The pad screen's window: this game's window, as the settings window is (B2) — the black edge,
/// the parchment, the title in its black ink between 취소 and 저장 (lit gold while there is something
/// to keep), the two lines on a row plate (what happened, what to do), the cells as row plates
/// ringed as the controls are with a badge at their right end, and 기본값 (grey with nothing to undo).
fn draw_remap_panel(c: &mut Canvas, hud: &Hud, sc: &hud::RemapScene, badges: &[Option<PadPos>], lit: &dyn Fn(usize) -> u32) {
    let p = &sc.panel;
    let u = p.unit;
    c.fill(p.panel, WIN_EDGE);
    parchment(c, p.body, (p.body.x, p.body.y), u);
    let (tx, ty) = p.title.centre();
    draw_segs(c, hud, &["PAD REMAP"], tx, ty, p.title.w * 9 / 10, p.title.h * 6 / 10, WIN_EDGE, true);
    // A plate as the game's rows are; `lit` = the game's cursor row (red, white letters) — 저장 while
    // the draft has something to keep (hero2 lights it gold; this game's gold is too near its ink).
    let plate = |c: &mut Canvas, r: Rect, word: &str, ink: u32, lit: bool| {
        let (line, fill, ink) = if lit { (WIN_CURSOR_LINE, WIN_CURSOR, WHITE) } else { (WIN_ROW_LINE, WIN_ROW, ink) };
        c.fill(r, line);
        c.fill(r.grown(-u), fill);
        let (cx, cy) = r.centre();
        draw_segs(c, hud, &[word], cx, cy, r.w - 2 * u, r.h * 6 / 10, ink, true);
    };
    plate(c, p.cancel, "CANCEL", WIN_ROW_INK, false);
    plate(c, p.save, "SAVE", WIN_ROW_INK, hud.remap_dirty());
    plate(c, p.reset, "RESET", if hud.remap_can_reset() { WIN_ROW_INK } else { WIN_GREY }, false);
    // The two lines.
    c.fill(p.status, WIN_ROW_LINE);
    c.fill(p.status.grown(-u), WIN_ROW);
    let lines = hud.remap_lines_at(std::time::Instant::now());
    let lh = (p.status.h - 2 * u) / 2;
    for (k, line) in lines.iter().enumerate() {
        let cy = p.status.y + u + lh * k as i32 + lh / 2;
        draw_segs(c, hud, line, p.status.x + 3 * u, cy, p.status.w - 6 * u, lh * 8 / 10, if k == 0 { WHITE } else { WIN_GOLD }, false);
    }
    // The cells.
    for (t, pos) in sc.targets.iter().zip(badges) {
        if t.shape != hud::TargetShape::Chip {
            continue;
        }
        let r = t.rect;
        let colour = lit(t.row);
        c.fill(r, WIN_ROW);
        if colour == PM_SEL {
            c.rect(r, PM_SEL, 60 + (70.0 * hud.remap_pulse_at(std::time::Instant::now()).unwrap_or(0.0)) as u32, 0, 0, 0);
        }
        let ring = if colour == PM_OFFER { u.max(1) } else { 2 * u.max(1) };
        c.rect(r, 0, 0, colour, if colour == PM_OFFER { 200 } else { 255 }, ring);
        let br = f64::from(r.h) * 0.34;
        let badge = Badge { cx: f64::from(r.right()) - f64::from(r.h) / 2.0, cy: f64::from(r.y) + f64::from(r.h) / 2.0, r: br };
        match pos {
            Some(pos) => draw_badge(c, badge, *pos),
            None => c.disc(badge.cx, badge.cy, br, BADGE_FILL, 225, BADGE_RIM, 190, 2.8),
        }
        let cap = hud.remap_caption(t.row);
        draw_segs(c, hud, &cap, r.x + 3 * u, r.centre().1, r.w - r.h - 5 * u, r.h * 6 / 10, WIN_ROW_INK, false);
    }
}

/// Host lines side by side, a font space apart (the pad screen's lines are put together from
/// pieces — a function, `=`, a button): their Korean in the game's font at one scale — the game
/// picture's or less, to fit `w × h` — from `x` (centred on it with `centre`); while any piece is
/// not lettered yet, all of them in the host's ASCII.
#[allow(clippy::too_many_arguments)]
fn draw_segs(c: &mut Canvas, hud: &Hud, segs: &[&str], x: i32, cy: i32, w: i32, h: i32, color: u32, centre: bool) {
    if segs.is_empty() || w <= 0 || h <= 0 {
        return;
    }
    let icons: Option<Vec<&hz_java::Icon>> = segs.iter().map(|s| hud.text(s)).collect();
    let Some(icons) = icons else {
        let text = segs.join(" ");
        let scale = Canvas::fit_scale(&text, w * 100 / 85, h * 100 / 45);
        let x0 = if centre { x - font::text_width(&text, scale) as i32 / 2 } else { x };
        c.text_at(&text, x0, cy, scale, color, 255);
        return;
    };
    let tall = icons.iter().map(|i| i.h).max().unwrap_or(1).max(1);
    let space = (tall / 3).max(1);
    let total = icons.iter().map(|i| i.w).sum::<u32>() + space * (icons.len() as u32 - 1);
    let game = f64::from(hud.screen().game.h) / f64::from(hud.screen().guest_h.max(1));
    let k = game.min(f64::from(w) / f64::from(total.max(1))).min(f64::from(h) / f64::from(tall));
    let mut at = if centre { f64::from(x) - f64::from(total) * k / 2.0 } else { f64::from(x) };
    for icon in icons {
        let dh = (f64::from(icon.h) * k).round() as i32;
        blit_line(c, icon, at.round() as i32, cy - dh / 2, k, color, 255);
        at += f64::from(icon.w + space) * k;
    }
}

// ------------------------------------------------------------- quick plates (6-2) ----

/// hero2's immersive plate colours (`overlay::draw_skill_buttons`): a dark translucent face, the
/// rim gold for a skill, steel for an item, dim when empty; the pressed face is its blue.
const PLATE_FACE: u32 = 0x0010_1420;
const PLATE_PRESSED: u32 = 0x004a_6cff;
const RIM_SKILL: u32 = 0x00d8_aa38;
const RIM_ITEM: u32 = 0x009d_b4d0;
const RIM_EMPTY: u32 = 0x004a_5568;
/// The cooldown: what is still to recover is veiled, what has recovered fills from the bottom.
const CD_VEIL: u32 = 0x0005_070c;
const CD_FILL: u32 = 0x004a_6cff;

/// A quick-slot key as a plate: what the slot holds (its icon, a potion's stock) and its cooldown —
/// what the game's bar showed. An empty slot is just its dim rim (hero2: the hotkey is what the
/// plate SENDS, not its label). A disc, wherever the layout seats it.
/// A control of the play HUD: a plate shows what it holds once the bar is read (Phase 6-2, HZ-058 —
/// ringed in gold while a window's 「퀵슬롯등록」 waits for one), the page button its page; the PC's
/// cells carry their key in a corner (the keyboard presses them). Everything else its own face.
fn draw_hud_control(c: &mut Canvas, ctl: &Control, on: bool, quick: Option<&hz_java::QuickBar>, desktop: bool) {
    match (ctl.action, quick) {
        (Action::Plate(n), Some(bar)) => {
            let slot = &bar.slots[usize::from(n).min(bar.slots.len() - 1)];
            draw_plate(c, ctl.shape, on, slot, slot.item);
            if bar.picking {
                let r = ctl.shape.bounds();
                let (cx, cy) = r.centre();
                let rad = f64::from(r.w.max(r.h)) / 2.0;
                c.ring_soft(f64::from(cx), f64::from(cy), rad, (rad / 9.0).max(1.6), GOLD, 230);
            }
            if desktop {
                draw_cell_key(c, ctl.shape.bounds(), ctl.label);
            }
        }
        // hero2's arrows, never the page's number (its 「퀵바 페이지 번호 제거」): the plates show the page.
        (Action::Page, _) => {
            draw_control(c, ctl, on);
            if desktop {
                draw_cell_key(c, ctl.shape.bounds(), ctl.label);
            }
        }
        _ => draw_control(c, ctl, on),
    }
}

fn draw_plate(c: &mut Canvas, shape: Shape, on: bool, slot: &hz_java::QuickSlot, item: bool) {
    let empty = slot.icon.is_none();
    let rim = if empty { RIM_EMPTY } else if item { RIM_ITEM } else { RIM_SKILL };
    let (face_a, rim_a) = if empty { (70, 120) } else { (108, 185) };
    let face = if on { PLATE_PRESSED } else { PLATE_FACE };
    let b = shape.bounds();
    let (cx, cy) = (f64::from(b.x) + f64::from(b.w) / 2.0, f64::from(b.y) + f64::from(b.h) / 2.0);
    let rad = f64::from(b.w.min(b.h)) / 2.0;
    let round = matches!(shape, Shape::Circle { .. });
    if round {
        c.disc(cx, cy, rad, face, face_a, rim, rim_a, (rad / 12.0).max(2.0));
    } else {
        c.rect(b, face, face_a, rim, rim_a, (b.h / 18).max(2));
    }
    let Some(icon) = &slot.icon else { return };
    if round {
        blit_icon_disc(c, cx, cy, rad * 0.88, icon);
    } else {
        let side = (f64::from(b.w.min(b.h)) * 0.72).round() as i32;
        blit_icon_rect(c, Rect::square_at(cx as i32, cy as i32, side), icon);
    }
    let left = slot.cooldown_left();
    if left > 0.0 {
        // The recovered share fills from the bottom; the rest is veiled.
        let fill_top = cy + rad - 2.0 * rad * (1.0 - left);
        let (x0, x1) = ((cx - rad).floor() as i32, (cx + rad).ceil() as i32);
        let (y0, y1) = ((cy - rad).floor() as i32, (cy + rad).ceil() as i32);
        for y in y0..y1 {
            let py = f64::from(y) + 0.5;
            for x in x0..x1 {
                let px = f64::from(x) + 0.5;
                let inside = if round { (px - cx).powi(2) + (py - cy).powi(2) <= rad * rad } else { b.contains(px, py) };
                if inside {
                    if py >= fill_top {
                        c.put(x, y, CD_FILL, 96);
                    } else {
                        c.put(x, y, CD_VEIL, 120);
                    }
                }
            }
        }
    }
    // The stock last, over the cooldown too: the potions' shared cooldown must not hide it.
    if item && slot.count > 0 {
        let text = slot.count.min(99).to_string();
        let scale = ((rad * 2.0 / 24.0) as u32).max(2);
        let (tw, th) = (font::text_width(&text, scale) as f64, f64::from(font::GLYPH_H * scale));
        let ty = cy + rad * 0.42 + th / 2.0;
        // A dim pad behind the digits so they read over bright art.
        c.disc(cx, ty, tw.max(th) * 0.72, CD_VEIL, 150, CD_VEIL, 0, 0.0);
        c.text_centred(&text, cx as i32, ty as i32, scale, WHITE, 255);
    }
}

/// Alpha-aware bilinear sample of `icon` at fractional source `(fx, fy)` (hero2's
/// `sample_bilinear`): only opaque texels count, re-weighted, so the silhouette stays crisp with
/// no dark halo; `None` where less than half the weight is opaque.
fn sample_bilinear(icon: &hz_java::Icon, fx: f64, fy: f64) -> Option<u32> {
    let (w, h) = (i64::from(icon.w), i64::from(icon.h));
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (xi, yi) = (x0 as i64, y0 as i64);
    let texel = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= w || y >= h {
            return None;
        }
        let p = icon.px[(y * w + x) as usize];
        (p >> 24 != 0).then(|| (f64::from((p >> 16) & 0xff), f64::from((p >> 8) & 0xff), f64::from(p & 0xff)))
    };
    let (mut r, mut g, mut b, mut sum) = (0.0, 0.0, 0.0, 0.0);
    for (t, wt) in [
        (texel(xi, yi), (1.0 - tx) * (1.0 - ty)),
        (texel(xi + 1, yi), tx * (1.0 - ty)),
        (texel(xi, yi + 1), (1.0 - tx) * ty),
        (texel(xi + 1, yi + 1), tx * ty),
    ] {
        if let Some((cr, cg, cb)) = t {
            r += cr * wt;
            g += cg * wt;
            b += cb * wt;
            sum += wt;
        }
    }
    if sum < 0.5 {
        return None;
    }
    let ch = |v: f64| (v / sum).round().clamp(0.0, 255.0) as u32;
    Some((ch(r) << 16) | (ch(g) << 8) | ch(b))
}

/// The icon in a disc's inscribed box (hero2's `blit_icon`): fitted to 1.5 × `rad` keeping its
/// aspect, its alpha feathered over the disc's outer band and over its own square edge, so the
/// art melts into the round plate instead of being cut by an arc.
fn blit_icon_disc(c: &mut Canvas, cx: f64, cy: f64, rad: f64, icon: &hz_java::Icon) {
    if icon.w == 0 || icon.h == 0 || rad < 3.0 {
        return;
    }
    let (iw, ih) = (f64::from(icon.w), f64::from(icon.h));
    let sc = (rad * 1.5 / iw).min(rad * 1.5 / ih);
    let (dw, dh) = ((iw * sc).round() as i32, (ih * sc).round() as i32);
    if dw <= 0 || dh <= 0 {
        return;
    }
    let (dx0, dy0) = ((cx - f64::from(dw) / 2.0) as i32, (cy - f64::from(dh) / 2.0) as i32);
    let feather = (rad * 0.20).max(2.0);
    let inner = (rad - feather).max(0.0);
    let edge_band = (f64::from(dw.min(dh)) * 0.16).max(1.5);
    for oy in 0..dh {
        for ox in 0..dw {
            let (x, y) = (dx0 + ox, dy0 + oy);
            let d = ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - cy).powi(2)).sqrt();
            if d >= rad {
                continue;
            }
            let k = if d <= inner { 1.0 } else { (rad - d) / feather };
            let mut cov = k * k * (3.0 - 2.0 * k);
            let ex = (f64::from(ox) + 0.5).min(f64::from(dw - ox) - 0.5);
            let ey = (f64::from(oy) + 0.5).min(f64::from(dh - oy) - 0.5);
            let e = (ex.min(ey) / edge_band).clamp(0.0, 1.0);
            cov *= e * e * (3.0 - 2.0 * e);
            let fx = (f64::from(ox) + 0.5) * iw / f64::from(dw) - 0.5;
            let fy = (f64::from(oy) + 0.5) * ih / f64::from(dh) - 0.5;
            if let Some(p) = sample_bilinear(icon, fx, fy) {
                c.put(x, y, p, (255.0 * cov) as u32);
            }
        }
    }
}

/// The icon fitted into `dst` (aspect kept, centred), bilinear, opaque where the art is.
fn blit_icon_rect(c: &mut Canvas, dst: Rect, icon: &hz_java::Icon) {
    if icon.w == 0 || icon.h == 0 || dst.is_empty() {
        return;
    }
    let (iw, ih) = (f64::from(icon.w), f64::from(icon.h));
    let sc = (f64::from(dst.w) / iw).min(f64::from(dst.h) / ih);
    let (dw, dh) = ((iw * sc).round() as i32, (ih * sc).round() as i32);
    let (ox, oy) = (dst.x + (dst.w - dw) / 2, dst.y + (dst.h - dh) / 2);
    for py in 0..dh {
        for px in 0..dw {
            let fx = (f64::from(px) + 0.5) / f64::from(dw) * iw - 0.5;
            let fy = (f64::from(py) + 0.5) / f64::from(dh) * ih - 0.5;
            if let Some(p) = sample_bilinear(icon, fx, fy) {
                c.put(ox + px, oy + py, p, 255);
            }
        }
    }
}

/// The game rect as `hud_view` takes it.
fn game_box(s: &hud::Screen) -> (u32, u32, u32, u32) {
    (s.game.x.max(0) as u32, s.game.y.max(0) as u32, s.game.w.max(0) as u32, s.game.h.max(0) as u32)
}

/// The status block (Phase 6-1, `hud_view` — hero2's hero5-style block with MP added) in the game
/// rect's top-left corner, the corner the game's field leaves alone, with the point badges and the
/// buff cells. The EXP bar is [`draw_exp`].
/// HZ-030: the line under the status block in 가디언 모드 (`host_text` has its Korean), in 「전설」's gold.
pub const GUARDIAN_MODE: &str = "GUARDIAN MODE";
const GUARDIAN_INK: u32 = hz_patches::hooks::LEGEND_COLOUR as u32;

/// That line's box, window px: a quarter of the block tall, a quarter of that under it. The notice
/// stack stops under it ([`draw_notices`]).
fn guardian_line(s: &hud::Screen) -> hud::Rect {
    let b = hud::status_box(s);
    let line_h = (b.h / 4).max(8);
    hud::Rect::new(b.x, b.bottom() + line_h / 4, b.w, line_h)
}

fn draw_status(c: &mut Canvas, hud: &Hud, stats: &hz_java::Stats) {
    let s = hud.screen();
    let (lw, lh) = (s.guest_w.max(1) as u32, s.guest_h.max(1) as u32);
    let n = |v: i32| v.max(0) as u32;
    let g = HvGeom::new(game_box(s), lw, lh);
    let block = HvStats {
        hp: n(stats.hp),
        hp_max: n(stats.hp_max),
        sp: n(stats.sp),
        sp_max: n(stats.sp_max),
        mp: n(stats.mp),
        mp_max: n(stats.mp_max),
        level: n(stats.level),
    };
    let (w, h) = (c.w as u32, c.h as u32);
    // The head in the ring (6-6): the hero's own doll, composed by the hook.
    let head = hud.portrait().map(|p| HvIcon { w: p.w, h: p.h, px: &p.px });
    hv_draw_status(c.px, w, h, &g, &block, head);
    // HZ-030: 가디언 모드, in the game's font under the block (the round the hook read with the stats).
    if stats.round >= hz_patches::hooks::GUARDIAN_ROUND {
        let l = guardian_line(s);
        draw_line(c, hud, GUARDIAN_MODE, l.x + l.w / 2, l.y + l.h / 2, l.w, l.h, GUARDIAN_INK, 255);
    }
    // The unspent-point badges (6-3): the game's own icons, while there are points, blinking.
    if let Some(bar) = hud.quick_shown() {
        let blink = hud.badge_blink_on();
        let badge = |i: usize, points: i32| {
            bar.points[i].as_ref().filter(|_| blink && points > 0).map(|ic| HvIcon { w: ic.w, h: ic.h, px: &ic.px })
        };
        hv_draw_badges(c.px, w, h, &g, [badge(0, stats.stat_points), badge(1, stats.skill_points)]);
        // The timed states (hero2's buff band), each the game's picture and the seconds left.
        let buffs: Vec<HvBuff> =
            bar.buffs.iter().map(|b| HvBuff { icon: b.icon.as_ref().map(|ic| HvIcon { w: ic.w, h: ic.h, px: &ic.px }), secs: b.secs }).collect();
        hv_draw_buffs(c.px, w, h, &g, &buffs);
    }
}

/// The EXP bar at the bottom centre — only when the game's own bar is gone (HZ-013): it would sit
/// on the bar otherwise.
fn draw_exp(c: &mut Canvas, hud: &Hud, stats: &hz_java::Stats) {
    let s = hud.screen();
    let n = |v: i32| v.max(0) as u32;
    let (w, h) = (c.w as u32, c.h as u32);
    let e = HvExp::new(game_box(s), s.guest_w.max(1) as u32, s.guest_h.max(1) as u32);
    hv_draw_exp(c.px, w, h, &e, n(stats.exp), n(stats.exp_next));
    // HZ-022: past 99 the bar is the guardian level's, its number on a plate over it.
    hv_draw_gl(c.px, w, h, &e, u32::from(stats.guardian));
}

/// hero2's `draw_mask_button`: a round key's translucent face with the icon's coverage mask in
/// white over 75 % of it, sampled bilinearly. A missing icon leaves the bare disc.
fn draw_mask_button(c: &mut Canvas, r: Rect, icon: crate::ui_icons::Icon) {
    draw_mask_icon(c, r, icon, 1.5, false, true);
}

/// hero2's `QUICKPAGE_ICON_SPAN`: the page button's arrows span 1.05 radii of its disc.
const QUICKPAGE_ICON_SPAN: f64 = 1.05;

/// hero2's `draw_mask_button_spanned`: the icon `span` radii across, centred in `r` — over hero2's
/// dark disc (darker pressed) when `round`, over whatever the caller drew otherwise.
fn draw_mask_icon(c: &mut Canvas, r: Rect, icon: crate::ui_icons::Icon, span: f64, pressed: bool, round: bool) {
    if r.is_empty() {
        return;
    }
    let (cx, cy) = (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
    let rad = f64::from(r.w.min(r.h)) / 2.0;
    if round {
        c.disc(cx, cy, rad, 0x000a_0c12, if pressed { 150 } else { 90 }, WHITE, 0, 0.0);
    }
    let Some(m) = crate::ui_icons::icons().get(icon) else { return };
    let side = rad * span;
    if side < 1.0 || m.w == 0 || m.h == 0 {
        return;
    }
    let dw = side.round().max(1.0) as i32;
    let (ox, oy) = ((cx - side / 2.0).round() as i32, (cy - side / 2.0).round() as i32);
    let (mw, mh) = (f64::from(m.w), f64::from(m.h));
    let at = |x: f64, y: f64| -> f64 {
        let cl = |v: f64, hi: u32| v.clamp(0.0, f64::from(hi.saturating_sub(1))) as u32;
        f64::from(m.coverage(cl(x, m.w), cl(y, m.h)))
    };
    for py in 0..dw {
        for px in 0..dw {
            let sx = (f64::from(px) + 0.5) / f64::from(dw) * mw - 0.5;
            let sy = (f64::from(py) + 0.5) / f64::from(dw) * mh - 0.5;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
            let bot = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
            let a = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u32;
            if a > 0 {
                c.put(ox + px, oy + py, WHITE, a);
            }
        }
    }
}

/// The minimap panel at `r`: `minimap_view::mm_draw_panel`, the call hero2's overlay makes — the
/// player's tile centred at a fixed zoom, the floor darkened with a light outline, the ways to
/// another map as blue bars (`MapGrid::doors`: not the shop counters and event spots the game's
/// window paints blue too), closed chests as green boxes, monsters red, the player teal. hero2's
/// file blends with an alpha byte; softbuffer wants `0x00RRGGBB`, so the panel's pixels lose it.
///
/// Over it, the quest's guide tile in this map (`guide`, 7-6): the game's yellow 「!」, anchored as
/// the world map's inset anchors it (the icon's top-left hangs from the tile, over the door that
/// runs on from it) and at the same ~1.6 tiles. Drawn only while its middle is in the view.
fn draw_minimap(c: &mut Canvas, map: &hz_java::MapGrid, marks: &hz_java::MapMarks, r: Rect, guide: Option<(u8, u8)>) {
    let (w, h) = (c.w as u32, c.h as u32);
    let area = (r.x.max(0) as u32, r.y.max(0) as u32, r.w.max(0) as u32, r.h.max(0) as u32);
    let drawn = mm_draw_panel(c.px, w, h, area, u32::from(map.w), u32::from(map.h), map.doors(), marks.player, &marks.mobs);
    if let Some((gx, gy)) = guide.filter(|_| drawn) {
        let (x0, y0, sc) = mm_tile_grid(area, u32::from(map.w), u32::from(map.h), marks.player);
        let d = i64::from((sc * 8 / 5).max(area.2.min(area.3) / 12));
        let (tx, ty) = (x0 + i64::from(gx) * i64::from(sc) + i64::from(sc / 4), y0 + i64::from(gy) * i64::from(sc));
        let (cx, cy) = (tx + d / 2, ty + d / 2);
        let (ax, ay) = (i64::from(area.0), i64::from(area.1));
        if (ax..ax + i64::from(area.2)).contains(&cx) && (ay..ay + i64::from(area.3)).contains(&cy) {
            let clip = crate::worldmap_view::WmBox { x: area.0, y: area.1, w: area.2, h: area.3 };
            crate::worldmap_view::wm_draw_quest(c.px, w, h, clip, cx as i32, cy as i32, d as f32 / 2.0);
        }
    }
    for y in area.1..(area.1 + area.3).min(h) {
        for x in area.0..(area.0 + area.2).min(w) {
            c.px[(y * w + x) as usize] &= 0x00ff_ffff;
        }
    }
}

/// Where `minimap_view::mm_draw_tiles` lays the map out in `area` = `(x, y, w, h)`: the screen
/// pixel of tile (0, 0)'s top-left corner and the tile's size. The copy's own sum, from its own
/// `mm_tile_px` and `mm_view_origin` — so a mark drawn over the copy lands on the tile the copy
/// drew, and the copy stays hero2's byte for byte.
fn mm_tile_grid(area: (u32, u32, u32, u32), map_w: u32, map_h: u32, marker: Option<(f32, f32)>) -> (i64, i64, u32) {
    let (ax, ay, aw, ah) = area;
    let sc = mm_tile_px(aw.min(ah));
    let scf = f64::from(sc);
    let centre = match marker {
        Some((mx, my)) if mx.is_finite() && my.is_finite() => (f64::from(mx), f64::from(my)),
        _ => (f64::from(map_w) / 2.0, f64::from(map_h) / 2.0),
    };
    let ox = (mm_view_origin(map_w, centre.0, f64::from(aw) / scf) * scf).round() as i64;
    let oy = (mm_view_origin(map_h, centre.1, f64::from(ah) / scf) * scf).round() as i64;
    (i64::from(ax) - ox, i64::from(ay) - oy, sc)
}

/// hero2's badge colours: a near-black slate disc with a steel rim, the bound member warm (the
/// action disc's gold family), the others the HUD's dim grey.
const BADGE_FILL: u32 = 0x0008_0b14;
const BADGE_RIM: u32 = 0x009f_b6d4;
const BADGE_LIT: u32 = 0x00ff_e0a0;
const BADGE_UNLIT: u32 = 0x0056_607a;

/// A pad position badge (hero2's `draw_pad_badge` + `draw_pad_glyph`, "draw the set, light the
/// bound member"): the four face buttons as a diamond of dots, the shoulders as a 2x2 of bars
/// (bumpers above, triggers below), the middle pair as two pills, the stick presses as two rings
/// with the bound one filled. The lit member is bigger as well as brighter. Positions, never
/// letters (`crate::padhud`).
fn draw_badge(c: &mut Canvas, b: Badge, pos: PadPos) {
    c.disc(b.cx, b.cy, b.r, BADGE_FILL, 225, BADGE_RIM, 190, 2.8);
    let ink = |lit: bool| if lit { BADGE_LIT } else { BADGE_UNLIT };
    let dot = |c: &mut Canvas, dx: f64, dy: f64, lit: bool| {
        let rr = b.r * if lit { 0.30 } else { 0.16 };
        c.disc(b.cx + dx, b.cy + dy, rr, ink(lit), 255, ink(lit), 0, 0.0);
    };
    // A bar centred on (dx, dy).
    let bar = |c: &mut Canvas, dx: f64, dy: f64, half_w: f64, half_h: f64, lit: bool| {
        let r = Rect::new((b.cx + dx - half_w).round() as i32, (b.cy + dy - half_h).round() as i32, (half_w * 2.0).max(1.0) as i32, (half_h * 2.0).max(1.0) as i32);
        c.rect(r, ink(lit), 255, ink(lit), 255, 0);
    };
    match pos {
        PadPos::Face(f) => {
            let d = b.r * 0.46;
            dot(c, 0.0, -d, f == FacePos::Up);
            dot(c, 0.0, d, f == FacePos::Down);
            dot(c, -d, 0.0, f == FacePos::Left);
            dot(c, d, 0.0, f == FacePos::Right);
        }
        PadPos::Shoulder { right, lower } => {
            let (dx, dy, hw) = (b.r * 0.40, b.r * 0.36, b.r * 0.30);
            for (rr, ll) in [(false, false), (true, false), (false, true), (true, true)] {
                let lit = rr == right && ll == lower;
                let hh = (b.r * if lit { 0.15 } else { 0.09 }).max(1.0);
                bar(c, if rr { dx } else { -dx }, if ll { dy } else { -dy }, hw, hh, lit);
            }
        }
        PadPos::Middle { right } => {
            let (dx, hw) = (b.r * 0.42, b.r * 0.30);
            bar(c, -dx, 0.0, hw, b.r * 0.14, !right);
            bar(c, dx, 0.0, hw, b.r * 0.14, right);
        }
        PadPos::StickPress { right } => {
            let (dx, rr) = (b.r * 0.44, b.r * 0.34);
            for side in [false, true] {
                let lit = side == right;
                let x = b.cx + if side { dx } else { -dx };
                c.disc(x, b.cy, rr, ink(lit), 0, ink(lit), 255, 2.8);
                if lit {
                    c.disc(x, b.cy, rr * 0.55, BADGE_LIT, 255, BADGE_LIT, 0, 0.0);
                }
            }
        }
    }
}

/// The cream line inside the game's pickup notice (`g.a(BBBZ)V`'s picture, probe 2026-10-26).
const NOTICE_LINE: u32 = 0x00d0_c8a8;

/// The host's notices (B5 — hero2's stack, `overlay.rs` `draw_notices`): each in this game's own
/// pickup-notice box — black, a cream line a guest px in, the line white in the game's font — at
/// the picture's left edge, the newest lowest and the older sliding up a slot as a new one comes,
/// as many as fit under the status block ([`hud::notice_stack`]). Opaque, as the game's is. Before
/// the game's font has lettered a line, its ASCII in the host's 5×7 in the same box.
fn draw_notices(c: &mut Canvas, hud: &Hud) {
    use crate::touch::Notice;
    let notes = hud.notices_at(std::time::Instant::now());
    if notes.is_empty() {
        return;
    }
    let s = *hud.screen();
    let k = notice_k(&s);
    let px = |gp: f64| (gp * k).round() as i32;
    // Each notice's box, window px: a line's, or the game's own picture (HZ-025).
    let boxes: Vec<(i32, i32)> = notes
        .iter()
        .map(|(n, _)| match n {
            Notice::Picture(p) => (px(f64::from(p.w)), px(f64::from(p.h))),
            Notice::Line(text) => line_box(hud, text, k),
        })
        .collect();
    let gap = px(1.0).max(1);
    let bh = boxes.iter().map(|b| b.1).max().unwrap_or(0);
    let widest = boxes.iter().map(|b| b.0).max().unwrap_or(0);
    let ceiling = match hud.stats_shown() {
        Some(st) if st.round >= hz_patches::hooks::GUARDIAN_ROUND => guardian_line(&s).bottom(),
        Some(_) => hud::status_box(&s).bottom(),
        None => s.game.y,
    };
    let (x, top, fit) = hud::notice_stack(hud.layout(), widest, bh, gap, ceiling);
    for ((n, slot), (bw, h)) in notes.iter().zip(boxes) {
        // The oldest beyond what fits under the status block wait off-screen until they go.
        if slot.ceil() as usize >= fit {
            continue;
        }
        let r = Rect::new(x, top - (slot * (bh + gap) as f32).round() as i32, bw, bh);
        let text = match n {
            // The game's box and letters as it made them, at the game picture's scale, on the row.
            Notice::Picture(p) => {
                blit_picture(c, p, r.x, r.y + (bh - h) / 2, k);
                continue;
            }
            Notice::Line(text) => *text,
        };
        draw_line_box(c, hud, r, text, k);
    }
}

/// How much smaller than the picture the stack is drawn (user 2026-10-29: *"좌측에 안내문구 뜨는거
/// (아이템 주운거 등) 폰트좀 줄여야될거같아"* — at the picture's own scale a pickup's 120 × 16 box was
/// 72 px of a 1080-high S9 screen).
pub const NOTICE_SHRINK: f64 = 2.0 / 3.0;

/// The notices' scale, window px a guest px: [`NOTICE_SHRINK`] of the picture's, WHOLE — the game's
/// boxes and its font are pixel art, and a fractional scale draws their one-pixel lines unevenly.
fn notice_k(s: &hud::Screen) -> f64 {
    (game_k(s) * NOTICE_SHRINK).round().max(1.0)
}

/// The game picture's scale, window px a guest px.
fn game_k(s: &hud::Screen) -> f64 {
    f64::from(s.game.h) / f64::from(s.guest_h.max(1))
}

/// The 5×7 font's scale for a line the game's font has not lettered yet, in a box at `k`.
fn ascii_scale(k: f64) -> u32 {
    (k * 10.0 / f64::from(font::GLYPH_H)).round().max(1.0) as u32
}

/// A host line's box (B5: the game's pickup-notice box grown to the line), window px, `k` window px
/// a guest px.
fn line_box(hud: &Hud, text: &str, k: f64) -> (i32, i32) {
    let ascii = ascii_scale(k);
    let (w, h) = match hud.text(text) {
        Some(icon) => hud::notice_box_gp(icon.w as i32, icon.h as i32),
        None => hud::notice_box_gp((f64::from(font::text_width(text, ascii)) / k).ceil() as i32, 14),
    };
    ((f64::from(w) * k).round() as i32, (f64::from(h) * k).round() as i32)
}

/// A host line in its box at `r`: a guest px of black, the cream line, black again, and the
/// letters white in the middle — the game's font once lettered, else the 5×7 one.
fn draw_line_box(c: &mut Canvas, hud: &Hud, r: Rect, text: &str, k: f64) {
    let ascii = ascii_scale(k);
    let gap = (k.round() as i32).max(1);
    c.fill(r, WIN_EDGE);
    c.fill(r.grown(-gap), NOTICE_LINE);
    c.fill(r.grown(-2 * gap), WIN_EDGE);
    let (cx, cy) = r.centre();
    match hud.text(text) {
        Some(icon) => {
            let (dw, dh) = ((f64::from(icon.w) * k).round() as i32, (f64::from(icon.h) * k).round() as i32);
            blit_line(c, icon, cx - dw / 2, cy - dh / 2, k, WHITE, 255);
        }
        None => c.text_centred(text, cx, cy, ascii, WHITE, 255),
    }
}

/// The 창고 question's answers under it (the user's choice ②, 2026-10-27): one line in the box the
/// host's notices use, where the game's soft-key pictures were.
fn draw_answer_hint(c: &mut Canvas, hud: &Hud) {
    let k = game_k(hud.screen());
    let (w, h) = line_box(hud, hud::ANSWER_HINT, k);
    draw_line_box(c, hud, hud::answer_hint(hud.screen(), w, h), hud::ANSWER_HINT, k);
}

/// A picture of the game's at its top-left `(x0, y0)`, `k` window px a guest px, nearest pixel — as
/// the game's own frame is scaled — opaque where the art is.
fn blit_picture(c: &mut Canvas, icon: &hz_java::Icon, x0: i32, y0: i32, k: f64) {
    let (dw, dh) = ((f64::from(icon.w) * k).round() as i32, (f64::from(icon.h) * k).round() as i32);
    if icon.px.len() < (icon.w * icon.h) as usize || dw <= 0 || dh <= 0 {
        return;
    }
    for py in 0..dh {
        let sy = ((f64::from(py) / k) as u32).min(icon.h - 1);
        for px in 0..dw {
            let sx = ((f64::from(px) / k) as u32).min(icon.w - 1);
            let p = icon.px[(sy * icon.w + sx) as usize];
            if p >> 24 >= 0x80 {
                c.put(x0 + px, y0 + py, p & 0x00ff_ffff, 255);
            }
        }
    }
}

/// The game stopped (hero2's `draw_fatal_notice`): not a notice — nothing will be drawn again, so
/// it stays — the frozen picture dimmed (a last frame of a running game reads as one), the line in
/// a dark red box with a rust edge at its middle; the whole window while there is no picture yet.
fn draw_fatal(c: &mut Canvas, hud: &Hud, text: &str) {
    let s = *hud.screen();
    let game = if s.game.is_empty() { s.window() } else { s.game };
    c.rect(game, 0, 150, 0, 0, 0);
    let k = f64::from(game.h) / f64::from(s.guest_h.max(1));
    let pad = ((f64::from(game.h) / 260.0).clamp(2.0, 5.0) * 6.0) as i32;
    let icon = hud.text(text);
    let k = icon.map_or(k, |i| k.min(f64::from(game.w - 4 * pad) / f64::from(i.w.max(1))));
    let scale = Canvas::fit_scale(text, game.w - 2 * pad, game.h / 8).min(4);
    let (tw, th) = match icon {
        Some(i) => ((f64::from(i.w) * k).round() as i32, (f64::from(i.h) * k).round() as i32),
        None => (font::text_width(text, scale) as i32, (font::GLYPH_H * scale) as i32),
    };
    let (cx, cy) = game.centre();
    let b = Rect::new(cx - tw / 2 - pad, cy - th / 2 - pad, tw + 2 * pad, th + 2 * pad);
    c.rect(b, 0x0010_0000, 235, 0x00c8_5a3c, 255, 2);
    match icon {
        Some(i) => blit_line(c, i, cx - tw / 2, cy - th / 2, k, WHITE, 255),
        None => c.text_centred(text, cx, cy, scale, WHITE, 255),
    }
}

/// The scale a game-font line is drawn at (B1): the game picture's own — so it reads like the
/// game's windows — or less where `w × h` is smaller.
fn line_scale(s: &hud::Screen, icon: &hz_java::Icon, w: i32, h: i32) -> f64 {
    let game = f64::from(s.game.h) / f64::from(s.guest_h.max(1));
    let fit = (f64::from(w) / f64::from(icon.w.max(1))).min(f64::from(h) / f64::from(icon.h.max(1)));
    game.min(fit).max(0.0)
}

/// A host line (`crate::host_text`) centred on (`cx`, `cy`) inside `w × h`: its Korean in the game's
/// own font once lettered ([`line_scale`], nearest pixel as the game's picture is, the lettering's
/// coverage as alpha in `color`), else its ASCII in the host's 5x7 font, as before B1.
fn draw_line(c: &mut Canvas, hud: &Hud, en: &str, cx: i32, cy: i32, w: i32, h: i32, color: u32, alpha: u32) {
    let Some(icon) = hud.text(en) else {
        c.text_centred(en, cx, cy, Canvas::fit_scale(en, w, h), color, alpha);
        return;
    };
    let k = line_scale(hud.screen(), icon, w * 92 / 100, h * 9 / 10);
    let (dw, dh) = ((f64::from(icon.w) * k).round() as i32, (f64::from(icon.h) * k).round() as i32);
    blit_line(c, icon, cx - dw / 2, cy - dh / 2, k, color, alpha);
}

/// A lettered line with its top left at (`x0`, `y0`), `k` window px a font px — nearest pixel, as
/// the game's picture is — its coverage as alpha in `color`.
fn blit_line(c: &mut Canvas, icon: &hz_java::Icon, x0: i32, y0: i32, k: f64, color: u32, alpha: u32) {
    let (dw, dh) = ((f64::from(icon.w) * k).round() as i32, (f64::from(icon.h) * k).round() as i32);
    if dw <= 0 || dh <= 0 || icon.w == 0 || icon.h == 0 {
        return;
    }
    for py in 0..dh {
        let sy = ((f64::from(py) + 0.5) / k) as u32;
        let sy = sy.min(icon.h - 1);
        for px in 0..dw {
            let sx = (((f64::from(px) + 0.5) / k) as u32).min(icon.w - 1);
            let a = icon.px[(sy * icon.w + sx) as usize] >> 24;
            if a > 0 {
                c.put(x0 + px, y0 + py, color, a * alpha / 255);
            }
        }
    }
}

/// The layout editor's hint under its row (hero2's, short — a sentence across the window ran out
/// under the ring plates and read as scribbled over them).
pub const EDITOR_HINT: &str = "TAP - DRAG - PINCH";

/// hero2's editor colours (`overlay.rs` `draw_layout_editor`): the boxes on its dark panel, the one
/// selected in the action disc's gold; PAD yellow in both states, the shade being the state (its
/// player: *"노란색 배경의 버튼 … ON/OFF 표시는 배경색으로"*); RESET and CANCEL — the two that throw
/// changes away — their own dark red, so neither is tapped by mistake beside SAVE.
const EDIT_PANEL: u32 = 0x001a_1c26;
const EDIT_PAD_ON: u32 = 0x00f2_c230;
const EDIT_PAD_OFF: u32 = 0x005a_4812;
const EDIT_PAD_INK_ON: u32 = 0x0020_1804;
const EDIT_PAD_INK_OFF: u32 = 0x00e0_c878;
const EDIT_DANGER: u32 = 0x003a_2430;

/// The layout editor (A9 — hero2's): the picture dimmed to 45 %, the obstacles in red (each wins a
/// touch first, so the refusal to place a control there reads as a rule), every control as the
/// shape it will be — the stick's square with the four arrows it is cut into and, while the field
/// walks with the stick, the stick's base — the selected one in gold with an inner ring, its name
/// over its size; PAD stamps the badges; then the row and the hint.
fn draw_editor(c: &mut Canvas, hud: &Hud, ed: &crate::touch::Editor) {
    let s = *hud.screen();
    let sel = ed.sel;
    c.rect(s.window(), 0, 140, 0, 0, 0);
    let chrome = hud::editor_chrome(&s);
    for o in hud::obstacles(&s) {
        if !chrome.iter().any(|(r, _)| *r == o) {
            c.rect(o, RED, 60, RED, 160, 2);
        }
    }
    // The minimap has more: the status block and the top tabs.
    if sel == Some(CtrlId::Minimap) {
        for o in hud::minimap_keep_clear(&s) {
            c.rect(o, RED, 60, RED, 160, 2);
        }
    }
    let rects = hud.editor_rects();
    for &(id, r) in &rects {
        let on = sel == Some(id);
        let rim = if on { GOLD } else { BORDER };
        let (fill, fill_a) = if on { (GOLD, 70) } else { (EDIT_PANEL, 120) };
        let (cx, cy) = (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
        if id.is_round() {
            let rad = f64::from(r.w.min(r.h)) / 2.0;
            c.disc_soft(cx, cy, rad, fill, fill_a, (rad * 0.08).max(1.0));
            c.ring_soft(cx, cy, rad - 1.0, 2.0, rim, 255);
            if on {
                c.ring_soft(cx, cy, rad * 0.8, (rad / 16.0).max(1.2), rim, 200);
            }
        } else {
            c.rect(r, fill, fill_a, rim, 255, 2);
            if on {
                let i = (r.w.min(r.h) / 12).max(2);
                c.rect(r.grown(-i), 0, 0, rim, 255, 2);
            }
        }
        // hero3 91차: the stick's box shows all the one seat governs — the arrows a window (and the
        // field under 십자키) gets, and the stick's base disc where the field walks with the stick.
        if id == CtrlId::Stick {
            if hud.control() != hz_java::ControlMode::Dpad {
                c.disc(cx, cy, f64::from(r.w.min(r.h)) / 2.0 * 0.62, STICK_BASE, 50, rim, 200, 3.0);
            }
            for (ink, key) in hud::dpad_arms(r) {
                hex_face(c, ink, key, BTN_DPAD, rim, 70, 150);
            }
        }
        // The stick's caption in the hole between the four arrows' inner tips, not across the
        // whole square (it would bury the very arrows it is previewing).
        let lr = if id == CtrlId::Stick {
            let side = r.w.min(r.h);
            let hole = (side - side / 50 * 2 - side * hud::ARM_LEN_PCT / 50).max(1);
            Rect::square_at(r.centre().0, r.centre().1, hole)
        } else {
            r
        };
        let label = id.label();
        let (lx, ly) = lr.centre();
        if on {
            let pct = format!("{}%", hud.table().get(id).map_or(crate::placement::PCT_DEFAULT, |p| p.pct));
            // hero2's `two_line`: both lines fitted, from the glyph height, not placed by eye.
            let chars = label.len().max(pct.len()) as u32 + 1;
            let by_h = (lr.h as u32 * 88 / 100 / (font::GLYPH_H * 2 + 2)).max(1);
            let by_w = (lr.w as u32 * 86 / 100 / (chars * (font::GLYPH_W + 1))).max(1);
            let k = by_h.min(by_w).clamp(1, 8);
            let line = (font::GLYPH_H * k) as i32 + (k as i32 * 3 / 7).max(1);
            c.text_centred(label, lx, ly - line / 2, k, WHITE, 255);
            c.text_centred(&pct, lx, ly + line / 2, k, GOLD, 255);
        } else {
            let k = Canvas::fit_scale(label, lr.w * 7 / 10, lr.h * 7 / 10);
            c.text_centred(label, lx, ly, k, WHITE, 255);
        }
    }
    // PAD: the in-play badges, after every box (a badge painted with its own box would end up
    // under the next one's fill wherever two overlap).
    if ed.pad {
        for &(id, r) in &rects {
            if let (Some(pos), Some(b)) = (hud.editor_pad_pos(id), crate::padhud::badge_at(r, s.w, s.h)) {
                draw_badge(c, b, pos);
            }
        }
    }
    // One size for the seven words — the biggest every cell takes — so the row reads as one row.
    let k = chrome
        .iter()
        .filter_map(|(r, a)| hud.text(a.label()).map(|i| line_scale(&s, i, r.w * 88 / 100, r.h * 9 / 10)))
        .fold(f64::INFINITY, f64::min);
    for (r, a) in &chrome {
        let dim = !hud.edit_enabled(*a);
        let (face, ink) = match a {
            _ if dim => (EDIT_PANEL, BORDER),
            hud::EditAction::Pad if ed.pad => (EDIT_PAD_ON, EDIT_PAD_INK_ON),
            hud::EditAction::Pad => (EDIT_PAD_OFF, EDIT_PAD_INK_OFF),
            hud::EditAction::Reset | hud::EditAction::Cancel => (EDIT_DANGER, WHITE),
            _ => (BTN, WHITE),
        };
        c.rect(*r, face, 255, BORDER, 255, 2);
        let (cx, cy) = r.centre();
        match hud.text(a.label()) {
            Some(icon) if k.is_finite() => {
                let (dw, dh) = ((f64::from(icon.w) * k).round() as i32, (f64::from(icon.h) * k).round() as i32);
                blit_line(c, icon, cx - dw / 2, cy - dh / 2, k, ink, 255);
            }
            _ => draw_line(c, hud, a.label(), cx, cy, r.w, r.h, ink, 255),
        }
    }
    // One line just under the row, held to the row's own span.
    if let (Some((first, _)), Some((last, _))) = (chrome.first(), chrome.last()) {
        let w = last.right() - first.x;
        let h = first.h * 3 / 4;
        draw_line(c, hud, EDITOR_HINT, first.x + w / 2, first.bottom() + s.pad() * 2 + h / 2, w, h, WHITE, 235);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hud::Screen, padhud::PadHud, placement::Table};

    /// The status block in the field (Phase 6-1): in the game rect's top-left in both orientations,
    /// and the EXP bar only with the game's bar off (HZ-013).
    #[test]
    fn the_status_block_and_the_exp_bar() {
        let stats = hz_java::Stats { hp: 130, hp_max: 150, sp: 105, sp_max: 105, exp: 40, exp_next: 100, mp: 0, mp_max: 100, level: 3, ..Default::default() };
        for (w, h) in [(2220u32, 1080u32), (1080, 2220)] {
            let (gw, gh) = crate::guest_size(w, h, w > h);
            let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
            hud.set_stats(stats);
            let s = *hud.screen();
            let g = HvGeom::new(game_box(&s), gw, gh);
            let e = HvExp::new(game_box(&s), gw, gh);
            let lit = |dst: &[u32], r: (u32, u32, u32, u32)| (r.1..r.1 + r.3).any(|y| (r.0..r.0 + r.2).any(|x| dst[(y * w + x) as usize] != 0));
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            assert!(lit(&dst, g.ring()) && lit(&dst, g.plate()), "{w}x{h}: the block is drawn");
            assert!(g.bbox().0 >= s.game.x as u32 && g.bbox().1 >= s.game.y as u32, "{w}x{h}: inside the game rect");
            assert!(!lit(&dst, e.rect()), "{w}x{h}: no EXP bar over the game's own bar");
            hud.set_bar_off(true);
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            assert!(lit(&dst, e.rect()), "{w}x{h}: the EXP bar once the game's bar is gone");
            // A7 (hero2's menu mode): both stay over the game's menu, not over the title.
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Menu, ..Default::default() });
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            assert!(lit(&dst, g.ring()) && lit(&dst, e.rect()), "{w}x{h}: over the menu");
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Title, ..Default::default() });
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            assert!(!lit(&dst, g.ring()) && !lit(&dst, e.rect()), "{w}x{h}: not over the title");
        }
    }

    /// B1: a host line once lettered is drawn from the game font's picture — at the game picture's
    /// own scale, nearest pixel, its ink in the line's colour — and before that in the host font.
    #[test]
    fn a_lettered_line_is_drawn_from_the_games_font() {
        let (w, h) = (2220u32, 1080u32);
        let (gw, gh) = crate::guest_size(w, h, true);
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        let white = |hud: &Hud| {
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, hud);
            dst.iter().filter(|&&p| p == WHITE).count()
        };
        // The rest of the HUD's white (the gear's teeth) is not the line's.
        let base = white(&hud);
        let white = |hud: &Hud| white(hud) - base;
        hud.notice("SAVE - NOT NOW");
        let ascii = white(&hud);
        assert!(ascii > 0, "the host font until lettered");
        // A 20 x 14 lettering whose left 10 columns are solid ink.
        let px: Vec<u32> = (0..14).flat_map(|_| (0..20).map(|x| if x < 10 { 0xffff_ffff } else { 0x00ff_ffff })).collect();
        let id = crate::host_text::index("SAVE - NOT NOW").unwrap();
        hud.set_texts(vec![(id, Some(hz_java::Icon { key: hz_java::IconKey::Text(id), w: 20, h: 14, px: px.into() }))]);
        // At the notices' scale: two thirds of the picture's, whole (2026-10-29) — 3 on the S9.
        let k = notice_k(hud.screen());
        assert_eq!(k, (f64::from(hud.screen().game.h) / f64::from(gh) * NOTICE_SHRINK).round());
        let ink = (10.0 * k).round() * (14.0 * k).round();
        let got = white(&hud) as f64;
        assert!((got - ink).abs() <= ink * 0.08, "the ink at the notices' scale ({k}): {got} px, ~{ink} expected");
        hud.set_texts(vec![(id, None)]);
        assert_eq!(white(&hud), ascii, "a line that could not be lettered keeps its ASCII");
    }

    /// Phase 6-6: the portrait the worker sends is drawn inside the ring, alpha dropped; without one
    /// the ring keeps its dark ground.
    #[test]
    fn the_portrait_fills_the_ring() {
        let (w, h) = (2220u32, 1080u32);
        let (gw, gh) = crate::guest_size(w, h, true);
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
        hud.set_stats(hz_java::Stats { hp: 1, hp_max: 1, level: 1, ..Default::default() });
        let g = HvGeom::new(game_box(hud.screen()), gw, gh);
        let (rx, ry, rw, rh) = g.ring();
        let centre = |hud: &Hud| {
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, hud);
            dst[((ry + rh / 2) * w + rx + rw / 2) as usize]
        };
        let ground = centre(&hud);
        let side = hz_java::portrait::PORTRAIT;
        let head = hz_java::Icon { key: hz_java::IconKey::Portrait(1), w: side, h: side, px: std::sync::Arc::from(vec![0xff12_3456u32; (side * side) as usize]) };
        hud.set_portrait(Some(head));
        assert_eq!(centre(&hud), 0x0012_3456, "the head at the ring's centre");
        hud.set_portrait(None);
        assert_eq!(centre(&hud), ground, "no portrait: the dark ground again");
        assert_ne!(ground, 0x0012_3456);
    }

    fn solid_icon(key: hz_java::IconKey, argb: u32) -> hz_java::Icon {
        hz_java::Icon { key, w: 12, h: 12, px: std::sync::Arc::from(vec![argb; 144]) }
    }

    /// A bar with a green character skill on `1`, a cooling-down potion stack on `0`, the rest empty.
    fn sample_bar() -> hz_java::QuickBar {
        let mut bar = hz_java::QuickBar::default();
        bar.slots[0] = hz_java::QuickSlot { icon: Some(solid_icon(hz_java::IconKey::CharSkill { class: 0, cell: 0 }, 0xff00_ff00)), count: 0, cooldown: 0, cooldown_max: 40, item: false };
        bar.slots[4] = hz_java::QuickSlot { icon: Some(solid_icon(hz_java::IconKey::Item(22), 0xffff_0000)), count: 5, cooldown: 5, cooldown_max: 10, item: true };
        let badge = |i: i8, argb: u32| hz_java::Icon { key: hz_java::IconKey::Points(i), w: 8, h: 8, px: std::sync::Arc::from(vec![argb; 64]) };
        bar.points = [Some(badge(0, 0xffd0_2020)), Some(badge(1, 0xff20_60f0))];
        bar
    }

    /// A running state draws hero2's buff cell right of the status block's bars: its icon inside the
    /// silver frame, nothing there without one.
    #[test]
    fn a_buff_draws_its_cell_beside_the_status_block() {
        let (w, h) = (2220u32, 1080u32);
        let field = hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() };
        let draw = |buffs: Vec<hz_java::Buff>| {
            let mut hud = Hud::new(Screen::for_window(w, h), Table::default());
            hud.set_view(field);
            hud.set_stats(hz_java::Stats { hp: 50, hp_max: 100, sp: 10, sp_max: 20, level: 3, ..Default::default() });
            let mut bar = sample_bar();
            bar.buffs = buffs;
            hud.set_quick(bar);
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            let s = hud.screen();
            let g = HvGeom::new(game_box(s), s.guest_w as u32, s.guest_h as u32);
            (dst, g.buff_cell(0), g.u)
        };
        let orb = hz_java::Buff { kind: hz_java::BuffKind::Guard, icon: Some(solid_icon(hz_java::IconKey::Orb, 0xff40_80ff)), secs: 5 };
        let (with, cell, u) = draw(vec![orb]);
        let at = |px: &[u32], lx: u32, ly: u32| px[((cell.1 + ly * u + u / 2) * w + cell.0 + lx * u + u / 2) as usize] & 0x00ff_ffff;
        assert_eq!(at(&with, 1, 1), 0x00de_dbce, "the silver frame");
        assert_eq!(at(&with, 7, 7), 0x0040_80ff, "the icon");
        let (without, _, _) = draw(Vec::new());
        assert_ne!(at(&without, 7, 7), 0x0040_80ff, "no state, no cell");
    }

    /// Phase 6-2: in the field the quick-slot keys are plates showing the slot; elsewhere, and
    /// before the first read, they keep their labels.
    #[test]
    fn the_quick_keys_are_plates_in_the_field() {
        let (w, h) = (2220u32, 1080u32);
        let field = hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() };
        let mut hud = Hud::new(Screen::for_window(w, h), Table::default());
        hud.set_view(field);
        hud.set_quick(sample_bar());
        let mut dst = vec![0u32; (w * h) as usize];
        draw_hud(&mut dst, w, h, &hud);
        let ctl = |n: u8| hud.layout().controls.iter().find(|c| c.action == Action::Plate(n)).unwrap().shape.bounds();
        let px = |x: i32, y: i32| dst[(y * w as i32 + x) as usize];
        let one = ctl(0);
        assert_eq!(px(one.centre().0, one.centre().1), 0x00ff00, "the skill's icon at the plate's centre");
        let zero = ctl(4);
        let lower = (zero.y + zero.h * 3 / 4..zero.bottom()).any(|y| (zero.x..zero.right()).any(|x| px(x, y) == WHITE));
        assert!(lower, "the potion's stock is printed low on the plate");
        // Half cooled down: the top of the plate is veiled darker than the bottom.
        let (zx, _) = zero.centre();
        let lum = |p: u32| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff);
        assert!(lum(px(zx, zero.y + zero.h / 6)) < lum(px(zx, zero.y + zero.h * 5 / 6)) || px(zx, zero.y + zero.h * 5 / 6) == WHITE);
        // An empty slot: no hotkey label (no bright ink) — just its rim.
        let bright = |p: u32| lum(p) > 600;
        let nine = ctl(3);
        let inner = Rect::new(nine.x + nine.w / 4, nine.y + nine.h / 4, nine.w / 2, nine.h / 2);
        assert!((inner.y..inner.bottom()).all(|y| (inner.x..inner.right()).all(|x| !bright(px(x, y)))), "no label on an empty plate");
        // Not the field (or nothing read yet): the label is back.
        for hud in [
            { let mut h = Hud::new(Screen::for_window(w, h), Table::default()); h.set_quick(sample_bar()); h },
            { let mut h = Hud::new(Screen::for_window(w, h), Table::default()); h.set_view(field); h },
        ] {
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            let one = hud.layout().controls.iter().find(|c| c.action == Action::Plate(0)).unwrap().shape.bounds();
            let ink = (one.y..one.bottom()).any(|y| (one.x..one.right()).any(|x| bright(dst[(y * w as i32 + x) as usize])));
            assert!(ink, "the plate's number");
        }
    }

    /// Phase 6-5: in the field with a map, the panel is drawn inside its rect only and leaves
    /// softbuffer's top byte clear; tapped away, the restore button is drawn instead.
    #[test]
    fn the_minimap_draws_in_its_corner() {
        let (w, h) = (2220u32, 1080u32);
        let mut hud = Hud::new(Screen::for_window(w, h), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
        // A 30x30 room of floor with an exit in the middle of the top wall and a chest inside.
        let mut attrs = vec![0i8; 900];
        for i in 0..30 {
            attrs[i] = 64;
            attrs[29 * 30 + i] = 64;
            attrs[i * 30] = 64;
            attrs[i * 30 + 29] = 64;
        }
        attrs[15] = 3;
        hud.set_map(hz_java::MapGrid::new(1, 30, 30, &attrs, &[(5, 5, true)]).unwrap());
        hud.set_marks(hz_java::MapMarks { player: Some((15.5, 15.5)), mobs: vec![(10.5, 12.5)] });
        let bg = 0x0020_3040;
        let mut dst = vec![bg; (w * h) as usize];
        draw_hud(&mut dst, w, h, &hud);
        let r = hud::minimap_panel(hud.screen());
        let changed: Vec<(i32, i32)> = (0..h as i32)
            .flat_map(|y| (0..w as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| r.contains(f64::from(x), f64::from(y)) && dst[(y * w as i32 + x) as usize] != bg)
            .collect();
        assert!(changed.len() > (r.w * r.h / 4) as usize, "the panel is drawn ({} px)", changed.len());
        assert!(dst.iter().all(|p| p >> 24 == 0), "softbuffer wants 0x00RRGGBB");
        let (cx, cy) = r.centre();
        let teal = dst[(cy * w as i32 + cx) as usize];
        assert!((teal >> 8) & 0xff > teal >> 16 && teal & 0xff > teal >> 16, "the player's teal square in the middle: {teal:06x}");
        // Tapped away: the button, and the panel's other pixels untouched.
        hud.started(1, f64::from(cx), f64::from(cy));
        assert_eq!(hud.ended(1), vec![crate::touch::Out::Minimap(false)]);
        let mut dst = vec![bg; (w * h) as usize];
        draw_hud(&mut dst, w, h, &hud);
        let b = hud::minimap_restore(hud.screen(), r);
        let (bx, by) = b.centre();
        assert_ne!(dst[(by * w as i32 + bx) as usize], bg, "the restore button");
        assert_eq!(dst[(cy * w as i32 + cx) as usize], bg, "no panel");
    }

    /// The field minimap's 「!」 (7-6): on a map bigger than the view, scrolled to the player, the
    /// grid sum puts the player where the copy drew its teal square, and the quest's guide tile in
    /// this map gets the game's mark hanging from it — none for another map's tile, none off view.
    #[test]
    fn the_field_minimap_marks_the_guide_tile() {
        let (w, h) = (2220u32, 1080u32);
        let mut hud = Hud::new(Screen::for_window(w, h), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
        let (mw, mh) = (60usize, 40usize);
        let mut attrs = vec![0i8; mw * mh];
        for x in 0..mw {
            attrs[x] = 64;
            attrs[(mh - 1) * mw + x] = 64;
        }
        for y in 0..mh {
            attrs[y * mw] = 64;
            attrs[y * mw + mw - 1] = 64;
        }
        hud.set_map(hz_java::MapGrid::new(7, 60, 40, &attrs, &[]).unwrap());
        let player = (40.5f32, 12.5f32);
        hud.set_marks(hz_java::MapMarks { player: Some(player), mobs: vec![] });
        let r = hud::minimap_panel(hud.screen());
        let (x0, y0, sc) = mm_tile_grid((r.x as u32, r.y as u32, r.w as u32, r.h as u32), 60, 40, Some(player));
        let (sc, d) = (i64::from(sc), i64::from((sc * 8 / 5).max(r.w as u32 / 12)));
        let draw = |hud: &Hud| {
            let mut dst = vec![0x0020_3040u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, hud);
            dst
        };
        let yellow = |dst: &[u32]| -> Vec<(i64, i64)> {
            (0..dst.len()).filter(|&i| dst[i] == 0x00ff_d820).map(|i| ((i % w as usize) as i64, (i / w as usize) as i64)).collect()
        };
        let dst = draw(&hud);
        let teal = dst[((y0 + 25 * sc / 2) * i64::from(w) + x0 + 81 * sc / 2) as usize];
        assert!((teal >> 8) & 0xff > teal >> 16 && teal & 0xff > teal >> 16, "the player's square where the sum says: {teal:06x}");
        assert!(yellow(&dst).is_empty(), "no quest guide, no mark");

        hud.set_guide(hz_java::QuestGuide { quest: Some(2), target: Some(9), tiles: vec![(7, 43, 10)] });
        let marks = yellow(&draw(&hud));
        let (gx, gy) = (x0 + 43 * sc + sc / 4, y0 + 10 * sc);
        assert!(marks.len() as i64 > d * d / 3, "the mark is drawn ({} px)", marks.len());
        assert!(
            marks.iter().all(|&(x, y)| (gx - 2..=gx + d + 2).contains(&x) && (gy - 2..=gy + d + 2).contains(&y)),
            "hanging from the guide tile at ({gx}, {gy}), {d} px"
        );
        hud.set_guide(hz_java::QuestGuide { quest: Some(2), target: Some(9), tiles: vec![(8, 43, 10)] });
        assert!(yellow(&draw(&hud)).is_empty(), "another map's guide tile");
        hud.set_guide(hz_java::QuestGuide { quest: Some(2), target: Some(9), tiles: vec![(7, 2, 38)] });
        assert!(yellow(&draw(&hud)).is_empty(), "a guide tile off the view leaves no stub at the edge");
    }

    #[test]
    fn the_icon_samples_only_its_opaque_texels() {
        let mut px = vec![0u32; 4];
        px[0] = 0xff10_2030;
        let icon = hz_java::Icon { key: hz_java::IconKey::Item(0), w: 2, h: 2, px: std::sync::Arc::from(px) };
        assert_eq!(sample_bilinear(&icon, 0.0, 0.0), Some(0x10_2030));
        assert_eq!(sample_bilinear(&icon, 0.25, 0.0), Some(0x10_2030), "re-weighted, not darkened");
        assert_eq!(sample_bilinear(&icon, 1.0, 1.0), None, "transparent");
    }

    #[test]
    fn integer_blit_repeats_each_source_pixel_exactly() {
        // 2x2 source into a 6x6 game rect at (1,1) of an 8x8 window: each source pixel is a 3x3 block.
        let src = [0xff11_1111, 0xff22_2222, 0xff33_3333, 0xff44_4444];
        let game = Rect::new(1, 1, 6, 6);
        let cols = column_map(6, 2);
        let mut dst = vec![0xdead_beef; 64];
        blit_game(&mut dst, 8, 8, game, &src, 2, 2, &cols);
        assert_eq!(dst[0], 0, "outside the picture is cleared to black");
        assert_eq!(dst[8 + 1], 0x11_1111, "alpha is dropped");
        assert_eq!(dst[3 * 8 + 3], 0x11_1111);
        assert_eq!(dst[3 * 8 + 4], 0x22_2222);
        assert_eq!(dst[4 * 8 + 1], 0x33_3333);
        assert_eq!(dst[6 * 8 + 6], 0x44_4444);
        assert_eq!(dst[7 * 8 + 7], 0);
    }

    #[test]
    fn a_short_source_draws_nothing_instead_of_panicking() {
        let mut dst = vec![1u32; 16];
        blit_game(&mut dst, 4, 4, Rect::new(0, 0, 4, 4), &[0; 3], 2, 2, &column_map(4, 2));
        assert!(dst.iter().all(|&p| p == 0));
    }

    #[test]
    fn blend_endpoints() {
        assert_eq!(blend(0x000000, 0xffffff, 255), 0xffffff);
        assert_eq!(blend(0x123456, 0xffffff, 0), 0x123456);
    }

    #[test]
    fn the_antialiased_disc_is_solid_inside_and_soft_at_the_edge() {
        let mut px = vec![0u32; 41 * 41];
        let mut c = Canvas { px: &mut px, w: 41, h: 41 };
        c.disc(20.5, 20.5, 15.0, WHITE, 255, WHITE, 0, 0.0);
        assert_eq!(px[20 * 41 + 20], 0xffffff);
        assert_eq!(px[0], 0);
        assert!(px.iter().any(|&p| p != 0 && p != 0xffffff), "an edge pixel is partly covered");
    }

    /// A window's set draws its four hexagons (an arrow's ink is lit at its centre, the dead centre
    /// is not) and the gear; a dialogue draws nothing at all — its ▶▶ target is the game's own ink.
    #[test]
    fn a_window_draws_its_arrows_and_a_dialogue_nothing() {
        let (w, h) = (2220u32, 1080u32);
        let draw = |view: hz_java::GuestView| {
            let mut hud = Hud::new(Screen::for_window(w, h), Table::default());
            hud.set_view(view);
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            (hud, dst)
        };
        let (hud, dst) = draw(hz_java::GuestView { scene: hz_java::Scene::Menu, ..Default::default() });
        let px = |x: i32, y: i32| dst[(y * w as i32 + x) as usize];
        for c in hud.layout().controls.iter() {
            if let Face::Hex(ink) = c.face {
                let (x, y) = ink.centre();
                assert_ne!(px(x, y), 0, "{:?}: its glyph", c.action);
            }
        }
        let (cx, cy) = hud.layout().dpad.unwrap().centre();
        assert_eq!(px(cx, cy), 0, "the dead centre is not drawn");
        let g = hud.layout().gear.unwrap();
        assert_ne!(px(g.centre().0, g.centre().1), 0, "the gear's hub");
        let (_, dst) = draw(hz_java::GuestView { scene: hz_java::Scene::Script, ..Default::default() });
        assert!(dst.iter().all(|&p| p == 0), "a dialogue: nothing over the game");
    }

    /// Opt-in visual check: `HZ_PREVIEW_DIR=<dir> cargo test -p hz_android preview -- --ignored`
    /// reads `<dir>/frame.ppm` (a guest frame, binary P6 — e.g. a probe dump converted) and writes
    /// the S9-sized composite of each HUD state as `<dir>/hud_<state>.ppm`.
    #[test]
    #[ignore]
    fn preview() {
        let Ok(dir) = std::env::var("HZ_PREVIEW_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        let raw = std::fs::read(dir.join("frame.ppm")).expect("frame.ppm");
        let mut parts = raw.splitn(4, |b| b.is_ascii_whitespace());
        let (_, gw, gh) = (parts.next(), parts.next().unwrap(), parts.next().unwrap());
        let (gw, gh): (u32, u32) = (
            std::str::from_utf8(gw).unwrap().parse().unwrap(),
            std::str::from_utf8(gh).unwrap().parse().unwrap(),
        );
        let px = &raw[raw.len() - (gw * gh * 3) as usize..];
        let src: Vec<u32> = px
            .chunks_exact(3)
            .map(|c| 0xff00_0000 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
            .collect();
        let (w, h) = (2220u32, 1080u32);
        let write = |name: &str, hud: &Hud| {
            let mut dst = vec![0u32; (w * h) as usize];
            let game = hud.layout().screen.game;
            blit_game(&mut dst, w, h, game, &src, gw, gh, &column_map(game.w as u32, gw));
            draw_hud(&mut dst, w, h, hud);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in dst {
                out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
            std::fs::write(dir.join(format!("hud_{name}.ppm")), out).unwrap();
        };
        // The host's lines in the game's font (B1), if the probe lettered them (`--host-texts` in
        // `host_text::LINES` order → `texts.txt`).
        let texts: Vec<(u16, Option<hz_java::Icon>)> = std::fs::read_to_string(dir.join("texts.txt"))
            .map(|t| parse_labels(&t).into_iter().enumerate().map(|(i, l)| (i as u16, l.map(|l| hz_java::Icon { key: hz_java::IconKey::Text(i as u16), ..l }))).collect())
            .unwrap_or_default();
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_texts(texts.clone());
        write("idle", &hud);
        let st = hud.layout().stick.unwrap();
        let (sx, sy) = st.seat.centre();
        hud.started(1, f64::from(sx), f64::from(sy));
        hud.moved(1, f64::from(sx) + st.engage_r * 2.0, f64::from(sy) - 10.0);
        let ok = hud.layout().controls[0].shape.bounds().centre();
        hud.started(2, f64::from(ok.0), f64::from(ok.1));
        write("playing", &hud);
        hud.release_all();
        // A window's set (hero2's menu mode): the arrows, OK, CLR and the gear; one arrow held.
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Menu, ..Default::default() });
        let sq = hud.layout().dpad.unwrap();
        hud.started(3, f64::from(sq.centre().0), f64::from(sq.y + 20));
        write("menu", &hud);
        hud.release_all();
        let g = hud.layout().gear.unwrap().centre();
        hud.started(4, f64::from(g.0), f64::from(g.1));
        hud.ended(4);
        write("settings", &hud);
        // A finger resting on 발열 절감: the game's cursor row (B2).
        let held = hud.settings_rows()[2].0.centre();
        hud.started(40, f64::from(held.0), f64::from(held.1));
        write("settings_pressed", &hud);
        hud.cancelled(40);
        let row = hud.settings_rows()[0].0.centre();
        hud.started(5, f64::from(row.0), f64::from(row.1));
        hud.ended(5);
        hud.started(6, f64::from(ok.0), f64::from(ok.1));
        hud.ended(6);
        write("editor", &hud);
        // A9: PAD on, plate 7 hidden (HIDE lit while it was selected, SHOW lit after), 9 selected.
        let button = |hud: &Hud, a: hud::EditAction| hud::editor_chrome(hud.screen()).into_iter().find(|(_, x)| *x == a).unwrap().0.centre();
        let tap = |hud: &mut Hud, id: u64, (x, y): (i32, i32)| {
            hud.started(id, f64::from(x), f64::from(y));
            hud.ended(id);
        };
        let at = button(&hud, hud::EditAction::Pad);
        tap(&mut hud, 41, at);
        let at = crate::placement::builtin(CtrlId::Ring(1), hud.screen()).centre();
        tap(&mut hud, 42, at);
        let at = button(&hud, hud::EditAction::Hide);
        tap(&mut hud, 43, at);
        let at = crate::placement::builtin(CtrlId::Ring(0), hud.screen()).centre();
        tap(&mut hud, 44, at);
        write("editor_pad", &hud);
        let at = button(&hud, hud::EditAction::Cancel);
        tap(&mut hud, 45, at);
        // The gamepad badges (PAD BADGES ON) and the first-pad notice.
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_texts(texts.clone());
        hud.set_pad_hud(PadHud::On);
        hud.pad_input();
        write("pad", &hud);
        // B5: three notices stacked over the stick's seat, under the status block; the game stopped.
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_texts(texts.clone());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        hud.set_stats(hz_java::Stats { hp: 100, hp_max: 150, sp: 30, sp_max: 40, level: 7, ..Default::default() });
        let t0 = std::time::Instant::now() - std::time::Duration::from_millis(1000);
        for (i, line) in ["LAYOUT SAVED", "SAVE - NOT NOW", "SAVE READ ERROR - THE FILE IS KEPT"].into_iter().enumerate() {
            hud.notice_at(line, t0 + std::time::Duration::from_millis(300 * i as u64));
        }
        write("notices", &hud);
        // HZ-025: the game's own pickup pictures (the probe's `pickups.txt`) under a line, newest lowest.
        if let Ok(t) = std::fs::read_to_string(dir.join("pickups.txt")) {
            let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
            hud.set_texts(texts.clone());
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
            hud.set_stats(hz_java::Stats { hp: 100, hp_max: 150, sp: 30, sp_max: 40, level: 7, ..Default::default() });
            hud.notice_at("LAYOUT SAVED", t0);
            for (i, p) in parse_labels(&t).into_iter().flatten().enumerate() {
                hud.pickup_at(hz_java::Icon { key: hz_java::IconKey::Pickup(i as u32), ..p }, t0 + std::time::Duration::from_millis(300 * (i as u64 + 1)));
            }
            write("pickups", &hud);
        }
        // HZ-061: a boss's story counting down — its held line, the pickups since stacked under it.
        {
            let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
            hud.set_texts(texts.clone());
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
            hud.set_stats(hz_java::Stats { hp: 1103, hp_max: 1130, sp: 1585, sp_max: 1585, level: 99, ..Default::default() });
            hud.countdown_at(crate::host_text::story_wait(3), t0);
            if let Ok(t) = std::fs::read_to_string(dir.join("pickups.txt")) {
                for (i, p) in parse_labels(&t).into_iter().flatten().enumerate().skip(1) {
                    hud.pickup_at(hz_java::Icon { key: hz_java::IconKey::Pickup(i as u32), ..p }, t0 + std::time::Duration::from_millis(300 * i as u64));
                }
            }
            write("story_wait", &hud);
        }
        hud.notice_stuck("THE GAME STOPPED - SAVES ARE SAFE - RESTART THE APP");
        write("fatal", &hud);
        // The status block (Phase 6-1): the field, numbers from a mid-game character; then with
        // the game's bar off (HZ-013 — give a frame dumped with it), the EXP bar too.
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
        hud.set_stats(hz_java::Stats {
            hp: 128,
            hp_max: 150,
            sp: 30,
            sp_max: 40,
            exp: 4233,
            exp_next: 10000,
            mp: 5,
            mp_max: 20,
            level: 12,
            stat_points: 3,
            skill_points: 1,
            guardian: 0,
            // HZ-030: 가디언 모드's line under the block.
            round: 3,
        });
        write("stats", &hud);
        // A notice (B1: its Korean in the game's font once lettered).
        hud.set_texts(texts.clone());
        hud.notice("SAVE - NOT NOW");
        write("toast", &hud);
        hud.clear_toast();
        hud.set_bar_off(true);
        write("status_bar_off", &hud);
        // The 창고 question (choice ②, give a frame dumped on it): its answer line under the windows.
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Menu, lsk_ok: true, ..Default::default() });
        write("answer", &hud);
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        // The quick plates (Phase 6-2): a skill on 1, a cooling-down potion stack on 0.
        hud.set_quick(sample_bar());
        write("quick", &hud);
        // The buff band (hero2's cells, right of the bars): a form, a spirit skill, the guard.
        let mut bar = sample_bar();
        bar.buffs = [(hz_java::BuffKind::Form, hz_java::IconKey::CharSkill { class: 5, cell: 1 }, 0xffe0_6020, 23), (hz_java::BuffKind::Spirit, hz_java::IconKey::SpiritSkill(2), 0xff30_c0a0, 41), (hz_java::BuffKind::Guard, hz_java::IconKey::Orb, 0xff40_80ff, 5)]
            .into_iter()
            .map(|(kind, key, argb, secs)| hz_java::Buff { kind, icon: Some(solid_icon(key, argb)), secs })
            .collect();
        hud.set_quick(bar);
        write("buffs", &hud);
        // A7 (hero2's menu mode): all of it over a game window — with the window's arrows and gear.
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Menu, ..Default::default() });
        write("play_menu", &hud);
        // 조작 방식 십자키 (A8): the field walks with a window's arrows.
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        hud.set_control(hz_java::ControlMode::Dpad);
        write("dpad_field", &hud);
        // The PAD REMAP panel (Phase 5-2): 7 picked, after the bottom face button was bound to it.
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_texts(texts.clone());
        hud.pad_input();
        hud.clear_toast();
        let g = hud.layout().gear.unwrap().centre();
        hud.started(8, f64::from(g.0), f64::from(g.1));
        hud.ended(8);
        // With a pad the list holds seven rows (B2): scrolled to its end, 패드 버튼 설정 last.
        let v = hud::settings_geom(hud.screen()).view;
        let (vx, vy) = (f64::from(v.centre().0), f64::from(v.centre().1));
        hud.started(30, vx, vy);
        hud.moved(30, vx, vy - f64::from(v.h));
        hud.ended(30);
        write("settings_scrolled", &hud);
        let row = hud.settings_rows().iter().find(|r| r.1 == hud::SettingsAction::PadRemap).unwrap().0.centre();
        hud.started(9, f64::from(row.0), f64::from(row.1));
        hud.ended(9);
        // B6: the field HUD lit, the window with back and `*` as cells; plate 7 picked, Y pressed
        // (7 = Y, 3 → L1), then # picked and waiting.
        hud.set_quick(sample_bar());
        let sc = hud.remap_scene();
        let at = |key: KeyCode| sc.targets.iter().find(|t| t.row == crate::gamepad::remap_row(crate::gamepad::PadAction::Key(key)).unwrap()).unwrap().rect.centre();
        let seven = at(KeyCode::NUM7);
        hud.started(10, f64::from(seven.0), f64::from(seven.1));
        hud.ended(10);
        hud.pad_button(crate::gamepad::KEYCODE_BUTTON_Y);
        write("remap", &hud);
        let hash = at(KeyCode::HASH);
        hud.started(11, f64::from(hash.0), f64::from(hash.1));
        hud.ended(11);
        write("remap_wait", &hud);
        // The minimap (Phase 6-5) over a field frame, from the probe's `minimap.txt` (the map and
        // marks the hook read on that frame), at the frame's own aspect (an S25's 520x240 is
        // 2340x1080) with the game's bar off — the field as the APK shows it.
        if let Ok(text) = std::fs::read_to_string(dir.join("minimap.txt"))
            && let Some((map, marks)) = parse_minimap(&text)
        {
            let (w, h) = (gw * 1080 / gh, 1080);
            let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
            hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, popup: false, notice: false, ..Default::default() });
            hud.set_bar_off(true);
            hud.set_stats(hz_java::Stats { hp: 128, hp_max: 150, sp: 30, sp_max: 40, exp: 4233, exp_next: 10000, mp: 5, mp_max: 20, level: 12, ..Default::default() });
            hud.set_quick(sample_bar());
            hud.set_map(map);
            hud.set_marks(marks);
            // The portrait the hook composed on that run (6-6), if the probe wrote one.
            hud.set_portrait(std::fs::read_to_string(dir.join("portrait.txt")).ok().and_then(|t| parse_portrait(&t)));
            eprintln!("preview: target panel shift, drop for this window = {:?} guest px", hud.target_place());
            let mut dst = vec![0u32; (w * h) as usize];
            let game = hud.layout().screen.game;
            blit_game(&mut dst, w, h, game, &src, gw, gh, &column_map(game.w as u32, gw));
            draw_hud(&mut dst, w, h, &hud);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in dst {
                out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
            std::fs::write(dir.join("hud_minimap.ppm"), out).unwrap();
            // Tapped away (A8): hero2's folded-map button in the panel's corner.
            hud.set_minimap_on(false);
            let mut dst = vec![0u32; (w * h) as usize];
            blit_game(&mut dst, w, h, game, &src, gw, gh, &column_map(game.w as u32, gw));
            draw_hud(&mut dst, w, h, &hud);
            let mut out = format!("P6
{w} {h}
255
").into_bytes();
            for p in dst {
                out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
            std::fs::write(dir.join("hud_minimap_off.ppm"), out).unwrap();
            hud.set_minimap_on(true);
            // The world map (Phase 7-2) opened from the MAP tab over that field, the pin settled,
            // with the names the probe's `--labels` lettered (`labels.txt`, 7-3) if it is there.
            if let Ok(text) = std::fs::read_to_string(dir.join("labels.txt")) {
                hud.set_labels(parse_labels(&text));
            }
            // The painted picture (7-4), if `hz_worldmap_draw.py --rgb565` wrote one there.
            if let Some(p) = std::fs::read(dir.join("hz_worldmap.rgb565")).ok().and_then(|raw| crate::worldmap_view::wm_parse_picture(&raw)) {
                crate::worldmap_view::wm_set_picture(p);
            }
            // The quest guidance the probe computed on the new-game save (7-6): quest 1, from the
            // orphanage room's door (136: 4, 10) to the room next door (map 10).
            hud.set_guide(hz_java::QuestGuide { quest: Some(1), target: Some(10), tiles: vec![(136, 4, 10)] });
            let (mx, my) = hud.tab(hud::Tab::Map).expect("MAP in the field").centre();
            hud.started(20, f64::from(mx), f64::from(my));
            hud.ended(20);
            for _ in 0..16 {
                hud.world_tick();
            }
            let mut dst = vec![0u32; (w * h) as usize];
            blit_game(&mut dst, w, h, game, &src, gw, gh, &column_map(game.w as u32, gw));
            draw_hud(&mut dst, w, h, &hud);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in dst {
                out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
            std::fs::write(dir.join("hud_world.ppm"), out).unwrap();
        }
    }

    /// The portrait HUD (2026-10-29) over a 240 × 320 probe frame in `HZ_PREVIEW_DIR` (`frame.ppm`,
    /// with the probe's `minimap.txt` if it is there), at the S9's 1080 × 2220: the field — the strip's
    /// stick, plates, OK, CLR, MAP, SAVE and gear, the minimap and the status block over the picture
    /// — then a window's arrows, 십자키, the editor and the pad screen, as `<dir>/hud_p_<state>.ppm`.
    #[test]
    #[ignore]
    fn preview_portrait() {
        let Ok(dir) = std::env::var("HZ_PREVIEW_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        let raw = std::fs::read(dir.join("frame.ppm")).expect("frame.ppm");
        let mut parts = raw.splitn(4, |b| b.is_ascii_whitespace());
        let (_, gw, gh) = (parts.next(), parts.next().unwrap(), parts.next().unwrap());
        let (gw, gh): (u32, u32) = (std::str::from_utf8(gw).unwrap().parse().unwrap(), std::str::from_utf8(gh).unwrap().parse().unwrap());
        let px = &raw[raw.len() - (gw * gh * 3) as usize..];
        let src: Vec<u32> = px.chunks_exact(3).map(|c| 0xff00_0000 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])).collect();
        let (w, h) = (1080u32, 2220u32);
        let write = |name: &str, hud: &Hud| {
            let mut dst = vec![0u32; (w * h) as usize];
            let game = hud.layout().screen.game;
            blit_game(&mut dst, w, h, game, &src, gw, gh, &column_map(game.w as u32, gw));
            draw_hud(&mut dst, w, h, hud);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in dst {
                out.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
            std::fs::write(dir.join(format!("hud_p_{name}.ppm")), out).unwrap();
        };
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        hud.set_bar_off(true);
        hud.set_stats(hz_java::Stats { hp: 128, hp_max: 150, sp: 30, sp_max: 40, exp: 4233, exp_next: 10000, mp: 5, mp_max: 20, level: 12, ..Default::default() });
        hud.set_quick(sample_bar());
        if let Ok(text) = std::fs::read_to_string(dir.join("minimap.txt"))
            && let Some((map, marks)) = parse_minimap(&text)
        {
            hud.set_map(map);
            hud.set_marks(marks);
        }
        eprintln!("preview: portrait target panel (shift, drop) = {:?} guest px", hud.target_place());
        let t0 = std::time::Instant::now() - std::time::Duration::from_millis(800);
        hud.notice_at("SAVED", t0);
        write("field", &hud);
        let st = hud.layout().stick.unwrap();
        let (sx, sy) = st.seat.centre();
        hud.started(1, f64::from(sx), f64::from(sy));
        hud.moved(1, f64::from(sx) + st.engage_r * 2.0, f64::from(sy) - 10.0);
        write("walking", &hud);
        hud.release_all();
        hud.set_control(hz_java::ControlMode::Dpad);
        write("dpad", &hud);
        hud.set_control(hz_java::ControlMode::Free);
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Menu, ..Default::default() });
        write("menu", &hud);
        // A scenario's dialogue keeps the strip in portrait (user 2026-10-31).
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Script, ..Default::default() });
        write("dialogue", &hud);
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        let g = hud.layout().gear.unwrap().centre();
        hud.started(2, f64::from(g.0), f64::from(g.1));
        hud.ended(2);
        write("settings", &hud);
        let row = hud.settings_rows()[0].0.centre();
        hud.started(3, f64::from(row.0), f64::from(row.1));
        hud.ended(3);
        let ok = crate::placement::builtin(CtrlId::Ok, hud.screen()).centre();
        hud.started(4, f64::from(ok.0), f64::from(ok.1));
        hud.ended(4);
        write("editor", &hud);
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        hud.set_quick(sample_bar());
        hud.set_pad_hud(PadHud::On);
        hud.pad_input();
        hud.clear_toast();
        write("pad", &hud);
        let g = hud.layout().gear.unwrap().centre();
        hud.started(5, f64::from(g.0), f64::from(g.1));
        hud.ended(5);
        let v = hud::settings_geom(hud.screen()).view;
        let (vx, vy) = (f64::from(v.centre().0), f64::from(v.centre().1));
        hud.started(7, vx, vy);
        hud.moved(7, vx, vy - f64::from(v.h));
        hud.ended(7);
        let row = hud.settings_rows().iter().find(|r| r.1 == hud::SettingsAction::PadRemap).unwrap().0.centre();
        hud.started(6, f64::from(row.0), f64::from(row.1));
        hud.ended(6);
        write("remap", &hud);
    }

    /// The probe's `labels.txt`: `label N W H`, then H rows of W ARGB words in hex.
    fn parse_labels(text: &str) -> Vec<Option<hz_java::Icon>> {
        let mut out: Vec<Option<hz_java::Icon>> = Vec::new();
        let mut lines = text.lines();
        while let Some(head) = lines.next() {
            let f: Vec<u32> = head.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
            let [n, w, h] = f[..] else { break };
            let px: Vec<u32> = (0..h).filter_map(|_| lines.next()).flat_map(|l| l.split_whitespace().filter_map(|v| u32::from_str_radix(v, 16).ok()).collect::<Vec<_>>()).collect();
            while out.len() <= n as usize {
                out.push(None);
            }
            out[n as usize] = Some(hz_java::Icon { key: hz_java::IconKey::Label(n as u8), w, h, px: px.into() });
        }
        out
    }

    /// The probe's `portrait.txt`: rows of ARGB words in hex.
    fn parse_portrait(text: &str) -> Option<hz_java::Icon> {
        let rows: Vec<Vec<u32>> = text.lines().map(|l| l.split_whitespace().filter_map(|v| u32::from_str_radix(v, 16).ok()).collect()).collect();
        let (w, h) = (rows.first()?.len() as u32, rows.len() as u32);
        let px: Vec<u32> = rows.into_iter().flatten().collect();
        (px.len() == (w * h) as usize).then(|| hz_java::Icon { key: hz_java::IconKey::Portrait(0), w, h, px: px.into() })
    }

    /// The probe's `minimap.txt`: `map ID W H`, H rows of W class digits (the game window's),
    /// `doors` + H rows (the field panel's, when they differ), `player X Y`, `mob X Y`…, and
    /// `title W H` + H rows of W ARGB words in hex (the game window's title for the map).
    fn parse_minimap(text: &str) -> Option<(hz_java::MapGrid, hz_java::MapMarks)> {
        let mut lines = text.lines();
        let head: Vec<u32> = lines.next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let [id, mw, mh] = head[..] else { return None };
        let mut cells = Vec::new();
        for _ in 0..mh {
            cells.extend(lines.next()?.bytes().map(|b| b - b'0'));
        }
        let mut marks = hz_java::MapMarks::default();
        let mut title = None;
        let mut doors = None;
        while let Some(l) = lines.next() {
            let v: Vec<&str> = l.split_whitespace().collect();
            let at = || Some((v.get(1)?.parse().ok()?, v.get(2)?.parse().ok()?));
            match v.first() {
                Some(&"player") => marks.player = at(),
                Some(&"mob") => marks.mobs.extend(at()),
                Some(&"doors") => {
                    let d: Vec<u8> = (0..mh).filter_map(|_| lines.next()).flat_map(|l| l.bytes().map(|b| b - b'0')).collect();
                    doors = (d.len() == cells.len()).then(|| std::sync::Arc::from(d));
                }
                Some(&"title") => {
                    let size = |i: usize| v.get(i).and_then(|n| n.parse::<u32>().ok());
                    let (Some(w), Some(h)) = (size(1), size(2)) else { continue };
                    let px: Vec<u32> = (0..h).filter_map(|_| lines.next()).flat_map(|l| l.split_whitespace().filter_map(|v| u32::from_str_radix(v, 16).ok()).collect::<Vec<_>>()).collect();
                    if px.len() == (w * h) as usize {
                        title = Some(hz_java::Icon { key: hz_java::IconKey::MapTitle(id as u8), w, h, px: px.into() });
                    }
                }
                _ => {}
            }
        }
        let map = hz_java::MapGrid { id: id as u8, w: mw as u8, h: mh as u8, cells: cells.into(), leaving: None, door_cells: doors, title, fights: None };
        (map.cells.len() == (mw * mh) as usize).then_some((map, marks))
    }

    #[test]
    fn every_state_draws_inside_the_window() {
        for (w, h) in [(2220u32, 1080u32), (1080, 2220)] {
            let (gw, gh) = if w > h { crate::guest_size(w, h, true) } else { (240, 320) };
            let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
            let mut dst = vec![0u32; (w * h) as usize];
            draw_hud(&mut dst, w, h, &hud);
            let g = hud.layout().gear.unwrap().centre();
            hud.started(1, f64::from(g.0), f64::from(g.1));
            hud.ended(1);
            draw_hud(&mut dst, w, h, &hud);
            let row = hud.settings_rows()[0].0.centre();
            hud.started(2, f64::from(row.0), f64::from(row.1));
            draw_hud(&mut dst, w, h, &hud);
            hud.ended(2);
            draw_hud(&mut dst, w, h, &hud);
        }
    }

    /// Opt-in cost check (the S25's landscape window, 2340×1080): what one present costs the UI
    /// thread in the field — the blit, then the HUD — with a finger on the stick and on OK.
    /// `cargo test -p hz_android --release frame_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn frame_cost() {
        let (w, h) = (2340u32, 1080u32);
        let (gw, gh) = crate::guest_size(w, h, true);
        let src: Vec<u32> = (0..gw * gh).map(|i| 0xff00_0000 | (i.wrapping_mul(2_654_435_761) & 0x00ff_ffff)).collect();
        let mut hud = Hud::new(Screen::new(w, h, gw, gh), Table::default());
        hud.set_view(hz_java::GuestView { scene: hz_java::Scene::Field, ..Default::default() });
        hud.set_bar_off(true);
        hud.set_stats(hz_java::Stats { hp: 128, hp_max: 150, sp: 30, sp_max: 40, exp: 4233, exp_next: 10000, mp: 5, mp_max: 20, level: 12, ..Default::default() });
        hud.set_quick(sample_bar());
        let (mw, mh) = (60usize, 40usize);
        let mut attrs = vec![0i8; mw * mh];
        for x in 0..mw {
            attrs[x] = 64;
            attrs[(mh - 1) * mw + x] = 64;
        }
        hud.set_map(hz_java::MapGrid::new(7, 60, 40, &attrs, &[]).unwrap());
        hud.set_marks(hz_java::MapMarks { player: Some((30.5, 20.5)), mobs: (0..12).map(|i| (10.0 + i as f32 * 3.0, 15.5)).collect() });
        let st = hud.layout().stick.unwrap();
        let (sx, sy) = st.seat.centre();
        hud.started(1, f64::from(sx), f64::from(sy));
        hud.moved(1, f64::from(sx) + st.engage_r * 2.0, f64::from(sy) - 10.0);
        let game = hud.layout().screen.game;
        let cols = column_map(game.w as u32, gw);
        let mut dst = vec![0u32; (w * h) as usize];
        let n = 60;
        let (mut blit, mut draw) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for _ in 0..n {
            let t = std::time::Instant::now();
            blit_game(&mut dst, w, h, game, &src, gw, gh, &cols);
            let t1 = std::time::Instant::now();
            draw_hud(&mut dst, w, h, &hud);
            draw += t1.elapsed();
            blit += t1 - t;
        }
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / f64::from(n);
        eprintln!("frame_cost {w}x{h} guest {gw}x{gh}: blit {:.2} ms, hud {:.2} ms", ms(blit), ms(draw));
        // The parts: each drawn alone over the same picture, n times.
        let mut time = |what: &str, f: &mut dyn FnMut(&mut Canvas)| {
            let t = std::time::Instant::now();
            for _ in 0..n {
                let mut c = Canvas { px: &mut dst, w: w as i32, h: h as i32 };
                f(&mut c);
            }
            eprintln!("  {what}: {:.3} ms", ms(t.elapsed()));
        };
        let (st, l) = (hud.stats_shown().unwrap(), hud.layout().clone());
        time("minimap", &mut |c| {
            if let Some((map, marks, r)) = hud.minimap_shown() {
                draw_minimap(c, map, marks, r, None);
            }
        });
        time("exp bar", &mut |c| draw_exp(c, &hud, &st));
        time("status", &mut |c| draw_status(c, &hud, &st));
        time("stick", &mut |c| {
            let s = l.stick.unwrap();
            c.disc(1000.0, 600.0, s.base_r, STICK_BASE, 55, BORDER, 90, 2.8);
            c.disc(1000.0, 600.0, s.base_r * hud::THUMB, STICK_THUMB, 105, STICK_THUMB, 0, 0.0);
        });
        let bar = hud.quick_shown().unwrap().clone();
        for ctl in &l.controls {
            let name = format!("{:?}", ctl.id);
            time(&name, &mut |c| draw_hud_control(c, ctl, false, Some(&bar), false));
        }
        time("tabs", &mut |c| {
            for (r, tab) in hud.tabs_shown() {
                draw_top_tab(c, *r, tab.label(), false);
            }
        });
    }
}
