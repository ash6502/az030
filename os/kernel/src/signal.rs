//! Signals.
//!
//! A caught signal is delivered by building this frame on the user stack and
//! entering the handler with it as the stack:
//!
//! ```text
//! sp+0   return address: the action's restorer, or the trampoline at sp+92
//! sp+4   signal number (the handler's argument)
//! sp+8   saved signal mask
//! sp+12  saved d0-d7, a0-a6, usp, sr, pc (18 words)
//! sp+84  saved format word (unused), padding
//! sp+92  trampoline: moveq #SIGRETURN,d0 / trap #0
//! ```
//!
//! When the handler returns it lands in the restorer, which calls `sigreturn` with
//! the stack pointing at sp+4.

use crate::proc::{self, Proc, State};
use crate::trap::TrapFrame;
use crate::vm::KResult;
use azsys::errno::*;
use azsys::sig::*;
use azsys::{SigAction, SA_NODEFER, SA_RESETHAND};

#[derive(Clone, Copy)]
pub struct SigState {
    pub pending: u32,
    pub blocked: u32,
    pub actions: [SigAction; 32],
    /// Signal that stopped the process (for wait status).
    pub stop_sig: u32,
}

impl Default for SigState {
    fn default() -> Self {
        SigState { pending: 0, blocked: 0, actions: [SigAction::default(); 32], stop_sig: 0 }
    }
}

const UNBLOCKABLE: u32 = (1 << SIGKILL) | (1 << SIGSTOP);
const STOP_SIGS: u32 = (1 << SIGSTOP) | (1 << SIGTSTP) | (1 << SIGTTIN) | (1 << SIGTTOU);

impl SigState {
    pub fn has_deliverable(&self) -> bool {
        self.pending & !(self.blocked & !UNBLOCKABLE) != 0
    }

    /// State inherited by a forked child: same actions and mask, nothing pending.
    pub fn fork_copy(&self) -> SigState {
        SigState { pending: 0, ..*self }
    }

    /// exec(): caught signals revert to the default action.
    pub fn exec_reset(&mut self) {
        for a in self.actions.iter_mut() {
            if a.handler > SIG_IGN {
                *a = SigAction::default();
            }
        }
    }
}

pub fn describe(sig: u32) -> &'static str {
    name(sig)
}

fn default_ignored(sig: u32) -> bool {
    matches!(sig, SIGCHLD | SIGWINCH | SIGCONT)
}

/// Post a signal to a process.
pub fn send(p: &mut Proc, sig: u32) {
    if sig == 0 || sig >= NSIG || p.state == State::Zombie {
        return;
    }
    if sig == SIGKILL || sig == SIGCONT {
        p.sig.pending &= !STOP_SIGS;
        if p.state == State::Stopped {
            p.state = State::Runnable;
        }
    }
    if STOP_SIGS & (1 << sig) != 0 {
        p.sig.pending &= !(1 << SIGCONT);
    }
    let h = p.sig.actions[sig as usize].handler;
    if sig != SIGKILL && sig != SIGSTOP && (h == SIG_IGN || (h == SIG_DFL && default_ignored(sig))) {
        return;
    }
    p.sig.pending |= 1 << sig;
    proc::wake_for_signal(p);
}

/// kill(2) target selection.
pub fn kill(pid: i32, sig: u32) -> KResult<()> {
    if sig >= NSIG {
        return Err(EINVAL);
    }
    let me = proc::current();
    let (my_pid, my_uid, my_pgid) = (me.pid, me.euid, me.pgid);
    let allowed = |q: &Proc| my_uid == 0 || q.uid == my_uid;
    let mut found = false;
    let mut denied = false;
    for slot in proc::table().iter_mut() {
        let Some(q) = slot.as_mut() else { continue };
        let hit = match pid {
            p if p > 0 => q.pid == p as u32,
            0 => q.pgid == my_pgid,
            -1 => q.pid != 1 && q.pid != my_pid,
            p => q.pgid == (-p) as u32,
        };
        if !hit || q.state == State::Zombie {
            continue;
        }
        if !allowed(q) {
            denied = true;
            continue;
        }
        found = true;
        send(q, sig);
    }
    if found {
        Ok(())
    } else if denied {
        Err(EPERM)
    } else {
        Err(ESRCH)
    }
}

