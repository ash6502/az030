//! Operand parsing: every 68030 addressing mode plus register lists, FPU/MMU registers,
//! register pairs and bit-field specifiers.

use crate::expr::{Ctx, Parser, Val};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sz {
    B,
    W,
    L,
    S,
    D,
    X,
    P,
}

impl Sz {
    pub fn from_suffix(s: &str) -> Option<Sz> {
        Some(match s.to_ascii_lowercase().as_str() {
            "b" => Sz::B,
            "w" => Sz::W,
            "l" => Sz::L,
            "s" => Sz::S,
            "d" => Sz::D,
            "x" => Sz::X,
            "p" => Sz::P,
            _ => return None,
        })
    }

    /// Bytes an immediate of this size occupies.
    pub fn imm_bytes(self) -> usize {
        match self {
            Sz::B | Sz::W => 2,
            Sz::L | Sz::S => 4,
            Sz::D => 8,
            Sz::X | Sz::P => 12,
        }
    }
}

/// Index register: `reg` 0-7 = d0-d7, 8-15 = a0-a7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Index {
    pub reg: u8,
    pub long: bool,
    pub scale: u8, // log2: 0..3
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    Sr,
    Ccr,
    Usp,
    Pc,
    // control registers (movec)
    Sfc,
    Dfc,
    Cacr,
    Vbr,
    Caar,
    Msp,
    Isp,
    // MMU
    Tc,
    Srp,
    Crp,
    Tt0,
    Tt1,
    Mmusr,
}

impl Special {
    pub fn movec_code(self) -> Option<u16> {
        Some(match self {
            Special::Sfc => 0x000,
            Special::Dfc => 0x001,
            Special::Cacr => 0x002,
            Special::Usp => 0x800,
            Special::Vbr => 0x801,
            Special::Caar => 0x802,
            Special::Msp => 0x803,
            Special::Isp => 0x804,
            _ => return None,
        })
    }
}

/// Displacement with an optional explicit size (`.w` / `.l`).
#[derive(Clone, Copy, Debug)]
pub struct Disp {
    pub v: Val,
    pub sz: Option<Sz>,
}

/// Base register of an indexed / memory-indirect mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseReg {
    An(u8),
    Pc,
    /// Suppressed base register (`za0`, `zpc`, or omitted).
    None,
    /// Suppressed PC (`zpc`): still encoded with the PC mode.
    Zpc,
}

#[derive(Clone, Copy, Debug)]
pub enum BfArg {
    Imm(u32),
    D(u8),
}

#[derive(Clone, Debug)]
pub enum Op {
    D(u8),
    A(u8),
    Ind(u8),
    Post(u8),
    Pre(u8),
    /// (d16,An) or (d16,PC) — for PC the value is the *target*, not the displacement.
    Disp { base: BaseReg, d: Disp },
    /// Indexed / memory indirect. For PC bases bd is the target address.
    Idx {
        base: BaseReg,
        bd: Option<Disp>,
        ix: Option<Index>,
        /// Some(..) = memory indirect: `post` is true when the index is outside the brackets.
        mem: Option<(bool, Option<Disp>)>,
    },
    Abs { v: Val, sz: Option<Sz> },
    Imm(Val),
    ImmF(f64),
    /// Register list, bit n = d0..d7, a0..a7.
    List(u16),
    Fp(u8),
    /// FP data register list, bit n = fpn.
    FpList(u8),
    /// FP control registers: bit2 FPCR, bit1 FPSR, bit0 FPIAR.
    FpCtl(u8),
    Spec(Special),
    /// `Rh:Rl` (0-15).
    Pair(u8, u8),
    /// `(Rn):(Rm)` for cas2.
    IndPair(u8, u8),
    /// `fpc:fps` for fsincos.
    FpPair(u8, u8),
    BitField { ea: Box<Op>, off: BfArg, width: BfArg },
}

/// Parse a general register name: d0-d7 -> 0..7, a0-a7/sp -> 8..15.
pub fn gpr(s: &str) -> Option<u8> {
    let l = s.to_ascii_lowercase();
    if l == "sp" {
        return Some(15);
    }
    let b = l.as_bytes();
    if b.len() == 2 && b[1].is_ascii_digit() && b[1] <= b'7' {
        match b[0] {
            b'd' => return Some(b[1] - b'0'),
            b'a' => return Some(8 + b[1] - b'0'),
            _ => {}
        }
    }
    None
}

