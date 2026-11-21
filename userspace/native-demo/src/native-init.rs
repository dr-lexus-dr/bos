#![no_std]
#![no_main]
libbos::entry!(main);
fn main() -> u8 {
    if libbos::getpid() != 1 {
        return 1;
    }
    let _ = libbos::write_all(1, b"init: PID 1 in ring 3\n");
    let shell=if libbos::File::open(c"/system/native",libbos::READ_ONLY).is_ok(){c"/applications/native-shell.bh"}else{c"/bin/nash"};
    loop {
        let child = match libbos::spawn(shell, &[]) {
            Ok(pid) => pid,
            Err(_) => return 2,
        };
        loop {
            match libbos::waitpid(None, false) {
                Ok((pid, _)) if pid == child => break,
                Ok(_) => {} // Reap adopted children while the shell is still alive.
                Err(_) => return 3,
            }
        }
        let _ = libbos::write_all(1, b"init: restarting shell\n");
    }
}
