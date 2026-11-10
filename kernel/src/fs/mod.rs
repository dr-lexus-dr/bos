//! Filesystem Abstraction Layer (VFS).
//!
//! Linux-like semantics: absolute + relative paths resolved against a
//! per-VFS current working directory, `open()` flags (`O_*`), `lseek`
//! whence codes, mode bits (`S_IFMT` and friends), symlinks, rename,
//! truncate and chmod. Concrete filesystems (`ramfs`, `bFS`) implement
//! the [`FileSystem`] trait; [`Vfs`] routes by longest-prefix mount match.

pub mod bfs;
pub mod ramfs;

use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::boxed::Box;
use log::info;
use lazy_static::lazy_static;

// ---- open() flags (Linux subset) ----
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_ACCMODE: u32 = 3;
pub const O_CREAT: u32 = 0x40;
pub const O_EXCL: u32 = 0x80;
pub const O_TRUNC: u32 = 0x200;
pub const O_APPEND: u32 = 0x400;
pub const O_DIRECTORY: u32 = 0x10000;

// ---- lseek whence ----
pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

// ---- mode bits (stat, mkdir, chmod) ----
pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFIFO: u32 = 0o010000;

/// File types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Unknown,
    RegularFile,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

/// File statistics
#[derive(Debug, Clone)]
pub struct FileStat {
    pub inode: u64,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub links: u32,
}

impl FileStat {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
    pub fn is_symlink(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }
}

/// Directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub inode: u64,
    pub name: String,
    pub file_type: FileType,
}

/// Mount table entry (for `mount` listing / `df`).
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub readonly: bool,
}

/// File system errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemError {
    NotFound,
    PermissionDenied,
    InvalidArgument,
    NoSpace,
    NotDirectory,
    IsDirectory,
    NotEmpty,
    AlreadyExists,
    InvalidFormat,
    UnsupportedVersion,
    IoError,
    NotImplemented,
    NoSuchDevice,
    TooManyOpenFiles,
    BadFileDescriptor,
    SymlinkLoop,
    NameTooLong,
    ReadOnly,
}

/// File trait
pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileSystemError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FileSystemError>;
    fn seek(&mut self, pos: i64, whence: u32) -> Result<u64, FileSystemError>;
    fn close(&mut self) -> Result<(), FileSystemError>;
    fn stat(&self) -> Result<FileStat, FileSystemError>;
    fn sync(&mut self) -> Result<(), FileSystemError> {
        Ok(())
    }
}

/// File system trait. Path arguments are FS-relative (`/` == FS root);
/// the VFS strips the mount prefix before delegating.
pub trait FileSystem: Send + Sync {
    fn name(&self) -> &str;
    fn open(&self, path: &str, flags: u32) -> Result<Box<dyn File>, FileSystemError>;
    fn create(&self, path: &str, file_type: FileType) -> Result<(), FileSystemError>;
    fn remove(&self, path: &str) -> Result<(), FileSystemError>;
    fn mkdir(&self, path: &str) -> Result<(), FileSystemError>;
    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, FileSystemError>;
    fn stat(&self, path: &str) -> Result<FileStat, FileSystemError>;
    fn mount(&mut self, device: &str, options: &str) -> Result<(), FileSystemError>;
    fn unmount(&mut self) -> Result<(), FileSystemError>;
    fn sync(&self) -> Result<(), FileSystemError> {
        Ok(())
    }
    // --- optional POSIX-like ops (default: not implemented) ---
    fn rename(&self, _from: &str, _to: &str) -> Result<(), FileSystemError> {
        Err(FileSystemError::NotImplemented)
    }
    fn symlink(&self, _target: &str, _link: &str) -> Result<(), FileSystemError> {
        Err(FileSystemError::NotImplemented)
    }
    fn readlink(&self, _path: &str) -> Result<String, FileSystemError> {
        Err(FileSystemError::NotImplemented)
    }
    fn truncate(&self, _path: &str, _len: u64) -> Result<(), FileSystemError> {
        Err(FileSystemError::NotImplemented)
    }
    fn chmod(&self, _path: &str, _mode: u32) -> Result<(), FileSystemError> {
        Err(FileSystemError::NotImplemented)
    }
}

