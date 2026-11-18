//! The music's top octave — Phase 4c+ (2026-10-22, "배경음 업스케일링").
//!
//! Measured on the sixteen songs (`music_survey` dumps, 4096-point Welch spectra): every one is
//! cut by its AAC encoder's low-pass at 15.75–17.5 kHz — the band below sits 5–50 dB under
//! 1–4 kHz, the band above is 60–100 dB down, i.e. nothing. And they are, in substance, MONO:
//! side over mid is −32 to −84 dB from 0.2 to 8 kHz (one song at −19 dB); what side there is
//! above 10 kHz is the codec's own stereo noise.
//!
//! ★ Redone 2026-10-26 (user: "첫 시작 고아원부터 배경음 중 음 몇 개가 어울리지 않고 따로따로
//! 나온다 — 다시 업스케일링"). The first version copied the band under the cutoff up by a fixed
//! 5 kHz (single-sideband, as HE-AAC's SBR patches it). Every partial moved by the same Hz is a
//! partial of no note at all: the orphanage song's bell lines at 11.5 / 12.5 / 14.8 kHz came back
//! as tones at 16.5 / 17.5 / 19.8 kHz, out of tune with everything, starting and stopping with
//! each note — measured on a 440 Hz tone, 0 % of what it added sat on a harmonic of 440 Hz. The
//! decoder was cleared first (symphonia against ffmpeg on all sixteen songs: 119–134 dB), and the
//! songs themselves are in tune (partials within ±5 cents but for the natural 5th and 7th
//! harmonics' −14 and −31 cents).
//!
//! So the top is now put back an OCTAVE up, the way a harmonic exciter does it: every frequency
//! in the band under the cutoff DOUBLED. A partial of a note doubled is another partial of the
//! same note (2·h·f₀), and two of them summed are too ((h + k)·f₀) — what is added belongs to the
//! note it came from. Here, per frame, at the song's own rate, on the wie-music thread:
//!
//!   1. the mid ((L + R) / 2) through a band-pass [`SOURCE_LO`]..[`SOURCE_HI`] (12th-order
//!      Butterworth below, [`SOURCE_HI_ORDER`] above) — the octave under the new band;
//!   2. an IIR allpass pair (Olli Niemitalo's 90° design: two chains of four second-order
//!      sections) splits it into two signals 90° apart — 89.3–90.0° from 100 Hz to 20 kHz — the
//!      band as `z = i + jq`;
//!   3. `Re(z²) = i² − q²` is the band with every frequency doubled and only SUMS of two (a
//!      complex square has no difference tones), divided by the band's envelope (up at once,
//!      down over [`ENV_RELEASE_MS`]) so it stays the band's own size, never larger;
//!   4. a 12th-order high-pass at [`NEW_LO`] keeps what lands above the cutoff (and whatever
//!      difference tones the pair's last fraction of a degree lets through, far below), a
//!      second-order low-pass at [`NEW_HI`] rolls the top off;
//!   5. its level FOLLOWS the song's own top ([`TOP_LO`] up, 8th order) over [`FOLLOW_MS`]: the
//!      new band is [`MUSIC_AIR`] of it, never more than the band it was doubled from
//!      ([`GAIN_MAX`]) — a dull note gets no air, a hi-hat gets its sizzle (SBR's envelope,
//!      estimated instead of sent);
//!   6. added to both channels alike — the songs are mono, and copying the codec's side noise up
//!      would be adding noise.
//!
//! Why a steep [`SOURCE_HI`]: doubling folds anything above a quarter of the rate back down from
//! the Nyquist rate, out of tune — the orphanage song's strong 12.5 kHz line would come back at
//! 44.1 − 25 = 19.1 kHz. 16th order at 9.8 kHz holds that to −57 dB under the music even with
//! nothing in the band to double, and −80 dB beside a note that is (measured on synthetic tones).
//! Why [`NEW_LO`] = 16 kHz: the songs' cutoff, frame by frame, is 15.7–16.0 kHz (median), now and
//! then 17 kHz. On the sixteen songs the new band sits 9–16 dB under the song's 12–16 kHz at
//! 16–18 kHz and 10–21 dB at 18–20 kHz (the old shift: 8–16 / 16–22), 37–45 dB down above
//! 20 kHz, and nothing under 15 kHz changes (0.000 dB).
//!
//! ★ Second pass (the same day): the codec's STEREO NOISE. Measured, the songs are mono or
//! nearly — side over mid −19 to −84 dB in the body (0.2–8 kHz) — yet above 10 kHz every one's
//! side channel sits only 20–43 dB under its mid, and in b/1 the side is LOUDER up there (−21 dB)
//! than in the body (−32): that is the encoder's mid/side quantisation noise, decorrelated fizz
//! between the ears that was never in the music. Even the one song with real stereo (b/14, −19 dB
//! in the body) has only −43 dB of side above 10 kHz. So the side channel is low-passed at
//! [`SIDE_LO`] (eighth order: −20 dB by 10 kHz, −38 dB by 13 kHz) — the stereo image stays as
//! authored where the music has one, and the fizz above goes — for every song alike, no judging.
//! The mid is untouched. Measured in `music_survey`.
//!
//! The new band follows the source band sample for sample (no analysis window, no latency beyond
//! the filters'), so a hi-hat's new sizzle starts and stops with the hi-hat. About 170
//! multiply-adds per frame. A song below [`MIN_RATE`] has no room above its cutoff and passes
//! untouched.

