//! Open files: regular files and directories, devices, and pipes.

use crate::fs::{self, InodeRef};
use crate::pipe::{self, Pipe};
use crate::proc::{self, Proc};
use crate::vm::KResult;
use crate::{scsi, tty};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use azfs::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG};
use azsys::dev::*;
use azsys::errno::*;
use azsys::flags::*;
use core::cell::RefCell;

pub type FileRef = Rc<RefCell<OpenFile>>;

pub enum Kind {
    Inode(InodeRef),
    PipeRead(Rc<RefCell<Pipe>>),
    PipeWrite(Rc<RefCell<Pipe>>),
    Tty(u32),
    Null,
    Zero,
    Random,
    /// Raw SCSI disk, by SCSI ID.
    Disk(u32),
}

pub struct OpenFile {
    pub kind: Kind,
    pub flags: u32,
    pub offset: u32,
    /// The device node a device file was opened through (for fstat).
    pub node: Option<InodeRef>,
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        match &self.kind {
            Kind::PipeRead(p) => pipe::close_end(p, false),
            Kind::PipeWrite(p) => pipe::close_end(p, true),
            _ => {}
        }
    }
}

pub fn new_file(kind: Kind, flags: u32, node: Option<InodeRef>) -> FileRef {
    Rc::new(RefCell::new(OpenFile { kind, flags, offset: 0, node }))
}

fn readable(flags: u32) -> bool {
    flags & O_ACCMODE != O_WRONLY
}

fn writable(flags: u32) -> bool {
    flags & O_ACCMODE != O_RDONLY
}

/// open(2) on behalf of process `p`.
pub fn open_path(p: &mut Proc, path: &str, flags: u32, mode: u32) -> KResult<FileRef> {
    let _ = &p;
    let path = path.as_bytes();
    let ip = if flags & O_CREAT != 0 {
        fs::create(path, S_IFREG as u32 | (mode & 0o7777), 0, flags & O_EXCL != 0)?
    } else {
        fs::lookup(path, flags & O_NOFOLLOW == 0)?
    };
    let (kind_bits, rdev) = {
        let i = ip.borrow();
        (i.kind(), i.di.rdev)
    };
    if kind_bits == S_IFLNK {
        return Err(ELOOP);
    }
    if flags & O_DIRECTORY != 0 && kind_bits != S_IFDIR {
        return Err(ENOTDIR);
    }
    {
        let i = ip.borrow();
        let want = if readable(flags) { 4 } else { 0 } | if writable(flags) { 2 } else { 0 };
        if want != 0 && !fs::permitted(&i, want) {
            return Err(EACCES);
        }
        if kind_bits == S_IFDIR && writable(flags) {
            return Err(EISDIR);
        }
    }
    let kind = match kind_bits {
        S_IFCHR => match (rdev >> 8, rdev & 0xFF) {
            (MEM_MAJOR, 3) => Kind::Null,
            (MEM_MAJOR, 5) => Kind::Zero,
            (MEM_MAJOR, 8) | (MEM_MAJOR, 9) => Kind::Random,
            (TTY_MAJOR, n) => {
                tty::open(proc::current(), n, flags)?;
                Kind::Tty(n)
            }
            (CTTY_MAJOR, 0) => {
                let t = proc::current().tty;
                if t == 0 {
                    return Err(ENXIO);
                }
                Kind::Tty(t & 0xFF)
            }
            _ => return Err(ENXIO),
        },
        S_IFBLK => match (rdev >> 8, rdev & 0xFF) {
            (SD_MAJOR, id) if scsi::disk(id).is_some() => Kind::Disk(id),
            _ => return Err(ENXIO),
        },
        S_IFIFO => return Err(ENXIO),
        _ => {
            if flags & O_TRUNC != 0 && writable(flags) && kind_bits == S_IFREG {
                fs::truncate(&ip, 0)?;
            }
            let f = new_file(Kind::Inode(ip), flags & !(O_CREAT | O_EXCL | O_TRUNC | O_NOCTTY), None);
            return Ok(f);
        }
    };
    Ok(new_file(kind, flags & !(O_CREAT | O_EXCL | O_TRUNC | O_NOCTTY), Some(ip)))
}

/// Largest single read or write the kernel buffers at once.
pub const MAX_IO: usize = 64 * 1024;

