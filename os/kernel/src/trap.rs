//! Exceptions, interrupts and the system-call entry.

use crate::{proc, signal, syscall, timer, tty};
use azsys::sig::*;

/// Saved user/kernel state, built by `exc_common` in asm/entry.s on the kernel stack.
/// The CPU's exception frame starts at `sr`; formats other than $0 carry more words
/// after `fv`.
#[repr(C)]
pub struct TrapFrame {
    pub usp: u32,
    pub d: [u32; 8],
    pub a: [u32; 7],
    pub sr: u16,
    pub pc: u32,
    /// Format (bits 15-12) and vector offset (bits 11-0).
    pub fv: u16,
}

const _: () = {
    assert!(core::mem::offset_of!(TrapFrame, sr) == 64);
    assert!(core::mem::offset_of!(TrapFrame, pc) == 66);
    assert!(core::mem::offset_of!(TrapFrame, fv) == 70);
    assert!(core::mem::size_of::<TrapFrame>() == 72);
};

pub const SR_SUPERVISOR: u16 = 0x2000;

impl TrapFrame {
    pub fn from_user(&self) -> bool {
        self.sr & SR_SUPERVISOR == 0
    }
    pub fn vector(&self) -> u32 {
        (self.fv as u32 & 0xFFF) >> 2
    }
    pub fn format(&self) -> u32 {
        self.fv as u32 >> 12
    }
    /// Data fault address of a bus error (format $A/$B frames).
    pub fn fault_address(&self) -> Option<u32> {
        match self.format() {
            0xA | 0xB => {
                let base = self as *const TrapFrame as u32 + 64;
                Some(unsafe { core::ptr::read_unaligned((base + 0x10) as *const u32) })
            }
            _ => None,
        }
    }
    /// Special status word of a bus error frame.
    fn ssw(&self) -> u16 {
        let base = self as *const TrapFrame as u32 + 64;
        unsafe { core::ptr::read_unaligned((base + 0x0A) as *const u16) }
    }
}

const VEC_BUS_ERROR: u32 = 2;
const VEC_ADDRESS_ERROR: u32 = 3;
const VEC_ILLEGAL: u32 = 4;
const VEC_ZERO_DIVIDE: u32 = 5;
const VEC_CHK: u32 = 6;
const VEC_TRAPV: u32 = 7;
const VEC_PRIVILEGE: u32 = 8;
const VEC_TRACE: u32 = 9;
const VEC_LINE_A: u32 = 10;
const VEC_LINE_F: u32 = 11;
const VEC_SPURIOUS: u32 = 24;
const VEC_SYSCALL: u32 = 32; // trap #0
const VEC_TRAP1: u32 = 33;
const VEC_TRAP15: u32 = 47;

const NAMES: [&str; 16] = [
    "?", "?", "bus error", "address error", "illegal instruction", "divide by zero", "CHK", "TRAPV",
    "privilege violation", "trace", "line A", "line F", "?", "coprocessor protocol violation", "format error",
    "uninitialised interrupt",
];

#[unsafe(no_mangle)]
pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
    let vec = tf.vector();
    let user = tf.from_user();
    if user {
        proc::enter_kernel(tf);
    }
    match vec {
        VEC_SYSCALL if user => syscall::dispatch(tf),
        25..=31 => irq(vec - 24),
        VEC_SPURIOUS => {}
        VEC_BUS_ERROR if user => {
            let addr = tf.fault_address().unwrap_or(0);
            // instruction fetch faults have no data address; a data fault below the stack
            // may just need the stack to grow
            let data_fault = tf.ssw() & 0x0100 != 0;
            if !(data_fault && proc::current().grow_stack(addr)) {
                fault(tf, SIGSEGV, addr);
            }
        }
        VEC_ADDRESS_ERROR | VEC_BUS_ERROR if user => fault(tf, SIGBUS, tf.fault_address().unwrap_or(tf.pc)),
        VEC_ILLEGAL | VEC_LINE_A | VEC_LINE_F | VEC_PRIVILEGE if user => fault(tf, SIGILL, tf.pc),
        VEC_ZERO_DIVIDE | VEC_CHK | VEC_TRAPV if user => fault(tf, SIGFPE, tf.pc),
        48..=54 if user => fault(tf, SIGFPE, tf.pc), // FPU exceptions
        VEC_TRACE | VEC_TRAP1..=VEC_TRAP15 if user => fault(tf, SIGTRAP, tf.pc),
        _ => kernel_fault(tf),
    }
    if user {
        return_to_user(tf);
    }
}

fn irq(level: u32) {
    match level {
        6 => timer::interrupt(),
        4 => tty::interrupt(),
        _ => kprintln!("spurious interrupt, level {}", level),
    }
}

/// A user program faulted: deliver a signal.
fn fault(tf: &mut TrapFrame, sig: u32, addr: u32) {
    let p = proc::current();
    if crate::config().quiet {
        let _ = addr;
    } else if sig == SIGSEGV || sig == SIGBUS || sig == SIGILL {
        kprintln!("{}[{}]: {} at pc {:08x}, address {:08x}", p.name(), p.pid, signal::describe(sig), tf.pc, addr);
    }
    // a fault frame cannot be resumed at another address, so a caught fault signal
    // still kills the process if it came from a bus or address error
    if tf.format() >= 0xA {
        proc::exit_current(sig as i32);
    }
    signal::send(p, sig);
}

fn kernel_fault(tf: &TrapFrame) -> ! {
    let vec = tf.vector();
    let name = NAMES.get(vec as usize).copied().unwrap_or("exception");
    kprintln!();
    kprintln!("*** kernel {} (vector {}) at pc {:08x} sr {:04x}", name, vec, tf.pc, tf.sr);
    if let Some(a) = tf.fault_address() {
        kprintln!("    fault address {:08x}", a);
    }
    for i in 0..8 {
        kprint!("d{}={:08x} ", i, tf.d[i]);
    }
    kprintln!();
    for i in 0..7 {
        kprint!("a{}={:08x} ", i, tf.a[i]);
    }
    kprintln!("sp={:08x}", tf as *const _ as u32 + 72);
    panic!("unhandled exception in kernel mode");
}

/// Last thing before returning to user mode: reschedule if needed, deliver signals.
pub fn return_to_user(tf: &mut TrapFrame) {
    loop {
        if timer::need_resched() {
            proc::yield_cpu();
        }
        if !signal::deliver(tf) {
            break;
        }
    }
    proc::leave_kernel();
}
