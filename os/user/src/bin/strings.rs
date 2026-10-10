//! strings: print printable character sequences in files.
//!
//!     strings [-n min] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "n:a", "[-n min] [file...]");
    let min = o.num('n', 4).unwrap_or(4) as usize;
    let mut st = 0;
    let mut out = String::new();
    for f in rt::io::inputs(&files) {
        let mut v = Vec::new();
        match rt::io::open_input(&f) {
            Ok(mut r) => {
                let _ = r.read_to_end(&mut v);
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
                continue;
            }
        }
        let mut cur = String::new();
        for b in v {
            if (0x20..0x7f).contains(&b) || b == b'\t' {
                cur.push(b as char);
            } else {
                if cur.len() >= min {
                    out.push_str(&cur);
                    out.push('\n');
                }
                cur.clear();
            }
        }
        if cur.len() >= min {
            out.push_str(&cur);
            out.push('\n');
        }
    }
    print!("{out}");
    st
}
