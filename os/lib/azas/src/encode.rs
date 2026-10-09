//! Instruction encoding for the 68000-68030 integer set, the 68881/68882 FPU and the
//! 68030 PMMU.

use crate::expr::{Base, Val};
use crate::float;
use crate::operand::{BaseReg, BfArg, Disp, Index, Op, Special, Sz};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixKind {
    Abs8,
    Abs16,
    Abs32,
    Pc8,
    Pc16,
    Pc32,
}

/// A value to patch into the instruction once addresses are final.
#[derive(Clone, Copy, Debug)]
pub struct Fixup {
    /// Byte offset of the field within the instruction.
    pub at: usize,
    pub kind: FixKind,
    pub val: Val,
    /// For PC-relative kinds: offset within the instruction of the PC the displacement
    /// is relative to.
    pub pcbase: usize,
}

pub struct Enc {
    /// Address of the instruction (Abs in binary output, Sec in object output).
    pub addr: Val,
    pub b: Vec<u8>,
    pub fix: Vec<Fixup>,
    /// A value that did not fit. The instruction is still emitted at its full size so
    /// that layout stays stable across passes; the error is reported afterwards.
    pub err: Option<String>,
    /// Size of this instruction in the previous pass (labels after it were placed
    /// with that size).
    pub prev_size: Option<usize>,
}

type R<T = ()> = Result<T, String>;

const EA_DN: u16 = 1 << 0;
const EA_AN: u16 = 1 << 1;
const EA_IND: u16 = 1 << 2;
const EA_POST: u16 = 1 << 3;
const EA_PRE: u16 = 1 << 4;
const EA_D16: u16 = 1 << 5;
const EA_IDX: u16 = 1 << 6;
const EA_ABS: u16 = 1 << 7;
const EA_PC: u16 = 1 << 8;
const EA_IMM: u16 = 1 << 9;

const ALL: u16 = 0x3FF;
const DATA: u16 = ALL & !EA_AN;
const MEMORY: u16 = ALL & !EA_DN & !EA_AN;
const CONTROL: u16 = EA_IND | EA_D16 | EA_IDX | EA_ABS | EA_PC;
const ALTER: u16 = EA_DN | EA_AN | EA_IND | EA_POST | EA_PRE | EA_D16 | EA_IDX | EA_ABS;
const DATA_ALT: u16 = ALTER & !EA_AN;
const MEM_ALT: u16 = ALTER & !EA_DN & !EA_AN;
const CTRL_ALT: u16 = CONTROL & !EA_PC;

fn ea_class(op: &Op) -> u16 {
    match op {
        Op::D(_) => EA_DN,
        Op::A(_) => EA_AN,
        Op::Ind(_) => EA_IND,
        Op::Post(_) => EA_POST,
        Op::Pre(_) => EA_PRE,
        Op::Disp { base: BaseReg::Pc, .. } => EA_PC,
        Op::Disp { .. } => EA_D16,
        Op::Idx { base: BaseReg::Pc | BaseReg::Zpc, .. } => EA_PC,
        Op::Idx { .. } => EA_IDX,
        Op::Abs { .. } => EA_ABS,
        Op::Imm(_) | Op::ImmF(_) => EA_IMM,
        _ => 0,
    }
}

fn fits8(v: i64) -> bool {
    (-128..=127).contains(&v)
}
fn fits16(v: i64) -> bool {
    (-32768..=32767).contains(&v)
}

/// Condition-code suffixes (Bcc/DBcc/Scc/TRAPcc).
pub fn cond(s: &str) -> Option<u16> {
    Some(match s {
        "t" | "ra" => 0,
        "f" | "sr" => 1,
        "hi" => 2,
        "ls" => 3,
        "cc" | "hs" => 4,
        "cs" | "lo" => 5,
        "ne" => 6,
        "eq" => 7,
        "vc" => 8,
        "vs" => 9,
        "pl" => 10,
        "mi" => 11,
        "ge" => 12,
        "lt" => 13,
        "gt" => 14,
        "le" => 15,
        _ => return None,
    })
}

fn fcond(s: &str) -> Option<u16> {
    const T: [&str; 32] = [
        "f", "eq", "ogt", "oge", "olt", "ole", "ogl", "or", "un", "ueq", "ugt", "uge", "ult", "ule", "ne", "t",
        "sf", "seq", "gt", "ge", "lt", "le", "gl", "gle", "ngle", "ngl", "nle", "nlt", "nge", "ngt", "sne", "st",
    ];
    T.iter().position(|&c| c == s).map(|i| i as u16)
}

fn fpu_fmt(sz: Sz) -> u16 {
    match sz {
        Sz::L => 0,
        Sz::S => 1,
        Sz::X => 2,
        Sz::P => 3,
        Sz::W => 4,
        Sz::D => 5,
        Sz::B => 6,
    }
}

/// FPU dyadic/monadic opmodes.
fn fpu_op(m: &str) -> Option<u16> {
    Some(match m {
        "fmove" => 0x00,
        "fint" => 0x01,
        "fsinh" => 0x02,
        "fintrz" => 0x03,
        "fsqrt" => 0x04,
        "flognp1" => 0x06,
        "fetoxm1" => 0x08,
        "ftanh" => 0x09,
        "fatan" => 0x0A,
        "fasin" => 0x0C,
        "fatanh" => 0x0D,
        "fsin" => 0x0E,
        "ftan" => 0x0F,
        "fetox" => 0x10,
        "ftwotox" => 0x11,
        "ftentox" => 0x12,
        "flogn" => 0x14,
        "flog10" => 0x15,
        "flog2" => 0x16,
        "fabs" => 0x18,
        "fcosh" => 0x19,
        "fneg" => 0x1A,
        "facos" => 0x1C,
        "fcos" => 0x1D,
        "fgetexp" => 0x1E,
        "fgetman" => 0x1F,
        "fdiv" => 0x20,
        "fmod" => 0x21,
        "fadd" => 0x22,
        "fmul" => 0x23,
        "fsgldiv" => 0x24,
        "frem" => 0x25,
        "fscale" => 0x26,
        "fsglmul" => 0x27,
        "fsub" => 0x28,
        "fcmp" => 0x38,
        _ => return None,
    })
}

fn monadic(opm: u16) -> bool {
    opm < 0x20 && opm != 0
}

fn bad(m: &str) -> String {
    format!("invalid operands for `{m}`")
}

impl Enc {
    pub fn new(addr: Val) -> Self {
        Enc { addr, b: Vec::new(), fix: Vec::new(), err: None, prev_size: None }
    }

    fn w(&mut self, v: u16) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    fn l(&mut self, v: u32) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    fn patch(&mut self, at: usize, v: u16) {
        let old = u16::from_be_bytes([self.b[at], self.b[at + 1]]);
        self.b[at..at + 2].copy_from_slice(&(old | v).to_be_bytes());
    }

    /// Displacement from the PC at instruction offset `pcoff` to `target`, if it is
    /// computable now.
    fn pcdisp(&self, target: Val, pcoff: usize) -> Option<i64> {
        if target.base == self.addr.base && target.base != Base::Unknown {
            Some(target.off - (self.addr.off + pcoff as i64))
        } else {
            None
        }
    }

    /// Append a value field of `kind`, recording a fixup when it is not a constant.
    fn field(&mut self, kind: FixKind, v: Val, pcbase: usize) -> R {
        let at = self.b.len();
        let n = match kind {
            FixKind::Abs8 | FixKind::Pc8 => 1,
            FixKind::Abs16 | FixKind::Pc16 => 2,
            FixKind::Abs32 | FixKind::Pc32 => 4,
        };
        self.b.resize(at + n, 0);
        let pcrel = matches!(kind, FixKind::Pc8 | FixKind::Pc16 | FixKind::Pc32);
        let known = if pcrel { self.pcdisp(v, pcbase) } else if v.is_abs() { Some(v.off) } else { None };
        match known {
            Some(x) => {
                if let Err(e) = self.put(at, kind, x) {
                    self.err.get_or_insert(e);
                }
                Ok(())
            }
            None => {
                self.fix.push(Fixup { at, kind, val: v, pcbase });
                Ok(())
            }
        }
    }

