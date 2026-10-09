//! System calls (ABI in lib/azsys).

use crate::file::{self, FileRef};
use crate::proc::{self, State, NOFILE};
use crate::trap::TrapFrame;
use crate::vm::KResult;
use crate::{arch, exec, fs, mem, pipe, signal, timer, tty, util};
use alloc::vec;
use alloc::vec::Vec;
use azsys::errno::*;
use azsys::flags::*;
use azsys::nr::*;
use azsys::*;

const PATH_MAX: usize = 1024;

fn path(addr: u32) -> KResult<Vec<u8>> {
    if addr == 0 {
        return Err(EFAULT);
    }
    let v = proc::current().space().read_cstr(addr, PATH_MAX)?;
    if v.is_empty() {
        return Err(ENOENT);
    }
    Ok(v)
}

/// Copy a plain-old-data structure to user memory.
fn put<T: Copy>(addr: u32, v: &T) -> KResult<()> {
    let b = unsafe { core::slice::from_raw_parts(v as *const T as *const u8, core::mem::size_of::<T>()) };
    proc::current().space().write(addr, b, false)
}

fn get<T: Copy + Default>(addr: u32) -> KResult<T> {
    let mut v = T::default();
    let b = unsafe { core::slice::from_raw_parts_mut(&mut v as *mut T as *mut u8, core::mem::size_of::<T>()) };
    proc::current().space().read(addr, b)?;
    Ok(v)
}

fn fd_file(fd: u32) -> KResult<FileRef> {
    proc::current().file(fd)
}

/// A NULL-terminated array of string pointers (argv / envp).
fn string_array(addr: u32) -> KResult<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    if addr == 0 {
        return Ok(out);
    }
    let sp = proc::current().space();
    let mut total = 0;
    for i in 0..4096u32 {
        let p = sp.read_u32(addr + i * 4)?;
        if p == 0 {
            return Ok(out);
        }
        let s = sp.read_cstr(p, 128 * 1024)?;
        total += s.len() + 1;
        if total > 128 * 1024 {
            return Err(E2BIG);
        }
        out.push(s);
    }
    Err(E2BIG)
}

