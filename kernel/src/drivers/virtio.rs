//! Virtio: blk/net/gpu/console probe
use log::info;
use crate::drivers::pci::PciDev;

pub fn probe_pci(d: PciDev) -> bool {
    if d.vendor != 0x1AF4 { return false; }
    let what = match d.device {
        0x1000 => "virtio-net", 0x1001 => "virtio-blk",
        0x1002 => "virtio-console", 0x1005 => "virtio-gpu",
        _ => "virtio-?",
    };
    info!("{} at {:02x}:{:02x}.{}", what, d.bus, d.dev, d.fun);
    true
}
pub fn init() {}
