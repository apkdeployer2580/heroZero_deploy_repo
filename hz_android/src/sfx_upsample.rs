//! Sound effects, from the rate the game ships them at to the rate the device plays.
//!
//! Verbatim from hero2NativeAPP's `wie_android/src/sfx_upsample.rs` (2026-10-22): 영웅서기 Zero's
//! effects are 8-bit 8 kHz PCM WAVs, the same band hero2's 8 kHz ADPCM clips had, and the same
//! three things apply — band-limited interpolation, a tail fade, "air". The host tests below are
//! hero2's `sfx_upsample_selftest.rs` (synthetic tones; the game's own clips stay out of the repo).
//
// ★ 2026-10-02 ("효과음도 업스케일링"). The effects are 4-bit ADPCM at 8 kHz (one at 4 kHz) —
// `wie_backend::system::atr` decodes them — and they used to reach the bus through rodio's
// LINEAR resampler. A straight line between samples is a weak low-pass with a weak stop band:
// it droops the top of the clip's own band (−6 dB at 3.5 kHz) and leaves the mirror images of
// that band above 4 kHz only 10–20 dB down, loudest where the clip is loudest. This file does
// the three things a clip needs instead, once per clip on the audio thread (never in the
// callback):
//
//   1. band-limited interpolation — a Kaiser-windowed sinc, 64 taps at the clip's rate (★ 32
//      until 2026-10-22's second pass: the transition band straddled the 4 kHz edge, so a clip's
//      top 500 Hz drooped and its image there was only −42 dB down — now 250 Hz and −70 dB), so
//      the clip's band arrives flat and its images do not arrive at all;
//   2. a 4 ms fade at the clip's end, so a clip the game authored to stop a little off zero
//      (effect 43 ends at −1,377) does not end on a tick;
//   3. "air" — the octave above the clip's band, regenerated from the clip's own top octave
//      by rectification (the classic high-frequency regeneration for narrow-band audio). It
//      follows the clip's envelope, is harmonic to what it comes from, and a clip with
//      nothing in its top octave gets nothing. [`SFX_AIR`] is how much; `0.0` is none.
//
// ★ and, before those three (영웅서기 Zero, 2026-10-22): the clip's drifting baseline taken out —
// [`sfx_low_cut`], measured below. ★ And after them (the same day's second pass): a SECOND
// octave of air, 8–16 kHz, regenerated from the first the same way at [`SFX_AIR2`] — the device
// plays to 22 kHz and an effect that stopped dead at 8 kHz was still a phone's. Measured on the
// fourteen effects: 8–16 kHz sat 15–20 dB under the first octave (the filters' skirts alone);
// at 0.4 it only came up 2.5–4.5 dB (a rectified copy of an already small band is small), at
// 1.0 — the first octave's own amount — it sits 10–11 dB under it, a continuing roll-off.
//
// No dependencies: pure DSP, host-tested.

use std::sync::OnceLock;

/// Interpolation kernel half-width, in input samples.
const SFX_HALF: usize = 32;
const SFX_TAPS: usize = SFX_HALF * 2;
/// Sub-sample positions the kernel is tabulated at (rows are blended linearly between).
const SFX_PHASES: usize = 256;
/// Kaiser window shape: about −80 dB of stop band.
const SFX_KAISER_BETA: f64 = 8.0;
/// The fade at the end of a clip, in ms.
const SFX_TAIL_FADE_MS: u32 = 4;
/// How much of the regenerated octave is added. Measured over hero2's eleven effects at this
/// value the new octave sits 11–16 dB under the clip's own top octave — a continuing roll-off,
/// not a shelf (the straight line's images sat 8–10 dB under it, and were not harmonic). ★ The knob to turn by ear; `0.0` leaves the clip band-limited and nothing else.
pub const SFX_AIR: f32 = 0.6;
/// How much of the SECOND regenerated octave (8–16 kHz) is added, relative to the first.
pub const SFX_AIR2: f32 = 1.0;
/// Clips above this rate have their top octave where there is nothing left to add.
const SFX_AIR_MAX_RATE: u32 = 16_000;

/// Modified Bessel function of the first kind, order 0 (the Kaiser window's).
fn sfx_bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..64 {
        term *= (half / k as f64) * (half / k as f64);
        sum += term;
        if term < sum * 1e-14 {
            break;
        }
    }
    sum
}

