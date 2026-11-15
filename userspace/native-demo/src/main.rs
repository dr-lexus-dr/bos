#![no_std]
#![no_main]
use libbos::{File, Result, CREATE, READ_WRITE, TRUNCATE};

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "xor rbp, rbp",
    "call native_main",
    "ud2"
);

fn run() -> Result<()> {
    let mut file = File::open(
        c"/users/default/documents/native.txt",
        READ_WRITE | CREATE | TRUNCATE,
    )?;
    file.write_all(b"native Rust file I/O\n")?;
    file.sync()?;
    file.seek(0, 0)?;
    let mut buffer = [0; 64];
    let count = file.read(&mut buffer)?;
    if &buffer[..count] != b"native Rust file I/O\n" {
        return Err(libbos::Error(5));
    }
    libbos::write_all(1, &buffer[..count])?;
    Ok(())
}

#[no_mangle]
extern "C" fn native_main() -> ! {
    match run() {
        Ok(()) => {
            let _ = libbos::write_all(1, b"libbos native test passed\n");
            libbos::exit(0)
        }
        Err(_) => {
            let _ = libbos::write_all(2, b"libbos native test FAILED\n");
            libbos::exit(1)
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libbos::exit(101)
}
