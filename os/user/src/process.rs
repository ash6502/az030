//! Processes: exit, fork/exec/wait, pipes and descriptors, ids, system information.

use crate::io::{self, Fd};
use crate::sys::{self, errno, nr, Errno, Result};
use alloc::string::String;
use alloc::vec::Vec;
use azsys::flags::*;

pub fn exit(code: i32) -> ! {
    io::stdout().flush_quiet();
    loop {
        sys::raw(nr::EXIT, code as u32, 0, 0, 0, 0);
    }
}

pub fn id() -> u32 {
    sys::call0(nr::GETPID).unwrap_or(0)
}

pub fn parent_id() -> u32 {
    sys::call0(nr::GETPPID).unwrap_or(0)
}

pub fn uid() -> u32 {
    sys::call0(nr::GETUID).unwrap_or(0)
}
pub fn gid() -> u32 {
    sys::call0(nr::GETGID).unwrap_or(0)
}
pub fn euid() -> u32 {
    sys::call0(nr::GETEUID).unwrap_or(0)
}
pub fn egid() -> u32 {
    sys::call0(nr::GETEGID).unwrap_or(0)
}
pub fn set_uid(uid: u32) -> Result<()> {
    sys::call1(nr::SETUID, uid).map(|_| ())
}
pub fn set_gid(gid: u32) -> Result<()> {
    sys::call1(nr::SETGID, gid).map(|_| ())
}

pub fn umask(mask: u32) -> u32 {
    sys::call1(nr::UMASK, mask).unwrap_or(0)
}

/// fork(2): Ok(0) in the child, Ok(pid) in the parent.
pub fn fork() -> Result<u32> {
    io::stdout().flush_quiet();
    sys::call0(nr::FORK)
}

/// Replace the process image. Only returns on failure.
pub fn execve(path: &str, args: &[String], env: &[String]) -> Errno {
    let p = sys::cstr(path);
    let cs = |v: &[String]| -> Vec<Vec<u8>> { v.iter().map(|s| sys::cstr(s)).collect() };
    let (a, e) = (cs(args), cs(env));
    let ptrs = |v: &Vec<Vec<u8>>| -> Vec<u32> { v.iter().map(|s| s.as_ptr() as u32).chain([0]).collect() };
    let (ap, ep) = (ptrs(&a), ptrs(&e));
    io::stdout().flush_quiet();
    match sys::call3(nr::EXECVE, p.as_ptr() as u32, ap.as_ptr() as u32, ep.as_ptr() as u32) {
        Err(e) => e,
        Ok(_) => Errno(errno::EIO),
    }
}

/// Find a command: names containing `/` are used as is; others are looked up in $PATH.
pub fn find_program(name: &str) -> Option<String> {
    if name.contains('/') {
        return Some(name.into());
    }
    let path = crate::env::var("PATH").unwrap_or_else(|| "/bin:/usr/bin:/sbin".into());
    for dir in path.split(':') {
        let p = crate::path::join(if dir.is_empty() { "." } else { dir }, name);
        if let Ok(m) = crate::fs::metadata(&p) {
            if m.is_file() && m.mode() & 0o111 != 0 {
                return Some(p);
            }
        }
    }
    None
}

/// execvp: search $PATH and use the current environment. Only returns on failure.
pub fn exec(args: &[String]) -> Errno {
    let Some(name) = args.first() else { return Errno(errno::EINVAL) };
    match find_program(name) {
        Some(p) => execve(&p, args, &crate::env::environ()),
        None => Errno(errno::ENOENT),
    }
}

/// How a child ended (`waitpid` status).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus(pub u32);

impl ExitStatus {
    pub fn exited(self) -> bool {
        self.0 & 0x7F == 0
    }
    pub fn code(self) -> Option<i32> {
        if self.exited() { Some(((self.0 >> 8) & 0xFF) as i32) } else { None }
    }
    pub fn signal(self) -> Option<u32> {
        let s = self.0 & 0x7F;
        if s != 0 && s != 0x7F { Some(s) } else { None }
    }
    pub fn stopped(self) -> bool {
        self.0 & 0xFF == 0x7F
    }
    pub fn stop_signal(self) -> u32 {
        (self.0 >> 8) & 0xFF
    }
    pub fn core_dumped(self) -> bool {
        self.0 & 0x80 != 0
    }
    pub fn success(self) -> bool {
        self.code() == Some(0)
    }
    /// The shell's `$?`: the exit code, or 128 + signal.
    pub fn shell_code(self) -> i32 {
        match (self.code(), self.signal()) {
            (Some(c), _) => c,
            (None, Some(s)) => 128 + s as i32,
            _ => 128 + self.stop_signal() as i32,
        }
    }
}

