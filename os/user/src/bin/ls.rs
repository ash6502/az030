//! ls: list directory contents.
//!
//!     ls [-1aAdFhilnRrStuCG] [file...]

#![no_std]
#![no_main]

use rt::fs::{self, Metadata};
use rt::prelude::*;

rt::main!(main);

struct Opts {
    all: bool,
    almost: bool,
    long: bool,
    dir: bool,
    classify: bool,
    human: bool,
    inode: bool,
    numeric: bool,
    recursive: bool,
    reverse: bool,
    by_size: bool,
    by_time: bool,
    one: bool,
    color: bool,
    width: usize,
}

struct Entry {
    name: String,
    path: String,
    meta: Option<Metadata>,
}

fn color_of(m: &Metadata) -> &'static str {
    use azsys::mode::*;
    match m.kind() {
        S_IFDIR => "\x1b[1;34m",
        S_IFLNK => "\x1b[1;36m",
        S_IFCHR | S_IFBLK => "\x1b[1;33m",
        S_IFIFO => "\x1b[33m",
        _ if m.mode() & 0o111 != 0 => "\x1b[1;32m",
        _ => "",
    }
}

fn suffix(m: &Metadata) -> &'static str {
    use azsys::mode::*;
    match m.kind() {
        S_IFDIR => "/",
        S_IFLNK => "@",
        S_IFIFO => "|",
        S_IFSOCK => "=",
        _ if m.mode() & 0o111 != 0 => "*",
        _ => "",
    }
}

fn display_name(e: &Entry, o: &Opts) -> String {
    let mut s = String::new();
    let col = match (&e.meta, o.color) {
        (Some(m), true) => color_of(m),
        _ => "",
    };
    s.push_str(col);
    s.push_str(&e.name);
    if !col.is_empty() {
        s.push_str("\x1b[0m");
    }
    if o.classify {
        if let Some(m) = &e.meta {
            s.push_str(suffix(m));
        }
    }
    s
}

fn plain_len(e: &Entry, o: &Opts) -> usize {
    e.name.chars().count() + if o.classify { e.meta.as_ref().map_or(0, |m| suffix(m).len()) } else { 0 }
}

fn sort(v: &mut [Entry], o: &Opts) {
    v.sort_by(|a, b| {
        let (ma, mb) = (a.meta.as_ref(), b.meta.as_ref());
        let ord = if o.by_size {
            mb.map_or(0, |m| m.len()).cmp(&ma.map_or(0, |m| m.len())).then(a.name.cmp(&b.name))
        } else if o.by_time {
            mb.map_or(0, |m| m.mtime()).cmp(&ma.map_or(0, |m| m.mtime())).then(a.name.cmp(&b.name))
        } else {
            a.name.cmp(&b.name)
        };
        if o.reverse { ord.reverse() } else { ord }
    });
}

fn print_entries(v: &[Entry], o: &Opts) {
    if v.is_empty() {
        return;
    }
    if o.long {
        let now = rt::time::now();
        let mut users = BTreeMap::new();
        let mut groups = BTreeMap::new();
        let rows: Vec<[String; 7]> = v
            .iter()
            .map(|e| {
                let Some(m) = &e.meta else { return Default::default() };
                let user = if o.numeric {
                    format!("{}", m.uid())
                } else {
                    users.entry(m.uid()).or_insert_with(|| rt::users::user_name(m.uid())).clone()
                };
                let group = if o.numeric {
                    format!("{}", m.gid())
                } else {
                    groups.entry(m.gid()).or_insert_with(|| rt::users::group_name(m.gid())).clone()
                };
                use azsys::mode::*;
                let size = if matches!(m.kind(), S_IFCHR | S_IFBLK) {
                    format!("{}, {}", m.rdev() >> 8, m.rdev() & 0xFF)
                } else if o.human {
                    rt::util::human_size(m.len() as u64)
                } else {
                    format!("{}", m.len())
                };
                [m.mode_string(), format!("{}", m.nlink()), user, group, size, rt::time::format_short(m.mtime(), now), format!("{}", m.ino())]
            })
            .collect();
        let w = |k: usize| rows.iter().map(|r| r[k].len()).max().unwrap_or(0);
        let (w1, w2, w3, w4, w6) = (w(1), w(2), w(3), w(4), w(6));
        let mut out = String::new();
        for (e, r) in v.iter().zip(&rows) {
            if e.meta.is_none() {
                continue;
            }
            if o.inode {
                out.push_str(&format!("{:>w6$} ", r[6]));
            }
            out.push_str(&format!("{} {:>w1$} {:<w2$} {:<w3$} {:>w4$} {} {}", r[0], r[1], r[2], r[3], r[4], r[5], display_name(e, o)));
            if e.meta.as_ref().is_some_and(|m| m.is_symlink()) {
                if let Ok(t) = fs::read_link(&e.path) {
                    out.push_str(" -> ");
                    out.push_str(&t);
                }
            }
            out.push('\n');
        }
        print!("{out}");
        return;
    }
    let names: Vec<String> = v
        .iter()
        .map(|e| {
            let ino = if o.inode { format!("{} ", e.meta.as_ref().map_or(0, |m| m.ino())) } else { String::new() };
            format!("{ino}{}", display_name(e, o))
        })
        .collect();
    if o.one {
        for n in names {
            println!("{n}");
        }
        return;
    }
    // columns, padding by the visible width
    let widths: Vec<usize> = v.iter().map(|e| plain_len(e, o) + if o.inode { e.meta.as_ref().map_or(1, |m| format!("{}", m.ino()).len()) + 1 } else { 0 }).collect();
    let colw = widths.iter().max().copied().unwrap_or(1) + 2;
    let cols = (o.width / colw).max(1);
    let rows = names.len().div_ceil(cols);
    let mut out = String::new();
    for r in 0..rows {
        for c in 0..cols {
            let i = c * rows + r;
            if i >= names.len() {
                continue;
            }
            out.push_str(&names[i]);
            if (c + 1) * rows + r < names.len() {
                for _ in widths[i]..colw {
                    out.push(' ');
                }
            }
        }
        out.push('\n');
    }
    print!("{out}");
}

