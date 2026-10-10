//! sed: stream editor.
//!
//!     sed [-nEr] [-i[suffix]] [-s] script [file...]
//!     sed [-nEr] [-i[suffix]] -e script... -f file... [file...]
//!
//! Commands: s y d D p P n N g G h H x a i c = l q Q r w b t T : { } and `!`.
//! Addresses: N, $, /re/, \cREc, first~step, addr1,addr2, addr1,+N.
//! `s` flags: g p N i I w file; `&` and `\1`-`\9` in the replacement, `\n` `\t`.

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::regex::Regex;

rt::main!(main);

#[derive(Clone)]
enum Addr {
    Line(usize),
    Last,
    Re(Option<Rc<Regex>>),
    Step(usize, usize),
    /// `,+N` as the second address
    Plus(usize),
}

#[derive(Clone)]
enum Cmd {
    Block(usize),
    EndBlock,
    Subst { re: Option<Rc<Regex>>, rep: Vec<Rep>, global: bool, nth: usize, print: bool, wfile: Option<String> },
    Trans(Vec<char>, Vec<char>),
    Simple(char),
    Text(char, String),
    Quit(char, i32),
    ReadFile(String),
    WriteFile(String),
    Branch(char, String),
}

#[derive(Clone)]
enum Rep {
    Lit(String),
    Group(usize),
}

struct Inst {
    a1: Option<Addr>,
    a2: Option<Addr>,
    neg: bool,
    cmd: Cmd,
    /// range state
    active: bool,
    end_line: usize,
}

struct Parser {
    s: Vec<char>,
    i: usize,
    ere: bool,
}

