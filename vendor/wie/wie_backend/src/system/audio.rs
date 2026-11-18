use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use smaf_player::{SmafEvent, parse_smaf};

use super::{
    atr::{AtrWave, parse_atr_waves},
    ma2::{MA2_CHANNELS, Ma2Event, Ma2Song, parse_ma2},
};
use crate::{System, audio_sink::AudioSink};

pub type AudioHandle = u32;
#[derive(Debug)]
pub enum AudioError {
    InvalidHandle,
    InvalidAudio,
}

/// A parsed SMAF file: the event list, plus which BUS the file belongs to.
///
/// The bus is a property of the FILE, not of the handle or the clip, so it is settled
/// once here at parse time and shared with every playback. SMAF keeps the two kinds in
/// physically different chunks — a `MTR` score track is MIDI, a `ATR` audio track is
/// PCM — and in Hero3 that maps exactly onto the game's two volume sliders:
/// `res/snd/bgm*_mf` are `MTR` (배경음) and `res/snd/sd*_mf` are `ATR` (효과음).
///
/// Knowing it matters because the music level is applied to a HOST bus
/// (`AudioSink::set_music_volume`): a sound effect must not be allowed to push its own
/// level onto that bus, or playing an effect at a different setting would duck or
/// silence the music under it — the very symptom this is here to fix.
pub struct Score {
    pub events: Vec<(usize, SmafEvent)>,
    pub is_music: bool,
    /// One pass of this file, in ms — including the tail of a PCM one-shot.
    pub duration_ms: u64,
    /// The same file read as the MA-2 score it is, with its own FM voices — every hero2 song
    /// (`super::ma2`). `None` for anything else; `events` is the General MIDI reading either
    /// way, for a sink without an FM synth.
    pub ma2: Option<Ma2Song>,
}

impl Score {
    fn parse(data: &[u8]) -> Self {
        let ma2 = parse_ma2(data);
        let mut events = parse_smaf(data);
        // ★ 2026-10-02: an effect's samples as the MA-2 decodes them. `smaf_player` reads the
        // ADPCM nibbles in the other Yamaha chip family's order, which turns every hero2
        // effect into a thump at each end — see [`super::atr`]. Taken only when that parser
        // reads the file AND agrees with `smaf_player` on what plays (as many waves, each as
        // long, at the same rate); anything else keeps the old samples.
        if let Some(waves) = parse_atr_waves(data) {
            let agrees = |(event, wave): (&SmafEvent, &AtrWave)| {
                matches!(event, SmafEvent::Wave { channel: 1, sampling_rate, data }
                    if *sampling_rate == wave.sampling_rate && data.len() == wave.data.len())
            };
            let played = || events.iter().filter_map(|(_, e)| matches!(e, SmafEvent::Wave { .. }).then_some(e));
            if played().count() == waves.len() && played().zip(&waves).all(agrees) {
                let mut waves = waves.into_iter();
                for (_, event) in events.iter_mut() {
                    if let SmafEvent::Wave { data, .. } = event {
                        *data = waves.next().map(|w| w.data).unwrap_or_default();
                    }
                }
            }
        }
        // Anything that is not a wave one-shot means a synth is involved.
        let is_music = events.iter().any(|(_, e)| !matches!(e, SmafEvent::Wave { .. } | SmafEvent::End));

        // How long one pass of this file actually SOUNDS. For a wave the event time is
        // when it starts, so the clip's own length has to be added — an effect the guest
        // fires and immediately `MC_mdaStop`s is still audible for its full duration
        // here, and [`Audio`] needs that number to know how long to hold the music open.
        let mut duration_ms = 0usize;
        for (time, event) in events.iter() {
            let end = match event {
                SmafEvent::Wave {
                    sampling_rate,
                    data,
                    channel,
                } => {
                    let frames = data.len() / (*channel).max(1) as usize;
                    time + frames * 1000 / (*sampling_rate).max(1) as usize
                }
                _ => *time,
            };
            duration_ms = duration_ms.max(end);
        }

        Self {
            events,
            is_music,
            duration_ms: duration_ms as u64,
            ma2,
        }
    }
}

/// A parsed SMAF file, shared by every handle and every playback of it. `Arc` rather
/// than a copy per handle: `MC_mdaClipPutData` is called again for every sound the game
/// plays, and the 33 files in Hero3 were being re-copied and re-parsed thousands of
/// times a session.
type Events = Arc<Score>;

enum AudioFile {
    Smaf(Events),
}

/// Why a playback is allowed to keep running, checked by [`SmafPlayer::play`] between
/// events.
///
/// `stop` is the old unconditional kill. `deadline_ms` is the new one: a wall-clock
/// epoch-ms at which a *deferred* stop becomes real, or 0 for "no stop pending". See
/// [`MusicChannel`] for why the music needs a stop that can still be taken back.
pub struct PlaybackGate {
    stop: AtomicBool,
    deadline_ms: AtomicU64,
    finished: AtomicBool,
    /// The MIDI channels this playback has put state on (bit n = channel n), so a stop
    /// can silence them from OUTSIDE the task. See [`PlaybackGate::silence`].
    channels: AtomicU64,
    /// Set by [`PlaybackGate::silence`]: the channels were already cleared, so the task
    /// must NOT send its own end-of-playback cleanup when it next wakes.
    silenced: AtomicBool,
    /// The sink's FM part this playback plays on, or [`NO_FM_PART`] for a General MIDI (or
    /// PCM) playback — what [`PlaybackGate::silence`] has to cut.
    fm_part: AtomicU8,
}

/// [`PlaybackGate::fm_part`] of a playback that is not on the FM synth.
const NO_FM_PART: u8 = u8::MAX;

impl PlaybackGate {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            deadline_ms: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            channels: AtomicU64::new(0),
            silenced: AtomicBool::new(false),
            fm_part: AtomicU8::new(NO_FM_PART),
        }
    }

    fn kill(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn touch_channel(&self, channel: u8) {
        if channel < 64 {
            self.channels.fetch_or(1 << channel, Ordering::Relaxed);
        }
    }

    /// Stop this playback AND silence it now, from the caller's thread.
    ///
    /// ★ 2026-09-25, and it is the fix for "맵이 바뀔때 배경음악이 겹쳐지는 버그 — 이전 맵의
    /// 배경음악이 들려서 두개의 배경음이 겹쳐져서 들려". [`kill`](Self::kill) only raises a
    /// flag; the playback's task sees it the next time it WAKES, which is its next score
    /// event. Until then every note it is holding keeps sounding over the new song. The
    /// hero2 scores hold chords across gaps of up to 2.7 s (`P/snd/02`; `09` has nine gaps
    /// over a second), and a map change also stalls the emulator for 0.4–1.1 s while the
    /// guest loads (S9 logcat 2026-09-25: `emu hitch: tick 1118.6 ms` between the two
    /// songs) — no task runs at all then, so the old chord drones through the load and on
    /// into the new map's music. When the old task finally woke, its cleanup (all-sound-off
    /// on every channel it had used) landed on channels the NEW song was already playing
    /// on and cut its opening notes.
    ///
    /// So the channel clear is sent here, synchronously, before the caller starts anything
    /// new — the sink's event queue then carries it ahead of the new song's first event —
    /// and the task is told not to repeat it.
    fn silence(&self, sink: &dyn AudioSink) {
        self.silenced.store(true, Ordering::Relaxed);
        self.kill();
        let channels = self.channels.swap(0, Ordering::Relaxed);
        // A playback that already ran to its end cleaned up after itself, and its channels
        // may belong to another song by now — clearing them again would cut that song.
        if self.finished.load(Ordering::Relaxed) {
            return;
        }
        let part = self.fm_part.load(Ordering::Relaxed);
        if part != NO_FM_PART {
            sink.fm_all_off(part);
        }
        for channel in 0..64u8 {
            if channels & (1 << channel) != 0 {
                sink.midi_control_change(channel, 64, 0);
                // The HARD stop, not control 120 — see `AudioSink::midi_all_sound_off`.
                sink.midi_all_sound_off(channel);
                sink.midi_control_change(channel, 123, 0);
            }
        }
    }

    /// Stop at `at_ms` unless something takes it back first.
    fn stop_at(&self, at_ms: u64) {
        self.deadline_ms.store(at_ms.max(1), Ordering::Relaxed);
    }

    /// Take back a pending stop — the reason for it turned out to be temporary.
    fn keep_going(&self) {
        self.deadline_ms.store(0, Ordering::Relaxed);
    }

    /// Push a pending stop further out. Never pulls it in.
    fn hold_until(&self, at_ms: u64) {
        let cur = self.deadline_ms.load(Ordering::Relaxed);
        if cur != 0 && at_ms > cur {
            self.deadline_ms.store(at_ms, Ordering::Relaxed);
        }
    }

    fn should_stop(&self, now_ms: u64) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }
        let deadline = self.deadline_ms.load(Ordering::Relaxed);
        deadline != 0 && now_ms >= deadline
    }

    fn is_running(&self) -> bool {
        !self.finished.load(Ordering::Relaxed) && !self.stop.load(Ordering::Relaxed)
    }
}

