//! The console terminal: line discipline on top of the UART.
//!
//! Input arrives by interrupt (UART receive, IPL 4) and is cooked here: canonical
//! line editing with echo, erase/kill/word-erase, ^C/^\/^Z signals to the
//! foreground process group, ^D end of file. Raw mode (no ICANON) honours VMIN and
//! VTIME. Output translates \n to \r\n when OPOST|ONLCR is set.

use crate::console::{self, UART_DATA, UART_RX_VALID, UART_STATUS};
use crate::proc::{self, Proc};
use crate::util::Global;
use crate::vm::KResult;
use crate::{arch, signal, timer};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use azsys::errno::*;
use azsys::sig::*;
use azsys::tty::*;
use azsys::{Termios, Winsize};

const LINE_MAX: usize = 255;
const RAW_MAX: usize = 4096;

pub struct Tty {
    pub termios: Termios,
    pub winsize: Winsize,
    /// The line being edited (canonical mode).
    line: Vec<u8>,
    /// Completed input: one element per line (an empty element is end of file).
    cooked: VecDeque<Vec<u8>>,
    /// Input in raw mode.
    raw: VecDeque<u8>,
    /// Foreground process group and session.
    pub pgrp: u32,
    pub sid: u32,
    /// Interrupts are delivering input.
    irq: bool,
}

fn default_termios() -> Termios {
    let mut cc = [0u8; NCCS];
    cc[VINTR] = 3;
    cc[VQUIT] = 0x1C;
    cc[VERASE] = 0x7F;
    cc[VKILL] = 0x15;
    cc[VEOF] = 4;
    cc[VMIN] = 1;
    cc[VTIME] = 0;
    cc[VSUSP] = 0x1A;
    cc[VWERASE] = 0x17;
    Termios {
        iflag: ICRNL,
        oflag: OPOST | ONLCR,
        cflag: 0,
        lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | IEXTEN,
        cc,
    }
}

static TTY: Global<Option<Tty>> = Global::new(None);

fn tty() -> &'static mut Tty {
    TTY.get().as_mut().unwrap()
}

fn chan() -> usize {
    TTY.get() as *const _ as usize
}

pub fn init() {
    *TTY.get() = Some(Tty {
        termios: default_termios(),
        winsize: Winsize { rows: 24, cols: 80, xpixel: 0, ypixel: 0 },
        line: Vec::new(),
        cooked: VecDeque::new(),
        raw: VecDeque::new(),
        pgrp: 0,
        sid: 0,
        irq: false,
    });
    // receive interrupts come from the timer block's interrupt enable register
    if timer::present() {
        arch::wr(timer::INTEN, arch::rd(timer::INTEN) | 2);
        tty().irq = true;
    }
    // drop anything typed during boot (e.g. the key that stopped autoboot)
    while arch::rd(UART_STATUS) & UART_RX_VALID != 0 {
        arch::rd(UART_DATA);
    }
}

pub fn set_session(sid: u32, pgrp: u32) {
    let t = tty();
    t.sid = sid;
    t.pgrp = pgrp;
}

/// Opening the terminal makes it the controlling terminal of a session leader
/// that has none.
pub fn open(p: &mut Proc, minor: u32, flags: u32) -> KResult<()> {
    if minor != 0 {
        return Err(ENXIO);
    }
    let t = tty();
    if flags & azsys::flags::O_NOCTTY == 0 && p.tty == 0 && p.sid == p.pid && t.sid == 0 {
        t.sid = p.sid;
        t.pgrp = p.pgid;
        p.tty = azsys::makedev(azsys::dev::TTY_MAJOR, 0);
    }
    Ok(())
}

pub fn process_exited(p: &Proc) {
    let t = tty();
    if t.sid == p.pid {
        // the session leader is gone: hang up the foreground group
        let pgrp = t.pgrp;
        t.sid = 0;
        t.pgrp = 0;
        for pid in proc::group_members(pgrp) {
            if let Some(q) = proc::find(pid) {
                if q.pid != p.pid {
                    signal::send(q, SIGHUP);
                    signal::send(q, SIGCONT);
                }
            }
        }
    }
}

fn echo(c: u8) {
    let t = &tty().termios;
    if c == b'\n' {
        if t.oflag & OPOST != 0 && t.oflag & ONLCR != 0 {
            console::putc(b'\r');
        }
        console::putc(b'\n');
    } else if c < 0x20 && c != b'\t' && t.lflag & ECHOCTL != 0 {
        console::putc(b'^');
        console::putc(c + 0x40);
    } else if c == 0x7F && t.lflag & ECHOCTL != 0 {
        console::putc(b'^');
        console::putc(b'?');
    } else {
        console::putc(c);
    }
}

