#![no_std]
#![no_main]
use libbos::*;
libbos::entry!(main);
fn check() -> Result<()> {
    if getpid() == 0 {
        return Err(Error(22));
    }
    if spawn(c"/applications/does-not-exist.bh", &[]) != Err(Error(2)) {
        return Err(Error(5));
    }
    if spawn(
        c"/applications/child-exit.bh",
        &[FdMapping {
            source: 63,
            target: 0,
        }],
    ) != Err(Error(9))
    {
        return Err(Error(5));
    }
    let child = spawn(c"/applications/child-exit.bh", &[])?;
    // A bad wait output pointer must not consume the child's exit status.
    if unsafe { syscall3(61, child, 1, 0) } != -14 {
        return Err(Error(5));
    }
    if waitpid(Some(child), false)? != (child, 23) {
        return Err(Error(5));
    }
    {
        let mut file = File::open(
            c"/users/default/documents/shared-offset.txt",
            READ_WRITE | CREATE | TRUNCATE,
        )?;
        file.write_all(b"abcd")?;
        file.seek(0, 0)?;
        dup2(file.as_raw_fd(), 20)?;
        let mut bytes = [0; 2];
        if read(20, &mut bytes)? != 2 || bytes != *b"ab" {
            return Err(Error(5));
        }
        if file.read(&mut bytes)? != 2 || bytes != *b"cd" {
            return Err(Error(5));
        }
        close(20)?;
    }
    // Reader starts first and blocks until the later writer produces data.
    let fds = pipe()?;
    let reader = spawn(
        c"/applications/pipe-reader.bh",
        &[FdMapping {
            source: fds[0],
            target: 0,
        }],
    )?;
    let writer = spawn(
        c"/applications/pipe-writer.bh",
        &[FdMapping {
            source: fds[1],
            target: 1,
        }],
    )?;
    close(fds[0] as u64)?;
    close(fds[1] as u64)?;
    if waitpid(Some(writer), false)? != (writer, 0) {
        return Err(Error(5));
    }
    if waitpid(Some(reader), false)? != (reader, 0) {
        return Err(Error(5));
    }
    if waitpid(Some(reader), false) != Err(Error(10)) {
        return Err(Error(5));
    }
    // EPIPE is returned after every read description closes.
    let fds = pipe()?;
    close(fds[0] as u64)?;
    if write(fds[1] as u64, b"broken") != Err(Error(32)) {
        return Err(Error(5));
    }
    close(fds[1] as u64)?;
    // Reuse zombie slots and frames, beyond the 64 process-table limit.
    for _ in 0..80 {
        let child = spawn(c"/applications/child-exit.bh", &[])?;
        if waitpid(Some(child), false)? != (child, 23) {
            return Err(Error(5));
        }
    }
    Ok(())
}
fn main() -> u8 {
    match check() {
        Ok(()) => {
            let _ = write_all(1, b"PROCESS PIPE TEST PASSED\n");
            0
        }
        Err(_) => {
            let _ = write_all(2, b"PROCESS PIPE TEST FAILED\n");
            1
        }
    }
}
