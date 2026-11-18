//! `hz_render --jar <game.jar> --sf3 <font.sf3> --out <file.wav> [--entry snd/b/15]`
//!
//! One Standard MIDI File from the jar → oxisynth (the same `SynthDescriptor` hero2's device synth
//! uses: polyphony 64, gain 1.0, reverb and chorus on) at 44.1 kHz stereo → 16-bit WAV, through
//! the bus's soft limiter. The WAV carries an `hzmd` chunk: the FNV-1a 64 of the MIDI bytes it was
//! rendered from, so the app plays it only for THAT file (`hz_android::music::rendition_key`).
//!
//! The render runs until the last event, then on until the synth's tail is under −60 dBFS for
//! 100 ms (at most [`TAIL_MAX_SECS`]), and a 50 ms margin of that tail is kept.

use std::{
    fs,
    io::{Cursor, Read, Write},
    process::exit,
};

use oxisynth::{MidiEvent, SoundFont, Synth, SynthDescriptor};

const RATE: u32 = 44_100;
const TAIL_MAX_SECS: u32 = 4;
const TAIL_QUIET: f32 = 0.001;

fn usage() -> ! {
    eprintln!("usage: hz_render --jar <game.jar> --sf3 <font.sf3> --out <file.wav> [--entry snd/b/15]");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut jar, mut sf3, mut out, mut entry) = (None, None, None, "snd/b/15".to_string());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v = it.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--jar" => jar = Some(v.clone()),
            "--sf3" => sf3 = Some(v.clone()),
            "--out" => out = Some(v.clone()),
            "--entry" => entry = v.clone(),
            _ => usage(),
        }
    }
    let (Some(jar), Some(sf3), Some(out)) = (jar, sf3, out) else { usage() };

    let midi = read_entry(&jar, &entry).unwrap_or_else(|e| {
        eprintln!("hz_render: {jar} / {entry}: {e}");
        exit(1)
    });
    if !midi.starts_with(b"MThd") {
        eprintln!("hz_render: {entry} is not a Standard MIDI File");
        exit(1);
    }
    let events = parse_smf(&midi).unwrap_or_else(|e| {
        eprintln!("hz_render: {entry}: {e}");
        exit(1)
    });

    let font_bytes = fs::read(&sf3).unwrap_or_else(|e| {
        eprintln!("hz_render: {sf3}: {e}");
        exit(1)
    });
    let font = SoundFont::load(&mut Cursor::new(font_bytes)).unwrap_or_else(|e| {
        eprintln!("hz_render: {sf3}: SoundFont::load: {e:?}");
        exit(1)
    });
    let mut synth = Synth::new(SynthDescriptor {
        sample_rate: RATE as f32,
        polyphony: 64,
        gain: 1.0,
        chorus_active: true,
        reverb_active: true,
        ..Default::default()
    })
    .expect("synth");
    synth.add_font(font, true);

    let pcm = render(&mut synth, &events);
    let key = fnv64(&midi);
    write_wav(&out, &pcm, key).unwrap_or_else(|e| {
        eprintln!("hz_render: {out}: {e}");
        exit(1)
    });
    let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!(
        "hz_render: {entry} ({} bytes, {} events, key {key:016x}) -> {out}: {:.2} s, peak {peak:.3}",
        midi.len(),
        events.len(),
        pcm.len() as f64 / 2.0 / RATE as f64
    );
}

fn read_entry(jar: &str, entry: &str) -> Result<Vec<u8>, String> {
    let file = fs::File::open(jar).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut f = zip.by_name(entry).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    f.read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}

