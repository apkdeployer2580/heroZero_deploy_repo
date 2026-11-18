//! The music from the rate the game ships it at (44.1 kHz) to the rate the device plays.
//!
//! Phase 4c (2026-10-22). [`crate::sfx_upsample`] converts a whole effect at once; a song is
//! minutes long and decoded a packet at a time on the wie-music thread, so this is the same
//! band-limited interpolation in STREAMING form: input frames go in as they are decoded, output
//! frames come out as soon as the kernel has the input it needs, and the position carries over
//! from packet to packet — and across a loop, where the song simply goes on.
//!
//!   * A Kaiser-windowed sinc, [`TAPS`] taps at the input rate, tabulated at [`PHASES`]
//!     sub-sample positions with rows blended linearly between (the effect path's layout).
//!   * The cutoff sits at [`CUTOFF`] of the lower Nyquist, so a 44.1 kHz song played at 48 kHz
//!     gets nothing between 22.05 and 24 kHz (its mirror image would land there) and a device
//!     slower than the song does not fold the top back down. The game's AAC (128 kbps) has
//!     nothing above ~16 kHz to lose.
//!   * The step is the exact ratio of the two rates (147/160 for 44.1 → 48 kHz) in integers,
//!     so an hour of looping drifts by nothing.
//!   * Equal rates are a straight copy.
//!
//! No dependencies: pure DSP, host-tested.

/// Kernel half-width, in input frames.
const HALF: usize = 16;
pub const TAPS: usize = HALF * 2;
/// Sub-sample positions the kernel is tabulated at.
pub const PHASES: usize = 256;
/// Kaiser window shape: about −80 dB of stop band.
const KAISER_BETA: f64 = 8.0;
/// Where the pass band ends, as a fraction of the lower of the two Nyquist rates.
pub const CUTOFF: f64 = 0.88;

