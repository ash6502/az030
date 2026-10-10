//! Word expansion: tilde, parameters, command substitution, arithmetic, field
//! splitting, pathname expansion and quote removal.

use crate::Shell;
use rt::prelude::*;

/// A field under construction: characters with a "quoted" flag (quoted characters
/// are not subject to pathname expansion).
#[derive(Default, Clone)]
struct Field {
    chars: Vec<(char, bool)>,
    /// contains a quoted part (so it survives even if empty)
    quoted: bool,
}

struct Builder {
    fields: Vec<Field>,
    cur: Field,
    split: bool,
    ifs: String,
}

impl Builder {
    fn lit(&mut self, c: char, quoted: bool) {
        self.cur.chars.push((c, quoted));
        if quoted {
            self.cur.quoted = true;
        }
    }

    fn quoted_empty(&mut self) {
        self.cur.quoted = true;
    }

    fn end_field(&mut self) {
        let f = core::mem::take(&mut self.cur);
        if !f.chars.is_empty() || f.quoted {
            self.fields.push(f);
        }
    }

    /// The result of an expansion: split on IFS unless quoted.
    fn text(&mut self, s: &str, quoted: bool) {
        if quoted || !self.split {
            for c in s.chars() {
                self.cur.chars.push((c, quoted));
            }
            if quoted {
                self.cur.quoted = true;
            }
            return;
        }
        let ifs: Vec<char> = self.ifs.chars().collect();
        let is_ws = |c: char| ifs.contains(&c) && c.is_whitespace();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if ifs.contains(&c) {
                // a run of IFS whitespace (plus at most one non-whitespace IFS char) ends a field
                let mut hard = !c.is_whitespace();
                while let Some(&n) = chars.peek() {
                    if is_ws(n) {
                        chars.next();
                    } else if ifs.contains(&n) && !hard {
                        hard = true;
                        chars.next();
                    } else {
                        break;
                    }
                }
                if hard {
                    let f = core::mem::take(&mut self.cur);
                    self.fields.push(f);
                } else {
                    self.end_field();
                }
            } else {
                self.cur.chars.push((c, false));
            }
        }
    }
}

