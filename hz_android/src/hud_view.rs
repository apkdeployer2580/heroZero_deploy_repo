//! The STATUS block (Phase 6-1, 2026-10-23): the hero5-style block at the top-left and the EXP bar
//! at the bottom centre — hero2's `wie_android/src/hud_view.rs` (2026-09-29; hero3 141차 carries a
//! byte-identical copy), adapted for this game.
//!
//! User (2026-10-23): *"게임하단바 같은거 다 없애고 hero2,3 처럼 HUD전부 바꿀꺼니까"* — the game's
//! bottom bar goes (HZ-013) and the host draws what hero2 and hero3 draw. This is the first piece.
//!
//! What hero2's block is: pixel art drawn in HUD PIXELS — one hud pixel `u` = the game's own pixel,
//! rounded ([`hv_unit`]), so the chrome has the grain of the world under it. Every colour was
//! measured off 영웅서기5's frame (the design is borrowed, no art is):
//!
//!   * RING — a gold-rimmed disc with the character's head inside (6-6: the hero's own paper doll,
//!     composed by `hz_java::portrait`; the dark ground until the first one).
//!   * BARS — one silver-framed block that starts UNDER the ring and ends in 45° slants, each bar
//!     with its value in a dark box at its middle.
//!   * LEVEL PLATE — the level in white.
//!   * EXP — a thin bar at the bottom centre: cream for what is earned, orange for what is not.
//!
//! What is different here:
//!
//!   * THREE bars, not two: this game has HP, SP and MP (`ek.e/f(B)I` 0, 1, 3 — `hz_java::Stats`).
//!     SP takes HP's shape (its own separator, no bottom bevel) and MP is the block's last bar. Their
//!     colours are the game's own: SP blue, MP green (`ce.<init>`'s `d[]` and `c[]`), in hero5's
//!     shading.
//!   * The level plate sits UNDER the bars (rows 28..36): hero2's spot beside the ring's foot is
//!     MP's row here.
//!   * The point badges are this game's own icons (6-3); the guardian level is HZ-022's (hero2's
//!     plate over the EXP bar, [`hv_draw_gl`]).
//!   * Colours are softbuffer's `0x00RRGGBB`; an icon's ARGB loses its alpha byte when drawn.
//!   * The buff cells stop short of THIS game's target panel: 80 px at the frame's right edge
//!     (`ce.a(cq,Z)`'s 80×34 image), which stays the game's own (user, 2026-10-23).
//!
//! Dependency-free and pure (`&mut [u32]` in, nothing kept), so the host tests draw it.

/// Border between the game rect's top-left corner and the block, in hud px (hero5: 3).
pub const HV_MARGIN: u32 = 3;
/// The ring's diameter, in hud px.
pub const HV_RING: u32 = 24;
/// Where the bar block starts — under the ring's centre, so the ring hides its left end.
pub const HV_BAR_X0: u32 = 12;
/// The column HP's and SP's fill starts from at 0 %: under the ring, so an empty bar shows no
/// sliver of colour beside it.
pub const HV_FILL_X0: u32 = 18;
/// The bars' right end on their top row, in the landscape layout (hero5: ~89).
pub const HV_BAR_R_MAX: u32 = 94;
/// …and never shorter than this, whatever the window.
pub const HV_BAR_R_MIN: u32 = 56;
/// The rows of the three bars: HP 1..=10, SP 10..=18, MP 18..=27 (the separators are shared).
pub const HV_BARS_BOTTOM: u32 = 27;
/// The level plate's left edge and its right end on its top row.
pub const HV_PLATE_X0: u32 = 15;
pub const HV_PLATE_R: u32 = 42;
/// The plate's first row (it runs to `HV_PLATE_Y0 + 8`): right under MP.
pub const HV_PLATE_Y0: u32 = HV_BARS_BOTTOM + 1;
/// Rows the block covers.
pub const HV_BLOCK_H: u32 = HV_PLATE_Y0 + 9;
/// The game's unspent-point icons are 8×8 (`rpg/e.c`, Phase 6-3), one hud px each, right of the
/// level plate (hero2's badge spot).
pub const HV_BADGE: u32 = 8;
/// The EXP bar: 4 rows (frame · light · dark · frame), `HV_MARGIN` above the bottom edge.
pub const HV_EXP_H: u32 = 4;
/// A BUFF CELL (hero2 2026-10-05): black outline · silver frame · the 12×12 icon · under it the
/// seconds left — 16 × 23 hud px, cells 1 px apart, the first `HV_BUFF_X_GAP` right of the bars'
/// tip, top on the bars' top row ([`HvGeom::buff_cell`]).
pub const HV_BUFF_W: u32 = 16;
pub const HV_BUFF_H: u32 = 23;
pub const HV_BUFF_ICON: u32 = 12;
pub const HV_BUFF_GAP: u32 = 1;
pub const HV_BUFF_X_GAP: u32 = 3;
/// This game's target panel (80 px at the frame's right edge) and 2 px of air: a row of cells
/// stops short of it.
pub const HV_BUFF_RIGHT_RESERVE: u32 = 82;
/// The game's timed states (`hz_java::state::BUFFS`): the most cells there can be.
pub const HV_BUFFS_MAX: u32 = 3;

