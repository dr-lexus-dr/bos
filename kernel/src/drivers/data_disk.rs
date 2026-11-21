//! Selection of the dedicated users volume. Transport never changes after mounting.
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
static KIND: AtomicU8 = AtomicU8::new(0);
static SECTORS: AtomicU64 = AtomicU64::new(0);
pub fn sectors() -> u64 {
    SECTORS.load(Ordering::Acquire)
}
pub fn probe() -> Result<Option<u64>, &'static str> {
    if let Some(sectors) = super::virtio_block::probe()? {
        SECTORS.store(sectors, Ordering::Release);
        KIND.store(1, Ordering::Release);
        return Ok(Some(sectors));
    }
    if let Some(sectors) = super::ahci::probe()? {
        SECTORS.store(sectors, Ordering::Release);
        KIND.store(2, Ordering::Release);
        return Ok(Some(sectors));
    }
    if let Some(sectors) = super::nvme::probe()? {
        SECTORS.store(sectors, Ordering::Release);
        KIND.store(3, Ordering::Release);
        return Ok(Some(sectors));
    }
    if let Some(sectors) = super::ata::probe() {
        SECTORS.store(sectors, Ordering::Release);
        KIND.store(4, Ordering::Release);
        return Ok(Some(sectors));
    }
    Ok(None)
}
pub fn name() -> &'static str {
    if KIND.load(Ordering::Acquire) == 4 {
        "ata0"
    } else if KIND.load(Ordering::Acquire) == 3 {
        "nvme0"
    } else if KIND.load(Ordering::Acquire) == 2 {
        "ahci0"
    } else {
        "virtio0"
    }
}
pub fn read(lba: u64, bytes: &mut [u8]) -> Result<(), &'static str> {
    match KIND.load(Ordering::Acquire) {
        1 => super::virtio_block::read(lba, bytes),
        2 => super::ahci::read_blocks(lba, bytes),
        3 => super::nvme::read_blocks(lba, bytes),
        4 => super::ata::read_lba28(
            super::ata::drive(),
            u32::try_from(lba).map_err(|_| "ATA overflow")?,
            bytes,
        ),
        _ => Err("no data disk"),
    }
}
pub fn write(lba: u64, bytes: &[u8]) -> Result<(), &'static str> {
    match KIND.load(Ordering::Acquire) {
        1 => super::virtio_block::write(lba, bytes),
        2 => super::ahci::write_blocks(lba, bytes),
        3 => super::nvme::write_blocks(lba, bytes),
        4 => super::ata::write_lba28(
            super::ata::drive(),
            u32::try_from(lba).map_err(|_| "ATA overflow")?,
            bytes,
        ),
        _ => Err("no data disk"),
    }
}
pub fn flush() -> Result<(), &'static str> {
    match KIND.load(Ordering::Acquire) {
        1 => super::virtio_block::flush(),
        2 => super::ahci::flush(),
        3 => super::nvme::flush(),
        4 => super::ata::flush(),
        _ => Err("no data disk"),
    }
}
