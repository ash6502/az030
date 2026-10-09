//! SCSI host adapter driver (the DMA adapter at 0xFE002000, see emu/README.md).
//!
//! Commands are issued and polled to completion; DMA uses physical addresses, which
//! are the kernel's own addresses.

use crate::arch;
use crate::util::Global;
use alloc::vec;

const BASE: u32 = 0xFE00_2000;
const CMD: u32 = BASE;
const STATUS: u32 = BASE + 0x04;
const TARGET: u32 = BASE + 0x08;
const DMA_ADDR: u32 = BASE + 0x0C;
const DMA_LEN: u32 = BASE + 0x10;
const CDB_LEN: u32 = BASE + 0x18;
const ID: u32 = BASE + 0x1C;
const CDB: u32 = BASE + 0x20;
const ID_MAGIC: u32 = 0x5343_5349;

const ST_DONE: u32 = 1 << 1;
const ST_ERRORS: u32 = 0xFF0C; // SCSI status byte | NO TARGET | DMA ERROR

pub const BLOCK: u32 = 512;
pub const HOST_ID: u32 = 7;

#[derive(Clone, Copy, Default)]
pub struct Disk {
    pub present: bool,
    pub blocks: u32,
    pub read_only: bool,
}

static DISKS: Global<[Disk; 8]> = Global::new([Disk { present: false, blocks: 0, read_only: false }; 8]);
static PRESENT: Global<bool> = Global::new(false);

fn exec(id: u32, cdb: &[u8], buf: u32, len: u32) -> Result<(), u32> {
    let mut c = [0u8; 16];
    c[..cdb.len()].copy_from_slice(cdb);
    arch::wr(TARGET, id & 7);
    arch::wr(DMA_ADDR, buf);
    arch::wr(DMA_LEN, len);
    arch::wr(CDB_LEN, cdb.len() as u32);
    for i in 0..4 {
        arch::wr(CDB + i * 4, u32::from_be_bytes(c[i as usize * 4..i as usize * 4 + 4].try_into().unwrap()));
    }
    arch::wr(CMD, 1);
    loop {
        let st = arch::rd(STATUS);
        if st & ST_DONE != 0 {
            return if st & ST_ERRORS != 0 { Err(st) } else { Ok(()) };
        }
    }
}

pub fn init() {
    if arch::probe(ID) != Some(ID_MAGIC) {
        kprintln!("scsi: no host adapter");
        return;
    }
    *PRESENT.get() = true;
    arch::wr(CMD, 2); // bus reset
    let buf = vec![0u8; 36];
    let disks = DISKS.get();
    for id in 0..8 {
        if id == HOST_ID {
            continue;
        }
        if exec(id, &[0x12, 0, 0, 0, 36, 0], buf.as_ptr() as u32, 36).is_err() || buf[0] & 0x1F != 0 {
            continue;
        }
        let cap = vec![0u8; 8];
        if exec(id, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], cap.as_ptr() as u32, 8).is_err() {
            continue;
        }
        let blocks = u32::from_be_bytes(cap[0..4].try_into().unwrap()) + 1;
        let mode = vec![0u8; 12];
        let ro = exec(id, &[0x1A, 0, 0, 0, 12, 0], mode.as_ptr() as u32, 12).is_ok() && mode[2] & 0x80 != 0;
        disks[id as usize] = Disk { present: true, blocks, read_only: ro };
        let vendor = crate::util::cstr(&buf[8..16]).trim();
        let product = crate::util::cstr(&buf[16..32]).trim();
        kprintln!(
            "scsi: sd{}: {} {}, {} MB{}",
            id,
            vendor,
            product,
            blocks / 2048,
            if ro { ", read-only" } else { "" }
        );
    }
}

pub fn disk(id: u32) -> Option<Disk> {
    DISKS.get().get(id as usize).copied().filter(|d| d.present)
}

/// Read `count` 512-byte blocks at `lba` into `buf` (physical/kernel address).
pub fn read(id: u32, lba: u32, count: u32, buf: &mut [u8]) -> Result<(), u32> {
    rw(id, lba, count, buf.as_mut_ptr() as u32, buf.len() as u32, false)
}

pub fn write(id: u32, lba: u32, count: u32, buf: &[u8]) -> Result<(), u32> {
    rw(id, lba, count, buf.as_ptr() as u32, buf.len() as u32, true)
}

fn rw(id: u32, lba: u32, count: u32, addr: u32, len: u32, write: bool) -> Result<(), u32> {
    if !*PRESENT.get() || count * BLOCK > len {
        return Err(u32::MAX);
    }
    let op = if write { 0x2A } else { 0x28 };
    let l = lba.to_be_bytes();
    let n = (count as u16).to_be_bytes();
    exec(id, &[op, 0, l[0], l[1], l[2], l[3], 0, n[0], n[1], 0], addr, count * BLOCK)
}

/// Flush the drive's write cache.
pub fn sync(id: u32) {
    let _ = exec(id, &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], 0, 0);
}
