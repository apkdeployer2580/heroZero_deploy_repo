//! hz_desktop — 영웅서기 Zero on the PC (Windows, 2026-10-29): the same host as the APK's
//! (`hz_android`: its emulator worker, its HUD, its saves and their vault) in a winit window. The
//! keyboard and the mouse stand in for the thumbs (`App::hw_key`: the arrows walk — two a diagonal —,
//! X · Space · Enter the HUD's OK, Esc its CLR, Q W E R A S D the quick plates and Tab their page,
//! the digits the keypad (`keys.rs`); F5 saves, M opens the world map, F11 full screen; the left
//! button is a finger on the HUD, the right the back key), a gamepad comes through gilrs.
//!
//! Files: the game next to the exe — `herozero.jar`, `hz_fanfare.wav`, `hz_worldmap.rgb565` — or
//! under `--assets`; the saves and options in `%APPDATA%\heroZero` (`--data`), as the APK keeps
//! them under its `files/` (the same `herozero.jar/db/<store>` layout, so a 세이브 옮기기 zip moves a
//! save between a phone and this PC); the player's exports in `문서\heroZero` — the known folder,
//! `OneDrive\문서` where OneDrive keeps the documents (`--docs` for another).
//!
//! usage: herozero [--jar <file>] [--assets <dir>] [--data <dir>] [--docs <dir>]
//!                 [--portrait | --landscape] [--log <filter>]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc::channel},
    time::{Duration, Instant},
};

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::WindowId,
};

use hz_host::{FANFARE_NAME, JAR_NAME, UserEvent, WORLDMAP_NAME, app, saves, saves_ui, worldmap_view};
use wie_backend::KeyCode;

/// The build's versionCode and versionName (`android_app/version.properties`, handed over by
/// `build_desktop.ps1`); a plain `cargo build` knows neither.
const VERSION_CODE: Option<&str> = option_env!("HEROZERO_VERSION_CODE");
const VERSION_NAME: Option<&str> = option_env!("HEROZERO_VERSION_NAME");

