//! dd: copy and convert blocks.
//!
//!     dd [if=file] [of=file] [bs=n] [ibs=n] [obs=n] [count=n] [skip=n] [seek=n] [conv=notrunc,sync,ucase,lcase]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn size(s: &str) -> usize {
    let (n, mult) = match s.chars().last() {
        Some('k' | 'K') => (&s[..s.len() - 1], 1024),
        Some('M') => (&s[..s.len() - 1], 1024 * 1024),
        Some('b') => (&s[..s.len() - 1], 512),
        Some('w') => (&s[..s.len() - 1], 2),
        _ => (s, 1),
    };
    n.parse::<usize>().unwrap_or_else(|_| rt::die!("invalid number '{s}'")) * mult
}

fn main(args: &[String]) -> i32 {
    let (mut inf, mut outf) = (None, None);
    let (mut ibs, mut obs) = (512, 512);
    let (mut count, mut skip, mut seek) = (None, 0, 0);
    let mut conv: Vec<String> = Vec::new();
    for a in &args[1..] {
        let Some((k, v)) = a.split_once('=') else { rt::die!("unrecognized operand '{a}'") };
        match k {
            "if" => inf = Some(v.to_string()),
            "of" => outf = Some(v.to_string()),
            "bs" => {
                ibs = size(v);
                obs = ibs;
            }
            "ibs" => ibs = size(v),
            "obs" => obs = size(v),
            "count" => count = Some(size(v)),
            "skip" => skip = size(v),
            "seek" => seek = size(v),
            "conv" => conv = v.split(',').map(String::from).collect(),
            "status" => {}
            _ => rt::die!("unrecognized operand '{a}'"),
        }
    }
    let mut input: Box<dyn Read> = match &inf {
        Some(f) => match rt::fs::File::open(f) {
            Ok(mut file) => {
                if skip > 0 {
                    let _ = file.seek((skip * ibs) as i32, azsys::flags::SEEK_SET);
                }
                Box::new(file)
            }
            Err(e) => rt::die!("{f}: {e}"),
        },
        None => {
            let mut r = rt::io::FdIo(0);
            let mut junk = vec![0u8; ibs];
            for _ in 0..skip {
                let _ = r.read_exact(&mut junk);
            }
            Box::new(r)
        }
    };
    let notrunc = conv.iter().any(|c| c == "notrunc");
    let mut output: Box<dyn Write> = match &outf {
        Some(f) => {
            let flags = azsys::flags::O_WRONLY | azsys::flags::O_CREAT | if notrunc || seek > 0 { 0 } else { azsys::flags::O_TRUNC };
            match rt::fs::File::open_with(f, flags, 0o666) {
                Ok(mut file) => {
                    if seek > 0 {
                        let _ = file.seek((seek * obs) as i32, azsys::flags::SEEK_SET);
                    }
                    Box::new(file)
                }
                Err(e) => rt::die!("{f}: {e}"),
            }
        }
        None => Box::new(rt::io::FdIo(1)),
    };
    let (mut full_in, mut part_in, mut bytes) = (0, 0, 0u64);
    let mut buf = vec![0u8; ibs];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        if count.is_some_and(|c| full_in + part_in >= c) {
            break;
        }
        let mut got = 0;
        while got < ibs {
            match input.read(&mut buf[got..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        if got == 0 {
            break;
        }
        if got == ibs {
            full_in += 1;
        } else {
            part_in += 1;
        }
        let mut block = buf[..got].to_vec();
        if conv.iter().any(|c| c == "sync") && got < ibs {
            block.resize(ibs, 0);
        }
        if conv.iter().any(|c| c == "ucase") {
            block.make_ascii_uppercase();
        }
        if conv.iter().any(|c| c == "lcase") {
            block.make_ascii_lowercase();
        }
        pending.extend_from_slice(&block);
        while pending.len() >= obs {
            let _ = output.write_all(&pending[..obs]);
            bytes += obs as u64;
            pending.drain(..obs);
        }
        if got < ibs {
            break;
        }
    }
    if !pending.is_empty() {
        let _ = output.write_all(&pending);
        bytes += pending.len() as u64;
    }
    let (fo, po) = (bytes / obs as u64, if bytes % obs as u64 != 0 { 1 } else { 0 });
    eprintln!("{full_in}+{part_in} records in\n{fo}+{po} records out\n{bytes} bytes copied");
    0
}
