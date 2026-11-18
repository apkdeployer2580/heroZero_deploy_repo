//! Host-side socket-bonus sum — lets an item carry orbs of DIFFERENT kinds.
//!
//! ── THE BUG THIS FIXES ───────────────────────────────────────────────────────
//! The player reported that socketing a second orb of a different kind replaced the
//! first: "오브를 2개이상 결합해도 마지막에 결합한 오브만 효과가 나타나고있어". That is not a
//! server bug — it is the shape of the record. An inventory item stores exactly ONE
//! orb id, so the game has nowhere to put a second kind.
//!
//! ── WHAT THE GAME ACTUALLY COMPUTES (LGT `binary.mod`, md5 6a352944…) ────────
//! `addItemToInventory` (`0x131d0`) unpacks a 20-byte warehouse record into a 60-byte
//! inventory slot at `gridBase + 0x394 + tab*0x3c4 + slot*0x3c`, then derives two
//! values. Naming offsets from that slot record:
//!
//! | slot off | absolute (the form `items.rs` writes) | meaning |
//! |---|---|---|
//! | `+0x31` | `+0x3c5` | refine level |
//! | `+0x32` | `+0x3c6` | total sockets |
//! | `+0x34` | `+0x3c8` | socketed orb id |
//! | `+0x35` | `+0x3c9` | filled sockets |
//! | `+0x38` | `+0x3cc` | **socket bonus, u16** |
//! | `+0x3a` | `+0x3ce` | effective stat, u16 |
//!
//! `0x133e4`..`0x13408` builds the bonus:
//!
//! ```text
//!     orbStat = (s8) orbTable[orbId * 0x18 + 0x15]
//!     filled  = (s8) rec[0x12]
//!     bonus   = orbStat * (filled^2 + filled) / 2
//! ```
//!
//! and `0x13416`..`0x13480` consumes it:
//!
//! ```text
//!     mul  = (4 <= cat <= 10) ? 2 : 1            ; the weapon categories
//!     base = (s16) itemDB[cat][idx * 0x20 + 0x16]
//!     eff  = (base + mul * refine) * (100 + bonus) / 100
//! ```
//!
//! ★ So the socket bonus is a **single percentage applied to the item's own main
//! stat** — NOT a per-stat-type bonus. Every orb turns the same knob and differs only
//! in magnitude; the flavour text ("HP의 최대치를 증가", "크리티컬의 발생 확률을 증가") is
//! decorative and no code reads it. That is what makes summing expressible at all: the
//! sum of several orbs is still just one percentage, and it fits the u16 the game
//! already has.
//!
//! ── THE RULE, WHICH GENERALISES THE GAME'S OWN ───────────────────────────────
//! Group the socketed orbs by kind; a kind present `k` times contributes
//! `orbStat * (k^2 + k) / 2`, and the contributions are summed:
//!
//! * all orbs the same kind  -> one group with `k = filled` -> **byte-identical to the
//!   guest's own formula**, so stacking one kind keeps its amplification.
//! * all orbs distinct       -> every group has `k = 1` -> `(1 + 1) / 2 = 1` -> a plain
//!   sum of the individual magnitudes.
//!
//! That is exactly the policy the player chose (PLAY_FEEDBACK_FIXES.md §40차): same
//! kind keeps amplifying, different kinds add up.
//!
//! ── WHERE THE ORB LIST COMES FROM ────────────────────────────────────────────
//! `hero3_server` packs the full set into wire bytes `[11..15]`, +1 biased and
//! left-packed (see `ProtocolHandler.set_orb_list`). Those five bytes are free: the
//! client's unpack reads only `[0] [4] [8] [9] [10] [16] [17] [18] [19]`, and five is
//! the game's own socket maximum (`rand(1,5)`).
//!
//! ── SAFETY ───────────────────────────────────────────────────────────────────
//! The hook replaces `movs r2, #0x13` at `0x133e4` and resumes at `0x1340a`, past the
//! `strh` it performs itself. It writes ONE halfword, to the address the guest already
//! computed into `r0`, and touches no other register — `r4` in particular still holds
//! the record base that `0x13412` needs.
//!
//! It DECLINES (falling back to the guest's own value, computed here identically) when
//! the record carries no orb list, which is every record written before the server
//! learned to emit one. So an old save, an old server, or a hand-made record all
//! behave exactly as they did before this file existed.

