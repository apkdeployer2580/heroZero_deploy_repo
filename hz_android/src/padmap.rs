// 패드 버튼 설정 (2026-10-09) — the pure half of the gear's 「패드 버튼 설정」 screen.
//
// User, 2026-10-09: *"상단 톱니바퀴 설정으로 패드 키를 맵핑하는 기능을 넣을꺼야 … hero2에도
// 마찬가지로 넣어야해"*, then the three decisions this file encodes: a conflict SWAPS, the
// screen is driven by touch (pick a function) + the pad (press the button), hero3 first and
// the same file into hero2. Second form, the same day: the function is picked ON THE IN-GAME
// HUD — the control that does it in play, lit where it is played — not from a list (*"실제
// 인게임 화면상에 버튼들을 하이라이트 처리하고 각 버튼을 눌르고 패드 버튼을 누르는 방식"*).
// Nothing in this file changed for it: what is picked and how it is drawn are each game's.
//
// ★ SHARED BYTE FOR BYTE with hero2 (`hero2NativeAPP/wie_android/src/padmap.rs` — CRLF
// there, so compare with `--strip-trailing-cr`), the way `padhud.rs` already is. Everything
// game-specific — which actions exist, what they are called, the built-in layout — is handed
// in through [`Pad`]; nothing here knows what a quickslot is. A change here goes into BOTH.
//
// Dependency-free on purpose: `padmap_selftest.rs` at the repo root `include!`s this file,
// which is the only way to test it on the Windows host (`cargo test -p wie_android` aborts
// there). Do not add a `use crate::...`.
//
// ── THE MODEL ────────────────────────────────────────────────────────────────
//
// The store is the one the app already had: `wie_gamepad.txt`, `<android keycode>=<TOKEN>`,
// consulted before the built-in layout (`Overrides`, `None` = the button is switched off).
// This screen is its first WRITER.
//
// * The player picks an ACTION and presses a BUTTON — not the other way round. Nobody has to
//   know which of their buttons Android calls "X", and a button outside the standard set
//   (C / Z / a vendor key) is caught by its keycode like any other.
// * A conflict SWAPS ([`Pad::assign`]): the action's current button takes over what the
//   pressed button did. No action loses its button to a remap, so the two the game cannot
//   be played without (OK / CLR — `Pad::essential`) can never end up unreachable.
// * Directions are not remappable ([`capturable`]): most pads send their d-pad as a HAT
//   axis, not as keys, and the walk / the menu cursor are tied to it.
// * Only entries that DIFFER from the built-in layout are stored ([`Pad::normalize`]): a
//   file that froze the whole table would stop every later change to the defaults (the
//   2026-08-20 L2 / R2 swap, say) from ever reaching that player.

use std::collections::HashMap;

/// The player's table: keycode → what it does now. `None` = switched off (`NONE` in the
/// file). A code with no entry falls through to the built-in layout.
pub type Overrides<A> = HashMap<i32, Option<A>>;

/// How long the screen waits for a button after a function is picked.
pub const CAPTURE_MS: u128 = 5_000;

/// `KEYCODE_DPAD_UP..=KEYCODE_DPAD_RIGHT`. Not imported from `padhud.rs`: this file must
/// stand alone in its selftest.
const DPAD_DIRS: [i32; 4] = [19, 20, 21, 22];

/// May a press of `code` answer the 「버튼을 누르세요」 prompt?
///
/// ⛔ The four d-pad directions may not: they are the walk and the menu cursor, and on most
/// pads the d-pad is a hat axis that never arrives as these codes anyway — binding one here
/// would work on some pads and silently not on others. The hat's CENTRE (23) is a button.
pub fn capturable(code: i32) -> bool {
    code > 0 && !DPAD_DIRS.contains(&code)
}

/// Whole seconds left on the prompt after `elapsed_ms`, rounded UP (5, 4, … 1), or `None`
/// once it has run out.
pub fn seconds_left(elapsed_ms: u128) -> Option<u64> {
    (elapsed_ms < CAPTURE_MS).then(|| (CAPTURE_MS - elapsed_ms).div_ceil(1_000) as u64)
}

/// A function's button list as one line: none, the one, or the first and how many more.
pub fn describe(codes: &[i32], name: fn(i32) -> String) -> String {
    match codes {
        [] => "없음".to_string(),
        [one] => name(*one),
        [first, rest @ ..] => format!("{} 외 {}", name(*first), rest.len()),
    }
}

/// What [`Pad::assign`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assign<A> {
    /// The button already does that — nothing changed.
    Same,
    /// The button now does the action. What it did before (`displaced`) moved to `to`, the
    /// action's previous button — or, when the action had no button (`to == None`), lost
    /// this one.
    Done { displaced: Option<A>, to: Option<i32> },
    /// Refused: it would have left `lost` — an essential action — with no button at all.
    Refused { lost: A },
    /// A direction (see [`capturable`]).
    NotCapturable,
}

