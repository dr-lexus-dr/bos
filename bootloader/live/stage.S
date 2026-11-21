/* Read the streamed kernel into high memory and decompress at its final address. */
.section .text.start,"ax"
.code16
.global start
start:
    mov %dl, drive
    in $0x92, %al
    or $2, %al
    and $0xfe, %al
    out %al, $0x92

    /* Find usable RAM containing the kernel; cap even a large machine. */
    xor %ebx, %ebx
map_next:
    mov $0x6200, %di
    mov $0xe820, %eax
    mov $0x534d4150, %edx
    mov $20, %ecx
    int $0x15
    jc map_done
    cmp $0x534d4150, %eax
    jne bad
    cmpl $1, 0x6210
    jne map_skip
    cmpl $0, 0x6204
    jne map_skip
    movl 0x6200, %eax
    cmp $0x100000, %eax
    ja map_skip
    addl 0x6208, %eax
    jc map_cap
    cmpl $0, 0x620c
    jne map_cap
    cmp $0x40000000, %eax
    jbe map_save
map_cap:
    mov $0x40000000, %eax
map_save:
    cmp ram_end, %eax
    jbe map_skip
    mov %eax, ram_end
map_skip:
    test %ebx, %ebx
    jnz map_next
map_done:
    cmpl $(KERNEL_END + PAYLOAD_SIZE + 24*1024*1024), ram_end
    jb bad

    /* Find a VBE direct-colour linear framebuffer (32-bit BGR or RGB). */
    mov $0x6200, %di
    movl $0x32454256, (%di)
    mov $0x4f00, %ax
    int $0x10
    cmp $0x004f, %ax
    jne video_done
    mov 0x6210, %ax
    mov %ax, %fs
    mov 0x620e, %si
    mov %si, first_mode
video_next:
    mov %fs:(%si), %cx
    add $2, %si
    cmp $0xffff, %cx
    je video_fallback
    push %si
    push %cx
    mov $0x6000, %di
    mov $0x4f01, %ax
    int $0x10
    pop %cx
    pop %si
    cmp $0x004f, %ax
    jne video_next
    mov 0x6000, %ax
    and $0x91, %ax
    cmp $0x91, %ax
    jne video_next
    movl 0x6012, %eax
    cmpl resolution, %eax
    jne video_next
    cmpb $32, 0x6019
    jne video_next
    cmpb $6, 0x601b
    jne video_next
    mov %cx, %bx
    or $0x4000, %bx
    mov $0x4f02, %ax
    int $0x10
    cmp $0x004f, %ax
    jne video_next
    movl $0x6000, video_info
    jmp video_done
video_fallback:
    movl resolution, %eax
    movl $(768 << 16 | 1024), %ecx
    cmpl $(800 << 16 | 1280), %eax
    je use_mode
    movl $(600 << 16 | 800), %ecx
    cmpl $(768 << 16 | 1024), %eax
    je use_mode
    movl $(480 << 16 | 640), %ecx
    cmpl $(600 << 16 | 800), %eax
    jne video_done
use_mode:
    movl %ecx, resolution
    mov first_mode, %si
    jmp video_next
video_done:
    xor %ax, %ax
    mov %ax, %ds
    mov %ax, %es
block:
    movl remaining, %eax
    test %eax, %eax
    jz enter_pm
    cmpl $64, %eax
    jbe read_chunk
    mov $64, %eax
read_chunk:
    mov %ax, count
    call read_disk
enter_pm:
    cli
    lgdt gdtr
    mov %cr0, %eax
    or $1, %al
    mov %eax, %cr0
    ljmpl $8, $protected
read_disk:
    mov count, %ax
    mov %ax, requested
    movb $3, tries
read_again:
    mov requested, %ax           /* EDD may overwrite the count on failure. */
    mov %ax, count
    mov $packet, %si
    mov drive, %dl
    mov $0x42, %ah
    int $0x13
    jnc read_ok
    decb tries
    jz bad
    xor %ax, %ax
    mov drive, %dl
    int $0x13
    jmp read_again
