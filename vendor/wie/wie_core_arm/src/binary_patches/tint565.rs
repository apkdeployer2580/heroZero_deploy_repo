//! Host-side `tintRect` — the game's per-pixel LUT COLOUR TRANSFORM of a rectangle
//! already on screen, in RGB565. The fourth and (so far) last of its pixel loops.
//!
//! ── WHY THIS EXISTS (2026-08-15, 49차) ───────────────────────────────────────
//! Player: "다른건 다 문제없는데 크리티컬이 터질때만 살짝 느려져." [`super::blitlut565`]
//! was supposed to have closed that in §2.20 and had not, so this time the frame drop was
//! MEASURED on the device instead of reasoned about, with a per-tick hitch report
//! (`wie_android::emulator::report_hitch`) plus the guest PC profiler:
//!
//! ```text
//!   normal tick   25 ms   inst   0.65 M   blit 569/576   hot pc 0x8=60% (idle)
//!   crit tick    124 ms   inst  16.05 M   blit 573/582   hot pc 0x2b998 0x2b99a
//!                                                               0x2b99e 0x2b9ac …
//! ```
//!
//! Twenty-five times the guest instructions for the SAME number of blits, the same
//! ~140 K pixels, the same ~1,300 SVCs, three allocations and no sound parse — i.e. a
//! pure guest compute loop that never talks to the host, which is why every counter
//! except the instruction count looked identical. The hot PCs are all inside
//! `0x2b98e..0x2b9ea`.
//!
//! That range was NOT covered by the blend hook next door: [`super::blend565`] is
//! `blendRect` (`0x2b700`, hooked at `0x2b786`, tail `0x2b8d6`), and this is the SEPARATE
//! function that starts right after it at `0x2b8f8`.
//!
//! ── THE GUEST FUNCTION ───────────────────────────────────────────────────────
//! `tintRect(gc, x, y, w, h, level, selR, selG, selB)` = `0x2b8f8`
//! (`push {r4-r7,lr}; sub sp,#0x34`). It takes NO source pixels: it re-colours what is
//! already in the destination. Before the loop it zero-extends `level` (returning
//! immediately when it is `0xff`), fills a graphics context at `sp+0x1c` via `0x2ad6c`,
//! clamps a negative x/y, clips w/h against the context, arithmetic-shifts each of the
//! three channel selectors right by 3, computes the first row pointer as
//! `stride*y + bpp*x + base`, and checks `*(u8*)0x150ecc6` for 1 (= RGB565); any other
//! format takes the 32-bit path at `0x2ba00`, which this hook never sees.
//!
//! The hook sits on the OUTER loop's first instruction, `0x2b982`
//! (`ldr r1,[sp,#8]`, halfword `0x9902`), which `0x2b9fc` branches back to — the same
//! shape as the blend hook, so one call finishes the whole rectangle:
//!
//! | slot | meaning |
//! |---|---|
//! | `[sp+0x00]` | row stride in BYTES |
//! | `[sp+0x08]` | current row's BYTE pointer |
//! | `[sp+0x10]` | rows left |
//! | `[sp+0x14]` | level, 0..254 (the guest already returned on 255) |
//! | `[sp+0x18]` | pixels per row (already clipped) |
//! | `[sp+0x50]` / `[sp+0x54]` / `[sp+0x58]` | R / G / B selector, ALREADY `>>3` |
//!
//! ── THE PIXEL MATH IS THE GUEST'S, TRANSCRIBED ───────────────────────────────
//! Unpack as `0x2b992`..`0x2b9c0` does — and note the GREEN field is halved to five bits
//! (`(p & 0x7e0) >> 6`, not `>> 5`), which is the same deliberate loss
//! [`super::blitlut565`] documents for its own tables. Each channel then indexes one
//! byte table:
//!
//! ```text
//!   nR = LUT[level*1024 + (p >> 11)        * 32 + selR]
//!   nG = LUT[level*1024 + ((p >> 6) & 0x1f) * 32 + selG]
//!   nB = LUT[level*1024 + (p & 0x1f)        * 32 + selB]
//!   out = (nR << 11) | ((nG << 6) & 0x07e0) | (nB & 0x1f)      ; truncated by `strh`
//! ```
//!
//! `nR << 11` is written unmasked exactly as the guest writes it; the `strh` drops
//! everything above bit 15, which is the same result as `(nR & 0x1f) << 11`. It is
//! transcribed the guest's way so the two cannot drift.
//!
//! ★ `LUT` is the SAME table [`super::blitlut565`] already receives as `blend_a`
//! (`0x150eccc` on 01.00.07) — an independent cross-check that this reading of the
//! indexing is right, since that hook derived the identical `level`/`sel` layout from a
//! different function. The game BUILDS those planes at runtime, so the 1 KB plane for
//! this call's `level` is read per call and nothing is cached across calls.
//!
//! ── SAFETY ───────────────────────────────────────────────────────────────────
//! Identical discipline to [`super::blend565`], which this deliberately mirrors:
//! * the hook displaces a 2-byte `ldr r1,[sp,#8]`, so declining performs that load
//!   itself and resumes two bytes on — otherwise the outer loop runs with a stale `r1`;
//! * every implausible frame (zero/huge extents, a null row pointer, a selector outside
//!   the table's 32 entries) declines, so the worst case is the performance the build had
//!   before this file existed;
//! * the whole 1 KB plane is read BEFORE any pixel is written, so an unmapped table
//!   cannot leave a half-tinted rectangle.
//!
//! On the fast path the loop state is written back as the guest's own loop would have
//! left it (rows 0, row pointer advanced) and control goes to `exit_pc` — the function's
//! shared tail at `0x2ba9c`. That tail is reached by three DO-NOTHING early exits
//! (`0x2b912` when level is 255, `0x2b97e` when the height is not positive, `0x2ba02`
//! likewise on the 32-bit path), which is the proof that it cannot depend on any register
//! or stack state the loop would have produced.