/// One game's pad, as this file needs to see it.
pub struct Pad<'a, A> {
    /// Every code the app knows a position for, in PHYSICAL order (face buttons first) — the
    /// order a function lists its buttons in and the tie-break for which one is "the" button.
    pub known: &'a [i32],
    /// Codes that are a second way to send something rather than a button of their own on
    /// most pads (the hat's centre, the BACK some pads send for B / SELECT). They do not
    /// count towards keeping an essential action reachable unless the player put them there.
    pub soft: &'a [i32],
    /// Actions that must always have a button (OK and CLR).
    pub essential: &'a [A],
    /// The built-in layout.
    pub default: fn(i32) -> Option<A>,
}

impl<A: Copy + Eq> Pad<'_, A> {
    /// What `code` does under `ov`.
    pub fn effective(&self, ov: &Overrides<A>, code: i32) -> Option<A> {
        match ov.get(&code) {
            Some(over) => *over,
            None => (self.default)(code),
        }
    }

    /// Every code worth asking about: the known ones in physical order, then any the player
    /// bound that the app has no position for, ascending.
    fn candidates(&self, ov: &Overrides<A>) -> Vec<i32> {
        let mut extra: Vec<i32> = ov.keys().copied().filter(|c| !self.known.contains(c)).collect();
        extra.sort_unstable();
        self.known.iter().copied().chain(extra).collect()
    }

    /// Every code that fires `action`, the PRIMARY FIRST: a button the player chose comes
    /// before a built-in one, each group in [`Self::candidates`] order.
    ///
    /// ★ That ordering is what makes a remap show up where the player put it. A plain scan in
    /// physical order would badge CLR on START (built-in) after the player moved it to R3,
    /// because START comes first in the pad's order.
    pub fn codes_for(&self, ov: &Overrides<A>, action: A) -> Vec<i32> {
        let all = self.candidates(ov);
        let fires = |c: i32| self.effective(ov, c) == Some(action);
        let chosen = all.iter().copied().filter(|c| ov.contains_key(c) && fires(*c));
        let built_in = all.iter().copied().filter(|c| !ov.contains_key(c) && fires(*c));
        chosen.chain(built_in).collect()
    }

    /// Can `action` be pressed at all — by a real button, not only by a soft duplicate?
    fn reachable(&self, ov: &Overrides<A>, action: A) -> bool {
        self.codes_for(ov, action).iter().any(|c| ov.contains_key(c) || !self.soft.contains(c))
    }

    /// Make `code` do `action`, swapping with the action's current button.
    ///
    /// ```text
    ///   before:  X = 퀵슬롯 1   Y = 퀵슬롯 2
    ///   퀵슬롯 1 ← Y
    ///   after:   Y = 퀵슬롯 1   X = 퀵슬롯 2
    /// ```
    ///
    /// Only the action's PRIMARY button takes part; a second button that also fires it (the
    /// hat's centre for OK, START and BACK for CLR) keeps doing so.
    ///
    /// ⛔ Refuses rather than strand an essential action. The swap alone cannot strand one —
    /// the displaced action always lands on the other button — except when the action being
    /// assigned had NO button: then what `code` did has nowhere to go, and if that was OK or
    /// CLR with nothing else to press it, the player would be locked out of the game.
    pub fn assign(&self, ov: &mut Overrides<A>, action: A, code: i32) -> Assign<A> {
        if !capturable(code) {
            return Assign::NotCapturable;
        }
        let displaced = self.effective(ov, code);
        if displaced == Some(action) {
            return Assign::Same;
        }
        let to = self.codes_for(ov, action).first().copied();
        let mut next = ov.clone();
        next.insert(code, Some(action));
        if let Some(from) = to {
            next.insert(from, displaced);
        }
        self.normalize(&mut next);
        for &lost in self.essential {
            if self.reachable(ov, lost) && !self.reachable(&next, lost) {
                return Assign::Refused { lost };
            }
        }
        *ov = next;
        Assign::Done { displaced, to }
    }

    /// Drop every entry that says what the built-in layout already says.
    pub fn normalize(&self, ov: &mut Overrides<A>) {
        ov.retain(|code, over| *over != (self.default)(*code));
    }

    /// The file: `header` (comment lines, each starting with `#`), then one
    /// `<code>=<token>` line per entry that differs from the built-in layout, ascending.
    /// `token(None)` must be `NONE`.
    pub fn serialize(&self, ov: &Overrides<A>, header: &str, token: fn(Option<A>) -> String) -> String {
        let mut codes: Vec<i32> = ov
            .iter()
            .filter(|(code, over)| **over != (self.default)(**code))
            .map(|(code, _)| *code)
            .collect();
        codes.sort_unstable();
        let mut out = String::from(header);
        for code in codes {
            out.push_str(&format!("{code}={}\n", token(ov[&code])));
        }
        out
    }
}