/// Normalize a user path against `cwd` into a canonical absolute path.
///
/// Handles `.`, `..`, duplicate and trailing slashes. Never escapes `/`.
/// Returns `Err(InvalidArgument)` for empty paths and `Err(NameTooLong)`
/// for components longer than 255 bytes or paths over 4096 bytes.
pub fn normalize_path(cwd: &str, path: &str) -> Result<String, FileSystemError> {
    if path.is_empty() {
        return Err(FileSystemError::InvalidArgument);
    }
    if path.len() > 4096 {
        return Err(FileSystemError::NameTooLong);
    }
    let mut parts: Vec<String> = Vec::new();
    // seed with cwd for relative paths
    if !path.starts_with('/') {
        for c in cwd.split('/') {
            if c.is_empty() || c == "." {
                continue;
            }
            if c == ".." {
                parts.pop();
            } else {
                parts.push(c.to_string());
            }
        }
    }
    for c in path.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            parts.pop();
            continue;
        }
        if c.len() > 255 {
            return Err(FileSystemError::NameTooLong);
        }
        parts.push(c.to_string());
    }
    if parts.is_empty() {
        return Ok(String::from("/"));
    }
    let mut out = String::from("/");
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(p);
    }
    Ok(out)
}

/// Split an absolute path into (parent_dir, file_name).
#[allow(dead_code)]
pub fn split_parent(path: &str) -> (String, String) {
    let t = path.trim_end_matches('/');
    if t.is_empty() {
        return (String::from("/"), String::new());
    }
    match t.rfind('/') {
        None => (String::from("/"), t.to_string()),
        Some(0) => (String::from("/"), t[1..].to_string()),
        Some(i) => (t[..i].to_string(), t[i + 1..].to_string()),
    }
}

/// Virtual File System (VFS)
pub struct Vfs {
    filesystems: Vec<String>,
    mounts: Vec<MountPoint>,
    file_handles: Vec<Option<Box<dyn File>>>,
    cwd: String,
}

struct MountPoint {
    device: String,
    mount_point: String,
    fs_type: String,
    fs: Box<dyn FileSystem>,
    readonly: bool,
}

impl Vfs {
    pub fn new() -> Self {
        Self {
            filesystems: Vec::new(),
            mounts: Vec::new(),
            file_handles: Vec::new(),
            cwd: String::from("/"),
        }
    }

    pub fn register_filesystem(&mut self, fs: Box<dyn FileSystem>) {
        let name = fs.name().to_string();
        info!("Registered filesystem: {}", name);
        if !self.filesystems.iter().any(|n| *n == name) {
            // Keep one prototype instance alive for `mount()` by name.
            // Prototypes are never unmounted; mounts get fresh instances.
            let _ = fs;
            self.filesystems.push(name);
        }
    }

    /// Mount a fresh filesystem instance by registered type name.
    /// Unlike the old implementation this does NOT consume the registry:
    /// mounts are independent instances, so the same type can be mounted
    /// many times.
    pub fn mount(
        &mut self,
        device: &str,
        mount_point: &str,
        fs_type: &str,
        options: &str,
    ) -> Result<(), FileSystemError> {
        let fs_type = if fs_type == "bfs" { "bFS" } else { fs_type };
        if !self.filesystems.iter().any(|n| n == fs_type) {
            return Err(FileSystemError::NoSuchDevice);
        }
        let mut fs: Box<dyn FileSystem> = match fs_type {
            "ramfs" => Box::new(crate::fs::ramfs::RamFs::new()),
            "bFS" => Box::new(crate::fs::bfs::BfsFileSystem::new(device)),
            _ => return Err(FileSystemError::NoSuchDevice),
        };
        fs.mount(device, options)?;
        self.mount_direct(mount_point, device, fs_type, fs, options.contains("ro"))
    }