const HV_OUTLINE: u32 = 0x0000_0000;
const HV_SILVER: u32 = 0x00de_dbce;
const HV_SILVER_LO: u32 = 0x009c_9684;
const HV_GAP: u32 = 0x0031_3431;
/// HP rows top to bottom: highlight · body ×4 · shadow (hero5's red, measured).
const HV_HP: [u32; 6] = [0x00ef_2429, 0x009c_1018, 0x009c_1018, 0x009c_1018, 0x009c_1018, 0x0063_0008];
/// …and the same rows where the bar is empty.
const HV_HP_TRACK: [u32; 6] = [0x004a_1418, 0x002c_080c, 0x002c_080c, 0x002c_080c, 0x002c_080c, 0x001c_0406];
/// SP rows: highlight · body ×2 · shadow ×2 (hero5's blue).
const HV_SP: [u32; 5] = [0x0008_b2f7, 0x0000_719c, 0x0000_719c, 0x0000_5984, 0x0000_2431];
const HV_SP_TRACK: [u32; 5] = [0x000c_3a50, 0x0006_222e, 0x0006_222e, 0x0004_1a24, 0x0002_0c10];
/// MP rows: the game's green (`ce.c:[I` 0x00cb00 / 0x99f700) in SP's shading.
const HV_MP: [u32; 5] = [0x0052_d63a, 0x0021_8c1a, 0x0021_8c1a, 0x0018_6b13, 0x0008_2a06];
const HV_MP_TRACK: [u32; 5] = [0x0016_3a10, 0x000c_220a, 0x000c_220a, 0x000a_1a08, 0x0004_0c03];
const HV_HP_INK: u32 = 0x00de_aeb5;
const HV_HP_INK_HI: u32 = 0x00f7_b6b5;
const HV_HP_BOX: u32 = 0x0031_0808;
const HV_HP_BOX_LO: u32 = 0x0021_0408;
const HV_SP_INK: u32 = 0x00ad_e3f7;
const HV_SP_BOX: u32 = 0x0008_3c52;
const HV_MP_INK: u32 = 0x00c4_f0b0;
const HV_MP_BOX: u32 = 0x001a_3c10;
const HV_PLATE_FILL: u32 = 0x0010_1010;
const HV_LV_INK: u32 = 0x00f4_f4f4;
/// The ring's inside, behind the head.
const HV_RING_IN: u32 = 0x0014_1820;
/// The rim, lit from the upper left: outer row dark → light, inner row dark → light.
const HV_RIM_OUT: (u32, u32) = (0x007b_7556, 0x00fb_f7b4);
const HV_RIM_IN: (u32, u32) = (0x005a_5542, 0x00c6_be8c);
/// EXP earned: grey frame, cream over tan. EXP still to earn: black frame, orange over brown.
const HV_EXP_FRAME: u32 = 0x0063_6164;
const HV_EXP_HI: u32 = 0x00f6_f3a4;
const HV_EXP_LO: u32 = 0x00ce_9663;
const HV_EXP_TRACK_HI: u32 = 0x00b5_5500;
const HV_EXP_TRACK_LO: u32 = 0x0073_3400;

/// The digits, 3×5, one row per byte, bit 2 = the left column.
const HV_DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b110, 0b010, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Screen pixels per hud pixel for a game rect `game_h` px tall that shows `logical_h` guest
/// rows: the game's own pixel, to the nearest whole pixel, never below 2.
pub fn hv_unit(game_h: u32, logical_h: u32) -> u32 {
    if logical_h == 0 {
        return 2;
    }
    ((game_h + logical_h / 2) / logical_h).max(2)
}

/// Where the block sits and how big its parts are. Screen px unless a name says `_l` (hud px,
/// relative to the block's origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HvGeom {
    /// Screen px per hud px.
    pub u: u32,
    /// The block's origin (its top-left hud pixel), screen px.
    pub x: u32,
    pub y: u32,
    /// The bars' right end on their top row, hud px.
    pub bar_r_l: u32,
    /// The game frame's width in hud px (= guest px) — where the buff cells must stop.
    pub lw_l: u32,
}

impl HvGeom {
    /// The block for a game rect `(x, y, w, h)` showing `lw × lh` guest pixels.
    pub fn new(game: (u32, u32, u32, u32), lw: u32, lh: u32) -> HvGeom {
        let u = hv_unit(game.3, lh);
        // hero5's 89 of 400 on a wide screen; on a narrow game, 45 % of it.
        let bar_r_l = (lw * 45 / 100).clamp(HV_BAR_R_MIN, HV_BAR_R_MAX);
        HvGeom { u, x: game.0 + HV_MARGIN * u, y: game.1 + HV_MARGIN * u, bar_r_l, lw_l: lw }
    }

    fn at(&self, lx: u32, ly: u32, lw: u32, lh: u32) -> (u32, u32, u32, u32) {
        (self.x + lx * self.u, self.y + ly * self.u, lw * self.u, lh * self.u)
    }

    /// The ring's square.
    pub fn ring(&self) -> (u32, u32, u32, u32) {
        self.at(0, 0, HV_RING, HV_RING)
    }

    /// The bars, from their left end to the slants' tip, all three.
    pub fn bars(&self) -> (u32, u32, u32, u32) {
        self.at(HV_BAR_X0, 1, self.bar_r_l + 1 - HV_BAR_X0, HV_BARS_BOTTOM)
    }

    /// The level plate.
    pub fn plate(&self) -> (u32, u32, u32, u32) {
        self.at(HV_PLATE_X0, HV_PLATE_Y0, HV_PLATE_R + 1 - HV_PLATE_X0, 9)
    }

    /// Unspent stat (`i == 0`) / skill (`i == 1`) point badge, beside the level plate.
    pub fn badge(&self, i: u32) -> (u32, u32, u32, u32) {
        self.at(HV_PLATE_R + 2 + i * (HV_BADGE + 1), HV_PLATE_Y0, HV_BADGE, HV_BADGE)
    }

    /// Buff cell `i` (0 = nearest the bars), screen px. Not part of [`Self::bbox`].
    pub fn buff_cell(&self, i: u32) -> (u32, u32, u32, u32) {
        let (lx, ly) = self.buff_at_l(i);
        self.at(lx, ly, HV_BUFF_W, HV_BUFF_H)
    }

    /// Cells a row: as many as end short of [`HV_BUFF_RIGHT_RESERVE`], never fewer than one — and
    /// one on the portrait canvas (240 wide), as on hero2's 176: the top tabs share its narrow band.
    fn buff_cols(&self) -> u32 {
        if self.lw_l < 320 {
            return 1;
        }
        let x0 = self.bar_r_l + HV_BUFF_X_GAP;
        let room = self.lw_l.saturating_sub(HV_BUFF_RIGHT_RESERVE).saturating_sub(x0) + HV_BUFF_GAP;
        (room / (HV_BUFF_W + HV_BUFF_GAP)).max(1)
    }

    /// Cell `i`'s top-left, hud px: hero2's, right of the bars on their top row (2026-10-26 — the
    /// SAVE / MAP tabs' band starts past them now, `hud::status_box`; from 2026-10-25 until the
    /// field's top chrome went they sat under the block).
    fn buff_at_l(&self, i: u32) -> (u32, u32) {
        let cols = self.buff_cols();
        (self.bar_r_l + HV_BUFF_X_GAP + (i % cols) * (HV_BUFF_W + HV_BUFF_GAP), 1 + (i / cols) * (HV_BUFF_H + HV_BUFF_GAP))
    }

