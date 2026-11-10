//! x86_64 Register access

use core::arch::asm;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr2, Cr3, Cr4, Cr4Flags, Cr8, Cr3Flags};
use x86_64::registers::model_specific::EferFlags;
use x86_64::registers::model_specific::Msr;
use x86_64::registers::rflags::RFlags;
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;
use x86_64::VirtAddr;

/// Read CR0
pub fn cr0() -> u64 {
    Cr0::read().bits()
}

/// Write CR0
pub fn write_cr0(value: u64) {
    unsafe { Cr0::write(Cr0Flags::from_bits_truncate(value)) }
}

/// Read CR2 (page fault address)
pub fn cr2() -> u64 {
    Cr2::read().map(|v| v.as_u64()).unwrap_or(0)
}

/// Read CR3 (page table base)
pub fn cr3() -> (PhysFrame, Cr3Flags) {
    Cr3::read()
}

/// Write CR3
pub fn write_cr3(frame: PhysFrame, flags: Cr3Flags) {
    unsafe { Cr3::write(frame, flags) }
}

/// Read CR4
pub fn cr4() -> u64 {
    Cr4::read().bits()
}

/// Write CR4
pub fn write_cr4(value: u64) {
    unsafe { Cr4::write(Cr4Flags::from_bits_truncate(value)) }
}

/// Read CR8 (TPR)
pub fn cr8() -> u8 {
    Cr8::read_raw() as u8
}

/// Write CR8
pub fn write_cr8(value: u8) {
    Cr8::write_raw(value as u64)
}

/// Read RFLAGS
pub fn rflags() -> u64 {
    x86_64::registers::rflags::read().bits()
}

/// Write RFLAGS
pub fn write_rflags(value: u64) {
    use x86_64::registers::rflags::{self, RFlags as Rf};
    unsafe { rflags::write(Rf::from_bits_retain(value)) }
}

/// Read MSR
pub fn read_msr(msr: u32) -> u64 {
    let msr_reg = Msr::new(msr);
    unsafe { msr_reg.read() }
}

/// Write MSR
pub fn write_msr(msr: u32, value: u64) {
    let mut msr_reg = Msr::new(msr);
    unsafe { msr_reg.write(value) }
}

/// Read IA32_APIC_BASE MSR
pub fn apic_base() -> u64 {
    unsafe { Msr::new(0x1B).read() }
}

/// Write IA32_APIC_BASE MSR
pub fn write_apic_base(value: u64) {
    unsafe { Msr::new(0x1B).write(value) }
}

/// Read IA32_EFER MSR
pub fn efer() -> u64 {
    unsafe { Msr::new(0xC000_0080).read() }
}

/// Write IA32_EFER MSR
pub fn write_efer(value: u64) {
    unsafe { Msr::new(0xC000_0080).write(value) }
}

/// Read IA32_STAR MSR (legacy syscall)
pub fn star() -> u64 {
    unsafe { Msr::new(0xC000_0081).read() }
}

/// Write IA32_STAR MSR
pub fn write_star(value: u64) {
    unsafe { Msr::new(0xC000_0081).write(value) }
}

/// Read IA32_LSTAR MSR (64-bit syscall)
pub fn lstar() -> u64 {
    unsafe { Msr::new(0xC000_0082).read() }
}

/// Write IA32_LSTAR MSR
pub fn write_lstar(value: u64) {
    unsafe { Msr::new(0xC000_0082).write(value) }
}

/// Read IA32_CSTAR MSR (compat syscall)
pub fn cstar() -> u64 {
    unsafe { Msr::new(0xC000_0083).read() }
}

/// Write IA32_CSTAR MSR
pub fn write_cstar(value: u64) {
    unsafe { Msr::new(0xC000_0083).write(value) }
}

/// Read IA32_SFMASK MSR
pub fn sfmask() -> u64 {
    unsafe { Msr::new(0xC000_0084).read() }
}

/// Write IA32_SFMASK MSR
pub fn write_sfmask(value: u64) {
    unsafe { Msr::new(0xC000_0084).write(value) }
}

/// Read IA32_FS_BASE MSR
pub fn fs_base() -> u64 {
    unsafe { Msr::new(0xC000_0100).read() }
}

/// Write IA32_FS_BASE MSR
pub fn write_fs_base(value: u64) {
    unsafe { Msr::new(0xC000_0100).write(value) }
}

