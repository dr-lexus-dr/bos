//! HDA/AC97 audio stub
use log::info;
use crate::drivers::pci::PciDev;
pub fn probe_pci(d: PciDev) -> bool {
    if d.class != 0x04 { return false; }
    info!("audio at {:02x}:{:02x}.{} ({:04x}:{:04x})", d.bus, d.dev, d.fun, d.vendor, d.device);
    true
}
pub fn init() { info!("audio: hda/ac97 stub (no output yet)"); }
pub fn beep(_hz: u32, _ms: u32) {}
