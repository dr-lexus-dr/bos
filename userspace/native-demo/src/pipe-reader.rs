#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    let mut total = 0;
    let mut buffer = [0; 257];
    loop {
        match libbos::read(0, &mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if buffer[..n].iter().any(|b| *b != b'x') {
                    return 2;
                }
                total += n;
            }
            Err(_) => return 1,
        }
    }
    if total != 8192 {
        return 3;
    }
    let _ = libbos::write_all(1, b"PIPE READ 8192\n");
    0
}
