//! Shell-style wildcard patterns: `*`, `?`, `[abc]`, `[a-z]`, `[!x]`, `\` escapes.

use alloc::string::String;
use alloc::vec::Vec;

/// Does the pattern contain unescaped wildcards?
pub fn has_wildcards(p: &str) -> bool {
    let mut esc = false;
    for c in p.chars() {
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if matches!(c, '*' | '?' | '[') {
            return true;
        }
    }
    false
}

/// Remove backslash escapes.
pub fn unescape(p: &str) -> String {
    let mut s = String::new();
    let mut esc = false;
    for c in p.chars() {
        if !esc && c == '\\' {
            esc = true;
        } else {
            s.push(c);
            esc = false;
        }
    }
    s
}

/// Match a bracket expression starting after `[`; returns (matched, chars consumed
/// including the closing `]`), or None if there is no closing `]`.
fn bracket(p: &[char], c: char) -> Option<(bool, usize)> {
    let mut i = 0;
    let neg = matches!(p.first(), Some('!') | Some('^'));
    if neg {
        i += 1;
    }
    let mut hit = false;
    let mut first = true;
    while i < p.len() {
        if p[i] == ']' && !first {
            return Some((hit != neg, i + 1));
        }
        first = false;
        let mut lo = p[i];
        if lo == '\\' && i + 1 < p.len() {
            i += 1;
            lo = p[i];
        }
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            let hi = p[i + 2];
            if lo <= c && c <= hi {
                hit = true;
            }
            i += 3;
        } else {
            if lo == c {
                hit = true;
            }
            i += 1;
        }
    }
    None
}

fn match_from(p: &[char], s: &[char]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, si));
                    pi += 1;
                    continue;
                }
                '?' => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                '[' => {
                    if let Some((ok, n)) = bracket(&p[pi + 1..], s[si]) {
                        if ok {
                            pi += 1 + n;
                            si += 1;
                            continue;
                        }
                    } else if s[si] == '[' {
                        pi += 1;
                        si += 1;
                        continue;
                    }
                }
                '\\' if pi + 1 < p.len() => {
                    if p[pi + 1] == s[si] {
                        pi += 2;
                        si += 1;
                        continue;
                    }
                }
                c => {
                    if c == s[si] {
                        pi += 1;
                        si += 1;
                        continue;
                    }
                }
            }
        }
        match star {
            Some((sp, ss)) => {
                pi = sp + 1;
                si = ss + 1;
                star = Some((sp, ss + 1));
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Does `s` match the whole pattern?
pub fn matches(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = s.chars().collect();
    match_from(&p, &s)
}

/// Expand a path pattern against the file system. Returns the sorted matches, or
/// an empty list if nothing matched. A leading `.` in a name must be matched
/// explicitly.
pub fn expand(pattern: &str) -> Vec<String> {
    let abs = pattern.starts_with('/');
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    let mut cur: Vec<String> = alloc::vec![if abs { "/".into() } else { String::new() }];
    for (k, comp) in comps.iter().enumerate() {
        let last = k == comps.len() - 1;
        let mut next = Vec::new();
        for base in &cur {
            let dir = if base.is_empty() { "." } else { base.as_str() };
            if !has_wildcards(comp) {
                let p = crate::path::join(base, &unescape(comp));
                if !last || crate::fs::symlink_metadata(&p).is_ok() {
                    if last || crate::fs::is_dir(&p) {
                        next.push(p);
                    }
                }
                continue;
            }
            let Ok(entries) = crate::fs::read_dir(dir) else { continue };
            let mut names: Vec<String> = entries
                .into_iter()
                .map(|e| e.name)
                .filter(|n| n != "." && n != "..")
                .filter(|n| !n.starts_with('.') || comp.starts_with('.'))
                .filter(|n| matches(comp, n))
                .collect();
            names.sort();
            for n in names {
                let p = crate::path::join(base, &n);
                if last || crate::fs::is_dir(&p) {
                    next.push(p);
                }
            }
        }
        cur = next;
        if cur.is_empty() {
            break;
        }
    }
    if pattern.ends_with('/') {
        for p in cur.iter_mut() {
            p.push('/');
        }
    }
    cur.retain(|p| !p.is_empty());
    cur
}
