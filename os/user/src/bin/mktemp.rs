//! mktemp: create a unique temporary file or directory.
//!
//!     mktemp [-d] [-p dir] [template]   (template ends in XXXXXX)

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, rest) = rt::getopt::parse(&args[1..], "dp:qut", "[-d] [-p dir] [template]");
    let template = rest.first().cloned().unwrap_or_else(|| "tmp.XXXXXX".into());
    let x = template.chars().rev().take_while(|c| *c == 'X').count();
    if x < 3 {
        rt::die!("too few X's in template '{template}'");
    }
    let base = &template[..template.len() - x];
    let dir = o.get('p').map(String::from).or_else(|| if template.contains('/') { None } else { Some(rt::env::var("TMPDIR").unwrap_or_else(|| "/tmp".into())) });
    const CH: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let (s, us) = rt::time::now_precise();
    let mut seed = (s as u64) << 20 ^ us as u64 ^ (rt::process::id() as u64) << 40;
    for _ in 0..100 {
        let mut name = String::from(base);
        for _ in 0..x {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            name.push(CH[(seed >> 33) as usize % CH.len()] as char);
        }
        let path = match &dir {
            Some(d) => rt::path::join(d, &name),
            None => name,
        };
        let r = if o.has('d') {
            rt::fs::create_dir_mode(&path, 0o700)
        } else {
            rt::fs::File::open_with(&path, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_EXCL, 0o600).map(|_| ())
        };
        match r {
            Ok(()) => {
                println!("{path}");
                return 0;
            }
            Err(e) if e.0 == azsys::errno::EEXIST => continue,
            Err(e) => rt::die!("{path}: {e}"),
        }
    }
    rt::die!("cannot create a unique name")
}
