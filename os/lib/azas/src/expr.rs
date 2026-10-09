//! Expression values and the expression parser.

use alloc::format;
use alloc::string::String;

/// What an expression value is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// Plain number.
    Abs,
    /// Offset into a section.
    Sec(usize),
    /// External symbol (index into the assembler's extern list).
    Ext(usize),
    /// Not known yet (forward reference in an early pass).
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Val {
    pub base: Base,
    pub off: i64,
}

impl Val {
    pub const fn abs(v: i64) -> Val {
        Val { base: Base::Abs, off: v }
    }
    pub const UNKNOWN: Val = Val { base: Base::Unknown, off: 0 };

    pub fn is_abs(&self) -> bool {
        self.base == Base::Abs
    }
    pub fn is_unknown(&self) -> bool {
        self.base == Base::Unknown
    }
}

/// Symbol lookups the parser needs from the assembler.
pub trait Ctx {
    fn symbol(&mut self, name: &str) -> Result<Val, String>;
    /// Value of `*`.
    fn pc(&self) -> Val;
}

pub fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'.'
}

pub fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$'
}

pub struct Parser<'a, 'c> {
    pub s: &'a [u8],
    pub pos: usize,
    ctx: &'c mut dyn Ctx,
}

type R = Result<Val, String>;

impl<'a, 'c> Parser<'a, 'c> {
    pub fn new(s: &'a str, ctx: &'c mut dyn Ctx) -> Self {
        Parser { s: s.as_bytes(), pos: 0, ctx }
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.s.len()
    }

    pub fn peek(&self) -> u8 {
        self.s.get(self.pos).copied().unwrap_or(0)
    }

    fn peek2(&self) -> u8 {
        self.s.get(self.pos + 1).copied().unwrap_or(0)
    }

    pub fn skip_ws(&mut self) {
        while matches!(self.peek(), b' ' | b'\t') {
            self.pos += 1;
        }
    }

