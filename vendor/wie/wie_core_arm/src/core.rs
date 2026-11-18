use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};
use core::{
    mem::size_of,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use spin::Mutex;

use wie_backend::{ProfileCallback, ProfileSample};
use wie_util::{ByteRead, ByteWrite, Result, WieError, read_generic};

use crate::{
    EmulatedFunction, ResultWriter, ThreadId,
    context::ArmCoreContext,
    engine::{Arm32CpuEngine, ArmEngine, ArmRegister, EngineRunResult, MemoryPermission},
    function::{RegisteredFunction, RegisteredFunctionHolder},
    thread::ThreadState,
    thread_wrapper::ArmCoreThreadWrapper,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::{
    engine::{DebugInner, DebuggedArm32CpuEngine},
    gdb::GdbTarget,
};

const GLOBAL_DATA_BASE: u32 = 0x7fff0000;
const FUNCTIONS_BASE: u32 = 0x71000000;
const FUNCTIONS_SIZE: usize = 0x10000;
const SVC_STUB_SIZE: u32 = 16;
pub const RUN_FUNCTION_LR: u32 = 0x7f000000;
pub const HEAP_BASE: u32 = 0x40000000;
pub const HEAP_SIZE: u32 = 0x10000000;

/// Limit on stack frames recorded per sample. Bounds memory and protects
/// against runaway loops if the R7 chain forms a cycle.
const PROFILE_MAX_STACK: usize = 32;
/// Flush the per-stack counter map every this many samples taken.
const PROFILE_FLUSH_INTERVAL: u32 = 1000;

struct ProfileState {
    samples: BTreeMap<Vec<u32>, u64>,
    counter: u32,
    callback: ProfileCallback,
}

// ── ★ GUEST-CALL NESTING PROBE (wie_app fork, 2026-08-06) ───────────────────
//
// The 2026-08-04 probe in `thread_wrapper.rs` measured the wrong layer: there is
// exactly ONE `ArmCoreThreadWrapper` per executor task (`System::spawn` ->
// `KtfTaskRunner::run` -> `run_in_thread`), so its nesting never exceeds 1 and a
// 30 000-frame headless run produced zero warnings. The chain that overflows the
// native stack therefore lives BELOW it, and every level of guest re-entry goes
// through `run_function`, so that is where the depth actually is.
//
// Two gauges, because they fail differently: a level COUNT says how many nested
// guest calls are outstanding, and a native stack WATERMARK says how much of the
// `wie-emu` stack one poll of that chain consumes. The crash is the second one
// hitting the guard page, but the first is what explains it.
//
// ★ MEASURED (headless, 40 000 frames, 2026-08-06): the count RULES ITSELF OUT — it
// climbs to 21 during warm-up and then stops, and the traced cycles at 19/20/21 are
// three DIFFERENT call trees of similar depth rather than one repeating one. So this
// is not the growth either. Both gauges are kept because they are cheap and because
// the headless run idles where a real session plays: if the count ever exceeds 21 on
// a device, that alone names the bug. The gauge that measures the stack itself, with
// no assumption about which function repeats, is `emulator::stack_rss_kb`.
static RUN_DEPTH: AtomicUsize = AtomicUsize::new(0);
static RUN_DEPTH_MAX: AtomicUsize = AtomicUsize::new(0);
static STACK_HIGH: AtomicUsize = AtomicUsize::new(0);
/// Entry address per nesting level, so a new maximum can print the whole cycle
/// instead of just its innermost frame. A depth counter says "it is growing";
/// this says WHAT repeats, which is the part that names the bug.
const RUN_TRACE_MAX: usize = 48;
static RUN_TRACE: [AtomicUsize; RUN_TRACE_MAX] = [const { AtomicUsize::new(0) }; RUN_TRACE_MAX];

/// Registers in an [`ArmCoreContext`], as [`ArmCore::context_words`] yields them.
const CONTEXT_WORDS: usize = 17;
/// Where `sp` sits in that order. Asserted against `context_words` by
/// `sp_word_index_matches_context_words`, because a scan that reads the wrong slot as a
/// stack pointer would silently stop covering the stack.
const SP_WORD_INDEX: usize = 13;

/// ★★ THE REGISTER FILE OF EVERY GUEST FRAME A NESTED `run_function` INTERRUPTED
/// (2026-09-19) — the fourth and last root hole of the collector.
///
/// An AOT-compiled guest (hero2) keeps java locals in ARM registers. When such a frame
/// calls out through an SVC, [`ArmCore::run_function`] parks its whole register file in a
/// Rust local and runs the callee on the same guest stack. If a collection happens down
/// there — hero2 calls `System.gc()` itself, so it does — an object whose ONLY reference
/// was in `r4` of the interrupted frame is invisible to the mark walk, gets freed, and the
/// frame reads a free-chain word when it resumes. Device-confirmed 2026-09-19 (S9,
/// 지옥불): `Invalid memory access; address: 604` inside `n.paint`, with `r4 = r6 = r7 =
/// 0x4843aa20` and `*(0x4843aa20) = 0x22f` — the free-chain fingerprint `liveness` names.
///
/// So the parked file is published here as well, in a slot the frame OWNS until it
/// returns, for [`ArmCore::scan_conservative_roots`] to read.
///
/// ⛔ A static array of atomics rather than a `Vec` behind the core's lock, exactly like
/// `RUN_TRACE` above: this is the hottest host/guest crossing in the tree (~9,000 SVCs a
/// second in the field) and 17 relaxed stores cost nothing, while a lock here would be
/// taken for nothing 9,000 times a second.
///
/// ★★ OWNED SLOTS, NOT `RUN_DEPTH` INDEXES (2026-09-20, review). The first cut indexed
/// this by the global nesting depth and had the scan read `take(depth)`, on the premise
/// that "slots at or beyond the current depth are stale". That premise needs a single
/// stack discipline, and `run_function` is ASYNC: each guest thread is its own executor
/// task, an SVC handler can await (hero2's own `Thread.sleep` does), and the depths are
/// then released in COMPLETION order, not LIFO. Two ways that frees a live object:
/// a task parked at depth 2 while the depth-1 task returns is no longer inside
/// `take(depth)` at all, and the next frame to enter at depth 2 overwrites its register
/// file. A claimed bit per slot removes both — a parked frame's registers stay visible,
/// and unreachable, until that frame itself returns.
static RUN_CTX: [[AtomicU32; CONTEXT_WORDS]; RUN_TRACE_MAX] =
    [const { [const { AtomicU32::new(0) }; CONTEXT_WORDS] }; RUN_TRACE_MAX];

/// Which [`RUN_CTX`] slots hold a register file that is still parked. Bit `i` = slot `i`.
static RUN_CTX_USED: AtomicU64 = AtomicU64::new(0);
const _: () = assert!(RUN_TRACE_MAX <= u64::BITS as usize, "RUN_CTX_USED has one bit per RUN_CTX slot");

/// Give a [`RUN_CTX`] slot back. The frame that claimed it has returned; its registers
/// are live in the engine again.
fn release_run_ctx_slot(i: usize) {
    RUN_CTX_USED.fetch_and(!(1u64 << i), Ordering::Relaxed);
}

/// Take a free [`RUN_CTX`] slot, or `None` when all are taken.
///
/// Exhaustion costs the scan the roots of THIS frame only (it publishes nowhere and the
/// frames below keep theirs), which is the same thing the old depth cap did past
/// `RUN_TRACE_MAX`. It is not silent: `run_function` already warns on every new maximum
/// nesting depth from 8 up, and the field maximum is 24.
fn claim_run_ctx_slot() -> Option<usize> {
    let mut used = RUN_CTX_USED.load(Ordering::Relaxed);
    loop {
        let free = (!used).trailing_zeros() as usize;
        if free >= RUN_TRACE_MAX {
            return None;
        }
        match RUN_CTX_USED.compare_exchange_weak(used, used | (1 << free), Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Some(free),
            Err(now) => used = now,
        }
    }
}

/// The half of the guest heap a KTF class-instance record can live in.
///
/// `Allocator::alloc` routes anything up to `BUCKET_MAX` to the bucket allocator, and an
/// instance record is 8 bytes, so every `ptr_raw` the collector could care about is in
/// here. That makes this the tightest honest filter a conservative scan can put on a stack
/// word — code addresses (`0x1xxxxx`), the static base, thread stacks (the LIST half) and
/// small integers are all rejected without a hash lookup.
pub const BUCKET_HEAP_BASE: u32 = HEAP_BASE + HEAP_SIZE / 2;
/// One past the end of [`BUCKET_HEAP_BASE`]'s range.
pub const BUCKET_HEAP_END: u32 = HEAP_BASE + HEAP_SIZE;

struct RunDepthGuard {
    /// The [`RUN_CTX`] slot this frame claimed, released here and nowhere else.
    ctx_slot: Option<usize>,
}

impl Drop for RunDepthGuard {
    fn drop(&mut self) {
        RUN_DEPTH.fetch_sub(1, Ordering::Relaxed);
        if let Some(i) = self.ctx_slot {
            release_run_ctx_slot(i);
        }
    }
}

pub(crate) struct ArmCoreInner {
    pub(crate) engine: Box<dyn ArmEngine>,
    last_thread_id: ThreadId,
    threads: BTreeMap<ThreadId, ThreadState>,
    svc_handlers: BTreeMap<u32, Arc<Box<dyn RegisteredFunction>>>,
    next_stub_address: u32,
    profile: Option<ProfileState>,
}

impl Drop for ArmCoreInner {
    fn drop(&mut self) {
        if let Some(mut profile) = self.profile.take() {
            let batch = drain_samples(&mut profile.samples);
            if !batch.is_empty() {
                (profile.callback)(batch);
            }
        }
    }
}

fn drain_samples(samples: &mut BTreeMap<Vec<u32>, u64>) -> Vec<ProfileSample> {
    core::mem::take(samples)
        .into_iter()
        .map(|(stack, count)| ProfileSample { stack, count })
        .collect()
}

#[derive(Clone)]
pub struct ArmCore {
    pub(crate) inner: Arc<Mutex<ArmCoreInner>>, // TODO can we change it to another lock like async-lock?
}

/// Whether ANY core was built with guest PC profiling on. Read once per engine batch by
/// [`ArmCore::sample_profile`] so the common case — profiling off, which is every build
/// the player ever runs — does not take the core's lock ~644,000 times a second to find
/// that out. Set at construction, never cleared (a core with a profile lives for the run).
static PROFILING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

impl ArmCore {
    pub fn new(enable_gdbserver: bool, profile: Option<ProfileCallback>) -> Result<Self> {
        let mut engine = if enable_gdbserver {
            #[cfg(not(target_arch = "wasm32"))]
            let engine = Box::new(DebuggedArm32CpuEngine::new()) as Box<dyn ArmEngine>;
            #[cfg(target_arch = "wasm32")]
            let engine = Box::new(Arm32CpuEngine::new());

            engine
        } else {
            Box::new(Arm32CpuEngine::new())
        };

        engine.mem_map(FUNCTIONS_BASE, FUNCTIONS_SIZE, MemoryPermission::ReadExecute);
        engine.mem_map(GLOBAL_DATA_BASE, 0x4000, MemoryPermission::ReadWriteExecute);

        let profile = profile.map(|callback| ProfileState {
            samples: BTreeMap::new(),
            counter: 0,
            callback,
        });
        if profile.is_some() {
            PROFILING.store(true, core::sync::atomic::Ordering::Relaxed);
        }

        let inner = ArmCoreInner {
            engine,
            last_thread_id: 0,
            threads: BTreeMap::new(),
            svc_handlers: BTreeMap::new(),
            next_stub_address: FUNCTIONS_BASE,
            profile,
        };

        let result = Self {
            inner: Arc::new(Mutex::new(inner)),
        };

        if enable_gdbserver {
            #[cfg(not(target_arch = "wasm32"))]
            GdbTarget::start(result.clone())?;
            #[cfg(target_arch = "wasm32")]
            panic!("GDB server is not supported on wasm32");
        }

        Ok(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn debug_inner(&self) -> Option<Arc<DebugInner>> {
        let inner = self.inner.lock();

        inner
            .engine
            .as_any()
            .downcast_ref::<DebuggedArm32CpuEngine>()
            .map(|engine| engine.debug_inner())
    }

    pub fn load(&mut self, data: &[u8], address: u32, map_size: usize) -> Result<()> {
        let mut inner = self.inner.lock();

        inner
            .engine
            .mem_map(address, map_size.next_multiple_of(0x1000), MemoryPermission::ReadWriteExecute);
        inner.engine.mem_write(address, data)?;

        Ok(())
    }

    pub fn run_in_thread<F, Fut>(&self, entry: F) -> Result<ArmCoreThreadWrapper>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let state = ThreadState::new(self.clone())?;

        let thread_id = {
            let mut inner = self.inner.lock();

            let thread_id = inner.last_thread_id + 1;
            inner.last_thread_id += 1;
            inner.threads.insert(thread_id, state);

            thread_id
        };

        tracing::info!("Create thread: {thread_id}");

        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(debug) = self.debug_inner() {
                debug.on_thread_created(thread_id);
            }
        }

        ArmCoreThreadWrapper::new(self.clone(), thread_id, entry)
    }

    pub fn delete_thread_context(&self, thread_id: ThreadId) {
        tracing::info!("Terminate thread: {thread_id}");

        // we should exit inner lock first to run cleanup on thread state drop
        let _thread_state = {
            let mut inner = self.inner.lock();
            inner.threads.remove(&thread_id)
        };

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(debug) = self.debug_inner() {
            debug.on_thread_deleted(thread_id);
        }
    }

    pub fn enter_thread_context(&self, thread_id: ThreadId) -> ThreadContextGuard {
        ThreadContextGuard::new(self.clone(), thread_id)
    }

    pub fn read_thread_context(&self, thread_id: ThreadId) -> Result<ArmCoreContext> {
        let inner = self.inner.lock();

        let context = inner.threads.get(&thread_id).unwrap().context.clone();

        Ok(context)
    }

    pub fn write_thread_context(&mut self, thread_id: ThreadId, context: &ArmCoreContext) {
        let mut inner = self.inner.lock();

        inner.threads.get_mut(&thread_id).unwrap().context = context.clone();
    }

    pub fn get_thread_ids(&self) -> Vec<ThreadId> {
        let inner = self.inner.lock();

        inner.threads.keys().cloned().collect()
    }

    /// One context's registers, in a fixed order. `CONTEXT_WORDS` of them.
    ///
    /// All seventeen, including `sl`, `lr` and `pc`, which can never be an object: the
    /// range test in [`Self::scan_conservative_roots`] rejects them for free, and a list
    /// that has to be kept in step with the ABI is a list that will one day be wrong in
    /// the direction that costs a character.
    fn context_words(context: &ArmCoreContext) -> [u32; CONTEXT_WORDS] {
        [
            context.r0,
            context.r1,
            context.r2,
            context.r3,
            context.r4,
            context.r5,
            context.r6,
            context.r7,
            context.r8,
            context.sb,
            context.sl,
            context.fp,
            context.ip,
            context.sp,
            context.lr,
            context.pc,
            context.cpsr,
        ]
    }

    /// ★★ EVERY GUEST WORD THAT COULD BE A LIVE OBJECT, for a conservative GC root scan.
    ///
    /// Calls `visit` with each 4-aligned word in `[BUCKET_HEAP_BASE, BUCKET_HEAP_END)`
    /// found in the guest's own state. There are four places such a word can hide, and the
    /// collector can reach none of them on its own:
    ///
    ///   1. the register file executing right now;
    ///   2. the register files nested [`Self::run_function`] calls parked (see `RUN_CTX`) —
    ///      this is the one the 지옥불 crash needed;
    ///   3. each guest thread's own saved register file;
    ///   4. the LIVE part of each guest thread's stack, `[sp, stack_top)`.
    ///
    /// ⛔ CONSERVATIVE means it can only ever keep a dead object alive, never free a live
    /// one: an integer that happens to look like a heap address costs one object's worth of
    /// memory until it changes, which is the direction this whole scan exists to choose.
    ///
    /// ⛔ It does NOT scan the whole 1 MB stack. Stacks grow down, so everything any frame
    /// of a thread can still reach is above the LOWEST stack pointer seen for that stack —
    /// and the nested contexts of (2) are exactly where the deeper ones are recorded.
    pub fn scan_conservative_roots(&self, visit: &mut dyn FnMut(u32)) {
        // ★ roadmap #12: the scan reports its own size. `examined`/`kept` are plain locals
        // — one `fetch_add` at the end, none inside the loop — so the measurement cannot
        // become the thing it measures. See `wie_util::stats::CONS_WORDS`.
        let (mut examined, mut kept, mut stack_bytes) = (0u64, 0u64, 0u64);
        let mut consider = |word: u32, visit: &mut dyn FnMut(u32), examined: &mut u64, kept: &mut u64| {
            *examined += 1;
            if word >= BUCKET_HEAP_BASE && word < BUCKET_HEAP_END && word & 3 == 0 {
                *kept += 1;
                visit(word);
            }
        };

        // (1) and (2) — and collect their stack pointers, which is how (4) learns how deep
        // each stack currently is. The running frame's `sp` is in the engine, not in the
        // thread record, and the nested frames' are in neither.
        let mut sps: Vec<u32> = Vec::with_capacity(RUN_TRACE_MAX + 4);

        let current = self.save_context();
        sps.push(current.sp);
        for word in Self::context_words(&current) {
            consider(word, visit, &mut examined, &mut kept);
        }

        let used = RUN_CTX_USED.load(Ordering::Relaxed);
        for (bit, slot) in RUN_CTX.iter().enumerate() {
            if used & (1 << bit) == 0 {
                continue;
            }
            for (i, reg) in slot.iter().enumerate() {
                let word = reg.load(Ordering::Relaxed);
                if i == SP_WORD_INDEX {
                    sps.push(word);
                }
                consider(word, visit, &mut examined, &mut kept);
            }
        }

        // (3) — and the stack geometry, taken in one lock so the scan itself holds none.
        let threads: Vec<(ArmCoreContext, u32, u32)> = {
            let inner = self.inner.lock();
            inner
                .threads
                .values()
                .map(|state| (state.context.clone(), state.stack_base as u32, state.stack_size as u32))
                .collect()
        };
        for (context, _, _) in &threads {
            sps.push(context.sp);
            for word in Self::context_words(context) {
                consider(word, visit, &mut examined, &mut kept);
            }
        }

        // (4) The live span of each stack: from the lowest stack pointer that falls inside
        // it up to its top. A stack pointer that is garbage or zero simply does not fall
        // inside any stack, so it cannot turn this into a scan of the whole 256 MB heap.
        let mut buf = Vec::new();
        for (context, base, size) in &threads {
            let top = base.saturating_add(*size);
            let low = sps
                .iter()
                .copied()
                .filter(|sp| *sp >= *base && *sp <= top)
                .min()
                .unwrap_or(context.sp)
                .clamp(*base, top)
                & !3;

            let len = (top - low) as usize;
            if len == 0 {
                continue;
            }
            buf.clear();
            buf.resize(len, 0);
            // A read that fails costs us the roots on that stack, so say it rather than
            // letting a collection quietly go back to the behaviour this scan replaced.
            if self.read_bytes(low, &mut buf).is_err() {
                tracing::error!("could not read guest stack {low:#x}..{top:#x} for the root scan");
                continue;
            }
            // ⛔ Charged AFTER the read succeeds (2026-09-21, review). Counting it first
            // meant the failure path reported a bigger scan that had examined nothing:
            // `stack/scan` and `words/scan` are the whole evidence for this feature's
            // cost, and they would have grown at the exact moment the scan silently
            // stopped finding roots.
            stack_bytes += len as u64;
            for word in buf.chunks_exact(4) {
                consider(u32::from_le_bytes([word[0], word[1], word[2], word[3]]), visit, &mut examined, &mut kept);
            }
        }

        wie_util::stats::add_cons_scan(examined, stack_bytes, kept);
    }

    fn sample_profile(&self) {
        // ★ 2026-09-18: the `is_none` test used to be INSIDE the lock, so every engine
        //   batch — 644,000 of them a second on the probe, one per ~79 guest instructions
        //   — took and dropped a spin lock to learn that guest profiling was off. The
        //   flag is host state, not engine state; ask it first.
        if !PROFILING.load(core::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let mut inner = self.inner.lock();
        if inner.profile.is_none() {
            return;
        }
        let pc = inner.engine.reg_read(ArmRegister::PC);
        let mut stack = vec![pc];
        let mut r7 = inner.engine.reg_read(ArmRegister::R7);
        for _ in 0..PROFILE_MAX_STACK {
            // Thumb frame: [saved R7 | saved LR] at [r7].
            let mut buf = [0u8; 8];
            if inner.engine.mem_read(r7, 8, &mut buf).is_err() {
                break;
            }
            let prev_r7 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            let lr = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            // Heuristic stop: zero/null frame or non-Thumb LR (we only ever
            // call into Thumb code from the guest).
            if prev_r7 == 0 || lr == 0 || lr & 1 == 0 {
                break;
            }
            stack.push(lr);
            if prev_r7 <= r7 {
                // R7 chain must walk upward; bail on any inversion to avoid loops.
                break;
            }
            r7 = prev_r7;
        }
        let profile = inner.profile.as_mut().unwrap();
        *profile.samples.entry(stack).or_insert(0) += 1;
        profile.counter = profile.counter.wrapping_add(1);
        if profile.counter >= PROFILE_FLUSH_INTERVAL {
            profile.counter = 0;
            let batch = drain_samples(&mut profile.samples);
            (profile.callback)(batch);
        }
    }

    pub async fn run_function<R>(&mut self, address: u32, params: &[u32]) -> Result<R>
    where
        R: RunFunctionResult<R>,
    {
        let depth = RUN_DEPTH.fetch_add(1, Ordering::Relaxed) + 1;
        // Claimed here, next to the depth it is no longer indexed by, so every `?` below
        // releases it through the same guard.
        let ctx_slot = claim_run_ctx_slot();
        let _run_depth_guard = RunDepthGuard { ctx_slot };
        if depth <= RUN_TRACE_MAX {
            RUN_TRACE[depth - 1].store(address as usize, Ordering::Relaxed);
        }
        if depth > RUN_DEPTH_MAX.load(Ordering::Relaxed) {
            RUN_DEPTH_MAX.store(depth, Ordering::Relaxed);
            if depth >= 8 {
                let mut chain = String::new();
                for slot in RUN_TRACE.iter().take(depth.min(RUN_TRACE_MAX)) {
                    chain += &format!("{:#x} ", slot.load(Ordering::Relaxed));
                }
                tracing::warn!("run_function nesting depth {depth}: {chain}");
            }
        }
        // Native stack consumed between the top of this poll chain and here. The
        // baseline is stamped by `ArmCoreThreadWrapper::poll` at its own depth 1,
        // i.e. the outermost frame of the executor's poll of this task.
        let here = &depth as *const usize as usize;
        let base = crate::thread_wrapper::STACK_BASE.load(Ordering::Relaxed);
        if base > here {
            let used = base - here;
            // Report in 64 KB steps so a healthy run is quiet and a leaking one
            // prints a staircase toward the 8 MB `EMU_STACK`.
            if used > STACK_HIGH.load(Ordering::Relaxed) + 65536 {
                STACK_HIGH.store(used, Ordering::Relaxed);
                tracing::warn!("guest-call native stack watermark {} KB (depth {depth})", used / 1024);
            }
        }

        // we don't need to save r0-r3, but to make it simple, we save all registers
        let previous_context = self.save_context();
        // …and publish them where the collector can see them. See `RUN_CTX`: from here
        // until `restore_context` below, this frame's java locals exist ONLY in this Rust
        // local, and a collection that runs in the callee would otherwise not see them.
        // ★ Nothing between the claim and here can collect (no await, no allocation), so
        // the scan never sees the previous owner's words in this slot.
        if let Some(i) = ctx_slot {
            for (dst, word) in RUN_CTX[i].iter().zip(Self::context_words(&previous_context)) {
                dst.store(word, Ordering::Relaxed);
            }
        }
        {
            let mut inner = self.inner.lock();

            if !params.is_empty() {
                inner.engine.reg_write(ArmRegister::R0, params[0]);
            }
            if params.len() > 1 {
                inner.engine.reg_write(ArmRegister::R1, params[1]);
            }
            if params.len() > 2 {
                inner.engine.reg_write(ArmRegister::R2, params[2]);
            }
            if params.len() > 3 {
                inner.engine.reg_write(ArmRegister::R3, params[3]);
            }
            if params.len() > 4 {
                for param in params[4..].iter().rev() {
                    let sp: u32 = inner.engine.reg_read(ArmRegister::SP) - 4;

                    inner.engine.mem_write(sp, &param.to_le_bytes())?;
                    inner.engine.reg_write(ArmRegister::SP, sp);
                }
            }

            inner.engine.reg_write(ArmRegister::PC, address);
            inner.engine.reg_write(ArmRegister::LR, RUN_FUNCTION_LR);

            let cpsr = inner.engine.reg_read(ArmRegister::Cpsr);
            let new_cpsr = (cpsr & !0x3f) | 0x1f | ((address & 1) << 5);
            inner.engine.reg_write(ArmRegister::Cpsr, new_cpsr);
        }

        loop {
            let result = {
                // ★ The two phases the host profiler (#9) splits the frame into at this
                //   level: interpreting guest ARM, and running the Rust behind an SVC.
                //   Everything else the emulator thread does is inside one of the two.
                let _phase = wie_util::stats::phase(wie_util::stats::PHASE_ENGINE);
                let mut inner = self.inner.lock();
                inner.engine.run(RUN_FUNCTION_LR, 1000)?
            };

            self.sample_profile();

            match result {
                EngineRunResult::End => break,
                EngineRunResult::CountExhausted => {}
                EngineRunResult::Svc { category, lr, spsr } => {
                    {
                        let mut inner = self.inner.lock();
                        // Restore the pre-exception execution state before running the Rust SVC handler.
                        inner.engine.reg_write(ArmRegister::Cpsr, spsr);
                        inner.engine.reg_write(ArmRegister::PC, lr);
                    }

                    let function = {
                        let inner = self.inner.lock();
                        inner
                            .svc_handlers
                            .get(&category)
                            .cloned()
                            .ok_or_else(|| WieError::FatalError(format!("Unknown SVC handler category: {category}")))?
                    };

                    let mut self1 = self.clone();
                    // Nested: this handler can call back into the engine (`run_function`),
                    // which publishes `PHASE_ENGINE` and restores this one on its way out,
                    // so a sample always names the innermost phase.
                    let _phase = wie_util::stats::phase(wie_util::stats::PHASE_SVC + (category as u8 & 7));
                    function.call(&mut self1).await?;
                }
            }
        }

        let result = R::get(self);
        self.restore_context(&previous_context);

        Ok(result)
    }

    pub fn register_svc_handler<F, C, R, P>(&mut self, category: u32, handler: F, context: &C) -> Result<()>
    where
        F: EmulatedFunction<C, R, P> + 'static + Sync + Send,
        C: Clone + 'static + Sync + Send,
        R: ResultWriter<R> + Sync + Send + 'static,
        P: Sync + Send + 'static,
    {
        let mut inner = self.inner.lock();

        if inner.svc_handlers.contains_key(&category) {
            return Err(WieError::FatalError(format!("SVC handler already registered for {category}")));
        }

        inner
            .svc_handlers
            .insert(category, Arc::new(Box::new(RegisteredFunctionHolder::new(handler, context))));

        Ok(())
    }

    pub fn make_svc_stub(&mut self, category: u32, id: impl Into<u32>) -> Result<u32> {
        let mut inner = self.inner.lock();
        let id = id.into();

        if !inner.svc_handlers.contains_key(&category) {
            return Err(WieError::FatalError(format!("Unknown SVC handler category: {category}")));
        }

        let address = inner.next_stub_address;
        if address + SVC_STUB_SIZE > FUNCTIONS_BASE + FUNCTIONS_SIZE as u32 {
            return Err(WieError::FatalError("SVC stub space exhausted".into()));
        }
        inner.next_stub_address += SVC_STUB_SIZE;

        let stub = [
            0x10,
            0xb4, // push {r4}
            0x02,
            0x4c, // ldr r4, [pc, #8]
            0xa4,
            0x46, // mov r12, r4
            0x10,
            0xbc, // pop {r4}
            category as u8,
            0xdf, // svc #category
            0x70,
            0x47, // bx lr
        ]
        .into_iter()
        .chain(id.to_le_bytes())
        .collect::<Vec<_>>();
        inner.engine.mem_write(address, &stub)?;

        tracing::trace!("Register SVC stub at {address:#x}, category={category}, id={id}");

        Ok(address + 1)
    }

    pub fn map(&mut self, address: u32, size: u32) -> Result<()> {
        tracing::trace!("Map address: {address:#x}, size: {size:#x}");

        let mut inner = self.inner.lock();

        inner.engine.mem_map(address, size as usize, MemoryPermission::ReadWrite);

        Ok(())
    }

    pub fn dump_reg_stack(&self, image_base: u32) -> String {
        format!(
            "\n{}\nPossible call stack:\n{}\nStack:\n{}",
            self.dump_regs(),
            self.dump_call_stack(image_base).unwrap(),
            self.dump_stack().unwrap()
        )
    }

    pub fn restore_context(&mut self, context: &ArmCoreContext) {
        let mut inner = self.inner.lock();

        inner.engine.reg_write(ArmRegister::R0, context.r0);
        inner.engine.reg_write(ArmRegister::R1, context.r1);
        inner.engine.reg_write(ArmRegister::R2, context.r2);
        inner.engine.reg_write(ArmRegister::R3, context.r3);
        inner.engine.reg_write(ArmRegister::R4, context.r4);
        inner.engine.reg_write(ArmRegister::R5, context.r5);
        inner.engine.reg_write(ArmRegister::R6, context.r6);
        inner.engine.reg_write(ArmRegister::R7, context.r7);
        inner.engine.reg_write(ArmRegister::R8, context.r8);
        inner.engine.reg_write(ArmRegister::SB, context.sb);
        inner.engine.reg_write(ArmRegister::SL, context.sl);
        inner.engine.reg_write(ArmRegister::FP, context.fp);
        inner.engine.reg_write(ArmRegister::IP, context.ip);
        inner.engine.reg_write(ArmRegister::SP, context.sp);
        inner.engine.reg_write(ArmRegister::LR, context.lr);
        inner.engine.reg_write(ArmRegister::PC, context.pc);
        inner.engine.reg_write(ArmRegister::Cpsr, context.cpsr);
    }

    pub fn save_context(&self) -> ArmCoreContext {
        let inner = self.inner.lock();

        ArmCoreContext {
            r0: inner.engine.reg_read(ArmRegister::R0),
            r1: inner.engine.reg_read(ArmRegister::R1),
            r2: inner.engine.reg_read(ArmRegister::R2),
            r3: inner.engine.reg_read(ArmRegister::R3),
            r4: inner.engine.reg_read(ArmRegister::R4),
            r5: inner.engine.reg_read(ArmRegister::R5),
            r6: inner.engine.reg_read(ArmRegister::R6),
            r7: inner.engine.reg_read(ArmRegister::R7),
            r8: inner.engine.reg_read(ArmRegister::R8),
            sb: inner.engine.reg_read(ArmRegister::SB),
            sl: inner.engine.reg_read(ArmRegister::SL),
            fp: inner.engine.reg_read(ArmRegister::FP),
            ip: inner.engine.reg_read(ArmRegister::IP),
            sp: inner.engine.reg_read(ArmRegister::SP),
            lr: inner.engine.reg_read(ArmRegister::LR),
            pc: inner.engine.reg_read(ArmRegister::PC),
            cpsr: inner.engine.reg_read(ArmRegister::Cpsr),
        }
    }

    pub fn read_pc_lr(&self) -> Result<(u32, u32)> {
        let inner = self.inner.lock();

        let lr = inner.engine.reg_read(ArmRegister::LR);
        let pc = inner.engine.reg_read(ArmRegister::PC);

        Ok((pc, lr))
    }

    pub fn write_return_value(&mut self, result: &[u32]) -> Result<()> {
        let mut inner = self.inner.lock();

        if !result.is_empty() {
            inner.engine.reg_write(ArmRegister::R0, result[0]);
        }
        if result.len() > 1 {
            inner.engine.reg_write(ArmRegister::R1, result[1]);
        }
        if result.len() > 2 {
            todo!() // TODO
        }

        Ok(())
    }

    pub fn set_next_pc(&mut self, pc: u32) -> Result<()> {
        let mut inner = self.inner.lock();

        inner.engine.reg_write(ArmRegister::PC, pc);

        let cpsr = inner.engine.reg_read(ArmRegister::Cpsr);
        let new_cpsr = if pc & 1 == 1 { cpsr | 0x20 } else { cpsr & !0x20 };
        inner.engine.reg_write(ArmRegister::Cpsr, new_cpsr);

        Ok(())
    }

    pub fn read_param(&self, pos: usize) -> Result<u32> {
        let inner = self.inner.lock();

        let result = if pos == 0 {
            inner.engine.reg_read(ArmRegister::R0)
        } else if pos == 1 {
            inner.engine.reg_read(ArmRegister::R1)
        } else if pos == 2 {
            inner.engine.reg_read(ArmRegister::R2)
        } else if pos == 3 {
            inner.engine.reg_read(ArmRegister::R3)
        } else {
            let sp = inner.engine.reg_read(ArmRegister::SP);

            drop(inner);

            read_generic(self, sp + 4 * (pos as u32 - 4))?
        };

        Ok(result)
    }

    pub(crate) fn dump_regs_inner(engine: &dyn ArmEngine) -> String {
        [
            format!(
                "R0: {:#x} R1: {:#x} R2: {:#x} R3: {:#x} R4: {:#x} R5: {:#x} R6: {:#x} R7: {:#x} R8: {:#x}",
                engine.reg_read(ArmRegister::R0),
                engine.reg_read(ArmRegister::R1),
                engine.reg_read(ArmRegister::R2),
                engine.reg_read(ArmRegister::R3),
                engine.reg_read(ArmRegister::R4),
                engine.reg_read(ArmRegister::R5),
                engine.reg_read(ArmRegister::R6),
                engine.reg_read(ArmRegister::R7),
                engine.reg_read(ArmRegister::R8),
            ),
            format!(
                "SB: {:#x} SL: {:#x} FP: {:#x} IP: {:#x} SP: {:#x} LR: {:#x} PC: {:#x}",
                engine.reg_read(ArmRegister::SB),
                engine.reg_read(ArmRegister::SL),
                engine.reg_read(ArmRegister::FP),
                engine.reg_read(ArmRegister::IP),
                engine.reg_read(ArmRegister::SP),
                engine.reg_read(ArmRegister::LR),
                engine.reg_read(ArmRegister::PC),
            ),
            format!("CPSR: {:032b}\n", engine.reg_read(ArmRegister::Cpsr)),
        ]
        .join("\n")
    }

    fn is_code_address(address: u32, image_base: u32) -> bool {
        // TODO image size temp

        (address % 2 == 1 && (image_base..image_base + 0x100000).contains(&address))
            || (FUNCTIONS_BASE..FUNCTIONS_BASE + FUNCTIONS_SIZE as u32).contains(&address)
    }

    fn dump_regs(&self) -> String {
        let inner = self.inner.lock();

        Self::dump_regs_inner(&*inner.engine)
    }

    fn format_callstack_address(address: u32, image_base: u32) -> String {
        let description = if (image_base..image_base + 0x100000).contains(&address) {
            format!("<Base>+{:#x}", address - image_base)
        } else if (FUNCTIONS_BASE..FUNCTIONS_BASE + FUNCTIONS_SIZE as u32).contains(&address) {
            "<Native function>".to_owned()
        } else {
            "<Unknown>".to_owned()
        };

        format!("{address:#x}: {description}\n")
    }

    fn dump_call_stack(&self, image_base: u32) -> Result<String> {
        let mut inner = self.inner.lock();

        let sp = inner.engine.reg_read(ArmRegister::SP);
        let pc = inner.engine.reg_read(ArmRegister::PC);
        let lr = inner.engine.reg_read(ArmRegister::LR);

        let mut call_stack = Self::format_callstack_address(pc, image_base);
        if lr != RUN_FUNCTION_LR && lr != 0 {
            call_stack += &Self::format_callstack_address(lr - 5, image_base);
        }

        for i in 0..128 {
            let address = sp + (i * 4);
            if !inner.engine.is_mapped(address, size_of::<u32>()) {
                break;
            }

            let mut value = [0; size_of::<u32>()];
            inner.engine.mem_read(address, size_of::<u32>(), &mut value)?;
            let value_u32 = u32::from_le_bytes(value);

            if value_u32 > 5 && Self::is_code_address(value_u32 - 4, image_base) {
                call_stack += &Self::format_callstack_address(value_u32 - 5, image_base);
            }
        }

        Ok(call_stack)
    }

    fn dump_stack(&self) -> Result<String> {
        let mut inner = self.inner.lock();

        let sp = inner.engine.reg_read(ArmRegister::SP);

        let mut result = String::new();
        for i in 0..16 {
            let address = sp + (i * 4);

            if !inner.engine.is_mapped(address, size_of::<u32>()) {
                break;
            }

            let mut value = [0; size_of::<u32>()];
            inner.engine.mem_read(address, size_of::<u32>(), &mut value)?;
            let value_u32 = u32::from_le_bytes(value);

            result += &format!("SP+{:#x}: {value_u32:#x}\n", i * 4);
        }

        Ok(result)
    }
}

impl ByteRead for ArmCore {
    fn read_bytes(&self, address: u32, result: &mut [u8]) -> wie_util::Result<usize> {
        let mut inner = self.inner.lock();

        let read = inner.engine.mem_read(address, result.len(), result)?;

        Ok(read)
    }
}

impl ByteWrite for ArmCore {
    fn write_bytes(&mut self, address: u32, data: &[u8]) -> wie_util::Result<()> {
        let mut inner = self.inner.lock();

        inner.engine.mem_write(address, data)?;

        Ok(())
    }
}

pub trait RunFunctionResult<R> {
    fn get(core: &ArmCore) -> R;
}

impl RunFunctionResult<u32> for u32 {
    fn get(core: &ArmCore) -> u32 {
        core.read_param(0).unwrap()
    }
}

impl RunFunctionResult<()> for () {
    fn get(_: &ArmCore) {}
}

pub struct ThreadContextGuard {
    core: ArmCore,
    thread_id: ThreadId,
}

impl ThreadContextGuard {
    pub fn new(mut core: ArmCore, thread_id: ThreadId) -> Self {
        let context = core.inner.lock().threads.get(&thread_id).unwrap().context.clone(); // TODO we might not need clone
        core.restore_context(&context);

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(debug) = core.debug_inner() {
            debug.on_thread_entered(thread_id);
        }

        Self { core, thread_id }
    }
}

impl Drop for ThreadContextGuard {
    fn drop(&mut self) {
        let context = self.core.save_context();

        let mut inner = self.core.inner.lock();
        inner.threads.get_mut(&self.thread_id).unwrap().context = context;
        drop(inner);

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(debug) = self.core.debug_inner() {
            debug.on_thread_exited(self.thread_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conservative root scan reads `RUN_CTX[..][SP_WORD_INDEX]` as a stack pointer to
    /// decide how deep each guest stack currently is. If that index ever stops naming `sp`,
    /// the scan keeps working — it just stops covering the stack, silently, in the
    /// direction that frees a live object.
    #[test]
    fn sp_word_index_matches_context_words() {
        let mut context = ArmCore::new(false, None).unwrap().save_context();
        context.sp = 0xdead_beef;

        let words = ArmCore::context_words(&context);
        assert_eq!(words.len(), CONTEXT_WORDS);
        assert_eq!(words[SP_WORD_INDEX], 0xdead_beef);
        assert_eq!(words.iter().filter(|w| **w == 0xdead_beef).count(), 1, "sp must be unambiguous");
    }

    /// Slots are OWNED, not indexed by nesting depth: a frame parked at a deeper slot must
    /// stay visible to the scan after a shallower one returns, and its slot must not be
    /// handed to the next frame. (`run_function` is async — guest threads are separate
    /// executor tasks, so the depths are released in completion order, not LIFO.)
    #[test]
    fn a_parked_run_ctx_slot_survives_a_shallower_frame_returning() {
        let outer = claim_run_ctx_slot().expect("a free slot");
        let parked = claim_run_ctx_slot().expect("a second free slot");
        assert_ne!(outer, parked);

        RUN_CTX[parked][0].store(0xdead_beef, Ordering::Relaxed);
        // The shallower frame returns first.
        release_run_ctx_slot(outer);
        assert_ne!(RUN_CTX_USED.load(Ordering::Relaxed) & (1 << parked), 0, "the parked frame lost its slot");
        // ★ Not `assert_eq!(.., outer)`: cargo runs these in parallel threads and another
        // test inside `run_function` holds slots of its own. What must hold is that a new
        // claim can never be the parked one.
        let reused = claim_run_ctx_slot().expect("a free slot");
        assert_ne!(reused, parked, "a claim took a slot that is still parked");
        assert_eq!(RUN_CTX[parked][0].load(Ordering::Relaxed), 0xdead_beef, "the parked registers were overwritten");

        release_run_ctx_slot(reused);
        release_run_ctx_slot(parked);
    }

    /// A conservative scan may only ever err towards KEEPING an object. The filter is what
    /// makes it affordable, so it has to reject everything that cannot be an instance
    /// record — and accept the whole bucket half, which is where every record lives.
    #[test]
    fn only_bucket_half_addresses_are_offered_as_roots() {
        let core = ArmCore::new(false, None).unwrap();
        let mut seen = Vec::new();
        core.scan_conservative_roots(&mut |word| seen.push(word));

        for word in &seen {
            assert!((BUCKET_HEAP_BASE..BUCKET_HEAP_END).contains(word), "{word:#x} is not an instance address");
            assert_eq!(word & 3, 0, "{word:#x} is not word-aligned");
        }
        // The list half holds the thread stacks themselves; offering those as roots would
        // make every stack pointer on every stack look like an object.
        assert!(BUCKET_HEAP_BASE > HEAP_BASE);
        assert_eq!(BUCKET_HEAP_END, HEAP_BASE + HEAP_SIZE);
    }

    async fn test_svc_handler(_core: &mut ArmCore, seen_id: &mut Option<u32>, id: crate::SvcId) -> Result<()> {
        *seen_id = Some(id.0);

        Ok(())
    }

    #[test]
    fn test_thumb_svc_stub_dispatch() {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x1000, 0x1000).unwrap();

        let mut context = core.save_context();
        context.sp = 0x2000;
        core.restore_context(&context);

        core.register_svc_handler(1, test_svc_handler, &None).unwrap();
        let first = core.make_svc_stub(1, 0u32).unwrap();
        let second = core.make_svc_stub(1, 1u32).unwrap();
        assert_eq!(first, FUNCTIONS_BASE + 1);
        assert_eq!(second, FUNCTIONS_BASE + SVC_STUB_SIZE + 1);

        let result = {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::Cpsr, 0x3f);
            inner.engine.reg_write(ArmRegister::PC, second);
            inner.engine.reg_write(ArmRegister::LR, RUN_FUNCTION_LR);
            inner.engine.run(RUN_FUNCTION_LR, 10).unwrap()
        };

        match result {
            EngineRunResult::Svc { category, lr, spsr } => {
                assert_eq!(category, 1);
                assert_eq!(lr, FUNCTIONS_BASE + SVC_STUB_SIZE + 10);
                assert_ne!(spsr & 0x20, 0);
            }
            EngineRunResult::End => panic!("expected SVC, got end"),
            EngineRunResult::CountExhausted => panic!("expected SVC, got count exhausted"),
        }
    }
}