fn erase_echo(c: u8) {
    let width = if (c < 0x20 && c != b'\t') || c == 0x7F { 2 } else { 1 };
    for _ in 0..width {
        console::putc(8);
        console::putc(b' ');
        console::putc(8);
    }
}

fn signal_group(sig: u32) {
    let pgrp = tty().pgrp;
    if pgrp == 0 {
        return;
    }
    for pid in proc::group_members(pgrp) {
        if let Some(p) = proc::find(pid) {
            signal::send(p, sig);
        }
    }
}

/// Process one received character (interrupt context).
fn input(mut c: u8) {
    let t = tty();
    let tio = t.termios;
    if tio.iflag & ISTRIP != 0 {
        c &= 0x7F;
    }
    if c == b'\r' {
        if tio.iflag & IGNCR != 0 {
            return;
        }
        if tio.iflag & ICRNL != 0 {
            c = b'\n';
        }
    } else if c == b'\n' && tio.iflag & INLCR != 0 {
        c = b'\r';
    }
    if tio.lflag & ISIG != 0 {
        let sig = if c == tio.cc[VINTR] {
            SIGINT
        } else if c == tio.cc[VQUIT] {
            SIGQUIT
        } else if c == tio.cc[VSUSP] {
            SIGTSTP
        } else {
            0
        };
        if sig != 0 {
            if tio.lflag & ECHO != 0 {
                echo(c);
                echo(b'\n');
            }
            t.line.clear();
            t.cooked.clear();
            t.raw.clear();
            signal_group(sig);
            return;
        }
    }
    if tio.lflag & ICANON == 0 {
        if t.raw.len() < RAW_MAX {
            t.raw.push_back(c);
        }
        if tio.lflag & ECHO != 0 {
            echo(c);
        }
        proc::wakeup(chan());
        return;
    }
    let echo_on = tio.lflag & ECHO != 0;
    if c == tio.cc[VERASE] || c == 8 {
        if let Some(x) = t.line.pop() {
            if echo_on && tio.lflag & ECHOE != 0 {
                erase_echo(x);
            }
        }
    } else if c == tio.cc[VKILL] {
        while let Some(x) = t.line.pop() {
            if echo_on {
                erase_echo(x);
            }
        }
    } else if c == tio.cc[VWERASE] {
        while t.line.last() == Some(&b' ') {
            let x = t.line.pop().unwrap();
            if echo_on {
                erase_echo(x);
            }
        }
        while t.line.last().is_some_and(|&x| x != b' ') {
            let x = t.line.pop().unwrap();
            if echo_on {
                erase_echo(x);
            }
        }
    } else if c == tio.cc[VEOF] {
        let l = core::mem::take(&mut t.line);
        t.cooked.push_back(l);
        proc::wakeup(chan());
    } else if c == b'\n' {
        t.line.push(b'\n');
        if echo_on || tio.lflag & ECHONL != 0 {
            echo(b'\n');
        }
        let l = core::mem::take(&mut t.line);
        t.cooked.push_back(l);
        proc::wakeup(chan());
    } else if t.line.len() < LINE_MAX {
        t.line.push(c);
        if echo_on {
            echo(c);
        }
    } else {
        console::putc(7); // line full: beep
    }
}

/// Poll the UART (used when there are no receive interrupts).
fn poll() {
    while arch::rd(UART_STATUS) & UART_RX_VALID != 0 {
        input(arch::rd(UART_DATA) as u8);
    }
}

/// UART receive interrupt.
pub fn interrupt() {
    poll();
}

fn ready(t: &Tty) -> usize {
    if t.termios.lflag & ICANON != 0 { t.cooked.len() } else { t.raw.len() }
}

pub fn input_pending() -> usize {
    let t = tty();
    if t.termios.lflag & ICANON != 0 { t.cooked.iter().map(|l| l.len()).sum() } else { t.raw.len() }
}

pub fn read(_minor: u32, len: usize, nonblock: bool) -> KResult<Vec<u8>> {
    let sr = arch::irq_save();
    let r = read_inner(len, nonblock);
    arch::irq_restore_sr(sr);
    r
}

fn wait_input(deadline: u64, nonblock: bool) -> KResult<bool> {
    if !tty().irq {
        // no interrupts: poll and yield
        poll();
        if ready(tty()) > 0 {
            return Ok(true);
        }
        if nonblock {
            return Err(EAGAIN);
        }
        proc::yield_cpu();
        poll();
        return Ok(ready(tty()) > 0 || (deadline != 0 && timer::ticks() >= deadline));
    }
    if nonblock {
        return Err(EAGAIN);
    }
    if !proc::sleep_until(chan(), true, deadline) {
        return Err(EINTR);
    }
    Ok(true)
}

