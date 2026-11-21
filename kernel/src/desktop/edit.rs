use crate::bsh::line::Key;
use alloc::string::{String, ToString};
use spin::Mutex;
static CLIP: Mutex<String> = Mutex::new(String::new());
pub fn copy(s: &str) {
    *CLIP.lock() = s.to_string();
}
pub fn paste() -> String {
    CLIP.lock().clone()
}
#[derive(Default)]
pub struct Edit {
    pub value: String,
    pub pos: usize,
    pub anchor: usize,
}
impl Edit {
    pub fn new(s: &str) -> Self {
        Self {
            value: s.into(),
            pos: s.len(),
            anchor: s.len(),
        }
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.pos = self.value.len();
    }
    pub fn selection(&self) -> (usize, usize) {
        (self.pos.min(self.anchor), self.pos.max(self.anchor))
    }
    pub fn insert(&mut self, s: &str) {
        let (a, b) = self.selection();
        if self.value.len() - (b - a) + s.len() > 65536 {
            return;
        }
        self.value.replace_range(a..b, s);
        self.pos = a + s.len();
        self.anchor = self.pos;
    }
    pub fn place(&mut self, p: usize, extend: bool) {
        self.pos = p.min(self.value.len());
        while !self.value.is_char_boundary(self.pos) {
            self.pos -= 1;
        }
        if !extend {
            self.anchor = self.pos;
        }
    }
    pub fn key(&mut self, k: Key, shift: bool, multi: bool) -> bool {
        let (a, b) = self.selection();
        match k {
            Key::Text(ch) if !ch.is_control() => {
                let mut buf = [0; 4];
                self.insert(ch.encode_utf8(&mut buf));
            }
            Key::Enter if multi => self.insert("\n"),
            Key::Tab if multi => self.insert("    "),
            Key::Control(b'a') => self.select_all(),
            Key::Control(b'c') | Key::Control(b'x') => {
                if a != b {
                    copy(&self.value[a..b]);
                    if k == Key::Control(b'x') {
                        self.insert("");
                    }
                }
            }
            Key::Control(b'v') => {
                let s = paste();
                if multi {
                    self.insert(&s)
                } else {
                    self.insert(s.lines().next().unwrap_or(""));
                }
            }
            Key::Backspace | Key::Delete => {
                if a == b {
                    self.anchor = if k == Key::Backspace {
                        self.value[..a]
                            .char_indices()
                            .next_back()
                            .map_or(0, |i| i.0)
                    } else {
                        a + self.value[a..].chars().next().map_or(0, char::len_utf8)
                    };
                }
                self.insert("");
            }
            Key::Left | Key::Right | Key::Home | Key::End | Key::Up | Key::Down => {
                let line = self.value[..self.pos].rfind('\n').map_or(0, |p| p + 1);
                let end = self.value[self.pos..]
                    .find('\n')
                    .map_or(self.value.len(), |p| self.pos + p);
                let column = self.value[line..self.pos].chars().count();
                let new = match k {
                    Key::Left if !shift && a != b => a,
                    Key::Right if !shift && a != b => b,
                    Key::Left => self.value[..self.pos]
                        .char_indices()
                        .next_back()
                        .map_or(0, |i| i.0),
                    Key::Right => {
                        self.pos
                            + self.value[self.pos..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                    }
                    Key::Home => line,
                    Key::End => end,
                    Key::Up if multi && line > 0 => {
                        let start = self.value[..line - 1].rfind('\n').map_or(0, |p| p + 1);
                        start
                            + self.value[start..line - 1]
                                .chars()
                                .take(column)
                                .map(char::len_utf8)
                                .sum::<usize>()
                    }
                    Key::Down if multi && end < self.value.len() => {
                        let s = self.value[end + 1..].split('\n').next().unwrap_or("");
                        end + 1 + s.chars().take(column).map(char::len_utf8).sum::<usize>()
                    }
                    _ => self.pos,
                };
                self.place(new, shift);
            }
            _ => return false,
        }
        true
    }
}
