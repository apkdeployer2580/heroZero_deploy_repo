#![no_std]
extern crate alloc;

use alloc::{format, string::String, vec::Vec};
use core::{
    any::Any,
    error::Error,
    fmt::{self, Display, Formatter},
    mem::{MaybeUninit, size_of},
    result,
    slice::from_raw_parts_mut,
};

use bytemuck::{AnyBitPattern, NoUninit, bytes_of};

#[derive(Debug)]
pub enum WieError {
    InvalidMemoryAccess(u32),
    AllocationFailure,
    JavaException(u32), // to pass java exception down to rust
    JavaExceptionUnwind { context_base: u32, target: u32, next_pc: u32 },
    Unimplemented(String),
    FatalError(String),
}

impl Display for WieError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            WieError::InvalidMemoryAccess(address) => write!(f, "Invalid memory access; address: {address}"),
            WieError::AllocationFailure => write!(f, "Allocation failure"),
            WieError::JavaException(exception) => write!(f, "Java exception: {exception:#x}"),
            WieError::JavaExceptionUnwind {
                context_base,
                target,
                next_pc,
            } => write!(
                f,
                "Java exception unwind: context_base={context_base:#x}, target={target:#x}, next_pc={next_pc:#x}"
            ),
            WieError::Unimplemented(message) => write!(f, "Unimplemented: {message}"),
            WieError::FatalError(message) => write!(f, "Fatal error: {message}"),
        }
    }
}

impl Error for WieError {}

pub type Result<T> = result::Result<T, WieError>;

/// ★ A STUB THE GAME CALLS IN ITS INNER LOOP MUST NOT NARRATE (2026-09-18).
///
/// Measured on device (Galaxy S25, hero2, standing still on the field with no input):
/// `stub java_check_type` alone came out at **~1,850 lines per second**, 481,733 of them
/// in one minute — and every one is a format plus a write to `logd` **from the emulator
/// thread**, the one thread that has to meet the guest's frame budget. It also buried
/// every other line in the log at ~1,800:1, which is its own cost: the log is how this
/// port is debugged.
///
/// So: say it the 1st, 2nd, 4th, 8th … time and no other. The magnitude stays visible
/// (the count is in the line), a stub that fires once still shows up immediately, and the
/// steady-state cost is one relaxed atomic add. `tracing::warn!` itself is left alone
/// everywhere it is used for something that happens at human rates.
///
/// Lives here, not in one emulator crate, because the hot stubs are on both sides of the
/// boundary: `wie_ktf`'s Java interface SVCs and `wie_wipi_java`'s per-draw graphics
/// methods. Every caller needs `tracing` and `core`, which every crate in this tree has.
#[macro_export]
macro_rules! stub_warn {
    ($($arg:tt)*) => {{
        static SEEN: ::core::sync::atomic::AtomicU64 = ::core::sync::atomic::AtomicU64::new(0);
        let n = SEEN.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
        if n.is_power_of_two() {
            ::tracing::warn!("[x{}] {}", n, ::core::format_args!($($arg)*));
        }
    }};
}

/// TEMPORARY measurement (2026-08-06): where does a frame actually go?
/// The game renders its world by writing the framebuffer from guest code, so the
/// interpreter — not the WIPI graphics layer — is the suspect. Counted here (a
/// no_std-safe atomic, bumped once per `ArmEngine::run` batch, not per
/// instruction) so `wie_probe` can print instructions/frame.
pub mod stats {
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

    // ── ★★ THE HOST PHASE PROFILER (2026-09-18, roadmap #9) ──────────────────────
    //
    // The port's frame budget is spent by the HOST, not by the guest's instructions:
    // ~8.5 ms of host time per painted frame on this desktop, ~20 ms on a Galaxy S25,
    // for a guest that asks to be woken every 50 ms. Until now nothing could say where
    // it went. `--profile` samples the GUEST pc, so everything the host does — the SVC
    // handlers, the JVM, the collector, the pixel paths — is one anonymous lump to it,
    // and "96 % is the host/guest boundary" was the whole of what could be said.
    //
    // This is the cheapest thing that can answer it and still run on a phone: the
    // emulator thread publishes WHICH PHASE it is in as one relaxed byte, and a sampler
    // thread (host side, `std`) reads that byte on a fixed tick and bins it. No clock
    // reads on the hot path, no stack walking, no per-SVC timing — 86,000 SVCs a second
    // would swamp anything that cost more than a store. A phase transition is a load and
    // a store; the histogram is one atomic add per SAMPLE (a thousand a second).
    //
    // Nesting is real (an SVC handler calls back into the engine, which takes another
    // SVC), so [`PhaseGuard`] restores the phase it displaced — the byte therefore names
    // the INNERMOST phase, which is exactly what a sampler should attribute to.
    pub static PHASE: AtomicU8 = AtomicU8::new(PHASE_IDLE);
    /// Samples binned by phase. Index = the phase byte.
    pub static PHASE_HIST: [AtomicU64; 16] = [
        AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
        AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
        AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
        AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    ];
    /// Nothing is running on the emulator thread (it is sleeping out the frame pace, or
    /// blocked). A sample here is host time the port is NOT spending.
    pub const PHASE_IDLE: u8 = 0;
    /// Inside `ArmEngine::run` — interpreting guest ARM.
    pub const PHASE_ENGINE: u8 = 1;
    /// Inside an SVC handler, `+ category` (1=init 2=java-interface 3=wipi-c 4=java
    /// 5=stdlib, 0=binary-patch hook). See `SVC_BY_CATEGORY`.
    pub const PHASE_SVC: u8 = 2;
    /// A stop-the-world collection.
    pub const PHASE_GC: u8 = 10;
    /// Handing the finished frame to the screen (conversion, hash, cross-thread send).
    pub const PHASE_PAINT: u8 = 11;
    pub const PHASE_NAMES: [&str; 16] = [
        "idle", "engine", "svc:hook", "svc:init", "svc:jif", "svc:wipic", "svc:java", "svc:stdlib",
        "svc:6", "svc:7", "gc", "paint", "?12", "?13", "?14", "?15",
    ];

    /// Publish `p` as the phase and restore the previous one when this drops. The store
    /// is relaxed and unsynchronised on purpose: one thread writes it, and a sampler that
    /// reads a stale byte once in a while costs one misattributed sample out of a
    /// thousand a second.
    #[must_use = "the phase is restored when the guard drops"]
    pub struct PhaseGuard(u8);

