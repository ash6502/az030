//! Code-generation self-test. The same code runs natively on the build host and on
//! the az030; the two outputs must be identical (see `make cgtest`). Each test uses
//! `black_box` inputs and `#[inline(never)]` helpers so the m68k back end really has to
//! compile the operations instead of constant-folding them.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;
use core::hint::black_box as bb;

macro_rules! out {
    ($o:expr, $($t:tt)*) => {{ let _ = writeln!($o, $($t)*); }};
}

#[inline(never)]
fn widen(a: &[u8], b: &[i8], c: &[u16], d: &[i16]) -> (u32, i32, u32, i32, u64, i64) {
    let mut x = (0u32, 0i32, 0u32, 0i32, 0u64, 0i64);
    for i in 0..a.len() {
        x.0 = x.0.wrapping_mul(31).wrapping_add(a[i] as u32);
        x.1 = x.1.wrapping_mul(31).wrapping_add(b[i] as i32);
        x.2 = x.2.wrapping_mul(31).wrapping_add(c[i] as u32);
        x.3 = x.3.wrapping_mul(31).wrapping_add(d[i] as i32);
        x.4 = x.4.wrapping_mul(131).wrapping_add(c[i] as u64);
        x.5 = x.5.wrapping_mul(131).wrapping_add(d[i] as i64);
    }
    x
}

#[inline(never)]
fn store_widened(src: &[u8], dst: &mut [u32], s16: &[i16], d32: &mut [i32]) {
    for i in 0..src.len() {
        dst[i] = src[i] as u32;
        d32[i] = s16[i] as i32;
    }
}

#[inline(never)]
fn hex_digits(mut v: u64) -> [u8; 16] {
    let mut buf = [b'0'; 16];
    let mut i = 16;
    while v != 0 {
        i -= 1;
        buf[i] = b"0123456789abcdef"[(v & 15) as usize];
        v >>= 4;
    }
    buf
}

#[inline(never)]
fn stack_index(n: usize) -> u32 {
    let mut a = [0u32; 64];
    for i in 0..64 {
        a[(i * 7 + n) % 64] = (i * i) as u32 ^ n as u32;
    }
    let mut s = 0u32;
    for i in (0..64).rev() {
        s = s.rotate_left(3) ^ a[i];
    }
    s
}

#[inline(never)]
fn count_until_zero(bytes: &[u8]) -> (usize, usize) {
    // compare + branch with lots of live values in between
    let (mut n, mut odd) = (0, 0);
    let mut p = 0;
    while bytes[p] != 0 {
        if bytes[p] & 1 == 1 {
            odd += 1;
        }
        n += 1;
        p += 1;
    }
    (n, odd)
}

struct Big {
    a: i16,
    b: u64,
    c: i16,
    d: [u8; 5],
}

#[inline(never)]
fn make_big(x: u32) -> Big {
    Big { a: -(x as i16), b: (x as u64) << 33 | 7, c: x as i16 ^ 0x55, d: [x as u8, 1, 2, 3, (x >> 8) as u8] }
}

#[inline(never)]
fn pair(x: u32) -> (u8, u32) {
    if x < 100 { (2, 100) } else if x < 10_000 { (4, 10_000) } else { (9, 1_000_000_000) }
}

/// The shape of Grisu's integral digit loop.
#[inline(never)]
fn kappa_digits(x: u32, frac: u64, e: usize) -> ([u8; 12], usize) {
    let (max_kappa, mut ten_kappa) = pair2(x);
    let mut buf = [b'.'; 12];
    let mut remainder = x;
    let mut i = 0;
    loop {
        let q = remainder / ten_kappa;
        let r = remainder % ten_kappa;
        buf[i] = b'0' + q as u8;
        i += 1;
        let rem = ((r as u64) << e) + frac;
        if rem < (frac >> 3) || i > max_kappa as usize {
            break;
        }
        ten_kappa /= 10;
        remainder = r;
    }
    (buf, i)
}

#[inline(never)]
fn pair2(x: u32) -> (u8, u32) {
    let mut k = 0u8;
    let mut t = 1u32;
    while t <= x / 10 {
        t *= 10;
        k += 1;
    }
    (k, t)
}

#[inline(never)]
fn int64(a: u64, b: u64, c: i64, d: i64) -> [u64; 12] {
    [
        a.wrapping_mul(b),
        a / b,
        a % b,
        (c / d) as u64,
        (c % d) as u64,
        a >> (b & 63),
        a << (b & 31),
        (c >> 17) as u64,
        a.rotate_left(13),
        a.leading_zeros() as u64 | (a.trailing_zeros() as u64) << 8 | (a.count_ones() as u64) << 16,
        (a as i64).wrapping_neg() as u64,
        a.wrapping_add(b).wrapping_sub(c as u64),
    ]
}

