#![no_std]
extern crate alloc;

mod audio_sink;
pub mod canvas;
mod database;
mod executor;
mod guest_call;
mod platform;
mod screen;
mod system;
mod task;
mod task_runner;
mod time;

pub use self::{
    audio_sink::{AudioSink, volume_gain},
    database::{Database, DatabaseRepository, RecordId},
    executor::{AsyncCallable, AsyncCallableResult},
    guest_call::{GuestCallResult, JavaArg, JavaReceiver},
    platform::{Filesystem, Network, Platform},
    screen::Screen,
    system::{
        Event, FilesystemOverlay, KeyCode, System, TimerKey,
        ma2::{MA2_CHANNELS, Ma2Event, Ma2Song, Ma2Voice, parse_ma2},
    },
    task_runner::{DefaultTaskRunner, TaskRunner},
    time::Instant,
};

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use wie_util::{Result, WieError};

/// Host observability: the last framebuffer the guest copied to the screen that
/// was NOT the screen itself, packed as `(handle << 32) | (w << 16) | h`.
///
/// Written by the WIPI-C `MC_grpCopyFrameBuffer` implementation, read by hosts
/// that mirror guest-rendered images (wie_android's HUD uses it to locate the
/// game's per-map minimap buffer, which is otherwise indistinguishable from the
/// stale buffers of previously visited maps). Zero = nothing seen yet.
pub static LAST_OFFSCREEN_COPY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Record an offscreen -> screen blit for [`LAST_OFFSCREEN_COPY`].
pub fn note_offscreen_copy(handle: u32, w: u32, h: u32) {
    let packed = (u64::from(handle) << 32) | (u64::from(w & 0xffff) << 16) | u64::from(h & 0xffff);
    LAST_OFFSCREEN_COPY.store(packed, core::sync::atomic::Ordering::Relaxed);
}

/// `(handle, w, h)` of the last offscreen -> screen blit, or `None`.
pub fn last_offscreen_copy() -> Option<(u32, u32, u32)> {
    let v = LAST_OFFSCREEN_COPY.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        return None;
    }
    Some(((v >> 32) as u32, ((v >> 16) & 0xffff) as u32, (v & 0xffff) as u32))
}

pub trait Emulator {
    fn handle_event(&mut self, event: Event);
    fn tick(&mut self) -> Result<()>;

    /// Milliseconds until the guest has work again — `Some(0)` = right now, `None` =
    /// unknown (the default), which callers must read as "use your own pacing".
    /// See [`System::next_wakeup_ms`]; the host worker loop uses it so it does not
    /// sleep past the guest's next frame timer.
    fn next_wakeup_ms(&self) -> Option<u64> {
        None
    }

    /// Read `len` bytes of guest memory at `address` (host-side introspection,
    /// e.g. reading live character state for overlay HUD). Default = unsupported
    /// (empty); backends that own an ARM core override it. Never faults the guest.
    fn read_memory(&self, _address: u32, _len: usize) -> Vec<u8> {
        Vec::new()
    }

    /// Write `data` into guest memory at `address` (host-side poke, e.g. forcing a
    /// game flag the host UI owns). Returns `true` on success. Default =
    /// unsupported (`false`); backends that own an ARM core override it.
    ///
    /// Only for bytes whose full writer set is known — a host write races the
    /// guest, so the caller owns the contract that nothing else writes them at the
    /// same time.
    fn write_memory(&mut self, _address: u32, _data: &[u8]) -> bool {
        false
    }

    /// Schedule a call to the guest function at `address` with `args` in r0..r3
    /// (spill to stack beyond 4). The call runs on its OWN cooperative task /
    /// ARM thread context on the next `tick()`, so it does not clobber the parked
    /// main-loop task — this is exactly how the runtime invokes WIPI callbacks.
    /// Host-initiated (e.g. an item-use HUD button driving the game's own
    /// `useItemEffect(tab,slot)` routine). `address` must include the Thumb bit
    /// where the target is Thumb. Default = unsupported (no-op); backends that own
    /// an ARM core override it. Only inject self-contained, non-blocking guest
    /// functions at a valid game state — the caller owns that contract.
    fn call_guest_function(&mut self, _address: u32, _args: &[u32]) {}

    /// Like [`Emulator::call_guest_function`], but the guest's return value (`r0`) is
    /// delivered into `out` when the call completes, so the host can CHAIN calls whose
    /// arguments come from an earlier call's result.
    ///
    /// Needed because the call is queued, not inline: the host issues it, returns, and
    /// picks the result up on a later poll. The save path is the motivating case —
    /// `serialiseSave(&len)` hands back a heap buffer that the very next call has to be
    /// given, and there is no way to express that without seeing the value.
    ///
    /// Same contract as `call_guest_function` otherwise: `address` must carry the Thumb
    /// bit where the target is Thumb, the target must be self-contained and
    /// non-blocking, and the caller owns "is the guest in a state where this is legal".
    ///
    /// Default = unsupported, and it FAILS the result rather than leaving it pending, so
    /// a host state machine on a backend without an ARM core does not hang.
    fn call_guest_function_out(&mut self, _address: u32, _args: &[u32], out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }

