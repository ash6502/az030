//! Files and directories.

use crate::io::{self, Fd, Read, Write};
use crate::sys::{self, nr, Errno, Result};
use alloc::string::String;
use alloc::vec::Vec;
use azsys::flags::*;
use azsys::mode::*;

/// An open file; closed when dropped.
pub struct File {
    fd: Fd,
}

impl File {
    pub fn open(path: &str) -> Result<File> {
        Self::open_with(path, O_RDONLY, 0)
    }

    /// Create (or truncate) for writing.
    pub fn create(path: &str) -> Result<File> {
        Self::open_with(path, O_WRONLY | O_CREAT | O_TRUNC, 0o666)
    }

    pub fn append(path: &str) -> Result<File> {
        Self::open_with(path, O_WRONLY | O_CREAT | O_APPEND, 0o666)
    }

    pub fn open_with(path: &str, flags: u32, mode: u32) -> Result<File> {
        let fd = sys::path_call(nr::OPEN, path, flags | O_CLOEXEC, mode)?;
        Ok(File { fd })
    }

    pub fn fd(&self) -> Fd {
        self.fd
    }

    /// Give up ownership of the descriptor (it is not closed).
    pub fn into_raw(self) -> Fd {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }

    pub fn from_raw(fd: Fd) -> File {
        File { fd }
    }

    pub fn seek(&mut self, off: i32, whence: u32) -> Result<u32> {
        sys::call3(nr::LSEEK, self.fd, off as u32, whence)
    }

    pub fn metadata(&self) -> Result<Metadata> {
        let mut st = azsys::Stat::default();
        sys::call2(nr::FSTAT, self.fd, &mut st as *mut _ as u32)?;
        Ok(Metadata(st))
    }

    pub fn set_len(&self, len: u32) -> Result<()> {
        sys::call2(nr::FTRUNCATE, self.fd, len).map(|_| ())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = io::close(self.fd);
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        io::read_fd(self.fd, buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        io::write_fd(self.fd, buf).map(|_| buf.len())
    }
}

pub fn read(path: &str) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    File::open(path)?.read_to_end(&mut v)?;
    Ok(v)
}

pub fn read_to_string(path: &str) -> Result<String> {
    let v = read(path)?;
    Ok(String::from_utf8_lossy(&v).into_owned())
}

pub fn write(path: &str, data: &[u8]) -> Result<()> {
    File::create(path)?.write_all(data)
}

/// `stat` result.
#[derive(Clone, Copy)]
pub struct Metadata(pub azsys::Stat);

impl Metadata {
    pub fn mode(&self) -> u32 {
        self.0.mode
    }
    pub fn kind(&self) -> u32 {
        self.0.mode & S_IFMT
    }
    pub fn is_dir(&self) -> bool {
        self.kind() == S_IFDIR
    }
    pub fn is_file(&self) -> bool {
        self.kind() == S_IFREG
    }
    pub fn is_symlink(&self) -> bool {
        self.kind() == S_IFLNK
    }
    pub fn len(&self) -> u32 {
        self.0.size
    }
    pub fn is_empty(&self) -> bool {
        self.0.size == 0
    }
    pub fn uid(&self) -> u32 {
        self.0.uid
    }
    pub fn gid(&self) -> u32 {
        self.0.gid
    }
    pub fn nlink(&self) -> u32 {
        self.0.nlink
    }
    pub fn mtime(&self) -> u32 {
        self.0.mtime
    }
    pub fn ino(&self) -> u32 {
        self.0.ino
    }
    pub fn dev(&self) -> u32 {
        self.0.dev
    }
    pub fn rdev(&self) -> u32 {
        self.0.rdev
    }

    /// `drwxr-xr-x`-style permission string.
    pub fn mode_string(&self) -> String {
        mode_string(self.0.mode)
    }
}

