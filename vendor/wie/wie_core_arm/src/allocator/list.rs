use core::mem::size_of;

use bytemuck::{Pod, Zeroable};

use wie_util::{Result, WieError, read_generic, write_generic};

use crate::core::ArmCore;

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct ListAllocationHeader {
    data: u32,
}

impl ListAllocationHeader {
    pub fn new(size: u32, in_use: bool) -> Self {
        Self {
            data: size | ((in_use as u32) << 31),
        }
    }

    pub fn size(&self) -> u32 {
        self.data & 0x7FFFFFFF
    }

    pub fn in_use(&self) -> bool {
        self.data & 0x80000000 != 0
    }
}

const CANARY_SIZE: u32 = 4;
const CANARY_VALUE: u32 = 0xDEADBEEF;

pub struct ListAllocator;

impl ListAllocator {
    pub fn init(core: &mut ArmCore, base_address: u32, size: u32) -> Result<()> {
        let header = ListAllocationHeader::new(size, false);

        write_generic(core, base_address, header)?;

        Ok(())
    }

    pub fn alloc(core: &mut ArmCore, base_address: u32, base_size: u32, size: u32) -> Result<u32> {
        // Every block size is a multiple of 4: `next_multiple_of(4)` rounds the payload up and
        // CANARY_SIZE is 4. The region starts as one block of `base_size` (also a multiple of 4)
        // and both splitting and coalescing only add or subtract such sizes, so a split remainder
        // is either 0 or at least large enough to hold its own header.
        let size_to_alloc = (size as usize + size_of::<ListAllocationHeader>()).next_multiple_of(4) as u32 + CANARY_SIZE;

        let address = Self::find_address(core, base_address, base_size, size_to_alloc)?;

        // Note this is re-read *after* `find_address` may have coalesced, so it is the merged size.
        let previous_header: ListAllocationHeader = read_generic(core, address)?;

        let header = ListAllocationHeader::new(size_to_alloc, true);
        write_generic(core, address, header)?;

        // write next
        if previous_header.size() > size_to_alloc {
            let next_header = ListAllocationHeader::new(previous_header.size() - size_to_alloc, false);
            write_generic(core, address + size_to_alloc, next_header)?;
        }

        // write canary
        write_generic(core, address + size_to_alloc - CANARY_SIZE, CANARY_VALUE)?;

        tracing::trace!("Allocated {size:#x} bytes at {:#x}", address + size_of::<ListAllocationHeader>() as u32);

        Ok(address + size_of::<ListAllocationHeader>() as u32)
    }

    pub fn free(core: &mut ArmCore, address: u32) -> Result<()> {
        let base_address = address - size_of::<ListAllocationHeader>() as u32;

        tracing::trace!("Freeing {address:#x}");

        let header: ListAllocationHeader = read_generic(core, base_address)?;
        debug_assert!(header.in_use());

        let canary_value: u32 = read_generic(core, base_address + header.size() - CANARY_SIZE)?;
        debug_assert!(
            canary_value == CANARY_VALUE,
            "Invalid canary value at {base_address:#x}: expected {CANARY_VALUE:#x}, got {canary_value:#x}"
        );

        // Only the in-use bit is cleared here; the block keeps its size and its neighbours are not
        // touched. Merging happens lazily in `find_address` instead - see the comment there for why
        // it cannot be done from this side.
        let header = ListAllocationHeader::new(header.size(), false);
        write_generic(core, base_address, header)?;

        Ok(())
    }

