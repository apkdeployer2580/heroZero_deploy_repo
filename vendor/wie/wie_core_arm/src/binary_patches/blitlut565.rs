//! Host reimplementation of the game's **LUT-blended** palettised sprite blitter — the
//! third and last of its per-pixel loops (MIGRATION_LGT.md §2.19-(4), §2.20).
//!
//! ── WHY ──────────────────────────────────────────────────────────────────────
//! The player reported a frame drop that only happens when a CRITICAL HIT effect is on
//! screen, and reported it again after [`super::blend565`] took ~64 % off ordinary field
//! frames. It is a different function: `blend565` fills a translucent RECTANGLE and
//! [`super::blit565`] draws an OPAQUE sprite, and neither is what a critical flash is.
//!
//! Found the same way both of those were — a difference profile. The probe's intro battle
//! sits at frames ~3000..4200, so a `--frames 2950` run (before the fight) against a
//! `--frames 4200` one (including it) isolates what the fight adds: an evenly distributed
//! cluster at `0x2ea38..0x2eba6`, i.e. a pixel loop, inside the function whose entry is
//! `0x2e8c4` — immediately after `blit565`'s `0x2dd7c..0x2e8c2`.
//!
//! ── WHAT THE GUEST DOES ──────────────────────────────────────────────────────
//! `push {r4-r7,lr}; sub sp,#0x108`, and the arguments are [`super::blit565`]'s THIRTEEN
//! plus two: a per-sprite blend level and a per-call one. It is 4bpp-only (`cmp r3,#0xb`,
//! anything else leaves for `0x3064a`), index 0 is transparent, and the palette entry is
//! RGB565 — all exactly as in `blit565`. What it adds is that it READS the destination
//! pixel and blends through two 32-stride byte lookup tables, ~44 guest instructions per
//! pixel against `blit565`'s ~11:
//!
//! ```text
//!   sR = si>>11, sG = (si & 0x7e0)>>6, sB = si & 0x1f      // note: green HALVED to 5 bits
//!   dR = di>>11, dG = (di & 0x7e0)>>6, dB = di & 0x1f      // so one 32x32 table serves all
//!   r1 = T[t][sR][dR] ; g1 = T[t][sG][dG] ; b1 = T[t][sB][dB]
//!   if a > 0xfe { R,G,B = r1, g1, b1 }                     // 0xff = "no second stage"
//!   else        { R = U[a][r1][dR] ; G = U[a][g1][dG] ; B = U[a][b1][dB] }
//!   store  (R<<11) | ((G*2)<<5 & 0x7e0) | (B & 0x1f)
//! ```
//!
//! `t` is not the argument itself but `alpha_map[arg]`, a byte table in `.data`. All three
//! table addresses are per-build, so they arrive from TOML rather than being hardcoded
//! here — a second revision needs a new table row and no code.
//!
//! ── WHY THE TABLE READS CANNOT RUN OFF THE END ────────────────────────────────
//! A plane is 32*32 = 1024 bytes and stage 1 indexes it as `s*32 + d` with both operands
//! 5-bit, so stage 1 is in-plane by construction. Stage 2 indexes with a stage-1 RESULT,
//! which is a table BYTE and therefore only 5-bit if the table says so. Rather than assume
//! it, the whole 1024-byte plane is scanned once per call and the job is declined if any
//! entry exceeds 31. That makes every stage-2 index provably `<= 31*32+31`, i.e. in-plane
//! too, and it is 1 KB of work against a sprite's worth of pixels.
//!
//! ── SAFETY ───────────────────────────────────────────────────────────────────
//! Identical discipline to `blit565`, which this deliberately mirrors rather than
//! generalises (the two share a signature prefix, not a body):
//! * every refusal runs the displaced `push {r4-r7,lr}` and resumes two bytes on, so the
//!   worst case is the performance the build had before this file existed;
//! * all reads happen before any write, so a declined or failed call cannot leave a
//!   half-drawn sprite;
//! * anything whose semantics would have to be guessed is refused outright — a non-zero
//!   transform (the guest has a full separate copy of the loop per transform, starting at
//!   `0x2ec70`, and none of them was analysed), a non-4bpp format, negative origins, a row
//!   that runs past the destination stride, an unreadable table plane.