    fn put(&mut self, at: usize, kind: FixKind, x: i64) -> R {
        put_field(&mut self.b, at, kind, x)
    }

    /// Immediate data of the given size.
    fn imm(&mut self, op: &Op, sz: Sz) -> R {
        match (op, sz) {
            (Op::Imm(v), Sz::B) => {
                if v.is_abs() && (-128..0).contains(&v.off) {
                    // sign-extend negative bytes into the word, like GNU as
                    self.w(v.off as i16 as u16);
                    return Ok(());
                }
                self.b.push(0);
                self.field(FixKind::Abs8, *v, 0)
            }
            (Op::Imm(v), Sz::W) => self.field(FixKind::Abs16, *v, 0),
            (Op::Imm(v), Sz::L) => self.field(FixKind::Abs32, *v, 0),
            (Op::Imm(v), Sz::S | Sz::D | Sz::X) => {
                if !v.is_abs() && !v.is_unknown() {
                    return Err("floating-point immediate must be a constant".into());
                }
                self.float(v.off as f64, sz)
            }
            (Op::ImmF(f), Sz::S | Sz::D | Sz::X) => self.float(*f, sz),
            (Op::ImmF(_), _) => Err("floating-point immediate needs .s, .d or .x".into()),
            (_, Sz::P) => Err("packed decimal immediates are not supported".into()),
            _ => Err("expected an immediate".into()),
        }
    }

    fn float(&mut self, f: f64, sz: Sz) -> R {
        match sz {
            Sz::S => self.l((f as f32).to_bits()),
            Sz::D => self.b.extend_from_slice(&f.to_bits().to_be_bytes()),
            _ => self.b.extend_from_slice(&float::to_extended(f)),
        }
        Ok(())
    }

    /// Encode an effective address. Extension words are appended; returns the 6-bit
    /// mode/register field.
    fn ea(&mut self, op: &Op, sz: Sz, allowed: u16) -> R<u16> {
        let class = ea_class(op);
        if class == 0 || class & allowed == 0 {
            return Err("addressing mode not allowed here".into());
        }
        Ok(match op {
            Op::D(r) => *r as u16,
            Op::A(r) => 8 | *r as u16,
            Op::Ind(r) => 0x10 | *r as u16,
            Op::Post(r) => 0x18 | *r as u16,
            Op::Pre(r) => 0x20 | *r as u16,
            Op::Disp { base: BaseReg::An(r), d } => {
                let fits = d.v.is_abs() && fits16(d.v.off);
                if d.sz == Some(Sz::W) || fits {
                    self.field(FixKind::Abs16, d.v, 0)?;
                    0x28 | *r as u16
                } else {
                    self.full(BaseReg::An(*r), Some(*d), None, None)?
                }
            }
            Op::Disp { base: BaseReg::Pc, d } if d.v.is_abs() && (d.sz == Some(Sz::W) || d.sz.is_none() && fits16(d.v.off)) => {
                // a number is the displacement itself; a label is the target
                self.field(FixKind::Abs16, d.v, 0)?;
                0x3A
            }
            Op::Disp { base: BaseReg::Pc, d } => {
                let pc = self.b.len();
                let disp = self.pcdisp(d.v, pc);
                if d.sz == Some(Sz::W) || disp.is_some_and(fits16) {
                    self.field(FixKind::Pc16, d.v, pc)?;
                    0x3A
                } else {
                    self.full(BaseReg::Pc, Some(*d), None, None)?
                }
            }
            Op::Disp { .. } => unreachable!(),
            Op::Idx { base, bd, ix, mem } => {
                // brief format when possible
                if let (Some(x), None, BaseReg::An(_) | BaseReg::Pc) = (ix, mem, base) {
                    let pc = self.b.len();
                    let d8 = match bd {
                        None => Some(0),
                        Some(Disp { sz: Some(_), .. }) => None,
                        Some(d) if *base == BaseReg::Pc && !d.v.is_abs() => self.pcdisp(d.v, pc).filter(|&v| fits8(v)),
                        Some(d) => Some(d.v.off).filter(|&v| d.v.is_abs() && fits8(v)),
                    };
                    if let Some(d8) = d8 {
                        let ext = ((x.reg as u16) << 12) | ((x.long as u16) << 11) | ((x.scale as u16) << 9) | (d8 as u8 as u16);
                        self.w(ext);
                        return Ok(match base {
                            BaseReg::An(r) => 0x30 | *r as u16,
                            _ => 0x3B,
                        });
                    }
                }
                self.full(*base, *bd, *ix, *mem)?
            }
            Op::Abs { v, sz: None } if allowed & EA_PC != 0 && self.pcdisp(*v, self.b.len()).is_some_and(fits16) => {
                // label in this section: use (d16,PC) like vasm does
                let pc = self.b.len();
                self.field(FixKind::Pc16, *v, pc)?;
                0x3A
            }
            Op::Abs { v, sz: asz } => {
                let short = match asz {
                    Some(Sz::W) => true,
                    Some(_) => false,
                    None => v.is_abs() && fits16(v.off as i32 as i64) && fits16(v.off),
                };
                if short {
                    let x = v.off as i32 as i64;
                    if v.is_abs() && !fits16(x) {
                        return Err(format!("address {:#x} does not fit in a word", v.off));
                    }
                    self.field(FixKind::Abs16, if v.is_abs() { Val::abs(x) } else { *v }, 0)?;
                    0x38
                } else {
                    self.field(FixKind::Abs32, *v, 0)?;
                    0x39
                }
            }
            Op::Imm(_) | Op::ImmF(_) => {
                self.imm(op, sz)?;
                0x3C
            }
            _ => return Err("bad addressing mode".into()),
        })
    }

    /// 68020 full-format extension.
    fn full(&mut self, base: BaseReg, bd: Option<Disp>, ix: Option<Index>, mem: Option<(bool, Option<Disp>)>) -> R<u16> {
        let ext_at = self.b.len();
        self.w(0);
        let mut ext: u16 = 0x0100;
        let mode = match base {
            BaseReg::An(r) => 0x30 | r as u16,
            BaseReg::Pc => 0x3B,
            BaseReg::Zpc => {
                ext |= 0x80;
                0x3B
            }
            BaseReg::None => {
                ext |= 0x80;
                0x30
            }
        };
        match ix {
            Some(x) => ext |= ((x.reg as u16) << 12) | ((x.long as u16) << 11) | ((x.scale as u16) << 9),
            None => ext |= 0x40,
        }
        // base displacement (relative to the extension word for PC bases, unless it is
        // a plain number, which is the displacement itself)
        let pcrel = base == BaseReg::Pc && bd.is_some_and(|d| !d.v.is_abs());
        match bd {
            None => ext |= 0x10,
            Some(d) => {
                let word = match d.sz {
                    Some(Sz::W) => true,
                    Some(_) => false,
                    None => {
                        if pcrel {
                            self.pcdisp(d.v, ext_at).is_some_and(fits16)
                        } else {
                            d.v.is_abs() && fits16(d.v.off)
                        }
                    }
                };
                if word {
                    ext |= 0x20;
                    if pcrel {
                        self.field(FixKind::Pc16, d.v, ext_at)?;
                    } else {
                        self.field(FixKind::Abs16, d.v, 0)?;
                    }
                } else {
                    ext |= 0x30;
                    if pcrel {
                        self.field(FixKind::Pc32, d.v, ext_at)?;
                    } else {
                        self.field(FixKind::Abs32, d.v, 0)?;
                    }
                }
            }
        }
        // memory indirection
        if let Some((post, od)) = mem {
            let iis: u16 = match od {
                None => 1,
                Some(d) => {
                    let word = match d.sz {
                        Some(Sz::W) => true,
                        Some(_) => false,
                        None => d.v.is_abs() && fits16(d.v.off),
                    };
                    if word {
                        self.field(FixKind::Abs16, d.v, 0)?;
                        2
                    } else {
                        self.field(FixKind::Abs32, d.v, 0)?;
                        3
                    }
                }
            };
            if post {
                if ix.is_none() {
                    return Err("postindexed mode needs an index register".into());
                }
                ext |= 4 | iis;
            } else {
                ext |= iis;
            }
        }
        self.patch(ext_at, ext);
        Ok(mode)
    }

