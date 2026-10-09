//! azld: a small static linker for big-endian ELF32 m68k objects.
//!
//! It reads relocatable objects (`ET_REL`) and `ar` archives (including Rust rlibs),
//! resolves symbols, optionally garbage-collects unreferenced sections, lays out
//! text / rodata / data / bss, applies `R_68K_*` relocations and writes either a
//! static ELF executable or a flat binary.
//!
//! The crate is `no_std` + `alloc` so the same code runs on the build host and as
//! `/bin/ld` on the az030.

#![no_std]

extern crate alloc;

pub mod archive;
pub mod elf;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use elf::*;

pub type Result<T> = core::result::Result<T, String>;

/// Output file format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// Static `ET_EXEC` with two `PT_LOAD` segments (RX text+rodata, RW data+bss).
    Elf,
    /// Raw image of text+rodata+data starting at the base address (bss not stored).
    Binary,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Address of the first byte of text.
    pub base: u32,
    /// Entry symbol.
    pub entry: String,
    pub format: Format,
    /// Drop sections nothing references (roots: entry, `keep` symbols, `.vectors*`/`.init*`).
    pub gc_sections: bool,
    /// Extra GC roots.
    pub keep: Vec<String>,
    /// Put data on its own page (ELF executables, so text can be mapped read-only).
    pub page_align_data: bool,
    /// Include a symbol table in ELF output.
    pub symbols: bool,
    /// Accept undefined symbols (treated as 0) instead of failing.
    pub allow_undefined: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            base: 0x4000_0000,
            entry: "_start".into(),
            format: Format::Elf,
            gc_sections: true,
            keep: Vec::new(),
            page_align_data: true,
            symbols: true,
            allow_undefined: false,
        }
    }
}

pub const PAGE: u32 = 4096;

/// One parsed input object.
struct Object {
    name: String,
    /// Came out of an archive: its definitions yield to those in plain objects,
    /// like a classic linker that only pulls archive members in when needed.
    from_archive: bool,
    data: Vec<u8>,
    sections: Vec<Shdr>,
    names: Vec<String>,
    syms: Vec<Sym>,
    sym_names: Vec<String>,
    /// rela sections that apply to each section index.
    relocs: Vec<Vec<usize>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Text = 0,
    Rodata = 1,
    Data = 2,
    Bss = 3,
}

#[derive(Clone, Debug)]
enum Def {
    Sec { obj: usize, sec: usize, value: u32, weak: bool },
    Abs { value: u32, weak: bool },
    Common { size: u32, align: u32 },
    /// Defined by the linker after layout.
    Linker,
}

const LINKER_SYMS: &[&str] = &[
    "__text_start", "_etext", "__etext", "__rodata_start", "__data_start", "_edata",
    "__bss_start", "__bss_end", "_end", "end", "__end",
];

/// Turns an LLVM bitcode object into an ELF object (the host front end runs llc).
pub type BitcodeHandler<'a> = &'a mut dyn FnMut(&str, &[u8]) -> Result<Vec<u8>>;

/// Collects inputs and links them.
pub struct Linker<'a> {
    objects: Vec<Object>,
    bitcode: Option<BitcodeHandler<'a>>,
}

impl Default for Linker<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// LLVM bitcode, raw or in its wrapper header.
pub fn is_bitcode(d: &[u8]) -> bool {
    d.starts_with(b"BC\xC0\xDE") || d.starts_with(&[0xDE, 0xC0, 0x17, 0x0B])
}

/// Address map produced by a link, for `--map` output.
pub struct LinkMap {
    pub symbols: Vec<(u32, String)>,
    pub text: (u32, u32),
    pub rodata: (u32, u32),
    pub data: (u32, u32),
    pub bss: (u32, u32),
    pub entry: u32,
}

impl<'a> Linker<'a> {
    pub fn new() -> Self {
        Linker { objects: Vec::new(), bitcode: None }
    }

    /// Accept LLVM bitcode inputs, compiling them with `h`.
    pub fn set_bitcode_handler(&mut self, h: BitcodeHandler<'a>) {
        self.bitcode = Some(h);
    }

