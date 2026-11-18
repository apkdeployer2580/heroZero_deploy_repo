//! The touch HUD's geometry (Phase 2): where the game picture goes, every control's BUILT-IN seat,
//! the host chrome, the obstacles a placed control may not cover, the hit-test chain and the
//! joystick quantiser. Pure functions of [`Screen`] (the window and the guest canvas, which is
//! fixed at boot) — no window, no JVM — so all of it is tested on the host.
//!
//! The design is hero2's (`wie_android/src/overlay.rs` + `placement.rs`, settled over two months
//! of player reports on device); the code is new and carries none of hero2's game knowledge. Two
//! layout families (featurephone-port android-host, input-and-hud.md §9):
//!
//! * **landscape immersive** — the picture fills the window and the controls are translucent
//!   overlays on top of it. Every control's seat comes from [`crate::placement`] (built-in here,
//!   the player's placement on top).
//! * **portrait** — the picture at the top, the controls in the strip below it (user 2026-10-29:
//!   the handset keypad gave way to the quick plates, MAP and SAVE came down from the picture's top,
//!   and the strip is placed, edited and walked as landscape's is — [`portrait_seats`] are its
//!   built-ins, and the player's own layout is a table of its own, `crate::placement`).
//!
//! Both have hero2's three control SETS ([`ControlSet`], hero2's `Mode`), picked by what the
//! game shows: the field (the walk stick, OK, CLR, the quick plates), a window (four hexagon
//! arrows cut from the stick's seat, OK, CLR and a small gear) and a scenario's dialogue (nothing
//! drawn: the picture turns the page, and an invisible target over the game's own ▶▶ skips —
//! hero2's `#SKIP`). The game's bottom bar (HZ-013) and its windows' soft-key pictures (HZ-024)
//! are gone, so the controls sit down to the window's bottom edge as hero2's do (2026-10-26).

use hz_java::{GuestView, Scene};
use wie_backend::KeyCode;

use crate::placement::{self, CtrlId, Table};
use crate::ui_icons::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// The square of side `side` centred on (`cx`, `cy`).
    pub fn square_at(cx: i32, cy: i32, side: i32) -> Self {
        Self::new(cx - side / 2, cy - side / 2, side, side)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn centre(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x) && y >= f64::from(self.y) && x < f64::from(self.right()) && y < f64::from(self.bottom())
    }

    /// Half-open overlap; touching edges do not overlap, and an empty rect overlaps nothing.
    pub fn intersects(&self, o: &Rect) -> bool {
        !self.is_empty() && !o.is_empty() && self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    pub fn grown(&self, n: i32) -> Rect {
        Rect::new(self.x - n, self.y - n, self.w + 2 * n, self.h + 2 * n)
    }

    pub fn inside(&self, o: &Rect) -> bool {
        self.x >= o.x && self.y >= o.y && self.right() <= o.right() && self.bottom() <= o.bottom()
    }

    pub fn clipped(&self, o: &Rect) -> Rect {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
}

/// The game picture's rectangle. Landscape: fitted into the window, centred (the guest width
/// already follows the window aspect, so this is the whole window up to rounding). Portrait: the
/// full width at the top, at most 62 % of the height, so the controls get the rest.
pub fn game_rect(win_w: u32, win_h: u32, guest_w: u32, guest_h: u32) -> Rect {
    let landscape = win_w > win_h;
    let (ww, wh, gw, gh) = (f64::from(win_w), f64::from(win_h), f64::from(guest_w.max(1)), f64::from(guest_h.max(1)));
    let scale = if landscape {
        (wh / gh).min(ww / gw)
    } else {
        (ww / gw).min(wh * 0.62 / gh)
    };
    let (w, h) = ((gw * scale).round() as i32, (gh * scale).round() as i32);
    let x = (win_w as i32 - w) / 2;
    let y = if landscape { (win_h as i32 - h) / 2 } else { 0 };
    Rect::new(x, y, w, h)
}

/// The picture fitted into the window on both axes and centred, whatever the window's shape (the
/// PC's resizable window, 2026-10-29): no strip for controls, the keyboard has them.
pub fn game_rect_fit(win_w: u32, win_h: u32, guest_w: u32, guest_h: u32) -> Rect {
    let (ww, wh, gw, gh) = (f64::from(win_w), f64::from(win_h), f64::from(guest_w.max(1)), f64::from(guest_h.max(1)));
    let scale = (wh / gh).min(ww / gw);
    let (w, h) = ((gw * scale).round() as i32, (gh * scale).round() as i32);
    Rect::new((win_w as i32 - w) / 2, (win_h as i32 - h) / 2, w, h)
}

/// The window, the picture in it and the guest canvas behind the picture. Everything in this
/// module is a function of this value alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub w: i32,
    pub h: i32,
    pub game: Rect,
    pub guest_w: i32,
    pub guest_h: i32,
    /// The PC (`hz_desktop`, 2026-10-29): a mouse and a keyboard, no thumbs — no stick, arrows, OK
    /// or CLR; the quick slots as a bar of cells with their keys ([`quick_cells`]); the picture
    /// fitted ([`game_rect_fit`]).
    pub desktop: bool,
}

impl Screen {
    pub fn new(win_w: u32, win_h: u32, guest_w: u32, guest_h: u32) -> Screen {
        Screen {
            w: win_w as i32,
            h: win_h as i32,
            game: game_rect(win_w, win_h, guest_w, guest_h),
            guest_w: guest_w.max(1) as i32,
            guest_h: guest_h.max(1) as i32,
            desktop: false,
        }
    }

    /// The PC's screen for a window ([`Screen::desktop`]).
    pub fn new_desktop(win_w: u32, win_h: u32, guest_w: u32, guest_h: u32) -> Screen {
        Screen {
            w: win_w as i32,
            h: win_h as i32,
            game: game_rect_fit(win_w, win_h, guest_w, guest_h),
            guest_w: guest_w.max(1) as i32,
            guest_h: guest_h.max(1) as i32,
            desktop: true,
        }
    }

    /// A landscape window with the guest canvas the APK would boot for it.
    pub fn for_window(win_w: u32, win_h: u32) -> Screen {
        let (gw, gh) = crate::guest_size(win_w, win_h, win_w > win_h);
        Screen::new(win_w, win_h, gw, gh)
    }

    pub fn landscape(&self) -> bool {
        self.w > self.h
    }

    pub fn window(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// The standard control margin.
    pub fn pad(&self) -> i32 {
        (self.w.min(self.h) / 110).max(3)
    }

    fn gx(&self, x: i32) -> i32 {
        self.game.x + x * self.game.w / self.guest_w
    }

    fn gy(&self, y: i32) -> i32 {
        self.game.y + y * self.game.h / self.guest_h
    }

    /// The guest pixel under a window point, if it is on the game's picture.
    pub fn guest_point(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        if !self.game.contains(x, y) || self.game.w <= 0 || self.game.h <= 0 {
            return None;
        }
        let gx = ((x - f64::from(self.game.x)) * f64::from(self.guest_w) / f64::from(self.game.w)) as i32;
        let gy = ((y - f64::from(self.game.y)) * f64::from(self.guest_h) / f64::from(self.game.h)) as i32;
        Some((gx.clamp(0, self.guest_w - 1), gy.clamp(0, self.guest_h - 1)))
    }
}

// ------------------------------------------------------------- the game's own HUD ----

/// Plates a page (HZ-058, hero2's seven — `hz_java::quickpage::PLATES`).
pub const QUICK_PLATES: usize = hz_java::quickpage::PLATES;

/// The PC's keys for the plates (user 2026-10-30: Q W E R A S D, `Tab` the other page —
/// `crate::app`), each plate's cell labelled with its key.
pub const PC_PLATE_KEYS: [&str; QUICK_PLATES] = ["Q", "W", "E", "R", "A", "S", "D"];

/// The PC's quick bar (the user's choice ②, 2026-10-29: *"PC버전용으로 퀵슬롯 새로 구현"*; HZ-058, 2026-10-30:
/// hero2's seven): the page's plates as square cells in a row at the picture's bottom centre, just
/// above the host's EXP bar — each shows what the plate holds as the phone's do and its key
/// ([`PC_PLATE_KEYS`]) in its corner; a click uses it. The page cell ([`page_cell`]) follows them.
/// 11 % of the picture's height a side, a sixth of that apart.
pub fn quick_cells(s: &Screen) -> [Rect; QUICK_PLATES] {
    let (x0, y, side, gap) = quick_row(s);
    std::array::from_fn(|i| Rect::new(x0 + i as i32 * (side + gap), y, side, side))
}

/// The PC's page cell: after the seventh plate, half as wide, `1/2` over `TAB`.
pub fn page_cell(s: &Screen) -> Rect {
    let (x0, y, side, gap) = quick_row(s);
    Rect::new(x0 + QUICK_PLATES as i32 * (side + gap), y, side / 2 + gap, side)
}

/// The PC's bar: `(left, top, side, gap)` — the seven cells and the half-wide page cell, centred.
fn quick_row(s: &Screen) -> (i32, i32, i32, i32) {
    let g = s.game;
    // Seven and a half cells and their gaps (about 8.8 sides) across the picture at most.
    let side = (g.h * 11 / 100).clamp(20, 160).min((g.w * 6 / 53).max(16));
    let gap = (side / 6).max(2);
    let n = QUICK_PLATES as i32;
    let total = n * side + n * gap + side / 2 + gap;
    let x0 = g.x + (g.w - total) / 2;
    let y = (exp_bar(s).y - gap - side).max(g.y);
    (x0, y, side, gap)
}

/// The game's ▶▶ in a scenario's dialogue box (`eb.a(dd)` @796..812: `/ff.png`, 10×7, at
/// (W − 1, H − 1) BOTTOM|RIGHT while no window is up) — the mark for RSK, which skips the
/// conversation (`eb.a(II)Z` @784). Guest px.
const SKIP_HINT_W: i32 = 10;
const SKIP_HINT_H: i32 = 7;
const SKIP_HINT_MARGIN: i32 = 1;
/// How much further up and left than the ink the target reaches, guest px: a thumb aiming at a
/// 10 px mark needs more than the mark (hero2's `#SKIP` takes 6 beyond a 30 px word).
const SKIP_HINT_REACH: i32 = 14;

/// The invisible RSK target over the game's ▶▶ (hero2's `game_skip_hint`): from a little above-left
/// of the ink OUT to the picture's bottom-right corner, so it holds the ink by construction — and
/// never less than a tenth of the window's height each way (a 720-high phone's 3× picture).
pub fn skip_hint(s: &Screen) -> Rect {
    let least = s.h / 10;
    let x = s.gx(s.guest_w - SKIP_HINT_MARGIN - SKIP_HINT_W - SKIP_HINT_REACH).min(s.game.right() - least);
    let y = s.gy(s.guest_h - SKIP_HINT_MARGIN - SKIP_HINT_H - SKIP_HINT_REACH).min(s.game.bottom() - least);
    Rect::new(x, y, s.game.right() - x, s.game.bottom() - y)
}

// ------------------------------------------------------- landscape built-in seats ----

/// The quick plates on the ring around the OK disc, from just right of straight up, over the
/// top, to below-left (the ring's seat 0, before them, is CLR's — hero2's ring). The thumb rests
/// on OK and sweeps left, so the keys it uses most sit there: the two character skills (`1`, `3`)
/// and the two recovery items (`0`, `#`); the spirit skills (`7`, `9`) above. Keys from the game's
/// own help (도움말 › 기본 조작: 스킬 1,3,7,9 · 회복 아이템 0,#). The `*` that sat in the top
/// corner (the game's minimap window) went with hero2's (2026-10-26): menu 8, the MAP tab and the
/// pad's SELECT open it.
///
/// HZ-058 (user 2026-10-30, hero2's seven): the seats hold the host's plates (`hz_java::quickpage`) —
/// `(plate, label, layout key)` in seat order. A player who never set plates starts with the game's
/// six where they were (plate 1 = the `1` slot … 5 = `0`, 6 = `#`), so the first six seats keep those
/// plates and their old layout keys; the seventh seat, below-left, is new.
pub const RING_PLATES: [(u8, &str, &str); 7] = [
    (3, "4", "k9"),
    (2, "3", "k7"),
    (1, "2", "k3"),
    (0, "1", "k1"),
    (4, "5", "k0"),
    (5, "6", "khash"),
    (6, "7", "p7"),
];

/// Seats on the ring: CLR's, then one per plate.
const RING_SEATS: usize = 1 + RING_PLATES.len();

/// The ring's arc, degrees, screen-up = 90 (hero2's, which is hero3's): from just right of the
/// top to just below-left, short of the bottom edge.
const RING_A0: f64 = 66.0;
const RING_A1: f64 = 195.0;
/// The widest pitch the ring uses: the six-seat ring's (hero2's `RING_STEP_MAX`).
const RING_STEP_MAX: f64 = (RING_A1 - RING_A0) / 5.0;

/// The y below which a landscape control may not reach: the window's bottom (hero2's — the game
/// draws nothing there since HZ-013 and HZ-024; the EXP bar between the stick and OK is clear of
/// both at their seats).
fn band_top(s: &Screen) -> i32 {
    s.h
}

/// The action cluster's centre (hero2's `action_centre`): from the cluster's nominal radius, not
/// the disc's own size, so a smaller disc does not drag the ring into the corner.
fn ok_centre(s: &Screen) -> (i32, i32) {
    let pad = s.pad();
    let nominal = (s.h / 6).clamp(56, 190);
    (s.w - pad * 3 - nominal, band_top(s) - pad * 3 - nominal)
}

/// The OK disc's bounding square.
pub fn ok_disc(s: &Screen) -> Rect {
    if !s.landscape() && !s.desktop {
        return portrait_seats(s).ok;
    }
    let (cx, cy) = ok_centre(s);
    Rect::square_at(cx, cy, (s.h / 8).clamp(48, 140) * 2)
}

/// hero2's `disc_side_button`: left of OK, level with it, three fifths of it — no button stands
/// there any more; it is CLR's size and the room the ring keeps clear beside the disc.
fn disc_side_button(s: &Screen) -> Rect {
    let pad = s.pad();
    let a = ok_disc(s);
    let d = (a.w * 3 / 5).max(56);
    Rect::new(a.x - pad * 2 - d, a.y + a.h / 2 - d / 2, d, d)
}

/// The ring's seats (hero2's `ring_slots`, [`RING_SEATS`] of them: 0 = CLR's, then the plates in
/// [`RING_PLATES`] order): equal steps from [`RING_A0`] (at most [`RING_STEP_MAX`]) at a radius that
/// clears the disc and the room beside it and keeps neighbouring squares apart (1.485 — a diagonal
/// offset can leave both |dx| and |dy| under the side), at the LARGEST plate that fits the window
/// (bisected); never empty.
pub fn ring(s: &Screen) -> Vec<Rect> {
    let n = RING_SEATS;
    let pad = f64::from(s.pad());
    let (w, h) = (f64::from(s.w), f64::from(s.h));
    let ar = f64::from(ok_disc(s).w) / 2.0;
    let (acx, acy) = ok_centre(s);
    let (acx, acy) = (f64::from(acx), f64::from(acy));
    let clear = acx - f64::from(disc_side_button(s).x);
    let step = ((RING_A1 - RING_A0) / (n - 1) as f64).min(RING_STEP_MAX);
    let radius = |side: f64| {
        let need = side * 1.485 / (2.0 * (step.to_radians() / 2.0).sin());
        need.max(ar + side * 0.75).max(clear + side / 2.0 + pad * 2.0)
    };
    let seat = |i: usize, r: f64, side: f64| {
        let a = (RING_A0 + step * i as f64).to_radians();
        (acx + r * a.cos(), acy - r * a.sin(), side)
    };
    let try_side = |side: f64| -> Option<Vec<Rect>> {
        let r = radius(side);
        (0..n)
            .map(|i| {
                let (cx, cy, side) = seat(i, r, side);
                let (x0, y0) = (cx - side / 2.0, cy - side / 2.0);
                (x0 >= 0.0 && y0 >= 0.0 && x0 + side <= w && y0 + side <= h).then(|| Rect::new(x0 as i32, y0 as i32, side as i32, side as i32))
            })
            .collect()
    };
    let base = f64::from((s.h / 11).clamp(40, 120));
    if let Some(seats) = try_side(base) {
        return seats;
    }
    let (mut lo, mut hi) = (0.0f64, base);
    let mut best = None;
    for _ in 0..14 {
        let mid = (lo + hi) / 2.0;
        match try_side(mid) {
            Some(seats) => {
                best = Some(seats);
                lo = mid;
            }
            None => hi = mid,
        }
    }
    if let Some(seats) = best {
        return seats;
    }
    // Clamp the last attempt into the window rather than lose controls.
    let side = lo.max(16.0);
    let r = (ar + side * 0.75).max(clear + side / 2.0 + pad * 2.0);
    (0..n)
        .map(|i| {
            let (cx, cy, side) = seat(i, r, side);
            let (cx, cy) = (cx.clamp(side / 2.0, w - side / 2.0), cy.clamp(side / 2.0, h - side / 2.0));
            Rect::new((cx - side / 2.0) as i32, (cy - side / 2.0) as i32, side as i32, side as i32)
        })
        .collect()
}

/// Quick plate `i`'s seat ([`RING_PLATES`] order): ring seat `i + 1` — portrait's grid.
pub fn plate_seat(s: &Screen, i: usize) -> Rect {
    if !s.landscape() && !s.desktop {
        return portrait_seats(s).plates[i];
    }
    ring(s)[i + 1]
}

/// HZ-058: the page button's seat — landscape: three fifths of a plate, left of the ring's last
/// seat and level with its bottom; portrait: the grid's eighth cell.
pub fn page_seat(s: &Screen) -> Rect {
    if !s.landscape() && !s.desktop {
        return portrait_seats(s).page;
    }
    let last = ring(s)[RING_SEATS - 1];
    let side = (last.w * 3 / 5).max(16);
    let pad = s.pad();
    let x = (last.x - pad * 2 - side).max(0);
    let y = (last.y + last.h - side).clamp(0, (s.h - side).max(0));
    Rect::new(x, y, side, side)
}

/// The CLR disc's seat (Phase 6-4, user 2026-10-23: *"hero2,3처럼 취소버튼 OK버튼위쪽에 넣어서
/// 그거누르면 메뉴열리게"*): hero2's `cancel_seat` — the ring's first seat, taken off the arc: at
/// the right edge, down from the first plate toward OK by exactly what clears the two circles,
/// [`disc_side_button`]'s size (three fifths of OK).
pub fn cancel_seat(s: &Screen) -> Rect {
    if !s.landscape() {
        return portrait_seats(s).cancel;
    }
    let pad = f64::from(s.pad());
    let d = disc_side_button(s).w.max(ring(s)[0].w).min(s.w).min(s.h);
    let r = f64::from(d) / 2.0;
    let q = plate_seat(s, 0);
    let (qcx, qcy) = (f64::from(q.x) + f64::from(q.w) / 2.0, f64::from(q.y) + f64::from(q.h) / 2.0);
    let need = r + f64::from(q.w.min(q.h)) / 2.0 + pad * 2.0;
    let cx = (f64::from(s.w) - pad * 2.0 - r).max(r);
    let dx = (cx - qcx).abs();
    let dy = (need * need - dx * dx).max(0.0).sqrt();
    let cy = (qcy + dy).clamp(r, (f64::from(band_top(s)) - r).max(r));
    Rect::new((cx - r).max(0.0) as i32, (cy - r).max(0.0) as i32, d, d)
}

/// What CLR sends: the game's RSK — "back" in a window, and in the field the menu (`ce.a(II)Z` @171
/// sends −7 and −6 alike to @299, the menu; probe 2026-10-30 with every patch). Until 2026-10-30 the
/// field's was LSK, which START's [`PAGE_PAD_KEY`] now turns the plates with — so CLR's pad badge is
/// B's, the button that opens the menu.
pub fn cancel_key(_view: GuestView) -> KeyCode {
    KeyCode::RIGHT_SOFT_KEY
}

/// What OK sends: OK — but LSK while the window with the keys answers only the soft keys (the
/// 창고 question, [`GuestView::lsk_ok`]: HZ-024 took the □ that said so).
///
/// On the 「# 확인」 notice it is `#`, the one key that closes it (HZ-058 took the `#` plate; the pad's
/// A does the same — `gamepad::view_key`).
pub fn ok_key(view: GuestView) -> KeyCode {
    if view.notice {
        KeyCode::HASH
    } else if view.lsk_ok {
        KeyCode::LEFT_SOFT_KEY
    } else {
        KeyCode::OK
    }
}

/// The ONE walk control's square (hero2's `stick_seat`): bottom-left, `pad * 4` in from both
/// edges, two of hero2's old three-key columns wide. The field's stick is a disc inside it
/// ([`Stick::base_r`]: the circle through the arrows' centres) and a window's four arrows tile it
/// ([`dpad_arms`]), so the player sizes and moves both at once.
pub fn stick_seat(s: &Screen) -> Rect {
    if !s.landscape() && !s.desktop {
        return portrait_seats(s).stick;
    }
    let pad = s.pad();
    let cell = (s.h / 11).clamp(40, 120);
    let span = ((cell * 3 + pad * 2) * 2).min((s.h - pad * 4).max(1)).min(s.w.max(1));
    Rect::new((pad * 4).min(s.w - span), band_top(s) - span - pad * 4, span, span)
}

/// The settings gear (landscape: hero2's `gear_row` — small, top centre a little right of the
/// middle, where its `123` toggle no longer stands beside it; portrait: top centre of the control
/// strip). Landscape shows it only over the game's windows ([`layout`]).
pub fn gear(s: &Screen) -> Rect {
    // The PC's portrait window has no strip under the picture: the gear sits as landscape's does.
    if s.landscape() || s.desktop {
        let pad = s.pad();
        let gs = (s.w.min(s.h) / 22).clamp(30, 64);
        let ts = (gs * 6 / 5).min(s.h / 8);
        let total = gs + pad * 2 + ts;
        let x0 = (s.w / 2 + s.w / 14).min(s.w - total - pad) - total / 2;
        Rect::new(x0, pad * 2 + (ts - gs).max(0) / 2, gs, gs)
    } else {
        portrait_seats(s).gear
    }
}

// ----------------------------------------------------------------------- minimap ----

/// The minimap panel (Phase 6-5): hero2's `minimap_panel_builtin` — a square in the picture's
/// top-right corner (hero2's player: *"the minimap has to be at the very top-right, and the
/// monster name/HP/LV box to its left"*), 26 % of the picture's short side within 120..280 px,
/// two pads in. The game's target-monster panel, which the game draws there, moves left of it
/// ([`target_dodge`], HZ-014). Portrait (user 2026-10-29, second word: *"몬스터 피와 이름이 위에
/// 나오고 그 밑에 미니맵이 나오게"*): the panel keeps the game's own place at the top, and the minimap
/// starts under it ([`target_place`]).
pub fn minimap_panel(s: &Screen) -> Rect {
    let g = s.game;
    let side = (g.w.min(g.h) * 26 / 100).clamp(120, 280);
    let pad = s.pad();
    let top = if s.landscape() || s.guest_h <= 0 {
        g.y + pad * 2
    } else {
        g.y + ((hz_patches::hooks::TARGET_PANEL_H + TARGET_GAP) * g.h + s.guest_h - 1) / s.guest_h
    };
    Rect::new(g.right() - pad * 2 - side, top, side, side)
}

/// Portrait: the guest px between the target panel and the minimap.
const TARGET_GAP: i32 = 2;

/// The round button that brings a hidden minimap back (hero2's `minimap_restore_button`, player
/// 2026-10-21: *"사라진 미니맵 자리에 … 아이콘이 나타나게"*): the top-right corner of the panel `p`
/// (where the player put it, [`Layout::panel`]), a ring seat's size; its face is hero2's folded
/// map (`crate::ui_icons`).
pub fn minimap_restore(s: &Screen, p: Rect) -> Rect {
    let k = plate_seat(s, 0);
    let side = k.w.min(k.h).min(p.w).min(p.h).max(1);
    Rect::new(p.right() - side, p.y, side, side)
}

/// What the restore button drawn at `r` answers to (hero2's `minimap_restore_hit`): a quarter of
/// a button bigger each way, inside the window — the hidden panel's empty corner.
pub fn minimap_restore_hit(s: &Screen, r: Rect) -> Rect {
    let grow = r.w / 4;
    r.grown(grow).clipped(&s.window())
}

/// The status block as a tap target (hero2's `stats_tap_box`, its player 2026-08-03: *"제일 좌측
/// 상단에 Lv과 HP 표시하는곳 클릭하면 메뉴가 열리게"*): the block and its buff row, one pad bigger
/// each way. Tested after every control ([`hit`]), so it never takes a control's touch; it is a
/// TAP — down and up inside, little movement — so a thumb crossing it does not open the menu.
pub fn status_tap(s: &Screen) -> Rect {
    status_box(s).grown(s.pad()).clipped(&s.window())
}

/// What the minimap may not cover besides a control's obstacles: the status block and the top
/// tabs' touch targets, both of which sit where they sit whatever the layout (the tabs are
/// measured off the built-in panel, so moving it never moves them) — the tabs wherever a map
/// puts them ([`top_tab_seats`] over every [`fight_map_widths`]).
pub fn minimap_keep_clear(s: &Screen) -> Vec<Rect> {
    let mut out = vec![status_box(s)];
    for w in fight_map_widths(s) {
        let (save, map) = top_tab_seats(s, w);
        for r in [top_tab_hit(s, save), top_tab_hit(s, map)] {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out.retain(|r| !r.is_empty());
    out
}

/// Where the player may put the minimap panel (the layout editor, `CtrlId::Minimap`) beyond the
/// rules every control obeys: inside the game picture — it is a view of the game's map — clear of
/// [`minimap_keep_clear`], and only where the game's target panel can still get out of its way
/// (hero2's `target_box_can_dodge`: HZ-014 moves it left by the overlap, and it cannot leave the
/// canvas — a panel it cannot clear would hide the monster's name and HP in a fight) — over every
/// map it fights on, since a narrow map keeps it inside its picture, further left and with less
/// room ([`fight_map_widths`]; review 2026-10-25: a seat clear of the canvas's corner covered it
/// on map 135).
pub fn minimap_seat_ok(s: &Screen, panel: Rect) -> bool {
    if !s.landscape() {
        // Portrait: the target panel follows the minimap (HZ-054), so it is enough that it fits.
        let (shift, drop) = target_place(s, panel, s.guest_w);
        let target = target_box_at(s, shift, drop, s.guest_w);
        let status = status_box(s);
        return panel.inside(&s.game) && !status.intersects(&panel) && target.inside(&s.game) && !target.intersects(&status);
    }
    panel.inside(&s.game)
        && !minimap_keep_clear(s).iter().any(|k| k.intersects(&panel))
        && fight_map_widths(s).all(|w| !target_box(s, target_dodge(s, panel, w), w).intersects(&panel))
}

/// The map widths (guest px) the target panel can be drawn over on this canvas: each whole number
/// of tiles from the narrowest map with monsters ([`hz_java::hooks::FIGHT_MAP_MIN_W`]) up to the
/// canvas's width — a wider map's picture fills the canvas as one exactly that wide does.
fn fight_map_widths(s: &Screen) -> impl Iterator<Item = i32> {
    let w = s.guest_w;
    (hz_java::hooks::FIGHT_MAP_MIN_W..w).step_by(hz_java::hooks::MAP_TILE as usize).chain([w])
}

/// The game's target-monster panel on screen (`ce.f()`: 80×34 at y 0) over a map `map_w` guest px
/// wide, `shift` guest px left of where the game puts it ([`target_guest_x`]).
pub fn target_box(s: &Screen, shift: i32, map_w: i32) -> Rect {
    target_box_at(s, shift, 0, map_w)
}

/// [`target_box`] `drop` guest px lower (HZ-054).
pub fn target_box_at(s: &Screen, shift: i32, drop: i32, map_w: i32) -> Rect {
    let x = target_guest_x(s, shift, map_w);
    let (l, t) = (s.gx(x), s.gy(drop));
    Rect::new(l, t, s.gx(x + hz_patches::hooks::TARGET_PANEL_W) - l, s.gy(drop + hz_patches::hooks::TARGET_PANEL_H) - t)
}

/// Where the game's target panel goes for the minimap at `panel` over a map `map_w` guest px wide,
/// `(shift, drop)` — what HZ-014's and HZ-054's hooks take. Landscape: left of the panel by the
/// overlap ([`target_dodge`]), at the top. Portrait (user 2026-10-29: the monster's name and HP on
/// top, the minimap under them): over the panel, the two right edges together — at the built-in
/// seat that is the game's own place, `drop` 0 — or under it, where the player put the panel too
/// high for one over it.
pub fn target_place(s: &Screen, panel: Rect, map_w: i32) -> (i32, i32) {
    if s.landscape() || s.game.w <= 0 || s.game.h <= 0 {
        return (target_dodge(s, panel, map_w), 0);
    }
    let (w, h) = (hz_patches::hooks::TARGET_PANEL_W, hz_patches::hooks::TARGET_PANEL_H);
    let right = ((panel.right() - s.game.x) * s.guest_w / s.game.w).min(s.guest_w);
    let x = (right - w).clamp(0, (s.guest_w - w).max(0));
    // Over it while the panel fits there — the gap may shrink to nothing at the top edge.
    let above = (panel.y - s.game.y) * s.guest_h / s.game.h;
    let below = ((panel.bottom() - s.game.y) * s.guest_h + s.game.h - 1) / s.game.h + TARGET_GAP;
    let drop = if above >= h { (above - TARGET_GAP - h).max(0) } else { below.min(s.guest_h - h).max(0) };
    (s.guest_w - w - x, drop)
}

/// The target panel's canvas x as HZ-014's hook puts it: `width − 80 − shift`
/// ([`hz_java::hooks::target_x`]) kept inside the picture of a map `map_w` wide
/// ([`hz_java::hooks::target_x_on_map`]; a map as wide as the canvas, `s.guest_w`, leaves it where
/// the shift alone puts it).
fn target_guest_x(s: &Screen, shift: i32, map_w: i32) -> i32 {
    let (left, right) = hz_java::hooks::map_span(s.guest_w, map_w);
    hz_java::hooks::target_x_on_map(s.guest_w, shift, left, right)
}

/// HZ-014: the shift the hook takes for the target panel to clear `panel` over a map `map_w` guest
/// px wide — hero2's `target_box_shift_for`, an OVERLAP QUERY: 0 when they do not meet where the
/// panel is unshifted, else until its right edge clears the panel's left edge, plus 2. The hook
/// counts the shift from the canvas's `width − 80`, so a narrow map's own pull into its picture is
/// added in.
pub fn target_dodge(s: &Screen, panel: Rect, map_w: i32) -> i32 {
    let base = target_box(s, 0, map_w);
    if !base.intersects(&panel) || s.game.w <= 0 {
        return 0;
    }
    let need = base.right() - panel.x;
    let pulled = hz_java::hooks::target_x(s.guest_w, 0) - target_guest_x(s, 0, map_w);
    pulled + (need * s.guest_w + s.game.w - 1) / s.game.w + 2
}

// ---------------------------------------------------------------------- top tabs ----

/// A tab hanging from the window's top edge (Phase 7, user 2026-10-23: *"save, map 탭
/// 넣을꺼고"*) — hero2's, which are hero3's (vc422 measured on the S25 at 2340×1080: two 86×51
/// tabs at y 0, MAP left of SAVE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The game's own save, without its question (`hz_java::hooks` `quick_save`, 2026-10-29).
    Save,
    /// The world map.
    Map,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Save => "SAVE",
            Tab::Map => "MAP",
        }
    }
}

