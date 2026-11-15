//! Single syscall dispatcher for the active ring-3 program.
//! Numbers not implemented below return ENOSYS; no user pointers are dereferenced.
use crate::drivers::console;

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

pub(crate) fn fs_err(e: crate::fs::FileSystemError) -> u64 {
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

#[unsafe(no_mangle)]
extern "C" fn dispatch_user_syscall(number: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    dispatch(number, arg0, arg1, arg2)
}

pub fn dispatch(number: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    if number == SYS_EXIT {
        crate::user::exit((arg0 & 0xff) as i64);
    }
    crate::user::with_active_program(|program| program.file_call(number,arg0,arg1,arg2).unwrap_or_else(|e|e))
}

// Lossy console decoding without allocating on a user-controlled write.
pub(crate) fn print_bytes(mut bytes: &[u8]) {
    fn text(output: &str) {
        for part in output.split_inclusive('\n') {
            if let Some(line) = part.strip_suffix('\n') {
                console::print(line.trim_end_matches('\r'));
                console::print("\r\n");
            } else {
                console::print(part);
            }
        }
    }
    while !bytes.is_empty() {
        match core::str::from_utf8(bytes) {
            Ok(output) => {
                text(output);
                return;
            }
            Err(error) => {
                let prefix = error.valid_up_to();
                text(core::str::from_utf8(&bytes[..prefix]).unwrap());
                console::print("\u{fffd}");
                let skip = error.error_len().unwrap_or(bytes.len() - prefix);
                bytes = &bytes[prefix + skip..];
            }
        }
    }
}
