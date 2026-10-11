//! Code generation: the typed syntax tree to azas (Motorola syntax) assembly.
//!
//! Values are computed in d0 (integers and pointers, extended to 32 bits),
//! d0:d1 (long long: d0 holds the high half) or fp0 (float and double); struct
//! values are represented by their address in d0. Intermediate results go on the
//! stack. d0/d1/a0/a1/fp0/fp1 are scratch; d2/d3 are used for long long operands
//! and saved when used; scalar locals whose address is never taken live in
//! d4-d7/a3-a5.
//!
//! Calling convention: arguments are pushed right to left in 4-byte slots
//! (char/short widened to int; long long and double take 8 bytes, high half at
//! the lower address; structs are copied); the caller pops them. Results come
//! back in d0 (also a0), d0:d1 or fp0. A function returning a struct gets the
//! address of the result buffer as a hidden first argument and returns it in d0.
//! d2-d7/a2-a6/fp2-fp7 are preserved by the callee.

use crate::ast::*;
use crate::types::*;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

/// An effective address.
#[derive(Clone, Debug)]
enum Ea {
    /// d(An)
    Disp(i32, &'static str),
    /// absolute symbol + offset
    Abs(String, i64),
    /// a register (register-allocated local)
    Reg(&'static str),
    /// d(An,Dn.l*scale)
    Index(i32, &'static str, &'static str, u8),
}

impl Ea {
    fn at(&self, k: i32) -> Ea {
        match self {
            Ea::Disp(d, r) => Ea::Disp(d + k, r),
            Ea::Abs(s, o) => Ea::Abs(s.clone(), o + k as i64),
            Ea::Reg(r) => Ea::Reg(r),
            Ea::Index(d, a, x, sc) => Ea::Index(d + k, a, x, *sc),
        }
    }
    fn s(&self) -> String {
        match self {
            Ea::Disp(0, r) => format!("({r})"),
            Ea::Disp(d, r) => format!("{d}({r})"),
            Ea::Abs(s, 0) => s.clone(),
            Ea::Abs(s, o) if *o > 0 => format!("{s}+{o}"),
            Ea::Abs(s, o) => format!("{s}{o}"),
            Ea::Reg(r) => String::from(*r),
            Ea::Index(d, a, x, sc) => {
                let scale = if *sc > 1 { format!("*{sc}") } else { String::new() };
                if *d == 0 { format!("({a},{x}.l{scale})") } else { format!("{d}({a},{x}.l{scale})") }
            }
        }
    }
    fn uses(&self, reg: &str) -> bool {
        match self {
            Ea::Disp(_, r) | Ea::Reg(r) => *r == reg,
            Ea::Index(_, a, x, _) => *a == reg || *x == reg,
            Ea::Abs(..) => false,
        }
    }
}

#[derive(Clone)]
enum Loc {
    Frame(i32),
    Reg(&'static str),
}


pub struct Gen<'u> {
    u: &'u Unit,
    out: String,
    body: String,
    label: usize,
    floats: Vec<(u64, bool)>,
    // current function
    locs: Vec<Loc>,
    frame: i32,
    saved: Vec<&'static str>,
    breaks: Vec<String>,
    conts: Vec<String>,
    case_labels: BTreeMap<usize, String>,
    sret: bool,
    fname: String,
    cur_locals: Vec<Local>,
    /// the value of the expression being generated is not used
    discard: bool,
    pub errors: Vec<String>,
}

fn sz(t: &Ty, s: &[StructDef]) -> u32 {
    size_of(t, s)
}

/// Size suffix for a 1/2/4-byte access.
fn suf(n: u32) -> &'static str {
    match n {
        1 => "b",
        2 => "w",
        _ => "l",
    }
}

fn is_word(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::UInt | Ty::Long | Ty::ULong | Ty::Ptr(_))
}

fn cc(op: &str, unsigned: bool) -> &'static str {
    match (op, unsigned) {
        ("==", _) => "eq",
        ("!=", _) => "ne",
        ("<", false) => "lt",
        ("<", true) => "cs",
        (">", false) => "gt",
        (">", true) => "hi",
        ("<=", false) => "le",
        ("<=", true) => "ls",
        (">=", false) => "ge",
        (">=", true) => "cc",
        _ => "ne",
    }
}

fn inverse(op: &str) -> &'static str {
    match op {
        "==" => "!=",
        "!=" => "==",
        "<" => ">=",
        ">" => "<=",
        "<=" => ">",
        _ => "<",
    }
}

fn fcc(op: &str) -> &'static str {
    match op {
        "==" => "eq",
        "!=" => "ne",
        "<" => "lt",
        ">" => "gt",
        "<=" => "le",
        _ => "ge",
    }
}

