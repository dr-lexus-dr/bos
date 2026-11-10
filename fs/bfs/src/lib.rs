//! bFS — bOS File System, host-side implementation.
//!
//! On-disk layout (little-endian, `BLOCK_SIZE` = 4096):
//! * block 0: superblock (one block, checksummed with FNV-1a)
//! * next N blocks: block bitmap (bit `i` = block `i` used)
//! * next M blocks: inode bitmap (bit `i` = inode `i+1` used)
//! * next K blocks: inode table (`INODE_SIZE` = 256 B, 16 per block)
//! * rest: data blocks (dirs = packed fixed-size entries, files = direct + single indirect)
//!
//! Inode numbers start at 1. Inode 1 is always the root directory.
//! This layout is mirrored by `kernel/src/fs/bfs.rs` (same constants,
//! same encode/decode). Keep them in sync.

pub const BFS_MAGIC: u64 = 0x4246534F464E4942;
pub const BFS_VERSION: u32 = 1;
pub const BLOCK_SIZE: usize = 4096;
pub const BLOCK_SIZE_U64: u64 = 4096;
pub const INODE_SIZE: usize = 256;
pub const INODES_PER_BLOCK: u64 = (BLOCK_SIZE / INODE_SIZE) as u64; // 16
pub const ROOT_INO: u64 = 1;
pub const NDIRECT: usize = 12;
/// Pointers per indirect block (4096 / 8).
pub const PTRS_PER_BLOCK: usize = BLOCK_SIZE / 8; // 512

// ---- mode bits (Linux-compatible subset) ----
pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFIFO: u32 = 0o010000;

// ---- dir entry file types ----
pub const FT_UNKNOWN: u8 = 0;
pub const FT_REG_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;
pub const FT_CHRDEV: u8 = 3;
pub const FT_BLKDEV: u8 = 4;
pub const FT_FIFO: u8 = 5;
pub const FT_SOCK: u8 = 6;
pub const FT_SYMLINK: u8 = 7;

pub const MAX_NAME: usize = 255;
/// Encoded dir entry size: u64 ino + u16 len + u8 type + u8 res + 255 name = 267.
pub const DIR_ENTRY_SIZE: usize = 8 + 2 + 1 + 1 + MAX_NAME;
pub const ENTRIES_PER_BLOCK: usize = BLOCK_SIZE / DIR_ENTRY_SIZE; // 15

// ---- superblock field offsets (LE) ----
const SB_OFF_MAGIC: usize = 0;
const SB_OFF_VERSION: usize = 8;
const SB_OFF_BLOCK_SIZE: usize = 12;
const SB_OFF_TOTAL_BLOCKS: usize = 16;
const SB_OFF_FREE_BLOCKS: usize = 24;
const SB_OFF_ROOT_INODE: usize = 32;
const SB_OFF_INODE_COUNT: usize = 40;
const SB_OFF_FREE_INODES: usize = 48;
const SB_OFF_BB_START: usize = 56;
const SB_OFF_IB_START: usize = 64;
const SB_OFF_IT_START: usize = 72;
const SB_OFF_DATA_START: usize = 80;
const SB_OFF_LABEL: usize = 88; // 32 bytes
const SB_OFF_CREATED: usize = 120;
const SB_OFF_MODIFIED: usize = 128;
const SB_OFF_CHECKSUM: usize = 136;

// ---- inode field offsets (LE, 256 B) ----
const IN_OFF_NUM: usize = 0;
const IN_OFF_MODE: usize = 8;
const IN_OFF_UID: usize = 12;
const IN_OFF_GID: usize = 16;
const IN_OFF_SIZE: usize = 20;
const IN_OFF_ATIME: usize = 28;
const IN_OFF_MTIME: usize = 36;
const IN_OFF_CTIME: usize = 44;
const IN_OFF_LINKS: usize = 52;
const IN_OFF_FLAGS: usize = 56;
const IN_OFF_DIRECT: usize = 60; // 12 x u64 = 96 bytes
const IN_OFF_INDIRECT: usize = 60 + 12 * 8; // 156
const IN_OFF_GEN: usize = 164;

#[derive(Debug, Clone)]
pub struct MkfsOptions {
    pub label: String,
    pub size_mb: u64,
}

impl Default for MkfsOptions {
    fn default() -> Self {
        Self { label: "bOS".into(), size_mb: 64 }
    }
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub inode: u64,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub links: u32,
    pub is_dir: bool,
    pub is_symlink: bool,
}

#[derive(Debug, Clone)]
pub struct SuperBlock {
    pub magic: u64,
    pub version: u32,
    pub block_size: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub root_inode: u64,
    pub inode_count: u64,
    pub free_inodes: u64,
    pub block_bitmap_start: u64,
    pub inode_bitmap_start: u64,
    pub inode_table_start: u64,
    pub data_start: u64,
    pub label: [u8; 32],
    pub created_time: u64,
    pub modified_time: u64,
    pub checksum: u32,
}

#[derive(Debug, Clone)]
pub struct Inode {
    pub inode_num: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub links: u32,
    pub flags: u32,
    pub direct: [u64; NDIRECT],
    pub indirect: u64,
    pub generation: u32,
}

