//! date: print or set the date (UTC).
//!
//!     date [-u] [-d @seconds] [+format]
//!     date -s 'YYYY-MM-DD HH:MM[:SS]'
//!
//! Format: %Y %y %m %d %e %H %I %M %S %p %a %A %b %B %j %s %Z %F %T %D %R %n %t %%

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::time::{self, Tm};

rt::main!(main);

const LONG_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const LONG_MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn format(fmt: &str, t: u32) -> String {
    let tm = time::gmtime(t);
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(f) = it.next() else {
            out.push('%');
            break;
        };
        let h12 = if tm.hour % 12 == 0 { 12 } else { tm.hour % 12 };
        let s = match f {
            'Y' => format!("{}", tm.year),
            'y' => format!("{:02}", tm.year % 100),
            'C' => format!("{:02}", tm.year / 100),
            'm' => format!("{:02}", tm.month),
            'd' => format!("{:02}", tm.day),
            'e' => format!("{:2}", tm.day),
            'H' => format!("{:02}", tm.hour),
            'I' => format!("{:02}", h12),
            'M' => format!("{:02}", tm.min),
            'S' => format!("{:02}", tm.sec),
            'p' => (if tm.hour < 12 { "AM" } else { "PM" }).into(),
            'a' => time::WDAYS[tm.wday as usize].into(),
            'A' => LONG_DAYS[tm.wday as usize].into(),
            'b' | 'h' => time::MONTHS[tm.month as usize - 1].into(),
            'B' => LONG_MONTHS[tm.month as usize - 1].into(),
            'j' => format!("{:03}", tm.yday + 1),
            'u' => format!("{}", if tm.wday == 0 { 7 } else { tm.wday }),
            'w' => format!("{}", tm.wday),
            's' => format!("{t}"),
            'Z' => "UTC".into(),
            'z' => "+0000".into(),
            'F' => format!("{}-{:02}-{:02}", tm.year, tm.month, tm.day),
            'T' => format!("{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
            'R' => format!("{:02}:{:02}", tm.hour, tm.min),
            'D' => format!("{:02}/{:02}/{:02}", tm.month, tm.day, tm.year % 100),
            'c' => time::format_date(t),
            'n' => "\n".into(),
            't' => "\t".into(),
            '%' => "%".into(),
            c => format!("%{c}"),
        };
        out.push_str(&s);
    }
    out
}

fn parse(s: &str) -> Option<u32> {
    if let Some(n) = s.strip_prefix('@') {
        return n.parse().ok();
    }
    let (date, time_s) = s.split_once([' ', 'T']).unwrap_or((s, "00:00:00"));
    let d: Vec<u32> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<u32> = time_s.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 || t.len() < 2 {
        return None;
    }
    Some(time::mktime(&Tm { year: d[0] as i32, month: d[1], day: d[2], hour: t[0], min: t[1], sec: *t.get(2).unwrap_or(&0), ..Default::default() }))
}

fn main(args: &[String]) -> i32 {
    let (o, rest) = rt::getopt::parse(&args[1..], "ud:s:R", "[-u] [-d date] [-s date] [+format]");
    if let Some(s) = o.get('s') {
        let Some(t) = parse(s) else { rt::die!("invalid date '{s}'") };
        if let Err(e) = time::set(t) {
            rt::die!("cannot set date: {e}");
        }
        println!("{}", time::format_date(t));
        return 0;
    }
    let t = match o.get('d') {
        Some(d) => parse(d).unwrap_or_else(|| rt::die!("invalid date '{d}'")),
        None => time::now(),
    };
    if o.has('R') {
        println!("{}", format("%a, %d %b %Y %T +0000", t));
        return 0;
    }
    match rest.first() {
        Some(f) if f.starts_with('+') => println!("{}", format(&f[1..], t)),
        Some(f) => rt::die!("invalid date format '{f}' (formats start with '+')"),
        None => println!("{}", time::format_date(t)),
    }
    0
}
