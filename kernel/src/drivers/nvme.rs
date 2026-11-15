//! Polled NVMe NVM transport: one controller, namespace and I/O queue pair.
//! NVMe Base / PCIe specifications: https://nvmexpress.org/specifications/
//! Probe is read-only and selects only the dedicated bOS-users volume.
use super::pci::{self, PciDev};
use core::{
    ptr,
    sync::atomic::{fence, Ordering},
};
use spin::Mutex;
const DEPTH: u16 = 32;
#[repr(C, align(4096))]
struct Page([u8; 4096]);
#[repr(C, align(4096))]
struct Dma {
    admin_sq: Page,
    admin_cq: Page,
    io_sq: Page,
    io_cq: Page,
    identify: Page,
    prps: Page,
    data: [u8; 65536],
}
static mut DMA: Dma = Dma {
    admin_sq: Page([0; 4096]),
    admin_cq: Page([0; 4096]),
    io_sq: Page([0; 4096]),
    io_cq: Page([0; 4096]),
    identify: Page([0; 4096]),
    prps: Page([0; 4096]),
    data: [0; 65536],
};
struct Queue {
    sq: u64,
    cq: u64,
    tail: u16,
    head: u16,
    phase: u32,
    cid: u16,
}
impl Queue {
    fn new(sq: u64, cq: u64) -> Self {
        Self {
            sq,
            cq,
            tail: 0,
            head: 0,
            phase: 1,
            cid: 0,
        }
    }
}
struct Device {
    base: u64,
    stride: u64,
    admin: Queue,
    io: Queue,
    ns: u32,
    capacity: u64,
    limit: usize,
    block_size: usize,
    failed: bool,
}
static DEVICE: Mutex<Option<Device>> = Mutex::new(None);
unsafe fn read(base: u64, off: u64) -> u32 {
    ((base + off) as *const u32).read_volatile()
}
unsafe fn write(base: u64, off: u64, value: u32) {
    ((base + off) as *mut u32).write_volatile(value);
}
unsafe fn write64(base: u64, off: u64, value: u64) {
    write(base, off, value as u32);
    write(base, off + 4, (value >> 32) as u32);
}
fn wait(mut ready: impl FnMut() -> bool) -> Result<(), &'static str> {
    let start = crate::boottime::elapsed_ms();
    for _ in 0..50_000_000 {
        if ready() {
            return Ok(());
        }
        if crate::boottime::elapsed_ms().wrapping_sub(start) >= 5000 {
            break;
        }
        core::hint::spin_loop();
    }
    Err("NVMe timeout")
}
impl Device {
    unsafe fn disable(&self) -> Result<(), &'static str> {
        write(self.base, 0x14, 0);
        wait(|| read(self.base, 0x1c) & 1 == 0)
    }
    unsafe fn command(&mut self, admin: bool, mut cmd: [u32; 16]) -> Result<(), &'static str> {
        if self.failed {
            return Err("NVMe device requires reboot after failure");
        }
        let q = if admin { &mut self.admin } else { &mut self.io };
        let qid = if admin { 0 } else { 1 };
        q.cid = q.cid.wrapping_add(1);
        cmd[0] |= (q.cid as u32) << 16;
        let dst = (q.sq as *mut u32).add(q.tail as usize * 16);
        for (i, v) in cmd.iter().enumerate() {
            dst.add(i).write_volatile(*v)
        }
        q.tail = (q.tail + 1) % DEPTH;
        fence(Ordering::SeqCst);
        write(self.base, 0x1000 + 2 * qid * self.stride, q.tail as u32);
        let completion = (q.cq as *const u32).add(q.head as usize * 4);
        let result = (|| {
            wait(|| {
                (completion.add(3).read_volatile() >> 16) & 1 == q.phase
                    || read(self.base, 0x1c) & 2 != 0
            })?;
            fence(Ordering::SeqCst);
            if read(self.base, 0x1c) & 2 != 0 {
                return Err("NVMe controller fatal status");
            }
            let status = completion.add(3).read_volatile();
            if status as u16 != q.cid || completion.add(2).read_volatile() >> 16 != qid as u32 {
                return Err("NVMe unexpected completion");
            }
            q.head += 1;
            if q.head == DEPTH {
                q.head = 0;
                q.phase ^= 1;
            }
            write(
                self.base,
                0x1000 + (2 * qid + 1) * self.stride,
                q.head as u32,
            );
            if status >> 17 != 0 {
                log::warn!("NVMe command status {:x}", status >> 17);
                return Err("NVMe command failed");
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
            let _ = self.disable();
        }
        result
    }
    unsafe fn transfer(&mut self, opcode: u8, lba: u64, bytes: usize) -> Result<(), &'static str> {
        let data = ptr::addr_of_mut!(DMA.data) as u64;
        let mut cmd = [0; 16];
        cmd[0] = opcode as u32;
        cmd[1] = self.ns;
        if bytes != 0 {
            cmd[6] = data as u32;
            cmd[7] = (data >> 32) as u32;
            let prp2 = if bytes > 8192 {
                let list = ptr::addr_of_mut!(DMA.prps).cast::<u64>();
                for i in 1..bytes.div_ceil(4096) {
                    list.add(i - 1).write_volatile(data + (i * 4096) as u64)
                }
                list as u64
            } else if bytes > 4096 {
                data + 4096
            } else {
                0
            };
            cmd[8] = prp2 as u32;
            cmd[9] = (prp2 >> 32) as u32;
            cmd[10] = lba as u32;
            cmd[11] = (lba >> 32) as u32;
            cmd[12] = (bytes / self.block_size - 1) as u32;
        }
        self.command(false, cmd)
    }
}
fn map_mmio(virt: u64, phys: u64) -> Result<(), &'static str> {
    use x86_64::{structures::paging::PageTableFlags as F, PhysAddr, VirtAddr};
    crate::mm::paging::map_to(
        VirtAddr::new(virt),
        PhysAddr::new(phys),
        F::PRESENT | F::WRITABLE | F::NO_EXECUTE | F::NO_CACHE | F::WRITE_THROUGH,
    )
    .map_err(|_| "NVMe MMIO mapping failed")
}
pub fn probe_pci(d: PciDev) -> bool {
    d.class == 1 && d.subclass == 8 && d.prog_if == 2
}
pub fn init() {}
pub fn probe() -> Result<Option<u64>, &'static str> {
    let mut slot = DEVICE.lock();
    if let Some(d) = slot.as_ref() {
        return Ok(Some(d.capacity));
    }
    for pci_dev in pci::enumerate().into_iter().filter(|d| probe_pci(*d)) {
        let bar = pci::read_config(pci_dev.bus, pci_dev.dev, pci_dev.fun, 0x10);
        if bar & 1 != 0 {
            continue;
        }
        let high = if bar & 6 == 4 {
            pci::read_config(pci_dev.bus, pci_dev.dev, pci_dev.fun, 0x14)
        } else {
            0
        };
        let physical = ((high as u64) << 32) | (bar & !15) as u64;
        if physical == 0 {
            continue;
        }
        // A dedicated supervisor-only uncached mapping also supports PCI BARs
        // above 4 GiB, which OVMF routinely assigns to NVMe controllers.
        let bdf = ((pci_dev.bus as u64) << 8) | ((pci_dev.dev as u64) << 3) | pci_dev.fun as u64;
        let base = 0xffff_9000_0000_0000 + (bdf << 20);
        map_mmio(base, physical)?;
        let command = pci::read_config(pci_dev.bus, pci_dev.dev, pci_dev.fun, 4) & 0xffff;
        pci::write_config(pci_dev.bus, pci_dev.dev, pci_dev.fun, 4, command | 0x406);
        unsafe {
            let cap = read(base, 0) as u64 | ((read(base, 4) as u64) << 32);

            if cap & 0xffff < (DEPTH - 1) as u64 || cap & (1 << 37) == 0 || (cap >> 48) & 15 != 0 {
                continue;
            }
            let stride = 4 << ((cap >> 32) & 15);
            for offset in (4096..(0x1000 + 3 * stride + 4 + 4095) & !4095).step_by(4096) {
                map_mmio(base + offset, physical + offset)?;
            }
            let mut d = Device {
                base,
                stride,
                admin: Queue::new(
                    ptr::addr_of_mut!(DMA.admin_sq) as u64,
                    ptr::addr_of_mut!(DMA.admin_cq) as u64,
                ),
                io: Queue::new(
                    ptr::addr_of_mut!(DMA.io_sq) as u64,
                    ptr::addr_of_mut!(DMA.io_cq) as u64,
                ),
                ns: 1,
                capacity: 0,
                limit: 65536,
                block_size: 512,
                failed: false,
            };
            d.disable()?;
            ptr::write_bytes(
                ptr::addr_of_mut!(DMA).cast::<u8>(),
                0,
                core::mem::size_of::<Dma>(),
            );
            write(base, 0x0c, u32::MAX);
            write(base, 0x24, ((DEPTH as u32 - 1) << 16) | (DEPTH as u32 - 1));
            write64(base, 0x28, d.admin.sq);
            write64(base, 0x30, d.admin.cq);
            write(base, 0x14, 1 | (6 << 16) | (4 << 20));
            if let Err(e) = wait(|| read(base, 0x1c) & 3 != 0) {
                let _ = d.disable();
                return Err(e);
            }
            if read(base, 0x1c) & 2 != 0 {
                let _ = d.disable();
                return Err("NVMe initialization failed");
            }
            let identify = ptr::addr_of_mut!(DMA.identify).cast::<u8>();
            let mut cmd = [0; 16];
            cmd[0] = 6;
            cmd[6] = identify as u32;
            cmd[7] = ((identify as u64) >> 32) as u32;
            cmd[10] = 1;
            d.command(true, cmd)?;
            let mdts = identify.add(77).read_volatile();
            if mdts != 0 {
                d.limit = 4096usize
                    .checked_shl(mdts as u32)
                    .unwrap_or(65536)
                    .min(65536);
            }
            // Enumerate active namespaces, rather than assuming namespace 1 exists.
            cmd[10] = 2;
            d.command(true, cmd)?;
            let mut namespaces = [0u32; 1024];
            for (i, ns) in namespaces.iter_mut().enumerate() {
                *ns = identify.cast::<u32>().add(i).read_volatile();
            }
            cmd = [0; 16];
            cmd[0] = 5;
            cmd[6] = d.io.cq as u32;
            cmd[7] = (d.io.cq >> 32) as u32;
            cmd[10] = ((DEPTH as u32 - 1) << 16) | 1;
            cmd[11] = 1;
            d.command(true, cmd)?;
            cmd[0] = 1;
            cmd[6] = d.io.sq as u32;
            cmd[7] = (d.io.sq >> 32) as u32;
            cmd[11] = (1 << 16) | 1;
            d.command(true, cmd)?;
            for ns in namespaces.into_iter().take_while(|ns| *ns != 0) {
                cmd = [0; 16];
                cmd[0] = 6;
                cmd[1] = ns;
                cmd[6] = identify as u32;
                cmd[7] = ((identify as u64) >> 32) as u32;
                d.command(true, cmd)?;
                log::info!("NVMe namespace {}", ns);
                let format = identify.add(26).read_volatile();
                if format & 0xf0 != 0 {
                    continue;
                }
                let lbaf = identify.add(128 + 4 * (format as usize));
                if lbaf.cast::<u16>().read_volatile() != 0
                    || !(9..=12).contains(&lbaf.add(2).read_volatile())
                    || identify.add(29).read_volatile() & 7 != 0
                {
                    continue;
                }
                d.block_size = 1 << lbaf.add(2).read_volatile();
                let Some(capacity) = identify
                    .cast::<u64>()
                    .read_volatile()
                    .checked_mul((d.block_size / 512) as u64)
                else {
                    continue;
                };
                d.capacity = capacity;
                d.ns = ns;
                if d.capacity == 0 {
                    continue;
                }
                d.transfer(2, 0, d.block_size)?;
                let label =
                    core::slice::from_raw_parts(ptr::addr_of!(DMA.data).cast::<u8>().add(88), 10);
                if label == b"bOS-users\0" {
                    let capacity = d.capacity;
                    log::info!(
                        "NVMe data disk: namespace {}, {} sectors, LBA {}",
                        ns,
                        capacity,
                        d.block_size
                    );
                    *slot = Some(d);
                    return Ok(Some(capacity));
                }
            }
            d.disable()?;
        }
    }
    Ok(None)
}
pub fn read_blocks(mut lba: u64, mut output: &mut [u8]) -> Result<(), &'static str> {
    let mut slot = DEVICE.lock();
    let d = slot.as_mut().ok_or("no NVMe data disk")?;
    if output.len() % 512 != 0
        || lba
            .checked_add(output.len() as u64 / 512)
            .is_none_or(|end| end > d.capacity)
    {
        return Err("NVMe read out of bounds");
    }
    while !output.is_empty() {
        let ratio = (d.block_size / 512) as u64;
        let offset = (lba % ratio) as usize * 512;
        let n = output.len().min(d.limit - offset);
        let bytes = (offset + n).div_ceil(d.block_size) * d.block_size;
        unsafe {
            d.transfer(2, lba / ratio, bytes)?;
            ptr::copy_nonoverlapping(
                ptr::addr_of!(DMA.data).cast::<u8>().add(offset),
                output.as_mut_ptr(),
                n,
            );
        }
        lba += n as u64 / 512;
        output = &mut output[n..];
    }
    Ok(())
}
pub fn write_blocks(mut lba: u64, mut input: &[u8]) -> Result<(), &'static str> {
    let mut slot = DEVICE.lock();
    let d = slot.as_mut().ok_or("no NVMe data disk")?;
    if input.len() % 512 != 0
        || lba
            .checked_add(input.len() as u64 / 512)
            .is_none_or(|end| end > d.capacity)
    {
        return Err("NVMe write out of bounds");
    }
    while !input.is_empty() {
        let ratio = (d.block_size / 512) as u64;
        let offset = (lba % ratio) as usize * 512;
        let n = input.len().min(d.limit - offset);
        let bytes = (offset + n).div_ceil(d.block_size) * d.block_size;
        unsafe {
            // The shared device lock serializes partial native-sector updates.
            // Preserve adjacent 512-byte sectors on devices with larger LBAs.
            if offset != 0 || n != bytes {
                d.transfer(2, lba / ratio, bytes)?;
            }
            ptr::copy_nonoverlapping(
                input.as_ptr(),
                ptr::addr_of_mut!(DMA.data).cast::<u8>().add(offset),
                n,
            );
            d.transfer(1, lba / ratio, bytes)?;
        }
        lba += n as u64 / 512;
        input = &input[n..];
    }
    Ok(())
}
pub fn flush() -> Result<(), &'static str> {
    let mut slot = DEVICE.lock();
    let d = slot.as_mut().ok_or("no NVMe data disk")?;
    unsafe { d.transfer(0, 0, 0) }
}
