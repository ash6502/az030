//! The parser: tokens to a typed syntax tree, with C's type rules applied.

use crate::ast::*;
use crate::lex::{Tok, Token};
use crate::pp::Pp;
use crate::types::*;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

type R<T> = Result<T, ()>;

/// Parse an integer literal: (value, type).
pub fn int_literal(s: &str) -> Option<(u64, Ty)> {
    let lower = s.to_ascii_lowercase();
    let digits_end = lower.trim_end_matches(['u', 'l']);
    let suffix = &lower[digits_end.len()..];
    let (radix, body) = if let Some(h) = digits_end.strip_prefix("0x") {
        (16, h)
    } else if let Some(b) = digits_end.strip_prefix("0b") {
        (2, b)
    } else if digits_end.len() > 1 && digits_end.starts_with('0') {
        (8, &digits_end[1..])
    } else {
        (10, digits_end)
    };
    if body.is_empty() {
        return None;
    }
    let v = u64::from_str_radix(body, radix).ok()?;
    let uns = suffix.contains('u');
    let longs = suffix.matches('l').count();
    let dec = radix == 10;
    // the first type that can represent the value
    let cands: &[Ty] = match (uns, longs) {
        (false, 0) if dec => &[Ty::Int, Ty::Long, Ty::LLong],
        (false, 0) => &[Ty::Int, Ty::UInt, Ty::Long, Ty::ULong, Ty::LLong, Ty::ULLong],
        (true, 0) => &[Ty::UInt, Ty::ULong, Ty::ULLong],
        (false, 1) if dec => &[Ty::Long, Ty::LLong],
        (false, 1) => &[Ty::Long, Ty::ULong, Ty::LLong, Ty::ULLong],
        (true, 1) => &[Ty::ULong, Ty::ULLong],
        (false, _) if dec => &[Ty::LLong],
        (false, _) => &[Ty::LLong, Ty::ULLong],
        (true, _) => &[Ty::ULLong],
    };
    for t in cands {
        let fits = match t {
            Ty::Int | Ty::Long => v <= i32::MAX as u64,
            Ty::UInt | Ty::ULong => v <= u32::MAX as u64,
            Ty::LLong => v <= i64::MAX as u64,
            _ => true,
        };
        if fits {
            return Some((v, t.clone()));
        }
    }
    Some((v, Ty::ULLong))
}

fn escape(it: &mut core::iter::Peekable<core::str::Chars>) -> Option<u32> {
    let c = it.next()?;
    Some(match c {
        'n' => 10,
        't' => 9,
        'r' => 13,
        'a' => 7,
        'b' => 8,
        'f' => 12,
        'v' => 11,
        'e' => 27,
        '0'..='7' => {
            let mut v = c as u32 - '0' as u32;
            for _ in 0..2 {
                match it.peek() {
                    Some(d @ '0'..='7') => {
                        v = v * 8 + (*d as u32 - '0' as u32);
                        it.next();
                    }
                    _ => break,
                }
            }
            v
        }
        'x' => {
            let mut v = 0u32;
            while let Some(d) = it.peek().and_then(|d| d.to_digit(16)) {
                v = v.wrapping_mul(16) + d;
                it.next();
            }
            v
        }
        c => c as u32,
    })
}

/// Decode the characters of a string or character literal body.
fn decode(body: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut it = body.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(v) = escape(&mut it) {
                out.push(v);
            }
        } else if (c as u32) < 0x80 {
            out.push(c as u32);
        } else {
            // UTF-8 bytes for narrow strings
            let mut b = [0u8; 4];
            for x in c.encode_utf8(&mut b).bytes() {
                out.push(x as u32);
            }
        }
    }
    out
}

pub fn char_literal(s: &str) -> Option<i64> {
    let (wide, body) = if let Some(r) = s.strip_prefix('L').or(s.strip_prefix('U')).or(s.strip_prefix('u')) { (true, r) } else { (false, s) };
    let body = body.strip_prefix('\'')?.strip_suffix('\'')?;
    let v = decode(body);
    if v.is_empty() {
        return None;
    }
    if wide {
        return Some(v[0] as i64);
    }
    // multi-character constants pack big-endian; a single char is sign-extended
    if v.len() == 1 {
        return Some(v[0] as u8 as i8 as i64);
    }
    let mut x: i64 = 0;
    for c in v.iter().take(4) {
        x = (x << 8) | (*c as i64 & 0xFF);
    }
    Some(x as i32 as i64)
}

/// Truncate a value to an integer type's width (with sign or zero extension).
pub fn wrap(v: i64, t: &Ty) -> i64 {
    match t {
        Ty::Bool => (v != 0) as i64,
        Ty::Char | Ty::SChar => v as i8 as i64,
        Ty::UChar => v as u8 as i64,
        Ty::Short => v as i16 as i64,
        Ty::UShort => v as u16 as i64,
        Ty::Int | Ty::Long => v as i32 as i64,
        Ty::UInt | Ty::ULong | Ty::Ptr(_) => v as u32 as i64,
        _ => v,
    }
}

#[derive(Clone)]
enum Sym {
    Local(usize),
    Global(String, Ty),
    Typedef(Ty),
    Enum(i64),
}

#[derive(Default)]
struct Scope {
    syms: BTreeMap<String, Sym>,
    tags: BTreeMap<String, usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Storage {
    #[default]
    None,
    Typedef,
    Extern,
    Static,
    Auto,
    Register,
}

#[derive(Default)]
struct Spec {
    storage: Storage,
    volatile: bool,
    konst: bool,
    inline: bool,
}

enum Deriv {
    Ptr,
    Array(Option<u32>),
    Func(Rc<FnTy>, Vec<Option<String>>),
}

struct GlobalInfo {
    idx: Option<usize>,
    ty: Ty,
    defined: bool,
    global: bool,
}

struct FnCtx {
    locals: Vec<Local>,
    labels: usize,
    loop_weight: u32,
    switches: Vec<(Vec<(i64, usize)>, Option<usize>)>,
    ret: Ty,
    name: String,
    /// offset from a6 just past the last named parameter (for va_start)
    va_off: i32,
    breakable: u32,
    continuable: u32,
}

pub struct Parser<'p, 'f> {
    pp: &'p mut Pp<'f>,
    look: VecDeque<Token>,
    last_line: (u16, u32),
    scopes: Vec<Scope>,
    pub unit: Unit,
    globals: BTreeMap<String, GlobalInfo>,
    f: Option<FnCtx>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    static_count: usize,
}

fn is_decl_start_word(w: &str) -> bool {
    matches!(
        w,
        "void" | "char" | "short" | "int" | "long" | "float" | "double" | "signed" | "unsigned" | "_Bool" | "struct" | "union" | "enum" | "typedef"
            | "extern" | "static" | "auto" | "register" | "const" | "volatile" | "inline" | "restrict" | "__restrict" | "__restrict__"
            | "__inline" | "__inline__" | "__const" | "__const__" | "__volatile__" | "__attribute__" | "__extension__" | "_Noreturn"
            | "__signed__" | "__signed" | "_Alignas" | "__builtin_va_list" | "_Complex" | "_Thread_local" | "_Static_assert"
    )
}

impl<'p, 'f> Parser<'p, 'f> {
    pub fn new(pp: &'p mut Pp<'f>) -> Self {
        Parser {
            pp,
            look: VecDeque::new(),
            last_line: (0, 0),
            scopes: vec![Scope::default()],
            unit: Unit { functions: Vec::new(), globals: Vec::new(), strings: Vec::new(), structs: Vec::new() },
            globals: BTreeMap::new(),
            f: None,
            errors: Vec::new(),
            warnings: Vec::new(),
            static_count: 0,
        }
    }

    // ---- tokens ----------------------------------------------------------------------

    fn fill(&mut self, n: usize) {
        while self.look.len() <= n {
            match self.pp.next() {
                Some(t) => self.look.push_back(t),
                None => break,
            }
        }
    }

    fn peek(&mut self) -> Option<&Token> {
        self.fill(0);
        self.look.front()
    }

    fn peek_at(&mut self, n: usize) -> Option<&Token> {
        self.fill(n);
        self.look.get(n)
    }

    fn next(&mut self) -> Option<Token> {
        self.fill(0);
        let t = self.look.pop_front();
        if let Some(t) = &t {
            self.last_line = (t.file, t.line);
        }
        t
    }

    fn is(&mut self, p: &str) -> bool {
        self.peek().is_some_and(|t| t.is(p))
    }

    fn is_word(&mut self, w: &str) -> bool {
        self.peek().is_some_and(|t| t.ident() == Some(w))
    }

    fn accept(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.next();
            true
        } else {
            false
        }
    }

