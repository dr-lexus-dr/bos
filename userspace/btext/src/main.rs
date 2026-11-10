//! btext — the standard bOS text editor.
//!
//! Line-oriented editor with numbered lines, search and substitute:
//!
//!   btext <file>        open (or create) a file
//!
//! Commands (at the `btext>` prompt):
//!   p [n]               print buffer (from line n)
//!   n / b               next / previous page
//!   e <n> <text>        replace line n with text
//!   i <n> <text>        insert text as a new line before line n
//!   a <text>            append text at the end of the buffer
//!   d <n> [m]           delete line n (or lines n..m)
//!   s/<old>/<new>[/g]   substitute (first match per line, all with g)
//!   /<text>             search for text
//!   w [file]            save buffer
//!   q                   quit (warns about unsaved changes)
//!   q!                  quit without saving
//!   clear               clear the screen
//!   h                   this help

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

const BTEXT_VERSION: &str = "0.1.0";
const PAGE: usize = 24;

const HELP: &str = "\
btext commands:
  p [n]               print buffer (from line n)
  n / b               next / previous page
  e <n> <text>        replace line n with text
  i <n> <text>        insert text as a new line before line n
  a <text>            append text at the end of the buffer
  d <n> [m]           delete line n (or lines n..m)
  s/<old>/<new>[/g]   substitute (first match per line, all with g)
  /<text>             search for text
  w [file]            save buffer
  q                   quit (warns about unsaved changes)
  q!                  quit without saving
  clear               clear the screen
  h                   this help";

enum Flow {
    Continue,
    Quit,
    Redraw,
}

struct Buffer {
    path: PathBuf,
    lines: Vec<String>,
    dirty: bool,
    top: usize,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("btext v{BTEXT_VERSION} — the standard bOS text editor");
        println!();
        println!("Usage: btext <file>");
        println!();
        println!("{HELP}");
        return;
    }
    let Some(arg) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("btext: usage: btext <file>");
        eprintln!("       btext -h for help");
        std::process::exit(2);
    };
    let mut buf = Buffer::load(PathBuf::from(arg));
    run(&mut buf);
}

impl Buffer {
    fn load(path: PathBuf) -> Buffer {
        let lines = match fs::read_to_string(&path) {
            Ok(s) => {
                let mut v: Vec<String> = s.split('\n').map(str::to_string).collect();
                if v.last().map(String::is_empty).unwrap_or(false) {
                    v.pop(); // trailing newline, not an empty last line
                }
                v
            }
            Err(_) => Vec::new(),
        };
        Buffer {
            path,
            lines,
            dirty: false,
            top: 0,
        }
    }

    fn save(&mut self) -> io::Result<()> {
        let mut s = self.lines.join("\n");
        s.push('\n');
        fs::write(&self.path, s)
    }

    fn clamp_top(&mut self) {
        if self.lines.is_empty() {
            self.top = 0;
        } else if self.top >= self.lines.len() {
            self.top = self.lines.len() - 1;
        }
    }
}

fn run(buf: &mut Buffer) {
    print!("\x1b[2J\x1b[H"); // clear
    render(buf);
    loop {
        print!("{}", prompt(buf));
        let _ = io::stdout().flush();
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => {
                println!();
                break; // EOF: leave without saving
            }
            Ok(_) => {}
            Err(_) => break,
        }
        let input = input.trim_end_matches(['\r', '\n']);
        if input.is_empty() {
            continue;
        }
        match handle(buf, input) {
            Flow::Continue => {}
            Flow::Quit => break,
            Flow::Redraw => render(buf),
        }
    }
}

fn prompt(buf: &Buffer) -> String {
    let dirty = if buf.dirty { "*" } else { "" };
    let name = buf.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
    format!("\x1b[1;32mbtext{dirty}\x1b[0m:{name}> ")
}

fn render(buf: &mut Buffer) {
    println!(
        "\x1b[1;36m── btext v{BTEXT_VERSION} ── {} \x1b[0m",
        buf.path.display()
    );
    if buf.lines.is_empty() {
        println!("    (empty buffer — use 'a <text>' to add lines)");
    } else {
        buf.clamp_top();
        let end = (buf.top + PAGE).min(buf.lines.len());
        let width = buf.lines.len().to_string().len().max(2);
        for (idx, line) in buf.lines[buf.top..end].iter().enumerate() {
            let n = buf.top + idx + 1;
            println!("\x1b[90m{:>width$}\x1b[0m │ {line}", n, width = width);
        }
        println!(
            "\x1b[90m── lines {}–{} of {} ──{}──\x1b[0m",
            buf.top + 1,
            end,
            buf.lines.len(),
            if buf.dirty { " *unsaved* " } else { "" }
        );
    }
}

fn err(msg: &str) {
    eprintln!("\x1b[31mbtext: {msg}\x1b[0m");
}

fn info(msg: &str) {
    println!("\x1b[90m{msg}\x1b[0m");
}

