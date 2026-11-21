.section .text.start,"ax"
.global _start
_start:
    mov (%esp),%eax
    mov 4(%esp),%edx
    and $-16,%esp
    sub $8,%esp
    push %edx
    push %eax
    call main
    mov %eax,%ebx
    xor %eax,%eax
    int $0x80
    ud2
.section .note.GNU-stack,"",@progbits