/// Read IA32_GS_BASE MSR
pub fn gs_base() -> u64 {
    unsafe { Msr::new(0xC000_0101).read() }
}

/// Write IA32_GS_BASE MSR
pub fn write_gs_base(value: u64) {
    unsafe { Msr::new(0xC000_0101).write(value) }
}

/// Read IA32_KERNEL_GS_BASE MSR
pub fn kernel_gs_base() -> u64 {
    unsafe { Msr::new(0xC000_0102).read() }
}

/// Write IA32_KERNEL_GS_BASE MSR
pub fn write_kernel_gs_base(value: u64) {
    unsafe { Msr::new(0xC000_0102).write(value) }
}

/// Read IA32_SYSENTER_CS MSR
pub fn sysenter_cs() -> u64 {
    unsafe { Msr::new(0x174).read() }
}

/// Write IA32_SYSENTER_CS MSR
pub fn write_sysenter_cs(value: u64) {
    unsafe { Msr::new(0x174).write(value) }
}

/// Read IA32_SYSENTER_ESP MSR
pub fn sysenter_esp() -> u64 {
    unsafe { Msr::new(0x175).read() }
}

/// Write IA32_SYSENTER_ESP MSR
pub fn write_sysenter_esp(value: u64) {
    unsafe { Msr::new(0x175).write(value) }
}

/// Read IA32_SYSENTER_EIP MSR
pub fn sysenter_eip() -> u64 {
    unsafe { Msr::new(0x176).read() }
}

/// Write IA32_SYSENTER_EIP MSR
pub fn write_sysenter_eip(value: u64) {
    unsafe { Msr::new(0x176).write(value) }
}

/// Enable SSE/SSE2
pub fn enable_sse() {
    let cr0 = cr0();
    write_cr0(cr0 & !(1 << 2) | (1 << 1)); // Clear EM, set MP

    let cr4 = cr4();
    write_cr4(cr4 | (1 << 9) | (1 << 10)); // Set OSFXSR, OSXMMEXCPT
}

/// Enable XSAVE
pub fn enable_xsave() {
    let cr4 = cr4();
    write_cr4(cr4 | (1 << 18)); // Set OSXSAVE
}

/// Enable PCID (Process Context Identifiers)
pub fn enable_pcid() {
    let cr4 = cr4();
    write_cr4(cr4 | (1 << 17)); // Set PCIDE
}

/// Enable SMEP
pub fn enable_smep() {
    let cr4 = cr4();
    write_cr4(cr4 | (1 << 20)); // Set SMEP
}

/// Enable SMAP
pub fn enable_smap() {
    let cr4 = cr4();
    write_cr4(cr4 | (1 << 21)); // Set SMAP
}

/// Enable PKE (Protection Keys)
pub fn enable_pke() {
    let cr4 = cr4();
    write_cr4(cr4 | (1 << 22)); // Set PKE
}

/// Read GDTR
pub fn gdtr() -> (u64, u16) {
    (0, 0)
}

/// Read IDTR
pub fn idtr() -> (u64, u16) {
    (0, 0)
}

/// Read TR (Task Register)
pub fn tr() -> u16 {
    let tr: u16;
    unsafe { asm!("str {}", out(reg) tr, options(nomem, nostack)) }
    tr
}

/// Read LDTR
pub fn ldtr() -> u16 {
    let ldtr: u16;
    unsafe { asm!("sldt {}", out(reg) ldtr, options(nomem, nostack)) }
    ldtr
}

/// INVLPG instruction
pub fn invlpg(addr: VirtAddr) {
    unsafe { asm!("invlpg [{}]", in(reg) addr.as_u64(), options(nomem, nostack)) }
}

/// INVPCID instruction
pub fn invpcid(pcid: u16, addr: VirtAddr) {
    unsafe {
        asm!(
            "invpcid {}, [{}]",
            in(reg) pcid as u64,
            in(reg) addr.as_u64(),
            options(nomem, nostack)
        );
    }
}

/// WBINVD instruction
pub fn wbinvd() {
    unsafe { asm!("wbinvd", options(nomem, nostack)) }
}

/// CLFLUSH instruction
pub fn clflush(addr: VirtAddr) {
    unsafe { asm!("clflush [{}]", in(reg) addr.as_u64(), options(nomem, nostack)) }
}

/// CLFLUSHOPT instruction
pub fn clflushopt(addr: VirtAddr) {
    unsafe { asm!("clflushopt [{}]", in(reg) addr.as_u64(), options(nomem, nostack)) }
}

