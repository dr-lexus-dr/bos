//! PS/2 mouse stub
pub fn init() { crate::arch::x86_64::pic::unmask(12); }
pub fn handle_irq() {}
