//! Host reimplementation of the guest's palettised sprite blitter.
//!
//! ── WHY ──────────────────────────────────────────────────────────────────────
//! Measured 2026-08-06 on the real game with the headless probe: a field frame costs
//! **2.46 M guest instructions**, and a sampled guest profile puts **26% of them inside
//! two inner loops of one function** at guest `0x110fe4` — 17.8% at `0x111078` (left to
//! right) and 8.0% at `0x11138c` (right to left). The game never calls
//! `MC_grpDrawImage`: it fetches the screen framebuffer pointer once and rasterises the
//! entire world itself, one indexed pixel at a time, through this routine. So this
//! function IS the map rendering, and interpreting it is the per-map lag.
//!
//! ── WHAT THE GUEST DOES (reverse engineered, see the table below) ────────────
//! 13 arguments. Reads an 8-bit (`fmt 0x0c`) or 4-bit (`fmt 0x0b`) index per pixel,
//! looks it up in an RGB565 palette, and stores it — skipping index 0, which is the
//! only transparency there is. No blending, no scaling, no clipping (the caller has
//! already clipped), no side effects other than the pixel stores, and no writes to the
//! sprite descriptor. That makes it a pure function of its arguments and guest memory,
//! which is what makes replacing it safe.
//!
//! ── HOW THIS IS GATED ────────────────────────────────────────────────────────
//! Every refusal falls back to the real guest code, so the gate can be as narrow as we
//! like and the worst case is "no faster than before". The fallback runs the prologue
//! the SVC displaced and jumps past it, so the guest never notices.
//!
//! The gate deliberately refuses anything whose exact semantics we would have to guess:
//! * the transposing transforms 4..=8 (see `HOST_TRANSFORMS`),
//! * `w <= 0`, which the 4bpp path turns into a *two pixel overrun* the guest really can
//!   produce at a clipped left edge; handing it back keeps that bug bit-exact,
//! * negative origins, non-positive strides, or a row that runs past the destination
//!   stride, none of which the clipped call sites should produce,
//! * anything whose source, palette or destination bytes are not readable.
//!
//! ★ Reads happen before any write, so a refusal can never leave a half-drawn sprite.

use alloc::vec;

use wie_util::{ByteRead, ByteWrite, Result};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// 8 bits per pixel, one palette index per byte.
const FMT_8BPP: u8 = 0x0c;
/// 4 bits per pixel, high nibble first.
const FMT_4BPP: u8 = 0x0b;

/// Identity.
const TRANSFORM_NONE: u32 = 0;
/// Mirror horizontally: the destination column walks backwards.
const TRANSFORM_MIRROR_X: u32 = 1;
/// Mirror vertically: the destination row walks backwards.
const TRANSFORM_MIRROR_Y: u32 = 2;
/// Both — a 180 degree rotation.
const TRANSFORM_ROT180: u32 = 3;

/// The four the host draws. Codes 4..=8 are the transposing ones (90 degree rotations
/// and diagonal flips); they are unreachable in this build — all three live call sites
/// pass a constant angle of 0 — so implementing them would be untested code that only a
/// future build could exercise. They fall back instead. Measured share of real field
/// play: 0 = 56.6%, 1 = 31.3%, 2 = 5.9%, 3 = 6.2%, 4..=8 = 0%.
const HOST_TRANSFORMS: [u32; 4] = [TRANSFORM_NONE, TRANSFORM_MIRROR_X, TRANSFORM_MIRROR_Y, TRANSFORM_ROT180];

/// An 8bpp index is a byte, so 256 entries covers every index the guest can produce —
/// including the ones past the palette's real length, which it reads without checking.
/// Copying the whole window reproduces that read instead of second-guessing it.
const PALETTE_ENTRIES: usize = 256;

