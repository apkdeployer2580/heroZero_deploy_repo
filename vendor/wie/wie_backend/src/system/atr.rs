// hero2's sound effects as the Yamaha MA-2 decodes them — SMAF `ATR` audio tracks.
//
// ★ 2026-10-02 ("효과음이 툭툭 끊기는 느낌"). Every hero2 effect (jar `snd/00`, `P/snd/40..53`
// but 42) is one `ATR` track: `AspI` · `Atsq` (a sequence that plays wave 1 once) · `Awa<n>`
// (4-bit Yamaha ADPCM, mono, 8 kHz — 46 is 4 kHz). `smaf_player` decodes that with the
// ADPCM-B routine of another chip family (Y8950: HIGH nibble first, step table `57 57 57 57
// 77 102 128 153 >> 6`). The MA-2's ADPCM is the other Yamaha one — LOW nibble first, step
// table `230 230 230 230 307 409 512 614 >> 8` — and with the nibbles swapped the predictor
// runs away: effect 40 decoded to a mean of −21,694 and ended at −22,950, i.e. one thump as
// the clip starts and another as it ends, with the sound itself a quiet rustle on top.
// Decoded as below, all eleven files come out centred (|mean| < 300) and end on silence
// (|last| < 1,400), which is what a recorded sample looks like and the other three
// combinations of nibble order and table do not (`HERO2_GUEST_RE.md` §8.43).
//
// Strict on purpose, like `ma2.rs`: a file this does not read exactly returns `None` and the
// caller keeps the `smaf_player` decode. Only `alloc` — tools `include!` this file.

use alloc::vec::Vec;

/// One wave an `ATR` sequence plays: mono 16-bit PCM at `sampling_rate`.
#[derive(Debug, PartialEq, Eq)]
pub struct AtrWave {
    pub sampling_rate: u32,
    pub data: Vec<i16>,
}

const ADPCM_STEP_MIN: i32 = 127;
const ADPCM_STEP_MAX: i32 = 24576;
/// Step size after a nibble of each magnitude, in 1/256.
const ADPCM_STEP_SCALE: [i32; 8] = [230, 230, 230, 230, 307, 409, 512, 614];

/// Yamaha 4-bit ADPCM as SMAF carries it: two samples a byte, the LOW nibble first; bit 3 is
/// the sign, bits 2-0 the magnitude `m` of a step of `(2m + 1) / 8` of the step size.
pub fn decode_yamaha_adpcm(data: &[u8]) -> Vec<i16> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut value = 0i32;
    let mut step = ADPCM_STEP_MIN;
    for byte in data {
        for nibble in [byte & 0x0f, byte >> 4] {
            let magnitude = (nibble & 7) as i32;
            let diff = (2 * magnitude + 1) * step / 8;
            value = if nibble & 8 != 0 { value - diff } else { value + diff };
            value = value.clamp(i16::MIN as i32, i16::MAX as i32);
            step = ((step * ADPCM_STEP_SCALE[magnitude as usize]) >> 8).clamp(ADPCM_STEP_MIN, ADPCM_STEP_MAX);
            out.push(value as i16);
        }
    }
    out
}

fn atr_be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// `(tag, body)` of each chunk in `b`; `None` if one runs past the end.
fn atr_chunks(b: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < b.len() {
        let tag = b.get(at..at + 4)?;
        let len = atr_be32(b, at + 4)? as usize;
        let body = b.get(at + 8..(at + 8).checked_add(len)?)?;
        out.push(([tag[0], tag[1], tag[2], tag[3]], body));
        at += 8 + len;
    }
    Some(out)
}

/// A variable-length number: bytes until one without bit 7. Only skipped — the order of the
/// wave messages is all this parser takes from a sequence.
fn atr_skip_var(d: &[u8], at: &mut usize) -> Option<()> {
    loop {
        let b = *d.get(*at)?;
        *at += 1;
        if b & 0x80 == 0 {
            return Some(());
        }
    }
}

/// `Atsq`: the wave numbers in the order the sequence plays them.
fn atr_sequence(d: &[u8]) -> Option<Vec<u8>> {
    let mut waves = Vec::new();
    let mut at = 0usize;
    loop {
        let rest = d.get(at..)?;
        if rest == [0, 0, 0, 0] {
            return Some(waves);
        }
        atr_skip_var(d, &mut at)?; // duration
        let first = *d.get(at)?;
        at += 1;
        match first {
            0x00 => {
                let kind = *d.get(at)? & 0x3f;
                at += 1;
                match kind {
                    // NOP · short expression · short pitch bend
                    0x00 | 0x01..=0x0e | 0x11..=0x1e => {}
                    // pitch bend · expression · volume · pan, one value byte each
                    0x34 | 0x36 | 0x37 | 0x3a | 0x3b => at += 1,
                    _ => return None,
                }
            }
            0xff => match *d.get(at)? {
                0x00 => at += 1,
                0xf0 => at += 2 + *d.get(at + 1)? as usize,
                _ => return None,
            },
            wave => {
                waves.push(wave & 0x3f);
                atr_skip_var(d, &mut at)?; // gate time
            }
        }
    }
}

