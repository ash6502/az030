//! Terminal control: termios, window size, raw mode, foreground process group.

use crate::io::Fd;
use crate::sys::{self, nr, Result};
pub use azsys::tty::*;
pub use azsys::{Termios, Winsize};

pub fn get_attr(fd: Fd) -> Result<Termios> {
    let mut t = Termios::default();
    sys::call3(nr::IOCTL, fd, TCGETS, &mut t as *mut _ as u32)?;
    Ok(t)
}

pub fn set_attr(fd: Fd, t: &Termios) -> Result<()> {
    sys::call3(nr::IOCTL, fd, TCSETSW, t as *const _ as u32).map(|_| ())
}

/// The terminal size; 24x80 if unknown.
pub fn size(fd: Fd) -> (u16, u16) {
    let mut w = Winsize::default();
    match sys::call3(nr::IOCTL, fd, TIOCGWINSZ, &mut w as *mut _ as u32) {
        Ok(_) if w.rows > 0 && w.cols > 0 => (w.rows, w.cols),
        _ => (24, 80),
    }
}

pub fn set_size(fd: Fd, rows: u16, cols: u16) -> Result<()> {
    let w = Winsize { rows, cols, xpixel: 0, ypixel: 0 };
    sys::call3(nr::IOCTL, fd, TIOCSWINSZ, &w as *const _ as u32).map(|_| ())
}

pub fn foreground(fd: Fd) -> Result<u32> {
    let mut g = 0u32;
    sys::call3(nr::IOCTL, fd, TIOCGPGRP, &mut g as *mut u32 as u32)?;
    Ok(g)
}

pub fn set_foreground(fd: Fd, pgid: u32) -> Result<()> {
    sys::call3(nr::IOCTL, fd, TIOCSPGRP, &pgid as *const u32 as u32).map(|_| ())
}

/// Bytes waiting to be read.
pub fn pending(fd: Fd) -> u32 {
    let mut n = 0u32;
    match sys::call3(nr::IOCTL, fd, FIONREAD, &mut n as *mut u32 as u32) {
        Ok(_) => n,
        Err(_) => 0,
    }
}

/// Raw mode: no echo, no line editing, no signals from keys, no output processing
/// except NL -> CRNL. Restores the old settings when dropped.
pub struct RawMode {
    fd: Fd,
    saved: Termios,
}

impl RawMode {
    pub fn enter(fd: Fd) -> Result<RawMode> {
        let saved = get_attr(fd)?;
        let mut t = saved;
        t.iflag &= !(ICRNL | INLCR | IGNCR | IXON | ISTRIP);
        t.lflag &= !(ICANON | ECHO | ECHOE | ECHOK | ECHONL | ISIG | IEXTEN);
        t.cc[VMIN] = 1;
        t.cc[VTIME] = 0;
        set_attr(fd, &t)?;
        Ok(RawMode { fd, saved })
    }

    /// Temporarily go back to the saved mode (e.g. to run a subcommand).
    pub fn suspend(&self) {
        let _ = set_attr(self.fd, &self.saved);
    }
    pub fn resume(&self) {
        if let Ok(mut t) = get_attr(self.fd) {
            t.iflag &= !(ICRNL | INLCR | IGNCR | IXON | ISTRIP);
            t.lflag &= !(ICANON | ECHO | ECHOE | ECHOK | ECHONL | ISIG | IEXTEN);
            t.cc[VMIN] = 1;
            t.cc[VTIME] = 0;
            let _ = set_attr(self.fd, &t);
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = set_attr(self.fd, &self.saved);
    }
}
