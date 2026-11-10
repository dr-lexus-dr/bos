/*
 * bOS Kernel Entry Assembly
 * x86_64 entry point - sets up minimal environment before jumping to Rust
 */

.att_syntax
.section .text
.code64
.global _start
.global _start_ap

/* Boot info pointer is passed in RDI (System V AMD64 ABI) */
/* We need to preserve it and set up stack before calling Rust */

_start:
    /* Interrupts must stay off: the UEFI IDT is still installed but our
     * own page tables are about to replace UEFI's — a timer IRQ in between
     * would vector into UEFI handlers with an address space they don't know. */
    cli

    /* Save boot info pointer */
    mov %rdi, %rbx

    /* Set up temporary stack (RIP-relative for PIE) */
    lea early_stack_top(%rip), %rsp

    /* Clear direction flag */
    cld

    /* Build early page tables: identity-map low 4GB with 1GB pages
     * and alias it at KERNEL_BASE (0xFFFF800000000000, PML4[256]).
     * UEFI runs us with its own tables; Rust needs higher-half access.
     * NOTE: must cover the whole low 4GB — the bootloader places the
     * kernel with AllocatePages(AnyPages), which typically lands above
     * 1GB (e.g. ~2GB under QEMU -m 2G). Mapping only 1GB page-faults
     * on the first instruction fetch after CR3 reload. */
    lea boot_pml4(%rip), %rax
    lea boot_pdpt_low(%rip), %rcx
    orq $0x3, %rcx
    mov %rcx, 0(%rax)            /* PML4[0] -> PDPT_low */
    lea boot_pdpt_high(%rip), %rcx
    orq $0x3, %rcx
    mov %rcx, 2048(%rax)         /* PML4[256] -> PDPT_high (256*8=2048) */
    /* Fill both PDPTs with 4 x 1GB pages: addr = i*1GB | PRESENT|WRITABLE|HUGE(PS) */
    lea boot_pdpt_low(%rip), %rdx
    lea boot_pdpt_high(%rip), %rsi
    xor %rcx, %rcx
1:
    mov %rcx, %rax
    shlq $30, %rax
    orq $0x83, %rax
    mov %rax, (%rdx,%rcx,8)
    mov %rax, (%rsi,%rcx,8)
    inc %rcx
    cmp $4, %rcx
    jne 1b
    /* Load our tables */
    lea boot_pml4(%rip), %rax
    mov %rax, %cr3

    /* Zero out BSS section */
    /* BSS is zeroed by the bootloader, but let's be sure */
    /* extern char _bss_start, _bss_end; */
    /* This is done by the linker script */

    /* Call Rust entry point */
    mov %rbx, %rdi
    call _start_rust

    /* Should never return */
    cli
    hlt
    jmp .

/* AP entry point - CPU ID in RDI, boot info in RSI */
_start_ap:
    /* Save arguments */
    mov %rdi, %rbx    /* cpu_id */
    mov %rsi, %r12    /* boot_info */

    /* Set up AP stack - each CPU gets its own stack */
    /* Stack pointer will be set by the BSP before starting AP */

    cld

    /* Call AP Rust entry */
    mov %rbx, %rdi
    mov %r12, %rsi
    call _start_ap_rust

    cli
    hlt
    jmp .

/* Symbol for linker */
.size _start, . - _start
.size _start_ap, . - _start_ap

.section .bss
.align 4096
.global early_stack_bottom
.global early_stack_top
early_stack_bottom:
    .skip 65536  /* 64 KB early stack: debug builds have fat frames and
                  * deep format/panic chains; 16 KB overflowed into the
                  * .bss statics living just below (lock bytes, Once state) */
early_stack_top:
    .skip 4096
boot_pml4:
    .skip 4096
boot_pdpt_low:
    .skip 4096
boot_pdpt_high:
    .skip 4096