/// The music the host is keeping alive across the guest's sound juggling.
struct MusicChannel {
    score: Events,
    gate: Arc<PlaybackGate>,
    volume: Arc<AtomicU8>,
    handle: AudioHandle,
}

/// How long a guest stop of the music waits before it is honoured.
///
/// ★ 2026-08-16, and it is the fix for "효과음이 들리면 배경음이 안 들리고 처음부터 다시
/// 시작돼".
///
/// Hero3 is a **single-channel** sound engine — device trace, one attack:
///
/// ```text
///   STOP    clip=0x4b85df00 handle=1        <- the game stops its own BGM
///   free    clip=0x4b85df00
///   create  clip=0x4b85df00 (0xa24)         <- SAME address, reused for the effect
///   putData len=2596 -> handle=2 / PLAY repeat=0
///   ... 626 ms later ...
///   create  clip=0x4b85df00 (0xdce = bgm5_mf)
///   putData -> handle=4 / PLAY repeat=1     <- BGM restarted FROM BAR ONE
/// ```
///
/// It owns exactly one `MC_MdaClip` and takes turns with it, because the phone it was
/// written for had one player (`MC_mdaClipAllocPlayer`, which hands out more, is a stub
/// here — and was on the real handset's budget too). Faithfully reproducing that on a
/// host with a real mixer is the wrong trade: the player hears the music cut out and
/// restart on every single skill.
///
/// So a stop of the MUSIC is deferred by this much instead of taken literally. What
/// happens inside the window decides what it meant:
///
///   * an EFFECT starts        -> the guest is taking its turn; hold the music open
///     until the guest re-issues it ([`effect_hold_ms`]: past the effect's end, and past
///     hero2's fixed 3 s timer) so the re-issue can land, and let the two play together;
///   * the SAME music restarts -> take the stop back and IGNORE the restart, so the
///     song simply never breaks;
///   * different music starts  -> commit the stop, start the new song;
///   * nothing at all          -> commit the stop.
///
/// The last case is real and must keep working: opening the in-game menu stops the
/// music and leaves it stopped (device trace: 4.5 s of silence until the menu closed).
/// That is why this is a short window and not "ignore guest stops" — 250 ms is longer
/// than the 37 ms the guest takes to get from the stop to the effect's `MC_mdaPlay`, and
/// short enough that a genuine silence starts on time.
const MUSIC_STOP_GRACE_MS: u64 = 250;

/// Extra time past the end of an effect for the guest to re-issue the music.
///
/// The game waits for its one clip to finish before putting the song back, so the
/// re-issue lands right about when the effect ends; this is slack for a slow frame or a
/// second effect queued behind the first.
const MUSIC_REJOIN_MS: u64 = 700;

/// When hero2 puts the music back after an effect: a fixed 3 s after the effect STARTS.
///
/// ★ 2026-10-01, and it is the fix for "효과음이 나면 배경음이 멈췄다가 처음부터 다시 나온다"
/// (`HERO2_BGM_FM_PLAN.md` §2, `HERO2_GUEST_RE.md` §8.37). hero2's sound manager `br` does
/// not wait for its effect to end the way Hero3 does: `br.a(B)V` @0x1388c8 stops the song
/// with `Player.stop`, plays the effect and arms a timer at now + 3000 ms (`0xbb8`); the
/// per-frame `br.a()V` @0x1385a4 re-issues the song once that timer has passed. With only
/// [`MUSIC_REJOIN_MS`] past a 246 ms effect the hold ran out at +0.95 s, the song stopped,
/// two seconds of silence followed and the re-issue at +3.0 s was a NEW playback — from bar
/// one (probe, slot 1: effect 21.04 s → stop 22.02 s → restart 24.06 s).
const GUEST_REISSUE_MS: u64 = 3_000;

/// Slack on top of [`GUEST_REISSUE_MS`]: the game checks its timer once per 50 ms frame, and
/// a frame can run late.
const REISSUE_SLACK_MS: u64 = 500;

/// How long past its start an effect holds a pending music stop open — until the effect is
/// over AND until the game has had its chance to re-issue the song.
///
/// The cost is only in a case not seen in hero2: a guest that stops the music for good
/// right after an effect is heard stopping up to this much late. This vendored core runs
/// hero2 alone.
fn effect_hold_ms(effect_ms: u64) -> u64 {
    (effect_ms + MUSIC_REJOIN_MS).max(GUEST_REISSUE_MS + REISSUE_SLACK_MS)
}

/// An MA-2 score shorter than this is a sound EFFECT, not a song.
///
/// ★ 2026-10-01 (`HERO2_BGM_FM_PLAN.md` §3.1). hero2 has one MIDI effect — the 환경설정
/// sound `P/snd/42`, 1,392 ms — and [`Score::is_music`] calls it music because it is a
/// score, so it replaced the song and the song came back from bar one. On the FM path it
/// plays on its own part ([`FM_PART_EFFECT`]) and sounds OVER the music like any effect.
/// Only there: under General MIDI both would share the synth's sixteen channels, so it still
/// replaces the song as before. The shortest real song, the game-over jingle `16` (8,748 ms),
/// must keep cutting the music — hence a bound well under it.
const MIDI_EFFECT_MAX_MS: u64 = 3_000;

/// The sink's FM part for the music.
const FM_PART_MUSIC: u8 = 0;
/// The sink's FM part for a MIDI effect played over the music (see [`MIDI_EFFECT_MAX_MS`]).
const FM_PART_EFFECT: u8 = 1;

/// Parsed-SMAF cache, keyed by the file's own bytes.
///
/// ★ 2026-08-09, and it is the fix for "효과음/크리티컬 이펙트 때문에 프레임이 떨어진다".
/// `Audio::play` used to run `parse_smaf` — a full chunk walk, an event `Vec`, and a
/// `sort_by` over every event in the file — **on the emulator thread, inside
/// `emulator.tick()`, on every single `MC_mdaPlay`**. The critical-hit sound
/// (`/snd/sd004_mf`, 3,833 bytes) is played on every crit, so every crit paid for a
/// full parse inside a frame that already budgets 24 ms for interpretation.
///
/// Keyed by `(len, FNV-1a-64)` and then verified by comparing the actual bytes, so a
/// hash collision cannot hand back the wrong song — the alternative (trusting a 64-bit
/// hash) would fail as a silently-wrong sound, which is exactly the kind of bug that
/// gets blamed on the emulator for a year.
///
/// Bounded: a guest that somehow feeds unbounded distinct clips stops being cached at
/// [`SMAF_CACHE_MAX`] rather than growing forever. The game has 19 BGM + 14 SFX files.
struct SmafCache {
    buckets: BTreeMap<(usize, u64), Vec<(Vec<u8>, Events)>>,
    entries: usize,
}

/// Distinct sound files kept parsed. Hero3 has 33; the headroom is for a guest that
/// synthesises clips rather than loading them from disk.
const SMAF_CACHE_MAX: usize = 128;

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl SmafCache {
    fn new() -> Self {
        Self {
            buckets: BTreeMap::new(),
            entries: 0,
        }
    }

    fn get_or_parse(&mut self, data: &[u8]) -> Events {
        let key = (data.len(), fnv1a64(data));
        if let Some(bucket) = self.buckets.get(&key)
            && let Some((_, events)) = bucket.iter().find(|(raw, _)| raw.as_slice() == data)
        {
            return events.clone();
        }

        wie_util::stats::add_smaf_parse(data.len() as u64);
        let events: Events = Arc::new(Score::parse(data));

        if self.entries < SMAF_CACHE_MAX {
            self.buckets.entry(key).or_default().push((data.to_vec(), events.clone()));
            self.entries += 1;
            wie_util::stats::set_smaf_cache_entries(self.entries as u64);
        }

        events
    }
}

pub struct Audio {
    sink: Arc<Box<dyn AudioSink>>,
    files: BTreeMap<AudioHandle, AudioFile>,
    /// Parsed events, shared across handles. See [`SmafCache`].
    cache: SmafCache,
    /// The handle a given guest clip pointer currently owns, so `MC_mdaClipFree` can
    /// drop the bookkeeping the guest is telling us it is finished with. Kept HOST-side
    /// rather than read back out of the guest's `MdaClip` struct: that struct is guest
    /// memory, `MC_mdaClipCreate` hands it back uninitialised, and closing whatever
    /// integer happened to be sitting in it could silence an unrelated clip.
    clip_handles: BTreeMap<u32, AudioHandle>,
    playing: BTreeMap<AudioHandle, Arc<PlaybackGate>>,
    /// The music playback the host keeps alive across the guest's single-clip juggling,
    /// independent of whichever handle the guest currently thinks owns it. See
    /// [`MUSIC_STOP_GRACE_MS`].
    music: Option<MusicChannel>,
    /// Live playback volume per *AudioHandle* 0..=100. Shared with the running
    /// `SmafPlayer` task (lock-free) so a volume change reaches a playing clip.
    volumes: BTreeMap<AudioHandle, Arc<AtomicU8>>,
    /// Volume per *guest clip pointer* (`MC_mdaClipSetVolume`'s clip arg). The
    /// game sets a clip's volume, then `clip_put_data` reloads it under a NEW
    /// handle before playing — so the handle at set-time != the handle at
    /// play-time. Keying by the stable clip pointer lets `play` recover the
    /// intended volume and apply it to whatever handle actually plays.
    clip_volumes: BTreeMap<u32, u8>,
    last_audio_handle: AudioHandle,
    /// ★ heroZero fork (2026-10-22): the media stream the sink is playing or holding paused
    /// ([`Audio::media_start`]) — the one decoder. A player that is STARTED but no longer owns
    /// it was stopped by the next one.
    media_owner: Option<u32>,
}

