//! Syntax highlighting for C, 68k assembly, shell scripts and Rust.

use crate::buffer::Buffer;
use rt::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    None,
    C,
    Asm,
    Sh,
    Rust,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Normal,
    Comment,
    Str,
    Keyword,
    Type,
    Number,
    Preproc,
}

impl Style {
    pub fn code(self) -> &'static str {
        match self {
            Style::Normal => "",
            Style::Comment => "\x1b[36m",
            Style::Str => "\x1b[32m",
            Style::Keyword => "\x1b[1;33m",
            Style::Type => "\x1b[1;32m",
            Style::Number => "\x1b[35m",
            Style::Preproc => "\x1b[1;34m",
        }
    }
}

/// Highlighter state carried between lines: inside a block comment.
pub type State = bool;

pub fn detect(name: &str, first: Option<&Vec<char>>) -> Lang {
    let ext = rt::path::extension(name).unwrap_or("");
    match ext {
        "c" | "h" | "cc" | "cpp" => return Lang::C,
        "s" | "S" | "asm" | "inc" | "i" => return Lang::Asm,
        "sh" => return Lang::Sh,
        "rs" => return Lang::Rust,
        _ => {}
    }
    let base = rt::path::basename(name);
    if matches!(base, "profile" | ".profile" | "rc" | ".shrc") {
        return Lang::Sh;
    }
    if let Some(l) = first {
        let s: String = l.iter().collect();
        if s.starts_with("#!") && s.contains("sh") {
            return Lang::Sh;
        }
    }
    Lang::None
}

const C_KW: &[&str] = &[
    "auto", "break", "case", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "register", "return",
    "sizeof", "static", "struct", "switch", "typedef", "union", "volatile", "while", "const", "restrict", "_Bool", "true", "false", "NULL",
];
const C_TY: &[&str] = &["char", "short", "int", "long", "float", "double", "void", "signed", "unsigned", "size_t", "FILE", "bool", "int8_t", "uint8_t", "int16_t", "uint16_t", "int32_t", "uint32_t", "int64_t", "uint64_t"];
const SH_KW: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "break", "continue",
    "export", "local", "readonly", "unset", "shift", "exit", "set", "trap", "eval", "exec", "cd", "echo", "read", "test",
];
const RS_KW: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
];
const RS_TY: &[&str] = &["u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result", "Box"];

/// The highlighter state at the start of line `row`.
pub fn line_states(b: &Buffer, row: usize) -> State {
    if !matches!(b.lang, Lang::C | Lang::Rust) {
        return false;
    }
    let mut st = false;
    for l in &b.lines[..row.min(b.lines.len())] {
        st = highlight(b.lang, l, st).1;
    }
    st
}

fn word_style(lang: Lang, w: &str) -> Style {
    let (kw, ty): (&[&str], &[&str]) = match lang {
        Lang::C => (C_KW, C_TY),
        Lang::Sh => (SH_KW, &[]),
        Lang::Rust => (RS_KW, RS_TY),
        _ => (&[], &[]),
    };
    if kw.contains(&w) {
        Style::Keyword
    } else if ty.contains(&w) {
        Style::Type
    } else {
        Style::Normal
    }
}

