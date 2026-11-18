use core::mem::size_of;

use bytemuck::{Pod, Zeroable};

use wie_util::{Result, WieError, read_generic, write_generic};

use crate::core::ArmCore;

pub const BUCKET_MAX: usize = 512;

// (slot_size, slot_count). slot_count is a multiple of 8 because the class header
// is a one-bit-per-slot in-use map (preceded by BucketHeader's two words). Sizes
// chosen generously per slot class to fit inside the 128 MB BucketAllocator half
// of the heap; total layout is ~0x785A000 (~120 MB), leaving ~8 MB of intentional
// slack.
const BUCKETS: [(usize, usize); 8] = [
    (4, 0x100000),
    (8, 0x80000),
    (16, 0x80000),
    (32, 0x40000),
    (64, 0x40000),
    (128, 0x20000),
    (256, 0x20000),
    (512, 0x10000),
];

/// Terminates the free chain. No class comes anywhere near `u32::MAX` slots, so this can never
/// collide with a real slot index.
const SLOT_SENTINEL: u32 = u32::MAX;

/// A freed slot stores the index of the next free slot in its own first four bytes, so the smallest
/// slot class has to be at least one word wide. Pinned here because widening the classes is easy and
/// narrowing them would silently corrupt the chain.
const _: () = assert!(BUCKETS[0].0 >= 4);

/// `free_head` is the first word of [`BucketHeader`], so its address is `header_address` itself;
/// only the second word needs an offset.
const NEXT_NEVER_USED_OFFSET: u32 = size_of::<u32>() as u32;

/// The one-bit-per-slot in-use map starts right after [`BucketHeader`], inside the same class header
/// area the old full bitmap occupied.
const BITMAP_OFFSET: u32 = size_of::<BucketHeader>() as u32;

/// Slots a class can actually hand out.
///
/// ★ The free chain alone cannot survive a double free: pushing an already-free slot links it to
/// itself, and every later `alloc` of that class then returns the SAME address forever, silently
/// aliasing live objects. The old full-bitmap code was immune (setting an already-set bit is
/// idempotent) and caught it with a `debug_assert`, so dropping the bitmap entirely would have
/// traded 74 KB per allocation for a new silent-corruption mode. So the bitmap stays - but it is
/// now INDEXED (one byte read, one byte written) instead of scanned, which is the part that cost
/// the 3.98 GB.
///
/// It lives after the two header words, so the last 64 slots of every class lose their bit and are
/// simply never handed out. That is at most 64 slots out of 65,536, and it keeps `header_length`
/// - and therefore every slot address in the heap - exactly where it was.
const fn usable_slots(idx: usize) -> usize {
    BUCKETS[idx].1 - 8 * size_of::<BucketHeader>()
}

// The bitmap for `usable_slots` must fit after the header, inside the class header area, for every
// class - otherwise a slot's in-use bit would land on top of slot 0's payload.
const _: () = {
    let mut i = 0;
    while i < BUCKETS.len() {
        assert!(usable_slots(i) > 0);
        assert!(BITMAP_OFFSET as usize + usable_slots(i).div_ceil(8) <= header_length(i));
        i += 1;
    }
};

/// Per-class bookkeeping, stored in the first eight bytes of the class' header area.
///
/// This replaces the old one-bit-per-slot bitmap. The bitmap made both `alloc` and `free` read the
/// *entire* header - 128 KB for the 4-byte class - on every single call: measured at 3.98 GB of
/// guest-memory reads across 53,496 allocations, i.e. 74 KB per allocation, which is most of the
/// cost of a map load. `alloc` also scanned it linearly from index 0, so it got slower as the class
/// filled up.
///
/// Why the two words live at the *start of the old bitmap* rather than in a compact header of their
/// own: every slot address in the heap is `region_offset(i) + header_length(i) + index * slot_size`.
/// Shrinking `header_length` would move every slot in every class. Keeping the old size costs
/// ~360 KB of now-dead address space across all eight classes and keeps every guest pointer, and the
/// absolute addresses asserted by the tests below, exactly where they were.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct BucketHeader {
    /// Head of the LIFO chain of freed slots, or `SLOT_SENTINEL` when the chain is empty.
    free_head: u32,
    /// Index of the first slot that has never been handed out. Slots at or above this index are
    /// untouched guest memory.
    next_never_used: u32,
}

const fn header_length(idx: usize) -> usize {
    BUCKETS[idx].1 / 8
}

const fn region_size(idx: usize) -> usize {
    let (slot_size, slot_count) = BUCKETS[idx];
    slot_count / 8 + slot_size * slot_count
}

const fn region_offset(idx: usize) -> usize {
    let mut offset = 0;
    let mut i = 0;
    while i < idx {
        offset += region_size(i);
        i += 1;
    }
    offset
}

