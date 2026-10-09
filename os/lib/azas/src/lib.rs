//! azas: a 68030 / 68882 / PMMU assembler with vasm-style Motorola syntax.
//!
//! Output is a flat binary (with `org`) or an ELF32 relocatable object for azld. The
//! crate is `no_std` + `alloc`, so it runs both on the build host and as `/bin/as` on
//! the az030.
//!
//! Syntax summary:
//! * `label:` (or a label in column 0), local labels `.name` scoped to the last
//!   global label; `;` comments, `*` comments in column 0.
//! * numbers `123`, `$7F`, `0x7F`, `%1010`, `@17`, `'c'`; `*` is the current address.
//! * directives: `org equ = set dc ds dcb even align cnop section text data bss rodata
//!   xdef xref global include incbin macro endm mexit rept endr if* else endif fail end`
//!   plus common GNU spellings (`.globl .text .long .ascii` ...).

#![no_std]

extern crate alloc;

pub mod encode;
pub mod expr;
pub mod float;
pub mod operand;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use azld::elf::*;
use encode::{Enc, FixKind};
use expr::{Base, Ctx, Parser, Val};
use operand::{split_commas, Sz};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// Flat binary; labels are absolute addresses (use `org`).
    Binary,
    /// ELF32 relocatable object.
    Elf,
}

/// Where `include`/`incbin` files come from.
pub trait Files {
    fn read(&mut self, name: &str) -> Option<Vec<u8>>;
}

pub struct Options {
    pub format: Format,
    /// Symbols predefined with `-D NAME=VALUE`.
    pub defines: Vec<(String, i64)>,
}

const MAX_PASSES: u32 = 40;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SecKind {
    Text,
    Data,
    Rodata,
    Bss,
}

struct Section {
    name: String,
    kind: SecKind,
    data: Vec<u8>,
    /// Current offset (equals data.len() except in bss).
    pc: u32,
    /// Absolute base address (binary output).
    base: u32,
    align: u32,
    relocs: Vec<Reloc>,
}

struct Reloc {
    off: u32,
    kind: FixKind,
    target: Base,
    addend: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SymKind {
    Label,
    Equ,
    Set,
}

#[derive(Clone)]
struct SymEnt {
    val: Val,
    kind: SymKind,
    pass: u32,
}

struct Macro {
    params: Vec<String>,
    body: Rc<Vec<String>>,
}

enum FrameKind {
    File,
    Macro { args: Vec<String>, size: String, id: u32, params: Rc<Vec<String>> },
    Rept,
}

struct Frame {
    name: Rc<String>,
    lines: Rc<Vec<String>>,
    idx: usize,
    /// Line number offset for error messages (file lines start at 1).
    line_base: usize,
    kind: FrameKind,
}

#[derive(Clone, Copy)]
struct Cond {
    /// Lines in this block are assembled.
    active: bool,
    /// Some branch of this if-chain has been taken.
    taken: bool,
    /// The enclosing block is active.
    parent: bool,
}

pub struct Asm<'f> {
    opts: Options,
    files: &'f mut dyn Files,
    file_cache: BTreeMap<String, Rc<Vec<String>>>,
    pass: u32,
    secs: Vec<Section>,
    cur: usize,
    syms: BTreeMap<String, SymEnt>,
    externs: Vec<String>,
    extern_idx: BTreeMap<String, usize>,
    globals: BTreeSet<String>,
    undefined: BTreeSet<String>,
    last_global: String,
    changed: bool,
    errors: Vec<String>,
    macros: BTreeMap<String, Macro>,
    macro_id: u32,
    stack: Vec<Frame>,
    conds: Vec<Cond>,
    ended: bool,
    /// Location of the line being assembled, for errors.
    loc: String,
    prev_sec_sizes: Vec<(String, u32)>,
    /// Instruction sizes by position in the previous pass / this pass.
    prev_sizes: Vec<usize>,
    cur_sizes: Vec<usize>,
}

/// Assemble `source` (named `name` in messages). Returns the output file, or the list of
/// error messages.
pub fn assemble(name: &str, source: &str, opts: Options, files: &mut dyn Files) -> Result<Vec<u8>, Vec<String>> {
    let mut a = Asm {
        opts,
        files,
        file_cache: BTreeMap::new(),
        pass: 0,
        secs: Vec::new(),
        cur: 0,
        syms: BTreeMap::new(),
        externs: Vec::new(),
        extern_idx: BTreeMap::new(),
        globals: BTreeSet::new(),
        undefined: BTreeSet::new(),
        last_global: String::new(),
        changed: false,
        errors: Vec::new(),
        macros: BTreeMap::new(),
        macro_id: 0,
        stack: Vec::new(),
        conds: Vec::new(),
        ended: false,
        loc: String::new(),
        prev_sec_sizes: Vec::new(),
        prev_sizes: Vec::new(),
        cur_sizes: Vec::new(),
    };
    let lines: Rc<Vec<String>> = Rc::new(source.lines().map(String::from).collect());
    a.file_cache.insert(name.into(), lines);
    for pass in 1..=MAX_PASSES {
        a.run_pass(pass, name);
        if !a.changed && pass >= 2 {
            if !a.errors.is_empty() {
                return Err(core::mem::take(&mut a.errors));
            }
            if !a.undefined.is_empty() {
                return Err(a.undefined.iter().map(|u| format!("undefined symbol `{u}`")).collect());
            }
            return Ok(match a.opts.format {
                Format::Binary => a.write_binary(),
                Format::Elf => a.write_elf(),
            });
        }
    }
    let mut e = core::mem::take(&mut a.errors);
    e.push("assembly did not converge (symbol values keep changing)".into());
    Err(e)
}

/// Split `mnemonic.size`.
fn split_size(m: &str) -> (&str, Option<&str>) {
    if let Some(i) = m[1..].find('.') {
        let i = i + 1;
        (&m[..i], Some(&m[i + 1..]))
    } else {
        (m, None)
    }
}

