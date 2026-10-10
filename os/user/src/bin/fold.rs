//! fold: wrap lines to a width.
//!
//!     fold [-bs] [-w width] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "bsw:", "[-bs] [-w width] [file...]");
    let width = o.num('w', 80).unwrap_or(80).max(1) as usize;
    let mut st = 0;
    for f in rt::io::inputs(&files) {
        let r = match rt::io::reader(&f) {
            Ok(r) => r,
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
                continue;
            }
        };
        for l in r.lines().map_while(|l| l.ok()) {
            let mut chars: Vec<char> = l.chars().collect();
            while chars.len() > width {
                let mut cut = width;
                if o.has('s') {
                    if let Some(sp) = chars[..width].iter().rposition(|c| *c == ' ') {
                        cut = sp + 1;
                    }
                }
                println!("{}", chars[..cut].iter().collect::<String>());
                chars.drain(..cut);
            }
            println!("{}", chars.iter().collect::<String>());
        }
    }
    st
}