use alloc::vec;

use wie_util::{ByteRead, ByteWrite, Result};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// 4 bits per pixel, high nibble first — the only format this function draws
/// (`0x2e904 cmp r3,#0xb`).
const FMT_4BPP: u8 = 0x0b;

/// Identity. The guest keeps a whole copy of the blend loop per transform and this hook
/// only implements the first one.
const TRANSFORM_NONE: u32 = 0;

/// `a > 0xfe` is the guest's own test (`0x2e9c8 cmp r3,#0xfe; bgt`) for "skip the second
/// blend stage", i.e. 255 means the per-call level contributes nothing.
const ALPHA_OPAQUE: u32 = 0xfe;

/// One blend plane: a 32x32 byte matrix indexed `[src5][dst5]`.
const PLANE: usize = 32 * 32;

/// Largest table entry the second stage can index with. See the module header.
const MAX_ENTRY: u8 = 31;

/// An 8-bit index space, matching `blit565` — the guest reads the palette without
/// checking the length, so copy the whole window instead of second-guessing it.
const PALETTE_ENTRIES: usize = 256;

/// Where the three per-build tables live. Carried on the hook (from TOML) because they are
/// `.data`/`.bss` addresses of one revision, exactly like `blend565`'s `exit_pc`.
#[derive(Debug, Clone, Copy)]
pub struct Tables {
    /// `.data` byte table: the raw sprite-level argument indexes it to get `t`.
    pub alpha_map: u32,
    /// Stage-1 planes, indexed by `t`.
    pub blend_t: u32,
    /// Stage-2 planes, indexed by the per-call level `a`.
    pub blend_a: u32,
}

/// Arguments, read at the hook's entry SP — the prologue has not run, so the stack
/// arguments start at `SP + 0`. The first nine are `blit565`'s, in the same order and at
/// the same indices, which is the cross-check that this reading is right.
struct Args {
    dst: u32,
    dst_x: i32,
    dst_y: i32,
    w: i32,
    h: i32,
    dst_stride: i32,
    spr: u32,
    src_x: i32,
    src_y: i32,
    transform: u32,
    /// Per-call blend level. `> 0xfe` disables the second stage.
    alpha: u32,
    /// Per-sprite blend level, BEFORE `alpha_map` translates it.
    level: u32,
}

impl Args {
    fn read(core: &ArmCore) -> Result<Self> {
        let (r0, r1, r2, r3, sp) = {
            let inner = core.inner.lock();
            (
                inner.engine.reg_read(ArmRegister::R0),
                inner.engine.reg_read(ArmRegister::R1),
                inner.engine.reg_read(ArmRegister::R2),
                inner.engine.reg_read(ArmRegister::R3),
                inner.engine.reg_read(ArmRegister::SP),
            )
        };
        let arg = |i: u32| -> Result<u32> { wie_util::read_generic(core, sp + i * 4) };

        Ok(Self {
            dst: r0,
            dst_x: r1 as i32,
            dst_y: r2 as i32,
            w: r3 as i32,
            h: arg(0)? as i32,
            dst_stride: arg(1)? as i32,
            // arg(2) is a destination height the guest never reads.
            spr: arg(3)?,
            src_x: arg(4)? as i32,
            src_y: arg(5)? as i32,
            // arg(6) / arg(7) are an image width/height the guest never reads.
            transform: arg(8)?,
            alpha: arg(9)?,
            level: arg(10)?,
        })
    }
}

/// The sprite descriptor, LGT layout (direct pointers at +8/+0xc). Same fields and the
/// same `pixels = data + 2*palette_len + 9` as [`super::blit565`]; re-derived from this
/// function's own prologue (`0x2e8ee`..`0x2e932`) rather than assumed.
struct Sprite {
    pitch: i32,
    pixels: u32,
    palette: u32,
    format: u8,
}

