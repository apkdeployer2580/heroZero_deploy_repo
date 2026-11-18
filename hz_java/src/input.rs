//! How 영웅서기 Zero's canvas consumes keys, and the projection of the host's key edges onto it.
//!
//! The game's canvas `f` keeps ONE key (javap, 2026-10-22):
//!
//! ```text
//! keyPressed(k)  { ...; a = k }          // the pending press — a second press overwrites it
//! keyReleased(k) { b = k }               // the pending release — of ANY key
//! a()  (once per game frame):
//!     if a != 0 { if c != 0 { release(c) }  c = a; press(a); a = 0 }
//!     if b != 0 { if c != 0 { release(c); c = 0 }  b = 0 }      // releases the CURRENT key
//! ```
//!
//! Three consequences, each measured or read off that code:
//!
//! * a release releases whatever is current, not the key released — so a host that sends
//!   keyup(old) + keydown(new) for a direction change inside one game frame ends with NOTHING
//!   held (the press is handled first, then the release drops it);
//! * a second press in the same frame overwrites the first, so a tap followed at once by a
//!   re-press of another key loses the tap;
//! * after a press of another key the game no longer holds the first, and nothing brings it
//!   back: probe `out/dir3_*` — RIGHT held, OK tapped: the walk stops for good.
//!
//! So the host sends the game only what it can represent. Presses go out at once (a press is
//! "this is now the key", which the game does itself). A release goes out only when NO host
//! key is still down, and not before the next game frame (so a same-frame press can cancel it).
//! When the key the game holds is lifted while another is still down, the other is pressed
//! again — two game frames later, so the lifted key's own press cannot be overwritten. Frames
//! are counted by the guest's paints, the one clock the host sees that runs at the game's pace.
//!
//! A release that went out is not taken at once either: the paint the host counts lands after
//! the next frame's `a()` (the game loop calls `repaint` and goes straight on to `a()`; the paint
//! runs when it sleeps), so the release waits a whole game frame — and an arrow pressed in that
//! frame is pressed and then dropped by it in the same `a()`. In the field the drop stops the walk
//! (`ed.e()`, the release while walking) and nothing presses the arrow again: a thumb flicked across
//! the stick's centre stood still (probe 2026-10-31, ↙ to ↗). So an arrow pressed before the next
//! paint after a release went out waits for it ([`UP_TAKEN_FRAMES`], [`walk_edge`](KeyProjection::walk_edge));
//! a tap in that time is still a tap. Other keys go at once — a press is what OK and the slots act
//! on, and their own timing is the combo's.

/// HZ-059: the game's walk directions (`rpg/d.V` / `W`, `ar.c`): the four arrows, and the diagonals
/// 5 ↖ (−1, −1) · 6 ↗ (+1, −1) · 7 ↙ (−1, +1) · 8 ↘ (+1, +1) the tables hold but no key reaches.
pub mod dir {
    pub const UP: i8 = 1;
    pub const DOWN: i8 = 2;
    pub const LEFT: i8 = 3;
    pub const RIGHT: i8 = 4;

    /// The diagonal of a horizontal arrow (← →) and a vertical one (↑ ↓); `None` for any other pair.
    pub fn diagonal(horizontal: i8, vertical: i8) -> Option<i8> {
        let east = match horizontal {
            LEFT => 0,
            RIGHT => 1,
            _ => return None,
        };
        let south = match vertical {
            UP => 0,
            DOWN => 2,
            _ => return None,
        };
        Some(5 + east + south)
    }

    /// A diagonal's two arrows, horizontal first; `None` for an arrow or anything else.
    pub fn split(d: i8) -> Option<(i8, i8)> {
        (5..=8).contains(&d).then(|| (if d % 2 == 1 { LEFT } else { RIGHT }, if d <= 6 { UP } else { DOWN }))
    }

    /// What the arrows held make (oldest first): the newest horizontal with the newest vertical, or 0.
    pub fn held(arrows: &[i8]) -> i8 {
        let newest = |a: i8, b: i8| arrows.iter().rev().copied().find(|&k| k == a || k == b);
        match (newest(LEFT, RIGHT), newest(UP, DOWN)) {
            (Some(h), Some(v)) => diagonal(h, v).unwrap_or(0),
            _ => 0,
        }
    }
}

/// What to send the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestKey<K> {
    Down(K),
    Up(K),
}