    /// Mount a caller-constructed instance (e.g. pre-formatted bFS).
    pub fn mount_fs(
        &mut self,
        mount_point: &str,
        device: &str,
        fs: Box<dyn FileSystem>,
        readonly: bool,
    ) -> Result<(), FileSystemError> {
        let t = fs.name().to_string();
        self.mount_direct(mount_point, device, &t, fs, readonly)
    }

    fn mount_direct(
        &mut self,
        mount_point: &str,
        device: &str,
        fs_type: &str,
        fs: Box<dyn FileSystem>,
        readonly: bool,
    ) -> Result<(), FileSystemError> {
        let mp = normalize_path("/", mount_point)?;
        if self.mounts.iter().any(|m| m.mount_point == mp) {
            return Err(FileSystemError::AlreadyExists);
        }
        self.mounts.push(MountPoint {
            device: device.to_string(),
            mount_point: mp.clone(),
            fs_type: fs_type.to_string(),
            fs,
            readonly,
        });
        info!("Mounted {} on {}", device, mp);
        Ok(())
    }

    pub fn unmount(&mut self, mount_point: &str) -> Result<(), FileSystemError> {
        let mp = normalize_path("/", mount_point)?;
        let idx =
            self.mounts.iter().position(|m| m.mount_point == mp).ok_or(FileSystemError::NotFound)?;
        self.mounts[idx].fs.unmount()?;
        self.mounts.remove(idx);
        Ok(())
    }

    pub fn sync(&self) -> Result<(), FileSystemError> {
        for mount in &self.mounts {
            mount.fs.sync()?;
        }
        Ok(())
    }

    pub fn mounts(&self) -> Vec<MountInfo> {
        self.mounts
            .iter()
            .map(|m| MountInfo {
                device: m.device.clone(),
                mount_point: m.mount_point.clone(),
                fs_type: m.fs_type.clone(),
                readonly: m.readonly,
            })
            .collect()
    }

    // ---- cwd ----
    pub fn get_cwd(&self) -> String {
        self.cwd.clone()
    }

    pub fn set_cwd(&mut self, path: &str) -> Result<(), FileSystemError> {
        let abs = normalize_path(&self.cwd, path)?;
        // must exist and be a directory (follow symlinks, capped)
        let st = self.stat_follow(&abs, 0)?;
        if !st.is_dir() {
            return Err(FileSystemError::NotDirectory);
        }
        self.cwd = abs;
        Ok(())
    }

    /// Resolve a user path to an absolute canonical path (no symlink
    /// following; lexical only).
    pub fn abspath(&self, path: &str) -> Result<String, FileSystemError> {
        normalize_path(&self.cwd, path)
    }

    fn resolve_path(&self, path: &str) -> Option<(&MountPoint, String)> {
        // `path` must already be absolute+canonical.
        let mut best: Option<&MountPoint> = None;
        let mut best_len = 0usize;
        for mount in &self.mounts {
            let mp = &mount.mount_point;
            let hit = if mp == "/" {
                path.starts_with('/')
            } else {
                path == mp || path.starts_with(&alloc::format!("{}/", mp))
            };
            if hit && mp.len() > best_len {
                best_len = mp.len();
                best = Some(mount);
            }
        }
        best.map(|mount| {
            let rel = if path.len() > best_len {
                &path[best_len..]
            } else {
                "/"
            };
            (mount, rel.to_string())
        })
    }

