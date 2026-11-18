//! The Android audio path — Phase 4a (2026-10-22): the game's WAV effects; Phase 4c (same day):
//! its AAC music.
//!
//! What arrives: `AudioSink::play_wave` one-shots — the core's `net.wie.MediaPlayer` decodes
//! each `/snd/e/*` (8-bit 8 kHz mono PCM) on `start()`, applies the game's `VolumeControl`
//! level and hands the clip to `Audio::play_pcm`, which is this sink — and `AudioSink::media_*`
//! for the songs, which go to the wie-music thread ([`crate::music`]) and come back to the bus
//! decoded, through [`MusicIn`]. The one MIDI song (Phase 4b) reaches the music thread and
//! stops the song it replaces, but is not rendered yet.
//!
//! The shape is hero2NativeAPP's `wie_android/audio_sink.rs` with the synths left out:
//!
//!   * [`AndroidAudioSink`] (the core's thread) pushes each clip down an mpsc channel;
//!   * the **wie-audio thread** ([`spawn_audio_thread`], Android only) opens the output
//!     through rodio (cpal → AAudio), retrying forever — a failed first open is a cold-boot
//!     race or a Bluetooth route mid-switch, not a fact about the device — and converts each
//!     clip ONCE to the device's rate and stereo (`sfx_upsample`, band-limited: rodio's
//!     linear resampler leaves an 8 kHz clip's mirror images audible), then
//!   * the [`Bus`] — the single source on rodio's device mixer, i.e. the audio callback —
//!     sums the music and the clips in flight, limits the sum (`audio_mix::soft_limit`) and is
//!     deliberately INFINITE: rodio drops a mixer input the moment it returns `None`, and a
//!     master bus that can be dropped is a session of silence with no way back.
//!
//! Backgrounded (`paused`, the same flag that freezes the worker): the bus drops the clips in
//! flight — an effect that resumed seconds later would be a blip from nowhere — and the music
//! fades out and holds its place, to go on from there on return. The bus and the conversion are
//! host-tested; only the rodio glue is Android-only.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender},
};

use wie_backend::AudioSink;

use crate::{
    audio_mix::{SfxClips, sfx_key, soft_limit},
    music::{MusicIn, MusicMsg},
    sfx_upsample::sfx_clip,
};

/// (channel count, sampling rate, interleaved i16 samples) — one clip, as `play_wave` gave it.
pub type WaveMsg = (u8, u32, Vec<i16>);

/// Frames per render chunk (~1.3 ms at 48 kHz): the granularity at which a new clip joins the
/// mix and the pause flag is read. The per-sample path is an index into the chunk.
pub const CHUNK_FRAMES: usize = 64;

/// Only if the device reports something absurd (rodio's rate is `NonZero`, so 0 cannot happen).
pub const FALLBACK_SAMPLE_RATE: u32 = 48_000;

/// The whole mix, as the single source the device mixer pulls from. Stereo `f32` at
/// `sample_rate`, rendered in [`CHUNK_FRAMES`]-frame chunks.
pub struct Bus {
    sfx_rx: Receiver<(u64, Vec<f32>)>,
    clips: SfxClips,
    music: MusicIn,
    sample_rate: u32,
    paused: Arc<AtomicBool>,
    buf: [f32; CHUNK_FRAMES * 2],
    pos: usize,
}

