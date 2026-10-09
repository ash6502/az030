//! Processes and the scheduler.
//!
//! The kernel is non-preemptive: a process runs in kernel mode until it blocks or
//! returns to user mode, where the timer's reschedule request is honoured. Sleeping
//! is done on "channels" (addresses of the thing waited for), UNIX style:
//!
//! ```ignore
//! let sr = arch::irq_save();
//! while !ready() { proc::sleep(chan, true); }
//! arch::irq_restore_sr(sr);
//! ```

use crate::arch::{self, FPU_AREA};
use crate::file::FileRef;
use crate::fs::InodeRef;
use crate::signal::SigState;
use crate::trap::TrapFrame;
use crate::util::{set_cstr, Global};
use crate::vm::{AddressSpace, KResult};
use crate::{exec, file, fs, timer, vm};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use azsys::errno::*;
use azsys::sig::*;

pub const NPROC: usize = 64;
pub const KSTACK_SIZE: usize = 16 * 1024;
pub const NOFILE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Runnable,
    Sleeping,
    Stopped,
    Zombie,
}

pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    pub sid: u32,
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub state: State,
    /// What a sleeping process waits for.
    pub chan: usize,
    /// Can a signal interrupt the sleep?
    pub intr: bool,
    /// Tick at which a sleep times out (0 = never).
    pub wake_at: u64,
    /// wait() status: exit code << 8, or the terminating signal.
    pub status: u32,
    /// A stop has been reported to the parent already.
    pub stop_reported: bool,
    kstack: Vec<u8>,
    pub ksp: u32,
    pub fpu: [u8; FPU_AREA],
    pub vm: Option<AddressSpace>,
    pub files: [Option<FileRef>; NOFILE],
    pub cloexec: u32,
    pub cwd: Option<InodeRef>,
    pub umask: u32,
    pub name: [u8; 32],
    pub sig: SigState,
    pub utime: u32,
    pub stime: u32,
    pub cutime: u32,
    pub cstime: u32,
    /// Uptime (seconds) at creation.
    pub start: u32,
    /// Tick at which SIGALRM fires (0 = no alarm).
    pub alarm_at: u64,
    /// Controlling terminal (device number, 0 = none).
    pub tty: u32,
    /// Trap frame of the current kernel entry from user mode.
    pub tf: *mut TrapFrame,
    pub in_kernel: bool,
}

impl Proc {
    pub fn name(&self) -> &str {
        crate::util::cstr(&self.name)
    }

    pub fn set_name(&mut self, n: &[u8]) {
        set_cstr(&mut self.name, n);
    }

    pub fn space(&self) -> &AddressSpace {
        self.vm.as_ref().expect("process has no address space")
    }

    pub fn space_mut(&mut self) -> &mut AddressSpace {
        self.vm.as_mut().expect("process has no address space")
    }

    pub fn grow_stack(&mut self, addr: u32) -> bool {
        self.vm.as_mut().is_some_and(|v| v.grow_stack(addr))
    }

    pub fn kstack_top(&self) -> u32 {
        self.kstack.as_ptr() as u32 + KSTACK_SIZE as u32
    }

    pub fn file(&self, fd: u32) -> KResult<FileRef> {
        self.files.get(fd as usize).and_then(|f| f.clone()).ok_or(EBADF)
    }

    pub fn alloc_fd(&self, from: u32) -> KResult<u32> {
        (from as usize..NOFILE).find(|&i| self.files[i].is_none()).map(|i| i as u32).ok_or(EMFILE)
    }

    pub fn mem_kb(&self) -> u32 {
        self.vm.as_ref().map_or(0, |v| v.pages * 4) + (KSTACK_SIZE / 1024) as u32
    }
}

static PROCS: Global<[Option<Box<Proc>>; NPROC]> = Global::new([const { None }; NPROC]);
static CURRENT: Global<usize> = Global::new(usize::MAX);
static NEXT_PID: Global<u32> = Global::new(1);

pub fn table() -> &'static mut [Option<Box<Proc>>; NPROC] {
    PROCS.get()
}

pub fn current() -> &'static mut Proc {
    let i = *CURRENT.get();
    PROCS.get()[i].as_mut().expect("no current process")
}

