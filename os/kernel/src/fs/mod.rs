//! File-system layer: the inode cache, the mount table, path name lookup and the
//! operations the system calls use. azfs (disk.rs) is the only file-system type.

pub mod disk;

use crate::bio::{self, BlockDev};
use crate::proc;
use crate::timer;
use crate::util::Global;
use crate::vm::KResult;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use azfs::{DiskInode, Superblock, S_IFDIR, S_IFLNK, S_IFMT};
use azsys::errno::*;
use core::cell::RefCell;

pub type InodeRef = Rc<RefCell<Inode>>;

pub struct Inode {
    pub dev: u32,
    pub ino: u32,
    pub di: DiskInode,
    pub dirty: bool,
    /// Device mounted on this directory, if any.
    pub mounted: Option<u32>,
}

impl Inode {
    pub fn mode(&self) -> u32 {
        self.di.mode as u32
    }
    pub fn kind(&self) -> u16 {
        self.di.mode & S_IFMT
    }
    pub fn is_dir(&self) -> bool {
        self.kind() == S_IFDIR
    }
    pub fn is_symlink(&self) -> bool {
        self.kind() == S_IFLNK
    }
    pub fn write_back(&mut self) -> KResult<()> {
        if self.dirty {
            disk::write_dinode(self.dev, self.ino, &self.di)?;
            self.dirty = false;
        }
        Ok(())
    }
}

impl Drop for Inode {
    fn drop(&mut self) {
        ICACHE.get().remove(&(self.dev, self.ino));
        if mount_mut(self.dev).is_err() {
            return;
        }
        if self.di.nlink == 0 {
            // last reference to an unlinked file: free it
            let _ = disk::truncate(self, 0);
            let _ = disk::free_inode(self.dev, self.ino);
        } else {
            let _ = self.write_back();
        }
    }
}

pub struct Mount {
    pub dev: u32,
    pub sb: Superblock,
    pub sb_dirty: bool,
    pub block_hint: u32,
    pub read_only: bool,
    pub root: Option<InodeRef>,
    /// The directory this file system is mounted on (None for /).
    pub covered: Option<InodeRef>,
    pub path: String,
    pub source: String,
}

static ICACHE: Global<BTreeMap<(u32, u32), Weak<RefCell<Inode>>>> = Global::new(BTreeMap::new());
static MOUNTS: Global<Vec<Mount>> = Global::new(Vec::new());

pub fn mount_mut(dev: u32) -> KResult<&'static mut Mount> {
    MOUNTS.get().iter_mut().find(|m| m.dev == dev).ok_or(ENODEV)
}

pub fn mounts() -> &'static [Mount] {
    MOUNTS.get()
}

pub fn iget(dev: u32, ino: u32) -> KResult<InodeRef> {
    if let Some(w) = ICACHE.get().get(&(dev, ino)) {
        if let Some(r) = w.upgrade() {
            return Ok(r);
        }
    }
    let di = disk::read_dinode(dev, ino)?;
    let r = Rc::new(RefCell::new(Inode { dev, ino, di, dirty: false, mounted: None }));
    ICACHE.get().insert((dev, ino), Rc::downgrade(&r));
    Ok(r)
}

pub fn root_inode() -> InodeRef {
    MOUNTS.get()[0].root.clone().unwrap()
}

/// Where the file system on a SCSI disk starts: after the boot area on an AZOS boot
/// disk, at LBA 0 otherwise. Returns (first LBA, length in 512-byte blocks).
pub fn fs_location(scsi_id: u32) -> KResult<(u32, u32)> {
    let disk = crate::scsi::disk(scsi_id).ok_or(ENXIO)?;
    let mut b = alloc::vec![0u8; 512];
    crate::scsi::read(scsi_id, 0, 1, &mut b).map_err(|_| EIO)?;
    if &b[0x20..0x24] == b"AZOS" {
        let lba = u32::from_be_bytes(b[0x34..0x38].try_into().unwrap());
        let n = u32::from_be_bytes(b[0x38..0x3C].try_into().unwrap());
        if lba < disk.blocks {
            return Ok((lba, n.min(disk.blocks - lba)));
        }
    }
    Ok((0, disk.blocks))
}