/// `SFX_PHASES + 1` rows of `SFX_TAPS` coefficients. Row `p`, tap `j` weighs the input sample
/// `SFX_HALF - 1 - j + p / SFX_PHASES` samples before the output position. Every row sums to 1.
fn sfx_kernel() -> &'static [[f32; SFX_TAPS]] {
    static KERNEL: OnceLock<Vec<[f32; SFX_TAPS]>> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let i0_beta = sfx_bessel_i0(SFX_KAISER_BETA);
        (0..=SFX_PHASES)
            .map(|p| {
                let mut row = [0.0f64; SFX_TAPS];
                for (j, tap) in row.iter_mut().enumerate() {
                    let u = p as f64 / SFX_PHASES as f64 + (SFX_HALF - 1) as f64 - j as f64;
                    let x = u / SFX_HALF as f64;
                    if x.abs() >= 1.0 {
                        continue;
                    }
                    let sinc = if u.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u) };
                    *tap = sinc * sfx_bessel_i0(SFX_KAISER_BETA * (1.0 - x * x).sqrt()) / i0_beta;
                }
                let sum: f64 = row.iter().sum();
                let mut out = [0.0f32; SFX_TAPS];
                for (o, tap) in out.iter_mut().zip(row) {
                    *o = (tap / sum) as f32;
                }
                out
            })
            .collect()
    })
}

/// One channel from `in_rate` to `out_rate`, band-limited to the input's own band. The output
/// runs [`SFX_HALF`] input samples past the input's end, so the last sample rings out.
///
/// A clip already at or above the device rate (none of hero2's) is interpolated linearly,
/// as before.
pub fn sfx_resample(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == 0 || out_rate == 0 {
        return Vec::new();
    }
    let (in_hz, out_hz) = (in_rate as u64, out_rate as u64);
    if in_rate >= out_rate {
        let out_len = (input.len() as u64 * out_hz / in_hz) as usize;
        return (0..out_len as u64)
            .map(|n| {
                let pos = n * in_hz;
                let i = (pos / out_hz) as usize;
                let frac = (pos % out_hz) as f32 / out_rate as f32;
                let a = input[i];
                let b = input.get(i + 1).copied().unwrap_or(a);
                a + (b - a) * frac
            })
            .collect();
    }
    let kernel = sfx_kernel();
    let out_len = ((input.len() + SFX_HALF) as u64 * out_hz / in_hz) as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len as u64 {
        let pos = n * in_hz;
        let centre = (pos / out_hz) as isize;
        let phase = (pos % out_hz) as f32 / out_rate as f32 * SFX_PHASES as f32;
        let p = (phase as usize).min(SFX_PHASES - 1);
        let blend = phase - p as f32;
        let (row, next) = (&kernel[p], &kernel[p + 1]);
        // Tap j reads input[first + j]; only the taps that land inside the clip.
        let first = centre - (SFX_HALF as isize - 1);
        let from = (-first).max(0) as usize;
        let to = (input.len() as isize - first).clamp(0, SFX_TAPS as isize) as usize;
        let mut acc = 0.0f32;
        for j in from..to {
            acc += input[(first + j as isize) as usize] * (row[j] + (next[j] - row[j]) * blend);
        }
        out.push(acc);
    }
    out
}