impl<'u> Gen<'u> {
    pub fn new(u: &'u Unit) -> Gen<'u> {
        Gen {
            u,
            out: String::new(),
            body: String::new(),
            label: 0,
            floats: Vec::new(),
            locs: Vec::new(),
            frame: 0,
            saved: Vec::new(),
            breaks: Vec::new(),
            conts: Vec::new(),
            case_labels: BTreeMap::new(),
            sret: false,
            fname: String::new(),
            cur_locals: Vec::new(),
            discard: false,
            errors: Vec::new(),
        }
    }

    fn st(&self) -> &[StructDef] {
        &self.u.structs
    }

    fn size(&self, t: &Ty) -> u32 {
        sz(t, &self.u.structs)
    }

    fn e(&mut self, s: &str) {
        self.body.push('\t');
        self.body.push_str(s);
        self.body.push('\n');
    }

    fn lab(&mut self, l: &str) {
        self.body.push_str(l);
        self.body.push_str(":\n");
    }

    fn new_label(&mut self) -> String {
        self.label += 1;
        format!(".L{}", self.label)
    }

    fn use_reg(&mut self, r: &'static str) {
        if !self.saved.contains(&r) {
            self.saved.push(r);
        }
    }

    fn float_const(&mut self, v: f64, single: bool) -> String {
        let bits = if single { (v as f32).to_bits() as u64 } else { v.to_bits() };
        let i = match self.floats.iter().position(|x| *x == (bits, single)) {
            Some(i) => i,
            None => {
                self.floats.push((bits, single));
                self.floats.len() - 1
            }
        };
        format!("F{i}")
    }

    // ---- loads and stores ----------------------------------------------------------------

    /// Load a value of type `t` from `ea` into the accumulator.
    fn load(&mut self, ea: &Ea, t: &Ty) {
        let s = ea.s();
        match t {
            Ty::Bool | Ty::UChar => {
                if ea.uses("d0") {
                    self.e(&format!("and.l #$FF,{s}"));
                } else {
                    self.e("moveq #0,d0");
                    self.e(&format!("move.b {s},d0"));
                }
            }
            Ty::Char | Ty::SChar => {
                self.e(&format!("move.b {s},d0"));
                self.e("extb.l d0");
            }
            Ty::Short => {
                self.e(&format!("move.w {s},d0"));
                self.e("ext.l d0");
            }
            Ty::UShort => {
                self.e("moveq #0,d0");
                self.e(&format!("move.w {s},d0"));
            }
            Ty::Int | Ty::UInt | Ty::Long | Ty::ULong | Ty::Ptr(_) => {
                if s != "d0" {
                    self.e(&format!("move.l {s},d0"));
                }
            }
            Ty::LLong | Ty::ULLong => {
                if ea.uses("d0") || ea.uses("d1") {
                    self.e(&format!("lea {s},a1"));
                    self.e("move.l (a1),d0");
                    self.e("move.l 4(a1),d1");
                } else {
                    self.e(&format!("move.l {s},d0"));
                    self.e(&format!("move.l {},d1", ea.at(4).s()));
                }
            }
            Ty::Float => self.e(&format!("fmove.s {s},fp0")),
            Ty::Double | Ty::LDouble => self.e(&format!("fmove.d {s},fp0")),
            Ty::Struct(_) | Ty::Array(..) | Ty::Func(_) => self.addr_of(ea),
            Ty::Void => {}
        }
    }

    /// d0 = address of ea.
    fn addr_of(&mut self, ea: &Ea) {
        match ea {
            Ea::Abs(_, _) => self.e(&format!("move.l #{},d0", ea.s())),
            Ea::Disp(0, r) => self.e(&format!("move.l {r},d0")),
            _ => {
                self.e(&format!("lea {},a0", ea.s()));
                self.e("move.l a0,d0");
            }
        }
    }

    /// Store the accumulator (of type `t`) to `ea`.
    fn store(&mut self, ea: &Ea, t: &Ty) {
        let s = ea.s();
        match t.class() {
            Class::I => {
                let n = self.size(t);
                if s != "d0" {
                    self.e(&format!("move.{} d0,{s}", suf(n)));
                }
            }
            Class::L => {
                self.e(&format!("move.l d0,{s}"));
                self.e(&format!("move.l d1,{}", ea.at(4).s()));
            }
            Class::F => {
                if *t == Ty::Float {
                    self.e(&format!("fmove.s fp0,{s}"));
                } else {
                    self.e(&format!("fmove.d fp0,{s}"));
                }
            }
            Class::S => {
                // copy from the address in d0
                let n = self.size(t);
                if !ea.uses("a1") {
                    self.e("move.l d0,a1");
                    self.e(&format!("lea {s},a0"));
                } else {
                    self.e(&format!("lea {s},a0"));
                    self.e("exg d0,a1");
                }
                self.copy(n);
                self.e(&format!("lea {s},a0"));
                self.e("move.l a0,d0");
            }
            Class::V => {}
        }
    }

    /// Copy n bytes from (a1) to (a0) (both advance).
    fn copy(&mut self, n: u32) {
        let mut left = n;
        if n > 64 {
            let l = self.new_label();
            self.e(&format!("move.l #{},d1", n / 4 - 1));
            self.lab(&l);
            self.e("move.l (a1)+,(a0)+");
            self.e(&format!("dbra d1,{l}"));
            if n / 4 > 65536 {
                // dbra counts 16 bits: continue the outer part
                self.e("sub.l #$10000,d1");
                self.e(&format!("bpl {l}"));
            }
            left = n % 4;
        }
        while left >= 4 {
            self.e("move.l (a1)+,(a0)+");
            left -= 4;
        }
        if left >= 2 {
            self.e("move.w (a1)+,(a0)+");
            left -= 2;
        }
        if left >= 1 {
            self.e("move.b (a1)+,(a0)+");
        }
    }

    fn push_acc(&mut self, t: &Ty) -> i32 {
        match t.class() {
            Class::I | Class::S => {
                self.e("move.l d0,-(sp)");
                4
            }
            Class::L => {
                self.e("move.l d1,-(sp)");
                self.e("move.l d0,-(sp)");
                8
            }
            Class::F => {
                self.e("fmove.x fp0,-(sp)");
                12
            }
            Class::V => 0,
        }
    }

    // ---- lvalues ---------------------------------------------------------------------------

    /// An effective address that needs no code to compute (or None).
    fn static_ea(&self, e: &Expr) -> Option<Ea> {
        match &e.k {
            E::Local(i) => Some(match &self.locs[*i] {
                Loc::Frame(o) => Ea::Disp(*o, "a6"),
                Loc::Reg(r) => Ea::Reg(r),
            }),
            E::Global(n) => Some(Ea::Abs(n.clone(), 0)),
            E::Str(n) => Some(Ea::Abs(format!("L{n}"), 0)),
            E::Member(b, off, _) => {
                let ea = self.static_ea(b)?;
                if matches!(ea, Ea::Reg(_)) {
                    return None;
                }
                Some(ea.at(*off as i32))
            }
            E::Deref(p) => {
                // through a pointer held in an address register
                match &p.k {
                    E::Local(i) => match &self.locs[*i] {
                        Loc::Reg(r) if r.starts_with('a') => Some(Ea::Disp(0, r)),
                        _ => None,
                    },
                    E::Bin("+", a, b) => {
                        let k = b.const_int()?;
                        if let E::Local(i) = &a.k {
                            if let Loc::Reg(r) = &self.locs[*i] {
                                if r.starts_with('a') && (-32768..32768).contains(&k) {
                                    return Some(Ea::Disp(k as i32, r));
                                }
                            }
                        }
                        None
                    }
                    E::Addr(x) => self.static_ea(x),
                    _ => None,
                }
                .or_else(|| self.index_ea(p))
            }
            E::Addr(_) => None,
            _ => None,
        }
    }

    /// p + i*k with p in an address register and i in a register: d(An,Dn*k).
    fn index_ea(&self, p: &Expr) -> Option<Ea> {
        let E::Bin("+", a, b) = &p.k else { return None };
        let E::Local(pi) = &a.k else { return None };
        let Loc::Reg(ar) = &self.locs[*pi] else { return None };
        if !ar.starts_with('a') {
            return None;
        }
        let (idx, scale) = match &b.k {
            E::Bin("*", x, k) => (x, k.const_int()?),
            E::Cast(x) => match &x.k {
                E::Bin("*", y, k) => (y, k.const_int()?),
                _ => (x, 1),
            },
            _ => (b, 1),
        };
        if !matches!(scale, 1 | 2 | 4 | 8) {
            return None;
        }
        let idx = match &idx.k {
            E::Cast(x) if is_word(&x.ty) => x,
            _ => idx,
        };
        let E::Local(ii) = &idx.k else { return None };
        let Loc::Reg(xr) = &self.locs[*ii] else { return None };
        if !is_word(&idx.ty) {
            return None;
        }
        Some(Ea::Index(0, ar, xr, scale as u8))
    }

    /// Compute an lvalue's address; the result may use a0.
    fn lvalue(&mut self, e: &Expr) -> Ea {
        if let Some(ea) = self.static_ea(e) {
            return ea;
        }
        match &e.k {
            E::Deref(p) => {
                // p + constant: fold into the displacement
                if let E::Bin("+", a, b) = &p.k {
                    if let Some(k) = b.const_int() {
                        if (-32768..32768).contains(&k) && a.ty.is_ptr() {
                            self.ex(a);
                            self.e("move.l d0,a0");
                            return Ea::Disp(k as i32, "a0");
                        }
                    }
                }
                if let E::Bin("+", a, b) = &p.k {
                    if a.ty.is_ptr() && b.ty.class() == Class::I {
                        let b = match &b.k {
                            E::Cast(x) if is_word(&x.ty) => x,
                            _ => b,
                        };
                        let (idx, scale) = match &b.k {
                            E::Bin("*", x, k) if matches!(k.const_int(), Some(1 | 2 | 4 | 8)) => (&**x, k.const_int().unwrap()),
                            _ => (&**b, 1),
                        };
                        self.ex(idx);
                        if let Some(sa) = self.src(a).filter(|s| !s.starts_with('#')) {
                            self.e("move.l d0,d1");
                            self.e(&format!("move.l {sa},a0"));
                        } else {
                            self.e("move.l d0,-(sp)");
                            self.ex(a);
                            self.e("move.l d0,a0");
                            self.e("move.l (sp)+,d1");
                        }
                        return Ea::Index(0, "a0", "d1", scale as u8);
                    }
                }
                self.ex(p);
                self.e("move.l d0,a0");
                Ea::Disp(0, "a0")
            }
            E::Member(b, off, _) => {
                let ea = self.lvalue(b);
                match ea {
                    Ea::Reg(_) => {
                        self.err("member of a register value");
                        Ea::Disp(0, "a0")
                    }
                    ea => ea.at(*off as i32),
                }
            }
            E::StmtExpr(s, v) => {
                self.stmt(s);
                self.lvalue(v)
            }
            E::Call(..) | E::Cond(..) | E::Comma(..) | E::Assign(..) | E::VaArg(..) => {
                // struct-valued expressions: their address
                self.ex(e);
                self.e("move.l d0,a0");
                Ea::Disp(0, "a0")
            }
            _ => {
                self.err("expression is not an lvalue");
                Ea::Disp(0, "a0")
            }
        }
    }

    fn err(&mut self, m: &str) {
        let f = self.fname.clone();
        self.errors.push(format!("in function '{f}': internal error: {m}"));
    }

    /// For lvalues whose address needs code: compute it and save it on the stack.
    /// Returns the EA to use after `restore_lvalue` (always (a0) based), or the static EA.
    fn lvalue_saved(&mut self, e: &Expr) -> (Ea, bool) {
        if let Some(ea) = self.static_ea(e) {
            return (ea, false);
        }
        let ea = self.lvalue(e);
        let ea = match ea {
            Ea::Index(..) if ea.uses("a0") => {
                self.e(&format!("lea {},a0", ea.s()));
                Ea::Disp(0, "a0")
            }
            ea => ea,
        };
        match ea {
            Ea::Disp(d, "a0") => {
                if d != 0 {
                    self.e(&format!("lea {d}(a0),a0"));
                }
                self.e("move.l a0,-(sp)");
                (Ea::Disp(0, "a1"), true)
            }
            ea => (ea, false),
        }
    }

    // ---- expressions -----------------------------------------------------------------------

    /// A 32-bit source operand for `e` that needs no code (or None).
    fn src(&mut self, e: &Expr) -> Option<String> {
        match &e.k {
            E::Int(v) if e.ty.class() == Class::I => Some(format!("#{}", *v as i32)),
            E::Member(_, _, Some(_)) => None,
            E::Local(_) | E::Global(_) | E::Member(..) | E::Deref(_) if is_word(&e.ty) => {
                let ea = self.static_ea(e)?;
                Some(ea.s())
            }
            E::Addr(x) if matches!(x.k, E::Global(_) | E::Str(_)) => Some(format!("#{}", self.static_ea(x)?.s())),
            E::Str(n) => Some(format!("#L{n}")),
            E::Cast(x) if is_word(&e.ty) && is_word(&x.ty) => self.src(x),
            _ => None,
        }
    }

    pub fn ex(&mut self, e: &Expr) {
        let discard = core::mem::replace(&mut self.discard, false);
        if discard {
            match &e.k {
                E::IncDec(l, amount, pre) => {
                    self.discard = true;
                    self.incdec(l, *amount, *pre);
                    self.discard = false;
                    return;
                }
                E::OpAssign(op, l, r, t) => {
                    self.op_assign_d(op, l, r, t, true);
                    return;
                }
                E::Assign(l, r) => {
                    self.assign_d(l, r, true);
                    return;
                }
                _ => {}
            }
        }
        match &e.k {
            E::Int(v) => match e.ty.class() {
                Class::L => {
                    let (hi, lo) = ((*v >> 32) as i32, *v as i32);
                    self.load_imm(hi, "d0");
                    self.load_imm(lo, "d1");
                }
                Class::F => {
                    let c = self.float_const(*v as f64, e.ty == Ty::Float);
                    self.e(&format!("fmove.{} {c},fp0", if e.ty == Ty::Float { "s" } else { "d" }));
                }
                _ => self.load_imm(*v as i32, "d0"),
            },
            E::Flt(f) => {
                if e.ty.is_float() {
                    let single = e.ty == Ty::Float;
                    let c = self.float_const(*f, single);
                    self.e(&format!("fmove.{} {c},fp0", if single { "s" } else { "d" }));
                } else {
                    self.load_imm(*f as i32, "d0");
                }
            }
            E::Str(n) => self.e(&format!("move.l #L{n},d0")),
            E::Local(_) | E::Global(_) | E::Deref(_) => {
                if e.ty.is_func() {
                    if let E::Global(n) = &e.k {
                        self.e(&format!("move.l #{n},d0"));
                        return;
                    }
                }
                let ea = self.lvalue(e);
                self.load(&ea, &e.ty);
            }
            E::Member(_, _, bits) => {
                let ea = self.lvalue(e);
                match bits {
                    Some((off, w)) => {
                        let op = if e.ty.is_unsigned() || e.ty == Ty::Bool { "bfextu" } else { "bfexts" };
                        self.e(&format!("{op} {}{{{off}:{w}}},d0", ea.s()));
                    }
                    None => self.load(&ea, &e.ty),
                }
            }
            E::Addr(x) => {
                if let E::StmtExpr(s, v) = &x.k {
                    self.stmt(s);
                    let ea = self.lvalue(v);
                    self.addr_of(&ea);
                    return;
                }
                if x.ty.is_func() {
                    if let E::Global(n) = &x.k {
                        self.e(&format!("move.l #{n},d0"));
                        return;
                    }
                }
                if let E::Str(n) = &x.k {
                    self.e(&format!("move.l #L{n},d0"));
                    return;
                }
                let ea = self.lvalue(x);
                self.addr_of(&ea);
            }
            E::Cast(x) => {
                self.ex(x);
                self.convert(&x.ty, &e.ty);
            }
            E::Unary(op, x) => {
                self.ex(x);
                match (x.ty.class(), *op) {
                    (Class::I, "-") => self.e("neg.l d0"),
                    (Class::I, "~") => self.e("not.l d0"),
                    (Class::I, "!") => {
                        self.e("tst.l d0");
                        self.set_cc("eq");
                    }
                    (Class::L, "-") => {
                        self.e("neg.l d1");
                        self.e("negx.l d0");
                    }
                    (Class::L, "~") => {
                        self.e("not.l d0");
                        self.e("not.l d1");
                    }
                    (Class::L, "!") => {
                        self.e("or.l d1,d0");
                        self.set_cc("eq");
                    }
                    (Class::F, "-") => self.e("fneg.x fp0"),
                    (Class::F, "!") => {
                        self.e("ftst.x fp0");
                        self.e("fseq d0");
                        self.e("extb.l d0");
                        self.e("neg.l d0");
                    }
                    _ => self.err("bad unary operand"),
                }
            }
            E::Bin(op, a, b) => self.binary(op, a, b, &e.ty),
            E::LogAnd(..) | E::LogOr(..) => {
                let f = self.new_label();
                let end = self.new_label();
                self.cond(e, &f, false);
                self.e("moveq #1,d0");
                self.e(&format!("bra {end}"));
                self.lab(&f);
                self.e("moveq #0,d0");
                self.lab(&end);
            }
            E::Assign(l, r) => self.assign_d(l, r, false),
            E::OpAssign(op, l, r, t) => self.op_assign_d(op, l, r, t, false),
            E::IncDec(l, amount, pre) => self.incdec(l, *amount, *pre),
            E::Cond(c, a, b) => {
                let f = self.new_label();
                let end = self.new_label();
                self.cond(c, &f, false);
                self.ex(a);
                self.e(&format!("bra {end}"));
                self.lab(&f);
                self.ex(b);
                self.lab(&end);
            }
            E::Comma(a, b) => {
                self.ex(a);
                self.ex(b);
            }
            E::Call(f, args) => self.call(f, args, &e.ty),
            E::VaStart(ap, off) => {
                let (ea, saved) = self.lvalue_saved(ap);
                if saved {
                    self.e("move.l (sp)+,a1");
                }
                self.e(&format!("lea {off}(a6),a0"));
                self.e("move.l a0,d0");
                self.store(&ea, &Ty::ptr(Ty::Char));
            }
            E::VaArg(ap) => {
                let (ea, saved) = self.lvalue_saved(ap);
                if saved {
                    self.e("move.l (sp),a1");
                }
                let t = &e.ty;
                let n = self.size(t);
                let slot = align_up(n, 4) as i32;
                self.e(&format!("move.l {},a0", ea.s()));
                self.e(&format!("add.l #{slot},{}", ea.s()));
                if saved {
                    self.e("addq.l #4,sp");
                }
                if t.is_struct() {
                    self.e("move.l a0,d0");
                } else {
                    let off = if n < 4 { 4 - n as i32 } else { 0 };
                    self.load(&Ea::Disp(off, "a0"), t);
                }
            }
            E::StmtExpr(s, v) => {
                self.stmt(s);
                self.ex(v);
            }
        }
    }

    /// Evaluate for side effects only.
    fn effect(&mut self, e: &Expr) {
        match &e.k {
            E::Assign(..) | E::OpAssign(..) | E::IncDec(..) => {
                self.discard = true;
                self.ex(e);
                self.discard = false;
            }
            E::Comma(a, b) => {
                self.effect(a);
                self.effect(b);
            }
            E::Cast(x) if e.ty.is_void() => self.effect(x),
            _ => self.ex(e),
        }
    }

    fn load_imm(&mut self, v: i32, r: &str) {
        if (-128..128).contains(&v) {
            self.e(&format!("moveq #{v},{r}"));
        } else {
            self.e(&format!("move.l #{v},{r}"));
        }
    }

    /// d0 = 0/1 from the condition codes.
    fn set_cc(&mut self, c: &str) {
        self.e(&format!("s{c} d0"));
        self.e("extb.l d0");
        self.e("neg.l d0");
    }

    fn helper(&mut self, name: &str, arg_bytes: i32) {
        self.e(&format!("jsr {name}"));
        self.pop_args(arg_bytes);
    }

    fn pop_args(&mut self, n: i32) {
        if n == 0 {
        } else if n <= 8 {
            self.e(&format!("addq.l #{n},sp"));
        } else {
            self.e(&format!("lea {n}(sp),sp"));
        }
    }

    /// Convert the accumulator from `from` to `to`.
    fn convert(&mut self, from: &Ty, to: &Ty) {
        if from == to || to.is_void() {
            return;
        }
        let (fc, tc) = (from.class(), to.class());
        match (fc, tc) {
            (Class::I, Class::I) | (Class::S, Class::I) => self.narrow(from, to),
            (Class::I, Class::L) => {
                self.e("move.l d0,d1");
                if from.is_unsigned() {
                    self.e("moveq #0,d0");
                } else {
                    self.e("add.l d0,d0");
                    self.e("subx.l d0,d0");
                }
            }
            (Class::L, Class::I) => {
                self.e("move.l d1,d0");
                self.narrow(&Ty::UInt, to);
            }
            (Class::L, Class::L) => {}
            (Class::I, Class::F) => {
                self.e("fmove.l d0,fp0");
                if from.is_unsigned() && self.size(from) == 4 {
                    let l = self.new_label();
                    let c = self.float_const(4294967296.0, false);
                    self.e("tst.l d0");
                    self.e(&format!("bpl {l}"));
                    self.e(&format!("fadd.d {c},fp0"));
                    self.lab(&l);
                }
                self.round_float(to);
            }
            (Class::F, Class::I) => {
                if to.is_unsigned() && self.size(to) == 4 {
                    // values >= 2^31 do not fit fmove.l
                    let l = self.new_label();
                    let end = self.new_label();
                    let c = self.float_const(2147483648.0, false);
                    self.e("fintrz.x fp0");
                    self.e(&format!("fcmp.d {c},fp0"));
                    self.e(&format!("fblt {l}"));
                    self.e(&format!("fsub.d {c},fp0"));
                    self.e("fmove.l fp0,d0");
                    self.e("bchg #31,d0");
                    self.e(&format!("bra {end}"));
                    self.lab(&l);
                    self.e("fmove.l fp0,d0");
                    self.lab(&end);
                } else {
                    self.e("fintrz.x fp0");
                    self.e("fmove.l fp0,d0");
                    self.narrow(&Ty::Int, to);
                }
            }
            (Class::F, Class::F) => self.round_float(to),
            (Class::L, Class::F) => {
                self.e("move.l d1,-(sp)");
                self.e("move.l d0,-(sp)");
                self.helper(if from.is_unsigned() { "__ulltod" } else { "__lltod" }, 8);
                self.round_float(to);
            }
            (Class::F, Class::L) => {
                self.e("fmove.d fp0,-(sp)");
                self.helper(if to.is_unsigned() { "__dtoull" } else { "__dtoll" }, 8);
            }
            (Class::I, Class::S) | (Class::S, Class::S) => {}
            _ => {}
        }
    }

    fn round_float(&mut self, to: &Ty) {
        if *to == Ty::Float {
            self.e("fmove.s fp0,-(sp)");
            self.e("fmove.s (sp)+,fp0");
        }
    }

    /// Re-extend d0 for a narrower integer type.
    fn narrow(&mut self, from: &Ty, to: &Ty) {
        let (fs, ts) = (self.size(from), self.size(to));
        match to {
            Ty::Bool => {
                if *from != Ty::Bool {
                    self.e("tst.l d0");
                    self.set_cc("ne");
                }
            }
            Ty::Char | Ty::SChar if fs > 1 || from.is_unsigned() => self.e("extb.l d0"),
            Ty::UChar if fs > 1 || !from.is_unsigned() => self.e("and.l #$FF,d0"),
            Ty::Short if fs > 2 || (fs == 2 && from.is_unsigned()) => self.e("ext.l d0"),
            Ty::UShort if fs > 2 || (fs == 2 && !from.is_unsigned()) || (fs == 1 && !from.is_unsigned()) => self.e("and.l #$FFFF,d0"),
            _ => {
                let _ = ts;
            }
        }
    }

    fn binary(&mut self, op: &str, a: &Expr, b: &Expr, rt: &Ty) {
        let t = &a.ty;
        let cmp = matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=");
        match t.class() {
            Class::I => {
                if cmp {
                    self.cmp_i(a, b);
                    self.set_cc(cc(op, t.is_unsigned()));
                    return;
                }
                self.arith_i(op, a, b, rt);
            }
            Class::L => {
                // right operand to d2:d3
                self.use_reg("d2");
                self.use_reg("d3");
                if op == "<<" || op == ">>" {
                    self.ex(b);
                    self.e("move.l d0,-(sp)");
                    self.ex(a);
                    self.e("move.l d1,-(sp)");
                    self.e("move.l d0,-(sp)");
                    let h = if op == "<<" { "__ashlll" } else if t.is_unsigned() { "__lshrll" } else { "__ashrll" };
                    self.helper(h, 12);
                    return;
                }
                self.ex(b);
                self.e("move.l d1,-(sp)");
                self.e("move.l d0,-(sp)");
                self.ex(a);
                if matches!(op, "*" | "/" | "%") {
                    self.e("move.l d1,-(sp)");
                    self.e("move.l d0,-(sp)");
                    let h = match (op, t.is_unsigned()) {
                        ("*", _) => "__mulll",
                        ("/", false) => "__divll",
                        ("/", true) => "__udivll",
                        (_, false) => "__modll",
                        (_, true) => "__umodll",
                    };
                    self.helper(h, 16);
                    return;
                }
                self.e("move.l (sp)+,d2");
                self.e("move.l (sp)+,d3");
                match op {
                    "+" => {
                        self.e("add.l d3,d1");
                        self.e("addx.l d2,d0");
                    }
                    "-" => {
                        self.e("sub.l d3,d1");
                        self.e("subx.l d2,d0");
                    }
                    "&" | "|" | "^" => {
                        let m = match op {
                            "&" => "and",
                            "|" => "or",
                            _ => "eor",
                        };
                        self.e(&format!("{m}.l d2,d0"));
                        self.e(&format!("{m}.l d3,d1"));
                    }
                    _ => {
                        // comparison
                        let yes = self.new_label();
                        let no = self.new_label();
                        let end = self.new_label();
                        self.ll_compare(op, t.is_unsigned(), &yes, &no);
                        self.lab(&yes);
                        self.e("moveq #1,d0");
                        self.e(&format!("bra {end}"));
                        self.lab(&no);
                        self.e("moveq #0,d0");
                        self.lab(&end);
                    }
                }
            }
            Class::F => {
                // right operand
                let rsrc = match &b.k {
                    E::Flt(f) => {
                        let single = b.ty == Ty::Float;
                        let c = self.float_const(*f, single);
                        Some(format!("{}{c}", if single { "s " } else { "d " }))
                    }
                    E::Local(_) | E::Global(_) => self.static_ea(b).filter(|ea| !matches!(ea, Ea::Reg(_))).map(|ea| format!("{}{}", if b.ty == Ty::Float { "s " } else { "d " }, ea.s())),
                    _ => None,
                };
                let (size, src) = match rsrc {
                    Some(s) => {
                        self.ex(a);
                        let (sz, ea) = s.split_once(' ').unwrap();
                        (String::from(sz), String::from(ea))
                    }
                    None => {
                        self.ex(b);
                        self.e("fmove.x fp0,-(sp)");
                        self.ex(a);
                        self.e("fmove.x (sp)+,fp1");
                        (String::from("x"), String::from("fp1"))
                    }
                };
                if cmp {
                    self.e(&format!("fcmp.{size} {src},fp0"));
                    self.e(&format!("fs{} d0", fcc(op)));
                    self.e("extb.l d0");
                    self.e("neg.l d0");
                    return;
                }
                let m = match op {
                    "+" => "fadd",
                    "-" => "fsub",
                    "*" => "fmul",
                    _ => "fdiv",
                };
                self.e(&format!("{m}.{size} {src},fp0"));
                self.round_float(rt);
            }
            _ => self.err("bad binary operands"),
        }
    }

    /// Compare d0:d1 with d2:d3 and branch.
    fn ll_compare(&mut self, op: &str, unsigned: bool, yes: &str, no: &str) {
        self.e("cmp.l d2,d0");
        match op {
            "==" => {
                self.e(&format!("bne {no}"));
                self.e("cmp.l d3,d1");
                self.e(&format!("beq {yes}"));
                self.e(&format!("bra {no}"));
            }
            "!=" => {
                self.e(&format!("bne {yes}"));
                self.e("cmp.l d3,d1");
                self.e(&format!("bne {yes}"));
                self.e(&format!("bra {no}"));
            }
            _ => {
                // high halves decide unless equal; then the low halves (unsigned)
                let strict = cc(if op.starts_with('<') { "<" } else { ">" }, unsigned);
                let other = cc(if op.starts_with('<') { ">" } else { "<" }, unsigned);
                self.e(&format!("b{strict} {yes}"));
                self.e(&format!("b{other} {no}"));
                self.e("cmp.l d3,d1");
                self.e(&format!("b{} {yes}", cc(op, true)));
                self.e(&format!("bra {no}"));
            }
        }
    }

    /// Set the condition codes for a - b (32-bit).
    fn cmp_i(&mut self, a: &Expr, b: &Expr) {
        if b.const_int() == Some(0) {
            if let Some(s) = self.src(a).filter(|s| !s.starts_with('#') && !s.starts_with('a')) {
                self.e(&format!("tst.l {s}"));
                return;
            }
            self.ex(a);
            self.e("tst.l d0");
            return;
        }
        if let (Some(sa), Some(sb)) = (self.src(a), self.src(b)) {
            // cmp #imm,<ea>, or cmp <ea>,Dn
            if sb.starts_with('#') && !sa.starts_with('#') {
                self.e(&format!("cmp.l {sb},{sa}"));
                return;
            }
            if sa.starts_with('d') && sa.len() == 2 {
                self.e(&format!("cmp.l {sb},{sa}"));
                return;
            }
        }
        if let Some(s) = self.src(b) {
            self.ex(a);
            self.e(&format!("cmp.l {s},d0"));
        } else {
            self.ex(b);
            self.e("move.l d0,-(sp)");
            self.ex(a);
            self.e("cmp.l (sp)+,d0");
        }
    }

    fn arith_i(&mut self, op: &str, a: &Expr, b: &Expr, t: &Ty) {
        let uns = t.is_unsigned();
        // constant right operands
        if let Some(k) = b.const_int() {
            let k32 = k as i32;
            match op {
                "+" | "-" => {
                    self.ex(a);
                    if k32 == 0 {
                        return;
                    }
                    let (m, v) = if (1..=8).contains(&k32) { (if op == "+" { "addq" } else { "subq" }, k32) } else if (-8..=-1).contains(&k32) { (if op == "+" { "subq" } else { "addq" }, -k32) } else { ("", 0) };
                    if m.is_empty() {
                        self.e(&format!("{}.l #{k32},d0", if op == "+" { "add" } else { "sub" }));
                    } else {
                        self.e(&format!("{m}.l #{v},d0"));
                    }
                    return;
                }
                "*" if k32 > 0 && (k32 as u32).is_power_of_two() => {
                    self.ex(a);
                    self.shift_const("asl", (k32 as u32).trailing_zeros());
                    return;
                }
                "/" if uns && k32 > 0 && (k32 as u32).is_power_of_two() => {
                    self.ex(a);
                    self.shift_const("lsr", (k32 as u32).trailing_zeros());
                    return;
                }
                "%" if uns && k32 > 0 && (k32 as u32).is_power_of_two() => {
                    self.ex(a);
                    self.e(&format!("and.l #{},d0", k32 - 1));
                    return;
                }
                "<<" | ">>" => {
                    self.ex(a);
                    let m = if op == "<<" { "lsl" } else if uns { "lsr" } else { "asr" };
                    self.shift_const(m, (k as u32) & 63);
                    return;
                }
                "&" | "|" | "^" => {
                    self.ex(a);
                    let m = match op {
                        "&" => "and",
                        "|" => "or",
                        _ => "eor",
                    };
                    self.e(&format!("{m}.l #{k32},d0"));
                    return;
                }
                _ => {}
            }
        }
        // the right operand as a source operand or in d1
        // address registers are only valid sources for add/sub
        let areg_ok = |s: &String| !(s.len() == 2 && s.starts_with('a')) || matches!(op, "+" | "-");
        let s = match self.src(b) {
            Some(s) if op != "^" && op != "<<" && op != ">>" && areg_ok(&s) => {
                self.ex(a);
                s
            }
            _ if self.src(a).is_some() => {
                self.ex(b);
                self.e("move.l d0,d1");
                let sa = self.src(a).unwrap();
                self.e(&format!("move.l {sa},d0"));
                String::from("d1")
            }
            _ => {
                self.ex(b);
                self.e("move.l d0,-(sp)");
                self.ex(a);
                self.e("move.l (sp)+,d1");
                String::from("d1")
            }
        };
        match op {
            "+" => self.e(&format!("add.l {s},d0")),
            "-" => self.e(&format!("sub.l {s},d0")),
            "&" => self.e(&format!("and.l {s},d0")),
            "|" => self.e(&format!("or.l {s},d0")),
            "^" => self.e(&format!("eor.l {s},d0")),
            "*" => self.e(&format!("{}.l {s},d0", if uns { "mulu" } else { "muls" })),
            "/" => self.e(&format!("{}.l {s},d0", if uns { "divu" } else { "divs" })),
            "%" => {
                self.use_reg("d2");
                self.e(&format!("{}.l {s},d2:d0", if uns { "divul" } else { "divsl" }));
                self.e("move.l d2,d0");
            }
            "<<" => self.e(&format!("lsl.l {s},d0")),
            ">>" => self.e(&format!("{}.l {s},d0", if uns { "lsr" } else { "asr" })),
            _ => self.err("bad operator"),
        }
    }

    fn shift_const(&mut self, m: &str, n: u32) {
        if n == 0 {
            return;
        }
        if n >= 32 {
            if m == "asr" {
                self.e("moveq #31,d1");
                self.e("asr.l d1,d0");
            } else {
                self.e("moveq #0,d0");
            }
            return;
        }
        if n <= 8 {
            self.e(&format!("{m}.l #{n},d0"));
        } else if n == 16 && m != "asr" {
            self.e("swap d0");
            if m == "lsr" {
                self.e("and.l #$FFFF,d0");
            } else {
                self.e("clr.w d0");
            }
        } else {
            self.e(&format!("moveq #{n},d1"));
            self.e(&format!("{m}.l d1,d0"));
        }
    }

    // ---- conditions ------------------------------------------------------------------------

    /// Branch to `target` if the condition is `when` (true/false).
    fn cond(&mut self, e: &Expr, target: &str, when: bool) {
        match &e.k {
            E::Int(v) => {
                if (*v != 0) == when {
                    self.e(&format!("bra {target}"));
                }
            }
            E::Unary("!", x) => self.cond(x, target, !when),
            E::LogAnd(a, b) => {
                if when {
                    let skip = self.new_label();
                    self.cond(a, &skip, false);
                    self.cond(b, target, true);
                    self.lab(&skip);
                } else {
                    self.cond(a, target, false);
                    self.cond(b, target, false);
                }
            }
            E::LogOr(a, b) => {
                if when {
                    self.cond(a, target, true);
                    self.cond(b, target, true);
                } else {
                    let skip = self.new_label();
                    self.cond(a, &skip, true);
                    self.cond(b, target, false);
                    self.lab(&skip);
                }
            }
            E::Bin(op, a, b) if matches!(*op, "==" | "!=" | "<" | ">" | "<=" | ">=") && a.ty.class() == Class::I => {
                self.cmp_i(a, b);
                let o = if when { *op } else { inverse(op) };
                self.e(&format!("b{} {target}", cc(o, a.ty.is_unsigned())));
            }
            E::Bin(op, a, b) if matches!(*op, "==" | "!=" | "<" | ">" | "<=" | ">=") && a.ty.class() == Class::L => {
                self.use_reg("d2");
                self.use_reg("d3");
                self.ex(b);
                self.e("move.l d1,-(sp)");
                self.e("move.l d0,-(sp)");
                self.ex(a);
                self.e("move.l (sp)+,d2");
                self.e("move.l (sp)+,d3");
                let other = self.new_label();
                if when {
                    self.ll_compare(op, a.ty.is_unsigned(), target, &other);
                } else {
                    self.ll_compare(op, a.ty.is_unsigned(), &other, target);
                }
                self.lab(&other);
            }
            _ => {
                match e.ty.class() {
                    Class::I => {
                        if let Some(s) = self.src(e).filter(|s| !s.starts_with('#')) {
                            self.e(&format!("tst.l {s}"));
                        } else {
                            self.ex(e);
                            self.e("tst.l d0");
                        }
                        self.e(&format!("b{} {target}", if when { "ne" } else { "eq" }));
                    }
                    Class::L => {
                        self.ex(e);
                        self.e("or.l d1,d0");
                        self.e(&format!("b{} {target}", if when { "ne" } else { "eq" }));
                    }
                    Class::F => {
                        self.ex(e);
                        self.e("ftst.x fp0");
                        self.e(&format!("fb{} {target}", if when { "ne" } else { "eq" }));
                    }
                    _ => {
                        self.ex(e);
                        self.e("tst.l d0");
                        self.e(&format!("b{} {target}", if when { "ne" } else { "eq" }));
                    }
                }
            }
        }
    }

    // ---- assignments -----------------------------------------------------------------------

    fn store_member(&mut self, l: &Expr, ea: &Ea) {
        if let E::Member(_, _, Some((off, w))) = &l.k {
            self.e(&format!("bfins d0,{}{{{off}:{w}}}", ea.s()));
            // the value of the assignment is the truncated field
            let op = if l.ty.is_unsigned() || l.ty == Ty::Bool { "bfextu" } else { "bfexts" };
            self.e(&format!("{op} {}{{{off}:{w}}},d0", ea.s()));
        } else {
            self.store(ea, &l.ty);
        }
    }

    fn assign_d(&mut self, l: &Expr, r: &Expr, discard: bool) {
        if let Some(ea) = self.static_ea(l) {
            // constants straight to memory or registers
            if let (Some(v), Class::I, Ea::Reg(reg)) = (r.const_int(), l.ty.class(), &ea) {
                let v = v as i32;
                if reg.starts_with('d') && (-128..128).contains(&v) {
                    self.e(&format!("moveq #{v},{reg}"));
                } else if reg.starts_with('a') && v == 0 {
                    self.e(&format!("sub.l {reg},{reg}"));
                } else {
                    self.e(&format!("move.l #{v},{reg}"));
                }
                if !discard {
                    self.e(&format!("move.l {reg},d0"));
                }
                return;
            }
            if let (Some(v), Class::I) = (r.const_int(), l.ty.class()) {
                if !matches!(l.k, E::Member(_, _, Some(_))) && !matches!(ea, Ea::Reg(_)) {
                    let n = self.size(&l.ty);
                    if v == 0 {
                        self.e(&format!("clr.{} {}", suf(n), ea.s()));
                        if !discard {
                            self.e("moveq #0,d0");
                        }
                    } else if discard {
                        self.e(&format!("move.{} #{},{}", suf(n), v as i32, ea.s()));
                    } else {
                        self.load_imm(v as i32, "d0");
                        self.e(&format!("move.{} d0,{}", suf(n), ea.s()));
                    }
                    return;
                }
            }
            self.ex(r);
            self.store_member(l, &ea);
            return;
        }
        let ea = self.lvalue(l);
        let ea = match ea {
            Ea::Index(..) if ea.uses("a0") => {
                self.e(&format!("lea {},a0", ea.s()));
                Ea::Disp(0, "a0")
            }
            ea => ea,
        };
        let base_a0 = match ea {
            Ea::Disp(d, "a0") => Some(d),
            _ => None,
        };
        match base_a0 {
            Some(d) => {
                self.e("move.l a0,-(sp)");
                self.ex(r);
                self.e("move.l (sp)+,a1");
                let ea = Ea::Disp(d, "a1");
                if l.ty.is_struct() {
                    // store() copies through a0/a1
                    self.e(&format!("lea {},a1", ea.s()));
                    self.e("move.l a1,-(sp)");
                    self.e("move.l d0,a1");
                    self.e("move.l (sp),a0");
                    let n = self.size(&l.ty);
                    self.copy(n);
                    self.e("move.l (sp)+,d0");
                } else {
                    self.store_member(l, &ea);
                }
            }
            None => {
                self.ex(r);
                self.store_member(l, &ea);
            }
        }
    }

    fn op_assign_d(&mut self, op: &str, l: &Expr, r: &Expr, t: &Ty, discard: bool) {
        let lt = &l.ty;
        let bitfield = matches!(l.k, E::Member(_, _, Some(_)));
        // fast path: word-sized integer lvalue with a constant or simple operand
        if let (Some(ea), true) = (self.static_ea(l), is_word(lt) && lt == t && !bitfield) {
            let is_dreg = matches!(&ea, Ea::Reg(r) if r.starts_with('d')) || !matches!(ea, Ea::Reg(_));
            if let Some(k) = r.const_int() {
                let k = k as i32;
                let m = match op {
                    "+" if (1..=8).contains(&k) => Some(format!("addq.l #{k},{}", ea.s())),
                    "-" if (1..=8).contains(&k) => Some(format!("subq.l #{k},{}", ea.s())),
                    "+" => Some(format!("add.l #{k},{}", ea.s())),
                    "-" => Some(format!("sub.l #{k},{}", ea.s())),
                    "&" if is_dreg => Some(format!("and.l #{k},{}", ea.s())),
                    "|" if is_dreg => Some(format!("or.l #{k},{}", ea.s())),
                    "^" if is_dreg => Some(format!("eor.l #{k},{}", ea.s())),
                    _ => None,
                };
                if let Some(m) = m {
                    self.e(&m);
                    if !discard {
                        self.load(&ea, lt);
                    }
                    return;
                }
            }
            if matches!(op, "+" | "-") || (matches!(op, "&" | "|") && is_dreg) {
                self.ex(r);
                let m = match op {
                    "+" => "add",
                    "-" => "sub",
                    "&" => "and",
                    _ => "or",
                };
                self.e(&format!("{m}.l d0,{}", ea.s()));
                if !discard {
                    self.load(&ea, lt);
                }
                return;
            }
        }
        // general case: address on the stack, value computed in the operation type
        let (ea, saved) = self.lvalue_saved(l);
        let rb = {
            self.ex(r);
            self.push_acc(&r.ty)
        };
        if saved {
            self.e(&format!("move.l {rb}(sp),a1"));
        }
        // load the left value and convert to the operation type
        if let E::Member(_, _, Some((off, w))) = &l.k {
            let opn = if lt.is_unsigned() || *lt == Ty::Bool { "bfextu" } else { "bfexts" };
            self.e(&format!("{opn} {}{{{off}:{w}}},d0", ea.s()));
        } else {
            self.load(&ea, lt);
        }
        self.convert(lt, t);
        // pop the right operand and operate
        match t.class() {
            Class::I => {
                self.e("move.l (sp)+,d1");
                let uns = t.is_unsigned();
                let ins = match op {
                    "+" => "add.l d1,d0".to_string(),
                    "-" => "sub.l d1,d0".to_string(),
                    "&" => "and.l d1,d0".to_string(),
                    "|" => "or.l d1,d0".to_string(),
                    "^" => "eor.l d1,d0".to_string(),
                    "*" => format!("{}.l d1,d0", if uns { "mulu" } else { "muls" }),
                    "/" => format!("{}.l d1,d0", if uns { "divu" } else { "divs" }),
                    "%" => {
                        self.use_reg("d2");
                        format!("{}.l d1,d2:d0\n\tmove.l d2,d0", if uns { "divul" } else { "divsl" })
                    }
                    "<<" => "lsl.l d1,d0".to_string(),
                    _ => format!("{}.l d1,d0", if uns { "lsr" } else { "asr" }),
                };
                self.e(&ins);
            }
            Class::L => {
                self.use_reg("d2");
                self.use_reg("d3");
                if op == "<<" || op == ">>" {
                    // count (int) is on the stack: call the helper with (value, count)
                    self.e("move.l (sp)+,d2");
                    self.e("move.l d2,-(sp)");
                    self.e("move.l d1,-(sp)");
                    self.e("move.l d0,-(sp)");
                    let h = if op == "<<" { "__ashlll" } else if t.is_unsigned() { "__lshrll" } else { "__ashrll" };
                    self.helper(h, 12);
                } else if matches!(op, "*" | "/" | "%") {
                    // right operand (8 bytes) is on the stack already: push left
                    self.e("move.l d1,-(sp)");
                    self.e("move.l d0,-(sp)");
                    let h = match (op, t.is_unsigned()) {
                        ("*", _) => "__mulll",
                        ("/", false) => "__divll",
                        ("/", true) => "__udivll",
                        (_, false) => "__modll",
                        (_, true) => "__umodll",
                    };
                    self.helper(h, 16);
                } else {
                    self.e("move.l (sp)+,d2");
                    self.e("move.l (sp)+,d3");
                    match op {
                        "+" => {
                            self.e("add.l d3,d1");
                            self.e("addx.l d2,d0");
                        }
                        "-" => {
                            self.e("sub.l d3,d1");
                            self.e("subx.l d2,d0");
                        }
                        _ => {
                            let m = match op {
                                "&" => "and",
                                "|" => "or",
                                _ => "eor",
                            };
                            self.e(&format!("{m}.l d2,d0"));
                            self.e(&format!("{m}.l d3,d1"));
                        }
                    }
                }
            }
            Class::F => {
                self.e("fmove.x (sp)+,fp1");
                let m = match op {
                    "+" => "fadd",
                    "-" => "fsub",
                    "*" => "fmul",
                    _ => "fdiv",
                };
                self.e(&format!("{m}.x fp1,fp0"));
            }
            _ => self.err("bad compound assignment"),
        }
        self.convert(t, lt);
        if saved {
            self.e("move.l (sp)+,a1");
        }
        self.store_member(l, &ea);
    }

    fn incdec(&mut self, l: &Expr, amount: i64, pre: bool) {
        let lt = &l.ty;
        let bitfield = matches!(l.k, E::Member(_, _, Some(_)));
        if lt.class() == Class::I && !bitfield {
            let (ea, saved) = self.lvalue_saved(l);
            if saved {
                self.e("move.l (sp)+,a1");
            }
            let n = self.size(lt);
            let k = amount as i32;
            let ins = if (1..=8).contains(&k) {
                format!("addq.{} #{k},{}", suf(n), ea.s())
            } else if (-8..=-1).contains(&k) {
                format!("subq.{} #{},{}", suf(n), -k, ea.s())
            } else {
                format!("add.{} #{k},{}", suf(n), ea.s())
            };
            if self.discard {
                self.e(&ins);
            } else if pre {
                self.e(&ins);
                self.load(&ea, lt);
            } else {
                self.load(&ea, lt);
                self.e(&ins);
            }
            return;
        }
        // general: load, add, store (value: old or new)
        let one = Expr::new(E::Int(amount), if lt.is_ptr() { Ty::Int } else { lt.clone() });
        let one = if lt.is_float() { Expr::new(E::Flt(amount as f64), lt.clone()) } else { one };
        let (ea, saved) = self.lvalue_saved(l);
        if saved {
            self.e("move.l (sp),a1");
        }
        if let E::Member(_, _, Some((off, w))) = &l.k {
            let opn = if lt.is_unsigned() { "bfextu" } else { "bfexts" };
            self.e(&format!("{opn} {}{{{off}:{w}}},d0", ea.s()));
        } else {
            self.load(&ea, lt);
        }
        if !pre {
            self.push_acc(lt);
        }
        match lt.class() {
            Class::I => self.e(&format!("add.l #{},d0", amount as i32)),
            Class::L => {
                self.use_reg("d2");
                self.use_reg("d3");
                self.load_imm((amount >> 32) as i32, "d2");
                self.load_imm(amount as i32, "d3");
                self.e("add.l d3,d1");
                self.e("addx.l d2,d0");
            }
            Class::F => {
                let _ = &one;
                let c = self.float_const(amount as f64, false);
                self.e(&format!("fadd.d {c},fp0"));
            }
            _ => {}
        }
        let pushed = if pre { 0 } else { self.size_on_stack(lt) };
        if saved {
            self.e(&format!("move.l {pushed}(sp),a1"));
        }
        self.store_member(l, &ea);
        if !pre {
            match lt.class() {
                Class::L => {
                    self.e("move.l (sp)+,d0");
                    self.e("move.l (sp)+,d1");
                }
                Class::F => self.e("fmove.x (sp)+,fp0"),
                _ => self.e("move.l (sp)+,d0"),
            }
        }
        if saved {
            self.e("addq.l #4,sp");
        }
    }

    fn size_on_stack(&self, t: &Ty) -> i32 {
        match t.class() {
            Class::L => 8,
            Class::F => 12,
            _ => 4,
        }
    }

    // ---- calls -----------------------------------------------------------------------------

    fn call(&mut self, f: &Expr, args: &[Expr], ret: &Ty) {
        let ft = f.ty.func().cloned();
        let mut bytes = 0;
        for (i, a) in args.iter().enumerate().rev() {
            let pt = ft.as_ref().and_then(|ft| if ft.proto { ft.params.get(i).cloned() } else { None });
            bytes += self.push_arg(a, pt.as_ref());
        }
        let sret = ret.is_struct();
        let mut temp = 0;
        if sret {
            // a buffer in the frame for the result
            let n = align_up(self.size(ret), 4) as i32;
            self.frame += n;
            temp = -self.frame;
            self.e(&format!("pea {temp}(a6)"));
            bytes += 4;
        }
        let direct = match &f.k {
            E::Addr(x) => match &x.k {
                E::Global(n) if x.ty.is_func() => Some(n.clone()),
                _ => None,
            },
            E::Global(n) if f.ty.is_func() => Some(n.clone()),
            _ => None,
        };
        match direct {
            Some(n) => self.e(&format!("jsr {n}")),
            None => {
                self.ex(f);
                self.e("move.l d0,a0");
                self.e("jsr (a0)");
            }
        }
        self.pop_args(bytes);
        if sret {
            self.e(&format!("lea {temp}(a6),a0"));
            self.e("move.l a0,d0");
        }
    }

    fn push_arg(&mut self, a: &Expr, pt: Option<&Ty>) -> i32 {
        let t = pt.cloned().unwrap_or_else(|| a.ty.clone());
        match t.class() {
            Class::I => {
                if let Some(s) = self.src(a) {
                    if let Some(v) = a.const_int() {
                        if (-32768..32768).contains(&v) {
                            self.e(&format!("pea {v}.w"));
                            return 4;
                        }
                    }
                    if let E::Addr(x) = &a.k {
                        if matches!(x.k, E::Global(_) | E::Str(_)) {
                            self.e(&format!("pea {}", &s[1..]));
                            return 4;
                        }
                    }
                    self.e(&format!("move.l {s},-(sp)"));
                    return 4;
                }
                self.ex(a);
                self.e("move.l d0,-(sp)");
                4
            }
            Class::L => {
                self.ex(a);
                self.e("move.l d1,-(sp)");
                self.e("move.l d0,-(sp)");
                8
            }
            Class::F => {
                self.ex(a);
                if t == Ty::Float {
                    self.e("fmove.s fp0,-(sp)");
                    4
                } else {
                    self.e("fmove.d fp0,-(sp)");
                    8
                }
            }
            Class::S => {
                let n = align_up(self.size(&t), 4);
                self.ex(a);
                self.e("move.l d0,a1");
                self.e(&format!("sub.l #{n},sp"));
                self.e("move.l sp,a0");
                self.copy(self.size(&t));
                n as i32
            }
            Class::V => 0,
        }
    }

    // ---- statements ------------------------------------------------------------------------

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Expr(e) => self.effect(e),
            Stmt::Block(v) => {
                for s in v {
                    self.stmt(s);
                }
            }
            Stmt::If(c, a, b) => {
                let f = self.new_label();
                self.cond(c, &f, false);
                self.stmt(a);
                match b {
                    Some(b) => {
                        let end = self.new_label();
                        self.e(&format!("bra {end}"));
                        self.lab(&f);
                        self.stmt(b);
                        self.lab(&end);
                    }
                    None => self.lab(&f),
                }
            }
            Stmt::While(c, body) => {
                let top = self.new_label();
                let test = self.new_label();
                let end = self.new_label();
                self.e(&format!("bra {test}"));
                self.lab(&top);
                self.breaks.push(end.clone());
                self.conts.push(test.clone());
                self.stmt(body);
                self.breaks.pop();
                self.conts.pop();
                self.lab(&test);
                self.cond(c, &top, true);
                self.lab(&end);
            }
            Stmt::DoWhile(body, c) => {
                let top = self.new_label();
                let test = self.new_label();
                let end = self.new_label();
                self.lab(&top);
                self.breaks.push(end.clone());
                self.conts.push(test.clone());
                self.stmt(body);
                self.breaks.pop();
                self.conts.pop();
                self.lab(&test);
                self.cond(c, &top, true);
                self.lab(&end);
            }
            Stmt::For(init, c, step, body) => {
                if let Some(i) = init {
                    self.stmt(i);
                }
                let top = self.new_label();
                let next = self.new_label();
                let test = self.new_label();
                let end = self.new_label();
                self.e(&format!("bra {test}"));
                self.lab(&top);
                self.breaks.push(end.clone());
                self.conts.push(next.clone());
                self.stmt(body);
                self.breaks.pop();
                self.conts.pop();
                self.lab(&next);
                if let Some(s) = step {
                    self.effect(s);
                }
                self.lab(&test);
                match c {
                    Some(c) => self.cond(c, &top, true),
                    None => self.e(&format!("bra {top}")),
                }
                self.lab(&end);
            }
            Stmt::Switch(e, body, cases, def) => {
                let end = self.new_label();
                for (_, l) in cases {
                    let n = self.new_label();
                    self.case_labels.insert(*l, n);
                }
                if let Some(d) = def {
                    let n = self.new_label();
                    self.case_labels.insert(*d, n);
                }
                let default = def.map(|d| self.case_labels[&d].clone()).unwrap_or_else(|| end.clone());
                self.ex(e);
                if e.ty.class() == Class::L {
                    self.e("move.l d1,d0");
                }
                let mut sorted: Vec<(i64, String)> = cases.iter().map(|(v, l)| (*v, self.case_labels[l].clone())).collect();
                sorted.sort_by_key(|x| x.0);
                let n = sorted.len() as i64;
                let (min, max) = (sorted.first().map_or(0, |x| x.0), sorted.last().map_or(0, |x| x.0));
                let range = max - min + 1;
                if n >= 4 && range <= 3 * n + 8 {
                    // jump table
                    let tab = self.new_label();
                    if min != 0 {
                        self.e(&format!("sub.l #{},d0", min as i32));
                    }
                    self.e(&format!("cmp.l #{},d0", (range - 1) as i32));
                    self.e(&format!("bhi {default}"));
                    self.e("lsl.l #2,d0");
                    self.e(&format!("lea {tab},a0"));
                    self.e("move.l (a0,d0.l),a0");
                    self.e("jmp (a0)");
                    self.e("even");
                    self.lab(&tab);
                    let mut it = sorted.iter().peekable();
                    for v in min..=max {
                        match it.peek() {
                            Some((cv, l)) if *cv == v => {
                                self.e(&format!("dc.l {l}"));
                                it.next();
                            }
                            _ => self.e(&format!("dc.l {default}")),
                        }
                    }
                } else {
                    for (v, l) in &sorted {
                        if *v == 0 {
                            self.e("tst.l d0");
                        } else {
                            self.e(&format!("cmp.l #{},d0", *v as i32));
                        }
                        self.e(&format!("beq {l}"));
                    }
                    self.e(&format!("bra {default}"));
                }
                self.breaks.push(end.clone());
                self.stmt(body);
                self.breaks.pop();
                self.lab(&end);
            }
            Stmt::CaseLabel(l) => {
                let n = self.case_labels.get(l).cloned().unwrap_or_else(|| format!(".Lcase{l}"));
                self.lab(&n);
            }
            Stmt::Break => {
                if let Some(l) = self.breaks.last().cloned() {
                    self.e(&format!("bra {l}"));
                }
            }
            Stmt::Continue => {
                if let Some(l) = self.conts.last().cloned() {
                    self.e(&format!("bra {l}"));
                }
            }
            Stmt::Return(v) => {
                if let Some(v) = v {
                    self.ex(v);
                    if self.sret {
                        self.e("move.l d0,a1");
                        self.e("move.l 8(a6),a0");
                        let n = self.size(&v.ty);
                        self.copy(n);
                        self.e("move.l 8(a6),d0");
                    }
                    if v.ty.is_ptr() {
                        self.e("move.l d0,a0");
                    }
                }
                self.e("bra .Lret");
            }
            Stmt::Goto(l) => self.e(&format!("bra .Lu_{l}")),
            Stmt::Label(l) => {
                if let Some(text) = l.strip_prefix('\u{1}') {
                    for line in text.split('\n') {
                        let line = line.trim();
                        if !line.is_empty() {
                            self.e(line);
                        }
                    }
                } else {
                    self.lab(&format!(".Lu_{l}"));
                }
            }
            Stmt::Init(id, items, zero) => self.init_local(*id, items, *zero),
            Stmt::Empty => {}
        }
    }

    fn init_local(&mut self, id: usize, items: &[InitItem], zero: bool) {
        let loc = self.locs[id].clone();
        let t = self.u_local_ty(id);
        let base = match loc {
            Loc::Frame(o) => Ea::Disp(o, "a6"),
            Loc::Reg(r) => Ea::Reg(r),
        };
        if zero {
            let n = self.size(&t);
            if let Ea::Disp(o, _) = base {
                if n <= 48 {
                    let mut k = 0;
                    while k + 4 <= n {
                        self.e(&format!("clr.l {}(a6)", o + k as i32));
                        k += 4;
                    }
                    if k + 2 <= n {
                        self.e(&format!("clr.w {}(a6)", o + k as i32));
                        k += 2;
                    }
                    if k < n {
                        self.e(&format!("clr.b {}(a6)", o + k as i32));
                    }
                } else {
                    let l = self.new_label();
                    self.e(&format!("lea {o}(a6),a0"));
                    self.e(&format!("move.l #{},d1", n / 4 - 1));
                    self.lab(&l);
                    self.e("clr.l (a0)+");
                    self.e(&format!("dbra d1,{l}"));
                    if n / 4 > 65536 {
                        self.e("sub.l #$10000,d1");
                        self.e(&format!("bpl {l}"));
                    }
                    for _ in 0..(n % 4) {
                        self.e("clr.b (a0)+");
                    }
                }
            }
        }
        for it in items {
            if zero && it.val.const_int() == Some(0) && it.bits.is_none() {
                continue;
            }
            let ea = base.at(it.off as i32);
            if let (Some(v), Ea::Reg(reg)) = (it.val.const_int(), &ea) {
                let v = v as i32;
                if reg.starts_with('d') && (-128..128).contains(&v) {
                    self.e(&format!("moveq #{v},{reg}"));
                } else {
                    self.e(&format!("move.l #{v},{reg}"));
                }
                continue;
            }
            if let (Some(v), Class::I, None) = (it.val.const_int(), it.ty.class(), it.bits) {
                if !matches!(ea, Ea::Reg(_)) {
                    let n = self.size(&it.ty);
                    if v == 0 {
                        self.e(&format!("clr.{} {}", suf(n), ea.s()));
                    } else if n == 4 {
                        self.e(&format!("move.l #{},{}", v as i32, ea.s()));
                    } else {
                        self.e(&format!("move.{} #{},{}", suf(n), v as i32, ea.s()));
                    }
                    continue;
                }
            }
            self.ex(&it.val);
            match it.bits {
                Some((off, w)) => self.e(&format!("bfins d0,{}{{{off}:{w}}}", ea.s())),
                None => self.store(&ea, &it.ty),
            }
        }
    }

    fn u_local_ty(&self, id: usize) -> Ty {
        self.cur_locals[id].ty.clone()
    }
}

