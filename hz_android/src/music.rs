//! The game's music — Phase 4c (2026-10-22): the AAC songs of `/snd/b/*`.
//!
//! What arrives: the core's `net.wie.MediaPlayer` hands each MP4 or MIDI player to the sink as a
//! MEDIA STREAM (`AudioSink::media_start` / `media_stop` / `media_level` / `media_close`) — the
//! whole file, an id, the MIDP loop count and the `VolumeControl` level. Sixteen of the game's
//! seventeen songs are AAC-LC, 44.1 kHz stereo, ~128 kbps, in MP4 (`ftyp M4A`), 41 s to 3 min,
//! played with `setLoopCount(-1)`; `/snd/b/15` is a 3.7 s MIDI fanfare — Phase 4b (2026-10-22):
//! played from a RENDITION, a WAV the build renders from that MIDI through a GM SoundFont
//! (`hz_render`, user decision: no synth in the APK) and the app ships as an asset. The music
//! thread holds the renditions it was given; a MIDI stream whose bytes hash to one's `hzmd` key
//! plays the WAV ([`WavSource`]), any other MIDI is silent (and still stops the song it
//! replaces, as on the handset).
//!
//!   * [`AndroidAudioSink`](crate::audio::AndroidAudioSink) turns those calls into [`MusicMsg`]s.
//!   * The **wie-music thread** ([`music_loop`]) owns the one stream: it decodes a packet at a
//!     time ([`AacSource`], symphonia's MP4 reader and AAC decoder — pure Rust), converts it to
//!     the device rate ([`Resampler`]) and queues [`MusicChunk`]s for the bus, a bounded queue
//!     of about half a second, so the decoder runs ahead of the device and never inside its
//!     callback. A loop is the same stream going on: the file is reopened and the resampler
//!     never notices.
//!   * The bus reads them through [`MusicIn`], the only part on the device's callback: it plays
//!     the current stream's chunks at [`MUSIC_GAIN`] × the game's level, and every change —
//!     a new song, a stop, a pause, the app going to the background — is a [`MUSIC_FADE_MS`]
//!     ramp instead of a cut. A stopped (MIDP `stop()`) or backgrounded stream HOLDS its place,
//!     queue and all, and goes on from exactly there.
//!   * A `close()` holds the place too, and a `start()` of ANOTHER Player on the SAME FILE goes
//!     on from it (user, 2026-10-23): the game leaves every room of a building through
//!     `stop(); close(); new Player; start()` on the building's one song, and a handset played it
//!     from the top each time. The bus also keeps playing a stopped song for
//!     [`MUSIC_STOP_GRACE_MS`], and a closed one for [`MUSIC_CLOSE_GRACE_MS`], before it ramps
//!     out — long enough for the map load, so the same song started again after it has no dip at
//!     all.
//!
//! The thread and the bus agree through [`MusicShared`]: the stream generation (a chunk of an
//! older one is stale and fades out), the held flag, the level. One stream at a time — a handset
//! had one media decoder, and this game starts a new song on a map change without stopping the
//! old one when an effect was the last thing it played.
//!
//! Everything but the rodio glue is host-tested, with synthetic sources; the game's own songs
//! stay out of the repo (an `#[ignore]`d test reads them from `HZ_MUSIC_DIR`).

use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    time::{Duration, Instant},
};

use symphonia::{
    core::{
        audio::SampleBuffer,
        codecs::{CODEC_TYPE_AAC, Decoder, DecoderOptions},
        errors::Error as DecodeFailure,
        formats::{FormatOptions, FormatReader},
        io::MediaSourceStream,
    },
    default::{codecs::AacDecoder, formats::IsoMp4Reader},
};

use crate::{
    audio_mix::{MUSIC_CLOSE_GRACE_MS, MUSIC_FADE_MS, MUSIC_GAIN, MUSIC_STOP_GRACE_MS},
    music_air::{MUSIC_AIR, MusicAir},
    music_resample::Resampler,
};

/// What the core's sink tells the music thread — `AudioSink::media_*`, one for one.
#[derive(Debug)]
pub enum MusicMsg {
    Start { id: u32, data: Arc<[u8]>, loops: i32, level: u8 },
    Stop { id: u32 },
    Level { id: u32, level: u8 },
    Close { id: u32 },
}

/// What the music thread and the bus share.
#[derive(Default)]
pub struct MusicShared {
    /// The stream the bus may play; a chunk of an older generation is stale.
    generation: AtomicU64,
    /// The stream is stopped (MIDP `stop()`): the bus ramps out and holds its place.
    held: AtomicBool,
    /// Its Player is closed too: the game is on its way to the next song ([`MUSIC_CLOSE_GRACE_MS`]).
    closed: AtomicBool,
    /// The game's `VolumeControl` level for the stream, `0..=100`.
    level: AtomicU8,
    /// The thread has produced the stream's last chunk: running out after that is the end of
    /// the song, not an underrun.
    ended: AtomicBool,
    /// Times the bus exhausted a chunk of the live stream and found the queue empty — the
    /// decoder behind the device. Counted by the bus, reported by the thread (`logcat -s hz`:
    /// `music: the bus ran dry …`); 0 is the measurement that the music plays unbroken.
    underruns: AtomicU32,
}

impl MusicShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Times the bus ran dry mid-stream so far.
    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }
}

/// Interleaved stereo at the device rate, one decoded packet's worth, of stream `generation`.
pub struct MusicChunk {
    pub generation: u64,
    pub frames: Vec<f32>,
}

/// A decoded song, a packet at a time.
pub trait MusicSource: Send {
    fn sample_rate(&self) -> u32;
    /// Append the next packet's frames (mono doubled, beyond two channels the first two).
    /// `false` at the end of the file.
    fn read(&mut self, out: &mut Vec<[f32; 2]>) -> bool;
}

/// The source for a file the core handed over, by its magic — the declared type is `audio/midi`
/// for every song in this game. No renditions: a MIDI file is silent.
pub fn open_source(data: &Arc<[u8]>) -> Result<Box<dyn MusicSource>, String> {
    open_source_with(&[], data)
}

/// [`open_source`] with the renditions the build shipped: a MIDI file plays the WAV rendered from
/// it, if there is one.
pub fn open_source_with(renditions: &[Arc<[u8]>], data: &Arc<[u8]>) -> Result<Box<dyn MusicSource>, String> {
    match data.get(..8) {
        Some([_, _, _, _, b'f', b't', b'y', b'p']) => Ok(Box::new(AacSource::open(data.clone())?)),
        Some([b'M', b'T', b'h', b'd', ..]) => {
            let key = rendition_key(data);
            match renditions.iter().find_map(|wav| WavSource::open(wav).ok().filter(|w| w.key == Some(key))) {
                Some(wav) => Ok(Box::new(wav)),
                None => Err(format!("a MIDI song without a rendition (key {key:016x}): silent")),
            }
        }
        _ => Err("not a format the music thread decodes".into()),
    }
}

/// FNV-1a 64 of a file's bytes — what `hz_render` writes into a rendition's `hzmd` chunk.
pub fn rendition_key(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// A rendition: 16-bit PCM WAV (mono or stereo), with the `hzmd` chunk naming its source.
pub struct WavSource {
    frames: Vec<[f32; 2]>,
    pos: usize,
    rate: u32,
    /// The source file's [`rendition_key`], if the WAV carries one.
    pub key: Option<u64>,
}

impl WavSource {
    pub fn open(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err("not a WAV".into());
        }
        let le16 = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let le32 = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let (mut fmt, mut key, mut pcm) = (None, None, None);
        let mut pos = 12;
        while pos + 8 <= data.len() {
            let size = le32(pos + 4).ok_or("short chunk")? as usize;
            let body = pos + 8;
            match &data[pos..pos + 4] {
                b"fmt " => fmt = Some((le16(body).ok_or("short fmt")?, le16(body + 2).ok_or("short fmt")?, le32(body + 4).ok_or("short fmt")?, le16(body + 14).ok_or("short fmt")?)),
                b"hzmd" if size == 8 => key = data.get(body..body + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())),
                b"data" => pcm = Some(&data[body..(body + size).min(data.len())]),
                _ => {}
            }
            pos = body.checked_add(size).ok_or("bad chunk size")?.checked_add(size & 1).ok_or("bad chunk size")?;
        }
        let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
        let pcm = pcm.ok_or("no data chunk")?;
        if tag != 1 || bits != 16 || !(1..=2).contains(&channels) || rate == 0 {
            return Err(format!("not 16-bit PCM mono/stereo: tag {tag}, {channels} ch, {bits} bit"));
        }
        let samples: Vec<f32> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        let frames = samples.chunks_exact(channels as usize).map(|f| if channels == 1 { [f[0], f[0]] } else { [f[0], f[1]] }).collect();
        Ok(Self { frames, pos: 0, rate, key })
    }
}