    fn accept_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.next();
            true
        } else {
            false
        }
    }

    fn loc(&mut self) -> String {
        let (f, l) = match self.peek().cloned() {
            Some(t) => (t.file, t.line as i64 + 0),
            None => (self.last_line.0, self.last_line.1 as i64),
        };
        let line = match self.peek().cloned() {
            Some(t) => self.pp.line_of(&t),
            None => l,
        };
        format!("{}:{}", self.pp.file_names.get(f as usize).map_or("?", |s| s.as_str()), line)
    }

    fn error<T>(&mut self, msg: &str) -> R<T> {
        let loc = self.loc();
        self.errors.push(format!("{loc}: error: {msg}"));
        Err(())
    }

    fn warn(&mut self, msg: &str) {
        let loc = self.loc();
        self.warnings.push(format!("{loc}: warning: {msg}"));
    }

    fn expect(&mut self, p: &str) -> R<()> {
        if self.accept(p) {
            return Ok(());
        }
        let got = self.peek().map(|t| t.text()).unwrap_or_else(|| "end of file".into());
        self.error(&format!("expected '{p}' before '{got}'"))
    }

    fn ident(&mut self) -> R<String> {
        match self.peek().and_then(|t| t.ident().map(String::from)) {
            Some(n) => {
                self.next();
                Ok(n)
            }
            None => {
                let got = self.peek().map(|t| t.text()).unwrap_or_else(|| "end of file".into());
                self.error(&format!("expected identifier before '{got}'"))
            }
        }
    }

    /// Skip to the end of the current statement or declaration after an error.
    fn recover(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.peek().cloned() {
            if t.is("{") || t.is("(") || t.is("[") {
                depth += 1;
            } else if t.is("}") || t.is(")") || t.is("]") {
                if depth == 0 {
                    if t.is("}") {
                        return;
                    }
                } else {
                    depth -= 1;
                }
                if depth == 0 && t.is("}") {
                    self.next();
                    return;
                }
            } else if t.is(";") && depth <= 0 {
                self.next();
                return;
            }
            self.next();
        }
    }

    // ---- scopes ------------------------------------------------------------------------

    fn lookup(&self, name: &str) -> Option<Sym> {
        self.scopes.iter().rev().find_map(|s| s.syms.get(name).cloned())
    }

    fn lookup_tag(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.tags.get(name).copied())
    }

    fn bind(&mut self, name: &str, s: Sym) {
        self.scopes.last_mut().unwrap().syms.insert(name.into(), s);
    }

    fn is_typedef_name(&self, w: &str) -> bool {
        matches!(self.lookup(w), Some(Sym::Typedef(_)))
    }

    fn starts_decl(&mut self) -> bool {
        let Some(w) = self.peek().and_then(|t| t.ident().map(String::from)) else { return false };
        is_decl_start_word(&w) || self.is_typedef_name(&w) || w == "__typeof__" || w == "typeof"
    }

    fn size(&self, t: &Ty) -> u32 {
        size_of(t, &self.unit.structs)
    }

    fn tyname(&self, t: &Ty) -> String {
        name(t, &self.unit.structs)
    }

    // ---- declarations --------------------------------------------------------------------

    fn skip_attribute(&mut self) -> R<()> {
        // __attribute__((...)) / __asm__("name") on declarations
        loop {
            if self.accept_word("__attribute__") || self.accept_word("__attribute") {
                self.expect("(")?;
                let mut depth = 1;
                while depth > 0 {
                    let Some(t) = self.next() else { return self.error("unterminated __attribute__") };
                    if t.is("(") {
                        depth += 1;
                    } else if t.is(")") {
                        depth -= 1;
                    }
                }
            } else if self.is_word("__asm__") || self.is_word("asm") || self.is_word("__asm") {
                self.next();
                self.expect("(")?;
                while !self.accept(")") {
                    if self.next().is_none() {
                        break;
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Declaration specifiers: storage class, qualifiers and the base type.
    fn specifiers(&mut self) -> R<(Ty, Spec)> {
        let mut sp = Spec::default();
        let (mut signed, mut unsigned, mut short, mut long, mut char_, mut int, mut float, mut double, mut void, mut bool_) =
            (false, false, false, 0, false, false, false, false, false, false);
        let mut other: Option<Ty> = None;
        let mut any = false;
        loop {
            let Some(w) = self.peek().and_then(|t| t.ident().map(String::from)) else { break };
            match w.as_str() {
                "typedef" => sp.storage = Storage::Typedef,
                "extern" => sp.storage = Storage::Extern,
                "static" => sp.storage = Storage::Static,
                "auto" => sp.storage = Storage::Auto,
                "register" => sp.storage = Storage::Register,
                "_Thread_local" | "__thread" => {}
                "inline" | "__inline" | "__inline__" | "_Noreturn" | "__extension__" => sp.inline = true,
                "const" | "__const" | "__const__" => sp.konst = true,
                "volatile" | "__volatile__" | "__volatile" => sp.volatile = true,
                "restrict" | "__restrict" | "__restrict__" | "_Complex" => {}
                "signed" | "__signed__" | "__signed" => signed = true,
                "unsigned" => unsigned = true,
                "short" => short = true,
                "long" => long += 1,
                "char" => char_ = true,
                "int" => int = true,
                "float" => float = true,
                "double" => double = true,
                "void" => void = true,
                "_Bool" => bool_ = true,
                "__builtin_va_list" => other = Some(Ty::ptr(Ty::Char)),
                "__attribute__" | "__attribute" => {
                    self.skip_attribute()?;
                    continue;
                }
                "_Alignas" => {
                    self.next();
                    self.expect("(")?;
                    let mut d = 1;
                    while d > 0 {
                        let Some(t) = self.next() else { break };
                        if t.is("(") {
                            d += 1;
                        } else if t.is(")") {
                            d -= 1;
                        }
                    }
                    continue;
                }
                "struct" | "union" => {
                    self.next();
                    other = Some(self.struct_spec(w == "union")?);
                    any = true;
                    continue;
                }
                "enum" => {
                    self.next();
                    self.enum_spec()?;
                    other = Some(Ty::Int);
                    any = true;
                    continue;
                }
                "typeof" | "__typeof__" | "__typeof" => {
                    self.next();
                    self.expect("(")?;
                    let t = if self.starts_decl() {
                        self.type_name()?
                    } else {
                        let e = self.expr()?;
                        e.ty
                    };
                    self.expect(")")?;
                    other = Some(t);
                    any = true;
                    continue;
                }
                _ => {
                    // a typedef name, unless a type was already given
                    let have_type = other.is_some() || signed || unsigned || short || long > 0 || char_ || int || float || double || void || bool_;
                    if !have_type {
                        if let Some(Sym::Typedef(t)) = self.lookup(&w) {
                            self.next();
                            other = Some(t);
                            any = true;
                            continue;
                        }
                    }
                    break;
                }
            }
            any = true;
            self.next();
        }
        if !any {
            return self.error("expected a type");
        }
        let ty = if let Some(t) = other {
            t
        } else if void {
            Ty::Void
        } else if bool_ {
            Ty::Bool
        } else if char_ {
            if unsigned {
                Ty::UChar
            } else if signed {
                Ty::SChar
            } else {
                Ty::Char
            }
        } else if float {
            Ty::Float
        } else if double {
            if long > 0 { Ty::LDouble } else { Ty::Double }
        } else if short {
            if unsigned { Ty::UShort } else { Ty::Short }
        } else if long >= 2 {
            if unsigned { Ty::ULLong } else { Ty::LLong }
        } else if long == 1 {
            if unsigned { Ty::ULong } else { Ty::Long }
        } else if unsigned {
            Ty::UInt
        } else {
            Ty::Int
        };
        Ok((ty, sp))
    }

    fn struct_spec(&mut self, union: bool) -> R<Ty> {
        self.skip_attribute()?;
        let tag = self.peek().and_then(|t| t.ident().map(String::from));
        if tag.is_some() {
            self.next();
        }
        let has_body = self.is("{");
        let idx = match &tag {
            Some(t) => {
                // a body or `struct x;` declares in the current scope
                let local = self.scopes.last().unwrap().tags.get(t).copied();
                let found = if has_body || self.is(";") { local } else { self.lookup_tag(t) };
                match found {
                    Some(i) if !(has_body && self.unit.structs[i].complete) => i,
                    Some(i) if !has_body => i,
                    _ => {
                        self.unit.structs.push(StructDef { tag: t.clone(), union, ..Default::default() });
                        let i = self.unit.structs.len() - 1;
                        self.scopes.last_mut().unwrap().tags.insert(t.clone(), i);
                        i
                    }
                }
            }
            None => {
                self.unit.structs.push(StructDef { tag: String::new(), union, ..Default::default() });
                self.unit.structs.len() - 1
            }
        };
        if !has_body {
            return Ok(Ty::Struct(idx));
        }
        self.next(); // {
        let mut members: Vec<Member> = Vec::new();
        let mut off = 0u32;
        let mut align = 1u32;
        let mut size = 0u32;
        // current bit-field unit: (byte offset, bits used)
        let mut unit: Option<(u32, u32)> = None;
        while !self.accept("}") {
            if self.peek().is_none() {
                return self.error("unterminated struct");
            }
            if self.accept_word("_Static_assert") {
                self.static_assert()?;
                continue;
            }
            let (base, _) = self.specifiers()?;
            // anonymous struct/union member
            if self.is(";") {
                self.next();
                if let Ty::Struct(si) = base {
                    let (s, a) = size_align(&base, &self.unit.structs);
                    let o = if union { 0 } else { align_up(off, a) };
                    let inner: Vec<Member> = self.unit.structs[si].members.clone();
                    for m in inner {
                        members.push(Member { offset: m.offset + o, ..m });
                    }
                    unit = None;
                    if union {
                        size = size.max(s);
                    } else {
                        off = o + s;
                        size = off;
                    }
                    align = align.max(a);
                }
                continue;
            }
            loop {
                let (name, ty) = if self.is(":") { (None, base.clone()) } else { self.declarator(base.clone())? };
                self.skip_attribute()?;
                if self.accept(":") {
                    let w = self.const_expr()? as u32;
                    if w > 32 {
                        return self.error("bit-field width exceeds its type");
                    }
                    // pack into 32-bit units
                    let base_off = if union { 0 } else { off };
                    let (uoff, used) = match unit {
                        Some((u, used)) if used + w <= 32 && w > 0 && !union => (u, used),
                        _ => (align_up(base_off, 2), 0),
                    };
                    if w == 0 {
                        unit = None;
                        if !union {
                            off = align_up(off, 4);
                        }
                    } else {
                        if let Some(n) = name {
                            members.push(Member { name: n, ty: ty.clone(), offset: uoff, bits: Some((used as u8, w as u8)) });
                        }
                        unit = Some((uoff, used + w));
                        let end = uoff + (used + w).div_ceil(8);
                        if union {
                            size = size.max(4);
                        } else {
                            off = off.max(end);
                            size = off;
                        }
                        align = align.max(2);
                    }
                } else {
                    unit = None;
                    let (s, a) = size_align(&ty, &self.unit.structs);
                    let o = if union { 0 } else { align_up(off, a) };
                    if let Some(n) = name {
                        members.push(Member { name: n, ty: ty.clone(), offset: o, bits: None });
                    }
                    align = align.max(a);
                    if union {
                        size = size.max(s);
                    } else {
                        off = o + s;
                        size = off;
                    }
                }
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(";")?;
        }
        self.skip_attribute()?;
        // bit-field units occupy 4 bytes when read as longs: keep them inside the struct
        let need = members.iter().filter(|m| m.bits.is_some()).map(|m| m.offset + 4).max().unwrap_or(0);
        let size = align_up(size.max(need), align);
        let d = &mut self.unit.structs[idx];
        d.members = members;
        d.size = size;
        d.align = align;
        d.complete = true;
        Ok(Ty::Struct(idx))
    }

    fn enum_spec(&mut self) -> R<()> {
        self.skip_attribute()?;
        if self.peek().is_some_and(|t| t.ident().is_some()) {
            self.next();
        }
        if !self.accept("{") {
            return Ok(());
        }
        let mut v: i64 = 0;
        while !self.accept("}") {
            let name = self.ident()?;
            if self.accept("=") {
                v = self.const_expr()?;
            }
            self.bind(&name, Sym::Enum(v));
            v += 1;
            if !self.accept(",") {
                self.expect("}")?;
                break;
            }
        }
        Ok(())
    }

    fn static_assert(&mut self) -> R<()> {
        self.expect("(")?;
        let v = self.const_expr()?;
        let mut msg = String::new();
        if self.accept(",") {
            if let Some(Tok::Str(s)) = self.peek().map(|t| t.tok.clone()) {
                msg = s;
                self.next();
            }
        }
        self.expect(")")?;
        self.expect(";")?;
        if v == 0 {
            return self.error(&format!("static assertion failed {msg}"));
        }
        Ok(())
    }

    /// Is a '(' in a declarator the start of a nested declarator (vs. parameters)?
    fn nested_paren(&mut self) -> bool {
        let Some(t) = self.peek_at(1).cloned() else { return false };
        if t.is("*") || t.is("(") || t.is("[") || t.is("^") {
            return true;
        }
        match t.ident() {
            Some(w) => !(is_decl_start_word(w) || self.is_typedef_name(w)),
            None => false,
        }
    }

    fn declarator_ops(&mut self, ops: &mut Vec<Deriv>) -> R<Option<String>> {
        let mut ptrs = 0;
        while self.accept("*") {
            ptrs += 1;
            loop {
                if self.accept_word("const") || self.accept_word("volatile") || self.accept_word("restrict") || self.accept_word("__restrict") || self.accept_word("__restrict__") || self.accept_word("__const") {
                    continue;
                }
                if self.is_word("__attribute__") {
                    self.skip_attribute()?;
                    continue;
                }
                break;
            }
        }
        let mut inner: Vec<Deriv> = Vec::new();
        let name = if self.is("(") && self.nested_paren() {
            self.next();
            let n = self.declarator_ops(&mut inner)?;
            self.expect(")")?;
            n
        } else if let Some(n) = self.peek().and_then(|t| t.ident().map(String::from)) {
            if is_decl_start_word(&n) {
                None
            } else {
                self.next();
                Some(n)
            }
        } else {
            None
        };
        self.skip_attribute()?;
        let mut suffix: Vec<Deriv> = Vec::new();
        loop {
            if self.accept("[") {
                while self.accept_word("static") || self.accept_word("const") || self.accept_word("restrict") {}
                if self.accept("]") {
                    suffix.push(Deriv::Array(None));
                } else if self.is("*") && self.peek_at(1).is_some_and(|t| t.is("]")) {
                    self.next();
                    self.next();
                    suffix.push(Deriv::Array(None));
                } else {
                    let n = self.const_expr()?;
                    if n < 0 {
                        return self.error("size of array is negative");
                    }
                    self.expect("]")?;
                    suffix.push(Deriv::Array(Some(n as u32)));
                }
            } else if self.is("(") {
                self.next();
                let (f, names) = self.params()?;
                suffix.push(Deriv::Func(f, names));
            } else {
                break;
            }
        }
        for _ in 0..ptrs {
            ops.push(Deriv::Ptr);
        }
        for s in suffix.into_iter().rev() {
            ops.push(s);
        }
        ops.extend(inner);
        Ok(name)
    }

    fn apply(&mut self, base: Ty, ops: Vec<Deriv>) -> R<Ty> {
        let mut t = base;
        for op in ops {
            t = match op {
                Deriv::Ptr => Ty::ptr(t),
                Deriv::Array(n) => {
                    if t.is_func() {
                        return self.error("declaration of array of functions");
                    }
                    Ty::Array(Rc::new(t), n)
                }
                Deriv::Func(f, _) => {
                    if t.is_array() || t.is_func() {
                        return self.error("function cannot return an array or function");
                    }
                    let mut f = (*f).clone();
                    f.ret = t;
                    Ty::Func(Rc::new(f))
                }
            };
        }
        Ok(t)
    }

    fn declarator(&mut self, base: Ty) -> R<(Option<String>, Ty)> {
        let mut ops = Vec::new();
        let n = self.declarator_ops(&mut ops)?;
        let t = self.apply(base, ops)?;
        Ok((n, t))
    }

    /// A declarator that also returns the parameter names of a function type.
    fn declarator_with_params(&mut self, base: Ty) -> R<(Option<String>, Ty, Vec<Option<String>>)> {
        let mut ops = Vec::new();
        let n = self.declarator_ops(&mut ops)?;
        // the parameter names belong to the outermost function derivation
        let names = ops.iter().rev().find_map(|d| match d {
            Deriv::Func(_, names) => Some(names.clone()),
            _ => None,
        });
        // (the outermost derivation applied last is the declared function itself)
        let names = match ops.last() {
            Some(Deriv::Func(_, names)) => names.clone(),
            _ => names.unwrap_or_default(),
        };
        let t = self.apply(base, ops)?;
        Ok((n, t, names))
    }

    fn params(&mut self) -> R<(Rc<FnTy>, Vec<Option<String>>)> {
        let mut params = Vec::new();
        let mut names = Vec::new();
        let mut variadic = false;
        if self.accept(")") {
            return Ok((Rc::new(FnTy { ret: Ty::Int, params, variadic: false, proto: false }), names));
        }
        if self.is_word("void") && self.peek_at(1).is_some_and(|t| t.is(")")) {
            self.next();
            self.next();
            return Ok((Rc::new(FnTy { ret: Ty::Int, params, variadic: false, proto: true }), names));
        }
        // K&R identifier list: f(a, b)
        if self.peek().and_then(|t| t.ident().map(String::from)).is_some_and(|w| !is_decl_start_word(&w) && !self.is_typedef_name(&w)) {
            loop {
                let n = self.ident()?;
                params.push(Ty::Int);
                names.push(Some(n));
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(")")?;
            return Ok((Rc::new(FnTy { ret: Ty::Int, params, variadic: false, proto: false }), names));
        }
        self.scopes.push(Scope::default());
        let r = (|| -> R<()> {
            loop {
                if self.accept("...") {
                    variadic = true;
                    break;
                }
                let (base, _) = self.specifiers()?;
                let (n, t) = self.declarator(base)?;
                self.skip_attribute()?;
                let t = match t {
                    Ty::Array(e, _) => Ty::Ptr(e),
                    Ty::Func(_) => Ty::ptr(t),
                    t => t,
                };
                params.push(t);
                names.push(n);
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(")")
        })();
        self.scopes.pop();
        r?;
        Ok((Rc::new(FnTy { ret: Ty::Int, params, variadic, proto: true }), names))
    }

    /// A type name (casts, sizeof).
    fn type_name(&mut self) -> R<Ty> {
        let (base, _) = self.specifiers()?;
        let (n, t) = self.declarator(base)?;
        if n.is_some() {
            return self.error("unexpected identifier in type name");
        }
        Ok(t)
    }

    fn const_expr(&mut self) -> R<i64> {
        let e = self.cond_expr()?;
        match e.const_int() {
            Some(v) => Ok(v),
            None => self.error("expression is not an integer constant"),
        }
    }

    // ---- top level -------------------------------------------------------------------------

    pub fn translation_unit(&mut self) {
        while self.peek().is_some() {
            if self.accept(";") {
                continue;
            }
            if self.is_word("_Static_assert") {
                self.next();
                if self.static_assert().is_err() {
                    self.recover();
                }
                continue;
            }
            if (self.is_word("__asm__") || self.is_word("asm")) && self.peek_at(1).is_some_and(|t| t.is("(")) {
                if self.toplevel_asm().is_err() {
                    self.recover();
                }
                continue;
            }
            if self.external_decl().is_err() {
                self.recover();
            }
            if self.errors.len() > 50 {
                break;
            }
        }
        // tentative definitions and used-but-undefined statics
        let names: Vec<String> = self.globals.keys().cloned().collect();
        for n in names {
            let g = &self.globals[&n];
            if g.idx.is_none() && g.defined {
                // defined without initializer: emit as bss
                let ty = g.ty.clone();
                let global = g.global;
                self.unit.globals.push(Global { asm_name: n.clone(), ty, global, init: None, readonly: false });
            }
        }
    }

    fn toplevel_asm(&mut self) -> R<()> {
        self.next();
        self.expect("(")?;
        let mut text = String::new();
        while let Some(Tok::Str(s)) = self.peek().map(|t| t.tok.clone()) {
            self.next();
            let v = decode(&s[1..s.len() - 1]);
            text.extend(v.iter().map(|c| char::from_u32(*c).unwrap_or('?')));
        }
        self.expect(")")?;
        self.accept(";");
        self.unit.globals.push(Global { asm_name: format!("\u{1}{text}"), ty: Ty::Void, global: false, init: None, readonly: false });
        Ok(())
    }

    fn asm_name(name: &str) -> String {
        format!("_{name}")
    }

    fn external_decl(&mut self) -> R<()> {
        let (base, sp) = if self.starts_decl() {
            self.specifiers()?
        } else {
            // implicit int (old-style function definitions)
            self.warn("type defaults to 'int'");
            (Ty::Int, Spec::default())
        };
        if self.accept(";") {
            return Ok(());
        }
        let mut first = true;
        loop {
            let (name, ty, pnames) = self.declarator_with_params(base.clone())?;
            self.skip_attribute()?;
            let Some(name) = name else { return self.error("expected a declarator") };
            if sp.storage == Storage::Typedef {
                self.bind(&name, Sym::Typedef(ty));
            } else if let Ty::Func(f) = &ty {
                let asm = Self::asm_name(&name);
                self.declare_global(&name, &asm, ty.clone(), sp.storage != Storage::Static, false);
                // a definition?
                if first && (self.is("{") || (!f.proto && !f.params.is_empty() && self.starts_decl())) {
                    return self.function(name, ty, pnames, sp.storage != Storage::Static);
                }
            } else {
                let asm = Self::asm_name(&name);
                let has_init = self.is("=");
                let defined = sp.storage != Storage::Extern || has_init;
                let mut ty = ty;
                if has_init {
                    self.next();
                    let (items, t) = self.initializer(&ty, true)?;
                    ty = t;
                    self.declare_global(&name, &asm, ty.clone(), sp.storage != Storage::Static, defined);
                    let data = self.static_data(&items)?;
                    self.define_global(&asm, ty, sp.storage != Storage::Static, Some(data), sp.konst);
                } else {
                    self.declare_global(&name, &asm, ty, sp.storage != Storage::Static, defined);
                }
            }
            first = false;
            if !self.accept(",") {
                break;
            }
        }
        self.expect(";")
    }

    fn declare_global(&mut self, name: &str, asm: &str, ty: Ty, global: bool, defined: bool) {
        // merge with a previous declaration (complete array sizes, etc.)
        let ty = match self.globals.get(asm) {
            Some(g) => {
                let merged = match (&g.ty, &ty) {
                    (Ty::Array(_, Some(_)), Ty::Array(_, None)) => g.ty.clone(),
                    _ => ty,
                };
                merged
            }
            None => ty,
        };
        let e = self.globals.entry(asm.into()).or_insert(GlobalInfo { idx: None, ty: ty.clone(), defined: false, global });
        e.ty = ty.clone();
        e.defined |= defined;
        if !global {
            e.global = false;
        }
        self.scopes[0].syms.insert(name.into(), Sym::Global(asm.into(), ty));
        if self.scopes.len() > 1 {
            self.scopes.last_mut().unwrap().syms.insert(name.into(), Sym::Global(asm.into(), self.globals[asm].ty.clone()));
        }
    }

    fn define_global(&mut self, asm: &str, ty: Ty, global: bool, init: Option<Vec<DataItem>>, readonly: bool) {
        let idx = self.unit.globals.len();
        let g = self.globals.entry(asm.into()).or_insert(GlobalInfo { idx: None, ty: ty.clone(), defined: true, global });
        if g.idx.is_some() {
            let loc = self.loc();
            self.errors.push(format!("{loc}: error: redefinition of '{}'", &asm[1..]));
            return;
        }
        g.idx = Some(idx);
        g.defined = true;
        let global = g.global && global;
        self.unit.globals.push(Global { asm_name: asm.into(), ty, global, init, readonly });
    }

    // ---- functions -------------------------------------------------------------------------

    fn function(&mut self, name: String, ty: Ty, pnames: Vec<Option<String>>, global: bool) -> R<()> {
        let Ty::Func(ft) = &ty else { unreachable!() };
        let mut ft = (**ft).clone();
        // K&R parameter declarations
        let mut knr: BTreeMap<String, Ty> = BTreeMap::new();
        while !self.is("{") {
            let (base, _) = self.specifiers()?;
            loop {
                let (n, t) = self.declarator(base.clone())?;
                if let Some(n) = n {
                    let t = match t {
                        Ty::Array(e, _) => Ty::Ptr(e),
                        t => t,
                    };
                    knr.insert(n, t);
                }
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(";")?;
        }
        if !knr.is_empty() {
            for (i, n) in pnames.iter().enumerate() {
                if let Some(t) = n.as_ref().and_then(|n| knr.get(n)) {
                    // old-style parameters arrive promoted
                    ft.params[i] = match t {
                        Ty::Float => Ty::Double,
                        t => t.clone(),
                    };
                }
            }
        }
        let sret = ft.ret.is_struct();
        let mut ctx = FnCtx {
            locals: Vec::new(),
            labels: 0,
            loop_weight: 1,
            switches: Vec::new(),
            ret: ft.ret.clone(),
            name: name.clone(),
            va_off: 0,
            breakable: 0,
            continuable: 0,
        };
        self.scopes.push(Scope::default());
        let mut off: i32 = if sret { 12 } else { 8 };
        for (i, pt) in ft.params.iter().enumerate() {
            let pn = pnames.get(i).cloned().flatten().unwrap_or_default();
            let size = self.size(pt).max(1);
            let slot = align_up(size, 4) as i32;
            // small integers sit at the end of their 4-byte slot (big-endian)
            let real_off = if size < 4 { off + 4 - size as i32 } else { off };
            ctx.locals.push(Local { name: pn.clone(), ty: pt.clone(), param_off: Some(real_off), addr_taken: false, volatile: false, uses: 0 });
            if !pn.is_empty() {
                self.bind(&pn, Sym::Local(ctx.locals.len() - 1));
            }
            off += slot;
        }
        ctx.va_off = off;
        self.f = Some(ctx);
        let fty = Ty::Func(Rc::new(ft));
        let asm = Self::asm_name(&name);
        self.declare_global(&name, &asm, fty.clone(), global, true);
        if self.globals.get(&asm).is_some_and(|g| g.idx.is_some()) {
            let loc = self.loc();
            self.errors.push(format!("{loc}: error: redefinition of '{name}'"));
        }
        if let Some(g) = self.globals.get_mut(&asm) {
            g.idx = Some(usize::MAX);
        }
        let body = self.compound(false);
        self.scopes.pop();
        let ctx = self.f.take().unwrap();
        let body = body?;
        let global = self.globals.get(&asm).is_none_or(|g| g.global) && global;
        self.unit.functions.push(Function { name, asm_name: asm, global, ty: fty, locals: ctx.locals, body, sret, labels: ctx.labels });
        Ok(())
    }

    fn new_label(&mut self) -> usize {
        let f = self.f.as_mut().unwrap();
        f.labels += 1;
        f.labels
    }

    fn add_local(&mut self, name: &str, ty: Ty, volatile: bool) -> usize {
        let f = self.f.as_mut().unwrap();
        f.locals.push(Local { name: name.into(), ty, param_off: None, addr_taken: false, volatile, uses: 0 });
        f.locals.len() - 1
    }

    // ---- statements ------------------------------------------------------------------------

    fn compound(&mut self, new_scope: bool) -> R<Stmt> {
        self.expect("{")?;
        if new_scope {
            self.scopes.push(Scope::default());
        }
        let mut v = Vec::new();
        while !self.accept("}") {
            if self.peek().is_none() {
                if new_scope {
                    self.scopes.pop();
                }
                return self.error("expected '}' at end of input");
            }
            match self.block_item() {
                Ok(s) => v.push(s),
                Err(()) => {
                    self.recover();
                    if self.errors.len() > 50 {
                        break;
                    }
                }
            }
        }
        if new_scope {
            self.scopes.pop();
        }
        Ok(Stmt::Block(v))
    }

    fn block_item(&mut self) -> R<Stmt> {
        // a label `name:` looks like an expression start
        if self.peek().is_some_and(|t| t.ident().is_some()) && self.peek_at(1).is_some_and(|t| t.is(":")) {
            let w = self.peek().unwrap().ident().unwrap().to_string();
            if w != "default" && !self.is_typedef_name(&w) {
                return self.stmt();
            }
        }
        if self.is_word("_Static_assert") {
            self.next();
            self.static_assert()?;
            return Ok(Stmt::Empty);
        }
        if self.starts_decl() {
            return self.local_decl();
        }
        self.stmt()
    }

    fn local_decl(&mut self) -> R<Stmt> {
        let (base, sp) = self.specifiers()?;
        let mut out = Vec::new();
        if self.accept(";") {
            return Ok(Stmt::Empty);
        }
        loop {
            let (name, ty) = self.declarator(base.clone())?;
            self.skip_attribute()?;
            let Some(name) = name else { return self.error("expected a declarator") };
            if sp.storage == Storage::Typedef {
                self.bind(&name, Sym::Typedef(ty));
            } else if ty.is_func() || sp.storage == Storage::Extern {
                let asm = Self::asm_name(&name);
                self.declare_global(&name, &asm, ty.clone(), true, false);
                self.bind(&name, Sym::Global(asm, ty));
            } else if sp.storage == Storage::Static {
                self.static_count += 1;
                let fname = self.f.as_ref().map(|f| f.name.clone()).unwrap_or_default();
                let asm = format!("S{}_{fname}_{name}", self.static_count);
                let mut ty = ty;
                let data = if self.accept("=") {
                    let (items, t) = self.initializer(&ty, true)?;
                    ty = t;
                    Some(self.static_data(&items)?)
                } else {
                    None
                };
                self.bind(&name, Sym::Global(asm.clone(), ty.clone()));
                self.unit.globals.push(Global { asm_name: asm, ty, global: false, init: data, readonly: sp.konst });
            } else {
                let mut ty = ty;
                let mut init = None;
                if self.accept("=") {
                    let (items, t) = self.initializer(&ty, false)?;
                    ty = t;
                    init = Some(items);
                }
                if matches!(ty, Ty::Array(_, None)) {
                    return self.error(&format!("array size missing in '{name}'"));
                }
                if ty.is_void() {
                    return self.error(&format!("variable '{name}' declared void"));
                }
                let id = self.add_local(&name, ty.clone(), sp.volatile);
                self.bind(&name, Sym::Local(id));
                if let Some(items) = init {
                    let zero = !ty.is_scalar();
                    out.push(Stmt::Init(id, items, zero));
                }
            }
            if !self.accept(",") {
                break;
            }
        }
        self.expect(";")?;
        Ok(match out.len() {
            0 => Stmt::Empty,
            1 => out.pop().unwrap(),
            _ => Stmt::Block(out),
        })
    }

    fn cond_of(&mut self, e: Expr) -> R<Expr> {
        let e = self.rv(e);
        if !e.ty.is_scalar() {
            return self.error("used a value that cannot be converted to a condition");
        }
        Ok(e)
    }

    fn stmt(&mut self) -> R<Stmt> {
        let Some(t) = self.peek().cloned() else { return self.error("expected a statement") };
        if t.is("{") {
            return self.compound(true);
        }
        if t.is(";") {
            self.next();
            return Ok(Stmt::Empty);
        }
        if let Some(w) = t.ident() {
            match w {
                "if" => {
                    self.next();
                    self.expect("(")?;
                    let c = self.expr()?;
                    let c = self.cond_of(c)?;
                    self.expect(")")?;
                    let a = self.stmt()?;
                    let b = if self.accept_word("else") { Some(Box::new(self.stmt()?)) } else { None };
                    return Ok(Stmt::If(c, Box::new(a), b));
                }
                "while" => {
                    self.next();
                    self.expect("(")?;
                    let c = self.expr()?;
                    let c = self.cond_of(c)?;
                    self.expect(")")?;
                    let body = self.loop_body()?;
                    return Ok(Stmt::While(c, Box::new(body)));
                }
                "do" => {
                    self.next();
                    let body = self.loop_body()?;
                    if !self.accept_word("while") {
                        return self.error("expected 'while' in do-while loop");
                    }
                    self.expect("(")?;
                    let c = self.expr()?;
                    let c = self.cond_of(c)?;
                    self.expect(")")?;
                    self.expect(";")?;
                    return Ok(Stmt::DoWhile(Box::new(body), c));
                }
                "for" => {
                    self.next();
                    self.expect("(")?;
                    self.scopes.push(Scope::default());
                    let r = (|| -> R<Stmt> {
                        let init = if self.accept(";") {
                            None
                        } else if self.starts_decl() {
                            Some(Box::new(self.local_decl()?))
                        } else {
                            let e = self.expr()?;
                            self.expect(";")?;
                            Some(Box::new(Stmt::Expr(e)))
                        };
                        let cond = if self.is(";") {
                            None
                        } else {
                            let c = self.expr()?;
                            Some(self.cond_of(c)?)
                        };
                        self.expect(";")?;
                        let step = if self.is(")") { None } else { Some(self.expr()?) };
                        self.expect(")")?;
                        let body = self.loop_body()?;
                        Ok(Stmt::For(init, cond, step, Box::new(body)))
                    })();
                    self.scopes.pop();
                    return r;
                }
                "switch" => {
                    self.next();
                    self.expect("(")?;
                    let e = self.expr()?;
                    let e = self.rv(e);
                    if !e.ty.is_integer() {
                        return self.error("switch quantity not an integer");
                    }
                    let e = self.promote(e);
                    self.expect(")")?;
                    self.f.as_mut().unwrap().switches.push((Vec::new(), None));
                    self.f.as_mut().unwrap().breakable += 1;
                    let body = self.stmt();
                    self.f.as_mut().unwrap().breakable -= 1;
                    let (cases, def) = self.f.as_mut().unwrap().switches.pop().unwrap();
                    return Ok(Stmt::Switch(e, Box::new(body?), cases, def));
                }
                "case" => {
                    self.next();
                    let v = self.const_expr()?;
                    let hi = if self.accept("...") { Some(self.const_expr()?) } else { None };
                    self.expect(":")?;
                    let l = self.new_label();
                    let Some(sw) = self.f.as_mut().unwrap().switches.last_mut() else { return self.error("case label not within a switch statement") };
                    for x in v..=hi.unwrap_or(v) {
                        if sw.0.iter().any(|(c, _)| *c == x) {
                            return self.error(&format!("duplicate case value {x}"));
                        }
                        sw.0.push((x, l));
                    }
                    let s = if self.is("}") { Stmt::Empty } else { self.stmt()? };
                    return Ok(Stmt::Block(vec![Stmt::CaseLabel(l), s]));
                }
                "default" => {
                    self.next();
                    self.expect(":")?;
                    let l = self.new_label();
                    let Some(sw) = self.f.as_mut().unwrap().switches.last_mut() else { return self.error("'default' label not within a switch statement") };
                    if sw.1.is_some() {
                        return self.error("multiple default labels in one switch");
                    }
                    sw.1 = Some(l);
                    let s = if self.is("}") { Stmt::Empty } else { self.stmt()? };
                    return Ok(Stmt::Block(vec![Stmt::CaseLabel(l), s]));
                }
                "break" => {
                    self.next();
                    self.expect(";")?;
                    if self.f.as_ref().unwrap().breakable == 0 {
                        return self.error("break statement not within loop or switch");
                    }
                    return Ok(Stmt::Break);
                }
                "continue" => {
                    self.next();
                    self.expect(";")?;
                    if self.f.as_ref().unwrap().continuable == 0 {
                        return self.error("continue statement not within a loop");
                    }
                    return Ok(Stmt::Continue);
                }
                "return" => {
                    self.next();
                    let ret = self.f.as_ref().unwrap().ret.clone();
                    if self.accept(";") {
                        if !ret.is_void() {
                            self.warn("'return' with no value, in function returning non-void");
                        }
                        return Ok(Stmt::Return(None));
                    }
                    let e = self.expr()?;
                    self.expect(";")?;
                    if ret.is_void() {
                        let e = self.rv(e);
                        if !e.ty.is_void() {
                            self.warn("'return' with a value, in function returning void");
                        }
                        return Ok(Stmt::Block(vec![Stmt::Expr(e), Stmt::Return(None)]));
                    }
                    let e = self.assign_conv(e, &ret, "return")?;
                    return Ok(Stmt::Return(Some(e)));
                }
                "goto" => {
                    self.next();
                    let l = self.ident()?;
                    self.expect(";")?;
                    return Ok(Stmt::Goto(l));
                }
                "__asm__" | "asm" | "__asm" => {
                    self.next();
                    while self.accept_word("volatile") || self.accept_word("__volatile__") {}
                    self.expect("(")?;
                    let mut text = String::new();
                    while let Some(Tok::Str(s)) = self.peek().map(|t| t.tok.clone()) {
                        self.next();
                        let v = decode(&s[1..s.len() - 1]);
                        text.extend(v.iter().map(|c| char::from_u32(*c).unwrap_or('?')));
                    }
                    // operands are not supported: skip them
                    let mut depth = 0;
                    while !(self.is(")") && depth == 0) {
                        let Some(t) = self.next() else { break };
                        if t.is("(") {
                            depth += 1;
                        } else if t.is(")") {
                            depth -= 1;
                        }
                    }
                    self.expect(")")?;
                    self.expect(";")?;
                    return Ok(Stmt::Label(format!("\u{1}{text}")));
                }
                _ => {
                    if self.peek_at(1).is_some_and(|t| t.is(":")) {
                        let l = self.ident()?;
                        self.next();
                        let s = if self.is("}") { Stmt::Empty } else { self.stmt()? };
                        return Ok(Stmt::Block(vec![Stmt::Label(l), s]));
                    }
                }
            }
        }
        let e = self.expr()?;
        self.expect(";")?;
        Ok(Stmt::Expr(e))
    }

    fn loop_body(&mut self) -> R<Stmt> {
        let f = self.f.as_mut().unwrap();
        f.loop_weight = (f.loop_weight * 8).min(1 << 20);
        f.breakable += 1;
        f.continuable += 1;
        let r = self.stmt();
        let f = self.f.as_mut().unwrap();
        f.loop_weight = (f.loop_weight / 8).max(1);
        f.breakable -= 1;
        f.continuable -= 1;
        r
    }

    // ---- initializers ------------------------------------------------------------------------

    /// Parse an initializer for `ty`; returns the scalar items and the completed type
    /// (array sizes from the initializer).
    fn initializer(&mut self, ty: &Ty, is_static: bool) -> R<(Vec<InitItem>, Ty)> {
        let mut items = Vec::new();
        let mut ty = ty.clone();
        // a string literal for a char array
        if let Ty::Array(elem, n) = &ty {
            let elem_size = self.size(elem);
            if elem.is_integer() && elem_size <= 4 {
                let paren = self.is("{") && self.peek_at(1).is_some_and(|t| matches!(t.tok, Tok::Str(_))) && self.peek_at(2).is_some_and(|t| t.is("}"));
                if paren {
                    self.next();
                }
                if let Some(Tok::Str(_)) = self.peek().map(|t| t.tok.clone()) {
                    let (units, _) = self.string_units()?;
                    let len = match n {
                        Some(n) => *n,
                        None => units.len() as u32 + 1,
                    };
                    for (i, u) in units.iter().enumerate().take(len as usize) {
                        items.push(InitItem { off: i as u32 * elem_size, ty: (**elem).clone(), bits: None, val: Expr::int(*u as i64) });
                    }
                    if paren {
                        self.expect("}")?;
                    }
                    return Ok((items, Ty::Array(elem.clone(), Some(len))));
                }
            }
        }
        if self.is("{") {
            self.next();
            let end = self.init_list(&mut ty, 0, &mut items, is_static, true)?;
            let _ = end;
            self.expect("}")?;
        } else {
            let e = self.assign_expr()?;
            if ty.is_struct() {
                let e = self.rv(e);
                if e.ty != ty {
                    return self.error("invalid initializer");
                }
                items.push(InitItem { off: 0, ty: ty.clone(), bits: None, val: e });
            } else if ty.is_array() {
                return self.error("array must be initialized with a brace-enclosed initializer");
            } else {
                let v = self.assign_conv(e, &ty, "initialization")?;
                items.push(InitItem { off: 0, ty: ty.clone(), bits: None, val: v });
            }
        }
        Ok((items, ty))
    }

    /// The elements of a braced list for `ty` at byte offset `base`. With `braced`, the
    /// list ends at '}'; otherwise (brace elision) it ends when `ty` is full.
    fn init_list(&mut self, ty: &mut Ty, base: u32, items: &mut Vec<InitItem>, is_static: bool, braced: bool) -> R<()> {
        match ty.clone() {
            Ty::Array(elem, n) => {
                let es = self.size(&elem);
                let mut i: u32 = 0;
                let mut max: u32 = 0;
                loop {
                    if self.is("}") {
                        break;
                    }
                    if !braced && n.is_some_and(|n| i >= n) {
                        break;
                    }
                    if braced && self.accept("[") {
                        i = self.const_expr()? as u32;
                        self.expect("]")?;
                        self.accept("=");
                    } else if !braced && (self.is("[") || self.is(".")) {
                        break;
                    }
                    if n.is_some_and(|n| i >= n) {
                        self.warn("excess elements in array initializer");
                        let mut dummy = Vec::new();
                        let mut et = (*elem).clone();
                        self.init_elem(&mut et, 0, &mut dummy, is_static)?;
                    } else {
                        let mut et = (*elem).clone();
                        self.init_elem(&mut et, base + i * es, items, is_static)?;
                    }
                    i += 1;
                    max = max.max(i);
                    if !self.accept(",") {
                        break;
                    }
                }
                if n.is_none() {
                    *ty = Ty::Array(elem, Some(max));
                }
                Ok(())
            }
            Ty::Struct(si) => {
                let members = self.unit.structs[si].members.clone();
                let union = self.unit.structs[si].union;
                let mut k = 0;
                loop {
                    if self.is("}") {
                        break;
                    }
                    if braced && self.is(".") {
                        self.next();
                        let name = self.ident()?;
                        match members.iter().position(|m| m.name == name) {
                            Some(p) => k = p,
                            None => return self.error(&format!("unknown field '{name}' specified in initializer")),
                        }
                        // nested designators .a.b
                        while self.is(".") || self.is("[") {
                            return self.error("nested designators are not supported");
                        }
                        self.accept("=");
                    } else if !braced && (self.is(".") || self.is("[")) {
                        break;
                    }
                    if k >= members.len() || (union && k > 0 && !braced) {
                        if braced {
                            self.warn("excess elements in struct initializer");
                            let mut dummy = Vec::new();
                            let mut t = Ty::Int;
                            self.init_elem(&mut t, 0, &mut dummy, is_static)?;
                            if !self.accept(",") {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                    let m = &members[k];
                    let mut mt = m.ty.clone();
                    if let Some(bits) = m.bits {
                        let e = self.assign_expr()?;
                        let v = self.assign_conv(e, &mt, "initialization")?;
                        items.push(InitItem { off: base + m.offset, ty: mt, bits: Some(bits), val: v });
                    } else {
                        self.init_elem(&mut mt, base + m.offset, items, is_static)?;
                    }
                    k += 1;
                    if union && !braced {
                        break;
                    }
                    if !self.accept(",") {
                        break;
                    }
                    if union && braced && !self.is(".") {
                        // only the first member of a union is initialized positionally
                        if k >= 1 && !self.is("}") {
                            k = members.len();
                        }
                    }
                }
                Ok(())
            }
            _ => {
                // scalar in braces
                let e = self.assign_expr()?;
                let v = self.assign_conv(e, ty, "initialization")?;
                items.push(InitItem { off: base, ty: ty.clone(), bits: None, val: v });
                while self.accept(",") {
                    if self.is("}") {
                        break;
                    }
                    self.assign_expr()?;
                    self.warn("excess elements in scalar initializer");
                }
                Ok(())
            }
        }
    }

    fn init_elem(&mut self, ty: &mut Ty, off: u32, items: &mut Vec<InitItem>, is_static: bool) -> R<()> {
        if self.is("{") {
            self.next();
            self.init_list(ty, off, items, is_static, true)?;
            self.accept(",");
            return self.expect("}");
        }
        // a string for a char array member
        if let Ty::Array(elem, n) = ty.clone() {
            if elem.is_integer() && matches!(self.peek().map(|t| &t.tok), Some(Tok::Str(_))) {
                let es = self.size(&elem);
                let (units, _) = self.string_units()?;
                let len = n.unwrap_or(units.len() as u32 + 1);
                for (i, u) in units.iter().enumerate().take(len as usize) {
                    items.push(InitItem { off: off + i as u32 * es, ty: (*elem).clone(), bits: None, val: Expr::int(*u as i64) });
                }
                return Ok(());
            }
        }
        if ty.is_array() || ty.is_struct() {
            // a struct-typed expression initializes the whole member
            if ty.is_struct() && !self.is(".") && !self.is("[") {
                let save = self.look.len();
                let _ = save;
                // try an expression of the struct type
                let start_is_value = !self.is("{");
                if start_is_value {
                    // brace elision unless the expression has the struct type: peek by
                    // parsing an expression only if it is not a plain scalar start
                }
            }
            return self.init_list(ty, off, items, is_static, false);
        }
        let e = self.assign_expr()?;
        let v = self.assign_conv(e, ty, "initialization")?;
        items.push(InitItem { off, ty: ty.clone(), bits: None, val: v });
        Ok(())
    }

    /// Evaluate initializer items for static storage.
    fn static_data(&mut self, items: &[InitItem]) -> R<Vec<DataItem>> {
        let mut out = Vec::new();
        for it in items {
            if it.ty.is_struct() {
                return self.error("initializer element is not constant");
            }
            let Some(c) = self.const_value(&it.val) else {
                return self.error("initializer element is not constant");
            };
            let size = self.size(&it.ty);
            out.push(DataItem { off: it.off, size, bits: it.bits, float: it.ty.is_float(), val: c });
        }
        Ok(out)
    }

    /// Constant value of an expression for static initialization.
    fn const_value(&self, e: &Expr) -> Option<Const> {
        match &e.k {
            E::Int(v) => Some(Const::Int(*v)),
            E::Flt(f) => Some(Const::Flt(*f)),
            E::Str(n) => Some(Const::Str(*n, 0)),
            E::Addr(x) => self.const_addr(x),
            E::Cast(x) => {
                let c = self.const_value(x)?;
                match c {
                    Const::Int(v) if e.ty.is_float() => Some(Const::Flt(if x.ty.is_unsigned() { v as u64 as f64 } else { v as f64 })),
                    Const::Flt(f) if e.ty.is_integer() => Some(Const::Int(wrap(f as i64, &e.ty))),
                    Const::Int(v) => Some(Const::Int(wrap(v, &e.ty))),
                    c => Some(c),
                }
            }
            E::Bin(op @ ("+" | "-"), a, b) => {
                let ca = self.const_value(a)?;
                let cb = self.const_value(b)?;
                match (ca, cb, *op) {
                    (Const::Addr(s, o), Const::Int(v), "+") | (Const::Int(v), Const::Addr(s, o), "+") => Some(Const::Addr(s, o + v)),
                    (Const::Addr(s, o), Const::Int(v), "-") => Some(Const::Addr(s, o - v)),
                    (Const::Str(s, o), Const::Int(v), "+") | (Const::Int(v), Const::Str(s, o), "+") => Some(Const::Str(s, o + v)),
                    (Const::Str(s, o), Const::Int(v), "-") => Some(Const::Str(s, o - v)),
                    _ => None,
                }
            }
            E::Global(n) if e.ty.is_func() => Some(Const::Addr(n.clone(), 0)),
            _ => None,
        }
    }

    fn const_addr(&self, e: &Expr) -> Option<Const> {
        match &e.k {
            E::Global(n) => Some(Const::Addr(n.clone(), 0)),
            E::Str(n) => Some(Const::Str(*n, 0)),
            E::Member(b, off, None) => match self.const_addr(b)? {
                Const::Addr(s, o) => Some(Const::Addr(s, o + *off as i64)),
                Const::Str(s, o) => Some(Const::Str(s, o + *off as i64)),
                _ => None,
            },
            E::Deref(p) => self.const_value(p),
            _ => None,
        }
    }

    // ---- expressions: conversions ------------------------------------------------------------

    /// Lvalue/array/function to rvalue conversion.
    fn rv(&mut self, e: Expr) -> Expr {
        match &e.ty {
            Ty::Array(elem, _) => {
                let t = Ty::Ptr(elem.clone());
                self.mark_addr(&e);
                Expr::new(E::Addr(Box::new(e)), t)
            }
            Ty::Func(_) => {
                let t = Ty::ptr(e.ty.clone());
                Expr::new(E::Addr(Box::new(e)), t)
            }
            _ => e,
        }
    }

    fn mark_addr(&mut self, e: &Expr) {
        match &e.k {
            E::Local(i) => {
                if let Some(f) = self.f.as_mut() {
                    f.locals[*i].addr_taken = true;
                }
            }
            E::Member(b, ..) => self.mark_addr(b),
            _ => {}
        }
    }

    /// Convert to `t` (inserting a cast; folding constants).
    fn conv(&self, e: Expr, t: &Ty) -> Expr {
        if &e.ty == t {
            return e;
        }
        if t.is_void() {
            return Expr::new(E::Cast(Box::new(e)), Ty::Void);
        }
        match (&e.k, t) {
            (E::Int(v), t) if t.is_integer() || t.is_ptr() => {
                let v = if e.ty.is_unsigned() && e.ty.is_integer() { wrap(*v, &e.ty.to_unsigned()) } else { *v };
                Expr::new(E::Int(wrap(v, t)), t.clone())
            }
            (E::Int(v), t) if t.is_float() => {
                let f = if e.ty.is_unsigned() { (*v as u64) as f64 } else { *v as f64 };
                Expr::new(E::Flt(if *t == Ty::Float { f as f32 as f64 } else { f }), t.clone())
            }
            (E::Flt(f), t) if t.is_integer() => Expr::new(E::Int(wrap(if t.is_unsigned() { *f as u64 as i64 } else { *f as i64 }, t)), t.clone()),
            (E::Flt(f), t) if t.is_float() => Expr::new(E::Flt(if *t == Ty::Float { *f as f32 as f64 } else { *f }), t.clone()),
            _ => Expr::new(E::Cast(Box::new(e)), t.clone()),
        }
    }

    fn promote(&self, e: Expr) -> Expr {
        if e.ty.is_integer() && e.ty.rank() < 4 {
            let t = Ty::Int;
            return self.conv(e, &t);
        }
        e
    }

    fn common(&self, a: &Ty, b: &Ty) -> Ty {
        if *a == Ty::LDouble || *b == Ty::LDouble {
            return Ty::LDouble;
        }
        if *a == Ty::Double || *b == Ty::Double {
            return Ty::Double;
        }
        if *a == Ty::Float || *b == Ty::Float {
            return Ty::Float;
        }
        let pa = if a.rank() < 4 { Ty::Int } else { a.clone() };
        let pb = if b.rank() < 4 { Ty::Int } else { b.clone() };
        if pa == pb {
            return pa;
        }
        let (ua, ub) = (pa.is_unsigned(), pb.is_unsigned());
        if ua == ub {
            return if pa.rank() >= pb.rank() { pa } else { pb };
        }
        let (u, s) = if ua { (pa, pb) } else { (pb, pa) };
        if u.rank() >= s.rank() {
            return u;
        }
        // the signed type is wider: long long can hold any unsigned int/long
        if self.size(&s) > self.size(&u) {
            return s;
        }
        s.to_unsigned()
    }

    fn assign_conv(&mut self, e: Expr, t: &Ty, what: &str) -> R<Expr> {
        let e = self.rv(e);
        if t.is_struct() {
            if e.ty != *t {
                return self.error(&format!("incompatible types in {what}"));
            }
            return Ok(e);
        }
        if t.is_ptr() {
            if e.ty.is_integer() && e.const_int() != Some(0) {
                self.warn(&format!("{what} makes pointer from integer without a cast"));
            } else if e.ty.is_ptr() && !compatible(t, &e.ty) {
                let (a, b) = (self.tyname(t), self.tyname(&e.ty));
                self.warn(&format!("incompatible pointer types in {what} ('{a}' from '{b}')"));
            } else if !e.ty.is_scalar() {
                return self.error(&format!("incompatible types in {what}"));
            }
            return Ok(self.conv(e, t));
        }
        if t.is_arith() {
            if e.ty.is_ptr() {
                if *t != Ty::Bool {
                    self.warn(&format!("{what} makes integer from pointer without a cast"));
                }
            } else if !e.ty.is_arith() {
                return self.error(&format!("incompatible types in {what}"));
            }
            if *t == Ty::Bool {
                return Ok(self.to_bool(e));
            }
            return Ok(self.conv(e, t));
        }
        self.error(&format!("cannot convert to '{}' in {what}", self.tyname(t)))
    }

    fn to_bool(&self, e: Expr) -> Expr {
        if e.ty == Ty::Bool {
            return e;
        }
        if let Some(v) = e.const_int() {
            return Expr::new(E::Int((v != 0) as i64), Ty::Bool);
        }
        let z = self.zero_of(&e.ty);
        let cmp = Expr::new(E::Bin("!=", Box::new(e), Box::new(z)), Ty::Int);
        self.conv(cmp, &Ty::Bool)
    }

    fn zero_of(&self, t: &Ty) -> Expr {
        if t.is_float() { Expr::new(E::Flt(0.0), t.clone()) } else { Expr::new(E::Int(0), t.clone()) }
    }

    // ---- expressions: parsing ------------------------------------------------------------------

    pub fn expr(&mut self) -> R<Expr> {
        let mut e = self.assign_expr()?;
        while self.accept(",") {
            let r = self.assign_expr()?;
            let r = self.rv(r);
            let t = r.ty.clone();
            e = Expr::new(E::Comma(Box::new(e), Box::new(r)), t);
        }
        Ok(e)
    }

    fn assign_expr(&mut self) -> R<Expr> {
        let lhs = self.cond_expr()?;
        let ops = ["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="];
        let Some(op) = ops.iter().find(|o| self.is(o)).copied() else { return Ok(lhs) };
        self.next();
        let rhs = self.assign_expr()?;
        if !lhs.is_lvalue() || lhs.ty.is_array() || lhs.ty.is_func() {
            return self.error("lvalue required as left operand of assignment");
        }
        let lt = lhs.ty.clone();
        if op == "=" {
            let r = self.assign_conv(rhs, &lt, "assignment")?;
            return Ok(Expr::new(E::Assign(Box::new(lhs), Box::new(r)), lt));
        }
        let bop: &'static str = &op[..op.len() - 1];
        let rhs = self.rv(rhs);
        // pointer += int
        if lt.is_ptr() && (bop == "+" || bop == "-") {
            if !rhs.ty.is_integer() {
                return self.error("invalid operands to compound assignment");
            }
            let es = self.size(lt.base().unwrap()).max(1) as i64;
            let r = self.conv(rhs, &Ty::Int);
            let r = self.mul_const(r, es);
            let r = self.conv(r, &lt);
            return Ok(Expr::new(E::OpAssign(bop, Box::new(lhs), Box::new(r), lt.clone()), lt));
        }
        if !lt.is_arith() || !rhs.ty.is_arith() {
            return self.error("invalid operands to compound assignment");
        }
        let optype = if bop == "<<" || bop == ">>" {
            let p = if lt.rank() < 4 && lt.is_integer() { Ty::Int } else { lt.clone() };
            if !p.is_integer() || !rhs.ty.is_integer() {
                return self.error("invalid operands to shift");
            }
            p
        } else {
            self.common(&lt, &rhs.ty)
        };
        if matches!(bop, "%" | "&" | "|" | "^") && !optype.is_integer() {
            return self.error("invalid operands to binary operator");
        }
        let r = if bop == "<<" || bop == ">>" { self.conv(rhs, &Ty::Int) } else { self.conv(rhs, &optype) };
        Ok(Expr::new(E::OpAssign(bop, Box::new(lhs), Box::new(r), optype), lt))
    }

    fn mul_const(&self, e: Expr, k: i64) -> Expr {
        if k == 1 {
            return e;
        }
        if let Some(v) = e.const_int() {
            return Expr::new(E::Int(wrap(v.wrapping_mul(k), &e.ty)), e.ty);
        }
        let t = e.ty.clone();
        Expr::new(E::Bin("*", Box::new(e), Box::new(Expr::new(E::Int(k), t.clone()))), t)
    }

    fn cond_expr(&mut self) -> R<Expr> {
        let c = self.binary(1)?;
        if !self.accept("?") {
            return Ok(c);
        }
        let c = self.cond_of(c)?;
        // GNU a ?: b
        let a = if self.is(":") { c.clone() } else { self.expr()? };
        self.expect(":")?;
        let b = self.cond_expr()?;
        let (a, b) = (self.rv(a), self.rv(b));
        let t = if a.ty.is_arith() && b.ty.is_arith() {
            self.common(&a.ty, &b.ty)
        } else if a.ty.is_ptr() && b.ty.is_ptr() {
            if a.ty.base().is_some_and(|x| x.is_void()) { a.ty.clone() } else { b.ty.clone() }
            .clone()
        } else if a.ty.is_ptr() && b.ty.is_integer() {
            a.ty.clone()
        } else if b.ty.is_ptr() && a.ty.is_integer() {
            b.ty.clone()
        } else if a.ty == b.ty {
            a.ty.clone()
        } else if a.ty.is_void() || b.ty.is_void() {
            Ty::Void
        } else {
            return self.error("type mismatch in conditional expression");
        };
        // fold constant conditions
        if let Some(v) = c.const_int() {
            let pick = if v != 0 { a } else { b };
            return Ok(self.conv(pick, &t));
        }
        let (a, b) = (self.conv(a, &t), self.conv(b, &t));
        Ok(Expr::new(E::Cond(Box::new(c), Box::new(a), Box::new(b)), t))
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
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut l = self.cast_expr()?;
        loop {
            let Some(Tok::Punct(op)) = self.peek().map(|t| t.tok.clone()) else { break };
            let Some(p) = Self::prec(op) else { break };
            if p < min {
                break;
            }
            self.next();
            let r = self.binary(p + 1)?;
            l = self.make_bin(op, l, r)?;
        }
        Ok(l)
    }

    pub fn make_bin(&mut self, op: &'static str, l: Expr, r: Expr) -> R<Expr> {
        let (l, r) = (self.rv(l), self.rv(r));
        match op {
            "&&" | "||" => {
                if !l.ty.is_scalar() || !r.ty.is_scalar() {
                    return self.error(&format!("invalid operands to binary {op}"));
                }
                if let (Some(a), Some(b)) = (l.const_int(), r.const_int()) {
                    let v = if op == "&&" { a != 0 && b != 0 } else { a != 0 || b != 0 };
                    return Ok(Expr::int(v as i64));
                }
                let k = if op == "&&" { E::LogAnd(Box::new(l), Box::new(r)) } else { E::LogOr(Box::new(l), Box::new(r)) };
                return Ok(Expr::new(k, Ty::Int));
            }
            "+" | "-" if l.ty.is_ptr() || r.ty.is_ptr() => return self.ptr_arith(op, l, r),
            "==" | "!=" | "<" | ">" | "<=" | ">=" if l.ty.is_ptr() || r.ty.is_ptr() => {
                let t = if l.ty.is_ptr() { l.ty.clone() } else { r.ty.clone() };
                if !(l.ty.is_ptr() || l.ty.is_integer()) || !(r.ty.is_ptr() || r.ty.is_integer()) {
                    return self.error(&format!("invalid operands to binary {op}"));
                }
                let (l, r) = (self.conv(l, &t), self.conv(r, &t));
                return Ok(Expr::new(E::Bin(op, Box::new(l), Box::new(r)), Ty::Int));
            }
            _ => {}
        }
        if !l.ty.is_arith() || !r.ty.is_arith() {
            let (a, b) = (self.tyname(&l.ty), self.tyname(&r.ty));
            return self.error(&format!("invalid operands to binary {op} (have '{a}' and '{b}')"));
        }
        let shift = op == "<<" || op == ">>";
        let t = if shift { self.promote(l.clone()).ty } else { self.common(&l.ty, &r.ty) };
        if matches!(op, "%" | "&" | "|" | "^" | "<<" | ">>") && !t.is_integer() {
            return self.error(&format!("invalid operands to binary {op}"));
        }
        let l = self.conv(l, &t);
        let r = if shift { self.conv(r, &Ty::Int) } else { self.conv(r, &t) };
        let cmp = matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=");
        // constant folding
        if let (Some(a), Some(b)) = (l.const_int(), r.const_int()) {
            let uns = t.is_unsigned();
            let (ua, ub) = (wrap(a, &t.to_unsigned()) as u64, wrap(b, &t.to_unsigned()) as u64);
            let v: Option<i64> = match op {
                "+" => Some(a.wrapping_add(b)),
                "-" => Some(a.wrapping_sub(b)),
                "*" => Some(a.wrapping_mul(b)),
                "/" | "%" if b == 0 => None,
                "/" => Some(if uns { (ua / ub) as i64 } else { a.wrapping_div(b) }),
                "%" => Some(if uns { (ua % ub) as i64 } else { a.wrapping_rem(b) }),
                "&" => Some(a & b),
                "|" => Some(a | b),
                "^" => Some(a ^ b),
                "<<" => Some(a.wrapping_shl(b as u32)),
                ">>" => Some(if uns { (ua >> (b as u32 & 63)) as i64 } else { a >> (b as u32 & 63) }),
                "==" => Some((a == b) as i64),
                "!=" => Some((a != b) as i64),
                "<" => Some(if uns { ua < ub } else { a < b } as i64),
                ">" => Some(if uns { ua > ub } else { a > b } as i64),
                "<=" => Some(if uns { ua <= ub } else { a <= b } as i64),
                ">=" => Some(if uns { ua >= ub } else { a >= b } as i64),
                _ => None,
            };
            if let Some(v) = v {
                return Ok(if cmp { Expr::int(v) } else { Expr::new(E::Int(wrap(v, &t)), t) });
            }
        }
        if let (E::Flt(a), E::Flt(b)) = (&l.k, &r.k) {
            let (a, b) = (*a, *b);
            let v = match op {
                "+" => Some(a + b),
                "-" => Some(a - b),
                "*" => Some(a * b),
                "/" => Some(a / b),
                _ => None,
            };
            if let Some(v) = v {
                return Ok(Expr::new(E::Flt(if t == Ty::Float { v as f32 as f64 } else { v }), t));
            }
            let c = match op {
                "==" => Some(a == b),
                "!=" => Some(a != b),
                "<" => Some(a < b),
                ">" => Some(a > b),
                "<=" => Some(a <= b),
                ">=" => Some(a >= b),
                _ => None,
            };
            if let Some(c) = c {
                return Ok(Expr::int(c as i64));
            }
        }
        let rt = if cmp { Ty::Int } else { t };
        Ok(Expr::new(E::Bin(op, Box::new(l), Box::new(r)), rt))
    }

    fn ptr_arith(&mut self, op: &'static str, l: Expr, r: Expr) -> R<Expr> {
        if l.ty.is_ptr() && r.ty.is_ptr() {
            if op != "-" {
                return self.error("invalid operands to binary + (two pointers)");
            }
            let es = self.size(l.ty.base().unwrap()).max(1) as i64;
            let (li, ri) = (self.conv(l, &Ty::Int), self.conv(r, &Ty::Int));
            let d = Expr::new(E::Bin("-", Box::new(li), Box::new(ri)), Ty::Int);
            if es == 1 {
                return Ok(d);
            }
            return Ok(Expr::new(E::Bin("/", Box::new(d), Box::new(Expr::int(es))), Ty::Int));
        }
        let (p, i) = if l.ty.is_ptr() { (l, r) } else { (r, l) };
        if !i.ty.is_integer() {
            return self.error(&format!("invalid operands to binary {op}"));
        }
        if op == "-" && !p.ty.is_ptr() {
            return self.error("invalid operands to binary -");
        }
        let pt = p.ty.clone();
        let es = match pt.base() {
            Some(b) if b.is_void() || b.is_func() => 1,
            Some(b) => self.size(b).max(1) as i64,
            None => 1,
        };
        let i = self.conv(i, &Ty::Int);
        let i = self.mul_const(i, es);
        let i = self.conv(i, &pt);
        // fold &x + const into the address
        Ok(Expr::new(E::Bin(op, Box::new(p), Box::new(i)), pt))
    }

    fn cast_expr(&mut self) -> R<Expr> {
        if self.is("(") && self.peek_at(1).and_then(|t| t.ident().map(String::from)).is_some_and(|w| is_decl_start_word(&w) || self.is_typedef_name(&w) || w == "typeof" || w == "__typeof__") {
            self.next();
            let t = self.type_name()?;
            self.expect(")")?;
            if self.is("{") {
                // compound literal
                let e = self.compound_literal(t)?;
                return self.postfix_ops(e);
            }
            let e = self.cast_expr()?;
            let e = self.rv(e);
            if t.is_void() {
                return Ok(Expr::new(E::Cast(Box::new(e)), Ty::Void));
            }
            if !t.is_scalar() || !(e.ty.is_scalar()) {
                if t == e.ty {
                    return Ok(e);
                }
                return self.error("conversion to non-scalar type requested");
            }
            if t.is_float() && e.ty.is_ptr() || t.is_ptr() && e.ty.is_float() {
                return self.error("invalid cast between pointer and floating type");
            }
            if t == Ty::Bool {
                return Ok(self.to_bool(e));
            }
            return Ok(self.conv(e, &t));
        }
        self.unary()
    }

    fn compound_literal(&mut self, t: Ty) -> R<Expr> {
        let is_static = self.f.is_none();
        let (items, t) = self.initializer(&t, is_static)?;
        if is_static {
            self.static_count += 1;
            let asm = format!("S{}_lit", self.static_count);
            let data = self.static_data(&items)?;
            self.unit.globals.push(Global { asm_name: asm.clone(), ty: t.clone(), global: false, init: Some(data), readonly: false });
            return Ok(Expr::new(E::Global(asm), t));
        }
        let id = self.add_local("", t.clone(), false);
        self.f.as_mut().unwrap().locals[id].addr_taken = true;
        let zero = !t.is_scalar();
        let init = Stmt::Init(id, items, zero);
        let v = Expr::new(E::Local(id), t.clone());
        Ok(Expr::new(E::StmtExpr(Box::new(init), Box::new(v)), t))
    }

    fn unary(&mut self) -> R<Expr> {
        let Some(t) = self.peek().cloned() else { return self.error("expected an expression") };
        if let Tok::Punct(p) = t.tok {
            match p {
                "++" | "--" => {
                    self.next();
                    let e = self.unary()?;
                    return self.incdec(e, p == "++", true);
                }
                "&" => {
                    self.next();
                    let e = self.cast_expr()?;
                    if e.ty.is_func() {
                        return Ok(self.rv(e));
                    }
                    if !e.is_lvalue() {
                        if let E::StmtExpr(..) = e.k {
                            let t = Ty::ptr(e.ty.clone());
                            return Ok(Expr::new(E::Addr(Box::new(e)), t));
                        }
                        return self.error("lvalue required as unary '&' operand");
                    }
                    if let E::Member(_, _, Some(_)) = e.k {
                        return self.error("cannot take address of bit-field");
                    }
                    self.mark_addr(&e);
                    let t = Ty::ptr(e.ty.clone());
                    // &*p is p
                    if let E::Deref(p) = e.k {
                        return Ok(self.conv(*p, &t));
                    }
                    return Ok(Expr::new(E::Addr(Box::new(e)), t));
                }
                "*" => {
                    self.next();
                    let e = self.cast_expr()?;
                    return self.deref(e);
                }
                "+" | "-" | "~" | "!" => {
                    self.next();
                    let e = self.cast_expr()?;
                    let e = self.rv(e);
                    if p == "!" {
                        if !e.ty.is_scalar() {
                            return self.error("wrong type argument to unary '!'");
                        }
                        if let Some(v) = e.const_int() {
                            return Ok(Expr::int((v == 0) as i64));
                        }
                        return Ok(Expr::new(E::Unary("!", Box::new(e)), Ty::Int));
                    }
                    if !e.ty.is_arith() || (p == "~" && !e.ty.is_integer()) {
                        return self.error(&format!("wrong type argument to unary '{p}'"));
                    }
                    let e = self.promote(e);
                    let t = e.ty.clone();
                    if p == "+" {
                        return Ok(e);
                    }
                    if let Some(v) = e.const_int() {
                        let r = if p == "-" { v.wrapping_neg() } else { !v };
                        return Ok(Expr::new(E::Int(wrap(r, &t)), t));
                    }
                    if let E::Flt(f) = e.k {
                        if p == "-" {
                            return Ok(Expr::new(E::Flt(-f), t));
                        }
                    }
                    let op: &'static str = if p == "-" { "-" } else { "~" };
                    return Ok(Expr::new(E::Unary(op, Box::new(e)), t));
                }
                _ => {}
            }
        }
        if let Some(w) = t.ident() {
            match w {
                "sizeof" => {
                    self.next();
                    let ty = if self.is("(") && self.peek_at(1).and_then(|t| t.ident().map(String::from)).is_some_and(|w| is_decl_start_word(&w) || self.is_typedef_name(&w) || w == "typeof" || w == "__typeof__") {
                        self.next();
                        let t = self.type_name()?;
                        self.expect(")")?;
                        if self.is("{") {
                            let e = self.compound_literal(t)?;
                            e.ty
                        } else {
                            t
                        }
                    } else {
                        let e = self.unary()?;
                        e.ty
                    };
                    if ty.is_func() {
                        return self.error("invalid application of 'sizeof' to a function type");
                    }
                    if let Ty::Struct(i) = ty {
                        if !self.unit.structs[i].complete {
                            return self.error("invalid application of 'sizeof' to incomplete type");
                        }
                    }
                    let s = self.size(&ty);
                    return Ok(Expr::new(E::Int(s as i64), Ty::UInt));
                }
                "_Alignof" | "__alignof__" | "__alignof" => {
                    self.next();
                    self.expect("(")?;
                    let ty = self.type_name()?;
                    self.expect(")")?;
                    let a = size_align(&ty, &self.unit.structs).1;
                    return Ok(Expr::new(E::Int(a as i64), Ty::UInt));
                }
                _ => {}
            }
        }
        self.postfix()
    }

    fn deref(&mut self, e: Expr) -> R<Expr> {
        let e = self.rv(e);
        let Some(b) = e.ty.base().cloned() else {
            let n = self.tyname(&e.ty);
            return self.error(&format!("invalid type argument of unary '*' (have '{n}')"));
        };
        if b.is_func() {
            // *fp is the function again
            return Ok(e);
        }
        if b.is_void() {
            self.warn("dereferencing 'void *' pointer");
        }
        // *&x is x
        if let E::Addr(x) = e.k {
            if x.ty == b {
                return Ok(*x);
            }
            return Ok(Expr::new(E::Deref(Box::new(Expr::new(E::Addr(x), e.ty))), b));
        }
        Ok(Expr::new(E::Deref(Box::new(e)), b))
    }

    fn incdec(&mut self, e: Expr, inc: bool, pre: bool) -> R<Expr> {
        if !e.is_lvalue() || !e.ty.is_scalar() {
            return self.error(&format!("lvalue required as {} operand", if inc { "increment" } else { "decrement" }));
        }
        let step = if let Some(b) = e.ty.base() { self.size(b).max(1) as i64 } else { 1 };
        let amount = if inc { step } else { -step };
        let t = e.ty.clone();
        Ok(Expr::new(E::IncDec(Box::new(e), amount, pre), t))
    }

    fn string_units(&mut self) -> R<(Vec<u32>, u32)> {
        let mut units = Vec::new();
        let mut size = 1;
        while let Some(Tok::Str(s)) = self.peek().map(|t| t.tok.clone()) {
            self.next();
            let (prefix, body) = match s.find('"') {
                Some(i) => (&s[..i], &s[i + 1..s.len() - 1]),
                None => ("", ""),
            };
            if prefix == "L" || prefix == "U" {
                size = 4;
            } else if prefix == "u" {
                size = 2;
            }
            if prefix.is_empty() || prefix == "u8" {
                units.extend(decode(body));
            } else {
                // wide: decode escapes but keep code points
                let mut it = body.chars().peekable();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        if let Some(v) = escape(&mut it) {
                            units.push(v);
                        }
                    } else {
                        units.push(c as u32);
                    }
                }
            }
        }
        Ok((units, size))
    }

    fn primary(&mut self) -> R<Expr> {
        let Some(t) = self.next() else { return self.error("expected an expression") };
        match &t.tok {
            Tok::Num(s) => {
                let s = s.clone();
                let lower = s.to_ascii_lowercase();
                let is_hex = lower.starts_with("0x");
                let is_float = if is_hex { lower.contains('p') || lower.contains('.') } else { lower.contains('.') || lower.contains('e') };
                if is_float {
                    let (body, ty) = if let Some(b) = lower.strip_suffix('f') {
                        (b, Ty::Float)
                    } else if let Some(b) = lower.strip_suffix('l') {
                        (b, Ty::LDouble)
                    } else {
                        (lower.as_str(), Ty::Double)
                    };
                    let v = if is_hex { parse_hex_float(body) } else { body.parse::<f64>().ok() };
                    let Some(v) = v else { return self.error(&format!("invalid floating constant '{s}'")) };
                    let v = if ty == Ty::Float { v as f32 as f64 } else { v };
                    return Ok(Expr::new(E::Flt(v), ty));
                }
                match int_literal(&s) {
                    Some((v, ty)) => Ok(Expr::new(E::Int(wrap(v as i64, &ty)), ty)),
                    None => self.error(&format!("invalid integer constant '{s}'")),
                }
            }
            Tok::Char(s) => match char_literal(s) {
                Some(v) => Ok(Expr::int(v)),
                None => self.error("empty character constant"),
            },
            Tok::Str(_) => {
                self.look.push_front(t.clone());
                let (units, size) = self.string_units()?;
                let n = units.len() as u32 + 1;
                let elem = match size {
                    4 => Ty::Int,
                    2 => Ty::UShort,
                    _ => Ty::Char,
                };
                self.unit.strings.push((units, size));
                let id = self.unit.strings.len() - 1;
                Ok(Expr::new(E::Str(id), Ty::Array(Rc::new(elem), Some(n))))
            }
            Tok::Punct("(") => {
                if self.is("{") {
                    // GNU statement expression
                    self.scopes.push(Scope::default());
                    let body = self.compound(false);
                    self.scopes.pop();
                    self.expect(")")?;
                    let mut body = match body? {
                        Stmt::Block(v) => v,
                        s => vec![s],
                    };
                    let last = match body.pop() {
                        Some(Stmt::Expr(e)) => e,
                        Some(s) => {
                            body.push(s);
                            Expr::new(E::Int(0), Ty::Void)
                        }
                        None => Expr::new(E::Int(0), Ty::Void),
                    };
                    let last = self.rv(last);
                    let t = last.ty.clone();
                    return Ok(Expr::new(E::StmtExpr(Box::new(Stmt::Block(body)), Box::new(last)), t));
                }
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(n) => {
                let n = n.clone();
                match n.as_str() {
                    "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__" => {
                        let name = self.f.as_ref().map(|f| f.name.clone()).unwrap_or_default();
                        let units: Vec<u32> = name.bytes().map(|b| b as u32).collect();
                        let len = units.len() as u32 + 1;
                        self.unit.strings.push((units, 1));
                        return Ok(Expr::new(E::Str(self.unit.strings.len() - 1), Ty::Array(Rc::new(Ty::Char), Some(len))));
                    }
                    "__builtin_va_start" => return self.va_start(),
                    "__builtin_va_arg" => return self.va_arg(),
                    "__builtin_va_end" => {
                        self.expect("(")?;
                        let e = self.assign_expr()?;
                        self.expect(")")?;
                        return Ok(Expr::new(E::Cast(Box::new(e)), Ty::Void));
                    }
                    "__builtin_va_copy" => {
                        self.expect("(")?;
                        let d = self.assign_expr()?;
                        self.expect(",")?;
                        let s = self.assign_expr()?;
                        self.expect(")")?;
                        let s = self.rv(s);
                        let t = d.ty.clone();
                        return Ok(Expr::new(E::Assign(Box::new(d), Box::new(s)), t));
                    }
                    "__builtin_offsetof" | "offsetof" if self.is("(") && n.starts_with("__") => {
                        self.expect("(")?;
                        let t = self.type_name()?;
                        self.expect(",")?;
                        let mut off = 0u32;
                        let mut cur = t;
                        loop {
                            let m = self.ident()?;
                            let Ty::Struct(si) = cur else { return self.error("offsetof on a non-struct") };
                            let Some(mem) = self.unit.structs[si].members.iter().find(|x| x.name == m).cloned() else {
                                return self.error(&format!("no member named '{m}'"));
                            };
                            off += mem.offset;
                            cur = mem.ty;
                            while self.accept("[") {
                                let i = self.const_expr()?;
                                self.expect("]")?;
                                if let Some(b) = cur.base().cloned() {
                                    off += self.size(&b) * i as u32;
                                    cur = b;
                                }
                            }
                            if !self.accept(".") {
                                break;
                            }
                        }
                        self.expect(")")?;
                        return Ok(Expr::new(E::Int(off as i64), Ty::UInt));
                    }
                    "__builtin_expect" => {
                        self.expect("(")?;
                        let e = self.assign_expr()?;
                        self.expect(",")?;
                        self.assign_expr()?;
                        self.expect(")")?;
                        return Ok(e);
                    }
                    "__builtin_constant_p" => {
                        self.expect("(")?;
                        let e = self.assign_expr()?;
                        self.expect(")")?;
                        return Ok(Expr::int(e.const_int().is_some() as i64));
                    }
                    "__builtin_unreachable" | "__builtin_trap" => {
                        // a call to abort()
                        self.expect("(")?;
                        self.expect(")")?;
                        let f = Expr::new(E::Global("_abort".into()), Ty::Func(Rc::new(FnTy { ret: Ty::Void, params: Vec::new(), variadic: false, proto: true })));
                        let f = self.rv(f);
                        return Ok(Expr::new(E::Call(Box::new(f), Vec::new()), Ty::Void));
                    }
                    _ => {}
                }
                match self.lookup(&n) {
                    Some(Sym::Local(i)) => {
                        let f = self.f.as_mut().unwrap();
                        f.locals[i].uses = f.locals[i].uses.saturating_add(f.loop_weight);
                        let ty = f.locals[i].ty.clone();
                        Ok(Expr::new(E::Local(i), ty))
                    }
                    Some(Sym::Global(asm, ty)) => {
                        let ty = self.globals.get(&asm).map(|g| g.ty.clone()).unwrap_or(ty);
                        Ok(Expr::new(E::Global(asm), ty))
                    }
                    Some(Sym::Enum(v)) => Ok(Expr::int(v)),
                    Some(Sym::Typedef(_)) => self.error(&format!("unexpected type name '{n}'")),
                    None => {
                        if self.is("(") {
                            // implicit declaration: int n()
                            self.warn(&format!("implicit declaration of function '{n}'"));
                            let ty = Ty::Func(Rc::new(FnTy { ret: Ty::Int, params: Vec::new(), variadic: false, proto: false }));
                            let asm = Self::asm_name(&n);
                            self.scopes[0].syms.insert(n.clone(), Sym::Global(asm.clone(), ty.clone()));
                            self.globals.entry(asm.clone()).or_insert(GlobalInfo { idx: None, ty: ty.clone(), defined: false, global: true });
                            return Ok(Expr::new(E::Global(asm), ty));
                        }
                        self.error(&format!("'{n}' undeclared"))
                    }
                }
            }
            _ => {
                self.look.push_front(t.clone());
                let s = t.text();
                self.error(&format!("expected expression before '{s}'"))
            }
        }
    }

    fn va_start(&mut self) -> R<Expr> {
        self.expect("(")?;
        let ap = self.assign_expr()?;
        if self.accept(",") {
            self.assign_expr()?;
        }
        self.expect(")")?;
        let off = self.f.as_ref().map_or(0, |f| f.va_off);
        if let E::Local(i) = ap.k {
            self.f.as_mut().unwrap().locals[i].addr_taken |= false;
        }
        Ok(Expr::new(E::VaStart(Box::new(ap), off), Ty::Void))
    }

    fn va_arg(&mut self) -> R<Expr> {
        self.expect("(")?;
        let ap = self.assign_expr()?;
        self.expect(",")?;
        let t = self.type_name()?;
        self.expect(")")?;
        Ok(Expr::new(E::VaArg(Box::new(ap)), t))
    }

    fn postfix(&mut self) -> R<Expr> {
        let e = self.primary()?;
        self.postfix_ops(e)
    }

    fn postfix_ops(&mut self, mut e: Expr) -> R<Expr> {
        loop {
            if self.accept("[") {
                let i = self.expr()?;
                self.expect("]")?;
                let sum = self.make_bin("+", e, i)?;
                e = self.deref(sum)?;
            } else if self.accept("(") {
                e = self.call(e)?;
            } else if self.is(".") || self.is("->") {
                let arrow = self.is("->");
                self.next();
                let m = self.ident()?;
                let base = if arrow { self.deref(e)? } else { e };
                let Ty::Struct(si) = base.ty else {
                    let n = self.tyname(&base.ty);
                    return self.error(&format!("request for member '{m}' in something not a structure or union ('{n}')"));
                };
                if !self.unit.structs[si].complete {
                    return self.error("dereferencing pointer to incomplete type");
                }
                let Some(mem) = self.unit.structs[si].members.iter().find(|x| x.name == m).cloned() else {
                    let n = self.tyname(&base.ty);
                    return self.error(&format!("'{n}' has no member named '{m}'"));
                };
                // members of a struct returned by value need an lvalue: wrap in a temporary
                let base = if base.is_lvalue() || matches!(base.k, E::StmtExpr(..)) { base } else { self.temp_of(base) };
                e = Expr::new(E::Member(Box::new(base), mem.offset, mem.bits), mem.ty);
            } else if self.is("++") || self.is("--") {
                let inc = self.is("++");
                self.next();
                e = self.incdec(e, inc, false)?;
            } else {
                return Ok(e);
            }
        }
    }

    /// Store a struct value in a temporary local so its members can be addressed.
    fn temp_of(&mut self, e: Expr) -> Expr {
        let t = e.ty.clone();
        if self.f.is_none() {
            return e;
        }
        let id = self.add_local("", t.clone(), false);
        self.f.as_mut().unwrap().locals[id].addr_taken = true;
        let init = Stmt::Init(id, vec![InitItem { off: 0, ty: t.clone(), bits: None, val: e }], false);
        Expr::new(E::StmtExpr(Box::new(init), Box::new(Expr::new(E::Local(id), t.clone()))), t)
    }

    fn call(&mut self, f: Expr) -> R<Expr> {
        let f = self.rv(f);
        let Some(ft) = f.ty.func().cloned() else {
            let n = self.tyname(&f.ty);
            return self.error(&format!("called object is not a function or function pointer (type '{n}')"));
        };
        let mut args = Vec::new();
        if !self.accept(")") {
            loop {
                args.push(self.assign_expr()?);
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        if ft.proto {
            if args.len() < ft.params.len() || (args.len() > ft.params.len() && !ft.variadic) {
                return self.error(&format!("wrong number of arguments to function (expected {}, have {})", ft.params.len(), args.len()));
            }
        }
        let mut conv_args = Vec::new();
        for (i, a) in args.into_iter().enumerate() {
            let a = if ft.proto && i < ft.params.len() {
                let pt = ft.params[i].clone();
                self.assign_conv(a, &pt, "argument")?
            } else {
                // default argument promotions
                let a = self.rv(a);
                if a.ty == Ty::Float {
                    self.conv(a, &Ty::Double)
                } else if a.ty.is_integer() {
                    self.promote(a)
                } else {
                    a
                }
            };
            conv_args.push(a);
        }
        let ret = ft.ret.clone();
        Ok(Expr::new(E::Call(Box::new(f), conv_args), ret))
    }
}

fn parse_hex_float(s: &str) -> Option<f64> {
    let s = s.strip_prefix("0x")?;
    let (mant, exp) = match s.find('p') {
        Some(i) => (&s[..i], s[i + 1..].parse::<i32>().ok()?),
        None => (s, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    let mut v = 0f64;
    for c in int.chars() {
        v = v * 16.0 + c.to_digit(16)? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in frac.chars() {
        v += c.to_digit(16)? as f64 * scale;
        scale /= 16.0;
    }
    let mut r = v;
    if exp > 0 {
        for _ in 0..exp {
            r *= 2.0;
        }
    } else {
        for _ in 0..(-exp) {
            r /= 2.0;
        }
    }
    Some(r)
}