    fn add_bitcode(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let h = self.bitcode.as_mut().ok_or_else(|| format!("{name}: LLVM bitcode input needs a code generator"))?;
        let obj = h(name, data)?;
        self.add_object(name, obj)
    }

    /// Add an object file or an archive. Non-ELF archive members (e.g. `lib.rmeta`) are skipped.
    pub fn add_file(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        if archive::is_archive(&data) {
            for m in archive::members(&data)? {
                let n = format!("{}({})", name, m.name);
                if is_elf(m.data) {
                    self.add_object(&n, m.data.to_vec())?;
                } else if is_bitcode(m.data) {
                    self.add_bitcode(&n, m.data)?;
                } else {
                    continue;
                }
                self.objects.last_mut().unwrap().from_archive = true;
            }
            Ok(())
        } else if is_elf(&data) {
            self.add_object(name, data)
        } else if is_bitcode(&data) {
            self.add_bitcode(name, &data)
        } else {
            Err(format!("{name}: not an ELF object or archive"))
        }
    }

    fn add_object(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        let eh = Ehdr::parse(&data).ok_or_else(|| format!("{name}: bad ELF header"))?;
        if eh.class != 1 || eh.data != 2 {
            return Err(format!("{name}: not a big-endian ELF32 file"));
        }
        if eh.machine != EM_68K {
            return Err(format!("{name}: not an m68k object (e_machine {})", eh.machine));
        }
        if eh.etype != ET_REL {
            return Err(format!("{name}: not a relocatable object"));
        }
        let mut sections = Vec::with_capacity(eh.shnum as usize);
        for i in 0..eh.shnum as usize {
            let off = eh.shoff as usize + i * eh.shentsize as usize;
            sections.push(Shdr::parse(&data, off).ok_or_else(|| format!("{name}: bad section header"))?);
        }
        let shstr = sections.get(eh.shstrndx as usize).copied();
        let names = sections
            .iter()
            .map(|s| match shstr {
                Some(t) => cstr_at(&data, t.offset as usize + s.name as usize),
                None => String::new(),
            })
            .collect();
        let mut syms = Vec::new();
        let mut sym_names = Vec::new();
        let mut relocs = vec![Vec::new(); sections.len()];
        for (i, s) in sections.iter().enumerate() {
            match s.stype {
                SHT_SYMTAB => {
                    let strtab = sections.get(s.link as usize).ok_or("bad symtab link")?;
                    let n = s.size as usize / 16;
                    for k in 0..n {
                        let sym = Sym::parse(&data, s.offset as usize + k * 16)
                            .ok_or_else(|| format!("{name}: bad symbol"))?;
                        sym_names.push(cstr_at(&data, strtab.offset as usize + sym.name as usize));
                        syms.push(sym);
                    }
                }
                SHT_RELA | SHT_REL => {
                    if let Some(v) = relocs.get_mut(s.info as usize) {
                        v.push(i);
                    }
                }
                _ => {}
            }
        }
        self.objects.push(Object { name: name.into(), from_archive: false, data, sections, names, syms, sym_names, relocs });
        Ok(())
    }

    fn kind(o: &Object, i: usize) -> Option<Kind> {
        let s = &o.sections[i];
        if s.flags & SHF_ALLOC == 0 {
            return None;
        }
        let name = o.names[i].as_str();
        if name.starts_with(".eh_frame") || name.starts_with(".gcc_except_table") {
            return None;
        }
        match s.stype {
            SHT_NOBITS => Some(Kind::Bss),
            SHT_PROGBITS | SHT_INIT_ARRAY | SHT_FINI_ARRAY | SHT_PREINIT_ARRAY => {
                if s.flags & SHF_EXECINSTR != 0 {
                    Some(Kind::Text)
                } else if s.flags & SHF_WRITE != 0 {
                    Some(Kind::Data)
                } else {
                    Some(Kind::Rodata)
                }
            }
            _ => None,
        }
    }