impl Bus {
    pub fn new(sfx_rx: Receiver<(u64, Vec<f32>)>, music: MusicIn, sample_rate: u32, paused: Arc<AtomicBool>) -> Self {
        Self {
            sfx_rx,
            clips: SfxClips::new(sample_rate),
            music,
            sample_rate,
            paused,
            buf: [0.0; CHUNK_FRAMES * 2],
            pos: CHUNK_FRAMES * 2,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Clips in flight (tests).
    pub fn playing(&self) -> usize {
        self.clips.len()
    }

    fn render_chunk(&mut self) {
        self.pos = 0;
        self.buf.fill(0.0);
        let paused = self.paused.load(Ordering::Relaxed);
        // The music first: it fades out and holds its place while the app is in the background.
        self.music.mix_into(&mut self.buf, paused);
        if paused {
            // The queue too: a clip the game sent just before the pause (or one the conversion
            // thread finished during it) would otherwise wait there and play on return.
            self.clips.clear();
            while self.sfx_rx.try_recv().is_ok() {}
        } else {
            // New effects join at a chunk boundary (≤ 1.3 ms late — inaudible). A copy of an
            // effect already playing twice takes the oldest one's place.
            while let Ok((key, data)) = self.sfx_rx.try_recv() {
                self.clips.push(key, data);
            }
            self.clips.mix_into(&mut self.buf);
        }
        for s in self.buf.iter_mut() {
            *s = soft_limit(*s);
        }
    }
}

impl Iterator for Bus {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.buf.len() {
            self.render_chunk();
        }
        let s = self.buf[self.pos];
        self.pos += 1;
        Some(s)
    }
}

/// One `play_wave` clip as the bus plays it: converted to the device rate and stereo, at
/// [`SFX_GAIN`] and the player's level (`audio_mix::sfx_gain`), with its identity for [`SfxClips`]. `None` for a clip that cannot play
/// (no channels, no rate, no samples).
pub fn convert_clip(channel: u8, sampling_rate: u32, wave_data: &[i16], device_rate: u32) -> Option<(u64, Vec<f32>)> {
    if channel == 0 || sampling_rate == 0 || wave_data.is_empty() {
        return None;
    }
    let clip = sfx_clip(wave_data, channel, sampling_rate, device_rate, crate::audio_mix::sfx_gain());
    Some((sfx_key(channel, sampling_rate, wave_data), clip))
}

/// The conversion loop of the wie-audio thread: every clip the core sends, converted once and
/// handed to the bus. Returns when either side is gone.
pub fn convert_loop(wave_rx: &Receiver<WaveMsg>, sfx_tx: &Sender<(u64, Vec<f32>)>, device_rate: u32) {
    while let Ok((channel, sampling_rate, wave_data)) = wave_rx.recv() {
        if let Some(clip) = convert_clip(channel, sampling_rate, &wave_data, device_rate) {
            if sfx_tx.send(clip).is_err() {
                break;
            }
        }
    }
}

/// The core's `AudioSink`: effects go to the audio thread, media streams (the songs) to the
/// music thread; the SMAF MIDI calls have nowhere to go (this game has no SMAF).
pub struct AndroidAudioSink {
    wave_tx: Sender<WaveMsg>,
    music_tx: Sender<MusicMsg>,
}

impl AndroidAudioSink {
    pub fn new(wave_tx: Sender<WaveMsg>, music_tx: Sender<MusicMsg>) -> Self {
        Self { wave_tx, music_tx }
    }
}

impl AudioSink for AndroidAudioSink {
    fn play_wave(&self, channel: u8, sampling_rate: u32, wave_data: &[i16]) {
        // One line per clip (effects are sparse): the logcat proof that a sound left the core,
        // with the level already in (`peak`), when a tester reports silence.
        let peak = wave_data.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0);
        log::info!("audio: clip {channel} ch {sampling_rate} Hz {} samples peak {peak}", wave_data.len());
        let _ = self.wave_tx.send((channel, sampling_rate, wave_data.to_vec()));
    }
    fn midi_note_on(&self, _channel_id: u8, _note: u8, _velocity: u8) {}
    fn midi_note_off(&self, _channel_id: u8, _note: u8, _velocity: u8) {}
    fn midi_program_change(&self, _channel_id: u8, _program: u8) {}
    fn midi_control_change(&self, _channel_id: u8, _control: u8, _value: u8) {}
    fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}
    fn midi_sysex(&self, _data: &[u8]) {}

    fn media_start(&self, id: u32, data: &Arc<[u8]>, loops: i32, level: u8) {
        log::info!("audio: media stream {id} start, {} bytes, loop count {loops}, level {level}", data.len());
        let _ = self.music_tx.send(MusicMsg::Start { id, data: data.clone(), loops, level });
    }
    fn media_stop(&self, id: u32) {
        log::info!("audio: media stream {id} stop");
        let _ = self.music_tx.send(MusicMsg::Stop { id });
    }
    fn media_level(&self, id: u32, level: u8) {
        let _ = self.music_tx.send(MusicMsg::Level { id, level });
    }
    fn media_close(&self, id: u32) {
        log::info!("audio: media stream {id} close");
        let _ = self.music_tx.send(MusicMsg::Close { id });
    }
}

