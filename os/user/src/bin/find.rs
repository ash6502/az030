//! find: search for files in a directory hierarchy.
//!
//!     find [path...] [expression]
//!
//! Tests: -name -iname -path -type [fdlcbp] -size [+-]N[ckMG] -mtime [+-]N
//!        -mmin [+-]N -newer f -user u -group g -perm [-]mode -empty -links N
//! Actions: -print -print0 -ls -exec cmd {} ; -exec cmd {} + -ok cmd {} ; -delete -prune -quit
//! Operators: ! -not -a -and -o -or ( )   Options: -maxdepth N -mindepth N -depth -xdev

#![no_std]
#![no_main]

use rt::fs::{self, Metadata};
use rt::prelude::*;

rt::main!(main);

#[derive(Clone)]
enum E {
    True,
    Name(String, bool),
    Path(String),
    Type(char),
    Size(char, u64, u64),
    Mtime(char, u32, u32),
    Newer(u32),
    User(u32),
    Group(u32),
    Perm(bool, u32),
    Empty,
    Links(char, u32),
    Print(bool),
    Ls,
    Exec(Vec<String>, bool, bool),
    Delete,
    Prune,
    Quit,
    Not(Box<E>),
    And(Box<E>, Box<E>),
    Or(Box<E>, Box<E>),
}

struct Ctx {
    now: u32,
    prune: bool,
    quit: bool,
    st: i32,
    batch: Vec<(Vec<String>, Vec<String>)>,
}

fn cmp_num(op: char, v: u64, n: u64) -> bool {
    match op {
        '+' => v > n,
        '-' => v < n,
        _ => v == n,
    }
}

fn split_num(s: &str) -> (char, &str) {
    match s.chars().next() {
        Some(c @ ('+' | '-')) => (c, &s[1..]),
        _ => ('=', s),
    }
}

struct P<'a> {
    a: &'a [String],
    i: usize,
    has_action: bool,
    maxdepth: Option<usize>,
    mindepth: usize,
    depth_first: bool,
    xdev: bool,
}