const fn total_size() -> usize {
    region_offset(BUCKETS.len() - 1) + region_size(BUCKETS.len() - 1)
}

pub struct BucketAllocator;

impl BucketAllocator {
    pub fn init(core: &mut ArmCore, base_address: u32, base_size: u32) -> Result<()> {
        let required = total_size() as u32;
        if base_size < required {
            tracing::error!("BucketAllocator region too small: got {base_size:#x}, need {required:#x}");
            return Err(WieError::AllocationFailure);
        }

        for (i, &(slot_size, slot_count)) in BUCKETS.iter().enumerate() {
            let header_address = base_address + region_offset(i) as u32;

            tracing::info!("Bucket {slot_size} slots {slot_count}");

            // Eight bytes per class, and nothing else. The old code filled the whole bitmap here
            // (0x5A000 bytes of 0xff), and pre-linking the free chain instead would be far worse:
            // writing a next pointer into all 2.9M slots means ~11 MB of writes touching every page
            // of the 120 MB region before the first frame is drawn. The bump index exists precisely
            // so that slots that were never allocated are never written to.
            write_generic(
                core,
                header_address,
                BucketHeader {
                    free_head: SLOT_SENTINEL,
                    next_never_used: 0,
                },
            )?;
        }

        Ok(())
    }

    pub fn alloc(core: &mut ArmCore, base_address: u32, size: u32) -> Result<u32> {
        let bucket_index = Self::find_bucket_index(size);
        let (slot_size, _) = BUCKETS[bucket_index];
        let slot_count = usable_slots(bucket_index);
        let header_address = base_address + region_offset(bucket_index) as u32;
        let slot_base = header_address + header_length(bucket_index) as u32;

        let header: BucketHeader = read_generic(core, header_address)?;
        wie_util::stats::add_bucket_header(size_of::<BucketHeader>() as u64);

        let free_head = header.free_head;
        if free_head != SLOT_SENTINEL {
            // A chained slot whose in-use bit is SET means the chain and the bitmap disagree, which
            // is exactly the double-free cycle this bitmap exists to stop. Drop the chain rather
            // than hand out a slot somebody else is still using.
            let head_valid = (free_head as usize) < slot_count && !Self::in_use(core, header_address, free_head)?;
            debug_assert!(head_valid, "bucket {slot_size} free chain head {free_head:#x} is out of range or already in use");

            if head_valid {
                let address = slot_base + free_head * slot_size as u32;

                // The freed slot's first word is the rest of the chain.
                let next: u32 = read_generic(core, address)?;
                wie_util::stats::add_bucket_header(size_of::<u32>() as u64);

                write_generic(core, header_address, next)?;
                Self::set_in_use(core, header_address, free_head, true)?;

                return Ok(address);
            }

            // A head outside the class, or one the bitmap says is live, means the chain was
            // corrupted - a stray guest write into a freed slot, a `free` whose size did not select
            // the class its `alloc` did, or a double free. Handing that index out would alias a live
            // object or return a pointer outside the class, so drop the chain and fall through to
            // the bump index. Leaking the chained slots is survivable; aliasing is not.
            tracing::error!("Bucket {slot_size} free chain corrupted (head {free_head:#x}), dropping it");

            write_generic(core, header_address, SLOT_SENTINEL)?;
        }

        if header.next_never_used as usize >= slot_count {
            return Err(WieError::AllocationFailure);
        }

        write_generic(core, header_address + NEXT_NEVER_USED_OFFSET, header.next_never_used + 1)?;
        Self::set_in_use(core, header_address, header.next_never_used, true)?;

        Ok(slot_base + header.next_never_used * slot_size as u32)
    }

    /// Is the slot's in-use bit set? See [`usable_slots`] for why this map still exists.
    fn in_use(core: &ArmCore, header_address: u32, index: u32) -> Result<bool> {
        let byte: u8 = read_generic(core, header_address + BITMAP_OFFSET + index / 8)?;
        wie_util::stats::add_bucket_header(1);

        Ok(byte & (1 << (index % 8)) != 0)
    }

    fn set_in_use(core: &mut ArmCore, header_address: u32, index: u32, in_use: bool) -> Result<()> {
        let address = header_address + BITMAP_OFFSET + index / 8;
        let mut byte: u8 = read_generic(core, address)?;
        wie_util::stats::add_bucket_header(1);

        let mask = 1u8 << (index % 8);
        byte = if in_use { byte | mask } else { byte & !mask };

        write_generic(core, address, byte)
    }