fn attach(dev: u32, lba: u32, sectors: u32, read_only: bool) -> KResult<Superblock> {
    let id = dev & 0xFF;
    let disk = crate::scsi::disk(id).ok_or(ENXIO)?;
    bio::register(dev, BlockDev { scsi_id: id, lba, blocks: sectors / 2, read_only: read_only || disk.read_only });
    let sb = match bio::read(dev, azfs::SUPERBLOCK) {
        Ok(b) => Superblock::decode(b.data()),
        Err(e) => {
            bio::unregister(dev);
            return Err(e);
        }
    };
    if !sb.valid() || sb.blocks > sectors / 2 {
        bio::unregister(dev);
        return Err(EINVAL);
    }
    Ok(sb)
}

fn add_mount(dev: u32, mut sb: Superblock, ro: bool, covered: Option<InodeRef>, path: &str, source: &str) -> KResult<()> {
    if sb.clean == 0 {
        kprintln!("fs: {} was not cleanly unmounted", source);
    }
    let ro = ro || bio::device(dev).is_some_and(|d| d.read_only);
    if !ro {
        sb.clean = 0;
        sb.mtime = timer::now();
    }
    MOUNTS.get().push(Mount {
        dev,
        sb,
        sb_dirty: !ro,
        block_hint: sb.data_start,
        read_only: ro,
        root: None,
        covered: covered.clone(),
        path: path.into(),
        source: source.into(),
    });
    let root = match iget(dev, sb.root_ino) {
        Ok(r) => r,
        Err(e) => {
            MOUNTS.get().pop();
            return Err(e);
        }
    };
    mount_mut(dev)?.root = Some(root);
    if let Some(c) = covered {
        c.borrow_mut().mounted = Some(dev);
    }
    write_super(dev)
}

fn write_super(dev: u32) -> KResult<()> {
    let m = mount_mut(dev)?;
    if m.sb_dirty && !m.read_only {
        let sb = m.sb;
        let b = bio::read(dev, azfs::SUPERBLOCK)?;
        sb.encode(b.data_mut());
        m.sb_dirty = false;
    }
    Ok(())
}

/// Mount the root file system.
pub fn init(boot_lba: u32, boot_sectors: u32) {
    let cfg = crate::config();
    let dev = cfg.root_dev;
    let id = dev & 0xFF;
    let (lba, sectors) = if id == cfg.boot_id {
        (boot_lba, boot_sectors)
    } else {
        fs_location(id).unwrap_or((boot_lba, boot_sectors))
    };
    let sb = attach(dev, lba, sectors, false).unwrap_or_else(|e| panic!("cannot read root file system on sd{}: {}", id, message(e)));
    add_mount(dev, sb, false, None, "/", &alloc::format!("/dev/sd{}", id)).unwrap_or_else(|e| panic!("cannot mount root: {}", message(e)));
    let s = &mount_mut(dev).unwrap().sb;
    kprintln!("fs: root on sd{} ({} KB, {} KB free)", id, s.blocks, s.free_blocks);
}

/// Write back inodes, superblocks and dirty buffers.
pub fn sync() {
    let live: Vec<InodeRef> = ICACHE.get().values().filter_map(|w| w.upgrade()).collect();
    for ip in live {
        if let Ok(mut i) = ip.try_borrow_mut() {
            let _ = i.write_back();
        }
    }
    let devs: Vec<u32> = MOUNTS.get().iter().map(|m| m.dev).collect();
    for d in devs {
        let _ = write_super(d);
    }
    bio::sync();
}

/// Mark every file system clean and flush (shutdown).
pub fn shutdown() {
    sync();
    for m in MOUNTS.get().iter_mut() {
        if !m.read_only {
            m.sb.clean = 1;
            m.sb_dirty = true;
        }
    }
    let devs: Vec<u32> = MOUNTS.get().iter().map(|m| m.dev).collect();
    for d in devs {
        let _ = write_super(d);
    }
    bio::sync();
}