/// waitpid(2). Returns (pid, status); pid 0 with WNOHANG means nothing is ready.
pub fn waitpid(pid: i32, options: u32) -> Result<(u32, ExitStatus)> {
    let mut st: u32 = 0;
    loop {
        match sys::call3(nr::WAITPID, pid as u32, &mut st as *mut u32 as u32, options) {
            Err(Errno(e)) if e == errno::EINTR => continue,
            r => return r.map(|p| (p, ExitStatus(st))),
        }
    }
}

pub fn wait(pid: u32) -> Result<ExitStatus> {
    waitpid(pid as i32, 0).map(|r| r.1)
}

/// Run a command (searching $PATH) and wait for it.
pub fn run(args: &[String]) -> Result<ExitStatus> {
    let pid = fork()?;
    if pid == 0 {
        let e = exec(args);
        crate::warn!("{}: {}", args[0], e);
        exit(127);
    }
    wait(pid)
}

pub fn kill(pid: i32, sig: u32) -> Result<()> {
    sys::call2(nr::KILL, pid as u32, sig).map(|_| ())
}

pub fn pipe() -> Result<(Fd, Fd)> {
    let mut fds = [0u32; 2];
    sys::call1(nr::PIPE, fds.as_mut_ptr() as u32)?;
    Ok((fds[0], fds[1]))
}

pub fn dup(fd: Fd) -> Result<Fd> {
    sys::call1(nr::DUP, fd)
}

pub fn dup2(old: Fd, new: Fd) -> Result<Fd> {
    sys::call2(nr::DUP2, old, new)
}

/// Duplicate to the lowest descriptor >= `min`, close-on-exec.
pub fn dup_above(fd: Fd, min: Fd) -> Result<Fd> {
    let n = sys::call3(nr::FCNTL, fd, F_DUPFD, min)?;
    sys::call3(nr::FCNTL, n, F_SETFD, FD_CLOEXEC)?;
    Ok(n)
}

pub fn set_cloexec(fd: Fd, on: bool) -> Result<()> {
    sys::call3(nr::FCNTL, fd, F_SETFD, if on { FD_CLOEXEC } else { 0 }).map(|_| ())
}

pub fn set_nonblocking(fd: Fd, on: bool) -> Result<()> {
    let fl = sys::call3(nr::FCNTL, fd, F_GETFL, 0)?;
    let fl = if on { fl | O_NONBLOCK } else { fl & !O_NONBLOCK };
    sys::call3(nr::FCNTL, fd, F_SETFL, fl).map(|_| ())
}

pub fn setpgid(pid: u32, pgid: u32) -> Result<()> {
    sys::call2(nr::SETPGID, pid, pgid).map(|_| ())
}

pub fn getpgid(pid: u32) -> Result<u32> {
    sys::call1(nr::GETPGID, pid)
}

pub fn setsid() -> Result<u32> {
    sys::call0(nr::SETSID)
}

/// Block until a signal arrives.
pub fn pause() {
    let _ = sys::call0(nr::PAUSE);
}

pub fn reboot(cmd: u32) -> Errno {
    io::stdout().flush_quiet();
    match sys::call1(nr::REBOOT, cmd) {
        Err(e) => e,
        Ok(_) => Errno(errno::EIO),
    }
}

/// All processes.
pub fn processes() -> Vec<azsys::ProcInfo> {
    let mut v = Vec::new();
    for i in 0.. {
        let mut p = azsys::ProcInfo {
            pid: 0,
            ppid: 0,
            pgid: 0,
            sid: 0,
            uid: 0,
            state: 0,
            mem_kb: 0,
            utime: 0,
            stime: 0,
            start: 0,
            tty: 0,
            name: [0; 32],
        };
        match sys::call2(nr::PROCINFO, i, &mut p as *mut _ as u32) {
            Ok(1) => v.push(p),
            _ => break,
        }
    }
    v
}

pub fn sysinfo() -> azsys::SysInfo {
    let mut s = azsys::SysInfo::default();
    let _ = sys::call1(nr::SYSINFO, &mut s as *mut _ as u32);
    s
}

pub fn uname() -> azsys::Utsname {
    let mut u = azsys::Utsname { sysname: [0; 32], nodename: [0; 32], release: [0; 32], version: [0; 64], machine: [0; 32] };
    let _ = sys::call1(nr::UNAME, &mut u as *mut _ as u32);
    u
}

/// CPU times of this process and its waited-for children, in ticks; returns uptime ticks.
pub fn times() -> (azsys::Tms, u32) {
    let mut t = azsys::Tms::default();
    let r = sys::call1(nr::TIMES, &mut t as *mut _ as u32).unwrap_or(0);
    (t, r)
}