    pub fn free(core: &mut ArmCore, base_address: u32, address: u32, size: u32) -> Result<()> {
        let bucket_index = Self::find_bucket_index(size);
        let (slot_size, _) = BUCKETS[bucket_index];
        let slot_count = usable_slots(bucket_index);
        let header_address = base_address + region_offset(bucket_index) as u32;
        let slot_base = header_address + header_length(bucket_index) as u32;

        // An address that is not a slot boundary of this class would be pushed onto the chain and
        // handed back to somebody else later, corrupting the heap silently and far away from the
        // real bug. Leak it instead, loudly. (The old bitmap code indexed a Vec with the bad offset,
        // so it either panicked or flipped an unrelated bit.) The usual cause is a `free` whose size
        // does not select the same class the matching `alloc` did.
        let offset = address.wrapping_sub(slot_base);
        let index = offset / slot_size as u32;
        let is_slot_of_class = address >= slot_base && offset % slot_size as u32 == 0 && (index as usize) < slot_count;
        debug_assert!(is_slot_of_class, "bucket {slot_size}: {address:#x} is not a slot of this class");

        if !is_slot_of_class {
            tracing::error!("Bucket {slot_size}: free of {address:#x} is not a slot of this class, leaking it");

            return Ok(());
        }

        // ★ Double free. Without this check the slot is chained to itself and every later `alloc`
        // of the class returns the same address forever, quietly aliasing live objects - strictly
        // worse than the old bitmap, where freeing twice just set an already-set bit. Refuse and
        // leak the slot: idempotent, exactly like the old code, and loud enough to find the caller.
        if !Self::in_use(core, header_address, index)? {
            debug_assert!(false, "bucket {slot_size}: double free of {address:#x}");
            tracing::error!("Bucket {slot_size}: {address:#x} is already free, ignoring the double free");

            return Ok(());
        }

        let header: BucketHeader = read_generic(core, header_address)?;
        wie_util::stats::add_bucket_header(size_of::<BucketHeader>() as u64);

        // Chain the slot in place: its first word becomes the old head.
        Self::set_in_use(core, header_address, index, false)?;
        write_generic(core, address, header.free_head)?;
        write_generic(core, header_address, index)?;

        Ok(())
    }

    fn find_bucket_index(size: u32) -> usize {
        BUCKETS.iter().position(|&(s, _)| size as usize <= s).unwrap_or(BUCKETS.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use wie_util::{Result, WieError};

    use crate::ArmCore;

    use super::BucketAllocator;

    // Bucket 0 (4-byte): header_length = 0x100000 / 8 = 0x20000.
    //   First slot at base + 0x20000 = 0x40020000.
    // Bucket 1 (8-byte): region_offset = 0x20000 + 4*0x100000 = 0x420000.
    //   header_length = 0x80000 / 8 = 0x10000.
    //   First slot at base + 0x420000 + 0x10000 = 0x40430000.
    // Bucket 7 (512-byte): region_offset = 0x5858000, header_length = 0x10000 / 8 = 0x2000.
    //   First slot at base + 0x585A000 = 0x4585A000, 0x10000 slots.

    #[test]
    fn test_allocator() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x8000000)?;

        BucketAllocator::init(&mut core, 0x40000000, 0x8000000)?;

        let address1 = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(address1, 0x40020000);

        let address2 = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(address2, 0x40020004);

        let address3 = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(address3, 0x40020008);

        BucketAllocator::free(&mut core, 0x40000000, address2, 4)?;

        let address4 = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(address4, 0x40020004);

        let address5 = BucketAllocator::alloc(&mut core, 0x40000000, 5)?;
        assert_eq!(address5, 0x40430000);

        let address6 = BucketAllocator::alloc(&mut core, 0x40000000, 6)?;
        assert_eq!(address6, 0x40430008);

        let address7 = BucketAllocator::alloc(&mut core, 0x40000000, 7)?;
        assert_eq!(address7, 0x40430010);

        let address8 = BucketAllocator::alloc(&mut core, 0x40000000, 8)?;
        assert_eq!(address8, 0x40430018);

        Ok(())
    }

    #[test]
    fn test_init_rejects_undersized_region() {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x1000000).unwrap();