/// Strip a `;` comment (outside quotes).
fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut q = 0u8;
    for (i, &c) in b.iter().enumerate() {
        if q != 0 {
            if c == q {
                q = 0;
            } else if c == b'\\' && q == b'"' {
                // skip escaped char handled loosely
            }
        } else if c == b'"' || (c == b'\'' && (i == 0 || !b[i - 1].is_ascii_alphanumeric())) {
            q = c;
        } else if c == b';' {
            return &line[..i];
        }
    }
    line
}

/// The operand field ends at whitespace that is not inside quotes/brackets and is not
/// next to an operator or comma; anything after it is a comment.
fn operand_field(s: &str) -> &str {
    let b = s.as_bytes();
    let mut q = 0u8;
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            if c == q {
                q = 0;
            }
        } else {
            match c {
                b'"' => q = c,
                b'\'' if i == 0 || !b[i - 1].is_ascii_alphanumeric() => q = c,
                b'(' | b'[' | b'{' | b'<' if c != b'<' || depth > 0 => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b' ' | b'\t' if depth <= 0 => {
                    let prev = b[..i].iter().rev().find(|c| !c.is_ascii_whitespace()).copied().unwrap_or(b',');
                    let mut j = i;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let next = b.get(j).copied().unwrap_or(0);
                    let op = |c: u8| b",+-*/&|^~!<>=(#[{:".contains(&c);
                    let nop = |c: u8| b",+-*/&|^<>=)]}:".contains(&c);
                    if next == 0 || !(op(prev) || nop(next)) {
                        return &s[..i];
                    }
                    i = j;
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }
    s
}

fn is_label_char(c: u8) -> bool {
    expr::is_ident_char(c) || c == b'\\' || c == b'@'
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let q = s.as_bytes().first().copied()?;
    if (q != b'"' && q != b'\'') || s.len() < 2 || !s.ends_with(q as char) {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    let mut out = String::new();
    let mut it = inner.chars().peekable();
    while let Some(c) = it.next() {
        if c == q as char && it.peek() == Some(&(q as char)) {
            it.next();
            out.push(c);
        } else if c == '\\' && q == b'"' {
            match it.next() {
                Some('x') => {
                    let mut v = 0u32;
                    while let Some(d) = it.peek().and_then(|c| c.to_digit(16)) {
                        v = v * 16 + d;
                        it.next();
                    }
                    out.push(char::from_u32(v & 0xFF).unwrap_or('?'));
                }
                Some(e) => out.push(expr::escape(e as u8) as char),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

struct SymCtx<'a, 'f> {
    a: &'a mut Asm<'f>,
}

impl Ctx for SymCtx<'_, '_> {
    fn symbol(&mut self, name: &str) -> Result<Val, String> {
        self.a.lookup(name)
    }
    fn pc(&self) -> Val {
        self.a.here()
    }
}

impl<'f> Asm<'f> {
    fn run_pass(&mut self, pass: u32, main: &str) {
        self.pass = pass;
        self.prev_sec_sizes = self.secs.iter().map(|s| (s.name.clone(), s.pc)).collect();
        self.prev_sizes = core::mem::take(&mut self.cur_sizes);
        self.secs.clear();
        self.cur = 0;
        self.changed = false;
        self.errors.clear();
        self.undefined.clear();
        self.macros.clear();
        self.macro_id = 0;
        self.conds.clear();
        self.ended = false;
        self.last_global.clear();
        self.select_section(".text", None);
        let defs = core::mem::take(&mut self.opts.defines);
        for (n, v) in &defs {
            self.define(n, Val::abs(*v), SymKind::Set);
        }
        self.opts.defines = defs;
        let lines = self.file_cache.get(main).cloned().unwrap_or_default();
        self.stack.push(Frame { name: Rc::new(main.into()), lines, idx: 0, line_base: 1, kind: FrameKind::File });
        while !self.ended {
            let Some((line, loc)) = self.next_line() else { break };
            self.loc = loc;
            self.line(&line);
        }
        self.stack.clear();
        if !self.conds.is_empty() {
            self.err("missing endif");
        }
    }

    fn next_line(&mut self) -> Option<(String, String)> {
        loop {
            let f = self.stack.last_mut()?;
            if f.idx < f.lines.len() {
                let raw = f.lines[f.idx].clone();
                let loc = format!("{}:{}", f.name, f.line_base + f.idx);
                f.idx += 1;
                let line = match &f.kind {
                    FrameKind::Macro { args, size, id, params } => substitute(&raw, args, size, *id, params),
                    _ => raw,
                };
                return Some((line, loc));
            }
            self.stack.pop();
        }
    }

    fn err(&mut self, msg: &str) {
        let m = format!("{}: {}", self.loc, msg);
        if !self.errors.contains(&m) {
            self.errors.push(m);
        }
    }

    fn active(&self) -> bool {
        self.conds.last().is_none_or(|c| c.active)
    }

    fn here(&self) -> Val {
        Val { base: Base::Sec(self.cur), off: self.secs[self.cur].pc as i64 }
    }

    /// Absolute address of a section-relative value (binary output only).
    fn absolute(&self, v: Val) -> Option<i64> {
        match v.base {
            Base::Abs => Some(v.off),
            Base::Sec(i) if self.opts.format == Format::Binary => Some(self.secs.get(i)?.base as i64 + v.off),
            _ => None,
        }
    }

    fn full_name(&self, name: &str) -> String {
        if name.starts_with('.') && name.len() > 1 {
            format!("{}{}", self.last_global, name)
        } else {
            name.to_string()
        }
    }

    fn lookup(&mut self, name: &str) -> Result<Val, String> {
        let full = self.full_name(name);
        if let Some(s) = self.syms.get(&full) {
            return Ok(s.val);
        }
        if name.eq_ignore_ascii_case("narg") {
            if let Some(n) = self.stack.iter().rev().find_map(|f| match &f.kind {
                FrameKind::Macro { args, .. } => Some(args.len()),
                _ => None,
            }) {
                return Ok(Val::abs(n as i64));
            }
        }
        if operand::is_reserved_name(name) {
            return Err(format!("register `{name}` used in an expression"));
        }
        if self.opts.format == Format::Elf && self.pass > 1 && !name.starts_with('.') {
            let i = match self.extern_idx.get(&full) {
                Some(&i) => i,
                None => {
                    self.externs.push(full.clone());
                    self.extern_idx.insert(full.clone(), self.externs.len() - 1);
                    self.externs.len() - 1
                }
            };
            return Ok(Val { base: Base::Ext(i), off: 0 });
        }
        self.undefined.insert(full);
        Ok(Val::UNKNOWN)
    }

    fn define(&mut self, name: &str, val: Val, kind: SymKind) {
        let full = self.full_name(name);
        if kind == SymKind::Label && !name.starts_with('.') {
            self.last_global = full.clone();
        }
        if let Some(old) = self.syms.get(&full) {
            if old.pass == self.pass && kind != SymKind::Set && old.kind != SymKind::Set {
                self.err(&format!("symbol `{full}` defined twice"));
                return;
            }
            if old.val != val && kind != SymKind::Set {
                self.changed = true;
            }
        } else {
            self.changed = true;
        }
        self.syms.insert(full, SymEnt { val, kind, pass: self.pass });
    }

    fn eval(&mut self, s: &str) -> Result<Val, String> {
        let mut ctx = SymCtx { a: self };
        let mut p = Parser::new(s, &mut ctx);
        let v = p.expr()?;
        p.skip_ws();
        if !p.at_end() {
            return Err(format!("junk after expression: `{}`", p.rest()));
        }
        Ok(v)
    }

    /// Evaluate to a constant (unknown values count as 0 in early passes).
    fn eval_const(&mut self, s: &str) -> Result<i64, String> {
        let v = self.eval(s)?;
        if let Some(x) = self.absolute(v) {
            return Ok(x);
        }
        match v.base {
            Base::Unknown => {
                if self.pass > 1 {
                    Err(format!("`{s}` is not defined yet"))
                } else {
                    Ok(0)
                }
            }
            _ => Err(format!("`{s}` is not a constant")),
        }
    }

    // ---- sections ---------------------------------------------------------------------

    fn select_section(&mut self, name: &str, typ: Option<&str>) {
        let name = match name.to_ascii_lowercase().as_str() {
            "text" | "code" => ".text".to_string(),
            "data" => ".data".to_string(),
            "bss" => ".bss".to_string(),
            "rodata" => ".rodata".to_string(),
            _ => name.to_string(),
        };
        if let Some(i) = self.secs.iter().position(|s| s.name == name) {
            self.cur = i;
            return;
        }
        let kind = match typ.map(|t| t.trim().to_ascii_lowercase()) {
            Some(t) if t == "code" || t == "text" => SecKind::Text,
            Some(t) if t == "bss" => SecKind::Bss,
            Some(t) if t == "rodata" => SecKind::Rodata,
            Some(t) if t == "data" => SecKind::Data,
            _ => {
                if name.starts_with(".text") {
                    SecKind::Text
                } else if name.starts_with(".bss") {
                    SecKind::Bss
                } else if name.starts_with(".rodata") {
                    SecKind::Rodata
                } else {
                    SecKind::Data
                }
            }
        };
        // binary output: sections follow each other (sizes from the previous pass)
        let base = if self.secs.is_empty() {
            0
        } else {
            let prev = &self.secs[self.secs.len() - 1];
            let prev_size = self.prev_sec_sizes.iter().find(|(n, _)| *n == prev.name).map_or(prev.pc, |(_, s)| *s);
            (prev.base + prev_size.max(prev.pc) + 3) & !3
        };
        self.secs.push(Section { name, kind, data: Vec::new(), pc: 0, base, align: 2, relocs: Vec::new() });
        self.cur = self.secs.len() - 1;
    }

    fn emit(&mut self, bytes: &[u8]) {
        let s = &mut self.secs[self.cur];
        if s.kind == SecKind::Bss {
            if bytes.iter().any(|&b| b != 0) {
                self.err("initialised data in a bss section");
                return;
            }
        } else {
            s.data.extend_from_slice(bytes);
        }
        s.pc += bytes.len() as u32;
    }

    fn reserve(&mut self, n: u32, fill: u8) {
        let s = &mut self.secs[self.cur];
        if s.kind != SecKind::Bss {
            s.data.resize(s.data.len() + n as usize, fill);
        }
        s.pc += n;
    }

    fn align(&mut self, a: u32, fill: u8) {
        if a <= 1 {
            return;
        }
        let s = &mut self.secs[self.cur];
        s.align = s.align.max(a);
        let pc = self.here().off as u32;
        let pad = (a - pc % a) % a;
        self.reserve(pad, fill);
    }

    /// Emit an encoded instruction or data item with its fixups.
    fn emit_enc(&mut self, e: Enc) {
        let start = self.secs[self.cur].pc;
        let mut bytes = e.b;
        for f in e.fix {
            let v = f.val;
            let pcrel = matches!(f.kind, FixKind::Pc8 | FixKind::Pc16 | FixKind::Pc32);
            match (v.base, self.opts.format) {
                (Base::Unknown, _) => {} // reported via `undefined`
                (Base::Ext(_), Format::Binary) => self.err("external reference in binary output"),
                (_, Format::Binary) => {
                    let target = self.absolute(v).unwrap_or(0);
                    let here = self.secs[self.cur].base as i64 + start as i64 + f.pcbase as i64;
                    let x = if pcrel { target - here } else { target };
                    if let Err(e) = encode::put_field(&mut bytes, f.at, f.kind, x) {
                        self.err(&e);
                    }
                }
                (target, Format::Elf) => {
                    let addend = if pcrel { v.off + f.at as i64 - f.pcbase as i64 } else { v.off };
                    if self.secs[self.cur].kind == SecKind::Bss {
                        self.err("relocation in bss");
                        continue;
                    }
                    self.secs[self.cur].relocs.push(Reloc { off: start + f.at as u32, kind: f.kind, target, addend });
                }
            }
        }
        self.emit(&bytes);
    }

    // ---- line processing ---------------------------------------------------------

    fn line(&mut self, raw: &str) {
        let trimmed = raw.trim_start();
        if raw.starts_with('*') || trimmed.starts_with(';') || trimmed.is_empty() {
            return;
        }
        let line = strip_comment(raw);
        let b = line.as_bytes();
        // label
        let mut label: Option<String> = None;
        let mut rest = line;
        let mut i = 0;
        while i < b.len() && is_label_char(b[i]) {
            i += 1;
        }
        let starts_col0 = !b.is_empty() && !b[0].is_ascii_whitespace();
        if i > 0 && b.get(i) == Some(&b':') {
            label = Some(line[..i].into());
            rest = &line[i + 1..];
            if rest.starts_with(':') {
                rest = &rest[1..]; // `label::` = global label
                if self.active() {
                    self.globals.insert(line[..i].into());
                }
            }
        } else if starts_col0 && i > 0 {
            label = Some(line[..i].into());
            rest = &line[i..];
        }
        let rest = rest.trim_start();
        let (mnem, after) = match rest.find(|c: char| c.is_ascii_whitespace()) {
            Some(p) => (&rest[..p], rest[p..].trim_start()),
            None => (rest, ""),
        };
        let mnem_l = mnem.to_ascii_lowercase();
        let (base, size) = if mnem_l.is_empty() { ("", None) } else { split_size(&mnem_l) };

        // conditionals are processed even in inactive blocks
        if self.conditional(base, size, operand_field(after)) {
            return;
        }
        if !self.active() {
            return;
        }
        // `label = value` with no spaces
        if mnem.starts_with('=') || mnem == "=" {
            if let Some(l) = &label {
                let ex = rest.trim_start_matches('=').trim();
                let ex = operand_field(ex).to_string();
                match self.eval(&ex) {
                    Ok(v) => self.define(l, v, SymKind::Set),
                    Err(e) => self.err(&e),
                }
            }
            return;
        }
        let ops_s = operand_field(after).to_string();
        match base {
            "equ" | "set" | "equr" | "reg" | "macro" => {
                if base == "macro" {
                    let Some(l) = label else {
                        // `macro name[,params]`
                        let mut parts = split_commas(&ops_s).into_iter();
                        let name = parts.next().unwrap_or("").to_ascii_lowercase();
                        let params = parts.map(|p| p.to_string()).collect();
                        self.record_macro(&name, params);
                        return;
                    };
                    let params = split_commas(&ops_s).into_iter().map(|p| p.to_string()).collect();
                    self.record_macro(&l.to_ascii_lowercase(), params);
                    return;
                }
                let Some(l) = label else {
                    self.err(&format!("`{base}` needs a label"));
                    return;
                };
                match self.eval(&ops_s) {
                    Ok(v) => self.define(&l, v, if base == "equ" { SymKind::Equ } else { SymKind::Set }),
                    Err(e) => self.err(&e),
                }
                return;
            }
            _ => {}
        }
        if let Some(l) = &label {
            if !matches!(base, "section" | "org" | "rept" | "endr") {
                // instructions are word aligned; place the label after the padding
                if self.secs[self.cur].kind == SecKind::Text && is_instruction(base) {
                    self.align(2, 0);
                }
                let v = self.here();
                self.define(l, v, SymKind::Label);
            }
        }
        if base.is_empty() {
            return;
        }
        if let Err(e) = self.statement(base, size, &ops_s, label.as_deref()) {
            self.err(&e);
        }
    }

    /// Handle if/else/endif. Returns true if the line was a conditional directive.
    fn conditional(&mut self, base: &str, _size: Option<&str>, ops: &str) -> bool {
        let base = base.trim_start_matches('.');
        match base {
            "if" | "ifeq" | "ifne" | "ifgt" | "ifge" | "iflt" | "ifle" | "ifd" | "ifnd" | "ifdef" | "ifndef" | "ifc"
            | "ifnc" => {
                let parent = self.active();
                let cond = if !parent {
                    false
                } else {
                    match base {
                        "ifd" | "ifdef" | "ifnd" | "ifndef" => {
                            let full = self.full_name(ops.trim());
                            let d = self.syms.get(&full).is_some_and(|s| s.pass == self.pass || s.kind == SymKind::Label);
                            d == (base == "ifd" || base == "ifdef")
                        }
                        "ifc" | "ifnc" => {
                            let p = split_commas(ops);
                            let eq = p.len() == 2 && unquote(p[0]).unwrap_or(p[0].into()) == unquote(p[1]).unwrap_or(p[1].into());
                            eq == (base == "ifc")
                        }
                        _ => match self.eval_const(ops) {
                            Ok(v) => match base {
                                "if" | "ifne" => v != 0,
                                "ifeq" => v == 0,
                                "ifgt" => v > 0,
                                "ifge" => v >= 0,
                                "iflt" => v < 0,
                                _ => v <= 0,
                            },
                            Err(e) => {
                                self.err(&e);
                                false
                            }
                        },
                    }
                };
                self.conds.push(Cond { active: parent && cond, taken: cond, parent });
                true
            }
            "else" | "elseif" => {
                let Some(c) = self.conds.last().copied() else {
                    self.err("else without if");
                    return true;
                };
                let take = if c.taken || !c.parent {
                    false
                } else if base == "elseif" {
                    self.eval_const(ops).map(|v| v != 0).unwrap_or(false)
                } else {
                    true
                };
                *self.conds.last_mut().unwrap() = Cond { active: take, taken: c.taken || take, parent: c.parent };
                true
            }
            "endif" | "endc" => {
                if self.conds.pop().is_none() {
                    self.err("endif without if");
                }
                true
            }
            _ => false,
        }
    }

    /// Collect lines until the matching `end`, honouring nesting of `start`.
    fn collect_block(&mut self, start: &[&str], end: &[&str]) -> Vec<String> {
        let mut body = Vec::new();
        let mut depth = 1;
        while let Some((line, _)) = self.next_line_raw() {
            let t = strip_comment(&line);
            let words: Vec<&str> = t.split_whitespace().take(2).collect();
            let kw = |w: &str| {
                let w = w.to_ascii_lowercase();
                let w = w.trim_start_matches('.').to_string();
                w
            };
            let is = |set: &[&str]| words.iter().take(2).any(|w| set.contains(&kw(w).as_str()));
            if is(start) {
                depth += 1;
            } else if is(end) {
                depth -= 1;
                if depth == 0 {
                    return body;
                }
            }
            body.push(line);
        }
        self.err(&format!("missing `{}`", end[0]));
        body
    }

    /// Next source line without macro-argument substitution (for recording bodies).
    fn next_line_raw(&mut self) -> Option<(String, String)> {
        loop {
            let f = self.stack.last_mut()?;
            if f.idx < f.lines.len() {
                let raw = f.lines[f.idx].clone();
                f.idx += 1;
                // inside a macro, substitute so `rept` bodies see the arguments
                let line = match &f.kind {
                    FrameKind::Macro { args, size, id, params } => substitute(&raw, args, size, *id, params),
                    _ => raw,
                };
                return Some((line, String::new()));
            }
            self.stack.pop();
        }
    }

    fn record_macro(&mut self, name: &str, params: Vec<String>) {
        let body = self.collect_block(&["macro"], &["endm"]);
        if name.is_empty() {
            self.err("macro needs a name");
            return;
        }
        self.macros.insert(name.into(), Macro { params, body: Rc::new(body) });
    }

    fn statement(&mut self, base: &str, size: Option<&str>, ops: &str, label: Option<&str>) -> Result<(), String> {
        // macros first (they may shadow instructions)
        if let Some(m) = self.macros.get(base) {
            let body = m.body.clone();
            let params = Rc::new(m.params.clone());
            self.macro_id += 1;
            let args = macro_args(ops);
            let loc = self.loc.clone();
            if self.stack.len() > 200 {
                return Err("macro recursion too deep".into());
            }
            self.stack.push(Frame {
                name: Rc::new(format!("{loc}: macro {base}")),
                lines: body,
                idx: 0,
                line_base: 1,
                kind: FrameKind::Macro { args, size: size.unwrap_or("").into(), id: self.macro_id, params },
            });
            return Ok(());
        }
        let d = base.strip_prefix('.').unwrap_or(base);
        match d {
            "org" => {
                let v = self.eval_const(ops)? as u32;
                if self.opts.format == Format::Elf {
                    return Err("org is only allowed in binary output".into());
                }
                let s = &mut self.secs[self.cur];
                if s.pc == 0 {
                    s.base = v;
                } else {
                    let here = s.base + s.pc;
                    if v < here {
                        return Err("org moves backwards".into());
                    }
                    self.reserve(v - here, 0);
                }
                Ok(())
            }
            "section" => {
                let p = split_commas(ops);
                let name = p.first().copied().unwrap_or(".text").trim_matches('"');
                self.select_section(name, p.get(1).copied());
                Ok(())
            }
            "text" | "code" | "data" | "bss" | "rodata" => {
                self.select_section(d, None);
                Ok(())
            }
            "xdef" | "global" | "globl" | "public" | "export" => {
                for p in split_commas(ops) {
                    self.globals.insert(self.full_name(p));
                }
                Ok(())
            }
            "xref" | "extern" | "extrn" | "import" => {
                if self.opts.format == Format::Elf {
                    for p in split_commas(ops) {
                        if !self.extern_idx.contains_key(p) && !self.syms.contains_key(p) {
                            self.externs.push(p.into());
                            self.extern_idx.insert(p.into(), self.externs.len() - 1);
                        }
                    }
                }
                Ok(())
            }
            "even" => {
                self.align(2, 0);
                Ok(())
            }
            "align" | "balign" | "p2align" => {
                let p = split_commas(ops);
                let v = self.eval_const(p.first().copied().unwrap_or("1"))?;
                let n = if d == "balign" { v as u32 } else { 1u32 << v.clamp(0, 16) };
                let fill = match p.get(1) {
                    Some(f) => self.eval_const(f)? as u8,
                    None => 0,
                };
                self.align(n, fill);
                Ok(())
            }
            "cnop" => {
                let p = split_commas(ops);
                if p.len() != 2 {
                    return Err("cnop offset,alignment".into());
                }
                let off = self.eval_const(p[0])? as u32;
                let al = self.eval_const(p[1])? as u32;
                if al > 1 {
                    let pc = self.here().off as u32;
                    let pad = (al - (pc.wrapping_sub(off)) % al) % al;
                    self.reserve(pad, 0);
                }
                Ok(())
            }
            "dc" | "byte" | "word" | "short" | "long" | "int" => {
                let sz = match d {
                    "byte" => "b",
                    "word" | "short" => "w",
                    "long" | "int" => "l",
                    _ => size.unwrap_or("w"),
                };
                self.dc(sz, ops)
            }
            "ascii" | "asciz" | "string" => {
                for p in split_commas(ops) {
                    let s = unquote(p).ok_or("expected a string")?;
                    self.emit(s.as_bytes());
                    if d != "ascii" {
                        self.emit(&[0]);
                    }
                }
                Ok(())
            }
            "ds" | "space" | "skip" | "zero" => {
                let unit: u32 = match size.unwrap_or(if d == "ds" { "w" } else { "b" }) {
                    "b" => 1,
                    "w" => 2,
                    "l" | "s" => 4,
                    "d" => 8,
                    "x" | "p" => 12,
                    _ => return Err("bad size".into()),
                };
                if unit > 1 && d == "ds" {
                    self.align(2, 0);
                }
                let p = split_commas(ops);
                let n = self.eval_const(p.first().copied().unwrap_or("0"))?;
                if n < 0 {
                    return Err("negative size".into());
                }
                let fill = match p.get(1) {
                    Some(f) => self.eval_const(f)? as u8,
                    None => 0,
                };
                self.reserve(n as u32 * unit, fill);
                Ok(())
            }
            "dcb" | "blk" | "fill" => {
                let p = split_commas(ops);
                let n = self.eval_const(p.first().copied().ok_or("dcb count[,value]")?)?;
                if n < 0 {
                    return Err("negative count".into());
                }
                let fill = p.get(1).copied().unwrap_or("0");
                let unit = size.unwrap_or("w");
                if unit != "b" {
                    self.align(2, 0);
                }
                let v = self.eval(fill)?;
                if v.is_abs() || v.is_unknown() {
                    let bytes: Vec<u8> = match unit {
                        "b" => vec![v.off as u8],
                        "w" => (v.off as u16).to_be_bytes().to_vec(),
                        "l" => (v.off as u32).to_be_bytes().to_vec(),
                        _ => return Err("bad dcb size".into()),
                    };
                    for _ in 0..n {
                        self.emit(&bytes);
                    }
                    Ok(())
                } else {
                    for _ in 0..n {
                        self.dc(unit, fill)?;
                    }
                    Ok(())
                }
            }
            "include" => {
                let name = unquote(ops).unwrap_or_else(|| ops.trim().into());
                let lines = match self.file_cache.get(&name) {
                    Some(l) => l.clone(),
                    None => {
                        let data = self.files.read(&name).ok_or_else(|| format!("cannot read `{name}`"))?;
                        let text = String::from_utf8_lossy(&data);
                        let l: Rc<Vec<String>> = Rc::new(text.lines().map(String::from).collect());
                        self.file_cache.insert(name.clone(), l.clone());
                        l
                    }
                };
                if self.stack.len() > 64 {
                    return Err("includes nested too deeply".into());
                }
                self.stack.push(Frame { name: Rc::new(name), lines, idx: 0, line_base: 1, kind: FrameKind::File });
                Ok(())
            }
            "incbin" => {
                let p = split_commas(ops);
                let name = unquote(p.first().copied().unwrap_or("")).unwrap_or_else(|| p.first().copied().unwrap_or("").into());
                let data = self.files.read(&name).ok_or_else(|| format!("cannot read `{name}`"))?;
                self.emit(&data);
                Ok(())
            }
            "rept" => {
                let n = self.eval_const(ops)?;
                let body = self.collect_block(&["rept", "irp"], &["endr"]);
                let mut lines = Vec::new();
                for _ in 0..n.max(0) {
                    lines.extend(body.iter().cloned());
                }
                let loc = self.loc.clone();
                self.stack.push(Frame { name: Rc::new(format!("{loc}: rept")), lines: Rc::new(lines), idx: 0, line_base: 1, kind: FrameKind::Rept });
                Ok(())
            }
            "endr" => Err("endr without rept".into()),
            "endm" => Err("endm without macro".into()),
            "mexit" => {
                // drop the innermost macro frame (and anything nested in it)
                while let Some(f) = self.stack.pop() {
                    if matches!(f.kind, FrameKind::Macro { .. }) {
                        break;
                    }
                }
                Ok(())
            }
            "fail" | "error" => Err(format!("fail: {}", unquote(ops).unwrap_or_else(|| ops.into()))),
            "echo" | "printt" | "print" | "warning" | "printv" => Ok(()),
            "end" => {
                self.ended = true;
                Ok(())
            }
            "opt" | "machine" | "mc68000" | "mc68010" | "mc68020" | "mc68030" | "mc68881" | "mc68882" | "cpu" | "fpu"
            | "mmu" | "list" | "nolist" | "page" | "title" | "ttl" | "idnt" | "near" | "far" | "file" | "type" | "size"
            | "ident" | "weak" | "local" | "nopage" | "spc" | "llen" | "plen" | "output" | "debug" | "cfi_startproc"
            | "cfi_endproc" => Ok(()),
            _ => {
                if base.starts_with('.') {
                    return Err(format!("unknown directive `{base}`"));
                }
                let _ = label;
                self.instruction(base, size, ops)
            }
        }
    }

    fn dc(&mut self, sz: &str, ops: &str) -> Result<(), String> {
        let unit = match sz {
            "b" => 1,
            "w" => 2,
            "l" => 4,
            "s" => 4,
            "d" => 8,
            "x" => 12,
            _ => return Err(format!("bad dc size .{sz}")),
        };
        if unit > 1 {
            self.align(2, 0);
        }
        for p in split_commas(ops) {
            if let Some(s) = unquote(p).filter(|s| sz == "b" || s.len() > unit) {
                // strings in dc.w/dc.l are padded to the unit size
                let mut bytes = s.into_bytes();
                while bytes.len() % unit != 0 {
                    bytes.push(0);
                }
                self.emit(&bytes);
                continue;
            }
            match sz {
                "s" | "d" | "x" => {
                    let f = match operand::float_literal(p) {
                        Some(f) => f,
                        None => self.eval_const(p)? as f64,
                    };
                    match sz {
                        "s" => self.emit(&(f as f32).to_bits().to_be_bytes()),
                        "d" => self.emit(&f.to_bits().to_be_bytes()),
                        _ => self.emit(&float::to_extended(f)),
                    }
                }
                _ => {
                    let v = self.eval(p)?;
                    let mut e = Enc::new(self.here());
                    let kind = match sz {
                        "b" => FixKind::Abs8,
                        "w" => FixKind::Abs16,
                        _ => FixKind::Abs32,
                    };
                    encode_value(&mut e, kind, v)?;
                    self.emit_enc(e);
                }
            }
        }
        Ok(())
    }

    fn instruction(&mut self, m: &str, size: Option<&str>, ops: &str) -> Result<(), String> {
        let sz = match size {
            Some(s) => Some(Sz::from_suffix(s).ok_or_else(|| format!("bad size suffix .{s}"))?),
            None => None,
        };
        if self.secs[self.cur].kind == SecKind::Bss {
            return Err("instruction in a bss section".into());
        }
        self.align(2, 0);
        let mut parsed = Vec::new();
        for p in split_commas(ops) {
            let mut ctx = SymCtx { a: self };
            parsed.push(operand::parse(p, &mut ctx)?);
        }
        let mut e = Enc::new(self.here());
        let idx = self.cur_sizes.len();
        e.prev_size = self.prev_sizes.get(idx).copied();
        self.cur_sizes.push(0);
        encode::encode(&mut e, m, sz, &parsed)?;
        self.cur_sizes[idx] = e.b.len();
        let err = e.err.take();
        self.emit_enc(e);
        match err {
            Some(msg) => Err(msg),
            None => Ok(()),
        }
    }

    // ---- output --------------------------------------------------------------------------

    fn write_binary(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let Some(origin) = self.secs.iter().filter(|s| s.kind != SecKind::Bss && !s.data.is_empty()).map(|s| s.base).min() else {
            return out;
        };
        for s in &self.secs {
            if s.kind == SecKind::Bss || s.data.is_empty() {
                continue;
            }
            let off = (s.base - origin) as usize;
            if out.len() < off {
                out.resize(off, 0);
            }
            if out.len() > off {
                out.truncate(off);
            }
            out.extend_from_slice(&s.data);
        }
        out
    }

    fn write_elf(&self) -> Vec<u8> {
        // section indices: 1..=n user sections, then rela sections, symtab, strtab, shstrtab
        let n = self.secs.len();
        let mut shstr = vec![0u8];
        let mut add_name = |s: &str| {
            let o = shstr.len() as u32;
            shstr.extend_from_slice(s.as_bytes());
            shstr.push(0);
            o
        };
        // symbols
        let mut strtab = vec![0u8];
        let mut add_str = |s: &str| {
            let o = strtab.len() as u32;
            strtab.extend_from_slice(s.as_bytes());
            strtab.push(0);
            o
        };
        let mut syms: Vec<Sym32> = vec![Sym32::default()];
        for i in 0..n {
            syms.push(Sym32 { name: 0, value: 0, info: STT_SECTION, shndx: (i + 1) as u16 });
        }
        // local labels
        for (name, s) in &self.syms {
            if self.globals.contains(name) {
                continue;
            }
            if let (SymKind::Label, Base::Sec(i)) = (s.kind, s.val.base) {
                let t = if self.secs[i].kind == SecKind::Text { STT_FUNC } else { STT_OBJECT };
                syms.push(Sym32 { name: add_str(name), value: s.val.off as u32, info: t, shndx: (i + 1) as u16 });
            }
        }
        let first_global = syms.len();
        let mut ext_sym = vec![0usize; self.externs.len()];
        for g in &self.globals {
            if let Some(s) = self.syms.get(g) {
                let (shndx, t) = match s.val.base {
                    Base::Sec(i) => ((i + 1) as u16, if self.secs[i].kind == SecKind::Text { STT_FUNC } else { STT_OBJECT }),
                    Base::Abs => (SHN_ABS, STT_NOTYPE),
                    _ => continue,
                };
                syms.push(Sym32 { name: add_str(g), value: s.val.off as u32, info: (STB_GLOBAL << 4) | t, shndx });
            }
        }
        for (i, e) in self.externs.iter().enumerate() {
            // `xref` of a name defined here: nothing refers to it as an extern
            if self.syms.contains_key(e) {
                continue;
            }
            ext_sym[i] = syms.len();
            syms.push(Sym32 { name: add_str(e), value: 0, info: STB_GLOBAL << 4, shndx: SHN_UNDEF });
        }

        let mut out = vec![0u8; 52];
        let mut shdrs: Vec<Shdr> = vec![Shdr::default()];
        for s in &self.secs {
            let off = align4(&mut out);
            out.extend_from_slice(&s.data);
            let (stype, flags) = match s.kind {
                SecKind::Text => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
                SecKind::Data => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
                SecKind::Rodata => (SHT_PROGBITS, SHF_ALLOC),
                SecKind::Bss => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
            };
            shdrs.push(Shdr { name: add_name(&s.name), stype, flags, offset: off, size: s.pc, addralign: s.align, ..Default::default() });
        }
        let symtab_idx = (n + 1 + self.secs.iter().filter(|s| !s.relocs.is_empty()).count()) as u32;
        for (i, s) in self.secs.iter().enumerate() {
            if s.relocs.is_empty() {
                continue;
            }
            let off = align4(&mut out);
            for r in &s.relocs {
                let (sym, addend) = match r.target {
                    Base::Sec(si) => (si + 1, r.addend),
                    Base::Ext(x) => (ext_sym[x], r.addend),
                    _ => (0, r.addend),
                };
                let t = match r.kind {
                    FixKind::Abs8 => R_68K_8,
                    FixKind::Abs16 => R_68K_16,
                    FixKind::Abs32 => R_68K_32,
                    FixKind::Pc8 => R_68K_PC8,
                    FixKind::Pc16 => R_68K_PC16,
                    FixKind::Pc32 => R_68K_PC32,
                };
                out.extend_from_slice(&r.off.to_be_bytes());
                out.extend_from_slice(&(((sym as u32) << 8) | t as u32).to_be_bytes());
                out.extend_from_slice(&(addend as i32).to_be_bytes());
            }
            let name = format!(".rela{}", s.name);
            shdrs.push(Shdr {
                name: add_name(&name),
                stype: SHT_RELA,
                offset: off,
                size: (s.relocs.len() * 12) as u32,
                link: symtab_idx,
                info: (i + 1) as u32,
                addralign: 4,
                entsize: 12,
                ..Default::default()
            });
        }
        let off = align4(&mut out);
        for s in &syms {
            Sym { name: s.name, value: s.value, size: 0, info: s.info, other: 0, shndx: s.shndx }.write(&mut out);
        }
        shdrs.push(Shdr {
            name: add_name(".symtab"),
            stype: SHT_SYMTAB,
            offset: off,
            size: (syms.len() * 16) as u32,
            link: symtab_idx + 1,
            info: first_global as u32,
            addralign: 4,
            entsize: 16,
            ..Default::default()
        });
        let off = out.len() as u32;
        out.extend_from_slice(&strtab);
        shdrs.push(Shdr { name: add_name(".strtab"), stype: SHT_STRTAB, offset: off, size: strtab.len() as u32, addralign: 1, ..Default::default() });
        let shs_name = add_name(".shstrtab");
        let off = out.len() as u32;
        out.extend_from_slice(&shstr);
        shdrs.push(Shdr { name: shs_name, stype: SHT_STRTAB, offset: off, size: shstr.len() as u32, addralign: 1, ..Default::default() });
        let shoff = align4(&mut out);
        for s in &shdrs {
            s.write(&mut out);
        }
        let eh = Ehdr {
            class: 1,
            data: 2,
            etype: ET_REL,
            machine: EM_68K,
            shoff,
            ehsize: 52,
            shentsize: 40,
            shnum: shdrs.len() as u16,
            shstrndx: (shdrs.len() - 1) as u16,
            ..Default::default()
        };
        let mut h = Vec::new();
        eh.write(&mut h);
        out[..52].copy_from_slice(&h);
        out
    }
}

#[derive(Default, Clone, Copy)]
struct Sym32 {
    name: u32,
    value: u32,
    info: u8,
    shndx: u16,
}

fn align4(v: &mut Vec<u8>) -> u32 {
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v.len() as u32
}

fn encode_value(e: &mut Enc, kind: FixKind, v: Val) -> Result<(), String> {
    let n = match kind {
        FixKind::Abs8 => 1,
        FixKind::Abs16 => 2,
        _ => 4,
    };
    let at = e.b.len();
    e.b.resize(at + n, 0);
    if v.is_abs() {
        let x = v.off;
        let ok = match n {
            1 => (-128..=255).contains(&x),
            2 => (-32768..=65535).contains(&x),
            _ => (-(1i64 << 31)..(1i64 << 32)).contains(&x),
        };
        if !ok {
            return Err(format!("value {x} does not fit"));
        }
        let bytes = (x as u32).to_be_bytes();
        e.b[at..].copy_from_slice(&bytes[4 - n..]);
    } else if !v.is_unknown() {
        e.fix.push(encode::Fixup { at, kind, val: v, pcbase: 0 });
    }
    Ok(())
}

fn is_instruction(base: &str) -> bool {
    !base.is_empty()
        && !base.starts_with('.')
        && !matches!(
            base,
            "dc" | "ds" | "dcb" | "blk" | "even" | "align" | "cnop" | "equ" | "set" | "section" | "text" | "data" | "bss"
                | "rodata" | "xdef" | "xref" | "global" | "org" | "include" | "incbin" | "macro" | "rept" | "end"
        )
}

/// Split macro invocation arguments (commas; `<...>` groups an argument).
fn macro_args(ops: &str) -> Vec<String> {
    let mut out = Vec::new();
    if ops.trim().is_empty() {
        return out;
    }
    let b = ops.as_bytes();
    let mut cur = String::new();
    let mut i = 0;
    let mut depth = 0;
    let mut q = 0u8;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            cur.push(c as char);
            if c == q {
                q = 0;
            }
        } else if c == b'<' && cur.trim().is_empty() {
            // <grouped, argument>
            let end = ops[i + 1..].find('>').map_or(b.len(), |e| i + 1 + e);
            cur.push_str(&ops[i + 1..end]);
            i = end;
        } else {
            match c {
                b'"' | b'\'' => {
                    q = c;
                    cur.push(c as char);
                }
                b'(' | b'[' => {
                    depth += 1;
                    cur.push(c as char);
                }
                b')' | b']' => {
                    depth -= 1;
                    cur.push(c as char);
                }
                b',' if depth == 0 => out.push(core::mem::take(&mut cur).trim().to_string()),
                _ => cur.push(c as char),
            }
        }
        i += 1;
    }
    out.push(cur.trim().to_string());
    out
}

/// Substitute `\1`..`\9`, `\0` (size), `\@` (unique id) and `\name` in a macro body line.
fn substitute(line: &str, args: &[String], size: &str, id: u32, params: &[String]) -> String {
    if !line.contains('\\') {
        return line.to_string();
    }
    let mut out = String::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let c = b[i + 1];
            if c.is_ascii_digit() {
                let n = (c - b'0') as usize;
                if n == 0 {
                    out.push_str(size);
                } else if let Some(a) = args.get(n - 1) {
                    out.push_str(a);
                }
                i += 2;
                continue;
            }
            if c == b'@' {
                out.push_str(&format!("_{id:06}"));
                i += 2;
                continue;
            }
            if c == b'#' {
                out.push_str(&format!("{}", args.len()));
                i += 2;
                continue;
            }
            // named parameter
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let name = &line[i + 1..j];
            if let Some(k) = params.iter().position(|p| p.eq_ignore_ascii_case(name)) {
                out.push_str(args.get(k).map(String::as_str).unwrap_or(""));
                i = j;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Convenience for host tools: assemble with no include support.
pub struct NoFiles;
impl Files for NoFiles {
    fn read(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
}
