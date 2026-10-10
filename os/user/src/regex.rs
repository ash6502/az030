//! Regular expressions: POSIX basic (BRE) and extended (ERE) syntax, matched by
//! backtracking.
//!
//! Supported: literals, `.`, bracket expressions (ranges, negation, `[:alpha:]`
//! classes), `*`, `+`, `?`, `{m,n}` (`\{m,n\}` in BREs), anchors `^` `$`, grouping
//! `( )` (`\( \)`), alternation `|` (`\|`), back-references `\1`-`\9`, word
//! boundaries `\<` `\>` `\b` `\B`, and the escapes `\w \W \s \S \d \D \t \n`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
enum ClassItem {
    Ch(char),
    Range(char, char),
    Named(fn(char) -> bool),
}

#[derive(Clone, Debug)]
enum Node {
    Char(char),
    Any,
    Class(Vec<ClassItem>, bool),
    Start,
    End,
    WordB,
    NotWordB,
    WordStart,
    WordEnd,
    Group(Box<Vec<Vec<Node>>>, Option<usize>),
    Backref(usize),
    Repeat(Box<Node>, u32, u32, bool),
}

#[derive(Clone, Debug)]
pub struct Regex {
    /// the whole pattern as capture group 0
    root: Vec<Node>,
    ngroups: usize,
    icase: bool,
    /// the pattern starts with `^` in every alternative (search only at 0)
    anchored: bool,
}

