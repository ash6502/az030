//! cut: select parts of lines.
//!
//!     cut -b list | -c list | -f list [-d delim] [-s] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn parse_list(s: &str) -> Option<Vec<(usize, usize)>> {
    let mut v = Vec::new();
    for part in s.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (if a.is_empty() { 1 } else { a.parse().ok()? }, if b.is_empty() { usize::MAX } else { b.parse().ok()? }),
            None => {
                let n = part.parse().ok()?;
                (n, n)
            }
        };
        if a == 0 || a > b {
            return None;
        }
        v.push((a, b));
    }
    Some(v)
}

fn selected(list: &[(usize, usize)], i: usize) -> bool {
    list.iter().any(|&(a, b)| a <= i && i <= b)
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "b:c:f:d:sn", "-b list | -c list | -f list [-d delim] [-s] [file...]");
    let (mode, spec) = match (o.get('b'), o.get('c'), o.get('f')) {
        (Some(l), None, None) => ('b', l),
        (None, Some(l), None) => ('c', l),
        (None, None, Some(l)) => ('f', l),
        _ => rt::die!("you must specify exactly one of -b, -c or -f"),
    };
    let Some(list) = parse_list(spec) else { rt::die!("invalid list '{spec}'") };
    let delim = o.get('d').map(|d| d.chars().next().unwrap_or('\t')).unwrap_or('\t');
    let mut out = rt::io::stdout();
    let mut st = 0;
    for f in rt::io::inputs(&files) {
        let r = match rt::io::reader(&f) {
            Ok(r) => r,
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
                continue;
            }
        };
        for line in r.lines() {
            let Ok(line) = line else { break };
            let res: String = match mode {
                'b' => {
                    let b: Vec<u8> = line.bytes().enumerate().filter(|(i, _)| selected(&list, i + 1)).map(|(_, b)| b).collect();
                    String::from_utf8_lossy(&b).into_owned()
                }
                'c' => line.chars().enumerate().filter(|(i, _)| selected(&list, i + 1)).map(|(_, c)| c).collect(),
                _ => {
                    if !line.contains(delim) {
                        if o.has('s') {
                            continue;
                        }
                        line.clone()
                    } else {
                        let parts: Vec<&str> = line.split(delim).enumerate().filter(|(i, _)| selected(&list, i + 1)).map(|(_, p)| p).collect();
                        parts.join(&String::from(delim))
                    }
                }
            };
            let _ = out.write_all(res.as_bytes());
            let _ = out.write_all(b"\n");
        }
    }
    st
}
