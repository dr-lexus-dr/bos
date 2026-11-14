OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(5); }
SECTIONS {
    . = 0x8000000000;
    .text : { *(.text*) *(.rodata*) *(.data*) *(.bss*) } :text
    /DISCARD/ : { *(.note*) *(.comment) }
}