/// Every wave the file's `ATR` tracks play, in playing order (track by track, then by the
/// track's sequence) — the order `smaf_player` emits its `Wave` events in.
///
/// `None` for anything but Handy Phone Standard tracks of mono 4-bit ADPCM, and for a file
/// with no `ATR` track at all.
pub fn parse_atr_waves(file: &[u8]) -> Option<Vec<AtrWave>> {
    if file.get(0..4)? != b"MMMD" {
        return None;
    }
    let len = atr_be32(file, 4)? as usize;
    // The last two bytes of the MMMD body are its CRC.
    let body = file.get(8..(8usize).checked_add(len)?)?;
    let body = body.get(..body.len().checked_sub(2)?)?;

    let mut out = Vec::new();
    let mut tracks = 0;
    for (tag, chunk) in atr_chunks(body)? {
        let [b'A', b'T', b'R', _] = tag else { continue };
        tracks += 1;
        // format type 0 · sequence type 0 · wave type: mono, ADPCM, 4-bit.
        if chunk.len() < 6 || chunk[0] != 0 || chunk[1] != 0 {
            return None;
        }
        let wave_type = u16::from_be_bytes([chunk[2], chunk[3]]);
        if wave_type & 0xf0ff != 0x1000 {
            return None;
        }
        let sampling_rate = match (wave_type >> 8) & 0x0f {
            0 => 4000,
            1 => 8000,
            _ => return None,
        };
        let mut sequence = None;
        let mut data = Vec::new();
        for (sub, bytes) in atr_chunks(&chunk[6..])? {
            match sub {
                [b'A', b's', b'p', b'I'] | [b'A', b't', b's', b'u'] => {}
                [b'A', b't', b's', b'q'] => sequence = Some(atr_sequence(bytes)?),
                [b'A', b'w', b'a', n] => data.push((n, bytes)),
                _ => return None,
            }
        }
        for number in sequence? {
            let (_, bytes) = data.iter().find(|(n, _)| *n == number)?;
            out.push(AtrWave {
                sampling_rate,
                data: decode_yamaha_adpcm(bytes),
            });
        }
    }
    if tracks == 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use super::{ADPCM_STEP_MAX, ADPCM_STEP_MIN, ADPCM_STEP_SCALE, AtrWave, decode_yamaha_adpcm, parse_atr_waves};

    /// The matching encoder: for each sample the nibble whose step lands nearest, state kept
    /// exactly as the decoder keeps it. Packed low nibble first.
    fn encode(samples: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut value, mut step) = (0i32, ADPCM_STEP_MIN);
        let mut low = None;
        for &s in samples {
            let delta = s as i32 - value;
            let magnitude = ((delta.abs() * 4 / step).min(7)) as usize;
            let nibble = magnitude as u8 | if delta < 0 { 8 } else { 0 };
            let diff = (2 * magnitude as i32 + 1) * step / 8;
            value = (if delta < 0 { value - diff } else { value + diff }).clamp(-32768, 32767);
            step = ((step * ADPCM_STEP_SCALE[magnitude]) >> 8).clamp(ADPCM_STEP_MIN, ADPCM_STEP_MAX);
            match low.take() {
                None => low = Some(nibble),
                Some(l) => out.push(l | (nibble << 4)),
            }
        }
        out
    }

    /// A decaying 500 Hz burst at 8 kHz — the shape of an effect: loud start, silent end.
    fn burst() -> Vec<i16> {
        (0..800)
            .map(|n| {
                let t = n as f32 / 8000.0;
                (12000.0 * (-t * 70.0).exp() * (2.0 * core::f32::consts::PI * 500.0 * t).sin()) as i16
            })
            .collect()
    }

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    /// An effect file shaped like hero2's: `CNTI`, one `ATR` with `AspI` · `Atsq` · `Awa<n>`.
    fn effect(wave_type: [u8; 2], sequence: &[u8], waves: &[(u8, &[u8])]) -> Vec<u8> {
        let mut atr = vec![0, 0, wave_type[0], wave_type[1], 2, 2];
        atr.extend(chunk(b"AspI", b"st:\0\0\0\0,sp:\0\0\0\x0a,"));
        atr.extend(chunk(b"Atsq", sequence));
        for (n, data) in waves {
            atr.extend(chunk(&[b'A', b'w', b'a', *n], data));
        }
        let mut body = chunk(b"CNTI", &[0, 1, 2, 0, 0]);
        body.extend(chunk(b"ATR\0", &atr));
        body.extend_from_slice(&[0, 0]); // CRC
        chunk(b"MMMD", &body)
    }

    /// hero2's effect 40: volume 127, wave 1 for 60 ticks, a NOP, the end.
    const PLAY_WAVE_1: [u8; 14] = [0x01, 0x00, 0x37, 0x7f, 0x01, 0x01, 0x3c, 0x3d, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00];

    #[test]
    fn decodes_low_nibble_first_with_the_ma2_step_table() {
        // 0x70: low nibble 0 → +127/8 = +15 (step 127·230>>8 = 114, held at 127), then high
        // nibble 7 → +15·127/8 = +238 (step 127·614>>8 = 304).
        // 0x08: low nibble 8 → −304/8 = −38 (step 304·230>>8 = 273), then high nibble 0 →
        // +273/8 = +34.
        assert_eq!(decode_yamaha_adpcm(&[0x70, 0x08]), [15, 253, 215, 249]);
    }

    #[test]
    fn a_recorded_burst_comes_back_centred_and_ends_on_silence() {
        let source = burst();
        let decoded = decode_yamaha_adpcm(&encode(&source));
        assert_eq!(decoded.len(), source.len());
        let mean = decoded.iter().map(|s| *s as i64).sum::<i64>() / decoded.len() as i64;
        assert!(mean.abs() < 200, "mean {mean}");
        assert!(decoded.last().unwrap().abs() < 300, "last {:?}", decoded.last());
        let err = source.iter().zip(&decoded).map(|(a, b)| ((*a as i64) - (*b as i64)).pow(2)).sum::<i64>() / source.len() as i64;
        let power = source.iter().map(|a| (*a as i64).pow(2)).sum::<i64>() / source.len() as i64;
        assert!(power > err * 20, "signal {power} over error {err} is under 13 dB");
    }

    #[test]
    fn the_other_chips_nibble_order_runs_away_on_the_same_bytes() {
        // What `smaf_player` did to hero2's effects: the same bytes, high nibble first.
        let swapped: Vec<u8> = encode(&burst()).iter().map(|b| b.rotate_left(4)).collect();
        let decoded = decode_yamaha_adpcm(&swapped);
        let mean = decoded.iter().map(|s| *s as i64).sum::<i64>() / decoded.len() as i64;
        assert!(
            mean.abs() > 400 || decoded.last().unwrap().abs() > 400,
            "mean {mean}, last {:?}",
            decoded.last()
        );
    }

    #[test]
    fn reads_an_effect_file() {
        let data = encode(&burst());
        let waves = parse_atr_waves(&effect([0x11, 0x00], &PLAY_WAVE_1, &[(1, &data)])).unwrap();
        assert_eq!(
            waves,
            [AtrWave {
                sampling_rate: 8000,
                data: decode_yamaha_adpcm(&data)
            }]
        );
        // 4 kHz (hero2's effect 46), and a two-byte gate time.
        let long_gate = [0x01, 0x01, 0x80, 0x35, 0x80, 0x36, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00];
        let waves = parse_atr_waves(&effect([0x10, 0x00], &long_gate, &[(1, &data)])).unwrap();
        assert_eq!(waves[0].sampling_rate, 4000);
    }

    #[test]
    fn plays_waves_in_sequence_order_by_number() {
        let (a, b) = (encode(&burst()), [0x70u8, 0x08]);
        let sequence = [0x00, 0x02, 0x10, 0x10, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00];
        let waves = parse_atr_waves(&effect([0x11, 0x00], &sequence, &[(1, &a), (2, &b)])).unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].data, decode_yamaha_adpcm(&b));
        assert_eq!(waves[1].data, decode_yamaha_adpcm(&a));
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        let data = encode(&burst());
        // PCM, stereo, 8-bit, 11 kHz: not this decoder's.
        for wave_type in [[0x01, 0x00], [0x91, 0x00], [0x11, 0x10], [0x12, 0x00]] {
            assert!(
                parse_atr_waves(&effect(wave_type, &PLAY_WAVE_1, &[(1, &data)])).is_none(),
                "{wave_type:?}"
            );
        }
        // A wave number with no data, an unknown control, a truncated sequence.
        assert!(parse_atr_waves(&effect([0x11, 0x00], &PLAY_WAVE_1, &[(2, &data)])).is_none());
        assert!(parse_atr_waves(&effect([0x11, 0x00], &[0x01, 0x00, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00], &[(1, &data)])).is_none());
        assert!(parse_atr_waves(&effect([0x11, 0x00], &PLAY_WAVE_1[..9], &[(1, &data)])).is_none());
        // A score has no audio track.
        let mut body = chunk(b"CNTI", &[0; 5]);
        body.extend(chunk(b"MTR\x01", &[0; 6]));
        body.extend_from_slice(&[0, 0]);
        assert!(parse_atr_waves(&chunk(b"MMMD", &body)).is_none());
        assert!(parse_atr_waves(b"RIFF").is_none());
    }
}