/// The status block's screen box (`hud_view`, top-left of the picture), badges included, and the
/// row of buff cells it claims right of its bars (hero2's).
pub fn status_box(s: &Screen) -> Rect {
    let g = s.game;
    let geom = crate::hud_view::HvGeom::new((g.x.max(0) as u32, g.y.max(0) as u32, g.w.max(0) as u32, g.h.max(0) as u32), s.guest_w as u32, s.guest_h as u32);
    let ((x, y, w, h), (bx, by, bw, bh)) = (geom.bbox(), geom.buff_row());
    let right = (x + w).max(bx + bw);
    let bottom = (y + h).max(by + bh);
    Rect::new(x as i32, y as i32, (right - x) as i32, (bottom - y) as i32)
}

/// The two tabs' faces, `(SAVE, MAP)`. hero2's rule (`quicksave_seat`) is "SAVE just left of the
/// game's target panel, MAP half a tab left of SAVE". The tabs share the top band between the
/// status block on the left (the field has no top chrome since the gear went to the windows,
/// 2026-10-26) and whatever starts furthest left on the right — the minimap panel and the target
/// panel where it goes to clear it (HZ-014), both taken as if the minimap were up so hiding it
/// never moves them, and at its built-in seat so moving it never does either (a seat the player
/// picks for it keeps clear of the tabs, [`minimap_seat_ok`]). A tab is a ring plate wide (hero2's
/// `top_tab_size`), narrowed until the two touch targets (1½ tabs each, [`top_tab_hit`]) fit the
/// band, three fifths of its width tall; SAVE's target ends at the band's right end, MAP's at its
/// left. Where the target panel already sits under the status block (a 4:3 window, portrait) the
/// band runs to the minimap and the panel may pass under the tabs in combat.
///
/// `map_w` is the width (guest px) of the map the target panel is drawn over: a map narrower than
/// the canvas keeps the panel inside its picture, further left (HZ-014, [`target_box`]), and the
/// band stops short of it there too while two tabs still fit (user 2026-10-25: a 20-tile room's
/// panel had SAVE on its corner). `s.guest_w` for a map with no monsters, or one as wide as the
/// canvas.
pub fn top_tab_seats(s: &Screen, map_w: i32) -> (Rect, Rect) {
    if !s.landscape() && !s.desktop {
        let p = portrait_seats(s);
        return (p.save, p.map);
    }
    // The PC's portrait window (2026-10-29): the top band is the game's own target panel, which
    // spans most of a 240-wide canvas, so the two tabs sit over the picture's bottom-right corner,
    // just above the quick bar — a cell wide, three fifths of that tall.
    if s.desktop && !s.landscape() {
        let cells = quick_cells(s);
        let (w, pad) = (cells[0].w.max(1), s.pad());
        let h = (w * 3 / 5).max(1);
        let y = (cells[0].y - pad - h).max(0);
        let save = Rect::new((s.game.right() - pad - w).max(0), y, w, h);
        let map = Rect::new((save.x - w - w / 2).max(0), y, w, h);
        return (save, map);
    }
    // The PC's landscape window: a quick cell wide (its ring plate is sized for a thumb).
    let plate = if s.desktop { quick_cells(s)[0].w.max(1) } else { plate_seat(s, 0).w.max(1) };
    let (left, right, _) = tab_band(s, map_w);
    let w = plate.min((right - left) / 3).max(1);
    let h = (w * 3 / 5).max(1);
    let save = Rect::new((right - w / 4 - w).max(0), 0, w, h);
    let map = Rect::new((save.x - w - w / 2).max(0), 0, w, h);
    (save, map)
}

/// The tabs' band over a map `map_w` wide, `(left, right, clears_target)`: `clears_target` = it
/// stops short of the target panel over a map as wide as the canvas (there was room for two tabs
/// at three fifths of a plate), else it runs to the minimap. Over a narrower map it stops short of
/// the panel where that one is, when two tabs still fit there; else it keeps the canvas's band.
fn tab_band(s: &Screen, map_w: i32) -> (i32, i32, bool) {
    let plate = plate_seat(s, 0).w.max(1);
    let pad = s.pad();
    let left = status_box(s).right() + pad * 2;
    let panel = minimap_panel(s);
    let short = |w| target_box(s, target_dodge(s, panel, w), w).x.min(panel.x) - pad * 2;
    let fits = |right: i32| right - left >= plate * 3 / 5 * 3;
    let wide = short(s.guest_w);
    if !fits(wide) {
        return (left, panel.x - pad * 2, false);
    }
    let here = short(map_w);
    (left, if fits(here) { here.min(wide) } else { wide }, true)
}

pub fn save_tab(s: &Screen, map_w: i32) -> Rect {
    top_tab_seats(s, map_w).0
}

pub fn map_tab(s: &Screen, map_w: i32) -> Rect {
    top_tab_seats(s, map_w).1
}

/// What a tab drawn at `r` answers to (hero2's `top_tab_hit`, the player's *"클릭영역은 실제버튼보다
/// 더 커야해"*): a square one and a half tabs wide, centred on it and hanging from the same edge, so
/// two tabs half a tab apart meet in the middle of the gap.
pub fn top_tab_hit(s: &Screen, r: Rect) -> Rect {
    if r.is_empty() {
        return r;
    }
    // Portrait's hang in the strip between the controls: a pad more each way, as a control's slack.
    if !s.landscape() && !s.desktop {
        return r.grown(s.pad()).clipped(&s.window());
    }
    let side = r.w / 4;
    let x0 = (r.x - side).max(0);
    let x1 = (r.right() + side).min(s.w).max(x0);
    let y1 = (r.y + r.h.max(r.w * 3 / 2)).min(s.h).max(r.y);
    Rect::new(x0, r.y, x1 - x0, y1 - r.y)
}

/// The tabs for [`Layout::tabs`]: the bare field only (a window or a message box owns the keys
/// there, and the game's quick save is a field action), MAP once the world map exists; over a map
/// `map_w` wide ([`top_tab_seats`]).
pub fn top_tabs(s: &Screen, view: GuestView, world_map: bool, map_w: i32) -> Vec<(Rect, Tab)> {
    if view.scene != Scene::Field || view.popup || view.notice {
        return Vec::new();
    }
    let (save, map) = top_tab_seats(s, map_w);
    let mut out = vec![(save, Tab::Save)];
    if world_map {
        out.push((map, Tab::Map));
    }
    out
}

/// What the layout editor's top row does — hero2's row, in hero2's order (A9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    /// The pad badges each control carries in play, on its box (a preview: nothing changes).
    Pad,
    /// The whole layout mirrored left ↔ right.
    Flip,
    /// The selected quick plate switched off ([`placement::Table::hide`]).
    Hide,
    /// Every plate switched off back on.
    Show,
    /// Every control back to its built-in seat.
    Reset,
    /// Leave, the layout as it was when the editor opened.
    Cancel,
    Save,
}

/// The row, left to right: PAD first — it changes nothing, so the far end from SAVE is where a
/// stray tap costs least; HIDE / SHOW after FLIP, so RESET / CANCEL / SAVE stay at the right end
/// where the thumb looks for them (hero2's `edit_items`).
pub const EDIT_ACTIONS: [EditAction; 7] = [
    EditAction::Pad,
    EditAction::Flip,
    EditAction::Hide,
    EditAction::Show,
    EditAction::Reset,
    EditAction::Cancel,
    EditAction::Save,
];

