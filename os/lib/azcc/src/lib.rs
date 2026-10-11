//! azcc: a C compiler for the 68030 (with 68881/68882 floating point).
//!
//! It implements most of C99 (no complex numbers or variable-length arrays) and
//! emits assembly for the azas assembler. The crate is `no_std` + `alloc` so the
//! same compiler runs on the build host (for building libc) and as /bin/cc on
//! AZ/UX.

#![no_std]

extern crate alloc;

pub mod ast;
pub mod codegen;
pub mod lex;
pub mod parse;
pub mod pp;
pub mod types;

use alloc::string::String;
use alloc::vec::Vec;
pub use pp::Files;

#[derive(Default, Clone)]
pub struct Options {
    /// -I directories (searched for "..." and <...> includes)
    pub include_dirs: Vec<String>,
    /// system header directories (after -I)
    pub system_dirs: Vec<String>,
    /// -D name[=value]
    pub defines: Vec<(String, String)>,
    /// -U name
    pub undefs: Vec<String>,
}

pub struct Output {
    pub text: String,
    pub warnings: Vec<String>,
}

fn setup<'f>(name: &str, src: &str, opts: &Options, files: &'f mut dyn Files) -> pp::Pp<'f> {
    let mut p = pp::Pp::new(files);
    p.include_dirs = opts.include_dirs.clone();
    p.system_dirs = opts.system_dirs.clone();
    for (n, v) in &opts.defines {
        p.define_str(n, v);
    }
    for n in &opts.undefs {
        p.undef(n);
    }
    p.push_file(name, src);
    p
}

/// Preprocess only (cc -E).
pub fn preprocess(name: &str, src: &str, opts: &Options, files: &mut dyn Files) -> Result<Output, Vec<String>> {
    let mut p = setup(name, src, opts, files);
    let mut text = String::new();
    let mut cur: Option<(u16, u32)> = None;
    while let Some(t) = p.next() {
        let line = p.line_of(&t) as u32;
        match cur {
            Some((f, l)) if f == t.file && line >= l && line <= l + 8 => {
                for _ in l..line {
                    text.push('\n');
                }
                if line == l && t.space {
                    text.push(' ');
                }
            }
            _ => {
                if cur.is_some() {
                    text.push('\n');
                }
                text.push_str(&alloc::format!("# {} \"{}\"\n", line, p.file_names[t.file as usize]));
            }
        }
        if cur.is_some_and(|(f, l)| f == t.file && line > l) || cur.is_none_or(|(f, _)| f != t.file) {
            // indentation is not preserved; keep tokens apart
        }
        cur = Some((t.file, line));
        text.push_str(&t.text());
    }
    text.push('\n');
    if !p.errors.is_empty() {
        return Err(p.errors);
    }
    Ok(Output { text, warnings: p.warnings })
}

/// Compile C source to assembly text.
pub fn compile(name: &str, src: &str, opts: &Options, files: &mut dyn Files) -> Result<Output, Vec<String>> {
    let mut p = setup(name, src, opts, files);
    let mut parser = parse::Parser::new(&mut p);
    parser.translation_unit();
    let mut errors = core::mem::take(&mut parser.errors);
    let mut warnings = core::mem::take(&mut parser.warnings);
    let unit = core::mem::replace(&mut parser.unit, ast::Unit { functions: Vec::new(), globals: Vec::new(), strings: Vec::new(), structs: Vec::new() });
    drop(parser);
    let mut all = core::mem::take(&mut p.errors);
    all.append(&mut errors);
    warnings.append(&mut p.warnings);
    if !all.is_empty() {
        return Err(all);
    }
    match codegen::Gen::new(&unit).unit(name) {
        Ok(text) => Ok(Output { text, warnings }),
        Err(e) => Err(e),
    }
}
