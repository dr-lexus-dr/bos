ENTRY(live_entry)
SECTIONS {
    . = 0x100000;
    .text : { KEEP(*(.text.live_entry)) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data : { *(.data .data.*) *(.got .got.*) }
    __file_end = .;
    .bss (NOLOAD) : { *(.bss .bss.* COMMON) }
    . = ALIGN(4096);
    __kernel_end = .;
    ASSERT(__kernel_end < 0x1000000, "live kernel leaves too little RAM")
    /DISCARD/ : { *(.eh_frame*) *(.note*) *(.comment*) *(.kernel_header) }
}
