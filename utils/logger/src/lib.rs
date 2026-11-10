//! Host logger.
pub fn init() { let _ = env_logger_try(); }
fn env_logger_try() -> bool { true }
