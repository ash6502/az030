//! cmp: compare two files byte by byte.
//!
//!     cmp [-ls] file1 file2

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn read(f: &str) -> Vec<u8> {
    let mut v = Vec::new();
    match rt::io::open_input(f) {
        Ok(mut r) => {
            let _ = r.read_to_end(&mut v);
        }
        Err(e) => {
            rt::warn!("{f}: {e}");
            rt::process::exit(2);
        }
    }
    v
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "ls", "[-ls] file1 file2");
    if files.len() != 2 {
        rt::die!("usage: cmp [-ls] file1 file2");
    }
    let (a, b) = (read(&files[0]), read(&files[1]));
    let mut differ = false;
    let mut line = 1;
    for i in 0..a.len().min(b.len()) {
        if a[i] != b[i] {
            differ = true;
            if o.has('s') {
                return 1;
            }
            if o.has('l') {
                println!("{} {:o} {:o}", i + 1, a[i], b[i]);
            } else {
                println!("{} {} differ: byte {}, line {}", files[0], files[1], i + 1, line);
                return 1;
            }
        }
        if a[i] == b'\n' {
            line += 1;
        }
    }
    if a.len() != b.len() {
        if !o.has('s') {
            let short = if a.len() < b.len() { &files[0] } else { &files[1] };
            eprintln!("cmp: EOF on {short}");
        }
        return 1;
    }
    differ as i32
}
