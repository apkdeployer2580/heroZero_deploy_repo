//! The touch HUD's state machine: fingers in, key edges out. The UI thread feeds it winit's touch
//! phases and sends what comes back to the worker; nothing here touches a window or the JVM, so
//! every rule below is tested on the host.
//!
//! The rules are the android-host skill's (input-and-hud.md, layout-editor.md) and hero2's, each
//! of which was paid for by a player report:
//!
//! * ONE ordered hit chain (`hud::hit`), with the layout editor and the settings panel above it;
//! * three touch-identity sets — the key each finger holds, the fingers host UI consumed, and the
//!   fingers that started on a control and so may SLIDE onto another (a steering thumb crossing a
//!   button must not press it);
//! * one key-edge function ([`Hud::update_touch`]) whose first test is "same key as before" (a
//!   resting finger's redelivered move is not a new press), with a keyup only when no other finger
//!   holds the key and a keydown only for the first;
//! * the stick's centre is fixed where the finger lands; it emits keyup(old) + keydown(new) on a
//!   turn — honest edges, which the worker projects onto the game's one-key model;
//! * everything held is released on suspend, focus loss and entering the editor;
//! * the editor: sticky selection, delta drag, a pinch that is absolute from its anchor and pairs
//!   the new finger with the dragging one, `CANCEL` restoring an entry snapshot, the layout
//!   persisted on every finger-up.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use hz_java::{ControlMode, GuestView, Icon, MapGrid, MapMarks, QuickBar, Scene, Stats};
use wie_backend::KeyCode;

use crate::{
    gamepad::{self, Overrides},
    hud::{self, Action, EditAction, Hit, Layout, Rect, Screen, SettingsAction},
    padhud::{self, Badge, PadHud, PadPos},
    placement::{self, CtrlId, Place, Table},
    worldmap_view as wm,
};

use crate::hostwin::{HostAct, HostKey, HostWin};

/// What the caller must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Send to the worker.
    Key { code: KeyCode, down: bool },
    /// The placement table changed: write `Hud::table().to_text()` to disk.
    SaveLayout,
    /// The HEAT SAVER toggle was tapped: persist it (`crate::config`); it applies at the next
    /// start.
    HeatSaver(bool),
    /// The PAD BADGES row was tapped: persist the new mode (`crate::config`).
    PadHud(PadHud),
    /// The STATS row was tapped: persist it (`crate::config`).
    Stats(bool),
    /// The minimap was tapped away or brought back: persist it (`crate::config`).
    Minimap(bool),
    /// The PAD REMAP panel changed the remap: write it (`gamepad::Overrides::save`) and feed
    /// the pad from it.
    Gamepad(Overrides),
    /// The SAVE tab (Phase 7-1): ask the worker for the game's own quick save.
    QuickSave,
    /// HZ-058: quick plate `n` (0-based) of the page shown was pressed — the worker's hook uses it,
    /// or puts on it what a window's 「퀵슬롯등록」 is waiting with (`hz_java::PlateAsk::Press`).
    Plate(u8),
    /// HZ-058: the plates' other page.
    PlatePage,
    /// The world map opened (`true`) or closed (Phase 7-2): the worker holds the game still while
    /// it is open.
    World(bool),
    /// A tap on the game's picture at this guest pixel, for the game's own windows
    /// (`hz_java::menu_touch`: a row gets the cursor, the cursor's row OK).
    GuestTap(i32, i32),
    /// SFX LEVEL was tapped: persist the new level (`crate::config`) and hear it from the next effect.
    SfxLevel(i32),
    /// 진단 정보 was tapped: the platform's dialog (B3, hero2's — `crate::java::show_diagnostics`),
    /// with 세이브 관리 under it (restore from the vault, export; `MainActivity` and `crate::saves`);
    /// on the PC a host window (`crate::saves_ui::diag_window`).
    OpenDiag,
    /// A host window's row acted (the PC's 진단 정보 · 세이브 관리 · 세이브 옮기기 —
    /// `crate::saves_ui::perform` carries it out and opens the next window).
    Host(HostAct),
    /// FULLSCREEN was tapped (the PC): the window borderless full screen, or windowed again.
    Fullscreen(bool),
    /// 지금 닫기: end the app (a scheduled restore or import applies at the next start).
    Quit,
}

/// The open world map (Phase 7-2): where the cursor is, where the player is, and the camera
/// gliding after the cursor (`crate::worldmap_view`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct World {
    pub cursor: usize,
    /// The player's place (`None` on a map no place has).
    pub here: Option<usize>,
    pub cam: wm::WmCam,
    target: wm::WmCam,
    /// Animation frames since the cursor last moved: the pin's hop, the stud's breathing.
    pub frame: u32,
    /// The arrow last pressed, the place it left from and when — until it is let go, a second
    /// arrow at right angles within [`WORLD_CHORD_MS`] is the diagonal from that place.
    arrow: Option<(wm::WmDir, usize, Instant)>,
    /// The pad's stick or hat, eight ways ([`pad_way`]), as last seen.
    pad: Option<wm::WmDir>,
}

/// Two arrows pressed this close together, the first still held, are one diagonal: a D-pad's
/// corner and a stick pushed between two arrows report them one after the other.
pub const WORLD_CHORD_MS: u64 = 150;

/// How long a notice ([`Hud::notice`]) stays up.
pub const TOAST_MS: u64 = 2500;

/// How many notices are kept (a memory bound — how many SHOW is what fits on the screen,
/// `hud::notice_stack`), and how long the stack takes to slide up a slot for a new one (hero2's).
pub const NOTICE_MAX: usize = 8;
pub const NOTICE_SLIDE_MS: u64 = 140;

/// What a notice in the stack is (B5): a host line (`crate::host_text`), or a picture the game made
/// for its own pickup list (HZ-025: 「귀환서」, 「N금」 — its box and its font, stacked with the host's
/// as hero2 stacks its game's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Line(&'static str),
    Picture(Icon),
}

/// A notice in the stack and when it came. A `held` one is HZ-061's countdown: it has no clock — it
/// stays until the next second replaces it in place or it is taken away ([`Hud::countdown`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Note {
    what: Notice,
    born: Instant,
    held: bool,
}

impl Note {
    fn up_at(&self, now: Instant) -> bool {
        self.held || now < self.born + Duration::from_millis(TOAST_MS)
    }
}

/// HZ-059: the id under which a finger on a D-pad corner holds the diagonal's second arrow — the key
/// edges count fingers ([`Hud::update_touch`]), so the corner is a second finger of the same touch.
fn corner_finger(id: u64) -> u64 {
    id ^ (1 << 62)
}

/// The live stick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joy {
    pub id: u64,
    pub cx: f64,
    pub cy: f64,
    pub tx: f64,
    pub ty: f64,
    /// The arrow held, or a diagonal's two (HZ-059 — the bare field's stick walks eight ways).
    pub way: Option<hud::Way>,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    id: u64,
    from: (f64, f64),
    centre0: (f64, f64),
}

#[derive(Clone, Copy, Debug)]
struct Pinch {
    a: u64,
    b: u64,
    dist0: f64,
    pct0: u32,
}

/// A line of the pad screen's window: `crate::host_text` lines side by side.
pub type Line = Vec<&'static str>;

/// The pad screen (B6 — hero2's and hero3's 「패드 버튼 설정」, `crate::padmap` for the rules): a copy
/// of the remap being edited, the function picked and since when it waits for a button, the one a
/// bare press just named, and what the window says.
#[derive(Clone, Debug)]
pub struct Remap {
    /// The table being edited: 저장 installs it, 취소 drops it.
    pub draft: Overrides,
    /// The function picked last (a `gamepad::REMAP_ROWS` index): lit while it waits.
    pub sel: Option<usize>,
    capture: Option<Instant>,
    /// What the last bare press fires (lit blue).
    pub probe: Option<usize>,
    lines: [Line; 2],
}

/// What the pad screen says when nothing is going on.
fn remap_hint() -> [Line; 2] {
    [vec!["TAP A CONTROL - THEN PRESS A PAD BUTTON"], vec!["A PAD BUTTON ALONE SHOWS WHAT IT DOES"]]
}

/// The seconds left on the prompt, as the window's second line says them.
pub const REMAP_SECONDS: [&str; 5] = ["(1S)", "(2S)", "(3S)", "(4S)", "(5S)"];

/// While the prompt waits, the screen redraws this often (the picked control breathes).
const REMAP_FRAME: Duration = Duration::from_millis(50);

/// The pad direction keycodes (`KEYCODE_DPAD_*`): never an answer (`crate::padmap::capturable`).
pub fn dpad_code(key: KeyCode) -> Option<u32> {
    match key {
        KeyCode::UP => Some(19),
        KeyCode::DOWN => Some(20),
        KeyCode::LEFT => Some(21),
        KeyCode::RIGHT => Some(22),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Editor {
    pub sel: Option<CtrlId>,
    /// PAD (A9, hero2's): each box shows the pad badge its control carries in play.
    pub pad: bool,
    snapshot: Table,
    drag: Option<Drag>,
    pinch: Option<Pinch>,
    /// Live position of every finger down. winit's Android backend reports `Moved` for EVERY
    /// pointer of a MotionEvent, so gestures are computed from these, never from deltas.
    points: HashMap<u64, (f64, f64)>,
}

pub struct Hud {
    screen: Screen,
    table: Table,
    /// What the game is showing (`hz_java::state`): the ring is for the field.
    view: GuestView,
    /// The HEAT SAVER preference as shown in the settings panel (the surface itself follows it
    /// only at the next start).
    heat_saver: bool,
    settings: bool,
    /// How far the settings list is scrolled (window px), and the finger on it (B2).
    settings_scroll: i32,
    list_drag: Option<ListDrag>,
    /// The pad screen (B6, settings › 패드 버튼 설정), while it is up.
    remap: Option<Remap>,
    editor: Option<Editor>,
    layout: Layout,
    /// finger -> the key it holds.
    keys: HashMap<u64, KeyCode>,
    /// Fingers consumed by host UI: never game input, whatever they cross.
    ui: HashSet<u64>,
    /// Fingers that started on a game control and may slide onto another, with the seat of the
    /// control they are on (`None`: one with no seat — a window's arrow).
    slide: HashMap<u64, Option<CtrlId>>,
    joy: Option<Joy>,
    /// PAD BADGES preference, whether the last input came from a pad, and whether a pad has
    /// been seen at all this run (`crate::padhud`).
    pad_hud: PadHud,
    pad_last: bool,
    pad_seen: bool,
    /// The player's gamepad remap, for the badges (`gamepad::codes_for`).
    overrides: Overrides,
    /// The host's notices, oldest first (B5: stacked as hero2's), and when the last one came — the
    /// older ones slide up a slot from then.
    notes: std::collections::VecDeque<Note>,
    pushed: Option<Instant>,
    /// A notice that stays up while the game shows its title ([`Hud::notice_through_title`]).
    title_notice: Option<&'static str>,
    /// The player's numbers (Phase 3), and whether the readout is on (`crate::config`).
    stats: Option<Stats>,
    stats_on: bool,
    /// The game's bottom bar is not drawn (HZ-013 chosen at boot): the host draws what it showed.
    bar_off: bool,
    /// The game's quick slots (Phase 6-2), as last read in the field.
    quick: Option<QuickBar>,
    /// The minimap (Phase 6-5): the map and its marks as last read, and whether the player has
    /// the panel up (`crate::config`).
    map: Option<MapGrid>,
    marks: MapMarks,
    minimap_on: bool,
    /// The head in the status block's ring (Phase 6-6), as last composed.
    portrait: Option<Icon>,
    /// The world map while it is open (Phase 7-2), and its names in the game's font (7-3,
    /// `worldmap_view::wm_label_lines` order).
    world: Option<World>,
    labels: Vec<Option<Icon>>,
    /// The host's own lines in the game's font (B1, `crate::host_text` numbers), as lettered.
    texts: HashMap<u16, Icon>,
    /// Every place's own map (7-5, `WM_PLACES` order), for the game-style minimap in the popup.
    place_maps: Vec<Option<MapGrid>>,
    /// The game's quest guidance (7-6), as last computed.
    guide: Option<hz_java::QuestGuide>,
    /// When this HUD was made: the clock the point badges blink on.
    born: Instant,
    /// SFX LEVEL as the settings row shows it (`audio_mix::SFX_STEPS_DB`).
    sfx_db: i32,
    /// A notice that never goes away (the game stopped: nothing else will be shown again).
    stuck: Option<&'static str>,
    /// 조작 방식 (`hz_java::options`): 고정 puts the stick's centre on its seat, not where the thumb
    /// lands; 십자키 walks the field with the arrows ([`hud::Walk`]).
    control: ControlMode,
    /// A finger on the minimap or the status block: a TAP there acts when it lifts, so a thumb
    /// crossing either on its way somewhere does not hide the map or open the menu.
    tap: Option<Tap>,
    /// While a finger holds OK in the field: when its next repeat is due ([`FIRE_REPEAT_DELAY`]).
    fire_next: Option<Instant>,
    /// Fingers on the world map's arrows, and the arrow each holds (A3).
    world_arms: HashMap<u64, KeyCode>,
    /// A host window (the PC's dialogs, `crate::hostwin`) while one is up, and the finger on its
    /// list (a press may be the start of a scroll; the row acts when the finger lifts).
    host: Option<HostWin>,
    host_drag: Option<ListDrag>,
    /// The next id for a host window's run-time lines (`HostWin::number`): from [`DYN_FIRST`],
    /// past every `crate::host_text` line.
    dyn_next: u16,
    /// The PC's window is full screen (the FULLSCREEN row's state; the app says).
    fullscreen: bool,
}

/// The first id a host window's run-time lines are lettered under — far past `crate::host_text`'s
/// table (its lines are numbered from 0), so a dynamic line never replaces a fixed one.
pub const DYN_FIRST: u16 = 4096;

impl Hud {
    pub fn host(&self) -> Option<&HostWin> {
        self.host.as_ref()
    }

    pub fn host_open(&self) -> bool {
        self.host.is_some()
    }

    /// A lettered line by its number (a host window's run-time line, `crate::hostwin::Text::Dyn`).
    pub fn text_id(&self, id: u16) -> Option<&Icon> {
        self.texts.get(&id)
    }

    /// Put `win` up — over the settings window, which closes, in place of a host window already up,
    /// whose lines are forgotten. Numbers its run-time lines; what comes back is for the worker to
    /// letter (`hz_java::GuestState::ask_texts`), and arrives as `UserEvent::Texts`.
    pub fn open_host(&mut self, mut win: HostWin) -> Vec<(u16, Vec<u8>)> {
        self.close_host();
        self.close_settings();
        let (asks, next) = win.number(self.dyn_next);
        self.dyn_next = if next < DYN_FIRST { DYN_FIRST } else { next };
        win.scroll = 0;
        self.host = Some(win);
        self.host_drag = None;
        asks
    }

    pub fn close_host(&mut self) {
        if let Some(w) = self.host.take() {
            for id in w.dyn_ids() {
                self.texts.remove(&id);
            }
        }
        self.host_drag = None;
    }

    /// The plate a finger rests on, not yet moved (drawn as the cursor's row while it is held).
    pub fn host_pressed(&self) -> Option<usize> {
        let d = self.host_drag.filter(|d| !d.moved)?;
        let w = self.host.as_ref()?;
        match hud::host_hit(&self.screen, w.rows.len(), w.scroll, d.from.0, d.from.1) {
            hud::HostHit::Row(i) if w.rows[i].act.is_some() => Some(i),
            _ => None,
        }
    }

    fn host_started(&mut self, id: u64, x: f64, y: f64) {
        let Some(w) = self.host.as_ref() else { return };
        match hud::host_hit(&self.screen, w.rows.len(), w.scroll, x, y) {
            hud::HostHit::Close => self.close_host(),
            hud::HostHit::Row(_) => {
                if self.host_drag.is_none() {
                    self.host_drag = Some(ListDrag { id, from: (x, y), scroll0: w.scroll, moved: false });
                }
            }
            hud::HostHit::Panel => {}
        }
    }

    fn host_moved(&mut self, x: f64, y: f64) {
        let Some(w) = self.host.as_mut() else { return };
        let max = hud::host_scroll_max(&self.screen, w.rows.len());
        if let Some(d) = self.host_drag.as_mut()
            && (d.moved || (x - d.from.0).abs() > TAP_SLOP || (y - d.from.1).abs() > TAP_SLOP)
        {
            d.moved = true;
            w.scroll = (f64::from(d.scroll0) - (y - d.from.1)).clamp(0.0, f64::from(max)) as i32;
        }
    }

    /// The finger on the list lifted (`act`) or was taken back: a drag settles on a whole row, a tap
    /// on a plate puts the cursor there and acts.
    fn host_lifted(&mut self, act: bool, out: &mut Vec<Out>) {
        let Some(d) = self.host_drag.take() else { return };
        let Some(w) = self.host.as_mut() else { return };
        let n = w.rows.len();
        if d.moved {
            let pitch = hud::host_geom(&self.screen).pitch.max(1);
            let max = hud::host_scroll_max(&self.screen, n);
            w.scroll = ((w.scroll + pitch / 2) / pitch * pitch).min(max);
            return;
        }
        if act
            && let hud::HostHit::Row(i) = hud::host_hit(&self.screen, n, w.scroll, d.from.0, d.from.1)
            && w.rows[i].act.is_some()
        {
            w.cursor = Some(i);
            self.host_act(out);
        }
    }

    /// The cursor's row acts: 닫기 here, the rest is the app's (`Out::Host`).
    fn host_act(&mut self, out: &mut Vec<Out>) {
        match self.host.as_ref().and_then(HostWin::act) {
            Some(HostAct::Close) => self.close_host(),
            Some(a) => out.push(Out::Host(a)),
            None => {}
        }
    }

    /// A key while a host window is up (the PC's keyboard, a pad): the cursor up and down — past the
    /// last plate the list scrolls a row — OK acts, the back keys close; every other key is
    /// swallowed. `None` = no window, the key is the game's.
    pub fn host_key(&mut self, code: KeyCode) -> Option<Vec<Out>> {
        let mut out = Vec::new();
        let key = crate::hostwin::host_key(code);
        let w = self.host.as_mut()?;
        match key {
            Some(k @ (HostKey::Up | HostKey::Down)) => {
                let down = k == HostKey::Down;
                let n = w.rows.len();
                if w.step(down) {
                    if let Some(i) = w.cursor {
                        w.scroll = hud::host_scroll_to(&self.screen, n, w.scroll, i);
                    }
                } else {
                    let pitch = hud::host_geom(&self.screen).pitch;
                    let max = hud::host_scroll_max(&self.screen, n);
                    w.scroll = (w.scroll + if down { pitch } else { -pitch }).clamp(0, max);
                }
            }
            Some(HostKey::Ok) => self.host_act(&mut out),
            Some(HostKey::Back) => self.close_host(),
            None => {}
        }
        Some(out)
    }

    /// A key while the settings window is up (the PC): the back keys close it, the rest is
    /// swallowed (the rows are the mouse's). `None` = not open.
    pub fn settings_key(&mut self, code: KeyCode) -> Option<Vec<Out>> {
        if !self.settings {
            return None;
        }
        if matches!(code, KeyCode::RIGHT_SOFT_KEY | KeyCode::CLEAR) {
            self.close_settings();
        }
        Some(Vec::new())
    }

    /// The mouse wheel (the PC): a host window's list or the settings', `rows` rows down (negative
    /// = up).
    pub fn wheel(&mut self, rows: i32) {
        if let Some(w) = self.host.as_mut() {
            let pitch = hud::host_geom(&self.screen).pitch;
            let max = hud::host_scroll_max(&self.screen, w.rows.len());
            w.scroll = (w.scroll + rows * pitch).clamp(0, max);
        } else if self.settings {
            let pitch = hud::settings_geom(&self.screen).pitch;
            let max = hud::settings_scroll_max(&self.screen, self.settings_list().len());
            self.settings_scroll = (self.settings_scroll + rows * pitch).clamp(0, max);
        }
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }
}

/// Holding OK in the field attacks again and again (hero3 132차): the game attacks once per press
/// (`ce.a(II)Z` @420..509: FIRE → `ed.b(0)`, nothing per frame for a held key), so the host lets
/// go and presses again — after this long held…
pub const FIRE_REPEAT_DELAY: Duration = Duration::from_millis(350);
/// …and then every this long. A press during an attack is the game's to queue (`ce.n:I`).
pub const FIRE_REPEAT_EVERY: Duration = Duration::from_millis(250);

/// The unspent-point badges blink on the HOST clock (hero2's `BADGE_BLINK_MS`): the game's own
/// blink counter lives in the bar painter HZ-013 skips, and a frame count would blink at a
/// device-dependent rate.
pub const BADGE_BLINK_MS: u128 = 250;

/// How far a finger may wander and still be a tap (hero2's `TAP_SLOP`, screen px): generous —
/// fingers wobble — and far under the size of the targets it guards.
pub const TAP_SLOP: f64 = 28.0;

/// What a pending tap is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapOn {
    /// The minimap panel (hides it) or its restore button (brings it back).
    Minimap,
    /// The status block: the menu (hero2's `stats_tap`).
    Status,
}

/// A finger that came down on a tap target ([`TapOn`]): where it came down and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    id: u64,
    on: TapOn,
    from: (f64, f64),
    at: (f64, f64),
}

/// A finger on the settings list (hero2's `settings_drag`): where it came down and the scroll then.
/// Until it wanders past [`TAP_SLOP`] it is a tap, decided when it lifts; after, the list follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ListDrag {
    id: u64,
    from: (f64, f64),
    scroll0: i32,
    moved: bool,
}