pub fn read(f: &FileRef, len: usize) -> KResult<Vec<u8>> {
    let (flags, off) = {
        let o = f.borrow();
        (o.flags, o.offset)
    };
    if !readable(flags) {
        return Err(EBADF);
    }
    let len = len.min(MAX_IO);
    let kind_ref = {
        let o = f.borrow();
        match &o.kind {
            Kind::Inode(ip) => Some(ip.clone()),
            _ => None,
        }
    };
    if let Some(ip) = kind_ref {
        let mut i = ip.borrow_mut();
        if i.kind() == S_IFDIR {
            return Err(EISDIR);
        }
        let mut buf = vec![0u8; len.min(i.di.size.saturating_sub(off) as usize)];
        let n = fs::disk::read(&mut i, off, &mut buf)?;
        buf.truncate(n);
        drop(i);
        f.borrow_mut().offset = off + n as u32;
        return Ok(buf);
    }
    let pipe = match &f.borrow().kind {
        Kind::PipeRead(p) => Some(p.clone()),
        _ => None,
    };
    if let Some(p) = pipe {
        return pipe::read(&p, len, flags & O_NONBLOCK != 0);
    }
    let kind = match &f.borrow().kind {
        Kind::Tty(n) => Some(*n),
        _ => None,
    };
    if let Some(n) = kind {
        return tty::read(n, len, flags & O_NONBLOCK != 0);
    }
    let o = f.borrow();
    match &o.kind {
        Kind::Null => Ok(Vec::new()),
        Kind::Zero => Ok(vec![0u8; len]),
        Kind::Random => Ok((0..len).map(|_| random_byte()).collect()),
        Kind::Disk(id) => {
            let id = *id;
            drop(o);
            let data = disk_io(id, off, len, None)?;
            f.borrow_mut().offset = off + data.len() as u32;
            Ok(data)
        }
        _ => Err(EBADF),
    }
}

pub fn write(f: &FileRef, data: &[u8]) -> KResult<usize> {
    let (flags, mut off) = {
        let o = f.borrow();
        (o.flags, o.offset)
    };
    if !writable(flags) {
        return Err(EBADF);
    }
    let ip = match &f.borrow().kind {
        Kind::Inode(ip) => Some(ip.clone()),
        _ => None,
    };
    if let Some(ip) = ip {
        let dev = ip.borrow().dev;
        if fs::mount_mut(dev)?.read_only {
            return Err(EROFS);
        }
        let mut i = ip.borrow_mut();
        if flags & O_APPEND != 0 {
            off = i.di.size;
        }
        let n = fs::disk::write(&mut i, off, data)?;
        drop(i);
        f.borrow_mut().offset = off + n as u32;
        return Ok(n);
    }
    let pipe = match &f.borrow().kind {
        Kind::PipeWrite(p) => Some(p.clone()),
        _ => None,
    };
    if let Some(p) = pipe {
        return pipe::write(&p, data, flags & O_NONBLOCK != 0);
    }
    let o = f.borrow();
    match &o.kind {
        Kind::Tty(n) => tty::write(*n, data),
        Kind::Null | Kind::Zero | Kind::Random => Ok(data.len()),
        Kind::Disk(id) => {
            let id = *id;
            drop(o);
            disk_io(id, off, data.len(), Some(data))?;
            f.borrow_mut().offset = off + data.len() as u32;
            Ok(data.len())
        }
        _ => Err(EBADF),
    }
}

/// Byte-addressed raw disk access (read-modify-write for partial sectors).
fn disk_io(id: u32, off: u32, len: usize, data: Option<&[u8]>) -> KResult<Vec<u8>> {
    let disk = scsi::disk(id).ok_or(ENXIO)?;
    let size = disk.blocks as u64 * 512;
    let off64 = off as u64;
    if off64 >= size {
        return Ok(Vec::new());
    }
    let len = len.min((size - off64) as usize);
    let first = off / 512;
    let last = (off64 + len as u64).div_ceil(512) as u32;
    let count = last - first;
    let mut buf = vec![0u8; count as usize * 512];
    let mut done = 0u32;
    while done < count {
        let n = (count - done).min(128);
        let s = (done * 512) as usize;
        scsi::read(id, first + done, n, &mut buf[s..s + n as usize * 512]).map_err(|_| EIO)?;
        done += n;
    }
    let start = (off % 512) as usize;
    match data {
        None => Ok(buf[start..start + len].to_vec()),
        Some(d) => {
            if disk.read_only {
                return Err(EROFS);
            }
            buf[start..start + len].copy_from_slice(&d[..len]);
            let mut done = 0u32;
            while done < count {
                let n = (count - done).min(128);
                let s = (done * 512) as usize;
                scsi::write(id, first + done, n, &buf[s..s + n as usize * 512]).map_err(|_| EIO)?;
                done += n;
            }
            Ok(Vec::new())
        }
    }
}

