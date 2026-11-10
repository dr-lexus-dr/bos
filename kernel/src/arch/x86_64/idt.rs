//! Interrupt Descriptor Table (IDT) for x86_64

use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::instructions::tables::lidt;
use lazy_static::lazy_static;
use crate::arch::x86_64::{interrupts, cpu};
use log::{error, trace};

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        // Exceptions
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.debug.set_handler_fn(debug_handler);
        idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.overflow.set_handler_fn(overflow_handler);
        idt.bound_range_exceeded.set_handler_fn(bound_range_exceeded_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.device_not_available.set_handler_fn(device_not_available_handler);
        idt.double_fault.set_handler_fn(double_fault_handler);
        idt.invalid_tss.set_handler_fn(invalid_tss_handler);
        idt.segment_not_present.set_handler_fn(segment_not_present_handler);
        idt.stack_segment_fault.set_handler_fn(stack_segment_fault_handler);
        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.x87_floating_point.set_handler_fn(x87_floating_point_handler);
        idt.alignment_check.set_handler_fn(alignment_check_handler);
        idt.machine_check.set_handler_fn(machine_check_handler);
        idt.simd_floating_point.set_handler_fn(simd_floating_point_handler);
        idt.virtualization.set_handler_fn(virtualization_handler);
        idt.cp_protection_exception.set_handler_fn(control_protection_handler);

        // Hardware interrupts (IRQ 0-15 remapped to 0x20-0x2F)
        for i in 0..16 {
            let vector = 0x20 + i;
            unsafe {
                idt[vector].set_handler_fn(irq_handler);
            }
        }

        // Spurious interrupt
        unsafe { idt[0x2F].set_handler_fn(spurious_irq_handler); }

        // Syscall (vector 0x80)
        unsafe { idt[0x80].set_handler_fn(syscall_handler); }
        // Alternate syscall entry (SYSCALL/SYSRET)
        unsafe { idt[0x81].set_handler_fn(syscall_handler); }

        // IPI vectors
        unsafe {
            idt[0xF0].set_handler_fn(ipi_timer_handler);      // Timer IPI
            idt[0xF1].set_handler_fn(ipi_reschedule_handler); // Reschedule IPI
            idt[0xF2].set_handler_fn(ipi_halt_handler);       // Halt IPI
            idt[0xF3].set_handler_fn(ipi_generic_handler);    // Generic IPI
        }

        idt
    };
}

/// IDT loaded flag (plain static: AtomicBool is const-constructible)
static IDT_LOADED: AtomicBool = AtomicBool::new(false);

/// Initialize IDT
pub fn init() {
    IDT.load();
    IDT_LOADED.store(true, Ordering::SeqCst);
    trace!("IDT loaded");
}

/// Check if IDT is loaded
pub fn is_loaded() -> bool {
    IDT_LOADED.load(Ordering::SeqCst)
}

// Exception handlers

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Divide Error\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn debug_handler(stack_frame: InterruptStackFrame) {
    trace!("EXCEPTION: Debug\n{:#?}", stack_frame);
    // Don't kill process for debug exceptions
}

extern "x86-interrupt" fn nmi_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Non-Maskable Interrupt\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    trace!("EXCEPTION: Breakpoint\n{:#?}", stack_frame);
    // Continue execution
}

extern "x86-interrupt" fn overflow_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Overflow\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn bound_range_exceeded_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Bound Range Exceeded\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Invalid Opcode\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn device_not_available_handler(stack_frame: InterruptStackFrame) {
    trace!("EXCEPTION: Device Not Available (FPU/SSE)");
    // Handle FPU/SSE lazy switching
    cpu::handle_fpu_exception();
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, error_code: u64) -> ! {
    error!("EXCEPTION: Double Fault (error code: {})\n{:#?}", error_code, stack_frame);
    panic!("Double fault");
}

extern "x86-interrupt" fn invalid_tss_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: Invalid TSS (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn segment_not_present_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: Segment Not Present (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn stack_segment_fault_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: Stack Segment Fault (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn general_protection_fault_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: General Protection Fault (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode) {
    use x86_64::registers::control::Cr2;
    let addr = Cr2::read().unwrap();
    trace!("EXCEPTION: Page Fault at {:?} (error: {:?})\n{:#?}", addr, error_code, stack_frame);

    // Try to handle page fault (demand paging, COW, etc.)
    if let Err(e) = crate::mm::paging::handle_page_fault(addr, error_code) {
        error!("Page fault handling failed: {}", e);
        crate::proc::exit_current(-1);
    }
}

extern "x86-interrupt" fn x87_floating_point_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: x87 Floating Point\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn alignment_check_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: Alignment Check (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn machine_check_handler(stack_frame: InterruptStackFrame) -> ! {
    error!("EXCEPTION: Machine Check\n{:#?}", stack_frame);
    panic!("Machine check exception");
}

extern "x86-interrupt" fn simd_floating_point_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: SIMD Floating Point\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn virtualization_handler(stack_frame: InterruptStackFrame) {
    error!("EXCEPTION: Virtualization\n{:#?}", stack_frame);
    crate::proc::exit_current(-1);
}

extern "x86-interrupt" fn control_protection_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    error!("EXCEPTION: Control Protection (error code: {})\n{:#?}", error_code, stack_frame);
    crate::proc::exit_current(-1);
}

// IRQ handlers

extern "x86-interrupt" fn irq_handler(_stack_frame: InterruptStackFrame) {
    // Generic IRQ stub: real dispatch needs IRQ number from IDT stub.
    // Timer is the most frequent; drivers poll the rest.
    crate::drivers::timer::handle_irq();
    crate::arch::x86_64::pic::send_eoi(0);
}

extern "x86-interrupt" fn spurious_irq_handler(_stack_frame: InterruptStackFrame) {
    trace!("Spurious IRQ");
    // No EOI needed for spurious interrupt
}

// Syscall handler

extern "x86-interrupt" fn syscall_handler(stack_frame: InterruptStackFrame) {
    crate::syscall::handle_interrupt(stack_frame);
}

// IPI handlers

extern "x86-interrupt" fn ipi_timer_handler(_stack_frame: InterruptStackFrame) {
    crate::sched::timer_tick();
}

extern "x86-interrupt" fn ipi_reschedule_handler(_stack_frame: InterruptStackFrame) {
    crate::sched::reschedule_ipi();
}

extern "x86-interrupt" fn ipi_halt_handler(_stack_frame: InterruptStackFrame) {
    crate::arch::x86_64::halt();
}

extern "x86-interrupt" fn ipi_generic_handler(_stack_frame: InterruptStackFrame) {
    crate::arch::x86_64::apic::handle_ipi();
}