/// A second-order filter section (transposed direct form II).
struct SfxBiquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl SfxBiquad {
    /// `high`: a high-pass at `hz`, else a low-pass; `q` 0.7071 is Butterworth.
    fn new(high: bool, hz: f32, rate: f32, q: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * hz / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if high {
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0]
        } else {
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
        };
        Self {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Add the octave above a resampled clip's band (`in_rate / 2` up to `in_rate`), regenerated
/// from the clip's top octave: that octave, rectified — which doubles every frequency in it —
/// and filtered down to the new octave alone. `amount` scales it; see [`SFX_AIR`].
pub fn sfx_add_air(signal: &mut [f32], in_rate: u32, out_rate: u32, amount: f32) {
    if amount <= 0.0 || in_rate == 0 || in_rate > SFX_AIR_MAX_RATE || in_rate as u64 * 2 > out_rate as u64 {
        return;
    }
    let (edge, rate) = (in_rate as f32 / 2.0, out_rate as f32);
    let mut top_octave = SfxBiquad::new(true, edge / 2.0, rate, std::f32::consts::FRAC_1_SQRT_2);
    // Fourth-order Butterworth high-pass at the clip's band edge: what rectifying puts back
    // INSIDE the band (the envelope, the difference tones) must not come through.
    let mut above = [SfxBiquad::new(true, edge, rate, 0.541_196_1), SfxBiquad::new(true, edge, rate, 1.306_563)];
    let mut ceiling = SfxBiquad::new(false, edge * 2.0, rate, std::f32::consts::FRAC_1_SQRT_2);
    for s in signal.iter_mut() {
        let mut new_octave = top_octave.run(*s).abs();
        for stage in above.iter_mut() {
            new_octave = stage.run(new_octave);
        }
        *s += amount * ceiling.run(new_octave);
    }
}

// ★ heroZero 2026-10-22 (일곱 번째 세션, "모든 효과음 업스케일링"). Measured on 영웅서기 Zero's fourteen
// 8-bit effects (`sfx_survey`): a defect the three steps above do not touch.
//
//   * A WANDERING BASELINE. Every clip starts on 128, but its slow average drifts — up to ±0.36
//     of full scale, at 5–20 Hz — and many END off centre: `e/13` on −0.31, `e/8` on −0.2,
//     `e/12` on +0.16, `e/9`/`e/10` spend their second half sitting on +0.07. A phone speaker
//     plays nothing down there; what is heard is the STEP back to zero when the clip ends (the
//     4 ms fade turns −0.31 into a thump, not into silence). [`sfx_low_cut`] takes it out, once
//     per clip, before the fade, at the clip's own rate: every end now sits within ±0.02.
//   * NOT REPAIRED: the flat tops. Five clips have samples on the 8-bit rails (`e/4` 101, `e/12`
//     61, `e/2` 55 in runs of up to 10). Measured on band-limited test signals: a local cubic
//     through the neighbours barely helps at 8 kHz (error 0.93–1.0 of the clipped wave), and the
//     reliable method — consistent band-limited reconstruction — needs the sound to stop short
//     of its Nyquist rate, which these do not (most carry −10 to −25 dB right up to 4 kHz; with
//     content that high it made the error worse in over half the trials). Left as authored.

/// Where [`sfx_low_cut`] starts: an octave and more below anything an 8 kHz phone effect
/// carries, far enough above the 5–20 Hz drift to take it out within a few milliseconds.
pub const SFX_LOW_CUT_HZ: f32 = 35.0;

/// The DC and the drift below [`SFX_LOW_CUT_HZ`] out of one channel: a second-order Butterworth
/// high-pass, from rest — every clip starts on its centre, so there is no step to ring on.
pub fn sfx_low_cut(x: &mut [f32], rate: u32) {
    if rate == 0 || SFX_LOW_CUT_HZ * 4.0 >= rate as f32 {
        return;
    }
    let mut cut = SfxBiquad::new(true, SFX_LOW_CUT_HZ, rate as f32, std::f32::consts::FRAC_1_SQRT_2);
    for s in x.iter_mut() {
        *s = cut.run(*s);
    }
}

/// One channel of a clip, at its own rate, made ready for the device: the low cut, the tail
/// fade, the band-limited resampling, the air and its second octave. `repair` off is the Phase 4a
/// path (no low cut, one octave of air) but for the kernel's width.
fn sfx_channel(mut x: Vec<f32>, in_rate: u32, out_rate: u32, repair: bool) -> Vec<f32> {
    if repair {
        sfx_low_cut(&mut x, in_rate);
    }
    let frames = x.len();
    let fade = ((in_rate as u64 * SFX_TAIL_FADE_MS as u64 / 1000) as usize).min(frames);
    for (i, s) in x[frames - fade..].iter_mut().enumerate() {
        *s *= 0.5 * (1.0 + (std::f32::consts::PI * (i + 1) as f32 / fade as f32).cos());
    }
    let mut y = sfx_resample(&x, in_rate, out_rate);
    sfx_add_air(&mut y, in_rate, out_rate, SFX_AIR);
    if repair {
        // The second octave: the first one's band (`in_rate / 2 .. in_rate`) is the new "top octave".
        sfx_add_air(&mut y, in_rate * 2, out_rate, SFX_AIR * SFX_AIR2);
    }
    y
}

/// One decoded effect — interleaved 16-bit PCM, `channels` wide, at `in_rate` — as the bus
/// plays it: interleaved stereo `f32` at `out_rate`, scaled by `gain`.
pub fn sfx_clip(samples: &[i16], channels: u8, in_rate: u32, out_rate: u32, gain: f32) -> Vec<f32> {
    sfx_clip_with(samples, channels, in_rate, out_rate, gain, true)
}

/// [`sfx_clip`], with the second pass's repairs on or off (`repair`) — off is the Phase 4a path,
/// kept for the before/after renders.
pub fn sfx_clip_with(samples: &[i16], channels: u8, in_rate: u32, out_rate: u32, gain: f32, repair: bool) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let frames = samples.len() / channels;
    let side = |c: usize| {
        let x: Vec<f32> = (0..frames).map(|i| samples[i * channels + c] as f32 / 32768.0).collect();
        sfx_channel(x, in_rate, out_rate, repair)
    };
    let left = side(0);
    let right = if channels > 1 { Some(side(1)) } else { None };
    let mut out = Vec::with_capacity(left.len() * 2);
    for (i, l) in left.iter().enumerate() {
        out.push(l * gain);
        out.push(right.as_ref().map_or(*l, |r| r[i]) * gain);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const IN: u32 = 8_000;
    const OUT: u32 = 48_000;

    fn tone(hz: f32, rate: u32, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames).map(|n| amp * (2.0 * std::f32::consts::PI * hz * n as f32 / rate as f32).sin()).collect()
    }

    fn tone_i16(hz: f32, rate: u32, frames: usize, amp: f32) -> Vec<i16> {
        tone(hz, rate, frames, amp).iter().map(|s| (s * 32767.0) as i16).collect()
    }

    /// Amplitude of the `hz` component of `x` (a Hann-windowed single-bin transform).
    fn level(x: &[f32], rate: u32, hz: f32) -> f32 {
        let n = x.len() as f32;
        let (mut re, mut im, mut win) = (0.0f64, 0.0f64, 0.0f64);
        for (i, s) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n).cos();
            let ph = 2.0 * std::f64::consts::PI * hz as f64 * i as f64 / rate as f64;
            re += (*s * w) as f64 * ph.cos();
            im += (*s * w) as f64 * ph.sin();
            win += w as f64;
        }
        (2.0 * (re * re + im * im).sqrt() / win) as f32
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-9).log10()
    }

    /// The steady middle of a resampled tone: clear of both ends' ramps.
    fn middle(y: &[f32]) -> &[f32] {
        &y[y.len() / 4..y.len() * 3 / 4]
    }

    #[test]
    fn every_kernel_row_sums_to_one_and_the_first_is_an_impulse() {
        let k = sfx_kernel();
        assert_eq!(k.len(), SFX_PHASES + 1);
        for (p, row) in k.iter().enumerate() {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "row {p} sums to {sum}");
        }
        assert!((k[0][SFX_HALF - 1] - 1.0).abs() < 1e-5);
        assert!(k[0].iter().enumerate().all(|(j, c)| j == SFX_HALF - 1 || c.abs() < 1e-5));
    }

    #[test]
    fn output_samples_on_input_instants_are_the_input() {
        let x = tone(700.0, IN, 400, 0.5);
        let y = sfx_resample(&x, IN, OUT);
        assert_eq!(y.len(), (400 + SFX_HALF) * 6);
        for i in SFX_HALF..400 - SFX_HALF {
            assert!((y[i * 6] - x[i]).abs() < 1e-5, "input sample {i}");
        }
    }

    #[test]
    fn a_tone_keeps_its_level_and_leaves_no_mirror_image() {
        for hz in [300.0f32, 1000.0, 2000.0, 3000.0, 3500.0] {
            let y = sfx_resample(&tone(hz, IN, 4000, 0.5), IN, OUT);
            let got = db(level(middle(&y), OUT, hz) / 0.5);
            assert!(got.abs() < 0.5, "{hz} Hz came through at {got:+.2} dB");
            // The kernel's transition band straddles the 4 kHz edge: a tone within 250 Hz of
            // it keeps its level, but its image lands inside the band too.
            if hz <= 3500.0 {
                let image = IN as f32 - hz;
                let img = db(level(middle(&y), OUT, image) / 0.5);
                assert!(img < -70.0, "{hz} Hz left an image at {image} Hz, {img:.1} dB");
            }
        }
    }

    #[test]
    fn same_or_lower_rates_interpolate_linearly() {
        let x = tone(300.0, 48_000, 480, 0.5);
        assert_eq!(sfx_resample(&x, 48_000, 48_000), x);
        assert_eq!(sfx_resample(&x, 48_000, 24_000).len(), 240);
        assert!(sfx_resample(&[], IN, OUT).is_empty());
    }

    #[test]
    fn air_is_off_where_it_has_no_room_and_at_zero() {
        let clean = sfx_resample(&tone(3000.0, IN, 2000, 0.5), IN, OUT);
        let mut y = clean.clone();
        sfx_add_air(&mut y, IN, OUT, 0.0);
        assert_eq!(y, clean);
        let mut y = clean.clone();
        sfx_add_air(&mut y, 32_000, OUT, SFX_AIR);
        assert_eq!(y, clean);
    }

    #[test]
    fn silence_stays_silence() {
        let out = sfx_clip(&vec![0i16; 800], 1, IN, OUT, 0.4);
        assert_eq!(out.len(), (800 + SFX_HALF) * 6 * 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(sfx_clip(&[], 1, IN, OUT, 0.4).is_empty());
    }

    #[test]
    fn the_low_cut_takes_out_the_drift_and_keeps_the_sound() {
        // A 1 kHz tone on a baseline that walks to -0.3 and stays there (e/13's shape).
        let n = 1600;
        let mut x: Vec<f32> = (0..n)
            .map(|i| {
                let drift = -0.3 * (i as f32 / 400.0).min(1.0);
                drift + 0.3 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / IN as f32).sin()
            })
            .collect();
        sfx_low_cut(&mut x, IN);
        let tail = &x[n - 400..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 0.005, "the end sits on {mean}");
        let kept = level(tail, IN, 1000.0);
        assert!((kept - 0.3).abs() < 0.01, "1 kHz kept at {kept}");
    }

    #[test]
    fn the_repairs_are_skippable_and_change_nothing_on_a_clean_tone() {
        let x = tone_i16(1000.0, IN, 1600, 16_000.0 / 32768.0);
        let on = sfx_clip_with(&x, 1, IN, OUT, 0.4, true);
        let off = sfx_clip_with(&x, 1, IN, OUT, 0.4, false);
        let left = |v: &[f32]| v.chunks(2).map(|f| f[0]).collect::<Vec<_>>();
        let (a, b) = (left(&on), left(&off));
        assert!((level(middle(&a), OUT, 1000.0) - level(middle(&b), OUT, 1000.0)).abs() < 1e-3);
    }

    /// The game's own effects (not in the repo): `HZ_SFX_DIR` = the jar's `snd/e/`. Prints what
    /// the repairs did to each, and with `HZ_AB_OUT` writes `sfx_<n>_before.wav` (the Phase 4a
    /// path) and `sfx_<n>_after.wav` at 48 kHz for listening —
    /// `cargo test -p hz_android --release sfx_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn sfx_survey() {
        let Ok(dir) = std::env::var("HZ_SFX_DIR") else {
            eprintln!("HZ_SFX_DIR not set");
            return;
        };
        for n in 0..32 {
            let Ok(file) = std::fs::read(format!("{dir}/{n}")) else { continue };
            let Some(at) = file.windows(4).position(|w| w == b"data") else { continue };
            let samples: Vec<i16> = file[at + 8..].iter().map(|&b| ((b as i16) - 128) << 8).collect();
            let raw: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
            let mut fixed = raw.clone();
            sfx_low_cut(&mut fixed, IN);
            let end = |v: &[f32]| v[v.len() - 80..].iter().sum::<f32>() / 80.0;
            println!("e/{n}: last 10 ms sits on {:+.3} -> {:+.3}", end(&raw), end(&fixed));
            if let Ok(out) = std::env::var("HZ_AB_OUT") {
                for (name, repair) in [("before", false), ("after", true)] {
                    let clip = sfx_clip_with(&samples, 1, IN, OUT, crate::audio_mix::SFX_GAIN, repair);
                    write_wav(&format!("{out}/sfx_{n}_{name}.wav"), &clip, OUT);
                }
            }
        }
    }

    /// Interleaved stereo `f32` as a 16-bit WAV, through the bus's limiter.
    pub(crate) fn write_wav(path: &str, stereo: &[f32], rate: u32) {
        let data: Vec<u8> = stereo
            .iter()
            .flat_map(|s| ((crate::audio_mix::soft_limit(*s) * 32767.0).round() as i16).to_le_bytes())
            .collect();
        let mut w = Vec::with_capacity(44 + data.len());
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 4).to_le_bytes());
        w.extend_from_slice(&4u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        std::fs::write(path, w).unwrap();
    }

    #[test]
    fn a_clip_is_stereo_scaled_and_ends_on_zero() {
        let x = tone_i16(1000.0, IN, 1600, 16_000.0 / 32768.0);
        let out = sfx_clip(&x, 1, IN, OUT, 0.4);
        assert!(out.chunks(2).all(|f| f[0] == f[1]), "mono plays on both sides");
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        let body = level(middle(&left), OUT, 1000.0);
        assert!((body - 0.4 * 16_000.0 / 32768.0).abs() < 0.01, "body {body}");
        assert!(out[out.len() - 1].abs() < 1e-3, "ends near zero: {}", out[out.len() - 1]);
    }
}