fn fpr(s: &str) -> Option<u8> {
    let l = s.to_ascii_lowercase();
    let b = l.as_bytes();
    if b.len() == 3 && &b[..2] == b"fp" && (b'0'..=b'7').contains(&b[2]) {
        return Some(b[2] - b'0');
    }
    None
}

fn fpctl(s: &str) -> Option<u8> {
    match s.to_ascii_lowercase().as_str() {
        "fpcr" => Some(4),
        "fpsr" => Some(2),
        "fpiar" | "fpi" => Some(1),
        _ => None,
    }
}

fn special(s: &str) -> Option<Special> {
    Some(match s.to_ascii_lowercase().as_str() {
        "sr" => Special::Sr,
        "ccr" => Special::Ccr,
        "usp" => Special::Usp,
        "pc" => Special::Pc,
        "sfc" => Special::Sfc,
        "dfc" => Special::Dfc,
        "cacr" => Special::Cacr,
        "vbr" => Special::Vbr,
        "caar" => Special::Caar,
        "msp" => Special::Msp,
        "isp" => Special::Isp,
        "tc" => Special::Tc,
        "srp" => Special::Srp,
        "crp" => Special::Crp,
        "tt0" => Special::Tt0,
        "tt1" => Special::Tt1,
        "mmusr" | "psr" => Special::Mmusr,
        _ => return None,
    })
}

pub fn is_reserved_name(s: &str) -> bool {
    gpr(s).is_some() || fpr(s).is_some() || fpctl(s).is_some() || special(s).is_some()
}

/// Split at top-level commas (outside (), [], {}, quotes).
pub fn split_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut q: u8 = 0;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            if c == q {
                if b.get(i + 1) == Some(&q) {
                    i += 1;
                } else {
                    q = 0;
                }
            }
        } else {
            match c {
                b'"' => q = c,
                // `'` starts a char constant only where an operand/term can begin
                b'\'' if i == 0 || !b[i - 1].is_ascii_alphanumeric() => q = c,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    out.push(s[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    out.push(s[start..].trim());
    if out.len() == 1 && out[0].is_empty() {
        out.clear();
    }
    out
}

fn expr(s: &str, ctx: &mut dyn Ctx) -> Result<Val, String> {
    let mut p = Parser::new(s, ctx);
    let v = p.expr()?;
    p.skip_ws();
    if !p.at_end() {
        return Err(format!("junk after expression: `{}`", p.rest()));
    }
    Ok(v)
}

/// Strip a trailing `.w` / `.l` size from an expression or address.
fn strip_size(s: &str) -> (&str, Option<Sz>) {
    let t = s.trim();
    if t.len() > 2 {
        let (head, tail) = t.split_at(t.len() - 2);
        match tail {
            ".w" | ".W" => return (head, Some(Sz::W)),
            ".l" | ".L" => return (head, Some(Sz::L)),
            _ => {}
        }
    }
    (t, None)
}

fn disp(s: &str, ctx: &mut dyn Ctx) -> Result<Disp, String> {
    let (e, sz) = strip_size(s);
    Ok(Disp { v: expr(e, ctx)?, sz })
}

/// `d0`, `d0.w`, `a1.l*4`, `d2*8`
fn index(s: &str) -> Option<Index> {
    let s = s.trim();
    let (r, scale) = match s.split_once('*') {
        Some((r, sc)) => {
            let sc = match sc.trim() {
                "1" => 0,
                "2" => 1,
                "4" => 2,
                "8" => 3,
                _ => return None,
            };
            (r.trim(), sc)
        }
        None => (s, 0),
    };
    let (r, long) = match r.len().checked_sub(2).map(|i| r.split_at(i)) {
        Some((h, ".w" | ".W")) => (h, false),
        Some((h, ".l" | ".L")) => (h, true),
        _ => (r, false),
    };
    gpr(r).map(|reg| Index { reg, long, scale })
}

fn base_reg(s: &str) -> Option<BaseReg> {
    let l = s.trim().to_ascii_lowercase();
    match l.as_str() {
        "pc" => return Some(BaseReg::Pc),
        "zpc" => return Some(BaseReg::Zpc),
        _ => {}
    }
    if let Some(r) = l.strip_prefix('z') {
        if let Some(n) = gpr(r).filter(|&n| n >= 8) {
            let _ = n;
            return Some(BaseReg::None);
        }
    }
    match gpr(&l) {
        Some(n) if n >= 8 => Some(BaseReg::An(n - 8)),
        _ => None,
    }
}

/// A register list like `d0-d3/a0/a2-a6` (also accepts a single register).
fn reglist(s: &str) -> Option<u16> {
    let mut m = 0u16;
    for part in s.split('/') {
        let part = part.trim();
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (gpr(a.trim())?, gpr(b.trim())?);
                if a > b {
                    return None;
                }
                for r in a..=b {
                    m |= 1 << r;
                }
            }
            None => m |= 1 << gpr(part)?,
        }
    }
    Some(m)
}

fn fplist(s: &str) -> Option<u8> {
    let mut m = 0u8;
    for part in s.split('/') {
        let part = part.trim();
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (fpr(a.trim())?, fpr(b.trim())?);
                if a > b {
                    return None;
                }
                for r in a..=b {
                    m |= 1 << r;
                }
            }
            None => m |= 1 << fpr(part)?,
        }
    }
    Some(m)
}

