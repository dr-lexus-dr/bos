//! Fixed-capacity bSH line editing; shared by serial and PS/2 input.
pub const CAPACITY: usize = 511;
pub const COMMANDS: &[&[u8]] = &[
    b"help",
    b"exit",
    b"sync",
    b"echo",
    b"cat",
    b"wc",
    b"run",
    b"/applications/hello.bh",
    b"/applications/native-demo.bh",
    b"/applications/process-test.bh",
    b"/applications/native-shell.bh",
    b"/applications/native-cat.bh",
    b"/applications/native-wc.bh",
];
#[derive(Clone, Copy)]
pub struct Line {
    bytes: [u8; CAPACITY],
    pub len: usize,
    pub cursor: usize,
}
impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
            cursor: 0,
        }
    }
    pub fn text(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn insert(&mut self, byte: u8) {
        if self.len == CAPACITY {
            return;
        }
        self.bytes
            .copy_within(self.cursor..self.len, self.cursor + 1);
        self.bytes[self.cursor] = byte;
        self.cursor += 1;
        self.len += 1;
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.delete();
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.len {
            self.bytes
                .copy_within(self.cursor + 1..self.len, self.cursor);
            self.len -= 1;
        }
    }
    pub fn kill_before(&mut self) {
        self.bytes.copy_within(self.cursor..self.len, 0);
        self.len -= self.cursor;
        self.cursor = 0;
    }
    pub fn kill_after(&mut self) {
        self.len = self.cursor;
    }
    pub fn kill_word(&mut self) {
        while self.cursor > 0 && self.bytes[self.cursor - 1].is_ascii_whitespace() {
            self.backspace();
        }
        while self.cursor > 0 && !self.bytes[self.cursor - 1].is_ascii_whitespace() {
            self.backspace();
        }
    }
    /// Complete the first word against builtins and bundled executable paths.
    /// Returns the number of matching candidates; multiple matches can be displayed.
    pub fn complete(&mut self) -> usize {
        if self.cursor != self.len
            || self.text().iter().any(u8::is_ascii_whitespace)
            || self.len == 0
        {
            return 0;
        }
        let mut first: Option<&[u8]> = None;
        let mut common = 0;
        let mut count = 0;
        for &candidate in COMMANDS {
            if candidate.starts_with(self.text()) {
                if let Some(first) = first {
                    common = common.min(
                        first
                            .iter()
                            .zip(candidate)
                            .take_while(|(a, b)| a == b)
                            .count(),
                    );
                } else {
                    first = Some(candidate);
                    common = candidate.len();
                }
                count += 1;
            }
        }
        if let Some(first) = first {
            for &b in &first[self.len..common] {
                self.insert(b);
            }
            if count == 1 {
                self.insert(b' ');
            }
        }
        count
    }
}
pub struct History {
    lines: [Line; 8],
    len: usize,
    selected: Option<usize>,
    draft: Line,
}
impl History {
    pub const fn new() -> Self {
        Self {
            lines: [Line::new(); 8],
            len: 0,
            selected: None,
            draft: Line::new(),
        }
    }
    pub fn record(&mut self, line: &Line) {
        self.selected = None;
        if line.text().iter().all(u8::is_ascii_whitespace)
            || self.len > 0 && self.lines[self.len - 1].text() == line.text()
        {
            return;
        }
        if self.len == self.lines.len() {
            self.lines.copy_within(1.., 0);
            self.len -= 1;
        }
        self.lines[self.len] = *line;
        self.len += 1;
    }
    /// Like Zsh's prefix history search: retain the draft when moving back down.
    pub fn navigate(&mut self, line: &mut Line, up: bool) {
        if self.selected.is_none() {
            if !up {
                return;
            }
            self.draft = *line;
        }
        let prefix = &self.draft.text()[..self.draft.cursor];
        let next = if up {
            (0..self.selected.unwrap_or(self.len))
                .rev()
                .find(|i| self.lines[*i].text().starts_with(prefix))
        } else {
            (self.selected.unwrap() + 1..self.len)
                .find(|i| self.lines[*i].text().starts_with(prefix))
        };
        if let Some(index) = next {
            self.selected = Some(index);
            *line = self.lines[index];
            line.cursor = line.len;
        } else if !up {
            self.selected = None;
            *line = self.draft;
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Text(u8),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    Control(u8),
}
#[derive(Default)]
pub struct Decoder {
    escape: u8,
    param: u8,
}
impl Decoder {
    pub fn feed(&mut self, byte: u8) -> Option<Key> {
        if byte == 27 {
            self.escape = 1;
            self.param = 0;
            return None;
        }
        if self.escape == 1 {
            self.escape = if byte == b'[' || byte == b'O' { 2 } else { 0 };
            return None;
        }
        if self.escape == 2 {
            if byte.is_ascii_digit() {
                self.param = self.param.saturating_mul(10).saturating_add(byte - b'0');
                return None;
            }
            // Keep consuming CSI modifiers until the final byte; never insert
            // the tail of a modified arrow sequence into the command.
            if (b' '..=b'?').contains(&byte) {
                return None;
            }
            self.escape = 0;
            return match byte {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                b'~' => match self.param {
                    1 | 7 => Some(Key::Home),
                    3 => Some(Key::Delete),
                    4 | 8 => Some(Key::End),
                    _ => None,
                },
                _ => None,
            };
        }
        match byte {
            b'\r' | b'\n' => Some(Key::Enter),
            8 | 127 => Some(Key::Backspace),
            9 => Some(Key::Tab),
            0..=31 => Some(Key::Control(byte)),
            32..=126 => Some(Key::Text(byte)),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn line(text: &[u8]) -> Line {
        let mut l = Line::new();
        for &b in text {
            l.insert(b);
        }
        l
    }
    #[test]
    fn edit_middle_and_capacity() {
        let mut l = line(b"echo ac");
        l.cursor -= 1;
        l.insert(b'b');
        assert_eq!(l.text(), b"echo abc");
        l.backspace();
        l.delete();
        assert_eq!(l.text(), b"echo a");
        for _ in 0..600 {
            l.insert(b'x');
        }
        assert_eq!(l.len, CAPACITY);
        l.kill_before();
        assert_eq!(l.len, 0);
    }
    #[test]
    fn prefix_history_restores_draft() {
        let mut h = History::new();
        for s in [b"echo one".as_slice(), b"help", b"echo two"] {
            h.record(&line(s));
        }
        let mut l = line(b"echo ");
        h.navigate(&mut l, true);
        assert_eq!(l.text(), b"echo two");
        h.navigate(&mut l, true);
        assert_eq!(l.text(), b"echo one");
        h.navigate(&mut l, false);
        assert_eq!(l.text(), b"echo two");
        h.navigate(&mut l, false);
        assert_eq!(l.text(), b"echo ");
    }
    #[test]
    fn completion_and_escape_decoder() {
        let mut l = line(b"he");
        assert_eq!(l.complete(), 1);
        assert_eq!(l.text(), b"help ");
        let mut l = line(b"/applications/native-");
        assert!(l.complete() > 1);
        assert_eq!(l.text(), b"/applications/native-");
        let mut d = Decoder::default();
        assert_eq!(d.feed(27), None);
        assert_eq!(d.feed(b'['), None);
        assert_eq!(d.feed(b'D'), Some(Key::Left));
        for b in b"\x1b[3" {
            assert_eq!(d.feed(*b), None);
        }
        assert_eq!(d.feed(b'~'), Some(Key::Delete));
        for b in b"\x1b[1;5" {
            assert_eq!(d.feed(*b), None);
        }
        assert_eq!(d.feed(b'D'), Some(Key::Left));
    }
}
