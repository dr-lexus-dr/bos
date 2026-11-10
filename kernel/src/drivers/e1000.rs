//! Polling Ethernet driver for Intel 82540EM and 82574L.
use core::{
    ptr,
    sync::atomic::{fence, Ordering},
};
use x86_64::{structures::paging::PageTableFlags as Flags, PhysAddr, VirtAddr};

use super::{
    net::{Frame, Nic, FRAME_CAPACITY},
    pci::{self, PciDev},
};
use crate::arch::x86_64::paging;

const MMIO: u64 = 0xffff_9000_0000_0000;
const RING: usize = 16;
const BUFFER: usize = 2048;

const CTRL: u64 = 0x0000;
const STATUS: u64 = 0x0008;
const ICR: u64 = 0x00c0;
const IMC: u64 = 0x00d8;
const RCTL: u64 = 0x0100;
const TCTL: u64 = 0x0400;
const TIPG: u64 = 0x0410;
const RX_RING: u64 = 0x2800;
const TX_RING: u64 = 0x3800;
const RAL: u64 = 0x5400;
const RAH: u64 = 0x5404;

#[derive(Clone, Copy)]
#[repr(C)]
struct Descriptor {
    address: u64,
    data: u64,
}

#[repr(C, align(128))]
struct Dma {
    rx: [Descriptor; RING],
    tx: [Descriptor; RING],
    rx_buffers: [[u8; BUFFER]; RING],
    tx_buffers: [[u8; BUFFER]; RING],
}

// Sole owner is the NIC behind the network mutex, on the boot CPU.
static mut DMA: Dma = unsafe { core::mem::zeroed() };

pub struct Device {
    rx: usize,
    tx: usize,
    mac: [u8; 6],
    model: &'static str,
}

pub fn probe_pci(dev: PciDev) -> bool {
    dev.vendor == 0x8086 && matches!(dev.device, 0x100e | 0x10d3)
}

impl Device {
    pub fn open(dev: PciDev) -> Result<Self, &'static str> {
        let read = |offset| pci::read_config(dev.bus, dev.dev, dev.fun, offset);
        let bar = read(0x10);
        let high = match bar & 7 {
            0 => 0,
            4 => read(0x14),
            _ => return Err("Intel Ethernet controller has no MMIO BAR"),
        };
        let physical = u64::from(bar & !15) | (u64::from(high) << 32);
        if physical == 0 || physical >= (1 << 52) - 0x6000 || physical & 4095 != 0 {
            return Err("invalid Intel Ethernet MMIO BAR");
        }
        for offset in [0, RX_RING & !4095, TX_RING & !4095, RAL & !4095] {
            paging::map_to(
                VirtAddr::new(MMIO + offset),
                PhysAddr::new(physical + offset),
                Flags::PRESENT | Flags::WRITABLE | Flags::NO_CACHE | Flags::NO_EXECUTE,
            )
            .map_err(|_| "Intel Ethernet MMIO mapping failed")?;
        }
        let dma = ptr::addr_of_mut!(DMA);
        if dma as u64 + size_of::<Dma>() as u64 > 1 << 32 {
            return Err("Intel Ethernet DMA lies outside the identity map");
        }

