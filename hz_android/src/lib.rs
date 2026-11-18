//! 영웅서기 Zero on Android — the host around the J2ME core.
//!
//! Three threads, two channels (the shape hero2's `wie_android` proved):
//!
//! * the **UI thread** (winit / GameActivity) owns the window, the softbuffer surface and the
//!   touch controls. It never touches the JVM;
//! * the **emulator worker** ([`emulator`]) owns the `J2MEEmulator` and everything inside it.
//!   It receives [`InputEvent`]s and sends finished frames back as [`UserEvent::Paint`];
//! * the JVM's own threads are not OS threads — they are tasks on the worker's executor.
//!
//! Phase 1 (`HEROZERO_REMAKE_PLAN.md` §8): boots, draws, saves, takes touch and key input.
//! Phase 2: the touch HUD — [`hud`] (geometry), [`placement`] (the player's layout), [`touch`] (the
//! state machine) and [`render`], all host-tested; `app` is the glue.
//! Phase 4a (2026-10-22): the WAV effects — [`audio`] (bus + sink), [`audio_mix`], [`sfx_upsample`],
//! host-tested; the rodio output is the Android-only part of `audio`.
//! Phase 4c (2026-10-22): the AAC music — [`music`] (the wie-music thread and the bus's end of it),
//! [`music_air`] (the top octave the codec cut, put back) and [`music_resample`], host-tested. The
//! one MIDI song (4b) is still silent.
//! Phase 2b remainder (2026-10-22): the gamepad's remap file ([`gamepad`]), its position badges
//! on the controls ([`padhud`]) and the host's toast line ([`touch::Hud::notice`]).

pub mod audio;
pub mod audio_mix;
pub mod config;
pub mod database;
pub mod font;
pub mod gamepad;
pub mod host_text;
pub mod hud;
pub mod hud_view;
pub mod keys;
/// hero2's `minimap_view.rs`, byte for byte (md5 `d67e68d0892d9e7917a2b56b2924ce7d`, 2026-10-23):
/// the hero5-style panel — floor, outline, exit bars, chest boxes, monster and player squares,
/// a fixed zoom that follows the player (Phase 6-5). Its tile classes are `hz_java`'s.
pub mod minimap_view;
pub mod music;
pub mod music_air;
pub mod music_resample;
pub mod padhud;
/// The pad screen's rules (B6) — hero2's and hero3's file, byte for byte.
pub mod padmap;
pub mod placement;
pub mod render;
pub mod save_transfer;
pub mod saves;
pub mod sfx_upsample;
pub mod touch;
pub mod ui_icons;
pub mod vault;
pub mod vibration;
pub mod worldmap_view;

/// The UI thread (winit) — on Android under GameActivity, on the PC in `hz_desktop`'s window
/// (2026-10-29: the same `App`, the touch HUD's chain fed by the mouse, the keyboard as before).
//
// ⛔ Android and the MSVC desktop build only, not the windows-gnu host (the probe, `cargo test` with
// the dlltool shim): with these three linked, the gnu test exe died at load with 0xC0000005 before
// `main` — the shim's raw-dylib import libs collide with mingw's own kernel32 (measured 2026-10-29:
// HEAD's test exe listed 263 tests, the same exe with `app` in it listed none). The modules
// themselves are platform-neutral; `hz_desktop` is built with MSVC (`build_desktop.ps1`).
#[cfg(any(target_os = "android", target_env = "msvc"))]
pub mod app;
/// The emulator worker: platform-neutral (its `Platform` is files, channels and the winit proxy).
#[cfg(any(target_os = "android", target_env = "msvc"))]
pub mod emulator;
/// A host window in the game's own look (the PC's stand-in for `MainActivity`'s dialogs).
pub mod hostwin;
#[cfg(target_os = "android")]
mod java;
#[cfg(any(target_os = "android", target_env = "msvc"))]
mod platform;
/// 진단 정보 · 세이브 관리 · 세이브 옮기기 as host windows (`hostwin`), and what their rows do.
pub mod saves_ui;