fn fail(m: &str) -> ! {
    rt::eprintln!("sed: {m}");
    rt::process::exit(1)
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.i += 1;
        }
    }
    fn skip_ws_nl(&mut self) {
        while self.peek().is_some_and(|c| c.is_whitespace() || c == ';') {
            self.i += 1;
        }
    }
    fn number(&mut self) -> Option<usize> {
        let st = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if st == self.i { None } else { self.s[st..self.i].iter().collect::<String>().parse().ok() }
    }

    /// Text up to an unescaped delimiter; `\delim` becomes delim, `\n` a newline (if `nl`).
    fn delimited(&mut self, d: char, keep_escapes: bool) -> String {
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else { fail("unterminated address regex or s command") };
            self.i += 1;
            if c == d {
                return out;
            }
            if c == '\\' {
                let Some(n) = self.peek() else { fail("trailing backslash") };
                self.i += 1;
                if n == d {
                    out.push(d);
                } else if n == 'n' && !keep_escapes {
                    out.push('\n');
                } else {
                    out.push('\\');
                    out.push(n);
                }
                continue;
            }
            if c == '[' && !keep_escapes {
                // bracket expressions may contain the delimiter
                out.push('[');
                let mut first = true;
                while let Some(x) = self.peek() {
                    self.i += 1;
                    out.push(x);
                    if x == ']' && !first {
                        break;
                    }
                    if !(first && x == '^') {
                        first = false;
                    }
                }
                continue;
            }
            out.push(c);
        }
    }

    fn regex(&mut self, d: char) -> Option<Rc<Regex>> {
        let pat = self.delimited(d, false);
        let mut icase = false;
        if self.peek() == Some('I') {
            self.i += 1;
            icase = true;
        }
        self.compile(&pat, icase)
    }

    fn compile(&self, pat: &str, icase: bool) -> Option<Rc<Regex>> {
        if pat.is_empty() {
            return None; // the last regex used
        }
        match Regex::new(pat, self.ere, icase) {
            Ok(r) => Some(Rc::new(r)),
            Err(e) => fail(&format!("{pat}: {e}")),
        }
    }

    fn addr(&mut self) -> Option<Addr> {
        match self.peek()? {
            '$' => {
                self.i += 1;
                Some(Addr::Last)
            }
            '/' => {
                self.i += 1;
                Some(Addr::Re(self.regex('/')))
            }
            '\\' => {
                self.i += 1;
                let d = self.peek().unwrap_or_else(|| fail("unexpected end"));
                self.i += 1;
                Some(Addr::Re(self.regex(d)))
            }
            c if c.is_ascii_digit() => {
                let n = self.number().unwrap();
                if self.peek() == Some('~') {
                    self.i += 1;
                    let step = self.number().unwrap_or(0);
                    return Some(Addr::Step(n, step));
                }
                Some(Addr::Line(n))
            }
            _ => None,
        }
    }

    /// The rest of the line (for a, i, c and file names).
    fn text_arg(&mut self) -> String {
        self.skip_ws();
        if self.peek() == Some('\\') {
            self.i += 1;
            if self.peek() == Some('\n') {
                self.i += 1;
            }
        }
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == '\\' {
                match self.peek() {
                    Some('\n') => {
                        out.push('\n');
                        self.i += 1;
                    }
                    Some(n) => {
                        out.push(n);
                        self.i += 1;
                    }
                    None => {}
                }
                continue;
            }
            if c == '\n' {
                break;
            }
            out.push(c);
        }
        out
    }

    fn word_arg(&mut self) -> String {
        self.skip_ws();
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '\n' || c == ';' || c == '}' {
                break;
            }
            out.push(c);
            self.i += 1;
        }
        out.trim_end().into()
    }

    fn parse(&mut self) -> Vec<Inst> {
        let mut prog: Vec<Inst> = Vec::new();
        let mut blocks = Vec::new();
        loop {
            self.skip_ws_nl();
            let Some(c) = self.peek() else { break };
            if c == '#' {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                continue;
            }
            let a1 = self.addr();
            let mut a2 = None;
            if a1.is_some() && self.peek() == Some(',') {
                self.i += 1;
                if self.peek() == Some('+') {
                    self.i += 1;
                    a2 = Some(Addr::Plus(self.number().unwrap_or(0)));
                } else {
                    a2 = self.addr();
                    if a2.is_none() {
                        fail("unexpected ','");
                    }
                }
            }
            self.skip_ws();
            let mut neg = false;
            while self.peek() == Some('!') {
                neg = true;
                self.i += 1;
                self.skip_ws();
            }
            let Some(c) = self.peek() else { fail("missing command") };
            self.i += 1;
            let cmd = match c {
                '{' => {
                    blocks.push(prog.len());
                    Cmd::Block(0)
                }
                '}' => {
                    let Some(start) = blocks.pop() else { fail("unexpected '}'") };
                    let end = prog.len();
                    if let Cmd::Block(ref mut e) = prog[start].cmd {
                        *e = end;
                    }
                    Cmd::EndBlock
                }
                's' => {
                    let d = self.peek().unwrap_or_else(|| fail("unterminated s command"));
                    self.i += 1;
                    let pat = self.delimited(d, false);
                    let rep_text = self.delimited(d, true);
                    let mut rep = Vec::new();
                    let mut lit = String::new();
                    let mut it = rep_text.chars();
                    while let Some(c) = it.next() {
                        match c {
                            '&' => {
                                rep.push(Rep::Lit(core::mem::take(&mut lit)));
                                rep.push(Rep::Group(0));
                            }
                            '\\' => match it.next() {
                                Some(d @ '0'..='9') => {
                                    rep.push(Rep::Lit(core::mem::take(&mut lit)));
                                    rep.push(Rep::Group(d as usize - '0' as usize));
                                }
                                Some('n') => lit.push('\n'),
                                Some('t') => lit.push('\t'),
                                Some('\n') => lit.push('\n'),
                                Some(x) => lit.push(x),
                                None => lit.push('\\'),
                            },
                            c => lit.push(c),
                        }
                    }
                    rep.push(Rep::Lit(lit));
                    let (mut global, mut nth, mut print, mut wfile) = (false, 1, false, None);
                    let mut icase = false;
                    loop {
                        match self.peek() {
                            Some('g') => global = true,
                            Some('p') => print = true,
                            Some('i') | Some('I') => icase = true,
                            Some(c) if c.is_ascii_digit() => {
                                nth = self.number().unwrap();
                                continue;
                            }
                            Some('w') => {
                                self.i += 1;
                                wfile = Some(self.word_arg());
                                break;
                            }
                            _ => break,
                        }
                        self.i += 1;
                    }
                    let re = self.compile(&pat, icase);
                    Cmd::Subst { re, rep, global, nth, print, wfile }
                }
                'y' => {
                    let d = self.peek().unwrap_or_else(|| fail("unterminated y command"));
                    self.i += 1;
                    let a: Vec<char> = self.delimited(d, false).chars().collect();
                    let b: Vec<char> = self.delimited(d, false).chars().collect();
                    if a.len() != b.len() {
                        fail("strings for y command are different lengths");
                    }
                    Cmd::Trans(a, b)
                }
                'a' | 'i' | 'c' => Cmd::Text(c, self.text_arg()),
                'q' | 'Q' => {
                    self.skip_ws();
                    Cmd::Quit(c, self.number().unwrap_or(0) as i32)
                }
                'r' | 'R' => Cmd::ReadFile(self.word_arg()),
                'w' | 'W' => Cmd::WriteFile(self.word_arg()),
                'b' | 't' | 'T' => Cmd::Branch(c, self.word_arg()),
                ':' => {
                    let l = self.word_arg();
                    if l.is_empty() {
                        fail("\":\" lacks a label");
                    }
                    prog.push(Inst { a1: None, a2: None, neg: false, cmd: Cmd::Text(':', l), active: false, end_line: 0 });
                    continue;
                }
                'd' | 'D' | 'p' | 'P' | 'n' | 'N' | 'g' | 'G' | 'h' | 'H' | 'x' | '=' | 'l' | 'z' => Cmd::Simple(c),
                c => fail(&format!("unknown command: '{c}'")),
            };
            prog.push(Inst { a1, a2, neg, cmd, active: false, end_line: 0 });
        }
        if !blocks.is_empty() {
            fail("unmatched '{'");
        }
        prog
    }
}

