//! wc: count lines, words and bytes.
//!
//!     wc [-lwcm] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "lwcmL", "[-lwcm] [file...]");
    let none = !(o.has('l') || o.has('w') || o.has('c') || o.has('m') || o.has('L'));
    let show = [o.has('l') || none, o.has('w') || none, o.has('m'), o.has('c') || none, o.has('L')];
    let named = !files.is_empty();
    let files = rt::io::inputs(&files);
    let mut total = [0u64; 5];
    let mut rows = Vec::new();
    let mut st = 0;
    for f in &files {
        let mut r = match rt::io::open_input(f) {
            Ok(r) => r,
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
                continue;
            }
        };
        let mut c = [0u64; 5];
        let mut in_word = false;
        let mut linelen = 0u64;
        let mut buf = vec![0u8; 8192];
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    rt::warn!("{f}: {e}");
                    st = 1;
                    break;
                }
            };
            for &b in &buf[..n] {
                c[3] += 1;
                if b & 0xC0 != 0x80 {
                    c[2] += 1;
                }
                if b == b'\n' {
                    c[0] += 1;
                    c[4] = c[4].max(linelen);
                    linelen = 0;
                } else if b & 0xC0 != 0x80 {
                    linelen += 1;
                }
                let ws = matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
                if !ws && !in_word {
                    c[1] += 1;
                }
                in_word = !ws;
            }
        }
        c[4] = c[4].max(linelen);
        for k in 0..4 {
            total[k] += c[k];
        }
        total[4] = total[4].max(c[4]);
        rows.push((c, if named { f.clone() } else { String::new() }));
    }
    if files.len() > 1 {
        rows.push((total, "total".into()));
    }
    let width = rows.iter().flat_map(|(c, _)| c.iter()).map(|v| format!("{v}").len()).max().unwrap_or(1).max(if files.len() > 1 { 3 } else { 1 });
    let single = show.iter().filter(|s| **s).count() == 1 && !named;
    for (c, name) in rows {
        let mut line = String::new();
        for k in 0..5 {
            if show[k] {
                if single {
                    line.push_str(&format!("{}", c[k]));
                } else {
                    line.push_str(&format!("{:>w$} ", c[k], w = width));
                }
            }
        }
        println!("{}", format!("{line}{name}").trim_end());
    }
    st
}
