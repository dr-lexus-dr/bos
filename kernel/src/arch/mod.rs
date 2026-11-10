//! Arch hub (x86_64 only for now)
pub mod x86_64;
pub use x86_64::{halt, halt_all, enable_interrupts, disable_interrupts, wait_for_interrupt};