    fn branch(&mut self, opcode: u16, target: &Op, sz: Option<Sz>) -> R {
        let v = match target {
            Op::Abs { v, .. } => *v,
            _ => return Err("branch target must be a label or address".into()),
        };
        let disp = self.pcdisp(v, 2);
        // A forward target moves when this branch changes size: its address was
        // computed with last pass's size of this instruction.
        let disp_as = |bytes: i64| -> Option<i64> {
            disp.map(|d| match self.prev_size {
                Some(p) if d > 0 => d + bytes - p as i64,
                _ => d,
            })
        };
        let size = match sz {
            Some(Sz::B | Sz::S) => Sz::B,
            Some(Sz::W) => Sz::W,
            Some(Sz::L) => Sz::L,
            Some(_) => return Err("bad branch size".into()),
            None => match (disp_as(2), disp_as(4)) {
                (Some(d), _) if fits8(d) && d != 0 && d != -1 => Sz::B,
                (_, Some(d)) if fits16(d) => Sz::W,
                _ => Sz::L,
            },
        };
        let disp = match size {
            Sz::B => disp_as(2),
            Sz::W => disp_as(4),
            _ => disp_as(6),
        };
        match size {
            Sz::B => {
                match disp {
                    Some(d) if d != 0 && d != -1 && fits8(d) => self.w(opcode | (d as u8 as u16)),
                    Some(d) => {
                        self.w(opcode | 1);
                        self.err.get_or_insert(format!("short branch displacement {d} out of range"));
                    }
                    None if v.is_unknown() => self.w(opcode | 1), // resolved in a later pass
                    None => return Err("short branch to an external symbol".into()),
                }
            }
            Sz::W => {
                self.w(opcode);
                self.field(FixKind::Pc16, v, 2)?;
            }
            _ => {
                self.w(opcode | 0xFF);
                self.field(FixKind::Pc32, v, 2)?;
            }
        }
        Ok(())
    }
}

/// Store `x` into a field of `kind` at `b[at..]`, checking its range.
pub fn put_field(b: &mut [u8], at: usize, kind: FixKind, x: i64) -> R {
    match kind {
        FixKind::Abs8 => {
            if !(-128..=255).contains(&x) {
                return Err(format!("value {x} does not fit in a byte"));
            }
            b[at] = x as u8;
        }
        FixKind::Pc8 => {
            if !fits8(x) {
                return Err(format!("displacement {x} does not fit in 8 bits"));
            }
            b[at] = x as u8;
        }
        FixKind::Abs16 => {
            if !(-32768..=65535).contains(&x) {
                return Err(format!("value {x} does not fit in a word"));
            }
            b[at..at + 2].copy_from_slice(&(x as u16).to_be_bytes());
        }
        FixKind::Pc16 => {
            if !fits16(x) {
                return Err(format!("displacement {x} does not fit in 16 bits"));
            }
            b[at..at + 2].copy_from_slice(&(x as u16).to_be_bytes());
        }
        FixKind::Abs32 | FixKind::Pc32 => {
            if !(-(1i64 << 31)..(1i64 << 32)).contains(&x) {
                return Err(format!("value {x} does not fit in 32 bits"));
            }
            b[at..at + 4].copy_from_slice(&(x as u32).to_be_bytes());
        }
    }
    Ok(())
}

fn std_size(sz: Sz) -> R<u16> {
    match sz {
        Sz::B => Ok(0),
        Sz::W => Ok(1),
        Sz::L => Ok(2),
        _ => Err("bad size".into()),
    }
}

fn dreg(op: &Op) -> Option<u16> {
    if let Op::D(r) = op { Some(*r as u16) } else { None }
}
fn areg(op: &Op) -> Option<u16> {
    if let Op::A(r) = op { Some(*r as u16) } else { None }
}
fn gp(op: &Op) -> Option<u16> {
    match op {
        Op::D(r) => Some(*r as u16),
        Op::A(r) => Some(8 + *r as u16),
        _ => None,
    }
}

/// Immediate that must be known now (quick values, shift counts, trap numbers...).
fn quick(op: &Op, lo: i64, hi: i64, what: &str) -> R<i64> {
    match op {
        Op::Imm(v) if v.is_unknown() => Ok(lo.max(1).min(hi)),
        Op::Imm(v) if v.is_abs() => {
            if (lo..=hi).contains(&v.off) {
                Ok(v.off)
            } else {
                Err(format!("{what} {} out of range {lo}..{hi}", v.off))
            }
        }
        _ => Err(format!("{what} must be an immediate constant")),
    }
}

/// Function code operand for PMMU instructions.
fn mmu_fc(op: &Op) -> R<u16> {
    match op {
        Op::Spec(Special::Sfc) => Ok(0),
        Op::Spec(Special::Dfc) => Ok(1),
        Op::D(r) => Ok(0x08 | *r as u16),
        Op::Imm(_) => Ok(0x10 | quick(op, 0, 7, "function code")? as u16),
        _ => Err("bad function code".into()),
    }
}

