//! edit: a small screen editor in the style of nano.
//!
//!     edit [-ilv] [-T tabsize] [+line[,col]] [file...]
//!
//! -i auto-indent, -l line numbers, -v view only. ^G shows the key bindings.

#![no_std]
#![no_main]

extern crate alloc;

mod buffer;
mod syntax;

use buffer::{Buffer, Pos};
use rt::io;
use rt::prelude::*;
use rt::term::RawMode;

rt::main!(main);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Ctrl(u8),
    Meta(char),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PgUp,
    PgDn,
    Del,
    Ins,
    CtrlLeft,
    CtrlRight,
    CtrlHome,
    CtrlEnd,
    Enter,
    Tab,
    Backspace,
    Esc,
    None,
}

fn read_byte() -> Option<u8> {
    let mut b = [0u8; 1];
    match io::read_fd(0, &mut b) {
        Ok(1) => Some(b[0]),
        _ => None,
    }
}

/// Is input waiting? (to tell a lone ESC from an escape sequence)
fn pending() -> bool {
    rt::term::pending(0) > 0
}

fn read_key() -> Option<Key> {
    let b = read_byte()?;
    Some(match b {
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7F | 0x08 => Key::Backspace,
        0x1B => {
            if !pending() {
                let _ = rt::time::sleep_ms(30);
                if !pending() {
                    return Some(Key::Esc);
                }
            }
            let c = read_byte()?;
            if c != b'[' && c != b'O' {
                // Meta + key
                return Some(Key::Meta((c as char).to_ascii_lowercase()));
            }
            let mut params: Vec<u32> = alloc::vec![0];
            let fin;
            loop {
                let x = read_byte()?;
                if x.is_ascii_digit() {
                    let l = params.last_mut().unwrap();
                    *l = *l * 10 + (x - b'0') as u32;
                } else if x == b';' {
                    params.push(0);
                } else {
                    fin = x;
                    break;
                }
            }
            let ctrl = params.get(1).is_some_and(|m| *m == 5);
            match fin {
                b'A' => Key::Up,
                b'B' => Key::Down,
                b'C' if ctrl => Key::CtrlRight,
                b'D' if ctrl => Key::CtrlLeft,
                b'C' => Key::Right,
                b'D' => Key::Left,
                b'H' if ctrl => Key::CtrlHome,
                b'F' if ctrl => Key::CtrlEnd,
                b'H' => Key::Home,
                b'F' => Key::End,
                b'~' => match params[0] {
                    1 | 7 => Key::Home,
                    4 | 8 => Key::End,
                    2 => Key::Ins,
                    3 => Key::Del,
                    5 => Key::PgUp,
                    6 => Key::PgDn,
                    _ => Key::None,
                },
                _ => Key::None,
            }
        }
        c if c < 0x20 => Key::Ctrl(c + 0x40),
        c => {
            let need = if c >= 0xF0 { 3 } else if c >= 0xE0 { 2 } else if c >= 0xC0 { 1 } else { 0 };
            let mut bytes = alloc::vec![c];
            for _ in 0..need {
                bytes.push(read_byte()?);
            }
            Key::Char(String::from_utf8_lossy(&bytes).chars().next().unwrap_or('?'))
        }
    })
}

struct Editor {
    buf: Buffer,
    /// first line and display column shown
    top: usize,
    left: usize,
    rows: usize,
    cols: usize,
    /// the display column the cursor tries to keep when moving vertically
    want_col: usize,
    msg: Option<String>,
    cutbuf: Vec<String>,
    /// the last command was a cut (consecutive cuts accumulate)
    cutting: bool,
    last_search: String,
    last_replace: String,
    tab: usize,
    autoindent: bool,
    numbers: bool,
    view_only: bool,
    mark: Option<Pos>,
    screen: Vec<String>,
    quit: bool,
    files: Vec<String>,
    file_idx: usize,
    /// the cut buffer holds whole lines (cut without a mark)
    cutbuf_lines: bool,
}

const HELP: &str = "\
edit -- key bindings  (^ = Ctrl, M- = Esc or Alt)

  ^G  this help                 ^X  exit (asks to save changes)
  ^O  write the file            ^R  insert another file
  ^W  search forward            M-W / F3  repeat the search
  ^Q  search backward           ^\\  search and replace
  ^K  cut the line / marked text ^U  paste (uncut)
  ^^ or M-A  set/clear the mark M-6  copy the line / marked text
  ^C  show the cursor position  ^_ or M-G  go to a line (and column)
  ^A / Home  start of line      ^E / End  end of line
  ^Y / PgUp  previous page      ^V / PgDn  next page
  M-\\ / ^Home  first line       M-/ / ^End  last line
  ^Left/^Right  word left/right  ^D / Del  delete the character
  ^H / Backspace  delete back   ^J  justify (fill) the paragraph
  M-U  undo                     M-E  redo
  M-N  toggle line numbers      M-I  toggle auto-indent
  ^L  redraw the screen         M-] go to the matching bracket