    /// Invoke a Java method on the guest through the JVM **by name** — `receiver` says
    /// whose method (a singleton held by a static field, an object handle the host holds,
    /// or a class for a `static` method), `method` + `descriptor` name it, and `args` are
    /// its arguments in order. The raw return word — a primitive's value, an object's
    /// handle, `0` for `void` — lands in `out`.
    ///
    /// ★ Why a separate entry point instead of [`Emulator::call_guest_function`]
    /// (2026-09-19). On a backend whose guest is AOT-compiled Java (hero2/KTF), the
    /// method body is NOT callable as a plain `bl` target: the compiler's calling
    /// convention passes a late-resolution method cell in `r0` and the receiver in
    /// `r1`, and the body is reached through one of three dispatch helpers rather
    /// than directly. Handing a raw body address to `call_guest_function` therefore
    /// enters the method with the wrong registers — the failure mode is not a clean
    /// error but a routine that runs on a garbage `this`.
    ///
    /// Going through the JVM sidesteps all of it: the runtime already knows how to
    /// call the method, because that is how the guest calls it. It also means the
    /// host names what it wants (`ag.l()V`, `bq.a(Lch;I)Z`) instead of an address, so
    /// the call survives a rebuild of the guest image.
    ///
    /// ★ 2026-09-21: generalised from the no-argument singleton call so the host can
    /// hand the game's own container code a record and a count. Same contract for the
    /// arguments as for the receiver: a [`JavaArg::Object`] is a handle the host READ,
    /// and the backend checks it still names a live instance before the call is made —
    /// an argument that fails that test fails the call, it does not enter the guest.
    ///
    /// ★★ An OBJECT the call returns is PINNED (`Jvm::pin_object`) before `out` is
    /// completed, and stays pinned until [`Emulator::release_java_object`] is called with
    /// its handle. Without that the host cannot use what it was given: the spawned task's
    /// frame is the object's only reference, the frame is gone by the time `out` reads
    /// `Ok`, and the guest collects on its own schedule (hero2 calls `System.gc()` itself)
    /// — measured 2026-09-21: a record fresh from the game's own factory was dead by the
    /// time the very next tick's call tried to place it, on 2 runs of 3. The host owns the
    /// release; a handle it forgets is a leak, not a crash.
    ///
    /// Same queuing contract as the other host-initiated calls: it runs on its own
    /// cooperative task on a later tick, so `out` is how the caller learns it
    /// finished. Default = unsupported, and it FAILS rather than leaving `out` pending.
    fn call_java_method(
        &mut self,
        _receiver: JavaReceiver<'_>,
        _method: &str,
        _descriptor: &str,
        _args: &[JavaArg],
        out: alloc::sync::Arc<GuestCallResult>,
    ) {
        out.fail();
    }

    /// Construct a Java object on the guest: `new <class>(<args>)` through the JVM by name
    /// — `init_descriptor` names the constructor (`(B)V`), `args` its arguments. The new
    /// object's handle lands in `out`, PINNED exactly like an object a call returns (see
    /// [`Emulator::call_java_method`]); the host releases it once something in the guest
    /// references it, or keeps it pinned for as long as it owns the object. Default =
    /// unsupported, FAILS `out`.
    fn new_java_object(&mut self, _class: &str, _init_descriptor: &str, _args: &[JavaArg], out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }

    /// Construct a `byte[]` on the guest holding `bytes` — for handing the game's own
    /// deserialisers (`bq.a([B)V`) a blob the host kept. PINNED like every object the
    /// host is handed; the host releases it once the call that consumes it has returned.
    /// Default = unsupported, FAILS `out`.
    fn new_java_byte_array(&mut self, _bytes: &[u8], out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }

    /// Construct a `String[]` on the guest holding a fresh `java/lang/String` for each of
    /// `values`, in order — for handing the game a table LONGER than the one its own
    /// `<clinit>` built. The motivating case (hero2, 2026-09-27) is the save-file names:
    /// `ag.<clinit>` builds `{"/g1","/g2","/g3"}` and `{"/n1","/n2","/n3"}` from string
    /// constants, and there is no `/g4` in the image to build a fourth from. PINNED like
    /// every object the host is handed; the host releases it once a static holds it.
    /// Default = unsupported, FAILS `out`.
    fn new_java_string_array(&mut self, _values: &[&str], out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }

