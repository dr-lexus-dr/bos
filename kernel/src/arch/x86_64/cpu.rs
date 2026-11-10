//! CPU management, per-CPU data, and SMP support

use core::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use core::arch::asm;
use core::ptr;
use x86_64::{VirtAddr, PhysAddr};
use crate::arch::x86_64::{apic, gdt, paging};
use x86_64::structures::paging::PageTableFlags;
use lazy_static::lazy_static;
use log::{info, warn, trace};

const MAX_CPUS: usize = 256;
const PERCPU_SIZE: usize = 4096; // 4KB per CPU

static CPU_COUNT: AtomicU32 = AtomicU32::new(1);
static BSP_LAPIC_ID: AtomicU32 = AtomicU32::new(0);
static CPUS_ONLINE: AtomicU32 = AtomicU32::new(1);
static PERCPU_BASE_ADDR: AtomicU32 = AtomicU32::new(0);

/// Per-CPU data structure
#[derive(Clone, Copy)]
#[repr(C, align(4096))]
pub struct PerCpuData {
    pub cpu_id: u32,
    pub lapic_id: u8,
    pub kernel_stack: u64,
    pub user_stack: u64,
    pub current_task: u64,
    pub tss: u64,
    pub gdt: u64,
    pub idle_task: u64,
    pub scheduler_data: [u64; 16],
    pub fpu_state: [u8; 512], // FXSAVE area
    pub apic_timer_freq: u32,
    pub flags: u32,
    pub reserved: [u8; 4096 - 8*8 - 512 - 8],
}

impl PerCpuData {
    pub const fn new() -> Self {
        Self {
            cpu_id: 0,
            lapic_id: 0,
            kernel_stack: 0,
            user_stack: 0,
            current_task: 0,
            tss: 0,
            gdt: 0,
            idle_task: 0,
            scheduler_data: [0; 16],
            fpu_state: [0; 512],
            apic_timer_freq: 0,
            flags: 0,
            reserved: [0; 4096 - 8*8 - 512 - 8],
        }
    }
}

static mut PERCPU_AREAS: [PerCpuData; MAX_CPUS] = [PerCpuData::new(); MAX_CPUS];

/// Get per-CPU data for current CPU
pub fn current() -> &'static mut PerCpuData {
    let cpu_id = current_cpu_id().min((MAX_CPUS as u32) - 1);
    unsafe { &mut PERCPU_AREAS[cpu_id as usize] }
}

/// Get per-CPU data by CPU ID
pub fn get(cpu_id: u32) -> Option<&'static mut PerCpuData> {
    if cpu_id < MAX_CPUS as u32 {
        unsafe { Some(&mut PERCPU_AREAS[cpu_id as usize]) }
    } else {
        None
    }
}

/// Get current CPU ID
pub fn current_cpu_id() -> u32 {
    // In x86_64, we can use the GS base to store CPU ID
    // For now, read from APIC ID
    apic::local_apic_id() as u32
}

/// Get BSP LAPIC ID
pub fn bsp_lapic_id() -> u8 {
    BSP_LAPIC_ID.load(Ordering::SeqCst) as u8
}

/// Set BSP LAPIC ID
pub fn set_bsp_lapic_id(id: u8) {
    BSP_LAPIC_ID.store(id as u32, Ordering::SeqCst);
}

/// Get CPU count
pub fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::SeqCst)
}

/// Set CPU count (called during ACPI parsing)
pub fn set_cpu_count(count: u32) {
    CPU_COUNT.store(count, Ordering::SeqCst);
}

/// Get number of online CPUs
pub fn online_cpus() -> u32 {
    CPUS_ONLINE.load(Ordering::SeqCst)
}

/// Mark CPU as online
pub fn cpu_online(cpu_id: u32) {
    CPUS_ONLINE.fetch_add(1, Ordering::SeqCst);
    get(cpu_id).map(|cpu| cpu.flags |= 1);
}

/// Get per-CPU base virtual address
pub fn percpu_base(cpu_id: u32) -> u64 {
    let base = PERCPU_BASE_ADDR.load(Ordering::SeqCst) as u64;
    base + (cpu_id as u64 * PERCPU_SIZE as u64)
}

