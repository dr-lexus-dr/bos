//! Physical Memory Manager

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use alloc::vec::Vec;
use alloc::vec;
use alloc::collections::BTreeMap;
use x86_64::{
    structures::paging::PhysFrame,
    PhysAddr, VirtAddr,
};
use crate::arch::x86_64::paging::PHYS_MEM_OFFSET;
use log::{info, trace, warn};
use bloader_shared::{MemoryFlags, MemoryRegion, MemoryType};

/// Physical frame size
pub const FRAME_SIZE: u64 = 4096;

/// Maximum physical address (52-bit = 4 PB)
const MAX_PHYS_ADDR: u64 = 1u64 << 52;

/// Frame allocator state
static FRAME_ALLOCATOR: spin::Mutex<Option<FrameAllocator>> = spin::Mutex::new(None);

/// Total physical memory
static TOTAL_MEMORY: AtomicU64 = AtomicU64::new(0);
/// Used physical memory
static USED_MEMORY: AtomicU64 = AtomicU64::new(0);
/// Free physical memory
static FREE_MEMORY: AtomicU64 = AtomicU64::new(0);

/// Static backing for the memory-map Vec.
///
/// `FrameAllocator::new` runs before the heap exists, so the map cannot be
/// copied with `to_vec()`. The buffer lives forever; the Vec built on top of
/// it must never grow (nothing pushes to `memory_map` after init) and must
/// never be dropped — both hold for the singleton in `FRAME_ALLOCATOR`.
const MAX_MAP_ENTRIES: usize = 256;
static mut MAP_BUF: [MemoryRegion; MAX_MAP_ENTRIES] = [MemoryRegion {
    base_addr: 0,
    length: 0,
    region_type: MemoryType::Reserved,
    flags: MemoryFlags::empty(),
}; MAX_MAP_ENTRIES];

/// Static backing for the frame bitmap (same no-heap constraint as above).
/// 262144 words = 2 MiB .bss, covers 64 GiB of RAM.  The previous 16 GiB cap
/// made a 32 GiB QEMU guest panic during boot with "frame bitmap needs ...".
const MAX_BITMAP_WORDS: usize = 262_144;
static mut BITMAP_BUF: [u64; MAX_BITMAP_WORDS] = [0; MAX_BITMAP_WORDS];

/// Bitmap allocator for frames
pub struct FrameAllocator {
    /// Memory map from bootloader
    memory_map: Vec<MemoryRegion>,
    /// Bitmap of used frames (1 bit per frame)
    bitmap: Vec<u64>,
    /// Total frames
    total_frames: usize,
    /// Free frames
    free_frames: usize,
    /// Start of managed memory
    managed_start: u64,
    /// End of managed memory
    managed_end: u64,
    /// Allocation hint (next free frame to check)
    alloc_hint: usize,
}

impl FrameAllocator {
    /// Create new frame allocator from memory map.
    ///
    /// Runs before the heap exists: `memory_map` is placed into the static
    /// `MAP_BUF` (no allocation). Panics if the map is larger than
    /// `MAX_MAP_ENTRIES`.
    pub fn new(memory_map: &[MemoryRegion]) -> Self {
        assert!(
            memory_map.len() <= MAX_MAP_ENTRIES,
            "memory map has {} entries, static buffer holds {}",
            memory_map.len(),
            MAX_MAP_ENTRIES
        );
        let map_vec = unsafe {
            core::ptr::copy_nonoverlapping(
                memory_map.as_ptr(),
                MAP_BUF.as_mut_ptr(),
                memory_map.len(),
            );
            Vec::from_raw_parts(MAP_BUF.as_mut_ptr(), memory_map.len(), MAX_MAP_ENTRIES)
        };
        let mut allocator = Self {
            memory_map: map_vec,
            bitmap: Vec::new(),
            total_frames: 0,
            free_frames: 0,
            managed_start: u64::MAX,
            managed_end: 0,
            alloc_hint: 0,
        };

        allocator.initialize();
        allocator
    }