/// FNV-1a 64 — the same function as `hz_android::music::rendition_key`.
fn fnv64(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// One MIDI event at a sample position.
struct Ev {
    at: u64,
    ev: MidiEvent,
}

enum Raw {
    Midi(MidiEvent),
    Tempo(u32),
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn read_vlq(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for _ in 0..4 {
        let b = *d.get(*pos)?;
        *pos += 1;
        v = (v << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Every track's events merged in tick order, the tempo map applied: sample positions.
fn parse_smf(d: &[u8]) -> Result<Vec<Ev>, String> {
    let header_len = be32(d, 4).ok_or("short header")? as usize;
    let tracks = be16(d, 10).ok_or("short header")?;
    let division = be16(d, 12).ok_or("short header")?;
    if division & 0x8000 != 0 {
        return Err("SMPTE time division is not supported".into());
    }
    let ppq = u64::from(division.max(1));

    // (tick, order, event) — order keeps a track's events in sequence at equal ticks.
    let mut all: Vec<(u64, usize, Raw)> = Vec::new();
    let mut pos = 8 + header_len;
    let mut seen = 0;
    while seen < tracks && pos + 8 <= d.len() {
        let len = be32(d, pos + 4).ok_or("short track")? as usize;
        let body = pos + 8;
        if &d[pos..pos + 4] == b"MTrk" {
            seen += 1;
            let end = body.saturating_add(len).min(d.len());
            parse_track(&d[body..end], &mut all)?;
        }
        pos = body.saturating_add(len);
    }
    all.sort_by_key(|(tick, order, _)| (*tick, *order));

    let mut out = Vec::new();
    let (mut tick0, mut sample0, mut tempo) = (0u64, 0f64, 500_000u64);
    for (tick, _, raw) in all {
        let at = sample0 + (tick - tick0) as f64 * tempo as f64 / 1e6 * RATE as f64 / ppq as f64;
        match raw {
            Raw::Tempo(t) => {
                (tick0, sample0, tempo) = (tick, at, u64::from(t.max(1)));
            }
            Raw::Midi(ev) => out.push(Ev { at: at as u64, ev }),
        }
    }
    Ok(out)
}

fn parse_track(t: &[u8], all: &mut Vec<(u64, usize, Raw)>) -> Result<(), String> {
    let (mut pos, mut tick, mut running) = (0usize, 0u64, 0u8);
    while pos < t.len() {
        let delta = read_vlq(t, &mut pos).ok_or("bad delta")?;
        tick += delta;
        let status = *t.get(pos).ok_or("truncated")?;
        match status {
            0xff => {
                let kind = *t.get(pos + 1).ok_or("truncated")?;
                pos += 2;
                let len = read_vlq(t, &mut pos).ok_or("bad meta length")? as usize;
                if kind == 0x51 && len == 3 {
                    let b = t.get(pos..pos + 3).ok_or("truncated")?;
                    all.push((tick, all.len(), Raw::Tempo(u32::from_be_bytes([0, b[0], b[1], b[2]]))));
                }
                if kind == 0x2f {
                    return Ok(());
                }
                pos += len;
            }
            0xf0 | 0xf7 => {
                pos += 1;
                let len = read_vlq(t, &mut pos).ok_or("bad sysex length")? as usize;
                pos += len; // GM reset and the like: the synth starts reset
            }
            _ => {
                let status = if status & 0x80 != 0 {
                    pos += 1;
                    running = status;
                    status
                } else if running != 0 {
                    running
                } else {
                    return Err("data byte without a status".into());
                };
                let channel = status & 0x0f;
                let d1 = *t.get(pos).ok_or("truncated")?;
                let ev = match status & 0xf0 {
                    0xc0 => {
                        pos += 1;
                        Some(MidiEvent::ProgramChange { channel, program_id: d1 })
                    }
                    0xd0 => {
                        pos += 1;
                        Some(MidiEvent::ChannelPressure { channel, value: d1 })
                    }
                    kind => {
                        let d2 = *t.get(pos + 1).ok_or("truncated")?;
                        pos += 2;
                        match kind {
                            0x80 => Some(MidiEvent::NoteOff { channel, key: d1 }),
                            0x90 if d2 == 0 => Some(MidiEvent::NoteOff { channel, key: d1 }),
                            0x90 => Some(MidiEvent::NoteOn { channel, key: d1, vel: d2 }),
                            0xa0 => Some(MidiEvent::PolyphonicKeyPressure { channel, key: d1, value: d2 }),
                            0xb0 => Some(MidiEvent::ControlChange { channel, ctrl: d1, value: d2 }),
                            0xe0 => Some(MidiEvent::PitchBend { channel, value: u16::from(d1) | (u16::from(d2) << 7) }),
                            _ => None,
                        }
                    }
                };
                if let Some(ev) = ev {
                    all.push((tick, all.len(), Raw::Midi(ev)));
                }
            }
        }
    }
    Ok(())
}

fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.7;
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    let head = 1.0 - KNEE;
    let over = a - KNEE;
    let y = KNEE + head * (over / (over + head));
    if x < 0.0 { -y } else { y }
}

/// Interleaved stereo at [`RATE`], limited, the tail trimmed.
fn render(synth: &mut Synth, events: &[Ev]) -> Vec<f32> {
    fn render_to(synth: &mut Synth, pcm: &mut Vec<f32>, rendered: &mut u64, upto: u64) {
        if *rendered < upto {
            let start = pcm.len();
            pcm.resize(start + (upto - *rendered) as usize * 2, 0.0);
            synth.write(&mut pcm[start..]);
            *rendered = upto;
        }
    }
    let mut pcm: Vec<f32> = Vec::new();
    let mut rendered = 0u64;
    for e in events {
        render_to(synth, &mut pcm, &mut rendered, e.at);
        let _ = synth.send_event(e.ev);
    }
    // The tail: render in 10 ms steps until 100 ms of it is quiet, or the cap.
    let step = u64::from(RATE) / 100;
    let cap = rendered + u64::from(RATE * TAIL_MAX_SECS);
    let mut quiet_steps = 0;
    while rendered < cap && quiet_steps < 10 {
        let from = pcm.len();
        let upto = rendered + step;
        render_to(synth, &mut pcm, &mut rendered, upto);
        let peak = pcm[from..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        quiet_steps = if peak < TAIL_QUIET { quiet_steps + 1 } else { 0 };
    }
    // Keep 50 ms of what was quiet, drop the rest of it.
    let keep = (10usize.saturating_sub(5).min(quiet_steps)) * step as usize * 2;
    let cut = quiet_steps * step as usize * 2;
    pcm.truncate(pcm.len() - cut + keep);
    for s in pcm.iter_mut() {
        *s = soft_limit(*s);
    }
    pcm
}

/// 16-bit stereo WAV with the `hzmd` chunk (the source's key) before `data`.
fn write_wav(path: &str, pcm: &[f32], key: u64) -> std::io::Result<()> {
    let data: Vec<u8> = pcm.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let mut w = Vec::with_capacity(60 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(4 + 24 + 16 + 8 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"hzmd");
    w.extend_from_slice(&8u32.to_le_bytes());
    w.extend_from_slice(&key.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    let mut f = fs::File::create(path)?;
    f.write_all(&w)
}