/// Deliver one pending signal to the current process on its way back to user mode.
/// Returns true if the caller should check again (the process was stopped and resumed).
pub fn deliver(tf: &mut TrapFrame) -> bool {
    let p = proc::current();
    loop {
        let ready = p.sig.pending & !(p.sig.blocked & !UNBLOCKABLE);
        if ready == 0 {
            return false;
        }
        let sig = ready.trailing_zeros();
        p.sig.pending &= !(1 << sig);
        let act = p.sig.actions[sig as usize];
        let handler = if sig == SIGKILL || sig == SIGSTOP { SIG_DFL } else { act.handler };
        match handler {
            SIG_IGN => continue,
            SIG_DFL => {
                if default_ignored(sig) {
                    continue;
                }
                if STOP_SIGS & (1 << sig) != 0 {
                    stop(p, sig);
                    return true;
                }
                proc::exit_current(sig as i32);
            }
            _ => {
                if setup_frame(p, tf, sig, &act).is_err() {
                    // no room on the user stack: the process cannot continue
                    proc::exit_current(SIGSEGV as i32);
                }
                if act.flags & SA_RESETHAND != 0 {
                    p.sig.actions[sig as usize] = SigAction::default();
                }
                return false;
            }
        }
    }
}

fn stop(p: &mut Proc, sig: u32) {
    p.sig.stop_sig = sig;
    p.stop_reported = false;
    let ppid = p.ppid;
    if let Some(parent) = proc::find(ppid) {
        if parent.sig.actions[SIGCHLD as usize].flags & azsys::SA_NOCLDSTOP == 0 {
            send(parent, SIGCHLD);
        }
    }
    proc::wakeup(0x5741_0000 + ppid as usize);
    let sr = crate::arch::irq_save();
    proc::current().state = State::Stopped;
    proc::schedule();
    crate::arch::irq_restore_sr(sr);
}

const FRAME: u32 = 96;
const TRAMPOLINE: u32 = 92;

fn setup_frame(p: &mut Proc, tf: &mut TrapFrame, sig: u32, act: &SigAction) -> KResult<()> {
    let sp = (tf.usp - FRAME) & !3;
    let vm = p.vm.as_mut().ok_or(EFAULT)?;
    if !vm.check(sp, FRAME, true) && !(vm.grow_stack(sp) && vm.check(sp, FRAME, true)) {
        return Err(EFAULT);
    }
    let mut w = [0u32; (FRAME / 4) as usize];
    w[0] = if act.restorer != 0 { act.restorer } else { sp + TRAMPOLINE };
    w[1] = sig;
    w[2] = p.sig.blocked;
    w[3..11].copy_from_slice(&tf.d);
    w[11..18].copy_from_slice(&tf.a);
    w[18] = tf.usp;
    w[19] = tf.sr as u32;
    w[20] = tf.pc;
    w[21] = tf.fv as u32;
    w[23] = (0x7000 | azsys::nr::SIGRETURN) << 16 | 0x4E40; // moveq #n,d0 ; trap #0
    let mut bytes = [0u8; FRAME as usize];
    for (i, x) in w.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    vm.write(sp, &bytes, false)?;
    let mut mask = act.mask;
    if act.flags & SA_NODEFER == 0 {
        mask |= 1 << sig;
    }
    p.sig.blocked |= mask & !UNBLOCKABLE;
    tf.usp = sp;
    tf.pc = act.handler;
    tf.sr &= 0x00FF & !0x0080; // user mode, no trace
    Ok(())
}

/// sigreturn: restore the context saved by setup_frame. The stack points at the
/// signal number slot (sp+4 of the frame).
pub fn sigreturn(tf: &mut TrapFrame) -> KResult<()> {
    let p = proc::current();
    let base = tf.usp - 4;
    let mut bytes = [0u8; 88];
    p.space().read(base, &mut bytes)?;
    let w = |i: usize| u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    p.sig.blocked = w(2) & !UNBLOCKABLE;
    for i in 0..8 {
        tf.d[i] = w(3 + i);
    }
    for i in 0..7 {
        tf.a[i] = w(11 + i);
    }
    tf.usp = w(18);
    tf.sr = (w(19) & 0x00FF) as u16 & !0x0080; // only the condition codes come back
    tf.pc = w(20);
    Ok(())
}

pub fn sigaction(sig: u32, new: Option<SigAction>) -> KResult<SigAction> {
    if sig == 0 || sig >= NSIG {
        return Err(EINVAL);
    }
    let p = proc::current();
    let old = p.sig.actions[sig as usize];
    if let Some(a) = new {
        if sig == SIGKILL || sig == SIGSTOP {
            return Err(EINVAL);
        }
        p.sig.actions[sig as usize] = a;
        if a.handler == SIG_IGN || (a.handler == SIG_DFL && default_ignored(sig)) {
            p.sig.pending &= !(1 << sig);
        }
    }
    Ok(old)
}

pub fn sigprocmask(how: u32, set: Option<u32>) -> KResult<u32> {
    let p = proc::current();
    let old = p.sig.blocked;
    if let Some(s) = set {
        p.sig.blocked = match how {
            SIG_BLOCK => old | s,
            SIG_UNBLOCK => old & !s,
            SIG_SETMASK => s,
            _ => return Err(EINVAL),
        } & !UNBLOCKABLE;
    }
    Ok(old)
}
