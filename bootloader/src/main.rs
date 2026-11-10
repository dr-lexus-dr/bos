#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(alloc_error_handler)]
#![feature(panic_info_message)]

extern crate alloc;

use alloc::{vec::Vec, string::String};
use alloc::vec;
use core::{mem, ptr, slice};
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileMode, FileAttribute, FileInfo, Directory};
use uefi::fs::{FileSystem, Path};
use uefi::table::boot::{MemoryType as UefiMemoryType, MemoryDescriptor, AllocateType};
use uefi::{CString16, Guid};
use log::{info, warn, error};
use sha2::{Digest, Sha256};
use uuid::{Builder, Uuid};

use bloader_shared::{
    KernelHeader, BootInfo, MemoryRegion, MemoryType, MemoryFlags,
    FramebufferInfo, PixelFormat,
    KernelEntryFn, KernelFlags, BootError,
    KERNEL_MAGIC, BOOT_PROTOCOL_VERSION, MAX_CMDLINE_LEN,
};

const KERNEL_FILE_PATH: &str = "\\EFI\\bOS\\kernel.bin";
const INITRD_FILE_PATH: &str = "\\EFI\\bOS\\initrd.img";
const BLOADER_VERSION: &str = env!("CARGO_PKG_VERSION");
const BLOADER_NAME: &str = "bLOADER";

#[entry]
fn efi_main(image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    uefi_services::init(&mut system_table).expect("bLOADER: uefi_services init failed");

    info!("=== {} v{} ===", BLOADER_NAME, BLOADER_VERSION);
    info!("bloader entry at 0x{:x}", efi_main as usize);
    info!("Starting bOS boot process...");

    boot(system_table, image_handle)
}

fn boot(mut system_table: SystemTable<Boot>, image_handle: Handle) -> Status {
    let boot_services = system_table.boot_services();

    // Disable watchdog timer (code must be > 0xffff per spec assert).
    if option_env!("BLOADER_NO_WDOG").is_none() { let _ = boot_services.set_watchdog_timer(0, 0x10000, None); }

    let (kernel_header, kernel_data) = match load_kernel(boot_services, image_handle) {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to load kernel: {}", e);
            return Status::LOAD_ERROR;
        }
    };

    info!("Kernel: {} v{}", kernel_header.name_str(), kernel_header.version);
    info!("Kernel size: {} bytes", kernel_header.size);

    let (initrd_addr, initrd_size) = if option_env!("BLOADER_NO_INITRD").is_some() { (None, None) } else { load_initrd(boot_services, image_handle).unwrap_or((None, None)) };
    if let Some(addr) = initrd_addr {
        info!("Initrd at 0x{:x} ({} bytes)", addr, initrd_size.unwrap_or(0));
    }

    let framebuffer = if option_env!("BLOADER_NO_GOP").is_some() { None } else { setup_framebuffer(boot_services) };
    let rsdp_addr = find_rsdp(&system_table);
    let cmdline = get_command_line(boot_services, image_handle);

    let mut boot_info = prepare_boot_info(
        &kernel_header,
        framebuffer,
        rsdp_addr,
        cmdline,
        initrd_addr,
        initrd_size,
    );

    // Copy kernel to its load address.
    let kernel_entry = place_kernel(boot_services, &kernel_header, &kernel_data);
    boot_info.kernel_entry = kernel_entry;
    info!("Kernel entry point: 0x{:x}", kernel_entry);

    // Collect memory map into our own format BEFORE exiting boot services.
    if option_env!("BLOADER_NO_MMAP").is_none() {
        let size = boot_services.memory_map_size();
        let buf_size = size.map_size + 8 * size.entry_size;
        let mut mmap_buf = vec![0u8; buf_size];
        match boot_services.memory_map(&mut mmap_buf) {
            Ok(mmap) => {
                let mut map = Vec::new();
                for desc in mmap.entries() {
                    map.push(convert_region(desc));
                }
                // Kernel region marker.
                map.push(MemoryRegion {
                    base_addr: kernel_header.load_addr,
                    length: kernel_header.size,
                    region_type: MemoryType::KernelCode,
                    flags: MemoryFlags::READ
                        | MemoryFlags::EXECUTE
                        | MemoryFlags::PRESENT,
                });
                boot_info.memory_map_entries = map.len();
                boot_info.memory_map = map;
            }
            Err(e) => {
                error!("memory_map failed: {:?}", e);
                return Status::BUFFER_TOO_SMALL;
            }
        }
    }

    info!("Exiting boot services...");
    // exit_boot_services consumes the system table; after this no UEFI logging.
    let _rt = system_table.exit_boot_services();

    unsafe {
        let entry_fn: KernelEntryFn = mem::transmute(kernel_entry);
        let boot_info_static: &'static BootInfo = &*(&boot_info as *const BootInfo);
        entry_fn(boot_info_static);
    }
}