    /// Make sure GAME class `class` is loaded and INITIALIZED: its `<clinit>` runs now if
    /// it has not run yet, and never again — the JVM marks the class initialized, so the
    /// guest's own first touch of it later finds it so. `out` completes with 0.
    ///
    /// ★ Why (hero2, 2026-09-27): `ag.<clinit>` and the title's first read of what it
    /// built run in ONE tick (the key that leaves the logo triggers both), so there is no
    /// host poll between them. Running the initializer early — it is pure static data and
    /// the `ag` singleton's `new` — opens that window: the host can rewrite the statics it
    /// built before anything has read them. Default = unsupported, FAILS `out`.
    fn init_java_class(&mut self, _class: &str, out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }

    /// Pin a live object the host READ (a handle out of guest memory) so the collector
    /// keeps it while the host holds the only reference to it — the mirror of the pin a
    /// returned object arrives with. `false` = not a live instance, nothing pinned.
    ///
    /// ★ Why (2026-09-21, S9): the ACCOUNT window's title swap pointed the gm table's row
    /// 57 at row 19 and kept row 57's OLD `byte[]` handle to put back later. Nothing in
    /// the guest referenced that `byte[]` any more, the collector freed it, and putting
    /// the handle back planted a dead reference in a table the collector walks every
    /// frame — `USE AFTER FREE … in gc mark` on every collection until the painter
    /// dereferenced it and killed the worker. A handle the host means to write back
    /// later is a reference the host holds, and the collector has to be told so.
    fn pin_java_object(&mut self, _handle: u32) -> bool {
        false
    }

    /// Unpin an object a [`Emulator::call_java_method`] returned, once the host is done
    /// with the handle. After this the collector treats it like any other object: alive
    /// while something in the guest references it, garbage otherwise. Default = no-op.
    fn release_java_object(&mut self, _handle: u32) {}

    /// Reserve `len` bytes of guest memory for the HOST — a block the guest's own
    /// allocator hands out and nothing ever frees, so its address is stable for the life
    /// of the process. For host-written guest code and data that a guest call site is
    /// pointed at (a trampoline, its literals, a context word) — things that must live
    /// at a fixed address inside the guest's address space and that no Java object is.
    /// The collector never sees it: it is not an object, and the raw block is not in the
    /// JVM's object set. Default = unsupported (`None`).
    fn alloc_guest(&mut self, _len: u32) -> Option<u32> {
        None
    }

    /// ★ 2026-10-21: hold the GUEST's own tasks — the game's threads — while the host's
    /// tasks (the music player) keep running under [`Emulator::tick`]. The hero2 world map
    /// holds the field the way a menu window does; a menu keeps the music going, and a
    /// tick stop silenced it. Guest memory and the event queue are untouched: keys queued
    /// while parked are read when the game runs again. Default = no-op.
    fn park_guest(&mut self, _on: bool) {}

    /// Invoke a no-argument `void` INSTANCE method on the object a static field holds —
    /// [`Emulator::call_java_method`] with a [`JavaReceiver::Static`] receiver, `()V`
    /// and no arguments. Kept as the spelling of the one-call save (`ag.l()V`).
    fn call_java_singleton_void(&mut self, class: &str, field: &str, descriptor: &str, method: &str, out: alloc::sync::Arc<GuestCallResult>) {
        self.call_java_method(JavaReceiver::Static { class, field, descriptor }, method, "()V", &[], out);
    }
}

pub struct ProfileSample {
    /// Leaf-first call stack: [pc, lr, lr_prev, ...].
    pub stack: Vec<u32>,
    pub count: u64,
}

/// Called periodically during execution with a batch of samples that the
/// profiler accumulated since the previous flush. The callback also fires once
/// more when the runtime shuts down to drain anything still in the buffer.
pub type ProfileCallback = Box<dyn FnMut(Vec<ProfileSample>) + Send + Sync>;

pub struct Options {
    pub enable_gdbserver: bool,
    pub profile: Option<ProfileCallback>,
}

pub fn extract_zip(zip: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    extern crate std; // XXX

    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(Cursor::new(zip)).map_err(|x| WieError::FatalError(format!("Invalid zip archive: {x}")))?;

    (0..archive.len())
        .filter_map(|x| {
            let mut file = match archive.by_index(x) {
                Ok(file) => file,
                Err(err) => return Some(Err(WieError::FatalError(format!("Failed to read zip entry {x}: {err}")))),
            };
            if !file.is_file() {
                return None;
            }

            let mut data = Vec::new();
            if let Err(err) = file.read_to_end(&mut data) {
                return Some(Err(WieError::FatalError(format!("Failed to read zip entry {}: {err}", file.name()))));
            }

            Some(Ok((file.name().to_string(), data)))
        })
        .collect::<Result<_>>()
}
