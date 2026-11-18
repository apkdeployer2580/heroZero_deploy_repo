//! hz_probe — headless runner for 영웅서기 Zero (J2ME MIDlet on `wie_j2me`).
//!
//! Boots the jar with no window and no audio, on a VIRTUAL clock (so a run is
//! deterministic and timers fire however fast the host ticks), plays a key script and
//! dumps frames as PNG. The emulator core is the one the APK runs, so a defect that
//! reproduces here reproduces on the device.
//!
//! usage:
//!   hz_probe --jar <game.jar> [--w 240] [--h 320] [--ms 20] [--frames 600]
//!            [--keys 120:OK,200:DOWN:3,...] [--dump 100,300 | --every 50]
//!            [--out <dir>] [--rms-in <dir>] [--rms-out <dir>] [--patches default]
//!            [--opt exp=2,drop=3,loot=on,orient=portrait]
//!
//!   --keys   `frame:KEY[:hold]` — pressed on `frame`, released `hold` frames later
//!            (default 3). KEY is UP DOWN LEFT RIGHT OK LSK RSK CLR 0-9 STAR HASH. The edges go
//!            through `hz_java::input::KeyProjection`, as the APK's do (the game keeps ONE key);
//!            `--raw-keys 1` sends them to the guest as they are.
//!   --dump   frames to write as `<out>/frame_<n>.png`; `--every N` adds every Nth.
//!   --sample F  from `--sample-from` (frame, default 1) on, sample the game's thread's stacks
//!            into file F (`sampler.rs`; `tools/hz_profile.py F` names and sums them).
//!   --gc N   collect garbage every N painted frames (default `hz_java::GC_EVERY`, what the APK sets)
//!   --timing N  every N frames, what the host spent: `tick` = wall ms per `tick()` (avg and
//!            worst), `hook` = the HZ-008 blit hook's reads per guest paint, `paints` per frame.
//!   --rms-in preload RecordStores from `<dir>/<name>/<id>.bin` (read-only: the guest's
//!            writes land in memory and never go back to disk).
//!   --rms-out write the RecordStores as they stand at the end of the run.
//!   --patches which Hero Zero patches (`hz_patches`) to apply: `default` (what the APK
//!            ships), `none` (the original game), `all`, or `HZ-001,HZ-002`. Applied only
//!            to a jar whose md5 is a known build; any other jar boots unpatched.
//!   --opt    the player's 「추가 설정」 options (`hz_java::GameOptions`) for this run, in
//!            memory only: exp=1|2|3|5, drop=1|2|3, diff=100|150|200, respawn=1..6, loot=on|off,
//!            orient=landscape|portrait, stick=free|dpad|fixed.
//!            Default = the shipped defaults (= the original game). Changes the guest makes on
//!            the page are logged, never written.
//!   --exp-test F:N            at frame F, call the game's own `ek.j(N)` on the player (field only)
//!   --drop-money F:DX:DY:N    at frame F, put N money on the ground DX/DY tiles from the player
//!   --drop-item F:DX:DY:CAT:ID   the same for one item
//!   --drop-test F             at frame F, call Hooks.dropThreshold on a 4833 slot
//!   --stat-test F:IDX:VALUE   at frame F, the game's own `ek.a(IDX, VALUE)` on the player (0 HP, 1 SP,
//!                             2 EXP, 3 MP) — a stand-in for a hit, to watch `hz: stats` follow
//!   --quick-set F:SLOT:ID     at frame F, put ID in quick slot SLOT (the game's bar redraws it)
//!   --warp F:MAP:X:Y          at frame F, the game's own "go to map" `ed.b(MAP,X,Y,0)`
//!   --equip-test F:SLOT:CAT:ID   at frame F, wear item `ba.a(CAT,ID)` in slot SLOT as the equipment
//!                             window does (slot 1 + category 10 = head gear — the portrait follows)
//!   --hit F:N                 at frame F, the game's own `ek.c(−N)` on the player (N damage; at 0 HP
//!                             the death flow `ek.C()` follows)
//!   --quest-set F:Q:STATE     at frame F, quest Q's state (`ed.a(Q)`: 0 · 1 under way · 2 done) —
//!                             then `--warp` so the game picks its guide tile (`taskX = …`) and the
//!                             world map follows the trail (`hz: guide quest …`) for that quest
//!   --mobs F                  at frame F, the living monsters' stats (kind, LV, HP, ATK, DEF) and the
//!                             respawn queue (`kind:countdown`) — HZ-017 난이도 / HZ-018 리젠
//!   --kill F                  at frame F, the first living monster's own death `cq.g()V` (and its drop)
//!   --item-grade F:I:G        at frame F, the equipment bag's item I given grade G (`bx.e`, 0..3)
//!   --recipe-test F:ID        at frame F, the game's ba.a(18, ID): the recipe's ingredients and result
//!   --give F:CAT:ID           at frame F, item ba.a(CAT, ID) into its bag as a pickup does (full: the 계정창고)
//!   --bags F                  at frame F, the equipment bag's and the 계정창고's items
//!   --item-check F            at frame F, every new piece made and worn alone (tools/hz_item_test.py reads it)
//!   --hp-pct F:P              at frame F, the player's HP made P % of its top
//!   --flag F:N                at frame F, story flag N by the game's ed.c(I)V (251 = 정령사 open, 250 = 총검사)
//!   --job F:N                 at frame F, the player made job N (0 워리어 .. 5 정령사) by the game's ek.m(B)V
//!   --round F:N               at frame F, the game's replay round `ed.h` set to N (0..2, 3 = 가디언 모드 HZ-030) - monsters made
//!                             after it (a warp, a respawn) get its levels (round 2 = LV 60..87, 3 = 85..110)
//!   --item-icons F            at frame F, every item picture (`/img/it`) as `<out>/item_NN.png` (4x)
//!            All run inside the field-tick hook (see `field_tick`), repeatable, comma-separated.
//!   --target-shift N          HZ-014: the target-monster panel N guest px left (what the APK's
//!            minimap asks for — 69 on a 520x240 canvas). Default 0 = where the game puts it.
//!   --target-drop N           HZ-054: the target-monster panel N guest px lower (the APK's portrait
//!            minimap asks for its height and two pads). Default 0 = where the game puts it.
//!   --save-ask F[,F…]         Phase 7-1: the SAVE tab "tapped" before frame F; the hook's answer
//!            prints as `[probe] f=N SAVE tab -> Saved|NotHere|Busy` (Saved = the game's own `ed.k()` wrote
//!            the slot, no question — 2026-10-29).
//!   --prefer-slot N           이어하기 starts on save slot N (the APK: the slot played last).
//!   --tap F:X:Y[,…]           a tap on the game's picture at guest (X, Y) before frame F — the game's
//!            own windows (`hz_java::menu_touch`); the hook logs `hz: tap X,Y (…) -> …`, and every
//!            change of the open windows as `hz: windows bh 3/10 @180,33 > …`.
//!   --pad-page F:D[,…]        a pad's page button before frame F, D −1 (LB) or 1 (RB) — what the app sends
//!            in a window with pages (`GuestState::ask_page`); the hook logs `hz: pad page …`.
//!   --labels 이름,이름,…        Phase 7-3: the first field frame letters these in the game's own
//!            font; at the end `label_N.png` (4x) and `labels.txt` (for hz_android's preview).
//!   --host-texts 줄,줄,…        B1: the host's own lines, lettered a few a frame from the title on
//!            (`GuestState::ask_texts`, numbered in order); at the end `text_N.png` (4x) and
//!            `texts.txt` (`text N W H` + rows — hz_android's preview, by `host_text` order).
//!   --place-maps N,N,…         Phase 7-5: the first field frame reads these maps from the jar by the
//!            game's own loader; at the end `[probe] place map N: WxH (floor, exits, chests)`.
//!
//! At the end the minimap the HZ-008 hook read (Phase 6-5) is written as `<out>/minimap.png`
//! (tile classes 4x, the marks on top) and `<out>/minimap.txt` (for hz_android's preview), and
//! the portrait it composed (Phase 6-6) as `<out>/portrait.png` (8x) and `<out>/portrait.txt`.
//!
//! The objective boot test is a dumped frame with `non-black > 0`, never an eyeballed
//! screenshot — every dump prints that count.
//!
//! The hooks log what they do at info (`hz: EXP +45 x2 = +90`); the default filter shows them.
//! So does the audio sink (`hz: audio play_wave …`, one line per clip the guest plays).
//!
//!   RUST_LOG=wie_midp=debug hz_probe ...   (needs a `--no-default-features` build)