/// Encode one instruction. `m` is the lower-case mnemonic without size suffix.
pub fn encode(e: &mut Enc, m: &str, sz: Option<Sz>, ops: &[Op]) -> R {
    let n = ops.len();
    let o = |i: usize| -> &Op { &ops[i] };
    let szd = |d: Sz| sz.unwrap_or(d);

    // ---- no-operand instructions -------------------------------------------------
    let simple = match m {
        "nop" => Some(0x4E71),
        "rts" => Some(0x4E75),
        "rte" => Some(0x4E73),
        "rtr" => Some(0x4E77),
        "reset" => Some(0x4E70),
        "trapv" => Some(0x4E76),
        "illegal" => Some(0x4AFC),
        _ => None,
    };
    if let Some(op) = simple {
        if n != 0 {
            return Err(bad(m));
        }
        e.w(op);
        return Ok(());
    }

    // ---- branches ---------------------------------------------------------------------
    if m == "bsr" || m == "jbsr" {
        return e.branch(0x6100, ops.first().ok_or_else(|| bad(m))?, sz);
    }
    if let Some(c) = m.strip_prefix('b').and_then(cond).filter(|_| m != "bsr") {
        if n != 1 {
            return Err(bad(m));
        }
        return e.branch(0x6000 | c << 8, o(0), sz);
    }
    if let Some(c) = m.strip_prefix("jb").and_then(cond) {
        return e.branch(0x6000 | c << 8, ops.first().ok_or_else(|| bad(m))?, sz);
    }
    if m == "dbra" || m.starts_with("db") && cond(&m[2..]).is_some() {
        let c = if m == "dbra" { 1 } else { cond(&m[2..]).unwrap() };
        let (Some(d), Some(Op::Abs { v, .. })) = (ops.first().and_then(dreg), ops.get(1)) else { return Err(bad(m)) };
        e.w(0x50C8 | c << 8 | d);
        return e.field(FixKind::Pc16, *v, 2);
    }
    if let Some(c) = m.strip_prefix("trap").and_then(cond) {
        match (n, sz) {
            (0, None) => e.w(0x50FC | c << 8),
            (1, Some(Sz::W)) => {
                e.w(0x50FA | c << 8);
                e.imm(o(0), Sz::W)?;
            }
            (1, Some(Sz::L)) => {
                e.w(0x50FB | c << 8);
                e.imm(o(0), Sz::L)?;
            }
            _ => return Err(bad(m)),
        }
        return Ok(());
    }
    if m.starts_with('s') && m.len() <= 3 && cond(&m[1..]).is_some() && m != "sr" {
        let c = cond(&m[1..]).unwrap();
        if n != 1 {
            return Err(bad(m));
        }
        e.w(0x50C0 | c << 8);
        let ea = e.ea(o(0), Sz::B, DATA_ALT)?;
        e.patch(0, ea);
        return Ok(());
    }

    // ---- FPU / PMMU -------------------------------------------------------------------
    if m.starts_with('f') {
        if let Some(r) = fpu(e, m, sz, ops)? {
            return Ok(r);
        }
    }
    if m.starts_with('p') {
        if let Some(r) = pmmu(e, m, sz, ops)? {
            return Ok(r);
        }
    }

    match m {
        // ---- arithmetic / logic ------------------------------------------------------
        "add" | "sub" | "and" | "or" | "cmp" | "eor" | "adda" | "suba" | "cmpa" | "addi" | "subi" | "andi"
        | "ori" | "eori" | "cmpi" | "cmpm" => {
            if n != 2 {
                return Err(bad(m));
            }
            let base = &m[..m.len() - (m.ends_with('i') || m.ends_with('a')) as usize];
            let base = if m == "eori" || m == "ori" || m == "andi" || m == "cmpm" { &m[..m.len() - 1] } else { base };
            let (s, d) = (o(0), o(1));
            // to CCR / SR
            if let Op::Spec(sp @ (Special::Ccr | Special::Sr)) = d {
                let opc = match base {
                    "or" => 0x003C,
                    "and" => 0x023C,
                    "eor" => 0x0A3C,
                    _ => return Err(bad(m)),
                } | if *sp == Special::Sr { 0x40 } else { 0 };
                e.w(opc);
                return e.imm(s, if *sp == Special::Sr { Sz::W } else { Sz::B });
            }
            // address register destination
            if let (Some(a), "add" | "sub" | "cmp") = (areg(d), base) {
                let mut s_ = szd(Sz::W);
                // vasm-style safe optimisations for constant immediates
                if let Op::Imm(v) = s {
                    if v.is_abs() && s_ == Sz::L {
                        if base != "cmp" && (1..=8).contains(&v.off) {
                            e.w(0x5088 | ((v.off as u16) & 7) << 9 | if base == "sub" { 0x100 } else { 0 } | a);
                            return Ok(());
                        }
                        if fits16(v.off) {
                            s_ = Sz::W;
                        }
                    }
                }
                let opm = match s_ {
                    Sz::W => 3,
                    Sz::L => 7,
                    _ => return Err("address register operations are .w or .l".into()),
                };
                let opc = match base {
                    "add" => 0xD000,
                    "sub" => 0x9000,
                    _ => 0xB000,
                };
                e.w(opc | a << 9 | opm << 6);
                let ea = e.ea(s, s_, ALL)?;
                e.patch(0, ea);
                return Ok(());
            }
            let s_ = szd(Sz::W);
            let ss = std_size(s_)?;
            // immediate source: `op #x,Dn` uses the <ea>,Dn form (as vasm does), anything
            // else the immediate instruction
            if let (Op::Imm(_), Some(dn), false) = (s, dreg(d), m.ends_with('i') || base == "eor") {
                let opc = match base {
                    "or" => 0x8000,
                    "sub" => 0x9000,
                    "cmp" => 0xB000,
                    "and" => 0xC000,
                    _ => 0xD000,
                };
                e.w(opc | dn << 9 | ss << 6);
                let ea = e.ea(s, s_, ALL)?;
                e.patch(0, ea);
                return Ok(());
            }
            if let Op::Imm(_) = s {
                let opc = match base {
                    "or" => 0x0000,
                    "and" => 0x0200,
                    "sub" => 0x0400,
                    "add" => 0x0600,
                    "eor" => 0x0A00,
                    "cmp" => 0x0C00,
                    _ => return Err(bad(m)),
                };
                e.w(opc | ss << 6);
                e.imm(s, s_)?;
                let allowed = if base == "cmp" { DATA & !EA_IMM } else { DATA_ALT };
                let ea = e.ea(d, s_, allowed)?;
                e.patch(0, ea);
                return Ok(());
            }
            if m.ends_with('i') {
                return Err(format!("`{m}` needs an immediate source"));
            }
            let opc = match base {
                "or" => 0x8000,
                "sub" => 0x9000,
                "cmp" | "eor" => 0xB000,
                "and" => 0xC000,
                "add" => 0xD000,
                _ => return Err(bad(m)),
            };
            if base == "cmp" || m == "cmpm" {
                if let (Op::Post(y), Op::Post(x)) = (s, d) {
                    e.w(0xB108 | (*x as u16) << 9 | ss << 6 | *y as u16);
                    return Ok(());
                }
            }
            if let (Some(dn), false) = (dreg(d), base == "eor") {
                // <ea>,Dn
                if s_ == Sz::B && matches!(s, Op::A(_)) {
                    return Err("byte operation on an address register".into());
                }
                e.w(opc | dn << 9 | ss << 6);
                let allowed = if base == "and" || base == "or" { DATA } else { ALL };
                let ea = e.ea(s, s_, allowed)?;
                e.patch(0, ea);
                return Ok(());
            }
            if let Some(dn) = dreg(s) {
                if base == "cmp" {
                    return Err(bad(m));
                }
                // Dn,<ea>
                e.w(opc | dn << 9 | (4 | ss) << 6);
                let allowed = if base == "eor" { DATA_ALT } else { MEM_ALT };
                let ea = e.ea(d, s_, allowed)?;
                e.patch(0, ea);
                return Ok(());
            }
            Err(bad(m))
        }
        "addq" | "subq" => {
            if n != 2 {
                return Err(bad(m));
            }
            let q = quick(o(0), 1, 8, "quick value")? as u16 & 7;
            let s_ = szd(Sz::W);
            if s_ == Sz::B && matches!(o(1), Op::A(_)) {
                return Err("byte operation on an address register".into());
            }
            e.w(0x5000 | q << 9 | if m == "subq" { 0x100 } else { 0 } | std_size(s_)? << 6);
            let ea = e.ea(o(1), s_, ALTER)?;
            e.patch(0, ea);
            Ok(())
        }
        "addx" | "subx" | "abcd" | "sbcd" => {
            let base: u16 = match m {
                "addx" => 0xD100,
                "subx" => 0x9100,
                "abcd" => 0xC100,
                _ => 0x8100,
            };
            let ss = if m.ends_with('x') { std_size(szd(Sz::W))? << 6 } else { 0 };
            match (ops.first(), ops.get(1)) {
                (Some(Op::D(y)), Some(Op::D(x))) => e.w(base | (*x as u16) << 9 | ss | *y as u16),
                (Some(Op::Pre(y)), Some(Op::Pre(x))) => e.w(base | (*x as u16) << 9 | ss | 8 | *y as u16),
                _ => return Err(bad(m)),
            }
            Ok(())
        }
        "pack" | "unpk" => {
            let base: u16 = if m == "pack" { 0x8140 } else { 0x8180 };
            if n != 3 {
                return Err(bad(m));
            }
            match (o(0), o(1)) {
                (Op::D(x), Op::D(y)) => e.w(base | (*y as u16) << 9 | *x as u16),
                (Op::Pre(x), Op::Pre(y)) => e.w(base | (*y as u16) << 9 | 8 | *x as u16),
                _ => return Err(bad(m)),
            }
            e.imm(o(2), Sz::W)
        }
        "mulu" | "muls" | "divu" | "divs" | "divul" | "divsl" => {
            let signed = m.ends_with('s') || m == "divsl";
            let s_ = if m.ends_with('l') { Sz::L } else { szd(Sz::W) };
            if n != 2 {
                return Err(bad(m));
            }
            if s_ == Sz::W {
                let dn = dreg(o(1)).ok_or_else(|| bad(m))?;
                let opc = match (m, signed) {
                    ("mulu", _) => 0xC0C0,
                    ("muls", _) => 0xC1C0,
                    ("divu", _) => 0x80C0,
                    _ => 0x81C0,
                };
                e.w(opc | dn << 9);
                let ea = e.ea(o(0), Sz::W, DATA)?;
                e.patch(0, ea);
                return Ok(());
            }
            let mul = m.starts_with("mul");
            let (lo, hi, wide) = match o(1) {
                Op::D(r) => (*r as u16, *r as u16, false),
                Op::Pair(h, l) if *h < 8 && *l < 8 => (*l as u16, *h as u16, !m.ends_with('l')),
                _ => return Err(bad(m)),
            };
            e.w(if mul { 0x4C00 } else { 0x4C40 });
            e.w(lo << 12 | (signed as u16) << 11 | (wide as u16) << 10 | if mul && !wide { 0 } else { hi });
            let ea = e.ea(o(0), Sz::L, DATA)?;
            e.patch(0, ea);
            Ok(())
        }
        "neg" | "negx" | "not" | "clr" | "tst" | "nbcd" | "tas" => {
            if n != 1 {
                return Err(bad(m));
            }
            let (opc, s_, allowed) = match m {
                "negx" => (0x4000, szd(Sz::W), DATA_ALT),
                "clr" => (0x4200, szd(Sz::W), DATA_ALT),
                "neg" => (0x4400, szd(Sz::W), DATA_ALT),
                "not" => (0x4600, szd(Sz::W), DATA_ALT),
                "tst" => (0x4A00, szd(Sz::W), ALL),
                "nbcd" => (0x4800, Sz::B, DATA_ALT),
                _ => (0x4AC0, Sz::B, DATA_ALT),
            };
            let ss = if m == "nbcd" || m == "tas" { 0 } else { std_size(s_)? << 6 };
            if m == "tst" && s_ == Sz::B && matches!(o(0), Op::A(_)) {
                return Err("tst.b on an address register".into());
            }
            e.w(opc | ss);
            let ea = e.ea(o(0), s_, allowed)?;
            e.patch(0, ea);
            Ok(())
        }
        "swap" => {
            e.w(0x4840 | ops.first().and_then(dreg).ok_or_else(|| bad(m))?);
            Ok(())
        }
        "ext" | "extb" => {
            let d = ops.first().and_then(dreg).ok_or_else(|| bad(m))?;
            let opc = match (m, szd(if m == "extb" { Sz::L } else { Sz::W })) {
                ("ext", Sz::W) => 0x4880,
                ("ext", Sz::L) => 0x48C0,
                ("extb", Sz::L) => 0x49C0,
                _ => return Err(bad(m)),
            };
            e.w(opc | d);
            Ok(())
        }
        "exg" => {
            let (Some(a), Some(b)) = (ops.first().and_then(gp), ops.get(1).and_then(gp)) else { return Err(bad(m)) };
            let w = match (a >= 8, b >= 8) {
                (false, false) => 0xC140 | a << 9 | b,
                (true, true) => 0xC148 | (a - 8) << 9 | (b - 8),
                (false, true) => 0xC188 | a << 9 | (b - 8),
                (true, false) => 0xC188 | b << 9 | (a - 8),
            };
            e.w(w);
            Ok(())
        }
        "lea" => {
            let a = ops.get(1).and_then(areg).ok_or_else(|| bad(m))?;
            e.w(0x41C0 | a << 9);
            let ea = e.ea(o(0), Sz::L, CONTROL)?;
            e.patch(0, ea);
            Ok(())
        }
        "pea" | "jmp" | "jsr" => {
            if n != 1 {
                return Err(bad(m));
            }
            e.w(match m {
                "pea" => 0x4840,
                "jmp" => 0x4EC0,
                _ => 0x4E80,
            });
            let ea = e.ea(o(0), Sz::L, CONTROL)?;
            e.patch(0, ea);
            Ok(())
        }
        "link" => {
            let a = ops.first().and_then(areg).ok_or_else(|| bad(m))?;
            let s_ = szd(Sz::W);
            if s_ == Sz::L {
                e.w(0x4808 | a);
            } else {
                e.w(0x4E50 | a);
            }
            e.imm(ops.get(1).ok_or_else(|| bad(m))?, s_)
        }
        "unlk" => {
            e.w(0x4E58 | ops.first().and_then(areg).ok_or_else(|| bad(m))?);
            Ok(())
        }
        "trap" => {
            e.w(0x4E40 | quick(ops.first().ok_or_else(|| bad(m))?, 0, 15, "trap number")? as u16);
            Ok(())
        }
        "bkpt" => {
            e.w(0x4848 | quick(ops.first().ok_or_else(|| bad(m))?, 0, 7, "breakpoint number")? as u16);
            Ok(())
        }
        "stop" | "rtd" => {
            e.w(if m == "stop" { 0x4E72 } else { 0x4E74 });
            e.imm(ops.first().ok_or_else(|| bad(m))?, Sz::W)
        }
        "chk" => {
            let d = ops.get(1).and_then(dreg).ok_or_else(|| bad(m))?;
            let s_ = szd(Sz::W);
            e.w(match s_ {
                Sz::W => 0x4180,
                Sz::L => 0x4100,
                _ => return Err(bad(m)),
            } | d << 9);
            let ea = e.ea(o(0), s_, DATA)?;
            e.patch(0, ea);
            Ok(())
        }
        "chk2" | "cmp2" => {
            let r = ops.get(1).and_then(gp).ok_or_else(|| bad(m))?;
            let s_ = szd(Sz::W);
            e.w(0x00C0 | std_size(s_)? << 9);
            e.w(r << 12 | if m == "chk2" { 0x800 } else { 0 });
            let ea = e.ea(o(0), s_, CONTROL)?;
            e.patch(0, ea);
            Ok(())
        }
        "cas" => {
            let (Some(dc), Some(du)) = (ops.first().and_then(dreg), ops.get(1).and_then(dreg)) else { return Err(bad(m)) };
            let s_ = szd(Sz::W);
            e.w(0x08C0 | (std_size(s_)? + 1) << 9);
            e.w(du << 6 | dc);
            let ea = e.ea(ops.get(2).ok_or_else(|| bad(m))?, s_, MEM_ALT)?;
            e.patch(0, ea);
            Ok(())
        }
        "cas2" => {
            let (Some(Op::Pair(c1, c2)), Some(Op::Pair(u1, u2)), Some(Op::IndPair(r1, r2))) = (ops.first(), ops.get(1), ops.get(2)) else {
                return Err(bad(m));
            };
            let s_ = szd(Sz::W);
            e.w(match s_ {
                Sz::W => 0x0CFC,
                Sz::L => 0x0EFC,
                _ => return Err(bad(m)),
            });
            e.w((*r1 as u16) << 12 | (*u1 as u16) << 6 | *c1 as u16);
            e.w((*r2 as u16) << 12 | (*u2 as u16) << 6 | *c2 as u16);
            Ok(())
        }
        // ---- bit operations --------------------------------------------------------------
        "btst" | "bchg" | "bclr" | "bset" => {
            let t: u16 = match m {
                "btst" => 0,
                "bchg" => 1,
                "bclr" => 2,
                _ => 3,
            };
            if n != 2 {
                return Err(bad(m));
            }
            let s_ = if matches!(o(1), Op::D(_)) { Sz::L } else { Sz::B };
            let allowed = if t == 0 { DATA } else { DATA_ALT };
            if let Some(d) = dreg(o(0)) {
                e.w(0x0100 | d << 9 | t << 6);
            } else {
                e.w(0x0800 | t << 6);
                e.imm(o(0), Sz::B)?;
            }
            let ea = e.ea(o(1), s_, allowed & if dreg(o(0)).is_none() { !EA_IMM } else { ALL })?;
            e.patch(0, ea);
            Ok(())
        }
        "bftst" | "bfextu" | "bfchg" | "bfexts" | "bfclr" | "bfffo" | "bfset" | "bfins" => {
            let opc: u16 = match m {
                "bftst" => 0xE8C0,
                "bfextu" => 0xE9C0,
                "bfchg" => 0xEAC0,
                "bfexts" => 0xEBC0,
                "bfclr" => 0xECC0,
                "bfffo" => 0xEDC0,
                "bfset" => 0xEEC0,
                _ => 0xEFC0,
            };
            let (bf, reg) = match m {
                "bfins" => (ops.get(1), ops.first().and_then(dreg)),
                "bfextu" | "bfexts" | "bfffo" => (ops.first(), ops.get(1).and_then(dreg)),
                _ => (ops.first(), Some(0)),
            };
            let Some(Op::BitField { ea, off, width }) = bf else { return Err("expected ea{offset:width}".into()) };
            let reg = reg.ok_or_else(|| bad(m))?;
            let mut ext = reg << 12;
            match off {
                BfArg::D(d) => ext |= 0x800 | (*d as u16) << 6,
                BfArg::Imm(v) => {
                    if *v > 31 {
                        return Err("bit field offset must be 0-31".into());
                    }
                    ext |= (*v as u16) << 6
                }
            }
            match width {
                BfArg::D(d) => ext |= 0x20 | *d as u16,
                BfArg::Imm(v) => {
                    if !(1..=32).contains(v) {
                        return Err("bit field width must be 1-32".into());
                    }
                    ext |= (*v as u16) & 31
                }
            }
            e.w(opc);
            e.w(ext);
            let allowed = if matches!(m, "bftst" | "bfextu" | "bfexts" | "bfffo") { EA_DN | CONTROL } else { EA_DN | CTRL_ALT };
            let mode = e.ea(ea, Sz::L, allowed)?;
            e.patch(0, mode);
            Ok(())
        }
        // ---- shifts -----------------------------------------------------------------------
        "asl" | "asr" | "lsl" | "lsr" | "rol" | "ror" | "roxl" | "roxr" => {
            let left = m.ends_with('l') as u16;
            let typ: u16 = match &m[..m.len() - 1] {
                "as" => 0,
                "ls" => 1,
                "rox" => 2,
                _ => 3,
            };
            match (ops.first(), ops.get(1)) {
                (Some(Op::D(y)), None) => {
                    // shift a register by one
                    let ss = std_size(szd(Sz::W))?;
                    e.w(0xE000 | 1 << 9 | left << 8 | ss << 6 | typ << 3 | *y as u16);
                }
                (Some(cnt), Some(Op::D(y))) => {
                    let ss = std_size(szd(Sz::W))?;
                    let (c, ir) = match cnt {
                        Op::D(x) => (*x as u16, 1),
                        _ => ((quick(cnt, 1, 8, "shift count")? as u16) & 7, 0),
                    };
                    e.w(0xE000 | c << 9 | left << 8 | ss << 6 | ir << 5 | typ << 3 | *y as u16);
                }
                (Some(ea), None) => {
                    if szd(Sz::W) != Sz::W {
                        return Err("memory shifts are word-sized".into());
                    }
                    e.w(0xE0C0 | typ << 9 | left << 8);
                    let mode = e.ea(ea, Sz::W, MEM_ALT)?;
                    e.patch(0, mode);
                }
                _ => return Err(bad(m)),
            }
            Ok(())
        }
        // ---- moves --------------------------------------------------------------------------
        "move" | "movea" => {
            if n != 2 {
                return Err(bad(m));
            }
            let (s, d) = (o(0), o(1));
            match (s, d) {
                (Op::Spec(Special::Usp), Op::A(a)) => {
                    e.w(0x4E68 | *a as u16);
                    return Ok(());
                }
                (Op::A(a), Op::Spec(Special::Usp)) => {
                    e.w(0x4E60 | *a as u16);
                    return Ok(());
                }
                (Op::Spec(Special::Sr), _) => {
                    e.w(0x40C0);
                    let ea = e.ea(d, Sz::W, DATA_ALT)?;
                    e.patch(0, ea);
                    return Ok(());
                }
                (Op::Spec(Special::Ccr), _) => {
                    e.w(0x42C0);
                    let ea = e.ea(d, Sz::W, DATA_ALT)?;
                    e.patch(0, ea);
                    return Ok(());
                }
                (_, Op::Spec(sp @ (Special::Sr | Special::Ccr))) => {
                    e.w(if *sp == Special::Sr { 0x46C0 } else { 0x44C0 });
                    let ea = e.ea(s, Sz::W, DATA)?;
                    e.patch(0, ea);
                    return Ok(());
                }
                _ => {}
            }
            let s_ = szd(Sz::W);
            if !matches!(s_, Sz::B | Sz::W | Sz::L) {
                return Err("bad size for move".into());
            }
            if s_ == Sz::B && (matches!(s, Op::A(_)) || matches!(d, Op::A(_))) {
                return Err("byte move to or from an address register".into());
            }
            // movea.l #0,An -> suba.l An,An; movea.l #x,An -> movea.w when x fits
            let mut s_ = s_;
            if let (Op::Imm(v), Op::A(a), Sz::L) = (s, d, s_) {
                if v.is_abs() && v.off == 0 {
                    e.w(0x91C8 | (*a as u16) << 9 | *a as u16);
                    return Ok(());
                }
                if v.is_abs() && fits16(v.off) {
                    s_ = Sz::W;
                }
            }
            let code: u16 = match s_ {
                Sz::B => 1,
                Sz::W => 3,
                _ => 2,
            };
            e.w(code << 12);
            let src = e.ea(s, s_, ALL)?;
            let dst = e.ea(d, s_, ALTER)?;
            let dst = (dst & 7) << 9 | (dst >> 3) << 6;
            e.patch(0, src | dst);
            Ok(())
        }
        "moveq" => {
            let d = ops.get(1).and_then(dreg).ok_or_else(|| bad(m))?;
            let v = quick(o(0), -128, 255, "moveq value")?;
            e.w(0x7000 | d << 9 | (v as u8 as u16));
            Ok(())
        }
        "movem" => {
            if n != 2 {
                return Err(bad(m));
            }
            let s_ = szd(Sz::W);
            let ss: u16 = match s_ {
                Sz::W => 0,
                Sz::L => 0x40,
                _ => return Err(bad(m)),
            };
            let list = |op: &Op| -> Option<u16> {
                match op {
                    Op::List(l) => Some(*l),
                    Op::D(r) => Some(1 << r),
                    Op::A(r) => Some(1 << (8 + r)),
                    Op::Imm(v) if v.is_abs() => Some(v.off as u16),
                    _ => None,
                }
            };
            if let Some(mask) = list(o(0)) {
                // registers to memory
                let mask = if matches!(o(1), Op::Pre(_)) { mask.reverse_bits() } else { mask };
                e.w(0x4880 | ss);
                e.w(mask);
                let ea = e.ea(o(1), s_, CTRL_ALT | EA_PRE)?;
                e.patch(0, ea);
            } else if let Some(mask) = list(o(1)) {
                e.w(0x4C80 | ss);
                e.w(mask);
                let ea = e.ea(o(0), s_, CONTROL | EA_POST)?;
                e.patch(0, ea);
            } else {
                return Err(bad(m));
            }
            Ok(())
        }
        "movep" => {
            let s_ = szd(Sz::W);
            let l = (s_ == Sz::L) as u16;
            let mem = |op: &Op| -> Option<(u16, Val)> {
                match op {
                    Op::Ind(r) => Some((*r as u16, Val::abs(0))),
                    Op::Disp { base: BaseReg::An(r), d } => Some((*r as u16, d.v)),
                    _ => None,
                }
            };
            if let (Some((a, v)), Some(d)) = (ops.first().and_then(mem), ops.get(1).and_then(dreg)) {
                e.w(0x0108 | d << 9 | l << 6 | a);
                e.field(FixKind::Abs16, v, 0)
            } else if let (Some(d), Some((a, v))) = (ops.first().and_then(dreg), ops.get(1).and_then(mem)) {
                e.w(0x0188 | d << 9 | l << 6 | a);
                e.field(FixKind::Abs16, v, 0)
            } else {
                Err(bad(m))
            }
        }
        "moves" => {
            let s_ = szd(Sz::W);
            e.w(0x0E00 | std_size(s_)? << 6);
            if let Some(r) = ops.first().and_then(gp) {
                e.w(r << 12 | 0x800);
                let ea = e.ea(ops.get(1).ok_or_else(|| bad(m))?, s_, MEM_ALT)?;
                e.patch(0, ea);
            } else if let Some(r) = ops.get(1).and_then(gp) {
                e.w(r << 12);
                let ea = e.ea(o(0), s_, MEM_ALT)?;
                e.patch(0, ea);
            } else {
                return Err(bad(m));
            }
            Ok(())
        }
        "movec" => {
            match (ops.first(), ops.get(1)) {
                (Some(Op::Spec(c)), Some(r)) if gp(r).is_some() => {
                    e.w(0x4E7A);
                    e.w(gp(r).unwrap() << 12 | c.movec_code().ok_or("not a control register")?);
                }
                (Some(r), Some(Op::Spec(c))) if gp(r).is_some() => {
                    e.w(0x4E7B);
                    e.w(gp(r).unwrap() << 12 | c.movec_code().ok_or("not a control register")?);
                }
                _ => return Err(bad(m)),
            }
            Ok(())
        }
        _ => Err(format!("unknown instruction `{m}`")),
    }
}