Prompts: Enter accepts, ^C cancels, Tab completes file names.
Press any key to return.";

impl Editor {
    fn text_rows(&self) -> usize {
        self.rows.saturating_sub(4).max(1)
    }

    fn gutter(&self) -> usize {
        if self.numbers { format!("{}", self.buf.lines.len()).len().max(3) + 1 } else { 0 }
    }

    fn text_cols(&self) -> usize {
        self.cols.saturating_sub(self.gutter()).max(8)
    }

    /// Display column of character index `c` in line `r`.
    fn disp_col(&self, r: usize, c: usize) -> usize {
        let mut col = 0;
        for ch in self.buf.lines[r].iter().take(c) {
            col += if *ch == '\t' { self.tab - col % self.tab } else { 1 };
        }
        col
    }

    /// Character index nearest display column `want` in line `r`.
    fn char_at_col(&self, r: usize, want: usize) -> usize {
        let mut col = 0;
        for (i, ch) in self.buf.lines[r].iter().enumerate() {
            let w = if *ch == '\t' { self.tab - col % self.tab } else { 1 };
            if col + w > want {
                return i;
            }
            col += w;
        }
        self.buf.lines[r].len()
    }

    fn scroll(&mut self) {
        let c = self.buf.cur;
        let h = self.text_rows();
        if c.row < self.top {
            self.top = c.row;
        }
        if c.row >= self.top + h {
            self.top = c.row + 1 - h;
        }
        let dc = self.disp_col(c.row, c.col);
        let w = self.text_cols();
        if dc < self.left {
            self.left = dc.saturating_sub(w / 4);
        }
        if dc >= self.left + w - 1 {
            self.left = dc + 1 + w / 4 - w;
        }
    }

    fn render_line(&self, r: usize, hl: &[syntax::Style]) -> String {
        let line = &self.buf.lines[r];
        let w = self.text_cols();
        let (sel_a, sel_b) = self.selection().unwrap_or((Pos { row: usize::MAX, col: 0 }, Pos { row: usize::MAX, col: 0 }));
        let mut out = String::new();
        if self.numbers {
            out.push_str(&format!("\x1b[2m{:>width$} \x1b[0m", r + 1, width = self.gutter() - 1));
        }
        let mut col = 0;
        let mut cur_style = syntax::Style::Normal;
        let mut shown = 0;
        let mut in_sel = false;
        for (i, ch) in line.iter().enumerate() {
            let p = Pos { row: r, col: i };
            let sel = p >= sel_a && p < sel_b;
            let style = hl.get(i).copied().unwrap_or(syntax::Style::Normal);
            let cells: Vec<char> = if *ch == '\t' {
                alloc::vec![' '; self.tab - col % self.tab]
            } else if (*ch as u32) < 0x20 {
                alloc::vec!['^', char::from_u32(*ch as u32 + 0x40).unwrap_or('?')]
            } else {
                alloc::vec![*ch]
            };
            for cell in cells {
                if col >= self.left && shown < w {
                    if sel != in_sel || style != cur_style {
                        out.push_str("\x1b[0m");
                        if sel {
                            out.push_str("\x1b[7m");
                        }
                        out.push_str(style.code());
                        in_sel = sel;
                        cur_style = style;
                    }
                    out.push(cell);
                    shown += 1;
                }
                col += 1;
            }
            if shown >= w {
                break;
            }
        }
        out.push_str("\x1b[0m");
        // a '$' marks text beyond the right edge
        if col > self.left + w {
            out.pop();
            out.push_str("\x1b[0m");
        }
        out
    }

    fn selection(&self) -> Option<(Pos, Pos)> {
        let m = self.mark?;
        let c = self.buf.cur;
        Some(if m <= c { (m, c) } else { (c, m) })
    }