fn open_fs(
    boot_services: &BootServices,
    image_handle: Handle,
) -> Result<FileSystem, BootError> {
    boot_services
        .get_image_file_system(image_handle)
        .map_err(|_| BootError::FilesystemError)
}

fn is_elf(data: &[u8]) -> bool {
    data.len() > 64
        && data[0] == 0x7F
        && data[1] == b'E'
        && data[2] == b'L'
        && data[3] == b'F'
        && data[4] == 2
}

fn read_u16le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn read_u32le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn read_u64le(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off], b[off + 1], b[off + 2], b[off + 3], b[off + 4], b[off + 5], b[off + 6],
        b[off + 7],
    ])
}
fn read_i64le(b: &[u8], off: usize) -> i64 {
    read_u64le(b, off) as i64
}

/// Load kernel.bin: native bOS image (KERNEL_MAGIC) or ELF64 (PIE supported).
fn load_kernel(
    boot_services: &BootServices,
    image_handle: Handle,
) -> Result<(KernelHeader, Vec<u8>), BootError> {
    let mut fs = open_fs(boot_services, image_handle)?;
    let path = CString16::try_from(KERNEL_FILE_PATH).map_err(|_| BootError::KernelNotFound)?;
    let data = fs
        .read(Path::new(&path))
        .map_err(|_| BootError::KernelNotFound)?;
    if data.len() < mem::size_of::<KernelHeader>() {
        return Err(BootError::KernelCorrupted);
    }

    let mut header = if is_elf(&data) {
        load_kernel_elf(&data)?
    } else {
        let header = unsafe { ptr::read_unaligned(data.as_ptr().cast::<KernelHeader>()) };
        if header.magic != KERNEL_MAGIC {
            error!("Invalid kernel magic: 0x{:x}", header.magic);
            return Err(BootError::InvalidKernelHeader);
        }
        if header.version > BOOT_PROTOCOL_VERSION {
            return Err(BootError::UnsupportedVersion);
        }
        header
    };
    if header.uuid.is_nil() {
        header.uuid = kernel_image_uuid(&data);
    }
    Ok((header, data))
}

/// Identify an exact kernel image independently of its address or boot session.
fn kernel_image_uuid(data: &[u8]) -> Uuid {
    let digest = Sha256::digest(data);
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    Builder::from_custom_bytes(bytes).into_uuid()
}

/// Build a synthetic KernelHeader for an ELF64 image (x86-64, PT_LOAD).
fn load_kernel_elf(data: &[u8]) -> Result<KernelHeader, BootError> {
    if read_u16le(data, 0x12) != 62 {
        return Err(BootError::UnsupportedArchitecture);
    }
    let entry = read_u64le(data, 0x18);
    let phoff = read_u64le(data, 0x20) as usize;
    let phentsize = read_u16le(data, 0x36) as usize;
    let phnum = read_u16le(data, 0x38) as usize;
    if phentsize < 56 || phoff + phentsize * phnum > data.len() {
        return Err(BootError::KernelCorrupted);
    }

    let mut max_end: u64 = 0;
    let mut has_load = false;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if read_u32le(data, ph) != 1 {
            continue; // PT_LOAD
        }
        has_load = true;
        let vaddr = read_u64le(data, ph + 16);
        let memsz = read_u64le(data, ph + 40);
        max_end = max_end.max(vaddr + memsz);
        let filesz = read_u64le(data, ph + 32) as usize;
        let off = read_u64le(data, ph + 8) as usize;
        if off + filesz > data.len() {
            return Err(BootError::KernelCorrupted);
        }
    }
    if !has_load || max_end == 0 {
        return Err(BootError::KernelCorrupted);
    }
    let total = (max_end + 4095) & !4095;

    // Synthesize header: RELOCATABLE ELF, entry relative to base.
    let mut header: KernelHeader = unsafe { mem::zeroed() };
    header.magic = KERNEL_MAGIC;
    header.version = BOOT_PROTOCOL_VERSION;
    header.entry_offset = entry; // rebased to absolute entry at placement
    header.load_addr = 0;
    header.size = total;
    header.flags = KernelFlags::RELOCATABLE;
    header.required_memory = total;
    Ok(header)
}

