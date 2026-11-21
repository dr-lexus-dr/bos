//! Polling ATA PIO for legacy IDE disks with 512-byte logical sectors.
use core::sync::atomic::{AtomicU8, Ordering};
use spin::Mutex;
use x86_64::instructions::port::Port;
static DRIVE: AtomicU8 = AtomicU8::new(0xff);
static BUS: Mutex<()> = Mutex::new(());
pub fn init() {}
pub fn handle_irq() {}
pub fn handle_irq_secondary() {}
fn ports(drive: u8) -> (u16, u16) {
    if drive < 2 {
        (0x1f0, 0x3f6)
    } else {
        (0x170, 0x376)
    }
}
unsafe fn inb(port: u16) -> u8 {
    Port::<u8>::new(port).read()
}
unsafe fn outb(port: u16, b: u8) {
    Port::<u8>::new(port).write(b)
}
unsafe fn delay(ctrl: u16) {
    for _ in 0..4 {
        let _ = inb(ctrl);
    }
}
unsafe fn wait(base: u16, drq: bool) -> Result<(), &'static str> {
    for _ in 0..100000 {
        let s = inb(base + 7);
        if s == 0 || s == 0xff {
            return Err("no ATA disk");
        }
        if s & 0x80 == 0 {
            if s & 0x21 != 0 {
                return Err("ATA I/O failed");
            }
            if !drq || s & 8 != 0 {
                return Ok(());
            }
        }
        core::hint::spin_loop();
    }
    Err("ATA timed out")
}
// Transfer one complete ATA sector using x86 string I/O. Emulators can handle
// the 256 words in one exit, and the device still sees 16-bit PIO cycles.
unsafe fn read_words(port: u16, destination: *mut u8) {
    core::arch::asm!("cld","rep insw",in("dx") port,inout("rdi") destination=>_,inout("rcx") 256usize=>_,options(nostack));
}
unsafe fn write_words(port: u16, source: *const u8) {
    core::arch::asm!("cld","rep outsw",in("dx") port,inout("rsi") source=>_,inout("rcx") 256usize=>_,options(nostack));
}
pub fn probe() -> Option<u64> {
    let _guard = BUS.lock();
    unsafe {
        for drive in 0..4 {
            let (base, ctrl) = ports(drive);
            outb(ctrl, 2);
            outb(base + 6, 0xa0 | ((drive & 1) << 4));
            delay(ctrl);
            for p in 2..6 {
                outb(base + p, 0);
            }
            outb(base + 7, 0xec);
            delay(ctrl);
            if wait(base, true).is_err() {
                continue;
            }
            let mut id = [0u16; 256];
            read_words(base, id.as_mut_ptr().cast());
            if id[49] & 0x200 == 0 || id[106] & 0xd000 == 0x5000 && (id[117] != 256 || id[118] != 0)
            {
                continue;
            }
            let sectors = ((id[61] as u64) << 16 | id[60] as u64).min(1 << 28);
            if sectors > 0 {
                DRIVE.store(drive, Ordering::Release);
                return Some(sectors);
            }
        }
    }
    None
}
unsafe fn command(drive: u8, lba: u32, cmd: u8) -> Result<u16, &'static str> {
    if drive >= 4 || lba >= 1 << 28 {
        return Err("ATA address out of range");
    }
    let (base, ctrl) = ports(drive);
    outb(base + 6, 0xe0 | ((drive & 1) << 4) | ((lba >> 24) as u8));
    delay(ctrl);
    outb(base + 2, 1);
    outb(base + 3, lba as u8);
    outb(base + 4, (lba >> 8) as u8);
    outb(base + 5, (lba >> 16) as u8);
    outb(base + 7, cmd);
    delay(ctrl);
    wait(base, true)?;
    Ok(base)
}
pub fn read_lba28(drive: u8, lba: u32, buf: &mut [u8]) -> Result<(), &'static str> {
    if buf.len() % 512 != 0 {
        return Err("unaligned ATA read");
    }
    let _guard = BUS.lock();
    for (i, sector) in buf.chunks_exact_mut(512).enumerate() {
        unsafe {
            let base = command(
                drive,
                lba.checked_add(i as u32).ok_or("ATA overflow")?,
                0x20,
            )?;
            read_words(base, sector.as_mut_ptr());
            wait(base, false)?;
        }
    }
    Ok(())
}
pub fn write_lba28(drive: u8, lba: u32, buf: &[u8]) -> Result<(), &'static str> {
    if buf.len() % 512 != 0 {
        return Err("unaligned ATA write");
    }
    let _guard = BUS.lock();
    for (i, sector) in buf.chunks_exact(512).enumerate() {
        unsafe {
            let base = command(
                drive,
                lba.checked_add(i as u32).ok_or("ATA overflow")?,
                0x30,
            )?;
            write_words(base, sector.as_ptr());
            wait(base, false)?;
        }
    }
    Ok(())
}
pub fn drive() -> u8 {
    DRIVE.load(Ordering::Acquire)
}
pub fn flush() -> Result<(), &'static str> {
    let _guard = BUS.lock();
    let (base, ctrl) = ports(drive());
    unsafe {
        outb(base + 7, 0xe7);
        delay(ctrl);
        wait(base, false)
    }
}
