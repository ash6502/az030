//! touch: change file timestamps, creating files that do not exist.
//!
//!     touch [-c] [-r ref | -d 'YYYY-MM-DD [HH:MM[:SS]]' | -t [[CC]YY]MMDDhhmm[.ss]] file...

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::time::Tm;

rt::main!(main);

fn num(s: &str) -> Option<u32> {
    s.parse().ok()
}

fn parse_d(s: &str) -> Option<u32> {
    let (date, time) = s.split_once([' ', 'T']).unwrap_or((s, "00:00"));
    let d: Vec<&str> = date.split('-').collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() < 2 {
        return None;
    }
    let tm = Tm {
        year: d[0].parse().ok()?,
        month: num(d[1])?,
        day: num(d[2])?,
        hour: num(t[0])?,
        min: num(t[1])?,
        sec: t.get(2).and_then(|s| num(s)).unwrap_or(0),
        ..Default::default()
    };
    Some(rt::time::mktime(&tm))
}

fn parse_t(s: &str) -> Option<u32> {
    let (main, sec) = s.split_once('.').unwrap_or((s, "0"));
    let now = rt::time::gmtime(rt::time::now());
    let (year, rest) = match main.len() {
        8 => (now.year, main),
        10 => {
            let yy: i32 = main[..2].parse().ok()?;
            (if yy < 70 { 2000 + yy } else { 1900 + yy }, &main[2..])
        }
        12 => (main[..4].parse().ok()?, &main[4..]),
        _ => return None,
    };
    let tm = Tm {
        year,
        month: num(&rest[0..2])?,
        day: num(&rest[2..4])?,
        hour: num(&rest[4..6])?,
        min: num(&rest[6..8])?,
        sec: num(sec)?,
        ..Default::default()
    };
    Some(rt::time::mktime(&tm))
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "camr:d:t:", "[-c] [-r ref | -d date | -t stamp] file...");
    if files.is_empty() {
        rt::die!("missing file operand");
    }
    let times = if let Some(r) = o.get('r') {
        match rt::fs::metadata(r) {
            Ok(m) => Some((m.0.atime, m.mtime())),
            Err(e) => rt::die!("{r}: {e}"),
        }
    } else if let Some(d) = o.get('d') {
        match parse_d(d) {
            Some(t) => Some((t, t)),
            None => rt::die!("invalid date '{d}'"),
        }
    } else if let Some(d) = o.get('t') {
        match parse_t(d) {
            Some(t) => Some((t, t)),
            None => rt::die!("invalid date format '{d}'"),
        }
    } else {
        None
    };
    let mut st = 0;
    for f in &files {
        if !rt::fs::exists(f) {
            if o.has('c') {
                continue;
            }
            if let Err(e) = rt::fs::File::open_with(f, azsys::flags::O_WRONLY | azsys::flags::O_CREAT, 0o666) {
                rt::warn!("cannot touch '{f}': {e}");
                st = 1;
                continue;
            }
        }
        if let Err(e) = rt::fs::set_times(f, times) {
            rt::warn!("setting times of '{f}': {e}");
            st = 1;
        }
    }
    st
}
