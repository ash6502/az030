//! mkdir: make directories.
//!
//!     mkdir [-pv] [-m mode] dir...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, dirs) = rt::getopt::parse(&args[1..], "pvm:", "[-pv] [-m mode] dir...");
    if dirs.is_empty() {
        rt::die!("missing operand");
    }
    let mode = match o.get('m') {
        Some(m) => match rt::util::parse_mode(m, 0o777, true) {
            Ok(m) => Some(m),
            Err(e) => rt::die!("{e}"),
        },
        None => None,
    };
    let mut st = 0;
    for d in &dirs {
        let r = if o.has('p') { rt::fs::create_dir_all(d) } else { rt::fs::create_dir(d) };
        match r {
            Ok(()) => {
                if let Some(m) = mode {
                    let _ = rt::fs::chmod(d, m);
                }
                if o.has('v') {
                    println!("mkdir: created directory '{d}'");
                }
            }
            Err(e) => {
                rt::warn!("cannot create directory '{d}': {e}");
                st = 1;
            }
        }
    }
    st
}