    fn draw(&mut self) {
        let (r, c) = rt::term::size(1);
        if r as usize != self.rows || c as usize != self.cols {
            self.rows = r as usize;
            self.cols = c as usize;
            self.screen.clear();
        }
        self.scroll();
        let mut lines: Vec<String> = Vec::with_capacity(self.rows);
        // title bar
        let name = if self.buf.name.is_empty() { String::from("New Buffer") } else { self.buf.name.clone() };
        let left = " edit 1.0";
        let right = if self.view_only {
            "View "
        } else if self.buf.modified {
            "Modified "
        } else {
            ""
        };
        let mid_w = self.cols.saturating_sub(left.len() + right.len());
        let pad = mid_w.saturating_sub(name.chars().count()) / 2;
        let mut title = format!("{left}{:pad$}{name}", "");
        while title.chars().count() < self.cols.saturating_sub(right.len()) {
            title.push(' ');
        }
        title.push_str(right);
        let title: String = title.chars().take(self.cols).collect();
        lines.push(format!("\x1b[7m{title}\x1b[0m"));
        // text
        let hl_states = syntax::line_states(&self.buf, self.top);
        let mut state = hl_states;
        for i in 0..self.text_rows() {
            let r = self.top + i;
            if r < self.buf.lines.len() {
                let (styles, next) = syntax::highlight(self.buf.lang, &self.buf.lines[r], state);
                state = next;
                lines.push(self.render_line(r, &styles));
            } else {
                lines.push(String::new());
            }
        }
        // status line
        let status = match self.msg.take() {
            Some(m) => {
                let m: String = m.chars().take(self.cols.saturating_sub(4)).collect();
                let pad = self.cols.saturating_sub(m.chars().count() + 4) / 2;
                format!("{:pad$}\x1b[7m[ {m} ]\x1b[0m", "")
            }
            None => String::new(),
        };
        lines.push(status);
        let shortcuts: [&[(&str, &str)]; 2] = [
            &[("^G", "Help"), ("^O", "Write Out"), ("^W", "Where Is"), ("^K", "Cut"), ("^C", "Location"), ("M-U", "Undo")],
            &[("^X", "Exit"), ("^R", "Read File"), ("^\\", "Replace"), ("^U", "Paste"), ("^_", "Go To Line"), ("M-E", "Redo")],
        ];
        let cell = (self.cols / 6).max(10);
        for row in shortcuts {
            let mut l = String::new();
            let mut used = 0;
            for (k, d) in row {
                if used + cell > self.cols {
                    break;
                }
                let text = format!("{k} {d}");
                l.push_str(&format!("\x1b[7m{k}\x1b[0m {d}"));
                for _ in text.chars().count()..cell {
                    l.push(' ');
                }
                used += cell;
            }
            lines.push(l);
        }
        // write only the rows that changed
        let mut out = String::new();
        if self.screen.len() != lines.len() {
            out.push_str("\x1b[H\x1b[2J");
            self.screen = alloc::vec![String::from("\u{0}"); lines.len()];
        }
        for (i, l) in lines.iter().enumerate() {
            if self.screen[i] != *l {
                out.push_str(&format!("\x1b[{};1H{l}\x1b[K", i + 1));
                self.screen[i] = l.clone();
            }
        }
        let cur = self.buf.cur;
        let y = cur.row - self.top + 2;
        let x = self.disp_col(cur.row, cur.col) - self.left + 1 + self.gutter();
        out.push_str(&format!("\x1b[{y};{x}H"));
        let _ = io::write_fd(1, out.as_bytes());
    }

    fn redraw_all(&mut self) {
        self.screen.clear();
        let _ = io::write_fd(1, b"\x1b[H\x1b[2J");
    }

    /// Read a line of input on the status row. None if cancelled.
    fn prompt(&mut self, label: &str, initial: &str, complete_files: bool) -> Option<String> {
        let mut s: Vec<char> = initial.chars().collect();
        let mut pos = s.len();
        loop {
            let row = self.rows - 2;
            let text: String = s.iter().collect();
            let line = format!("\x1b[7m{label}: {text}");
            let mut l = line.clone();
            let vis = label.chars().count() + 2 + s.len();
            for _ in vis..self.cols {
                l.push(' ');
            }
            l.push_str("\x1b[0m");
            let x = label.chars().count() + 3 + pos;
            let _ = io::write_fd(1, format!("\x1b[{row};1H{l}\x1b[{row};{x}H").as_bytes());
            self.screen[row - 1] = String::from("\u{0}");
            match read_key() {
                None => return None,
                Some(Key::Enter) => return Some(s.into_iter().collect()),
                Some(Key::Ctrl(b'C')) | Some(Key::Esc) => {
                    self.msg = Some("Cancelled".into());
                    return None;
                }
                Some(Key::Backspace) => {
                    if pos > 0 {
                        pos -= 1;
                        s.remove(pos);
                    }
                }
                Some(Key::Del) | Some(Key::Ctrl(b'D')) => {
                    if pos < s.len() {
                        s.remove(pos);
                    }
                }
                Some(Key::Left) => pos = pos.saturating_sub(1),
                Some(Key::Right) => pos = (pos + 1).min(s.len()),
                Some(Key::Home) | Some(Key::Ctrl(b'A')) => pos = 0,
                Some(Key::End) | Some(Key::Ctrl(b'E')) => pos = s.len(),
                Some(Key::Ctrl(b'U')) => {
                    s.clear();
                    pos = 0;
                }
                Some(Key::Tab) if complete_files => {
                    let word: String = s.iter().collect();
                    let c = rt::readline::complete_path(&word, false);
                    if c.len() == 1 {
                        s = c[0].chars().collect();
                        pos = s.len();
                    } else if c.len() > 1 {
                        let mut p = c[0].clone();
                        for x in &c[1..] {
                            while !x.starts_with(&p) {
                                p.pop();
                            }
                        }
                        s = p.chars().collect();
                        pos = s.len();
                    }
                }
                Some(Key::Char(c)) => {
                    s.insert(pos, c);
                    pos += 1;
                }
                _ => {}
            }
        }
    }