/// Styles for each character of a line, and the state after it.
pub fn highlight(lang: Lang, l: &[char], mut in_comment: State) -> (Vec<Style>, State) {
    let n = l.len();
    let mut st = vec![Style::Normal; n];
    if lang == Lang::None {
        return (st, false);
    }
    let mut i = 0;
    // asm: labels in column 0 and the mnemonic
    if lang == Lang::Asm {
        let first_non_ws = l.iter().position(|c| !c.is_whitespace()).unwrap_or(n);
        let mut j = first_non_ws;
        if first_non_ws == 0 && n > 0 && l[0] != ';' && l[0] != '*' {
            while j < n && !l[j].is_whitespace() && l[j] != ';' {
                st[j] = Style::Type;
                j += 1;
            }
            while j < n && l[j].is_whitespace() {
                j += 1;
            }
        }
        let ms = j;
        while j < n && !l[j].is_whitespace() && l[j] != ';' {
            j += 1;
        }
        for s in st.iter_mut().take(j).skip(ms) {
            *s = Style::Keyword;
        }
        i = j;
    }
    if lang == Lang::C {
        let fnw = l.iter().position(|c| !c.is_whitespace());
        if !in_comment && fnw.is_some_and(|p| l[p] == '#') {
            // preprocessor line: highlight the directive, then continue normally
            let p = fnw.unwrap();
            let mut j = p + 1;
            while j < n && (l[j].is_alphanumeric() || l[j] == ' ') && !(j > p + 1 && l[j] == ' ' && l[j - 1] != ' ' && l[j - 1] != '#') {
                j += 1;
            }
            for s in st.iter_mut().take(j).skip(p) {
                *s = Style::Preproc;
            }
            i = j;
        }
    }
    while i < n {
        if in_comment {
            st[i] = Style::Comment;
            if l[i] == '*' && i + 1 < n && l[i + 1] == '/' {
                st[i + 1] = Style::Comment;
                i += 2;
                in_comment = false;
                continue;
            }
            i += 1;
            continue;
        }
        let c = l[i];
        let next = l.get(i + 1).copied();
        let line_comment = match lang {
            Lang::C | Lang::Rust => c == '/' && next == Some('/'),
            Lang::Asm => c == ';' || (i == 0 && c == '*'),
            Lang::Sh => c == '#' && (i == 0 || l[i - 1].is_whitespace()),
            Lang::None => false,
        };
        if line_comment {
            for s in st.iter_mut().skip(i) {
                *s = Style::Comment;
            }
            break;
        }
        if matches!(lang, Lang::C | Lang::Rust) && c == '/' && next == Some('*') {
            in_comment = true;
            st[i] = Style::Comment;
            st[i + 1] = Style::Comment;
            i += 2;
            continue;
        }
        if c == '"' || (c == '\'' && lang != Lang::Rust) {
            let q = c;
            st[i] = Style::Str;
            i += 1;
            while i < n {
                st[i] = Style::Str;
                if l[i] == '\\' && lang != Lang::Sh || (l[i] == '\\' && q == '"') {
                    if i + 1 < n {
                        st[i + 1] = Style::Str;
                    }
                    i += 2;
                    continue;
                }
                if l[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() || (lang == Lang::Asm && (c == '$' || c == '#' || c == '%') && next.is_some_and(|x| x.is_ascii_hexdigit() || x == '$')) {
            let prev_word = i > 0 && (l[i - 1].is_alphanumeric() || l[i - 1] == '_');
            if !prev_word {
                let s = i;
                i += 1;
                while i < n && (l[i].is_ascii_alphanumeric() || l[i] == '.' || l[i] == '$') {
                    i += 1;
                }
                for x in st.iter_mut().take(i).skip(s) {
                    *x = Style::Number;
                }
                continue;
            }
        }
        if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < n && (l[i].is_alphanumeric() || l[i] == '_') {
                i += 1;
            }
            let w: String = l[s..i].iter().collect();
            let style = word_style(lang, &w);
            let style = if style == Style::Normal && lang == Lang::Rust && i < n && l[i] == '!' { Style::Preproc } else { style };
            for x in st.iter_mut().take(i).skip(s) {
                *x = style;
            }
            continue;
        }
        if lang == Lang::Sh && c == '$' {
            let s = i;
            i += 1;
            if i < n && l[i] == '{' {
                while i < n && l[i] != '}' {
                    i += 1;
                }
                i += 1;
            } else {
                while i < n && (l[i].is_alphanumeric() || l[i] == '_' || "?#$@*!".contains(l[i]) && i == s + 1) {
                    i += 1;
                }
            }
            for x in st.iter_mut().take(i.min(n)).skip(s) {
                *x = Style::Type;
            }
            continue;
        }
        i += 1;
    }
    (st, in_comment)
}
