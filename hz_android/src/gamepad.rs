//! A Bluetooth / USB gamepad → the handset keys the game expects. Pure (no window, no JVM), so
//! the quantiser, the button table and the remap file are tested on the host.
//!
//! Two routes, as hero2 found (its `gamepad.rs`, android-activity 0.6 + winit 0.30 read
//! end-to-end): **button and D-pad `KeyEvent`s reach winit** as `KeyboardInput` (D-pad →
//! `Arrow*`, buttons → `PhysicalKey::Unidentified(Android(96..))`) and are mapped in
//! [`crate::keys`] through [`button_key`] (the app consults the player's [`Overrides`] first,
//! [`key_for`]); **joystick axis `MotionEvent`s do not** — the glue's motion filter drops every
//! non-touchscreen motion event — so `MainActivity.dispatchGenericMotionEvent` reads the axes
//! and hands them to Rust over JNI (`crate::UserEvent::GamepadAxes`), where [`Pad::axes`] turns
//! them into key edges.
//!
//! The game walks ONE latched direction (`hz_java::input`), so the stick is quantised through
//! the same dead zones and turn ratio as the on-screen stick ([`crate::hud::joy_dir`]) with the
//! axis range as the radius. Analog triggers become digital with hysteresis ([`trigger_held`]):
//! many pads report L2/R2 only as axes, and those take whatever the player gave the L2/R2
//! BUTTON codes, so one remap line covers both forms of a trigger.
//!
//! **Remap file** `hz_gamepad.txt` (hero2's `wie_gamepad.txt` model): `<android keycode>=<TOKEN>`
//! per line, `#` comments at line start, tokens `OK LSK RSK 0`..`9 * #`, the two top tabs `SAVE MAP`
//! and `NONE` (the button does nothing). Only buttons: the D-pad directions are the walk and on most pads a hat axis,
//! so a line for them is ignored. The in-app editor is hero2's and hero3's 「패드 버튼 설정」 (B6,
//! 2026-10-26 — it replaced Phase 5-2's 4×4 of keys): the field HUD as it is played, every control
//! a pad button can be given lit where it sits ([`REMAP_ROWS`], `crate::hud::remap_scene`); tap
//! one, press the pad button; a conflict swaps and OK, the menu and back always keep a button
//! (`crate::padmap`, the two games' rules byte for byte); 저장 writes only what differs from the
//! built-in layout.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use wie_backend::KeyCode;

use crate::{hud, padmap};

// `android.view.KeyEvent` codes.
pub const KEYCODE_BUTTON_A: u32 = 96;
pub const KEYCODE_BUTTON_B: u32 = 97;
pub const KEYCODE_BUTTON_X: u32 = 99;
pub const KEYCODE_BUTTON_Y: u32 = 100;
pub const KEYCODE_BUTTON_L1: u32 = 102;
pub const KEYCODE_BUTTON_R1: u32 = 103;
pub const KEYCODE_BUTTON_L2: u32 = 104;
pub const KEYCODE_BUTTON_R2: u32 = 105;
pub const KEYCODE_BUTTON_THUMBL: u32 = 106;
pub const KEYCODE_BUTTON_THUMBR: u32 = 107;
pub const KEYCODE_BUTTON_START: u32 = 108;
pub const KEYCODE_BUTTON_SELECT: u32 = 109;

/// Every button this host knows a position for, in PHYSICAL order (face buttons first) — the
/// order a key lists its buttons in and the tie-break for which one is "the" button a badge
/// shows ([`codes_for`]).
pub const BUTTONS: [u32; 12] = [
    KEYCODE_BUTTON_A,
    KEYCODE_BUTTON_B,
    KEYCODE_BUTTON_X,
    KEYCODE_BUTTON_Y,
    KEYCODE_BUTTON_L1,
    KEYCODE_BUTTON_R1,
    KEYCODE_BUTTON_L2,
    KEYCODE_BUTTON_R2,
    KEYCODE_BUTTON_SELECT,
    KEYCODE_BUTTON_START,
    KEYCODE_BUTTON_THUMBL,
    KEYCODE_BUTTON_THUMBR,
];

/// Is `code` a pad button (as opposed to a phone key or a keyboard key that also arrives as an
/// Android keycode)? The app marks the pad as the last input source on these.
pub fn is_button(code: u32) -> bool {
    BUTTONS.contains(&code)
}

/// What a pad button does (hero2's `PadAction`): a key of the game's, or one of the host's two top
/// tabs, which no game key means — a pad-only player could not save or open the world map without
/// reaching for the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadAction {
    Key(KeyCode),
    /// The SAVE tab: the game's own save, at once (`touch::Out::QuickSave`).
    Save,
    /// The MAP tab: the world map.
    Map,
}

impl PadAction {
    /// The game key it presses, if it is one.
    pub fn key(self) -> Option<KeyCode> {
        match self {
            PadAction::Key(k) => Some(k),
            _ => None,
        }
    }

    /// The top tab it presses, if it is one ([`tab_action`] read backwards).
    pub fn tab(self) -> Option<hud::Tab> {
        match self {
            PadAction::Save => Some(hud::Tab::Save),
            PadAction::Map => Some(hud::Tab::Map),
            PadAction::Key(_) => None,
        }
    }
}

/// The action a top tab is to the pad (its badge, the pad screen's target on it).
pub fn tab_action(tab: hud::Tab) -> PadAction {
    match tab {
        hud::Tab::Save => PadAction::Save,
        hud::Tab::Map => PadAction::Map,
    }
}

/// What a pad button does. The game's own help (도움말 › 기본 조작): skills `1 3 7 9`, recovery
/// items `0 #`, minimap `*`, LSK = menu, RSK = back / STORE. A = OK, B = back as on every pad;
/// the face buttons X/Y and the shoulders take the four skills, the triggers the two items.
/// The left stick's press is SAVE — hero2's 빠른저장 (59차: the one button nothing used, and hard to
/// press by accident, which matters for a save) — and SELECT the world map (user 2026-10-27, *"hero2
/// 식"*); the right stick's press keeps the game's `*`.
pub fn button_action(code: u32) -> Option<PadAction> {
    Some(match code {
        KEYCODE_BUTTON_THUMBL => PadAction::Save,
        KEYCODE_BUTTON_SELECT => PadAction::Map,
        _ => PadAction::Key(button_key(code)?),
    })
}

/// The game key of [`button_action`], for the buttons that press one.
pub fn button_key(code: u32) -> Option<KeyCode> {
    Some(match code {
        KEYCODE_BUTTON_A => KeyCode::OK,
        KEYCODE_BUTTON_B => KeyCode::RIGHT_SOFT_KEY,
        KEYCODE_BUTTON_X => KeyCode::NUM1,
        KEYCODE_BUTTON_Y => KeyCode::NUM3,
        KEYCODE_BUTTON_L1 => KeyCode::NUM7,
        KEYCODE_BUTTON_R1 => KeyCode::NUM9,
        KEYCODE_BUTTON_L2 => KeyCode::NUM0,
        KEYCODE_BUTTON_R2 => KeyCode::HASH,
        KEYCODE_BUTTON_START => KeyCode::LEFT_SOFT_KEY,
        KEYCODE_BUTTON_THUMBR => KeyCode::STAR,
        _ => return None,
    })
}

