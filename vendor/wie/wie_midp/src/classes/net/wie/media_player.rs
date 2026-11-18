use alloc::{format, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use java_runtime::classes::java::{io::InputStream, lang::String};
use jvm::{
    Array, ClassInstanceRef, JavaError, Jvm, Result as JvmResult,
    runtime::{JavaIoInputStream, JavaLangString},
};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::media::{
    Control,
    control::VolumeControl,
    player::{CLOSED, PREFETCHED, REALIZED, STARTED, TIME_UNKNOWN, UNREALIZED},
};

/// `audioHandle` of a player the backend does not render.
const NO_AUDIO: i32 = -1;

/// ★ heroZero fork (2026-10-21): the MIDP 2.0 `Player` — state machine, loop count and
/// `VolumeControl` — for every content type `Manager.createPlayer` accepts.
///
/// 영웅서기 Zero's sound manager `cc` creates its players with `audio/midi` (BGM, `/snd/b/N`) and
/// `audio/x-wav` (effects, `/snd/e/N`), calls `realize(); prefetch()` at load, then
/// `setLoopCount(-1 | 1); start()` to play, polls `getState() == STARTED` to decide whether the
/// music needs restarting, and sets the level through `getControl("VolumeControl")`.
///
/// What actually SOUNDS is decided by the bytes, not the declared type:
///
///   * SMAF (`MMMD`, or declared `application/vnd.smaf`) goes to the backend exactly as the old
///     `SmafPlayer` sent it — `Audio::load_smaf` / `play` / `stop` / `close` — so the WIPI clip
///     path and its music-bus handling are unchanged. `VolumeControl` is the backend's per-handle
///     level (`Audio::set_clip_volume`), live on a playing clip.
///   * ★ 2026-10-22 RIFF WAVE with PCM samples (8 or 16 bit, any channel count) is decoded to
///     16-bit on every `start()` — the bytes stay on the player (`pcm`, a few KB each in this
///     game) — scaled by the `VolumeControl` level through `wie_backend::volume_gain` (the same
///     law the SMAF PCM path uses; muted or level 0 plays nothing) and handed to the sink as one
///     clip by `Audio::play_pcm`. Fire-and-forget: `stop()` cannot cut a clip that has left
///     (all of `/snd/e/*` are under a second), and a loop count other than 1 plays ONCE — the
///     game never loops an effect. A WAV the decoder does not read (compressed, 24-bit) is silent.
///   * ★ 2026-10-22 MP4 (`ftyp`) and Standard MIDI (`MThd`) are MEDIA STREAMS: the file stays on
///     the player (`media`, a few MB for a song), and `start()` hands it to the sink's one
///     stream (`Audio::media_start` → `AudioSink::media_start`), which decodes and renders it on
///     its own thread; `stop()` pauses it there, `start()` again resumes, `close()` frees it,
///     and `VolumeControl` reaches it live. One stream at a time — a handset had one media
///     decoder — so starting a second player STOPS the first: its `getState()` drops out of
///     STARTED (`current_state`, against `Audio::media_owner`). In this game: `/snd/b/15` is a
///     Standard MIDI File, and the other `/snd/b/*` "audio/midi" files are really AAC in MP4
///     (`ftyp M4A`). Whether a stream SOUNDS is the sink's business (the default sink plays
///     nothing).
///   * Everything else is a SILENT player. The lifecycle still runs in full, and the length is
///     read from the file's own header (`Format::duration_ms`), so `getState()` leaves STARTED
///     when the sound would have ended, as on the handset.
///
/// Not honoured (documented rather than guessed):
///   * a loop count `n > 1` on the SMAF path plays ONCE — the backend loops forever or not at all;
///   * `stop()` then `start()` restarts from the beginning instead of resuming (no media time) —
///     except on a media stream, which the sink resumes; there a finite-loop player resumed
///     after `stop()` still counts its whole length from the new `start()` for `getState()`;
///   * a SMAF player's length is unknown (the backend keeps it private), so a finite-loop SMAF
///     playback counts as finished for `getState()` as soon as it starts — which keeps the
///     upstream "every `start()` plays" behaviour for effects — while `stop()` is still passed
///     to the backend whatever the state says, so the sound itself can always be cut;
///   * no `PlayerListener`, media time, rate or `getMediaTime`/`setMediaTime`.
// class net.wie.MediaPlayer
pub struct MediaPlayer;

impl MediaPlayer {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "net/wie/MediaPlayer",
            parent_class: Some("java/lang/Object"),
            interfaces: vec!["javax/microedition/media/Player"],
            methods: vec![
                JavaMethodProto::new("<init>", "(Ljava/io/InputStream;Ljava/lang/String;)V", Self::init, Default::default()),
                JavaMethodProto::new("realize", "()V", Self::realize, Default::default()),
                JavaMethodProto::new("prefetch", "()V", Self::prefetch, Default::default()),
                JavaMethodProto::new("start", "()V", Self::start, Default::default()),
                JavaMethodProto::new("stop", "()V", Self::stop, Default::default()),
                JavaMethodProto::new("deallocate", "()V", Self::deallocate, Default::default()),
                JavaMethodProto::new("close", "()V", Self::close, Default::default()),
                JavaMethodProto::new("getState", "()I", Self::get_state, Default::default()),
                JavaMethodProto::new("setLoopCount", "(I)V", Self::set_loop_count, Default::default()),
                JavaMethodProto::new("getDuration", "()J", Self::get_duration, Default::default()),
                JavaMethodProto::new("getContentType", "()Ljava/lang/String;", Self::get_content_type, Default::default()),
                JavaMethodProto::new(
                    "getControl",
                    "(Ljava/lang/String;)Ljavax/microedition/media/Control;",
                    Self::get_control,
                    Default::default(),
                ),
                JavaMethodProto::new(
                    "getControls",
                    "()[Ljavax/microedition/media/Control;",
                    Self::get_controls,
                    Default::default(),
                ),
                // wie private methods...
                // `org.kwis.msp.media.Player` (WIPI clips) plays through this; see `start_with_repeat`.
                JavaMethodProto::new("start", "(Z)V", Self::start_with_repeat, Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("state", "I", Default::default()),
                JavaFieldProto::new("loopCount", "I", Default::default()),
                JavaFieldProto::new("contentType", "Ljava/lang/String;", Default::default()),
                JavaFieldProto::new("audioHandle", "I", Default::default()),
                JavaFieldProto::new("durationMs", "J", Default::default()),
                JavaFieldProto::new("startedAt", "J", Default::default()),
                JavaFieldProto::new("level", "I", Default::default()),
                JavaFieldProto::new("muted", "Z", Default::default()),
                JavaFieldProto::new("volumeControl", "Ljavax/microedition/media/control/VolumeControl;", Default::default()),
                // ★ 2026-10-22: the WAV file, kept for `start()` to decode; null for every other format.
                JavaFieldProto::new("pcm", "[B", Default::default()),
                // ★ 2026-10-22: an MP4 or MIDI file, kept for `start()` to hand to the sink's media
                // stream, and the stream's id (`0` for every other format).
                JavaFieldProto::new("media", "[B", Default::default()),
                JavaFieldProto::new("mediaId", "I", Default::default()),
            ],
            access_flags: Default::default(),
        }
    }

    async fn init(
        jvm: &Jvm,
        context: &mut WieJvmContext,
        mut this: ClassInstanceRef<Self>,
        stream: ClassInstanceRef<InputStream>,
        r#type: ClassInstanceRef<String>,
    ) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::<init>({this:?}, {stream:?}, {type:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        let data = JavaIoInputStream::read_until_end(jvm, &stream).await?;
        let format = Format::sniff(&data);
        let declared = if r#type.is_null() {
            None
        } else {
            Some(JavaLangString::to_rust_string(jvm, &r#type).await?)
        };
        let is_smaf = format == Format::Smaf || declared.as_deref().is_some_and(|x| x.eq_ignore_ascii_case("application/vnd.smaf"));

        let audio_handle = if is_smaf {
            // A statement of its own, so the audio lock is released before anything awaits.
            let loaded = context.system().audio().load_smaf(&data);
            match loaded {
                Ok(handle) => handle as i32,
                Err(err) => {
                    return Err(jvm
                        .exception("javax/microedition/media/MediaException", &format!("cannot load SMAF: {err:?}"))
                        .await);
                }
            }
        } else {
            NO_AUDIO
        };
        let duration_ms = if is_smaf { None } else { format.duration_ms(&data) };

        // ★ 2026-10-22: a WAV keeps its bytes on the player and plays through `Audio::play_pcm`;
        // an MP4 or MIDI file keeps them for the sink's media stream (`Audio::media_start`).
        let is_pcm = audio_handle == NO_AUDIO && format == Format::Wav && wav_pcm(&data).is_some();
        let is_media = audio_handle == NO_AUDIO && matches!(format, Format::Mp4 | Format::Midi);
        if is_pcm || is_media {
            let mut array = jvm.instantiate_array("B", data.len()).await?;
            jvm.array_raw_buffer_mut(&mut array).await?.write(0, &data)?;
            jvm.put_field(&mut this, if is_pcm { "pcm" } else { "media" }, "[B", array).await?;
        }
        if is_media {
            static NEXT_MEDIA_ID: AtomicU32 = AtomicU32::new(1);
            let id = NEXT_MEDIA_ID.fetch_add(1, Ordering::Relaxed);
            jvm.put_field(&mut this, "mediaId", "I", id as i32).await?;
        } else if !is_pcm && audio_handle == NO_AUDIO {
            static SILENT_NOTED: AtomicBool = AtomicBool::new(false);
            if !SILENT_NOTED.swap(true, Ordering::Relaxed) {
                tracing::info!(
                    "net.wie.MediaPlayer: only SMAF, PCM WAV and media streams (MP4, MIDI) are played by this backend; other players run their lifecycle silently"
                );
            }
        }
        tracing::debug!(
            "net.wie.MediaPlayer: {declared:?} sniffed as {format:?}, {} bytes, {duration_ms:?} ms, {}",
            data.len(),
            if is_pcm {
                "pcm"
            } else if is_media {
                "media"
            } else if audio_handle == NO_AUDIO {
                "silent"
            } else {
                "backend"
            }
        );

        let content_type = if r#type.is_null() {
            JavaLangString::from_rust_string(jvm, format.content_type()).await?.into()
        } else {
            r#type
        };

        jvm.put_field(&mut this, "state", "I", UNREALIZED).await?;
        jvm.put_field(&mut this, "loopCount", "I", 1).await?;
        jvm.put_field(&mut this, "contentType", "Ljava/lang/String;", content_type).await?;
        jvm.put_field(&mut this, "audioHandle", "I", audio_handle).await?;
        jvm.put_field(&mut this, "durationMs", "J", duration_ms.map_or(-1, |x| x as i64)).await?;
        jvm.put_field(&mut this, "level", "I", 100).await?;
        jvm.put_field(&mut this, "muted", "Z", false).await?;

        Ok(())
    }

    async fn realize(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::realize({this:?})");

        match Self::current_state(jvm, context, &mut this).await? {
            CLOSED => Err(illegal_state(jvm, "realize() on a closed Player").await),
            UNREALIZED => jvm.put_field(&mut this, "state", "I", REALIZED).await,
            _ => Ok(()),
        }
    }

    async fn prefetch(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::prefetch({this:?})");

        match Self::current_state(jvm, context, &mut this).await? {
            CLOSED => Err(illegal_state(jvm, "prefetch() on a closed Player").await),
            UNREALIZED | REALIZED => jvm.put_field(&mut this, "state", "I", PREFETCHED).await,
            _ => Ok(()),
        }
    }

    /// MIDP: realizes and prefetches first if needed; a no-op while already STARTED.
    async fn start(jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::start({this:?})");

        Self::start_playback(jvm, context, this, false).await
    }

    /// The old `SmafPlayer.start(Z)V`, kept for `org.kwis.msp.media.Player`: loop forever or play
    /// once, and (re)issue the playback even if one is running — upstream's semantics, untouched.
    async fn start_with_repeat(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>, repeat: bool) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::start({this:?}, {repeat})");

        if Self::current_state(jvm, context, &mut this).await? == CLOSED {
            return Err(illegal_state(jvm, "start() on a closed Player").await);
        }
        jvm.put_field(&mut this, "loopCount", "I", if repeat { -1 } else { 1 }).await?;

        Self::start_playback(jvm, context, this, true).await
    }

    async fn start_playback(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>, restart: bool) -> JvmResult<()> {
        let state = Self::current_state(jvm, context, &mut this).await?;
        if state == CLOSED {
            return Err(illegal_state(jvm, "start() on a closed Player").await);
        }
        if state == STARTED && !restart {
            return Ok(());
        }

        let now = context.system().platform().now().raw() as i64;
        jvm.put_field(&mut this, "startedAt", "J", now).await?;
        jvm.put_field(&mut this, "state", "I", STARTED).await?;

        let audio_handle: i32 = jvm.get_field(&this, "audioHandle", "I").await?;
        if audio_handle != NO_AUDIO {
            let loop_count: i32 = jvm.get_field(&this, "loopCount", "I").await?;
            if loop_count > 1 {
                tracing::debug!("net.wie.MediaPlayer: loop count {loop_count} plays once on the SMAF backend");
            }

            let system = context.system();
            let played = system.audio().play(system, audio_handle as u32, loop_count < 0);
            if let Err(err) = played {
                return Err(jvm
                    .exception("javax/microedition/media/MediaException", &format!("cannot play: {err:?}"))
                    .await);
            }
        } else if !Self::start_media(jvm, context, &this).await? {
            Self::start_pcm(jvm, context, &this).await?;
        }

        Ok(())
    }

    /// ★ 2026-10-22: the media path of `start_playback` — the kept file to the sink's one
    /// stream, which resumes it if this player stopped it and nothing took the decoder since.
    /// `false` for a player that is not a media stream.
    async fn start_media(jvm: &Jvm, context: &mut WieJvmContext, this: &ClassInstanceRef<Self>) -> JvmResult<bool> {
        let media: ClassInstanceRef<Array<i8>> = jvm.get_field(this, "media", "[B").await?;
        if media.is_null() {
            return Ok(false);
        }
        let id: i32 = jvm.get_field(this, "mediaId", "I").await?;
        let loop_count: i32 = jvm.get_field(this, "loopCount", "I").await?;
        let level = Self::effective_level(jvm, this).await?;

        // One copy, straight into the shared buffer the sink keeps while the stream plays.
        let length = jvm.array_length(&media).await?;
        let mut data: Arc<[u8]> = core::iter::repeat_n(0u8, length).collect();
        jvm.array_raw_buffer(&media).await?.read(0, Arc::get_mut(&mut data).unwrap())?;
        tracing::debug!("net.wie.MediaPlayer: media stream {id}, {length} bytes, loop count {loop_count}, level {level}");

        // A statement of its own, so the audio lock is released before anything awaits.
        context.system().audio().media_start(id as u32, &data, loop_count, level);

        Ok(true)
    }

    /// The `VolumeControl` level with muting applied, `0..=100`.
    async fn effective_level(jvm: &Jvm, this: &ClassInstanceRef<Self>) -> JvmResult<u8> {
        let level: i32 = jvm.get_field(this, "level", "I").await?;
        let muted: bool = jvm.get_field(this, "muted", "Z").await?;

        Ok(if muted { 0 } else { level.clamp(0, 100) as u8 })
    }

    /// ★ 2026-10-22: the WAV path of `start_playback` — decode the kept file, apply the level,
    /// hand the clip to the sink. A player without `pcm` (every non-WAV format) stays silent.
    async fn start_pcm(jvm: &Jvm, context: &mut WieJvmContext, this: &ClassInstanceRef<Self>) -> JvmResult<()> {
        let pcm: ClassInstanceRef<Array<i8>> = jvm.get_field(this, "pcm", "[B").await?;
        if pcm.is_null() {
            return Ok(());
        }
        let level: i32 = jvm.get_field(this, "level", "I").await?;
        let muted: bool = jvm.get_field(this, "muted", "Z").await?;
        if muted || level <= 0 {
            return Ok(());
        }
        let loop_count: i32 = jvm.get_field(this, "loopCount", "I").await?;
        if loop_count != 1 {
            tracing::debug!("net.wie.MediaPlayer: loop count {loop_count} on a WAV plays once");
        }

        let length = jvm.array_length(&pcm).await?;
        let bytes: Vec<i8> = jvm.load_array(&pcm, 0, length).await?;
        let data: Vec<u8> = bytes.into_iter().map(|b| b as u8).collect();
        let Some((channels, sampling_rate, mut samples)) = wav_pcm(&data) else {
            return Ok(());
        };
        let gain = wie_backend::volume_gain(level.clamp(0, 100) as u8);
        if gain < 1.0 {
            for s in samples.iter_mut() {
                *s = (*s as f32 * gain) as i16;
            }
        }
        tracing::debug!("net.wie.MediaPlayer: pcm {channels} ch {sampling_rate} Hz {} samples, level {level}", samples.len());

        // A statement of its own, so the audio lock is released before anything awaits.
        context.system().audio().play_pcm(channels, sampling_rate, &samples);

        Ok(())
    }

    /// MIDP: STARTED -> PREFETCHED, ignored in any other live state. A backend playback is told
    /// to stop regardless — see the type docs for why the state alone cannot be trusted there.
    async fn stop(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::stop({this:?})");

        let state = Self::current_state(jvm, context, &mut this).await?;
        if state == CLOSED {
            return Err(illegal_state(jvm, "stop() on a closed Player").await);
        }

        let audio_handle: i32 = jvm.get_field(&this, "audioHandle", "I").await?;
        if audio_handle != NO_AUDIO {
            let system = context.system();
            let now_ms = system.platform().now().raw();
            system.audio().stop(now_ms, audio_handle as u32);
        }
        let media_id: i32 = jvm.get_field(&this, "mediaId", "I").await?;
        if media_id != 0 {
            context.system().audio().media_stop(media_id as u32);
        }

        if state == STARTED {
            jvm.put_field(&mut this, "state", "I", PREFETCHED).await?;
        }

        Ok(())
    }

    /// MIDP: stops if STARTED, then PREFETCHED -> REALIZED. The backend keeps the loaded file.
    async fn deallocate(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::deallocate({this:?})");

        match Self::current_state(jvm, context, &mut this).await? {
            CLOSED => Err(illegal_state(jvm, "deallocate() on a closed Player").await),
            STARTED => {
                Self::stop(jvm, context, this.clone()).await?;
                jvm.put_field(&mut this, "state", "I", REALIZED).await
            }
            PREFETCHED => jvm.put_field(&mut this, "state", "I", REALIZED).await,
            _ => Ok(()),
        }
    }

    /// MIDP: releases everything; closing a closed Player is a no-op. Never closes a backend
    /// handle twice (`Audio::close` errs on an unknown handle, which the old code unwrapped).
    async fn close(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::close({this:?})");

        let state: i32 = jvm.get_field(&this, "state", "I").await?;
        if state == CLOSED {
            return Ok(());
        }

        let audio_handle: i32 = jvm.get_field(&this, "audioHandle", "I").await?;
        if audio_handle != NO_AUDIO {
            let closed = context.system().audio().close(audio_handle as u32);
            if let Err(err) = closed {
                tracing::debug!("net.wie.MediaPlayer: backend close of handle {audio_handle}: {err:?}");
            }
        }
        let media_id: i32 = jvm.get_field(&this, "mediaId", "I").await?;
        if media_id != 0 {
            context.system().audio().media_close(media_id as u32);
            jvm.put_field(&mut this, "media", "[B", None).await?;
        }

        jvm.put_field(&mut this, "audioHandle", "I", NO_AUDIO).await?;
        jvm.put_field(&mut this, "state", "I", CLOSED).await?;

        Ok(())
    }

    async fn get_state(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<i32> {
        tracing::debug!("net.wie.MediaPlayer::getState({this:?})");

        Self::current_state(jvm, context, &mut this).await
    }

    /// MIDP: `-1` loops until stopped, `n > 0` plays n times in all, `0` is an
    /// IllegalArgumentException; not allowed while STARTED or CLOSED. Any other negative count
    /// is taken as "forever".
    async fn set_loop_count(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>, count: i32) -> JvmResult<()> {
        tracing::debug!("net.wie.MediaPlayer::setLoopCount({this:?}, {count})");

        if count == 0 {
            return Err(jvm.exception("java/lang/IllegalArgumentException", "loop count 0").await);
        }

        match Self::current_state(jvm, context, &mut this).await? {
            CLOSED => Err(illegal_state(jvm, "setLoopCount() on a closed Player").await),
            STARTED => Err(illegal_state(jvm, "setLoopCount() on a started Player").await),
            _ => jvm.put_field(&mut this, "loopCount", "I", count).await,
        }
    }

    /// One pass, in microseconds, or `TIME_UNKNOWN`.
    async fn get_duration(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<i64> {
        tracing::debug!("net.wie.MediaPlayer::getDuration({this:?})");

        if Self::current_state(jvm, context, &mut this).await? == CLOSED {
            return Err(illegal_state(jvm, "getDuration() on a closed Player").await);
        }

        let duration_ms: i64 = jvm.get_field(&this, "durationMs", "J").await?;

        Ok(if duration_ms < 0 { TIME_UNKNOWN } else { duration_ms * 1000 })
    }

    async fn get_content_type(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<ClassInstanceRef<String>> {
        tracing::debug!("net.wie.MediaPlayer::getContentType({this:?})");

        if matches!(Self::current_state(jvm, context, &mut this).await?, UNREALIZED | CLOSED) {
            return Err(illegal_state(jvm, "getContentType() on an unrealized or closed Player").await);
        }

        jvm.get_field(&this, "contentType", "Ljava/lang/String;").await
    }

    /// MMAPI: `VolumeControl` (bare or fully qualified — a bare name means the
    /// `javax.microedition.media.control` package) is the one control a player has; any other
    /// name is `null`.
    async fn get_control(
        jvm: &Jvm,
        context: &mut WieJvmContext,
        mut this: ClassInstanceRef<Self>,
        control_type: ClassInstanceRef<String>,
    ) -> JvmResult<ClassInstanceRef<Control>> {
        tracing::debug!("net.wie.MediaPlayer::getControl({this:?}, {control_type:?})");

        if control_type.is_null() {
            return Err(jvm.exception("java/lang/IllegalArgumentException", "control type is null").await);
        }
        if matches!(Self::current_state(jvm, context, &mut this).await?, UNREALIZED | CLOSED) {
            return Err(illegal_state(jvm, "getControl() on an unrealized or closed Player").await);
        }

        let control_type = JavaLangString::to_rust_string(jvm, &control_type).await?;
        if control_type != "VolumeControl" && control_type != "javax.microedition.media.control.VolumeControl" {
            return Ok(None.into());
        }

        let control = Self::volume_control(jvm, &mut this).await?;

        Ok(ClassInstanceRef::new(control.instance))
    }

    async fn get_controls(jvm: &Jvm, context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<ClassInstanceRef<Array<Control>>> {
        tracing::debug!("net.wie.MediaPlayer::getControls({this:?})");

        if matches!(Self::current_state(jvm, context, &mut this).await?, UNREALIZED | CLOSED) {
            return Err(illegal_state(jvm, "getControls() on an unrealized or closed Player").await);
        }

        let control = Self::volume_control(jvm, &mut this).await?;
        let mut controls = jvm.instantiate_array("Ljavax/microedition/media/Control;", 1).await?;
        jvm.store_array(&mut controls, 0, vec![control]).await?;

        Ok(controls.into())
    }

    /// The player's one `VolumeControl`, created on first request and the same object after.
    async fn volume_control(jvm: &Jvm, this: &mut ClassInstanceRef<Self>) -> JvmResult<ClassInstanceRef<VolumeControl>> {
        let control: ClassInstanceRef<VolumeControl> = jvm
            .get_field(this, "volumeControl", "Ljavax/microedition/media/control/VolumeControl;")
            .await?;
        if !control.is_null() {
            return Ok(control);
        }

        let control: ClassInstanceRef<VolumeControl> = jvm
            .new_class("net/wie/PlayerVolumeControl", "(Lnet/wie/MediaPlayer;)V", (this.clone(),))
            .await?
            .into();
        jvm.put_field(this, "volumeControl", "Ljavax/microedition/media/control/VolumeControl;", control.clone())
            .await?;

        Ok(control)
    }

    /// `VolumeControl.setLevel`: clamps to 0..=100, returns what was set, and pushes the level
    /// to the backend handle (if any) — live, on a clip that is already playing.
    pub async fn set_level(jvm: &Jvm, context: &mut WieJvmContext, this: &mut ClassInstanceRef<Self>, level: i32) -> JvmResult<i32> {
        let level = level.clamp(0, 100);
        jvm.put_field(this, "level", "I", level).await?;
        Self::apply_volume(jvm, context, this).await?;

        Ok(level)
    }

    pub async fn level(jvm: &Jvm, this: &ClassInstanceRef<Self>) -> JvmResult<i32> {
        jvm.get_field(this, "level", "I").await
    }

    /// `VolumeControl.setMute`: silences without forgetting the level.
    pub async fn set_mute(jvm: &Jvm, context: &mut WieJvmContext, this: &mut ClassInstanceRef<Self>, muted: bool) -> JvmResult<()> {
        jvm.put_field(this, "muted", "Z", muted).await?;

        Self::apply_volume(jvm, context, this).await
    }

    pub async fn is_muted(jvm: &Jvm, this: &ClassInstanceRef<Self>) -> JvmResult<bool> {
        jvm.get_field(this, "muted", "Z").await
    }

    /// The backend's per-handle level is the same cell `MC_mdaClipSetVolume` drives, read by the
    /// running playback on every event and mapped through `wie_backend::volume_gain`. ★ A media
    /// stream gets it through `Audio::media_level` (2026-10-22). A silent player, and a WAV (whose
    /// level is read at each `start()`), only keep the numbers.
    async fn apply_volume(jvm: &Jvm, context: &mut WieJvmContext, this: &ClassInstanceRef<Self>) -> JvmResult<()> {
        let audio_handle: i32 = jvm.get_field(this, "audioHandle", "I").await?;
        let media_id: i32 = jvm.get_field(this, "mediaId", "I").await?;
        if audio_handle == NO_AUDIO && media_id == 0 {
            return Ok(());
        }

        let effective = Self::effective_level(jvm, this).await?;
        if audio_handle != NO_AUDIO {
            context.system().audio().set_clip_volume(audio_handle as u32, effective);
        }
        if media_id != 0 {
            context.system().audio().media_level(media_id as u32, effective);
        }

        Ok(())
    }

    /// The state with end-of-media applied: a finite-loop playback whose last pass is over is
    /// PREFETCHED again (MIDP's END_OF_MEDIA). Checked lazily, on each call, against the wall
    /// clock — no timer, nothing for the backend to call back into. An unknown length counts as
    /// over at once (see the type docs). ★ A media stream another player took the decoder from
    /// is stopped too (2026-10-22).
    async fn current_state(jvm: &Jvm, context: &mut WieJvmContext, this: &mut ClassInstanceRef<Self>) -> JvmResult<i32> {
        let state: i32 = jvm.get_field(this, "state", "I").await?;
        if state != STARTED {
            return Ok(state);
        }

        let media_id: i32 = jvm.get_field(this, "mediaId", "I").await?;
        if media_id != 0 && context.system().audio().media_owner() != Some(media_id as u32) {
            jvm.put_field(this, "state", "I", PREFETCHED).await?;
            return Ok(PREFETCHED);
        }

        let loop_count: i32 = jvm.get_field(this, "loopCount", "I").await?;
        if loop_count < 0 {
            return Ok(STARTED);
        }

        let duration_ms: i64 = jvm.get_field(this, "durationMs", "J").await?;
        let ended = if duration_ms < 0 {
            true
        } else {
            let started_at: i64 = jvm.get_field(this, "startedAt", "J").await?;
            let now = context.system().platform().now().raw() as i64;
            now.saturating_sub(started_at) >= duration_ms.saturating_mul(loop_count as i64)
        };

        if ended {
            jvm.put_field(this, "state", "I", PREFETCHED).await?;
            return Ok(PREFETCHED);
        }

        Ok(STARTED)
    }
}

/// `java/lang/IllegalStateException`, which MIDP's Player throws — or its parent
/// `RuntimeException` while `java_runtime` does not define it (it does not, as of 2026-10-21).
/// `Jvm::exception` unwraps the constructor, so throwing a class that does not exist would panic
/// the emulator; the parent is still caught by every `catch (Exception)` in the game. Looked up
/// once per process: a failed lookup logs "No such class" at error level.
async fn illegal_state(jvm: &Jvm, message: &str) -> JavaError {
    const CLASS: &str = "java/lang/IllegalStateException";
    // 0 = not looked up yet, 1 = present, 2 = absent.
    static PRESENT: AtomicU8 = AtomicU8::new(0);

    let present = match PRESENT.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let present = jvm.has_class(CLASS) || jvm.resolve_class(CLASS).await.is_ok();
            PRESENT.store(if present { 1 } else { 2 }, Ordering::Relaxed);
            present
        }
    };

    jvm.exception(if present { CLASS } else { "java/lang/RuntimeException" }, message).await
}

/// What the bytes handed to `Manager.createPlayer` are, whatever type they were declared as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Smaf,
    Wav,
    Midi,
    Mp4,
    Unknown,
}

impl Format {
    fn sniff(data: &[u8]) -> Self {
        match data {
            [b'M', b'M', b'M', b'D', ..] => Self::Smaf,
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => Self::Wav,
            [b'M', b'T', b'h', b'd', ..] => Self::Midi,
            [_, _, _, _, b'f', b't', b'y', b'p', ..] => Self::Mp4,
            _ => Self::Unknown,
        }
    }

    /// `getContentType` for a player created with a null type.
    fn content_type(self) -> &'static str {
        match self {
            Self::Smaf => "application/vnd.smaf",
            Self::Wav => "audio/x-wav",
            Self::Midi => "audio/midi",
            Self::Mp4 => "audio/mp4",
            Self::Unknown => "application/octet-stream",
        }
    }

    /// One pass of the file, in ms, from its own header — `None` if it cannot be told.
    fn duration_ms(self, data: &[u8]) -> Option<u64> {
        match self {
            Self::Wav => wav_duration_ms(data),
            Self::Midi => midi_duration_ms(data),
            Self::Mp4 => mp4_duration_ms(data),
            Self::Smaf | Self::Unknown => None,
        }
    }
}