/// Game frames between a press and the re-press of a still-held key: one so the game's `a()` has
/// consumed the press, one more because a paint may land between a press and the `a()` that
/// takes it.
pub const REASSERT_FRAMES: u32 = 2;

/// Game frames after a release goes out (at a paint) before the game has surely taken it: the next
/// paint lands after the `a()` that takes it. A release sent at any other moment
/// ([`release_all`](KeyProjection::release_all)) waits [`REASSERT_FRAMES`], as a press does.
pub const UP_TAKEN_FRAMES: u32 = 1;

#[derive(Debug)]
pub struct KeyProjection<K> {
    /// Host keys down, oldest first. The last is the one the game holds (or will).
    held: Vec<K>,
    /// The game's key, lifted with nothing else down: released at the next frame unless pressed over.
    pending_up: Option<K>,
    /// A still-held key to press again once `frames` reaches [`REASSERT_FRAMES`].
    reassert: Option<K>,
    /// An arrow pressed before the game took the release that went out: its press, once `up_wait`
    /// runs out. Let go of before then (not in `held`), a tap: the press, then the release a frame
    /// later.
    waiting: Option<K>,
    /// Game frames since the last press sent.
    frames: u32,
    /// Game frames until the game has taken the last release sent.
    up_wait: u32,
}

impl<K: Copy + PartialEq> Default for KeyProjection<K> {
    fn default() -> Self {
        KeyProjection {
            held: Vec::new(),
            pending_up: None,
            reassert: None,
            waiting: None,
            frames: REASSERT_FRAMES,
            up_wait: 0,
        }
    }
}

impl<K: Copy + PartialEq> KeyProjection<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A host key went down.
    pub fn press(&mut self, k: K) -> Vec<GuestKey<K>> {
        if self.held.last() == Some(&k) {
            return Vec::new();
        }
        self.held.retain(|&h| h != k);
        self.held.push(k);
        self.pending_up = None;
        self.reassert = None;
        self.waiting = None;
        self.frames = 0;
        vec![GuestKey::Down(k)]
    }

    /// An arrow went down: at once, or — a release out that the game may not have taken yet —
    /// when it has (`waiting`, sent by [`frame`](Self::frame)).
    fn press_arrow(&mut self, k: K) -> Vec<GuestKey<K>> {
        if self.up_wait == 0 || self.held.last() == Some(&k) {
            return self.press(k);
        }
        self.held.retain(|&h| h != k);
        self.held.push(k);
        self.reassert = None;
        self.waiting = Some(k);
        Vec::new()
    }

    /// A host key went up.
    pub fn release(&mut self, k: K) -> Vec<GuestKey<K>> {
        let Some(i) = self.held.iter().position(|&h| h == k) else {
            return Vec::new();
        };
        let was_top = i + 1 == self.held.len();
        self.held.remove(i);
        if self.waiting == Some(k) {
            if self.held.is_empty() {
                // A tap whose press has not gone out: `frame` presses it, then lets go.
                return Vec::new();
            }
            self.waiting = None;
        }
        match self.held.last() {
            None => {
                self.reassert = None;
                self.pending_up = Some(k);
            }
            Some(&top) if was_top => self.reassert = Some(top),
            Some(_) => {}
        }
        Vec::new()
    }

    /// The guest painted a frame.
    pub fn frame(&mut self) -> Vec<GuestKey<K>> {
        self.frames = self.frames.saturating_add(1);
        self.up_wait = self.up_wait.saturating_sub(1);
        if let Some(k) = self.pending_up.take() {
            self.up_wait = UP_TAKEN_FRAMES;
            return vec![GuestKey::Up(k)];
        }
        let settled = self.frames >= REASSERT_FRAMES && self.up_wait == 0;
        if settled && let Some(k) = self.waiting.take() {
            self.frames = 0;
            if self.held.is_empty() {
                self.pending_up = Some(k);
            }
            return vec![GuestKey::Down(k)];
        }
        match self.reassert {
            Some(k) if settled && self.held.last() == Some(&k) => {
                self.reassert = None;
                self.frames = 0;
                vec![GuestKey::Down(k)]
            }
            _ => Vec::new(),
        }
    }

    /// HZ-059: a host key's edge ([`press`](Self::press) / [`release`](Self::release)) and the diagonal the
    /// arrows held make after it (`arrow` names a key's direction, `None` for a key that is no arrow;
    /// 0 = no diagonal) — the caller hands it to the walk hook (`GuestState::set_walk_diag`) BEFORE it
    /// sends the keys. A diagonal changed by an edge that sends nothing (the arrow let go of is not the
    /// one the game holds) presses the game's arrow again, so the walk turns. An arrow pressed right
    /// after a release went out waits for the game to take it (the module's last paragraph).
    pub fn walk_edge(&mut self, k: K, down: bool, arrow: impl Fn(K) -> Option<i8>) -> (Vec<GuestKey<K>>, i8) {
        let diagonal = |held: &[K]| dir::held(&held.iter().filter_map(|&k| arrow(k)).collect::<Vec<_>>());
        let before = diagonal(&self.held);
        let out = match (down, arrow(k)) {
            (true, Some(_)) => self.press_arrow(k),
            (true, None) => self.press(k),
            (false, _) => self.release(k),
        };
        let after = diagonal(&self.held);
        if after != before && out.is_empty() && self.held.last().is_some_and(|&top| arrow(top).is_some()) {
            self.repress();
        }
        (out, after)
    }

    /// Press the game's key again at the next frame the re-press rule allows — HZ-059: what the key
    /// means changed under it (an arrow let go of turns a diagonal back into the arrow still held),
    /// and the game reads a key only when it is pressed.
    pub fn repress(&mut self) {
        if self.waiting.is_some() {
            return; // its own press is still to go, and carries the turn
        }
        if let Some(&top) = self.held.last() {
            self.reassert = Some(top);
        }
    }

    /// Everything up NOW (pause, focus loss, editor): one release drops whatever the game holds.
    pub fn release_all(&mut self) -> Vec<GuestKey<K>> {
        let top = self.held.last().copied().or(self.pending_up);
        self.held.clear();
        self.pending_up = None;
        self.reassert = None;
        self.waiting = None;
        if top.is_some() {
            self.up_wait = REASSERT_FRAMES;
        }
        top.map(|k| vec![GuestKey::Up(k)]).unwrap_or_default()
    }

    /// Host keys currently down.
    pub fn held(&self) -> &[K] {
        &self.held
    }

    /// The arrow whose press waits for the game to take a release (`frame` sends it) — the
    /// key → drawn gauges time it from the host's press.
    pub fn waiting(&self) -> Option<K> {
        self.waiting
    }
}

