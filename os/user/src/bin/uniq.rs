//! uniq: report or omit repeated lines.
//!
//!     uniq [-cdui] [-f fields] [-s chars] [input [output]]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "cduif:s:w:", "[-cdui] [-f fields] [-s chars] [input [output]]");
    let skip_f = o.num('f', 0).unwrap_or(0) as usize;
    let skip_c = o.num('s', 0).unwrap_or(0) as usize;
    let width = o.get('w').and_then(|s| s.parse::<usize>().ok());
    let input = files.first().cloned().unwrap_or_else(|| "-".into());
    let mut r = match rt::io::reader(&input) {
        Ok(r) => r,
        Err(e) => rt::die!("{input}: {e}"),
    };
    let key = |l: &str| -> String {
        let mut s = l;
        for _ in 0..skip_f {
            s = s.trim_start_matches([' ', '\t']);
            s = s.trim_start_matches(|c| c != ' ' && c != '\t');
        }
        let mut k: String = s.chars().skip(skip_c).collect();
        if let Some(w) = width {
            k = k.chars().take(w).collect();
        }
        if o.has('i') { k.to_lowercase() } else { k }
    };
    let mut out = String::new();
    let mut emit = |line: &str, n: usize| {
        if (o.has('d') && n < 2) || (o.has('u') && n > 1) {
            return;
        }
        if o.has('c') {
            out.push_str(&format!("{n:7} {line}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    };
    let mut prev: Option<(String, String)> = None;
    let mut count = 0;
    let mut s = String::new();
    loop {
        s.clear();
        match r.read_line(&mut s) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = s.trim_end_matches('\n').to_string();
        let k = key(&line);
        match &prev {
            Some((_, pk)) if *pk == k => count += 1,
            _ => {
                if let Some((pl, _)) = prev.take() {
                    emit(&pl, count);
                }
                prev = Some((line, k));
                count = 1;
            }
        }
    }
    if let Some((pl, _)) = prev {
        emit(&pl, count);
    }
    match files.get(1) {
        Some(f) => {
            if let Err(e) = rt::fs::write(f, out.as_bytes()) {
                rt::die!("{f}: {e}");
            }
        }
        None => print!("{out}"),
    }
    0
}
