//! tr: translate or delete characters.
//!
//!     tr [-cds] set1 [set2]
//!
//! Sets: literal characters, ranges `a-z`, escapes `\n \t \\ \NNN`, classes
//! `[:alpha:] [:digit:] [:alnum:] [:upper:] [:lower:] [:space:] [:punct:]`, and
//! `[c*n]` repeats in set2.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn expand(s: &str, is_set2: bool, len1: usize) -> Result<Vec<char>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let esc = |c: &[char], i: &mut usize| -> char {
        // at a backslash
        *i += 1;
        let Some(&n) = c.get(*i) else { return '\\' };
        *i += 1;
        match n {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0c',
            'v' => '\x0b',
            '0'..='7' => {
                let mut v = n as u32 - '0' as u32;
                for _ in 0..2 {
                    match c.get(*i) {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + (*d as u32 - '0' as u32);
                            *i += 1;
                        }
                        _ => break,
                    }
                }
                char::from_u32(v).unwrap_or('?')
            }
            x => x,
        }
    };
    while i < c.len() {
        if c[i] == '[' && c.get(i + 1) == Some(&':') {
            let rest: String = c[i + 2..].iter().collect();
            if let Some(end) = rest.find(":]") {
                let name = &rest[..end];
                let f: fn(char) -> bool = match name {
                    "alpha" => |c| c.is_ascii_alphabetic(),
                    "digit" => |c| c.is_ascii_digit(),
                    "alnum" => |c| c.is_ascii_alphanumeric(),
                    "upper" => |c| c.is_ascii_uppercase(),
                    "lower" => |c| c.is_ascii_lowercase(),
                    "space" => |c| c.is_ascii_whitespace(),
                    "blank" => |c| c == ' ' || c == '\t',
                    "punct" => |c| c.is_ascii_punctuation(),
                    "cntrl" => |c| c.is_ascii_control(),
                    "print" => |c| (' '..='~').contains(&c),
                    "graph" => |c| ('!'..='~').contains(&c),
                    "xdigit" => |c| c.is_ascii_hexdigit(),
                    _ => return Err(format!("invalid class [:{name}:]")),
                };
                out.extend((0u8..128).map(|b| b as char).filter(|c| f(*c)));
                i += 2 + end + 2;
                continue;
            }
        }
        if is_set2 && c[i] == '[' {
            // [c*n] / [c*]
            if let Some(close) = c[i..].iter().position(|&x| x == ']') {
                let inner: String = c[i + 1..i + close].iter().collect();
                if let Some((ch, n)) = inner.split_once('*') {
                    if ch.chars().count() == 1 {
                        let ch = ch.chars().next().unwrap();
                        let n: usize = if n.is_empty() { len1.saturating_sub(out.len()) } else { n.parse().map_err(|_| "invalid repeat count")? };
                        out.extend(core::iter::repeat_n(ch, n));
                        i += close + 1;
                        continue;
                    }
                }
            }
        }
        let a = if c[i] == '\\' { esc(&c, &mut i) } else {
            i += 1;
            c[i - 1]
        };
        if i + 1 < c.len() && c[i] == '-' {
            i += 1;
            let b = if c[i] == '\\' { esc(&c, &mut i) } else {
                i += 1;
                c[i - 1]
            };
            if b < a {
                return Err(format!("range-endpoints of '{a}-{b}' are in reverse order"));
            }
            out.extend((a as u32..=b as u32).filter_map(char::from_u32));
        } else {
            out.push(a);
        }
    }
    Ok(out)
}

fn main(args: &[String]) -> i32 {
    let (o, sets) = rt::getopt::parse(&args[1..], "cCdst", "[-cds] set1 [set2]");
    let delete = o.has('d');
    let squeeze = o.has('s');
    let complement = o.has('c') || o.has('C');
    if sets.is_empty() || (!delete && !squeeze && sets.len() < 2) {
        rt::die!("missing operand");
    }
    let mut s1 = match expand(&sets[0], false, 0) {
        Ok(s) => s,
        Err(e) => rt::die!("{e}"),
    };
    if complement {
        s1 = (0u32..256).filter_map(char::from_u32).filter(|c| !s1.contains(c)).collect();
    }
    let s2 = match sets.get(1) {
        Some(s) => match expand(s, true, s1.len()) {
            Ok(s) => s,
            Err(e) => rt::die!("{e}"),
        },
        None => Vec::new(),
    };
    let mut map: BTreeMap<char, char> = BTreeMap::new();
    if !delete && !s2.is_empty() {
        for (i, c) in s1.iter().enumerate() {
            map.insert(*c, *s2.get(i).unwrap_or(s2.last().unwrap()));
        }
    }
    let squeeze_set: &[char] = if delete || s2.is_empty() { &s1 } else { &s2 };
    let mut input = Vec::new();
    let _ = rt::io::FdIo(0).read_to_end(&mut input);
    let text = String::from_utf8_lossy(&input);
    let mut out = String::with_capacity(text.len());
    let mut last: Option<char> = None;
    for c in text.chars() {
        if delete && s1.contains(&c) {
            continue;
        }
        let c = if delete { c } else { map.get(&c).copied().unwrap_or(c) };
        if squeeze && last == Some(c) && squeeze_set.contains(&c) {
            continue;
        }
        out.push(c);
        last = Some(c);
    }
    print!("{out}");
    0
}
