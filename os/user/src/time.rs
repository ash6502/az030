//! Time: the clock, sleeping and calendar conversion (UTC only).

use crate::sys::{self, nr};
use alloc::string::String;

/// Seconds since 1970-01-01 00:00 UTC.
pub fn now() -> u32 {
    sys::call0(nr::TIME).unwrap_or(0)
}

/// (seconds, microseconds) since the epoch.
pub fn now_precise() -> (u32, u32) {
    let mut t = azsys::Timeval::default();
    let _ = sys::call1(nr::GETTIMEOFDAY, &mut t as *mut _ as u32);
    (t.sec, t.usec)
}

pub fn set(secs: u32) -> crate::Result<()> {
    sys::call1(nr::SETTIMEOFDAY, secs).map(|_| ())
}

/// Sleep; Err(EINTR) if a signal interrupted it.
pub fn sleep_ms(ms: u32) -> crate::Result<()> {
    sys::call1(nr::SLEEP_MS, ms).map(|_| ())
}

/// Milliseconds since boot (from the clock tick counter).
pub fn ticks_ms() -> u64 {
    let (_, t) = crate::process::times();
    t as u64 * 1000 / azsys::HZ as u64
}

/// A broken-down UTC time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tm {
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    /// 0 = Sunday
    pub wday: u32,
    /// 0..=365
    pub yday: u32,
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const MDAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

pub fn month_days(y: i32, m: u32) -> u32 {
    if m == 2 && is_leap(y) { 29 } else { MDAYS[(m - 1) as usize] }
}

pub fn gmtime(t: u32) -> Tm {
    let mut days = t / 86400;
    let rem = t % 86400;
    let wday = (days + 4) % 7;
    let mut year = 1970;
    loop {
        let n = if is_leap(year) { 366 } else { 365 };
        if days < n {
            break;
        }
        days -= n;
        year += 1;
    }
    let yday = days;
    let mut month = 1;
    while days >= month_days(year, month) {
        days -= month_days(year, month);
        month += 1;
    }
    Tm { year, month, day: days + 1, hour: rem / 3600, min: rem % 3600 / 60, sec: rem % 60, wday, yday }
}

pub fn mktime(tm: &Tm) -> u32 {
    let mut days: u32 = 0;
    for y in 1970..tm.year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    for m in 1..tm.month {
        days += month_days(tm.year, m);
    }
    days += tm.day.saturating_sub(1);
    days * 86400 + tm.hour * 3600 + tm.min * 60 + tm.sec
}

pub const WDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `Thu Oct  9 21:56:07 UTC 2026`
pub fn format_date(t: u32) -> String {
    let tm = gmtime(t);
    alloc::format!(
        "{} {} {:2} {:02}:{:02}:{:02} UTC {}",
        WDAYS[tm.wday as usize],
        MONTHS[tm.month as usize - 1],
        tm.day,
        tm.hour,
        tm.min,
        tm.sec,
        tm.year
    )
}

/// `ls -l` style: `Oct  9 21:56` for recent times, `Oct  9  2025` otherwise.
pub fn format_short(t: u32, now: u32) -> String {
    let tm = gmtime(t);
    let recent = t <= now + 3600 && now - t.min(now) < 180 * 86400;
    if recent {
        alloc::format!("{} {:2} {:02}:{:02}", MONTHS[tm.month as usize - 1], tm.day, tm.hour, tm.min)
    } else {
        alloc::format!("{} {:2}  {}", MONTHS[tm.month as usize - 1], tm.day, tm.year)
    }
}
