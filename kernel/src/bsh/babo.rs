//! Full-screen text editor for the bOS console.

use super::{
    line::{self, Key},
    print_error,
};
use crate::{
    drivers::console,
    fs::{self, FileSystemError, VFS},
};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::ops::Range;

const MAX_BYTES: usize = 128 * 1024;
const MAX_UNDO: usize = 64;
const MAX_UNDO_BYTES: usize = 512 * 1024;
const WHITE: &str = "\x1b[97m";
const GREEN: &str = "\x1b[92m";
const BLUE: &str = "\x1b[94m";
const YELLOW: &str = "\x1b[93m";

struct Change {
    offset: usize,
    removed: String,
    inserted: String,
    before_cursor: usize,
    after_cursor: usize,
    before_revision: u64,
    after_revision: u64,
}

struct Document {
    text: String,
    cursor: usize,
    revision: u64,
    next_revision: u64,
    saved_revision: u64,
    undo: Vec<Change>,
    redo: Vec<Change>,
}

impl Document {
    fn new(text: String) -> Self {
        Self {
            text,
            cursor: 0,
            revision: 0,
            next_revision: 1,
            saved_revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    fn edit(&mut self, range: Range<usize>, inserted: &str) -> Result<(), &'static str> {
        if self.text.len() - range.len() + inserted.len() > MAX_BYTES {
            return Err("File limit: 128 KiB");
        }
        if range.is_empty() && inserted.is_empty() {
            return Ok(());
        }
        let change = Change {
            offset: range.start,
            removed: self.text[range.clone()].to_string(),
            inserted: inserted.to_string(),
            before_cursor: self.cursor,
            after_cursor: range.start + inserted.len(),
            before_revision: self.revision,
            after_revision: self.next_revision,
        };
        self.next_revision += 1;
        self.revision = change.after_revision;
        self.cursor = change.after_cursor;
        self.text.replace_range(range, inserted);
        self.redo.clear();
        self.undo.push(change);
        while self.undo.len() > MAX_UNDO
            || self
                .undo
                .iter()
                .map(|entry| entry.removed.len() + entry.inserted.len())
                .sum::<usize>()
                > MAX_UNDO_BYTES
        {
            self.undo.remove(0);
        }
        Ok(())
    }

    fn undo(&mut self) {
        if let Some(change) = self.undo.pop() {
            self.text.replace_range(
                change.offset..change.offset + change.inserted.len(),
                &change.removed,
            );
            self.cursor = change.before_cursor;
            self.revision = change.before_revision;
            self.redo.push(change);
        }
    }

    fn redo(&mut self) {
        if let Some(change) = self.redo.pop() {
            self.text.replace_range(
                change.offset..change.offset + change.removed.len(),
                &change.inserted,
            );
            self.cursor = change.after_cursor;
            self.revision = change.after_revision;
            self.undo.push(change);
        }
    }

    fn start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn end(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .find('\n')
                .unwrap_or(self.text.len() - self.cursor)
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
    }

    fn vertical(&mut self, delta: isize, column: usize) {
        let mut start = self.start();
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                if start == 0 {
                    break;
                }
                start = self.text[..start - 1]
                    .rfind('\n')
                    .map_or(0, |index| index + 1);
            } else {
                let Some(end) = self.text[start..].find('\n') else {
                    break;
                };
                start += end + 1;
            }
        }
        let text = self.text[start..].split('\n').next().unwrap_or("");
        let mut width = 0;
        self.cursor = start;
        for ch in text.chars() {
            let next_width = width + char_width(ch, width);
            if next_width > column {
                break;
            }
            width = next_width;
            self.cursor += ch.len_utf8();
        }
    }
}

struct Screen;

impl Drop for Screen {
    fn drop(&mut self) {
        console::end_editor();
    }
}

struct Editor {
    path: String,
    document: Document,
    crlf: bool,
    top: usize,
    left: usize,
    width: usize,
    height: usize,
    goal_column: Option<usize>,
    message: String,
    search: String,
    clipboard: String,
    rendered: Vec<String>,
}