#[cfg(test)]
mod tests {
    use super::{GuestKey::*, *};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum K {
        Right,
        Down,
        Left,
        Up,
        Ok,
    }

    fn arrow(k: K) -> Option<i8> {
        match k {
            K::Right => Some(dir::RIGHT),
            K::Down => Some(dir::DOWN),
            K::Left => Some(dir::LEFT),
            K::Up => Some(dir::UP),
            K::Ok => None,
        }
    }

    /// The game's own model (`f.keyPressed/keyReleased/a()`), to check what it ends up holding.
    #[derive(Default)]
    struct Game {
        a: Option<K>,
        b: bool,
        c: Option<K>,
        pressed: Vec<K>,
    }

    impl Game {
        fn send(&mut self, events: Vec<GuestKey<K>>) {
            for e in events {
                match e {
                    Down(k) => self.a = Some(k),
                    Up(_) => self.b = true,
                }
            }
        }

        /// One game frame: `a()`, then the paint the host counts.
        fn frame(&mut self, p: &mut KeyProjection<K>) {
            if let Some(k) = self.a.take() {
                self.c = Some(k);
                self.pressed.push(k);
            }
            if std::mem::take(&mut self.b) {
                self.c = None;
            }
            let out = p.frame();
            self.send(out);
        }
    }

    #[test]
    fn a_tap_while_walking_lands_and_the_walk_resumes() {
        let (mut p, mut g) = (KeyProjection::new(), Game::default());
        g.send(p.press(K::Right));
        g.frame(&mut p);
        assert_eq!(g.c, Some(K::Right));
        // A fast tap: down and up inside one game frame.
        g.send(p.press(K::Ok));
        g.send(p.release(K::Ok));
        g.frame(&mut p);
        assert_eq!(g.pressed, vec![K::Right, K::Ok], "the tap is not overwritten");
        for _ in 0..REASSERT_FRAMES {
            g.frame(&mut p);
        }
        assert_eq!(g.c, Some(K::Right), "walking again — the probe showed it stops for good otherwise");
    }

