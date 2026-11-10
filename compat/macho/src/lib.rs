//! Host-side Mach-O inspect helper.
pub fn is_macho(d: &[u8]) -> bool {
    d.len() >= 4 && (u32::from_le_bytes([d[0],d[1],d[2],d[3]]) == 0xfeed_facf)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detect() {
        assert!(is_macho(&[0xCF, 0xFA, 0xED, 0xFE]));
        assert!(!is_macho(&[0x7F, b'E', b'L', b'F']));
        assert!(!is_macho(&[]));
    }
}
