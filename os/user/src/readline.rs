//! An interactive line editor for terminals: cursor movement, history, kill and
//! yank, and tab completion. Falls back to plain line reading when stdin is not a
//! terminal.
//!
//! Keys: ←/→ ^B/^F move, ↑/↓ ^P/^N history, Home/End ^A/^E, ^U/^K kill to start/end,
//! ^W kill word, ^Y yank, ^L redraw, ^C cancel the line, ^D end of input on an
//! empty line, Tab complete.

use crate::io::{self, Write};
use crate::term::{self, RawMode};
use alloc::string::String;
use alloc::vec::Vec;

/// Completion: given the line and the cursor position, return (start of the word
/// being completed, candidates).
pub type Completer<'a> = &'a dyn Fn(&str, usize) -> (usize, Vec<String>);

pub enum Line {
    Text(String),
    /// ^C
    Interrupted,
    /// ^D on an empty line, or end of input
    Eof,
}

pub struct Editor {
    pub history: Vec<String>,
    pub max_history: usize,
    kill: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Visible width of a prompt: skips ANSI escape sequences.
fn visible_len(s: &str) -> usize {
    let mut n = 0;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            if c.is_ascii_alphabetic() {
                esc = false;
            }
        } else if c == '\x1b' {
            esc = true;
        } else if c != '\r' && c != '\n' {
            n += 1;
        }
    }
    n
}

fn read_byte() -> Option<u8> {
    let mut b = [0u8; 1];
    match io::read_fd(io::STDIN, &mut b) {
        Ok(1) => Some(b[0]),
        _ => None,
    }
}

struct State<'p> {
    prompt: &'p str,
    plen: usize,
    buf: Vec<char>,
    pos: usize,
    cols: usize,
    /// cursor position at the last refresh, and the most rows the line has used
    oldpos: usize,
    maxrows: usize,
}

impl State<'_> {
    /// Redraw the line, wrapping over several terminal rows (as linenoise does).
    fn refresh(&mut self) {
        let cols = self.cols.max(1);
        let (plen, len, pos) = (self.plen, self.buf.len(), self.pos);
        let mut rows = (plen + len + cols - 1) / cols;
        let rpos = (plen + self.oldpos + cols) / cols;
        let old_rows = self.maxrows;
        if rows > self.maxrows {
            self.maxrows = rows;
        }
        let mut out = String::new();
        if old_rows > rpos {
            out.push_str(&alloc::format!("\x1b[{}B", old_rows - rpos));
        }
        for _ in 1..old_rows.max(1) {
            out.push_str("\r\x1b[0K\x1b[1A");
        }
        out.push_str("\r\x1b[0K");
        out.push_str(self.prompt);
        out.extend(&self.buf);
        if pos > 0 && pos == len && (pos + plen) % cols == 0 {
            out.push_str("\n\r");
            rows += 1;
            if rows > self.maxrows {
                self.maxrows = rows;
            }
        }
        let rpos2 = (plen + pos + cols) / cols;
        if rows > rpos2 {
            out.push_str(&alloc::format!("\x1b[{}A", rows - rpos2));
        }
        let col = (plen + pos) % cols;
        if col > 0 {
            out.push_str(&alloc::format!("\r\x1b[{col}C"));
        } else {
            out.push('\r');
        }
        self.oldpos = pos;
        let _ = io::write_fd(io::STDOUT, out.as_bytes());
    }

    /// Move the cursor below the last row of the line (before printing a newline).
    fn to_end(&mut self) {
        let cols = self.cols.max(1);
        let rows = (self.plen + self.buf.len() + cols - 1) / cols;
        let rpos = (self.plen + self.oldpos + cols) / cols;
        if rows > rpos {
            let _ = io::write_fd(io::STDOUT, alloc::format!("\x1b[{}B", rows - rpos).as_bytes());
        }
    }

    fn text(&self) -> String {
        self.buf.iter().collect()
    }

    fn set(&mut self, s: &str) {
        self.buf = s.chars().collect();
        self.pos = self.buf.len();
    }

    fn insert(&mut self, s: &str) {
        for c in s.chars() {
            self.buf.insert(self.pos, c);
            self.pos += 1;
        }
    }
}

fn common_prefix(v: &[String]) -> String {
    let Some(first) = v.first() else { return String::new() };
    let mut n = first.len();
    for s in &v[1..] {
        n = n.min(first.bytes().zip(s.bytes()).take_while(|(a, b)| a == b).count());
    }
    while !first.is_char_boundary(n) {
        n -= 1;
    }
    first[..n].into()
}

