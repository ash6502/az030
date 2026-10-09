//! ELF32 big-endian structures and constants used by the linker, the assembler and the
//! kernel's program loader.

use alloc::vec::Vec;

pub const EM_68K: u16 = 4;
pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;

pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_REL: u32 = 9;
pub const SHT_INIT_ARRAY: u32 = 14;
pub const SHT_FINI_ARRAY: u32 = 15;
pub const SHT_PREINIT_ARRAY: u32 = 16;

pub const SHF_WRITE: u32 = 1;
pub const SHF_ALLOC: u32 = 2;
pub const SHF_EXECINSTR: u32 = 4;

pub const SHN_UNDEF: u16 = 0;
pub const SHN_LORESERVE: u16 = 0xFF00;
pub const SHN_ABS: u16 = 0xFFF1;
pub const SHN_COMMON: u16 = 0xFFF2;

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;

pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

pub const R_68K_NONE: u8 = 0;
pub const R_68K_32: u8 = 1;
pub const R_68K_16: u8 = 2;
pub const R_68K_8: u8 = 3;
pub const R_68K_PC32: u8 = 4;
pub const R_68K_PC16: u8 = 5;
pub const R_68K_PC8: u8 = 6;
pub const R_68K_PLT32: u8 = 13;
pub const R_68K_PLT16: u8 = 14;
pub const R_68K_PLT8: u8 = 15;

pub fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}

pub fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Ehdr {
    pub class: u8,
    pub data: u8,
    pub etype: u16,
    pub machine: u16,
    pub entry: u32,
    pub phoff: u32,
    pub shoff: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

impl Ehdr {
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.len() < 52 || &d[..4] != b"\x7fELF" {
            return None;
        }
        Some(Ehdr {
            class: d[4],
            data: d[5],
            etype: be16(d, 16),
            machine: be16(d, 18),
            entry: be32(d, 24),
            phoff: be32(d, 28),
            shoff: be32(d, 32),
            ehsize: be16(d, 40),
            phentsize: be16(d, 42),
            phnum: be16(d, 44),
            shentsize: be16(d, 46),
            shnum: be16(d, 48),
            shstrndx: be16(d, 50),
        })
    }

    pub fn write(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&[0x7F, b'E', b'L', b'F', self.class, self.data, 1, 0]);
        v.extend_from_slice(&[0; 8]);
        put16(v, self.etype);
        put16(v, self.machine);
        put32(v, 1);
        put32(v, self.entry);
        put32(v, self.phoff);
        put32(v, self.shoff);
        put32(v, 0);
        put16(v, self.ehsize);
        put16(v, self.phentsize);
        put16(v, self.phnum);
        put16(v, self.shentsize);
        put16(v, self.shnum);
        put16(v, self.shstrndx);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Shdr {
    pub name: u32,
    pub stype: u32,
    pub flags: u32,
    pub addr: u32,
    pub offset: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub addralign: u32,
    pub entsize: u32,
}

impl Shdr {
    pub fn parse(d: &[u8], o: usize) -> Option<Self> {
        if o + 40 > d.len() {
            return None;
        }
        Some(Shdr {
            name: be32(d, o),
            stype: be32(d, o + 4),
            flags: be32(d, o + 8),
            addr: be32(d, o + 12),
            offset: be32(d, o + 16),
            size: be32(d, o + 20),
            link: be32(d, o + 24),
            info: be32(d, o + 28),
            addralign: be32(d, o + 32),
            entsize: be32(d, o + 36),
        })
    }

    pub fn write(&self, v: &mut Vec<u8>) {
        for x in [
            self.name, self.stype, self.flags, self.addr, self.offset, self.size, self.link, self.info,
            self.addralign, self.entsize,
        ] {
            put32(v, x);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sym {
    pub name: u32,
    pub value: u32,
    pub size: u32,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
}

impl Sym {
    pub fn parse(d: &[u8], o: usize) -> Option<Self> {
        if o + 16 > d.len() {
            return None;
        }
        Some(Sym {
            name: be32(d, o),
            value: be32(d, o + 4),
            size: be32(d, o + 8),
            info: d[o + 12],
            other: d[o + 13],
            shndx: be16(d, o + 14),
        })
    }

    pub fn write(&self, v: &mut Vec<u8>) {
        put32(v, self.name);
        put32(v, self.value);
        put32(v, self.size);
        v.push(self.info);
        v.push(self.other);
        put16(v, self.shndx);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Phdr {
    pub ptype: u32,
    pub offset: u32,
    pub vaddr: u32,
    pub paddr: u32,
    pub filesz: u32,
    pub memsz: u32,
    pub flags: u32,
    pub align: u32,
}

impl Phdr {
    pub fn parse(d: &[u8], o: usize) -> Option<Self> {
        if o + 32 > d.len() {
            return None;
        }
        Some(Phdr {
            ptype: be32(d, o),
            offset: be32(d, o + 4),
            vaddr: be32(d, o + 8),
            paddr: be32(d, o + 12),
            filesz: be32(d, o + 16),
            memsz: be32(d, o + 20),
            flags: be32(d, o + 24),
            align: be32(d, o + 28),
        })
    }

    pub fn write(&self, v: &mut Vec<u8>) {
        for x in [self.ptype, self.offset, self.vaddr, self.paddr, self.filesz, self.memsz, self.flags, self.align] {
            put32(v, x);
        }
    }
}