    /// The first row of cells as [`HV_BUFFS_MAX`] of them fill it, screen px — what the block
    /// claims beside it whether or not a state is running, so one starting never lands on a tab.
    pub fn buff_row(&self) -> (u32, u32, u32, u32) {
        let n = self.buff_cols().min(HV_BUFFS_MAX);
        self.at(self.bar_r_l + HV_BUFF_X_GAP, 1, n * (HV_BUFF_W + HV_BUFF_GAP) - HV_BUFF_GAP, HV_BUFF_H)
    }

    /// Everything the block can cover — the badges included even while none are shown.
    pub fn bbox(&self) -> (u32, u32, u32, u32) {
        self.at(0, 0, (self.bar_r_l + 1).max(HV_PLATE_R + 2 + 2 * HV_BADGE + 1), HV_BLOCK_H)
    }
}

/// The EXP bar's rectangle (screen px) and its width in hud px, for the same game rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HvExp {
    pub u: u32,
    pub x: u32,
    pub y: u32,
    pub w_l: u32,
}

impl HvExp {
    pub fn new(game: (u32, u32, u32, u32), lw: u32, lh: u32) -> HvExp {
        let u = hv_unit(game.3, lh);
        // ~36 % of the width, centred: clear of the stick on the left and of the OK disc on the
        // right at the default seats.
        let w_l = (lw * 36 / 100).clamp(40, 200).min(game.2 / u.max(1));
        let x = game.0 + game.2.saturating_sub(w_l * u) / 2;
        let y = (game.1 + game.3).saturating_sub((HV_MARGIN + HV_EXP_H) * u);
        HvExp { u, x, y, w_l }
    }

    pub fn rect(&self) -> (u32, u32, u32, u32) {
        (self.x, self.y, self.w_l * self.u, HV_EXP_H * self.u)
    }
}

/// A picture for the ring or a buff cell: `w × h`, row-major ARGB, alpha 0 = transparent.
#[derive(Clone, Copy)]
pub struct HvIcon<'a> {
    pub w: u32,
    pub h: u32,
    pub px: &'a [u32],
}

/// A buff cell's content: the icon (if there is one) and the whole seconds left.
#[derive(Clone, Copy)]
pub struct HvBuff<'a> {
    pub icon: Option<HvIcon<'a>>,
    pub secs: u32,
}

/// What the block shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HvStats {
    pub hp: u32,
    pub hp_max: u32,
    pub sp: u32,
    pub sp_max: u32,
    pub mp: u32,
    pub mp_max: u32,
    pub level: u32,
}

/// `color` over the screen rect, opaque, clipped to the window.
fn hv_rect(buf: &mut [u32], win_w: u32, win_h: u32, r: (u32, u32, u32, u32), color: u32) {
    let (x, y, w, h) = r;
    let x1 = x.saturating_add(w).min(win_w);
    let y1 = y.saturating_add(h).min(win_h);
    if x >= x1 {
        return;
    }
    for yy in y..y1 {
        let row = (yy as usize) * (win_w as usize);
        if let Some(s) = buf.get_mut(row + x as usize..row + x1 as usize) {
            s.fill(color & 0x00ff_ffff);
        }
    }
}

/// hud pixels `lx0..lx1` of row `ly` (block-relative) in `color`.
#[allow(clippy::too_many_arguments)]
fn hv_run(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, lx0: u32, lx1: u32, ly: u32, color: u32) {
    if lx1 > lx0 {
        hv_rect(buf, win_w, win_h, g.at(lx0, ly, lx1 - lx0, 1), color);
    }
}

/// The decimal digits of `v`, most significant first.
fn hv_digits(v: u32) -> Vec<u8> {
    v.to_string().bytes().map(|b| b - b'0').collect()
}

/// Width of `n` digits in hud px (3 each, 1 apart).
fn hv_text_w(n: u32) -> u32 {
    (n * 4).saturating_sub(1)
}

/// Digits at hud px `(lx, ly)`; row 0 in `ink_top`, the other four in `ink`.
#[allow(clippy::too_many_arguments)]
fn hv_draw_number(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, digits: &[u8], lx: u32, ly: u32, ink: u32, ink_top: u32) {
    for (i, d) in digits.iter().enumerate() {
        let glyph = HV_DIGITS[usize::from(*d).min(9)];
        for (r, bits) in glyph.iter().enumerate() {
            for c in 0..3u32 {
                if bits & (0b100 >> c) != 0 {
                    let x = lx + i as u32 * 4 + c;
                    hv_run(buf, win_w, win_h, g, x, x + 1, ly + r as u32, if r == 0 { ink_top } else { ink });
                }
            }
        }
    }
}

/// The right end (inclusive, the outline pixel) of bar row `ly`: 45° slants down-left — HP from
/// `bar_r`, SP and MP from one short of it — and the separators (rows 10 and 18) run out to the
/// tip of the next bar's top row, which leaves hero5's notch between the bars.
pub fn hv_bar_end(bar_r_l: u32, ly: u32) -> u32 {
    match ly {
        0..=9 => bar_r_l - ly.saturating_sub(1),
        10 | 18 => bar_r_l,
        11..=17 => bar_r_l - (ly - 10),
        _ => bar_r_l - (ly - 18).min(bar_r_l),
    }
}

/// The fill's edge in hud px for `cur / max` — `start` at 0 %, the bar's inner end at 100 %. A
/// column is filled when its left edge is short of this. HP and SP start under the ring
/// ([`HV_FILL_X0`]); MP, whose left end shows below it, inside its cap.
fn hv_fill_edge(bar_r_l: u32, start: u32, cur: u32, max: u32) -> f64 {
    let frac = if max == 0 { 0.0 } else { f64::from(cur.min(max)) / f64::from(max) };
    let span = f64::from(bar_r_l.saturating_sub(2 + start));
    f64::from(start) + frac * span
}

/// What row `ly` of the bar block is.
enum BarRow {
    /// Black to the end (the block's top and bottom, and the separators).
    Outline,
    /// The frame's light line (`lo` = its shadowed bottom line).
    Silver { lo: bool },
    /// The dark row under the frame line.
    Gap,
    /// Fill row `k` of bar `bar` (0 HP, 1 SP, 2 MP).
    Fill { bar: usize, k: usize },
}

fn bar_row(ly: u32) -> BarRow {
    match ly {
        1 | 10 | 18 | 27 => BarRow::Outline,
        2 | 11 | 19 => BarRow::Silver { lo: false },
        26 => BarRow::Silver { lo: true },
        3 | 12 | 20 => BarRow::Gap,
        4..=9 => BarRow::Fill { bar: 0, k: (ly - 4) as usize },
        13..=17 => BarRow::Fill { bar: 1, k: (ly - 13) as usize },
        _ => BarRow::Fill { bar: 2, k: (ly - 21) as usize },
    }
}