impl EditAction {
    /// The button's word: a `crate::host_text` line (drawn in Korean in the game's font).
    pub fn label(self) -> &'static str {
        match self {
            EditAction::Pad => "PAD",
            EditAction::Flip => "FLIP",
            EditAction::Hide => "HIDE",
            EditAction::Show => "SHOW",
            EditAction::Reset => "RESET",
            EditAction::Cancel => "CANCEL",
            EditAction::Save => "SAVE",
        }
    }
}

/// The layout editor's buttons: one row at the top centre (hero2's third try — a row nobody's
/// thumb crosses on the way to a control). Obstacles at all times, so a control can never be
/// buried under one (it could not be picked up again, and the tap meant for it would act).
///
/// Seven cells inside the footprint the old row of four had, to the pixel (hero2 did the same when
/// its PAD joined: the row is an obstacle, so a wider one would silently send every stored
/// placement that cleared the old ends back to its built-in seat). The cells pay with narrower
/// faces and half the gaps.
pub fn editor_chrome(s: &Screen) -> Vec<(Rect, EditAction)> {
    let bh = (s.h / 16).clamp(28, 72);
    let span = 4 * (bh * 5 / 2) + 3 * s.pad();
    let x0 = s.w / 2 - span / 2;
    // Portrait: across the middle of the picture, dimmed while the strip under it is arranged — its
    // top, where landscape's sits, is the status block and the minimap (which is placed too), and
    // its bottom is where the notices stack up from the EXP bar.
    let y = if s.landscape() { s.pad() } else { s.game.y + s.game.h / 2 - bh / 2 };
    let n = EDIT_ACTIONS.len() as i32;
    let gap = (s.pad() / 2).max(2);
    let bw = ((span - gap * (n - 1)) / n).max(1);
    // The last cell takes the rounding, so the row ends where the old one did.
    EDIT_ACTIONS
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let x = x0 + i as i32 * (bw + gap);
            let w = if i as i32 == n - 1 { x0 + span - x } else { bw };
            (Rect::new(x, y, w, bh), *a)
        })
        .collect()
}

/// The settings window's rows (B2, hero2's window — S9 2026-10-26 had a full-height column of
/// nine upper-case rows, the player's 「가운데 setting 화면도 다르고」). One line each: a name on
/// the left, its state on the right ([`SettingsAction::name`], `touch::Hud::settings_words`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Open the layout editor (both orientations since 2026-10-29, each its own layout).
    LayoutEdit,
    /// Open the platform's 진단 정보 dialog (B3, hero2's): what the host knows about this install,
    /// 복사, and under 세이브 관리 the vault's restore points and the export (`crate::saves`).
    Diag,
    /// Toggle HEAT SAVER (`crate::config`): half-resolution host surface, applies at the next start.
    HeatSaver,
    /// Cycle the effects' level (`crate::audio_mix::SFX_STEPS_DB`), heard from the next effect.
    SfxLevel,
    /// Toggle the status block — a row only while the game's own bar is drawn (HZ-013 off, a debug
    /// boot): without the bar the block is the only HP on screen and stays on.
    Stats,
    /// Cycle the PAD BADGES preference (`crate::padhud::PadHud`): AUTO → ON → OFF.
    PadHud,
    /// Open the PAD REMAP panel (Phase 5-2): bind pad buttons to keys.
    PadRemap,
    /// The PC: the window full screen (borderless) or windowed again; F11 does the same.
    Fullscreen,
    /// 닫기: the plate at the title's right, and a tap outside the window.
    Close,
}

impl SettingsAction {
    /// The row's name: a `crate::host_text` line, drawn in Korean in the game's font (B1).
    pub fn name(self) -> &'static str {
        match self {
            SettingsAction::LayoutEdit => "LAYOUT",
            SettingsAction::Diag => "DIAG",
            SettingsAction::HeatSaver => "HEAT SAVER",
            SettingsAction::SfxLevel => "SFX LEVEL",
            SettingsAction::Stats => "STATS",
            SettingsAction::PadHud => "PAD BADGES",
            SettingsAction::PadRemap => "PAD REMAP",
            SettingsAction::Fullscreen => "FULLSCREEN",
            SettingsAction::Close => "CLOSE",
        }
    }
}

/// How a settings row's state reads (hero2's `RowState`) — in INK, as the game marks its own rows
/// (its cursor row's letters are white where the rest are not; it has no second plate colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// At its default.
    Normal,
    /// The player has changed it: the window's standing job is to answer "have I changed
    /// anything?" at a glance.
    Active,
    /// Cannot act here; the state says why.
    Disabled,
}

/// The rows the window shows at once (hero2's five, 2026-09-18); every row past them scrolls.
pub const SETTINGS_VISIBLE_ROWS: i32 = 5;

/// hero2's settings window: 74 % × 72 % of the window, centred (never under 360 × 340 where the
/// window has that).
pub fn settings_panel(s: &Screen) -> Rect {
    let pw = (s.w * 74 / 100).clamp(360.min(s.w), s.w.max(1));
    let ph = (s.h * 72 / 100).clamp(340.min(s.h), s.h.max(1));
    Rect::new((s.w - pw) / 2, (s.h - ph) / 2, pw, ph)
}

/// The side of the square 닫기 plate.
fn settings_close_side(p: Rect) -> i32 {
    (p.w / 8).clamp(56, 110)
}

/// The header strip — the title and 닫기 — RESERVED, so the title can never be sized onto row 1.
fn settings_head(p: Rect) -> i32 {
    (p.h / 30 + settings_close_side(p) + p.h / 40).max(p.h * 18 / 100)
}

/// 닫기: a plate at the header's right — hero2's square, widened by half again for the game's font,
/// whose two syllables at the rows' size filled the square edge to edge (preview, 2026-10-26).
pub fn settings_close(s: &Screen) -> Rect {
    let p = settings_panel(s);
    let side = settings_close_side(p);
    let w = side * 8 / 5;
    Rect::new(p.right() - w - p.w / 30, p.y + p.h / 30, w, side)
}

/// The list's fixed geometry (hero2's `SettingsGeom`): its window under the header (`view`), the
/// pitch of FIVE rows whatever the list holds — so the plates never shrink as rows are added —
/// the plate inside it (70 %), and the plates' left edge and width (78 % of the window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsGeom {
    pub panel: Rect,
    pub view: Rect,
    pub pitch: i32,
    pub plate: i32,
    pub x0: i32,
    pub bw: i32,
}

pub fn settings_geom(s: &Screen) -> SettingsGeom {
    let p = settings_panel(s);
    let head = settings_head(p);
    let rows_h = (p.h - head - p.h / 24).max(3);
    let pitch = (rows_h / SETTINGS_VISIBLE_ROWS).max(1);
    let bw = p.w * 78 / 100;
    SettingsGeom {
        panel: p,
        view: Rect::new(p.x, p.y + head, p.w, pitch * SETTINGS_VISIBLE_ROWS),
        pitch,
        plate: (pitch * 70 / 100).max(1),
        x0: p.x + (p.w - bw) / 2,
        bw,
    }
}

/// How far a list of `n` rows scrolls, in window px (0 while they fit).
pub fn settings_scroll_max(s: &Screen, n: usize) -> i32 {
    (n as i32 - SETTINGS_VISIBLE_ROWS).max(0) * settings_geom(s).pitch
}

/// The plates of `rows` (`(action, can act here)`) with the list scrolled `scroll` px: those in the
/// list's window, one its edge cuts included whole — the drawing clips it, and [`settings_hit`]
/// answers only inside the window.
pub fn settings_rows(s: &Screen, rows: &[(SettingsAction, bool)], scroll: i32) -> Vec<(Rect, SettingsAction, bool)> {
    let g = settings_geom(s);
    let scroll = scroll.clamp(0, settings_scroll_max(s, rows.len()));
    rows.iter()
        .enumerate()
        .filter_map(|(i, &(a, on))| {
            let r = Rect::new(g.x0, g.view.y + i as i32 * g.pitch + (g.pitch - g.plate) / 2 - scroll, g.bw, g.plate);
            r.intersects(&g.view).then_some((r, a, on))
        })
        .collect()
}

/// What a tap at (`x`, `y`) does (hero2's `settings_hit`): 닫기, and a tap outside the window,
/// close it; a row acts — one that cannot act here swallows the tap (`None`: falling through would
/// close the window); anywhere else on the window, nothing.
pub fn settings_hit(s: &Screen, rows: &[(SettingsAction, bool)], scroll: i32, x: f64, y: f64) -> Option<SettingsAction> {
    if settings_close(s).contains(x, y) {
        return Some(SettingsAction::Close);
    }
    if settings_geom(s).view.contains(x, y)
        && let Some((_, a, on)) = settings_rows(s, rows, scroll).into_iter().find(|(r, _, _)| r.contains(x, y))
    {
        return on.then_some(a);
    }
    (!settings_panel(s).contains(x, y)).then_some(SettingsAction::Close)
}

// ------------------------------------------------------- the host windows (the PC's) ----

/// The rows a host window (`crate::hostwin`) shows at once: text lines are shorter than the
/// settings' plates, and a diagnosis is a dozen of them.
pub const HOST_VISIBLE_ROWS: i32 = 9;

/// A host window's geometry: the settings window's panel and header, [`HOST_VISIBLE_ROWS`] rows under
/// it, the rows 86 % of the panel wide (a path or a sentence needs the room).
pub fn host_geom(s: &Screen) -> SettingsGeom {
    let p = settings_panel(s);
    let head = settings_head(p);
    let rows_h = (p.h - head - p.h / 24).max(3);
    let pitch = (rows_h / HOST_VISIBLE_ROWS).max(1);
    let bw = p.w * 86 / 100;
    SettingsGeom {
        panel: p,
        view: Rect::new(p.x, p.y + head, p.w, pitch * HOST_VISIBLE_ROWS),
        pitch,
        plate: (pitch * 80 / 100).max(1),
        x0: p.x + (p.w - bw) / 2,
        bw,
    }
}

/// How far `n` host rows scroll, in window px.
pub fn host_scroll_max(s: &Screen, n: usize) -> i32 {
    (n as i32 - HOST_VISIBLE_ROWS).max(0) * host_geom(s).pitch
}

/// The rows of a window of `n` scrolled `scroll` px that reach into the list's window, with their
/// plates (a text row is drawn in the same box, without the plate).
pub fn host_rows(s: &Screen, n: usize, scroll: i32) -> Vec<(usize, Rect)> {
    let g = host_geom(s);
    let scroll = scroll.clamp(0, host_scroll_max(s, n));
    (0..n)
        .filter_map(|i| {
            let r = Rect::new(g.x0, g.view.y + i as i32 * g.pitch + (g.pitch - g.plate) / 2 - scroll, g.bw, g.plate);
            r.intersects(&g.view).then_some((i, r))
        })
        .collect()
}

/// The scroll that shows row `i` whole, moving as little as possible from `scroll`.
pub fn host_scroll_to(s: &Screen, n: usize, scroll: i32, i: usize) -> i32 {
    let g = host_geom(s);
    let top = i as i32 * g.pitch;
    let bottom = top + g.pitch;
    let max = host_scroll_max(s, n);
    if top < scroll {
        top.clamp(0, max)
    } else if bottom > scroll + g.view.h {
        (bottom - g.view.h).clamp(0, max)
    } else {
        scroll.clamp(0, max)
    }
}

/// What a press on a host window lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostHit {
    /// 닫기, or outside the window.
    Close,
    /// A row (text or plate) in the list's window.
    Row(usize),
    /// The window's own body (the header, the margins): nothing.
    Panel,
}

pub fn host_hit(s: &Screen, n: usize, scroll: i32, x: f64, y: f64) -> HostHit {
    if settings_close(s).contains(x, y) {
        return HostHit::Close;
    }
    if host_geom(s).view.contains(x, y)
        && let Some((i, _)) = host_rows(s, n, scroll).into_iter().find(|(_, r)| r.contains(x, y))
    {
        return HostHit::Row(i);
    }
    if settings_panel(s).contains(x, y) { HostHit::Panel } else { HostHit::Close }
}

// ------------------------------------------------------------- the pad screen (B6) ----

/// One guest pixel in window px, whole — the game's windows draw their lines this thick.
pub fn guest_unit(s: &Screen) -> i32 {
    ((f64::from(s.game.h) / f64::from(s.guest_h.max(1))).round() as i32).max(1)
}

/// How a target of the pad screen is drawn and lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetShape {
    /// A disc control: OK, CLR, a quick plate.
    Round,
    /// A flat one: a tab.
    Square,
    /// No control on screen: a cell in the window ([`RemapPanel::chips`]).
    Chip,
}

/// One thing the pad screen offers a button for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemapTarget {
    /// Index into `crate::gamepad::REMAP_ROWS`.
    pub row: usize,
    pub rect: Rect,
    pub shape: TargetShape,
}

/// The pad screen's window (hero2's `PadMapPanel`), window px — this game's window, as the settings
/// window is (B2): its edge and parchment, the title with 취소 at its left end and 저장 at its right,
/// a plate for the two status lines, a cell for every function with no control on screen, 기본값.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemapPanel {
    pub panel: Rect,
    /// Inside the edge: the parchment.
    pub body: Rect,
    /// The title's band, between 취소 and 저장.
    pub title: Rect,
    pub cancel: Rect,
    pub save: Rect,
    pub status: Rect,
    pub chips: Vec<Rect>,
    pub reset: Rect,
    /// One guest pixel ([`guest_unit`]).
    pub unit: i32,
}

/// Everything the pad screen draws and answers to for one window (hero2's `PadMapScene`): the
/// FIELD set as the player plays it (`table` places it) less what a pad cannot be given — the walk,
/// the gear, the tabs, the minimap — the targets on it, and the window.
#[derive(Clone, Debug, PartialEq)]
pub struct RemapScene {
    pub layout: Layout,
    /// The controls on screen first, then the window's cells.
    pub targets: Vec<RemapTarget>,
    pub panel: RemapPanel,
}

/// What a touch on the pad screen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemapHit {
    Cancel,
    Reset,
    Save,
    /// A control on screen, or a cell: the `crate::gamepad::REMAP_ROWS` index it offers.
    Row(usize),
    /// The window, but nothing on it.
    Panel,
}

/// The pad screen for this window (hero2's `padmap_scene`): the field's controls a pad button can be
/// given, in their seats — a plate the player switched off is not one — the two top tabs where they
/// hang (hero2 offers its SAVE tab so), and a cell in the window for every function left without one
/// (here `back` and `*`, which have no field control).
pub fn remap_scene(s: &Screen, table: &Table) -> RemapScene {
    use crate::gamepad::{PadAction, remap_row, tab_action};
    let field = GuestView { scene: Scene::Field, ..Default::default() };
    let mut layout = layout_for(*s, table, field, Walk::Pad);
    layout.controls.retain(|c| c.face != Face::Hidden);
    (layout.gear, layout.minimap, layout.status) = (None, None, None);
    layout.tabs = top_tabs(s, field, true, s.guest_w);
    let mut targets: Vec<RemapTarget> = Vec::new();
    for c in &layout.controls {
        if let Some(k) = c.action.pad_key()
            && let Some(row) = remap_row(PadAction::Key(k))
            && !targets.iter().any(|t| t.row == row)
        {
            let shape = if matches!(c.shape, Shape::Circle { .. }) { TargetShape::Round } else { TargetShape::Square };
            targets.push(RemapTarget { row, rect: c.shape.bounds(), shape });
        }
    }
    for (r, tab) in &layout.tabs {
        if let Some(row) = remap_row(tab_action(*tab)) {
            targets.push(RemapTarget { row, rect: *r, shape: TargetShape::Square });
        }
    }
    let missing: Vec<usize> = (0..crate::gamepad::REMAP_ROWS.len()).filter(|r| !targets.iter().any(|t| t.row == *r)).collect();
    let avoid: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
    let panel = remap_panel(s, missing.len(), &avoid);
    for (row, rect) in missing.iter().zip(&panel.chips) {
        targets.push(RemapTarget { row: *row, rect: *rect, shape: TargetShape::Chip });
    }
    RemapScene { layout, targets, panel }
}

/// The window (hero2's `padmap_panel`), sized in guest px: centred across the screen, as high as it
/// can sit without covering a control — the first free spot from the top — or in the middle when
/// every spot covers one. Two cells a row.
fn remap_panel(s: &Screen, chips: usize, avoid: &[Rect]) -> RemapPanel {
    let u = guest_unit(s);
    let w = if s.landscape() { (s.w * 46 / 100).clamp(360.min(s.w), s.w) } else { s.w - 4 * s.pad() };
    let (edge, pad, gap) = (2 * u, 3 * u, 2 * u);
    let (title_h, line, chip_h) = (16 * u, 13 * u, 15 * u);
    let status_h = 2 * line + 2 * u;
    let rows = chips.div_ceil(2) as i32;
    let h = (2 * edge + 2 * pad + title_h + gap + status_h + gap + rows * (chip_h + gap) + chip_h).min(s.h);
    // Centred first; then at the left edge, where the walk control (not on this screen) sits — a
    // 4:3 window's ring reaches under the centred one.
    let free = |x: i32, y: i32| !avoid.iter().any(|a| a.intersects(&Rect::new(x, y, w, h).grown(2 * u)));
    let spot = [(s.w - w) / 2, (2 * u).min(s.w - w)]
        .into_iter()
        .find_map(|x| (0..).map(|k| 2 * u + k * u).take_while(|y| y + h + 2 * u <= s.h).find(|y| free(x, *y)).map(|y| (x, y)));
    let (x0, y0) = spot.unwrap_or(((s.w - w) / 2, (s.h - h) / 2));
    let panel = Rect::new(x0, y0, w, h);
    let body = panel.grown(-edge);
    let inner = Rect::new(body.x + pad, body.y + pad, body.w - 2 * pad, body.h - 2 * pad);
    let plate_w = (inner.w / 5).max(title_h * 2);
    let cancel = Rect::new(inner.x, inner.y + u, plate_w, title_h - 2 * u);
    let save = Rect::new(inner.right() - plate_w, inner.y + u, plate_w, title_h - 2 * u);
    let title = Rect::new(cancel.right(), inner.y, save.x - cancel.right(), title_h);
    let status = Rect::new(inner.x, inner.y + title_h + gap, inner.w, status_h);
    let cw = (inner.w - gap) / 2;
    let chips_y = status.bottom() + gap;
    let chips = (0..chips as i32).map(|i| Rect::new(inner.x + (i % 2) * (cw + gap), chips_y + (i / 2) * (chip_h + gap), cw, chip_h)).collect();
    let reset_w = cw * 2 / 3;
    let reset = Rect::new(inner.right() - reset_w, chips_y + rows * (chip_h + gap), reset_w, chip_h);
    RemapPanel { panel, body, title, cancel, save, status, chips, reset, unit: u }
}

/// What a touch at (`x`, `y`) is on the pad screen (hero2's `padmap_hit`); `None` = the picture.
pub fn remap_hit(sc: &RemapScene, x: f64, y: f64) -> Option<RemapHit> {
    let p = &sc.panel;
    // The window is drawn over whatever it covers, so it answers first.
    if p.panel.contains(x, y) {
        let hit = if p.cancel.contains(x, y) {
            RemapHit::Cancel
        } else if p.save.contains(x, y) {
            RemapHit::Save
        } else if p.reset.contains(x, y) {
            RemapHit::Reset
        } else {
            sc.targets.iter().find(|t| t.shape == TargetShape::Chip && t.rect.contains(x, y)).map_or(RemapHit::Panel, |t| RemapHit::Row(t.row))
        };
        return Some(hit);
    }
    // Controls may overlap each other; the smallest one under the finger is the one a thumb can
    // only have meant on purpose.
    sc.targets
        .iter()
        .filter(|t| t.shape != TargetShape::Chip && t.rect.contains(x, y))
        .min_by_key(|t| i64::from(t.rect.w) * i64::from(t.rect.h))
        .map(|t| RemapHit::Row(t.row))
}