fn read_inner(len: usize, nonblock: bool) -> KResult<Vec<u8>> {
    if len == 0 {
        return Ok(Vec::new());
    }
    loop {
        let t = tty();
        if t.termios.lflag & ICANON != 0 {
            if let Some(l) = t.cooked.front_mut() {
                if l.is_empty() {
                    t.cooked.pop_front();
                    return Ok(Vec::new()); // end of file
                }
                let n = len.min(l.len());
                let out: Vec<u8> = l.drain(..n).collect();
                if l.is_empty() {
                    t.cooked.pop_front();
                }
                return Ok(out);
            }
            wait_input(0, nonblock)?;
            continue;
        }
        let vmin = t.termios.cc[VMIN] as usize;
        let vtime = t.termios.cc[VTIME] as u32;
        let want = vmin.min(len).max(if vmin == 0 { 0 } else { 1 });
        if t.raw.len() >= want.max(1) || (vmin == 0 && vtime == 0) {
            let n = len.min(t.raw.len());
            return Ok(t.raw.drain(..n).collect());
        }
        let deadline = if vtime > 0 && (vmin == 0 || !t.raw.is_empty()) { timer::ticks() + timer::ms_to_ticks(vtime * 100) } else { 0 };
        let before = t.raw.len();
        let got = wait_input(deadline, nonblock)?;
        let t = tty();
        if deadline != 0 && t.raw.len() == before && timer::ticks() >= deadline {
            // timed out: return whatever there is (maybe nothing)
            let n = len.min(t.raw.len());
            return Ok(t.raw.drain(..n).collect());
        }
        let _ = got;
    }
}

pub fn write(_minor: u32, data: &[u8]) -> KResult<usize> {
    let onlcr = {
        let t = &tty().termios;
        t.oflag & OPOST != 0 && t.oflag & ONLCR != 0
    };
    for &b in data {
        if b == b'\n' && onlcr {
            console::putc(b'\r');
        }
        console::putc(b);
    }
    Ok(data.len())
}

pub fn ioctl(_minor: u32, req: u32, arg: u32) -> KResult<u32> {
    let p = proc::current();
    let t = tty();
    match req {
        TCGETS => {
            let b = unsafe { core::slice::from_raw_parts(&t.termios as *const Termios as *const u8, core::mem::size_of::<Termios>()) };
            p.space().write(arg, b, false)?;
            Ok(0)
        }
        TCSETS | TCSETSW | TCSETSF => {
            let mut n = Termios::default();
            let b = unsafe { core::slice::from_raw_parts_mut(&mut n as *mut Termios as *mut u8, core::mem::size_of::<Termios>()) };
            p.space().read(arg, b)?;
            let sr = arch::irq_save();
            let was_canon = t.termios.lflag & ICANON != 0;
            t.termios = n;
            if req == TCSETSF {
                t.line.clear();
                t.cooked.clear();
                t.raw.clear();
            } else if was_canon && n.lflag & ICANON == 0 {
                // pending cooked input becomes raw input
                for l in t.cooked.drain(..) {
                    t.raw.extend(l);
                }
                t.raw.extend(t.line.drain(..));
            }
            arch::irq_restore_sr(sr);
            Ok(0)
        }
        TIOCGWINSZ => {
            let w = t.winsize;
            let mut b = [0u8; 8];
            b[0..2].copy_from_slice(&w.rows.to_be_bytes());
            b[2..4].copy_from_slice(&w.cols.to_be_bytes());
            p.space().write(arg, &b, false)?;
            Ok(0)
        }
        TIOCSWINSZ => {
            let mut b = [0u8; 8];
            p.space().read(arg, &mut b)?;
            t.winsize.rows = u16::from_be_bytes([b[0], b[1]]);
            t.winsize.cols = u16::from_be_bytes([b[2], b[3]]);
            signal_group(SIGWINCH);
            Ok(0)
        }
        TIOCGPGRP => {
            p.space().write_u32(arg, t.pgrp)?;
            Ok(0)
        }
        TIOCSPGRP => {
            let g = p.space().read_u32(arg)?;
            t.pgrp = g;
            Ok(0)
        }
        FIONREAD => {
            let n = input_pending() as u32;
            p.space().write_u32(arg, n)?;
            Ok(0)
        }
        _ => Err(ENOTTY),
    }
}
