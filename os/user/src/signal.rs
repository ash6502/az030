//! Signals.

use crate::sys::{self, nr, Result};
pub use azsys::sig::*;
use azsys::SigAction;

unsafe extern "C" {
    fn rt_sigreturn();
}

/// What to do with a signal.
#[derive(Clone, Copy)]
pub enum Handler {
    Default,
    Ignore,
    /// Called with the signal number.
    Func(extern "C" fn(u32)),
}

fn to_raw(h: Handler) -> u32 {
    match h {
        Handler::Default => SIG_DFL,
        Handler::Ignore => SIG_IGN,
        Handler::Func(f) => f as usize as u32,
    }
}

fn from_raw(v: u32) -> Handler {
    match v {
        SIG_DFL => Handler::Default,
        SIG_IGN => Handler::Ignore,
        f => Handler::Func(unsafe { core::mem::transmute::<usize, extern "C" fn(u32)>(f as usize) }),
    }
}

/// Install a handler (restarting interrupted system calls); returns the old one.
pub fn signal(sig: u32, h: Handler) -> Result<Handler> {
    sigaction(sig, h, azsys::SA_RESTART)
}

pub fn sigaction(sig: u32, h: Handler, flags: u32) -> Result<Handler> {
    let act = SigAction { handler: to_raw(h), mask: 0, flags, restorer: rt_sigreturn as unsafe extern "C" fn() as usize as u32 };
    let mut old = SigAction::default();
    sys::call3(nr::SIGACTION, sig, &act as *const _ as u32, &mut old as *mut _ as u32)?;
    Ok(from_raw(old.handler))
}

/// Change the blocked-signal mask (SIG_BLOCK / SIG_UNBLOCK / SIG_SETMASK); returns the old mask.
pub fn sigprocmask(how: u32, set: u32) -> Result<u32> {
    let mut old = 0u32;
    sys::call3(nr::SIGPROCMASK, how, &set as *const u32 as u32, &mut old as *mut u32 as u32)?;
    Ok(old)
}

pub fn mask(sig: u32) -> u32 {
    1 << sig
}

pub fn raise(sig: u32) -> Result<()> {
    crate::process::kill(crate::process::id() as i32, sig)
}

/// Deliver SIGALRM after `secs` seconds (0 cancels); returns the seconds left on the old alarm.
pub fn alarm(secs: u32) -> u32 {
    sys::call1(nr::ALARM, secs).unwrap_or(0)
}

const NAMES: [(u32, &str); 22] = [
    (SIGHUP, "HUP"),
    (SIGINT, "INT"),
    (SIGQUIT, "QUIT"),
    (SIGILL, "ILL"),
    (SIGTRAP, "TRAP"),
    (SIGABRT, "ABRT"),
    (SIGBUS, "BUS"),
    (SIGFPE, "FPE"),
    (SIGKILL, "KILL"),
    (SIGUSR1, "USR1"),
    (SIGSEGV, "SEGV"),
    (SIGUSR2, "USR2"),
    (SIGPIPE, "PIPE"),
    (SIGALRM, "ALRM"),
    (SIGTERM, "TERM"),
    (SIGCHLD, "CHLD"),
    (SIGCONT, "CONT"),
    (SIGSTOP, "STOP"),
    (SIGTSTP, "TSTP"),
    (SIGTTIN, "TTIN"),
    (SIGTTOU, "TTOU"),
    (SIGWINCH, "WINCH"),
];

/// The short name of a signal (`TERM`).
pub fn abbrev(sig: u32) -> Option<&'static str> {
    NAMES.iter().find(|(n, _)| *n == sig).map(|(_, s)| *s)
}

/// All (number, short name) pairs.
pub fn all() -> &'static [(u32, &'static str)] {
    &NAMES
}

/// Parse `TERM`, `SIGTERM`, `term` or `15`.
pub fn from_name(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return if n < NSIG { Some(n) } else { None };
    }
    let up = s.to_ascii_uppercase();
    let up = up.strip_prefix("SIG").unwrap_or(&up);
    NAMES.iter().find(|(_, n)| *n == up).map(|(n, _)| *n)
}