impl<'u> Gen<'u> {
    /// Compile the whole unit to assembly text.
    pub fn unit(mut self, src_name: &str) -> Result<String, Vec<String>> {
        let _ = writeln!(self.out, "; generated by azcc from {src_name}");
        let u = self.u;
        for f in &u.functions {
            self.function(f);
        }
        // data
        for g in &u.globals {
            if let Some(text) = g.asm_name.strip_prefix('\u{1}') {
                self.out.push_str(text);
                self.out.push('\n');
                continue;
            }
            self.global(g);
        }
        // string literals
        if !self.u.strings.is_empty() {
            self.out.push_str("\tsection .rodata\n");
            for (i, (units, size)) in self.u.strings.iter().enumerate() {
                if *size > 1 {
                    self.out.push_str("\teven\n");
                }
                let _ = write!(self.out, "L{i}:");
                match size {
                    1 => {
                        let bytes: Vec<u8> = units.iter().map(|u| *u as u8).chain(core::iter::once(0)).collect();
                        let s = bytes_directive(&bytes);
                        self.out.push_str(&s);
                    }
                    2 => {
                        let v: Vec<String> = units.iter().chain(core::iter::once(&0)).map(|u| format!("{}", *u as u16)).collect();
                        let _ = writeln!(self.out, "\tdc.w {}", v.join(","));
                    }
                    _ => {
                        let v: Vec<String> = units.iter().chain(core::iter::once(&0)).map(|u| format!("{}", *u)).collect();
                        let _ = writeln!(self.out, "\tdc.l {}", v.join(","));
                    }
                }
            }
        }
        if !self.floats.is_empty() {
            self.out.push_str("\tsection .rodata\n\teven\n");
            for (i, (bits, single)) in self.floats.iter().enumerate() {
                if *single {
                    let _ = writeln!(self.out, "F{i}:\tdc.l ${:08X}", *bits as u32);
                } else {
                    let _ = writeln!(self.out, "F{i}:\tdc.l ${:08X},${:08X}", (*bits >> 32) as u32, *bits as u32);
                }
            }
        }
        if self.errors.is_empty() { Ok(self.out) } else { Err(self.errors) }
    }

