//! Host harness for the actual kernel frame allocator (no privileged instructions).
extern crate alloc;
mod arch {
    pub mod x86_64 {
        pub mod paging {
            pub static PHYS_MEM_OFFSET: core::sync::atomic::AtomicU64 =
                core::sync::atomic::AtomicU64::new(0);
        }
    }
}
#[path = "../../../kernel/src/mm/phys.rs"]
pub mod phys;

mod mm {
    pub mod paging {
        pub struct AddressSpace;
    }
}
#[path = "../../../kernel/src/compat/elf.rs"]
mod elf;

#[cfg(test)]
mod elf_tests {
    use super::elf::Executable;
    fn image() -> Vec<u8> {
        let mut image = vec![0; 4097];
        image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&62u16.to_le_bytes());
        image[20..24].copy_from_slice(&1u32.to_le_bytes());
        image[24..32].copy_from_slice(&0x8000000000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[52..54].copy_from_slice(&64u16.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&4096u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x8000000000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&4096u64.to_le_bytes());
        image[112..120].copy_from_slice(&4096u64.to_le_bytes());
        image[4096] = 0xc3;
        image
    }
    #[test]
    fn valid_static_image() {
        let bytes = image();
        let elf = Executable::parse(&bytes).unwrap();
        let segment = elf.segments().next().unwrap().unwrap();
        assert_eq!(segment.bytes, &[0xc3]);
        assert_eq!(segment.size, 4096);
        assert!(segment.executable && !segment.writable);
    }
    #[test]
    fn rejects_bad_permissions_alignment_overflow_and_dynamic_linker() {
        for (offset, bytes) in [
            (68, 7u64.to_le_bytes()),
            (112, 3u64.to_le_bytes()),
            (104, u64::MAX.to_le_bytes()),
            (96, 8192u64.to_le_bytes()),
            (64, 3u64.to_le_bytes()),
        ] {
            let mut bytes_image = image();
            bytes_image[offset..offset + 8].copy_from_slice(&bytes);
            let elf = Executable::parse(&bytes_image).unwrap();
            assert!(elf.segments().next().unwrap().is_err(), "offset {offset}");
        }
    }
    #[test]
    fn all_truncations_and_deterministic_mutations_do_not_panic() {
        let bytes = image();
        for length in 0..bytes.len() {
            if let Ok(elf) = Executable::parse(&bytes[..length]) {
                for segment in elf.segments() {
                    assert!(segment.is_err());
                }
            }
        }
        let mut state = 17u32;
        for _ in 0..10000 {
            let mut bytes = image();
            for _ in 0..4 {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                bytes[state as usize % 120] = (state >> 16) as u8;
            }
            if let Ok(elf) = Executable::parse(&bytes) {
                for _ in elf.segments() {}
            }
        }
    }
}
