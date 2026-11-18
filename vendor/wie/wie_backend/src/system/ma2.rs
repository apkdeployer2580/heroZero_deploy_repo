// hero2's songs as the Yamaha MA-2 plays them — SMAF **Handy Phone Standard** scores parsed
// for the FM path.
//
// ★ 2026-10-01 (`HERO2_BGM_FM_PLAN.md` §3 B1, facts in §1 and `HERO2_GUEST_RE.md` §8.37).
// All 18 hero2 songs are HPS: `MMMD` › `CNTI` › `MTR1..4` (format 0, four channels per track,
// logical channel = `track × 4 + ch`, 4 ms ticks), and track 1 carries the song's OWN FM voices
// in `Mtsu`. `smaf_player` turns a score into General MIDI and loses exactly what an FM synth
// needs: it drops `Mtsu` (it does not know `FF F0`), plays a drum channel's program number as
// the key instead of its pitch, and folds the sixteen channels onto GM channels. This parser
// keeps all three. It does no synthesis — the voices go out as the bytes the song carries.
//
// Strict on purpose: anything outside what the 18 songs use (another chunk kind, an unknown
// control, a voice that is not `43 03 …` of the expected length) returns `None`, and the caller
// plays the file the old way. A song this cannot read must never become a silent one.
//
// Only `alloc` — tools and selftests `include!` this file.

use alloc::vec::Vec;

/// Four channels per score track, at most four tracks.
pub const MA2_CHANNELS: usize = 16;

/// One FM voice from a song's `Mtsu`: `FF F0 12 43 03 <n> <bank> <program> <data>`, where
/// `data` = `g0 g1` + two 5-byte operators, exactly as the file has them (`g0` = LFO·FB·ALG,
/// operator = MULT·VIB·EGT·SUS·KSR | RR·DR | AR·SL | TL·KSL | DVB·DAM·AM·WS). Bank `0x80` is
/// a drum voice, its program the GM drum key it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ma2Voice {
    pub bank: u8,
    pub program: u8,
    pub data: [u8; 12],
}

/// One score event on a logical channel (`0..16`), in the order a synth must apply it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ma2Event {
    /// The channel's voice from here on: the song's voice with this bank and program. A note
    /// takes the voice its channel has when it starts.
    Program {
        channel: u8,
        bank: u8,
        program: u8,
    },
    Volume {
        channel: u8,
        value: u8,
    },
    Expression {
        channel: u8,
        value: u8,
    },
    /// `64` = centre.
    Pan {
        channel: u8,
        value: u8,
    },
    /// 14-bit, `0x2000` = centre.
    PitchBend {
        channel: u8,
        value: u16,
    },
    Modulation {
        channel: u8,
        value: u8,
    },
    /// Key off — for this note only; a channel that has moved on to another note ignores it.
    NoteOff {
        channel: u8,
        note: u8,
    },
    /// Key on. `note` is a MIDI note number, octave shift applied. A channel is monophonic
    /// (the songs never overlap two notes on one): a new note re-strikes the channel.
    NoteOn {
        channel: u8,
        note: u8,
    },
    /// One pass of the song ends here; the loop starts over.
    End,
}

impl Ma2Event {
    /// Order among events at the same millisecond: voice and levels first, a note's end
    /// before the next note's start, the song's end last.
    fn rank(&self) -> u8 {
        match self {
            Self::Program { .. } => 0,
            Self::Volume { .. } | Self::Expression { .. } | Self::Pan { .. } | Self::PitchBend { .. } | Self::Modulation { .. } => 1,
            Self::NoteOff { .. } => 2,
            Self::NoteOn { .. } => 3,
            Self::End => 4,
        }
    }
}

/// A parsed song: its voices, its events in milliseconds, one pass long.
#[derive(Debug, Default)]
pub struct Ma2Song {
    /// Every `Mtsu` voice, in file order (a later one with the same bank and program wins).
    pub voices: Vec<Ma2Voice>,
    /// Sorted by time and [`Ma2Event::rank`]; the last is always `End` at `len_ms`.
    pub events: Vec<(u32, Ma2Event)>,
    pub len_ms: u32,
    /// Bit n = logical channel n is declared rhythm (channel type 3). Information only: a drum
    /// voice is an ordinary FM voice played at its note's pitch.
    pub rhythm: u16,
    /// `FF F0` exclusives written while the song plays (song 14's `43 03 90 …`, not understood
    /// yet) — skipped, counted.
    pub exclusive_ignored: u32,
}