pub fn mount(source: &[u8], target: &[u8], flags: u32) -> KResult<()> {
    require_root()?;
    let dev_ip = lookup(source, true)?;
    let (kind, rdev) = {
        let d = dev_ip.borrow();
        (d.kind(), d.di.rdev)
    };
    if kind != azfs::S_IFBLK || rdev >> 8 != azsys::dev::SD_MAJOR {
        return Err(ENOTBLK);
    }
    if MOUNTS.get().iter().any(|m| m.dev == rdev) {
        return Err(EBUSY);
    }
    let dir = lookup(target, true)?;
    {
        let d = dir.borrow();
        if !d.is_dir() {
            return Err(ENOTDIR);
        }
        if d.mounted.is_some() || MOUNTS.get().iter().any(|m| m.root.as_ref().is_some_and(|r| Rc::ptr_eq(r, &dir))) {
            return Err(EBUSY);
        }
    }
    let (lba, sectors) = fs_location(rdev & 0xFF)?;
    let ro = flags & azsys::flags::MS_RDONLY != 0;
    let sb = attach(rdev, lba, sectors, ro)?;
    let path = String::from_utf8_lossy(&absolute(target)).into_owned();
    add_mount(rdev, sb, ro, Some(dir), &path, &String::from_utf8_lossy(source))
}

pub fn umount(target: &[u8]) -> KResult<()> {
    require_root()?;
    let ip = lookup(target, true)?;
    let dev = ip.borrow().dev;
    let idx = MOUNTS.get().iter().position(|m| m.dev == dev && m.root.as_ref().is_some_and(|r| Rc::ptr_eq(r, &ip))).ok_or(EINVAL)?;
    if idx == 0 {
        return Err(EBUSY);
    }
    drop(ip);
    // busy if anything besides the mount's own root reference is in use
    sync();
    let root_ino = MOUNTS.get()[idx].sb.root_ino;
    let busy = ICACHE.get().iter().any(|((d, i), w)| *d == dev && *i != root_ino && w.strong_count() > 0);
    if busy || MOUNTS.get()[idx].root.as_ref().is_some_and(|r| Rc::strong_count(r) > 1) {
        return Err(EBUSY);
    }
    let m = &mut MOUNTS.get()[idx];
    if !m.read_only {
        m.sb.clean = 1;
        m.sb_dirty = true;
    }
    write_super(dev)?;
    let m = MOUNTS.get().remove(idx);
    if let Some(c) = &m.covered {
        c.borrow_mut().mounted = None;
    }
    drop(m);
    bio::unregister(dev);
    Ok(())
}

fn require_root() -> KResult<()> {
    if proc::current().euid == 0 { Ok(()) } else { Err(EPERM) }
}

/// Can the current process access `ip` with `want` (4 read, 2 write, 1 exec)?
pub fn permitted(ip: &Inode, want: u32) -> bool {
    let p = proc::current();
    let mode = ip.mode();
    if p.euid == 0 {
        // root may do anything except execute files nobody can execute
        return want & 1 == 0 || ip.is_dir() || mode & 0o111 != 0;
    }
    let bits = if p.euid == ip.di.uid as u32 {
        mode >> 6
    } else if p.egid == ip.di.gid as u32 {
        mode >> 3
    } else {
        mode
    } & 7;
    bits & want == want
}

fn check(ip: &InodeRef, want: u32) -> KResult<()> {
    if permitted(&ip.borrow(), want) { Ok(()) } else { Err(EACCES) }
}

fn read_only(ip: &InodeRef) -> KResult<()> {
    let dev = ip.borrow().dev;
    if mount_mut(dev)?.read_only { Err(EROFS) } else { Ok(()) }
}

/// Path made absolute against the current directory (no symlink resolution).
fn absolute(path: &[u8]) -> Vec<u8> {
    if path.starts_with(b"/") {
        return path.to_vec();
    }
    let mut v = getcwd().unwrap_or_else(|_| b"/".to_vec());
    if !v.ends_with(b"/") {
        v.push(b'/');
    }
    v.extend_from_slice(path);
    v
}

fn components(path: &[u8]) -> VecDeque<Vec<u8>> {
    path.split(|&c| c == b'/').filter(|c| !c.is_empty()).map(|c| c.to_vec()).collect()
}

/// Parent of a directory, crossing mount points upwards.
fn parent_dir(cur: &InodeRef) -> KResult<InodeRef> {
    let (dev, ino) = {
        let c = cur.borrow();
        (c.dev, c.ino)
    };
    let mut cur = cur.clone();
    if let Ok(m) = mount_mut(dev) {
        if m.root.as_ref().is_some_and(|r| r.borrow().ino == ino) {
            match &m.covered {
                None => return Ok(cur), // "/.." is "/"
                Some(c) => cur = c.clone(),
            }
        }
    }
    let (pino, pdev) = {
        let mut c = cur.borrow_mut();
        let dev = c.dev;
        (disk::dir_lookup(&mut c, b"..")?.ok_or(ENOENT)?.0, dev)
    };
    iget(pdev, pino)
}