use wie_backend::KeyCode;

// The view's tile classes are the hook's (`hz_java::state`), repeated because the view is hero2's
// file byte for byte. A drift would draw floor as wall or exits as nothing — silently — so it is a
// build error instead (hero2's `overlay.rs` makes the same check).
const _: () = assert!(
    minimap_view::MM_WALL == hz_java::state::MM_WALL
        && minimap_view::MM_FLOOR == hz_java::state::MM_FLOOR
        && minimap_view::MM_EXIT == hz_java::state::MM_EXIT
        && minimap_view::MM_CHEST == hz_java::state::MM_CHEST
);

/// The asset the APK carries, and the name the jar is mounted under. `J2MEEmulator::from_jar`
/// keys the RecordStores (the saves) with it, so it is a CONSTANT, the same one `hz_probe` uses
/// (`hz_probe::JAR_NAME`): renaming it would make every existing save look like a fresh install.
pub const JAR_NAME: &str = "herozero.jar";

/// The fanfare `build_android.ps1` renders from the jar's `/snd/b/15` with `hz_render` (Phase 4b).
/// Optional: a build without the SoundFont ships none, and that one MIDI is silent.
pub const FANFARE_NAME: &str = "hz_fanfare.wav";

/// The world map's painted picture (Phase 7-4), generated at build time by
/// `tools_re/hz_worldmap_draw.py --rgb565` (`worldmap_view::wm_parse_picture`'s format). Optional:
/// without it the map draws a flat sea.
pub const WORLDMAP_NAME: &str = "hz_worldmap.rgb565";

/// UI-thread events, sent from the worker through the winit proxy.
#[derive(Debug)]
pub enum UserEvent {
    /// A finished guest frame, 0xAARRGGBB, guest framebuffer size.
    Paint(Vec<u32>),
    /// What the game is showing changed (the HZ-008 blit hook, `hz_java::state`): the HUD
    /// shows the number keys only where the game listens to them.
    Guest(hz_java::GuestView),
    /// The player's numbers changed (the same hook, `hz_java::state::Stats`): the HUD's readout.
    Stats(hz_java::Stats),
    /// Sent once at boot: whether HZ-013 (the game's bottom bar off) is among the chosen patches.
    BarOff(bool),
    /// The quick slots changed (Phase 6-2, the same hook: `hz_java::state::QuickBar`) — what the
    /// plates show; sent on every change, cooldown ticks included.
    Quick(hz_java::QuickBar),
    /// The map changed (Phase 6-5: another map, a chest opened — `hz_java::state::MapGrid`).
    Map(hz_java::MapGrid),
    /// The player or a monster moved on it (`hz_java::state::MapMarks`).
    Marks(hz_java::MapMarks),
    /// The status block's portrait changed (Phase 6-6: another helmet, the buff head —
    /// `hz_java::portrait`); `None` = the doll could not be composed.
    Portrait(Option<hz_java::Icon>),
    /// The world map's names in the game's own font (Phase 7-3), once:
    /// `worldmap_view::wm_label_lines` order.
    Labels(Vec<Option<hz_java::Icon>>),
    /// The host's own lines in the game's font (B1, `host_text`): numbered pictures as they are
    /// lettered, a few a frame from the title on (`None` = could not be).
    Texts(Vec<(u16, Option<hz_java::Icon>)>),
    /// The game's own pickup notices (HZ-025: 「귀환서」, 「N금」 — its pictures), as it first draws
    /// them, for the host's notice stack.
    Pickups(Vec<hz_java::Icon>),
    /// HZ-061: a boss's story counting down — the seconds left (0 = it started, or its wait was
    /// dropped), for the notice stack's held line (`host_text::story_wait`).
    StoryWait(u8),
    /// Every world-map place's own map, read from the jar by the game's loader (Phase 7-5), once:
    /// `WM_PLACES` order.
    PlaceMaps(Vec<Option<hz_java::MapGrid>>),
    /// The game's quest guidance changed (Phase 7-6: a quest moved on, or the player changed maps).
    Guide(hz_java::QuestGuide),
    /// A gamepad axis sample from `MainActivity.dispatchGenericMotionEvent` over JNI (the only
    /// route joystick motion has into this process — see `gamepad`).
    GamepadAxes { x: f32, y: f32, hx: f32, hy: f32, lt: f32, rt: f32 },
    /// The pad went away (Android: an input device that sent stick motion removed; the PC: gilrs's
    /// `Disconnected`) — no release will come for what it held (`App::pad_lost`).
    GamepadLost,
    /// Something the player must hear from the host because the game will not say it (a save
    /// that failed — `database`): the HUD's notice line, ASCII.
    Notice(&'static str),
    /// The MIDlet called `notifyDestroyed` (the game's own 「종료」).
    GuestExit,
    /// The worker stopped on an error (the game threw, or would not load): nothing will draw
    /// again, and the player must be told rather than left looking at the last frame.
    GuestStopped,
}

/// Worker-bound input, from the UI thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key { code: KeyCode, down: bool },
    /// HZ-014: how many guest px the game's target-monster panel moves left to clear the minimap
    /// (`touch::Hud::target_shift`), for the hook to read.
    TargetShift(i32),
    /// HZ-054: how many guest px lower the panel goes, under the portrait minimap
    /// (`touch::Hud::target_place`).
    TargetDrop(i32),
    /// The SAVE tab (Phase 7-1): the next field frame opens the game's own quick save.
    QuickSave,
    /// The world map is open (Phase 7-2): the game stands still — no ticks — while the music,
    /// which has its own thread, plays on.
    Hold(bool),
    /// A tap on the game's picture, guest px: the next frame does what the keys would in the
    /// game's own windows (`hz_java::menu_touch`).
    GuestTap { x: i32, y: i32 },
    /// HZ-058: a quick plate pressed or the page turned — the next drawn frame acts on it
    /// (`hz_java::GuestState::ask_plate`).
    Plate(hz_java::PlateAsk),
    /// A pad's page button in a window with pages, −1 back · +1 on — the next drawn frame turns it
    /// (`hz_java::GuestState::ask_page`).
    Page(i8),
}

