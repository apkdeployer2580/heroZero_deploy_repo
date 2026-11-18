//! The emulator worker: owns the `J2MEEmulator` (and so the JVM and every guest thread), turns UI
//! input into guest events, ticks, and paces itself to the guest's own timers.
//!
//! The loop's shape is hero2's (`wie_android/src/emulator.rs` `run`), minus everything that knew
//! about hero2: pause gate → redraw → drain input → tick → nap until the guest's next wake-up,
//! interruptible by input.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use winit::event_loop::EventLoopProxy;

use hz_java::input::{GuestKey, KeyProjection};
use hz_patches::{Build, Patches, Selection};
use wie_backend::{Emulator, Event};
use wie_j2me::{J2MEEmulator, J2meOptions};

use crate::{
    InputEvent, JAR_NAME, UserEvent,
    platform::{AndroidPlatform, AndroidScreen},
};

/// The JVM interpreter recurses through async frames on this thread; the reservation is virtual,
/// RSS is only what is touched.
const EMU_STACK: usize = 64 * 1024 * 1024;
/// A ceiling on how often the loop runs when the guest has nothing due sooner.
const FRAME_PACE: Duration = Duration::from_millis(16);

pub struct Boot {
    pub proxy: EventLoopProxy<UserEvent>,
    pub input_rx: Receiver<InputEvent>,
    pub jar: Vec<u8>,
    /// The WAVs the build rendered from the game's MIDI files (Phase 4b); may be empty.
    pub renditions: Vec<Arc<[u8]>>,
    pub options: Arc<hz_java::GameOptions>,
    /// The patches to choose (the default set, unless a developer's `crate::config::PATCHES_FILE`).
    pub patches: Selection,
    pub data_dir: PathBuf,
    pub paused: Arc<AtomicBool>,
    pub guest_w: u32,
    pub guest_h: u32,
    /// What the game is showing (`hz_java::GuestState`), made by the entry point and shared with
    /// the UI thread, which asks it to letter a host window's lines (the PC, 2026-10-29).
    pub scene: Arc<hz_java::GuestState>,
}

/// The nap: wait on the input channel for `nap`, so a key that arrives mid-nap is taken at once.
///
/// ★ 2026-10-29 (the PC's frame drops): on Windows a timed channel wait lands on the scheduler's
/// next tick — 0.6 ms late on average, up to 3 ms, even at 1 ms timer resolution — and the game's
/// field frame is THREE sleeps (its head, its short wait, its budget remainder), so frames came out
/// 54~62 ms instead of 50 (13~18 fps, measured; the phone's futex wakes within 0.1 ms and runs 20).
/// So on Windows the channel wait ends 2 ms early and the rest is `thread::sleep` (a high-resolution
/// waitable timer there) and a last spin of under half a millisecond; the channel is polled once
/// more at the end, so a key waits 2 ms at most. Everywhere else the plain wait is accurate enough.
fn nap_until(input_rx: &Receiver<InputEvent>, nap: Duration) -> Result<InputEvent, RecvTimeoutError> {
    #[cfg(target_os = "windows")]
    {
        const TAIL: Duration = Duration::from_millis(2);
        const SPIN: Duration = Duration::from_micros(400);
        let deadline = Instant::now() + nap;
        if nap > TAIL {
            match input_rx.recv_timeout(nap - TAIL) {
                Err(RecvTimeoutError::Timeout) => {}
                other => return other,
            }
        }
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            if left > SPIN {
                thread::sleep(left - SPIN);
            } else {
                std::hint::spin_loop();
            }
        }
        match input_rx.try_recv() {
            Ok(ev) => Ok(ev),
            Err(TryRecvError::Empty) => Err(RecvTimeoutError::Timeout),
            Err(TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        input_rx.recv_timeout(nap)
    }
}

pub fn spawn(boot: Boot) {
    let proxy = boot.proxy.clone();
    let stopped = boot.proxy.clone();
    let result = thread::Builder::new().name("hz-emu".into()).stack_size(EMU_STACK).spawn(move || {
        // However the worker ends short of the player's own exit — an error, a panic — the UI is
        // told, so the player is not left looking at the last frame.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || run(boot))).is_err() {
            log::error!("the emulator thread panicked");
            let _ = stopped.send_event(UserEvent::GuestStopped);
        }
    });
    if let Err(e) = result {
        log::error!("cannot start the emulator thread: {e}");
        let _ = proxy.send_event(UserEvent::GuestStopped);
    }
}