// ---------------------------------------------------------------------------
// small LE helpers
// ---------------------------------------------------------------------------

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn rd_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off], b[off + 1], b[off + 2], b[off + 3], b[off + 4], b[off + 5], b[off + 6],
        b[off + 7],
    ])
}
fn wr_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn wr_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn wr_u64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn sb_checksum(block: &[u8]) -> u32 {
    // FNV over whole block with checksum field zeroed.
    let mut h: u32 = 0x811c_9dc5;
    for (i, &b) in block.iter().enumerate().take(BLOCK_SIZE) {
        let v = if (SB_OFF_CHECKSUM..SB_OFF_CHECKSUM + 4).contains(&i) { 0 } else { b };
        h ^= v as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn decode_sb(block: &[u8]) -> Result<SuperBlock, String> {
    if block.len() < BLOCK_SIZE {
        return Err("image smaller than one block".into());
    }
    let mut label = [0u8; 32];
    label.copy_from_slice(&block[SB_OFF_LABEL..SB_OFF_LABEL + 32]);
    Ok(SuperBlock {
        magic: rd_u64(block, SB_OFF_MAGIC),
        version: rd_u32(block, SB_OFF_VERSION),
        block_size: rd_u32(block, SB_OFF_BLOCK_SIZE),
        total_blocks: rd_u64(block, SB_OFF_TOTAL_BLOCKS),
        free_blocks: rd_u64(block, SB_OFF_FREE_BLOCKS),
        root_inode: rd_u64(block, SB_OFF_ROOT_INODE),
        inode_count: rd_u64(block, SB_OFF_INODE_COUNT),
        free_inodes: rd_u64(block, SB_OFF_FREE_INODES),
        block_bitmap_start: rd_u64(block, SB_OFF_BB_START),
        inode_bitmap_start: rd_u64(block, SB_OFF_IB_START),
        inode_table_start: rd_u64(block, SB_OFF_IT_START),
        data_start: rd_u64(block, SB_OFF_DATA_START),
        label,
        created_time: rd_u64(block, SB_OFF_CREATED),
        modified_time: rd_u64(block, SB_OFF_MODIFIED),
        checksum: rd_u32(block, SB_OFF_CHECKSUM),
    })
}

fn encode_sb(sb: &SuperBlock, block: &mut [u8]) {
    assert!(block.len() >= BLOCK_SIZE);
    block[..BLOCK_SIZE].fill(0);
    wr_u64(block, SB_OFF_MAGIC, sb.magic);
    wr_u32(block, SB_OFF_VERSION, sb.version);
    wr_u32(block, SB_OFF_BLOCK_SIZE, sb.block_size);
    wr_u64(block, SB_OFF_TOTAL_BLOCKS, sb.total_blocks);
    wr_u64(block, SB_OFF_FREE_BLOCKS, sb.free_blocks);
    wr_u64(block, SB_OFF_ROOT_INODE, sb.root_inode);
    wr_u64(block, SB_OFF_INODE_COUNT, sb.inode_count);
    wr_u64(block, SB_OFF_FREE_INODES, sb.free_inodes);
    wr_u64(block, SB_OFF_BB_START, sb.block_bitmap_start);
    wr_u64(block, SB_OFF_IB_START, sb.inode_bitmap_start);
    wr_u64(block, SB_OFF_IT_START, sb.inode_table_start);
    wr_u64(block, SB_OFF_DATA_START, sb.data_start);
    block[SB_OFF_LABEL..SB_OFF_LABEL + 32].copy_from_slice(&sb.label);
    wr_u64(block, SB_OFF_CREATED, sb.created_time);
    wr_u64(block, SB_OFF_MODIFIED, sb.modified_time);
    wr_u32(block, SB_OFF_CHECKSUM, 0);
    let c = sb_checksum(block);
    wr_u32(block, SB_OFF_CHECKSUM, c);
}

fn decode_inode(buf: &[u8]) -> Inode {
    let mut direct = [0u64; NDIRECT];
    for i in 0..NDIRECT {
        direct[i] = rd_u64(buf, IN_OFF_DIRECT + i * 8);
    }
    Inode {
        inode_num: rd_u64(buf, IN_OFF_NUM),
        mode: rd_u32(buf, IN_OFF_MODE),
        uid: rd_u32(buf, IN_OFF_UID),
        gid: rd_u32(buf, IN_OFF_GID),
        size: rd_u64(buf, IN_OFF_SIZE),
        atime: rd_u64(buf, IN_OFF_ATIME),
        mtime: rd_u64(buf, IN_OFF_MTIME),
        ctime: rd_u64(buf, IN_OFF_CTIME),
        links: rd_u32(buf, IN_OFF_LINKS),
        flags: rd_u32(buf, IN_OFF_FLAGS),
        direct,
        indirect: rd_u64(buf, IN_OFF_INDIRECT),
        generation: rd_u32(buf, IN_OFF_GEN),
    }
}

fn encode_inode(inode: &Inode, buf: &mut [u8]) {
    assert!(buf.len() >= INODE_SIZE);
    buf[..INODE_SIZE].fill(0);
    wr_u64(buf, IN_OFF_NUM, inode.inode_num);
    wr_u32(buf, IN_OFF_MODE, inode.mode);
    wr_u32(buf, IN_OFF_UID, inode.uid);
    wr_u32(buf, IN_OFF_GID, inode.gid);
    wr_u64(buf, IN_OFF_SIZE, inode.size);
    wr_u64(buf, IN_OFF_ATIME, inode.atime);
    wr_u64(buf, IN_OFF_MTIME, inode.mtime);
    wr_u64(buf, IN_OFF_CTIME, inode.ctime);
    wr_u32(buf, IN_OFF_LINKS, inode.links);
    wr_u32(buf, IN_OFF_FLAGS, inode.flags);
    for i in 0..NDIRECT {
        wr_u64(buf, IN_OFF_DIRECT + i * 8, inode.direct[i]);
    }
    wr_u64(buf, IN_OFF_INDIRECT, inode.indirect);
    wr_u32(buf, IN_OFF_GEN, inode.generation);
}

#[derive(Debug, Clone)]
struct DirEnt {
    inode: u64,
    name: String,
    ftype: u8,
}

fn decode_dir_block(block: &[u8]) -> Vec<DirEnt> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + DIR_ENTRY_SIZE <= block.len() {
        let ino = rd_u64(block, off);
        if ino == 0 {
            break;
        }
        let nlen = rd_u16(block, off + 8) as usize;
        let ftype = block[off + 10];
        if nlen == 0 || nlen > MAX_NAME {
            break;
        }
        let name_bytes = &block[off + 12..off + 12 + nlen];
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        out.push(DirEnt { inode: ino, name, ftype });
        off += DIR_ENTRY_SIZE;
    }
    out
}

fn encode_dir_block(entries: &[DirEnt], block: &mut [u8]) {
    block.fill(0);
    let mut off = 0;
    for e in entries.iter().take(ENTRIES_PER_BLOCK) {
        let nb = e.name.as_bytes();
        let n = nb.len().min(MAX_NAME);
        wr_u64(block, off, e.inode);
        wr_u16(block, off + 8, n as u16);
        block[off + 10] = e.ftype;
        block[off + 11] = 0;
        block[off + 12..off + 12 + n].copy_from_slice(&nb[..n]);
        off += DIR_ENTRY_SIZE;
    }
    // trailing entries stay zero (inode 0 = end marker)
}

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

fn div_ceil(a: u64, b: u64) -> u64 {
    (a + b - 1) / b
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    total_blocks: u64,
    inode_count: u64,
    bb_start: u64,
    #[allow(dead_code)]
    bb_blocks: u64,
    ib_start: u64,
    #[allow(dead_code)]
    ib_blocks: u64,
    it_start: u64,
    #[allow(dead_code)]
    it_blocks: u64,
    data_start: u64,
}

fn plan_layout(size_bytes: u64) -> Result<Layout, String> {
    if size_bytes < 4 * 1024 * 1024 {
        return Err("bFS image must be at least 4 MiB".into());
    }
    if size_bytes % BLOCK_SIZE_U64 != 0 {
        return Err("image size must be a multiple of 4096".into());
    }
    let total_blocks = size_bytes / BLOCK_SIZE_U64;
    let inode_count = (total_blocks / 4).max(16);
    // iterate: bitmap sizes depend only on totals, table on inode count
    for _ in 0..4 {
        let bb = div_ceil(total_blocks, BLOCK_SIZE_U64 * 8);
        let ib = div_ceil(inode_count, BLOCK_SIZE_U64 * 8);
        let it = div_ceil(inode_count * INODE_SIZE as u64, BLOCK_SIZE_U64);
        let data_start = 1 + bb + ib + it;
        if data_start >= total_blocks {
            return Err("image too small for bFS metadata".into());
        }
        // inode_count stable wrt totals; keep as is
        return Ok(Layout {
            total_blocks,
            inode_count,
            bb_start: 1,
            bb_blocks: bb,
            ib_start: 1 + bb,
            ib_blocks: ib,
            it_start: 1 + bb + ib,
            it_blocks: it,
            data_start,
        });
    }
    unreachable!()
}

// ---------------------------------------------------------------------------
// core image handle
// ---------------------------------------------------------------------------

struct Img<'a> {
    data: &'a mut [u8],
    sb: SuperBlock,
}

