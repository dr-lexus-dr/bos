//! PS/2 keyboard (polling + IRQ stub)
use x86_64::instructions::port::Port;
pub fn init() { crate::arch::x86_64::pic::unmask(1); }
pub fn handle_irq() { let mut p = Port::<u8>::new(0x60); let _sc: u8 = unsafe { p.read() }; }
pub fn try_read() -> Option<u8> {
    let mut st = Port::<u8>::new(0x64);
    unsafe { if st.read() & 1 == 0 { return None; } }
    let mut d = Port::<u8>::new(0x60);
    Some(unsafe { d.read() })
}
