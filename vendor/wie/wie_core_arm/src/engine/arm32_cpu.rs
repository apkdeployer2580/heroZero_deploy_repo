use alloc::format;

use arm32_cpu::{Cpu, Memory, Mode, reg};

use wie_util::{Result, WieError};

use crate::engine::{ArmEngine, ArmRegister, EngineRunResult, MemoryPermission};

pub struct Arm32CpuEngine {
    cpu: Cpu,
    mem: PageTable,
}

/// How a `run_counted` batch left its loop.
///
/// The SVC case cannot carry its `EngineRunResult` out of the loop: decoding the
/// SVC number re-reads guest memory through `&mut self`, and inside the loop
/// `self.mem` is exclusively borrowed by the batch-wide memory adapter (see
/// `run_counted`). So the loop only reports *that* an SVC was taken and the
/// decode happens after the adapter's borrow ends.
enum BatchExit {
    Done(EngineRunResult),
    Svc,
}

impl Arm32CpuEngine {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            mem: PageTable::new(),
        }
    }

    fn read_svc_result(&mut self) -> Result<EngineRunResult> {
        let lr = self.cpu.reg_get(Mode::Supervisor, reg::LR);
        let spsr = self.cpu.reg_get(Mode::Supervisor, reg::SPSR);

        let svc_address = lr.checked_sub(2).ok_or(WieError::InvalidMemoryAccess(lr))?;
        let mut svc_bytes = [0u8; 2];
        self.mem
            .read_range(svc_address, 2, &mut svc_bytes)
            .map_err(WieError::InvalidMemoryAccess)?;
        let instruction = u16::from_le_bytes(svc_bytes);
        if instruction & 0xff00 != 0xdf00 {
            return Err(WieError::FatalError(format!("Invalid Thumb SVC instruction {instruction:#06x}")));
        }

        let category = instruction as u32 & 0xff;

        Ok(EngineRunResult::Svc { category, lr, spsr })
    }
}

impl ArmEngine for Arm32CpuEngine {
    fn run(&mut self, end: u32, count: u32) -> Result<EngineRunResult> {
        // TEMPORARY measurement: charge the batch to `wie_util::stats` once, on
        // the way out, so counting costs one atomic per batch (not per step).
        // `executed` is the number of instructions that actually retired, on every
        // exit path including the error ones — same value the previous
        // `budget - remaining` out-param produced.
        let (result, executed) = self.run_counted(end, count);
        wie_util::stats::add_instructions(executed as u64);
        // Committed-page gauge, published once per batch rather than per commit: it is a
        // level, not a rate, and `PageTable` is deliberately dependency-free so it cannot
        // reach `wie_util` itself. See the lazy-map note in `page_table.rs`.
        wie_util::stats::set_guest_pages(self.mem.committed_pages());
        if let Ok(EngineRunResult::Svc { category, .. }) = &result {
            wie_util::stats::add_svc();
            // `r12` is where `ArmCore::make_svc_stub` parks the function id and where
            // `SvcIdParam` reads it back, so it is still the id here — the exception
            // does not bank r12. One site catches every dispatch path (init / wipic /
            // stdlib / binary-patch hooks) instead of instrumenting each handler.
            wie_util::stats::add_svc_detail(*category, ArmEngine::reg_read(self, ArmRegister::IP));
        }
        result
    }

    fn reg_write(&mut self, reg: ArmRegister, value: u32) {
        if reg == ArmRegister::PC && value % 2 == 1 {
            self.cpu.reg_set(Mode::User, reg.into_armv4t(), value - 1);

            let cpsr = self.cpu.reg_get(Mode::User, reg::CPSR);
            self.cpu.reg_set(Mode::User, reg::CPSR, cpsr | (1 << 5)); // T bit

            return;
        }
        self.cpu.reg_set(Mode::User, reg.into_armv4t(), value);
    }

