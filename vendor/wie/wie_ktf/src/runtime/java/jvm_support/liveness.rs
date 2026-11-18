//! Is this guest record still a live java instance, or did the collector already free it?
//!
//! ## The fingerprint (2026-09-10)
//!
//! A KTF instance is two guest words, `wipi_types::ktf::java::JavaClassInstance`:
//!
//! ```text
//!   ptr_raw + 0 : ptr_fields   -> the field block (its word 0 is the vtable index)
//!   ptr_raw + 4 : ptr_class    -> the JavaClass
//! ```
//!
//! `JavaClassInstance::destroy` frees both blocks, and `Allocator::free` of an 8-byte
//! block lands in `BucketAllocator`, whose `free` chains the slot by writing the previous
//! free-list head **into the slot's first word**. That word is `ptr_fields`. `ptr_class`
//! is never touched.
//!
//! So a freed instance is not obviously broken - it is *plausibly* broken, which is far
//! worse:
//!
//! * `read_raw()` succeeds (the slot is still mapped heap),
//! * `class()` succeeds and `name()` returns the RIGHT class name (`ptr_class` survived),
//! * and only `field_address()` is wrong, because `ptr_fields` now holds a slot INDEX
//!   (`0 ..= 0x7ffff`) or `SLOT_SENTINEL` (`0xffff_ffff`).
//!
//! `ptr_fields + offset + 4` is then a single- or double-digit address - or, from the
//! sentinel, `offset + 3` after the wrap - and the guest read of it fails. That is the
//! device freeze of 2026-09-09: the worker panicked inside `get_field`, the process lived
//! on with the last frame still painted, and it looked like a hang.
//!
//! ## Why a range check is the right test, and why it is free
//!
//! The guest heap is exactly `[HEAP_BASE, HEAP_BASE + HEAP_SIZE)`. Every value the free
//! chain can leave behind is a slot index or the sentinel, so *every* freed instance fails
//! the range test and no live one can: a live `ptr_fields` came straight out of
//! `Allocator::alloc`. The check costs two comparisons on a word the caller has already
//! read, so the success path pays nothing - which matters, because the collector's mark
//! phase runs this on every object-typed field of every reachable object, every frame.
//!
//! It is a *necessary* condition, not a sufficient one: a slot that has been freed AND
//! handed back out will pass. Nothing cheap can catch that, which is why the report below
//! exists - a hit here means the collector is freeing objects that are still in use, and
//! the aliasing case is the same bug arriving silently.
//!
//! ## What a hit tells you
//!
//! `Jvm::collect_garbage` is the only code in the tree that frees an instance. The KTF
//! init table (`runtime/init.rs`) gives the guest `fn_alloc` and no matching free, and the
//! `WIPIJBInterface` has no delete either, so the guest cannot have done it. A hit
//! therefore always means: the collector's root set did not see a reference that was still
//! live. `jvm::is_collecting` says whether the dangling edge was followed by the collector itself
//! (a field of some object still points at the corpse) or handed to us from elsewhere.

use alloc::string::String;

use wie_core_arm::{HEAP_BASE, HEAP_SIZE};
use wie_util::stats::UAF_REPORT_LIMIT;

/// Was the collector running when this went wrong? See `jvm::is_collecting` for why the flag has
/// to live inside the collector rather than at one of its three call sites.
fn in_gc() -> bool {
    jvm::is_collecting()
}

/// `ptr_fields` of a live instance always points into the guest heap. See the module note
/// for why nothing a freed slot can hold does.
pub fn fields_pointer_is_live(ptr_fields: u32) -> bool {
    ptr_fields >= HEAP_BASE && ptr_fields < HEAP_BASE.wrapping_add(HEAP_SIZE)
}

