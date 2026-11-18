use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Where a host-initiated guest call drops its return value.
///
/// [`crate::Emulator::call_guest_function`] cannot hand a value back directly: the
/// call does not run inline, it is queued as a cooperative task and executes on a
/// later `System::tick` (the same mechanism WIPI callbacks use), so by the time the
/// guest's `r0` exists the caller has long returned. A host that needs to CHAIN calls
/// — "serialise the save, then write the buffer that returned" — therefore needs a
/// place for the result to land, plus a way to know it has landed.
///
/// Shared by `Arc` between the host thread and the spawned task. Deliberately tiny and
/// lock-free: `wie_backend` is `no_std` and the task may run on any executor turn.
///
/// The three states are distinguished (rather than "value or nothing") because a host
/// state machine must be able to tell "not yet" from "it failed" — treating a failure
/// as "not yet" would hang the machine forever, and treating "not yet" as a failure
/// would abandon a call that was about to succeed.
#[derive(Debug)]
pub struct GuestCallResult {
    value: AtomicU32,
    state: AtomicU8,
}

const PENDING: u8 = 0;
const OK: u8 = 1;
const FAILED: u8 = 2;

impl Default for GuestCallResult {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestCallResult {
    pub const fn new() -> Self {
        Self {
            value: AtomicU32::new(0),
            state: AtomicU8::new(PENDING),
        }
    }

    /// `None` = still running. `Some(Ok(r0))` = returned. `Some(Err(()))` = the call
    /// could not be made or the guest faulted.
    pub fn poll(&self) -> Option<Result<u32, ()>> {
        // Acquire pairs with the Release in `finish`, so a visible state implies the
        // value written before it is visible too.
        match self.state.load(Ordering::Acquire) {
            PENDING => None,
            OK => Some(Ok(self.value.load(Ordering::Relaxed))),
            _ => Some(Err(())),
        }
    }

    /// Record success. Called by the backend from the spawned task.
    pub fn ok(&self, value: u32) {
        self.value.store(value, Ordering::Relaxed);
        self.state.store(OK, Ordering::Release);
    }

    /// Record failure. Also what the trait's DEFAULT implementation does, so a host
    /// waiting on a backend that does not support guest calls fails fast instead of
    /// waiting forever.
    pub fn fail(&self) {
        self.state.store(FAILED, Ordering::Release);
    }
}

/// One argument of a host-initiated JVM call ([`crate::Emulator::call_java_method`]).
///
/// Deliberately two shapes and no more. The backend marshals a Java method's arguments
/// into machine words (a `B`/`S`/`Z`/`C`/`I` slot is one word, an object is its handle),
/// so the host has to be able to say exactly those two things and nothing else — a
/// `Long`/`Double` (two words) is not something any call the host makes needs, and a
/// typed enum would only have to be re-checked against the descriptor anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaArg {
    /// A primitive slot, as the raw word the callee reads: `Int(5)` for a `B` parameter
    /// is `(byte) 5`, `Int(1)` for a `Z` parameter is `true`.
    Int(i32),
    /// An object HANDLE — the guest address of the instance record, i.e. what the host
    /// read out of a static slot or a `Vector`'s element array, or what an earlier call
    /// returned. `0` is `null`.
    Object(u32),
}

/// Who receives a host-initiated JVM call ([`crate::Emulator::call_java_method`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaReceiver<'a> {
    /// The object a static field holds — `<class>.<field>:<descriptor>` — resolved by
    /// the JVM by name. The receiver of a singleton's method (`ag.a:Lag;` → `ag.l()V`).
    Static { class: &'a str, field: &'a str, descriptor: &'a str },
    /// An object handle the host holds (see [`JavaArg::Object`]). `class` names the class
    /// the host BELIEVES it is, and the backend refuses the call if the live object says
    /// otherwise — a handle is a heap address, and a heap address can be recycled.
    Handle { handle: u32, class: &'a str },
    /// No receiver: a `static` method of `class`.
    Class(&'a str),
}
