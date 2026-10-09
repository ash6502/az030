//! Build an az030 OS disk image.
//!
//! ```text
//! azmkdisk -o IMAGE --size SIZE [--root DIR] [--manifest FILE]
//!          [--boot BOOT.bin --kernel KERNEL.bin --load ADDR --entry ADDR]
//!          [--cmdline TEXT] [--inodes N] [--label NAME]
//! ```
//!
//! With `--boot`/`--kernel` the image is bootable: LBA 0 holds the AZ30BOOT block the
//! ROM reads (pointing at the bootloader at LBA 1) plus the AZOS fields the bootloader
//! reads, the kernel sits at LBA 64, and the root file system starts at LBA 8192.
//! Without them the whole disk is one file system starting at LBA 0.
//!
//! The file system is filled from `--root` (files keep their executable bit; everything
//! is owned by root) and from a manifest with one entry per line:
//!
//! ```text
//! d PATH MODE UID GID              directory
//! c PATH MODE UID GID MAJOR MINOR  character device
//! b PATH MODE UID GID MAJOR MINOR  block device
//! l PATH TARGET                    symbolic link
//! h PATH EXISTING                  hard link
//! m PATH MODE UID GID              change mode/owner of an existing entry
//! ```

use azfs::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const BOOT_LBA: u32 = 1;
const KERNEL_LBA: u32 = 64;
const ROOT_LBA: u32 = 8192;

struct Fs {
    sb: Superblock,
    img: Vec<u8>,
    now: u32,
    /// Next block to hand out (the builder never frees blocks).
    cursor: u32,
}

impl Fs {
    fn new(blocks: u32, inodes: u32, label: &str, now: u32) -> Fs {
        let mut sb = Superblock::layout(blocks, inodes);
        sb.mtime = now;
        let n = label.len().min(15);
        sb.label[..n].copy_from_slice(&label.as_bytes()[..n]);
        let mut fs = Fs { sb, img: vec![0; blocks as usize * BLOCK_SIZE], now, cursor: sb.data_start };
        for b in 0..fs.sb.data_start {
            fs.set_bit(fs.sb.bbitmap_start, b);
        }
        fs.set_bit(fs.sb.ibitmap_start, 0);
        fs
    }

    fn block(&mut self, b: u32) -> &mut [u8] {
        let o = b as usize * BLOCK_SIZE;
        &mut self.img[o..o + BLOCK_SIZE]
    }

    fn set_bit(&mut self, start: u32, n: u32) {
        let bits = (BLOCK_SIZE * 8) as u32;
        let blk = start + n / bits;
        let byte = (n % bits) as usize / 8;
        self.block(blk)[byte] |= 0x80 >> (n % 8);
    }

    fn alloc_block(&mut self) -> Result<u32, String> {
        if self.cursor >= self.sb.blocks {
            return Err("file system full".into());
        }
        let b = self.cursor;
        self.cursor += 1;
        self.set_bit(self.sb.bbitmap_start, b);
        self.sb.free_blocks -= 1;
        self.block(b).fill(0);
        Ok(b)
    }

    fn bit(&self, start: u32, n: u32) -> bool {
        let bits = (BLOCK_SIZE * 8) as u32;
        let blk = start + n / bits;
        let byte = (n % bits) as usize / 8;
        self.img[blk as usize * BLOCK_SIZE + byte] & (0x80 >> (n % 8)) != 0
    }

    fn alloc_inode(&mut self) -> Result<u32, String> {
        let ino = (1..=self.sb.inodes).find(|&i| !self.bit(self.sb.ibitmap_start, i)).ok_or("out of inodes")?;
        self.set_bit(self.sb.ibitmap_start, ino);
        self.sb.free_inodes -= 1;
        Ok(ino)
    }

    fn read_inode(&mut self, ino: u32) -> DiskInode {
        let (b, o) = self.sb.inode_pos(ino);
        DiskInode::decode(&self.block(b)[o..o + INODE_SIZE])
    }

    fn write_inode(&mut self, ino: u32, di: &DiskInode) {
        let (b, o) = self.sb.inode_pos(ino);
        di.encode(&mut self.block(b)[o..o + INODE_SIZE]);
    }