fn handle(buf: &mut Buffer, input: &str) -> Flow {
    let (cmd, rest) = input.split_once(' ').unwrap_or((input, ""));
    match cmd {
        "h" | "help" | "?" => {
            println!("{HELP}");
            Flow::Continue
        }
        "clear" | "cls" => {
            print!("\x1b[2J\x1b[H");
            Flow::Redraw
        }
        "p" | "print" | "list" => {
            if let Ok(n) = rest.trim().parse::<usize>() {
                if n == 0 {
                    err("line numbers start at 1");
                    return Flow::Continue;
                }
                buf.top = n - 1;
            } else {
                buf.top = 0;
            }
            buf.clamp_top();
            Flow::Redraw
        }
        "n" | "next" => {
            if buf.top + PAGE < buf.lines.len() {
                buf.top += PAGE;
            }
            Flow::Redraw
        }
        "b" | "prev" | "back" => {
            buf.top = buf.top.saturating_sub(PAGE);
            Flow::Redraw
        }
        "e" | "edit" => cmd_edit(buf, rest),
        "i" | "insert" => cmd_insert(buf, rest),
        "a" | "append" => {
            if rest.is_empty() {
                err("usage: a <text>");
                return Flow::Continue;
            }
            buf.lines.push(rest.to_string());
            buf.dirty = true;
            buf.top = buf.lines.len().saturating_sub(PAGE).min(buf.top.max(0));
            Flow::Redraw
        }
        "d" | "del" | "delete" => cmd_delete(buf, rest),
        "s" | "sub" | "substitute" => cmd_substitute(buf, rest),
        "w" | "write" | "save" => {
            let target = rest.trim();
            if !target.is_empty() {
                buf.path = PathBuf::from(target);
            }
            match buf.save() {
                Ok(_) => {
                    buf.dirty = false;
                    info(&format!("saved {}", buf.path.display()));
                }
                Err(e) => err(&format!("save failed: {e}")),
            }
            Flow::Continue
        }
        "q" | "quit" | "exit" => {
            if buf.dirty {
                println!("unsaved changes — 'w' to save, 'q!' to discard and quit");
                Flow::Continue
            } else {
                Flow::Quit
            }
        }
        "q!" => Flow::Quit,
        _ if cmd.starts_with('/') => cmd_search(buf, &cmd[1..]),
        _ => {
            err(&format!("unknown command '{cmd}' — type 'h' for help"));
            Flow::Continue
        }
    }
}

fn cmd_edit(buf: &mut Buffer, rest: &str) -> Flow {
    let Some((num, text)) = rest.split_once(' ') else {
        err("usage: e <n> <text>");
        return Flow::Continue;
    };
    let Ok(n) = num.parse::<usize>() else {
        err("usage: e <n> <text> (n is a line number)");
        return Flow::Continue;
    };
    if n == 0 || n > buf.lines.len() {
        err(&format!("line {n} is out of range (1..={})", buf.lines.len()));
        return Flow::Continue;
    }
    buf.lines[n - 1] = text.to_string();
    buf.dirty = true;
    Flow::Redraw
}

fn cmd_insert(buf: &mut Buffer, rest: &str) -> Flow {
    let Some((num, text)) = rest.split_once(' ') else {
        err("usage: i <n> <text>");
        return Flow::Continue;
    };
    let Ok(n) = num.parse::<usize>() else {
        err("usage: i <n> <text> (n is a line number)");
        return Flow::Continue;
    };
    if n == 0 || n > buf.lines.len() + 1 {
        err(&format!("line {n} is out of range (1..={})", buf.lines.len() + 1));
        return Flow::Continue;
    }
    buf.lines.insert(n - 1, text.to_string());
    buf.dirty = true;
    Flow::Redraw
}

fn cmd_delete(buf: &mut Buffer, rest: &str) -> Flow {
    let nums: Vec<&str> = rest.split_whitespace().collect();
    let (start, end) = match nums.as_slice() {
        [n] => {
            let n = match n.parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    err("usage: d <n> [m]");
                    return Flow::Continue;
                }
            };
            (n, n)
        }
        [n, m] => {
            let parse = |s: &str| s.parse::<usize>().ok();
            match (parse(n), parse(m)) {
                (Some(n), Some(m)) => (n, m),
                _ => {
                    err("usage: d <n> [m]");
                    return Flow::Continue;
                }
            }
        }
        _ => {
            err("usage: d <n> [m]");
            return Flow::Continue;
        }
    };
    if start == 0 || end < start || end > buf.lines.len() {
        err(&format!(
            "range {start}..{end} is out of bounds (1..={})",
            buf.lines.len()
        ));
        return Flow::Continue;
    }
    buf.lines.drain(start - 1..end);
    if buf.lines.is_empty() {
        buf.lines.push(String::new());
    }
    buf.dirty = true;
    buf.clamp_top();
    Flow::Redraw
}

fn cmd_substitute(buf: &mut Buffer, rest: &str) -> Flow {
    // s/<old>/<new>[/g]
    let parts: Vec<&str> = rest.splitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() {
        err("usage: s/<old>/<new>[/g]");
        return Flow::Continue;
    }
    let old = parts[0];
    let new = parts[1];
    let replace_all = parts.get(2).map(|f| *f == "g").unwrap_or(false);
    let mut hits = 0;
    for line in buf.lines.iter_mut() {
        if replace_all {
            let new_line = line.replace(old, new);
            if new_line != *line {
                hits += line.matches(old).count();
                *line = new_line;
            }
        } else if let Some(pos) = line.find(old) {
            line.replace_range(pos..pos + old.len(), new);
            hits += 1;
        }
    }
    if hits == 0 {
        info("no matches");
    } else {
        buf.dirty = true;
        info(&format!("{hits} substitution(s)"));
    }
    Flow::Redraw
}

fn cmd_search(buf: &mut Buffer, pattern: &str) -> Flow {
    if pattern.is_empty() {
        err("usage: /<text>");
        return Flow::Continue;
    }
    let mut matches = 0;
    for (idx, line) in buf.lines.iter().enumerate() {
        if line.contains(pattern) {
            matches += 1;
            let n = idx + 1;
            println!("\x1b[90m{:>4}\x1b[0m │ {line}", n);
        }
    }
    if matches == 0 {
        info(&format!("no matches for '{pattern}'"));
    } else {
        info(&format!("{matches} match(es)"));
    }
    Flow::Continue
}
