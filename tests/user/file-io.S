.text
.global _start
_start:
    mov $2, %eax
    lea path(%rip), %rdi
    mov $0x242, %esi
    int $0x80
    cmp $3, %rax
    jne fail
    mov %rax, %r12
    mov %rax, %rdi
    mov $1, %eax
    lea message(%rip), %rsi
    mov $8, %edx
    int $0x80
    cmp $8, %rax
    jne fail
    mov $8, %eax
    xor %esi, %esi
    xor %edx, %edx
    int $0x80
    test %rax, %rax
    jne fail
    # A read into read-only text must fail without moving the offset.
    xor %eax, %eax
    lea message(%rip), %rsi
    mov $8, %edx
    int $0x80
    cmp $-14, %rax
    jne fail
    # A range crossing the end of the user stack must also fail atomically.
    xor %eax, %eax
    lea -4(%rsp), %rsi
    int $0x80
    cmp $-14, %rax
    jne fail
    sub $16, %rsp
    xor %eax, %eax
    mov %rsp, %rsi
    int $0x80
    cmp $8, %rax
    jne fail
    movabs $0x0a4b4f2d454c4946, %rbx
    cmp %rbx, (%rsp)
    jne fail
    mov $1, %eax
    mov $1, %edi
    int $0x80
    # Full-width descriptor validation (no truncation to u32).
    mov $3, %eax
    movabs $0x100000003, %rdi
    int $0x80
    cmp $-9, %rax
    jne fail
    mov $3, %eax
    mov %r12, %rdi
    int $0x80
    test %rax, %rax
    jne fail
    mov $3, %eax
    int $0x80
    cmp $-9, %rax
    jne fail
    mov $2, %eax
    xor %edi, %edi
    xor %esi, %esi
    int $0x80
    cmp $-14, %rax
    jne fail
    # Fill the per-program table, then leave every handle for teardown.
    mov $3, %r13
open_loop:
    mov $2, %eax
    lea path(%rip), %rdi
    xor %esi, %esi
    int $0x80
    cmp %r13, %rax
    jne fail
    inc %r13
    cmp $64, %r13
    jne open_loop
    mov $2, %eax
    int $0x80
    cmp $-24, %rax
    jne fail
    mov $1, %eax
    mov $1, %edi
    lea success(%rip), %rsi
    mov $17, %edx
    int $0x80
    mov $60, %eax
    xor %edi, %edi
    int $0x80
fail:
    mov $1, %eax
    mov $2, %edi
    lea failure(%rip), %rsi
    mov $16, %edx
    int $0x80
    mov $60, %eax
    mov $1, %edi
    int $0x80
.section .rodata
path: .asciz "/users/default/documents/user-io.txt"
message: .ascii "FILE-OK\n"
success: .ascii "FILE TEST PASSED\n"
failure: .ascii "FILE TEST FAILED\n"