        // 0x1000000 (16 MB) is far too small for the full bucket layout.
        assert!(BucketAllocator::init(&mut core, 0x40000000, 0x1000000).is_err());
    }

    #[test]
    fn test_allocator_small_sizes() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x8000000)?;

        BucketAllocator::init(&mut core, 0x40000000, 0x8000000)?;

        // sizes 0..=3 must all land in the smallest (4-byte) bucket
        let a0 = BucketAllocator::alloc(&mut core, 0x40000000, 0)?;
        assert_eq!(a0, 0x40020000);
        let a1 = BucketAllocator::alloc(&mut core, 0x40000000, 1)?;
        assert_eq!(a1, 0x40020004);
        let a2 = BucketAllocator::alloc(&mut core, 0x40000000, 2)?;
        assert_eq!(a2, 0x40020008);
        let a3 = BucketAllocator::alloc(&mut core, 0x40000000, 3)?;
        assert_eq!(a3, 0x4002000c);

        // free with the same (small) size must round-trip without panicking
        BucketAllocator::free(&mut core, 0x40000000, a1, 1)?;
        let a1b = BucketAllocator::alloc(&mut core, 0x40000000, 1)?;
        assert_eq!(a1b, 0x40020004);

        Ok(())
    }

    /// The free chain is LIFO, and mixing it with the bump index must never hand out the same slot
    /// twice - the failure mode that would let two guest objects alias.
    #[test]
    fn test_free_chain_is_lifo_and_never_aliases_bump() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x8000000)?;

        BucketAllocator::init(&mut core, 0x40000000, 0x8000000)?;

        let mut handed_out = Vec::new();

        let a = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        let b = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        let c = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        handed_out.extend_from_slice(&[a, b, c]);

        BucketAllocator::free(&mut core, 0x40000000, b, 4)?;
        BucketAllocator::free(&mut core, 0x40000000, c, 4)?;

        // Last freed comes back first.
        let d = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(d, c);
        let e = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(e, b);

        // Chain exhausted: the bump index must resume where it left off, not at zero.
        let f = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        assert_eq!(f, 0x4002000c);
        handed_out.push(f);

        // Churn with the chain and the bump index interleaved, then check nothing was aliased.
        for _ in 0..64 {
            let x = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
            let y = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
            BucketAllocator::free(&mut core, 0x40000000, x, 4)?;
            handed_out.push(y);
        }
        // `x` slots were returned to the chain and may legitimately reappear, so only the addresses
        // still live are compared.
        handed_out.sort_unstable();
        let live = handed_out.len();
        handed_out.dedup();
        assert_eq!(handed_out.len(), live);

        Ok(())
    }

    /// ★ A double free must not be able to alias live objects. With a bare free chain, freeing the
    /// same slot twice links it to itself and every later allocation of that class returns the SAME
    /// address forever; the in-use map exists to make that impossible. Freeing twice has to stay
    /// idempotent, exactly as it was with the old full bitmap.
    #[test]
    #[cfg(not(debug_assertions))] // the double free trips a debug_assert by design
    fn test_double_free_cannot_alias() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x8000000)?;

        BucketAllocator::init(&mut core, 0x40000000, 0x8000000)?;

        let a = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
        let b = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;

        BucketAllocator::free(&mut core, 0x40000000, a, 4)?;
        BucketAllocator::free(&mut core, 0x40000000, a, 4)?; // the double free: must be ignored

        // `a` comes back once (it is legitimately on the chain), and after that the allocator must
        // move on instead of handing `a` out again.
        assert_eq!(BucketAllocator::alloc(&mut core, 0x40000000, 4)?, a);

        let mut seen = Vec::new();
        for _ in 0..8 {
            let x = BucketAllocator::alloc(&mut core, 0x40000000, 4)?;
            assert_ne!(x, a, "double free handed a live slot back out");
            assert_ne!(x, b, "double free handed a live slot back out");
            seen.push(x);
        }
        let live = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), live, "the same slot was handed out twice");

        Ok(())
    }

    /// A class that runs out must report `AllocationFailure` rather than returning an address past
    /// the end of its region, and a single free must make exactly one more allocation possible.
    #[test]
    fn test_class_exhaustion() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x8000000)?;

        BucketAllocator::init(&mut core, 0x40000000, 0x8000000)?;

        // Bucket 7 is the smallest class by slot count (0x10000 slots of 512 bytes), of which the
        // last 64 are reserved so the in-use map fits after the header - see `usable_slots`.
        let usable = super::usable_slots(7);
        assert_eq!(usable, 0x10000 - 64);

        let mut last = 0;
        for _ in 0..usable {
            last = BucketAllocator::alloc(&mut core, 0x40000000, 512)?;
        }
        assert_eq!(last, 0x4585A000 + (usable as u32 - 1) * 0x200);

        let exhausted = BucketAllocator::alloc(&mut core, 0x40000000, 512);
        assert!(matches!(exhausted, Err(WieError::AllocationFailure)));

        BucketAllocator::free(&mut core, 0x40000000, last, 512)?;
        assert_eq!(BucketAllocator::alloc(&mut core, 0x40000000, 512)?, last);
        assert!(matches!(
            BucketAllocator::alloc(&mut core, 0x40000000, 512),
            Err(WieError::AllocationFailure)
        ));

        Ok(())
    }
}
