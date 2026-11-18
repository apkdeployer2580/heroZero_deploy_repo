//! HZ-057 — the bosses' reflect stance (user 2026-10-29: 「상시 반격모드 … 특정 패턴시에만 반격모드가
//! 켜지도록 … 보스 주변에 뭔가 표시」; look B 「거울 코팅」 of the mock-ups).
//!
//! A boss that picks its next attack may take the stance: a warning (nothing reflected yet), the
//! reflection (its last stretch blinking), a fade. Counted in field ticks — the game's 20 a second,
//! stopped under its windows. Pure: the hooks feed [`Stance`] the picks, the ticks and the blows, and
//! draw what [`Stance::look`] says with [`coat`].

/// The warning: the outline comes in and nothing is reflected yet — time to stop swinging.
pub const WARN_TICKS: u16 = 20;
/// The reflection.
pub const REFLECT_TICKS: u16 = 80;
/// The reflection's last stretch blinks.
pub const BLINK_TICKS: u16 = 20;
/// The coat fades.
pub const FADE_TICKS: u16 = 8;
/// After a stance, before a pick may roll again.
pub const COOLDOWN_TICKS: u16 = 160;
/// Into a fight, before the first.
pub const OPENING_TICKS: u16 = 100;
/// The chance a pick takes the stance once the cooldown is over, percent. Bosses pick every ~15
/// ticks (키메라 /t/bd [9]), so a stance comes some 4 s after the cooldown on average.
pub const CHANCE_PCT: u32 = 20;
/// No boss has thought for this long: the fight is over (won, left or lost) — the next starts afresh.
pub const QUIET_TICKS: u16 = 60;
/// Ticks a reflected blow lights the coat.
pub const FLASH_TICKS: u8 = 4;
/// What of the HP a blow took comes back (a fifth, the game's own share), and the most it can be
/// (percent of the player's max HP).
pub const REFLECT_DIV: i32 = 5;
pub const REFLECT_CAP_PCT: i32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Warn,
    Reflect,
    Fade,
}

/// What the coat looks like this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// The outline coming in: `progress` 0 → 1.
    Warn { progress: f32, clock: u32 },
    /// The coat at `strength` (dim on a blink's off beat), lit for a moment by a blow sent back.
    Coat { strength: f32, flash: bool, clock: u32 },
    /// Going: `left` 1 → 0.
    Fade { left: f32, clock: u32 },
}

