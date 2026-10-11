//! The preprocessor (translation phase 4): directives and macro expansion.

use crate::lex::{self, Tok, Token};
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Where `#include` files come from.
pub trait Files {
    /// Read a file by path (as built from the search directories).
    fn read(&mut self, path: &str) -> Option<String>;
}

#[derive(Clone)]
struct Macro {
    /// None for object-like macros
    params: Option<Vec<String>>,
    variadic: bool,
    body: Vec<Token>,
    builtin: bool,
}

struct Cond {
    /// some branch of this #if has been taken
    taken: bool,
    /// the current branch is active
    active: bool,
    /// the enclosing context was active
    outer: bool,
    seen_else: bool,
}

struct Source {
    toks: VecDeque<Token>,
    dir: String,
    conds: usize,
}

pub struct Pp<'f> {
    macros: BTreeMap<String, Macro>,
    files: &'f mut dyn Files,
    pub file_names: Vec<String>,
    stack: Vec<Source>,
    conds: Vec<Cond>,
    pub include_dirs: Vec<String>,
    pub system_dirs: Vec<String>,
    once: BTreeSet<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// tokens produced by expansion, waiting to be returned
    pending: VecDeque<Token>,
    /// file and line overrides from #line
    line_delta: BTreeMap<u16, i64>,
    depth: usize,
}

fn tok(t: Tok, like: &Token) -> Token {
    Token { tok: t, file: like.file, line: like.line, bol: false, space: like.space, hide: like.hide.clone() }
}

/// Spell a token list as text (for # stringizing and -E).
pub fn spell(toks: &[Token]) -> String {
    let mut s = String::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        s.push_str(&t.text());
    }
    s
}

fn stringize(toks: &[Token]) -> String {
    let mut s = String::from("\"");
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        let txt = t.text();
        if matches!(t.tok, Tok::Str(_) | Tok::Char(_)) {
            for c in txt.chars() {
                if c == '"' || c == '\\' {
                    s.push('\\');
                }
                s.push(c);
            }
        } else {
            s.push_str(&txt);
        }
    }
    s.push('"');
    s
}

fn with_hide(t: &Token, name: &str) -> Token {
    let mut t = t.clone();
    let mut h: Vec<String> = t.hide.as_deref().cloned().unwrap_or_default();
    if !h.iter().any(|x| x == name) {
        h.push(name.into());
    }
    t.hide = Some(Rc::new(h));
    t
}