/// The arrow a key is on the world map: the D-pad's, and the keypad's 2·4·6·8.
fn world_arrow(code: KeyCode) -> Option<wm::WmDir> {
    match code {
        KeyCode::UP | KeyCode::NUM2 => Some(wm::WmDir::Up),
        KeyCode::DOWN | KeyCode::NUM8 => Some(wm::WmDir::Down),
        KeyCode::LEFT | KeyCode::NUM4 => Some(wm::WmDir::Left),
        KeyCode::RIGHT | KeyCode::NUM6 => Some(wm::WmDir::Right),
        _ => None,
    }
}

/// The pad's stick or hat on the world map (−1..1, y down), eight ways by 45° sectors, nothing
/// inside half a throw. A HID hat reports a D-pad corner as ONE sample with both axes, which the
/// game's one-direction latch ([`crate::gamepad::Pad::axes`]) turns into one arrow — so the map
/// reads the axes itself (S9 2026-10-24, a virtual USB pad: ↗ from 실바나르 남부 went to 휼렌).
pub fn pad_way(dx: f64, dy: f64) -> Option<wm::WmDir> {
    use wm::WmDir::*;
    const WAYS: [wm::WmDir; 8] = [Right, UpRight, Up, UpLeft, Left, DownLeft, Down, DownRight];
    if dx.abs().max(dy.abs()) < 0.5 {
        return None;
    }
    let k = ((-dy).atan2(dx) / std::f64::consts::FRAC_PI_4).round() as i64;
    Some(WAYS[k.rem_euclid(8) as usize])
}

/// The key a way is on the map: an arrow, or the keypad's 1·3·7·9 for a diagonal.
fn way_key(d: wm::WmDir) -> KeyCode {
    match d {
        wm::WmDir::Up => KeyCode::UP,
        wm::WmDir::Down => KeyCode::DOWN,
        wm::WmDir::Left => KeyCode::LEFT,
        wm::WmDir::Right => KeyCode::RIGHT,
        wm::WmDir::UpLeft => KeyCode::NUM1,
        wm::WmDir::UpRight => KeyCode::NUM3,
        wm::WmDir::DownLeft => KeyCode::NUM7,
        wm::WmDir::DownRight => KeyCode::NUM9,
    }
}

/// A diagonal's two arrows.
fn way_arrows(d: wm::WmDir) -> Option<[wm::WmDir; 2]> {
    use wm::WmDir::*;
    match d {
        UpLeft => Some([Up, Left]),
        UpRight => Some([Up, Right]),
        DownLeft => Some([Down, Left]),
        DownRight => Some([Down, Right]),
        _ => None,
    }
}

fn distance(p: (f64, f64), q: (f64, f64)) -> f64 {
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
}

impl Hud {
    pub fn new(screen: Screen, table: Table) -> Hud {
        let view = GuestView::default();
        let layout = hud::layout(screen, &table, view);
        Hud {
            screen,
            table,
            view,
            heat_saver: false,
            settings: false,
            settings_scroll: 0,
            list_drag: None,
            remap: None,
            editor: None,
            layout,
            keys: HashMap::new(),
            ui: HashSet::new(),
            slide: HashMap::new(),
            joy: None,
            pad_hud: PadHud::Auto,
            pad_last: false,
            pad_seen: false,
            overrides: Overrides::default(),
            notes: Default::default(),
            pushed: None,
            title_notice: None,
            stats: None,
            stats_on: true,
            bar_off: false,
            quick: None,
            map: None,
            marks: MapMarks::default(),
            minimap_on: true,
            portrait: None,
            world: None,
            labels: Vec::new(),
            texts: HashMap::new(),
            place_maps: Vec::new(),
            guide: None,
            born: Instant::now(),
            sfx_db: 0,
            stuck: None,
            control: ControlMode::Free,
            tap: None,
            fire_next: None,
            world_arms: HashMap::new(),
            host: None,
            host_drag: None,
            dyn_next: DYN_FIRST,
            fullscreen: false,
        }
    }

    pub fn set_guide(&mut self, g: hz_java::QuestGuide) {
        self.guide = Some(g);
    }

    /// The place the quest under way leads to (7-6): where its guide tiles end.
    pub fn quest_place(&self) -> Option<usize> {
        self.guide.as_ref().and_then(|g| g.target).and_then(wm::wm_place_index)
    }

    /// The game's guide tile (its yellow 「!」) in map `map`, if the quest under way has one there.
    pub fn guide_tile(&self, map: u8) -> Option<(u8, u8)> {
        self.guide.as_ref()?.tiles.iter().find(|t| t.0 == map).map(|&(_, x, y)| (x, y))
    }

    pub fn set_place_maps(&mut self, maps: Vec<Option<MapGrid>>) {
        self.place_maps = maps;
    }

    /// What the world map's minimap shows for the cursor's place (7-5, the fusion with the game's
    /// own minimap window): on the player's place the map the player is in and where they stand,
    /// elsewhere the place's own map.
    pub fn world_minimap(&self) -> Option<(&MapGrid, Option<(f32, f32)>)> {
        let w = self.world.as_ref()?;
        if Some(w.cursor) == w.here
            && let Some(m) = self.map.as_ref()
        {
            return Some((m, self.marks.player));
        }
        self.place_maps.get(w.cursor).and_then(Option::as_ref).map(|m| (m, None))
    }

    pub fn set_labels(&mut self, labels: Vec<Option<Icon>>) {
        self.labels = labels;
    }

    /// Host lines lettered in the game's font (B1); a line that could not be keeps its ASCII.
    pub fn set_texts(&mut self, texts: Vec<(u16, Option<Icon>)>) {
        for (id, icon) in texts {
            match icon {
                Some(i) => self.texts.insert(id, i),
                None => self.texts.remove(&id),
            };
        }
    }

    /// The game-font picture of the host line `en` (its Korean, `crate::host_text`), once lettered.
    pub fn text(&self, en: &str) -> Option<&Icon> {
        crate::host_text::index(en).and_then(|i| self.texts.get(&i))
    }

    /// The world map's name `i` (a place, or the title after the last place), once lettered.
    pub fn label(&self, i: usize) -> Option<&Icon> {
        self.labels.get(i).and_then(Option::as_ref)
    }