fn le_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn be_u64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// ★ 2026-10-22: RIFF WAVE with PCM samples (format tag 1), 8-bit unsigned or 16-bit signed
/// little-endian, as `(channels, sampling rate, interleaved 16-bit samples)`. Anything else —
/// compressed, 24/32-bit, no `fmt ` before `data` — is `None`. A `data` chunk that claims more
/// than the file holds counts only what is there, as [`wav_duration_ms`] does.
fn wav_pcm(data: &[u8]) -> Option<(u8, u32, Vec<i16>)> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut fmt = None;
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let size = le_u32(data, pos + 4)? as usize;
        let body = pos + 8;
        match &data[pos..pos + 4] {
            b"fmt " => {
                let tag = u16::from_le_bytes(data.get(body..body + 2)?.try_into().ok()?);
                let channels = u16::from_le_bytes(data.get(body + 2..body + 4)?.try_into().ok()?);
                let rate = le_u32(data, body + 4)?;
                let bits = u16::from_le_bytes(data.get(body + 14..body + 16)?.try_into().ok()?);
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = fmt?;
                if tag != 1 || channels == 0 || channels > 255 || rate == 0 {
                    return None;
                }
                let bytes = &data[body..body + size.min(data.len() - body)];
                let samples: Vec<i16> = match bits {
                    8 => bytes.iter().map(|&b| ((b as i16) - 128) << 8).collect(),
                    16 => bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
                    _ => return None,
                };
                return Some((channels as u8, rate, samples));
            }
            _ => {}
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }

    None
}

