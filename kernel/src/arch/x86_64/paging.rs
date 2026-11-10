//! Virtual Memory Management - x86_64 4-level paging

use core::sync::atomic::{AtomicU64, Ordering};
pub use x86_64::{PhysAddr, VirtAddr};
pub use x86_64::structures::paging::{
        PageTable, PageTableFlags, PhysFrame, Size4KiB, Mapper,
        OffsetPageTable, Page, FrameAllocator, Translate,
    };
use crate::mm::phys::allocate_frames;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr3Flags, Cr4, Cr4Flags};
use x86_64::registers::model_specific::{Efer, EferFlags};
use lazy_static::lazy_static;
use log::{info, trace, warn, error};
use alloc::vec::Vec;

/// Kernel page table base (PML4)
static KERNEL_PML4: AtomicU64 = AtomicU64::new(0);
/// Physical memory offset for direct mapping
pub static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Page table flags for kernel mappings
pub const KERNEL_PT_FLAGS: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::WRITABLE)
    .union(PageTableFlags::NO_EXECUTE);

/// Page table flags for user mappings
pub const USER_PT_FLAGS: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::WRITABLE)
    .union(PageTableFlags::USER_ACCESSIBLE)
    .union(PageTableFlags::NO_EXECUTE);

/// Page table flags for device mappings
pub const DEVICE_PT_FLAGS: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::WRITABLE)
    .union(PageTableFlags::NO_EXECUTE)
    .union(PageTableFlags::NO_CACHE);

/// Higher half kernel base
pub const KERNEL_BASE: u64 = 0xFFFF_8000_0000_0000;
/// Kernel heap base
pub const KERNEL_HEAP_BASE: u64 = 0xFFFF_8000_1000_0000;
/// Kernel heap size (1 GB)
pub const KERNEL_HEAP_SIZE: u64 = 1024 * 1024 * 1024;
/// Per-CPU area base
pub const PERCPU_BASE: u64 = 0xFFFF_FF80_0000_0000;
/// Module load base
pub const MODULE_BASE: u64 = 0xFFFF_8100_0000_0000;
/// User space base
pub const USER_BASE: u64 = 0x0000_0000_0000_0000;
/// User space top (128 TB)
pub const USER_TOP: u64 = 0x0000_7FFF_FFFF_F000;

lazy_static! {
    /// Kernel page table mapper
    pub static ref KERNEL_MAPPER: OffsetPageTable<'static> = {
        let phys_mem_offset = PHYS_MEM_OFFSET.load(Ordering::SeqCst);
        unsafe { OffsetPageTable::new(
            &mut *(KERNEL_PML4.load(Ordering::SeqCst) as *mut PageTable),
            VirtAddr::new(phys_mem_offset)
        )}
    };
}

/// Initialize paging
pub fn init(memory_map: &[bloader_shared::MemoryRegion]) {
    info!("Initializing virtual memory management...");

    // Find physical memory offset (usually 0 for identity mapping in lower half)
    // We'll use the direct mapping approach: virtual = physical + PHYS_MEM_OFFSET
    let phys_offset = find_physical_memory_offset(memory_map);
    PHYS_MEM_OFFSET.store(phys_offset, Ordering::SeqCst);
    info!("Physical memory offset: 0x{:x}", phys_offset);

    // Get current PML4 from CR3
    let pml4_addr = Cr3::read().0.start_address().as_u64();
    KERNEL_PML4.store(pml4_addr, Ordering::SeqCst);
    info!("Current PML4: 0x{:x}", pml4_addr);

    // Drop the coarse early-boot alias before building precise mappings:
    // entry.asm maps low 4GB at PML4[256] with four 1GB pages, and map_to
    // cannot split huge pages (heap/APIC/IOAPIC mappings would fail).
    teardown_early_alias();

    // Set up kernel higher-half mapping
    setup_kernel_mapping();

    // Set up direct physical mapping
    setup_direct_mapping(memory_map);

    // Enable NX bit and other features
    enable_paging_features();

    info!("Virtual memory initialized");
}

/// Find physical memory offset for direct mapping
fn find_physical_memory_offset(memory_map: &[bloader_shared::MemoryRegion]) -> u64 {
    // Look for the first conventional memory region
    for region in memory_map {
        if region.region_type == bloader_shared::MemoryType::Conventional
            && region.base_addr < 0x100000000
        {
            // Use 0 offset for identity mapping of low memory
            // Higher half will be at KERNEL_BASE + physical
            return 0;
        }
    }
    0
}