    fn function(&mut self, f: &Function) {
        self.body.clear();
        self.saved.clear();
        self.frame = 0;
        self.case_labels.clear();
        self.breaks.clear();
        self.conts.clear();
        self.sret = f.sret;
        self.fname = f.name.clone();
        self.cur_locals = f.locals.clone();
        // register allocation: the most used word-sized locals whose address is not taken
        let mut cands: Vec<usize> = (0..f.locals.len())
            .filter(|&i| {
                let l = &f.locals[i];
                is_word(&l.ty) && !l.addr_taken && !l.volatile && l.uses > 0
            })
            .collect();
        cands.sort_by(|a, b| f.locals[*b].uses.cmp(&f.locals[*a].uses));
        let mut free_a: Vec<&'static str> = vec!["a3", "a4", "a5"];
        let mut free_d: Vec<&'static str> = vec!["d4", "d5", "d6", "d7"];
        let mut regs: BTreeMap<usize, &'static str> = BTreeMap::new();
        for i in cands {
            let ptr = f.locals[i].ty.is_ptr();
            let r = if ptr { if !free_a.is_empty() { Some(free_a.remove(0)) } else if !free_d.is_empty() { Some(free_d.remove(0)) } else { None } } else if !free_d.is_empty() { Some(free_d.remove(0)) } else if !free_a.is_empty() { Some(free_a.remove(0)) } else { None };
            match r {
                Some(r) => {
                    regs.insert(i, r);
                }
                None => break,
            }
        }
        self.locs.clear();
        let mut param_loads = Vec::new();
        for (i, l) in f.locals.iter().enumerate() {
            if let Some(r) = regs.get(&i) {
                self.use_reg(r);
                if let Some(po) = l.param_off {
                    param_loads.push(format!("move.l {po}(a6),{r}"));
                }
                self.locs.push(Loc::Reg(r));
                continue;
            }
            match l.param_off {
                Some(po) => self.locs.push(Loc::Frame(po)),
                None => {
                    let (s, a) = size_align(&l.ty, self.st());
                    let a = a.clamp(2, 4) as i32;
                    self.frame = ((self.frame + s as i32 + a - 1) / a) * a;
                    self.locs.push(Loc::Frame(-self.frame));
                }
            }
        }
        self.stmt(&f.body);
        // prologue and epilogue
        let frame = (self.frame + 3) & !3;
        let mut order: Vec<&'static str> = Vec::new();
        for r in ["d2", "d3", "d4", "d5", "d6", "d7", "a2", "a3", "a4", "a5"] {
            if self.saved.contains(&r) {
                order.push(r);
            }
        }
        let list = order.join("/");
        let mut out = String::new();
        out.push_str("\tsection .text\n\teven\n");
        if f.global {
            let _ = writeln!(out, "\txdef {}", f.asm_name);
        }
        let _ = writeln!(out, "{}:", f.asm_name);
        if frame <= 32767 {
            let _ = writeln!(out, "\tlink a6,#{}", -frame);
        } else {
            let _ = writeln!(out, "\tlink a6,#0\n\tsub.l #{frame},sp");
        }
        match order.len() {
            0 => {}
            1 => {
                let _ = writeln!(out, "\tmove.l {list},-(sp)");
            }
            _ => {
                let _ = writeln!(out, "\tmovem.l {list},-(sp)");
            }
        }
        for l in &param_loads {
            let _ = writeln!(out, "\t{l}");
        }
        let mut body = core::mem::take(&mut self.body);
        body.push_str(".Lret:\n");
        out.push_str(&peephole(&body));
        let save_off = frame + 4 * order.len() as i32;
        match order.len() {
            0 => {}
            1 => {
                let _ = writeln!(out, "\tmove.l {}(a6),{list}", -save_off);
            }
            _ => {
                let _ = writeln!(out, "\tmovem.l {}(a6),{list}", -save_off);
            }
        }
        out.push_str("\tunlk a6\n\trts\n");
        self.out.push_str(&out);
    }