impl Look {
    fn clock(self) -> u32 {
        match self {
            Look::Warn { clock, .. } | Look::Coat { clock, .. } | Look::Fade { clock, .. } => clock,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Stance {
    phase: Phase,
    /// Ticks into the phase.
    t: u16,
    /// Idle: ticks before a pick may roll.
    cooldown: u16,
    /// Ticks since a boss last thought; past [`QUIET_TICKS`] there is no fight.
    quiet: u16,
    flash: u8,
    rng: u32,
    /// Ticks counted, for the coat's motion.
    clock: u32,
}

impl Default for Stance {
    fn default() -> Self {
        Self::new()
    }
}

impl Stance {
    pub fn new() -> Stance {
        Stance { phase: Phase::Idle, t: 0, cooldown: OPENING_TICKS, quiet: QUIET_TICKS + 1, flash: 0, rng: 0x9E37_79B9, clock: 0 }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether a blow on a boss comes back now.
    pub fn reflecting(&self) -> bool {
        self.phase == Phase::Reflect
    }

    /// A field tick. The phase it moved into, if it moved.
    pub fn tick(&mut self) -> Option<Phase> {
        if self.quiet > QUIET_TICKS {
            return None;
        }
        self.quiet += 1;
        if self.quiet > QUIET_TICKS {
            let was = self.phase;
            self.fight_starts();
            return (was != Phase::Idle).then_some(Phase::Idle);
        }
        self.clock = self.clock.wrapping_add(1);
        self.flash = self.flash.saturating_sub(1);
        self.t = self.t.saturating_add(1);
        let next = match self.phase {
            Phase::Idle => {
                self.cooldown = self.cooldown.saturating_sub(1);
                None
            }
            Phase::Warn if self.t >= WARN_TICKS => Some(Phase::Reflect),
            Phase::Reflect if self.t >= REFLECT_TICKS => Some(Phase::Fade),
            Phase::Fade if self.t >= FADE_TICKS => Some(Phase::Idle),
            _ => None,
        };
        if let Some(p) = next {
            self.enter(p);
        }
        next
    }

    /// A boss thought (its `k()` ran): the fight goes on — or, after a quiet, a new one begins.
    pub fn thought(&mut self) {
        if self.quiet > QUIET_TICKS {
            self.fight_starts();
        }
        self.quiet = 0;
    }

    /// A boss picked its next attack: the stance, maybe. Whether it began.
    pub fn picked(&mut self) -> bool {
        if self.phase != Phase::Idle || self.cooldown > 0 || self.quiet > QUIET_TICKS {
            return false;
        }
        if self.roll() % 100 >= CHANCE_PCT {
            return false;
        }
        self.enter(Phase::Warn);
        true
    }

    /// A blow just came back.
    pub fn hit(&mut self) {
        self.flash = FLASH_TICKS;
    }

    /// How to draw the bosses now; `None` = as the game does.
    pub fn look(&self) -> Option<Look> {
        let clock = self.clock;
        match self.phase {
            Phase::Idle => None,
            Phase::Warn => Some(Look::Warn { progress: f32::from(self.t + 1) / f32::from(WARN_TICKS), clock }),
            Phase::Reflect => {
                let left = REFLECT_TICKS.saturating_sub(self.t);
                // The last stretch: three ticks on, three dim.
                let strength = if left <= BLINK_TICKS && (left / 3) % 2 == 1 { 0.3 } else { 1.0 };
                Some(Look::Coat { strength, flash: self.flash > 0, clock })
            }
            Phase::Fade => Some(Look::Fade { left: 1.0 - f32::from(self.t) / f32::from(FADE_TICKS), clock }),
        }
    }

    /// Ticks counted, for the coat's and the badge's motion.
    pub fn clock(&self) -> u32 {
        self.clock
    }

    /// The badge's bar: the warning filling it, the reflection draining it. `None` = no badge.
    pub fn gauge(&self) -> Option<f32> {
        match self.phase {
            Phase::Warn => Some(f32::from(self.t + 1) / f32::from(WARN_TICKS)),
            Phase::Reflect => Some(f32::from(REFLECT_TICKS.saturating_sub(self.t)) / f32::from(REFLECT_TICKS)),
            Phase::Idle | Phase::Fade => None,
        }
    }

    fn fight_starts(&mut self) {
        self.phase = Phase::Idle;
        self.t = 0;
        self.cooldown = OPENING_TICKS;
        self.flash = 0;
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.t = 0;
        if phase == Phase::Idle {
            self.cooldown = COOLDOWN_TICKS;
        }
    }

    /// xorshift32: the game's own `Random` is left alone, so a fight plays the same with the patch off.
    fn roll(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }
}

/// What the player takes for a blow that took `took` HP off a boss, with `defence` and `max_hp`: a
/// [`REFLECT_DIV`]th, mitigated as a monster's hit is (`− defence`, at least 5, ÷ 3, at least 1),
/// at most [`REFLECT_CAP_PCT`] of max HP. 0 for a blow that took nothing.
pub fn reflected_hit(took: i32, defence: i32, max_hp: i32) -> i32 {
    if took <= 0 {
        return 0;
    }
    let mitigated = ((took / REFLECT_DIV - defence).max(5) / 3).max(1);
    mitigated.min((max_hp * REFLECT_CAP_PCT / 100).max(1))
}

// ------------------------------------------------------------------------------------------------
// the coat

/// The mock-up's colours: the outline, its glow, the silver the body turns.
const ICE: Rgb = Rgb(205, 252, 255);
const CYAN: Rgb = Rgb(110, 240, 255);
const SILVER: Rgb = Rgb(150, 215, 235);
const WHITE: Rgb = Rgb(255, 255, 255);

/// The sheen: a band that crosses the body along `x + 0.7 y` every [`SHEEN_PERIOD`] px, moving
/// [`SHEEN_SPEED`] px a tick.
const SHEEN_PERIOD: f32 = 96.0;
const SHEEN_SPEED: f32 = 4.0;

/// The sheen's band across one period, [`SHEEN_STEPS`] entries a pixel: a broad stroke and a thin
/// one just behind it.
const SHEEN_STEPS: f32 = 4.0;

fn sheen_band() -> &'static [f32] {
    static BAND: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    BAND.get_or_init(|| {
        let mid = SHEEN_PERIOD / 2.0;
        (0..(SHEEN_PERIOD * SHEEN_STEPS) as usize)
            .map(|i| {
                let pos = i as f32 / SHEEN_STEPS;
                0.55 * (-((pos - mid) / 3.0).powi(2)).exp() + 0.35 * (-((pos - mid - 6.0) / 1.2).powi(2)).exp()
            })
            .collect()
    })
}

/// The rings round a silhouette: the outline and two of glow.
pub const RINGS: usize = 3;

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);

/// `p` toward `c` by `a` (0 → 1), opaque. In 256ths, rounded.
fn mix(p: i32, c: Rgb, a: f32) -> i32 {
    let a = (a.clamp(0.0, 1.0) * 256.0 + 0.5) as i32;
    let ch = |s: u32, t: u8| {
        let v = (p >> s) & 0xff;
        ((v + (((i32::from(t) - v) * a + 128) >> 8)) as u32) << s
    };
    (0xff00_0000 | ch(16, c.0) | ch(8, c.1) | ch(0, c.2)) as i32
}

/// A mask a row of 64-bit words at a time — the close and the rings are whole-row shifts and ORs.
#[derive(Clone)]
struct Bits {
    w: usize,
    h: usize,
    words: usize,
    v: Vec<u64>,
}

impl Bits {
    fn new(w: usize, h: usize, at: impl Fn(usize, usize) -> bool) -> Bits {
        let words = w.div_ceil(64);
        let mut v = vec![0u64; words * h];
        for y in 0..h {
            for x in 0..w {
                if at(x, y) {
                    v[y * words + x / 64] |= 1 << (x % 64);
                }
            }
        }
        Bits { w, h, words, v }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.v[y * self.words + x / 64] >> (x % 64) & 1 != 0
    }

    fn row(&self, y: usize) -> &[u64] {
        &self.v[y * self.words..(y + 1) * self.words]
    }

    /// Bits past `w` in a row's last word cleared (a shift or a NOT may set them).
    fn trim(&mut self) {
        let spare = self.words * 64 - self.w;
        if spare == 0 {
            return;
        }
        let keep = u64::MAX >> spare;
        for y in 0..self.h {
            self.v[y * self.words + self.words - 1] &= keep;
        }
    }

    fn not(&self) -> Bits {
        let mut out = Bits { v: self.v.iter().map(|w| !w).collect(), ..*self };
        out.trim();
        out
    }

    /// One pass of growth: `cross` = by the four neighbours, else the eight. Nothing grows in from
    /// past the edges.
    fn grow(&self, cross: bool) -> Bits {
        let n = self.words;
        // A row with its bits moved one place either way, ORed with itself.
        let spread = |row: &[u64], out: &mut [u64]| {
            for k in 0..n {
                let up = row[k] << 1 | if k > 0 { row[k - 1] >> 63 } else { 0 };
                let down = row[k] >> 1 | if k + 1 < n { row[k + 1] << 63 } else { 0 };
                out[k] |= row[k] | up | down;
            }
        };
        let mut out = Bits { v: vec![0; self.v.len()], ..*self };
        for y in 0..self.h {
            let (lo, hi) = (y * n, (y + 1) * n);
            spread(self.row(y), &mut out.v[lo..hi]);
            for ny in [y.wrapping_sub(1), y + 1] {
                if ny >= self.h {
                    continue;
                }
                if cross {
                    for k in 0..n {
                        out.v[lo + k] |= self.v[ny * n + k];
                    }
                } else {
                    let row = self.row(ny).to_vec();
                    spread(&row, &mut out.v[lo..hi]);
                }
            }
        }
        out.trim();
        out
    }
}

/// What [`coat`] draws: a `w` × `h` region's new pixels (0 = left as it is) and the box (x, y, w, h,
/// in the region) the drawing and the silhouette cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coat {
    pub px: Vec<i32>,
    pub drawn: (usize, usize, usize, usize),
    pub silhouette: (usize, usize, usize, usize),
}

fn bounds(m: &[bool], w: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, _) in m.iter().enumerate().filter(|(_, v)| **v) {
        let (x, y) = (i % w, i / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    (x0 != usize::MAX).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// The coat over one boss part: `before` / `after` its draw, the `w` × `h` region at (`x0`, `y0`) on
/// the screen. What the draw changed is its silhouette (one-pixel holes — a sprite pixel the colour
/// of the floor under it — closed). `None`: it drew nothing here.
pub fn coat(before: &[i32], after: &[i32], w: usize, h: usize, x0: i32, y0: i32, look: Look) -> Option<Coat> {
    let changed: Vec<bool> = (0..w * h).map(|i| before[i] != after[i]).collect();
    let silhouette = bounds(&changed, w)?;
    // Work only round it: the close and the rings reach RINGS + 1 past it.
    let reach = RINGS + 1;
    let (ax, ay) = (silhouette.0.saturating_sub(reach), silhouette.1.saturating_sub(reach));
    let (aw, ah) = ((silhouette.0 + silhouette.2 + reach).min(w) - ax, (silhouette.1 + silhouette.3 + reach).min(h) - ay);
    let n = aw * ah;
    let at = |i: usize| (ay + i / aw) * w + ax + i % aw;
    let drawn = Bits::new(aw, ah, |x, y| changed[(ay + y) * w + ax + x]);
    // Close: grow then shrink once (a shrink = the complement grown).
    let shrunk = drawn.grow(false).not().grow(false);
    let body = Bits::new(aw, ah, |x, y| !shrunk.get(x, y) || drawn.get(x, y));
    // The rings: outline (cross), glow (square), glow (cross) — an octagon-ish halo.
    let mut ring = vec![0u8; n];
    let mut reached = body.clone();
    for r in 1..=RINGS {
        let next = reached.grow(r != 2);
        for (j, ring) in ring.iter_mut().enumerate() {
            let (x, y) = (j % aw, j / aw);
            if *ring == 0 && next.get(x, y) && !reached.get(x, y) {
                *ring = r as u8;
            }
        }
        reached = next;
    }

    let clock = look.clock() as f32;
    let pulse = 0.85 + 0.15 * (clock * std::f32::consts::TAU / 16.0).sin();
    let sheen = sheen_band();
    let mut px = vec![0i32; w * h];
    for (j, &r) in ring.iter().enumerate() {
        let inside = body.get(j % aw, j / aw);
        if !inside && r == 0 {
            continue;
        }
        let i = at(j);
        let (sx, sy) = (x0 + (i % w) as i32, y0 + (i / w) as i32);
        let p = after[i];
        px[i] = match (look, inside, r) {
            (Look::Coat { strength: s, flash, .. }, true, _) => {
                let mut c = mix(p, SILVER, 0.30 * s);
                let pos = (sx as f32 + sy as f32 * 0.7 - clock * SHEEN_SPEED).rem_euclid(SHEEN_PERIOD);
                c = mix(c, WHITE, sheen[(pos * SHEEN_STEPS) as usize % sheen.len()] * s);
                if flash { mix(c, WHITE, 0.35) } else { c }
            }
            (Look::Coat { strength: s, flash, .. }, false, r) if r > 0 => {
                match r {
                    1 => mix(p, if flash { WHITE } else { ICE }, s),
                    2 => mix(p, CYAN, 0.55 * s * pulse),
                    _ => mix(p, CYAN, 0.25 * s * pulse),
                }
            }
            // The warning: a dashed outline marching round, its glow coming up.
            (Look::Warn { progress, clock }, false, 1) => {
                let dash = !((sx + sy) as u32 / 2).wrapping_add(clock / 2).is_multiple_of(3);
                if dash && clock % 4 != 3 { mix(p, ICE, 0.9) } else { mix(p, CYAN, 0.3 * progress) }
            }
            (Look::Warn { progress, .. }, false, 2) => mix(p, CYAN, 0.25 * progress),
            (Look::Fade { left, .. }, true, _) => mix(p, SILVER, 0.30 * left),
            (Look::Fade { left, .. }, false, 1) => mix(p, ICE, left),
            (Look::Fade { left, .. }, false, 2) => mix(p, CYAN, 0.45 * left),
            _ => 0,
        };
    }
    let touched: Vec<bool> = px.iter().map(|&v| v != 0).collect();
    let drawn = bounds(&touched, w).unwrap_or(silhouette);
    Some(Coat { px, drawn, silhouette })
}

// ------------------------------------------------------------------------------------------------
// the badge

/// The badge's word, in the game's font.
pub const BADGE_TEXT: &str = "반사";

/// The game's 12-px glyphs sit in a label 14 high ([`crate::hooks`]' labels): the rows used.
const INK_TOP: usize = 1;
const INK_ROWS: usize = 12;
const PAD: usize = 3;
const DIAMOND: usize = 7;
const BAR: usize = 2;
/// The box's fill: the mock-up's near-black blue, mostly opaque.
const BOX: u32 = 0xD908_1622;
const BAR_BACK: Rgb = Rgb(40, 70, 80);

fn argb(c: Rgb, alpha: u32) -> i32 {
    ((alpha.min(255) << 24) | (u32::from(c.0) << 16) | (u32::from(c.1) << 8) | u32::from(c.2)) as i32
}

/// The badge: a mirror diamond and 「반사」 (`ink` — the game's letters as alpha over `w` × `h`, or
/// none) in a dark box, a bar under them — the warning fills it (blinking), the reflection drains
/// it. Its width, height and ARGB pixels.
pub fn badge(ink: Option<(&[u32], usize, usize)>, gauge: f32, warn: bool, clock: u32) -> (usize, usize, Vec<i32>) {
    let lw = ink.map_or(0, |(_, w, _)| w);
    let w = PAD + DIAMOND + if lw > 0 { 2 + lw } else { 0 } + PAD;
    let h = 2 + INK_ROWS + 1 + BAR + 2;
    let mut px = vec![BOX as i32; w * h];
    // Border, corners cut.
    for y in 0..h {
        for x in 0..w {
            let edge_x = x == 0 || x == w - 1;
            let edge_y = y == 0 || y == h - 1;
            if edge_x && edge_y {
                px[y * w + x] = 0;
            } else if edge_x || edge_y {
                px[y * w + x] = argb(CYAN, 255);
            }
        }
    }
    let lit = !warn || (clock / 4).is_multiple_of(2);
    // The diamond: |dx| + |dy| ≤ 3, its rim cyan, a glint top-left.
    let (cx, cy) = (PAD + DIAMOND / 2, 2 + INK_ROWS / 2);
    for dy in -3i32..=3 {
        for dx in -3i32..=3 {
            let d = dx.abs() + dy.abs();
            if d > 3 {
                continue;
            }
            let c = if d == 3 { CYAN } else if dx < 0 && dy < 0 { WHITE } else { ICE };
            px[(cy as i32 + dy) as usize * w + (cx as i32 + dx) as usize] = argb(c, if lit { 255 } else { 110 });
        }
    }
    if let Some((ink, iw, ih)) = ink {
        let x0 = PAD + DIAMOND + 2;
        for y in 0..INK_ROWS.min(ih.saturating_sub(INK_TOP)) {
            for x in 0..iw {
                let a = ink[(y + INK_TOP) * iw + x] >> 24;
                if a == 0 {
                    continue;
                }
                let a = if lit { a } else { a * 2 / 5 };
                let at = (2 + y) * w + x0 + x;
                let under = px[at] as u32;
                // Ink over the box: blend toward ICE by its alpha, keep the box's alpha.
                let mixed = mix(under as i32, ICE, a as f32 / 255.0) as u32;
                px[at] = ((under & 0xff00_0000) | (mixed & 0x00ff_ffff)) as i32;
            }
        }
    }
    let (bx0, bx1, by) = (PAD, w - PAD, 2 + INK_ROWS + 1);
    let fill = bx0 + ((bx1 - bx0) as f32 * gauge.clamp(0.0, 1.0)).round() as usize;
    for y in by..by + BAR {
        for x in bx0..bx1 {
            px[y * w + x] = if x < fill { argb(if warn { CYAN } else { ICE }, 255) } else { argb(BAR_BACK, 255) };
        }
    }
    (w, h, px)
}

/// Where the badge goes for the parts' box (x, y, w, h) on a `screen`: centred over it, or under it
/// when there is no room above (a boss at the top of the room), or inside its top as a last resort —
/// and on the screen.
pub fn badge_at(parts: (i32, i32, i32, i32), (w, h): (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    const GAP: i32 = 4;
    const EDGE: i32 = 2;
    let (px, py, pw, ph) = parts;
    let x = (px + pw / 2 - w / 2).clamp(EDGE, (screen.0 - w - EDGE).max(EDGE));
    let y = if py - h - GAP >= EDGE {
        py - h - GAP
    } else if py + ph + GAP + h <= screen.1 - EDGE {
        py + ph + GAP
    } else {
        EDGE
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Stance, ticks: u16) {
        for _ in 0..ticks {
            s.thought();
            s.tick();
        }
    }

    #[test]
    fn a_stance_waits_for_the_opening_then_warns_reflects_blinks_and_fades() {
        let mut s = Stance::new();
        s.thought();
        assert!(!s.picked(), "not in the fight's opening");
        run(&mut s, OPENING_TICKS);
        let mut picks = 0;
        while !s.picked() {
            picks += 1;
            assert!(picks < 200, "a fifth of the picks take it");
        }
        assert_eq!(s.phase(), Phase::Warn);
        assert!(!s.reflecting(), "the warning reflects nothing");
        assert!(matches!(s.look(), Some(Look::Warn { .. })));
        run(&mut s, WARN_TICKS);
        assert!(s.reflecting());
        assert_eq!(s.gauge(), Some(1.0));
        run(&mut s, REFLECT_TICKS - BLINK_TICKS);
        assert!(matches!(s.look(), Some(Look::Coat { strength, .. }) if strength == 1.0));
        let dims = (0..BLINK_TICKS)
            .filter(|_| {
                run(&mut s, 1);
                matches!(s.look(), Some(Look::Coat { strength, .. }) if strength < 1.0)
            })
            .count();
        assert!(dims > 0, "the last second blinks");
        assert_eq!(s.phase(), Phase::Fade);
        assert!(!s.reflecting());
        run(&mut s, FADE_TICKS);
        assert_eq!(s.phase(), Phase::Idle);
        assert!(!s.picked(), "the cooldown");
        run(&mut s, COOLDOWN_TICKS);
        assert!((0..200).any(|_| s.picked()));
    }

    #[test]
    fn a_quiet_ends_the_fight_and_the_next_one_opens_again() {
        let mut s = Stance::new();
        run(&mut s, OPENING_TICKS);
        while !s.picked() {}
        assert_eq!(s.phase(), Phase::Warn);
        for _ in 0..=QUIET_TICKS {
            s.tick();
        }
        assert_eq!(s.phase(), Phase::Idle, "no boss thought: the stance is gone");
        assert_eq!(s.look(), None);
        for _ in 0..500 {
            assert_eq!(s.tick(), None, "no fight, no clock");
        }
        s.thought();
        assert!(!s.picked(), "a new fight has its opening again");
    }

    #[test]
    fn the_reflection_is_a_fifth_mitigated_and_capped() {
        assert_eq!(reflected_hit(3000, 90, 1800), 170);
        assert_eq!(reflected_hit(30_000, 90, 1800), 360, "20 % of 1,800");
        assert_eq!(reflected_hit(30, 500, 1800), 1);
        assert_eq!(reflected_hit(0, 0, 1800), 0, "a blow that took nothing");
    }

    /// A 12 × 10 region: a 4 × 4 body drawn at (4, 3) with one pixel the floor's colour.
    fn part() -> (Vec<i32>, Vec<i32>) {
        let floor = 0xff_20_30_40u32 as i32;
        let before = vec![floor; 120];
        let mut after = before.clone();
        for y in 3..7 {
            for x in 4..8 {
                after[y * 12 + x] = 0xff_80_40_20u32 as i32;
            }
        }
        after[4 * 12 + 5] = floor;
        (before, after)
    }

    #[test]
    fn the_coat_covers_the_body_and_rings_it() {
        let (before, after) = part();
        let c = coat(&before, &after, 12, 10, 100, 50, Look::Coat { strength: 1.0, flash: false, clock: 0 }).unwrap();
        assert_eq!(c.silhouette, (4, 3, 4, 4));
        assert_eq!(c.drawn, (1, 0, 10, 10), "three rings round it");
        assert_ne!(c.px[4 * 12 + 5], 0, "the hole is closed and coated");
        let silvered = c.px[3 * 12 + 4];
        assert_ne!(silvered, after[3 * 12 + 4]);
        assert_eq!(silvered as u32 >> 24, 0xff, "opaque");
        assert_eq!(c.px[3 * 12 + 3], mix(after[3 * 12 + 3], ICE, 1.0), "the outline");
        assert_eq!(c.px[0], 0, "a corner far off is left alone");
    }

    #[test]
    fn the_badge_boxes_the_word_and_its_bar_drains() {
        let ink: Vec<u32> = (0..14 * 20).map(|i| if i / 20 == 6 { 0xff00_0000 } else { 0 }).collect();
        let (w, h, full) = badge(Some((&ink, 20, 14)), 1.0, false, 0);
        assert_eq!((w, h), (PAD + DIAMOND + 2 + 20 + PAD, 19));
        assert_eq!(full[0], 0, "corners cut");
        assert_eq!(full[w + 1], BOX as i32);
        let bar = |px: &[i32]| (PAD..w - PAD).filter(|&x| px[(2 + INK_ROWS + 1) * w + x] == argb(ICE, 255)).count();
        assert_eq!(bar(&full), w - 2 * PAD);
        let (_, _, half) = badge(Some((&ink, 20, 14)), 0.5, false, 0);
        assert_eq!(bar(&half), (w - 2 * PAD).div_ceil(2));
        let lettered = (2 + 5) * w + PAD + DIAMOND + 2;
        assert_ne!(full[lettered], BOX as i32, "the word's ink row");
        let (w0, _, _) = badge(None, 1.0, false, 0);
        assert_eq!(w0, PAD + DIAMOND + PAD, "no letters: the diamond alone");
    }

    #[test]
    fn the_badge_sits_over_the_boss_or_under_one_at_the_top() {
        assert_eq!(badge_at((100, 80, 60, 40), (40, 19), (320, 240)), (110, 57));
        assert_eq!(badge_at((100, 0, 60, 100), (40, 19), (320, 240)), (110, 104), "no room above");
        assert_eq!(badge_at((300, 0, 60, 230), (40, 19), (320, 240)), (278, 2), "no room at all; kept on the screen");
    }

    /// What a coat costs (x86, release): a part read around its last box, and read whole (the first
    /// frame). `cargo test -p hz_java --release coat_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn coat_cost() {
        for (w, h) in [(150, 140), (520, 240)] {
            let before = vec![0xff_20_30_40u32 as i32; w * h];
            let mut after = before.clone();
            for y in 20..120 {
                for x in 25..125 {
                    if (x * 7 + y * 3) % 11 != 0 {
                        after[y * w + x] = 0xff_80_40_20u32 as i32;
                    }
                }
            }
            let look = Look::Coat { strength: 1.0, flash: false, clock: 7 };
            let start = std::time::Instant::now();
            for _ in 0..100 {
                std::hint::black_box(coat(&before, &after, w, h, 0, 0, look));
            }
            println!("coat {w}x{h}: {:.0} us", start.elapsed().as_secs_f64() * 1e6 / 100.0);
        }
    }

    #[test]
    fn nothing_drawn_is_no_coat_and_the_warning_leaves_the_body() {
        let (before, after) = part();
        assert_eq!(coat(&before, &before, 12, 10, 0, 0, Look::Fade { left: 1.0, clock: 0 }), None);
        let c = coat(&before, &after, 12, 10, 0, 0, Look::Warn { progress: 0.5, clock: 0 }).unwrap();
        assert_eq!(c.px[3 * 12 + 4], 0, "no silver while it only warns");
        assert!((0..12).any(|x| c.px[2 * 12 + x] != 0), "but an outline");
    }
}
