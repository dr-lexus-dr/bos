//! PIT/APIC timer driver
use core::sync::atomic::{AtomicU64, Ordering};
static TICKS: AtomicU64 = AtomicU64::new(0);
pub fn init() { crate::arch::x86_64::pic::unmask(0); }
pub fn handle_irq() { TICKS.fetch_add(1, Ordering::SeqCst); crate::sched::timer_tick(); }
pub fn ticks() -> u64 { TICKS.load(Ordering::SeqCst) }