impl Audio {
    pub fn new(sink: Box<dyn AudioSink>) -> Self {
        Self {
            sink: Arc::new(sink),
            files: BTreeMap::new(),
            cache: SmafCache::new(),
            clip_handles: BTreeMap::new(),
            playing: BTreeMap::new(),
            volumes: BTreeMap::new(),
            music: None,
            clip_volumes: BTreeMap::new(),
            last_audio_handle: 0,
            media_owner: None,
        }
    }

    /// Record that `clip_ptr` now owns `handle` (called by `MC_mdaClipPutData`).
    ///
    /// The PREVIOUS handle for this pointer is deliberately left alone: today a clip
    /// that is re-loaded while its old playback is still running lets that playback
    /// finish, and rapid SFX depend on that (13차: "크리티컬 효과음이 겹쳐 들린다"
    /// is the game's own behaviour, not a defect). Only [`release_clip`] drops it.
    pub fn bind_clip(&mut self, clip_ptr: u32, handle: AudioHandle) {
        self.clip_handles.insert(clip_ptr, handle);
    }

    /// `MC_mdaClipFree`: the guest says it is finished with this clip. Drop the
    /// bookkeeping it minted.
    ///
    /// ★ Does NOT stop a playback in flight. The running task owns its own `Arc` of the
    /// events and its own stop flag, so a sound that was already started still finishes
    /// exactly as it does today — freeing the clip is not "cut the sound off".
    pub fn release_clip(&mut self, clip_ptr: u32) {
        self.clip_volumes.remove(&clip_ptr);
        if let Some(handle) = self.clip_handles.remove(&clip_ptr) {
            self.files.remove(&handle);
            self.volumes.remove(&handle);
            wie_util::stats::set_audio_handles(self.files.len() as u64);
        }
    }

    /// Get-or-create the shared volume cell for a handle (default 100 = full).
    fn volume_arc(&mut self, handle: AudioHandle) -> Arc<AtomicU8> {
        self.volumes.entry(handle).or_insert_with(|| Arc::new(AtomicU8::new(100))).clone()
    }

    /// Set the live volume for a handle (0..=100). Applies to a playing clip.
    pub fn set_clip_volume(&mut self, handle: AudioHandle, volume: u8) {
        self.volume_arc(handle).store(volume.min(100), Ordering::Relaxed);
    }

    /// Read the live volume for a handle.
    pub fn clip_volume(&mut self, handle: AudioHandle) -> u8 {
        self.volume_arc(handle).load(Ordering::Relaxed)
    }

    /// `MC_mdaClipSetVolume` stores the level against the stable clip pointer.
    pub fn set_clip_ptr_volume(&mut self, clip_ptr: u32, volume: u8) {
        self.clip_volumes.insert(clip_ptr, volume.min(100));
    }

    /// Volume the game last set for this clip pointer (default 100 = full).
    pub fn clip_ptr_volume(&self, clip_ptr: u32) -> u8 {
        self.clip_volumes.get(&clip_ptr).copied().unwrap_or(100)
    }

    pub fn load_smaf(&mut self, data: &[u8]) -> Result<AudioHandle, AudioError> {
        wie_util::stats::add_smaf_load();

        // ★ Parsed HERE and shared, not stored raw and re-parsed at every `play`.
        let events = self.cache.get_or_parse(data);

        let audio_handle = self.last_audio_handle;

        self.last_audio_handle += 1;
        self.files.insert(audio_handle, AudioFile::Smaf(events));
        wie_util::stats::set_audio_handles(self.files.len() as u64);

        Ok(audio_handle)
    }

    /// ★ heroZero fork (2026-10-22): one PCM clip straight to the sink, fire-and-forget — the
    /// `net.wie.MediaPlayer` WAV path (영웅서기 Zero's `/snd/e/*`, sub-second 8 kHz one-shots).
    /// No handle, no gate, no stop and no music-bus bookkeeping: it is exactly what a SMAF
    /// `SmafEvent::Wave` hands the sink, without the score around it. `channels` is the
    /// interleave width, as [`AudioSink::play_wave`] takes it; the level is already in the samples.
    pub fn play_pcm(&self, channels: u8, sampling_rate: u32, samples: &[i16]) {
        self.sink.play_wave(channels, sampling_rate, samples);
    }

    /// ★ heroZero fork (2026-10-22): media stream `id` starts (or resumes) on the sink and
    /// becomes the owner of its one decoder — see [`AudioSink::media_start`]. The `net.wie.MediaPlayer`
    /// path for MP4/AAC and MIDI files (영웅서기 Zero's `/snd/b/*` music).
    pub fn media_start(&mut self, id: u32, data: &Arc<[u8]>, loops: i32, level: u8) {
        self.media_owner = Some(id);
        self.sink.media_start(id, data, loops, level);
    }

    /// ★ heroZero fork (2026-10-22): pause stream `id`, if it still owns the decoder.
    pub fn media_stop(&mut self, id: u32) {
        if self.media_owner == Some(id) {
            self.sink.media_stop(id);
        }
    }

    /// ★ heroZero fork (2026-10-22): the level of stream `id`, if it still owns the decoder.
    pub fn media_level(&mut self, id: u32, level: u8) {
        if self.media_owner == Some(id) {
            self.sink.media_level(id, level.min(100));
        }
    }

    /// ★ heroZero fork (2026-10-22): stream `id` is closed; the decoder is free if it had it.
    pub fn media_close(&mut self, id: u32) {
        if self.media_owner == Some(id) {
            self.media_owner = None;
            self.sink.media_close(id);
        }
    }

    /// ★ heroZero fork (2026-10-22): the stream that owns the decoder, if any.
    pub fn media_owner(&self) -> Option<u32> {
        self.media_owner
    }

    pub fn play(&mut self, system: &System, audio_handle: AudioHandle, repeat: bool) -> Result<(), AudioError> {
        wie_util::stats::add_smaf_play();

        // A clone of an `Arc`, where this used to be a full `parse_smaf` of the file —
        // on the emulator thread, inside the guest's `MC_mdaPlay`. See [`SmafCache`].
        let score = match self.files.get(&audio_handle) {
            Some(AudioFile::Smaf(events)) => events.clone(),
            None => return Err(AudioError::InvalidHandle),
        };
        // An MA-2 score goes to the sink's FM synth when it has one (`AudioSink::fm_supported`),
        // and a short one is an effect there. See [`MIDI_EFFECT_MAX_MS`].
        let fm = score.ma2.is_some() && self.sink.fm_supported();
        let is_music = score.is_music && !(fm && score.duration_ms < MIDI_EFFECT_MAX_MS);
        let player = SmafPlayer {
            events: score.clone(),
            is_music,
            fm_part: fm.then_some(if is_music { FM_PART_MUSIC } else { FM_PART_EFFECT }),
        };

        let now_ms = system.platform().now().raw();

        // ── the guest's single sound slot, undone ────────────────────────────────
        // See [`MUSIC_STOP_GRACE_MS`]: a music stop is deferred, and what happens next
        // is what decides whether it was real.
        // Taken out of `self` up front so the branches below can still touch `self`.
        let live_music = self
            .music
            .as_ref()
            .filter(|m| m.gate.is_running())
            .map(|m| (m.score.clone(), m.gate.clone(), m.volume.clone(), m.handle));

        if is_music {
            if let Some((live_score, gate, volume, handle)) = live_music {
                if Arc::ptr_eq(&live_score, &score) {
                    // The same song the host is already playing. The guest only re-issued
                    // it because it had to borrow its one clip for an effect — take the
                    // pending stop back and DO NOT restart, so the player hears one
                    // unbroken piece of music.
                    gate.keep_going();
                    // The guest may have changed 배경음 while it was away; the live
                    // playback reads its own cell, so move the new level onto it.
                    let level = self.clip_volume(audio_handle);
                    volume.store(level, Ordering::Relaxed);
                    self.playing.insert(handle, gate);
                    return Ok(());
                }
                // A different song: the deferred stop was real after all — and the old one
                // must be SILENT before the new one starts. See [`PlaybackGate::silence`].
                gate.silence(&**self.sink);
                self.playing.remove(&handle);
            }
        } else if let Some((_, gate, _, _)) = live_music {
            // An effect. This is the guest taking its turn with the one clip, so hold
            // the music open until the guest puts it back and let them overlap. See
            // [`effect_hold_ms`].
            gate.hold_until(now_ms + effect_hold_ms(score.duration_ms));
        }

        self.stop_now(audio_handle);

        let mut system_clone = system.clone();
        let sink_clone = self.sink.clone();
        let volume = self.volume_arc(audio_handle);

        let gate = Arc::new(PlaybackGate::new());
        let gate_clone = gate.clone();
        self.playing.insert(audio_handle, gate.clone());

        if is_music {
            self.music = Some(MusicChannel {
                score,
                gate: gate.clone(),
                volume: volume.clone(),
                handle: audio_handle,
            });
        }

        // TODO use dedicated audio player task
        system.spawn(async move || {
            player.play(&mut system_clone, &**sink_clone, &gate_clone, repeat, volume).await;
            gate_clone.finished.store(true, Ordering::Relaxed);

            Ok(())
        });

        Ok(())
    }