/// Capture positions (character indices) of a match: [0] is the whole match.
pub type Captures = Vec<Option<(usize, usize)>>;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    ere: bool,
    ngroups: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn at_alt(&self) -> bool {
        if self.ere {
            self.peek() == Some('|')
        } else {
            self.peek() == Some('\\') && self.s.get(self.i + 1) == Some(&'|')
        }
    }

    fn at_close(&self) -> bool {
        if self.ere {
            self.peek() == Some(')')
        } else {
            self.peek() == Some('\\') && self.s.get(self.i + 1) == Some(&')')
        }
    }

    fn alternation(&mut self, depth: u32) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = Vec::new();
        loop {
            alts.push(self.sequence(depth)?);
            if self.at_alt() {
                self.i += if self.ere { 1 } else { 2 };
                continue;
            }
            return Ok(alts);
        }
    }

    fn sequence(&mut self, depth: u32) -> Result<Vec<Node>, String> {
        let mut seq: Vec<Node> = Vec::new();
        while self.i < self.s.len() {
            if self.at_alt() {
                break;
            }
            if self.at_close() {
                if depth == 0 {
                    if self.ere {
                        // an unmatched ')' is literal in an ERE
                        self.i += 1;
                        seq.push(Node::Char(')'));
                        continue;
                    }
                    return Err("unmatched \\)".into());
                }
                break;
            }
            let c = self.s[self.i];
            // repetition operators
            let rep = match c {
                '*' if !seq.is_empty() => Some((0, u32::MAX)),
                '+' | '?' if self.ere => Some(if c == '+' { (1, u32::MAX) } else { (0, 1) }),
                '{' if self.ere && !seq.is_empty() => self.braces()?,
                '\\' if !self.ere && self.s.get(self.i + 1) == Some(&'{') && !seq.is_empty() => self.braces()?,
                '\\' if !self.ere && matches!(self.s.get(self.i + 1), Some('+') | Some('?')) && !seq.is_empty() => {
                    let q = self.s[self.i + 1];
                    self.i += 1;
                    Some(if q == '+' { (1, u32::MAX) } else { (0, 1) })
                }
                _ => None,
            };
            if let Some((min, max)) = rep {
                if c != '{' && !(c == '\\' && self.s.get(self.i + 1) == Some(&'{')) {
                    self.i += 1;
                }
                let Some(last) = seq.pop() else {
                    seq.push(Node::Char(c));
                    continue;
                };
                if matches!(last, Node::Start | Node::End) {
                    seq.push(last);
                    seq.push(Node::Char(c));
                    continue;
                }
                let greedy = !(self.ere && self.peek() == Some('?'));
                if !greedy {
                    self.i += 1;
                }
                seq.push(Node::Repeat(Box::new(last), min, max, greedy));
                continue;
            }
            if c == '*' {
                // leading '*' in a BRE is literal
                self.i += 1;
                seq.push(Node::Char('*'));
                continue;
            }
            self.i += 1;
            let node = match c {
                '.' => Node::Any,
                '^' if self.ere || seq.is_empty() => Node::Start,
                '$' if self.ere || self.i == self.s.len() || self.at_close() || self.at_alt() => Node::End,
                '[' => self.bracket()?,
                '(' if self.ere => self.group(depth)?,
                '\\' => {
                    let Some(n) = self.peek() else { return Err("trailing backslash".into()) };
                    self.i += 1;
                    match n {
                        '(' if !self.ere => self.group(depth)?,
                        '1'..='9' => Node::Backref(n as usize - '0' as usize),
                        '<' => Node::WordStart,
                        '>' => Node::WordEnd,
                        'b' => Node::WordB,
                        'B' => Node::NotWordB,
                        'w' => Node::Class(alloc::vec![ClassItem::Named(is_word)], false),
                        'W' => Node::Class(alloc::vec![ClassItem::Named(is_word)], true),
                        's' => Node::Class(alloc::vec![ClassItem::Named(char::is_whitespace)], false),
                        'S' => Node::Class(alloc::vec![ClassItem::Named(char::is_whitespace)], true),
                        'd' => Node::Class(alloc::vec![ClassItem::Range('0', '9')], false),
                        'D' => Node::Class(alloc::vec![ClassItem::Range('0', '9')], true),
                        't' => Node::Char('\t'),
                        'n' => Node::Char('\n'),
                        c => Node::Char(c),
                    }
                }
                c => Node::Char(c),
            };
            seq.push(node);
        }
        Ok(seq)
    }

    fn group(&mut self, depth: u32) -> Result<Node, String> {
        self.ngroups += 1;
        let idx = self.ngroups;
        let alts = self.alternation(depth + 1)?;
        if !self.at_close() {
            return Err("unmatched (".into());
        }
        self.i += if self.ere { 1 } else { 2 };
        Ok(Node::Group(Box::new(alts), Some(idx)))
    }

    /// `{m}`, `{m,}`, `{m,n}` (ERE) or `\{...\}` (BRE); None if not a valid interval
    /// (then the brace is literal).
    fn braces(&mut self) -> Result<Option<(u32, u32)>, String> {
        let save = self.i;
        self.i += if self.ere { 1 } else { 2 };
        let num = |p: &mut Self| {
            let st = p.i;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.i += 1;
            }
            p.s[st..p.i].iter().collect::<String>().parse::<u32>().ok()
        };
        let min = num(self);
        let max = if self.peek() == Some(',') {
            self.i += 1;
            num(self).unwrap_or(u32::MAX)
        } else {
            min.unwrap_or(0)
        };
        let close = if self.ere {
            self.peek() == Some('}')
        } else {
            self.peek() == Some('\\') && self.s.get(self.i + 1) == Some(&'}')
        };
        match (min, close) {
            (Some(m), true) if m <= max => {
                self.i += if self.ere { 1 } else { 2 };
                Ok(Some((m, max)))
            }
            _ if self.ere => {
                self.i = save;
                Ok(None)
            }
            _ => Err("invalid \\{\\} interval".into()),
        }
    }

    fn bracket(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        let neg = self.peek() == Some('^');
        if neg {
            self.i += 1;
        }
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err("unmatched [".into()) };
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            if c == '[' && self.s.get(self.i + 1) == Some(&':') {
                let rest: String = self.s[self.i + 2..].iter().collect();
                if let Some(end) = rest.find(":]") {
                    let name = &rest[..end];
                    let f: fn(char) -> bool = match name {
                        "alpha" => char::is_alphabetic,
                        "digit" => |c| c.is_ascii_digit(),
                        "alnum" => char::is_alphanumeric,
                        "upper" => char::is_uppercase,
                        "lower" => char::is_lowercase,
                        "space" => char::is_whitespace,
                        "blank" => |c| c == ' ' || c == '\t',
                        "punct" => |c| c.is_ascii_punctuation(),
                        "print" => |c| !c.is_control(),
                        "graph" => |c| !c.is_control() && c != ' ',
                        "cntrl" => char::is_control,
                        "xdigit" => |c| c.is_ascii_hexdigit(),
                        _ => return Err(alloc::format!("invalid character class [:{name}:]")),
                    };
                    items.push(ClassItem::Named(f));
                    self.i += 2 + end + 2;
                    continue;
                }
            }
            self.i += 1;
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                let hi = self.s[self.i + 1];
                self.i += 2;
                items.push(ClassItem::Range(c, hi));
            } else {
                items.push(ClassItem::Ch(c));
            }
        }
        Ok(Node::Class(items, neg))
    }
}

