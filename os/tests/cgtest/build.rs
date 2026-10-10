//! Bare-metal image booted directly by the ROM at 0x10000.
fn main() {
    let objs = ["mem.s", "int.s", "float.s"].map(|f| azbuild::assemble(&azbuild::rtasm(f)));
    let out = std::env::var("OUT_DIR").unwrap();
    azbuild::link_args(objs.iter().map(|p| p.display().to_string()));
    azbuild::link_args(["--base", "0x10000", "--format", "bin", "--entry", "_start", "--map", &format!("{out}/cgtest.map")]);
}