    /// `MC_mdaStop`. A stop of the MUSIC is DEFERRED — see [`MUSIC_STOP_GRACE_MS`] for
    /// why taking it literally makes every skill restart the song from the beginning.
    /// Everything else stops immediately, as before.
    pub fn stop(&mut self, now_ms: u64, audio_handle: AudioHandle) {
        if let Some(music) = &self.music
            && music.handle == audio_handle
            && music.gate.is_running()
        {
            music.gate.stop_at(now_ms + MUSIC_STOP_GRACE_MS);
            return;
        }
        self.stop_now(audio_handle);
    }

    /// Stop a playback for real, right now — and silence what it is holding, so its notes
    /// do not ring on until its next score event. See [`PlaybackGate::silence`].
    fn stop_now(&mut self, audio_handle: AudioHandle) {
        if let Some(gate) = self.playing.remove(&audio_handle) {
            gate.silence(&**self.sink);
        }
    }

    pub fn close(&mut self, audio_handle: AudioHandle) -> Result<(), AudioError> {
        self.stop_now(audio_handle);
        self.volumes.remove(&audio_handle);

        if self.files.remove(&audio_handle).is_none() {
            return Err(AudioError::InvalidHandle);
        }
        wie_util::stats::set_audio_handles(self.files.len() as u64);

        Ok(())
    }
}

/// How far behind the wall clock a song may fall before it stops trying to catch up
/// event-by-event and simply resumes from now.
///
/// ★ 2026-08-09. The app STOPS TICKING the emulator while it is backgrounded or the
/// screen is locked (`emulator.rs` step 0), but `Platform::now()` is the wall clock and
/// keeps running. On the next foreground the song is behind by however long the screen
/// was off, and the loop below — which only `sleep`s when the next event is still in the
/// future — dispatched the ENTIRE remaining backlog inside one poll: thousands of MIDI
/// events pushed at the synth in a burst, on the emulator thread, in a single frame.
/// Past this threshold the timeline is re-based instead, which is what a real sequencer
/// does with a stalled clock.
const MAX_CATCHUP_MS: u64 = 250;

/// The longest a playback sleeps before looking at its gate again.
///
/// ★ 2026-09-25. A playback used to sleep straight through to its next score event, so a
/// deferred stop ([`MUSIC_STOP_GRACE_MS`]) that came due in a long held chord was only
/// noticed when that chord ended — up to 2.7 s late in hero2 (`P/snd/02`), with the notes
/// sounding the whole time. Waking in slices bounds that at this much. The guest's own
/// frame timer is 50 ms, so this adds no wakeups the emulator loop is not taking anyway.
const STOP_POLL_MS: u64 = 50;

/// Wait until `time` ms into a timeline that started at `*start_ms` (epoch ms), in slices of
/// [`STOP_POLL_MS`], re-reading the clock each time — a deferred stop may come due in the
/// middle of a long held chord. `false` = the gate says stop.
///
/// Epoch millis, saturating: `Platform::now()` is the WALL clock and can step backwards
/// (NTP), which as an `Instant - Instant` would underflow to a 584-million-year "elapsed"
/// and dispatch the whole song at once.
async fn wait_for_event(system: &mut System, gate: &PlaybackGate, start_ms: &mut u64, time: u64) -> bool {
    let mut now = system.platform().now().raw();
    if gate.should_stop(now) {
        return false;
    }
    let mut elapsed = now.saturating_sub(*start_ms);
    if time > elapsed {
        while time > elapsed {
            system.sleep((time - elapsed).min(STOP_POLL_MS) as _).await;
            now = system.platform().now().raw();
            if gate.should_stop(now) {
                return false;
            }
            elapsed = now.saturating_sub(*start_ms);
        }
    } else if elapsed - time > MAX_CATCHUP_MS {
        // The clock ran away from us (backgrounded, or the device could not keep up).
        // Re-base the timeline so the rest of the song plays in real time from here, instead
        // of flushing the whole backlog into one poll. `start_ms` moves forward by exactly the
        // deficit, so this event is "due now" and every later one keeps its original spacing.
        *start_ms = start_ms.saturating_add(elapsed - time);
        wie_util::stats::add_smaf_catchup();
    }
    true
}

pub struct SmafPlayer {
    events: Events,
    /// Whether this playback owns the sink's music bus — `Audio::play` decides, because a
    /// short score on the FM path is an effect (see [`MIDI_EFFECT_MAX_MS`]).
    is_music: bool,
    /// `Some(part)`: play the score's own FM voices on that part of the sink's FM synth
    /// ([`AudioSink::fm_supported`]) instead of General MIDI.
    fm_part: Option<u8>,
}

impl SmafPlayer {
    pub async fn play(&self, system: &mut System, sink: &dyn AudioSink, gate: &PlaybackGate, repeat: bool, volume: Arc<AtomicU8>) {
        if let (Some(part), Some(song)) = (self.fm_part, self.events.ma2.as_ref()) {
            return self.play_fm(song, part, system, sink, gate, repeat, volume).await;
        }
        // The level last pushed to the sink's music bus, and whether the sink took it.
        // `u8::MAX` is not a reachable volume (`clip_set_volume` clamps to 100), so the
        // first event of every playback always publishes the current level.
        //
        // ★ Only a MUSIC file ever touches that bus — see [`Score`]. An effect playing
        // at a different setting must not drag the music's level with it.
        let is_music = self.is_music;
        let mut pushed_vol = u8::MAX;
        let mut host_music_bus = false;

        loop {
            let mut active_notes: Vec<(u8, u8)> = Vec::new();
            let mut used_channels: BTreeSet<u8> = BTreeSet::new();

            let mut start_ms = system.platform().now().raw();
            for (time, event) in self.events.events.iter() {
                if !wait_for_event(system, gate, &mut start_ms, *time as u64).await {
                    break;
                }

                // Per-clip volume (MC_mdaClipSetVolume), read live so the game's
                // volume slider takes effect on the currently-playing clip.
                let vol = volume.load(Ordering::Relaxed); // 0..=100
                if is_music && vol != pushed_vol {
                    pushed_vol = vol;
                    // Hand the level to the sink's music bus. See
                    // `AudioSink::set_music_volume`: when the sink owns a bus it applies
                    // `volume_gain` to the rendered audio and we must leave velocity alone.
                    host_music_bus = sink.set_music_volume(vol);
                }
                let gain = crate::volume_gain(vol);

                match event {
                    SmafEvent::Wave {
                        channel,
                        sampling_rate,
                        data,
                    } => {
                        // SFX volume: scale PCM by the shared law. Skip entirely when muted.
                        if vol == 0 {
                            // muted
                        } else if vol >= 100 {
                            sink.play_wave(*channel, *sampling_rate, data);
                        } else {
                            let scaled: Vec<i16> = data.iter().map(|s| (*s as f32 * gain) as i16).collect();
                            sink.play_wave(*channel, *sampling_rate, &scaled);
                        }
                    }
                    SmafEvent::MidiNoteOn { channel, note, velocity } => {
                        // BGM volume. With a host music bus the level is already applied to
                        // the synth's OUTPUT, so the note keeps the velocity the score wrote
                        // — velocity also picks the SoundFont layer and the filter cutoff, so
                        // scaling it here made low settings dull as well as quiet, and by a
                        // completely different curve than the PCM bus above.
                        //
                        // Sinks without a bus (external synths) keep the old behaviour;
                        // >=1 so an audible note never collapses into a note-off.
                        let v = if *velocity == 0 || vol == 0 {
                            0
                        } else if host_music_bus {
                            *velocity
                        } else {
                            (((*velocity as u32) * vol as u32 / 100) as u8).max(1)
                        };
                        sink.midi_note_on(*channel, *note, v);
                        active_notes.push((*channel, *note));
                        used_channels.insert(*channel);
                        gate.touch_channel(*channel);
                    }
                    SmafEvent::MidiNoteOff { channel, note, velocity } => {
                        sink.midi_note_off(*channel, *note, *velocity);
                        active_notes.retain(|(c, n)| !(*c == *channel && *n == *note));
                    }
                    SmafEvent::MidiProgramChange { channel, program } => {
                        sink.midi_program_change(*channel, *program);
                        used_channels.insert(*channel);
                        gate.touch_channel(*channel);
                    }
                    SmafEvent::MidiControlChange { channel, control, value } => {
                        sink.midi_control_change(*channel, *control, *value);
                        used_channels.insert(*channel);
                        gate.touch_channel(*channel);
                    }
                    SmafEvent::MidiPitchBend { channel, value } => {
                        sink.midi_pitch_bend(*channel, *value);
                        used_channels.insert(*channel);
                        gate.touch_channel(*channel);
                    }
                    SmafEvent::MidiSysEx(data) => {
                        sink.midi_sysex(data);
                    }
                    SmafEvent::End => {}
                }
            }

            // ★ Not after [`PlaybackGate::silence`]: those channels were cleared when the
            // stop happened, and by now they may belong to the song that replaced this one —
            // clearing them again would cut its notes.
            if !gate.silenced.load(Ordering::Relaxed) {
                for (channel, note) in &active_notes {
                    sink.midi_note_off(*channel, *note, 0);
                }

                for channel in &used_channels {
                    sink.midi_control_change(*channel, 64, 0);
                    sink.midi_control_change(*channel, 120, 0);
                    sink.midi_control_change(*channel, 123, 0);
                }
            }

            if !repeat || gate.should_stop(system.platform().now().raw()) {
                break;
            }
        }
    }

