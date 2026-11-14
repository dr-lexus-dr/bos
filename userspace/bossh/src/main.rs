#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { loop {} }

fn syscall(nr: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") nr, in("rdi") a1, in("rsi") a2, in("rdx") a3,
            lateout("rax") ret,
        );
    }
    ret
}

fn write_bytes(s: &[u8]) { syscall(1, 1, s.as_ptr() as u64, s.len() as u64); }
fn write_str(s: &str) { write_bytes(s.as_bytes()); }

fn read_line(buf: &mut [u8]) -> usize {
    let n = syscall(0, 0, buf.as_mut_ptr() as u64, buf.len() as u64);
    if n < 0 { 0 } else { n as usize }
}

fn getcwd(buf: &mut [u8]) -> usize {
    let n = syscall(79, buf.as_mut_ptr() as u64, buf.len() as u64, 0);
    if n < 0 { 0 } else { n as usize }
}

fn chdir(path: &[u8]) -> i64 { syscall(80, path.as_ptr() as u64, 0, 0) }

fn exit(code: i64) -> ! { syscall(60, code as u64, 0, 0); loop {} }

fn trim(buf: &mut [u8], n: usize) -> usize {
    let mut end = n;
    while end > 0 && (buf[end-1] == b'\n' || buf[end-1] == b'\r' || buf[end-1] == b' ') {
        end -= 1;
    }
    if end < buf.len() { buf[end] = 0; }
    end
}

fn cstr(buf: &[u8]) -> &[u8] {
    // return slice up to null
    let mut i = 0;
    while i < buf.len() && buf[i] != 0 { i += 1; }
    &buf[..i]
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut cwd = [0u8; 256];
    let mut line = [0u8; 256];
    let mut arg = [0u8; 256];
    loop {
        getcwd(&mut cwd);
        let mut clen = 0;
        while clen < cwd.len() && cwd[clen] != 0 { clen += 1; }
        write_str("bSH:");
        write_bytes(&cwd[..clen]);
        write_str("# ");
        let n = read_line(&mut line);
        if n == 0 { exit(0); }
        // echo input to mimic bSH
        write_bytes(&line[..n]);
        if n > 0 && line[n-1] == b'\r' { write_str("\n"); }
        let len = trim(&mut line, n);
        if len == 0 { continue; }

        // parse first token
        let mut i = 0;
        while i < len && line[i] != b' ' { i += 1; }
        let cmd = &line[..i];
        // skip spaces
        let mut j = i;
        while j < len && line[j] == b' ' { j += 1; }
        let rest = &line[j..len];

        if cmd == b"exit" || cmd == b"quit" {
            exit(0);
        } else if cmd == b"pwd" {
            write_bytes(&cwd[..clen]);
            write_str("\n");
        } else if cmd == b"cd" {
            if rest.is_empty() {
                let p = b"/users/default\0";
                chdir(&p[..p.len()-1]);
            } else {
                // copy rest to arg null-terminated
                let mut k = 0;
                while k < rest.len() && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
                arg[k] = 0;
                chdir(&arg[..k]);
            }
        } else if cmd == b"echo" {
            write_bytes(rest);
            write_str("\n");
        } else if cmd == b"clear" {
            write_str("\x1b[2J\x1b[H");
        } else if cmd == b"help" {
            write_str("bSH userspace shell commands:\n");
            write_str("  cd [dir] pwd ls [path] cat <file> echo <text>\n");
            write_str("  mkdir <dir> rmdir <dir> rm <file> touch <file>\n");
            write_str("  mv <src> <dst> clear exit help\n");
        } else if cmd == b"ls" {
            let path = if rest.is_empty() { b".\0" } else { rest };
            // build null-terminated path in arg
            let mut k = 0;
            let src = if rest.is_empty() { &b"."[..] } else { rest };
            while k < src.len() && k < arg.len()-1 { arg[k]=src[k]; k+=1; }
            arg[k]=0;
            let mut buf = [0u8; 4096];
            let n = syscall(1000, arg.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
            if n < 0 {
                write_str("ls: cannot list directory\n");
            } else {
                write_bytes(&buf[..n as usize]);
            }
        } else if cmd == b"cat" {
            if rest.is_empty() { write_str("cat: missing operand\n"); continue; }
            let mut k = 0;
            while k < rest.len() && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
            arg[k]=0;
            let fd = syscall(2, arg.as_ptr() as u64, 0, 0);
            if fd < 0 { write_str("cat: cannot open file\n"); continue; }
            let mut buf = [0u8; 512];
            loop {
                let r = syscall(0, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
                if r <= 0 { break; }
                write_bytes(&buf[..r as usize]);
            }
            syscall(3, fd as u64, 0, 0);
        } else if cmd == b"mkdir" {
            if rest.is_empty() { write_str("mkdir: missing operand\n"); continue; }
            let mut k = 0;
            while k < rest.len() && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
            arg[k]=0;
            if syscall(83, arg.as_ptr() as u64, 0, 0) < 0 { write_str("mkdir: failed\n"); }
        } else if cmd == b"rmdir" || cmd == b"rm" {
            if rest.is_empty() { write_str("rm: missing operand\n"); continue; }
            let mut k = 0;
            while k < rest.len() && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
            arg[k]=0;
            if syscall(84, arg.as_ptr() as u64, 0, 0) < 0 { write_str("rm: failed\n"); }
        } else if cmd == b"touch" {
            if rest.is_empty() { write_str("touch: missing operand\n"); continue; }
            let mut k = 0;
            while k < rest.len() && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
            arg[k]=0;
            // create via open with flags? open flags not handled; use create by open then close
            let fd = syscall(2, arg.as_ptr() as u64, 0, 0);
            if fd >= 0 { syscall(3, fd as u64, 0, 0); }
            else {
                // try create not supported; ignore
            }
        } else if cmd == b"mv" {
            // parse two args
            let mut k = 0;
            while k < rest.len() && rest[k] != b' ' && k < arg.len()-1 { arg[k]=rest[k]; k+=1; }
            arg[k]=0;
            let mut j2 = k;
            while j2 < rest.len() && rest[j2] == b' ' { j2 += 1; }
            let mut dst = [0u8; 256];
            let mut dlen = 0;
            while j2 < rest.len() && dlen < dst.len()-1 { dst[dlen]=rest[j2]; dlen+=1; j2+=1; }
            dst[dlen]=0;
            if dlen == 0 { write_str("mv: missing destination\n"); continue; }
            if syscall(82, arg.as_ptr() as u64, dst.as_ptr() as u64, 0) < 0 { write_str("mv: failed\n"); }
        } else {
            // delegate to bSH (kernel) for builtins/network commands
            let mut k = 0;
            while k < len && k < arg.len()-1 { arg[k]=line[k]; k+=1; }
            arg[k]=0;
            syscall(1004, arg.as_ptr() as u64, 0, 0);
        }
    }
}