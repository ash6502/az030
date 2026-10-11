//! Tokens and the lexer (translation phases 1-3: line splicing, comments, tokens).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// a preprocessing number, as written
    Num(String),
    /// a character constant, as written (with quotes and any prefix)
    Char(String),
    /// a string literal, as written (with quotes and any prefix)
    Str(String),
    Punct(&'static str),
    /// a stray character
    Other(char),
    /// `#include <...>` header name (only produced on request)
    Header(String),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    /// index into the file name table
    pub file: u16,
    pub line: u32,
    /// first token on its line
    pub bol: bool,
    /// preceded by white space
    pub space: bool,
    /// macros that must not be expanded again in this token
    pub hide: Option<Rc<Vec<String>>>,
}

impl Token {
    pub fn is(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::Punct(x) if *x == p)
    }
    pub fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(s) => Some(s),
            _ => None,
        }
    }
    pub fn hidden(&self, name: &str) -> bool {
        self.hide.as_ref().is_some_and(|h| h.iter().any(|x| x == name))
    }
    /// The token's spelling.
    pub fn text(&self) -> String {
        match &self.tok {
            Tok::Ident(s) | Tok::Num(s) | Tok::Char(s) | Tok::Str(s) => s.clone(),
            Tok::Punct(p) => String::from(*p),
            Tok::Other(c) => String::from(*c),
            Tok::Header(h) => alloc::format!("<{h}>"),
        }
    }
}

const PUNCTS: [&str; 48] = [
    "...", "<<=", ">>=", "->", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "##",
    "[", "]", "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<", ">", "^", "|", "?", ":", ";", "=", ",", "#",
];

/// Split a source file into tokens. Errors are reported as (line, message).
pub fn lex(src: &str, file: u16) -> Result<Vec<Token>, (u32, String)> {
    // phase 2: splice lines, keeping the original line number of each character
    let mut chars: Vec<char> = Vec::with_capacity(src.len());
    let mut lines: Vec<u32> = Vec::with_capacity(src.len());
    let mut line = 1u32;
    let raw: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == '\\' && i + 1 < raw.len() && (raw[i + 1] == '\n' || (raw[i + 1] == '\r' && raw.get(i + 2) == Some(&'\n'))) {
            i += if raw[i + 1] == '\r' { 3 } else { 2 };
            line += 1;
            continue;
        }
        if c == '\r' {
            i += 1;
            continue;
        }
        chars.push(c);
        lines.push(line);
        if c == '\n' {
            line += 1;
        }
        i += 1;
    }
    lines.push(line);
    let mut out = Vec::new();
    let mut i = 0;
    let mut bol = true;
    let mut space = false;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        let ln = lines[i];
        if c == '\n' {
            bol = true;
            space = false;
            i += 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\x0c' || c == '\x0b' {
            space = true;
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            space = true;
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            loop {
                if i + 1 >= n {
                    return Err((ln, "unterminated comment".into()));
                }
                if chars[i] == '*' && chars[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            space = true;
            continue;
        }
        let start = i;
        let tok = if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            i += 1;
            while i < n {
                let d = chars[i];
                if matches!(d, 'e' | 'E' | 'p' | 'P') && i + 1 < n && (chars[i + 1] == '+' || chars[i + 1] == '-') {
                    i += 2;
                } else if d.is_ascii_alphanumeric() || d == '_' || d == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            Tok::Num(chars[start..i].iter().collect())
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            // string/char prefixes L u U u8
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '$') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            if j < n && (chars[j] == '"' || chars[j] == '\'') && matches!(word.as_str(), "L" | "u" | "U" | "u8") {
                let q = chars[j];
                i = j + 1;
                while i < n && chars[i] != q {
                    if chars[i] == '\n' {
                        return Err((ln, "missing terminating quote".into()));
                    }
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                let s: String = chars[start..i.min(n)].iter().collect();
                if q == '"' { Tok::Str(s) } else { Tok::Char(s) }
            } else {
                i = j;
                Tok::Ident(word)
            }
        } else if c == '"' || c == '\'' {
            i += 1;
            while i < n && chars[i] != c {
                if chars[i] == '\n' {
                    return Err((ln, alloc::format!("missing terminating {c} character")));
                }
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i >= n {
                return Err((ln, alloc::format!("missing terminating {c} character")));
            }
            i += 1;
            let s: String = chars[start..i].iter().collect();
            if c == '"' { Tok::Str(s) } else { Tok::Char(s) }
        } else {
            let mut found = None;
            for p in PUNCTS {
                if p.as_bytes()[0] as char != c {
                    continue;
                }
                if p.chars().enumerate().all(|(k, pc)| chars.get(i + k) == Some(&pc)) {
                    found = Some(p);
                    break;
                }
            }
            match found {
                Some(p) => {
                    i += p.len();
                    Tok::Punct(p)
                }
                None => {
                    i += 1;
                    Tok::Other(c)
                }
            }
        };
        out.push(Token { tok, file, line: ln, bol, space, hide: None });
        bol = false;
        space = false;
    }
    Ok(out)
}

/// The punctuator for a spelling (used when pasting tokens).
pub fn punct(s: &str) -> Option<&'static str> {
    PUNCTS.iter().find(|p| **p == s).copied()
}
