//! Path strings.

use alloc::string::String;
use alloc::vec::Vec;

/// The last component (`/a/b/` -> `b`, `/` -> `/`).
pub fn basename(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { "" } else { "/" };
    }
    match t.rfind('/') {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

/// Everything before the last component (`a/b` -> `a`, `b` -> `.`, `/b` -> `/`).
pub fn dirname(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { "." } else { "/" };
    }
    match t.rfind('/') {
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() { "/" } else { d }
        }
        None => ".",
    }
}

/// The parent directory, or None for a single component or the root.
pub fn parent(p: &str) -> Option<&str> {
    let t = p.trim_end_matches('/');
    let i = t.rfind('/')?;
    let d = t[..i].trim_end_matches('/');
    Some(if d.is_empty() { "/" } else { d })
}

pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        return b.into();
    }
    let mut s = String::from(a);
    if !s.ends_with('/') {
        s.push('/');
    }
    s.push_str(b);
    s
}

/// The file name's extension, if any (`x.tar.gz` -> `gz`).
pub fn extension(p: &str) -> Option<&str> {
    let b = basename(p);
    let i = b.rfind('.')?;
    if i == 0 { None } else { Some(&b[i + 1..]) }
}

/// Lexically normalize: drop `.` and empty components, resolve `..`.
pub fn normalize(p: &str) -> String {
    let abs = p.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !abs {
                    parts.push("..");
                }
            }
            c => parts.push(c),
        }
    }
    let body = parts.join("/");
    match (abs, body.is_empty()) {
        (true, _) => alloc::format!("/{body}"),
        (false, true) => ".".into(),
        (false, false) => body,
    }
}

/// Make `p` absolute relative to the working directory.
pub fn absolute(p: &str) -> String {
    if p.starts_with('/') {
        normalize(p)
    } else {
        let cwd = crate::env::current_dir().unwrap_or_else(|_| "/".into());
        normalize(&join(&cwd, p))
    }
}
