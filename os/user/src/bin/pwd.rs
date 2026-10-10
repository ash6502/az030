//! pwd: print the working directory.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(_: &[String]) -> i32 {
    match rt::env::current_dir() {
        Ok(d) => {
            println!("{d}");
            0
        }
        Err(e) => rt::die!("{e}"),
    }
}
