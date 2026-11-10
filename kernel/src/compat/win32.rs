//! Minimal PE (Windows) loader -> native bOS mapping + Win32 shim
use crate::mm::paging::AddressSpace;

pub fn is_pe(data: &[u8]) -> bool { data.len() > 2 && data[0] == b'M' && data[1] == b'Z' }

pub fn load_pe(_path: &str, _as: &mut AddressSpace) -> Result<u64, &'static str> {
    Err("pe loader: file not found (stub)")
}

pub fn load_pe_bytes(data: &[u8], _as: &mut AddressSpace) -> Result<u64, &'static str> {
    if !is_pe(data) { return Err("not pe"); }
    // Real impl: parse COFF/optional header, map sections, resolve imports
    // kernel32->bOS: CreateFile->open, WriteFile->write, etc.
    Err("pe mapping not implemented yet")
}

/// Win32 API number -> bOS syscall
pub fn win32_to_bos(dll: &str, func: &str) -> Option<u64> {
    match (dll, func) {
        ("kernel32.dll", "WriteFile") | ("kernel32.dll", "WriteConsoleA") => Some(crate::syscall::SYS_WRITE),
        ("kernel32.dll", "ReadFile") | ("kernel32.dll", "ReadConsoleA") => Some(crate::syscall::SYS_READ),
        ("kernel32.dll", "ExitProcess") => Some(crate::syscall::SYS_EXIT),
        ("kernel32.dll", "GetCurrentProcessId") => Some(crate::syscall::SYS_GETPID),
        ("kernel32.dll", "Sleep") => Some(crate::syscall::SYS_SLEEP),
        _ => None,
    }
}
