//! NVMe driver (probe + admin queue stub)
use log::info;
use crate::drivers::pci::PciDev;

pub fn probe_pci(d: PciDev) -> bool {
    if d.class != 0x01 || d.subclass != 0x08 { return false; }
    info!("NVMe at {:02x}:{:02x}.{} ({:04x}:{:04x})", d.bus, d.dev, d.fun, d.vendor, d.device);
    true
}
pub fn init() {}
pub fn read_blocks(_ns: u32, _lba: u64, _buf: &mut [u8]) -> Result<(), &'static str> { Err("nvme: no ns") }