use alloc::{format, vec::Vec};

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// Wire-record bytes holding the orb list, and the bias applied to each id.
/// Mirrors `ProtocolHandler.ORB_LIST_LO/HI/BIAS` in `hero3_server/hero3_protocol.py`.
const ORB_LIST_LO: u32 = 11;
const ORB_LIST_LEN: usize = 5;
const ORB_LIST_BIAS: u8 = 1;
/// Both fillers that occur naturally in that window: `[11]` is 0x00 and `[12..15]`
/// are 0xff in a canonical record, so neither may decode as an orb.
const EMPTY_BYTES: [u8; 2] = [0x00, 0xff];

/// Wire-record offsets the guest itself reads for the single-orb path.
const REC_FILLED: u32 = 0x12;
const REC_ORB: u32 = 0x13;

/// Orb stat table: stride and the signed magnitude byte, from `0x133e6`..`0x133f2`.
const ORB_STRIDE: u32 = 0x18;
const ORB_STAT_OFF: u32 = 0x15;
/// `[sp + 0x3c]` is where `0x13208` parked the orb table base.
const SP_ORB_TABLE: u32 = 0x3c;

/// The largest orb count the game can produce (`rand(1,5)` sockets).
const MAX_SOCKETS: usize = 5;
/// A bonus is a percentage; the guest stores it in a u16 and then does
/// `(base + mul*refine) * (100 + bonus) / 100` in 32-bit. Refuse anything that could
/// only come from garbage rather than silently scaling a stat into nonsense.
const MAX_BONUS: i32 = 10_000;

/// `movs r2, #0x13` — the two bytes the SVC displaces. Checked at install time for the
/// same reason the blitter hooks check theirs: the hook resumes at a SECOND fixed
/// address, so a moved function would both skip the wrong span and leave the bonus
/// unwritten, silently.
const EXPECTED_HEAD: [u8; 2] = [0x13, 0x22];

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "orb_sum hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (movs r2,#0x13), found {head:02x?}"
    )))
}

/// Group `orbs` by kind and sum `stat * (k^2 + k) / 2` over the groups.
///
/// `stat_of` yields an orb's magnitude; it returns `None` for an id the table cannot
/// answer for, and such an orb contributes nothing rather than poisoning the total.
pub fn sum_bonus(orbs: &[u8], mut stat_of: impl FnMut(u8) -> Option<i32>) -> i32 {
    let mut seen: Vec<(u8, i32)> = Vec::new();
    for &id in orbs.iter().take(MAX_SOCKETS) {
        match seen.iter_mut().find(|(k, _)| *k == id) {
            Some((_, n)) => *n += 1,
            None => seen.push((id, 1)),
        }
    }
    seen.iter().filter_map(|&(id, k)| stat_of(id).map(|s| s * (k * k + k) / 2)).sum()
}

/// Decode the +1-biased, left-packed orb list out of a 20-byte wire record window.
pub fn decode_orb_list(window: &[u8; ORB_LIST_LEN]) -> Vec<u8> {
    window
        .iter()
        .copied()
        .filter(|b| !EMPTY_BYTES.contains(b))
        .map(|b| b - ORB_LIST_BIAS)
        .collect()
}

pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (dest, rec, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R6),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };

    let mut table = [0u8; 4];
    core.read_bytes(sp + SP_ORB_TABLE, &mut table)?;
    let orb_table = u32::from_le_bytes(table);

    // `orbTable[id * 0x18 + 0x15]`, signed exactly as the guest's `ldrsb` reads it.
    let stat_of = |core: &mut ArmCore, id: u8| -> Option<i32> {
        let mut b = [0u8; 1];
        core.read_bytes(orb_table + u32::from(id) * ORB_STRIDE + ORB_STAT_OFF, &mut b).ok()?;
        Some(i32::from(b[0] as i8))
    };

    let mut window = [0u8; ORB_LIST_LEN];
    core.read_bytes(rec + ORB_LIST_LO, &mut window)?;
    let orbs = decode_orb_list(&window);

    // The whole tail the guest reads, so a bad value can be attributed to the wire record
    // rather than to this arithmetic.
    let mut raw = [0u8; 12];
    core.read_bytes(rec + 8, &mut raw)?;

    let bonus = if orbs.is_empty() {
        // No list: reproduce the guest's own single-orb formula so declining costs
        // nothing. This is the path every pre-2026-08-14 record takes.
        let mut b = [0u8; 1];
        core.read_bytes(rec + REC_FILLED, &mut b)?;
        let filled = i32::from(b[0] as i8);
        core.read_bytes(rec + REC_ORB, &mut b)?;
        let orb = b[0];
        let stat = stat_of(core, orb).unwrap_or(0);
        stat * (filled * filled + filled) / 2
    } else {
        // Pre-read the magnitudes: `sum_bonus` takes a plain closure, and reading guest
        // memory needs `&mut core`, which cannot be borrowed inside it.
        let stats: Vec<(u8, i32)> = {
            let mut v: Vec<(u8, i32)> = Vec::new();
            for &id in &orbs {
                if !v.iter().any(|(k, _)| *k == id) {
                    let s = stat_of(core, id).unwrap_or(0);
                    v.push((id, s));
                }
            }
            v
        };
        sum_bonus(&orbs, |id| stats.iter().find(|(k, _)| *k == id).map(|&(_, s)| s))
    };

    // A negative or absurd total can only come from a corrupt table or record; clamp to
    // the guest's own no-op rather than scale a stat by it.
    let bonus = bonus.clamp(0, MAX_BONUS) as u16;

    // warn, not trace: the Android build filters the wie crates to warn, and "what did the
    // hook actually see" is the only question worth asking when a socket bonus looks wrong.
    tracing::warn!("hook orb_sum(rec={rec:#x}) raw[8..20]={raw:02x?} orbs={orbs:?} -> bonus {bonus}");
    core.write_bytes(dest, &bonus.to_le_bytes())?;
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The magnitudes read off the shipped `i16_dat` orb rows, in index order.
    const STAT: [i32; 15] = [2, 12, 15, 15, 15, 8, 8, 8, 6, 6, 10, 15, 2, 6, 6];
    fn stat(id: u8) -> Option<i32> {
        STAT.get(id as usize).copied()
    }

    #[test]
    fn empty_list_is_zero() {
        assert_eq!(sum_bonus(&[], stat), 0);
    }

    #[test]
    fn one_orb_is_its_own_magnitude() {
        // (1^2 + 1) / 2 == 1, so a single orb contributes exactly its stat.
        assert_eq!(sum_bonus(&[2], stat), 15);
        assert_eq!(sum_bonus(&[0], stat), 2);
    }

    #[test]
    fn same_kind_matches_the_guest_formula() {
        // The whole point of grouping: stacking one kind must stay byte-identical to
        // `stat * (filled^2 + filled) / 2`, which is what the guest would have computed.
        for k in 1..=MAX_SOCKETS as i32 {
            let orbs = alloc::vec![2u8; k as usize];
            assert_eq!(sum_bonus(&orbs, stat), 15 * (k * k + k) / 2, "k={k}");
        }
    }

    #[test]
    fn different_kinds_add_up() {
        // 직격의(0x0e)=6 + 흡혈의(0x0c)=2 + 정령의(0x04)=15
        assert_eq!(sum_bonus(&[0x0e, 0x0c, 0x04], stat), 6 + 2 + 15);
    }

    #[test]
    fn mixed_groups_amplify_within_a_kind_then_add() {
        // Two 뇌제의(15) amplify to 15*3=45; one 속박의(6) adds 6.
        assert_eq!(sum_bonus(&[2, 2, 8], stat), 45 + 6);
        // Order must not matter — grouping is by kind, not by adjacency.
        assert_eq!(sum_bonus(&[2, 8, 2], stat), 45 + 6);
    }

    #[test]
    fn beyond_five_sockets_is_ignored() {
        // The game can never roll more than 5, so a longer list is a corrupt record.
        let six = [2u8; 6];
        assert_eq!(sum_bonus(&six, stat), sum_bonus(&[2u8; 5], stat));
    }

    #[test]
    fn an_unknown_orb_contributes_nothing_instead_of_poisoning_the_sum() {
        assert_eq!(sum_bonus(&[0x0e, 0xfe], stat), 6);
    }

    #[test]
    fn decode_rejects_both_canonical_fillers() {
        // [11] is 0x00 and [12..15] are 0xff in a record the game itself wrote.
        assert!(decode_orb_list(&[0x00, 0xff, 0xff, 0xff, 0xff]).is_empty());
        assert!(decode_orb_list(&[0x00; 5]).is_empty());
        assert!(decode_orb_list(&[0xff; 5]).is_empty());
    }

    #[test]
    fn decode_unbiases_so_orb_zero_survives() {
        // 투신의 is index 0; without the +1 bias it would be indistinguishable from the
        // 0x00 filler, which is the reason the bias exists.
        assert_eq!(decode_orb_list(&[0x01, 0xff, 0xff, 0xff, 0xff]), alloc::vec![0]);
        assert_eq!(decode_orb_list(&[0x0f, 0x01, 0xff, 0xff, 0xff]), alloc::vec![14, 0]);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The NAME half: "직격흡혈정령의 호크아이"
// ─────────────────────────────────────────────────────────────────────────────
//
// The item's displayed name is NOT stored anywhere — `formatItemName` (LGT `0x6b90`,
// `(dst = r0, record = r1)`) rebuilds it every time a row is drawn, by switching on the
// category and then on two record bytes:
//
// | branch | refine | orb | at | sprintf format |
// |---|---|---|---|---|
// | A | != 0 | present | `0x6bfe` -> `0x6c20` | `"%s %s +%d"` (orb, base, refine) |
// | B | != 0 | none    | `0x6c26` -> `0x6c5a` | `"%s +%d"` |
// | C | == 0 | present | `0x6c40` -> `0x6c5c` | `"%s %s"` (orb, base) |
// | D | == 0 | none    | `0x6c62` -> `0x6cf6` | the base name, copied |
//
// Only A and C print an orb prefix, and both reach it the same way:
//
//     orbName = getString(*(u32*)0x1512928, orbId + 0xab)
//
// `getString` (`0x1bdcc`) is a plain offset table and needs no guest call at all:
//
//     getString(base, i) = base + u16_le(base + 4 + i*2)
//
// So the hook sits on the instruction right AFTER that lookup returns (`movs r3, #9`,
// at `0x6c0a` in A and `0x6c4c` in C), composes the chained prefix into a scratch
// buffer, and swaps `r0` for it. The game's own `sprintf` then runs untouched — which is
// why this is preferred over reimplementing all four branches host-side.
//
// ── THE BUDGET IS REAL, AND IT IS 48 BYTES ───────────────────────────────────
// All five callers of `formatItemName` live in ONE function (`0x8848`, `sub sp,#0x150`)
// and pass a stack buffer at `sp+0x120`, so `sprintf` has exactly `0x150 - 0x120` = 48
// bytes. `sprintf` truncates nothing, so a prefix that does not fit would SMASH THE
// GUEST STACK. The hook therefore computes what is left after the base name, the
// separator, the ` +NN` suffix and the NUL, and truncates the prefix to that — on a
// 2-byte EUC-KR character boundary. That is the player's own rule ("넘치면 그냥 잘리게")
// and the overflow guard at the same time.
//
// Declines (leaving `r0` exactly as the game computed it) whenever there are fewer than
// two orbs, so single-orb items and every legacy record render byte-identically.

/// `*(u32*)STRING_TABLE_PTR` is the string-table base; orb names sit at `id + 0xab`.
const ORB_NAME_BIAS: u32 = 0xab;
/// EUC-KR `의` — the syllable every orb name ends with, dropped to make a stem.
const EUI: [u8; 2] = [0xc0, 0xc7];
/// `sprintf`'s destination buffer, in BYTES.
///
/// ★ NOT the slack to the top of the frame. An earlier version of this file used
/// `0x150 - 0x120 = 48`, measured from fn 0x8848's frame size, and that was wrong: the
/// destination is a 30-byte array the callers `memset` themselves. `formatItemName`
/// (0x6b90) has 29 call sites across 5 functions; four of them clear 0x1e bytes
/// (0x8896/0x8898 in 0x8848, 0x96e2/0x96e4 in 0x9694, 0xfa02/0xfa04 in 0xf958,
/// 0x10a02/0x10a04 in 0x10958) and one clears 0x40 (0x13e60/0x13e64 in 0x13834).
/// The minimum binds, and `sprintf` does not truncate — with five orbs the 48-byte
/// figure wrote ~9 bytes past the array.
const DEST_CAPACITY: usize = 0x1e;
/// `" +NN"` plus the NUL `sprintf` writes, and the space between prefix and base name.
const SUFFIX_BUDGET: usize = 5;
const SEPARATOR_BUDGET: usize = 1;
/// Item-DB record stride, and the name pointer at its offset 0 (`0x6c12`: `lsls #5`).
const DB_STRIDE: u32 = 0x20;
/// Wire-record item index, the same byte `0x6c0a` reloads.
const REC_IDX: u32 = 0x09;
/// The two build-specific addresses the name hook cannot derive.
#[derive(Debug, Clone, Copy)]
pub struct NameTables {
    /// `.bss` word holding the string-table base (`0x1512928`), read fresh per call.
    pub string_table: u32,
    /// Guest-writable scratch for the composed prefix. Must sit in a range proven dead
    /// and mapped RW — see `wie_android::guest_anchors::save_chain`'s three-legged proof
    /// for `0x1512958..0x1513000`, of which the save chain uses only 4 bytes.
    pub scratch: u32,
}

/// `movs r3, #9` — the two bytes displaced at BOTH name-hook sites.
const EXPECTED_NAME_HEAD: [u8; 2] = [0x09, 0x23];

pub fn check_name_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_NAME_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "orb_name hook at {pc:#x}: expected {EXPECTED_NAME_HEAD:02x?} (movs r3,#9), found {head:02x?}"
    )))
}

