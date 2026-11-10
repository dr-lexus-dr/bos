//! Minimal ELF64 loader
use x86_64::{VirtAddr, PhysAddr};
use crate::mm::paging::AddressSpace;

const ELFMAG: [u8; 4] = [0x7F, b'E', b'L', b'F'];

#[repr(C)]
struct Elf64Ehdr { magic: [u8;4], class: u8, data: u8, ver: u8, osabi: u8,
    _pad: [u8;8], etype: u16, machine: u16, version: u32, entry: u64,
    phoff: u64, shoff: u64, flags: u32, ehsize: u16, phentsize: u16,
    phnum: u16, shentsize: u16, shnum: u16, shstrndx: u16 }

#[repr(C)]
struct Elf64Phdr { ptype: u32, flags: u32, off: u64, vaddr: u64,
    paddr: u64, filesz: u64, memsz: u64, align: u64 }

pub fn is_elf(data: &[u8]) -> bool { data.len() > 4 && data[..4] == ELFMAG }

pub fn load_elf(_path: &str, _as: &mut AddressSpace) -> Result<u64, &'static str> {
    // Real impl: read via VFS, map PT_LOAD with paging::map_to, set entry.
    Err("elf loader: file not found (stub)")
}

pub fn load_elf_bytes(data: &[u8], as_: &mut AddressSpace) -> Result<u64, &'static str> {
    if !is_elf(data) { return Err("not elf"); }
    let eh = unsafe { &*(data.as_ptr() as *const Elf64Ehdr) };
    if eh.machine != 62 { return Err("not x86_64"); }
    let _ = as_;
    Ok(eh.entry)
}