        let command = read(4) & 0xffff;
        pci::write_config(dev.bus, dev.dev, dev.fun, 4, (command | 0x402) & !4);
        write(IMC, u32::MAX);
        write(CTRL, reg(CTRL) | 1 << 26);
        for _ in 0..100_000 {
            if reg(CTRL) & (1 << 26) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if reg(CTRL) & (1 << 26) != 0 {
            return Err("Intel Ethernet reset timed out");
        }
        write(IMC, u32::MAX);
        let _ = reg(ICR);
        let low = reg(RAL).to_le_bytes();
        let high = reg(RAH).to_le_bytes();
        let mac = [low[0], low[1], low[2], low[3], high[0], high[1]];
        if mac == [0; 6] || mac[0] & 1 != 0 {
            return Err("Intel Ethernet controller has no valid MAC address");
        }
        unsafe {
            for index in 0..RING {
                ptr::addr_of_mut!((*dma).rx[index]).write(Descriptor {
                    address: ptr::addr_of!((*dma).rx_buffers[index]) as u64,
                    data: 0,
                });
                ptr::addr_of_mut!((*dma).tx[index]).write(Descriptor {
                    address: ptr::addr_of!((*dma).tx_buffers[index]) as u64,
                    data: 1 << 32, // descriptor done
                });
            }
            set_ring(RX_RING, ptr::addr_of!((*dma).rx) as u64, (RING - 1) as u32);
            set_ring(TX_RING, ptr::addr_of!((*dma).tx) as u64, 0);
        }
        write(RAH, reg(RAH) | 1 << 31);
        write(CTRL, reg(CTRL) | 1 << 6); // set link up
        pci::write_config(dev.bus, dev.dev, dev.fun, 4, command | 0x406);
        write(TIPG, 10 | (8 << 10) | (6 << 20));
        write(TCTL, (1 << 1) | (1 << 3) | (15 << 4) | (64 << 12));
        write(RCTL, (1 << 1) | (1 << 15) | (1 << 26)); // RX, broadcast, strip CRC
        let model = if dev.device == 0x10d3 {
            "Intel 82574L"
        } else {
            "Intel 82540EM"
        };
        Ok(Self {
            rx: 0,
            tx: 0,
            mac,
            model,
        })
    }
}

impl Nic for Device {
    fn name(&self) -> &'static str {
        self.model
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        reg(STATUS) & 2 != 0
    }

    fn can_send(&self) -> bool {
        unsafe { ptr::addr_of!(DMA.tx[self.tx].data).read_volatile() & (1 << 32) != 0 }
    }

    fn send(&mut self, frame: &[u8]) {
        debug_assert!(frame.len() <= FRAME_CAPACITY && self.can_send());
        let index = self.tx;
        fence(Ordering::Acquire);
        unsafe {
            ptr::copy_nonoverlapping(
                frame.as_ptr(),
                ptr::addr_of_mut!(DMA.tx_buffers[index]).cast(),
                frame.len(),
            );
            // EOP, insert FCS, report status.
            ptr::addr_of_mut!(DMA.tx[index].data).write_volatile(frame.len() as u64 | (0x0b << 24));
        }
        self.tx = (index + 1) % RING;
        write(TX_RING + 0x18, self.tx as u32);
    }

    fn receive(&mut self) -> Option<Frame> {
        let index = self.rx;
        let descriptor = unsafe { ptr::addr_of_mut!(DMA.rx[index].data) };
        let data = unsafe { descriptor.read_volatile() };
        if data & (1 << 32) == 0 {
            return None;
        }
        fence(Ordering::Acquire);
        let len = data as u16 as usize;
        let frame = if data & (1 << 33) != 0
            && (data >> 40) as u8 == 0
            && (14..=FRAME_CAPACITY).contains(&len)
        {
            let mut frame = Frame::new(len);
            unsafe {
                ptr::copy_nonoverlapping(
                    ptr::addr_of!(DMA.rx_buffers[index]).cast(),
                    frame.bytes().as_mut_ptr(),
                    len,
                );
            }
            Some(frame)
        } else {
            None
        };
        unsafe {
            descriptor.write_volatile(0);
        }
        self.rx = (index + 1) % RING;
        write(RX_RING + 0x18, index as u32);
        frame
    }
}

fn reg(offset: u64) -> u32 {
    unsafe { ((MMIO + offset) as *const u32).read_volatile() }
}

fn write(offset: u64, value: u32) {
    fence(Ordering::Release);
    unsafe {
        ((MMIO + offset) as *mut u32).write_volatile(value);
    }
}

fn set_ring(register: u64, address: u64, tail: u32) {
    write(register, address as u32);
    write(register + 4, (address >> 32) as u32);
    write(register + 8, (RING * size_of::<Descriptor>()) as u32);
    write(register + 0x10, 0);
    write(register + 0x18, tail);
}