impl MusicSource for WavSource {
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn read(&mut self, out: &mut Vec<[f32; 2]>) -> bool {
        if self.pos >= self.frames.len() {
            return false;
        }
        let end = (self.pos + 1024).min(self.frames.len());
        out.extend_from_slice(&self.frames[self.pos..end]);
        self.pos = end;
        true
    }
}

/// AAC in MP4: symphonia's reader and decoder over the shared bytes.
pub struct AacSource {
    reader: IsoMp4Reader,
    decoder: AacDecoder,
    track: u32,
    rate: u32,
    samples: Option<SampleBuffer<f32>>,
}

impl AacSource {
    pub fn open(data: Arc<[u8]>) -> Result<Self, String> {
        let stream = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());
        let reader = IsoMp4Reader::try_new(stream, &FormatOptions::default()).map_err(|e| format!("mp4: {e}"))?;
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_AAC)
            .ok_or("mp4: no AAC track")?;
        let rate = track.codec_params.sample_rate.filter(|&r| r > 0).ok_or("aac: no sample rate")?;
        let decoder = AacDecoder::try_new(&track.codec_params, &DecoderOptions::default()).map_err(|e| format!("aac: {e}"))?;
        let track = track.id;

        Ok(Self { reader, decoder, track, rate, samples: None })
    }
}

impl MusicSource for AacSource {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn read(&mut self, out: &mut Vec<[f32; 2]>) -> bool {
        loop {
            // The end of the file, or a container it cannot go on in: either way the pass is over.
            let Ok(packet) = self.reader.next_packet() else {
                return false;
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // One damaged packet costs its 23 ms, not the song.
                Err(DecodeFailure::DecodeError(_)) => continue,
                Err(_) => return false,
            };
            let spec = *decoded.spec();
            let channels = spec.channels.count();
            if decoded.frames() == 0 || channels == 0 {
                continue;
            }
            let needed = decoded.capacity() * channels;
            if self.samples.as_ref().is_none_or(|s| s.capacity() < needed) {
                self.samples = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
            }
            let samples = self.samples.as_mut().unwrap();
            samples.copy_interleaved_ref(decoded);
            out.extend(samples.samples().chunks_exact(channels).map(|f| if channels == 1 { [f[0], f[0]] } else { [f[0], f[1]] }));
            return true;
        }
    }
}

/// How long the thread waits before offering a chunk again to a FULL queue: the queue holds
/// ~0.5 s, so the bus is never close to empty when the thread comes back. Only then — a packet
/// is 23 ms of music, and waiting this long before every chunk fed the bus 23 ms per 40 ms:
/// the title song stuttered on the S25 (2026-10-23, the first time anyone listened).
const FULL_WAIT: Duration = Duration::from_millis(40);

/// How often at most the thread logs the bus running dry.
const DRY_LOG_EVERY: Duration = Duration::from_secs(1);

/// Chunks the bus may have queued: ~0.55 s of a 44.1 kHz song at 48 kHz (1,024-frame packets).
pub const MUSIC_QUEUE_CHUNKS: usize = 24;

/// Near-silence at a loop seam: below −60 dBFS on both channels.
const SEAM_QUIET: f32 = 1e-3;

/// The most the seam trims from either side of it, in frames at the song's rate (70 ms at
/// 44.1 kHz) — more quiet than that is a rest in the music, and plays.
const SEAM_TRIM_MAX: usize = 3_072;

/// The loop seam. Measured on the sixteen songs (`music_survey`, 2026-10-22): every pass starts
/// with 1,515–2,466 frames below −60 dBFS — the AAC encoder's priming, which the files' edit
/// lists do not declare (`media_time` 0) — and ends with 332–1,405, so a song looped as decoded
/// has a ~50 ms hole every time round. Between two passes this drops the quiet end of the first
/// and the quiet start of the next, each up to [`SEAM_TRIM_MAX`]; a song's very first start and
/// its very last end are left as they are. (Songs 3, 7 and 13 rest 0.4–1.4 s at an end: those
/// keep all but the trimmed 70 ms.)
#[derive(Default)]
struct Seam {
    /// The quiet end of the pass so far, held back: the seam drops it, anything louder after it
    /// plays it first.
    held: Vec<[f32; 2]>,
    /// Frames still to trim off the start of this pass (0 on a first pass).
    lead: usize,
}

impl Seam {
    fn quiet(f: &[f32; 2]) -> bool {
        f[0].abs() < SEAM_QUIET && f[1].abs() < SEAM_QUIET
    }

    /// One packet of the pass; what plays now goes to `out`. `looping`: another pass follows.
    fn take(&mut self, packet: &[[f32; 2]], looping: bool, out: &mut Vec<[f32; 2]>) {
        let mut p = packet;
        if self.lead > 0 {
            let quiet = p.iter().take_while(|f| Self::quiet(f)).count().min(self.lead);
            p = &p[quiet..];
            self.lead = if p.is_empty() { self.lead - quiet } else { 0 };
        }
        if !looping {
            out.append(&mut self.held);
            out.extend_from_slice(p);
            return;
        }
        let tail = p.iter().rev().take_while(|f| Self::quiet(f)).count();
        if tail < p.len() {
            out.append(&mut self.held);
            out.extend_from_slice(&p[..p.len() - tail]);
        }
        self.held.extend_from_slice(&p[p.len() - tail..]);
        if self.held.len() > SEAM_TRIM_MAX {
            let rest = self.held.len() - SEAM_TRIM_MAX;
            out.extend(self.held.drain(..rest));
        }
    }

    /// The file starts again: the held end is dropped, the coming start is trimmed.
    fn next_pass(&mut self) {
        self.held.clear();
        self.lead = SEAM_TRIM_MAX;
    }

    /// The stream ends here after all: the held end plays.
    fn finish(&mut self, out: &mut Vec<[f32; 2]>) {
        out.append(&mut self.held);
    }
}

/// The one stream the thread is decoding.
struct Stream {
    id: u32,
    data: Arc<[u8]>,
    source: Box<dyn MusicSource>,
    /// The top octave the codec cut ([`crate::music_air`]), then the device rate.
    air: MusicAir,
    resampler: Resampler,
    /// Passes left, the one playing included; negative = forever.
    loops: i32,
    /// MIDP `stop()`: the next `start()` of the same id goes on from here.
    paused: bool,
    /// MIDP `close()`: the id is gone, but the next `start()` of the same file goes on from here.
    closed: bool,
    ended: bool,
    generation: u64,
    /// Frames the current pass has produced — a pass of none ends a looping stream.
    pass_frames: usize,
    seam: Seam,
    /// A packet as decoded, then as it goes to the resampler.
    scratch: Vec<[f32; 2]>,
    played: Vec<[f32; 2]>,
}

/// Opens a file for the thread — [`open_source`], or a synthetic source in the tests.
pub type OpenSource<'a> = &'a dyn Fn(&Arc<[u8]>) -> Result<Box<dyn MusicSource>, String>;