impl Sprite {
    fn read(core: &ArmCore, spr: u32) -> Result<Self> {
        let data: u32 = wie_util::read_generic(core, spr + 0x8)?;
        let palette: u32 = wie_util::read_generic(core, spr + 0xc)?;
        let pitch: u16 = wie_util::read_generic(core, spr)?;
        let palette_len: u16 = wie_util::read_generic(core, spr + 0x6)?;
        let format: u8 = wie_util::read_generic(core, data)?;

        Ok(Self {
            pitch: pitch as i16 as i32,
            pixels: data.wrapping_add(2 * palette_len as i16 as i32 as u32).wrapping_add(9),
            palette,
            format,
        })
    }
}

/// Shares `blit565`'s refusal counters: both are "the host declined to draw a sprite", the
/// numbers are read together in the device log, and splitting them would make the existing
/// `blit` percentage mean something different depending on which build printed it.
const REFUSED_FORMAT: usize = 0;
const REFUSED_TRANSFORM: usize = 1;
const REFUSED_GEOMETRY: usize = 2;
const REFUSED_MEMORY: usize = 3;
const REFUSED_NULL: usize = 4;

fn refuse(reason: usize) {
    wie_util::stats::BLIT_REFUSED[reason].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub async fn handle_lgt(core: &mut ArmCore, hook_pc: u32, tables: Tables) -> Result<JumpTo> {
    use core::sync::atomic::Ordering::Relaxed;

    let args = Args::read(core)?;
    wie_util::stats::BLIT_CALLS.fetch_add(1, Relaxed);
    let pixels = (args.w.max(0) as u64) * (args.h.max(0) as u64);
    wie_util::stats::BLIT_PIXELS.fetch_add(pixels, Relaxed);
    wie_util::stats::BLIT_TRANSFORM[(args.transform as usize).min(9)].fetch_add(1, Relaxed);

    let lr = core.inner.lock().engine.reg_read(ArmRegister::LR);

    match try_blit(core, &args, tables) {
        Ok(true) => {
            wie_util::stats::BLIT_HLE.fetch_add(1, Relaxed);
            wie_util::stats::BLIT_PIXELS_HLE.fetch_add(pixels, Relaxed);
            Ok(JumpTo(lr))
        }
        Ok(false) | Err(_) => super::hook::resume_past_push(core, hook_pc),
    }
}

/// `Ok(true)` = drawn here. `Ok(false)` = refused, nothing written. `Err` = a guest read
/// failed, which also happens before any write.
fn try_blit(core: &mut ArmCore, args: &Args, tables: Tables) -> Result<bool> {
    use core::sync::atomic::Ordering::Relaxed;

    if !wie_util::stats::BLIT_HLE_ENABLED.load(Relaxed) {
        return Ok(false);
    }
    if args.dst == 0 || args.spr == 0 {
        refuse(REFUSED_NULL);
        return Ok(false);
    }
    if args.transform != TRANSFORM_NONE {
        refuse(REFUSED_TRANSFORM);
        return Ok(false);
    }

    let sprite = Sprite::read(core, args.spr)?;
    let fmt_slot = if sprite.format == FMT_4BPP { 0 } else { 2 };
    wie_util::stats::BLIT_FMT[fmt_slot].fetch_add(1, Relaxed);
    if sprite.format != FMT_4BPP {
        refuse(REFUSED_FORMAT);
        return Ok(false);
    }

    // Same rectangle discipline as `blit565`: one clean rectangle of the destination, so
    // every row is contiguous and no index can wrap. The guest wraps rather than clamping,
    // so anything outside this shape goes back to it instead of being approximated.
    let (w, h) = (args.w, args.h);
    if w <= 0
        || h <= 0
        || args.dst_stride <= 0
        || sprite.pitch <= 0
        || args.dst_x < 0
        || args.dst_y < 0
        || args.src_x < 0
        || args.src_y < 0
        || w > args.dst_stride
        || args.dst_x > args.dst_stride - w
    {
        refuse(REFUSED_GEOMETRY);
        return Ok(false);
    }

    // --- reads first: a failure here must not leave a half-drawn sprite ---

    // `t = alpha_map[level]`. The guest reads that byte unchecked; bound the index to the
    // byte the argument can actually be so a wild value cannot walk `.data`.
    if args.level > u32::from(u8::MAX) {
        refuse(REFUSED_GEOMETRY);
        return Ok(false);
    }
    let mut one = [0u8; 1];
    if core.read_bytes(tables.alpha_map.wrapping_add(args.level), &mut one).is_err() {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    }
    let t = u32::from(one[0]);

    let plane = |core: &mut ArmCore, base: u32, index: u32| -> Option<vec::Vec<u8>> {
        let mut buf = vec![0u8; PLANE];
        let at = base.wrapping_add(index.wrapping_mul(PLANE as u32));
        core.read_bytes(at, &mut buf).ok()?;
        // Everything the second stage indexes with comes out of a plane, so a plane that
        // can produce a value past 31 would put that stage's read outside the 1024 bytes
        // we hold. Refuse the call instead of guessing what the guest would have read.
        buf.iter().all(|&v| v <= MAX_ENTRY).then_some(buf)
    };

    let Some(table_t) = plane(core, tables.blend_t, t) else {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    };
    let two_stage = args.alpha <= ALPHA_OPAQUE;
    let table_a = if two_stage {
        match plane(core, tables.blend_a, args.alpha) {
            Some(p) => p,
            None => {
                refuse(REFUSED_MEMORY);
                return Ok(false);
            }
        }
    } else {
        vec::Vec::new()
    };

    let (w, h) = (w as u32, h as u32);
    let dst_stride = args.dst_stride as u32;
    // 4bpp packs two pixels per byte, so the row pitch halves — rounded UP, because an odd
    // pitch still needs the byte holding its last pixel. `phase` is which nibble the first
    // source pixel sits in; it is fixed for the whole blit, not per row.
    let src_stride = (sprite.pitch as u32).div_ceil(2);
    let phase = (args.src_x & 1) as u32;
    let src_row_bytes = (w + phase).div_ceil(2);

    let mut palette = vec![0u8; PALETTE_ENTRIES * 2];
    if core.read_bytes(sprite.palette, &mut palette).is_err() {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    }

    let src_origin = sprite
        .pixels
        .wrapping_add((args.src_x >> 1) as u32)
        .wrapping_add((args.src_y as u32).wrapping_mul(src_stride));
    let src_span = (h - 1) as usize * src_stride as usize + src_row_bytes as usize;
    let mut src = vec![0u8; src_span];
    if core.read_bytes(src_origin, &mut src).is_err() {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    }

    // The destination is read back because this blitter BLENDS with it — unlike `blit565`,
    // where the read-back only preserved transparent pixels.
    let dst_origin = args
        .dst
        .wrapping_add(2 * ((args.dst_y as u32).wrapping_mul(dst_stride).wrapping_add(args.dst_x as u32)));
    let row_bytes = w as usize * 2;
    let mut rows = vec![0u8; row_bytes * h as usize];
    for v in 0..h {
        let at = dst_origin.wrapping_add(2 * v * dst_stride);
        if core
            .read_bytes(at, &mut rows[v as usize * row_bytes..(v as usize + 1) * row_bytes])
            .is_err()
        {
            refuse(REFUSED_MEMORY);
            return Ok(false);
        }
    }

    // --- now the blit itself; from here on nothing can fail ---

    for v in 0..h as usize {
        let src_row = &src[v * src_stride as usize..][..src_row_bytes as usize];
        let dst_row = &mut rows[v * row_bytes..][..row_bytes];

        let mut put = |column: usize, index: u8| {
            // Index 0 is the one and only transparency.
            if index == 0 {
                return;
            }
            let entry = index as usize * 2;
            let si = u32::from(u16::from_le_bytes([palette[entry], palette[entry + 1]]));
            let di = u32::from(u16::from_le_bytes([dst_row[column * 2], dst_row[column * 2 + 1]]));
            let out = blend(si, di, &table_t, two_stage.then_some(table_a.as_slice()));
            dst_row[column * 2..column * 2 + 2].copy_from_slice(&out.to_le_bytes());
        };

        // Mirrors the guest byte for byte: an odd `srcX` starts on the LOW nibble of the
        // first byte, then whole bytes high-nibble-first, then a lone high nibble for an
        // odd remainder.
        let mut column = 0usize;
        let mut byte = 0usize;
        let mut remaining = w as usize;
        if phase == 1 {
            put(column, src_row[byte] & 0xf);
            byte += 1;
            column += 1;
            remaining -= 1;
        }
        for _ in 0..remaining / 2 {
            let packed = src_row[byte];
            byte += 1;
            put(column, packed >> 4);
            column += 1;
            put(column, packed & 0xf);
            column += 1;
        }
        if remaining % 2 == 1 {
            put(column, src_row[byte] >> 4);
        }
    }

    for v in 0..h {
        let at = dst_origin.wrapping_add(2 * v * dst_stride);
        core.write_bytes(at, &rows[v as usize * row_bytes..(v as usize + 1) * row_bytes])?;
    }

    Ok(true)
}

/// One pixel, transcribed from `0x2ea4e`..`0x2eb8c`.
///
/// The green channel is deliberately taken as `(p & 0x7e0) >> 6` and written back doubled:
/// that is what lets ONE 32x32 table serve all three channels, and it is the guest's own
/// `asrs r3,r3,#6` / `lsls r0,r3,#1` pair, not a simplification of it.
fn blend(si: u32, di: u32, table_t: &[u8], table_a: Option<&[u8]>) -> u16 {
    let look = |t: &[u8], s: u32, d: u32| u32::from(t[(s as usize) * 32 + d as usize]);

    let (s_r, s_g, s_b) = (si >> 11, (si & 0x7e0) >> 6, si & 0x1f);
    let (d_r, d_g, d_b) = (di >> 11, (di & 0x7e0) >> 6, di & 0x1f);

    let (mut r, mut g, mut b) = (look(table_t, s_r, d_r), look(table_t, s_g, d_g), look(table_t, s_b, d_b));
    if let Some(table_a) = table_a {
        r = look(table_a, r, d_r);
        g = look(table_a, g, d_g);
        b = look(table_a, b, d_b);
    }

    // The guest's recombination: red unshifted into bits 11.., green doubled to six bits
    // and masked into 5..10, blue masked to five bits, then a `strh` truncation.
    (((r << 11) | (((g * 2) << 5) & 0x7e0) | (b & 0x1f)) & 0xffff) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Green's LOW BIT is gone unconditionally — a property of the guest, not of this
    /// transcription. Both operands are halved to index a 32-wide table and the result is
    /// doubled on the way out, so the six-bit green field this blitter writes is always
    /// even. Every expectation below is stated against that mask, and the pixel-identical
    /// probe A/B (MIGRATION_LGT.md §2.20) is what says the game really does this.
    const GREEN_LSB: u32 = 0xffdf;

    /// A plane that is the identity in `dst` (`T[s][d] = d`) leaves the destination pixel
    /// alone through both stages — the property that says the index arithmetic
    /// (`s*32 + d`) is the right way round. Swapping the two operands still type-checks
    /// and still looks like a blend, so this is the test that catches it.
    #[test]
    fn a_dst_identity_plane_is_a_no_op() {
        let plane: vec::Vec<u8> = (0..PLANE).map(|i| (i % 32) as u8).collect();
        for di in [0x0000u32, 0xffff, 0x1234, 0xf81f] {
            let expect = di & GREEN_LSB;
            assert_eq!(u32::from(blend(0x0821, di, &plane, None)), expect, "dst {di:#06x}");
            let two = blend(0x0821, di, &plane, Some(&plane));
            assert_eq!(u32::from(two), expect, "dst {di:#06x}, two stages");
        }
    }

    /// ...and a plane that is the identity in `src` reproduces the SOURCE pixel, which is
    /// what "fully opaque" means in this encoding.
    #[test]
    fn a_src_identity_plane_reproduces_the_source() {
        let plane: vec::Vec<u8> = (0..PLANE).map(|i| (i / 32) as u8).collect();
        for si in [0x0000u32, 0xffff, 0x1234, 0xf81f] {
            assert_eq!(u32::from(blend(si, 0x5a5a, &plane, None)), si & GREEN_LSB, "src {si:#06x}");
        }
    }

    /// The second stage indexes with a first-stage RESULT, so the whole safety argument is
    /// "no plane entry exceeds 31". Assert the bound the loader checks against is the one
    /// that keeps `look` inside a plane.
    #[test]
    fn the_entry_bound_keeps_the_second_stage_in_plane() {
        assert_eq!((MAX_ENTRY as usize) * 32 + 31, PLANE - 1);
    }
}