/// Arguments, read at the hook's entry SP.
///
/// The hook replaces the function's FIRST instruction, so the prologue has not run and
/// SP is still the caller's: the stack arguments start at `SP + 0`, not at the `SP +
/// 0x80` they sit at inside the function body.
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
        })
    }
}

/// How the sprite descriptor's pixel/palette fields are reached. The two carrier
/// builds share the descriptor layout (pitch at +0, palette length at +6, pixel block
/// at +8, palette block at +0xc) and the pixel arithmetic (`pixels = data +
/// 2*palette_len + 9`, verified instruction-by-instruction in both disassemblies), but
/// differ in ONE thing: what the +8/+0xc words actually are.
#[derive(Clone, Copy)]
pub enum SpriteLayout {
    /// KTF `client.bin`: both words are HANDLES — double indirections whose target
    /// block starts with an 8-byte allocator header.
    KtfHandles,
    /// LGT `binary.mod` (Hero3 01.00.07): both words are direct pointers; the palette
    /// pointer aims straight at the RGB565 entries. Verified against the guest code at
    /// `0x2dd7c` (`ldr r0,[r2,#8]` / `ldr r4,[r2,#0xc]`, used with no further deref).
    LgtDirect,
}

/// The sprite descriptor, passed by value at the API level and by pointer to here.
struct Sprite {
    /// Source row pitch in PIXELS, signed. NOT the image width — they are separate
    /// fields, and using the width instead is the classic stride bug that shears every
    /// sprite diagonally.
    pitch: i32,
    /// First pixel byte.
    pixels: u32,
    /// First palette byte; entries are RGB565, stored verbatim, no channel swizzle.
    palette: u32,
    format: u8,
}

impl Sprite {
    fn read(core: &ArmCore, spr: u32, layout: SpriteLayout) -> Result<Self> {
        let (data, palette) = match layout {
            SpriteLayout::KtfHandles => {
                // Both handles are double indirections whose block starts with an
                // 8-byte allocator header.
                let pixel_handle: u32 = wie_util::read_generic(core, spr + 0x8)?;
                let data: u32 = wie_util::read_generic(core, pixel_handle)?;
                let palette_handle: u32 = wie_util::read_generic(core, spr + 0xc)?;
                let palette: u32 = wie_util::read_generic(core, palette_handle)?;
                (data + 8, palette + 8)
            }
            SpriteLayout::LgtDirect => {
                let data: u32 = wie_util::read_generic(core, spr + 0x8)?;
                let palette: u32 = wie_util::read_generic(core, spr + 0xc)?;
                (data, palette)
            }
        };

        let pitch: u16 = wie_util::read_generic(core, spr)?;
        let palette_len: u16 = wie_util::read_generic(core, spr + 0x6)?;
        let format: u8 = wie_util::read_generic(core, data)?;

        Ok(Self {
            pitch: pitch as i16 as i32,
            // Header byte, then the palette, then the pixels — all in one block.
            pixels: data.wrapping_add(2 * palette_len as i16 as i32 as u32).wrapping_add(9),
            palette,
            format,
        })
    }
}

/// Why the gate refused. Indices match `wie_util::stats::BLIT_REFUSED`.
const REFUSED_FORMAT: usize = 0;
const REFUSED_TRANSFORM: usize = 1;
const REFUSED_GEOMETRY: usize = 2;
const REFUSED_MEMORY: usize = 3;
const REFUSED_NULL: usize = 4;

fn refuse(reason: usize) {
    wie_util::stats::BLIT_REFUSED[reason].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub async fn handle(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    handle_with_layout(core, hook_pc, lr, SpriteLayout::KtfHandles).await
}

pub async fn handle_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    handle_with_layout(core, hook_pc, lr, SpriteLayout::LgtDirect).await
}