struct Args {
    jar: Option<PathBuf>,
    assets: Option<PathBuf>,
    data: Option<PathBuf>,
    /// The player's folder (exports, the transfer zips) in place of `문서\heroZero`.
    docs: Option<PathBuf>,
    landscape: Option<bool>,
    log: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { jar: None, assets: None, data: None, docs: None, landscape: None, log: None };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut value = |what: &str| it.next().ok_or_else(|| format!("{what} needs a value"));
        match k.as_str() {
            "--jar" => a.jar = Some(PathBuf::from(value("--jar")?)),
            "--assets" => a.assets = Some(PathBuf::from(value("--assets")?)),
            "--data" => a.data = Some(PathBuf::from(value("--data")?)),
            "--docs" => a.docs = Some(PathBuf::from(value("--docs")?)),
            "--log" => a.log = Some(value("--log")?),
            "--portrait" => a.landscape = Some(false),
            "--landscape" => a.landscape = Some(true),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

/// `ms` since the epoch in the PC's local time, `YYYY-MM-DD HH:MM`.
fn local_stamp(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().map_or_else(|| saves_ui::utc_stamp(ms), |t| t.format("%Y-%m-%d %H:%M").to_string())
}

fn read_asset(dir: &Path, name: &str) -> Option<Vec<u8>> {
    match std::fs::read(dir.join(name)) {
        Ok(b) => {
            log::info!("loaded {name} ({} bytes) from {}", b.len(), dir.display());
            Some(b)
        }
        Err(e) => {
            log::warn!("no {name} in {}: {e}", dir.display());
            None
        }
    }
}

/// The gamepad's axes as last sent to the app (`UserEvent::GamepadAxes`, Android's conventions:
/// y down is positive, the triggers 0..1).
#[derive(Clone, Copy, Default, PartialEq)]
struct PadAxes {
    x: f32,
    y: f32,
    hx: f32,
    hy: f32,
    lt: f32,
    rt: f32,
}

/// A gilrs button as the Android keycode the host's remap table speaks (`hz_host::gamepad`).
fn pad_code(b: gilrs::Button) -> Option<u32> {
    use gilrs::Button;
    use hz_host::gamepad::*;
    Some(match b {
        Button::South => KEYCODE_BUTTON_A,
        Button::East => KEYCODE_BUTTON_B,
        Button::West => KEYCODE_BUTTON_X,
        Button::North => KEYCODE_BUTTON_Y,
        Button::LeftTrigger => KEYCODE_BUTTON_L1,
        Button::RightTrigger => KEYCODE_BUTTON_R1,
        Button::LeftThumb => KEYCODE_BUTTON_THUMBL,
        Button::RightThumb => KEYCODE_BUTTON_THUMBR,
        Button::Start => KEYCODE_BUTTON_START,
        Button::Select => KEYCODE_BUTTON_SELECT,
        _ => return None,
    })
}

/// A key by the name the probe's `--keys` uses.
fn key_named(name: &str) -> Option<KeyCode> {
    Some(match name {
        "OK" => KeyCode::OK,
        "UP" => KeyCode::UP,
        "DOWN" => KeyCode::DOWN,
        "LEFT" => KeyCode::LEFT,
        "RIGHT" => KeyCode::RIGHT,
        "LSK" => KeyCode::LEFT_SOFT_KEY,
        "RSK" => KeyCode::RIGHT_SOFT_KEY,
        "CLR" => KeyCode::CLEAR,
        "STAR" => KeyCode::STAR,
        "HASH" => KeyCode::HASH,
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
        _ => return None,
    })
}

/// `HZ_KEYS` — a developer's scripted session (never a player's): `ms:WORD` entries, comma-separated,
/// each at that many ms after start. WORD is a key (`key_named`, held 120 ms), `F5` / `M` / `F11` (the
/// PC's own keys), a keyboard key as the player presses it (`KEY_X`, `KEY_ESC`, `KEY_SPACE`,
/// `KEY_ENTER`, `KEY_Q`, `KEY_W`, `KEY_TAB`, the arrows `KEY_UP` … `KEY_RIGHT` — `App::hw_key`, held 120 ms or
/// `KEY_…:ms`; two arrows held overlapping walk a diagonal, HZ-059), or an `App::dev_action` (`SHOT:name` with `HZ_SHOT_DIR`
/// set, `GEAR`, `ROW:n`, …).
fn parse_script(s: &str) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> = s
        .split(',')
        .filter_map(|e| {
            let (ms, word) = e.trim().split_once(':')?;
            Some((ms.trim().parse().ok()?, word.trim().to_string()))
        })
        .collect();
    out.sort_by_key(|(ms, _)| *ms);
    out
}

/// What gilrs had queued ([`Desktop::drain_pads`]).
#[derive(Default)]
struct PadEvents {
    axes: PadAxes,
    edges: Vec<(u32, bool)>,
    dpad: Vec<(KeyCode, bool)>,
    lost: bool,
}

/// The window's handler: the host's `App` with a gamepad polled around it.
struct Desktop {
    app: app::App,
    pads: Option<gilrs::Gilrs>,
    axes: PadAxes,
    /// The pad's D-pad arrows down (they go to the app as keys, beside the keyboard's).
    dpad_held: Vec<KeyCode>,
    /// The window has the focus. gilrs reads the pads whatever window is in front, so without it a
    /// pad used in another game after Alt+Tab drove this one too (code review 2026-10-31).
    focused: bool,
    /// The `HZ_KEYS` script, what is left of it, and the key releases it owes.
    script: Vec<(u64, String)>,
    started: Instant,
    releases: Vec<(Instant, Release)>,
}

/// A key the script pressed and must let go of: the game's, or the keyboard's.
#[derive(Clone, Copy)]
enum Release {
    Game(KeyCode),
    Hw(winit::keyboard::PhysicalKey),
}

impl Desktop {
    /// Run what the script owes by now; when the next entry is due.
    fn run_script(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.started).as_millis() as u64;
        while self.script.first().is_some_and(|(ms, _)| *ms <= elapsed) {
            let (_, word) = self.script.remove(0);
            log::info!("script: {word}");
            use winit::keyboard::{KeyCode as W, PhysicalKey};
            // `KEY_…:ms` holds the key that long.
            let (word, hold) = match word.split_once(':') {
                Some((name, ms)) if name.starts_with("KEY_") => (name.to_string(), ms.parse().unwrap_or(120)),
                _ => (word, 120),
            };
            let hw = match word.as_str() {
                "KEY_X" => Some(W::KeyX),
                "KEY_ESC" => Some(W::Escape),
                "KEY_SPACE" => Some(W::Space),
                "KEY_ENTER" => Some(W::Enter),
                "KEY_Q" => Some(W::KeyQ),
                "KEY_W" => Some(W::KeyW),
                "KEY_TAB" => Some(W::Tab),
                "KEY_UP" => Some(W::ArrowUp),
                "KEY_DOWN" => Some(W::ArrowDown),
                "KEY_LEFT" => Some(W::ArrowLeft),
                "KEY_RIGHT" => Some(W::ArrowRight),
                _ => None,
            };
            if let Some(code) = key_named(&word) {
                self.app.key_press(code, true, false);
                self.releases.push((now + Duration::from_millis(120), Release::Game(code)));
            } else if let Some(k) = hw {
                self.app.hw_key(PhysicalKey::Code(k), true, false);
                self.releases.push((now + Duration::from_millis(hold), Release::Hw(PhysicalKey::Code(k))));
            } else {
                match word.as_str() {
                    "F5" => self.app.hw_key(PhysicalKey::Code(W::F5), true, false),
                    "M" => self.app.hw_key(PhysicalKey::Code(W::KeyM), true, false),
                    "F11" => self.app.hw_key(PhysicalKey::Code(W::F11), true, false),
                    "QUIT" => event_loop.exit(),
                    _ => self.app.dev_action(&word),
                }
            }
        }
        let mut due: Vec<Release> = Vec::new();
        self.releases.retain(|(at, r)| {
            if *at <= now {
                due.push(*r);
                false
            } else {
                true
            }
        });
        for r in due {
            match r {
                Release::Game(code) => self.app.key_press(code, false, false),
                Release::Hw(k) => self.app.hw_key(k, false, false),
            }
        }
        let next_entry = self.script.first().map(|(ms, _)| self.started + Duration::from_millis(*ms));
        let next_release = self.releases.iter().map(|(at, _)| *at).min();
        [next_entry, next_release].into_iter().flatten().min()
    }