    /// Ask a question answered by one key from `keys`; None if cancelled.
    fn ask(&mut self, q: &str, keys: &str) -> Option<char> {
        loop {
            let row = self.rows - 2;
            let mut l = format!("\x1b[7m{q}");
            for _ in q.chars().count()..self.cols {
                l.push(' ');
            }
            l.push_str("\x1b[0m");
            let _ = io::write_fd(1, format!("\x1b[{row};1H{l}\x1b[{row};{}H", q.chars().count() + 1).as_bytes());
            self.screen[row - 1] = String::from("\u{0}");
            match read_key()? {
                Key::Ctrl(b'C') | Key::Esc => return None,
                Key::Char(c) if keys.contains(c.to_ascii_lowercase()) => return Some(c.to_ascii_lowercase()),
                Key::Enter if keys.contains('\n') => return Some('\n'),
                _ => {}
            }
        }
    }

    fn save(&mut self, ask_name: bool) -> bool {
        let name = if ask_name || self.buf.name.is_empty() {
            match self.prompt("File Name to Write", &self.buf.name.clone(), true) {
                Some(n) if !n.is_empty() => n,
                _ => return false,
            }
        } else {
            self.buf.name.clone()
        };
        if name != self.buf.name && rt::fs::exists(&name) && self.ask("File exists, OVERWRITE ? (Y/N)", "yn") != Some('y') {
            return false;
        }
        match self.buf.save(&name) {
            Ok(n) => {
                self.buf.name = name;
                self.buf.lang = syntax::detect(&self.buf.name, self.buf.lines.first());
                self.msg = Some(format!("Wrote {n} line{}", if n == 1 { "" } else { "s" }));
                true
            }
            Err(e) => {
                self.msg = Some(format!("Error writing {name}: {e}"));
                false
            }
        }
    }

    fn exit(&mut self) {
        if self.buf.modified && !self.view_only {
            match self.ask("Save modified buffer?  (Answering \"No\" will DISCARD changes.)  Y/N", "yn") {
                Some('y') => {
                    if !self.save(false) {
                        return;
                    }
                }
                Some('n') => {}
                _ => {
                    self.msg = Some("Cancelled".into());
                    return;
                }
            }
        }
        // next file, if several were given
        if self.file_idx + 1 < self.files.len() {
            self.file_idx += 1;
            let f = self.files[self.file_idx].clone();
            self.open(&f);
            return;
        }
        self.quit = true;
    }

    fn open(&mut self, f: &str) {
        self.buf = Buffer::load(f);
        self.msg = Some(match &self.buf.load_msg {
            Some(m) => m.clone(),
            None => format!("Read {} line{}", self.buf.lines.len(), if self.buf.lines.len() == 1 { "" } else { "s" }),
        });
        self.top = 0;
        self.left = 0;
        self.mark = None;
    }

    fn search(&mut self, backward: bool, ask: bool) -> bool {
        if ask || self.last_search.is_empty() {
            let label = if backward { "Search (backward)" } else { "Search" };
            let label = if self.last_search.is_empty() { String::from(label) } else { format!("{label} [{}]", self.last_search) };
            match self.prompt(&label, "", false) {
                Some(s) if !s.is_empty() => self.last_search = s,
                Some(_) if !self.last_search.is_empty() => {}
                _ => return false,
            }
        }
        let pat: Vec<char> = self.last_search.chars().collect();
        let icase = !pat.iter().any(|c| c.is_uppercase());
        match self.buf.find(&pat, backward, icase) {
            Some((p, wrapped)) => {
                self.buf.cur = p;
                self.want_col = self.disp_col(p.row, p.col);
                if wrapped {
                    self.msg = Some("Search Wrapped".into());
                }
                true
            }
            None => {
                self.msg = Some(format!("\"{}\" not found", self.last_search));
                false
            }
        }
    }

    fn replace(&mut self) {
        let Some(find) = self.prompt("Search (to replace)", &self.last_search.clone(), false) else { return };
        if find.is_empty() {
            return;
        }
        self.last_search = find.clone();
        let Some(rep) = self.prompt("Replace with", &self.last_replace.clone(), false) else { return };
        self.last_replace = rep.clone();
        let pat: Vec<char> = find.chars().collect();
        let icase = !pat.iter().any(|c| c.is_uppercase());
        let start = self.buf.cur;
        let mut count = 0;
        let mut all = false;
        self.buf.begin_group();
        loop {
            let Some((p, _)) = self.buf.find_from(&pat, self.buf.cur, false, icase, count > 0) else { break };
            // stop after wrapping around past the start
            if count > 0 && p == start {
                break;
            }
            self.buf.cur = p;
            self.mark = Some(Pos { row: p.row, col: p.col + pat.len() });
            if !all {
                self.draw();
                match self.ask("Replace this instance? (Y/N/A/^C)", "yna") {
                    Some('y') => {}
                    Some('a') => all = true,
                    Some('n') => {
                        self.buf.cur = Pos { row: p.row, col: p.col + 1 };
                        self.mark = None;
                        if self.buf.cur.col > self.buf.lines[p.row].len() {
                            break;
                        }
                        continue;
                    }
                    _ => {
                        self.mark = None;
                        break;
                    }
                }
            }
            self.mark = None;
            let end = Pos { row: p.row, col: p.col + pat.len() };
            self.buf.delete(p, end);
            self.buf.insert(p, &rep);
            self.buf.cur = Pos { row: p.row, col: p.col + rep.chars().count() };
            count += 1;
            if count > 100_000 {
                break;
            }
        }
        self.buf.end_group();
        self.mark = None;
        self.msg = Some(format!("Replaced {count} occurrence{}", if count == 1 { "" } else { "s" }));
    }

