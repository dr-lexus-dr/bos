//! Bounded, non-destructive checks against the running kernel allocators.
use super::{heap, phys};
use core::alloc::Layout;

pub fn run() -> Result<(), &'static str> {
    let before = phys::free_memory();
    let frames = phys::allocate_frames(8).ok_or("frame allocation failed")?;
    if phys::free_memory() + 8 * phys::FRAME_SIZE != before {
        phys::deallocate_frames(frames);
        return Err("frame accounting mismatch");
    }
    phys::deallocate_frames(frames);
    // An immediate double free must not inflate the pool.
    phys::deallocate_frames(frames);
    if phys::free_memory() != before {
        return Err("frame release mismatch");
    }
    let used = heap::stats().used_size;
    let mut slots = [(core::ptr::null_mut::<u8>(), 0usize, 0usize); 64];
    let result = (|| {
        for (i, slot) in slots.iter_mut().enumerate() {
            let size = 17 + i * 13;
            let align = 16 << (i % 9);
            let layout = Layout::from_size_align(size, align).unwrap();
            let p = heap::allocate(layout);
            if p.is_null() {
                return Err("heap allocation failed");
            }
            *slot = (p, size, align);
            if p as usize % align != 0 {
                return Err("heap alignment mismatch");
            }
            unsafe {
                p.write_bytes(i as u8, size);
            }
        }
        for (i, (p, size, _)) in slots.iter().enumerate() {
            if unsafe { core::slice::from_raw_parts(*p, *size) }
                .iter()
                .any(|b| *b != i as u8)
            {
                return Err("overlapping heap allocations");
            }
        }
        Ok(())
    })();
    for (p, size, align) in slots.into_iter().rev() {
        if !p.is_null() {
            heap::deallocate(p, Layout::from_size_align(size, align).unwrap());
        }
    }
    result?;
    if heap::stats().used_size != used {
        return Err("heap accounting mismatch");
    }
    Ok(())
}
