//! fsutil — shared Linux-like file utilities for bOS userspace.
//!
//! Compiled into both `bsh` (interactive shell) and `bcoreutils`
//! (multicall binary) via `#[path]`. Std-only, no external crates.
//! Each util is `run_<name>(args, stdin) -> (output, exit_code)` so the
//! shell can capture output for `>` / `>>` redirection.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const BOS_NAME: &str = "bOS";
pub const BOS_VERSION: &str = "0.1.0";
pub const TARGET_ARCH: &str = "x86_64";

pub const COMMANDS: &[(&str, &str)] = &[
    ("help", "help [cmd] — this text or details for one util"),
    ("info", "info [section] — full system info"),
    ("uname", "uname [-a] — system name"),
    ("echo", "echo <text...> — print arguments"),
    ("pwd", "pwd — print working directory"),
    ("ls", "ls [-l] [-a] [-h] [-d] [-R] [path...] — list directory"),
    ("cat", "cat [-n] <file...> — print files"),
    ("touch", "touch [-c] <file...> — create empty files"),
    ("mkdir", "mkdir [-p] [-m mode] <dir...> — create directories"),
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
    ("du", "du [-h] [-s] [path...] — disk usage"),
    ("df", "df [-h] — mounted filesystems"),
    ("find", "find [path...] [-name pat] [-type f|d|l] — walk tree"),
    ("chmod", "chmod <oct> <path...> — change permission bits"),
    ("truncate", "truncate -s <size> <file...> — resize files"),
];

/// Dispatch one util. `stdin` is `< file` content when the shell provides it.
pub fn run(cmd: &str, args: &[&str], stdin: Option<&str>) -> (String, i32) {
    let mut out = String::new();
    let rc = match cmd {
        "help" => cmd_help(args.first().copied(), &mut out),
        "info" => cmd_info(&args[..], &mut out),
        "uname" => cmd_uname(args, &mut out),
        "echo" => {
            out.push_str(&args.join(" "));
            out.push('\n');
            0
        }
        "pwd" => match std::env::current_dir() {
            Ok(p) => {
                out.push_str(&p.display().to_string());
                out.push('\n');
                0
            }
            Err(e) => err(&mut out, &format!("pwd: {}", e), 1),
        },
        "ls" => cmd_ls(args, &mut out),
        "cat" => cmd_cat(args, stdin, &mut out),
        "touch" => cmd_touch(args, &mut out),
        "mkdir" => cmd_mkdir(args, &mut out),
        "rmdir" => cmd_rmdir(args, &mut out),
        "rm" => cmd_rm(args, &mut out),
        "cp" => cmd_cp(args, &mut out),
        "mv" => cmd_mv(args, &mut out),
        "ln" => cmd_ln(args, &mut out),
        "readlink" => cmd_readlink(args, &mut out),
        "stat" => cmd_stat(args, &mut out),
        "wc" => cmd_wc(args, stdin, &mut out),
        "head" => cmd_head(args, stdin, &mut out),
        "tail" => cmd_tail(args, stdin, &mut out),
        "du" => cmd_du(args, &mut out),
        "df" => cmd_df(args, &mut out),
        "find" => cmd_find(args, &mut out),
        "chmod" => cmd_chmod(args, &mut out),
        "truncate" => cmd_truncate(args, &mut out),
        "version" | "ver" | "--version" => {
            out.push_str(&format!("{BOS_NAME} {BOS_VERSION} {TARGET_ARCH}\n"));
            0
        }
        _ => err(&mut out, &format!("unknown command '{cmd}' (try 'help')"), 127),
    };
    (out, rc)
}

fn err(out: &mut String, msg: &str, rc: i32) -> i32 {
    out.push_str(msg);
    out.push('\n');
    rc
}

fn emit(out: &mut String, s: &str) {
    out.push_str(s);
    out.push('\n');
}

// ---------------------------------------------------------------------------
// shell word-splitting shared with bsh
// ---------------------------------------------------------------------------

/// Split a line into tokens with quotes/escapes; `>`, `>>`, `<` split.
#[allow(dead_code)] // used by bsh through the shared include
pub fn tokenize(line: &str) -> Vec<String> {
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
                if let Some(n) = it.next() {
                    cur.push(n);
                }
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
                if let Some(n) = it.next() {
                    cur.push(n);
                    has = true;
                }
            }
            ' ' | '\t' => {
                if has {
                    toks.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            '>' => {
                if has {
                    toks.push(std::mem::take(&mut cur));
                    has = false;
                }
                if it.peek() == Some(&'>') {
                    it.next();
                    toks.push(">>".into());
                } else {
                    toks.push(">".into());
                }
            }
            '<' => {
                if has {
                    toks.push(std::mem::take(&mut cur));
                    has = false;
                }
                toks.push("<".into());
            }
            _ => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        toks.push(cur);
    }
    toks
}

