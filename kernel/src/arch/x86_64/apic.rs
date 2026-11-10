//! Local APIC and I/O APIC driver

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use x86_64::instructions::port::Port;
use x86_64::{PhysAddr, VirtAddr};
use crate::mm::paging::{map_to, translate, PageTableFlags};
use crate::arch::x86_64::paging::KERNEL_BASE;
use log::{info, warn, trace};
use crate::arch::x86_64::msr::{read_msr, write_msr, IA32_APIC_BASE};

// APIC register offsets
const APIC_ID: u32 = 0x20;
const APIC_VERSION: u32 = 0x30;
const APIC_TPR: u32 = 0x80;
const APIC_APR: u32 = 0x90;
const APIC_PPR: u32 = 0xA0;
const APIC_EOI: u32 = 0xB0;
const APIC_RRR: u32 = 0xC0;
const APIC_LDR: u32 = 0xD0;
const APIC_DFR: u32 = 0xE0;
const APIC_SVR: u32 = 0xF0;
const APIC_ISR: u32 = 0x100;
const APIC_TMR: u32 = 0x180;
const APIC_IRR: u32 = 0x200;
const APIC_ESR: u32 = 0x280;
const APIC_ICR_LOW: u32 = 0x300;
const APIC_ICR_HIGH: u32 = 0x310;
const APIC_TIMER_LVT: u32 = 0x320;
const APIC_THERMAL_LVT: u32 = 0x330;
const APIC_PERF_LVT: u32 = 0x340;
const APIC_LINT0_LVT: u32 = 0x350;
const APIC_LINT1_LVT: u32 = 0x360;
const APIC_ERROR_LVT: u32 = 0x370;
const APIC_TIMER_INIT_COUNT: u32 = 0x380;
const APIC_TIMER_CURRENT_COUNT: u32 = 0x390;
const APIC_TIMER_DIVIDE: u32 = 0x3E0;

// ICR delivery modes
const ICR_FIXED: u32 = 0x00000000;
const ICR_LOWEST_PRIORITY: u32 = 0x00000100;
const ICR_SMI: u32 = 0x00000200;
const ICR_NMI: u32 = 0x00000400;
const ICR_INIT: u32 = 0x00000500;
const ICR_STARTUP: u32 = 0x00000600;
const ICR_LEVEL_ASSERT: u32 = 0x00004000;
const ICR_LEVEL_DEASSERT: u32 = 0x00000000;
const ICR_DEST_SHORTHAND_NONE: u32 = 0x00000000;
const ICR_DEST_SHORTHAND_SELF: u32 = 0x00040000;
const ICR_DEST_SHORTHAND_ALL_INCL_SELF: u32 = 0x00080000;
const ICR_DEST_SHORTHAND_ALL_EXCL_SELF: u32 = 0x000C0000;

// SVR enable bit
const SVR_ENABLE: u32 = 0x100;

// LVT mask bit
const LVT_MASK: u32 = 0x10000;

// Timer modes
const TIMER_ONE_SHOT: u32 = 0x00000000;
const TIMER_PERIODIC: u32 = 0x00020000;
const TIMER_TSC_DEADLINE: u32 = 0x00040000;

// IPI vectors
pub const IPI_TIMER: u8 = 0xF0;
pub const IPI_RESCHEDULE: u8 = 0xF1;
pub const IPI_HALT: u8 = 0xF2;
pub const IPI_GENERIC: u8 = 0xF3;

static LOCAL_APIC_ADDR: AtomicU64 = AtomicU64::new(0);
static IO_APIC_ADDR: AtomicU64 = AtomicU64::new(0);
static APIC_FREQUENCY: AtomicU32 = AtomicU32::new(0);

/// Read local APIC register
pub unsafe fn read_reg(offset: u32) -> u32 {
    let base = LOCAL_APIC_ADDR.load(Ordering::SeqCst) as *mut u32;
    base.add(offset as usize / 4).read_volatile()
}

/// Write local APIC register
pub unsafe fn write_reg(offset: u32, value: u32) {
    let base = LOCAL_APIC_ADDR.load(Ordering::SeqCst) as *mut u32;
    base.add(offset as usize / 4).write_volatile(value);
}

