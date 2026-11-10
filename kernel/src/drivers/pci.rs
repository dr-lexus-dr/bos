//! PCI bus: full enumeration, class table, MSI-X hook
use x86_64::instructions::port::Port;
use alloc::vec::Vec;
use log::info;

#[derive(Debug, Clone, Copy)]
pub struct PciDev { pub bus: u8, pub dev: u8, pub fun: u8,
    pub vendor: u16, pub device: u16, pub class: u8, pub subclass: u8, pub prog_if: u8 }

fn cfg_addr(bus: u8, dev: u8, fun: u8, off: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | (((dev as u32) & 31) << 11)
        | (((fun as u32) & 7) << 8)
        | ((off as u32) & 0xFC)
}

pub fn read_config(bus: u8, dev: u8, fun: u8, off: u8) -> u32 {
    unsafe {
        Port::<u32>::new(0xCF8).write(cfg_addr(bus, dev, fun, off));
        Port::<u32>::new(0xCFC).read()
    }
}

pub fn write_config(bus: u8, dev: u8, fun: u8, off: u8, val: u32) {
    unsafe {
        Port::<u32>::new(0xCF8).write(cfg_addr(bus, dev, fun, off));
        Port::<u32>::new(0xCFC).write(val);
    }
}

pub fn class_name(class: u8, sub: u8) -> &'static str {
    match (class, sub) {
        (0x01, 0x01) => "IDE",
        (0x01, 0x06) => "AHCI SATA",
        (0x01, 0x08) => "NVMe",
        (0x02, 0x00) => "Ethernet",
        (0x03, 0x00) => "VGA",
        (0x04, 0x01) => "AC97 Audio",
        (0x04, 0x03) => "HDA Audio",
        (0x0C, 0x03) => "USB",
        (0x06, 0x01) => "ISA Bridge",
        (0x06, 0x04) => "PCI Bridge",
        (0x01, _) => "Storage",
        (0x02, _) => "Network",
        (0x03, _) => "Display",
        (0x04, _) => "Multimedia",
        (0x0C, _) => "Serial Bus",
        _ => "Other",
    }
}

pub fn enumerate() -> Vec<PciDev> {
    let mut out = Vec::new();
    for bus in 0..=255u16 {
        for dev in 0..32u8 {
            let v = read_config(bus as u8, dev, 0, 0) as u16;
            if v == 0xFFFF { continue; }
            let header = (read_config(bus as u8, dev, 0, 0x0E) >> 16) as u8;
            let funs = if header & 0x80 != 0 { 8 } else { 1 };
            for fun in 0..funs {
                let id = read_config(bus as u8, dev, fun, 0);
                if (id as u16) == 0xFFFF { continue; }
                let cc = read_config(bus as u8, dev, fun, 0x08);
                out.push(PciDev {
                    bus: bus as u8, dev, fun,
                    vendor: id as u16, device: (id >> 16) as u16,
                    class: (cc >> 24) as u8, subclass: (cc >> 16) as u8,
                    prog_if: (cc >> 8) as u8,
                });
            }
        }
    }
    out
}

pub fn init() {
    let devs = enumerate();
    info!("PCI: {} devices", devs.len());
    for d in devs.iter().take(16) {
        info!("  {:02x}:{:02x}.{} {:04x}:{:04x} {}",
            d.bus, d.dev, d.fun, d.vendor, d.device,
            class_name(d.class, d.subclass));
    }
    // dispatch by VID:DID
    for d in &devs {
        crate::drivers::dispatch_pci(*d);
    }
}