/// RIFF WAVE: the `data` chunk's length over the `fmt ` chunk's byte rate. A chunk that claims
/// more than the file holds (streamed or truncated) counts only what is there.
fn wav_duration_ms(data: &[u8]) -> Option<u64> {
    let mut byte_rate = None;
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let size = le_u32(data, pos + 4)? as usize;
        let body = pos + 8;
        match &data[pos..pos + 4] {
            b"fmt " => byte_rate = le_u32(data, body + 8).map(u64::from),
            b"data" => {
                let rate = byte_rate.filter(|&rate| rate > 0)?;
                return Some(size.min(data.len() - body) as u64 * 1000 / rate);
            }
            _ => {}
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }

    None
}

/// Standard MIDI File: the last event of the longest track, through the tempo map (the default
/// 120 bpm until the first Set Tempo), or through the SMPTE rate when the division is one.
fn midi_duration_ms(data: &[u8]) -> Option<u64> {
    let header_len = be_u32(data, 4)? as usize;
    let tracks = be_u16(data, 10)?;
    let division = be_u16(data, 12)?;

    let mut tempo_changes = Vec::new();
    let mut end_tick = 0u64;
    let mut pos = 8usize.checked_add(header_len)?;
    let mut seen = 0;
    while seen < tracks && pos + 8 <= data.len() {
        let len = be_u32(data, pos + 4)? as usize;
        let body = pos + 8;
        if &data[pos..pos + 4] == b"MTrk" {
            seen += 1;
            let end = body.saturating_add(len).min(data.len());
            end_tick = end_tick.max(midi_track_end(&data[body..end], &mut tempo_changes));
        }
        pos = body.saturating_add(len);
    }

    if division & 0x8000 != 0 {
        // SMPTE: frames per second (negated, high byte) x ticks per frame (low byte).
        let fps = -((division >> 8) as u8 as i8 as i64);
        let ticks_per_second = fps.max(0) as u64 * (division & 0xff) as u64;
        return (ticks_per_second > 0).then(|| end_tick * 1000 / ticks_per_second);
    }

    let ticks_per_quarter = division as u128;
    if ticks_per_quarter == 0 {
        return None;
    }

    tempo_changes.sort_by_key(|&(tick, _)| tick);
    // Sum of ticks x microseconds-per-quarter, divided once at the end.
    let mut total = 0u128;
    let (mut tick, mut tempo) = (0u64, 500_000u64);
    for (at, new_tempo) in tempo_changes {
        if at >= end_tick {
            break;
        }
        total += (at - tick) as u128 * tempo as u128;
        (tick, tempo) = (at, new_tempo);
    }
    total += (end_tick - tick) as u128 * tempo as u128;

    Some((total / ticks_per_quarter / 1000) as u64)
}

