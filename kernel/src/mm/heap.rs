//! Kernel Heap Allocator

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::ptr::{self, NonNull};
use alloc::alloc::{alloc, dealloc, handle_alloc_error};
use spin::Mutex;
use lazy_static::lazy_static;
use log::{info, trace, warn};
use crate::mm::phys::{allocate_frames, deallocate_frames, FrameRange, FRAME_SIZE};
use crate::arch::x86_64::paging::{map_to, unmap, KERNEL_HEAP_BASE, KERNEL_HEAP_SIZE, PageTableFlags};

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
        Self { inner: Mutex::new(Heap::new()) }
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
        info!("Heap initialized: 0x{:x} - 0x{:x} ({} MB)",
              heap_start, self.end, heap_size / 1024 / 1024);

        HEAP_START.store(heap_start, Ordering::SeqCst);
        HEAP_END.store(self.end, Ordering::SeqCst);
    }

    /// Allocate memory
    pub fn allocate(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(MIN_ALLOC_SIZE);
        let align = layout.align().max(ALLOC_ALIGNMENT);

        // Try free list first
        if let Some(ptr) = self.free_list.allocate(size, align) {
            self.allocated_bytes += size;
            self.allocation_count += 1;
            return ptr;
        }

        // Bump allocation
        let aligned_next = align_up(self.next, align);
        let end_addr = aligned_next + size;

        if end_addr > self.end {
            // Try to expand heap
            if let Err(_) = self.expand_heap(size) {
                handle_alloc_error(layout);
            }
            return self.allocate(layout); // Retry after expansion
        }

        self.next = end_addr;
        self.allocated_bytes += size;
        self.allocation_count += 1;

        aligned_next as *mut u8
    }

    /// Deallocate memory
    pub fn deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(MIN_ALLOC_SIZE);
        self.free_list.deallocate(ptr as usize, size);
        self.freed_bytes += size;
    }

    /// Expand heap by mapping more pages
    fn expand_heap(&mut self, needed: usize) -> Result<(), &'static str> {
        let current_end = self.end;
        let new_end = align_up(current_end + needed, FRAME_SIZE as usize);
        let additional_frames = (new_end - current_end) / FRAME_SIZE as usize;

        for i in 0..additional_frames {
            let frame_addr = current_end + i * FRAME_SIZE as usize;
            let virt_addr = frame_addr as u64;
            let frame = allocate_frames(1).ok_or("Out of physical memory")?;

            map_to(
                x86_64::VirtAddr::new(virt_addr),
                frame.start_frame().start_address(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
            ).map_err(|_| "Failed to map heap page")?;
        }

        self.end = new_end;
        HEAP_END.store(new_end, Ordering::SeqCst);
        trace!("Heap expanded to 0x{:x}", new_end);

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

/// Free list for managing deallocated blocks
struct FreeList {
    head: Option<NonNull<FreeBlock>>,
    block_count: usize,
}

struct FreeBlock {
    size: usize,
    next: Option<NonNull<FreeBlock>>,
}

impl FreeList {
    const fn new() -> Self {
        Self { head: None, block_count: 0 }
    }

    fn allocate(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let mut current = self.head;
        let mut prev: Option<NonNull<FreeBlock>> = None;

        while let Some(block_ptr) = current {
            let block = unsafe { block_ptr.as_ref() };
            let block_addr = block_ptr.as_ptr() as usize;
            let aligned_addr = align_up(block_addr, align);

            if aligned_addr + size <= block_addr + block.size {
                // Found suitable block
                let remaining = block_addr + block.size - (aligned_addr + size);

                // Remove block from list
                if let Some(mut prev_ptr) = prev {
                    unsafe { prev_ptr.as_mut().next = block.next; }
                } else {
                    self.head = block.next;
                }

                // If there's remaining space, add it back to free list
                if remaining >= MIN_ALLOC_SIZE {
                    let new_block_addr = aligned_addr + size;
                    let mut new_block = NonNull::new(new_block_addr as *mut FreeBlock).unwrap();
                    unsafe {
                        new_block.as_mut().size = remaining;
                        new_block.as_mut().next = self.head;
                    }
                    self.head = Some(new_block);
                    self.block_count += 1;
                } else if remaining > 0 {
                    // Too small, just waste it (could coalesce)
                }

                self.block_count -= 1;
                return Some(aligned_addr as *mut u8);
            }

            prev = current;
            current = block.next;
        }

        None
    }

    fn deallocate(&mut self, addr: usize, size: usize) {
        // Simple: add to front of free list
        let mut block = NonNull::new(addr as *mut FreeBlock).unwrap();
        unsafe {
            block.as_mut().size = size;
            block.as_mut().next = self.head;
        }
        self.head = Some(block);
        self.block_count += 1;

        // TODO: Coalesce adjacent blocks
    }

    fn block_count(&self) -> usize {
        self.block_count
    }
}

/// Align address up to alignment
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
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
    // Start with 16 MB heap
    const INITIAL_HEAP_SIZE: usize = 16 * 1024 * 1024;

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
        ).expect("Failed to map heap page");
    }

    HEAP_ALLOCATOR.lock().init(heap_start, INITIAL_HEAP_SIZE);
    info!("Kernel heap initialized: {} MB at 0x{:x}", INITIAL_HEAP_SIZE / 1024 / 1024, heap_start);
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
        unsafe { ptr.write_bytes(0, layout.size()); }
    }
    ptr
}

/// Reallocate memory
pub fn reallocate(ptr: *mut u8, old_layout: Layout, new_layout: Layout) -> *mut u8 {
    let new_ptr = allocate(new_layout);
    if !new_ptr.is_null() && !ptr.is_null() {
        let copy_size = old_layout.size().min(new_layout.size());
        unsafe { core::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size); }
        deallocate(ptr, old_layout);
    }
    new_ptr
}