    pub fn rest(&self) -> &'a str {
        core::str::from_utf8(&self.s[self.pos.min(self.s.len())..]).unwrap_or("")
    }

    /// Parse a full expression.
    pub fn expr(&mut self) -> R {
        self.binary(0)
    }

    fn binary(&mut self, min_prec: u8) -> R {
        let mut lhs = self.unary()?;
        loop {
            self.skip_ws();
            let (op, len, prec) = match (self.peek(), self.peek2()) {
                (b'|', b'|') => ("||", 2, 1),
                (b'&', b'&') => ("&&", 2, 2),
                (b'|', _) => ("|", 1, 3),
                (b'!', b'!') => ("^", 2, 4),
                (b'^', _) => ("^", 1, 4),
                (b'&', _) => ("&", 1, 5),
                (b'=', b'=') => ("==", 2, 6),
                (b'!', b'=') => ("!=", 2, 6),
                (b'<', b'>') => ("!=", 2, 6),
                (b'=', _) => ("==", 1, 6),
                (b'<', b'=') => ("<=", 2, 7),
                (b'>', b'=') => (">=", 2, 7),
                (b'<', b'<') => ("<<", 2, 8),
                (b'>', b'>') => (">>", 2, 8),
                (b'<', _) => ("<", 1, 7),
                (b'>', _) => (">", 1, 7),
                (b'+', _) => ("+", 1, 9),
                (b'-', _) => ("-", 1, 9),
                (b'*', _) => ("*", 1, 10),
                (b'/', b'/') => ("%", 2, 10),
                (b'/', _) => ("/", 1, 10),
                _ => return Ok(lhs),
            };
            if prec < min_prec {
                return Ok(lhs);
            }
            self.pos += len;
            let rhs = self.binary(prec + 1)?;
            lhs = apply(op, lhs, rhs)?;
        }
    }

    fn unary(&mut self) -> R {
        self.skip_ws();
        match self.peek() {
            b'-' => {
                self.pos += 1;
                let v = self.unary()?;
                apply("-", Val::abs(0), v)
            }
            b'+' => {
                self.pos += 1;
                self.unary()
            }
            b'~' => {
                self.pos += 1;
                let v = self.unary()?;
                num1(v, |a| !a)
            }
            b'!' => {
                self.pos += 1;
                let v = self.unary()?;
                num1(v, |a| (a == 0) as i64)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> R {
        self.skip_ws();
        let c = self.peek();
        match c {
            b'(' => {
                self.pos += 1;
                let v = self.expr()?;
                self.skip_ws();
                if self.peek() != b')' {
                    return Err("missing `)`".into());
                }
                self.pos += 1;
                Ok(v)
            }
            b'*' => {
                self.pos += 1;
                Ok(self.ctx.pc())
            }
            b'$' => {
                self.pos += 1;
                self.radix(16)
            }
            b'%' => {
                self.pos += 1;
                self.radix(2)
            }
            b'@' => {
                self.pos += 1;
                self.radix(8)
            }
            b'\'' | b'"' => {
                self.pos += 1;
                let mut v: i64 = 0;
                let mut n = 0;
                loop {
                    let ch = self.peek();
                    if ch == 0 {
                        return Err("unterminated character constant".into());
                    }
                    self.pos += 1;
                    if ch == c {
                        if self.peek() == c {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let ch = if ch == b'\\' && c == b'\'' && self.peek() != c && self.peek() != 0 {
                        let e = self.peek();
                        self.pos += 1;
                        escape(e)
                    } else {
                        ch
                    };
                    v = (v << 8) | ch as i64;
                    n += 1;
                }
                if n == 0 || n > 4 {
                    return Err("bad character constant".into());
                }
                Ok(Val::abs(v))
            }
            b'0' if matches!(self.peek2(), b'x' | b'X') => {
                self.pos += 2;
                self.radix(16)
            }
            b'0'..=b'9' => self.radix(10),
            c if is_ident_start(c) || c == b'\\' => {
                let start = self.pos;
                while is_ident_char(self.peek()) {
                    self.pos += 1;
                }
                let name = core::str::from_utf8(&self.s[start..self.pos]).unwrap();
                if name.is_empty() {
                    return Err(format!("unexpected `{}`", self.rest()));
                }
                self.ctx.symbol(name)
            }
            0 => Err("missing expression".into()),
            _ => Err(format!("unexpected `{}`", self.rest())),
        }
    }

    fn radix(&mut self, r: u32) -> R {
        let start = self.pos;
        let mut v: i64 = 0;
        while let Some(d) = (self.peek() as char).to_digit(r) {
            v = v.wrapping_mul(r as i64).wrapping_add(d as i64);
            self.pos += 1;
        }
        if self.pos == start {
            return Err("bad number".into());
        }
        if is_ident_char(self.peek()) && !(self.peek() == b'.' && !self.peek2().is_ascii_digit()) {
            // things like `1f` / `12abc` are not numbers
            return Err(format!("bad number `{}`", self.rest()));
        }
        Ok(Val::abs(v))
    }
}

pub fn escape(e: u8) -> u8 {
    match e {
        b'n' => 10,
        b'r' => 13,
        b't' => 9,
        b'0' => 0,
        b'e' => 27,
        b'a' => 7,
        b'b' => 8,
        b'f' => 12,
        b'v' => 11,
        o => o,
    }
}

fn num1(v: Val, f: impl Fn(i64) -> i64) -> R {
    match v.base {
        Base::Abs => Ok(Val::abs(f(v.off))),
        Base::Unknown => Ok(Val::UNKNOWN),
        _ => Err("relocatable value not allowed here".into()),
    }
}

pub fn apply(op: &str, a: Val, b: Val) -> R {
    use Base::*;
    if a.is_unknown() || b.is_unknown() {
        return Ok(Val::UNKNOWN);
    }
    match op {
        "+" => match (a.base, b.base) {
            (Abs, x) | (x, Abs) => Ok(Val { base: x, off: a.off.wrapping_add(b.off) }),
            _ => Err("cannot add two relocatable values".into()),
        },
        "-" => match (a.base, b.base) {
            (x, Abs) => Ok(Val { base: x, off: a.off.wrapping_sub(b.off) }),
            (x, y) if x == y => Ok(Val::abs(a.off - b.off)),
            _ => Err("cannot subtract values from different sections".into()),
        },
        _ => {
            if !a.is_abs() || !b.is_abs() {
                // comparisons between labels of the same section are fine
                if a.base == b.base && matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=") {
                    return apply(op, Val::abs(a.off), Val::abs(b.off));
                }
                return Err(format!("operator `{op}` needs absolute values"));
            }
            let (x, y) = (a.off, b.off);
            let v = match op {
                "*" => x.wrapping_mul(y),
                "/" | "%" => {
                    if y == 0 {
                        return Err("division by zero".into());
                    }
                    if op == "/" { x / y } else { x % y }
                }
                "<<" => x.wrapping_shl(y as u32),
                ">>" => ((x as u64 & 0xFFFF_FFFF) >> (y as u32 & 63)) as i64,
                "&" => x & y,
                "|" => x | y,
                "^" => x ^ y,
                "&&" => (x != 0 && y != 0) as i64,
                "||" => (x != 0 || y != 0) as i64,
                // vasm: true is -1
                "==" => -((x == y) as i64),
                "!=" => -((x != y) as i64),
                "<" => -((x < y) as i64),
                ">" => -((x > y) as i64),
                "<=" => -((x <= y) as i64),
                ">=" => -((x >= y) as i64),
                _ => unreachable!(),
            };
            Ok(Val::abs(v))
        }
    }
}
