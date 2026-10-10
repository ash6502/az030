//! Small helpers shared by the utilities.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// `1.5K`, `23M`... (powers of 1024).
pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n}")
    } else if v < 10.0 {
        format!("{:.1}{}", v, UNITS[u])
    } else {
        format!("{:.0}{}", v, UNITS[u])
    }
}

/// Apply a chmod mode (octal, or symbolic like `u+x,go-w,a=r`) to `old`.
pub fn parse_mode(spec: &str, old: u32, is_dir: bool) -> Result<u32, String> {
    if !spec.is_empty() && spec.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return u32::from_str_radix(spec, 8).map_err(|_| format!("invalid mode: {spec}"));
    }
    let mut mode = old & 0o7777;
    for clause in spec.split(',') {
        let b = clause.as_bytes();
        let mut i = 0;
        let mut who = 0u32;
        while i < b.len() && matches!(b[i], b'u' | b'g' | b'o' | b'a') {
            who |= match b[i] {
                b'u' => 0o4700,
                b'g' => 0o2070,
                b'o' => 0o1007,
                _ => 0o7777,
            };
            i += 1;
        }
        let umask_applies = who == 0;
        if who == 0 {
            who = 0o7777;
        }
        if i >= b.len() {
            return Err(format!("invalid mode: {spec}"));
        }
        while i < b.len() {
            let op = b[i];
            if !matches!(op, b'+' | b'-' | b'=') {
                return Err(format!("invalid mode: {spec}"));
            }
            i += 1;
            let mut perm = 0u32;
            while i < b.len() && !matches!(b[i], b'+' | b'-' | b'=') {
                perm |= match b[i] {
                    b'r' => 0o444,
                    b'w' => 0o222,
                    b'x' => 0o111,
                    b'X' => {
                        if is_dir || old & 0o111 != 0 {
                            0o111
                        } else {
                            0
                        }
                    }
                    b's' => 0o6000,
                    b't' => 0o1000,
                    _ => return Err(format!("invalid mode: {spec}")),
                };
                i += 1;
            }
            let mut bits = perm & who;
            if umask_applies && op != b'-' {
                let um = crate::process::umask(0);
                crate::process::umask(um);
                bits &= !um;
            }
            match op {
                b'+' => mode |= bits,
                b'-' => mode &= !bits,
                _ => mode = (mode & !(who & 0o7777)) | bits,
            }
        }
    }
    Ok(mode)
}

/// Ask a yes/no question on the terminal (stderr prompt, answer from stdin).
pub fn confirm(prompt: &str) -> bool {
    crate::eprint!("{prompt}");
    match crate::io::stdin().line() {
        Some(l) => l.trim_start().starts_with(['y', 'Y']),
        None => false,
    }
}

/// Split text into lines, keeping track of whether the last line had a newline.
pub fn lines(text: &str) -> Vec<&str> {
    let mut v: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        v.pop();
    }
    v
}

/// Lay out names in columns for a terminal of `width` columns.
pub fn columns(names: &[String], width: usize) -> String {
    if names.is_empty() {
        return String::new();
    }
    let w = names.iter().map(|n| n.chars().count()).max().unwrap_or(1) + 2;
    let cols = (width / w).max(1);
    let rows = names.len().div_ceil(cols);
    let mut out = String::new();
    for r in 0..rows {
        for c in 0..cols {
            let i = c * rows + r;
            let Some(n) = names.get(i) else { continue };
            out.push_str(n);
            if (c + 1) * rows + r < names.len() {
                for _ in n.chars().count()..w {
                    out.push(' ');
                }
            }
        }
        out.push('\n');
    }
    out
}
