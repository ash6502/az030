//! azfs: the az030 OS's native file system. On-disk layout shared by the kernel and
//! the host-side `mkfs`.
//!
//! All integers are big-endian. Blocks are 1 KB.
//!
//! ```text
//! block 0              unused (room for a boot sector)
//! block 1              superblock
//! ibitmap_start..      inode bitmap   (bit n = inode n in use; bit 0 always set)
//! bbitmap_start..      block bitmap   (bit n = block n in use)
//! itable_start..       inode table    (8 inodes of 128 bytes per block, inode 1 first)
//! data_start..         data blocks
//! ```
//!
//! Directories are arrays of 64-byte entries: a 4-byte inode number (0 = free slot)
//! and a NUL-padded name of up to 59 bytes. Every directory has `.` and `..`.

#![no_std]

pub const BLOCK_SIZE: usize = 1024;
pub const SECTORS_PER_BLOCK: u32 = 2;
pub const MAGIC: u32 = 0x415A_4653; // "AZFS"
pub const VERSION: u32 = 1;
pub const SUPERBLOCK: u32 = 1;
pub const ROOT_INO: u32 = 1;

pub const INODE_SIZE: usize = 128;
pub const INODES_PER_BLOCK: u32 = (BLOCK_SIZE / INODE_SIZE) as u32;
pub const NDIRECT: usize = 12;
pub const PTRS_PER_BLOCK: u32 = (BLOCK_SIZE / 4) as u32;
pub const DIRENT_SIZE: usize = 64;
pub const NAME_MAX: usize = DIRENT_SIZE - 4 - 1;
pub const DIRENTS_PER_BLOCK: usize = BLOCK_SIZE / DIRENT_SIZE;
/// Largest file: direct + single + double indirect blocks.
pub const MAX_FILE_BLOCKS: u32 = NDIRECT as u32 + PTRS_PER_BLOCK + PTRS_PER_BLOCK * PTRS_PER_BLOCK;

// mode bits
pub const S_IFMT: u16 = 0o170000;
pub const S_IFSOCK: u16 = 0o140000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFIFO: u16 = 0o010000;
pub const S_ISUID: u16 = 0o4000;
pub const S_ISGID: u16 = 0o2000;
pub const S_ISVTX: u16 = 0o1000;

fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn get16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_be_bytes());
}
fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_be_bytes());
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Superblock {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub blocks: u32,
    pub inodes: u32,
    pub ibitmap_start: u32,
    pub ibitmap_blocks: u32,
    pub bbitmap_start: u32,
    pub bbitmap_blocks: u32,
    pub itable_start: u32,
    pub itable_blocks: u32,
    pub data_start: u32,
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub root_ino: u32,
    pub mtime: u32,
    /// 1 = cleanly unmounted.
    pub clean: u32,
    pub label: [u8; 16],
}

impl Superblock {
    /// Compute a layout for a file system of `blocks` blocks with `inodes` inodes.
    pub fn layout(blocks: u32, inodes: u32) -> Superblock {
        let bits = (BLOCK_SIZE * 8) as u32;
        let inodes = inodes.div_ceil(INODES_PER_BLOCK) * INODES_PER_BLOCK;
        let ibitmap_blocks = (inodes + 1).div_ceil(bits);
        let bbitmap_blocks = blocks.div_ceil(bits);
        let itable_blocks = inodes / INODES_PER_BLOCK;
        let ibitmap_start = SUPERBLOCK + 1;
        let bbitmap_start = ibitmap_start + ibitmap_blocks;
        let itable_start = bbitmap_start + bbitmap_blocks;
        let data_start = itable_start + itable_blocks;
        Superblock {
            magic: MAGIC,
            version: VERSION,
            block_size: BLOCK_SIZE as u32,
            blocks,
            inodes,
            ibitmap_start,
            ibitmap_blocks,
            bbitmap_start,
            bbitmap_blocks,
            itable_start,
            itable_blocks,
            data_start,
            free_blocks: blocks.saturating_sub(data_start),
            free_inodes: inodes,
            root_ino: ROOT_INO,
            mtime: 0,
            clean: 1,
            label: [0; 16],
        }
    }

