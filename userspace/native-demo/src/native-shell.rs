#![no_std]
#![no_main]
use core::ffi::CStr;
use libbos::*;
libbos::entry!(main);
fn output(bytes: &[u8]) {
    let _ = write_all(1, bytes);
}
fn path<'a>(text: &[u8], buffer: &'a mut [u8; 512]) -> Result<&'a CStr> {
    if text.is_empty() || text.len() >= buffer.len() || text.contains(&0) {
        return Err(Error(22));
    }
    buffer[..text.len()].copy_from_slice(text);
    buffer[text.len()] = 0;
    CStr::from_bytes_with_nul(&buffer[..text.len() + 1]).map_err(|_| Error(22))
}
fn trimmed(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    &bytes[start..end]
}
fn program(command: &[u8], input: Option<u32>, output_fd: Option<u32>) -> Result<u64> {
    let command = trimmed(command);
    let mut buffer = [0; 512];
    let mut file = None;
    let executable = if let Some(name) = command.strip_prefix(b"cat ") {
        let name = path(trimmed(name), &mut buffer)?;
        file = Some(File::open(name, READ_ONLY)?);
        c"/applications/native-cat.bh"
    } else if command == b"cat" {
        c"/applications/native-cat.bh"
    } else if command == b"wc" || command == b"wc -c" {
        c"/applications/native-wc.bh"
    } else {
        path(
            command.strip_prefix(b"run ").unwrap_or(command),
            &mut buffer,
        )?
    };
    let mut mappings = [FdMapping {
        source: 0,
        target: 0,
    }; 2];
    let mut count = 0;
    if let Some(fd) = file.as_ref().map(|f| f.as_raw_fd() as u32).or(input) {
        mappings[count] = FdMapping {
            source: fd,
            target: 0,
        };
        count += 1;
    }
    if let Some(fd) = output_fd {
        mappings[count] = FdMapping {
            source: fd,
            target: 1,
        };
        count += 1;
    }
    spawn(executable, &mappings[..count])
}
fn execute(line: &[u8]) -> Result<()> {
    if let Some(i) = line.iter().position(|b| *b == b'|') {
        if line[i + 1..].contains(&b'|') {
            return Err(Error(22));
        }
        let fds = pipe()?;
        let left = program(&line[..i], None, Some(fds[1]));
        let right = if left.is_ok() {
            program(&line[i + 1..], Some(fds[0]), None)
        } else {
            Err(Error(22))
        };
        let _ = close(fds[0] as u64);
        let _ = close(fds[1] as u64);
        // Reap every successfully spawned child even when the other side failed.
        let mut result = Ok(());
        for child in [left, right] {
            match child {
                Ok(pid) => {
                    if waitpid(Some(pid), false)?.1 != 0 {
                        result = Err(Error(5));
                    }
                }
                Err(e) => result = Err(e),
            }
        }
        result
    } else {
        let pid = program(line, None, None)?;
        if waitpid(Some(pid), false)?.1 == 0 {
            Ok(())
        } else {
            Err(Error(5))
        }
    }
}
fn main() -> u8 {
    output(b"native bSH: help for commands\n");
    loop {
        output(b"bSH> ");
        let mut line = [0; 512];
        let mut length = 0;
        loop {
            let mut byte = [0];
            match read(0, &mut byte) {
                Ok(0) | Err(_) => return 1,
                _ => {}
            }
            match byte[0] {
                b'\r' | b'\n' => {
                    output(b"\n");
                    break;
                }
                8 | 127 => {
                    if length > 0 {
                        length -= 1;
                        output(b"\x08 \x08");
                    }
                }
                b if b >= 32 && length < 511 => {
                    line[length] = b;
                    length += 1;
                    output(&[b]);
                }
                _ => {}
            }
        }
        let line = trimmed(&line[..length]);
        if line.is_empty() {
            continue;
        }
        if line == b"sync" { if sync().is_err() { output(b"sync failed\n"); } continue; }
        if line == b"exit" {
            return 0;
        }
        if line == b"help" {
            output(
                b"help, exit, sync, echo TEXT, cat FILE, cat FILE | wc -c, /applications/PROGRAM.bh\n",
            );
            continue;
        }
        if let Some(text) = line.strip_prefix(b"echo ") {
            output(text);
            output(b"\n");
            continue;
        }
        if execute(line).is_err() {
            output(b"bSH: command failed\n");
        }
    }
}
