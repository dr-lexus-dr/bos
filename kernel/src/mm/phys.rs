//! Physical Memory Manager

#[path = "frame_bitmap.rs"]
mod frame_bitmap;

use crate::arch::x86_64::paging::PHYS_MEM_OFFSET;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use bloader_shared::{MemoryFlags, MemoryRegion, MemoryType};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use log::{info, trace, warn};
use x86_64::{structures::paging::PhysFrame, PhysAddr, VirtAddr};

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
#[cfg(feature = "live")]
static PEAK_USED: AtomicU64 = AtomicU64::new(0);
/// Free physical memory
static FREE_MEMORY: AtomicU64 = AtomicU64::new(0);

/// Static backing for the memory-map slice.
///
/// Initialization precedes the heap. The singleton borrows static storage;
/// it must not be constructed a second time while the first instance is live.
const MAX_MAP_ENTRIES: usize = if cfg!(feature = "live") { 1 } else { 256 };
static mut MAP_BUF: [MemoryRegion; MAX_MAP_ENTRIES] = [MemoryRegion {
    base_addr: 0,
    length: 0,
    region_type: MemoryType::Reserved,
    flags: MemoryFlags::empty(),
}; MAX_MAP_ENTRIES];

/// Static backing for the frame bitmap (same no-heap constraint as above).
/// 262144 words = 2 MiB .bss, covers 64 GiB of RAM.  The previous 16 GiB cap
/// made a 32 GiB QEMU guest panic during boot with "frame bitmap needs ...".
const MAX_BITMAP_WORDS: usize = if cfg!(feature = "live") { 4096 } else { 262_144 };
static mut BITMAP_BUF: [u64; MAX_BITMAP_WORDS] = [0; MAX_BITMAP_WORDS];
static mut ALLOCATED_BUF: [u64; MAX_BITMAP_WORDS] = [0; MAX_BITMAP_WORDS];