/// Text-colour escapes the font layer consumes as ONE byte anywhere in a string
/// (`isEscape` 0x35d8, `applyColourEscape` 0x37e4; rodata alphabet 0x5b2bc).
///
/// ★ EVERY string-table orb name starts with `@` (0x40): entries 0xab..0xb9 are each
/// exactly SEVEN bytes, e.g. 0xb9 = `40 c1 f7 b0 dd c0 c7` (직격의). That is why a stem is
/// 5 bytes and not 4, and why the old even-length rounding below cut a syllable in half.
const ESCAPES: [u8; 7] = [0x40, 0x7c, 0x24, 0x7b, 0x7d, 0x27, 0x22];

fn strip_escape(name: &[u8]) -> &[u8] {
    match name.first() {
        Some(b) if ESCAPES.contains(b) => &name[1..],
        _ => name,
    }
}

/// Drop a trailing `의` so `직격의` becomes `직격`. Names that do not end in one are kept
/// whole rather than blindly losing two bytes.
fn stem(name: &[u8]) -> &[u8] {
    if name.len() > EUI.len() && name[name.len() - EUI.len()..] == EUI {
        &name[..name.len() - EUI.len()]
    } else {
        name
    }
}

/// Append as much of `part` as fits, cutting only where the font layer agrees a character
/// ends: a byte with the high bit set opens a 2-byte EUC-KR syllable (draw loop
/// 0x3918 -> 0x395a advances two), anything else — an escape or ASCII — is one byte.
///
/// This replaces an `& !1` even-length round, which was wrong precisely because the leading
/// colour escape makes the stream ODD-aligned: with three orbs the prefix is 17 bytes, the
/// round chopped it to 16, and the trailing lone `c0` lead byte made the font layer eat the
/// following separator and draw one undefined glyph. That is the garbage the player saw at
/// three orbs, and why two looked fine.
fn push_bounded(out: &mut Vec<u8>, part: &[u8], budget: usize) {
    let mut i = 0;
    while i < part.len() {
        let w = if part[i] & 0x80 != 0 { 2 } else { 1 };
        if i + w > part.len() || out.len() + w > budget {
            return;
        }
        out.extend_from_slice(&part[i..i + w]);
        i += w;
    }
}

