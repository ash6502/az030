//! echo: write arguments.
//!
//!     echo [-neE] [string...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn unescape(s: &str, out: &mut String) -> bool {
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
            Some('c') => return true,
            Some('0') => {
                let mut v = 0;
                for _ in 0..3 {
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
            Some(c) => {
                out.push('\\');
                out.push(c);
            }
            None => out.push('\\'),
        }
    }
    false
}

fn main(args: &[String]) -> i32 {
    let mut nl = true;
    let mut esc = false;
    let mut i = 1;
    while i < args.len() && args[i].len() > 1 && args[i].starts_with('-') && args[i][1..].chars().all(|c| "neE".contains(c)) {
        for c in args[i][1..].chars() {
            match c {
                'n' => nl = false,
                'e' => esc = true,
                _ => esc = false,
            }
        }
        i += 1;
    }
    let mut out = String::new();
    for (k, a) in args[i..].iter().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        if esc {
            if unescape(a, &mut out) {
                print!("{out}");
                return 0;
            }
        } else {
            out.push_str(a);
        }
    }
    if nl {
        out.push('\n');
    }
    print!("{out}");
    0
}
