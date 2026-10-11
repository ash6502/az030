//! man: display manual pages.
//!
//!     man [section] name
//!     man -k keyword        (apropos: search the page summaries)
//!     man -l                (list all pages)
//!
//! Pages are plain text files in /usr/share/man/manN/NAME.N; the first line
//! after "NAME" is the summary searched by -k. Output goes through $PAGER
//! (default `more`) when standard output is a terminal.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

const ROOT: &str = "/usr/share/man";

fn sections() -> Vec<String> {
    let mut v: Vec<String> = rt::fs::read_dir(ROOT)
        .map(|es| es.into_iter().map(|e| e.name).filter(|n| n.starts_with("man")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// (section, name, path) of every page.
fn all_pages() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for s in sections() {
        let dir = rt::path::join(ROOT, &s);
        let Ok(es) = rt::fs::read_dir(&dir) else { continue };
        for e in es {
            if e.name.starts_with('.') {
                continue;
            }
            let (name, sec) = match e.name.rfind('.') {
                Some(i) => (String::from(&e.name[..i]), String::from(&e.name[i + 1..])),
                None => (e.name.clone(), String::from(&s[3..])),
            };
            out.push((sec, name, rt::path::join(&dir, &e.name)));
        }
    }
    out.sort();
    out
}

fn summary(path: &str) -> String {
    let text = rt::fs::read_to_string(path).unwrap_or_default();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "NAME" {
            return String::from(lines.next().unwrap_or("").trim());
        }
    }
    String::from(text.lines().next().unwrap_or("").trim())
}

fn show(path: &str) -> i32 {
    let pager = rt::env::var("PAGER").unwrap_or_else(|| "more".into());
    if rt::io::isatty(1) {
        let args = vec![pager.clone(), String::from(path)];
        match rt::process::run(&args) {
            Ok(s) => return s.shell_code(),
            Err(_) => {}
        }
    }
    match rt::fs::read_to_string(path) {
        Ok(t) => {
            print!("{t}");
            0
        }
        Err(e) => rt::die!("{path}: {e}"),
    }
}

fn main(args: &[String]) -> i32 {
    match args.get(1).map(|s| s.as_str()) {
        None => {
            eprintln!("What manual page do you want?\nFor example, try 'man man' or 'man intro'.");
            1
        }
        Some("-k") => {
            let Some(k) = args.get(2) else { rt::die!("usage: man -k keyword") };
            let k = k.to_lowercase();
            let mut found = false;
            for (sec, name, path) in all_pages() {
                let s = summary(&path);
                if name.to_lowercase().contains(&k) || s.to_lowercase().contains(&k) {
                    println!("{:<20} {}", format!("{name}({sec})"), s.split_once(" - ").map_or(s.as_str(), |x| x.1));
                    found = true;
                }
            }
            if found { 0 } else {
                eprintln!("{k}: nothing appropriate.");
                1
            }
        }
        Some("-l") => {
            for (sec, name, path) in all_pages() {
                let s = summary(&path);
                println!("{:<20} {}", format!("{name}({sec})"), s.split_once(" - ").map_or(s.as_str(), |x| x.1));
            }
            0
        }
        Some(_) => {
            let (sec, name) = if args.len() > 2 { (Some(args[1].as_str()), args[2].as_str()) } else { (None, args[1].as_str()) };
            let pages = all_pages();
            let hit = pages.iter().find(|(s, n, _)| n == name && sec.is_none_or(|x| x == s));
            match hit {
                Some((_, _, path)) => show(path),
                None => {
                    match sec {
                        Some(s) => eprintln!("No entry for {name} in section {s} of the manual"),
                        None => eprintln!("No manual entry for {name}"),
                    }
                    1
                }
            }
        }
    }
}