/// The whole block: bars first, then the level plate, then the ring over both.
pub fn hv_draw_status(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, s: &HvStats, head: Option<HvIcon>) {
    if g.u == 0 {
        return;
    }
    let r = g.bar_r_l;
    // ── bars ──
    let edges = [
        hv_fill_edge(r, HV_FILL_X0, s.hp, s.hp_max),
        hv_fill_edge(r, HV_FILL_X0, s.sp, s.sp_max),
        hv_fill_edge(r, HV_BAR_X0 + 2, s.mp, s.mp_max),
    ];
    for ly in 1..=HV_BARS_BOTTOM {
        let end = hv_bar_end(r, ly);
        // MP's rows show their left end below the ring: a cap of outline · silver.
        let cap = ly >= 19;
        let x0 = if cap { HV_BAR_X0 + 2 } else { HV_BAR_X0 };
        match bar_row(ly) {
            BarRow::Outline => {
                hv_run(buf, win_w, win_h, g, HV_BAR_X0, end + 1, ly, HV_OUTLINE);
                continue;
            }
            BarRow::Silver { lo } => {
                hv_run(buf, win_w, win_h, g, HV_BAR_X0, end, ly, if lo { HV_SILVER_LO } else { HV_SILVER });
            }
            BarRow::Gap => hv_run(buf, win_w, win_h, g, x0, end.saturating_sub(1), ly, HV_GAP),
            BarRow::Fill { bar, k } => {
                let (fill, track) = match bar {
                    0 => (HV_HP[k], HV_HP_TRACK[k]),
                    1 => (HV_SP[k], HV_SP_TRACK[k]),
                    _ => (HV_MP[k], HV_MP_TRACK[k]),
                };
                let inner_end = end.saturating_sub(1);
                // Filled columns are those whose left edge is short of the edge.
                let split = (edges[bar].ceil().max(f64::from(x0)) as u32).min(inner_end);
                hv_run(buf, win_w, win_h, g, x0, split, ly, fill);
                hv_run(buf, win_w, win_h, g, split, inner_end, ly, track);
            }
        }
        if cap && !matches!(bar_row(ly), BarRow::Silver { .. }) {
            hv_run(buf, win_w, win_h, g, HV_BAR_X0, HV_BAR_X0 + 1, ly, HV_OUTLINE);
            hv_run(buf, win_w, win_h, g, HV_BAR_X0 + 1, HV_BAR_X0 + 2, ly, if ly >= 25 { HV_SILVER_LO } else { HV_SILVER });
        } else if cap {
            hv_run(buf, win_w, win_h, g, HV_BAR_X0, HV_BAR_X0 + 1, ly, HV_OUTLINE);
        }
        if !matches!(bar_row(ly), BarRow::Silver { .. }) {
            // The slant: silver inside (shadowed at the block's foot, as hero2's last bar), then
            // the outline.
            hv_run(buf, win_w, win_h, g, end.saturating_sub(1), end, ly, if ly >= 25 { HV_SILVER_LO } else { HV_SILVER });
        }
        hv_run(buf, win_w, win_h, g, end, end + 1, ly, HV_OUTLINE);
    }
    // The values, centred on each bar's visible middle, in a dark box as tall as the bar.
    for (value, top, rows, ink, ink_top, bx, bx_lo) in [
        (s.hp, 4u32, 6u32, HV_HP_INK, HV_HP_INK_HI, HV_HP_BOX, HV_HP_BOX_LO),
        (s.sp, 13, 5, HV_SP_INK, HV_SP_INK, HV_SP_BOX, HV_SP_BOX),
        (s.mp, 21, 5, HV_MP_INK, HV_MP_INK, HV_MP_BOX, HV_MP_BOX),
    ] {
        let digits = hv_digits(value);
        let tw = hv_text_w(digits.len() as u32);
        let visible_end = hv_bar_end(r, top).saturating_sub(2);
        let mid = (HV_RING + 1 + visible_end) / 2;
        let x0 = mid.saturating_sub(tw / 2).max(HV_RING + 2);
        for ly in top..top + rows {
            let c = if ly + 1 == top + rows && rows == 6 { bx_lo } else { bx };
            hv_run(buf, win_w, win_h, g, x0 - 1, x0 + tw + 1, ly, c);
        }
        hv_draw_number(buf, win_w, win_h, g, &digits, x0, top, ink, ink_top);
    }

    // ── level plate ──
    let y0 = HV_PLATE_Y0;
    for ly in y0..=y0 + 8 {
        let end = HV_PLATE_R - (ly - y0);
        if ly == y0 || ly == y0 + 8 {
            hv_run(buf, win_w, win_h, g, HV_PLATE_X0, end + 1, ly, HV_OUTLINE);
            continue;
        }
        let line = if ly == y0 + 7 { HV_SILVER_LO } else { HV_SILVER };
        hv_run(buf, win_w, win_h, g, HV_PLATE_X0, HV_PLATE_X0 + 1, ly, HV_OUTLINE);
        if ly == y0 + 1 || ly == y0 + 7 {
            hv_run(buf, win_w, win_h, g, HV_PLATE_X0 + 1, end, ly, line);
        } else {
            hv_run(buf, win_w, win_h, g, HV_PLATE_X0 + 1, HV_PLATE_X0 + 2, ly, HV_SILVER);
            hv_run(buf, win_w, win_h, g, HV_PLATE_X0 + 2, end - 1, ly, HV_PLATE_FILL);
            hv_run(buf, win_w, win_h, g, end - 1, end, ly, line);
        }
        hv_run(buf, win_w, win_h, g, end, end + 1, ly, HV_OUTLINE);
    }
    let digits = hv_digits(s.level);
    let tw = hv_text_w(digits.len() as u32);
    let mid = (HV_PLATE_X0 + 2 + HV_PLATE_R - 6) / 2;
    hv_draw_number(buf, win_w, win_h, g, &digits, mid.saturating_sub(tw / 2), y0 + 2, HV_LV_INK, HV_LV_INK);

    // ── ring, over the bars' left ends ──
    hv_draw_ring(buf, win_w, win_h, g, head);
}

