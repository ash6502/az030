//! sha256sum: compute or check SHA-256 checksums.
//!
//!     sha256sum [-c] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::sha256::{self, Sha256};

rt::main!(main);

fn hash(f: &str) -> rt::Result<String> {
    let mut r = rt::io::open_input(f)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(sha256::hex(&h.finish()))
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "cb", "[-c] [file...]");
    let mut st = 0;
    if o.has('c') {
        for list in rt::io::inputs(&files) {
            let text = match rt::fs::read_to_string(&list) {
                Ok(t) => t,
                Err(e) => rt::die!("{list}: {e}"),
            };
            for l in text.lines() {
                let Some((sum, name)) = l.split_once("  ").or(l.split_once(" *")) else { continue };
                match hash(name) {
                    Ok(h) if h == sum => println!("{name}: OK"),
                    Ok(_) => {
                        println!("{name}: FAILED");
                        st = 1;
                    }
                    Err(e) => {
                        println!("{name}: FAILED open or read ({e})");
                        st = 1;
                    }
                }
            }
        }
        return st;
    }
    for f in rt::io::inputs(&files) {
        match hash(&f) {
            Ok(h) => println!("{h}  {f}"),
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    st
}
