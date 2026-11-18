//! The UI thread: window, surface, the picture, the touch HUD, hardware keys.
//!
//! It never touches the JVM. Every touch goes through [`touch::Hud`] (the host-tested state
//! machine) and what comes back is sent to the worker as [`InputEvent`]s or written to disk; frames
//! come back as [`UserEvent::Paint`] and are drawn on the next `RedrawRequested`.

use std::{
    cell::Cell,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
    time::Instant,
};

use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    keyboard::{KeyCode as WinitKey, NativeKeyCode, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};

use wie_backend::KeyCode;

use crate::{
    InputEvent, UserEvent, config, emulator,
    gamepad::{self, Overrides},
    hostwin::{HostAct, HostWin},
    hud::Screen,
    keys, placement, render, saves_ui,
    touch::{Hud, Out},
};

/// What the first `resumed` needs to start the worker (the guest canvas is sized from the real
/// window, which does not exist before then).
pub struct Boot {
    pub proxy: EventLoopProxy<UserEvent>,
    pub input_rx: Receiver<InputEvent>,
    pub jar: Vec<u8>,
    /// The WAVs rendered from the game's MIDI files (Phase 4b), as loaded from the assets.
    pub renditions: Vec<std::sync::Arc<[u8]>>,
    pub landscape: bool,
    pub options: Arc<hz_java::GameOptions>,
    /// The app's external files directory (exports go there), if the device has one.
    pub ext_dir: Option<PathBuf>,
    /// What the boot's restore did (`crate::saves::apply_pending`, before anything else ran).
    pub restored: crate::saves::Applied,
    /// What the boot's 세이브 옮기기 import did, for the platform's dialog (`saves::ImportReport`).
    pub imported: Option<String>,
    /// The PC (`hz_desktop`, 2026-10-29): a resizable window, the mouse as a finger, the keyboard's
    /// own keys, the quick bar, host windows for the dialogs (`hud::Screen::desktop`).
    pub desktop: bool,
    /// What the game is showing (`hz_java::GuestState`), shared with the worker — the UI asks it to
    /// letter a host window's run-time lines.
    pub scene: Arc<hz_java::GuestState>,
    /// This build's versionCode for the transfer file's newer-app check (0 = unknown).
    pub app_code: u32,
    /// What 진단 정보 calls the app (`PC 0.1.0 (48)`).
    pub version: String,
    /// A moment as the player reads it (`saves_ui::Ctx::fmt_time`).
    pub fmt_time: fn(u64) -> String,
}

pub struct App {
    input_tx: Sender<InputEvent>,
    data_dir: PathBuf,
    paused: Arc<AtomicBool>,
    boot: Option<Boot>,

    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    /// RENDER size: the window divided by `render_div`. Everything below — the surface, the
    /// layout, touch — is in render space (hero2's HEAT SAVER, 57차).
    win_w: u32,
    win_h: u32,
    /// 1 = native, 2 = HEAT SAVER (half resolution on each axis, upscaled by the compositor).
    /// Fixed at start from `config::read_heat_saver`.
    render_div: u32,

    guest_w: u32,
    guest_h: u32,
    frame: Vec<u32>,
    cols: Vec<u32>,
    /// The layout as loaded, until the HUD exists (it needs the guest canvas, sized at boot).
    table: Option<placement::Table>,
    /// Which orientation's layout file `table` came from and is written back to.
    layout_landscape: bool,
    hud: Option<Hud>,

    /// Hardware keys currently down, so a key's repeat events do not re-press it.
    hw_down: Vec<KeyCode>,
    /// The PC's OK and CLR keys (X · Space · Enter · Esc): the game key each press sent — OK is
    /// LSK at the 창고 question and CLR is the menu in the field, so a release must let go of what
    /// the press chose, whatever the scene has become since.
    desk_sent: Vec<(WinitKey, KeyCode)>,
    /// What each pad button's press sent (`gamepad::view_key` may have made A a `#` or LSK), so its
    /// release lets go of that key and never of another.
    pad_sent: std::collections::HashMap<u32, Option<gamepad::PadAction>>,
    /// HZ-058: which pad sources' presses went to a plate — their releases and repeats go the same
    /// way (`gamepad::PlateRoute`).
    plate_route: gamepad::PlateRoute,
    /// The gamepad's stick / triggers, latched between axis samples (`crate::gamepad`).
    pad: gamepad::Pad,
    /// The player's button remap (`hz_gamepad.txt`), read once at start.
    overrides: Overrides,
    /// HZ-014 / HZ-054: the target panel's shift and drop the worker last got (`Hud::target_place`).
    target_sent: (i32, i32),
    /// When the open world map's next animation frame is due (Phase 7-2).
    world_next: Instant,
    /// Where an export goes (`Boot::ext_dir`): 진단 정보 says so.
    ext_dir: Option<PathBuf>,
    /// The player's 「추가 설정」 options (the worker's hooks have the same ones): the HUD reads
    /// 조작 방식 from them at each touch.
    options: Arc<hz_java::GameOptions>,
    /// When the window was last presented, and whether a redraw waits for [`MIN_PRESENT`] to pass
    /// since (`request_redraw`).
    last_present: Cell<Instant>,
    redraw_held: Cell<bool>,
    /// The PC (`Boot::desktop`).
    desktop: bool,
    scene: Arc<hz_java::GuestState>,
    app_code: u32,
    version: String,
    fmt_time: fn(u64) -> String,
    /// The mouse (the PC): where it is in render space, and whether its left button is down —
    /// finger 0 of the touch HUD.
    mouse: Option<(f64, f64)>,
    mouse_down: bool,
    /// 지금 닫기 (a host window): the loop exits on its next turn.
    exit_wanted: Cell<bool>,
    /// The boot's 세이브 옮기기 account (the PC's host window), waiting for the game's screen: its
    /// lines are lettered in the game's font from the title on, and before that the window showed
    /// only their ASCII scraps (`: 1 -`) and held the key the logo waits for (2026-10-30).
    boot_message: Option<String>,
    fullscreen: bool,
    /// The UI's gauge (★ 2026-10-29, the PC's frame-drop hunt): presents since the last line, what
    /// they cost (blit + HUD + present), and when the line was last written.
    ui_gauge: (u64, u64, u64, Instant),
}

/// Where a moment is read from.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// `p` in the system's file manager (the PC's 폴더 열기).
fn open_folder(p: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "android")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
    #[cfg(target_os = "android")]
    {
        log::info!("open folder {p:?}: not on Android");
    }
}

/// The open world map's animation step: hero5's camera halves its distance every ~33 ms.
const WORLD_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