    fn global(&mut self, g: &Global) {
        let (size, align) = size_align(&g.ty, self.st());
        let size = size.max(1);
        let sec = match (&g.init, g.readonly) {
            (None, _) => ".bss",
            (Some(_), true) => ".rodata",
            _ => ".data",
        };
        let _ = writeln!(self.out, "\tsection {sec}");
        if align >= 2 {
            let _ = writeln!(self.out, "\teven");
        }
        if align >= 4 {
            let _ = writeln!(self.out, "\tcnop 0,4");
        }
        if g.global {
            let _ = writeln!(self.out, "\txdef {}", g.asm_name);
        }
        let _ = write!(self.out, "{}:", g.asm_name);
        let Some(items) = &g.init else {
            let _ = writeln!(self.out, "\tds.b {size}");
            return;
        };
        // build the image, with relocations
        let mut img = vec![0u8; size as usize];
        let mut relocs: BTreeMap<u32, String> = BTreeMap::new();
        for it in items {
            let o = it.off as usize;
            match &it.val {
                Const::Int(v) => {
                    if let Some((boff, w)) = it.bits {
                        let mut unit = u32::from_be_bytes([img[o], img[o + 1], img[o + 2], img[o + 3]]);
                        let shift = 32 - boff as u32 - w as u32;
                        let mask = (if w == 32 { u32::MAX } else { (1u32 << w) - 1 }) << shift;
                        unit = (unit & !mask) | (((*v as u32) << shift) & mask);
                        img[o..o + 4].copy_from_slice(&unit.to_be_bytes());
                    } else if it.float {
                        let f = *v as f64;
                        if it.size == 4 {
                            img[o..o + 4].copy_from_slice(&(f as f32).to_bits().to_be_bytes());
                        } else {
                            img[o..o + 8].copy_from_slice(&f.to_bits().to_be_bytes());
                        }
                    } else {
                        let b = v.to_be_bytes();
                        let n = it.size as usize;
                        img[o..o + n].copy_from_slice(&b[8 - n..]);
                    }
                }
                Const::Flt(f) => {
                    if it.size == 4 {
                        img[o..o + 4].copy_from_slice(&(*f as f32).to_bits().to_be_bytes());
                    } else if it.size == 8 {
                        img[o..o + 8].copy_from_slice(&f.to_bits().to_be_bytes());
                    } else {
                        let b = (*f as i64).to_be_bytes();
                        let n = it.size as usize;
                        img[o..o + n].copy_from_slice(&b[8 - n..]);
                    }
                }
                Const::Addr(s, off) => {
                    relocs.insert(it.off, if *off == 0 { s.clone() } else if *off > 0 { format!("{s}+{off}") } else { format!("{s}{off}") });
                }
                Const::Str(n, off) => {
                    relocs.insert(it.off, if *off == 0 { format!("L{n}") } else if *off > 0 { format!("L{n}+{off}") } else { format!("L{n}{off}") });
                }
            }
        }
        let mut p = 0u32;
        let mut first = true;
        while p < size {
            if let Some(r) = relocs.get(&p) {
                let _ = writeln!(self.out, "\tdc.l {r}");
                p += 4;
                first = false;
                continue;
            }
            let next = relocs.range(p..).next().map_or(size, |(k, _)| *k);
            let chunk = &img[p as usize..next as usize];
            // runs of zeros as ds.b
            let mut i = 0;
            while i < chunk.len() {
                let z = chunk[i..].iter().take_while(|b| **b == 0).count();
                if z >= 16 {
                    let _ = writeln!(self.out, "\tds.b {z}");
                    i += z;
                    continue;
                }
                let end = (i + 16).min(chunk.len());
                let v: Vec<String> = chunk[i..end].iter().map(|b| format!("{b}")).collect();
                let _ = writeln!(self.out, "\tdc.b {}", v.join(","));
                i = end;
            }
            p = next;
            first = false;
        }
        if first {
            self.out.push('\n');
        }
    }
}