/// `직격의 + 흡혈의 + 정령의` -> `직격흡혈정령의`: every orb but the last contributes its
/// stem, the last keeps its `의`, and the whole thing is cut to `budget` on a real
/// character boundary.
///
/// The FIRST name keeps its colour escape so the chained name renders in the same colour
/// the game's own single-orb name would have; the rest drop theirs, since they would only
/// re-select the colour that is already active.
pub fn chain_names(names: &[&[u8]], budget: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let n: &[u8] = if i == 0 { n } else { strip_escape(n) };
        let part = if i + 1 == names.len() { n } else { stem(n) };
        push_bounded(&mut out, part, budget);
    }
    out
}

/// Read a NUL-terminated string out of guest memory, bounded so a corrupt pointer
/// cannot spin. `cap` is generous: item names are a handful of syllables.
fn read_cstr(core: &mut ArmCore, addr: u32, cap: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    for i in 0..cap {
        core.read_bytes(addr + i as u32, &mut b)?;
        if b[0] == 0 {
            break;
        }
        out.push(b[0]);
    }
    Ok(out)
}

/// `getString(base, i) = base + u16_le(base + 4 + i*2)` — the guest's own table walk
/// (`0x1bdcc`), transcribed so the hook needs no guest call.
fn get_string(core: &mut ArmCore, table: u32, index: u32) -> Result<u32> {
    let mut off = [0u8; 2];
    core.read_bytes(table + 4 + index * 2, &mut off)?;
    Ok(table + u32::from(u16::from_le_bytes(off)))
}

