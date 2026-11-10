//! ATA PIO stub
pub fn init() {}
pub fn handle_irq() {}
pub fn handle_irq_secondary() {}
pub fn read_lba28(_drive: u8, _lba: u32, _buf: &mut [u8]) -> Result<(), &'static str> { Err("no ata device") }
pub fn write_lba28(_drive: u8, _lba: u32, _buf: &[u8]) -> Result<(), &'static str> { Err("no ata device") }