impl<'a> Img<'a> {
    fn load(data: &'a mut [u8]) -> Result<Self, String> {
        if data.len() < BLOCK_SIZE || data.len() % BLOCK_SIZE != 0 {
            return Err("image size must be a non-zero multiple of 4096".into());
        }
        let sb = decode_sb(data)?;
        if sb.magic != BFS_MAGIC {
            return Err(format!("bad magic 0x{:016x}", sb.magic));
        }
        if sb.version != BFS_VERSION {
            return Err(format!("unsupported version {}", sb.version));
        }
        if sb.block_size as usize != BLOCK_SIZE {
            return Err("bad block size".into());
        }
        if sb.total_blocks * BLOCK_SIZE_U64 != data.len() as u64 {
            return Err("superblock total_blocks does not match image size".into());
        }
        let expect = sb_checksum(&data[..BLOCK_SIZE]);
        if expect != sb.checksum {
            return Err("superblock checksum mismatch".into());
        }
        Ok(Self { data, sb })
    }

    fn flush_sb(&mut self) {
        let mut blk = [0u8; BLOCK_SIZE];
        encode_sb(&self.sb, &mut blk);
        self.data[..BLOCK_SIZE].copy_from_slice(&blk);
    }

    fn block(&self, n: u64) -> Result<&[u8], String> {
        let off = n as usize * BLOCK_SIZE;
        if n >= self.sb.total_blocks || off + BLOCK_SIZE > self.data.len() {
            return Err(format!("block {} out of range", n));
        }
        Ok(&self.data[off..off + BLOCK_SIZE])
    }

    fn block_mut(&mut self, n: u64) -> Result<&mut [u8], String> {
        let total = self.sb.total_blocks;
        if n >= total {
            return Err(format!("block {} out of range", n));
        }
        let off = n as usize * BLOCK_SIZE;
        Ok(&mut self.data[off..off + BLOCK_SIZE])
    }

    // -- bitmaps --
    fn bit_get(&self, start: u64, bit: u64) -> Result<bool, String> {
        let byte = bit / 8;
        let blk = start + byte / BLOCK_SIZE_U64;
        let off = (byte % BLOCK_SIZE_U64) as usize;
        Ok(self.block(blk)?[off] & (1 << (bit % 8)) != 0)
    }

    fn bit_set(&mut self, start: u64, bit: u64, v: bool) -> Result<(), String> {
        let byte = bit / 8;
        let blk = start + byte / BLOCK_SIZE_U64;
        let off = (byte % BLOCK_SIZE_U64) as usize;
        let b = self.block_mut(blk)?;
        if v {
            b[off] |= 1 << (bit % 8);
        } else {
            b[off] &= !(1 << (bit % 8));
        }
        Ok(())
    }

    fn alloc_block(&mut self) -> Result<u64, String> {
        if self.sb.free_blocks == 0 {
            return Err("no space left (blocks)".into());
        }
        let total = self.sb.total_blocks;
        let start = self.sb.block_bitmap_start;
        for b in self.sb.data_start..total {
            if !self.bit_get(start, b)? {
                self.bit_set(start, b, true)?;
                self.block_mut(b)?.fill(0);
                self.sb.free_blocks -= 1;
                return Ok(b);
            }
        }
        Err("no space left (blocks)".into())
    }

    fn free_block(&mut self, b: u64) -> Result<(), String> {
        if b < self.sb.data_start || b >= self.sb.total_blocks {
            return Err("bad block number".into());
        }
        if self.bit_get(self.sb.block_bitmap_start, b)? {
            self.bit_set(self.sb.block_bitmap_start, b, false)?;
            self.sb.free_blocks += 1;
        }
        Ok(())
    }

