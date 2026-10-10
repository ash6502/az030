//! Lexer and parser: shell source text to a syntax tree.
//!
//! Words are kept as raw source text (quotes and all) and expanded when executed.

use alloc::rc::Rc;
use core::cell::RefCell;
use rt::prelude::*;

#[derive(Debug)]
pub enum Node {
    Simple(Simple),
    /// commands, negated with `!`
    Pipe(Vec<Node>, bool),
    /// first, then (is_and, next)...
    AndOr(Box<Node>, Vec<(bool, Node)>),
    List(Vec<Item>),
    Subshell(Box<Node>, Vec<Redir>),
    Group(Box<Node>, Vec<Redir>),
    /// (condition, body) branches, else
    If(Vec<(Node, Node)>, Option<Box<Node>>, Vec<Redir>),
    /// condition, body, until
    While(Box<Node>, Box<Node>, bool, Vec<Redir>),
    For(String, Option<Vec<String>>, Box<Node>, Vec<Redir>),
    Case(String, Vec<(Vec<String>, Node)>, Vec<Redir>),
    Func(String, Rc<Node>),
}

#[derive(Debug)]
pub struct Item {
    pub node: Node,
    pub background: bool,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Simple {
    pub assigns: Vec<(String, String)>,
    pub words: Vec<String>,
    pub redirs: Vec<Redir>,
}

#[derive(Debug, Clone)]
pub enum RedirKind {
    In,
    Out,
    Clobber,
    Append,
    ReadWrite,
    /// `>&N` / `<&N` / `>&-`
    Dup,
    HereDoc(Rc<RefCell<String>>, bool),
}

#[derive(Debug, Clone)]
pub struct Redir {
    pub fd: u32,
    pub kind: RedirKind,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Word(String),
    Op(&'static str),
    IoNum(u32),
    Newline,
    Eof,
}

pub enum Error {
    /// The input ended in the middle of a command (read another line).
    Incomplete,
    Syntax(String),
}

const OPS: [&str; 15] = [";;", "&&", "||", ">>", "<<-", "<<", ">&", "<&", "<>", ">|", "|", "&", ";", "<", ">"];

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    tok_start: usize,
}

impl Lexer {
    pub fn new(s: &str) -> Lexer {
        Lexer { src: s.chars().collect(), pos: 0, tok_start: 0 }
    }

    fn peekc(&self, off: usize) -> Option<char> {
        self.src.get(self.pos + off).copied()
    }