/// The band that is doubled: 8–9.8 kHz lands on 16–19.6 kHz.
pub const SOURCE_LO: f32 = 8_000.0;
pub const SOURCE_HI: f32 = 9_800.0;
/// Order of the source band's low-pass: what is above a quarter of the rate must not get in.
const SOURCE_HI_ORDER: usize = 16;
/// Where the new band starts — the songs' codec cutoff (15.7–16.0 kHz, frame by frame).
pub const NEW_LO: f32 = 16_000.0;
/// Where the new band starts rolling off (second order).
pub const NEW_HI: f32 = 19_000.0;
/// Order of the source's and the new band's Butterworth high-passes (sections = order / 2).
const ORDER: usize = 12;
/// The song's own top, which the new band follows.
pub const TOP_LO: f32 = 13_000.0;
const TOP_ORDER: usize = 8;
/// How fast the doubled band's envelope falls (it rises at once).
pub const ENV_RELEASE_MS: f32 = 5.0;
/// How fast the new band's level follows the song's top.
pub const FOLLOW_MS: f32 = 20.0;
/// The new band is never louder than the band it was doubled from.
pub const GAIN_MAX: f32 = 1.0;
/// Below this rate there is no octave above the cutoff to fill.
pub const MIN_RATE: u32 = 40_000;

/// The new band's level against the song's own top ([`TOP_LO`] up), in amplitude (−9 dB).
/// ★ The knob to turn by ear; `0.0` is the song as decoded.
pub const MUSIC_AIR: f32 = 0.35;

/// Where the side channel (L − R) is cut: the songs' stereo lives below it, the codec's does not.
pub const SIDE_LO: f32 = 7_500.0;
/// Order of the side channel's low-pass.
const SIDE_ORDER: usize = 8;

/// Niemitalo's coefficients, as published: each section uses the SQUARE (`a²`), and path A's
/// output is the one delayed a sample. (Used unsquared, the split was 78° at 4 kHz — measured.)
const PATH_A: [f64; 4] = [0.692_387_8, 0.936_065_432_295_9, 0.988_229_522_686, 0.998_748_845_273_7];
const PATH_B: [f64; 4] = [0.402_192_116_242_6, 0.856_171_088_242, 0.972_290_954_565_1, 0.995_288_479_127_8];