/// Copy the loaded image to executable pages. Returns absolute entry point.
fn place_kernel(
    boot_services: &BootServices,
    kernel_header: &KernelHeader,
    kernel_data: &[u8],
) -> u64 {
    if is_elf(kernel_data) {
        return place_kernel_elf(boot_services, kernel_data);
    }
    let pages = ((kernel_header.size + 4095) / 4096) as usize;
    let target = if kernel_header.flags.contains(KernelFlags::RELOCATABLE) {
        boot_services
            .allocate_pages(
                AllocateType::AnyPages,
                UefiMemoryType::LOADER_CODE,
                pages,
            )
            .unwrap_or(kernel_header.load_addr)
    } else {
        match boot_services.allocate_pages(
            AllocateType::Address(kernel_header.load_addr),
            UefiMemoryType::LOADER_CODE,
            pages,
        ) {
            Ok(addr) => addr,
            Err(_) => {
                warn!("preferred load addr busy, using AnyPages");
                boot_services
                    .allocate_pages(
                        AllocateType::AnyPages,
                        UefiMemoryType::LOADER_CODE,
                        pages,
                    )
                    .expect("out of memory for kernel")
            }
        }
    };

    unsafe {
        ptr::copy_nonoverlapping(
            kernel_data.as_ptr(),
            target as *mut u8,
            kernel_data.len().min(kernel_header.size as usize),
        );
    }
    target + kernel_header.entry_offset
}

/// Load ELF64 PT_LOAD segments into contiguous pages. Returns entry point.
fn place_kernel_elf(boot_services: &BootServices, data: &[u8]) -> u64 {
    let entry = read_u64le(data, 0x18);
    let phoff = read_u64le(data, 0x20) as usize;
    let phentsize = read_u16le(data, 0x36) as usize;
    let phnum = read_u16le(data, 0x38) as usize;

    let mut max_end: u64 = 0;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if read_u32le(data, ph) != 1 {
            continue;
        }
        let vaddr = read_u64le(data, ph + 16);
        let memsz = read_u64le(data, ph + 40);
        max_end = max_end.max(vaddr + memsz);
    }
    let total = (max_end + 4095) & !4095;
    let pages = (total / 4096) as usize;
    // Keep the kernel below 4GB: early page tables only map low 4GB.
    let base = match boot_services.allocate_pages(
        AllocateType::MaxAddress(0x1_0000_0000),
        UefiMemoryType::LOADER_CODE,
        pages,
    ) {
        Ok(addr) => addr,
        Err(_) => {
            warn!("no room below 4GB, trying AnyPages (may not boot)");
            boot_services
                .allocate_pages(
                    AllocateType::AnyPages,
                    UefiMemoryType::LOADER_CODE,
                    pages,
                )
                .expect("out of memory for kernel")
        }
    };

    unsafe {
        ptr::write_bytes(base as *mut u8, 0, total as usize);
    }
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if read_u32le(data, ph) != 1 {
            continue;
        }
        let off = read_u64le(data, ph + 8) as usize;
        let vaddr = read_u64le(data, ph + 16);
        let filesz = read_u64le(data, ph + 32) as usize;
        let memsz = read_u64le(data, ph + 40) as usize;
        if filesz > 0 {
            unsafe {
                ptr::copy_nonoverlapping(
                    data.as_ptr().add(off),
                    (base + vaddr) as *mut u8,
                    filesz.min(memsz).min(data.len().saturating_sub(off)),
                );
            }
        }
    }
    // Static-PIE kernels (ET_DYN) carry R_X86_64_RELATIVE relocations that
    // must be applied for the load bias `base`; otherwise every absolute
    // address in the image still points at link-time vaddrs and the kernel
    // jumps into garbage right after entry.
    if let Err(e) = apply_relocations(base, total, data) {
        error!("kernel relocation failed: {}", e);
        // Relocation failure is fatal: boot the (broken) image would only
        // triple-fault. Halt here with boot services still alive so the
        // message above stays visible.
        loop {
            core::hint::spin_loop();
        }
    }
    info!(
        "ELF kernel placed: base=0x{:x} size={} entry=0x{:x}",
        base,
        total,
        base + entry
    );
    base + entry
}

