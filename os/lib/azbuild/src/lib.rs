//! Helpers for the build scripts of crates that run on the az030.

use std::path::{Path, PathBuf};

struct Files(PathBuf);

impl azas::Files for Files {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(name)).ok()
    }
}

/// Assemble `src` (an azas source file) into an ELF object in OUT_DIR and return
/// its path. Panics with the assembler's messages on failure.
pub fn assemble(src: &Path) -> PathBuf {
    println!("cargo:rerun-if-changed={}", src.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let text = std::fs::read_to_string(src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let dir = src.parent().unwrap_or(Path::new(".")).to_path_buf();
    let opts = azas::Options { format: azas::Format::Elf, defines: Vec::new() };
    let obj = match azas::assemble(&src.display().to_string(), &text, opts, &mut Files(dir)) {
        Ok(o) => o,
        Err(errs) => {
            for e in &errs {
                eprintln!("{e}");
            }
            panic!("assembling {} failed", src.display());
        }
    };
    let name = src.file_stem().unwrap().to_string_lossy();
    let path = out.join(format!("{name}.o"));
    std::fs::write(&path, obj).unwrap();
    path
}

/// Pass extra arguments to azld for every binary of this crate.
pub fn link_args<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) {
    for a in args {
        println!("cargo:rustc-link-arg={}", a.as_ref());
    }
}

/// The os/lib/rtasm directory, relative to a crate's manifest directory.
pub fn rtasm(file: &str) -> PathBuf {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates live one or two levels below os/
    for up in ["../lib/rtasm", "../../lib/rtasm"] {
        let p = dir.join(up).join(file);
        if p.exists() {
            return p;
        }
    }
    panic!("cannot find lib/rtasm/{file}");
}
