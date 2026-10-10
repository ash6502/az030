//! Shell arithmetic: `$(( expr ))` with C operators on 64-bit integers,
//! variables, and assignment (`=`, `+=`, ...).

use crate::Shell;
use rt::prelude::*;

#[derive(Clone, Debug, PartialEq)]
enum T {
    Num(i64),
    Name(String),
    Op(&'static str),
}

const OPS: [&str; 34] = [
    "<<=", ">>=", "**", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "+", "-", "*", "/",
    "%", "<", ">", "&", "|", "^", "!", "~", "(", ")", "=",
];

fn lex(s: &str) -> Result<Vec<T>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    'outer: while i < c.len() {
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        if c[i].is_ascii_digit() {
            let st = i;
            while i < c.len() && c[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let t: String = c[st..i].iter().collect();
            let v = if let Some(h) = t.strip_prefix("0x").or(t.strip_prefix("0X")) {
                i64::from_str_radix(h, 16)
            } else if t.len() > 1 && t.starts_with('0') {
                i64::from_str_radix(&t[1..], 8)
            } else {
                t.parse()
            };
            out.push(T::Num(v.map_err(|_| format!("{t}: invalid number"))?));
            continue;
        }
        if c[i] == '_' || c[i].is_ascii_alphabetic() {
            let st = i;
            while i < c.len() && (c[i] == '_' || c[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            out.push(T::Name(c[st..i].iter().collect()));
            continue;
        }
        if c[i] == '?' || c[i] == ':' {
            out.push(T::Op(if c[i] == '?' { "?" } else { ":" }));
            i += 1;
            continue;
        }
        for op in OPS {
            if op.chars().enumerate().all(|(k, oc)| c.get(i + k) == Some(&oc)) {
                out.push(T::Op(op));
                i += op.len();
                continue 'outer;
            }
        }
        return Err(format!("syntax error in expression (at '{}')", c[i]));
    }
    Ok(out)
}

struct P<'a> {
    t: Vec<T>,
    i: usize,
    sh: &'a mut Shell,
    /// evaluate (false inside the untaken side of && || ?:)
    live: bool,
}

fn prec(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | ">" | "<=" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        "**" => 11,
        _ => return None,
    })
}

impl P<'_> {
    fn peek(&self) -> Option<&T> {
        self.t.get(self.i)
    }

    fn var(&mut self, n: &str) -> Result<i64, String> {
        let v = self.sh.get_var(n).unwrap_or_default();
        let v = v.trim();
        if v.is_empty() {
            return Ok(0);
        }
        if let Ok(x) = v.parse() {
            return Ok(x);
        }
        // a variable may hold an expression
        eval(self.sh, v)
    }

    fn assign(&mut self) -> Result<i64, String> {
        if let (Some(T::Name(n)), Some(T::Op(op))) = (self.t.get(self.i).cloned(), self.t.get(self.i + 1).cloned()) {
            if op.ends_with('=') && !matches!(op, "==" | "!=" | "<=" | ">=") {
                self.i += 2;
                let rhs = self.assign()?;
                let v = if op == "=" {
                    rhs
                } else {
                    let cur = self.var(&n)?;
                    binop(&op[..op.len() - 1], cur, rhs)?
                };
                if self.live {
                    self.sh.set_var(&n, &format!("{v}"))?;
                }
                return Ok(v);
            }
        }
        self.ternary()
    }

    fn ternary(&mut self) -> Result<i64, String> {
        let c = self.binary(1)?;
        if self.peek() == Some(&T::Op("?")) {
            self.i += 1;
            let live = self.live;
            self.live = live && c != 0;
            let a = self.assign()?;
            if self.peek() != Some(&T::Op(":")) {
                return Err("expected ':' in expression".into());
            }
            self.i += 1;
            self.live = live && c == 0;
            let b = self.assign()?;
            self.live = live;
            return Ok(if c != 0 { a } else { b });
        }
        Ok(c)
    }

    fn binary(&mut self, min: u8) -> Result<i64, String> {
        let mut l = self.unary()?;
        loop {
            let Some(T::Op(op)) = self.peek().cloned() else { break };
            let Some(p) = prec(op) else { break };
            if p < min {
                break;
            }
            self.i += 1;
            let live = self.live;
            if op == "&&" {
                self.live = live && l != 0;
            } else if op == "||" {
                self.live = live && l == 0;
            }
            let r = self.binary(if op == "**" { p } else { p + 1 })?;
            self.live = live;
            l = if self.live || op == "&&" || op == "||" { binop(op, l, r)? } else { 0 };
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<i64, String> {
        match self.peek().cloned() {
            Some(T::Op(op @ ("-" | "+" | "!" | "~"))) => {
                self.i += 1;
                let v = self.unary()?;
                Ok(match op {
                    "-" => v.wrapping_neg(),
                    "+" => v,
                    "!" => (v == 0) as i64,
                    _ => !v,
                })
            }
            Some(T::Op("(")) => {
                self.i += 1;
                let v = self.assign()?;
                if self.peek() != Some(&T::Op(")")) {
                    return Err("missing ')' in expression".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(T::Num(n)) => {
                self.i += 1;
                Ok(n)
            }
            Some(T::Name(n)) => {
                self.i += 1;
                self.var(&n)
            }
            Some(T::Op(o)) => Err(format!("syntax error in expression (unexpected '{o}')")),
            None => Err("syntax error in expression (missing operand)".into()),
        }
    }
}

fn binop(op: &str, l: i64, r: i64) -> Result<i64, String> {
    Ok(match op {
        "+" => l.wrapping_add(r),
        "-" => l.wrapping_sub(r),
        "*" => l.wrapping_mul(r),
        "/" | "%" if r == 0 => return Err("division by zero".into()),
        "/" => l.wrapping_div(r),
        "%" => l.wrapping_rem(r),
        "**" => {
            if r < 0 {
                return Err("negative exponent".into());
            }
            l.wrapping_pow(r as u32)
        }
        "<<" => l.wrapping_shl(r as u32),
        ">>" => l.wrapping_shr(r as u32),
        "<" => (l < r) as i64,
        ">" => (l > r) as i64,
        "<=" => (l <= r) as i64,
        ">=" => (l >= r) as i64,
        "==" => (l == r) as i64,
        "!=" => (l != r) as i64,
        "&" => l & r,
        "|" => l | r,
        "^" => l ^ r,
        "&&" => (l != 0 && r != 0) as i64,
        "||" => (l != 0 || r != 0) as i64,
        _ => return Err(format!("unknown operator '{op}'")),
    })
}

pub fn eval(sh: &mut Shell, s: &str) -> Result<i64, String> {
    let t = lex(s)?;
    if t.is_empty() {
        return Ok(0);
    }
    let mut p = P { t, i: 0, sh, live: true };
    let v = p.assign()?;
    if p.i < p.t.len() {
        return Err(format!("syntax error in expression (at {:?})", p.t[p.i]));
    }
    Ok(v)
}
