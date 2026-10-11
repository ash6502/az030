//! C types.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Void,
    Bool,
    /// plain char (signed)
    Char,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LLong,
    ULLong,
    Float,
    Double,
    LDouble,
    Ptr(Rc<Ty>),
    Array(Rc<Ty>, Option<u32>),
    Func(Rc<FnTy>),
    /// index into the struct/union table
    Struct(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnTy {
    pub ret: Ty,
    pub params: Vec<Ty>,
    pub variadic: bool,
    /// declared with a prototype (not `int f()`)
    pub proto: bool,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub ty: Ty,
    pub offset: u32,
    /// (bit offset from the MSB of the 32-bit unit at `offset`, width)
    pub bits: Option<(u8, u8)>,
}

#[derive(Clone, Debug, Default)]
pub struct StructDef {
    pub tag: String,
    pub union: bool,
    pub members: Vec<Member>,
    pub size: u32,
    pub align: u32,
    pub complete: bool,
}

/// How a value is held during evaluation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// in d0 (up to 32 bits, extended)
    I,
    /// in d0 (high) : d1 (low)
    L,
    /// in fp0
    F,
    /// address in d0
    S,
    V,
}

impl Ty {
    pub fn ptr(t: Ty) -> Ty {
        Ty::Ptr(Rc::new(t))
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Ty::Bool | Ty::Char | Ty::SChar | Ty::UChar | Ty::Short | Ty::UShort | Ty::Int | Ty::UInt | Ty::Long | Ty::ULong | Ty::LLong | Ty::ULLong
        )
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float | Ty::Double | Ty::LDouble)
    }
    pub fn is_arith(&self) -> bool {
        self.is_integer() || self.is_float()
    }
    pub fn is_ptr(&self) -> bool {
        matches!(self, Ty::Ptr(_))
    }
    pub fn is_scalar(&self) -> bool {
        self.is_arith() || self.is_ptr()
    }
    pub fn is_unsigned(&self) -> bool {
        matches!(self, Ty::Bool | Ty::UChar | Ty::UShort | Ty::UInt | Ty::ULong | Ty::ULLong | Ty::Ptr(_))
    }
    pub fn is_ll(&self) -> bool {
        matches!(self, Ty::LLong | Ty::ULLong)
    }
    pub fn is_struct(&self) -> bool {
        matches!(self, Ty::Struct(_))
    }
    pub fn is_func(&self) -> bool {
        matches!(self, Ty::Func(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Ty::Array(..))
    }
    pub fn is_void(&self) -> bool {
        matches!(self, Ty::Void)
    }
    /// Pointed-to or element type.
    pub fn base(&self) -> Option<&Ty> {
        match self {
            Ty::Ptr(t) | Ty::Array(t, _) => Some(t),
            _ => None,
        }
    }
    pub fn func(&self) -> Option<&FnTy> {
        match self {
            Ty::Func(f) => Some(f),
            Ty::Ptr(t) => match &**t {
                Ty::Func(f) => Some(f),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn class(&self) -> Class {
        match self {
            Ty::Void => Class::V,
            Ty::LLong | Ty::ULLong => Class::L,
            Ty::Float | Ty::Double | Ty::LDouble => Class::F,
            Ty::Struct(_) | Ty::Array(..) | Ty::Func(_) => Class::S,
            _ => Class::I,
        }
    }

    /// Integer conversion rank (for the usual arithmetic conversions).
    pub fn rank(&self) -> u8 {
        match self {
            Ty::Bool => 1,
            Ty::Char | Ty::SChar | Ty::UChar => 2,
            Ty::Short | Ty::UShort => 3,
            Ty::Int | Ty::UInt => 4,
            Ty::Long | Ty::ULong => 5,
            Ty::LLong | Ty::ULLong => 6,
            _ => 0,
        }
    }

    pub fn to_unsigned(&self) -> Ty {
        match self {
            Ty::Char | Ty::SChar => Ty::UChar,
            Ty::Short => Ty::UShort,
            Ty::Int => Ty::UInt,
            Ty::Long => Ty::ULong,
            Ty::LLong => Ty::ULLong,
            t => t.clone(),
        }
    }
}

/// Size and alignment, given the struct table.
pub fn size_align(t: &Ty, structs: &[StructDef]) -> (u32, u32) {
    match t {
        Ty::Void => (1, 1),
        Ty::Bool | Ty::Char | Ty::SChar | Ty::UChar => (1, 1),
        Ty::Short | Ty::UShort => (2, 2),
        Ty::Int | Ty::UInt | Ty::Long | Ty::ULong | Ty::Float | Ty::Ptr(_) => (4, 4),
        Ty::LLong | Ty::ULLong | Ty::Double | Ty::LDouble => (8, 4),
        Ty::Array(e, n) => {
            let (s, a) = size_align(e, structs);
            (s * n.unwrap_or(0), a)
        }
        Ty::Func(_) => (1, 1),
        Ty::Struct(i) => {
            let d = &structs[*i];
            (d.size, d.align.max(1))
        }
    }
}

pub fn size_of(t: &Ty, structs: &[StructDef]) -> u32 {
    size_align(t, structs).0
}

pub fn align_up(n: u32, a: u32) -> u32 {
    n.div_ceil(a.max(1)) * a.max(1)
}

/// Are two types compatible enough for assignment/redeclaration (loosely)?
pub fn compatible(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Ptr(x), Ty::Ptr(y)) => x.is_void() || y.is_void() || compatible(x, y),
        (Ty::Array(x, _), Ty::Array(y, _)) => compatible(x, y),
        (Ty::Func(f), Ty::Func(g)) => compatible(&f.ret, &g.ret),
        (Ty::Char, Ty::SChar) | (Ty::SChar, Ty::Char) => true,
        (x, y) => x == y,
    }
}

/// Spell a type for diagnostics.
pub fn name(t: &Ty, structs: &[StructDef]) -> String {
    use alloc::format;
    match t {
        Ty::Void => "void".into(),
        Ty::Bool => "_Bool".into(),
        Ty::Char => "char".into(),
        Ty::SChar => "signed char".into(),
        Ty::UChar => "unsigned char".into(),
        Ty::Short => "short".into(),
        Ty::UShort => "unsigned short".into(),
        Ty::Int => "int".into(),
        Ty::UInt => "unsigned int".into(),
        Ty::Long => "long".into(),
        Ty::ULong => "unsigned long".into(),
        Ty::LLong => "long long".into(),
        Ty::ULLong => "unsigned long long".into(),
        Ty::Float => "float".into(),
        Ty::Double => "double".into(),
        Ty::LDouble => "long double".into(),
        Ty::Ptr(b) => format!("{} *", name(b, structs)),
        Ty::Array(b, Some(n)) => format!("{}[{n}]", name(b, structs)),
        Ty::Array(b, None) => format!("{}[]", name(b, structs)),
        Ty::Func(f) => format!("{} (*)(...)", name(&f.ret, structs)),
        Ty::Struct(i) => {
            let d = &structs[*i];
            format!("{} {}", if d.union { "union" } else { "struct" }, if d.tag.is_empty() { "<anonymous>" } else { &d.tag })
        }
    }
}
