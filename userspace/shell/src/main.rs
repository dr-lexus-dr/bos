//! bSH — bOS shell: interactive loop over the shared fsutil commands.
//!
//! Builtins: `cd`, `pwd`, `exit`/`quit`, `clear`, `help`. Everything else
//! dispatches to `fsutil::run`, with `>` / `>>` / `<` redirection and
//! single/double-quote handling. Inside bOS the same commands talk to the
//! kernel over syscalls; this host build operates on the host FS.

#[path = "../../coreutils/src/fsutil.rs"]
mod fsutil;

use std::io::{self, Write};

const BSH_VERSION: &str = "0.1.0";

fn main() {
    println!("bOS bSH v{BSH_VERSION} — type 'help'");
    loop {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        print!("bSH:{cwd}> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        if !exec_line(line.trim()) {
            break;
        }
    }
}

/// Execute one line. Returns false when the shell should exit.
fn exec_line(line: &str) -> bool {
    let toks = fsutil::tokenize(line);
    if toks.is_empty() {
        return true;
    }
    // Split redirection: `> file`, `>> file`, `< file`.
    let mut cmd: Vec<String> = Vec::new();
    let mut outfile: Option<String> = None;
    let mut append = false;
    let mut infile: Option<String> = None;
    let mut i = 0;
    while i < toks.len() {
        match toks[i].as_str() {
            ">" | ">>" => {
                append = toks[i] == ">>";
                i += 1;
                if i < toks.len() {
                    outfile = Some(toks[i].clone());
                } else {
                    eprintln!("bSH: syntax error: redirection without target");
                    return true;
                }
            }
            "<" => {
                i += 1;
                if i < toks.len() {
                    infile = Some(toks[i].clone());
                } else {
                    eprintln!("bSH: syntax error: redirection without target");
                    return true;
                }
            }
            _ => cmd.push(toks[i].clone()),
        }
        i += 1;
    }
    if cmd.is_empty() {
        return true;
    }
    let name = cmd[0].as_str();
    let args: Vec<&str> = cmd[1..].iter().map(|s| s.as_str()).collect();

    // Builtins that never redirect output.
    match name {
        "exit" | "quit" => return false,
        "cd" => {
            let target = args.first().copied().unwrap_or("/");
            // `cd` without args goes home; bare `cd -` reverts via OLDPWD.
            let dest = if target == "-" {
                std::env::var("OLDPWD").unwrap_or_else(|_| "/".into())
            } else if target == "~" || target.is_empty() {
                std::env::var("HOME").unwrap_or_else(|_| "/".into())
            } else {
                target.to_string()
            };
            if let Ok(cur) = std::env::current_dir() {
                std::env::set_var("OLDPWD", cur);
            }
            if let Err(e) = std::env::set_current_dir(&dest) {
                eprintln!("cd: {dest}: {e}");
            }
            return true;
        }
        "clear" => {
            print!("\x1b[2J\x1b[H");
            let _ = io::stdout().flush();
            return true;
        }
        _ => {}
    }

    let stdin_data = infile.as_deref().map(|p| match fsutil::read_input_file(p) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("bSH: {e}");
            None
        }
    });
    let stdin_opt: Option<Option<String>> = stdin_data;
    if infile.is_some() && stdin_opt.as_ref().map(|o| o.is_none()).unwrap_or(false) {
        return true; // input file unreadable; error already printed
    }
    let stdin_ref = stdin_opt.as_ref().and_then(|o| o.as_deref());

    let (out, _rc) = fsutil::run(name, &args, stdin_ref);
    match outfile {
        Some(path) => {
            let r = if append {
                std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(
                    |mut f| {
                        use std::io::Write as _;
                        f.write_all(out.as_bytes())
                    },
                )
            } else {
                std::fs::write(&path, out.as_bytes())
            };
            if let Err(e) = r {
                eprintln!("bSH: {path}: {e}");
            }
        }
        None => print!("{out}"),
    }
    true
}
