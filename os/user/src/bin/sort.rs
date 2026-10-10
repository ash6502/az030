//! sort: sort lines of text.
//!
//!     sort [-nrufbdMhVsc] [-t sep] [-k field[,field]] [-o file] [file...]

#![no_std]
#![no_main]

use core::cmp::Ordering;
use rt::prelude::*;

rt::main!(main);

#[derive(Clone, Copy, Default)]
struct KeyOpts {
    numeric: bool,
    human: bool,
    reverse: bool,
    fold: bool,
    blanks: bool,
    dict: bool,
    month: bool,
    version: bool,
}

struct Key {
    start: (usize, usize),
    end: Option<(usize, usize)>,
    o: KeyOpts,
}

fn parse_key(s: &str, global: KeyOpts) -> Option<Key> {
    let (a, b) = match s.split_once(',') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let mut o = KeyOpts::default();
    let mut any = false;
    let pos = |t: &str, o: &mut KeyOpts, any: &mut bool| -> Option<(usize, usize)> {
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        for c in t[digits.len()..].chars() {
            *any = true;
            match c {
                'n' => o.numeric = true,
                'h' => o.human = true,
                'r' => o.reverse = true,
                'f' => o.fold = true,
                'b' => o.blanks = true,
                'd' => o.dict = true,
                'M' => o.month = true,
                'V' => o.version = true,
                _ => return None,
            }
        }
        let (f, c) = digits.split_once('.').unwrap_or((&digits, "0"));
        Some((f.parse().ok()?, c.parse().ok()?))
    };
    let start = pos(a, &mut o, &mut any)?;
    let end = match b {
        Some(b) => Some(pos(b, &mut o, &mut any)?),
        None => None,
    };
    Some(Key { start, end, o: if any { o } else { global } })
}

fn fields(line: &str, sep: Option<char>) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let b = line.as_bytes();
    match sep {
        Some(c) => {
            let mut st = 0;
            for (i, ch) in line.char_indices() {
                if ch == c {
                    v.push((st, i));
                    st = i + ch.len_utf8();
                }
            }
            v.push((st, line.len()));
        }
        None => {
            // fields include their leading blanks
            let mut i = 0;
            while i < b.len() {
                let st = i;
                while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                    i += 1;
                }
                while i < b.len() && b[i] != b' ' && b[i] != b'\t' {
                    i += 1;
                }
                v.push((st, i));
            }
        }
    }
    v
}

fn key_text<'a>(line: &'a str, k: &Key, sep: Option<char>) -> &'a str {
    let f = fields(line, sep);
    let (sf, sc) = k.start;
    if sf == 0 || sf > f.len() {
        return "";
    }
    let mut s = f[sf - 1].0;
    if k.o.blanks || sep.is_none() && sc > 0 {
        while s < line.len() && line.as_bytes()[s] == b' ' || s < line.len() && line.as_bytes()[s] == b'\t' {
            s += 1;
        }
    }
    if sc > 0 {
        s = (s + sc - 1).min(line.len());
    }
    let e = match k.end {
        None => line.len(),
        Some((ef, ec)) => {
            if ef > f.len() || ef == 0 {
                line.len()
            } else if ec == 0 {
                f[ef - 1].1
            } else {
                let mut st = f[ef - 1].0;
                if sep.is_none() {
                    while st < line.len() && (line.as_bytes()[st] == b' ' || line.as_bytes()[st] == b'\t') {
                        st += 1;
                    }
                }
                (st + ec).min(line.len())
            }
        }
    };
    if s >= e || !line.is_char_boundary(s) || !line.is_char_boundary(e) { "" } else { &line[s..e] }
}

fn num(s: &str, human: bool) -> f64 {
    let t = s.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+')) {
            end = i + 1;
        } else {
            break;
        }
    }
    let mut v: f64 = t[..end].parse().unwrap_or(0.0);
    if human {
        let mult = match t[end..].chars().next() {
            Some('K' | 'k') => 1e3,
            Some('M') => 1e6,
            Some('G') => 1e9,
            Some('T') => 1e12,
            _ => 1.0,
        };
        v *= mult;
    }
    v
}

fn month(s: &str) -> usize {
    let t = s.trim_start().to_ascii_uppercase();
    ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"].iter().position(|m| t.starts_with(m)).map_or(0, |i| i + 1)
}