/// The unspent-point badges (Phase 6-3): `badges[0]` the stat icon, `[1]` the skill icon — each the
/// game's own picture, drawn one hud px per art px at [`HvGeom::badge`], transparent where it is.
/// `None` = not shown (no points, or the blink is off).
pub fn hv_draw_badges(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, badges: [Option<HvIcon>; 2]) {
    if g.u == 0 {
        return;
    }
    for (i, icon) in badges.iter().enumerate() {
        let Some(ic) = icon else { continue };
        let (x0, y0) = (HV_PLATE_R + 2 + i as u32 * (HV_BADGE + 1), HV_PLATE_Y0);
        for y in 0..ic.h.min(HV_BADGE) {
            for x in 0..ic.w.min(HV_BADGE) {
                if let Some(&p) = ic.px.get((y * ic.w + x) as usize)
                    && p >> 24 != 0
                {
                    hv_run(buf, win_w, win_h, g, x0 + x, x0 + x + 1, y0 + y, p);
                }
            }
        }
    }
}

/// The buff cells, left to right in list order:
/// ```text
/// row 0        outline
/// row 1        silver           col 0 outline · col 1 silver · cols 2..13 icon · col 14 silver lo · col 15 outline
/// rows 2..13   the icon (dark ground where it is transparent)
/// row 14       silver lo
/// rows 15..21  the plate's dark, the seconds in white on rows 16..20, centred
/// row 22       outline
/// ```
pub fn hv_draw_buffs(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, buffs: &[HvBuff]) {
    if g.u == 0 {
        return;
    }
    let (w, h, n) = (HV_BUFF_W, HV_BUFF_H, HV_BUFF_ICON);
    for (i, b) in buffs.iter().enumerate() {
        let (x0, y0) = g.buff_at_l(i as u32);
        hv_run(buf, win_w, win_h, g, x0, x0 + w, y0, HV_OUTLINE);
        hv_run(buf, win_w, win_h, g, x0, x0 + w, y0 + h - 1, HV_OUTLINE);
        for ly in y0 + 1..y0 + h - 1 {
            hv_run(buf, win_w, win_h, g, x0, x0 + 1, ly, HV_OUTLINE);
            hv_run(buf, win_w, win_h, g, x0 + w - 1, x0 + w, ly, HV_OUTLINE);
        }
        hv_run(buf, win_w, win_h, g, x0 + 1, x0 + w - 1, y0 + 1, HV_SILVER);
        hv_run(buf, win_w, win_h, g, x0 + 1, x0 + w - 1, y0 + 2 + n, HV_SILVER_LO);
        for ly in y0 + 2..y0 + 2 + n {
            hv_run(buf, win_w, win_h, g, x0 + 1, x0 + 2, ly, HV_SILVER);
            hv_run(buf, win_w, win_h, g, x0 + w - 2, x0 + w - 1, ly, HV_SILVER_LO);
        }
        // The icon, centred in its 12 × 12 if it is smaller.
        let (ix, iy) = b.icon.map_or((0, 0), |ic| ((n as i64 - ic.w as i64) / 2, (n as i64 - ic.h as i64) / 2));
        for ly in 0..n {
            for lx in 0..n {
                let p = b.icon.and_then(|ic| {
                    let (sx, sy) = (lx as i64 - ix, ly as i64 - iy);
                    if sx < 0 || sy < 0 || sx >= ic.w as i64 || sy >= ic.h as i64 {
                        return None;
                    }
                    ic.px.get((sy as u32 * ic.w + sx as u32) as usize).copied().filter(|p| p >> 24 != 0)
                });
                hv_run(buf, win_w, win_h, g, x0 + 2 + lx, x0 + 3 + lx, y0 + 2 + ly, p.unwrap_or(HV_RING_IN));
            }
        }
        for ly in y0 + 3 + n..y0 + h - 1 {
            hv_run(buf, win_w, win_h, g, x0 + 1, x0 + w - 1, ly, HV_PLATE_FILL);
        }
        let digits = hv_digits(b.secs.min(999));
        let tw = hv_text_w(digits.len() as u32);
        hv_draw_number(buf, win_w, win_h, g, &digits, x0 + (w - tw) / 2, y0 + 4 + n, HV_LV_INK, HV_LV_INK);
    }
}