impl Stream {
    /// The next chunk at the device rate, `None` once the last pass is out.
    fn next_chunk(&mut self, open: OpenSource) -> Option<MusicChunk> {
        let mut frames = Vec::new();
        while frames.is_empty() && !self.ended {
            self.scratch.clear();
            self.played.clear();
            if self.source.read(&mut self.scratch) {
                self.pass_frames += self.scratch.len();
                self.seam.take(&self.scratch, self.loops != 1, &mut self.played);
                self.air.process(&mut self.played);
                self.resampler.process(&self.played, &mut frames);
                continue;
            }
            // The end of a pass: the stream ends, or the file starts again under the resampler.
            let again = self.loops != 1 && self.pass_frames > 0;
            let reopened = if again { open(&self.data).ok() } else { None };
            match reopened {
                Some(source) => {
                    self.source = source;
                    self.pass_frames = 0;
                    self.seam.next_pass();
                    if self.loops > 1 {
                        self.loops -= 1;
                    }
                }
                None => {
                    self.seam.finish(&mut self.played);
                    self.air.process(&mut self.played);
                    self.resampler.process(&self.played, &mut frames);
                    self.resampler.flush(&mut frames);
                    self.ended = true;
                }
            }
        }

        (!frames.is_empty()).then_some(MusicChunk { generation: self.generation, frames })
    }
}

/// The thread's report of [`MusicShared::underruns`]: one warning per second at most, with the
/// count — a line in logcat is the evidence, since no one can hear a test.
#[derive(Default)]
struct DryLog {
    seen: u32,
    last: Option<Instant>,
}

impl DryLog {
    fn report(&mut self, shared: &MusicShared) {
        let n = shared.underruns();
        if n != self.seen && self.last.is_none_or(|t| t.elapsed() >= DRY_LOG_EVERY) {
            log::warn!("music: the bus ran dry {n} times so far ({} new): the decoder fell behind the device", n.wrapping_sub(self.seen));
            self.seen = n;
            self.last = Some(Instant::now());
        }
    }
}

