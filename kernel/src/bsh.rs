//! bSH — built-in kernel console shell (bring-up stage).
//!
//! Runs on polling only: serial COM1 plus PS/2 Set-1 scancodes. The VFS
//! (ramfs on `/`, bFS available for `mount`) is initialized lazily on
//! first entry, so the shell offers Linux-like file commands — `ls`, `cd`,
//! `pwd`, `cat`, `touch`, `mkdir`, `rm`, `cp`, `mv`, `ln`, `stat`, `wc`,
//! `head`, `tail`, `du`, `df`, `mount`, `find` and friends — alongside the
//! classic bring-up commands (`help`, `info`, `uname`, `mem`, ...).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::vec;
use core::sync::atomic::{AtomicBool, Ordering};
use log::info;
use x86_64::instructions::port::Port;

use crate::arch::x86_64::serial;
use crate::drivers::{console, keyboard, timer};
use crate::fs::{self, FileSystemError, FileType};
use crate::mm::{heap, phys};

const DEFAULT_USER: &str = "default";
const DEFAULT_HOME: &str = "/users/default";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

static FS_READY: AtomicBool = AtomicBool::new(false);

fn ensure_fs(boot_info: &'static bloader_shared::BootInfo) {
    if FS_READY.swap(true, Ordering::SeqCst) {
        return;
    }
    fs::init(boot_info);
    info!("bSH: VFS ready (ramfs on /, bFS mountable)");
}

pub fn run(boot_info: &'static bloader_shared::BootInfo) -> ! {
    info!("bSH: built-in console ready (polling serial + PS/2)");
    console::init(boot_info.framebuffer);
    console::clear();
    crate::arch::x86_64::speaker_beep(660, 90);
    ensure_fs(boot_info);
    cmd_cd(None);
    print_str("bOS kernel console (bSH) v");
    print_str(KERNEL_VERSION);
    print_str(" — type 'help'\r\n");
    loop {
        let cwd = fs::VFS.lock().get_cwd();
        print_str("bSH:");
        print_str(&cwd);
        print_str("# ");
        let line = read_line();
        exec(&line);
    }
}

// ---------------------------------------------------------------------------
// command dispatch
// ---------------------------------------------------------------------------

fn exec(line: &str) {
    let (toks, redirections) = match tokenize(line).and_then(split_redir) {
        Ok(parsed) => parsed,
        Err(error) => {
            print_line(error);
            return;
        }
    };
    if toks.is_empty() {
        return;
    }
    let cmd = toks[0].as_str();
    let args: Vec<&str> = toks[1..].iter().map(|s| s.as_str()).collect();

    // Interactive commands require direct access to the console.
    if matches!(cmd, "cd" | "edit" | "reboot" | "halt" | "clear")
        && (redirections.input.is_some() || redirections.output.is_some())
    {
        print_line("bSH: this command does not support redirection");
        return;
    }
    match cmd {
        "cd" => return cmd_cd(args.first().copied()),
        "edit" => return cmd_edit(&args),
        "reboot" => {
            print_str("rebooting...\r\n");
            reboot();
        }
        "halt" => {
            print_str("halted.\r\n");
            halt();
        }
        "clear" => return console::clear(),
        _ => {}
    }

    let input = match redirections.input {
        Some(path) => {
            let result = fs::VFS.lock().read_file(&path);
            match result {
                Ok(data) => Some(data),
                Err(error) => {
                    print_line(&alloc::format!("bSH: {}: {}", path, fs_err(error)));
                    return;
                }
            }
        }
        None => None,
    };

    let mut out = String::new();
    let rc = run_capture(cmd, &args, input.as_deref(), &mut out);
    if let (0, Some((path, append))) = (rc, redirections.output) {
        let mut vfs = fs::VFS.lock();
        let data = out.replace("\r\n", "\n");
        let result = if append {
            vfs.append_file(&path, data.as_bytes())
        } else {
            vfs.write_file(&path, data.as_bytes())
        };
        if let Err(error) = result {
            print_line(&alloc::format!("bSH: {}: {}", path, fs_err(error)));
        }
    } else {
        print_str(&out);
    }
}

fn cmd_edit(args: &[&str]) {
    let (path, append) = match args {
        [path] => (*path, false),
        ["-a", path] => (*path, true),
        _ => return print_line("usage: edit [-a] <file>"),
    };
    print_line(if append {
        "Appending text. .save writes, .quit discards."
    } else {
        "Replacing text. .save writes, .quit discards."
    });
    print_line("Prefix a leading dot with another dot to enter it literally.");
    let mut text = String::new();
    loop {
        print_str("> ");
        let line = read_line();
        match line.as_str() {
            ".quit" => return,
            ".save" => {
                let result = {
                    let mut vfs = fs::VFS.lock();
                    if append {
                        vfs.append_file(path, text.as_bytes())
                    } else {
                        vfs.write_file(path, text.as_bytes())
                    }
                };
                match result {
                    Ok(()) => return print_line("Saved."),
                    Err(error) => print_line(&alloc::format!("edit: {}: {}", path, fs_err(error))),
                }
            }
            _ => {
                let line = if line.starts_with("..") { &line[1..] } else { &line };
                text.push_str(line);
                text.push('\n');
            }
        }
    }
}

/// Execute a command with captured output. Returns exit code.
/// `stdin_data` is the content of a `< file` redirect, if any.
fn run_capture(cmd: &str, args: &[&str], stdin_data: Option<&[u8]>, out: &mut String) -> i32 {
    match cmd {
        "" => 0,
        "help" => {
            cmd_help(args.first().copied(), out);
            0
        }
        "info" => {
            cmd_info(out);
            0
        }
        "uname" => {
            cmd_uname(args, out);
            0
        }
        "echo" => {
            emit(out, &args.join(" "));
            0
        }
        "whoami" => {
            emit(out, DEFAULT_USER);
            0
        }
        "pwd" => {
            emit(out, &fs::VFS.lock().get_cwd());
            0
        }
        "ls" => cmd_ls(args, out),
        "cat" => cmd_cat(args, stdin_data, out),
        "touch" => cmd_touch(args, out),
        "mkdir" => cmd_mkdir(args, out),
        "rmdir" => cmd_rmdir(args, out),
        "rm" => cmd_rm(args, out),
        "cp" => cmd_cp(args, out),
        "mv" => cmd_mv(args, out),
        "ln" => cmd_ln(args, out),
        "readlink" => cmd_readlink(args, out),
        "stat" => cmd_stat(args, out),
        "wc" => cmd_wc(args, stdin_data, out),
        "head" => cmd_head(args, stdin_data, out),
        "tail" => cmd_tail(args, stdin_data, out),
        "du" => cmd_du(args, out),
        "df" => cmd_df(args, out),
        "mount" => cmd_mount(args, out),
        "umount" => cmd_umount(args, out),
        "truncate" => cmd_truncate(args, out),
        "chmod" => cmd_chmod(args, out),
        "find" => cmd_find(args, out),
        "mem" => {
            let (htotal, hused) = heap_usage();
            emit(
                out,
                &alloc::format!(
                    "phys {} MB total / {} MB free / {} MB used; heap {} KiB total / {} KiB used",
                    phys::total_memory() / 1024 / 1024,
                    phys::free_memory() / 1024 / 1024,
                    phys::used_memory() / 1024 / 1024,
                    htotal / 1024,
                    hused / 1024,
                ),
            );
            0
        }
        "uptime" => {
            emit(out, &alloc::format!("ticks: {}", timer::ticks()));
            0
        }
        c => {
            emit(out, &alloc::format!("bSH: unknown command '{}' (try 'help')", c));
            127
        }
    }
}