    fn reg_read(&self, reg: ArmRegister) -> u32 {
        self.cpu.reg_get(Mode::User, reg.into_armv4t())
    }

    fn mem_map(&mut self, address: u32, size: usize, _permission: MemoryPermission) {
        self.mem.map(address, size);
    }

    fn mem_write(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.mem.write_range(address, data).map_err(WieError::InvalidMemoryAccess)
    }

    fn mem_read(&mut self, address: u32, size: usize, result: &mut [u8]) -> Result<usize> {
        self.mem.read_range(address, size, result).map_err(WieError::InvalidMemoryAccess)
    }

    fn is_mapped(&self, address: u32, size: usize) -> bool {
        self.mem.is_mapped(address, size)
    }
}

impl Arm32CpuEngine {
    /// Runs at most `budget` guest instructions. Returns the outcome plus how many
    /// instructions actually retired (for the `wie_util::stats` counters).
    fn run_counted(&mut self, end: u32, budget: u32) -> (Result<EngineRunResult>, u32) {
        let mut remaining = budget;

        // Split-borrow the two fields so the memory adapter can be built ONCE for
        // the whole batch instead of once per instruction. `cpu` and `mem` are
        // disjoint fields, so the borrow checker accepts holding both.
        //
        // That borrow is also what makes the batch-wide adapter safe: while it
        // lives, `&mut self` is unavailable, so `mem_map` (the only thing that can
        // add pages) provably cannot run inside the loop. Nothing reached from
        // `Cpu::step` calls back into the engine either — it only goes through the
        // `Memory` trait, which touches `PageTable` and nothing else.
        //
        // Scoped to a block rather than merely `drop`ped so the borrow of
        // `self.mem` ends lexically — `read_svc_result` needs `&mut self` back.
        let exit = {
            let Arm32CpuEngine { cpu, mem } = &mut *self;
            let mut memory = mem.as_arm32cpu_memory();

            loop {
                let pc = cpu.reg_get(Mode::User, reg::PC);

                // SVC entry = the ARM vector at 0x08 with the core already in
                // Supervisor mode. Reuse the `pc` we just read (the old
                // `is_svc_exception` re-read it), and read CPSR only after the
                // address test — that test is false on all but a handful of
                // instructions per batch, so CPSR's register-file lookup is
                // skipped almost always. Same predicate, evaluated lazily.
                if pc == 0x08 && (cpu.reg_get(Mode::User, reg::CPSR) & 0x1f) == 0x13 {
                    break BatchExit::Svc;
                }

                if pc < 0x1000 {
                    return (Err(WieError::InvalidMemoryAccess(pc)), budget - remaining);
                }

                if pc == end {
                    break BatchExit::Done(EngineRunResult::End);
                }

                if remaining == 0 {
                    break BatchExit::Done(EngineRunResult::CountExhausted);
                }

                if !(cpu.step(&mut memory)) {
                    return (Err(WieError::FatalError("Undefined instruction".into())), budget - remaining);
                }
                remaining -= 1;

                // Checked after every single step, exactly as before: moving this
                // out of the loop would report a bad access several instructions
                // late (and let the guest keep running on garbage in between).
                //
                // The adapter now outlives the instruction that set the flag, but
                // that changes nothing: the only writer is the fault path, and
                // reaching it returns immediately, so the flag can never be
                // observed stale.
                if let Some(x) = memory.memory_error() {
                    return (Err(WieError::InvalidMemoryAccess(x)), budget - remaining);
                }
            }
        };

        let executed = budget - remaining;
        match exit {
            BatchExit::Done(x) => (Ok(x), executed),
            BatchExit::Svc => (self.read_svc_result(), executed),
        }
    }
}