mod field_tick;
#[cfg(all(windows, target_arch = "x86_64"))]
mod sampler;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use wie_backend::{
    AudioSink, Database, DatabaseRepository, Emulator, Event, Filesystem, Instant, KeyCode, Platform, RecordId, Screen, canvas::Image,
};
use hz_java::GameOptions;
use hz_patches::{Build, Patches, Selection};
use wie_j2me::{J2MEEmulator, J2meOptions};

/// The name the jar is mounted under. `J2MEEmulator::from_jar` uses it as the app id too,
/// which keys the RecordStores — so it is a constant, not whatever the file on disk is
/// called, or renaming the input would make every save look like a fresh install.
const JAR_NAME: &str = "herozero.jar";

// ---------------------------------------------------------------- screen ----

#[derive(Clone, Default)]
struct Frame {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

struct ProbeScreen {
    w: u32,
    h: u32,
    last: Arc<Mutex<Frame>>,
    redraw: Arc<AtomicBool>,
    paints: Arc<AtomicU64>,
}

impl Screen for ProbeScreen {
    fn request_redraw(&self) -> wie_util::Result<()> {
        self.redraw.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn paint(&self, image: &dyn Image) {
        let mut rgba = Vec::with_capacity((image.width() * image.height() * 4) as usize);
        for c in image.colors() {
            rgba.extend_from_slice(&[c.r, c.g, c.b, 0xff]);
        }
        *self.last.lock().unwrap() = Frame { w: image.width(), h: image.height(), rgba };
        self.paints.fetch_add(1, Ordering::SeqCst);
    }

    fn width(&self) -> u32 {
        self.w
    }

    fn height(&self) -> u32 {
        self.h
    }
}

// ------------------------------------------------------------ filesystem ----

/// The MIDlet's private files. A J2ME guest reads its resources from the jar (the
/// classpath), so this only ever sees what the runtime itself parks here.
#[derive(Default)]
struct MemFs {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemFs {
    fn key(aid: &str, path: &str) -> String {
        format!("{aid}:{}", path.trim_start_matches('/'))
    }
}

#[async_trait::async_trait]
impl Filesystem for MemFs {
    async fn exists(&self, aid: &str, path: &str) -> bool {
        self.files.lock().unwrap().contains_key(&Self::key(aid, path))
    }

    async fn size(&self, aid: &str, path: &str) -> Option<usize> {
        self.files.lock().unwrap().get(&Self::key(aid, path)).map(|x| x.len())
    }

    async fn read(&self, aid: &str, path: &str, offset: usize, count: usize, buf: &mut [u8]) -> Option<usize> {
        let files = self.files.lock().unwrap();
        let data = files.get(&Self::key(aid, path))?;
        if offset >= data.len() {
            return Some(0);
        }
        let n = count.min(data.len() - offset);
        buf[..n].copy_from_slice(&data[offset..offset + n]);
        Some(n)
    }

    async fn write(&self, aid: &str, path: &str, offset: usize, data: &[u8]) -> usize {
        let mut files = self.files.lock().unwrap();
        let file = files.entry(Self::key(aid, path)).or_default();
        if file.len() < offset + data.len() {
            file.resize(offset + data.len(), 0);
        }
        file[offset..offset + data.len()].copy_from_slice(data);
        data.len()
    }

    async fn truncate(&self, aid: &str, path: &str, len: usize) {
        self.files.lock().unwrap().entry(Self::key(aid, path)).or_default().resize(len, 0);
    }
}

// ------------------------------------------------------------- RMS stores ----

type Store = Arc<Mutex<BTreeMap<RecordId, Vec<u8>>>>;

/// MIDP RecordStores in memory. 영웅서기 Zero saves through these (`HOL.gamedata1..3`,
/// `HOL.slot1..3`, `HOL.item1..3`, `HOL.storage`, `HOL.gamestore`), so this is where a
/// save A/B looks — and since nothing here touches the disk, a probe run cannot lose one.
#[derive(Default)]
struct MemRms {
    stores: Mutex<BTreeMap<String, Store>>,
}

impl MemRms {
    fn store(&self, name: &str) -> Store {
        self.stores.lock().unwrap().entry(name.to_string()).or_default().clone()
    }

    /// `<dir>/<store name>/<record id>.bin`
    fn preload(&self, dir: &Path) -> std::io::Result<usize> {
        let mut n = 0;
        for store in fs::read_dir(dir)? {
            let store = store?;
            if !store.file_type()?.is_dir() {
                continue;
            }
            let name = store.file_name().to_string_lossy().into_owned();
            let records = self.store(&name);
            for rec in fs::read_dir(store.path())? {
                let rec = rec?.path();
                let Some(id) = rec.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<RecordId>().ok()) else {
                    continue;
                };
                records.lock().unwrap().insert(id, fs::read(&rec)?);
                n += 1;
            }
        }
        Ok(n)
    }

    fn dump(&self, dir: &Path) -> std::io::Result<()> {
        for (name, store) in self.stores.lock().unwrap().iter() {
            let d = dir.join(name);
            fs::create_dir_all(&d)?;
            for (id, data) in store.lock().unwrap().iter() {
                fs::write(d.join(format!("{id}.bin")), data)?;
            }
        }
        Ok(())
    }

    fn summary(&self) -> Vec<String> {
        self.stores
            .lock()
            .unwrap()
            .iter()
            .map(|(name, s)| {
                let s = s.lock().unwrap();
                let sizes: Vec<String> = s.iter().map(|(id, d)| format!("#{id}={}B", d.len())).collect();
                format!("{name}: {}", if sizes.is_empty() { "(empty)".to_string() } else { sizes.join(" ") })
            })
            .collect()
    }
}

struct MemDatabase {
    name: String,
    records: Store,
    /// `HZ_PROBE_RMS_FAIL=<store>[,<store>…]`: every write to this store fails (a full disk, an I/O
    /// error) — the record keeps its bytes, as the app's atomic replace does. For testing what the
    /// game and the hooks do with a refused write (HZ-065's read-back, 2026-11-03).
    fail: bool,
}

#[async_trait::async_trait]
impl Database for MemDatabase {
    async fn next_id(&self) -> RecordId {
        // MIDP record ids start at 1 and are never reused.
        self.records.lock().unwrap().keys().next_back().map_or(1, |k| k + 1)
    }