/// Report one use-after-free, rate limited. `what` names the access (`"get_field"`,
/// `"array length"`, ...) and `detail` carries whatever the caller can cheaply name -
/// class, field, descriptor. `detail` is a closure so that the formatting is skipped
/// entirely once the report limit is reached.
pub fn report(what: &str, ptr_raw: u32, raw: Option<(u32, u32)>, detail: impl FnOnce() -> String) {
    if !wie_util::stats::note_instance_uaf() {
        return;
    }

    let heap_end = HEAP_BASE.wrapping_add(HEAP_SIZE);
    let in_gc = in_gc();
    let limit = UAF_REPORT_LIMIT;

    match raw {
        // The free-chain fingerprint. `ptr_class` is intact, so `detail` can still name the class.
        Some((ptr_fields, ptr_class)) => tracing::error!(
            "USE AFTER FREE of java instance {ptr_raw:#x} in {what}: ptr_fields={ptr_fields:#x} is outside the guest heap [{HEAP_BASE:#x},{heap_end:#x}); ptr_class={ptr_class:#x}; during_gc={in_gc}; {}. The collector freed an object that is still referenced. First {limit} only.",
            detail()
        ),
        // Not that fingerprint: the two-word record itself could not be read, so this pointer was
        // never an instance at all. A different bug, and it must not read as a collector bug.
        None => tracing::error!(
            "NOT AN INSTANCE at {ptr_raw:#x} in {what}: the two-word record itself is unreadable; during_gc={in_gc}; {}. First {limit} only.",
            detail()
        ),
    }
}

/// An operation refused to run because a guest read it needed had already failed and been
/// reported by one of the two functions above. Says WHICH operation gave up, so the fallback it
/// took (a zero length, an empty load, a dropped store) is never silent.
pub fn report_refusal(what: &str, ptr_raw: u32, detail: impl FnOnce() -> String) {
    if !wie_util::stats::note_instance_uaf() {
        return;
    }

    tracing::error!(
        "REFUSED {what} on java object {ptr_raw:#x}; during_gc={}; {}. The cause is in the report just above. First {} only.",
        in_gc(),
        detail(),
        UAF_REPORT_LIMIT
    );
}

/// The instance looked live - `ptr_fields` was a real heap pointer - and the access at
/// `address` still failed.
///
/// This is a DIFFERENT bug from the one above and must never be folded into it. It is what
/// `--gc 0` and `--gc 2` hit on hero2: the guest heap fills, and something hands back an
/// in-range address that is not backed. Swallowing it would have been the easy way to make
/// those two modes stop crashing while learning nothing, so it gets its own name.
pub fn report_access(what: &str, ptr_raw: u32, address: u32, detail: impl FnOnce() -> String) {
    if !wie_util::stats::note_instance_uaf() {
        return;
    }

    tracing::error!(
        "GUEST ACCESS FAILED at {address:#x} in {what} on instance {ptr_raw:#x}, whose ptr_fields IS a heap pointer; during_gc={}; {}. Not the use-after-free fingerprint. First {} only.",
        in_gc(),
        detail(),
        UAF_REPORT_LIMIT
    );
}

#[cfg(test)]
mod tests {
    use super::fields_pointer_is_live;
    use wie_core_arm::{HEAP_BASE, HEAP_SIZE};

    /// The exact record this module was written for, captured from a headless run on
    /// 2026-09-10: `ptr_raw=0x48435198 ptr_fields=0x71 ptr_class=0x49048240`, field `alive:Z`
    /// at offset 0xc, giving the address `0x81` that the guest write then failed on.
    ///
    /// `ptr_fields` is `0x71` because `BucketAllocator::free` had chained the slot and written
    /// the free-list head - a slot INDEX - over the record's first word. That is the whole
    /// fingerprint, and the point of the test is that the index is small while a real field
    /// block never is.
    #[test]
    fn the_device_fingerprint_reads_as_dead() {
        assert!(!fields_pointer_is_live(0x71));
    }

    /// Anything the free chain can leave behind: the sentinel that terminates it, and a slot
    /// index from any of the eight size classes (the largest has 0x100000 slots).
    #[test]
    fn nothing_the_free_chain_writes_reads_as_live() {
        assert!(!fields_pointer_is_live(u32::MAX));
        assert!(!fields_pointer_is_live(0));
        for index in [1u32, 0x71, 0x1000, 0x7ffff, 0xfffff] {
            assert!(!fields_pointer_is_live(index), "slot index {index:#x} must not read as live");
        }
    }

    /// ...and the converse, or the check would refuse live objects and hide the whole heap
    /// behind a zero-valued field.
    #[test]
    fn the_heap_reads_as_live() {
        assert!(fields_pointer_is_live(HEAP_BASE));
        assert!(fields_pointer_is_live(HEAP_BASE + HEAP_SIZE / 2));
        assert!(fields_pointer_is_live(HEAP_BASE + HEAP_SIZE - 1));

        // Exclusive upper bound: one past the end is not the heap.
        assert!(!fields_pointer_is_live(HEAP_BASE + HEAP_SIZE));
        // And neither is anything below it - the guest image and stacks live down there.
        assert!(!fields_pointer_is_live(HEAP_BASE - 1));
    }
}