impl ArmRegister {
    fn into_armv4t(self) -> u8 {
        match self {
            ArmRegister::R0 => 0,
            ArmRegister::R1 => 1,
            ArmRegister::R2 => 2,
            ArmRegister::R3 => 3,
            ArmRegister::R4 => 4,
            ArmRegister::R5 => 5,
            ArmRegister::R6 => 6,
            ArmRegister::R7 => 7,
            ArmRegister::R8 => 8,
            ArmRegister::SB => 9,
            ArmRegister::SL => 10,
            ArmRegister::FP => 11,
            ArmRegister::IP => 12,
            ArmRegister::SP => reg::SP,
            ArmRegister::LR => reg::LR,
            ArmRegister::PC => reg::PC,
            ArmRegister::Cpsr => reg::CPSR,
        }
    }
}

// The guest page table, with lazy commit. `include!` rather than `mod` so the repo-root
// `page_table_selftest.rs` can execute this exact source on a host where `cargo test` cannot
// build this workspace — see the file's own header. Being textually inlined also means the
// accessors below are ordinary same-module calls on the interpreter's hot path.
include!("page_table.rs");

impl PageTable {
    fn as_arm32cpu_memory(&mut self) -> Arm32CpuMemory<'_> {
        Arm32CpuMemory::new(self)
    }
}

struct Arm32CpuMemory<'a> {
    emulated_memory: &'a mut PageTable,
    // Plain field, not a `RefCell`. Every `Memory` trait method already takes
    // `&mut self`, so the interior mutability bought nothing while charging a
    // borrow-flag load/store (and an unreachable panic path) on the fault check
    // that runs after every single guest instruction.
    memory_error: Option<u32>,
}

impl<'a> Arm32CpuMemory<'a> {
    fn new(emulated_memory: &'a mut PageTable) -> Self {
        Self {
            emulated_memory,
            memory_error: None,
        }
    }

    fn memory_error(&self) -> Option<u32> {
        self.memory_error
    }

    // No page cache here on purpose. The obvious "remember the last page pointer"
    // trick needs a raw pointer kept across reborrows of `*self.emulated_memory`,
    // which is exactly the aliasing pattern LLVM's `noalias` on `&mut` is allowed
    // to miscompile — and the lookup it would replace is already three
    // instructions: `addr >> 16`, one load from the page table (`Option<Box<_>>`
    // is niche-packed to a nullable pointer), one null test. The index is provably
    // in range so there is no bounds check. Not worth the unsoundness risk.
    //
    // Split into a read and a write half by the lazy-commit change: a read of a mapped page
    // that has never been written answers zero WITHOUT allocating (otherwise any scan of the
    // heap materialises all 256 MB again), while a write to one has to allocate. Both return
    // `None` for "no bytes to touch"; only the write half distinguishes that from a fault,
    // because for a read the zeroes and the fault produce the same value and differ solely in
    // whether `memory_error` is recorded.
    #[inline]
    fn read_page(&mut self, addr: u32) -> Option<&[u8; PAGE_SIZE]> {
        match self.emulated_memory.page_for_read(PageTable::page_index(addr)) {
            Ok(page) => page,
            Err(()) => {
                self.memory_error = Some(addr);
                None
            }
        }
    }

    #[inline]
    fn write_page(&mut self, addr: u32) -> Option<&mut [u8; PAGE_SIZE]> {
        match self.emulated_memory.page_for_write(PageTable::page_index(addr)) {
            Ok(page) => Some(page),
            Err(()) => {
                self.memory_error = Some(addr);
                None
            }
        }
    }
}

