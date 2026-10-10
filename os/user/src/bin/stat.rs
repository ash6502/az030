//! stat: display file status.
//!
//!     stat [-L] file...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn kind(m: &rt::fs::Metadata) -> &'static str {
    use azsys::mode::*;
    match m.kind() {
        S_IFREG if m.is_empty() => "regular empty file",
        S_IFREG => "regular file",
        S_IFDIR => "directory",
        S_IFLNK => "symbolic link",
        S_IFCHR => "character special file",
        S_IFBLK => "block special file",
        S_IFIFO => "fifo",
        S_IFSOCK => "socket",
        _ => "unknown",
    }
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "L", "[-L] file...");
    if files.is_empty() {
        rt::die!("missing operand");
    }
    let mut st = 0;
    for f in &files {
        let r = if o.has('L') { rt::fs::metadata(f) } else { rt::fs::symlink_metadata(f) };
        let m = match r {
            Ok(m) => m,
            Err(e) => {
                rt::warn!("cannot stat '{f}': {e}");
                st = 1;
                continue;
            }
        };
        let name = if m.is_symlink() { format!("{f} -> {}", rt::fs::read_link(f).unwrap_or_default()) } else { f.clone() };
        println!("  File: {name}");
        println!("  Size: {:<12} Blocks: {:<8} IO Block: {:<6} {}", m.len(), m.0.blocks, m.0.blksize, kind(&m));
        let dev = m.dev();
        print!("Device: {},{:<8} Inode: {:<10} Links: {}", dev >> 8, dev & 0xFF, m.ino(), m.nlink());
        if matches!(m.kind(), azsys::mode::S_IFCHR | azsys::mode::S_IFBLK) {
            print!("  Device type: {},{}", m.rdev() >> 8, m.rdev() & 0xFF);
        }
        println!();
        println!(
            "Access: ({:04o}/{})  Uid: ({:5}/{:>8})   Gid: ({:5}/{:>8})",
            m.mode() & 0o7777,
            m.mode_string(),
            m.uid(),
            rt::users::user_name(m.uid()),
            m.gid(),
            rt::users::group_name(m.gid())
        );
        println!("Access: {}", rt::time::format_date(m.0.atime));
        println!("Modify: {}", rt::time::format_date(m.mtime()));
        println!("Change: {}", rt::time::format_date(m.0.ctime));
    }
    st
}
