//! The text buffer: lines of characters, editing primitives and undo/redo.

use crate::syntax::{self, Lang};
use rt::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Pos {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
enum Op {
    /// `text` was inserted at `at`, ending at `end`
    Ins { at: Pos, end: Pos, text: String },
    /// `text` was deleted from `at`
    Del { at: Pos, text: String },
}

pub struct Buffer {
    pub lines: Vec<Vec<char>>,
    pub cur: Pos,
    pub name: String,
    pub modified: bool,
    pub lang: Lang,
    pub load_msg: Option<String>,
    undo: Vec<Vec<Op>>,
    redo: Vec<Vec<Op>>,
    /// nesting of begin_group / end_group
    group: u32,
    /// the last undo group is a run of typed characters that may grow
    typing: bool,
    /// mode of the file when it was loaded
    mode: Option<u32>,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            lines: alloc::vec![Vec::new()],
            cur: Pos::default(),
            name: String::new(),
            modified: false,
            lang: Lang::None,
            load_msg: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: 0,
            typing: false,
            mode: None,
        }
    }

    pub fn load(path: &str) -> Buffer {
        let mut b = Buffer::new();
        b.name = path.into();
        match rt::fs::read(path) {
            Ok(data) => {
                let text = String::from_utf8_lossy(&data);
                let mut lines: Vec<Vec<char>> = text.split('\n').map(|l| l.trim_end_matches('\r').chars().collect()).collect();
                if lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
                    lines.pop();
                }
                b.lines = lines;
                b.mode = rt::fs::metadata(path).ok().map(|m| m.mode() & 0o7777);
                if rt::fs::access(path, azsys::flags::W_OK).is_err() {
                    b.load_msg = Some(format!("{path} is not writable"));
                }
            }
            Err(e) if e.0 == azsys::errno::ENOENT => b.load_msg = Some("New File".into()),
            Err(e) => b.load_msg = Some(format!("Error reading {path}: {e}")),
        }
        b.lang = syntax::detect(path, b.lines.first());
        b
    }

    pub fn save(&mut self, path: &str) -> rt::Result<usize> {
        let mut text = String::new();
        for l in &self.lines {
            text.extend(l.iter());
            text.push('\n');
        }
        // an empty buffer writes an empty file
        if self.lines.len() == 1 && self.lines[0].is_empty() {
            text.clear();
        }
        let mode = self.mode.unwrap_or(0o644);
        let mut f = rt::fs::File::open_with(path, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, mode)?;
        f.write_all(text.as_bytes())?;
        self.modified = false;
        Ok(self.lines.len())
    }

    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        let mut s = String::new();
        if a.row == b.row {
            s.extend(&self.lines[a.row][a.col..b.col]);
            return s;
        }
        s.extend(&self.lines[a.row][a.col..]);
        for r in a.row + 1..b.row {
            s.push('\n');
            s.extend(&self.lines[r]);
        }
        s.push('\n');
        s.extend(&self.lines[b.row][..b.col]);
        s
    }

    fn raw_insert(&mut self, at: Pos, text: &str) -> Pos {
        let tail: Vec<char> = self.lines[at.row].split_off(at.col);
        let mut row = at.row;
        let mut parts = text.split('\n');
        self.lines[row].extend(parts.next().unwrap_or("").chars());
        for p in parts {
            row += 1;
            self.lines.insert(row, p.chars().collect());
        }
        let col = self.lines[row].len();
        self.lines[row].extend(tail);
        Pos { row, col }
    }

    fn raw_delete(&mut self, a: Pos, b: Pos) -> String {
        let text = self.text_range(a, b);
        if a.row == b.row {
            self.lines[a.row].drain(a.col..b.col);
        } else {
            let tail: Vec<char> = self.lines[b.row][b.col..].to_vec();
            self.lines[a.row].truncate(a.col);
            self.lines[a.row].extend(tail);
            self.lines.drain(a.row + 1..=b.row);
        }
        text
    }

    fn record(&mut self, op: Op) {
        self.modified = true;
        self.redo.clear();
        if self.group > 0 {
            if let Some(g) = self.undo.last_mut() {
                g.push(op);
                return;
            }
        }
        self.undo.push(alloc::vec![op]);
        if self.undo.len() > 1000 {
            self.undo.remove(0);
        }
    }

    pub fn begin_group(&mut self) {
        if self.group == 0 {
            self.undo.push(Vec::new());
            self.typing = false;
        }
        self.group += 1;
    }

    pub fn end_group(&mut self) {
        self.group = self.group.saturating_sub(1);
        if self.group == 0 && self.undo.last().is_some_and(|g| g.is_empty()) {
            self.undo.pop();
        }
    }

    pub fn insert(&mut self, at: Pos, text: &str) -> Pos {
        self.typing = false;
        let end = self.raw_insert(at, text);
        self.record(Op::Ins { at, end, text: text.into() });
        end
    }

    /// Insert one typed character; runs of typing undo together.
    pub fn insert_char(&mut self, at: Pos, c: char) -> Pos {
        let end = self.raw_insert(at, &String::from(c));
        if self.typing && self.group == 0 && c != ' ' {
            if let Some(Op::Ins { end: e, text, .. }) = self.undo.last_mut().and_then(|g| g.last_mut()) {
                if *e == at {
                    *e = end;
                    text.push(c);
                    self.modified = true;
                    self.redo.clear();
                    return end;
                }
            }
        }
        self.record(Op::Ins { at, end, text: String::from(c) });
        self.typing = self.group == 0;
        end
    }

    pub fn delete(&mut self, a: Pos, b: Pos) -> String {
        self.typing = false;
        let text = self.raw_delete(a, b);
        self.record(Op::Del { at: a, text: text.clone() });
        text
    }

    pub fn undo(&mut self) -> Option<Pos> {
        self.typing = false;
        let g = self.undo.pop()?;
        let mut pos = self.cur;
        for op in g.iter().rev() {
            match op {
                Op::Ins { at, end, .. } => {
                    self.raw_delete(*at, *end);
                    pos = *at;
                }
                Op::Del { at, text } => {
                    pos = self.raw_insert(*at, text);
                }
            }
        }
        self.redo.push(g);
        self.modified = true;
        Some(pos)
    }

    pub fn redo(&mut self) -> Option<Pos> {
        self.typing = false;
        let g = self.redo.pop()?;
        let mut pos = self.cur;
        for op in g.iter() {
            match op {
                Op::Ins { at, text, .. } => pos = self.raw_insert(*at, text),
                Op::Del { at, text } => {
                    let end = self.end_of(*at, text);
                    self.raw_delete(*at, end);
                    pos = *at;
                }
            }
        }
        self.undo.push(g);
        self.modified = true;
        Some(pos)
    }

    /// Where `text` inserted at `at` would end.
    fn end_of(&self, at: Pos, text: &str) -> Pos {
        let n = text.split('\n').count();
        if n == 1 {
            Pos { row: at.row, col: at.col + text.chars().count() }
        } else {
            Pos { row: at.row + n - 1, col: text.rsplit('\n').next().unwrap_or("").chars().count() }
        }
    }

    fn matches_at(&self, p: Pos, pat: &[char], icase: bool) -> bool {
        let l = &self.lines[p.row];
        if p.col + pat.len() > l.len() {
            return false;
        }
        pat.iter().enumerate().all(|(i, c)| {
            let x = l[p.col + i];
            x == *c || (icase && x.to_lowercase().eq(c.to_lowercase()))
        })
    }

    /// Search from the cursor (skipping a match at the cursor itself).
    pub fn find(&self, pat: &[char], backward: bool, icase: bool) -> Option<(Pos, bool)> {
        self.find_from(pat, self.cur, backward, icase, true)
    }

    pub fn find_from(&self, pat: &[char], from: Pos, backward: bool, icase: bool, skip_here: bool) -> Option<(Pos, bool)> {
        if pat.is_empty() {
            return None;
        }
        let n = self.lines.len();
        if !backward {
            let mut col = from.col + skip_here as usize;
            for k in 0..=n {
                let r = (from.row + k) % n;
                let start = if k == 0 { col } else { 0 };
                let l = &self.lines[r];
                let mut c = start;
                while c + pat.len() <= l.len() {
                    let p = Pos { row: r, col: c };
                    if self.matches_at(p, pat, icase) {
                        let wrapped = k > 0 && r <= from.row && (k == n || r < from.row || c < from.col);
                        return Some((p, wrapped || (k == n)));
                    }
                    c += 1;
                }
                col = 0;
            }
        } else {
            for k in 0..=n {
                let r = (from.row + n - k % n) % n;
                let l = &self.lines[r];
                let max = if k == 0 {
                    if from.col == 0 {
                        continue;
                    }
                    from.col - 1
                } else {
                    l.len()
                };
                let mut c = max.min(l.len());
                loop {
                    let p = Pos { row: r, col: c };
                    if self.matches_at(p, pat, icase) {
                        return Some((p, k > 0 && r >= from.row));
                    }
                    if c == 0 {
                        break;
                    }
                    c -= 1;
                }
            }
        }
        None
    }
}
