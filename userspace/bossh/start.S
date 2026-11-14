.section .text
.global _start

# buffers
.section .bss
.lcomm linebuf, 256
.lcomm cwdbuf, 256

.section .text
_start:
loop:
    # print prompt "bsh$ " ... use cwd
    # getcwd(cwdbuf, 256)
    mov $79, %eax
    lea cwdbuf(%rip), %rdi
    mov $256, %rsi
    int $0x80
    # print cwdbuf
    mov $1, %eax
    mov $1, %edi
    lea cwdbuf(%rip), %rsi
    mov $256, %rdx
    # find len? just print 64
    mov $64, %rdx
    int $0x80
    # print "> "
    mov $1, %eax
    mov $1, %edi
    lea prompt(%rip), %rsi
    mov $2, %rdx
    int $0x80
    # read line from stdin
    mov $0, %eax
    mov $0, %edi
    lea linebuf(%rip), %rsi
    mov $255, %rdx
    int $0x80
    # if 0 bytes (EOF) -> exit
    cmp $0, %rax
    jle exit
    # null-terminate at rax
    lea linebuf(%rip), %rdi
    add %rax, %rdi
    movb $0, (%rdi)
    # builtin: exit/quit
    lea linebuf(%rip), %rsi
    # check first 4 chars "exit"
    movb (%rsi), %al
    cmp $'e', %al
    jne notexit
    movb 1(%rsi), %al
    cmp $'x', %al
    jne notexit
    movb 2(%rsi), %al
    cmp $'i', %al
    jne notexit
    movb 3(%rsi), %al
    cmp $'t', %al
    jne notexit
    jmp exit
notexit:
    # builtin: pwd -> getcwd already; just print newline
    # try exec linebuf
    mov $59, %eax
    lea linebuf(%rip), %rdi
    int $0x80
    # print newline
    mov $1, %eax
    mov $1, %edi
    lea nl(%rip), %rsi
    mov $1, %rdx
    int $0x80
    jmp loop
exit:
    mov $60, %eax
    xor %edi, %edi
    int $0x80
    ud2

.section .rodata
prompt:
    .ascii "> "
nl:
    .ascii "\n"