    fn build_symtab(&self) -> Result<BTreeMap<String, Def>> {
        let mut globals: BTreeMap<String, Def> = BTreeMap::new();
        let mut errors = Vec::new();
        for (oi, o) in self.objects.iter().enumerate() {
            for (si, s) in o.syms.iter().enumerate() {
                let bind = s.info >> 4;
                if si == 0 || bind == STB_LOCAL || s.shndx == SHN_UNDEF {
                    continue;
                }
                let name = &o.sym_names[si];
                let weak = bind == STB_WEAK;
                let def = match s.shndx {
                    SHN_ABS => Def::Abs { value: s.value, weak },
                    SHN_COMMON => Def::Common { size: s.size, align: s.value.max(1) },
                    sec => Def::Sec { obj: oi, sec: sec as usize, value: s.value, weak },
                };
                match globals.get(name) {
                    None => {
                        globals.insert(name.clone(), def);
                    }
                    Some(old) => {
                        // a plain object's definition beats an archive member's
                        let old_arch = matches!(old, Def::Sec { obj, .. } if self.objects[*obj].from_archive);
                        if old_arch && !o.from_archive && !weak && !matches!(def, Def::Common { .. }) {
                            globals.insert(name.clone(), def);
                            continue;
                        }
                        if o.from_archive && matches!(old, Def::Sec { obj, .. } if !self.objects[*obj].from_archive) {
                            continue;
                        }
                        let old_weak = matches!(old, Def::Sec { weak: true, .. } | Def::Abs { weak: true, .. });
                        match (old, &def) {
                            (Def::Common { size: a, align: x }, Def::Common { size: b, align: y }) => {
                                let d = Def::Common { size: (*a).max(*b), align: (*x).max(*y) };
                                globals.insert(name.clone(), d);
                            }
                            (Def::Common { .. }, _) if !weak => {
                                globals.insert(name.clone(), def);
                            }
                            (_, Def::Common { .. }) => {}
                            _ if old_weak && !weak => {
                                globals.insert(name.clone(), def);
                            }
                            _ if weak => {}
                            _ => errors.push(format!("duplicate symbol `{}` in {}", name, o.name)),
                        }
                    }
                }
            }
        }
        for n in LINKER_SYMS {
            globals.entry((*n).into()).or_insert(Def::Linker);
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        Ok(globals)
    }

    /// Resolve symbol `si` of object `oi` to the section it lives in (for GC).
    fn sym_section(&self, globals: &BTreeMap<String, Def>, oi: usize, si: usize) -> Option<(usize, usize)> {
        let o = &self.objects[oi];
        let s = o.syms.get(si)?;
        if s.info >> 4 == STB_LOCAL {
            if s.shndx == SHN_UNDEF || s.shndx >= SHN_LORESERVE {
                return None;
            }
            return Some((oi, s.shndx as usize));
        }
        match globals.get(&o.sym_names[si])? {
            Def::Sec { obj, sec, .. } => Some((*obj, *sec)),
            _ => None,
        }
    }

    /// Link everything. Returns the output file and an address map.
    pub fn link(&self, opts: &Options) -> Result<(Vec<u8>, LinkMap)> {
        let globals = self.build_symtab()?;

        // ---- liveness ------------------------------------------------------------
        let mut live: Vec<Vec<bool>> = self
            .objects
            .iter()
            .map(|o| (0..o.sections.len()).map(|i| !opts.gc_sections && Self::kind(o, i).is_some()).collect())
            .collect();
        if opts.gc_sections {
            let mut work: Vec<(usize, usize)> = Vec::new();
            let root = |name: &str, work: &mut Vec<(usize, usize)>| {
                if let Some(Def::Sec { obj, sec, .. }) = globals.get(name) {
                    work.push((*obj, *sec));
                }
            };
            root(&opts.entry, &mut work);
            for k in &opts.keep {
                root(k, &mut work);
            }
            for (oi, o) in self.objects.iter().enumerate() {
                for (i, n) in o.names.iter().enumerate() {
                    if (n.starts_with(".vectors") || n.starts_with(".init") || n.starts_with(".keep"))
                        && Self::kind(o, i).is_some()
                    {
                        work.push((oi, i));
                    }
                }
            }
            while let Some((oi, si)) = work.pop() {
                if live[oi][si] || Self::kind(&self.objects[oi], si).is_none() {
                    continue;
                }
                live[oi][si] = true;
                let o = &self.objects[oi];
                for &ri in &o.relocs[si] {
                    for r in rel_entries(o, ri) {
                        if let Some(t) = self.sym_section(&globals, oi, r.sym as usize) {
                            if !live[t.0][t.1] {
                                work.push(t);
                            }
                        }
                    }
                }
            }
        }

        // ---- layout -------------------------------------------------------------------
        let mut addr: Vec<Vec<Option<u32>>> = self.objects.iter().map(|o| vec![None; o.sections.len()]).collect();
        let mut pc = opts.base;
        let mut bounds = [(0u32, 0u32); 4];
        let mut commons: BTreeMap<String, u32> = BTreeMap::new();
        for kind in [Kind::Text, Kind::Rodata, Kind::Data, Kind::Bss] {
            if kind == Kind::Data && opts.page_align_data {
                pc = align(pc, PAGE);
            }
            if kind == Kind::Rodata || kind == Kind::Data {
                pc = align(pc, 16);
            }
            let start = pc;
            for (oi, o) in self.objects.iter().enumerate() {
                for i in 0..o.sections.len() {
                    if live[oi][i] && Self::kind(o, i) == Some(kind) {
                        let s = &o.sections[i];
                        pc = align(pc, s.addralign.max(1));
                        addr[oi][i] = Some(pc);
                        pc = pc.checked_add(s.size).ok_or("address space overflow")?;
                    }
                }
            }
            if kind == Kind::Bss {
                for (name, def) in &globals {
                    if let Def::Common { size, align: a } = def {
                        pc = align(pc, *a);
                        commons.insert(name.clone(), pc);
                        pc += size;
                    }
                }
                pc = align(pc, 4);
            }
            bounds[kind as usize] = (start, pc);
        }
        let [text, rodata, data, bss] = bounds;

        let linker_sym = |name: &str| -> u32 {
            match name {
                "__text_start" => text.0,
                "_etext" | "__etext" => text.1,
                "__rodata_start" => rodata.0,
                "__data_start" => data.0,
                "_edata" => data.1,
                "__bss_start" => bss.0,
                _ => bss.1, // __bss_end, _end, end, __end
            }
        };

        // Address of a global by name.
        let global_addr = |name: &str| -> Option<u32> {
            match globals.get(name)? {
                Def::Sec { obj, sec, value, .. } => addr[*obj][*sec].map(|a| a + value),
                Def::Abs { value, .. } => Some(*value),
                Def::Common { .. } => commons.get(name).copied(),
                Def::Linker => Some(linker_sym(name)),
            }
        };

        // ---- section contents + relocation -------------------------------------
        let file_end = data.1.max(rodata.1).max(text.1);
        let mut image = vec![0u8; (file_end - opts.base) as usize];
        let mut undefined: Vec<String> = Vec::new();
        for (oi, o) in self.objects.iter().enumerate() {
            for i in 0..o.sections.len() {
                let Some(base) = addr[oi][i] else { continue };
                let s = &o.sections[i];
                if s.stype == SHT_NOBITS {
                    continue;
                }
                let off = (base - opts.base) as usize;
                let src = o
                    .data
                    .get(s.offset as usize..(s.offset + s.size) as usize)
                    .ok_or_else(|| format!("{}: section {} out of bounds", o.name, o.names[i]))?;
                image[off..off + src.len()].copy_from_slice(src);
            }
        }
        for (oi, o) in self.objects.iter().enumerate() {
            for i in 0..o.sections.len() {
                let Some(base) = addr[oi][i] else { continue };
                if o.sections[i].stype == SHT_NOBITS {
                    continue;
                }
                for &ri in &o.relocs[i] {
                    let is_rel = o.sections[ri].stype == SHT_REL;
                    for r in rel_entries(o, ri) {
                        let p = base + r.offset;
                        let si = r.sym as usize;
                        let sym = o.syms.get(si).ok_or_else(|| format!("{}: bad symbol index", o.name))?;
                        let s = if si == 0 {
                            0
                        } else if sym.info >> 4 == STB_LOCAL {
                            match sym.shndx {
                                SHN_ABS => sym.value,
                                SHN_UNDEF => 0,
                                sh => match addr[oi].get(sh as usize).copied().flatten() {
                                    Some(a) => a + sym.value,
                                    None => {
                                        return Err(format!(
                                            "{}: relocation against discarded section {}",
                                            o.name,
                                            o.names.get(sh as usize).map(String::as_str).unwrap_or("?")
                                        ));
                                    }
                                },
                            }
                        } else {
                            let name = &o.sym_names[si];
                            match global_addr(name) {
                                Some(a) => a,
                                None => {
                                    if sym.info >> 4 != STB_WEAK && !undefined.contains(name) {
                                        undefined.push(name.clone());
                                    }
                                    0
                                }
                            }
                        };
                        let off = (p - opts.base) as usize;
                        let a = if is_rel { implicit_addend(&image, off, r.rtype) } else { r.addend };
                        apply(&mut image, off, r.rtype, s, a, p).map_err(|e| {
                            format!("{}: {} (in {}+{:#x}, symbol `{}`)", o.name, e, o.names[i], r.offset, o.sym_names[si])
                        })?;
                    }
                }
            }
        }
        if !undefined.is_empty() && !opts.allow_undefined {
            undefined.sort();
            let list: Vec<String> = undefined.iter().map(|u| format!("  {u}")).collect();
            return Err(format!("undefined symbols:\n{}", list.join("\n")));
        }
        let entry = global_addr(&opts.entry).ok_or_else(|| format!("entry symbol `{}` not defined", opts.entry))?;

        // ---- symbol list ----------------------------------------------------------
        let mut symbols: Vec<(u32, String)> = Vec::new();
        for (oi, o) in self.objects.iter().enumerate() {
            for (si, s) in o.syms.iter().enumerate() {
                let t = s.info & 0xF;
                if si == 0 || s.shndx == SHN_UNDEF || s.shndx >= SHN_LORESERVE || (t != STT_FUNC && t != STT_OBJECT) {
                    continue;
                }
                if s.info >> 4 != STB_LOCAL {
                    // only the winning definition
                    if !matches!(globals.get(&o.sym_names[si]), Some(Def::Sec { obj, sec, .. }) if *obj == oi && *sec == s.shndx as usize) {
                        continue;
                    }
                }
                if let Some(a) = addr[oi][s.shndx as usize] {
                    symbols.push((a + s.value, o.sym_names[si].clone()));
                }
            }
        }
        for (n, a) in &commons {
            symbols.push((*a, n.clone()));
        }
        symbols.sort();
        let map = LinkMap { symbols, text, rodata, data, bss, entry };

        let out = match opts.format {
            Format::Binary => image,
            Format::Elf => write_exec(&image, opts, &map),
        };
        Ok((out, map))
    }
}

fn align(v: u32, a: u32) -> u32 {
    if a <= 1 { v } else { (v + a - 1) & !(a - 1) }
}

struct Reloc {
    offset: u32,
    sym: u32,
    rtype: u8,
    addend: i32,
}

fn rel_entries(o: &Object, ri: usize) -> impl Iterator<Item = Reloc> + '_ {
    let s = &o.sections[ri];
    let rela = s.stype == SHT_RELA;
    let ent = if rela { 12 } else { 8 };
    let n = s.size as usize / ent;
    (0..n).map(move |k| {
        let p = s.offset as usize + k * ent;
        let offset = be32(&o.data, p);
        let info = be32(&o.data, p + 4);
        let addend = if rela { be32(&o.data, p + 8) as i32 } else { 0 };
        Reloc { offset, sym: info >> 8, rtype: info as u8, addend }
    })
}