    /// Follow symlinks (up to 8 hops) and return the final absolute path.
    fn follow(&self, abs: &str, depth: u32) -> Result<String, FileSystemError> {
        if depth > 8 {
            return Err(FileSystemError::SymlinkLoop);
        }
        let (mount, rel) = self.resolve_path(abs).ok_or(FileSystemError::NotFound)?;
        // Check each prefix of the FS-relative path for symlinks.
        let comps: Vec<&str> = rel.split('/').filter(|c| !c.is_empty()).collect();
        let mut prefix = String::from("/");
        for (i, c) in comps.iter().enumerate() {
            if prefix.len() > 1 {
                prefix.push('/');
            }
            prefix.push_str(c);
            match mount.fs.readlink(&prefix) {
                Ok(target) => {
                    // Absolute target is FS-relative; relative target
                    // resolves against the link's parent directory.
                    let mut base = mount.mount_point.clone();
                    for k in comps[..i].iter() {
                        if base != "/" {
                            base.push('/');
                        }
                        base.push_str(k);
                    }
                    let joined = if target.starts_with('/') {
                        normalize_path("/", &target)?
                    } else {
                        normalize_path(&base, &target)?
                    };
                    // append remainder
                    let mut rest = joined;
                    for k in comps[i + 1..].iter() {
                        rest.push('/');
                        rest.push_str(k);
                    }
                    return self.follow(&rest, depth + 1);
                }
                Err(FileSystemError::NotImplemented) => break, // FS has no symlinks
                Err(FileSystemError::NotFound) => continue,    // not a symlink, keep walking
                Err(FileSystemError::InvalidArgument) => continue, // e.g. RamFS readlink on non-link
                Err(e) => return Err(e),
            }
        }
        Ok(abs.to_string())
    }

    fn stat_follow(&self, abs: &str, _depth: u32) -> Result<FileStat, FileSystemError> {
        let real = self.follow(abs, 0)?;
        let (mount, rel) = self.resolve_path(&real).ok_or(FileSystemError::NotFound)?;
        mount.fs.stat(&rel)
    }

    fn is_readonly(&self, abs: &str) -> bool {
        self.resolve_path(abs).map(|(m, _)| m.readonly).unwrap_or(false)
    }

    fn check_writable(&self, abs: &str) -> Result<(), FileSystemError> {
        if self.is_readonly(abs) {
            return Err(FileSystemError::ReadOnly);
        }
        Ok(())
    }

    // ---- fd-based I/O ----
    pub fn open(&mut self, path: &str, flags: u32) -> Result<u32, FileSystemError> {
        let abs = self.abspath(path)?;
        let acc = flags & O_ACCMODE;
        if acc != O_RDONLY && (flags & (O_CREAT | O_TRUNC | O_APPEND) != 0 || acc != O_RDONLY) {
            self.check_writable(&abs)?;
        } else if acc != O_RDONLY {
            self.check_writable(&abs)?;
        }
        if flags & O_DIRECTORY != 0 {
            let st = self.stat_follow(&abs, 0)?;
            if !st.is_dir() {
                return Err(FileSystemError::NotDirectory);
            }
        }
        let real = self.follow(&abs, 0)?;
        let (mount, relative_path) =
            self.resolve_path(&real).ok_or(FileSystemError::NotFound)?;

        let file = mount.fs.open(&relative_path, flags)?;

        for (i, handle) in self.file_handles.iter_mut().enumerate() {
            if handle.is_none() {
                *handle = Some(file);
                return Ok(i as u32);
            }
        }
        let fd = self.file_handles.len() as u32;
        if fd >= 1024 {
            return Err(FileSystemError::TooManyOpenFiles);
        }
        self.file_handles.push(Some(file));
        Ok(fd)
    }

    pub fn close(&mut self, fd: u32) -> Result<(), FileSystemError> {
        if fd >= self.file_handles.len() as u32 {
            return Err(FileSystemError::BadFileDescriptor);
        }
        if self.file_handles[fd as usize].is_none() {
            return Err(FileSystemError::BadFileDescriptor);
        }
        self.file_handles[fd as usize] = None;
        Ok(())
    }

    pub fn read(&mut self, fd: u32, buf: &mut [u8]) -> Result<usize, FileSystemError> {
        if fd >= self.file_handles.len() as u32 {
            return Err(FileSystemError::BadFileDescriptor);
        }
        if let Some(file) = &mut self.file_handles[fd as usize] {
            file.read(buf)
        } else {
            Err(FileSystemError::BadFileDescriptor)
        }
    }

