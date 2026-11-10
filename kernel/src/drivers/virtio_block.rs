//! Polling VirtIO block transport for the QEMU legacy PCI interface.

use core::{
    arch::asm,
    ptr,
    sync::atomic::{fence, Ordering},
};
use spin::Mutex;
use x86_64::instructions::port::Port;

use super::pci;

const SECTOR: usize = 512;
const TRANSFER: usize = 64 * 1024;
const FLUSH: u32 = 1 << 9;
const READ_ONLY: u32 = 1 << 5;
const QUEUE_LIMIT: u16 = 256;
const NEXT: u16 = 1;
const DEVICE_WRITES: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct Request {
    kind: u32,
    reserved: u32,
    sector: u64,
}

#[repr(C, align(4096))]
struct Dma {
    ring: [u8; 12288],
    request: Request,
    data: [u8; TRANSFER],
    status: u8,
}

// bLOADER places the kernel below 4 GiB and the BSP identity maps it.
static mut DMA: Dma = Dma {
    ring: [0; 12288],
    request: Request {
        kind: 0,
        reserved: 0,
        sector: 0,
    },
    data: [0; TRANSFER],
    status: 0,
};
static DEVICE: Mutex<Option<Device>> = Mutex::new(None);

struct Device {
    port: u16,
    capacity: u64,
    queue_size: u16,
    used_offset: usize,
    index: u16,
    failed: bool,
}

pub fn probe() -> Result<Option<u64>, &'static str> {
    let mut slot = DEVICE.lock();
    if let Some(device) = slot.as_ref() {
        return Ok(Some(device.capacity));
    }
    let mut candidates = pci::enumerate()
        .into_iter()
        .filter(|d| d.vendor == 0x1af4 && d.device == 0x1001);
    let Some(d) = candidates.next() else {
        return Ok(None);
    };
    if candidates.next().is_some() {
        return Err("expected exactly one VirtIO data disk");
    }
    let read = |offset| pci::read_config(d.bus, d.dev, d.fun, offset);
    let bar = read(0x10);
    if bar & 1 == 0 || bar & !3 > u16::MAX as u32 - 24 {
        return Err("VirtIO disk requires a legacy I/O BAR");
    }
    let port = (bar & !3) as u16;
    // I/O space and bus mastering, with INTx disabled (polling).
    pci::write_config(d.bus, d.dev, d.fun, 4, (read(4) & 0xffff) | 0x405);
    unsafe {
        Port::<u8>::new(port + 18).write(0);
        Port::<u8>::new(port + 18).write(1 | 2);
        let features = Port::<u32>::new(port).read();
        if features & (FLUSH | READ_ONLY) != FLUSH {
            return Err("VirtIO disk must be writable and support flush");
        }
        Port::<u32>::new(port + 4).write(FLUSH);
        Port::<u16>::new(port + 14).write(0);
        let queue_size = Port::<u16>::new(port + 12).read();
        if !(3..=QUEUE_LIMIT).contains(&queue_size) || !queue_size.is_power_of_two() {
            return Err("unsupported VirtIO queue size");
        }
        let used_offset = (16 * queue_size as usize + 6 + 2 * queue_size as usize + 4095) & !4095;
        let ring = ptr::addr_of_mut!(DMA.ring).cast::<u8>();
        let address = ring as usize;
        if address
            .checked_add(size_of::<Dma>())
            .is_none_or(|end| end > 1usize << 32)
        {
            return Err("VirtIO DMA memory lies outside the boot mapping");
        }
        ptr::write_bytes(ring, 0, 12288);
        Port::<u32>::new(port + 8).write((address >> 12) as u32);
        let low = Port::<u32>::new(port + 20).read();
        let high = Port::<u32>::new(port + 24).read();
        let capacity = (u64::from(high) << 32) | u64::from(low);
        Port::<u8>::new(port + 18).write(1 | 2 | 4);
        *slot = Some(Device {
            port,
            capacity,
            queue_size,
            used_offset,
            index: 0,
            failed: false,
        });
        Ok(Some(capacity))
    }
}

