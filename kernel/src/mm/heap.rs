//! Kernel Heap Allocator

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
#[path = "free_list.rs"]
mod free_list;
use crate::arch::x86_64::paging::{
    map_to, unmap, PageTableFlags, KERNEL_HEAP_BASE, KERNEL_HEAP_SIZE,
};
use crate::mm::phys::{allocate_frames, deallocate_frames, FrameRange, FRAME_SIZE};
use alloc::alloc::{alloc, dealloc, handle_alloc_error};
use free_list::{align_up, FreeList};
use lazy_static::lazy_static;
use log::{info, trace, warn};
use spin::Mutex;

/// Heap allocator state
static HEAP_START: AtomicUsize = AtomicUsize::new(0);
static HEAP_END: AtomicUsize = AtomicUsize::new(0);
static HEAP_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

/// Minimum allocation size (16 bytes)
const MIN_ALLOC_SIZE: usize = 16;
/// Alignment for allocations
const ALLOC_ALIGNMENT: usize = 16;

/// Global heap allocator (also the `#[global_allocator]`)
#[global_allocator]
pub static HEAP_ALLOCATOR: LockedHeap = LockedHeap::new();

/// Locked heap wrapper
pub struct LockedHeap {
    inner: Mutex<Heap>,
}

impl LockedHeap {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(Heap::new()),
        }
    }

    pub fn lock(&self) -> spin::MutexGuard<'_, Heap> {
        self.inner.lock()
    }
}

unsafe impl Send for LockedHeap {}
unsafe impl Sync for LockedHeap {}
unsafe impl Send for Heap {}
unsafe impl Sync for Heap {}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner.lock().allocate(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.inner.lock().deallocate(ptr, layout)
    }
}

