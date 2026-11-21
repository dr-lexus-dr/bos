#![no_std]
#![no_main]
use core::ffi::CStr;
use libbos::*;
libbos::entry!(main);
mod shell_line;
use shell_line::{Decoder, History, Key, Line, COMMANDS};
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
fn prompt(failed: bool) {
    output(b"\x1b[96mbSH \x1b[94m~ ");
    output(if failed { b"\x1b[91m% " } else { b"\x1b[92m% " });
    output(b"\x1b[0m");
}
fn repeat(byte: u8, count: usize) {
    let bytes = [byte; 64];
    let mut remaining = count;
    while remaining > 0 {
        let n = remaining.min(bytes.len());
        output(&bytes[..n]);
        remaining -= n;
    }
}
fn redraw(line: &Line, old_len: usize, old_cursor: usize) {
    // Keep the editing viewport within an 80-column terminal, even for long commands.
    const WIDTH: usize = 64;
    let old_start = old_cursor.saturating_sub(WIDTH);
    let old_width = (old_len - old_start).min(WIDTH);
    repeat(8, old_cursor - old_start);
    let start = line.cursor.saturating_sub(WIDTH);
    let end = (start + WIDTH).min(line.len);
    let visible = &line.text()[start..end];
    let command_end = line
        .text()
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(line.len);
    let colored = command_end.saturating_sub(start).min(visible.len());
    output(b"\x1b[96m");
    output(&visible[..colored]);
    output(b"\x1b[0m");
    output(&visible[colored..]);
    let width = old_width.max(visible.len());
    repeat(b' ', width - visible.len());
    repeat(8, width - (line.cursor - start));
}
fn read_line(history: &mut History, decoder: &mut Decoder, failed: bool) -> Option<Line> {
    let mut line = Line::new();
    loop {
        let mut byte = [0];
        if !matches!(read(0, &mut byte), Ok(1)) {
            return None;
        }
        let Some(key) = decoder.feed(byte[0]) else {
            continue;
        };
        let old_len = line.len;
        let old_cursor = line.cursor;
        match key {
            Key::Enter => {
                output(b"\x1b[0m\n");
                return Some(line);
            }
            Key::Control(4) if line.len == 0 => return None,
            Key::Control(4) | Key::Delete => line.delete(),
            Key::Text(byte) => line.insert(byte),
            Key::Backspace => line.backspace(),
            Key::Left | Key::Control(2) => line.cursor = line.cursor.saturating_sub(1),
            Key::Right | Key::Control(6) => line.cursor = (line.cursor + 1).min(line.len),
            Key::Home | Key::Control(1) => line.cursor = 0,
            Key::End | Key::Control(5) => line.cursor = line.len,
            Key::Control(21) => line.kill_before(),
            Key::Control(11) => line.kill_after(),
            Key::Control(23) => line.kill_word(),
            Key::Up | Key::Control(16) => history.navigate(&mut line, true),
            Key::Down | Key::Control(14) => history.navigate(&mut line, false),
            Key::Tab => {
                let before = line;
                if line.complete() > 1 {
                    output(b"\n\x1b[90m");
                    for &candidate in COMMANDS {
                        if candidate.starts_with(before.text()) {
                            output(candidate);
                            output(b"  ");
                        }
                    }
                    output(b"\x1b[0m\n");
                    prompt(failed);
                    redraw(&line, 0, 0);
                    continue;
                }
            }
            _ => continue,
        }
        redraw(&line, old_len, old_cursor);
    }
}
fn main() -> u8 {
    output(b"\x1b[96mbSH\x1b[0m / bOS\n\x1b[90mTab completes  |  Up/Down history  |  help for commands\x1b[0m\n");
    let mut history = History::new();
    let mut decoder = Decoder::default();
    let mut failed = false;
    loop {
        prompt(failed);
        let Some(input) = read_line(&mut history, &mut decoder, failed) else {
            output(b"\n");
            return 0;
        };
        let line = trimmed(input.text());
        if line.is_empty() {
            continue;
        }
        history.record(&input);
        failed = false;
        if line == b"sync" {
            if sync().is_err() {
                output(b"\x1b[91mbSH: sync failed\x1b[0m\n");
                failed = true;
            }
            continue;
        }
        if line == b"exit" {
            return 0;
        }
        if line == b"help" {
            output(b"\x1b[96mbSH commands\x1b[0m\nhelp, exit, sync, echo TEXT, cat FILE, cat FILE | wc -c, /applications/PROGRAM.bh\n\x1b[90mEdit: Left/Right, Home/End, Delete, Ctrl-A/E, Ctrl-U/K/W\nHistory: Up/Down searches by prefix; Tab completes commands.\x1b[0m\n");
            continue;
        }
        if line == b"echo" {
            output(b"\n");
            continue;
        }
        if let Some(text) = line.strip_prefix(b"echo ") {
            output(text);
            output(b"\n");
            continue;
        }
        if execute(line).is_err() {
            output(b"\x1b[91mbSH: command failed\x1b[0m\n");
            failed = true;
        }
    }
}