fn emit(out: &mut String, s: &str) {
    out.push_str(s);
    out.push_str("\r\n");
}

// ---------------------------------------------------------------------------
// tokenizer + redirection
// ---------------------------------------------------------------------------

/// Split a line into tokens: whitespace-separated, with single/double
/// quotes and backslash escapes. `>`, `>>`, `<` become own tokens when
/// unquoted.
enum Token {
    Word(String),
    Input,
    Output { append: bool },
}

fn tokenize(line: &str) -> Result<Vec<Token>, &'static str> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        if let Some(q) = quote {
            has = true;
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' {
                cur.push(it.next().ok_or("bSH: incomplete escape")?);
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                has = true;
            }
            '\\' => {
                cur.push(it.next().ok_or("bSH: incomplete escape")?);
                has = true;
            }
            ' ' | '\t' => {
                if has {
                    toks.push(Token::Word(core::mem::take(&mut cur)));
                    has = false;
                }
            }
            '>' => {
                if has {
                    toks.push(Token::Word(core::mem::take(&mut cur)));
                    has = false;
                }
                let append = it.next_if_eq(&'>').is_some();
                toks.push(Token::Output { append });
            }
            '<' => {
                if has {
                    toks.push(Token::Word(core::mem::take(&mut cur)));
                    has = false;
                }
                toks.push(Token::Input);
            }
            _ => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quote.is_some() {
        return Err("bSH: unclosed quote");
    }
    if has {
        toks.push(Token::Word(cur));
    }
    Ok(toks)
}

#[derive(Default)]
struct Redirections {
    input: Option<String>,
    output: Option<(String, bool)>,
}

fn split_redir(toks: Vec<Token>) -> Result<(Vec<String>, Redirections), &'static str> {
    let mut words = Vec::new();
    let mut redirects = Redirections::default();
    let mut tokens = toks.into_iter();
    while let Some(token) = tokens.next() {
        let redirect = match token {
            Token::Word(word) => {
                words.push(word);
                continue;
            }
            redirect => redirect,
        };
        let Some(Token::Word(path)) = tokens.next() else {
            return Err("bSH: expected a file after redirection");
        };
        match redirect {
            Token::Input if redirects.input.is_none() => redirects.input = Some(path),
            Token::Output { append } if redirects.output.is_none() => {
                redirects.output = Some((path, append));
            }
            _ => return Err("bSH: duplicate redirection"),
        }
    }
    if words.is_empty() && (redirects.input.is_some() || redirects.output.is_some()) {
        return Err("bSH: expected a command");
    }
    Ok((words, redirects))
}

// ---------------------------------------------------------------------------
// help / info / uname
// ---------------------------------------------------------------------------

fn cmd_help(topic: Option<&str>, out: &mut String) {
    const LIST: &[(&str, &str)] = &[
        ("help", "help [cmd] — this text or details for one command"),
        ("info", "info — full system info"),
        ("uname", "uname [-a] — system name"),
        ("edit", "edit [-a] <file> — enter new text or append with -a"),
        ("echo", "echo <text...> — print arguments (supports > >>)"),
        ("pwd", "pwd — print working directory"),
        ("whoami", "whoami — show the current user"),
        ("cd", "cd [dir] — change directory, defaults to home"),
        ("ls", "ls [-l] [-a] [-h] [-d] [-R] [path...] — list directory"),
        ("cat", "cat [-n] <file...> — print files (supports < > >>)"),
        ("touch", "touch [-c] <file...> — create empty files"),
        ("mkdir", "mkdir [-p] <dir...> — create directories"),
        ("rmdir", "rmdir <dir...> — remove empty directories"),
        ("rm", "rm [-r] [-f] <path...> — remove files/dirs"),
        ("cp", "cp [-r] <src...> <dst> — copy files/dirs"),
        ("mv", "mv [-f] <src...> <dst> — move/rename"),
        ("ln", "ln -s <target> <link> — create symlink"),
        ("readlink", "readlink <link...> — print symlink targets"),
        ("stat", "stat <path...> — file status"),
        ("wc", "wc [-l] [-w] [-c] [file...] — counts"),
        ("head", "head [-n N] [file...] — first lines"),
        ("tail", "tail [-n N] [file...] — last lines"),
        ("du", "du [-h] [path...] — disk usage"),
        ("df", "df [-h] — mounted filesystems"),
        ("mount", "mount [dev mp type] — list or mount (ramfs, bFS)"),
        ("umount", "umount <mp> — unmount"),
        ("truncate", "truncate -s <size> <file...> — resize files"),
        ("chmod", "chmod <oct> <path...> — change permission bits"),
        ("find", "find [path] [-name pat] [-type f|d|l] — walk tree"),
        ("mem", "mem — memory stats"),
        ("uptime", "uptime — timer ticks"),
        ("clear", "clear — clear screen"),
        ("reboot", "reboot — reset via 8042"),
        ("halt", "halt — cli + hlt"),
    ];
    match topic {
        None => {
            emit(out, "commands:");
            for (_, desc) in LIST {
                emit(out, &alloc::format!("  {}", desc));
            }
            emit(out, "redirection: cmd > file | cmd >> file | cmd < file");
            emit(out, "quoting: 'single' \"double\" back\\slash work");
        }
        Some(t) => match LIST.iter().find(|(n, _)| *n == t) {
            Some((_, d)) => emit(out, d),
            None => emit(out, &alloc::format!("help: no such command '{}'", t)),
        },
    }
}

fn cmd_info(out: &mut String) {
    let (htotal, hused) = heap_usage();
    let vfs = fs::VFS.lock();
    let cwd = vfs.get_cwd();
    let mounts = vfs.mounts();
    drop(vfs);
    emit(out, "== System ==");
    emit(out, &alloc::format!("  OS:        bOS {} x86_64", KERNEL_VERSION));
    emit(out, "  Mode:      64-bit, higher-half, SMP");
    emit(out, "  Shell:     bSH (in-kernel, polling)");
    emit(out, "== Filesystem ==");
    emit(out, &alloc::format!("  Cwd:       {}", cwd));
    for m in &mounts {
        emit(
            out,
            &alloc::format!(
                "  Mount:     {} on {} type {}{}",
                m.device,
                m.mount_point,
                m.fs_type,
                if m.readonly { " (ro)" } else { "" }
            ),
        );
    }
    emit(out, "== Memory ==");
    emit(
        out,
        &alloc::format!(
            "  Phys:      {} MB total, {} MB free, {} MB used",
            phys::total_memory() / 1024 / 1024,
            phys::free_memory() / 1024 / 1024,
            phys::used_memory() / 1024 / 1024,
        ),
    );
    emit(out, &alloc::format!("  Heap:      {} KiB total, {} KiB used", htotal / 1024, hused / 1024));
    emit(out, "== Kernel ==");
    emit(out, "  Subsystems: GDT/IDT/PIC/APIC, phys+heap+paging, sched, VFS+bFS");
    emit(out, "  Syscalls:   0/1/2/3/4/8, 12/35/39/57/59/60, 76/79/80/82-84/87-90, 1000-1003");
}