pub fn dispatch(tf: &mut TrapFrame) {
    let nr = tf.d[0];
    let a = [tf.d[1], tf.d[2], tf.d[3], tf.d[4], tf.d[5]];
    let r: KResult<u32> = match nr {
        EXIT => proc::exit_current(((a[0] & 0xFF) << 8) as i32),
        FORK => proc::fork(tf),
        READ => sys_read(a[0], a[1], a[2]),
        WRITE => sys_write(a[0], a[1], a[2]),
        OPEN => sys_open(a[0], a[1], a[2]),
        CLOSE => sys_close(a[0]),
        WAITPID => sys_waitpid(a[0] as i32, a[1], a[2]),
        LINK => path(a[0]).and_then(|o| fs::link(&o, &path(a[1])?)).map(|_| 0),
        UNLINK => path(a[0]).and_then(|p| fs::unlink(&p)).map(|_| 0),
        EXECVE => sys_execve(tf, a[0], a[1], a[2]),
        CHDIR => sys_chdir(a[0]),
        TIME => Ok(timer::now()),
        MKNOD => sys_mknod(a[0], a[1], a[2]),
        CHMOD => path(a[0]).and_then(|p| fs::chmod(&p, a[1])).map(|_| 0),
        CHOWN => path(a[0]).and_then(|p| fs::chown(&p, a[1], a[2])).map(|_| 0),
        BRK => sys_brk(a[0]),
        STAT => sys_stat(a[0], a[1], true),
        LSTAT => sys_stat(a[0], a[1], false),
        LSEEK => fd_file(a[0]).and_then(|f| file::lseek(&f, a[1] as i32, a[2])),
        GETPID => Ok(proc::current().pid),
        GETPPID => Ok(proc::current().ppid),
        MOUNT => path(a[0]).and_then(|s| fs::mount(&s, &path(a[1])?, a[2])).map(|_| 0),
        UMOUNT => path(a[0]).and_then(|p| fs::umount(&p)).map(|_| 0),
        SETUID => sys_setuid(a[0]),
        SETGID => sys_setgid(a[0]),
        GETUID => Ok(proc::current().uid),
        GETGID => Ok(proc::current().gid),
        GETEUID => Ok(proc::current().euid),
        GETEGID => Ok(proc::current().egid),
        FSTAT => fd_file(a[0]).and_then(|f| put(a[1], &file::stat(&f)?)).map(|_| 0),
        PAUSE => sys_pause(),
        UTIME => sys_utime(a[0], a[1]),
        ACCESS => sys_access(a[0], a[1]),
        SYNC | FSYNC => {
            fs::sync();
            Ok(0)
        }
        KILL => signal::kill(a[0] as i32, a[1]).map(|_| 0),
        RENAME => path(a[0]).and_then(|o| fs::rename(&o, &path(a[1])?)).map(|_| 0),
        MKDIR => path(a[0]).and_then(|p| fs::create(&p, azfs::S_IFDIR as u32 | (a[1] & 0o7777), 0, true)).map(|_| 0),
        RMDIR => path(a[0]).and_then(|p| fs::rmdir(&p)).map(|_| 0),
        DUP => sys_dup(a[0], 0),
        DUP2 => sys_dup2(a[0], a[1]),
        PIPE => sys_pipe(a[0]),
        TIMES => sys_times(a[0]),
        SIGACTION => sys_sigaction(a[0], a[1], a[2]),
        SIGPROCMASK => sys_sigprocmask(a[0], a[1], a[2]),
        SIGRETURN => {
            if signal::sigreturn(tf).is_err() {
                proc::exit_current(sig::SIGSEGV as i32);
            }
            return; // d0 was restored with the rest of the context
        }
        IOCTL => sys_ioctl(a[0], a[1], a[2]),
        FCNTL => sys_fcntl(a[0], a[1], a[2]),
        SETPGID => sys_setpgid(a[0], a[1]),
        GETPGID => sys_getpgid(a[0]),
        SETSID => sys_setsid(),
        UMASK => {
            let p = proc::current();
            let old = p.umask;
            p.umask = a[0] & 0o777;
            Ok(old)
        }
        GETCWD => sys_getcwd(a[0], a[1]),
        GETDENTS => sys_getdents(a[0], a[1], a[2]),
        SYMLINK => path(a[0]).and_then(|t| fs::symlink(&t, &path(a[1])?)).map(|_| 0),
        READLINK => sys_readlink(a[0], a[1], a[2]),
        SLEEP_MS => sys_sleep(a[0]),
        GETTIMEOFDAY => {
            let (s, us) = timer::now_precise();
            put(a[0], &Timeval { sec: s, usec: us }).map(|_| 0)
        }
        SETTIMEOFDAY => {
            if proc::current().euid != 0 {
                Err(EPERM)
            } else {
                timer::set_time(a[0]);
                Ok(0)
            }
        }
        UNAME => sys_uname(a[0]),
        REBOOT => sys_reboot(a[0]),
        PROCINFO => sys_procinfo(a[0], a[1]),
        SYSINFO => sys_sysinfo(a[0]),
        ALARM => sys_alarm(a[0]),
        FTRUNCATE => fd_file(a[0]).and_then(|f| file::inode(&f).ok_or(EINVAL)).and_then(|ip| fs::truncate(&ip, a[1])).map(|_| 0),
        STATFS => path(a[0]).and_then(|p| fs::lookup(&p, true)).and_then(|ip| put(a[1], &fs::statfs(&ip)?)).map(|_| 0),
        MOUNTINFO => sys_mountinfo(a[0], a[1]),
        _ => Err(ENOSYS),
    };
    tf.d[0] = match r {
        Ok(v) => v,
        Err(e) => (-e) as u32,
    };
    if timer::take_sync_due() {
        fs::sync();
    }
}

fn sys_read(fd: u32, buf: u32, len: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    if !proc::current().space().check(buf, len, true) {
        return Err(EFAULT);
    }
    let data = file::read(&f, len as usize)?;
    proc::current().space().write(buf, &data, false)?;
    Ok(data.len() as u32)
}

fn sys_write(fd: u32, buf: u32, len: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    let n = (len as usize).min(file::MAX_IO);
    let mut data = vec![0u8; n];
    proc::current().space().read(buf, &mut data)?;
    Ok(file::write(&f, &data)? as u32)
}

fn sys_open(p_addr: u32, flags: u32, mode: u32) -> KResult<u32> {
    let path = path(p_addr)?;
    let path = core::str::from_utf8(&path).map_err(|_| EINVAL)?;
    let p = proc::current();
    let fd = p.alloc_fd(0)?;
    let f = file::open_path(p, path, flags, mode)?;
    let p = proc::current();
    p.files[fd as usize] = Some(f);
    if flags & O_CLOEXEC != 0 {
        p.cloexec |= 1 << fd;
    } else {
        p.cloexec &= !(1 << fd);
    }
    Ok(fd)
}

