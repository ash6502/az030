//! head: print the first lines of files.
//!
//!     head [-n lines | -c bytes] [-qv] [file...]   (also -NUM)

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut a: Vec<String> = args[1..].to_vec();
    for x in a.iter_mut() {
        if x.len() > 1 && x.starts_with('-') && x[1..].bytes().all(|b| b.is_ascii_digit()) {
            *x = format!("-n{}", &x[1..]);
        }
    }
    let (o, files) = rt::getopt::parse(&a, "n:c:qv", "[-n lines | -c bytes] [file...]");
    let lines = match o.num('n', 10) {
        Ok(n) => n,
        Err(e) => rt::die!("{e}"),
    };
    let bytes = o.get('c').map(|s| s.parse::<i64>().unwrap_or_else(|_| rt::die!("invalid number of bytes: '{s}'")));
    let files = rt::io::inputs(&files);
    let headers = (files.len() > 1 && !o.has('q')) || o.has('v');
    let mut out = rt::io::stdout();
    let mut st = 0;
    for (i, f) in files.iter().enumerate() {
        let mut r = match rt::io::reader(f) {
            Ok(r) => r,
            Err(e) => {
                rt::warn!("cannot open '{f}' for reading: {e}");
                st = 1;
                continue;
            }
        };
        if headers {
            println!("{}==> {} <==", if i > 0 { "\n" } else { "" }, if f == "-" { "standard input" } else { f });
        }
        if let Some(n) = bytes {
            let mut v = Vec::new();
            let _ = r.read_to_end(&mut v);
            let n = if n < 0 { v.len().saturating_sub((-n) as usize) } else { (n as usize).min(v.len()) };
            let _ = out.write_all(&v[..n]);
            continue;
        }
        if lines < 0 {
            let mut v = Vec::new();
            let _ = r.read_to_end(&mut v);
            let text = String::from_utf8_lossy(&v);
            let ls: Vec<&str> = text.split_inclusive('\n').collect();
            let keep = ls.len().saturating_sub((-lines) as usize);
            for l in &ls[..keep] {
                let _ = out.write_all(l.as_bytes());
            }
            continue;
        }
        let mut line = Vec::new();
        for _ in 0..lines {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = out.write_all(&line);
                }
            }
        }
    }
    st
}
