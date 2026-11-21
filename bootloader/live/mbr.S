/* Raw removable disk, BIOS EDD. Stage two occupies the following 48 sectors. */
.code16
.global _start
_start:
    ljmp $0, $real
real:
    xor %ax, %ax
    mov %ax, %ds
    mov %ax, %es
    cli
    mov %ax, %ss
    mov $0x7c00, %sp
    sti
    cld
    mov %dl, drive
read_stage:
    movw $48, packet+2
    mov $packet, %si
    mov drive, %dl
    mov $0x42, %ah
    int $0x13
    jnc loaded
    decb tries
    jz fail
    xor %ax, %ax
    mov drive, %dl
    int $0x13
    jmp read_stage
loaded:
    mov drive, %dl
    ljmp $0, $0x8000
fail:
    mov $error, %si
1:  lodsb
    test %al, %al
    jz 2f
    mov $0x0e, %ah
    mov $7, %bx
    int $0x10
    jmp 1b
2:  cli
    hlt
    jmp 2b
error: .asciz "bOS: BIOS disk read failed"
drive: .byte 0
tries: .byte 3
.balign 4
packet: .byte 16, 0
    .word 48, 0x8000, 0
    .quad 1
.org 510
.word 0xaa55