/// The worker while the app is backgrounded or locked: what the UI sent is taken off the
/// channel so it cannot back up, and the keys and taps are dropped (the worker lets go of
/// everything anyway), but the rest is state the UI has already moved to — it waits in `pending` for the
/// first loop after the resume. Dropping a `Hold(false)` sent just before the app went away left
/// the game standing still behind a closed world map; dropping a `Hold(true)` let it run behind
/// an open one (review 2026-10-23). `false` once the UI's end of the channel is gone.
pub fn drain_while_paused(pending: &mut std::collections::VecDeque<InputEvent>, rx: &std::sync::mpsc::Receiver<InputEvent>) -> bool {
    use std::sync::mpsc::TryRecvError;

    pending.retain(|ev| !matches!(ev, InputEvent::Key { .. } | InputEvent::GuestTap { .. } | InputEvent::Plate(_) | InputEvent::Page(_)));
    loop {
        match rx.try_recv() {
            Ok(InputEvent::Key { .. } | InputEvent::GuestTap { .. } | InputEvent::Plate(_) | InputEvent::Page(_)) => {}
            Ok(ev) => pending.push_back(ev),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

/// The guest's height in landscape. The game re-lays itself out for whatever canvas it is given
/// (probe, 2026-10-22: 320/427/496/520 x 240 all draw the field, the menus and the bottom bar
/// correctly), so the width follows the window's aspect and the picture fills the screen with no
/// letterbox and no distortion. 240 because that is the game's own short side.
pub const LANDSCAPE_H: u32 = 240;
/// Bounds for the derived landscape width. 640 is past every phone measured (a 2.17:1 S25 Ultra
/// asks for 520); a wider window letterboxes rather than give the game a canvas nobody has
/// looked at.
pub const LANDSCAPE_W_MIN: u32 = 320;
pub const LANDSCAPE_W_MAX: u32 = 640;
/// Portrait: the game's native canvas. The controls live in the space below it.
pub const PORTRAIT_SIZE: (u32, u32) = (240, 320);

/// The guest canvas for a window. Decided ONCE, before the emulator starts: the game allocates
/// its back buffer from `getWidth()`/`getHeight()` in a static initialiser and its
/// `sizeChanged` is empty (javap, 2026-10-22), so a later change of canvas would not reach it.
///
/// `landscape` is the PREFERENCE, not `win_w > win_h`: the first `resumed` after
/// `setRequestedOrientation` can still report the pre-rotation size (measured on hero2,
/// 2026-07-28), so the aspect comes from the long and short sides.
pub fn guest_size(win_w: u32, win_h: u32, landscape: bool) -> (u32, u32) {
    let (long, short) = (win_w.max(win_h), win_w.min(win_h));
    if !landscape || short == 0 {
        return PORTRAIT_SIZE;
    }
    // Even width: harmless for the game, and keeps rows tidy for the blit.
    let w = ((u64::from(LANDSCAPE_H) * u64::from(long) / u64::from(short)) as u32).clamp(LANDSCAPE_W_MIN, LANDSCAPE_W_MAX) & !1;
    (w, LANDSCAPE_H)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use std::sync::{Arc, atomic::AtomicBool, mpsc::channel};

    use winit::{event_loop::EventLoop, platform::android::EventLoopBuilderExtAndroid};

    // A developer's `files/hz_debug.txt` (any content, `run-as` on a debuggable build) turns the
    // host's debug lines on — per-key latency samples and the like; players never have it.
    let debug = app.internal_data_path().is_some_and(|d| d.join(config::DEBUG_FILE).exists());
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(if debug { log::LevelFilter::Debug } else { log::LevelFilter::Info })
            .with_filter(android_logger::FilterBuilder::new().parse(if debug { "warn,main=debug,hz=debug" } else { "warn,main=info,hz=info" }).build())
            .with_tag("hz"),
    );
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));
    log::info!("android_main: 영웅서기 Zero host starting");

    let Some(jar) = read_asset(&app, JAR_NAME) else {
        log::error!("bundled game asset {JAR_NAME} not found; aborting");
        return;
    };
    log::info!("loaded asset {JAR_NAME} ({} bytes)", jar.len());
    let renditions: Vec<Arc<[u8]>> = match read_asset(&app, FANFARE_NAME) {
        Some(wav) => {
            log::info!("loaded asset {FANFARE_NAME} ({} bytes)", wav.len());
            vec![Arc::from(wav)]
        }
        None => {
            log::warn!("no {FANFARE_NAME} asset: the MIDI fanfare will be silent");
            Vec::new()
        }
    };
    match read_asset(&app, WORLDMAP_NAME).and_then(|raw| worldmap_view::wm_parse_picture(&raw)) {
        Some(p) => {
            log::info!("loaded asset {WORLDMAP_NAME} ({}x{})", p.w, p.h);
            worldmap_view::wm_set_picture(p);
        }
        None => log::warn!("no {WORLDMAP_NAME} asset: the world map draws a flat sea"),
    }
    let Some(data_dir) = app.internal_data_path() else {
        log::error!("no internal_data_path; aborting");
        return;
    };
    log::info!("internal data dir = {data_dir:?}");
    let ext_dir = app.external_data_path();
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    // A 세이브 옮기기 import the player confirmed last run (B4), before the game opens any store —
    // staging one cancelled any restore, and asking for a restore cancels a staged import.
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
    // A restore the player confirmed last run, before the game opens any store (save-vault §9).
    let restored = saves::apply_pending(&data_dir, now_ms);
    match &restored {
        saves::Applied::None => {}
        saves::Applied::Done(stores) => log::warn!("saves: restored {stores:?}"),
        saves::Applied::Partial(stores, why) => log::error!("saves: restore incomplete ({why}); put back {stores:?}"),
    }

    // The player's 「추가 설정」 options (hz_java). The orientation is the one THIS run uses; a
    // change made in game is written for the next launch.
    let options = Arc::new(hz_java::GameOptions::load(&data_dir));
    let landscape = options.landscape_now();
    java::request_orientation(landscape);

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_android_app(app)
        .build()
        .expect("failed to build android event loop");
    let proxy = event_loop.create_proxy();
    // The JNI natives below relay into the loop through this.
    let _ = GAMEPAD_PROXY.set(std::sync::Mutex::new(event_loop.create_proxy()));

    let (input_tx, input_rx) = channel::<InputEvent>();
    let paused = Arc::new(AtomicBool::new(false));
    let boot = app::Boot {
        proxy,
        input_rx,
        jar,
        renditions,
        landscape,
        options,
        ext_dir,
        restored,
        imported,
        desktop: false,
        scene: Arc::new(hz_java::GuestState::new()),
        // The transfer dialogs are Java's here (they pass the versionCode themselves).
        app_code: 0,
        version: String::new(),
        fmt_time: saves_ui::utc_stamp,
    };
    let mut handler = app::App::new(input_tx, data_dir, paused, boot);
    if let Err(e) = event_loop.run_app(&mut handler) {
        log::error!("event loop exited with error: {e}");
    }
}