impl<'f> Pp<'f> {
    pub fn new(files: &'f mut dyn Files) -> Pp<'f> {
        let mut p = Pp {
            macros: BTreeMap::new(),
            files,
            file_names: Vec::new(),
            stack: Vec::new(),
            conds: Vec::new(),
            include_dirs: Vec::new(),
            system_dirs: Vec::new(),
            once: BTreeSet::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            pending: VecDeque::new(),
            line_delta: BTreeMap::new(),
            depth: 0,
        };
        for (n, v) in [
            ("__STDC__", "1"),
            ("__STDC_VERSION__", "199901L"),
            ("__STDC_HOSTED__", "1"),
            ("__m68k__", "1"),
            ("__mc68000__", "1"),
            ("__mc68020__", "1"),
            ("__mc68030__", "1"),
            ("__HAVE_68881__", "1"),
            ("__AZUX__", "1"),
            ("__unix__", "1"),
            ("__azcc__", "1"),
            ("__CHAR_BIT__", "8"),
            ("__SIZEOF_SHORT__", "2"),
            ("__SIZEOF_INT__", "4"),
            ("__SIZEOF_LONG__", "4"),
            ("__SIZEOF_LONG_LONG__", "8"),
            ("__SIZEOF_POINTER__", "4"),
            ("__SIZEOF_FLOAT__", "4"),
            ("__SIZEOF_DOUBLE__", "8"),
            ("__BIG_ENDIAN__", "1"),
            ("__ORDER_BIG_ENDIAN__", "4321"),
            ("__ORDER_LITTLE_ENDIAN__", "1234"),
            ("__BYTE_ORDER__", "4321"),
            ("__INT_MAX__", "2147483647"),
            ("__LONG_MAX__", "2147483647L"),
            ("__SIZE_TYPE__", "unsigned int"),
            ("__PTRDIFF_TYPE__", "int"),
            ("__WCHAR_TYPE__", "int"),
            ("__CHAR_UNSIGNED__", "0"),
        ] {
            p.define_str(n, v);
        }
        for b in ["__FILE__", "__LINE__", "__DATE__", "__TIME__", "__COUNTER__"] {
            p.macros.insert(b.into(), Macro { params: None, variadic: false, body: Vec::new(), builtin: true });
        }
        p
    }

    fn err(&mut self, t: Option<&Token>, msg: &str) {
        let loc = match t {
            Some(t) => format!("{}:{}: ", self.file_names.get(t.file as usize).map_or("?", |s| s.as_str()), self.line_of(t)),
            None => String::new(),
        };
        self.errors.push(format!("{loc}error: {msg}"));
    }

    pub fn line_of(&self, t: &Token) -> i64 {
        t.line as i64 + self.line_delta.get(&t.file).copied().unwrap_or(0)
    }

    /// `-D name[=value]`
    pub fn define_str(&mut self, name: &str, value: &str) {
        let text = format!("#define {name} {value}\n");
        if let Ok(toks) = lex::lex(&text, 0) {
            let mut q: VecDeque<Token> = toks.into();
            q.pop_front();
            self.define(&mut q);
        }
    }

    pub fn undef(&mut self, name: &str) {
        self.macros.remove(name);
    }

    /// Start preprocessing a main file.
    pub fn push_file(&mut self, name: &str, text: &str) -> bool {
        let id = self.file_names.len() as u16;
        self.file_names.push(name.into());
        match lex::lex(text, id) {
            Ok(toks) => {
                let dir = match name.rfind('/') {
                    Some(i) => String::from(&name[..i]),
                    None => String::from("."),
                };
                self.stack.push(Source { toks: toks.into(), dir, conds: self.conds.len() });
                true
            }
            Err((line, m)) => {
                self.errors.push(format!("{name}:{line}: error: {m}"));
                false
            }
        }
    }

    /// The rest of the current directive line.
    fn rest_of_line(&mut self) -> Vec<Token> {
        let mut v = Vec::new();
        let Some(src) = self.stack.last_mut() else { return v };
        while let Some(t) = src.toks.front() {
            if t.bol {
                break;
            }
            v.push(src.toks.pop_front().unwrap());
        }
        v
    }

    fn active(&self) -> bool {
        self.conds.last().is_none_or(|c| c.active)
    }

    fn define(&mut self, line: &mut VecDeque<Token>) {
        let Some(name_tok) = line.pop_front() else {
            self.err(None, "macro name missing");
            return;
        };
        let Some(name) = name_tok.ident().map(String::from) else {
            self.err(Some(&name_tok), "macro names must be identifiers");
            return;
        };
        let mut params = None;
        let mut variadic = false;
        // function-like only if '(' follows with no space
        if line.front().is_some_and(|t| t.is("(") && !t.space) {
            line.pop_front();
            let mut ps = Vec::new();
            loop {
                let Some(t) = line.pop_front() else {
                    self.err(Some(&name_tok), "missing ')' in macro parameter list");
                    return;
                };
                if t.is(")") {
                    break;
                }
                if t.is(",") {
                    continue;
                }
                if t.is("...") {
                    variadic = true;
                    ps.push(String::from("__VA_ARGS__"));
                    continue;
                }
                match t.ident() {
                    Some(p) => {
                        // GNU named variadic: name...
                        if line.front().is_some_and(|x| x.is("...")) {
                            line.pop_front();
                            variadic = true;
                        }
                        ps.push(p.into())
                    }
                    None => {
                        self.err(Some(&t), "invalid macro parameter");
                        return;
                    }
                }
            }
            params = Some(ps);
        }
        let mut body: Vec<Token> = line.drain(..).collect();
        if let Some(f) = body.first_mut() {
            f.space = false;
        }
        self.macros.insert(name, Macro { params, variadic, body, builtin: false });
    }

    fn find_include(&mut self, name: &str, quoted: bool) -> Option<(String, String)> {
        if name.starts_with('/') {
            return self.files.read(name).map(|t| (String::from(name), t));
        }
        let mut dirs: Vec<String> = Vec::new();
        if quoted {
            if let Some(s) = self.stack.last() {
                dirs.push(s.dir.clone());
            }
        }
        dirs.extend(self.include_dirs.iter().cloned());
        dirs.extend(self.system_dirs.iter().cloned());
        for d in dirs {
            let p = if d.is_empty() || d == "." { String::from(name) } else { format!("{d}/{name}") };
            if let Some(t) = self.files.read(&p) {
                return Some((p, t));
            }
        }
        None
    }

    fn include(&mut self, hash: &Token, line: Vec<Token>) {
        let line = if line.first().is_some_and(|t| matches!(t.tok, Tok::Str(_)) || t.is("<")) { line } else { self.expand_list(line) };
        let (name, quoted) = match line.first() {
            Some(Token { tok: Tok::Str(s), .. }) => (String::from(&s[1..s.len() - 1]), true),
            Some(t) if t.is("<") => {
                let mut s = String::new();
                let mut ok = false;
                for x in &line[1..] {
                    if x.is(">") {
                        ok = true;
                        break;
                    }
                    if x.space && !s.is_empty() {
                        s.push(' ');
                    }
                    s.push_str(&x.text());
                }
                if !ok {
                    self.err(Some(hash), "missing '>' in #include");
                    return;
                }
                (s, false)
            }
            _ => {
                self.err(Some(hash), "#include expects \"FILENAME\" or <FILENAME>");
                return;
            }
        };
        if self.stack.len() > 64 {
            self.err(Some(hash), "#include nested too deeply");
            return;
        }
        let Some((path, text)) = self.find_include(&name, quoted) else {
            self.err(Some(hash), &format!("{name}: No such file or directory"));
            return;
        };
        if self.once.contains(&path) {
            return;
        }
        self.push_file(&path, &text);
    }

    fn eval_if(&mut self, hash: &Token, line: Vec<Token>) -> bool {
        // handle `defined` before expansion
        let mut v = Vec::new();
        let mut i = 0;
        while i < line.len() {
            if line[i].ident() == Some("defined") {
                let (name, skip) = if line.get(i + 1).is_some_and(|t| t.is("(")) {
                    (line.get(i + 2).and_then(|t| t.ident()).map(String::from), 4)
                } else {
                    (line.get(i + 1).and_then(|t| t.ident()).map(String::from), 2)
                };
                let Some(name) = name else {
                    self.err(Some(hash), "operator \"defined\" requires an identifier");
                    return false;
                };
                let d = self.macros.contains_key(&name);
                v.push(tok(Tok::Num(if d { "1".into() } else { "0".into() }), &line[i]));
                i += skip;
                continue;
            }
            v.push(line[i].clone());
            i += 1;
        }
        let v = self.expand_list(v);
        let v: Vec<Token> = v
            .into_iter()
            .map(|t| match &t.tok {
                Tok::Ident(n) if n == "true" => tok(Tok::Num("1".into()), &t),
                Tok::Ident(_) => tok(Tok::Num("0".into()), &t),
                _ => t,
            })
            .collect();
        let mut e = IfExpr { t: &v, i: 0 };
        match e.cond() {
            Ok(x) if e.i == v.len() => x != 0,
            Ok(_) => {
                self.err(Some(hash), "garbage at end of #if expression");
                false
            }
            Err(m) => {
                self.err(Some(hash), &m);
                false
            }
        }
    }

    fn directive(&mut self, hash: Token) {
        let mut line: VecDeque<Token> = self.rest_of_line().into();
        let Some(d) = line.pop_front() else { return }; // null directive
        let name = match &d.tok {
            Tok::Ident(n) => n.clone(),
            Tok::Num(_) => String::from("line"),
            _ => String::new(),
        };
        if name == "line" && matches!(d.tok, Tok::Num(_)) {
            line.push_front(d.clone());
        }
        let active = self.active();
        match name.as_str() {
            "if" | "ifdef" | "ifndef" => {
                let val = if !active {
                    false
                } else if name == "if" {
                    self.eval_if(&hash, line.into_iter().collect())
                } else {
                    let n = line.front().and_then(|t| t.ident()).map(String::from);
                    match n {
                        Some(n) => self.macros.contains_key(&n) == (name == "ifdef"),
                        None => {
                            self.err(Some(&hash), &format!("no macro name given in #{name} directive"));
                            false
                        }
                    }
                };
                self.conds.push(Cond { taken: val, active: active && val, outer: active, seen_else: false });
            }
            "elif" => {
                let Some(c) = self.conds.last() else {
                    self.err(Some(&hash), "#elif without #if");
                    return;
                };
                let (outer, taken, seen_else) = (c.outer, c.taken, c.seen_else);
                if seen_else {
                    self.err(Some(&hash), "#elif after #else");
                }
                let val = outer && !taken && self.eval_if(&hash, line.into_iter().collect());
                let c = self.conds.last_mut().unwrap();
                c.active = val;
                c.taken |= val;
            }
            "else" => {
                let Some(c) = self.conds.last_mut() else {
                    self.err(Some(&hash), "#else without #if");
                    return;
                };
                if c.seen_else {
                    let h = hash.clone();
                    self.err(Some(&h), "#else after #else");
                    return;
                }
                c.seen_else = true;
                c.active = c.outer && !c.taken;
                c.taken = true;
            }
            "endif" => {
                let base = self.stack.last().map_or(0, |s| s.conds);
                if self.conds.len() <= base {
                    self.err(Some(&hash), "#endif without #if");
                } else {
                    self.conds.pop();
                }
            }
            _ if !active => {}
            "define" => self.define(&mut line),
            "undef" => {
                if let Some(n) = line.front().and_then(|t| t.ident()) {
                    let n = String::from(n);
                    self.macros.remove(&n);
                }
            }
            "include" | "include_next" => self.include(&hash, line.into_iter().collect()),
            "error" => {
                let msg = spell(&line.into_iter().collect::<Vec<_>>());
                self.err(Some(&hash), &format!("#error {msg}"));
            }
            "warning" => {
                let msg = spell(&line.into_iter().collect::<Vec<_>>());
                let loc = format!("{}:{}", self.file_names[hash.file as usize], self.line_of(&hash));
                self.warnings.push(format!("{loc}: warning: #warning {msg}"));
            }
            "pragma" => {
                if line.front().and_then(|t| t.ident()) == Some("once") {
                    let f = self.file_names[hash.file as usize].clone();
                    self.once.insert(f);
                }
            }
            "line" => {
                let v = self.expand_list(line.into_iter().collect());
                if let Some(Tok::Num(n)) = v.first().map(|t| &t.tok) {
                    if let Ok(n) = n.parse::<i64>() {
                        // the line after the directive gets number n
                        let next = hash.line as i64 + 1;
                        self.line_delta.insert(hash.file, n - next);
                    }
                }
                if let Some(Tok::Str(s)) = v.get(1).map(|t| &t.tok) {
                    let f = hash.file as usize;
                    self.file_names[f] = String::from(&s[1..s.len() - 1]);
                }
            }
            "ident" | "sccs" | "assert" | "unassert" => {}
            _ => self.err(Some(&hash), &format!("invalid preprocessing directive #{name}")),
        }
    }

    /// Next token from the current source files (directives handled), unexpanded.
    fn raw_next(&mut self) -> Option<Token> {
        loop {
            let src = self.stack.last_mut()?;
            let Some(t) = src.toks.pop_front() else {
                let s = self.stack.pop().unwrap();
                if self.conds.len() > s.conds {
                    self.errors.push(String::from("error: unterminated #if at end of file"));
                    self.conds.truncate(s.conds);
                }
                continue;
            };
            if t.bol && t.is("#") {
                self.directive(t);
                continue;
            }
            if !self.active() {
                continue;
            }
            return Some(t);
        }
    }

    /// Next fully macro-expanded token.
    pub fn next(&mut self) -> Option<Token> {
        loop {
            let t = match self.pending.pop_front() {
                Some(t) => t,
                None => self.raw_next()?,
            };
            if let Some(name) = t.ident() {
                if !t.hidden(name) && self.macros.contains_key(name) {
                    let name = String::from(name);
                    if let Some(exp) = self.expand(&t, &name, &mut Mode::Main) {
                        for x in exp.into_iter().rev() {
                            self.pending.push_front(x);
                        }
                        continue;
                    }
                }
            }
            return Some(t);
        }
    }

    /// Peek the next unexpanded token for a function-like macro call.
    fn peek_paren(&mut self, mode: &mut Mode) -> bool {
        match mode {
            Mode::List(q) => q.front().is_some_and(|t| t.is("(")),
            Mode::Main => {
                if let Some(t) = self.pending.front() {
                    return t.is("(");
                }
                // peek in the file, skipping directives is not allowed here
                match self.raw_next() {
                    Some(t) => {
                        let is = t.is("(");
                        self.pending.push_front(t);
                        is
                    }
                    None => false,
                }
            }
        }
    }

    fn take(&mut self, mode: &mut Mode) -> Option<Token> {
        match mode {
            Mode::List(q) => q.pop_front(),
            Mode::Main => match self.pending.pop_front() {
                Some(t) => Some(t),
                None => self.raw_next(),
            },
        }
    }

    /// Expand macro `name` invoked by token `t`. None if it is a function-like macro
    /// not followed by '('.
    fn expand(&mut self, t: &Token, name: &str, mode: &mut Mode) -> Option<Vec<Token>> {
        let m = self.macros.get(name)?.clone();
        if m.builtin {
            let v = match name {
                "__FILE__" => Tok::Str(format!("\"{}\"", self.file_names.get(t.file as usize).map_or("?", |s| s.as_str()))),
                "__LINE__" => Tok::Num(format!("{}", self.line_of(t))),
                "__DATE__" => Tok::Str("\"Oct 10 2026\"".into()),
                "__TIME__" => Tok::Str("\"00:00:00\"".into()),
                _ => {
                    self.depth += 1;
                    Tok::Num(format!("{}", self.depth - 1))
                }
            };
            return Some(vec![Token { tok: v, file: t.file, line: t.line, bol: false, space: t.space, hide: None }]);
        }
        let Some(params) = &m.params else {
            let mut out: Vec<Token> = m.body.iter().map(|b| with_hide(b, name)).collect();
            out = self.paste_all(out);
            for x in out.iter_mut() {
                x.line = t.line;
                x.file = t.file;
            }
            if let Some(f) = out.first_mut() {
                f.space = t.space;
            }
            return Some(out);
        };
        if !self.peek_paren(mode) {
            return None;
        }
        self.take(mode); // (
        // collect arguments
        let mut args: Vec<Vec<Token>> = vec![Vec::new()];
        let mut depth = 0;
        loop {
            let Some(a) = self.take(mode) else {
                self.err(Some(t), &format!("unterminated argument list invoking macro \"{name}\""));
                return Some(Vec::new());
            };
            if a.is("(") {
                depth += 1;
            } else if a.is(")") {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if a.is(",") && depth == 0 && !(m.variadic && args.len() == params.len()) {
                args.push(Vec::new());
                continue;
            }
            let mut a = a;
            a.bol = false;
            args.last_mut().unwrap().push(a);
        }
        if params.is_empty() && args.len() == 1 && args[0].is_empty() {
            args.clear();
        }
        if m.variadic && args.len() == params.len() - 1 {
            args.push(Vec::new());
        }
        if args.len() != params.len() {
            self.err(Some(t), &format!("macro \"{name}\" requires {} arguments, but {} given", params.len(), args.len()));
            return Some(Vec::new());
        }
        // substitute
        let body = &m.body;
        let param_index = |tk: &Token| -> Option<usize> { tk.ident().and_then(|n| params.iter().position(|p| p == n)) };
        let mut out: Vec<Token> = Vec::new();
        let mut i = 0;
        let mut expanded_cache: BTreeMap<usize, Vec<Token>> = BTreeMap::new();
        while i < body.len() {
            let b = &body[i];
            if b.is("#") {
                if let Some(pi) = body.get(i + 1).and_then(param_index) {
                    let mut s = tok(Tok::Str(stringize(&args[pi])), b);
                    s.space = b.space;
                    out.push(s);
                    i += 2;
                    continue;
                }
            }
            let next_is_paste = body.get(i + 1).is_some_and(|x| x.is("##"));
            let prev_is_paste = i > 0 && body[i - 1].is("##");
            if let Some(pi) = param_index(b) {
                let mut arg: Vec<Token> = if next_is_paste || prev_is_paste {
                    args[pi].clone()
                } else {
                    if !expanded_cache.contains_key(&pi) {
                        let e = self.expand_list(args[pi].clone());
                        expanded_cache.insert(pi, e);
                    }
                    expanded_cache[&pi].clone()
                };
                // GNU: , ## __VA_ARGS__ drops the comma when empty
                if prev_is_paste && arg.is_empty() && m.variadic && pi == params.len() - 1 && out.len() >= 2 && out[out.len() - 2].is(",") {
                    out.pop(); // the ##
                    out.pop(); // the comma
                    i += 1;
                    continue;
                }
                if let Some(f) = arg.first_mut() {
                    f.space = b.space;
                }
                if arg.is_empty() && (next_is_paste || prev_is_paste) {
                    // placemarker
                    out.push(Token { tok: Tok::Other('\u{0}'), file: b.file, line: b.line, bol: false, space: b.space, hide: None });
                }
                out.extend(arg);
                i += 1;
                continue;
            }
            out.push(b.clone());
            i += 1;
        }
        let out = self.paste_all(out);
        let mut res = Vec::with_capacity(out.len());
        for (k, x) in out.into_iter().enumerate() {
            if matches!(x.tok, Tok::Other('\u{0}')) {
                continue;
            }
            let mut x = with_hide(&x, name);
            x.line = t.line;
            x.file = t.file;
            if k == 0 {
                x.space = t.space;
            }
            res.push(x);
        }
        Some(res)
    }

    /// Apply the ## operators in a replacement list.
    fn paste_all(&mut self, toks: Vec<Token>) -> Vec<Token> {
        if !toks.iter().any(|t| t.is("##")) {
            return toks;
        }
        let mut out: Vec<Token> = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i].is("##") && !out.is_empty() && i + 1 < toks.len() {
                let l = out.pop().unwrap();
                let r = &toks[i + 1];
                let lt = if matches!(l.tok, Tok::Other('\u{0}')) { String::new() } else { l.text() };
                let rt = if matches!(r.tok, Tok::Other('\u{0}')) { String::new() } else { r.text() };
                let s = format!("{lt}{rt}");
                if s.is_empty() {
                    out.push(Token { tok: Tok::Other('\u{0}'), ..l });
                } else {
                    match lex::lex(&s, l.file) {
                        Ok(v) if v.len() == 1 => {
                            let mut n = v.into_iter().next().unwrap();
                            n.space = l.space;
                            n.bol = false;
                            n.hide = l.hide.clone();
                            out.push(n);
                        }
                        _ => {
                            self.err(Some(&l), &format!("pasting \"{lt}\" and \"{rt}\" does not give a valid preprocessing token"));
                            out.push(l);
                        }
                    }
                }
                i += 2;
                continue;
            }
            out.push(toks[i].clone());
            i += 1;
        }
        out
    }