impl P<'_> {
    fn peek(&self) -> Option<&str> {
        self.a.get(self.i).map(|s| s.as_str())
    }
    fn arg(&mut self, what: &str) -> String {
        self.i += 1;
        self.a.get(self.i - 1).cloned().unwrap_or_else(|| rt::die!("missing argument to '{what}'"))
    }
    fn or(&mut self) -> E {
        let mut l = self.and();
        while matches!(self.peek(), Some("-o") | Some("-or")) {
            self.i += 1;
            let r = self.and();
            l = E::Or(Box::new(l), Box::new(r));
        }
        l
    }
    fn and(&mut self) -> E {
        let mut l = self.not();
        loop {
            match self.peek() {
                Some("-a") | Some("-and") => {
                    self.i += 1;
                }
                Some("-o") | Some("-or") | Some(")") | None => break,
                _ => {}
            }
            let r = self.not();
            l = E::And(Box::new(l), Box::new(r));
        }
        l
    }
    fn not(&mut self) -> E {
        if matches!(self.peek(), Some("!") | Some("-not")) {
            self.i += 1;
            return E::Not(Box::new(self.not()));
        }
        self.primary()
    }
    fn primary(&mut self) -> E {
        let t = self.arg("expression");
        match t.as_str() {
            "(" => {
                let e = self.or();
                if self.arg("(") != ")" {
                    rt::die!("missing ')'");
                }
                e
            }
            "-name" | "-iname" => {
                let p = self.arg(&t);
                E::Name(if t == "-iname" { p.to_lowercase() } else { p }, t == "-iname")
            }
            "-path" | "-wholename" => E::Path(self.arg(&t)),
            "-type" => E::Type(self.arg(&t).chars().next().unwrap_or('f')),
            "-size" => {
                let s = self.arg(&t);
                let (op, n) = split_num(&s);
                let (num, unit) = match n.chars().last() {
                    Some('c') => (&n[..n.len() - 1], 1),
                    Some('k') => (&n[..n.len() - 1], 1024),
                    Some('M') => (&n[..n.len() - 1], 1 << 20),
                    Some('G') => (&n[..n.len() - 1], 1 << 30),
                    Some('b') => (&n[..n.len() - 1], 512),
                    _ => (n, 512),
                };
                E::Size(op, num.parse().unwrap_or_else(|_| rt::die!("invalid -size '{s}'")), unit)
            }
            "-mtime" | "-mmin" => {
                let s = self.arg(&t);
                let (op, n) = split_num(&s);
                E::Mtime(op, n.parse().unwrap_or_else(|_| rt::die!("invalid {t} '{s}'")), if t == "-mtime" { 86400 } else { 60 })
            }
            "-newer" => {
                let f = self.arg(&t);
                E::Newer(fs::metadata(&f).map(|m| m.mtime()).unwrap_or_else(|e| rt::die!("{f}: {e}")))
            }
            "-user" => {
                let u = self.arg(&t);
                E::User(u.parse().ok().or_else(|| rt::users::by_name(&u).map(|x| x.uid)).unwrap_or_else(|| rt::die!("'{u}' is not the name of a known user")))
            }
            "-group" => {
                let g = self.arg(&t);
                E::Group(g.parse().ok().or_else(|| rt::users::group_by_name(&g).map(|x| x.gid)).unwrap_or_else(|| rt::die!("'{g}' is not the name of a known group")))
            }
            "-perm" => {
                let p = self.arg(&t);
                let (all, m) = match p.strip_prefix('-') {
                    Some(m) => (true, m),
                    None => (false, p.as_str()),
                };
                E::Perm(all, rt::util::parse_mode(m, 0, false).unwrap_or_else(|e| rt::die!("{e}")))
            }
            "-empty" => E::Empty,
            "-links" => {
                let s = self.arg(&t);
                let (op, n) = split_num(&s);
                E::Links(op, n.parse().unwrap_or(1))
            }
            "-print" => {
                self.has_action = true;
                E::Print(false)
            }
            "-print0" => {
                self.has_action = true;
                E::Print(true)
            }
            "-ls" => {
                self.has_action = true;
                E::Ls
            }
            "-exec" | "-ok" | "-execdir" => {
                self.has_action = true;
                let mut cmd = Vec::new();
                let mut plus = false;
                loop {
                    let x = self.arg(&t);
                    if x == ";" {
                        break;
                    }
                    if x == "+" && cmd.last().is_some_and(|l| l == "{}") {
                        plus = true;
                        break;
                    }
                    cmd.push(x);
                }
                if cmd.is_empty() {
                    rt::die!("missing command for {t}");
                }
                E::Exec(cmd, t == "-ok", plus)
            }
            "-delete" => {
                self.has_action = true;
                self.depth_first = true;
                E::Delete
            }
            "-prune" => E::Prune,
            "-quit" => E::Quit,
            "-true" => E::True,
            "-false" => E::Not(Box::new(E::True)),
            "-maxdepth" => {
                self.maxdepth = Some(self.arg(&t).parse().unwrap_or(0));
                E::True
            }
            "-mindepth" => {
                self.mindepth = self.arg(&t).parse().unwrap_or(0);
                E::True
            }
            "-depth" | "-d" => {
                self.depth_first = true;
                E::True
            }
            "-xdev" | "-mount" => {
                self.xdev = true;
                E::True
            }
            "-follow" | "-noleaf" => E::True,
            _ => rt::die!("unknown predicate '{t}'"),
        }
    }
}

fn type_char(m: &Metadata) -> char {
    use azsys::mode::*;
    match m.kind() {
        S_IFDIR => 'd',
        S_IFLNK => 'l',
        S_IFCHR => 'c',
        S_IFBLK => 'b',
        S_IFIFO => 'p',
        S_IFSOCK => 's',
        _ => 'f',
    }
}

fn run_cmd(cmd: &[String], paths: &[String], ask: bool) -> bool {
    let mut argv = Vec::new();
    for c in cmd {
        if c == "{}" && paths.len() > 1 {
            argv.extend(paths.iter().cloned());
        } else {
            argv.push(c.replace("{}", &paths[0]));
        }
    }
    if ask && !rt::util::confirm(&format!("< {} ... {} > ? ", argv[0], paths[0])) {
        return false;
    }
    rt::process::run(&argv).is_ok_and(|s| s.success())
}