/// Initialize per-CPU areas
pub fn init_percpu_areas() {
    use crate::mm::phys::allocate_frames;
    use crate::mm::paging::map_to;

    let cpu_count = cpu_count() as usize;
    let total_size = cpu_count * PERCPU_SIZE;

    // Allocate physical frames
    let frames = allocate_frames((total_size + 4095) / 4096)
        .expect("Failed to allocate per-CPU frames");

    let phys_base = frames.start_frame().start_address().as_u64();
    let virt_base = 0xFFFF_FF80_0000_0000; // Per-CPU area in higher half

    PERCPU_BASE_ADDR.store(virt_base as u32, Ordering::SeqCst);

    // Map per-CPU area
    for i in 0..cpu_count {
        let offset = (i * PERCPU_SIZE) as u64;
        map_to(
            VirtAddr::new(virt_base + offset),
            PhysAddr::new(phys_base + offset),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        ).expect("Failed to map per-CPU area");
    }

    // Initialize per-CPU data
    for i in 0..cpu_count {
        let cpu = unsafe { &mut PERCPU_AREAS[i] };
        cpu.cpu_id = i as u32;
        // lapic_id will be set when CPU starts
    }

    info!("Per-CPU areas initialized: {} CPUs at 0x{:x}", cpu_count, virt_base);
}

/// Initialize BSP (Bootstrap Processor)
pub fn init_bsp() {
    let cpu = current();
    cpu.cpu_id = 0;
    cpu.lapic_id = apic::local_apic_id();
    cpu.apic_timer_freq = apic::timer_frequency();
    cpu.flags = 1; // Online

    // Set up GS base for per-CPU access
    set_gs_base(percpu_base(0));

    // Initialize TSS for BSP
    let tss = gdt::init_ap_tss(0);
    cpu.tss = tss as *mut _ as u64;

    set_bsp_lapic_id(cpu.lapic_id);
    info!("BSP initialized: CPU 0, LAPIC ID {}", cpu.lapic_id);
}

/// Initialize AP (Application Processor)
pub fn init_ap(cpu_id: u32) {
    let cpu = get(cpu_id).expect("Invalid CPU ID");
    cpu.cpu_id = cpu_id;
    cpu.lapic_id = apic::local_apic_id();
    cpu.apic_timer_freq = apic::timer_frequency();
    cpu.flags = 1; // Online

    // Set up GS base
    set_gs_base(percpu_base(cpu_id));

    // Initialize TSS for AP
    let tss = gdt::init_ap_tss(cpu_id);
    cpu.tss = tss as *mut _ as u64;

    // Allocate kernel stack
    use crate::mm::phys::allocate_frames;
    let stack_frames = allocate_frames(4).expect("Failed to allocate AP stack");
    cpu.kernel_stack = stack_frames.start_frame().start_address().as_u64() + 4096 * 4;

    cpu_online(cpu_id);
    trace!("AP initialized: CPU {}, LAPIC ID {}", cpu_id, cpu.lapic_id);
}

/// Set GS base MSR
fn set_gs_base(addr: u64) {
    unsafe {
        crate::arch::x86_64::msr::write_msr(0xC0000101, addr); // IA32_GS_BASE
    }
}

/// Get GS base
pub fn gs_base() -> u64 {
    unsafe { crate::arch::x86_64::msr::read_msr(0xC0000101) }
}

/// Start AP
pub fn start_ap(cpu_id: u32, entry_point: u64, stack_top: u64) {
    use crate::arch::x86_64::apic;

    if cpu_id == 0 {
        warn!("Cannot start BSP as AP");
        return;
    }

    let cpu = get(cpu_id).expect("Invalid CPU ID");
    let lapic_id = cpu.lapic_id;

    // INIT-SIPI-SIPI sequence
    // 1. Send INIT IPI
    apic::send_ipi(lapic_id, 0); // INIT
    // Wait 10ms
    for _ in 0..1_000_000 { core::hint::spin_loop(); }

    // 2. Send SIPI (Startup IPI)
    let startup_vector = (entry_point >> 12) as u8;
    apic::send_ipi(lapic_id, startup_vector); // SIPI
    // Wait 200us
    for _ in 0..20_000 { core::hint::spin_loop(); }

    // 3. Send second SIPI
    apic::send_ipi(lapic_id, startup_vector);
    // Wait for AP to signal ready
    for _ in 0..1_000_000 { core::hint::spin_loop(); }

    info!("AP {} started (LAPIC {})", cpu_id, lapic_id);
}

/// Get current task pointer
pub fn current_task() -> u64 {
    current().current_task
}

/// Set current task pointer
pub fn set_current_task(task: u64) {
    current().current_task = task;
}

/// Get kernel stack
pub fn kernel_stack() -> u64 {
    current().kernel_stack
}

