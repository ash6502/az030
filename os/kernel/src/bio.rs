//! Block I/O: the buffer cache for 1 KB file-system blocks, with write-back of dirty
//! blocks (every few seconds and on sync).

use crate::scsi;
use crate::util::Global;
use crate::vm::KResult;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use azsys::errno::*;

pub const BSIZE: usize = 1024;
const SECTORS: u32 = (BSIZE as u32) / scsi::BLOCK;

/// Where a block device's blocks live.
#[derive(Clone, Copy)]
pub struct BlockDev {
    pub scsi_id: u32,
    /// First LBA of the device (partition) on the disk.
    pub lba: u32,
    /// Size in 1 KB blocks.
    pub blocks: u32,
    pub read_only: bool,
}

struct Buf {
    dev: u32,
    block: u32,
    data: Box<[u8; BSIZE]>,
    valid: bool,
    dirty: bool,
    refs: u32,
    used: u64,
}

struct Cache {
    bufs: Vec<Buf>,
    map: BTreeMap<(u32, u32), usize>,
    clock: u64,
    devs: BTreeMap<u32, BlockDev>,
}

static CACHE: Global<Cache> = Global::new(Cache { bufs: Vec::new(), map: BTreeMap::new(), clock: 0, devs: BTreeMap::new() });

pub fn init(ram: u32) {
    let n = ((ram / 128) as usize / BSIZE).clamp(128, 4096);
    let c = CACHE.get();
    for _ in 0..n {
        c.bufs.push(Buf { dev: 0, block: 0, data: Box::new([0; BSIZE]), valid: false, dirty: false, refs: 0, used: 0 });
    }
    kprintln!("bio: {} KB buffer cache", n * BSIZE / 1024);
}

pub fn register(dev: u32, bd: BlockDev) {
    CACHE.get().devs.insert(dev, bd);
}

pub fn unregister(dev: u32) {
    sync_dev(dev);
    let c = CACHE.get();
    for b in c.bufs.iter_mut() {
        if b.valid && b.dev == dev {
            b.valid = false;
            c.map.remove(&(dev, b.block));
        }
    }
    c.devs.remove(&dev);
}

pub fn device(dev: u32) -> Option<BlockDev> {
    CACHE.get().devs.get(&dev).copied()
}

pub fn cached_kb() -> u32 {
    (CACHE.get().bufs.iter().filter(|b| b.valid).count() * BSIZE / 1024) as u32
}

fn io(dev: u32, block: u32, data: &mut [u8; BSIZE], write: bool) -> KResult<()> {
    let bd = CACHE.get().devs.get(&dev).copied().ok_or(ENXIO)?;
    if block >= bd.blocks {
        kprintln!("bio: block {} beyond end of device {:x}", block, dev);
        return Err(EIO);
    }
    let lba = bd.lba + block * SECTORS;
    let r = if write { scsi::write(bd.scsi_id, lba, SECTORS, &data[..]) } else { scsi::read(bd.scsi_id, lba, SECTORS, &mut data[..]) };
    r.map_err(|st| {
        kprintln!("bio: sd{} {} error at block {} (status {:x})", bd.scsi_id, if write { "write" } else { "read" }, block, st);
        EIO
    })
}

/// A referenced cache buffer.
pub struct BufRef {
    idx: usize,
}

impl BufRef {
    pub fn data(&self) -> &[u8; BSIZE] {
        &CACHE.get().bufs[self.idx].data
    }

    /// Mutable access; marks the block dirty.
    pub fn data_mut(&self) -> &mut [u8; BSIZE] {
        let b = &mut CACHE.get().bufs[self.idx];
        b.dirty = true;
        &mut b.data
    }

    pub fn block(&self) -> u32 {
        CACHE.get().bufs[self.idx].block
    }
}

impl Drop for BufRef {
    fn drop(&mut self) {
        CACHE.get().bufs[self.idx].refs -= 1;
    }
}

fn slot(dev: u32, block: u32) -> KResult<usize> {
    let c = CACHE.get();
    c.clock += 1;
    if let Some(&i) = c.map.get(&(dev, block)) {
        c.bufs[i].refs += 1;
        c.bufs[i].used = c.clock;
        return Ok(i);
    }
    // least recently used free buffer
    let i = (0..c.bufs.len())
        .filter(|&i| c.bufs[i].refs == 0)
        .min_by_key(|&i| if c.bufs[i].valid { c.bufs[i].used } else { 0 })
        .ok_or(ENOMEM)?;
    let b = &mut c.bufs[i];
    if b.valid {
        if b.dirty {
            let (d, blk) = (b.dev, b.block);
            io(d, blk, &mut b.data, true)?;
            b.dirty = false;
        }
        c.map.remove(&(b.dev, b.block));
    }
    let b = &mut c.bufs[i];
    b.dev = dev;
    b.block = block;
    b.valid = false;
    b.dirty = false;
    b.refs = 1;
    b.used = c.clock;
    c.map.insert((dev, block), i);
    Ok(i)
}

/// Get a block, reading it from disk if it is not cached.
pub fn read(dev: u32, block: u32) -> KResult<BufRef> {
    let i = slot(dev, block)?;
    let r = BufRef { idx: i };
    let b = &mut CACHE.get().bufs[i];
    if !b.valid {
        if let Err(e) = io(dev, block, &mut b.data, false) {
            CACHE.get().map.remove(&(dev, block));
            return Err(e);
        }
        b.valid = true;
    }
    Ok(r)
}

/// Get a block that is about to be completely overwritten (no read), zeroed.
pub fn zeroed(dev: u32, block: u32) -> KResult<BufRef> {
    let i = slot(dev, block)?;
    let b = &mut CACHE.get().bufs[i];
    b.data.fill(0);
    b.valid = true;
    b.dirty = true;
    Ok(BufRef { idx: i })
}

pub fn sync_dev(dev: u32) {
    let c = CACHE.get();
    for b in c.bufs.iter_mut() {
        if b.valid && b.dirty && b.dev == dev {
            if io(b.dev, b.block, &mut b.data, true).is_ok() {
                b.dirty = false;
            }
        }
    }
    if let Some(bd) = c.devs.get(&dev) {
        scsi::sync(bd.scsi_id);
    }
}

/// Write back every dirty buffer.
pub fn sync() {
    let devs: Vec<u32> = CACHE.get().devs.keys().copied().collect();
    for d in devs {
        sync_dev(d);
    }
}