static mut RNG: u32 = 0x2545_F491;

fn random_byte() -> u8 {
    unsafe {
        if RNG == 0x2545_F491 {
            RNG ^= crate::timer::ticks() as u32 ^ crate::timer::now();
        }
        RNG ^= RNG << 13;
        RNG ^= RNG >> 17;
        RNG ^= RNG << 5;
        (RNG >> 11) as u8
    }
}

pub fn lseek(f: &FileRef, off: i32, whence: u32) -> KResult<u32> {
    let mut o = f.borrow_mut();
    let size = match &o.kind {
        Kind::Inode(ip) => ip.borrow().di.size,
        Kind::Disk(id) => scsi::disk(*id).map_or(0, |d| d.blocks.saturating_mul(512)),
        Kind::Null | Kind::Zero | Kind::Random => 0,
        _ => return Err(ESPIPE),
    };
    let base = match whence {
        SEEK_SET => 0i64,
        SEEK_CUR => o.offset as i64,
        SEEK_END => size as i64,
        _ => return Err(EINVAL),
    };
    let n = base + off as i64;
    if !(0..=u32::MAX as i64).contains(&n) {
        return Err(EINVAL);
    }
    o.offset = n as u32;
    Ok(n as u32)
}

pub fn stat(f: &FileRef) -> KResult<azsys::Stat> {
    let o = f.borrow();
    match &o.kind {
        Kind::Inode(ip) => Ok(fs::stat(ip)),
        Kind::PipeRead(_) | Kind::PipeWrite(_) => {
            Ok(azsys::Stat { mode: S_IFIFO as u32 | 0o600, nlink: 1, blksize: 4096, ..Default::default() })
        }
        _ => match &o.node {
            Some(ip) => Ok(fs::stat(ip)),
            None => Ok(azsys::Stat { mode: S_IFCHR as u32 | 0o666, ..Default::default() }),
        },
    }
}

pub fn inode(f: &FileRef) -> Option<InodeRef> {
    match &f.borrow().kind {
        Kind::Inode(ip) => Some(ip.clone()),
        _ => None,
    }
}

pub fn is_tty(f: &FileRef) -> Option<u32> {
    match &f.borrow().kind {
        Kind::Tty(n) => Some(*n),
        _ => None,
    }
}

/// getdents: as many records as fit in `max` bytes, starting at the file offset.
pub fn getdents(f: &FileRef, max: usize) -> KResult<Vec<u8>> {
    let ip = inode(f).ok_or(ENOTDIR)?;
    if !ip.borrow().is_dir() {
        return Err(ENOTDIR);
    }
    let off = f.borrow().offset;
    let mut entries: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    {
        let mut i = ip.borrow_mut();
        let mut size = 0usize;
        fs::disk::dir_scan(&mut i, |pos, ino, name| {
            if pos < off {
                return false;
            }
            let reclen = (8 + name.len() + 1).next_multiple_of(4);
            if size + reclen > max {
                return true;
            }
            size += reclen;
            entries.push((pos, ino, name.to_vec()));
            false
        })?;
    }
    if entries.is_empty() {
        let end = ip.borrow().di.size;
        if off < end && max < 8 + 61 {
            return Err(EINVAL);
        }
        f.borrow_mut().offset = end;
        return Ok(Vec::new());
    }
    let dev = ip.borrow().dev;
    let mut out = Vec::new();
    for (_, ino, name) in &entries {
        let dtype = match fs::iget(dev, *ino).map(|c| c.borrow().kind()) {
            Ok(S_IFDIR) => azsys::DT_DIR,
            Ok(S_IFREG) => azsys::DT_REG,
            Ok(S_IFLNK) => azsys::DT_LNK,
            Ok(S_IFCHR) => azsys::DT_CHR,
            Ok(S_IFBLK) => azsys::DT_BLK,
            Ok(S_IFIFO) => azsys::DT_FIFO,
            _ => azsys::DT_UNKNOWN,
        };
        let reclen = (8 + name.len() + 1).next_multiple_of(4);
        out.extend_from_slice(&ino.to_be_bytes());
        out.extend_from_slice(&(reclen as u16).to_be_bytes());
        out.push(dtype);
        out.push(name.len() as u8);
        out.extend_from_slice(name);
        out.resize(out.len() + reclen - 8 - name.len(), 0);
    }
    let last = entries.last().unwrap().0;
    f.borrow_mut().offset = last + azfs::DIRENT_SIZE as u32;
    let _ = S_IFMT;
    Ok(out)
}
