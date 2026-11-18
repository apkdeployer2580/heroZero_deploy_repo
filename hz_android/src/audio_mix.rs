//! Master-bus math for the effect path. Pure arithmetic, no dependencies, host-tested.
//!
//! Brought over from hero2NativeAPP's `audio_mix.rs` (2026-10-22, Phase 4a: WAV effects) with the
//! music-bus constants left behind — this game's music (AAC, one MIDI) is not rendered yet, so for
//! now the bus carries effects alone. What stays is what the device needs whatever plays:
//!
//!   * [`soft_limit`] — the SUM is limited before it reaches AAudio, which hard-clamps. Two
//!     effects landing together (a hit and its crit) must not clip into each other.
//!   * [`SfxClips`] — effects play CONCURRENTLY (the phone had one player; a rapid burst used to
//!     cut itself off), but one effect at most [`SFX_SAME_MAX`] times at once, so a per-tile
//!     effect that restarts every frame does not pile up sixteen deep (hero2: 지옥불).
//!   * [`SFX_GAIN`] — where the effects sit below full scale. The game's own level
//!     (`VolumeControl.setLevel`, applied in `net.wie.MediaPlayer`) is already in the samples.

/// The effects' place below full scale, applied once per clip on the audio thread.
///
/// ★ 0.7 → 0.6 with the music (Phase 4c, 2026-10-22). Measured on the game's own files: the
/// fourteen effects peak at 0 dBFS (eight of them) and sit at −6 to −19 dBFS RMS, median −12.5;
/// the sixteen songs peak at −1.3 to 0 dBFS and sit at −13 to −21 dBFS RMS, median −15. hero2's
/// player, listening to both buses, settled on the effects ≈ +2.6 dB over the music ("효과음이
/// 상대적으로 좀 크게 들려서 … 아주 조금만 낮추는게 좋을것같아" at +4.6 dB); equal trims keep
/// this game's own gap, which is that. And 0.6 keeps a full-scale hit under the limiter's knee
/// alone. The knob to turn by ear, with [`MUSIC_GAIN`].
pub const SFX_GAIN: f32 = 0.6;

/// The player's effects level over [`SFX_GAIN`] (settings › SFX LEVEL, 2026-10-25 — hero2's five
/// steps): what the gear's row cycles through, in dB.
pub const SFX_STEPS_DB: [i32; 5] = [-6, -3, 0, 3, 6];

/// The level now, in dB (the UI thread sets it, the audio thread reads it once per clip).
static SFX_DB: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub fn set_sfx_db(db: i32) {
    SFX_DB.store(db.clamp(-6, 6), std::sync::atomic::Ordering::Relaxed);
}

pub fn sfx_db() -> i32 {
    SFX_DB.load(std::sync::atomic::Ordering::Relaxed)
}

/// [`SFX_GAIN`] with the player's level on it — the gain a new clip is converted at. +6 dB on a
/// full-scale hit goes into [`soft_limit`]'s knee, which is what the limiter is for.
pub fn sfx_gain() -> f32 {
    SFX_GAIN * 10f32.powf(sfx_db() as f32 / 20.0)
}

/// The step after `db` in [`SFX_STEPS_DB`], round again after the loudest.
pub fn next_sfx_db(db: i32) -> i32 {
    let i = SFX_STEPS_DB.iter().position(|&s| s >= db).unwrap_or(0);
    SFX_STEPS_DB[(i + 1) % SFX_STEPS_DB.len()]
}

/// What the settings row says for `db` (a `crate::host_text` line): its step, 0 dB being hero2's
/// 「기본」 — what every build before the row sounded like.
pub fn sfx_db_word(db: i32) -> &'static str {
    match db {
        ..=-6 => "-6 DB",
        -5..=-3 => "-3 DB",
        -2..=2 => "DEFAULT",
        3..=5 => "+3 DB",
        _ => "+6 DB",
    }
}

