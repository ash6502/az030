//! sync: write cached data to disk.

#![no_std]
#![no_main]

rt::main!(main);

fn main(_: &[rt::String]) -> i32 {
    rt::fs::sync();
    0
}