/// Bitmap allocator for frames
pub struct FrameAllocator {
    /// Memory map from bootloader
    memory_map: &'static [MemoryRegion],
    /// Bitmap of free frames (1 = available, 0 = unavailable)
    bitmap: &'static mut [u64],
    allocated: &'static mut [u64],
    usable_frames: usize,
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
    fn new(memory_map: &[MemoryRegion]) -> Self {
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
            core::slice::from_raw_parts(MAP_BUF.as_ptr(), memory_map.len())
        };
        let mut allocator = Self {
            memory_map: map_vec,
            bitmap: &mut [],
            allocated: &mut [],
            usable_frames: 0,
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
        for region in self.memory_map {
            if region.region_type == MemoryType::Conventional
                || region.region_type == MemoryType::PersistentMemory
            {
                self.managed_start = self.managed_start.min(region.base_addr);
                self.managed_end = self.managed_end.max(
                    region
                        .base_addr
                        .checked_add(region.length)
                        .expect("memory region overflow"),
                );
            }
        }

        // Align to frame boundaries
        self.managed_start = self
            .managed_start
            .checked_add(FRAME_SIZE - 1)
            .expect("No usable memory found")
            & !(FRAME_SIZE - 1);
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
        self.bitmap =
            unsafe { core::slice::from_raw_parts_mut(BITMAP_BUF.as_mut_ptr(), bitmap_size) };
        self.allocated =
            unsafe { core::slice::from_raw_parts_mut(ALLOCATED_BUF.as_mut_ptr(), bitmap_size) };
        self.allocated.fill(0);
        self.free_frames = 0;

        // Mark all frames as used initially
        for i in 0..bitmap_size {
            self.bitmap[i] = 0;
        }

        // Mark available frames as free (index loop: avoids cloning the
        // map, which would allocate before the heap exists).
        for i in 0..self.memory_map.len() {
            let region = self.memory_map[i];
            if region.region_type == MemoryType::Conventional
                || region.region_type == MemoryType::PersistentMemory
            {
                let aligned = region
                    .base_addr
                    .checked_add(FRAME_SIZE - 1)
                    .expect("memory region overflow")
                    & !(FRAME_SIZE - 1);
                let start_frame = self.addr_to_frame(aligned);
                let end_frame = self.addr_to_frame(
                    region
                        .base_addr
                        .checked_add(region.length)
                        .expect("memory region overflow"),
                );
                for frame in start_frame..end_frame {
                    self.set_frame_free(frame);
                }
            }
        }

        self.usable_frames = self.free_frames;

        // Only fully usable frames were inserted; firmware holes remain unavailable.
        self.free_frames = self.count_free_frames();

        // Reserve frames for kernel, page tables, etc.
        self.reserve_kernel_memory();

        self.free_frames = self.count_free_frames();
        TOTAL_MEMORY.store(self.usable_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        FREE_MEMORY.store(self.free_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        USED_MEMORY.store(
            (self.usable_frames - self.free_frames) as u64 * FRAME_SIZE,
            Ordering::SeqCst,
        );

        info!(
            "Frame allocator: {} total frames, {} free ({} MB)",
            self.total_frames,
            self.free_frames,
            self.free_frames * 4 / 1024
        );
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
        for frame in 0..self.addr_to_frame(0x100000).min(self.total_frames) {
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
            return self
                .allocate_frame()
                .map(|f| FrameRange::new(f.start_address().as_u64(), 1));
        }

        let start =
            frame_bitmap::find_run(&self.bitmap, self.total_frames, self.alloc_hint, count)?;
        for frame in start..start + count {
            self.set_frame_used(frame);
            self.allocated[frame / 64] |= 1u64 << (frame % 64);
        }
        self.alloc_hint = start + count;
        self.update_stats();
        Some(FrameRange::new(self.frame_to_addr(start), count))
    }

    /// Allocate a single frame
    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = frame_bitmap::find_run(&self.bitmap, self.total_frames, self.alloc_hint, 1)?;
        self.set_frame_used(frame);
        self.allocated[frame / 64] |= 1u64 << (frame % 64);
        self.alloc_hint = frame + 1;
        self.update_stats();
        Some(PhysFrame::containing_address(PhysAddr::new(
            self.frame_to_addr(frame),
        )))
    }

    /// Validate the entire range before changing any bit. Firmware reservations,
    /// out-of-range addresses and repeated frees cannot enter the free pool.
    pub fn deallocate_frames(&mut self, range: FrameRange) {
        if range.count == 0
            || range.start_addr % FRAME_SIZE != 0
            || range.start_addr < self.managed_start
        {
            return;
        }
        let start = self.addr_to_frame(range.start_addr);
        let Some(end) = start
            .checked_add(range.count)
            .filter(|e| *e <= self.total_frames)
        else {
            return;
        };
        if !(start..end).all(|frame| self.allocated[frame / 64] & (1u64 << (frame % 64)) != 0) {
            return;
        }
        for frame in start..end {
            self.allocated[frame / 64] &= !(1u64 << (frame % 64));
            self.set_frame_free(frame);
        }
        self.alloc_hint = self.alloc_hint.min(start);
        self.update_stats();
    }

    pub fn deallocate_frame(&mut self, frame: PhysFrame) {
        self.deallocate_frames(FrameRange::new(frame.start_address().as_u64(), 1));
    }

    fn update_stats(&self) {
        FREE_MEMORY.store(self.free_frames as u64 * FRAME_SIZE, Ordering::SeqCst);
        let used = (self.usable_frames - self.free_frames) as u64 * FRAME_SIZE;
        USED_MEMORY.store(used, Ordering::SeqCst);
        #[cfg(feature = "live")]
        PEAK_USED.fetch_max(used, Ordering::Relaxed);
    }

    /// Get total memory
    pub fn total_memory(&self) -> u64 {
        self.usable_frames as u64 * FRAME_SIZE
    }

    /// Get free memory
    pub fn free_memory(&self) -> u64 {
        self.free_frames as u64 * FRAME_SIZE
    }

    /// Get used memory
    pub fn used_memory(&self) -> u64 {
        (self.usable_frames - self.free_frames) as u64 * FRAME_SIZE
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
    let mut slot = FRAME_ALLOCATOR.lock();
    assert!(slot.is_none(), "physical allocator initialized twice");
    *slot = Some(FrameAllocator::new(memory_map));
    drop(slot);
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

/// Keep 256 frames (1 MiB) available to kernel teardown and diagnostics.
/// Reservation and allocation share one lock, so concurrent callers cannot
/// independently consume the same reserve.
pub fn allocate_user_frame() -> Option<PhysFrame> {
    let mut guard = FRAME_ALLOCATOR.lock();
    let allocator = guard.as_mut()?;
    if allocator.free_frames <= 256 {
        return None;
    }
    allocator.allocate_frame()
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

#[cfg(feature = "live")]
pub fn peak_used() -> u64 { PEAK_USED.load(Ordering::Relaxed) }

/// Get memory map
pub fn memory_map() -> Vec<MemoryRegion> {
    FRAME_ALLOCATOR
        .lock()
        .as_ref()
        .map(|a| a.memory_map().to_vec())
        .unwrap_or_default()
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firmware_holes_padding_and_invalid_frees() {
        let regions = [
            MemoryRegion {
                base_addr: 0x100000,
                length: 5 * FRAME_SIZE,
                region_type: MemoryType::Conventional,
                flags: MemoryFlags::empty(),
            },
            MemoryRegion {
                base_addr: 0x105000,
                length: 3 * FRAME_SIZE,
                region_type: MemoryType::Reserved,
                flags: MemoryFlags::empty(),
            },
            MemoryRegion {
                base_addr: 0x108000,
                length: 3 * FRAME_SIZE,
                region_type: MemoryType::Conventional,
                flags: MemoryFlags::empty(),
            },
        ];
        let mut allocator = FrameAllocator::new(&regions);
        assert_eq!(allocator.total_memory(), 8 * FRAME_SIZE);
        assert_eq!(allocator.free_memory(), 8 * FRAME_SIZE);
        let frames: Vec<_> = (0..8)
            .map(|_| allocator.allocate_frame().unwrap())
            .collect();
        assert!(allocator.allocate_frame().is_none());
        assert_eq!(allocator.free_memory(), 0);
        for frame in &frames {
            let address = frame.start_address().as_u64();
            assert!(!(0x105000..0x108000).contains(&address));
            assert!((0x100000..0x10b000).contains(&address));
        }
        for range in [
            FrameRange::new(0, 1),
            FrameRange::new(0x105000, 1),
            FrameRange::new(0x100001, 1),
            FrameRange::new(0x100000, usize::MAX),
        ] {
            allocator.deallocate_frames(range);
            assert_eq!(allocator.free_memory(), 0);
        }
        for frame in frames {
            allocator.deallocate_frame(frame);
        }
        assert_eq!(allocator.free_memory(), 8 * FRAME_SIZE);
        allocator.deallocate_frames(FrameRange::new(0x100000, 1));
        assert_eq!(allocator.free_memory(), 8 * FRAME_SIZE);
        assert!(allocator.allocate_frames(6).is_none());
        assert_eq!(allocator.allocate_frames(5).unwrap().start_addr, 0x100000);
        assert_eq!(allocator.allocate_frames(3).unwrap().start_addr, 0x108000);
        assert!(allocator.allocate_frame().is_none());
    }
}