    pub fn write(&mut self, fd: u32, buf: &[u8]) -> Result<usize, FileSystemError> {
        if fd >= self.file_handles.len() as u32 {
            return Err(FileSystemError::BadFileDescriptor);
        }
        if let Some(file) = &mut self.file_handles[fd as usize] {
            file.write(buf)
        } else {
            Err(FileSystemError::BadFileDescriptor)
        }
    }

    pub fn seek(&mut self, fd: u32, pos: i64, whence: u32) -> Result<u64, FileSystemError> {
        if fd >= self.file_handles.len() as u32 {
            return Err(FileSystemError::BadFileDescriptor);
        }
        if let Some(file) = &mut self.file_handles[fd as usize] {
            file.seek(pos, whence)
        } else {
            Err(FileSystemError::BadFileDescriptor)
        }
    }

    pub fn fstat(&self, fd: u32) -> Result<FileStat, FileSystemError> {
        if fd >= self.file_handles.len() as u32 {
            return Err(FileSystemError::BadFileDescriptor);
        }
        if let Some(file) = &self.file_handles[fd as usize] {
            file.stat()
        } else {
            Err(FileSystemError::BadFileDescriptor)
        }
    }

    // ---- path-based ops ----
    pub fn stat(&self, path: &str) -> Result<FileStat, FileSystemError> {
        let abs = self.abspath(path)?;
        self.stat_follow(&abs, 0)
    }

    pub fn lstat(&self, path: &str) -> Result<FileStat, FileSystemError> {
        let abs = self.abspath(path)?;
        let (mount, rel) = self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.stat(&rel)
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, FileSystemError> {
        let abs = self.abspath(path)?;
        let real = self.follow(&abs, 0)?;
        let (mount, relative_path) =
            self.resolve_path(&real).ok_or(FileSystemError::NotFound)?;
        mount.fs.list_dir(&relative_path)
    }

    pub fn create(&self, path: &str, file_type: FileType) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let (mount, relative_path) =
            self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.create(&relative_path, file_type)
    }

