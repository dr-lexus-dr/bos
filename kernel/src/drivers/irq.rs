//! Generic IRQ registry
use alloc::vec::Vec;
use spin::Mutex;
type IrqHandler = fn();
static HANDLERS: Mutex<Vec<Option<IrqHandler>>> = Mutex::new(Vec::new());
pub fn init() {
    let mut h = HANDLERS.lock();
    h.clear(); h.resize(16, None);
}
pub fn register(irq: u8, h: IrqHandler) {
    let mut t = HANDLERS.lock();
    if (irq as usize) < t.len() { t[irq as usize] = Some(h); }
    crate::arch::x86_64::pic::unmask(irq);
}
pub fn handle_irq(irq: u8) {
    if let Some(h) = HANDLERS.lock().get(irq as usize).and_then(|x| *x) { h(); }
}