read_ok:
    movzwl requested, %eax
    add %eax, lba
    ret
bad:
    mov $error, %si
1:  lodsb
    test %al, %al
    jz stop
    mov $0x0e, %ah
    mov $7, %bx
    int $0x10
    jmp 1b
stop: cli
    hlt
    jmp stop

.code32
protected:
    mov $16, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov $0x7c00, %esp
    cld
    cmpl $0, remaining
    je loaded
    movzwl requested, %ecx
    sub %ecx, remaining
    shl $7, %ecx                /* sectors to dwords */
    mov $0x10000, %esi
    mov destination, %edi
    rep movsl
    mov %edi, destination
    ljmp $24, $pm16
pm_fail:
    movb $1, failed
    ljmp $24, $pm16
.code16
pm16:
    mov $32, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov %cr0, %eax
    and $0xfe, %al
    mov %eax, %cr0
    ljmp $0, $resume
resume:
    xor %ax, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    sti
    cmpb $0, failed
    jne bad
    jmp block

.code32
loaded:
    push $0x40000               /* probability table scratch */
    push $(FILE_END-0x100000)
    push $0x100000
    push $PACKED_SIZE
    push $KERNEL_END
    call unpack
    add $20, %esp
    test %eax, %eax
    jz pm_fail
    mov $FILE_END, %edi
    mov $KERNEL_END, %ecx
    sub %edi, %ecx
    xor %eax, %eax
    rep stosb
    /* Map 4 GiB with 2 MiB pages, including PCI video RAM. */
    mov $0x20000, %edi
    mov $6144, %ecx
    rep stosl
    movl $0x21003, 0x20000
    mov $0x21000, %edi
    mov $0x22003, %eax
    mov $4, %ecx
2:  stosl
    add $4, %edi
    add $4096, %eax
    loop 2b
    mov $0x22000, %edi
    mov $0x83, %eax
    mov $2048, %ecx
3:  stosl
    add $4, %edi
    add $0x200000, %eax
    loop 3b
    mov %cr4, %eax
    or $0x620, %eax             /* PAE, OSFXSR, OSXMMEXCPT */
    mov %eax, %cr4
    mov $0x20000, %eax
    mov %eax, %cr3
    mov $0xc0000080, %ecx
    rdmsr
    or $0x900, %eax             /* LME, NXE */
    wrmsr
    mov %cr0, %eax
    and $0xfffffff3, %eax
    or $0x80000003, %eax
    mov %eax, %cr0
    ljmp $40, $longmode
.code64
longmode:
    mov ram_end(%rip), %edi
    mov video_info(%rip), %ebx
    xor %esi, %esi
    xor %edx, %edx
    xor %ecx, %ecx
    test %ebx, %ebx
    jz 4f
    mov 40(%rbx), %esi
    mov 18(%rbx), %edx          /* width | height << 16 */
    movzwl 16(%rbx), %ecx
    shr $2, %ecx                /* pixels per scanline */
    cmpb $0, 32(%rbx)
    jne 4f
    bts $31, %ecx               /* RGB byte order */
4:
    mov $((KERNEL_END+PAYLOAD_SIZE+4095) & -4096), %r8d
    mov $(KERNEL_END+PACKED_SIZE), %r9d
    movabs $0x100000, %rax
    jmp *%rax
.balign 8
gdt: .quad 0, 0x00cf9a000000ffff, 0x00cf92000000ffff
     .quad 0x00009a000000ffff, 0x000092000000ffff, 0x00af9a000000ffff
gdtr: .word . - gdt - 1
      .long gdt
error: .asciz "bOS Live: disk, memory or image error"
drive: .byte 0
failed: .byte 0
tries: .byte 0
.balign 2
requested: .word 0
.balign 4
ram_end: .long 0
video_info: .long 0
resolution: .long 800 << 16 | 1280
first_mode: .word 0
.balign 4
destination: .long KERNEL_END
remaining: .long (PAYLOAD_SIZE + 511) / 512
packet: .byte 16, 0
count: .word 1
offset: .word 0
    .word 0x1000
lba: .quad PAYLOAD_LBA