/// Heap implementation using a simple bump allocator with free list
pub struct Heap {
    /// Start of heap
    start: usize,
    /// End of heap
    end: usize,
    /// Current bump pointer
    next: usize,
    /// Free list for deallocated blocks
    free_list: FreeList,
    /// Statistics
    allocated_bytes: usize,
    freed_bytes: usize,
    allocation_count: usize,
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            next: 0,
            free_list: FreeList::new(),
            allocated_bytes: 0,
            freed_bytes: 0,
            allocation_count: 0,
        }
    }

    /// Initialize heap
    pub fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.start = heap_start;
        self.end = heap_start + heap_size;
        self.next = heap_start;
        info!(
            "Heap initialized: 0x{:x} - 0x{:x} ({} MB)",
            heap_start,
            self.end,
            heap_size / 1024 / 1024
        );

        HEAP_START.store(heap_start, Ordering::SeqCst);
        HEAP_END.store(self.end, Ordering::SeqCst);
    }

    /// Allocate memory
    pub fn allocate(&mut self, layout: Layout) -> *mut u8 {
        let Some(size) = align_up(layout.size().max(MIN_ALLOC_SIZE), ALLOC_ALIGNMENT) else {
            return ptr::null_mut();
        };
        let align = layout.align().max(ALLOC_ALIGNMENT);

        // Try free list first
        if let Some(ptr) = self.free_list.allocate(size, align) {
            self.allocated_bytes += size;
            self.allocation_count += 1;
            return ptr;
        }

        let Some(aligned_next) = align_up(self.next, align) else {
            return ptr::null_mut();
        };
        let Some(end_addr) = aligned_next.checked_add(size) else {
            return ptr::null_mut();
        };
        if end_addr > self.end && self.expand_heap(end_addr - self.end).is_err() {
            return ptr::null_mut();
        }
        // Return alignment padding to the allocator instead of losing it.
        if aligned_next > self.next {
            unsafe {
                assert!(self
                    .free_list
                    .deallocate(self.next, aligned_next - self.next));
            }
        }
        self.next = end_addr;
        self.allocated_bytes += size;
        self.allocation_count += 1;
        aligned_next as *mut u8
    }

    pub fn deallocate(&mut self, pointer: *mut u8, layout: Layout) {
        let Some(size) = align_up(layout.size().max(MIN_ALLOC_SIZE), ALLOC_ALIGNMENT) else {
            return;
        };
        let address = pointer as usize;
        if address < self.start || address.checked_add(size).is_none_or(|end| end > self.next) {
            return;
        }
        if unsafe { self.free_list.deallocate(address, size) } {
            self.freed_bytes += size;
        }
    }

    /// Map incrementally. On failure retain successfully mapped capacity so a retry
    /// never remaps or leaks those frames; free the frame for the failed mapping.
    fn expand_heap(&mut self, needed: usize) -> Result<(), &'static str> {
        let new_end = self
            .end
            .checked_add(needed)
            .and_then(|end| align_up(end, FRAME_SIZE as usize))
            .filter(|end| *end as u64 <= KERNEL_HEAP_BASE + KERNEL_HEAP_SIZE)
            .ok_or("Heap limit reached")?;
        while self.end < new_end {
            let frame = allocate_frames(1).ok_or("Out of physical memory")?;
            if map_to(
                x86_64::VirtAddr::new(self.end as u64),
                frame.start_frame().start_address(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
            )
            .is_err()
            {
                deallocate_frames(frame);
                return Err("Failed to map heap page");
            }
            self.end += FRAME_SIZE as usize;
            HEAP_END.store(self.end, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Get heap statistics
    pub fn stats(&self) -> HeapStats {
        HeapStats {
            total_size: self.end - self.start,
            used_size: self.allocated_bytes - self.freed_bytes,
            allocated_bytes: self.allocated_bytes,
            freed_bytes: self.freed_bytes,
            allocation_count: self.allocation_count,
            free_list_blocks: self.free_list.block_count(),
        }
    }
}

/// Heap statistics
#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    pub total_size: usize,
    pub used_size: usize,
    pub allocated_bytes: usize,
    pub freed_bytes: usize,
    pub allocation_count: usize,
    pub free_list_blocks: usize,
}

/// Initialize kernel heap
pub fn init() {
    // The live profile grows on demand from a few pages.
    const INITIAL_HEAP_SIZE: usize = if cfg!(feature = "live") { 64 * 1024 } else { 16 * 1024 * 1024 };

    // Reserve virtual address space for heap
    let heap_start = KERNEL_HEAP_BASE as usize;

    // Allocate initial frames and map them
    let initial_frames = INITIAL_HEAP_SIZE / FRAME_SIZE as usize;
    for i in 0..initial_frames {
        let virt_addr = heap_start + i * FRAME_SIZE as usize;
        let frame = allocate_frames(1).expect("Failed to allocate heap frame");

        map_to(
            x86_64::VirtAddr::new(virt_addr as u64),
            frame.start_frame().start_address(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        )
        .expect("Failed to map heap page");
    }

    HEAP_ALLOCATOR.lock().init(heap_start, INITIAL_HEAP_SIZE);
    info!(
        "Kernel heap initialized: {} KiB at 0x{:x}",
        INITIAL_HEAP_SIZE / 1024,
        heap_start
    );
}

/// Get heap statistics
pub fn stats() -> HeapStats {
    HEAP_ALLOCATOR.lock().stats()
}

/// Allocate memory (convenience function)
pub fn allocate(layout: Layout) -> *mut u8 {
    unsafe { HEAP_ALLOCATOR.alloc(layout) }
}

/// Deallocate memory (convenience function)
pub fn deallocate(ptr: *mut u8, layout: Layout) {
    unsafe { HEAP_ALLOCATOR.dealloc(ptr, layout) }
}

/// Allocate zeroed memory
pub fn allocate_zeroed(layout: Layout) -> *mut u8 {
    let ptr = allocate(layout);
    if !ptr.is_null() {
        unsafe {
            ptr.write_bytes(0, layout.size());
        }
    }
    ptr
}

/// Reallocate memory
pub fn reallocate(ptr: *mut u8, old_layout: Layout, new_layout: Layout) -> *mut u8 {
    let new_ptr = allocate(new_layout);
    if !new_ptr.is_null() && !ptr.is_null() {
        let copy_size = old_layout.size().min(new_layout.size());
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
        }
        deallocate(ptr, old_layout);
    }
    new_ptr
}