fn implicit_addend(img: &[u8], off: usize, t: u8) -> i32 {
    match t {
        R_68K_32 | R_68K_PC32 | R_68K_PLT32 => be32(img, off) as i32,
        R_68K_16 | R_68K_PC16 | R_68K_PLT16 => i16::from_be_bytes([img[off], img[off + 1]]) as i32,
        R_68K_8 | R_68K_PC8 | R_68K_PLT8 => img[off] as i8 as i32,
        _ => 0,
    }
}

fn apply(img: &mut [u8], off: usize, t: u8, s: u32, a: i32, p: u32) -> Result<()> {
    let abs = s.wrapping_add(a as u32);
    let rel = abs.wrapping_sub(p) as i32;
    match t {
        R_68K_NONE => {}
        R_68K_32 => img[off..off + 4].copy_from_slice(&abs.to_be_bytes()),
        R_68K_16 => {
            let v = abs as i32;
            if !(-32768..=65535).contains(&v) {
                return Err(format!("R_68K_16 value {abs:#x} out of range"));
            }
            img[off..off + 2].copy_from_slice(&(v as u16).to_be_bytes());
        }
        R_68K_8 => {
            let v = abs as i32;
            if !(-128..=255).contains(&v) {
                return Err(format!("R_68K_8 value {abs:#x} out of range"));
            }
            img[off] = v as u8;
        }
        R_68K_PC32 | R_68K_PLT32 => img[off..off + 4].copy_from_slice(&(rel as u32).to_be_bytes()),
        R_68K_PC16 | R_68K_PLT16 => {
            if !(-32768..=32767).contains(&rel) {
                return Err(format!("PC16 displacement {rel} out of range"));
            }
            img[off..off + 2].copy_from_slice(&(rel as i16).to_be_bytes());
        }
        R_68K_PC8 | R_68K_PLT8 => {
            if !(-128..=127).contains(&rel) {
                return Err(format!("PC8 displacement {rel} out of range"));
            }
            img[off] = rel as i8 as u8;
        }
        _ => return Err(format!("unsupported relocation type {t} (build with -C relocation-model=static)")),
    }
    Ok(())
}