    async fn add(&mut self, data: &[u8]) -> Option<RecordId> {
        if self.fail {
            eprintln!("[rms] {} add ({} B) REFUSED (HZ_PROBE_RMS_FAIL)", self.name, data.len());
            return None;
        }
        let mut r = self.records.lock().unwrap();
        let id = r.keys().next_back().map_or(1, |k| k + 1);
        r.insert(id, data.to_vec());
        eprintln!("[rms] {} add #{id} ({} B)", self.name, data.len());
        Some(id)
    }

    async fn get(&self, id: RecordId) -> Option<Vec<u8>> {
        self.records.lock().unwrap().get(&id).cloned()
    }

    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool {
        if self.fail {
            eprintln!("[rms] {} set #{id} ({} B) REFUSED (HZ_PROBE_RMS_FAIL)", self.name, data.len());
            return false;
        }
        let mut r = self.records.lock().unwrap();
        if !r.contains_key(&id) {
            return false;
        }
        r.insert(id, data.to_vec());
        eprintln!("[rms] {} set #{id} ({} B)", self.name, data.len());
        true
    }

    async fn delete(&mut self, id: RecordId) -> bool {
        let gone = self.records.lock().unwrap().remove(&id).is_some();
        eprintln!("[rms] {} delete #{id} -> {gone}", self.name);
        gone
    }

    async fn get_record_ids(&self) -> Vec<RecordId> {
        self.records.lock().unwrap().keys().copied().collect()
    }
}

#[async_trait::async_trait]
impl DatabaseRepository for MemRms {
    async fn open(&self, name: &str, _app_id: &str) -> Box<dyn Database> {
        let fail = std::env::var("HZ_PROBE_RMS_FAIL").is_ok_and(|v| v.split(',').any(|s| s.trim() == name));
        Box::new(MemDatabase { name: name.to_string(), records: self.store(name), fail })
    }

    async fn exists(&self, name: &str, _app_id: &str) -> bool {
        self.stores.lock().unwrap().contains_key(name)
    }

    async fn delete(&self, name: &str, _app_id: &str) -> bool {
        eprintln!("[rms] deleteRecordStore {name}");
        self.stores.lock().unwrap().remove(name).is_some()
    }
}

// ----------------------------------------------------------------- audio ----

/// No sound; counts what the guest asked for, so a run can say whether audio started.
#[derive(Clone, Default)]
struct NoAudio {
    waves: Arc<AtomicU64>,
    midi: Arc<AtomicU64>,
    media: Arc<AtomicU64>,
}

impl AudioSink for NoAudio {
    /// Logged per clip (`hz: audio …`, shown by default like the hook logs): channels, rate,
    /// length and the loudest sample — the proof that a WAV reached the sink with its level in.
    fn play_wave(&self, channel: u8, rate: u32, data: &[i16]) {
        self.waves.fetch_add(1, Ordering::Relaxed);
        let peak = data.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0);
        let ms = if rate > 0 { data.len() as u64 * 1000 / (rate as u64 * channel.max(1) as u64) } else { 0 };
        tracing::info!("hz: audio play_wave {channel} ch {rate} Hz {} samples ({ms} ms) peak {peak}", data.len());
    }
    fn midi_note_on(&self, _c: u8, _n: u8, _v: u8) {
        self.midi.fetch_add(1, Ordering::Relaxed);
    }
    fn midi_note_off(&self, _c: u8, _n: u8, _v: u8) {}
    fn midi_program_change(&self, _c: u8, _p: u8) {}
    fn midi_control_change(&self, _c: u8, _ctl: u8, _v: u8) {}
    fn midi_pitch_bend(&self, _c: u8, _v: u16) {}
    fn midi_sysex(&self, _d: &[u8]) {}

    /// The songs (Phase 4c): which stream, what the file is, and the lifecycle the game drives —
    /// the proof that a song reached the sink, and when it stopped, resumed or was replaced.
    fn media_start(&self, id: u32, data: &Arc<[u8]>, loops: i32, level: u8) {
        self.media.fetch_add(1, Ordering::Relaxed);
        let kind = match data.get(..8) {
            Some([_, _, _, _, b'f', b't', b'y', b'p']) => "mp4",
            Some([b'M', b'T', b'h', b'd', ..]) => "midi",
            _ => "unknown",
        };
        tracing::info!("hz: audio media_start {id} {kind} {} bytes, loop count {loops}, level {level}", data.len());
    }
    fn media_stop(&self, id: u32) {
        tracing::info!("hz: audio media_stop {id}");
    }
    fn media_level(&self, id: u32, level: u8) {
        tracing::info!("hz: audio media_level {id} {level}");
    }
    fn media_close(&self, id: u32) {
        tracing::info!("hz: audio media_close {id}");
    }
}

// -------------------------------------------------------------- platform ----

struct ProbePlatform {
    screen: ProbeScreen,
    fs: MemFs,
    rms: Arc<MemRms>,
    audio: NoAudio,
    /// Virtual clock (ms), advanced by the frame loop.
    clock: Arc<AtomicU64>,
    exited: Arc<AtomicBool>,
}

impl Platform for ProbePlatform {
    fn screen(&self) -> &dyn Screen {
        &self.screen
    }

    fn now(&self) -> Instant {
        Instant::from_epoch_millis(self.clock.load(Ordering::SeqCst))
    }

    fn database_repository(&self) -> &dyn DatabaseRepository {
        self.rms.as_ref()
    }

    fn filesystem(&self) -> &dyn Filesystem {
        &self.fs
    }

    fn audio_sink(&self) -> Box<dyn AudioSink> {
        Box::new(self.audio.clone())
    }

    fn write_stdout(&self, buf: &[u8]) {
        print!("{}", String::from_utf8_lossy(buf));
    }

    fn write_stderr(&self, buf: &[u8]) {
        eprint!("{}", String::from_utf8_lossy(buf));
    }

    fn exit(&self) {
        eprintln!("[probe] guest called exit()");
        self.exited.store(true, Ordering::SeqCst);
    }

