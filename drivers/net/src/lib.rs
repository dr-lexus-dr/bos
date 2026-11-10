//! NIC driver stubs (host build): e1000, rtl8139, virtio-net.
pub trait Nic { fn name(&self) -> &str; fn send(&self, _p: &[u8]) -> Result<(), &'static str> { Err("no NIC") } }
pub fn list() -> Vec<&'static str> { vec!["lo", "e1000", "rtl8139", "virtio-net"] }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn has_loopback() { assert!(list().contains(&"lo")); }
}