/// Write a static ELF executable. Text+rodata go in one RX segment at `base`,
/// data+bss in one RW segment.
fn write_exec(image: &[u8], opts: &Options, map: &LinkMap) -> Vec<u8> {
    let base = opts.base;
    let rx_end = map.rodata.1.max(map.text.1);
    let has_rw = map.bss.1 > map.data.0;
    let phnum: u16 = if has_rw { 2 } else { 1 };
    let hdr_end = 52 + 32 * phnum as u32;
    let text_off = align(hdr_end, 16);
    let mut out = vec![0u8; text_off as usize];
    out.extend_from_slice(&image[..(rx_end - base) as usize]);
    let data_off = align(out.len() as u32, 16);
    out.resize(data_off as usize, 0);
    if has_rw {
        out.extend_from_slice(&image[(map.data.0 - base) as usize..(map.data.1 - base) as usize]);
    }

    // Section headers: null, .text, .rodata, .data, .bss, [.symtab, .strtab], .shstrtab
    let mut shstr = vec![0u8];
    let mut name = |n: &str| {
        let o = shstr.len() as u32;
        shstr.extend_from_slice(n.as_bytes());
        shstr.push(0);
        o
    };
    let n_text = name(".text");
    let n_ro = name(".rodata");
    let n_data = name(".data");
    let n_bss = name(".bss");
    let n_sym = name(".symtab");
    let n_str = name(".strtab");
    let n_shs = name(".shstrtab");
    let mut sh: Vec<Shdr> = vec![Shdr::default()];
    sh.push(Shdr { name: n_text, stype: SHT_PROGBITS, flags: SHF_ALLOC | SHF_EXECINSTR, addr: map.text.0, offset: text_off + (map.text.0 - base), size: map.text.1 - map.text.0, addralign: 4, ..Default::default() });
    sh.push(Shdr { name: n_ro, stype: SHT_PROGBITS, flags: SHF_ALLOC, addr: map.rodata.0, offset: text_off + (map.rodata.0 - base), size: map.rodata.1 - map.rodata.0, addralign: 4, ..Default::default() });
    sh.push(Shdr { name: n_data, stype: SHT_PROGBITS, flags: SHF_ALLOC | SHF_WRITE, addr: map.data.0, offset: data_off, size: map.data.1 - map.data.0, addralign: 4, ..Default::default() });
    sh.push(Shdr { name: n_bss, stype: SHT_NOBITS, flags: SHF_ALLOC | SHF_WRITE, addr: map.bss.0, offset: data_off + (map.data.1 - map.data.0), size: map.bss.1 - map.bss.0, addralign: 4, ..Default::default() });
    if opts.symbols {
        let mut strtab = vec![0u8];
        let mut symtab = vec![0u8; 16];
        let section_of = |a: u32| -> u16 {
            if a >= map.bss.0 && a < map.bss.1 { 4 } else if a >= map.data.0 && a < map.data.1 { 3 } else if a >= map.rodata.0 && a < map.rodata.1 { 2 } else { 1 }
        };
        for (a, n) in &map.symbols {
            let no = strtab.len() as u32;
            strtab.extend_from_slice(n.as_bytes());
            strtab.push(0);
            let shn = section_of(*a);
            let typ = if shn == 1 { STT_FUNC } else { STT_OBJECT };
            symtab.extend_from_slice(&no.to_be_bytes());
            symtab.extend_from_slice(&a.to_be_bytes());
            symtab.extend_from_slice(&0u32.to_be_bytes());
            symtab.push((STB_GLOBAL << 4) | typ);
            symtab.push(0);
            symtab.extend_from_slice(&shn.to_be_bytes());
        }
        let so = align(out.len() as u32, 4);
        out.resize(so as usize, 0);
        out.extend_from_slice(&symtab);
        let st = out.len() as u32;
        out.extend_from_slice(&strtab);
        let idx = sh.len() as u32;
        sh.push(Shdr { name: n_sym, stype: SHT_SYMTAB, offset: so, size: symtab.len() as u32, link: idx + 1, info: 1, addralign: 4, entsize: 16, ..Default::default() });
        sh.push(Shdr { name: n_str, stype: SHT_STRTAB, offset: st, size: strtab.len() as u32, addralign: 1, ..Default::default() });
    }
    let shs_off = out.len() as u32;
    let shs_idx = sh.len() as u16;
    out.extend_from_slice(&shstr);
    sh.push(Shdr { name: n_shs, stype: SHT_STRTAB, offset: shs_off, size: shstr.len() as u32, addralign: 1, ..Default::default() });
    let shoff = align(out.len() as u32, 4);
    out.resize(shoff as usize, 0);
    for s in &sh {
        s.write(&mut out);
    }

    let eh = Ehdr {
        class: 1,
        data: 2,
        etype: ET_EXEC,
        machine: EM_68K,
        entry: map.entry,
        phoff: 52,
        shoff,
        ehsize: 52,
        phentsize: 32,
        phnum,
        shentsize: 40,
        shnum: sh.len() as u16,
        shstrndx: shs_idx,
    };
    let mut h = Vec::new();
    eh.write(&mut h);
    let rx = Phdr { ptype: PT_LOAD, offset: text_off, vaddr: base, paddr: base, filesz: rx_end - base, memsz: rx_end - base, flags: PF_R | PF_X, align: PAGE };
    rx.write(&mut h);
    if has_rw {
        let rw = Phdr { ptype: PT_LOAD, offset: data_off, vaddr: map.data.0, paddr: map.data.0, filesz: map.data.1 - map.data.0, memsz: map.bss.1 - map.data.0, flags: PF_R | PF_W, align: PAGE };
        rw.write(&mut h);
    }
    out[..h.len()].copy_from_slice(&h);
    out
}

/// Render a link map as text.
pub fn format_map(map: &LinkMap) -> String {
    let mut s = String::new();
    let line = |s: &mut String, n: &str, r: (u32, u32)| {
        s.push_str(&format!("{n:<8} {:08x}-{:08x} ({} bytes)\n", r.0, r.1, r.1 - r.0));
    };
    line(&mut s, ".text", map.text);
    line(&mut s, ".rodata", map.rodata);
    line(&mut s, ".data", map.data);
    line(&mut s, ".bss", map.bss);
    s.push_str(&format!("entry    {:08x}\n\n", map.entry));
    for (a, n) in &map.symbols {
        s.push_str(&format!("{a:08x} {n}\n"));
    }
    s
}

pub fn is_elf(d: &[u8]) -> bool {
    d.len() >= 52 && &d[..4] == b"\x7fELF"
}

pub fn cstr_at(d: &[u8], off: usize) -> String {
    let Some(rest) = d.get(off..) else { return String::new() };
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    String::from_utf8_lossy(&rest[..end]).to_string()
}
