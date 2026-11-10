//! GPU/Display: VGA text + GOP framebuffer + virtio-gpu stub
use log::info;
use crate::drivers::pci::PciDev;

pub fn probe_pci(d: PciDev) -> bool {
    if d.class != 0x03 { return false; }
    info!("GPU {} at {:02x}:{:02x}.{} ({:04x}:{:04x})",
        crate::drivers::pci::class_name(d.class, d.subclass),
        d.bus, d.dev, d.fun, d.vendor, d.device);
    true
}
pub fn init(fb: Option<bloader_shared::FramebufferInfo>) {
    match fb {
        Some(f) => info!("display: {}x{} stride {} fb=0x{:x}", f.width, f.height, f.stride, f.base_addr),
        None => info!("display: VGA text 80x25"),
    }
}