const PT_DYNAMIC: u32 = 2;
const DT_NULL: i64 = 0;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const R_X86_64_RELATIVE: u32 = 8;
const RELA_ENTRY_SIZE: u64 = 24;

/// Map a link-time vaddr to a file offset via PT_LOAD segments.
fn va_to_offset(data: &[u8], va: u64) -> Option<usize> {
    let phoff = read_u64le(data, 0x20) as usize;
    let phentsize = read_u16le(data, 0x36) as usize;
    let phnum = read_u16le(data, 0x38) as usize;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if ph + 56 > data.len() {
            return None;
        }
        if read_u32le(data, ph) != 1 {
            continue; // PT_LOAD
        }
        let off = read_u64le(data, ph + 8);
        let vaddr = read_u64le(data, ph + 16);
        let filesz = read_u64le(data, ph + 32);
        if va >= vaddr && va < vaddr + filesz {
            return Some((off + (va - vaddr)) as usize);
        }
    }
    None
}

/// Apply R_X86_64_RELATIVE relocations to the loaded image at `base`
/// (`image_size` bounds-checks relocation targets).
fn apply_relocations(base: u64, image_size: u64, data: &[u8]) -> Result<(), BootError> {
    let phoff = read_u64le(data, 0x20) as usize;
    let phentsize = read_u16le(data, 0x36) as usize;
    let phnum = read_u16le(data, 0x38) as usize;

    // Locate PT_DYNAMIC.
    let mut dyn_va = None;
    let mut dyn_filesz = 0u64;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if ph + 56 > data.len() {
            return Err(BootError::KernelCorrupted);
        }
        if read_u32le(data, ph) == PT_DYNAMIC {
            dyn_va = Some(read_u64le(data, ph + 16));
            dyn_filesz = read_u64le(data, ph + 32);
        }
    }
    let dyn_va = match dyn_va {
        Some(v) => v,
        None => return Ok(()), // no dynamic section: nothing to relocate
    };
    let dyn_off = va_to_offset(data, dyn_va).ok_or(BootError::KernelCorrupted)?;

    // Walk Elf64_Dyn { d_tag: i64, d_val: u64 } until DT_NULL.
    let mut rela_va = 0u64;
    let mut relasz = 0u64;
    let mut relaent = RELA_ENTRY_SIZE;
    let mut off = dyn_off;
    let dyn_end = dyn_off.saturating_add(dyn_filesz as usize);
    while off + 16 <= data.len() && off + 16 <= dyn_end {
        let tag = read_i64le(data, off);
        let val = read_u64le(data, off + 8);
        match tag {
            DT_NULL => break,
            DT_RELA => rela_va = val,
            DT_RELASZ => relasz = val,
            DT_RELAENT => relaent = val,
            _ => {}
        }
        off += 16;
    }
    if relasz == 0 {
        return Ok(());
    }
    if relaent != RELA_ENTRY_SIZE {
        return Err(BootError::KernelCorrupted);
    }
    let rela_off = va_to_offset(data, rela_va).ok_or(BootError::KernelCorrupted)?;
    let count = relasz / relaent;
    let mut applied = 0u64;
    for i in 0..count {
        let e = rela_off + (i as usize) * (RELA_ENTRY_SIZE as usize);
        if e + 24 > data.len() {
            return Err(BootError::KernelCorrupted);
        }
        let r_offset = read_u64le(data, e);
        let r_info = read_u64le(data, e + 8);
        // NB: RELA addend lives in the entry, NOT in memory (linkers may
        // leave the location zeroed) — so this must be `base + r_addend`,
        // never `*loc += base`.
        let r_addend = read_i64le(data, e + 16);
        if (r_info & 0xffff_ffff) != R_X86_64_RELATIVE as u64 {
            error!("unsupported rela type {} @0x{:x}", r_info & 0xffff_ffff, r_offset);
            return Err(BootError::KernelCorrupted);
        }
        if r_offset + 8 > image_size {
            return Err(BootError::KernelCorrupted);
        }
        unsafe {
            let loc = (base + r_offset) as *mut u64;
            loc.write_unaligned(base.wrapping_add(r_addend as u64));
        }
        applied += 1;
    }
    info!("kernel relocations applied: {}", applied);
    Ok(())
}