impl Ma2Song {
    /// The voice a `Program` event names, if the song carries it.
    pub fn voice(&self, bank: u8, program: u8) -> Option<&Ma2Voice> {
        self.voices.iter().rev().find(|v| v.bank == bank && v.program == program)
    }
}

const SHORT_EXPRESSION: [u8; 15] = [0x00, 0x00, 0x1f, 0x27, 0x2f, 0x37, 0x3f, 0x47, 0x4f, 0x57, 0x5f, 0x67, 0x6f, 0x77, 0x7f];
const SHORT_MODULATION: [u8; 15] = [0x00, 0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50, 0x60, 0x70, 0x7f];

/// `Mtsu` voice length: `43 03 n bank program` + `g0 g1` + 10 operator bytes + `F7`.
const VOICE_LEN: usize = 18;

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// `(tag, body)` of each chunk in `b`; `None` if one runs past the end.
fn chunks(b: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < b.len() {
        let tag = b.get(at..at + 4)?;
        let len = be32(b, at + 4)? as usize;
        let body = b.get(at + 8..(at + 8).checked_add(len)?)?;
        out.push(([tag[0], tag[1], tag[2], tag[3]], body));
        at += 8 + len;
    }
    Some(out)
}

/// Milliseconds per tick for a time-base byte.
fn timebase(raw: u8) -> Option<u32> {
    Some(match raw {
        0x00 => 1,
        0x01 => 2,
        0x02 => 4,
        0x03 => 5,
        0x10 => 10,
        0x11 => 20,
        0x12 => 40,
        0x13 => 50,
        _ => return None,
    })
}

/// HPS variable length: one byte `< 0x80`, or `((b0 & 0x7f) + 1) << 7 | b1` with `b1 < 0x80`.
fn var(d: &[u8], at: &mut usize) -> Option<u32> {
    let b0 = *d.get(*at)?;
    *at += 1;
    if b0 & 0x80 == 0 {
        return Some(b0 as u32);
    }
    let b1 = *d.get(*at)?;
    *at += 1;
    if b1 & 0x80 != 0 {
        return None;
    }
    Some((((b0 & 0x7f) as u32 + 1) << 7) | b1 as u32)
}

fn byte(d: &[u8], at: &mut usize) -> Option<u8> {
    let b = *d.get(*at)?;
    *at += 1;
    Some(b)
}

/// `Mtsu`: nothing but `FF F0 <len> <voice>` records.
fn parse_voices(d: &[u8], out: &mut Vec<Ma2Voice>) -> Option<()> {
    let mut at = 0usize;
    while at < d.len() {
        if byte(d, &mut at)? != 0xff || byte(d, &mut at)? != 0xf0 {
            return None;
        }
        // ⛔ By the length byte — never by searching for F7: a voice's own bytes can hold one
        // (song 02: `14 62 f7 00 a0`).
        let len = byte(d, &mut at)? as usize;
        let p = d.get(at..at + len)?;
        at += len;
        if len != VOICE_LEN || p[0] != 0x43 || p[1] != 0x03 || p[VOICE_LEN - 1] != 0xf7 {
            return None;
        }
        let mut data = [0u8; 12];
        data.copy_from_slice(&p[5..17]);
        out.push(Ma2Voice {
            bank: p[3],
            program: p[4],
            data,
        });
    }
    Some(())
}

/// `MspI`: `st:` u32 `,` `sp:` u32 `,` — start and stop offsets into `Mtsq`.
fn parse_mspi(d: &[u8]) -> Option<(u32, u32)> {
    let (mut st, mut sp) = (None, None);
    let mut at = 0usize;
    while at < d.len() {
        let key = d.get(at..at + 2)?;
        if d.get(at + 2) != Some(&b':') {
            return None;
        }
        let comma = at + 3 + d.get(at + 3..)?.iter().position(|&b| b == b',')?;
        let val = &d[at + 3..comma];
        if val.len() == 4 {
            let v = u32::from_be_bytes([val[0], val[1], val[2], val[3]]);
            match key {
                b"st" => st = Some(v),
                b"sp" => sp = Some(v),
                _ => {}
            }
        }
        at = comma + 1;
    }
    Some((st?, sp?))
}

