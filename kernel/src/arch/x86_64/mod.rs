//! x86_64 Architecture-specific code

pub mod gdt;
pub mod idt;
pub mod pic;
pub mod apic;
pub mod cpu;
pub mod paging;
pub mod serial;
pub mod registers;
pub mod interrupts;
pub mod msr;
pub mod tss;

use core::arch::asm;

/// Halt the current CPU
pub fn halt() {
    unsafe { asm!("hlt", options(nomem, nostack)) }
}

/// Halt all CPUs (broadcast IPI)
pub fn halt_all() {
    // Send IPI to all other CPUs to halt
    if crate::arch::x86_64::apic::is_apic_available() {
        crate::arch::x86_64::apic::send_ipi_all(crate::arch::x86_64::apic::IPI_HALT);
    }
    halt();
}

/// Enable interrupts
pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nomem, nostack)) }
}

/// Disable interrupts
pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nomem, nostack)) }
}

/// Short PC-speaker beep (PIT channel 2). Works with interrupts off;
/// used as proof-of-life where no console is visible yet.
pub fn speaker_beep(freq_hz: u32, ms: u32) {
    use x86_64::instructions::port::Port;
    let div = (1193182 / freq_hz.max(1)).clamp(1, 65535) as u16;
    unsafe {
        Port::<u8>::new(0x43).write(0xB6);
        Port::<u8>::new(0x42).write((div & 0xFF) as u8);
        Port::<u8>::new(0x42).write((div >> 8) as u8);
        let mut p61 = Port::<u8>::new(0x61);
        let v = p61.read();
        p61.write(v | 0x03);
        // ~1us per 0x80 read: crude but good enough for a beep.
        for _ in 0..ms * 1000 {
            Port::<u8>::new(0x80).read();
        }
        p61.write(v & !0x03);
    }
}

/// Check if interrupts are enabled
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem, nostack)) }
    (flags & (1 << 9)) != 0
}

/// Wait for interrupt
pub fn wait_for_interrupt() {
    unsafe { asm!("sti; hlt", options(nomem, nostack)) }
}

/// Read time stamp counter
pub fn rdtsc() -> u64 {
    let mut lo: u32;
    let mut hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack))
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Read time stamp counter with processor ID
pub fn rdtscp() -> (u64, u32) {
    let mut lo: u32;
    let mut hi: u32;
    let mut aux: u32;
    unsafe {
        asm!("rdtscp", out("eax") lo, out("edx") hi, out("ecx") aux, options(nomem, nostack))
    }
    (((hi as u64) << 32) | (lo as u64), aux)
}

/// Memory barrier
pub fn mfence() {
    unsafe { asm!("mfence", options(nomem, nostack)) }
}

/// Read memory barrier
pub fn rmb() {
    unsafe { asm!("lfence", options(nomem, nostack)) }
}

/// Write memory barrier
pub fn wmb() {
    unsafe { asm!("sfence", options(nomem, nostack)) }
}

/// Compiler barrier
pub fn compiler_barrier() {
    unsafe { asm!("", options(nomem, nostack, preserves_flags)) }
}

/// Pause instruction (for spin loops)
pub fn pause() {
    unsafe { asm!("pause", options(nomem, nostack)) }
}

/// Get current stack pointer
pub fn stack_pointer() -> u64 {
    let rsp: u64;
    unsafe { asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack)) }
    rsp
}

/// Get current instruction pointer
pub fn instruction_pointer() -> u64 {
    let rip: u64;
    unsafe { asm!("lea {}, [rip]", out(reg) rip, options(nomem, nostack)) }
    rip
}