#[inline(never)]
fn int128(a: u128, b: u128) -> [u128; 5] {
    [a.wrapping_mul(b), a / (b | 1), a % (b | 1), a >> 70, a.wrapping_add(b) ^ (b << 3)]
}

#[inline(never)]
fn small(a: i8, b: u8, c: i16, d: u16) -> [i32; 8] {
    [
        a as i32 * b as i32,
        (a / 3) as i32,
        (b % 7) as i32,
        (c >> 3) as i32,
        (d >> 5) as i32,
        a.wrapping_mul(a) as i32,
        (c as i32).wrapping_mul(d as i32),
        (a.min(b as i8) as i32) + (c.max(d as i16) as i32),
    ]
}

#[inline(never)]
fn floats(x: f64, y: f64, z: f32) -> [f64; 10] {
    [
        x + y,
        x * y,
        x / y,
        x - y * 3.5,
        (x as i64) as f64,
        (z as f64) * 2.0,
        (x * 1e6) as u32 as f64,
        -x,
        x.max(y),
        if x < y { 1.0 } else { -1.0 },
    ]
}

trait Shape {
    fn area(&self) -> u32;
    fn name(&self) -> String;
}
struct Rect(u32, u32);
struct Tri(u32, u32);
impl Shape for Rect {
    fn area(&self) -> u32 {
        self.0 * self.1
    }
    fn name(&self) -> String {
        format!("rect {}x{}", self.0, self.1)
    }
}
impl Shape for Tri {
    fn area(&self) -> u32 {
        self.0 * self.1 / 2
    }
    fn name(&self) -> String {
        format!("tri {}/{}", self.0, self.1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Tok {
    Num(i64),
    Word(String),
    Op(char),
}

fn lex(s: &str) -> Vec<Tok> {
    let mut v = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_ascii_digit() {
            let mut n = 0i64;
            while let Some(&d) = it.peek().filter(|d| d.is_ascii_digit()) {
                n = n * 10 + d.to_digit(10).unwrap() as i64;
                it.next();
            }
            v.push(Tok::Num(n));
        } else if c.is_alphabetic() {
            let mut w = String::new();
            while let Some(&d) = it.peek().filter(|d| d.is_alphanumeric()) {
                w.push(d);
                it.next();
            }
            v.push(Tok::Word(w));
        } else if c.is_whitespace() {
            it.next();
        } else {
            v.push(Tok::Op(c));
            it.next();
        }
    }
    v
}

/// Streams test output to the caller as it is produced.
pub struct Out<'a>(pub &'a mut dyn FnMut(&str));

impl Write for Out<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        (self.0)(s);
        Ok(())
    }
}

