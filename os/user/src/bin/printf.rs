//! printf: formatted output.
//!
//!     printf format [argument...]
//!
//! Conversions: %d %i %u %o %x %X %c %s %b %e %f %g %% with flags `-+ 0#`,
//! width and precision (`*` takes an argument). The format is reused while
//! arguments remain.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn escape(s: &str, out: &mut String) -> bool {
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some('e') => out.push('\x1b'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('c') => return true,
            Some(d @ '0'..='7') => {
                let mut v = d as u32 - '0' as u32;
                for _ in 0..2 {
                    match it.peek() {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + (*d as u32 - '0' as u32);
                            it.next();
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            Some('x') => {
                let mut v = 0;
                for _ in 0..2 {
                    match it.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            it.next();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            Some(c) => {
                out.push('\\');
                out.push(c);
            }
            None => out.push('\\'),
        }
    }
    false
}

fn int_arg(s: &str, st: &mut i32) -> i64 {
    let t = s.trim();
    if let Some(c) = t.strip_prefix(['\'', '"']) {
        return c.chars().next().map_or(0, |c| c as i64);
    }
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let v = if let Some(h) = t.strip_prefix("0x").or(t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16)
    } else if t.len() > 1 && t.starts_with('0') {
        i64::from_str_radix(&t[1..], 8)
    } else if t.is_empty() {
        Ok(0)
    } else {
        t.parse()
    };
    match v {
        Ok(v) => if neg { -v } else { v },
        Err(_) => {
            rt::warn!("{s}: expected a numeric value");
            *st = 1;
            0
        }
    }
}

fn pad(s: String, width: usize, left: bool, zero: bool) -> String {
    let n = s.chars().count();
    if n >= width {
        return s;
    }
    let fill = width - n;
    if left {
        let mut r = s;
        r.extend(core::iter::repeat_n(' ', fill));
        r
    } else if zero {
        let (sign, digits) = if s.starts_with(['-', '+', ' ']) { s.split_at(1) } else { ("", s.as_str()) };
        let (pfx, digits) = if digits.starts_with("0x") || digits.starts_with("0X") { digits.split_at(2) } else { ("", digits) };
        let mut r = String::from(sign);
        r.push_str(pfx);
        r.extend(core::iter::repeat_n('0', fill));
        r.push_str(digits);
        r
    } else {
        let mut r: String = core::iter::repeat_n(' ', fill).collect();
        r.push_str(&s);
        r
    }
}

/// Format once; returns (output, arguments consumed, stop).
fn format_once(fmt: &[char], args: &[String], st: &mut i32) -> (String, usize, bool) {
    let mut out = String::new();
    let mut ai = 0;
    let next = |ai: &mut usize| -> Option<&String> {
        let a = args.get(*ai);
        *ai += 1;
        a
    };
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c == '\\' {
            let mut j = i + 1;
            if j < fmt.len() {
                j += 1;
                // octal escapes take up to 3 digits
                if fmt[i + 1].is_digit(8) {
                    while j < fmt.len() && j < i + 4 && fmt[j].is_digit(8) {
                        j += 1;
                    }
                } else if fmt[i + 1] == 'x' {
                    while j < fmt.len() && j < i + 4 && fmt[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                }
            }
            let s: String = fmt[i..j].iter().collect();
            if escape(&s, &mut out) {
                return (out, ai, true);
            }
            i = j;
            continue;
        }
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i < fmt.len() && fmt[i] == '%' {
            out.push('%');
            i += 1;
            continue;
        }
        let (mut left, mut plus, mut space, mut zero, mut alt) = (false, false, false, false, false);
        while i < fmt.len() && "-+ 0#".contains(fmt[i]) {
            match fmt[i] {
                '-' => left = true,
                '+' => plus = true,
                ' ' => space = true,
                '0' => zero = true,
                _ => alt = true,
            }
            i += 1;
        }
        let mut width = 0usize;
        if i < fmt.len() && fmt[i] == '*' {
            let w = int_arg(next(&mut ai).map_or("0", |s| s), st);
            if w < 0 {
                left = true;
            }
            width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                width = width * 10 + fmt[i] as usize - '0' as usize;
                i += 1;
            }
        }
        let mut prec: Option<usize> = None;
        if i < fmt.len() && fmt[i] == '.' {
            i += 1;
            let mut p = 0;
            if i < fmt.len() && fmt[i] == '*' {
                p = int_arg(next(&mut ai).map_or("0", |s| s), st).max(0) as usize;
                i += 1;
            } else {
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + fmt[i] as usize - '0' as usize;
                    i += 1;
                }
            }
            prec = Some(p);
        }
        // length modifiers are accepted and ignored
        while i < fmt.len() && "hlLqjzt".contains(fmt[i]) {
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else {
            out.push('%');
            break;
        };
        i += 1;
        let arg = next(&mut ai).cloned();
        let sarg = arg.clone().unwrap_or_default();
        let sign = |v: bool, neg: bool| if neg { "-" } else if v && plus { "+" } else if v && space { " " } else { "" };
        let s = match conv {
            'd' | 'i' => {
                let v = int_arg(&sarg, st);
                let mut digits = format!("{}", v.unsigned_abs());
                if let Some(p) = prec {
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                format!("{}{}", sign(true, v < 0), digits)
            }
            'u' | 'o' | 'x' | 'X' => {
                let v = int_arg(&sarg, st) as u64;
                let mut d = match conv {
                    'o' => format!("{v:o}"),
                    'x' => format!("{v:x}"),
                    'X' => format!("{v:X}"),
                    _ => format!("{v}"),
                };
                if let Some(p) = prec {
                    while d.len() < p {
                        d.insert(0, '0');
                    }
                }
                if alt && v != 0 {
                    d = match conv {
                        'o' => format!("0{d}"),
                        'x' => format!("0x{d}"),
                        'X' => format!("0X{d}"),
                        _ => d,
                    };
                }
                d
            }
            'c' => sarg.chars().next().map(String::from).unwrap_or_default(),
            's' => match prec {
                Some(p) => sarg.chars().take(p).collect(),
                None => sarg,
            },
            'b' => {
                let mut o = String::new();
                let stop = escape(&sarg, &mut o);
                if stop {
                    out.push_str(&pad(o, width, left, false));
                    return (out, ai, true);
                }
                o
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let v: f64 = sarg.trim().parse().unwrap_or_else(|_| {
                    if arg.is_some() && !sarg.is_empty() {
                        rt::warn!("{sarg}: expected a numeric value");
                        *st = 1;
                    }
                    0.0
                });
                let p = prec.unwrap_or(6);
                let body = match conv {
                    'f' | 'F' => format!("{:.*}", p, v.abs()),
                    'e' | 'E' => {
                        let s = format!("{:.*e}", p, v.abs());
                        // Rust writes 1.5e2; C writes 1.5e+02
                        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
                        let ev: i32 = e.parse().unwrap_or(0);
                        let r = format!("{m}e{}{:02}", if ev < 0 { '-' } else { '+' }, ev.abs());
                        if conv == 'E' { r.to_uppercase() } else { r }
                    }
                    _ => {
                        let p = if p == 0 { 1 } else { p };
                        let exp = if v == 0.0 { 0 } else { libm_floor_log10(v.abs()) };
                        let r = if exp < -4 || exp >= p as i32 {
                            let s = format!("{:.*e}", p - 1, v.abs());
                            let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
                            let m = if alt { m.into() } else { trim_zeros(m) };
                            let ev: i32 = e.parse().unwrap_or(0);
                            format!("{m}e{}{:02}", if ev < 0 { '-' } else { '+' }, ev.abs())
                        } else {
                            let s = format!("{:.*}", (p as i32 - 1 - exp).max(0) as usize, v.abs());
                            if alt { s } else { trim_zeros(&s) }
                        };
                        if conv == 'G' { r.to_uppercase() } else { r }
                    }
                };
                format!("{}{}", sign(true, v.is_sign_negative() && v != 0.0), body)
            }
            c => {
                rt::warn!("%{c}: invalid conversion");
                *st = 1;
                String::new()
            }
        };
        let numeric = !matches!(conv, 's' | 'c' | 'b');
        out.push_str(&pad(s, width, left, zero && numeric && prec.is_none() || zero && matches!(conv, 'f' | 'e' | 'g' | 'E' | 'G' | 'F')));
        if arg.is_none() && ai > args.len() {
            ai = args.len() + 1;
        }
    }
    (out, ai, false)
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').into() } else { s.into() }
}

fn libm_floor_log10(v: f64) -> i32 {
    let mut e = 0;
    let mut x = v;
    while x >= 10.0 {
        x /= 10.0;
        e += 1;
    }
    while x < 1.0 {
        x *= 10.0;
        e -= 1;
    }
    e
}

fn main(args: &[String]) -> i32 {
    let Some(fmt) = args.get(1) else { rt::die!("usage: printf format [argument...]") };
    let fmt: Vec<char> = fmt.chars().collect();
    let mut rest = &args[2..];
    let mut st = 0;
    let mut out = String::new();
    loop {
        let (s, used, stop) = format_once(&fmt, rest, &mut st);
        out.push_str(&s);
        if stop || used == 0 || used >= rest.len() {
            break;
        }
        rest = &rest[used..];
    }
    print!("{out}");
    st
}
