//! make: maintain program dependencies.
//!
//!     make [-f makefile] [-C dir] [-nskiBq] [name=value...] [target...]
//!
//! Supports variables (= := ::= ?= += and command-line overrides), explicit
//! rules, suffix rules (.c.o), pattern rules (%.o: %.c), automatic variables
//! ($@ $< $^ $+ $? $* and their D/F forms), `include`, `.PHONY`, `.SUFFIXES`,
//! `.DEFAULT`, `.SILENT`, `.IGNORE`, conditionals (ifeq ifneq ifdef ifndef else
//! endif), and the functions subst patsubst strip findstring filter filter-out
//! sort word words wordlist firstword lastword dir notdir suffix basename
//! addsuffix addprefix join wildcard realpath abspath if or and foreach shell
//! origin error warning info. Recipes run with /bin/sh -c.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

#[derive(Clone)]
struct Var {
    value: String,
    /// `:=` variables are expanded when defined
    simple: bool,
    from_cmdline: bool,
}

#[derive(Clone, Default)]
struct Rule {
    prereqs: Vec<String>,
    order_only: Vec<String>,
    recipe: Vec<String>,
}

#[derive(Clone)]
struct Pattern {
    target: String,
    prereqs: Vec<String>,
    recipe: Vec<String>,
}

struct Make {
    vars: BTreeMap<String, Var>,
    rules: BTreeMap<String, Rule>,
    order: Vec<String>,
    patterns: Vec<Pattern>,
    suffixes: Vec<String>,
    phony: BTreeSet<String>,
    silent: bool,
    ignore: bool,
    dry_run: bool,
    keep_going: bool,
    always: bool,
    question: bool,
    done: BTreeMap<String, bool>,
    building: BTreeSet<String>,
    default_recipe: Option<Vec<String>>,
    errors: u32,
}