/// Set kernel stack
pub fn set_kernel_stack(stack: u64) {
    current().kernel_stack = stack;
}

/// FPU state management
pub fn save_fpu_state() {
    let cpu = current();
    unsafe {
        asm!("fxsave [{}]", in(reg) cpu.fpu_state.as_mut_ptr(), options(nomem, nostack));
    }
}

pub fn restore_fpu_state() {
    let cpu = current();
    unsafe {
        asm!("fxrstor [{}]", in(reg) cpu.fpu_state.as_ptr(), options(nomem, nostack));
    }
}

/// Enable FPU/SSE
pub fn enable_fpu() {
    unsafe {
        // Clear CR0.TS (Task Switched) and set CR0.MP (Monitor Coprocessor)
        asm!(
            "mov rax, cr0",
            "and rax, ~(1 << 3)",  // Clear TS
            "or rax, 1 << 1",      // Set MP
            "mov cr0, rax",
            options(nomem, nostack)
        );
        // Enable FXSAVE/FXRSTOR
        asm!(
            "mov rax, cr4",
            "or rax, 1 << 9",      // Set OSFXSR
            "or rax, 1 << 10",     // Set OSXMMEXCPT
            "mov cr4, rax",
            options(nomem, nostack)
        );
    }
}

/// Handle FPU exception (device not available)
pub fn handle_fpu_exception() {
    // Clear TS bit
    unsafe {
        asm!(
            "mov rax, cr0",
            "and rax, ~(1 << 3)",
            "mov cr0, rax",
            options(nomem, nostack)
        );
    }
    // FPU will be lazily restored on next use
}

/// Get CPU features
pub fn cpu_features() -> CpuFeatures {
    let mut features = CpuFeatures::default();

    let regs = cpuid(1);
    features.sse = (regs.edx >> 25) & 1 != 0;
    features.sse2 = (regs.edx >> 26) & 1 != 0;
    features.sse3 = regs.ecx & 1 != 0;
    features.ssse3 = (regs.ecx >> 9) & 1 != 0;
    features.sse4_1 = (regs.ecx >> 19) & 1 != 0;
    features.sse4_2 = (regs.ecx >> 20) & 1 != 0;
    features.avx = (regs.ecx >> 28) & 1 != 0;
    features.fma = (regs.ecx >> 12) & 1 != 0;
    features.aes = (regs.ecx >> 25) & 1 != 0;
    features.xsave = (regs.ecx >> 26) & 1 != 0;
    features.osxsave = (regs.ecx >> 27) & 1 != 0;

    let regs = cpuid(7);
    features.avx2 = (regs.ebx >> 5) & 1 != 0;
    features.bmi1 = (regs.ebx >> 3) & 1 != 0;
    features.bmi2 = (regs.ebx >> 8) & 1 != 0;
    features.avx512f = (regs.ebx >> 16) & 1 != 0;

    let regs = cpuid(0x8000_0001);
    features.lm = (regs.edx >> 29) & 1 != 0;
    features.nx = (regs.edx >> 20) & 1 != 0;
    features.syscall = (regs.edx >> 11) & 1 != 0;
    features.rdtscp = (regs.edx >> 27) & 1 != 0;
    features.fxsr_opt = (regs.edx >> 8) & 1 != 0;
    features.gbpages = (regs.edx >> 26) & 1 != 0;

    features
}

/// CPUID instruction wrapper
fn cpuid(eax: u32) -> CpuidResult {
    let r = unsafe { core::arch::x86_64::__cpuid(eax) };
    CpuidResult { eax: r.eax, ebx: r.ebx, ecx: r.ecx, edx: r.edx }
}

#[derive(Default)]
pub struct CpuFeatures {
    pub sse: bool,
    pub sse2: bool,
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub avx: bool,
    pub avx2: bool,
    pub avx512f: bool,
    pub fma: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    pub aes: bool,
    pub xsave: bool,
    pub osxsave: bool,
    pub lm: bool,        // Long mode
    pub nx: bool,        // No-execute
    pub syscall: bool,
    pub rdtscp: bool,
    pub fxsr_opt: bool,
    pub gbpages: bool,
}

struct CpuidResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

/// AP main loop
pub fn ap_main(cpu_id: u32) -> ! {
    init_ap(cpu_id);

    // Enable interrupts
    crate::arch::x86_64::enable_interrupts();

    // AP scheduler loop
    crate::sched::ap_main(cpu_id)
}