/// What a pad press sends for what the game shows: A's OK, the pad's "go on", is `#` while the
/// game's 「# 확인」 notice is up (`ed.f:Z` — only `#` closes it, `ce.a(II)Z` @14, as the game's own
/// tip in the opening says) and LSK while the window with the keys answers only the soft keys
/// (`GuestView::lsk_ok`, the 창고 question — the touch OK does the same, `hud::ok_key`). Anything
/// else as it is. The app keeps what a press sent for its release.
pub fn view_key(action: PadAction, view: hz_java::GuestView) -> PadAction {
    match action {
        PadAction::Key(KeyCode::OK) if view.notice => PadAction::Key(KeyCode::HASH),
        PadAction::Key(KeyCode::OK) if view.lsk_ok => PadAction::Key(KeyCode::LEFT_SOFT_KEY),
        a => a,
    }
}

/// What a pad button's edge carries, `sent` remembering each press's (button → action): a press is
/// [`key_for`] through [`view_key`]; the system's repeat of it keeps that action, and the release
/// lets go of it. A held A that closed the 「# 확인」 notice as `#` must not turn into OK on its
/// repeats — the release would lift OK and leave `#` held (code review 2026-10-29).
pub fn button_edge(sent: &mut HashMap<u32, Option<PadAction>>, ov: &Overrides, code: u32, down: bool, repeat: bool, view: hz_java::GuestView) -> Option<PadAction> {
    if !down {
        return sent.remove(&code).unwrap_or_else(|| key_for(ov, code));
    }
    if repeat && let Some(&action) = sent.get(&code) {
        return action;
    }
    let action = key_for(ov, code).map(|a| view_key(a, view));
    sent.insert(code, action);
    action
}

/// The page buttons (user 2026-10-30: 「패드일때 다음페이지는 1,3번 버튼은 LB RB 버튼으로」): in a window
/// with pages (`GuestView::pager`) the quick functions `7` and `9` — LB and RB in the built-in layout —
/// turn back and on (`hz_java::menu_touch::page`: the shop's and 계정창고's `1` / `3`, a bag's pages,
/// the title's slot pages). Anywhere else they are the keys they always were.
pub const PAGE_KEYS: [(KeyCode, i8); 2] = [(KeyCode::NUM7, -1), (KeyCode::NUM9, 1)];

/// Where a pad function's press goes now ([`PlateRoute::edge`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Here {
    /// The game's keys.
    Keys,
    /// HZ-058's plates take the quick functions (`hud::PLATE_PAD_KEYS`) and START's `hud::PAGE_PAD_KEY`
    /// turns their page: the bare field, or a window's 「퀵슬롯등록」 waiting.
    Plates,
    /// A window with pages: [`PAGE_KEYS`] turn them.
    Pages,
}

/// HZ-058: where a pad function's edge goes. A quick function (`hud::PLATE_PAD_KEYS`) pressed where
/// the plates are used is plate `n`'s tap, and a page function ([`PAGE_KEYS`]) pressed in a window
/// with pages is a page turned; anything else is the game's key. **A release or a repeat follows the
/// path its press took**, whatever the game shows by then (PR review 2026-10-30: A — or a trigger
/// given OK — closed the 「# 확인」 notice as `#`, a key; its release came in the bare field, was taken
/// for plate 6 and swallowed, and `#` stayed held, so the next notice's 확인 was ignored). Remembered
/// per source — a button's keycode, a trigger's `KEYCODE_BUTTON_L2` / `R2` — so two sources sending
/// one key keep their own paths.
#[derive(Clone, Debug, Default)]
pub struct PlateRoute {
    /// The sources whose press went to a plate or a page and are still down.
    plates: Vec<u32>,
}

/// What [`PlateRoute::edge`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Tap plate `n` (0-based).
    Plate(u8),
    /// Turn the plates' page (HZ-058's two).
    PlatePage,
    /// Turn the window's page, −1 back · +1 on.
    Page(i8),
    /// The game's key path, as it is (`App::action_edge`).
    Key,
    /// Nothing: the repeat or release of a plate's tap or a page's turn.
    Swallow,
}

impl PlateRoute {
    pub fn edge(&mut self, source: u32, action: PadAction, pressed: bool, repeat: bool, here: Here) -> Route {
        if let Some(i) = self.plates.iter().position(|s| *s == source) {
            if !pressed {
                self.plates.swap_remove(i);
            }
            return Route::Swallow;
        }
        let PadAction::Key(k) = action else { return Route::Key };
        if !pressed || repeat {
            return Route::Key;
        }
        let route = match here {
            Here::Keys => None,
            Here::Plates if k == hud::PAGE_PAD_KEY => Some(Route::PlatePage),
            Here::Plates => hud::PLATE_PAD_KEYS.iter().position(|q| *q == k).map(|n| Route::Plate(n as u8)),
            Here::Pages => PAGE_KEYS.iter().find(|(q, _)| *q == k).map(|(_, d)| Route::Page(*d)),
        };
        match route {
            Some(r) => {
                self.plates.push(source);
                r
            }
            None => Route::Key,
        }
    }

    /// Everything let go at once (focus loss, suspend): no source is down any more.
    pub fn clear(&mut self) {
        self.plates.clear();
    }
}

// ------------------------------------------------------------------- the remap file ----

pub const MAP_FILE: &str = "hz_gamepad.txt";

/// The player's table: keycode → what it does now. `None` = switched off (`NONE` in the file).
/// A code with no entry falls through to [`button_action`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides(HashMap<u32, Option<PadAction>>);

/// The file token for an action — [`parse_token`] read backwards.
pub fn token(action: Option<PadAction>) -> &'static str {
    let key = match action {
        None => return "NONE",
        Some(PadAction::Save) => return "SAVE",
        Some(PadAction::Map) => return "MAP",
        Some(PadAction::Key(k)) => Some(k),
    };
    match key {
        None => "NONE",
        Some(KeyCode::OK) => "OK",
        Some(KeyCode::LEFT_SOFT_KEY) => "LSK",
        Some(KeyCode::RIGHT_SOFT_KEY) => "RSK",
        Some(KeyCode::NUM0) => "0",
        Some(KeyCode::NUM1) => "1",
        Some(KeyCode::NUM2) => "2",
        Some(KeyCode::NUM3) => "3",
        Some(KeyCode::NUM4) => "4",
        Some(KeyCode::NUM5) => "5",
        Some(KeyCode::NUM6) => "6",
        Some(KeyCode::NUM7) => "7",
        Some(KeyCode::NUM8) => "8",
        Some(KeyCode::NUM9) => "9",
        Some(KeyCode::STAR) => "*",
        Some(KeyCode::HASH) => "#",
        // Not a pad-assignable key (directions, phone keys): written as off rather than as a
        // token the reader would refuse.
        Some(_) => "NONE",
    }
}

