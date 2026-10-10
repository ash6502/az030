//! Assemble the kernel's low-level code and runtime helpers with azas and tell azld to
//! lay the kernel out as a flat image at the physical address the bootloader loads it to.

const LOAD_ADDR: &str = "0x10000";

fn main() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = std::env::var("OUT_DIR").unwrap();
    let mut objs = vec![azbuild::assemble(&dir.join("asm/entry.s"))];
    // no float.s: the kernel never uses the FPU for itself
    for f in ["mem.s", "int.s"] {
        objs.push(azbuild::assemble(&azbuild::rtasm(f)));
    }
    azbuild::link_args(objs.iter().map(|p| p.display().to_string()));
    azbuild::link_args([
        "--base", LOAD_ADDR, "--format", "bin", "--entry", "_start",
        "--keep", "trap_handler", "--map", &format!("{out}/kernel.map"),
    ]);
}