/// Drop the coarse early-boot higher-half alias (PML4[256] -> four 1GB
/// pages covering 0xFFFF800000000000..0xFFFF800100000000).
///
/// Must run while RIP/RSP and all touched data live in identity-mapped low
/// memory (true throughout early_init): afterwards every higher-half address
/// needs an explicit mapping. Callers after this point (heap, APIC) map what
/// they need with 4K pages.
fn teardown_early_alias() {
    let pml4 = unsafe { &mut *(KERNEL_PML4.load(Ordering::SeqCst) as *mut PageTable) };
    pml4[256].set_unused();
    // Evict any cached translations of the removed 1GB pages.
    x86_64::instructions::tlb::flush_all();
    info!("Early higher-half alias removed");
}

/// Set up kernel higher-half mapping
fn setup_kernel_mapping() {
    // Map kernel text/data/bss to higher half
    // This is typically done by the bootloader, but we verify and extend

    // Map first 2MB of kernel (text + rodata + data + bss)
    let kernel_start = PhysAddr::new(0x100000); // 1MB
    let kernel_virt = VirtAddr::new(KERNEL_BASE + 0x100000);
    let size = 2 * 1024 * 1024; // 2MB

    map_range(
        kernel_virt,
        kernel_start,
        size,
        KERNEL_PT_FLAGS,
    );

    // Set up recursive mapping for page table walking
    // PML4[511] points to PML4 itself
    let mapper = unsafe { &mut *(KERNEL_PML4.load(Ordering::SeqCst) as *mut PageTable) };
    mapper[511].set_addr(PhysAddr::new(KERNEL_PML4.load(Ordering::SeqCst)), KERNEL_PT_FLAGS);

    info!("Kernel mapping established");
}

/// Set up direct physical mapping (physmap)
fn setup_direct_mapping(memory_map: &[bloader_shared::MemoryRegion]) {
    // entry.asm already identity-maps the first 4 GiB using huge pages.
    // Only map the part of each region beyond that boundary.
    const EARLY_MAP_END: u64 = 1 << 32;
    for region in memory_map {
        if region.region_type != bloader_shared::MemoryType::Conventional
            && region.region_type != bloader_shared::MemoryType::PersistentMemory
        {
            continue;
        }
        let start = region.base_addr.max(EARLY_MAP_END);
        let end = region.base_addr + region.length;
        if start < end {
            map_range(VirtAddr::new(start), PhysAddr::new(start), end - start, KERNEL_PT_FLAGS);
        }
    }

    info!("Direct physical mapping established");
}

/// Map a range of memory
fn map_range(
    virt_start: VirtAddr,
    phys_start: PhysAddr,
    size: u64,
    flags: PageTableFlags,
) {
    let mut mapper = get_kernel_mapper();
    let mut frame_allocator = KernelFrameAllocator;
    let start_page = Page::<Size4KiB>::containing_address(virt_start);
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_start.as_u64() + size - 1));
    let mut frame = PhysFrame::containing_address(phys_start);
    for page in Page::<Size4KiB>::range_inclusive(start_page, end_page) {
        unsafe {
            let _ = mapper.map_to(page, frame, flags, &mut frame_allocator).map(|f| f.flush());
        }
        frame += 1;
    }
}

/// Enable paging features (NX, SMEP, SMAP, etc.)
fn enable_paging_features() {
    // Enable NX (No Execute) via EFER
    unsafe {
        Efer::update(|efer| efer.insert(EferFlags::NO_EXECUTE_ENABLE));
    }

    // Enable SMEP (Supervisor Mode Execution Prevention) and SMAP (Supervisor Mode Access Prevention)
    unsafe {
        Cr4::update(|cr4| {
            cr4.insert(
                Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION
                    | Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION,
            );
        });
    }

    // Enable WP (Write Protect) in CR0
    unsafe {
        Cr0::update(|cr0| {
            cr0.insert(Cr0Flags::WRITE_PROTECT);
        });
    }

    info!("Paging features enabled: NX, SMEP, SMAP, WP");
}

/// Map a virtual page to a physical frame
pub fn map_to(
    virt: VirtAddr,
    phys: PhysAddr,
    flags: PageTableFlags,
) -> Result<(), MapError> {
    let mut mapper = get_kernel_mapper();
    let page = Page::<Size4KiB>::containing_address(virt);
    let frame = PhysFrame::containing_address(phys);

    let mut frame_allocator = KernelFrameAllocator;

    unsafe {
        mapper.map_to(page, frame, flags, &mut frame_allocator)
            .map_err(|_| MapError::MappingFailed)?
            .flush();
    }

    Ok(())
}

