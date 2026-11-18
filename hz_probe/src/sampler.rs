//! `--sample <file>`: a sampling profiler for the game's thread (Windows x86-64 only).
//!
//! A host thread suspends the probe's loop thread about a thousand times a second, reads its
//! registers, unwinds its stack with the system's own unwind tables (`RtlVirtualUnwind` — every
//! x64 function has them, the system DLLs' too; LLVM's Win64 frame pointer does not point at the
//! saved one, so an `rbp` chain cannot be walked), and resumes it. The stacks go to `<file>` as
//! image-relative hex addresses, one sample a line, leaf first — `tools/hz_profile.py` names them
//! with `llvm-symbolizer` and sums the functions. Build the probe with line tables for it (the
//! script says how).
//!
//! No crate: a handful of kernel32 calls. `CONTEXT` is the AMD64 layout, read by offset.

use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Handle = *mut core::ffi::c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> Handle;
    fn GetCurrentProcess() -> Handle;
    fn DuplicateHandle(src_process: Handle, src: Handle, dst_process: Handle, dst: *mut Handle, access: u32, inherit: i32, options: u32) -> i32;
    fn SuspendThread(thread: Handle) -> u32;
    fn ResumeThread(thread: Handle) -> u32;
    fn GetThreadContext(thread: Handle, context: *mut Context) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn RtlLookupFunctionEntry(pc: u64, image_base: *mut u64, history: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn RtlVirtualUnwind(
        handler_type: u32,
        image_base: u64,
        pc: u64,
        entry: *mut core::ffi::c_void,
        context: *mut Context,
        handler_data: *mut *mut core::ffi::c_void,
        establisher: *mut u64,
        pointers: *mut core::ffi::c_void,
    ) -> *mut core::ffi::c_void;
}

/// AMD64 `CONTEXT`: 1232 bytes, 16-aligned. `ContextFlags` at 0x30, `Rsp` 0x98, `Rip` 0xF8.
#[repr(C, align(16))]
struct Context([u8; 1232]);

impl Context {
    fn u64_at(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.0[off..off + 8].try_into().unwrap())
    }

    fn set_u64(&mut self, off: usize, v: u64) {
        self.0[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

/// `CONTEXT_FULL` for AMD64 (control, integer, floating point: the unwinder restores all three).
const CONTEXT_FULL: u32 = 0x0010_000B;
const DUPLICATE_SAME_ACCESS: u32 = 2;
/// Frames walked at most, and how far above `rsp` a frame may be (the thread's stack).
const MAX_FRAMES: usize = 96;
const STACK_SPAN: u64 = 256 * 1024 * 1024;

pub struct Sampler {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

struct SendHandle(Handle);
// SAFETY: a thread handle is a kernel object reference, usable from any thread.
unsafe impl Send for SendHandle {}

impl Sampler {
    /// Start sampling the CALLING thread into `path`.
    pub fn start(path: std::path::PathBuf) -> std::io::Result<Self> {
        let mut me: Handle = std::ptr::null_mut();
        // SAFETY: plain Win32 calls on this thread's own pseudo-handle.
        let ok = unsafe { DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut me, 0, 0, DUPLICATE_SAME_ACCESS) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a null name is the process's own image.
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        writeln!(out, "# base {base:#x}")?;
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let target = SendHandle(me);
        let worker = std::thread::Builder::new().name("hz-sampler".into()).spawn(move || {
            let target = target;
            let mut next = Instant::now();
            let mut line = String::new();
            // Filled while the target is suspended — no allocation then: it may be holding the
            // heap's lock.
            let mut frames = [0u64; MAX_FRAMES + 1];
            while !quit.load(Ordering::Relaxed) {
                next += Duration::from_micros(1000);
                while Instant::now() < next {
                    std::hint::spin_loop();
                }
                let mut n = 0;
                // SAFETY: the target is suspended while its registers and stack are read; every
                // stack read is bounded to [rsp, rsp + STACK_SPAN) and 8-aligned, and the walk
                // only goes up.
                unsafe {
                    if SuspendThread(target.0) == u32::MAX {
                        break;
                    }
                    let mut ctx = Context([0; 1232]);
                    ctx.0[0x30..0x34].copy_from_slice(&CONTEXT_FULL.to_le_bytes());
                    if GetThreadContext(target.0, &mut ctx) != 0 {
                        let top = ctx.u64_at(0x98);
                        while n <= MAX_FRAMES {
                            let (rip, rsp) = (ctx.u64_at(0xF8), ctx.u64_at(0x98));
                            if rip == 0 || rsp < top || rsp >= top.saturating_add(STACK_SPAN) {
                                break;
                            }
                            // A return address points past its call: one byte back is inside it.
                            frames[n] = if n == 0 { rip } else { rip - 1 };
                            n += 1;
                            let mut image = 0u64;
                            let entry = RtlLookupFunctionEntry(rip, &mut image, std::ptr::null_mut());
                            if entry.is_null() {
                                // A leaf with no unwind data: its return address is on top.
                                ctx.set_u64(0xF8, *(rsp as *const u64));
                                ctx.set_u64(0x98, rsp + 8);
                            } else {
                                let (mut data, mut frame) = (std::ptr::null_mut(), 0u64);
                                RtlVirtualUnwind(0, image, rip, entry, &mut ctx, &mut data, &mut frame, std::ptr::null_mut());
                            }
                            if ctx.u64_at(0x98) <= rsp {
                                break;
                            }
                        }
                    }
                    ResumeThread(target.0);
                }
                line.clear();
                for &a in &frames[..n] {
                    push(&mut line, a, base);
                }
                if !line.is_empty() {
                    let _ = writeln!(out, "{line}");
                }
            }
            let _ = out.flush();
        })?;
        Ok(Self { stop, worker: Some(worker) })
    }
}

fn push(line: &mut String, addr: u64, base: u64) {
    use std::fmt::Write as _;
    if !line.is_empty() {
        line.push(' ');
    }
    let _ = write!(line, "{:x}", addr.wrapping_sub(base));
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