pub(super) fn run(args: &[&str]) {
    let path = match args {
        [] => super::DEFAULT_NOTES,
        [path] => *path,
        _ => return print_error("usage: babo [file]"),
    };
    let (width, height) = console::editor_dimensions();
    if width < 40 || height < 8 {
        return print_error("babo: a console of at least 40x8 cells is required");
    }
    let loaded = (|| {
        let mut vfs = VFS.lock();
        let path = vfs.realpath(path)?;
        let data = match vfs.stat(&path) {
            Ok(stat) if stat.is_dir() || stat.size > MAX_BYTES as u64 => {
                return Err(FileSystemError::InvalidArgument)
            }
            Ok(_) => vfs.read_file(&path)?,
            Err(FileSystemError::NotFound) => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok((path, data))
    })();
    let (path, bytes) = match loaded {
        Ok(result) => result,
        Err(error) => return print_error(&format!("babo: {}", super::fs_err(error))),
    };
    let text = match String::from_utf8(bytes) {
        Ok(text)
            if !text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) =>
        {
            text
        }
        _ => return print_error("babo: expected a UTF-8 text file"),
    };
    let crlf = text.contains("\r\n") && !text.replace("\r\n", "").contains('\n');
    let document = Document::new(text.replace("\r\n", "\n"));
    let mut editor = Editor {
        path,
        document,
        crlf,
        top: 0,
        left: 0,
        width: width - 1,
        height,
        goal_column: None,
        message: "Ctrl+S saves; Ctrl+Q exits. Arrows move the cursor.".into(),
        search: String::new(),
        clipboard: String::new(),
        rendered: Vec::new(),
    };
    console::begin_editor();
    let _screen = Screen;
    let _color = console::ColorGuard::new(console::Color::White);
    editor.run();
}

impl Editor {
    fn run(&mut self) {
        loop {
            self.draw();
            let (row, col) = self.cursor_position();
            console::editor_cursor(row, col);
            console::editor_caret(true);
            let key = line::key();
            console::editor_caret(false);
            if matches!(key, Key::Control(b'q' | b'x')) {
                if !self.document.dirty() {
                    return;
                }
                match self.prompt("Unsaved changes: S save / D discard, Enter confirms", "") {
                    Some(answer) if answer.eq_ignore_ascii_case("d") => return,
                    Some(answer) if answer.eq_ignore_ascii_case("s") => {
                        if self.save() {
                            return;
                        }
                    }
                    _ => self.message = "Exit canceled".into(),
                }
                continue;
            }
            if let Err(message) = self.handle(key) {
                self.message = message.into();
            }
        }
    }

