//! Network stack stub: loopback + e1000/rtl8139 hooks
use log::info;
pub fn init() { info!("net stack init (stub): lo only"); }
pub fn handle_rx(_iface: &str, _pkt: &[u8]) {}
pub fn send(_iface: &str, _pkt: &[u8]) -> Result<(), &'static str> { Err("no NIC") }
pub fn ifconfig() -> &'static str { "lo 127.0.0.1/8 UP" }
