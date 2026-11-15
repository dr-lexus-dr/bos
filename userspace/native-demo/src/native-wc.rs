#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    let mut bytes = [0; 1024];
    let mut count = 0usize;
    loop {
        match libbos::read(0, &mut bytes) {
            Ok(0) => break,
            Ok(n) => count += n,
            Err(_) => return 1,
        }
    }
    let mut output = [0; 32];
    let mut index = 31;
    output[index] = b'\n';
    loop {
        index -= 1;
        output[index] = b'0' + (count % 10) as u8;
        count /= 10;
        if count == 0 {
            break;
        }
    }
    if libbos::write_all(1, &output[index..]).is_err() {
        1
    } else {
        0
    }
}