/// `Some(None)` = `NONE`, `Some(Some(a))` = an action, `None` = not a token (the line is dropped).
/// Does NOT call `KeyCode::parse` — it `unimplemented!`s on unknown tokens, and a config typo
/// must never panic the app.
pub fn parse_token(s: &str) -> Option<Option<PadAction>> {
    let key = match s.trim() {
        "NONE" => return Some(None),
        "SAVE" => return Some(Some(PadAction::Save)),
        "MAP" => return Some(Some(PadAction::Map)),
        t => t,
    };
    Some(Some(PadAction::Key(match key {
        "OK" => KeyCode::OK,
        "LSK" => KeyCode::LEFT_SOFT_KEY,
        "RSK" => KeyCode::RIGHT_SOFT_KEY,
        "0" => KeyCode::NUM0,
        "1" => KeyCode::NUM1,
        "2" => KeyCode::NUM2,
        "3" => KeyCode::NUM3,
        "4" => KeyCode::NUM4,
        "5" => KeyCode::NUM5,
        "6" => KeyCode::NUM6,
        "7" => KeyCode::NUM7,
        "8" => KeyCode::NUM8,
        "9" => KeyCode::NUM9,
        "*" => KeyCode::STAR,
        "#" => KeyCode::HASH,
        _ => return None,
    })))
}

impl Overrides {
    /// Parse the file's text; the count is the lines that were not blank, not a comment and
    /// not a valid `<code>=<TOKEN>` (logged by the caller, never fatal). A direction keycode
    /// (19..=22) is dropped too: the walk is not remappable.
    pub fn from_text(text: &str) -> (Overrides, usize) {
        let mut map = HashMap::new();
        let mut dropped = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parsed = line
                .split_once('=')
                .and_then(|(c, t)| Some((c.trim().parse::<u32>().ok()?, parse_token(t)?)))
                .filter(|(c, _)| !(19..=22).contains(c) && *c > 0);
            match parsed {
                Some((code, key)) => {
                    map.insert(code, key);
                }
                None => dropped += 1,
            }
        }
        (Overrides(map), dropped)
    }

    pub fn to_text(&self) -> String {
        let mut codes: Vec<u32> = self.0.keys().copied().collect();
        codes.sort_unstable();
        let mut out = String::from(
            "# hz_gamepad.txt - gamepad button remap: <android keycode>=<TOKEN>\n\
             # tokens: OK LSK RSK 0 1 2 3 4 5 6 7 8 9 * # SAVE MAP NONE\n\
             # built-in: 96(A)=OK 97(B)=RSK 99(X)=1 100(Y)=3 102(L1)=7 103(R1)=9 104(L2)=0\n\
             #           105(R2)=# 108(START)=LSK 109(SELECT)=MAP 106(L3)=SAVE 107(R3)=*\n",
        );
        for code in codes {
            out.push_str(&format!("{code}={}\n", token(self.0[&code])));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Does the player have a line for `code`?
    pub fn has(&self, code: u32) -> bool {
        self.0.contains_key(&code)
    }

    pub fn set(&mut self, code: u32, action: Option<PadAction>) {
        self.0.insert(code, action);
    }

    /// Read `dir/hz_gamepad.txt`; missing = no overrides. A line that says what the built-in layout
    /// says is dropped as it is read, so the settings row's 「사용자 배치」 means a real change.
    pub fn load(dir: &Path) -> (Overrides, usize) {
        match std::fs::read_to_string(path(dir)) {
            Ok(text) => {
                let (ov, bad) = Overrides::from_text(&text);
                (Overrides::from_table(ov.table()), bad)
            }
            Err(_) => (Overrides::default(), 0),
        }
    }

    /// Write `dir/hz_gamepad.txt` — the PAD REMAP panel's result (`crate::touch::Hud::pad_button`).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::write(path(dir), self.to_text())
    }

    /// Every button the player switched off (`NONE`), ascending.
    pub fn codes_off(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.0.iter().filter(|(_, k)| k.is_none()).map(|(c, _)| *c).collect();
        v.sort_unstable();
        v
    }

    /// The table as `crate::padmap` takes it.
    pub fn table(&self) -> padmap::Overrides<PadAction> {
        self.0.iter().filter_map(|(c, k)| Some((i32::try_from(*c).ok()?, *k))).collect()
    }

    /// Back from `crate::padmap`, only what differs from the built-in layout (hero2's `normalize`: a
    /// file that froze the whole table would keep every later change of the defaults from it).
    pub fn from_table(mut t: padmap::Overrides<PadAction>) -> Overrides {
        pad().normalize(&mut t);
        Overrides(t.into_iter().filter_map(|(c, k)| Some((u32::try_from(c).ok()?, k))).collect())
    }
}

// ------------------------------------------------------------- the pad screen (B6) ----

/// What the pad screen offers a button for (hero2's `REMAP_ROWS`): the keys the game plays with,
/// each with its name — a `crate::host_text` line. The game's own help (도움말 › 기본 조작) is
/// the vocabulary: OK, back (RSK — the field's menu too), the quick slots `1 3 7 9 0 #` and `*` (the
/// seventh plate, 2026-10-30), LSK (the plates' page in the field, `hud::PAGE_PAD_KEY`); then the
/// host's two tabs. `7` / `9` turn a window's pages ([`PAGE_KEYS`]). Not the walk (directions are not remappable) nor the keypad's `2 4 5 6 8`, which
/// no field control asks for — a line in the file still binds them.
pub const REMAP_ROWS: [(PadAction, &str); 12] = [
    (PadAction::Key(KeyCode::OK), "OK - ATTACK"),
    (PadAction::Key(KeyCode::LEFT_SOFT_KEY), "QUICK PAGE"),
    (PadAction::Key(KeyCode::RIGHT_SOFT_KEY), "BACK - MENU"),
    (PadAction::Key(KeyCode::NUM1), "QUICK SLOT 1"),
    (PadAction::Key(KeyCode::NUM3), "QUICK SLOT 3"),
    (PadAction::Key(KeyCode::NUM7), "QUICK SLOT 7 - PAGE BACK"),
    (PadAction::Key(KeyCode::NUM9), "QUICK SLOT 9 - PAGE ON"),
    (PadAction::Key(KeyCode::NUM0), "QUICK SLOT 0"),
    (PadAction::Key(KeyCode::HASH), "QUICK SLOT #"),
    (PadAction::Key(KeyCode::STAR), "QUICK SLOT *"),
    (PadAction::Save, "SAVE TAB"),
    (PadAction::Map, "MAP TAB"),
];

