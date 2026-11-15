//! Intrusive address-ordered free list. All extents are multiples of 16 bytes.
//! The caller owns the arena and serializes access; no allocation is performed here.
use core::ptr::NonNull;

pub const GRANULE: usize = 16;

pub fn align_up(value: usize, alignment: usize) -> Option<usize> {
    if !alignment.is_power_of_two() {
        return None;
    }
    value
        .checked_add(alignment - 1)
        .map(|n| n & !(alignment - 1))
}

struct Block {
    size: usize,
    next: Option<NonNull<Block>>,
}

pub struct FreeList {
    head: Option<NonNull<Block>>,
    count: usize,
}

impl FreeList {
    pub const fn new() -> Self {
        Self {
            head: None,
            count: 0,
        }
    }
    pub fn block_count(&self) -> usize {
        self.count
    }

    /// Caller must provide a writable, exclusively owned arena extent.
    /// Overlap with an existing free extent is rejected before writing metadata.
    pub unsafe fn deallocate(&mut self, address: usize, size: usize) -> bool {
        if address == 0 || address % GRANULE != 0 || size < GRANULE || size % GRANULE != 0 {
            return false;
        }
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        let mut previous: Option<NonNull<Block>> = None;
        let mut next = self.head;
        while let Some(node) = next {
            if node.as_ptr() as usize >= address {
                break;
            }
            previous = next;
            next = node.as_ref().next;
        }
        if let Some(node) = previous {
            if (node.as_ptr() as usize)
                .checked_add(node.as_ref().size)
                .is_none_or(|e| e > address)
            {
                return false;
            }
        }
        if next.is_some_and(|node| (node.as_ptr() as usize) < end) {
            return false;
        }
        let mut inserted = NonNull::new_unchecked(address as *mut Block);
        inserted.as_ptr().write(Block { size, next });
        if let Some(mut node) = previous {
            node.as_mut().next = Some(inserted);
        } else {
            self.head = Some(inserted);
        }
        self.count += 1;
        if let Some(node) = next {
            if end == node.as_ptr() as usize {
                inserted.as_mut().size += node.as_ref().size;
                inserted.as_mut().next = node.as_ref().next;
                self.count -= 1;
            }
        }
        if let Some(mut node) = previous {
            if node.as_ptr() as usize + node.as_ref().size == address {
                node.as_mut().size += inserted.as_ref().size;
                node.as_mut().next = inserted.as_ref().next;
                self.count -= 1;
            }
        }
        true
    }

    pub fn allocate(&mut self, size: usize, alignment: usize) -> Option<*mut u8> {
        if size < GRANULE || size % GRANULE != 0 || alignment < GRANULE {
            return None;
        }
        let mut previous: Option<NonNull<Block>> = None;
        let mut current = self.head;
        while let Some(node) = current {
            // Copy metadata before modifying the list; never keep a reference into a reused block.
            let (block_size, next) = unsafe { (node.as_ref().size, node.as_ref().next) };
            let start = node.as_ptr() as usize;
            let aligned = align_up(start, alignment)?;
            let end = start.checked_add(block_size)?;
            if let Some(allocated_end) = aligned.checked_add(size).filter(|e| *e <= end) {
                unsafe {
                    if let Some(mut p) = previous {
                        p.as_mut().next = next;
                    } else {
                        self.head = next;
                    }
                    self.count -= 1;
                    if aligned > start {
                        assert!(self.deallocate(start, aligned - start));
                    }
                    if allocated_end < end {
                        assert!(self.deallocate(allocated_end, end - allocated_end));
                    }
                }
                return Some(aligned as *mut u8);
            }
            previous = current;
            current = next;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(align(4096))]
    struct Arena([u8; 16384]);

    #[test]
    fn alignment_prefix_and_coalescing() {
        let mut arena = Arena([0; 16384]);
        let base = arena.0.as_mut_ptr() as usize;
        let mut list = FreeList::new();
        unsafe {
            assert!(list.deallocate(base + 16, 16368));
        }
        let p = list.allocate(16, 4096).unwrap() as usize;
        assert_eq!(p, base + 4096);
        let prefix = list.allocate(4080, 16).unwrap() as usize;
        assert_eq!(prefix, base + 16);
        unsafe {
            assert!(list.deallocate(p, 16));
            assert!(list.deallocate(prefix, 4080));
        }
        assert_eq!(list.block_count(), 1);
        assert_eq!(list.allocate(16368, 16).unwrap() as usize, base + 16);
        assert_eq!(list.block_count(), 0);
    }

    #[test]
    fn rejects_overlap_double_free_and_overflow() {
        let mut arena = Arena([0; 16384]);
        let base = arena.0.as_mut_ptr() as usize;
        let mut list = FreeList::new();
        unsafe {
            assert!(list.deallocate(base, 16384));
            assert!(!list.deallocate(base, 16384));
            assert!(!list.deallocate(base + 16, 16));
            assert!(!list.deallocate(usize::MAX - 15, 32));
        }
        assert_eq!(align_up(usize::MAX, 16), None);
        assert_eq!(list.allocate(16384, 16).unwrap() as usize, base);
    }

    #[test]
    fn fragmentation_stress_restores_entire_arena() {
        let mut arena = Arena([0; 16384]);
        let base = arena.0.as_mut_ptr() as usize;
        let mut list = FreeList::new();
        unsafe {
            assert!(list.deallocate(base, 16384));
        }
        for _ in 0..100 {
            let mut blocks = Vec::new();
            for i in 0..32 {
                let size = (i % 7 + 1) * 16;
                let alignment = 16 << (i % 5);
                let p = list.allocate(size, alignment).unwrap();
                assert_eq!(p as usize % alignment, 0);
                unsafe {
                    p.write_bytes(i as u8, size);
                }
                blocks.push((p, size, i as u8));
            }
            for (p, size, value) in &blocks {
                assert!(unsafe { core::slice::from_raw_parts(*p, *size) }
                    .iter()
                    .all(|b| b == value));
            }
            for index in (0..32).step_by(2).chain((1..32).rev().step_by(2)) {
                let (p, size, _) = blocks[index];
                unsafe {
                    assert!(list.deallocate(p as usize, size));
                }
            }
            assert_eq!(list.block_count(), 1);
            let p = list.allocate(16384, 16).unwrap();
            unsafe {
                assert!(list.deallocate(p as usize, 16384));
            }
        }
    }
}