pub fn mode_string(mode: u32) -> String {
    let t = match mode & S_IFMT {
        S_IFDIR => 'd',
        S_IFLNK => 'l',
        S_IFCHR => 'c',
        S_IFBLK => 'b',
        S_IFIFO => 'p',
        S_IFSOCK => 's',
        _ => '-',
    };
    let mut s = String::new();
    s.push(t);
    let bit = |b: u32, c: char| if mode & b != 0 { c } else { '-' };
    s.push(bit(0o400, 'r'));
    s.push(bit(0o200, 'w'));
    s.push(match (mode & 0o100 != 0, mode & S_ISUID != 0) {
        (true, true) => 's',
        (false, true) => 'S',
        (true, false) => 'x',
        _ => '-',
    });
    s.push(bit(0o040, 'r'));
    s.push(bit(0o020, 'w'));
    s.push(match (mode & 0o010 != 0, mode & S_ISGID != 0) {
        (true, true) => 's',
        (false, true) => 'S',
        (true, false) => 'x',
        _ => '-',
    });
    s.push(bit(0o004, 'r'));
    s.push(bit(0o002, 'w'));
    s.push(match (mode & 0o001 != 0, mode & S_ISVTX != 0) {
        (true, true) => 't',
        (false, true) => 'T',
        (true, false) => 'x',
        _ => '-',
    });
    s
}

pub fn metadata(path: &str) -> Result<Metadata> {
    let mut st = azsys::Stat::default();
    sys::path_call(nr::STAT, path, &mut st as *mut _ as u32, 0)?;
    Ok(Metadata(st))
}

pub fn symlink_metadata(path: &str) -> Result<Metadata> {
    let mut st = azsys::Stat::default();
    sys::path_call(nr::LSTAT, path, &mut st as *mut _ as u32, 0)?;
    Ok(Metadata(st))
}

pub fn exists(path: &str) -> bool {
    symlink_metadata(path).is_ok()
}

pub fn is_dir(path: &str) -> bool {
    metadata(path).is_ok_and(|m| m.is_dir())
}

/// One directory entry.
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub ino: u32,
    pub name: String,
    pub dtype: u8,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.dtype == azsys::DT_DIR
    }
}

/// All entries of a directory (including `.` and `..`), in directory order.
pub fn read_dir(path: &str) -> Result<Vec<DirEntry>> {
    let f = File::open_with(path, O_RDONLY | O_DIRECTORY, 0)?;
    let mut out = Vec::new();
    let mut buf = alloc::vec![0u8; 4096];
    loop {
        let n = sys::call3(nr::GETDENTS, f.fd, buf.as_mut_ptr() as u32, buf.len() as u32)? as usize;
        if n == 0 {
            return Ok(out);
        }
        let mut p = 0;
        while p + 8 <= n {
            let ino = u32::from_be_bytes(buf[p..p + 4].try_into().unwrap());
            let reclen = u16::from_be_bytes([buf[p + 4], buf[p + 5]]) as usize;
            let dtype = buf[p + 6];
            let namlen = buf[p + 7] as usize;
            let name = String::from_utf8_lossy(&buf[p + 8..p + 8 + namlen]).into_owned();
            out.push(DirEntry { ino, name, dtype });
            if reclen == 0 {
                break;
            }
            p += reclen;
        }
    }
}

pub fn create_dir(path: &str) -> Result<()> {
    create_dir_mode(path, 0o777)
}

pub fn create_dir_mode(path: &str, mode: u32) -> Result<()> {
    sys::path_call(nr::MKDIR, path, mode, 0).map(|_| ())
}

/// mkdir -p
pub fn create_dir_all(path: &str) -> Result<()> {
    if path.is_empty() || is_dir(path) {
        return Ok(());
    }
    if let Some(parent) = crate::path::parent(path) {
        create_dir_all(parent)?;
    }
    match create_dir(path) {
        Err(Errno(e)) if e == sys::errno::EEXIST && is_dir(path) => Ok(()),
        r => r,
    }
}