/// The [`REMAP_ROWS`] index of an action.
pub fn remap_row(action: PadAction) -> Option<usize> {
    REMAP_ROWS.iter().position(|(a, _)| *a == action)
}

/// An action's name for the pad screen's lines: its row's, a keypad digit's, or "nothing".
pub fn key_label(action: Option<PadAction>) -> &'static str {
    match action {
        None => "NO BUTTON",
        Some(a) => remap_row(a).map_or_else(
            || match a.key().unwrap_or(KeyCode::OK) {
                KeyCode::NUM2 => "KEY 2",
                KeyCode::NUM4 => "KEY 4",
                KeyCode::NUM5 => "KEY 5",
                KeyCode::NUM6 => "KEY 6",
                KeyCode::NUM8 => "KEY 8",
                _ => "ANOTHER KEY",
            },
            |r| REMAP_ROWS[r].1,
        ),
    }
}

/// A pad button's name (hero2's `button_name`): where it is on the pad, and what an Xbox pad prints
/// there — positions first, as the badges are (`crate::padhud`).
pub fn button_name(code: u32) -> &'static str {
    use crate::padhud::{FacePos, PadPos};
    match crate::padhud::pos_for_code(code) {
        Some(PadPos::Face(FacePos::Up)) => "FACE TOP (XBOX Y)",
        Some(PadPos::Face(FacePos::Down)) => "FACE BOTTOM (XBOX A)",
        Some(PadPos::Face(FacePos::Left)) => "FACE LEFT (XBOX X)",
        Some(PadPos::Face(FacePos::Right)) => "FACE RIGHT (XBOX B)",
        Some(PadPos::Shoulder { right: false, lower: false }) => "LEFT BUMPER (L1)",
        Some(PadPos::Shoulder { right: false, lower: true }) => "LEFT TRIGGER (L2)",
        Some(PadPos::Shoulder { right: true, lower: false }) => "RIGHT BUMPER (R1)",
        Some(PadPos::Shoulder { right: true, lower: true }) => "RIGHT TRIGGER (R2)",
        Some(PadPos::Middle { right: false }) => "MIDDLE LEFT (SELECT)",
        Some(PadPos::Middle { right: true }) => "MIDDLE RIGHT (START)",
        Some(PadPos::StickPress { right: false }) => "LEFT STICK PRESS (L3)",
        Some(PadPos::StickPress { right: true }) => "RIGHT STICK PRESS (R3)",
        None => "ANOTHER BUTTON",
    }
}

/// [`BUTTONS`] as `crate::padmap` counts codes.
const KNOWN: [i32; BUTTONS.len()] = {
    let mut out = [0i32; BUTTONS.len()];
    let mut i = 0;
    while i < BUTTONS.len() {
        out[i] = BUTTONS[i] as i32;
        i += 1;
    }
    out
};

/// The three a player cannot be left without (hero2's OK and CLR — here the game's "back" and its
/// menu are two keys): choosing in a window, opening the menu, leaving a window.
static ESSENTIAL: [PadAction; 3] = [PadAction::Key(KeyCode::OK), PadAction::Key(KeyCode::LEFT_SOFT_KEY), PadAction::Key(KeyCode::RIGHT_SOFT_KEY)];

fn built_in(code: i32) -> Option<PadAction> {
    u32::try_from(code).ok().and_then(button_action)
}

/// This game's pad as `crate::padmap` sees it. No soft codes: the phone's BACK and a hat's centre
/// do not arrive as pad buttons here (`crate::keys` maps BACK to the game's back on its own).
pub fn pad() -> padmap::Pad<'static, PadAction> {
    padmap::Pad { known: &KNOWN, soft: &[], essential: &ESSENTIAL, default: built_in }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(MAP_FILE)
}

/// What `code` does under `ov`: the player's line, else the built-in layout.
pub fn key_for(ov: &Overrides, code: u32) -> Option<PadAction> {
    match ov.0.get(&code) {
        Some(over) => *over,
        None => button_action(code),
    }
}

/// Every button that fires `action`, the PRIMARY first: a button the player chose comes before a
/// built-in one, each group in [`BUTTONS`] order, then any code the player bound that the app
/// has no position for, ascending. That ordering is what makes a remap show up where the player
/// put it on the badges.
pub fn codes_for(ov: &Overrides, action: PadAction) -> Vec<u32> {
    let mut extra: Vec<u32> = ov.0.keys().copied().filter(|c| !BUTTONS.contains(c)).collect();
    extra.sort_unstable();
    let all: Vec<u32> = BUTTONS.iter().copied().chain(extra).collect();
    let fires = |c: &u32| key_for(ov, *c) == Some(action);
    let chosen = all.iter().copied().filter(|c| ov.has(*c)).filter(fires);
    let built_in = all.iter().copied().filter(|c| !ov.has(*c)).filter(fires);
    chosen.chain(built_in).collect()
}

/// An analog trigger's digital edge: held past 0.5, released below 0.3, so a trigger resting
/// on a threshold cannot chatter.
pub fn trigger_held(was_held: bool, value: f32) -> bool {
    if was_held { value > 0.3 } else { value > 0.5 }
}

/// The pad's latched state between axis samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    /// The direction the stick (or hat) holds — a diagonal's two arrows in the bare field (HZ-059).
    pub way: Option<hud::Way>,
    pub lt: bool,
    pub rt: bool,
    /// What the analog triggers press: the L2/R2 button codes' actions ([`key_for`]), `None` =
    /// switched off.
    pub lt_key: Option<PadAction>,
    pub rt_key: Option<PadAction>,
    /// What each trigger's press sent ([`view_key`] of its key), for its release.
    lt_sent: Option<PadAction>,
    rt_sent: Option<PadAction>,
    /// The last value each trigger read (0..1).
    lt_value: f32,
    rt_value: f32,
    /// A trigger pulled when the input came back ([`Pad::release_all`], [`Pad::resume_triggers`]) presses
    /// nothing until it has read at rest (code review 2026-10-31: on the PC, L2 pulled in another window
    /// and let go slowly after Alt+Tab read 1.0 → 0.8 as a fresh press and fired a quick slot).
    lt_muted: bool,
    rt_muted: bool,
}

impl Default for Pad {
    fn default() -> Self {
        Pad::new(&Overrides::default())
    }
}

impl Pad {
    pub fn new(ov: &Overrides) -> Pad {
        Pad {
            way: None,
            lt: false,
            rt: false,
            lt_key: key_for(ov, KEYCODE_BUTTON_L2),
            rt_key: key_for(ov, KEYCODE_BUTTON_R2),
            lt_sent: None,
            rt_sent: None,
            lt_value: 0.0,
            rt_value: 0.0,
            lt_muted: false,
            rt_muted: false,
        }
    }