#[cfg(any(target_os = "android", target_env = "msvc"))]
pub use device::spawn_audio_thread;

/// No device on this host (the windows-gnu toolchain — see `Cargo.toml`): the senders go to
/// receivers nobody reads, every clip is dropped silently, and the game runs mute.
#[cfg(not(any(target_os = "android", target_env = "msvc")))]
pub fn spawn_audio_thread(_paused: Arc<AtomicBool>, _renditions: Vec<Arc<[u8]>>) -> (Sender<WaveMsg>, Sender<MusicMsg>) {
    log::warn!("audio: no output on this toolchain (build the desktop app with MSVC) — running silent");
    let (wave_tx, _wave_rx) = std::sync::mpsc::channel::<WaveMsg>();
    let (music_tx, _music_rx) = std::sync::mpsc::channel::<MusicMsg>();
    (wave_tx, music_tx)
}

/// The rodio glue: Android (cpal's AAudio backend wants `ndk_context`, which the worker thread has
/// by the time the platform is built) and the MSVC desktop build (cpal's WASAPI).
#[cfg(any(target_os = "android", target_env = "msvc"))]
mod device {
    use std::{
        num::NonZero,
        sync::mpsc::{channel, sync_channel},
        thread,
        time::Duration,
    };

    use rodio::{ChannelCount, DeviceSinkBuilder, SampleRate, Source};

    use super::*;
    use crate::music::{MUSIC_QUEUE_CHUNKS, MusicChunk, MusicShared, music_loop, open_source_with};

