//! Implementation of software buffering for Android.
//!
//! ★ wie fork — see the header of this crate's `Cargo.toml` for why this crate is vendored.
//! Every deviation from softbuffer 0.4.8 in this file is marked `★ wie fork`. Two changes,
//! both on the per-frame present path of a 1080x2340 window:
//!
//!   1. The staging buffer is owned by the SURFACE and reused, instead of a fresh zeroed
//!      `vec![0; w * h]` (~10.1 MB of malloc + memset) per `buffer_mut()` call.
//!   2. `present()` stores four bytes at a time instead of one, via `util::write_rgba_line`.

use std::fmt;
use std::marker::PhantomData;
use std::num::{NonZeroI32, NonZeroU32};

use ndk::{
    hardware_buffer_format::HardwareBufferFormat,
    native_window::{NativeWindow, NativeWindowBufferLockGuard},
};
#[cfg(doc)]
use raw_window_handle::AndroidNdkWindowHandle;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};

use crate::error::InitError;
use crate::{util, BufferInterface, Rect, SoftBufferError, SurfaceInterface};

/// The handle to a window for software buffering.
#[derive(Debug)]
pub struct AndroidImpl<D, W> {
    native_window: NativeWindow,
    /// ★ wie fork: the staging buffer, owned by the SURFACE so it outlives one frame.
    ///
    /// Upstream allocated this inside every `buffer_mut()` call (`vec![0; w * h]`) and
    /// dropped it at `present()`. For a 1080x2340 window that is a 10.1 MB allocation that
    /// is memset to zero and then, without exception, fully overwritten by the caller
    /// before it is presented — the allocator traffic and the zeroing were both pure loss,
    /// once per frame, forever. Keeping the `Vec` here lets `buffer_mut()` reuse the same
    /// capacity every frame.
    ///
    /// `util::PixelBuffer` rather than a bare `Vec<u32>` only so the `derive(Debug)` above
    /// keeps printing a placeholder instead of two and a half million pixels.
    buffer: util::PixelBuffer,
    window: W,
    _display: PhantomData<D>,
}

impl<D: HasDisplayHandle, W: HasWindowHandle> SurfaceInterface<D, W> for AndroidImpl<D, W> {
    type Context = D;
    type Buffer<'a>
        = BufferImpl<'a, D, W>
    where
        Self: 'a;

    /// Create a new [`AndroidImpl`] from an [`AndroidNdkWindowHandle`].
    fn new(window: W, _display: &Self::Context) -> Result<Self, InitError<W>> {
        let raw = window.window_handle()?.as_raw();
        let RawWindowHandle::AndroidNdk(a) = raw else {
            return Err(InitError::Unsupported(window));
        };

        // Acquire a new owned reference to the window, that will be freed on drop.
        // SAFETY: We have confirmed that the window handle is valid.
        let native_window = unsafe { NativeWindow::clone_from_ptr(a.a_native_window.cast()) };

        Ok(Self {
            native_window,
            // ★ wie fork: grown to the window size on the first `buffer_mut()` and kept.
            buffer: util::PixelBuffer(Vec::new()),
            _display: PhantomData,
            window,
        })
    }

    #[inline]
    fn window(&self) -> &W {
        &self.window
    }

    /// Also changes the pixel format to [`HardwareBufferFormat::R8G8B8A8_UNORM`].
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> Result<(), SoftBufferError> {
        let (width, height) = (|| {
            let width = NonZeroI32::try_from(width).ok()?;
            let height = NonZeroI32::try_from(height).ok()?;
            Some((width, height))
        })()
        .ok_or(SoftBufferError::SizeOutOfRange { width, height })?;

        self.native_window
            .set_buffers_geometry(
                width.into(),
                height.into(),
                // Default is typically R5G6B5 16bpp, switch to 32bpp
                Some(HardwareBufferFormat::R8G8B8X8_UNORM),
            )
            .map_err(|err| {
                SoftBufferError::PlatformError(
                    Some("Failed to set buffer geometry on ANativeWindow".to_owned()),
                    Some(Box::new(err)),
                )
            })
    }

    fn buffer_mut(&mut self) -> Result<BufferImpl<'_, D, W>, SoftBufferError> {
        // ★ wie fork: split borrow. The returned `BufferImpl` has to hold the lock guard
        // (which borrows `self.native_window` for as long as the guard lives) AND a mutable
        // slice of `self.buffer` at the same time, both for the lifetime of the `&mut self`
        // this method was called with. Destructuring splits that one `&mut self` into two
        // provably disjoint field borrows up front, so neither can be blamed for the other.
        let Self {
            native_window,
            buffer,
            ..
        } = self;

        let native_window_buffer = native_window.lock(None).map_err(|err| {
            SoftBufferError::PlatformError(
                Some("Failed to lock ANativeWindow".to_owned()),
                Some(Box::new(err)),
            )
        })?;

        if !matches!(
            native_window_buffer.format(),
            // These are the only formats we support
            HardwareBufferFormat::R8G8B8A8_UNORM | HardwareBufferFormat::R8G8B8X8_UNORM
        ) {
            return Err(SoftBufferError::PlatformError(
                Some(format!(
                    "Unexpected buffer format {:?}, please call \
                    .resize() first to change it to RGBx8888",
                    native_window_buffer.format()
                )),
                None,
            ));
        }

