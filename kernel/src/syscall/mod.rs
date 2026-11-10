//! bOS syscalls: int 0x80 + syscall/sysret entry.
//!
//! Linux-compatible numbers where they exist (0/1/2/3/4/8/76/79/80/82/83/
//! 84/87/88/89/90, 12/35/39/57/59/60) plus bOS extensions 1000+.
//! Path arguments are NUL-terminated user strings (capped at 4096 B);
//! errors return `-errno` sign-extended to u64 (Linux style), so user code
//! can decode `-ret`.
//!
//! errno subset: 2 ENOENT, 13 EACCES, 17 EEXIST, 20 ENOTDIR, 21 EISDIR,
//! 22 EINVAL, 28 ENOSPC, 36 ENAMETOOLONG, 39 ENOTEMPTY, 30 EROFS, 9 EBADF.

use log::{trace, warn};
use x86_64::structures::idt::InterruptStackFrame;

pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_STAT: u64 = 4;
pub const SYS_FSTAT: u64 = 5;
pub const SYS_LSEEK: u64 = 8;
pub const SYS_EXIT: u64 = 60;
pub const SYS_FORK: u64 = 57;
pub const SYS_EXEC: u64 = 59;
pub const SYS_SBRK: u64 = 12;
pub const SYS_GETPID: u64 = 39;
pub const SYS_SLEEP: u64 = 35;
pub const SYS_TRUNCATE: u64 = 76;
pub const SYS_GETCWD: u64 = 79;
pub const SYS_CHDIR: u64 = 80;
pub const SYS_RENAME: u64 = 82;
pub const SYS_MKDIR: u64 = 83;
pub const SYS_RMDIR: u64 = 84;
pub const SYS_UNLINK: u64 = 87;
pub const SYS_SYMLINK: u64 = 88;
pub const SYS_READLINK: u64 = 89;
pub const SYS_CHMOD: u64 = 90;
pub const SYS_LISTDIR: u64 = 1000;
pub const SYS_LOADMOD: u64 = 1001; // bOS: load Win/Mac/Linux binary
pub const SYS_NET_SEND: u64 = 1002;
pub const SYS_NET_RECV: u64 = 1003;

// errno values (Linux)
const EPERM: u64 = 1;
const ENOENT: u64 = 2;
const EBADF: u64 = 9;
const EACCES: u64 = 13;
const EEXIST: u64 = 17;
const ENOTDIR: u64 = 20;
const EISDIR: u64 = 21;
const EINVAL: u64 = 22;
const ENOSPC: u64 = 28;
const EROFS: u64 = 30;
const ENOTEMPTY: u64 = 39;
const ENAMETOOLONG: u64 = 36;
const ENOSYS: u64 = 38;
const ELOOP: u64 = 40;

fn err(e: u64) -> u64 {
    u64::MAX - e + 1 // -e as u64
}

fn fs_err(e: crate::fs::FileSystemError) -> u64 {
    err(match e {
        crate::fs::FileSystemError::NotFound => ENOENT,
        crate::fs::FileSystemError::PermissionDenied => EACCES,
        crate::fs::FileSystemError::InvalidArgument => EINVAL,
        crate::fs::FileSystemError::NoSpace => ENOSPC,
        crate::fs::FileSystemError::NotDirectory => ENOTDIR,
        crate::fs::FileSystemError::IsDirectory => EISDIR,
        crate::fs::FileSystemError::NotEmpty => ENOTEMPTY,
        crate::fs::FileSystemError::AlreadyExists => EEXIST,
        crate::fs::FileSystemError::InvalidFormat => EINVAL,
        crate::fs::FileSystemError::UnsupportedVersion => EINVAL,
        crate::fs::FileSystemError::IoError => 5, // EIO
        crate::fs::FileSystemError::NotImplemented => ENOSYS,
        crate::fs::FileSystemError::NoSuchDevice => 19, // ENODEV
        crate::fs::FileSystemError::TooManyOpenFiles => 24, // EMFILE
        crate::fs::FileSystemError::BadFileDescriptor => EBADF,
        crate::fs::FileSystemError::SymlinkLoop => ELOOP,
        crate::fs::FileSystemError::NameTooLong => ENAMETOOLONG,
        crate::fs::FileSystemError::ReadOnly => EROFS,
    })
}

