// The guest's 4 GB page table, with **lazy commit**.
//
// Regular `//` comments, not `//!`: this file is `include!`d into the middle of another
// module, where an inner doc comment is a syntax error.
//
// ── WHY THIS FILE HAS NO DEPENDENCIES ───────────────────────────────────────
// It is `include!`d, not `mod`ed: by `engine/arm32_cpu.rs` (where it is the real memory of
// the running emulator) and by the repo-root `page_table_selftest.rs`. `cargo test` cannot
// run in this workspace on the Windows host — the dev-dependency graph pulls `parking_lot`,
// whose `windows` import libraries need a `dlltool` the gnu toolchain here cannot drive — so
// the standalone test is the only way this logic is ever executed by a test. Keeping the file
// free of `wie_util`, `arm32_cpu` and even `wie_util::Result` is what makes that possible:
// faults are reported as `Err(faulting_address)` and the caller maps them to `WieError`.
//
// Being `include!`d also means there is no module boundary and no cross-crate call: the
// accessors below inline into the interpreter's load/store path exactly as the hand-written
// code they replaced did.
//
// ── WHY LAZY ────────────────────────────────────────────────────────────────
// `map` used to allocate every page in the range up front, so `Allocator::init`'s
// `map(0x40000000, 0x10000000)` committed and zeroed the **whole 256 MB guest heap** before
// the first frame. Measured on the player's device at the title screen (`dumpsys meminfo`):
// native heap RSS 379 MB, process RSS 533 MB. That is why Android's low-memory killer picks
// this process first — `dumpsys activity exit-info` recorded `reason=3 (LOW_MEMORY)` — and a
// killed process is a lost run.
//
// Nothing needed those pages. Both guest allocators are already written to touch as little of
// the heap as they can (`BucketAllocator`'s bump index exists precisely so slots that were
// never handed out are never written; `ListAllocator::init` writes one header). So `map` now
// only records permission and a page is allocated on its **first write**.
//
// A read of a mapped page that was never written answers zero *without* allocating. That is
// not an approximation: an eagerly mapped page was `Box::new([0; PAGE_SIZE])`, so the bytes
// are identical — and it is what stops a scan of the heap (the KTF collector walks it) from
// re-materialising everything the lazy map just saved.

use alloc::{boxed::Box, vec::Vec};
// Spelled out because the module this is `include!`d into has `use wie_util::Result` in scope,
// and that alias fixes the error type — a bare `Result<T, E>` here would not compile there.
use core::result::Result as CoreResult;

pub const PAGE_SIZE: usize = 0x10000;
pub const PAGE_MASK: u32 = (PAGE_SIZE - 1) as u32;
/// 4 GB / 64 KB. The guest is 32-bit, so this covers the entire address space and every page
/// index derived from a `u32` is provably in range — hence no bounds check on the hot path.
pub const PAGE_COUNT: usize = (0x1_0000_0000u64 / PAGE_SIZE as u64) as usize;

/// A faulting access. Carries the address the guest asked for, which is what
/// `WieError::InvalidMemoryAccess` wants.
pub type Fault = u32;

pub struct PageTable {
    /// `Option<Box<_>>` is niche-packed to a nullable pointer, so this is one load and one
    /// null test per access.
    ///
    /// Boxed as a fixed-size ARRAY, not a slice: the length stays a compile-time constant, so
    /// an index derived from a `u32` by [`Self::page_index`] is still provably in range and the
    /// bounds check still optimises away. A `Box<[_]>` would keep a runtime length and put a
    /// real check on the interpreter's every load and store.
    pages: Box<[Option<Box<[u8; PAGE_SIZE]>>; PAGE_COUNT]>,
    /// One bit per page: "the guest may touch this address". Set by [`Self::map`], and the
    /// only thing [`Self::is_mapped`] consults.
    ///
    /// Deliberately separate from `pages`: once commit is lazy, the presence of a box no
    /// longer answers "is this mapped", and conflating the two would report the freshly mapped
    /// heap as unmapped until something wrote to it — which `ArmCore`'s guest-pointer checks
    /// would then reject.
    mapped: [u64; PAGE_COUNT / 64],
    committed: u64,
}

