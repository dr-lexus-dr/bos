//! BIOS entry to the same kernel, VFS, shell and network stack used by bLOADER.
use alloc::{boxed::Box, vec::Vec};
use bloader_shared::{
    BootInfo, FramebufferInfo, MemoryFlags, MemoryRegion, MemoryType, PixelFormat,
};

core::arch::global_asm!(
    r#"
    .section .text.live_entry,"ax"
    .global live_entry
    live_entry:
        cli
        cld
        lea rsp, [rip + live_stack_top]
        // Preserve the boot parameters across the switch from firmware paging.
        mov r13d, r9d
        mov r12d, r8d
        mov r8d, edi
        mov r9d, esi
        mov r10d, edx
        mov r11d, ecx
        lea rdi, [rip + live_tables]
        lea rax, [rdi + 4096 + 3]
        mov [rdi], rax
        add rdi, 4096
        lea rax, [rdi + 4096 + 3]
        mov ecx, 4
    3:  stosq
        add rax, 4096
        loop 3b
        lea rdi, [rip + live_tables + 8192]
        mov eax, 0x83
        mov ecx, 2048
    2:  stosq
        add rax, 0x200000
        loop 2b
        lea rax, [rip + live_tables]
        mov cr3, rax
        mov rax, cr0
        and rax, -13
        or rax, 2
        mov cr0, rax
        mov rax, cr4
        or rax, 0x600
        mov cr4, rax
        mov edi, r8d
        mov esi, r9d
        mov edx, r10d
        mov ecx, r11d
        mov r8d, r12d
        mov r9d, r13d
        call live_start
        ud2
    .section .bss.live_stack,"aw",@nobits
    .balign 16
        .skip 131072
    live_stack_top:
    .balign 4096
    live_tables:
        .skip 24576
    .text
"#
);

extern "C" {
    static __kernel_end: u8;
}

pub fn resident_bytes() -> u64 {
    core::ptr::addr_of!(__kernel_end) as u64 - 0x100000
}

#[no_mangle]
extern "sysv64" fn live_start(
    ram_end: u32,
    base: u32,
    size: u32,
    pitch: u32,
    arena: u32,
    web: u32,
) -> ! {
    unsafe {
        crate::web::engine::install(web as usize);
    }
    let start = if arena == 0 {
        core::ptr::addr_of!(__kernel_end) as u64
    } else {
        arena as u64
    };
    let region = MemoryRegion {
        base_addr: start,
        length: u64::from(ram_end) - start,
        region_type: MemoryType::Conventional,
        flags: MemoryFlags::READ | MemoryFlags::WRITE,
    };
    let mut info = BootInfo {
        magic: bloader_shared::BLOADER_MAGIC,
        version: bloader_shared::BOOT_PROTOCOL_VERSION,
        kernel_entry: 0x100000,
        kernel_size: resident_bytes(),
        memory_map: Vec::new(),
        memory_map_entries: 1,
        framebuffer: (base != 0).then_some(FramebufferInfo {
            base_addr: u64::from(base),
            width: size & 0xffff,
            height: size >> 16,
            stride: pitch & 0x7fff_ffff,
            bits_per_pixel: 32,
            pixel_format: if pitch >> 31 == 1 {
                PixelFormat::RGB
            } else {
                PixelFormat::BGR
            },
        }),
        rsdp_addr: None,
        cmdline: [0; bloader_shared::MAX_CMDLINE_LEN],
        cmdline_len: 4,
        modules: Vec::new(),
        boot_time_us: 0,
        bootloader_name: [0; 64],
        bootloader_version: [0; 32],
        cpu_count: 1,
        bsp_lapic_id: 0,
        kernel_uuid: uuid::Uuid::nil(),
        initrd_addr: None,
        initrd_size: None,
    };
    info.cmdline[..4].copy_from_slice(b"live");
    info.bootloader_name[..8].copy_from_slice(b"bOS Live");
    info.bootloader_version[..3].copy_from_slice(b"1.0");
    crate::boottime::init();
    crate::early_init(&info, core::slice::from_ref(&region));
    crate::kernel_init(&info);
    info.memory_map.push(region);
    log::info!(
        "Live kernel {} bytes, usable RAM ends at {:#x}",
        info.kernel_size,
        ram_end
    );
    crate::init::kernel_main(Box::leak(Box::new(info)))
}