fn version_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    while !x.is_empty() && !y.is_empty() {
        let dx = x[0].is_ascii_digit();
        let dy = y[0].is_ascii_digit();
        if dx && dy {
            let nx = x.iter().take_while(|c| c.is_ascii_digit()).count();
            let ny = y.iter().take_while(|c| c.is_ascii_digit()).count();
            let vx: u64 = core::str::from_utf8(&x[..nx]).unwrap().parse().unwrap_or(0);
            let vy: u64 = core::str::from_utf8(&y[..ny]).unwrap().parse().unwrap_or(0);
            match vx.cmp(&vy) {
                Ordering::Equal => {}
                o => return o,
            }
            x = &x[nx..];
            y = &y[ny..];
        } else {
            match x[0].cmp(&y[0]) {
                Ordering::Equal => {}
                o => return o,
            }
            x = &x[1..];
            y = &y[1..];
        }
    }
    x.len().cmp(&y.len())
}

fn cmp_text(a: &str, b: &str, o: &KeyOpts) -> Ordering {
    let r = if o.numeric || o.human {
        num(a, o.human).partial_cmp(&num(b, o.human)).unwrap_or(Ordering::Equal)
    } else if o.month {
        month(a).cmp(&month(b))
    } else if o.version {
        version_cmp(a, b)
    } else {
        let prep = |s: &str| -> String {
            let s = if o.blanks { s.trim_start() } else { s };
            s.chars()
                .filter(|c| !o.dict || c.is_alphanumeric() || c.is_whitespace())
                .map(|c| if o.fold { c.to_ascii_uppercase() } else { c })
                .collect()
        };
        if o.fold || o.dict || o.blanks { prep(a).cmp(&prep(b)) } else { a.cmp(b) }
    };
    if o.reverse { r.reverse() } else { r }
}

fn main(args: &[String]) -> i32 {
    let (p, files) = rt::getopt::parse(&args[1..], "nrufbdMhVsct:k:o:zm", "[-nrufbdMhVsc] [-t sep] [-k key] [-o file] [file...]");
    let global = KeyOpts {
        numeric: p.has('n'),
        human: p.has('h'),
        reverse: p.has('r'),
        fold: p.has('f'),
        blanks: p.has('b'),
        dict: p.has('d'),
        month: p.has('M'),
        version: p.has('V'),
    };
    let sep = p.get('t').map(|s| s.chars().next().unwrap_or('\t'));
    let mut keys = Vec::new();
    for k in p.all('k') {
        match parse_key(k, global) {
            Some(k) => keys.push(k),
            None => rt::die!("invalid key: '{k}'"),
        }
    }
    let mut lines: Vec<String> = Vec::new();
    let mut st = 0;
    for f in rt::io::inputs(&files) {
        match rt::io::open_input(&f) {
            Ok(mut r) => {
                let mut v = Vec::new();
                let _ = r.read_to_end(&mut v);
                let text = String::from_utf8_lossy(&v);
                lines.extend(rt::util::lines(&text).into_iter().map(String::from));
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 2;
            }
        }
    }
    let cmp = |a: &String, b: &String| -> Ordering {
        for k in &keys {
            let o = cmp_text(key_text(a, k, sep), key_text(b, k, sep), &k.o);
            if o != Ordering::Equal {
                return o;
            }
        }
        if keys.is_empty() {
            let o = cmp_text(a, b, &global);
            if o != Ordering::Equal || p.has('s') {
                return o;
            }
        } else if p.has('s') {
            return Ordering::Equal;
        }
        // last resort: whole-line byte comparison
        if p.has('u') { Ordering::Equal } else if global.reverse { b.cmp(a) } else { a.cmp(b) }
    };
    if p.has('c') {
        for (i, w) in lines.windows(2).enumerate() {
            let o = cmp(&w[0], &w[1]);
            if o == Ordering::Greater || (p.has('u') && o == Ordering::Equal) {
                eprintln!("sort: {}:{}: disorder: {}", files.first().map_or("-", |s| s), i + 2, w[1]);
                return 1;
            }
        }
        return 0;
    }
    lines.sort_by(cmp);
    if p.has('u') {
        lines.dedup_by(|a, b| cmp(a, b) == Ordering::Equal);
    }
    let mut text = String::new();
    for l in &lines {
        text.push_str(l);
        text.push('\n');
    }
    match p.get('o') {
        Some(o) => {
            if let Err(e) = rt::fs::write(o, text.as_bytes()) {
                rt::die!("{o}: {e}");
            }
        }
        None => print!("{text}"),
    }
    st
}