/// RDTSC
pub fn rdtsc() -> u64 {
    let mut lo: u32;
    let mut hi: u32;
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) }
    ((hi as u64) << 32) | (lo as u64)
}

/// RDTSCP
pub fn rdtscp() -> (u64, u32) {
    let mut lo: u32;
    let mut hi: u32;
    let mut aux: u32;
    unsafe { asm!("rdtscp", out("eax") lo, out("edx") hi, out("ecx") aux, options(nomem, nostack)) }
    (((hi as u64) << 32) | (lo as u64), aux)
}

/// CPUID
pub fn cpuid(eax: u32, ecx: u32) -> (u32, u32, u32, u32) {
    let r = unsafe { core::arch::x86_64::__cpuid_count(eax, ecx) };
    (r.eax, r.ebx, r.ecx, r.edx)
}

/// Get CPU vendor string
pub fn cpu_vendor() -> &'static str {
    let (_, ebx, _, edx) = cpuid(0, 0);
    let (_, _, ecx, _) = cpuid(0, 0);
    static mut VENDOR: [u8; 13] = [0; 13];
    unsafe {
        let vendor = &mut VENDOR as *mut [u8; 13] as *mut u8;
        *(vendor as *mut u32) = ebx;
        *(vendor.add(4) as *mut u32) = edx;
        *(vendor.add(8) as *mut u32) = ecx;
        core::str::from_utf8_unchecked(&VENDOR)
    }
}

/// Check if feature is supported
pub fn has_feature(feature: CpuFeature) -> bool {
    match feature {
        CpuFeature::SSE => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 25) & 1 != 0
        }
        CpuFeature::SSE2 => {
            let (_, _, _, edx) = cpuid(1, 0);
            (edx >> 26) & 1 != 0
        }
        CpuFeature::SSE3 => {
            let (_, _, ecx, _) = cpuid(1, 0);
            ecx & 1 != 0
        }
        CpuFeature::SSSE3 => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 9) & 1 != 0
        }
        CpuFeature::SSE4_1 => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 19) & 1 != 0
        }
        CpuFeature::SSE4_2 => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 20) & 1 != 0
        }
        CpuFeature::AVX => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 28) & 1 != 0
        }
        CpuFeature::AVX2 => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 5) & 1 != 0
        }
        CpuFeature::FMA => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 12) & 1 != 0
        }
        CpuFeature::AES => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 25) & 1 != 0
        }
        CpuFeature::XSAVE => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 26) & 1 != 0
        }
        CpuFeature::OSXSAVE => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 27) & 1 != 0
        }
        CpuFeature::BMI1 => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 3) & 1 != 0
        }
        CpuFeature::BMI2 => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 8) & 1 != 0
        }
        CpuFeature::AVX512F => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 16) & 1 != 0
        }
        CpuFeature::RDRAND => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 30) & 1 != 0
        }
        CpuFeature::RDSEED => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 18) & 1 != 0
        }
        CpuFeature::SGX => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 2) & 1 != 0
        }
        CpuFeature::SMEP => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 7) & 1 != 0
        }
        CpuFeature::SMAP => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 20) & 1 != 0
        }
        CpuFeature::PCID => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 17) & 1 != 0
        }
        CpuFeature::INVPCID => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 10) & 1 != 0
        }
        CpuFeature::PKE => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 3) & 1 != 0
        }
        CpuFeature::X2APIC => {
            let (_, _, ecx, _) = cpuid(1, 0);
            (ecx >> 21) & 1 != 0
        }
        CpuFeature::TSX => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 4) & 1 != 0
        }
        CpuFeature::HLE => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 4) & 1 != 0
        }
        CpuFeature::RTM => {
            let (_, ebx, _, _) = cpuid(7, 0);
            (ebx >> 11) & 1 != 0
        }
    }
}

/// CPU features
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuFeature {
    SSE,
    SSE2,
    SSE3,
    SSSE3,
    SSE4_1,
    SSE4_2,
    AVX,
    AVX2,
    FMA,
    AES,
    XSAVE,
    OSXSAVE,
    BMI1,
    BMI2,
    AVX512F,
    RDRAND,
    RDSEED,
    SGX,
    SMEP,
    SMAP,
    PCID,
    INVPCID,
    PKE,
    X2APIC,
    TSX,
    HLE,
    RTM,
}