/// Unmap a virtual page
pub fn unmap(virt: VirtAddr) -> Result<PhysFrame<Size4KiB>, MapError> {
    let mut mapper = get_kernel_mapper();
    let page = Page::<Size4KiB>::containing_address(virt);

    let (frame, flush) = mapper.unmap(page).map_err(|_| MapError::UnmapFailed)?;
    flush.flush();

    Ok(frame)
}

/// Translate virtual address to physical
pub fn translate(virt: VirtAddr) -> Option<PhysAddr> {
    let mapper = get_kernel_mapper();
    mapper.translate_addr(virt)
}

/// Get kernel mapper instance
fn get_kernel_mapper() -> OffsetPageTable<'static> {
    let phys_mem_offset = PHYS_MEM_OFFSET.load(Ordering::SeqCst);
    unsafe { OffsetPageTable::new(
        &mut *(KERNEL_PML4.load(Ordering::SeqCst) as *mut PageTable),
        VirtAddr::new(phys_mem_offset)
    )}
}

/// Kernel frame allocator for page table operations
struct KernelFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for KernelFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        allocate_frames(1).map(|range| range.start_frame())
    }
}

/// Map error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    MappingFailed,
    UnmapFailed,
    InvalidAddress,
    OutOfMemory,
}

/// Handle page fault
pub fn handle_page_fault(addr: VirtAddr, error_code: PageFaultErrorCode) -> Result<(), &'static str> {
    // Check if it's a kernel address
    if addr.as_u64() >= KERNEL_BASE {
        return handle_kernel_page_fault(addr, error_code);
    }

    // User space page fault - delegate to process manager
    crate::proc::handle_user_page_fault(addr, error_code)
}

fn handle_kernel_page_fault(addr: VirtAddr, error_code: PageFaultErrorCode) -> Result<(), &'static str> {
    // Check for demand paging of kernel modules, etc.
    if addr.as_u64() >= MODULE_BASE && addr.as_u64() < MODULE_BASE + 1024*1024*1024 {
        // Module lazy loading
        return Err("Module demand paging not implemented");
    }

    // Check for kernel heap expansion
    if addr.as_u64() >= KERNEL_HEAP_BASE && addr.as_u64() < KERNEL_HEAP_BASE + KERNEL_HEAP_SIZE {
        return expand_kernel_heap(addr);
    }

    Err("Unhandled kernel page fault")
}

fn expand_kernel_heap(addr: VirtAddr) -> Result<(), &'static str> {
    // Allocate a new frame and map it
    let page = Page::<Size4KiB>::containing_address(addr);
    let mut frame_allocator = KernelFrameAllocator;

    let mut mapper = get_kernel_mapper();
    unsafe { mapper.map_to(page, frame_allocator.allocate_frame().ok_or("OOM")?, USER_PT_FLAGS, &mut frame_allocator) }
        .map_err(|_| "Mapping failed")?
        .flush();

    Ok(())
}

/// Create a new address space for a process
pub fn create_address_space() -> AddressSpace {
    let pml4_range = allocate_frames(1).expect("Failed to allocate PML4 frame");
    let pml4_frame = pml4_range.start_frame();
    let pml4_addr = pml4_frame.start_address().as_u64();

    // Zero the new PML4
    unsafe {
        core::ptr::write_bytes(pml4_addr as *mut u8, 0, 4096);
    }

    // Map kernel space into new address space (higher half)
    let new_pml4 = unsafe { &mut *(pml4_addr as *mut PageTable) };
    let kernel_pml4 = unsafe { &*(KERNEL_PML4.load(Ordering::SeqCst) as *const PageTable) };

    // Copy kernel mappings (entries 256-511 for higher half)
    for i in 256..511 {
        new_pml4[i] = kernel_pml4[i].clone();
    }

    AddressSpace {
        pml4_frame,
        pml4_addr,
    }
}

/// Process address space
#[derive(Debug)]
pub struct AddressSpace {
    pml4_frame: PhysFrame<Size4KiB>,
    pml4_addr: u64,
}

impl Clone for AddressSpace {
    fn clone(&self) -> Self { self.clone_as() }
}

impl AddressSpace {
    /// Get PML4 physical address
    pub fn pml4_phys(&self) -> PhysAddr {
        PhysAddr::new(self.pml4_addr)
    }

