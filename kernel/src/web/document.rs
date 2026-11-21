//! Text layout for a small HTML reader. One bounded list owns all link targets.
use super::http::URL_LIMIT;
use alloc::{
    borrow::Cow,
    string::{String, ToString},
    vec::Vec,
};

pub struct Row {
    pub text: String,
    pub heading: bool,
    pub link: Option<usize>,
}
pub struct Document {
    pub title: String,
    pub rows: Vec<Row>,
    pub links: Vec<String>,
    pub clipped: bool,
}

const ROW_LIMIT: usize = 384;
const LINK_LIMIT: usize = 64;

struct Layout {
    doc: Document,
    width: usize,
    text: String,
    columns: usize,
    heading: bool,
    link: Option<usize>,
    space: bool,
}

impl Layout {
    fn line(&mut self) {
        if !self.text.is_empty() {
            let text = core::mem::take(&mut self.text);
            if self.doc.rows.len() < ROW_LIMIT {
                self.doc.rows.push(Row {
                    text,
                    heading: self.heading,
                    link: self.link,
                });
            } else {
                self.doc.clipped = true;
            }
        }
        self.columns = 0;
        self.space = false;
    }

    fn text(&mut self, text: &str, html: bool) {
        let decoded = if html {
            entities(text)
        } else {
            Cow::Borrowed(text)
        };
        for word in decoded.split_inclusive(char::is_whitespace) {
            let clean = word.trim_end();
            let length = clean.chars().count();
            if self.columns > 0 && self.columns + usize::from(self.space) + length > self.width {
                self.line();
            }
            if self.space && self.columns > 0 && length != 0 {
                self.text.push(' ');
                self.columns += 1;
            }
            for ch in clean.chars().filter(|c| !c.is_control()) {
                if self.columns == self.width {
                    self.line();
                }
                self.text.push(ch);
                self.columns += 1;
            }
            self.space = clean.len() != word.len();
            if !html && word.ends_with('\n') {
                self.line();
            }
        }
    }
}

fn entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, tail)) = rest.split_once('&') {
        output.push_str(before);
        let semi = tail.as_bytes().iter().take(12).position(|&b| b == b';');
        let decoded = semi.and_then(|end| match &tail[..end] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" | "mdash" => Some('-'),
            s if s.starts_with("#x") || s.starts_with("#X") => u32::from_str_radix(&s[2..], 16)
                .ok()
                .and_then(char::from_u32),
            s if s.starts_with('#') => s[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        });
        if let (Some(ch), Some(end)) = (decoded, semi) {
            output.push(ch);
            rest = &tail[end + 1..];
        } else {
            output.push('&');
            rest = tail;
        }
    }
    output.push_str(rest);
    Cow::Owned(output)
}

fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = 0;
    for (i, b) in tag.bytes().enumerate() {
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
        } else if matches!(b, b'\'' | b'"') {
            quote = b;
        } else if b == b'>' {
            return Some(i);
        }
    }
    None
}

fn href(mut attrs: &str) -> Option<String> {
    while !attrs.trim_start().is_empty() {
        attrs = attrs.trim_start();
        let end = attrs
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(attrs.len());
        if end == 0 {
            return None;
        }
        let name = &attrs[..end];
        attrs = attrs[end..].trim_start();
        let Some(value) = attrs.strip_prefix('=') else {
            continue;
        };
        attrs = value.trim_start();
        let value = if attrs.starts_with(['\'', '"']) {
            let quote = attrs.as_bytes()[0] as char;
            let (val, tail) = attrs[1..].split_once(quote)?;
            attrs = tail;
            val
        } else {
            let end = attrs.find(char::is_whitespace).unwrap_or(attrs.len());
            let val = &attrs[..end];
            attrs = &attrs[end..];
            val
        };
        if name.eq_ignore_ascii_case("href") && value.len() <= URL_LIMIT {
            return Some(entities(value).into_owned());
        }
    }
    None
}

pub fn render(bytes: &[u8], html: bool, columns: usize) -> Document {
    let source = String::from_utf8_lossy(bytes);
    let mut rest = source.as_ref();
    let mut layout = Layout {
        doc: Document {
            title: String::new(),
            rows: Vec::new(),
            links: Vec::new(),
            clipped: false,
        },
        text: String::new(),
        width: columns.clamp(16, 120),
        columns: 0,
        heading: false,
        link: None,
        space: false,
    };
    let mut head = false;
    while html && !rest.is_empty() && layout.doc.rows.len() < ROW_LIMIT {
        let Some((text, tail)) = rest.split_once('<') else {
            break;
        };
        if !head {
            layout.text(text, true);
        }
        if let Some(comment) = tail.strip_prefix("!--") {
            rest = comment.split_once("-->").map_or("", |v| v.1);
            continue;
        }
        let Some(end) = tag_end(tail) else {
            rest = "";
            break;
        };
        let tag = tail[..end].trim();
        rest = &tail[end + 1..];
        let (name, attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let name = name.trim_end_matches('/').to_ascii_lowercase();
        // HTML permits omission of </head> and <body> before visible content.
        if head
            && matches!(
                name.as_str(),
                "main" | "section" | "div" | "p" | "h1" | "h2" | "a" | "article"
            )
        {
            head = false;
        }
        match name.as_str() {
            "head" => head = true,
            "/head" | "body" => head = false,
            name @ ("script" | "style" | "title") => {
                let close = match name {
                    "script" => "</script",
                    "style" => "</style",
                    _ => "</title",
                };
                let found = rest
                    .as_bytes()
                    .windows(close.len())
                    .position(|s| s.eq_ignore_ascii_case(close.as_bytes()));
                let len = found.unwrap_or(rest.len());
                if name == "title" {
                    layout.doc.title = entities(rest[..len].trim())
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(100)
                        .collect();
                }
                rest = rest[len..].split_once('>').map_or("", |v| v.1);
            }
            _ if head => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                layout.line();
                layout.heading = true;
            }
            "/h1" | "/h2" | "/h3" | "/h4" | "/h5" | "/h6" => {
                layout.line();
                layout.heading = false;
            }
            "br" | "p" | "/p" | "div" | "/div" | "li" | "tr" | "hr" => layout.line(),
            "a" => {
                layout.line();
                layout.link = href(attrs)
                    .filter(|_| layout.doc.links.len() < LINK_LIMIT)
                    .map(|url| {
                        layout.doc.links.push(url);
                        layout.doc.links.len() - 1
                    });
            }
            "/a" => {
                layout.line();
                layout.link = None;
            }
            _ => {}
        }
    }
    if layout.doc.rows.len() >= ROW_LIMIT {
        layout.doc.clipped |= !rest.is_empty();
    } else if !head {
        layout.text(rest, html);
    }
    layout.line();
    if layout.doc.title.is_empty() {
        layout.doc.title = layout
            .doc
            .rows
            .iter()
            .find(|r| r.heading)
            .map(|r| r.text.to_string())
            .unwrap_or_default();
    }
    layout.doc
}