    pub fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let (mount, relative_path) =
            self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.mkdir(&relative_path)
    }

    /// `mkdir -p`: create all missing parents. Idempotent.
    pub fn mkdir_p(&self, path: &str) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let mut cur = String::from("/");
        for comp in abs.split('/').filter(|c| !c.is_empty()) {
            if cur != "/" {
                cur.push('/');
            }
            cur.push_str(comp);
            match self.mkdir(&cur) {
                Ok(()) => {}
                Err(FileSystemError::AlreadyExists) => {
                    if !self.stat(&cur)?.is_dir() {
                        return Err(FileSystemError::AlreadyExists);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let (mount, relative_path) =
            self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.remove(&relative_path)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), FileSystemError> {
        let a = self.abspath(from)?;
        let b = self.abspath(to)?;
        self.check_writable(&a)?;
        self.check_writable(&b)?;
        let (ma, ra) = self.resolve_path(&a).ok_or(FileSystemError::NotFound)?;
        let (mb, rb) = self.resolve_path(&b).ok_or(FileSystemError::NotFound)?;
        if ma.mount_point != mb.mount_point {
            return Err(FileSystemError::InvalidArgument); // no cross-mount rename
        }
        ma.fs.rename(&ra, &rb)
    }

    pub fn symlink(&self, target: &str, link: &str) -> Result<(), FileSystemError> {
        let abs = self.abspath(link)?;
        self.check_writable(&abs)?;
        let (mount, rel) = self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.symlink(target, &rel)
    }

    pub fn readlink(&self, path: &str) -> Result<String, FileSystemError> {
        let abs = self.abspath(path)?;
        let (mount, rel) = self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.readlink(&rel)
    }

    pub fn truncate(&self, path: &str, len: u64) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let real = self.follow(&abs, 0)?;
        let (mount, rel) = self.resolve_path(&real).ok_or(FileSystemError::NotFound)?;
        mount.fs.truncate(&rel, len)
    }

    pub fn chmod(&self, path: &str, mode: u32) -> Result<(), FileSystemError> {
        let abs = self.abspath(path)?;
        self.check_writable(&abs)?;
        let (mount, rel) = self.resolve_path(&abs).ok_or(FileSystemError::NotFound)?;
        mount.fs.chmod(&rel, mode)
    }

    // ---- convenience whole-file I/O (for shells / cp / cat) ----
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FileSystemError> {
        let fd = self.open(path, O_RDONLY)?;
        let mut out = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = self.read(fd, &mut chunk)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        self.close(fd)?;
        Ok(out)
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), FileSystemError> {
        let fd = self.open(path, O_WRONLY | O_CREAT | O_TRUNC)?;
        let mut off = 0;
        while off < data.len() {
            let n = self.write(fd, &data[off..])?;
            if n == 0 {
                self.close(fd)?;
                return Err(FileSystemError::NoSpace);
            }
            off += n;
        }
        self.close(fd)?;
        Ok(())
    }

    pub fn append_file(&mut self, path: &str, data: &[u8]) -> Result<(), FileSystemError> {
        let fd = self.open(path, O_WRONLY | O_CREAT | O_APPEND)?;
        let mut off = 0;
        while off < data.len() {
            let n = self.write(fd, &data[off..])?;
            if n == 0 {
                self.close(fd)?;
                return Err(FileSystemError::NoSpace);
            }
            off += n;
        }
        self.close(fd)?;
        Ok(())
    }
}

lazy_static! {
    pub static ref VFS: spin::Mutex<Vfs> = spin::Mutex::new(Vfs::new());
}

/// Initialize VFS: ramfs on `/` + skeleton dirs, initrd tarball imported.
pub fn init(boot_info: &bloader_shared::BootInfo) -> &'static str {
    // Register known FS types, then mount a fresh ramfs at /.
    VFS.lock().register_filesystem(Box::new(crate::fs::ramfs::RamFs::new()));
    VFS.lock().register_filesystem(Box::new(crate::fs::bfs::BfsFileSystem::new("hd0")));
    {
        let mut vfs = VFS.lock();
        if let Err(e) = vfs.mount("ramdisk", "/", "ramfs", "") {
            info!("VFS: root mount failed: {:?}", e);
            return "VFS initialization failed";
        }
        for path in [
            "/applications",
            "/system/logs",
            "/system/mounts",
            "/system/tmp",
            "/users/default/downloads",
            "/users/default/documents",
            "/users/default/pictures",
            "/users/default/music",
            "/users/default/videos",
        ] {
            if let Err(error) = vfs.mkdir_p(path) {
                info!("VFS: could not create {}: {:?}", path, error);
            }
        }
        if let Err(error) = vfs.create("/users/default/documents/notes.txt", FileType::RegularFile) {
            info!("VFS: could not create default notes: {:?}", error);
        }
    }

    // Import initrd tarball into / (bundled files).
    if let (Some(addr), Some(size)) = (boot_info.initrd_addr, boot_info.initrd_size) {
        if size > 0 {
            // Initrd pages live below 4GB: identity-mapped, safe to read.
            let data = unsafe { core::slice::from_raw_parts(addr as *const u8, size as usize) };
            match crate::fs::ramfs::RamFs::import_tar_vfs(data) {
                Ok(n) => info!("VFS: imported {} initrd entries", n),
                Err(e) => info!("VFS: initrd import failed: {:?}", e),
            }
        }
    }

    let storage = match crate::drivers::virtio_block::probe() {
        Ok(Some(sectors)) => {
            match bfs::BfsFileSystem::load_users_disk(sectors)
                .and_then(|disk| VFS.lock().mount_fs("/users", "virtio0", Box::new(disk), false))
            {
                Ok(()) => "/users is persistent on virtio0",
                Err(error) => {
                    log::warn!("bFS data disk rejected: {:?}", error);
                    "Data disk rejected; /users is in RAM"
                }
            }
        }
        Ok(None) => "No data disk; /users is in RAM",
        Err(error) => {
            log::warn!("VirtIO data disk unavailable: {}", error);
            "Data disk unavailable; /users is in RAM"
        }
    };
    info!("VFS initialized");
    storage
}
