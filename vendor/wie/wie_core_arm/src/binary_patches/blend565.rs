//! Host-side `blendRect` — the game's semi-transparent rectangle fill, in RGB565.
//!
//! ── WHY THIS EXISTS (MIGRATION_LGT.md §2.17) ─────────────────────────────────
//! Profiling an LGT field run put **~21 % of all guest samples** inside one tight
//! loop at `0x2b790..0x2b816`, spread almost perfectly evenly across its ~35
//! instructions — the signature of a loop body, not a hot branch. That loop is a
//! per-pixel alpha blend costing ~44 guest instructions PER PIXEL, and the game uses
//! it for every menu panel, dialogue bar, HUD panel and screen fade. One full-screen
//! 240x320 fade is ~3.4 M guest instructions, against ~1.0 M for an entire ordinary
//! field frame. It is a completely separate function from the palettised sprite
//! blitter [`super::blit565`] already handles, and nothing was hooking it.
//!
//! ── THE GUEST FUNCTION ───────────────────────────────────────────────────────
//! `blendRect` = `0x2b700` (`push {r4-r7,lr}; sub sp,#0x5c`). Before the loop it
//! zero-extends the alpha byte (returning immediately when it is 0), fills a graphics
//! context at `sp+0x2c`, clamps a negative x/y, clips the width/height against the
//! context, computes the first row's byte pointer as `stride*y + bpp*x + base`, and
//! checks a global format byte for 1 (= RGB565); any other format takes a different
//! path at `0x2b846` which this hook never sees.
//!
//! The hook sits on the OUTER loop's first instruction, `0x2b786`
//! (`ldr r4,[sp,#0x28]`, halfword `0x9c0a`), which `0x2b836` branches back to. Every
//! parameter is already on the frame by then — in particular `0x2b784` has just
//! stored the remaining row count — so one hook can finish the whole rectangle:
//!
//! | slot | meaning |
//! |---|---|
//! | `[sp+0x20]` | rows left |
//! | `[sp+0x28]` | pixels per row (already clipped) |
//! | `[sp+0x1c]` | current row's BYTE pointer |
//! | `[sp+0x18]` | row stride in BYTES |
//! | `[sp+0x24]` | alpha, 0..255 |
//! | `[sp+0x78]` / `[sp+0x7c]` / `[sp+0x80]` | source R / G / B, 0..255 |
//!
//! ── THE PIXEL MATH IS THE GUEST'S, TRANSCRIBED ───────────────────────────────
//! Unpack exactly as `0x2b790`..`0x2b7ac` does — `dR = (p & 0xf800) >> 8`,
//! `dG = (p & 0x07e0) >> 3`, `dB = (p & 0x1f) << 3` — accumulate
//! `src*alpha + dst*(255-alpha)` per channel, then recombine the way
//! `0x2b7f8`..`0x2b810` does:
//!
//! * red keeps its accumulator UNSHIFTED and masks with the pc-relative literal
//!   `0xfffff800`; bit 16 then falls off because the store is a `strh`. That is the
//!   same result as `(acc >> 11 & 0x1f) << 11`, and it is written the guest's way so
//!   the two cannot drift.
//! * green is `((acc >> 8) << 3) & 0x07e0`, blue is `(acc >> 11) & 0x1f`.
//!
//! Since `alpha + (255 - alpha) == 255` and both operands are <= 255, no accumulator
//! can exceed `255 * 255 = 0xfe01`, so the red field stays inside five bits.
//!
//! ── SAFETY ───────────────────────────────────────────────────────────────────
//! The hook displaces a 2-byte `ldr r4,[sp,#0x28]`. Declining the job therefore has
//! to perform that load itself and resume two bytes on, exactly as
//! [`super::hook::resume_past_push`] does for the blitter's prologue — otherwise the
//! outer loop would run with a stale `r4`. Every implausible frame (zero/huge extents,
//! a null row pointer, an out-of-range alpha) declines, so the worst case is the
//! performance the build had before this file existed.
//!
//! On the fast path the loop state is written back as the guest's own loop would have
//! left it (rows 0, row pointer advanced) even though the exit target does not read
//! it, and control goes to `exit_pc` — the function's shared tail. That tail is
//! reached by the two do-nothing early exits (`0x2b71a` when alpha is 0, `0x2b776`
//! when the height is not positive), which is the proof that it cannot depend on any
//! register or stack state the loop would have produced.

