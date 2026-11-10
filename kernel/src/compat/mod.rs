//! bOS binary compatibility: native loaders (no emulation)
//! ELF (Linux), PE (Windows), Mach-O (macOS) -> bOS ABI via syscall shims.

pub mod elf;
pub mod win32;
pub mod macho;

use log::info;
pub fn init() { info!("compat layer: elf/pe/macho native loaders ready"); }
