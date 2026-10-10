//! diff: compare files line by line.
//!
//!     diff [-u | -U n | -c] [-q] [-i] [-b] [-w] [-r] [-N] file1 file2

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Eq,
    Del,
    Ins,
}

struct Opts {
    unified: Option<usize>,
    context: Option<usize>,
    brief: bool,
    icase: bool,
    space_change: bool,
    all_space: bool,
    recursive: bool,
    new_file: bool,
}

fn norm(s: &str, o: &Opts) -> String {
    let mut t: String = if o.all_space {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    } else if o.space_change {
        let mut r = String::new();
        let mut sp = false;
        for c in s.trim_end().chars() {
            if c.is_whitespace() {
                sp = true;
            } else {
                if sp {
                    r.push(' ');
                }
                sp = false;
                r.push(c);
            }
        }
        r
    } else {
        s.into()
    };
    if o.icase {
        t = t.to_lowercase();
    }
    t
}

/// Myers' O(ND) diff: the edit script as a sequence of operations.
fn myers(a: &[String], b: &[String]) -> Vec<Op> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let off = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'outer: for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { v[idx + 1] } else { v[idx - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                break 'outer;
            }
            k += 2;
        }
    }
    // backtrack
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let idx = |k: isize| (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) { k + 1 } else { k - 1 };
        let prev_x = v[idx(prev_k)];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Eq);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x {
                ops.push(Op::Ins);
            } else {
                ops.push(Op::Del);
            }
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    ops
}

struct Hunk {
    a0: usize,
    a1: usize,
    b0: usize,
    b1: usize,
}

/// Group the edit script into change hunks (ranges in a and b).
fn hunks(ops: &[Op]) -> Vec<Hunk> {
    let mut v = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut k = 0;
    while k < ops.len() {
        if ops[k] == Op::Eq {
            i += 1;
            j += 1;
            k += 1;
            continue;
        }
        let (a0, b0) = (i, j);
        while k < ops.len() && ops[k] != Op::Eq {
            match ops[k] {
                Op::Del => i += 1,
                _ => j += 1,
            }
            k += 1;
        }
        v.push(Hunk { a0, a1: i, b0, b1: j });
    }
    v
}

fn range(a: usize, b: usize) -> String {
    // 1-based, normal-format line range
    if b - a <= 1 { format!("{}", if b == a { a } else { a + 1 }) } else { format!("{},{}", a + 1, b) }
}

fn read_lines(f: &str, o: &Opts) -> Option<Vec<String>> {
    if o.new_file && !rt::fs::exists(f) {
        return Some(Vec::new());
    }
    match rt::io::open_input(f) {
        Ok(mut r) => {
            let mut v = Vec::new();
            let _ = r.read_to_end(&mut v);
            let t = String::from_utf8_lossy(&v);
            Some(rt::util::lines(&t).into_iter().map(String::from).collect())
        }
        Err(e) => {
            rt::warn!("{f}: {e}");
            None
        }
    }
}

fn mtime(f: &str) -> String {
    rt::fs::metadata(f).map(|m| rt::time::format_date(m.mtime())).unwrap_or_default()
}

