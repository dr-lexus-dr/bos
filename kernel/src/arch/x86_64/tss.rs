//! TSS helpers (thin wrapper over x86_64 crate + GDT module)

pub use x86_64::structures::tss::TaskStateSegment;

/// Create a TSS with the given privilege stack (ring0) and IST entries.
pub fn new_with_stacks(priv_stack: x86_64::VirtAddr, ist0: x86_64::VirtAddr) -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();
    tss.privilege_stack_table[0] = priv_stack;
    tss.interrupt_stack_table[0] = ist0;
    tss
}
