//! cat: concatenate files.
//!
//!     cat [-nbsvE] [file...]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "nbsvEAu", "[-nbsvE] [file...]");
    let (number, nonblank, squeeze) = (o.has('n'), o.has('b'), o.has('s'));
    let (visible, ends) = (o.has('v') || o.has('A'), o.has('E') || o.has('A'));
    let plain = !number && !nonblank && !squeeze && !visible && !ends;
    let mut out = rt::io::stdout();
    let mut st = 0;
    let mut lineno = 0;
    let mut blank_run = 0;
    for f in rt::io::inputs(&files) {
        let mut inp = match rt::io::open_input(&f) {
            Ok(i) => i,
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
                continue;
            }
        };
        if plain {
            let mut buf = vec![0u8; 8192];
            loop {
                match inp.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out.write_all(&buf[..n]).is_err() {
                            return 1;
                        }
                    }
                    Err(e) => {
                        rt::warn!("{f}: {e}");
                        st = 1;
                        break;
                    }
                }
            }
            continue;
        }
        let mut r = rt::io::BufReader::new(inp);
        let mut line = Vec::new();
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    rt::warn!("{f}: {e}");
                    st = 1;
                    break;
                }
            }
            let has_nl = line.last() == Some(&b'\n');
            let body = if has_nl { &line[..line.len() - 1] } else { &line[..] };
            if body.is_empty() {
                blank_run += 1;
                if squeeze && blank_run > 1 {
                    continue;
                }
            } else {
                blank_run = 0;
            }
            let mut o = Vec::new();
            if (number && !nonblank) || (nonblank && !body.is_empty()) {
                lineno += 1;
                o.extend_from_slice(format!("{lineno:6}\t").as_bytes());
            }
            for &b in body {
                if visible && b != b'\t' {
                    if b >= 0x80 {
                        o.extend_from_slice(b"M-");
                    }
                    let c = b & 0x7F;
                    if c < 0x20 {
                        o.push(b'^');
                        o.push(c + 0x40);
                    } else if c == 0x7F {
                        o.extend_from_slice(b"^?");
                    } else {
                        o.push(c);
                    }
                } else {
                    o.push(b);
                }
            }
            if ends && has_nl {
                o.push(b'$');
            }
            if has_nl {
                o.push(b'\n');
            }
            if out.write_all(&o).is_err() {
                return 1;
            }
        }
    }
    st
}