    fn handle(&mut self, key: Key) -> Result<(), &'static str> {
        let doc = &mut self.document;
        match key {
            Key::Left => doc.cursor = doc.previous(),
            Key::Right => doc.cursor = doc.next(),
            Key::Home | Key::Control(b'a') => doc.cursor = doc.start(),
            Key::End | Key::Control(b'e') => doc.cursor = doc.end(),
            Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
                let column = *self
                    .goal_column
                    .get_or_insert_with(|| display_width(&doc.text[doc.start()..doc.cursor]));
                let delta = match key {
                    Key::Up => -1,
                    Key::Down => 1,
                    Key::PageUp => -(self.height as isize - 4),
                    _ => self.height as isize - 4,
                };
                doc.vertical(delta, column);
                return Ok(());
            }
            Key::Text(ch) => {
                doc.edit(doc.cursor..doc.cursor, ch.encode_utf8(&mut [0; 4]))?;
            }
            Key::Enter => {
                doc.edit(doc.cursor..doc.cursor, "\n")?;
            }
            Key::Tab => {
                let spaces = 4 - display_width(&doc.text[doc.start()..doc.cursor]) % 4;
                doc.edit(doc.cursor..doc.cursor, &"    "[..spaces])?;
            }
            Key::Backspace if doc.cursor > 0 => {
                doc.edit(doc.previous()..doc.cursor, "")?;
            }
            Key::Delete if doc.cursor < doc.text.len() => {
                doc.edit(doc.cursor..doc.next(), "")?;
            }
            Key::Control(b'z') => {
                doc.undo();
                self.message = "Undo".into();
            }
            Key::Control(b'y') => {
                doc.redo();
                self.message = "Redo".into();
            }
            Key::Control(b'k') => {
                let start = doc.start();
                let end = (doc.end() + 1).min(doc.text.len());
                self.clipboard = doc.text[start..end].to_string();
                doc.edit(start..end, "")?;
                self.message = "Line cut. Ctrl+U pastes.".into();
            }
            Key::Control(b'u') => {
                doc.edit(doc.cursor..doc.cursor, &self.clipboard)?;
            }
            Key::Control(b's' | b'o') => {
                self.save();
            }
            Key::Control(b'w') => {
                let previous = self.search.clone();
                if let Some(query) = self.prompt("Search / Ctrl+Q cancel", &previous) {
                    self.search = query;
                    self.find(false);
                }
            }
            Key::Control(b'n') => self.find(true),
            Key::Control(b'g') => {
                if let Some(value) = self.prompt("Go to line / Ctrl+Q cancel", "") {
                    match value.parse::<usize>() {
                        Ok(number) if number > 0 => {
                            let mut start = 0;
                            for _ in 1..number.min(self.document.text.lines().count() + 1) {
                                let Some(end) = self.document.text[start..].find('\n') else {
                                    break;
                                };
                                start += end + 1;
                            }
                            self.document.cursor = start;
                        }
                        _ => self.message = "Enter a positive line number".into(),
                    }
                }
            }
            _ => {}
        }
        self.goal_column = None;
        Ok(())
    }

    fn find(&mut self, next: bool) {
        if self.search.is_empty() {
            self.message = "Ctrl+W starts a search".into();
            return;
        }
        let doc = &mut self.document;
        let start = if next { doc.next() } else { doc.cursor };
        let found = doc.text[start..]
            .find(&self.search)
            .map(|offset| start + offset)
            .or_else(|| doc.text.find(&self.search));
        match found {
            Some(offset) => {
                doc.cursor = offset;
                self.message = "Match found. Ctrl+N finds the next.".into();
            }
            None => self.message = "No matches".into(),
        }
    }

    fn save(&mut self) -> bool {
        let bytes = if self.crlf {
            self.document.text.replace("\n", "\r\n")
        } else {
            self.document.text.clone()
        };
        let result = save_file(&self.path, bytes.as_bytes());
        match result {
            Ok(persistent) => {
                self.document.saved_revision = self.document.revision;
                self.message = format!(
                    "Saved {} bytes{}",
                    bytes.len(),
                    if persistent {
                        " and synced to disk"
                    } else {
                        " (RAM session)"
                    }
                );
                true
            }
            Err(error) => {
                self.message = format!(
                    "Save failed: {}. Changes kept in editor.",
                    super::fs_err(error)
                );
                false
            }
        }
    }

    fn prompt(&mut self, label: &str, initial: &str) -> Option<String> {
        let mut input = String::from(initial);
        loop {
            console::editor_row(self.height - 3, &clip_ansi_free(label, &input, self.width));
            // Keep the status text in the next full redraw.
            self.rendered.clear();
            let column = (label.chars().count().min(self.width / 2) + 2 + input.chars().count())
                .min(self.width - 1);
            console::editor_cursor(self.height - 3, column);
            console::editor_caret(true);
            let key = line::key();
            console::editor_caret(false);
            match key {
                Key::Enter => return Some(input),
                Key::Control(b'q' | b'x') => return None,
                Key::Backspace => {
                    input.pop();
                }
                Key::Text(ch) if input.len() + ch.len_utf8() <= 256 => input.push(ch),
                _ => {}
            }
        }
    }

    fn geometry(&self) -> (usize, usize, usize, usize) {
        let row = self.document.text[..self.document.cursor]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        let col = display_width(&self.document.text[self.document.start()..self.document.cursor]);
        let lines = self
            .document
            .text
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let gutter = lines.to_string().len().max(3) + 2;
        (row, col, lines, gutter)
    }

    fn cursor_position(&self) -> (usize, usize) {
        let (row, col, _, gutter) = self.geometry();
        (row - self.top + 1, col - self.left + gutter)
    }

    fn draw(&mut self) {
        let (row, col, lines, gutter) = self.geometry();
        let rows = self.height - 4;
        let columns = self.width - gutter;
        if row < self.top {
            self.top = row;
        }
        if row >= self.top + rows {
            self.top = row + 1 - rows;
        }
        if col < self.left {
            self.left = col;
        }
        if col >= self.left + columns {
            self.left = col + 1 - columns;
        }
        let mut screen = Vec::new();
        screen.push(format!(
            "{}{}",
            GREEN,
            clip(
                &format!(
                    " BABO  {}{}",
                    self.path,
                    if self.document.dirty() {
                        "  [modified]"
                    } else {
                        ""
                    }
                ),
                self.width
            )
        ));
        for (index, text) in self
            .document
            .text
            .split('\n')
            .enumerate()
            .skip(self.top)
            .take(rows)
        {
            let number = format!("{:>width$}  ", index + 1, width = gutter - 2);
            screen.push(format!(
                "{}{}{}{}",
                if index == row { GREEN } else { BLUE },
                number,
                WHITE,
                visible(text, self.left, columns)
            ));
        }
        while screen.len() < self.height - 3 {
            screen.push(String::new());
        }
        screen.push(format!("{}{}", YELLOW, clip(&self.message, self.width)));
        screen.push(format!(
            "{}{}",
            BLUE,
            clip(
                &format!(
                    " Ln {}/{}  Col {}  {}  Undo {}  {} bytes",
                    row + 1,
                    lines,
                    col + 1,
                    if self.crlf { "CRLF" } else { "LF" },
                    self.document.undo.len(),
                    self.document.text.len()
                ),
                self.width
            )
        ));
        screen.push(format!(
            "{}{}",
            GREEN,
            clip(
                "^S Save  ^Q Exit  ^W Search  ^N Next  ^Z Undo  ^Y Redo  ^K Cut ^U Paste ^G Line",
                self.width
            )
        ));
        for (index, text) in screen.iter().enumerate() {
            if self.rendered.get(index) != Some(text) {
                console::editor_row(index, text);
            }
        }
        self.rendered = screen;
    }
}