    /// The same playback on the sink's FM synth: the song's voices first, then its events on
    /// their millisecond — same clock, same stop rules ([`wait_for_event`]), same loop.
    ///
    /// ★ No velocity: an MA-2 score has none, and the level is the sink's music bus
    /// ([`AudioSink::set_music_volume`]) like the General MIDI path's. A muted clip plays
    /// no notes.
    #[allow(clippy::too_many_arguments)]
    async fn play_fm(
        &self,
        song: &Ma2Song,
        part: u8,
        system: &mut System,
        sink: &dyn AudioSink,
        gate: &PlaybackGate,
        repeat: bool,
        volume: Arc<AtomicU8>,
    ) {
        gate.fm_part.store(part, Ordering::Relaxed);
        sink.fm_voices(part, &song.voices);
        let mut pushed_vol = u8::MAX;
        loop {
            // The note each channel holds, so a stop mid-note can release it.
            let mut held: [Option<u8>; MA2_CHANNELS] = [None; MA2_CHANNELS];
            let mut start_ms = system.platform().now().raw();
            for (time, event) in song.events.iter() {
                if !wait_for_event(system, gate, &mut start_ms, *time as u64).await {
                    break;
                }
                let vol = volume.load(Ordering::Relaxed);
                if self.is_music && vol != pushed_vol {
                    pushed_vol = vol;
                    sink.set_music_volume(vol);
                }
                match *event {
                    Ma2Event::End => {}
                    Ma2Event::NoteOn { channel, note } => {
                        if vol != 0 {
                            held[channel as usize % MA2_CHANNELS] = Some(note);
                            sink.fm_event(part, *event);
                        }
                    }
                    Ma2Event::NoteOff { channel, note } => {
                        let slot = &mut held[channel as usize % MA2_CHANNELS];
                        if *slot == Some(note) {
                            *slot = None;
                        }
                        sink.fm_event(part, *event);
                    }
                    _ => sink.fm_event(part, *event),
                }
            }

            // Release what is still held — a key-off, so the notes end the way they would
            // have. ★ Not after [`PlaybackGate::silence`]: the part was cut when the stop
            // happened and may be another song's by now.
            if !gate.silenced.load(Ordering::Relaxed) {
                for (channel, note) in held.iter().enumerate() {
                    if let Some(note) = *note {
                        sink.fm_event(
                            part,
                            Ma2Event::NoteOff {
                                channel: channel as u8,
                                note,
                            },
                        );
                    }
                }
            }

            if !repeat || gate.should_stop(system.platform().now().raw()) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
    use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    use smaf_player::SmafEvent;

    use super::{GUEST_REISSUE_MS, MUSIC_REJOIN_MS, MUSIC_STOP_GRACE_MS, PlaybackGate, REISSUE_SLACK_MS, Score, SmafPlayer, effect_hold_ms};
    use crate::{AudioSink, Database, DatabaseRepository, DefaultTaskRunner, Filesystem, Instant, Platform, Screen, System, canvas::Image};

    struct NullDatabase;

    #[async_trait::async_trait]
    impl Database for NullDatabase {
        async fn next_id(&self) -> u32 {
            1
        }

        async fn add(&mut self, _data: &[u8]) -> Option<u32> {
            Some(1)
        }

        async fn get(&self, _id: u32) -> Option<alloc::vec::Vec<u8>> {
            None
        }

        async fn set(&mut self, _id: u32, _data: &[u8]) -> bool {
            true
        }

        async fn delete(&mut self, _id: u32) -> bool {
            true
        }

        async fn get_record_ids(&self) -> alloc::vec::Vec<u32> {
            vec![]
        }
    }

    struct NullDatabaseRepository;

    #[async_trait::async_trait]
    impl DatabaseRepository for NullDatabaseRepository {
        async fn open(&self, _name: &str, _app_id: &str) -> Box<dyn Database> {
            Box::new(NullDatabase)
        }

        async fn exists(&self, _name: &str, _app_id: &str) -> bool {
            false
        }

        async fn delete(&self, _name: &str, _app_id: &str) -> bool {
            false
        }
    }

    struct NullFilesystem;

    #[async_trait::async_trait]
    impl Filesystem for NullFilesystem {
        async fn exists(&self, _aid: &str, _path: &str) -> bool {
            false
        }

        async fn size(&self, _aid: &str, _path: &str) -> Option<usize> {
            None
        }

        async fn read(&self, _aid: &str, _path: &str, _offset: usize, _count: usize, _buf: &mut [u8]) -> Option<usize> {
            None
        }

        async fn write(&self, _aid: &str, _path: &str, _offset: usize, data: &[u8]) -> usize {
            data.len()
        }

        async fn truncate(&self, _aid: &str, _path: &str, _len: usize) {}
    }

    struct NullScreen;

    impl Screen for NullScreen {
        fn request_redraw(&self) -> wie_util::Result<()> {
            Ok(())
        }

        fn paint(&self, _image: &dyn Image) {}

        fn width(&self) -> u32 {
            240
        }

        fn height(&self) -> u32 {
            320
        }
    }

    struct NullPlatform {
        screen: NullScreen,
        database_repository: NullDatabaseRepository,
        filesystem: NullFilesystem,
        now: AtomicUsize,
    }

    impl NullPlatform {
        fn new() -> Self {
            Self {
                screen: NullScreen,
                database_repository: NullDatabaseRepository,
                filesystem: NullFilesystem,
                now: AtomicUsize::new(0),
            }
        }
    }

    impl Platform for NullPlatform {
        fn screen(&self) -> &dyn Screen {
            &self.screen
        }

        fn now(&self) -> Instant {
            Instant::from_epoch_millis(self.now.fetch_add(8, Ordering::SeqCst) as u64)
        }

        fn database_repository(&self) -> &dyn DatabaseRepository {
            &self.database_repository
        }

        fn filesystem(&self) -> &dyn Filesystem {
            &self.filesystem
        }

        fn audio_sink(&self) -> Box<dyn AudioSink> {
            Box::new(NoopAudioSink)
        }

        fn write_stdout(&self, _buf: &[u8]) {}

        fn write_stderr(&self, _buf: &[u8]) {}

        fn exit(&self) {}

        fn vibrate(&self, _duration_ms: u64, _intensity: u8) {}
    }

    struct NoopAudioSink;

    impl AudioSink for NoopAudioSink {
        fn play_wave(&self, _channel: u8, _sampling_rate: u32, _wave_data: &[i16]) {}

        fn midi_note_on(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_note_off(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_program_change(&self, _channel_id: u8, _program: u8) {}

        fn midi_control_change(&self, _channel_id: u8, _control: u8, _value: u8) {}

        fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}

        fn midi_sysex(&self, _data: &[u8]) {}
    }

    struct CountingSink {
        program_change_count: Arc<AtomicUsize>,
        stop_after: usize,
        stop_flag: Arc<PlaybackGate>,
    }

    impl AudioSink for CountingSink {
        fn play_wave(&self, _channel: u8, _sampling_rate: u32, _wave_data: &[i16]) {}

        fn midi_note_on(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_note_off(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_program_change(&self, _channel_id: u8, _program: u8) {
            let count = self.program_change_count.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= self.stop_after {
                self.stop_flag.kill();
            }
        }

        fn midi_control_change(&self, _channel_id: u8, _control: u8, _value: u8) {}

        fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}

        fn midi_sysex(&self, _data: &[u8]) {}
    }

    fn new_system() -> System {
        System::new(Box::new(NullPlatform::new()), "test-pid", "test-aid", DefaultTaskRunner)
    }

    #[test]
    fn score_classifies_the_two_buses() {
        // A Hero3 SFX (`res/snd/sd*_mf`, a SMAF `ATR` chunk) decodes to nothing but
        // wave one-shots. It must NOT be allowed to drive the host music bus.
        let sfx = Score {
            events: vec![
                (
                    0,
                    SmafEvent::Wave {
                        channel: 1,
                        sampling_rate: 16000,
                        data: vec![0, 1, 2],
                    },
                ),
                (10, SmafEvent::End),
            ],
            is_music: false,
            duration_ms: 10,
            ma2: None,
        };
        assert!(!sfx.is_music);

        // The real classifier, over the same shapes.
        let classify = |events: Vec<(usize, SmafEvent)>| events.iter().any(|(_, e)| !matches!(e, SmafEvent::Wave { .. } | SmafEvent::End));
        assert!(!classify(sfx.events));
        assert!(classify(vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })]));
        assert!(classify(vec![(
            0,
            SmafEvent::MidiNoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }
        )]));
        // An empty file is not music (nothing to render, nothing to level).
        assert!(!classify(vec![]));
    }

