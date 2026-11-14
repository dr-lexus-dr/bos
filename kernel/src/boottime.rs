//! Early boot timing.
//!
//! The kernel has no calibrated clock this early in the boot path (the PIT
//! IRQ handler is installed much later), so boot time is measured with the
//! TSC against a one-shot PIT channel-2 calibration.  This is deliberately
//! small and self-contained: it must run before the heap, IDT or scheduler.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

use crate::arch::x86_64::rdtsc;

static BOOT_TSC: AtomicU64 = AtomicU64::new(0);
static TSC_PER_MS: AtomicU64 = AtomicU64::new(0);

/// Calibrate the TSC and remember the kernel entry timestamp.
///
/// Safe to call once during early boot, with interrupts disabled.  A PIT
/// failure is not fatal: boot timing simply reports zero.
pub fn init() {
    let per_ms = calibrate_tsc().map(|cycles| (cycles / 10).max(1)).unwrap_or(0);
    TSC_PER_MS.store(per_ms, Ordering::Relaxed);
    BOOT_TSC.store(rdtsc(), Ordering::Relaxed);
}

/// Milliseconds elapsed since [`init`].
pub fn elapsed_ms() -> u64 {
    let per_ms = TSC_PER_MS.load(Ordering::Relaxed);
    if per_ms == 0 {
        return 0;
    }
    rdtsc().wrapping_sub(BOOT_TSC.load(Ordering::Relaxed)) / per_ms
}

/// TSC ticks per millisecond (zero when calibration failed).
pub fn tsc_per_ms() -> u64 {
    TSC_PER_MS.load(Ordering::Relaxed)
}

/// PIT channel 2 one-shot, approximately 10 ms.
fn calibrate_tsc() -> Option<u64> {
    // 1193182 Hz / 11932 = 100 Hz -> 10 ms.
    unsafe {
        let mut gate = Port::<u8>::new(0x61);
        let previous = gate.read() & 3;
        gate.write(0);
        Port::<u8>::new(0x43).write(0xb0);
        let [low, high] = 11_932u16.to_le_bytes();
        Port::<u8>::new(0x42).write(low);
        Port::<u8>::new(0x42).write(high);

        let start = rdtsc();
        gate.write(1);
        let finished = (0..1_000_000).any(|_| gate.read() & 0x20 != 0);
        let cycles = rdtsc().wrapping_sub(start);
        gate.write(previous);
        finished.then_some(cycles)
    }
}