fn into_mounted(ip: InodeRef) -> KResult<InodeRef> {
    let m = ip.borrow().mounted;
    match m {
        Some(dev) => Ok(mount_mut(dev)?.root.clone().unwrap()),
        None => Ok(ip),
    }
}

pub fn read_link(ip: &InodeRef) -> KResult<Vec<u8>> {
    let mut i = ip.borrow_mut();
    let mut v = alloc::vec![0u8; i.di.size.min(4096) as usize];
    let n = disk::read(&mut i, 0, &mut v)?;
    v.truncate(n);
    Ok(v)
}

/// Resolve a path. With `parent`, stop before the last component and return it.
fn resolve(path: &[u8], follow_last: bool, parent: bool) -> KResult<(InodeRef, Option<Vec<u8>>)> {
    if path.is_empty() {
        return Err(ENOENT);
    }
    if path.len() > 1024 {
        return Err(ENAMETOOLONG);
    }
    let p = proc::current();
    let mut cur = if path.starts_with(b"/") { root_inode() } else { p.cwd.clone().unwrap_or_else(root_inode) };
    let mut comps = components(path);
    let mut links = 0;
    if parent && comps.is_empty() {
        return Err(EBUSY); // "/" has no parent entry
    }
    while let Some(c) = comps.pop_front() {
        let last = comps.is_empty();
        if parent && last {
            if !cur.borrow().is_dir() {
                return Err(ENOTDIR);
            }
            return Ok((cur, Some(c)));
        }
        if c == b"." {
            continue;
        }
        if !cur.borrow().is_dir() {
            return Err(ENOTDIR);
        }
        check(&cur, 1)?;
        if c == b".." {
            cur = parent_dir(&cur)?;
            continue;
        }
        if c.len() > azfs::NAME_MAX {
            return Err(ENAMETOOLONG);
        }
        let (dev, child) = {
            let mut d = cur.borrow_mut();
            let dev = d.dev;
            (dev, disk::dir_lookup(&mut d, &c)?.ok_or(ENOENT)?.0)
        };
        let child = into_mounted(iget(dev, child)?)?;
        if child.borrow().is_symlink() && (!last || follow_last) {
            links += 1;
            if links > 8 {
                return Err(ELOOP);
            }
            let target = read_link(&child)?;
            if target.starts_with(b"/") {
                cur = root_inode();
            }
            let mut t = components(&target);
            while let Some(x) = t.pop_back() {
                comps.push_front(x);
            }
            if parent && comps.is_empty() {
                return Err(ENOENT);
            }
            continue;
        }
        cur = child;
    }
    Ok((cur, None))
}

pub fn lookup(path: &[u8], follow: bool) -> KResult<InodeRef> {
    resolve(path, follow, false).map(|r| r.0)
}

/// The directory that would contain `path`, and the final name.
pub fn lookup_parent(path: &[u8]) -> KResult<(InodeRef, Vec<u8>)> {
    let (d, n) = resolve(path, true, true)?;
    Ok((d, n.unwrap()))
}

fn dir_find(dir: &InodeRef, name: &[u8]) -> KResult<Option<(u32, u32)>> {
    disk::dir_lookup(&mut dir.borrow_mut(), name)
}