    impl Source for Bus {
        fn current_span_len(&self) -> Option<usize> {
            None // infinite, parameters never change
        }
        fn channels(&self) -> ChannelCount {
            NonZero::new(2).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(self.sample_rate.max(1)).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    /// Spawn the output thread; the senders are what [`AndroidAudioSink`] pushes to — effects and
    /// songs. `renditions` are the WAVs the build rendered from the game's MIDI files (Phase 4b).
    /// Call once, from the worker (`AndroidPlatform::new`).
    pub fn spawn_audio_thread(paused: Arc<AtomicBool>, renditions: Vec<Arc<[u8]>>) -> (Sender<WaveMsg>, Sender<MusicMsg>) {
        let (wave_tx, wave_rx) = channel::<WaveMsg>();
        let (music_tx, music_rx) = channel::<MusicMsg>();
        if let Err(e) = thread::Builder::new()
            .name("wie-audio".into())
            .spawn(move || audio_thread(wave_rx, music_rx, paused, renditions))
        {
            log::error!("audio: cannot start the audio thread: {e}");
        }
        (wave_tx, music_tx)
    }

    fn audio_thread(wave_rx: Receiver<WaveMsg>, music_rx: Receiver<MusicMsg>, paused: Arc<AtomicBool>, renditions: Vec<Arc<[u8]>>) {
        // Retry forever (hero2 2026-08-07): one failed open turned into a session of silence.
        // Quick retries first (transient races), then a slow patient loop; queued clips are
        // dropped each attempt so a long retry cannot accumulate a session of them.
        let output_sink = {
            let mut attempt = 0u32;
            loop {
                match DeviceSinkBuilder::open_default_sink() {
                    Ok(s) => break s,
                    Err(e) => {
                        attempt += 1;
                        log::error!("audio: open_default_sink failed (attempt {attempt}): {e:?}");
                        while wave_rx.try_recv().is_ok() {}
                        thread::sleep(Duration::from_secs(if attempt < 5 { 1 } else { 10 }));
                    }
                }
            }
        };
        let device_rate = output_sink.config().sample_rate().get();
        let device_rate = if (8_000..=192_000).contains(&device_rate) { device_rate } else { FALLBACK_SAMPLE_RATE };
        let (sfx_tx, sfx_rx) = channel::<(u64, Vec<f32>)>();
        // The songs: decoded ahead on their own thread (a song asked for while the output was
        // still opening waits in `music_rx`), queued for the bus.
        let (chunk_tx, chunk_rx) = sync_channel::<MusicChunk>(MUSIC_QUEUE_CHUNKS);
        let shared = MusicShared::new();
        let music = MusicIn::new(chunk_rx, shared.clone(), device_rate);
        let spawned = thread::Builder::new()
            .name("wie-music".into())
            .spawn(move || music_loop(&music_rx, &chunk_tx, &shared, device_rate, &|d| open_source_with(&renditions, d)));
        if let Err(e) = spawned {
            log::error!("audio: cannot start the music thread: {e}");
        }
        output_sink.mixer().add(Bus::new(sfx_rx, music, device_rate, paused));
        log::info!("audio: output open at {device_rate} Hz, effects bus up (gain {}), music thread up", crate::audio_mix::sfx_gain());

        convert_loop(&wave_rx, &sfx_tx, device_rate);
        // `output_sink` stays alive until the channel disconnects (app teardown).
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    const RATE: u32 = 48_000;

    fn bus() -> (Bus, Sender<(u64, Vec<f32>)>, Arc<AtomicBool>) {
        let (tx, rx) = channel();
        let paused = Arc::new(AtomicBool::new(false));
        let (_, chunk_rx) = std::sync::mpsc::sync_channel(1);
        let music = MusicIn::new(chunk_rx, crate::music::MusicShared::new(), RATE);
        (Bus::new(rx, music, RATE, paused.clone()), tx, paused)
    }

    #[test]
    fn an_idle_bus_is_endless_silence() {
        let (mut bus, _tx, _) = bus();
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2 * 5).all(|s| s == 0.0));
        assert_eq!(bus.next(), Some(0.0), "never None: rodio would drop the bus");
    }

    #[test]
    fn a_clip_plays_once_through_the_limiter() {
        let (mut bus, tx, _) = bus();
        tx.send((1, vec![0.5; 2 * CHUNK_FRAMES * 2])).unwrap();
        let out: Vec<f32> = bus.by_ref().take(CHUNK_FRAMES * 2 * 3).collect();
        assert!(out[..CHUNK_FRAMES * 4].iter().all(|s| (s - 0.5).abs() < 1e-6), "the clip, unchanged below the knee");
        assert!(out[CHUNK_FRAMES * 4..].iter().all(|s| *s == 0.0), "then silence");
        assert_eq!(bus.playing(), 0);
    }

    #[test]
    fn the_sum_of_two_loud_clips_never_clips() {
        let (mut bus, tx, _) = bus();
        tx.send((1, vec![0.9; CHUNK_FRAMES * 2])).unwrap();
        tx.send((2, vec![0.9; CHUNK_FRAMES * 2])).unwrap();
        let out: Vec<f32> = bus.by_ref().take(CHUNK_FRAMES * 2).collect();
        assert!(out.iter().all(|s| *s < 1.0 && *s > 0.9), "{:?}", &out[..4]);
    }

