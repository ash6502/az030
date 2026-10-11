//! The typed syntax tree produced by the parser.

use crate::types::Ty;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct Expr {
    pub k: E,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub enum E {
    Int(i64),
    Flt(f64),
    /// address of string literal n
    Str(usize),
    /// local variable (lvalue)
    Local(usize),
    /// global variable or function, by assembly name (lvalue for objects)
    Global(String),
    /// *p (lvalue)
    Deref(Box<Expr>),
    /// s.m (lvalue): base lvalue (struct/union typed), byte offset, bit-field
    Member(Box<Expr>, u32, Option<(u8, u8)>),
    /// &lvalue; also array-to-pointer and function-to-pointer decay
    Addr(Box<Expr>),
    /// convert the operand to `ty`
    Cast(Box<Expr>),
    /// "-", "~", "!"
    Unary(&'static str, Box<Expr>),
    /// arithmetic in `ty` (+ - * / % & | ^ << >>), or comparison (< > <= >= == !=)
    /// with both operands of the same type (result int)
    Bin(&'static str, Box<Expr>, Box<Expr>),
    LogAnd(Box<Expr>, Box<Expr>),
    LogOr(Box<Expr>, Box<Expr>),
    /// lhs = rhs (rhs already converted)
    Assign(Box<Expr>, Box<Expr>),
    /// lhs op= rhs: computed in the given type (rhs converted to it, scaled for
    /// pointers), stored back with conversion to lhs's type
    OpAssign(&'static str, Box<Expr>, Box<Expr>, Ty),
    /// ++/--: amount added (scaled for pointers), prefix?
    IncDec(Box<Expr>, i64, bool),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
    /// function (designator or pointer), arguments (converted)
    Call(Box<Expr>, Vec<Expr>),
    /// va_start(ap): ap = frame offset of the first variadic argument
    VaStart(Box<Expr>, i32),
    /// va_arg(ap, ty)
    VaArg(Box<Expr>),
    /// GNU statement expression / compound literal: run the statements, then value
    StmtExpr(Box<crate::ast::Stmt>, Box<Expr>),
}

impl Expr {
    pub fn new(k: E, ty: Ty) -> Expr {
        Expr { k, ty }
    }
    pub fn int(v: i64) -> Expr {
        Expr { k: E::Int(v), ty: Ty::Int }
    }
    pub fn is_lvalue(&self) -> bool {
        matches!(self.k, E::Local(_) | E::Global(_) | E::Deref(_) | E::Member(..))
    }
    pub fn const_int(&self) -> Option<i64> {
        match self.k {
            E::Int(v) => Some(v),
            _ => None,
        }
    }
    /// No side effects and cheap to evaluate twice.
    pub fn is_pure(&self) -> bool {
        match &self.k {
            E::Int(_) | E::Flt(_) | E::Str(_) | E::Local(_) | E::Global(_) => true,
            E::Addr(e) | E::Cast(e) | E::Deref(e) | E::Member(e, ..) | E::Unary(_, e) => e.is_pure(),
            E::Bin(_, a, b) => a.is_pure() && b.is_pure(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    /// value, body, (case value, label id), default label id
    Switch(Expr, Box<Stmt>, Vec<(i64, usize)>, Option<usize>),
    /// a case/default label (label id) in a switch body
    CaseLabel(usize),
    Break,
    Continue,
    Return(Option<Expr>),
    Goto(String),
    Label(String),
    /// initialize local `id` (zero-fill first for aggregates)
    Init(usize, Vec<InitItem>, bool),
    Empty,
}

/// One scalar of an initializer: byte offset, type, bit-field, value.
#[derive(Clone, Debug)]
pub struct InitItem {
    pub off: u32,
    pub ty: Ty,
    pub bits: Option<(u8, u8)>,
    pub val: Expr,
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    /// parameters: offset from a6
    pub param_off: Option<i32>,
    pub addr_taken: bool,
    pub volatile: bool,
    /// weighted use count (for register allocation)
    pub uses: u32,
}

pub struct Function {
    pub name: String,
    pub asm_name: String,
    pub global: bool,
    pub ty: Ty,
    pub locals: Vec<Local>,
    pub body: Stmt,
    /// struct-returning functions get a hidden pointer at 8(a6)
    pub sret: bool,
    pub labels: usize,
}

/// A constant initializer value for static storage.
#[derive(Clone, Debug)]
pub enum Const {
    Int(i64),
    Flt(f64),
    /// address of a symbol + offset
    Addr(String, i64),
    /// address of string literal n + offset
    Str(usize, i64),
}

#[derive(Clone, Debug)]
pub struct DataItem {
    pub off: u32,
    pub size: u32,
    pub bits: Option<(u8, u8)>,
    pub float: bool,
    pub val: Const,
}

pub struct Global {
    pub asm_name: String,
    pub ty: Ty,
    pub global: bool,
    /// None: zero-initialized (bss)
    pub init: Option<Vec<DataItem>>,
    pub readonly: bool,
}

pub struct Unit {
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
    /// string literals: units and unit size (1, 2 or 4)
    pub strings: Vec<(Vec<u32>, u32)>,
    pub structs: Vec<crate::types::StructDef>,
}

pub fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}
