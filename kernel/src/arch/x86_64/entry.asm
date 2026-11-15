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

    /* Use 2 MiB pages: Skylake mobile CPUs need not support 1 GiB pages.
     * Cover 64 GiB, including firmware pool allocations above 4 GiB. */
    lea boot_pml4(%rip), %rax
    lea boot_pdpt_low(%rip), %rcx
    orq $3, %rcx
    mov %rcx, 0(%rax)
    lea boot_pdpt_high(%rip), %rcx
    orq $3, %rcx
    mov %rcx, 2048(%rax)
    lea boot_pdpt_low(%rip), %rdx
    lea boot_pdpt_high(%rip), %rsi
    lea boot_pds(%rip), %rdi
    xor %rcx, %rcx
1:
    mov %rcx, %rax
    shlq $12, %rax
    add %rdi, %rax
    orq $3, %rax
    mov %rax, (%rdx,%rcx,8)
    mov %rax, (%rsi,%rcx,8)
    inc %rcx
    cmp $64, %rcx
    jne 1b
    xor %rcx, %rcx
2:
    mov %rcx, %rax
    shlq $21, %rax
    orq $0x83, %rax
    mov %rax, (%rdi,%rcx,8)
    inc %rcx
    cmp $32768, %rcx
    jne 2b
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

.align 4096
boot_pds:
    .skip 262144
