//! grep: search files for lines matching a pattern.
//!
//!     grep [-EFGivwxcnlLhHqsrRo] [-e pattern]... [-f file] [-A n] [-B n] [-C n] [pattern] [file...]
//!     egrep / fgrep  (as grep -E / grep -F)

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::regex::Regex;

rt::main!(main);

enum Pat {
    Re(Regex),
    Fixed(Vec<char>),
}

struct G {
    pats: Vec<Pat>,
    icase: bool,
    invert: bool,
    word: bool,
    line: bool,
    count: bool,
    number: bool,
    files_with: bool,
    files_without: bool,
    quiet: bool,
    only: bool,
    names: bool,
    after: usize,
    before: usize,
    color: bool,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl G {
    /// All match ranges (character indices) in a line.
    fn matches(&self, s: &[char]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for p in &self.pats {
            let mut from = 0;
            while from <= s.len() {
                let m = match p {
                    Pat::Re(r) => r.find_chars(s, from).and_then(|c| c[0]),
                    Pat::Fixed(f) => {
                        if f.len() > s.len() {
                            None
                        } else {
                            (from..=s.len() - f.len())
                                .find(|&i| (0..f.len()).all(|k| s[i + k] == f[k] || (self.icase && fold(s[i + k]) == fold(f[k]))))
                                .map(|i| (i, i + f.len()))
                        }
                    }
                };
                let Some((a, b)) = m else { break };
                let ok_word = !self.word || ((a == 0 || !is_word(s[a - 1])) && (b == s.len() || !is_word(s[b])));
                let ok_line = !self.line || (a == 0 && b == s.len());
                if ok_word && ok_line {
                    out.push((a, b));
                }
                from = if b > a { b } else { a + 1 };
                if self.line {
                    break;
                }
            }
        }
        out.sort();
        out
    }

    fn search(&self, name: &str, display: &str) -> (usize, bool) {
        let mut r = match rt::io::reader(name) {
            Ok(r) => r,
            Err(e) => {
                if !self.quiet {
                    rt::warn!("{name}: {e}");
                }
                return (0, true);
            }
        };
        let mut out = rt::io::stdout();
        let mut count = 0;
        let mut lineno = 0;
        let mut before: VecDeque<(usize, String)> = VecDeque::new();
        let mut after_left = 0;
        let mut last_printed = 0usize;
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match r.read_until(b'\n', &mut raw) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    rt::warn!("{name}: {e}");
                    return (count, true);
                }
            }
            if raw.last() == Some(&b'\n') {
                raw.pop();
            }
            lineno += 1;
            let text = String::from_utf8_lossy(&raw).into_owned();
            let chars: Vec<char> = text.chars().collect();
            let ms = self.matches(&chars);
            let hit = ms.is_empty() == self.invert;
            if !hit {
                if after_left > 0 {
                    after_left -= 1;
                    self.emit(&mut out, display, lineno, &text, &[], '-');
                    last_printed = lineno;
                } else if self.before > 0 {
                    before.push_back((lineno, text));
                    if before.len() > self.before {
                        before.pop_front();
                    }
                }
                continue;
            }
            count += 1;
            if self.quiet {
                return (count, false);
            }
            if self.files_with || self.files_without {
                break;
            }
            if self.count {
                continue;
            }
            if (self.before > 0 || self.after > 0) && last_printed > 0 {
                let first = before.front().map_or(lineno, |b| b.0);
                if first > last_printed + 1 {
                    let _ = out.write_all(b"--\n");
                }
            }
            for (n, t) in before.drain(..) {
                self.emit(&mut out, display, n, &t, &[], '-');
            }
            if self.only && !self.invert {
                for &(a, b) in &ms {
                    let s: String = chars[a..b].iter().collect();
                    self.emit(&mut out, display, lineno, &s, &[], ':');
                }
            } else {
                self.emit(&mut out, display, lineno, &text, if self.invert { &[] } else { &ms }, ':');
            }
            last_printed = lineno;
            after_left = self.after;
        }
        if self.count {
            if self.names {
                println!("{display}:{count}");
            } else {
                println!("{count}");
            }
        }
        if self.files_with && count > 0 {
            println!("{display}");
        }
        if self.files_without && count == 0 {
            println!("{display}");
        }
        (count, false)
    }

    fn emit(&self, out: &mut rt::io::Stdout, name: &str, n: usize, text: &str, ms: &[(usize, usize)], sep: char) {
        let mut s = String::new();
        if self.names {
            if self.color {
                s.push_str(&format!("\x1b[35m{name}\x1b[36m{sep}\x1b[0m"));
            } else {
                s.push_str(&format!("{name}{sep}"));
            }
        }
        if self.number {
            if self.color {
                s.push_str(&format!("\x1b[32m{n}\x1b[36m{sep}\x1b[0m"));
            } else {
                s.push_str(&format!("{n}{sep}"));
            }
        }
        if self.color && !ms.is_empty() {
            let chars: Vec<char> = text.chars().collect();
            let mut pos = 0;
            for &(a, b) in ms {
                if a < pos {
                    continue;
                }
                s.extend(&chars[pos..a]);
                s.push_str("\x1b[1;31m");
                s.extend(&chars[a..b]);
                s.push_str("\x1b[0m");
                pos = b;
            }
            s.extend(&chars[pos..]);
        } else {
            s.push_str(text);
        }
        s.push('\n');
        let _ = out.write_all(s.as_bytes());
    }
}