/// A second-order section (transposed direct form II), Butterworth-style high- or low-pass.
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
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
        Self { b: [b[0] / a0, b[1] / a0, b[2] / a0], a: [-2.0 * cos / a0, (1.0 - alpha) / a0], z: [0.0; 2] }
    }

    /// The `order / 2` sections of a Butterworth of (even) `order`.
    fn butterworth_of(order: usize, high: bool, hz: f32, rate: f32) -> Vec<Self> {
        (0..order / 2)
            .map(|k| {
                let q = 1.0 / (2.0 * (std::f64::consts::PI * (2 * k + 1) as f64 / (2 * order) as f64).cos());
                Self::new(high, hz, rate, q as f32)
            })
            .collect()
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// `y[n] = c (x[n] + y[n-2]) - x[n-2]`: an allpass in `z^-2`.
#[derive(Clone, Copy)]
struct Allpass {
    c: f32,
    x: [f32; 2],
    y: [f32; 2],
}

impl Allpass {
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.c * (x + self.y[1]) - self.x[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// The 90° pair: `(in phase, quadrature)`.
pub struct Hilbert {
    a: [Allpass; 4],
    b: [Allpass; 4],
    /// Path A's one-sample delay.
    delay: f32,
}

impl Hilbert {
    pub fn new() -> Self {
        let chain = |c: [f64; 4]| c.map(|a| Allpass { c: (a * a) as f32, x: [0.0; 2], y: [0.0; 2] });
        Self { a: chain(PATH_A), b: chain(PATH_B), delay: 0.0 }
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> (f32, f32) {
        let a = self.a.iter_mut().fold(x, |s, ap| ap.run(s));
        let b = self.b.iter_mut().fold(x, |s, ap| ap.run(s));
        let i = std::mem::replace(&mut self.delay, a);
        (i, b)
    }
}

impl Default for Hilbert {
    fn default() -> Self {
        Self::new()
    }
}

/// One song's band extension, frame by frame, for as long as the stream lasts (loops included).
pub struct MusicAir {
    amount: f32,
    source: Vec<Biquad>,
    hilbert: Hilbert,
    /// The source band's envelope, and its fall per frame.
    env: f32,
    env_fall: f32,
    above: Vec<Biquad>,
    ceiling: Biquad,
    /// The song's top, and the two powers the new band's level comes from (one-pole, `follow`).
    top: Vec<Biquad>,
    top_power: f32,
    new_power: f32,
    follow: f32,
    /// The side channel's low-pass (Butterworth of order [`SIDE_ORDER`] at [`SIDE_LO`]).
    side: Vec<Biquad>,
}

impl MusicAir {
    /// Off — a straight wire — for a rate without room above the cutoff, or `amount` 0.
    pub fn new(rate: u32, amount: f32) -> Self {
        let amount = if rate >= MIN_RATE { amount.max(0.0) } else { 0.0 };
        let r = rate.max(1) as f32;
        let mut source = Biquad::butterworth_of(ORDER, true, SOURCE_LO.min(r * 0.2), r);
        source.extend(Biquad::butterworth_of(SOURCE_HI_ORDER, false, SOURCE_HI.min(r * 0.24), r));
        Self {
            amount,
            source,
            hilbert: Hilbert::new(),
            env: 0.0,
            env_fall: (-1000.0 / (ENV_RELEASE_MS * r)).exp(),
            above: Biquad::butterworth_of(ORDER, true, NEW_LO.min(r * 0.4), r),
            ceiling: Biquad::new(false, NEW_HI.min(r * 0.47), r, std::f32::consts::FRAC_1_SQRT_2),
            top: Biquad::butterworth_of(TOP_ORDER, true, TOP_LO.min(r * 0.33), r),
            top_power: 0.0,
            new_power: 0.0,
            follow: 1.0 - (-1000.0 / (FOLLOW_MS * r)).exp(),
            side: Biquad::butterworth_of(SIDE_ORDER, false, SIDE_LO.min(r * 0.4), r),
        }
    }

    pub fn is_on(&self) -> bool {
        self.amount > 0.0
    }

    /// Every frame (stereo, at the rate given to [`MusicAir::new`]): the side channel's top cut,
    /// the new band added to both channels alike.
    pub fn process(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames.iter_mut() {
            let mid = 0.5 * (f[0] + f[1]);
            let side = self.side.iter_mut().fold(0.5 * (f[0] - f[1]), |s, b| b.run(s));
            let hf = if self.is_on() { self.next(mid) } else { 0.0 };
            f[0] = mid + side + hf;
            f[1] = mid - side + hf;
        }
    }

    #[inline]
    fn next(&mut self, mid: f32) -> f32 {
        let band = self.source.iter_mut().fold(mid, |s, f| f.run(s));
        let (i, q) = self.hilbert.run(band);
        self.env = (i * i + q * q).sqrt().max(self.env * self.env_fall);
        // Re((i + jq)²): every frequency in the band doubled, at the band's own size.
        let doubled = if self.env > 1e-9 { (i * i - q * q) / self.env } else { 0.0 };
        let new = self.ceiling.run(self.above.iter_mut().fold(doubled, |s, f| f.run(s)));
        let top = self.top.iter_mut().fold(mid, |s, f| f.run(s));
        self.top_power += self.follow * (top * top - self.top_power);
        self.new_power += self.follow * (new * new - self.new_power);
        let gain = if self.new_power > 1e-14 { (self.amount * (self.top_power / self.new_power).sqrt()).min(GAIN_MAX) } else { 0.0 };
        gain * new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn sine(hz: f32, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames).map(|n| amp * (std::f32::consts::TAU * hz * n as f32 / RATE as f32).sin()).collect()
    }

    /// Amplitude of `hz` in `x` (Goertzel), skipping the first eighth (filters settling).
    fn level(x: &[f32], hz: f32) -> f32 {
        let x = &x[x.len() / 8..];
        let w = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = f64::from(v) + 2.0 * w.cos() * s1 - s2;
            (s2, s1) = (s1, s);
        }
        let power = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
        (power.max(0.0).sqrt() * 2.0 / x.len() as f64) as f32
    }

    /// What `MusicAir` adds to a mono tone (both channels alike), one channel.
    fn added(hz: f32, amount: f32) -> Vec<f32> {
        let x = sine(hz, RATE as usize / 2, 0.5);
        let mut frames: Vec<[f32; 2]> = x.iter().map(|&s| [s, s]).collect();
        MusicAir::new(RATE, amount).process(&mut frames);
        frames.iter().zip(&x).map(|(f, s)| f[0] - s).collect()
    }

    #[test]
    fn the_pair_is_ninety_degrees_apart_and_equal_across_the_band() {
        for hz in [500.0f32, 4_000.0, 11_000.0, 14_000.0, 16_000.0, 20_000.0] {
            let mut h = Hilbert::new();
            let (i, q): (Vec<f32>, Vec<f32>) = sine(hz, 20_000, 0.5).into_iter().map(|x| h.run(x)).unzip();
            let (i, q) = (&i[4_000..], &q[4_000..]);
            let dot: f32 = i.iter().zip(q).map(|(a, b)| a * b).sum();
            let (ei, eq): (f32, f32) = (i.iter().map(|a| a * a).sum(), q.iter().map(|b| b * b).sum());
            let cos = dot / (ei * eq).sqrt();
            assert!(cos.abs() < 0.03, "{hz} Hz: cos of the split {cos}");
            assert!((ei / eq - 1.0).abs() < 0.01, "{hz} Hz: levels {ei} vs {eq}");
        }
    }

    /// What `MusicAir` adds to a mono signal (both channels alike), one channel, over its last
    /// half second (the filters and the level follower settled).
    fn added_to(x: &[f32], amount: f32) -> Vec<f32> {
        let mut frames: Vec<[f32; 2]> = x.iter().map(|&s| [s, s]).collect();
        MusicAir::new(RATE, amount).process(&mut frames);
        let tail = x.len() - RATE as usize / 2;
        frames[tail..].iter().zip(&x[tail..]).map(|(f, s)| f[0] - s).collect()
    }

    fn mix(tones: &[(f32, f32)], frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| tones.iter().map(|&(hz, amp)| amp * (std::f32::consts::TAU * hz * n as f32 / RATE as f32).sin()).sum())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn a_note_gets_its_own_octave_and_nothing_out_of_tune() {
        // A 440 Hz note with partials up to the songs' cutoff (in half a second, a whole number of
        // cycles of every harmonic — Goertzel sees each one alone).
        let partials: Vec<(f32, f32)> = (1..=36).map(|h| (440.0 * h as f32, 0.08 / (h as f32).sqrt())).collect();
        let air = added_to(&mix(&partials, RATE as usize * 3 / 2), MUSIC_AIR);
        let total = rms(&air).powi(2);
        assert!(total.sqrt() > 1e-4, "no new band at all ({})", total.sqrt());
        let on: f32 = (30..=50).map(|k| level(&air, 440.0 * k as f32).powi(2) / 2.0).sum();
        // The shift this replaced put 0 % of its band here: every partial moved by 5 kHz.
        assert!(on / total > 0.97, "{:.1} % of the new band on 440 Hz harmonics", 100.0 * on / total);
    }

    #[test]
    fn a_strong_line_above_the_band_folds_nothing_back() {
        // The orphanage song's 12.5 kHz line, and nothing in 8–9.8 kHz to double: doubled, it
        // would fold back from 25 kHz to an out-of-tune 19.1 kHz.
        let air = added_to(&mix(&[(12_500.0, 0.3), (14_200.0, 0.1), (440.0, 0.2)], RATE as usize * 3 / 2), MUSIC_AIR);
        assert!(rms(&air) < 0.3 * 1e-3, "added {} (> -60 dB under the line)", rms(&air));
        // Beside a note that is in the band: its octave, and the fold far under it.
        let air = added_to(&mix(&[(12_500.0, 0.3), (9_000.0, 0.05), (13_500.0, 0.05)], RATE as usize * 3 / 2), MUSIC_AIR);
        let octave = level(&air, 18_000.0);
        assert!(octave > 0.01, "18 kHz at {octave}");
        assert!(level(&air, 19_100.0) < 0.3 * 3e-4, "the fold at {} (> -70 dB under the line)", level(&air, 19_100.0));
    }

    #[test]
    fn the_body_of_the_music_gets_nothing() {
        for hz in [100.0f32, 1_000.0, 3_000.0, 5_000.0] {
            let out = added(hz, 1.0);
            let peak = out[out.len() / 4..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak < 0.5 * 1e-3, "{hz} Hz added {peak} (> -60 dB)");
        }
    }

    #[test]
    fn the_new_band_follows_the_songs_own_top() {
        let note = |top: f32, amount: f32| level(&added_to(&mix(&[(9_000.0, 0.1), (14_000.0, top)], RATE as usize * 3 / 2), amount), 18_000.0);
        // A note with a top gets its octave, in proportion to the amount.
        let some = note(0.2, MUSIC_AIR);
        assert!(some > 0.02, "18 kHz at {some}");
        assert!((note(0.2, MUSIC_AIR / 2.0) / some - 0.5).abs() < 0.02);
        // Twice the top, twice the air.
        assert!((note(0.4, MUSIC_AIR / 2.0) / some - 1.0).abs() < 0.05);
        // A dull note — nothing above 13 kHz — gets next to none.
        assert!(note(0.0, MUSIC_AIR) < 0.1 * 0.05, "a dull note's octave at {}", note(0.0, MUSIC_AIR));
        // A loud top over a faint note: never louder than what was doubled.
        let faint = level(&added_to(&mix(&[(9_000.0, 0.01), (14_000.0, 0.5)], RATE as usize * 3 / 2), MUSIC_AIR), 18_000.0);
        assert!(faint <= 0.01 * GAIN_MAX * 1.05, "{faint}");
    }

    #[test]
    fn the_side_channel_keeps_its_stereo_below_8k_and_loses_the_codec_fizz_above() {
        // Mid at 440 Hz; side: real stereo at 660 Hz plus codec noise as a 13 kHz tone.
        let n = RATE as usize;
        let mut frames: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let m = 0.5 * (std::f32::consts::TAU * 440.0 * t).sin();
                let s = 0.05 * (std::f32::consts::TAU * 660.0 * t).sin() + 0.02 * (std::f32::consts::TAU * 13_000.0 * t).sin();
                [m + s, m - s]
            })
            .collect();
        MusicAir::new(RATE, 0.0).process(&mut frames);
        let mid: Vec<f32> = frames.iter().map(|f| 0.5 * (f[0] + f[1])).collect();
        let side: Vec<f32> = frames.iter().map(|f| 0.5 * (f[0] - f[1])).collect();
        assert!((level(&mid, 440.0) - 0.5).abs() < 1e-3, "the mid is untouched");
        assert!((level(&side, 660.0) - 0.05).abs() < 1e-3, "the stereo stays");
        assert!(level(&side, 13_000.0) < 0.02 * 0.02, "the fizz is gone (> -34 dB)");
    }

    #[test]
    fn off_without_room_or_amount_and_silence_stays_silence() {
        for air in [MusicAir::new(32_000, MUSIC_AIR), MusicAir::new(RATE, 0.0)] {
            assert!(!air.is_on());
        }
        let mut quiet = vec![[0.0f32; 2]; 4_096];
        MusicAir::new(RATE, MUSIC_AIR).process(&mut quiet);
        assert!(quiet.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
    }
}