/// FPU instructions. Returns Ok(None) if `m` is not an FPU mnemonic.
fn fpu(e: &mut Enc, m: &str, sz: Option<Sz>, ops: &[Op]) -> R<Option<()>> {
    let n = ops.len();
    if m == "fnop" {
        e.w(0xF280);
        e.w(0);
        return Ok(Some(()));
    }
    if m == "fsave" || m == "frestore" {
        let save = m == "fsave";
        e.w(if save { 0xF300 } else { 0xF340 });
        let allowed = if save { CTRL_ALT | EA_PRE } else { CONTROL | EA_POST };
        let ea = e.ea(ops.first().ok_or_else(|| bad(m))?, Sz::L, allowed)?;
        e.patch(0, ea);
        return Ok(Some(()));
    }
    if let Some(c) = m.strip_prefix("fdb").and_then(fcond) {
        let (Some(d), Some(Op::Abs { v, .. })) = (ops.first().and_then(dreg), ops.get(1)) else { return Err(bad(m)) };
        e.w(0xF248 | d);
        e.w(c);
        e.field(FixKind::Pc16, *v, 4)?;
        return Ok(Some(()));
    }
    if let Some(c) = m.strip_prefix("ftrap").and_then(fcond) {
        match (n, sz) {
            (0, None) => {
                e.w(0xF27C);
                e.w(c);
            }
            (1, Some(s @ (Sz::W | Sz::L))) => {
                e.w(if s == Sz::W { 0xF27A } else { 0xF27B });
                e.w(c);
                e.imm(&ops[0], s)?;
            }
            _ => return Err(bad(m)),
        }
        return Ok(Some(()));
    }
    if let Some(c) = m.strip_prefix("fb").and_then(fcond) {
        let Some(Op::Abs { v, .. }) = ops.first() else { return Err(bad(m)) };
        let long = match sz {
            Some(Sz::L) => true,
            Some(Sz::W) => false,
            Some(_) => return Err(bad(m)),
            None => !e.pcdisp(*v, 2).is_some_and(fits16),
        };
        if long {
            e.w(0xF2C0 | c);
            e.field(FixKind::Pc32, *v, 2)?;
        } else {
            e.w(0xF280 | c);
            e.field(FixKind::Pc16, *v, 2)?;
        }
        return Ok(Some(()));
    }
    if let Some(c) = m.strip_prefix("fs").and_then(fcond).filter(|_| !matches!(m, "fsin" | "fsub" | "fsinh" | "fsqrt" | "fscale" | "fsave")) {
        e.w(0xF240);
        e.w(c);
        let ea = e.ea(ops.first().ok_or_else(|| bad(m))?, Sz::B, DATA_ALT)?;
        e.patch(0, ea);
        return Ok(Some(()));
    }
    if m == "fmovecr" {
        let (Some(rom), Some(Op::Fp(d))) = (ops.first(), ops.get(1)) else { return Err(bad(m)) };
        let off = quick(rom, 0, 0x7F, "constant ROM offset")? as u16;
        e.w(0xF200);
        e.w(0x5C00 | (*d as u16) << 7 | off);
        return Ok(Some(()));
    }
    if m == "fmovem" || m == "fmove" && matches!(ops.first(), Some(Op::FpCtl(_))) || m == "fmove" && matches!(ops.get(1), Some(Op::FpCtl(_))) {
        // control registers
        match (ops.first(), ops.get(1)) {
            (Some(Op::FpCtl(l)), Some(d)) => {
                e.w(0xF200);
                e.w(0xA000 | (*l as u16) << 10);
                let allowed = if l.count_ones() == 1 { ALTER } else { MEM_ALT };
                let ea = e.ea(d, Sz::L, allowed)?;
                e.patch(0, ea);
                return Ok(Some(()));
            }
            (Some(s), Some(Op::FpCtl(l))) => {
                e.w(0xF200);
                e.w(0x8000 | (*l as u16) << 10);
                let allowed = if l.count_ones() == 1 { ALL } else { MEMORY };
                let ea = e.ea(s, Sz::L, allowed)?;
                e.patch(0, ea);
                return Ok(Some(()));
            }
            _ => {}
        }
        if m == "fmove" {
            return Err(bad(m));
        }
        let list = |op: &Op| -> Option<Result<u8, u8>> {
            match op {
                Op::FpList(l) => Some(Ok(*l)),
                Op::Fp(r) => Some(Ok(1 << r)),
                Op::D(r) => Some(Err(*r)),
                _ => None,
            }
        };
        let (regs, ea, to_mem) = match (ops.first().and_then(list), ops.get(1).and_then(list)) {
            (Some(l), _) if !matches!(ops.get(1), Some(Op::FpList(_) | Op::Fp(_))) => (l, ops.get(1), true),
            (_, Some(l)) => (l, ops.first(), false),
            _ => return Err(bad(m)),
        };
        let ea = ea.ok_or_else(|| bad(m))?;
        let predec = matches!(ea, Op::Pre(_));
        let (mode, mask) = match regs {
            Ok(l) => (if predec { 0 } else { 2 }, if predec { l as u16 } else { l.reverse_bits() as u16 }),
            Err(d) => (if predec { 1 } else { 3 }, (d as u16) << 4),
        };
        e.w(0xF200);
        e.w(0xC000 | (to_mem as u16) << 13 | mode << 11 | mask);
        let allowed = if to_mem { CTRL_ALT | EA_PRE } else { CONTROL | EA_POST };
        let mode = e.ea(ea, Sz::X, allowed)?;
        e.patch(0, mode);
        return Ok(Some(()));
    }
    if m == "fsincos" {
        let (Some(src), Some(Op::FpPair(c, s))) = (ops.first(), ops.get(1)) else { return Err(bad(m)) };
        fsincos(e, sz, src, *c, *s)?;
        return Ok(Some(()));
    }
    if m == "ftst" {
        let s = ops.first().ok_or_else(|| bad(m))?;
        e.w(0xF200);
        if let Op::Fp(r) = s {
            e.w((*r as u16) << 10 | 0x3A);
        } else {
            let s_ = sz.unwrap_or(Sz::X);
            e.w(0x4000 | fpu_fmt(s_) << 10 | 0x3A);
            let ea = e.ea(s, s_, if matches!(s_, Sz::B | Sz::W | Sz::L | Sz::S) { DATA } else { MEMORY })?;
            e.patch(0, ea);
        }
        return Ok(Some(()));
    }
    let Some(opm) = fpu_op(m) else {
        return Ok(None);
    };
    let s_ = sz.unwrap_or(Sz::X);
    // fmove fpn,<ea>
    if m == "fmove" {
        if let (Some(Op::Fp(r)), Some(d)) = (ops.first(), ops.get(1)) {
            if !matches!(d, Op::Fp(_)) {
                e.w(0xF200);
                e.w(0x6000 | fpu_fmt(s_) << 10 | (*r as u16) << 7);
                let allowed = if matches!(s_, Sz::B | Sz::W | Sz::L | Sz::S) { DATA_ALT } else { MEM_ALT };
                let ea = e.ea(d, s_, allowed)?;
                e.patch(0, ea);
                return Ok(Some(()));
            }
        }
    }
    let (src, dst) = match (n, ops.first(), ops.get(1)) {
        (1, Some(Op::Fp(r)), None) if monadic(opm) => (&ops[0], *r),
        (2, Some(s), Some(Op::Fp(d))) => (s, *d),
        _ => return Err(bad(m)),
    };
    e.w(0xF200);
    if let Op::Fp(r) = src {
        e.w((*r as u16) << 10 | (dst as u16) << 7 | opm);
    } else {
        e.w(0x4000 | fpu_fmt(s_) << 10 | (dst as u16) << 7 | opm);
        let allowed = if matches!(s_, Sz::B | Sz::W | Sz::L | Sz::S) { DATA } else { MEMORY };
        let ea = e.ea(src, s_, allowed)?;
        e.patch(0, ea);
    }
    Ok(Some(()))
}