    fn goto_line(&mut self) {
        let Some(s) = self.prompt("Enter line number, column number", "", false) else { return };
        let mut it = s.split([',', ' ']).filter(|x| !x.is_empty());
        let line: usize = match it.next().map(|x| x.parse::<isize>()) {
            Some(Ok(n)) if n < 0 => (self.buf.lines.len() as isize + n + 1).max(1) as usize,
            Some(Ok(n)) => n.max(1) as usize,
            _ => {
                self.msg = Some("Invalid line or column number".into());
                return;
            }
        };
        let col: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(1).max(1);
        let row = (line - 1).min(self.buf.lines.len() - 1);
        self.buf.cur = Pos { row, col: (col - 1).min(self.buf.lines[row].len()) };
        self.want_col = self.disp_col(row, self.buf.cur.col);
        // center the line
        self.top = row.saturating_sub(self.text_rows() / 2);
    }

    fn cut(&mut self) {
        if let Some((a, b)) = self.selection() {
            let text = self.buf.delete(a, b);
            self.cutbuf = text.split('\n').map(String::from).collect();
            self.buf.cur = a;
            self.mark = None;
            self.cutting = false;
            return;
        }
        let row = self.buf.cur.row;
        if !self.cutting {
            self.cutbuf.clear();
        }
        let line: String = self.buf.lines[row].iter().collect();
        if row + 1 < self.buf.lines.len() {
            self.buf.delete(Pos { row, col: 0 }, Pos { row: row + 1, col: 0 });
        } else {
            // the last line: clear it
            let n = self.buf.lines[row].len();
            if n == 0 && row > 0 {
                let plen = self.buf.lines[row - 1].len();
                self.buf.delete(Pos { row: row - 1, col: plen }, Pos { row, col: 0 });
            } else {
                self.buf.delete(Pos { row, col: 0 }, Pos { row, col: n });
            }
        }
        self.cutbuf.push(line);
        self.buf.cur.col = 0;
        self.buf.cur.row = self.buf.cur.row.min(self.buf.lines.len() - 1);
        self.cutting = true;
    }

    fn copy(&mut self) {
        if let Some((a, b)) = self.selection() {
            self.cutbuf = self.buf.text_range(a, b).split('\n').map(String::from).collect();
            self.mark = None;
            self.msg = Some("Copied".into());
            return;
        }
        let row = self.buf.cur.row;
        if !self.cutting {
            self.cutbuf.clear();
        }
        self.cutbuf.push(self.buf.lines[row].iter().collect());
        if row + 1 < self.buf.lines.len() {
            self.buf.cur = Pos { row: row + 1, col: 0 };
        }
        self.cutting = true;
    }

    fn paste(&mut self) {
        if self.cutbuf.is_empty() {
            return;
        }
        // whole lines (from line cuts) go above the cursor line
        let text = if self.cutting_lines() { format!("{}\n", self.cutbuf.join("\n")) } else { self.cutbuf.join("\n") };
        let at = if self.cutting_lines() { Pos { row: self.buf.cur.row, col: 0 } } else { self.buf.cur };
        let end = self.buf.insert(at, &text);
        self.buf.cur = end;
    }

    /// The cut buffer holds whole lines (from ^K without a mark).
    fn cutting_lines(&self) -> bool {
        self.cutbuf_lines
    }