impl PageTable {
    pub fn new() -> Self {
        // Built through a `Vec` rather than `Box::new(array::from_fn(..))`: the latter
        // materialises the 512 KB table as a stack temporary first, which a non-optimised
        // build does not elide — it overflowed a 2 MB test thread outright, and on the device
        // it is 512 KB of the emulator thread's stack for no reason.
        let pages: Vec<Option<Box<[u8; PAGE_SIZE]>>> = (0..PAGE_COUNT).map(|_| None).collect();

        Self {
            pages: pages
                .into_boxed_slice()
                .try_into()
                .ok()
                .expect("exactly PAGE_COUNT elements were just collected"),
            mapped: [0; PAGE_COUNT / 64],
            committed: 0,
        }
    }

    #[inline]
    pub fn page_index(address: u32) -> usize {
        (address >> 16) as usize
    }

    /// Pages actually allocated. The gauge that says whether the lazy map is doing its job on
    /// a device, where the app cannot read its own `dumpsys meminfo`.
    pub fn committed_pages(&self) -> u64 {
        self.committed
    }

    #[inline]
    pub fn is_page_mapped(&self, index: usize) -> bool {
        self.mapped[index / 64] & (1 << (index % 64)) != 0
    }

    pub fn map(&mut self, address: u32, size: usize) {
        let page_start = address & !PAGE_MASK;
        let page_end = (address + size as u32 + PAGE_MASK) & !PAGE_MASK;

        for page in (page_start..page_end).step_by(PAGE_SIZE) {
            let index = page as usize / PAGE_SIZE;
            self.mapped[index / 64] |= 1 << (index % 64);
        }
    }

    pub fn is_mapped(&self, address: u32, size: usize) -> bool {
        let page_start = address & !PAGE_MASK;
        let page_end = (address + size as u32 + PAGE_MASK) & !PAGE_MASK;

        for page in (page_start..page_end).step_by(PAGE_SIZE) {
            if !self.is_page_mapped(page as usize / PAGE_SIZE) {
                return false;
            }
        }

        true
    }

    /// The page's bytes, or `Ok(None)` when it is mapped but has never been written (= all
    /// zeroes). `Err` only for an address the guest was never given.
    #[inline]
    pub fn page_for_read(&self, index: usize) -> CoreResult<Option<&[u8; PAGE_SIZE]>, ()> {
        match self.pages[index].as_deref() {
            Some(page) => Ok(Some(page)),
            None if self.is_page_mapped(index) => Ok(None),
            None => Err(()),
        }
    }

    /// The page's bytes for writing, allocating it if this is its first write.
    #[inline]
    pub fn page_for_write(&mut self, index: usize) -> CoreResult<&mut [u8; PAGE_SIZE], ()> {
        if self.pages[index].is_none() {
            if !self.is_page_mapped(index) {
                return Err(());
            }
            self.pages[index] = Some(Box::new([0; PAGE_SIZE]));
            self.committed += 1;
        }

        // Cannot be `None`: either it already was `Some`, or the branch above just filled it.
        Ok(self.pages[index].as_deref_mut().unwrap())
    }

    pub fn read_range(&self, address: u32, size: usize, result: &mut [u8]) -> CoreResult<usize, Fault> {
        let mut remaining_size = size;
        let mut current_address = address;

        while remaining_size > 0 {
            let page_address = current_address & !PAGE_MASK;
            let index = page_address as usize / PAGE_SIZE;
            let offset = (current_address - page_address) as usize;
            let available_bytes = (PAGE_SIZE - offset).min(remaining_size);
            let out = &mut result[size - remaining_size..size - remaining_size + available_bytes];

            match self.page_for_read(index) {
                Ok(Some(page)) => out.copy_from_slice(&page[offset..offset + available_bytes]),
                // Never written = zeroes. Written rather than skipped: `result` is the
                // caller's buffer and may hold anything.
                Ok(None) => out.fill(0),
                Err(()) => return Err(current_address),
            }

            remaining_size -= available_bytes;
            current_address += available_bytes as u32;
        }

        Ok(size)
    }

    pub fn write_range(&mut self, address: u32, data: &[u8]) -> CoreResult<(), Fault> {
        let mut current_address = address;
        let mut data_index = 0;

        while data_index < data.len() {
            let page_address = current_address & !PAGE_MASK;
            let index = page_address as usize / PAGE_SIZE;
            let offset = (current_address - page_address) as usize;
            let available_bytes = (PAGE_SIZE - offset).min(data.len() - data_index);

            let page = self.page_for_write(index).map_err(|()| current_address)?;
            page[offset..offset + available_bytes].copy_from_slice(&data[data_index..data_index + available_bytes]);

            data_index += available_bytes;
            current_address += available_bytes as u32;
        }

        Ok(())
    }
}
