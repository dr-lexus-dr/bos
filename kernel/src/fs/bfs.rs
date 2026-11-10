//! bOS Filesystem (bFS) — kernel side.
//!
//! Same on-disk layout as the host `bfs` crate (`fs/bfs/src/lib.rs`):
//! superblock block 0, block bitmap, inode bitmap, inode table
//! (`INODE_SIZE` = 256 B, 16 per block), data blocks with packed
//! fixed-size dir entries and direct + single-indirect file blocks.
//! Keep constants and encode/decode in sync with the host crate.
//!
//! Until a real block device driver is wired up, the filesystem lives in
//! an in-RAM image (`Vec<u8>`) protected by `RwLock`. `format()` builds a
//! fresh image; `mount()` auto-formats an empty instance so
//! `mount ramdisk /system/mounts/data bFS` just works from bSH.

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::vec;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use spin::RwLock;
use log::info;

use crate::fs::{
    DirEntry, File, FileStat, FileSystem, FileSystemError, FileType, O_ACCMODE, O_APPEND, O_CREAT,
    O_DIRECTORY, O_EXCL, O_RDWR, O_TRUNC, O_WRONLY, SEEK_CUR, SEEK_END, SEEK_SET, S_IFDIR,
    S_IFLNK, S_IFMT, S_IFREG,
};

pub const BFS_MAGIC: u64 = 0x4246534F464E4942;
pub const BFS_VERSION: u32 = 1;
pub const BLOCK_SIZE: usize = 4096;
pub const BLOCK_SIZE_U64: u64 = 4096;
pub const INODE_SIZE: usize = 256;
pub const INODES_PER_BLOCK: u64 = 16;
pub const ROOT_INO: u64 = 1;
pub const NDIRECT: usize = 12;
pub const PTRS_PER_BLOCK: usize = 512;
pub const MAX_NAME: usize = 255;
pub const DIR_ENTRY_SIZE: usize = 8 + 2 + 1 + 1 + 255;
pub const ENTRIES_PER_BLOCK: usize = BLOCK_SIZE / DIR_ENTRY_SIZE;

pub const FT_UNKNOWN: u8 = 0;
pub const FT_REG_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;
pub const FT_CHRDEV: u8 = 3;
pub const FT_BLKDEV: u8 = 4;
pub const FT_FIFO: u8 = 5;
pub const FT_SOCK: u8 = 6;
pub const FT_SYMLINK: u8 = 7;

// superblock offsets (must match host crate)
const SB_MAGIC: usize = 0;
const SB_VERSION: usize = 8;
const SB_BSIZE: usize = 12;
const SB_TOTAL: usize = 16;
const SB_FREE_B: usize = 24;
const SB_ROOT: usize = 32;
const SB_ICOUNT: usize = 40;
const SB_FREE_I: usize = 48;
const SB_BB: usize = 56;
const SB_IB: usize = 64;
const SB_IT: usize = 72;
const SB_DATA: usize = 80;
const SB_LABEL: usize = 88;
const SB_CRC: usize = 136;

// inode offsets (must match host crate)
const IN_NUM: usize = 0;
const IN_MODE: usize = 8;
const IN_UID: usize = 12;
const IN_GID: usize = 16;
const IN_SIZE: usize = 20;
const IN_ATIME: usize = 28;
const IN_MTIME: usize = 36;
const IN_CTIME: usize = 44;
const IN_LINKS: usize = 52;
const IN_FLAGS: usize = 56;
const IN_DIRECT: usize = 60;
const IN_INDIRECT: usize = 156;
const IN_GEN: usize = 164;

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([
        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7],
    ])
}
fn wr16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn wr64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

