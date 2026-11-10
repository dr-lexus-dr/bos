//! RamFS: in-memory filesystem implementing the VFS [`FileSystem`] trait.
//!
//! Tree of dirs/files/symlinks in the heap, shared via
//! `Arc<Mutex<DirNode>>` so open file handles stay valid. No block layer
//! needed; the initrd tarball is imported at mount time by `import_tar_vfs`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use super::{
    DirEntry, File, FileStat, FileSystem, FileSystemError, FileType, O_ACCMODE, O_APPEND, O_CREAT,
    O_DIRECTORY, O_EXCL, O_RDWR, O_TRUNC, O_WRONLY, S_IFDIR, S_IFLNK, S_IFMT, S_IFREG, SEEK_CUR,
    SEEK_END, SEEK_SET,
};

static NEXT_INODE: AtomicU64 = AtomicU64::new(2); // 1 == root
fn fresh_ino() -> u64 {
    NEXT_INODE.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug)]
enum Node {
    File {
        inode: u64,
        data: Vec<u8>,
        mode: u32, // S_IFREG | perms
        uid: u32,
        gid: u32,
    },
    Dir {
        inode: u64,
        entries: BTreeMap<String, Node>,
        mode: u32, // S_IFDIR | perms
        uid: u32,
        gid: u32,
    },
    Symlink {
        inode: u64,
        target: String,
        uid: u32,
        gid: u32,
    },
}

impl Node {
    fn inode(&self) -> u64 {
        match self {
            Node::File { inode, .. } | Node::Dir { inode, .. } | Node::Symlink { inode, .. } => {
                *inode
            }
        }
    }
    fn is_dir(&self) -> bool {
        matches!(self, Node::Dir { .. })
    }
    #[allow(dead_code)]
    fn mode(&self) -> u32 {
        match self {
            Node::File { mode, .. } | Node::Dir { mode, .. } => *mode,
            Node::Symlink { .. } => S_IFLNK | 0o777,
        }
    }
}

#[derive(Debug)]
struct DirNode {
    root: Node, // always a Dir
}

pub struct RamFs {
    inner: Arc<Mutex<DirNode>>,
}

