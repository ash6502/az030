//! Every program gets the start-up code and the assembly runtime, and is linked as a
//! static ELF executable at 0x40000000 (azld's defaults).
fn main() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut objs = vec![azbuild::assemble(&dir.join("asm/crt0.s"))];
    for f in ["mem.s", "int.s", "float.s"] {
        objs.push(azbuild::assemble(&azbuild::rtasm(f)));
    }
    azbuild::link_args(objs.iter().map(|p| p.display().to_string()));
    azbuild::link_args(["--keep", "rt_sigreturn"]);
}
