//! `Platform` for Android: the screen hands finished frames to the UI thread, the RecordStores
//! live in app-private storage ([`crate::database`]), the effects play through [`crate::audio`]
//! (Phase 4a; the music is still silent).

use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use winit::event_loop::EventLoopProxy;

use wie_backend::{AudioSink, DatabaseRepository, Filesystem, Instant, Platform, Screen, canvas::Image};

use crate::{
    UserEvent,
    audio::{AndroidAudioSink, WaveMsg, spawn_audio_thread},
    database::AndroidDatabaseRepository,
    music::MusicMsg,
};

pub struct AndroidPlatform {
    screen: AndroidScreen,
    filesystem: AndroidFilesystem,
    database: AndroidDatabaseRepository,
    proxy: EventLoopProxy<UserEvent>,
    /// The audio thread's inboxes (effects, songs); every `audio_sink()` gets a clone.
    wave_tx: Sender<WaveMsg>,
    music_tx: Sender<MusicMsg>,
}

impl AndroidPlatform {
    /// Built on the worker thread: the audio thread it spawns opens AAudio through cpal, which
    /// needs `ndk_context` — populated by then. `paused` is the app's background flag;
    /// `renditions` the WAVs rendered from the game's MIDI files (Phase 4b).
    pub fn new(
        screen: AndroidScreen,
        data_dir: PathBuf,
        proxy: EventLoopProxy<UserEvent>,
        paused: Arc<AtomicBool>,
        renditions: Vec<Arc<[u8]>>,
    ) -> Self {
        let (wave_tx, music_tx) = spawn_audio_thread(paused, renditions);
        let notice = proxy.clone();
        Self {
            screen,
            filesystem: AndroidFilesystem { base: data_dir.clone() },
            database: AndroidDatabaseRepository::new(
                data_dir,
                Arc::new(move |text| {
                    let _ = notice.send_event(UserEvent::Notice(text));
                }),
            ),
            proxy,
            wave_tx,
            music_tx,
        }
    }

    /// The failed save writes since boot ([`AndroidDatabaseRepository::save_failures`]).
    pub fn save_failures(&self) -> Arc<AtomicU32> {
        self.database.save_failures()
    }
}

impl Platform for AndroidPlatform {
    fn screen(&self) -> &dyn Screen {
        &self.screen
    }

    fn now(&self) -> Instant {
        Instant::from_epoch_millis(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
    }

    fn database_repository(&self) -> &dyn DatabaseRepository {
        &self.database
    }

    fn filesystem(&self) -> &dyn Filesystem {
        &self.filesystem
    }

    fn audio_sink(&self) -> Box<dyn AudioSink> {
        Box::new(AndroidAudioSink::new(self.wave_tx.clone(), self.music_tx.clone()))
    }

    fn write_stdout(&self, buf: &[u8]) {
        log::info!("[guest] {}", String::from_utf8_lossy(buf).trim_end());
    }

    fn write_stderr(&self, buf: &[u8]) {
        log::warn!("[guest err] {}", String::from_utf8_lossy(buf).trim_end());
    }

    fn exit(&self) {
        log::info!("guest requested exit");
        let _ = self.proxy.send_event(UserEvent::GuestExit);
    }

    /// The emulator thread: a `try_send` and back to the game (C1, `crate::vibration`).
    fn vibrate(&self, duration_ms: u64, _intensity: u8) {
        log::debug!("vibrate {duration_ms} ms");
        crate::vibration::vibrate(duration_ms);
    }
}

/// The guest's screen. `paint` converts the finished frame and leaves it in `frame` for the worker
/// to post to the UI thread, unless it is byte-identical to the last one — the game repaints on
/// its timer whether or not anything moved, and an unchanged frame is a whole convert + send +
/// blit + present for nothing.
///
/// ★ 2026-10-27: left, not posted: the worker posts it after the tick, behind what the same paint
/// reported (stats, quick slots, minimap marks …). Posted from here it went first, the rest came
/// at the tick's end, and the UI thread presented twice for one game frame.
pub struct AndroidScreen {
    should_redraw: Arc<AtomicBool>,
    width: u32,
    height: u32,
    last_hash: AtomicU64,
    /// Every guest paint, identical or not: the game's frame clock (`hz_java::input`).
    paints: Arc<AtomicU64>,
    /// The newest changed frame, until the worker takes it.
    frame: Arc<std::sync::Mutex<Option<Vec<u32>>>>,
}

impl AndroidScreen {
    pub fn new(should_redraw: Arc<AtomicBool>, paints: Arc<AtomicU64>, frame: Arc<std::sync::Mutex<Option<Vec<u32>>>>, width: u32, height: u32) -> Self {
        Self { should_redraw, width, height, last_hash: AtomicU64::new(0), paints, frame }
    }
}

/// FNV-1a over 8-byte words: a fast content fingerprint, not a cryptographic one. A collision shows
/// one stale frame until the next change.
fn frame_hash(bytes: &[u8]) -> u64 {
    const PRIME: u64 = 0x100_0000_01b3;
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ bytes.len() as u64;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(PRIME);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rem.len()].copy_from_slice(rem);
        h = (h ^ u64::from_le_bytes(tail)).wrapping_mul(PRIME);
    }
    h
}

