//! Block device layer (RAM disk + ATA hook)
use alloc::vec::Vec;
use alloc::string::String;
use spin::RwLock;
pub trait BlockDevice: Send + Sync {
    fn name(&self) -> &str;
    fn sectors(&self) -> u64;
    fn read_sector(&self, lba: u64, buf: &mut [u8]) -> Result<(), &'static str>;
    fn write_sector(&self, lba: u64, buf: &[u8]) -> Result<(), &'static str>;
}
static DEVICES: RwLock<Vec<alloc::boxed::Box<dyn BlockDevice>>> = RwLock::new(Vec::new());
pub fn init() {}
pub fn register(d: alloc::boxed::Box<dyn BlockDevice>) { DEVICES.write().push(d); }
pub fn read(dev: &str, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    let _ = (dev, off, buf); Err("no block device")
}
pub fn write(dev: &str, off: u64, buf: &[u8]) -> Result<(), &'static str> {
    let _ = (dev, off, buf); Err("no block device")
}
pub struct RamDisk { pub name: String, pub data: spin::Mutex<Vec<u8>> }
impl BlockDevice for RamDisk {
    fn name(&self) -> &str { &self.name }
    fn sectors(&self) -> u64 { (self.data.lock().len() / 512) as u64 }
    fn read_sector(&self, lba: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        let d = self.data.lock(); let o = lba as usize * 512;
        buf.copy_from_slice(&d[o..o + buf.len().min(512)]); Ok(())
    }
    fn write_sector(&self, lba: u64, buf: &[u8]) -> Result<(), &'static str> {
        let mut d = self.data.lock(); let o = lba as usize * 512;
        d[o..o + buf.len().min(512)].copy_from_slice(&buf[..buf.len().min(512)]); Ok(())
    }
}