fn cmd_uname(args: &[&str], out: &mut String) {
    if args.iter().any(|a| *a == "-a" || *a == "--all") {
        emit(out, &alloc::format!("bOS {} x86_64 bos_kernel bSH", KERNEL_VERSION));
    } else {
        emit(out, &alloc::format!("bOS {} x86_64", KERNEL_VERSION));
    }
}

// ---------------------------------------------------------------------------
// fs helpers
// ---------------------------------------------------------------------------

fn fs_err(e: FileSystemError) -> &'static str {
    match e {
        FileSystemError::NotFound => "No such file or directory",
        FileSystemError::PermissionDenied => "Permission denied",
        FileSystemError::InvalidArgument => "Invalid argument",
        FileSystemError::NoSpace => "No space left on device",
        FileSystemError::NotDirectory => "Not a directory",
        FileSystemError::IsDirectory => "Is a directory",
        FileSystemError::NotEmpty => "Directory not empty",
        FileSystemError::AlreadyExists => "File exists",
        FileSystemError::InvalidFormat => "Invalid format",
        FileSystemError::UnsupportedVersion => "Unsupported version",
        FileSystemError::IoError => "Input/output error",
        FileSystemError::NotImplemented => "Operation not supported",
        FileSystemError::NoSuchDevice => "No such device",
        FileSystemError::TooManyOpenFiles => "Too many open files",
        FileSystemError::BadFileDescriptor => "Bad file descriptor",
        FileSystemError::SymlinkLoop => "Too many levels of symbolic links",
        FileSystemError::NameTooLong => "File name too long",
        FileSystemError::ReadOnly => "Read-only file system",
    }
}

fn heap_usage() -> (usize, usize) {
    let s = heap::stats();
    (s.total_size, s.used_size)
}

fn parse_flags<'a>(args: &[&'a str]) -> (Vec<char>, Vec<&'a str>) {
    let mut flags = Vec::new();
    let mut rest = Vec::new();
    for a in args {
        if a.starts_with('-') && a.len() > 1 && !a[1..].chars().all(|c| c.is_ascii_digit()) {
            if *a == "--" {
                continue;
            }
            for c in a[1..].chars() {
                flags.push(c);
            }
        } else {
            rest.push(*a);
        }
    }
    (flags, rest)
}

fn has(f: &[char], c: char) -> bool {
    f.contains(&c)
}

fn human(n: u64) -> String {
    const U: &[&str] = &["B", "K", "M", "G"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < U.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        alloc::format!("{}{}", n, U[u])
    } else if v >= 100.0 {
        alloc::format!("{:.0}{}", v, U[u])
    } else {
        alloc::format!("{:.1}{}", v, U[u])
    }
}

fn mode_str(mode: u32, ftype: FileType) -> String {
    let mut s = String::with_capacity(10);
    s.push(match ftype {
        FileType::Directory => 'd',
        FileType::Symlink => 'l',
        FileType::BlockDevice => 'b',
        FileType::CharDevice => 'c',
        FileType::Fifo => 'p',
        FileType::Socket => 's',
        _ => '-',
    });
    const BITS: &[(u32, char)] =
        &[(0o400, 'r'), (0o200, 'w'), (0o100, 'x'), (0o040, 'r'), (0o020, 'w'), (0o010, 'x'), (0o004, 'r'), (0o002, 'w'), (0o001, 'x')];
    let perms = mode & 0o777;
    for (i, (b, c)) in BITS.iter().enumerate() {
        let _ = i;
        s.push(if perms & b != 0 { *c } else { '-' });
    }
    s
}

fn type_of_stat(mode: u32) -> FileType {
    match mode & fs::S_IFMT {
        fs::S_IFDIR => FileType::Directory,
        fs::S_IFLNK => FileType::Symlink,
        fs::S_IFCHR => FileType::CharDevice,
        fs::S_IFBLK => FileType::BlockDevice,
        fs::S_IFIFO => FileType::Fifo,
        _ => FileType::RegularFile,
    }
}

/// Join a parent absolute path and a child name.
fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        alloc::format!("/{}", name)
    } else {
        alloc::format!("{}/{}", parent, name)
    }
}

// ---------------------------------------------------------------------------
// file commands
// ---------------------------------------------------------------------------

fn cmd_cd(arg: Option<&str>) {
    let target = arg.unwrap_or(DEFAULT_HOME);
    match fs::VFS.lock().set_cwd(target) {
        Ok(()) => {}
        Err(e) => print_line(&alloc::format!("cd: {}: {}", target, fs_err(e))),
    }
}

fn cmd_ls(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let long = has(&flags, 'l');
    let all = has(&flags, 'a');
    let humanize = has(&flags, 'h');
    let dir_as_file = has(&flags, 'd');
    let recursive = has(&flags, 'R');
    for f in &flags {
        if !matches!(f, 'l' | 'a' | 'h' | 'd' | 'R') {
            emit(out, &alloc::format!("ls: invalid option -- '{}'", f));
            return 2;
        }
    }
    let paths: Vec<&str> = if rest.is_empty() { vec!["."] } else { rest };
    let mut rc = 0;
    let multi = paths.len() > 1 || recursive;
    for (pi, p) in paths.iter().enumerate() {
        // `ls file` shows the file itself; `ls dir` lists entries.
        let st = fs::VFS.lock().stat(p);
        match st {
            Ok(s) if dir_as_file || !s.is_dir() => {
                if long {
                    emit_ls_line(out, p, &s, None, humanize);
                } else {
                    emit(out, p);
                }
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                emit(out, &alloc::format!("ls: {}: {}", p, fs_err(e)));
                rc = 1;
                continue;
            }
        }
        // Breadth-first walk for -R (absolute paths, symlink-safe).
        let mut queue: Vec<String> = vec![fs::VFS.lock().abspath(p).unwrap_or_else(|_| String::from("/"))];
        let mut first = true;
        while let Some(dir) = queue.pop() {
            let entries = fs::VFS.lock().list_dir(&dir);
            let entries = match entries {
                Ok(e) => e,
                Err(e) => {
                    emit(out, &alloc::format!("ls: {}: {}", dir, fs_err(e)));
                    rc = 1;
                    continue;
                }
            };
            let mut names: Vec<(String, u64, FileType)> =
                entries.into_iter().map(|e| (e.name, e.inode, e.file_type)).collect();
            names.sort_by(|a, b| a.0.cmp(&b.0));
            if multi || !first {
                if pi > 0 || !first {
                    emit(out, "");
                }
                // show the user's spelling for the first dir, abs after
                if first {
                    emit(out, &alloc::format!("{}:", p));
                } else {
                    emit(out, &alloc::format!("{}:", dir));
                }
            }
            first = false;
            let mut subdirs: Vec<String> = Vec::new();
            for (name, ino, ft) in &names {
                if name == "." || name == ".." {
                    if !all {
                        continue;
                    }
                } else if !all && name.starts_with('.') {
                    continue;
                }
                if long {
                    let full = join(&dir, name);
                    let st = fs::VFS.lock().lstat(&full).unwrap_or(fs::FileStat {
                        inode: *ino,
                        size: 0,
                        mode: 0,
                        uid: 0,
                        gid: 0,
                        atime: 0,
                        mtime: 0,
                        ctime: 0,
                        links: 1,
                    });
                    let link = if *ft == FileType::Symlink {
                        fs::VFS.lock().readlink(&full).ok()
                    } else {
                        None
                    };
                    emit_ls_line(out, name, &st, link.as_deref(), humanize);
                } else {
                    emit(out, name);
                }
                if recursive && *ft == FileType::Directory && name != "." && name != ".." {
                    subdirs.push(join(&dir, name));
                }
            }
            // push in reverse so output stays sorted
            for s in subdirs.into_iter().rev() {
                queue.push(s);
            }
            if !recursive {
                break;
            }
        }
    }
    rc
}