    // ------------------------------------------------------------------ the world map ----

    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }

    /// The popup's frame: below the hanging tabs, so the MAP tab that opened it closes it again.
    pub fn world_panel(&self) -> wm::WmPanel {
        let top = hud::save_tab(&self.screen, self.tab_map_w()).bottom() + self.screen.pad();
        wm::wm_panel(self.screen.w.max(0) as u32, self.screen.h.max(0) as u32, top.max(0) as u32)
    }

    fn camera_on(&self, place: usize) -> wm::WmCam {
        let (iw, ih) = wm::wm_picture_size();
        let p = self.world_panel();
        wm::wm_open_at(place, iw, ih, p.view.w, p.view.h)
    }

    /// The MAP tab: the cursor on the player's place, the camera already there. Every finger and
    /// key the game holds is let go first — the game stands still until the map closes.
    fn open_world(&mut self, out: &mut Vec<Out>) {
        out.extend(self.release_all());
        let here = self.map.as_ref().and_then(|m| wm::wm_place_index(m.id));
        let cursor = here.unwrap_or(wm::WM_SILVA);
        let cam = self.camera_on(cursor);
        self.world = Some(World { cursor, here, cam, target: cam, frame: 0, arrow: None, pad: None });
        out.push(Out::World(true));
    }

    pub fn close_world(&mut self) -> Vec<Out> {
        if self.world.take().is_some() { vec![Out::World(false)] } else { Vec::new() }
    }

    fn world_cursor(&mut self, to: usize) {
        let target = self.camera_on(to);
        if let Some(w) = self.world.as_mut()
            && w.cursor != to
        {
            w.cursor = to;
            w.target = target;
            w.frame = 0;
        }
    }

    /// A key while the map is open (keyboard, pad): the arrows walk the cursor along the roads —
    /// 1·3·7·9 and two arrows pressed together along the diagonals — the soft keys and CLR close
    /// it. Nothing reaches the game.
    pub fn world_key(&mut self, code: KeyCode) -> Vec<Out> {
        self.world_key_at(code, Instant::now())
    }

    pub fn world_key_at(&mut self, code: KeyCode, now: Instant) -> Vec<Out> {
        let Some(w) = self.world else { return Vec::new() };
        let dir = match code {
            KeyCode::LEFT_SOFT_KEY | KeyCode::RIGHT_SOFT_KEY | KeyCode::CLEAR => return self.close_world(),
            KeyCode::NUM1 => wm::WmDir::UpLeft,
            KeyCode::NUM3 => wm::WmDir::UpRight,
            KeyCode::NUM7 => wm::WmDir::DownLeft,
            KeyCode::NUM9 => wm::WmDir::DownRight,
            c => match world_arrow(c) {
                Some(d) => d,
                None => return Vec::new(),
            },
        };
        // The second arrow of a pair: the diagonal from where the first one left, if a road runs
        // that way; else it is an arrow of its own.
        let chord = w
            .arrow
            .filter(|&(_, _, at)| now.saturating_duration_since(at) < Duration::from_millis(WORLD_CHORD_MS))
            .and_then(|(held, from, _)| held.with(dir).and_then(|diag| wm::wm_step(from, diag)));
        let arrow = world_arrow(code).filter(|_| chord.is_none()).map(|d| (d, w.cursor, now));
        if let Some(to) = chord.or_else(|| wm::wm_step(w.cursor, dir)) {
            self.world_cursor(to);
        }
        if let Some(w) = self.world.as_mut() {
            w.arrow = arrow;
        }
        Vec::new()
    }

    /// A key let go while the map is open: an arrow no longer held pairs with nothing.
    pub fn world_key_up(&mut self, code: KeyCode) {
        if let Some(w) = self.world.as_mut()
            && w.arrow.is_some_and(|(d, _, _)| world_arrow(code) == Some(d))
        {
            w.arrow = None;
        }
    }

    /// The pad's stick and hat while the map is open (the summed axes, −1..1, y down): a step when
    /// the way changes ([`pad_way`]). A D-pad corner is the diagonal at once; a stick rolled from an
    /// arrow onto its diagonal is that arrow's pair (the chord: the diagonal from where the arrow
    /// left, when it was fresh); a corner let go through one of its own arrows does not step.
    /// The analog triggers' edges (`gamepad::Pad::triggers`) are pad keys to it, as a button's
    /// are ([`Hud::world_key`]: L2 put on back closes the map; on a tab, [`Hud::pad_tab`]); a
    /// release is nothing to it, a trigger is never an arrow.
    pub fn world_pad(&mut self, dx: f64, dy: f64, triggers: &[(gamepad::PadAction, bool)]) -> Vec<Out> {
        let now = Instant::now();
        let mut out = self.world_pad_at(dx, dy, now);
        for &(action, _) in triggers.iter().filter(|(_, down)| *down) {
            out.extend(match action {
                gamepad::PadAction::Key(code) => self.world_key_at(code, now),
                gamepad::PadAction::Save => self.pad_tab(hud::Tab::Save),
                gamepad::PadAction::Map => self.pad_tab(hud::Tab::Map),
            });
        }
        out
    }

    /// A pad button given a top tab (hero2's 빠른저장 on L3; here SAVE and MAP, user 2026-10-27):
    /// what tapping the tab does, wherever the tab is drawn. SAVE asks wherever no host window is up
    /// — the worker answers 「SAVE - NOT NOW」 off the bare field, which is the button's feedback, as
    /// hero2's is — and over the world map closes it first, as its tab does there; MAP opens the
    /// map where its tab is and closes it again.
    pub fn pad_tab(&mut self, tab: hud::Tab) -> Vec<Out> {
        let mut out = Vec::new();
        match tab {
            hud::Tab::Map if self.world.is_some() => out.extend(self.close_world()),
            hud::Tab::Map if self.tab(hud::Tab::Map).is_some() => self.open_world(&mut out),
            hud::Tab::Map => {}
            hud::Tab::Save if self.world.is_some() => {
                out.extend(self.close_world());
                out.push(Out::QuickSave);
            }
            hud::Tab::Save if self.settings || self.remap.is_some() || self.editor.is_some() => {}
            hud::Tab::Save => out.push(Out::QuickSave),
        }
        out
    }

    pub fn world_pad_at(&mut self, dx: f64, dy: f64, now: Instant) -> Vec<Out> {
        let Some(w) = self.world else { return Vec::new() };
        let way = pad_way(dx, dy);
        if way == w.pad {
            return Vec::new();
        }
        if let Some(w) = self.world.as_mut() {
            w.pad = way;
        }
        let Some(d) = way else {
            if let Some(prev) = w.pad {
                self.world_key_up(way_key(prev));
            }
            return Vec::new();
        };
        if w.pad.and_then(way_arrows).is_some_and(|arrows| arrows.contains(&d)) {
            if let Some(w) = self.world.as_mut() {
                w.arrow = None;
            }
            return Vec::new();
        }
        match (w.pad, way_arrows(d)) {
            (Some(prev), Some(arrows)) if arrows.contains(&prev) => {
                let fresh = w
                    .arrow
                    .is_some_and(|(held, _, at)| held == prev && now.saturating_duration_since(at) < Duration::from_millis(WORLD_CHORD_MS));
                let other = if arrows[0] == prev { arrows[1] } else { arrows[0] };
                self.world_key_at(way_key(if fresh { other } else { d }), now)
            }
            _ => self.world_key_at(way_key(d), now),
        }
    }

    /// One animation frame (~30 a second while the map is open): the camera's glide, the pin's
    /// hop, the stud's breathing.
    pub fn world_tick(&mut self) {
        if let Some(w) = self.world.as_mut() {
            wm::wm_glide(&mut w.cam, w.target);
            w.frame = w.frame.wrapping_add(1);
        }
    }

    /// The world map's arrows (A3, hero2's `draw_menu_dpad` over the map): a window's four, on the
    /// walk control's seat.
    pub fn world_arrows(&self) -> Vec<hud::Control> {
        // The PC (2026-10-29): the keyboard's arrows walk the roads; nothing to draw or press.
        if self.world.is_none() || self.screen.desktop {
            return Vec::new();
        }
        placement::resolve_with(&self.screen, &self.table)
            .into_iter()
            .find(|(id, _)| *id == CtrlId::Stick)
            .map_or_else(Vec::new, |(_, seat)| hud::dpad_controls(seat))
    }

    /// The arrows the fingers hold on the world map (for the pressed look).
    pub fn world_held(&self) -> HashSet<KeyCode> {
        self.world_arms.values().copied().collect()
    }

    /// A finger down while the map is open: the back button, outside the frame or the MAP tab close
    /// it; the SAVE tab closes it and asks for the save; an arrow steps the cursor along a road; a
    /// place moves the cursor there.
    fn world_started(&mut self, id: u64, x: f64, y: f64, out: &mut Vec<Out>) {
        let Some(w) = self.world else { return };
        if let Some((_, tab)) = self.layout.tabs.iter().find(|(r, _)| hud::top_tab_hit(&self.screen, *r).contains(x, y)).copied() {
            out.extend(self.close_world());
            if tab == hud::Tab::Save {
                out.push(Out::QuickSave);
            }
            return;
        }
        let arrows = self.world_arrows();
        if let Some(Action::Key(k)) = arrows.iter().find(|c| c.shape.contains(x, y, 0.0)).map(|c| c.action) {
            self.world_arms.insert(id, k);
            out.extend(self.world_key(k));
            return;
        }
        if arrows.first().is_some_and(|c| c.shape.bounds().contains(x, y)) {
            return; // between the arrows
        }
        let p = self.world_panel();
        let (xi, yi) = (x as i32, y as i32);
        if p.back.contains(xi, yi) || !p.frame.contains(xi, yi) {
            out.extend(self.close_world());
            return;
        }
        // The game-style minimap (7-5) is a picture, not a place.
        let inset = self.world_minimap().and_then(|(m, _)| wm::wm_inset(p.view, m.w, m.h, p.unit)).map(|(panel, ..)| panel);
        if inset.is_some_and(|panel| panel.contains(xi, yi)) {
            return;
        }
        let (iw, ih) = wm::wm_picture_size();
        let label_w = |i: usize| self.label(i).map_or(0, |l| l.w * p.unit);
        if let Some(i) = wm::wm_hit_place(w.cam, iw, ih, p.view, p.unit, label_w, inset, xi, yi) {
            self.world_cursor(i);
        }
    }

    // ---------------------------------------------------------------- what is shown ----

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn settings_open(&self) -> bool {
        self.settings
    }

    /// The settings window's rows here and now, in order (`(action, can act)`): hero2's 화면 배치 ·
    /// 진단 정보 (세이브 관리 under it) · 발열 절감 · 효과음 크기 — its 조작 방식 is the game's own
    /// 「추가 설정」 row here — then 상태 표시 while the game's bar is drawn, and the pad's two rows
    /// once a pad has been used or while either is off its default (else a player who changed one
    /// could not change it back without picking the pad up again).
    pub fn settings_list(&self) -> Vec<(SettingsAction, bool)> {
        // The PC (2026-10-29): no touch layout to edit, no heat to save, no badges to show on
        // controls it does not draw — full screen, 진단 정보 (세이브 관리 under it), the effects' level,
        // and the status block while the game's bar is drawn.
        if self.screen.desktop {
            let mut rows = vec![(SettingsAction::Fullscreen, true), (SettingsAction::Diag, true), (SettingsAction::SfxLevel, true)];
            if !self.bar_off {
                rows.push((SettingsAction::Stats, true));
            }
            return rows;
        }
        let mut rows = vec![
            (SettingsAction::LayoutEdit, true),
            (SettingsAction::Diag, true),
            (SettingsAction::HeatSaver, true),
            (SettingsAction::SfxLevel, true),
        ];
        if !self.bar_off {
            rows.push((SettingsAction::Stats, true));
        }
        if self.pad_seen || self.pad_hud != PadHud::Auto || self.overrides != Overrides::default() {
            rows.push((SettingsAction::PadHud, true));
            rows.push((SettingsAction::PadRemap, true));
        }
        rows
    }

    /// The plates the settings list shows now ([`hud::settings_rows`]).
    pub fn settings_rows(&self) -> Vec<(Rect, SettingsAction, bool)> {
        hud::settings_rows(&self.screen, &self.settings_list(), self.settings_scroll)
    }

    pub fn settings_scroll(&self) -> i32 {
        self.settings_scroll
    }

    /// A settings row's two words — `crate::host_text` lines — and how its state reads (hero2's
    /// row texts, `overlay.rs` `draw_settings`).
    pub fn settings_words(&self, a: SettingsAction, can: bool) -> (&'static str, &'static str, hud::RowState) {
        use hud::RowState::{Active, Disabled, Normal};
        let lit = |on: bool| if on { Active } else { Normal };
        let state = match a {
            SettingsAction::LayoutEdit if !self.table.is_empty() => ("CUSTOM", Active),
            SettingsAction::LayoutEdit => ("EDIT", Normal),
            SettingsAction::Diag => ("SAVES", Normal),
            SettingsAction::HeatSaver => (if self.heat_saver { "ON" } else { "OFF" }, lit(self.heat_saver)),
            SettingsAction::SfxLevel => (crate::audio_mix::sfx_db_word(self.sfx_db), lit(self.sfx_db != 0)),
            SettingsAction::Stats => (if self.stats_on { "ON" } else { "OFF" }, lit(!self.stats_on)),
            SettingsAction::PadHud => match self.pad_hud {
                PadHud::Auto => ("AUTO", Normal),
                PadHud::On => ("ALWAYS", Active),
                PadHud::Off => ("OFF", Active),
            },
            SettingsAction::PadRemap if self.overrides != Overrides::default() => ("CUSTOM", Active),
            SettingsAction::PadRemap => ("DEFAULT", Normal),
            SettingsAction::Fullscreen => (if self.fullscreen { "ON" } else { "OFF" }, lit(self.fullscreen)),
            SettingsAction::Close => ("", Normal),
        };
        // A row that cannot act here reads so, whatever it says.
        (a.name(), state.0, if can { state.1 } else { Disabled })
    }

    /// The row a finger rests on, not yet moved (drawn as the game's cursor row while it is held).
    pub fn settings_pressed(&self) -> Option<SettingsAction> {
        let d = self.list_drag.filter(|d| !d.moved)?;
        hud::settings_hit(&self.screen, &self.settings_list(), self.settings_scroll, d.from.0, d.from.1)
    }

    pub fn remap_open(&self) -> bool {
        self.remap.is_some()
    }

    pub fn remap(&self) -> Option<&Remap> {
        self.remap.as_ref()
    }

    /// The pad screen's controls, targets and window for this screen and this layout.
    pub fn remap_scene(&self) -> hud::RemapScene {
        hud::remap_scene(&self.screen, &self.table)
    }

    /// Open the pad screen over a copy of the remap. Everything held is let go first — while it is
    /// up every pad button is an answer, not a game key (the app routes them to [`Hud::pad_button`]).
    fn open_remap(&mut self, out: &mut Vec<Out>) {
        out.extend(self.release_all());
        self.close_settings();
        self.remap = Some(Remap { draft: self.overrides.clone(), sel: None, capture: None, probe: None, lines: remap_hint() });
    }

    /// Take the screen down and put the settings window back — where the player came from, and
    /// whose 패드 버튼 설정 row then says whether anything is changed.
    fn close_remap(&mut self) {
        self.remap = None;
        self.settings = true;
        self.settings_scroll = 0;
    }

    /// What the window says at `now`: while a picked control waits for its button, the seconds left
    /// in the second line.
    pub fn remap_lines_at(&self, now: Instant) -> [Line; 2] {
        let Some(r) = self.remap.as_ref() else { return [Vec::new(), Vec::new()] };
        let left = r.capture.and_then(|at| crate::padmap::seconds_left(now.saturating_duration_since(at).as_millis()));
        match left {
            Some(s) => [r.lines[0].clone(), vec!["PRESS THE PAD BUTTON FOR IT", REMAP_SECONDS[(s as usize).clamp(1, 5) - 1]]],
            None => r.lines.clone(),
        }
    }

    /// The picked control's breath while it waits (0..=1, one every 1.1 s from the pick).
    pub fn remap_pulse_at(&self, now: Instant) -> Option<f64> {
        let at = self.remap.as_ref()?.capture?;
        let t = now.saturating_duration_since(at);
        crate::padmap::seconds_left(t.as_millis())?;
        Some(0.5 - 0.5 * (t.as_secs_f64() * std::f64::consts::TAU / 1.1).cos())
    }

    /// Per target of [`Hud::remap_scene`]: the position of the button that fires its function in
    /// the DRAFT (`None`: no button, or one with no known position).
    pub fn remap_badges(&self, sc: &hud::RemapScene) -> Vec<Option<PadPos>> {
        let Some(r) = self.remap.as_ref() else { return Vec::new() };
        sc.targets
            .iter()
            .map(|t| gamepad::codes_for(&r.draft, gamepad::REMAP_ROWS[t.row].0).first().and_then(|c| padhud::pos_for_code(*c)))
            .collect()
    }

    /// Has 저장 anything to keep (the draft differs from the remap in force — lit gold), and has
    /// 기본값 anything to undo (it differs from the built-in layout)?
    pub fn remap_dirty(&self) -> bool {
        self.remap.as_ref().is_some_and(|r| r.draft != self.overrides)
    }

    pub fn remap_can_reset(&self) -> bool {
        self.remap.as_ref().is_some_and(|r| !r.draft.is_empty())
    }

    /// A cell's caption: the function's name — and why it has no control on screen when that was
    /// the player's own choice (a quick plate switched off in the layout editor).
    pub fn remap_caption(&self, row: usize) -> Line {
        let (_, name) = gamepad::REMAP_ROWS[row];
        vec![name]
    }

    /// When the pad screen next changes on its own: while the prompt waits, the next breath.
    pub fn remap_until(&self) -> Option<Instant> {
        let r = self.remap.as_ref()?;
        r.capture.map(|_| Instant::now() + REMAP_FRAME)
    }

    /// The prompt ran out at `now`: nothing is picked any more, and the window says so.
    pub fn remap_tick(&mut self, now: Instant) {
        if let Some(r) = self.remap.as_mut()
            && let Some(at) = r.capture
            && crate::padmap::seconds_left(now.saturating_duration_since(at).as_millis()).is_none()
        {
            r.capture = None;
            r.sel = None;
            r.lines = [vec!["TIME IS UP"], vec!["TAP THE CONTROL AGAIN"]];
        }
    }

    /// A finger down on the pad screen (hero2's `padmap_started`, answered on the way down —
    /// nothing here scrolls): a lit control or a cell picks its function and starts the prompt;
    /// 취소 · 기본값 · 저장 do that; the picture lets go of whatever was picked.
    fn remap_started(&mut self, x: f64, y: f64, out: &mut Vec<Out>) {
        let hit = hud::remap_hit(&self.remap_scene(), x, y);
        match hit {
            Some(hud::RemapHit::Cancel) => return self.close_remap(),
            Some(hud::RemapHit::Save) => {
                if let Some(r) = self.remap.take() {
                    self.overrides = r.draft;
                    out.push(Out::Gamepad(self.overrides.clone()));
                }
                return self.close_remap();
            }
            _ => {}
        }
        let Some(r) = self.remap.as_mut() else { return };
        match hit {
            Some(hud::RemapHit::Reset) => {
                r.draft = Overrides::default();
                (r.sel, r.capture, r.probe) = (None, None, None);
                r.lines = [vec!["BACK TO THE BUILT-IN LAYOUT"], vec!["SAVE TO APPLY"]];
            }
            Some(hud::RemapHit::Row(row)) => {
                let (key, name) = gamepad::REMAP_ROWS[row];
                let now = gamepad::codes_for(&r.draft, key).first().map_or("NO BUTTON", |c| gamepad::button_name(*c));
                (r.sel, r.capture, r.probe) = (Some(row), Some(Instant::now()), None);
                r.lines = [vec![name, "=", now], Vec::new()];
            }
            // The picture: "never mind".
            None if r.sel.is_some() || r.probe.is_some() => {
                (r.sel, r.capture, r.probe) = (None, None, None);
                r.lines = remap_hint();
            }
            _ => {}
        }
    }

    /// SFX LEVEL as the settings row shows it (seeded from `crate::config` at start).
    pub fn set_sfx_db(&mut self, db: i32) {
        self.sfx_db = db;
    }

    pub fn sfx_db(&self) -> i32 {
        self.sfx_db
    }

    /// 조작 방식 (the 「추가 설정」 row): 자유, 십자키 (the field's arrows) or 고정 (the stick's
    /// centre its seat).
    pub fn set_control(&mut self, mode: ControlMode) {
        if mode != self.control {
            self.control = mode;
            self.relayout();
        }
    }

    pub fn control(&self) -> ControlMode {
        self.control
    }

    /// The left seat's walk control ([`hud::Walk`]): nothing while the pad drives (the badges are
    /// up), else the field's per 조작 방식.
    fn walk(&self) -> hud::Walk {
        if self.pad_hud.active(self.pad_last) {
            hud::Walk::Pad
        } else if self.control == ControlMode::Dpad {
            hud::Walk::Arrows
        } else {
            hud::Walk::Stick
        }
    }

    /// The game stopped: say so for good (every other notice gives way to it).
    pub fn notice_stuck(&mut self, text: &'static str) {
        self.stuck = Some(text);
    }

    /// A pad button pressed while the pad screen is up — the app routes every press here instead of
    /// to the game, a direction as its `KEYCODE_DPAD_*` ([`dpad_code`]) and an analog trigger as
    /// its L2 / R2 (hero2's `padmap_button`). With a control picked and the prompt up it becomes
    /// that control's button (a conflict swaps, `crate::padmap::Pad::assign`); with nothing picked
    /// the window says what the button does and its control is lit. Nothing is written until 저장.
    pub fn pad_button(&mut self, code: u32) {
        self.pad_button_at(code, Instant::now());
    }

    pub fn pad_button_at(&mut self, code: u32, now: Instant) {
        let Some(r) = self.remap.as_mut() else { return };
        let icode = i32::try_from(code).unwrap_or(0);
        let name = gamepad::button_name(code);
        let waiting = r.capture.is_some_and(|at| crate::padmap::seconds_left(now.saturating_duration_since(at).as_millis()).is_some());
        let Some(row) = r.sel.filter(|_| waiting) else {
            if r.capture.take().is_some() {
                // Late: the redraw that expires it may not have run yet.
                r.sel = None;
                r.lines = [vec!["TIME IS UP"], vec!["TAP THE CONTROL AGAIN"]];
            } else if !crate::padmap::capturable(icode) {
                r.probe = None;
                r.lines = [vec!["DIRECTIONS CANNOT CHANGE"], vec!["THEY WALK AND MOVE THE CURSOR"]];
            } else {
                let key = gamepad::key_for(&r.draft, code);
                r.probe = key.and_then(gamepad::remap_row);
                r.sel = None;
                r.lines = [vec![name, "=", gamepad::key_label(key)], vec!["TO CHANGE IT TAP A CONTROL FIRST"]];
            }
            return;
        };
        let (key, label) = gamepad::REMAP_ROWS[row];
        let mut table = r.draft.table();
        let result = gamepad::pad().assign(&mut table, key, icode);
        r.draft = Overrides::from_table(table);
        use crate::padmap::Assign;
        r.lines = match result {
            // The prompt stays up: a direction is not an answer, it is a wrong key.
            Assign::NotCapturable => [vec!["DIRECTIONS CANNOT CHANGE"], Vec::new()],
            Assign::Same => [vec![label, "=", name], vec!["NOTHING CHANGED"]],
            Assign::Refused { lost } => [vec![gamepad::key_label(Some(lost)), "WOULD HAVE NO BUTTON"], vec!["TRY ANOTHER PAD BUTTON"]],
            // The second line says where the swap sent the other function, in the first one's form.
            Assign::Done { displaced: Some(d), to: Some(from) } => {
                [vec![label, "=", name], vec![gamepad::key_label(Some(d)), "=", gamepad::button_name(u32::try_from(from).unwrap_or(0))]]
            }
            Assign::Done { displaced: Some(d), to: None } => [vec![label, "=", name], vec![gamepad::key_label(Some(d)), "=", "NO BUTTON"]],
            Assign::Done { displaced: None, .. } => [vec![label, "=", name], vec!["SAVE TO APPLY"]],
        };
        if result != Assign::NotCapturable {
            r.capture = None;
        }
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    pub fn joy(&self) -> Option<Joy> {
        self.joy
    }

    /// Keys a finger is holding (for the pressed look).
    pub fn held(&self) -> HashSet<KeyCode> {
        self.keys.values().copied().chain(self.joy.and_then(|j| j.way).into_iter().flat_map(hud::Way::keys)).collect()
    }

    pub fn view(&self) -> GuestView {
        self.view
    }

    pub fn heat_saver(&self) -> bool {
        self.heat_saver
    }

    /// What the settings panel shows for HEAT SAVER (seeded from `crate::config` at start).
    pub fn set_heat_saver(&mut self, on: bool) {
        self.heat_saver = on;
    }

    // ---------------------------------------------------------------------- gamepad ----

    pub fn pad_hud(&self) -> PadHud {
        self.pad_hud
    }

    /// The PAD BADGES preference (seeded from `crate::config` at start).
    /// The STATS preference (seeded from `crate::config` at start).
    pub fn set_stats_on(&mut self, on: bool) {
        self.stats_on = on;
    }

    /// Whether the status block shows — always once the game's bar is gone (Phase 6-4): it is
    /// then the only place HP is read, so the STATS preference cannot hide it.
    pub fn stats_on(&self) -> bool {
        self.stats_on || self.bar_off
    }

    /// The player's numbers, from the worker (`UserEvent::Stats`).
    pub fn set_stats(&mut self, stats: Stats) {
        self.stats = Some(stats);
    }

    /// What the status block (and the EXP bar) shows: the numbers, in play (the field and the
    /// game's windows over it — hero2's menu mode, A7: [`GuestView::in_play`]), with STATS on and
    /// no host UI up.
    pub fn stats_shown(&self) -> Option<Stats> {
        if self.stats_on() && self.view.in_play() && !self.settings && self.editor.is_none() && self.remap.is_none() { self.stats } else { None }
    }

    /// Whether the 창고 question's answer line shows ([`hud::answer_hint`]): while that window has
    /// the keys ([`GuestView::lsk_ok`]) and no host window is over it.
    pub fn answer_hint_shown(&self) -> bool {
        self.view.lsk_ok && !self.settings && self.editor.is_none() && self.remap.is_none() && self.world.is_none()
    }

    /// Whether the game's bottom bar is off (HZ-013 in this boot's patches — fixed at boot): the
    /// status block is then always on, and the EXP bar is the host's.
    pub fn set_bar_off(&mut self, off: bool) {
        self.bar_off = off;
    }

    pub fn bar_off(&self) -> bool {
        self.bar_off
    }

    /// The quick slots, from the worker (`UserEvent::Quick`). A window's 「퀵슬롯등록」 starting to
    /// wait for a plate says so (HZ-058).
    pub fn set_quick(&mut self, bar: QuickBar) {
        if bar.picking && !self.quick.as_ref().is_some_and(|q| q.picking) {
            self.notice("PICK A QUICK SLOT");
        }
        self.quick = Some(bar);
    }

    /// What the slot keys show as plates: the slots, in play ([`GuestView::in_play`] — over the
    /// menu too, A7), once read. Elsewhere (and before the first read) the keys keep their labels.
    pub fn quick_shown(&self) -> Option<&QuickBar> {
        if self.view.in_play() { self.quick.as_ref() } else { None }
    }

    /// The quick slots as last read, wherever the game is (the pad screen shows the field's plates).
    pub fn quick_bar(&self) -> Option<&QuickBar> {
        self.quick.as_ref()
    }

    /// The map, from the worker (`UserEvent::Map`). The first one brings the panel's tap target,
    /// and one the target panel sits elsewhere over moves the tabs ([`Hud::tab_map_w`]).
    pub fn set_map(&mut self, map: MapGrid) {
        let (first, tabs_w) = (self.map.is_none(), self.tab_map_w());
        self.map = Some(map);
        if first || self.tab_map_w() != tabs_w {
            self.relayout();
        }
    }

    /// The width (guest px) of the map the top tabs keep clear of the target panel over
    /// ([`hud::top_tab_seats`]): this map's on a map with monsters (or one whose file is unread),
    /// the canvas's where the panel is never drawn — a town's tabs stay as they are.
    fn tab_map_w(&self) -> i32 {
        match &self.map {
            Some(m) if m.fights != Some(false) => i32::from(m.w) * hz_java::hooks::MAP_TILE,
            _ => self.screen.guest_w,
        }
    }

    /// What moves on it, from the worker (`UserEvent::Marks`).
    pub fn set_marks(&mut self, marks: MapMarks) {
        self.marks = marks;
    }

    /// The head for the status block's ring, from the worker (`UserEvent::Portrait`).
    pub fn set_portrait(&mut self, portrait: Option<Icon>) {
        self.portrait = portrait;
    }

    /// What the ring shows: the head, wherever the block shows (`None` = the dark ground).
    pub fn portrait(&self) -> Option<&Icon> {
        self.portrait.as_ref()
    }

    /// The player's minimap preference (seeded from `crate::config` at start).
    pub fn set_minimap_on(&mut self, on: bool) {
        if on != self.minimap_on {
            self.minimap_on = on;
            self.relayout();
        }
    }

    pub fn minimap_on(&self) -> bool {
        self.minimap_on
    }

    /// What the minimap panel draws, and where: in the field, once a map is read, with the panel
    /// up and no host UI over the picture.
    pub fn minimap_shown(&self) -> Option<(&MapGrid, &MapMarks, Rect)> {
        match self.layout.minimap {
            Some(hud::Minimap::Panel(r)) if !self.settings && self.editor.is_none() && self.remap.is_none() => self.map.as_ref().map(|m| (m, &self.marks, r)),
            _ => None,
        }
    }

    /// The restore button, while the panel is tapped away (field only).
    pub fn minimap_restore_shown(&self) -> Option<Rect> {
        match self.layout.minimap {
            Some(hud::Minimap::Restore(r)) if !self.settings && self.editor.is_none() && self.remap.is_none() => Some(r),
            _ => None,
        }
    }

    /// HZ-014 / HZ-054: where the game's target panel goes, `(shift, drop)` in guest px — clear of
    /// the minimap while the player has it up (left of it, or in portrait under it), counted over
    /// the map it is drawn on (a narrow one pulls it into its picture), where the game puts it
    /// otherwise. Not tied to the scene: the target panel is only drawn in the field anyway, and a
    /// value that followed the menus would only churn.
    pub fn target_place(&self) -> (i32, i32) {
        match &self.map {
            Some(m) if self.minimap_on => hud::target_place(&self.screen, self.layout.panel, i32::from(m.w) * hz_java::hooks::MAP_TILE),
            _ => (0, 0),
        }
    }

    /// Is a point badge visible this instant ([`BADGE_BLINK_MS`] on, as long off)?
    pub fn badge_blink_on(&self) -> bool {
        (self.born.elapsed().as_millis() / BADGE_BLINK_MS) % 2 == 0
    }

    pub fn set_pad_hud(&mut self, mode: PadHud) {
        let walk = self.walk();
        self.pad_hud = mode;
        if self.walk() != walk {
            self.relayout();
        }
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// The player's remap (`gamepad::Overrides::load` at start), so the badges point at the
    /// button that really fires a control.
    pub fn set_overrides(&mut self, ov: Overrides) {
        self.overrides = ov;
    }

    /// A pad button went down or an axis moved: the pad is now the last input source (PAD
    /// BADGES AUTO shows the badges until the next touch). The first time in a run, a notice
    /// tells the player what the badges are.
    pub fn pad_input(&mut self) {
        if !self.pad_seen {
            self.pad_seen = true;
            self.notice("GAMEPAD - BADGES SHOW ITS BUTTONS");
        }
        let walk = self.walk();
        self.pad_last = true;
        // The arrows and the stick stand down while the pad drives (hero2's pad HUD).
        if self.walk() != walk {
            self.relayout();
        }
    }

    pub fn pad_last(&self) -> bool {
        self.pad_last
    }

    /// Are the badges showing now?
    pub fn badges_shown(&self) -> bool {
        self.pad_hud.active(self.pad_last) && self.editor.is_none() && self.remap.is_none()
    }

    /// One badge per control (the plates, OK, CLR, a window's arrows) whose key a pad button fires, and
    /// per top tab a button is given (hero2's SAVE and MAP badges), with the PRIMARY button's
    /// position (`gamepad::codes_for`). Empty unless [`Self::badges_shown`].
    pub fn badges(&self) -> Vec<(Badge, PadPos)> {
        if !self.badges_shown() {
            return Vec::new();
        }
        let s = self.screen;
        let badge = |action: gamepad::PadAction, r: Rect| {
            let code = *gamepad::codes_for(&self.overrides, action).first()?;
            Some((padhud::badge_at(r, s.w, s.h)?, padhud::pos_for_code(code)?))
        };
        let controls = self
            .layout
            .controls
            .iter()
            .filter(|c| c.face != hud::Face::Hidden)
            .filter_map(|c| badge(gamepad::PadAction::Key(c.action.pad_key()?), c.shape.bounds()));
        let tabs = self.tabs_shown().iter().filter_map(|(r, t)| badge(gamepad::tab_action(*t), *r));
        controls.chain(tabs).collect()
    }

    // ------------------------------------------------------------------------ toast ----

    /// Show `text` — a `crate::host_text` line — for [`TOAST_MS`] (B5: hero2's stack, the newest
    /// lowest; the same line again moves to the bottom with a new clock rather than stacking twice).
    pub fn notice(&mut self, text: &'static str) {
        self.notice_at(text, Instant::now());
    }

    pub fn notice_at(&mut self, text: &'static str, now: Instant) {
        self.notes.retain(|n| n.what != Notice::Line(text));
        self.push_note(Notice::Line(text), now);
    }

    /// A pickup notice the game made (HZ-025), into the same stack for as long as a line — each its
    /// own, however alike (two potions picked up are two notices, as the game's list shows them).
    pub fn pickup(&mut self, picture: Icon) {
        self.pickup_at(picture, Instant::now());
    }

    pub fn pickup_at(&mut self, picture: Icon, now: Instant) {
        self.push_note(Notice::Picture(picture), now);
    }

    fn push_note(&mut self, what: Notice, now: Instant) {
        self.push(Note { what, born: now, held: false }, now);
    }

    fn push(&mut self, note: Note, now: Instant) {
        self.notes.push_back(note);
        // Over the bound the oldest goes — never the countdown.
        while self.notes.len() > NOTICE_MAX {
            match self.notes.iter().position(|n| !n.held) {
                Some(i) => drop(self.notes.remove(i)),
                None => break,
            }
        }
        self.pushed = Some(now);
    }

    /// HZ-061: a boss's story counting down — `text` (a `crate::host_text::STORY_WAIT` line) held in
    /// the stack, the next second replacing the last in place, as hero2's raid countdown does
    /// (「다음 보스로 이동 중...5」); `None` takes it away (the story started, or its wait was dropped).
    pub fn countdown(&mut self, text: Option<&'static str>) {
        self.countdown_at(text, Instant::now());
    }

    pub fn countdown_at(&mut self, text: Option<&'static str>, now: Instant) {
        let Some(text) = text else {
            self.notes.retain(|n| !n.held);
            return;
        };
        match self.notes.iter_mut().find(|n| n.held) {
            Some(n) => (n.what, n.born) = (Notice::Line(text), now),
            None => self.push(Note { what: Notice::Line(text), born: now, held: true }, now),
        }
    }

    /// The newest notice up (the game stopped, else the newest of the stack, else one held over
    /// the title) — the PAD REMAP panel's hint line shows it.
    pub fn toast(&self) -> Option<&'static str> {
        self.toast_at(Instant::now())
    }

    /// The stack to draw, oldest first: each line and the slot it is at (0 = the bottom row; the
    /// older ones slide up from a slot lower for [`NOTICE_SLIDE_MS`] after a new one comes). A
    /// notice held over the title is the oldest, for as long as the title is up.
    pub fn notices_at(&self, now: Instant) -> Vec<(Notice, f32)> {
        let mut lines: Vec<Notice> = self.notes.iter().filter(|n| n.up_at(now)).map(|n| n.what.clone()).collect();
        if let Some(held) = self.title_notice.filter(|t| self.view.scene == Scene::Title && !lines.contains(&Notice::Line(t))) {
            lines.insert(0, Notice::Line(held));
        }
        let slide = self.pushed.map_or(1.0, |p| (now.saturating_duration_since(p).as_millis() as f32 / NOTICE_SLIDE_MS as f32).min(1.0));
        let n = lines.len();
        lines
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let slot = (n - 1 - i) as f32;
                (t, if slot > 0.0 { slot - (1.0 - slide) } else { 0.0 })
            })
            .collect()
    }

    /// The game stopped (`notice_stuck`): the line for the fatal box.
    pub fn stuck(&self) -> Option<&'static str> {
        self.stuck
    }

    /// Show `text` now as [`notice`](Self::notice) does, and again for as long as the game shows
    /// its title — the menu, the slot list, the job choice — then for [`TOAST_MS`] after it leaves.
    ///
    /// For a save that cannot be read: the game reads the slots during its boot load, so a plain
    /// notice was gone with the EA logo (S9 2026-10-23), and the slot list then showed the slot as
    /// 「빈 슬롯」 with nothing to say why — the moment a player starts a new game in it.
    pub fn notice_through_title(&mut self, text: &'static str) {
        self.notice(text);
        self.title_notice = Some(text);
    }

    pub fn toast_at(&self, now: Instant) -> Option<&'static str> {
        let held = self.title_notice.filter(|_| self.view.scene == Scene::Title);
        let newest = self.notes.iter().rev().filter(|n| n.up_at(now)).find_map(|n| match n.what {
            Notice::Line(t) => Some(t),
            Notice::Picture(_) => None,
        });
        self.stuck.or(newest).or(held)
    }

    /// When the stack next changes — a notice goes, or a slide ends (the app sleeps until then,
    /// then redraws and [`expire_notices`](Self::expire_notices)).
    pub fn toast_until(&self) -> Option<Instant> {
        let goes = self.notes.iter().filter(|n| !n.held).map(|n| n.born + Duration::from_millis(TOAST_MS)).min();
        let slid = self.pushed.map(|p| p + Duration::from_millis(NOTICE_SLIDE_MS));
        match (goes, slid) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Drop the notices whose time is up, and the slide once it is over.
    pub fn expire_notices(&mut self, now: Instant) {
        self.notes.retain(|n| n.up_at(now));
        if self.pushed.is_some_and(|p| now >= p + Duration::from_millis(NOTICE_SLIDE_MS)) {
            self.pushed = None;
        }
    }

    /// Every notice gone at once.
    pub fn clear_toast(&mut self) {
        self.notes.clear();
        self.pushed = None;
    }

    fn relayout(&mut self) {
        self.layout = hud::layout_for(self.screen, &self.table, self.view, self.walk());
        self.layout.minimap = hud::minimap_target(&self.screen, self.layout.panel, self.view, self.map.as_ref().map(|_| self.minimap_on));
        self.layout.tabs = hud::top_tabs(&self.screen, self.view, true, self.tab_map_w());
        // A new window (rotation): the open map's camera re-frames its cursor for the new panel.
        if let Some(cursor) = self.world.map(|w| w.cursor) {
            let cam = self.camera_on(cursor);
            if let Some(w) = self.world.as_mut() {
                (w.cam, w.target) = (cam, cam);
            }
        }
    }

    /// The top tabs to draw: the layout's, unless a host panel or the editor is up.
    pub fn tabs_shown(&self) -> &[(Rect, hud::Tab)] {
        if self.settings || self.remap.is_some() || self.editor.is_some() { &[] } else { &self.layout.tabs }
    }

    /// Where tab `t` is drawn now, if it is.
    pub fn tab(&self, t: hud::Tab) -> Option<Rect> {
        self.tabs_shown().iter().find(|(_, x)| *x == t).map(|(r, _)| *r)
    }

    /// The window changed size or orientation: whatever a finger held belongs to the old layout.
    pub fn set_screen(&mut self, screen: Screen) -> Vec<Out> {
        if screen == self.screen {
            return Vec::new();
        }
        let out = self.release_all();
        self.screen = screen;
        self.relayout();
        out
    }

    /// The game moved to another scene: the controls it does not listen to there go away. A
    /// finger already holding one keeps its key until it lifts (`ended` releases by finger, not
    /// by control), so nothing is sent here.
    pub fn set_view(&mut self, view: GuestView) -> Vec<Out> {
        if view != self.view {
            if self.view.scene == Scene::Title
                && view.scene != Scene::Title
                && let Some(text) = self.title_notice.take()
            {
                self.notice(text);
            }
            self.view = view;
            self.relayout();
        }
        Vec::new()
    }

    // ------------------------------------------------------------------- key edges ----

    /// A finger on control `i` (its key `k`) at (`x`, `y`): that key — and on the field's D-pad, in the
    /// gap between two arms (HZ-059, [`hud::dpad_corner`]), the diagonal's two, the second held by the
    /// finger's [`corner_finger`].
    fn press_control(&mut self, id: u64, i: usize, k: KeyCode, x: f64, y: f64, out: &mut Vec<Out>) {
        let corner = match self.layout.controls[i].shape {
            hud::Shape::Wedge { square, .. } if self.eight_ways() => hud::dpad_corner(square, x, y),
            _ => None,
        };
        let (k, second) = match corner {
            Some((h, v)) => (h, Some(v)),
            None => (k, None),
        };
        self.update_touch(id, Some(k), out);
        self.update_touch(corner_finger(id), second, out);
    }

    /// The ONE place key edges come from (input-and-hud.md §3).
    fn update_touch(&mut self, id: u64, new: Option<KeyCode>, out: &mut Vec<Out>) {
        let old = self.keys.get(&id).copied();
        if old == new {
            return;
        }
        match new {
            Some(k) => self.keys.insert(id, k),
            None => self.keys.remove(&id),
        };
        if let Some(k) = old
            && !self.keys.values().any(|v| *v == k)
        {
            out.push(Out::Key { code: k, down: false });
        }
        if let Some(k) = new
            && self.keys.values().filter(|v| **v == k).count() == 1
        {
            out.push(Out::Key { code: k, down: true });
        }
        let ok_held = self.keys.values().any(|v| *v == KeyCode::OK);
        match (ok_held, self.fire_next) {
            (true, None) => self.fire_next = Some(Instant::now() + FIRE_REPEAT_DELAY),
            (false, Some(_)) => self.fire_next = None,
            _ => {}
        }
    }

    /// When the held OK's next repeat is due (the app wakes for it).
    pub fn fire_next(&self) -> Option<Instant> {
        self.fire_next
    }

    /// The held OK's repeat, if due at `now`: let go and press again — in the bare field only (a
    /// window takes OK as "choose", and choosing twice is not a combo). A wake after a long sleep
    /// repeats once, not for every period missed.
    pub fn fire_repeat_at(&mut self, now: Instant) -> Vec<Out> {
        let Some(due) = self.fire_next else { return Vec::new() };
        if now < due {
            return Vec::new();
        }
        let next = due + FIRE_REPEAT_EVERY;
        self.fire_next = Some(if next <= now { now + FIRE_REPEAT_EVERY } else { next });
        if self.view.scene == Scene::Field && !self.view.popup && !self.view.notice && self.keys.values().any(|v| *v == KeyCode::OK) {
            vec![Out::Key { code: KeyCode::OK, down: false }, Out::Key { code: KeyCode::OK, down: true }]
        } else {
            Vec::new()
        }
    }

    /// Everything up: suspend, focus loss, the editor opening, a new screen.
    pub fn release_all(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        for id in self.keys.keys().copied().collect::<Vec<_>>() {
            self.update_touch(id, None, &mut out);
        }
        self.joy_end(&mut out);
        self.ui.clear();
        self.slide.clear();
        self.world_arms.clear();
        self.tap = None;
        self.host_drag = None;
        if let Some(ed) = self.editor.as_mut() {
            ed.drag = None;
            ed.pinch = None;
            ed.points.clear();
        }
        out
    }

    // ---------------------------------------------------------------------- phases ----

    pub fn started(&mut self, id: u64, x: f64, y: f64) -> Vec<Out> {
        let walk = self.walk();
        let out = self.touch_started(id, x, y);
        // A touch is the last input source: PAD BADGES AUTO hides the badges again, and the walk
        // controls come back — after this touch, which landed where none were (hero2's: a live
        // target nobody could see would have taken it).
        self.pad_last = false;
        if self.walk() != walk {
            self.relayout();
        }
        out
    }

    fn touch_started(&mut self, id: u64, x: f64, y: f64) -> Vec<Out> {
        let mut out = Vec::new();
        // A host window (the PC's dialogs) is modal over everything.
        if self.host.is_some() {
            self.ui.insert(id);
            self.host_started(id, x, y);
            return out;
        }
        // The world map is modal: every finger is its own.
        if self.world.is_some() {
            self.ui.insert(id);
            self.world_started(id, x, y, &mut out);
            return out;
        }
        if self.editor.is_some() {
            self.editor_started(id, x, y, &mut out);
            return out;
        }
        if self.remap.is_some() {
            self.ui.insert(id);
            self.remap_started(x, y, &mut out);
            return out;
        }
        if self.settings {
            self.ui.insert(id);
            // In the list a press may be the start of a scroll, so its row is decided when the
            // finger lifts (hero2 2026-09-24); 닫기 and a tap outside close at once.
            let in_list = hud::settings_geom(&self.screen).view.contains(x, y) && !hud::settings_close(&self.screen).contains(x, y);
            if in_list {
                if self.list_drag.is_none() {
                    self.list_drag = Some(ListDrag { id, from: (x, y), scroll0: self.settings_scroll, moved: false });
                }
            } else if hud::settings_hit(&self.screen, &self.settings_list(), self.settings_scroll, x, y) == Some(SettingsAction::Close) {
                self.close_settings();
            }
            return out;
        }
        match hud::hit(&self.layout, x, y) {
            Hit::Gear => {
                self.ui.insert(id);
                // The list opens at its top, where the familiar rows are.
                self.settings = true;
                self.settings_scroll = 0;
                self.list_drag = None;
            }
            Hit::Tab(hud::Tab::Save) => {
                self.ui.insert(id);
                out.push(Out::QuickSave);
            }
            Hit::Tab(hud::Tab::Map) => {
                self.open_world(&mut out);
                self.ui.insert(id);
            }
            Hit::Control(i) => match self.layout.controls[i].action {
                Action::Key(k) => {
                    self.slide.insert(id, self.layout.controls[i].id);
                    self.press_control(id, i, k, x, y, &mut out);
                }
                // A plate is a tap: one use (or one choice) at the press, nothing held.
                Action::Plate(n) => {
                    self.ui.insert(id);
                    out.push(Out::Plate(n));
                }
                Action::Page => {
                    self.ui.insert(id);
                    out.push(Out::PlatePage);
                }
            },
            // Between a window's arrows: nothing, and not the window under them.
            Hit::Dead => {
                self.ui.insert(id);
            }
            // hero2's: a tap on the panel puts it away, the button in its corner brings it back —
            // when the finger lifts ([`Hud::ended`]).
            Hit::Minimap => {
                self.ui.insert(id);
                self.tap = Some(Tap { id, on: TapOn::Minimap, from: (x, y), at: (x, y) });
            }
            // The status block: the menu, when the finger lifts — while the block is drawn (an
            // invisible target is a bug); else the field's nothing.
            Hit::Status if self.stats_shown().is_some() => {
                self.ui.insert(id);
                self.tap = Some(Tap { id, on: TapOn::Status, from: (x, y), at: (x, y) });
            }
            Hit::Status => {}
            Hit::StickZone => self.joy_start(id, x, y, &mut out),
            // The game's own windows take taps (the bare field has none to take), and a dialogue's
            // picture turns its page (hero3's cutscenes; the ▶▶ skips).
            Hit::Nothing => {
                let bare_field = self.view.scene == Scene::Field && !self.view.popup && !self.view.notice;
                if let Some((gx, gy)) = self.screen.guest_point(x, y).filter(|_| !bare_field) {
                    self.ui.insert(id);
                    out.push(Out::GuestTap(gx, gy));
                }
            }
        }
        out
    }

    pub fn moved(&mut self, id: u64, x: f64, y: f64) -> Vec<Out> {
        let mut out = Vec::new();
        if self.host_drag.is_some_and(|d| d.id == id) {
            self.host_moved(x, y);
            return out;
        }
        // The settings list follows a finger once it has wandered past a tap.
        if self.list_drag.is_some_and(|d| d.id == id) {
            let max = hud::settings_scroll_max(&self.screen, self.settings_list().len());
            if let Some(d) = self.list_drag.as_mut()
                && (d.moved || (x - d.from.0).abs() > TAP_SLOP || (y - d.from.1).abs() > TAP_SLOP)
            {
                d.moved = true;
                self.settings_scroll = (f64::from(d.scroll0) - (y - d.from.1)).clamp(0.0, f64::from(max)) as i32;
            }
            return out;
        }
        if self.editor.is_some() {
            self.editor_moved(id, x, y, &mut out);
            return out;
        }
        if let Some(t) = self.tap.as_mut().filter(|t| t.id == id) {
            t.at = (x, y);
            // Smeared into a drag: not a tap. The finger stays the host's.
            if (x - t.from.0).abs() > TAP_SLOP || (y - t.from.1).abs() > TAP_SLOP {
                self.tap = None;
            }
        } else if self.joy.is_some_and(|j| j.id == id) {
            self.joy_update(x, y, &mut out);
        } else if let Some(&on) = self.slide.get(&id) {
            // Only a finger that started on a control may change key by sliding; over anything
            // that is not a game key it keeps the key it has until it lifts.
            let k = match hud::hit(&self.layout, x, y) {
                Hit::Control(i) => Some((self.layout.controls[i].id, self.layout.controls[i].action, i)),
                _ => None,
            };
            // Still on the seat it pressed: it keeps the key it pressed, even if the scene changed
            // that seat's key meanwhile. CLR in the field presses LSK, the menu opens, CLR becomes
            // RSK — re-read on the next Moved (a resting finger sends them), it said "back" and
            // shut the menu it had just opened.
            if let Some((seat, Action::Key(k), i)) = k
                && !(seat.is_some() && seat == on)
            {
                self.slide.insert(id, seat);
                self.press_control(id, i, k, x, y, &mut out);
            }
        }
        out
    }

    pub fn ended(&mut self, id: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if self.host_drag.is_some_and(|d| d.id == id) {
            self.host_lifted(true, &mut out);
            self.ui.remove(&id);
            return out;
        }
        if self.list_drag.is_some_and(|d| d.id == id) {
            self.list_lifted(true, &mut out);
            self.ui.remove(&id);
            return out;
        }
        if self.editor.is_some() {
            self.editor_ended(id, &mut out);
            return out;
        }
        if let Some(t) = self.tap.filter(|t| t.id == id) {
            self.tap = None;
            self.tap_lifted(t, &mut out);
        }
        if self.joy.is_some_and(|j| j.id == id) {
            self.joy_end(&mut out);
        }
        if let Some(k) = self.world_arms.remove(&id) {
            self.world_key_up(k);
        }
        self.update_touch(id, None, &mut out);
        self.update_touch(corner_finger(id), None, &mut out);
        self.ui.remove(&id);
        self.slide.remove(&id);
        out
    }

    /// The system took the finger back (winit's `Cancelled`): it lifts, and a tap it was on never
    /// counts.
    pub fn cancelled(&mut self, id: u64) -> Vec<Out> {
        if self.tap.is_some_and(|t| t.id == id) {
            self.tap = None;
        }
        let mut out = Vec::new();
        if self.host_drag.is_some_and(|d| d.id == id) {
            self.host_lifted(false, &mut out);
        }
        if self.list_drag.is_some_and(|d| d.id == id) {
            self.list_lifted(false, &mut out);
        }
        out.extend(self.ended(id));
        out
    }

    /// The finger on the settings list lifted (`act`) or was taken back: a drag settles on a whole
    /// row — the list at rest never shows a plate cut in half — and a tap acts on the row it came
    /// down on.
    fn list_lifted(&mut self, act: bool, out: &mut Vec<Out>) {
        let Some(d) = self.list_drag.take() else { return };
        let rows = self.settings_list();
        if d.moved {
            let pitch = hud::settings_geom(&self.screen).pitch.max(1);
            let max = hud::settings_scroll_max(&self.screen, rows.len());
            self.settings_scroll = ((self.settings_scroll + pitch / 2) / pitch * pitch).min(max);
        } else if act && let Some(a) = hud::settings_hit(&self.screen, &rows, self.settings_scroll, d.from.0, d.from.1) {
            self.settings_act(a, out);
        }
    }

    /// A settings row acts (B2).
    fn settings_act(&mut self, a: SettingsAction, out: &mut Vec<Out>) {
        match a {
            SettingsAction::LayoutEdit => out.extend(self.enter_editor()),
            SettingsAction::Diag => {
                self.close_settings();
                out.push(Out::OpenDiag);
            }
            SettingsAction::HeatSaver => {
                self.heat_saver = !self.heat_saver;
                out.push(Out::HeatSaver(self.heat_saver));
                self.notice(if self.heat_saver { "HEAT SAVER ON - NEXT START" } else { "HEAT SAVER OFF - NEXT START" });
            }
            SettingsAction::SfxLevel => {
                self.sfx_db = crate::audio_mix::next_sfx_db(self.sfx_db);
                out.push(Out::SfxLevel(self.sfx_db));
            }
            SettingsAction::Stats => {
                self.stats_on = !self.stats_on;
                out.push(Out::Stats(self.stats_on));
            }
            SettingsAction::PadHud => {
                self.pad_hud = self.pad_hud.next();
                out.push(Out::PadHud(self.pad_hud));
            }
            SettingsAction::PadRemap => self.open_remap(out),
            SettingsAction::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                out.push(Out::Fullscreen(self.fullscreen));
            }
            SettingsAction::Close => self.close_settings(),
        }
    }

    fn close_settings(&mut self) {
        self.settings = false;
        self.list_drag = None;
    }

    /// A tap lifted where it came down, still on its target: it acts.
    fn tap_lifted(&mut self, t: Tap, out: &mut Vec<Out>) {
        let (x, y) = t.at;
        match t.on {
            TapOn::Minimap if self.layout.minimap.is_some_and(|m| m.target(&self.screen).contains(x, y)) => {
                self.minimap_on = !self.minimap_on;
                self.relayout();
                out.push(Out::Minimap(self.minimap_on));
            }
            // The field's menu key — CLR's there — while the field's set still stands.
            TapOn::Status if self.layout.status.is_some_and(|r| r.contains(x, y)) && self.stats_shown().is_some() => {
                let code = hud::cancel_key(self.view);
                out.push(Out::Key { code, down: true });
                out.push(Out::Key { code, down: false });
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------ stick ----

    /// A thumb on the stick's zone: its centre where it lands (자유), or the seat's centre (고정 —
    /// then the thumb is already a deflection, and the key follows at once).
    fn joy_start(&mut self, id: u64, x: f64, y: f64, out: &mut Vec<Out>) {
        if self.joy.is_some() {
            return;
        }
        let fixed = self.control == ControlMode::Fixed;
        let (cx, cy) = match self.layout.stick.filter(|_| fixed) {
            Some(st) => {
                let (sx, sy) = st.seat.centre();
                (f64::from(sx), f64::from(sy))
            }
            None => (x, y),
        };
        self.joy = Some(Joy { id, cx, cy, tx: x, ty: y, way: None });
        if fixed {
            self.joy_update(x, y, out);
        }
    }

    fn joy_update(&mut self, x: f64, y: f64, out: &mut Vec<Out>) {
        let Some(st) = self.layout.stick else { return };
        let eight = self.eight_ways();
        let Some(joy) = self.joy.as_mut() else { return };
        joy.tx = x;
        joy.ty = y;
        let want = hud::joy_way(x - joy.cx, y - joy.cy, st.engage_r, joy.way, eight);
        if want != joy.way {
            out.extend(hud::way_edges(joy.way, want).into_iter().map(|(code, down)| Out::Key { code, down }));
            joy.way = want;
        }
    }

    fn joy_end(&mut self, out: &mut Vec<Out>) {
        if let Some(Joy { way, .. }) = self.joy.take() {
            out.extend(hud::way_edges(way, None).into_iter().map(|(code, down)| Out::Key { code, down }));
        }
    }

    /// HZ-059: the arrows walk eight ways — the bare field (a window's cursor steps four).
    fn eight_ways(&self) -> bool {
        self.view.scene == Scene::Field && !self.view.popup && !self.view.notice && !self.view.window
    }

    // ----------------------------------------------------------------------- editor ----

    fn enter_editor(&mut self) -> Vec<Out> {
        let out = self.release_all();
        self.close_settings();
        self.editor = Some(Editor {
            sel: None,
            pad: false,
            snapshot: self.table.clone(),
            drag: None,
            pinch: None,
            points: HashMap::new(),
        });
        self.relayout();
        out
    }

    /// The resolved rect of every placeable control the editor offers — what it draws and lets you
    /// grab, one list for both (hero2's `in_editor`): a plate switched off is neither, until SHOW.
    pub fn editor_rects(&self) -> Vec<(CtrlId, Rect)> {
        placement::resolve_with(&self.screen, &self.table).into_iter().filter(|(id, _)| !self.table.is_hidden(*id)).collect()
    }

    /// The pad position whose badge control `id` carries in play (hero2's `editor_pad_pos`) — the
    /// editor's PAD preview: OK's, the field CLR's (the menu, RSK), each plate's key's, the page's
    /// (START's LSK); none for the stick (the pad walks with its own) or the minimap (a picture). The
    /// player's remap first, as the live badges ([`Hud::badges`]).
    pub fn editor_pad_pos(&self, id: CtrlId) -> Option<PadPos> {
        let key = match id {
            CtrlId::Ok => KeyCode::OK,
            CtrlId::Cancel => hud::cancel_key(self.view),
            CtrlId::Ring(i) => hud::Action::Plate(hud::RING_PLATES[i as usize].0).pad_key()?,
            CtrlId::Page => hud::Action::Page.pad_key()?,
            CtrlId::Stick | CtrlId::Minimap => return None,
        };
        padhud::pos_for_code(*gamepad::codes_for(&self.overrides, gamepad::PadAction::Key(key)).first()?)
    }

    /// Can the editor's button act now? HIDE needs a plate selected that may go (not the last one
    /// up), SHOW a plate to bring back (hero2's: a button that silently does nothing reads as a
    /// broken app, so it says so on its face).
    pub fn edit_enabled(&self, a: EditAction) -> bool {
        match a {
            EditAction::Hide => self.editor.as_ref().and_then(|e| e.sel).is_some_and(|id| self.table.can_hide(id)),
            EditAction::Show => self.table.any_hidden(),
            _ => true,
        }
    }

    fn centre_of(&self, id: CtrlId) -> (f64, f64) {
        match self.table.get(id) {
            Some(p) => (f64::from(p.cx) * f64::from(self.screen.w), f64::from(p.cy) * f64::from(self.screen.h)),
            None => {
                let (cx, cy) = placement::builtin(id, &self.screen).centre();
                (f64::from(cx), f64::from(cy))
            }
        }
    }

    /// Store a new centre and percent for `id` if the resolver would honour it without moving
    /// anything else; otherwise nothing changes (the control slides along what blocks it).
    fn editor_commit(&mut self, id: CtrlId, cx: f64, cy: f64, pct: u32) -> bool {
        let s = self.screen;
        let place = Place {
            cx: (cx / f64::from(s.w.max(1))) as f32,
            cy: (cy / f64::from(s.h.max(1))) as f32,
            pct: pct.clamp(placement::PCT_MIN, placement::PCT_MAX),
        };
        if !placement::would_accept_place(&s, &self.table, id, place) {
            return false;
        }
        self.table.set(id, Some(place));
        self.relayout();
        true
    }

    fn editor_started(&mut self, id: u64, x: f64, y: f64, out: &mut Vec<Out>) {
        if let Some((_, action)) = hud::editor_chrome(&self.screen).into_iter().find(|(r, _)| r.contains(x, y)) {
            match action {
                // A preview only: the selection and every placement stay as they are.
                EditAction::Pad => {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.pad = !ed.pad;
                    }
                }
                EditAction::Hide => match self.editor.as_ref().and_then(|e| e.sel) {
                    Some(sel @ CtrlId::Ring(_)) if self.table.hide(sel) => {
                        if let Some(ed) = self.editor.as_mut() {
                            (ed.sel, ed.drag, ed.pinch) = (None, None, None);
                        }
                        out.push(Out::SaveLayout);
                    }
                    Some(CtrlId::Ring(_)) => self.notice("KEEP ONE QUICK SLOT"),
                    _ => self.notice("SELECT A QUICK SLOT TO HIDE"),
                },
                EditAction::Show => {
                    if self.table.show_all() {
                        out.push(Out::SaveLayout);
                        self.notice("QUICK SLOTS ALL SHOWN");
                    } else {
                        self.notice("NO QUICK SLOT IS HIDDEN");
                    }
                }
                EditAction::Reset => {
                    self.table.clear();
                    if let Some(ed) = self.editor.as_mut() {
                        (ed.sel, ed.drag, ed.pinch) = (None, None, None);
                    }
                    out.push(Out::SaveLayout);
                }
                EditAction::Flip => {
                    let s = self.screen;
                    self.table.mirror(&s);
                    if let Some(ed) = self.editor.as_mut() {
                        (ed.drag, ed.pinch) = (None, None);
                    }
                    out.push(Out::SaveLayout);
                }
                EditAction::Cancel => {
                    if let Some(ed) = self.editor.take() {
                        self.table = ed.snapshot;
                    }
                    out.push(Out::SaveLayout);
                    self.notice("LAYOUT RESTORED");
                }
                EditAction::Save => {
                    self.editor = None;
                    out.push(Out::SaveLayout);
                    self.notice("LAYOUT SAVED");
                }
            }
            if let Some(ed) = self.editor.as_mut() {
                ed.points.insert(id, (x, y));
            }
            self.relayout();
            return;
        }
        let rects = self.editor_rects();
        let pct_of = |t: &Table, c: CtrlId| t.get(c).map_or(placement::PCT_DEFAULT, |p| p.pct);
        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.insert(id, (x, y));
        // ★ The selection is STICKY: a miss does not deselect. The second finger of a pinch lands
        // on empty screen as its own `Started`; "miss = deselect" would make the pinch impossible.
        if let Some((cid, _)) = rects.iter().find(|(_, r)| r.contains(x, y))
            && Some(*cid) != ed.sel
        {
            (ed.sel, ed.drag, ed.pinch) = (Some(*cid), None, None);
        }
        let Some(sel) = ed.sel else { return };
        if ed.pinch.is_none() && ed.points.len() >= 2 {
            // Pair the new finger with the one driving the gesture (the dragging finger, else the
            // nearest) — not the two lowest ids, which are the oldest contacts.
            let other = ed.drag.map(|d| d.id).filter(|d| *d != id && ed.points.contains_key(d)).or_else(|| {
                ed.points
                    .iter()
                    .filter(|(k, _)| **k != id)
                    .min_by(|a, b| distance(*a.1, (x, y)).total_cmp(&distance(*b.1, (x, y))))
                    .map(|(k, _)| *k)
            });
            if let Some(a) = other {
                let dist0 = distance(ed.points[&a], (x, y));
                if dist0 > 1.0 {
                    ed.pinch = Some(Pinch {
                        a,
                        b: id,
                        dist0,
                        pct0: pct_of(&self.table, sel),
                    });
                    ed.drag = None;
                    return;
                }
            }
        }
        if ed.pinch.is_none() {
            let centre0 = self.centre_of(sel);
            if let Some(ed) = self.editor.as_mut() {
                ed.drag = Some(Drag { id, from: (x, y), centre0 });
            }
        }
    }

    fn editor_moved(&mut self, id: u64, x: f64, y: f64, _out: &mut Vec<Out>) {
        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.insert(id, (x, y));
        let Some(sel) = ed.sel else { return };
        if let Some(p) = ed.pinch {
            let (Some(&pa), Some(&pb)) = (ed.points.get(&p.a), ed.points.get(&p.b)) else {
                return;
            };
            // ABSOLUTE from the anchor: winit repeats `Moved` for every pointer, so an accumulated
            // delta would apply twice.
            let pct = ((f64::from(p.pct0) * distance(pa, pb) / p.dist0.max(1.0)) as u32).clamp(placement::PCT_MIN, placement::PCT_MAX);
            let (cx, cy) = self.centre_of(sel);
            self.editor_commit(sel, cx, cy, pct);
            return;
        }
        if let Some(d) = ed.drag
            && d.id == id
        {
            let pct = self.table.get(sel).map_or(placement::PCT_DEFAULT, |p| p.pct);
            self.editor_commit(sel, d.centre0.0 + (x - d.from.0), d.centre0.1 + (y - d.from.1), pct);
        }
    }

    fn editor_ended(&mut self, id: u64, out: &mut Vec<Out>) {
        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.remove(&id);
        if ed.drag.is_some_and(|d| d.id == id) {
            ed.drag = None;
        }
        if ed.pinch.is_some_and(|p| p.a == id || p.b == id) {
            ed.pinch = None;
            // One finger of the pinch is still down: it moves the control from here.
            let still: Vec<(u64, (f64, f64))> = ed.points.iter().map(|(k, v)| (*k, *v)).collect();
            if let (Some(sel), [(other, at)]) = (ed.sel, still.as_slice()) {
                let (other, at) = (*other, *at);
                let centre0 = self.centre_of(sel);
                if let Some(ed) = self.editor.as_mut() {
                    ed.drag = Some(Drag {
                        id: other,
                        from: at,
                        centre0,
                    });
                }
            }
        }
        // Persist on every finger-up: Android kills a backgrounded app without warning.
        out.push(Out::SaveLayout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s9() -> Hud {
        Hud::new(Screen::for_window(2220, 1080), Table::default())
    }

    fn centre(r: Rect) -> (f64, f64) {
        (f64::from(r.centre().0), f64::from(r.centre().1))
    }

    fn control(h: &Hud, k: KeyCode) -> (f64, f64) {
        centre(h.layout().controls.iter().find(|c| c.action == Action::Key(k)).unwrap().shape.bounds())
    }

    /// HZ-058: plate `n`'s centre.
    fn plate(h: &Hud, n: u8) -> (f64, f64) {
        centre(h.layout().controls.iter().find(|c| c.action == Action::Plate(n)).unwrap().shape.bounds())
    }

    fn keys(out: &[Out]) -> Vec<(KeyCode, bool)> {
        out.iter()
            .filter_map(|o| if let Out::Key { code, down } = o { Some((*code, *down)) } else { None })
            .collect()
    }

    /// The gear (a window's) opens the settings window.
    fn open_settings(h: &mut Hud) {
        let g = centre(h.layout().gear.unwrap());
        h.started(1, g.0, g.1);
        h.ended(1);
        assert!(h.settings_open());
    }

    /// Tap the settings row `a` — scrolled to first if it is past the fifth — and what came out: a
    /// row acts when the finger lifts (B2, hero2's scroll rule).
    fn tap_row(h: &mut Hud, id: u64, a: SettingsAction) -> Vec<Out> {
        let view = hud::settings_geom(h.screen()).view;
        let whole = |h: &Hud| h.settings_rows().into_iter().find(|(r, b, _)| *b == a && r.inside(&view)).map(|r| r.0);
        if whole(h).is_none() {
            let (x, y) = centre(view);
            h.started(id + 100, x, y);
            h.moved(id + 100, x, y - f64::from(view.h));
            h.ended(id + 100);
        }
        let (x, y) = centre(whole(h).unwrap_or_else(|| panic!("{a:?} is not listed: {:?}", h.settings_list())));
        assert!(h.started(id, x, y).is_empty(), "a row acts when the finger lifts");
        h.ended(id)
    }

    #[test]
    fn two_fingers_on_one_key_release_once() {
        let mut h = s9();
        let ok = control(&h, KeyCode::OK);
        assert_eq!(keys(&h.started(1, ok.0, ok.1)), vec![(KeyCode::OK, true)]);
        assert_eq!(keys(&h.started(2, ok.0 + 5.0, ok.1)), vec![], "no second keydown");
        assert_eq!(keys(&h.ended(1)), vec![], "the other finger still holds it");
        assert_eq!(keys(&h.ended(2)), vec![(KeyCode::OK, false)]);
    }

    #[test]
    fn a_resting_finger_is_not_a_new_press() {
        let mut h = s9();
        let ok = control(&h, KeyCode::OK);
        h.started(1, ok.0, ok.1);
        for _ in 0..5 {
            assert!(h.moved(1, ok.0, ok.1).is_empty(), "redelivered Moved");
        }
    }

    #[test]
    fn only_a_finger_that_started_on_a_control_may_slide_onto_one() {
        let mut h = s9();
        let st = centre(h.layout().stick.unwrap().seat);
        h.started(1, st.0, st.1);
        let one = plate(&h, 0);
        let out = h.moved(1, one.0, one.1);
        assert!(!out.contains(&Out::Plate(0)), "a steering thumb crossing plate 1 does not use it");
        // A finger that started on OK slides onto CLR (keys slide; a plate is a tap, HZ-058).
        let ok = control(&h, KeyCode::OK);
        h.started(2, ok.0, ok.1);
        let clr = centre(h.layout().controls.iter().find(|c| c.id == Some(CtrlId::Cancel)).unwrap().shape.bounds());
        let back = KeyCode::RIGHT_SOFT_KEY;
        assert_eq!(keys(&h.moved(2, clr.0, clr.1)), vec![(KeyCode::OK, false), (back, true)]);
        assert_eq!(keys(&h.ended(2)), vec![(back, false)]);
        // Nor does it slide onto a plate: the plates answer a touch that starts on them.
        h.started(3, ok.0, ok.1);
        let zero = plate(&h, 4);
        assert!(!h.moved(3, zero.0, zero.1).contains(&Out::Plate(4)));
        h.ended(3);
    }

    /// Codex review 2026-10-23: one tap of CLR in the field opened the menu and shut it again.
    #[test]
    fn a_finger_on_clr_keeps_its_key_when_the_scene_changes_under_it() {
        let mut h = s9();
        h.set_bar_off(true);
        let field = GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() };
        h.set_view(field);
        let clr = centre(h.layout().controls.iter().find(|c| c.id == Some(CtrlId::Cancel)).unwrap().shape.bounds());
        assert_eq!(keys(&h.started(1, clr.0, clr.1)), vec![(KeyCode::RIGHT_SOFT_KEY, true)], "the menu");
        // The game opened it: CLR now says "back".
        h.set_view(GuestView { scene: Scene::Menu, ..field });
        for _ in 0..3 {
            assert!(h.moved(1, clr.0 + 1.0, clr.1).is_empty(), "a resting finger is not a new press");
        }
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::RIGHT_SOFT_KEY, false)]);
        // The next tap, in the menu, is "back".
        assert_eq!(keys(&h.started(2, clr.0, clr.1)), vec![(KeyCode::RIGHT_SOFT_KEY, true)]);
        h.ended(2);
        // A finger that slides off and back on reads the seat again.
        let ok = control(&h, KeyCode::OK);
        h.started(3, ok.0, ok.1);
        assert_eq!(keys(&h.moved(3, clr.0, clr.1)), vec![(KeyCode::OK, false), (KeyCode::RIGHT_SOFT_KEY, true)]);
    }

    #[test]
    fn the_stick_floats_and_turns_old_up_then_new_down() {
        let mut h = s9();
        let st = h.layout().stick.unwrap();
        let (x, y) = centre(st.seat);
        assert!(h.started(1, x, y).is_empty(), "landing is neutral");
        assert_eq!(h.joy().unwrap().cx, x, "the centre is where the finger landed");
        assert_eq!(keys(&h.moved(1, x + st.engage_r, y)), vec![(KeyCode::RIGHT, true)]);
        assert_eq!(
            keys(&h.moved(1, x + 5.0, y + st.engage_r)),
            vec![(KeyCode::RIGHT, false), (KeyCode::DOWN, true)]
        );
        assert_eq!(h.joy().unwrap().cx, x, "the centre did not follow");
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::DOWN, false)]);
        assert!(h.joy().is_none());
    }

    /// HZ-059: in the bare field the stick walks eight ways — a diagonal holds both arrows, a turn
    /// between diagonals lifts one and presses the other, and the lift lets go of both.
    #[test]
    fn the_fields_stick_holds_a_diagonals_two_arrows() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        let st = h.layout().stick.unwrap();
        let (x, y) = centre(st.seat);
        let d = st.engage_r * std::f64::consts::FRAC_1_SQRT_2;
        h.started(1, x, y);
        assert_eq!(keys(&h.moved(1, x + d, y + d)), vec![(KeyCode::RIGHT, true), (KeyCode::DOWN, true)]);
        assert_eq!(h.held().len(), 2, "both arrows look pressed");
        assert_eq!(keys(&h.moved(1, x - d, y + d)), vec![(KeyCode::RIGHT, false), (KeyCode::LEFT, true)]);
        assert_eq!(keys(&h.moved(1, x, y + st.engage_r)), vec![(KeyCode::LEFT, false)], "↓ alone");
        h.moved(1, x - d, y + d);
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::LEFT, false), (KeyCode::DOWN, false)]);
        // A popup up: four ways, as a window's cursor.
        h.set_view(GuestView { scene: Scene::Field, popup: true, ..Default::default() });
        if let Some(st) = h.layout().stick {
            let (x, y) = centre(st.seat);
            h.started(2, x, y);
            assert_eq!(keys(&h.moved(2, x + d * 1.1, y + d)).len(), 1);
            h.ended(2);
        }
    }

    /// HZ-059 (십자키 「어 넣어」): on the field's D-pad the gap between two arms holds the diagonal;
    /// sliding onto an arm keeps that arm's arrow alone; a window's D-pad stays four arrows.
    #[test]
    fn the_fields_dpad_corner_holds_a_diagonal() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        h.set_control(ControlMode::Dpad);
        let sq = h.layout().dpad.unwrap();
        let (x, y) = centre(sq);
        let r = f64::from(sq.w.min(sq.h)) * 0.35;
        let (c, s) = (r * std::f64::consts::FRAC_1_SQRT_2, r * std::f64::consts::FRAC_1_SQRT_2);
        assert_eq!(keys(&h.started(1, x + c, y + s)), vec![(KeyCode::RIGHT, true), (KeyCode::DOWN, true)]);
        assert_eq!(keys(&h.moved(1, x + r, y + 2.0)), vec![(KeyCode::DOWN, false)], "onto the → arm");
        assert_eq!(keys(&h.moved(1, x - c, y - s)), vec![(KeyCode::RIGHT, false), (KeyCode::LEFT, true), (KeyCode::UP, true)]);
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::LEFT, false), (KeyCode::UP, false)]);
        // Two fingers on two arms are a diagonal too — two keys held; the worker makes it.
        h.started(2, x + r, y);
        assert_eq!(keys(&h.started(3, x, y + r)), vec![(KeyCode::DOWN, true)]);
        h.ended(2);
        h.ended(3);
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        let sq = h.layout().dpad.unwrap();
        let (x, y) = centre(sq);
        assert_eq!(keys(&h.started(4, x + c, y + s + 1.0)).len(), 1, "a window's cursor: one arrow");
        h.ended(4);
    }

    /// Holding OK in the field: one press, then after FIRE_REPEAT_DELAY a let-go-and-press every
    /// FIRE_REPEAT_EVERY until the finger lifts; never in a window.
    #[test]
    fn a_held_ok_attacks_again_in_the_field() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        let ok = control(&h, KeyCode::OK);
        let t0 = Instant::now();
        assert_eq!(keys(&h.started(1, ok.0, ok.1)), vec![(KeyCode::OK, true)]);
        let due = h.fire_next().expect("armed");
        assert!(h.fire_repeat_at(due - Duration::from_millis(1)).is_empty(), "not yet");
        assert_eq!(keys(&h.fire_repeat_at(due)), vec![(KeyCode::OK, false), (KeyCode::OK, true)]);
        assert!(h.fire_repeat_at(due + Duration::from_millis(1)).is_empty());
        assert_eq!(h.fire_next(), Some(due + FIRE_REPEAT_EVERY));
        // A long sleep: one repeat, then the next a period on.
        let late = due + FIRE_REPEAT_EVERY * 9;
        assert_eq!(h.fire_repeat_at(late).len(), 2);
        assert!(h.fire_repeat_at(late + Duration::from_millis(1)).is_empty());
        // A window: OK is "choose" there, never repeated.
        h.set_view(GuestView { scene: Scene::Menu, popup: false, notice: false, ..Default::default() });
        assert!(h.fire_repeat_at(late + FIRE_REPEAT_EVERY).is_empty());
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::OK, false)]);
        assert!(h.fire_next().is_none(), "lifted");
        assert!(t0.elapsed() < FIRE_REPEAT_DELAY * 20);
    }

    /// 조작 방식 고정: the centre is the seat's, wherever the thumb lands — a thumb landing right
    /// of it walks right at once.
    #[test]
    fn a_fixed_stick_is_centred_on_its_seat() {
        let mut h = s9();
        h.set_control(ControlMode::Fixed);
        let st = h.layout().stick.unwrap();
        let (x, y) = centre(st.seat);
        assert_eq!(keys(&h.started(1, x + st.engage_r * 1.5, y)), vec![(KeyCode::RIGHT, true)], "deflected on landing");
        assert_eq!(h.joy().unwrap().cx, x, "the seat's centre");
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::RIGHT, false)]);
        assert!(h.started(2, x, y).is_empty(), "on the centre: neutral");
        h.ended(2);
    }

    /// A window's arrows (hero2's): a finger holds its arrow, sliding onto the next turns (keyup
    /// then keydown), the dead centre between them is nothing — not a tap for the window either.
    #[test]
    fn a_windows_arrows_hold_slide_and_leave_the_centre_dead() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        let sq = h.layout().dpad.expect("a window has arrows");
        let (cx, cy) = centre(sq);
        let (up, left) = ((cx, f64::from(sq.y) + 20.0), (f64::from(sq.x) + 20.0, cy));
        assert_eq!(keys(&h.started(1, up.0, up.1)), vec![(KeyCode::UP, true)]);
        assert_eq!(keys(&h.moved(1, left.0, left.1)), vec![(KeyCode::UP, false), (KeyCode::LEFT, true)]);
        assert!(h.moved(1, cx, cy).is_empty(), "the dead centre keeps the arrow it has");
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::LEFT, false)]);
        assert!(h.started(2, cx, cy).is_empty(), "between the arrows: nothing at all");
        h.ended(2);
        // The picture elsewhere is the window's.
        assert!(matches!(h.started(3, 1110.0, 500.0).as_slice(), [Out::GuestTap(..)]));
    }

    /// A dialogue (no window up): no controls; the picture turns the page (a tap for the game) and
    /// the ▶▶ in the corner skips (RSK).
    #[test]
    fn a_dialogue_turns_its_page_and_the_mark_skips() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Script, ..Default::default() });
        assert!(h.layout().stick.is_none() && h.layout().gear.is_none());
        assert!(matches!(h.started(1, 300.0, 900.0).as_slice(), [Out::GuestTap(..)]), "where the stick was");
        h.ended(1);
        let (x, y) = (f64::from(h.screen().w) - 10.0, f64::from(h.screen().h) - 10.0);
        assert_eq!(keys(&h.started(2, x, y)), vec![(KeyCode::RIGHT_SOFT_KEY, true)]);
        assert_eq!(keys(&h.ended(2)), vec![(KeyCode::RIGHT_SOFT_KEY, false)]);
        // A choice over it is a window: the arrows are back.
        h.set_view(GuestView { scene: Scene::Script, window: true, ..Default::default() });
        assert!(h.layout().dpad.is_some());
    }

    /// User 2026-10-31: in portrait a dialogue keeps the strip's keys — the thumb need not reach up to the
    /// picture: OK turns the page, CLR skips (RSK, as the ▶▶), the arrows stay as a window's; the picture
    /// still turns the page and the ▶▶ still skips. The strip is the same through dialogue and choices.
    #[test]
    fn a_portrait_dialogue_keeps_the_strip() {
        let mut h = Hud::new(Screen::new(1080, 2220, 240, 320), Table::default());
        let strip_top = h.screen().game.bottom();
        h.set_view(GuestView { scene: Scene::Script, window: true, ..Default::default() });
        let choices = h.layout().clone();
        h.set_view(GuestView { scene: Scene::Script, ..Default::default() });
        let l = h.layout().clone();
        assert_eq!(l.dpad, choices.dpad, "the arrows where the choice window has them");
        assert!(l.dpad.is_some_and(|d| d.y >= strip_top) && l.stick.is_none());
        let (ox, oy) = control(&h, KeyCode::OK);
        assert!(oy > f64::from(strip_top));
        assert_eq!(keys(&h.started(1, ox, oy)), vec![(KeyCode::OK, true)], "OK turns the page");
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::OK, false)]);
        let clr = l.controls.iter().find(|c| c.label == "CLR").map(|c| centre(c.shape.bounds())).expect("CLR");
        assert!(clr.1 > f64::from(strip_top));
        assert_eq!(keys(&h.started(2, clr.0, clr.1)), vec![(KeyCode::RIGHT_SOFT_KEY, true)], "CLR skips");
        h.ended(2);
        let pic = h.screen().game;
        assert!(matches!(h.started(3, f64::from(pic.x + pic.w / 3), f64::from(pic.y + pic.h / 3)).as_slice(), [Out::GuestTap(..)]), "the picture turns the page");
        h.ended(3);
        let (sx, sy) = centre(hud::skip_hint(h.screen()));
        assert_eq!(keys(&h.started(4, sx, sy)), vec![(KeyCode::RIGHT_SOFT_KEY, true)], "the ▶▶ skips");
        h.ended(4);
        // The pad driving: no arrows, OK and CLR still there (as a window's).
        h.pad_input();
        assert!(h.layout().dpad.is_none() && h.layout().controls.iter().any(|c| c.action == Action::Key(KeyCode::OK)));
    }

    /// B6 (hero2's 「패드 버튼 설정」): the field's controls are the targets; a tap picks one and the
    /// next pad button is its button — a conflict swaps, a direction is no answer, OK / the menu /
    /// back never lose theirs — all in a draft: 취소 drops it, 저장 writes only the difference, and
    /// both go back to the settings window. A bare press names what the button does.
    #[test]
    fn the_pad_screen_picks_on_the_hud_swaps_and_saves_a_draft() {
        let mut h = s9();
        h.pad_input();
        open_settings(&mut h);
        assert!(tap_row(&mut h, 2, SettingsAction::PadRemap).is_empty());
        assert!(h.remap_open() && !h.settings_open() && !h.badges_shown());
        let sc = h.remap_scene();
        let at = |key: KeyCode| {
            let row = gamepad::remap_row(gamepad::PadAction::Key(key)).unwrap();
            centre(sc.targets.iter().find(|t| t.row == row).unwrap().rect)
        };
        let line = |h: &Hud, k: usize| h.remap_lines_at(Instant::now())[k].clone();
        // A bare press: what it does, its control lit blue.
        h.pad_button(gamepad::KEYCODE_BUTTON_X);
        assert_eq!(line(&h, 0), vec!["FACE LEFT (XBOX X)", "=", "QUICK SLOT 1"]);
        assert_eq!(h.remap().unwrap().probe, gamepad::remap_row(gamepad::PadAction::Key(KeyCode::NUM1)));
        // Pick quick slot 7 (its plate), press Y: 7 takes Y, 3 goes to L1. Nothing is written yet.
        let (x, y) = at(KeyCode::NUM7);
        assert!(h.started(3, x, y).is_empty(), "a tap here is never a game key");
        h.ended(3);
        let t0 = Instant::now();
        assert_eq!(h.remap().unwrap().sel, gamepad::remap_row(gamepad::PadAction::Key(KeyCode::NUM7)));
        assert_eq!(h.remap_lines_at(t0)[1], vec!["PRESS THE PAD BUTTON FOR IT", "(5S)"]);
        assert_eq!(h.remap_lines_at(t0 + Duration::from_millis(4200))[1], vec!["PRESS THE PAD BUTTON FOR IT", "(1S)"]);
        h.pad_button_at(19, t0);
        assert_eq!(line(&h, 0), vec!["DIRECTIONS CANNOT CHANGE"], "a direction: the prompt stays");
        h.pad_button_at(gamepad::KEYCODE_BUTTON_Y, t0);
        assert_eq!(line(&h, 0), vec!["QUICK SLOT 7 - PAGE BACK", "=", "FACE TOP (XBOX Y)"]);
        assert_eq!(line(&h, 1), vec!["QUICK SLOT 3", "=", "LEFT BUMPER (L1)"]);
        let draft = h.remap().unwrap().draft.clone();
        assert_eq!((gamepad::key_for(&draft, gamepad::KEYCODE_BUTTON_Y), gamepad::key_for(&draft, gamepad::KEYCODE_BUTTON_L1)), (Some(gamepad::PadAction::Key(KeyCode::NUM7)), Some(gamepad::PadAction::Key(KeyCode::NUM3))));
        assert!(h.overrides().is_empty() && h.remap_dirty() && h.remap_can_reset());
        // Back (a cell — no field control has it) onto A: OK takes back's B, a swap.
        let (x, y) = at(KeyCode::RIGHT_SOFT_KEY);
        h.started(4, x, y);
        h.ended(4);
        h.pad_button(gamepad::KEYCODE_BUTTON_A);
        assert_eq!(line(&h, 1), vec!["OK - ATTACK", "=", "FACE RIGHT (XBOX B)"]);
        // The prompt runs out on its own.
        let (x, y) = at(KeyCode::OK);
        h.started(5, x, y);
        h.ended(5);
        h.remap_tick(Instant::now() + Duration::from_secs(6));
        assert_eq!(line(&h, 0), vec!["TIME IS UP"]);
        assert_eq!(h.remap().unwrap().sel, None);
        // 취소: nothing kept, back to the settings window.
        let p = sc.panel.clone();
        let cancel = centre(p.cancel);
        assert!(h.started(6, cancel.0, cancel.1).is_empty());
        h.ended(6);
        assert!(!h.remap_open() && h.settings_open() && h.overrides().is_empty());
        // Again, swap 1 and 3 on X / Y, and 저장: the file holds the two lines that differ.
        tap_row(&mut h, 7, SettingsAction::PadRemap);
        let (x, y) = at(KeyCode::NUM1);
        h.started(8, x, y);
        h.ended(8);
        h.pad_button(gamepad::KEYCODE_BUTTON_Y);
        let save = centre(p.save);
        let out = h.started(9, save.0, save.1);
        h.ended(9);
        let [Out::Gamepad(ov)] = out.as_slice() else { panic!("{out:?}") };
        assert_eq!(ov.to_text().lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>(), vec!["99=3", "100=1"]);
        assert_eq!(h.overrides(), ov);
        assert!(!h.remap_open() && h.settings_open());
        assert_eq!(h.settings_words(SettingsAction::PadRemap, true).1, "CUSTOM");
        // 기본값 empties the draft; a tap on the picture lets go of a pick.
        tap_row(&mut h, 10, SettingsAction::PadRemap);
        let reset = centre(p.reset);
        h.started(11, reset.0, reset.1);
        h.ended(11);
        assert!(h.remap().unwrap().draft.is_empty() && !h.remap_can_reset() && h.remap_dirty());
        let (x, y) = at(KeyCode::NUM9);
        h.started(12, x, y);
        h.ended(12);
        h.started(13, 600.0, 900.0);
        h.ended(13);
        assert_eq!(h.remap().unwrap().sel, None);
        assert_eq!(line(&h, 0), vec!["TAP A CONTROL - THEN PRESS A PAD BUTTON"]);
        // Every line it can say is a host line with its Korean.
        for s in ["TIME IS UP", "TAP THE CONTROL AGAIN", "THEY WALK AND MOVE THE CURSOR", "TO CHANGE IT TAP A CONTROL FIRST", "NOTHING CHANGED", "WOULD HAVE NO BUTTON", "TRY ANOTHER PAD BUTTON", "SAVE TO APPLY", "BACK TO THE BUILT-IN LAYOUT", "(HIDDEN)", "PAD REMAP"]
            .into_iter()
            .chain(REMAP_SECONDS)
            .chain(remap_hint().concat())
        {
            assert!(crate::host_text::korean(s).is_some(), "{s}");
        }
    }

    #[test]
    fn the_gear_opens_settings_and_a_tap_outside_closes_them() {
        let mut h = s9();
        let g = centre(h.layout().gear.unwrap());
        h.started(1, g.0, g.1);
        h.ended(1);
        assert!(h.settings_open());
        let ok = control(&h, KeyCode::OK);
        assert!(h.started(2, ok.0, ok.1).is_empty(), "a tap outside the panel is not OK");
        assert!(!h.settings_open());
    }

    /// HZ-008: a scene change relays out; a finger already on a ring key that vanished keeps
    /// its key and releases it when it lifts, so the game never sees a key stuck down. (The menu
    /// keeps the ring since A7 — hero2's plates stay over its windows — so the title it is.)
    #[test]
    fn a_scene_change_hides_the_ring_but_releases_what_is_held() {
        use hz_java::Scene;
        let mut h = s9();
        let one = plate(&h, 0);
        assert_eq!(h.started(1, one.0, one.1), vec![Out::Plate(0)], "HZ-058: a plate is a tap — used once, nothing held");
        assert!(h.held().is_empty());
        h.ended(1);
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        assert!(h.layout().controls.iter().any(|c| c.action == Action::Plate(0)), "over the menu the plate stays (it takes a 「퀵슬롯등록」)");
        // A key held across the change is still released when its finger lifts.
        let ok = control(&h, KeyCode::OK);
        assert_eq!(keys(&h.started(4, ok.0, ok.1)), vec![(KeyCode::OK, true)]);
        let title = GuestView { scene: Scene::Title, popup: false, notice: false, ..Default::default() };
        assert!(h.set_view(title).is_empty(), "nothing sent on the change itself");
        assert_eq!(h.view(), title);
        assert!(h.layout().controls.iter().all(|c| !matches!(c.action, Action::Plate(_) | Action::Page)));
        assert!(h.held().contains(&KeyCode::OK), "still held until the finger lifts");
        assert_eq!(keys(&h.ended(4)), vec![(KeyCode::OK, false)]);
        // A new finger where plate 1 was uses no plate now (it is a tap for the title).
        assert!(!h.started(2, one.0, one.1).contains(&Out::Plate(0)));
        h.ended(2);
        // Back in the field the ring returns.
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert_eq!(h.started(3, one.0, one.1), vec![Out::Plate(0)]);
    }

    /// The HEAT SAVER row toggles the shown state and asks the caller to persist it; the panel
    /// stays open (the row's label now says what the next start will do).
    #[test]
    fn the_heat_saver_row_toggles_and_persists() {
        use hud::RowState::{Active, Normal};
        let mut h = s9();
        open_settings(&mut h);
        assert_eq!(h.settings_words(SettingsAction::HeatSaver, true), ("HEAT SAVER", "OFF", Normal));
        assert_eq!(tap_row(&mut h, 2, SettingsAction::HeatSaver), vec![Out::HeatSaver(true)]);
        assert!(h.heat_saver() && h.settings_open());
        assert_eq!(h.settings_words(SettingsAction::HeatSaver, true), ("HEAT SAVER", "ON", Active), "the row shows ON, lit");
        assert_eq!(tap_row(&mut h, 3, SettingsAction::HeatSaver), vec![Out::HeatSaver(false)]);
    }

    /// B2: a finger on the list that wanders past a tap scrolls it and acts on nothing; let go, the
    /// list settles on a whole row; a row it lands on acts only when it lifts, and lights as the
    /// game's cursor row meanwhile.
    #[test]
    fn the_settings_list_scrolls_under_a_finger_and_a_tap_acts_on_the_lift() {
        let mut h = s9();
        h.pad_input();
        open_settings(&mut h);
        let n = h.settings_list().len();
        assert_eq!(n, 7, "hero2's four, 상태 표시 with the game's bar, the pad's two: {:?}", h.settings_list());
        let g = hud::settings_geom(h.screen());
        let max = hud::settings_scroll_max(h.screen(), n);
        assert_eq!(max, 2 * g.pitch);
        let (x, y) = centre(h.settings_rows()[2].0);
        assert!(h.started(5, x, y).is_empty());
        assert_eq!(h.settings_pressed(), Some(SettingsAction::HeatSaver), "lit while held");
        assert!(h.moved(5, x, y - g.pitch as f64 * 0.8).is_empty());
        assert_eq!(h.settings_pressed(), None, "a drag, not a tap");
        assert_eq!(h.settings_scroll(), (g.pitch as f64 * 0.8) as i32);
        assert!(h.ended(5).is_empty(), "a drag acts on nothing");
        assert_eq!(h.settings_scroll(), g.pitch, "settled on a whole row");
        assert!(!h.heat_saver() && h.settings_open());
        // Past the end it stops; the system taking the finger acts on nothing either.
        h.started(6, x, y);
        h.moved(6, x, y - 5000.0);
        assert_eq!(h.settings_scroll(), max);
        h.ended(6);
        let (px, py) = centre(h.settings_rows().iter().find(|r| r.1 == SettingsAction::PadRemap).unwrap().0);
        h.started(7, px, py);
        assert!(h.cancelled(7).is_empty() && !h.remap_open());
        // 닫기 closes at once, on the press; the gear opens the list at its top again.
        let c = centre(hud::settings_close(h.screen()));
        h.started(8, c.0, c.1);
        assert!(!h.settings_open());
        h.ended(8);
        open_settings(&mut h);
        assert_eq!(h.settings_scroll(), 0);
    }

    /// The STATS row toggles the readout and asks the caller to persist it; the readout itself
    /// shows in play — the field and the game's windows over it (A7) — with the panel closed.
    #[test]
    fn the_stats_row_toggles_and_the_readout_is_for_play() {
        let mut h = s9();
        let stats = Stats { hp: 1, hp_max: 2, sp: 3, sp_max: 4, exp: 5, exp_next: 6, mp: 7, mp_max: 8, level: 9, ..Default::default() };
        h.set_stats(stats);
        h.set_quick(QuickBar::default());
        assert_eq!(h.stats_shown(), None, "the scene is unknown: not in play");
        assert!(h.quick_shown().is_none());
        for v in [
            GuestView { scene: Scene::Field, ..Default::default() },
            GuestView { scene: Scene::Field, popup: true, ..Default::default() },
            GuestView { scene: Scene::Menu, ..Default::default() },
            GuestView { scene: Scene::MapWindow, ..Default::default() },
        ] {
            h.set_view(v);
            assert_eq!(h.stats_shown(), Some(stats), "{v:?}");
            assert!(h.quick_shown().is_some(), "{v:?}: the plates show the slots");
        }
        for v in [
            GuestView { scene: Scene::Title, ..Default::default() },
            GuestView { scene: Scene::Script, window: true, ..Default::default() },
            GuestView { scene: Scene::Other(7), ..Default::default() },
        ] {
            h.set_view(v);
            assert_eq!(h.stats_shown(), None, "{v:?}");
            assert!(h.quick_shown().is_none(), "{v:?}");
        }
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert_eq!(h.stats_shown(), Some(stats));
        // The gear is a window's (the menu); the panel stays up back in the field.
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        let g = centre(h.layout().gear.unwrap());
        h.started(1, g.0, g.1);
        h.ended(1);
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        assert_eq!(h.stats_shown(), None, "the panel is up");
        assert_eq!(h.settings_words(SettingsAction::Stats, true), ("STATS", "ON", hud::RowState::Normal));
        assert_eq!(tap_row(&mut h, 2, SettingsAction::Stats), vec![Out::Stats(false)]);
        assert!(!h.stats_on());
        assert_eq!(h.settings_words(SettingsAction::Stats, true), ("STATS", "OFF", hud::RowState::Active));
        let cp = centre(hud::settings_close(h.screen()));
        h.started(3, cp.0, cp.1);
        h.ended(3);
        assert!(!h.settings_open());
        assert_eq!(h.stats_shown(), None, "off");
        assert!(crate::font::drawable(&stats.lines().0) && crate::font::drawable(&stats.lines().1));
    }

    /// Phase 6-4: with the game's bar gone the status block is the only HP reading — the STATS
    /// preference cannot hide it, and the settings window has no row for it (B2: hero2's rows).
    #[test]
    fn without_the_game_bar_stats_stay_on() {
        let mut h = s9();
        h.set_stats_on(false);
        h.set_bar_off(true);
        h.set_stats(Stats { hp: 1, hp_max: 2, ..Default::default() });
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert!(h.stats_shown().is_some(), "shown despite the preference");
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        open_settings(&mut h);
        let rows: Vec<_> = h.settings_list().into_iter().map(|r| r.0).collect();
        use SettingsAction as A;
        assert_eq!(rows, vec![A::LayoutEdit, A::Diag, A::HeatSaver, A::SfxLevel], "hero2's rows, no pad yet");
    }

    /// Phase 6-5: the panel appears with the first map in the field; a tap puts it away (no key
    /// reaches the game), the corner button brings it back; the target panel dodges only while it
    /// is up.
    #[test]
    fn the_minimap_comes_with_the_map_and_a_tap_puts_it_away() {
        let mut h = s9();
        let field = GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() };
        h.set_view(field);
        assert!(h.layout().minimap.is_none() && h.minimap_shown().is_none(), "no map read yet");
        assert_eq!(h.target_place().0, 0);
        // Map 44's real size, 30 × 50 tiles (a 3-tile toy map would be a room the panel moves into).
        let grid = MapGrid::new(44, 30, 50, &[0; 1500], &[]).unwrap();
        h.set_map(grid.clone());
        h.set_marks(MapMarks { player: Some((1.5, 0.5)), mobs: vec![(0.5, 1.5)] });
        let (_, marks, panel) = h.minimap_shown().expect("up by default");
        assert_eq!(marks.mobs.len(), 1);
        assert!(h.target_place().0 > 0, "the target panel clears it");
        assert!(h.started(1, centre(panel).0, centre(panel).1).is_empty(), "a tap acts when it lifts (A8)");
        assert!(h.minimap_shown().is_some());
        assert_eq!(h.ended(1), vec![Out::Minimap(false)], "the finger was the HUD's: no key");
        assert!(h.minimap_shown().is_none() && !h.minimap_on());
        assert_eq!(h.target_place().0, 0, "back where the game puts it");
        let button = h.minimap_restore_shown().expect("the way back");
        h.started(2, centre(button).0, centre(button).1);
        assert_eq!(h.ended(2), vec![Out::Minimap(true)]);
        assert!(h.minimap_shown().is_some());
        // Not the field: nothing to tap, nothing drawn.
        h.set_view(GuestView { scene: Scene::Menu, ..field });
        assert!(h.layout().minimap.is_none() && h.minimap_shown().is_none());
        // The preference seeded off at start.
        let mut h = s9();
        h.set_minimap_on(false);
        h.set_view(field);
        h.set_map(grid);
        assert!(h.minimap_shown().is_none() && h.minimap_restore_shown().is_some());
    }

    /// A8 (hero2's tap discipline): the minimap and the status block act on a TAP — down and up
    /// within [`TAP_SLOP`], still on the target — so a thumb crossing them does nothing, and a
    /// touch the system takes back never counts. The block is a target only while it is drawn, and
    /// only in the field's set (hero2's).
    #[test]
    fn the_minimap_and_the_status_block_act_on_a_tap() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        h.set_map(MapGrid::new(44, 30, 50, &[0; 1500], &[]).unwrap());
        let (px, py) = centre(h.minimap_shown().unwrap().2);
        h.started(1, px, py);
        h.moved(1, px + TAP_SLOP + 1.0, py);
        assert!(h.ended(1).is_empty() && h.minimap_on(), "dragged off: not a tap");
        h.started(2, px, py);
        assert!(h.cancelled(2).is_empty() && h.minimap_on(), "cancelled");
        h.started(3, px, py);
        h.moved(3, px + TAP_SLOP / 2.0, py);
        assert_eq!(h.ended(3), vec![Out::Minimap(false)], "a wobble is still a tap");
        let (sx, sy) = centre(hud::status_box(h.screen()));
        h.started(4, sx, sy);
        assert!(h.ended(4).is_empty(), "no numbers yet: no block drawn, no target");
        h.set_stats(Stats { hp: 1, hp_max: 1, level: 1, ..Default::default() });
        assert!(h.started(5, sx, sy).is_empty(), "nothing on the way down");
        assert_eq!(keys(&h.ended(5)), vec![(KeyCode::RIGHT_SOFT_KEY, true), (KeyCode::RIGHT_SOFT_KEY, false)], "the field's menu, CLR's key");
        h.started(6, sx, sy);
        h.moved(6, sx, sy + TAP_SLOP * 2.0);
        assert!(h.ended(6).is_empty() && h.joy().is_none(), "dragged off: no menu, and no walk either");
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        assert!(h.stats_shown().is_some() && h.layout().status.is_none(), "over a window: drawn (A7), no target");
    }

    /// A8 (hero2's pad HUD): while the pad drives, the stick and a window's arrows stand down —
    /// their targets too — while OK, CLR and the plates stay; the touch that brings the touch HUD
    /// back lands where none were, and they are back for the next. PAD BADGES OFF keeps them.
    #[test]
    fn the_walk_controls_stand_down_for_the_pad() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        let seat = h.layout().stick.unwrap().seat;
        h.pad_input();
        assert!(h.layout().stick.is_none() && h.layout().dpad.is_none());
        assert!(h.layout().controls.iter().any(|c| c.action == Action::Key(KeyCode::OK)), "OK stays");
        let (x, y) = centre(seat);
        assert!(h.started(1, x, y).is_empty() && h.joy().is_none(), "the waking touch walks nothing");
        h.ended(1);
        assert!(h.layout().stick.is_some(), "the stick is back");
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        h.pad_input();
        assert!(h.layout().dpad.is_none() && !h.layout().controls.iter().any(|c| matches!(c.shape, hud::Shape::Wedge { .. })), "a window's arrows too");
        h.set_pad_hud(PadHud::Off);
        assert!(h.layout().dpad.is_some(), "OFF: the touch HUD whatever drives");
    }

    /// 조작 방식 십자키 (A8 — hero3's 자유/십자키/고정, hero2's touch type B): the field walks with a
    /// window's four arrows on the stick's seat; a held arrow is the walk, the centre is dead.
    #[test]
    fn the_dpad_control_walks_the_field_with_arrows() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        let seat = h.layout().stick.unwrap().seat;
        h.set_control(ControlMode::Dpad);
        assert!(h.layout().stick.is_none());
        assert_eq!(h.layout().dpad, Some(seat), "the arrows on the stick's seat");
        let (x, y) = centre(seat);
        assert_eq!(keys(&h.started(1, x, f64::from(seat.y) + 10.0)), vec![(KeyCode::UP, true)]);
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::UP, false)]);
        assert!(h.started(2, x, y).is_empty() && h.joy().is_none(), "the dead centre");
        h.ended(2);
        h.set_control(ControlMode::Free);
        assert!(h.layout().stick.is_some() && h.layout().dpad.is_none());
    }

    #[test]
    fn release_all_lifts_everything_once() {
        let mut h = s9();
        let ok = control(&h, KeyCode::OK);
        h.started(1, ok.0, ok.1);
        let st = h.layout().stick.unwrap();
        let (x, y) = centre(st.seat);
        h.started(2, x, y);
        h.moved(2, x - st.engage_r, y);
        let mut up = keys(&h.release_all());
        up.sort_by_key(|(k, _)| format!("{k:?}"));
        assert_eq!(up.len(), 2);
        assert!(up.iter().all(|(_, d)| !d));
        assert!(h.release_all().is_empty());
    }

    fn open_editor(h: &mut Hud) {
        open_settings(h);
        let (_, a, on) = h.settings_rows()[0];
        assert!(a == SettingsAction::LayoutEdit && on);
        tap_row(h, 2, SettingsAction::LayoutEdit);
        assert!(h.editor().is_some());
    }

    #[test]
    fn editor_drag_pinch_and_cancel() {
        let mut h = s9();
        let ok_rect = placement::builtin(CtrlId::Ok, h.screen());
        let ok = centre(ok_rect);
        // A finger holding OK when the editor opens is released.
        h.started(9, ok.0, ok.1);
        open_editor(&mut h);
        assert!(!h.held().contains(&KeyCode::OK));
        let entry = h.table().clone();
        // Tap selects; a drag that starts OFF the control moves it by the delta (no teleport).
        h.started(3, ok.0, ok.1);
        h.ended(3);
        assert_eq!(h.editor().unwrap().sel, Some(CtrlId::Ok));
        assert_eq!(keys(&h.started(4, 1100.0, 600.0)), vec![]);
        h.moved(4, 1000.0, 600.0);
        let out = h.ended(4);
        assert!(out.contains(&Out::SaveLayout), "persisted on finger-up");
        let moved = h.table().get(CtrlId::Ok).unwrap();
        assert!((f64::from(moved.cx) * 2220.0 - (ok.0 - 100.0)).abs() < 2.0);
        assert_eq!(moved.pct, 100, "a drag never changes the size");
        // Pinch: a second finger anywhere, distance x1.4 -> 140 %, even with Moved repeated.
        h.started(5, 800.0, 500.0);
        h.started(6, 900.0, 500.0);
        for _ in 0..3 {
            h.moved(5, 780.0, 500.0);
            h.moved(6, 920.0, 500.0);
        }
        assert_eq!(h.table().get(CtrlId::Ok).unwrap().pct, 140);
        // Past the largest size: held there.
        h.moved(6, 1100.0, 500.0);
        assert_eq!(h.table().get(CtrlId::Ok).unwrap().pct, placement::PCT_MAX);
        h.ended(5);
        h.ended(6);
        // CANCEL restores the snapshot and closes.
        let cancel = hud::editor_chrome(h.screen())
            .into_iter()
            .find(|(_, a)| *a == EditAction::Cancel)
            .unwrap()
            .0;
        let c = centre(cancel);
        h.started(7, c.0, c.1);
        assert!(h.editor().is_none());
        assert_eq!(h.table(), &entry);
    }

    /// A9 (hero2's row): PAD previews the badges and changes nothing; HIDE switches the selected plate
    /// off — gone from the editor, the field and the windows over play, but `#` still closes the
    /// 「# 확인」 notice — and never the last one; SHOW brings them back; CANCEL restores them too.
    #[test]
    fn editor_pad_hide_show_and_cancel() {
        let mut h = s9();
        open_editor(&mut h);
        let button = |h: &Hud, a: EditAction| centre(hud::editor_chrome(h.screen()).into_iter().find(|(_, x)| *x == a).unwrap().0);
        let press = |h: &mut Hud, id: u64, a: EditAction| {
            let (x, y) = button(h, a);
            let out = h.started(id, x, y);
            h.ended(id);
            out
        };
        let entry = h.table().clone();
        // PAD: a preview of the in-play badges — OK's bottom face button, plate 7's L1.
        assert!(press(&mut h, 1, EditAction::Pad).is_empty() && h.editor().unwrap().pad);
        assert_eq!(h.editor_pad_pos(CtrlId::Ok), Some(PadPos::Face(padhud::FacePos::Down)));
        assert_eq!(h.editor_pad_pos(CtrlId::Ring(1)), Some(PadPos::Shoulder { right: false, lower: false }));
        assert_eq!(h.editor_pad_pos(CtrlId::Cancel), Some(PadPos::Face(padhud::FacePos::Right)), "B = RSK, the field's menu");
        assert_eq!(h.editor_pad_pos(CtrlId::Page), Some(PadPos::Middle { right: true }), "START = the plates' page");
        assert_eq!((h.editor_pad_pos(CtrlId::Stick), h.editor_pad_pos(CtrlId::Minimap)), (None, None));
        assert_eq!(h.table(), &entry);
        // HIDE with nothing selected, then with OK: dim, says so, nothing changes.
        assert!(!h.edit_enabled(EditAction::Hide) && !h.edit_enabled(EditAction::Show));
        press(&mut h, 2, EditAction::Hide);
        assert_eq!(h.toast(), Some("SELECT A QUICK SLOT TO HIDE"));
        // Select plate 7 and hide it.
        let seven = centre(placement::builtin(CtrlId::Ring(1), h.screen()));
        h.started(3, seven.0, seven.1);
        h.ended(3);
        assert!(h.edit_enabled(EditAction::Hide));
        assert!(press(&mut h, 4, EditAction::Hide).contains(&Out::SaveLayout));
        assert!(h.table().is_hidden(CtrlId::Ring(1)) && h.editor().unwrap().sel.is_none());
        assert!(!h.editor_rects().iter().any(|(c, _)| *c == CtrlId::Ring(1)), "not offered while off");
        assert!(h.edit_enabled(EditAction::Show));
        // SAVE: the field has no 7 plate now; the menu over play neither.
        press(&mut h, 5, EditAction::Save);
        assert!(h.editor().is_none());
        let has = |h: &Hud, k: KeyCode| h.layout().controls.iter().any(|c| c.action.pad_key() == Some(k));
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        assert!(!has(&h, KeyCode::NUM7) && has(&h, KeyCode::NUM9));
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        assert!(!has(&h, KeyCode::NUM7));
        // `#`'s plate hidden too: gone in the field — and the 「# 확인」 notice is OK's (HZ-058).
        let mut t = h.table().clone();
        assert!(t.hide(CtrlId::Ring(5)));
        let mut h = Hud::new(Screen::for_window(2220, 1080), t);
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        assert!(!has(&h, KeyCode::HASH));
        h.set_view(GuestView { scene: Scene::Field, notice: true, ..Default::default() });
        assert!(has(&h, KeyCode::HASH), "the notice still closes: OK sends #");
        // The last plate up cannot go; SHOW brings all back; CANCEL puts the entry's back.
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        open_editor(&mut h);
        let entry = h.table().clone();
        for i in [0u8, 2, 3, 4, 5] {
            let p = centre(placement::builtin(CtrlId::Ring(i), h.screen()));
            h.started(10 + u64::from(i), p.0, p.1);
            h.ended(10 + u64::from(i));
            press(&mut h, 20 + u64::from(i), EditAction::Hide);
        }
        let zero = centre(placement::builtin(CtrlId::Ring(6), h.screen()));
        h.started(30, zero.0, zero.1);
        h.ended(30);
        assert!(!h.edit_enabled(EditAction::Hide), "the last plate stays");
        press(&mut h, 31, EditAction::Hide);
        assert_eq!(h.toast(), Some("KEEP ONE QUICK SLOT"));
        assert!(!h.table().is_hidden(CtrlId::Ring(6)));
        assert!(press(&mut h, 32, EditAction::Show).contains(&Out::SaveLayout));
        assert!(!h.table().any_hidden());
        press(&mut h, 33, EditAction::Show);
        assert_eq!(h.toast(), Some("NO QUICK SLOT IS HIDDEN"));
        press(&mut h, 34, EditAction::Cancel);
        assert_eq!(h.table(), &entry, "CANCEL: 7 and # off again, as on entry");
    }

    #[test]
    fn editor_selection_is_sticky_and_obstacles_refuse() {
        let mut h = s9();
        open_editor(&mut h);
        let ok = centre(placement::builtin(CtrlId::Ok, h.screen()));
        h.started(3, ok.0, ok.1);
        h.ended(3);
        h.started(4, 50.0, 50.0); // empty screen
        h.ended(4);
        assert_eq!(h.editor().unwrap().sel, Some(CtrlId::Ok), "a miss does not deselect");
        // Dragging OK onto the gear is refused: it stays where it last legally was.
        let g = centre(hud::gear(h.screen()));
        h.started(5, ok.0, ok.1);
        h.moved(5, g.0, g.1);
        h.ended(5);
        let r = h.editor_rects().into_iter().find(|(c, _)| *c == CtrlId::Ok).unwrap().1;
        assert!(!r.grown(h.screen().pad()).intersects(&hud::gear(h.screen())), "{r:?}");
    }

    /// Portrait (2026-10-29): the strip walks, steers and is edited as landscape is — the stick per
    /// 조작 방식 (자유 where the thumb lands, 고정 its seat, 십자키 a window's arrows), from the strip
    /// only; the quick plates, OK, CLR, SAVE and MAP there; the layout editor open; and the target
    /// panel over the minimap, at the game's own height.
    #[test]
    fn portrait_walks_and_edits_as_landscape_does() {
        let mut h = Hud::new(Screen::new(1080, 2220, 240, 320), Table::default());
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        let strip_top = h.screen().game.bottom();
        let st = h.layout().stick.expect("the field's stick");
        assert!(st.seat.y >= strip_top && st.zone.y >= strip_top, "in the strip: {st:?}");
        let (x, y) = centre(st.seat);
        assert!(h.started(1, x, y).is_empty() && h.joy().unwrap().cx == x, "자유: the centre where it lands");
        assert_eq!(keys(&h.moved(1, x + st.engage_r, y)), vec![(KeyCode::RIGHT, true)]);
        assert_eq!(keys(&h.ended(1)), vec![(KeyCode::RIGHT, false)]);
        assert!(h.started(9, x, f64::from(strip_top) - 30.0).is_empty() && h.joy().is_none(), "the picture is not the stick's");
        h.ended(9);
        h.set_control(ControlMode::Fixed);
        assert_eq!(keys(&h.started(2, x + st.engage_r * 1.5, y)), vec![(KeyCode::RIGHT, true)], "고정: deflected on landing");
        h.ended(2);
        h.set_control(ControlMode::Dpad);
        assert!(h.layout().stick.is_none() && h.layout().dpad == Some(st.seat), "십자키: the arrows on the stick's seat");
        assert_eq!(keys(&h.started(3, x, f64::from(st.seat.y) + 10.0)), vec![(KeyCode::UP, true)]);
        h.ended(3);
        h.set_control(ControlMode::Free);
        // The plates, OK, CLR in the strip, and the tabs with them.
        for k in [KeyCode::OK, KeyCode::RIGHT_SOFT_KEY] {
            let (_, cy) = control(&h, k);
            assert!(cy > f64::from(strip_top), "{k:?} in the strip");
        }
        for c in h.layout().controls.iter().filter(|c| matches!(c.action, Action::Plate(_) | Action::Page)) {
            assert!(c.shape.bounds().y > strip_top, "{:?} in the strip", c.action);
        }
        let tabs: Vec<hud::Tab> = h.layout().tabs.iter().map(|(_, t)| *t).collect();
        assert_eq!(tabs, vec![hud::Tab::Save, hud::Tab::Map]);
        let save = h.tab(hud::Tab::Save).unwrap();
        assert!(save.y >= strip_top);
        assert_eq!(h.started(4, centre(save).0, centre(save).1), vec![Out::QuickSave]);
        h.ended(4);
        // The editor opens here too, and a plate moves.
        open_settings(&mut h);
        let (_, a, on) = h.settings_rows()[0];
        assert_eq!(h.settings_words(a, on), ("LAYOUT", "EDIT", hud::RowState::Normal));
        open_editor(&mut h);
        let plate = h.editor_rects().into_iter().find(|(c, _)| *c == CtrlId::Ring(3)).unwrap().1;
        let (px, py) = centre(plate);
        h.started(5, px, py);
        h.moved(5, px - 40.0, py);
        let out = h.ended(5);
        assert!(out.contains(&Out::SaveLayout));
        let moved = h.editor_rects().into_iter().find(|(c, _)| *c == CtrlId::Ring(3)).unwrap().1;
        assert_eq!(moved.x, plate.x - 40, "{plate:?} -> {moved:?}");
        // The minimap's map: the target panel over it, at the game's own height.
        h.set_map(MapGrid { fights: Some(true), ..MapGrid::new(44, 50, 40, &vec![0i8; 50 * 40], &[]).unwrap() });
        let (shift, drop) = h.target_place();
        let over = hud::target_box_at(h.screen(), shift, drop, h.screen().guest_w);
        assert!(drop == 0 && over.bottom() <= h.layout().panel.y, "{over:?} over {:?}", h.layout().panel);
    }

    // ------------------------------------------------------------ gamepad badges ----

    #[test]
    fn badges_follow_the_last_input_source_in_auto() {
        let mut h = s9();
        assert!(!h.badges_shown() && h.badges().is_empty(), "no pad seen yet");
        h.pad_input();
        assert!(h.badges_shown());
        let badges = h.badges();
        // Every plate (the quick functions 1 3 7 9 0 # and R3's `*`), the page button (START), OK and
        // CLR (B) have a built-in button; the stick gets none.
        assert_eq!(badges.len(), h.layout().controls.iter().filter(|c| matches!(c.action, Action::Key(_) | Action::Plate(_) | Action::Page)).count());
        assert!(badges.iter().any(|(_, p)| *p == PadPos::Face(padhud::FacePos::Down)), "OK = bottom face button");
        let s = *h.screen();
        for (b, _) in &badges {
            assert!(b.cx - b.r >= 0.0 && b.cy + b.r <= f64::from(s.h), "inside the window: {b:?}");
        }
        // A touch anywhere is the last input source again.
        let ok = control(&h, KeyCode::OK);
        h.started(1, ok.0, ok.1);
        h.ended(1);
        assert!(!h.badges_shown());
        h.set_pad_hud(PadHud::On);
        assert!(h.badges_shown(), "ON: always");
        h.set_pad_hud(PadHud::Off);
        h.pad_input();
        assert!(!h.badges_shown(), "OFF: never");
    }

    #[test]
    fn a_remap_moves_the_badge_and_a_switched_off_button_removes_it() {
        let mut h = s9();
        let mut ov = Overrides::default();
        ov.set(gamepad::KEYCODE_BUTTON_THUMBR, Some(gamepad::PadAction::Key(KeyCode::NUM7)));
        ov.set(gamepad::KEYCODE_BUTTON_A, None);
        h.set_overrides(ov);
        h.set_pad_hud(PadHud::On);
        let badge_of = |h: &Hud, k: KeyCode| {
            let bounds = h.layout().controls.iter().find(|c| c.action.pad_key() == Some(k)).unwrap().shape.bounds();
            let s = *h.screen();
            let want = padhud::badge_at(bounds, s.w, s.h).unwrap();
            h.badges().into_iter().find(|(b, _)| *b == want).map(|(_, p)| p)
        };
        assert_eq!(badge_of(&h, KeyCode::NUM7), Some(PadPos::StickPress { right: true }), "7 now shows R3, not L1");
        assert_eq!(badge_of(&h, KeyCode::OK), None, "nothing fires OK: no badge");
        assert_eq!(badge_of(&h, KeyCode::NUM1), Some(PadPos::Face(padhud::FacePos::Left)), "untouched");
    }

    /// The SAVE and MAP tabs to a pad (user 2026-10-27, hero2's 빠른저장 on L3): L3 asks for the save
    /// wherever no host window is up (the worker says 「SAVE - NOT NOW」 off the field), SELECT opens
    /// the world map where its tab hangs and closes it again; both tabs carry their button's badge.
    #[test]
    fn the_pad_saves_and_opens_the_world_map_as_the_tabs_do() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        h.set_map(MapGrid::new(44, 3, 3, &[0i8; 9], &[]).unwrap());
        h.set_pad_hud(PadHud::On);
        let badge = |h: &Hud, t: hud::Tab| {
            let s = *h.screen();
            let want = padhud::badge_at(h.tab(t).unwrap(), s.w, s.h).unwrap();
            h.badges().into_iter().find(|(b, _)| *b == want).map(|(_, p)| p)
        };
        assert_eq!(badge(&h, hud::Tab::Save), Some(PadPos::StickPress { right: false }), "L3");
        assert_eq!(badge(&h, hud::Tab::Map), Some(PadPos::Middle { right: false }), "SELECT");
        assert_eq!(h.pad_tab(hud::Tab::Save), vec![Out::QuickSave]);
        assert_eq!(h.pad_tab(hud::Tab::Map), vec![Out::World(true)]);
        assert!(h.world().is_some());
        assert_eq!(h.pad_tab(hud::Tab::Map), vec![Out::World(false)], "again: closed");
        h.pad_tab(hud::Tab::Map);
        assert_eq!(h.pad_tab(hud::Tab::Save), vec![Out::World(false), Out::QuickSave], "over the map: closed, then asked");
        // A trigger given the map works the same, over the map too.
        let mut ov = Overrides::default();
        ov.set(gamepad::KEYCODE_BUTTON_R2, Some(gamepad::PadAction::Map));
        let mut pad = gamepad::Pad::new(&ov);
        assert_eq!(h.world_pad(0.0, 0.0, &pad.triggers(0.0, 0.9)), vec![Out::World(true)]);
        assert!(h.world_pad(0.0, 0.0, &pad.triggers(0.0, 0.0)).is_empty(), "its release");
        assert_eq!(h.world_pad(0.0, 0.0, &pad.triggers(0.0, 0.9)), vec![Out::World(false)]);
        // A window over the field: no MAP tab, so nothing; SAVE still asks (the worker refuses).
        h.set_view(GuestView { scene: Scene::Menu, ..Default::default() });
        assert!(h.pad_tab(hud::Tab::Map).is_empty() && h.world().is_none());
        assert_eq!(h.pad_tab(hud::Tab::Save), vec![Out::QuickSave]);
        // A host window up: neither.
        open_settings(&mut h);
        assert!(h.pad_tab(hud::Tab::Save).is_empty() && h.pad_tab(hud::Tab::Map).is_empty());
    }

    /// hero2's pad rows come with a pad (and stay while either is off its default).
    #[test]
    fn the_pad_badges_row_cycles_and_is_persisted() {
        use hud::RowState::{Active, Normal};
        let mut h = s9();
        assert!(h.settings_list().iter().all(|(a, _)| *a != SettingsAction::PadHud), "no pad yet: no pad rows");
        h.pad_input();
        open_settings(&mut h);
        assert_eq!(h.settings_words(SettingsAction::PadHud, true), ("PAD BADGES", "AUTO", Normal));
        assert_eq!(tap_row(&mut h, 2, SettingsAction::PadHud), vec![Out::PadHud(PadHud::On)]);
        assert_eq!(h.settings_words(SettingsAction::PadHud, true), ("PAD BADGES", "ALWAYS", Active));
        assert_eq!(tap_row(&mut h, 3, SettingsAction::PadHud), vec![Out::PadHud(PadHud::Off)]);
        assert_eq!(h.settings_words(SettingsAction::PadHud, true), ("PAD BADGES", "OFF", Active));
        assert!(h.badges().is_empty(), "no badges while the panel is open anyway");
        let mut fresh = s9();
        fresh.set_pad_hud(PadHud::Off);
        assert!(fresh.settings_list().iter().any(|(a, _)| *a == SettingsAction::PadHud), "off its default: the row stays");
    }

    #[test]
    fn a_notice_shows_for_a_while_then_goes_and_the_editor_posts_them() {
        let mut h = s9();
        let t0 = Instant::now();
        assert_eq!(h.toast_at(t0), None);
        h.notice_at("LAYOUT SAVED", t0);
        assert_eq!(h.toast_at(t0), Some("LAYOUT SAVED"));
        assert_eq!(h.toast_at(t0 + Duration::from_millis(TOAST_MS - 1)), Some("LAYOUT SAVED"));
        assert_eq!(h.toast_at(t0 + Duration::from_millis(TOAST_MS)), None);
        assert_eq!(h.toast_until(), Some(t0 + Duration::from_millis(NOTICE_SLIDE_MS)), "first the slide ends");
        h.expire_notices(t0 + Duration::from_millis(NOTICE_SLIDE_MS));
        assert_eq!(h.toast_until(), Some(t0 + Duration::from_millis(TOAST_MS)), "then the notice goes");
        h.expire_notices(t0 + Duration::from_millis(TOAST_MS));
        assert_eq!(h.toast_until(), None);
        assert_eq!(h.toast_at(t0), None, "gone for good");
        // SAVE in the editor posts one; the first pad input posts one; all drawable.
        open_editor(&mut h);
        let save = hud::editor_chrome(h.screen()).into_iter().find(|(_, a)| *a == EditAction::Save).unwrap().0;
        let sv = centre(save);
        h.started(3, sv.0, sv.1);
        h.ended(3);
        assert_eq!(h.toast(), Some("LAYOUT SAVED"));
        h.pad_input();
        assert!(h.toast().unwrap().starts_with("GAMEPAD"));
        assert!(crate::font::drawable(h.toast().unwrap()));
        h.pad_input();
        assert!(h.toast().unwrap().starts_with("GAMEPAD"), "a later pad input does not repeat the notice but does not clear it either");
    }

    /// B5: the notices stack (hero2's) — the newest lowest, the older sliding up a slot, the same
    /// line again moving to the bottom with a new clock rather than showing twice, each going on its
    /// own clock; a notice held over the title is there while the title is.
    #[test]
    fn notices_stack_newest_lowest_and_go_on_their_own_clocks() {
        let mut h = s9();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let lines = |v: Vec<(Notice, f32)>| -> Vec<(&'static str, f32)> {
            v.into_iter()
                .map(|(n, slot)| match n {
                    Notice::Line(t) => (t, slot),
                    Notice::Picture(_) => ("<PICTURE>", slot),
                })
                .collect()
        };
        h.notice_at("LAYOUT SAVED", t0);
        h.notice_at("SAVE - NOT NOW", ms(1000));
        assert_eq!(lines(h.notices_at(ms(1000))), vec![("LAYOUT SAVED", 0.0), ("SAVE - NOT NOW", 0.0)], "the older starts a slot down…");
        assert_eq!(lines(h.notices_at(ms(1070))), vec![("LAYOUT SAVED", 0.5), ("SAVE - NOT NOW", 0.0)]);
        assert_eq!(lines(h.notices_at(ms(1200))), vec![("LAYOUT SAVED", 1.0), ("SAVE - NOT NOW", 0.0)], "…and slides up");
        h.notice_at("LAYOUT SAVED", ms(1500));
        assert_eq!(lines(h.notices_at(ms(2000))), vec![("SAVE - NOT NOW", 1.0), ("LAYOUT SAVED", 0.0)], "once, at the bottom");
        assert_eq!(h.toast_at(ms(2000)), Some("LAYOUT SAVED"), "the newest is the hint lines'");
        assert_eq!(lines(h.notices_at(ms(3600))), vec![("LAYOUT SAVED", 0.0)]);
        assert!(h.notices_at(ms(4000)).is_empty());
        for i in 0..NOTICE_MAX + 3 {
            h.notice_at(["LAYOUT SAVED", "SAVE - NOT NOW", "LAYOUT RESTORED", "SAVE READ ERROR - THE FILE IS KEPT"][i % 4], ms(5000 + i as u64));
        }
        assert_eq!(h.notices_at(ms(5100)).len(), 4, "a line is up once");
        h.set_view(GuestView { scene: Scene::Title, ..Default::default() });
        h.notice_through_title("SAVE 1 IS GONE - THE VAULT HAS IT - GEAR > SAVES");
        let later = Instant::now() + Duration::from_millis(TOAST_MS * 4);
        assert_eq!(lines(h.notices_at(later)), vec![("SAVE 1 IS GONE - THE VAULT HAS IT - GEAR > SAVES", 0.0)], "held over the title");
        // HZ-025: the game's pickup pictures join the stack, each its own, newest lowest; the hint
        // line is still the newest LINE.
        let mut h = s9();
        let picture = |n: u32| Icon { key: hz_java::IconKey::Pickup(n), w: 120, h: 16, px: vec![0xff00_0000; 120 * 16].into() };
        h.notice_at("LAYOUT SAVED", t0);
        h.pickup_at(picture(0), ms(10));
        h.pickup_at(picture(1), ms(20));
        assert_eq!(lines(h.notices_at(ms(500))), vec![("LAYOUT SAVED", 2.0), ("<PICTURE>", 1.0), ("<PICTURE>", 0.0)]);
        assert_eq!(h.notices_at(ms(500))[2].0, Notice::Picture(picture(1)), "the newest at the bottom");
        assert_eq!(h.toast_at(ms(500)), Some("LAYOUT SAVED"));
        assert_eq!(h.notices_at(ms(2505)).len(), 2, "the line goes on its own clock");
        assert!(h.notices_at(ms(2600)).is_empty());
    }

    /// HZ-061: the boss story's countdown is one line held in the stack — the next second replaces
    /// it in place (no slide, no second line), it outlives the toast clock (the wait stands still
    /// under the menu), a pickup stacks under it, the bound never drops it, and `None` takes it away.
    #[test]
    fn the_story_countdown_is_one_held_line_replaced_in_place() {
        let mut h = s9();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let lines = |v: Vec<(Notice, f32)>| -> Vec<(&'static str, f32)> {
            v.into_iter()
                .map(|(n, slot)| match n {
                    Notice::Line(t) => (t, slot),
                    Notice::Picture(_) => ("<PICTURE>", slot),
                })
                .collect()
        };
        let wait = |s: u8| crate::host_text::story_wait(s);
        h.countdown_at(wait(5), t0);
        assert_eq!(lines(h.notices_at(ms(500))), vec![("STORY IN 5", 0.0)]);
        h.countdown_at(wait(4), ms(1000));
        assert_eq!(lines(h.notices_at(ms(1000))), vec![("STORY IN 4", 0.0)], "replaced in place");
        assert_eq!(lines(h.notices_at(ms(1000 + TOAST_MS * 4))), vec![("STORY IN 4", 0.0)], "held, no clock");
        assert_eq!(h.toast_until(), Some(t0 + Duration::from_millis(NOTICE_SLIDE_MS)), "no clock of its own: only its first slide");
        let picture = Icon { key: hz_java::IconKey::Pickup(7), w: 120, h: 16, px: vec![0xff00_0000; 120 * 16].into() };
        h.pickup_at(picture, ms(1500));
        h.countdown_at(wait(3), ms(2000));
        assert_eq!(lines(h.notices_at(ms(2000))), vec![("STORY IN 3", 1.0), ("<PICTURE>", 0.0)], "the pickup under it, the number in its row");
        for i in 0..NOTICE_MAX + 2 {
            h.notice_at(["LAYOUT SAVED", "SAVE - NOT NOW", "LAYOUT RESTORED"][i % 3], ms(2100 + i as u64));
            h.pickup_at(Icon { key: hz_java::IconKey::Pickup(10 + i as u32), w: 120, h: 16, px: vec![0; 120 * 16].into() }, ms(2100 + i as u64));
        }
        assert!(lines(h.notices_at(ms(2500))).iter().any(|(t, _)| *t == "STORY IN 3"), "the bound drops the oldest other notice");
        h.expire_notices(ms(2100 + TOAST_MS * 2));
        assert_eq!(lines(h.notices_at(ms(2100 + TOAST_MS * 2))), vec![("STORY IN 3", 0.0)], "only the countdown outlives the clock");
        h.countdown_at(None, ms(9000));
        assert!(h.notices_at(ms(9000)).is_empty(), "taken away when the story starts");
        assert_eq!(wait(0), None);
        assert_eq!(wait(hz_java::story::WAIT_SECONDS + 1), None);
    }

    /// The world map by keys: an arrow walks a road; from 실바나르 남부 — five roads, four arrows —
    /// 동부 is the keypad's 3 or up and right pressed together (either order, a D-pad's corner);
    /// two arrows too far apart, or the first let go, are two arrows.
    #[test]
    fn the_world_maps_arrows_walk_the_cursor() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        h.set_map(MapGrid::new(44, 3, 3, &[0i8; 9], &[]).unwrap());
        assert!(h.world_arrows().is_empty(), "only over the open map");
        let tab = centre(h.tab(hud::Tab::Map).unwrap());
        h.started(1, tab.0, tab.1);
        h.ended(1);
        let arrows = h.world_arrows();
        assert_eq!(arrows.len(), 4, "a window's four, on the walk control's seat");
        let up = centre(arrows[0].shape.bounds());
        let up = (up.0, f64::from(arrows[0].shape.bounds().y) + 20.0);
        assert!(h.started(2, up.0, up.1).is_empty(), "nothing reaches the game");
        assert_eq!(h.world().unwrap().cursor, wm::WM_SILVA, "up: 실바나르");
        assert_eq!(h.world_held(), HashSet::from([KeyCode::UP]));
        h.ended(2);
        assert!(h.world_held().is_empty());
        let (cx, cy) = centre(arrows[0].shape.bounds());
        h.started(3, cx, cy);
        h.ended(3);
        assert!(h.world().is_some(), "the dead centre does not close the map");
        assert_eq!(h.world().unwrap().cursor, wm::WM_SILVA);
    }

    #[test]
    fn the_world_map_reaches_every_road_by_keys() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        h.set_map(MapGrid::new(44, 3, 3, &[0i8; 9], &[]).unwrap());
        let tab = centre(h.tab(hud::Tab::Map).expect("MAP in the field"));
        let out = h.started(1, tab.0, tab.1);
        h.ended(1);
        assert!(out.contains(&Out::World(true)), "{out:?}");
        let at = |h: &Hud| h.world().unwrap().cursor;
        assert_eq!(at(&h), wm::WM_SILVA_S, "the player's place");
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let tap = |h: &mut Hud, k: KeyCode, t: u64| {
            h.world_key_at(k, ms(t));
            h.world_key_up(k);
        };
        tap(&mut h, KeyCode::UP, 0);
        assert_eq!(at(&h), wm::WM_SILVA, "up alone: 실바나르");
        tap(&mut h, KeyCode::DOWN, 1000);
        h.world_key_at(KeyCode::NUM3, ms(2000));
        assert_eq!(at(&h), wm::WM_SILVA_E, "the keypad's 3: up-right");
        tap(&mut h, KeyCode::LEFT, 3000);
        assert_eq!(at(&h), wm::WM_SILVA_S);

        h.world_key_at(KeyCode::UP, ms(4000));
        assert_eq!(at(&h), wm::WM_SILVA, "the first of the pair moves as an arrow");
        h.world_key_at(KeyCode::RIGHT, ms(4000 + WORLD_CHORD_MS - 1));
        assert_eq!(at(&h), wm::WM_SILVA_E, "the second makes it the diagonal from 남부");
        h.world_key_up(KeyCode::UP);
        h.world_key_up(KeyCode::RIGHT);
        tap(&mut h, KeyCode::LEFT, 5000);
        h.world_key_at(KeyCode::RIGHT, ms(6000));
        assert_eq!(at(&h), wm::WM_HYULEN);
        h.world_key_at(KeyCode::UP, ms(6050));
        assert_eq!(at(&h), wm::WM_SILVA_E, "the other order");
        h.world_key_up(KeyCode::RIGHT);
        h.world_key_up(KeyCode::UP);

        tap(&mut h, KeyCode::LEFT, 7000);
        h.world_key_at(KeyCode::UP, ms(8000));
        h.world_key_at(KeyCode::RIGHT, ms(8000 + WORLD_CHORD_MS));
        assert_eq!(at(&h), wm::WM_SILVA_S, "too far apart: right from 실바나르 is its one road back");
        h.world_key_up(KeyCode::UP);
        h.world_key_up(KeyCode::RIGHT);
        h.world_key_at(KeyCode::UP, ms(9000));
        h.world_key_up(KeyCode::UP);
        h.world_key_at(KeyCode::RIGHT, ms(9010));
        assert_eq!(at(&h), wm::WM_SILVA_S, "the first let go: two arrows");
        assert_eq!(h.world_key(KeyCode::CLEAR), vec![Out::World(false)]);
    }

    /// The pad's axes on the world map, as a HID pad sends them: a hat corner is one sample with
    /// both axes (the S9's virtual pad went to 휼렌 on ↗ before), a stick rolls through an arrow.
    #[test]
    fn the_pads_corner_and_stick_reach_the_diagonal() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        h.set_map(MapGrid::new(44, 3, 3, &[0i8; 9], &[]).unwrap());
        let tab = centre(h.tab(hud::Tab::Map).expect("MAP in the field"));
        h.started(1, tab.0, tab.1);
        h.ended(1);
        let at = |h: &Hud| h.world().unwrap().cursor;
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        assert_eq!(pad_way(0.3, -0.4), None, "inside half a throw");
        assert_eq!(pad_way(1.0, -1.0), Some(wm::WmDir::UpRight));
        assert_eq!(pad_way(0.9, 0.2), Some(wm::WmDir::Right));
        assert_eq!(pad_way(-0.7, 0.8), Some(wm::WmDir::DownLeft));

        h.world_pad_at(1.0, -1.0, ms(0));
        assert_eq!(at(&h), wm::WM_SILVA_E, "the hat's corner: 동부 from 남부 at once");
        h.world_pad_at(0.0, -1.0, ms(40));
        assert_eq!(at(&h), wm::WM_SILVA_E, "the corner let go through its up: no step");
        h.world_pad_at(0.0, 0.0, ms(60));
        h.world_pad_at(-1.0, 0.0, ms(1000));
        assert_eq!(at(&h), wm::WM_SILVA_S, "left: back to 남부");
        h.world_pad_at(0.0, 0.0, ms(1100));

        h.world_pad_at(0.95, -0.1, ms(2000));
        assert_eq!(at(&h), wm::WM_HYULEN, "the stick's right first: 휼렌");
        h.world_pad_at(0.8, -0.75, ms(2060));
        assert_eq!(at(&h), wm::WM_SILVA_E, "rolled onto the diagonal at once: from 남부");
        h.world_pad_at(0.0, 0.0, ms(2200));
        h.world_pad_at(-1.0, 0.0, ms(3000));
        h.world_pad_at(0.0, 0.0, ms(3100));
        h.world_pad_at(0.95, 0.0, ms(4000));
        assert_eq!(at(&h), wm::WM_HYULEN);
        h.world_pad_at(0.8, -0.8, ms(4000 + WORLD_CHORD_MS));
        assert_ne!(at(&h), wm::WM_SILVA_E, "rolled late: the diagonal from where the cursor is");
        assert!(h.world_pad_at(0.8, -0.8, ms(4500)).is_empty(), "the same way again is no step");
    }

    /// SFX LEVEL cycles its five steps, lit off 기본; 진단 정보 asks for the platform's dialog (B3 —
    /// 세이브 관리 lives under it there) and closes the window.
    #[test]
    fn sfx_level_cycles_and_the_diagnostics_row_asks_for_the_dialog() {
        let mut h = s9();
        open_settings(&mut h);
        let mut seen = Vec::new();
        for i in 0..5 {
            let out = tap_row(&mut h, 10 + i, SettingsAction::SfxLevel);
            let [Out::SfxLevel(db)] = out[..] else { panic!("{out:?}") };
            seen.push(db);
            let (_, word, state) = h.settings_words(SettingsAction::SfxLevel, true);
            assert!(crate::host_text::korean(word).is_some(), "{word}");
            assert_eq!(state == hud::RowState::Normal, db == 0, "lit off 기본");
        }
        assert_eq!(seen, vec![3, 6, -6, -3, 0], "from 0, round again");
        assert_eq!(tap_row(&mut h, 2, SettingsAction::Diag), vec![Out::OpenDiag]);
        assert!(!h.settings_open(), "the dialog is the platform's, over the game");
    }

    /// A tap on the game's picture that no control takes goes to the game's own windows, in guest
    /// px (user 2026-10-25, hero3's touch menus) — not in the bare field, which has none.
    #[test]
    fn a_tap_on_the_games_window_goes_to_the_game() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Menu, popup: false, notice: false, ..Default::default() });
        let s = *h.screen();
        // The main menu's second row on the S9's 492 × 240: guest (246, 70).
        let (x, y) = (f64::from(s.game.x) + 246.5 * f64::from(s.game.w) / f64::from(s.guest_w), f64::from(s.game.y) + 70.5 * f64::from(s.game.h) / f64::from(s.guest_h));
        assert_eq!(h.started(1, x, y), vec![Out::GuestTap(246, 70)]);
        assert!(h.ended(1).is_empty(), "a tap is its press; the lift is nothing");
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert!(h.started(2, x, y).is_empty(), "the bare field");
        h.ended(2);
        h.set_view(GuestView { scene: Scene::Field, popup: true, notice: false, ..Default::default() });
        assert_eq!(h.started(3, x, y), vec![Out::GuestTap(246, 70)], "a message box over the field");
        h.ended(3);
        let ok = h.layout().controls.iter().find(|c| c.action == Action::Key(KeyCode::OK)).map(|c| c.shape.bounds().centre()).unwrap();
        h.set_view(GuestView { scene: Scene::Menu, popup: false, notice: false, ..Default::default() });
        assert!(!h.started(4, f64::from(ok.0), f64::from(ok.1)).contains(&Out::GuestTap(0, 0)), "a control keeps its touch");
        assert!(h.started(5, -1.0, 5.0).is_empty(), "off the picture");
        // A scenario: the stick's zone turns the page too; in the field it is the stick.
        h.set_view(GuestView { scene: Scene::Field, ..Default::default() });
        let zone = h.layout().stick.unwrap().zone;
        let (zx, zy) = (f64::from(zone.x + zone.w / 2), f64::from(zone.y + zone.h / 2));
        h.set_view(GuestView { scene: Scene::Script, popup: false, notice: false, ..Default::default() });
        assert!(matches!(h.started(6, zx, zy)[..], [Out::GuestTap(_, _)]), "the dialogue");
        h.ended(6);
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        assert!(h.started(7, zx, zy).is_empty() && h.joy().is_some(), "the field's stick");
    }

    /// The tabs follow the map the target panel is drawn over (user 2026-10-25: SAVE sat on the
    /// panel's corner in a 20-tile room): a narrow fight map moves them off it, a town as narrow
    /// leaves them where a wide field has them, and a map whose file is unread counts as a fight.
    #[test]
    fn the_tabs_step_aside_over_a_narrow_fight_map_only() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        let grid = |id, w: u8, fights| MapGrid { fights, ..MapGrid::new(id, w, 30, &vec![0i8; usize::from(w) * 30], &[]).unwrap() };
        h.set_map(grid(46, 40, Some(true)));
        let field = h.tab(hud::Tab::Save).expect("SAVE in the field");
        assert_eq!(field, hud::save_tab(h.screen(), h.screen().guest_w));
        let top = h.world_panel().view.y;
        h.set_map(grid(135, 20, Some(true)));
        let room = h.tab(hud::Tab::Save).unwrap();
        assert!(room.right() < field.right() && room.w <= field.w, "{room:?} vs {field:?}");
        let target = hud::target_box(h.screen(), h.target_place().0, hz_java::hooks::FIGHT_MAP_MIN_W);
        assert!(!hud::top_tab_hit(h.screen(), room).intersects(&target), "{room:?} clear of {target:?}");
        assert!(h.world_panel().view.y <= top, "the world map hangs below the tabs as they are");
        h.set_map(grid(45, 20, Some(false)));
        assert_eq!(h.tab(hud::Tab::Save), Some(field), "a town: no target panel to clear");
        h.set_map(grid(89, 20, None));
        assert_eq!(h.tab(hud::Tab::Save), Some(room), "unread: as a fight map");
        h.set_map(grid(46, 40, Some(true)));
        assert_eq!(h.tab(hud::Tab::Save), Some(field));
    }

    /// The analog triggers keep the keys the player gave them on the world map (review
    /// 2026-10-25: once the stick was read eight ways, L2 put on back no longer closed it).
    #[test]
    fn a_trigger_on_back_closes_the_world_map() {
        let mut h = s9();
        h.set_view(GuestView { scene: Scene::Field, popup: false, notice: false, ..Default::default() });
        h.set_map(MapGrid::new(44, 3, 3, &[0i8; 9], &[]).unwrap());
        let tab = centre(h.tab(hud::Tab::Map).expect("MAP in the field"));
        h.started(1, tab.0, tab.1);
        h.ended(1);
        assert!(h.world().is_some());
        let mut pad = gamepad::Pad::default();
        let press = pad.triggers(0.9, 0.0);
        assert_eq!(press, vec![(gamepad::PadAction::Key(KeyCode::NUM0), true)], "the built-in L2 is 0");
        assert!(h.world_pad(0.0, 0.0, &press).is_empty() && h.world().is_some(), "nothing to the map");
        let mut ov = Overrides::default();
        ov.set(gamepad::KEYCODE_BUTTON_L2, Some(gamepad::PadAction::Key(KeyCode::RIGHT_SOFT_KEY)));
        let mut pad = gamepad::Pad::new(&ov);
        let press = pad.triggers(0.9, 0.0);
        assert_eq!(h.world_pad(0.0, 0.0, &press), vec![Out::World(false)], "L2 on back closes it");
        assert!(h.world().is_none());
        assert!(h.world_pad(0.0, 0.0, &pad.triggers(0.0, 0.0)).is_empty(), "its release: the map is gone");
    }

    /// The unreadable-save notice comes during the boot load and must still be up at the slot
    /// list: it holds through the title, then fades as any notice does once the game leaves it.
    #[test]
    fn a_title_notice_waits_for_the_slot_list() {
        let text = crate::database::SAVE_UNREADABLE;
        let view = |scene| GuestView { scene, popup: false, notice: false, ..Default::default() };
        let mut h = s9();
        h.set_view(view(Scene::Loading));
        h.notice_through_title(text);
        assert_eq!(h.toast(), Some(text), "shown at once, over the boot load");
        let later = Instant::now() + Duration::from_millis(TOAST_MS * 4);
        assert_eq!(h.toast_at(later), None, "the load's copy times out like any notice");
        h.set_view(view(Scene::Other(0)));
        h.set_view(view(Scene::Title));
        assert_eq!(h.toast_at(later), Some(text), "back while the title — menu, slots, job — is up");
        h.clear_toast();
        assert_eq!(h.toast_at(later), Some(text), "the app's clearing of the timed copy leaves it");
        h.notice("GAMEPAD - BADGES SHOW ITS BUTTONS");
        assert!(h.toast().unwrap().starts_with("GAMEPAD"), "a newer notice still shows over it");
        assert_eq!(h.toast_at(later), Some(text), "and it comes back after");
        h.set_view(view(Scene::Script));
        assert_eq!(h.toast(), Some(text), "leaving the title: a last while");
        assert_eq!(h.toast_at(later), None, "then gone");
        h.set_view(view(Scene::Title));
        assert_eq!(h.toast_at(later), None, "and not again on a later title");
        assert!(crate::font::drawable(text));
    }
}