    fn justify(&mut self) {
        // fill the paragraph around the cursor to 72 columns
        let n = self.buf.lines.len();
        let blank = |r: usize, b: &Buffer| b.lines[r].iter().all(|c| c.is_whitespace());
        let mut a = self.buf.cur.row;
        if blank(a, &self.buf) {
            return;
        }
        while a > 0 && !blank(a - 1, &self.buf) {
            a -= 1;
        }
        let mut b = self.buf.cur.row;
        while b + 1 < n && !blank(b + 1, &self.buf) {
            b += 1;
        }
        let indent: String = self.buf.lines[a].iter().take_while(|c| c.is_whitespace()).collect();
        let words: Vec<String> = (a..=b).flat_map(|r| self.buf.lines[r].iter().collect::<String>().split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        let width = 72;
        let mut out: Vec<String> = Vec::new();
        let mut cur = indent.clone();
        for w in words {
            if cur.len() > indent.len() && cur.len() + 1 + w.len() > width {
                out.push(core::mem::replace(&mut cur, indent.clone()));
            }
            if cur.len() > indent.len() {
                cur.push(' ');
            }
            cur.push_str(&w);
        }
        out.push(cur);
        let end = Pos { row: b, col: self.buf.lines[b].len() };
        self.buf.begin_group();
        self.buf.delete(Pos { row: a, col: 0 }, end);
        let e = self.buf.insert(Pos { row: a, col: 0 }, &out.join("\n"));
        self.buf.end_group();
        self.buf.cur = e;
        self.msg = Some("Justified".into());
    }

    fn match_bracket(&mut self) {
        let c = self.buf.cur;
        let Some(&ch) = self.buf.lines[c.row].get(c.col) else { return };
        let (open, close, fwd) = match ch {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => {
                self.msg = Some("Not a bracket".into());
                return;
            }
        };
        let mut depth = 0i32;
        let mut p = c;
        loop {
            let ch = self.buf.lines[p.row].get(p.col).copied();
            if ch == Some(open) {
                depth += if fwd { 1 } else { -1 };
            } else if ch == Some(close) {
                depth += if fwd { -1 } else { 1 };
            }
            if depth == 0 && ch.is_some() {
                self.buf.cur = p;
                return;
            }
            // step
            if fwd {
                if p.col < self.buf.lines[p.row].len() {
                    p.col += 1;
                } else if p.row + 1 < self.buf.lines.len() {
                    p = Pos { row: p.row + 1, col: 0 };
                } else {
                    break;
                }
            } else if p.col > 0 {
                p.col -= 1;
            } else if p.row > 0 {
                p = Pos { row: p.row - 1, col: self.buf.lines[p.row - 1].len() };
            } else {
                break;
            }
        }
        self.msg = Some("No matching bracket".into());
    }

    fn help(&mut self) {
        let mut out = String::from("\x1b[H\x1b[2J");
        for l in HELP.lines() {
            out.push_str(l);
            out.push_str("\r\n");
        }
        let _ = io::write_fd(1, out.as_bytes());
        let _ = read_key();
        self.redraw_all();
    }

    fn word_left(&mut self) {
        let mut p = self.buf.cur;
        if p.col == 0 {
            if p.row > 0 {
                p = Pos { row: p.row - 1, col: self.buf.lines[p.row - 1].len() };
            }
            self.buf.cur = p;
            return;
        }
        let l = &self.buf.lines[p.row];
        while p.col > 0 && !l[p.col - 1].is_alphanumeric() {
            p.col -= 1;
        }
        while p.col > 0 && l[p.col - 1].is_alphanumeric() {
            p.col -= 1;
        }
        self.buf.cur = p;
    }

    fn word_right(&mut self) {
        let mut p = self.buf.cur;
        let l = &self.buf.lines[p.row];
        if p.col >= l.len() {
            if p.row + 1 < self.buf.lines.len() {
                p = Pos { row: p.row + 1, col: 0 };
            }
            self.buf.cur = p;
            return;
        }
        while p.col < l.len() && l[p.col].is_alphanumeric() {
            p.col += 1;
        }
        while p.col < l.len() && !l[p.col].is_alphanumeric() {
            p.col += 1;
        }
        self.buf.cur = p;
    }

    fn edit_allowed(&mut self) -> bool {
        if self.view_only {
            self.msg = Some("Key is invalid in view mode".into());
            false
        } else {
            true
        }
    }

    fn handle(&mut self, k: Key) {
        let was_cutting = self.cutting;
        let is_cut_key = matches!(k, Key::Ctrl(b'K') | Key::Meta('6'));
        let edits = matches!(k, Key::Char(_) | Key::Enter | Key::Tab | Key::Backspace | Key::Del | Key::Ctrl(b'K') | Key::Ctrl(b'U') | Key::Ctrl(b'D') | Key::Ctrl(b'J') | Key::Ctrl(b'R') | Key::Ctrl(b'\\') | Key::Meta('u') | Key::Meta('e'));
        if edits && !self.edit_allowed() {
            return;
        }
        let rows = self.buf.lines.len();
        let mut vertical = false;
        match k {
            Key::Char(c) => {
                let p = self.buf.cur;
                self.buf.cur = self.buf.insert_char(p, c);
            }
            Key::Tab => {
                let p = self.buf.cur;
                self.buf.cur = self.buf.insert_char(p, '\t');
            }
            Key::Enter => {
                let p = self.buf.cur;
                let indent: String = if self.autoindent { self.buf.lines[p.row].iter().take(p.col).take_while(|c| **c == ' ' || **c == '\t').collect() } else { String::new() };
                self.buf.cur = self.buf.insert(p, &format!("\n{indent}"));
            }
            Key::Backspace => {
                if let Some((a, b)) = self.selection() {
                    self.buf.delete(a, b);
                    self.buf.cur = a;
                    self.mark = None;
                } else {
                    let p = self.buf.cur;
                    let prev = if p.col > 0 {
                        Pos { row: p.row, col: p.col - 1 }
                    } else if p.row > 0 {
                        Pos { row: p.row - 1, col: self.buf.lines[p.row - 1].len() }
                    } else {
                        p
                    };
                    if prev != p {
                        self.buf.delete(prev, p);
                        self.buf.cur = prev;
                    }
                }
            }
            Key::Del | Key::Ctrl(b'D') => {
                let p = self.buf.cur;
                let next = if p.col < self.buf.lines[p.row].len() {
                    Pos { row: p.row, col: p.col + 1 }
                } else if p.row + 1 < rows {
                    Pos { row: p.row + 1, col: 0 }
                } else {
                    p
                };
                if next != p {
                    self.buf.delete(p, next);
                }
            }
            Key::Left | Key::Ctrl(b'B') => {
                let p = self.buf.cur;
                if p.col > 0 {
                    self.buf.cur.col -= 1;
                } else if p.row > 0 {
                    self.buf.cur = Pos { row: p.row - 1, col: self.buf.lines[p.row - 1].len() };
                }
            }
            Key::Right | Key::Ctrl(b'F') => {
                let p = self.buf.cur;
                if p.col < self.buf.lines[p.row].len() {
                    self.buf.cur.col += 1;
                } else if p.row + 1 < rows {
                    self.buf.cur = Pos { row: p.row + 1, col: 0 };
                }
            }
            Key::Up | Key::Ctrl(b'P') => {
                vertical = true;
                if self.buf.cur.row > 0 {
                    self.buf.cur.row -= 1;
                }
            }
            Key::Down | Key::Ctrl(b'N') => {
                vertical = true;
                if self.buf.cur.row + 1 < rows {
                    self.buf.cur.row += 1;
                }
            }
            Key::PgUp | Key::Ctrl(b'Y') => {
                vertical = true;
                let h = self.text_rows().saturating_sub(2).max(1);
                self.buf.cur.row = self.buf.cur.row.saturating_sub(h);
                self.top = self.top.saturating_sub(h);
            }
            Key::PgDn | Key::Ctrl(b'V') => {
                vertical = true;
                let h = self.text_rows().saturating_sub(2).max(1);
                self.buf.cur.row = (self.buf.cur.row + h).min(rows - 1);
                self.top = (self.top + h).min(rows.saturating_sub(1));
            }
            Key::Home | Key::Ctrl(b'A') => {
                // first press: after the indentation; second: column 0
                let ind = self.buf.lines[self.buf.cur.row].iter().take_while(|c| c.is_whitespace()).count();
                self.buf.cur.col = if self.buf.cur.col == ind { 0 } else { ind };
            }
            Key::End | Key::Ctrl(b'E') => self.buf.cur.col = self.buf.lines[self.buf.cur.row].len(),
            Key::CtrlHome | Key::Meta('\\') | Key::Meta('<') => self.buf.cur = Pos { row: 0, col: 0 },
            Key::CtrlEnd | Key::Meta('/') | Key::Meta('>') => {
                let r = rows - 1;
                self.buf.cur = Pos { row: r, col: self.buf.lines[r].len() };
            }
            Key::CtrlLeft => self.word_left(),
            Key::CtrlRight | Key::Ctrl(b' ') => self.word_right(),
            Key::Ctrl(b'G') => self.help(),
            Key::Ctrl(b'O') => {
                if !self.view_only {
                    self.save(true);
                }
            }
            Key::Ctrl(b'X') => self.exit(),
            Key::Ctrl(b'R') => {
                if let Some(f) = self.prompt("File to insert", "", true) {
                    match rt::fs::read_to_string(&f) {
                        Ok(t) => {
                            let p = self.buf.cur;
                            self.buf.cur = self.buf.insert(p, &t);
                            self.msg = Some(format!("Read {} lines", t.lines().count()));
                        }
                        Err(e) => self.msg = Some(format!("{f}: {e}")),
                    }
                }
            }
            Key::Ctrl(b'W') => {
                self.search(false, true);
            }
            Key::Ctrl(b'Q') => {
                self.search(true, true);
            }
            Key::Meta('w') => {
                self.search(false, false);
            }
            Key::Meta('q') => {
                self.search(true, false);
            }
            Key::Ctrl(b'\\') | Key::Meta('r') | Key::Meta('%') => self.replace(),
            Key::Ctrl(b'K') => {
                self.cutbuf_lines = self.mark.is_none();
                self.cutting = was_cutting && self.cutbuf_lines;
                self.cut();
            }
            Key::Meta('6') | Key::Meta('^') => {
                self.cutbuf_lines = self.mark.is_none();
                self.cutting = was_cutting && self.cutbuf_lines;
                self.copy();
            }
            Key::Ctrl(b'U') => self.paste(),
            Key::Ctrl(b'^') | Key::Meta('a') => {
                if self.mark.is_some() {
                    self.mark = None;
                    self.msg = Some("Mark Unset".into());
                } else {
                    self.mark = Some(self.buf.cur);
                    self.msg = Some("Mark Set".into());
                }
            }
            Key::Ctrl(b'C') => {
                let c = self.buf.cur;
                let chars: usize = self.buf.lines.iter().map(|l| l.len() + 1).sum();
                let before: usize = self.buf.lines[..c.row].iter().map(|l| l.len() + 1).sum::<usize>() + c.col;
                self.msg = Some(format!(
                    "line {}/{} ({}%), col {}/{} , char {}/{} ({}%)",
                    c.row + 1,
                    rows,
                    (c.row + 1) * 100 / rows,
                    c.col + 1,
                    self.buf.lines[c.row].len() + 1,
                    before,
                    chars,
                    if chars > 0 { before * 100 / chars } else { 0 }
                ));
            }
            Key::Ctrl(b'_') | Key::Meta('g') => self.goto_line(),
            Key::Ctrl(b'J') => self.justify(),
            Key::Ctrl(b'L') => self.redraw_all(),
            Key::Meta('u') => match self.buf.undo() {
                Some(p) => self.buf.cur = p,
                None => self.msg = Some("Nothing to undo".into()),
            },
            Key::Meta('e') => match self.buf.redo() {
                Some(p) => self.buf.cur = p,
                None => self.msg = Some("Nothing to redo".into()),
            },
            Key::Meta('n') | Key::Meta('#') => {
                self.numbers = !self.numbers;
                self.screen.clear();
            }
            Key::Meta('i') => {
                self.autoindent = !self.autoindent;
                self.msg = Some(format!("Auto indent {}", if self.autoindent { "enabled" } else { "disabled" }));
            }
            Key::Meta(']') => self.match_bracket(),
            _ => {}
        }
        if !is_cut_key {
            self.cutting = false;
        }
        let c = self.buf.cur;
        if vertical {
            self.buf.cur.col = self.char_at_col(c.row, self.want_col);
        } else {
            self.want_col = self.disp_col(c.row, c.col);
        }
    }
}

fn main(args: &[String]) -> i32 {
    let mut files = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    let mut opt_args = Vec::new();
    for a in &args[1..] {
        if let Some(n) = a.strip_prefix('+') {
            let (l, c) = n.split_once(',').unwrap_or((n, "1"));
            start = Some((l.parse().unwrap_or(1), c.parse().unwrap_or(1)));
        } else {
            opt_args.push(a.clone());
        }
    }
    let (o, rest) = rt::getopt::parse(&opt_args, "ilvT:w", "[-ilv] [-T tabsize] [+line[,col]] [file...]");
    files.extend(rest);
    if !io::isatty(0) || !io::isatty(1) {
        rt::die!("standard input and output must be a terminal");
    }
    let (rows, cols) = rt::term::size(1);
    let mut ed = Editor {
        buf: Buffer::new(),
        top: 0,
        left: 0,
        rows: rows as usize,
        cols: cols as usize,
        want_col: 0,
        msg: None,
        cutbuf: Vec::new(),
        cutting: false,
        last_search: String::new(),
        last_replace: String::new(),
        tab: o.get('T').and_then(|t| t.parse().ok()).filter(|t| *t > 0).unwrap_or(8),
        autoindent: o.has('i'),
        numbers: o.has('l'),
        view_only: o.has('v'),
        mark: None,
        screen: Vec::new(),
        quit: false,
        files: files.clone(),
        file_idx: 0,
        cutbuf_lines: true,
    };
    if let Some(f) = files.first() {
        ed.open(f);
    }
    if let Some((l, c)) = start {
        let row = l.saturating_sub(1).min(ed.buf.lines.len() - 1);
        ed.buf.cur = Pos { row, col: c.saturating_sub(1).min(ed.buf.lines[row].len()) };
        ed.want_col = ed.disp_col(row, ed.buf.cur.col);
    }
    let Ok(raw) = RawMode::enter(0) else { rt::die!("cannot set terminal mode") };
    // disable output processing so the screen layout is exact
    if let Ok(mut t) = rt::term::get_attr(0) {
        t.oflag &= !rt::term::OPOST;
        let _ = rt::term::set_attr(0, &t);
    }
    let _ = io::write_fd(1, b"\x1b[?1049h\x1b[H\x1b[2J");
    while !ed.quit {
        ed.draw();
        match read_key() {
            Some(k) => ed.handle(k),
            None => break,
        }
    }
    let _ = io::write_fd(1, format!("\x1b[{};1H\x1b[K\x1b[?1049l", ed.rows).as_bytes());
    drop(raw);
    0
}