struct Matcher<'a> {
    s: &'a [char],
    caps: Captures,
    icase: bool,
    steps: u32,
}

fn fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

impl Matcher<'_> {
    fn class_has(&self, items: &[ClassItem], c: char) -> bool {
        let test = |c: char| {
            items.iter().any(|it| match *it {
                ClassItem::Ch(x) => x == c,
                ClassItem::Range(a, b) => a <= c && c <= b,
                ClassItem::Named(f) => f(c),
            })
        };
        test(c) || (self.icase && (test(fold(c)) || test(c.to_uppercase().next().unwrap_or(c))))
    }

    /// Does a single-character node match at i?
    fn single(&self, n: &Node, i: usize) -> Option<bool> {
        let c = self.s.get(i).copied();
        Some(match n {
            Node::Char(x) => c.is_some_and(|c| c == *x || (self.icase && fold(c) == fold(*x))),
            Node::Any => c.is_some_and(|c| c != '\n'),
            Node::Class(items, neg) => c.is_some_and(|c| self.class_has(items, c) != *neg),
            _ => return None,
        })
    }

    fn at_word(&self, i: usize) -> bool {
        self.s.get(i).is_some_and(|&c| is_word(c))
    }

    fn before_word(&self, i: usize) -> bool {
        i > 0 && is_word(self.s[i - 1])
    }

    /// Match `seq` at `i`, then the continuation `k`.
    fn seq(&mut self, seq: &[Node], i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        self.steps += 1;
        if self.steps > 2_000_000 {
            return false;
        }
        let Some((n, rest)) = seq.split_first() else { return k(self, i) };
        if let Some(ok) = self.single(n, i) {
            return ok && self.seq(rest, i + 1, k);
        }
        match n {
            Node::Start => i == 0 && self.seq(rest, i, k),
            Node::End => i == self.s.len() && self.seq(rest, i, k),
            Node::WordB => (self.before_word(i) != self.at_word(i)) && self.seq(rest, i, k),
            Node::NotWordB => (self.before_word(i) == self.at_word(i)) && self.seq(rest, i, k),
            Node::WordStart => !self.before_word(i) && self.at_word(i) && self.seq(rest, i, k),
            Node::WordEnd => self.before_word(i) && !self.at_word(i) && self.seq(rest, i, k),
            Node::Backref(g) => {
                let Some(Some((a, b))) = self.caps.get(*g).copied() else { return self.seq(rest, i, k) };
                let len = b - a;
                if i + len > self.s.len() {
                    return false;
                }
                for j in 0..len {
                    let (x, y) = (self.s[a + j], self.s[i + j]);
                    if x != y && !(self.icase && fold(x) == fold(y)) {
                        return false;
                    }
                }
                self.seq(rest, i + len, k)
            }
            Node::Group(alts, idx) => {
                for alt in alts.iter() {
                    let saved = idx.map(|g| self.caps[g]);
                    let ok = self.seq(alt, i, &mut |m: &mut Self, j| {
                        let old = idx.map(|g| m.caps[g]);
                        if let Some(g) = idx {
                            m.caps[*g] = Some((i, j));
                        }
                        if m.seq(rest, j, k) {
                            return true;
                        }
                        if let (Some(g), Some(o)) = (idx, old) {
                            m.caps[*g] = o;
                        }
                        false
                    });
                    if ok {
                        return true;
                    }
                    if let (Some(g), Some(s)) = (idx, saved) {
                        self.caps[*g] = s;
                    }
                }
                false
            }
            Node::Repeat(inner, min, max, greedy) => {
                if self.single(inner, i).is_some() {
                    // fast path: count how many single characters match
                    let mut n = 0u32;
                    while n < *max && self.single(inner, i + n as usize) == Some(true) {
                        n += 1;
                    }
                    if n < *min {
                        return false;
                    }
                    if *greedy {
                        let mut c = n;
                        loop {
                            if self.seq(rest, i + c as usize, k) {
                                return true;
                            }
                            if c == *min {
                                return false;
                            }
                            c -= 1;
                        }
                    } else {
                        for c in *min..=n {
                            if self.seq(rest, i + c as usize, k) {
                                return true;
                            }
                        }
                        return false;
                    }
                }
                self.repeat(inner, *min, *max, *greedy, 0, i, rest, k)
            }
            _ => unreachable!(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(&mut self, inner: &Node, min: u32, max: u32, greedy: bool, count: u32, i: usize, rest: &[Node], k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        let try_more = |m: &mut Self, k: &mut dyn FnMut(&mut Self, usize) -> bool| {
            count < max
                && m.seq(core::slice::from_ref(inner), i, &mut |m: &mut Self, j| {
                    // an empty iteration cannot make progress
                    j != i && m.repeat(inner, min, max, greedy, count + 1, j, rest, k)
                })
        };
        if count < min {
            return try_more(self, k);
        }
        if greedy {
            try_more(self, k) || self.seq(rest, i, k)
        } else {
            self.seq(rest, i, k) || try_more(self, k)
        }
    }
}

impl Regex {
    /// Compile a basic (`ere` = false) or extended regular expression.
    pub fn new(pattern: &str, ere: bool, icase: bool) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0, ere, ngroups: 0 };
        let alts = p.alternation(0)?;
        if p.i < chars.len() {
            return Err("unmatched )".into());
        }
        let anchored = alts.iter().all(|a| matches!(a.first(), Some(Node::Start)));
        Ok(Regex { root: alloc::vec![Node::Group(Box::new(alts), Some(0))], ngroups: p.ngroups, icase, anchored })
    }

    pub fn groups(&self) -> usize {
        self.ngroups
    }

    /// Find the first (leftmost, then longest-preferred by greediness) match in
    /// `s` at or after character `from`.
    pub fn find_chars(&self, s: &[char], from: usize) -> Option<Captures> {
        let last = if self.anchored { from.min(0) } else { s.len() };
        for start in from..=last {
            let mut m = Matcher { s, caps: alloc::vec![None; self.ngroups + 1], icase: self.icase, steps: 0 };
            if m.seq(&self.root, start, &mut |_, _| true) {
                return Some(m.caps);
            }
            if self.anchored {
                break;
            }
        }
        None
    }

    /// Byte-offset match: (start, end) of the first match in `s`.
    pub fn find(&self, s: &str) -> Option<(usize, usize)> {
        let chars: Vec<char> = s.chars().collect();
        let caps = self.find_chars(&chars, 0)?;
        let (a, b) = caps[0]?;
        Some((char_to_byte(s, a), char_to_byte(s, b)))
    }

    pub fn is_match(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        self.find_chars(&chars, 0).is_some()
    }
}

/// Convert a character index into a byte offset.
pub fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}