/// 68030 PMMU instructions. Returns Ok(None) if `m` is not one.
fn pmmu(e: &mut Enc, m: &str, _sz: Option<Sz>, ops: &[Op]) -> R<Option<()>> {
    match m {
        "pmove" | "pmovefd" => {
            let fd: u16 = if m == "pmovefd" { 0x100 } else { 0 };
            let code = |sp: &Special| -> Option<u16> {
                Some(match sp {
                    Special::Tc => 0x4000,
                    Special::Srp => 0x4800,
                    Special::Crp => 0x4C00,
                    Special::Tt0 => 0x0800,
                    Special::Tt1 => 0x0C00,
                    Special::Mmusr => 0x6000,
                    _ => return None,
                })
            };
            let (ext, ea, read) = match (ops.first(), ops.get(1)) {
                (Some(Op::Spec(sp)), Some(d)) if code(sp).is_some() => (code(sp).unwrap(), d, true),
                (Some(s), Some(Op::Spec(sp))) if code(sp).is_some() => (code(sp).unwrap(), s, false),
                _ => return Err(bad(m)),
            };
            if fd != 0 && ext == 0x6000 {
                return Err("pmovefd cannot access the MMUSR".into());
            }
            e.w(0xF000);
            e.w(ext | if read { 0x200 } else { 0 } | fd);
            let size = match ext {
                0x4800 | 0x4C00 => Sz::D,
                0x6000 => Sz::W,
                _ => Sz::L,
            };
            let allowed = if read { CTRL_ALT } else { CONTROL };
            let mode = e.ea(ea, size, allowed)?;
            e.patch(0, mode);
            Ok(Some(()))
        }
        "pflusha" => {
            e.w(0xF000);
            e.w(0x2400);
            Ok(Some(()))
        }
        "pflush" => {
            let fc = mmu_fc(ops.first().ok_or_else(|| bad(m))?)?;
            let mask = quick(ops.get(1).ok_or_else(|| bad(m))?, 0, 7, "pflush mask")? as u16;
            if let Some(ea) = ops.get(2) {
                e.w(0xF000);
                e.w(0x3800 | mask << 5 | fc);
                let mode = e.ea(ea, Sz::L, CTRL_ALT)?;
                e.patch(0, mode);
            } else {
                e.w(0xF000);
                e.w(0x3000 | mask << 5 | fc);
            }
            Ok(Some(()))
        }
        "ploadr" | "ploadw" => {
            let fc = mmu_fc(ops.first().ok_or_else(|| bad(m))?)?;
            e.w(0xF000);
            e.w(if m == "ploadr" { 0x2200 } else { 0x2000 } | fc);
            let mode = e.ea(ops.get(1).ok_or_else(|| bad(m))?, Sz::L, CTRL_ALT)?;
            e.patch(0, mode);
            Ok(Some(()))
        }
        "ptestr" | "ptestw" => {
            let fc = mmu_fc(ops.first().ok_or_else(|| bad(m))?)?;
            let level = quick(ops.get(2).ok_or_else(|| bad(m))?, 0, 7, "ptest level")? as u16;
            let mut ext = 0x8000 | level << 10 | if m == "ptestr" { 0x200 } else { 0 } | fc;
            if let Some(a) = ops.get(3) {
                ext |= 0x100 | areg(a).ok_or_else(|| bad(m))? << 5;
            }
            e.w(0xF000);
            e.w(ext);
            let mode = e.ea(ops.get(1).ok_or_else(|| bad(m))?, Sz::L, CTRL_ALT)?;
            e.patch(0, mode);
            Ok(Some(()))
        }
        _ => Ok(None),
    }
}

/// `fsincos.<fmt> <ea>,fpc:fps`
fn fsincos(e: &mut Enc, sz: Option<Sz>, src: &Op, fpc: u8, fps: u8) -> R {
    let s_ = sz.unwrap_or(Sz::X);
    e.w(0xF200);
    if let Op::Fp(r) = src {
        e.w((*r as u16) << 10 | (fps as u16) << 7 | 0x30 | fpc as u16);
    } else {
        e.w(0x4000 | fpu_fmt(s_) << 10 | (fps as u16) << 7 | 0x30 | fpc as u16);
        let allowed = if matches!(s_, Sz::B | Sz::W | Sz::L | Sz::S) { DATA } else { MEMORY };
        let ea = e.ea(src, s_, allowed)?;
        e.patch(0, ea);
    }
    Ok(())
}