    /// Fully expand a token list in isolation (macro arguments, #if, #include).
    fn expand_list(&mut self, toks: Vec<Token>) -> Vec<Token> {
        let mut q: VecDeque<Token> = toks.into();
        let mut out = Vec::new();
        let mut guard = 0;
        while let Some(t) = q.pop_front() {
            if let Some(name) = t.ident() {
                if !t.hidden(name) && self.macros.contains_key(name) {
                    guard += 1;
                    if guard > 100_000 {
                        self.err(Some(&t), "macro expansion too deep");
                        break;
                    }
                    let name = String::from(name);
                    let mut mode = Mode::List(core::mem::take(&mut q));
                    let r = self.expand(&t, &name, &mut mode);
                    if let Mode::List(rest) = mode {
                        q = rest;
                    }
                    match r {
                        Some(v) => {
                            for x in v.into_iter().rev() {
                                q.push_front(x);
                            }
                            continue;
                        }
                        None => {}
                    }
                }
            }
            out.push(t);
        }
        out
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Where macro arguments come from: the source files or a token list.
enum Mode {
    Main,
    List(VecDeque<Token>),
}

/// Integer constant expressions for #if.
struct IfExpr<'a> {
    t: &'a [Token],
    i: usize,
}

impl IfExpr<'_> {
    fn peek(&self) -> Option<&Token> {
        self.t.get(self.i)
    }
    fn is(&self, p: &str) -> bool {
        self.peek().is_some_and(|t| t.is(p))
    }
    fn cond(&mut self) -> Result<i64, String> {
        let c = self.bin(1)?;
        if self.is("?") {
            self.i += 1;
            let a = self.cond()?;
            if !self.is(":") {
                return Err("expected ':' in #if".into());
            }
            self.i += 1;
            let b = self.cond()?;
            return Ok(if c != 0 { a } else { b });
        }
        Ok(c)
    }
    fn prec(p: &str) -> Option<u8> {
        Some(match p {
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
    fn bin(&mut self, min: u8) -> Result<i64, String> {
        let mut l = self.unary()?;
        loop {
            let Some(Tok::Punct(p)) = self.peek().map(|t| &t.tok) else { break };
            let p = *p;
            let Some(pr) = Self::prec(p) else { break };
            if pr < min {
                break;
            }
            self.i += 1;
            let r = self.bin(pr + 1)?;
            l = match p {
                "||" => (l != 0 || r != 0) as i64,
                "&&" => (l != 0 && r != 0) as i64,
                "|" => l | r,
                "^" => l ^ r,
                "&" => l & r,
                "==" => (l == r) as i64,
                "!=" => (l != r) as i64,
                "<" => (l < r) as i64,
                ">" => (l > r) as i64,
                "<=" => (l <= r) as i64,
                ">=" => (l >= r) as i64,
                "<<" => l.wrapping_shl(r as u32),
                ">>" => l.wrapping_shr(r as u32),
                "+" => l.wrapping_add(r),
                "-" => l.wrapping_sub(r),
                "*" => l.wrapping_mul(r),
                "/" | "%" if r == 0 => return Err("division by zero in #if".into()),
                "/" => l.wrapping_div(r),
                _ => l.wrapping_rem(r),
            };
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<i64, String> {
        let Some(t) = self.peek().cloned() else { return Err("#if with no expression".into()) };
        self.i += 1;
        match &t.tok {
            Tok::Punct("(") => {
                let v = self.cond()?;
                if !self.is(")") {
                    return Err("missing ')' in #if".into());
                }
                self.i += 1;
                Ok(v)
            }
            Tok::Punct("-") => Ok(self.unary()?.wrapping_neg()),
            Tok::Punct("+") => self.unary(),
            Tok::Punct("!") => Ok((self.unary()? == 0) as i64),
            Tok::Punct("~") => Ok(!self.unary()?),
            Tok::Num(n) => crate::parse::int_literal(n).map(|(v, _)| v as i64).ok_or_else(|| format!("invalid integer \"{n}\" in #if")),
            Tok::Char(c) => crate::parse::char_literal(c).map(|v| v as i64).ok_or_else(|| "invalid character constant".into()),
            _ => Err(format!("token \"{}\" is not valid in preprocessor expressions", t.text())),
        }
    }
}