pub async fn handle_name_lgt(core: &mut ArmCore, _hook_pc: u32, tables: NameTables, exit_pc: u32) -> Result<JumpTo> {
    let (rec, db) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::R5), inner.engine.reg_read(ArmRegister::R4))
    };

    let mut window = [0u8; ORB_LIST_LEN];
    core.read_bytes(rec + ORB_LIST_LO, &mut window)?;
    let orbs = decode_orb_list(&window);

    // Fewer than two orbs: the game's single-orb prefix is already right. Perform the
    // displaced `movs r3, #9` and leave `r0` alone — byte-identical to no hook at all.
    if orbs.len() >= 2 {
        let mut ptr = [0u8; 4];
        core.read_bytes(tables.string_table, &mut ptr)?;
        let strings = u32::from_le_bytes(ptr);

        let mut names: Vec<Vec<u8>> = Vec::new();
        for &id in &orbs {
            let addr = get_string(core, strings, u32::from(id) + ORB_NAME_BIAS)?;
            names.push(read_cstr(core, addr, 32)?);
        }

        // What is left of the 48-byte destination once sprintf has written everything
        // else. The base name is reached exactly as `0x6c12` does: `ldrsb` the index,
        // scale by the 0x20 record stride, and load the name pointer at offset 0. A
        // negative index is not something the game can produce, so treat it as garbage
        // and keep the whole budget for the prefix rather than read a wild address.
        let mut idx = [0u8; 1];
        core.read_bytes(rec + REC_IDX, &mut idx)?;
        let base_len = if (idx[0] as i8) >= 0 {
            let mut name_ptr = [0u8; 4];
            core.read_bytes(db + u32::from(idx[0]) * DB_STRIDE, &mut name_ptr)?;
            read_cstr(core, u32::from_le_bytes(name_ptr), 40)?.len()
        } else {
            0
        };
        let budget = DEST_CAPACITY.saturating_sub(base_len + SEPARATOR_BUDGET + SUFFIX_BUDGET);

        let refs: Vec<&[u8]> = names.iter().map(|v| v.as_slice()).collect();
        let chained = chain_names(&refs, budget);
        if !chained.is_empty() {
            core.write_bytes(tables.scratch, &chained)?;
            core.write_bytes(tables.scratch + chained.len() as u32, &[0u8])?;
            tracing::warn!("hook orb_name(rec={rec:#x}, orbs={orbs:?}) -> {} bytes", chained.len());
            core.inner.lock().engine.reg_write(ArmRegister::R0, tables.scratch);
        }
    }

    // The displaced instruction.
    core.inner.lock().engine.reg_write(ArmRegister::R3, 9);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod name_tests {
    use super::*;

    // The REAL string-table bytes, entries 0xab.. — each seven bytes with a leading `@`
    // colour escape. The earlier fixtures omitted the escape, which is exactly why the
    // unit tests passed while the device drew garbage.
    const JIKGYEOK: [u8; 7] = [0x40, 0xc1, 0xf7, 0xb0, 0xdd, 0xc0, 0xc7]; // 직격의
    const HEUPHYEOL: [u8; 7] = [0x40, 0xc8, 0xed, 0xc7, 0xf7, 0xc0, 0xc7]; // 흡혈의
    const JEONGRYEONG: [u8; 7] = [0x40, 0xc1, 0xa4, 0xb7, 0xc9, 0xc0, 0xc7]; // 정령의

    /// Every byte must belong to a whole character: a 0x80+ byte opens a 2-byte syllable.
    fn well_formed(b: &[u8]) -> bool {
        let mut i = 0;
        while i < b.len() {
            let w = if b[i] & 0x80 != 0 { 2 } else { 1 };
            if i + w > b.len() {
                return false;
            }
            i += w;
        }
        true
    }

    #[test]
    fn a_single_name_is_unchanged() {
        assert_eq!(chain_names(&[&JIKGYEOK], 64), JIKGYEOK);
    }

    #[test]
    fn three_orbs_no_longer_end_in_half_a_syllable() {
        // The device case: orbs=[10, 1, 14]. The old code produced 17 bytes, rounded to an
        // even 16, and left a lone 0xc0 lead byte that ate the following separator.
        let got = chain_names(&[&JIKGYEOK, &HEUPHYEOL, &JEONGRYEONG], 64);
        assert!(well_formed(&got), "{got:02x?}");
        // escape + 2 syllables (5) | 2 syllables (4) | the full final name, escape dropped (6).
        assert_eq!(got.len(), 5 + 4 + 6);
        assert_eq!(got[0], 0x40, "the first name keeps its colour escape");
        assert_eq!(got[5], 0xc8, "the second name's escape is dropped");
    }

    #[test]
    fn every_prefix_length_stays_well_formed_and_within_budget() {
        let names: [&[u8]; 5] = [&JIKGYEOK, &HEUPHYEOL, &JEONGRYEONG, &JIKGYEOK, &HEUPHYEOL];
        for n in 1..=5 {
            for budget in 0..40 {
                let got = chain_names(&names[..n], budget);
                assert!(got.len() <= budget, "n={n} budget={budget} overflowed");
                assert!(well_formed(&got), "n={n} budget={budget} -> {got:02x?}");
            }
        }
    }

    #[test]
    fn five_orbs_fit_the_real_thirty_byte_destination() {
        // 4 stems (5 B) + one full name (7 B) = 27, which must still leave room for the
        // separator, a base name and " +NN" inside DEST_CAPACITY — it does not, so the
        // budget MUST bite. This is the case that used to write ~9 bytes past the array.
        let names: [&[u8]; 5] = [&JIKGYEOK, &HEUPHYEOL, &JEONGRYEONG, &JIKGYEOK, &HEUPHYEOL];
        let budget = DEST_CAPACITY - (8 + SEPARATOR_BUDGET + SUFFIX_BUDGET);
        let got = chain_names(&names, budget);
        assert!(got.len() <= budget);
        assert!(well_formed(&got));
    }

    #[test]
    fn a_name_without_the_trailing_eui_is_not_truncated() {
        let odd = alloc::vec![0xb0, 0xa1, 0xb0, 0xa2];
        assert_eq!(stem(&odd), &odd[..]);
    }

    #[test]
    fn strip_escape_only_touches_a_real_escape() {
        assert_eq!(strip_escape(&JIKGYEOK), &JIKGYEOK[1..]);
        let plain = [0xc1u8, 0xf7, 0xb0, 0xdd];
        assert_eq!(strip_escape(&plain), &plain[..]);
    }
}