fn sys_close(fd: u32) -> KResult<u32> {
    let p = proc::current();
    let slot = p.files.get_mut(fd as usize).ok_or(EBADF)?;
    if slot.take().is_none() {
        return Err(EBADF);
    }
    p.cloexec &= !(1 << fd);
    Ok(0)
}

fn sys_waitpid(pid: i32, status: u32, options: u32) -> KResult<u32> {
    let (cpid, st) = proc::wait(pid, options)?;
    if status != 0 && cpid != 0 {
        proc::current().space().write_u32(status, st)?;
    }
    Ok(cpid)
}

fn sys_execve(tf: &mut TrapFrame, p_addr: u32, argv: u32, envp: u32) -> KResult<u32> {
    let path = path(p_addr)?;
    let argv = string_array(argv)?;
    let envp = string_array(envp)?;
    let p = proc::current();
    let path = core::str::from_utf8(&path).map_err(|_| EINVAL)?;
    exec::exec(p, tf, path, argv, envp)?;
    Ok(0)
}

fn sys_chdir(p_addr: u32) -> KResult<u32> {
    let ip = fs::lookup(&path(p_addr)?, true)?;
    {
        let i = ip.borrow();
        if !i.is_dir() {
            return Err(ENOTDIR);
        }
        if !fs::permitted(&i, 1) {
            return Err(EACCES);
        }
    }
    proc::current().cwd = Some(ip);
    Ok(0)
}

fn sys_mknod(p_addr: u32, mode: u32, dev: u32) -> KResult<u32> {
    if proc::current().euid != 0 {
        return Err(EPERM);
    }
    fs::create(&path(p_addr)?, mode, dev, true)?;
    Ok(0)
}

fn sys_brk(addr: u32) -> KResult<u32> {
    let vm = proc::current().space_mut();
    if addr == 0 {
        return Ok(vm.brk);
    }
    vm.set_brk(addr)
}

fn sys_stat(p_addr: u32, buf: u32, follow: bool) -> KResult<u32> {
    let ip = fs::lookup(&path(p_addr)?, follow)?;
    put(buf, &fs::stat(&ip))?;
    Ok(0)
}

fn sys_setuid(uid: u32) -> KResult<u32> {
    let p = proc::current();
    if p.euid == 0 {
        p.uid = uid;
        p.euid = uid;
    } else if uid == p.uid {
        p.euid = uid;
    } else {
        return Err(EPERM);
    }
    Ok(0)
}

fn sys_setgid(gid: u32) -> KResult<u32> {
    let p = proc::current();
    if p.euid == 0 {
        p.gid = gid;
        p.egid = gid;
    } else if gid == p.gid {
        p.egid = gid;
    } else {
        return Err(EPERM);
    }
    Ok(0)
}

fn sys_pause() -> KResult<u32> {
    let sr = arch::irq_save();
    let chan = proc::current() as *const _ as usize;
    while proc::sleep(chan, true) {}
    arch::irq_restore_sr(sr);
    Err(EINTR)
}

fn sys_sleep(ms: u32) -> KResult<u32> {
    if !timer::present() {
        return Ok(0);
    }
    let deadline = timer::ticks() + timer::ms_to_ticks(ms);
    let chan = proc::current() as *const _ as usize;
    let sr = arch::irq_save();
    while timer::ticks() < deadline {
        if !proc::sleep_until(chan, true, deadline) {
            arch::irq_restore_sr(sr);
            let left = (deadline.saturating_sub(timer::ticks()) * 1000 / HZ as u64) as u32;
            return if left > 0 { Err(EINTR) } else { Ok(0) };
        }
    }
    arch::irq_restore_sr(sr);
    Ok(0)
}

fn sys_utime(p_addr: u32, times: u32) -> KResult<u32> {
    let (a, m) = if times == 0 {
        let n = timer::now();
        (n, n)
    } else {
        let sp = proc::current().space();
        (sp.read_u32(times)?, sp.read_u32(times + 4)?)
    };
    fs::utime(&path(p_addr)?, a, m)?;
    Ok(0)
}

fn sys_access(p_addr: u32, mode: u32) -> KResult<u32> {
    let ip = fs::lookup(&path(p_addr)?, true)?;
    if mode & 7 != 0 && !fs::permitted(&ip.borrow(), mode & 7) {
        return Err(EACCES);
    }
    Ok(0)
}

fn sys_dup(fd: u32, from: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    let p = proc::current();
    let n = p.alloc_fd(from)?;
    p.files[n as usize] = Some(f);
    p.cloexec &= !(1 << n);
    Ok(n)
}