    fn alloc_inode(&mut self) -> Result<u64, String> {
        if self.sb.free_inodes == 0 {
            return Err("no space left (inodes)".into());
        }
        let count = self.sb.inode_count;
        let start = self.sb.inode_bitmap_start;
        for i in 1..=count {
            if !self.bit_get(start, i - 1)? {
                self.bit_set(start, i - 1, true)?;
                self.sb.free_inodes -= 1;
                return Ok(i);
            }
        }
        Err("no space left (inodes)".into())
    }

    fn free_inode_num(&mut self, ino: u64) -> Result<(), String> {
        if ino == 0 || ino > self.sb.inode_count || ino == ROOT_INO {
            return Err("bad inode number".into());
        }
        self.bit_set(self.sb.inode_bitmap_start, ino - 1, false)?;
        self.sb.free_inodes += 1;
        // zero the slot
        let mut slot = [0u8; INODE_SIZE];
        encode_inode(
            &Inode {
                inode_num: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                size: 0,
                atime: 0,
                mtime: 0,
                ctime: 0,
                links: 0,
                flags: 0,
                direct: [0; NDIRECT],
                indirect: 0,
                generation: 0,
            },
            &mut slot,
        );
        self.write_inode_raw(ino, &slot)?;
        Ok(())
    }

    fn inode_block_off(&self, ino: u64) -> Result<(u64, usize), String> {
        if ino == 0 || ino > self.sb.inode_count {
            return Err(format!("bad inode {}", ino));
        }
        let idx = ino - 1;
        let blk = self.sb.inode_table_start + idx / INODES_PER_BLOCK;
        let off = (idx % INODES_PER_BLOCK) as usize * INODE_SIZE;
        Ok((blk, off))
    }

    fn read_inode(&self, ino: u64) -> Result<Inode, String> {
        let (blk, off) = self.inode_block_off(ino)?;
        let b = self.block(blk)?;
        let mut slot = [0u8; INODE_SIZE];
        slot.copy_from_slice(&b[off..off + INODE_SIZE]);
        let inod = decode_inode(&slot);
        if inod.mode == 0 && ino != 0 {
            return Err(format!("inode {} is free", ino));
        }
        Ok(inod)
    }

    fn write_inode_raw(&mut self, ino: u64, slot: &[u8; INODE_SIZE]) -> Result<(), String> {
        let (blk, off) = self.inode_block_off(ino)?;
        let b = self.block_mut(blk)?;
        b[off..off + INODE_SIZE].copy_from_slice(slot);
        Ok(())
    }

    fn write_inode(&mut self, inod: &Inode) -> Result<(), String> {
        let mut slot = [0u8; INODE_SIZE];
        encode_inode(inod, &mut slot);
        self.write_inode_raw(inod.inode_num, &slot)
    }

    // -- file block mapping (direct + single indirect) --
    fn file_block(&self, inod: &Inode, idx: u64) -> Result<u64, String> {
        if idx < NDIRECT as u64 {
            return Ok(inod.direct[idx as usize]);
        }
        let rel = idx - NDIRECT as u64;
        if rel >= PTRS_PER_BLOCK as u64 || inod.indirect == 0 {
            return Ok(0);
        }
        let ib = self.block(inod.indirect)?;
        Ok(rd_u64(ib, rel as usize * 8))
    }

