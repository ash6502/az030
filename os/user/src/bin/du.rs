//! du: estimate file space usage.
//!
//!     du [-ashkc] [-d depth] [file...]

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;

rt::main!(main);

struct O {
    all: bool,
    summary: bool,
    human: bool,
    depth: Option<usize>,
    bytes: bool,
}

fn show(n: u64, path: &str, o: &O) {
    if o.human {
        println!("{}\t{path}", rt::util::human_size(n));
    } else if o.bytes {
        println!("{n}\t{path}");
    } else {
        println!("{}\t{path}", n.div_ceil(1024));
    }
}

fn walk(path: &str, depth: usize, o: &O, seen: &mut Vec<(u32, u32)>) -> u64 {
    let Ok(m) = fs::symlink_metadata(path) else {
        rt::warn!("cannot access '{path}'");
        return 0;
    };
    if m.nlink() > 1 && !m.is_dir() {
        let key = (m.dev(), m.ino());
        if seen.contains(&key) {
            return 0;
        }
        seen.push(key);
    }
    let mut total = if o.bytes { m.len() as u64 } else { m.0.blocks as u64 * 512 };
    if m.is_dir() {
        match fs::read_dir(path) {
            Ok(es) => {
                for e in es {
                    if e.name == "." || e.name == ".." {
                        continue;
                    }
                    total += walk(&rt::path::join(path, &e.name), depth + 1, o, seen);
                }
            }
            Err(e) => rt::warn!("cannot read directory '{path}': {e}"),
        }
        if !o.summary && o.depth.is_none_or(|d| depth <= d) {
            show(total, path, o);
        }
    } else if (o.all && !o.summary && o.depth.is_none_or(|d| depth <= d)) || depth == 0 && !o.summary {
        show(total, path, o);
    }
    total
}

fn main(args: &[String]) -> i32 {
    let (p, files) = rt::getopt::parse(&args[1..], "ashkcbd:", "[-ashkcb] [-d depth] [file...]");
    let o = O { all: p.has('a'), summary: p.has('s'), human: p.has('h'), depth: p.get('d').and_then(|d| d.parse().ok()), bytes: p.has('b') };
    let files = if files.is_empty() { vec![String::from(".")] } else { files };
    let mut grand = 0;
    let mut seen = Vec::new();
    for f in &files {
        let n = walk(f, 0, &o, &mut seen);
        if o.summary {
            show(n, f, &o);
        }
        grand += n;
    }
    if p.has('c') {
        show(grand, "total", &o);
    }
    0
}