impl Memory for Arm32CpuMemory<'_> {
    fn r8(&mut self, addr: u32) -> u8 {
        let offset = addr & PAGE_MASK;

        let page = self.read_page(addr);
        if page.is_none() {
            return 0;
        }

        let data = page.unwrap();

        data[offset as usize]
    }

    fn r16(&mut self, addr: u32) -> u16 {
        let offset = addr & PAGE_MASK;

        let page = self.read_page(addr);
        if page.is_none() {
            return 0;
        }

        let data = page.unwrap();

        (data[offset as usize] as u16) | ((data[offset as usize + 1] as u16) << 8)
    }

    fn r32(&mut self, addr: u32) -> u32 {
        let offset = addr & PAGE_MASK;

        let page = self.read_page(addr);
        if page.is_none() {
            return 0;
        }

        let data = page.unwrap();
        (data[offset as usize] as u32)
            | ((data[offset as usize + 1] as u32) << 8)
            | ((data[offset as usize + 2] as u32) << 16)
            | ((data[offset as usize + 3] as u32) << 24)
    }

    fn w8(&mut self, addr: u32, val: u8) {
        let offset = addr & PAGE_MASK;

        let page = self.write_page(addr);
        if page.is_none() {
            return;
        }

        let data = page.unwrap();

        data[offset as usize] = val;
    }

    fn w16(&mut self, addr: u32, val: u16) {
        let offset = addr & PAGE_MASK;

        let page = self.write_page(addr);
        if page.is_none() {
            return;
        }

        let data = page.unwrap();

        data[offset as usize] = val as u8;
        data[offset as usize + 1] = (val >> 8) as u8;
    }

    fn w32(&mut self, addr: u32, val: u32) {
        let offset = addr & PAGE_MASK;

        let page = self.write_page(addr);
        if page.is_none() {
            return;
        }

        let data = page.unwrap();

        data[offset as usize] = val as u8;
        data[offset as usize + 1] = (val >> 8) as u8;
        data[offset as usize + 2] = (val >> 16) as u8;
        data[offset as usize + 3] = (val >> 24) as u8;
    }
}

#[cfg(test)]
mod tests {
    use arm32_cpu::Memory;

    use super::PageTable;

    #[test]
    fn test_memory_basic() {
        let mut memory = PageTable::new();

        memory.map(0x10000, 0x1000);
        memory.map(0x11000, 0x1000);
        memory.map(0x20000, 0x10000);

        memory.write_range(0x10000, &[123; 0x1000]).unwrap();

        let mut buf = [0; 0x1000];
        memory.read_range(0x10000, 0x1000, &mut buf).unwrap();
        assert_eq!(buf, [123; 0x1000]);

        memory.write_range(0x10900, &[100; 0x1000]).unwrap();

        memory.read_range(0x10900, 0x1000, &mut buf).unwrap();
        assert_eq!(buf, [100; 0x1000]);

        let mut arm32cpu_memory = memory.as_arm32cpu_memory();

        let r8 = arm32cpu_memory.r8(0x10000);
        assert_eq!(r8, 123);

        let r16 = arm32cpu_memory.r16(0x10000);
        assert_eq!(r16, 123 | (123 << 8));

        let r32 = arm32cpu_memory.r32(0x10000);
        assert_eq!(r32, 123 | (123 << 8) | (123 << 16) | (123 << 24));

        arm32cpu_memory.w8(0x10000, 12);
        let r8 = arm32cpu_memory.r8(0x10000);
        assert_eq!(r8, 12);

        arm32cpu_memory.w16(0x10000, 0x1234);
        let r16 = arm32cpu_memory.r16(0x10000);
        assert_eq!(r16, 0x1234);

        arm32cpu_memory.w32(0x10000, 0x12345678);
        let r32 = arm32cpu_memory.r32(0x10000);
        assert_eq!(r32, 0x12345678);
    }

    #[test]
    fn test_memory_unmapped_read() {
        let mut memory = PageTable::new();

        memory.map(0x10000, 0x10000);

        let mut buf = [0; 0x1000];
        assert!(memory.read_range(0x1f500, 0x1000, &mut buf).is_err());
    }

