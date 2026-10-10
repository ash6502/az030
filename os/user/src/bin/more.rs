//! more / less: page through text.
//!
//!     more [-N] [+line] [file...]
//!
//! Keys: space/f/PgDn next page, b/PgUp previous page, Enter/j/↓ next line,
//! k/↑ previous line, d/u half page, g/< top, G/> bottom, /re search forward,
//! ?re backward, n/N repeat search, :n/:p next/previous file, h help, q quit.

#![no_std]
#![no_main]

use rt::io;
use rt::prelude::*;
use rt::regex::Regex;

rt::main!(main);

struct Pager {
    lines: Vec<String>,
    top: usize,
    rows: usize,
    cols: usize,
    name: String,
    search: Option<(Regex, bool)>,
    numbers: bool,
    tty: rt::io::Fd,
    msg: Option<String>,
}

fn expand_tabs(s: &str) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for c in s.chars() {
        if c == '\t' {
            let n = 8 - col % 8;
            for _ in 0..n {
                out.push(' ');
            }
            col += n;
        } else if c == '\x08' {
            // overstrike (man pages): drop
            out.pop();
            col = col.saturating_sub(1);
        } else if (c as u32) < 0x20 && c != '\x1b' {
            out.push('^');
            out.push(char::from_u32(c as u32 + 0x40).unwrap_or('?'));
            col += 2;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

impl Pager {
    fn page(&self) -> usize {
        self.rows.saturating_sub(1).max(1)
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.page())
    }

    fn draw(&mut self) {
        let mut out = String::from("\x1b[H\x1b[2J");
        let hl = self.search.as_ref().map(|(r, _)| r);
        for i in self.top..(self.top + self.page()).min(self.lines.len()) {
            let mut l = String::new();
            if self.numbers {
                l.push_str(&format!("{:6} ", i + 1));
            }
            let text = &self.lines[i];
            let room = self.cols.saturating_sub(l.chars().count());
            let shown: String = text.chars().take(room).collect();
            match hl.and_then(|r| r.find(&shown)) {
                Some((a, b)) if b > a => {
                    l.push_str(&shown[..a]);
                    l.push_str("\x1b[7m");
                    l.push_str(&shown[a..b]);
                    l.push_str("\x1b[0m");
                    l.push_str(&shown[b..]);
                }
                _ => l.push_str(&shown),
            }
            out.push_str(&l);
            out.push_str("\r\n");
        }
        for _ in self.lines.len().saturating_sub(self.top)..self.page() {
            out.push_str("~\r\n");
        }
        let pct = if self.lines.is_empty() { 100 } else { ((self.top + self.page()).min(self.lines.len()) * 100) / self.lines.len() };
        let status = match self.msg.take() {
            Some(m) => m,
            None if self.top >= self.max_top() => format!("{} (END)", self.name),
            None => format!("{} {}%", self.name, pct),
        };
        out.push_str(&format!("\x1b[7m{}\x1b[0m\x1b[K", status.chars().take(self.cols.saturating_sub(1)).collect::<String>()));
        let _ = io::write_fd(1, out.as_bytes());
    }

    fn key(&self) -> u32 {
        let mut b = [0u8; 1];
        if io::read_fd(self.tty, &mut b).unwrap_or(0) == 0 {
            return b'q' as u32;
        }
        if b[0] != 0x1b {
            return b[0] as u32;
        }
        let mut s = [0u8; 1];
        if io::read_fd(self.tty, &mut s).unwrap_or(0) == 0 || (s[0] != b'[' && s[0] != b'O') {
            return 0x1b;
        }
        let mut n = 0u32;
        loop {
            if io::read_fd(self.tty, &mut s).unwrap_or(0) == 0 {
                return 0x1b;
            }
            if s[0].is_ascii_digit() {
                n = n * 10 + (s[0] - b'0') as u32;
                continue;
            }
            return match (s[0], n) {
                (b'A', _) => 0x100,
                (b'B', _) => 0x101,
                (b'~', 5) => 0x102,
                (b'~', 6) => 0x103,
                (b'H', _) | (b'~', 1) => 0x104,
                (b'F', _) | (b'~', 4) => 0x105,
                _ => 0,
            };
        }
    }

    fn prompt(&mut self, p: &str) -> String {
        let _ = io::write_fd(1, format!("\r\x1b[K{p}").as_bytes());
        let mut s = String::new();
        loop {
            let k = self.key();
            match k {
                0x0d | 0x0a => break,
                0x1b | 0x03 => return String::new(),
                0x7f | 0x08 => {
                    if s.pop().is_some() {
                        let _ = io::write_fd(1, b"\x08 \x08");
                    } else {
                        return String::new();
                    }
                }
                c if (0x20..0x7f).contains(&c) => {
                    s.push(c as u8 as char);
                    let _ = io::write_fd(1, &[c as u8]);
                }
                _ => {}
            }
        }
        s
    }

    fn find(&mut self, forward: bool) {
        let Some((re, dir)) = &self.search else { return };
        let fwd = forward == *dir;
        let n = self.lines.len();
        let mut i = self.top;
        for _ in 0..n {
            if fwd {
                if i + 1 >= n {
                    break;
                }
                i += 1;
            } else {
                if i == 0 {
                    break;
                }
                i -= 1;
            }
            if re.is_match(&self.lines[i]) {
                self.top = i;
                return;
            }
        }
        self.msg = Some(String::from("Pattern not found"));
    }

    /// Returns 1 = next file, -1 = previous file, 0 = quit.
    fn run(&mut self) -> i32 {
        loop {
            if self.top > self.max_top() && self.top >= self.lines.len() {
                self.top = self.max_top();
            }
            self.draw();
            let half = (self.page() / 2).max(1);
            match self.key() {
                k if k == b'q' as u32 || k == b'Q' as u32 => return 0,
                k if k == b' ' as u32 || k == b'f' as u32 || k == 0x103 || k == 0x06 => {
                    if self.top >= self.max_top() {
                        return 1;
                    }
                    self.top = (self.top + self.page()).min(self.max_top());
                }
                k if k == b'b' as u32 || k == 0x102 || k == 0x02 => self.top = self.top.saturating_sub(self.page()),
                k if k == 0x0d || k == 0x0a || k == b'j' as u32 || k == 0x101 || k == 0x0e || k == b'e' as u32 => {
                    if self.top < self.max_top() {
                        self.top += 1;
                    } else if k != 0x101 && k != b'j' as u32 {
                        return 1;
                    }
                }
                k if k == b'k' as u32 || k == 0x100 || k == 0x10 || k == b'y' as u32 => self.top = self.top.saturating_sub(1),
                k if k == b'd' as u32 || k == 0x04 => self.top = (self.top + half).min(self.max_top()),
                k if k == b'u' as u32 || k == 0x15 => self.top = self.top.saturating_sub(half),
                k if k == b'g' as u32 || k == b'<' as u32 || k == 0x104 => self.top = 0,
                k if k == b'G' as u32 || k == b'>' as u32 || k == 0x105 => self.top = self.max_top(),
                k if k == b'/' as u32 || k == b'?' as u32 => {
                    let fwd = k == b'/' as u32;
                    let p = self.prompt(if fwd { "/" } else { "?" });
                    if !p.is_empty() {
                        match Regex::new(&p, true, false) {
                            Ok(r) => {
                                self.search = Some((r, fwd));
                                self.find(true);
                            }
                            Err(e) => self.msg = Some(e),
                        }
                    }
                }
                k if k == b'n' as u32 => self.find(true),
                k if k == b'N' as u32 => self.find(false),
                k if k == b':' as u32 => {
                    let c = self.prompt(":");
                    match c.as_str() {
                        "n" => return 1,
                        "p" => return -1,
                        "q" => return 0,
                        _ => {}
                    }
                }
                k if k == b'h' as u32 => {
                    self.msg = Some("SPACE/f next page  b back  ENTER/j line  k up  d/u half  g/G top/bottom  /re search  n/N again  q quit".into());
                }
                k if k == 0x0c || k == b'r' as u32 => {}
                k if k == b'=' as u32 => {
                    self.msg = Some(format!("{}: lines {}-{} of {}", self.name, self.top + 1, (self.top + self.page()).min(self.lines.len()), self.lines.len()));
                }
                _ => {}
            }
        }
    }
}

fn main(args: &[String]) -> i32 {
    let (o, rest) = rt::getopt::parse(&args[1..], "NnsRrXFe", "[-N] [+line] [file...]");
    let mut start = 0;
    let mut files = Vec::new();
    for r in rest {
        if let Some(n) = r.strip_prefix('+') {
            start = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
        } else {
            files.push(r);
        }
    }
    let files = rt::io::inputs(&files);
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut st = 0;
    for f in &files {
        match rt::io::open_input(f) {
            Ok(mut r) => {
                let mut v = Vec::new();
                let _ = r.read_to_end(&mut v);
                texts.push((f.clone(), String::from_utf8_lossy(&v).into_owned()));
            }
            Err(e) => {
                rt::warn!("{f}: {e}");
                st = 1;
            }
        }
    }
    // not on a terminal: behave like cat
    if !io::isatty(1) {
        for (_, t) in &texts {
            print!("{t}");
        }
        return st;
    }
    let tty = if io::isatty(0) { 0 } else {
        match rt::fs::File::open_with("/dev/tty", azsys::flags::O_RDONLY, 0) {
            Ok(f) => f.into_raw(),
            Err(_) => {
                for (_, t) in &texts {
                    print!("{t}");
                }
                return st;
            }
        }
    };
    let (rows, cols) = rt::term::size(1);
    // short input that fits on the screen is just printed
    if texts.len() == 1 && rt::util::lines(&texts[0].1).len() < rows as usize && start == 0 {
        print!("{}", texts[0].1);
        return st;
    }
    let Ok(_raw) = rt::term::RawMode::enter(tty) else {
        for (_, t) in &texts {
            print!("{t}");
        }
        return st;
    };
    let mut k = 0usize;
    while k < texts.len() {
        let (name, text) = &texts[k];
        let lines: Vec<String> = rt::util::lines(text).into_iter().map(expand_tabs).collect();
        let mut p = Pager {
            lines,
            top: start,
            rows: rows as usize,
            cols: cols as usize,
            name: if name == "-" { "(stdin)".into() } else { name.clone() },
            search: None,
            numbers: o.has('N'),
            tty,
            msg: if texts.len() > 1 { Some(format!("{name} (file {} of {})", k + 1, texts.len())) } else { None },
        };
        start = 0;
        match p.run() {
            1 => k += 1,
            -1 => k = k.saturating_sub(1),
            _ => break,
        }
    }
    let _ = io::write_fd(1, b"\r\x1b[K");
    st
}
