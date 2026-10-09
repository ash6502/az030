//! Assemble the kernel's low-level code with azas and tell azld how to lay out the
//! kernel: a flat image at the physical load address the bootloader uses.

use std::path::PathBuf;

const LOAD_ADDR: &str = "0x10000";

struct Files(PathBuf);

impl azas::Files for Files {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(name)).ok()
    }
}

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = dir.join("asm/entry.s");
    println!("cargo:rerun-if-changed={}", src.display());
    let text = std::fs::read_to_string(&src).expect("read entry.s");
    let opts = azas::Options { format: azas::Format::Elf, defines: Vec::new() };
    let obj = match azas::assemble("asm/entry.s", &text, opts, &mut Files(dir.join("asm"))) {
        Ok(o) => o,
        Err(errs) => {
            for e in errs {
                eprintln!("{e}");
            }
            panic!("assembling entry.s failed");
        }
    };
    let obj_path = out.join("entry.o");
    std::fs::write(&obj_path, obj).unwrap();
    let map = out.join("kernel.map");
    for arg in [
        obj_path.display().to_string(),
        "--base".into(),
        LOAD_ADDR.into(),
        "--format".into(),
        "bin".into(),
        "--entry".into(),
        "_start".into(),
        "--keep".into(),
        "trap_handler".into(),
        "--map".into(),
        map.display().to_string(),
    ] {
        println!("cargo:rustc-link-arg={arg}");
    }
}
