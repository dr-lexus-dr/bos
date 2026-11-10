/*
 * bOS Kernel Linker Script
 * x86_64 higher-half kernel
 */

ENTRY(_start)

OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)

/* Higher half kernel base address */
KERNEL_BASE = 0xFFFF800000000000;
KERNEL_OFFSET = 0xFFFF800000000000;

/* Sections */
SECTIONS
{
    /* Kernel header at 1MB for bootloader to find */
    . = 1M;
    .kernel_header ALIGN(4096) :
    {
        KEEP(*(.kernel_header))
    } > 0

    /* Code and read-only data */
    .text KERNEL_BASE + 0x100000 : ALIGN(4096)
    {
        _text = .;
        *(.text .text.*)
        *(.rodata .rodata.*)
        *(.eh_frame*)
        *(.gcc_except_table)
        . = ALIGN(16);
    } > 0

    /* Read-only data */
    .rodata : ALIGN(4096)
    {
        *(.rodata .rodata.*)
        . = ALIGN(16);
    } > 0

    /* Read-write data */
    .data : ALIGN(4096)
    {
        _data = .;
        *(.data .data.*)
        . = ALIGN(16);
    } > 0

    /* BSS section */
    .bss : ALIGN(4096)
    {
        _bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        _bss_end = .;
    } > 0

    /* Per-CPU data */
    .percpu : ALIGN(4096)
    {
        _percpu_start = .;
        *(.percpu .percpu.*)
        . = ALIGN(16);
        _percpu_end = .;
    } > 0

    /* Kernel end marker */
    _kernel_end = .;

    /* Discard sections */
    /DISCARD/ :
    {
        *(.note.GNU-stack)
        *(.gnu.hash)
        *(.dynsym)
        *(.dynstr)
        *(.interp)
        *(.hash)
    }
}

/* Symbol definitions for Rust */
PROVIDE(_text_start = _text);
PROVIDE(_text_end = _etext);
PROVIDE(_data_start = _data);
PROVIDE(_data_end = _edata);
PROVIDE(_bss_start = _bss_start);
PROVIDE(_bss_end = _bss_end);
PROVIDE(_kernel_size = _kernel_end - 0xFFFF800000100000);