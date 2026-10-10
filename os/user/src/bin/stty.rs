//! stty: show or change terminal settings.
//!
//!     stty [-a | -g]
//!     stty [-]echo [-]icanon [-]isig [-]icrnl [-]onlcr [-]ixon raw cooked sane
//!          rows N cols N size intr ^C erase ^? kill ^U eof ^D susp ^Z min N time N

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::term::*;

rt::main!(main);

const FLAGS: [(&str, u8, u32); 13] = [
    ("isig", 3, ISIG),
    ("icanon", 3, ICANON),
    ("echo", 3, ECHO),
    ("echoe", 3, ECHOE),
    ("echok", 3, ECHOK),
    ("echonl", 3, ECHONL),
    ("echoctl", 3, ECHOCTL),
    ("iexten", 3, IEXTEN),
    ("icrnl", 0, ICRNL),
    ("inlcr", 0, INLCR),
    ("igncr", 0, IGNCR),
    ("ixon", 0, IXON),
    ("onlcr", 1, ONLCR),
];

const CCS: [(&str, usize); 8] = [("intr", VINTR), ("quit", VQUIT), ("erase", VERASE), ("kill", VKILL), ("eof", VEOF), ("susp", VSUSP), ("werase", VWERASE), ("min", VMIN)];

fn field(t: &mut Termios, k: u8) -> &mut u32 {
    match k {
        0 => &mut t.iflag,
        1 => &mut t.oflag,
        2 => &mut t.cflag,
        _ => &mut t.lflag,
    }
}

fn cc_name(c: u8) -> String {
    match c {
        0 => "<undef>".into(),
        0x7F => "^?".into(),
        c if c < 0x20 => format!("^{}", (c + 0x40) as char),
        c => format!("{}", c as char),
    }
}

fn parse_cc(s: &str) -> Option<u8> {
    if s == "^?" {
        return Some(0x7F);
    }
    if s == "undef" || s == "^-" {
        return Some(0);
    }
    if let Some(c) = s.strip_prefix('^') {
        return c.bytes().next().map(|b| b.to_ascii_uppercase() & 0x1F);
    }
    if s.len() == 1 {
        return s.bytes().next();
    }
    s.parse().ok()
}

fn main(args: &[String]) -> i32 {
    let fd = 0;
    let mut t = match get_attr(fd) {
        Ok(t) => t,
        Err(e) => rt::die!("standard input: {e}"),
    };
    if args.len() == 1 || args[1] == "-a" {
        let (r, c) = size(fd);
        println!("rows {r}; columns {c};");
        let mut s = String::new();
        for (i, (n, k, b)) in FLAGS.iter().enumerate() {
            let on = *field(&mut t, *k) & b != 0;
            s.push_str(&format!("{}{n} ", if on { "" } else { "-" }));
            if i % 8 == 7 {
                s.push('\n');
            }
        }
        println!("{}", s.trim_end());
        let cc: Vec<String> = CCS.iter().map(|(n, i)| if *n == "min" { format!("min = {}", t.cc[*i]) } else { format!("{n} = {}", cc_name(t.cc[*i])) }).collect();
        println!("{}; time = {};", cc.join("; "), t.cc[VTIME]);
        return 0;
    }
    if args[1] == "-g" {
        println!("{:x}:{:x}:{:x}:{:x}:{}", t.iflag, t.oflag, t.cflag, t.lflag, t.cc.iter().map(|c| format!("{c:x}")).collect::<Vec<_>>().join(":"));
        return 0;
    }
    if args[1].contains(':') {
        let v: Vec<u32> = args[1].split(':').filter_map(|x| u32::from_str_radix(x, 16).ok()).collect();
        if v.len() >= 4 {
            t.iflag = v[0];
            t.oflag = v[1];
            t.cflag = v[2];
            t.lflag = v[3];
            for (i, c) in v[4..].iter().enumerate().take(NCCS) {
                t.cc[i] = *c as u8;
            }
            let _ = set_attr(fd, &t);
        }
        return 0;
    }
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        let (neg, name) = match a.strip_prefix('-') {
            Some(n) => (true, n),
            None => (false, a),
        };
        if let Some((_, k, b)) = FLAGS.iter().find(|(n, _, _)| *n == name) {
            let f = field(&mut t, *k);
            if neg { *f &= !b } else { *f |= b }
        } else if let Some((_, idx)) = CCS.iter().find(|(n, _)| *n == name) {
            i += 1;
            let Some(v) = args.get(i).and_then(|s| parse_cc(s)) else { rt::die!("missing or invalid argument to '{name}'") };
            t.cc[*idx] = v;
        } else {
            match a {
                "raw" => {
                    t.iflag &= !(ICRNL | INLCR | IGNCR | IXON | ISTRIP);
                    t.lflag &= !(ICANON | ECHO | ISIG | IEXTEN);
                    t.oflag &= !OPOST;
                    t.cc[VMIN] = 1;
                    t.cc[VTIME] = 0;
                }
                "-raw" | "cooked" | "sane" => {
                    t.iflag |= ICRNL | IXON;
                    t.oflag |= OPOST | ONLCR;
                    t.lflag |= ICANON | ECHO | ECHOE | ECHOK | ISIG | IEXTEN | ECHOCTL;
                    if a == "sane" {
                        t.cc[VINTR] = 3;
                        t.cc[VQUIT] = 0x1C;
                        t.cc[VERASE] = 0x7F;
                        t.cc[VKILL] = 0x15;
                        t.cc[VEOF] = 4;
                        t.cc[VSUSP] = 0x1A;
                        t.cc[VWERASE] = 0x17;
                    }
                }
                "time" => {
                    i += 1;
                    t.cc[VTIME] = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
                }
                "rows" | "cols" | "columns" => {
                    i += 1;
                    let n: u16 = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| rt::die!("invalid size"));
                    let (r, c) = size(fd);
                    let _ = if a == "rows" { set_size(fd, n, c) } else { set_size(fd, r, n) };
                }
                "size" => {
                    let (r, c) = size(fd);
                    println!("{r} {c}");
                }
                _ => rt::die!("invalid argument '{a}'"),
            }
        }
        i += 1;
    }
    if let Err(e) = set_attr(fd, &t) {
        rt::die!("{e}");
    }
    0
}