    /// The game's `Display.vibrate` (C1: `au.a(B)V` from its thread `az`, woken by `rpg/c.k()`).
    fn vibrate(&self, duration_ms: u64, _intensity: u8) {
        eprintln!("[probe] vibrate {duration_ms} ms");
    }
}

// ------------------------------------------------------------------ args ----

struct Args {
    jar: PathBuf,
    w: u32,
    h: u32,
    ms: u64,
    frames: u64,
    keys: Vec<(u64, KeyCode, u64)>,
    dump: Vec<u64>,
    every: u64,
    timing: u64,
    sample: Option<PathBuf>,
    sample_from: u64,
    gc: u64,
    out: PathBuf,
    rms_in: Option<PathBuf>,
    rms_out: Option<PathBuf>,
    patches: Selection,
    raw_keys: bool,
    options: GameOptions,
    actions: Vec<field_tick::Action>,
    target_shift: i32,
    target_drop: i32,
    /// Frames at which the SAVE tab is "tapped" (Phase 7-1: `GuestState::ask_save`).
    save_ask: Vec<u64>,
    /// HZ-065 `--reward F:CAT:ID:N`: a quest reward the full bag could not take, waiting for the next
    /// save (`GuestState::reward_wait` — what the give hook does; the save's put-away is then tried
    /// with `--save-ask`, the storage seeded with `--stash`, a refused write with `HZ_PROBE_RMS_FAIL`).
    rewards: Vec<(u64, i8, i8, i32)>,
    /// HZ-058: `(frame, plate)` presses (`--plate F:N`, 1-based) and page turns (`--plate-page F`) —
    /// `GuestState::ask_plate`.
    plates: Vec<(u64, hz_java::PlateAsk)>,
    /// `(frame, x, y)`: taps on the game's picture in guest px (`GuestState::ask_tap`, the game's
    /// own windows — `hz_java::menu_touch`).
    taps: Vec<(u64, i32, i32)>,
    /// `(frame, delta)`: a pad's page button (`--pad-page`, `GuestState::ask_page`).
    pad_pages: Vec<(u64, i8)>,
    /// 이어하기's starting slot, 1-based (`GuestState::set_preferred_slot` — the APK reads it from
    /// the save files; `crate::saves::last_played`).
    prefer_slot: Option<u8>,
    /// Lines for the game's font (Phase 7-3: `GuestState::ask_labels`), EUC-KR.
    labels: Vec<Vec<u8>>,
    /// The host's lines for the game's font (B1: `GuestState::ask_texts`), EUC-KR.
    host_texts: Vec<Vec<u8>>,
    /// Maps read from the jar by the game's loader (Phase 7-5: `GuestState::ask_maps`).
    place_maps: Vec<u8>,
}

fn parse_key(s: &str) -> Option<KeyCode> {
    Some(match s {
        "UP" => KeyCode::UP,
        "DOWN" => KeyCode::DOWN,
        "LEFT" => KeyCode::LEFT,
        "RIGHT" => KeyCode::RIGHT,
        "OK" => KeyCode::OK,
        "LSK" => KeyCode::LEFT_SOFT_KEY,
        "RSK" => KeyCode::RIGHT_SOFT_KEY,
        "CLR" | "CLEAR" => KeyCode::CLEAR,
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
        "STAR" | "*" => KeyCode::STAR,
        "HASH" | "#" => KeyCode::HASH,
        _ => return None,
    })
}

/// HZ-059: a key's walk direction (`hz_java::input::dir`) — the four arrows, as the APK's worker has it.
fn arrow(k: KeyCode) -> Option<i8> {
    use hz_java::input::dir;
    match k {
        KeyCode::UP => Some(dir::UP),
        KeyCode::DOWN => Some(dir::DOWN),
        KeyCode::LEFT => Some(dir::LEFT),
        KeyCode::RIGHT => Some(dir::RIGHT),
        _ => None,
    }
}

fn parse_args() -> Result<Args, Vec<String>> {
    let mut a = Args {
        jar: PathBuf::new(),
        w: 240,
        h: 320,
        ms: 20,
        frames: 600,
        keys: Vec::new(),
        dump: Vec::new(),
        every: 0,
        timing: 0,
        sample: None,
        sample_from: 1,
        gc: hz_java::GC_EVERY,
        out: PathBuf::from("out"),
        rms_in: None,
        rms_out: None,
        patches: Selection::Default,
        raw_keys: false,
        options: GameOptions::new(),
        actions: Vec::new(),
        target_shift: 0,
        target_drop: 0,
        save_ask: Vec::new(),
        rewards: Vec::new(),
        plates: Vec::new(),
        taps: Vec::new(),
        pad_pages: Vec::new(),
        prefer_slot: None,
        labels: Vec::new(),
        host_texts: Vec::new(),
        place_maps: Vec::new(),
    };
    let mut errs = Vec::new();
    let mut jar = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut it = argv.iter();
    while let Some(flag) = it.next() {
        let Some(v) = it.next() else {
            errs.push(format!("{flag}: missing value"));
            break;
        };
        let num = |errs: &mut Vec<String>| v.parse::<u64>().map_err(|_| errs.push(format!("{flag} {v}: not a number"))).unwrap_or(0);
        match flag.as_str() {
            "--jar" => jar = Some(PathBuf::from(v)),
            "--w" => a.w = num(&mut errs) as u32,
            "--h" => a.h = num(&mut errs) as u32,
            "--ms" => a.ms = num(&mut errs),
            "--frames" => a.frames = num(&mut errs),
            "--every" => a.every = num(&mut errs),
            "--timing" => a.timing = num(&mut errs),
            "--sample" => a.sample = Some(PathBuf::from(v)),
            "--sample-from" => a.sample_from = num(&mut errs),
            "--gc" => a.gc = num(&mut errs),
            "--out" => a.out = PathBuf::from(v),
            "--raw-keys" => a.raw_keys = num(&mut errs) != 0,
            "--target-shift" => a.target_shift = num(&mut errs) as i32,
            "--target-drop" => a.target_drop = num(&mut errs) as i32,
            "--place-maps" => {
                for id in v.split(',').filter(|s| !s.is_empty()) {
                    match id.parse() {
                        Ok(n) => a.place_maps.push(n),
                        Err(_) => errs.push(format!("--place-maps {id}: not a map number")),
                    }
                }
            }
            "--host-texts" => {
                for line in v.split(',').filter(|s| !s.is_empty()) {
                    match hz_java::euc_kr(line) {
                        Some(b) => a.host_texts.push(b),
                        None => errs.push(format!("--host-texts {line}: not in the game's font")),
                    }
                }
            }
            "--labels" => {
                for name in v.split(',').filter(|s| !s.is_empty()) {
                    match hz_java::euc_kr(name) {
                        Some(b) => a.labels.push(b),
                        None => errs.push(format!("--labels {name}: not in the game's font")),
                    }
                }
            }
            "--save-ask" => {
                for f in v.split(',').filter(|s| !s.is_empty()) {
                    match f.parse() {
                        Ok(n) => a.save_ask.push(n),
                        Err(_) => errs.push(format!("--save-ask {f}: not a frame number")),
                    }
                }
            }
            "--reward" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    let p: Vec<&str> = item.split(':').collect();
                    let parsed = match p.as_slice() {
                        [at, cat, id, n] => at.parse().ok().zip(cat.parse().ok()).zip(id.parse().ok()).zip(n.parse().ok()).map(|(((at, cat), id), n)| (at, cat, id, n)),
                        _ => None,
                    };
                    match parsed {
                        Some(r) => a.rewards.push(r),
                        None => errs.push(format!("--reward {item}: want F:CAT:ID:N")),
                    }
                }
            }
            "--plate" | "--plate-page" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    let p: Vec<&str> = item.split(':').collect();
                    let frame = p.first().and_then(|s| s.parse::<u64>().ok());
                    let ask = match (flag.as_str(), p.get(1).and_then(|s| s.parse::<u8>().ok())) {
                        ("--plate-page", None) if p.len() == 1 => Some(hz_java::PlateAsk::Page),
                        ("--plate", Some(n)) if (1..=7).contains(&n) && p.len() == 2 => Some(hz_java::PlateAsk::Press(n - 1)),
                        _ => None,
                    };
                    match (frame, ask) {
                        (Some(f), Some(ask)) => a.plates.push((f, ask)),
                        _ => errs.push(format!("{flag} {item}: expected F:N (plate 1..7) / F")),
                    }
                }
            }
            "--tap" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    let p: Vec<&str> = item.split(':').collect();
                    match (p.len(), p.first().and_then(|s| s.parse().ok()), p.get(1).and_then(|s| s.parse().ok()), p.get(2).and_then(|s| s.parse().ok())) {
                        (3, Some(f), Some(x), Some(y)) => a.taps.push((f, x, y)),
                        _ => errs.push(format!("--tap {item}: expected frame:X:Y (guest px)")),
                    }
                }
            }
            "--pad-page" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    match item.split_once(':').and_then(|(f, d)| Some((f.parse().ok()?, d.parse::<i8>().ok()?))) {
                        Some((f, d)) if d != 0 => a.pad_pages.push((f, d)),
                        _ => errs.push(format!("--pad-page {item}: expected frame:-1 / frame:1")),
                    }
                }
            }
            "--prefer-slot" => a.prefer_slot = Some(num(&mut errs) as u8),
            "--rms-in" => a.rms_in = Some(PathBuf::from(v)),
            "--rms-out" => a.rms_out = Some(PathBuf::from(v)),
            "--patches" => match Selection::parse(v) {
                Ok(s) => a.patches = s,
                Err(e) => errs.push(format!("--patches: {e}")),
            },
            "--exp-test" | "--drop-money" | "--drop-item" | "--drop-test" | "--stat-test" | "--quick-set" | "--warp" | "--equip-test" | "--quest-set" | "--hit"
            | "--item-icons" | "--mobs" | "--kill" | "--item-grade" | "--round" | "--job" | "--skill-level" | "--base-stat" | "--spirit-level" | "--refill" | "--walk-dir" | "--recipe-test" | "--give" | "--bags" | "--item-check" | "--hp-pct" | "--flag" | "--stash" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    let parsed = match flag.as_str() {
                        "--exp-test" => field_tick::Action::parse_exp(item),
                        "--drop-money" => field_tick::Action::parse_money(item),
                        "--drop-item" => field_tick::Action::parse_item(item),
                        "--stat-test" => field_tick::Action::parse_stat(item),
                        "--quick-set" => field_tick::Action::parse_quick(item),
                        "--warp" => field_tick::Action::parse_warp(item),
                        "--equip-test" => field_tick::Action::parse_equip(item),
                        "--quest-set" => field_tick::Action::parse_quest(item),
                        "--hit" => field_tick::Action::parse_hit(item),
                        "--item-icons" => field_tick::Action::parse_item_icons(item),
                        "--mobs" => field_tick::Action::parse_mobs(item),
                        "--kill" => field_tick::Action::parse_kill(item),
                        "--item-grade" => field_tick::Action::parse_grade(item),
                        "--round" => field_tick::Action::parse_round(item),
                        "--job" => field_tick::Action::parse_job(item),
                        "--skill-level" => field_tick::Action::parse_skill_level(item),
                        "--base-stat" => field_tick::Action::parse_base_stat(item),
                        "--spirit-level" => field_tick::Action::parse_spirit_level(item),
                        "--refill" => field_tick::Action::parse_refill(item),
                        "--walk-dir" => field_tick::Action::parse_walk_dir(item),
                        "--recipe-test" => field_tick::Action::parse_recipe(item),
                        "--give" => field_tick::Action::parse_give(item),
                        "--bags" => field_tick::Action::parse_bags(item),
                        "--item-check" => field_tick::Action::parse_item_check(item),
                        "--hp-pct" => field_tick::Action::parse_hp_pct(item),
                        "--flag" => field_tick::Action::parse_flag(item),
                        "--stash" => field_tick::Action::parse_stash(item),
                        _ => field_tick::Action::parse_drop_test(item),
                    };
                    match parsed {
                        Some(action) => a.actions.push(action),
                        None => errs.push(format!("{flag} {item}: bad value")),
                    }
                }
            }
            "--opt" => {
                if let Err(e) = a.options.apply_spec(v) {
                    errs.push(format!("--opt: {e}"));
                }
            }
            "--dump" => {
                for f in v.split(',').filter(|s| !s.is_empty()) {
                    match f.parse() {
                        Ok(n) => a.dump.push(n),
                        Err(_) => errs.push(format!("--dump {f}: not a frame number")),
                    }
                }
            }
            "--keys" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    let parts: Vec<&str> = item.split(':').collect();
                    let frame = parts.first().and_then(|s| s.parse::<u64>().ok());
                    let key = parts.get(1).and_then(|s| parse_key(s));
                    let hold = parts.get(2).map_or(Some(3), |s| s.parse::<u64>().ok());
                    match (frame, key, hold, parts.len() <= 3) {
                        (Some(f), Some(k), Some(h), true) => a.keys.push((f, k, h.max(1))),
                        _ => errs.push(format!("--keys {item}: expected frame:KEY[:hold]")),
                    }
                }
            }
            other => errs.push(format!("unknown flag {other}")),
        }
    }
    match jar {
        Some(j) => a.jar = j,
        None => errs.push("--jar <game.jar> is required".to_string()),
    }
    if errs.is_empty() { Ok(a) } else { Err(errs) }
}

