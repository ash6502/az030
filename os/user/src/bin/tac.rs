//! tac: print files in reverse line order.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut st = 0;
    let mut out = String::new();
    for f in rt::io::inputs(&args[1..]) {
        match rt::io::open_input(&f) {
            Ok(mut r) => {
                let mut v = Vec::new();
                let _ = r.read_to_end(&mut v);
                let t = String::from_utf8_lossy(&v);
                for l in rt::util::lines(&t).iter().rev() {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    print!("{out}");
    st
}
