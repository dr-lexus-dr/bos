//! Terminal scrollback keeps styled cells, so wrapping and selection use the
//! same columns the user sees. ANSI sequences never enter the clipboard.
use alloc::{collections::VecDeque, string::String, vec::Vec};
const NORMAL: u32 = 0xe4e5ef;
const COLORS: [u32; 16] = [
    0x252632, 0xe88594, 0xa1d4ab, 0xe0bd84, 0x85a5df, 0xb99cde, 0x85cace, 0xd0d0dc, 0x838492,
    0xf39aa7, 0xa8e6c0, 0xf5d79c, 0xa2bcf6, 0xd4b1f5, 0xa0e7e9, 0xf5f3ff,
];
#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: u32,
    pub bg: Option<u32>,
}
pub struct Screen {
    lines: VecDeque<Vec<Cell>>,
    column: usize,
    fg: u32,
    bg: Option<u32>,
    esc: u8,
    params: String,
}
impl Screen {
    pub fn new() -> Self {
        Self {
            lines: VecDeque::from([Vec::new()]),
            column: 0,
            fg: NORMAL,
            bg: None,
            esc: 0,
            params: String::new(),
        }
    }
    pub fn clear(&mut self) {
        *self = Self::new()
    }
    fn newline(&mut self) {
        self.lines.push_back(Vec::new());
        if self.lines.len() > 800 {
            self.lines.pop_front();
        }
        self.column = 0;
    }
    fn cell(&mut self, ch: char) {
        if self.column >= 4096 {
            self.newline()
        }
        let c = Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
        };
        let row = self.lines.back_mut().unwrap();
        if self.column == row.len() {
            row.push(c)
        } else {
            row[self.column] = c
        }
        self.column += 1;
    }
    fn sgr(&mut self) {
        let mut pp = self
            .params
            .split(';')
            .map(|p| p.parse::<u16>().unwrap_or(0));
        while let Some(n) = pp.next() {
            match n {
                0 => {
                    self.fg = NORMAL;
                    self.bg = None
                }
                39 => self.fg = NORMAL,
                49 => self.bg = None,
                30..=37 => self.fg = COLORS[(n - 30) as usize],
                40..=47 => self.bg = Some(COLORS[(n - 40) as usize]),
                90..=97 => self.fg = COLORS[(n - 90 + 8) as usize],
                100..=107 => self.bg = Some(COLORS[(n - 100 + 8) as usize]),
                38 | 48 => {
                    let color = match pp.next() {
                        Some(2) => match (pp.next(), pp.next(), pp.next()) {
                            (Some(r), Some(g), Some(b)) => Some(
                                (u32::from(r.min(255)) << 16)
                                    | (u32::from(g.min(255)) << 8)
                                    | u32::from(b.min(255)),
                            ),
                            _ => None,
                        },
                        Some(5) => pp.next().map(|v| {
                            let x = u32::from(v.min(255));
                            if x < 16 {
                                COLORS[x as usize]
                            } else if x < 232 {
                                let x = x - 16;
                                let c = |q| if q == 0 { 0 } else { 55 + q * 40 };
                                c(x / 36) << 16 | c(x / 6 % 6) << 8 | c(x % 6)
                            } else {
                                (8 + (x - 232) * 10) * 0x010101
                            }
                        }),
                        _ => None,
                    };
                    if let Some(color) = color {
                        if n == 38 {
                            self.fg = color
                        } else {
                            self.bg = Some(color)
                        }
                    }
                }
                _ => {}
            }
        }
    }
    pub fn push_str(&mut self, s: &str) {
        for ch in s.chars() {
            match self.esc {
                1 => {
                    self.esc = if ch == '[' { 2 } else { 0 };
                    self.params.clear();
                }
                2 => {
                    if ('@'..='~').contains(&ch) {
                        match ch {
                            'm' => self.sgr(),
                            'K' => self.lines.back_mut().unwrap().truncate(self.column),
                            'J' if self.params == "2" => self.clear(),
                            _ => {}
                        }
                        self.esc = 0;
                    } else if self.params.len() < 48 {
                        self.params.push(ch)
                    }
                }
                _ => match ch {
                    '\u{1b}' => self.esc = 1,
                    '\r' => self.column = 0,
                    '\n' => self.newline(),
                    '\u{8}' => self.column = self.column.saturating_sub(1),
                    '\t' => {
                        let n = 8 - self.column % 8;
                        for _ in 0..n {
                            self.cell(' ')
                        }
                    }
                    c if !c.is_control() => self.cell(c),
                    _ => {}
                },
            }
        }
    }
    pub fn rows(&self, cols: usize) -> impl Iterator<Item = &[Cell]> {
        self.lines
            .iter()
            .enumerate()
            .filter(move |(i, line)| !line.is_empty() || i + 1 < self.lines.len())
            .flat_map(move |(_, line)| {
                let empty = line.is_empty().then_some(&line[..]);
                empty.into_iter().chain(line.chunks(cols.max(1)))
            })
    }
    pub fn copy(&self, cols: usize, a: usize, b: usize) -> String {
        let mut text = String::new();
        for (i, row) in self
            .rows(cols)
            .skip(a.min(b))
            .take(a.abs_diff(b) + 1)
            .enumerate()
        {
            if i != 0 {
                text.push('\n')
            }
            text.extend(row.iter().map(|c| c.ch));
        }
        text
    }
}