async fn handle_with_layout(core: &mut ArmCore, hook_pc: u32, _lr: u32, layout: SpriteLayout) -> Result<JumpTo> {
    use core::sync::atomic::Ordering::Relaxed;

    let args = Args::read(core)?;
    wie_util::stats::BLIT_CALLS.fetch_add(1, Relaxed);
    let pixels = (args.w.max(0) as u64) * (args.h.max(0) as u64);
    wie_util::stats::BLIT_PIXELS.fetch_add(pixels, Relaxed);
    wie_util::stats::BLIT_TRANSFORM[(args.transform as usize).min(9)].fetch_add(1, Relaxed);

    let lr = core.inner.lock().engine.reg_read(ArmRegister::LR);

    match try_blit(core, &args, layout) {
        Ok(true) => {
            wie_util::stats::BLIT_HLE.fetch_add(1, Relaxed);
            wie_util::stats::BLIT_PIXELS_HLE.fetch_add(pixels, Relaxed);
            Ok(JumpTo(lr))
        }
        // Refused, or a guest-memory read failed before anything was written: run the
        // displaced prologue and let the real code do it.
        Ok(false) | Err(_) => fall_back(core, hook_pc),
    }
}

/// Emulate the `push {r4, r5, r6, r7, lr}` the SVC overwrote, then resume at the
/// instruction after it. Shared with the other function-entry hooks — see
/// `super::hook::resume_past_push`.
fn fall_back(core: &mut ArmCore, hook_pc: u32) -> Result<JumpTo> {
    super::hook::resume_past_push(core, hook_pc)
}

/// `Ok(true)` = drawn here. `Ok(false)` = refused, nothing written. `Err` = a guest read
/// failed, which also happens before any write.
fn try_blit(core: &mut ArmCore, args: &Args, layout: SpriteLayout) -> Result<bool> {
    use core::sync::atomic::Ordering::Relaxed;

    if !wie_util::stats::BLIT_HLE_ENABLED.load(Relaxed) {
        return Ok(false);
    }
    if args.dst == 0 || args.spr == 0 {
        refuse(REFUSED_NULL);
        return Ok(false);
    }

    let sprite = Sprite::read(core, args.spr, layout)?;
    let fmt_slot = match sprite.format {
        FMT_4BPP => 0,
        FMT_8BPP => 1,
        _ => 2,
    };
    wie_util::stats::BLIT_FMT[fmt_slot].fetch_add(1, Relaxed);

    if sprite.format != FMT_8BPP && sprite.format != FMT_4BPP {
        refuse(REFUSED_FORMAT);
        return Ok(false);
    }
    if !HOST_TRANSFORMS.contains(&args.transform) {
        refuse(REFUSED_TRANSFORM);
        return Ok(false);
    }

    // Everything below keeps the blit inside one clean rectangle of the destination, so
    // the byte ranges are contiguous per row and no index can wrap. The guest wraps
    // instead of clamping, so anything outside this shape goes back to the guest rather
    // than being approximated.
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

    let (w, h) = (w as u32, h as u32);
    let dst_stride = args.dst_stride as u32;
    let four_bpp = sprite.format == FMT_4BPP;
    // 4bpp packs two pixels per byte, so the row pitch halves — rounded UP, because an
    // odd pitch still needs the byte holding its last pixel. `phase` is which nibble the
    // first source pixel sits in; it is fixed for the whole blit, not per row.
    let src_stride = if four_bpp {
        (sprite.pitch as u32).div_ceil(2)
    } else {
        sprite.pitch as u32
    };
    let phase = if four_bpp { (args.src_x & 1) as u32 } else { 0 };
    let src_row_bytes = if four_bpp { (w + phase).div_ceil(2) } else { w };

    // --- reads first: a failure here must not leave a half-drawn sprite ---

    let mut palette = vec![0u8; PALETTE_ENTRIES * 2];
    if core.read_bytes(sprite.palette, &mut palette).is_err() {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    }

    let src_origin = sprite
        .pixels
        .wrapping_add(if four_bpp { (args.src_x >> 1) as u32 } else { args.src_x as u32 })
        .wrapping_add((args.src_y as u32).wrapping_mul(src_stride));
    let src_span = (h - 1) as usize * src_stride as usize + src_row_bytes as usize;
    let mut src = vec![0u8; src_span];
    if core.read_bytes(src_origin, &mut src).is_err() {
        refuse(REFUSED_MEMORY);
        return Ok(false);
    }

    // The destination rows are read back so untouched (transparent) pixels survive; the
    // guest stores per pixel, we store per row.
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

    // The four host transforms are exactly "walk the destination columns and/or rows
    // backwards"; the source is always read forwards. That is why they cost nothing here
    // and 8% of the frame in the guest.
    let mirror_x = args.transform == TRANSFORM_MIRROR_X || args.transform == TRANSFORM_ROT180;
    let mirror_y = args.transform == TRANSFORM_MIRROR_Y || args.transform == TRANSFORM_ROT180;
    for v in 0..h as usize {
        let src_row = &src[v * src_stride as usize..][..src_row_bytes as usize];
        let row = if mirror_y { h as usize - 1 - v } else { v };
        let dst_row = &mut rows[row * row_bytes..][..row_bytes];

        let mut put = |u: usize, index: u8| {
            // Index 0 is the one and only transparency.
            if index == 0 {
                return;
            }
            let column = if mirror_x { w as usize - 1 - u } else { u };
            let entry = index as usize * 2;
            dst_row[column * 2] = palette[entry];
            dst_row[column * 2 + 1] = palette[entry + 1];
        };

        if four_bpp {
            // Mirrors the guest byte for byte: an odd `srcX` starts on the LOW nibble of
            // the first byte, then whole bytes high-nibble-first, then a lone high nibble
            // for an odd remainder — and that last byte is read WITHOUT advancing, which
            // is what makes the guest's `w <= 0` case run off the end. `w > 0` is gated
            // above so that case never reaches here.
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
        } else {
            for (u, &index) in src_row.iter().enumerate() {
                put(u, index);
            }
        }
    }

    for v in 0..h {
        let at = dst_origin.wrapping_add(2 * v * dst_stride);
        core.write_bytes(at, &rows[v as usize * row_bytes..(v as usize + 1) * row_bytes])?;
    }

    Ok(true)
}