pub fn remove_file(path: &str) -> Result<()> {
    sys::path_call(nr::UNLINK, path, 0, 0).map(|_| ())
}

pub fn remove_dir(path: &str) -> Result<()> {
    sys::path_call(nr::RMDIR, path, 0, 0).map(|_| ())
}

/// rm -r
pub fn remove_dir_all(path: &str) -> Result<()> {
    let m = symlink_metadata(path)?;
    if !m.is_dir() {
        return remove_file(path);
    }
    for e in read_dir(path)? {
        if e.name == "." || e.name == ".." {
            continue;
        }
        remove_dir_all(&crate::path::join(path, &e.name))?;
    }
    remove_dir(path)
}

fn two_paths(n: u32, a: &str, b: &str) -> Result<()> {
    let (x, y) = (sys::cstr(a), sys::cstr(b));
    sys::call2(n, x.as_ptr() as u32, y.as_ptr() as u32).map(|_| ())
}

pub fn rename(from: &str, to: &str) -> Result<()> {
    two_paths(nr::RENAME, from, to)
}

pub fn hard_link(from: &str, to: &str) -> Result<()> {
    two_paths(nr::LINK, from, to)
}

pub fn symlink(target: &str, path: &str) -> Result<()> {
    two_paths(nr::SYMLINK, target, path)
}

pub fn read_link(path: &str) -> Result<String> {
    let mut buf = alloc::vec![0u8; 1024];
    let n = sys::path_call(nr::READLINK, path, buf.as_mut_ptr() as u32, buf.len() as u32)? as usize;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

pub fn chmod(path: &str, mode: u32) -> Result<()> {
    sys::path_call(nr::CHMOD, path, mode, 0).map(|_| ())
}

pub fn chown(path: &str, uid: u32, gid: u32) -> Result<()> {
    sys::path_call(nr::CHOWN, path, uid, gid).map(|_| ())
}

/// Set access and modification times (None = now).
pub fn set_times(path: &str, times: Option<(u32, u32)>) -> Result<()> {
    match times {
        None => sys::path_call(nr::UTIME, path, 0, 0).map(|_| ()),
        Some((a, m)) => {
            let t = [a, m];
            sys::path_call(nr::UTIME, path, t.as_ptr() as u32, 0).map(|_| ())
        }
    }
}

pub fn mknod(path: &str, mode: u32, dev: u32) -> Result<()> {
    sys::path_call(nr::MKNOD, path, mode, dev).map(|_| ())
}

pub fn access(path: &str, mode: u32) -> Result<()> {
    sys::path_call(nr::ACCESS, path, mode, 0).map(|_| ())
}

pub fn statfs(path: &str) -> Result<azsys::StatFs> {
    let mut s = azsys::StatFs::default();
    sys::path_call(nr::STATFS, path, &mut s as *mut _ as u32, 0)?;
    Ok(s)
}

pub fn sync() {
    let _ = sys::call0(nr::SYNC);
}

pub fn mount(source: &str, target: &str, read_only: bool) -> Result<()> {
    let (a, b) = (sys::cstr(source), sys::cstr(target));
    sys::call3(nr::MOUNT, a.as_ptr() as u32, b.as_ptr() as u32, if read_only { MS_RDONLY } else { 0 }).map(|_| ())
}

pub fn umount(target: &str) -> Result<()> {
    sys::path_call(nr::UMOUNT, target, 0, 0).map(|_| ())
}

/// The mount table.
pub fn mounts() -> Vec<azsys::MountInfo> {
    let mut v = Vec::new();
    for i in 0.. {
        let mut m = azsys::MountInfo { dev: 0, flags: 0, path: [0; 64], source: [0; 32] };
        match sys::call2(nr::MOUNTINFO, i, &mut m as *mut _ as u32) {
            Ok(1) => v.push(m),
            _ => break,
        }
    }
    v
}

pub fn cstr_field(b: &[u8]) -> &str {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..n]).unwrap_or("?")
}
