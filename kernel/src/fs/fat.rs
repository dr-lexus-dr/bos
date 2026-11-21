//! FAT volumes on a dedicated data disk. Access is serialized across handles.
use super::*;
use crate::drivers::data_disk;
use alloc::sync::Arc;
use fatfs::{Error, IoBase, Read, Seek, SeekFrom, Write};
use spin::Mutex;

type ResultFs<T> = Result<T, FileSystemError>;
type Volume = fatfs::FileSystem<Disk>;
struct Disk {
    start: u64,
    pos: u64,
    len: u64,
    cache: [(u64, [u8; 512]); 8],
}
impl IoBase for Disk {
    type Error = ();
}
impl Disk {
    fn sector(&mut self, lba: u64) -> Result<&[u8; 512], ()> {
        let slot = &mut self.cache[lba as usize % 8];
        if slot.0 != lba {
            slot.0 = u64::MAX;
            data_disk::read(lba, &mut slot.1).map_err(|_| ())?;
            slot.0 = lba;
        }
        Ok(&slot.1)
    }
    fn chunk(&self, n: usize) -> (u64, usize, usize) {
        let offset = (self.pos % 512) as usize;
        (
            self.start + self.pos / 512,
            offset,
            n.min(512 - offset).min((self.len - self.pos) as usize),
        )
    }
}
impl Read for Disk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        let (lba, offset, n) = self.chunk(buf.len());
        if n == 0 {
            return Ok(0);
        }
        buf[..n].copy_from_slice(&self.sector(lba)?[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}
impl Write for Disk {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        let (lba, offset, n) = self.chunk(buf.len());
        if n == 0 {
            return Ok(0);
        }
        let mut sector = [0; 512];
        if n != 512 {
            sector = *self.sector(lba)?;
        }
        sector[offset..offset + n].copy_from_slice(&buf[..n]);
        let slot = &mut self.cache[lba as usize % 8];
        slot.0 = u64::MAX;
        data_disk::write(lba, &sector).map_err(|_| ())?;
        *slot = (lba, sector);
        self.pos += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> Result<(), ()> {
        data_disk::flush().map_err(|_| ())
    }
}
impl Seek for Disk {
    fn seek(&mut self, from: SeekFrom) -> Result<u64, ()> {
        let n = match from {
            SeekFrom::Start(x) => Some(x),
            SeekFrom::Current(x) => self.pos.checked_add_signed(x),
            SeekFrom::End(x) => self.len.checked_add_signed(x),
        }
        .filter(|&n| n <= self.len)
        .ok_or(())?;
        self.pos = n;
        Ok(n)
    }
}
fn error(e: Error<()>) -> FileSystemError {
    use FileSystemError as F;
    match e {
        Error::NotFound => F::NotFound,
        Error::AlreadyExists => F::AlreadyExists,
        Error::DirectoryIsNotEmpty => F::NotEmpty,
        Error::InvalidFileNameLength => F::NameTooLong,
        Error::NotEnoughSpace | Error::WriteZero => F::NoSpace,
        Error::UnsupportedFileNameCharacter | Error::InvalidInput => F::InvalidArgument,
        Error::CorruptedFileSystem => F::InvalidFormat,
        _ => F::IoError,
    }
}
fn inode(path: &str) -> u64 {
    path.bytes().fold(0xcbf29ce484222325, |a, b| {
        (a ^ (b.to_ascii_lowercase() as u64)).wrapping_mul(0x100000001b3)
    })
}
fn info(path: &str, size: u64, dir: bool) -> FileStat {
    FileStat {
        inode: inode(path),
        size,
        mode: if dir {
            S_IFDIR | 0o755
        } else {
            S_IFREG | 0o644
        },
        uid: 0,
        gid: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        links: 1,
    }
}
fn metadata(fs: &Volume, path: &str) -> ResultFs<FileStat> {
    let (parent, name) = split_parent(path);
    if name.is_empty() {
        return Ok(info("/", 0, true));
    }
    let dir = if parent == "/" {
        fs.root_dir()
    } else {
        fs.root_dir().open_dir(&parent).map_err(error)?
    };
    for e in dir.iter() {
        let e = e.map_err(error)?;
        if e.file_name().to_lowercase() == name.to_lowercase() {
            return Ok(info(path, e.len(), e.is_dir()));
        }
    }
    Err(FileSystemError::NotFound)
}
pub struct Fat {
    volume: Arc<Mutex<Volume>>,
}
impl Fat {
    pub fn load(sectors: u64, labelled: bool) -> ResultFs<Self> {
        let mut header = [0; 512];
        data_disk::read(0, &mut header).map_err(|_| FileSystemError::IoError)?;
        let mut candidates = alloc::vec![(0, sectors)];
        if header[510..] == [0x55, 0xaa] {
            for e in header[446..510].chunks_exact(16) {
                if matches!(e[4], 1 | 4 | 6 | 14) {
                    let start = u32::from_le_bytes(e[8..12].try_into().unwrap()) as u64;
                    let len = u32::from_le_bytes(e[12..16].try_into().unwrap()) as u64;
                    if start > 0 && len > 0 && start + len <= sectors {
                        candidates.push((start, len));
                    }
                }
            }
        }
        for (start, len) in candidates {
            data_disk::read(start, &mut header).map_err(|_| FileSystemError::IoError)?;
            if header[510..] != [0x55, 0xaa]
                || header[11..13] != [0, 2]
                || header[17..19] == [0, 0]
                || !matches!(header[0], 0xeb | 0xe9)
                || (labelled && &header[43..54] != b"BOSDATA    ")
            {
                continue;
            }
            let disk = Disk {
                start,
                pos: 0,
                cache: [(u64::MAX, [0; 512]); 8],
                len: len.checked_mul(512).ok_or(FileSystemError::InvalidFormat)?,
            };
            if let Ok(fs) = Volume::new(disk, fatfs::FsOptions::new()) {
                if matches!(fs.fat_type(), fatfs::FatType::Fat12 | fatfs::FatType::Fat16) {
                    return Ok(Self {
                        volume: Arc::new(Mutex::new(fs)),
                    });
                }
            }
        }
        Err(FileSystemError::InvalidFormat)
    }
}
struct FatFile {
    volume: Arc<Mutex<Volume>>,
    path: String,
    flags: u32,
    pos: u64,
}
fn extend(
    file: &mut fatfs::File<Disk, fatfs::DefaultTimeProvider, fatfs::LossyOemCpConverter>,
    end: u64,
) -> ResultFs<()> {
    if end > u32::MAX as u64 {
        return Err(FileSystemError::NoSpace);
    }
    let mut pos = file.seek(SeekFrom::Start(end)).map_err(error)?;
    let zeros = [0; 512];
    while pos < end {
        let n = (end - pos).min(512) as usize;
        file.write_all(&zeros[..n]).map_err(error)?;
        pos += n as u64;
    }
    Ok(())
}
impl File for FatFile {
    fn read(&mut self, buf: &mut [u8]) -> ResultFs<usize> {
        if self.flags & O_ACCMODE == O_WRONLY {
            return Err(FileSystemError::BadFileDescriptor);
        }
        let fs = self.volume.lock();
        let mut f = fs.root_dir().open_file(&self.path).map_err(error)?;
        if f.seek(SeekFrom::Start(self.pos)).map_err(error)? != self.pos {
            return Ok(0);
        }
        let n = f.read(buf).map_err(error)?;
        self.pos += n as u64;
        Ok(n)
    }
    fn write(&mut self, buf: &[u8]) -> ResultFs<usize> {
        if self.flags & O_ACCMODE == O_RDONLY {
            return Err(FileSystemError::BadFileDescriptor);
        }
        let fs = self.volume.lock();
        let mut f = fs.root_dir().open_file(&self.path).map_err(error)?;
        if self.flags & O_APPEND != 0 {
            self.pos = f.seek(SeekFrom::End(0)).map_err(error)?;
        } else {
            extend(&mut f, self.pos)?;
        }
        let n = f.write(buf).map_err(error)?;
        f.flush().map_err(error)?;
        self.pos += n as u64;
        Ok(n)
    }
    fn seek(&mut self, pos: i64, whence: u32) -> ResultFs<u64> {
        let base = match whence {
            SEEK_SET => 0,
            SEEK_CUR => self.pos,
            SEEK_END => self.stat()?.size,
            _ => return Err(FileSystemError::InvalidArgument),
        };
        self.pos = base
            .checked_add_signed(pos)
            .filter(|&p| p <= u32::MAX as u64)
            .ok_or(FileSystemError::InvalidArgument)?;
        Ok(self.pos)
    }
    fn stat(&self) -> ResultFs<FileStat> {
        metadata(&self.volume.lock(), &self.path)
    }
    fn close(&mut self) -> ResultFs<()> {
        if self.flags & O_ACCMODE != O_RDONLY {
            self.sync()?;
        }
        Ok(())
    }
    fn sync(&mut self) -> ResultFs<()> {
        data_disk::flush().map_err(|_| FileSystemError::IoError)
    }
}
impl FileSystem for Fat {
    fn name(&self) -> &str {
        "fat"
    }
    fn open(&self, path: &str, flags: u32) -> ResultFs<Box<dyn File>> {
        let fs = self.volume.lock();
        let mut f = match metadata(&fs, path) {
            Ok(s) if s.is_dir() => return Err(FileSystemError::IsDirectory),
            Ok(_) if flags & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL => {
                return Err(FileSystemError::AlreadyExists)
            }
            Ok(_) => fs.root_dir().open_file(path).map_err(error)?,
            Err(FileSystemError::NotFound) if flags & O_CREAT != 0 => {
                fs.root_dir().create_file(path).map_err(error)?
            }
            Err(e) => return Err(e),
        };
        if flags & O_TRUNC != 0 && flags & O_ACCMODE != O_RDONLY {
            f.truncate().map_err(error)?;
            f.flush().map_err(error)?;
        }
        Ok(Box::new(FatFile {
            volume: self.volume.clone(),
            path: path.into(),
            flags,
            pos: 0,
        }))
    }
    fn create(&self, path: &str, kind: FileType) -> ResultFs<()> {
        if kind == FileType::Directory {
            return self.mkdir(path);
        }
        if kind != FileType::RegularFile {
            return Err(FileSystemError::NotImplemented);
        }
        self.open(path, O_CREAT | O_EXCL | O_WRONLY)?.close()
    }
    fn remove(&self, path: &str) -> ResultFs<()> {
        self.volume.lock().root_dir().remove(path).map_err(error)
    }
    fn mkdir(&self, path: &str) -> ResultFs<()> {
        let fs = self.volume.lock();
        if metadata(&fs, path).is_ok() {
            return Err(FileSystemError::AlreadyExists);
        }
        let result = fs.root_dir().create_dir(path).map(|_| ()).map_err(error);
        result
    }
    fn list_dir(&self, path: &str) -> ResultFs<Vec<DirEntry>> {
        let fs = self.volume.lock();
        let mut items = Vec::new();
        let dir = if path == "/" {
            fs.root_dir()
        } else {
            fs.root_dir().open_dir(path).map_err(error)?
        };
        for e in dir.iter() {
            let e = e.map_err(error)?;
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            items.push(DirEntry {
                inode: inode(&alloc::format!("{path}/{name}")),
                name,
                file_type: if e.is_dir() {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                },
            });
        }
        Ok(items)
    }
    fn stat(&self, path: &str) -> ResultFs<FileStat> {
        metadata(&self.volume.lock(), path)
    }
    fn rename(&self, from: &str, to: &str) -> ResultFs<()> {
        let fs = self.volume.lock();
        let result = fs
            .root_dir()
            .rename(from, &fs.root_dir(), to)
            .map_err(error);
        result
    }
    fn truncate(&self, path: &str, len: u64) -> ResultFs<()> {
        let fs = self.volume.lock();
        let mut f = fs.root_dir().open_file(path).map_err(error)?;
        extend(&mut f, len)?;
        f.truncate().map_err(error)?;
        f.flush().map_err(error)
    }
    fn mount(&mut self, _: &str, _: &str) -> ResultFs<()> {
        Ok(())
    }
    fn unmount(&mut self) -> ResultFs<()> {
        self.sync()
    }
    fn sync(&self) -> ResultFs<()> {
        data_disk::flush().map_err(|_| FileSystemError::IoError)
    }
}
