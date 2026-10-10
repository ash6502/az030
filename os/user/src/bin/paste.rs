//! paste: merge lines of files.
//!
//!     paste [-s] [-d delims] file...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "sd:", "[-s] [-d delims] file...");
    let delims: Vec<char> = o.get('d').map(|d| d.replace("\\t", "\t").replace("\\n", "\n").chars().collect()).unwrap_or_else(|| vec!['\t']);
    let files = rt::io::inputs(&files);
    let mut contents: Vec<Vec<String>> = Vec::new();
    for f in &files {
        match rt::io::open_input(f) {
            Ok(mut r) => {
                let mut v = Vec::new();
                let _ = r.read_to_end(&mut v);
                let t = String::from_utf8_lossy(&v);
                contents.push(rt::util::lines(&t).into_iter().map(String::from).collect());
            }
            Err(e) => rt::die!("{f}: {e}"),
        }
    }
    let d = |i: usize| if delims.is_empty() { String::new() } else { String::from(delims[i % delims.len()]) };
    if o.has('s') {
        for c in &contents {
            let mut line = String::new();
            for (i, l) in c.iter().enumerate() {
                if i > 0 {
                    line.push_str(&d(i - 1));
                }
                line.push_str(l);
            }
            println!("{line}");
        }
        return 0;
    }
    let rows = contents.iter().map(|c| c.len()).max().unwrap_or(0);
    for r in 0..rows {
        let mut line = String::new();
        for (i, c) in contents.iter().enumerate() {
            if i > 0 {
                line.push_str(&d(i - 1));
            }
            if let Some(l) = c.get(r) {
                line.push_str(l);
            }
        }
        println!("{line}");
    }
    0
}