/// Read a NUL-terminated user string, capped at 4096 bytes.
fn user_str(ptr: u64) -> Result<alloc::string::String, u64> {
    if ptr == 0 {
        return Err(err(EINVAL));
    }
    let mut out = alloc::string::String::new();
    for i in 0..4096usize {
        let b = unsafe { core::ptr::read_volatile((ptr as usize + i) as *const u8) };
        if b == 0 {
            return Ok(out);
        }
        out.push(b as char);
    }
    Err(err(ENAMETOOLONG))
}

fn user_slice<'a>(ptr: u64, len: u64) -> Result<&'a [u8], u64> {
    if ptr == 0 && len != 0 {
        return Err(err(EINVAL));
    }
    if len > 16 * 1024 * 1024 {
        return Err(err(EINVAL));
    }
    Ok(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
}

fn user_slice_mut<'a>(ptr: u64, len: u64) -> Result<&'a mut [u8], u64> {
    if ptr == 0 && len != 0 {
        return Err(err(EINVAL));
    }
    if len > 16 * 1024 * 1024 {
        return Err(err(EINVAL));
    }
    Ok(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize) })
}

pub fn init() {
    // GDT/STAR setup for SYSCALL would go here; IDT 0x80 already installed.
}

pub fn handle_interrupt(_frame: InterruptStackFrame) {
    // int 0x80 path (stub): full register capture lives in asm stub.
    // Route a placeholder write syscall so early userspace prints work.
    trace!("syscall int 0x80");
}