struct Sed {
    prog: Vec<Inst>,
    quiet: bool,
    last_re: Option<Rc<Regex>>,
    hold: String,
    line_no: usize,
    wfiles: BTreeMap<String, rt::fs::File>,
    /// print output as it accumulates (not when editing in place)
    flush: bool,
}

struct Input {
    files: Vec<String>,
    idx: usize,
    cur: Option<rt::io::BufReader<Box<dyn Read>>>,
    peeked: Option<String>,
    separate: bool,
    st: i32,
}

impl Input {
    fn raw_next(&mut self) -> Option<String> {
        loop {
            if self.cur.is_none() {
                if self.idx >= self.files.len() {
                    return None;
                }
                let f = self.files[self.idx].clone();
                self.idx += 1;
                match rt::io::reader(&f) {
                    Ok(r) => self.cur = Some(r),
                    Err(e) => {
                        rt::warn!("{f}: {e}");
                        self.st = 2;
                        continue;
                    }
                }
            }
            let mut s = String::new();
            match self.cur.as_mut().unwrap().read_line(&mut s) {
                Ok(n) if n > 0 => {
                    if s.ends_with('\n') {
                        s.pop();
                    }
                    return Some(s);
                }
                _ => {
                    self.cur = None;
                    if self.separate {
                        return None;
                    }
                }
            }
        }
    }
    fn next(&mut self) -> Option<String> {
        if let Some(p) = self.peeked.take() {
            return Some(p);
        }
        self.raw_next()
    }
    fn is_last(&mut self) -> bool {
        if self.peeked.is_none() {
            self.peeked = self.raw_next();
        }
        self.peeked.is_none()
    }
}

fn escape_l(s: &str) -> String {
    let mut out = String::new();
    let mut col = 0;
    for b in s.bytes() {
        let piece = match b {
            b'\\' => "\\\\".into(),
            b'\t' => "\\t".into(),
            b'\n' => "\\n".into(),
            0x07 => "\\a".into(),
            0x08 => "\\b".into(),
            0x0c => "\\f".into(),
            0x0d => "\\r".into(),
            0x0b => "\\v".into(),
            0x20..=0x7e => String::from(b as char),
            _ => format!("\\{b:03o}"),
        };
        if col + piece.len() > 69 {
            out.push_str("\\\n");
            col = 0;
        }
        col += piece.len();
        out.push_str(&piece);
    }
    out.push('$');
    out
}