use alloc::format;

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldr r1, [sp, #8]` — the instruction the hook replaces. Checked at install time (see
/// `hook::apply_hooks`) because emulating a load that is not there on the decline path
/// would corrupt the outer loop silently.
pub const EXPECTED_HEAD: [u8; 2] = [0x02, 0x99];

/// Frame slots, all SP-relative at the hook site.
const OFF_STRIDE: u32 = 0x00;
const OFF_ROWPTR: u32 = 0x08;
const OFF_ROWS: u32 = 0x10;
const OFF_LEVEL: u32 = 0x14;
const OFF_WIDTH: u32 = 0x18;
const OFF_SEL_R: u32 = 0x50;
const OFF_SEL_G: u32 = 0x54;
const OFF_SEL_B: u32 = 0x58;

/// Refusal bounds, matching [`super::blend565`]. Generous — they exist to reject a frame
/// we are misreading, not to second-guess the game.
const MAX_EXTENT: u32 = 4096;
const MAX_STRIDE: u32 = 1 << 16;

/// One `level` plane: 32 channel values x 32 selector slots, one byte each. The guest
/// indexes `LUT + level*1024 + channel*32 + sel`, so this is exactly the window a single
/// call can touch.
const PLANE_BYTES: usize = 32 * 32;

/// `level` 255 is the guest's own "do nothing" sentinel and returns before the loop, so
/// the hook can only ever see 0..=254.
const MAX_LEVEL: u32 = 0xfe;

/// Selectors are `arg >> 3` of a byte-ish argument and index the 32 slots of a row.
const MAX_SEL: u32 = 31;

pub async fn handle_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32, lut: u32) -> Result<JumpTo> {
    let sp = core.inner.lock().engine.reg_read(ArmRegister::SP);
    let slot = |core: &mut ArmCore, off: u32| -> Result<u32> {
        let mut b = [0u8; 4];
        core.read_bytes(sp + off, &mut b)?;
        Ok(u32::from_le_bytes(b))
    };

    let rows = slot(core, OFF_ROWS)?;
    let width = slot(core, OFF_WIDTH)?;
    let mut rowptr = slot(core, OFF_ROWPTR)?;
    let stride = slot(core, OFF_STRIDE)?;
    let level = slot(core, OFF_LEVEL)?;
    let sel_r = slot(core, OFF_SEL_R)?;
    let sel_g = slot(core, OFF_SEL_G)?;
    let sel_b = slot(core, OFF_SEL_B)?;

    // The same master switch the sprite blitters obey, so `wie_probe --hle 0` is a true
    // baseline for the pixel-identical A/B — and so the whole thing can be turned off on
    // a device without reinstalling.
    if !wie_util::stats::BLIT_HLE_ENABLED.load(core::sync::atomic::Ordering::Relaxed) {
        return decline(core, hook_pc, rowptr);
    }

    let plausible = (1..=MAX_EXTENT).contains(&rows)
        && (1..=MAX_EXTENT).contains(&width)
        && (2..=MAX_STRIDE).contains(&stride)
        && level <= MAX_LEVEL
        && sel_r <= MAX_SEL
        && sel_g <= MAX_SEL
        && sel_b <= MAX_SEL
        && rowptr != 0
        && lut != 0
        // The row must fit inside the stride, or we are not looking at a framebuffer.
        && width * 2 <= stride;
    wie_util::stats::add_tint(u64::from(rows) * u64::from(width), plausible);
    if !plausible {
        tracing::trace!(
            "tint565 declined: rows={rows} width={width} stride={stride} level={level} ptr={rowptr:#x} sel=({sel_r},{sel_g},{sel_b})"
        );
        return decline(core, hook_pc, rowptr);
    }

    // The whole plane this call can index, read up front: three lookups per pixel would
    // otherwise be three guest reads per pixel, and a failure halfway would leave the
    // rectangle half tinted.
    let mut plane = [0u8; PLANE_BYTES];
    core.read_bytes(lut + level * PLANE_BYTES as u32, &mut plane)?;
    let map = |channel: u32, sel: u32| u32::from(plane[(channel * 32 + sel) as usize]);

    let mut row_bytes = alloc::vec![0u8; (width * 2) as usize];
    for _ in 0..rows {
        core.read_bytes(rowptr, &mut row_bytes)?;
        for px in row_bytes.chunks_exact_mut(2) {
            let p = u32::from(u16::from_le_bytes([px[0], px[1]]));
            let n_r = map(p >> 11, sel_r);
            // Green is deliberately narrowed to five bits before the lookup — the guest
            // shifts the 6-bit field right by SIX, not five.
            let n_g = map((p >> 6) & 0x1f, sel_g);
            let n_b = map(p & 0x1f, sel_b);
            // The guest's own recombination, including the unmasked red that the `strh`
            // then truncates to 16 bits.
            let out = ((n_r << 11) | ((n_g << 6) & 0x07e0) | (n_b & 0x1f)) as u16;
            px.copy_from_slice(&out.to_le_bytes());
        }
        core.write_bytes(rowptr, &row_bytes)?;
        rowptr = rowptr.wrapping_add(stride);
    }

    // Leave the loop variables where the guest's loop would have: it exits with the row
    // counter at 0 and the row pointer one stride past the last row.
    core.write_bytes(sp + OFF_ROWS, &0u32.to_le_bytes())?;
    core.write_bytes(sp + OFF_ROWPTR, &rowptr.to_le_bytes())?;
    Ok(JumpTo(exit_pc))
}

/// Perform the displaced `ldr r1, [sp, #8]` and resume at the next instruction, so the
/// guest's own loop runs exactly as if the hook were not installed.
fn decline(core: &mut ArmCore, hook_pc: u32, rowptr: u32) -> Result<JumpTo> {
    core.inner.lock().engine.reg_write(ArmRegister::R1, rowptr);
    // `hook_pc` carries the Thumb bit and points AT the SVC we wrote over a 2-byte
    // instruction, so the next instruction is two bytes on.
    Ok(JumpTo(hook_pc + 2))
}

/// Install-time guard, mirroring the blend hook's.
pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "tint565 hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (ldr r1,[sp,#8]), found {head:02x?}"
    )))
}