    /// Block number of file block `n`, allocating as needed.
    fn bmap(&mut self, di: &mut DiskInode, n: u32) -> Result<u32, String> {
        let n = n as usize;
        if n < NDIRECT {
            if di.direct[n] == 0 {
                di.direct[n] = self.alloc_block()?;
            }
            return Ok(di.direct[n]);
        }
        let n = n - NDIRECT;
        let ppb = PTRS_PER_BLOCK as usize;
        if n < ppb {
            if di.indirect == 0 {
                di.indirect = self.alloc_block()?;
            }
            return self.slot(di.indirect, n);
        }
        let n = n - ppb;
        if n >= ppb * ppb {
            return Err("file too large".into());
        }
        if di.dindirect == 0 {
            di.dindirect = self.alloc_block()?;
        }
        let mid = self.slot(di.dindirect, n / ppb)?;
        self.slot(mid, n % ppb)
    }

    fn slot(&mut self, table: u32, i: usize) -> Result<u32, String> {
        let v = block_ptr(self.block(table), i);
        if v != 0 {
            return Ok(v);
        }
        let nb = self.alloc_block()?;
        set_block_ptr(self.block(table), i, nb);
        Ok(nb)
    }

    fn write_data(&mut self, ino: u32, data: &[u8]) -> Result<(), String> {
        let mut di = self.read_inode(ino);
        for (i, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
            let b = self.bmap(&mut di, i as u32)?;
            self.block(b)[..chunk.len()].copy_from_slice(chunk);
        }
        di.size = data.len() as u32;
        self.write_inode(ino, &di);
        Ok(())
    }

    fn new_inode(&mut self, mode: u16, uid: u16, gid: u16) -> Result<u32, String> {
        let ino = self.alloc_inode()?;
        let di = DiskInode { mode, nlink: 1, uid, gid, atime: self.now, mtime: self.now, ctime: self.now, ..Default::default() };
        self.write_inode(ino, &di);
        Ok(ino)
    }

    fn dir_entries(&mut self, dir: u32) -> Vec<(String, u32)> {
        let di = self.read_inode(dir);
        let mut out = Vec::new();
        let mut d2 = di;
        for i in 0..di.size.div_ceil(BLOCK_SIZE as u32) {
            let b = self.bmap(&mut d2, i).unwrap();
            let blk = self.block(b).to_vec();
            for e in blk.chunks(DIRENT_SIZE) {
                let ino = dirent_ino(e);
                if ino != 0 {
                    out.push((String::from_utf8_lossy(dirent_name(e)).into_owned(), ino));
                }
            }
        }
        out
    }

    fn add_dirent(&mut self, dir: u32, name: &str, ino: u32) -> Result<(), String> {
        if name.len() > NAME_MAX {
            return Err(format!("name too long: {name}"));
        }
        let mut di = self.read_inode(dir);
        let mut ent = [0u8; DIRENT_SIZE];
        dirent_set(&mut ent, ino, name.as_bytes());
        // find a free slot
        let nblocks = di.size.div_ceil(BLOCK_SIZE as u32);
        for i in 0..nblocks {
            let b = self.bmap(&mut di, i)?;
            let blk = self.block(b);
            for s in 0..DIRENTS_PER_BLOCK {
                let o = s * DIRENT_SIZE;
                if dirent_ino(&blk[o..]) == 0 && (i * BLOCK_SIZE as u32 + o as u32) < di.size {
                    blk[o..o + DIRENT_SIZE].copy_from_slice(&ent);
                    self.write_inode(dir, &di);
                    return Ok(());
                }
            }
        }
        let off = di.size;
        let b = self.bmap(&mut di, off / BLOCK_SIZE as u32)?;
        let o = (off % BLOCK_SIZE as u32) as usize;
        self.block(b)[o..o + DIRENT_SIZE].copy_from_slice(&ent);
        di.size += DIRENT_SIZE as u32;
        self.write_inode(dir, &di);
        Ok(())
    }

    fn mkdir(&mut self, parent: u32, name: &str, mode: u16, uid: u16, gid: u16) -> Result<u32, String> {
        let ino = self.new_inode(S_IFDIR | (mode & 0o7777), uid, gid)?;
        let mut di = self.read_inode(ino);
        di.nlink = 2;
        self.write_inode(ino, &di);
        self.add_dirent(ino, ".", ino)?;
        self.add_dirent(ino, "..", parent)?;
        if parent != ino {
            self.add_dirent(parent, name, ino)?;
            let mut p = self.read_inode(parent);
            p.nlink += 1;
            self.write_inode(parent, &p);
        }
        Ok(ino)
    }

    fn lookup(&mut self, path: &str) -> Option<u32> {
        let mut cur = ROOT_INO;
        for comp in path.split('/').filter(|c| !c.is_empty()) {
            cur = self.dir_entries(cur).into_iter().find(|(n, _)| n == comp)?.1;
        }
        Some(cur)
    }