impl Sed {
    fn re(&mut self, r: &Option<Rc<Regex>>) -> Rc<Regex> {
        match r {
            Some(r) => {
                self.last_re = Some(r.clone());
                r.clone()
            }
            None => self.last_re.clone().unwrap_or_else(|| fail("no previous regular expression")),
        }
    }

    fn match_addr(&mut self, a: &Addr, ps: &str, last: bool) -> bool {
        match a {
            Addr::Line(n) => self.line_no == *n,
            Addr::Last => last,
            Addr::Re(r) => {
                let r = self.re(r);
                r.is_match(ps)
            }
            Addr::Step(f, s) => {
                if *s == 0 {
                    self.line_no == *f
                } else {
                    self.line_no >= *f && (self.line_no - f) % s == 0
                }
            }
            Addr::Plus(_) => false,
        }
    }

    fn selected(&mut self, k: usize, ps: &str, last: bool) -> bool {
        let (a1, a2) = (self.prog[k].a1.clone(), self.prog[k].a2.clone());
        let r = match (a1, a2) {
            (None, _) => true,
            (Some(a1), None) => self.match_addr(&a1, ps, last),
            (Some(a1), Some(a2)) => {
                if self.prog[k].active {
                    let end = match &a2 {
                        Addr::Line(n) => self.line_no >= *n,
                        Addr::Plus(_) => self.line_no >= self.prog[k].end_line,
                        a => self.match_addr(a, ps, last),
                    };
                    if end {
                        self.prog[k].active = false;
                    }
                    true
                } else if self.match_addr(&a1, ps, last) {
                    match &a2 {
                        Addr::Line(n) if *n <= self.line_no => {}
                        Addr::Plus(n) => {
                            if *n > 0 {
                                self.prog[k].active = true;
                                self.prog[k].end_line = self.line_no + n;
                            }
                        }
                        _ => self.prog[k].active = true,
                    }
                    true
                } else {
                    false
                }
            }
        };
        r != self.prog[k].neg
    }

    fn subst(&mut self, ps: &mut String, re: &Option<Rc<Regex>>, rep: &[Rep], global: bool, nth: usize) -> bool {
        let r = self.re(re);
        let chars: Vec<char> = ps.chars().collect();
        let mut out = String::new();
        let mut pos = 0;
        let mut count = 0;
        let mut did = false;
        let mut from = 0;
        while from <= chars.len() {
            let Some(caps) = r.find_chars(&chars, from) else { break };
            let (a, b) = caps[0].unwrap();
            count += 1;
            if count >= nth {
                out.extend(&chars[pos..a]);
                for piece in rep {
                    match piece {
                        Rep::Lit(s) => out.push_str(s),
                        Rep::Group(g) => {
                            if let Some(Some((x, y))) = caps.get(*g) {
                                out.extend(&chars[*x..*y]);
                            }
                        }
                    }
                }
                pos = b;
                did = true;
                if !global {
                    break;
                }
            }
            if b == a {
                // empty match: copy one character and move on
                if b < chars.len() && count >= nth {
                    out.push(chars[b]);
                    pos = b + 1;
                }
                from = b + 1;
            } else {
                from = b;
            }
        }
        if did {
            out.extend(&chars[pos.min(chars.len())..]);
            *ps = out;
        }
        did
    }

    fn write_to(&mut self, file: &str, text: &str) {
        if file == "/dev/stdout" {
            println!("{text}");
            return;
        }
        if !self.wfiles.contains_key(file) {
            match rt::fs::File::create(file) {
                Ok(f) => {
                    self.wfiles.insert(file.into(), f);
                }
                Err(e) => fail(&format!("couldn't open file {file}: {e}")),
            }
        }
        let f = self.wfiles.get_mut(file).unwrap();
        let _ = f.write_all(text.as_bytes());
        let _ = f.write_all(b"\n");
    }

    fn label(&self, name: &str) -> usize {
        if name.is_empty() {
            return self.prog.len();
        }
        for (i, inst) in self.prog.iter().enumerate() {
            if let Cmd::Text(':', l) = &inst.cmd {
                if l == name {
                    return i;
                }
            }
        }
        fail(&format!("can't find label for jump to '{name}'"))
    }

