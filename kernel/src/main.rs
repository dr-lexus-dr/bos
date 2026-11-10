#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(abi_x86_interrupt)]
#![feature(asm_sym)]
#![feature(alloc_error_handler)]
#![feature(panic_info_message)]
#![feature(stmt_expr_attributes)]
#![feature(const_mut_refs)]
#![feature(slice_ptr_get)]
#![feature(ptr_metadata)]

extern crate alloc;

use core::{arch::global_asm, panic::PanicInfo, sync::atomic::{AtomicBool, Ordering}};
use alloc::{vec::Vec, string::String, format, boxed::Box, sync::Arc};
use log::{info, warn, error, debug, trace, LevelFilter};
use bloader_shared::{BootInfo, KernelEntryFn, KERNEL_MAGIC, BOOT_PROTOCOL_VERSION};

mod arch;
mod mm;
mod fs;
mod proc;
mod sched;
mod drivers;
mod syscall;
mod ipc;
mod compat;
mod init;
mod bsh;
mod user;
mod logger;

use arch::x86_64::{gdt, idt, pic, apic, cpu, paging, serial};
use init::kernel_main;

static KERNEL_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "sysv64" fn _start_rust(boot_info: &'static BootInfo) -> ! {
    // Early initialization - before memory management
    early_init(boot_info);

    // Initialize kernel subsystems
    kernel_init(boot_info);

    // Start the kernel main loop
    kernel_main(boot_info)
}

fn early_init(boot_info: &'static BootInfo) {
    // Verify boot info magic
    assert_eq!(boot_info.magic, bloader_shared::BLOADER_MAGIC, "Invalid boot info magic");
    assert_eq!(boot_info.version, BOOT_PROTOCOL_VERSION, "Unsupported boot protocol version");

    // Proof of life before anything else (audible where no console is up).
    arch::x86_64::speaker_beep(880, 120);

    // Initialize serial logging first
    serial::init();
    logger::init(LevelFilter::Trace);

    info!("=== bOS Kernel Starting ===");
    info!("Bootloader: {} v{}", boot_info.bootloader_name_str(), boot_info.bootloader_version_str());
    info!("Kernel UUID: {}", boot_info.kernel_uuid);
    info!("Command line: {}", boot_info.cmdline_str());

    // Initialize GDT
    gdt::init();
    info!("GDT initialized");

    // Initialize IDT
    idt::init();
    info!("IDT initialized");

    // Initialize PIC (no paging needed — pure port I/O)
    pic::init();
    info!("PIC initialized");

    // Initialize physical memory manager
    mm::phys::init(&boot_info.memory_map);
    info!("Physical memory manager initialized");

    // Initialize virtual memory manager (must precede APIC: apic::init
    // uses map_to, which needs KERNEL_PML4/PHYS_MEM_OFFSET from here)
    paging::init(&boot_info.memory_map);
    info!("Virtual memory manager initialized");

    // Initialize APIC (needs map_to for MMIO mapping)
    apic::init();
    info!("Interrupt controllers initialized");

    KERNEL_INITIALIZED.store(true, Ordering::SeqCst);
}

fn kernel_init(_boot_info: &'static BootInfo) {
    info!("Initializing kernel subsystems...");

    // Initialize heap allocator
    mm::heap::init();
    info!("Heap allocator initialized");

    // Initialize scheduler (tables only; no threads yet)
    sched::init();
    info!("Scheduler initialized");

    // NOTE (bSH bring-up): process management, drivers, VFS, syscalls, IPC
    // and compat are not started yet — their init paths touch subsystems
    // that are still stubs and would triple-fault. The kernel drops into
    // the built-in bSH console instead; userspace comes back in stages.
    info!("proc/drivers/fs/syscall/ipc/compat skipped (bSH bring-up)");

    info!("Kernel initialization complete");
}

/// Kernel entry point for secondary CPUs (SMP)
#[no_mangle]
pub extern "sysv64" fn _start_ap_rust(cpu_id: u32, boot_info: &'static BootInfo) -> ! {
    // AP initialization
    cpu::init_ap(cpu_id);

    // Start AP scheduler
    sched::ap_main(cpu_id)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("KERNEL PANIC: {}", info);

    // Print stack trace if available
    #[cfg(feature = "stack_trace")]
    {
        use core::fmt::Write;
        let mut buf = String::new();
        let _ = write!(&mut buf, "{}", info);
        error!("{}", buf);
    }

    // Halt all CPUs
    arch::x86_64::halt_all();

    loop {
        core::hint::spin_loop();
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    error!("Allocation error: {:?}", layout);
    panic!("Out of memory");
}

// Assembly entry point
global_asm!(include_str!("arch/x86_64/entry.asm"));

// Kernel header for bootloader
#[used]
#[link_section = ".kernel_header"]
static KERNEL_HEADER: bloader_shared::KernelHeader = bloader_shared::KernelHeader {
    magic: KERNEL_MAGIC,
    version: BOOT_PROTOCOL_VERSION,
    entry_offset: 0, // Filled by linker
    load_addr: 0xFFFF_8000_0000_0000, // Higher half kernel
    size: 0, // Filled by linker
    flags: bloader_shared::KernelFlags::from_bits_truncate(0x3), // RELOCATABLE | REQUIRES_SMP
    required_memory: 64 * 1024 * 1024, // 64 MB minimum
    uuid: uuid::Uuid::nil(), // bLOADER derives the image UUID when none is embedded.
    name: *b"bOS Kernel\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    cmdline_default: *b"loglevel=debug console=serial\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
};