    /// The deferred-stop state machine, driven with the exact call order the device
    /// trace recorded. See [`MUSIC_STOP_GRACE_MS`].
    #[test]
    fn a_guest_stop_of_the_music_survives_an_effect_but_not_a_menu() {
        let gate = PlaybackGate::new();
        let t0 = 1_000_000u64;

        // The guest stops its BGM to free its one clip (device: 20:19:19.435).
        gate.stop_at(t0 + MUSIC_STOP_GRACE_MS);
        assert!(!gate.should_stop(t0), "the stop must not be immediate");
        assert!(gate.should_stop(t0 + MUSIC_STOP_GRACE_MS), "…but it must be armed");

        // 37 ms later the effect arrives (device: 20:19:19.472). It is ~600 ms long.
        gate.hold_until(t0 + 37 + effect_hold_ms(600));
        assert!(!gate.should_stop(t0 + MUSIC_STOP_GRACE_MS), "an effect must hold the music open");
        // The guest re-issues the SAME song when the effect ends (device: +626 ms).
        assert!(!gate.should_stop(t0 + 626), "the music must still be alive at the re-issue");
        gate.keep_going();
        assert!(!gate.should_stop(t0 + 60_000), "…and then run indefinitely");
        assert!(gate.is_running());

        // Now the menu: a stop with nothing after it (device: 4.5 s of silence).
        gate.stop_at(t0 + 100_000 + MUSIC_STOP_GRACE_MS);
        assert!(!gate.should_stop(t0 + 100_000 + MUSIC_STOP_GRACE_MS - 1));
        assert!(gate.should_stop(t0 + 100_000 + MUSIC_STOP_GRACE_MS), "a real stop must land");
    }

    #[test]
    fn hold_until_never_pulls_a_deadline_in_and_never_revives_a_dead_one() {
        let gate = PlaybackGate::new();
        // No pending stop: an effect must not invent one.
        gate.hold_until(5_000);
        assert!(!gate.should_stop(u64::MAX), "an effect alone must never stop the music");

        gate.stop_at(10_000);
        gate.hold_until(9_000); // a shorter effect than the window already granted
        assert!(!gate.should_stop(9_500), "the deadline must not move backwards");
        assert!(gate.should_stop(10_000));

        // A hard kill outranks everything.
        let gate = PlaybackGate::new();
        gate.kill();
        gate.keep_going();
        assert!(gate.should_stop(0), "a different song must stop the old one at once");
        assert!(!gate.is_running());
    }

    #[test]
    fn an_effect_holds_the_music_until_hero2_puts_it_back() {
        // hero2's effect 0 (크리티컬, 246 ms): the game re-issues the song 3 s after the
        // effect started, so the hold must outlast that, not the effect.
        assert_eq!(effect_hold_ms(246), GUEST_REISSUE_MS + REISSUE_SLACK_MS);
        assert!(effect_hold_ms(0) > GUEST_REISSUE_MS);
        // An effect longer than the game's timer still gets its own length plus the slack.
        assert_eq!(effect_hold_ms(5_000), 5_000 + MUSIC_REJOIN_MS);
    }