/// The least time between two presents asked for by anything but a new game frame (2026-10-27).
/// A finger on the stick reports at the panel's touch rate — 120~240 a second on the S25 — and
/// each report asked for a whole present (picture + HUD, 2.5 M pixels); the game draws 20 frames
/// a second. Keys are sent when the touch is handled, not when it is drawn, so holding the HUD's
/// picture to 60 a second costs no input latency; a game frame is drawn at once (`redraw_now`).
const MIN_PRESENT: std::time::Duration = std::time::Duration::from_millis(16);

impl App {
    pub fn new(input_tx: Sender<InputEvent>, data_dir: PathBuf, paused: Arc<AtomicBool>, boot: Boot) -> Self {
        // Seeded BEFORE the first layout pass, or the defaults would be drawn until something
        // relaid out and the player's arrangement would seem to reset itself on every launch.
        let (table, dropped) = placement::load(&data_dir, boot.landscape);
        if dropped > 0 {
            log::warn!("layout: {dropped} unreadable line(s) in {} dropped", placement::layout_file(boot.landscape));
        }
        let heat_saver = config::read_heat_saver(&data_dir);
        if heat_saver {
            log::info!("heat saver on: rendering at half resolution");
        }
        let (overrides, bad) = Overrides::load(&data_dir);
        if bad > 0 {
            log::warn!("gamepad: {bad} unreadable line(s) in {} dropped", gamepad::MAP_FILE);
        }
        if !overrides.is_empty() {
            log::info!("gamepad: remap loaded from {}", gamepad::MAP_FILE);
        }
        crate::audio_mix::set_sfx_db(config::read_sfx_db(&data_dir));
        let ext_dir = boot.ext_dir.clone();
        let layout_landscape = boot.landscape;
        let options = boot.options.clone();
        let (desktop, scene, app_code, version, fmt_time) = (boot.desktop, boot.scene.clone(), boot.app_code, boot.version.clone(), boot.fmt_time);
        Self {
            input_tx,
            data_dir,
            paused,
            boot: Some(boot),
            window: None,
            context: None,
            surface: None,
            win_w: 1,
            win_h: 1,
            render_div: config::render_div(heat_saver),
            guest_w: crate::PORTRAIT_SIZE.0,
            guest_h: crate::PORTRAIT_SIZE.1,
            frame: Vec::new(),
            cols: Vec::new(),
            table: Some(table),
            layout_landscape,
            hud: None,
            hw_down: Vec::new(),
            desk_sent: Vec::new(),
            pad_sent: Default::default(),
            plate_route: Default::default(),
            pad: gamepad::Pad::new(&overrides),
            overrides,
            target_sent: (0, 0),
            world_next: Instant::now(),
            ext_dir,
            options,
            last_present: Cell::new(Instant::now()),
            redraw_held: Cell::new(false),
            desktop,
            scene,
            app_code,
            version,
            fmt_time,
            mouse: None,
            mouse_down: false,
            exit_wanted: Cell::new(false),
            boot_message: None,
            fullscreen: false,
            ui_gauge: (0, 0, 0, Instant::now()),
        }
    }

    /// 진단 정보's Rust half (B3 — `MainActivity.showDiagnostics` puts the app version, Android and
    /// the device above it): what this install holds, read from the files now.
    #[cfg(target_os = "android")]
    fn diag_text(&self) -> String {
        let mut out = vec![format!(
            "화면 {}x{} (렌더 1/{}) · 게임 그림 {}x{}",
            self.win_w * self.render_div,
            self.win_h * self.render_div,
            self.render_div,
            self.guest_w,
            self.guest_h
        )];
        out.extend(crate::saves::slot_report(&crate::saves::slots(&self.data_dir)));
        out.push(format!("금고 파일 {}개", saves_ui::count_files(&self.data_dir.join("save_vault"))));
        out.push(format!(
            "효과음 크기 {} dB · 발열 절감 {}",
            crate::audio_mix::sfx_db(),
            if self.render_div > 1 { "켬" } else { "끔" }
        ));
        out.push(format!(
            "패치: {}",
            if self.data_dir.join(config::PATCHES_FILE).exists() { "파일에서 (hz_patches.txt)" } else { "기본 묶음" }
        ));
        out.push(match self.ext_dir.as_ref() {
            Some(d) => format!("내보내기 폴더: {}", d.join(crate::saves::EXPORT_DIR).display()),
            None => "내보내기: 외부 저장소 없음".to_string(),
        });
        out.join("\n")
    }

    /// Tell the worker where the game's target panel goes when the minimap's state or geometry
    /// moved it (the hooks' default, 0 and 0, is where the game puts it).
    fn sync_target_shift(&mut self) {
        let Some((shift, drop)) = self.hud.as_ref().map(Hud::target_place) else { return };
        let (sent_shift, sent_drop) = self.target_sent;
        if shift != sent_shift {
            let _ = self.input_tx.send(InputEvent::TargetShift(shift));
        }
        if drop != sent_drop {
            let _ = self.input_tx.send(InputEvent::TargetDrop(drop));
        }
        self.target_sent = (shift, drop);
    }

    fn send_key(&self, code: KeyCode, down: bool) {
        let _ = self.input_tx.send(InputEvent::Key { code, down });
    }

