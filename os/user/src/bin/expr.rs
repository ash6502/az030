//! expr: evaluate expressions.
//!
//!     expr a | b, a & b, a < b (<= = != >= >), a + b (- * / %), str : regex,
//!     match str regex, substr str pos len, index str chars, length str, ( ... )

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

#[derive(Clone)]
enum V {
    I(i64),
    S(String),
}

impl V {
    fn s(&self) -> String {
        match self {
            V::I(i) => format!("{i}"),
            V::S(s) => s.clone(),
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            V::I(i) => Some(*i),
            V::S(s) => s.parse().ok(),
        }
    }
    fn truthy(&self) -> bool {
        match self {
            V::I(i) => *i != 0,
            V::S(s) => !s.is_empty() && s != "0",
        }
    }
}

struct P<'a> {
    a: &'a [String],
    i: usize,
}

fn fail(m: &str) -> ! {
    rt::eprintln!("expr: {m}");
    rt::process::exit(2)
}

impl P<'_> {
    fn peek(&self) -> Option<&str> {
        self.a.get(self.i).map(|s| s.as_str())
    }
    fn next(&mut self) -> String {
        let s = self.a.get(self.i).cloned().unwrap_or_else(|| fail("syntax error"));
        self.i += 1;
        s
    }
    fn or(&mut self) -> V {
        let mut l = self.and();
        while self.peek() == Some("|") {
            self.i += 1;
            let r = self.and();
            l = if l.truthy() { l } else if r.truthy() { r } else { V::I(0) };
        }
        l
    }
    fn and(&mut self) -> V {
        let mut l = self.cmp();
        while self.peek() == Some("&") {
            self.i += 1;
            let r = self.cmp();
            l = if l.truthy() && r.truthy() { l } else { V::I(0) };
        }
        l
    }
    fn cmp(&mut self) -> V {
        let mut l = self.add();
        while let Some(op @ ("<" | "<=" | "=" | "==" | "!=" | ">=" | ">")) = self.peek() {
            let op = op.to_string();
            self.i += 1;
            let r = self.add();
            let ord = match (l.int(), r.int()) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => l.s().cmp(&r.s()),
            };
            use core::cmp::Ordering::*;
            let v = match op.as_str() {
                "<" => ord == Less,
                "<=" => ord != Greater,
                "=" | "==" => ord == Equal,
                "!=" => ord != Equal,
                ">=" => ord != Less,
                _ => ord == Greater,
            };
            l = V::I(v as i64);
        }
        l
    }
    fn add(&mut self) -> V {
        let mut l = self.mul();
        while let Some(op @ ("+" | "-")) = self.peek() {
            let op = op.to_string();
            self.i += 1;
            let r = self.mul();
            let (a, b) = (l.int().unwrap_or_else(|| fail("non-integer argument")), r.int().unwrap_or_else(|| fail("non-integer argument")));
            l = V::I(if op == "+" { a.wrapping_add(b) } else { a.wrapping_sub(b) });
        }
        l
    }
    fn mul(&mut self) -> V {
        let mut l = self.colon();
        while let Some(op @ ("*" | "/" | "%")) = self.peek() {
            let op = op.to_string();
            self.i += 1;
            let r = self.colon();
            let (a, b) = (l.int().unwrap_or_else(|| fail("non-integer argument")), r.int().unwrap_or_else(|| fail("non-integer argument")));
            if op != "*" && b == 0 {
                fail("division by zero");
            }
            l = V::I(match op.as_str() {
                "*" => a.wrapping_mul(b),
                "/" => a / b,
                _ => a % b,
            });
        }
        l
    }
    fn colon(&mut self) -> V {
        let mut l = self.primary();
        while self.peek() == Some(":") {
            self.i += 1;
            let r = self.primary();
            l = re_match(&l.s(), &r.s());
        }
        l
    }
    fn primary(&mut self) -> V {
        let t = self.next();
        match t.as_str() {
            "(" => {
                let v = self.or();
                if self.next() != ")" {
                    fail("syntax error: expecting ')'");
                }
                v
            }
            "match" => {
                let s = self.primary().s();
                let r = self.primary().s();
                re_match(&s, &r)
            }
            "substr" => {
                let s = self.primary().s();
                let p = self.primary().int().unwrap_or(0);
                let n = self.primary().int().unwrap_or(0);
                if p < 1 || n < 1 {
                    return V::S(String::new());
                }
                V::S(s.chars().skip(p as usize - 1).take(n as usize).collect())
            }
            "index" => {
                let s = self.primary().s();
                let cs = self.primary().s();
                V::I(s.chars().position(|c| cs.contains(c)).map_or(0, |i| i as i64 + 1))
            }
            "length" => V::I(self.primary().s().chars().count() as i64),
            "+" => V::S(self.next()),
            _ => match t.parse::<i64>() {
                Ok(i) => V::I(i),
                Err(_) => V::S(t),
            },
        }
    }
}

fn re_match(s: &str, re: &str) -> V {
    let r = match rt::regex::Regex::new(&format!("^{re}"), false, false) {
        Ok(r) => r,
        Err(e) => fail(&e),
    };
    let chars: Vec<char> = s.chars().collect();
    match r.find_chars(&chars, 0) {
        Some(c) => {
            if r.groups() > 0 {
                V::S(c[1].map(|(a, b)| chars[a..b].iter().collect()).unwrap_or_default())
            } else {
                let (a, b) = c[0].unwrap();
                V::I((b - a) as i64)
            }
        }
        None => {
            if r.groups() > 0 {
                V::S(String::new())
            } else {
                V::I(0)
            }
        }
    }
}

fn main(args: &[String]) -> i32 {
    if args.len() < 2 {
        fail("missing operand");
    }
    let mut p = P { a: &args[1..], i: 0 };
    let v = p.or();
    if p.i < args.len() - 1 {
        fail(&format!("syntax error: unexpected argument '{}'", args[p.i + 1]));
    }
    println!("{}", v.s());
    if v.truthy() { 0 } else { 1 }
}