/// Set once in `android_main`, used by the JNI natives (which run on the Java main thread and
/// must only relay).
#[cfg(target_os = "android")]
static GAMEPAD_PROXY: std::sync::OnceLock<std::sync::Mutex<winit::event_loop::EventLoopProxy<UserEvent>>> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn send_gamepad_event(ev: UserEvent) {
    if let Some(proxy) = GAMEPAD_PROXY.get()
        && let Ok(proxy) = proxy.lock()
    {
        // Fails only while the loop is shutting down.
        let _ = proxy.send_event(ev);
    }
}

/// The `native` method `MainActivity` declares, resolved by SYMBOL NAME when Java calls it
/// (`System.loadLibrary("main")` has loaded this cdylib): the JNI naming convention in the
/// function name and `extern "system"` are both load-bearing.
#[cfg(target_os = "android")]
mod gamepad_jni {
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeGamepadAxes(
        _env: *mut jni::sys::JNIEnv,
        _class: jni::sys::jclass,
        x: jni::sys::jfloat,
        y: jni::sys::jfloat,
        hx: jni::sys::jfloat,
        hy: jni::sys::jfloat,
        lt: jni::sys::jfloat,
        rt: jni::sys::jfloat,
    ) {
        crate::send_gamepad_event(crate::UserEvent::GamepadAxes { x, y, hx, hy, lt, rt });
    }