/// Find the index just past the matching `close` for an opening at `i` (which is
/// just past the opener), skipping quotes.
fn find_close(s: &[char], mut i: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 1;
    while i < s.len() {
        let c = s[i];
        i += 1;
        match c {
            '\\' => i += 1,
            '\'' if close != '`' => {
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '"' if close != '`' => {
                while i < s.len() && s[i] != '"' {
                    if s[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            c if c == open && open != close => depth += 1,
            _ => {}
        }
    }
    None
}

/// Remove quotes without expanding anything (here-document delimiters).
pub fn unquote(w: &str) -> String {
    let mut out = String::new();
    let mut it = w.chars();
    let mut dq = false;
    while let Some(c) = it.next() {
        match c {
            '\'' if !dq => {
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    out.push(c);
                }
            }
            '"' => dq = !dq,
            '\\' => {
                if let Some(n) = it.next() {
                    out.push(n);
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn glob_escape(f: &Field) -> (String, bool) {
    let mut s = String::new();
    let mut wild = false;
    for &(c, q) in &f.chars {
        if q && matches!(c, '*' | '?' | '[' | ']' | '\\') {
            s.push('\\');
        } else if !q && matches!(c, '*' | '?' | '[') {
            wild = true;
        }
        s.push(c);
    }
    (s, wild)
}

impl Shell {
    fn ifs(&self) -> String {
        self.get_var("IFS").unwrap_or_else(|| " \t\n".into())
    }

    /// Expand command words: the full treatment, producing zero or more fields.
    pub fn expand_words(&mut self, words: &[String]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for w in words {
            let fields = self.expand_fields(w, true)?;
            for f in fields {
                let (pat, wild) = glob_escape(&f);
                if wild && !self.opt_noglob {
                    let m = rt::glob::expand(&pat);
                    if !m.is_empty() {
                        out.extend(m);
                        continue;
                    }
                }
                out.push(f.chars.iter().map(|(c, _)| *c).collect());
            }
        }
        Ok(out)
    }

    /// Expand to a single string (assignments, redirection targets, `case` words).
    pub fn expand_one(&mut self, w: &str) -> Result<String, String> {
        let fields = self.expand_fields(w, false)?;
        let mut s = String::new();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.extend(f.chars.iter().map(|(c, _)| *c));
        }
        Ok(s)
    }

    /// Expand a pattern (`case`, `${x#pat}`): quoted characters are escaped.
    pub fn expand_pattern(&mut self, w: &str) -> Result<String, String> {
        let fields = self.expand_fields(w, false)?;
        Ok(fields.iter().map(|f| glob_escape(f).0).collect::<Vec<_>>().join(" "))
    }

    /// Expand a here-document body: parameters, command substitution, arithmetic and
    /// backslash before `$`, `` ` `` and `\`.
    pub fn expand_heredoc(&mut self, body: &str) -> Result<String, String> {
        let src: Vec<char> = body.chars().collect();
        let mut b = Builder { fields: Vec::new(), cur: Field::default(), split: false, ifs: String::new() };
        let mut i = 0;
        while i < src.len() {
            let c = src[i];
            if c == '\\' && i + 1 < src.len() && matches!(src[i + 1], '$' | '`' | '\\' | '\n') {
                if src[i + 1] != '\n' {
                    b.lit(src[i + 1], true);
                }
                i += 2;
            } else if c == '$' || c == '`' {
                i = self.dollar(&src, i, true, &mut b)?;
            } else {
                b.lit(c, true);
                i += 1;
            }
        }
        b.end_field();
        Ok(b.fields.iter().flat_map(|f| f.chars.iter().map(|(c, _)| *c)).collect())
    }

    fn expand_fields(&mut self, w: &str, split: bool) -> Result<Vec<Field>, String> {
        let src: Vec<char> = w.chars().collect();
        let mut b = Builder { fields: Vec::new(), cur: Field::default(), split, ifs: self.ifs() };
        let mut i = 0;
        // tilde expansion
        if src.first() == Some(&'~') {
            let end = src.iter().position(|&c| c == '/').unwrap_or(src.len());
            let user: String = src[1..end].iter().collect();
            if user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                let home = if user.is_empty() {
                    self.get_var("HOME")
                } else {
                    rt::users::by_name(&user).map(|u| u.home)
                };
                if let Some(h) = home {
                    for c in h.chars() {
                        b.lit(c, true);
                    }
                    i = end;
                }
            }
        }
        while i < src.len() {
            let c = src[i];
            match c {
                '\'' => {
                    i += 1;
                    b.quoted_empty();
                    while i < src.len() && src[i] != '\'' {
                        b.lit(src[i], true);
                        i += 1;
                    }
                    i += 1;
                }
                '"' => {
                    i += 1;
                    b.quoted_empty();
                    while i < src.len() && src[i] != '"' {
                        let c = src[i];
                        if c == '\\' && i + 1 < src.len() {
                            let n = src[i + 1];
                            if matches!(n, '$' | '`' | '"' | '\\') {
                                b.lit(n, true);
                            } else if n != '\n' {
                                b.lit('\\', true);
                                b.lit(n, true);
                            }
                            i += 2;
                        } else if c == '$' || c == '`' {
                            i = self.dollar(&src, i, true, &mut b)?;
                        } else {
                            b.lit(c, true);
                            i += 1;
                        }
                    }
                    i += 1;
                }
                '\\' => {
                    if i + 1 < src.len() && src[i + 1] != '\n' {
                        b.lit(src[i + 1], true);
                    }
                    i += 2;
                }
                '$' | '`' => i = self.dollar(&src, i, false, &mut b)?,
                c => {
                    b.lit(c, false);
                    i += 1;
                }
            }
        }
        b.end_field();
        Ok(b.fields)
    }

    /// Expand a `$...` or backquote construct at `src[i]`; returns the index after it.
    fn dollar(&mut self, src: &[char], i: usize, dq: bool, b: &mut Builder) -> Result<usize, String> {
        if src[i] == '`' {
            let end = find_close(src, i + 1, '`', '`').ok_or("missing '`'")?;
            let mut cmd = String::new();
            let mut k = i + 1;
            while k < end - 1 {
                if src[k] == '\\' && k + 1 < end - 1 && matches!(src[k + 1], '$' | '`' | '\\') {
                    k += 1;
                }
                cmd.push(src[k]);
                k += 1;
            }
            let out = self.command_subst(&cmd);
            b.text(&out, dq);
            return Ok(end);
        }
        let Some(&n) = src.get(i + 1) else {
            b.lit('$', dq);
            return Ok(i + 1);
        };
        match n {
            '(' if src.get(i + 2) == Some(&'(') => {
                // $(( arithmetic )) -- or a command substitution starting with a subshell
                if let Some(end) = find_close(src, i + 3, '(', ')') {
                    if src.get(end) == Some(&')') {
                        let inner: String = src[i + 3..end - 1].iter().collect();
                        let e = self.expand_heredoc(&inner)?;
                        let v = crate::arith::eval(self, &e)?;
                        b.text(&format!("{v}"), dq);
                        return Ok(end + 1);
                    }
                }
                let end = find_close(src, i + 2, '(', ')').ok_or("missing ')'")?;
                let cmd: String = src[i + 2..end - 1].iter().collect();
                let out = self.command_subst(&cmd);
                b.text(&out, dq);
                Ok(end)
            }
            '(' => {
                let end = find_close(src, i + 2, '(', ')').ok_or("missing ')'")?;
                let cmd: String = src[i + 2..end - 1].iter().collect();
                let out = self.command_subst(&cmd);
                b.text(&out, dq);
                Ok(end)
            }
            '{' => {
                let end = find_close(src, i + 2, '{', '}').ok_or("missing '}'")?;
                let inner: String = src[i + 2..end - 1].iter().collect();
                self.brace_param(&inner, dq, b)?;
                Ok(end)
            }
            '@' | '*' => {
                self.all_params(n == '@', dq, b);
                Ok(i + 2)
            }
            c if c.is_ascii_digit() || matches!(c, '?' | '$' | '!' | '#' | '-') => {
                let v = self.special(c).unwrap_or_default();
                b.text(&v, dq);
                Ok(i + 2)
            }
            c if c == '_' || c.is_ascii_alphabetic() => {
                let mut k = i + 1;
                while k < src.len() && (src[k] == '_' || src[k].is_ascii_alphanumeric()) {
                    k += 1;
                }
                let name: String = src[i + 1..k].iter().collect();
                let v = self.get_var(&name).unwrap_or_default();
                b.text(&v, dq);
                Ok(k)
            }
            _ => {
                b.lit('$', dq);
                Ok(i + 1)
            }
        }
    }

    /// `$@` / `$*`.
    fn all_params(&mut self, at: bool, dq: bool, b: &mut Builder) {
        let params = self.params.clone();
        if dq && at {
            if params.is_empty() {
                // "$@" with no parameters produces no field
                b.cur.quoted = b.cur.quoted && !b.cur.chars.is_empty();
                return;
            }
            for (k, p) in params.iter().enumerate() {
                if k > 0 {
                    let f = core::mem::take(&mut b.cur);
                    b.fields.push(f);
                }
                b.text(p, true);
            }
        } else if dq {
            let sep = self.ifs().chars().next().map(String::from).unwrap_or_default();
            b.text(&params.join(&sep), true);
        } else {
            for (k, p) in params.iter().enumerate() {
                if k > 0 {
                    b.end_field();
                }
                b.text(p, false);
            }
        }
    }

    pub fn special(&self, c: char) -> Option<String> {
        Some(match c {
            '?' => format!("{}", self.status),
            '$' => format!("{}", self.pid),
            '!' => {
                if self.last_bg == 0 {
                    return None;
                }
                format!("{}", self.last_bg)
            }
            '#' => format!("{}", self.params.len()),
            '-' => {
                let mut s = String::new();
                if self.opt_errexit {
                    s.push('e');
                }
                if self.interactive {
                    s.push('i');
                }
                if self.opt_xtrace {
                    s.push('x');
                }
                if self.opt_noglob {
                    s.push('f');
                }
                s
            }
            '0' => self.arg0.clone(),
            c if c.is_ascii_digit() => self.params.get(c as usize - '1' as usize)?.clone(),
            _ => return None,
        })
    }

    /// The value of a parameter by name (variable, positional or special).
    fn param(&self, name: &str) -> Option<String> {
        if let Ok(n) = name.parse::<usize>() {
            return if n == 0 { Some(self.arg0.clone()) } else { self.params.get(n - 1).cloned() };
        }
        let mut cs = name.chars();
        if let (Some(c), None) = (cs.next(), cs.next()) {
            if matches!(c, '?' | '$' | '!' | '#' | '-') {
                return self.special(c);
            }
            if c == '@' || c == '*' {
                return Some(self.params.join(" "));
            }
        }
        self.get_var(name)
    }

    fn brace_param(&mut self, inner: &str, dq: bool, b: &mut Builder) -> Result<(), String> {
        if let Some(name) = inner.strip_prefix('#') {
            if !name.is_empty() {
                let n = if name == "@" || name == "*" {
                    self.params.len()
                } else {
                    self.param(name).map(|v| v.chars().count()).unwrap_or(0)
                };
                b.text(&format!("{n}"), dq);
                return Ok(());
            }
        }
        let name_len = if inner.starts_with(|c: char| c.is_ascii_digit()) {
            inner.find(|c: char| !c.is_ascii_digit()).unwrap_or(inner.len())
        } else if inner.starts_with(['@', '*', '?', '$', '!', '#', '-']) {
            1
        } else {
            inner.find(|c: char| !(c == '_' || c.is_ascii_alphanumeric())).unwrap_or(inner.len())
        };
        if name_len == 0 {
            return Err(format!("${{{inner}}}: bad substitution"));
        }
        let name = &inner[..name_len];
        let rest = &inner[name_len..];
        if rest.is_empty() {
            if name == "@" || name == "*" {
                self.all_params(name == "@", dq, b);
            } else {
                let v = self.param(name).unwrap_or_default();
                b.text(&v, dq);
            }
            return Ok(());
        }
        let val = self.param(name);
        let (colon, op_rest) = match rest.strip_prefix(':') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let op = op_rest.chars().next().unwrap();
        let word = &op_rest[op.len_utf8()..];
        let unset = match &val {
            None => true,
            Some(v) => colon && v.is_empty(),
        };
        match op {
            '-' => {
                if unset {
                    let w = self.expand_one(word)?;
                    b.text(&w, dq);
                } else {
                    b.text(&val.unwrap(), dq);
                }
            }
            '=' => {
                if unset {
                    let w = self.expand_one(word)?;
                    self.set_var(name, &w)?;
                    b.text(&w, dq);
                } else {
                    b.text(&val.unwrap(), dq);
                }
            }
            '+' => {
                if !unset {
                    let w = self.expand_one(word)?;
                    b.text(&w, dq);
                }
            }
            '?' => {
                if unset {
                    let w = self.expand_one(word)?;
                    return Err(if w.is_empty() { format!("{name}: parameter not set") } else { format!("{name}: {w}") });
                }
                b.text(&val.unwrap(), dq);
            }
            '#' | '%' if !colon => {
                let v = val.unwrap_or_default();
                let longest = word.starts_with(op);
                let pat = self.expand_pattern(if longest { &word[1..] } else { word })?;
                let chars: Vec<char> = v.chars().collect();
                let s = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
                let n = chars.len();
                let mut result = v.clone();
                if op == '#' {
                    let mut cands: Vec<usize> = (0..=n).collect();
                    if longest {
                        cands.reverse();
                    }
                    for k in cands {
                        if rt::glob::matches(&pat, &s(0, k)) {
                            result = s(k, n);
                            break;
                        }
                    }
                } else {
                    let mut cands: Vec<usize> = (0..=n).rev().collect();
                    if longest {
                        cands.reverse();
                    }
                    for k in cands {
                        if rt::glob::matches(&pat, &s(k, n)) {
                            result = s(0, k);
                            break;
                        }
                    }
                }
                b.text(&result, dq);
            }
            _ => return Err(format!("${{{inner}}}: bad substitution")),
        }
        Ok(())
    }
}
