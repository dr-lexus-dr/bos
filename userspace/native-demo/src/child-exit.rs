#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    if libbos::getppid() == 0 {
        99
    } else {
        23
    }
}