    pub fn decode(b: &[u8]) -> Superblock {
        let mut label = [0u8; 16];
        label.copy_from_slice(&b[68..84]);
        Superblock {
            magic: get32(b, 0),
            version: get32(b, 4),
            block_size: get32(b, 8),
            blocks: get32(b, 12),
            inodes: get32(b, 16),
            ibitmap_start: get32(b, 20),
            ibitmap_blocks: get32(b, 24),
            bbitmap_start: get32(b, 28),
            bbitmap_blocks: get32(b, 32),
            itable_start: get32(b, 36),
            itable_blocks: get32(b, 40),
            data_start: get32(b, 44),
            free_blocks: get32(b, 48),
            free_inodes: get32(b, 52),
            root_ino: get32(b, 56),
            mtime: get32(b, 60),
            clean: get32(b, 64),
            label,
        }
    }

    pub fn encode(&self, b: &mut [u8]) {
        for (i, v) in [
            self.magic,
            self.version,
            self.block_size,
            self.blocks,
            self.inodes,
            self.ibitmap_start,
            self.ibitmap_blocks,
            self.bbitmap_start,
            self.bbitmap_blocks,
            self.itable_start,
            self.itable_blocks,
            self.data_start,
            self.free_blocks,
            self.free_inodes,
            self.root_ino,
            self.mtime,
            self.clean,
        ]
        .iter()
        .enumerate()
        {
            put32(b, i * 4, *v);
        }
        b[68..84].copy_from_slice(&self.label);
    }

    pub fn valid(&self) -> bool {
        self.magic == MAGIC && self.version == VERSION && self.block_size == BLOCK_SIZE as u32
    }

    /// Block holding inode `ino` and the byte offset inside it.
    pub fn inode_pos(&self, ino: u32) -> (u32, usize) {
        let i = ino - 1;
        (self.itable_start + i / INODES_PER_BLOCK, (i % INODES_PER_BLOCK) as usize * INODE_SIZE)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskInode {
    pub mode: u16,
    pub nlink: u16,
    pub uid: u16,
    pub gid: u16,
    pub size: u32,
    pub atime: u32,
    pub mtime: u32,
    pub ctime: u32,
    pub direct: [u32; NDIRECT],
    pub indirect: u32,
    pub dindirect: u32,
    pub flags: u32,
    /// Device number (major << 8 | minor) for device nodes.
    pub rdev: u32,
}

impl DiskInode {
    pub fn decode(b: &[u8]) -> DiskInode {
        let mut direct = [0u32; NDIRECT];
        for (i, d) in direct.iter_mut().enumerate() {
            *d = get32(b, 24 + i * 4);
        }
        DiskInode {
            mode: get16(b, 0),
            nlink: get16(b, 2),
            uid: get16(b, 4),
            gid: get16(b, 6),
            size: get32(b, 8),
            atime: get32(b, 12),
            mtime: get32(b, 16),
            ctime: get32(b, 20),
            direct,
            indirect: get32(b, 72),
            dindirect: get32(b, 76),
            flags: get32(b, 84),
            rdev: get32(b, 88),
        }
    }

    pub fn encode(&self, b: &mut [u8]) {
        b[..INODE_SIZE].fill(0);
        put16(b, 0, self.mode);
        put16(b, 2, self.nlink);
        put16(b, 4, self.uid);
        put16(b, 6, self.gid);
        put32(b, 8, self.size);
        put32(b, 12, self.atime);
        put32(b, 16, self.mtime);
        put32(b, 20, self.ctime);
        for (i, d) in self.direct.iter().enumerate() {
            put32(b, 24 + i * 4, *d);
        }
        put32(b, 72, self.indirect);
        put32(b, 76, self.dindirect);
        put32(b, 84, self.flags);
        put32(b, 88, self.rdev);
    }

    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
}

/// Directory entry helpers.
pub fn dirent_ino(b: &[u8]) -> u32 {
    get32(b, 0)
}

pub fn dirent_name(b: &[u8]) -> &[u8] {
    let n = &b[4..DIRENT_SIZE];
    &n[..n.iter().position(|&c| c == 0).unwrap_or(n.len())]
}

pub fn dirent_set(b: &mut [u8], ino: u32, name: &[u8]) {
    b[..DIRENT_SIZE].fill(0);
    put32(b, 0, ino);
    let n = name.len().min(NAME_MAX);
    b[4..4 + n].copy_from_slice(&name[..n]);
}

pub fn block_ptr(b: &[u8], i: usize) -> u32 {
    get32(b, i * 4)
}

pub fn set_block_ptr(b: &mut [u8], i: usize, v: u32) {
    put32(b, i * 4, v)
}