pub fn current_opt() -> Option<&'static Proc> {
    let i = *CURRENT.get();
    PROCS.get().get(i)?.as_deref()
}

pub fn find(pid: u32) -> Option<&'static mut Proc> {
    PROCS.get().iter_mut().flatten().find(|p| p.pid == pid).map(|b| &mut **b)
}

pub fn count() -> usize {
    PROCS.get().iter().flatten().count()
}

fn new_pid() -> u32 {
    let n = NEXT_PID.get();
    loop {
        let pid = *n;
        *n = if *n >= 30000 { 2 } else { *n + 1 };
        if find(pid).is_none() {
            return pid;
        }
    }
}

fn blank(pid: u32) -> Box<Proc> {
    Box::new(Proc {
        pid,
        ppid: 0,
        pgid: pid,
        sid: pid,
        uid: 0,
        gid: 0,
        euid: 0,
        egid: 0,
        state: State::Runnable,
        chan: 0,
        intr: false,
        wake_at: 0,
        status: 0,
        stop_reported: false,
        kstack: vec![0u8; KSTACK_SIZE],
        ksp: 0,
        fpu: [0; FPU_AREA],
        vm: None,
        files: [const { None }; NOFILE],
        cloexec: 0,
        cwd: None,
        umask: 0o022,
        name: [0; 32],
        sig: SigState::default(),
        utime: 0,
        stime: 0,
        cutime: 0,
        cstime: 0,
        start: timer::uptime(),
        alarm_at: 0,
        tty: 0,
        tf: core::ptr::null_mut(),
        in_kernel: true,
    })
}

/// Lay out a fresh kernel stack: a trap frame at the top (returned) and below it a
/// switch_context frame that "returns" into ret_to_user.
fn prepare_kstack(p: &mut Proc) -> *mut TrapFrame {
    let top = p.kstack_top();
    let tf = (top - core::mem::size_of::<TrapFrame>() as u32) as *mut TrapFrame;
    let frame = tf as u32 - 48;
    unsafe {
        core::ptr::write_bytes(frame as *mut u8, 0, 48 + core::mem::size_of::<TrapFrame>());
        *((frame + 44) as *mut u32) = arch::ret_to_user as usize as u32;
    }
    p.ksp = frame;
    tf
}

fn insert(p: Box<Proc>) -> KResult<usize> {
    let sr = arch::irq_save();
    let t = PROCS.get();
    let r = match t.iter().position(|s| s.is_none()) {
        Some(i) => {
            t[i] = Some(p);
            Ok(i)
        }
        None => Err(EAGAIN),
    };
    arch::irq_restore_sr(sr);
    r
}

/// Create process 1 running the configured init program, and start scheduling.
pub fn start_init() -> ! {
    let mut p = blank(*NEXT_PID.get());
    *NEXT_PID.get() += 1;
    p.cwd = Some(fs::root_inode());
    p.tty = azsys::makedev(azsys::dev::TTY_MAJOR, 0);
    // fds 0, 1, 2 on the console
    let con = file::open_path(&mut p, "/dev/console", azsys::flags::O_RDWR, 0).expect("cannot open /dev/console");
    p.files[0] = Some(con.clone());
    p.files[1] = Some(con.clone());
    p.files[2] = Some(con);
    let tf = prepare_kstack(&mut p);
    let init = crate::config().init.clone();
    let argv = vec![init.clone().into_bytes()];
    let envp = vec![b"PATH=/bin:/sbin:/usr/bin".to_vec(), b"HOME=/".to_vec(), b"TERM=vt100".to_vec()];
    if let Err(e) = exec::exec(&mut p, unsafe { &mut *tf }, &init, argv, envp) {
        panic!("cannot run {}: {}", init, message(e));
    }
    crate::tty::set_session(p.sid, p.pgid);
    let slot = insert(p).unwrap();
    let mut dummy = 0u32;
    *CURRENT.get() = slot;
    let p = current();
    p.space().activate();
    kprintln!("Starting {} ...", crate::config().init);
    unsafe { arch::switch_context(&mut dummy, p.ksp, core::ptr::null_mut(), p.fpu.as_ptr()) };
    unreachable!()
}