fn fpctl_list(s: &str) -> Option<u8> {
    let mut m = 0;
    for p in s.split('/') {
        m |= fpctl(p.trim())?;
    }
    Some(m)
}

/// Does `s` look like a floating-point literal?
pub fn float_literal(s: &str) -> Option<f64> {
    let t = s.trim();
    let body = t.strip_prefix('-').or_else(|| t.strip_prefix('+')).unwrap_or(t);
    if body.is_empty() || !body.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if !(body.contains('.') || body.contains('e') || body.contains('E')) || body.starts_with("0x") {
        return None;
    }
    if !body.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Find the `(` that opens the trailing parenthesised group of `s` (s ends with `)`).
fn trailing_group(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.last() != Some(&b')') {
        return None;
    }
    let mut depth = 0;
    for i in (0..b.len()).rev() {
        match b[i] {
            b')' | b']' => depth += 1,
            b'(' | b'[' => {
                depth -= 1;
                if depth == 0 {
                    return if b[i] == b'(' { Some(i) } else { None };
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse(s: &str, ctx: &mut dyn Ctx) -> Result<Op, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("missing operand".into());
    }
    // bit field: ea{off:width}
    if s.ends_with('}') {
        if let Some(open) = s.rfind('{') {
            let inner = &s[open + 1..s.len() - 1];
            let (o, w) = inner.split_once(':').ok_or("bit field needs {offset:width}")?;
            let arg = |t: &str, ctx: &mut dyn Ctx| -> Result<BfArg, String> {
                match gpr(t.trim()) {
                    Some(r) if r < 8 => Ok(BfArg::D(r)),
                    Some(_) => Err("bit field register must be a data register".into()),
                    None => {
                        let v = expr(t.trim().trim_start_matches('#'), ctx)?;
                        if v.is_unknown() {
                            Ok(BfArg::Imm(0))
                        } else if !v.is_abs() {
                            Err("bit field offset/width must be absolute".into())
                        } else {
                            Ok(BfArg::Imm(v.off as u32))
                        }
                    }
                }
            };
            let off = arg(o, ctx)?;
            let width = arg(w, ctx)?;
            let ea = parse(&s[..open], ctx)?;
            return Ok(Op::BitField { ea: Box::new(ea), off, width });
        }
    }
    if let Some(imm) = s.strip_prefix('#') {
        if let Some(f) = float_literal(imm) {
            return Ok(Op::ImmF(f));
        }
        return Ok(Op::Imm(expr(imm, ctx)?));
    }
    if let Some(r) = gpr(s) {
        return Ok(if r < 8 { Op::D(r) } else { Op::A(r - 8) });
    }
    if let Some(r) = fpr(s) {
        return Ok(Op::Fp(r));
    }
    if let Some(sp) = special(s) {
        return Ok(Op::Spec(sp));
    }
    if s.contains('/') || s.contains('-') && !s.contains('(') {
        if let Some(m) = reglist(s) {
            return Ok(Op::List(m));
        }
        if let Some(m) = fplist(s) {
            return Ok(Op::FpList(m));
        }
        if let Some(m) = fpctl_list(s) {
            return Ok(Op::FpCtl(m));
        }
    }
    if let Some(m) = fpctl(s) {
        return Ok(Op::FpCtl(m));
    }
    if let Some((a, b)) = s.split_once(':') {
        if let (Some(x), Some(y)) = (gpr(a.trim()), gpr(b.trim())) {
            return Ok(Op::Pair(x, y));
        }
        if let (Some(x), Some(y)) = (fpr(a.trim()), fpr(b.trim())) {
            return Ok(Op::FpPair(x, y));
        }
        let ind = |t: &str| t.trim().strip_prefix('(').and_then(|t| t.strip_suffix(')')).and_then(|t| gpr(t.trim()));
        if let (Some(x), Some(y)) = (ind(a), ind(b)) {
            return Ok(Op::IndPair(x, y));
        }
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("-(") && lower.ends_with(')') {
        if let Some(r) = gpr(&s[2..s.len() - 1]).filter(|&r| r >= 8) {
            return Ok(Op::Pre(r - 8));
        }
    }
    if lower.starts_with('(') && lower.ends_with(")+") {
        if let Some(r) = gpr(&s[1..s.len() - 2]).filter(|&r| r >= 8) {
            return Ok(Op::Post(r - 8));
        }
    }
    // absolute with explicit size in parentheses: (expr).w
    let (core_s, abs_sz) = strip_size(s);
    if let Some(open) = trailing_group(core_s) {
        let prefix = core_s[..open].trim();
        let inner = &core_s[open + 1..core_s.len() - 1];
        if let Some(op) = paren_mode(prefix, inner, ctx)? {
            if abs_sz.is_some() {
                return Err(format!("bad operand `{s}`"));
            }
            return Ok(op);
        }
    }
    // plain absolute address
    let (e, sz) = strip_size(s);
    Ok(Op::Abs { v: expr(e, ctx)?, sz })
}

/// Parse `prefix(inner)`. Returns None if this is just a parenthesised expression.
fn paren_mode(prefix: &str, inner: &str, ctx: &mut dyn Ctx) -> Result<Option<Op>, String> {
    let parts = split_commas(inner);
    // Does any part name a register? If not, it's an expression like (1+2).
    let has_reg = parts.iter().any(|p| {
        let p = p.trim();
        p.starts_with('[') || base_reg(p).is_some() || index(p).is_some()
    });
    if !has_reg {
        return Ok(None);
    }
    let mut base: Option<BaseReg> = None;
    let mut bd: Option<Disp> = None;
    let mut ix: Option<Index> = None;
    let mut mem: Option<(bool, Option<Disp>)> = None;
    if !prefix.is_empty() {
        bd = Some(disp(prefix, ctx)?);
    }
    for (i, p) in parts.iter().enumerate() {
        let p = p.trim();
        if let Some(br) = p.strip_prefix('[') {
            let br = br.strip_suffix(']').ok_or("missing `]`")?;
            if i != 0 || mem.is_some() {
                return Err("memory indirect brackets must come first".into());
            }
            for q in split_commas(br) {
                let q = q.trim();
                if let Some(b) = base_reg(q).filter(|_| base.is_none() && ix.is_none() && !q.contains('*') && !q.contains('.')) {
                    base = Some(b);
                } else if let Some(x) = index(q).filter(|_| ix.is_none()) {
                    ix = Some(x);
                } else if bd.is_none() && base.is_none() && ix.is_none() {
                    bd = Some(disp(q, ctx)?);
                } else {
                    return Err(format!("bad memory indirect operand `{q}`"));
                }
            }
            mem = Some((false, None));
            continue;
        }
        let is_first_an = base.is_none() && ix.is_none() && mem.is_none();
        if let Some(b) = base_reg(p).filter(|_| is_first_an && !p.contains('*') && !p.contains('.')) {
            base = Some(b);
        } else if let Some(x) = index(p) {
            if ix.is_some() {
                return Err("two index registers".into());
            }
            if mem.is_some() {
                mem = Some((true, None));
            }
            ix = Some(x);
        } else if let Some((post, None)) = mem {
            mem = Some((post, Some(disp(p, ctx)?)));
        } else if bd.is_none() && base.is_none() && ix.is_none() {
            bd = Some(disp(p, ctx)?);
        } else {
            return Err(format!("bad operand part `{p}`"));
        }
    }
    let base = base.unwrap_or(BaseReg::None);
    if mem.is_none() && ix.is_none() {
        match base {
            BaseReg::An(r) => {
                return Ok(Some(match bd {
                    None => Op::Ind(r),
                    Some(d) if d.sz != Some(Sz::L) => Op::Disp { base, d },
                    Some(d) => Op::Idx { base, bd: Some(d), ix: None, mem: None },
                }));
            }
            BaseReg::Pc => {
                let d = bd.unwrap_or(Disp { v: ctx.pc(), sz: None });
                return Ok(Some(if d.sz == Some(Sz::L) {
                    Op::Idx { base, bd: Some(d), ix: None, mem: None }
                } else {
                    Op::Disp { base, d }
                }));
            }
            _ => {}
        }
    }
    Ok(Some(Op::Idx { base, bd, ix, mem }))
}
