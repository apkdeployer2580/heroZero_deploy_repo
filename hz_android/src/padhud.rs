//! The gamepad on the touch HUD (hero2's `padhud.rs`, 59차): a POSITION badge on every control a
//! pad button fires, so a player can tell which button does what on ANY pad.
//!
//! Positions, not letters (hero2's player, 2026-08-19: "제조사별로 패드가 다르니까 … 위치로
//! 표시하는게 나을거같은데"): Android's `generic.kl` maps `BTN_SOUTH → BUTTON_A`, `BTN_EAST →
//! BUTTON_B`, `BTN_NORTH → BUTTON_Y`, `BTN_WEST → BUTTON_X`, so keycode 96 is "the BOTTOM face
//! button" on every standard pad — printed `A` on an Xbox pad, `B` on a Switch pad (mirrored
//! letters), `✕` on a PlayStation pad. A badge that draws the position is right on all three.
//!
//! Pure (no window, no JVM): the position table, the badge geometry and the preference are
//! tested on the host; `render` draws them.

use crate::{gamepad, hud::Rect};

/// Which of the four face positions. `Down` is Xbox `A` / Switch `B` / PS `✕`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FacePos {
    Up,
    Down,
    Left,
    Right,
}

/// Where a pad control sits, in the only terms every pad family shares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadPos {
    /// One of the four face buttons (drawn as a diamond of four dots, the bound one lit).
    Face(FacePos),
    /// A shoulder button or trigger: which side, and which of the two rows.
    /// `lower` = the trigger (L2/LT/ZL), `!lower` = the bumper (L1/LB/L).
    Shoulder { right: bool, lower: bool },
    /// The two small middle buttons: SELECT/`−`/SHARE (left) and START/`+`/OPTIONS (right).
    Middle { right: bool },
    /// Pressing a stick in (L3 / R3).
    StickPress { right: bool },
}

/// Where on the pad this keycode is. `None` = a code this HUD has no picture for (the d-pad —
/// it is the walk, never badged — and every code the mapping table ignores).
pub fn pos_for_code(code: u32) -> Option<PadPos> {
    Some(match code {
        gamepad::KEYCODE_BUTTON_A => PadPos::Face(FacePos::Down),
        gamepad::KEYCODE_BUTTON_B => PadPos::Face(FacePos::Right),
        gamepad::KEYCODE_BUTTON_X => PadPos::Face(FacePos::Left),
        gamepad::KEYCODE_BUTTON_Y => PadPos::Face(FacePos::Up),
        gamepad::KEYCODE_BUTTON_L1 => PadPos::Shoulder { right: false, lower: false },
        gamepad::KEYCODE_BUTTON_R1 => PadPos::Shoulder { right: true, lower: false },
        gamepad::KEYCODE_BUTTON_L2 => PadPos::Shoulder { right: false, lower: true },
        gamepad::KEYCODE_BUTTON_R2 => PadPos::Shoulder { right: true, lower: true },
        gamepad::KEYCODE_BUTTON_THUMBL => PadPos::StickPress { right: false },
        gamepad::KEYCODE_BUTTON_THUMBR => PadPos::StickPress { right: true },
        gamepad::KEYCODE_BUTTON_SELECT => PadPos::Middle { right: false },
        gamepad::KEYCODE_BUTTON_START => PadPos::Middle { right: true },
        _ => return None,
    })
}

impl PadPos {
    /// The position as four characters of overlay text (the PAD REMAP panel): `F-UP` … `F-RT`
    /// the face buttons, `L-HI`/`L-LO` the left bumper and trigger (`R-…` the right), `M-LT`/
    /// `M-RT` the middle pair, `S-LT`/`S-RT` a stick press.
    pub fn short(self) -> &'static str {
        match self {
            PadPos::Face(FacePos::Up) => "F-UP",
            PadPos::Face(FacePos::Down) => "F-DN",
            PadPos::Face(FacePos::Left) => "F-LT",
            PadPos::Face(FacePos::Right) => "F-RT",
            PadPos::Shoulder { right: false, lower: false } => "L-HI",
            PadPos::Shoulder { right: false, lower: true } => "L-LO",
            PadPos::Shoulder { right: true, lower: false } => "R-HI",
            PadPos::Shoulder { right: true, lower: true } => "R-LO",
            PadPos::Middle { right: false } => "M-LT",
            PadPos::Middle { right: true } => "M-RT",
            PadPos::StickPress { right: false } => "S-LT",
            PadPos::StickPress { right: true } => "S-RT",
        }
    }
}

/// The PAD BADGES preference (`hz_pad_hud.txt`, settings panel row).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PadHud {
    /// The last input source wins: a pad press shows the badges, a screen touch hides them.
    /// Default, and the reason there is no timeout — every switch is something the player just
    /// did, so nothing changes on its own.
    #[default]
    Auto,
    /// Always the badges.
    On,
    /// Never.
    Off,
}

