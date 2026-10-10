//! cal: display a calendar.
//!
//!     cal [[month] year]

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::time::{self, Tm};

rt::main!(main);

const NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn month_lines(y: i32, m: u32, today: Option<u32>) -> Vec<String> {
    let mut v = Vec::new();
    let title = format!("{} {}", NAMES[m as usize - 1], y);
    let pad = (20 - title.len()) / 2;
    v.push(format!("{:pad$}{title:<w$}", "", w = 20 - pad));
    v.push("Su Mo Tu We Th Fr Sa".into());
    let first = time::gmtime(time::mktime(&Tm { year: y, month: m, day: 1, ..Default::default() })).wday as usize;
    let days = time::month_days(y, m) as usize;
    let mut line = String::new();
    for _ in 0..first {
        line.push_str("   ");
    }
    for d in 1..=days {
        if today == Some(d as u32) {
            line.push_str(&format!("\x1b[7m{d:2}\x1b[0m "));
        } else {
            line.push_str(&format!("{d:2} "));
        }
        if (first + d) % 7 == 0 {
            v.push(line.trim_end().into());
            line.clear();
        }
    }
    if !line.is_empty() {
        v.push(line.trim_end().into());
    }
    while v.len() < 8 {
        v.push(String::new());
    }
    v
}

fn main(args: &[String]) -> i32 {
    let now = time::gmtime(time::now());
    let tty = rt::io::isatty(1);
    match args.len() {
        1 => {
            for l in month_lines(now.year, now.month, if tty { Some(now.day) } else { None }) {
                println!("{l}");
            }
        }
        2 => {
            let y: i32 = args[1].parse().unwrap_or_else(|_| rt::die!("invalid year"));
            println!("{:>32}\n", y);
            for row in 0..4 {
                let ms: Vec<Vec<String>> = (1..=3).map(|c| month_lines(y, row * 3 + c, None)).collect();
                for i in 0..8 {
                    let l: Vec<String> = ms.iter().map(|m| format!("{:<20}", m[i])).collect();
                    println!("{}", l.join("  ").trim_end());
                }
            }
        }
        _ => {
            let m: u32 = args[1].parse().ok().filter(|m| (1..=12).contains(m)).unwrap_or_else(|| rt::die!("invalid month"));
            let y: i32 = args[2].parse().unwrap_or_else(|_| rt::die!("invalid year"));
            for l in month_lines(y, m, None) {
                println!("{l}");
            }
        }
    }
    0
}
