//! Kernel logger over the serial port, with a RAM ring buffer for `dmesg`.

use core::fmt::{self, Write};
use alloc::string::String;
use log::{Log, Metadata, Record, LevelFilter, Level};
use spin::Mutex;

struct SerialLogger {
    filter: LevelFilter,
}

static LOGGER: SerialLogger = SerialLogger { filter: LevelFilter::Trace };

struct SerialWriter;
impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::arch::x86_64::serial::write_str(s);
        Ok(())
    }
}

static WRITER_LOCK: Mutex<()> = Mutex::new(());

/// 64 KiB of recent kernel messages.  Stored in `.bss`; no heap is needed to
/// append, so messages survive until the heap and bSH are ready.
const DMESG_CAP: usize = 64 * 1024;

struct RingBuffer {
    buf: [u8; DMESG_CAP],
    head: usize,
    len: usize,
}

impl RingBuffer {
    const fn new() -> Self {
        Self {
            buf: [0; DMESG_CAP],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.len < DMESG_CAP {
            let tail = (self.head + self.len) % DMESG_CAP;
            self.buf[tail] = byte;
            self.len += 1;
        } else {
            self.buf[self.head] = byte;
            self.head = (self.head + 1) % DMESG_CAP;
        }
    }

    fn write_str(&mut self, text: &str) {
        for &byte in text.as_bytes() {
            self.push(byte);
        }
    }

    fn snapshot(&self) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec::Vec::with_capacity(self.len);
        for offset in 0..self.len {
            out.push(self.buf[(self.head + offset) % DMESG_CAP]);
        }
        out
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

struct RingWriter<'a> {
    ring: &'a mut RingBuffer,
}

impl Write for RingWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.ring.write_str(s);
        Ok(())
    }
}

static DMESG: Mutex<RingBuffer> = Mutex::new(RingBuffer::new());

impl Log for SerialLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.filter
    }
    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            let _g = WRITER_LOCK.lock();

            // Plain text copy for `dmesg` (no ANSI escapes).
            {
                let mut ring = DMESG.lock();
                let _ = writeln!(
                    RingWriter { ring: &mut ring },
                    "[{:>5} {}] {}",
                    r.level(),
                    r.target(),
                    r.args()
                );
            }

            let mut w = SerialWriter;
            let color = match r.level() {
                Level::Error => "\x1b[91m",
                Level::Warn => "\x1b[93m",
                Level::Info => "\x1b[97m",
                Level::Debug => "\x1b[92m",
                Level::Trace => "\x1b[94m",
            };
            let _ = writeln!(w, "{}[{:>5} {}] {}\x1b[97m", color, r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

pub fn init(filter: LevelFilter) {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(filter);
}

/// Return the current kernel log ring.  When `clear` is true the ring is
/// emptied after the snapshot is copied (like `dmesg -c`).
pub fn dmesg(clear: bool) -> String {
    let mut ring = DMESG.lock();
    let bytes = ring.snapshot();
    if clear {
        ring.clear();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Number of bytes currently held in the ring.
pub fn dmesg_len() -> usize {
    DMESG.lock().len
}