    /// One axis sample: left stick `x`/`y`, hat `hx`/`hy` (the D-pad on most pads reports as
    /// hat axes), triggers `lt`/`rt`, all −1..1 / 0..1 as Android gives them. Returns the key
    /// edges to send, old key up before new key down — [`Pad::stick`]'s (four ways), then [`Pad::triggers`].
    pub fn axes(&mut self, x: f32, y: f32, hx: f32, hy: f32, lt: f32, rt: f32) -> Vec<(PadAction, bool)> {
        let mut out: Vec<(PadAction, bool)> = self.stick(x, y, hx, hy, false).into_iter().map(|(k, d)| (PadAction::Key(k), d)).collect();
        out.extend(self.triggers(lt, rt));
        out
    }

    /// The stick and hat half of a sample: the direction's edges — eight ways when `eight` (the bare
    /// field, HZ-059: a hat's corner or a stick's diagonal holds two arrows), else four.
    pub fn stick(&mut self, x: f32, y: f32, hx: f32, hy: f32, eight: bool) -> Vec<(KeyCode, bool)> {
        // Stick + hat summed: whichever the thumb is on wins, and both at rest is rest.
        let (dx, dy) = (f64::from(x + hx).clamp(-1.0, 1.0), f64::from(y + hy).clamp(-1.0, 1.0));
        let want = hud::joy_way(dx, dy, 1.0, self.way, eight);
        if want == self.way {
            return Vec::new();
        }
        let out = hud::way_edges(self.way, want);
        self.way = want;
        out
    }

    /// The trigger half of a sample: the edges of whatever the player gave L2/R2 — never an arrow
    /// (directions are not pad-assignable).
    pub fn triggers(&mut self, lt: f32, rt: f32) -> Vec<(PadAction, bool)> {
        self.triggers_in(lt, rt, hz_java::GuestView::default())
    }

    /// [`Pad::triggers`] for what the game shows: a press is what a button's would be
    /// ([`view_key`] — a trigger given OK closes the 「# 확인」 notice as A does), a release lets go of
    /// what its press sent.
    pub fn triggers_in(&mut self, lt: f32, rt: f32, view: hz_java::GuestView) -> Vec<(PadAction, bool)> {
        self.trigger_edges(lt, rt, view).into_iter().map(|(_, action, down)| (action, down)).collect()
    }

    /// [`Pad::triggers_in`] with the button each edge is (`KEYCODE_BUTTON_L2` / `R2`) — for a caller
    /// that routes a press and must route its release the same way ([`PlateRoute`]).
    pub fn trigger_edges(&mut self, lt: f32, rt: f32, view: hz_java::GuestView) -> Vec<(u32, PadAction, bool)> {
        let mut out = Vec::new();
        let triggers = [
            (KEYCODE_BUTTON_L2, &mut self.lt, lt, self.lt_key, &mut self.lt_sent, &mut self.lt_value, &mut self.lt_muted),
            (KEYCODE_BUTTON_R2, &mut self.rt, rt, self.rt_key, &mut self.rt_sent, &mut self.rt_value, &mut self.rt_muted),
        ];
        for (code, held, value, key, sent, last, muted) in triggers {
            *last = value;
            if *muted {
                // Still pulled from before: nothing until it reads at rest (it holds nothing to let go of).
                *muted = trigger_held(true, value);
                continue;
            }
            let now = trigger_held(*held, value);
            if now != *held {
                *held = now;
                if now {
                    *sent = key.map(|k| view_key(k, view));
                    out.extend(sent.map(|k| (code, k, true)));
                } else if let Some(k) = sent.take() {
                    out.push((code, k, false));
                }
            }
        }
        out
    }

    /// The triggers' values when the input comes back (the PC: what gilrs read while the window was in
    /// the background): one still pulled presses nothing until it has read at rest. Sends nothing.
    pub fn resume_triggers(&mut self, lt: f32, rt: f32) {
        (self.lt_value, self.rt_value) = (lt, rt);
        self.lt_muted = !self.lt && trigger_held(true, lt);
        self.rt_muted = !self.rt && trigger_held(true, rt);
    }

