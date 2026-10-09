//! Host front end for the azld linker. Accepts the GNU ld-style arguments rustc passes
//! (`-o`, `-L`, objects, rlibs, assorted flags it can ignore) plus its own options:
//!
//!   --base ADDR       address of .text (default 0x40000000)
//!   --entry SYM / -e  entry symbol (default _start)
//!   --format elf|bin  output format (default elf)
//!   --no-gc           keep every section
//!   --keep SYM        extra GC root
//!   --no-page-align   do not start .data on a new page
//!   --map FILE        write a link map
//!   -s / --strip-all  no symbol table
//!   -lNAME            link libNAME.a from the -L directories
//!
//! LLVM bitcode inputs (rustc's `-C linker-plugin-lto` output) are compiled with the
//! llc named by $AZLD_LLC, caching objects by content hash in $AZLD_CACHE (default
//! $TMPDIR/azld-cache). `-plugin-opt=mcpu=CPU` and `-plugin-opt=O<n>` are honoured.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("azld: {e}");
            ExitCode::FAILURE
        }
    }
}

fn num(s: &str) -> Result<u32, String> {
    let r = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else {
        s.parse()
    };
    r.map_err(|_| format!("bad number {s:?}"))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = azld::Options::default();
    let mut out: Option<PathBuf> = None;
    let mut map: Option<PathBuf> = None;
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut libdirs: Vec<PathBuf> = Vec::new();
    let mut libs: Vec<String> = Vec::new();
    let mut cpu = String::from("M68030");
    let mut opt_level = String::from("2");
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = |what: &str| it.next().ok_or_else(|| format!("{what} needs an argument"));
        match a.as_str() {
            "-o" => out = Some(val("-o")?.into()),
            "--base" | "-Ttext" => opts.base = num(&val("--base")?)?,
            "--entry" | "-e" => opts.entry = val("--entry")?,
            "--format" => {
                opts.format = match val("--format")?.as_str() {
                    "elf" => azld::Format::Elf,
                    "bin" | "binary" => azld::Format::Binary,
                    f => return Err(format!("unknown format {f}")),
                }
            }
            "--no-gc" | "--no-gc-sections" => opts.gc_sections = false,
            "--gc-sections" => opts.gc_sections = true,
            "--keep" => opts.keep.push(val("--keep")?),
            "--no-page-align" => opts.page_align_data = false,
            "--map" | "-Map" => map = Some(val("--map")?.into()),
            "-s" | "--strip-all" => opts.symbols = false,
            "-L" => libdirs.push(val("-L")?.into()),
            "-l" => libs.push(val("-l")?),
            "-z" | "-m" | "-plugin" | "-plugin-opt" | "--version-script" | "-soname" => {
                val(&a)?;
            }
            _ if a.starts_with("-L") => libdirs.push(a[2..].into()),
            _ if a.starts_with("-l") => libs.push(a[2..].into()),
            _ if a.starts_with("--entry=") => opts.entry = a[8..].into(),
            _ if a.starts_with("-plugin-opt=mcpu=") => cpu = a["-plugin-opt=mcpu=".len()..].into(),
            _ if a.starts_with("-plugin-opt=O") => opt_level = a["-plugin-opt=O".len()..].into(),
            _ if a.starts_with('-') => {} // rustc's --as-needed, -Bstatic, --strip-debug, ...
            _ => inputs.push(a.into()),
        }
    }
    for l in &libs {
        let name = format!("lib{l}.a");
        let path = libdirs.iter().map(|d| d.join(&name)).find(|p| p.exists());
        inputs.push(path.ok_or_else(|| format!("cannot find -l{l}"))?);
    }
    let out = out.ok_or("no output file (-o)")?;
    if let Ok(o) = std::env::var("AZLD_OPT") {
        opt_level = o; // debugging aid: force llc's optimisation level
    }
    let mut codegen = |name: &str, bc: &[u8]| llc(name, bc, &cpu, &opt_level);
    let mut linker = azld::Linker::new();
    linker.set_bitcode_handler(&mut codegen);
    for p in &inputs {
        let data = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        linker.add_file(&p.display().to_string(), data)?;
    }
    let (bytes, lmap) = linker.link(&opts)?;
    std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    if let Some(m) = map {
        std::fs::write(&m, azld::format_map(&lmap)).map_err(|e| format!("{}: {e}", m.display()))?;
    }
    Ok(())
}

/// Compile one LLVM bitcode module to an m68k ELF object with llc (cached).
fn llc(name: &str, bc: &[u8], cpu: &str, opt: &str) -> Result<Vec<u8>, String> {
    let llc = std::env::var("AZLD_LLC").map_err(|_| format!("{name}: is LLVM bitcode; set AZLD_LLC to an llc with the M68k backend"))?;
    let cache = std::env::var_os("AZLD_CACHE").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("azld-cache"));
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let args = [
        "-mtriple=m68k-unknown-none-elf".to_string(),
        format!("-mcpu={cpu}"),
        format!("-O{opt}"),
        "-relocation-model=static".into(),
        "-code-model=large".into(),
        "-function-sections".into(),
        "-data-sections".into(),
        "-filetype=obj".into(),
    ];
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bc.hash(&mut h);
    args.hash(&mut h);
    llc.hash(&mut h);
    // a rebuilt llc must not reuse objects from the old one
    if let Ok(m) = std::fs::metadata(&llc) {
        m.len().hash(&mut h);
        m.modified().ok().hash(&mut h);
    }
    let key = format!("{:016x}", h.finish());
    let obj = cache.join(format!("{key}.o"));
    if let Ok(o) = std::fs::read(&obj) {
        return Ok(o);
    }
    let pid = std::process::id();
    let input = cache.join(format!("{key}.{pid}.bc"));
    let tmp = cache.join(format!("{key}.{pid}.o"));
    std::fs::write(&input, bc).map_err(|e| e.to_string())?;
    let out = Command::new(&llc)
        .args(&args)
        .arg("-o")
        .arg(&tmp)
        .arg(&input)
        .output()
        .map_err(|e| format!("cannot run {llc}: {e}"))?;
    let _ = std::fs::remove_file(&input);
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("llc failed on {name}:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    std::fs::rename(&tmp, &obj).map_err(|e| e.to_string())?;
    std::fs::read(&obj).map_err(|e| e.to_string())
}