/// The tick of a track's last event (its End of Track), collecting its Set Tempo events.
/// Running status is kept across meta and SysEx events: data bytes there can only mean that.
fn midi_track_end(track: &[u8], tempo_changes: &mut Vec<(u64, u64)>) -> u64 {
    fn read_vlq(data: &[u8], pos: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..4 {
            let byte = *data.get(*pos)?;
            *pos += 1;
            value = (value << 7) | (byte & 0x7f) as u64;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    let (mut pos, mut tick, mut running) = (0usize, 0u64, 0u8);
    while pos < track.len() {
        let Some(delta) = read_vlq(track, &mut pos) else { break };
        tick += delta;
        let Some(&status) = track.get(pos) else { break };

        match status {
            0xff => {
                let Some(&kind) = track.get(pos + 1) else { break };
                pos += 2;
                let Some(len) = read_vlq(track, &mut pos) else { break };
                if kind == 0x51
                    && len == 3
                    && let Some(t) = track.get(pos..pos + 3)
                {
                    tempo_changes.push((tick, u32::from_be_bytes([0, t[0], t[1], t[2]]) as u64));
                }
                if kind == 0x2f {
                    break;
                }
                pos = pos.saturating_add(len as usize);
            }
            0xf0 | 0xf7 => {
                pos += 1;
                let Some(len) = read_vlq(track, &mut pos) else { break };
                pos = pos.saturating_add(len as usize);
            }
            _ => {
                let status = if status & 0x80 != 0 {
                    pos += 1;
                    running = status;
                    status
                } else if running != 0 {
                    running
                } else {
                    break;
                };
                pos += if matches!(status & 0xf0, 0xc0 | 0xd0) { 1 } else { 2 };
            }
        }
    }

    tick
}

/// MP4 / M4A: `moov/mvhd` duration over its timescale.
fn mp4_duration_ms(data: &[u8]) -> Option<u64> {
    fn find_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
        let mut pos = 0usize;
        while pos + 8 <= data.len() {
            let (header, size) = match be_u32(data, pos)? {
                0 => (8, data.len() - pos),
                1 => (16, usize::try_from(be_u64(data, pos + 8)?).ok()?),
                size => (8, size as usize),
            };
            if size < header {
                return None;
            }
            let end = pos.saturating_add(size).min(data.len());
            if &data[pos + 4..pos + 8] == kind {
                return data.get(pos + header..end);
            }
            pos = end;
        }
        None
    }

    let moov = find_box(data, b"moov")?;
    let mvhd = find_box(moov, b"mvhd")?;
    let (timescale, duration) = if *mvhd.first()? == 1 {
        (be_u32(mvhd, 20)?, be_u64(mvhd, 24)?)
    } else {
        (be_u32(mvhd, 12)?, be_u32(mvhd, 16)? as u64)
    };

    (timescale > 0).then(|| (duration as u128 * 1000 / timescale as u128) as u64)
}

#[cfg(test)]
mod tests {
    use super::{Format, wav_pcm};
    use alloc::vec::Vec;

    /// The game's effects: `fmt ` PCM, mono, 8 kHz, 8-bit unsigned, then `data`.
    fn wav8(samples: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + samples.len()) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&8000u32.to_le_bytes()); // byte rate
        v.extend_from_slice(&1u16.to_le_bytes()); // block align
        v.extend_from_slice(&8u16.to_le_bytes()); // bits
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        v.extend_from_slice(samples);
        v
    }

    #[test]
    fn eight_bit_mono_wav_decodes_to_centred_i16() {
        let file = wav8(&[0x80, 0xff, 0x00, 0x7f]);
        assert_eq!(Format::sniff(&file), Format::Wav);
        let (channels, rate, samples) = wav_pcm(&file).unwrap();
        assert_eq!((channels, rate), (1, 8000));
        assert_eq!(samples, alloc::vec![0, 127 << 8, -128 << 8, -1 << 8]);
    }

    #[test]
    fn a_data_chunk_longer_than_the_file_counts_what_is_there() {
        let mut file = wav8(&[0x80; 10]);
        let data_len = file.len() - 4 - 10;
        file[data_len..data_len + 4].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(wav_pcm(&file).unwrap().2.len(), 10);
    }

    #[test]
    fn compressed_or_wide_samples_are_refused() {
        let mut file = wav8(&[0x80; 4]);
        file[20] = 2; // ADPCM
        assert!(wav_pcm(&file).is_none());
        let mut file = wav8(&[0x80; 4]);
        file[34] = 24;
        assert!(wav_pcm(&file).is_none());
        assert!(wav_pcm(b"RIFF    WAVE").is_none());
        assert!(wav_pcm(b"MThd").is_none());
    }
}
