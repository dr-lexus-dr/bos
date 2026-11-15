//! Shared open descriptions. Descriptor duplication shares position/endpoints.
use alloc::sync::Arc;
use spin::Mutex;

pub struct Pipe {
    bytes: [u8; 4096],
    head: usize,
    length: usize,
    readers: usize,
    writers: usize,
}
impl Pipe {
    pub fn pair() -> (Arc<Handle>, Arc<Handle>) {
        let pipe = Arc::new(Mutex::new(Self {
            bytes: [0; 4096],
            head: 0,
            length: 0,
            readers: 1,
            writers: 1,
        }));
        (
            Arc::new(Handle::PipeRead(pipe.clone())),
            Arc::new(Handle::PipeWrite(pipe)),
        )
    }
}
pub enum Handle {
    ConsoleRead,
    ConsoleWrite,
    File(u32),
    PipeRead(Arc<Mutex<Pipe>>),
    PipeWrite(Arc<Mutex<Pipe>>),
}
impl Handle {
    pub fn read(&self, bytes: &mut [u8]) -> Result<usize, i64> {
        match self {
            Self::File(fd) => crate::fs::VFS
                .lock()
                .read(*fd, bytes)
                .map_err(|e| crate::syscall::fs_err(e) as i64),
            Self::PipeRead(pipe) => {
                let mut p = pipe.lock();
                if bytes.is_empty() {
                    return Ok(0);
                }
                if p.length == 0 {
                    return if p.writers == 0 { Ok(0) } else { Err(-11) };
                }
                let n = bytes.len().min(p.length);
                for byte in &mut bytes[..n] {
                    *byte = p.bytes[p.head];
                    p.head = (p.head + 1) % 4096;
                }
                p.length -= n;
                Ok(n)
            }
            Self::ConsoleRead => {
                if bytes.is_empty() {
                    return Ok(0);
                }
                if let Some(byte) = super::input_byte() {
                    bytes[0] = byte;
                    Ok(1)
                } else {
                    Err(-11)
                }
            }
            _ => Err(-9),
        }
    }
    pub fn write(&self, bytes: &[u8]) -> Result<usize, i64> {
        match self {
            Self::File(fd) => crate::fs::VFS
                .lock()
                .write(*fd, bytes)
                .map_err(|e| crate::syscall::fs_err(e) as i64),
            Self::ConsoleWrite => {
                crate::syscall::print_bytes(bytes);
                Ok(bytes.len())
            }
            Self::PipeWrite(pipe) => {
                let mut p = pipe.lock();
                if bytes.is_empty() {
                    return Ok(0);
                }
                if p.readers == 0 {
                    return Err(-32);
                }
                // Up to PIPE_BUF bytes are atomic; sleep until the full request fits.
                if bytes.len() > 4096 - p.length {
                    return Err(-11);
                }
                for byte in bytes {
                    let tail = (p.head + p.length) % 4096;
                    p.bytes[tail] = *byte;
                    p.length += 1;
                }
                Ok(bytes.len())
            }
            _ => Err(-9),
        }
    }
    pub fn seek(&self, offset: i64, whence: u32) -> Result<i64, i64> {
        match self {
            Self::File(fd) => crate::fs::VFS
                .lock()
                .seek(*fd, offset, whence)
                .map(|n| n as i64)
                .map_err(|e| crate::syscall::fs_err(e) as i64),
            _ => Err(-29),
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        match self {
            Self::File(fd) => {
                let _ = crate::fs::VFS.lock().close(*fd);
            }
            Self::PipeRead(pipe) => {
                pipe.lock().readers -= 1;
            }
            Self::PipeWrite(pipe) => {
                pipe.lock().writers -= 1;
            }
            _ => {}
        }
    }
}
