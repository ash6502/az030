//! tee: copy standard input to standard output and files.
//!
//!     tee [-ai] [file...]

#![no_std]
#![no_main]

use rt::fs::File;
use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "ai", "[-ai] [file...]");
    if o.has('i') {
        let _ = rt::signal::signal(rt::signal::SIGINT, rt::signal::Handler::Ignore);
    }
    let mut st = 0;
    let mut outs: Vec<File> = Vec::new();
    for f in &files {
        let r = if o.has('a') { File::append(f) } else { File::create(f) };
        match r {
            Ok(f) => outs.push(f),
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    let mut buf = vec![0u8; 8192];
    loop {
        let n = match rt::io::read_fd(0, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if rt::io::write_fd(1, &buf[..n]).is_err() {
            st = 1;
        }
        for f in outs.iter_mut() {
            if f.write_all(&buf[..n]).is_err() {
                st = 1;
            }
        }
    }
    st
}