impl Editor {
    pub fn new() -> Editor {
        Editor { history: Vec::new(), max_history: 500, kill: String::new() }
    }

    pub fn add_history(&mut self, line: &str) {
        let l = line.trim_end();
        if l.is_empty() || self.history.last().is_some_and(|h| h == l) {
            return;
        }
        self.history.push(l.into());
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    /// Read a line, editing it interactively if stdin is a terminal.
    pub fn read_line(&mut self, prompt: &str, complete: Option<Completer>) -> Line {
        io::stdout().flush_quiet();
        let raw = if io::isatty(io::STDIN) { RawMode::enter(io::STDIN).ok() } else { None };
        let Some(_raw) = raw else {
            let _ = io::write_fd(io::STDOUT, prompt.as_bytes());
            return match io::stdin().line() {
                Some(l) => Line::Text(l),
                None => Line::Eof,
            };
        };
        let (_, cols) = term::size(io::STDOUT);
        let mut st = State { prompt, plen: visible_len(prompt), buf: Vec::new(), pos: 0, cols: cols as usize, oldpos: 0, maxrows: 0 };
        let mut hist_idx = self.history.len();
        let mut saved = String::new();
        let mut last_tab = false;
        st.refresh();
        loop {
            let Some(c) = read_byte() else { return Line::Eof };
            let was_tab = last_tab;
            last_tab = false;
            match c {
                b'\r' | b'\n' => {
                    st.to_end();
                    let _ = io::write_fd(io::STDOUT, b"\r\n");
                    return Line::Text(st.text());
                }
                0x03 => {
                    st.pos = st.buf.len();
                    st.refresh();
                    let _ = io::write_fd(io::STDOUT, b"^C\r\n");
                    return Line::Interrupted;
                }
                0x04 => {
                    if st.buf.is_empty() {
                        let _ = io::write_fd(io::STDOUT, b"\r\n");
                        return Line::Eof;
                    }
                    if st.pos < st.buf.len() {
                        st.buf.remove(st.pos);
                    }
                }
                0x7F | 0x08 => {
                    if st.pos > 0 {
                        st.pos -= 1;
                        st.buf.remove(st.pos);
                    }
                }
                0x01 => st.pos = 0,
                0x05 => st.pos = st.buf.len(),
                0x02 => st.pos = st.pos.saturating_sub(1),
                0x06 => st.pos = (st.pos + 1).min(st.buf.len()),
                0x0B => {
                    self.kill = st.buf[st.pos..].iter().collect();
                    st.buf.truncate(st.pos);
                }
                0x15 => {
                    self.kill = st.buf[..st.pos].iter().collect();
                    st.buf.drain(..st.pos);
                    st.pos = 0;
                }
                0x17 => {
                    let mut i = st.pos;
                    while i > 0 && st.buf[i - 1] == ' ' {
                        i -= 1;
                    }
                    while i > 0 && st.buf[i - 1] != ' ' {
                        i -= 1;
                    }
                    self.kill = st.buf[i..st.pos].iter().collect();
                    st.buf.drain(i..st.pos);
                    st.pos = i;
                }
                0x19 => {
                    let k = self.kill.clone();
                    st.insert(&k);
                }
                0x0C => {
                    let _ = io::write_fd(io::STDOUT, b"\x1b[H\x1b[2J");
                    st.oldpos = 0;
                    st.maxrows = 0;
                }
                0x10 | 0x0E => self.history_step(&mut st, &mut hist_idx, &mut saved, c == 0x10),
                b'\t' => {
                    if let Some(f) = complete {
                        last_tab = self.complete(&mut st, f, was_tab);
                    }
                }
                0x1B => {
                    // escape sequences: ESC [ X, ESC [ n ~, ESC O X
                    let Some(b) = read_byte() else { return Line::Eof };
                    if b != b'[' && b != b'O' {
                        continue;
                    }
                    let mut num = 0u32;
                    let mut fin;
                    loop {
                        let Some(x) = read_byte() else { return Line::Eof };
                        fin = x;
                        if x.is_ascii_digit() {
                            num = num * 10 + (x - b'0') as u32;
                        } else if x != b';' {
                            break;
                        }
                    }
                    match (fin, num) {
                        (b'A', _) => self.history_step(&mut st, &mut hist_idx, &mut saved, true),
                        (b'B', _) => self.history_step(&mut st, &mut hist_idx, &mut saved, false),
                        (b'C', _) => st.pos = (st.pos + 1).min(st.buf.len()),
                        (b'D', _) => st.pos = st.pos.saturating_sub(1),
                        (b'H', _) | (b'~', 1) | (b'~', 7) => st.pos = 0,
                        (b'F', _) | (b'~', 4) | (b'~', 8) => st.pos = st.buf.len(),
                        (b'~', 3) => {
                            if st.pos < st.buf.len() {
                                st.buf.remove(st.pos);
                            }
                        }
                        _ => {}
                    }
                }
                c if c >= 0x20 => {
                    // collect a UTF-8 sequence
                    let mut bytes = alloc::vec![c];
                    let need = if c >= 0xF0 { 3 } else if c >= 0xE0 { 2 } else if c >= 0xC0 { 1 } else { 0 };
                    for _ in 0..need {
                        if let Some(b) = read_byte() {
                            bytes.push(b);
                        }
                    }
                    let s = String::from_utf8_lossy(&bytes).into_owned();
                    let at_end = st.pos == st.buf.len();
                    st.insert(&s);
                    // fast path: typing at the end of the line just echoes (the terminal
                    // wraps), unless the cursor would land exactly on a row boundary
                    if at_end && (st.plen + st.buf.len()) % st.cols.max(1) != 0 {
                        st.oldpos = st.pos;
                        let _ = io::write_fd(io::STDOUT, s.as_bytes());
                        continue;
                    }
                }
                _ => {}
            }
            st.refresh();
        }
    }

    fn history_step(&self, st: &mut State, idx: &mut usize, saved: &mut String, up: bool) {
        if up {
            if *idx == 0 {
                return;
            }
            if *idx == self.history.len() {
                *saved = st.text();
            }
            *idx -= 1;
            st.set(&self.history[*idx]);
        } else {
            if *idx >= self.history.len() {
                return;
            }
            *idx += 1;
            if *idx == self.history.len() {
                st.set(&saved.clone());
            } else {
                st.set(&self.history[*idx]);
            }
        }
    }

    /// Returns true if the completion was ambiguous (a second Tab lists candidates).
    fn complete(&self, st: &mut State, f: Completer, list: bool) -> bool {
        let line = st.text();
        let byte_pos: usize = st.buf[..st.pos].iter().map(|c| c.len_utf8()).sum();
        let (start, cands) = f(&line, byte_pos);
        if cands.is_empty() {
            let _ = io::write_fd(io::STDOUT, b"\x07");
            return false;
        }
        let word_chars = line[start..byte_pos].chars().count();
        let prefix = common_prefix(&cands);
        if cands.len() == 1 {
            let mut c = cands[0].clone();
            if !c.ends_with('/') {
                c.push(' ');
            }
            let tail: String = c.chars().skip(word_chars).collect();
            st.insert(&tail);
            return false;
        }
        if prefix.chars().count() > word_chars {
            let tail: String = prefix.chars().skip(word_chars).collect();
            st.insert(&tail);
            return true;
        }
        if list {
            let width = cands.iter().map(|c| c.chars().count()).max().unwrap_or(1) + 2;
            let per = (st.cols / width).max(1);
            let mut out = String::from("\r\n");
            for (i, c) in cands.iter().enumerate() {
                out.push_str(c);
                if (i + 1) % per == 0 || i + 1 == cands.len() {
                    out.push_str("\r\n");
                } else {
                    for _ in c.chars().count()..width {
                        out.push(' ');
                    }
                }
            }
            st.to_end();
            let _ = io::write_fd(io::STDOUT, out.as_bytes());
            st.oldpos = 0;
            st.maxrows = 0;
        } else {
            let _ = io::write_fd(io::STDOUT, b"\x07");
        }
        true
    }
}

/// Complete a file name: `word` is the partial path. Directories get a trailing `/`.
pub fn complete_path(word: &str, dirs_only: bool) -> Vec<String> {
    let (dir, base) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let Ok(entries) = crate::fs::read_dir(if dir.is_empty() { "." } else { dir }) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .into_iter()
        .filter(|e| e.name != "." && e.name != "..")
        .filter(|e| e.name.starts_with(base) && (!e.name.starts_with('.') || base.starts_with('.')))
        .filter_map(|e| {
            let full = alloc::format!("{dir}{}", e.name);
            let is_dir = crate::fs::is_dir(&full);
            if dirs_only && !is_dir {
                return None;
            }
            Some(if is_dir { full + "/" } else { full })
        })
        .collect();
    out.sort();
    out
}

/// Flush stdout (prompts).
pub fn flush() {
    let _ = io::stdout().flush();
}
