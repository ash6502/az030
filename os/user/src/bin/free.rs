//! free: display memory usage.
//!
//!     free [-h | -m | -k]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, _) = rt::getopt::parse(&args[1..], "hmk", "[-h | -m | -k]");
    let s = rt::process::sysinfo();
    let f = |kb: u32| -> String {
        if o.has('h') {
            rt::util::human_size(kb as u64 * 1024)
        } else if o.has('m') {
            format!("{}", kb / 1024)
        } else {
            format!("{kb}")
        }
    };
    let used = s.total_kb - s.free_kb;
    println!("{:>7} {:>10} {:>10} {:>10} {:>10} {:>10}", "", "total", "used", "free", "kernel", "buffers");
    println!("{:>7} {:>10} {:>10} {:>10} {:>10} {:>10}", "Mem:", f(s.total_kb), f(used), f(s.free_kb), f(s.kernel_kb), f(s.buffers_kb));
    0
}
