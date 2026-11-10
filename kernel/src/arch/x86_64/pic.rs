//! 8259 PIC (Programmable Interrupt Controller) driver

use x86_64::instructions::port::Port;
use crate::arch::x86_64::cpu;

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const PIC_EOI: u8 = 0x20;
const PIC_READ_IRR: u8 = 0x0A;
const PIC_READ_ISR: u8 = 0x0B;

const ICW1_INIT: u8 = 0x11;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;

/// Initialize PIC with remapped IRQs
pub fn init() {
    let mut pic1_cmd = Port::<u8>::new(PIC1_COMMAND);
    let mut pic1_data = Port::<u8>::new(PIC1_DATA);
    let mut pic2_cmd = Port::<u8>::new(PIC2_COMMAND);
    let mut pic2_data = Port::<u8>::new(PIC2_DATA);

    // Save masks
    let mask1 = unsafe { pic1_data.read() };
    let mask2 = unsafe { pic2_data.read() };

    // Start initialization sequence
    unsafe {
        pic1_cmd.write(ICW1_INIT | ICW1_ICW4);
        pic2_cmd.write(ICW1_INIT | ICW1_ICW4);
    }

    // ICW2: Vector offsets
    unsafe {
        pic1_data.write(0x20); // Master PIC vectors 0x20-0x27
        pic2_data.write(0x28); // Slave PIC vectors 0x28-0x2F
    }

    // ICW3: Cascading
    unsafe {
        pic1_data.write(0x04); // Slave at IRQ2
        pic2_data.write(0x02); // Cascade identity
    }

    // ICW4: 8086 mode
    unsafe {
        pic1_data.write(ICW4_8086);
        pic2_data.write(ICW4_8086);
    }

    // Restore masks
    unsafe {
        pic1_data.write(mask1);
        pic2_data.write(mask2);
    }

    // Mask all interrupts initially
    mask_all();

    log::trace!("PIC initialized (remapped to 0x20-0x2F)");
}

/// Mask all interrupts
pub fn mask_all() {
    unsafe {
        Port::<u8>::new(PIC1_DATA).write(0xFF);
        Port::<u8>::new(PIC2_DATA).write(0xFF);
    }
}

/// Unmask specific IRQ
pub fn unmask(irq: u8) {
    if irq < 8 {
        let mut port = Port::<u8>::new(PIC1_DATA);
        let mask = unsafe { port.read() };
        unsafe { port.write(mask & !(1 << irq)) };
    } else {
        let mut port = Port::<u8>::new(PIC2_DATA);
        let mask = unsafe { port.read() };
        unsafe { port.write(mask & !(1 << (irq - 8))) };
    }
}

/// Mask specific IRQ
pub fn mask(irq: u8) {
    if irq < 8 {
        let mut port = Port::<u8>::new(PIC1_DATA);
        let mask = unsafe { port.read() };
        unsafe { port.write(mask | (1 << irq)) };
    } else {
        let mut port = Port::<u8>::new(PIC2_DATA);
        let mask = unsafe { port.read() };
        unsafe { port.write(mask | (1 << (irq - 8))) };
    }
}

/// Send End of Interrupt
pub fn send_eoi(irq: u8) {
    if irq >= 8 {
        unsafe { Port::<u8>::new(PIC2_COMMAND).write(PIC_EOI); }
    }
    unsafe { Port::<u8>::new(PIC1_COMMAND).write(PIC_EOI); }
}

/// Read In-Service Register
pub fn read_isr() -> u16 {
    unsafe {
        Port::<u8>::new(PIC1_COMMAND).write(PIC_READ_ISR);
        Port::<u8>::new(PIC2_COMMAND).write(PIC_READ_ISR);
        let isr1 = Port::<u8>::new(PIC1_COMMAND).read() as u16;
        let isr2 = Port::<u8>::new(PIC2_COMMAND).read() as u16;
        (isr2 << 8) | isr1
    }
}

/// Read Interrupt Request Register
pub fn read_irr() -> u16 {
    unsafe {
        Port::<u8>::new(PIC1_COMMAND).write(PIC_READ_IRR);
        Port::<u8>::new(PIC2_COMMAND).write(PIC_READ_IRR);
        let irr1 = Port::<u8>::new(PIC1_COMMAND).read() as u16;
        let irr2 = Port::<u8>::new(PIC2_COMMAND).read() as u16;
        (irr2 << 8) | irr1
    }
}

/// Disable PIC (when using APIC)
pub fn disable() {
    unsafe {
        Port::<u8>::new(PIC1_DATA).write(0xFF);
        Port::<u8>::new(PIC2_DATA).write(0xFF);
    }
}