/// One track's `Mtsq`. Returns the byte offset of its end-of-sequence and the time it lands on.
fn parse_sequence(d: &[u8], base: u8, tbd: u32, tbg: u32, song: &mut Ma2Song) -> Option<(usize, u32)> {
    let mut at = 0usize;
    let mut now: u32 = 0;
    let mut octave = [0i8; 4];
    let mut bank = [0u8; 4];
    loop {
        if d.get(at..at + 4)? == [0, 0, 0, 0] {
            return Some((at, now));
        }
        now = now.checked_add(var(d, &mut at)?.checked_mul(tbd)?)?;
        match byte(d, &mut at)? {
            0x00 => {
                let b = byte(d, &mut at)?;
                let ch = b >> 6;
                let channel = base + ch;
                let ev = match b & 0x3f {
                    0x00 => {
                        byte(d, &mut at)?;
                        None
                    }
                    t @ 0x01..=0x0e => Some(Ma2Event::Expression {
                        channel,
                        value: SHORT_EXPRESSION[t as usize],
                    }),
                    t @ 0x11..=0x1e => Some(Ma2Event::PitchBend {
                        channel,
                        value: ((t as u16 - 0x10) * 1024).min(0x3fff),
                    }),
                    t @ 0x21..=0x2e => Some(Ma2Event::Modulation {
                        channel,
                        value: SHORT_MODULATION[(t - 0x20) as usize],
                    }),
                    t => {
                        let v = byte(d, &mut at)?;
                        match t {
                            0x30 => Some(Ma2Event::Program {
                                channel,
                                bank: bank[ch as usize],
                                program: v,
                            }),
                            0x31 => {
                                bank[ch as usize] = v;
                                None
                            }
                            0x32 => {
                                match v {
                                    0x00..=0x04 => octave[ch as usize] = v as i8,
                                    0x81..=0x84 => octave[ch as usize] = -((v - 0x80) as i8),
                                    _ => {}
                                }
                                None
                            }
                            0x33 => Some(Ma2Event::Modulation { channel, value: v }),
                            0x34 => Some(Ma2Event::PitchBend {
                                channel,
                                value: (8192 + (v as i32 - 128) * 64).clamp(0, 0x3fff) as u16,
                            }),
                            0x36 | 0x3b => Some(Ma2Event::Expression { channel, value: v & 0x7f }),
                            0x37 => Some(Ma2Event::Volume { channel, value: v & 0x7f }),
                            0x3a => Some(Ma2Event::Pan { channel, value: v & 0x7f }),
                            _ => return None,
                        }
                    }
                };
                if let Some(ev) = ev {
                    song.events.push((now, ev));
                }
            }
            0xff => match byte(d, &mut at)? {
                0x00 => {}
                0xf0 => {
                    // Skipped by its length, like a voice. See `parse_voices`.
                    let len = byte(d, &mut at)? as usize;
                    d.get(at..at + len)?;
                    at += len;
                    song.exclusive_ignored += 1;
                }
                _ => return None,
            },
            st => {
                let ch = st >> 6;
                let key = (((st >> 4) & 3) * 12 + (st & 0x0f)) as i32;
                let note = (key + 36 + 12 * octave[ch as usize] as i32).clamp(0, 127) as u8;
                let gate = var(d, &mut at)?.checked_mul(tbg)?;
                let channel = base + ch;
                song.events.push((now, Ma2Event::NoteOn { channel, note }));
                // A zero gate would sort its key-off in front of its key-on and hold the note
                // forever; none of the songs has one, but a short strike is the safe reading.
                song.events.push((now.checked_add(gate.max(1))?, Ma2Event::NoteOff { channel, note }));
            }
        }
    }
}