pub fn dispatch(nr: u64, a1: u64, a2: u64, a3: u64, _a4: u64, _a5: u64) -> u64 {
    match nr {
        SYS_EXIT => {
            crate::proc::exit_current(a1 as i32);
        }
        SYS_GETPID => crate::proc::current_pid().unwrap_or(0),
        SYS_SLEEP => {
            crate::sched::sleep_ns(a1);
            0
        }
        SYS_WRITE => {
            // fd=a1, buf=a2, len=a3. fds 1/2 go to console; other fds to VFS.
            if a1 == 1 || a1 == 2 {
                let bytes = unsafe { core::slice::from_raw_parts(a2 as *const u8, a3 as usize) };
                crate::drivers::console::print(core::str::from_utf8(bytes).unwrap_or("?"));
                return a3;
            }
            let data = match user_slice(a2, a3) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().write(a1 as u32, data) {
                Ok(n) => n as u64,
                Err(e) => fs_err(e),
            }
        }
        SYS_READ => {
            if a1 == 0 {
                // stdin: polling serial not wired to syscalls yet
                return err(ENOSYS);
            }
            let buf = match user_slice_mut(a2, a3) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().read(a1 as u32, buf) {
                Ok(n) => n as u64,
                Err(e) => fs_err(e),
            }
        }
        SYS_OPEN => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().open(&path, a2 as u32) {
                Ok(fd) => fd as u64,
                Err(e) => fs_err(e),
            }
        }
        SYS_CLOSE => match crate::fs::VFS.lock().close(a1 as u32) {
            Ok(()) => 0,
            Err(e) => fs_err(e),
        },
        SYS_LSEEK => match crate::fs::VFS.lock().seek(a1 as u32, a2 as i64, a3 as u32) {
            Ok(off) => off,
            Err(e) => fs_err(e),
        },
        SYS_STAT | SYS_FSTAT => {
            // stat(path, buf, len) — writes "ino size mode uid gid" debug line.
            // Full `struct stat` ABI comes with userspace headers; for now
            // report size via the return channel is insufficient, so we fill
            // a 48-byte caller buffer: ino(8) size(8) mode(4) uid(4) gid(4).
            let st = if nr == SYS_STAT {
                let path = match user_str(a1) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                crate::fs::VFS.lock().stat(&path)
            } else {
                crate::fs::VFS.lock().fstat(a1 as u32)
            };
            match st {
                Ok(s) => {
                    let buf = match user_slice_mut(a2, a3.max(32)) {
                        Ok(b) => b,
                        Err(e) => return e,
                    };
                    if buf.len() >= 32 {
                        buf[0..8].copy_from_slice(&s.inode.to_le_bytes());
                        buf[8..16].copy_from_slice(&s.size.to_le_bytes());
                        buf[16..20].copy_from_slice(&s.mode.to_le_bytes());
                        buf[20..24].copy_from_slice(&s.uid.to_le_bytes());
                        buf[24..28].copy_from_slice(&s.gid.to_le_bytes());
                        buf[28..32].copy_from_slice(&s.links.to_le_bytes());
                    }
                    0
                }
                Err(e) => fs_err(e),
            }
        }
        SYS_MKDIR => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().mkdir(&path) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_RMDIR | SYS_UNLINK => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().remove(&path) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_RENAME => {
            let from = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let to = match user_str(a2) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().rename(&from, &to) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_SYMLINK => {
            let target = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let link = match user_str(a2) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().symlink(&target, &link) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_READLINK => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let buf = match user_slice_mut(a2, a3) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().readlink(&path) {
                Ok(t) => {
                    let b = t.as_bytes();
                    let n = b.len().min(buf.len());
                    buf[..n].copy_from_slice(&b[..n]);
                    n as u64
                }
                Err(e) => fs_err(e),
            }
        }
        SYS_CHMOD => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().chmod(&path, a2 as u32) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_TRUNCATE => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().truncate(&path, a2) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_GETCWD => {
            let buf = match user_slice_mut(a1, a2) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let cwd = crate::fs::VFS.lock().get_cwd();
            let b = cwd.as_bytes();
            if b.len() + 1 > buf.len() {
                return err(ENAMETOOLONG);
            }
            buf[..b.len()].copy_from_slice(b);
            buf[b.len()] = 0;
            (b.len() + 1) as u64
        }
        SYS_CHDIR => {
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().set_cwd(&path) {
                Ok(()) => 0,
                Err(e) => fs_err(e),
            }
        }
        SYS_LISTDIR => {
            // listdir(path, buf, len): NUL-separated names into caller buf.
            let path = match user_str(a1) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let buf = match user_slice_mut(a2, a3) {
                Ok(s) => s,
                Err(e) => return e,
            };
            match crate::fs::VFS.lock().list_dir(&path) {
                Ok(entries) => {
                    let mut off = 0usize;
                    for e in &entries {
                        let nb = e.name.as_bytes();
                        if off + nb.len() + 1 > buf.len() {
                            break;
                        }
                        buf[off..off + nb.len()].copy_from_slice(nb);
                        off += nb.len();
                        buf[off] = 0;
                        off += 1;
                    }
                    off as u64
                }
                Err(e) => fs_err(e),
            }
        }
        SYS_SBRK => 0, // bump allocator hook
        _ => {
            warn!("unhandled syscall {}", nr);
            err(ENOSYS)
        }
    }
}

/// bOS-local alias kept for etrace readability.
#[allow(dead_code)]
const EIO: u64 = 5;

/// Decode a `-errno` return into a short name (for bSH `echo $?`-style logs).
pub fn errno_name(ret: u64) -> &'static str {
    match (u64::MAX - ret + 1) as i64 {
        -1 => "EPERM",
        -2 => "ENOENT",
        -5 => "EIO",
        -9 => "EBADF",
        -13 => "EACCES",
        -17 => "EEXIST",
        -19 => "ENODEV",
        -20 => "ENOTDIR",
        -21 => "EISDIR",
        -22 => "EINVAL",
        -24 => "EMFILE",
        -28 => "ENOSPC",
        -30 => "EROFS",
        -36 => "ENAMETOOLONG",
        -38 => "ENOSYS",
        -39 => "ENOTEMPTY",
        -40 => "ELOOP",
        _ => "EUNKNOWN",
    }
}

#[allow(dead_code)]
fn _use_perm() {
    let _ = EPERM;
    let _ = EIO;
}