fn emit_ls_line(out: &mut String, name: &str, st: &fs::FileStat, link: Option<&str>, h: bool) {
    let ft = type_of_stat(st.mode);
    let size = if h { human(st.size) } else { alloc::format!("{}", st.size) };
    let mut line = alloc::format!(
        "{} {:>3} {:>3} {:>3} {:>8} {}",
        mode_str(st.mode, ft),
        st.links,
        st.uid,
        st.gid,
        size,
        name
    );
    if let Some(t) = link {
        line.push_str(" -> ");
        line.push_str(t);
    }
    emit(out, &line);
}

fn cmd_cat(args: &[&str], stdin: Option<&[u8]>, out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let number = has(&flags, 'n');
    for f in &flags {
        if *f != 'n' {
            emit(out, &alloc::format!("cat: invalid option -- '{}'", f));
            return 2;
        }
    }
    let mut rc = 0;
    if rest.is_empty() {
        match stdin {
            Some(s) => dump_cat(out, s, number),
            None => {
                emit(out, "cat: missing operand (hint: cat <file> or cat < <file>)");
                return 2;
            }
        }
        return 0;
    }
    for p in rest {
        match fs::VFS.lock().read_file(p) {
            Ok(b) => dump_cat(out, &b, number),
            Err(e) => {
                emit(out, &alloc::format!("cat: {}: {}", p, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

/// Print file bytes to `out`, normalizing `\n` -> `\r\n` for the console.
/// With `number`, prefix each line like `cat -n`.
fn dump_cat(out: &mut String, bytes: &[u8], number: bool) {
    // Strip one trailing newline: `split('\n')` yields a phantom last piece.
    let text = String::from_utf8_lossy(bytes);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    if !number {
        for part in text.split_inclusive('\n') {
            if let Some(line) = part.strip_suffix('\n') {
                emit(out, line.trim_end_matches('\r'));
            } else {
                out.push_str(part);
            }
        }
        return;
    }
    for (i, ln) in body.split('\n').enumerate() {
        if body.is_empty() {
            break;
        }
        emit(out, &alloc::format!("{:>6}\t{}", i + 1, ln));
    }
}

fn cmd_touch(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let no_create = has(&flags, 'c');
    for f in &flags {
        if *f != 'c' {
            emit(out, &alloc::format!("touch: invalid option -- '{}'", f));
            return 2;
        }
    }
    if rest.is_empty() {
        emit(out, "touch: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in rest {
        let st = fs::VFS.lock().stat(p);
        match st {
            Ok(_) => {} // mtimes not stored; existing file is a no-op success
            Err(FileSystemError::NotFound) => {
                if no_create {
                    continue;
                }
                if let Err(e) = fs::VFS.lock().create(p, FileType::RegularFile) {
                    emit(out, &alloc::format!("touch: {}: {}", p, fs_err(e)));
                    rc = 1;
                }
            }
            Err(e) => {
                emit(out, &alloc::format!("touch: {}: {}", p, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_mkdir(args: &[&str], out: &mut String) -> i32 {
    // Manual parse: -p, -m <octal>, --, operands.
    let mut parents = false;
    let mut mode_bits: Option<u32> = None;
    let mut operands: Vec<&str> = Vec::new();
    let mut i = 0;
    let mut dashdash = false;
    while i < args.len() {
        let a = args[i];
        if !dashdash && a == "--" {
            dashdash = true;
        } else if !dashdash && a == "-p" {
            parents = true;
        } else if !dashdash && (a == "-m" || a.starts_with("-m")) {
            let val = if a == "-m" {
                i += 1;
                if i >= args.len() {
                    emit(out, "mkdir: -m needs an octal mode");
                    return 2;
                }
                args[i]
            } else {
                &a[2..]
            };
            match parse_octal(val) {
                Some(m) => mode_bits = Some(m),
                None => {
                    emit(out, &alloc::format!("mkdir: invalid mode '{}'", val));
                    return 2;
                }
            }
        } else if !dashdash && a.starts_with('-') && a.len() > 1 {
            emit(out, &alloc::format!("mkdir: invalid option '{}'", a));
            return 2;
        } else {
            operands.push(a);
        }
        i += 1;
    }
    if operands.is_empty() {
        emit(out, "mkdir: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in operands {
        let r = if parents { fs::VFS.lock().mkdir_p(p) } else { fs::VFS.lock().mkdir(p) };
        match r {
            Ok(()) => {
                if let Some(m) = mode_bits {
                    let _ = fs::VFS.lock().chmod(p, m);
                }
            }
            Err(e) => {
                emit(out, &alloc::format!("mkdir: {}: {}", p, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_rmdir(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.is_empty() {
        emit(out, "rmdir: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in rest {
        match fs::VFS.lock().stat(p) {
            Ok(s) if !s.is_dir() => {
                emit(out, &alloc::format!("rmdir: {}: Not a directory", p));
                rc = 1;
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                emit(out, &alloc::format!("rmdir: {}: {}", p, fs_err(e)));
                rc = 1;
                continue;
            }
        }
        if let Err(e) = fs::VFS.lock().remove(p) {
            emit(out, &alloc::format!("rmdir: {}: {}", p, fs_err(e)));
            rc = 1;
        }
    }
    rc
}

fn cmd_rm(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let rec = has(&flags, 'r') || has(&flags, 'R');
    let force = has(&flags, 'f');
    for f in &flags {
        if !matches!(f, 'r' | 'R' | 'f') {
            emit(out, &alloc::format!("rm: invalid option -- '{}'", f));
            return 2;
        }
    }
    if rest.is_empty() {
        if !force {
            emit(out, "rm: missing operand");
            return 2;
        }
        return 0;
    }
    let mut rc = 0;
    for p in rest {
        match rm_one(p, rec) {
            Ok(()) => {}
            Err(FileSystemError::NotFound) if force => {}
            Err(e) => {
                if !force {
                    emit(out, &alloc::format!("rm: {}: {}", p, fs_err(e)));
                }
                rc = 1;
            }
        }
    }
    rc
}

fn rm_one(path: &str, rec: bool) -> Result<(), FileSystemError> {
    let st = fs::VFS.lock().stat(path)?;
    if st.is_dir() {
        if !rec {
            return Err(FileSystemError::IsDirectory);
        }
        // refuse to remove mount points or /
        let abs = fs::VFS.lock().abspath(path)?;
        if abs == "/" {
            return Err(FileSystemError::InvalidArgument);
        }
        let mounts = fs::VFS.lock().mounts();
        for m in mounts.iter() {
            if m.mount_point == abs {
                return Err(FileSystemError::InvalidArgument);
            }
        }
        let kids: Vec<String> =
            fs::VFS.lock().list_dir(path)?.into_iter().map(|e| e.name).collect();
        for k in kids {
            if k == "." || k == ".." {
                continue;
            }
            rm_one(&join(&abs, &k), true)?;
        }
        fs::VFS.lock().remove(path)
    } else {
        fs::VFS.lock().remove(path)
    }
}

fn cmd_cp(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let rec = has(&flags, 'r') || has(&flags, 'R');
    for f in &flags {
        if !matches!(f, 'r' | 'R') {
            emit(out, &alloc::format!("cp: invalid option -- '{}'", f));
            return 2;
        }
    }
    if rest.len() < 2 {
        emit(out, "cp: missing operand (usage: cp [-r] <src...> <dst>)");
        return 2;
    }
    let (srcs, dst) = rest.split_at(rest.len() - 1);
    let dst = dst[0];
    let dst_is_dir = fs::VFS.lock().stat(dst).map(|s| s.is_dir()).unwrap_or(false);
    if srcs.len() > 1 && !dst_is_dir {
        emit(out, &alloc::format!("cp: target '{}': Not a directory", dst));
        return 1;
    }
    let mut rc = 0;
    for s in srcs {
        let target = if dst_is_dir {
            let base = s.trim_end_matches('/').rsplit('/').next().unwrap_or(s);
            join(&fs::VFS.lock().abspath(dst).unwrap_or_else(|_| String::from("/")), base)
        } else {
            dst.to_string()
        };
        if let Err(e) = cp_one(s, &target, rec) {
            emit(out, &alloc::format!("cp: {} -> {}: {}", s, target, fs_err(e)));
            rc = 1;
        }
    }
    rc
}

fn cp_one(src: &str, dst: &str, rec: bool) -> Result<(), FileSystemError> {
    let st = fs::VFS.lock().stat(src)?;
    if st.is_symlink() {
        let t = fs::VFS.lock().readlink(src)?;
        // replace destination
        let _ = fs::VFS.lock().remove(dst);
        return fs::VFS.lock().symlink(&t, dst);
    }
    if st.is_dir() {
        if !rec {
            return Err(FileSystemError::IsDirectory);
        }
        match fs::VFS.lock().mkdir(dst) {
            Ok(()) | Err(FileSystemError::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
        let kids = fs::VFS.lock().list_dir(src)?;
        for k in kids.iter() {
            // bFS stores "." / ".." on disk: never descend into them.
            if k.name == "." || k.name == ".." {
                continue;
            }
            let sabs = join(&fs::VFS.lock().abspath(src).unwrap_or_else(|_| String::from("/")), &k.name);
            let dabs = join(&fs::VFS.lock().abspath(dst).unwrap_or_else(|_| String::from("/")), &k.name);
            cp_one(&sabs, &dabs, true)?;
        }
        return Ok(());
    }
    let data = fs::VFS.lock().read_file(src)?;
    let _ = fs::VFS.lock().remove(dst);
    // create parent chain if needed
    let cr = fs::VFS.lock().create(dst, FileType::RegularFile);
    if let Err(e) = cr {
        if e != FileSystemError::AlreadyExists {
            // try mkdir -p on parent then retry
            let abs = fs::VFS.lock().abspath(dst)?;
            if let Some(pos) = abs.rfind('/') {
                if pos > 0 {
                    fs::VFS.lock().mkdir_p(&abs[..pos])?;
                    fs::VFS.lock().create(dst, FileType::RegularFile)?;
                } else {
                    return Err(e);
                }
            } else {
                return Err(e);
            }
        }
    }
    fs::VFS.lock().write_file(dst, &data)
}

fn cmd_mv(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    for f in &flags {
        if *f != 'f' {
            emit(out, &alloc::format!("mv: invalid option -- '{}'", f));
            return 2;
        }
    }
    if rest.len() < 2 {
        emit(out, "mv: missing operand (usage: mv <src...> <dst>)");
        return 2;
    }
    let (srcs, dst) = rest.split_at(rest.len() - 1);
    let dst = dst[0];
    let dst_is_dir = fs::VFS.lock().stat(dst).map(|s| s.is_dir()).unwrap_or(false);
    if srcs.len() > 1 && !dst_is_dir {
        emit(out, &alloc::format!("mv: target '{}': Not a directory", dst));
        return 1;
    }
    let mut rc = 0;
    for s in srcs {
        let target = if dst_is_dir {
            let base = s.trim_end_matches('/').rsplit('/').next().unwrap_or(s);
            join(&fs::VFS.lock().abspath(dst).unwrap_or_else(|_| String::from("/")), base)
        } else {
            dst.to_string()
        };
        // fast path: same-filesystem rename
        let rn = fs::VFS.lock().rename(s, &target);
        match rn {
            Ok(()) => {}
            Err(FileSystemError::NotImplemented) | Err(FileSystemError::InvalidArgument) => {
                // cross-mount or dir: copy + remove
                let st = fs::VFS.lock().stat(s);
                let st = match st {
                    Ok(st) => st,
                    Err(e) => {
                        emit(out, &alloc::format!("mv: {}: {}", s, fs_err(e)));
                        rc = 1;
                        continue;
                    }
                };
                let r = if st.is_dir() { cp_one(s, &target, true) } else { cp_one(s, &target, false) };
                match r {
                    Ok(()) => {
                        if let Err(e) = rm_one(s, true) {
                            emit(out, &alloc::format!("mv: {}: {}", s, fs_err(e)));
                            rc = 1;
                        }
                    }
                    Err(e) => {
                        emit(out, &alloc::format!("mv: {} -> {}: {}", s, target, fs_err(e)));
                        rc = 1;
                    }
                }
            }
            Err(e) => {
                emit(out, &alloc::format!("mv: {} -> {}: {}", s, target, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_ln(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let sym = has(&flags, 's');
    for f in &flags {
        if *f != 's' {
            emit(out, &alloc::format!("ln: invalid option -- '{}' (only -s supported)", f));
            return 2;
        }
    }
    if !sym {
        emit(out, "ln: only symbolic links are supported (usage: ln -s <target> <link>)");
        return 2;
    }
    if rest.len() != 2 {
        emit(out, "ln: usage: ln -s <target> <link>");
        return 2;
    }
    match fs::VFS.lock().symlink(rest[0], rest[1]) {
        Ok(()) => 0,
        Err(e) => {
            emit(out, &alloc::format!("ln: {}: {}", rest[1], fs_err(e)));
            1
        }
    }
}

fn cmd_readlink(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.is_empty() {
        emit(out, "readlink: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in rest {
        match fs::VFS.lock().readlink(p) {
            Ok(t) => emit(out, &t),
            Err(FileSystemError::InvalidArgument) => {
                emit(out, &alloc::format!("readlink: {}: Not a symlink", p));
                rc = 1;
            }
            Err(e) => {
                emit(out, &alloc::format!("readlink: {}: {}", p, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_stat(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.is_empty() {
        emit(out, "stat: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in rest {
        let lst = fs::VFS.lock().lstat(p);
        match lst {
            Ok(s) => {
                let kind = match s.mode & fs::S_IFMT {
                    fs::S_IFDIR => "directory",
                    fs::S_IFLNK => "symbolic link",
                    fs::S_IFCHR => "character device",
                    fs::S_IFBLK => "block device",
                    fs::S_IFIFO => "fifo",
                    _ => "regular file",
                };
                emit(out, &alloc::format!("  File: {}", p));
                emit(out, &alloc::format!("  Size: {:<12} Inode: {}", s.size, s.inode));
                emit(
                    out,
                    &alloc::format!(
                        "Access: {:04o} ({})  Uid: {}  Gid: {}",
                        s.mode & 0o7777,
                        mode_str(s.mode, type_of_stat(s.mode)),
                        s.uid,
                        s.gid
                    ),
                );
                emit(out, &alloc::format!("  Type: {}", kind));
                if s.is_symlink() {
                    match fs::VFS.lock().readlink(p) {
                        Ok(t) => emit(out, &alloc::format!("Target: {}", t)),
                        Err(_) => {}
                    }
                }
            }
            Err(e) => {
                emit(out, &alloc::format!("stat: {}: {}", p, fs_err(e)));
                rc = 1;
            }
        }
    }
    rc
}

fn input_bytes(operands: &[&str], stdin: Option<&[u8]>, out: &mut String) -> Option<Vec<(String, Vec<u8>)>> {
    if operands.is_empty() {
        match stdin {
            Some(s) => Some(vec![(String::from("<stdin>"), s.to_vec())]),
            None => {
                emit(out, "missing operand (hint: pass a file or use < file)");
                None
            }
        }
    } else {
        let mut v = Vec::new();
        for p in operands {
            match fs::VFS.lock().read_file(p) {
                Ok(b) => v.push((p.to_string(), b)),
                Err(e) => {
                    emit(out, &alloc::format!("{}: {}", p, fs_err(e)));
                    return None;
                }
            }
        }
        Some(v)
    }
}

fn cmd_wc(args: &[&str], stdin: Option<&[u8]>, out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let mut show_l = false;
    let mut show_w = false;
    let mut show_c = false;
    for f in &flags {
        match f {
            'l' => show_l = true,
            'w' => show_w = true,
            'c' => show_c = true,
            _ => {
                emit(out, &alloc::format!("wc: invalid option -- '{}'", f));
                return 2;
            }
        }
    }
    if !show_l && !show_w && !show_c {
        show_l = true;
        show_w = true;
        show_c = true;
    }
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    let mut tl = 0u64;
    let mut tw = 0u64;
    let mut tc = 0u64;
    for (name, data) in &inputs {
        let l = data.iter().filter(|&&b| b == b'\n').count() as u64;
        let w = String::from_utf8_lossy(data).split_whitespace().count() as u64;
        let c = data.len() as u64;
        tl += l;
        tw += w;
        tc += c;
        let mut line = String::new();
        if show_l {
            line.push_str(&alloc::format!("{:>7}", l));
        }
        if show_w {
            line.push_str(&alloc::format!("{:>8}", w));
        }
        if show_c {
            line.push_str(&alloc::format!("{:>8}", c));
        }
        if inputs.len() > 1 || !rest.is_empty() {
            line.push(' ');
            line.push_str(name);
        }
        emit(out, line.trim_start());
    }
    if inputs.len() > 1 {
        let mut line = String::new();
        if show_l {
            line.push_str(&alloc::format!("{:>7}", tl));
        }
        if show_w {
            line.push_str(&alloc::format!("{:>8}", tw));
        }
        if show_c {
            line.push_str(&alloc::format!("{:>8}", tc));
        }
        line.push_str(" total");
        emit(out, line.trim_start());
    }
    0
}

fn parse_head_n<'a>(args: &[&'a str]) -> Result<(u64, Vec<&'a str>), String> {
    let mut n = 10u64;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "-n" {
            i += 1;
            if i >= args.len() {
                return Err(String::from("head: -n needs a number"));
            }
            n = args[i].parse::<u64>().map_err(|_| alloc::format!("head: invalid number '{}'", args[i]))?;
        } else if a.starts_with("-n") && a.len() > 2 {
            n = a[2..].parse::<u64>().map_err(|_| alloc::format!("head: invalid number '{}'", a))?;
        } else if a.starts_with('-') && a.len() > 1 && a[1..].chars().all(|c| c.is_ascii_digit()) {
            n = a[1..].parse::<u64>().map_err(|_| alloc::format!("head: invalid number '{}'", a))?;
        } else {
            rest.push(a);
        }
        i += 1;
    }
    Ok((n, rest))
}

fn cmd_head(args: &[&str], stdin: Option<&[u8]>, out: &mut String) -> i32 {
    let (n, rest) = match parse_head_n(args) {
        Ok(v) => v,
        Err(e) => {
            emit(out, &e);
            return 2;
        }
    };
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    for (name, data) in &inputs {
        if inputs.len() > 1 {
            emit(out, &alloc::format!("==> {} <==", name));
        }
        let text = String::from_utf8_lossy(data);
        for ln in text.lines().take(n as usize) {
            emit(out, ln);
        }
    }
    0
}

fn cmd_tail(args: &[&str], stdin: Option<&[u8]>, out: &mut String) -> i32 {
    let (n, rest) = match parse_head_n(args) {
        Ok(v) => v,
        Err(e) => {
            emit(out, &e.replace("head", "tail"));
            return 2;
        }
    };
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    for (name, data) in &inputs {
        if inputs.len() > 1 {
            emit(out, &alloc::format!("==> {} <==", name));
        }
        let text = String::from_utf8_lossy(data);
        let lines: Vec<&str> = text.lines().collect();
        let skip = lines.len().saturating_sub(n as usize);
        for ln in &lines[skip..] {
            emit(out, ln);
        }
    }
    0
}

fn du_path(abs: &str, depth: usize) -> u64 {
    if depth > 32 {
        return 0;
    }
    let st = fs::VFS.lock().stat(abs);
    let st = match st {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if !st.is_dir() {
        return st.size;
    }
    let mut total = 0u64;
    let kids = fs::VFS.lock().list_dir(abs);
    if let Ok(kids) = kids {
        for k in kids {
            // bFS stores "." / ".." on disk: skip to avoid infinite walk.
            if k.name == "." || k.name == ".." {
                continue;
            }
            total += du_path(&join(abs, &k.name), depth + 1);
        }
    }
    total
}

fn cmd_du(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = parse_flags(args);
    let h = has(&flags, 'h');
    for f in &flags {
        if *f != 'h' {
            emit(out, &alloc::format!("du: invalid option -- '{}'", f));
            return 2;
        }
    }
    let paths: Vec<&str> = if rest.is_empty() { vec!["."] } else { rest };
    let mut rc = 0;
    for p in paths {
        let abs = fs::VFS.lock().abspath(p);
        let abs = match abs {
            Ok(a) => a,
            Err(e) => {
                emit(out, &alloc::format!("du: {}: {}", p, fs_err(e)));
                rc = 1;
                continue;
            }
        };
        let bytes = du_path(&abs, 0);
        let show = if h { human(bytes) } else { alloc::format!("{}", bytes / 1024 + 1) };
        // size in 1K blocks unless -h
        if h {
            emit(out, &alloc::format!("{}\t{}", show, p));
        } else {
            emit(out, &alloc::format!("{}\t{}", show, p));
        }
    }
    rc
}

fn cmd_df(args: &[&str], out: &mut String) -> i32 {
    let (flags, _) = parse_flags(args);
    let h = has(&flags, 'h');
    for f in &flags {
        if *f != 'h' {
            emit(out, &alloc::format!("df: invalid option -- '{}'", f));
            return 2;
        }
    }
    emit(out, "Filesystem     1K-blocks    Used   Avail Use% Mounted on");
    let mounts = fs::VFS.lock().mounts();
    for m in mounts.iter() {
        // Precise block accounting lives inside each FS; approximate with a
        // recursive walk plus a capacity hint (bFS image size vs heap).
        let used = du_path(&m.mount_point, 0);
        let cap = if m.fs_type == "bFS" {
            (16 * 1024 * 1024u64).max(used)
        } else {
            let (ht, hu) = heap_usage();
            (ht as u64).max(used + hu as u64).max(1)
        };
        let free = cap - used.min(cap);
        let used = cap - free;
        let pct = if cap == 0 { 0 } else { used * 100 / cap };
        let fmt = |b: u64| if h { human(b) } else { alloc::format!("{}", b / 1024) };
        emit(
            out,
            &alloc::format!(
                "{:<13} {:>9} {:>7} {:>7} {:>3}% {}",
                m.device,
                fmt(cap),
                fmt(used),
                fmt(free),
                pct,
                m.mount_point
            ),
        );
    }
    0
}

fn cmd_mount(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.is_empty() {
        let mounts = fs::VFS.lock().mounts();
        for m in mounts.iter() {
            emit(
                out,
                &alloc::format!(
                    "{} on {} type {}{}",
                    m.device,
                    m.mount_point,
                    m.fs_type,
                    if m.readonly { " (ro)" } else { "" }
                ),
            );
        }
        return 0;
    }
    if rest.len() != 3 {
        emit(out, "mount: usage: mount <device> <mountpoint> <ramfs|bFS>");
        return 2;
    }
    let (dev, mp, typ) = (rest[0], rest[1], rest[2]);
    if !matches!(typ, "ramfs" | "bFS" | "bfs") {
        emit(out, &alloc::format!("mount: unknown filesystem '{}'", typ));
        return 1;
    }
    // ensure mountpoint exists (mkdir -p best effort)
    let _ = fs::VFS.lock().mkdir_p(mp);
    match fs::VFS.lock().mount(dev, mp, typ, "") {
        Ok(()) => 0,
        Err(e) => {
            emit(out, &alloc::format!("mount: {}: {}", mp, fs_err(e)));
            1
        }
    }
}

fn cmd_umount(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.len() != 1 {
        emit(out, "umount: usage: umount <mountpoint>");
        return 2;
    }
    match fs::VFS.lock().unmount(rest[0]) {
        Ok(()) => 0,
        Err(e) => {
            emit(out, &alloc::format!("umount: {}: {}", rest[0], fs_err(e)));
            1
        }
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = match s.chars().last()? {
        'K' | 'k' => (&s[..s.len() - 1], 1024u64),
        'M' | 'm' => (&s[..s.len() - 1], 1024 * 1024),
        'G' | 'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        c if c.is_ascii_digit() => (s, 1),
        _ => return None,
    };
    num.parse::<u64>().ok()?.checked_mul(mult)
}

fn cmd_truncate(args: &[&str], out: &mut String) -> i32 {
    let mut size: Option<u64> = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-s" {
            i += 1;
            if i >= args.len() {
                emit(out, "truncate: -s needs a size");
                return 2;
            }
            size = parse_size(args[i]);
            if size.is_none() {
                emit(out, &alloc::format!("truncate: invalid size '{}'", args[i]));
                return 2;
            }
        } else if args[i].starts_with("-s") && args[i].len() > 2 {
            size = parse_size(&args[i][2..]);
            if size.is_none() {
                emit(out, &alloc::format!("truncate: invalid size '{}'", args[i]));
                return 2;
            }
        } else {
            rest.push(args[i]);
        }
        i += 1;
    }
    let size = match size {
        Some(s) => s,
        None => {
            emit(out, "truncate: usage: truncate -s <size> <file...>");
            return 2;
        }
    };
    if rest.is_empty() {
        emit(out, "truncate: missing operand");
        return 2;
    }
    let mut rc = 0;
    for p in rest {
        // create-if-missing, then resize (GNU behavior)
        if fs::VFS.lock().stat(p).is_err() {
            if let Err(e) = fs::VFS.lock().create(p, FileType::RegularFile) {
                emit(out, &alloc::format!("truncate: {}: {}", p, fs_err(e)));
                rc = 1;
                continue;
            }
        }
        if let Err(e) = fs::VFS.lock().truncate(p, size) {
            emit(out, &alloc::format!("truncate: {}: {}", p, fs_err(e)));
            rc = 1;
        }
    }
    rc
}

fn parse_octal(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 5 || !s.chars().all(|c| ('0'..='7').contains(&c)) {
        return None;
    }
    u32::from_str_radix(s, 8).ok()
}

fn cmd_chmod(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = parse_flags(args);
    if rest.len() < 2 {
        emit(out, "chmod: usage: chmod <octal-mode> <path...>");
        return 2;
    }
    let mode = match parse_octal(rest[0]) {
        Some(m) => m,
        None => {
            emit(out, &alloc::format!("chmod: invalid mode '{}'", rest[0]));
            return 2;
        }
    };
    let mut rc = 0;
    for p in &rest[1..] {
        if let Err(e) = fs::VFS.lock().chmod(p, mode) {
            emit(out, &alloc::format!("chmod: {}: {}", p, fs_err(e)));
            rc = 1;
        }
    }
    rc
}

fn glob_match(pat: &str, name: &str) -> bool {
    // `*` and `?` matcher (linear, no allocations beyond recursion).
    let (px, nx) = (pat.as_bytes(), name.as_bytes());
    // iterative with backtracking on '*'
    let (mut star, mut mark) = (None::<usize>, 0usize);
    let (mut i, mut j) = (0usize, 0usize);
    while j < nx.len() {
        if i < px.len() && (px[i] == b'?' || px[i] == nx[j]) {
            i += 1;
            j += 1;
        } else if i < px.len() && px[i] == b'*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < px.len() && px[i] == b'*' {
        i += 1;
    }
    i == px.len()
}

fn cmd_find(args: &[&str], out: &mut String) -> i32 {
    let mut roots: Vec<&str> = Vec::new();
    let mut name_pat: Option<&str> = None;
    let mut type_f: Option<char> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-name" => {
                i += 1;
                if i >= args.len() {
                    emit(out, "find: -name needs a pattern");
                    return 2;
                }
                name_pat = Some(args[i]);
            }
            "-type" => {
                i += 1;
                if i >= args.len() {
                    emit(out, "find: -type needs f, d or l");
                    return 2;
                }
                let t = args[i].chars().next().unwrap_or('?');
                if !matches!(t, 'f' | 'd' | 'l') {
                    emit(out, &alloc::format!("find: unknown type '{}'", args[i]));
                    return 2;
                }
                type_f = Some(t);
            }
            a => roots.push(a),
        }
        i += 1;
    }
    if roots.is_empty() {
        roots.push(".");
    }
    let mut rc = 0;
    for r in roots {
        let abs = fs::VFS.lock().abspath(r);
        let abs = match abs {
            Ok(a) => a,
            Err(e) => {
                emit(out, &alloc::format!("find: {}: {}", r, fs_err(e)));
                rc = 1;
                continue;
            }
        };
        find_walk(&abs, name_pat, type_f, 0, out);
    }
    rc
}

fn find_walk(abs: &str, pat: Option<&str>, typ: Option<char>, depth: usize, out: &mut String) {
    if depth > 32 {
        return;
    }
    let st = fs::VFS.lock().stat(abs);
    let st = match st {
        Ok(s) => s,
        Err(_) => return,
    };
    let is_link = st.is_symlink();
    let kind = if is_link {
        'l'
    } else if st.is_dir() {
        'd'
    } else {
        'f'
    };
    let base = abs.rsplit('/').next().unwrap_or(abs);
    let name_ok = pat.map(|p| glob_match(p, base)).unwrap_or(true);
    let type_ok = typ.map(|t| t == kind).unwrap_or(true);
    if name_ok && type_ok {
        emit(out, abs);
    }
    // do not descend into symlinks (loop safety)
    if st.is_dir() && !is_link {
        let kids = fs::VFS.lock().list_dir(abs);
        if let Ok(kids) = kids {
            let mut names: Vec<String> = kids.into_iter().map(|e| e.name).collect();
            names.sort();
            for k in names {
                if k == "." || k == ".." {
                    continue;
                }
                find_walk(&join(abs, &k), pat, typ, depth + 1, out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// console plumbing (unchanged)
// ---------------------------------------------------------------------------

fn print_str(s: &str) {
    console::print(s);
}

fn print_line(s: &str) {
    console::print(s);
    console::print("\r\n");
}

/// Read one line, polling serial COM1 and the PS/2 keyboard.
/// Echoes input; handles CR/LF, backspace. Returns without the newline.
fn read_line() -> String {
    let mut line = String::new();
    let mut shift = false;
    let mut ext = false;
    loop {
        // Serial first (QEMU -serial stdio feeds keystrokes here).
        if let Some(b) = serial::try_read_byte() {
            match b {
                b'\r' | b'\n' => {
                    print_str("\r\n");
                    break;
                }
                0x08 | 0x7f => {
                    if line.pop().is_some() {
                        print_str("\x08 \x08");
                    }
                }
                0x20..=0x7e => {
                    if line.len() < 1024 {
                        line.push(b as char);
                        serial::write_byte(b);
                    }
                }
                _ => {}
            }
            continue;
        }
        // PS/2 Set-1 scancodes.
        if let Some(sc) = keyboard::try_read() {
            if ext {
                ext = false;
                continue;
            }
            match sc {
                0xE0 => ext = true,
                0x2A | 0x36 => shift = true,
                0xAA | 0xB6 => shift = false,
                0x1C => {
                    print_str("\r\n");
                    break;
                }
                0x0E => {
                    if line.pop().is_some() {
                        print_str("\x08 \x08");
                    }
                }
                s if s >= 0x80 => {}
                s => {
                    if let Some(ch) = decode(s, shift) {
                        if line.len() < 1024 {
                            line.push(ch);
                            print_str(ch.encode_utf8(&mut [0; 4]));
                        }
                    }
                }
            }
            continue;
        }
        core::hint::spin_loop();
    }
    line
}

/// PS/2 Set-1 press-code -> char (letters, digits, space and basics).
fn decode(sc: u8, shift: bool) -> Option<char> {
    const ROW: &[(u8, char, char)] = &[
        (0x02, '1', '!'), (0x03, '2', '@'), (0x04, '3', '#'), (0x05, '4', '$'),
        (0x06, '5', '%'), (0x07, '6', '^'), (0x08, '7', '&'), (0x09, '8', '*'),
        (0x0A, '9', '('), (0x0B, '0', ')'), (0x0C, '-', '_'), (0x0D, '=', '+'),
        (0x0F, '\t', '\t'), (0x10, 'q', 'Q'), (0x11, 'w', 'W'), (0x12, 'e', 'E'),
        (0x13, 'r', 'R'), (0x14, 't', 'T'), (0x15, 'y', 'Y'), (0x16, 'u', 'U'),
        (0x17, 'i', 'I'), (0x18, 'o', 'O'), (0x19, 'p', 'P'), (0x1A, '[', '{'),
        (0x1B, ']', '}'), (0x1E, 'a', 'A'), (0x1F, 's', 'S'), (0x20, 'd', 'D'),
        (0x21, 'f', 'F'), (0x22, 'g', 'G'), (0x23, 'h', 'H'), (0x24, 'j', 'J'),
        (0x25, 'k', 'K'), (0x26, 'l', 'L'), (0x27, ';', ':'), (0x28, '\'', '"'),
        (0x29, '`', '~'), (0x2B, '\\', '|'), (0x2C, 'z', 'Z'), (0x2D, 'x', 'X'),
        (0x2E, 'c', 'C'), (0x2F, 'v', 'V'), (0x30, 'b', 'B'), (0x31, 'n', 'N'),
        (0x32, 'm', 'M'), (0x33, ',', '<'), (0x34, '.', '>'), (0x35, '/', '?'),
        (0x39, ' ', ' '),
    ];
    for (code, lo, hi) in ROW {
        if *code == sc {
            return Some(if shift { *hi } else { *lo });
        }
    }
    None
}

fn reboot() -> ! {
    unsafe {
        Port::<u8>::new(0x64).write(0xFE);
    }
    loop {
        core::hint::spin_loop();
    }
}

fn halt() -> ! {
    unsafe {
        core::arch::asm!("cli");
    }
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}