    #[test]
    fn a_direction_change_in_the_generic_order_keeps_walking() {
        // keyup(old) then keydown(new), the order a generic HUD emits, inside one game frame.
        let (mut p, mut g) = (KeyProjection::new(), Game::default());
        g.send(p.press(K::Right));
        g.frame(&mut p);
        g.send(p.release(K::Right));
        g.send(p.press(K::Down));
        g.frame(&mut p);
        g.frame(&mut p);
        assert_eq!(g.c, Some(K::Down), "a raw keyup+keydown would leave nothing held");
    }

    #[test]
    fn letting_go_releases_once_a_frame_later() {
        let mut p = KeyProjection::new();
        assert_eq!(p.press(K::Right), vec![Down(K::Right)]);
        assert_eq!(p.release(K::Right), vec![], "not inside the frame the press may share");
        assert_eq!(p.frame(), vec![Up(K::Right)]);
        assert_eq!(p.frame(), vec![]);
        assert!(p.held().is_empty());
    }

    #[test]
    fn lifting_a_key_the_game_no_longer_holds_sends_nothing() {
        let mut p = KeyProjection::new();
        p.press(K::Right);
        p.press(K::Ok);
        assert_eq!(p.release(K::Right), vec![], "a release would drop OK (the game's current key)");
        for _ in 0..4 {
            assert_eq!(p.frame(), vec![], "OK is still the key — nothing to re-press");
        }
        assert_eq!(p.release(K::Ok), vec![]);
        assert_eq!(p.frame(), vec![Up(K::Ok)]);
    }

    #[test]
    fn release_all_is_immediate_and_single() {
        let mut p = KeyProjection::new();
        p.press(K::Right);
        p.press(K::Ok);
        assert_eq!(p.release_all(), vec![Up(K::Ok)]);
        assert_eq!(p.release_all(), vec![]);
        // A lift still waiting for its frame is flushed too.
        p.press(K::Down);
        p.release(K::Down);
        assert_eq!(p.release_all(), vec![Up(K::Down)]);
        assert_eq!(p.frame(), vec![]);
    }

    #[test]
    fn a_held_key_pressed_again_is_not_a_second_press() {
        let mut p = KeyProjection::new();
        p.press(K::Right);
        assert_eq!(p.press(K::Right), vec![], "a second finger on the same key");
    }

    #[test]
    fn letting_go_of_a_diagonals_other_arrow_presses_the_held_one_again() {
        // ↘: RIGHT then DOWN; the game holds DOWN (walking 8). RIGHT up: the game must hear DOWN again.
        let (mut p, mut g) = (KeyProjection::new(), Game::default());
        g.send(p.press(K::Right));
        g.frame(&mut p);
        g.send(p.press(K::Down));
        g.frame(&mut p);
        assert_eq!(p.release(K::Right), vec![]);
        p.repress();
        for _ in 0..REASSERT_FRAMES {
            g.frame(&mut p);
        }
        assert_eq!(g.pressed, vec![K::Right, K::Down, K::Down]);
        assert_eq!(g.c, Some(K::Down));
        p.repress();
        p.release(K::Down);
        assert_eq!(p.frame(), vec![Up(K::Down)], "nothing held: the lift, not a re-press");
        p.repress();
        assert_eq!(p.frame(), vec![]);
    }

    #[test]
    fn walk_edges_carry_the_diagonal() {
        let mut p = KeyProjection::new();
        assert_eq!(p.walk_edge(K::Right, true, arrow), (vec![Down(K::Right)], 0));
        assert_eq!(p.walk_edge(K::Down, true, arrow), (vec![Down(K::Down)], 8), "the press itself turns the walk");
        p.frame();
        p.frame();
        // OK cuts in; letting go of DOWN under it re-presses nothing — OK is no arrow.
        assert_eq!(p.walk_edge(K::Ok, true, arrow), (vec![Down(K::Ok)], 8));
        assert_eq!(p.walk_edge(K::Down, false, arrow), (vec![], 0));
        p.frame();
        p.frame();
        assert_eq!(p.frame(), vec![], "no second OK");
        assert_eq!(p.walk_edge(K::Ok, false, arrow), (vec![], 0));
        assert_eq!(p.frame(), vec![Down(K::Right)], "RIGHT comes back, as it always did");
        // ↘ again, then RIGHT (not the game's key) let go of: DOWN again.
        p.walk_edge(K::Down, true, arrow);
        assert_eq!(p.walk_edge(K::Right, false, arrow), (vec![], 0));
        p.frame();
        assert_eq!(p.frame(), vec![Down(K::Down)]);
    }

