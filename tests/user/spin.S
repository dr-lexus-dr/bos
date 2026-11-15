.text
.global _start
_start:
    mov $1, %eax
    mov $1, %edi
    lea ready(%rip), %rsi
    mov $11, %edx
    int $0x80
    movabs $0x123456789abcdef0, %r12
    movq %r12, %xmm0
    movabs $0x0fedcba987654321, %r13
spin:
    movq %xmm0, %rax
    cmp %r12, %rax
    jne fail
    movabs $0x0fedcba987654321, %rax
    cmp %rax, %r13
    jne fail
    pause
    jmp spin
fail:
    mov $1, %eax
    mov $2, %edi
    lea failure(%rip), %rsi
    mov $18, %edx
    int $0x80
    mov $60, %eax
    mov $99, %edi
    int $0x80
.section .rodata
ready: .ascii "SPIN READY\n"
failure: .ascii "CONTEXT CORRUPTED\n"
