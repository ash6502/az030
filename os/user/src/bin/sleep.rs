//! sleep: pause for a time.
//!
//!     sleep number[smhd]...    (fractions allowed: sleep 0.5)

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    if args.len() < 2 {
        rt::die!("missing operand");
    }
    let mut ms = 0.0f64;
    for a in &args[1..] {
        let (num, mult) = match a.chars().last() {
            Some('s') => (&a[..a.len() - 1], 1000.0),
            Some('m') => (&a[..a.len() - 1], 60_000.0),
            Some('h') => (&a[..a.len() - 1], 3_600_000.0),
            Some('d') => (&a[..a.len() - 1], 86_400_000.0),
            _ => (a.as_str(), 1000.0),
        };
        match num.parse::<f64>() {
            Ok(v) if v >= 0.0 => ms += v * mult,
            _ => rt::die!("invalid time interval '{a}'"),
        }
    }
    let mut left = ms as u64;
    while left > 0 {
        let chunk = left.min(u32::MAX as u64 / 2) as u32;
        if rt::time::sleep_ms(chunk).is_err() {
            return 1;
        }
        left -= chunk as u64;
    }
    0
}