/// Compare two files; returns 0 same, 1 different, 2 trouble.
fn diff_files(f1: &str, f2: &str, o: &Opts) -> i32 {
    let (Some(a), Some(b)) = (read_lines(f1, o), read_lines(f2, o)) else { return 2 };
    let (na, nb): (Vec<String>, Vec<String>) = (a.iter().map(|s| norm(s, o)).collect(), b.iter().map(|s| norm(s, o)).collect());
    let ops = myers(&na, &nb);
    let hs = hunks(&ops);
    if hs.is_empty() {
        return 0;
    }
    if o.brief {
        println!("Files {f1} and {f2} differ");
        return 1;
    }
    let mut out = String::new();
    if let Some(ctx) = o.unified.or(o.context) {
        let unified = o.unified.is_some();
        if unified {
            out.push_str(&format!("--- {f1}\t{}\n+++ {f2}\t{}\n", mtime(f1), mtime(f2)));
        } else {
            out.push_str(&format!("*** {f1}\t{}\n--- {f2}\t{}\n", mtime(f1), mtime(f2)));
        }
        // merge hunks whose contexts overlap
        let mut groups: Vec<Vec<&Hunk>> = Vec::new();
        for h in &hs {
            match groups.last_mut() {
                Some(g) if h.a0 <= g.last().unwrap().a1 + 2 * ctx => g.push(h),
                _ => groups.push(vec![h]),
            }
        }
        for g in groups {
            let (first, last) = (g[0], *g.last().unwrap());
            let a0 = first.a0.saturating_sub(ctx);
            let b0 = first.b0.saturating_sub(ctx);
            let a1 = (last.a1 + ctx).min(a.len());
            let b1 = (last.b1 + ctx).min(b.len());
            if unified {
                let r = |s: usize, e: usize| if e - s == 1 { format!("{}", s + 1) } else { format!("{},{}", if e == s { s } else { s + 1 }, e - s) };
                out.push_str(&format!("@@ -{} +{} @@\n", r(a0, a1), r(b0, b1)));
                let mut i = a0;
                for h in &g {
                    while i < h.a0 {
                        out.push_str(&format!(" {}\n", a[i]));
                        i += 1;
                    }
                    for l in &a[h.a0..h.a1] {
                        out.push_str(&format!("-{l}\n"));
                    }
                    for l in &b[h.b0..h.b1] {
                        out.push_str(&format!("+{l}\n"));
                    }
                    i = h.a1;
                }
                while i < a1 {
                    out.push_str(&format!(" {}\n", a[i]));
                    i += 1;
                }
            } else {
                out.push_str("***************\n");
                out.push_str(&format!("*** {},{} ****\n", a0 + 1, a1));
                if g.iter().any(|h| h.a1 > h.a0) {
                    let mut i = a0;
                    for h in &g {
                        while i < h.a0 {
                            out.push_str(&format!("  {}\n", a[i]));
                            i += 1;
                        }
                        let mark = if h.b1 > h.b0 { "! " } else { "- " };
                        for l in &a[h.a0..h.a1] {
                            out.push_str(&format!("{mark}{l}\n"));
                        }
                        i = h.a1;
                    }
                    while i < a1 {
                        out.push_str(&format!("  {}\n", a[i]));
                        i += 1;
                    }
                }
                out.push_str(&format!("--- {},{} ----\n", b0 + 1, b1));
                if g.iter().any(|h| h.b1 > h.b0) {
                    let mut j = b0;
                    for h in &g {
                        while j < h.b0 {
                            out.push_str(&format!("  {}\n", b[j]));
                            j += 1;
                        }
                        let mark = if h.a1 > h.a0 { "! " } else { "+ " };
                        for l in &b[h.b0..h.b1] {
                            out.push_str(&format!("{mark}{l}\n"));
                        }
                        j = h.b1;
                    }
                    while j < b1 {
                        out.push_str(&format!("  {}\n", b[j]));
                        j += 1;
                    }
                }
            }
        }
    } else {
        for h in &hs {
            let (del, ins) = (h.a1 > h.a0, h.b1 > h.b0);
            let cmd = match (del, ins) {
                (true, true) => 'c',
                (true, false) => 'd',
                _ => 'a',
            };
            let ar = if cmd == 'a' { format!("{}", h.a0) } else { range(h.a0, h.a1) };
            let br = if cmd == 'd' { format!("{}", h.b0) } else { range(h.b0, h.b1) };
            out.push_str(&format!("{ar}{cmd}{br}\n"));
            for l in &a[h.a0..h.a1] {
                out.push_str(&format!("< {l}\n"));
            }
            if del && ins {
                out.push_str("---\n");
            }
            for l in &b[h.b0..h.b1] {
                out.push_str(&format!("> {l}\n"));
            }
        }
    }
    print!("{out}");
    1
}

fn diff_dirs(d1: &str, d2: &str, o: &Opts) -> i32 {
    let names = |d: &str| -> Vec<String> {
        let mut v: Vec<String> = rt::fs::read_dir(d).map(|es| es.into_iter().map(|e| e.name).filter(|n| n != "." && n != "..").collect()).unwrap_or_default();
        v.sort();
        v
    };
    let (n1, n2) = (names(d1), names(d2));
    let mut all: Vec<String> = n1.iter().chain(n2.iter()).cloned().collect();
    all.sort();
    all.dedup();
    let mut st = 0;
    for n in all {
        let (p1, p2) = (rt::path::join(d1, &n), rt::path::join(d2, &n));
        let (e1, e2) = (n1.contains(&n), n2.contains(&n));
        if !(e1 && e2) && !o.new_file {
            println!("Only in {}: {n}", if e1 { d1 } else { d2 });
            st = st.max(1);
            continue;
        }
        let (dir1, dir2) = (rt::fs::is_dir(&p1), rt::fs::is_dir(&p2));
        let r = if dir1 && dir2 {
            if o.recursive { diff_dirs(&p1, &p2, o) } else {
                println!("Common subdirectories: {p1} and {p2}");
                0
            }
        } else if dir1 != dir2 && e1 && e2 {
            println!("File {p1} is a {} while file {p2} is a {}", if dir1 { "directory" } else { "regular file" }, if dir2 { "directory" } else { "regular file" });
            1
        } else {
            let before = st;
            let _ = before;
            if o.unified.is_none() && o.context.is_none() && !o.brief {
                println!("diff {p1} {p2}");
            }
            diff_files(&p1, &p2, o)
        };
        st = st.max(r);
    }
    st
}

fn main(args: &[String]) -> i32 {
    let (p, files) = rt::getopt::parse(&args[1..], "uU:cC:qibwrNa", "[-u | -U n | -c] [-qibwrN] file1 file2");
    if files.len() != 2 {
        eprintln!("usage: diff [-u | -U n | -c] [-qibwrN] file1 file2");
        return 2;
    }
    let o = Opts {
        unified: p.get('U').and_then(|n| n.parse().ok()).or(if p.has('u') { Some(3) } else { None }),
        context: p.get('C').and_then(|n| n.parse().ok()).or(if p.has('c') { Some(3) } else { None }),
        brief: p.has('q'),
        icase: p.has('i'),
        space_change: p.has('b'),
        all_space: p.has('w'),
        recursive: p.has('r'),
        new_file: p.has('N'),
    };
    let (mut f1, mut f2) = (files[0].clone(), files[1].clone());
    let (d1, d2) = (rt::fs::is_dir(&f1), rt::fs::is_dir(&f2));
    if d1 && d2 {
        return diff_dirs(&f1, &f2, &o);
    }
    if d1 {
        f1 = rt::path::join(&f1, rt::path::basename(&f2));
    } else if d2 {
        f2 = rt::path::join(&f2, rt::path::basename(&f1));
    }
    diff_files(&f1, &f2, &o)
}
