//! Interrupt helpers (CLI/STI wrappers + IRQ wait)

use core::arch::asm;

#[inline]
pub fn enable() {
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn disable() {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn enable_and_halt() {
    unsafe { asm!("sti; hlt", options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn are_enabled() -> bool {
    let rflags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) rflags, options(nomem, nostack, preserves_flags)) };
    (rflags & (1 << 9)) != 0
}

/// Run `f` with interrupts disabled, restore previous state afterwards.
pub fn without_interrupts<T>(f: impl FnOnce() -> T) -> T {
    let was_enabled = are_enabled();
    if was_enabled {
        disable();
    }
    let r = f();
    if was_enabled {
        enable();
    }
    r
}