/// `dc.b` lines for a byte string, quoting printable runs.
fn bytes_directive(b: &[u8]) -> String {
    let mut out = String::new();
    let mut parts: Vec<String> = Vec::new();
    let mut run = String::new();
    let flush = |parts: &mut Vec<String>, run: &mut String| {
        if !run.is_empty() {
            parts.push(format!("\"{run}\""));
            run.clear();
        }
    };
    for &c in b {
        if (0x20..0x7f).contains(&c) && c != b'"' && c != b'\\' && c != b';' {
            run.push(c as char);
        } else {
            flush(&mut parts, &mut run);
            parts.push(format!("{c}"));
        }
        if parts.len() >= 12 {
            flush(&mut parts, &mut run);
            let _ = writeln!(out, "\tdc.b {}", parts.join(","));
            parts.clear();
        }
    }
    flush(&mut parts, &mut run);
    if !parts.is_empty() {
        let _ = writeln!(out, "\tdc.b {}", parts.join(","));
    }
    out
}

/// Simple clean-ups on a function body.
fn peephole(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    let mut dead = false;
    while i < lines.len() {
        let l = lines[i];
        let is_label = !l.starts_with('\t');
        if is_label {
            dead = false;
        } else if dead && !l.trim_start().starts_with("dc.") && !l.trim_start().starts_with("even") {
            i += 1;
            continue;
        }
        let t = l.trim();
        let next = lines.get(i + 1).map(|s| s.trim()).unwrap_or("");
        if t == "move.l d0,-(sp)" && next == "move.l (sp)+,d0" {
            i += 2;
            continue;
        }
        if t == "move.l d0,-(sp)" && next == "move.l (sp)+,d1" {
            out.push(String::from("\tmove.l d0,d1"));
            i += 2;
            continue;
        }
        if let Some(target) = t.strip_prefix("bra ") {
            // a branch to the next label
            let mut j = i + 1;
            let mut hit = false;
            while j < lines.len() && !lines[j].starts_with('\t') {
                if lines[j].trim_end_matches(':') == target {
                    hit = true;
                }
                j += 1;
            }
            if hit {
                i += 1;
                continue;
            }
            out.push(String::from(l));
            dead = true;
            i += 1;
            continue;
        }
        if t == "jmp (a0)" || t == "rts" {
            dead = true;
        }
        out.push(String::from(l));
        i += 1;
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