    /// Walks the block chain for a free block of at least `size` bytes, coalescing runs of adjacent
    /// free blocks into one as it goes.
    ///
    /// Why coalescing lives here and not in `free`: the chain is singly linked forwards - a header
    /// records only its own size - so `free` cannot reach the block *preceding* the one being
    /// released without a footer or a back pointer, and adding either would change the block layout
    /// that the canary offset and all the size arithmetic depend on. The walk needs no back pointer
    /// at all: the cursor is already standing on the predecessor.
    ///
    /// Before this, `free` only cleared the in-use bit and `alloc` only ever split blocks, so a
    /// block could never grow back. The number of blocks therefore rose monotonically and large
    /// allocations began to fail while plenty of free space remained, contiguous but unmergeable -
    /// that is the "crashes after 21-35 minutes of play" report. The same growth made every walk
    /// longer, which is the "gets slower and slower first" part of it.
    fn find_address(core: &mut ArmCore, base_address: u32, base_size: u32, size: u32) -> Result<u32> {
        let region_end = base_address + base_size;

        let mut cursor = base_address;
        let mut steps = 0u64;

        while cursor < region_end {
            steps += 1;

            let header: ListAllocationHeader = read_generic(core, cursor)?;
            // Corrupt header: stop the walk, exactly as the old cursor-overrun did. Blocks BEFORE
            // this one were already offered to the caller, so a corruption late in the heap does not
            // fail an allocation that a healthy earlier block could satisfy.
            let Some(block_size) = Self::checked_block_size(cursor, header, region_end) else {
                break;
            };

            if header.in_use() {
                cursor += block_size;
                continue;
            }

            let block_size = Self::coalesce(core, cursor, block_size, region_end, &mut steps)?;

            if block_size >= size {
                wie_util::stats::add_list_walk(steps);

                return Ok(cursor);
            }

            // A coalesced block is strictly larger than the one we started from, so the cursor
            // always advances by at least the original size and the walk cannot loop forever.
            cursor += block_size;
        }

        wie_util::stats::add_list_walk(steps);

        Err(WieError::AllocationFailure)
    }

    /// Absorbs every free block immediately following the free block at `cursor` into it, rewrites
    /// the header and returns the merged size.
    fn coalesce(core: &mut ArmCore, cursor: u32, mut block_size: u32, region_end: u32, steps: &mut u64) -> Result<u32> {
        let original_size = block_size;

        loop {
            let next = cursor + block_size;
            if next >= region_end {
                break;
            }

            *steps += 1;

            let next_header: ListAllocationHeader = read_generic(core, next)?;
            // A corrupt neighbour is simply not absorbed. Merging an unknown extent into this block
            // would spread the damage; refusing to merge only costs us the coalescing opportunity,
            // and the block we already have is still perfectly usable.
            let Some(next_size) = Self::checked_block_size(next, next_header, region_end) else {
                break;
            };
            if next_header.in_use() {
                break;
            }

            // Bounded by `checked_block_size`: every block ends at or before `region_end`, so the
            // running total can never exceed the region and never overflows.
            block_size += next_size;
        }

        if block_size != original_size {
            // Writing the merged header back is load-bearing, not a cache: `alloc` re-reads this
            // header to work out how much of the block to give back as the trailing free remainder.
            // Returning a merged size without storing it would make `alloc` split against the stale
            // (smaller) size and leak everything we just absorbed.
            //
            // Canary: the merged block swallows the stale 0xDEADBEEF words of the blocks it ate.
            // That is harmless. The canary is only read by `free`, only at `block + size - 4`, and
            // only for a block that is in use; a block becomes in use again solely through `alloc`,
            // which writes a fresh canary at its own new end. Free blocks are never canary-checked.
            write_generic(core, cursor, ListAllocationHeader::new(block_size, false))?;
        }

        Ok(block_size)
    }

