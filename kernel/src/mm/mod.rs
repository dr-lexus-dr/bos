//! Memory Management Module

pub mod phys;
pub mod heap;
pub mod paging;

use crate::mm::phys::init as phys_init;
use crate::mm::heap::init as heap_init;

/// Initialize memory management
pub fn init(memory_map: &[bloader_shared::MemoryRegion]) {
    phys_init(memory_map);
    heap_init();
}