    /// Parent directory inode and final name; creates missing parents.
    fn parent_of(&mut self, path: &str) -> Result<(u32, String), String> {
        let path = path.trim_end_matches('/');
        let (dir, name) = path.rsplit_once('/').ok_or_else(|| format!("bad path {path}"))?;
        let mut cur = ROOT_INO;
        for comp in dir.split('/').filter(|c| !c.is_empty()) {
            cur = match self.dir_entries(cur).into_iter().find(|(n, _)| n == comp) {
                Some((_, i)) => i,
                None => self.mkdir(cur, comp, 0o755, 0, 0)?,
            };
        }
        Ok((cur, name.to_string()))
    }

    fn add_tree(&mut self, host: &Path, dir: u32) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(host).map_err(|e| format!("{}: {e}", host.display()))?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".DS_Store" {
                continue;
            }
            let meta = std::fs::symlink_metadata(e.path()).map_err(|x| x.to_string())?;
            use std::os::unix::fs::PermissionsExt;
            let perm = meta.permissions().mode() as u16;
            if meta.file_type().is_symlink() {
                let t = std::fs::read_link(e.path()).map_err(|x| x.to_string())?;
                let ino = self.new_inode(S_IFLNK | 0o777, 0, 0)?;
                self.write_data(ino, t.to_string_lossy().as_bytes())?;
                self.add_dirent(dir, &name, ino)?;
            } else if meta.is_dir() {
                let sub = self.mkdir(dir, &name, 0o755, 0, 0)?;
                self.add_tree(&e.path(), sub)?;
            } else {
                let mode = if perm & 0o111 != 0 { 0o755 } else { 0o644 };
                let ino = self.new_inode(S_IFREG | mode, 0, 0)?;
                let data = std::fs::read(e.path()).map_err(|x| x.to_string())?;
                self.write_data(ino, &data)?;
                self.add_dirent(dir, &name, ino)?;
            }
        }
        Ok(())
    }

    fn manifest(&mut self, text: &str) -> Result<(), String> {
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            let err = |m: &str| format!("manifest line {}: {m}", n + 1);
            let num = |s: &str, radix| u32::from_str_radix(s, radix).map_err(|_| err("bad number"));
            match f.as_slice() {
                ["d", path, mode, uid, gid] => {
                    let mode = num(mode, 8)? as u16;
                    if let Some(ino) = self.lookup(path) {
                        let mut di = self.read_inode(ino);
                        di.mode = S_IFDIR | mode;
                        self.write_inode(ino, &di);
                    } else {
                        let (p, name) = self.parent_of(path)?;
                        self.mkdir(p, &name, mode, num(uid, 10)? as u16, num(gid, 10)? as u16)?;
                    }
                }
                [t @ ("c" | "b"), path, mode, uid, gid, major, minor] => {
                    let kind = if *t == "c" { S_IFCHR } else { S_IFBLK };
                    let (p, name) = self.parent_of(path)?;
                    let ino = self.new_inode(kind | num(mode, 8)? as u16, num(uid, 10)? as u16, num(gid, 10)? as u16)?;
                    let mut di = self.read_inode(ino);
                    di.rdev = num(major, 10)? << 8 | num(minor, 10)?;
                    self.write_inode(ino, &di);
                    self.add_dirent(p, &name, ino)?;
                }
                ["l", path, target] => {
                    let (p, name) = self.parent_of(path)?;
                    let ino = self.new_inode(S_IFLNK | 0o777, 0, 0)?;
                    self.write_data(ino, target.as_bytes())?;
                    self.add_dirent(p, &name, ino)?;
                }
                ["h", path, existing] => {
                    let ino = self.lookup(existing).ok_or_else(|| err("link target missing"))?;
                    let (p, name) = self.parent_of(path)?;
                    self.add_dirent(p, &name, ino)?;
                    let mut di = self.read_inode(ino);
                    di.nlink += 1;
                    self.write_inode(ino, &di);
                }
                ["m", path, mode, uid, gid] => {
                    let ino = self.lookup(path).ok_or_else(|| err("no such file"))?;
                    let mut di = self.read_inode(ino);
                    di.mode = (di.mode & S_IFMT) | num(mode, 8)? as u16;
                    di.uid = num(uid, 10)? as u16;
                    di.gid = num(gid, 10)? as u16;
                    self.write_inode(ino, &di);
                }
                _ => return Err(err("unrecognised entry")),
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        let sb = self.sb;
        sb.encode(self.block(SUPERBLOCK));
        self.img
    }
}