    /// The adapter is now built once per `run` batch instead of once per guest
    /// instruction, so `memory_error` outlives the instruction that raised it.
    /// `run_counted` relies on the flag being sticky (it returns immediately on
    /// the first fault, so a stale flag can never be observed) — pin that down.
    #[test]
    fn test_adapter_memory_error_is_sticky() {
        let mut memory = PageTable::new();
        memory.map(0x10000, 0x10000);

        let mut adapter = memory.as_arm32cpu_memory();
        assert_eq!(adapter.memory_error(), None);

        // Faulting accesses read as zero and record the address.
        assert_eq!(adapter.r32(0x50000), 0);
        assert_eq!(adapter.memory_error(), Some(0x50000));

        // A later good access must not silently clear it.
        adapter.w8(0x10000, 7);
        assert_eq!(adapter.r8(0x10000), 7);
        assert_eq!(adapter.memory_error(), Some(0x50000));
    }

    #[test]
    fn test_memory_unmapped_write() {
        let mut memory = PageTable::new();

        memory.map(0x10000, 0x10000);

        assert!(memory.write_range(0x1f500, &[12; 0x1000]).is_err());
    }

    /// The point of lazy commit: `map` must cost nothing. A 256 MB range is 4096 pages, so an
    /// eager `map` would allocate 256 MB here.
    #[test]
    fn map_commits_nothing_until_written() {
        let mut memory = PageTable::new();
        memory.map(0x40000000, 0x10000000);

        assert_eq!(memory.committed_pages(), 0);
        // Mapped is mapped, box or no box — `ArmCore::read_null_terminated_string` and the
        // guest-pointer validity checks in `core.rs` gate on exactly this.
        assert!(memory.is_mapped(0x40000000, 0x10000000));

        memory.write_range(0x40000000, &[1, 2, 3, 4]).unwrap();
        assert_eq!(memory.committed_pages(), 1);
    }

    /// A read of a mapped page that was never written answers zero and does NOT commit —
    /// otherwise a heap scan (the KTF collector walks free space) re-materialises everything
    /// the lazy map just saved.
    #[test]
    fn reading_an_uncommitted_page_is_zero_and_allocates_nothing() {
        let mut memory = PageTable::new();
        memory.map(0x40000000, 0x20000);

        let mut buf = [0xffu8; 0x100];
        memory.read_range(0x40000000, 0x100, &mut buf).unwrap();
        assert_eq!(buf, [0u8; 0x100]);

        let mut adapter = memory.as_arm32cpu_memory();
        assert_eq!(adapter.r32(0x40010000), 0);
        assert_eq!(adapter.r8(0x40010004), 0);
        // Reading zeroes from a mapped page is not a fault.
        assert_eq!(adapter.memory_error(), None);

        assert_eq!(memory.committed_pages(), 0);
    }

    /// Writing through the CPU adapter has to commit, and what is read back afterwards must be
    /// what was written — i.e. the commit is wired into the same page the reader looks at.
    #[test]
    fn adapter_write_commits_and_round_trips() {
        let mut memory = PageTable::new();
        memory.map(0x40000000, 0x10000);

        let mut adapter = memory.as_arm32cpu_memory();
        adapter.w32(0x40001000, 0xdeadbeef);
        assert_eq!(adapter.r32(0x40001000), 0xdeadbeef);
        assert_eq!(adapter.memory_error(), None);

        assert_eq!(memory.committed_pages(), 1);

        // And the non-adapter path sees the same bytes.
        let mut buf = [0u8; 4];
        memory.read_range(0x40001000, 4, &mut buf).unwrap();
        assert_eq!(u32::from_le_bytes(buf), 0xdeadbeef);
    }

    /// An unmapped write still faults rather than quietly mapping the page — the lazy path is
    /// "commit what `map` permitted", not "map on demand".
    #[test]
    fn adapter_write_to_unmapped_page_faults() {
        let mut memory = PageTable::new();
        memory.map(0x10000, 0x10000);

        let mut adapter = memory.as_arm32cpu_memory();
        adapter.w8(0x50000, 7);
        assert_eq!(adapter.memory_error(), Some(0x50000));
        assert_eq!(memory.committed_pages(), 0);
    }
}