/// The music's place below full scale (Phase 4c), applied on the bus with the game's
/// `VolumeControl` level through `wie_backend::volume_gain`. See [`SFX_GAIN`] for the numbers:
/// the same trim, so the songs' peaks stay under the limiter's knee and the effects keep the
/// ~2.5 dB the game authored them above the music.
pub const MUSIC_GAIN: f32 = 0.6;

/// How long the music takes to come in or go out — a new song, a stop, the app going to the
/// background. Cutting a song in one sample clicks; 10 ms is too short to hear as a fade.
pub const MUSIC_FADE_MS: u32 = 10;

/// How long a song the game stopped plays on before it ramps out — a plain MIDP `stop()`, as the
/// game's settings turn the music off (that takes this long to go quiet), and the moment between
/// the `stop()` and the `close()` of a door. The song's place is kept past it: a restart after the
/// grace ramps in from there, not from the top (user, 2026-10-23).
pub const MUSIC_STOP_GRACE_MS: u32 = 300;

/// How long a song whose Player the game CLOSED plays on: the game closes its song only on the way
/// to the next one — a door (`rpg/e.g`, "go to map": `stop(); close()`, the map load, a NEW Player
/// for the next map's song, which in a building is the same file) and the fanfare (`ek.C`). The
/// load is what this covers: door to next `start()` measured 0.8–2.3 s on the S25 and 1.8 s for
/// one small room on the S9 (2026-10-23, `logcat -s hz`), never within [`MUSIC_STOP_GRACE_MS`].
/// So the same song goes on through the load unbroken; another song takes over from the old one
/// at its `start()` instead of after a silence.
pub const MUSIC_CLOSE_GRACE_MS: u32 = 4_000;

/// Where [`soft_limit`] stops being a straight wire, in linear amplitude (-3.1 dBFS).
pub const LIMIT_KNEE: f32 = 0.7;

/// Master soft limiter — the last thing the mixed bus passes through.
///
/// Below [`LIMIT_KNEE`] this is the identity, so ordinary play is bit-for-bit untouched. Above
/// it the curve bends asymptotically to 1.0 and **can never exceed it**: AAudio hard-clamps
/// float PCM to [-1, 1], and hard clipping one loud effect summed with a quiet one does not
/// just distort the loud one — it deletes the quiet one inside every clipped sample.
///
/// Stateless (no attack/release, so no pumping and nothing to get wrong in the real-time
/// callback) and C¹-continuous at the knee: the derivative of the upper branch at `over = 0`
/// is exactly 1, so there is no audible corner where it engages.
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        return x;
    }
    let head = 1.0 - LIMIT_KNEE;
    let over = a - LIMIT_KNEE;
    let y = LIMIT_KNEE + head * (over / (over + head));
    if x < 0.0 { -y } else { y }
}

/// How many copies of ONE effect the bus plays at once. A newer copy takes the place of the
/// oldest, which fades out over [`SFX_STEAL_FADE_MS`]. Different effects never take each
/// other's place.
pub const SFX_SAME_MAX: usize = 2;

/// How long a copy that gives up its place takes to go silent. Cutting it in one sample
/// clicks; 10 ms is too short to hear as a fade.
pub const SFX_STEAL_FADE_MS: u32 = 10;