use alloc::format;

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldr r4, [sp, #0x28]` — the instruction the hook replaces. Checked at install time
/// (see `hook::apply_hooks`) because emulating a load that is not there on the decline
/// path would corrupt the outer loop silently.
pub const EXPECTED_HEAD: [u8; 2] = [0x0a, 0x9c];

/// Frame slots, all SP-relative at the hook site.
const OFF_STRIDE: u32 = 0x18;
const OFF_ROWPTR: u32 = 0x1c;
const OFF_ROWS: u32 = 0x20;
const OFF_ALPHA: u32 = 0x24;
const OFF_WIDTH: u32 = 0x28;
const OFF_SRC_R: u32 = 0x78;
const OFF_SRC_G: u32 = 0x7c;
const OFF_SRC_B: u32 = 0x80;

/// Refusal bounds. Generous — they exist to reject a frame we are misreading, not to
/// second-guess the game. The guest screen is 240x320 and the framebuffer stride a few
/// hundred bytes, so anything past these is not a rectangle.
const MAX_EXTENT: u32 = 4096;
const MAX_STRIDE: u32 = 1 << 16;

pub async fn handle_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
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
    let alpha = slot(core, OFF_ALPHA)?;
    let src_r = slot(core, OFF_SRC_R)?;
    let src_g = slot(core, OFF_SRC_G)?;
    let src_b = slot(core, OFF_SRC_B)?;

    let plausible = (1..=MAX_EXTENT).contains(&rows)
        && (1..=MAX_EXTENT).contains(&width)
        && (2..=MAX_STRIDE).contains(&stride)
        && (1..=255).contains(&alpha)
        && rowptr != 0
        && src_r <= 255
        && src_g <= 255
        && src_b <= 255
        // The row must fit inside the stride, or we are not looking at a framebuffer.
        && width * 2 <= stride;
    if !plausible {
        tracing::trace!(
            "blend565 declined: rows={rows} width={width} stride={stride} alpha={alpha} ptr={rowptr:#x} rgb=({src_r},{src_g},{src_b})"
        );
        return decline(core, hook_pc, width);
    }

    let inv = 255 - alpha;
    let mut row_bytes = alloc::vec![0u8; (width * 2) as usize];
    for _ in 0..rows {
        core.read_bytes(rowptr, &mut row_bytes)?;
        for px in row_bytes.chunks_exact_mut(2) {
            let p = u32::from(u16::from_le_bytes([px[0], px[1]]));
            let dst_r = (p & 0xf800) >> 8;
            let dst_g = (p & 0x07e0) >> 3;
            let dst_b = (p & 0x001f) << 3;
            let acc_r = src_r * alpha + dst_r * inv;
            let acc_g = src_g * alpha + dst_g * inv;
            let acc_b = src_b * alpha + dst_b * inv;
            // The guest's own recombination, including the unshifted red mask that the
            // `strh` then truncates to 16 bits.
            let out = ((acc_r & 0xffff_f800) | (((acc_g >> 8) << 3) & 0x07e0) | ((acc_b >> 11) & 0x1f)) as u16;
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

/// Perform the displaced `ldr r4, [sp, #0x28]` and resume at the next instruction, so
/// the guest's own loop runs exactly as if the hook were not installed.
fn decline(core: &mut ArmCore, hook_pc: u32, width: u32) -> Result<JumpTo> {
    core.inner.lock().engine.reg_write(ArmRegister::R4, width);
    // `hook_pc` carries the Thumb bit and points AT the SVC we wrote over a 2-byte
    // instruction, so the next instruction is two bytes on.
    Ok(JumpTo(hook_pc + 2))
}

/// Install-time guard, mirroring the blitter's prologue check.
pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "blend565 hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (ldr r4,[sp,#0x28]), found {head:02x?}"
    )))
}