fn load_initrd(
    boot_services: &BootServices,
    image_handle: Handle,
) -> Result<(Option<u64>, Option<u64>), BootError> {
    let mut fs = match open_fs(boot_services, image_handle) {
        Ok(f) => f,
        Err(_) => return Ok((None, None)),
    };
    let path = match CString16::try_from(INITRD_FILE_PATH) {
        Ok(p) => p,
        Err(_) => return Ok((None, None)),
    };
    let data = match fs.read(Path::new(&path)) {
        Ok(d) => d,
        Err(_) => return Ok((None, None)),
    };
    let file_size = data.len();
    if file_size == 0 {
        return Ok((None, None));
    }

    let pages = (file_size + 4095) / 4096;
    // Same <4GB constraint as the kernel: the kernel touches initrd via
    // identity-mapped physical addresses.
    let addr = match boot_services.allocate_pages(
        AllocateType::MaxAddress(0x1_0000_0000),
        UefiMemoryType::LOADER_DATA,
        pages,
    ) {
        Ok(addr) => addr,
        Err(_) => boot_services
            .allocate_pages(AllocateType::AnyPages, UefiMemoryType::LOADER_DATA, pages)
            .map_err(|_| BootError::InsufficientMemory)?,
    };

    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), addr as *mut u8, file_size);
    }

    info!("Initrd loaded: {} bytes at 0x{:x}", file_size, addr);
    Ok((Some(addr), Some(file_size as u64)))
}

fn setup_framebuffer(boot_services: &BootServices) -> Option<FramebufferInfo> {
    let gop_handle = boot_services
        .get_handle_for_protocol::<GraphicsOutput>()
        .ok()?;
    let mut gop = boot_services
        .open_protocol_exclusive::<GraphicsOutput>(gop_handle)
        .ok()?;

    let mode = gop.current_mode_info();
    let (w, h) = mode.resolution();
    let pixel_format = match mode.pixel_format() {
        UefiPixelFormat::Rgb => PixelFormat::RGB,
        UefiPixelFormat::Bgr => PixelFormat::BGR,
        UefiPixelFormat::Bitmask => PixelFormat::Bitmask,
        UefiPixelFormat::BltOnly => return None,
    };
    let base = gop.frame_buffer().as_mut_ptr() as u64;

    Some(FramebufferInfo {
        base_addr: base,
        width: w as u32,
        height: h as u32,
        stride: mode.stride() as u32,
        pixel_format,
        bits_per_pixel: 32,
    })
}

fn find_rsdp(system_table: &SystemTable<Boot>) -> Option<u64> {
    for entry in system_table.config_table() {
        if entry.guid == uefi::table::cfg::ACPI2_GUID
            || entry.guid == uefi::table::cfg::ACPI_GUID
        {
            return Some(entry.address as u64);
        }
    }
    None
}

