//! Guest-heap-exhaustion handling for the KTF JVM bridge.
//!
//! Why this module exists: `Jvm::exception` builds the exception message as a real `java.lang.String`, which means a `char[]` allocation on the
//! *guest* heap. When the guest heap is exactly what ran out, that allocation fails, our `instantiate_array` reports the failure by calling
//! `Jvm::exception` again, that call allocates another message, and the two mutually recurse until the 8 MiB host stack is gone. Five device
//! tombstones (2026-08-05, vc11) end precisely that way: SIGSEGV in the stack probe with the ASCII "Allocation failure" still sitting in x6/x7 and
//! roughly 1 KiB of stack burned per cycle.
//!
//! A plain re-entrancy guard does not fix it. If the *inner* call declines to build an exception it must return an `Err`, and `Jvm::exception`
//! `.unwrap()`s the result — we would only trade SIGSEGV for a panic. The *outer* call is the one that must never enter `Jvm::exception`, so we
//! keep one `net/wie/WieError` instance that was allocated at startup while the heap was still healthy and hand that out instead. All four throw
//! sites in this crate throw `net/wie/WieError` anyway, so the substitution costs the message text and nothing else — a `catch` that would have
//! matched still matches.
//!
//! `jvm` is a git dependency and cannot be patched here, which is why the fix lives entirely on our side of the boundary.

use alloc::{boxed::Box, string::String};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use jvm::{ClassInstance, JavaError, Jvm};

use wie_core_arm::ArmCore;
use wie_jvm_support::JvmSupport;
use wie_util::WieError;

use super::JavaClassInstance;

pub const WIE_ERROR_CLASS: &str = "net/wie/WieError";
pub const WIE_ERROR_DESCRIPTOR: &str = "Lnet/wie/WieError;";

/// The singleton is reachable only from `OOM_EXCEPTION` below, and `determine_garbage` roots objects from class static fields, thread frames and
/// interned strings — a raw `u32` held in Rust is none of those. Without a root the collector would free it out from under us on the next
/// `System.gc()` (wie_midp's `Display` calls `collect_garbage` directly) and we would later hand out a dangling guest pointer. So park it in a
/// static field of `net/wie/KtfClassLoader`: that class is defined by this crate and registered before any guest code can run.
const ROOT_CLASS: &str = "net/wie/KtfClassLoader";
pub const ROOT_FIELD: &str = "oomException";

/// Raw guest pointer of the pre-allocated `net/wie/WieError`. 0 means "unavailable" — see `init`.
static OOM_EXCEPTION: AtomicU32 = AtomicU32::new(0);

/// Latched once the guest heap has failed an allocation. From that point on every throw site in this crate uses the singleton: a heap that just
/// failed will very likely fail the message allocation too, and a failure *inside* `Jvm::exception` lands on its `.unwrap()` and panics.
static HEAP_DEAD: AtomicBool = AtomicBool::new(false);

/// Backstop for error kinds other than `AllocationFailure`: set while a throw site is inside `Jvm::exception`, so a nested throw takes the
/// singleton instead of recursing. Deliberately global rather than per-task; over-blocking only costs a message string because every site guarded
/// here throws the same class, and that is a much better trade than dying.
static IN_EXCEPTION: AtomicBool = AtomicBool::new(false);

/// Reported at most once. 18th round already caught a separate performance bug where `__android_log_write` ran 124x/s, and a recursion storm here
/// could emit tens of thousands of lines per second.
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Same idea, for the first exception message we throw away because the heap is already dead.
static MESSAGE_DROPPED: AtomicBool = AtomicBool::new(false);

struct ExceptionGuard;

impl ExceptionGuard {
    fn try_acquire() -> Option<Self> {
        if IN_EXCEPTION.swap(true, Ordering::AcqRel) { None } else { Some(Self) }
    }
}

impl Drop for ExceptionGuard {
    fn drop(&mut self) {
        IN_EXCEPTION.store(false, Ordering::Release);
    }
}

/// Pure decision so it can be unit tested without a JVM. Without a singleton there is nothing to hand out, so `ptr == 0` always falls through to
/// the legacy path regardless of how bad things are.
fn use_singleton(ptr: u32, allocation_failure: bool, heap_dead: bool, reentered: bool) -> bool {
    ptr != 0 && (allocation_failure || heap_dead || reentered)
}

fn instance_raw(instance: &dyn ClassInstance) -> Option<u32> {
    // Only a plain (non-array) instance can be the singleton; downcasting by hand avoids `KtfJvmSupport::class_instance_raw`'s `unwrap` on the
    // array branch, which would panic for any instance this crate did not create.
    instance.as_any().downcast_ref::<JavaClassInstance>().map(|x| x.ptr_raw)
}

fn note_allocation_failure() {
    HEAP_DEAD.store(true, Ordering::Release);

    if !REPORTED.swap(true, Ordering::AcqRel) {
        tracing::error!("Guest heap exhausted; java exceptions now use the pre-allocated net/wie/WieError and drop their message");
    }
}

