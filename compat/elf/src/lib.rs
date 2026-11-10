//! Host-side ELF inspect helper.
pub fn is_elf(d: &[u8]) -> bool { d.len() > 4 && d[..4] == [0x7F, b'E', b'L', b'F'] }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detect() {
        assert!(is_elf(&[0x7F, b'E', b'L', b'F', 2, 1]));
        assert!(!is_elf(&[b'M', b'Z', 0, 0]));
        assert!(!is_elf(&[]));
    }
}