/// Create a file system object. Returns the existing inode if it is already there
/// (unless `excl`).
pub fn create(path: &[u8], mode: u32, rdev: u32, excl: bool) -> KResult<InodeRef> {
    let (dir, name) = lookup_parent(path)?;
    if name == b"." || name == b".." {
        return Err(EEXIST);
    }
    if let Some((ino, _)) = dir_find(&dir, &name)? {
        if excl {
            return Err(EEXIST);
        }
        let dev = dir.borrow().dev;
        let ip = into_mounted(iget(dev, ino)?)?;
        if ip.borrow().is_symlink() {
            return lookup(path, true);
        }
        return Ok(ip);
    }
    read_only(&dir)?;
    check(&dir, 2 | 1)?;
    let p = proc::current();
    let kind = mode & azsys::mode::S_IFMT;
    let perm = mode & 0o7777 & !p.umask;
    let dev = dir.borrow().dev;
    let ino = disk::alloc_inode(dev, (kind | perm) as u16, p.euid as u16, p.egid as u16)?;
    let ip = iget(dev, ino)?;
    if kind == azsys::mode::S_IFDIR as u32 {
        let pino = dir.borrow().ino;
        let r = (|| -> KResult<()> {
            let mut i = ip.borrow_mut();
            disk::dir_add(&mut i, b".", ino)?;
            disk::dir_add(&mut i, b"..", pino)?;
            i.di.nlink = 2;
            i.dirty = true;
            Ok(())
        })();
        if let Err(e) = r {
            ip.borrow_mut().di.nlink = 0;
            return Err(e);
        }
    }
    {
        let mut i = ip.borrow_mut();
        i.di.rdev = rdev;
        i.dirty = true;
    }
    let mut d = dir.borrow_mut();
    if let Err(e) = disk::dir_add(&mut d, &name, ino) {
        ip.borrow_mut().di.nlink = 0;
        return Err(e);
    }
    if kind == azsys::mode::S_IFDIR as u32 {
        d.di.nlink += 1;
    }
    d.di.mtime = timer::now();
    d.dirty = true;
    Ok(ip)
}