fn sys_dup2(old: u32, new: u32) -> KResult<u32> {
    let f = fd_file(old)?;
    if new as usize >= NOFILE {
        return Err(EBADF);
    }
    if old == new {
        return Ok(new);
    }
    let p = proc::current();
    p.files[new as usize] = Some(f);
    p.cloexec &= !(1 << new);
    Ok(new)
}

fn sys_pipe(fds: u32) -> KResult<u32> {
    let p = proc::current();
    let r = p.alloc_fd(0)?;
    let (rf, wf) = pipe::new();
    p.files[r as usize] = Some(rf);
    let w = match p.alloc_fd(0) {
        Ok(w) => w,
        Err(e) => {
            p.files[r as usize] = None;
            return Err(e);
        }
    };
    p.files[w as usize] = Some(wf);
    p.cloexec &= !((1 << r) | (1 << w));
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&r.to_be_bytes());
    b[4..8].copy_from_slice(&w.to_be_bytes());
    if let Err(e) = p.space().write(fds, &b, false) {
        p.files[r as usize] = None;
        p.files[w as usize] = None;
        return Err(e);
    }
    Ok(0)
}

fn sys_times(buf: u32) -> KResult<u32> {
    let p = proc::current();
    let t = Tms { utime: p.utime, stime: p.stime, cutime: p.cutime, cstime: p.cstime };
    if buf != 0 {
        put(buf, &t)?;
    }
    Ok(timer::ticks() as u32)
}

fn sys_sigaction(sig: u32, act: u32, oact: u32) -> KResult<u32> {
    let new = if act != 0 { Some(get::<SigAction>(act)?) } else { None };
    let old = signal::sigaction(sig, new)?;
    if oact != 0 {
        put(oact, &old)?;
    }
    Ok(0)
}

fn sys_sigprocmask(how: u32, set: u32, oset: u32) -> KResult<u32> {
    let s = if set != 0 { Some(proc::current().space().read_u32(set)?) } else { None };
    let old = signal::sigprocmask(how, s)?;
    if oset != 0 {
        proc::current().space().write_u32(oset, old)?;
    }
    Ok(0)
}

fn sys_ioctl(fd: u32, req: u32, arg: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    if let Some(n) = file::is_tty(&f) {
        return tty::ioctl(n, req, arg);
    }
    if req == azsys::tty::FIONREAD {
        if let file::Kind::PipeRead(p) = &f.borrow().kind {
            let n = pipe::available(p) as u32;
            proc::current().space().write_u32(arg, n)?;
            return Ok(0);
        }
    }
    Err(ENOTTY)
}

fn sys_fcntl(fd: u32, cmd: u32, arg: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    let p = proc::current();
    match cmd {
        F_DUPFD => sys_dup(fd, arg),
        F_GETFD => Ok((p.cloexec >> fd) & 1),
        F_SETFD => {
            if arg & FD_CLOEXEC != 0 {
                p.cloexec |= 1 << fd;
            } else {
                p.cloexec &= !(1 << fd);
            }
            Ok(0)
        }
        F_GETFL => Ok(f.borrow().flags),
        F_SETFL => {
            let mut o = f.borrow_mut();
            o.flags = (o.flags & !(O_APPEND | O_NONBLOCK)) | (arg & (O_APPEND | O_NONBLOCK));
            Ok(0)
        }
        _ => Err(EINVAL),
    }
}

fn sys_setpgid(pid: u32, pgid: u32) -> KResult<u32> {
    let me = proc::current();
    let (my_pid, my_sid) = (me.pid, me.sid);
    let pid = if pid == 0 { my_pid } else { pid };
    let target = proc::find(pid).ok_or(ESRCH)?;
    if target.pid != my_pid && target.ppid != my_pid {
        return Err(ESRCH);
    }
    if target.sid != my_sid || target.sid == target.pid {
        return Err(EPERM);
    }
    target.pgid = if pgid == 0 { pid } else { pgid };
    Ok(0)
}

fn sys_getpgid(pid: u32) -> KResult<u32> {
    if pid == 0 {
        return Ok(proc::current().pgid);
    }
    Ok(proc::find(pid).ok_or(ESRCH)?.pgid)
}

fn sys_setsid() -> KResult<u32> {
    let p = proc::current();
    let pid = p.pid;
    if proc::table().iter().flatten().any(|q| q.pgid == pid && q.pid != pid) || p.pgid == pid && p.sid == pid {
        return Err(EPERM);
    }
    p.sid = pid;
    p.pgid = pid;
    p.tty = 0;
    Ok(pid)
}