    /// Process one input stream; returns Some(exit code) if `q` was hit.
    fn run(&mut self, input: &mut Input, out: &mut String) -> Option<i32> {
        let mut ps = match input.next() {
            Some(l) => l,
            None => return None,
        };
        self.line_no += 1;
        loop {
            let mut append: Vec<String> = Vec::new();
            let mut tflag = false;
            let mut pc = 0;
            let mut delete = false;
            let mut restart = false;
            let mut quit: Option<(char, i32)> = None;
            while pc < self.prog.len() {
                let last = input.is_last();
                if let Cmd::Text(':', _) = self.prog[pc].cmd {
                    pc += 1;
                    continue;
                }
                if matches!(self.prog[pc].cmd, Cmd::EndBlock) {
                    pc += 1;
                    continue;
                }
                if !self.selected(pc, &ps, last) {
                    if let Cmd::Block(end) = self.prog[pc].cmd {
                        pc = end + 1;
                    } else {
                        pc += 1;
                    }
                    continue;
                }
                let cmd = self.prog[pc].cmd.clone();
                pc += 1;
                match cmd {
                    Cmd::Block(_) | Cmd::EndBlock => {}
                    Cmd::Subst { re, rep, global, nth, print, wfile } => {
                        if self.subst(&mut ps, &re, &rep, global, nth) {
                            tflag = true;
                            if print {
                                out.push_str(&ps);
                                out.push('\n');
                            }
                            if let Some(w) = wfile {
                                let p = ps.clone();
                                self.write_to(&w, &p);
                            }
                        }
                    }
                    Cmd::Trans(a, b) => {
                        ps = ps.chars().map(|c| a.iter().position(|x| *x == c).map_or(c, |i| b[i])).collect();
                    }
                    Cmd::Text(c, t) => match c {
                        'a' => append.push(t),
                        'i' => {
                            out.push_str(&t);
                            out.push('\n');
                        }
                        _ => {
                            // c: print the text at the end of a range
                            if !self.prog[pc - 1].active || self.prog[pc - 1].a2.is_none() || self.prog[pc - 1].neg {
                                out.push_str(&t);
                                out.push('\n');
                            }
                            delete = true;
                            break;
                        }
                    },
                    Cmd::Quit(c, code) => {
                        quit = Some((c, code));
                        break;
                    }
                    Cmd::ReadFile(f) => {
                        if let Ok(t) = rt::fs::read_to_string(&f) {
                            append.push(t.trim_end_matches('\n').into());
                        }
                    }
                    Cmd::WriteFile(f) => {
                        let p = ps.clone();
                        self.write_to(&f, &p);
                    }
                    Cmd::Branch(c, l) => {
                        let jump = match c {
                            'b' => true,
                            't' => tflag,
                            _ => !tflag,
                        };
                        if c != 'b' {
                            tflag = false;
                        }
                        if jump {
                            pc = self.label(&l);
                        }
                    }
                    Cmd::Simple(c) => match c {
                        'd' => {
                            delete = true;
                            break;
                        }
                        'D' => {
                            match ps.find('\n') {
                                Some(i) => {
                                    ps = ps[i + 1..].into();
                                    restart = true;
                                }
                                None => delete = true,
                            }
                            break;
                        }
                        'p' => {
                            out.push_str(&ps);
                            out.push('\n');
                        }
                        'P' => {
                            out.push_str(ps.split('\n').next().unwrap_or(""));
                            out.push('\n');
                        }
                        'n' => {
                            if !self.quiet {
                                out.push_str(&ps);
                                out.push('\n');
                            }
                            for a in append.drain(..) {
                                out.push_str(&a);
                                out.push('\n');
                            }
                            match input.next() {
                                Some(l) => {
                                    ps = l;
                                    self.line_no += 1;
                                }
                                None => return None,
                            }
                        }
                        'N' => match input.next() {
                            Some(l) => {
                                ps.push('\n');
                                ps.push_str(&l);
                                self.line_no += 1;
                            }
                            None => {
                                // GNU prints the pattern space if there is no next line
                                if !self.quiet {
                                    out.push_str(&ps);
                                    out.push('\n');
                                }
                                return None;
                            }
                        },
                        'g' => ps = self.hold.clone(),
                        'G' => {
                            ps.push('\n');
                            ps.push_str(&self.hold);
                        }
                        'h' => self.hold = ps.clone(),
                        'H' => {
                            self.hold.push('\n');
                            self.hold.push_str(&ps);
                        }
                        'x' => core::mem::swap(&mut ps, &mut self.hold),
                        '=' => out.push_str(&format!("{}\n", self.line_no)),
                        'l' => {
                            out.push_str(&escape_l(&ps));
                            out.push('\n');
                        }
                        'z' => ps.clear(),
                        _ => {}
                    },
                }
            }
            if !delete && !restart && quit.is_none_or(|(c, _)| c == 'q') && !self.quiet {
                out.push_str(&ps);
                out.push('\n');
            }
            for a in append {
                out.push_str(&a);
                out.push('\n');
            }
            if self.flush && out.len() > 8192 {
                print!("{out}");
                out.clear();
            }
            if let Some((_, code)) = quit {
                return Some(code);
            }
            if restart {
                continue;
            }
            match input.next() {
                Some(l) => {
                    ps = l;
                    self.line_no += 1;
                }
                None => return None,
            }
        }
    }
}