/// Allocate the singleton and root it. Best effort: if either step fails we leave `OOM_EXCEPTION` at 0 and every throw site keeps the old
/// behaviour, which is the pre-existing (recursive) risk rather than a new one. Failing here means the heap was already broken at startup, in
/// which case the emulator is not going to get far anyway.
pub async fn init(jvm: &Jvm) {
    let instance = match jvm.new_class(WIE_ERROR_CLASS, "()V", ()).await {
        Ok(x) => x,
        Err(_) => {
            tracing::error!("Could not pre-allocate the OOM exception; heap exhaustion will use the recursive legacy path");
            return;
        }
    };

    let ptr = match instance_raw(&*instance) {
        Some(x) if x != 0 => x,
        _ => {
            tracing::error!("Pre-allocated OOM exception has no raw pointer; heap exhaustion will use the recursive legacy path");
            return;
        }
    };

    let rooted = jvm.put_static_field(ROOT_CLASS, ROOT_FIELD, WIE_ERROR_DESCRIPTOR, instance).await;
    if rooted.is_err() {
        // Unrooted it would be collected by the next GC, so we would be handing out a freed guest pointer. That is strictly worse than not
        // having the fast path at all.
        tracing::error!("Could not root the OOM exception in {ROOT_CLASS}.{ROOT_FIELD}; heap exhaustion will use the recursive legacy path");
        return;
    }

    OOM_EXCEPTION.store(ptr, Ordering::Release);
}

/// Build the `JavaError` for a `WieError` that crossed back into the JVM.
///
/// `message` is a closure, not a `String`, because on the exhausted path even `format!` is a host allocation we do not want to pay for and, more
/// to the point, the formatted message would only be thrown away.
pub async fn java_error<F>(jvm: &Jvm, core: &ArmCore, allocation_failure: bool, message: F) -> JavaError
where
    F: FnOnce() -> String + Send,
{
    if allocation_failure {
        note_allocation_failure();
    }

    let ptr = OOM_EXCEPTION.load(Ordering::Acquire);
    let heap_dead = HEAP_DEAD.load(Ordering::Acquire);

    if use_singleton(ptr, allocation_failure, heap_dead, false) {
        return singleton(ptr, core, allocation_failure, message);
    }

    // Held across the await on purpose: the recursion we are guarding against goes *through* that await, so releasing early would defeat it.
    let _guard = match ExceptionGuard::try_acquire() {
        Some(x) => x,
        None => {
            if use_singleton(ptr, allocation_failure, heap_dead, true) {
                return singleton(ptr, core, allocation_failure, message);
            }

            // No singleton to fall back on; the legacy path is all that is left.
            return jvm.exception(WIE_ERROR_CLASS, &message()).await;
        }
    };

    jvm.exception(WIE_ERROR_CLASS, &message()).await
}

fn singleton<F>(ptr: u32, core: &ArmCore, allocation_failure: bool, message: F) -> JavaError
where
    F: FnOnce() -> String,
{
    // The message is thrown away here, which would silently hide a genuine `FatalError` behind a blank exception once the latch is set. Render it
    // once, to the host log, so that debugging an unrelated failure does not get harder. Never on the allocation-failure path itself: `format!`
    // there is pure waste and this is the frame we are trying to keep cheap.
    if !allocation_failure && !MESSAGE_DROPPED.swap(true, Ordering::AcqRel) {
        tracing::error!("Dropping java exception message on the exhausted-heap path: {}", message());
    }

    JavaError::JavaException(Box::new(JavaClassInstance::from_raw(ptr, core)))
}

/// `JvmSupport::to_wie_err` renders a stack trace, which needs a `StringWriter`, a `PrintWriter` and the `StringBuffer` behind them — three more
/// guest allocations, each `unwrap`ped. Handing it the OOM singleton would therefore panic, so short-circuit before it. Callers in this crate
/// should use this instead of `JvmSupport::to_wie_err` directly.
pub async fn to_wie_err(jvm: &Jvm, error: JavaError) -> WieError {
    let ptr = OOM_EXCEPTION.load(Ordering::Acquire);
    let is_singleton = match &error {
        JavaError::JavaException(x) => ptr != 0 && instance_raw(&**x) == Some(ptr),
    };

    if is_singleton {
        return WieError::AllocationFailure;
    }

    JvmSupport::to_wie_err(jvm, error).await
}

#[cfg(test)]
mod tests {
    use super::{ExceptionGuard, IN_EXCEPTION, Ordering, use_singleton};

    // One test, not several: the statics are process-global and `cargo test` runs test functions on parallel threads, so splitting these would
    // make them race each other rather than test anything.
    #[test]
    fn guard_blocks_reentry_and_releases_on_drop() {
        let outer = ExceptionGuard::try_acquire();
        assert!(outer.is_some(), "first acquire must succeed");
        assert!(ExceptionGuard::try_acquire().is_none(), "re-entry must be refused");

        drop(outer);
        assert!(!IN_EXCEPTION.load(Ordering::Acquire));

        let again = ExceptionGuard::try_acquire();
        assert!(again.is_some(), "the guard must be reusable");
        drop(again);
        assert!(!IN_EXCEPTION.load(Ordering::Acquire));
    }

    #[test]
    fn singleton_is_used_only_when_it_exists() {
        // Without a pre-allocated instance there is nothing to hand out, whatever the state.
        assert!(!use_singleton(0, true, true, true));

        // Allocation failure alone is enough — this is the tombstone case.
        assert!(use_singleton(0x1234, true, false, false));
        // So is the latch, so that a second failure never re-enters Jvm::exception.
        assert!(use_singleton(0x1234, false, true, false));
        // So is re-entry, the backstop for error kinds other than AllocationFailure.
        assert!(use_singleton(0x1234, false, false, true));

        // A healthy heap keeps the informative message.
        assert!(!use_singleton(0x1234, false, false, false));
    }
}
