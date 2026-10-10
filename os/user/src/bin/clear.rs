//! clear: clear the terminal screen.

#![no_std]
#![no_main]

rt::main!(main);

fn main(_: &[rt::String]) -> i32 {
    let _ = rt::io::write_fd(1, b"\x1b[H\x1b[2J");
    0
}
