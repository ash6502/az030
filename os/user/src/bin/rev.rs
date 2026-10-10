//! rev: reverse the characters of each line.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut st = 0;
    for f in rt::io::inputs(&args[1..]) {
        match rt::io::reader(&f) {
            Ok(r) => {
                for l in r.lines().map_while(|l| l.ok()) {
                    println!("{}", l.chars().rev().collect::<String>());
                }
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    st
}
