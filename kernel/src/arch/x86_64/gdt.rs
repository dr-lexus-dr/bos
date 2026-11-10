//! Global Descriptor Table (GDT) for x86_64

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, FS, GS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::PrivilegeLevel;
use lazy_static::lazy_static;

/// GDT entry count
const GDT_ENTRIES: usize = 7;

/// Kernel code segment selector
pub const KERNEL_CS: SegmentSelector = SegmentSelector::new(1, PrivilegeLevel::Ring0);
/// Kernel data segment selector
pub const KERNEL_DS: SegmentSelector = SegmentSelector::new(2, PrivilegeLevel::Ring0);
/// User code segment selector (64-bit)
pub const USER_CS64: SegmentSelector = SegmentSelector::new(3, PrivilegeLevel::Ring3);
/// User data segment selector
pub const USER_DS: SegmentSelector = SegmentSelector::new(4, PrivilegeLevel::Ring3);
/// User code segment selector (32-bit compat)
pub const USER_CS32: SegmentSelector = SegmentSelector::new(5, PrivilegeLevel::Ring3);
/// TSS selector
pub const KERNEL_TSS: SegmentSelector = SegmentSelector::new(6, PrivilegeLevel::Ring0);

lazy_static! {
    static ref GDT: GlobalDescriptorTable = {
        let mut gdt = GlobalDescriptorTable::new();
        gdt.append(Descriptor::kernel_code_segment());      // 1: Kernel CS
        gdt.append(Descriptor::kernel_data_segment());      // 2: Kernel DS
        gdt.append(Descriptor::user_code_segment());        // 3: User CS64
        gdt.append(Descriptor::user_data_segment());        // 4: User DS
        gdt.append(Descriptor::kernel_code_segment());      // 5: User CS32 (compat)
        gdt.append(Descriptor::tss_segment(&TSS));          // 6: TSS
        gdt
    };

    static ref TSS: x86_64::structures::tss::TaskStateSegment = {
        let mut tss = x86_64::structures::tss::TaskStateSegment::new();
        // Separate stack for synchronous transitions from user mode.
        #[repr(align(16))]
        struct UserTrapStack([u8; 32 * 1024]);
        static mut USER_TRAP_STACK: UserTrapStack = UserTrapStack([0; 32 * 1024]);
        let stack = unsafe { core::ptr::addr_of!(USER_TRAP_STACK.0) as u64 };
        tss.privilege_stack_table[0] = x86_64::VirtAddr::new(stack + 32 * 1024);
        // Set up IST (Interrupt Stack Table) for double fault, NMI, etc.
        tss.interrupt_stack_table[0] = {
            static mut IST_STACK: [u8; 4096 * 4] = [0; 4096 * 4];
            let stack_end = unsafe { &IST_STACK as *const _ as u64 + 4096 * 4 };
            x86_64::VirtAddr::new(stack_end)
        };
        tss.interrupt_stack_table[1] = {
            static mut NMI_STACK: [u8; 4096 * 2] = [0; 4096 * 2];
            let stack_end = unsafe { &NMI_STACK as *const _ as u64 + 4096 * 2 };
            x86_64::VirtAddr::new(stack_end)
        };
        tss
    };
}

/// Per-CPU TSS storage
static PER_CPU_TSS: AtomicU64 = AtomicU64::new(0);

/// Initialize GDT
pub fn init() {
    GDT.load();
    unsafe {
        CS::set_reg(KERNEL_CS);
        DS::set_reg(KERNEL_DS);
        ES::set_reg(KERNEL_DS);
        SS::set_reg(KERNEL_DS);
        FS::set_reg(KERNEL_DS);
        GS::set_reg(KERNEL_DS);
        load_tss(KERNEL_TSS);
    }
    log::trace!("GDT loaded");
}

/// Set current CPU's TSS
pub fn set_current_tss(tss: &'static mut x86_64::structures::tss::TaskStateSegment) {
    PER_CPU_TSS.store(tss as *mut _ as u64, Ordering::SeqCst);
    unsafe { load_tss(KERNEL_TSS); }
}

/// Initialize per-CPU TSS for AP
pub fn init_ap_tss(cpu_id: u32) -> &'static mut x86_64::structures::tss::TaskStateSegment {
    use crate::mm::phys::allocate_frames;
    use crate::mm::paging::map_to;
    use x86_64::VirtAddr;

    // Allocate stack pages for this CPU
    let stack_pages = allocate_frames(4).expect("Failed to allocate AP stack");
    let stack_top = VirtAddr::new(
        stack_pages.start_frame().start_address().as_u64() + 4096 * 4
    );

    // Allocate TSS
    let tss_pages = allocate_frames(1).expect("Failed to allocate AP TSS");
    let tss_ptr = tss_pages.start_frame().start_address().as_u64() as *mut x86_64::structures::tss::TaskStateSegment;

    let tss = unsafe { &mut *tss_ptr };
    *tss = x86_64::structures::tss::TaskStateSegment::new();
    tss.interrupt_stack_table[0] = stack_top;

    // Map TSS in per-CPU area
    let percpu_vaddr = crate::arch::x86_64::cpu::percpu_base(cpu_id) + 0x1000;
    map_to(
        x86_64::VirtAddr::new(percpu_vaddr),
        tss_pages.start_frame().start_address(),
        crate::mm::paging::PageTableFlags::PRESENT | crate::mm::paging::PageTableFlags::WRITABLE,
    ).expect("Failed to map AP TSS");

    set_current_tss(unsafe { &mut *tss_ptr });
    unsafe { &mut *tss_ptr }
}