    /// What gilrs has queued since the last look: the sticks, hat and triggers as one axis sample
    /// (from the last one), the buttons' edges, the D-pad's, and whether a pad went away. `None`
    /// without gilrs. L2/R2 come as analog values only — a pad that also reports them as buttons would
    /// press twice.
    fn drain_pads(&mut self) -> Option<PadEvents> {
        let g = self.pads.as_mut()?;
        let mut ev = PadEvents { axes: self.axes, ..PadEvents::default() };
        while let Some(gilrs::Event { event, .. }) = g.next_event() {
            use gilrs::{Axis, Button, EventType};
            match event {
                EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) => {
                    let down = matches!(event, EventType::ButtonPressed(..));
                    match b {
                        Button::DPadUp => ev.dpad.push((KeyCode::UP, down)),
                        Button::DPadDown => ev.dpad.push((KeyCode::DOWN, down)),
                        Button::DPadLeft => ev.dpad.push((KeyCode::LEFT, down)),
                        Button::DPadRight => ev.dpad.push((KeyCode::RIGHT, down)),
                        Button::LeftTrigger2 | Button::RightTrigger2 => {}
                        b => {
                            if let Some(code) = pad_code(b) {
                                ev.edges.push((code, down));
                            }
                        }
                    }
                }
                EventType::ButtonChanged(Button::LeftTrigger2, v, _) => ev.axes.lt = v.clamp(0.0, 1.0),
                EventType::ButtonChanged(Button::RightTrigger2, v, _) => ev.axes.rt = v.clamp(0.0, 1.0),
                EventType::AxisChanged(a, v, _) => match a {
                    Axis::LeftStickX => ev.axes.x = v,
                    Axis::LeftStickY => ev.axes.y = -v,
                    Axis::DPadX => ev.axes.hx = v,
                    Axis::DPadY => ev.axes.hy = -v,
                    _ => {}
                },
                EventType::Connected => log::info!("gamepad connected"),
                EventType::Disconnected => {
                    log::info!("gamepad disconnected");
                    ev.lost = true;
                }
                _ => {}
            }
        }
        Some(ev)
    }

    /// The pads to the app: the axis sample when it changed (the app quantises it as it does the
    /// phone's over JNI), the buttons as the host's pad buttons, the D-pad as the arrows. Away from
    /// the window (`focused` false) the events are drained and dropped — none waits to replay on the
    /// way back ([`Self::focus`] drains what came since the last look); a stick still leaning then
    /// counts from its next move.
    fn poll_pads(&mut self, event_loop: &ActiveEventLoop) {
        let Some(PadEvents { axes, edges, dpad, lost }) = self.drain_pads() else { return };
        if self.focused {
            if axes != self.axes {
                self.app.user_event(event_loop, UserEvent::GamepadAxes { x: axes.x, y: axes.y, hx: axes.hx, hy: axes.hy, lt: axes.lt, rt: axes.rt });
            }
            for (code, down) in edges {
                self.app.pad_button(code, down, false);
            }
            for (k, down) in dpad {
                if down {
                    if !self.dpad_held.contains(&k) {
                        self.dpad_held.push(k);
                    }
                } else if self.dpad_held.contains(&k) {
                    self.dpad_held.retain(|h| *h != k);
                } else {
                    continue; // pressed while away: its press never went
                }
                self.app.key_press(k, down, false);
            }
        }
        self.axes = axes;
        if lost {
            self.pad_gone();
        }
    }

    /// A pad unplugged (or its battery gone) with a stick leaning or a button down sends no release:
    /// let go of what it held — its D-pad's arrows here, the rest in `App::pad_lost` — and start the
    /// next pad's axes from rest.
    fn pad_gone(&mut self) {
        self.axes = PadAxes::default();
        for k in std::mem::take(&mut self.dpad_held) {
            self.app.key_press(k, false, false);
        }
        self.app.pad_lost();
    }

    /// The window got or lost the focus. Losing it, the D-pad's arrows let go here (they went as
    /// keys, which the app does not track); the app's own `Focused(false)` lets go of the stick, the
    /// buttons and the keyboard's keys. Getting it back, what gilrs queued since the background's
    /// last look (it looks every half second) is dropped before the next poll could deliver it (code
    /// review 2026-10-31); the axes it held are kept as the pad's state, sent with its next move — a
    /// trigger still pulled then presses nothing until it is let go (2nd code review 2026-10-31: L2 pulled
    /// in another window and let go slowly read 1.0 → 0.8 as a new press and fired a quick slot).
    fn focus(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        log::info!("window {}: the gamepad {}", if focused { "focused" } else { "in the background" }, if focused { "drives the game again" } else { "is ignored" });
        if focused && let Some(PadEvents { axes, lost, .. }) = self.drain_pads() {
            self.axes = axes;
            if lost {
                self.pad_gone();
            } else {
                self.app.pad_resume(axes.lt, axes.rt);
            }
        }
        self.focused = focused;
        if !focused {
            for k in std::mem::take(&mut self.dpad_held) {
                self.app.key_press(k, false, false);
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for Desktop {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.app.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.app.suspended(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let WindowEvent::Focused(focused) = event {
            self.focus(focused);
        }
        self.app.window_event(event_loop, id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.app.user_event(event_loop, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_pads(event_loop);
        let script_next = if self.script.is_empty() && self.releases.is_empty() { None } else { self.run_script(event_loop) };
        self.app.about_to_wait(event_loop);
        if let Some(next) = script_next {
            match event_loop.control_flow() {
                ControlFlow::WaitUntil(t) if t <= next => {}
                ControlFlow::Poll => {}
                _ => event_loop.set_control_flow(ControlFlow::WaitUntil(next)),
            }
        }
        // gilrs is polled, not pushed: with a pad connected wake every 8 ms (under the game's own
        // 50 ms frame), else — or in the background, where it is only drained — every half second,
        // unless the app wakes sooner anyway.
        if let Some(g) = &self.pads {
            let connected = g.gamepads().next().is_some();
            let next = Instant::now() + Duration::from_millis(if connected && self.focused { 8 } else { 500 });
            match event_loop.control_flow() {
                ControlFlow::WaitUntil(t) if t <= next => {}
                ControlFlow::Poll => {}
                _ => event_loop.set_control_flow(ControlFlow::WaitUntil(next)),
            }
        }
    }
}

/// Windows' scheduler ticks every 15.6 ms unless someone asks for less: every timed wait — the
/// worker's nap to the game's next frame (`recv_timeout`), winit's `WaitUntil` — then lands on the
/// next tick, and the game's 50 ms frame became 62~78 ms (13 fps idle in the field, measured
/// 2026-10-29 against 20 on the phone). `timeBeginPeriod(1)` is the classic cure (winmm); it is
/// process-wide and undone at exit.
fn raise_timer_resolution() {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
    }
    // SAFETY: a plain Win32 call with no pointers; TIMERR_NOERROR = 0.
    let r = unsafe { timeBeginPeriod(1) };
    if r == 0 {
        log::info!("timer resolution: 1 ms");
    } else {
        log::warn!("timer resolution: timeBeginPeriod(1) refused ({r})");
    }
}

/// The player's 문서 as Explorer shows it (`SHGetKnownFolderPath(FOLDERID_Documents)`): where
/// OneDrive keeps the documents it is `OneDrive\문서`, not `%USERPROFILE%\Documents` — measured
/// 2026-10-30 on this repo's own PC, where the readme's 「문서\heroZero\save_import」 was a folder the
/// game never read.
fn documents_dir() -> Option<PathBuf> {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};
    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    // {FDD39AD0-238F-46AF-ADB4-6C85480369C7}
    const FOLDERID_DOCUMENTS: Guid = Guid { data1: 0xFDD3_9AD0, data2: 0x238F, data3: 0x46AF, data4: [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7] };
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(rfid: *const Guid, flags: u32, token: *mut c_void, path: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *mut c_void);
    }
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: a valid GUID and out-pointer; on success `raw` is a NUL-terminated wide string the
    // caller frees with CoTaskMemFree (which also takes the null of a failure).
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_DOCUMENTS, 0, std::ptr::null_mut(), &mut raw) };
    let path = (hr == 0 && !raw.is_null()).then(|| {
        // SAFETY: read up to the terminator of the string the call returned.
        let len = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
        PathBuf::from(std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, len) }))
    });
    // SAFETY: the buffer SHGetKnownFolderPath allocated (or null).
    unsafe { CoTaskMemFree(raw.cast()) };
    path
}

