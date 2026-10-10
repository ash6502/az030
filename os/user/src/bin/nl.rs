//! nl: number lines.
//!
//!     nl [-b a|t|n] [-w width] [-s sep] [-v start] [-i incr] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "b:w:s:v:i:n:", "[-b a|t|n] [-w width] [-s sep] [-v start] [-i incr] [file...]");
    let style = o.get('b').unwrap_or("t");
    let width = o.num('w', 6).unwrap_or(6) as usize;
    let sep = o.get('s').unwrap_or("\t");
    let mut n = o.num('v', 1).unwrap_or(1);
    let incr = o.num('i', 1).unwrap_or(1);
    let fmt = o.get('n').unwrap_or("rn");
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
            let number = match style {
                "a" => true,
                "n" => false,
                _ => !l.trim().is_empty(),
            };
            if number {
                let s = match fmt {
                    "ln" => format!("{n:<width$}"),
                    "rz" => format!("{n:0width$}"),
                    _ => format!("{n:>width$}"),
                };
                println!("{s}{sep}{l}");
                n += incr;
            } else {
                println!("{:width$}{}{l}", "", if l.is_empty() { "" } else { sep });
            }
        }
    }
    st
}