    /// A file shaped like hero2's effects: `CNTI`, one `ATR` (mono 4-bit ADPCM at 8 kHz) whose
    /// sequence plays wave 1 once.
    fn effect_file(adpcm: &[u8]) -> Vec<u8> {
        fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut out = tag.to_vec();
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(body);
            out
        }
        let mut atr = vec![0x00, 0x00, 0x11, 0x00, 0x02, 0x02];
        atr.extend(chunk(b"AspI", b"st:\0\0\0\0,sp:\0\0\0\x0a,"));
        atr.extend(chunk(
            b"Atsq",
            &[0x01, 0x00, 0x37, 0x7f, 0x01, 0x01, 0x3c, 0x3d, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00],
        ));
        atr.extend(chunk(b"Awa\x01", adpcm));
        let mut body = chunk(b"CNTI", &[0x00, 0x01, 0x02, 0x00, 0x00]);
        body.extend(chunk(b"ATR\x00", &atr));
        body.extend_from_slice(&[0, 0]); // CRC
        chunk(b"MMMD", &body)
    }

    #[test]
    fn an_effect_plays_the_samples_the_ma2_decodes() {
        // ★ 2026-10-02 ("효과음이 툭툭 끊기는 느낌"): `smaf_player` reads the nibbles in the
        // other chip family's order; the score must carry `atr`'s decode instead.
        let adpcm: Vec<u8> = (0..400u32).map(|i| (i * 37 % 251) as u8).collect();
        let score = Score::parse(&effect_file(&adpcm));
        let waves: Vec<(u8, u32, &Vec<i16>)> = score
            .events
            .iter()
            .filter_map(|(_, e)| match e {
                SmafEvent::Wave {
                    channel,
                    sampling_rate,
                    data,
                } => Some((*channel, *sampling_rate, data)),
                _ => None,
            })
            .collect();
        assert_eq!(waves, [(1, 8000, &super::super::atr::decode_yamaha_adpcm(&adpcm))]);
        assert!(!score.is_music && score.ma2.is_none());
        // The sequence's own end (1 + 1 + 61 ticks of 4 ms) outlasts its 100 ms of samples.
        assert_eq!(score.duration_ms, 252);
    }

    #[test]
    fn score_measures_the_audible_length_of_an_effect() {
        // `Audio::play` holds the music open for the effect's DURATION, and a PCM
        // one-shot's event time is only when it starts — the samples are the length.
        let one_second_at_8k = Score {
            events: vec![(
                0,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 8000],
                },
            )],
            is_music: false,
            duration_ms: 0,
            ma2: None,
        };
        let _ = one_second_at_8k;
        // The real classifier, over the same shape (mirrors `Score::parse`).
        let measure = |events: &[(usize, SmafEvent)]| {
            let mut d = 0usize;
            for (time, event) in events.iter() {
                let end = match event {
                    SmafEvent::Wave {
                        sampling_rate,
                        data,
                        channel,
                    } => time + (data.len() / (*channel).max(1) as usize) * 1000 / (*sampling_rate).max(1) as usize,
                    _ => *time,
                };
                d = d.max(end);
            }
            d
        };
        assert_eq!(
            measure(&[(
                0,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 8000]
                }
            )]),
            1000
        );
        // Starting 500 ms in pushes the end out by 500 ms.
        assert_eq!(
            measure(&[(
                500,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 4000]
                }
            )]),
            1000
        );
        // A zero sampling rate must not divide by zero.
        assert_eq!(
            measure(&[(
                7,
                SmafEvent::Wave {
                    channel: 0,
                    sampling_rate: 0,
                    data: vec![0; 4]
                }
            )]),
            4007
        );
    }

    #[futures_test::test]
    async fn plays_once_when_repeat_is_false() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(PlaybackGate::new());
        let sink = CountingSink {
            program_change_count: counter.clone(),
            stop_after: usize::MAX,
            stop_flag: stop_flag.clone(),
        };
        let player = SmafPlayer {
            events: Arc::new(Score {
                events: vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })],
                is_music: true,
                duration_ms: 0,
                ma2: None,
            }),
            is_music: true,
            fm_part: None,
        };
        let mut system = new_system();

        player.play(&mut system, &sink, &stop_flag, false, Arc::new(AtomicU8::new(100))).await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[futures_test::test]
    async fn repeats_until_stop_flag_is_set() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(PlaybackGate::new());
        let sink = CountingSink {
            program_change_count: counter.clone(),
            stop_after: 2,
            stop_flag: stop_flag.clone(),
        };
        let player = SmafPlayer {
            events: Arc::new(Score {
                events: vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })],
                is_music: true,
                duration_ms: 0,
                ma2: None,
            }),
            is_music: true,
            fm_part: None,
        };
        let mut system = new_system();

        player.play(&mut system, &sink, &stop_flag, true, Arc::new(AtomicU8::new(100))).await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    // ── Song hand-over (2026-09-25, "맵이 바뀔때 배경음악이 겹쳐지는 버그") ───────────────
    // A platform whose clock the test sets (plus one ms per read, so `Executor::tick`'s
    // 8 ms budget still ends) and a sink that records every MIDI event in order.

    extern crate std;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Midi {
        On(u8, u8),
        Off(u8, u8),
        Cc(u8, u8),
        Program(u8),
        SoundOff(u8),
        /// FM: a part's voice table (a new score on it).
        FmVoices(u8),
        Fm(u8, crate::Ma2Event),
        FmOff(u8),
    }

    type MidiLog = Arc<std::sync::Mutex<Vec<Midi>>>;

    /// `.1` = the sink has an FM synth (`AudioSink::fm_supported`).
    struct RecordingSink(MidiLog, bool);

    impl AudioSink for RecordingSink {
        fn fm_supported(&self) -> bool {
            self.1
        }

        fn fm_voices(&self, part: u8, _voices: &[crate::Ma2Voice]) {
            self.0.lock().unwrap().push(Midi::FmVoices(part));
        }

        fn fm_event(&self, part: u8, event: crate::Ma2Event) {
            self.0.lock().unwrap().push(Midi::Fm(part, event));
        }

        fn fm_all_off(&self, part: u8) {
            self.0.lock().unwrap().push(Midi::FmOff(part));
        }

        fn play_wave(&self, _channel: u8, _sampling_rate: u32, _wave_data: &[i16]) {}

        fn midi_note_on(&self, channel_id: u8, note: u8, _velocity: u8) {
            self.0.lock().unwrap().push(Midi::On(channel_id, note));
        }

        fn midi_note_off(&self, channel_id: u8, note: u8, _velocity: u8) {
            self.0.lock().unwrap().push(Midi::Off(channel_id, note));
        }

        fn midi_program_change(&self, channel_id: u8, _program: u8) {
            self.0.lock().unwrap().push(Midi::Program(channel_id));
        }

        fn midi_control_change(&self, channel_id: u8, control: u8, _value: u8) {
            self.0.lock().unwrap().push(Midi::Cc(channel_id, control));
        }

        fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}

        fn midi_sysex(&self, _data: &[u8]) {}

        fn midi_all_sound_off(&self, channel_id: u8) {
            self.0.lock().unwrap().push(Midi::SoundOff(channel_id));
        }
    }

    struct ClockPlatform {
        screen: NullScreen,
        database_repository: NullDatabaseRepository,
        filesystem: NullFilesystem,
        now: Arc<AtomicUsize>,
        log: MidiLog,
        fm: bool,
    }

    impl Platform for ClockPlatform {
        fn screen(&self) -> &dyn Screen {
            &self.screen
        }

        fn now(&self) -> Instant {
            Instant::from_epoch_millis(self.now.fetch_add(1, Ordering::SeqCst) as u64)
        }

        fn database_repository(&self) -> &dyn DatabaseRepository {
            &self.database_repository
        }

        fn filesystem(&self) -> &dyn Filesystem {
            &self.filesystem
        }

        fn audio_sink(&self) -> Box<dyn AudioSink> {
            Box::new(RecordingSink(self.log.clone(), self.fm))
        }

        fn write_stdout(&self, _buf: &[u8]) {}

        fn write_stderr(&self, _buf: &[u8]) {}

        fn exit(&self) {}

        fn vibrate(&self, _duration_ms: u64, _intensity: u8) {}
    }

    fn clock_system() -> (System, Arc<AtomicUsize>, MidiLog) {
        clock_system_with(false)
    }

    fn clock_system_with(fm: bool) -> (System, Arc<AtomicUsize>, MidiLog) {
        let now = Arc::new(AtomicUsize::new(1_000));
        let log: MidiLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let platform = ClockPlatform {
            screen: NullScreen,
            database_repository: NullDatabaseRepository,
            filesystem: NullFilesystem,
            now: now.clone(),
            log: log.clone(),
            fm,
        };
        (System::new(Box::new(platform), "test-pid", "test-aid", DefaultTaskRunner), now, log)
    }

    /// The shape that droned: a chord struck at 0 and held until 3000 — the task sleeps
    /// through the whole hold. `P/snd/02` holds six notes across 2.7 s like this.
    fn held_chord() -> Arc<Score> {
        Arc::new(Score {
            events: vec![
                (0, SmafEvent::MidiProgramChange { channel: 0, program: 48 }),
                (
                    0,
                    SmafEvent::MidiNoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                ),
                (
                    0,
                    SmafEvent::MidiNoteOn {
                        channel: 1,
                        note: 64,
                        velocity: 100,
                    },
                ),
                (
                    3000,
                    SmafEvent::MidiNoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                ),
                (
                    3000,
                    SmafEvent::MidiNoteOff {
                        channel: 1,
                        note: 64,
                        velocity: 0,
                    },
                ),
                (4000, SmafEvent::End),
            ],
            is_music: true,
            duration_ms: 4000,
            ma2: None,
        })
    }

    fn next_song() -> Arc<Score> {
        Arc::new(Score {
            events: vec![
                (
                    0,
                    SmafEvent::MidiNoteOn {
                        channel: 0,
                        note: 67,
                        velocity: 100,
                    },
                ),
                (
                    5000,
                    SmafEvent::MidiNoteOff {
                        channel: 0,
                        note: 67,
                        velocity: 0,
                    },
                ),
                (6000, SmafEvent::End),
            ],
            is_music: true,
            duration_ms: 6000,
            ma2: None,
        })
    }

    fn set_clock(now: &AtomicUsize, ms: usize) {
        now.store(ms, Ordering::SeqCst);
    }

    /// The map change as hero2 drives it (`br.a(Clip,Z)V`: `Player.stop` of the old clip,
    /// then `Player.play` of the new one), in the middle of the old song's held chord.
    #[test]
    fn a_replaced_song_is_silent_before_the_new_one_starts_and_stays_out_of_its_way() {
        let (mut system, now, log) = clock_system();
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(held_chord()));
            audio.files.insert(2, super::AudioFile::Smaf(next_song()));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();
        assert!(log.lock().unwrap().contains(&Midi::On(1, 64)), "the old chord is sounding");

        // 1 s into the 3 s hold: the guest stops the old clip and plays the new map's.
        set_clock(&now, 2_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, true).unwrap();

        // Before the new song has run at all, the old one's channels are already clear.
        let at_handover = log.lock().unwrap().len();
        {
            let l = log.lock().unwrap();
            for ch in [0u8, 1] {
                // The HARD stop: a control-120 release would let the old chord fade on under
                // the new song for as long as its release envelope (seconds on GeneralUser).
                assert!(l.contains(&Midi::SoundOff(ch)), "channel {ch} must be silenced at the hand-over: {l:?}");
            }
            assert!(!l.contains(&Midi::On(0, 67)), "the new song must not have started yet");
        }

        system.tick().unwrap();
        // Past the old song's next event: its task wakes, sees the stop — and must send
        // NOTHING, because anything it sends now lands on the new song's channel 0.
        set_clock(&now, 4_500);
        system.tick().unwrap();
        set_clock(&now, 5_500);
        system.tick().unwrap();

        let l = log.lock().unwrap();
        let after = &l[at_handover..];
        let new_on = after.iter().position(|e| *e == Midi::On(0, 67)).expect("the new song started");
        for e in &after[new_on..] {
            assert!(
                !matches!(
                    e,
                    Midi::Off(0, 60) | Midi::Off(1, 64) | Midi::Cc(0, 120) | Midi::Cc(0, 123) | Midi::SoundOff(_) | Midi::Cc(1, _)
                ),
                "the old song touched the synth after the new one started: {e:?} in {after:?}"
            );
        }
    }

    /// A stop with nothing after it (the guest opening a window) used to wait for the old
    /// song's next event — here 2.6 s past the deadline, chord sounding all the while.
    #[test]
    fn a_deferred_stop_lands_within_a_slice_even_inside_a_held_chord() {
        let (mut system, now, log) = clock_system();
        system.audio().files.insert(1, super::AudioFile::Smaf(held_chord()));
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();

        set_clock(&now, 1_100);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);

        // Walk the clock past the grace in frame-sized steps, as the emulator loop would.
        let deadline = t as usize + MUSIC_STOP_GRACE_MS as usize;
        let mut ms = 1_150;
        while ms <= deadline + super::STOP_POLL_MS as usize + 20 {
            set_clock(&now, ms);
            system.tick().unwrap();
            ms += 20;
        }
        let l = log.lock().unwrap();
        assert!(
            l.contains(&Midi::Off(0, 60)) && l.contains(&Midi::Cc(1, 120)),
            "the stop must have landed by now: {l:?}"
        );
        assert!(ms < 3_000, "…well before the chord's own end");
    }

    // ── Effect and re-issue (2026-10-01, "효과음이 나면 배경음이 처음부터 다시 나온다") ──────

    /// A song long enough that nothing in the test reaches its end: its program change at 0
    /// is the fingerprint of a (re)start.
    fn long_song() -> Arc<Score> {
        Arc::new(Score {
            events: vec![
                (0, SmafEvent::MidiProgramChange { channel: 0, program: 48 }),
                (
                    0,
                    SmafEvent::MidiNoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                ),
                (
                    20_000,
                    SmafEvent::MidiNoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                ),
                (30_000, SmafEvent::End),
            ],
            is_music: true,
            duration_ms: 30_000,
            ma2: None,
        })
    }

    /// hero2's effect 0 (크리티컬, 246 ms) as the PCM one-shot it is.
    fn crit_effect() -> Arc<Score> {
        Arc::new(Score {
            events: vec![(
                0,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 1968],
                },
            )],
            is_music: false,
            duration_ms: 246,
            ma2: None,
        })
    }

    /// An effect as hero2's `br` drives it (`HERO2_GUEST_RE.md` §8.37): `Player.stop` of the
    /// song and `Player.play` of the effect; then, 3 s later from the per-frame `br.a()V`,
    /// `Player.stop` of the effect and `Player.play` of the SAME song. The song must play on
    /// through all of it — no stop, no second start.
    #[test]
    fn hero2_puts_the_song_back_three_seconds_after_an_effect_and_it_never_restarts() {
        let (mut system, now, log) = clock_system();
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(long_song()));
            audio.files.insert(2, super::AudioFile::Smaf(crit_effect()));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();
        let starts = |l: &[Midi]| l.iter().filter(|e| **e == Midi::Program(0)).count();
        assert_eq!(starts(&log.lock().unwrap()), 1, "the song started once");

        // The effect, 2 s into the song.
        set_clock(&now, 3_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, false).unwrap();

        // The game waits 3 s, one 50 ms frame at a time …
        let mut ms = 3_000;
        while ms < 3_000 + GUEST_REISSUE_MS as usize {
            ms += 50;
            set_clock(&now, ms);
            system.tick().unwrap();
        }
        // … and puts the song back.
        let t = system.platform().now().raw();
        system.audio().stop(t, 2);
        system.audio().play(&system, 1, true).unwrap();
        // On past every hold there was.
        while ms < 3_000 + 8_000 {
            ms += 50;
            set_clock(&now, ms);
            system.tick().unwrap();
        }

        let l = log.lock().unwrap();
        assert_eq!(starts(&l), 1, "the song restarted from the top: {l:?}");
        assert!(
            !l.iter().any(|e| matches!(e, Midi::SoundOff(0) | Midi::Off(0, 60) | Midi::Cc(0, 120))),
            "the song was stopped under the effect: {l:?}"
        );
    }

    /// The other half: a stop that nothing takes back (the game opening a window, a map with
    /// no music) still lands — later than before, but it lands.
    #[test]
    fn an_effect_does_not_keep_a_real_stop_from_landing() {
        let (mut system, now, log) = clock_system();
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(long_song()));
            audio.files.insert(2, super::AudioFile::Smaf(crit_effect()));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();

        set_clock(&now, 3_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, false).unwrap();

        let give_up = 3_000 + (effect_hold_ms(246) + super::STOP_POLL_MS) as usize + 50;
        let mut ms = 3_000;
        while ms < give_up {
            ms += 50;
            set_clock(&now, ms);
            system.tick().unwrap();
        }
        let l = log.lock().unwrap();
        assert!(
            l.contains(&Midi::Off(0, 60)) && l.contains(&Midi::Cc(0, 120)),
            "the stop never landed: {l:?}"
        );
    }

    // ── The FM path (2026-10-01, `HERO2_BGM_FM_PLAN.md` §3 B3) ──────────────────────────────

    /// An MA-2 score: one voice, `notes` as `(on, off, channel, note)`, `len_ms` long — and
    /// the General MIDI reading beside it (a program change on channel 0) for a sink with
    /// no FM synth.
    fn ma2_score(len_ms: u32, notes: &[(u32, u32, u8, u8)]) -> Arc<Score> {
        use crate::{Ma2Event, Ma2Song, Ma2Voice};
        let mut events = vec![(
            0u32,
            Ma2Event::Program {
                channel: 0,
                bank: 0,
                program: 1,
            },
        )];
        for &(on, off, channel, note) in notes {
            events.push((on, Ma2Event::NoteOn { channel, note }));
            events.push((off, Ma2Event::NoteOff { channel, note }));
        }
        events.sort_by_key(|(t, _)| *t);
        events.push((len_ms, Ma2Event::End));
        Arc::new(Score {
            events: vec![
                (0, SmafEvent::MidiProgramChange { channel: 0, program: 1 }),
                (len_ms as usize, SmafEvent::End),
            ],
            is_music: true,
            duration_ms: len_ms as u64,
            ma2: Some(Ma2Song {
                voices: vec![Ma2Voice {
                    bank: 0,
                    program: 1,
                    data: [0; 12],
                }],
                events,
                len_ms,
                ..Default::default()
            }),
        })
    }

    fn fm_note_on(part: u8, channel: u8, note: u8) -> Midi {
        Midi::Fm(part, crate::Ma2Event::NoteOn { channel, note })
    }

    #[test]
    fn an_ma2_song_plays_on_the_fm_synth_when_the_sink_has_one() {
        let (mut system, _now, log) = clock_system_with(true);
        system
            .audio()
            .files
            .insert(1, super::AudioFile::Smaf(ma2_score(20_000, &[(0, 15_000, 0, 60)])));
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();
        let l = log.lock().unwrap();
        assert_eq!(l.first(), Some(&Midi::FmVoices(0)), "the voices go first, on the music part: {l:?}");
        assert!(l.contains(&fm_note_on(0, 0, 60)), "{l:?}");
        assert!(
            !l.iter().any(|e| matches!(e, Midi::Program(_) | Midi::On(..) | Midi::Cc(..))),
            "nothing may reach the General MIDI synth: {l:?}"
        );
    }

    #[test]
    fn without_an_fm_synth_the_same_score_plays_general_midi_as_before() {
        let (mut system, now, log) = clock_system_with(false);
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(ma2_score(20_000, &[(0, 15_000, 0, 60)])));
            audio.files.insert(2, super::AudioFile::Smaf(ma2_score(1_392, &[(0, 500, 0, 72)])));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();
        assert!(log.lock().unwrap().contains(&Midi::Program(0)));
        // A short score is still MUSIC under General MIDI: it shares the song's channels, so
        // it replaces the song, exactly as before the FM path existed.
        set_clock(&now, 3_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, false).unwrap();
        let l = log.lock().unwrap();
        assert!(l.contains(&Midi::SoundOff(0)), "{l:?}");
        assert!(!l.iter().any(|e| matches!(e, Midi::FmVoices(_) | Midi::Fm(..) | Midi::FmOff(_))), "{l:?}");
    }

    /// A map change on the FM path: the old song is cut on its part BEFORE the new one's
    /// voices arrive, and its task sends nothing afterwards.
    #[test]
    fn a_replaced_fm_song_is_cut_before_the_next_one_starts() {
        let (mut system, now, log) = clock_system_with(true);
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(ma2_score(20_000, &[(0, 15_000, 0, 60)])));
            audio.files.insert(2, super::AudioFile::Smaf(ma2_score(20_000, &[(0, 15_000, 1, 67)])));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();

        set_clock(&now, 2_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, true).unwrap();
        let cut = log.lock().unwrap().len();
        assert_eq!(log.lock().unwrap().last(), Some(&Midi::FmOff(0)), "the old song is cut at the hand-over");

        for ms in [2_100, 4_000, 16_000, 17_000] {
            set_clock(&now, ms);
            system.tick().unwrap();
        }
        let l = log.lock().unwrap();
        let after = &l[cut..];
        assert_eq!(after.first(), Some(&Midi::FmVoices(0)), "{after:?}");
        assert!(after.contains(&fm_note_on(0, 1, 67)), "{after:?}");
        assert!(
            !after
                .iter()
                .any(|e| matches!(e, Midi::Fm(0, crate::Ma2Event::NoteOff { channel: 0, note: 60 }))),
            "the old song touched the synth after it was replaced: {after:?}"
        );
    }

    /// hero2's 환경설정 sound (`P/snd/42`, 1,392 ms) on the FM path: its own part, over the
    /// song — the song is neither cut nor restarted.
    #[test]
    fn a_short_score_is_an_effect_on_its_own_part_and_the_song_plays_on() {
        let (mut system, now, log) = clock_system_with(true);
        {
            let mut audio = system.audio();
            audio.files.insert(1, super::AudioFile::Smaf(ma2_score(20_000, &[(0, 15_000, 0, 60)])));
            audio.files.insert(2, super::AudioFile::Smaf(ma2_score(1_392, &[(0, 500, 0, 72)])));
        }
        system.audio().play(&system, 1, true).unwrap();
        system.tick().unwrap();

        // The game stops the song for its effect, plays the effect, and puts the song back
        // 3 s later (`br.a(B)V`).
        set_clock(&now, 3_000);
        system.tick().unwrap();
        let t = system.platform().now().raw();
        system.audio().stop(t, 1);
        system.audio().play(&system, 2, false).unwrap();
        let mut ms = 3_000;
        while ms < 3_000 + GUEST_REISSUE_MS as usize {
            ms += 50;
            set_clock(&now, ms);
            system.tick().unwrap();
        }
        let t = system.platform().now().raw();
        system.audio().stop(t, 2);
        system.audio().play(&system, 1, true).unwrap();
        while ms < 12_000 {
            ms += 50;
            set_clock(&now, ms);
            system.tick().unwrap();
        }

        let l = log.lock().unwrap();
        assert!(
            l.contains(&Midi::FmVoices(1)) && l.contains(&fm_note_on(1, 0, 72)),
            "the effect plays on part 1: {l:?}"
        );
        assert_eq!(l.iter().filter(|e| **e == Midi::FmVoices(0)).count(), 1, "the song restarted: {l:?}");
        assert!(!l.contains(&Midi::FmOff(0)), "the song was cut: {l:?}");
        assert!(
            !l.iter().any(|e| matches!(e, Midi::Fm(0, crate::Ma2Event::NoteOff { .. }))),
            "the song's note was released under the effect: {l:?}"
        );
    }
}
