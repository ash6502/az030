//! seq: print a sequence of numbers.
//!
//!     seq [-w] [-s sep] [-f format] [first [incr]] last

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn decimals(s: &str) -> usize {
    s.split_once('.').map_or(0, |(_, d)| d.len())
}

fn main(args: &[String]) -> i32 {
    // negative numbers are operands, not options
    let mut a: Vec<String> = Vec::new();
    let mut opts: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let x = &args[i];
        if x.starts_with('-') && x.len() > 1 && !x[1..].starts_with(|c: char| c.is_ascii_digit() || c == '.') {
            opts.push(x.clone());
            if (x == "-s" || x == "-f") && i + 1 < args.len() {
                i += 1;
                opts.push(args[i].clone());
            }
        } else {
            a.push(x.clone());
        }
        i += 1;
    }
    let (o, _) = rt::getopt::parse(&opts, "ws:f:", "[-w] [-s sep] [first [incr]] last");
    let nums: Vec<f64> = a.iter().map(|s| s.parse().unwrap_or_else(|_| rt::die!("invalid floating point argument: '{s}'"))).collect();
    let (first, incr, last) = match nums.len() {
        1 => (1.0, 1.0, nums[0]),
        2 => (nums[0], 1.0, nums[1]),
        3 => (nums[0], nums[1], nums[2]),
        _ => rt::die!("usage: seq [-w] [-s sep] [first [incr]] last"),
    };
    if incr == 0.0 {
        rt::die!("increment must not be 0");
    }
    let prec = a.iter().map(|s| decimals(s)).max().unwrap_or(0);
    let sep = o.get('s').unwrap_or("\n").to_string();
    let fmt = |v: f64| format!("{:.*}", prec, v);
    let width = if o.has('w') { fmt(first).len().max(fmt(last).len()) } else { 0 };
    let mut out = String::new();
    let mut k = 0u64;
    loop {
        let v = first + incr * k as f64;
        if (incr > 0.0 && v > last + 1e-9) || (incr < 0.0 && v < last - 1e-9) {
            break;
        }
        if k > 0 {
            out.push_str(&sep);
        }
        let s = fmt(v);
        if let Some(f) = o.get('f') {
            // a single %g / %f / %e conversion
            let shown = if f.contains("%g") || f.contains("%f") || f.contains("%e") {
                f.replacen("%g", &s, 1).replacen("%f", &format!("{v:.6}"), 1).replacen("%e", &format!("{v:e}"), 1)
            } else {
                f.into()
            };
            out.push_str(&shown);
        } else if width > s.len() {
            let neg = s.starts_with('-');
            let digits = s.trim_start_matches('-');
            if neg {
                out.push('-');
            }
            for _ in s.len()..width {
                out.push('0');
            }
            out.push_str(digits);
        } else {
            out.push_str(&s);
        }
        k += 1;
        if out.len() > 8192 {
            print!("{out}");
            out.clear();
        }
    }
    if k > 0 {
        out.push('\n');
    }
    print!("{out}");
    0
}