fn char_width(ch: char, column: usize) -> usize {
    if ch == '\t' {
        4 - column % 4
    } else {
        1
    }
}
fn display_width(text: &str) -> usize {
    text.chars()
        .fold(0, |column, ch| column + char_width(ch, column))
}
fn clip(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

fn clip_ansi_free(label: &str, input: &str, width: usize) -> String {
    let prefix = clip(label, width / 2);
    let available = width.saturating_sub(prefix.chars().count() + 3);
    let tail: String = input
        .chars()
        .rev()
        .take(available)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}{}: {}", YELLOW, prefix, tail)
}

fn visible(text: &str, left: usize, width: usize) -> String {
    let mut result = String::new();
    let mut column = 0;
    for ch in text.chars() {
        let length = char_width(ch, column);
        for part in 0..length {
            if column + part >= left && column + part < left + width {
                result.push(if ch == '\t' {
                    ' '
                } else if ch.is_control() {
                    '?'
                } else {
                    ch
                });
            }
        }
        column += length;
        if column >= left + width {
            break;
        }
    }
    result
}

/// Stage the complete file before replacing the target and flushing its mount.
fn save_file(path: &str, data: &[u8]) -> Result<bool, FileSystemError> {
    let mut vfs = VFS.lock();
    let mode = match vfs.stat(path) {
        Ok(stat) => stat.mode & 0o7777,
        Err(FileSystemError::NotFound) => 0o644,
        Err(error) => return Err(error),
    };
    let (parent, _) = fs::split_parent(path);
    if !vfs.stat(&parent)?.is_dir() {
        return Err(FileSystemError::NotDirectory);
    }
    let mut staged = None;
    for index in 0..32 {
        let path = format!("{}/.babo-save-{}", parent.trim_end_matches('/'), index);
        match vfs.open(&path, fs::O_WRONLY | fs::O_CREAT | fs::O_EXCL) {
            Ok(fd) => {
                staged = Some((path, fd));
                break;
            }
            Err(FileSystemError::AlreadyExists) => {}
            Err(error) => return Err(error),
        }
    }
    let (temporary, fd) = staged.ok_or(FileSystemError::AlreadyExists)?;
    let result = (|| {
        let mut offset = 0;
        while offset < data.len() {
            let written = vfs.write(fd, &data[offset..])?;
            if written == 0 {
                return Err(FileSystemError::NoSpace);
            }
            offset += written;
        }
        Ok(())
    })();
    let closed = vfs.close(fd);
    let result = result
        .and(closed)
        .and_then(|_| vfs.chmod(&temporary, mode))
        .and_then(|_| vfs.rename(&temporary, path))
        .and_then(|_| vfs.sync_path(path));
    if result.is_err() {
        let _ = vfs.remove(&temporary);
    }
    result?;
    let mount = vfs
        .mounts()
        .into_iter()
        .filter(|mount| {
            path == mount.mount_point
                || path.starts_with(&format!("{}/", mount.mount_point.trim_end_matches('/')))
        })
        .max_by_key(|mount| mount.mount_point.len());
    Ok(mount.is_some_and(|mount| mount.device == "virtio0"))
}
