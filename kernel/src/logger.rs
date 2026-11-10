//! Kernel logger over serial port

use core::fmt::{self, Write};
use log::{Log, Metadata, Record, LevelFilter};
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

impl Log for SerialLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.filter
    }
    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            let _g = WRITER_LOCK.lock();
            let mut w = SerialWriter;
            let _ = writeln!(w, "[{:>5} {}] {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

pub fn init(filter: LevelFilter) {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(filter);
}
