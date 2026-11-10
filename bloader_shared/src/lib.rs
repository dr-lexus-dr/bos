#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use uuid::Uuid;
use bitflags::bitflags;
use core::fmt;

/// Magic number for bOS kernel identification
pub const KERNEL_MAGIC: u64 = 0x424F534B45524E4C; // "BOSKERNL" in ASCII
pub const BLOADER_MAGIC: u64 = 0x424C4F4144455221; // "BLOADER!" in ASCII

/// Boot protocol version
pub const BOOT_PROTOCOL_VERSION: u32 = 1;

/// Maximum command line length
pub const MAX_CMDLINE_LEN: usize = 4096;

/// Memory map entry types (UEFI specification)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryType {
    Reserved = 0,
    LoaderCode = 1,
    LoaderData = 2,
    BootServicesCode = 3,
    BootServicesData = 4,
    RuntimeServicesCode = 5,
    RuntimeServicesData = 6,
    Conventional = 7,
    Unusable = 8,
    ACPIReclaim = 9,
    ACPIMemoryNVS = 10,
    MemoryMappedIO = 11,
    MemoryMappedIOPortSpace = 12,
    PalCode = 13,
    PersistentMemory = 14,
    KernelCode = 0x8000_0000,
    KernelData = 0x8000_0001,
    KernelStack = 0x8000_0002,
    KernelHeap = 0x8000_0003,
    Modules = 0x8000_0004,
    Framebuffer = 0x8000_0005,
    BFSMetadata = 0x8000_0006,
}

impl MemoryType {
    pub fn is_available(&self) -> bool {
        matches!(self, MemoryType::Conventional | MemoryType::PersistentMemory)
    }

    pub fn is_kernel(&self) -> bool {
        (*self as u32) >= 0x8000_0000
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MemoryFlags: u64 {
        const READ        = 1 << 0;
        const WRITE       = 1 << 1;
        const EXECUTE     = 1 << 2;
        const PRESENT     = 1 << 3;
        const USER_ACCESS = 1 << 4;
        const NX          = 1 << 5;
        const HUGE_PAGE   = 1 << 6;
        const NO_CACHE    = 1 << 7;
        const WRITE_COMBINE = 1 << 8;
    }
}

/// Memory region descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MemoryRegion {
    pub base_addr: u64,
    pub length: u64,
    pub region_type: MemoryType,
    pub flags: MemoryFlags,
}

impl MemoryRegion {
    pub fn end_addr(&self) -> u64 {
        self.base_addr + self.length
    }

    pub fn page_count(&self) -> u64 {
        (self.length + 4095) / 4096
    }
}

/// Framebuffer information
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct FramebufferInfo {
    pub base_addr: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: PixelFormat,
    pub bits_per_pixel: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    RGB = 0,
    BGR = 1,
    Bitmask = 2,
    BGRA = 3,
    RGBA = 4,
}

/// Module information (initrd, dtb, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ModuleInfo {
    pub name: [u8; 64],
    pub base_addr: u64,
    pub size: u64,
    pub flags: u32,
}

impl ModuleInfo {
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// Boot information passed from bootloader to kernel
#[derive(Debug, Clone)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub kernel_entry: u64,
    pub kernel_size: u64,
    pub memory_map: Vec<MemoryRegion>,
    pub memory_map_entries: usize,
    pub framebuffer: Option<FramebufferInfo>,
    pub rsdp_addr: Option<u64>, // ACPI RSDP address
    pub cmdline: [u8; MAX_CMDLINE_LEN],
    pub cmdline_len: usize,
    pub modules: Vec<ModuleInfo>,
    pub boot_time_us: u64,
    pub bootloader_name: [u8; 64],
    pub bootloader_version: [u8; 32],
    pub cpu_count: u32,
    pub bsp_lapic_id: u32,
    pub kernel_uuid: Uuid,
    pub initrd_addr: Option<u64>,
    pub initrd_size: Option<u64>,
}

impl BootInfo {
    pub fn cmdline_str(&self) -> &str {
        let slice = &self.cmdline[..self.cmdline_len];
        core::str::from_utf8(slice).unwrap_or("")
    }

    pub fn bootloader_name_str(&self) -> &str {
        let end = self.bootloader_name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.bootloader_name[..end]).unwrap_or("")
    }

    pub fn bootloader_version_str(&self) -> &str {
        let end = self.bootloader_version.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.bootloader_version[..end]).unwrap_or("")
    }
}

/// Kernel entry point signature
pub type KernelEntryFn = extern "sysv64" fn(&'static BootInfo) -> !;

/// Kernel header - placed at the beginning of kernel binary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KernelHeader {
    pub magic: u64,
    pub version: u32,
    pub entry_offset: u64,
    pub load_addr: u64,
    pub size: u64,
    pub flags: KernelFlags,
    pub required_memory: u64,
    pub uuid: Uuid,
    pub name: [u8; 64],
    pub cmdline_default: [u8; 256],
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct KernelFlags: u64 {
        const RELOCATABLE      = 1 << 0;
        const REQUIRES_SMP     = 1 << 1;
        const REQUIRES_ACPI    = 1 << 2;
        const REQUIRES_FRAMEBUFFER = 1 << 3;
        const DEBUG            = 1 << 4;
        const KASLR            = 1 << 5;
        const COMPRESSED       = 1 << 6;
        const HAS_INITRD       = 1 << 7;
    }
}

impl KernelHeader {
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// RSDP (Root System Description Pointer) for ACPI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, packed)]
pub struct RSDP {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

impl RSDP {
    pub const SIGNATURE: [u8; 8] = *b"RSD PTR ";
}

/// Error codes for boot process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum BootError {
    Success = 0,
    InvalidMagic = 1,
    UnsupportedVersion = 2,
    KernelNotFound = 3,
    KernelCorrupted = 4,
    InsufficientMemory = 5,
    InvalidMemoryMap = 6,
    InvalidKernelHeader = 7,
    RelocationFailed = 8,
    EntryPointMissing = 9,
    ACPIMissing = 10,
    FramebufferMissing = 11,
    ModuleLoadFailed = 12,
    InvalidCommandLine = 13,
    UUIDMismatch = 14,
    ChecksumMismatch = 15,
    UnsupportedArchitecture = 16,
    SecureBootViolation = 17,
    DriverLoadFailed = 18,
    FilesystemError = 19,
    Unknown = 0xFFFF_FFFF,
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_magic() {
        assert_eq!(KERNEL_MAGIC, 0x424F534B45524E4C);
    }

    #[test]
    fn test_memory_region_page_count() {
        let region = MemoryRegion {
            base_addr: 0x100000,
            length: 0x100000,
            region_type: MemoryType::Conventional,
            flags: MemoryFlags::READ | MemoryFlags::WRITE | MemoryFlags::EXECUTE,
        };
        assert_eq!(region.page_count(), 256);
    }
}