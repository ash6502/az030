//! Host archiver: `azar ARCHIVE FILE...` writes a new archive of the files.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, files)) = args.split_first() else {
        eprintln!("usage: azar ARCHIVE FILE...");
        return ExitCode::FAILURE;
    };
    let mut members = Vec::new();
    for f in files {
        match std::fs::read(f) {
            Ok(d) => members.push((f.clone(), d)),
            Err(e) => {
                eprintln!("azar: {f}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = std::fs::write(out, azld::archive::write(&members)) {
        eprintln!("azar: {out}: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
