//! Minimal Mach-O loader -> native mapping + libSystem shim
use crate::mm::paging::AddressSpace;

const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM_64: u32 = 0xcffa_edfe;

pub fn is_macho(data: &[u8]) -> bool {
    if data.len() < 4 { return false; }
    let m = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    m == MH_MAGIC_64 || m == MH_CIGAM_64
}

pub fn load_macho(_path: &str, _as: &mut AddressSpace) -> Result<u64, &'static str> {
    Err("macho loader: file not found (stub)")
}

pub fn libsystem_to_bos(func: &str) -> Option<u64> {
    match func {
        "_write" => Some(crate::syscall::SYS_WRITE),
        "_read" => Some(crate::syscall::SYS_READ),
        "_exit" => Some(crate::syscall::SYS_EXIT),
        "_getpid" => Some(crate::syscall::SYS_GETPID),
        _ => None,
    }
}