    #[test]
    fn pausing_silences_the_bus_and_drops_what_was_playing() {
        let (mut bus, tx, paused) = bus();
        tx.send((1, vec![0.5; CHUNK_FRAMES * 2 * 10])).unwrap();
        assert_eq!(bus.next(), Some(0.5));
        paused.store(true, Ordering::Relaxed);
        let _: Vec<f32> = bus.by_ref().take(CHUNK_FRAMES * 2).collect(); // the rest of this chunk
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2).all(|s| s == 0.0));
        assert_eq!(bus.playing(), 0, "an effect does not resume seconds later");
        paused.store(false, Ordering::Relaxed);
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2).all(|s| s == 0.0));
    }

    /// Codex review 2026-10-23: the queue was left alone while paused.
    #[test]
    fn a_clip_queued_during_the_pause_does_not_play_on_return() {
        let (mut bus, tx, paused) = bus();
        paused.store(true, Ordering::Relaxed);
        tx.send((1, vec![0.5; CHUNK_FRAMES * 2])).unwrap();
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2).all(|s| s == 0.0));
        paused.store(false, Ordering::Relaxed);
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2 * 2).all(|s| s == 0.0), "no blip from nowhere");
        // …and the bus still takes new ones.
        tx.send((2, vec![0.5; CHUNK_FRAMES * 2])).unwrap();
        assert!(bus.by_ref().take(CHUNK_FRAMES * 2).any(|s| s != 0.0));
    }

    #[test]
    fn the_sink_hands_the_clip_to_the_thread_as_it_came() {
        let (tx, rx) = channel();
        let (music_tx, _music_rx) = channel();
        let sink = AndroidAudioSink::new(tx, music_tx);
        sink.play_wave(1, 8_000, &[1, -2, 3]);
        assert_eq!(rx.recv().unwrap(), (1, 8_000, vec![1, -2, 3]));
    }

    #[test]
    fn the_sink_hands_a_song_to_the_music_thread_without_copying_it() {
        let (tx, _rx) = channel();
        let (music_tx, music_rx) = channel();
        let sink = AndroidAudioSink::new(tx, music_tx);
        let song: Arc<[u8]> = Arc::from(vec![0u8; 1_000]);
        sink.media_start(3, &song, -1, 100);
        sink.media_level(3, 40);
        sink.media_stop(3);
        sink.media_close(3);
        match music_rx.recv().unwrap() {
            MusicMsg::Start { id: 3, data, loops: -1, level: 100 } => assert!(Arc::ptr_eq(&data, &song)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(music_rx.recv().unwrap(), MusicMsg::Level { id: 3, level: 40 }));
        assert!(matches!(music_rx.recv().unwrap(), MusicMsg::Stop { id: 3 }));
        assert!(matches!(music_rx.recv().unwrap(), MusicMsg::Close { id: 3 }));
    }

    #[test]
    fn a_game_effect_converts_to_device_stereo_at_the_gain() {
        // 8 kHz mono (the game's format): six output frames per input sample, both sides. A tone,
        // not a constant: the clip repairs take DC out (`sfx_low_cut`).
        let samples: Vec<i16> = (0..800).map(|n| (16_000.0 * (std::f32::consts::TAU * 500.0 * n as f32 / 8_000.0).sin()) as i16).collect();
        let (key, clip) = convert_clip(1, 8_000, &samples, RATE).unwrap();
        assert_eq!(key, sfx_key(1, 8_000, &samples));
        assert_eq!(clip.len(), (800 + 32) * 6 * 2);
        assert!(clip.chunks(2).all(|f| f[0] == f[1]), "mono plays on both sides");
        let middle = &clip[clip.len() / 4..clip.len() / 2];
        let body = middle.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((body - crate::audio_mix::SFX_GAIN * 16_000.0 / 32768.0).abs() < 0.01, "body {body}");
        assert!(convert_clip(0, 8_000, &samples, RATE).is_none());
        assert!(convert_clip(1, 0, &samples, RATE).is_none());
        assert!(convert_clip(1, 8_000, &[], RATE).is_none());
    }

    #[test]
    fn the_conversion_loop_feeds_the_bus_and_ends_with_the_sender() {
        let (wave_tx, wave_rx) = channel();
        let (sfx_tx, sfx_rx) = channel();
        wave_tx.send((1, 8_000, vec![1000i16; 80])).unwrap();
        wave_tx.send((0, 8_000, vec![1000i16; 80])).unwrap(); // refused, not forwarded
        drop(wave_tx);
        convert_loop(&wave_rx, &sfx_tx, RATE);
        assert_eq!(sfx_rx.try_iter().count(), 1);
    }
}