/// The bytes the hook replaces, so a build whose blitter moved fails the install instead
/// of corrupting the guest: `push {r4, r5, r6, r7, lr}`.
pub const EXPECTED_PROLOGUE: [u8; 2] = [0xf0, 0xb5];

#[cfg(test)]
mod tests {
    use super::*;

    /// The mirrored transform must be an exact column reversal of the identity one — the
    /// property the whole `TRANSFORM_MIRROR_X` path rests on.
    #[test]
    fn mirror_reverses_columns() {
        let w = 5usize;
        let src = [1u8, 0, 2, 3, 0];
        let mut plain = [0u16; 5];
        let mut mirrored = [0u16; 5];
        for (u, &index) in src.iter().enumerate() {
            if index == 0 {
                continue;
            }
            plain[u] = index as u16;
            mirrored[w - 1 - u] = index as u16;
        }

        assert_eq!(plain, [1, 0, 2, 3, 0]);
        assert_eq!(mirrored, [0, 3, 2, 0, 1]);
        // Transparent columns are untouched in BOTH, not zeroed — that is why the
        // destination row is read back before the loop rather than built from scratch.
        for (u, &index) in src.iter().enumerate() {
            if index == 0 {
                assert_eq!(plain[u], 0);
                assert_eq!(mirrored[w - 1 - u], 0);
            }
        }
    }

    /// A palette window of 256 entries covers every index an 8bpp source can hold, so the
    /// guest's unchecked `pal[idx]` can never read past what we copied.
    #[test]
    fn palette_window_covers_every_8bpp_index() {
        assert!(PALETTE_ENTRIES >= u8::MAX as usize + 1);
    }
}