fn main(args: &[String]) -> i32 {
    // -i takes an optional attached suffix; handle it before getopt
    let mut inplace: Option<String> = None;
    let mut a: Vec<String> = Vec::new();
    for x in &args[1..] {
        if let Some(s) = x.strip_prefix("-i") {
            if !x.starts_with("--") {
                inplace = Some(s.into());
                continue;
            }
        }
        a.push(x.clone());
    }
    let (o, mut rest) = rt::getopt::parse(&a, "nErse:f:u", "[-nEr] [-i[suffix]] [-e script] [-f file] [script] [file...]");
    let mut script = String::new();
    for e in o.all('e') {
        script.push_str(e);
        script.push('\n');
    }
    for f in o.all('f') {
        match rt::fs::read_to_string(f) {
            Ok(t) => {
                script.push_str(&t);
                script.push('\n');
            }
            Err(e) => fail(&format!("couldn't open file {f}: {e}")),
        }
    }
    if script.is_empty() {
        if rest.is_empty() {
            fail("usage: sed [-nEr] [-i] script [file...]");
        }
        script = rest.remove(0);
    }
    let mut p = Parser { s: script.chars().collect(), i: 0, ere: o.has('E') || o.has('r') };
    let prog = p.parse();
    let mut sed = Sed { prog, quiet: o.has('n'), last_re: None, hold: String::new(), line_no: 0, wfiles: BTreeMap::new(), flush: true };
    // a leading "#n" line means -n
    if script.starts_with("#n\n") {
        sed.quiet = true;
    }
    let files = rt::io::inputs(&rest);
    if let Some(suffix) = inplace {
        let mut st = 0;
        for f in &files {
            let mut input = Input { files: vec![f.clone()], idx: 0, cur: None, peeked: None, separate: true, st: 0 };
            let mut out = String::new();
            sed.line_no = 0;
            sed.flush = false;
            let q = sed.run(&mut input, &mut out);
            if input.st != 0 {
                st = input.st;
                continue;
            }
            if !suffix.is_empty() {
                let _ = rt::fs::rename(f, &format!("{f}{suffix}"));
            }
            if let Err(e) = rt::fs::write(f, out.as_bytes()) {
                rt::warn!("{f}: {e}");
                st = 4;
            }
            if let Some(c) = q {
                return c;
            }
        }
        return st;
    }
    let mut input = Input { files, idx: 0, cur: None, peeked: None, separate: o.has('s'), st: 0 };
    let mut out = String::new();
    loop {
        let q = sed.run(&mut input, &mut out);
        if let Some(code) = q {
            print!("{out}");
            return code;
        }
        if !o.has('s') || input.idx >= input.files.len() {
            break;
        }
        sed.line_no = 0;
    }
    print!("{out}");
    input.st
}