/// What makes two clips "the same effect": the samples the game handed over (FNV-1a 64), with
/// their format. The game's own level is already in the samples, so the same effect at two
/// levels counts as two — harmless, the level does not move mid-burst.
pub fn sfx_key(channels: u8, sampling_rate: u32, samples: &[i16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(channels);
    sampling_rate.to_le_bytes().into_iter().for_each(&mut eat);
    for s in samples {
        s.to_le_bytes().into_iter().for_each(&mut eat);
    }
    h
}

/// One effect in flight, already in the bus format (interleaved stereo at the device rate).
struct SfxClip {
    key: u64,
    data: Vec<f32>,
    pos: usize,
    /// Frames of fade left once a newer copy took this one's place.
    fading: Option<u32>,
}

/// The effects the bus is playing. Kept in arrival order, so the first match is the oldest.
pub struct SfxClips {
    clips: Vec<SfxClip>,
    fade_frames: u32,
}

impl SfxClips {
    pub fn new(sample_rate: u32) -> Self {
        Self { clips: Vec::new(), fade_frames: (sample_rate * SFX_STEAL_FADE_MS / 1000).max(1) }
    }

    pub fn push(&mut self, key: u64, data: Vec<f32>) {
        if data.is_empty() {
            return;
        }
        let mut same = self.clips.iter().filter(|c| c.key == key && c.fading.is_none()).count();
        for clip in self.clips.iter_mut() {
            if same < SFX_SAME_MAX {
                break;
            }
            if clip.key == key && clip.fading.is_none() {
                clip.fading = Some(self.fade_frames);
                same -= 1;
            }
        }
        self.clips.push(SfxClip { key, data, pos: 0, fading: None });
    }

    pub fn clear(&mut self) {
        self.clips.clear();
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Add every clip's next `buf.len()` samples (interleaved stereo) into `buf`.
    pub fn mix_into(&mut self, buf: &mut [f32]) {
        let fade = self.fade_frames as f32;
        self.clips.retain_mut(|clip| {
            let rest = &clip.data[clip.pos..];
            let n = rest.len().min(buf.len());
            match clip.fading.as_mut() {
                None => {
                    for (o, s) in buf[..n].iter_mut().zip(rest) {
                        *o += s;
                    }
                }
                Some(left) => {
                    for (o, s) in buf[..n].chunks_exact_mut(2).zip(rest.chunks_exact(2)) {
                        if *left == 0 {
                            break;
                        }
                        let g = *left as f32 / fade;
                        o[0] += s[0] * g;
                        o[1] += s[1] * g;
                        *left -= 1;
                    }
                    if *left == 0 {
                        return false;
                    }
                }
            }
            clip.pos += n;
            clip.pos < clip.data.len()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_is_transparent_below_the_knee() {
        for &x in &[0.0f32, 0.01, 0.25, 0.5, 0.69, LIMIT_KNEE] {
            assert_eq!(soft_limit(x), x, "positive {x}");
            assert_eq!(soft_limit(-x), -x, "negative {x}");
        }
    }

    #[test]
    fn limiter_never_reaches_full_scale() {
        for i in 0..2000 {
            let x = i as f32 * 0.01; // up to 20.0, far past anything reachable
            let y = soft_limit(x);
            assert!(y < 1.0, "soft_limit({x}) = {y} must stay below full scale");
            assert!(soft_limit(-x) > -1.0);
        }
    }

    #[test]
    fn limiter_is_monotonic_odd_and_smooth_at_the_knee() {
        let mut prev = f32::NEG_INFINITY;
        for i in 0..500 {
            let x = i as f32 * 0.01;
            let y = soft_limit(x);
            assert!(y > prev, "must be monotonic at {x}");
            prev = y;
            assert!((soft_limit(-x) + y).abs() < 1e-6, "must be odd at {x}");
        }
        let e = 1e-4;
        let slope = (soft_limit(LIMIT_KNEE + e) - soft_limit(LIMIT_KNEE)) / e;
        assert!((slope - 1.0).abs() < 1e-2, "slope at knee = {slope}");
    }

    #[test]
    fn two_full_scale_effects_together_stay_under_full_scale() {
        // The game's 8-bit effects reach full scale; two at once at SFX_GAIN must not clip.
        let sum = 2.0 * SFX_GAIN;
        let out = soft_limit(sum);
        assert!(out < 1.0 && out > 0.85, "two effects {sum} -> {out}");
        // And one alone passes the limiter untouched.
        assert_eq!(soft_limit(SFX_GAIN), SFX_GAIN);
    }

    /// A stereo clip of `frames` frames, every sample `v`.
    fn flat(frames: usize, v: f32) -> Vec<f32> {
        vec![v; frames * 2]
    }

    #[test]
    fn two_copies_of_one_effect_play_in_full() {
        let mut clips = SfxClips::new(48_000);
        clips.push(7, flat(100, 0.1));
        clips.push(7, flat(100, 0.1));
        let mut buf = [0.0f32; 20];
        clips.mix_into(&mut buf);
        assert!(buf.iter().all(|s| (s - 0.2).abs() < 1e-6), "{buf:?}");
    }

    #[test]
    fn a_third_copy_fades_the_oldest_out_without_a_jump() {
        let rate = 48_000;
        let fade = (rate * SFX_STEAL_FADE_MS / 1000) as usize;
        let mut clips = SfxClips::new(rate);
        clips.push(7, flat(10_000, 0.1));
        clips.push(7, flat(10_000, 0.1));
        let mut buf = vec![0.0f32; 64];
        clips.mix_into(&mut buf);
        clips.push(7, flat(10_000, 0.1));
        let mut out = vec![0.0f32; (fade + 64) * 2];
        clips.mix_into(&mut out);
        assert!((out[0] - 0.3).abs() < 1e-6, "{}", out[0]);
        for f in out.chunks_exact(2).collect::<Vec<_>>().windows(2) {
            assert!(f[0][0] - f[1][0] < 0.1 / fade as f32 + 1e-6);
        }
        assert!((out[out.len() - 2] - 0.2).abs() < 1e-6, "{}", out[out.len() - 2]);
        assert_eq!(clips.len(), SFX_SAME_MAX);
    }

    #[test]
    fn different_effects_do_not_take_each_others_place() {
        let mut clips = SfxClips::new(48_000);
        for key in 0..5 {
            clips.push(key, flat(2_000, 0.1));
        }
        let mut buf = vec![0.0f32; 2_000];
        clips.mix_into(&mut buf);
        assert!((buf[1_998] - 0.5).abs() < 1e-5, "{}", buf[1_998]);
        assert_eq!(clips.len(), 5);
    }

    #[test]
    fn a_rapid_burst_of_one_effect_never_piles_up() {
        let rate = 48_000u32;
        let clip_frames = rate as usize * 967 / 1000;
        let step = rate as usize * 60 / 1000;
        let mut clips = SfxClips::new(rate);
        let mut buf = vec![0.0f32; step * 2];
        let mut loudest = 0.0f32;
        for _ in 0..35 {
            clips.push(43, flat(clip_frames, 0.1));
            buf.fill(0.0);
            clips.mix_into(&mut buf);
            loudest = buf.iter().fold(loudest, |m, s| m.max(*s));
        }
        assert!(loudest <= 0.1 * SFX_SAME_MAX as f32 + 0.1 + 1e-6, "{loudest}");
        assert!(clips.len() <= SFX_SAME_MAX);
    }

    #[test]
    fn a_finished_clip_leaves_the_bus() {
        let mut clips = SfxClips::new(8_000);
        clips.push(1, flat(10, 0.1));
        let mut buf = vec![0.0f32; 40];
        clips.mix_into(&mut buf);
        assert!(clips.is_empty());
        assert!((buf[19] - 0.1).abs() < 1e-6 && buf[20] == 0.0);
    }

    #[test]
    fn the_key_is_the_samples_and_their_format() {
        let a = [1i16, -2, 3];
        assert_eq!(sfx_key(1, 8_000, &a), sfx_key(1, 8_000, &a));
        assert_ne!(sfx_key(1, 8_000, &a), sfx_key(1, 4_000, &a));
        assert_ne!(sfx_key(1, 8_000, &a), sfx_key(2, 8_000, &a));
        assert_ne!(sfx_key(1, 8_000, &a), sfx_key(1, 8_000, &[1, -2, 4]));
    }
}
