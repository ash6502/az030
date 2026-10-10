//! od / hexdump: dump files in octal, hexadecimal or as characters.
//!
//!     od [-A d|o|x|n] [-t x1|x2|x4|o1|o2|d1|d2|u1|c|a] [-bcdox] [-j skip] [-N count] [file...]
//!     hexdump [-C] [file...]   (-C: canonical hex + ASCII)

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let hexdump = rt::env::progname() == "hexdump";
    let (o, files) = rt::getopt::parse(&args[1..], "A:t:bcdoxCj:N:v", "[-A base] [-t type] [-bcdox] [-j skip] [-N count] [file...]");
    let mut data = Vec::new();
    let mut st = 0;
    for f in rt::io::inputs(&files) {
        match rt::io::open_input(&f) {
            Ok(mut r) => {
                let _ = r.read_to_end(&mut data);
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    let skip = o.num('j', 0).unwrap_or(0) as usize;
    let data = &data[skip.min(data.len())..];
    let data = match o.get('N') {
        Some(n) => &data[..n.parse::<usize>().unwrap_or(data.len()).min(data.len())],
        None => data,
    };
    let mut out = String::new();
    if hexdump || o.has('C') {
        for (k, chunk) in data.chunks(16).enumerate() {
            out.push_str(&format!("{:08x}  ", skip + k * 16));
            for i in 0..16 {
                match chunk.get(i) {
                    Some(b) => out.push_str(&format!("{b:02x} ")),
                    None => out.push_str("   "),
                }
                if i == 7 {
                    out.push(' ');
                }
            }
            out.push_str(" |");
            for &b in chunk {
                out.push(if (0x20..0x7f).contains(&b) { b as char } else { '.' });
            }
            out.push_str("|\n");
        }
        if !data.is_empty() {
            out.push_str(&format!("{:08x}\n", skip + data.len()));
        }
        print!("{out}");
        return st;
    }
    let ty = if let Some(t) = o.get('t') {
        t.to_string()
    } else if o.has('b') {
        "o1".into()
    } else if o.has('c') {
        "c".into()
    } else if o.has('d') {
        "u2".into()
    } else if o.has('x') {
        "x2".into()
    } else {
        "o2".into()
    };
    let base = o.get('A').unwrap_or("o");
    let addr = |n: usize| -> String {
        match base {
            "d" => format!("{n:07}"),
            "x" => format!("{n:06x}"),
            "n" => String::new(),
            _ => format!("{n:07o}"),
        }
    };
    let size: usize = ty.chars().skip(1).collect::<String>().parse().unwrap_or(1);
    for (k, chunk) in data.chunks(16).enumerate() {
        out.push_str(&addr(skip + k * 16));
        match ty.chars().next() {
            Some('c') | Some('a') => {
                for &b in chunk {
                    let s = match b {
                        0 => "\\0".into(),
                        b'\n' => "\\n".into(),
                        b'\t' => "\\t".into(),
                        b'\r' => "\\r".into(),
                        0x20..=0x7e => format!("{}", b as char),
                        _ => format!("{b:03o}"),
                    };
                    out.push_str(&format!("{s:>4}"));
                }
            }
            Some(t) => {
                for w in chunk.chunks(size) {
                    let mut v: u64 = 0;
                    for &b in w {
                        v = v << 8 | b as u64;
                    }
                    v <<= 8 * (size - w.len());
                    let s = match (t, size) {
                        ('x', _) => format!("{:0w$x}", v, w = size * 2),
                        ('o', _) => format!("{:0w$o}", v, w = (size * 8).div_ceil(3)),
                        ('d', 1) => format!("{}", v as u8 as i8),
                        ('d', 2) => format!("{}", v as u16 as i16),
                        ('d', _) => format!("{}", v as u32 as i32),
                        _ => format!("{v}"),
                    };
                    let width = match t {
                        'x' => size * 2,
                        'o' => (size * 8).div_ceil(3),
                        _ => [0, 4, 6, 0, 11][size.min(4)],
                    };
                    out.push_str(&format!(" {s:>width$}"));
                }
            }
            None => {}
        }
        out.push('\n');
    }
    if base != "n" {
        out.push_str(&addr(skip + data.len()));
        out.push('\n');
    }
    print!("{out}");
    st
}
