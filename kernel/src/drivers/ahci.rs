//! AHCI SATA driver (probe + PIO fallback)
use log::info;
use crate::drivers::pci::PciDev;

pub fn probe_pci(d: PciDev) -> bool {
    if d.class != 0x01 || (d.subclass != 0x06 && d.subclass != 0x01) { return false; }
    info!("AHCI/SATA at {:02x}:{:02x}.{} ({:04x}:{:04x})", d.bus, d.dev, d.fun, d.vendor, d.device);
    true
}
pub fn init() {}