/// Everything a control may NOT be placed on — each wins a touch the control would otherwise get,
/// or paints over it: the gear (reserved in the field too, where landscape shows none, so opening a
/// window never moves a control), the editor's own buttons and portrait's tabs.
pub fn obstacles(s: &Screen) -> Vec<Rect> {
    let mut out = vec![gear(s)];
    out.extend(editor_chrome(s).into_iter().map(|(r, _)| r));
    // Portrait's tabs stand in the strip, among the controls, whatever the scene (like the gear).
    if !s.landscape() {
        let p = portrait_seats(s);
        out.extend([top_tab_hit(s, p.save), top_tab_hit(s, p.map)]);
    }
    out.retain(|r| !r.is_empty());
    out
}

// ------------------------------------------------------------- host notices (B5) ----

/// A host notice's box, guest px, round a lettered line `w` × `h`: the game's own pickup notice
/// (`g.a(BBBZ)V`, 120 × 16 — a black box, a cream line a px in, the item's name white in the
/// middle; probe 2026-10-26) grown to the line — a px of black, the cream line and a px of black
/// again either side of the letters, and never narrower than the game's 120.
pub fn notice_box_gp(w: i32, h: i32) -> (i32, i32) {
    ((w + 6).max(120), h + 4)
}

/// Where the host's notices stack (B5, hero2's `draw_notices`): at the picture's left edge a guest
/// px in, the newest lowest — a guest px over the EXP bar, or over the walk control on the left
/// (the field's stick seat, a window's arrows) when a stack `box_w` wide would cross it — and
/// nothing above `ceiling` (the status block's bottom while it is drawn). The newest box's top
/// left, and how many boxes fit (the newest always does).
pub fn notice_stack(l: &Layout, box_w: i32, box_h: i32, gap: i32, ceiling: i32) -> (i32, i32, usize) {
    let s = &l.screen;
    let x = s.game.x + gap;
    let mut bottom = exp_bar(s).y.min(s.game.bottom()) - gap;
    if let Some(a) = l.stick.map(|st| st.seat).or(l.dpad)
        && x < a.right()
        && a.x < x + box_w
        && bottom > a.y
    {
        bottom = a.y - gap;
    }
    let top = bottom - box_h;
    let fit = if top >= ceiling + gap { ((top - ceiling - gap) / (box_h + gap).max(1)) as usize + 1 } else { 1 };
    (x, top, fit)
}

/// The host's EXP bar at the bottom centre (`hud_view::HvExp`), screen px.
pub fn exp_bar(s: &Screen) -> Rect {
    let g = s.game;
    let e = crate::hud_view::HvExp::new((g.x.max(0) as u32, g.y.max(0) as u32, g.w.max(0) as u32, g.h.max(0) as u32), s.guest_w as u32, s.guest_h as u32);
    let (x, y, w, h) = e.rect();
    Rect::new(x as i32, y as i32, w as i32, h as i32)
}

/// What the 창고 question's answers are (the user's choice ②, 2026-10-27): the window that answers
/// only the soft keys ([`GuestView::lsk_ok`] — 「저장 후 창고로 접속하시겠습니까?」) lost its □ (예)
/// and ↩ (아니오) with HZ-024, and had nothing else that said so.
pub const ANSWER_HINT: &str = "OK - YES   CLR - NO";

/// The first guest row under the game's windows, counted up from the canvas bottom: they stop
/// where its bar began (`ce.h = H − 33`) — the frames of `bh` and `de` end on row H − 32 (probe
/// 2026-10-27, 520×240: rows 207 and 208 black).
const WINDOW_FLOOR_GP: i32 = 31;

/// Where [`ANSWER_HINT`]'s box (`w × h` screen px) goes: centred on the picture, in the band under
/// the game's windows that its bar kept — where the soft-key pictures were — and a guest px over
/// the EXP bar: the band's middle when it fits, else on the bar.
pub fn answer_hint(s: &Screen, w: i32, h: i32) -> Rect {
    let gap = (s.game.h / s.guest_h).max(1);
    let top = s.gy(s.guest_h - WINDOW_FLOOR_GP);
    let floor = exp_bar(s).y.min(s.game.bottom()) - gap;
    let y = if floor - top >= h { top + (floor - top - h) / 2 } else { floor - h };
    Rect::new(s.game.centre().0 - w / 2, y, w, h)
}

// ---------------------------------------------------------------------- portrait ----

/// The control strip under the picture, and its unit (1 % of its short side).
fn strip(s: &Screen) -> (Rect, i32) {
    let strip = Rect::new(0, s.game.bottom(), s.w, s.h - s.game.bottom());
    (strip, (strip.w.min(strip.h) / 100).max(1))
}

/// Portrait's built-in seats (user 2026-10-29: *"세로버전에서 map이랑 save버튼을 아래 버튼 영역에
/// 넣을꺼고 … 퀵슬롯도 아래쪽영역에 따로 표시할꺼야(키패드가 아니라 퀵슬롯이 표시되게) … 현재형태에서 가운데
/// 키패드없고 퀵슬롯나오는 형태에서 가로처럼 커스텀"*), all in the strip, in its unit: the walk square
/// bottom-left where the D-pad was, OK bottom-right with CLR over it toward the edge (landscape's
/// pair), the six quick plates between them where the handset keypad was — two across, three down,
/// 1 3 / 7 9 / 0 # where the keypad had them ([`PORTRAIT_GRID`]) — MAP and SAVE hanging from the
/// strip's top edge in its corners, where L and R were, and the gear between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortraitSeats {
    pub stick: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    /// In [`RING_PLATES`] order.
    pub plates: [Rect; RING_PLATES.len()],
    /// HZ-058: the page button, the grid's eighth cell.
    pub page: Rect,
    pub save: Rect,
    pub map: Rect,
    pub gear: Rect,
}

/// Each [`RING_PLATES`] plate's cell in portrait's grid, `(column, row)`: the first six where the
/// keypad had 9 7 3 1 0 # (the game's six, where a new player's plates start), the seventh under
/// them; the page button beside it ([`PORTRAIT_PAGE`]).
pub const PORTRAIT_GRID: [(i32, i32); RING_PLATES.len()] = [(1, 1), (0, 1), (1, 0), (0, 0), (0, 2), (1, 2), (0, 3)];
pub const PORTRAIT_PAGE: (i32, i32) = (1, 3);

pub fn portrait_seats(s: &Screen) -> PortraitSeats {
    let (strip, u) = strip(s);
    let gear_side = u * 10;
    let gear = Rect::new(s.w / 2 - gear_side / 2, strip.y + u * 3, gear_side, gear_side);
    // A tab three fifths as tall as it is wide, as landscape's.
    let (tw, th) = (u * 20, u * 12);
    let map = Rect::new(strip.x + u * 3, strip.y, tw, th);
    let save = Rect::new(strip.right() - u * 3 - tw, strip.y, tw, th);
    let (top, bottom) = (strip.y + u * 16, strip.bottom() - u * 3);
    let side = (u * 46).min(bottom - top).min(strip.w * 2 / 5).max(1);
    let stick = Rect::new(strip.x + u * 3, bottom - side, side, side);
    let ok_d = u * 28;
    let ok = Rect::new(strip.right() - u * 5 - ok_d, bottom - u * 2 - ok_d, ok_d, ok_d);
    let clr_d = u * 18;
    let cancel = Rect::new(strip.right() - u * 4 - clr_d, ok.y - u * 3 - clr_d, clr_d, clr_d);
    let gap = u * 3;
    let (left, right) = (stick.right() + gap, ok.x.min(cancel.x) - gap);
    let p = (u * 14).min((right - left - gap) / 2).min((bottom - top - gap * 3) / 4).max(1);
    let (gw, gh) = (p * 2 + gap, p * 4 + gap * 3);
    let (gx, gy) = (left + (right - left - gw) / 2, top + (bottom - top - gh) / 2);
    let cell = |(c, r): (i32, i32)| Rect::new(gx + c * (p + gap), gy + r * (p + gap), p, p);
    let plates = PORTRAIT_GRID.map(cell);
    PortraitSeats { stick, ok, cancel, plates, page: cell(PORTRAIT_PAGE), save, map, gear }
}

// --------------------------------------------------------------------- the D-pad ----

/// An arrow's hexagon, along its axis, as a percent of the square's side (hero2's 91차-B "fat
/// hexagon": [`ARM_LEN_PCT`] long, [`ARM_WIDTH_PCT`] wide, tips [`HEX_TIP_PCT`] of its length).
pub const ARM_LEN_PCT: i32 = 34;
pub const ARM_WIDTH_PCT: i32 = 44;
pub const HEX_TIP_PCT: i32 = 30;
/// The dead disc at the square's centre that belongs to no arrow, percent of the side.
pub const DEAD_PCT: i32 = 8;

/// The four arrows a window gets (hero2's `dpad_arms`), cut from the walk control's square `r`:
/// each arrow's INK (its hexagon's box, the tip outward, `inset` in from the rim) and its key, in
/// draw / hit order up, down, left, right. A touch counts anywhere in the arrow's diagonal quarter
/// of the square ([`Shape::Wedge`]) — the quarters tile it, so sliding a thumb from one arrow to
/// the next crosses a line, not a dead strip.
pub fn dpad_arms(r: Rect) -> [(Rect, KeyCode); 4] {
    let s = r.w.min(r.h);
    let inset = (s / 50).max(1);
    let len = (s * ARM_LEN_PCT / 100).max(3);
    let wid = (s * ARM_WIDTH_PCT / 100).max(3);
    let (cx, cy) = r.centre();
    [
        (Rect::new(cx - wid / 2, r.y + inset, wid, len), KeyCode::UP),
        (Rect::new(cx - wid / 2, r.bottom() - inset - len, wid, len), KeyCode::DOWN),
        (Rect::new(r.x + inset, cy - wid / 2, len, wid), KeyCode::LEFT),
        (Rect::new(r.right() - inset - len, cy - wid / 2, len, wid), KeyCode::RIGHT),
    ]
}

/// The six corners of an arrow's hexagon in its box `r`, clockwise from the outward tip.
pub fn hexagon(r: Rect, key: KeyCode) -> [(f64, f64); 6] {
    let (x0, y0) = (f64::from(r.x), f64::from(r.y));
    let (x1, y1) = (x0 + f64::from(r.w), y0 + f64::from(r.h));
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let tip = f64::from(HEX_TIP_PCT) / 100.0;
    if matches!(key, KeyCode::UP | KeyCode::DOWN) {
        let t = (y1 - y0) * tip;
        [(cx, y0), (x1, y0 + t), (x1, y1 - t), (cx, y1), (x0, y1 - t), (x0, y0 + t)]
    } else {
        let t = (x1 - x0) * tip;
        [(x1, cy), (x1 - t, y1), (x0 + t, y1), (x0, cy), (x0 + t, y0), (x1 - t, y0)]
    }
}

/// The four arrows as controls: their keys, their quarters, their hexagons.
pub fn dpad_controls(square: Rect) -> Vec<Control> {
    dpad_arms(square)
        .into_iter()
        .map(|(ink, key)| Control { face: Face::Hex(ink), ..Control::new(None, Shape::Wedge { square, key }, Action::Key(key), "") })
        .collect()
}

// ------------------------------------------------------------------------ layout ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle { cx: i32, cy: i32, r: i32 },
    Box(Rect),
    /// An arrow's quarter of the D-pad's square (hero2's `Wedge`): the points nearer its axis than
    /// the others' (a diagonal goes to UP / DOWN), less a small dead disc at the centre.
    Wedge { square: Rect, key: KeyCode },
}

impl Shape {
    pub fn bounds(&self) -> Rect {
        match *self {
            Shape::Circle { cx, cy, r } => Rect::new(cx - r, cy - r, 2 * r, 2 * r),
            Shape::Box(b) | Shape::Wedge { square: b, .. } => b,
        }
    }

    /// Inside, widened by `slack` px (fingers are not pixels) — but a wedge is exact: the four
    /// tile their square, and a slack would make neighbours overlap.
    pub fn contains(&self, x: f64, y: f64, slack: f64) -> bool {
        match *self {
            Shape::Circle { cx, cy, r } => {
                let (dx, dy) = (x - f64::from(cx), y - f64::from(cy));
                dx * dx + dy * dy <= (f64::from(r) + slack).powi(2)
            }
            Shape::Box(b) => b.grown(slack as i32).contains(x, y),
            Shape::Wedge { square, key } => {
                if !square.contains(x, y) {
                    return false;
                }
                let (cx, cy) = (f64::from(square.x) + f64::from(square.w) / 2.0, f64::from(square.y) + f64::from(square.h) / 2.0);
                let (dx, dy) = (x - cx, y - cy);
                let dead = f64::from(square.w.min(square.h) * DEAD_PCT) / 100.0;
                if dx * dx + dy * dy < dead * dead {
                    return false;
                }
                match key {
                    KeyCode::UP => -dy >= dx.abs(),
                    KeyCode::DOWN => dy >= dx.abs(),
                    KeyCode::LEFT => -dx > dy.abs(),
                    _ => dx > dy.abs(),
                }
            }
        }
    }
}

/// What a control does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Key(KeyCode),
    /// HZ-058: plate `n` (0-based) of the page shown — used in the field, chosen after a window's
    /// 「퀵슬롯등록」 (`hz_java::quickpage`).
    Plate(u8),
    /// HZ-058: the plates' other page.
    Page,
}

impl Action {
    /// The game key it presses, if it is one.
    pub fn key(self) -> Option<KeyCode> {
        match self {
            Action::Key(k) => Some(k),
            Action::Plate(_) | Action::Page => None,
        }
    }

    /// The pad function that presses it (its badge, the pad screen's target): a key's own; the plates
    /// the pad's quick functions ([`PLATE_PAD_KEYS`]), the page its [`PAGE_PAD_KEY`].
    pub fn pad_key(self) -> Option<KeyCode> {
        match self {
            Action::Key(k) => Some(k),
            Action::Plate(n) => PLATE_PAD_KEYS.get(usize::from(n)).copied(),
            Action::Page => Some(PAGE_PAD_KEY),
        }
    }
}

/// HZ-058: the pad's quick functions (`gamepad::REMAP_ROWS` 「QUICK SLOT 1 3 7 9 0 # *」) press plates
/// 1..7 in the field — the plates a player who never set any starts with holding the game's six slots
/// (`hz_java::quickpage::Plates::adopt`), so a pad does what it did; the seventh is R3's `*` (user
/// 2026-10-30: its game minimap window is the MAP tab's too). Over the game's windows they are the keys
/// they always were.
pub const PLATE_PAD_KEYS: [KeyCode; QUICK_PLATES] =
    [KeyCode::NUM1, KeyCode::NUM3, KeyCode::NUM7, KeyCode::NUM9, KeyCode::NUM0, KeyCode::HASH, KeyCode::STAR];

/// The pad function that turns the plates' page where the plates are used: START's LSK (user
/// 2026-10-30, 「start로 하자」 — the field's menu is B's RSK and the CLR disc's, [`cancel_key`]).
pub const PAGE_PAD_KEY: KeyCode = KeyCode::LEFT_SOFT_KEY;

/// How a control is drawn (`crate::render`, hero2's faces).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// A translucent round key with its label.
    Plain,
    /// The OK disc: no label, a gold rim and an inner gold ring (hero2's `Face::Action`).
    Action,
    /// A D-pad arrow: its hexagon inked in this box, an arrow glyph inside.
    Hex(Rect),
    /// hero2's mask button: a dark face and one of its icons in white, no label (the plates' page —
    /// hero2's 「퀵바 페이지 번호 제거」: the arrows, never the page's number).
    Icon(Icon),
    /// Nothing: the game drew what this answers to (the ▶▶ of a dialogue).
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    /// The placeable seat this came from, or `None` for one that is not (a window's arrow, the ▶▶).
    pub id: Option<CtrlId>,
    pub shape: Shape,
    pub action: Action,
    pub label: &'static str,
    pub face: Face,
}

impl Control {
    pub fn new(id: Option<CtrlId>, shape: Shape, action: Action, label: &'static str) -> Control {
        Control { id, shape, action, label, face: Face::Plain }
    }
}

/// The walk stick: a FLOATING stick. A touch anywhere in `zone` fixes the centre where
/// it lands, for the life of the gesture (hero2: three attempts at a centre that follows were each
/// reported as a defect). `seat` is where its idle hint is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stick {
    pub seat: Rect,
    pub zone: Rect,
    /// The drawn base disc; the thumb's centre stays within `base_r * (1 - THUMB)`.
    pub base_r: f64,
    /// The deflection the quantiser measures against ([`joy_dir`]).
    pub engage_r: f64,
}

/// The thumb disc's radius as a fraction of the base's.
pub const THUMB: f64 = 0.44;

/// Which controls a scene gets — hero2's `Mode`. Each has its own: walking needs the
/// stick, a window's cursor needs arrows that step one row at a time, a dialogue needs nothing a
/// thumb could press by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSet {
    /// The field: the stick, OK, CLR (the menu) and the quick plates.
    Field,
    /// A window — the menu, the title, a message box, a scenario's choices or shop, the game's
    /// minimap window, boot: four arrows, OK, CLR (back) and the gear.
    Menu,
    /// A scenario's dialogue with no window up: the picture turns the page, the ▶▶ skips.
    Cutscene,
}

/// What the left seat walks with: the field's per 조작 방식 (`hz_java::ControlMode`) —
/// a window always has its arrows — or nothing while the pad drives (hero2's pad HUD, its player
/// 2026-08-19: *"패드모드일때 화살표는 인게임과 메뉴 전부 없애도 될것같고"*). Nothing means no zone and
/// no arrows either, not only none drawn: a live target nobody can see would eat the very touch
/// that brings the touch HUD back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Walk {
    /// The stick (자유 or 고정 — where its centre goes is the touch side's).
    #[default]
    Stick,
    /// The four arrows in the field too (십자키, hero2's touch type B).
    Arrows,
    /// The pad: neither.
    Pad,
}

/// The set for `view`. Before the game says where it is (no HZ-008), the field's, which has every
/// key the game reads.
pub fn control_set(view: GuestView) -> ControlSet {
    match view.scene {
        Scene::Unknown => ControlSet::Field,
        Scene::Field if !view.popup => ControlSet::Field,
        Scene::Script if !view.window && !view.popup => ControlSet::Cutscene,
        _ => ControlSet::Menu,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub screen: Screen,
    /// Game controls, first match wins.
    pub controls: Vec<Control>,
    /// The field's walk stick.
    pub stick: Option<Stick>,
    /// The square a window's arrows are cut from: its dead centre swallows a touch, so a thumb
    /// resting between the arrows never taps the window under them.
    pub dpad: Option<Rect>,
    /// The gear: over the game's windows (hero2's — the field and a dialogue have none), but not
    /// the game's minimap window in landscape, whose red title band runs right under it
    /// ([`Scene::MapWindow`], S25 2026-10-23); always in portrait's strip, under the picture.
    pub gear: Option<Rect>,
    /// The minimap (Phase 6-5): the panel while it is up, or the button that brings it back.
    pub minimap: Option<Minimap>,
    /// Where the minimap panel goes: the player's seat for it or the built-in
    /// [`minimap_panel`] — whether or not it is up, so the target panel's dodge has one answer.
    pub panel: Rect,
    /// The top tabs as drawn (Phase 7, [`top_tabs`]); each answers to [`top_tab_hit`].
    pub tabs: Vec<(Rect, Tab)>,
    /// The status block's tap target ([`status_tap`]): the field's set. Whether the
    /// block is drawn just now (numbers read, STATS) is the touch side's to ask.
    pub status: Option<Rect>,
}

/// The minimap's tap target, as [`layout`] places it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Minimap {
    /// The panel ([`Layout::panel`]): a tap hides it.
    Panel(Rect),
    /// The panel is hidden: its restore button ([`minimap_restore`]).
    Restore(Rect),
}

