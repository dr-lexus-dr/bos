.section .bss
.balign 8
saved_user_return_stack:
    .quad 0

.section .text
.global enter_user
enter_user:
    push %rbx
    push %rbp
    push %r12
    push %r13
    push %r14
    push %r15
    mov %rsp, saved_user_return_stack(%rip)

    mov ${user_ds}, %ax
    mov %ax, %ds
    mov %ax, %es
    jmp resume_user

.global leave_user
leave_user:
    cld
    mov $16, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %rdi, %rax
    mov saved_user_return_stack(%rip), %rsp
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    pop %rbx
    ret

 .global user_syscall
user_syscall:
    cld
    push %rax
    push %rbx
    push %rcx
    push %rdx
    push %rbp
    push %rsi
    push %rdi
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
    mov %rsp, %rdi
    and $-16, %rsp
    call switch_user_syscall
    mov %rax, %rdi
    jmp resume_user

.global user_timer
user_timer:
    cld
    push %rax
    push %rbx
    push %rcx
    push %rdx
    push %rbp
    push %rsi
    push %rdi
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
    mov %rsp, %rdi
    and $-16, %rsp
    call switch_user_timer
    mov %rax, %rdi
.global resume_user
resume_user:
    mov %rdi, %rsp
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %r11
    pop %r10
    pop %r9
    pop %r8
    pop %rdi
    pop %rsi
    pop %rbp
    pop %rdx
    pop %rcx
    pop %rbx
    pop %rax
    iretq

# BSP-only SYSCALL entry. FMASK disables IRQs before touching shared scratch.
.section .bss
.balign 16
linux_trap_stack:
    .skip {trap_bytes}
linux_trap_top:
linux_user_rsp:
    .quad 0
.section .text
.global linux_syscall
linux_syscall:
    mov %rsp, linux_user_rsp(%rip)
    lea linux_trap_top(%rip), %rsp
    push ${user_ds}
    push linux_user_rsp(%rip)
    push %r11
    push ${user_cs}
    push %rcx
    jmp user_syscall