    fn initialize(&mut self) {
        // Find managed memory range (conventional memory only)
        for region in &self.memory_map {
            if region.region_type == MemoryType::Conventional
                || region.region_type == MemoryType::PersistentMemory
            {
                self.managed_start = self.managed_start.min(region.base_addr);
                self.managed_end = self.managed_end.max(region.base_addr + region.length);
            }
        }

        // Align to frame boundaries
        self.managed_start = (self.managed_start + FRAME_SIZE - 1) & !(FRAME_SIZE - 1);
        self.managed_end &= !(FRAME_SIZE - 1);

        if self.managed_start >= self.managed_end {
            panic!("No usable memory found");
        }

        self.total_frames = ((self.managed_end - self.managed_start) / FRAME_SIZE) as usize;
        let bitmap_size = (self.total_frames + 63) / 64;
        assert!(
            bitmap_size <= MAX_BITMAP_WORDS,
            "frame bitmap needs {} words for {} RAM, static buffer holds {}",
            bitmap_size,
            self.managed_end - self.managed_start,
            MAX_BITMAP_WORDS
        );
        // Static backing: no heap allocation (heap does not exist yet).
        self.bitmap = unsafe {
            Vec::from_raw_parts(BITMAP_BUF.as_mut_ptr(), bitmap_size, MAX_BITMAP_WORDS)
        };
        self.free_frames = 0;

        // Mark all frames as used initially
        for i in 0..bitmap_size {
            self.bitmap[i] = u64::MAX;
        }

        // Mark available frames as free (index loop: avoids cloning the
        // map, which would allocate before the heap exists).
        for i in 0..self.memory_map.len() {
            let region = self.memory_map[i];
            if region.region_type == MemoryType::Conventional
                || region.region_type == MemoryType::PersistentMemory
            {
                let start_frame = self.addr_to_frame(region.base_addr);
                let end_frame = self.addr_to_frame(region.base_addr + region.length);
                for frame in start_frame..end_frame {
                    self.set_frame_free(frame);
                }
            }
        }

        // Sync the counter with the bitmap before reserving: the bitmap
        // starts as all-free (MAX) with free_frames == 0, and
        // reserve_kernel_memory() decrements the counter per frame.
        self.free_frames = self.count_free_frames();

        // Reserve frames for kernel, page tables, etc.
        self.reserve_kernel_memory();

        self.free_frames = self.count_free_frames();
        TOTAL_MEMORY.store(self.total_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        FREE_MEMORY.store(self.free_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        USED_MEMORY.store((self.total_frames - self.free_frames) as u64 * FRAME_SIZE, Ordering::SeqCst);

        info!("Frame allocator: {} total frames, {} free ({} MB)",
              self.total_frames, self.free_frames,
              self.free_frames * 4 / 1024);
    }

    fn addr_to_frame(&self, addr: u64) -> usize {
        if addr < self.managed_start {
            return 0;
        }
        ((addr - self.managed_start) / FRAME_SIZE) as usize
    }

    fn frame_to_addr(&self, frame: usize) -> u64 {
        self.managed_start + (frame as u64 * FRAME_SIZE)
    }

    fn set_frame_used(&mut self, frame: usize) {
        if frame >= self.total_frames {
            return;
        }
        let idx = frame / 64;
        let bit = frame % 64;
        if self.bitmap[idx] & (1 << bit) != 0 {
            self.bitmap[idx] &= !(1 << bit);
            self.free_frames -= 1;
        }
    }

    fn set_frame_free(&mut self, frame: usize) {
        if frame >= self.total_frames {
            return;
        }
        let idx = frame / 64;
        let bit = frame % 64;
        if self.bitmap[idx] & (1 << bit) == 0 {
            self.bitmap[idx] |= 1 << bit;
            self.free_frames += 1;
        }
    }

    fn is_frame_free(&self, frame: usize) -> bool {
        if frame >= self.total_frames {
            return false;
        }
        let idx = frame / 64;
        let bit = frame % 64;
        self.bitmap[idx] & (1 << bit) != 0
    }

    fn count_free_frames(&self) -> usize {
        self.bitmap.iter().map(|w| w.count_ones() as usize).sum()
    }

    fn reserve_kernel_memory(&mut self) {
        // Reserve first 1MB (BIOS, VGA, etc.)
        for frame in 0..(0x100000 / FRAME_SIZE) as usize {
            self.set_frame_used(frame);
        }

        // Reserve kernel image area (will be mapped by bootloader)
        // This is typically around 1MB-2MB

        // Reserve page tables area (if known)
    }

    /// Allocate contiguous frames
    pub fn allocate_frames(&mut self, count: usize) -> Option<FrameRange> {
        if count == 0 {
            return None;
        }

        if count == 1 {
            return self.allocate_frame().map(|f| FrameRange::new(f.start_address().as_u64(), 1));
        }

        // Search for contiguous free frames
        let mut start = self.alloc_hint;
        let mut consecutive = 0;

        for i in start..self.total_frames {
            if self.is_frame_free(i) {
                if consecutive == 0 {
                    start = i;
                }
                consecutive += 1;
                if consecutive >= count {
                    // Found!
                    for j in 0..count {
                        self.set_frame_used(start + j);
                    }
                    self.alloc_hint = start + count;
                    self.update_stats();
                    return Some(FrameRange::new(self.frame_to_addr(start), count));
                }
            } else {
                consecutive = 0;
            }
        }

        // Wrap around and search from beginning
        for i in 0..start {
            if self.is_frame_free(i) {
                if consecutive == 0 {
                    start = i;
                }
                consecutive += 1;
                if consecutive >= count {
                    for j in 0..count {
                        self.set_frame_used(start + j);
                    }
                    self.alloc_hint = start + count;
                    self.update_stats();
                    return Some(FrameRange::new(self.frame_to_addr(start), count));
                }
            } else {
                consecutive = 0;
            }
        }

        None
    }

    /// Allocate a single frame
    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        // Fast path: check hint
        if self.alloc_hint < self.total_frames && self.is_frame_free(self.alloc_hint) {
            let frame = self.alloc_hint;
            self.set_frame_used(frame);
            self.alloc_hint += 1;
            self.update_stats();
            return Some(PhysFrame::containing_address(PhysAddr::new(self.frame_to_addr(frame))));
        }

        // Search from hint
        for i in self.alloc_hint..self.total_frames {
            if self.is_frame_free(i) {
                self.set_frame_used(i);
                self.alloc_hint = i + 1;
                self.update_stats();
                return Some(PhysFrame::containing_address(PhysAddr::new(self.frame_to_addr(i))));
            }
        }

        // Search from beginning
        for i in 0..self.alloc_hint {
            if self.is_frame_free(i) {
                self.set_frame_used(i);
                self.alloc_hint = i + 1;
                self.update_stats();
                return Some(PhysFrame::containing_address(PhysAddr::new(self.frame_to_addr(i))));
            }
        }

        None
    }

    /// Deallocate frames
    pub fn deallocate_frames(&mut self, range: FrameRange) {
        let start_frame = self.addr_to_frame(range.start_addr);
        for i in 0..range.count {
            self.set_frame_free(start_frame + i);
        }
        if start_frame < self.alloc_hint {
            self.alloc_hint = start_frame;
        }
        self.update_stats();
    }

    /// Deallocate a single frame
    pub fn deallocate_frame(&mut self, frame: PhysFrame) {
        let frame_idx = self.addr_to_frame(frame.start_address().as_u64());
        self.set_frame_free(frame_idx);
        if frame_idx < self.alloc_hint {
            self.alloc_hint = frame_idx;
        }
        self.update_stats();
    }

    fn update_stats(&self) {
        FREE_MEMORY.store(self.free_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        USED_MEMORY.store((self.total_frames - self.free_frames) as u64 * FRAME_SIZE, Ordering::SeqCst);
    }

    /// Get total memory
    pub fn total_memory(&self) -> u64 {
        self.total_frames as u64 * FRAME_SIZE
    }

    /// Get free memory
    pub fn free_memory(&self) -> u64 {
        self.free_frames as u64 * FRAME_SIZE
    }

    /// Get used memory
    pub fn used_memory(&self) -> u64 {
        (self.total_frames - self.free_frames) as u64 * FRAME_SIZE
    }

    /// Get memory map
    pub fn memory_map(&self) -> &[MemoryRegion] {
        &self.memory_map
    }
}

/// Range of contiguous frames
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRange {
    pub start_addr: u64,
    pub count: usize,
}

impl FrameRange {
    pub fn new(start_addr: u64, count: usize) -> Self {
        Self { start_addr, count }
    }

    pub fn start_frame(&self) -> PhysFrame {
        PhysFrame::containing_address(PhysAddr::new(self.start_addr))
    }

    pub fn end_addr(&self) -> u64 {
        self.start_addr + self.count as u64 * FRAME_SIZE
    }

    pub fn size(&self) -> u64 {
        self.count as u64 * FRAME_SIZE
    }

    pub fn frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        (0..self.count).map(move |i| {
            PhysFrame::containing_address(PhysAddr::new(self.start_addr + i as u64 * FRAME_SIZE))
        })
    }
}

/// Initialize physical memory manager
pub fn init(memory_map: &[MemoryRegion]) {
    let allocator = FrameAllocator::new(memory_map);
    *FRAME_ALLOCATOR.lock() = Some(allocator);
    info!("Physical memory manager initialized");
}

/// Allocate frames
pub fn allocate_frames(count: usize) -> Option<FrameRange> {
    FRAME_ALLOCATOR.lock().as_mut()?.allocate_frames(count)
}

/// Allocate a single frame
pub fn allocate_frame() -> Option<PhysFrame> {
    FRAME_ALLOCATOR.lock().as_mut()?.allocate_frame()
}

/// Deallocate frames
pub fn deallocate_frames(range: FrameRange) {
    if let Some(allocator) = FRAME_ALLOCATOR.lock().as_mut() {
        allocator.deallocate_frames(range);
    }
}

/// Deallocate a single frame
pub fn deallocate_frame(frame: PhysFrame) {
    if let Some(allocator) = FRAME_ALLOCATOR.lock().as_mut() {
        allocator.deallocate_frame(frame);
    }
}

/// Get total memory
pub fn total_memory() -> u64 {
    TOTAL_MEMORY.load(Ordering::SeqCst)
}

/// Get free memory
pub fn free_memory() -> u64 {
    FREE_MEMORY.load(Ordering::SeqCst)
}

/// Get used memory
pub fn used_memory() -> u64 {
    USED_MEMORY.load(Ordering::SeqCst)
}

/// Get memory map
pub fn memory_map() -> Vec<MemoryRegion> {
    FRAME_ALLOCATOR.lock().as_ref().map(|a| a.memory_map().to_vec()).unwrap_or_default()
}

/// Convert physical address to virtual (direct map)
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst) + phys.as_u64())
}

/// Convert virtual address to physical (direct map)
pub fn virt_to_phys(virt: VirtAddr) -> Option<PhysAddr> {
    let offset = PHYS_MEM_OFFSET.load(Ordering::SeqCst);
    if virt.as_u64() >= offset {
        Some(PhysAddr::new(virt.as_u64() - offset))
    } else {
        None
    }
}