    impl PhaseGuard {
        #[inline]
        pub fn new(p: u8) -> Self {
            let prev = PHASE.load(Ordering::Relaxed);
            PHASE.store(p, Ordering::Relaxed);
            Self(prev)
        }
    }

    impl Drop for PhaseGuard {
        #[inline]
        fn drop(&mut self) {
            PHASE.store(self.0, Ordering::Relaxed);
        }
    }

    /// Enter `p` for as long as the returned guard lives. `stats::phase(PHASE_GC)`.
    #[inline]
    pub fn phase(p: u8) -> PhaseGuard {
        PhaseGuard::new(p)
    }

    /// One sample, for the host-side sampler thread to call on its tick.
    pub fn phase_sample() {
        let p = PHASE.load(Ordering::Relaxed) as usize & 15;
        PHASE_HIST[p].fetch_add(1, Ordering::Relaxed);
    }

    /// The histogram since the last call, and RESET — an interval, like the SVC mix, so
    /// a gauge reports what the player is doing now and not an average with the title
    /// screen mixed in.
    pub fn phase_take() -> Vec<u64> {
        PHASE_HIST.iter().map(|c| c.swap(0, Ordering::Relaxed)).collect()
    }

    pub static GUEST_INSTRUCTIONS: AtomicU64 = AtomicU64::new(0);
    /// Frames the GUEST produced (one per `Screen::paint`). Not the compositor's rate:
    /// the host presents at the display's 60 Hz whether or not the emulator managed to
    /// draw anything new, so this is the only number that says how fast the game is
    /// actually running. The game targets 25.
    pub static GUEST_FRAMES: AtomicU64 = AtomicU64::new(0);
    pub static SVC_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RUN_BATCHES: AtomicU64 = AtomicU64::new(0);
    /// ★ SVC MIX (2026-08-13). `SVC_CALLS` alone says the guest left the interpreter N
    /// times; it cannot say whether those were cheap ones (`memcpy` — a plain Rust
    /// helper) or expensive ones (`MC_netSocketRecv` — a context object, a boxed body
    /// and a socket syscall). On LGT the whole C stdlib is an SVC too, so the mix is
    /// the only way to attribute a stall. Keyed by `category` (1=init, 3=wipic,
    /// 5=stdlib, 8+=binary-patch hooks) and by the id the stub leaves in `r12`.
    ///
    /// Ids alias mod 2048 — deliberate: every id Hero3 actually uses (net `0x258..`,
    /// sockets `0x7d0..`, 901/904, and the small stdlib/graphics enums) is below that,
    /// so the aliasing only affects ids nothing calls.
    pub static SVC_BY_CATEGORY: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
    pub static SVC_BY_ID: [AtomicU64; 2048] = [const { AtomicU64::new(0) }; 2048];
    /// Guest-heap allocator traffic. `ALLOCS - FREES` is the live object count;
    /// if it climbs during steady play, that is the exhaustion the device crash
    /// ends in. `LIST_WALK` is the first-fit cursor cost, and `BUCKET_HEADER_BYTES`
    /// the per-allocation bookkeeping read out of guest memory — both were the
    /// measurements that justified the 2026-08-06 allocator rewrite (lazy
    /// coalescing in `ListAllocator::find_address`, and an indexed rather than
    /// scanned in-use map in `BucketAllocator`), so both should now stay flat
    /// where they used to climb.
    pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static FREES: AtomicU64 = AtomicU64::new(0);
    pub static LIST_WALK: AtomicU64 = AtomicU64::new(0);
    pub static BUCKET_HEADER_BYTES: AtomicU64 = AtomicU64::new(0);

    /// ★ THE CONSERVATIVE ROOT SCAN's own cost (2026-09-20, roadmap #12).
    ///
    /// The scan (`ArmCore::scan_conservative_roots`) is what keeps an object alive that
    /// only an AOT frame's ARM register or guest stack word still points at — the 지옥불
    /// crash's fix. Headless it looked free (13~16 CANDIDATE words per collection), but
    /// that number is the output, not the work: the work is every word EXAMINED, and that
    /// grows with how deep the guest stacks are — exactly what a 24-deep `run_function`
    /// nest does. So count the input, not the output.
    ///
    /// `CONS_WORDS` is words examined, `CONS_STACK_BYTES` the part of that which came from
    /// walking `[sp, top)` (the part that grows with depth; the rest is a fixed ~20 words
    /// of register file per thread). Per-word cost is a bounds test plus, for the stack
    /// part, one sequential guest read — so words/s is the cost, and a device can report
    /// it without a second build to diff against.
    pub static CONS_SCANS: AtomicU64 = AtomicU64::new(0);
    pub static CONS_WORDS: AtomicU64 = AtomicU64::new(0);
    pub static CONS_STACK_BYTES: AtomicU64 = AtomicU64::new(0);
    pub static CONS_KEPT_WORDS: AtomicU64 = AtomicU64::new(0);
    /// The deepest single scan seen since the last read — a mean hides the 지옥불 spike,
    /// which is the whole question. A LEVEL that `cons_scan_take` resets.
    pub static CONS_WORDS_MAX: AtomicU64 = AtomicU64::new(0);

    /// One completed scan. Called once per collection with totals accumulated in locals,
    /// so the scan loop itself stays free of atomics.
    pub fn add_cons_scan(words: u64, stack_bytes: u64, kept: u64) {
        CONS_SCANS.fetch_add(1, Ordering::Relaxed);
        CONS_WORDS.fetch_add(words, Ordering::Relaxed);
        CONS_STACK_BYTES.fetch_add(stack_bytes, Ordering::Relaxed);
        CONS_KEPT_WORDS.fetch_add(kept, Ordering::Relaxed);
        CONS_WORDS_MAX.fetch_max(words, Ordering::Relaxed);
    }

    /// `(scans, words, stack_bytes, kept, words_max)` since the last call, and reset.
    pub fn cons_scan_take() -> (u64, u64, u64, u64, u64) {
        (
            CONS_SCANS.swap(0, Ordering::Relaxed),
            CONS_WORDS.swap(0, Ordering::Relaxed),
            CONS_STACK_BYTES.swap(0, Ordering::Relaxed),
            CONS_KEPT_WORDS.swap(0, Ordering::Relaxed),
            CONS_WORDS_MAX.swap(0, Ordering::Relaxed),
        )
    }