fn parse_num(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (n, mult) = match s.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&s[..s.len() - 1], 1 << 10),
        Some('M') => (&s[..s.len() - 1], 1 << 20),
        Some('G') => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let v = match n.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => n.parse(),
    };
    v.map(|v| v * mult).map_err(|_| format!("bad number {s:?}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("azmkdisk: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut a: BTreeMap<String, String> = BTreeMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k} needs a value"))?;
        a.insert(k.trim_start_matches('-').to_string(), v);
    }
    let out = PathBuf::from(a.get("o").ok_or("-o IMAGE is required")?);
    let size = parse_num(a.get("size").ok_or("--size is required")?)?;
    if size % 1024 != 0 || size < 1 << 20 {
        return Err("size must be a multiple of 1K and at least 1M".into());
    }
    let sectors = (size / 512) as u32;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
    let bootable = a.contains_key("kernel");
    let fs_lba = if bootable { ROOT_LBA } else { 0 };
    if sectors <= fs_lba + 2048 {
        return Err("disk too small".into());
    }
    let fs_blocks = (sectors - fs_lba) / SECTORS_PER_BLOCK;
    let inodes = match a.get("inodes") {
        Some(n) => parse_num(n)? as u32,
        None => (fs_blocks / 8).clamp(256, 65536),
    };
    let label = a.get("label").map(String::as_str).unwrap_or(if bootable { "root" } else { "data" });
    let mut fs = Fs::new(fs_blocks, inodes, label, now);
    fs.mkdir(ROOT_INO, "", 0o755, 0, 0)?;
    if let Some(root) = a.get("root") {
        fs.add_tree(Path::new(root), ROOT_INO)?;
    }
    if let Some(m) = a.get("manifest") {
        let text = std::fs::read_to_string(m).map_err(|e| format!("{m}: {e}"))?;
        fs.manifest(&text)?;
    }
    let free = fs.sb.free_blocks;
    let fsimg = fs.finish();

    let mut disk = vec![0u8; size as usize];
    disk[fs_lba as usize * 512..fs_lba as usize * 512 + fsimg.len()].copy_from_slice(&fsimg);
    if bootable {
        let boot_path = a.get("boot").ok_or("--boot is required with --kernel")?;
        let boot = std::fs::read(boot_path).map_err(|e| format!("{boot_path}: {e}"))?;
        let kpath = &a["kernel"];
        let kernel = std::fs::read(kpath).map_err(|e| format!("{kpath}: {e}"))?;
        let load = parse_num(a.get("load").ok_or("--load is required")?)? as u32;
        let entry = a.get("entry").map(|e| parse_num(e)).transpose()?.map_or(load, |e| e as u32);
        let boot_blocks = boot.len().div_ceil(512) as u32;
        let kblocks = kernel.len().div_ceil(512) as u32;
        if BOOT_LBA + boot_blocks > KERNEL_LBA {
            return Err("bootloader too large".into());
        }
        if KERNEL_LBA + kblocks > ROOT_LBA {
            return Err(format!("kernel too large ({} bytes)", kernel.len()));
        }
        let bb = &mut disk[..512];
        bb[0..8].copy_from_slice(b"AZ30BOOT");
        bb[8..12].copy_from_slice(&0x8000u32.to_be_bytes());
        bb[12..16].copy_from_slice(&0x8000u32.to_be_bytes());
        bb[16..20].copy_from_slice(&boot_blocks.to_be_bytes());
        bb[20..24].copy_from_slice(&(boot.len() as u32).to_be_bytes());
        bb[0x20..0x24].copy_from_slice(b"AZOS");
        for (o, v) in [(0x24, KERNEL_LBA), (0x28, kblocks), (0x2C, load), (0x30, entry), (0x34, ROOT_LBA), (0x38, sectors - ROOT_LBA), (0x3C, kernel.len() as u32)] {
            bb[o..o + 4].copy_from_slice(&v.to_be_bytes());
        }
        let cmd = a.get("cmdline").map(String::as_str).unwrap_or("");
        let n = cmd.len().min(127);
        bb[0x40..0x40 + n].copy_from_slice(&cmd.as_bytes()[..n]);
        disk[512..512 + boot.len()].copy_from_slice(&boot);
        let k = KERNEL_LBA as usize * 512;
        disk[k..k + kernel.len()].copy_from_slice(&kernel);
    }
    std::fs::write(&out, &disk).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "{}: {} MB, file system {} KB ({} KB free){}",
        out.display(),
        size >> 20,
        fs_blocks,
        free,
        if bootable { ", bootable" } else { "" }
    );
    Ok(())
}
