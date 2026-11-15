//! Bounded polling SATA DMA transport (AHCI 1.3.1, one command slot).
//! Register/command layouts: Intel AHCI 1.3.1 sections 3.3 and 4.2.
//! Only a dedicated, labelled bOS-users disk is selected. No probe writes data.
use super::pci::{self, PciDev};
use core::{
    ptr,
    sync::atomic::{fence, Ordering},
};
use spin::Mutex;

#[repr(C, align(4096))]
struct Dma {
    list: [u8; 1024],
    fis: [u8; 256],
    table: [u8; 256],
    data: [u8; 65536],
}
static mut DMA: Dma = Dma {
    list: [0; 1024],
    fis: [0; 256],
    table: [0; 256],
    data: [0; 65536],
};
struct Device {
    port: u64,
    capacity: u64,
    failed: bool,
}
static DEVICE: Mutex<Option<Device>> = Mutex::new(None);
unsafe fn read(base: u64, offset: u64) -> u32 {
    ((base + offset) as *const u32).read_volatile()
}
unsafe fn write(base: u64, offset: u64, value: u32) {
    ((base + offset) as *mut u32).write_volatile(value);
}
fn wait(mut ready: impl FnMut() -> bool) -> Result<(), &'static str> {
    let start = crate::boottime::elapsed_ms();
    for _ in 0..10_000_000 {
        if ready() {
            return Ok(());
        }
        if crate::boottime::elapsed_ms().wrapping_sub(start) >= 2000 {
            break;
        }
        core::hint::spin_loop();
    }
    Err("AHCI timeout")
}
unsafe fn stop(port: u64) -> Result<(), &'static str> {
    write(port, 0x18, read(port, 0x18) & !1);
    wait(|| read(port, 0x18) & (1 << 15) == 0)?;
    write(port, 0x18, read(port, 0x18) & !(1 << 4));
    wait(|| read(port, 0x18) & (1 << 14) == 0)
}
unsafe fn start(port: u64) -> Result<(), &'static str> {
    stop(port)?;
    let list = ptr::addr_of_mut!(DMA.list) as u64;
    let fis = ptr::addr_of_mut!(DMA.fis) as u64;
    if (ptr::addr_of!(DMA) as u64) + core::mem::size_of::<Dma>() as u64 > 1 << 32 {
        return Err("AHCI DMA above 4 GiB");
    }
    ptr::write_bytes(
        ptr::addr_of_mut!(DMA).cast::<u8>(),
        0,
        core::mem::size_of::<Dma>(),
    );
    write(port, 0, list as u32);
    write(port, 4, 0);
    write(port, 8, fis as u32);
    write(port, 12, 0);
    write(port, 0x14, 0); // no INTx: polling, kernel owns completion
    write(port, 0x10, u32::MAX);
    write(port, 0x30, u32::MAX);
    write(port, 0x18, read(port, 0x18) | (1 << 4));
    write(port, 0x18, read(port, 0x18) | 1);
    Ok(())
}
impl Device {
    unsafe fn command(
        &mut self,
        command: u8,
        lba: u64,
        bytes: usize,
        outbound: bool,
    ) -> Result<(), &'static str> {
        if self.failed {
            return Err("AHCI device requires reboot after failure");
        }
        if bytes > 65536 || bytes % 512 != 0 || lba >= 1 << 48 {
            return Err("invalid AHCI request");
        }
        let result = (|| {
            wait(|| read(self.port, 0x20) & 0x88 == 0)?;
            if read(self.port, 0x38) != 0 || read(self.port, 0x34) != 0 {
                return Err("AHCI queue busy");
            }
            let table = ptr::addr_of_mut!(DMA.table).cast::<u8>();
            let list = ptr::addr_of_mut!(DMA.list).cast::<u8>();
            ptr::write_bytes(table, 0, 256);
            ptr::write_bytes(list, 0, 32);
            let flags =
                5u32 | if outbound { 1 << 6 } else { 0 } | if bytes != 0 { 1 << 16 } else { 0 };
            list.cast::<u32>().write_volatile(flags);
            list.add(8).cast::<u32>().write_volatile(table as u32);
            let fis = core::slice::from_raw_parts_mut(table, 20);
            fis[0] = 0x27;
            fis[1] = 0x80;
            fis[2] = command;
            fis[7] = 0x40;
            for (shift, offset) in [4, 5, 6, 8, 9, 10].into_iter().enumerate() {
                fis[offset] = (lba >> (shift * 8)) as u8;
            }
            if command != 0xec {
                let count = (bytes / 512) as u16;
                fis[12..14].copy_from_slice(&count.to_le_bytes());
            }
            if bytes != 0 {
                table
                    .add(128)
                    .cast::<u32>()
                    .write_volatile(ptr::addr_of!(DMA.data) as u32);
                table
                    .add(140)
                    .cast::<u32>()
                    .write_volatile((bytes - 1) as u32);
            }
            write(self.port, 0x10, u32::MAX);
            fence(Ordering::SeqCst);
            write(self.port, 0x38, 1);
            wait(|| read(self.port, 0x38) & 1 == 0 || read(self.port, 0x10) & (1 << 30) != 0)?;
            fence(Ordering::SeqCst);
            if read(self.port, 0x10) & 0x7d000000 != 0 || read(self.port, 0x20) & 1 != 0 {
                return Err("AHCI command error");
            }
            if bytes != 0 && list.add(4).cast::<u32>().read_volatile() != bytes as u32 {
                return Err("AHCI short transfer");
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
            // Keep static DMA reserved even if the controller cannot be stopped.
            let _ = stop(self.port);
        }
        result
    }
}
pub fn probe_pci(d: PciDev) -> bool {
    d.class == 1 && d.subclass == 6 && d.prog_if == 1
}
pub fn init() {}
pub fn probe() -> Result<Option<u64>, &'static str> {
    let mut slot = DEVICE.lock();
    if let Some(d) = slot.as_ref() {
        return Ok(Some(d.capacity));
    }
    for pci_dev in pci::enumerate().into_iter().filter(|d| probe_pci(*d)) {
        let d = pci_dev;
        let bar = pci::read_config(d.bus, d.dev, d.fun, 0x24);
        if bar & 1 != 0 || bar & !15 == 0 {
            continue;
        }
        let base = (bar & !15) as u64;
        let command = pci::read_config(d.bus, d.dev, d.fun, 4) & 0xffff;
        pci::write_config(d.bus, d.dev, d.fun, 4, command | 0x406);
        unsafe {
            if read(base, 0x24) & 1 != 0 {
                write(base, 0x28, read(base, 0x28) | 2);
                wait(|| read(base, 0x28) & 0x11 == 0)?;
            }
            write(base, 4, (read(base, 4) | (1 << 31)) & !2);
            let ports = read(base, 12);
            for index in 0..32 {
                if ports & (1 << index) == 0 {
                    continue;
                }
                let port = base + 0x100 + index * 0x80;
                if read(port, 0x28) & 0xf != 3 || read(port, 0x24) != 0x101 {
                    continue;
                }
                start(port)?;
                let mut device = Device {
                    port,
                    capacity: 0,
                    failed: false,
                };
                device.command(0xec, 0, 512, false)?;
                let words = ptr::addr_of!(DMA.data).cast::<u16>();
                if words.add(83).read_volatile() & (1 << 10) == 0 {
                    stop(port)?;
                    continue;
                }
                // Reject logical sectors larger than 512 rather than misaddress data.
                let sector_info = words.add(106).read_volatile();
                if sector_info & 0xc000 == 0x4000 && sector_info & (1 << 12) != 0 {
                    let size = words.add(117).read_volatile() as u32
                        | ((words.add(118).read_volatile() as u32) << 16);
                    if size != 256 {
                        stop(port)?;
                        continue;
                    }
                }
                let mut sectors = 0u64;
                for n in 0..4 {
                    sectors |= (words.add(100 + n).read_volatile() as u64) << (n * 16);
                }
                device.capacity = sectors;
                device.command(0x25, 0, 512, false)?;
                let header = core::slice::from_raw_parts(ptr::addr_of!(DMA.data).cast::<u8>(), 512);
                if &header[88..98] == b"bOS-users\0" {
                    *slot = Some(device);
                    log::info!("AHCI data disk: port {}, {} sectors", index, sectors);
                    return Ok(Some(sectors));
                }
                stop(port)?;
            }
        }
    }
    Ok(None)
}
pub fn read_blocks(mut lba: u64, mut output: &mut [u8]) -> Result<(), &'static str> {
    let mut guard = DEVICE.lock();
    let d = guard.as_mut().ok_or("no AHCI data disk")?;
    if output.len() % 512 != 0
        || lba
            .checked_add(output.len() as u64 / 512)
            .is_none_or(|end| end > d.capacity)
    {
        return Err("AHCI read out of bounds");
    }
    while !output.is_empty() {
        let n = output.len().min(65536);
        unsafe {
            d.command(0x25, lba, n, false)?;
            ptr::copy_nonoverlapping(ptr::addr_of!(DMA.data).cast::<u8>(), output.as_mut_ptr(), n);
        }
        output = &mut output[n..];
        lba += n as u64 / 512;
    }
    Ok(())
}
pub fn write_blocks(mut lba: u64, mut input: &[u8]) -> Result<(), &'static str> {
    let mut guard = DEVICE.lock();
    let d = guard.as_mut().ok_or("no AHCI data disk")?;
    if input.len() % 512 != 0
        || lba
            .checked_add(input.len() as u64 / 512)
            .is_none_or(|end| end > d.capacity)
    {
        return Err("AHCI write out of bounds");
    }
    while !input.is_empty() {
        let n = input.len().min(65536);
        unsafe {
            ptr::copy_nonoverlapping(input.as_ptr(), ptr::addr_of_mut!(DMA.data).cast::<u8>(), n);
            d.command(0x35, lba, n, true)?;
        }
        input = &input[n..];
        lba += n as u64 / 512;
    }
    Ok(())
}
pub fn flush() -> Result<(), &'static str> {
    let mut guard = DEVICE.lock();
    let d = guard.as_mut().ok_or("no AHCI data disk")?;
    unsafe { d.command(0xea, 0, 0, false) }
}
