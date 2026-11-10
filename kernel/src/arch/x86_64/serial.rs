//! Serial port driver (16550 UART) for kernel logging

use core::fmt::{self, Write};
use x86_64::instructions::port::Port;
use spin::Mutex;
use lazy_static::lazy_static;

const COM1: u16 = 0x3F8;
const COM2: u16 = 0x2F8;
const COM3: u16 = 0x3E8;
const COM4: u16 = 0x2E8;

const UART_THR: u16 = 0; // Transmitter Holding Register
const UART_RBR: u16 = 0; // Receiver Buffer Register
const UART_IER: u16 = 1; // Interrupt Enable Register
const UART_FCR: u16 = 2; // FIFO Control Register
const UART_LCR: u16 = 3; // Line Control Register
const UART_MCR: u16 = 4; // Modem Control Register
const UART_LSR: u16 = 5; // Line Status Register
const UART_MSR: u16 = 6; // Modem Status Register
const UART_SCR: u16 = 7; // Scratch Register

const UART_DLL: u16 = 0; // Divisor Latch Low (when DLAB=1)
const UART_DLM: u16 = 1; // Divisor Latch High (when DLAB=1)

const LCR_DLAB: u8 = 0x80; // Divisor Latch Access Bit
const LCR_8N1: u8 = 0x03;  // 8 bits, no parity, 1 stop bit

const LSR_THRE: u8 = 0x20; // Transmitter Holding Register Empty
const LSR_DR: u8 = 0x01;   // Data Ready

const IER_RDI: u8 = 0x01;  // Enable Received Data Interrupt
const IER_THRI: u8 = 0x02; // Enable THR Empty Interrupt

const FCR_ENABLE: u8 = 0x01;     // Enable FIFOs
const FCR_CLEAR_RX: u8 = 0x02;   // Clear RX FIFO
const FCR_CLEAR_TX: u8 = 0x04;   // Clear TX FIFO
const FCR_TRIGGER_14: u8 = 0xC0; // Trigger at 14 bytes

lazy_static! {
    static ref SERIAL_PORT: Mutex<SerialPort> = Mutex::new(SerialPort::new(COM1));
}

/// Serial port structure
pub struct SerialPort {
    base_port: u16,
    initialized: bool,
}

impl SerialPort {
    const fn new(base_port: u16) -> Self {
        Self {
            base_port,
            initialized: false,
        }
    }

    /// Initialize the serial port
    pub fn init(&mut self) {
        let mut lcr = Port::<u8>::new(self.base_port + UART_LCR);
        let mut dll = Port::<u8>::new(self.base_port + UART_DLL);
        let mut dlm = Port::<u8>::new(self.base_port + UART_DLM);
        let mut fcr = Port::<u8>::new(self.base_port + UART_FCR);
        let mut mcr = Port::<u8>::new(self.base_port + UART_MCR);
        let mut ier = Port::<u8>::new(self.base_port + UART_IER);

        // Disable interrupts
        unsafe { ier.write(0u8); }

        // Enable DLAB to set baud rate
        unsafe { lcr.write(LCR_DLAB); }

        // Set baud rate to 115200 (divisor = 1)
        unsafe {
            dll.write(1u8);
            dlm.write(0u8);
        }

        // 8 bits, no parity, 1 stop bit
        unsafe { lcr.write(LCR_8N1); }

        // Enable FIFO, clear them, 14-byte threshold
        unsafe { fcr.write(FCR_ENABLE | FCR_CLEAR_RX | FCR_CLEAR_TX | FCR_TRIGGER_14); }

        // Enable RTS/DTR
        unsafe { mcr.write(0x0B); }

        // Enable interrupts
        unsafe { ier.write(IER_RDI | IER_THRI); }

        self.initialized = true;
    }

    /// Check if transmitter is ready
    fn is_transmit_empty(&mut self) -> bool {
        let mut lsr = Port::<u8>::new(self.base_port + UART_LSR);
        unsafe { lsr.read() & LSR_THRE != 0 }
    }

    /// Check if data is ready
    fn is_data_ready(&mut self) -> bool {
        let mut lsr = Port::<u8>::new(self.base_port + UART_LSR);
        unsafe { lsr.read() & LSR_DR != 0 }
    }

    /// Write a byte (blocking)
    pub fn write_byte(&mut self, byte: u8) {
        if !self.initialized {
            self.init();
        }

        while !self.is_transmit_empty() {
            core::hint::spin_loop();
        }

        let mut thr = Port::<u8>::new(self.base_port + UART_THR);
        unsafe { thr.write(byte); }
    }

    /// Read a byte (blocking)
    pub fn read_byte(&mut self) -> u8 {
        if !self.initialized {
            self.init();
        }

        while !self.is_data_ready() {
            core::hint::spin_loop();
        }

        let mut rbr = Port::<u8>::new(self.base_port + UART_RBR);
        unsafe { rbr.read() }
    }

    /// Try to read a byte (non-blocking)
    pub fn try_read_byte(&mut self) -> Option<u8> {
        if !self.initialized {
            self.init();
        }

        if self.is_data_ready() {
            let mut rbr = Port::<u8>::new(self.base_port + UART_RBR);
            Some(unsafe { rbr.read() })
        } else {
            None
        }
    }

    /// Write a string
    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}

impl Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

/// Initialize serial port
pub fn init() {
    SERIAL_PORT.lock().init();
    log::trace!("Serial port initialized");
}

/// Write a byte to serial port (for early boot)
pub fn write_byte(byte: u8) {
    SERIAL_PORT.lock().write_byte(byte);
}

/// Write a string to serial port
pub fn write_str(s: &str) {
    SERIAL_PORT.lock().write_str(s);
}

/// Read a byte from serial port
pub fn read_byte() -> u8 {
    SERIAL_PORT.lock().read_byte()
}

/// Try to read a byte (non-blocking)
pub fn try_read_byte() -> Option<u8> {
    SERIAL_PORT.lock().try_read_byte()
}

/// Print macro for early boot
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::arch::x86_64::serial::write_str(&format_args!($($arg)*).to_string());
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(concat!($fmt, "\n"), $($arg)*));
}