impl PadHud {
    /// File token. Anything else parses back to [`PadHud::Auto`] — a corrupt or hand-edited
    /// file must not change behaviour silently.
    pub fn as_token(self) -> &'static str {
        match self {
            PadHud::Auto => "auto",
            PadHud::On => "on",
            PadHud::Off => "off",
        }
    }

    pub fn parse(s: &str) -> PadHud {
        match s.trim() {
            "on" => PadHud::On,
            "off" => PadHud::Off,
            _ => PadHud::Auto,
        }
    }

    /// The settings row cycles AUTO → ON → OFF → AUTO.
    pub fn next(self) -> PadHud {
        match self {
            PadHud::Auto => PadHud::On,
            PadHud::On => PadHud::Off,
            PadHud::Off => PadHud::Auto,
        }
    }

    /// Are the badges showing? `pad_last` = the most recent input came from a pad.
    pub fn active(self, pad_last: bool) -> bool {
        match self {
            PadHud::Auto => pad_last,
            PadHud::On => true,
            PadHud::Off => false,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PadHud::Auto => "PAD BADGES AUTO",
            PadHud::On => "PAD BADGES ON",
            PadHud::Off => "PAD BADGES OFF",
        }
    }
}

/// A position badge: a small disc stuck on a control, holding the position glyph.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Badge {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
}

/// Fraction of the control's inscribed radius the badge occupies, and how far out its centre
/// sits. 0.74 puts the badge centre ON the rim, so it reads as stuck to the control (an app-icon
/// badge) instead of floating beside it, and leaves the middle of the face — the label — alone.
const BADGE_R: f64 = 0.36;
const BADGE_OUT: f64 = 0.74;
/// Never smaller than this many pixels of radius, or the glyph inside stops being a glyph.
const BADGE_MIN: f64 = 12.0;

/// Badge for the control occupying `rect`, placed at its LOWER-LEFT rim and clamped inside the
/// window.
///
/// Lower-left because of how this HUD is arranged: the ring climbs up-and-left from the OK disc
/// and the keypad is a grid, so the down-left diagonal is the direction that is empty for every
/// control. A badge that lands on the game is fine (the whole overlay does); a badge that lands
/// on the NEXT control's badge is not.
pub fn badge_at(rect: Rect, win_w: i32, win_h: i32) -> Option<Badge> {
    if rect.w <= 0 || rect.h <= 0 || win_w <= 0 || win_h <= 0 {
        return None;
    }
    let rad = f64::from(rect.w.min(rect.h)) / 2.0;
    // The floor can exceed the control (a tiny control gets a proportionally huge badge), so cap
    // at 55 % of it: a badge must never be mistaken for the control.
    let r = (rad * BADGE_R).max(BADGE_MIN.min(rad * 0.55)).min(rad);
    let cx = f64::from(rect.x) + f64::from(rect.w) / 2.0 - rad * BADGE_OUT;
    let cy = f64::from(rect.y) + f64::from(rect.h) / 2.0 + rad * BADGE_OUT;
    // `max` before `min` so a window narrower than the badge yields the centre, not a negative.
    let cx = cx.max(r).min(f64::from(win_w) - r).max(0.0);
    let cy = cy.max(r).min(f64::from(win_h) - r).max(0.0);
    Some(Badge { cx, cy, r })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pad_button_has_a_position_and_the_dpad_has_none() {
        for code in gamepad::BUTTONS {
            assert!(pos_for_code(code).is_some(), "keycode {code}");
        }
        assert_eq!(pos_for_code(19), None, "DPAD_UP is the walk");
        assert_eq!(pos_for_code(gamepad::KEYCODE_BUTTON_A), Some(PadPos::Face(FacePos::Down)));
        assert_eq!(pos_for_code(gamepad::KEYCODE_BUTTON_R2), Some(PadPos::Shoulder { right: true, lower: true }));
        assert_eq!(pos_for_code(gamepad::KEYCODE_BUTTON_START), Some(PadPos::Middle { right: true }));
    }

    #[test]
    fn the_preference_round_trips_cycles_and_decides() {
        for p in [PadHud::Auto, PadHud::On, PadHud::Off] {
            assert_eq!(PadHud::parse(p.as_token()), p);
            assert!(crate::font::drawable(p.label()));
        }
        assert_eq!(PadHud::parse("garbage"), PadHud::Auto);
        assert_eq!(PadHud::Auto.next().next().next(), PadHud::Auto);
        assert!(PadHud::Auto.active(true) && !PadHud::Auto.active(false));
        assert!(PadHud::On.active(false));
        assert!(!PadHud::Off.active(true));
    }

    #[test]
    fn the_badge_sits_on_the_lower_left_rim_inside_the_window() {
        let b = badge_at(Rect::new(100, 100, 100, 100), 1000, 1000).unwrap();
        assert!(b.cx < 150.0 && b.cy > 150.0, "lower-left of the centre: {b:?}");
        assert!((b.r - 18.0).abs() < 1e-9, "36 % of the inscribed radius");
        // Against the window's edge: whole badge still inside.
        let e = badge_at(Rect::new(0, 900, 100, 100), 1000, 1000).unwrap();
        assert!(e.cx >= e.r && e.cy + e.r <= 1000.0, "{e:?}");
        // A tiny control: floored, but capped at 55 % so it cannot pass for the control.
        let t = badge_at(Rect::new(0, 0, 20, 20), 1000, 1000).unwrap();
        assert!((t.r - 5.5).abs() < 1e-9, "{t:?}");
        assert!(badge_at(Rect::new(0, 0, 0, 10), 1000, 1000).is_none());
    }
}