    /// A header whose size is 0, or that claims to run past the end of the region, means the heap is
    /// corrupt (guest code writing out of bounds). `None` means "stop walking here" — with
    /// coalescing a bad size would otherwise be folded into its neighbour and the damage would
    /// spread across the heap.
    ///
    /// ★ Deliberately NOT a `FatalError`. The old walk reacted to an over-large size by adding it to
    /// the cursor, jumping past `base + base_size` and ending the loop, so a corrupt block cost the
    /// caller one `AllocationFailure` at worst and allocations that never reached it still
    /// succeeded. Turning that into a fatal error would make this change — whose entire purpose is
    /// to stop the emulator dying after 21-35 minutes — kill the emulator EARLIER on a heap that the
    /// old code survived.
    fn checked_block_size(cursor: u32, header: ListAllocationHeader, region_end: u32) -> Option<u32> {
        let size = header.size();

        match cursor.checked_add(size) {
            Some(block_end) if size != 0 && block_end <= region_end => Some(size),
            _ => {
                tracing::error!("Invalid allocation header at {cursor:#x}: size {size:#x}, region end {region_end:#x}");

                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use wie_util::Result;

    use crate::ArmCore;

    use super::ListAllocator;

    const BASE: u32 = 0x40000000;
    const SIZE: u32 = 0x1000;

    fn heap() -> Result<ArmCore> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(BASE, SIZE)?;
        ListAllocator::init(&mut core, BASE, SIZE)?;

        Ok(core)
    }

    #[test]
    fn test_allocator() -> Result<()> {
        let mut core = ArmCore::new(false, None).unwrap();
        core.map(0x40000000, 0x1000)?;

        ListAllocator::init(&mut core, 0x40000000, 0x1000)?;
        let address = ListAllocator::alloc(&mut core, 0x40000000, 0x1000, 4)?;

        assert_eq!(address, 0x40000004);

        Ok(())
    }

    /// The regression test for the fragmentation crash: after churn, freeing everything must make
    /// the whole region allocatable again. Without coalescing the region is left as a dozen small
    /// free blocks that never rejoin, the largest of them far smaller than the request, and the
    /// final allocation fails with `AllocationFailure` even though every byte is free.
    #[test]
    fn test_coalescing_reclaims_whole_region_after_churn() -> Result<()> {
        let mut core = heap()?;

        // 8 * 0x108 = 0x840 of the 0x1000 region.
        let mut blocks = Vec::new();
        for _ in 0..8 {
            blocks.push(ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?);
        }

        // Interleave: release every other block, then refill the holes with a smaller size so each
        // hole is split into a used part and a free tail. This is the shape that made the old
        // allocator's block count grow without bound.
        for (i, &block) in blocks.iter().enumerate() {
            if i % 2 == 0 {
                ListAllocator::free(&mut core, block)?;
            }
        }

        let mut refills = Vec::new();
        for _ in 0..4 {
            refills.push(ListAllocator::alloc(&mut core, BASE, SIZE, 0x80)?);
        }

        for (i, &block) in blocks.iter().enumerate() {
            if i % 2 == 1 {
                ListAllocator::free(&mut core, block)?;
            }
        }
        for &block in &refills {
            ListAllocator::free(&mut core, block)?;
        }

        // Everything is free, so a request that needs nearly the entire region must succeed, and it
        // must come from the very start of the region.
        let whole = ListAllocator::alloc(&mut core, BASE, SIZE, 0xF00)?;
        assert_eq!(whole, BASE + 4);

        Ok(())
    }

    /// Three adjacent free blocks must fuse into one, and a request smaller than their sum must be
    /// served out of that fused block (not from the untouched tail of the region), leaving a usable
    /// remainder behind it.
    #[test]
    fn test_three_adjacent_free_blocks_fuse() -> Result<()> {
        let mut core = heap()?;

        // Five 0x108-byte blocks: a, b, c, d, e.
        let a = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        let b = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        let c = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        let d = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        let e = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        assert_eq!(a, BASE + 4);
        assert_eq!(b, a + 0x108);
        assert_eq!(c, b + 0x108);
        assert_eq!(d, c + 0x108);
        assert_eq!(e, d + 0x108);

        // b, c, d are adjacent and total 0x318 bytes; a and e stay in use so the merge is forced to
        // stop at both ends.
        ListAllocator::free(&mut core, b)?;
        ListAllocator::free(&mut core, c)?;
        ListAllocator::free(&mut core, d)?;

        // 0x208 needed - larger than any single freed block, so this address proves the merge.
        let merged = ListAllocator::alloc(&mut core, BASE, SIZE, 0x200)?;
        assert_eq!(merged, b);

        // The 0x110-byte remainder of the fused block is still usable and sits right behind it.
        let remainder = ListAllocator::alloc(&mut core, BASE, SIZE, 0x100)?;
        assert_eq!(remainder, b + 0x208);

        // e's canary must be intact: coalescing must not have written into a live block.
        ListAllocator::free(&mut core, e)?;

        Ok(())
    }
}
