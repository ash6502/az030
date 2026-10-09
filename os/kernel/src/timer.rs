//! System clock, driven by the timer block at 0xFE004000 (see emu/README.md).
//! Without it the system still runs, but only switches tasks when one blocks and
//! has no notion of time.

use crate::arch;
use crate::proc;
use crate::util::Global;
use azsys::HZ;

const BASE: u32 = 0xFE00_4000;
const CTRL: u32 = BASE;
const PERIOD: u32 = BASE + 0x04;
const STATUS: u32 = BASE + 0x08;
const RTC: u32 = BASE + 0x14;
pub const INTEN: u32 = BASE + 0x18;
const POWER: u32 = BASE + 0x20;
const ID: u32 = BASE + 0x24;
const ID_MAGIC: u32 = 0x5449_4D52;

/// Ticks a process may run before another runnable process gets the CPU.
const SLICE: u32 = 5;
/// Flush dirty buffers this often.
const SYNC_INTERVAL: u64 = 5 * HZ as u64;

struct Clock {
    present: bool,
    ticks: u64,
    boot_time: u32,
    slice_left: u32,
    resched: bool,
    sync_due: bool,
}

static CLOCK: Global<Clock> =
    Global::new(Clock { present: false, ticks: 0, boot_time: 0, slice_left: SLICE, resched: false, sync_due: false });

pub fn init() {
    let c = CLOCK.get();
    if arch::probe(ID) == Some(ID_MAGIC) {
        c.present = true;
        c.boot_time = arch::rd(RTC);
        arch::wr(PERIOD, 1_000_000 / HZ);
        arch::wr(STATUS, 1);
        arch::wr(CTRL, 1);
        arch::wr(INTEN, arch::rd(INTEN) | 1);
        kprintln!("timer: {} Hz", HZ);
    } else {
        kprintln!("timer: not present, cooperative scheduling only");
    }
}

pub fn present() -> bool {
    CLOCK.get().present
}

/// Level 6 interrupt.
pub fn interrupt() {
    arch::wr(STATUS, 1);
    let c = CLOCK.get();
    c.ticks += 1;
    if proc::current_opt().is_some() {
        let p = proc::current();
        if p.in_kernel {
            p.stime += 1;
        } else {
            p.utime += 1;
        }
    }
    if c.slice_left > 0 {
        c.slice_left -= 1;
    }
    if c.slice_left == 0 && proc::any_other_runnable() {
        c.resched = true;
    }
    if c.ticks % SYNC_INTERVAL == 0 {
        c.sync_due = true;
    }
    proc::tick(c.ticks);
}

pub fn need_resched() -> bool {
    CLOCK.get().resched
}

pub fn clear_resched() {
    let c = CLOCK.get();
    c.resched = false;
    c.slice_left = SLICE;
}

pub fn take_sync_due() -> bool {
    core::mem::take(&mut CLOCK.get().sync_due)
}

pub fn ticks() -> u64 {
    CLOCK.get().ticks
}

pub fn uptime() -> u32 {
    (CLOCK.get().ticks / HZ as u64) as u32
}

/// Seconds since 1970.
pub fn now() -> u32 {
    let c = CLOCK.get();
    c.boot_time + (c.ticks / HZ as u64) as u32
}

/// (seconds, microseconds) since 1970.
pub fn now_precise() -> (u32, u32) {
    let c = CLOCK.get();
    let sub = (c.ticks % HZ as u64) as u32 * (1_000_000 / HZ);
    (now(), sub)
}

pub fn set_time(secs: u32) {
    let c = CLOCK.get();
    c.boot_time = secs.wrapping_sub((c.ticks / HZ as u64) as u32);
}

pub fn ms_to_ticks(ms: u32) -> u64 {
    (ms as u64 * HZ as u64).div_ceil(1000).max(1)
}

/// Ask the emulator to power off (no-op on hardware without the timer block).
pub fn power_off() {
    if present() {
        arch::wr(POWER, 0x504F_4646);
    }
}
