//! Shared line editing for COM1 and the PS/2 console.

use alloc::{string::String, vec::Vec};
use spin::Mutex;

use super::{print_str, COMMANDS};
use crate::arch::x86_64::serial;
use crate::drivers::{console, keyboard};
use crate::fs::{FileType, VFS};

const MAX_LINE: usize = 1024;
const HISTORY_LIMIT: usize = 64;

pub(super) struct History(Vec<String>);

impl History {
    pub(super) fn new() -> Self {
        Self(Vec::new())
    }

    pub(super) fn record(&mut self, line: &str) {
        if line.trim().is_empty() || self.0.last().is_some_and(|last| last == line) {
            return;
        }
        if self.0.len() == HISTORY_LIMIT {
            self.0.remove(0);
        }
        self.0.push(String::from(line));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Text(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Home,
    End,
    PageUp,
    PageDown,
    Control(u8),
}

#[derive(Clone, Copy)]
enum Escape {
    None,
    Start,
    Ss3,
    Csi { param: u8, modified: bool },
}

struct Input {
    escape: Escape,
    after_cr: bool,
    extended: bool,
    shifts: u8,
    controls: u8,
    utf8: [u8; 4],
    utf8_length: usize,
    utf8_expected: usize,
}

static INPUT: Mutex<Input> = Mutex::new(Input {
    escape: Escape::None,
    after_cr: false,
    extended: false,
    shifts: 0,
    controls: 0,
    utf8: [0; 4],
    utf8_length: 0,
    utf8_expected: 0,
});

pub(super) fn key() -> Key {
    INPUT.lock().next_key(true)
}

impl Input {
    fn next_key(&mut self, unicode: bool) -> Key {
        loop {
            crate::drivers::net::poll();
            if let Some(byte) = serial::try_read_byte() {
                if let Some(key) = self.serial(byte, unicode) {
                    return key;
                }
            }
            if let Some(scancode) = keyboard::try_read() {
                if let Some(key) = self.ps2(scancode) {
                    return key;
                }
            }
            core::hint::spin_loop();
        }
    }

    fn serial(&mut self, byte: u8, unicode: bool) -> Option<Key> {
        if unicode && byte >= 0x80 {
            if self.utf8_length == 0 {
                self.utf8_expected = match byte {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => return None,
                };
            } else if byte & 0xc0 != 0x80 {
                self.utf8_length = 0;
                self.utf8_expected = 0;
                return None;
            }
            self.utf8[self.utf8_length] = byte;
            self.utf8_length += 1;
            if self.utf8_length == self.utf8_expected {
                self.utf8_length = 0;
                return core::str::from_utf8(&self.utf8[..self.utf8_expected])
                    .ok()?
                    .chars()
                    .next()
                    .map(Key::Text);
            }
            return None;
        }
        self.utf8_length = 0;
        if byte == 0x1b {
            self.escape = Escape::Start;
            self.after_cr = false;
            return None;
        }
        match core::mem::replace(&mut self.escape, Escape::None) {
            Escape::Start if byte == b'[' => {
                self.escape = Escape::Csi {
                    param: 0,
                    modified: false,
                };
                return None;
            }
            Escape::Start if byte == b'O' => {
                self.escape = Escape::Ss3;
                return None;
            }
            Escape::Ss3 => return arrow(byte),
            Escape::Csi { param, modified } => match byte {
                b'~' => {
                    return match param {
                        1 | 7 => Some(Key::Home),
                        3 => Some(Key::Delete),
                        4 | 8 => Some(Key::End),
                        5 => Some(Key::PageUp),
                        6 => Some(Key::PageDown),
                        _ => None,
                    }
                }
                0x40..=0x7e => return arrow(byte),
                b'0'..=b'9' if !modified => {
                    self.escape = Escape::Csi {
                        param: param.saturating_mul(10).saturating_add(byte - b'0'),
                        modified,
                    };
                    return None;
                }
                0x20..=0x3f => {
                    self.escape = Escape::Csi {
                        param,
                        modified: true,
                    };
                    return None;
                }
                _ => {}
            },
            _ => {}
        }
        let after_cr = core::mem::replace(&mut self.after_cr, byte == b'\r');
        match byte {
            b'\n' if after_cr => None,
            b'\r' | b'\n' => Some(Key::Enter),
            b'\t' => Some(Key::Tab),
            0x08 | 0x7f => Some(Key::Backspace),
            1..=26 => Some(Key::Control(b'a' + byte - 1)),
            0x20..=0x7e => Some(Key::Text(byte as char)),
            _ => None,
        }
    }

    fn ps2(&mut self, code: u8) -> Option<Key> {
        if code == 0xe0 {
            self.extended = true;
            return None;
        }
        if core::mem::take(&mut self.extended) {
            return match code {
                0x1d => {
                    self.controls |= 2;
                    None
                }
                0x9d => {
                    self.controls &= !2;
                    None
                }
                0x47 => Some(Key::Home),
                0x4f => Some(Key::End),
                0x49 => Some(Key::PageUp),
                0x51 => Some(Key::PageDown),
                0x48 => Some(Key::Up),
                0x50 => Some(Key::Down),
                0x4b => Some(Key::Left),
                0x4d => Some(Key::Right),
                0x53 => Some(Key::Delete),
                0x1c => Some(Key::Enter),
                _ => None,
            };
        }
        match code {
            0x1d => self.controls |= 1,
            0x9d => self.controls &= !1,
            0x2a => self.shifts |= 1,
            0x36 => self.shifts |= 2,
            0xaa => self.shifts &= !1,
            0xb6 => self.shifts &= !2,
            0x1c => return Some(Key::Enter),
            0x0e => return Some(Key::Backspace),
            0x0f => return Some(Key::Tab),
            _ => {
                return decode(code, self.shifts != 0).map(|ch| {
                    if self.controls != 0 && ch.is_ascii_alphabetic() {
                        Key::Control(ch.to_ascii_lowercase() as u8)
                    } else {
                        Key::Text(ch)
                    }
                })
            }
        }
        None
    }
}

fn arrow(byte: u8) -> Option<Key> {
    match byte {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    }
}

pub(super) fn read(prompt: &str, history: Option<&History>) -> String {
    let mut input = INPUT.lock();
    let mut line = String::new();
    let mut cursor = 0;
    let commands = history.map_or(&[][..], |history| history.0.as_slice());
    let mut position = commands.len();
    let mut draft = (String::new(), 0);
    console::print_colored(prompt, console::Color::Green);
    let _input_color = console::ColorGuard::new(console::Color::White);

    loop {
        console::toggle_input_cursor();
        let key = input.next_key(false);
        console::toggle_input_cursor();
        match key {
            Key::Enter => {
                print_str(&line[cursor..]);
                print_str("\r\n");
                return line;
            }
            Key::Left if cursor > 0 => {
                back(1);
                cursor -= 1;
            }
            Key::Right if cursor < line.len() => {
                print_str(&line[cursor..cursor + 1]);
                cursor += 1;
            }
            Key::Text(ch) if line.len() < MAX_LINE => {
                line.insert(cursor, ch);
                redraw_tail(&line, cursor, cursor + 1, 0);
                cursor += 1;
            }
            Key::Backspace if cursor > 0 => {
                back(1);
                cursor -= 1;
                line.remove(cursor);
                redraw_tail(&line, cursor, cursor, 1);
            }
            Key::Delete if cursor < line.len() => {
                line.remove(cursor);
                redraw_tail(&line, cursor, cursor, 1);
            }
            Key::Up if position > 0 => {
                if position == commands.len() {
                    draft = (line.clone(), cursor);
                }
                position -= 1;
                replace(
                    &mut line,
                    &mut cursor,
                    &commands[position],
                    commands[position].len(),
                );
            }
            Key::Down if position < commands.len() => {
                position += 1;
                let (next, next_cursor) = if let Some(command) = commands.get(position) {
                    (command.as_str(), command.len())
                } else {
                    (draft.0.as_str(), draft.1)
                };
                replace(&mut line, &mut cursor, next, next_cursor);
            }
            Key::Tab if history.is_some() => complete(prompt, &mut line, &mut cursor),
            Key::Tab if line.len() <= MAX_LINE - 4 => {
                line.insert_str(cursor, "    ");
                redraw_tail(&line, cursor, cursor + 4, 0);
                cursor += 4;
            }
            _ => {}
        }
    }
}

fn back(count: usize) {
    for _ in 0..count {
        print_str("\x08");
    }
}

fn redraw_tail(line: &str, start: usize, cursor: usize, erase: usize) {
    print_str(&line[start..]);
    for _ in 0..erase {
        print_str(" ");
    }
    back(line.len() - cursor + erase);
}

fn replace(line: &mut String, cursor: &mut usize, replacement: &str, next: usize) {
    let erase = line.len().saturating_sub(replacement.len());
    back(*cursor);
    redraw_tail(replacement, 0, next, erase);
    line.clear();
    line.push_str(replacement);
    *cursor = next;
}

fn separator(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '<' | '>')
}

/// Return the raw word's start and its decoded prefix, including unfinished quotes.
fn word_prefix(line: &str) -> Option<(usize, String)> {
    let mut start = 0;
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            word.push(ch);
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else {
                word.push(ch);
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if separator(ch) {
            start = index + 1;
            word.clear();
        } else {
            word.push(ch);
        }
    }
    (!escaped).then_some((start, word))
}

fn complete(prompt: &str, line: &mut String, cursor: &mut usize) {
    if line[*cursor..]
        .chars()
        .next()
        .is_some_and(|ch| !separator(ch))
    {
        return;
    }
    let Some((start, word)) = word_prefix(&line[..*cursor]) else {
        return;
    };
    let mut matches: Vec<String> = if line[..start].trim().is_empty() && !word.contains('/') {
        COMMANDS
            .iter()
            .map(|(command, _)| *command)
            .filter(|command| command.starts_with(&word))
            .map(String::from)
            .collect()
    } else {
        let split = word.rfind('/').map_or(0, |index| index + 1);
        let (base, prefix) = word.split_at(split);
        let directory = if base.is_empty() { "." } else { base };
        let Ok(entries) = VFS.lock().list_dir(directory) else {
            return;
        };
        entries
            .into_iter()
            .filter(|entry| {
                entry.name.starts_with(prefix)
                    && !matches!(entry.name.as_str(), "." | "..")
                    && (prefix.starts_with('.') || !entry.name.starts_with('.'))
                    && entry.name.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            })
            .map(|entry| {
                let slash = if entry.file_type == FileType::Directory {
                    "/"
                } else {
                    ""
                };
                alloc::format!("{}{}{}", base, entry.name, slash)
            })
            .collect()
    };
    if matches.is_empty() {
        return;
    }
    matches.sort_unstable();
    let first = &matches[0];
    let last = matches.last().unwrap();
    let common = first
        .bytes()
        .zip(last.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    if matches.len() > 1 && common <= word.len() {
        print_str("\r\n");
        for candidate in matches.iter().take(24) {
            console::print_colored(candidate, console::Color::Blue);
            print_str("  ");
        }
        if matches.len() > 24 {
            print_str("...");
        }
        print_str("\r\n");
        console::print_colored(prompt, console::Color::Green);
        print_str(line);
        back(line.len() - *cursor);
        return;
    }

    let mut replacement = String::from(&line[..start]);
    for ch in first[..common].chars() {
        if separator(ch) || matches!(ch, '\\' | '\'' | '"') {
            replacement.push('\\');
        }
        replacement.push(ch);
    }
    if matches.len() == 1 && !first.ends_with('/') && *cursor == line.len() {
        replacement.push(' ');
    }
    let next = replacement.len();
    replacement.push_str(&line[*cursor..]);
    if replacement.len() <= MAX_LINE {
        replace(line, cursor, &replacement, next);
    }
}

/// PS/2 Set-1 press-code -> char (letters, digits, space and basics).
fn decode(sc: u8, shift: bool) -> Option<char> {
    const ROW: &[(u8, char, char)] = &[
        (0x02, '1', '!'),
        (0x03, '2', '@'),
        (0x04, '3', '#'),
        (0x05, '4', '$'),
        (0x06, '5', '%'),
        (0x07, '6', '^'),
        (0x08, '7', '&'),
        (0x09, '8', '*'),
        (0x0A, '9', '('),
        (0x0B, '0', ')'),
        (0x0C, '-', '_'),
        (0x0D, '=', '+'),
        (0x10, 'q', 'Q'),
        (0x11, 'w', 'W'),
        (0x12, 'e', 'E'),
        (0x13, 'r', 'R'),
        (0x14, 't', 'T'),
        (0x15, 'y', 'Y'),
        (0x16, 'u', 'U'),
        (0x17, 'i', 'I'),
        (0x18, 'o', 'O'),
        (0x19, 'p', 'P'),
        (0x1A, '[', '{'),
        (0x1B, ']', '}'),
        (0x1E, 'a', 'A'),
        (0x1F, 's', 'S'),
        (0x20, 'd', 'D'),
        (0x21, 'f', 'F'),
        (0x22, 'g', 'G'),
        (0x23, 'h', 'H'),
        (0x24, 'j', 'J'),
        (0x25, 'k', 'K'),
        (0x26, 'l', 'L'),
        (0x27, ';', ':'),
        (0x28, '\'', '"'),
        (0x29, '`', '~'),
        (0x2B, '\\', '|'),
        (0x2C, 'z', 'Z'),
        (0x2D, 'x', 'X'),
        (0x2E, 'c', 'C'),
        (0x2F, 'v', 'V'),
        (0x30, 'b', 'B'),
        (0x31, 'n', 'N'),
        (0x32, 'm', 'M'),
        (0x33, ',', '<'),
        (0x34, '.', '>'),
        (0x35, '/', '?'),
        (0x39, ' ', ' '),
    ];
    for (code, lo, hi) in ROW {
        if *code == sc {
            return Some(if shift { *hi } else { *lo });
        }
    }
    None
}