/// Initialize local APIC
pub fn init() {
    // Get APIC base from MSR
    let apic_base = unsafe { read_msr(IA32_APIC_BASE) };
    let apic_addr = (apic_base & 0xFFFF_F000) as u32;

    LOCAL_APIC_ADDR.store(apic_addr as u64, Ordering::SeqCst);

    // Map APIC if not identity mapped
    if apic_addr >= 0xFEE0_0000 {
        // Map into the higher half (KERNEL_BASE alias covers low 4GB from
        // early boot; paging::init keeps it). NOTE: plain
        // `0xFFFF_FF00_0000 + apic` overflows into bit 48 and is
        // non-canonical — do not use a 0xFFFF_FFxx base here.
        let virt_addr = KERNEL_BASE + apic_addr as u64;
        // The early-boot 1GB alias (PML4[256]) already covers this address;
        // map_to cannot split a huge page, so map only when unmapped.
        if translate(VirtAddr::new(virt_addr)).is_none() {
            map_to(
                VirtAddr::new(virt_addr),
                PhysAddr::new(apic_addr as u64),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE,
            ).expect("Failed to map local APIC");
        }
        LOCAL_APIC_ADDR.store(virt_addr, Ordering::SeqCst);
    }

    // Enable APIC
    unsafe {
        let mut svr = read_reg(APIC_SVR);
        svr |= SVR_ENABLE;
        write_reg(APIC_SVR, svr);
    }

    // Set spurious interrupt vector
    unsafe { write_reg(APIC_SVR, 0xFF | SVR_ENABLE); }

    // Mask all LVT entries
    unsafe {
        write_reg(APIC_TIMER_LVT, LVT_MASK);
        write_reg(APIC_THERMAL_LVT, LVT_MASK);
        write_reg(APIC_PERF_LVT, LVT_MASK);
        write_reg(APIC_LINT0_LVT, LVT_MASK);
        write_reg(APIC_LINT1_LVT, LVT_MASK);
        write_reg(APIC_ERROR_LVT, LVT_MASK | 0xFF);
    }

    // Calibrate timer
    calibrate_timer();

    info!("Local APIC initialized at 0x{:x}", LOCAL_APIC_ADDR.load(Ordering::SeqCst));
}

/// Initialize I/O APIC
pub fn init_ioapic(ioapic_addr: u32, gsi_base: u32) {
    IO_APIC_ADDR.store(ioapic_addr as u64, Ordering::SeqCst);

    // Map I/O APIC (same higher-half scheme as local APIC; the old
    // 0xFFFF_FE00_0000 base is non-canonical once the offset is added).
    let virt_addr = KERNEL_BASE + ioapic_addr as u64;
    // See local APIC above: skip if the early alias already maps it.
    if translate(VirtAddr::new(virt_addr)).is_none() {
        map_to(
            VirtAddr::new(virt_addr),
            PhysAddr::new(ioapic_addr as u64),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE,
        ).expect("Failed to map I/O APIC");
    }
    IO_APIC_ADDR.store(virt_addr, Ordering::SeqCst);

    info!("I/O APIC initialized at 0x{:x}", IO_APIC_ADDR.load(Ordering::SeqCst));
}

/// Check if APIC is available
pub fn is_apic_available() -> bool {
    LOCAL_APIC_ADDR.load(Ordering::SeqCst) != 0
}

/// Get local APIC ID
pub fn local_apic_id() -> u8 {
    (unsafe { read_reg(APIC_ID) } >> 24) as u8
}

/// Send EOI
pub fn send_eoi() {
    unsafe { write_reg(APIC_EOI, 0); }
}

/// Send IPI to specific CPU
pub fn send_ipi(target_apic_id: u8, vector: u8) {
    unsafe {
        // Wait for ICR to be ready
        while read_reg(APIC_ICR_LOW) & 0x1000 != 0 {
            core::hint::spin_loop();
        }

        // Set target
        write_reg(APIC_ICR_HIGH, (target_apic_id as u32) << 24);

        // Send IPI
        write_reg(APIC_ICR_LOW, ICR_FIXED | ICR_LEVEL_ASSERT | ICR_DEST_SHORTHAND_NONE | (vector as u32));
    }
}

/// Send IPI to all CPUs including self
pub fn send_ipi_all_incl_self(vector: u8) {
    unsafe {
        while read_reg(APIC_ICR_LOW) & 0x1000 != 0 {
            core::hint::spin_loop();
        }
        write_reg(APIC_ICR_HIGH, 0);
        write_reg(APIC_ICR_LOW, ICR_FIXED | ICR_LEVEL_ASSERT | ICR_DEST_SHORTHAND_ALL_INCL_SELF | (vector as u32));
    }
}

/// Send IPI to all CPUs excluding self
pub fn send_ipi_all_excl_self(vector: u8) {
    unsafe {
        while read_reg(APIC_ICR_LOW) & 0x1000 != 0 {
            core::hint::spin_loop();
        }
        write_reg(APIC_ICR_HIGH, 0);
        write_reg(APIC_ICR_LOW, ICR_FIXED | ICR_LEVEL_ASSERT | ICR_DEST_SHORTHAND_ALL_EXCL_SELF | (vector as u32));
    }
}