fn eval(e: &E, path: &str, m: &Metadata, c: &mut Ctx) -> bool {
    match e {
        E::True => true,
        E::Name(p, icase) => {
            let b = rt::path::basename(path);
            if *icase { rt::glob::matches(p, &b.to_lowercase()) } else { rt::glob::matches(p, b) }
        }
        E::Path(p) => rt::glob::matches(p, path),
        E::Type(t) => type_char(m) == *t,
        E::Size(op, n, unit) => cmp_num(*op, (m.len() as u64).div_ceil(*unit), *n),
        E::Mtime(op, n, unit) => cmp_num(*op, (c.now.saturating_sub(m.mtime()) / unit) as u64, *n as u64),
        E::Newer(t) => m.mtime() > *t,
        E::User(u) => m.uid() == *u,
        E::Group(g) => m.gid() == *g,
        E::Perm(all, p) => {
            if *all { m.mode() & p == *p } else { m.mode() & 0o7777 == *p }
        }
        E::Empty => {
            if m.is_dir() {
                fs::read_dir(path).is_ok_and(|v| v.iter().all(|e| e.name == "." || e.name == ".."))
            } else {
                m.is_file() && m.is_empty()
            }
        }
        E::Links(op, n) => cmp_num(*op, m.nlink() as u64, *n as u64),
        E::Print(zero) => {
            if *zero {
                print!("{path}\0");
            } else {
                println!("{path}");
            }
            true
        }
        E::Ls => {
            println!(
                "{:7} {:4} {} {:3} {:<8} {:<8} {:8} {} {}",
                m.ino(),
                m.0.blocks / 2,
                m.mode_string(),
                m.nlink(),
                rt::users::user_name(m.uid()),
                rt::users::group_name(m.gid()),
                m.len(),
                rt::time::format_short(m.mtime(), c.now),
                path
            );
            true
        }
        E::Exec(cmd, ask, plus) => {
            if *plus {
                match c.batch.iter_mut().find(|(k, _)| k == cmd) {
                    Some((_, v)) => v.push(path.into()),
                    None => c.batch.push((cmd.clone(), vec![path.into()])),
                }
                true
            } else {
                run_cmd(cmd, &[path.into()], *ask)
            }
        }
        E::Delete => {
            let r = if m.is_dir() { fs::remove_dir(path) } else { fs::remove_file(path) };
            if let Err(e) = r {
                rt::warn!("cannot delete '{path}': {e}");
                c.st = 1;
                return false;
            }
            true
        }
        E::Prune => {
            c.prune = true;
            true
        }
        E::Quit => {
            c.quit = true;
            true
        }
        E::Not(x) => !eval(x, path, m, c),
        E::And(a, b) => eval(a, path, m, c) && eval(b, path, m, c),
        E::Or(a, b) => eval(a, path, m, c) || eval(b, path, m, c),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(path: &str, depth: usize, e: &E, p: &P, c: &mut Ctx, dev: u32) {
    if c.quit {
        return;
    }
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) => {
            rt::warn!("'{path}': {err}");
            c.st = 1;
            return;
        }
    };
    let in_range = depth >= p.mindepth && p.maxdepth.is_none_or(|d| depth <= d);
    c.prune = false;
    if in_range && !p.depth_first {
        eval(e, path, &m, c);
    }
    let pruned = c.prune;
    if m.is_dir() && !pruned && p.maxdepth.is_none_or(|d| depth < d) && !(p.xdev && m.dev() != dev) {
        match fs::read_dir(path) {
            Ok(es) => {
                for ent in es {
                    if ent.name == "." || ent.name == ".." {
                        continue;
                    }
                    let child = if path.ends_with('/') { format!("{path}{}", ent.name) } else { format!("{path}/{}", ent.name) };
                    walk(&child, depth + 1, e, p, c, dev);
                    if c.quit {
                        return;
                    }
                }
            }
            Err(err) => {
                rt::warn!("'{path}': {err}");
                c.st = 1;
            }
        }
    }
    if in_range && p.depth_first {
        eval(e, path, &m, c);
    }
}

fn main(args: &[String]) -> i32 {
    let a = &args[1..];
    let first_expr = a.iter().position(|x| x.starts_with('-') && x.len() > 1 || x == "!" || x == "(").unwrap_or(a.len());
    let paths: Vec<String> = if first_expr == 0 { vec![".".into()] } else { a[..first_expr].to_vec() };
    let ex = &a[first_expr..];
    let mut p = P { a: ex, i: 0, has_action: false, maxdepth: None, mindepth: 0, depth_first: false, xdev: false };
    let mut e = if ex.is_empty() { E::True } else { p.or() };
    if p.i < ex.len() {
        rt::die!("unexpected '{}'", ex[p.i]);
    }
    if !p.has_action {
        e = E::And(Box::new(e), Box::new(E::Print(false)));
    }
    let mut c = Ctx { now: rt::time::now(), prune: false, quit: false, st: 0, batch: Vec::new() };
    for path in &paths {
        let dev = fs::metadata(path).map(|m| m.dev()).unwrap_or(0);
        walk(path, 0, &e, &p, &mut c, dev);
    }
    for (cmd, files) in core::mem::take(&mut c.batch) {
        for chunk in files.chunks(200) {
            if !run_cmd(&cmd, chunk, false) {
                c.st = 1;
            }
        }
    }
    c.st
}
