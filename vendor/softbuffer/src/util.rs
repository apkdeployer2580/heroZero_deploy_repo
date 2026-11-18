// Not needed on all platforms
#![allow(dead_code)]

use std::cmp;
use std::fmt;
use std::mem::MaybeUninit;
use std::num::NonZeroU32;
use std::ops;

use crate::Rect;
use crate::SoftBufferError;

/// Takes a mutable reference to a container and a function deriving a
/// reference into it, and stores both, making it possible to get back the
/// reference to the container once the other reference is no longer needed.
///
/// This should be consistent with stacked borrow rules, and miri seems to
/// accept it at least in simple cases.
pub struct BorrowStack<'a, T: 'a + ?Sized, U: 'a + ?Sized> {
    container: *mut T,
    member: *mut U,
    _phantom: std::marker::PhantomData<&'a mut T>,
}

unsafe impl<'a, T: 'a + Send + ?Sized, U: 'a + Send + ?Sized> Send for BorrowStack<'a, T, U> {}

impl<'a, T: 'a + ?Sized, U: 'a + ?Sized> BorrowStack<'a, T, U> {
    pub fn new<F>(container: &'a mut T, f: F) -> Result<Self, SoftBufferError>
    where
        F: for<'b> FnOnce(&'b mut T) -> Result<&'b mut U, SoftBufferError>,
    {
        let container = container as *mut T;
        let member = f(unsafe { &mut *container })? as *mut U;
        Ok(Self {
            container,
            member,
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn member(&self) -> &U {
        unsafe { &*self.member }
    }

    pub fn member_mut(&mut self) -> &mut U {
        unsafe { &mut *self.member }
    }

    pub fn into_container(self) -> &'a mut T {
        // SAFETY: Since we consume self and no longer reference member, this
        // mutable reference is unique.
        unsafe { &mut *self.container }
    }
}

impl<'a, T: 'a + ?Sized, U: 'a + ?Sized + fmt::Debug> fmt::Debug for BorrowStack<'a, T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowStack")
            .field("member", &self.member())
            .finish_non_exhaustive()
    }
}

/// Calculates the smallest `Rect` necessary to represent all damaged `Rect`s.
pub(crate) fn union_damage(damage: &[Rect]) -> Option<Rect> {
    struct Region {
        left: u32,
        top: u32,
        bottom: u32,
        right: u32,
    }

    let region = damage
        .iter()
        .map(|rect| Region {
            left: rect.x,
            top: rect.y,
            right: rect.x + rect.width.get(),
            bottom: rect.y + rect.height.get(),
        })
        .reduce(|mut prev, next| {
            prev.left = cmp::min(prev.left, next.left);
            prev.top = cmp::min(prev.top, next.top);
            prev.right = cmp::max(prev.right, next.right);
            prev.bottom = cmp::max(prev.bottom, next.bottom);
            prev
        })?;

    Some(Rect {
        x: region.left,
        y: region.top,
        width: NonZeroU32::new(region.right - region.left)
            .expect("`right` must always be bigger then `left`"),
        height: NonZeroU32::new(region.bottom - region.top)
            .expect("`bottom` must always be bigger then `top`"),
    })
}

/// A wrapper around a `Vec` of pixels that doesn't print the whole buffer on `Debug`.
#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct PixelBuffer(pub Vec<u32>);

impl fmt::Debug for PixelBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PixelBuffer").finish_non_exhaustive()
    }
}