    pub fn add_alloc() {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_free() {
        FREES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_list_walk(n: u64) {
        LIST_WALK.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_bucket_header(n: u64) {
        BUCKET_HEADER_BYTES.fetch_add(n, Ordering::Relaxed);
    }

    /// 64 KB guest pages that have actually been committed (see the lazy-`map` note in
    /// `wie_core_arm::engine::arm32_cpu`). Before that change this was pinned at 4096 pages
    /// (256 MB) for the heap alone the moment the allocator initialised; it is the number that
    /// says whether the low-memory-kill fix is doing its job on a device, where `dumpsys
    /// meminfo` is not available to the app itself.
    pub static GUEST_PAGES: AtomicU64 = AtomicU64::new(0);

    /// A LEVEL, not a counter: the engine republishes its own total once per `run` batch,
    /// because the page table is deliberately dependency-free and cannot reach this module.
    pub fn set_guest_pages(pages: u64) {
        GUEST_PAGES.store(pages, Ordering::Relaxed);
    }

    pub fn guest_page_bytes() -> u64 {
        GUEST_PAGES.load(Ordering::Relaxed) * 0x10000
    }

    /// Guest sprite blitter (`0x110fe4`) telemetry. The game rasterises its whole
    /// world through that one function — 26% of all guest instructions — so before
    /// reimplementing it natively we measure what it is actually asked to do:
    /// which pixel formats and transforms turn up, how big the blits are (an SVC
    /// round trip has to be cheaper than the guest loop it replaces), and how often
    /// the safety gate would refuse.
    pub static BLIT_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_HLE: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_PIXELS: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_PIXELS_HLE: AtomicU64 = AtomicU64::new(0);
    /// Indexed by pixel format: [0] = 4bpp (0x0b), [1] = 8bpp (0x0c), [2] = neither.
    pub static BLIT_FMT: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    /// Indexed by transform code 0..=8, with [9] collecting everything out of range.
    pub static BLIT_TRANSFORM: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
    /// Why the gate refused: [0] format, [1] transform, [2] geometry (w/h/stride/
    /// negative origin/row wrap), [3] guest memory not readable, [4] null argument.
    pub static BLIT_REFUSED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

    /// Master switch for the host sprite blitter. Lives here, beside the counters, so
    /// the host binaries can flip it without an export chain through `wie_core_arm`.
    /// ★ It is what makes the A/B frame-hash check possible: the SAME binary and the
    /// SAME deterministic input, run twice, differing only in this bool. Any pixel
    /// difference is then unambiguously the host blitter's fault.
    pub static BLIT_HLE_ENABLED: AtomicBool = AtomicBool::new(true);

    pub fn set_blit_hle(enabled: bool) {
        BLIT_HLE_ENABLED.store(enabled, Ordering::Relaxed);
    }

    /// ★ PER-FRAME GC (2026-09-08, 순서표 #8 계측).
    ///
    /// `wie_midp`'s `Display::handlePaintEvent` calls `Jvm::collect_garbage()` on
    /// EVERY painted frame — a stop-the-world mark&sweep over every object, every
    /// class static and every interned string, cloning a `Box` per visited object.
    /// On a KTF build (hero2) that is worse than it sounds: the JVM is the KTF one,
    /// so field reads during the mark walk resolve through GUEST ARM MEMORY.
    ///
    /// hero2 is the first game in this fork to feel it. Hero3 never does: it is a
    /// WIPI-C title and presents through `wie_wipi_c/src/api/graphics.rs`, which
    /// never enters this path. That is the structural reason none of hero3's
    /// graphics work transfers.
    ///
    /// These exist to ATTRIBUTE the cost rather than guess at it, using the same A/B
    /// idiom as `BLIT_HLE_ENABLED` right above: the same binary and the same
    /// deterministic input, run twice, differing only in this knob. The difference in
    /// wall time IS the GC's share. (Timing it inline is not an option here — these
    /// crates are `no_std`, so there is no host clock.)
    ///
    /// `GC_EVERY` is a PERIOD, not a bool, and that is the whole point:
    ///   1 = collect on every PAINTED FRAME (upstream behaviour, the default)
    ///   N = collect on every Nth painted frame
    ///   0 = never collect ON THE PAINT PATH
    ///
    /// ⛔ **0 does not mean "no collection".** `Display::handlePaintEvent` is only one of
    /// three `collect_garbage` call sites; the other two are `System.gc()` and `Runtime.gc()`,
    /// which the GUEST calls and nothing here throttles. hero2 calls `System.gc()`: measured
    /// 2026-09-10, a 400-frame `--gc 0` run still destroys 6,852 instances. So this knob
    /// bounds what the paint path spends on collection; it does not switch the collector off,
    /// and an A/B across its values is not an A/B against "no GC".
    ///
    /// ⛔ The note that used to stand here — that `--gc 0` crashes around f≈300 from guest
    /// heap exhaustion — was wrong twice over. It did crash, reproducibly, at every `--ms`;
    /// but only 4,144 objects were live at the time, and the real fault was
    /// `java.lang.Thread`'s exit epilogue writing `alive=false` into an instance the collector
    /// had already freed. See `wie_ktf`'s `jvm_support::liveness`. Fixed 2026-09-10 by making
    /// a frame's receiver a GC root; `--gc 0` now runs 4,000 frames clean.
    pub static GC_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static GC_RUNS: AtomicU64 = AtomicU64::new(0);
    pub static GC_EVERY: AtomicU64 = AtomicU64::new(1);

    pub fn set_gc_every(period: u64) {
        GC_EVERY.store(period, Ordering::Relaxed);
    }

    /// ★ heroZero fork (2026-10-28): whether `Display::handlePaintEvent` collects at all. The
    /// host that turns this off collects on its own — after it has taken the painted frame out
    /// of the worker, during the game thread's own sleep — because a collection on the paint path
    /// sits between the frame and the screen: the frame left the worker only after it, and with
    /// `GC_EVERY` 4 every fourth frame was that much late (judder at a steady 20 fps). The probe
    /// keeps it on: one collection per `GC_EVERY` paints, in the paint, deterministic.
    pub static GC_IN_PAINT: AtomicBool = AtomicBool::new(true);

    pub fn set_gc_in_paint(on: bool) {
        GC_IN_PAINT.store(on, Ordering::Relaxed);
    }

    pub fn gc_in_paint() -> bool {
        GC_IN_PAINT.load(Ordering::Relaxed)
    }

    /// Called once per painted frame. `true` = collect now. Counts both the
    /// opportunities (`GC_CALLS`) and the collections actually run (`GC_RUNS`), so a
    /// report can show the period that was really in force rather than the one asked for.
    pub fn gc_due() -> bool {
        let n = GC_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
        let every = GC_EVERY.load(Ordering::Relaxed);
        if every == 0 || n % every != 0 {
            return false;
        }
        GC_RUNS.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn gc_snapshot() -> (u64, u64) {
        (GC_CALLS.load(Ordering::Relaxed), GC_RUNS.load(Ordering::Relaxed))
    }

    /// ★★ USE-AFTER-FREE OF A KTF JAVA INSTANCE (2026-09-10).
    ///
    /// A KTF `JavaClassInstance` is an 8-byte guest record `{ptr_fields, ptr_class}`.
    /// `BucketAllocator::free` chains a freed slot by writing the previous free-list head
    /// into the slot's FIRST word - which is exactly `ptr_fields`. So a freed instance
    /// keeps a perfectly valid `ptr_class` (the class name still resolves!) and carries a
    /// slot INDEX, or `SLOT_SENTINEL`, where its field block used to be. Every later field
    /// access then computes `ptr_fields + offset + 4`, a tiny address nowhere near the heap
    /// at `wie_core_arm::HEAP_BASE`, and the guest read fails.
    ///
    /// That is the whole of the device freeze reported on 2026-09-09: the worker panicked
    /// on `read_generic(...).unwrap()` inside `JavaClassInstance::get_field` while loading a
    /// save. It reads as a hang rather than a crash because only the worker dies and the
    /// last frame stays on screen.
    ///
    /// `Jvm::collect_garbage` is the ONLY code in the tree that frees an instance (the KTF
    /// init table hands the guest `fn_alloc` and no matching free, so the guest cannot do
    /// it), which means a hit here says the collector freed something still in use.
    ///
    /// These counters exist so a device log can say HOW MUCH, not just THAT it happened.
    pub static INSTANCE_UAF: AtomicU64 = AtomicU64::new(0);

    /// The first few hits are printed in full; after that only the counter moves, because the
    /// mark phase re-walks the same dangling edge on EVERY collected frame and an unbounded
    /// log would itself become the performance bug (`__android_log_write` at 124x/s was
    /// already found once - see the `REPORTED` latch in `wie_ktf`'s `oom.rs`).
    pub const UAF_REPORT_LIMIT: u64 = 16;

    /// Counts the hit and answers "should this one be logged?".
    pub fn note_instance_uaf() -> bool {
        INSTANCE_UAF.fetch_add(1, Ordering::Relaxed) < UAF_REPORT_LIMIT
    }

    pub fn instance_uaf_count() -> u64 {
        INSTANCE_UAF.load(Ordering::Relaxed)
    }

    /// ★★ KTF JAVA INTERFACE COST (2026-09-09).
    ///
    /// hero2 is an AOT-compiled Java MIDlet, so every one of its calls into a host
    /// class — `Graphics::fillRect`, `Image::getWidth`, every field read — leaves the
    /// interpreter through `WIPIJBInterface::fn_get_java_method` / `fn_get_field` and
    /// is RESOLVED BY NAME. That resolution is, per call:
    ///
    ///   * read the requested name record out of guest memory, **one byte at a time**
    ///     (`read_null_terminated_string_bytes` -> one `ByteRead` round trip, and hence
    ///     one core-mutex lock, per byte), then split it into three `String`s;
    ///   * walk the class's (or vtable's) NULL-terminated method table, and for EVERY
    ///     entry repeat that same byte-at-a-time read + three-`String` parse just to
    ///     compare two `&str`s.
    ///
    /// A class with ~120 methods therefore costs a few thousand locked guest reads and
    /// several hundred heap allocations **per resolved call**. That is what the 2026-08-22
    /// profile saw as "96 % of samples are at the host/guest boundary" without being able
    /// to say where the host time went (`--profile` is a GUEST PC sampler — it cannot).
    ///
    /// These counters exist to say it in numbers instead:
    ///   `LOOKUPS`  resolutions requested        `HITS`   served from the cache
    ///   `SCAN`     table entries compared       `NAME_*` name records / bytes read
    ///
    /// `SCAN` and `NAME_BYTES` are the ones to watch: they are the work, and the whole
    /// point of the cache is to drive them to ~0 in steady state while `LOOKUPS` is
    /// unchanged (the guest asks exactly as often; we just stop re-deriving the answer).
    pub static JVM_METHOD_LOOKUPS: AtomicU64 = AtomicU64::new(0);
    pub static JVM_METHOD_HITS: AtomicU64 = AtomicU64::new(0);
    pub static JVM_FIELD_LOOKUPS: AtomicU64 = AtomicU64::new(0);
    pub static JVM_FIELD_HITS: AtomicU64 = AtomicU64::new(0);
    /// By-name member resolutions (`JavaClassDefinition::method`/`field`). This is the
    /// number the GUEST forces on us; the cache cannot change it.
    pub static JVM_MEMBERS: AtomicU64 = AtomicU64::new(0);
    /// Table entries actually compared while serving those resolutions. Before the
    /// member memo each of these was a 16-byte guest read plus a name-record parse;
    /// now it is a `&str` comparison in host memory.
    pub static JVM_SCAN: AtomicU64 = AtomicU64::new(0);
    /// Name records actually parsed out of guest memory (cache misses included).
    pub static JVM_NAME_READS: AtomicU64 = AtomicU64::new(0);
    /// Bytes those reads pulled through `ByteRead` — one locked round trip each.
    pub static JVM_NAME_BYTES: AtomicU64 = AtomicU64::new(0);

    pub fn add_jvm_method_lookup(hit: bool) {
        JVM_METHOD_LOOKUPS.fetch_add(1, Ordering::Relaxed);
        if hit {
            JVM_METHOD_HITS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn add_jvm_field_lookup(hit: bool) {
        JVM_FIELD_LOOKUPS.fetch_add(1, Ordering::Relaxed);
        if hit {
            JVM_FIELD_HITS.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Canvas row-blit outcome: [0] rows raw-copied (all-opaque), [1] rows composed in
    /// host memory, [2] draw calls that fell back to the per-pixel path.
    pub static CANVAS_ROWS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

    pub fn add_canvas_row(kind: usize) {
        CANVAS_ROWS[kind].fetch_add(1, Ordering::Relaxed);
    }

    pub fn canvas_rows_snapshot() -> [u64; 3] {
        [
            CANVAS_ROWS[0].load(Ordering::Relaxed),
            CANVAS_ROWS[1].load(Ordering::Relaxed),
            CANVAS_ROWS[2].load(Ordering::Relaxed),
        ]
    }

    pub fn add_jvm_scan(n: u64) {
        JVM_SCAN.fetch_add(n, Ordering::Relaxed);
    }

    /// One by-name member resolution that compared `compared` entries.
    pub fn add_jvm_member(compared: u64) {
        JVM_MEMBERS.fetch_add(1, Ordering::Relaxed);
        JVM_SCAN.fetch_add(compared, Ordering::Relaxed);
    }

    pub fn add_jvm_name_read(bytes: u64) {
        JVM_NAME_READS.fetch_add(1, Ordering::Relaxed);
        JVM_NAME_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }

    /// `(jb method lookups, jb field lookups, member resolutions, entries compared, name reads, name bytes)`
    pub fn jvm_snapshot() -> (u64, u64, u64, u64, u64, u64) {
        (
            JVM_METHOD_LOOKUPS.load(Ordering::Relaxed),
            JVM_FIELD_LOOKUPS.load(Ordering::Relaxed),
            JVM_MEMBERS.load(Ordering::Relaxed),
            JVM_SCAN.load(Ordering::Relaxed),
            JVM_NAME_READS.load(Ordering::Relaxed),
            JVM_NAME_BYTES.load(Ordering::Relaxed),
        )
    }

    /// ★ CANVAS PIXEL TRAFFIC (2026-09-08, 순서표 #8).
    ///
    /// `BLIT_*` above counts the WIPI-C blitter, which hero2 never enters
    /// (`[blit] calls=0` for a whole run — it is a Java MIDlet, so its drawing goes
    /// through MIDP `Graphics` -> `ImageBufferCanvas` instead). These count THAT path,
    /// which is the one #8 is actually about.
    ///
    /// Counted PER CALL with the region's area, never per pixel: an atomic add per
    /// pixel at these volumes would be a large share of what it claims to measure.
    /// `[0]` = calls, `[1]` = pixels.
    ///
    /// Why the split matters: `fill_span` and `put_row_raw` are this fork's own
    /// additions and `JavaImageBuffer` implements NEITHER, so hero2's fills fall back
    /// to the per-pixel default and its blits cannot take the raw-row path (that one
    /// is gated on src AND dst both being 2 bytes/pixel; hero2 is 4/4, so it is
    /// unreachable rather than merely unimplemented). These numbers say whether
    /// implementing them is worth anything before anyone writes the code.
    pub static CANVAS_FILL: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
    pub static CANVAS_DRAW: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
    pub static CANVAS_COPY: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

    pub fn add_canvas_fill(px: u64) {
        CANVAS_FILL[0].fetch_add(1, Ordering::Relaxed);
        CANVAS_FILL[1].fetch_add(px, Ordering::Relaxed);
    }

    pub fn add_canvas_draw(px: u64) {
        CANVAS_DRAW[0].fetch_add(1, Ordering::Relaxed);
        CANVAS_DRAW[1].fetch_add(px, Ordering::Relaxed);
    }

    pub fn add_canvas_copy(px: u64) {
        CANVAS_COPY[0].fetch_add(1, Ordering::Relaxed);
        CANVAS_COPY[1].fetch_add(px, Ordering::Relaxed);
    }

    pub fn canvas_snapshot() -> [[u64; 2]; 3] {
        let g = |a: &[AtomicU64; 2]| [a[0].load(Ordering::Relaxed), a[1].load(Ordering::Relaxed)];
        [g(&CANVAS_FILL), g(&CANVAS_DRAW), g(&CANVAS_COPY)]
    }

    pub fn blit_snapshot() -> ([u64; 4], [u64; 3], [u64; 10], [u64; 5]) {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        (
            [load(&BLIT_CALLS), load(&BLIT_HLE), load(&BLIT_PIXELS), load(&BLIT_PIXELS_HLE)],
            core::array::from_fn(|i| load(&BLIT_FMT[i])),
            core::array::from_fn(|i| load(&BLIT_TRANSFORM[i])),
            core::array::from_fn(|i| load(&BLIT_REFUSED[i])),
        )
    }

    /// `tintRect` — the per-pixel LUT colour transform a CRITICAL HIT runs over the
    /// screen (LGT `0x2b8f8`, see `wie_core_arm::binary_patches::tint565`).
    ///
    /// Kept apart from the `BLIT_*` counters on purpose: those mean "a SPRITE the host
    /// drew instead of interpreting", and this function draws no sprite. Folding it in
    /// would quietly change what the `blit N/M` ratio in the device log means.
    ///
    /// Reading these: `calls` climbing only while a critical flash is on screen is the
    /// expected shape, and `hle` short of `calls` is the gate declining — which is the
    /// first thing to check if the crit hitch ever comes back.
    pub static TINT_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static TINT_HLE: AtomicU64 = AtomicU64::new(0);
    pub static TINT_PIXELS: AtomicU64 = AtomicU64::new(0);

    pub fn add_tint(pixels: u64, hosted: bool) {
        TINT_CALLS.fetch_add(1, Ordering::Relaxed);
        if hosted {
            TINT_HLE.fetch_add(1, Ordering::Relaxed);
            TINT_PIXELS.fetch_add(pixels, Ordering::Relaxed);
        }
    }

    /// `(calls, hosted, hosted pixels)`.
    pub fn tint_snapshot() -> (u64, u64, u64) {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        (load(&TINT_CALLS), load(&TINT_HLE), load(&TINT_PIXELS))
    }

    /// Guest SOUND-PATH telemetry (2026-08-09). The player reports that BGM and the
    /// critical-hit SFX cost frames, and the sound path runs **on the emulator thread**:
    /// `MC_mdaClipPutData` copies the file out of guest memory and `MC_mdaPlay` used to
    /// re-run the whole SMAF parse (chunk walk + event vector + sort) every single time a
    /// clip was played. These say how often that happens and how much of it the parse
    /// cache now absorbs.
    ///
    /// `PARSES` is what actually costs: a session where `PARSES` stays at the number of
    /// distinct sound files (33 in Hero3) while `PLAYS` climbs into the thousands is the
    /// cache working. `PARSES` tracking `PLAYS` means it is not.
    pub static SMAF_LOADS: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PLAYS: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PARSES: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PARSE_BYTES: AtomicU64 = AtomicU64::new(0);
    /// LEVELS (republished, not accumulated): parse-cache entries and live audio handles.
    /// The handle count is the one that used to grow without bound — every load minted a
    /// handle that nothing ever removed, and each one held its own copy of the file.
    pub static SMAF_CACHE_ENTRIES: AtomicU64 = AtomicU64::new(0);
    pub static AUDIO_HANDLES: AtomicU64 = AtomicU64::new(0);
    /// MIDI/PCM events a `SmafPlayer` skipped because the clock ran away from it — the
    /// app pauses the emulator worker while backgrounded, so on every foreground return
    /// a playing song is behind by however long the screen was off. Without the guard
    /// that whole backlog is dispatched in ONE poll (see `SmafPlayer::play`).
    pub static SMAF_CATCHUP: AtomicU64 = AtomicU64::new(0);

    pub fn add_smaf_load() {
        SMAF_LOADS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_smaf_play() {
        SMAF_PLAYS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_smaf_parse(bytes: u64) {
        SMAF_PARSES.fetch_add(1, Ordering::Relaxed);
        SMAF_PARSE_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_smaf_catchup() {
        SMAF_CATCHUP.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_smaf_cache_entries(n: u64) {
        SMAF_CACHE_ENTRIES.store(n, Ordering::Relaxed);
    }

    pub fn set_audio_handles(n: u64) {
        AUDIO_HANDLES.store(n, Ordering::Relaxed);
    }

    /// `(loads, plays, parses, parse_bytes, cache_entries, live_handles, catchups)`
    pub fn audio_snapshot() -> [u64; 7] {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        [
            load(&SMAF_LOADS),
            load(&SMAF_PLAYS),
            load(&SMAF_PARSES),
            load(&SMAF_PARSE_BYTES),
            load(&SMAF_CACHE_ENTRIES),
            load(&AUDIO_HANDLES),
            load(&SMAF_CATCHUP),
        ]
    }

    pub fn heap_snapshot() -> (u64, u64, u64, u64) {
        (
            ALLOCS.load(Ordering::Relaxed),
            FREES.load(Ordering::Relaxed),
            LIST_WALK.load(Ordering::Relaxed),
            BUCKET_HEADER_BYTES.load(Ordering::Relaxed),
        )
    }

    pub fn add_instructions(n: u64) {
        GUEST_INSTRUCTIONS.fetch_add(n, Ordering::Relaxed);
        RUN_BATCHES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_frame() {
        GUEST_FRAMES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frames() -> u64 {
        GUEST_FRAMES.load(Ordering::Relaxed)
    }

    pub fn add_svc() {
        SVC_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    /// ★ FRAME-PERIOD ACCOUNTING (2026-08-13, `MIGRATION_LGT.md` §2.24). The panel
    /// screens run at ~7 fps with the host thread 78 % IDLE, so the limiter is not work
    /// — it is how long the guest is left alone between frames. Two things decide that,
    /// and only measurement can say which: what the game ASKS for
    /// (`MC_knlSetTimer`'s timeout) and how long the event pump actually SLEEPS before
    /// looking again. `TIMER_MS_MIN` is the interesting one: the game's real frame
    /// interval. If min is ~20 and the pump's average nap is ~16, the loss is ours.
    pub static TIMER_SETS: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_MS_MIN: AtomicU64 = AtomicU64::new(u64::MAX);
    pub static TIMER_MS_MAX: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_MS_SUM: AtomicU64 = AtomicU64::new(0);
    /// Times the event pump found nothing due and slept, and the milliseconds it asked
    /// for in total.
    pub static PUMP_NAPS: AtomicU64 = AtomicU64::new(0);
    pub static PUMP_NAP_MS: AtomicU64 = AtomicU64::new(0);

    pub fn note_timer_set(timeout_ms: u64) {
        TIMER_SETS.fetch_add(1, Ordering::Relaxed);
        TIMER_MS_SUM.fetch_add(timeout_ms, Ordering::Relaxed);
        TIMER_MS_MIN.fetch_min(timeout_ms, Ordering::Relaxed);
        TIMER_MS_MAX.fetch_max(timeout_ms, Ordering::Relaxed);
    }

    /// ★ TIMER CANCELLATION (2026-08-14, PLAY_FEEDBACK_FIXES.md §41 item 7). Until this
    /// round `MC_knlUnsetTimer` was a do-nothing stub and `MC_knlSetTimer` PUSHED rather
    /// than replaced, so every screen the guest ever visited left one more live frame
    /// timer behind — the game sped up by one whole multiple per network round trip.
    /// `UNSETS` counts the guest's cancel calls; `STALE_DROPS` counts pending shots the
    /// event pump actually discarded because a re-arm or cancel outdated them. On a
    /// healthy build both track each other and `guest set N/s` stays flat across a
    /// warehouse visit; before this fix the second number did not exist and N/s grew.
    pub static TIMER_UNSETS: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_STALE_DROPS: AtomicU64 = AtomicU64::new(0);

    pub fn note_timer_unset() {
        TIMER_UNSETS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_timer_stale_drop() {
        TIMER_STALE_DROPS.fetch_add(1, Ordering::Relaxed);
    }

    /// `(unsets, stale_drops)` since the last call; resets.
    pub fn timer_cancel_take() -> (u64, u64) {
        (TIMER_UNSETS.swap(0, Ordering::Relaxed), TIMER_STALE_DROPS.swap(0, Ordering::Relaxed))
    }

    pub fn note_pump_nap(ms: u64) {
        PUMP_NAPS.fetch_add(1, Ordering::Relaxed);
        PUMP_NAP_MS.fetch_add(ms, Ordering::Relaxed);
    }

    /// ★ GUEST FRAME-TIMER CEILING (2026-08-14). §2.24 fixed the host's own two
    /// roundings and left one term standing: in its modal/SLOT states Hero3 asks to be
    /// woken every **244 ms** (the field asks 50), so those screens run at ~7 fps by the
    /// game's own design and no amount of host work changes it. Device measurement the
    /// same day pinned everything else — warehouse page turns are a single round trip at
    /// 24~49 ms and `recv polls/s` is 0 — so this constant is the whole remaining
    /// complaint.
    ///
    /// `0` disables the clamp (ship the guest's number unchanged). Any other value is
    /// the longest wake-up interval the host will honour — but ONLY for requests at or
    /// above [`TIMER_CLAMP_FLOOR_MS`], which is what keeps the player's own speed
    /// setting out of it. See that constant; it is not an optional refinement.
    ///
    /// ⚠ This is NOT free: a guest that counts frame ticks rather than wall-clock will
    /// count them faster. That is why it is a runtime switch rather than a constant —
    /// see `wie_android`'s `frame_timer_cap_ms` config key and the A/B on device.
    pub static TIMER_CLAMP_MS: AtomicU64 = AtomicU64::new(0);
    /// ★ THE FLOOR IS THE WHOLE SAFETY ARGUMENT (2026-08-14).
    ///
    /// The first version of this clamp had no floor and shipped as vc60. The player
    /// reported "네트워크에 접속한 이후로 게임이 너무 빨라졌다", and device measurement
    /// found the reason immediately: **Hero3's 게임속도 option IS this timer.**
    ///
    /// | 게임속도 | guest asks | (formula `110 - 10*speed`) |
    /// |---|---|---|
    /// | 1  | **100 ms** | slowest |
    /// | 6  | 50 ms | the shipped default |
    /// | 10 | **10 ms** | fastest |
    ///
    /// So a flat 50 ms cap silently rewrote every setting from 1 to 5 into 6 — at speed
    /// 1 the game ran at exactly double the chosen speed (`capped 100% to 50 ms`,
    /// measured in the field). 34차's note that "게임속도 설정은 게스트 로직 진행량만
    /// 바꾼다 (호스트에 speed 개념 없음)" is refuted by that measurement.
    ///
    /// The modal/network screens ask for **244 ms**, which is far outside the 10..100
    /// band the option can produce. Clamping only at or above this floor therefore
    /// keeps the entire speed setting untouched while still fixing the warehouse.
    pub const TIMER_CLAMP_FLOOR_MS: u64 = 200;
    /// How many `set_timer` calls the clamp actually shortened, since the last take.
    pub static TIMER_CLAMPED: AtomicU64 = AtomicU64::new(0);

    pub fn set_timer_clamp_ms(ms: u64) {
        TIMER_CLAMP_MS.store(ms, Ordering::Relaxed);
    }

    /// The interval the host will actually wait for, given what the guest asked for.
    /// Counts a clamp when it shortens one.
    ///
    /// Requests below [`TIMER_CLAMP_FLOOR_MS`] pass through untouched even when they are
    /// longer than the cap — those are the player's 게임속도 setting and are not ours to
    /// override.
    pub fn clamp_timer_ms(requested: u64) -> u64 {
        let cap = TIMER_CLAMP_MS.load(Ordering::Relaxed);
        if cap == 0 || requested < TIMER_CLAMP_FLOOR_MS || requested <= cap {
            return requested;
        }
        TIMER_CLAMPED.fetch_add(1, Ordering::Relaxed);
        cap
    }

    /// `(clamped_calls, cap_ms)` — clamped count resets, the cap does not.
    pub fn timer_clamp_take() -> (u64, u64) {
        (TIMER_CLAMPED.swap(0, Ordering::Relaxed), TIMER_CLAMP_MS.load(Ordering::Relaxed))
    }

    /// `(sets, min, max, mean, naps, nap_ms)` since the last call; resets.
    pub fn frame_timing_take() -> (u64, u64, u64, u64, u64, u64) {
        let sets = TIMER_SETS.swap(0, Ordering::Relaxed);
        let sum = TIMER_MS_SUM.swap(0, Ordering::Relaxed);
        let min = TIMER_MS_MIN.swap(u64::MAX, Ordering::Relaxed);
        (
            sets,
            if min == u64::MAX { 0 } else { min },
            TIMER_MS_MAX.swap(0, Ordering::Relaxed),
            sum / sets.max(1),
            PUMP_NAPS.swap(0, Ordering::Relaxed),
            PUMP_NAP_MS.swap(0, Ordering::Relaxed),
        )
    }

    /// Two more relaxed increments per SVC, on top of [`add_svc`]. See [`SVC_BY_ID`].
    pub fn add_svc_detail(category: u32, id: u32) {
        SVC_BY_CATEGORY[category as usize & 7].fetch_add(1, Ordering::Relaxed);
        SVC_BY_ID[id as usize & 2047].fetch_add(1, Ordering::Relaxed);
    }

    /// The hottest SVC ids since the last call, and the per-category totals, and
    /// RESET both — so the caller reports an interval rate rather than a lifetime
    /// average (the gauge that prints this makes the same choice for the same
    /// reason: a running average mixes the title screen with real play).
    pub fn svc_mix_take(top: usize) -> (Vec<u64>, Vec<(u32, u64)>) {
        let cats = SVC_BY_CATEGORY.iter().map(|c| c.swap(0, Ordering::Relaxed)).collect::<Vec<_>>();
        let mut hot = SVC_BY_ID
            .iter()
            .enumerate()
            .filter_map(|(id, c)| {
                let n = c.swap(0, Ordering::Relaxed);
                if n > 0 { Some((id as u32, n)) } else { None }
            })
            .collect::<Vec<_>>();
        hot.sort_unstable_by_key(|(_, n)| core::cmp::Reverse(*n));
        hot.truncate(top);
        (cats, hot)
    }

    pub fn snapshot() -> (u64, u64, u64) {
        (
            GUEST_INSTRUCTIONS.load(Ordering::Relaxed),
            SVC_CALLS.load(Ordering::Relaxed),
            RUN_BATCHES.load(Ordering::Relaxed),
        )
    }
}

pub trait ByteRead {
    fn read_bytes(&self, address: u32, result: &mut [u8]) -> Result<usize>;
}

pub trait ByteWrite {
    fn write_bytes(&mut self, address: u32, data: &[u8]) -> Result<()>;
}

pub fn read_generic<T, R>(reader: &R, address: u32) -> Result<T>
where
    T: Copy + AnyBitPattern + NoUninit,
    R: ?Sized + ByteRead,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let mut destination = MaybeUninit::<T>::uninit();
    let destination_bytes = unsafe { from_raw_parts_mut(destination.as_mut_ptr().cast::<u8>(), size_of::<T>()) };
    let read = reader.read_bytes(address, destination_bytes)?;
    if read != destination_bytes.len() {
        return Err(WieError::FatalError(format!(
            "Short read at {address:#x}: expected {}, got {read}",
            destination_bytes.len()
        )));
    }

    Ok(unsafe { destination.assume_init() })
}

pub fn read_null_terminated_string_bytes<R>(reader: &R, address: u32) -> Result<Vec<u8>>
where
    R: ?Sized + ByteRead,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let mut result = Vec::with_capacity(20);
    let mut cursor = address;
    let mut byte = [0; 1];
    loop {
        let read = reader.read_bytes(cursor, &mut byte)?;
        if read != 1 {
            return Err(WieError::FatalError(format!("Short read at {cursor:#x}: expected 1, got {read}")));
        }

        if byte[0] == 0 {
            break;
        }

        result.push(byte[0]);
        cursor += 1;
    }

    Ok(result)
}

pub fn write_null_terminated_string_bytes<W>(writer: &mut W, address: u32, bytes: &[u8]) -> Result<()>
where
    W: ?Sized + ByteWrite,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    // TODO temp
    writer.write_bytes(address, bytes)?;
    writer.write_bytes(address + bytes.len() as u32, &[0])?;

    Ok(())
}

pub fn write_generic<W, T>(writer: &mut W, address: u32, data: T) -> Result<()>
where
    W: ?Sized + ByteWrite,
    T: NoUninit,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let data_slice = bytes_of(&data);

    writer.write_bytes(address, data_slice)
}

pub fn read_null_terminated_table<R>(reader: &R, base_address: u32) -> Result<Vec<u32>>
where
    R: ?Sized + ByteRead,
{
    if base_address == 0 {
        return Err(WieError::InvalidMemoryAccess(base_address));
    }

    let mut cursor = base_address;
    let mut result = Vec::new();
    loop {
        let item: u32 = read_generic(reader, cursor)?;
        if item == 0 {
            break;
        }
        result.push(item);

        cursor += 4;
    }

    Ok(result)
}

pub fn write_null_terminated_table<W>(writer: &mut W, base_address: u32, items: &[u32]) -> Result<()>
where
    W: ?Sized + ByteWrite,
{
    if base_address == 0 {
        return Err(WieError::InvalidMemoryAccess(base_address));
    }

    let mut cursor = base_address;
    for &item in items {
        write_generic(writer, cursor, item)?;

        cursor += 4;
    }
    write_generic(writer, cursor, 0u32)
}

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T> AsAny for T
where
    T: Any,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    struct StrictMemory {
        memory: Vec<u8>,
    }

    impl ByteRead for StrictMemory {
        fn read_bytes(&self, address: u32, result: &mut [u8]) -> Result<usize> {
            let address = address as usize;
            let end = address + result.len();
            if end > self.memory.len() {
                return Err(WieError::InvalidMemoryAccess(address as u32));
            }

            result.copy_from_slice(&self.memory[address..end]);

            Ok(result.len())
        }
    }

    #[test]
    fn read_generic_reads_into_initialized_storage() {
        let memory = StrictMemory {
            memory: vec![0, 0x78, 0x56, 0x34, 0x12],
        };

        let value: u32 = read_generic(&memory, 1).unwrap();

        assert_eq!(value, 0x1234_5678);
    }

    #[test]
    fn read_null_terminated_string_handles_four_byte_boundaries() {
        let memory = StrictMemory {
            memory: vec![0, b't', b'e', b's', b't', 0],
        };

        let value = read_null_terminated_string_bytes(&memory, 1).unwrap();

        assert_eq!(value, b"test");
    }

    /// The guest frame-timer ceiling (`stats::clamp_timer_ms`). It must be OFF unless a
    /// front end asks for it — every non-Hero3 target shares this code path — and it
    /// must never LENGTHEN an interval, only shorten one.
    #[test]
    fn timer_clamp_is_off_by_default_and_only_shortens() {
        use super::stats;

        stats::set_timer_clamp_ms(0);
        let _ = stats::timer_clamp_take();
        for ms in [1, 20, 50, 244, 1000] {
            assert_eq!(stats::clamp_timer_ms(ms), ms, "a zero cap must pass {ms} through");
        }
        assert_eq!(stats::timer_clamp_take().0, 0, "nothing was clamped");

        stats::set_timer_clamp_ms(50);
        assert_eq!(stats::clamp_timer_ms(244), 50, "the modal interval is the point of this");
        assert_eq!(stats::clamp_timer_ms(50), 50, "a request AT the cap is untouched");
        assert_eq!(stats::clamp_timer_ms(20), 20, "a faster request is never slowed down");
        let (clamped, cap) = stats::timer_clamp_take();
        assert_eq!((clamped, cap), (1, 50), "only the 244 counts as a clamp");
        assert_eq!(stats::timer_clamp_take().0, 0, "the counter resets, the cap does not");

        stats::set_timer_clamp_ms(0);
    }

    /// ★ The regression vc60 shipped: a flat cap rewrote the player's 게임속도.
    /// Every interval the option can produce (`110 - 10*speed`, speed 1..10, measured on
    /// device 2026-08-14) must survive the clamp untouched, and the modal 244 must not.
    #[test]
    fn timer_clamp_never_touches_the_game_speed_setting() {
        use super::stats;

        stats::set_timer_clamp_ms(50);
        let _ = stats::timer_clamp_take();
        for speed in 1..=10u64 {
            let ms = 110 - 10 * speed; // 100 (slowest) .. 10 (fastest)
            assert!(ms < stats::TIMER_CLAMP_FLOOR_MS, "speed {speed} = {ms} ms must sit below the floor");
            assert_eq!(
                stats::clamp_timer_ms(ms),
                ms,
                "게임속도 {speed} ({ms} ms) must reach the guest unchanged — vc60 halved 1..5"
            );
        }
        assert_eq!(stats::timer_clamp_take().0, 0, "no speed setting may count as a clamp");

        // …and the thing the clamp exists for is still clamped.
        assert_eq!(stats::clamp_timer_ms(244), 50, "the modal 244 ms is above the floor");
        assert_eq!(stats::timer_clamp_take().0, 1);

        // The floor is a floor, not a window: anything between it and the cap still gets cut.
        assert_eq!(stats::clamp_timer_ms(stats::TIMER_CLAMP_FLOOR_MS), 50);
        // One millisecond below it is the player's territory and is left alone.
        assert_eq!(stats::clamp_timer_ms(stats::TIMER_CLAMP_FLOOR_MS - 1), stats::TIMER_CLAMP_FLOOR_MS - 1);

        stats::set_timer_clamp_ms(0);
    }
}