/// The wie-music thread: applies the sink's messages, decodes ahead, feeds the bus. Returns when
/// either side is gone.
pub fn music_loop(ctrl: &Receiver<MusicMsg>, out: &SyncSender<MusicChunk>, shared: &MusicShared, device_rate: u32, open: OpenSource) {
    let mut stream: Option<Stream> = None;
    // A chunk the full queue would not take yet.
    let mut pending: Option<MusicChunk> = None;
    // The queue refused `pending`: wait [`FULL_WAIT`] before offering it again. Never before a
    // fresh chunk — that wait on every chunk is what starved the bus.
    let mut full = false;
    let mut dry = DryLog::default();
    loop {
        let decoding = stream.as_ref().is_some_and(|s| !s.ended);
        let msg = if full {
            match ctrl.recv_timeout(FULL_WAIT) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else if decoding || pending.is_some() {
            match ctrl.try_recv() {
                Ok(msg) => Some(msg),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match ctrl.recv() {
                Ok(msg) => Some(msg),
                Err(_) => return,
            }
        };
        dry.report(shared);
        // Every message first: a new song must not wait behind the old one's queue.
        if let Some(msg) = msg {
            apply(msg, &mut stream, &mut pending, shared, device_rate, open);
            full = false;
            continue;
        }
        if let Some(chunk) = pending.take() {
            match out.try_send(chunk) {
                Ok(()) => full = false,
                Err(TrySendError::Full(chunk)) => {
                    pending = Some(chunk);
                    full = true;
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
            continue;
        }
        pending = stream.as_mut().and_then(|s| s.next_chunk(open));
        if stream.as_ref().is_some_and(|s| s.ended) {
            shared.ended.store(true, Ordering::Release);
        }
    }
}

fn apply(msg: MusicMsg, stream: &mut Option<Stream>, pending: &mut Option<MusicChunk>, shared: &MusicShared, device_rate: u32, open: OpenSource) {
    let is = |stream: &Option<Stream>, id: u32| stream.as_ref().is_some_and(|s| s.id == id);
    match msg {
        MusicMsg::Start { id, data, loops, level } => {
            shared.level.store(level.min(100), Ordering::Relaxed);
            // MIDP `start()` after `stop()`: on from where it stopped. After the end: from the top.
            // Another Player on the same file (the next room of the building): on from here too.
            if let Some(s) = stream.as_mut().filter(|s| !s.ended && (s.id == id && s.paused || s.data == data)) {
                if s.id != id {
                    log::info!("music: stream {id} is the song of stream {} ({} bytes): going on from its place", s.id, data.len());
                    s.id = id;
                    s.loops = if loops == 0 { 1 } else { loops };
                }
                s.paused = false;
                s.closed = false;
                shared.held.store(false, Ordering::Release);
                shared.closed.store(false, Ordering::Release);
                return;
            }
            let generation = shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
            *pending = None;
            shared.held.store(false, Ordering::Release);
            shared.closed.store(false, Ordering::Release);
            shared.ended.store(false, Ordering::Release);
            *stream = match open(&data) {
                Ok(source) => {
                    log::info!(
                        "music: stream {id} starts, {} bytes, {} Hz -> {device_rate} Hz, loop count {loops}, level {level}",
                        data.len(),
                        source.sample_rate()
                    );
                    let air = MusicAir::new(source.sample_rate(), MUSIC_AIR);
                    let resampler = Resampler::new(source.sample_rate(), device_rate);
                    Some(Stream {
                        id,
                        data,
                        source,
                        air,
                        resampler,
                        loops: if loops == 0 { 1 } else { loops },
                        paused: false,
                        closed: false,
                        ended: false,
                        generation,
                        pass_frames: 0,
                        seam: Seam::default(),
                        scratch: Vec::new(),
                        played: Vec::new(),
                    })
                }
                Err(e) => {
                    log::info!("music: stream {id} ({} bytes) is silent: {e}", data.len());
                    None
                }
            };
        }
        MusicMsg::Stop { id } => {
            if let Some(s) = stream.as_mut().filter(|s| s.id == id) {
                s.paused = true;
                shared.held.store(true, Ordering::Release);
            }
        }
        MusicMsg::Level { id, level } => {
            if is(stream, id) {
                shared.level.store(level.min(100), Ordering::Relaxed);
            }
        }
        MusicMsg::Close { id } => {
            // The decoder is kept, place and queue: the next song may be this one again. Another
            // song's `start()` is what frees it.
            if let Some(s) = stream.as_mut().filter(|s| s.id == id) {
                s.paused = true;
                s.closed = true;
                shared.closed.store(true, Ordering::Release);
                shared.held.store(true, Ordering::Release);
            }
        }
    }
}

/// The bus's end of the music: the only part that runs on the device callback.
pub struct MusicIn {
    rx: Receiver<MusicChunk>,
    shared: Arc<MusicShared>,
    cur: Option<MusicChunk>,
    /// Next sample of `cur` (interleaved index).
    pos: usize,
    /// The envelope, `0..=1` per frame: it ramps to its target instead of jumping.
    env: f32,
    /// Envelope change per frame: a full swing in [`MUSIC_FADE_MS`].
    step: f32,
    /// Frames a held stream still plays before it ramps out ([`MUSIC_STOP_GRACE_MS`], or
    /// [`MUSIC_CLOSE_GRACE_MS`] once it is closed).
    grace: usize,
    grace_frames: usize,
    close_grace_frames: usize,
    /// `held` and `closed` as of the last call: the grace is armed on the stop and lengthened on
    /// the close, not on every call.
    was_held: bool,
    was_closed: bool,
}

impl MusicIn {
    pub fn new(rx: Receiver<MusicChunk>, shared: Arc<MusicShared>, sample_rate: u32) -> Self {
        let fade_frames = (sample_rate as f32 * MUSIC_FADE_MS as f32 / 1000.0).max(1.0);
        let frames_of = |ms: u32| (sample_rate as u64 * ms as u64 / 1000) as usize;
        Self {
            rx,
            shared,
            cur: None,
            pos: 0,
            env: 0.0,
            step: 1.0 / fade_frames,
            grace: 0,
            grace_frames: frames_of(MUSIC_STOP_GRACE_MS),
            close_grace_frames: frames_of(MUSIC_CLOSE_GRACE_MS),
            was_held: false,
            was_closed: false,
        }
    }

    /// Add the music into `buf` (interleaved stereo). `silenced` is the app in the background:
    /// the stream ramps out and holds its place, as it does for MIDP `stop()`.
    pub fn mix_into(&mut self, buf: &mut [f32], silenced: bool) {
        let generation = self.shared.generation.load(Ordering::Acquire);
        let held = self.shared.held.load(Ordering::Acquire);
        let closed = held && self.shared.closed.load(Ordering::Acquire);
        // A stop plays on through the grace, a close through the map load — the same song started
        // again within it is never heard to stop; the background does not wait. A close after the
        // stop's grace ran out does not bring the song back.
        if !held || silenced {
            self.grace = 0;
        } else if !self.was_held {
            self.grace = if closed { self.close_grace_frames } else { self.grace_frames };
        } else if closed && !self.was_closed && self.grace > 0 {
            self.grace += self.close_grace_frames.saturating_sub(self.grace_frames);
        }
        self.was_held = held;
        self.was_closed = closed;
        // The level is read once per call (~1.3 ms); the game only moves it between songs.
        let gain = MUSIC_GAIN * wie_backend::volume_gain(self.shared.level.load(Ordering::Relaxed));
        for frame in buf.chunks_exact_mut(2) {
            let play = !silenced && (!held || self.grace > 0);
            // A chunk of a replaced stream fades out before the next stream comes in.
            let stale = self.cur.as_ref().is_some_and(|c| c.generation < generation);
            let target = if play && !stale { 1.0 } else { 0.0 };
            if self.env <= 0.0 && target <= 0.0 {
                if !stale {
                    return; // stopped or backgrounded: keep the place
                }
                self.cur = None;
            }
            if !self.ready(generation) {
                // Nothing to play (no stream, its end, or the decoder behind): come back from 0.
                self.env = 0.0;
                return;
            }
            self.env += (target - self.env).clamp(-self.step, self.step);
            let g = self.env * gain;
            let frames = &self.cur.as_ref().unwrap().frames;
            frame[0] += frames[self.pos] * g;
            frame[1] += frames[self.pos + 1] * g;
            self.pos += 2;
            if held && self.grace > 0 {
                self.grace -= 1;
            }
        }
    }

    /// Make sure `cur` has a frame at `pos`, taking the next chunk of the current stream and
    /// dropping stale ones — except that a fade-out in progress goes on into its own stream's
    /// next chunk. `false` if there is nothing yet.
    fn ready(&mut self, generation: u64) -> bool {
        if self.cur.as_ref().is_some_and(|c| self.pos + 1 < c.frames.len()) {
            return true;
        }
        // A chunk of the live stream just ran out: if nothing follows it, that is an underrun
        // (counted once per dry spell — `cur` is gone on the next call) unless the song ended.
        let exhausted = self.cur.as_ref().is_some_and(|c| c.generation >= generation);
        let fading = self.cur.take().filter(|c| c.generation < generation && self.env > 0.0).map(|c| c.generation);
        while let Ok(chunk) = self.rx.try_recv() {
            let current = chunk.generation >= generation;
            if chunk.frames.len() >= 2 && (current || fading == Some(chunk.generation)) {
                if current && fading.is_some() {
                    self.env = 0.0; // the old stream ran out mid-fade
                }
                self.cur = Some(chunk);
                self.pos = 0;
                return true;
            }
        }
        if exhausted && !self.shared.ended.load(Ordering::Acquire) {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        false
    }

    /// The envelope now (tests).
    pub fn envelope(&self) -> f32 {
        self.env
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, sync_channel};

    use super::*;

    /// A source of `frames` frames of a constant `value` per pass, at `rate`.
    struct Flat {
        rate: u32,
        left: usize,
        value: f32,
    }

    impl MusicSource for Flat {
        fn sample_rate(&self) -> u32 {
            self.rate
        }
        fn read(&mut self, out: &mut Vec<[f32; 2]>) -> bool {
            if self.left == 0 {
                return false;
            }
            let n = self.left.min(100);
            self.left -= n;
            out.extend(std::iter::repeat_n([self.value, -self.value], n));
            true
        }
    }

    /// Files for the fake opener: byte 0 = frames per pass / 100, byte 1 = value × 100.
    fn flat_open(data: &Arc<[u8]>) -> Result<Box<dyn MusicSource>, String> {
        match data.first() {
            Some(&n) => Ok(Box::new(Flat { rate: 48_000, left: n as usize * 100, value: data[1] as f32 / 100.0 })),
            None => Err("empty".into()),
        }
    }

    fn file(hundreds: u8, value: u8) -> Arc<[u8]> {
        Arc::from(vec![hundreds, value])
    }

    /// Runs the thread loop on `msgs` — all queued before it starts, so it takes every one
    /// before it decodes anything — and returns what it queued for the bus (up to 100 chunks:
    /// a looping song never ends) until it went quiet. Then hangs up.
    fn run(msgs: Vec<MusicMsg>, shared: &Arc<MusicShared>) -> Vec<MusicChunk> {
        run_with(msgs, shared, flat_open)
    }

    type Opener = fn(&Arc<[u8]>) -> Result<Box<dyn MusicSource>, String>;

    fn run_with(msgs: Vec<MusicMsg>, shared: &Arc<MusicShared>, open: Opener) -> Vec<MusicChunk> {
        let (ctrl_tx, ctrl_rx) = channel();
        for m in msgs {
            ctrl_tx.send(m).unwrap();
        }
        let (out_tx, out_rx) = sync_channel(8);
        let shared = shared.clone();
        let thread = std::thread::spawn(move || music_loop(&ctrl_rx, &out_tx, &shared, 48_000, &open));
        let mut chunks = Vec::new();
        while let Ok(chunk) = out_rx.recv_timeout(Duration::from_millis(300)) {
            chunks.push(chunk);
            if chunks.len() == 100 {
                break;
            }
        }
        drop(ctrl_tx);
        drop(out_rx);
        thread.join().unwrap();
        chunks
    }

    #[test]
    fn the_thread_fills_the_queue_without_waiting_before_every_chunk() {
        // 2026-10-23: the thread waited FULL_WAIT (40 ms) before EVERY chunk, not only when the
        // queue had refused one — 23 ms of music per 40 ms, and the title song stuttered on the
        // S25. A queue's worth of chunks must come in well under MUSIC_QUEUE_CHUNKS × 40 ms.
        let shared = MusicShared::new();
        let (ctrl_tx, ctrl_rx) = channel();
        ctrl_tx.send(MusicMsg::Start { id: 1, data: file(255, 50), loops: -1, level: 100 }).unwrap();
        let (out_tx, out_rx) = sync_channel(MUSIC_QUEUE_CHUNKS);
        let thread_shared = shared.clone();
        let started = Instant::now();
        let thread = std::thread::spawn(move || music_loop(&ctrl_rx, &out_tx, &thread_shared, 48_000, &flat_open));
        for _ in 0..MUSIC_QUEUE_CHUNKS {
            out_rx.recv_timeout(Duration::from_secs(2)).expect("a chunk");
        }
        let took = started.elapsed();
        drop(ctrl_tx);
        drop(out_rx);
        thread.join().unwrap();
        assert!(took < Duration::from_millis(300), "{MUSIC_QUEUE_CHUNKS} chunks took {took:?}");
        assert_eq!(shared.underruns(), 0);
    }

    #[test]
    fn the_bus_counts_each_time_the_decoder_lets_it_run_dry() {
        let shared = MusicShared::new();
        let (tx, rx) = sync_channel(4);
        let mut music = MusicIn::new(rx, shared.clone(), 48_000);
        shared.generation.store(1, Ordering::Relaxed);
        let mut buf = vec![0.0; 400]; // 200 frames a call
        tx.send(MusicChunk { generation: 1, frames: vec![0.5; 200] }).unwrap(); // 100 frames
        music.mix_into(&mut buf, false);
        assert_eq!(shared.underruns(), 1, "the chunk ran out with nothing behind it");
        music.mix_into(&mut buf, false);
        assert_eq!(shared.underruns(), 1, "one dry spell, counted once");
        tx.send(MusicChunk { generation: 1, frames: vec![0.5; 800] }).unwrap(); // 400 frames
        music.mix_into(&mut buf, false);
        music.mix_into(&mut buf, false);
        assert_eq!(shared.underruns(), 1, "a chunk that lasts the call is not an underrun");
        shared.ended.store(true, Ordering::Relaxed);
        music.mix_into(&mut buf, false);
        assert_eq!(shared.underruns(), 1, "the song's own end is not one either");
        shared.ended.store(false, Ordering::Relaxed);
        tx.send(MusicChunk { generation: 1, frames: vec![0.5; 200] }).unwrap();
        music.mix_into(&mut buf, false);
        assert_eq!(shared.underruns(), 2);
    }

    #[test]
    fn a_song_played_once_is_queued_once() {
        let shared = MusicShared::new();
        let chunks = run(vec![MusicMsg::Start { id: 1, data: file(5, 50), loops: 1, level: 100 }], &shared);
        let frames: usize = chunks.iter().map(|c| c.frames.len() / 2).sum();
        assert_eq!(frames, 500);
        assert!(chunks.iter().all(|c| c.generation == 1));
        // Left and right as decoded — once the side channel's low-pass has settled on them.
        let last = &chunks.last().unwrap().frames;
        let (l, r) = (last[last.len() - 2], last[last.len() - 1]);
        assert!((l - 0.5).abs() < 1e-3 && (r + 0.5).abs() < 1e-3, "{l} {r}");
    }

    #[test]
    fn a_loop_count_of_three_plays_three_passes_back_to_back() {
        let shared = MusicShared::new();
        let chunks = run(vec![MusicMsg::Start { id: 1, data: file(2, 50), loops: 3, level: 100 }], &shared);
        let frames: usize = chunks.iter().map(|c| c.frames.len() / 2).sum();
        assert_eq!(frames, 600);
    }

    #[test]
    fn an_empty_file_does_not_loop_forever() {
        let shared = MusicShared::new();
        let chunks = run(vec![MusicMsg::Start { id: 1, data: file(0, 50), loops: -1, level: 100 }], &shared);
        assert!(chunks.is_empty());
    }

    /// A rendition WAV as `hz_render` writes it: 16-bit stereo, `hzmd` = the key of `source`.
    fn rendition(source: &[u8], frames: &[[f32; 2]], rate: u32) -> Arc<[u8]> {
        let data: Vec<u8> = frames.iter().flatten().flat_map(|s| ((s * 32767.0) as i16).to_le_bytes()).collect();
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(4 + 24 + 16 + 8 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 4).to_le_bytes());
        w.extend_from_slice(&4u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"hzmd");
        w.extend_from_slice(&8u32.to_le_bytes());
        w.extend_from_slice(&rendition_key(source).to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        Arc::from(w)
    }

    #[test]
    fn a_midi_song_plays_its_rendition_and_only_its_own() {
        let midi: Arc<[u8]> = Arc::from(&b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk\0\0\0\x04\0\xff\x2f\0"[..]);
        let other: Arc<[u8]> = Arc::from(&b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x04\0\xff\x2f\0"[..]);
        let frames: Vec<[f32; 2]> = (0..3000).map(|i| [0.25, if i < 1500 { -0.5 } else { 0.5 }]).collect();
        let wavs = vec![rendition(&midi, &frames, 22_050)];
        let mut source = open_source_with(&wavs, &midi).unwrap();
        assert_eq!(source.sample_rate(), 22_050);
        let mut got = Vec::new();
        while source.read(&mut got) {}
        assert_eq!(got.len(), 3000);
        assert!((got[0][0] - 0.25).abs() < 1e-3 && (got[0][1] + 0.5).abs() < 1e-3 && (got[2999][1] - 0.5).abs() < 1e-3);
        assert!(open_source_with(&wavs, &other).is_err(), "another MIDI gets no one else's rendition");
        assert!(open_source(&midi).is_err(), "no renditions, no sound");
        assert!(WavSource::open(b"RIFF\0\0\0\0WAVE").is_err());
    }

    #[test]
    fn a_new_song_is_a_new_generation_and_a_midi_song_still_stops_the_old_one() {
        let shared = MusicShared::new();
        let (ctrl_tx, ctrl_rx) = channel();
        let (out_tx, out_rx) = sync_channel(10_000);
        ctrl_tx.send(MusicMsg::Start { id: 1, data: file(1, 50), loops: 1, level: 100 }).unwrap();
        ctrl_tx.send(MusicMsg::Start { id: 2, data: Arc::from(&b"MThd\0\0\0\x06"[..]), loops: 1, level: 100 }).unwrap();
        drop(ctrl_tx);
        music_loop(&ctrl_rx, &out_tx, &shared, 48_000, &|d| if d.starts_with(b"MThd") { open_source(d) } else { flat_open(d) });
        assert_eq!(shared.generation.load(Ordering::Relaxed), 2, "the fanfare took the decoder");
        assert!(out_rx.try_iter().all(|c| c.generation == 1), "and has nothing to play itself");
        assert!(open_source(&Arc::from(&b"RIFF....WAVE"[..])).is_err());
    }

    #[test]
    fn stop_holds_and_the_same_start_resumes_without_a_new_generation() {
        let shared = MusicShared::new();
        run(
            vec![
                MusicMsg::Start { id: 7, data: file(50, 50), loops: -1, level: 40 },
                MusicMsg::Stop { id: 7 },
            ],
            &shared,
        );
        assert!(shared.held.load(Ordering::Relaxed));
        let shared = MusicShared::new();
        run(
            vec![
                MusicMsg::Start { id: 7, data: file(50, 50), loops: 1, level: 40 },
                MusicMsg::Stop { id: 7 },
                MusicMsg::Stop { id: 8 },
                MusicMsg::Start { id: 7, data: file(50, 50), loops: 1, level: 60 },
                MusicMsg::Level { id: 9, level: 1 },
            ],
            &shared,
        );
        assert!(!shared.held.load(Ordering::Relaxed));
        assert_eq!(shared.generation.load(Ordering::Relaxed), 1, "resumed, not restarted");
        assert_eq!(shared.level.load(Ordering::Relaxed), 60, "the level of the restart, not another stream's");
    }

    #[test]
    fn close_holds_the_place_and_only_another_song_frees_the_decoder() {
        let shared = MusicShared::new();
        run(vec![MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 }, MusicMsg::Close { id: 2 }], &shared);
        assert_eq!(shared.generation.load(Ordering::Relaxed), 1, "a foreign close is nothing");
        assert!(!shared.held.load(Ordering::Relaxed));
        let shared = MusicShared::new();
        run(vec![MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 }, MusicMsg::Close { id: 1 }], &shared);
        assert_eq!(shared.generation.load(Ordering::Relaxed), 1, "closed: silent, the place kept");
        assert!(shared.held.load(Ordering::Relaxed));
        assert!(shared.closed.load(Ordering::Relaxed), "the bus gives it the map load's grace");
        let shared = MusicShared::new();
        run(vec![MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 }, MusicMsg::Stop { id: 1 }], &shared);
        assert!(!shared.closed.load(Ordering::Relaxed), "a plain stop is not a close");
        let shared = MusicShared::new();
        run(
            vec![
                MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 },
                MusicMsg::Stop { id: 1 },
                MusicMsg::Close { id: 1 },
                MusicMsg::Start { id: 2, data: file(50, 60), loops: -1, level: 100 },
            ],
            &shared,
        );
        assert_eq!(shared.generation.load(Ordering::Relaxed), 2, "another song: a new stream");
        assert!(!shared.held.load(Ordering::Relaxed));
        assert!(!shared.closed.load(Ordering::Relaxed));
    }

    #[test]
    fn the_same_song_started_by_the_next_rooms_player_goes_on_from_its_place() {
        // The game leaves a room: stop, close, a new Player on the building's song — the same
        // file. The handset started it from the top every door; here it goes on (user, 2026-10-23).
        let shared = MusicShared::new();
        let chunks = run(
            vec![
                MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 },
                MusicMsg::Stop { id: 1 },
                MusicMsg::Close { id: 1 },
                MusicMsg::Start { id: 2, data: file(50, 50), loops: -1, level: 80 },
                MusicMsg::Stop { id: 2 },
                MusicMsg::Close { id: 2 },
                MusicMsg::Start { id: 3, data: file(50, 50), loops: -1, level: 100 },
            ],
            &shared,
        );
        assert_eq!(shared.generation.load(Ordering::Relaxed), 1, "one stream the whole way");
        assert!(chunks.iter().all(|c| c.generation == 1));
        assert!(!shared.held.load(Ordering::Relaxed), "playing");
        assert!(!shared.closed.load(Ordering::Relaxed));
        assert_eq!(shared.level.load(Ordering::Relaxed), 100, "the new Player's level");
        // The new Player's own messages reach the stream it took over; the closed id's do not.
        let shared = MusicShared::new();
        run(
            vec![
                MusicMsg::Start { id: 1, data: file(50, 50), loops: -1, level: 100 },
                MusicMsg::Close { id: 1 },
                MusicMsg::Start { id: 2, data: file(50, 50), loops: -1, level: 100 },
                MusicMsg::Level { id: 1, level: 5 },
                MusicMsg::Stop { id: 1 },
                MusicMsg::Level { id: 2, level: 70 },
            ],
            &shared,
        );
        assert_eq!(shared.level.load(Ordering::Relaxed), 70);
        assert!(!shared.held.load(Ordering::Relaxed));
    }

    // ---- the loop seam ----

    fn frames(quiet: usize, loud: usize, quiet_after: usize) -> Vec<[f32; 2]> {
        let mut v = vec![[0.0, 0.0]; quiet];
        v.extend(std::iter::repeat_n([0.5, -0.5], loud));
        v.extend(std::iter::repeat_n([0.0005, 0.0], quiet_after));
        v
    }

    #[test]
    fn the_seam_drops_the_quiet_on_both_sides_of_it_and_nowhere_else() {
        let mut seam = Seam::default();
        let mut out = Vec::new();
        // First pass: its start plays as it is; its quiet end is held.
        seam.take(&frames(100, 200, 50), true, &mut out);
        assert_eq!(out, frames(100, 200, 0));
        // Quiet in the middle of the music is held, then played when the music goes on.
        seam.take(&frames(30, 10, 0), true, &mut out);
        assert_eq!(out.len(), 100 + 200 + 50 + 30 + 10);
        seam.take(&frames(0, 0, 70), true, &mut out);
        seam.next_pass();
        // Second pass: the 70 held frames are gone, and so is the new start's quiet.
        out.clear();
        seam.take(&frames(40, 0, 0), true, &mut out);
        seam.take(&frames(60, 5, 0), true, &mut out);
        assert_eq!(out, frames(0, 5, 0));
        // A last pass keeps its end.
        out.clear();
        seam.take(&frames(0, 5, 20), false, &mut out);
        assert_eq!(out.len(), 25);
    }

    #[test]
    fn a_rest_longer_than_the_cap_plays_all_but_the_cap() {
        let mut seam = Seam::default();
        let mut out = Vec::new();
        seam.take(&frames(0, 10, 5_000), true, &mut out);
        assert_eq!(out.len(), 10 + 5_000 - SEAM_TRIM_MAX);
        seam.next_pass();
        out.clear();
        seam.take(&frames(5_000, 1, 0), true, &mut out);
        assert_eq!(out.len(), 5_000 - SEAM_TRIM_MAX + 1);
    }

    /// Byte 0 / 1 / 2 = hundreds of frames: quiet start, music, quiet end.
    struct Shaped(Vec<[f32; 2]>);

    impl MusicSource for Shaped {
        fn sample_rate(&self) -> u32 {
            48_000
        }
        fn read(&mut self, out: &mut Vec<[f32; 2]>) -> bool {
            if self.0.is_empty() {
                return false;
            }
            let n = self.0.len().min(1024);
            out.extend(self.0.drain(..n));
            true
        }
    }

    fn shaped_open(data: &Arc<[u8]>) -> Result<Box<dyn MusicSource>, String> {
        Ok(Box::new(Shaped(frames(data[0] as usize * 100, data[1] as usize * 100, data[2] as usize * 100))))
    }

    #[test]
    fn a_looped_song_goes_round_without_the_encoders_silence() {
        let song: Arc<[u8]> = Arc::from(vec![3u8, 10, 2]);
        let total = |loops| {
            let chunks = run_with(vec![MusicMsg::Start { id: 1, data: song.clone(), loops, level: 100 }], &MusicShared::new(), shaped_open);
            chunks.iter().map(|c| c.frames.len() / 2).sum::<usize>()
        };
        assert_eq!(total(1), 1_500, "played once: untouched");
        assert_eq!(total(2), 300 + 1_000 + 1_000 + 200, "the seam between the passes is gone");
    }

    // ---- the bus side ----

    const RATE: u32 = 48_000;

    fn bus_end() -> (MusicIn, SyncSender<MusicChunk>, Arc<MusicShared>) {
        let (tx, rx) = sync_channel(64);
        let shared = MusicShared::new();
        shared.level.store(100, Ordering::Relaxed);
        (MusicIn::new(rx, shared.clone(), RATE), tx, shared)
    }

    fn chunk(generation: u64, frames: usize, v: f32) -> MusicChunk {
        MusicChunk { generation, frames: vec![v; frames * 2] }
    }

    fn fade_frames() -> usize {
        (RATE * MUSIC_FADE_MS / 1000) as usize
    }

    #[test]
    fn a_stream_ramps_in_then_plays_at_the_music_gain() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        tx.send(chunk(1, 2_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 2_000 * 2];
        music.mix_into(&mut buf, false);
        assert!(buf[0] < 0.01, "no jump at the start: {}", buf[0]);
        let steady = 0.5 * MUSIC_GAIN;
        assert!((buf[buf.len() - 2] - steady).abs() < 1e-5, "{}", buf[buf.len() - 2]);
        let first_full = buf.chunks_exact(2).position(|f| (f[0] - steady).abs() < 1e-5).unwrap();
        assert!(first_full.abs_diff(fade_frames()) <= 2, "ramp of {first_full} frames");
    }

    #[test]
    fn the_games_level_is_the_square_law_of_the_effects() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        shared.level.store(50, Ordering::Relaxed);
        tx.send(chunk(1, 4_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 4_000 * 2];
        music.mix_into(&mut buf, false);
        let expected = 0.5 * MUSIC_GAIN * wie_backend::volume_gain(50);
        assert!((buf[buf.len() - 2] - expected).abs() < 1e-5);
    }

    fn grace_frames() -> usize {
        (RATE * MUSIC_STOP_GRACE_MS / 1000) as usize
    }

    #[test]
    fn stopping_ramps_out_holds_the_place_and_resumes_from_it() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        // A ramp of known values: frame n carries n / 1e5.
        let frames: Vec<f32> = (0..40_000).flat_map(|n| [n as f32 / 1e5, 0.0]).collect();
        tx.send(MusicChunk { generation: 1, frames }).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed);
        // The grace first: the song plays on, unbroken, in case it is the next room's too.
        let mut buf = vec![0.0f32; grace_frames() * 2];
        music.mix_into(&mut buf, false);
        assert_eq!(music.envelope(), 1.0);
        assert!((buf[buf.len() - 2] / MUSIC_GAIN - (1_000 + grace_frames() - 1) as f32 / 1e5).abs() < 1e-5);
        let mut buf = vec![0.0f32; 2_000 * 2];
        music.mix_into(&mut buf, false);
        assert_eq!(music.envelope(), 0.0);
        let silent_from = buf.chunks_exact(2).position(|f| f[0] == 0.0).unwrap();
        assert!(silent_from.abs_diff(fade_frames()) <= 2, "fade of {silent_from} frames");
        // Held: nothing consumed however long it stays stopped.
        for _ in 0..10 {
            let mut buf = vec![0.0f32; 512];
            music.mix_into(&mut buf, false);
            assert!(buf.iter().all(|s| *s == 0.0));
        }
        shared.held.store(false, Ordering::Relaxed);
        let mut buf = vec![0.0f32; 2_000 * 2];
        music.mix_into(&mut buf, false);
        // Frame 1000 + the grace + the fade-out's frames is where it went on.
        let resumed_at = (1_000 + grace_frames() + silent_from) as f32 / 1e5;
        let last = buf[buf.len() - 2] / MUSIC_GAIN;
        assert!((last - (resumed_at + 1_999.0 / 1e5)).abs() < 1e-4, "{last} vs {resumed_at}");
    }

    #[test]
    fn a_stop_undone_within_the_grace_is_never_heard() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        let frames: Vec<f32> = (0..40_000).flat_map(|n| [n as f32 / 1e5, 0.0]).collect();
        tx.send(MusicChunk { generation: 1, frames }).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed); // the door: stop, close
        let mut buf = vec![0.0f32; 500 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(false, Ordering::Relaxed); // the next room: the same song
        let mut buf2 = vec![0.0f32; 500 * 2];
        music.mix_into(&mut buf2, false);
        let all: Vec<f32> = buf.iter().chain(buf2.iter()).step_by(2).map(|s| s / MUSIC_GAIN).collect();
        for (i, s) in all.iter().enumerate() {
            assert!((s - (1_000 + i) as f32 / 1e5).abs() < 1e-5, "frame {i}: {s}");
        }
        assert_eq!(music.envelope(), 1.0);
        // Stopped again much later: a fresh grace, not the remainder of the first.
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed);
        let mut buf = vec![0.0f32; grace_frames() * 2];
        music.mix_into(&mut buf, false);
        assert!(buf.iter().step_by(2).all(|s| *s > 0.0), "the whole grace plays");
        // The background does not wait for the grace.
        shared.held.store(false, Ordering::Relaxed);
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed);
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, true);
        assert_eq!(music.envelope(), 0.0);
        assert_eq!(buf[buf.len() - 2], 0.0);
    }

    fn close_grace_frames() -> usize {
        (RATE * MUSIC_CLOSE_GRACE_MS / 1000) as usize
    }

    #[test]
    fn a_door_plays_the_song_on_through_the_map_load() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        // Frame n carries n / 1e6, longer than the close grace.
        let len = close_grace_frames() + 20_000;
        let frames: Vec<f32> = (0..len).flat_map(|n| [n as f32 / 1e6, 0.0]).collect();
        tx.send(MusicChunk { generation: 1, frames }).unwrap();
        let mut played = Vec::new();
        let mut mix = |music: &mut MusicIn, n: usize| {
            let mut buf = vec![0.0f32; n * 2];
            music.mix_into(&mut buf, false);
            played.extend(buf.iter().step_by(2).map(|s| s / MUSIC_GAIN));
        };
        mix(&mut music, 1_000);
        shared.held.store(true, Ordering::Relaxed); // the door: stop …
        mix(&mut music, 200);
        shared.closed.store(true, Ordering::Relaxed); // … close, and the map loads
        mix(&mut music, close_grace_frames() - 200 - 1_000);
        assert_eq!(music.envelope(), 1.0, "still playing, 1,000 frames short of the close grace");
        shared.held.store(false, Ordering::Relaxed); // the next map's Player: the same song
        shared.closed.store(false, Ordering::Relaxed);
        mix(&mut music, 5_000);
        for (i, s) in played.iter().enumerate().skip(fade_frames() + 2) {
            assert!((s - i as f32 / 1e6).abs() < 5e-7, "frame {i}: {s} — the song never stopped");
        }

        // Nothing comes after the close: it ramps out at the close grace.
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        tx.send(chunk(1, close_grace_frames() + 20_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed);
        shared.closed.store(true, Ordering::Relaxed); // stop and close seen in one call
        let mut buf = vec![0.0f32; close_grace_frames() * 2];
        music.mix_into(&mut buf, false);
        assert!(buf.iter().step_by(2).all(|s| *s > 0.0), "the whole close grace plays");
        let mut buf = vec![0.0f32; 2_000 * 2];
        music.mix_into(&mut buf, false);
        assert_eq!(music.envelope(), 0.0);
    }

    #[test]
    fn a_close_after_the_stops_grace_ran_out_does_not_bring_the_song_back() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        tx.send(chunk(1, 40_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        shared.held.store(true, Ordering::Relaxed); // the settings turn the music off
        let mut buf = vec![0.0f32; (grace_frames() + 2_000) * 2];
        music.mix_into(&mut buf, false);
        assert_eq!(music.envelope(), 0.0);
        shared.closed.store(true, Ordering::Relaxed); // a door, long after
        let mut buf = vec![0.0f32; 2_000 * 2];
        music.mix_into(&mut buf, false);
        assert!(buf.iter().all(|s| *s == 0.0), "it stays quiet");
    }

    #[test]
    fn a_replaced_stream_fades_out_and_its_queue_is_dropped() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        tx.send(chunk(1, 1_000, 0.5)).unwrap();
        tx.send(chunk(1, 1_000, 0.5)).unwrap();
        tx.send(chunk(1, 1_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 800 * 2];
        music.mix_into(&mut buf, false);
        shared.generation.store(2, Ordering::Relaxed);
        tx.send(chunk(2, 5_000, -0.25)).unwrap();
        let mut buf = vec![0.0f32; 3_000 * 2];
        music.mix_into(&mut buf, false);
        // The old song goes down without a jump, then the new one comes up from zero.
        for w in buf.chunks_exact(2).collect::<Vec<_>>().windows(2) {
            assert!((w[0][0] - w[1][0]).abs() <= 0.5 * MUSIC_GAIN / fade_frames() as f32 + 1e-5);
        }
        assert!(buf.chunks_exact(2).take(fade_frames()).all(|f| f[0] >= 0.0), "old song first");
        assert!((buf[buf.len() - 2] + 0.25 * MUSIC_GAIN).abs() < 1e-5, "then only the new song");
    }

    #[test]
    fn the_background_holds_the_music_like_a_stop() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        tx.send(chunk(1, 100_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, true);
        assert_eq!(music.envelope(), 0.0);
        assert!(buf[buf.len() - 2] == 0.0);
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        assert!((buf[buf.len() - 2] - 0.5 * MUSIC_GAIN).abs() < 1e-5);
    }

    #[test]
    fn nothing_queued_is_silence_and_a_late_chunk_ramps_in() {
        let (mut music, tx, shared) = bus_end();
        shared.generation.store(1, Ordering::Relaxed);
        let mut buf = vec![0.0f32; 256];
        music.mix_into(&mut buf, false);
        assert!(buf.iter().all(|s| *s == 0.0));
        tx.send(chunk(0, 100, 0.9)).unwrap(); // stale: never heard
        tx.send(chunk(1, 1_000, 0.5)).unwrap();
        let mut buf = vec![0.0f32; 1_000 * 2];
        music.mix_into(&mut buf, false);
        assert!(buf[0] < 0.01 && buf.iter().all(|s| *s <= 0.5 * MUSIC_GAIN + 1e-6));
    }

    /// Before/after renders of the game's own songs for listening (not in the repo): 20 s of each
    /// from 10 s in, as the bus would get them at 48 kHz at `MUSIC_GAIN` — `bgm_<n>_before.wav`
    /// (decoded and resampled: Phase 4c, no [`MusicAir`] at all) and `bgm_<n>_after.wav` into
    /// `HZ_AB_OUT`. `HZ_MUSIC_DIR` as for `music_survey`.
    #[test]
    #[ignore]
    fn music_ab() {
        let (Ok(dir), Ok(out)) = (std::env::var("HZ_MUSIC_DIR"), std::env::var("HZ_AB_OUT")) else {
            eprintln!("HZ_MUSIC_DIR / HZ_AB_OUT not set");
            return;
        };
        for n in 0..17 {
            let Ok(bytes) = std::fs::read(format!("{dir}/{n}")) else { continue };
            let Ok(mut source) = open_source(&Arc::from(bytes)) else { continue };
            let rate = source.sample_rate();
            let mut frames: Vec<[f32; 2]> = Vec::new();
            while frames.len() < rate as usize * 30 && source.read(&mut frames) {}
            let from = (rate as usize * 10).min(frames.len());
            let excerpt = &frames[from..(from + rate as usize * 20).min(frames.len())];
            for (name, air) in [("before", None), ("after", Some(MUSIC_AIR))] {
                let mut x = excerpt.to_vec();
                if let Some(air) = air {
                    MusicAir::new(rate, air).process(&mut x);
                }
                let mut y = Vec::new();
                let mut rs = Resampler::new(rate, 48_000);
                rs.process(&x, &mut y);
                rs.flush(&mut y);
                y.iter_mut().for_each(|s| *s *= MUSIC_GAIN);
                crate::sfx_upsample::tests::write_wav(&format!("{out}/bgm_{n}_{name}.wav"), &y, 48_000);
            }
        }
    }

    /// The game's own songs (not in the repo): `HZ_MUSIC_DIR` = a directory holding the jar's
    /// `snd/b/*`. Prints each song's rate, length, peak, RMS and silent edges, and the decode
    /// speed — `cargo test -p hz_android --release music_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn music_survey() {
        let Ok(dir) = std::env::var("HZ_MUSIC_DIR") else {
            eprintln!("HZ_MUSIC_DIR not set");
            return;
        };
        for n in 0..17 {
            let Ok(bytes) = std::fs::read(format!("{dir}/{n}")) else { continue };
            let data: Arc<[u8]> = Arc::from(bytes);
            let started = std::time::Instant::now();
            let mut source = match open_source(&data) {
                Ok(source) => source,
                Err(e) => {
                    println!("snd/b/{n}: {e}");
                    continue;
                }
            };
            let mut frames: Vec<[f32; 2]> = Vec::new();
            while source.read(&mut frames) {}
            let secs = started.elapsed().as_secs_f64();
            let rate = source.sample_rate();
            let loud = |f: &[f32; 2]| f[0].abs().max(f[1].abs()) > 1e-3;
            let lead = frames.iter().position(loud).unwrap_or(frames.len());
            let tail = frames.iter().rev().position(loud).unwrap_or(frames.len());
            let peak = frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
            let side_db = |fr: &[[f32; 2]]| {
                let (m, s) = fr.iter().fold((0.0f64, 0.0f64), |(m, s), f| {
                    let (mid, side) = (f64::from(f[0] + f[1]) / 2.0, f64::from(f[0] - f[1]) / 2.0);
                    (m + mid * mid, s + side * side)
                });
                10.0 * (s / m.max(1e-30)).log10()
            };
            println!("snd/b/{n}: side/mid over the whole song {:.1} dB", side_db(&frames));
            let rms = (frames.iter().map(|f| f64::from(f[0] * f[0] + f[1] * f[1]) / 2.0).sum::<f64>() / frames.len().max(1) as f64).sqrt();
            let length = frames.len() as f64 / f64::from(rate);
            println!(
                "snd/b/{n}: {rate} Hz {length:.2} s, peak {peak:.3} ({:.1} dBFS), RMS {:.1} dBFS, silent lead {lead} / tail {tail} frames, decode {:.0}x real time",
                20.0 * peak.log10(),
                20.0 * rms.log10(),
                length / secs
            );
            // `HZ_MUSIC_OUT`: 20 s of each song from 10 s in, as decoded — interleaved f32 LE — for
            // spectrum work outside the tests.
            if let Ok(out) = std::env::var("HZ_MUSIC_OUT") {
                let from = (rate as usize * 10).min(frames.len());
                let to = (from + rate as usize * 20).min(frames.len());
                let bytes: Vec<u8> = frames[from..to].iter().flatten().flat_map(|s| s.to_le_bytes()).collect();
                std::fs::write(format!("{out}/{n}.f32"), bytes).unwrap();
            }
        }
    }

    /// What the thread queues for the bus at 48 kHz for `data` played `loops` times, the real
    /// decoder behind it — every chunk, interleaved.
    fn rendered(data: &Arc<[u8]>, loops: i32) -> Vec<f32> {
        let (ctrl_tx, ctrl_rx) = channel();
        ctrl_tx.send(MusicMsg::Start { id: 1, data: data.clone(), loops, level: 100 }).unwrap();
        let (out_tx, out_rx) = sync_channel(MUSIC_QUEUE_CHUNKS);
        let shared = MusicShared::new();
        let thread = std::thread::spawn(move || music_loop(&ctrl_rx, &out_tx, &shared, 48_000, &open_source));
        let mut out = Vec::new();
        while let Ok(chunk) = out_rx.recv_timeout(Duration::from_secs(2)) {
            out.extend(chunk.frames);
        }
        drop(ctrl_tx);
        drop(out_rx);
        thread.join().unwrap();
        out
    }

    /// The loop seam of each of the game's songs as the thread plays it (`HZ_MUSIC_DIR` as for
    /// `music_survey`): two passes against one — they part where the seam drops the first pass's
    /// quiet end — then, around that point, the longest run below −60 dBFS (the hole a listener
    /// would hear) and the largest second difference against the song's own 99.9th percentile
    /// (a click stands out of it). With `HZ_AB_OUT`, `seam_<n>.wav`: 3 s either side at
    /// `MUSIC_GAIN`, to listen to every seam without waiting minutes for each to come round —
    /// `cargo test -p hz_android --release music_seams -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn music_seams() {
        let Ok(dir) = std::env::var("HZ_MUSIC_DIR") else {
            eprintln!("HZ_MUSIC_DIR not set");
            return;
        };
        const RATE: usize = 48_000;
        for n in 0..17 {
            let Ok(bytes) = std::fs::read(format!("{dir}/{n}")) else { continue };
            let data: Arc<[u8]> = Arc::from(bytes);
            if open_source(&data).is_err() {
                println!("snd/b/{n}: not a song the thread decodes");
                continue;
            }
            let (once, twice) = (rendered(&data, 1), rendered(&data, 2));
            let frame = |v: &[f32], i: usize| [v[2 * i], v[2 * i + 1]];
            let (fa, fb) = (once.len() / 2, twice.len() / 2);
            let Some(seam) = (0..fa.min(fb)).find(|&i| {
                let (a, b) = (frame(&once, i), frame(&twice, i));
                (a[0] - b[0]).abs() > 1e-6 || (a[1] - b[1]).abs() > 1e-6
            }) else {
                println!("snd/b/{n}: two passes never part from one ({fa} / {fb} frames)");
                continue;
            };
            let quiet = |i: usize| frame(&twice, i).iter().all(|s| s.abs() < SEAM_QUIET);
            let (from, to) = (seam.saturating_sub(RATE / 10), (seam + RATE / 10).min(fb));
            let (mut hole, mut run) = (0usize, 0usize);
            for i in from..to {
                run = if quiet(i) { run + 1 } else { 0 };
                hole = hole.max(run);
            }
            let d2 = |v: &[f32], i: usize| (0..2).map(|c| (v[2 * (i + 1) + c] - 2.0 * v[2 * i + c] + v[2 * (i - 1) + c]).abs()).fold(0.0f32, f32::max);
            let mut song: Vec<f32> = (1..fa - 1).map(|i| d2(&once, i)).collect();
            song.sort_by(f32::total_cmp);
            let p999 = song[song.len() * 999 / 1000].max(1e-9);
            let peak = |v: &[f32], at: std::ops::Range<usize>| at.map(|i| d2(v, i)).fold(0.0f32, f32::max);
            let near = peak(&twice, seam.saturating_sub(RATE / 200).max(1)..(seam + RATE / 200).min(fb - 1));
            // The same measure over the song's own first entry, out of its silence: a seam no
            // sharper than that is the music's attack, not a splice.
            let first = (0..fa).find(|&i| frame(&once, i).iter().any(|s| s.abs() >= SEAM_QUIET)).unwrap_or(0);
            let entry = peak(&once, first.max(1)..(first + RATE / 100).min(fa - 1));
            println!(
                "snd/b/{n}: pass {:.2} s, seam at {:.2} s: hole {:.1} ms, second difference {near:.4} = {:.2}x the song's 99.9th percentile (its first entry: {entry:.4})",
                (fb - fa) as f64 / RATE as f64,
                seam as f64 / RATE as f64,
                hole as f64 * 1000.0 / RATE as f64,
                near / p999
            );
            if let Ok(out) = std::env::var("HZ_AB_OUT") {
                let (a, b) = (seam.saturating_sub(3 * RATE), (seam + 3 * RATE).min(fb));
                let clip: Vec<f32> = twice[2 * a..2 * b].iter().map(|s| s * MUSIC_GAIN).collect();
                crate::sfx_upsample::tests::write_wav(&format!("{out}/seam_{n}.wav"), &clip, RATE as u32);
            }
        }
    }
}