impl Screen for AndroidScreen {
    fn request_redraw(&self) -> wie_util::Result<()> {
        self.should_redraw.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn paint(&self, image: &dyn Image) {
        self.paints.fetch_add(1, Ordering::Relaxed);
        let raw = image.raw();
        let bpp = image.bytes_per_pixel();
        if bpp == 2 || bpp == 4 {
            let h = frame_hash(&raw);
            if self.last_hash.swap(h, Ordering::Relaxed) == h {
                return;
            }
        } else {
            self.last_hash.store(0, Ordering::Relaxed);
        }
        // A J2ME screen is a 4-byte ARGB image (`Image.createImage(II)`): each pixel IS the host
        // word, little-endian. 2-byte RGB565 and anything else go through `colors()`.
        let data: Vec<u32> = if bpp == 4 {
            raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
        } else {
            image.colors().iter().map(|c| 0xff00_0000 | (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)).collect()
        };
        if let Ok(mut slot) = self.frame.lock() {
            *slot = Some(data);
        }
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}

/// App files under `<data>/<aid>/fs/`. A J2ME MIDlet reads its resources from the jar and saves
/// through the RecordStores, so this is here for the runtime, not for the game.
struct AndroidFilesystem {
    base: PathBuf,
}

impl AndroidFilesystem {
    fn path_for(&self, aid: &str, path: &str) -> Option<PathBuf> {
        let aid: String = aid.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
        if aid.is_empty() || aid == "." || aid == ".." {
            return None;
        }
        let mut rel = PathBuf::new();
        for c in Path::new(path).components() {
            match c {
                Component::Normal(s) => rel.push(s),
                Component::CurDir => {}
                _ => {
                    log::error!("filesystem: rejected path {path:?}");
                    return None;
                }
            }
        }
        (!rel.as_os_str().is_empty()).then(|| self.base.join(aid).join("fs").join(rel))
    }
}

#[async_trait::async_trait]
impl Filesystem for AndroidFilesystem {
    async fn exists(&self, aid: &str, path: &str) -> bool {
        self.path_for(aid, path).is_some_and(|p| p.is_file())
    }

    async fn size(&self, aid: &str, path: &str) -> Option<usize> {
        let md = self.path_for(aid, path)?.metadata().ok()?;
        md.is_file().then_some(md.len() as usize)
    }

    async fn read(&self, aid: &str, path: &str, offset: usize, count: usize, buf: &mut [u8]) -> Option<usize> {
        let mut f = fs::File::open(self.path_for(aid, path)?).ok()?;
        f.seek(SeekFrom::Start(offset as u64)).ok()?;
        let n = count.min(buf.len());
        let mut read = 0;
        while read < n {
            match f.read(&mut buf[read..n]) {
                Ok(0) => break,
                Ok(k) => read += k,
                Err(_) => break,
            }
        }
        Some(read)
    }

    async fn write(&self, aid: &str, path: &str, offset: usize, data: &[u8]) -> usize {
        let Some(p) = self.path_for(aid, path) else { return 0 };
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(mut f) = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&p) else {
            return 0;
        };
        if f.seek(SeekFrom::Start(offset as u64)).is_err() || f.write_all(data).is_err() {
            return 0;
        }
        data.len()
    }

    async fn truncate(&self, aid: &str, path: &str, len: usize) {
        if let Some(p) = self.path_for(aid, path)
            && let Ok(f) = fs::OpenOptions::new().write(true).create(true).truncate(false).open(&p)
        {
            let _ = f.set_len(len as u64);
        }
    }
}