    /// `MainActivity.onInputDeviceRemoved` for a pad that sent stick motion: Android sends no
    /// resting sample after it, so the stick, hat and triggers would stay where they were.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeGamepadLost(_env: *mut jni::sys::JNIEnv, _class: jni::sys::jclass) {
        crate::send_gamepad_event(crate::UserEvent::GamepadLost);
    }
}

/// `MainActivity`'s 진단 정보 › 세이브 관리 (B3, hero2's `vault_jni`). Java hands over its own
/// `getFilesDir()` — the directory `android_main` was given — so each answer is a plain function of
/// the files (`crate::saves`), read when the dialog asks. They run on the Java main thread while the
/// game runs: they only read the vault and write the restore request, which the next launch applies.
#[cfg(target_os = "android")]
mod saves_jni {
    use jni::{
        JNIEnv,
        objects::{JObject, JString},
        sys::{jboolean, jclass, jstring},
    };

    /// A panic must not cross into Java (it would abort the process): log it, answer `fallback`.
    fn guard<T>(what: &str, fallback: T, body: impl FnOnce() -> T) -> T {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).unwrap_or_else(|_| {
            log::error!("{what}: panicked — answering the safe value instead");
            fallback
        })
    }

    fn read(env: &mut JNIEnv, s: jstring) -> Option<String> {
        if s.is_null() {
            return None;
        }
        let s: JString = unsafe { JObject::from_raw(s) }.into();
        env.get_string(&s).ok().map(Into::into)
    }

    fn answer(env: &mut JNIEnv, text: &str) -> jstring {
        env.new_string(text).map_or(std::ptr::null_mut(), |s| s.into_raw())
    }

    /// `String nativeRestoreList(String filesDir)` — `slot|ms|bytes|gone` lines (`saves::offer_lines`).
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeRestoreList(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
    ) -> jstring {
        guard("nativeRestoreList", std::ptr::null_mut(), || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return std::ptr::null_mut() };
            let lines = read(&mut env, files_dir).map(|d| crate::saves::offer_lines(std::path::Path::new(&d))).unwrap_or_default();
            answer(&mut env, &lines)
        })
    }

    /// `boolean nativeRestoreRequest(String filesDir, String key)` — schedule the point a list line
    /// names for the next launch (`saves::request_offer`); the dialog then offers to close the app.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeRestoreRequest(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
        key: jstring,
    ) -> jboolean {
        guard("nativeRestoreRequest", 0, || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return 0 };
            let (Some(dir), Some(key)) = (read(&mut env, files_dir), read(&mut env, key)) else { return 0 };
            match crate::saves::request_offer(std::path::Path::new(&dir), &key) {
                Ok(slot) => {
                    log::warn!("saves: restore of slot {slot} ({key}) set for the next launch");
                    1
                }
                Err(e) => {
                    log::error!("saves: restore {key:?} refused: {e}");
                    0
                }
            }
        })
    }

    /// 세이브 옮기기 (B4): `ok…` or `err|<a sentence for the player>`.
    fn reply(r: Result<String, String>) -> String {
        r.unwrap_or_else(|e| format!("err|{e}"))
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
    }

    /// `String nativeTransferExport(String filesDir, String outPath, int app)` — this phone's transfer
    /// file at `outPath` (`saves::export_to`): `ok|<slots, comma separated>`.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeTransferExport(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
        out_path: jstring,
        app: jni::sys::jint,
    ) -> jstring {
        guard("nativeTransferExport", std::ptr::null_mut(), || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return std::ptr::null_mut() };
            let r = match (read(&mut env, files_dir), read(&mut env, out_path)) {
                (Some(dir), Some(out)) => crate::saves::export_to(std::path::Path::new(&dir), crate::saves::game(), app.max(0) as u32, std::path::Path::new(&out), now_ms())
                    .map(|slots| format!("ok|{}", slots.iter().map(u8::to_string).collect::<Vec<_>>().join(","))),
                _ => Err("앱의 저장 위치를 알 수 없습니다.".to_string()),
            };
            answer(&mut env, &reply(r))
        })
    }

    /// `String nativeTransferCheck(String filesDir, String inPath, int app)` — what the picked file
    /// carries and what this phone holds (`saves::import_summary`).
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeTransferCheck(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
        in_path: jstring,
        app: jni::sys::jint,
    ) -> jstring {
        guard("nativeTransferCheck", std::ptr::null_mut(), || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return std::ptr::null_mut() };
            let r = match (read(&mut env, files_dir), read(&mut env, in_path)) {
                (Some(dir), Some(p)) => crate::saves::import_summary(std::path::Path::new(&dir), crate::saves::game(), app.max(0) as u32, std::path::Path::new(&p)),
                _ => Err("앱의 저장 위치를 알 수 없습니다.".to_string()),
            };
            answer(&mut env, &reply(r))
        })
    }

    /// `String nativeTransferSchedule(String filesDir, String inPath, int app)` — stage the picked
    /// file for the next launch (`saves::stage_import`): `ok`.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeTransferSchedule(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
        in_path: jstring,
        app: jni::sys::jint,
    ) -> jstring {
        guard("nativeTransferSchedule", std::ptr::null_mut(), || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return std::ptr::null_mut() };
            let r = match (read(&mut env, files_dir), read(&mut env, in_path)) {
                (Some(dir), Some(p)) => crate::saves::stage_import(std::path::Path::new(&dir), crate::saves::game(), app.max(0) as u32, std::path::Path::new(&p)).map(|()| "ok".to_string()),
                _ => Err("앱의 저장 위치를 알 수 없습니다.".to_string()),
            };
            answer(&mut env, &reply(r))
        })
    }

    /// `String nativeSaveExport(String filesDir, String extDir)` — copy every save and the vault
    /// out (`saves::export_sentence`) and say what happened; `extDir` null = no external storage.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_net_wie_emulator_MainActivity_nativeSaveExport(
        env: *mut jni::sys::JNIEnv,
        _class: jclass,
        files_dir: jstring,
        ext_dir: jstring,
    ) -> jstring {
        guard("nativeSaveExport", std::ptr::null_mut(), || {
            let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else { return std::ptr::null_mut() };
            let Some(dir) = read(&mut env, files_dir) else { return std::ptr::null_mut() };
            let ext = read(&mut env, ext_dir).map(std::path::PathBuf::from);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
            let text = crate::saves::export_sentence(std::path::Path::new(&dir), ext.as_deref(), now);
            answer(&mut env, &text)
        })
    }
}