fn list_dir(path: &str, o: &Opts, header: bool, st: &mut i32) {
    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            rt::warn!("{path}: {e}");
            *st = 1;
            return;
        }
    };
    let mut v: Vec<Entry> = entries
        .into_iter()
        .filter(|e| o.all || (o.almost && e.name != "." && e.name != "..") || !e.name.starts_with('.'))
        .map(|e| {
            let p = rt::path::join(path, &e.name);
            let meta = fs::symlink_metadata(&p).ok();
            Entry { name: e.name, path: p, meta }
        })
        .collect();
    sort(&mut v, o);
    if header {
        println!("{path}:");
    }
    if o.long {
        let blocks: u32 = v.iter().filter_map(|e| e.meta.as_ref()).map(|m| m.0.blocks).sum();
        println!("total {}", blocks / 2);
    }
    print_entries(&v, o);
    if o.recursive {
        for e in &v {
            if e.name == "." || e.name == ".." {
                continue;
            }
            if e.meta.as_ref().is_some_and(|m| m.is_dir()) {
                println!();
                list_dir(&e.path, o, true, st);
            }
        }
    }
}

fn main(args: &[String]) -> i32 {
    let (p, files) = rt::getopt::parse(&args[1..], "1aAdFhilnRrStuCG", "[-1aAdFhilnRrStCG] [file...]");
    let tty = rt::io::isatty(1);
    let o = Opts {
        all: p.has('a'),
        almost: p.has('A'),
        long: p.has('l') || p.has('n'),
        dir: p.has('d'),
        classify: p.has('F'),
        human: p.has('h'),
        inode: p.has('i'),
        numeric: p.has('n'),
        recursive: p.has('R'),
        reverse: p.has('r'),
        by_size: p.has('S'),
        by_time: p.has('t'),
        one: p.has('1') || (!tty && !p.has('C')),
        color: p.has('G') || (tty && rt::env::var("TERM").is_some_and(|t| t != "dumb")),
        width: rt::term::size(1).1 as usize,
    };
    let files = if files.is_empty() { vec![String::from(".")] } else { files };
    let mut st = 0;
    let mut plain = Vec::new();
    let mut dirs = Vec::new();
    for f in &files {
        match fs::symlink_metadata(f) {
            Ok(m) => {
                // follow a symlink to a directory given on the command line
                let target_dir = m.is_symlink() && !o.long && fs::is_dir(f);
                if (m.is_dir() || target_dir) && !o.dir {
                    dirs.push(f.clone());
                } else {
                    plain.push(Entry { name: f.clone(), path: f.clone(), meta: Some(m) });
                }
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    sort(&mut plain, &o);
    print_entries(&plain, &o);
    dirs.sort();
    let many = files.len() > 1 || o.recursive;
    for (i, d) in dirs.iter().enumerate() {
        if i > 0 || !plain.is_empty() {
            println!();
        }
        list_dir(d, &o, many, &mut st);
    }
    st
}