/// Called on every entry from user mode.
pub fn enter_kernel(tf: &mut TrapFrame) {
    let p = current();
    p.tf = tf;
    p.in_kernel = true;
}

pub fn leave_kernel() {
    current().in_kernel = false;
}

/// fork(): duplicate the current process. Returns the child's pid.
pub fn fork(tf: &TrapFrame) -> KResult<u32> {
    let parent = current();
    let pid = new_pid();
    let mut c = blank(pid);
    c.ppid = parent.pid;
    c.pgid = parent.pgid;
    c.sid = parent.sid;
    c.uid = parent.uid;
    c.gid = parent.gid;
    c.euid = parent.euid;
    c.egid = parent.egid;
    c.vm = Some(parent.space().duplicate()?);
    c.files = parent.files.clone();
    c.cloexec = parent.cloexec;
    c.cwd = parent.cwd.clone();
    c.umask = parent.umask;
    c.name = parent.name;
    c.sig = parent.sig.fork_copy();
    c.tty = parent.tty;
    c.fpu = parent.fpu;
    let ctf = prepare_kstack(&mut c);
    unsafe {
        core::ptr::copy_nonoverlapping(tf as *const TrapFrame, ctf, 1);
        (*ctf).d[0] = 0; // fork() returns 0 in the child
        (*ctf).fv = 0; // format $0 frame
    }
    insert(c)?;
    Ok(pid)
}

/// Terminate the current process with a wait status (exit code << 8, or a signal).
pub fn exit_current(status: i32) -> ! {
    let p = current();
    if p.pid == 1 {
        panic!("init exited (status {:#x})", status);
    }
    for f in p.files.iter_mut() {
        *f = None;
    }
    p.cwd = None;
    vm::activate_none();
    p.vm = None;
    p.status = status as u32;
    let pid = p.pid;
    let ppid = p.ppid;
    let (ut, st) = (p.utime + p.cutime, p.stime + p.cstime);
    // orphans go to init
    let mut zombie_orphan = false;
    for q in PROCS.get().iter_mut().flatten() {
        if q.ppid == pid {
            q.ppid = 1;
            zombie_orphan |= q.state == State::Zombie;
        }
    }
    if let Some(parent) = find(ppid) {
        parent.cutime += ut;
        parent.cstime += st;
        crate::signal::send(parent, SIGCHLD);
        wakeup(wait_chan(ppid));
    }
    if zombie_orphan {
        wakeup(wait_chan(1));
    }
    crate::tty::process_exited(current());
    let sr = arch::irq_save();
    current().state = State::Zombie;
    schedule();
    arch::irq_restore_sr(sr);
    unreachable!("zombie scheduled")
}

fn wait_chan(pid: u32) -> usize {
    0x5741_0000 + pid as usize
}

/// waitpid(): returns (pid, status) of a child that changed state, or (0, 0) with
/// WNOHANG when none has yet.
pub fn wait(pid: i32, options: u32) -> KResult<(u32, u32)> {
    let me = current().pid;
    let mypgid = current().pgid;
    let matches = |q: &Proc| q.ppid == me && (pid == -1 || (pid > 0 && q.pid == pid as u32) || (pid == 0 && q.pgid == mypgid) || (pid < -1 && q.pgid == (-pid) as u32));
    let sr = arch::irq_save();
    loop {
        let mut any = false;
        let t = PROCS.get();
        for slot in t.iter_mut() {
            let Some(q) = slot.as_mut() else { continue };
            if !matches(q) {
                continue;
            }
            any = true;
            if q.state == State::Zombie {
                let r = (q.pid, q.status);
                *slot = None; // frees the kernel stack and everything else
                arch::irq_restore_sr(sr);
                return Ok(r);
            }
            if q.state == State::Stopped && !q.stop_reported && options & azsys::flags::WUNTRACED != 0 {
                q.stop_reported = true;
                let r = (q.pid, 0x7F | (q.sig.stop_sig << 8));
                arch::irq_restore_sr(sr);
                return Ok(r);
            }
        }
        if !any {
            arch::irq_restore_sr(sr);
            return Err(ECHILD);
        }
        if options & azsys::flags::WNOHANG != 0 {
            arch::irq_restore_sr(sr);
            return Ok((0, 0));
        }
        if !sleep(wait_chan(me), true) {
            arch::irq_restore_sr(sr);
            return Err(EINTR);
        }
    }
}