    /// Carry out what the HUD asked for.
    fn apply(&mut self, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Key { code, down } => self.send_key(code, down),
                Out::SaveLayout => {
                    // The orientation the table was loaded for (`App::new`): the boot's, which the
                    // window keeps until the next start.
                    if let Some(hud) = self.hud.as_ref()
                        && let Err(e) = placement::save(&self.data_dir, hud.table(), self.layout_landscape)
                    {
                        log::warn!("layout: cannot write {}: {e}", placement::layout_file(self.layout_landscape));
                    }
                }
                Out::HeatSaver(on) => match config::write_heat_saver(&self.data_dir, on) {
                    Ok(()) => log::info!("heat saver -> {on} (applies at the next start)"),
                    Err(e) => log::warn!("heat saver: cannot write {}: {e}", config::HEAT_SAVER_FILE),
                },
                Out::PadHud(mode) => match config::write_pad_hud(&self.data_dir, mode) {
                    Ok(()) => log::info!("pad badges -> {}", mode.as_token()),
                    Err(e) => log::warn!("pad badges: cannot write {}: {e}", config::PAD_HUD_FILE),
                },
                Out::Stats(on) => match config::write_stats(&self.data_dir, on) {
                    Ok(()) => log::info!("stats readout -> {on}"),
                    Err(e) => log::warn!("stats: cannot write {}: {e}", config::STATS_FILE),
                },
                Out::Minimap(on) => match config::write_minimap(&self.data_dir, on) {
                    Ok(()) => log::info!("minimap -> {on}"),
                    Err(e) => log::warn!("minimap: cannot write {}: {e}", config::MINIMAP_FILE),
                },
                Out::Gamepad(ov) => {
                    match ov.save(&self.data_dir) {
                        Ok(()) => log::info!("gamepad: remap written to {}", gamepad::MAP_FILE),
                        Err(e) => log::warn!("gamepad: cannot write {}: {e}", gamepad::MAP_FILE),
                    }
                    // What is held was pressed under the old map; its release would look up the
                    // new one and lift the wrong key.
                    self.release_hw();
                    self.pad = gamepad::Pad::new(&ov);
                    self.overrides = ov;
                }
                Out::QuickSave => {
                    log::info!("SAVE tab");
                    let _ = self.input_tx.send(InputEvent::QuickSave);
                }
                Out::GuestTap(x, y) => {
                    let _ = self.input_tx.send(InputEvent::GuestTap { x, y });
                }
                Out::Plate(n) => {
                    let _ = self.input_tx.send(InputEvent::Plate(hz_java::PlateAsk::Press(n)));
                }
                Out::PlatePage => {
                    let _ = self.input_tx.send(InputEvent::Plate(hz_java::PlateAsk::Page));
                }
                Out::SfxLevel(db) => {
                    crate::audio_mix::set_sfx_db(db);
                    match config::write_sfx_db(&self.data_dir, db) {
                        Ok(()) => log::info!("sfx level -> {db} dB"),
                        Err(e) => log::warn!("sfx level: cannot write {}: {e}", config::SFX_LEVEL_FILE),
                    }
                }
                // B3: the platform's dialog (hero2's) — 복사, and 세이브 관리 under it in Java; a
                // host window on the PC.
                Out::OpenDiag => self.open_diag(),
                Out::Host(act) => self.host_act(act),
                Out::Fullscreen(on) => self.set_fullscreen(on),
                Out::Quit => self.exit_wanted.set(true),
                Out::World(open) => {
                    log::info!("world map -> {open}");
                    if open {
                        // Up first, then hold: nothing may stay pressed in a game that stands still.
                        self.release_hw();
                        self.world_next = Instant::now();
                    }
                    let _ = self.input_tx.send(InputEvent::Hold(open));
                }
            }
        }
        self.sync_target_shift();
    }

    /// A pad button or axis acted: the badges may appear, and the first time a notice.
    fn pad_input(&mut self) {
        if let Some(hud) = self.hud.as_mut() {
            hud.pad_input();
        }
        self.request_redraw();
    }

    /// A pad edge: a key to the game, or a top tab's press (`Hud::pad_tab` — SAVE and MAP have no
    /// release).
    fn pad_action(&mut self, action: gamepad::PadAction, down: bool) {
        match action.tab() {
            None => {
                if let Some(code) = action.key() {
                    self.send_key(code, down);
                }
            }
            Some(tab) if down => {
                let outs = self.hud.as_mut().map_or_else(Vec::new, |h| h.pad_tab(tab));
                self.apply(outs);
                self.request_redraw();
            }
            Some(_) => {}
        }
    }

    /// Keep the loop asleep until the notice should go away, then redraw without it — and, while
    /// the world map is open, wake every [`WORLD_FRAME`] for its animation (the game is held, so no
    /// frame of its would).
    fn schedule_toast(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut until = self.hud.as_ref().and_then(Hud::toast_until);
        // A held OK in the field attacks again (`Hud::fire_repeat_at`).
        let repeat = self.hud.as_mut().map(|h| h.fire_repeat_at(now)).unwrap_or_default();
        if !repeat.is_empty() {
            self.apply(repeat);
        }
        if let Some(at) = self.hud.as_ref().and_then(Hud::fire_next) {
            until = Some(until.map_or(at, |t| t.min(at)));
        }
        if let Some(t) = until
            && t <= now
        {
            if let Some(hud) = self.hud.as_mut() {
                hud.expire_notices(now);
            }
            // The stack's next change (another notice going, a slide ending) or the held OK's next
            // attack — whichever is still ahead comes first.
            let hud = self.hud.as_ref();
            until = [hud.and_then(Hud::toast_until), hud.and_then(Hud::fire_next)].into_iter().flatten().filter(|t| *t > now).min();
            self.request_redraw();
        }
        // The pad screen's prompt: its countdown and breath, and its running out.
        if let Some(h) = self.hud.as_mut()
            && h.remap_open()
        {
            h.remap_tick(now);
            if let Some(at) = h.remap_until() {
                until = Some(until.map_or(at, |t| t.min(at)));
                self.request_redraw();
            }
        }
        if self.hud.as_ref().is_some_and(|h| h.world().is_some()) {
            if now >= self.world_next {
                if let Some(hud) = self.hud.as_mut() {
                    hud.world_tick();
                }
                self.world_next = now + WORLD_FRAME;
                self.request_redraw();
            }
            until = Some(until.map_or(self.world_next, |t| t.min(self.world_next)));
        }
        // A redraw held back by `MIN_PRESENT`: now, or wake when it may go.
        if self.redraw_held.get() {
            let due = self.last_present.get() + MIN_PRESENT;
            if now >= due {
                self.redraw_now();
            } else {
                until = Some(until.map_or(due, |t| t.min(due)));
            }
        }
        event_loop.set_control_flow(match until {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn screen(&self) -> Screen {
        if self.desktop {
            Screen::new_desktop(self.win_w, self.win_h, self.guest_w, self.guest_h)
        } else {
            Screen::new(self.win_w, self.win_h, self.guest_w, self.guest_h)
        }
    }

    fn relayout(&mut self) {
        let screen = self.screen();
        let outs = match self.hud.as_mut() {
            Some(hud) => hud.set_screen(screen),
            None => Vec::new(),
        };
        self.apply(outs);
        if let Some(hud) = self.hud.as_ref() {
            self.cols = render::column_map(hud.layout().screen.game.w.max(0) as u32, self.guest_w);
        }
    }

    fn configure_surface(&mut self) {
        let (Some(context), Some(window)) = (self.context.as_ref(), self.window.as_ref()) else {
            return;
        };
        if self.surface.is_none() {
            match Surface::new(context, window.clone()) {
                Ok(s) => self.surface = Some(s),
                Err(e) => {
                    log::error!("Surface::new failed: {e}");
                    return;
                }
            }
        }
        if let (Some(w), Some(h), Some(surface)) = (NonZeroU32::new(self.win_w), NonZeroU32::new(self.win_h), self.surface.as_mut())
            && let Err(e) = surface.resize(w, h)
        {
            log::error!("surface resize failed: {e}");
        }
    }

    /// Present again — at once if the last present is [`MIN_PRESENT`] old, else when it is
    /// (`schedule_toast` wakes for it). Many asks before then are one present.
    fn request_redraw(&self) {
        if self.last_present.get().elapsed() >= MIN_PRESENT {
            self.redraw_now();
        } else {
            self.redraw_held.set(true);
        }
    }

    /// Present on the loop's next turn (a new game frame: never held back).
    fn redraw_now(&self) {
        self.redraw_held.set(false);
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let t0 = Instant::now();
        self.last_present.set(t0);
        self.redraw_inner();
        let us = t0.elapsed().as_micros() as u64;
        let g = &mut self.ui_gauge;
        g.0 += 1;
        g.1 += us;
        g.2 = g.2.max(us);
        if g.3.elapsed() >= std::time::Duration::from_secs(30) {
            log::info!("ui gauge: {} presents in {:.0} s, {:.2} ms a present, worst {:.1} ms ({}x{})", g.0, g.3.elapsed().as_secs_f64(), g.1 as f64 / 1000.0 / g.0.max(1) as f64, g.2 as f64 / 1000.0, self.win_w, self.win_h);
            self.ui_gauge = (0, 0, 0, Instant::now());
        }
    }

    fn redraw_inner(&mut self) {
        let (win_w, win_h) = (self.win_w, self.win_h);
        let Some(hud) = self.hud.as_ref() else { return };
        let Some(surface) = self.surface.as_mut() else { return };
        let mut buffer = match surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                log::error!("buffer_mut failed: {e}");
                return;
            }
        };
        if buffer.len() != (win_w * win_h) as usize {
            // The surface has not caught up with a resize yet; draw nothing rather than mis-index.
            return;
        }
        render::blit_game(
            &mut buffer,
            win_w,
            win_h,
            hud.layout().screen.game,
            &self.frame,
            self.guest_w,
            self.guest_h,
            &self.cols,
        );
        render::draw_hud(&mut buffer, win_w, win_h, hud);
        if let Err(e) = buffer.present() {
            log::error!("present failed: {e}");
        }
    }

    fn touch(&mut self, id: u64, phase: TouchPhase, x: f64, y: f64) {
        let Some(hud) = self.hud.as_mut() else { return };
        let outs = match phase {
            TouchPhase::Started => {
                hud.set_control(self.options.control());
                hud.started(id, x, y)
            }
            TouchPhase::Moved => hud.moved(id, x, y),
            TouchPhase::Ended => hud.ended(id),
            TouchPhase::Cancelled => hud.cancelled(id),
        };
        self.apply(outs);
    }

    fn release_all(&mut self) {
        let outs = self.hud.as_mut().map(Hud::release_all).unwrap_or_default();
        self.apply(outs);
        self.release_hw();
    }

    /// The pad's axes, its buttons and the keyboard's keys up. Focus loss is not a pause, so the
    /// worker does not release them, and a key-up the window no longer receives must not leave a
    /// key held in the game (or `hw_down` swallowing the next press).
    fn release_hw(&mut self) {
        for (action, down) in self.pad.release_all() {
            if let Some(code) = action.key() {
                self.send_key(code, down);
            }
        }
        for code in std::mem::take(&mut self.hw_down) {
            self.send_key(code, false);
        }
        self.pad_sent.clear();
        self.plate_route.clear();
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    // ------------------------------------------------------------------ hardware keys ----

    /// A hardware key (the keyboard, a phone's keys, a pad's buttons through winit): the pad's go
    /// through the player's remap, everything else through `keys`.
    pub fn hw_key(&mut self, physical: PhysicalKey, pressed: bool, repeat: bool) {
        // The PC's own keys (2026-10-29): F5 saves and M opens the world map (what the top tabs
        // do), F11 toggles full screen — on the press, never on a repeat, and never into a host
        // window or the settings (their keys are theirs).
        if self.desktop
            && pressed
            && !repeat
            && let PhysicalKey::Code(code) = physical
        {
            let panel = self.hud.as_ref().is_some_and(|h| h.host_open() || h.settings_open());
            match code {
                WinitKey::F11 => {
                    self.set_fullscreen(!self.fullscreen);
                    return;
                }
                WinitKey::F5 if !panel => {
                    self.pad_action(gamepad::PadAction::Save, true);
                    return;
                }
                WinitKey::KeyM if !panel => {
                    self.pad_action(gamepad::PadAction::Map, true);
                    return;
                }
                _ => {}
            }
        }
        // The PC's keyboard (사용자 2026-10-30): the arrows walk, X attacks and Space · Enter talk —
        // both the game's OK, the HUD's OK button (`hud::ok_key`) — Esc is the CLR button
        // (`hud::cancel_key`: the menu in the field, back in a window; back out of a host window,
        // the settings or the world map). WASD and Q/E are not the D-pad and soft keys here.
        if self.desktop
            && let PhysicalKey::Code(code) = physical
        {
            match code {
                WinitKey::KeyX | WinitKey::Space | WinitKey::Enter | WinitKey::NumpadEnter | WinitKey::Escape => {
                    let sent = match pressed {
                        true => self.desk_sent.iter().find(|(k, _)| *k == code).map(|(_, c)| *c).unwrap_or_else(|| {
                            let c = self.desk_key(code == WinitKey::Escape);
                            self.desk_sent.push((code, c));
                            c
                        }),
                        false => match self.desk_sent.iter().position(|(k, _)| *k == code) {
                            Some(i) => self.desk_sent.remove(i).1,
                            None => return,
                        },
                    };
                    self.key_press(sent, pressed, repeat);
                    return;
                }
                // HZ-058's plates (user 2026-10-30): Q W E R A S D, Tab the other page — on the press,
                // never on a repeat, never into a host window or the settings (their keys are theirs).
                WinitKey::KeyQ | WinitKey::KeyW | WinitKey::KeyE | WinitKey::KeyR | WinitKey::KeyA | WinitKey::KeyS | WinitKey::KeyD | WinitKey::Tab => {
                    let panel = self.hud.as_ref().is_some_and(|h| h.host_open() || h.settings_open() || h.world().is_some());
                    if pressed && !repeat && !panel {
                        const PLATES: [WinitKey; 7] = [WinitKey::KeyQ, WinitKey::KeyW, WinitKey::KeyE, WinitKey::KeyR, WinitKey::KeyA, WinitKey::KeyS, WinitKey::KeyD];
                        let ask = match PLATES.iter().position(|k| *k == code) {
                            Some(n) => hz_java::PlateAsk::Press(n as u8),
                            None => hz_java::PlateAsk::Page,
                        };
                        let _ = self.input_tx.send(InputEvent::Plate(ask));
                    }
                    return;
                }
                _ => {}
            }
        }
        match physical {
            PhysicalKey::Unidentified(NativeKeyCode::Android(c)) if gamepad::is_button(c) => self.pad_button(c, pressed, repeat),
            _ => {
                if let Some(code) = keys::map(physical) {
                    self.key_press(code, pressed, repeat);
                }
            }
        }
    }

    /// The game key the PC's OK (X · Space · Enter, `cancel` false) or CLR (Esc) sends now: what the
    /// HUD's OK and CLR buttons send in this scene — plain OK and back while a host window, the
    /// settings or the world map has the keyboard.
    fn desk_key(&self, cancel: bool) -> KeyCode {
        let Some(h) = self.hud.as_ref() else { return if cancel { KeyCode::RIGHT_SOFT_KEY } else { KeyCode::OK } };
        let panel = h.host_open() || h.settings_open() || h.world().is_some();
        match (cancel, panel) {
            (false, true) => KeyCode::OK,
            (false, false) => crate::hud::ok_key(h.view()),
            (true, true) => KeyCode::RIGHT_SOFT_KEY,
            (true, false) => crate::hud::cancel_key(h.view()),
        }
    }

    /// A pad button by its Android keycode (`gamepad::BUTTONS`): winit's on Android, gilrs's on
    /// the PC. The player's remap decides what it does; with the pad screen up a press is the
    /// screen's, never the game's (a release takes the usual path: a button held when the screen
    /// opened must still let go).
    pub fn pad_button(&mut self, code: u32, pressed: bool, repeat: bool) {
        if pressed && self.hud.as_ref().is_some_and(Hud::remap_open) {
            if !repeat && let Some(h) = self.hud.as_mut() {
                h.pad_button(code);
            }
            self.request_redraw();
            return;
        }
        let view = self.hud.as_ref().map(|h| h.view()).unwrap_or_default();
        let action = gamepad::button_edge(&mut self.pad_sent, &self.overrides, code, pressed, repeat, view);
        if pressed {
            self.pad_input();
        }
        let Some(action) = action else { return };
        self.pad_edge(code, action, pressed, repeat, view);
    }

    /// A pad function's edge — a button's (`source` its keycode), or an analog trigger's (L2/R2 are
    /// the same functions, `gamepad::key_for`; `source` `KEYCODE_BUTTON_L2`/`R2`): HZ-058's six quick
    /// functions press plates 1..6 where plates are used — the bare field, or a window's 「퀵슬롯등록」
    /// waiting for one — once, on the press; the rest is [`Self::action_edge`]. A release and a repeat
    /// go where their press went (`gamepad::PlateRoute`), not where the scene now says. A host window,
    /// the settings or the world map on top keeps the plates out, as the PC's Q…D (code review
    /// 2026-10-30: the game is held under the world map, so a plate asked there fired after it closed).
    fn pad_edge(&mut self, source: u32, action: gamepad::PadAction, pressed: bool, repeat: bool, view: hz_java::GuestView) {
        // In a window with pages the page functions (LB / RB) turn them (user 2026-10-30).
        let here = match self.hud.as_ref() {
            Some(h) if h.host_open() || h.settings_open() || h.world().is_some() => gamepad::Here::Keys,
            Some(h) if (view.scene == hz_java::Scene::Field && !view.popup && !view.notice && !view.window) || h.quick_bar().is_some_and(|b| b.picking) => {
                gamepad::Here::Plates
            }
            Some(_) if view.pager => gamepad::Here::Pages,
            _ => gamepad::Here::Keys,
        };
        match self.plate_route.edge(source, action, pressed, repeat, here) {
            gamepad::Route::Plate(n) => {
                let _ = self.input_tx.send(InputEvent::Plate(hz_java::PlateAsk::Press(n)));
            }
            gamepad::Route::PlatePage => {
                let _ = self.input_tx.send(InputEvent::Plate(hz_java::PlateAsk::Page));
            }
            gamepad::Route::Page(d) => {
                let _ = self.input_tx.send(InputEvent::Page(d));
            }
            gamepad::Route::Swallow => {}
            gamepad::Route::Key => self.action_edge(action, pressed, repeat),
        }
    }

    /// One axis sample from the pad (`UserEvent::GamepadAxes`, `live`; a resting one from
    /// [`Self::pad_lost`], which must not make the pad the input source the HUD shows badges for).
    fn pad_axes(&mut self, x: f32, y: f32, hx: f32, hy: f32, lt: f32, rt: f32, live: bool) {
        // Honest edges, like the touch stick's; the worker projects them — eight ways in the
        // bare field (HZ-059), where two arrows are a diagonal.
        let eight = self.hud.as_ref().is_some_and(|h| {
            let v = h.view();
            v.scene == hz_java::Scene::Field && !v.popup && !v.notice && !v.window && !h.remap_open() && h.world().is_none() && !h.host_open() && !h.settings_open()
        });
        let stick = self.pad.stick(x, y, hx, hy, eight);
        let (lt0, rt0) = (self.pad.lt, self.pad.rt);
        // A trigger given OK answers what A answers (`gamepad::view_key`).
        let view = self.hud.as_ref().map(|h| h.view()).unwrap_or_default();
        let edges = self.pad.trigger_edges(lt, rt, view);
        let triggers: Vec<(gamepad::PadAction, bool)> = edges.iter().map(|&(_, action, down)| (action, down)).collect();
        if live && (!stick.is_empty() || !triggers.is_empty()) {
            self.pad_input();
        }
        // The pad screen takes the presses (hero2's): an analog trigger as its L2 / R2 button,
        // the stick or hat as a direction; the releases still go to the game below, where the
        // latch lets go of what it had pressed before the screen opened.
        if self.hud.as_ref().is_some_and(Hud::remap_open) {
            // A trigger by its own state (it may have no key to send — switched off).
            let pulled = [(!lt0 && self.pad.lt, gamepad::KEYCODE_BUTTON_L2), (!rt0 && self.pad.rt, gamepad::KEYCODE_BUTTON_R2)];
            let dirs = stick.iter().filter(|(_, down)| *down).filter_map(|(k, _)| crate::touch::dpad_code(*k));
            let codes: Vec<u32> = dirs.chain(pulled.into_iter().filter(|(on, _)| *on).map(|(_, c)| c)).collect();
            if let Some(h) = self.hud.as_mut() {
                for c in codes {
                    h.pad_button(c);
                }
            }
            for (code, down) in stick {
                if !down {
                    self.send_key(code, false);
                }
            }
            for (source, action, down) in edges {
                if !down {
                    self.pad_edge(source, action, false, false, view);
                }
            }
            self.request_redraw();
            return;
        }
        // The open world map reads the stick and hat eight ways itself (a D-pad corner is
        // one sample the latch above makes one arrow) and takes the triggers' presses as
        // the keys the player gave them; the game is held.
        if self.hud.as_ref().is_some_and(|h| h.world().is_some()) {
            let (dx, dy) = (f64::from(x + hx).clamp(-1.0, 1.0), f64::from(y + hy).clamp(-1.0, 1.0));
            let outs = self.hud.as_mut().map_or_else(Vec::new, |h| h.world_pad(dx, dy, &triggers));
            self.apply(outs);
            self.request_redraw();
            return;
        }
        // A host window or the settings has the arrows (the PC — code review 2026-10-30: the stick
        // moved the game's menu behind 진단 정보): a press is theirs, as the D-pad's
        // (`action_edge`); a release still goes to the game, which may hold what the stick pressed
        // before the window opened.
        let panel = self.hud.as_ref().is_some_and(|h| h.host_open() || h.settings_open());
        for (code, down) in stick {
            if panel && down {
                self.action_edge(gamepad::PadAction::Key(code), true, false);
            } else {
                self.send_key(code, down);
            }
        }
        // A trigger is a pad function as a button is: a quick function presses its plate.
        for (source, action, down) in edges {
            self.pad_edge(source, action, down, false, view);
        }
    }

    /// The pad went away with a stick leaning, a trigger pulled or a button down (the PC: gilrs's
    /// `Disconnected`; Android: the input device removed) — no release would ever come. What the pad
    /// holds lets go through the paths that pressed it: a resting sample for the stick, hat and
    /// triggers, then each button still down. The keyboard's keys and the fingers keep theirs.
    pub fn pad_lost(&mut self) {
        log::info!("gamepad gone: letting go of what it held");
        self.pad_axes(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, false);
        let held: Vec<u32> = self.pad_sent.keys().copied().collect();
        for code in held {
            self.pad_button(code, false, false);
        }
        self.request_redraw();
    }

    /// The window has the input again and the pad's triggers read `lt`/`rt` (the PC: what gilrs read in
    /// the background) — one still pulled presses nothing until it is let go (`gamepad::Pad::resume_triggers`).
    pub fn pad_resume(&mut self, lt: f32, rt: f32) {
        self.pad.resume_triggers(lt, rt);
    }

    /// A key of the game's from the keyboard (or a pad's D-pad): the pad screen's arrows while it
    /// is up (a direction is no answer, the screen says so), else the game's.
    pub fn key_press(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if pressed
            && self.hud.as_ref().is_some_and(Hud::remap_open)
            && let Some(c) = crate::touch::dpad_code(code)
        {
            if !repeat && let Some(h) = self.hud.as_mut() {
                h.pad_button(c);
            }
            self.request_redraw();
            return;
        }
        self.action_edge(gamepad::PadAction::Key(code), pressed, repeat);
    }

    /// An action's edge from any hardware: a top tab's press (once, not again on the system's
    /// repeat), a host window's or the settings' key (the PC), the open world map's, else the
    /// game's key — pressed once and released once whatever the hardware repeats.
    fn action_edge(&mut self, action: gamepad::PadAction, pressed: bool, repeat: bool) {
        let Some(code) = action.key() else {
            if !repeat {
                self.pad_action(action, pressed);
            }
            return;
        };
        // A host window or the settings window has the keyboard: its keys, the rest swallowed; a
        // release still reaches the game below so nothing stays held.
        if pressed {
            let outs = self.hud.as_mut().and_then(|h| h.host_key(code).or_else(|| h.settings_key(code)));
            if let Some(outs) = outs {
                self.apply(outs);
                self.request_redraw();
                return;
            }
        }
        // The open world map takes the presses; a release of a key pressed before it opened still
        // goes to the game (below), and `release_hw` already let go of it.
        if pressed && self.hud.as_ref().is_some_and(|h| h.world().is_some()) {
            let outs = self.hud.as_mut().map_or_else(Vec::new, |h| h.world_key(code));
            self.apply(outs);
            self.request_redraw();
            return;
        }
        if !pressed && let Some(h) = self.hud.as_mut() {
            h.world_key_up(code);
        }
        match pressed {
            true if !self.hw_down.contains(&code) => {
                self.hw_down.push(code);
                self.send_key(code, true);
            }
            // Only what this path pressed: a button that went to the remap panel, or one already
            // let go by `release_hw`, must not lift a key a finger holds.
            false if self.hw_down.contains(&code) => {
                self.hw_down.retain(|c| *c != code);
                self.send_key(code, false);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------- the PC's host windows ----

    /// 진단 정보 (B3): the platform's dialog on Android (hero2's, `MainActivity`); a host window in
    /// the game's own look on the PC (the user's choice ④, 2026-10-29).
    fn open_diag(&mut self) {
        #[cfg(target_os = "android")]
        {
            crate::java::show_diagnostics(&self.diag_text());
        }
        #[cfg(not(target_os = "android"))]
        {
            let win = saves_ui::diag_window(&self.saves_ctx());
            self.open_host(win);
        }
    }

    fn diag_info(&self) -> saves_ui::DiagInfo {
        saves_ui::DiagInfo {
            version: self.version.clone(),
            window: (self.win_w * self.render_div, self.win_h * self.render_div),
            guest: (self.guest_w, self.guest_h),
            sfx_db: crate::audio_mix::sfx_db(),
            patches_file: self.data_dir.join(config::PATCHES_FILE).exists(),
        }
    }

    fn saves_ctx(&self) -> saves_ui::Ctx<'_> {
        saves_ui::Ctx {
            data_dir: &self.data_dir,
            ext_dir: self.ext_dir.as_deref().unwrap_or(&self.data_dir),
            app: self.app_code,
            game: crate::saves::game(),
            now_ms: now_ms(),
            fmt_time: self.fmt_time,
            diag: self.diag_info(),
        }
    }

    /// Put a host window up and have its run-time lines lettered — before the boot's queue, they are
    /// what is on screen.
    fn open_host(&mut self, win: HostWin) {
        if let Some(h) = self.hud.as_mut() {
            let asks = h.open_host(win);
            self.scene.ask_texts_first(asks);
        }
        self.request_redraw();
    }

    fn host_act(&mut self, act: HostAct) {
        let reply = saves_ui::perform(&self.saves_ctx(), act);
        match reply {
            saves_ui::Reply::Window(w) => self.open_host(w),
            saves_ui::Reply::Close => {
                if let Some(h) = self.hud.as_mut() {
                    h.close_host();
                }
                self.request_redraw();
            }
            saves_ui::Reply::Quit => self.exit_wanted.set(true),
            saves_ui::Reply::Folder(p) => open_folder(&p),
        }
    }

    // ------------------------------------------------------------- developer aids (PC) ----

    /// A click: finger 0 down and up at (`x`, `y`) in render space.
    #[cfg(not(target_os = "android"))]
    pub fn dev_click(&mut self, x: f64, y: f64) {
        self.touch(0, TouchPhase::Started, x, y);
        self.touch(0, TouchPhase::Ended, x, y);
        self.request_redraw();
    }

    /// `hz_desktop`'s `HZ_KEYS` script (a scripted session for a screenshot check, never a player's):
    /// `GEAR` clicks the gear, `ROW:n` the settings' row n, `HROW:n` a host window's row n, `CELL:n`
    /// the quick bar's cell n, `TAP:x:y` a point, `SHOT:name` writes the composed frame as
    /// `HZ_SHOT_DIR/name.ppm` (P6). Unknown words are logged and ignored.
    #[cfg(not(target_os = "android"))]
    pub fn dev_action(&mut self, cmd: &str) {
        let f: Vec<&str> = cmd.split(':').collect();
        let n = |i: usize| f.get(i).and_then(|v| v.trim().parse::<usize>().ok());
        let centre = |r: crate::hud::Rect| (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
        match f[0] {
            "GEAR" => {
                if let Some(g) = self.hud.as_ref().and_then(|h| h.layout().gear) {
                    let (x, y) = centre(g);
                    self.dev_click(x, y);
                } else {
                    log::warn!("dev: no gear now");
                }
            }
            "ROW" => {
                let rows = self.hud.as_ref().map(|h| h.settings_rows()).unwrap_or_default();
                match n(1).and_then(|i| rows.get(i).copied()) {
                    Some((r, _, _)) => {
                        let (x, y) = centre(r);
                        self.dev_click(x, y);
                    }
                    None => log::warn!("dev: no settings row {cmd}"),
                }
            }
            "HROW" => {
                let rows = self.hud.as_ref().and_then(|h| h.host().map(|w| crate::hud::host_rows(h.screen(), w.rows.len(), w.scroll))).unwrap_or_default();
                match n(1).and_then(|i| rows.iter().find(|(j, _)| *j == i).map(|(_, r)| *r)) {
                    Some(r) => {
                        let (x, y) = centre(r);
                        self.dev_click(x, y);
                    }
                    None => log::warn!("dev: no host row {cmd}"),
                }
            }
            "CELL" => {
                if let Some(i) = n(1).filter(|i| *i < 6) {
                    let (x, y) = centre(crate::hud::quick_cells(&self.screen())[i]);
                    self.dev_click(x, y);
                }
            }
            "TAP" => {
                if let (Some(x), Some(y)) = (n(1), n(2)) {
                    self.dev_click(x as f64, y as f64);
                }
            }
            "SHOT" => {
                let Some(dir) = std::env::var_os("HZ_SHOT_DIR") else { return };
                let Some(hud) = self.hud.as_ref() else { return };
                let (w, h) = (self.win_w, self.win_h);
                let mut buf = vec![0u32; (w * h) as usize];
                render::blit_game(&mut buf, w, h, hud.layout().screen.game, &self.frame, self.guest_w, self.guest_h, &self.cols);
                render::draw_hud(&mut buf, w, h, hud);
                let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                ppm.extend(buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
                let path = std::path::Path::new(&dir).join(format!("{}.ppm", f.get(1).unwrap_or(&"shot")));
                match std::fs::write(&path, ppm) {
                    Ok(()) => log::info!("dev: shot {}", path.display()),
                    Err(e) => log::warn!("dev: cannot write {}: {e}", path.display()),
                }
            }
            _ => log::warn!("dev: unknown action {cmd}"),
        }
    }

    /// The PC's window borderless full screen, or windowed again (FULLSCREEN, F11).
    fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        if let Some(w) = self.window.as_ref() {
            w.set_fullscreen(if on { Some(Fullscreen::Borderless(None)) } else { None });
        }
        if let Some(h) = self.hud.as_mut() {
            h.set_fullscreen(on);
        }
        self.request_redraw();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        self.set_paused(false);

        let mut attrs = Window::default_attributes().with_title("영웅서기 Zero");
        if self.desktop {
            // The PC's window (2026-10-29): 16:9 for the landscape canvas (1280×720 is the 426×240
            // picture at 3×), 3:4 for the portrait one; resizable (the user: 「리사이즈 허용」) — the
            // picture follows, the canvas does not (`crate::guest_size`).
            let landscape = self.boot.as_ref().is_none_or(|b| b.landscape);
            let (w, h) = if landscape { (1280.0, 720.0) } else { (600.0, 800.0) };
            attrs = attrs.with_inner_size(LogicalSize::new(w, h)).with_min_inner_size(LogicalSize::new(320.0, 240.0)).with_resizable(true);
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create_window failed: {e}");
                return;
            }
        };
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                log::error!("softbuffer Context::new failed: {e}");
                return;
            }
        };
        let size = window.inner_size();
        self.win_w = (size.width.max(1) / self.render_div).max(1);
        self.win_h = (size.height.max(1) / self.render_div).max(1);
        log::info!(
            "resumed: window {}x{} -> render {}x{} (heat saver div {})",
            size.width,
            size.height,
            self.win_w,
            self.win_h,
            self.render_div
        );
        self.window = Some(window);
        self.context = Some(context);
        self.surface = None;

        // First resume: size the guest from the real window, then start the worker.
        if let Some(boot) = self.boot.take() {
            let (gw, gh) = crate::guest_size(self.win_w, self.win_h, boot.landscape);
            self.guest_w = gw;
            self.guest_h = gh;
            self.frame = vec![0xff00_0000; (gw * gh) as usize];
            log::info!(
                "boot: window {}x{} landscape={} -> guest canvas {gw}x{gh}",
                self.win_w,
                self.win_h,
                boot.landscape
            );
            let mut hud = Hud::new(self.screen(), self.table.take().unwrap_or_default());
            hud.set_heat_saver(config::read_heat_saver(&self.data_dir));
            hud.set_pad_hud(config::read_pad_hud(&self.data_dir));
            hud.set_stats_on(config::read_stats(&self.data_dir));
            hud.set_minimap_on(config::read_minimap(&self.data_dir));
            hud.set_overrides(self.overrides.clone());
            hud.set_sfx_db(crate::audio_mix::sfx_db());
            hud.set_control(self.options.control());
            // The boot's restore, then the boot check (save-vault §13: from the files, before the
            // game has opened anything) — both last through the title, where the slot list is.
            match &boot.restored {
                crate::saves::Applied::None => {}
                crate::saves::Applied::Done(_) => hud.notice_through_title("SAVE RESTORED - THE ONE IT REPLACED IS IN THE VAULT"),
                crate::saves::Applied::Partial(_, _) => hud.notice_through_title("RESTORE INCOMPLETE - GEAR > DIAG"),
            }
            // The import's account, in the platform's dialog (hero2's): which slots came, which of this
            // phone's went to the vault — more than a notice line holds.
            if let Some(text) = boot.imported.as_deref() {
                #[cfg(target_os = "android")]
                crate::java::show_save_message(text);
                // The PC's host window: once the game's screen is up (`UserEvent::Guest`).
                #[cfg(not(target_os = "android"))]
                {
                    self.boot_message = Some(text.to_string());
                }
            }
            if let Some(s) = crate::saves::slots(&self.data_dir).iter().find(|s| s.missing_but_recoverable()) {
                log::warn!("saves: slot {} is gone and the vault has {} points for it", s.slot, s.points.len());
                // Slot by slot — HZ-020's ten (until 2026-10-26 every slot past 2 read 「세이브 3」).
                hud.notice_through_title(crate::host_text::SAVE_GONE[usize::from(s.slot.clamp(1, 10)) - 1]);
            }
            self.hud = Some(hud);
            emulator::spawn(emulator::Boot {
                proxy: boot.proxy,
                input_rx: boot.input_rx,
                jar: boot.jar,
                renditions: boot.renditions,
                options: boot.options,
                patches: config::read_patches(&self.data_dir),
                data_dir: self.data_dir.clone(),
                paused: self.paused.clone(),
                guest_w: gw,
                guest_h: gh,
                scene: boot.scene,
            });
        }

        self.relayout();
        self.configure_surface();
        self.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_wanted.get() {
            log::info!("quit asked (a host window)");
            event_loop.exit();
            return;
        }
        self.schedule_toast(event_loop);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("suspended: pausing the guest, dropping the surface");
        self.set_paused(true);
        // The worker releases what the guest holds when it sees the pause; the HUD forgets its
        // fingers so a resume does not resurrect a press.
        self.release_all();
        self.surface = None;
        self.context = None;
        self.window = None;
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Paint(frame) => {
                if frame.len() == (self.guest_w * self.guest_h) as usize {
                    self.frame = frame;
                    self.redraw_now();
                }
            }
            UserEvent::Guest(view) => {
                log::info!("scene: {view:?}");
                // 조작 방식 as the game's 「추가 설정」 left it (the field shows its walk control as
                // soon as the window closes, not at the next touch).
                let control = self.options.control();
                let outs = self
                    .hud
                    .as_mut()
                    .map(|h| {
                        h.set_control(control);
                        h.set_view(view)
                    })
                    .unwrap_or_default();
                self.apply(outs);
                if !matches!(view.scene, hz_java::Scene::Unknown | hz_java::Scene::Loading)
                    && let Some(text) = self.boot_message.take()
                {
                    self.open_host(saves_ui::message_window(&text));
                }
                self.request_redraw();
            }
            UserEvent::Stats(stats) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_stats(stats);
                }
                self.request_redraw();
            }
            UserEvent::Quick(bar) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_quick(bar);
                }
                self.request_redraw();
            }
            UserEvent::Map(map) => {
                log::info!("minimap: map {} {}x{}", map.id, map.w, map.h);
                if let Some(h) = self.hud.as_mut() {
                    h.set_map(map);
                }
                self.sync_target_shift();
                self.request_redraw();
            }
            UserEvent::Marks(marks) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_marks(marks);
                }
                self.request_redraw();
            }
            UserEvent::Portrait(portrait) => {
                log::info!("portrait: {:?}", portrait.as_ref().map(|p| p.key));
                if let Some(h) = self.hud.as_mut() {
                    h.set_portrait(portrait);
                }
                self.request_redraw();
            }
            UserEvent::Labels(labels) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_labels(labels);
                }
                self.request_redraw();
            }
            UserEvent::Pickups(pictures) => {
                if let Some(h) = self.hud.as_mut() {
                    for p in pictures {
                        h.pickup(p);
                    }
                }
                self.request_redraw();
            }
            UserEvent::StoryWait(seconds) => {
                log::info!("boss story countdown: {seconds}");
                if let Some(h) = self.hud.as_mut() {
                    h.countdown(crate::host_text::story_wait(seconds));
                }
                self.request_redraw();
            }
            UserEvent::Texts(texts) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_texts(texts);
                }
                self.request_redraw();
            }
            UserEvent::Guide(g) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_guide(g);
                }
                self.request_redraw();
            }
            UserEvent::PlaceMaps(maps) => {
                if let Some(h) = self.hud.as_mut() {
                    h.set_place_maps(maps);
                }
                self.request_redraw();
            }
            UserEvent::BarOff(off) => {
                log::info!("game bar off (HZ-013): {off}");
                if let Some(h) = self.hud.as_mut() {
                    h.set_bar_off(off);
                }
                self.request_redraw();
            }
            UserEvent::GamepadAxes { x, y, hx, hy, lt, rt } => self.pad_axes(x, y, hx, hy, lt, rt, true),
            UserEvent::GamepadLost => self.pad_lost(),
            UserEvent::Notice(text) => {
                log::warn!("notice: {text}");
                if let Some(h) = self.hud.as_mut() {
                    // The unreadable save is told where the game shows that slot as empty.
                    if text == crate::database::SAVE_UNREADABLE {
                        h.notice_through_title(text);
                    } else {
                        h.notice(text);
                    }
                }
                self.request_redraw();
            }
            UserEvent::GuestExit => {
                log::info!("the game asked to exit");
                #[cfg(target_os = "android")]
                crate::java::quit_app();
                event_loop.exit();
            }
            UserEvent::GuestStopped => {
                log::error!("the game stopped");
                if let Some(h) = self.hud.as_mut() {
                    h.notice_stuck("THE GAME STOPPED - SAVES ARE SAFE - RESTART THE APP");
                }
                self.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.win_w = (size.width.max(1) / self.render_div).max(1);
                self.win_h = (size.height.max(1) / self.render_div).max(1);
                log::info!("resized: {}x{} -> render {}x{}", size.width, size.height, self.win_w, self.win_h);
                self.relayout();
                self.configure_surface();
                self.request_redraw();
            }
            // The notification shade over the game: the app is still resumed, but no Ended will
            // come for a finger that was down.
            WindowEvent::Focused(false) => {
                self.release_all();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Touch(t) => {
                // Physical -> render space (HEAT SAVER divisor; 1 = identity).
                let div = f64::from(self.render_div);
                self.touch(t.id, t.phase, t.location.x / div, t.location.y / div);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.hw_key(event.physical_key, event.state == ElementState::Pressed, event.repeat);
            }
            // The PC's mouse (2026-10-29): the left button is one finger on the touch HUD's chain —
            // the quick bar, the tabs, the gear, the game's own windows (`menu_touch`) — the right
            // button the game's back key, the wheel a host window's or the settings' list.
            WindowEvent::CursorMoved { position, .. } => {
                let div = f64::from(self.render_div);
                let at = (position.x / div, position.y / div);
                self.mouse = Some(at);
                if self.mouse_down {
                    self.touch(0, TouchPhase::Moved, at.0, at.1);
                    self.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if self.mouse_down {
                    self.mouse_down = false;
                    self.touch(0, TouchPhase::Cancelled, 0.0, 0.0);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if let Some((x, y)) = self.mouse {
                    let down = state == ElementState::Pressed;
                    if down != self.mouse_down {
                        self.mouse_down = down;
                        self.touch(0, if down { TouchPhase::Started } else { TouchPhase::Ended }, x, y);
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => {
                self.key_press(KeyCode::RIGHT_SOFT_KEY, state == ElementState::Pressed, false);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y.round() as i32,
                    MouseScrollDelta::PixelDelta(p) => -(p.y / 40.0).round() as i32,
                };
                if rows != 0
                    && let Some(h) = self.hud.as_mut()
                {
                    h.wheel(rows);
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }
}
