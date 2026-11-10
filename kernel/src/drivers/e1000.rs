//! e1000 (Intel 8254x) + rtl8139 + virtio-net probe
use log::info;
use crate::drivers::pci::PciDev;

const E1000_VIDS: [(u16, u16); 4] = [(0x8086,0x100E),(0x8086,0x100F),(0x8086,0x1075),(0x8086,0x10D3)];
const RTL_VIDS: [(u16, u16); 2] = [(0x10EC,0x8139),(0x10EC,0x8138)];
const VIRTIO_NET_DID: u16 = 0x1000;

pub fn probe_pci(d: PciDev) -> bool {
    if E1000_VIDS.contains(&(d.vendor, d.device)) {
        info!("e1000 NIC at {:02x}:{:02x}.{}", d.bus, d.dev, d.fun);
        return true;
    }
    if RTL_VIDS.contains(&(d.vendor, d.device)) {
        info!("rtl8139 NIC at {:02x}:{:02x}.{}", d.bus, d.dev, d.fun);
        return true;
    }
    if d.vendor == 0x1AF4 && d.device == VIRTIO_NET_DID {
        info!("virtio-net at {:02x}:{:02x}.{}", d.bus, d.dev, d.fun);
        return true;
    }
    if d.class == 0x02 {
        info!("generic net {:?} {:04x}:{:04x}", (d.bus,d.dev,d.fun), d.vendor, d.device);
        return true;
    }
    false
}
pub fn init() { info!("net NICs: e1000/rtl8139/virtio-net hooks ready"); }