/// Parse an HPS song, or `None` if the file is anything else (or anything this does not know
/// how to play faithfully) — the caller then plays it the General MIDI way.
pub fn parse_ma2(file: &[u8]) -> Option<Ma2Song> {
    if file.get(0..4)? != b"MMMD" {
        return None;
    }
    let len = be32(file, 4)? as usize;
    // The last two bytes of the MMMD body are its CRC.
    let body = file.get(8..(8usize).checked_add(len)?)?;
    let body = body.get(..body.len().checked_sub(2)?)?;

    let mut song = Ma2Song::default();
    let mut tracks = 0u8;
    for (tag, chunk) in chunks(body)? {
        match &tag {
            b"CNTI" | b"OPDA" => {}
            [b'M', b'T', b'R', _] => {
                if tracks as usize * 4 >= MA2_CHANNELS || chunk.len() < 6 || chunk[0] != 0x00 {
                    return None;
                }
                let base = tracks * 4;
                tracks += 1;
                let (tbd, tbg) = (timebase(chunk[2])?, timebase(chunk[3])?);
                let status = u16::from_be_bytes([chunk[4], chunk[5]]);
                for ch in 0..4 {
                    if (status >> (12 - 4 * ch)) & 3 == 3 {
                        song.rhythm |= 1 << (base + ch);
                    }
                }
                let mut stop = None;
                let mut end = None;
                for (sub, data) in chunks(&chunk[6..])? {
                    match &sub {
                        b"MspI" => {
                            let (st, sp) = parse_mspi(data)?;
                            if st != 0 {
                                return None;
                            }
                            stop = Some(sp);
                        }
                        b"Mtsu" => parse_voices(data, &mut song.voices)?,
                        b"Mtsq" => end = Some(parse_sequence(data, base, tbd, tbg, &mut song)?),
                        _ => return None,
                    }
                }
                let (eos, end_ms) = end?;
                if stop.is_some_and(|sp| sp as usize != eos) {
                    return None;
                }
                song.len_ms = song.len_ms.max(end_ms);
            }
            _ => return None,
        }
    }
    if tracks == 0 || song.len_ms == 0 {
        return None;
    }
    song.events.sort_by_key(|(t, e)| (*t, e.rank()));
    song.events.push((song.len_ms, Ma2Event::End));
    Some(song)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{Ma2Event, parse_ma2};

    /// `tag` + big-endian length + body.
    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// `MMMD` around `CNTI` + the given score tracks, with a (dummy) CRC.
    fn smaf(tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut body = chunk(b"CNTI", &[0, 1, 1, 0, 0]);
        for t in tracks {
            body.extend_from_slice(t);
        }
        body.extend_from_slice(&[0x12, 0x34]);
        chunk(b"MMMD", &body)
    }

    fn mspi(sp: u32) -> Vec<u8> {
        let mut v = b"st:".to_vec();
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b",sp:");
        v.extend_from_slice(&sp.to_be_bytes());
        v.push(b',');
        chunk(b"MspI", &v)
    }

    /// An HPS track (4 ms ticks) with its `MspI` pointing at the sequence's end marker.
    fn track(n: u8, status: u16, voices: &[u8], seq: &[u8]) -> Vec<u8> {
        let mut body = vec![0x00, 0x00, 0x02, 0x02];
        body.extend_from_slice(&status.to_be_bytes());
        body.extend_from_slice(&mspi(seq.len() as u32));
        if !voices.is_empty() {
            body.extend_from_slice(&chunk(b"Mtsu", voices));
        }
        let mut s = seq.to_vec();
        s.extend_from_slice(&[0, 0, 0, 0]);
        body.extend_from_slice(&chunk(b"Mtsq", &s));
        chunk(&[b'M', b'T', b'R', n], &body)
    }

    /// A voice record whose data holds an `F7` byte, like song 02's first voice.
    fn voice(bank: u8, program: u8) -> Vec<u8> {
        let mut v = vec![0xff, 0xf0, 18, 0x43, 0x03, 0x00, bank, program, 0x59, 0x01];
        v.extend_from_slice(&[0x14, 0x52, 0xf4, 0x3a, 0xa0, 0x14, 0x62, 0xf7, 0x00, 0xa0]);
        v.push(0xf7);
        v
    }

    use alloc::vec;

    #[test]
    fn notes_controls_and_timing() {
        let mut voices = voice(0x00, 89);
        voices.extend(voice(0x80, 36));
        let seq = [
            0x00, 0x00, 0x31, 0x00, // bank 0 on ch0
            0x00, 0x00, 0x30, 89, // program 89
            0x00, 0x00, 0x37, 100, // volume
            0x00, 0x00, 0x3b, 90, // expression (long)
            0x00, 0x00, 0x05, // expression (short, index 5 = 0x37)
            0x00, 0x00, 0x32, 0x81, // octave shift −1
            // note ch0 oct1 nnnn 4 (E) — gate 2 ticks = 8 ms
            0x00, 0x14, 0x02, // 0x80 + 0x01 = two-byte duration: ((0 + 1) << 7) | 1 = 129 ticks = 516 ms
            0x80, 0x01, 0x00, 0x7a, 64, // pan on ch1 (0x40 | 0x3a)
            0x00, 0x00, 0x71, 0x80, // bank 0x80 on ch1
            0x00, 0x00, 0x70, 36, // program 36 on ch1
            0x00, 0x7c, 0x10, // ch1 oct3 nnnn 12 (C) gate 16 ticks
            0x05, 0xff, 0x00, // FF 00: 5 ticks of nothing (track end padding)
        ];
        let file = smaf(&[track(1, 0x2a32, &voices, &seq)]);
        let song = parse_ma2(&file).expect("parses");

        assert_eq!(song.voices.len(), 2);
        assert_eq!(
            (song.voices[0].data[9], song.voices[0].data[11]),
            (0xf7, 0xa0),
            "the F7 inside a voice is data, not its end"
        );
        assert_eq!(song.voice(0x80, 36).map(|v| v.bank), Some(0x80));
        assert_eq!(song.rhythm, 0b0100, "channel 2's type nibble is 3");

        // E, octave block 1, shifted down an octave: 4 + 12 + 36 − 12.
        assert!(song.events.contains(&(0, Ma2Event::NoteOn { channel: 0, note: 40 })));
        assert!(song.events.contains(&(8, Ma2Event::NoteOff { channel: 0, note: 40 })));
        assert!(song.events.contains(&(0, Ma2Event::Expression { channel: 0, value: 0x37 })));
        assert!(song.events.contains(&(
            0,
            Ma2Event::Program {
                channel: 0,
                bank: 0,
                program: 89
            }
        )));
        // The two-byte duration, and ch1's bank reaching its program.
        assert!(song.events.contains(&(
            516,
            Ma2Event::Program {
                channel: 1,
                bank: 0x80,
                program: 36
            }
        )));
        assert!(song.events.contains(&(
            516,
            Ma2Event::NoteOn {
                channel: 1,
                note: 3 * 12 + 12 + 36
            }
        )));
        assert!(song.events.contains(&(516 + 64, Ma2Event::NoteOff { channel: 1, note: 84 })));
        // The FF 00 padding still moves the clock: the song is 516 + 20 ms long.
        assert_eq!(song.len_ms, 536);
        assert_eq!(song.events.last(), Some(&(536, Ma2Event::End)));
    }

    #[test]
    fn events_at_one_instant_are_ordered_for_a_synth() {
        // Two back-to-back notes on one channel: the first's key-off and the second's key-on
        // share a millisecond, and the key-off has to go first.
        let seq = [
            0x00, 0x05, 0x02, // F, gate 2 ticks
            0x02, 0x00, 0x30, 0x10, // program change at the same instant as…
            0x00, 0x06, 0x02, // …the next note
            0x05, 0xff, 0x00,
        ];
        let song = parse_ma2(&smaf(&[track(1, 0, &[], &seq)])).expect("parses");
        let at8: Vec<_> = song.events.iter().filter(|(t, _)| *t == 8).map(|(_, e)| *e).collect();
        assert!(
            matches!(
                at8.as_slice(),
                [Ma2Event::Program { .. }, Ma2Event::NoteOff { .. }, Ma2Event::NoteOn { .. }]
            ),
            "{at8:?}"
        );
    }

    #[test]
    fn four_tracks_are_sixteen_channels_and_the_song_is_its_longest_track() {
        let t = |n: u8, ticks: u8| track(n, 0, &[], &[0x00, 0xc1, 0x01, ticks, 0xff, 0x00]);
        let song = parse_ma2(&smaf(&[t(1, 10), t(2, 20), t(3, 30), t(4, 40)])).expect("parses");
        // ch 3 of each track (status 0xc1 = ch3, oct0, C#).
        let chans: Vec<u8> = song
            .events
            .iter()
            .filter_map(|(_, e)| match e {
                Ma2Event::NoteOn { channel, .. } => Some(*channel),
                _ => None,
            })
            .collect();
        assert_eq!(chans, [3, 7, 11, 15]);
        assert_eq!(song.len_ms, 160);
        // A fifth track would need channels 16..19.
        assert!(parse_ma2(&smaf(&[t(1, 1), t(2, 1), t(3, 1), t(4, 1), t(5, 1)])).is_none());
    }

    #[test]
    fn a_realtime_exclusive_is_skipped_by_its_length() {
        // Song 14's shape: FF F0 06 43 03 90 ba xx F7 — then a note that must still parse.
        let seq = [
            0x00, 0xff, 0xf0, 0x06, 0x43, 0x03, 0x90, 0xba, 0x7f, 0xf7, 0x00, 0x01, 0x01, 0x05, 0xff, 0x00,
        ];
        let song = parse_ma2(&smaf(&[track(1, 0, &[], &seq)])).expect("parses");
        assert_eq!(song.exclusive_ignored, 1);
        assert!(song.events.contains(&(0, Ma2Event::NoteOn { channel: 0, note: 37 })));
    }

    #[test]
    fn what_it_cannot_play_faithfully_it_refuses() {
        let good = [0x00, 0x01, 0x01, 0x05, 0xff, 0x00];
        assert!(parse_ma2(&smaf(&[track(1, 0, &[], &good)])).is_some());
        // A song with no length would loop in no time at all.
        assert!(parse_ma2(&smaf(&[track(1, 0, &[], &[0x00, 0x01, 0x01])])).is_none());
        // An unknown control type.
        assert!(parse_ma2(&smaf(&[track(1, 0, &[], &[0x00, 0x00, 0x3f, 0x00])])).is_none());
        // FF followed by anything but 00 / F0.
        assert!(parse_ma2(&smaf(&[track(1, 0, &[], &[0x00, 0xff, 0x2f])])).is_none());
        // A two-byte length whose second byte has its top bit set.
        assert!(parse_ma2(&smaf(&[track(1, 0, &[], &[0x81, 0x80, 0x01, 0x01])])).is_none());
        // A voice of the wrong length.
        let short = [0xff, 0xf0, 0x03, 0x43, 0x03, 0xf7];
        assert!(parse_ma2(&smaf(&[track(1, 0, &short, &good)])).is_none());
        // A PCM track next to the score (hero2's effects are ATR only, its songs MTR only).
        let mut file = smaf(&[track(1, 0, &[], &good)]);
        let atr = chunk(b"ATR\x01", &[0; 6]);
        let body_end = file.len() - 2;
        file.splice(body_end..body_end, atr.iter().copied());
        let len = (file.len() - 8) as u32;
        file[4..8].copy_from_slice(&len.to_be_bytes());
        assert!(parse_ma2(&file).is_none());
        // MspI's stop point must be the end-of-sequence the parser found.
        let mut t = track(1, 0, &[], &good);
        let sp_at = t.windows(3).position(|w| w == b"sp:").unwrap() + 3;
        t[sp_at + 3] ^= 1;
        assert!(parse_ma2(&smaf(&[t])).is_none());
        // Not SMAF at all.
        assert!(parse_ma2(b"RIFF....").is_none());
        assert!(parse_ma2(&[]).is_none());
    }

    #[test]
    fn octave_shift_and_pitch_bend_values() {
        let seq = [
            0x00, 0x00, 0x32, 0x04, // +4 octaves
            0x00, 0x0c, 0x01, // nnnn 12 (C): 12 + 36 + 48 = 96
            0x00, 0x00, 0x32, 0x05, // not a shift value: ignored, still +4
            0x00, 0x0c, 0x01, 0x00, 0x00, 0x34, 0x00, // long bend, lowest
            0x00, 0x00, 0x18, // short bend 8 = centre
            0x05, 0xff, 0x00,
        ];
        let song = parse_ma2(&smaf(&[track(1, 0, &[], &seq)])).expect("parses");
        let notes: Vec<u8> = song
            .events
            .iter()
            .filter_map(|(_, e)| match e {
                Ma2Event::NoteOn { note, .. } => Some(*note),
                _ => None,
            })
            .collect();
        assert_eq!(notes, [96, 96]);
        assert!(song.events.contains(&(0, Ma2Event::PitchBend { channel: 0, value: 0 })));
        assert!(song.events.contains(&(0, Ma2Event::PitchBend { channel: 0, value: 0x2000 })));
    }
}