// ---------------------------------------------------------------------------
// help / info / uname
// ---------------------------------------------------------------------------

fn cmd_help(topic: Option<&str>, out: &mut String) -> i32 {
    match topic {
        None => {
            emit(out, &format!("{BOS_NAME} utils {BOS_VERSION} — available commands:"));
            for (_, desc) in COMMANDS {
                emit(out, &format!("  {desc}"));
            }
        }
        Some(t) => match COMMANDS.iter().find(|(n, _)| *n == t) {
            Some((_, d)) => emit(out, d),
            None => emit(out, &format!("help: no such command '{t}'")),
        },
    }
    0
}

fn cmd_info(args: &[&str], out: &mut String) -> i32 {
    let full = args.iter().any(|a| *a == "--full" || *a == "-f");
    let sections: Vec<&str> =
        args.iter().copied().filter(|a| !a.starts_with('-')).collect();
    let show =
        |name: &str| full || sections.is_empty() || sections.contains(&"all") || sections.contains(&name);
    for s in &sections {
        if !matches!(
            *s,
            "all" | "system" | "kernel" | "boot" | "cpu" | "mem" | "fs" | "syscalls"
                | "compat" | "drivers" | "userspace" | "host"
        ) {
            emit(out, "valid sections: system kernel boot cpu mem fs syscalls compat drivers userspace host all");
            return 1;
        }
    }
    if show("system") {
        emit(out, "== System ==");
        emit(out, &format!("  OS               {BOS_NAME} {BOS_VERSION}"));
        emit(out, &format!("  Arch/mode        {TARGET_ARCH} (64-bit, higher-half, SMP)"));
        emit(out, "  Syscall ABI      int 0x80 + syscall/sysret");
        emit(out, "  Filesystem       bFS (bOS File System) + VFS");
    }
    if show("kernel") {
        emit(out, "== Kernel ==");
        emit(out, &format!("  Crate            bos_kernel v{BOS_VERSION} (#![no_std], x86_64)"));
        emit(out, "  Load address     0xFFFF_8000_0000_0000 (higher-half)");
        emit(out, "  Subsystems       GDT/IDT/PIC/APIC, mm/phys+heap+paging, sched, proc, fs/VFS+bFS, drivers, syscall, ipc, compat");
    }
    if show("boot") {
        emit(out, "== Boot ==");
        emit(out, "  Loader           bLOADER (UEFI)");
        emit(out, "  ESP              EFI/BOOT/BOOTX64.EFI, EFI/bOS/kernel.bin, EFI/bOS/initrd.img");
    }
    if show("cpu") {
        let ncpu =
            std::thread::available_parallelism().map(|n| n.get().to_string()).unwrap_or_else(|_| "?".into());
        emit(out, "== CPU / Arch ==");
        emit(out, &format!("  Target           {} 64-bit", std::env::consts::ARCH));
        emit(out, "  Page size        4 KiB");
        emit(out, &format!("  Host CPUs        {ncpu}"));
    }
    if show("mem") {
        emit(out, "== Memory ==");
        emit(out, "  Model            UEFI map → phys allocator → higher-half 4K paging → heap");
        emit(out, "  sbrk             syscall 12");
    }
    if show("fs") {
        emit(out, "== Filesystem ==");
        emit(out, "  Stack            VFS → bFS (+ initrd.img) / ramfs on /");
        emit(out, "  Host tools       fs/bfs: mkfs/fsck + file ops");
        emit(out, "  Syscalls         2 open, 3 close, 0 read, 1 write, 4 stat, 8 lseek, 76 truncate, 79 getcwd, 80 chdir, 82 rename, 83 mkdir, 84 rmdir, 87 unlink, 88 symlink, 89 readlink, 90 chmod, 1000 listdir");
    }
    if show("syscalls") {
        emit(out, "== Syscalls (int 0x80) ==");
        emit(out, "  0 read / 1 write / 2 open / 3 close / 4 stat / 8 lseek");
        emit(out, "  12 sbrk / 35 sleep / 39 getpid / 57 fork / 59 exec / 60 exit");
        emit(out, "  76 truncate / 79 getcwd / 80 chdir / 82 rename / 83 mkdir / 84 rmdir");
        emit(out, "  87 unlink / 88 symlink / 89 readlink / 90 chmod");
        emit(out, "  1000 listdir / 1001 loadmod / 1002 net_send / 1003 net_recv");
    }
    if show("compat") {
        emit(out, "== Compat (native, no emulation) ==");
        emit(out, "  Linux ELF / Windows PE (+Win32 shim) / macOS Mach-O (+libSystem shim)");
    }
    if show("drivers") {
        emit(out, "== Drivers ==");
        emit(out, "  Input: kbd/mouse; Storage: ata/ahci/nvme/block; Net: e1000/virtio/stub");
    }
    if show("userspace") {
        emit(out, "== Userspace ==");
        emit(out, "  binit (PID1), bsh (shell), bcoreutils (multicall)");
    }
    if show("host") {
        emit(out, "== Host runtime ==");
        emit(
            out,
            &format!(
                "  OS/family        {}/{}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::env::consts::FAMILY
            ),
        );
        emit(out, &format!("  PID                {}", std::process::id()));
    }
    0
}