// ------------------------------------------------------------------ main ----

fn write_png(path: &Path, f: &Frame) -> Result<(), String> {
    image::save_buffer(path, &f.rgba, f.w, f.h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())
}

fn non_black(f: &Frame) -> usize {
    f.rgba.chunks_exact(4).filter(|p| p[0] | p[1] | p[2] != 0).count()
}

fn main() {
    // The rotation bench (tools/hz_rotation.py): every hit, hit taken and kill logged (`hz: bench …`).
    if std::env::var_os("HZ_BENCH").is_some() {
        hz_java::effects::BENCH.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,hz_java=info,hz_probe=info".into()))
        .with_writer(std::io::stderr)
        .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(errs) => {
            for e in errs {
                eprintln!("hz_probe: {e}");
            }
            eprintln!("usage: hz_probe --jar <game.jar> [--w 240 --h 320 --ms 20 --frames 600 --keys f:KEY[:hold],.. --dump f,.. --every N --out dir]");
            std::process::exit(2);
        }
    };

    let jar = fs::read(&args.jar).unwrap_or_else(|e| {
        eprintln!("hz_probe: {}: {e}", args.jar.display());
        std::process::exit(2);
    });
    eprintln!("[probe] jar {} — {} B, md5 {:x}", args.jar.display(), jar.len(), md5::compute(&jar));
    let patches = match Build::identify(&jar) {
        Some(build) => {
            let p = Patches::new(build, &args.patches);
            eprintln!("[probe] build: {} — patches {:?}: {}", build.label(), args.patches, p.chosen().map(|x| x.id).collect::<Vec<_>>().join(" "));
            Some(p)
        }
        None => {
            eprintln!("[probe] UNKNOWN BUILD — booting with no patches at all (hz_patches only knows its own jars)");
            None
        }
    };
    fs::create_dir_all(&args.out).expect("create --out");
    let _ = field_tick::OUT.set(args.out.clone());

    let clock = Arc::new(AtomicU64::new(0));
    let last = Arc::new(Mutex::new(Frame::default()));
    let redraw = Arc::new(AtomicBool::new(false));
    let paints = Arc::new(AtomicU64::new(0));
    let exited = Arc::new(AtomicBool::new(false));
    let rms = Arc::new(MemRms::default());
    let audio = NoAudio::default();

    if let Some(dir) = &args.rms_in {
        match rms.preload(dir) {
            Ok(n) => eprintln!("[probe] preloaded {n} RMS record(s) from {} (read-only)", dir.display()),
            Err(e) => {
                eprintln!("hz_probe: --rms-in {}: {e}", dir.display());
                std::process::exit(2);
            }
        }
    }

    let platform = ProbePlatform {
        screen: ProbeScreen { w: args.w, h: args.h, last: last.clone(), redraw: redraw.clone(), paints: paints.clone() },
        fs: MemFs::default(),
        rms: rms.clone(),
        audio: audio.clone(),
        clock: clock.clone(),
        exited: exited.clone(),
    };

    let game_options = Arc::new(args.options);
    // The frame the loop is on, for `--exp-test` / `--drop-money` (read on the game thread).
    let frame_now = Arc::new(AtomicU64::new(0));
    // The scene the HZ-008 blit hook reports (it logs every change as `hz: scene …`).
    let scene = Arc::new(hz_java::GuestState::new());
    if let Some(p) = &patches {
        scene.set_patches(p.chosen().map(|x| x.id));
    }
    scene.set_preferred_slot(args.prefer_slot.map(|n| n.saturating_sub(1)));
    scene.set_target_shift(args.target_shift);
    scene.set_target_drop(args.target_drop);
    if !args.labels.is_empty() {
        scene.ask_labels(args.labels.clone());
    }
    if !args.host_texts.is_empty() {
        scene.ask_texts(args.host_texts.iter().enumerate().map(|(i, b)| (i as u16, b.clone())));
    }
    if !args.place_maps.is_empty() {
        scene.ask_maps(args.place_maps.clone());
    }
    eprintln!(
        "[probe] options: {}",
        hz_java::Row::ALL.iter().map(|&r| format!("{r:?}={}", game_options.describe(r))).collect::<Vec<_>>().join(" ")
    );
    let options = match &patches {
        Some(p) => J2meOptions {
            property_overrides: p.property_overrides(),
            class_transformer: p.class_transformer().map(Arc::from),
            extra_protos: vec![field_tick::wrap(
                hz_java::hooks::proto(game_options.clone(), scene.clone()),
                args.actions.clone(),
                frame_now.clone(),
            )],
        },
        None => J2meOptions::default(),
    };
    let mut emu = match J2MEEmulator::from_jar_with(Box::new(platform), JAR_NAME, jar, options) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[probe] LOAD FAILED: {e:?}");
            std::process::exit(1);
        }
    };
    eprintln!("[probe] loaded as {JAR_NAME}, screen {}x{}, {} ms/frame", args.w, args.h, args.ms);

    let mut held: Vec<(u64, KeyCode)> = Vec::new();
    let mut keys: hz_java::input::KeyProjection<KeyCode> = hz_java::input::KeyProjection::new();
    // Key → drawn (2026-10-28): each press sent to the guest, with the key the canvas held on the
    // last drawn frame (`f.c`, read by the blit hook); resolved by the first frame drawn with
    // another — the game thread's own latency, in virtual ms. `(pressed at, key, held before)`.
    let mut lat_pending: Vec<(u64, KeyCode, i32, bool)> = Vec::new();
    let mut lat: Vec<u64> = Vec::new();
    let mut seen_paints = 0u64;
    let raw_keys = args.raw_keys;
    let send = |emu: &mut J2MEEmulator, out: Vec<hz_java::input::GuestKey<KeyCode>>| {
        for k in out {
            emu.handle_event(match k {
                hz_java::input::GuestKey::Down(c) => Event::Keydown(c),
                hz_java::input::GuestKey::Up(c) => Event::Keyup(c),
            });
        }
    };
    let mut last_paints = 0;
    // The STATS readout as the blit hook last read it: printed on the frame it changes (the
    // hook's own per-change line is debug, compiled out of this release build).
    let mut last_stats: Option<hz_java::Stats> = None;
    let mut host_texts: Vec<(u16, Option<hz_java::Icon>)> = Vec::new();
    let mut pickups: Vec<hz_java::Icon> = Vec::new();
    let started = std::time::Instant::now();
    // `--timing`: the window's tick wall time (sum, worst) and paints at its start.
    let (mut t_sum, mut t_worst, mut t_paints) = (std::time::Duration::ZERO, std::time::Duration::ZERO, 0u64);
    let mut t_worst_at = 0u64;
    let _ = hz_java::hooks::take_blit_gauge();
    wie_util::stats::set_gc_every(args.gc);
    #[cfg(all(windows, target_arch = "x86_64"))]
    let mut sampler: Option<sampler::Sampler> = None;
    for f in 1..=args.frames {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if f == args.sample_from
            && let Some(path) = &args.sample
        {
            match sampler::Sampler::start(path.clone()) {
                Ok(s) => {
                    eprintln!("[probe] f={f} sampling into {}", path.display());
                    sampler = Some(s);
                }
                Err(e) => eprintln!("[probe] sampler: {e}"),
            }
        }
        clock.fetch_add(args.ms, Ordering::SeqCst);
        frame_now.store(f, Ordering::SeqCst);

        for &(kf, key, hold) in &args.keys {
            if kf == f {
                if raw_keys {
                    emu.handle_event(Event::Keydown(key));
                } else {
                    let (out, diagonal) = keys.walk_edge(key, true, arrow);
                    scene.set_walk_diag(diagonal);
                    // An arrow that waits for a release to be taken is timed from here too (its
                    // first change drawn is that release's 0, not the press).
                    let waited = keys.waiting() == Some(key);
                    if !out.is_empty() || waited {
                        lat_pending.push((f, key, scene.key(), waited));
                    }
                    send(&mut emu, out);
                }
                held.push((f + hold, key));
            }
        }
        if args.save_ask.contains(&f) {
            scene.ask_save();
        }
        for &(_, cat, id, n) in args.rewards.iter().filter(|r| r.0 == f) {
            scene.reward_wait(cat, id, n);
            eprintln!("[probe] f={f} reward ({cat},{id}) x{n} waits for the next save");
        }
        for &(_, ask) in args.plates.iter().filter(|p| p.0 == f) {
            scene.ask_plate(ask);
        }
        for &(_, x, y) in args.taps.iter().filter(|t| t.0 == f) {
            scene.ask_tap(x, y);
        }
        for &(_, d) in args.pad_pages.iter().filter(|t| t.0 == f) {
            scene.ask_page(d);
        }
        let (due, still): (Vec<_>, Vec<_>) = std::mem::take(&mut held).into_iter().partition(|&(up, _)| up == f);
        held = still;
        for (_, key) in due {
            if raw_keys {
                emu.handle_event(Event::Keyup(key));
            } else {
                let (out, diagonal) = keys.walk_edge(key, false, arrow);
                scene.set_walk_diag(diagonal);
                send(&mut emu, out);
            }
        }

        let tick_at = std::time::Instant::now();
        if let Err(e) = emu.tick() {
            eprintln!("[probe] f={f} GUEST ERROR: {e:?}");
            break;
        }
        let took = tick_at.elapsed();
        t_sum += took;
        if took > t_worst {
            t_worst = took;
            t_worst_at = f;
        }
        if args.timing > 0 && f % args.timing == 0 {
            let (hook_frames, hook_ns) = hz_java::hooks::take_blit_gauge();
            let p = paints.load(Ordering::SeqCst);
            let (gc_calls, gc_runs) = wie_util::stats::gc_snapshot();
            eprintln!(
                "[probe] f={f} timing: tick avg {:.2} ms worst {:.2} ms (f={t_worst_at}) | hook {:.2} ms/paint over {hook_frames} | paints {} in {} frames | gc {gc_runs}/{gc_calls} | scene {:?}",
                t_sum.as_secs_f64() * 1000.0 / args.timing as f64,
                t_worst.as_secs_f64() * 1000.0,
                hook_ns as f64 / 1e6 / hook_frames.max(1) as f64,
                p - t_paints,
                args.timing,
                scene.view().scene
            );
            (t_sum, t_worst, t_paints) = (std::time::Duration::ZERO, std::time::Duration::ZERO, p);
        }
        // One projection frame per guest paint, as the APK's worker does.
        let painted = paints.load(Ordering::SeqCst);
        if !raw_keys {
            for _ in seen_paints..painted {
                send(&mut emu, keys.frame());
            }
        }
        seen_paints = painted;
        if painted > last_paints || !lat_pending.is_empty() {
            let now_key = scene.key();
            lat_pending.retain(|&(kf, key, before, waited)| {
                if painted > last_paints && now_key != before && !(waited && now_key == 0) {
                    let ms = (f - kf) * args.ms;
                    eprintln!("[probe] f={f} key {key:?} pressed f={kf} -> drawn f={f} (+{ms} virtual ms, held {now_key})");
                    lat.push(ms);
                    false
                } else {
                    // Never shown as a change (the same key again, a window that does not read it).
                    f - kf < 100
                }
            });
        }
        if redraw.swap(false, Ordering::SeqCst) {
            emu.handle_event(Event::Redraw);
        }
        if exited.load(Ordering::SeqCst) {
            eprintln!("[probe] f={f} guest exited");
            break;
        }
        if let Some(answer) = scene.take_save_answer() {
            eprintln!("[probe] f={f} SAVE tab -> {answer:?}");
        }
        for p in scene.take_pickups() {
            eprintln!("[probe] f={f} pickup notice {:?} {}x{}", p.key, p.w, p.h);
            pickups.push(p);
        }
        let lettered = scene.take_texts();
        if !lettered.is_empty() {
            eprintln!("[probe] f={f} host texts: {} lettered (mode {:?})", lettered.len(), scene.view().scene);
            host_texts.extend(lettered);
        }
        let now_stats = scene.stats();
        if now_stats != last_stats {
            if let Some(s) = now_stats {
                let (a, b) = s.lines();
                eprintln!("[probe] f={f} stats {a} | {b}");
            }
            last_stats = now_stats;
        }

        let wanted = args.dump.contains(&f) || (args.every > 0 && f % args.every == 0) || f == args.frames;
        if wanted {
            let frame = last.lock().unwrap().clone();
            let p = paints.load(Ordering::SeqCst);
            if frame.rgba.is_empty() {
                eprintln!("[probe] f={f} nothing painted yet (paints {p})");
            } else {
                let path = args.out.join(format!("frame_{f:05}.png"));
                let nb = non_black(&frame);
                match write_png(&path, &frame) {
                    Ok(()) => eprintln!(
                        "[probe] f={f} {}x{} non-black {nb} paints {p} (+{}) -> {}",
                        frame.w,
                        frame.h,
                        p - last_paints,
                        path.display()
                    ),
                    Err(e) => eprintln!("[probe] f={f} png write failed: {e}"),
                }
            }
            last_paints = p;
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    drop(sampler.take());
    if !lat.is_empty() {
        eprintln!(
            "[probe] key->drawn: {} presses, avg {:.1} virtual ms, min {} max {}",
            lat.len(),
            lat.iter().sum::<u64>() as f64 / lat.len() as f64,
            lat.iter().min().unwrap(),
            lat.iter().max().unwrap()
        );
    }
    eprintln!(
        "[probe] done: {} virtual ms, {:.1}s wall, paints {}, audio waves {} midi notes {} media starts {}",
        clock.load(Ordering::SeqCst),
        started.elapsed().as_secs_f64(),
        paints.load(Ordering::SeqCst),
        audio.waves.load(Ordering::Relaxed),
        audio.midi.load(Ordering::Relaxed),
        audio.media.load(Ordering::Relaxed)
    );
    for line in patches.iter().flat_map(|p| p.report()) {
        eprintln!("[patch] {line}");
    }
    // The quick-slot icons the HZ-008 hook read (Phase 6-2), 4x, so they can be looked at.
    if let Some(bar) = scene.quick() {
        eprintln!("[probe] quick slots at the end: {}", bar.summary());
    }
    for icon in scene.icons() {
        let name = format!("{:?}", icon.key).replace([' ', '{', '}', ':', ','], "").replace(['(', ')'], "_");
        let path = args.out.join(format!("icon_{name}.png"));
        let s = 4u32;
        let (w, h) = (icon.w * s, icon.h * s);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let p = icon.px[((y / s) * icon.w + x / s) as usize];
                rgba.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
            }
        }
        match image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8) {
            Ok(()) => eprintln!("[probe] icon {:?} {}x{} -> {}", icon.key, icon.w, icon.h, path.display()),
            Err(e) => eprintln!("[probe] icon {:?}: {e}", icon.key),
        }
    }
    // The portrait the hook composed (Phase 6-6), 8x.
    if let Some(p) = scene.portrait() {
        let path = args.out.join("portrait.png");
        let s = 8u32;
        let (w, h) = (p.w * s, p.h * s);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = p.px[((y / s) * p.w + x / s) as usize];
                rgba.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]);
            }
        }
        match image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8) {
            Ok(()) => eprintln!("[probe] portrait {:?} {}x{} -> {}", p.key, p.w, p.h, path.display()),
            Err(e) => eprintln!("[probe] portrait: {e}"),
        }
        // The pixels as text, for hz_android's preview (`render::tests::preview`).
        let text: String = p.px.chunks(p.w as usize).map(|row| row.iter().map(|c| format!("{c:08x}")).collect::<Vec<_>>().join(" ") + "\n").collect();
        if let Err(e) = fs::write(args.out.join("portrait.txt"), text) {
            eprintln!("[probe] portrait.txt: {e}");
        }
    }
    // The quest guidance the field computed (Phase 7-6).
    if let Some(g) = scene.guide() {
        eprintln!("[probe] quest guide: quest {:?} -> map {:?}, guide tiles {:?}", g.quest, g.target, g.tiles);
    }
    // The maps `--place-maps` asked for (Phase 7-5): one line each.
    if let Some(maps) = scene.take_maps() {
        for (id, g) in args.place_maps.iter().zip(&maps) {
            match g {
                Some(g) => eprintln!("[probe] place map {id}: {}x{} (floor, exits, chests) {:?}", g.w, g.h, g.census()),
                None => eprintln!("[probe] place map {id}: not read"),
            }
        }
    }
    // The names `--labels` asked the game's font for (Phase 7-3): `label_N.png` (4x, black on
    // white) and `labels.txt` (`label N W H` then the rows in hex — hz_android's preview).
    if let Some(labels) = scene.take_labels() {
        let mut text = String::new();
        for (i, l) in labels.iter().enumerate() {
            let Some(l) = l else {
                eprintln!("[probe] label {i}: not drawn");
                continue;
            };
            let s = 4u32;
            let (w, h) = (l.w * s, l.h * s);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let ink = (l.px[((y / s) * l.w + x / s) as usize] >> 24) as u8;
                    rgba.extend([255 - ink, 255 - ink, 255 - ink, 255]);
                }
            }
            let path = args.out.join(format!("label_{i}.png"));
            match image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                Ok(()) => eprintln!("[probe] label {i} {}x{} -> {}", l.w, l.h, path.display()),
                Err(e) => eprintln!("[probe] label {i}: {e}"),
            }
            text += &format!("label {i} {} {}\n", l.w, l.h);
            text += &l.px.chunks(l.w as usize).map(|row| row.iter().map(|c| format!("{c:08x}")).collect::<Vec<_>>().join(" ") + "\n").collect::<String>();
        }
        if let Err(e) = fs::write(args.out.join("labels.txt"), text) {
            eprintln!("[probe] labels.txt: {e}");
        }
    }
    // The game's pickup notices the HZ-025 hook handed over: `pickup_N.png` (4x) and `pickups.txt`
    // (`pickup N W H` then the rows in hex — hz_android's preview stacks them).
    if !pickups.is_empty() {
        let mut text = String::new();
        for (i, p) in pickups.iter().enumerate() {
            let s = 4u32;
            let (w, h) = (p.w * s, p.h * s);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let c = p.px[((y / s) * p.w + x / s) as usize];
                    rgba.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]);
                }
            }
            if let Err(e) = image::save_buffer(args.out.join(format!("pickup_{i}.png")), &rgba, w, h, image::ExtendedColorType::Rgba8) {
                eprintln!("[probe] pickup {i}: {e}");
            }
            text += &format!("pickup {i} {} {}\n", p.w, p.h);
            text += &p.px.chunks(p.w as usize).map(|row| row.iter().map(|c| format!("{c:08x}")).collect::<Vec<_>>().join(" ") + "\n").collect::<String>();
        }
        eprintln!("[probe] pickup notices: {} -> pickups.txt", pickups.len());
        if let Err(e) = fs::write(args.out.join("pickups.txt"), text) {
            eprintln!("[probe] pickups.txt: {e}");
        }
    }
    // The host's lines `--host-texts` asked for (B1): `text_N.png` (4x) and `texts.txt`.
    if !host_texts.is_empty() {
        let mut text = String::new();
        for (i, l) in &host_texts {
            let Some(l) = l else {
                eprintln!("[probe] host text {i}: not drawn");
                continue;
            };
            let s = 4u32;
            let (w, h) = (l.w * s, l.h * s);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let ink = (l.px[((y / s) * l.w + x / s) as usize] >> 24) as u8;
                    rgba.extend([255 - ink, 255 - ink, 255 - ink, 255]);
                }
            }
            if let Err(e) = image::save_buffer(args.out.join(format!("text_{i}.png")), &rgba, w, h, image::ExtendedColorType::Rgba8) {
                eprintln!("[probe] host text {i}: {e}");
            }
            text += &format!("text {i} {} {}
", l.w, l.h);
            text += &l.px.chunks(l.w as usize).map(|row| row.iter().map(|c| format!("{c:08x}")).collect::<Vec<_>>().join(" ") + "
").collect::<String>();
        }
        eprintln!("[probe] host texts: {} of {} drawn -> texts.txt", host_texts.iter().filter(|t| t.1.is_some()).count(), host_texts.len());
        if let Err(e) = fs::write(args.out.join("texts.txt"), text) {
            eprintln!("[probe] texts.txt: {e}");
        }
    }
    // The minimap the hook read (Phase 6-5): the field panel's tile classes 4x — wall dark, floor
    // grey, a way to another map blue, closed chest green — with the monsters red and the player
    // teal, as the HUD marks them.
    if let Some(grid) = scene.map() {
        let marks = scene.marks();
        let s = 4u32;
        let (w, h) = (u32::from(grid.w) * s, u32::from(grid.h) * s);
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        let mut put = |x: u32, y: u32, c: [u8; 3]| {
            if x < w && y < h {
                let i = ((y * w + x) * 3) as usize;
                rgb[i..i + 3].copy_from_slice(&c);
            }
        };
        for ty in 0..u32::from(grid.h) {
            for tx in 0..u32::from(grid.w) {
                let c = match grid.doors()[(ty * u32::from(grid.w) + tx) as usize] {
                    hz_java::state::MM_FLOOR => [110, 110, 110],
                    hz_java::state::MM_EXIT => [57, 125, 255],
                    hz_java::state::MM_CHEST => [0, 226, 72],
                    _ => [24, 24, 24],
                };
                for dy in 0..s {
                    for dx in 0..s {
                        put(tx * s + dx, ty * s + dy, c);
                    }
                }
            }
        }
        let dot = |put: &mut dyn FnMut(u32, u32, [u8; 3]), (mx, my): (f32, f32), c: [u8; 3]| {
            let (cx, cy) = ((mx * s as f32) as i64, (my * s as f32) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if cx + dx >= 0 && cy + dy >= 0 {
                        put((cx + dx) as u32, (cy + dy) as u32, c);
                    }
                }
            }
        };
        for &m in &marks.mobs {
            dot(&mut put, m, [239, 24, 33]);
        }
        if let Some(p) = marks.player {
            dot(&mut put, p, [80, 220, 220]);
        }
        // The same as text, for hz_android's preview (`render::tests::preview`).
        let mut text = format!("map {} {} {}\n", grid.id, grid.w, grid.h);
        for row in grid.cells.chunks(usize::from(grid.w)) {
            text.extend(row.iter().map(|&c| char::from(b'0' + c)));
            text.push('\n');
        }
        // The field panel's classes where they differ: only the triggers that leave the map.
        if grid.doors() != &grid.cells[..] {
            text.push_str("doors\n");
            for row in grid.doors().chunks(usize::from(grid.w)) {
                text.extend(row.iter().map(|&c| char::from(b'0' + c)));
                text.push('\n');
            }
        }
        if let Some((x, y)) = marks.player {
            text.push_str(&format!("player {x} {y}\n"));
        }
        for (x, y) in &marks.mobs {
            text.push_str(&format!("mob {x} {y}\n"));
        }
        // The game window's title for the map (7-5), lettered by the hook: `title W H` + hex rows,
        // and `map_title.png` (4x, black on white).
        if let Some(t) = &grid.title {
            text += &format!("title {} {}\n", t.w, t.h);
            text += &t.px.chunks(t.w as usize).map(|row| row.iter().map(|c| format!("{c:08x}")).collect::<Vec<_>>().join(" ") + "\n").collect::<String>();
            let s = 4u32;
            let (w, h) = (t.w * s, t.h * s);
            let rgba: Vec<u8> = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    let ink = (t.px[((y / s) * t.w + x / s) as usize] >> 24) as u8;
                    [255 - ink, 255 - ink, 255 - ink, 255]
                })
                .collect();
            let path = args.out.join("map_title.png");
            match image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                Ok(()) => eprintln!("[probe] map title {}x{} -> {}", t.w, t.h, path.display()),
                Err(e) => eprintln!("[probe] map title: {e}"),
            }
        } else {
            eprintln!("[probe] map title: not lettered");
        }
        if let Err(e) = fs::write(args.out.join("minimap.txt"), text) {
            eprintln!("[probe] minimap.txt: {e}");
        }
        let path = args.out.join("minimap.png");
        let (floor, exits, chests) = grid.census();
        let doors = grid.doors().iter().filter(|&&c| c == hz_java::state::MM_EXIT).count();
        match image::save_buffer(&path, &rgb, w, h, image::ExtendedColorType::Rgb8) {
            Ok(()) => eprintln!(
                "[probe] minimap: map {} {}x{} floor {floor} exit {exits} (doors {doors}, leaving triggers {:?}) chest {chests}, player {:?}, {} monster(s) -> {}",
                grid.id,
                grid.w,
                grid.h,
                grid.leaving,
                marks.player,
                marks.mobs.len(),
                path.display()
            ),
            Err(e) => eprintln!("[probe] minimap: {e}"),
        }
    }
    for line in rms.summary() {
        eprintln!("[rms] {line}");
    }
    if let Some(dir) = &args.rms_out {
        match rms.dump(dir) {
            Ok(()) => eprintln!("[probe] RecordStores written to {}", dir.display()),
            Err(e) => eprintln!("[probe] --rms-out {}: {e}", dir.display()),
        }
    }
}
