//! basename: strip directory and suffix from a file name.
//!
//!     basename name [suffix]
//!     basename -a [-s suffix] name...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn strip<'a>(b: &'a str, suffix: Option<&str>) -> &'a str {
    match suffix {
        Some(s) if !s.is_empty() && b.len() > s.len() && b.ends_with(s) => &b[..b.len() - s.len()],
        _ => b,
    }
}

fn main(args: &[String]) -> i32 {
    let (o, names) = rt::getopt::parse_strict(&args[1..], "as:", "name [suffix] | -a [-s suffix] name...");
    if names.is_empty() {
        rt::die!("missing operand");
    }
    if o.has('a') || o.has('s') {
        for n in &names {
            println!("{}", strip(rt::path::basename(n), o.get('s')));
        }
    } else {
        println!("{}", strip(rt::path::basename(&names[0]), names.get(1).map(|s| s.as_str())));
    }
    0
}