/// A 0..=1 mix of two colours.
fn hv_mix(a: u32, b: u32, t: f64) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| {
        let (x, y) = (f64::from((a >> s) & 0xff), f64::from((b >> s) & 0xff));
        ((x + (y - x) * t).round() as u32) & 0xff
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// The ring and the head inside it. Radii in hud px from the centre `(12, 12)`: outline 11..12,
/// rim 9..11 (lit from the upper left), inner outline 8..9, the inside below 8.
fn hv_draw_ring(buf: &mut [u32], win_w: u32, win_h: u32, g: &HvGeom, head: Option<HvIcon>) {
    let c = f64::from(HV_RING) / 2.0;
    // Where the head's top-left lands: centred.
    let (hx0, hy0) = match head {
        Some(h) => ((HV_RING as i64 - h.w as i64) / 2, (HV_RING as i64 - h.h as i64) / 2),
        None => (0, 0),
    };
    for ly in 0..HV_RING {
        for lx in 0..HV_RING {
            let (dx, dy) = (f64::from(lx) + 0.5 - c, f64::from(ly) + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let color = if d >= 12.0 {
                continue;
            } else if d >= 11.0 || (8.0..9.0).contains(&d) {
                HV_OUTLINE
            } else if d >= 9.0 {
                let lit = 0.5 - 0.5 * (0.6 * dx + 0.8 * dy) / d;
                let (lo, hi) = if d >= 10.0 { HV_RIM_OUT } else { HV_RIM_IN };
                hv_mix(lo, hi, lit)
            } else {
                // Inside: the head's pixel if it has one here, the dark ground if not.
                let hp = head.and_then(|h| {
                    let (sx, sy) = (lx as i64 - hx0, ly as i64 - hy0);
                    if sx < 0 || sy < 0 || sx >= h.w as i64 || sy >= h.h as i64 {
                        return None;
                    }
                    h.px.get((sy as u32 * h.w + sx as u32) as usize).copied().filter(|p| p >> 24 != 0)
                });
                hp.unwrap_or(HV_RING_IN)
            };
            hv_run(buf, win_w, win_h, g, lx, lx + 1, ly, color);
        }
    }
}

/// The EXP bar: the columns up to `exp / next` of its width earned, the rest still to go.
/// The guardian level's plate (hero2 2026-10-22, HZ-022 here): its height in hud px (outline · 5
/// digit rows · outline), one hud px above the EXP bar.
pub const HV_GL_H: u32 = 7;
/// hero2's guardian violet, on its dark plate.
const HV_GL_INK: u32 = 0x00c0_80ff;
const HV_GL_INK_TOP: u32 = 0x00e8_d0ff;
const HV_GL_PLATE: u32 = 0x0018_0c24;

/// hero2's 가디언 레벨 plate (user, hero2: *"레벨 99를 찍은 뒤부터는 경험치바 바로 위쪽에 숫자가
/// 생기고"*): the GL on a small dark plate centred on the EXP bar, one hud px above it. Nothing for 0.
pub fn hv_draw_gl(buf: &mut [u32], win_w: u32, win_h: u32, e: &HvExp, gl: u32) {
    if e.u == 0 || e.w_l == 0 || gl == 0 {
        return;
    }
    let u = e.u;
    let digits = hv_digits(gl);
    let pw = hv_text_w(digits.len() as u32) + 4;
    let x0 = e.x + e.w_l.saturating_sub(pw) / 2 * u;
    let y0 = e.y.saturating_sub((HV_GL_H + 1) * u);
    hv_rect(buf, win_w, win_h, (x0, y0, pw * u, HV_GL_H * u), HV_OUTLINE);
    hv_rect(buf, win_w, win_h, (x0 + u, y0 + u, (pw - 2) * u, (HV_GL_H - 2) * u), HV_GL_PLATE);
    for (i, d) in digits.iter().enumerate() {
        for (r, bits) in HV_DIGITS[usize::from(*d).min(9)].iter().enumerate() {
            for c in 0..3u32 {
                if bits & (0b100 >> c) != 0 {
                    let ink = if r == 0 { HV_GL_INK_TOP } else { HV_GL_INK };
                    hv_rect(buf, win_w, win_h, (x0 + (2 + i as u32 * 4 + c) * u, y0 + (1 + r as u32) * u, u, u), ink);
                }
            }
        }
    }
}

pub fn hv_draw_exp(buf: &mut [u32], win_w: u32, win_h: u32, e: &HvExp, exp: u32, next: u32) {
    if e.u == 0 || e.w_l == 0 {
        return;
    }
    let frac = if next == 0 { 0.0 } else { f64::from(exp.min(next)) / f64::from(next) };
    let earned = ((frac * f64::from(e.w_l)).round() as u32).min(e.w_l);
    let col = |lx0: u32, lx1: u32, ly: u32, color: u32, buf: &mut [u32]| {
        if lx1 > lx0 {
            hv_rect(buf, win_w, win_h, (e.x + lx0 * e.u, e.y + ly * e.u, (lx1 - lx0) * e.u, e.u), color);
        }
    };
    for (lx0, lx1, frame, hi, lo) in [(0, earned, HV_EXP_FRAME, HV_EXP_HI, HV_EXP_LO), (earned, e.w_l, HV_OUTLINE, HV_EXP_TRACK_HI, HV_EXP_TRACK_LO)] {
        col(lx0, lx1, 0, frame, buf);
        col(lx0, lx1, 1, hi, buf);
        col(lx0, lx1, 2, lo, buf);
        col(lx0, lx1, 3, frame, buf);
    }
    // The end caps are frame, whichever part reaches them.
    for (lx, frame) in [(0, if earned > 0 { HV_EXP_FRAME } else { HV_OUTLINE }), (e.w_l - 1, if earned >= e.w_l { HV_EXP_FRAME } else { HV_OUTLINE })] {
        col(lx, lx + 1, 1, frame, buf);
        col(lx, lx + 1, 2, frame, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 2220;
    const H: u32 = 1080;
    const BG: u32 = 0x0012_3456;

    fn window() -> Vec<u32> {
        vec![BG; (W * H) as usize]
    }

    fn at(buf: &[u32], x: u32, y: u32) -> u32 {
        buf[(y * W + x) as usize]
    }

    /// The screen pixel in the middle of hud pixel `(lx, ly)` of the block.
    fn hud(buf: &[u32], g: &HvGeom, lx: u32, ly: u32) -> u32 {
        at(buf, g.x + lx * g.u + g.u / 2, g.y + ly * g.u + g.u / 2)
    }

    /// The S9 in landscape: 2220×1080 over a 492×240 guest.
    fn geom() -> HvGeom {
        HvGeom::new((0, 0, W, H), 492, 240)
    }

    fn stats(hp: u32, sp: u32, mp: u32) -> HvStats {
        HvStats { hp, hp_max: 700, sp, sp_max: 390, mp, mp_max: 200, level: 48 }
    }

    #[test]
    fn a_hud_pixel_is_the_games_own_pixel() {
        assert_eq!(hv_unit(1080, 240), 5, "S9 / S25 landscape: 1080 / 240 = 4.5");
        assert_eq!(hv_unit(1440, 320), 5, "portrait 240x320 on a 1080-wide screen");
        assert_eq!(hv_unit(1440, 240), 6, "a 1440-tall landscape");
        assert_eq!(hv_unit(200, 240), 2, "never below two");
        assert_eq!(hv_unit(1080, 0), 2, "no logical size: a floor, not a division by zero");
    }

    #[test]
    fn the_block_fits_the_window_and_holds_its_parts() {
        for (w, h, lw, lh) in [(2220u32, 1080u32, 492u32, 240u32), (2340, 1080, 520, 240), (1080, 1440, 240, 320), (1600, 720, 532, 240)] {
            let g = HvGeom::new((0, 0, w, h), lw, lh);
            let (bx, by, bw, bh) = g.bbox();
            assert!(bx + bw <= w && by + bh <= h, "{w}x{h}: bbox {:?}", g.bbox());
            for part in [g.ring(), g.bars(), g.plate(), g.badge(0), g.badge(1)] {
                assert!(part.0 >= bx && part.1 >= by && part.0 + part.2 <= bx + bw && part.1 + part.3 <= by + bh, "{w}x{h}: {part:?} outside {:?}", g.bbox());
            }
            assert!(g.plate().1 >= g.bars().1 + g.bars().3, "{w}x{h}: the plate is under the bars");
        }
        assert_eq!(HvGeom::new((0, 0, 1080, 1440), 240, 320).bar_r_l, HV_BAR_R_MAX, "45 % of 240 is past the cap");
        assert_eq!(HvGeom::new((0, 0, 600, 600), 150, 240).bar_r_l, 67, "a narrow game: 45 % of it");
    }

    #[test]
    fn the_three_bars_fill_from_the_ring_and_carry_their_values() {
        let g = geom();
        let mut buf = window();
        hv_draw_status(&mut buf, W, H, &g, &stats(700, 0, 100), None);
        // Near the right end of each bar's body row: HP full, SP empty, MP half (= track there).
        assert_eq!(hud(&buf, &g, hv_bar_end(g.bar_r_l, 6) - 3, 6), HV_HP[2], "a full HP bar is red to its end");
        assert_eq!(hud(&buf, &g, hv_bar_end(g.bar_r_l, 14) - 3, 14), HV_SP_TRACK[1], "an empty SP bar is all track");
        assert_eq!(hud(&buf, &g, hv_bar_end(g.bar_r_l, 22) - 3, 22), HV_MP_TRACK[1], "half MP: track at the right");
        assert_eq!(hud(&buf, &g, HV_RING + 1, 22), HV_MP[1], "…and green at the left");
        // Just right of the ring, an empty bar shows track, not a sliver of colour.
        assert_eq!(hud(&buf, &g, HV_RING + 1, 14), HV_SP_TRACK[1]);
        // The separators are outline rows; the block's bottom too.
        for ly in [1, 10, 18, 27] {
            assert_eq!(hud(&buf, &g, 40, ly), HV_OUTLINE, "row {ly}");
        }
        // Each value box sits on its bar's visible middle.
        for (top, ink, bx) in [(13u32, HV_SP_INK, HV_SP_BOX), (21, HV_MP_INK, HV_MP_BOX)] {
            let mid = (HV_RING + 1 + hv_bar_end(g.bar_r_l, top) - 2) / 2;
            let p = hud(&buf, &g, mid, top + 1);
            assert!(p == bx || p == ink, "row {}: {p:#08x}", top + 1);
        }
        // The slant: the outline pixel of MP's top body row, and nothing beyond it.
        assert_eq!(hud(&buf, &g, hv_bar_end(g.bar_r_l, 21), 21), HV_OUTLINE);
        assert_eq!(hud(&buf, &g, hv_bar_end(g.bar_r_l, 21) + 1, 21), BG);
    }

    #[test]
    fn mps_left_end_is_capped_below_the_ring() {
        let g = geom();
        let mut buf = window();
        hv_draw_status(&mut buf, W, H, &g, &stats(1, 1, 200), None);
        assert_eq!(hud(&buf, &g, HV_BAR_X0, 24), HV_OUTLINE, "the cap's outline");
        assert_eq!(hud(&buf, &g, HV_BAR_X0 + 1, 24), HV_SILVER, "the cap's silver");
        assert_eq!(hud(&buf, &g, HV_BAR_X0 + 2, 24), HV_MP[3], "the fill starts inside the cap");
        assert_eq!(hud(&buf, &g, HV_BAR_X0 - 1, 24), BG, "nothing left of the cap");
        let mut empty = window();
        hv_draw_status(&mut empty, W, H, &g, &stats(1, 1, 0), None);
        assert_eq!(hud(&empty, &g, HV_BAR_X0 + 2, 24), HV_MP_TRACK[3], "an empty MP bar shows no green sliver below the ring");
    }

    #[test]
    fn the_notches_between_the_bars_follow_the_reference() {
        let r = HV_BAR_R_MAX;
        assert_eq!(hv_bar_end(r, 1), r);
        assert_eq!(hv_bar_end(r, 9), r - 8);
        assert_eq!(hv_bar_end(r, 10), r, "the separator runs out to SP's top-right corner");
        assert_eq!(hv_bar_end(r, 11), r - 1);
        assert_eq!(hv_bar_end(r, 17), r - 7);
        assert_eq!(hv_bar_end(r, 18), r, "and MP's");
        assert_eq!(hv_bar_end(r, 19), r - 1);
        assert_eq!(hv_bar_end(r, 27), r - 9);
    }

    #[test]
    fn the_level_is_on_the_plate() {
        let g = geom();
        let mut buf = window();
        hv_draw_status(&mut buf, W, H, &g, &HvStats { level: 7, ..stats(1, 1, 1) }, None);
        let (px, py, pw, ph) = g.plate();
        let ink = (py..py + ph).flat_map(|y| (px..px + pw).map(move |x| (x, y))).filter(|(x, y)| at(&buf, *x, *y) == HV_LV_INK).count();
        assert_eq!(ink as u32, 7 * g.u * g.u, "the 7 glyph: seven hud pixels");
    }

    #[test]
    fn the_badges_sit_right_of_the_plate_one_hud_pixel_per_art_pixel() {
        let g = geom();
        assert!(g.badge(0).0 > g.plate().0 + g.plate().2, "right of the plate");
        assert!(g.badge(0).0 + g.badge(0).2 < g.badge(1).0, "the two do not touch");
        assert!(g.badge(1).0 + g.badge(1).2 <= g.bbox().0 + g.bbox().2, "inside the block's box");
        // An 8×8 red plus, transparent corners.
        let mut px = vec![0xffc0_0000u32; 64];
        px[0] = 0;
        let mut buf = window();
        hv_draw_badges(&mut buf, W, H, &g, [Some(HvIcon { w: 8, h: 8, px: &px }), None]);
        let (bx, by, _, _) = g.badge(0);
        assert_eq!(at(&buf, bx + g.u / 2, by + g.u / 2), BG, "a transparent pixel");
        assert_eq!(at(&buf, bx + g.u + g.u / 2, by + g.u / 2), 0x00c0_0000, "the art, alpha dropped");
        let (sx, sy, sw, sh) = g.badge(1);
        assert!((sy..sy + sh).all(|y| (sx..sx + sw).all(|x| at(&buf, x, y) == BG)), "no skill points: nothing");
    }

    #[test]
    fn the_head_is_drawn_inside_the_ring_and_nowhere_else() {
        let g = geom();
        // A 16x14 head: all opaque white except a transparent top-left pixel.
        let mut px = vec![0xffff_ffffu32; 16 * 14];
        px[0] = 0;
        let mut buf = window();
        hv_draw_status(&mut buf, W, H, &g, &stats(1, 1, 1), Some(HvIcon { w: 16, h: 14, px: &px }));
        assert_eq!(hud(&buf, &g, 12, 12), 0x00ff_ffff, "the ring's centre shows the head, alpha dropped");
        assert_ne!(hud(&buf, &g, 4, 5), 0x00ff_ffff, "clipped to the inside");
        let lum = |p: u32| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff);
        assert!(lum(hud(&buf, &g, 5, 5)) > lum(hud(&buf, &g, 18, 18)), "lit from the upper left");
        let mut none = window();
        hv_draw_status(&mut none, W, H, &g, &HvStats::default(), None);
        assert_eq!(hud(&none, &g, 12, 12), HV_RING_IN);
    }

    #[test]
    fn nothing_is_drawn_outside_the_block_and_no_alpha_byte_is_written() {
        let g = geom();
        let mut buf = window();
        hv_draw_status(&mut buf, W, H, &g, &HvStats { hp: 99999, hp_max: 99999, sp: 65535, sp_max: 65535, mp: 9999, mp_max: 9999, level: 99 }, None);
        let (bx, by, bw, bh) = g.bbox();
        for y in 0..H {
            for x in 0..W {
                let p = at(&buf, x, y);
                assert_eq!(p >> 24, 0, "({x},{y}): softbuffer wants 0x00RRGGBB");
                if !(x >= bx && x < bx + bw && y >= by && y < by + bh) {
                    assert_eq!(p, BG, "({x},{y}) is outside the block");
                }
            }
        }
    }

    #[test]
    fn the_exp_bar_splits_earned_from_owed() {
        let e = HvExp::new((0, 0, W, H), 492, 240);
        let (x, y, w, h) = e.rect();
        assert!(x + w <= W && y + h <= H && y > H / 2, "bottom half: {:?}", e.rect());
        assert!((x + w / 2).abs_diff(W / 2) <= 1, "centred: {:?}", e.rect());
        let row = |buf: &[u32], lx: u32, ly: u32| at(buf, e.x + lx * e.u + e.u / 2, e.y + ly * e.u + e.u / 2);
        let mut empty = window();
        hv_draw_exp(&mut empty, W, H, &e, 0, 1000);
        assert_eq!(row(&empty, e.w_l / 2, 1), HV_EXP_TRACK_HI);
        let mut full = window();
        hv_draw_exp(&mut full, W, H, &e, 1000, 1000);
        assert_eq!(row(&full, e.w_l / 2, 1), HV_EXP_HI);
        assert_eq!(row(&full, e.w_l / 2, 0), HV_EXP_FRAME);
        let mut half = window();
        hv_draw_exp(&mut half, W, H, &e, 500, 1000);
        assert_eq!(row(&half, e.w_l / 4, 2), HV_EXP_LO);
        assert_eq!(row(&half, e.w_l * 3 / 4, 2), HV_EXP_TRACK_LO);
        assert_eq!(row(&half, e.w_l * 3 / 4, 3), HV_OUTLINE);
        for yy in 0..H {
            for xx in 0..W {
                if !(xx >= x && xx < x + w && yy >= y && yy < y + h) {
                    assert_eq!(at(&half, xx, yy), BG);
                }
            }
        }
    }

    #[test]
    fn a_tiny_window_clips_instead_of_panicking() {
        let (w, h) = (40u32, 30u32);
        let mut buf = vec![BG; (w * h) as usize];
        let g = HvGeom::new((0, 0, w, h), 240, 320);
        hv_draw_status(&mut buf, w, h, &g, &HvStats { hp: 5, hp_max: 9, sp: 1, sp_max: 2, mp: 0, mp_max: 3, level: 7 }, None);
        hv_draw_exp(&mut buf, w, h, &HvExp::new((0, 0, w, h), 240, 320), 3, 9);
        hv_draw_buffs(&mut buf, w, h, &g, &[HvBuff { icon: None, secs: 30 }; 4]);
    }

    /// HZ-022's plate: dark, over the EXP bar's middle, the GL's digits in violet; nothing for 0.
    #[test]
    fn the_guardian_plate_sits_over_the_exp_bar() {
        let (w, h) = (2220u32, 1080u32);
        let e = HvExp::new((0, 0, w, h), 520, 240);
        let mut buf = vec![BG; (w * h) as usize];
        hv_draw_gl(&mut buf, w, h, &e, 0);
        assert!(buf.iter().all(|&p| p == BG), "GL 0 draws nothing");
        hv_draw_gl(&mut buf, w, h, &e, 37);
        let (ex, ey, ew, _) = e.rect();
        let above: Vec<u32> = (ey.saturating_sub(HV_GL_H * e.u)..ey).flat_map(|y| (ex..ex + ew).map(move |x| (x, y))).map(|(x, y)| buf[(y * w + x) as usize]).collect();
        assert!(above.contains(&HV_GL_INK) && above.contains(&HV_GL_PLATE), "violet digits on the plate");
        assert!((ey..h).all(|y| (0..w).all(|x| buf[(y * w + x) as usize] == BG)), "nothing on the bar itself");
    }

    /// HZ-019 lifts the game's pickup notices `NOTICE_LIFT` guest px off the bottom: over the EXP
    /// bar's top on the phones this runs on (a hud px is the guest px rounded, so up to half a
    /// screen px more each).
    #[test]
    fn the_pickup_notices_clear_the_exp_bar() {
        let lift = hz_patches::hooks::NOTICE_LIFT as u32;
        for (w, h, lw, lh) in [(2220u32, 1080u32, 520u32, 240u32), (2340, 1080, 520, 240), (1600, 720, 520, 240), (1080, 1440, 240, 320), (720, 960, 240, 320)] {
            let e = HvExp::new((0, 0, w, h), lw, lh);
            let notices_bottom = (lh - lift) * h / lh;
            assert!(e.rect().1 >= notices_bottom, "{w}x{h}: the bar's top {} under the notices' bottom {notices_bottom}", e.rect().1);
        }
    }

    /// hero2's cells: right of the bars on their top row, clear of the block, inside the row the
    /// block claims ([`HvGeom::buff_row`]); the game's three states fit one row in landscape.
    #[test]
    fn the_buff_cells_sit_right_of_the_bars() {
        for (w, h, lw, lh) in [(2220u32, 1080u32, 520u32, 240u32), (2220, 1080, 492, 240), (1080, 1440, 240, 320)] {
            let g = HvGeom::new((0, 0, w, h), lw, lh);
            let (bx, by, bw, _) = g.bbox();
            let (rx, ry, rw, rh) = g.buff_row();
            for i in 0..HV_BUFFS_MAX.min(if w > h { 3 } else { 1 }) {
                let c = g.buff_cell(i);
                assert!(c.0 >= bx + bw && c.1 == by + g.u, "{w}x{h}: cell {i} {c:?} right of the bars");
                assert!(c.0 >= rx && c.0 + c.2 <= rx + rw && c.1 >= ry && c.1 + c.3 <= ry + rh, "{w}x{h}: cell {i} {c:?} in the claimed row");
            }
        }
    }
}