    fn file_block_mut_alloc(&mut self, inod: &mut Inode, idx: u64) -> Result<u64, String> {
        if idx < NDIRECT as u64 {
            if inod.direct[idx as usize] == 0 {
                inod.direct[idx as usize] = self.alloc_block()?;
            }
            return Ok(inod.direct[idx as usize]);
        }
        let rel = idx - NDIRECT as u64;
        if rel >= PTRS_PER_BLOCK as u64 {
            return Err("file too large (single-indirect limit ~2 MiB)".into());
        }
        if inod.indirect == 0 {
            inod.indirect = self.alloc_block()?;
        }
        let ib = inod.indirect;
        let cur = rd_u64(self.block(ib)?, rel as usize * 8);
        if cur == 0 {
            let nb = self.alloc_block()?;
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.block(ib)?);
            wr_u64(&mut tmp, rel as usize * 8, nb);
            self.block_mut(ib)?.copy_from_slice(&tmp);
            return Ok(nb);
        }
        Ok(cur)
    }

    fn free_file_blocks(&mut self, inod: &mut Inode) -> Result<(), String> {
        for i in 0..NDIRECT {
            if inod.direct[i] != 0 {
                self.free_block(inod.direct[i])?;
                inod.direct[i] = 0;
            }
        }
        if inod.indirect != 0 {
            let mut ptrs = [0u8; BLOCK_SIZE];
            ptrs.copy_from_slice(self.block(inod.indirect)?);
            for i in 0..PTRS_PER_BLOCK {
                let p = rd_u64(&ptrs, i * 8);
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

    fn read_file_data(&self, inod: &Inode) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(inod.size.min(1 << 20) as usize);
        let mut left = inod.size;
        let mut idx = 0u64;
        while left > 0 {
            let b = self.file_block(inod, idx)?;
            idx += 1;
            if b == 0 {
                // sparse hole
                let take = left.min(BLOCK_SIZE_U64);
                out.extend(core::iter::repeat(0).take(take as usize));
                left -= take;
                continue;
            }
            let blk = self.block(b)?;
            let take = left.min(BLOCK_SIZE_U64) as usize;
            out.extend_from_slice(&blk[..take]);
            left -= take as u64;
        }
        Ok(out)
    }

    fn write_file_data(&mut self, ino: u64, data: &[u8]) -> Result<(), String> {
        let mut inod = self.read_inode(ino)?;
        self.free_file_blocks(&mut inod)?;
        let mut off = 0usize;
        let mut idx = 0u64;
        while off < data.len() {
            let b = self.file_block_mut_alloc(&mut inod, idx)?;
            idx += 1;
            let take = (data.len() - off).min(BLOCK_SIZE);
            // copy via temp to satisfy borrowck
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp[..take].copy_from_slice(&data[off..off + take]);
            self.block_mut(b)?.copy_from_slice(&tmp);
            off += take;
        }
        inod.size = data.len() as u64;
        inod.mtime = inod.ctime;
        self.write_inode(&inod)?;
        Ok(())
    }

    // -- paths --
    fn split(path: &str) -> Vec<String> {
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

    fn lookup(&self, path: &str) -> Result<u64, String> {
        let comps = Self::split(path);
        let mut cur = self.sb.root_inode;
        for c in &comps {
            let inod = self.read_inode(cur)?;
            if inod.mode & S_IFMT != S_IFDIR {
                return Err(format!("not a directory in '{}'", path));
            }
            cur = self.find_in_dir(&inod, c)?.ok_or_else(|| format!("not found: '{}'", path))?;
        }
        Ok(cur)
    }

    fn find_in_dir(&self, dir: &Inode, name: &str) -> Result<Option<u64>, String> {
        let nblocks = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..nblocks {
            let b = self.file_block(dir, i)?;
            if b == 0 {
                continue;
            }
            let blk = self.block(b)?;
            for e in decode_dir_block(blk) {
                if e.name == name {
                    return Ok(Some(e.inode));
                }
            }
        }
        Ok(None)
    }

    fn all_dir_entries(&self, dir: &Inode) -> Result<Vec<DirEnt>, String> {
        let mut out = Vec::new();
        let nblocks = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..nblocks {
            let b = self.file_block(dir, i)?;
            if b == 0 {
                continue;
            }
            let blk = self.block(b)?;
            out.extend(decode_dir_block(blk));
        }
        Ok(out)
    }

    fn dir_add_entry(
        &mut self,
        dir_ino: u64,
        name: &str,
        child: u64,
        ftype: u8,
    ) -> Result<(), String> {
        if name.is_empty() || name.len() > MAX_NAME || name.contains('/') {
            return Err("bad name".into());
        }
        let mut dir = self.read_inode(dir_ino)?;
        if dir.mode & S_IFMT != S_IFDIR {
            return Err("not a directory".into());
        }
        if self.find_in_dir(&dir, name)?.is_some() {
            return Err(format!("already exists: '{}'", name));
        }
        // find a block with room
        let mut nblocks = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..nblocks {
            let b = self.file_block(&dir.clone(), i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.block(b)?);
            let mut ents = decode_dir_block(&tmp);
            if ents.len() < ENTRIES_PER_BLOCK {
                ents.push(DirEnt { inode: child, name: name.into(), ftype });
                encode_dir_block(&ents, &mut tmp);
                self.block_mut(b)?.copy_from_slice(&tmp);
                dir.mtime = dir.ctime;
                self.write_inode(&dir)?;
                return Ok(());
            }
        }
        // need new block
        if nblocks >= NDIRECT as u64 + PTRS_PER_BLOCK as u64 {
            return Err("directory too large".into());
        }
        let nb = self.file_block_mut_alloc(&mut dir, nblocks)?;
        nblocks += 1;
        let ents = vec![DirEnt { inode: child, name: name.into(), ftype }];
        let mut tmp = [0u8; BLOCK_SIZE];
        encode_dir_block(&ents, &mut tmp);
        self.block_mut(nb)?.copy_from_slice(&tmp);
        dir.size = nblocks * BLOCK_SIZE_U64;
        dir.mtime = dir.ctime;
        self.write_inode(&dir)?;
        Ok(())
    }

    fn dir_remove_entry(&mut self, dir_ino: u64, name: &str) -> Result<u64, String> {
        let mut dir = self.read_inode(dir_ino)?;
        if dir.mode & S_IFMT != S_IFDIR {
            return Err("not a directory".into());
        }
        let nblocks = ((dir.size + BLOCK_SIZE_U64 - 1) / BLOCK_SIZE_U64).max(1);
        for i in 0..nblocks {
            let b = self.file_block(&dir.clone(), i)?;
            if b == 0 {
                continue;
            }
            let mut tmp = [0u8; BLOCK_SIZE];
            tmp.copy_from_slice(self.block(b)?);
            let mut ents = decode_dir_block(&tmp);
            if let Some(pos) = ents.iter().position(|e| e.name == name) {
                let child = ents[pos].inode;
                ents.remove(pos);
                encode_dir_block(&ents, &mut tmp);
                self.block_mut(b)?.copy_from_slice(&tmp);
                dir.mtime = dir.ctime;
                self.write_inode(&dir)?;
                return Ok(child);
            }
        }
        Err(format!("not found: '{}'", name))
    }
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

pub fn mkfs(img: &mut Vec<u8>, opt: &MkfsOptions) -> Result<(), String> {
    let size = opt.size_mb.checked_mul(1024 * 1024).ok_or("size overflow")?;
    let lay = plan_layout(size)?;
    img.resize(size as usize, 0);
    img.fill(0);

    let mut sb = SuperBlock {
        magic: BFS_MAGIC,
        version: BFS_VERSION,
        block_size: BLOCK_SIZE as u32,
        total_blocks: lay.total_blocks,
        free_blocks: lay.total_blocks - lay.data_start - 1, // -1 root data block
        root_inode: ROOT_INO,
        inode_count: lay.inode_count,
        free_inodes: lay.inode_count - 1,
        block_bitmap_start: lay.bb_start,
        inode_bitmap_start: lay.ib_start,
        inode_table_start: lay.it_start,
        data_start: lay.data_start,
        label: [0; 32],
        created_time: 0,
        modified_time: 0,
        checksum: 0,
    };
    let lb = opt.label.as_bytes();
    sb.label[..lb.len().min(32)].copy_from_slice(&lb[..lb.len().min(32)]);

    // mark reserved blocks used
    {
        let data = img.as_mut_slice();
        for b in 0..lay.data_start {
            let byte = (b / 8) as usize;
            let blk = lay.bb_start + byte as u64 / BLOCK_SIZE_U64;
            let off = byte % BLOCK_SIZE;
            let abs = blk as usize * BLOCK_SIZE + off;
            data[abs] |= 1 << (b % 8);
        }
        // root data block
        let rb = lay.data_start;
        let byte = (rb / 8) as usize;
        let blk = lay.bb_start + byte as u64 / BLOCK_SIZE_U64;
        let off = byte % BLOCK_SIZE;
        data[blk as usize * BLOCK_SIZE + off] |= 1 << (rb % 8);
        // root inode used
        let ibyte = 0usize;
        let iblk = lay.ib_start;
        data[iblk as usize * BLOCK_SIZE + ibyte] |= 1;
    }

    // root inode: dir with "." and ".."
    {
        let root_blk = lay.data_start;
        let mut tmp = [0u8; BLOCK_SIZE];
        encode_dir_block(
            &[
                DirEnt { inode: ROOT_INO, name: ".".into(), ftype: FT_DIR },
                DirEnt { inode: ROOT_INO, name: "..".into(), ftype: FT_DIR },
            ],
            &mut tmp,
        );
        img[root_blk as usize * BLOCK_SIZE..root_blk as usize * BLOCK_SIZE + BLOCK_SIZE]
            .copy_from_slice(&tmp);

        let root = Inode {
            inode_num: ROOT_INO,
            mode: S_IFDIR | 0o755,
            uid: 0,
            gid: 0,
            size: BLOCK_SIZE_U64,
            atime: 0,
            mtime: 0,
            ctime: 0,
            links: 2,
            flags: 0,
            direct: {
                let mut d = [0u64; NDIRECT];
                d[0] = root_blk;
                d
            },
            indirect: 0,
            generation: 1,
        };
        let mut slot = [0u8; INODE_SIZE];
        encode_inode(&root, &mut slot);
        let abs = lay.it_start as usize * BLOCK_SIZE;
        img[abs..abs + INODE_SIZE].copy_from_slice(&slot);
    }

    let mut blk = [0u8; BLOCK_SIZE];
    encode_sb(&sb, &mut blk);
    img[..BLOCK_SIZE].copy_from_slice(&blk);
    Ok(())
}

pub fn fsck(img: &[u8]) -> Result<(), String> {
    if img.len() < BLOCK_SIZE || img.len() % BLOCK_SIZE != 0 {
        return Err("image size must be a non-zero multiple of 4096".into());
    }
    let sb = decode_sb(img)?;
    if sb.magic != BFS_MAGIC {
        return Err(format!("bad magic 0x{:016x}", sb.magic));
    }
    if sb.version != BFS_VERSION {
        return Err(format!("unsupported version {}", sb.version));
    }
    if sb.block_size as usize != BLOCK_SIZE {
        return Err("bad block size".into());
    }
    if sb.total_blocks * BLOCK_SIZE_U64 != img.len() as u64 {
        return Err("total_blocks mismatch".into());
    }
    if sb_checksum(&img[..BLOCK_SIZE]) != sb.checksum {
        return Err("superblock checksum mismatch".into());
    }
    if sb.root_inode != ROOT_INO {
        return Err("bad root inode".into());
    }
    if sb.data_start >= sb.total_blocks || sb.inode_table_start >= sb.data_start {
        return Err("bad metadata pointers".into());
    }
    if sb.free_blocks > sb.total_blocks || sb.free_inodes > sb.inode_count {
        return Err("bad free counts".into());
    }
    // root must be a directory
    let idx = sb.root_inode - 1;
    let blk_n = sb.inode_table_start + idx / INODES_PER_BLOCK;
    let off = (idx % INODES_PER_BLOCK) as usize * INODE_SIZE;
    let abs = blk_n as usize * BLOCK_SIZE + off;
    let root = decode_inode(&img[abs..abs + INODE_SIZE]);
    if root.mode & S_IFMT != S_IFDIR {
        return Err("root is not a directory".into());
    }
    if root.direct[0] < sb.data_start || root.direct[0] >= sb.total_blocks {
        return Err("root data block out of range".into());
    }
    Ok(())
}

fn with_img<T>(img: &mut [u8], f: impl FnOnce(&mut Img<'_>) -> Result<T, String>) -> Result<T, String> {
    let mut h = Img::load(img)?;
    let r = f(&mut h)?;
    h.flush_sb();
    Ok(r)
}

#[allow(dead_code)]
fn ftype_of(mode: u32) -> u8 {
    match mode & S_IFMT {
        S_IFDIR => FT_DIR,
        S_IFLNK => FT_SYMLINK,
        S_IFCHR => FT_CHRDEV,
        S_IFBLK => FT_BLKDEV,
        S_IFIFO => FT_FIFO,
        _ => FT_REG_FILE,
    }
}

pub fn stat(img: &[u8], path: &str) -> Result<FileInfo, String> {
    let mut copy = img.to_vec();
    // read-only: avoid flush by working on a scratch handle without writeback
    let h = Img::load(&mut copy)?;
    let ino = h.lookup(path)?;
    let inod = h.read_inode(ino)?;
    Ok(FileInfo {
        inode: ino,
        size: inod.size,
        mode: inod.mode,
        uid: inod.uid,
        gid: inod.gid,
        links: inod.links,
        is_dir: inod.mode & S_IFMT == S_IFDIR,
        is_symlink: inod.mode & S_IFMT == S_IFLNK,
    })
}

pub fn list_dir(img: &[u8], path: &str) -> Result<Vec<(String, u8)>, String> {
    let mut copy = img.to_vec();
    let h = Img::load(&mut copy)?;
    let ino = h.lookup(path)?;
    let inod = h.read_inode(ino)?;
    if inod.mode & S_IFMT != S_IFDIR {
        return Err(format!("not a directory: '{}'", path));
    }
    Ok(h.all_dir_entries(&inod)?.into_iter().map(|e| (e.name, e.ftype)).collect())
}

pub fn mkdir(img: &mut [u8], path: &str) -> Result<(), String> {
    with_img(img, |h| {
        let comps = Img::split(path);
        if comps.is_empty() {
            return Err("cannot mkdir /".into());
        }
        let (parent, name) = comps.split_at(comps.len() - 1);
        let mut cur = h.sb.root_inode;
        for c in parent {
            let inod = h.read_inode(cur)?;
            cur = h.find_in_dir(&inod, c)?.ok_or_else(|| format!("not found: '{}'", path))?;
        }
        let p = h.read_inode(cur)?;
        if p.mode & S_IFMT != S_IFDIR {
            return Err("not a directory".into());
        }
        if h.find_in_dir(&p, &name[0])?.is_some() {
            return Err(format!("already exists: '{}'", path));
        }
        let ino = h.alloc_inode()?;
        let blk = h.alloc_block()?;
        let mut tmp = [0u8; BLOCK_SIZE];
        encode_dir_block(
            &[
                DirEnt { inode: ino, name: ".".into(), ftype: FT_DIR },
                DirEnt { inode: cur, name: "..".into(), ftype: FT_DIR },
            ],
            &mut tmp,
        );
        h.block_mut(blk)?.copy_from_slice(&tmp);
        h.write_inode(&Inode {
            inode_num: ino,
            mode: S_IFDIR | 0o755,
            uid: 0,
            gid: 0,
            size: BLOCK_SIZE_U64,
            atime: 0,
            mtime: 0,
            ctime: 0,
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
        h.dir_add_entry(cur, &name[0], ino, FT_DIR)?;
        // parent link count bump
        let mut pp = h.read_inode(cur)?;
        pp.links += 1;
        h.write_inode(&pp)?;
        Ok(())
    })
}

pub fn mkdir_p(img: &mut [u8], path: &str) -> Result<(), String> {
    let comps = Img::split(path);
    let mut cur = String::new();
    for c in &comps {
        cur.push('/');
        cur.push_str(c);
        match mkdir(img, &cur) {
            Ok(()) => {}
            Err(e) if e.contains("already exists") => {
                // must be a dir
                let st = stat(img, &cur)?;
                if !st.is_dir {
                    return Err(format!("not a directory: '{}'", cur));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn create_file(img: &mut [u8], path: &str, data: &[u8]) -> Result<(), String> {
    with_img(img, |h| {
        let comps = Img::split(path);
        if comps.is_empty() {
            return Err("bad path".into());
        }
        let (parent, name) = comps.split_at(comps.len() - 1);
        let mut cur = h.sb.root_inode;
        for c in parent {
            let inod = h.read_inode(cur)?;
            cur = h.find_in_dir(&inod, c)?.ok_or_else(|| format!("not found: '{}'", path))?;
        }
        let p = h.read_inode(cur)?;
        if p.mode & S_IFMT != S_IFDIR {
            return Err("not a directory".into());
        }
        if h.find_in_dir(&p, &name[0])?.is_some() {
            return Err(format!("already exists: '{}'", path));
        }
        let ino = h.alloc_inode()?;
        h.write_inode(&Inode {
            inode_num: ino,
            mode: S_IFREG | 0o644,
            uid: 0,
            gid: 0,
            size: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            links: 1,
            flags: 0,
            direct: [0; NDIRECT],
            indirect: 0,
            generation: 1,
        })?;
        h.write_file_data(ino, data)?;
        h.dir_add_entry(cur, &name[0], ino, FT_REG_FILE)?;
        Ok(())
    })
}

pub fn read_file(img: &[u8], path: &str) -> Result<Vec<u8>, String> {
    let mut copy = img.to_vec();
    let h = Img::load(&mut copy)?;
    let ino = h.lookup(path)?;
    let inod = h.read_inode(ino)?;
    match inod.mode & S_IFMT {
        S_IFREG | S_IFLNK => h.read_file_data(&inod),
        S_IFDIR => Err(format!("is a directory: '{}'", path)),
        _ => Err("unsupported file type".into()),
    }
}

pub fn write_file(img: &mut [u8], path: &str, data: &[u8]) -> Result<(), String> {
    with_img(img, |h| {
        let ino = h.lookup(path)?;
        let inod = h.read_inode(ino)?;
        if inod.mode & S_IFMT == S_IFDIR {
            return Err(format!("is a directory: '{}'", path));
        }
        if inod.mode & S_IFMT != S_IFREG && inod.mode & S_IFMT != S_IFLNK {
            return Err("unsupported file type".into());
        }
        h.write_file_data(ino, data)
    })
}

pub fn symlink(img: &mut [u8], target: &str, link_path: &str) -> Result<(), String> {
    with_img(img, |h| {
        let comps = Img::split(link_path);
        if comps.is_empty() {
            return Err("bad path".into());
        }
        let (parent, name) = comps.split_at(comps.len() - 1);
        let mut cur = h.sb.root_inode;
        for c in parent {
            let inod = h.read_inode(cur)?;
            cur = h.find_in_dir(&inod, c)?.ok_or_else(|| format!("not found: '{}'", link_path))?;
        }
        let p = h.read_inode(cur)?;
        if h.find_in_dir(&p, &name[0])?.is_some() {
            return Err(format!("already exists: '{}'", link_path));
        }
        let ino = h.alloc_inode()?;
        h.write_inode(&Inode {
            inode_num: ino,
            mode: S_IFLNK | 0o777,
            uid: 0,
            gid: 0,
            size: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            links: 1,
            flags: 0,
            direct: [0; NDIRECT],
            indirect: 0,
            generation: 1,
        })?;
        h.write_file_data(ino, target.as_bytes())?;
        h.dir_add_entry(cur, &name[0], ino, FT_SYMLINK)?;
        Ok(())
    })
}

pub fn readlink(img: &[u8], path: &str) -> Result<String, String> {
    let data = read_file(img, path)?;
    String::from_utf8(data).map_err(|_| "bad symlink target".into())
}

pub fn remove(img: &mut [u8], path: &str) -> Result<(), String> {
    with_img(img, |h| {
        let comps = Img::split(path);
        if comps.is_empty() {
            return Err("cannot remove /".into());
        }
        let (parent, name) = comps.split_at(comps.len() - 1);
        let mut cur = h.sb.root_inode;
        for c in parent {
            let inod = h.read_inode(cur)?;
            cur = h.find_in_dir(&inod, c)?.ok_or_else(|| format!("not found: '{}'", path))?;
        }
        let child_ino = h.dir_remove_entry(cur, &name[0])?;
        let mut child = h.read_inode(child_ino)?;
        if child.mode & S_IFMT == S_IFDIR {
            let ents = h.all_dir_entries(&child)?;
            let real: Vec<&DirEnt> =
                ents.iter().filter(|e| e.name != "." && e.name != "..").collect();
            if !real.is_empty() {
                // restore entry
                h.dir_add_entry(cur, &name[0], child_ino, FT_DIR)?;
                return Err(format!("directory not empty: '{}'", path));
            }
            // update parent links
            let mut pp = h.read_inode(cur)?;
            pp.links = pp.links.saturating_sub(1);
            h.write_inode(&pp)?;
        }
        child.links = child.links.saturating_sub(1);
        if child.links == 0 {
            h.free_file_blocks(&mut child)?;
            h.write_inode(&child)?;
            h.free_inode_num(child_ino)?;
        } else {
            h.write_inode(&child)?;
        }
        Ok(())
    })
}

pub fn rename(img: &mut [u8], from: &str, to: &str) -> Result<(), String> {
    let data = read_file(img, from).or_else(|_| {
        // maybe dir: check stat
        let st = stat(img, from)?;
        if st.is_dir {
            return Err(format!("rename dir via copy not supported: '{}'", from));
        }
        Err(format!("not found: '{}'", from))
    })?;
    let st = stat(img, from)?;
    if st.is_dir {
        return Err("renaming directories is not supported by this helper (use mkdir + move entries)".into());
    }
    // remove destination if it exists (file only)
    if stat(img, to).is_ok() {
        remove(img, to)?;
    } else {
        // ensure parents exist
        let comps = Img::split(to);
        if comps.len() > 1 {
            let mut p = String::new();
            for c in &comps[..comps.len() - 1] {
                p.push('/');
                p.push_str(c);
            }
            mkdir_p(img, &p)?;
        }
    }
    create_file(img, to, &data)?;
    remove(img, from)?;
    Ok(())
}

/// Usage stats: (total_bytes, free_bytes, total_inodes, free_inodes).
pub fn statvfs(img: &[u8]) -> Result<(u64, u64, u64, u64), String> {
    if img.len() < BLOCK_SIZE {
        return Err("too small".into());
    }
    let sb = decode_sb(img)?;
    if sb.magic != BFS_MAGIC {
        return Err("bad magic".into());
    }
    Ok((
        sb.total_blocks * BLOCK_SIZE_U64,
        sb.free_blocks * BLOCK_SIZE_U64,
        sb.inode_count,
        sb.free_inodes,
    ))
}

pub fn label(img: &[u8]) -> Result<String, String> {
    let sb = decode_sb(img)?;
    if sb.magic != BFS_MAGIC {
        return Err("bad magic".into());
    }
    let end = sb.label.iter().position(|&b| b == 0).unwrap_or(32);
    Ok(String::from_utf8_lossy(&sb.label[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_img(mb: u64) -> Vec<u8> {
        let mut img = Vec::new();
        mkfs(&mut img, &MkfsOptions { label: "test".into(), size_mb: mb }).unwrap();
        img
    }

    #[test]
    fn magic_ok() {
        assert_eq!(BFS_MAGIC, 0x4246534F464E4942);
        assert_eq!(BFS_VERSION, 1);
    }

    #[test]
    fn mkfs_fsck_roundtrip() {
        let mut img = vec![0u8; 1024 * 1024];
        let opt = MkfsOptions::default();
        assert!(mkfs(&mut img, &opt).is_ok());
        assert!(fsck(&img).is_ok());
    }

    #[test]
    fn file_roundtrip() {
        let mut img = test_img(8);
        assert!(fsck(&img).is_ok());
        mkdir(&mut img, "/etc").unwrap();
        create_file(&mut img, "/etc/motd", b"hello bOS").unwrap();
        assert_eq!(read_file(&img, "/etc/motd").unwrap(), b"hello bOS");
        let ents = list_dir(&img, "/etc").unwrap();
        assert!(ents.iter().any(|(n, _)| n == "motd"));
        assert!(fsck(&img).is_ok());
    }

    #[test]
    fn nested_and_overwrite() {
        let mut img = test_img(8);
        mkdir_p(&mut img, "/a/b/c").unwrap();
        create_file(&mut img, "/a/b/c/f.txt", b"v1").unwrap();
        write_file(&mut img, "/a/b/c/f.txt", b"v2 longer content here").unwrap();
        assert_eq!(read_file(&img, "/a/b/c/f.txt").unwrap(), b"v2 longer content here");
        // .. normalization
        assert_eq!(read_file(&img, "/a/b/./c/../c/f.txt").unwrap(), b"v2 longer content here");
    }

    #[test]
    fn big_file_uses_indirect() {
        let mut img = test_img(16);
        let big = vec![0xABu8; 200 * 1024]; // > 12 direct blocks (48 KiB)
        create_file(&mut img, "/big.bin", &big).unwrap();
        assert_eq!(read_file(&img, "/big.bin").unwrap(), big);
        assert!(fsck(&img).is_ok());
    }

    #[test]
    fn symlink_remove_rename() {
        let mut img = test_img(8);
        create_file(&mut img, "/orig.txt", b"data").unwrap();
        symlink(&mut img, "/orig.txt", "/link").unwrap();
        assert_eq!(readlink(&img, "/link").unwrap(), "/orig.txt");
        rename(&mut img, "/orig.txt", "/renamed.txt").unwrap();
        assert!(read_file(&img, "/orig.txt").is_err());
        assert_eq!(read_file(&img, "/renamed.txt").unwrap(), b"data");
        remove(&mut img, "/renamed.txt").unwrap();
        assert!(read_file(&img, "/renamed.txt").is_err());
        assert!(fsck(&img).is_ok());
    }

    #[test]
    fn rmdir_nonempty_fails() {
        let mut img = test_img(8);
        mkdir(&mut img, "/d").unwrap();
        create_file(&mut img, "/d/f", b"x").unwrap();
        assert!(remove(&mut img, "/d").is_err());
        remove(&mut img, "/d/f").unwrap();
        remove(&mut img, "/d").unwrap();
        assert!(fsck(&img).is_ok());
    }

    #[test]
    fn fsck_catches_corruption() {
        let mut img = test_img(8);
        img[0] ^= 0xFF;
        assert!(fsck(&img).is_err());
    }

    #[test]
    fn statvfs_sane() {
        let img = test_img(8);
        let (total, free, tin, fin) = statvfs(&img).unwrap();
        assert_eq!(total, 8 * 1024 * 1024);
        assert!(free < total && free > 0);
        assert!(fin < tin);
    }
}