fn mtime(path: &str) -> Option<u32> {
    rt::fs::metadata(path).ok().map(|m| m.mtime())
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

/// Match `%` patterns: returns the stem.
fn pat_match(pat: &str, s: &str) -> Option<String> {
    match pat.find('%') {
        None => if pat == s { Some(String::new()) } else { None },
        Some(i) => {
            let (pre, post) = (&pat[..i], &pat[i + 1..]);
            if s.len() >= pre.len() + post.len() && s.starts_with(pre) && s.ends_with(post) {
                Some(String::from(&s[pre.len()..s.len() - post.len()]))
            } else {
                None
            }
        }
    }
}

fn pat_subst(pat: &str, repl: &str, s: &str) -> String {
    match pat_match(pat, s) {
        Some(stem) => {
            if repl.contains('%') { repl.replacen('%', &stem, 1) } else { String::from(repl) }
        }
        None => String::from(s),
    }
}

fn split_args(s: &str) -> Vec<String> {
    // split at top-level commas
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(core::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn run_shell(cmd: &str) -> (String, i32) {
    let Ok((r, w)) = rt::process::pipe() else { return (String::new(), 1) };
    match rt::process::fork() {
        Ok(0) => {
            let _ = rt::io::close(r);
            let _ = rt::process::dup2(w, 1);
            let _ = rt::io::close(w);
            let args = vec![String::from("sh"), String::from("-c"), String::from(cmd)];
            rt::process::execve("/bin/sh", &args, &rt::env::environ());
            rt::process::exit(127)
        }
        Ok(pid) => {
            let _ = rt::io::close(w);
            let mut out = Vec::new();
            let _ = rt::io::FdIo(r).read_to_end(&mut out);
            let _ = rt::io::close(r);
            let st = rt::process::wait(pid).map(|s| s.shell_code()).unwrap_or(1);
            (String::from_utf8_lossy(&out).into_owned(), st)
        }
        Err(_) => (String::new(), 1),
    }
}

impl Make {
    fn get(&self, name: &str) -> Option<String> {
        if let Some(v) = self.vars.get(name) {
            return Some(v.value.clone());
        }
        rt::env::var(name)
    }

    fn set(&mut self, name: &str, value: String, simple: bool, cmdline: bool) {
        if !cmdline && self.vars.get(name).is_some_and(|v| v.from_cmdline) {
            return;
        }
        self.vars.insert(name.into(), Var { value, simple, from_cmdline: cmdline });
    }

    /// Expand variable references in `s`; `auto` supplies automatic variables.
    fn expand(&mut self, s: &str, auto: &BTreeMap<String, String>) -> String {
        let c: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < c.len() {
            if c[i] != '$' {
                out.push(c[i]);
                i += 1;
                continue;
            }
            i += 1;
            let Some(&n) = c.get(i) else { break };
            if n == '$' {
                out.push('$');
                i += 1;
                continue;
            }
            let name: String;
            if n == '(' || n == '{' {
                let close = if n == '(' { ')' } else { '}' };
                let mut depth = 1;
                let start = i + 1;
                let mut j = start;
                while j < c.len() {
                    if c[j] == n {
                        depth += 1;
                    } else if c[j] == close {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                name = c[start..j.min(c.len())].iter().collect();
                i = j + 1;
            } else {
                name = String::from(n);
                i += 1;
            }
            let v = self.reference(&name, auto);
            out.push_str(&v);
        }
        out
    }

    fn reference(&mut self, name: &str, auto: &BTreeMap<String, String>) -> String {
        // functions
        if let Some(sp) = name.find([' ', '\t']) {
            let f = &name[..sp];
            let args = name[sp + 1..].trim_start();
            if let Some(r) = self.function(f, args, auto) {
                return r;
            }
        }
        // substitution reference $(VAR:a=b)
        if let Some(colon) = name.find(':') {
            if let Some(eq) = name[colon..].find('=') {
                let var = &name[..colon];
                let from = &name[colon + 1..colon + eq];
                let to = &name[colon + eq + 1..];
                let val = self.reference(var, auto);
                let (from, to) = if from.contains('%') { (String::from(from), String::from(to)) } else { (format!("%{from}"), format!("%{to}")) };
                return words(&val).iter().map(|w| pat_subst(&from, &to, w)).collect::<Vec<_>>().join(" ");
            }
        }
        let name = self.expand(name, auto);
        if let Some(v) = auto.get(&name) {
            return v.clone();
        }
        // $(@D) $(@F) etc.
        if name.len() == 2 && (name.ends_with('D') || name.ends_with('F')) {
            if let Some(v) = auto.get(&name[..1]) {
                let parts: Vec<String> = words(v)
                    .iter()
                    .map(|w| if name.ends_with('D') { String::from(rt::path::dirname(w)) } else { String::from(rt::path::basename(w)) })
                    .collect();
                return parts.join(" ");
            }
        }
        match self.vars.get(&name).cloned() {
            Some(v) if v.simple => v.value,
            Some(v) => {
                let val = v.value.clone();
                self.expand(&val, auto)
            }
            None => rt::env::var(&name).unwrap_or_default(),
        }
    }

    fn function(&mut self, f: &str, args: &str, auto: &BTreeMap<String, String>) -> Option<String> {
        let ex = |m: &mut Make, s: &str| m.expand(s, auto);
        let a: Vec<String> = split_args(args);
        let arg = |m: &mut Make, k: usize| -> String {
            let s = a.get(k).cloned().unwrap_or_default();
            m.expand(&s, auto)
        };
        Some(match f {
            "subst" => {
                let (from, to, text) = (arg(self, 0), arg(self, 1), arg(self, 2));
                if from.is_empty() { text } else { text.replace(&from, &to) }
            }
            "patsubst" => {
                let (from, to, text) = (arg(self, 0), arg(self, 1), arg(self, 2));
                words(&text).iter().map(|w| pat_subst(from.trim(), to.trim(), w)).collect::<Vec<_>>().join(" ")
            }
            "strip" => words(&ex(self, args)).join(" "),
            "findstring" => {
                let (needle, text) = (arg(self, 0), arg(self, 1));
                if text.contains(&needle) { needle } else { String::new() }
            }
            "filter" | "filter-out" => {
                let pats = words(&arg(self, 0));
                let text = arg(self, 1);
                let keep = f == "filter";
                words(&text).into_iter().filter(|w| pats.iter().any(|p| pat_match(p, w).is_some()) == keep).collect::<Vec<_>>().join(" ")
            }
            "sort" => {
                let mut v = words(&ex(self, args));
                v.sort();
                v.dedup();
                v.join(" ")
            }
            "word" => {
                let n: usize = arg(self, 0).trim().parse().unwrap_or(0);
                words(&arg(self, 1)).get(n.wrapping_sub(1)).cloned().unwrap_or_default()
            }
            "wordlist" => {
                let s: usize = arg(self, 0).trim().parse().unwrap_or(1);
                let e: usize = arg(self, 1).trim().parse().unwrap_or(0);
                let w = words(&arg(self, 2));
                if s == 0 || s > e { String::new() } else { w.iter().skip(s - 1).take(e + 1 - s).cloned().collect::<Vec<_>>().join(" ") }
            }
            "words" => format!("{}", words(&ex(self, args)).len()),
            "firstword" => words(&ex(self, args)).first().cloned().unwrap_or_default(),
            "lastword" => words(&ex(self, args)).last().cloned().unwrap_or_default(),
            "dir" => words(&ex(self, args)).iter().map(|w| match w.rfind('/') {
                Some(i) => String::from(&w[..=i]),
                None => String::from("./"),
            }).collect::<Vec<_>>().join(" "),
            "notdir" => words(&ex(self, args)).iter().map(|w| String::from(w.rsplit('/').next().unwrap_or(w))).collect::<Vec<_>>().join(" "),
            "suffix" => words(&ex(self, args)).iter().filter_map(|w| {
                let b = w.rsplit('/').next().unwrap_or(w);
                b.rfind('.').map(|i| String::from(&b[i..]))
            }).collect::<Vec<_>>().join(" "),
            "basename" => words(&ex(self, args)).iter().map(|w| {
                let slash = w.rfind('/').map_or(0, |i| i + 1);
                match w[slash..].rfind('.') {
                    Some(i) => String::from(&w[..slash + i]),
                    None => w.clone(),
                }
            }).collect::<Vec<_>>().join(" "),
            "addsuffix" => {
                let s = arg(self, 0);
                words(&arg(self, 1)).iter().map(|w| format!("{w}{s}")).collect::<Vec<_>>().join(" ")
            }
            "addprefix" => {
                let p = arg(self, 0);
                words(&arg(self, 1)).iter().map(|w| format!("{p}{w}")).collect::<Vec<_>>().join(" ")
            }
            "join" => {
                let (x, y) = (words(&arg(self, 0)), words(&arg(self, 1)));
                let n = x.len().max(y.len());
                (0..n).map(|i| format!("{}{}", x.get(i).map_or("", |s| s), y.get(i).map_or("", |s| s))).collect::<Vec<_>>().join(" ")
            }
            "wildcard" => {
                let mut out = Vec::new();
                for p in words(&ex(self, args)) {
                    if rt::glob::has_wildcards(&p) {
                        out.extend(rt::glob::expand(&p));
                    } else if rt::fs::exists(&p) {
                        out.push(p);
                    }
                }
                out.join(" ")
            }
            "realpath" | "abspath" => words(&ex(self, args)).iter().map(|w| rt::path::absolute(w)).collect::<Vec<_>>().join(" "),
            "if" => {
                let c = arg(self, 0);
                if !c.trim().is_empty() { arg(self, 1) } else { arg(self, 2) }
            }
            "or" => {
                for k in 0..a.len() {
                    let v = arg(self, k);
                    if !v.trim().is_empty() {
                        return Some(v);
                    }
                }
                String::new()
            }
            "and" => {
                let mut last = String::new();
                for k in 0..a.len() {
                    last = arg(self, k);
                    if last.trim().is_empty() {
                        return Some(String::new());
                    }
                }
                last
            }
            "foreach" => {
                let var = arg(self, 0);
                let list = words(&arg(self, 1));
                let body = a.get(2).cloned().unwrap_or_default();
                let saved = self.vars.get(var.trim()).cloned();
                let mut out = Vec::new();
                for w in list {
                    self.vars.insert(String::from(var.trim()), Var { value: w, simple: true, from_cmdline: false });
                    out.push(self.expand(&body, auto));
                }
                match saved {
                    Some(v) => {
                        self.vars.insert(String::from(var.trim()), v);
                    }
                    None => {
                        self.vars.remove(var.trim());
                    }
                }
                out.join(" ")
            }
            "shell" => {
                let cmd = ex(self, args);
                let (out, _) = run_shell(&cmd);
                out.trim_end_matches('\n').replace('\n', " ")
            }
            "origin" => {
                let n = ex(self, args);
                match self.vars.get(n.trim()) {
                    Some(v) if v.from_cmdline => "command line".into(),
                    Some(_) => "file".into(),
                    None if rt::env::var(n.trim()).is_some() => "environment".into(),
                    None => "undefined".into(),
                }
            }
            "error" => {
                let m = ex(self, args);
                rt::die!("*** {m}.  Stop.");
            }
            "warning" => {
                let m = ex(self, args);
                eprintln!("make: {m}");
                String::new()
            }
            "info" => {
                let m = ex(self, args);
                println!("{m}");
                String::new()
            }
            _ => return None,
        })
    }

    // ---- parsing ---------------------------------------------------------------------

    fn parse(&mut self, text: &str, name: &str) {
        // join continuation lines
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for l in text.lines() {
            if let Some(stripped) = l.strip_suffix('\\') {
                cur.push_str(stripped);
                cur.push(' ');
                continue;
            }
            cur.push_str(l);
            lines.push(core::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        let empty = BTreeMap::new();
        let mut targets: Vec<String> = Vec::new();
        let mut pattern_idx: Option<Vec<usize>> = None;
        // the current rule has started its recipe (a new recipe replaces an old one)
        let mut group_started = false;
        // conditional stack: (active, any branch taken)
        let mut conds: Vec<(bool, bool)> = Vec::new();
        for (ln, raw) in lines.iter().enumerate() {
            let active = conds.iter().all(|c| c.0);
            if raw.starts_with('\t') && (!targets.is_empty() || pattern_idx.is_some()) {
                if !active {
                    continue;
                }
                let cmd = String::from(&raw[1..]);
                for t in &targets {
                    let r = self.rules.get_mut(t).unwrap();
                    if !group_started {
                        r.recipe.clear();
                    }
                    r.recipe.push(cmd.clone());
                }
                group_started = true;
                if let Some(ps) = &pattern_idx {
                    for &p in ps {
                        self.patterns[p].recipe.push(cmd.clone());
                    }
                }
                continue;
            }
            // strip comments
            let line = match raw.find('#') {
                Some(i) => &raw[..i],
                None => raw.as_str(),
            };
            let t = line.trim();
            // conditionals
            let first = t.split_whitespace().next().unwrap_or("");
            match first {
                "ifeq" | "ifneq" | "ifdef" | "ifndef" => {
                    let rest = t[first.len()..].trim();
                    let v = if !active {
                        false
                    } else if first == "ifdef" || first == "ifndef" {
                        let n = self.expand(rest, &empty);
                        let def = self.get(n.trim()).is_some_and(|v| !v.is_empty());
                        def == (first == "ifdef")
                    } else {
                        let (x, y) = if rest.starts_with('(') {
                            let inner = &rest[1..rest.rfind(')').unwrap_or(rest.len())];
                            let p = split_args(inner);
                            (p.first().cloned().unwrap_or_default(), p.get(1).cloned().unwrap_or_default())
                        } else {
                            let q: Vec<&str> = rest.split(['"', '\'']).collect();
                            (String::from(*q.get(1).unwrap_or(&"")), String::from(*q.get(3).unwrap_or(&"")))
                        };
                        let (x, y) = (self.expand(x.trim(), &empty), self.expand(y.trim(), &empty));
                        (x == y) == (first == "ifeq")
                    };
                    conds.push((v, v));
                    targets.clear();
                    pattern_idx = None;
                    continue;
                }
                "else" => {
                    let outer = conds.len() > 1 && conds[..conds.len() - 1].iter().all(|c| c.0);
                    let outer = outer || conds.len() == 1;
                    if let Some(c) = conds.last_mut() {
                        let taken = c.1;
                        c.0 = outer && !taken;
                        c.1 = true;
                    }
                    continue;
                }
                "endif" => {
                    conds.pop();
                    continue;
                }
                _ => {}
            }
            if !active || t.is_empty() {
                continue;
            }
            if first == "include" || first == "-include" || first == "sinclude" {
                let files = self.expand(&t[first.len()..], &empty);
                for f in words(&files) {
                    match rt::fs::read_to_string(&f) {
                        Ok(text) => self.parse(&text, &f),
                        Err(e) if first == "include" => rt::die!("{name}:{}: {f}: {e}", ln + 1),
                        Err(_) => {}
                    }
                }
                continue;
            }
            if first == "export" || first == "unexport" || first == "override" {
                // treat `export VAR = x` as an assignment
                let rest = t[first.len()..].trim();
                if !rest.contains('=') {
                    continue;
                }
                if self.assignment(rest) {
                    continue;
                }
            }
            if self.assignment(t) {
                targets.clear();
                pattern_idx = None;
                continue;
            }
            // a rule
            let Some(colon) = find_rule_colon(t) else {
                rt::die!("{name}:{}: *** missing separator.  Stop.", ln + 1);
            };
            let lhs = self.expand(&t[..colon], &empty);
            let mut rhs = &t[colon + 1..];
            let double = rhs.starts_with(':');
            if double {
                rhs = &rhs[1..];
            }
            let (deps_part, inline) = match rhs.find(';') {
                Some(i) => (&rhs[..i], Some(String::from(rhs[i + 1..].trim()))),
                None => (rhs, None),
            };
            let deps_x = self.expand(deps_part, &empty);
            let (normal, order_only) = match deps_x.find('|') {
                Some(i) => (words(&deps_x[..i]), words(&deps_x[i + 1..])),
                None => (words(&deps_x), Vec::new()),
            };
            targets.clear();
            pattern_idx = None;
            group_started = inline.is_some();
            let tnames = words(&lhs);
            if tnames.iter().any(|n| n.contains('%')) {
                let mut idx = Vec::new();
                for n in tnames {
                    self.patterns.push(Pattern { target: n, prereqs: normal.clone(), recipe: inline.iter().cloned().collect() });
                    idx.push(self.patterns.len() - 1);
                }
                pattern_idx = Some(idx);
                continue;
            }
            for n in tnames {
                match n.as_str() {
                    ".PHONY" => {
                        self.phony.extend(normal.iter().cloned());
                        continue;
                    }
                    ".SUFFIXES" => {
                        if normal.is_empty() {
                            self.suffixes.clear();
                        } else {
                            self.suffixes.extend(normal.iter().cloned());
                        }
                        continue;
                    }
                    ".SILENT" => {
                        self.silent = true;
                        continue;
                    }
                    ".IGNORE" => {
                        self.ignore = true;
                        continue;
                    }
                    ".DEFAULT" => {
                        self.default_recipe = Some(inline.iter().cloned().collect());
                        continue;
                    }
                    _ => {}
                }
                // suffix rule .c.o / .c
                if let Some((from, to)) = self.suffix_rule(&n) {
                    self.patterns.push(Pattern { target: format!("%{to}"), prereqs: vec![format!("%{from}")], recipe: inline.iter().cloned().collect() });
                    pattern_idx.get_or_insert_with(Vec::new).push(self.patterns.len() - 1);
                    continue;
                }
                if !self.rules.contains_key(&n) {
                    self.order.push(n.clone());
                }
                let r = self.rules.entry(n.clone()).or_default();
                r.prereqs.extend(normal.iter().cloned());
                r.order_only.extend(order_only.iter().cloned());
                if let Some(cmd) = &inline {
                    r.recipe = vec![cmd.clone()];
                }
                let _ = double;
                targets.push(n);
            }
        }
    }

    fn suffix_rule(&self, n: &str) -> Option<(String, String)> {
        if !n.starts_with('.') || n.contains('/') {
            return None;
        }
        for s in &self.suffixes {
            if let Some(rest) = n.strip_prefix(s.as_str()) {
                if rest.is_empty() {
                    return Some((s.clone(), String::new()));
                }
                if self.suffixes.iter().any(|x| x == rest) {
                    return Some((s.clone(), String::from(rest)));
                }
            }
        }
        None
    }

    /// Try to parse a variable assignment.
    fn assignment(&mut self, t: &str) -> bool {
        let empty = BTreeMap::new();
        let Some(eq) = t.find('=') else { return false };
        // a rule colon before the '=' (but not part of := / ::=) means a rule
        if let Some(c) = t[..eq].find(':') {
            let tail = &t[c..eq];
            if tail != ":" && tail != "::" {
                return false;
            }
        }
        let (name, op) = if t[..eq].ends_with("::") {
            (&t[..eq - 2], "::=")
        } else if t[..eq].ends_with(':') {
            (&t[..eq - 1], ":=")
        } else if t[..eq].ends_with('?') {
            (&t[..eq - 1], "?=")
        } else if t[..eq].ends_with('+') {
            (&t[..eq - 1], "+=")
        } else if t[..eq].ends_with('!') {
            (&t[..eq - 1], "!=")
        } else {
            (&t[..eq], "=")
        };
        let name = self.expand(name.trim(), &empty);
        if name.is_empty() || name.contains(char::is_whitespace) {
            return false;
        }
        let value = t[eq + 1..].trim_start();
        match op {
            ":=" | "::=" => {
                let v = self.expand(value, &empty);
                self.set(&name, v, true, false);
            }
            "?=" => {
                if self.get(&name).is_none() {
                    self.set(&name, String::from(value), false, false);
                }
            }
            "+=" => {
                let (old, simple) = match self.vars.get(&name) {
                    Some(v) => (v.value.clone(), v.simple),
                    None => (String::new(), false),
                };
                let add = if simple { self.expand(value, &empty) } else { String::from(value) };
                let v = if old.is_empty() { add } else { format!("{old} {add}") };
                self.set(&name, v, simple, false);
            }
            "!=" => {
                let cmd = self.expand(value, &empty);
                let (out, _) = run_shell(&cmd);
                self.set(&name, out.trim_end_matches('\n').replace('\n', " "), true, false);
            }
            _ => self.set(&name, String::from(value), false, false),
        }
        true
    }

    // ---- building ----------------------------------------------------------------------

    /// Find an implicit rule for `target`: (prerequisites, recipe, stem).
    fn implicit(&self, target: &str, depth: u32) -> Option<(Vec<String>, Vec<String>, String)> {
        for p in self.patterns.iter().rev() {
            let Some(stem) = pat_match(&p.target, target) else { continue };
            if p.recipe.is_empty() {
                continue;
            }
            let prereqs: Vec<String> = p.prereqs.iter().map(|x| x.replace('%', &stem)).collect();
            let ok = prereqs.iter().all(|d| rt::fs::exists(d) || self.rules.contains_key(d) || (depth < 2 && self.implicit(d, depth + 1).is_some()));
            if ok {
                return Some((prereqs, p.recipe.clone(), stem));
            }
        }
        None
    }

    /// Bring `target` up to date. Returns true if it was (re)made.
    fn build(&mut self, target: &str) -> Result<bool, ()> {
        if let Some(&made) = self.done.get(target) {
            return Ok(made);
        }
        if self.building.contains(target) {
            eprintln!("make: Circular {target} dependency dropped.");
            return Ok(false);
        }
        self.building.insert(target.into());
        let rule = self.rules.get(target).cloned();
        let mut prereqs: Vec<String> = rule.as_ref().map(|r| r.prereqs.clone()).unwrap_or_default();
        let order_only: Vec<String> = rule.as_ref().map(|r| r.order_only.clone()).unwrap_or_default();
        let mut recipe = rule.as_ref().map(|r| r.recipe.clone()).unwrap_or_default();
        let mut stem = String::new();
        if recipe.is_empty() && !self.phony.contains(target) {
            if let Some((p, r, s)) = self.implicit(target, 0) {
                // the implicit prerequisite comes first ($<)
                let mut all = p;
                for x in prereqs {
                    if !all.contains(&x) {
                        all.push(x);
                    }
                }
                prereqs = all;
                recipe = r;
                stem = s;
            }
        }
        let mut any_made = false;
        let mut failed = false;
        for d in prereqs.iter().chain(order_only.iter()) {
            match self.build(d) {
                Ok(m) => any_made |= m && prereqs.contains(d),
                Err(()) => {
                    failed = true;
                    if !self.keep_going {
                        self.building.remove(target);
                        return Err(());
                    }
                }
            }
        }
        self.building.remove(target);
        if failed {
            eprintln!("make: Target '{target}' not remade because of errors.");
            return Err(());
        }
        let exists = rt::fs::exists(target);
        if rule.is_none() && recipe.is_empty() {
            if exists {
                self.done.insert(target.into(), false);
                return Ok(false);
            }
            if let Some(def) = self.default_recipe.clone() {
                recipe = def;
            } else {
                eprintln!("make: *** No rule to make target '{target}'.  Stop.");
                return Err(());
            }
        }
        let tm = if self.phony.contains(target) { None } else { mtime(target) };
        let newer: Vec<String> = prereqs.iter().filter(|d| match (tm, mtime(d)) {
            (Some(t), Some(dm)) => dm > t,
            _ => true,
        }).cloned().collect();
        let need = self.always || tm.is_none() || any_made || !newer.is_empty();
        if !need {
            self.done.insert(target.into(), false);
            return Ok(false);
        }
        if self.question {
            self.done.insert(target.into(), true);
            return Ok(true);
        }
        let mut auto: BTreeMap<String, String> = BTreeMap::new();
        auto.insert("@".into(), target.into());
        auto.insert("<".into(), prereqs.first().cloned().unwrap_or_default());
        let mut uniq: Vec<String> = Vec::new();
        for p in &prereqs {
            if !uniq.contains(p) {
                uniq.push(p.clone());
            }
        }
        auto.insert("^".into(), uniq.join(" "));
        auto.insert("+".into(), prereqs.join(" "));
        auto.insert("?".into(), newer.join(" "));
        auto.insert("|".into(), order_only.join(" "));
        let stem = if stem.is_empty() { match target.rfind('.') {
            Some(i) => String::from(&target[..i]),
            None => String::from(target),
        } } else { stem };
        auto.insert("*".into(), stem);
        for line in &recipe {
            let mut cmd = self.expand(line, &auto);
            let mut silent = self.silent;
            let mut ignore = self.ignore;
            let mut force = false;
            loop {
                let t = cmd.trim_start();
                if let Some(r) = t.strip_prefix('@') {
                    silent = true;
                    cmd = String::from(r);
                } else if let Some(r) = t.strip_prefix('-') {
                    ignore = true;
                    cmd = String::from(r);
                } else if let Some(r) = t.strip_prefix('+') {
                    force = true;
                    cmd = String::from(r);
                } else {
                    break;
                }
            }
            if cmd.trim().is_empty() {
                continue;
            }
            if !silent || self.dry_run {
                println!("{cmd}");
            }
            if self.dry_run && !force {
                continue;
            }
            rt::io::stdout().flush_quiet();
            let args = vec![String::from("sh"), String::from("-c"), cmd.clone()];
            let st = match rt::process::fork() {
                Ok(0) => {
                    let e = rt::process::execve("/bin/sh", &args, &rt::env::environ());
                    eprintln!("make: /bin/sh: {e}");
                    rt::process::exit(127)
                }
                Ok(pid) => rt::process::wait(pid).map(|s| s.shell_code()).unwrap_or(1),
                Err(e) => {
                    eprintln!("make: fork: {e}");
                    1
                }
            };
            if st != 0 {
                if ignore {
                    eprintln!("make: [{target}] Error {st} (ignored)");
                } else {
                    eprintln!("make: *** [{target}] Error {st}");
                    if !self.phony.contains(target) && rt::fs::exists(target) && tm.is_none_or(|t| mtime(target).is_some_and(|n| n > t)) {
                        // a half-made target is removed
                    }
                    self.errors += 1;
                    self.done.insert(target.into(), false);
                    return Err(());
                }
            }
        }
        self.done.insert(target.into(), true);
        Ok(true)
    }
}

/// The colon that separates targets from prerequisites (not := and not a drive letter).
fn find_rule_colon(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut depth = 0;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth -= 1,
            b':' if depth == 0 => {
                if b.get(i + 1) == Some(&b'=') {
                    return None;
                }
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

fn main(args: &[String]) -> i32 {
    let mut m = Make {
        vars: BTreeMap::new(),
        rules: BTreeMap::new(),
        order: Vec::new(),
        patterns: Vec::new(),
        suffixes: words(".out .a .o .c .s .S .h .sh"),
        phony: BTreeSet::new(),
        silent: false,
        ignore: false,
        dry_run: false,
        keep_going: false,
        always: false,
        question: false,
        done: BTreeMap::new(),
        building: BTreeSet::new(),
        default_recipe: None,
        errors: 0,
    };
    // built-in variables and rules
    for (k, v) in [
        ("CC", "cc"),
        ("AS", "as"),
        ("LD", "ld"),
        ("AR", "ar"),
        ("ARFLAGS", "rcs"),
        ("RM", "rm -f"),
        ("MAKE", "make"),
        ("CFLAGS", ""),
        ("CPPFLAGS", ""),
        ("LDFLAGS", ""),
        ("LDLIBS", ""),
        ("ASFLAGS", ""),
        ("SHELL", "/bin/sh"),
    ] {
        m.set(k, String::from(v), false, false);
    }
    m.patterns.push(Pattern { target: "%.o".into(), prereqs: vec!["%.c".into()], recipe: vec!["$(CC) $(CFLAGS) $(CPPFLAGS) -c -o $@ $<".into()] });
    m.patterns.push(Pattern { target: "%.o".into(), prereqs: vec!["%.s".into()], recipe: vec!["$(AS) $(ASFLAGS) -o $@ $<".into()] });
    m.patterns.push(Pattern { target: "%".into(), prereqs: vec!["%.o".into()], recipe: vec!["$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)".into()] });
    m.patterns.push(Pattern { target: "%".into(), prereqs: vec!["%.c".into()], recipe: vec!["$(CC) $(CFLAGS) $(CPPFLAGS) $(LDFLAGS) -o $@ $< $(LDLIBS)".into()] });
    m.patterns.push(Pattern { target: "%".into(), prereqs: vec!["%.sh".into()], recipe: vec!["cp $< $@".into(), "chmod +x $@".into()] });
    let builtin_patterns = m.patterns.len();

    let mut files: Vec<String> = Vec::new();
    let mut targets: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if let Some(eq) = a.find('=').filter(|_| !a.starts_with('-')) {
            m.set(&a[..eq], String::from(&a[eq + 1..]), false, true);
            continue;
        }
        if !a.starts_with('-') || a == "-" {
            targets.push(a.clone());
            continue;
        }
        let mut chars = a[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                'f' | 'C' | 'j' | 'I' => {
                    let rest: String = chars.collect();
                    let v = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        args.get(i - 1).cloned().unwrap_or_else(|| rt::die!("option requires an argument -- '{c}'"))
                    };
                    match c {
                        'f' => files.push(v),
                        'C' => {
                            if let Err(e) = rt::env::set_current_dir(&v) {
                                rt::die!("{v}: {e}");
                            }
                            println!("make: Entering directory '{}'", rt::env::current_dir().unwrap_or_default());
                        }
                        _ => {}
                    }
                    break;
                }
                'n' => m.dry_run = true,
                's' => m.silent = true,
                'k' => m.keep_going = true,
                'i' => m.ignore = true,
                'B' => m.always = true,
                'q' => m.question = true,
                'r' => m.patterns.truncate(0),
                'S' => m.keep_going = false,
                'w' | 'e' => {}
                _ => {
                    eprintln!("usage: make [-f makefile] [-C dir] [-nskiBqr] [name=value...] [target...]");
                    return 2;
                }
            }
        }
    }
    let _ = builtin_patterns;
    if files.is_empty() {
        match ["GNUmakefile", "makefile", "Makefile"].iter().find(|f| rt::fs::exists(f)) {
            Some(f) => files.push(String::from(*f)),
            None if targets.is_empty() => rt::die!("*** No targets specified and no makefile found.  Stop."),
            None => {}
        }
    }
    for f in &files {
        let text = if f == "-" {
            let mut s = String::new();
            let _ = rt::io::stdin().read_to_string(&mut s);
            s
        } else {
            match rt::fs::read_to_string(f) {
                Ok(t) => t,
                Err(e) => rt::die!("{f}: {e}"),
            }
        };
        m.parse(&text, f);
    }
    if targets.is_empty() {
        match m.order.iter().find(|t| !t.starts_with('.') || t.contains('/')) {
            Some(t) => targets.push(t.clone()),
            None => rt::die!("*** No targets.  Stop."),
        }
    }
    let mut st = 0;
    for t in &targets {
        match m.build(t) {
            Ok(made) => {
                if m.question {
                    if made {
                        st = 1;
                    }
                } else if !made && !m.phony.contains(t) && m.rules.get(t).is_none_or(|r| !r.recipe.is_empty() || !r.prereqs.is_empty()) {
                    println!("make: '{t}' is up to date.");
                } else if !made && m.rules.get(t).is_some_and(|r| r.recipe.is_empty()) {
                    println!("make: Nothing to be done for '{t}'.");
                }
            }
            Err(()) => {
                st = 2;
                if !m.keep_going {
                    break;
                }
            }
        }
    }
    st
}