fn bessel_i0(x: f64) -> f64 {
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

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `PHASES + 1` rows of `TAPS` coefficients for a cutoff of `fc` (1.0 = the input's Nyquist).
/// Row `p`, tap `j` weighs the input frame `HALF - 1 - j + p / PHASES` frames before the output
/// position. Every row sums to 1, so a constant passes unchanged.
fn kernel(fc: f64) -> Vec<[f32; TAPS]> {
    let i0_beta = bessel_i0(KAISER_BETA);
    (0..=PHASES)
        .map(|p| {
            let mut row = [0.0f64; TAPS];
            for (j, tap) in row.iter_mut().enumerate() {
                let u = p as f64 / PHASES as f64 + (HALF - 1) as f64 - j as f64;
                let x = u / HALF as f64;
                if x.abs() >= 1.0 {
                    continue;
                }
                let a = std::f64::consts::PI * fc * u;
                let sinc = if a.abs() < 1e-12 { 1.0 } else { a.sin() / a };
                *tap = sinc * bessel_i0(KAISER_BETA * (1.0 - x * x).sqrt()) / i0_beta;
            }
            let sum: f64 = row.iter().sum();
            let mut out = [0.0f32; TAPS];
            for (o, tap) in out.iter_mut().zip(row) {
                *o = (tap / sum) as f32;
            }
            out
        })
        .collect()
}

/// A stereo stream from `from` Hz to `to` Hz.
pub struct Resampler {
    /// Input frames per output frame, as `num / den` in lowest terms.
    num: u64,
    den: u64,
    /// The output position's fraction of an input frame, in `den`ths.
    frac: u64,
    /// The output position's whole input frame, as an index into `hist`.
    pos: usize,
    /// The input still needed: from `HALF - 1` frames before `pos` on.
    hist: Vec<[f32; 2]>,
    table: Vec<[f32; TAPS]>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let (from, to) = (u64::from(from.max(1)), u64::from(to.max(1)));
        let g = gcd(from, to);
        let fc = CUTOFF * (to as f64 / from as f64).min(1.0);
        Self {
            num: from / g,
            den: to / g,
            frac: 0,
            // The first output lands on the first input frame: the kernel's past is silence.
            pos: HALF - 1,
            hist: vec![[0.0; 2]; HALF - 1],
            table: if from == to { Vec::new() } else { kernel(fc) },
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.table.is_empty()
    }

    /// Feed `input` (stereo frames at the input rate); append every output frame now complete
    /// to `out`, interleaved.
    pub fn process(&mut self, input: &[[f32; 2]], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend(input.iter().flatten());
            return;
        }
        self.hist.extend_from_slice(input);
        while self.pos + HALF < self.hist.len() {
            // Which tabulated row the position falls on, and how far towards the next.
            let at = self.frac * PHASES as u64;
            let row = (at / self.den) as usize;
            let t = (at % self.den) as f32 / self.den as f32;
            let (a, b) = (&self.table[row], &self.table[row + 1]);
            let window = &self.hist[self.pos + 1 - HALF..=self.pos + HALF];
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for ((x, wa), wb) in window.iter().zip(a).zip(b) {
                let w = wa + (wb - wa) * t;
                l += x[0] * w;
                r += x[1] * w;
            }
            out.push(l);
            out.push(r);
            self.frac += self.num;
            self.pos += (self.frac / self.den) as usize;
            self.frac %= self.den;
        }
        // Downsampling can step `pos` past the input that has arrived; it stays relative.
        let consumed = (self.pos + 1).saturating_sub(HALF).min(self.hist.len());
        self.hist.drain(..consumed);
        self.pos -= consumed;
    }

    /// The end of the stream: the last input frames, against silence after them.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if !self.is_passthrough() {
            self.process(&[[0.0; 2]; HALF], out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f32, frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|n| {
                let s = (std::f32::consts::TAU * hz * n as f32 / rate as f32).sin() * 0.5;
                [s, -s]
            })
            .collect()
    }

    /// Amplitude of `hz` in one channel of interleaved `out` (Goertzel), skipping the edges.
    fn level(out: &[f32], rate: u32, hz: f32, ch: usize) -> f32 {
        let x: Vec<f32> = out.chunks_exact(2).map(|f| f[ch]).collect();
        let x = &x[x.len() / 8..x.len() * 7 / 8];
        let w = std::f64::consts::TAU * f64::from(hz) / f64::from(rate);
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = f64::from(v) + 2.0 * w.cos() * s1 - s2;
            (s2, s1) = (s1, s);
        }
        let power = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
        (power.sqrt() * 2.0 / x.len() as f64) as f32
    }

    #[test]
    fn equal_rates_are_a_straight_copy() {
        let mut rs = Resampler::new(44_100, 44_100);
        assert!(rs.is_passthrough());
        let mut out = Vec::new();
        rs.process(&[[0.25, -0.5], [1.0, 0.0]], &mut out);
        rs.flush(&mut out);
        assert_eq!(out, vec![0.25, -0.5, 1.0, 0.0]);
    }

    #[test]
    fn a_second_of_song_is_a_second_of_device_audio() {
        for (from, to) in [(44_100, 48_000), (44_100, 96_000), (48_000, 44_100), (44_100, 32_000)] {
            let mut rs = Resampler::new(from, to);
            let mut out = Vec::new();
            // In uneven packets, as a decoder hands them over.
            let input = sine(from, 440.0, from as usize);
            for chunk in input.chunks(1024 + 7) {
                rs.process(chunk, &mut out);
            }
            rs.flush(&mut out);
            let frames = out.len() / 2;
            assert!(frames.abs_diff(to as usize) <= 1, "{from} -> {to}: {frames} frames");
        }
    }

    #[test]
    fn the_pass_band_arrives_flat_on_both_channels() {
        for hz in [100.0f32, 1_000.0, 8_000.0, 15_000.0] {
            let mut out = Vec::new();
            Resampler::new(44_100, 48_000).process(&sine(44_100, hz, 44_100), &mut out);
            let (l, r) = (level(&out, 48_000, hz, 0), level(&out, 48_000, hz, 1));
            assert!((l - 0.5).abs() < 0.01, "{hz} Hz left {l}");
            assert!((r - 0.5).abs() < 0.01, "{hz} Hz right {r}");
        }
    }

    #[test]
    fn the_mirror_image_above_the_song_band_is_gone() {
        // A 21 kHz tone at 44.1 kHz has its image at 23.1 kHz — inside a 48 kHz device's band.
        let mut rs = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        rs.process(&sine(44_100, 21_000.0, 44_100), &mut out);
        let image = level(&out, 48_000, 23_100.0, 0);
        assert!(image < 0.5 * 0.01, "image at {image} (> -40 dB)");
        // A tone the AAC actually carries is not where the filter works.
        out.clear();
        let mut rs = Resampler::new(44_100, 48_000);
        rs.process(&sine(44_100, 16_000.0, 44_100), &mut out);
        assert!(level(&out, 48_000, 16_000.0, 0) > 0.5 * 0.95);
    }

    #[test]
    fn the_stream_is_seamless_across_packets() {
        // One pass in one call and the same input in 300 small calls give the same output.
        let input = sine(44_100, 1_234.0, 20_000);
        let (mut a, mut b) = (Resampler::new(44_100, 48_000), Resampler::new(44_100, 48_000));
        let (mut whole, mut parts) = (Vec::new(), Vec::new());
        a.process(&input, &mut whole);
        for chunk in input.chunks(67) {
            b.process(chunk, &mut parts);
        }
        assert_eq!(whole.len(), parts.len());
        assert!(whole.iter().zip(&parts).all(|(x, y)| (x - y).abs() < 1e-6));
    }

    #[test]
    fn every_row_sums_to_one() {
        for fc in [CUTOFF, CUTOFF * 32_000.0 / 44_100.0] {
            for row in kernel(fc) {
                let sum: f32 = row.iter().sum();
                assert!((sum - 1.0).abs() < 1e-5, "{sum}");
            }
        }
    }
}