    fn skip_blanks(&mut self) {
        loop {
            match self.peekc(0) {
                Some(' ' | '\t') => self.pos += 1,
                Some('\\') if self.peekc(1) == Some('\n') => self.pos += 2,
                Some('#') => {
                    while self.peekc(0).is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// Skip a `$(...)`, `${...}` or backquoted body; `close` is the closing char.
    fn skip_nested(&mut self, open: char, close: char) -> Result<(), Error> {
        let mut depth = 1;
        while depth > 0 {
            let Some(c) = self.peekc(0) else { return Err(Error::Incomplete) };
            self.pos += 1;
            match c {
                '\\' => self.pos += 1,
                '\'' if close != '`' => self.skip_squote()?,
                '"' if close != '`' => self.skip_dquote()?,
                c if c == close => depth -= 1,
                c if c == open && open != close => depth += 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn skip_squote(&mut self) -> Result<(), Error> {
        loop {
            let Some(c) = self.peekc(0) else { return Err(Error::Incomplete) };
            self.pos += 1;
            if c == '\'' {
                return Ok(());
            }
        }
    }

    fn skip_dquote(&mut self) -> Result<(), Error> {
        loop {
            let Some(c) = self.peekc(0) else { return Err(Error::Incomplete) };
            self.pos += 1;
            match c {
                '"' => return Ok(()),
                '\\' => self.pos += 1,
                '`' => self.skip_nested('`', '`')?,
                '$' if self.peekc(0) == Some('(') => {
                    self.pos += 1;
                    self.skip_nested('(', ')')?;
                }
                '$' if self.peekc(0) == Some('{') => {
                    self.pos += 1;
                    self.skip_nested('{', '}')?;
                }
                _ => {}
            }
        }
    }

    pub fn next(&mut self) -> Result<Tok, Error> {
        self.skip_blanks();
        self.tok_start = self.pos;
        let Some(c) = self.peekc(0) else { return Ok(Tok::Eof) };
        if c == '\n' {
            self.pos += 1;
            return Ok(Tok::Newline);
        }
        if c == '(' || c == ')' {
            self.pos += 1;
            return Ok(Tok::Op(if c == '(' { "(" } else { ")" }));
        }
        for op in OPS {
            if op.chars().enumerate().all(|(i, oc)| self.peekc(i) == Some(oc)) {
                self.pos += op.chars().count();
                return Ok(Tok::Op(op));
            }
        }
        let start = self.pos;
        while let Some(c) = self.peekc(0) {
            match c {
                ' ' | '\t' | '\n' | '|' | '&' | ';' | '<' | '>' | '(' | ')' => break,
                '\\' => {
                    self.pos += 2;
                }
                '\'' => {
                    self.pos += 1;
                    self.skip_squote()?;
                }
                '"' => {
                    self.pos += 1;
                    self.skip_dquote()?;
                }
                '`' => {
                    self.pos += 1;
                    self.skip_nested('`', '`')?;
                }
                '$' if self.peekc(1) == Some('(') => {
                    self.pos += 2;
                    self.skip_nested('(', ')')?;
                }
                '$' if self.peekc(1) == Some('{') => {
                    self.pos += 2;
                    self.skip_nested('{', '}')?;
                }
                _ => self.pos += 1,
            }
        }
        self.pos = self.pos.min(self.src.len());
        let w: String = self.src[start..self.pos].iter().collect();
        if w.bytes().all(|b| b.is_ascii_digit()) && matches!(self.peekc(0), Some('<' | '>')) {
            return Ok(Tok::IoNum(w.parse().unwrap_or(0)));
        }
        Ok(Tok::Word(w))
    }

    /// Read a here-document body (the lines after the current one) up to `delim`.
    fn heredoc(&mut self, delim: &str, strip: bool, eof_ok: bool) -> Result<String, Error> {
        let mut body = String::new();
        loop {
            if self.pos >= self.src.len() {
                return if eof_ok { Ok(body) } else { Err(Error::Incomplete) };
            }
            let start = self.pos;
            while self.pos < self.src.len() && self.src[self.pos] != '\n' {
                self.pos += 1;
            }
            let had_nl = self.pos < self.src.len();
            let mut line: String = self.src[start..self.pos].iter().collect();
            if had_nl {
                self.pos += 1;
            }
            if strip {
                line = line.trim_start_matches('\t').into();
            }
            if line == delim {
                return Ok(body);
            }
            if !had_nl && !eof_ok {
                return Err(Error::Incomplete);
            }
            body.push_str(&line);
            body.push('\n');
        }
    }

    /// Insert alias text in place of the word that ended at the current position.
    fn replace_word(&mut self, len: usize, text: &str) {
        let start = self.pos - len;
        self.src.splice(start..self.pos, text.chars());
        self.pos = start;
    }
}

const RESERVED: [&str; 16] = ["if", "then", "elif", "else", "fi", "while", "until", "do", "done", "for", "in", "case", "esac", "{", "}", "!"];

pub fn is_reserved(w: &str) -> bool {
    RESERVED.contains(&w)
}

pub fn is_name(s: &str) -> bool {
    let mut it = s.chars();
    matches!(it.next(), Some(c) if c == '_' || c.is_ascii_alphabetic()) && it.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

pub struct Parser<'a> {
    lx: Lexer,
    /// lookahead tokens with their start offsets
    peeked: Vec<(Tok, usize)>,
    last_start: usize,
    heredocs: Vec<(Rc<RefCell<String>>, String, bool)>,
    /// at end of input, accept unterminated constructs as errors rather than
    /// asking for more input
    eof_final: bool,
    aliases: Option<&'a BTreeMap<String, String>>,
    alias_depth: u32,
}

type PResult<T> = Result<T, Error>;

impl<'a> Parser<'a> {
    pub fn new(src: &str, eof_final: bool, aliases: Option<&'a BTreeMap<String, String>>) -> Parser<'a> {
        Parser { lx: Lexer::new(src), peeked: Vec::new(), last_start: 0, heredocs: Vec::new(), eof_final, aliases, alias_depth: 0 }
    }

    fn fetch(&mut self) -> PResult<Tok> {
        if let Some((t, s)) = self.peeked.pop() {
            self.last_start = s;
            return Ok(t);
        }
        let t = self.lx.next()?;
        self.last_start = self.lx.tok_start;
        if t == Tok::Newline && !self.heredocs.is_empty() {
            for (cell, delim, strip) in core::mem::take(&mut self.heredocs) {
                let body = self.lx.heredoc(&delim, strip, self.eof_final)?;
                *cell.borrow_mut() = body;
            }
        }
        Ok(t)
    }

    fn peek(&mut self) -> PResult<&Tok> {
        if self.peeked.is_empty() {
            let t = self.fetch()?;
            self.peeked.push((t, self.last_start));
        }
        Ok(&self.peeked.last().unwrap().0)
    }

    fn unget(&mut self, t: Tok) {
        self.peeked.push((t, self.last_start));
    }

    /// Source offset where the next token starts / the last one ended.
    fn next_start(&mut self) -> PResult<usize> {
        self.peek()?;
        Ok(self.peeked.last().unwrap().1)
    }

    fn cur_end(&self) -> usize {
        self.peeked.last().map(|(_, s)| *s).unwrap_or(self.lx.pos)
    }

    fn eof_error<T>(&self, what: &str) -> PResult<T> {
        if self.eof_final {
            Err(Error::Syntax(format!("unexpected end of file (expecting {what})")))
        } else {
            Err(Error::Incomplete)
        }
    }

    fn unexpected<T>(&self, t: &Tok) -> PResult<T> {
        match t {
            Tok::Eof => self.eof_error("a command"),
            Tok::Newline => Err(Error::Syntax("syntax error near unexpected newline".into())),
            Tok::Op(o) => Err(Error::Syntax(format!("syntax error near unexpected token '{o}'"))),
            Tok::Word(w) => Err(Error::Syntax(format!("syntax error near unexpected token '{w}'"))),
            Tok::IoNum(n) => Err(Error::Syntax(format!("syntax error near unexpected token '{n}'"))),
        }
    }

    fn skip_newlines(&mut self) -> PResult<()> {
        while *self.peek()? == Tok::Newline {
            self.fetch()?;
        }
        Ok(())
    }

    fn peek_reserved(&mut self) -> PResult<Option<String>> {
        Ok(match self.peek()? {
            Tok::Word(w) if is_reserved(w) => Some(w.clone()),
            _ => None,
        })
    }

    fn expect_word(&mut self, w: &str) -> PResult<()> {
        self.skip_newlines()?;
        match self.fetch()? {
            Tok::Word(x) if x == w => Ok(()),
            Tok::Eof => self.eof_error(&format!("'{w}'")),
            t => self.unexpected(&t),
        }
    }

    /// The next complete command (a list ending in a newline), or None at the end.
    pub fn next_command(&mut self) -> PResult<Option<Node>> {
        self.skip_newlines()?;
        if *self.peek()? == Tok::Eof {
            return Ok(None);
        }
        let n = self.list(true)?;
        match self.fetch()? {
            Tok::Newline | Tok::Eof => {}
            t => return self.unexpected(&t),
        }
        if !self.heredocs.is_empty() {
            // here-document at the very end of the input
            for (cell, delim, strip) in core::mem::take(&mut self.heredocs) {
                *cell.borrow_mut() = self.lx.heredoc(&delim, strip, self.eof_final)?;
            }
        }
        Ok(Some(n))
    }

    /// Is the next token one that ends a compound list?
    fn at_list_end(&mut self) -> PResult<bool> {
        Ok(match self.peek()? {
            Tok::Eof => true,
            Tok::Op(")") | Tok::Op(";;") => true,
            Tok::Word(w) => matches!(w.as_str(), "then" | "elif" | "else" | "fi" | "do" | "done" | "esac" | "}"),
            _ => false,
        })
    }

    /// A list of and-or items separated by `;`, `&` (and newlines unless `top`).
    fn list(&mut self, top: bool) -> PResult<Node> {
        let mut items = Vec::new();
        loop {
            if !top {
                self.skip_newlines()?;
            }
            if self.at_list_end()? {
                break;
            }
            let start = self.next_start()?;
            let node = self.and_or()?;
            let end = self.cur_end();
            let mut item = Item { node, background: false, text: self.text(start, end) };
            match self.peek()?.clone() {
                Tok::Op(";") => {
                    self.fetch()?;
                }
                Tok::Op("&") => {
                    self.fetch()?;
                    item.background = true;
                }
                Tok::Newline if !top => {}
                _ => {
                    items.push(item);
                    break;
                }
            }
            items.push(item);
            if top && *self.peek()? == Tok::Newline {
                break;
            }
        }
        if items.is_empty() {
            let t = self.peek()?.clone();
            return self.unexpected(&t);
        }
        if items.len() == 1 && !items[0].background {
            return Ok(items.pop().unwrap().node);
        }
        Ok(Node::List(items))
    }

    fn text(&self, start: usize, end: usize) -> String {
        let s = start.min(self.lx.src.len());
        let e = end.min(self.lx.src.len()).max(s);
        self.lx.src[s..e].iter().collect::<String>().trim().into()
    }

    fn and_or(&mut self) -> PResult<Node> {
        let first = self.pipeline()?;
        let mut rest = Vec::new();
        loop {
            let is_and = match self.peek()? {
                Tok::Op("&&") => true,
                Tok::Op("||") => false,
                _ => break,
            };
            self.fetch()?;
            self.skip_newlines()?;
            rest.push((is_and, self.pipeline()?));
        }
        if rest.is_empty() {
            return Ok(first);
        }
        Ok(Node::AndOr(Box::new(first), rest))
    }

    fn pipeline(&mut self) -> PResult<Node> {
        let mut bang = false;
        if self.peek_reserved()?.as_deref() == Some("!") {
            self.fetch()?;
            bang = true;
        }
        let mut cmds = alloc::vec![self.command()?];
        while *self.peek()? == Tok::Op("|") {
            self.fetch()?;
            self.skip_newlines()?;
            cmds.push(self.command()?);
        }
        if cmds.len() == 1 && !bang {
            return Ok(cmds.pop().unwrap());
        }
        Ok(Node::Pipe(cmds, bang))
    }

    fn redirect(&mut self, fd: Option<u32>, op: &str) -> PResult<Redir> {
        let target = match self.fetch()? {
            Tok::Word(w) => w,
            Tok::Eof => return self.eof_error("a file name"),
            t => return self.unexpected(&t),
        };
        let default_fd = if op.starts_with('<') { 0 } else { 1 };
        let fd = fd.unwrap_or(default_fd);
        let kind = match op {
            "<" => RedirKind::In,
            ">" => RedirKind::Out,
            ">|" => RedirKind::Clobber,
            ">>" => RedirKind::Append,
            "<>" => RedirKind::ReadWrite,
            ">&" | "<&" => RedirKind::Dup,
            "<<" | "<<-" => {
                let quoted = target.contains(['\'', '"', '\\']);
                let delim = crate::expand::unquote(&target);
                let cell = Rc::new(RefCell::new(String::new()));
                self.heredocs.push((cell.clone(), delim, op == "<<-"));
                RedirKind::HereDoc(cell, !quoted)
            }
            _ => unreachable!(),
        };
        Ok(Redir { fd, kind, target })
    }

    /// Redirections after a compound command.
    fn redirs(&mut self) -> PResult<Vec<Redir>> {
        let mut v = Vec::new();
        loop {
            match self.peek()?.clone() {
                Tok::IoNum(n) => {
                    self.fetch()?;
                    let Tok::Op(op) = self.fetch()? else { return Err(Error::Syntax("bad redirection".into())) };
                    v.push(self.redirect(Some(n), op)?);
                }
                Tok::Op(op) if op.starts_with(['<', '>']) => {
                    self.fetch()?;
                    v.push(self.redirect(None, op)?);
                }
                _ => return Ok(v),
            }
        }
    }

    fn command(&mut self) -> PResult<Node> {
        if *self.peek()? == Tok::Op("(") {
            self.fetch()?;
            let body = self.list(false)?;
            match self.fetch()? {
                Tok::Op(")") => {}
                Tok::Eof => return self.eof_error("')'"),
                t => return self.unexpected(&t),
            }
            let r = self.redirs()?;
            return Ok(Node::Subshell(Box::new(body), r));
        }
        if let Some(w) = self.peek_reserved()? {
            match w.as_str() {
                "{" => {
                    self.fetch()?;
                    let body = self.list(false)?;
                    self.expect_word("}")?;
                    let r = self.redirs()?;
                    return Ok(Node::Group(Box::new(body), r));
                }
                "if" => return self.if_clause(),
                "while" | "until" => {
                    self.fetch()?;
                    let cond = self.list(false)?;
                    self.expect_word("do")?;
                    let body = self.list(false)?;
                    self.expect_word("done")?;
                    let r = self.redirs()?;
                    return Ok(Node::While(Box::new(cond), Box::new(body), w == "until", r));
                }
                "for" => return self.for_clause(),
                "case" => return self.case_clause(),
                _ => {
                    let t = self.fetch()?;
                    return self.unexpected(&t);
                }
            }
        }
        self.simple()
    }

    fn if_clause(&mut self) -> PResult<Node> {
        self.fetch()?; // if
        let mut branches = Vec::new();
        let mut else_part = None;
        loop {
            let cond = self.list(false)?;
            self.expect_word("then")?;
            let body = self.list(false)?;
            branches.push((cond, body));
            self.skip_newlines()?;
            match self.fetch()? {
                Tok::Word(w) if w == "elif" => continue,
                Tok::Word(w) if w == "else" => {
                    else_part = Some(Box::new(self.list(false)?));
                    self.expect_word("fi")?;
                    break;
                }
                Tok::Word(w) if w == "fi" => break,
                Tok::Eof => return self.eof_error("'fi'"),
                t => return self.unexpected(&t),
            }
        }
        let r = self.redirs()?;
        Ok(Node::If(branches, else_part, r))
    }

    fn for_clause(&mut self) -> PResult<Node> {
        self.fetch()?; // for
        let name = match self.fetch()? {
            Tok::Word(w) if is_name(&w) => w,
            Tok::Eof => return self.eof_error("a variable name"),
            t => return self.unexpected(&t),
        };
        while *self.peek()? == Tok::Newline {
            self.fetch()?;
        }
        let mut words = None;
        if self.peek_reserved()?.as_deref() == Some("in") {
            self.fetch()?;
            let mut v = Vec::new();
            loop {
                match self.fetch()? {
                    Tok::Word(w) => v.push(w),
                    Tok::Op(";") | Tok::Newline => break,
                    Tok::Eof => return self.eof_error("'do'"),
                    t => return self.unexpected(&t),
                }
            }
            words = Some(v);
        } else if *self.peek()? == Tok::Op(";") {
            self.fetch()?;
        }
        self.expect_word("do")?;
        let body = self.list(false)?;
        self.expect_word("done")?;
        let r = self.redirs()?;
        Ok(Node::For(name, words, Box::new(body), r))
    }

    fn case_clause(&mut self) -> PResult<Node> {
        self.fetch()?; // case
        let word = match self.fetch()? {
            Tok::Word(w) => w,
            Tok::Eof => return self.eof_error("a word"),
            t => return self.unexpected(&t),
        };
        self.expect_word("in")?;
        let mut arms = Vec::new();
        loop {
            self.skip_newlines()?;
            match self.fetch()? {
                Tok::Word(w) if w == "esac" => break,
                Tok::Eof => return self.eof_error("'esac'"),
                t => {
                    let mut pats = Vec::new();
                    let mut t = t;
                    if t == Tok::Op("(") {
                        t = self.fetch()?;
                    }
                    loop {
                        match t {
                            Tok::Word(w) => pats.push(w),
                            Tok::Eof => return self.eof_error("')'"),
                            t => return self.unexpected(&t),
                        }
                        match self.fetch()? {
                            Tok::Op("|") => t = self.fetch()?,
                            Tok::Op(")") => break,
                            Tok::Eof => return self.eof_error("')'"),
                            t => return self.unexpected(&t),
                        }
                    }
                    self.skip_newlines()?;
                    let body = if matches!(self.peek()?, Tok::Op(";;")) || self.peek_reserved()?.as_deref() == Some("esac") {
                        Node::List(Vec::new())
                    } else {
                        self.list(false)?
                    };
                    arms.push((pats, body));
                    self.skip_newlines()?;
                    match self.fetch()? {
                        Tok::Op(";;") => {}
                        Tok::Word(w) if w == "esac" => break,
                        Tok::Eof => return self.eof_error("'esac'"),
                        t => return self.unexpected(&t),
                    }
                }
            }
        }
        let r = self.redirs()?;
        Ok(Node::Case(word, arms, r))
    }

    fn simple(&mut self) -> PResult<Node> {
        let mut s = Simple::default();
        loop {
            match self.fetch()? {
                Tok::Word(w) => {
                    if s.words.is_empty() {
                        if let Some(eq) = w.find('=') {
                            if is_name(&w[..eq]) {
                                s.assigns.push((w[..eq].into(), w[eq + 1..].into()));
                                continue;
                            }
                        }
                        // alias substitution in command position
                        if let Some(al) = self.aliases {
                            if self.alias_depth < 16 && self.peeked.is_empty() {
                                if let Some(text) = al.get(&w) {
                                    self.alias_depth += 1;
                                    let n = w.chars().count();
                                    // avoid re-expanding `ls` in `alias ls='ls -F'`
                                    let text = if text.split_whitespace().next() == Some(w.as_str()) {
                                        format!("\\{text}")
                                    } else {
                                        text.clone()
                                    };
                                    self.lx.replace_word(n, &text);
                                    continue;
                                }
                            }
                        }
                        // function definition: name ( ) body
                        if s.assigns.is_empty() && s.redirs.is_empty() && *self.peek()? == Tok::Op("(") {
                            self.fetch()?;
                            match self.fetch()? {
                                Tok::Op(")") => {}
                                t => return self.unexpected(&t),
                            }
                            self.skip_newlines()?;
                            let body = self.command()?;
                            return Ok(Node::Func(w, Rc::new(body)));
                        }
                    }
                    s.words.push(w);
                }
                Tok::IoNum(n) => {
                    let Tok::Op(op) = self.fetch()? else { return Err(Error::Syntax("bad redirection".into())) };
                    s.redirs.push(self.redirect(Some(n), op)?);
                }
                Tok::Op(op) if op.starts_with(['<', '>']) => {
                    s.redirs.push(self.redirect(None, op)?);
                }
                t => {
                    self.unget(t);
                    break;
                }
            }
        }
        self.alias_depth = 0;
        if s.words.is_empty() && s.assigns.is_empty() && s.redirs.is_empty() {
            let t = self.fetch()?;
            return self.unexpected(&t);
        }
        Ok(Node::Simple(s))
    }
}
