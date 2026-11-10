.section .text
.global _start
_start:
    mov $1, %eax
    mov $1, %edi
    lea message(%rip), %rsi
    mov $message_size, %edx
    int $0x80
    xor %edi, %edi
    mov $60, %eax
    int $0x80
    ud2

.section .rodata
message:
    .ascii "Hello from bOS user mode!\n"
.set message_size, . - message