    #[test]
    fn an_arrow_right_after_a_release_went_out_waits_for_the_game_to_take_it() {
        let (mut p, mut g) = (KeyProjection::new(), Game::default());
        g.send(p.walk_edge(K::Right, true, arrow).0);
        g.frame(&mut p);
        g.send(p.walk_edge(K::Right, false, arrow).0);
        g.frame(&mut p);
        assert!(g.b, "the release went out with the paint; the game takes it at its next a()");
        assert_eq!(p.walk_edge(K::Down, true, arrow).0, vec![], "sent now, the release would drop it in that a()");
        for _ in 0..=REASSERT_FRAMES {
            g.frame(&mut p);
        }
        assert_eq!(g.c, Some(K::Down));
        assert_eq!(g.pressed, vec![K::Right, K::Down]);
        // A tap in that time is still a tap: pressed, then let go of.
        g.send(p.walk_edge(K::Down, false, arrow).0);
        g.frame(&mut p);
        g.send(p.walk_edge(K::Right, true, arrow).0);
        g.send(p.walk_edge(K::Right, false, arrow).0);
        for _ in 0..=REASSERT_FRAMES + 1 {
            g.frame(&mut p);
        }
        assert_eq!(g.pressed, vec![K::Right, K::Down, K::Right]);
        assert_eq!(g.c, None);
        // OK is not held back: what it does, it does on the press, and its timing is the combo's.
        g.send(p.walk_edge(K::Ok, true, arrow).0);
        g.send(p.walk_edge(K::Ok, false, arrow).0);
        g.frame(&mut p);
        assert_eq!(p.walk_edge(K::Ok, true, arrow).0, vec![Down(K::Ok)]);
    }

    #[test]
    fn a_stick_flicked_across_its_centre_walks_the_new_way() {
        // ↘ held (the game holds DOWN); the thumb crosses the dead zone to ↖: both lifted, the release
        // goes out with the paint, and both new arrows come before the game has taken it.
        let (mut p, mut g) = (KeyProjection::new(), Game::default());
        g.send(p.walk_edge(K::Right, true, arrow).0);
        g.send(p.walk_edge(K::Down, true, arrow).0);
        g.frame(&mut p);
        for k in [K::Right, K::Down] {
            g.send(p.walk_edge(k, false, arrow).0);
        }
        g.frame(&mut p);
        let mut diagonal = 0;
        for k in [K::Left, K::Up] {
            let (out, d) = p.walk_edge(k, true, arrow);
            g.send(out);
            diagonal = d;
        }
        assert_eq!(diagonal, 5);
        for _ in 0..=REASSERT_FRAMES {
            g.frame(&mut p);
        }
        assert_eq!(g.c, Some(K::Up), "the walk hook makes it ↖");
        assert_eq!(g.pressed, vec![K::Down, K::Up], "one press — the turn needs no second");
    }

    #[test]
    fn diagonals_from_the_arrows_held() {
        use dir::*;
        assert_eq!([diagonal(LEFT, UP), diagonal(RIGHT, UP), diagonal(LEFT, DOWN), diagonal(RIGHT, DOWN)], [Some(5), Some(6), Some(7), Some(8)]);
        assert_eq!(diagonal(UP, LEFT), None);
        for d in 5..=8 {
            let (h, v) = split(d).unwrap();
            assert_eq!(diagonal(h, v), Some(d));
        }
        assert_eq!((split(4), split(9), split(0)), (None, None, None));
        assert_eq!(held(&[RIGHT, DOWN]), 8);
        assert_eq!(held(&[DOWN, RIGHT]), 8);
        assert_eq!(held(&[RIGHT, DOWN, LEFT]), 7, "the newest horizontal");
        assert_eq!(held(&[RIGHT, LEFT]), 0, "two horizontals are no diagonal");
        assert_eq!(held(&[UP]), 0);
        assert_eq!(held(&[]), 0);
    }
}