#[cfg(target_os = "android")]
fn read_asset(app: &winit::platform::android::activity::AndroidApp, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;

    let cname = std::ffi::CString::new(name).ok()?;
    let mut asset = app.asset_manager().open(&cname)?;
    let mut buf = Vec::new();
    asset.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_follows_the_window_aspect_at_height_240() {
        assert_eq!(guest_size(3120, 1440, true), (520, 240), "S25 Ultra");
        assert_eq!(guest_size(2960, 1440, true), (492, 240), "Galaxy S9, even width");
        assert_eq!(guest_size(1920, 1080, true), (426, 240));
    }

    #[test]
    fn the_preference_decides_not_the_transient_window_shape() {
        // First `resumed` before the rotation has landed: portrait-shaped, landscape wanted.
        assert_eq!(guest_size(1440, 3120, true), (520, 240));
        assert_eq!(guest_size(3120, 1440, false), PORTRAIT_SIZE);
    }

    #[test]
    fn a_pause_drops_the_keys_but_keeps_the_world_maps_hold() {
        use std::{collections::VecDeque, sync::mpsc::channel};

        let key = |down| InputEvent::Key { code: KeyCode::OK, down };
        // The map closed (and the minimap moved) just before the app went away, with keys around.
        let mut pending = VecDeque::from([key(true), InputEvent::Hold(true)]);
        let (tx, rx) = channel();
        for ev in [key(false), InputEvent::Hold(false), InputEvent::GuestTap { x: 260, y: 55 }, InputEvent::TargetShift(69), key(true), InputEvent::QuickSave] {
            tx.send(ev).unwrap();
        }
        assert!(drain_while_paused(&mut pending, &rx));
        assert_eq!(
            Vec::from(pending.clone()),
            vec![InputEvent::Hold(true), InputEvent::Hold(false), InputEvent::TargetShift(69), InputEvent::QuickSave],
            "in order, so the last hold wins on the resume"
        );
        assert!(drain_while_paused(&mut pending, &rx), "nothing new: still connected");
        assert_eq!(pending.len(), 4, "a second pass keeps what is waiting");
        drop(tx);
        assert!(!drain_while_paused(&mut pending, &rx), "the UI is gone");
    }

    #[test]
    fn extreme_aspects_are_clamped() {
        assert_eq!(guest_size(4000, 1000, true), (LANDSCAPE_W_MAX, 240));
        assert_eq!(guest_size(1000, 1000, true), (LANDSCAPE_W_MIN, 240));
        assert_eq!(guest_size(0, 0, true), PORTRAIT_SIZE);
    }
}
