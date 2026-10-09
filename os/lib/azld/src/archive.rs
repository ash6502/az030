//! Unix `ar` archives: reading (GNU and BSD member names) and writing (GNU style,
//! no symbol index; the linker loads every member).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const MAGIC: &[u8] = b"!<arch>\n";

pub struct Member<'a> {
    pub name: String,
    pub data: &'a [u8],
}

pub fn is_archive(d: &[u8]) -> bool {
    d.starts_with(MAGIC)
}

fn field(h: &[u8], a: usize, b: usize) -> &str {
    core::str::from_utf8(&h[a..b]).unwrap_or("").trim_end()
}

/// List the members of an archive, skipping symbol tables.
pub fn members(d: &[u8]) -> Result<Vec<Member<'_>>, String> {
    let mut out = Vec::new();
    let mut longnames: &[u8] = &[];
    let mut p = MAGIC.len();
    while p + 60 <= d.len() {
        let h = &d[p..p + 60];
        if &h[58..60] != b"`\n" {
            return Err(format!("bad archive header at offset {p}"));
        }
        let size: usize = field(h, 48, 58).parse().map_err(|_| "bad archive member size")?;
        let mut body = p + 60;
        let end = body.checked_add(size).filter(|&e| e <= d.len()).ok_or("truncated archive")?;
        let raw = field(h, 0, 16);
        let name: String;
        if raw == "/" || raw == "/SYM64/" || raw.starts_with("__.SYMDEF") {
            name = String::new();
        } else if raw == "//" {
            longnames = &d[body..end];
            name = String::new();
        } else if let Some(n) = raw.strip_prefix("#1/") {
            // BSD: name stored at the start of the data
            let len: usize = n.parse().map_err(|_| "bad BSD name length")?;
            let nm = &d[body..body + len];
            let nm = &nm[..nm.iter().position(|&b| b == 0).unwrap_or(len)];
            name = String::from_utf8_lossy(nm).to_string();
            body += len;
            if name.starts_with("__.SYMDEF") {
                p = end + (end & 1);
                continue;
            }
        } else if let Some(off) = raw.strip_prefix('/') {
            let off: usize = off.parse().map_err(|_| "bad long-name offset")?;
            let rest = longnames.get(off..).ok_or("bad long-name offset")?;
            let e = rest.iter().position(|&b| b == b'\n' || b == 0).unwrap_or(rest.len());
            name = String::from_utf8_lossy(&rest[..e]).trim_end_matches('/').to_string();
        } else {
            name = raw.trim_end_matches('/').to_string();
        }
        if !name.is_empty() {
            out.push(Member { name, data: &d[body..end] });
        }
        p = end + (end & 1);
    }
    Ok(out)
}

/// Build a GNU-style archive from (name, data) pairs.
pub fn write(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let mut longnames = Vec::new();
    let mut names = Vec::new();
    for (n, _) in files {
        let base = n.rsplit('/').next().unwrap_or(n);
        if base.len() < 16 {
            names.push(format!("{base}/"));
        } else {
            names.push(format!("/{}", longnames.len()));
            longnames.extend_from_slice(base.as_bytes());
            longnames.extend_from_slice(b"/\n");
        }
    }
    let header = |out: &mut Vec<u8>, name: &str, size: usize| {
        let h = format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", name, 0, 0, 0, 644, size);
        out.extend_from_slice(h.as_bytes());
    };
    if !longnames.is_empty() {
        header(&mut out, "//", longnames.len());
        out.extend_from_slice(&longnames);
        if out.len() & 1 == 1 {
            out.push(b'\n');
        }
    }
    for ((_, data), name) in files.iter().zip(&names) {
        header(&mut out, name, data.len());
        out.extend_from_slice(data);
        if out.len() & 1 == 1 {
            out.push(b'\n');
        }
    }
    out
}