pub fn unlink(path: &[u8]) -> KResult<()> {
    let (dir, name) = lookup_parent(path)?;
    read_only(&dir)?;
    check(&dir, 2 | 1)?;
    let (ino, off) = dir_find(&dir, &name)?.ok_or(ENOENT)?;
    let dev = dir.borrow().dev;
    let ip = iget(dev, ino)?;
    if ip.borrow().is_dir() {
        return Err(EISDIR);
    }
    sticky_ok(&dir, &ip)?;
    disk::dir_set(&mut dir.borrow_mut(), off, 0, b"")?;
    let mut i = ip.borrow_mut();
    i.di.nlink = i.di.nlink.saturating_sub(1);
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

/// In a sticky directory only the owner (or root) may remove an entry.
fn sticky_ok(dir: &InodeRef, ip: &InodeRef) -> KResult<()> {
    let p = proc::current();
    let d = dir.borrow();
    if d.mode() & azsys::mode::S_ISVTX != 0 && p.euid != 0 && ip.borrow().di.uid as u32 != p.euid && d.di.uid as u32 != p.euid {
        return Err(EPERM);
    }
    Ok(())
}

pub fn rmdir(path: &[u8]) -> KResult<()> {
    let (dir, name) = lookup_parent(path)?;
    if name == b"." {
        return Err(EINVAL);
    }
    if name == b".." {
        return Err(ENOTEMPTY);
    }
    read_only(&dir)?;
    check(&dir, 2 | 1)?;
    let (ino, off) = dir_find(&dir, &name)?.ok_or(ENOENT)?;
    let dev = dir.borrow().dev;
    let ip = iget(dev, ino)?;
    {
        let mut i = ip.borrow_mut();
        if !i.is_dir() {
            return Err(ENOTDIR);
        }
        if i.mounted.is_some() {
            return Err(EBUSY);
        }
        if !disk::dir_empty(&mut i)? {
            return Err(ENOTEMPTY);
        }
    }
    if proc::current().cwd.as_ref().is_some_and(|c| Rc::ptr_eq(c, &ip)) {
        // allowed in UNIX, but keeps things simple here
        return Err(EBUSY);
    }
    sticky_ok(&dir, &ip)?;
    let mut d = dir.borrow_mut();
    disk::dir_set(&mut d, off, 0, b"")?;
    d.di.nlink = d.di.nlink.saturating_sub(1);
    d.dirty = true;
    let mut i = ip.borrow_mut();
    i.di.nlink = 0;
    i.dirty = true;
    Ok(())
}

pub fn link(old: &[u8], new: &[u8]) -> KResult<()> {
    let ip = lookup(old, false)?;
    if ip.borrow().is_dir() {
        return Err(EPERM);
    }
    let (dir, name) = lookup_parent(new)?;
    if dir.borrow().dev != ip.borrow().dev {
        return Err(EXDEV);
    }
    read_only(&dir)?;
    check(&dir, 2 | 1)?;
    if dir_find(&dir, &name)?.is_some() {
        return Err(EEXIST);
    }
    if ip.borrow().di.nlink >= 0x7FFF {
        return Err(EMLINK);
    }
    let ino = ip.borrow().ino;
    disk::dir_add(&mut dir.borrow_mut(), &name, ino)?;
    let mut i = ip.borrow_mut();
    i.di.nlink += 1;
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

pub fn symlink(target: &[u8], path: &[u8]) -> KResult<()> {
    if target.is_empty() || target.len() > 1023 {
        return Err(EINVAL);
    }
    let ip = create(path, (azfs::S_IFLNK as u32) | 0o777, 0, true)?;
    let mut i = ip.borrow_mut();
    i.di.mode = azfs::S_IFLNK | 0o777;
    disk::write(&mut i, 0, target)?;
    Ok(())
}

/// Is `a` the same directory as `b` or an ancestor of it?
fn is_ancestor(a: &InodeRef, b: &InodeRef) -> KResult<bool> {
    let key = |x: &InodeRef| {
        let i = x.borrow();
        (i.dev, i.ino)
    };
    let target = key(a);
    let mut cur = b.clone();
    for _ in 0..256 {
        if key(&cur) == target {
            return Ok(true);
        }
        let up = parent_dir(&cur)?;
        if key(&up) == key(&cur) {
            return Ok(false);
        }
        cur = up;
    }
    Ok(false)
}

pub fn rename(old: &[u8], new: &[u8]) -> KResult<()> {
    let (odir, oname) = lookup_parent(old)?;
    let (ndir, nname) = lookup_parent(new)?;
    if oname == b"." || oname == b".." || nname == b"." || nname == b".." {
        return Err(EINVAL);
    }
    let dev = odir.borrow().dev;
    if ndir.borrow().dev != dev {
        return Err(EXDEV);
    }
    read_only(&odir)?;
    check(&odir, 3)?;
    check(&ndir, 3)?;
    let (ino, ooff) = dir_find(&odir, &oname)?.ok_or(ENOENT)?;
    let ip = iget(dev, ino)?;
    let is_dir = ip.borrow().is_dir();
    if is_dir && is_ancestor(&ip, &ndir)? {
        return Err(EINVAL);
    }
    sticky_ok(&odir, &ip)?;
    if let Some((tino, toff)) = dir_find(&ndir, &nname)? {
        if tino == ino {
            return Ok(());
        }
        let tip = iget(dev, tino)?;
        let tdir = tip.borrow().is_dir();
        if tdir != is_dir {
            return Err(if tdir { EISDIR } else { ENOTDIR });
        }
        if tdir {
            if tip.borrow().mounted.is_some() {
                return Err(EBUSY);
            }
            if !disk::dir_empty(&mut tip.borrow_mut())? {
                return Err(ENOTEMPTY);
            }
        }
        disk::dir_set(&mut ndir.borrow_mut(), toff, ino, &nname)?;
        let mut t = tip.borrow_mut();
        t.di.nlink = if tdir { 0 } else { t.di.nlink.saturating_sub(1) };
        t.dirty = true;
        if tdir {
            let mut n = ndir.borrow_mut();
            n.di.nlink = n.di.nlink.saturating_sub(1);
            n.dirty = true;
        }
    } else {
        disk::dir_add(&mut ndir.borrow_mut(), &nname, ino)?;
    }
    // remove the old name (find it again: adding may have reused a slot)
    let ooff = match dir_find(&odir, &oname)? {
        Some((i, o)) if i == ino => o,
        _ => ooff,
    };
    disk::dir_set(&mut odir.borrow_mut(), ooff, 0, b"")?;
    if is_dir && !Rc::ptr_eq(&odir, &ndir) {
        let pino = ndir.borrow().ino;
        let mut i = ip.borrow_mut();
        if let Some((_, off)) = disk::dir_lookup(&mut i, b"..")? {
            disk::dir_set(&mut i, off, pino, b"..")?;
        }
        drop(i);
        let mut o = odir.borrow_mut();
        o.di.nlink = o.di.nlink.saturating_sub(1);
        o.dirty = true;
        drop(o);
        let mut n = ndir.borrow_mut();
        n.di.nlink += 1;
        n.dirty = true;
    }
    let mut i = ip.borrow_mut();
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

pub fn stat(ip: &InodeRef) -> azsys::Stat {
    let i = ip.borrow();
    azsys::Stat {
        dev: i.dev,
        ino: i.ino,
        mode: i.di.mode as u32,
        nlink: i.di.nlink as u32,
        uid: i.di.uid as u32,
        gid: i.di.gid as u32,
        rdev: i.di.rdev,
        size: i.di.size,
        atime: i.di.atime,
        mtime: i.di.mtime,
        ctime: i.di.ctime,
        blksize: crate::bio::BSIZE as u32,
        blocks: i.di.size.div_ceil(512),
    }
}

pub fn statfs(ip: &InodeRef) -> KResult<azsys::StatFs> {
    let m = mount_mut(ip.borrow().dev)?;
    Ok(azsys::StatFs {
        bsize: crate::bio::BSIZE as u32,
        blocks: m.sb.blocks - m.sb.data_start,
        bfree: m.sb.free_blocks,
        files: m.sb.inodes,
        ffree: m.sb.free_inodes,
        flags: m.read_only as u32,
    })
}

pub fn chmod(path: &[u8], mode: u32) -> KResult<()> {
    let ip = lookup(path, true)?;
    read_only(&ip)?;
    let p = proc::current();
    let mut i = ip.borrow_mut();
    if p.euid != 0 && p.euid != i.di.uid as u32 {
        return Err(EPERM);
    }
    i.di.mode = (i.di.mode & S_IFMT) | (mode & 0o7777) as u16;
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

pub fn chown(path: &[u8], uid: u32, gid: u32) -> KResult<()> {
    require_root()?;
    let ip = lookup(path, false)?;
    read_only(&ip)?;
    let mut i = ip.borrow_mut();
    if uid != u32::MAX {
        i.di.uid = uid as u16;
    }
    if gid != u32::MAX {
        i.di.gid = gid as u16;
    }
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

pub fn utime(path: &[u8], atime: u32, mtime: u32) -> KResult<()> {
    let ip = lookup(path, true)?;
    read_only(&ip)?;
    let p = proc::current();
    let mut i = ip.borrow_mut();
    if p.euid != 0 && p.euid != i.di.uid as u32 && !permitted(&i, 2) {
        return Err(EPERM);
    }
    i.di.atime = atime;
    i.di.mtime = mtime;
    i.di.ctime = timer::now();
    i.dirty = true;
    Ok(())
}

pub fn truncate(ip: &InodeRef, size: u32) -> KResult<()> {
    read_only(ip)?;
    let mut i = ip.borrow_mut();
    if i.is_dir() {
        return Err(EISDIR);
    }
    if size > i.di.size {
        // extend with a hole
        i.di.size = size;
        i.dirty = true;
        return Ok(());
    }
    disk::truncate(&mut i, size)
}

/// getcwd: walk up from the current directory.
pub fn getcwd() -> KResult<Vec<u8>> {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut cur = proc::current().cwd.clone().unwrap_or_else(root_inode);
    for _ in 0..64 {
        let up = parent_dir(&cur)?;
        let (cdev, cino) = {
            let c = cur.borrow();
            (c.dev, c.ino)
        };
        let (udev, uino) = {
            let u = up.borrow();
            (u.dev, u.ino)
        };
        if (udev, uino) == (cdev, cino) {
            break;
        }
        // name of cur in up; a mount root is named by its mount point
        let (want_dev, want_ino) = match mount_mut(cdev) {
            Ok(m) if m.root.as_ref().is_some_and(|r| r.borrow().ino == cino) => match &m.covered {
                Some(c) => {
                    let c = c.borrow();
                    (c.dev, c.ino)
                }
                None => (cdev, cino),
            },
            _ => (cdev, cino),
        };
        let mut name = None;
        {
            let mut u = up.borrow_mut();
            if u.dev == want_dev {
                disk::dir_scan(&mut u, |_, ino, n| {
                    if ino == want_ino && n != b"." && n != b".." {
                        name = Some(n.to_vec());
                        true
                    } else {
                        false
                    }
                })?;
            }
        }
        parts.push(name.ok_or(ENOENT)?);
        cur = up;
    }
    let mut out = Vec::new();
    for p in parts.iter().rev() {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    Ok(out)
}
