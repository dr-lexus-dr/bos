//! Native bOS ABI. This crate does not use Linux or the host standard library.
#![no_std]
use core::{arch::asm, ffi::CStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub i64);
pub type Result<T> = core::result::Result<T, Error>;
pub const READ_ONLY: u64 = 0;
pub const WRITE_ONLY: u64 = 1;
pub const READ_WRITE: u64 = 2;
pub const CREATE: u64 = 0x40;
pub const TRUNCATE: u64 = 0x200;
pub const APPEND: u64 = 0x400;

/// Invoke the three-argument compatibility ABI.
///
/// # Safety
/// Pointer arguments must remain valid for the operation, and the caller must
/// obey the syscall's aliasing and ownership rules. Only execute inside bOS.
pub unsafe fn syscall3(number: u64, a: u64, b: u64, c: u64) -> i64 {
    let result: i64;
    asm!("int 0x80", inlateout("rax") number => result,
        in("rdi") a, in("rsi") b, in("rdx") c);
    result
}

fn decode(value: i64) -> Result<usize> {
    if value < 0 {
        Err(Error(-value))
    } else {
        Ok(value as usize)
    }
}

pub fn write(fd: u64, bytes: &[u8]) -> Result<usize> {
    let bytes = &bytes[..bytes.len().min(4096)];
    decode(unsafe { syscall3(1, fd, bytes.as_ptr() as u64, bytes.len() as u64) })
}

pub fn write_all(fd: u64, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let n = write(fd, bytes)?;
        if n == 0 || n > bytes.len() {
            return Err(Error(5));
        }
        bytes = &bytes[n..];
    }
    Ok(())
}

pub struct File {
    fd: u64,
}
impl File {
    pub fn sync(&self) -> Result<()> {
        decode(unsafe { syscall3(74, self.fd, 0, 0) }).map(|_| ())
    }
    pub fn as_raw_fd(&self) -> u64 {
        self.fd
    }
    pub fn open(path: &CStr, flags: u64) -> Result<Self> {
        let fd = decode(unsafe { syscall3(2, path.as_ptr() as u64, flags, 0) })?;
        Ok(Self { fd: fd as u64 })
    }
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<usize> {
        let length = bytes.len().min(4096);
        decode(unsafe { syscall3(0, self.fd, bytes.as_mut_ptr() as u64, length as u64) })
    }
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        write_all(self.fd, bytes)
    }
    pub fn seek(&mut self, offset: i64, whence: u64) -> Result<usize> {
        decode(unsafe { syscall3(8, self.fd, offset as u64, whence) })
    }
}
impl Drop for File {
    fn drop(&mut self) {
        unsafe {
            syscall3(3, self.fd, 0, 0);
        }
    }
}

pub fn exit(status: u8) -> ! {
    unsafe {
        syscall3(60, status as u64, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_errno_is_preserved() {
        assert_eq!(decode(-14), Err(Error(14)));
        assert_eq!(decode(0), Ok(0));
        assert_eq!(decode(4096), Ok(4096));
    }
}

pub fn read(fd: u64, bytes: &mut [u8]) -> Result<usize> {
    decode(unsafe {
        syscall3(
            0,
            fd,
            bytes.as_mut_ptr() as u64,
            bytes.len().min(4096) as u64,
        )
    })
}
pub fn close(fd: u64) -> Result<()> {
    decode(unsafe { syscall3(3, fd, 0, 0) }).map(|_| ())
}
pub fn dup2(from: u64, to: u64) -> Result<usize> {
    decode(unsafe { syscall3(33, from, to, 0) })
}
pub fn getpid() -> u64 {
    unsafe { syscall3(39, 0, 0, 0) as u64 }
}
pub fn getppid() -> u64 {
    unsafe { syscall3(110, 0, 0, 0) as u64 }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FdMapping {
    pub source: u32,
    pub target: u32,
}
pub fn spawn(path: &CStr, mappings: &[FdMapping]) -> Result<u64> {
    decode(unsafe {
        syscall3(
            1004,
            path.as_ptr() as u64,
            mappings.as_ptr() as u64,
            mappings.len() as u64,
        )
    })
    .map(|p| p as u64)
}
pub fn waitpid(pid: Option<u64>, nonblocking: bool) -> Result<(u64, i64)> {
    let mut status = 0i64;
    let pid = decode(unsafe {
        syscall3(
            61,
            pid.unwrap_or(u64::MAX),
            &mut status as *mut i64 as u64,
            nonblocking as u64,
        )
    })?;
    Ok((pid as u64, status))
}
pub fn pipe() -> Result<[u32; 2]> {
    let mut fds = [0u32; 2];
    decode(unsafe { syscall3(22, fds.as_mut_ptr() as u64, 0, 0) })?;
    Ok(fds)
}
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        core::arch::global_asm!(
            ".global _start",
            "_start:",
            "xor rbp, rbp",
            "call bos_main",
            "ud2"
        );
        #[no_mangle]
        extern "C" fn bos_main() -> ! {
            $crate::exit($main());
        }
        #[panic_handler]
        fn panic(_: &core::panic::PanicInfo) -> ! {
            $crate::exit(101)
        }
    };
}

pub fn sync() -> Result<()> {
    decode(unsafe { syscall3(162, 0, 0, 0) }).map(|_| ())
}
