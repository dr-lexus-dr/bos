//! Host-side PE inspect helper.
pub fn is_pe(d: &[u8]) -> bool { d.len() > 2 && d[0] == b'M' && d[1] == b'Z' }
pub fn win32_to_bos(_dll: &str, _f: &str) -> Option<u64> { None }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detect() {
        assert!(is_pe(&[b'M', b'Z', 0]));
        assert!(!is_pe(&[0x7F, b'E', b'L', b'F']));
    }
}
