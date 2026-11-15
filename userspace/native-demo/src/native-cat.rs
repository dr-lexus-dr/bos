#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    let mut bytes = [0; 1024];
    loop {
        match libbos::read(0, &mut bytes) {
            Ok(0) => return 0,
            Ok(n) => {
                if libbos::write_all(1, &bytes[..n]).is_err() {
                    return 1;
                }
            }
            Err(_) => return 1,
        }
    }
}
