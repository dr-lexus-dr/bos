#![no_std]
#![no_main]

//! Removable-media boot. BIOS reads the same compressed blocks from their
//! sector-aligned PE section. EFI already has the image in memory.
use core::{mem, ptr, slice};
use uefi::{
    prelude::*,
    table::boot::{AllocateType, MemoryType},
};
#[path = "../display.rs"]
mod display;

const BASE: u64 = 0x100000;

const DATA: &[u8] = include_bytes!(env!("BOS_LIVE_BLOCKS"));
#[used]
#[link_section = ".bos"]
static PACKED: [u8; DATA.len()] = *include_bytes!(env!("BOS_LIVE_BLOCKS"));

extern "C" {
    fn unpack(src: *const u8, size: u32, dst: *mut u8, plain: u32, workspace: *mut u8) -> i32;
}

fn decode(image: &mut [u8]) -> bool {
    let source = &PACKED[..];
    if source.len() < 512 || &source[..4] != b"bOS3" {
        return false;
    }
    let plain = u32::from_le_bytes(source[4..8].try_into().unwrap());
    let size = u32::from_le_bytes(source[8..12].try_into().unwrap());
    if plain == 0 || plain as usize + 32768 > image.len() || size as usize > source.len() - 512 {
        return false;
    }
    let (output, free) = image.split_at_mut(image.len() - 32768);
    unsafe {
        unpack(
            source[512..].as_ptr(),
            size,
            output.as_mut_ptr(),
            plain,
            free.as_mut_ptr(),
        ) == 1
    }
}

#[entry]
fn main(_handle: Handle, mut system: SystemTable<Boot>) -> Status {
    if let Err(e) = uefi_services::init(&mut system) {
        return e.status();
    }
    let bs = system.boot_services();
    let resident = u32::from_le_bytes(PACKED[12..16].try_into().unwrap()) as usize;
    if !(4096..16 * 1024 * 1024).contains(&resident) {
        return Status::LOAD_ERROR;
    }
    let len = (resident + 32768 + 4095) & !4095;
    if let Err(e) = bs.allocate_pages(
        AllocateType::Address(BASE),
        MemoryType::LOADER_CODE,
        len / 4096,
    ) {
        log::error!("bOS Live requires contiguous low memory at 1 MiB");
        return e.status();
    }
    let image = unsafe { slice::from_raw_parts_mut(BASE as *mut u8, len) };
    unsafe {
        ptr::write_bytes(image.as_mut_ptr(), 0, len);
    }
    if !decode(image) {
        let _ = bs.free_pages(BASE, len / 4096);
        return Status::LOAD_ERROR;
    }
    let offset = 512 + u32::from_le_bytes(PACKED[8..12].try_into().unwrap()) as usize;
    let blob = &PACKED[offset..];
    if blob.len() < 12
        || blob.len() != u32::from_le_bytes(PACKED[16..20].try_into().unwrap()) as usize
    {
        return Status::LOAD_ERROR;
    }
    let Ok(web) = bs.allocate_pages(
        AllocateType::MaxAddress(0x3fff_ffff),
        MemoryType::LOADER_DATA,
        blob.len().div_ceil(4096),
    ) else {
        return Status::OUT_OF_RESOURCES;
    };
    unsafe {
        ptr::copy_nonoverlapping(blob.as_ptr(), web as *mut u8, blob.len());
    }
    let arena = [768usize, 512, 384, 256, 192, 128, 96, 64, 48, 32, 24, 20]
        .into_iter()
        .find_map(|mib| {
            let bytes = mib * 1024 * 1024;
            bs.allocate_pages(
                AllocateType::MaxAddress(0x3fff_ffff),
                MemoryType::LOADER_DATA,
                bytes / 4096,
            )
            .ok()
            .map(|addr| (addr as u32, addr as u32 + bytes as u32))
        });
    let Some((start, end)) = arena else {
        let _ = bs.free_pages(BASE, len / 4096);
        return Status::OUT_OF_RESOURCES;
    };
    let mut video = (0, 0, 0);
    if let Some(fb) = display::setup_framebuffer(bs, true) {
        if matches!(
            fb.pixel_format,
            bloader_shared::PixelFormat::RGB | bloader_shared::PixelFormat::BGR
        ) && fb.base_addr + fb.stride as u64 * fb.height as u64 * 4 <= 1u64 << 32
        {
            video = (
                fb.base_addr as u32,
                fb.width | fb.height << 16,
                fb.stride | (u32::from(fb.pixel_format == bloader_shared::PixelFormat::RGB) << 31),
            );
        }
    }
    let entry: extern "sysv64" fn(u32, u32, u32, u32, u32, u32) -> ! =
        unsafe { mem::transmute(BASE as usize) };
    let _runtime = system.exit_boot_services();
    entry(end, video.0, video.1, video.2, start, web as u32)
}
