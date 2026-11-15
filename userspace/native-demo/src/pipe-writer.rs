#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    for _ in 0..2 {
        if libbos::write_all(1, &[b'x'; 4096]).is_err() {
            return 1;
        }
    }
    0
}