/// Send IPI to self
pub fn send_ipi_self(vector: u8) {
    unsafe {
        while read_reg(APIC_ICR_LOW) & 0x1000 != 0 {
            core::hint::spin_loop();
        }
        write_reg(APIC_ICR_HIGH, 0);
        write_reg(APIC_ICR_LOW, ICR_FIXED | ICR_LEVEL_ASSERT | ICR_DEST_SHORTHAND_SELF | (vector as u32));
    }
}

/// Handle incoming IPI
pub fn handle_ipi() {
    // Generic IPI handler
    trace!("Received generic IPI");
}

/// Compat alias used by arch::halt_all
pub fn send_ipi_all(vector: u8) {
    send_ipi_all_excl_self(vector);
}

/// Set timer to one-shot mode
pub fn set_timer_one_shot(ticks: u32) {
    unsafe {
        write_reg(APIC_TIMER_LVT, LVT_MASK | TIMER_ONE_SHOT | IPI_TIMER as u32);
        write_reg(APIC_TIMER_INIT_COUNT, ticks);
    }
}

/// Set timer to periodic mode
pub fn set_timer_periodic(ticks: u32) {
    unsafe {
        write_reg(APIC_TIMER_LVT, LVT_MASK | TIMER_PERIODIC | IPI_TIMER as u32);
        write_reg(APIC_TIMER_INIT_COUNT, ticks);
    }
}

/// Stop timer
pub fn stop_timer() {
    unsafe { write_reg(APIC_TIMER_LVT, LVT_MASK); }
}

/// Get current timer count
pub fn timer_current_count() -> u32 {
    unsafe { read_reg(APIC_TIMER_CURRENT_COUNT) }
}

/// Calibrate APIC timer using PIT
fn calibrate_timer() {
    // Simple calibration: run timer for 10ms and count ticks
    // This is a simplified version - real implementation would use PIT or TSC
    unsafe {
        // Set divide by 16
        write_reg(APIC_TIMER_DIVIDE, 0x3);

        // Set initial count to max
        write_reg(APIC_TIMER_INIT_COUNT, 0xFFFF_FFFF);
        write_reg(APIC_TIMER_LVT, TIMER_ONE_SHOT | LVT_MASK | IPI_TIMER as u32);

        // Wait ~10ms using busy loop (in real kernel, use PIT)
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }

        let elapsed = 0xFFFF_FFFF - read_reg(APIC_TIMER_CURRENT_COUNT);
        let freq = (elapsed as u64 * 100) as u32; // Approximate Hz

        APIC_FREQUENCY.store(freq, Ordering::SeqCst);
        trace!("APIC timer frequency: ~{} Hz", freq);
    }
}

/// Get APIC timer frequency
pub fn timer_frequency() -> u32 {
    APIC_FREQUENCY.load(Ordering::SeqCst)
}

/// Set timer divide configuration
pub fn set_timer_divide(divide: u32) {
    unsafe { write_reg(APIC_TIMER_DIVIDE, divide & 0xF); }
}

/// Read I/O APIC register
unsafe fn ioapic_read(reg: u32) -> u32 {
    let base = IO_APIC_ADDR.load(Ordering::SeqCst) as *mut u32;
    base.write_volatile(reg);
    base.add(1).read_volatile()
}

/// Write I/O APIC register
unsafe fn ioapic_write(reg: u32, value: u32) {
    let base = IO_APIC_ADDR.load(Ordering::SeqCst) as *mut u32;
    base.write_volatile(reg);
    base.add(1).write_volatile(value);
}

/// Set I/O APIC redirection entry
pub fn ioapic_set_irq(gsi: u32, vector: u8, delivery_mode: u32, dest_mode: u32, polarity: u32, trigger: u32, masked: bool, destination: u8) {
    let reg_low = 0x10 + gsi * 2;
    let reg_high = 0x11 + gsi * 2;

    let mut low = (vector as u32) |
        (delivery_mode & 0x7) << 8 |
        (dest_mode & 0x1) << 11 |
        (polarity & 0x1) << 13 |
        (trigger & 0x1) << 15 |
        (if masked { 1 } else { 0 }) << 16;

    let high = (destination as u32) << 24;

    unsafe {
        ioapic_write(reg_low, low);
        ioapic_write(reg_high, high);
    }
}

/// Enable IRQ on I/O APIC
pub fn ioapic_enable_irq(gsi: u32, vector: u8, destination: u8) {
    ioapic_set_irq(gsi, vector, 0, 0, 0, 0, false, destination);
}

/// Disable IRQ on I/O APIC
pub fn ioapic_disable_irq(gsi: u32) {
    let reg_low = 0x10 + gsi * 2;
    unsafe {
        let mut low = ioapic_read(reg_low);
        low |= 1 << 16; // Mask bit
        ioapic_write(reg_low, low);
    }
}