impl ops::Deref for PixelBuffer {
    type Target = Vec<u32>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for PixelBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// ★ wie fork (see the header of this crate's `Cargo.toml`): the pixel swizzle the Android
/// backend's `present()` needs, split out of `backends/android.rs` so that it is compiled —
/// and TESTED — on every host. `backends::android` is `cfg(target_os = "android")`, so a
/// `#[cfg(test)]` module living in there would never run for anybody actually developing
/// this app on Windows/Linux, and this is exactly the kind of bit-twiddling that has to be
/// pinned down by a test.
///
/// Swaps the R and B channels of a `0xAARRGGBB` word, giving `0xAABBGGRR`.
///
/// The derivation, because the whole point is to be BIT-IDENTICAL to what it replaces.
/// Upstream wrote, per pixel:
///
/// ```text
/// let [b, g, r, a] = pixel.to_le_bytes();   // 0xAARRGGBB -> [BB, GG, RR, AA]
/// out[0].write(r); out[1].write(g); out[2].write(b); out[3].write(a);
/// ```
///
/// so the four emitted bytes, in ascending address order, are `[RR, GG, BB, AA]`. The u32
/// whose little-endian encoding is `[RR, GG, BB, AA]` is `0xAABBGGRR`. And for
/// `p = 0xAARRGGBB`:
///
/// ```text
///   p & 0xff00_ff00        == 0xAA00_GG00     (alpha and green stay put)
///  (p & 0x00ff_0000) >> 16 == 0x0000_00RR     (red   moves down into the low byte)
///  (p & 0x0000_00ff) << 16 == 0x00BB_0000     (blue  moves up  into bits 16..24)
///   OR of the three        == 0xAABB_GGRR
/// ```
///
/// The three masks are disjoint and together cover all 32 bits, so the OR loses nothing
/// and invents nothing. `swizzle_rb` is its own inverse.
#[inline]
pub(crate) fn swizzle_rb(p: u32) -> u32 {
    (p & 0xff00_ff00) | ((p & 0x00ff_0000) >> 16) | ((p & 0x0000_00ff) << 16)
}

/// ★ wie fork: write one scanline of `0xAARRGGBB` pixels out as R,G,B,A bytes.
///
/// One 4-byte `copy_from_slice` per pixel — which lowers to a single 32-bit store, the
/// length being a compile-time constant 4 — instead of upstream's four separate
/// `MaybeUninit::write` calls. At 1080x2340 that is 2.5 M stores per presented frame
/// instead of 10 M.
///
/// No `unsafe` is needed for it: `MaybeUninit<u8>` is `Copy`, so the destination can be
/// filled by a plain slice copy from an array built with `MaybeUninit::new`, and nothing
/// here ever READS the possibly-uninitialised destination. `chunks_exact_mut(4)` both
/// satisfies the `copy_from_slice` equal-length precondition (so it cannot panic) and
/// hands out non-overlapping windows. `zip` stops at the shorter side, so a caller that
/// got the geometry wrong under-writes instead of panicking — callers assert the 1:4
/// ratio themselves, exactly as upstream did.
pub(crate) fn write_rgba_line(input: &[u32], output: &mut [MaybeUninit<u8>]) {
    // ★ wie fork 2026-09-19: the per-pixel `copy_from_slice` above was 36 % of the UI
    // thread on S9 — it lowers to a 32-bit store, but not to a NEON store of four. When
    // the destination is 4-byte aligned (`ANativeWindow_Buffer::bits` always is), view
    // it as `MaybeUninit<u32>` and let the plain zip loop vectorise. The bytes written
    // are identical: `MaybeUninit::new(swizzle(p))` on little-endian IS
    // `swizzle(p).to_le_bytes()` in order. An unaligned start (never seen on Android)
    // and the sub-pixel tail keep the byte path.
    //
    // SAFETY: `MaybeUninit<u32>` has no validity or initialisation requirement, and the
    // slice returned by `align_to_mut` covers exactly the aligned middle of `output`;
    // nothing reads either view.
    let (head, mid, tail) = unsafe { output.align_to_mut::<MaybeUninit<u32>>() };
    if head.is_empty() {
        let mid_px = mid.len();
        let mut input = input.iter();
        for (pixel, out) in input.by_ref().take(mid_px).zip(mid.iter_mut()) {
            *out = MaybeUninit::new(swizzle_rb(*pixel).to_le());
        }
        for (pixel, out) in input.zip(tail.chunks_exact_mut(4)) {
            out.copy_from_slice(&swizzle_rb(*pixel).to_le_bytes().map(MaybeUninit::new));
        }
        return;
    }
    // Unaligned start: pixel 0 would straddle `head` and `mid`, so take the byte path.
    for (pixel, out) in input.iter().zip(output.chunks_exact_mut(4)) {
        out.copy_from_slice(&swizzle_rb(*pixel).to_le_bytes().map(MaybeUninit::new));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_borrowstack_slice_int() {
        fn f(mut stack: BorrowStack<[u32], u32>) {
            assert_eq!(*stack.member(), 3);
            *stack.member_mut() = 42;
            assert_eq!(stack.into_container(), &[1, 2, 42, 4, 5]);
        }

        let mut v = vec![1, 2, 3, 4, 5];
        f(BorrowStack::new(v.as_mut(), |v: &mut [u32]| Ok(&mut v[2])).unwrap());
        assert_eq!(&v, &[1, 2, 42, 4, 5]);
    }

    /// ★ wie fork. Literal transcription of the loop body this fork replaced, kept here as
    /// the oracle so the two can be compared instead of merely reasoned about.
    fn upstream_reference(pixel: u32) -> [u8; 4] {
        let [b, g, r, a] = pixel.to_le_bytes();
        [r, g, b, a]
    }

    /// ★ wie fork: the swizzle must be bit-identical to upstream's four byte writes.
    /// Exhaustive over every byte value in every channel position, plus the corners.
    #[test]
    fn swizzle_rb_matches_upstream_byte_order() {
        for edge in [0x0000_0000u32, 0xffff_ffff, 0x1234_5678, 0x8000_0001, 0x00ff_00ff, 0xff00_ff00] {
            assert_eq!(swizzle_rb(edge).to_le_bytes(), upstream_reference(edge));
        }
        // One channel swept over its full range at a time, against a non-trivial
        // background, so a mask that leaked into a neighbouring byte cannot hide.
        for v in 0..=0xffu32 {
            for shift in [0, 8, 16, 24] {
                let p = (0x5a5a_5a5au32 & !(0xffu32 << shift)) | (v << shift);
                assert_eq!(swizzle_rb(p).to_le_bytes(), upstream_reference(p), "p = {p:#010x}");
            }
        }
        // Self-inverse, which is what makes "only R and B moved" true.
        for v in 0..=0xffffu32 {
            let p = v.wrapping_mul(0x9e37_79b9);
            assert_eq!(swizzle_rb(swizzle_rb(p)), p);
        }
    }

    /// ★ wie fork: a whole scanline through `write_rgba_line` equals the upstream loop,
    /// byte for byte, and every byte of the destination really is written.
    #[test]
    fn write_rgba_line_matches_upstream_loop() {
        let input: Vec<u32> = (0..64u32).map(|i| i.wrapping_mul(0x0102_0304) ^ 0xdead_beef).collect();

        let mut output = vec![MaybeUninit::new(0xccu8); input.len() * 4];
        write_rgba_line(&input, &mut output);

        let mut expected = Vec::with_capacity(input.len() * 4);
        for p in &input {
            expected.extend_from_slice(&upstream_reference(*p));
        }

        // SAFETY: `write_rgba_line` wrote all `input.len() * 4` bytes (one 4-byte chunk per
        // input pixel, and `output` is exactly that long), and they were initialised to
        // begin with anyway — this vec was built from `MaybeUninit::new`, never `uninit`.
        let got: Vec<u8> = output.iter().map(|b| unsafe { b.assume_init() }).collect();
        assert_eq!(got, expected);
    }

    /// ★ wie fork: a short destination must under-write rather than panic (`zip`), and it
    /// must not scribble past the chunks it did get.
    #[test]
    fn write_rgba_line_stops_at_the_shorter_side() {
        let input = [0x1122_3344u32; 4];
        let mut output = vec![MaybeUninit::new(0u8); 4 * 4];

        write_rgba_line(&input[..1], &mut output);
        // SAFETY: as above — every element was initialised by `MaybeUninit::new(0)`.
        let got: Vec<u8> = output.iter().map(|b| unsafe { b.assume_init() }).collect();
        assert_eq!(&got[..4], &[0x22, 0x33, 0x44, 0x11]);
        assert!(got[4..].iter().all(|&b| b == 0));
    }

    /// ★ wie fork 2026-09-19: the aligned (`u32`) path and the byte path must write the
    /// same bytes — exercised by offsetting the destination by 0..4 bytes, which
    /// selects one or the other, and checking against the upstream oracle each time.
    #[test]
    fn write_rgba_line_aligned_and_unaligned_agree() {
        let input: Vec<u32> = (0..37u32).map(|i| i.wrapping_mul(0x9e37_79b9)).collect();
        let mut expected = Vec::with_capacity(input.len() * 4);
        for p in &input {
            expected.extend_from_slice(&upstream_reference(*p));
        }
        let mut backing = vec![MaybeUninit::new(0xa5u8); input.len() * 4 + 8];
        for off in 0..4 {
            backing.fill(MaybeUninit::new(0xa5));
            let dst = &mut backing[off..off + input.len() * 4];
            write_rgba_line(&input, dst);
            // SAFETY: every element was initialised by `MaybeUninit::new`.
            let got: Vec<u8> = backing.iter().map(|b| unsafe { b.assume_init() }).collect();
            assert!(got[..off].iter().all(|&b| b == 0xa5), "off {off}: wrote before the slice");
            assert_eq!(&got[off..off + expected.len()], &expected[..], "off {off}");
            assert!(got[off + expected.len()..].iter().all(|&b| b == 0xa5), "off {off}: wrote past the slice");
        }
    }
}