/// A message box the player sees whether or not a console is attached.
fn message_box(text: &str) {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide("영웅서기 Zero"));
    // SAFETY: NUL-terminated wide strings that outlive the call; MB_ICONINFORMATION.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x40) };
}

/// One game per data folder (the APK has one activity, `singleTask`): a second herozero.exe on the
/// same saves would apply a staged import or restore under the first one's running game — which
/// then writes its own character back over it — and the two would save the same slots in turn.
/// `None` when another one holds the folder; a file system without locks is let through.
fn lock_data(data_dir: &Path) -> Option<Option<std::fs::File>> {
    let Ok(f) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("herozero.lock")) else {
        return Some(None);
    };
    match f.try_lock() {
        Ok(()) => Some(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => None,
        Err(e) => {
            log::warn!("data folder not locked: {e}");
            Some(None)
        }
    }
}

pub fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("herozero: {e}");
            }
            eprintln!("usage: herozero [--jar <file>] [--assets <dir>] [--data <dir>] [--docs <dir>] [--portrait | --landscape] [--log <filter>]");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    let filter = args.log.clone().or_else(|| std::env::var("RUST_LOG").ok()).unwrap_or_else(|| "warn,main=info,hz=info,hz_java=info,herozero=info".into());
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new(filter)).with_writer(std::io::stderr).init();
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));
    let app_code: u32 = VERSION_CODE.and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let version = format!("PC {} ({})", VERSION_NAME.unwrap_or("dev").trim(), app_code);
    log::info!("herozero {version}: 영웅서기 Zero PC host starting");
    raise_timer_resolution();

    // The game next to the exe.
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
    let assets = args.assets.clone().unwrap_or_else(|| exe_dir.clone());
    let jar_path = args.jar.clone().unwrap_or_else(|| assets.join(JAR_NAME));
    let jar = match std::fs::read(&jar_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("herozero: cannot read the game {}: {e}\nput herozero.jar next to herozero.exe, or pass --jar <file>", jar_path.display());
            std::process::exit(2);
        }
    };
    log::info!("loaded {} ({} bytes)", jar_path.display(), jar.len());
    let renditions: Vec<Arc<[u8]>> = read_asset(&assets, FANFARE_NAME).map(|w| vec![Arc::from(w)]).unwrap_or_default();
    match read_asset(&assets, WORLDMAP_NAME).and_then(|raw| worldmap_view::wm_parse_picture(&raw)) {
        Some(p) => {
            log::info!("world map picture {}x{}", p.w, p.h);
            worldmap_view::wm_set_picture(p);
        }
        None => log::warn!("no {WORLDMAP_NAME}: the world map draws a flat sea"),
    }

    // The saves and options: `%APPDATA%\heroZero`; the player's folder: `문서\heroZero`.
    let data_dir = args.data.clone().unwrap_or_else(|| {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| exe_dir.join("data")).join("heroZero")
    });
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("herozero: cannot make the data directory {}: {e}", data_dir.display());
        std::process::exit(2);
    }
    // Held to the end of `main`: the lock goes with the process.
    let Some(_lock) = lock_data(&data_dir) else {
        log::warn!("another herozero holds {}", data_dir.display());
        message_box("영웅서기 Zero 가 이미 실행 중입니다.\n\n같은 세이브로 두 개를 함께 띄울 수 없습니다. 먼저 띄운 창을 써 주세요.");
        std::process::exit(1);
    };
    let documents = documents_dir().or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Documents")));
    let ext_dir = args.docs.clone().or_else(|| documents.map(|d| d.join("heroZero"))).filter(|d| std::fs::create_dir_all(d).is_ok()).unwrap_or_else(|| data_dir.clone());
    log::info!("data dir = {} · player's folder = {}", data_dir.display(), ext_dir.display());
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);

    // As `android_main`: a 세이브 옮기기 import the player confirmed last run, then a restore, both
    // before the game opens any store.
    let imported = if saves::import_pending(&data_dir) {
        if let Some(build) = hz_patches::Build::identify(&jar) {
            saves::set_game(build.jar_md5());
        }
        saves::apply_pending_import(&data_dir, saves::game(), now_ms).map(|r| {
            log::warn!("save transfer: {}", r.log);
            r.notice
        })
    } else {
        None
    };
    let restored = saves::apply_pending(&data_dir, now_ms);
    match &restored {
        saves::Applied::None => {}
        saves::Applied::Done(stores) => log::warn!("saves: restored {stores:?}"),
        saves::Applied::Partial(stores, why) => log::error!("saves: restore incomplete ({why}); put back {stores:?}"),
    }

    let options = Arc::new(hz_java::GameOptions::load(&data_dir));
    let landscape = args.landscape.unwrap_or_else(|| options.landscape_now());

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("herozero: cannot open a window: {e}");
            std::process::exit(2);
        }
    };
    let proxy = event_loop.create_proxy();
    let (input_tx, input_rx) = channel();
    let paused = Arc::new(AtomicBool::new(false));
    let boot = app::Boot {
        proxy,
        input_rx,
        jar,
        renditions,
        landscape,
        options,
        ext_dir: Some(ext_dir),
        restored,
        imported,
        desktop: true,
        scene: Arc::new(hz_java::GuestState::new()),
        app_code,
        version,
        fmt_time: local_stamp,
    };
    let app = app::App::new(input_tx, data_dir, paused, boot);
    let pads = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("gamepad: gilrs unavailable: {e}");
            None
        }
    };
    let script = std::env::var("HZ_KEYS").map(|s| parse_script(&s)).unwrap_or_default();
    if !script.is_empty() {
        log::warn!("HZ_KEYS: a scripted session of {} entries", script.len());
    }
    let mut desktop = Desktop { app, pads, axes: PadAxes::default(), dpad_held: Vec::new(), focused: true, script, started: Instant::now(), releases: Vec::new() };
    if let Err(e) = event_loop.run_app(&mut desktop) {
        log::error!("event loop exited with error: {e}");
    }
    log::info!("herozero: bye");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known folder, whatever `USERPROFILE` says — on this repo's PC `OneDrive\문서`.
    #[test]
    fn documents_is_the_folder_explorer_calls_documents() {
        let d = documents_dir().expect("FOLDERID_Documents");
        eprintln!("documents = {}", d.display());
        assert!(d.is_absolute() && d.is_dir(), "{}", d.display());
    }

    #[test]
    fn a_second_game_on_the_same_data_folder_is_turned_away() {
        let d = std::env::temp_dir().join(format!("hz_desktop_lock_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let first = lock_data(&d).expect("the first takes it");
        assert!(first.is_some());
        assert!(lock_data(&d).is_none(), "the second is turned away");
        drop(first);
        assert!(lock_data(&d).is_some_and(|f| f.is_some()), "free again once the first is gone");
        let _ = std::fs::remove_dir_all(&d);
    }
}