impl Minimap {
    pub fn rect(&self) -> Rect {
        match *self {
            Minimap::Panel(r) | Minimap::Restore(r) => r,
        }
    }

    /// What a touch has to land in: the panel, or the restore button's bigger target
    /// ([`minimap_restore_hit`]).
    pub fn target(&self, s: &Screen) -> Rect {
        match *self {
            Minimap::Panel(r) => r,
            Minimap::Restore(r) => minimap_restore_hit(s, r),
        }
    }
}

impl Layout {
    pub fn landscape(&self) -> bool {
        self.screen.landscape()
    }
}

/// The layout for `s`, with the player's placements from `table` and the
/// [`ControlSet`] of `view`'s scene: the quick plates wherever the player's HUD is up — the field
/// and the game's windows over it, hero2's menu mode (A7, [`GuestView::ring_keys`]) — and `#`
/// also on the 「# 확인」 notice, in any set. The field walks with the stick.
pub fn layout(s: Screen, table: &Table, view: GuestView) -> Layout {
    layout_for(s, table, view, Walk::Stick)
}

/// [`layout`] with the left seat's [`Walk`].
pub fn layout_for(s: Screen, table: &Table, view: GuestView, walk: Walk) -> Layout {
    if s.desktop {
        return desktop_layout(s, table, view);
    }
    let set = control_set(view);
    // The keys the strip shows. Portrait keeps a window's through a dialogue (user 2026-10-31: 「세로모드에서는
    // 이벤트컷씬에서 하단에 버튼들 안없어지게 … 상단 게임 화면까지 손가락을 올려서 터치해야해서 불편해. 가로모드는 지금이
    // 좋아」): OK turns the page (`eb.a(II)Z` @952 `o.f` = FIRE / 5 / LSK), CLR is RSK — the skip, as the ▶▶
    // (@784) — and the arrows, which a dialogue ignores, keep the strip as the choice windows between its pages
    // have it. Landscape draws nothing there, as hero2's: the picture turns the page.
    let keys = if set == ControlSet::Cutscene && !s.landscape() { ControlSet::Menu } else { set };
    let arrows = walk != Walk::Pad && (keys == ControlSet::Menu || walk == Walk::Arrows);
    let mut controls = Vec::new();
    let mut stick = None;
    let mut dpad = None;
    let mut map_panel = minimap_panel(&s);
    if set == ControlSet::Cutscene {
        controls.push(Control { face: Face::Hidden, ..Control::new(None, Shape::Box(skip_hint(&s)), Action::Key(KeyCode::RIGHT_SOFT_KEY), "") });
    }
    for (id, r) in placement::resolve_with(&s, table) {
        match id {
            CtrlId::Stick if keys == ControlSet::Cutscene => {}
            CtrlId::Stick if arrows => {
                controls.extend(dpad_controls(r));
                dpad = Some(r);
            }
            CtrlId::Stick if set == ControlSet::Field && walk == Walk::Stick => stick = Some(stick_for(&s, r)),
            CtrlId::Stick => {}
            CtrlId::Minimap => map_panel = r,
            CtrlId::Ok if keys != ControlSet::Cutscene => {
                controls.push(Control { face: Face::Action, ..Control::new(Some(id), circle_in(r), Action::Key(ok_key(view)), "OK") });
            }
            CtrlId::Cancel if keys != ControlSet::Cutscene => controls.push(Control::new(Some(id), circle_in(r), Action::Key(cancel_key(view)), "CLR")),
            CtrlId::Ok | CtrlId::Cancel => {}
            // A plate the player switched off. (The 「# 확인」 notice's `#` is OK's there — `ok_key`.)
            CtrlId::Ring(_) if table.is_hidden(id) => {}
            CtrlId::Ring(i) => {
                let (plate, label, _) = RING_PLATES[i as usize];
                if view.ring_keys() {
                    controls.push(Control::new(Some(id), circle_in(r), Action::Plate(plate), label));
                }
            }
            CtrlId::Page => {
                if view.ring_keys() {
                    controls.push(Control { face: Face::Icon(Icon::QuickPage), ..Control::new(Some(id), circle_in(r), Action::Page, "") });
                }
            }
        }
    }
    let gear_up = !s.landscape() || (set == ControlSet::Menu && view.scene != Scene::MapWindow) || view.scene == Scene::Unknown;
    let status = (set == ControlSet::Field).then(|| status_tap(&s));
    Layout { screen: s, controls, stick, dpad, gear: gear_up.then(|| gear(&s)), minimap: None, panel: map_panel, tabs: Vec::new(), status }
}

/// The PC's layout ([`Screen::desktop`]): the quick bar's cells wherever the phone shows its plates
/// (`#` on the 「# 확인」 notice too), the dialogue's invisible ▶▶, the gear over the windows, the
/// status block in the field — and nothing a thumb would need: the keyboard walks, confirms, cancels.
/// The player's minimap seat (the layout file) is honoured; the rest of the table is the phone's.
fn desktop_layout(s: Screen, table: &Table, view: GuestView) -> Layout {
    let set = control_set(view);
    let mut controls = Vec::new();
    if set == ControlSet::Cutscene {
        controls.push(Control { face: Face::Hidden, ..Control::new(None, Shape::Box(skip_hint(&s)), Action::Key(KeyCode::RIGHT_SOFT_KEY), "") });
    }
    let map_panel = placement::resolve_with(&s, table).into_iter().find(|(id, _)| *id == CtrlId::Minimap).map_or_else(|| minimap_panel(&s), |(_, r)| r);
    if set != ControlSet::Cutscene {
        if view.ring_keys() {
            for (i, r) in quick_cells(&s).into_iter().enumerate() {
                controls.push(Control::new(None, Shape::Box(r), Action::Plate(i as u8), PC_PLATE_KEYS[i]));
            }
            controls.push(Control { face: Face::Icon(Icon::QuickPage), ..Control::new(None, Shape::Box(page_cell(&s)), Action::Page, "TAB") });
        }
    }
    let gear_up = (set == ControlSet::Menu && view.scene != Scene::MapWindow) || view.scene == Scene::Unknown;
    let status = (set == ControlSet::Field).then(|| status_tap(&s));
    Layout { screen: s, controls, stick: None, dpad: None, gear: gear_up.then(|| gear(&s)), minimap: None, panel: map_panel, tabs: Vec::new(), status }
}

/// The minimap's tap target for [`Layout::minimap`]: the panel `panel` in the field once a map has
/// been read (`shown` = whether the player has it up), nowhere else.
pub fn minimap_target(s: &Screen, panel: Rect, view: GuestView, shown: Option<bool>) -> Option<Minimap> {
    if view.scene != Scene::Field {
        return None;
    }
    shown.map(|up| if up { Minimap::Panel(panel) } else { Minimap::Restore(minimap_restore(s, panel)) })
}

fn circle_in(r: Rect) -> Shape {
    let (cx, cy) = r.centre();
    Shape::Circle { cx, cy, r: r.w.min(r.h) / 2 }
}

/// The stick for a resolved seat. The zone is a POLICY (input-and-hud.md §5): a band around the
/// seat, the seat's half of the window, under the status block — not the whole window, or every
/// near-miss of every other control walks the character.
fn stick_for(s: &Screen, seat: Rect) -> Stick {
    let (cx, _) = seat.centre();
    let half = if cx < s.w / 2 {
        Rect::new(0, 0, s.w / 2, s.h)
    } else {
        Rect::new(s.w / 2, 0, s.w - s.w / 2, s.h)
    };
    // Portrait: a seat in the strip walks from the strip only — the picture over it is the game's.
    let top = if !s.landscape() && seat.y >= s.game.bottom() { s.game.bottom() } else { status_box(s).bottom() + s.pad() };
    let band = Rect::new(cx - seat.w, top, seat.w * 2, band_top(s) - top);
    let builtin = stick_seat(s);
    // hero2's numbers (tuned by feel on S9/S25-class phones): the base is the circle through the
    // arrows' centres (`arm_centre_radius`: 0.5 − 0.02 − 0.17 of the side); the deflection basis
    // is h/12 scaled by the seat's size, so at 100 % the character starts walking after ~27 px on
    // a 1080-high window and a bigger stick starts later.
    let base_r = f64::from(seat.w.min(seat.h)) / 2.0 * 0.62;
    let engage_r = f64::from((s.h / 12).clamp(34, 96)) * f64::from(seat.w.max(1)) / f64::from(builtin.w.max(1));
    Stick {
        seat,
        zone: band.clipped(&half).clipped(&s.window()),
        base_r,
        engage_r,
    }
}

/// What a touch at (`x`, `y`) lands on — THE hit-test chain (input-and-hud.md §1), in its one
/// order. The layout editor and the settings panel sit above it and are handled by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Gear,
    /// A top tab ([`Layout::tabs`]).
    Tab(Tab),
    /// Index into `layout.controls`.
    Control(usize),
    /// The D-pad's dead centre: nothing, and not the window under it either.
    Dead,
    /// The minimap panel or its restore button ([`Layout::minimap`]).
    Minimap,
    /// The status block ([`Layout::status`]).
    Status,
    StickZone,
    Nothing,
}

pub fn hit(l: &Layout, x: f64, y: f64) -> Hit {
    let slack = f64::from(l.screen.pad());
    if l.gear.is_some_and(|g| g.grown(l.screen.pad()).contains(x, y)) {
        return Hit::Gear;
    }
    if let Some((_, tab)) = l.tabs.iter().find(|(r, _)| top_tab_hit(&l.screen, *r).contains(x, y)) {
        return Hit::Tab(*tab);
    }
    if let Some(i) = l.controls.iter().position(|c| c.shape.contains(x, y, slack)) {
        return Hit::Control(i);
    }
    if l.dpad.is_some_and(|d| d.contains(x, y)) {
        return Hit::Dead;
    }
    // After the controls: a placement the player dragged onto the panel keeps working (the panel
    // is not a placement obstacle, so no saved layout falls back to the defaults for it).
    if l.minimap.is_some_and(|m| m.target(&l.screen).contains(x, y)) {
        return Hit::Minimap;
    }
    if l.status.is_some_and(|r| r.contains(x, y)) {
        return Hit::Status;
    }
    if l.stick.is_some_and(|st| st.zone.contains(x, y)) {
        return Hit::StickZone;
    }
    Hit::Nothing
}

// ---------------------------------------------------------------------- the stick ----

/// The direction for a stick deflected by (`dx`, `dy`) — four-way, dominant axis, ONE key: the
/// game keeps one held key (`hz_java::input`), so a diagonal is not something it can walk.
/// `active` is kept unless the challenging axis beats it by [`TURN_RATIO`], and two dead zones
/// stop a resting thumb chattering. hero2's constants, tuned by feel; re-tune on device.
pub fn joy_dir(dx: f64, dy: f64, radius: f64, active: Option<KeyCode>) -> Option<KeyCode> {
    const DEAD_ENGAGE: f64 = 0.30;
    const DEAD_KEEP: f64 = 0.15;
    let dist = (dx * dx + dy * dy).sqrt();
    let dead = radius * if active.is_some() { DEAD_KEEP } else { DEAD_ENGAGE };
    if dist <= dead || dist == 0.0 {
        return None;
    }
    let comp = |k: KeyCode| match k {
        KeyCode::UP => -dy,
        KeyCode::DOWN => dy,
        KeyCode::LEFT => -dx,
        _ => dx,
    };
    let cand = if dx.abs() >= dy.abs() {
        if dx >= 0.0 { KeyCode::RIGHT } else { KeyCode::LEFT }
    } else if dy >= 0.0 {
        KeyCode::DOWN
    } else {
        KeyCode::UP
    };
    if let Some(cur) = active
        && cur != cand
        && comp(cur) > 0.0
        && comp(cand) < comp(cur) * TURN_RATIO
    {
        return Some(cur);
    }
    Some(cand)
}

/// How far the challenging axis must beat the held one to take the turn.
pub const TURN_RATIO: f64 = 1.2;

/// HZ-059 (user 2026-10-30, 대각선 이동): what a stick holds in the bare field — one arrow, or a
/// diagonal's two (horizontal first). Two honest key edges: the worker makes them the diagonal
/// (`KeyProjection::walk_edge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    One(KeyCode),
    Two(KeyCode, KeyCode),
}

impl Way {
    pub fn keys(self) -> impl Iterator<Item = KeyCode> {
        let (a, b) = match self {
            Way::One(k) => (k, None),
            Way::Two(h, v) => (h, Some(v)),
        };
        std::iter::once(a).chain(b)
    }

    fn holds(self, k: KeyCode) -> bool {
        self.keys().any(|h| h == k)
    }
}

/// The edges from what a stick held to what it holds now: the arrows let go of first, then the new
/// ones, horizontal before vertical — a turn from ↘ to ↙ lifts → and presses ←, ↓ stays down.
pub fn way_edges(old: Option<Way>, new: Option<Way>) -> Vec<(KeyCode, bool)> {
    let ups = old.into_iter().flat_map(Way::keys).filter(|&k| !new.is_some_and(|w| w.holds(k))).map(|k| (k, false));
    let downs = new.into_iter().flat_map(Way::keys).filter(|&k| !old.is_some_and(|w| w.holds(k))).map(|k| (k, true));
    ups.chain(downs).collect()
}

/// Degrees past a sector's own half-width the held way keeps against a challenger — a thumb
/// resting on a border must not chatter between an arrow and a diagonal.
pub const WAY_HOLD_DEG: f64 = 7.0;

/// [`joy_dir`] eight ways (`eight`, the bare field): 45° sectors round the arrows and the diagonals,
/// the same two dead zones, and the held way kept until the thumb is [`WAY_HOLD_DEG`] past its
/// sector. Four ways (`!eight`, a window's cursor): `joy_dir`, a held diagonal counting as its
/// horizontal arrow.
pub fn joy_way(dx: f64, dy: f64, radius: f64, active: Option<Way>, eight: bool) -> Option<Way> {
    if !eight {
        let held = active.map(|w| w.keys().next().unwrap_or(KeyCode::RIGHT));
        return joy_dir(dx, dy, radius, held).map(Way::One);
    }
    const DEAD_ENGAGE: f64 = 0.30;
    const DEAD_KEEP: f64 = 0.15;
    let dist = (dx * dx + dy * dy).sqrt();
    let dead = radius * if active.is_some() { DEAD_KEEP } else { DEAD_ENGAGE };
    if dist <= dead || dist == 0.0 {
        return None;
    }
    // 0 → (east), counting clockwise on the screen (y grows down): 1 ↘, 2 ↓ … 7 ↗.
    let deg = dy.atan2(dx).to_degrees().rem_euclid(360.0);
    let sector_of = |w: Way| -> f64 {
        match w {
            Way::One(KeyCode::RIGHT) => 0.0,
            Way::Two(KeyCode::RIGHT, KeyCode::DOWN) => 45.0,
            Way::One(KeyCode::DOWN) => 90.0,
            Way::Two(KeyCode::LEFT, KeyCode::DOWN) => 135.0,
            Way::One(KeyCode::LEFT) => 180.0,
            Way::Two(KeyCode::LEFT, KeyCode::UP) => 225.0,
            Way::One(KeyCode::UP) => 270.0,
            _ => 315.0,
        }
    };
    if let Some(w) = active {
        let off = (deg - sector_of(w)).rem_euclid(360.0);
        if off.min(360.0 - off) <= 22.5 + WAY_HOLD_DEG {
            return Some(w);
        }
    }
    const WAYS: [Way; 8] = [
        Way::One(KeyCode::RIGHT),
        Way::Two(KeyCode::RIGHT, KeyCode::DOWN),
        Way::One(KeyCode::DOWN),
        Way::Two(KeyCode::LEFT, KeyCode::DOWN),
        Way::One(KeyCode::LEFT),
        Way::Two(KeyCode::LEFT, KeyCode::UP),
        Way::One(KeyCode::UP),
        Way::Two(KeyCode::RIGHT, KeyCode::UP),
    ];
    Some(WAYS[((deg + 22.5) / 45.0) as usize % 8])
}

/// HZ-059: how near a diagonal (degrees, either side) a thumb on the field's D-pad holds both of its
/// arrows — the gap between two arms, where no arrow is drawn (an arm's hexagon reaches about 24°
/// off its axis at the rim, [`ARM_WIDTH_PCT`]).
pub const DPAD_CORNER_DEG: f64 = 20.0;

/// HZ-059: a point on the D-pad's `square` in a corner — within [`DPAD_CORNER_DEG`] of a diagonal,
/// outside the dead disc — and the diagonal's two arrows, horizontal first; `None` elsewhere.
pub fn dpad_corner(square: Rect, x: f64, y: f64) -> Option<(KeyCode, KeyCode)> {
    if !square.contains(x, y) {
        return None;
    }
    let (cx, cy) = (f64::from(square.x) + f64::from(square.w) / 2.0, f64::from(square.y) + f64::from(square.h) / 2.0);
    let (dx, dy) = (x - cx, y - cy);
    let dead = f64::from(square.w.min(square.h) * DEAD_PCT) / 100.0;
    if dx * dx + dy * dy < dead * dead {
        return None;
    }
    // 0° on an axis, 45° on a diagonal.
    let off_axis = dx.abs().min(dy.abs()).atan2(dx.abs().max(dy.abs())).to_degrees();
    (off_axis >= 45.0 - DPAD_CORNER_DEG).then_some((if dx < 0.0 { KeyCode::LEFT } else { KeyCode::RIGHT }, if dy < 0.0 { KeyCode::UP } else { KeyCode::DOWN }))
}

#[cfg(test)]
pub(crate) mod tests {
    /// The PC's layout (2026-10-29): the quick bar's six cells and nothing a thumb would need, in a
    /// landscape window and in a portrait one — the tabs and the gear where landscape keeps them
    /// (no strip under the picture), the picture fitted on both axes.
    #[test]
    fn the_desktop_layout_is_the_quick_bar_and_the_chrome_in_any_window() {
        use crate::placement::Table;
        use hz_java::{GuestView, Scene};
        for (w, h, gw, gh) in [(1280u32, 720u32, 426u32, 240u32), (600, 800, 240, 320), (1000, 1000, 426, 240)] {
            let s = super::Screen::new_desktop(w, h, gw, gh);
            assert!(s.desktop && s.game.w > 0 && s.game.h > 0 && s.game.right() <= s.w && s.game.bottom() <= s.h, "{w}x{h}");
            let field = GuestView { scene: Scene::Field, ..Default::default() };
            let l = super::layout(s, &Table::default(), field);
            assert!(l.stick.is_none() && l.dpad.is_none(), "{w}x{h}: no thumb controls");
            let acts: Vec<_> = l.controls.iter().map(|c| c.action).collect();
            let want: Vec<_> = (0..super::QUICK_PLATES as u8).map(super::Action::Plate).chain([super::Action::Page]).collect();
            assert_eq!(acts, want, "{w}x{h}: the seven plates in order, then the page cell");
            let page = super::page_cell(&s);
            assert!(page.inside(&s.window()) && page.x > super::quick_cells(&s)[6].right() && page.w < super::quick_cells(&s)[6].w, "{w}x{h}: the half-wide page cell after them");
            assert!(l.controls.iter().all(|c| matches!(c.shape, super::Shape::Box(_))));
            let cells = super::quick_cells(&s);
            assert!(cells.iter().all(|c| c.inside(&s.window()) && c.bottom() <= super::exp_bar(&s).y), "{w}x{h}: in the window, above the EXP bar");
            assert!(l.status.is_some() && l.gear.is_none(), "{w}x{h}: the field has the status block, no gear");
            let (save, map) = super::top_tab_seats(&s, s.guest_w);
            assert!(!save.is_empty() && !map.is_empty() && save.inside(&s.window()) && map.inside(&s.window()), "{w}x{h}: two tabs on screen");
            assert!(save.w >= 40 && !save.intersects(&map) && cells.iter().all(|c| !c.intersects(&save) && !c.intersects(&map)), "{w}x{h}: tabs a hand can hit, clear of the cells");
            if s.landscape() {
                assert_eq!((save.y, map.y), (0, 0), "{w}x{h}: at the top");
            }
            let menu = GuestView { scene: Scene::Menu, ..Default::default() };
            let m = super::layout(s, &Table::default(), menu);
            assert!(m.gear.is_some_and(|g| g.inside(&s.window())), "{w}x{h}: the gear over a window, on screen");
            assert_eq!(m.controls.len(), super::QUICK_PLATES + 1, "{w}x{h}: the cells stay over the menu (A7) — they take a 「퀵슬롯등록」");
        }
    }

