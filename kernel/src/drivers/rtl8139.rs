//! Legacy DMA mode for the Realtek RTL8139 (PCI 10ec:8139).
use core::{
    ptr,
    sync::atomic::{fence, Ordering},
};
use x86_64::instructions::port::Port;

use super::{
    net::{Frame, Nic, FRAME_CAPACITY},
    pci::{self, PciDev},
};

const TX_STATUS: u16 = 0x10;
const TX_ADDRESS: u16 = 0x20;
const RX_ADDRESS: u16 = 0x30;
const COMMAND: u16 = 0x37;
const RX_READ: u16 = 0x38;
const RX_WRITE: u16 = 0x3a;
const INTERRUPT_MASK: u16 = 0x3c;
const INTERRUPT_STATUS: u16 = 0x3e;
const TX_CONFIG: u16 = 0x40;
const RX_CONFIG: u16 = 0x44;
const CONFIG1: u16 = 0x52;
const MEDIA_STATUS: u16 = 0x58;

const RESET: u8 = 1 << 4;
const ENABLE_RX: u8 = 1 << 3;
const ENABLE_TX: u8 = 1 << 2;
const RX_EMPTY: u8 = 1;
const TX_AVAILABLE: u32 = 1 << 13;
const RX_RING: usize = 8192;
const TX_SLOTS: usize = 4;
const TX_BUFFER: usize = 1536;

#[repr(C, align(256))]
struct Dma {
    // The chip places a frame that crosses the ring boundary in the extra space.
    rx: [u8; RX_RING + 16 + 2048],
    tx: [[u8; TX_BUFFER]; TX_SLOTS],
}

static mut DMA: Dma = unsafe { core::mem::zeroed() };

pub struct Device {
    port: u16,
    rx: usize,
    tx: usize,
    mac: [u8; 6],
}

pub fn probe_pci(dev: PciDev) -> bool {
    (dev.vendor, dev.device) == (0x10ec, 0x8139)
}

impl Device {
    pub fn open(dev: PciDev) -> Result<Self, &'static str> {
        let read = |offset| pci::read_config(dev.bus, dev.dev, dev.fun, offset);
        let bar = read(0x10);
        let base = bar & !3;
        if bar & 1 == 0 || base == 0 || base > u16::MAX as u32 - 0xff {
            return Err("RTL8139 requires a legacy I/O BAR");
        }
        let dma = ptr::addr_of_mut!(DMA);
        if dma as u64 + size_of::<Dma>() as u64 > 1 << 32 {
            return Err("RTL8139 DMA lies outside the identity map");
        }

        let mut device = Self {
            port: base as u16,
            rx: 0,
            tx: 0,
            mac: [0; 6],
        };
        let command = read(4) & 0xffff;
        pci::write_config(dev.bus, dev.dev, dev.fun, 4, (command | 0x401) & !4);
        device.write8(CONFIG1, 0);
        device.write8(COMMAND, RESET);
        for _ in 0..100_000 {
            if device.read8(COMMAND) & RESET == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if device.read8(COMMAND) & RESET != 0 {
            return Err("RTL8139 reset timed out");
        }
        for index in 0..6 {
            device.mac[index] = device.read8(index as u16);
        }
        if device.mac == [0; 6] || device.mac[0] & 1 != 0 {
            return Err("RTL8139 has no valid MAC address");
        }

        device.write16(INTERRUPT_MASK, 0);
        device.write16(INTERRUPT_STATUS, u16::MAX);
        unsafe {
            device.write32(RX_ADDRESS, ptr::addr_of!((*dma).rx) as u32);
            for index in 0..TX_SLOTS {
                device.write32(
                    TX_ADDRESS + 4 * index as u16,
                    ptr::addr_of!((*dma).tx[index]) as u32,
                );
            }
        }
        device.write16(RX_READ, 0u16.wrapping_sub(16));
        pci::write_config(dev.bus, dev.dev, dev.fun, 4, command | 0x405);
        device.write8(COMMAND, ENABLE_RX | ENABLE_TX);
        device.write32(TX_CONFIG, (3 << 24) | (6 << 8));
        // Accept own unicast and broadcast; use an 8 KiB ring with no frame wrap.
        device.write32(
            RX_CONFIG,
            (7 << 13) | (7 << 8) | (1 << 7) | (1 << 3) | (1 << 1),
        );
        Ok(device)
    }

    fn read8(&self, reg: u16) -> u8 {
        unsafe { Port::<u8>::new(self.port + reg).read() }
    }

    fn read16(&self, reg: u16) -> u16 {
        unsafe { Port::<u16>::new(self.port + reg).read() }
    }

    fn read32(&self, reg: u16) -> u32 {
        unsafe { Port::<u32>::new(self.port + reg).read() }
    }

    fn write8(&self, reg: u16, value: u8) {
        unsafe {
            Port::<u8>::new(self.port + reg).write(value);
        }
    }

    fn write16(&self, reg: u16, value: u16) {
        unsafe {
            Port::<u16>::new(self.port + reg).write(value);
        }
    }

    fn write32(&self, reg: u16, value: u32) {
        fence(Ordering::Release);
        unsafe {
            Port::<u32>::new(self.port + reg).write(value);
        }
    }
}

impl Nic for Device {
    fn name(&self) -> &'static str {
        "Realtek RTL8139"
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        self.read8(MEDIA_STATUS) & (1 << 2) == 0
    }

    fn can_send(&self) -> bool {
        self.read32(TX_STATUS + 4 * self.tx as u16) & TX_AVAILABLE != 0
    }

    fn send(&mut self, frame: &[u8]) {
        debug_assert!(frame.len() <= FRAME_CAPACITY && self.can_send());
        let len = frame.len().max(60);
        fence(Ordering::Acquire);
        unsafe {
            let buffer = ptr::addr_of_mut!(DMA.tx[self.tx]).cast::<u8>();
            ptr::copy_nonoverlapping(frame.as_ptr(), buffer, frame.len());
            ptr::write_bytes(buffer.add(frame.len()), 0, len - frame.len());
        }
        self.write32(TX_STATUS + 4 * self.tx as u16, (8 << 16) | len as u32);
        self.tx = (self.tx + 1) % TX_SLOTS;
    }

    fn receive(&mut self) -> Option<Frame> {
        if self.read8(COMMAND) & RX_EMPTY != 0 {
            return None;
        }
        fence(Ordering::Acquire);
        let entry = unsafe { ptr::addr_of!(DMA.rx).cast::<u8>().add(self.rx) };
        let header = unsafe { entry.cast::<u32>().read_volatile() };
        let size = (header >> 16) as usize;
        if size == 0xfff0 {
            return None; // DMA hasn't finished writing the frame.
        }

        let mut frame = None;
        if (4..=2048).contains(&size) {
            let len = size - 4; // FCS
            if header & 1 != 0 && (14..=FRAME_CAPACITY).contains(&len) {
                let mut received = Frame::new(len);
                unsafe {
                    ptr::copy_nonoverlapping(entry.add(4), received.bytes().as_mut_ptr(), len);
                }
                frame = Some(received);
            }
            self.rx = (self.rx + 4 + size + 3) & !3;
        } else {
            // Discard the pending receive data when the header cannot be followed.
            self.rx = self.read16(RX_WRITE) as usize & !3;
        }
        self.rx %= RX_RING;
        fence(Ordering::Release);
        self.write16(RX_READ, (self.rx as u16).wrapping_sub(16));
        self.write16(INTERRUPT_STATUS, 0x51); // RX OK / overflow
        frame
    }
}