        // ★ wie fork: REUSE the surface's buffer instead of `vec![0; w * h]`.
        //
        // `resize` only touches the length: when the geometry is unchanged (the steady
        // state — every frame after the first, and after every rotation) it is a no-op, so
        // there is no allocation and, crucially, no memset. When the window grows it
        // reallocates once and zeroes only the new tail; when it shrinks it just truncates
        // and the capacity stays for the next rotation back.
        //
        // The stale contents are DELIBERATELY not cleared — erasing them is precisely the
        // 10 MB memset this change exists to delete. That is sound because this backend
        // reports `age() == 0`, softbuffer's contract for "the contents are undefined, the
        // caller must draw every pixel before presenting", which is the same contract
        // upstream's zero-filled `vec!` satisfied (a caller that trusted the zeros would
        // already be broken on every other backend). It is also true in practice for the
        // one caller that matters here: `wie_android`'s `redraw()` partitions the window
        // exactly — either a full-window `fill`, or a background `fill` of everything
        // outside the game rect plus a blit that writes every pixel inside it — before it
        // calls `present()`.
        //
        // NB the size comes from the LOCK GUARD, not from the last `resize()` call, so a
        // rotation that changes the window under us still gets a correctly-sized slice.
        let len = native_window_buffer.width() * native_window_buffer.height();
        buffer.resize(len, 0);

        Ok(BufferImpl {
            native_window_buffer,
            buffer: buffer.as_mut_slice(),
            marker: PhantomData,
        })
    }

    /// Fetch the buffer from the window.
    fn fetch(&mut self) -> Result<Vec<u32>, SoftBufferError> {
        Err(SoftBufferError::Unimplemented)
    }
}

pub struct BufferImpl<'a, D: ?Sized, W> {
    native_window_buffer: NativeWindowBufferLockGuard<'a>,
    /// ★ wie fork: borrowed from `AndroidImpl::buffer`, no longer owned. `'a` is the
    /// `&mut AndroidImpl` that `buffer_mut()` was called with, the same lifetime the lock
    /// guard above uses, so the surface cannot be touched again while this is alive.
    buffer: &'a mut [u32],
    marker: PhantomData<(&'a D, &'a W)>,
}

/// ★ wie fork: hand-written because the field above is now a slice — `derive(Debug)` would
/// dump every pixel, which is exactly what `util::PixelBuffer` existed to prevent.
impl<D: ?Sized, W> fmt::Debug for BufferImpl<'_, D, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferImpl")
            .field("native_window_buffer", &self.native_window_buffer)
            .finish_non_exhaustive()
    }
}

// TODO: Move to NativeWindowBufferLockGuard?
unsafe impl<'a, D, W> Send for BufferImpl<'a, D, W> {}

impl<'a, D: HasDisplayHandle, W: HasWindowHandle> BufferInterface for BufferImpl<'a, D, W> {
    fn width(&self) -> NonZeroU32 {
        NonZeroU32::new(self.native_window_buffer.width() as u32).unwrap()
    }

    fn height(&self) -> NonZeroU32 {
        NonZeroU32::new(self.native_window_buffer.height() as u32).unwrap()
    }

    // ★ wie fork: reborrows of the surface-owned slice (upstream deref'd an owned `Vec`).
    #[inline]
    fn pixels(&self) -> &[u32] {
        &*self.buffer
    }

    #[inline]
    fn pixels_mut(&mut self) -> &mut [u32] {
        &mut *self.buffer
    }

    #[inline]
    fn age(&self) -> u8 {
        0
    }

    // ★ wie fork: upstream's `// TODO: This function is pretty slow this way` is done.
    // The body used to swizzle BGRA->RGBA with four separate `MaybeUninit::write` calls per
    // pixel — 10 M single-byte stores per frame at 1080x2340. `util::write_rgba_line` does
    // the same job with one 32-bit store per pixel; see its doc comment for the derivation
    // showing the emitted bytes are bit-identical, and `util`'s tests, which pin that
    // against a transcription of the loop that used to stand here.
    //
    // `width` is read out first: `chunks` borrows `self.buffer` and `lines()` needs
    // `&mut self.native_window_buffer`, and while those are disjoint fields, sinking the
    // call into the `chunks(..)` argument as upstream did is needlessly fragile.
    fn present(mut self) -> Result<(), SoftBufferError> {
        let width = self.native_window_buffer.width();
        let input_lines = self.buffer.chunks(width);
        for (output, input) in self
            .native_window_buffer
            .lines()
            // Unreachable as we checked before that this is a valid, mappable format
            .unwrap()
            .zip(input_lines)
        {
            // .lines() removed the stride
            assert_eq!(output.len(), input.len() * 4);

            // Swizzle colors from BGR(A) to RGB(A)
            util::write_rgba_line(input, output);
        }
        Ok(())
    }

    fn present_with_damage(self, _damage: &[Rect]) -> Result<(), SoftBufferError> {
        // TODO: Android requires the damage rect _at lock time_
        // Since we're faking the backing buffer _anyway_, we could even fake the surface lock
        // and lock it here (if it doesn't influence timings).
        self.present()
    }
}