    /// Get PML4 virtual address
    pub fn pml4_virt(&self) -> VirtAddr {
        VirtAddr::new(self.pml4_addr + PHYS_MEM_OFFSET.load(Ordering::SeqCst))
    }

    /// Switch to this address space
    pub fn activate(&self) {
        unsafe { Cr3::write(self.pml4_frame, Cr3::read().1) };
    }

    /// Map a page in this address space
    pub fn map(&mut self, virt: VirtAddr, phys: PhysAddr, flags: PageTableFlags) -> Result<(), MapError> {
        let mut mapper = unsafe {
            OffsetPageTable::new(
                &mut *(self.pml4_addr as *mut PageTable),
                VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst)),
            )
        };

        let page = Page::<Size4KiB>::containing_address(virt);
        let frame = PhysFrame::containing_address(phys);
        let mut frame_allocator = KernelFrameAllocator;

        unsafe {
            mapper.map_to(page, frame, flags, &mut frame_allocator)
                .map_err(|_| MapError::MappingFailed)?
                .flush();
        }

        Ok(())
    }

    /// Unmap a page
    pub fn unmap(&mut self, virt: VirtAddr) -> Result<(), MapError> {
        let mut mapper = unsafe {
            OffsetPageTable::new(
                &mut *(self.pml4_addr as *mut PageTable),
                VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst)),
            )
        };

        let page = Page::<Size4KiB>::containing_address(virt);
        let (_frame, flush) = mapper.unmap(page).map_err(|_| MapError::UnmapFailed)?;
        flush.flush();

        Ok(())
    }

    /// Translate virtual address
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let mapper = unsafe {
            OffsetPageTable::new(
                &mut *(self.pml4_addr as *mut PageTable),
                VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst)),
            )
        };
        mapper.translate_addr(virt)
    }

    /// Clone address space (for fork)
    pub fn clone_as(&self) -> AddressSpace {
        let new_pml4_range = allocate_frames(1).expect("Failed to allocate PML4 for clone");
        let new_pml4_frame = new_pml4_range.start_frame();
        let new_pml4_addr = new_pml4_frame.start_address().as_u64();

        // Copy PML4
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.pml4_addr as *const u8,
                new_pml4_addr as *mut u8,
                4096,
            );
        }

        // TODO: Copy lower page tables for user space (copy-on-write)

        AddressSpace {
            pml4_frame: new_pml4_frame,
            pml4_addr: new_pml4_addr,
        }
    }
}

/// Map a frame with specific flags (for framebuffer, etc.)
pub fn map_frame(
    virt: VirtAddr,
    frame: PhysFrame<Size4KiB>,
    flags: PageTableFlags,
) -> Result<(), MapError> {
    let mut mapper = get_kernel_mapper();
    let page = Page::<Size4KiB>::containing_address(virt);
    let mut frame_allocator = KernelFrameAllocator;

    unsafe {
        mapper.map_to(page, frame, flags, &mut frame_allocator)
            .map_err(|_| MapError::MappingFailed)?
            .flush();
    }

    Ok(())
}

/// Identity map a range (virtual = physical)
pub fn identity_map_range(
    start: PhysAddr,
    size: u64,
    flags: PageTableFlags,
) -> Result<(), MapError> {
    let virt = VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst) + start.as_u64());
    map_range(
        virt,
        start,
        size,
        flags,
    );
    Ok(())
}

/// Get physical memory offset
pub fn phys_mem_offset() -> u64 {
    PHYS_MEM_OFFSET.load(Ordering::SeqCst)
}

/// Convert physical to virtual (kernel direct map)
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst) + phys.as_u64())
}

/// Convert virtual to physical (kernel direct map)
pub fn virt_to_phys(virt: VirtAddr) -> Option<PhysAddr> {
    if virt.as_u64() >= PHYS_MEM_OFFSET.load(Ordering::SeqCst) {
        Some(PhysAddr::new(virt.as_u64() - PHYS_MEM_OFFSET.load(Ordering::SeqCst)))
    } else {
        translate(virt)
    }
}

/// Page fault error code
pub type PageFaultErrorCode = x86_64::structures::idt::PageFaultErrorCode;

/// Flush TLB for a specific address
pub fn flush_tlb(addr: VirtAddr) {
    x86_64::instructions::tlb::flush(addr);
}

/// Flush entire TLB
pub fn flush_tlb_all() {
    x86_64::instructions::tlb::flush_all();
}