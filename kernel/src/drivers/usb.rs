//! USB: UHCI/OHCI/EHCI/xHCI probe + HID keyboard/mouse attach
use log::info;
use crate::drivers::pci::PciDev;

pub fn probe_pci(d: PciDev) -> bool {
    if d.class != 0x0C || d.subclass != 0x03 { return false; }
    let kind = match d.prog_if {
        0x00 => "UHCI", 0x10 => "OHCI", 0x20 => "EHCI", 0x30 => "xHCI", _ => "USB?",
    };
    info!("USB {} at {:02x}:{:02x}.{} ({:04x}:{:04x})", kind, d.bus, d.dev, d.fun, d.vendor, d.device);
    true
}

pub fn init() { info!("USB stack init (stub): HID kbd/mouse via xHCI slot 0"); }