    use super::*;
    use crate::placement::Place;

    /// S9 (2220x1080), S25 Ultra (3120x1440), a small 1600x720, a near-square 1440x1080 and a
    /// very wide 2400x900.
    pub(crate) const WINDOWS: [(u32, u32); 5] = [(2220, 1080), (3120, 1440), (1600, 720), (1440, 1080), (2400, 900)];

    /// The 창고 question's answer line (choice ②, 2026-10-27), a box the size the game's font makes
    /// it (≈ 150 × 17 guest px): in the picture, under the game's windows and over the EXP bar —
    /// and on a phone clear of the window set's controls (a 4:3 window has no room between the
    /// arrows and the plates: it goes under them there, `render::draw_hud`).
    #[test]
    fn the_answer_line_sits_under_the_windows_over_the_exp_bar() {
        let question = GuestView { scene: Scene::Menu, lsk_ok: true, ..Default::default() };
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let k = f64::from(s.game.h) / f64::from(s.guest_h);
            let r = answer_hint(&s, (150.0 * k) as i32, (17.0 * k) as i32);
            assert!(r.inside(&s.game), "{w}x{h}: {r:?} off the picture {:?}", s.game);
            assert!(r.y >= s.gy(s.guest_h - WINDOW_FLOOR_GP), "{w}x{h}: {r:?} on the game's windows");
            assert!(r.bottom() < exp_bar(&s).y, "{w}x{h}: {r:?} on the EXP bar {:?}", exp_bar(&s));
            if f64::from(w) / f64::from(h) < 1.5 {
                continue;
            }
            let l = layout(s, &Table::default(), question);
            for c in &l.controls {
                assert!(!c.shape.bounds().intersects(&r), "{w}x{h}: {r:?} on {} {:?}", c.label, c.shape.bounds());
            }
            assert!(!l.gear.is_some_and(|g| g.intersects(&r)) && !r.intersects(&status_box(&s)), "{w}x{h}: {r:?}");
        }
    }

    /// hero2's seats: OK, CLR, the plates and the stick inside the window down to its bottom edge,
    /// clear of the gear, the discs apart, the stick and the action cluster on opposite sides — and
    /// the EXP bar between them clear of all of it.
    #[test]
    fn built_ins_are_inside_the_window_and_clear_of_every_obstacle() {
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let obstacles = obstacles(&s);
            let plates: Vec<Rect> = (0..RING_PLATES.len()).map(|i| plate_seat(&s, i)).collect();
            let mut seats = vec![ok_disc(&s), stick_seat(&s), cancel_seat(&s)];
            seats.extend(plates.iter().copied());
            for r in &seats {
                assert!(r.inside(&s.window()), "{w}x{h}: {r:?} outside the window");
                for o in &obstacles {
                    assert!(!r.grown(s.pad()).intersects(o), "{w}x{h}: {r:?} on obstacle {o:?}");
                }
                assert!(!r.intersects(&exp_bar(&s)), "{w}x{h}: {r:?} on the EXP bar {:?}", exp_bar(&s));
            }
            let ok = ok_disc(&s);
            let d = |a: Rect, b: Rect| {
                let ((ax, ay), (bx, by)) = (a.centre(), b.centre());
                f64::from((ax - bx).pow(2) + (ay - by).pow(2)).sqrt()
            };
            for (i, a) in plates.iter().enumerate() {
                assert!(d(*a, ok) >= f64::from(a.w + ok.w) / 2.0, "{w}x{h}: plate {i} overlaps OK");
                for b in &plates[i + 1..] {
                    assert!(d(*a, *b) >= f64::from(a.w), "{w}x{h}: plates overlap");
                }
            }
            assert!(stick_seat(&s).right() < ok.x, "{w}x{h}: stick and OK cluster on opposite sides");
            assert_eq!(ok.bottom() + s.pad() * 3 + (s.h / 6).clamp(56, 190) - ok.h / 2, s.h, "{w}x{h}: hero2's corner, from the window's bottom");
            assert_eq!(stick_seat(&s).bottom(), s.h - s.pad() * 4, "{w}x{h}: the stick's square too");
        }
        // The S25 at FHD+: hero2's numbers — a 624 px walk square, plates of 98, CLR 162.
        let s = Screen::for_window(2340, 1080);
        // HZ-058's seven plates on hero2's eight-seat ring: 86.
        assert_eq!((stick_seat(&s).w, plate_seat(&s, 0).w, cancel_seat(&s).w), (624, 86, 162));
    }
    /// Phase 6-4 (hero2's `cancel_seat`): CLR at the right edge between the first plate and OK,
    /// three fifths of OK, clear of OK, every plate and the gear — on every window.
    #[test]
    fn the_cancel_disc_sits_above_ok_clear_of_everything() {
        let apart = |a: Rect, b: Rect| {
            let ((ax, ay), (bx, by)) = (a.centre(), b.centre());
            f64::from((ax - bx).pow(2) + (ay - by).pow(2)).sqrt() >= f64::from(a.w.min(a.h) + b.w.min(b.h)) / 2.0
        };
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let c = cancel_seat(&s);
            let ok = ok_disc(&s);
            assert!(c.inside(&s.window()), "{w}x{h}: {c:?} outside the window");
            assert!(c.centre().1 < ok.centre().1 && c.right() > ok.centre().0, "{w}x{h}: above OK, toward the edge");
            assert!(apart(c, ok), "{w}x{h}: on OK");
            for i in 0..RING_PLATES.len() {
                assert!(apart(c, plate_seat(&s, i)), "{w}x{h}: on plate {i}");
            }
            for o in obstacles(&s) {
                assert!(!c.intersects(&o), "{w}x{h}: on obstacle {o:?}");
            }
            assert_eq!(c.w, (ok.w * 3 / 5).max(56).max(ring(&s)[0].w), "{w}x{h}: OK's three fifths");
        }
    }
    /// Phase 6-5: the minimap sits in the picture's top-right corner, clear of every built-in
    /// control and the chrome, in both orientations; its restore button is in its own corner.
    #[test]
    fn the_minimap_is_in_the_top_right_corner_clear_of_the_controls() {
        let portrait = [(1080u32, 2220u32), (1440, 3120)].map(|(w, h)| Screen::new(w, h, 240, 320));
        let landscape = WINDOWS.map(|(w, h)| Screen::for_window(w, h));
        for s in landscape.iter().chain(portrait.iter()) {
            let (w, h) = (s.w, s.h);
            let p = minimap_panel(s);
            assert!(p.inside(&s.game), "{w}x{h}: {p:?} inside the picture {:?}", s.game);
            assert!(p.right() >= s.game.right() - s.pad() * 3, "{w}x{h}: the right edge");
            // The top-right corner — in portrait under the game's target panel at its own place.
            let top = if s.landscape() { s.game.y + s.pad() * 3 } else { target_box(s, 0, s.guest_w).bottom() + guest_unit(s) * 3 };
            assert!(p.y <= top, "{w}x{h}: {p:?} the top-right corner");
            assert!((120..=280).contains(&p.w) && p.w == p.h, "{w}x{h}: {p:?}");
            let r = minimap_restore(s, p);
            assert!(r.inside(&p) && r.right() == p.right() && r.y == p.y, "{w}x{h}: the restore button in the panel's corner");
            assert!(minimap_seat_ok(s, p), "{w}x{h}: the built-in seat passes the editor's own rule");
            let mut seats = vec![ok_disc(s), stick_seat(s), cancel_seat(s), gear(s)];
            seats.extend((0..RING_PLATES.len()).map(|i| plate_seat(s, i)));
            for c in seats {
                assert!(!c.intersects(&p), "{w}x{h}: {c:?} under the minimap {p:?}");
            }
            assert!(!gear(s).intersects(&p) && !hz_status_corner(s).intersects(&p), "{w}x{h}: clear of the chrome and the status block");
        }
    }

    /// The status block's corner (`hud_view`'s top-left of the picture), coarsely.
    fn hz_status_corner(s: &Screen) -> Rect {
        Rect::new(s.game.x, s.game.y, s.game.w / 4, s.game.h / 5)
    }

    /// HZ-014: the target panel moves left only as far as the minimap needs, and then clears it.
    #[test]
    fn the_target_panel_dodges_the_minimap_by_the_overlap() {
        // S25 Ultra at FHD+ (the device's setting): 2340x1080, a 520x240 canvas at 4.5x.
        let s = Screen::for_window(2340, 1080);
        assert_eq!((s.guest_w, s.guest_h), (520, 240));
        let p = minimap_panel(&s);
        assert_eq!(target_box(&s, 0, s.guest_w), Rect::new(1980, 0, 360, 153), "the game's spot: 440..520 x 0..34");
        let shift = target_dodge(&s, p, s.guest_w);
        assert_eq!(shift, 69, "(2340 - {}) / 4.5, rounded up, + 2", p.x);
        let moved = target_box(&s, shift, s.guest_w);
        assert!(!moved.intersects(&p) && moved.right() <= p.x, "{moved:?} clear of {p:?}");
        assert!(p.x - moved.right() < 3 * 5, "and no further than it takes");
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let p = minimap_panel(&s);
            let shift = target_dodge(&s, p, s.guest_w);
            assert!(shift > 0 && !target_box(&s, shift, s.guest_w).intersects(&p), "{w}x{h}: {shift}");
        }
        assert_eq!(target_dodge(&s, Rect::new(0, 600, 200, 200), s.guest_w), 0, "a panel elsewhere leaves it alone");
    }

    /// HZ-014 over a narrow map (review 2026-10-25): the hook keeps the target panel inside the
    /// map's picture — over map 135 (20 tiles) that is 340..420 on the S25's 520 canvas — so the
    /// dodge counts from there, and a seat the editor takes is one the panel clears over every map
    /// it fights on.
    #[test]
    fn the_target_panel_dodges_the_minimap_over_a_narrow_map() {
        let s = Screen::for_window(2340, 1080);
        let room = hz_java::hooks::FIGHT_MAP_MIN_W;
        let (l, r) = (s.gx(340), s.gx(420));
        assert_eq!(target_box(&s, 0, room), Rect::new(l, 0, r - l, 153), "inside the picture 100..420");
        let at = |cx: i32, cy: i32, pct: u32| Place { cx: cx as f32 / s.w as f32, cy: cy as f32 / s.h as f32, pct };
        let accepted = |p: Place| placement::would_accept_place(&s, &Table::default(), CtrlId::Minimap, p);
        let seat = |p: Place| {
            let mut t = Table::default();
            t.set(CtrlId::Minimap, Some(p));
            layout(s, &t, GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() }).panel
        };
        // Half size at the top, between the tabs and the corner: clear of 440..520, over 340..420.
        let place = at(s.gx(400), minimap_panel(&s).h / 4 + s.pad() * 2, 50);
        assert!(accepted(place));
        let p = seat(place);
        assert_eq!(target_dodge(&s, p, s.guest_w), 0, "{p:?}: nothing to dodge on a wide map");
        assert!(target_box(&s, 0, room).intersects(&p), "{p:?}: but over the room's panel");
        let shift = target_dodge(&s, p, room);
        let moved = target_box(&s, shift, room);
        assert!(!moved.intersects(&p) && p.x - moved.right() < 3 * 5, "{moved:?} just clear of {p:?}");
        assert!(moved.x >= s.gx(100), "still inside the picture");
        // Every seat the editor takes, the panel clears on every map width a fight can have.
        assert_eq!(fight_map_widths(&s).next(), Some(320));
        assert_eq!(fight_map_widths(&s).last(), Some(s.guest_w));
        for pct in [placement::PCT_MIN, 100, 160, placement::PCT_MAX] {
            for cy in (0..s.h / 2).step_by(30) {
                for cx in (0..s.w).step_by(30) {
                    if !accepted(at(cx, cy, pct)) {
                        continue;
                    }
                    let p = seat(at(cx, cy, pct));
                    for w in fight_map_widths(&s) {
                        let t = target_box(&s, target_dodge(&s, p, w), w);
                        assert!(!t.intersects(&p), "{pct}% at ({cx}, {cy}) = {p:?}, a {w}-px map: {t:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_minimap_target_is_for_the_field_with_a_map() {
        let s = Screen::for_window(2220, 1080);
        let v = |scene| GuestView { scene, popup: false, notice: false, ..Default::default() };
        let p = minimap_panel(&s);
        assert_eq!(minimap_target(&s, p, v(Scene::Field), None), None, "no map read yet");
        assert_eq!(minimap_target(&s, p, v(Scene::Field), Some(true)), Some(Minimap::Panel(p)));
        assert_eq!(minimap_target(&s, p, v(Scene::Field), Some(false)), Some(Minimap::Restore(minimap_restore(&s, p))));
        assert_eq!(minimap_target(&s, p, v(Scene::Menu), Some(true)), None, "the menu covers the field");
        let mut l = layout(s, &Table::default(), v(Scene::Field));
        assert_eq!(l.panel, p, "no seat stored: the built-in");
        l.minimap = minimap_target(&s, l.panel, v(Scene::Field), Some(true));
        let (cx, cy) = p.centre();
        assert_eq!(hit(&l, f64::from(cx), f64::from(cy)), Hit::Minimap);
    }

    /// The minimap as a placeable control: the player's seat is the panel, the target panel's
    /// dodge and the restore button follow it, and a seat over the status block, a top tab, the
    /// gear or the bottom band is refused (the panel stays where it was).
    #[test]
    fn the_minimap_moves_where_the_editor_puts_it() {
        let s = Screen::for_window(2340, 1080);
        let base = minimap_panel(&s);
        let at = |cx: i32, cy: i32, pct: u32| Place { cx: cx as f32 / s.w as f32, cy: cy as f32 / s.h as f32, pct };
        let seat = |p: Place| {
            let mut t = Table::default();
            t.set(CtrlId::Minimap, Some(p));
            layout(s, &t, GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() }).panel
        };
        // Lower down the right edge, a little smaller: taken, and the target panel stays home.
        let low = seat(at(base.centre().0, s.h / 2, 80));
        let (lx, ly) = low.centre();
        assert!(low.w == base.w * 80 / 100 && (lx - base.centre().0).abs() <= 1 && (ly - s.h / 2).abs() <= 1, "{low:?}");
        assert!(minimap_seat_ok(&s, low));
        assert_eq!(target_dodge(&s, low, s.guest_w), 0, "nothing to dodge");
        let r = minimap_restore(&s, low);
        assert!(r.inside(&low) && r.right() == low.right() && r.y == low.y);
        // Refused seats: the built-in panel stays.
        let sb = status_box(&s);
        let tab = top_tab_hit(&s, save_tab(&s, s.guest_w));
        let refused = [
            ("status block", at(sb.centre().0, sb.centre().1, 100)),
            ("SAVE tab", at(tab.centre().0, tab.y + base.h / 2, 100)),
            ("gear", at(gear(&s).centre().0, gear(&s).centre().1, 100)),
        ];
        for (what, p) in refused {
            assert_eq!(seat(p), base, "{what}: {p:?}");
        }
        // The editor's predicate agrees with the resolver.
        assert!(placement::would_accept_place(&s, &Table::default(), CtrlId::Minimap, at(base.centre().0, s.h / 2, 80)));
        assert!(!placement::would_accept_place(&s, &Table::default(), CtrlId::Minimap, refused[0].1));
        // FLIP leaves it where it is.
        let mut t = Table::default();
        t.set(CtrlId::Minimap, Some(at(base.centre().0, s.h / 2, 80)));
        t.mirror(&s);
        assert_eq!(t.get(CtrlId::Minimap), Some(at(base.centre().0, s.h / 2, 80)), "the picture of the map does not flip");
    }

    /// Phase 7: the tabs hang from the top edge left of the minimap and the dodged target panel,
    /// MAP half a tab left of SAVE, and neither the faces nor the bigger touch targets reach the
    /// chrome, the minimap, the target panel, the status block or a control — both orientations,
    /// over every map a fight can be on (a narrow one keeps the panel further left, and the tabs
    /// stop short of it there while two still fit — user 2026-10-25).
    #[test]
    fn the_top_tabs_hang_left_of_the_corner_clear_of_everything() {
        // Landscape's: portrait's are in its strip (`portrait_tabs_are_in_the_strip`).
        let landscape = [(2340u32, 1080u32)].into_iter().chain(WINDOWS).map(|(w, h)| Screen::for_window(w, h));
        for s in landscape {
            let (w, h) = (s.w, s.h);
            let wide = save_tab(&s, s.guest_w);
            let panel = minimap_panel(&s);
            for mw in fight_map_widths(&s) {
                let (save, map) = (save_tab(&s, mw), map_tab(&s, mw));
                let (sh, mh) = (top_tab_hit(&s, save), top_tab_hit(&s, map));
                assert_eq!((save.y, map.y), (0, 0), "{w}x{h} {mw}: hanging from the top edge");
                let plate = plate_seat(&s, 0).w;
                assert!(save.w <= plate && save.w >= plate / 2 && save.h == save.w * 3 / 5, "{w}x{h} {mw}: {save:?} vs a {plate} plate");
                assert_eq!(map.right() + save.w / 2, save.x, "{w}x{h} {mw}: MAP half a tab left of SAVE");
                assert!(sh.inside(&s.window()) && mh.inside(&s.window()), "{w}x{h} {mw}");
                assert!(save.inside(&sh) && map.inside(&mh), "{w}x{h} {mw}: the touch target holds the face");
                assert!(!sh.intersects(&mh), "{w}x{h} {mw}: the two targets meet in the gap, never overlap");
                assert!(save.right() <= wide.right(), "{w}x{h} {mw}: never right of the canvas's seat");
                let target = target_box(&s, target_dodge(&s, panel, mw), mw);
                // Not the gear: it is a window's, and the tabs are the bare field's.
                let mut keep_clear = vec![panel, status_box(&s)];
                // The target panel too, where the band beside it had room (not 4:3, not portrait;
                // over a narrow map, where two tabs fit before it — else the canvas's tabs stay).
                if tab_band(&s, s.guest_w).2 {
                    if save == wide && target.intersects(&sh) {
                        let room = target.x.min(panel.x) - s.pad() * 2 - tab_band(&s, mw).0;
                        assert!(mw < s.guest_w && room < plate * 3 / 5 * 3, "{w}x{h} {mw}: {room} px of room, kept the canvas's tabs");
                    } else {
                        keep_clear.push(target);
                    }
                } else {
                    assert!(!s.landscape() || w * 3 < h * 4 + h / 10, "{w}x{h}: only a squarish window gives up the target panel");
                }
                keep_clear.extend([ok_disc(&s), cancel_seat(&s), stick_seat(&s)]);
                keep_clear.extend((0..RING_PLATES.len()).map(|i| plate_seat(&s, i)));
                for (name, t) in [("SAVE", sh), ("MAP", mh)] {
                    for o in &keep_clear {
                        assert!(!t.intersects(o), "{w}x{h} {mw}: {name} target {t:?} on {o:?}");
                    }
                }
            }
        }
        // The rooms the user saw (S25 map 135, 20 tiles; the S9 too): SAVE clears the panel there
        // now, narrower than on a wide map but no less than three fifths of a plate.
        for (w, h) in [(2340, 1080), (2220, 1080)] {
            let s = Screen::for_window(w, h);
            let room = hz_java::hooks::FIGHT_MAP_MIN_W;
            let target = target_box(&s, target_dodge(&s, minimap_panel(&s), room), room);
            let save = save_tab(&s, room);
            assert!(!top_tab_hit(&s, save).intersects(&target), "{w}x{h}: {save:?} vs {target:?}");
            assert!(save.w >= plate_seat(&s, 0).w * 3 / 5 && save.w <= save_tab(&s, s.guest_w).w, "{w}x{h}: {save:?}");
        }
        // The devices: hero2's full plate on both now that the field has no top chrome.
        for (w, h) in [(2340, 1080), (2220, 1080)] {
            let s = Screen::for_window(w, h);
            assert_eq!(save_tab(&s, s.guest_w).w, plate_seat(&s, 0).w, "{w}x{h}");
            assert!(top_tab_hit(&s, map_tab(&s, s.guest_w)).x > status_box(&s).right(), "{w}x{h}");
        }
        let s25 = Screen::for_window(2340, 1080);
        assert!(save_tab(&s25, s25.guest_w).right() < target_box(&s25, 69, s25.guest_w).x);
    }

    #[test]
    fn the_tabs_are_for_the_bare_field_and_answer_beyond_their_faces() {
        let s = Screen::for_window(2340, 1080);
        let v = |scene, popup, notice| GuestView { scene, popup, notice, ..Default::default() };
        assert_eq!(top_tabs(&s, v(Scene::Field, false, false), false, s.guest_w), vec![(save_tab(&s, s.guest_w), Tab::Save)], "no world map yet");
        assert_eq!(top_tabs(&s, v(Scene::Field, false, false), true, s.guest_w).len(), 2);
        for view in [v(Scene::Menu, false, false), v(Scene::Field, true, false), v(Scene::Field, false, true), v(Scene::Title, false, false)] {
            assert!(top_tabs(&s, view, true, s.guest_w).is_empty(), "{view:?}");
        }
        let mut l = layout(s, &Table::default(), v(Scene::Field, false, false));
        l.tabs = top_tabs(&s, v(Scene::Field, false, false), true, s.guest_w);
        let (save, map) = (save_tab(&s, s.guest_w), map_tab(&s, s.guest_w));
        // Below the face and a quarter tab beside it still count.
        assert_eq!(hit(&l, f64::from(save.centre().0), f64::from(save.bottom() + save.h / 2)), Hit::Tab(Tab::Save));
        assert_eq!(hit(&l, f64::from(save.x - save.w / 8), 2.0), Hit::Tab(Tab::Save));
        assert_eq!(hit(&l, f64::from(map.centre().0), f64::from(map.centre().1)), Hit::Tab(Tab::Map));
        assert!(crate::font::drawable(Tab::Save.label()) && crate::font::drawable(Tab::Map.label()));
    }

    /// CLR is RSK everywhere — the field's menu (the game takes −7 there as −6) and "back" in a window —
    /// since START's LSK turns the plates' page in the field (2026-10-30).
    #[test]
    fn cancel_opens_the_menu_in_the_field_and_goes_back_elsewhere() {
        let v = |scene, popup| GuestView { scene, popup, notice: false, ..Default::default() };
        for view in [v(Scene::Field, false), v(Scene::Field, true), v(Scene::Menu, false), v(Scene::Title, false), v(Scene::Unknown, false)] {
            assert_eq!(cancel_key(view), KeyCode::RIGHT_SOFT_KEY, "{view:?}");
        }
        assert_eq!((Action::Page.pad_key(), Action::Plate(6).pad_key()), (Some(KeyCode::LEFT_SOFT_KEY), Some(KeyCode::STAR)), "START, R3");
        let s = Screen::for_window(2220, 1080);
        let field = layout(s, &Table::default(), v(Scene::Field, false));
        let clr = field.controls.iter().find(|c| c.id == Some(CtrlId::Cancel)).expect("CLR is laid out");
        assert_eq!((clr.action, clr.label), (Action::Key(KeyCode::RIGHT_SOFT_KEY), "CLR"));
        let (x, y) = clr.shape.bounds().centre();
        assert!(matches!(hit(&field, f64::from(x), f64::from(y)), Hit::Control(i) if field.controls[i].id == Some(CtrlId::Cancel)));
    }

    /// HZ-024: the windows' corners hold no soft-key picture any more, and no target — a tap there is
    /// the window's (or the arrows', bottom-left); the window's answer is OK, or LSK on the one
    /// window that takes only the soft keys.
    #[test]
    fn the_window_corners_are_the_windows_and_ok_answers_the_soft_key_question() {
        let s = Screen::for_window(2220, 1080);
        let menu = layout(s, &Table::default(), GuestView { scene: Scene::Menu, ..Default::default() });
        assert_eq!(hit(&menu, f64::from(s.w) - 5.0, f64::from(s.h) - 5.0), Hit::Nothing, "bottom-right: the window's");
        let ok = |view: GuestView| layout(s, &Table::default(), view).controls.iter().find(|c| c.id == Some(CtrlId::Ok)).map(|c| c.action);
        assert_eq!(ok(GuestView { scene: Scene::Menu, ..Default::default() }), Some(Action::Key(KeyCode::OK)));
        assert_eq!(ok(GuestView { scene: Scene::Menu, lsk_ok: true, ..Default::default() }), Some(Action::Key(KeyCode::LEFT_SOFT_KEY)));
        assert_eq!(ok_key(GuestView { lsk_ok: true, ..Default::default() }), KeyCode::LEFT_SOFT_KEY);
    }
    /// B6: the field's controls a pad button can be given are the screen's targets where they are
    /// played, the SAVE and MAP tabs too — back and the menu on CLR, the page on its button, `*` on the
    /// seventh plate (2026-10-30); a plate switched off is a cell in the window, which keeps clear of
    /// every control and inside the screen, in both orientations.
    #[test]
    fn the_pad_screen_offers_the_field_controls_and_cells_for_the_rest() {
        use crate::gamepad::{PadAction, REMAP_ROWS, remap_row};
        let row = |k: KeyCode| remap_row(PadAction::Key(k)).unwrap();
        for (w, h) in WINDOWS.into_iter().chain([(1080, 2220)]) {
            let s = if w > h { Screen::for_window(w, h) } else { Screen::new(w, h, 240, 320) };
            let mut t = Table::default();
            assert!(t.hide(CtrlId::Ring(1)));
            let sc = remap_scene(&s, &t);
            let rows: Vec<usize> = sc.targets.iter().map(|t| t.row).collect();
            assert_eq!(rows.len(), REMAP_ROWS.len(), "{w}x{h}: every function once: {rows:?}");
            let p = &sc.panel;
            assert!(p.panel.inside(&s.window()), "{w}x{h}: {p:?}");
            let chips: Vec<usize> = sc.targets.iter().filter(|t| t.shape == TargetShape::Chip).map(|t| t.row).collect();
            // Both orientations alike since portrait's keypad went (2026-10-29).
            assert_eq!(chips, vec![row(KeyCode::NUM7)], "{w}x{h}");
            // OK and the menu (CLR in the field) are on screen, where the field has them.
            let ok = sc.targets.iter().find(|t| t.row == row(KeyCode::OK)).unwrap();
            assert_eq!(ok.rect, placement::builtin(CtrlId::Ok, &s), "{w}x{h}");
            assert!(sc.layout.stick.is_none() && sc.layout.dpad.is_none());
            assert_eq!(sc.layout.gear, None, "{w}x{h}");
            // The tabs, where the field hangs them.
            let save = sc.targets.iter().find(|t| t.row == remap_row(PadAction::Save).unwrap()).unwrap();
            assert_eq!((save.rect, save.shape), (save_tab(&s, s.guest_w), TargetShape::Square), "{w}x{h}");
            assert!(sc.targets.iter().any(|t| t.row == remap_row(PadAction::Map).unwrap() && t.rect == map_tab(&s, s.guest_w)));
            for t in sc.targets.iter().filter(|t| t.shape != TargetShape::Chip) {
                assert!(!t.rect.intersects(&p.panel), "{w}x{h}: the window covers {t:?}");
                assert_eq!(remap_hit(&sc, f64::from(t.rect.centre().0), f64::from(t.rect.centre().1)), Some(RemapHit::Row(t.row)));
            }
            for (r, want) in [(p.cancel, RemapHit::Cancel), (p.save, RemapHit::Save), (p.reset, RemapHit::Reset), (p.status, RemapHit::Panel)] {
                assert!(r.inside(&p.body), "{w}x{h}: {want:?}");
                assert_eq!(remap_hit(&sc, f64::from(r.centre().0), f64::from(r.centre().1)), Some(want));
            }
            for c in &p.chips {
                assert!(c.inside(&p.body) && !c.intersects(&p.status) && !c.intersects(&p.reset), "{w}x{h}: {c:?}");
            }
            assert_eq!(remap_hit(&sc, 1.0, f64::from(s.h - 2)), None, "{w}x{h}: the picture");
        }
    }

    /// B2: hero2's window — 74 % × 72 % centred, 닫기 in the header, five rows at a fixed pitch and
    /// the rest scrolled to; a cut row answers only inside the list, a dim one swallows its tap.
    #[test]
    fn the_settings_window_is_hero2s_and_scrolls_past_five_rows() {
        use SettingsAction as A;
        let all = [(A::LayoutEdit, false), (A::Diag, true), (A::HeatSaver, true), (A::SfxLevel, true), (A::Stats, true), (A::PadHud, true), (A::PadRemap, true)];
        let at = |r: Rect| (f64::from(r.centre().0), f64::from(r.centre().1));
        for s in [Screen::for_window(2220, 1080), Screen::for_window(1080, 2220), Screen::for_window(1280, 720), Screen::for_window(640, 360)] {
            let (p, g, close) = (settings_panel(&s), settings_geom(&s), settings_close(&s));
            assert!(p.inside(&s.window()) && (p.centre().0 - s.w / 2).abs() <= 1 && (p.centre().1 - s.h / 2).abs() <= 1, "{p:?} in {}x{}", s.w, s.h);
            if s.w >= 487 && s.h >= 473 {
                assert_eq!((p.w, p.h), (s.w * 74 / 100, s.h * 72 / 100));
            }
            assert!(g.view.inside(&p) && close.inside(&p) && close.bottom() <= g.view.y, "닫기 in the header");
            // Four rows: no scroll, every plate whole in the list.
            let four = settings_rows(&s, &all[..4], 0);
            assert_eq!(four.len(), 4);
            assert_eq!(settings_scroll_max(&s, 4), 0);
            assert!(four.iter().all(|(r, _, _)| r.inside(&g.view) && r.w == g.bw && r.h == g.plate));
            // Seven: five at the top, the last two at the bottom, a cut one in between.
            assert_eq!(settings_scroll_max(&s, 7), 2 * g.pitch);
            let top = settings_rows(&s, &all, 0);
            assert_eq!(top.iter().map(|r| r.1).collect::<Vec<_>>(), all[..5].iter().map(|r| r.0).collect::<Vec<_>>());
            let end = settings_rows(&s, &all, 99_999);
            assert_eq!(end.len(), 5);
            assert!(end.iter().all(|(r, _, _)| r.inside(&g.view)) && end[4].1 == A::PadRemap);
            let half = settings_rows(&s, &all, g.pitch / 2);
            assert_eq!(half.len(), 6, "a plate the list's edge cuts is still listed");
            // Taps: 닫기 and outside close, a row acts, a dim row and the panel's margin do nothing,
            // the cut part of a plate (under the header) does nothing.
            assert_eq!(settings_hit(&s, &all, 0, 0.0, 0.0), Some(A::Close), "outside the window");
            assert_eq!(settings_hit(&s, &all, 0, at(close).0, at(close).1), Some(A::Close));
            let (dx, dy) = at(top[1].0);
            assert_eq!(settings_hit(&s, &all, 0, dx, dy), Some(A::Diag));
            let (lx, ly) = at(top[0].0);
            assert_eq!(settings_hit(&s, &all, 0, lx, ly), None, "LAYOUT is dim here: swallowed");
            assert_eq!(settings_hit(&s, &all, 0, f64::from(p.x + 2), f64::from(p.bottom() - 2)), None);
            let cut = half.iter().find(|(r, _, _)| r.y < g.view.y).unwrap().0;
            assert_eq!(settings_hit(&s, &all, g.pitch / 2, f64::from(cut.centre().0), f64::from(cut.y + 1)), None);
        }
    }

    #[test]
    fn the_hit_chain_order() {
        let s = Screen::for_window(2220, 1080);
        let at = |r: Rect| (f64::from(r.centre().0), f64::from(r.centre().1));
        // A window: the gear, the arrows, the arrows' dead centre, OK.
        let l = layout(s, &Table::default(), GuestView { scene: Scene::Menu, ..Default::default() });
        let g = at(l.gear.unwrap());
        assert_eq!(hit(&l, g.0, g.1), Hit::Gear);
        let sq = l.dpad.unwrap();
        let (cx, cy) = at(sq);
        assert_eq!(hit(&l, cx, cy), Hit::Dead, "between the arrows");
        let up = hit(&l, cx, f64::from(sq.y) + 5.0);
        assert!(matches!(up, Hit::Control(i) if l.controls[i].action == Action::Key(KeyCode::UP)), "{up:?}");
        let o = at(ok_disc(&s));
        assert!(matches!(hit(&l, o.0, o.1), Hit::Control(i) if l.controls[i].action == Action::Key(KeyCode::OK)));
        assert_eq!(hit(&l, 1110.0, 500.0), Hit::Nothing, "the middle of the picture is the window's");
        // The field: the stick, OK; no gear.
        let f = layout(s, &Table::default(), GuestView { scene: Scene::Field, ..Default::default() });
        assert!(f.gear.is_none() && f.dpad.is_none());
        let st = at(f.stick.unwrap().seat);
        assert_eq!(hit(&f, st.0, st.1), Hit::StickZone);
        assert_ne!(hit(&f, g.0, g.1), Hit::Gear);
    }
    #[test]
    fn the_stick_zone_is_its_side_of_the_screen_not_the_window() {
        let s = Screen::for_window(2220, 1080);
        let st = layout(s, &Table::default(), GuestView::default()).stick.unwrap();
        assert!(st.zone.contains(f64::from(st.seat.centre().0), f64::from(st.seat.centre().1)));
        assert!(st.zone.right() <= s.w / 2);
        assert!(st.zone.y > status_box(&s).bottom(), "the status block is not the stick");
        assert_eq!(st.zone.bottom(), s.h, "down to the window's bottom");
        assert!((st.engage_r * 0.30 - 27.0).abs() < 1.0, "walks after ~27 px at 100 %");
    }
    /// The control sets (hero2's modes): the field's stick, OK, CLR and plates; a window's arrows,
    /// OK, CLR and gear (not over the game's minimap window, whose band runs under it); a
    /// dialogue's nothing but the ▶▶. The plates are for play — the field and the game's windows
    /// over it (A7: hero2's plates stay over its menus) — not the title, a scenario or loading; `#`
    /// also on the 「# 확인」 notice, in any set. Until the game says where it is, the field's — with
    /// the gear.
    #[test]
    fn the_controls_follow_the_scene() {
        let s = Screen::for_window(2220, 1080);
        let view = |scene, popup, notice, window| GuestView { scene, popup, notice, window, ..Default::default() };
        let plates = |v: GuestView| -> Vec<u8> {
            layout(s, &Table::default(), v)
                .controls
                .iter()
                .filter_map(|c| match (c.id, c.action) {
                    (Some(CtrlId::Ring(_)), Action::Plate(n)) => Some(n),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(plates(GuestView::default()).len(), RING_PLATES.len(), "unknown: all seven");
        let seven: Vec<u8> = RING_PLATES.iter().map(|k| k.0).collect();
        let page = |v: GuestView| layout(s, &Table::default(), v).controls.iter().any(|c| c.action == Action::Page);
        for v in [view(Scene::Field, false, false, false), view(Scene::Field, true, false, false), view(Scene::Menu, false, false, false), view(Scene::MapWindow, false, false, false)] {
            assert_eq!(plates(v), seven, "{v:?}: in play, over the game's windows too");
            assert!(page(v), "{v:?}: and the page button");
        }
        for scene in [Scene::Loading, Scene::Title, Scene::Script] {
            assert!(plates(view(scene, false, false, false)).is_empty() && !page(view(scene, false, false, false)), "{scene:?}");
            assert!(plates(view(scene, false, true, false)).is_empty(), "{scene:?} with the notice: OK is its #");
        }
        assert_eq!(ok_key(view(Scene::Field, false, true, false)), KeyCode::HASH, "the 「# 확인」 notice's key");
        assert!(plates(view(Scene::Script, false, false, true)).is_empty(), "a scenario's choices: none");
        // Over a window the plates keep the field's seats, and a tap on one is that plate's — not
        // the gear's, an arrow's, the dead centre's, OK's or CLR's.
        let field = layout(s, &Table::default(), view(Scene::Field, false, false, false));
        let menu = layout(s, &Table::default(), view(Scene::Menu, false, false, false));
        let seats = |l: &Layout| l.controls.iter().filter(|c| matches!(c.id, Some(CtrlId::Ring(_)))).map(|c| c.shape.bounds()).collect::<Vec<_>>();
        assert_eq!(seats(&menu), seats(&field));
        for (i, c) in menu.controls.iter().enumerate().filter(|(_, c)| matches!(c.id, Some(CtrlId::Ring(_)))) {
            let (x, y) = c.shape.bounds().centre();
            assert_eq!(hit(&menu, f64::from(x), f64::from(y)), Hit::Control(i), "{:?}", c.id);
        }
        let arrows = |l: &Layout| l.controls.iter().filter(|c| matches!(c.shape, Shape::Wedge { .. })).count();
        let has = |l: &Layout, k: KeyCode| l.controls.iter().any(|c| c.action == Action::Key(k) && c.face != Face::Hidden);
        assert_eq!(control_set(view(Scene::Field, false, false, false)), ControlSet::Field);
        let field = layout(s, &Table::default(), view(Scene::Field, false, false, false));
        assert!(field.stick.is_some() && arrows(&field) == 0 && field.gear.is_none());
        assert!(has(&field, KeyCode::OK) && has(&field, KeyCode::RIGHT_SOFT_KEY), "OK and CLR (the menu)");
        for v in [view(Scene::Menu, false, false, false), view(Scene::Title, false, false, false), view(Scene::Field, true, false, false), view(Scene::Script, false, false, true), view(Scene::Loading, false, false, false)] {
            assert_eq!(control_set(v), ControlSet::Menu, "{v:?}");
            let l = layout(s, &Table::default(), v);
            assert!(l.stick.is_none() && arrows(&l) == 4 && l.dpad.is_some() && l.gear.is_some(), "{v:?}");
            assert!(has(&l, KeyCode::OK) && has(&l, KeyCode::RIGHT_SOFT_KEY), "{v:?}: OK and CLR (back)");
        }
        let window = layout(s, &Table::default(), view(Scene::MapWindow, false, false, false));
        assert!(window.gear.is_none() && arrows(&window) == 4, "the minimap window's band runs under the gear");
        let cutscene = layout(s, &Table::default(), view(Scene::Script, false, false, false));
        assert_eq!(control_set(view(Scene::Script, false, false, false)), ControlSet::Cutscene);
        assert!(cutscene.stick.is_none() && cutscene.gear.is_none() && cutscene.dpad.is_none());
        assert_eq!(cutscene.controls.len(), 1, "only the skip");
        assert_eq!((cutscene.controls[0].action, cutscene.controls[0].face), (Action::Key(KeyCode::RIGHT_SOFT_KEY), Face::Hidden));
        let unknown = layout(s, &Table::default(), GuestView::default());
        assert!(unknown.stick.is_some() && unknown.gear.is_some());
    }

    /// A8: the status block's tap target holds the block, takes no control's touch and stops where
    /// the stick's zone starts; the minimap's restore button is a ring seat (hero2's) with a target
    /// a quarter bigger each way; the pad leaves no walk control in either set, 십자키 puts a
    /// window's arrows on the field's stick seat.
    #[test]
    fn the_status_target_the_restore_button_and_the_walk() {
        let field = GuestView { scene: Scene::Field, ..Default::default() };
        let menu = GuestView { scene: Scene::Menu, ..Default::default() };
        let wedges = |l: &Layout| l.controls.iter().filter(|c| matches!(c.shape, Shape::Wedge { .. })).count();
        let plates = |l: &Layout| l.controls.iter().filter(|c| matches!(c.id, Some(CtrlId::Ring(_)))).count();
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let l = layout(s, &Table::default(), field);
            let st = l.status.expect("the field's");
            assert!(status_box(&s).inside(&st), "{w}x{h}");
            assert!(!st.intersects(&l.stick.unwrap().zone), "{w}x{h}: not the stick's");
            for (i, c) in l.controls.iter().enumerate() {
                let (x, y) = c.shape.bounds().centre();
                assert_eq!(hit(&l, f64::from(x), f64::from(y)), Hit::Control(i), "{w}x{h}: {:?}", c.id);
            }
            let (x, y) = status_box(&s).centre();
            assert_eq!(hit(&l, f64::from(x), f64::from(y)), Hit::Status, "{w}x{h}");
            assert!(layout(s, &Table::default(), menu).status.is_none(), "{w}x{h}: the field's only");
            let panel = minimap_panel(&s);
            let b = minimap_restore(&s, panel);
            assert_eq!(b.w, ring(&s)[0].w.min(panel.w), "{w}x{h}: a ring seat");
            let t = minimap_restore_hit(&s, b);
            assert!(b.inside(&t) && t.inside(&s.window()) && t.w > b.w, "{w}x{h}");
            let pad = layout_for(s, &Table::default(), field, Walk::Pad);
            assert!(pad.stick.is_none() && pad.dpad.is_none() && wedges(&pad) == 0, "{w}x{h}: the pad walks");
            assert_eq!(plates(&pad), RING_PLATES.len(), "{w}x{h}: the plates stay");
            let pad = layout_for(s, &Table::default(), menu, Walk::Pad);
            assert!(pad.dpad.is_none() && wedges(&pad) == 0, "{w}x{h}: a window's arrows too");
            let arrows = layout_for(s, &Table::default(), field, Walk::Arrows);
            assert!(arrows.stick.is_none() && wedges(&arrows) == 4 && arrows.dpad == Some(l.stick.unwrap().seat), "{w}x{h}: 십자키");
            assert_eq!(control_set(field), ControlSet::Field);
            assert!(arrows.gear.is_none() && arrows.status.is_some(), "{w}x{h}: still the field's set");
        }
    }

    /// The four arrows tile their square: every point is one arrow's or the dead centre's, never
    /// two, and each hexagon is inside its own quarter — the ink a subset of the target.
    #[test]
    fn the_arrows_tile_their_square() {
        let sq = Rect::new(36, 420, 624, 624);
        let arms = dpad_controls(sq);
        assert_eq!(arms.iter().map(|c| c.action).collect::<Vec<_>>(), [KeyCode::UP, KeyCode::DOWN, KeyCode::LEFT, KeyCode::RIGHT].map(Action::Key));
        let (cx, cy) = (f64::from(sq.x) + 312.0, f64::from(sq.y) + 312.0);
        for y in (sq.y..sq.bottom()).step_by(7) {
            for x in (sq.x..sq.right()).step_by(7) {
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let n = arms.iter().filter(|c| c.shape.contains(fx, fy, 30.0)).count();
                let dead = (fx - cx).hypot(fy - cy) < 624.0 * f64::from(DEAD_PCT) / 100.0;
                assert_eq!(n, usize::from(!dead), "({x}, {y})");
            }
        }
        for c in &arms {
            let (Face::Hex(ink), Action::Key(k)) = (c.face, c.action) else { panic!("{c:?}") };
            for (vx, vy) in hexagon(ink, k) {
                let (vx, vy) = (vx.clamp(f64::from(ink.x) + 0.5, f64::from(ink.right()) - 0.5), vy.clamp(f64::from(ink.y) + 0.5, f64::from(ink.bottom()) - 0.5));
                assert!(c.shape.contains(vx, vy, 0.0), "{k:?}: corner ({vx}, {vy}) of {ink:?}");
            }
        }
        assert!(!arms[0].shape.contains(cx, f64::from(sq.y) - 1.0, 30.0), "nothing outside the square");
    }

    /// The ▶▶ target holds the game's ink (`/ff.png` 10×7 at the canvas's bottom-right, 1 px in) and
    /// runs out to the picture's corner.
    #[test]
    fn the_skip_target_holds_the_games_mark() {
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let t = skip_hint(&s);
            let ink = Rect::new(s.gx(s.guest_w - 11), s.gy(s.guest_h - 8), s.gx(s.guest_w - 1) - s.gx(s.guest_w - 11), s.gy(s.guest_h - 1) - s.gy(s.guest_h - 8));
            assert!(ink.inside(&t), "{w}x{h}: {ink:?} in {t:?}");
            assert_eq!((t.right(), t.bottom()), (s.game.right(), s.game.bottom()), "{w}x{h}");
            assert!(t.w >= s.h / 10 && t.h >= s.h / 10, "{w}x{h}: a thumb's size: {t:?}");
        }
    }
    /// Portrait's built-ins (2026-10-29): every control and both tabs in the strip under the picture,
    /// in the window, clear of each other, of the gear and of the editor's row by a pad — the stick
    /// left of the plates, the plates left of OK and CLR, CLR over OK — on the phones' windows; the
    /// layout is landscape's (the field's set: the stick, OK, CLR, the six plates) with them seated.
    #[test]
    fn portrait_seats_are_in_the_strip_and_apart() {
        for (w, h) in [(1080u32, 2220u32), (1440, 3120), (1080, 1920), (1080, 2400)] {
            let s = Screen::new(w, h, 240, 320);
            let p = portrait_seats(&s);
            let strip = s.game.bottom();
            let mut all = vec![("stick", p.stick), ("ok", p.ok), ("clr", p.cancel), ("save", p.save), ("map", p.map), ("gear", p.gear)];
            all.extend(p.plates.iter().map(|r| ("plate", *r)));
            for (i, (n, r)) in all.iter().enumerate() {
                assert!(r.y >= strip && r.inside(&s.window()) && !r.is_empty(), "{w}x{h}: {n} {r:?}");
                for (m, q) in &all[i + 1..] {
                    assert!(!r.intersects(q), "{w}x{h}: {n} {r:?} on {m} {q:?}");
                }
            }
            for (n, r) in all.iter().filter(|(n, _)| !matches!(*n, "save" | "map" | "gear")) {
                for o in obstacles(&s) {
                    assert!(!r.grown(s.pad()).intersects(&o), "{w}x{h}: {n} {r:?} on obstacle {o:?}");
                }
            }
            let plates_x = (p.plates.iter().map(|r| r.x).min().unwrap(), p.plates.iter().map(|r| r.right()).max().unwrap());
            assert!(p.stick.right() < plates_x.0 && plates_x.1 < p.ok.x.min(p.cancel.x), "{w}x{h}: stick | plates | OK");
            assert!(p.cancel.bottom() < p.ok.y && p.map.x < p.gear.x && p.gear.right() < p.save.x, "{w}x{h}");
            assert!(p.plates.iter().all(|r| r.w == r.h && r.w == p.plates[0].w), "{w}x{h}: one plate size");
            let l = layout(s, &Table::default(), GuestView { scene: Scene::Field, ..Default::default() });
            assert!(l.stick.is_some_and(|st| st.seat == p.stick) && l.dpad.is_none());
            assert_eq!(l.controls.len(), 3 + RING_PLATES.len(), "{w}x{h}: OK, CLR, the plates and the page button");
            assert!(p.page.w == p.plates[0].w && !p.plates.iter().any(|r| r.intersects(&p.page)), "{w}x{h}: the page button, a plate's size, beside them");
            assert!(l.gear.is_some(), "the gear stays in the strip");
        }
    }

    /// Portrait's tabs (2026-10-29): in the strip's top corners, MAP left of SAVE, answering a pad
    /// beyond their faces — not hanging from the picture's top any more, which keeps the status
    /// block, the minimap and the target panel under it to themselves.
    #[test]
    fn portrait_tabs_are_in_the_strip() {
        for (w, h) in [(1080u32, 2220u32), (1440, 3120)] {
            let s = Screen::new(w, h, 240, 320);
            let (save, map) = top_tab_seats(&s, s.guest_w);
            assert!(save.y == s.game.bottom() && map.y == save.y && map.right() < save.x, "{w}x{h}: {save:?} {map:?}");
            assert_eq!(top_tab_hit(&s, save), save.grown(s.pad()));
            let field = GuestView { scene: Scene::Field, ..Default::default() };
            assert_eq!(top_tabs(&s, field, true, s.guest_w), vec![(save, Tab::Save), (map, Tab::Map)]);
            assert!(top_tabs(&s, GuestView { scene: Scene::Menu, ..Default::default() }, true, s.guest_w).is_empty(), "the bare field's");
            for r in [save, map] {
                assert!(!r.intersects(&s.game) && !r.intersects(&minimap_panel(&s)));
            }
        }
    }

    /// HZ-054: in portrait the target panel keeps the game's own place at the top and the minimap's
    /// built-in seat is under it (user 2026-10-29, second word); a minimap the player moves takes the
    /// panel along over it, right edges together — under it when put at the very top — and the
    /// editor takes no seat that leaves it nowhere.
    #[test]
    fn the_portrait_target_panel_sits_over_the_minimap() {
        for (w, h) in [(1080u32, 2220u32), (1440, 3120), (1080, 1920)] {
            let s = Screen::new(w, h, 240, 320);
            let panel = minimap_panel(&s);
            let (shift, drop) = target_place(&s, panel, s.guest_w);
            assert_eq!(drop, 0, "{w}x{h}: the game's own height");
            let t = target_box_at(&s, shift, drop, s.guest_w);
            assert!(t.bottom() <= panel.y && panel.y - t.bottom() <= guest_unit(&s) * 4, "{w}x{h}: {t:?} over {panel:?}");
            assert!((t.right() - panel.right()).abs() <= guest_unit(&s) * 2, "{w}x{h}: {t:?} / {panel:?}");
            assert!(t.inside(&s.game) && !t.intersects(&status_box(&s)), "{w}x{h}");
            assert!(minimap_seat_ok(&s, panel), "{w}x{h}");
        }
        let s = Screen::new(1080, 2220, 240, 320);
        let panel = minimap_panel(&s);
        let low = Rect::new(panel.x, s.game.bottom() - panel.h - s.pad(), panel.w, panel.h);
        let (shift, drop) = target_place(&s, low, s.guest_w);
        let t = target_box_at(&s, shift, drop, s.guest_w);
        assert!(t.bottom() <= low.y && low.y - t.bottom() <= guest_unit(&s) * 4 && t.inside(&s.game), "over a panel put low: {t:?}");
        let high = Rect::new(panel.x, s.game.y + s.pad() * 2, panel.w, panel.h);
        let (shift, drop) = target_place(&s, high, s.guest_w);
        let t = target_box_at(&s, shift, drop, s.guest_w);
        assert!(t.y >= high.bottom() && t.inside(&s.game), "under a panel at the very top: {t:?}");
        assert!(minimap_seat_ok(&s, low) && minimap_seat_ok(&s, high));
        assert!(!minimap_seat_ok(&s, Rect::new(s.game.x, s.game.y, panel.w, panel.h)), "on the status block");
        // Landscape: where it was (HZ-014's shift, no drop).
        let l = Screen::for_window(2340, 1080);
        assert_eq!(target_place(&l, minimap_panel(&l), l.guest_w), (target_dodge(&l, minimap_panel(&l), l.guest_w), 0));
    }

    #[test]
    fn joy_dir_dead_zones_and_turn_ratio() {
        let r = 100.0;
        assert_eq!(joy_dir(25.0, 0.0, r, None), None, "inside the engage zone");
        assert_eq!(joy_dir(31.0, 0.0, r, None), Some(KeyCode::RIGHT));
        assert_eq!(
            joy_dir(20.0, 0.0, r, Some(KeyCode::RIGHT)),
            Some(KeyCode::RIGHT),
            "kept above the keep zone"
        );
        assert_eq!(joy_dir(10.0, 0.0, r, Some(KeyCode::RIGHT)), None);
        assert_eq!(joy_dir(50.0, 55.0, r, Some(KeyCode::RIGHT)), Some(KeyCode::RIGHT), "not a decisive turn");
        assert_eq!(joy_dir(50.0, 65.0, r, Some(KeyCode::RIGHT)), Some(KeyCode::DOWN));
        assert_eq!(joy_dir(0.0, -40.0, r, None), Some(KeyCode::UP));
        assert_eq!(joy_dir(-40.0, 0.0, r, None), Some(KeyCode::LEFT));
    }

    #[test]
    fn joy_way_has_eight_sectors_that_hold_on_their_borders() {
        use KeyCode::*;
        let r = 100.0;
        let at = |deg: f64, active, eight| {
            let a = f64::to_radians(deg);
            joy_way(60.0 * a.cos(), 60.0 * a.sin(), r, active, eight)
        };
        // Screen degrees: y grows down, so 90° is ↓.
        let want = [(0.0, Way::One(RIGHT)), (45.0, Way::Two(RIGHT, DOWN)), (90.0, Way::One(DOWN)), (135.0, Way::Two(LEFT, DOWN)), (180.0, Way::One(LEFT)), (225.0, Way::Two(LEFT, UP)), (270.0, Way::One(UP)), (315.0, Way::Two(RIGHT, UP))];
        for (deg, w) in want {
            assert_eq!(at(deg, None, true), Some(w), "{deg}°");
            assert_eq!(at(deg + 20.0, None, true), Some(w), "{deg}° + 20");
        }
        assert_eq!(at(24.0, None, true), Some(Way::Two(RIGHT, DOWN)), "past the border");
        assert_eq!(at(28.0, Some(Way::One(RIGHT)), true), Some(Way::One(RIGHT)), "held within the margin");
        assert_eq!(at(31.0, Some(Way::One(RIGHT)), true), Some(Way::Two(RIGHT, DOWN)), "beyond it");
        assert_eq!(at(17.0, Some(Way::Two(RIGHT, DOWN)), true), Some(Way::Two(RIGHT, DOWN)));
        assert_eq!(at(14.0, Some(Way::Two(RIGHT, DOWN)), true), Some(Way::One(RIGHT)));
        assert_eq!(joy_way(25.0, 0.0, r, None, true), None, "the engage zone");
        assert_eq!(joy_way(20.0, 0.0, r, Some(Way::One(RIGHT)), true), Some(Way::One(RIGHT)), "the keep zone");
        // Four ways: joy_dir's, a held diagonal as its horizontal arrow.
        assert_eq!(at(40.0, None, false), Some(Way::One(RIGHT)));
        assert_eq!(at(48.0, Some(Way::Two(RIGHT, DOWN)), false), Some(Way::One(RIGHT)), "joy_dir's turn ratio");
    }

    #[test]
    fn way_edges_lift_then_press() {
        use KeyCode::*;
        assert_eq!(way_edges(None, Some(Way::Two(RIGHT, DOWN))), vec![(RIGHT, true), (DOWN, true)]);
        assert_eq!(way_edges(Some(Way::One(RIGHT)), Some(Way::Two(RIGHT, DOWN))), vec![(DOWN, true)]);
        assert_eq!(way_edges(Some(Way::Two(RIGHT, DOWN)), Some(Way::Two(LEFT, DOWN))), vec![(RIGHT, false), (LEFT, true)]);
        assert_eq!(way_edges(Some(Way::Two(RIGHT, DOWN)), Some(Way::One(DOWN))), vec![(RIGHT, false)]);
        assert_eq!(way_edges(Some(Way::Two(RIGHT, DOWN)), None), vec![(RIGHT, false), (DOWN, false)]);
        assert_eq!(way_edges(Some(Way::One(UP)), Some(Way::One(UP))), vec![]);
    }

    #[test]
    fn dpad_corners_are_the_gaps_between_arms() {
        use KeyCode::*;
        let sq = Rect::new(0, 0, 200, 200);
        let at = |deg: f64, d: f64| {
            let a = f64::to_radians(deg);
            dpad_corner(sq, 100.0 + d * a.cos(), 100.0 + d * a.sin())
        };
        assert_eq!(at(45.0, 80.0), Some((RIGHT, DOWN)));
        assert_eq!(at(135.0, 80.0), Some((LEFT, DOWN)));
        assert_eq!(at(225.0, 80.0), Some((LEFT, UP)));
        assert_eq!(at(315.0, 80.0), Some((RIGHT, UP)));
        assert_eq!(at(45.0 - DPAD_CORNER_DEG + 1.0, 80.0), Some((RIGHT, DOWN)));
        assert_eq!(at(45.0 - DPAD_CORNER_DEG - 1.0, 80.0), None, "on the → arm");
        assert_eq!(at(0.0, 80.0), None);
        assert_eq!(at(45.0, 5.0), None, "the dead disc");
        assert_eq!(dpad_corner(sq, 250.0, 250.0), None, "off the pad");
    }

    #[test]
    fn labels_are_drawable() {
        for (_, label, _) in RING_PLATES {
            assert!(crate::font::drawable(label));
        }
        for a in EDIT_ACTIONS {
            assert!(crate::font::drawable(a.label()));
        }
    }

    /// A9: hero2's seven buttons in the old row's footprint — the obstacle a stored layout was
    /// checked against does not move, on any window.
    #[test]
    fn the_editor_row_holds_seven_in_the_old_footprint() {
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let row = editor_chrome(&s);
            assert_eq!(row.iter().map(|(_, a)| *a).collect::<Vec<_>>(), EDIT_ACTIONS.to_vec());
            let bh = (s.h / 16).clamp(28, 72);
            let span = 4 * (bh * 5 / 2) + 3 * s.pad();
            let (first, last) = (row[0].0, row[6].0);
            assert_eq!((first.x, last.right()), (s.w / 2 - span / 2, s.w / 2 - span / 2 + span), "{w}x{h}");
            for pair in row.windows(2) {
                assert!(pair[0].0.right() < pair[1].0.x, "{w}x{h}: cells apart");
            }
            assert!(row.iter().all(|(r, _)| r.y == s.pad() && r.h == bh && r.w >= 30), "{w}x{h}: {row:?}");
        }
    }
}
