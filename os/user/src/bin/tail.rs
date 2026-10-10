//! tail: print the last lines of files.
//!
//!     tail [-n [+]lines | -c [+]bytes] [-f] [-qv] [file...]   (also -NUM)

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut a: Vec<String> = args[1..].to_vec();
    for x in a.iter_mut() {
        if x.len() > 1 && x.starts_with('-') && x[1..].bytes().all(|b| b.is_ascii_digit()) {
            *x = format!("-n{}", &x[1..]);
        } else if x.len() > 1 && x.starts_with('+') && x[1..].bytes().all(|b| b.is_ascii_digit()) {
            *x = format!("-n{x}");
        }
    }
    let (o, files) = rt::getopt::parse(&a, "n:c:fqvF", "[-n [+]lines | -c [+]bytes] [-f] [file...]");
    let (spec, bytes) = match (o.get('c'), o.get('n')) {
        (Some(c), _) => (c.to_string(), true),
        (None, Some(n)) => (n.to_string(), false),
        _ => ("10".into(), false),
    };
    let from_start = spec.starts_with('+');
    let n: usize = spec.trim_start_matches(['+', '-']).parse().unwrap_or_else(|_| rt::die!("invalid number: '{spec}'"));
    let files = rt::io::inputs(&files);
    let headers = (files.len() > 1 && !o.has('q')) || o.has('v');
    let follow = o.has('f') || o.has('F');
    let mut out = rt::io::stdout();
    let mut st = 0;
    let mut last_size = 0u32;
    for (i, f) in files.iter().enumerate() {
        let mut r = match rt::io::open_input(f) {
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
        let mut v = Vec::new();
        let _ = r.read_to_end(&mut v);
        last_size = v.len() as u32;
        let start = if bytes {
            if from_start { n.saturating_sub(1).min(v.len()) } else { v.len().saturating_sub(n) }
        } else if from_start {
            let mut k = 0;
            let mut line = 1;
            while line < n && k < v.len() {
                if v[k] == b'\n' {
                    line += 1;
                }
                k += 1;
            }
            k
        } else {
            let mut k = v.len();
            let mut count = 0;
            if k > 0 && v[k - 1] == b'\n' {
                k -= 1;
            }
            while k > 0 {
                if v[k - 1] == b'\n' {
                    count += 1;
                    if count == n {
                        break;
                    }
                }
                k -= 1;
            }
            if n == 0 { v.len() } else { k }
        };
        let _ = out.write_all(&v[start..]);
    }
    if follow && files.len() == 1 && files[0] != "-" {
        let _ = out.flush();
        let f = &files[0];
        loop {
            let _ = rt::time::sleep_ms(500);
            let Ok(m) = rt::fs::metadata(f) else { continue };
            if m.len() < last_size {
                rt::warn!("{f}: file truncated");
                last_size = 0;
            }
            if m.len() > last_size {
                if let Ok(mut file) = rt::fs::File::open(f) {
                    let _ = file.seek(last_size as i32, azsys::flags::SEEK_SET);
                    let mut v = Vec::new();
                    let _ = file.read_to_end(&mut v);
                    last_size += v.len() as u32;
                    let _ = out.write_all(&v);
                    let _ = out.flush();
                }
            }
        }
    }
    st
}