impl RamFs {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DirNode {
                root: Node::Dir { inode: 1, entries: BTreeMap::new(), mode: S_IFDIR | 0o755, uid: 0, gid: 0 },
            })),
        }
    }

    /// Split an FS-relative path into clean components. "" and "/" -> [].
    fn comps(path: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                name => out.push(name.to_string()),
            }
        }
        out
    }

    fn get<'a>(dir: &'a Node, comps: &[String]) -> Result<&'a Node, FileSystemError> {
        let mut cur = dir;
        for c in comps {
            match cur {
                Node::Dir { entries, .. } => {
                    cur = entries.get(c).ok_or(FileSystemError::NotFound)?;
                }
                Node::File { .. } => return Err(FileSystemError::NotDirectory),
                Node::Symlink { .. } => return Err(FileSystemError::InvalidArgument),
            }
        }
        Ok(cur)
    }

    fn get_mut<'a>(
        dir: &'a mut Node,
        comps: &[String],
    ) -> Result<&'a mut Node, FileSystemError> {
        let mut cur = dir;
        for c in comps {
            match cur {
                Node::Dir { entries, .. } => {
                    cur = entries.get_mut(c).ok_or(FileSystemError::NotFound)?;
                }
                Node::File { .. } => return Err(FileSystemError::NotDirectory),
                Node::Symlink { .. } => return Err(FileSystemError::InvalidArgument),
            }
        }
        Ok(cur)
    }

    /// Import a ustar tarball (initrd) through the mounted VFS.
    /// Tolerant reader: skips pax headers, stops at two zero blocks or
    /// end of data. Parents are created as needed. Typeflag `2` entries
    /// become symlinks (linkname field).
    pub fn import_tar_vfs(data: &[u8]) -> Result<usize, FileSystemError> {
        let mut vfs = super::VFS.lock();
        let mut off = 0usize;
        let mut count = 0usize;
        while off + 512 <= data.len() {
            let hdr = &data[off..off + 512];
            if hdr.iter().all(|&b| b == 0) {
                break;
            }
            let name = tar_str(&hdr[0..100]);
            let prefix = tar_str(&hdr[345..500]);
            let size = tar_oct(&hdr[124..136]).ok_or(FileSystemError::InvalidFormat)?;
            let kind = hdr[156];
            off += 512;
            let blocks = size.saturating_add(511) / 512;
            if off + blocks * 512 > data.len() {
                return Err(FileSystemError::InvalidFormat);
            }
            let body = &data[off..off + size as usize];
            off += blocks * 512;

            if kind == b'x' || kind == b'g' || kind == b'L' || kind == b'K' {
                continue; // pax headers / GNU longnames (build.sh emits none)
            }
            let full = if prefix.is_empty() { name } else { prefix + "/" + &name };
            if full.is_empty() {
                continue;
            }
            let comps = Self::comps(&full);
            if comps.is_empty() {
                continue;
            }
            mkdir_p_vfs(&vfs, &comps[..comps.len() - 1])?;
            let mut path = String::new();
            for c in &comps {
                path.push('/');
                path.push_str(c);
            }
            if kind == b'5' {
                match vfs.mkdir(&path) {
                    Ok(()) | Err(FileSystemError::AlreadyExists) => {}
                    Err(e) => return Err(e),
                }
            } else if kind == b'2' {
                let target = tar_str(&hdr[157..257]);
                match vfs.symlink(&target, &path) {
                    Ok(()) | Err(FileSystemError::AlreadyExists) => {}
                    Err(e) => return Err(e),
                }
            } else {
                let fd = vfs.open(&path, O_WRONLY | O_CREAT | O_TRUNC)?;
                let mut done = 0usize;
                while done < body.len() {
                    let n = vfs.write(fd, &body[done..])?;
                    if n == 0 {
                        vfs.close(fd)?;
                        return Err(FileSystemError::NoSpace);
                    }
                    done += n;
                }
                vfs.close(fd)?;
            }
            count += 1;
        }
        Ok(count)
    }

    fn mkdir_parents(root: &mut Node, comps: &[String]) -> Result<(), FileSystemError> {
        let mut cur = root;
        for c in comps {
            match cur {
                Node::Dir { entries, .. } => {
                    cur = entries.entry(c.clone()).or_insert_with(|| Node::Dir {
                        inode: fresh_ino(),
                        entries: BTreeMap::new(),
                        mode: S_IFDIR | 0o755,
                        uid: 0,
                        gid: 0,
                    });
                    if !cur.is_dir() {
                        return Err(FileSystemError::NotDirectory);
                    }
                }
                _ => return Err(FileSystemError::NotDirectory),
            }
        }
        Ok(())
    }

    fn mkdir_locked(root: &mut Node, comps: &[String]) -> Result<(), FileSystemError> {
        if comps.is_empty() {
            return Ok(());
        }
        Self::mkdir_parents(root, &comps[..comps.len() - 1])?;
        let name = &comps[comps.len() - 1];
        let parent = Self::get_mut(root, &comps[..comps.len() - 1])?;
        match parent {
            Node::Dir { entries, .. } => {
                if let Some(n) = entries.get(name) {
                    if !n.is_dir() {
                        return Err(FileSystemError::AlreadyExists);
                    }
                    return Ok(());
                }
                entries.insert(
                    name.clone(),
                    Node::Dir {
                        inode: fresh_ino(),
                        entries: BTreeMap::new(),
                        mode: S_IFDIR | 0o755,
                        uid: 0,
                        gid: 0,
                    },
                );
                Ok(())
            }
            _ => Err(FileSystemError::NotDirectory),
        }
    }

    fn put_file_locked(root: &mut Node, comps: &[String], body: &[u8]) -> Result<(), FileSystemError> {
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        Self::mkdir_parents(root, &comps[..comps.len() - 1])?;
        let parent = Self::get_mut(root, &comps[..comps.len() - 1])?;
        let name = &comps[comps.len() - 1];
        match parent {
            Node::Dir { entries, .. } => {
                if let Some(Node::Dir { .. }) = entries.get(name) {
                    return Err(FileSystemError::IsDirectory);
                }
                entries.insert(
                    name.clone(),
                    Node::File {
                        inode: fresh_ino(),
                        data: body.to_vec(),
                        mode: S_IFREG | 0o644,
                        uid: 0,
                        gid: 0,
                    },
                );
                Ok(())
            }
            _ => Err(FileSystemError::NotDirectory),
        }
    }
}

