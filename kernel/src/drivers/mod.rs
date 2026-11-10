//! Device drivers hub — 64-bit, все устройства через PCI dispatch

pub mod timer;
pub mod keyboard;
pub mod mouse;
pub mod ata;
pub mod irq;
pub mod console;
pub(crate) mod font8x16;
pub mod pci;
pub mod net;
pub mod block;
pub mod usb;
pub mod ahci;
pub mod nvme;
pub mod e1000;
pub mod virtio;
pub mod gpu;
pub mod audio;

use log::info;

pub fn init(boot_info: &bloader_shared::BootInfo) {
    irq::init();
    timer::init();
    keyboard::init();
    mouse::init();
    console::init(boot_info.framebuffer);
    pci::init();          // внутри вызывает dispatch_pci для каждого устройства
    ata::init();
    ahci::init();
    nvme::init();
    block::init();
    e1000::init();
    virtio::init();
    net::init();
    usb::init();
    gpu::init(boot_info.framebuffer);
    audio::init();
    info!("all drivers init done (x86_64, PCI/USB/NVMe/AHCI/NIC/GPU/audio)");
}

/// PCI dispatch: вызывается для каждого найденного устройства.
pub fn dispatch_pci(d: pci::PciDev) {
    // порядок: специфичные, потом generic
    if virtio::probe_pci(d) { return; }
    if nvme::probe_pci(d) { return; }
    if ahci::probe_pci(d) { return; }
    if e1000::probe_pci(d) { return; }
    if usb::probe_pci(d) { return; }
    if gpu::probe_pci(d) { return; }
    if audio::probe_pci(d) { return; }
    let _ = d;
}
