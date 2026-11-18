use alloc::boxed::Box;
use core::{
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};
use wie_util::Result;

use crate::{ArmCore, ThreadId};

// ── ★ POLL-NESTING PROBE (added by the wie_app fork, 2026-08-04) ────────────────
//
// Diagnosing a device crash: three tombstones, all `signal 11 / stack overflow` on
// the `wie-emu` thread, all after 21-24 minutes of play, all showing the same two
// functions alternating for ~930 frames. Nothing about the game state correlated;
// only elapsed time did.
//
// The chain is on the HEAP, not the stack: nested boxed futures. Between emulator
// ticks the native stack is unwound completely, so sampling SP from the tick loop
// shows nothing. The depth only exists *during* a poll, when the executor polls the
// outermost future and that walks every link. So the thing to measure is the poll
// nesting itself, and this is the cheapest place that sees it: every guest thread
// poll passes through here.
//
// Single global atomics rather than thread-locals: `#[thread_local]` is not stable
// in no_std, and the emulator is one thread by construction (`wie-emu`). Cost is two
// relaxed atomic ops per poll. Logs only on a NEW maximum, so a healthy session
// prints a handful of lines at startup and then goes quiet — a session that is
// leaking prints a rising staircase, which is the signal we are looking for.
//
// ★ 2026-08-06, measured: this gauge NEVER FIRES. There is one wrapper per executor
// task, so its nesting is 1 by construction and 30 000 headless frames produced zero
// warnings. It is kept because its `STACK_BASE` stamp is the baseline the real gauge
// (`core.rs`, around `run_function`) measures against — the outermost native frame of
// one poll of this task's future.
static POLL_DEPTH: AtomicUsize = AtomicUsize::new(0);
static POLL_DEPTH_MAX: AtomicUsize = AtomicUsize::new(0);
/// Address of a local in the outermost `poll` of this task's future, i.e. the top of
/// the native stack region one poll of the whole future chain is allowed to use.
pub(crate) static STACK_BASE: AtomicUsize = AtomicUsize::new(0);

/// Depth at which a nesting level is no longer plausibly legitimate re-entry
/// (guest -> WIPI -> guest is a handful deep, not tens).
const DEPTH_SUSPICIOUS: usize = 16;

struct DepthGuard;

impl Drop for DepthGuard {
    fn drop(&mut self) {
        POLL_DEPTH.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct ArmCoreThreadWrapper {
    core: ArmCore,
    thread_id: ThreadId,
    future: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl ArmCoreThreadWrapper {
    pub fn new<F, Fut>(core: ArmCore, thread_id: ThreadId, entry: F) -> Result<Self>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Ok(Self {
            core,
            thread_id,
            future: Box::pin(entry()),
        })
    }
}

impl Future for ArmCoreThreadWrapper {
    type Output = Result<()>;

    // `level = "debug"` (was the instrument default, INFO): device-measured at ~92
    // records/second, i.e. ~92 `__android_log_write` calls per second from inside the
    // frame budget. This span is on the hottest function in the emulator.
    #[tracing::instrument(name = "native thread", level = "debug", fields(id = self.thread_id), skip_all)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let depth = POLL_DEPTH.fetch_add(1, Ordering::Relaxed) + 1;
        let _depth_guard = DepthGuard;
        if depth == 1 {
            STACK_BASE.store(&depth as *const usize as usize, Ordering::Relaxed);
        }
        if depth > POLL_DEPTH_MAX.load(Ordering::Relaxed) {
            POLL_DEPTH_MAX.store(depth, Ordering::Relaxed);
            if depth >= DEPTH_SUSPICIOUS {
                // Warn, not info: this is the staircase that ends in a stack overflow,
                // and the host's log filter keeps warnings.
                tracing::warn!("poll nesting depth {depth} (thread id={})", self.thread_id);
            }
        }

        let _guard = self.core.enter_thread_context(self.thread_id);

        self.future.as_mut().poll(cx)
    }
}

impl Unpin for ArmCoreThreadWrapper {}

impl Drop for ArmCoreThreadWrapper {
    fn drop(&mut self) {
        self.core.delete_thread_context(self.thread_id);
    }
}