fn cmd_uname(args: &[&str], out: &mut String) -> i32 {
    if args.iter().any(|a| *a == "-a" || *a == "--all") {
        emit(out, &format!("{BOS_NAME} {BOS_VERSION} {TARGET_ARCH} bos_kernel bLOADER"));
    } else {
        emit(out, &format!("{BOS_NAME} {BOS_VERSION} {TARGET_ARCH}"));
    }
    0
}

// ---------------------------------------------------------------------------
// flag helpers
// ---------------------------------------------------------------------------

fn split_flags<'a>(args: &'a [&'a str]) -> (Vec<char>, Vec<&'a str>) {
    let mut flags = Vec::new();
    let mut rest = Vec::new();
    for a in args {
        if a.starts_with('-') && a.len() > 1 && !a[1..].chars().all(|c| c.is_ascii_digit()) && *a != "-" {
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
    const U: &[&str] = &["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < U.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{}{}", n, U[u])
    } else if v >= 100.0 {
        format!("{:.0}{}", v, U[u])
    } else {
        format!("{:.1}{}", v, U[u])
    }
}

#[cfg(unix)]
fn mode_bits(md: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode()
}
#[cfg(not(unix))]
fn mode_bits(md: &fs::Metadata) -> u32 {
    if md.is_dir() {
        0o040755
    } else {
        0o100644
    }
}

fn mode_str(mode: u32, is_dir: bool, is_link: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_link {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    });
    const B: &[(u32, char)] = &[
        (0o400, 'r'), (0o200, 'w'), (0o100, 'x'), (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
        (0o004, 'r'), (0o002, 'w'), (0o001, 'x'),
    ];
    for (b, c) in B {
        s.push(if mode & b != 0 { *c } else { '-' });
    }
    s
}

fn parse_octal(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 5 || !s.chars().all(|c| ('0'..='7').contains(&c)) {
        return None;
    }
    u32::from_str_radix(s, 8).ok()
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last()? {
        'K' | 'k' => (&s[..s.len() - 1], 1024u64),
        'M' | 'm' => (&s[..s.len() - 1], 1024 * 1024),
        'G' | 'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        c if c.is_ascii_digit() => (s, 1),
        _ => return None,
    };
    num.parse::<u64>().ok()?.checked_mul(mult)
}

fn glob_match(pat: &str, name: &str) -> bool {
    let (px, nx) = (pat.as_bytes(), name.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
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

// ---------------------------------------------------------------------------
// ls / cat
// ---------------------------------------------------------------------------

fn cmd_ls(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let (long, all, hum) =
        (has(&flags, 'l'), has(&flags, 'a'), has(&flags, 'h'));
    let dir_as_file = has(&flags, 'd');
    let recursive = has(&flags, 'R');
    for f in &flags {
        if !matches!(f, 'l' | 'a' | 'h' | 'd' | 'R') {
            return err(out, &format!("ls: invalid option -- '{f}'"), 2);
        }
    }
    let paths: Vec<&str> = if rest.is_empty() { vec!["."] } else { rest };
    let mut rc = 0;
    let multi = paths.len() > 1 || recursive;
    for (pi, p) in paths.iter().enumerate() {
        let md = fs::symlink_metadata(p);
        match md {
            Ok(m) if dir_as_file || !m.is_dir() => {
                if long {
                    ls_long(out, p, Path::new(p), &m, hum);
                } else {
                    emit(out, p);
                }
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                emit(out, &format!("ls: {p}: {e}"));
                rc = 1;
                continue;
            }
        }
        let mut queue: Vec<PathBuf> = vec![PathBuf::from(p)];
        let mut first = true;
        while let Some(dir) = queue.pop() {
            let rd = match fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) => {
                    emit(out, &format!("ls: {}: {e}", dir.display()));
                    rc = 1;
                    continue;
                }
            };
            let mut names: Vec<String> = Vec::new();
            for e in rd.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    if !all && n.starts_with('.') {
                        continue;
                    }
                    names.push(n.to_string());
                }
            }
            names.sort();
            if multi || !first {
                if pi > 0 || !first {
                    emit(out, "");
                }
                if first {
                    emit(out, &format!("{p}:"));
                } else {
                    emit(out, &format!("{}:", dir.display()));
                }
            }
            first = false;
            let mut subdirs: Vec<PathBuf> = Vec::new();
            if long {
                for n in &names {
                    let full = dir.join(n);
                    match fs::symlink_metadata(&full) {
                        Ok(m) => {
                            if recursive && m.is_dir() && !m.file_type().is_symlink() {
                                subdirs.push(full.clone());
                            }
                            ls_long(out, n, &full, &m, hum)
                        }
                        Err(e) => {
                            emit(out, &format!("ls: {}: {e}", full.display()));
                            rc = 1;
                        }
                    }
                }
            } else {
                for n in &names {
                    emit(out, n);
                }
                if recursive {
                    for n in &names {
                        let full = dir.join(n);
                        if full.is_dir() && !fs::symlink_metadata(&full).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
                            subdirs.push(full);
                        }
                    }
                }
            }
            subdirs.sort();
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

fn ls_long(out: &mut String, name: &str, real: &Path, md: &fs::Metadata, hum: bool) {
    let ft = md.file_type();
    let is_link = ft.is_symlink();
    let is_dir = md.is_dir();
    let mode = mode_bits(md);
    let size = if hum { human(md.len()) } else { md.len().to_string() };
    let mut line = format!("{} {:>8} {}", mode_str(mode, is_dir, is_link), size, name);
    if is_link {
        match fs::read_link(real) {
            Ok(t) => line.push_str(&format!(" -> {}", t.display())),
            Err(_) => line.push_str(" -> ?"),
        }
    }
    emit(out, &line);
}

fn cmd_cat(args: &[&str], stdin: Option<&str>, out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let number = has(&flags, 'n');
    for f in &flags {
        if *f != 'n' {
            return err(out, &format!("cat: invalid option -- '{f}'"), 2);
        }
    }
    if rest.is_empty() {
        return match stdin {
            Some(s) => {
                dump_cat(out, s.as_bytes(), number);
                0
            }
            None => err(out, "cat: missing operand", 2),
        };
    }
    let mut rc = 0;
    for p in rest {
        match fs::read(p) {
            Ok(b) => dump_cat(out, &b, number),
            Err(e) => {
                emit(out, &format!("cat: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

fn dump_cat(out: &mut String, bytes: &[u8], number: bool) {
    let text = String::from_utf8_lossy(bytes);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    if !number {
        for ln in body.split('\n') {
            emit(out, ln);
        }
        if body.is_empty() {
            emit(out, "");
        }
        return;
    }
    if body.is_empty() {
        return;
    }
    for (i, ln) in body.split('\n').enumerate() {
        emit(out, &format!("{:>6}\t{}", i + 1, ln));
    }
}

// ---------------------------------------------------------------------------
// touch / mkdir / rmdir / rm
// ---------------------------------------------------------------------------

fn cmd_touch(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let no_create = has(&flags, 'c');
    for f in &flags {
        if *f != 'c' {
            return err(out, &format!("touch: invalid option -- '{f}'"), 2);
        }
    }
    if rest.is_empty() {
        return err(out, "touch: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        if Path::new(p).exists() {
            continue; // mtime bump skipped (no-op success)
        }
        if no_create {
            continue;
        }
        if let Some(parent) = Path::new(p).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent) {
                    emit(out, &format!("touch: {p}: {e}"));
                    rc = 1;
                    continue;
                }
            }
        }
        match fs::OpenOptions::new().create(true).write(true).open(p) {
            Ok(_) => {}
            Err(e) => {
                emit(out, &format!("touch: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_mkdir(args: &[&str], out: &mut String) -> i32 {
    let mut parents = false;
    let mut mode: Option<u32> = None;
    let mut operands: Vec<&str> = Vec::new();
    let mut i = 0;
    let mut dd = false;
    while i < args.len() {
        let a = args[i];
        if !dd && a == "--" {
            dd = true;
        } else if !dd && a == "-p" {
            parents = true;
        } else if !dd && (a == "-m" || a.starts_with("-m")) {
            let v = if a == "-m" {
                i += 1;
                if i >= args.len() {
                    return err(out, "mkdir: -m needs an octal mode", 2);
                }
                args[i]
            } else {
                &a[2..]
            };
            match parse_octal(v) {
                Some(m) => mode = Some(m),
                None => return err(out, &format!("mkdir: invalid mode '{v}'"), 2),
            }
        } else if !dd && a.starts_with('-') && a.len() > 1 {
            return err(out, &format!("mkdir: invalid option '{a}'"), 2);
        } else {
            operands.push(a);
        }
        i += 1;
    }
    if operands.is_empty() {
        return err(out, "mkdir: missing operand", 2);
    }
    let mut rc = 0;
    for p in operands {
        let r = if parents { fs::create_dir_all(p) } else { fs::create_dir(p) };
        match r {
            Ok(()) => {
                #[cfg(unix)]
                if let Some(m) = mode {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(p, fs::Permissions::from_mode(m));
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
            Err(e) => {
                emit(out, &format!("mkdir: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_rmdir(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = split_flags(args);
    if rest.is_empty() {
        return err(out, "rmdir: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        if let Err(e) = fs::remove_dir(p) {
            emit(out, &format!("rmdir: {p}: {e}"));
            rc = 1;
        }
    }
    rc
}

fn cmd_rm(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let (rec, force) = (has(&flags, 'r') || has(&flags, 'R'), has(&flags, 'f'));
    for f in &flags {
        if !matches!(f, 'r' | 'R' | 'f') {
            return err(out, &format!("rm: invalid option -- '{f}'"), 2);
        }
    }
    if rest.is_empty() {
        if force {
            return 0;
        }
        return err(out, "rm: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        let r = rm_one(Path::new(p), rec);
        match r {
            Ok(()) => {}
            Err(e) if force && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                if !force {
                    emit(out, &format!("rm: {p}: {e}"));
                }
                rc = 1;
            }
        }
    }
    rc
}

fn rm_one(p: &Path, rec: bool) -> std::io::Result<()> {
    let md = fs::symlink_metadata(p)?;
    if md.is_dir() && !md.file_type().is_symlink() {
        if !rec {
            return Err(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                "Is a directory",
            ));
        }
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

// ---------------------------------------------------------------------------
// cp / mv / ln / readlink / stat
// ---------------------------------------------------------------------------

fn cmd_cp(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let rec = has(&flags, 'r') || has(&flags, 'R');
    for f in &flags {
        if !matches!(f, 'r' | 'R') {
            return err(out, &format!("cp: invalid option -- '{f}'"), 2);
        }
    }
    if rest.len() < 2 {
        return err(out, "cp: missing operand (usage: cp [-r] <src...> <dst>)", 2);
    }
    let (srcs, dst) = rest.split_at(rest.len() - 1);
    let dst = Path::new(dst[0]);
    if srcs.len() > 1 && !dst.is_dir() {
        return err(out, &format!("cp: target '{}': Not a directory", dst.display()), 1);
    }
    let mut rc = 0;
    for s in srcs {
        let target = if dst.is_dir() {
            let base = Path::new(s).file_name().unwrap_or_default();
            dst.join(base)
        } else {
            dst.to_path_buf()
        };
        if let Err(e) = cp_one(Path::new(s), &target, rec) {
            emit(out, &format!("cp: {s} -> {}: {e}", target.display()));
            rc = 1;
        }
    }
    rc
}

fn cp_one(src: &Path, dst: &Path, rec: bool) -> std::io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        let t = fs::read_link(src)?;
        let _ = rm_one(dst, true);
        #[cfg(unix)]
        return std::os::unix::fs::symlink(t, dst);
        #[cfg(not(unix))]
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "symlinks unsupported"));
    }
    if md.is_dir() {
        if !rec {
            return Err(std::io::Error::new(std::io::ErrorKind::IsADirectory, "Is a directory"));
        }
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)?.flatten() {
            cp_one(&e.path(), &dst.join(e.file_name()), true)?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::copy(src, dst)?;
    Ok(())
}

fn cmd_mv(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    for f in &flags {
        if *f != 'f' {
            return err(out, &format!("mv: invalid option -- '{f}'"), 2);
        }
    }
    if rest.len() < 2 {
        return err(out, "mv: missing operand (usage: mv <src...> <dst>)", 2);
    }
    let (srcs, dst) = rest.split_at(rest.len() - 1);
    let dst = Path::new(dst[0]);
    if srcs.len() > 1 && !dst.is_dir() {
        return err(out, &format!("mv: target '{}': Not a directory", dst.display()), 1);
    }
    let mut rc = 0;
    for s in srcs {
        let target = if dst.is_dir() {
            let base = Path::new(s).file_name().unwrap_or_default();
            dst.join(base)
        } else {
            dst.to_path_buf()
        };
        if let Err(e) = fs::rename(s, &target) {
            // cross-device fallback: copy + remove
            if e.kind() == std::io::ErrorKind::CrossesDevices {
                let md = match fs::symlink_metadata(s) {
                    Ok(m) => m,
                    Err(e) => {
                        emit(out, &format!("mv: {s}: {e}"));
                        rc = 1;
                        continue;
                    }
                };
                let r = if md.is_dir() { cp_one(Path::new(s), &target, true) } else { cp_one(Path::new(s), &target, false) };
                match r {
                    Ok(()) => {
                        if let Err(e) = rm_one(Path::new(s), true) {
                            emit(out, &format!("mv: {s}: {e}"));
                            rc = 1;
                        }
                    }
                    Err(e) => {
                        emit(out, &format!("mv: {s} -> {}: {e}", target.display()));
                        rc = 1;
                    }
                }
            } else {
                emit(out, &format!("mv: {s} -> {}: {e}", target.display()));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_ln(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    if !has(&flags, 's') {
        return err(out, "ln: only symbolic links are supported (usage: ln -s <target> <link>)", 2);
    }
    for f in &flags {
        if *f != 's' {
            return err(out, &format!("ln: invalid option -- '{f}'"), 2);
        }
    }
    if rest.len() != 2 {
        return err(out, "ln: usage: ln -s <target> <link>", 2);
    }
    #[cfg(unix)]
    match std::os::unix::fs::symlink(rest[0], rest[1]) {
        Ok(()) => 0,
        Err(e) => err(out, &format!("ln: {}: {e}", rest[1]), 1),
    }
    #[cfg(not(unix))]
    {
        let _ = rest;
        err(out, "ln: symlinks unsupported on this platform", 1)
    }
}

fn cmd_readlink(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = split_flags(args);
    if rest.is_empty() {
        return err(out, "readlink: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        match fs::read_link(p) {
            Ok(t) => emit(out, &t.display().to_string()),
            Err(e) => {
                emit(out, &format!("readlink: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

fn cmd_stat(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = split_flags(args);
    if rest.is_empty() {
        return err(out, "stat: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                let ft = m.file_type();
                let kind = if ft.is_symlink() {
                    "symbolic link"
                } else if ft.is_dir() {
                    "directory"
                } else {
                    "regular file"
                };
                emit(out, &format!("  File: {p}"));
                emit(out, &format!("  Size: {:<12} Blocks: {}", m.len(), m.len().div_ceil(512)));
                emit(out, &format!("Access: {:04o} ({})", mode_bits(&m) & 0o7777, mode_str(mode_bits(&m), m.is_dir(), ft.is_symlink())));
                emit(out, &format!("  Type: {kind}"));
                if ft.is_symlink() {
                    match fs::read_link(p) {
                        Ok(t) => emit(out, &format!("Target: {}", t.display())),
                        Err(_) => {}
                    }
                }
            }
            Err(e) => {
                emit(out, &format!("stat: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

// ---------------------------------------------------------------------------
// wc / head / tail
// ---------------------------------------------------------------------------

fn input_bytes<'a>(
    operands: &[&'a str],
    stdin: Option<&'a str>,
    out: &mut String,
) -> Option<Vec<(String, Vec<u8>)>> {
    if operands.is_empty() {
        return match stdin {
            Some(s) => Some(vec![("<stdin>".into(), s.as_bytes().to_vec())]),
            None => {
                emit(out, "missing operand (hint: pass a file or use < file)");
                None
            }
        };
    }
    let mut v = Vec::new();
    for p in operands {
        match fs::read(p) {
            Ok(b) => v.push((p.to_string(), b)),
            Err(e) => {
                emit(out, &format!("{p}: {e}"));
                return None;
            }
        }
    }
    Some(v)
}

fn cmd_wc(args: &[&str], stdin: Option<&str>, out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let (mut sl, mut sw, mut sc) = (false, false, false);
    for f in &flags {
        match f {
            'l' => sl = true,
            'w' => sw = true,
            'c' => sc = true,
            _ => return err(out, &format!("wc: invalid option -- '{f}'"), 2),
        }
    }
    if !sl && !sw && !sc {
        sl = true;
        sw = true;
        sc = true;
    }
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    let (mut tl, mut tw, mut tc) = (0u64, 0u64, 0u64);
    for (name, data) in &inputs {
        let l = data.iter().filter(|&&b| b == b'\n').count() as u64;
        let w = String::from_utf8_lossy(data).split_whitespace().count() as u64;
        let c = data.len() as u64;
        tl += l;
        tw += w;
        tc += c;
        let mut line = String::new();
        if sl {
            line.push_str(&format!("{:>7}", l));
        }
        if sw {
            line.push_str(&format!("{:>8}", w));
        }
        if sc {
            line.push_str(&format!("{:>8}", c));
        }
        if inputs.len() > 1 || !rest.is_empty() {
            line.push(' ');
            line.push_str(name);
        }
        emit(out, line.trim_start());
    }
    if inputs.len() > 1 {
        let mut line = String::new();
        if sl {
            line.push_str(&format!("{:>7}", tl));
        }
        if sw {
            line.push_str(&format!("{:>8}", tw));
        }
        if sc {
            line.push_str(&format!("{:>8}", tc));
        }
        line.push_str(" total");
        emit(out, line.trim_start());
    }
    0
}

fn parse_n<'a>(args: &[&'a str], what: &str) -> Result<(u64, Vec<&'a str>), String> {
    let mut n = 10u64;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "-n" {
            i += 1;
            if i >= args.len() {
                return Err(format!("{what}: -n needs a number"));
            }
            n = args[i].parse::<u64>().map_err(|_| format!("{what}: invalid number '{}'", args[i]))?;
        } else if a.starts_with("-n") && a.len() > 2 {
            n = a[2..].parse::<u64>().map_err(|_| format!("{what}: invalid number '{a}'"))?;
        } else if a.starts_with('-') && a.len() > 1 && a[1..].chars().all(|c| c.is_ascii_digit()) {
            n = a[1..].parse::<u64>().map_err(|_| format!("{what}: invalid number '{a}'"))?;
        } else {
            rest.push(a);
        }
        i += 1;
    }
    Ok((n, rest))
}

fn cmd_head(args: &[&str], stdin: Option<&str>, out: &mut String) -> i32 {
    let (n, rest) = match parse_n(args, "head") {
        Ok(v) => v,
        Err(e) => return err(out, &e, 2),
    };
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    for (name, data) in &inputs {
        if inputs.len() > 1 {
            emit(out, &format!("==> {name} <=="));
        }
        for ln in String::from_utf8_lossy(data).lines().take(n as usize) {
            emit(out, ln);
        }
    }
    0
}

fn cmd_tail(args: &[&str], stdin: Option<&str>, out: &mut String) -> i32 {
    let (n, rest) = match parse_n(args, "tail") {
        Ok(v) => v,
        Err(e) => return err(out, &e, 2),
    };
    let inputs = match input_bytes(&rest, stdin, out) {
        Some(v) => v,
        None => return 1,
    };
    for (name, data) in &inputs {
        if inputs.len() > 1 {
            emit(out, &format!("==> {name} <=="));
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

// ---------------------------------------------------------------------------
// du / df / find / chmod / truncate
// ---------------------------------------------------------------------------

fn du_path(p: &Path, depth: usize, budget: &mut u64) -> u64 {
    if depth > 64 || *budget == 0 {
        return 0;
    }
    *budget -= 1;
    let md = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if !md.is_dir() || md.file_type().is_symlink() {
        return md.len();
    }
    let mut total = 0u64;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            if *budget == 0 {
                break;
            }
            total += du_path(&e.path(), depth + 1, budget);
        }
    }
    total
}

fn cmd_du(args: &[&str], out: &mut String) -> i32 {
    let (flags, rest) = split_flags(args);
    let (hum, sum) = (has(&flags, 'h'), has(&flags, 's'));
    for f in &flags {
        if !matches!(f, 'h' | 's') {
            return err(out, &format!("du: invalid option -- '{f}'"), 2);
        }
    }
    let paths: Vec<&str> = if rest.is_empty() { vec!["."] } else { rest };
    let mut rc = 0;
    for p in paths {
        if !Path::new(p).exists() && fs::symlink_metadata(p).is_err() {
            emit(out, &format!("du: {p}: No such file or directory"));
            rc = 1;
            continue;
        }
        let mut budget = u64::MAX;
        let bytes = du_path(Path::new(p), 0, &mut budget);
        let show = if hum { human(bytes) } else { (bytes / 1024 + 1).to_string() };
        if sum || !Path::new(p).is_dir() {
            emit(out, &format!("{show}\t{p}"));
        } else {
            // GNU du lists subdirs too; walk and print each dir.
            du_tree(Path::new(p), hum, out);
        }
    }
    rc
}

fn du_tree(dir: &Path, hum: bool, out: &mut String) {
    let mut budget = u64::MAX;
    let bytes = du_path(dir, 0, &mut budget);
    let show = if hum { human(bytes) } else { (bytes / 1024 + 1).to_string() };
    emit(out, &format!("{show}\t{}", dir.display()));
    if let Ok(rd) = fs::read_dir(dir) {
        let mut subs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.is_symlink())
            .collect();
        subs.sort();
        for s in subs {
            du_tree(&s, hum, out);
        }
    }
}

fn read_mounts() -> Vec<(String, String, String)> {
    // (device, mountpoint, fstype) from /proc/mounts, else cwd fallback.
    if let Ok(text) = fs::read_to_string("/proc/mounts") {
        let mut v = Vec::new();
        for ln in text.lines() {
            let f: Vec<&str> = ln.split_whitespace().collect();
            if f.len() >= 3 {
                v.push((f[0].to_string(), f[1].to_string(), f[2].to_string()));
            }
        }
        if !v.is_empty() {
            return v;
        }
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/".into());
    vec![("hostfs".into(), cwd, "hostfs".into())]
}

fn cmd_df(args: &[&str], out: &mut String) -> i32 {
    let (flags, _) = split_flags(args);
    let hum = has(&flags, 'h');
    for f in &flags {
        if *f != 'h' {
            return err(out, &format!("df: invalid option -- '{f}'"), 2);
        }
    }
    emit(out, "Filesystem     1K-blocks    Used   Avail Use% Mounted on");
    for (dev, mp, _fst) in read_mounts() {
        // Bounded walk: cap entries so `/` doesn't take forever; show `?`
        // when the cap trips (capacity needs statvfs; std-only here).
        let mut budget = 200_000u64;
        let used = du_path(Path::new(&mp), 0, &mut budget);
        if budget == 0 {
            let q = "?";
            emit(out, &format!("{dev:<13} {q:>9} {q} n/a      n/a {mp}"));
        } else {
            let used_s = if hum { human(used) } else { (used / 1024 + 1).to_string() };
            emit(out, &format!("{dev:<13} {used_s:>9} {used_s:>7} n/a      n/a {mp}"));
        }
    }
    0
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
                    return err(out, "find: -name needs a pattern", 2);
                }
                name_pat = Some(args[i]);
            }
            "-type" => {
                i += 1;
                if i >= args.len() {
                    return err(out, "find: -type needs f, d or l", 2);
                }
                let t = args[i].chars().next().unwrap_or('?');
                if !matches!(t, 'f' | 'd' | 'l') {
                    return err(out, &format!("find: unknown type '{}'", args[i]), 2);
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
        if fs::symlink_metadata(r).is_err() {
            emit(out, &format!("find: {r}: No such file or directory"));
            rc = 1;
            continue;
        }
        find_walk(Path::new(r), name_pat, type_f, 0, out);
    }
    rc
}

fn find_walk(p: &Path, pat: Option<&str>, typ: Option<char>, depth: usize, out: &mut String) {
    if depth > 64 {
        return;
    }
    let md = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(_) => return,
    };
    let kind = if md.file_type().is_symlink() {
        'l'
    } else if md.is_dir() {
        'd'
    } else {
        'f'
    };
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let disp = p.display().to_string();
    if pat.map(|x| glob_match(x, base)).unwrap_or(true) && typ.map(|t| t == kind).unwrap_or(true) {
        emit(out, &disp);
    }
    if kind == 'd' {
        if let Ok(rd) = fs::read_dir(p) {
            let mut kids: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            kids.sort();
            for k in kids {
                find_walk(&k, pat, typ, depth + 1, out);
            }
        }
    }
}

fn cmd_chmod(args: &[&str], out: &mut String) -> i32 {
    let (_, rest) = split_flags(args);
    if rest.len() < 2 {
        return err(out, "chmod: usage: chmod <octal-mode> <path...>", 2);
    }
    let mode = match parse_octal(rest[0]) {
        Some(m) => m,
        None => return err(out, &format!("chmod: invalid mode '{}'", rest[0]), 2),
    };
    #[cfg(not(unix))]
    {
        let _ = mode;
        return err(out, "chmod: not supported on this platform", 1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut rc = 0;
        for p in &rest[1..] {
            if let Err(e) = fs::set_permissions(p, fs::Permissions::from_mode(mode)) {
                emit(out, &format!("chmod: {p}: {e}"));
                rc = 1;
            }
        }
        rc
    }
}

fn cmd_truncate(args: &[&str], out: &mut String) -> i32 {
    let mut size: Option<u64> = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-s" {
            i += 1;
            if i >= args.len() {
                return err(out, "truncate: -s needs a size", 2);
            }
            size = parse_size(args[i]);
            if size.is_none() {
                return err(out, &format!("truncate: invalid size '{}'", args[i]), 2);
            }
        } else if args[i].starts_with("-s") && args[i].len() > 2 {
            size = parse_size(&args[i][2..]);
            if size.is_none() {
                return err(out, &format!("truncate: invalid size '{}'", args[i]), 2);
            }
        } else {
            rest.push(args[i]);
        }
        i += 1;
    }
    let size = match size {
        Some(s) => s,
        None => return err(out, "truncate: usage: truncate -s <size> <file...>", 2),
    };
    if rest.is_empty() {
        return err(out, "truncate: missing operand", 2);
    }
    let mut rc = 0;
    for p in rest {
        let f = fs::OpenOptions::new().create(true).write(true).open(p);
        match f {
            Ok(f) => {
                if let Err(e) = f.set_len(size) {
                    emit(out, &format!("truncate: {p}: {e}"));
                    rc = 1;
                }
            }
            Err(e) => {
                emit(out, &format!("truncate: {p}: {e}"));
                rc = 1;
            }
        }
    }
    rc
}

/// Read a whole file (helper for `<` redirection in bsh).
#[allow(dead_code)]
pub fn read_input_file(path: &str) -> Result<String, String> {
    let mut s = String::new();
    fs::File::open(path)
        .and_then(|mut f| f.read_to_string(&mut s))
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(s)
}