fn get_command_line(boot_services: &BootServices, image_handle: Handle) -> String {
    if let Ok(img) = boot_services.open_protocol_exclusive::<LoadedImage>(image_handle) {
        if let Some(bytes) = img.load_options_as_bytes() {
            let s: String = bytes
                .chunks(2)
                .map(|c| c[0] as char)
                .filter(|c| *c != '\0')
                .collect();
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

fn convert_region(desc: &MemoryDescriptor) -> MemoryRegion {
    let region_type = match desc.ty {
        UefiMemoryType::RESERVED => MemoryType::Reserved,
        UefiMemoryType::LOADER_CODE => MemoryType::LoaderCode,
        UefiMemoryType::LOADER_DATA => MemoryType::LoaderData,
        UefiMemoryType::BOOT_SERVICES_CODE => MemoryType::BootServicesCode,
        UefiMemoryType::BOOT_SERVICES_DATA => MemoryType::BootServicesData,
        UefiMemoryType::RUNTIME_SERVICES_CODE => MemoryType::RuntimeServicesCode,
        UefiMemoryType::RUNTIME_SERVICES_DATA => MemoryType::RuntimeServicesData,
        UefiMemoryType::CONVENTIONAL => MemoryType::Conventional,
        UefiMemoryType::UNUSABLE => MemoryType::Unusable,
        UefiMemoryType::ACPI_RECLAIM => MemoryType::ACPIReclaim,
        UefiMemoryType::ACPI_NON_VOLATILE => MemoryType::ACPIMemoryNVS,
        UefiMemoryType::MMIO => MemoryType::MemoryMappedIO,
        UefiMemoryType::MMIO_PORT_SPACE => MemoryType::MemoryMappedIOPortSpace,
        UefiMemoryType::PAL_CODE => MemoryType::PalCode,
        UefiMemoryType::PERSISTENT_MEMORY => MemoryType::PersistentMemory,
        _ => MemoryType::Reserved,
    };

    let mut flags = MemoryFlags::PRESENT;
    if desc.ty == UefiMemoryType::CONVENTIONAL
        || desc.ty == UefiMemoryType::PERSISTENT_MEMORY
    {
        flags |= MemoryFlags::READ | MemoryFlags::WRITE | MemoryFlags::EXECUTE;
    }

    MemoryRegion {
        base_addr: desc.phys_start,
        length: desc.page_count * 4096,
        region_type,
        flags,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_boot_info(
    kernel_header: &KernelHeader,
    framebuffer: Option<FramebufferInfo>,
    rsdp_addr: Option<u64>,
    cmdline: String,
    initrd_addr: Option<u64>,
    initrd_size: Option<u64>,
) -> BootInfo {
    let mut cmdline_buf = [0u8; MAX_CMDLINE_LEN];
    let cmdline_bytes = cmdline.as_bytes();
    let cmdline_len = cmdline_bytes.len().min(MAX_CMDLINE_LEN);
    cmdline_buf[..cmdline_len].copy_from_slice(&cmdline_bytes[..cmdline_len]);

    let mut bootloader_name = [0u8; 64];
    let name_bytes = BLOADER_NAME.as_bytes();
    bootloader_name[..name_bytes.len().min(64)].copy_from_slice(name_bytes);

    let mut bootloader_version = [0u8; 32];
    let ver_bytes = BLOADER_VERSION.as_bytes();
    bootloader_version[..ver_bytes.len().min(32)].copy_from_slice(ver_bytes);

    BootInfo {
        magic: bloader_shared::BLOADER_MAGIC,
        version: BOOT_PROTOCOL_VERSION,
        kernel_entry: 0, // filled in after placement
        kernel_size: kernel_header.size,
        memory_map: Vec::new(), // filled in after memory_map call
        memory_map_entries: 0,
        framebuffer,
        rsdp_addr,
        cmdline: cmdline_buf,
        cmdline_len,
        modules: Vec::new(),
        boot_time_us: 0,
        bootloader_name,
        bootloader_version,
        cpu_count: 1,
        bsp_lapic_id: 0,
        kernel_uuid: kernel_header.uuid,
        initrd_addr,
        initrd_size,
    }
}

#[allow(dead_code)]
fn guid_from_u128(_v: u128) -> Guid {
    unimplemented!()
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    error!("Allocation error: {:?}", layout);
    loop {
        core::hint::spin_loop();
    }
}