fn sb_checksum(block: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for (i, &x) in block.iter().enumerate().take(BLOCK_SIZE) {
        let v = if (SB_CRC..SB_CRC + 4).contains(&i) { 0 } else { x };
        h ^= v as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[derive(Debug, Clone)]
struct Sb {
    total_blocks: u64,
    free_blocks: u64,
    inode_count: u64,
    free_inodes: u64,
    bb_start: u64,
    ib_start: u64,
    it_start: u64,
    data_start: u64,
}

#[derive(Debug, Clone)]
struct Inode {
    num: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    size: u64,
    links: u32,
    flags: u32,
    direct: [u64; NDIRECT],
    indirect: u64,
    generation: u32,
}

#[derive(Debug, Clone)]
struct Dent {
    inode: u64,
    name: String,
    ftype: u8,
}

struct Inner {
    img: Vec<u8>,
    sb: Sb,
    mounted: bool,
}

fn div_ceil(a: u64, b: u64) -> u64 {
    (a + b - 1) / b
}

fn build_image(size_bytes: u64, label: &str) -> Result<Vec<u8>, FileSystemError> {
    if size_bytes < 4 * 1024 * 1024 || size_bytes % BLOCK_SIZE_U64 != 0 {
        return Err(FileSystemError::InvalidArgument);
    }
    let total = size_bytes / BLOCK_SIZE_U64;
    let icount = (total / 4).max(16);
    let bb = div_ceil(total, BLOCK_SIZE_U64 * 8);
    let ib = div_ceil(icount, BLOCK_SIZE_U64 * 8);
    let it = div_ceil(icount * INODE_SIZE as u64, BLOCK_SIZE_U64);
    let data_start = 1 + bb + ib + it;
    if data_start + 1 >= total {
        return Err(FileSystemError::NoSpace);
    }
    let mut img = vec![0u8; size_bytes as usize];

    // superblock
    {
        let b = &mut img[..BLOCK_SIZE];
        wr64(b, SB_MAGIC, BFS_MAGIC);
        wr32(b, SB_VERSION, BFS_VERSION);
        wr32(b, SB_BSIZE, BLOCK_SIZE as u32);
        wr64(b, SB_TOTAL, total);
        wr64(b, SB_FREE_B, total - data_start - 1);
        wr64(b, SB_ROOT, ROOT_INO);
        wr64(b, SB_ICOUNT, icount);
        wr64(b, SB_FREE_I, icount - 1);
        wr64(b, SB_BB, 1);
        wr64(b, SB_IB, 1 + bb);
        wr64(b, SB_IT, 1 + bb + ib);
        wr64(b, SB_DATA, data_start);
        let lb = label.as_bytes();
        let n = lb.len().min(32);
        b[SB_LABEL..SB_LABEL + n].copy_from_slice(&lb[..n]);
        wr32(b, SB_CRC, 0);
        let c = sb_checksum(b);
        wr32(b, SB_CRC, c);
    }
    // block bitmap: reserve 0..=data_start
    for blk in 0..=data_start {
        let byte = (blk / 8) as usize;
        let abs = (1 + byte as u64 / BLOCK_SIZE_U64) as usize * BLOCK_SIZE + byte % BLOCK_SIZE;
        img[abs] |= 1 << (blk % 8);
    }
    // inode bitmap: root used
    img[(1 + bb) as usize * BLOCK_SIZE] |= 1;
    // root dir block
    {
        let off = data_start as usize * BLOCK_SIZE;
        let dents = [
            Dent { inode: ROOT_INO, name: String::from("."), ftype: FT_DIR },
            Dent { inode: ROOT_INO, name: String::from(".."), ftype: FT_DIR },
        ];
        encode_dents(&dents, &mut img[off..off + BLOCK_SIZE]);
    }
    // root inode
    {
        let off = (1 + bb + ib) as usize * BLOCK_SIZE;
        let mut slot = [0u8; INODE_SIZE];
        wr64(&mut slot, IN_NUM, ROOT_INO);
        wr32(&mut slot, IN_MODE, S_IFDIR | 0o755);
        wr64(&mut slot, IN_SIZE, BLOCK_SIZE_U64);
        wr32(&mut slot, IN_LINKS, 2);
        wr64(&mut slot, IN_DIRECT, data_start);
        wr32(&mut slot, IN_GEN, 1);
        img[off..off + INODE_SIZE].copy_from_slice(&slot);
    }
    Ok(img)
}

fn decode_dents(block: &[u8]) -> Vec<Dent> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + DIR_ENTRY_SIZE <= block.len() {
        let ino = rd64(block, off);
        if ino == 0 {
            break;
        }
        let nlen = rd16(block, off + 8) as usize;
        let ft = block[off + 10];
        if nlen == 0 || nlen > MAX_NAME {
            break;
        }
        let name = String::from_utf8_lossy(&block[off + 12..off + 12 + nlen]).into_owned();
        out.push(Dent { inode: ino, name, ftype: ft });
        off += DIR_ENTRY_SIZE;
    }
    out
}

fn encode_dents(ents: &[Dent], block: &mut [u8]) {
    block.fill(0);
    let mut off = 0;
    for e in ents.iter().take(ENTRIES_PER_BLOCK) {
        let nb = e.name.as_bytes();
        let n = nb.len().min(MAX_NAME);
        wr64(block, off, e.inode);
        wr16(block, off + 8, n as u16);
        block[off + 10] = e.ftype;
        block[off + 11] = 0;
        block[off + 12..off + 12 + n].copy_from_slice(&nb[..n]);
        off += DIR_ENTRY_SIZE;
    }
}

impl Inner {
    fn parse_sb(img: &[u8]) -> Result<Sb, FileSystemError> {
        if img.len() < BLOCK_SIZE || img.len() % BLOCK_SIZE != 0 {
            return Err(FileSystemError::InvalidFormat);
        }
        let b = &img[..BLOCK_SIZE];
        if rd64(b, SB_MAGIC) != BFS_MAGIC {
            return Err(FileSystemError::InvalidFormat);
        }
        if rd32(b, SB_VERSION) != BFS_VERSION {
            return Err(FileSystemError::UnsupportedVersion);
        }
        if rd32(b, SB_BSIZE) as usize != BLOCK_SIZE {
            return Err(FileSystemError::InvalidFormat);
        }
        if sb_checksum(b) != rd32(b, SB_CRC) {
            return Err(FileSystemError::InvalidFormat);
        }
        let total = rd64(b, SB_TOTAL);
        if total * BLOCK_SIZE_U64 != img.len() as u64 {
            return Err(FileSystemError::InvalidFormat);
        }
        Ok(Sb {
            total_blocks: total,
            free_blocks: rd64(b, SB_FREE_B),
            inode_count: rd64(b, SB_ICOUNT),
            free_inodes: rd64(b, SB_FREE_I),
            bb_start: rd64(b, SB_BB),
            ib_start: rd64(b, SB_IB),
            it_start: rd64(b, SB_IT),
            data_start: rd64(b, SB_DATA),
        })
    }

    fn sync_sb(&mut self) {
        // total/readonly fields never change; counts + label area refreshed.
        let b = &mut self.img[..BLOCK_SIZE];
        wr64(b, SB_FREE_B, self.sb.free_blocks);
        wr64(b, SB_FREE_I, self.sb.free_inodes);
        wr32(b, SB_CRC, 0);
        let c = sb_checksum(b);
        wr32(b, SB_CRC, c);
    }

    fn blk(&self, n: u64) -> Result<&[u8], FileSystemError> {
        if n >= self.sb.total_blocks {
            return Err(FileSystemError::IoError);
        }
        let o = n as usize * BLOCK_SIZE;
        Ok(&self.img[o..o + BLOCK_SIZE])
    }

    fn blk_mut(&mut self, n: u64) -> Result<&mut [u8], FileSystemError> {
        if n >= self.sb.total_blocks {
            return Err(FileSystemError::IoError);
        }
        let o = n as usize * BLOCK_SIZE;
        Ok(&mut self.img[o..o + BLOCK_SIZE])
    }

    fn bit_get(&self, start: u64, bit: u64) -> Result<bool, FileSystemError> {
        let byte = bit / 8;
        let blk = start + byte / BLOCK_SIZE_U64;
        let off = (byte % BLOCK_SIZE_U64) as usize;
        Ok(self.blk(blk)?[off] & (1 << (bit % 8)) != 0)
    }

    fn bit_set(&mut self, start: u64, bit: u64, v: bool) -> Result<(), FileSystemError> {
        let byte = bit / 8;
        let blk = start + byte / BLOCK_SIZE_U64;
        let off = (byte % BLOCK_SIZE_U64) as usize;
        let b = self.blk_mut(blk)?;
        if v {
            b[off] |= 1 << (bit % 8);
        } else {
            b[off] &= !(1 << (bit % 8));
        }
        Ok(())
    }

    fn alloc_block(&mut self) -> Result<u64, FileSystemError> {
        if self.sb.free_blocks == 0 {
            return Err(FileSystemError::NoSpace);
        }
        for b in self.sb.data_start..self.sb.total_blocks {
            if !self.bit_get(self.sb.bb_start, b)? {
                self.bit_set(self.sb.bb_start, b, true)?;
                self.blk_mut(b)?.fill(0);
                self.sb.free_blocks -= 1;
                return Ok(b);
            }
        }
        Err(FileSystemError::NoSpace)
    }

    fn free_block(&mut self, n: u64) -> Result<(), FileSystemError> {
        if n < self.sb.data_start || n >= self.sb.total_blocks {
            return Err(FileSystemError::InvalidArgument);
        }
        if self.bit_get(self.sb.bb_start, n)? {
            self.bit_set(self.sb.bb_start, n, false)?;
            self.sb.free_blocks += 1;
        }
        Ok(())
    }

    fn alloc_inode(&mut self) -> Result<u64, FileSystemError> {
        if self.sb.free_inodes == 0 {
            return Err(FileSystemError::NoSpace);
        }
        for i in 1..=self.sb.inode_count {
            if !self.bit_get(self.sb.ib_start, i - 1)? {
                self.bit_set(self.sb.ib_start, i - 1, true)?;
                self.sb.free_inodes -= 1;
                return Ok(i);
            }
        }
        Err(FileSystemError::NoSpace)
    }

    fn free_inode(&mut self, ino: u64) -> Result<(), FileSystemError> {
        if ino == 0 || ino > self.sb.inode_count || ino == ROOT_INO {
            return Err(FileSystemError::InvalidArgument);
        }
        self.bit_set(self.sb.ib_start, ino - 1, false)?;
        self.sb.free_inodes += 1;
        let (blk, off) = self.ino_loc(ino)?;
        let b = self.blk_mut(blk)?;
        b[off..off + INODE_SIZE].fill(0);
        Ok(())
    }

    fn ino_loc(&self, ino: u64) -> Result<(u64, usize), FileSystemError> {
        if ino == 0 || ino > self.sb.inode_count {
            return Err(FileSystemError::InvalidArgument);
        }
        let idx = ino - 1;
        Ok((self.sb.it_start + idx / INODES_PER_BLOCK, (idx % INODES_PER_BLOCK) as usize * INODE_SIZE))
    }

    fn read_inode(&self, ino: u64) -> Result<Inode, FileSystemError> {
        let (blk, off) = self.ino_loc(ino)?;
        let b = self.blk(blk)?;
        let s = &b[off..off + INODE_SIZE];
        let mode = rd32(s, IN_MODE);
        if mode == 0 {
            return Err(FileSystemError::NotFound);
        }
        let mut direct = [0u64; NDIRECT];
        for i in 0..NDIRECT {
            direct[i] = rd64(s, IN_DIRECT + i * 8);
        }
        Ok(Inode {
            num: rd64(s, IN_NUM),
            mode,
            uid: rd32(s, IN_UID),
            gid: rd32(s, IN_GID),
            size: rd64(s, IN_SIZE),
            links: rd32(s, IN_LINKS),
            flags: rd32(s, IN_FLAGS),
            direct,
            indirect: rd64(s, IN_INDIRECT),
            generation: rd32(s, IN_GEN),
        })
    }

    fn write_inode(&mut self, inod: &Inode) -> Result<(), FileSystemError> {
        let (blk, off) = self.ino_loc(inod.num)?;
        let mut slot = [0u8; INODE_SIZE];
        wr64(&mut slot, IN_NUM, inod.num);
        wr32(&mut slot, IN_MODE, inod.mode);
        wr32(&mut slot, IN_UID, inod.uid);
        wr32(&mut slot, IN_GID, inod.gid);
        wr64(&mut slot, IN_SIZE, inod.size);
        wr32(&mut slot, IN_LINKS, inod.links);
        wr32(&mut slot, IN_FLAGS, inod.flags);
        for i in 0..NDIRECT {
            wr64(&mut slot, IN_DIRECT + i * 8, inod.direct[i]);
        }
        wr64(&mut slot, IN_INDIRECT, inod.indirect);
        wr32(&mut slot, IN_GEN, inod.generation);
        self.blk_mut(blk)?[off..off + INODE_SIZE].copy_from_slice(&slot);
        Ok(())
    }

    fn file_block(&self, inod: &Inode, idx: u64) -> Result<u64, FileSystemError> {
        if idx < NDIRECT as u64 {
            return Ok(inod.direct[idx as usize]);
        }
        let rel = idx - NDIRECT as u64;
        if rel >= PTRS_PER_BLOCK as u64 || inod.indirect == 0 {
            return Ok(0);
        }
        Ok(rd64(self.blk(inod.indirect)?, rel as usize * 8))
    }

    fn file_block_alloc(&mut self, inod: &mut Inode, idx: u64) -> Result<u64, FileSystemError> {
        if idx < NDIRECT as u64 {
            if inod.direct[idx as usize] == 0 {
                let nb = self.alloc_block()?;
                inod.direct[idx as usize] = nb;
            }
            return Ok(inod.direct[idx as usize]);
        }
        let rel = idx - NDIRECT as u64;
        if rel >= PTRS_PER_BLOCK as u64 {
            return Err(FileSystemError::NoSpace);
        }
        if inod.indirect == 0 {
            inod.indirect = self.alloc_block()?;
        }
        let ib = inod.indirect;
        let cur = rd64(self.blk(ib)?, rel as usize * 8);
        if cur != 0 {
            return Ok(cur);
        }
        let nb = self.alloc_block()?;
        let mut tmp = [0u8; BLOCK_SIZE];
        tmp.copy_from_slice(self.blk(ib)?);
        wr64(&mut tmp, rel as usize * 8, nb);
        self.blk_mut(ib)?.copy_from_slice(&tmp);
        Ok(nb)
    }

    fn free_file_blocks(&mut self, inod: &mut Inode) -> Result<(), FileSystemError> {
        for i in 0..NDIRECT {
            if inod.direct[i] != 0 {
                self.free_block(inod.direct[i])?;
                inod.direct[i] = 0;
            }
        }
        if inod.indirect != 0 {
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.blk(inod.indirect)?);
            for i in 0..PTRS_PER_BLOCK {
                let p = rd64(&tmp, i * 8);
                if p != 0 {
                    self.free_block(p)?;
                }
            }
            let ib = inod.indirect;
            inod.indirect = 0;
            self.free_block(ib)?;
        }
        inod.size = 0;
        Ok(())
    }

    fn read_data(&self, inod: &Inode) -> Result<Vec<u8>, FileSystemError> {
        let mut out = Vec::new();
        if inod.size > 0 {
            out.reserve(inod.size.min(65536) as usize);
        }
        let mut left = inod.size;
        let mut idx = 0u64;
        while left > 0 {
            let b = self.file_block(inod, idx)?;
            idx += 1;
            if b == 0 {
                let take = left.min(BLOCK_SIZE_U64) as usize;
                out.extend(core::iter::repeat(0).take(take));
                left -= take as u64;
                continue;
            }
            let blk = self.blk(b)?;
            let take = left.min(BLOCK_SIZE_U64) as usize;
            out.extend_from_slice(&blk[..take]);
            left -= take as u64;
        }
        Ok(out)
    }

    fn write_data(&mut self, ino: u64, data: &[u8]) -> Result<(), FileSystemError> {
        let mut inod = self.read_inode(ino)?;
        self.free_file_blocks(&mut inod)?;
        let mut off = 0usize;
        let mut idx = 0u64;
        while off < data.len() {
            let b = self.file_block_alloc(&mut inod, idx)?;
            idx += 1;
            let take = (data.len() - off).min(BLOCK_SIZE);
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp[..take].copy_from_slice(&data[off..off + take]);
            self.blk_mut(b)?.copy_from_slice(&tmp);
            off += take;
        }
        inod.size = data.len() as u64;
        self.write_inode(&inod)?;
        Ok(())
    }

    fn comps(path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for p in path.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                n => out.push(n.to_string()),
            }
        }
        out
    }

    fn lookup(&self, path: &str) -> Result<u64, FileSystemError> {
        let mut cur = ROOT_INO;
        for c in Self::comps(path) {
            let inod = self.read_inode(cur)?;
            if inod.mode & S_IFMT != S_IFDIR {
                return Err(FileSystemError::NotDirectory);
            }
            cur = self.find_in(&inod, &c)?.ok_or(FileSystemError::NotFound)?;
        }
        Ok(cur)
    }

    fn find_in(&self, dir: &Inode, name: &str) -> Result<Option<u64>, FileSystemError> {
        for e in self.all_ents(dir)? {
            if e.name == name {
                return Ok(Some(e.inode));
            }
        }
        Ok(None)
    }

    fn all_ents(&self, dir: &Inode) -> Result<Vec<Dent>, FileSystemError> {
        let mut out = Vec::new();
        let n = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..n {
            let b = self.file_block(dir, i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.blk(b)?);
            out.extend(decode_dents(&tmp));
        }
        Ok(out)
    }

    fn dir_add(&mut self, dir_ino: u64, name: &str, child: u64, ft: u8) -> Result<(), FileSystemError> {
        if name.is_empty() || name.len() > MAX_NAME || name.contains('/') {
            return Err(FileSystemError::InvalidArgument);
        }
        let mut dir = self.read_inode(dir_ino)?;
        if dir.mode & S_IFMT != S_IFDIR {
            return Err(FileSystemError::NotDirectory);
        }
        if self.find_in(&dir, name)?.is_some() {
            return Err(FileSystemError::AlreadyExists);
        }
        let n = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..n {
            let b = self.file_block(&dir.clone(), i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.blk(b)?);
            let mut ents = decode_dents(&tmp);
            if ents.len() < ENTRIES_PER_BLOCK {
                ents.push(Dent { inode: child, name: name.to_string(), ftype: ft });
                encode_dents(&ents, &mut tmp);
                self.blk_mut(b)?.copy_from_slice(&tmp);
                self.write_inode(&dir)?;
                return Ok(());
            }
        }
        if n >= NDIRECT as u64 + PTRS_PER_BLOCK as u64 {
            return Err(FileSystemError::NoSpace);
        }
        let nb = self.file_block_alloc(&mut dir, n)?;
        let mut tmp = [0u8; BLOCK_SIZE];
        encode_dents(&[Dent { inode: child, name: name.to_string(), ftype: ft }], &mut tmp);
        self.blk_mut(nb)?.copy_from_slice(&tmp);
        dir.size = (n + 1) * BLOCK_SIZE_U64;
        self.write_inode(&dir)?;
        Ok(())
    }

    fn dir_remove(&mut self, dir_ino: u64, name: &str) -> Result<u64, FileSystemError> {
        let dir = self.read_inode(dir_ino)?;
        if dir.mode & S_IFMT != S_IFDIR {
            return Err(FileSystemError::NotDirectory);
        }
        let n = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..n {
            let b = self.file_block(&dir, i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.blk(b)?);
            let mut ents = decode_dents(&tmp);
            if let Some(p) = ents.iter().position(|e| e.name == name) {
                let child = ents[p].inode;
                ents.remove(p);
                encode_dents(&ents, &mut tmp);
                self.blk_mut(b)?.copy_from_slice(&tmp);
                return Ok(child);
            }
        }
        Err(FileSystemError::NotFound)
    }

    fn trim_trailing_empty_dir_blocks(&mut self, dir_ino: u64) -> Result<(), FileSystemError> {
        // Free trailing dir blocks that hold no entries (keeps `du` sane).
        let mut dir = self.read_inode(dir_ino)?;
        loop {
            let n = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
            if n <= 1 {
                break;
            }
            let last = n - 1;
            let b = self.file_block(&dir, last)?;
            if b == 0 {
                break;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.blk(b)?);
            if !decode_dents(&tmp).is_empty() {
                break;
            }
            // unlink block pointer then free
            if last < NDIRECT as u64 {
                dir.direct[last as usize] = 0;
            } else {
                let rel = (last - NDIRECT as u64) as usize * 8;
                let ib = dir.indirect;
                let mut it = [0u8; BLOCK_SIZE];
                it.copy_from_slice(self.blk(ib)?);
                wr64(&mut it, rel, 0);
                self.blk_mut(ib)?.copy_from_slice(&it);
            }
            self.free_block(b)?;
            dir.size = (n - 1) * BLOCK_SIZE_U64;
            self.write_inode(&dir)?;
        }
        Ok(())
    }
}