pub fn read(mut sector: u64, buffer: &mut [u8]) -> Result<(), &'static str> {
    let mut slot = DEVICE.lock();
    let device = slot.as_mut().ok_or("no VirtIO disk")?;
    for chunk in buffer.chunks_mut(TRANSFER) {
        device.transfer(0, sector, chunk.len(), None)?;
        unsafe {
            ptr::copy_nonoverlapping(
                ptr::addr_of!(DMA.data).cast(),
                chunk.as_mut_ptr(),
                chunk.len(),
            );
        }
        sector += (chunk.len() / SECTOR) as u64;
    }
    Ok(())
}

pub fn write(mut sector: u64, buffer: &[u8]) -> Result<(), &'static str> {
    let mut slot = DEVICE.lock();
    let device = slot.as_mut().ok_or("no VirtIO disk")?;
    for chunk in buffer.chunks(TRANSFER) {
        device.transfer(1, sector, chunk.len(), Some(chunk))?;
        sector += (chunk.len() / SECTOR) as u64;
    }
    Ok(())
}

pub fn flush() -> Result<(), &'static str> {
    DEVICE
        .lock()
        .as_mut()
        .ok_or("no VirtIO disk")?
        .transfer(4, 0, 0, None)
}

impl Device {
    fn transfer(
        &mut self,
        kind: u32,
        sector: u64,
        length: usize,
        data: Option<&[u8]>,
    ) -> Result<(), &'static str> {
        if self.failed {
            return Err("VirtIO queue is unavailable after a timeout");
        }
        if length % SECTOR != 0
            || sector
                .checked_add((length / SECTOR) as u64)
                .is_none_or(|end| end > self.capacity)
        {
            return Err("VirtIO transfer outside data disk");
        }
        unsafe {
            let ring = ptr::addr_of_mut!(DMA.ring).cast::<u8>();
            let desc = ring.cast::<Descriptor>();
            let request = ptr::addr_of_mut!(DMA.request);
            let payload = ptr::addr_of_mut!(DMA.data).cast::<u8>();
            let status = ptr::addr_of_mut!(DMA.status);
            if let Some(data) = data {
                ptr::copy_nonoverlapping(data.as_ptr(), payload, length);
            }
            request.write_volatile(Request {
                kind,
                reserved: 0,
                sector,
            });
            status.write_volatile(0xff);
            desc.write_volatile(Descriptor {
                address: request as u64,
                length: 16,
                flags: NEXT,
                next: 1,
            });
            let status_index = if length != 0 {
                desc.add(1).write_volatile(Descriptor {
                    address: payload as u64,
                    length: length as u32,
                    flags: NEXT | if kind == 0 { DEVICE_WRITES } else { 0 },
                    next: 2,
                });
                2
            } else {
                1
            };
            desc.add(status_index).write_volatile(Descriptor {
                address: status as u64,
                length: 1,
                flags: DEVICE_WRITES,
                next: 0,
            });

            let available = ring.add(16 * self.queue_size as usize).cast::<u16>();
            available.write_volatile(1); // suppress used-buffer interrupts
            available
                .add(2 + (self.index % self.queue_size) as usize)
                .write_volatile(0);
            fence(Ordering::SeqCst);
            self.index = self.index.wrapping_add(1);
            available.add(1).write_volatile(self.index);
            fence(Ordering::SeqCst);
            Port::<u16>::new(self.port + 16).write(0);
            let used_index = ring.add(self.used_offset + 2).cast::<u16>();
            let start = core::arch::x86_64::_rdtsc();
            while used_index.read_volatile() != self.index {
                // Bounded wait even when the device stops responding.
                if core::arch::x86_64::_rdtsc().wrapping_sub(start) > 5_000_000_000 {
                    self.failed = true;
                    return Err("VirtIO request timed out");
                }
                asm!("pause", options(nomem, nostack, preserves_flags));
            }
            fence(Ordering::SeqCst);
            let _ = Port::<u8>::new(self.port + 19).read();
            if status.read_volatile() != 0 {
                return Err("VirtIO request failed");
            }
        }
        Ok(())
    }
}