fn sys_getcwd(buf: u32, size: u32) -> KResult<u32> {
    let mut cwd = fs::getcwd()?;
    if cwd.len() + 1 > size as usize {
        return Err(ERANGE);
    }
    cwd.push(0);
    proc::current().space().write(buf, &cwd, false)?;
    Ok(cwd.len() as u32 - 1)
}

fn sys_getdents(fd: u32, buf: u32, len: u32) -> KResult<u32> {
    let f = fd_file(fd)?;
    let data = file::getdents(&f, (len as usize).min(file::MAX_IO))?;
    proc::current().space().write(buf, &data, false)?;
    Ok(data.len() as u32)
}

fn sys_readlink(p_addr: u32, buf: u32, size: u32) -> KResult<u32> {
    let ip = fs::lookup(&path(p_addr)?, false)?;
    if !ip.borrow().is_symlink() {
        return Err(EINVAL);
    }
    let t = fs::read_link(&ip)?;
    let n = t.len().min(size as usize);
    proc::current().space().write(buf, &t[..n], false)?;
    Ok(n as u32)
}

fn sys_uname(buf: u32) -> KResult<u32> {
    let mut u = Utsname { sysname: [0; 32], nodename: [0; 32], release: [0; 32], version: [0; 64], machine: [0; 32] };
    util::set_cstr(&mut u.sysname, b"az030");
    util::set_cstr(&mut u.nodename, b"az030");
    util::set_cstr(&mut u.release, crate::VERSION.as_bytes());
    util::set_cstr(&mut u.version, b"az030 UNIX for the MC68030");
    util::set_cstr(&mut u.machine, b"m68k");
    put(buf, &u)?;
    Ok(0)
}

fn sys_reboot(cmd: u32) -> KResult<u32> {
    if proc::current().euid != 0 {
        return Err(EPERM);
    }
    if !matches!(cmd, REBOOT_RESTART | REBOOT_HALT | REBOOT_POWEROFF) {
        return Err(EINVAL);
    }
    fs::shutdown();
    arch::irq_off();
    match cmd {
        REBOOT_RESTART => {
            kprintln!("Restarting system.");
            arch::restart()
        }
        REBOOT_POWEROFF => {
            kprintln!("Power down.");
            timer::power_off();
        }
        _ => {}
    }
    kprintln!("System halted.");
    arch::halt()
}

fn sys_procinfo(index: u32, buf: u32) -> KResult<u32> {
    let Some(p) = proc::table().iter().flatten().nth(index as usize) else { return Ok(0) };
    let mut info = ProcInfo {
        pid: p.pid,
        ppid: p.ppid,
        pgid: p.pgid,
        sid: p.sid,
        uid: p.uid,
        state: match p.state {
            State::Runnable => PS_RUN,
            State::Sleeping => PS_SLEEP,
            State::Stopped => PS_STOP,
            State::Zombie => PS_ZOMBIE,
        },
        mem_kb: p.mem_kb(),
        utime: p.utime,
        stime: p.stime,
        start: p.start,
        tty: p.tty,
        name: [0; 32],
    };
    info.name = p.name;
    put(buf, &info)?;
    Ok(1)
}

fn sys_sysinfo(buf: u32) -> KResult<u32> {
    let m = mem::stats();
    let runnable = proc::table().iter().flatten().filter(|p| p.state == State::Runnable).count() as u32;
    let s = SysInfo {
        uptime: timer::uptime(),
        total_kb: m.total_kb,
        free_kb: m.free_kb,
        kernel_kb: m.kernel_kb + m.heap_used_kb,
        buffers_kb: crate::bio::cached_kb(),
        procs: proc::count() as u32,
        hz: HZ,
        load: runnable,
    };
    put(buf, &s)?;
    Ok(0)
}

fn sys_alarm(secs: u32) -> KResult<u32> {
    let p = proc::current();
    let now = timer::ticks();
    let left = if p.alarm_at > now { ((p.alarm_at - now) / HZ as u64) as u32 + 1 } else { 0 };
    p.alarm_at = if secs == 0 { 0 } else { now + secs as u64 * HZ as u64 };
    Ok(left)
}

fn sys_mountinfo(index: u32, buf: u32) -> KResult<u32> {
    let Some(m) = fs::mounts().get(index as usize) else { return Ok(0) };
    let mut info = MountInfo { dev: m.dev, flags: m.read_only as u32, path: [0; 64], source: [0; 32] };
    util::set_cstr(&mut info.path, m.path.as_bytes());
    util::set_cstr(&mut info.source, m.source.as_bytes());
    put(buf, &info)?;
    Ok(1)
}