/// bFS Filesystem: block-image in RAM (see module docs).
pub struct BfsFileSystem {
    device: String,
    inner: Arc<RwLock<Inner>>,
}

impl BfsFileSystem {
    pub fn new(device: &str) -> Self {
        // Empty image: first `mount()` auto-formats (16 MiB default).
        // `format()` / `load_image()` can replace it beforehand.
        let sb = Sb {
            total_blocks: 0,
            free_blocks: 0,
            inode_count: 0,
            free_inodes: 0,
            bb_start: 0,
            ib_start: 0,
            it_start: 0,
            data_start: 0,
        };
        Self {
            device: device.to_string(),
            inner: Arc::new(RwLock::new(Inner { img: Vec::new(), sb, mounted: false })),
        }
    }

    /// Build a freshly formatted filesystem (default 16 MiB).
    pub fn format(device: &str, size_bytes: u64, label: &str) -> Result<Self, FileSystemError> {
        let size = if size_bytes == 0 { 16 * 1024 * 1024 } else { size_bytes };
        let img = build_image(size, label)?;
        let sb = Inner::parse_sb(&img)?;
        Ok(Self {
            device: device.to_string(),
            inner: Arc::new(RwLock::new(Inner { img, sb, mounted: false })),
        })
    }

    /// Load a host-created image (mkfs output) into memory.
    pub fn load_image(device: &str, img: Vec<u8>) -> Result<Self, FileSystemError> {
        let sb = Inner::parse_sb(&img)?;
        Ok(Self {
            device: device.to_string(),
            inner: Arc::new(RwLock::new(Inner { img, sb, mounted: false })),
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// (total_bytes, free_bytes) for `df`.
    pub fn usage(&self) -> (u64, u64) {
        let g = self.inner.read();
        (g.sb.total_blocks * BLOCK_SIZE_U64, g.sb.free_blocks * BLOCK_SIZE_U64)
    }
}

fn bfs_stat(ino: u64, inod: &Inode) -> FileStat {
    FileStat {
        inode: ino,
        size: inod.size,
        mode: inod.mode,
        uid: inod.uid,
        gid: inod.gid,
        atime: 0,
        mtime: 0,
        ctime: 0,
        links: inod.links,
    }
}

impl FileSystem for BfsFileSystem {
    fn name(&self) -> &str {
        "bFS"
    }

    fn open(&self, path: &str, flags: u32) -> Result<Box<dyn File>, FileSystemError> {
        let acc = flags & O_ACCMODE;
        let readable = acc == crate::fs::O_RDONLY || acc == O_RDWR;
        let writable = acc == O_WRONLY || acc == O_RDWR;
        let mut g = self.inner.write();
        if !g.mounted {
            return Err(FileSystemError::IoError);
        }
        let comps = Inner::comps(path);
        let exists = g.lookup(path).is_ok();
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
            // create parent chain lookup
            let mut cur = ROOT_INO;
            for c in &comps[..comps.len() - 1] {
                let inod = g.read_inode(cur)?;
                cur = g.find_in(&inod, c)?.ok_or(FileSystemError::NotFound)?;
                if g.read_inode(cur)?.mode & S_IFMT != S_IFDIR {
                    return Err(FileSystemError::NotDirectory);
                }
            }
            let parent = cur;
            let ino = g.alloc_inode()?;
            g.write_inode(&Inode {
                num: ino,
                mode: S_IFREG | 0o644,
                uid: 0,
                gid: 0,
                size: 0,
                links: 1,
                flags: 0,
                direct: [0; NDIRECT],
                indirect: 0,
                generation: 1,
            })?;
            g.dir_add(parent, &comps[comps.len() - 1], ino, FT_REG_FILE)?;
            g.sync_sb();
        } else {
            if flags & O_EXCL != 0 && flags & O_CREAT != 0 {
                return Err(FileSystemError::AlreadyExists);
            }
            let ino = g.lookup(path)?;
            let inod = g.read_inode(ino)?;
            if flags & O_DIRECTORY != 0 && inod.mode & S_IFMT != S_IFDIR {
                return Err(FileSystemError::NotDirectory);
            }
            if inod.mode & S_IFMT == S_IFDIR && writable {
                return Err(FileSystemError::IsDirectory);
            }
            if inod.mode & S_IFMT != S_IFREG && inod.mode & S_IFMT != S_IFLNK {
                if writable || flags & O_TRUNC != 0 {
                    return Err(FileSystemError::IsDirectory);
                }
            }
            if flags & O_TRUNC != 0 {
                if !writable {
                    return Err(FileSystemError::PermissionDenied);
                }
                if inod.mode & S_IFMT != S_IFREG {
                    return Err(FileSystemError::IsDirectory);
                }
                let mut m = inod;
                g.free_file_blocks(&mut m)?;
                g.write_inode(&m)?;
                g.sync_sb();
            }
        }
        let ino = g.lookup(path)?;
        drop(g);
        Ok(Box::new(BfsFile {
            fs: self.inner.clone(),
            ino,
            offset: 0,
            readable,
            writable,
            append: flags & O_APPEND != 0,
        }))
    }

    fn create(&self, path: &str, file_type: FileType) -> Result<(), FileSystemError> {
        let mut g = self.inner.write();
        if !g.mounted {
            return Err(FileSystemError::IoError);
        }
        let comps = Inner::comps(path);
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        if g.lookup(path).is_ok() {
            return Err(FileSystemError::AlreadyExists);
        }
        let mut cur = ROOT_INO;
        for c in &comps[..comps.len() - 1] {
            let inod = g.read_inode(cur)?;
            if inod.mode & S_IFMT != S_IFDIR {
                return Err(FileSystemError::NotDirectory);
            }
            cur = g.find_in(&inod, c)?.ok_or(FileSystemError::NotFound)?;
        }
        let parent = cur;
        let p = g.read_inode(parent)?;
        if p.mode & S_IFMT != S_IFDIR {
            return Err(FileSystemError::NotDirectory);
        }
        match file_type {
            FileType::Directory => {
                let ino = g.alloc_inode()?;
                let blk = g.alloc_block()?;
                let mut tmp = [0u8; BLOCK_SIZE];
                encode_dents(
                    &[
                        Dent { inode: ino, name: String::from("."), ftype: FT_DIR },
                        Dent { inode: parent, name: String::from(".."), ftype: FT_DIR },
                    ],
                    &mut tmp,
                );
                g.blk_mut(blk)?.copy_from_slice(&tmp);
                g.write_inode(&Inode {
                    num: ino,
                    mode: S_IFDIR | 0o755,
                    uid: 0,
                    gid: 0,
                    size: BLOCK_SIZE_U64,
                    links: 2,
                    flags: 0,
                    direct: {
                        let mut d = [0u64; NDIRECT];
                        d[0] = blk;
                        d
                    },
                    indirect: 0,
                    generation: 1,
                })?;
                g.dir_add(parent, &comps[comps.len() - 1], ino, FT_DIR)?;
                let mut pp = g.read_inode(parent)?;
                pp.links += 1;
                g.write_inode(&pp)?;
                g.sync_sb();
                info!("bFS: mkdir {} (ino {})", path, ino);
                Ok(())
            }
            FileType::RegularFile => {
                let ino = g.alloc_inode()?;
                g.write_inode(&Inode {
                    num: ino,
                    mode: S_IFREG | 0o644,
                    uid: 0,
                    gid: 0,
                    size: 0,
                    links: 1,
                    flags: 0,
                    direct: [0; NDIRECT],
                    indirect: 0,
                    generation: 1,
                })?;
                g.dir_add(parent, &comps[comps.len() - 1], ino, FT_REG_FILE)?;
                g.sync_sb();
                Ok(())
            }
            _ => Err(FileSystemError::NotImplemented),
        }
    }

    fn remove(&self, path: &str) -> Result<(), FileSystemError> {
        let mut g = self.inner.write();
        if !g.mounted {
            return Err(FileSystemError::IoError);
        }
        let comps = Inner::comps(path);
        if comps.is_empty() {
            return Err(FileSystemError::InvalidArgument);
        }
        let mut cur = ROOT_INO;
        for c in &comps[..comps.len() - 1] {
            let inod = g.read_inode(cur)?;
            cur = g.find_in(&inod, c)?.ok_or(FileSystemError::NotFound)?;
        }
        let parent = cur;
        let child = g.dir_remove(parent, &comps[comps.len() - 1])?;
        let mut cin = g.read_inode(child)?;
        if cin.mode & S_IFMT == S_IFDIR {
            let ents: Vec<Dent> =
                g.all_ents(&cin)?.into_iter().filter(|e| e.name != "." && e.name != "..").collect();
            if !ents.is_empty() {
                g.dir_add(parent, &comps[comps.len() - 1], child, FT_DIR)?;
                return Err(FileSystemError::NotEmpty);
            }
            let mut pp = g.read_inode(parent)?;
            pp.links = pp.links.saturating_sub(1);
            g.write_inode(&pp)?;
        }
        cin.links = cin.links.saturating_sub(1);
        if cin.links == 0 {
            g.free_file_blocks(&mut cin)?;
            g.write_inode(&cin)?;
            g.free_inode(child)?;
        } else {
            g.write_inode(&cin)?;
        }
        g.trim_trailing_empty_dir_blocks(parent)?;
        g.sync_sb();
        Ok(())
    }

    fn mkdir(&self, path: &str) -> Result<(), FileSystemError> {
        self.create(path, FileType::Directory)
    }

    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, FileSystemError> {
        let g = self.inner.read();
        if !g.mounted {
            return Err(FileSystemError::IoError);
        }
        let ino = g.lookup(path)?;
        let inod = g.read_inode(ino)?;
        if inod.mode & S_IFMT != S_IFDIR {
            return Err(FileSystemError::NotDirectory);
        }
        Ok(g.all_ents(&inod)?
            .into_iter()
            .map(|e| DirEntry {
                inode: e.inode,
                name: e.name,
                file_type: match e.ftype {
                    FT_DIR => FileType::Directory,
                    FT_REG_FILE => FileType::RegularFile,
                    FT_SYMLINK => FileType::Symlink,
                    FT_CHRDEV => FileType::CharDevice,
                    FT_BLKDEV => FileType::BlockDevice,
                    FT_FIFO => FileType::Fifo,
                    FT_SOCK => FileType::Socket,
                    _ => FileType::Unknown,
                },
            })
            .collect())
    }

    fn stat(&self, path: &str) -> Result<FileStat, FileSystemError> {
        let g = self.inner.read();
        if !g.mounted {
            return Err(FileSystemError::IoError);
        }
        let ino = g.lookup(path)?;
        Ok(bfs_stat(ino, &g.read_inode(ino)?))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FileSystemError> {
        // File-only fast path: copy bytes + relink (dirs: refuse for now).
        let (data, mode) = {
            let g = self.inner.read();
            let ino = g.lookup(from)?;
            let inod = g.read_inode(ino)?;
            if inod.mode & S_IFMT == S_IFDIR {
                return Err(FileSystemError::NotImplemented);
            }
            if inod.mode & S_IFMT != S_IFREG && inod.mode & S_IFMT != S_IFLNK {
                return Err(FileSystemError::NotImplemented);
            }
            (g.read_data(&inod)?, inod.mode)
        };
        // create destination (replace file/symlink, refuse non-empty dir)
        match self.stat(to) {
            Ok(st) => {
                if st.is_dir() {
                    return Err(FileSystemError::IsDirectory);
                }
                self.remove(to)?;
            }
            Err(FileSystemError::NotFound) => {}
            Err(e) => return Err(e),
        }
        // ensure parents exist is the caller's job (VFS shells use mkdir -p);
        // try direct create, fall back to AlreadyExists propagation.
        if self.lookup_exists(to)? {
            return Err(FileSystemError::AlreadyExists);
        }
        self.create(to, FileType::RegularFile)?;
        {
            let mut g = self.inner.write();
            let ino = g.lookup(to)?;
            let mut inod = g.read_inode(ino)?;
            inod.mode = mode;
            g.write_inode(&inod)?;
            g.write_data(ino, &data)?;
            g.sync_sb();
        }
        self.remove(from)?;
        Ok(())
    }

    fn symlink(&self, target: &str, link: &str) -> Result<(), FileSystemError> {
        if target.is_empty() || target.len() > 4096 {
            return Err(FileSystemError::InvalidArgument);
        }
        self.create(link, FileType::RegularFile)?;
        let mut g = self.inner.write();
        let ino = g.lookup(link)?;
        let mut inod = g.read_inode(ino)?;
        inod.mode = S_IFLNK | 0o777;
        g.write_inode(&inod)?;
        g.write_data(ino, target.as_bytes())?;
        // fix dir entry type
        let comps = Inner::comps(link);
        let mut cur = ROOT_INO;
        for c in &comps[..comps.len() - 1] {
            let n = g.read_inode(cur)?;
            cur = g.find_in(&n, c)?.ok_or(FileSystemError::NotFound)?;
        }
        let parent = cur;
        let n = {
            let p = g.read_inode(parent)?;
            ((p.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1)
        };
        for i in 0..n {
            let b = g.file_block(&g.read_inode(parent)?, i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(g.blk(b)?);
            let mut ents = decode_dents(&tmp);
            let mut dirty = false;
            for e in ents.iter_mut() {
                if e.name == comps[comps.len() - 1] {
                    e.ftype = FT_SYMLINK;
                    dirty = true;
                }
            }
            if dirty {
                encode_dents(&ents, &mut tmp);
                g.blk_mut(b)?.copy_from_slice(&tmp);
                break;
            }
        }
        g.sync_sb();
        Ok(())
    }

    fn readlink(&self, path: &str) -> Result<String, FileSystemError> {
        let g = self.inner.read();
        let ino = g.lookup(path)?;
        let inod = g.read_inode(ino)?;
        if inod.mode & S_IFMT != S_IFLNK {
            return Err(FileSystemError::InvalidArgument);
        }
        let data = g.read_data(&inod)?;
        String::from_utf8(data).map_err(|_| FileSystemError::InvalidFormat)
    }

    fn truncate(&self, path: &str, len: u64) -> Result<(), FileSystemError> {
        let mut g = self.inner.write();
        let ino = g.lookup(path)?;
        let mut inod = g.read_inode(ino)?;
        if inod.mode & S_IFMT == S_IFDIR {
            return Err(FileSystemError::IsDirectory);
        }
        if inod.mode & S_IFMT != S_IFREG {
            return Err(FileSystemError::InvalidArgument);
        }
        if len < inod.size {
            // shrink: rewrite prefix
            let data = g.read_data(&inod)?;
            g.free_file_blocks(&mut inod)?;
            let mut off = 0usize;
            let mut idx = 0u64;
            let want = len as usize;
            while off < want {
                let b = g.file_block_alloc(&mut inod, idx)?;
                idx += 1;
                let take = (want - off).min(BLOCK_SIZE);
                let mut tmp = [0u8; BLOCK_SIZE];
                tmp[..take].copy_from_slice(&data[off..off + take]);
                g.blk_mut(b)?.copy_from_slice(&tmp);
                off += take;
            }
            inod.size = len;
            g.write_inode(&inod)?;
        } else if len > inod.size {
            // grow: zero-fill tail
            let mut data = g.read_data(&inod)?;
            data.resize(len as usize, 0);
            g.free_file_blocks(&mut inod)?;
            let mut off = 0usize;
            let mut idx = 0u64;
            while off < data.len() {
                let b = g.file_block_alloc(&mut inod, idx)?;
                idx += 1;
                let take = (data.len() - off).min(BLOCK_SIZE);
                let mut tmp = [0u8; BLOCK_SIZE];
                tmp[..take].copy_from_slice(&data[off..off + take]);
                g.blk_mut(b)?.copy_from_slice(&tmp);
                off += take;
            }
            inod.size = len;
            g.write_inode(&inod)?;
        }
        g.sync_sb();
        Ok(())
    }

    fn chmod(&self, path: &str, mode: u32) -> Result<(), FileSystemError> {
        let mut g = self.inner.write();
        let ino = g.lookup(path)?;
        let mut inod = g.read_inode(ino)?;
        match inod.mode & S_IFMT {
            S_IFDIR => inod.mode = S_IFDIR | (mode & 0o7777),
            S_IFREG => inod.mode = S_IFREG | (mode & 0o7777),
            S_IFLNK => return Ok(()),
            _ => return Err(FileSystemError::NotImplemented),
        }
        g.write_inode(&inod)?;
        Ok(())
    }

    fn mount(&mut self, device: &str, _options: &str) -> Result<(), FileSystemError> {
        let _ = device;
        let mut g = self.inner.write();
        if g.img.is_empty() {
            let img = build_image(16 * 1024 * 1024, "bOS")?;
            g.sb = Inner::parse_sb(&img)?;
            g.img = img;
        } else {
            g.sb = Inner::parse_sb(&g.img)?;
        }
        g.mounted = true;
        info!(
            "Mounted bFS on {}: {} blocks free",
            self.device, g.sb.free_blocks
        );
        Ok(())
    }

    fn unmount(&mut self) -> Result<(), FileSystemError> {
        let mut g = self.inner.write();
        g.sync_sb();
        g.mounted = false;
        Ok(())
    }
}

impl BfsFileSystem {
    fn lookup_exists(&self, path: &str) -> Result<bool, FileSystemError> {
        let g = self.inner.read();
        Ok(g.lookup(path).is_ok())
    }
}

/// bFS File handle.
pub struct BfsFile {
    fs: Arc<RwLock<Inner>>,
    ino: u64,
    offset: u64,
    readable: bool,
    writable: bool,
    append: bool,
}

unsafe impl Send for BfsFile {}
unsafe impl Sync for BfsFile {}

impl File for BfsFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileSystemError> {
        if !self.readable {
            return Err(FileSystemError::PermissionDenied);
        }
        let g = self.fs.read();
        let inod = g.read_inode(self.ino)?;
        if inod.mode & S_IFMT == S_IFDIR {
            return Err(FileSystemError::IsDirectory);
        }
        if self.offset >= inod.size {
            return Ok(0);
        }
        let mut read = 0usize;
        while read < buf.len() && self.offset < inod.size {
            let idx = self.offset / BLOCK_SIZE_U64;
            let boff = (self.offset % BLOCK_SIZE_U64) as usize;
            let b = g.file_block(&inod, idx)?;
            if b == 0 {
                // sparse: zeros
                let take = (buf.len() - read)
                    .min((BLOCK_SIZE - boff).min((inod.size - self.offset) as usize));
                buf[read..read + take].fill(0);
                read += take;
                self.offset += take as u64;
                continue;
            }
            let blk = g.blk(b)?;
            let take = (buf.len() - read)
                .min((BLOCK_SIZE - boff).min((inod.size - self.offset) as usize));
            buf[read..read + take].copy_from_slice(&blk[boff..boff + take]);
            read += take;
            self.offset += take as u64;
        }
        Ok(read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FileSystemError> {
        if !self.writable {
            return Err(FileSystemError::PermissionDenied);
        }
        let mut g = self.fs.write();
        let mut inod = g.read_inode(self.ino)?;
        if inod.mode & S_IFMT == S_IFDIR {
            return Err(FileSystemError::IsDirectory);
        }
        if inod.mode & S_IFMT != S_IFREG && inod.mode & S_IFMT != S_IFLNK {
            return Err(FileSystemError::InvalidArgument);
        }
        if self.append {
            self.offset = inod.size;
        }
        // zero-fill gap when seeking past EOF
        if self.offset > inod.size {
            let gap = (self.offset - inod.size) as usize;
            let mut data = g.read_data(&inod)?;
            data.resize(data.len() + gap, 0);
            let ino = self.ino;
            g.free_file_blocks(&mut inod)?;
            let mut off = 0usize;
            let mut idx = 0u64;
            while off < data.len() {
                let b = g.file_block_alloc(&mut inod, idx)?;
                idx += 1;
                let take = (data.len() - off).min(BLOCK_SIZE);
                let mut tmp = [0u8; BLOCK_SIZE];
                tmp[..take].copy_from_slice(&data[off..off + take]);
                g.blk_mut(b)?.copy_from_slice(&tmp);
                off += take;
            }
            inod.size = data.len() as u64;
            g.write_inode(&inod)?;
            let _ = ino;
        }
        let mut written = 0usize;
        while written < buf.len() {
            let idx = self.offset / BLOCK_SIZE_U64;
            let boff = (self.offset % BLOCK_SIZE_U64) as usize;
            let b = g.file_block_alloc(&mut inod, idx)?;
            let take = (buf.len() - written).min(BLOCK_SIZE - boff);
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(g.blk(b)?);
            tmp[boff..boff + take].copy_from_slice(&buf[written..written + take]);
            g.blk_mut(b)?.copy_from_slice(&tmp);
            written += take;
            self.offset += take as u64;
        }
        if self.offset > inod.size {
            inod.size = self.offset;
            g.write_inode(&inod)?;
        }
        g.sync_sb();
        Ok(written)
    }

    fn seek(&mut self, pos: i64, whence: u32) -> Result<u64, FileSystemError> {
        let g = self.fs.read();
        let inod = g.read_inode(self.ino)?;
        let base: i64 = match whence {
            SEEK_SET => 0,
            SEEK_CUR => self.offset as i64,
            SEEK_END => inod.size as i64,
            _ => return Err(FileSystemError::InvalidArgument),
        };
        let next = base.checked_add(pos).ok_or(FileSystemError::InvalidArgument)?;
        if next < 0 {
            return Err(FileSystemError::InvalidArgument);
        }
        self.offset = next as u64;
        Ok(self.offset)
    }

    fn close(&mut self) -> Result<(), FileSystemError> {
        Ok(())
    }

    fn stat(&self) -> Result<FileStat, FileSystemError> {
        let g = self.fs.read();
        Ok(bfs_stat(self.ino, &g.read_inode(self.ino)?))
    }
}