/// Block on `chan` until woken (or until a signal arrives, if `intr`). Returns false
/// if a signal interrupted the sleep. Call with interrupts masked, after checking the
/// condition being waited for.
pub fn sleep(chan: usize, intr: bool) -> bool {
    sleep_until(chan, intr, 0)
}

/// Like sleep(), but also wakes at tick `deadline` (0 = no timeout).
pub fn sleep_until(chan: usize, intr: bool, deadline: u64) -> bool {
    let p = current();
    if intr && p.sig.has_deliverable() {
        return false;
    }
    p.chan = chan;
    p.intr = intr;
    p.wake_at = deadline;
    p.state = State::Sleeping;
    schedule();
    let p = current();
    p.wake_at = 0;
    !(intr && p.sig.has_deliverable())
}

pub fn wakeup(chan: usize) {
    let sr = arch::irq_save();
    for p in PROCS.get().iter_mut().flatten() {
        if p.state == State::Sleeping && p.chan == chan {
            p.state = State::Runnable;
        }
    }
    arch::irq_restore_sr(sr);
}

/// Wake a process for a signal (if its sleep is interruptible).
pub fn wake_for_signal(p: &mut Proc) {
    let sr = arch::irq_save();
    if p.state == State::Sleeping && p.intr {
        p.state = State::Runnable;
    }
    arch::irq_restore_sr(sr);
}

/// Called by the timer: wake sleepers whose timeout passed, fire alarms.
pub fn tick(now: u64) {
    for p in PROCS.get().iter_mut().flatten() {
        if p.state == State::Sleeping && p.wake_at != 0 && p.wake_at <= now {
            p.state = State::Runnable;
        }
        if p.alarm_at != 0 && p.alarm_at <= now {
            p.alarm_at = 0;
            crate::signal::send(p, SIGALRM);
        }
    }
}

/// Give up the CPU but stay runnable.
pub fn yield_cpu() {
    let sr = arch::irq_save();
    schedule();
    arch::irq_restore_sr(sr);
}

pub fn any_other_runnable() -> bool {
    let cur = *CURRENT.get();
    PROCS.get().iter().enumerate().any(|(i, p)| i != cur && p.as_ref().is_some_and(|p| p.state == State::Runnable))
}

/// Switch to the next runnable process (round robin). Call with interrupts masked.
pub fn schedule() {
    timer::clear_resched();
    let t = PROCS.get();
    let cur = *CURRENT.get();
    let next = loop {
        let found = (1..=NPROC)
            .map(|k| (cur + k) % NPROC)
            .find(|&i| t[i].as_ref().is_some_and(|p| p.state == State::Runnable));
        match found {
            Some(i) => break i,
            None => arch::idle(), // nothing to do until an interrupt
        }
    };
    if next == cur {
        return;
    }
    *CURRENT.get() = next;
    let (old_ksp, old_fpu): (*mut u32, *mut u8) = match t[cur].as_mut() {
        Some(o) => (&mut o.ksp, o.fpu.as_mut_ptr()),
        None => (core::ptr::null_mut(), core::ptr::null_mut()),
    };
    let n = t[next].as_mut().unwrap();
    match &n.vm {
        Some(v) => v.activate(),
        None => vm::activate_none(),
    }
    let mut scratch = 0u32;
    let old_ksp = if old_ksp.is_null() { &mut scratch as *mut u32 } else { old_ksp };
    unsafe { arch::switch_context(old_ksp, n.ksp, old_fpu, n.fpu.as_ptr()) };
}

/// Pids of all processes in a process group.
pub fn group_members(pgid: u32) -> Vec<u32> {
    PROCS.get().iter().flatten().filter(|p| p.pgid == pgid && p.state != State::Zombie).map(|p| p.pid).collect()
}