fn run(boot: Boot) {
    let Boot { proxy, input_rx, jar, renditions, options: game_options, patches, data_dir, paused, guest_w, guest_h, scene } = boot;

    // What the game is showing, reported by the HZ-008 blit hook once a drawn frame; the worker
    // forwards each change to the UI thread for the HUD.
    let mut scene_sent = scene.view();
    let mut stats_sent = scene.stats();
    let mut quick_sent = scene.quick();
    let mut map_sent = scene.map();
    let mut marks_sent = scene.marks();
    let mut portrait_sent = scene.portrait();
    let mut story_sent = 0u8;
    // The world map's names (Phase 7-3): the first field frame letters them in the game's font.
    scene.ask_labels(crate::worldmap_view::wm_label_lines());
    // …and the host's own lines (B1), from the title on.
    scene.ask_texts(crate::host_text::lettering());
    // …and each place's own map, for the game-style minimap the world map shows (Phase 7-5).
    scene.ask_maps(crate::worldmap_view::WM_PLACES.iter().map(|p| p.maps[0]).collect());
    // …and the game's quest guide tiles in those maps (Phase 7-6).
    scene.ask_guide_maps(crate::worldmap_view::WM_PLACES.iter().map(|p| p.maps[0]).collect());
    let mut guide_sent = scene.guide();
    // 이어하기 starts on the slot played last (read from the files, before the game opens them).
    let last = crate::saves::last_played(&data_dir);
    log::info!("saves: slot played last = {last:?}");
    scene.set_preferred_slot(last.map(|n| n - 1));

    // Patches: only for a jar this build knows (md5); anything else boots as it is.
    let options = match Build::identify(&jar) {
        Some(build) => {
            // The game a 세이브 옮기기 file names (B4, `saves::game`).
            crate::saves::set_game(build.jar_md5());
            if patches != Selection::Default {
                log::warn!("patches from {}: {patches:?}", crate::config::PATCHES_FILE);
            }
            let p = Patches::new(build, &patches);
            scene.set_patches(p.chosen().map(|x| x.id));
            log::info!("build: {} — patches {}", build.label(), p.chosen().map(|x| x.id).collect::<Vec<_>>().join(" "));
            // Phase 6: with the game's bar off the HUD draws what it showed.
            let _ = proxy.send_event(UserEvent::BarOff(p.chosen().any(|x| x.id == "HZ-013")));
            let options = J2meOptions {
                property_overrides: p.property_overrides(),
                class_transformer: p.class_transformer().map(Arc::from),
                // The class the hook patches call; it reads the player's 「추가 설정」 options
                // and reports the scene.
                extra_protos: vec![hz_java::hooks::proto(game_options, scene.clone())],
            };
            // The report is only complete once the patched classes have loaded; log what is known now.
            for line in p.report() {
                log::info!("patch: {line}");
            }
            options
        }
        None => {
            log::warn!("UNKNOWN BUILD (jar md5 not in hz_patches) — booting with no patches");
            J2meOptions::default()
        }
    };

    // The collector on a quarter of the painted frames (`hz_java::GC_EVERY`, measured in the probe) —
    // run by this loop after the frame is out (★ 2026-10-28), not by the paint path.
    wie_util::stats::set_gc_every(hz_java::GC_EVERY);
    wie_util::stats::set_gc_in_paint(false);
    let should_redraw = Arc::new(AtomicBool::new(true));
    let paints = Arc::new(AtomicU64::new(0));
    // The newest changed frame (`AndroidScreen`), posted after what the same paint reported.
    let frame = Arc::new(std::sync::Mutex::new(None));
    let screen = AndroidScreen::new(should_redraw.clone(), paints.clone(), frame.clone(), guest_w, guest_h);
    let platform = Box::new(AndroidPlatform::new(screen, data_dir, proxy.clone(), paused.clone(), renditions));
    let save_failures = platform.save_failures();
    let mut emulator = match J2MEEmulator::from_jar_with(platform, JAR_NAME, jar, options) {
        Ok(e) => e,
        Err(e) => {
            log::error!("J2ME load failed: {e:?}");
            let _ = proxy.send_event(UserEvent::GuestStopped);
            return;
        }
    };
    log::info!("emulator: guest canvas {guest_w}x{guest_h}");

    let mut pending: VecDeque<InputEvent> = VecDeque::new();
    // The UI sends honest edges; the game keeps ONE key (`hz_java::input`), so they are projected.
    let mut keys: KeyProjection<wie_backend::KeyCode> = KeyProjection::new();
    let mut seen_paints = 0u64;
    let send = |emulator: &mut J2MEEmulator, out: Vec<GuestKey<wie_backend::KeyCode>>| {
        for k in out {
            emulator.handle_event(match k {
                GuestKey::Down(c) => Event::Keydown(c),
                GuestKey::Up(c) => Event::Keyup(c),
            });
        }
    };
    let mut gauge_at = Instant::now();
    let (mut gauge_loops, mut gauge_tick_us, mut gauge_worst_us) = (0u64, 0u64, 0u64);
    // Key → drawn (★ 2026-10-28): from a press the guest was sent to the first drawn frame whose
    // held key (`f.c`, read by the blit hook) differs from the one before the press — the game
    // thread's own share of the latency, on this phone. `(pressed at, the key drawn before it, an
    // arrow that waited for a release to be taken — its first change is that release's 0)`.
    let mut lat_pending: Option<(Instant, i32, bool)> = None;
    let (mut lat_n, mut lat_sum_us, mut lat_max_us) = (0u64, 0u64, 0u64);
    // The collections this loop ran, and what they cost.
    let (mut gc_n, mut gc_sum_us, mut gc_max_us) = (0u64, 0u64, 0u64);
    // The game's frame pacing (★ 2026-10-29, the PC's frame-drop hunt): the time from one guest
    // paint to the next — count, sum, max, and how many fell in <45 / 45..52 / 52..60 / 60+ ms —
    // and how much the nap overslept what it asked for (the host's timer, not the game).
    let mut last_paint: Option<Instant> = None;
    let (mut gap_n, mut gap_sum_us, mut gap_max_us, mut gap_hist) = (0u64, 0u64, 0u64, [0u64; 4]);
    let (mut over_n, mut over_sum_us, mut over_max_us) = (0u64, 0u64, 0u64);
    // A debug trace of the last loops — `(ms since start, ticks, nap asked ms, nap took ms,
    // wake-up asked ms)` — dumped when a frame gap passes 56 ms, to see where the time went.
    let started = Instant::now();
    let mut trace: VecDeque<(f64, u32, f64, f64, i64)> = VecDeque::new();
    let mut loop_ticks: u32;
    let mut loop_nap: (f64, f64, i64) = (0.0, 0.0, -1);
    // The world map is open (Phase 7-2, `InputEvent::Hold`).
    let mut held = false;
    // The failed save writes counted when the SAVE tab asked (`database`): the game's save swallows
    // a failed write, so only the count says whether the quick save landed.
    let mut save_failures_at: Option<u32> = None;

    loop {
        // 0) Backgrounded or locked: do not tick. Drain input so the channel cannot back up, and
        //    release every held key so nothing is still "down" in the guest on resume. What is
        //    not a key (the world map's hold, the minimap's shift, a SAVE tap) waits in `pending`.
        if paused.load(Ordering::Relaxed) {
            if !crate::drain_while_paused(&mut pending, &input_rx) {
                return;
            }
            send(&mut emulator, keys.release_all());
            scene.set_walk_diag(0);
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let frame_start = Instant::now();

        // 1) The guest asked for a repaint.
        if should_redraw.swap(false, Ordering::SeqCst) {
            emulator.handle_event(Event::Redraw);
        }

        // 2) Input, in order: what the previous nap picked up first, then the channel.
        loop {
            let got = match pending.pop_front() {
                Some(ev) => Ok(ev),
                None => input_rx.try_recv(),
            };
            match got {
                Ok(InputEvent::Key { code, down }) => {
                    // HZ-059: two arrows held = a diagonal, handed to the walk hook before the key goes.
                    let (out, diagonal) = keys.walk_edge(code, down, crate::keys::arrow);
                    scene.set_walk_diag(diagonal);
                    let waited = keys.waiting() == Some(code);
                    if down && (!out.is_empty() || waited) {
                        log::debug!("key: {code:?} pressed (the game holds {}){}", scene.key(), if waited { ", after the release" } else { "" });
                        lat_pending = Some((Instant::now(), scene.key(), waited));
                    }
                    send(&mut emulator, out);
                }
                Ok(InputEvent::TargetShift(px)) => {
                    if px != scene.target_shift() {
                        log::info!("target panel shift (HZ-014): {px} guest px");
                    }
                    scene.set_target_shift(px);
                }
                Ok(InputEvent::TargetDrop(px)) => {
                    if px != scene.target_drop() {
                        log::info!("target panel drop (HZ-054): {px} guest px");
                    }
                    scene.set_target_drop(px);
                }
                Ok(InputEvent::QuickSave) => {
                    save_failures_at.get_or_insert(save_failures.load(Ordering::Relaxed));
                    scene.ask_save();
                }
                Ok(InputEvent::GuestTap { x, y }) => scene.ask_tap(x, y),
                Ok(InputEvent::Plate(ask)) => scene.ask_plate(ask),
                Ok(InputEvent::Page(delta)) => scene.ask_page(delta),
                Ok(InputEvent::Hold(on)) => {
                    if on != held {
                        log::info!("world map {}: the game {}", if on { "open" } else { "closed" }, if on { "holds" } else { "runs again" });
                    }
                    held = on;
                    if on {
                        send(&mut emulator, keys.release_all());
                        scene.set_walk_diag(0);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    log::info!("input channel closed; stopping the emulator");
                    return;
                }
            }
        }

        // The world map is open: no tick, the game stands still (its music has its own thread);
        // wait for the input that ends it.
        if held {
            match input_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(ev) => pending.push_back(ev),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            continue;
        }

        // 3) Run the guest. A paint it asks for in this tick is delivered at once (★ 2026-10-28):
        //    `Event::Redraw` wakes the event pump (`System::push_event`) and one more tick runs the
        //    paint, so the frame leaves before the nap — not after the next one, up to 16 + 16 ms
        //    later (the nap, then the pump's own). Bounded: a paint that asked for another stops here.
        let tick_start = Instant::now();
        let mut ticks = 0;
        loop {
            if let Err(e) = emulator.tick() {
                log::error!("guest error: {e:?}");
                let _ = proxy.send_event(UserEvent::GuestStopped);
                return;
            }
            ticks += 1;
            if ticks < 3 && should_redraw.swap(false, Ordering::SeqCst) {
                emulator.handle_event(Event::Redraw);
                continue;
            }
            break;
        }
        let tick_us = tick_start.elapsed().as_micros() as u64;
        // One projection frame per guest paint in this tick.
        let painted = paints.load(Ordering::Relaxed);
        let seen_paints_before = seen_paints;
        if painted > seen_paints {
            let now = Instant::now();
            if let Some(last) = last_paint {
                let us = now.duration_since(last).as_micros() as u64;
                gap_n += 1;
                gap_sum_us += us;
                gap_max_us = gap_max_us.max(us);
                gap_hist[if us < 45_000 { 0 } else if us < 52_000 { 1 } else if us < 60_000 { 2 } else { 3 }] += 1;
                if us > 56_000 && log::log_enabled!(log::Level::Debug) {
                    log::debug!("pace: gap {:.1} ms at {:.1} ms; last loops (at, ticks, nap asked, nap took, wake asked): {:?}", us as f64 / 1000.0, started.elapsed().as_secs_f64() * 1000.0, trace);
                }
            }
            last_paint = Some(now);
        }
        loop_ticks = ticks;
        for _ in seen_paints..painted {
            send(&mut emulator, keys.frame());
        }
        seen_paints = painted;
        if let Some((pressed, before, waited)) = lat_pending {
            if scene.key() != before && !(waited && scene.key() == 0) {
                let us = pressed.elapsed().as_micros() as u64;
                log::debug!("key: drawn held {} after {:.1} ms", scene.key(), us as f64 / 1000.0);
                lat_n += 1;
                lat_sum_us += us;
                lat_max_us = lat_max_us.max(us);
                lat_pending = None;
            } else if pressed.elapsed() > Duration::from_secs(1) {
                // A press the game never showed as a change (the same key again, a window that
                // does not read it): not a sample.
                log::debug!("key: no change drawn within 1 s (the game holds {})", scene.key());
                lat_pending = None;
            }
        }
        let view = scene.view();
        if view != scene_sent {
            scene_sent = view;
            let _ = proxy.send_event(UserEvent::Guest(view));
        }
        let stats = scene.stats();
        if stats != stats_sent {
            stats_sent = stats;
            if let Some(stats) = stats {
                let _ = proxy.send_event(UserEvent::Stats(stats));
            }
        }
        let quick = scene.quick();
        if quick != quick_sent {
            if let Some(q) = quick.clone() {
                let _ = proxy.send_event(UserEvent::Quick(q));
            }
            quick_sent = quick;
        }
        // The minimap (Phase 6-5): the map when it changes, the marks when they move.
        let map = scene.map();
        if map != map_sent {
            if let Some(m) = map.clone() {
                let _ = proxy.send_event(UserEvent::Map(m));
            }
            map_sent = map;
        }
        let marks = scene.marks();
        if marks != marks_sent {
            let _ = proxy.send_event(UserEvent::Marks(marks.clone()));
            marks_sent = marks;
        }
        let pickups = scene.take_pickups();
        if !pickups.is_empty() {
            let _ = proxy.send_event(UserEvent::Pickups(pickups));
        }
        // HZ-061: a boss's story counting down — each second as it changes.
        let story = scene.story_seconds();
        if story != story_sent {
            story_sent = story;
            let _ = proxy.send_event(UserEvent::StoryWait(story));
        }
        // HZ-065: a quest reward the full bag could not take — it waits for the next save, which puts
        // it into the 계정창고 (or says why it could not).
        for n in scene.take_reward_notices() {
            let line = match n {
                hz_java::reward::Notice::Waiting => "BAG FULL - REWARD GOES TO STORAGE ON SAVE",
                hz_java::reward::Notice::Stored => "REWARD PUT IN STORAGE",
                hz_java::reward::Notice::NotStored => "REWARD NOT STORED - SAVE AGAIN",
                hz_java::reward::Notice::StorageFull => "STORAGE FULL - REWARD WAITS - CLEAR IT AND SAVE",
                hz_java::reward::Notice::Unverified => "REWARD IN STORAGE - NOT VERIFIED",
            };
            log::info!("quest reward: {n:?}");
            let _ = proxy.send_event(UserEvent::Notice(line));
        }
        let texts = scene.take_texts();
        if !texts.is_empty() {
            log::info!("host text: {} of {} lettered", texts.iter().filter(|t| t.1.is_some()).count(), texts.len());
            let _ = proxy.send_event(UserEvent::Texts(texts));
        }
        if let Some(labels) = scene.take_labels() {
            log::info!("world map names: {} of {} lettered", labels.iter().flatten().count(), labels.len());
            let _ = proxy.send_event(UserEvent::Labels(labels));
        }
        let guide = scene.guide();
        if guide != guide_sent {
            if let Some(g) = guide.clone() {
                log::info!("quest guide: quest {:?} -> map {:?}", g.quest, g.target);
                let _ = proxy.send_event(UserEvent::Guide(g));
            }
            guide_sent = guide;
        }
        if let Some(maps) = scene.take_maps() {
            log::info!("world map places' maps: {} of {} read", maps.iter().flatten().count(), maps.len());
            let _ = proxy.send_event(UserEvent::PlaceMaps(maps));
        }
        // The SAVE tab (Phase 7-1; at once since 2026-10-29): the host says how it went — the game
        // says nothing when it is not asked. A failed write has its own notice (`database::Notify`),
        // and then the save is not called done: `ed.k()` returns as if it worked.
        if let Some(answer) = scene.take_save_answer() {
            let failed = save_failures_at.take().is_some_and(|at| save_failures.load(Ordering::Relaxed) != at);
            match answer {
                hz_java::SaveAnswer::Saved if failed => log::error!("quick save: a write failed (the storage notice is up)"),
                hz_java::SaveAnswer::Saved => {
                    log::info!("quick save: written");
                    let _ = proxy.send_event(UserEvent::Notice("SAVED"));
                }
                hz_java::SaveAnswer::NotHere => {
                    log::info!("quick save: the map allows none");
                    let _ = proxy.send_event(UserEvent::Notice("SAVE - NOT HERE"));
                }
                hz_java::SaveAnswer::Busy => {
                    log::info!("quick save: not now (not the bare field, or the player is busy)");
                    let _ = proxy.send_event(UserEvent::Notice("SAVE - NOT NOW"));
                }
            }
        }
        // The portrait (Phase 6-6): only when the look changes.
        let portrait = scene.portrait();
        if portrait != portrait_sent {
            let _ = proxy.send_event(UserEvent::Portrait(portrait.clone()));
            portrait_sent = portrait;
        }
        // The picture last, so the UI thread presents it with everything it goes with.
        if let Some(f) = frame.lock().ok().and_then(|mut s| s.take()) {
            let _ = proxy.send_event(UserEvent::Paint(f));
        }
        // 3b) The collector, once the picture is out (★ 2026-10-28): the paint path used to collect
        //     right after handing the frame over, inside the tick, so the frame reached the UI only
        //     after the pause — every GC_EVERY-th frame that much late. Here it runs during the game
        //     thread's own sleep, and is timed for the gauge.
        for _ in seen_paints_before..painted {
            if wie_util::stats::gc_due() {
                let t = Instant::now();
                match scene.collect_garbage() {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => log::warn!("gc: {e:?}"),
                    None => {}
                }
                let us = t.elapsed().as_micros() as u64;
                gc_n += 1;
                gc_sum_us += us;
                gc_max_us = gc_max_us.max(us);
            }
        }
        gauge_loops += 1;
        gauge_tick_us += tick_us;
        gauge_worst_us = gauge_worst_us.max(tick_us);
        if gauge_at.elapsed() >= Duration::from_secs(30) {
            let secs = gauge_at.elapsed().as_secs_f64();
            // The game's own frames, and what the HZ-008 hook's reads cost the game's thread in them.
            let (hook_frames, hook_ns) = hz_java::hooks::take_blit_gauge();
            log::info!(
                "emu pace: frame gap avg {:.1} ms, max {:.1} ms ({} frames: <45 {} / 45-52 {} / 52-60 {} / 60+ {}) | nap overslept avg {:.2} ms, max {:.2} ms ({} naps)",
                gap_sum_us as f64 / 1000.0 / gap_n.max(1) as f64,
                gap_max_us as f64 / 1000.0,
                gap_n,
                gap_hist[0],
                gap_hist[1],
                gap_hist[2],
                gap_hist[3],
                over_sum_us as f64 / 1000.0 / over_n.max(1) as f64,
                over_max_us as f64 / 1000.0,
                over_n
            );
            (gap_n, gap_sum_us, gap_max_us, gap_hist) = (0, 0, 0, [0; 4]);
            (over_n, over_sum_us, over_max_us) = (0, 0, 0);
            log::info!(
                "emu gauge: {:.1} loops/s, tick avg {:.2} ms, worst {:.2} ms, in-tick {:.0} % | game {:.1} fps, hook {:.2} ms a frame | key->drawn avg {:.1} ms, max {:.1} ms ({} presses) | gc avg {:.1} ms, max {:.1} ms ({} runs)",
                gauge_loops as f64 / secs,
                gauge_tick_us as f64 / gauge_loops.max(1) as f64 / 1000.0,
                gauge_worst_us as f64 / 1000.0,
                gauge_tick_us as f64 / 1e6 / secs * 100.0,
                hook_frames as f64 / secs,
                hook_ns as f64 / 1e6 / hook_frames.max(1) as f64,
                lat_sum_us as f64 / 1000.0 / lat_n.max(1) as f64,
                lat_max_us as f64 / 1000.0,
                lat_n,
                gc_sum_us as f64 / 1000.0 / gc_n.max(1) as f64,
                gc_max_us as f64 / 1000.0,
                gc_n
            );
            gauge_at = Instant::now();
            (gauge_loops, gauge_tick_us, gauge_worst_us) = (0, 0, 0);
            (lat_n, lat_sum_us, lat_max_us) = (0, 0, 0);
            (gc_n, gc_sum_us, gc_max_us) = (0, 0, 0);
        }

        // 4) Nap until the guest's next wake-up (never past it, never longer than FRAME_PACE),
        //    interruptibly: a key that arrives mid-nap is taken at once.
        let elapsed = frame_start.elapsed();
        if elapsed < FRAME_PACE {
            let mut nap = FRAME_PACE - elapsed;
            if let Some(ms) = emulator.next_wakeup_ms() {
                // 1 ms floor: `Some(0)` = runnable now, and this must not become a spin.
                nap = nap.min(Duration::from_millis(ms.max(1)));
            }
            let nap_at = Instant::now();
            loop_nap = (nap.as_secs_f64() * 1000.0, 0.0, emulator.next_wakeup_ms().map_or(-1, |v| v as i64));
            match nap_until(&input_rx, nap) {
                Ok(ev) => pending.push_back(ev),
                Err(RecvTimeoutError::Timeout) => {
                    let us = nap_at.elapsed().saturating_sub(nap).as_micros() as u64;
                    over_n += 1;
                    over_sum_us += us;
                    over_max_us = over_max_us.max(us);
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
            loop_nap.1 = nap_at.elapsed().as_secs_f64() * 1000.0;
        }
        if log::log_enabled!(log::Level::Debug) {
            trace.push_back((started.elapsed().as_secs_f64() * 1000.0, loop_ticks, loop_nap.0, loop_nap.1, loop_nap.2));
            if trace.len() > 14 {
                trace.pop_front();
            }
            loop_nap = (0.0, 0.0, -1);
        }
    }
}