    /// Everything the pad holds goes up (suspend, focus loss). A trigger pulled now stays muted until it
    /// reads at rest — its release, likely slow, must not come back as a press (the phone gets no sample
    /// while away; the PC then says what the background read, [`Pad::resume_triggers`]).
    pub fn release_all(&mut self) -> Vec<(PadAction, bool)> {
        self.lt_muted |= trigger_held(true, self.lt_value);
        self.rt_muted |= trigger_held(true, self.rt_value);
        let mut out = Vec::new();
        out.extend(hud::way_edges(self.way.take(), None).into_iter().map(|(k, d)| (PadAction::Key(k), d)));
        if std::mem::take(&mut self.lt)
            && let Some(k) = self.lt_sent.take()
        {
            out.push((k, false));
        }
        if std::mem::take(&mut self.rt)
            && let Some(k) = self.rt_sent.take()
        {
            out.push((k, false));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PR review 2026-10-30: a release follows its press, not the scene it comes in.
    #[test]
    fn a_release_takes_the_path_its_press_took() {
        let notice = hz_java::GuestView { scene: hz_java::Scene::Field, notice: true, ..Default::default() };
        let mut r = PlateRoute::default();
        // A closes the 「# 확인」 notice as #, a key (the plates take no press under the notice) …
        let hash = view_key(k(KeyCode::OK), notice);
        assert_eq!(hash, k(KeyCode::HASH));
        assert_eq!(r.edge(KEYCODE_BUTTON_A, hash, true, false, Here::Keys), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_A, hash, true, true, Here::Plates), Route::Key, "its repeat in the field is still the key's");
        // … and its release in the bare field lets go of # — not plate 6's tap.
        assert_eq!(r.edge(KEYCODE_BUTTON_A, hash, false, false, Here::Plates), Route::Key);
        // A trigger given OK, the same way.
        assert_eq!(r.edge(KEYCODE_BUTTON_L2, hash, true, false, Here::Keys), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_L2, hash, false, false, Here::Plates), Route::Key);
        // A quick function pressed in the field is a plate's tap: its repeat and its release in a
        // window are nothing (no stray key up), and the next press there is the window's digit.
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, false, Here::Plates), Route::Plate(2));
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, true, Here::Keys), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), false, false, Here::Keys), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, false, Here::Keys), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), false, false, Here::Plates), Route::Key, "pressed as a digit, let go as one");
        // Two sources on one key keep their own paths.
        assert_eq!(r.edge(KEYCODE_BUTTON_R2, k(KeyCode::HASH), true, false, Here::Plates), Route::Plate(5));
        assert_eq!(r.edge(KEYCODE_BUTTON_Y, k(KeyCode::HASH), true, false, Here::Keys), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_Y, k(KeyCode::HASH), false, false, Here::Plates), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_R2, k(KeyCode::HASH), false, false, Here::Keys), Route::Swallow);
        // Not a quick function: always the key's.
        assert_eq!(r.edge(KEYCODE_BUTTON_B, k(KeyCode::RIGHT_SOFT_KEY), true, false, Here::Plates), Route::Key);
        r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM1), true, false, Here::Plates);
        r.clear();
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM1), false, false, Here::Keys), Route::Key, "after a clear nothing is a plate's");
    }

    /// User 2026-10-30: in a window with pages the page functions (LB / RB = 7 / 9) turn them — their
    /// repeat and release are nothing, wherever they come — X / Y stay the game's 1 / 3 there, and
    /// without pages LB / RB are 7 / 9 as before.
    #[test]
    fn the_page_buttons_turn_a_windows_pages() {
        let mut r = PlateRoute::default();
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, false, Here::Pages), Route::Page(-1));
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, true, Here::Pages), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), false, false, Here::Keys), Route::Swallow, "let go after the window closed");
        assert_eq!(r.edge(KEYCODE_BUTTON_R1, k(KeyCode::NUM9), true, false, Here::Pages), Route::Page(1));
        assert_eq!(r.edge(KEYCODE_BUTTON_R1, k(KeyCode::NUM9), false, false, Here::Pages), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_X, k(KeyCode::NUM1), true, false, Here::Pages), Route::Key, "X is the game's 1 there too");
        assert_eq!(r.edge(KEYCODE_BUTTON_X, k(KeyCode::NUM1), false, false, Here::Pages), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), true, false, Here::Keys), Route::Key, "no pages: 7 as before");
        assert_eq!(r.edge(KEYCODE_BUTTON_L1, k(KeyCode::NUM7), false, false, Here::Pages), Route::Key, "pressed as a digit, let go as one");
        assert_eq!(r.edge(KEYCODE_BUTTON_Y, k(KeyCode::NUM9), true, false, Here::Pages), Route::Page(1), "whichever button the player gave 9");
    }

    /// User 2026-10-30 (「start로 하자」): where the plates are used START's LSK turns their page and R3's
    /// `*` presses plate 7; over a window they are the soft key and `*` as before.
    #[test]
    fn start_turns_the_plates_and_r3_is_the_seventh() {
        let mut r = PlateRoute::default();
        let (start, r3) = (key_for(&Overrides::default(), KEYCODE_BUTTON_START).unwrap(), key_for(&Overrides::default(), KEYCODE_BUTTON_THUMBR).unwrap());
        assert_eq!(r.edge(KEYCODE_BUTTON_START, start, true, false, Here::Plates), Route::PlatePage);
        assert_eq!(r.edge(KEYCODE_BUTTON_START, start, false, false, Here::Plates), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_THUMBR, r3, true, false, Here::Plates), Route::Plate(6));
        assert_eq!(r.edge(KEYCODE_BUTTON_THUMBR, r3, false, false, Here::Keys), Route::Swallow);
        assert_eq!(r.edge(KEYCODE_BUTTON_START, start, true, false, Here::Keys), Route::Key, "a window's soft key");
        assert_eq!(r.edge(KEYCODE_BUTTON_START, start, false, false, Here::Keys), Route::Key);
        assert_eq!(r.edge(KEYCODE_BUTTON_THUMBR, r3, true, false, Here::Pages), Route::Key, "* over a window with pages");
    }

    #[test]
    fn a_trigger_edge_names_its_trigger() {
        let mut p = Pad::new(&Overrides::default());
        let view = hz_java::GuestView::default();
        let pressed = p.trigger_edges(0.9, 0.0, view);
        assert!(pressed.iter().all(|(c, _, d)| *c == KEYCODE_BUTTON_L2 && *d), "{pressed:?}");
        let both = p.trigger_edges(0.0, 0.9, view);
        assert!(both.iter().any(|(c, _, d)| *c == KEYCODE_BUTTON_L2 && !*d) && both.iter().all(|(c, _, d)| (*c == KEYCODE_BUTTON_R2) == *d), "{both:?}");
    }

    const fn k(key: KeyCode) -> PadAction {
        PadAction::Key(key)
    }

    #[test]
    fn buttons_follow_the_games_help() {
        assert_eq!(button_key(KEYCODE_BUTTON_A), Some(KeyCode::OK));
        assert_eq!(button_key(KEYCODE_BUTTON_B), Some(KeyCode::RIGHT_SOFT_KEY));
        assert_eq!(button_key(KEYCODE_BUTTON_START), Some(KeyCode::LEFT_SOFT_KEY));
        assert_eq!(button_key(KEYCODE_BUTTON_X), Some(KeyCode::NUM1));
        assert_eq!(button_key(KEYCODE_BUTTON_R2), Some(KeyCode::HASH));
        assert_eq!(button_key(4), None, "BACK is the phone's, mapped in keys.rs");
        // hero2's: the left stick's press saves, SELECT is the world map, the right stick's press `*`.
        assert_eq!(button_action(KEYCODE_BUTTON_THUMBL), Some(PadAction::Save));
        assert_eq!(button_action(KEYCODE_BUTTON_SELECT), Some(PadAction::Map));
        assert_eq!(button_action(KEYCODE_BUTTON_THUMBR), Some(k(KeyCode::STAR)));
        assert_eq!((button_key(KEYCODE_BUTTON_THUMBL), button_key(KEYCODE_BUTTON_SELECT)), (None, None), "no game key");
        let view = |notice, lsk_ok| hz_java::GuestView { notice, lsk_ok, ..Default::default() };
        assert_eq!(view_key(k(KeyCode::OK), view(true, false)), k(KeyCode::HASH), "A closes the 「# 확인」 notice");
        assert_eq!(view_key(k(KeyCode::OK), view(false, false)), k(KeyCode::OK));
        assert_eq!(view_key(k(KeyCode::OK), view(false, true)), k(KeyCode::LEFT_SOFT_KEY), "A answers the 창고 question");
        assert_eq!(view_key(k(KeyCode::RIGHT_SOFT_KEY), view(true, true)), k(KeyCode::RIGHT_SOFT_KEY), "B still skips");
        assert_eq!(view_key(PadAction::Save, view(true, true)), PadAction::Save);
        assert!(BUTTONS.iter().all(|c| button_action(*c).is_some()), "every known button does something");
        assert!(is_button(KEYCODE_BUTTON_A) && !is_button(4) && !is_button(19));
    }

    #[test]
    fn the_stick_latches_one_direction_and_turns_old_up_new_down() {
        let mut p = Pad::default();
        assert!(p.axes(0.2, 0.0, 0.0, 0.0, 0.0, 0.0).is_empty(), "inside the dead zone");
        assert_eq!(p.axes(0.8, 0.0, 0.0, 0.0, 0.0, 0.0), vec![(k(KeyCode::RIGHT), true)]);
        assert!(p.axes(0.9, 0.1, 0.0, 0.0, 0.0, 0.0).is_empty(), "same direction, nothing");
        assert_eq!(
            p.axes(0.3, 0.9, 0.0, 0.0, 0.0, 0.0),
            vec![(k(KeyCode::RIGHT), false), (k(KeyCode::DOWN), true)]
        );
        assert_eq!(p.axes(0.0, 0.0, 0.0, 0.0, 0.0, 0.0), vec![(k(KeyCode::DOWN), false)]);
        // The hat is the D-pad on most pads.
        assert_eq!(p.axes(0.0, 0.0, 0.0, -1.0, 0.0, 0.0), vec![(k(KeyCode::UP), true)]);
        assert_eq!(p.release_all(), vec![(k(KeyCode::UP), false)]);
        assert!(p.release_all().is_empty());
    }

    /// HZ-059: in the bare field a hat's corner and a stick's diagonal hold two arrows.
    #[test]
    fn the_fields_stick_and_hat_walk_diagonals() {
        let mut p = Pad::default();
        assert_eq!(p.stick(0.0, 0.0, 1.0, 1.0, true), vec![(KeyCode::RIGHT, true), (KeyCode::DOWN, true)], "the hat's corner");
        assert_eq!(p.stick(0.0, 0.0, 0.0, 1.0, true), vec![(KeyCode::RIGHT, false)]);
        assert_eq!(p.stick(-0.7, -0.7, 0.0, 0.0, true), vec![(KeyCode::DOWN, false), (KeyCode::LEFT, true), (KeyCode::UP, true)]);
        assert_eq!(p.release_all(), vec![(k(KeyCode::LEFT), false), (k(KeyCode::UP), false)]);
        assert_eq!(p.stick(0.0, 0.0, 1.0, 1.0, false).len(), 1, "a window: four ways");
    }

    #[test]
    fn triggers_have_hysteresis() {
        assert!(!trigger_held(false, 0.45));
        assert!(trigger_held(false, 0.55));
        assert!(trigger_held(true, 0.35), "still held above the release point");
        assert!(!trigger_held(true, 0.25));
        let mut p = Pad::default();
        assert_eq!(p.axes(0.0, 0.0, 0.0, 0.0, 0.9, 0.0), vec![(k(KeyCode::NUM0), true)]);
        assert!(p.axes(0.0, 0.0, 0.0, 0.0, 0.4, 0.0).is_empty());
        assert_eq!(p.axes(0.0, 0.0, 0.0, 0.0, 0.0, 0.7), vec![(k(KeyCode::NUM0), false), (k(KeyCode::HASH), true)]);
    }

    /// Code review 2026-10-31: a trigger still pulled when the input comes back is no new press while it is
    /// let go slowly (1.0 → 0.8 used to read as one and fire a quick slot), and presses again once at rest.
    #[test]
    fn a_trigger_pulled_across_a_focus_change_waits_for_rest() {
        let l2 = k(KeyCode::NUM0);
        // The phone: L2 held through the notification shade (no sample comes while away).
        let mut p = Pad::default();
        assert_eq!(p.triggers(1.0, 0.0), vec![(l2, true)]);
        assert_eq!(p.release_all(), vec![(l2, false)]);
        for v in [0.95, 0.8, 0.6, 0.4, 0.31] {
            assert!(p.triggers(v, 0.0).is_empty(), "{v}: no press while it is let go");
        }
        assert!(p.triggers(0.2, 0.0).is_empty(), "at rest: nothing to let go of");
        assert_eq!(p.triggers(0.9, 0.0), vec![(l2, true)], "the next pull is a press");
        // A trigger at rest when the focus went presses at once on return — a digital one too.
        let mut p = Pad::default();
        p.release_all();
        assert_eq!(p.triggers(1.0, 0.0), vec![(l2, true)]);
        // The PC: pulled only while the window was away — gilrs's background reading says so on return.
        let mut p = Pad::default();
        p.release_all();
        p.resume_triggers(1.0, 0.0);
        assert!(p.triggers(0.8, 0.0).is_empty() && p.triggers(0.5, 0.0).is_empty());
        assert_eq!(p.triggers(0.5, 0.9), vec![(k(KeyCode::HASH), true)], "R2 was at rest: its pull is a press");
        assert!(p.triggers(0.1, 0.9).is_empty());
        assert_eq!(p.triggers(0.9, 0.9), vec![(l2, true)]);
        // … and pulled before, let go while away: back at rest, the next pull presses.
        let mut p = Pad::default();
        p.triggers(1.0, 0.0);
        p.release_all();
        p.resume_triggers(0.0, 0.0);
        assert_eq!(p.triggers(0.9, 0.0), vec![(l2, true)]);
    }

    /// Code review 2026-10-29: A held through the 「# 확인」 notice it closed — the repeats keep `#`,
    /// and the release lifts `#`, not the OK the view now says.
    #[test]
    fn a_held_button_lets_go_of_what_its_press_sent() {
        let notice = hz_java::GuestView { notice: true, ..Default::default() };
        let (ov, mut sent) = (Overrides::default(), HashMap::new());
        let edge = |sent: &mut HashMap<_, _>, down, repeat, view| button_edge(sent, &ov, KEYCODE_BUTTON_A, down, repeat, view);
        assert_eq!(edge(&mut sent, true, false, notice), Some(k(KeyCode::HASH)));
        assert_eq!(edge(&mut sent, true, true, hz_java::GuestView::default()), Some(k(KeyCode::HASH)), "a repeat after the notice closed");
        assert_eq!(edge(&mut sent, false, false, hz_java::GuestView::default()), Some(k(KeyCode::HASH)));
        assert_eq!(edge(&mut sent, true, false, hz_java::GuestView::default()), Some(k(KeyCode::OK)), "the next press is the view's");
    }

    /// Code review 2026-10-29: a trigger given OK answers the 「# 확인」 notice and the 창고 question as
    /// A does, and lets go of what it sent.
    #[test]
    fn a_trigger_given_ok_answers_what_a_answers() {
        let mut ov = Overrides::default();
        ov.set(KEYCODE_BUTTON_L2, Some(k(KeyCode::OK)));
        let mut p = Pad::new(&ov);
        let notice = hz_java::GuestView { notice: true, ..Default::default() };
        assert_eq!(p.triggers_in(0.9, 0.0, notice), vec![(k(KeyCode::HASH), true)]);
        assert_eq!(p.triggers_in(0.0, 0.0, hz_java::GuestView::default()), vec![(k(KeyCode::HASH), false)]);
        let question = hz_java::GuestView { lsk_ok: true, ..Default::default() };
        assert_eq!(p.triggers_in(0.9, 0.0, question), vec![(k(KeyCode::LEFT_SOFT_KEY), true)]);
        assert_eq!(p.release_all(), vec![(k(KeyCode::LEFT_SOFT_KEY), false)]);
        assert_eq!(p.triggers_in(0.0, 0.9, notice), vec![(k(KeyCode::HASH), true)], "R2's own # stays #");
    }

    #[test]
    fn the_remap_file_round_trips_and_a_typo_is_dropped_not_fatal() {
        let (ov, dropped) = Overrides::from_text(
            "# comment\n\n 99 = 7 \n105=NONE\n300=OK\n19=OK\nabc=1\n96=BANANA\n96\n103=MAP\n",
        );
        assert_eq!(dropped, 4, "the d-pad line, two garbage lines and the missing '='");
        assert_eq!(key_for(&ov, 99), Some(k(KeyCode::NUM7)), "X now fires 7");
        assert_eq!(key_for(&ov, 105), None, "R2 switched off");
        assert_eq!(key_for(&ov, 300), Some(k(KeyCode::OK)), "an unknown code can still be bound");
        assert_eq!(key_for(&ov, 96), Some(k(KeyCode::OK)), "untouched = built-in");
        assert_eq!(key_for(&ov, 103), Some(PadAction::Map), "R1 opens the world map");
        let (again, d2) = Overrides::from_text(&ov.to_text());
        assert_eq!((again, d2), (ov.clone(), 0));
        for a in [k(KeyCode::OK), k(KeyCode::LEFT_SOFT_KEY), k(KeyCode::NUM0), k(KeyCode::STAR), k(KeyCode::HASH), PadAction::Save, PadAction::Map] {
            assert_eq!(parse_token(token(Some(a))), Some(Some(a)));
        }
        assert_eq!(parse_token("NONE"), Some(None));
        assert_eq!(parse_token("UP"), None, "directions are not pad-assignable");
    }

    /// B6, hero2's rules on this game's pad: a conflict swaps, the three essentials never lose their
    /// button, a direction is no answer, and only what differs from the built-in layout is kept.
    #[test]
    fn the_pad_screen_swaps_keeps_the_essentials_and_stores_the_difference() {
        let pad = pad();
        let mut t = Overrides::default().table();
        // Quick slot 1 (X) onto Y: Y fires 1, X takes Y's 3.
        let r = pad.assign(&mut t, k(KeyCode::NUM1), KEYCODE_BUTTON_Y as i32);
        assert_eq!(r, padmap::Assign::Done { displaced: Some(k(KeyCode::NUM3)), to: Some(KEYCODE_BUTTON_X as i32) });
        let ov = Overrides::from_table(t.clone());
        assert_eq!((key_for(&ov, KEYCODE_BUTTON_Y), key_for(&ov, KEYCODE_BUTTON_X)), (Some(k(KeyCode::NUM1)), Some(k(KeyCode::NUM3))));
        // Back again: the table is empty — nothing differs.
        pad.assign(&mut t, k(KeyCode::NUM1), KEYCODE_BUTTON_X as i32);
        assert!(Overrides::from_table(t.clone()).is_empty());
        // OK onto the button it already has: nothing.
        assert_eq!(pad.assign(&mut t, k(KeyCode::OK), KEYCODE_BUTTON_A as i32), padmap::Assign::Same);
        assert_eq!(pad.assign(&mut t, k(KeyCode::OK), 19), padmap::Assign::NotCapturable);
        // `5` has no button: giving it B would leave back with none — refused.
        assert_eq!(pad.assign(&mut t, k(KeyCode::NUM5), KEYCODE_BUTTON_B as i32), padmap::Assign::Refused { lost: k(KeyCode::RIGHT_SOFT_KEY) });
        // …but a quick slot's button may go to it, and the slot is then without one.
        assert_eq!(pad.assign(&mut t, k(KeyCode::NUM5), KEYCODE_BUTTON_R2 as i32), padmap::Assign::Done { displaced: Some(k(KeyCode::HASH)), to: None });
        assert_eq!(pad.codes_for(&t, k(KeyCode::HASH)), Vec::<i32>::new());
        // SAVE onto X: X saves, L3 takes quick slot 1 — a tab swaps like any other function.
        let r = pad.assign(&mut t, PadAction::Save, KEYCODE_BUTTON_X as i32);
        assert_eq!(r, padmap::Assign::Done { displaced: Some(k(KeyCode::NUM1)), to: Some(KEYCODE_BUTTON_THUMBL as i32) });
        // The file says only that.
        let ov = Overrides::from_table(t);
        assert_eq!(ov.to_text().lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>(), vec!["99=SAVE", "105=5", "106=1"]);
        // Every name the screen says is a host line with its Korean.
        for (_, l) in REMAP_ROWS {
            assert!(crate::host_text::korean(l).is_some(), "{l}");
        }
        for code in BUTTONS.into_iter().chain([300]) {
            assert!(crate::host_text::korean(button_name(code)).is_some(), "{code}");
        }
        for a in [None, Some(k(KeyCode::NUM5)), Some(k(KeyCode::UP)), Some(k(KeyCode::OK)), Some(PadAction::Map)] {
            assert!(crate::host_text::korean(key_label(a)).is_some(), "{a:?}");
        }
    }

    #[test]
    fn a_remap_shows_up_first_and_reaches_the_analog_triggers() {
        let mut ov = Overrides::default();
        assert_eq!(codes_for(&ov, k(KeyCode::STAR)), vec![KEYCODE_BUTTON_THUMBR]);
        assert_eq!((codes_for(&ov, PadAction::Save), codes_for(&ov, PadAction::Map)), (vec![KEYCODE_BUTTON_THUMBL], vec![KEYCODE_BUTTON_SELECT]));
        assert_eq!(codes_for(&ov, k(KeyCode::NUM7)), vec![KEYCODE_BUTTON_L1]);
        // Move 7 to R3: the badge for 7 must point at R3, not at the built-in L1 that still
        // fires it too.
        ov.set(KEYCODE_BUTTON_THUMBR, Some(k(KeyCode::NUM7)));
        assert_eq!(codes_for(&ov, k(KeyCode::NUM7)), vec![KEYCODE_BUTTON_THUMBR, KEYCODE_BUTTON_L1]);
        // Switch L2 off and give R2 the world map: the analog triggers follow.
        ov.set(KEYCODE_BUTTON_L2, None);
        ov.set(KEYCODE_BUTTON_R2, Some(PadAction::Map));
        let mut p = Pad::new(&ov);
        assert!(p.axes(0.0, 0.0, 0.0, 0.0, 0.9, 0.0).is_empty(), "L2 is off: no edge");
        assert_eq!(p.axes(0.0, 0.0, 0.0, 0.0, 0.9, 0.9), vec![(PadAction::Map, true)]);
        assert_eq!(p.release_all(), vec![(PadAction::Map, false)]);
        assert_eq!(codes_for(&ov, k(KeyCode::NUM0)), Vec::<u32>::new(), "nothing fires 0 now");
    }
}