fn walk(path: &str, out: &mut Vec<String>) {
    match rt::fs::read_dir(path) {
        Ok(es) => {
            let mut names: Vec<String> = es.into_iter().map(|e| e.name).filter(|n| n != "." && n != "..").collect();
            names.sort();
            for n in names {
                let p = rt::path::join(path, &n);
                if rt::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                    walk(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
        Err(e) => rt::warn!("{path}: {e}"),
    }
}

fn main(args: &[String]) -> i32 {
    let prog = rt::env::progname();
    let (o, mut rest) = rt::getopt::parse(&args[1..], "EFGivwxcnlLhHqsrRoe:f:A:B:C:", "[-EFivwxcnlLhHqrso] [-e pattern] [-f file] [-A n] [-B n] [-C n] pattern [file...]");
    let ere = o.has('E') || prog == "egrep";
    let fixed = o.has('F') || prog == "fgrep";
    let mut patterns: Vec<String> = o.all('e').iter().map(|s| s.to_string()).collect();
    if let Some(f) = o.get('f') {
        match rt::fs::read_to_string(f) {
            Ok(t) => patterns.extend(t.lines().map(String::from)),
            Err(e) => rt::die!("{f}: {e}"),
        }
    }
    if patterns.is_empty() {
        if rest.is_empty() {
            eprintln!("usage: {prog} [options] pattern [file...]");
            return 2;
        }
        patterns.extend(rest.remove(0).split('\n').map(String::from));
    }
    let icase = o.has('i');
    let mut pats = Vec::new();
    for p in &patterns {
        if fixed {
            pats.push(Pat::Fixed(p.chars().collect()));
        } else {
            match Regex::new(p, ere, icase) {
                Ok(r) => pats.push(Pat::Re(r)),
                Err(e) => {
                    rt::warn!("{p}: {e}");
                    return 2;
                }
            }
        }
    }
    let recursive = o.has('r') || o.has('R');
    let mut files = rest;
    if recursive {
        if files.is_empty() {
            files.push(".".into());
        }
        let mut expanded = Vec::new();
        for f in files {
            if rt::fs::is_dir(&f) {
                walk(&f, &mut expanded);
            } else {
                expanded.push(f);
            }
        }
        files = expanded;
    }
    let ctx = o.get('C').and_then(|s| s.parse().ok()).unwrap_or(0);
    let g = G {
        pats,
        icase,
        invert: o.has('v'),
        word: o.has('w'),
        line: o.has('x'),
        count: o.has('c'),
        number: o.has('n'),
        files_with: o.has('l'),
        files_without: o.has('L'),
        quiet: o.has('q'),
        only: o.has('o'),
        names: (files.len() > 1 || recursive || o.has('H')) && !o.has('h'),
        after: o.get('A').and_then(|s| s.parse().ok()).unwrap_or(ctx),
        before: o.get('B').and_then(|s| s.parse().ok()).unwrap_or(ctx),
        color: rt::io::isatty(1),
    };
    let quiet_errors = o.has('s');
    let files = rt::io::inputs(&files);
    let mut found = false;
    let mut err = false;
    for f in &files {
        let display = if f == "-" { "(standard input)" } else { f.as_str() };
        if !quiet_errors || rt::fs::exists(f) || f == "-" {
            let (n, e) = g.search(f, display);
            found |= n > 0;
            err |= e;
            if g.quiet && found {
                return 0;
            }
        } else {
            err = true;
        }
    }
    if found { 0 } else if err { 2 } else { 1 }
}