fn mkdir_p_vfs(vfs: &super::Vfs, comps: &[String]) -> Result<(), FileSystemError> {
    let mut path = String::new();
    for c in comps {
        path.push('/');
        path.push_str(c);
        match vfs.mkdir(&path) {
            Ok(()) | Err(FileSystemError::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn tar_str(b: &[u8]) -> String {
    let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn tar_oct(b: &[u8]) -> Option<usize> {
    let s: String =
        b.iter().take_while(|&&x| x != 0 && x != b' ').map(|&x| x as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(s, 8).ok()
}

/// Open file handle: path-based, offset-tracked.
pub struct RamFile {
    tree: Arc<Mutex<DirNode>>,
    comps: Vec<String>,
    offset: usize,
    readable: bool,
    writable: bool,
    append: bool,
}

impl File for RamFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileSystemError> {
        if !self.readable {
            return Err(FileSystemError::PermissionDenied);
        }
        let guard = self.tree.lock();
        match RamFs::get(&guard.root, &self.comps)? {
            Node::File { data, .. } => {
                if self.offset >= data.len() {
                    return Ok(0);
                }
                let n = (data.len() - self.offset).min(buf.len());
                buf[..n].copy_from_slice(&data[self.offset..self.offset + n]);
                self.offset += n;
                Ok(n)
            }
            Node::Symlink { target, .. } => {
                // readlink-via-read: expose target bytes (Linux read() on a
                // symlink fd fails, but our VFS follows links before open,
                // so this only triggers on races — return target anyway).
                let b = target.as_bytes();
                if self.offset >= b.len() {
                    return Ok(0);
                }
                let n = (b.len() - self.offset).min(buf.len());
                buf[..n].copy_from_slice(&b[self.offset..self.offset + n]);
                self.offset += n;
                Ok(n)
            }
            Node::Dir { .. } => Err(FileSystemError::IsDirectory),
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FileSystemError> {
        if !self.writable {
            return Err(FileSystemError::PermissionDenied);
        }
        let mut guard = self.tree.lock();
        match RamFs::get_mut(&mut guard.root, &self.comps)? {
            Node::File { data, .. } => {
                if self.append {
                    self.offset = data.len();
                }
                if self.offset > data.len() {
                    data.resize(self.offset, 0);
                }
                if self.offset + buf.len() > data.len() {
                    data.resize(self.offset + buf.len(), 0);
                }
                data[self.offset..self.offset + buf.len()].copy_from_slice(buf);
                self.offset += buf.len();
                Ok(buf.len())
            }
            Node::Dir { .. } => Err(FileSystemError::IsDirectory),
            Node::Symlink { .. } => Err(FileSystemError::InvalidArgument),
        }
    }

    fn seek(&mut self, pos: i64, whence: u32) -> Result<u64, FileSystemError> {
        if self.append && whence != SEEK_END {
            // O_APPEND: writers stay at end; allow SEEK_END queries.
        }
        let guard = self.tree.lock();
        let len = match RamFs::get(&guard.root, &self.comps)? {
            Node::File { data, .. } => data.len() as i64,
            Node::Symlink { target, .. } => target.len() as i64,
            Node::Dir { .. } => return Err(FileSystemError::IsDirectory),
        };
        let base: i64 = match whence {
            SEEK_SET => 0,
            SEEK_CUR => self.offset as i64,
            SEEK_END => len,
            _ => return Err(FileSystemError::InvalidArgument),
        };
        let next = base.checked_add(pos).ok_or(FileSystemError::InvalidArgument)?;
        if next < 0 {
            return Err(FileSystemError::InvalidArgument);
        }
        self.offset = next as usize;
        Ok(self.offset as u64)
    }

    fn close(&mut self) -> Result<(), FileSystemError> {
        Ok(())
    }

    fn stat(&self) -> Result<FileStat, FileSystemError> {
        let guard = self.tree.lock();
        node_stat(RamFs::get(&guard.root, &self.comps)?)
    }
}

fn node_stat(node: &Node) -> Result<FileStat, FileSystemError> {
    let (size, mode, uid, gid) = match node {
        Node::File { data, mode, uid, gid, .. } => (data.len() as u64, *mode, *uid, *gid),
        Node::Dir { mode, uid, gid, .. } => (0, *mode, *uid, *gid),
        Node::Symlink { target, uid, gid, .. } => (target.len() as u64, S_IFLNK | 0o777, *uid, *gid),
    };
    Ok(FileStat {
        inode: node.inode(),
        size,
        mode,
        uid,
        gid,
        atime: 0,
        mtime: 0,
        ctime: 0,
        links: 1,
    })
}

fn ramfs_type(t: FileType) -> Result<(), FileSystemError> {
    match t {
        FileType::RegularFile | FileType::Directory => Ok(()),
        _ => Err(FileSystemError::NotImplemented),
    }
}

fn node_file_type(n: &Node) -> super::FileType {
    match n {
        Node::Dir { .. } => super::FileType::Directory,
        Node::File { .. } => super::FileType::RegularFile,
        Node::Symlink { .. } => super::FileType::Symlink,
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn open(&self, path: &str, flags: u32) -> Result<Box<dyn File>, FileSystemError> {
        let comps = Self::comps(path);
        let acc = flags & O_ACCMODE;
        let readable = acc == super::O_RDONLY || acc == O_RDWR;
        let writable = acc == O_WRONLY || acc == O_RDWR;
        let append = flags & O_APPEND != 0;

        let mut guard = self.inner.lock();
        let exists = RamFs::get(&guard.root, &comps).is_ok();
        if !exists {
            if flags & O_CREAT == 0 {
                return Err(FileSystemError::NotFound);
            }
            if comps.is_empty() {
                return Err(FileSystemError::InvalidArgument);
            }
            if flags & O_DIRECTORY != 0 {
                return Err(FileSystemError::NotFound);
            }
            Self::put_file_locked(&mut guard.root, &comps, &[])?;
        } else {
            if flags & O_EXCL != 0 && flags & O_CREAT != 0 {
                return Err(FileSystemError::AlreadyExists);
            }
            // O_DIRECTORY requires a dir.
            if flags & O_DIRECTORY != 0 {
                match RamFs::get(&guard.root, &comps)? {
                    Node::Dir { .. } => {}
                    _ => return Err(FileSystemError::NotDirectory),
                }
            }
            if flags & O_TRUNC != 0 {
                if !writable {
                    return Err(FileSystemError::PermissionDenied);
                }
                match RamFs::get_mut(&mut guard.root, &comps)? {
                    Node::File { data, .. } => data.clear(),
                    Node::Dir { .. } => return Err(FileSystemError::IsDirectory),
                    Node::Symlink { .. } => return Err(FileSystemError::InvalidArgument),
                }
            }
            // Opening a directory writeable is an error.
            if writable {
                match RamFs::get(&guard.root, &comps)? {
                    Node::Dir { .. } => return Err(FileSystemError::IsDirectory),
                    _ => {}
                }
            }
        }
        drop(guard);
        Ok(Box::new(RamFile {
            tree: self.inner.clone(),
            comps,
            offset: 0,
            readable,
            writable,
            append,
        }))
    }

    fn create(&self, path: &str, file_type: FileType) -> Result<(), FileSystemError> {
        ramfs_type(file_type)?;
        let comps = Self::comps(path);
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        let mut guard = self.inner.lock();
        if RamFs::get(&guard.root, &comps).is_ok() {
            return Err(FileSystemError::AlreadyExists);
        }
        match file_type {
            FileType::Directory => Self::mkdir_locked(&mut guard.root, &comps),
            _ => Self::put_file_locked(&mut guard.root, &comps, &[]),
        }
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let comps = Self::comps(path);
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument); // refuse to remove /
        }
        let mut guard = self.inner.lock();
        let parent = RamFs::get_mut(&mut guard.root, &comps[..comps.len() - 1])?;
        let name = &comps[comps.len() - 1];
        match parent {
            Node::Dir { entries, .. } => match entries.get(name) {
                None => Err(FileSystemError::NotFound),
                Some(Node::Dir { entries: kids, .. }) if !kids.is_empty() => {
                    Err(FileSystemError::NotEmpty)
                }
                Some(_) => {
                    entries.remove(name);
                    Ok(())
                }
            },
            _ => Err(FileSystemError::NotDirectory),
        }
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        self.create(path, FileType::Directory)
    }

    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, FileSystemError> {
        let comps = Self::comps(path);
        let guard = self.inner.lock();
        match RamFs::get(&guard.root, &comps)? {
            Node::Dir { entries, inode, .. } => {
                let mut out = Vec::new();
                // Linux getdents semantics: every dir shows "." and "..".
                let parent_ino = if comps.is_empty() {
                    *inode
                } else {
                    RamFs::get(&guard.root, &comps[..comps.len() - 1])
                        .map(|n| n.inode())
                        .unwrap_or(1)
                };
                out.push(DirEntry { inode: *inode, name: String::from("."), file_type: super::FileType::Directory });
                out.push(DirEntry { inode: parent_ino, name: String::from(".."), file_type: super::FileType::Directory });
                for (name, node) in entries.iter() {
                    out.push(DirEntry {
                        inode: node.inode(),
                        name: name.clone(),
                        file_type: node_file_type(node),
                    });
                }
                Ok(out)
            }
            Node::Symlink { .. } => Err(FileSystemError::InvalidArgument),
            Node::File { .. } => Err(FileSystemError::NotDirectory),
        }
    }

    fn stat(&self, path: &str) -> Result<FileStat, FileSystemError> {
        let comps = Self::comps(path);
        let guard = self.inner.lock();
        node_stat(RamFs::get(&guard.root, &comps)?)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FileSystemError> {
        let fc = Self::comps(from);
        let tc = Self::comps(to);
        if fc.is_empty() || tc.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        // No-op rename onto itself.
        if fc == tc {
            return Ok(());
        }
        // Refuse to move a dir into its own subtree.
        if tc.len() > fc.len() && tc[..fc.len()] == fc {
            return Err(FileSystemError::InvalidArgument);
        }
        let mut guard = self.inner.lock();
        // Take the source node out.
        let node = {
            let parent = Self::get_mut(&mut guard.root, &fc[..fc.len() - 1])?;
            match parent {
                Node::Dir { entries, .. } => {
                    entries.remove(&fc[fc.len() - 1]).ok_or(FileSystemError::NotFound)?
                }
                _ => return Err(FileSystemError::NotDirectory),
            }
        };
        // Drop destination if it exists (file/symlink/empty dir only).
        {
            let dparent = Self::get_mut(&mut guard.root, &tc[..tc.len() - 1])?;
            match dparent {
                Node::Dir { entries, .. } => {
                    if let Some(old) = entries.get(&tc[tc.len() - 1]) {
                        match old {
                            Node::Dir { entries: kids, .. } if kids.is_empty() => {}
                            Node::Dir { .. } => {
                                // restore source
                                let sp = Self::get_mut(&mut guard.root, &fc[..fc.len() - 1])?;
                                match sp {
                                    Node::Dir { entries, .. } => {
                                        entries.insert(fc[fc.len() - 1].clone(), node);
                                    }
                                    _ => {}
                                }
                                return Err(FileSystemError::NotEmpty);
                            }
                            _ => {}
                        }
                        entries.remove(&tc[tc.len() - 1]);
                    }
                    entries.insert(tc[tc.len() - 1].clone(), node);
                    Ok(())
                }
                _ => Err(FileSystemError::NotDirectory),
            }
        }
    }

    fn symlink(&self, target: &str, link: &str) -> Result<(), FileSystemError> {
        if target.is_empty() || target.len() > 4096 {
            return Err(FileSystemError::InvalidArgument);
        }
        let comps = Self::comps(link);
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        let mut guard = self.inner.lock();
        if RamFs::get(&guard.root, &comps).is_ok() {
            return Err(FileSystemError::AlreadyExists);
        }
        Self::mkdir_parents(&mut guard.root, &comps[..comps.len() - 1])?;
        let parent = Self::get_mut(&mut guard.root, &comps[..comps.len() - 1])?;
        match parent {
            Node::Dir { entries, .. } => {
                entries.insert(
                    comps[comps.len() - 1].clone(),
                    Node::Symlink { inode: fresh_ino(), target: target.to_string(), uid: 0, gid: 0 },
                );
                Ok(())
            }
            _ => Err(FileSystemError::NotDirectory),
        }
    }

    fn readlink(&self, path: &str) -> Result<String, FileSystemError> {
        let comps = Self::comps(path);
        let guard = self.inner.lock();
        match RamFs::get(&guard.root, &comps)? {
            Node::Symlink { target, .. } => Ok(target.clone()),
            _ => Err(FileSystemError::InvalidArgument), // not a symlink
        }
    }

    fn truncate(&self, path: &str, len: u64) -> Result<(), FileSystemError> {
        let comps = Self::comps(path);
        let mut guard = self.inner.lock();
        match RamFs::get_mut(&mut guard.root, &comps)? {
            Node::File { data, .. } => {
                data.resize(len as usize, 0);
                Ok(())
            }
            Node::Dir { .. } => Err(FileSystemError::IsDirectory),
            Node::Symlink { .. } => Err(FileSystemError::InvalidArgument),
        }
    }

    fn chmod(&self, path: &str, mode: u32) -> Result<(), FileSystemError> {
        let comps = Self::comps(path);
        let mut guard = self.inner.lock();
        match RamFs::get_mut(&mut guard.root, &comps)? {
            Node::File { mode: m, .. } => {
                *m = (S_IFREG) | (mode & 0o7777);
                Ok(())
            }
            Node::Dir { mode: m, .. } => {
                *m = (S_IFDIR) | (mode & 0o7777);
                Ok(())
            }
            Node::Symlink { .. } => Ok(()), // chmod on symlink: no-op (Linux lchmod-ish)
        }
    }

    fn mount(&mut self, _device: &str, _options: &str) -> Result<(), FileSystemError> {
        Ok(())
    }

    fn unmount(&mut self) -> Result<(), FileSystemError> {
        Ok(())
    }
}

// silence unused-import warnings if flag sets change
#[allow(dead_code)]
fn _use_flags() {
    let _ = SEEK_SET;
    let _ = SEEK_CUR;
    let _ = SEEK_END;
    let _ = S_IFMT;
}