/// Run every test, writing the transcript to `o`.
pub fn run(o: &mut Out) {

    let a: Vec<u8> = (0..40u32).map(|i| (i * 37 + 200) as u8).collect();
    let b: Vec<i8> = a.iter().map(|&x| x as i8).collect();
    let c: Vec<u16> = (0..40u32).map(|i| (i * 4099 + 60000) as u16).collect();
    let d: Vec<i16> = c.iter().map(|&x| x as i16).collect();
    out!(o, "widen {:?}", widen(bb(&a), bb(&b), bb(&c), bb(&d)));
    let mut d1 = vec![0u32; 40];
    let mut d2 = vec![0i32; 40];
    store_widened(bb(&a), &mut d1, bb(&d), &mut d2);
    out!(o, "store {:?} {:?}", &d1[..8], &d2[..8]);
    out!(o, "hex {:?}", core::str::from_utf8(&hex_digits(bb(0x0123_4567_89AB_CDEF))).unwrap());
    out!(o, "stack {} {}", stack_index(bb(3)), stack_index(bb(61)));
    out!(o, "count {:?}", count_until_zero(bb(b"hello, world\0junk")));
    let g = make_big(bb(0x1234_5678));
    out!(o, "big {} {} {} {:?}", g.a, g.b, g.c, g.d);
    out!(o, "pair {:?} {:?} {:?}", pair(bb(5)), pair(bb(5000)), pair(bb(50000)));
    for (x, f, e) in [(31415u32, 0x6666_6666_651cu64, 48usize), (1234560, 0x10e, 43), (2500, 1 << 40, 50)] {
        let (b, n) = kappa_digits(bb(x), bb(f), bb(e));
        out!(o, "kappa {} {}", core::str::from_utf8(&b).unwrap(), n);
    }
    out!(o, "int64 {:?}", int64(bb(0xDEAD_BEEF_1234_5678), bb(0x0000_0001_F00D_0003), bb(-123_456_789_012), bb(-1777)));
    out!(o, "int64b {:?}", int64(bb(12345), bb(7), bb(99), bb(-4)));
    out!(o, "int128 {:?}", int128(bb(0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210), bb(0x0000_0000_0000_0003_8000_0000_0000_0001)));
    out!(o, "small {:?}", small(bb(-77), bb(201), bb(-30001), bb(65000)));
    let x = bb(3.14159f64);
    out!(o, "fbits {:x} {:x} {:x} {:x}", x.to_bits(), (x * 1e6).to_bits(), (x + 1.0).to_bits(), (x as f32).to_bits());
    out!(o, "fint {} {} {}", (x * 1e6) as u32, (x * 1000.0) as i64, (-x * 1e6) as i32);
    out!(o, "floats {:?}", floats(bb(3.14159), bb(-2.5e-3), bb(1.75)));
    for x in [3.14159f64, 0.1, 123.456, 2.5, 1e100, 6.02214076e23, -0.0, 1.0 / 3.0] {
        let x = bb(x);
        out!(o, "f {} {:.3} {:e} {:.0} {:10.4}|", x, x, x, x, x);
    }
    for x in [1.5f32, 0.1, 3.4028235e38, 1e-10] {
        out!(o, "f32 {} {:.2}", bb(x), bb(x));
    }
    out!(o, "parse {:?} {:?} {:?}", "3.25".parse::<f64>(), "-17".parse::<i32>(), "1e3".parse::<f32>());
    out!(o, "ints {} {} {} {:#x} {:#b} {:08} {:>6} {:+}", u64::MAX, i64::MIN, u128::MAX, 255, 5, -42, 7, 9);

    let shapes: Vec<Box<dyn Shape>> = vec![Box::new(Rect(3, 4)), Box::new(Tri(10, 7)), Box::new(Rect(100, 2))];
    for s in &shapes {
        out!(o, "shape {} {}", s.name(), s.area());
    }
    let total: u32 = shapes.iter().map(|s| s.area()).sum();
    out!(o, "total {}", total);

    let toks = lex(bb("let x1 = 42 + foo(7, 9) * 300;"));
    out!(o, "lex {:?}", toks);
    let mut sorted = toks.clone();
    sorted.sort();
    sorted.dedup();
    out!(o, "sorted {:?}", sorted);

    let text = "the quick brown fox jumps over the lazy dog the end of the story";
    let mut freq: BTreeMap<&str, usize> = BTreeMap::new();
    for w in text.split_whitespace() {
        *freq.entry(w).or_default() += 1;
    }
    out!(o, "freq {:?}", freq);
    let set: BTreeSet<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    out!(o, "set {}", set.iter().collect::<String>());
    let mut dq: VecDeque<i32> = (1..=10).collect();
    dq.rotate_left(3);
    dq.push_front(-1);
    let back = dq.pop_back();
    out!(o, "deque {:?} {:?}", dq, back);

    let mut v: Vec<i64> = (0..200).map(|i| ((i * 7919) % 211 - 100) as i64).collect();
    v.sort_unstable();
    let v2: Vec<i64> = v.iter().filter(|&&x| x % 3 == 0).map(|x| x * x).collect();
    out!(o, "vec {} {} {:?} {}", v.len(), v2.len(), &v2[..5], v2.iter().sum::<i64>());
    v.dedup();
    out!(o, "dedup {} {:?}", v.len(), v.binary_search(&17));

    let s = String::from("Hello, Wörld! ünïcödé ✓");
    out!(o, "str {} {} {} {:?}", s.len(), s.chars().count(), s.to_uppercase(), s.find('W'));
    out!(o, "words {:?}", s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>());
    let joined = (1..=12).map(|i| i.to_string()).collect::<Vec<_>>().join("-");
    out!(o, "join {} {}", joined, joined.replace("1", "one"));
    out!(o, "trim {:?} {:?}", "  padded\t".trim(), "a,b,,c".split(',').collect::<Vec<_>>());

    let mut fib = vec![0u64, 1];
    while fib.len() < 90 {
        let n = fib[fib.len() - 1] + fib[fib.len() - 2];
        fib.push(n);
    }
    out!(o, "fib {} {}", fib[89], fib.iter().fold(0u64, |a, &x| a ^ x.rotate_left((x % 64) as u32)));

    let closures: Vec<Box<dyn Fn(i32) -> i32>> = (1..5).map(|k| Box::new(move |x| x * k + k) as Box<dyn Fn(i32) -> i32>).collect();
    out!(o, "closures {:?}", closures.iter().map(|f| f(bb(10))).collect::<Vec<_>>());

    let opt: Option<&str> = bb(Some("x"));
    let res: Result<u8, String> = "300".parse::<u8>().map_err(|e| e.to_string());
    out!(o, "opt {:?} {:?}", opt.map(|s| s.len()), res);
}
