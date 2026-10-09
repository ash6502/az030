//! azfs on-disk operations (layout in lib/azfs): allocation bitmaps, block mapping,
//! file data and directories. All I/O goes through the buffer cache.

use super::{mount_mut, Inode};
use crate::bio::{self, BSIZE};
use crate::timer;
use crate::vm::KResult;
use alloc::vec::Vec;
use azfs::*;
use azsys::errno::*;

const BITS: u32 = (BSIZE * 8) as u32;

pub fn read_dinode(dev: u32, ino: u32) -> KResult<DiskInode> {
    let sb = mount_mut(dev)?.sb;
    if ino == 0 || ino > sb.inodes {
        return Err(EIO);
    }
    let (blk, off) = sb.inode_pos(ino);
    let b = bio::read(dev, blk)?;
    Ok(DiskInode::decode(&b.data()[off..off + INODE_SIZE]))
}

pub fn write_dinode(dev: u32, ino: u32, di: &DiskInode) -> KResult<()> {
    let sb = mount_mut(dev)?.sb;
    let (blk, off) = sb.inode_pos(ino);
    let b = bio::read(dev, blk)?;
    di.encode(&mut b.data_mut()[off..off + INODE_SIZE]);
    Ok(())
}

/// Find and set a clear bit among the first `total` bits of the bitmap starting at
/// block `start`, searching from bit `hint` and wrapping around.
fn bitmap_alloc(dev: u32, start: u32, total: u32, hint: u32) -> KResult<Option<u32>> {
    let nblocks = total.div_ceil(BITS);
    let hint = if hint >= total { 0 } else { hint };
    for k in 0..=nblocks {
        let bi = (hint / BITS + k) % nblocks;
        let from = if k == 0 { hint % BITS } else { 0 };
        let b = bio::read(dev, start + bi)?;
        let data = b.data();
        let mut found = None;
        'scan: for byte in (from / 8) as usize..BSIZE {
            let v = data[byte];
            if v == 0xFF {
                continue;
            }
            for bit in 0..8u32 {
                let n = byte as u32 * 8 + bit;
                let g = bi * BITS + n;
                if g >= total {
                    break 'scan;
                }
                if n >= from && v & (0x80 >> bit) == 0 {
                    found = Some((byte, bit, g));
                    break 'scan;
                }
            }
        }
        if let Some((byte, bit, g)) = found {
            b.data_mut()[byte] |= 0x80 >> bit;
            return Ok(Some(g));
        }
    }
    Ok(None)
}

fn bitmap_clear(dev: u32, start: u32, n: u32) -> KResult<()> {
    let b = bio::read(dev, start + n / BITS)?;
    let i = n % BITS;
    b.data_mut()[(i / 8) as usize] &= !(0x80 >> (i % 8));
    Ok(())
}

/// Allocate a zeroed data block.
pub fn alloc_block(dev: u32) -> KResult<u32> {
    let m = mount_mut(dev)?;
    if m.read_only {
        return Err(EROFS);
    }
    let sb = m.sb;
    let blk = bitmap_alloc(dev, sb.bbitmap_start, sb.blocks, m.block_hint.max(sb.data_start))?.ok_or(ENOSPC)?;
    if blk < sb.data_start {
        // metadata area is pre-marked used; this would mean a corrupt bitmap
        return Err(EIO);
    }
    let m = mount_mut(dev)?;
    m.block_hint = blk + 1;
    m.sb.free_blocks = m.sb.free_blocks.saturating_sub(1);
    m.sb_dirty = true;
    drop(bio::zeroed(dev, blk)?);
    Ok(blk)
}

pub fn free_block(dev: u32, blk: u32) -> KResult<()> {
    let m = mount_mut(dev)?;
    let sb = m.sb;
    if blk < sb.data_start || blk >= sb.blocks {
        kprintln!("azfs: freeing bad block {}", blk);
        return Err(EIO);
    }
    bitmap_clear(dev, sb.bbitmap_start, blk)?;
    let m = mount_mut(dev)?;
    m.sb.free_blocks += 1;
    m.sb_dirty = true;
    if blk < m.block_hint {
        m.block_hint = blk;
    }
    Ok(())
}

pub fn alloc_inode(dev: u32, mode: u16, uid: u16, gid: u16) -> KResult<u32> {
    let m = mount_mut(dev)?;
    if m.read_only {
        return Err(EROFS);
    }
    let sb = m.sb;
    let ino = bitmap_alloc(dev, sb.ibitmap_start, sb.inodes + 1, 1)?.ok_or(ENOSPC)?;
    if ino == 0 {
        return Err(EIO);
    }
    let m = mount_mut(dev)?;
    m.sb.free_inodes = m.sb.free_inodes.saturating_sub(1);
    m.sb_dirty = true;
    let now = timer::now();
    let di = DiskInode { mode, nlink: 1, uid, gid, atime: now, mtime: now, ctime: now, ..Default::default() };
    write_dinode(dev, ino, &di)?;
    Ok(ino)
}

pub fn free_inode(dev: u32, ino: u32) -> KResult<()> {
    let sb = mount_mut(dev)?.sb;
    write_dinode(dev, ino, &DiskInode::default())?;
    bitmap_clear(dev, sb.ibitmap_start, ino)?;
    let m = mount_mut(dev)?;
    m.sb.free_inodes += 1;
    m.sb_dirty = true;
    Ok(())
}

fn ptr_slot(dev: u32, table: u32, i: usize, alloc: bool) -> KResult<u32> {
    let b = bio::read(dev, table)?;
    let v = block_ptr(b.data(), i);
    if v != 0 || !alloc {
        return Ok(v);
    }
    drop(b);
    let nb = alloc_block(dev)?;
    let b = bio::read(dev, table)?;
    set_block_ptr(b.data_mut(), i, nb);
    Ok(nb)
}

/// Disk block holding file block `n` (0 = hole). Allocates with `alloc`.
pub fn bmap(ip: &mut Inode, n: u32, alloc: bool) -> KResult<u32> {
    let dev = ip.dev;
    let n = n as usize;
    if n < NDIRECT {
        if ip.di.direct[n] == 0 && alloc {
            ip.di.direct[n] = alloc_block(dev)?;
            ip.dirty = true;
        }
        return Ok(ip.di.direct[n]);
    }
    let n = n - NDIRECT;
    let ppb = PTRS_PER_BLOCK as usize;
    if n < ppb {
        if ip.di.indirect == 0 {
            if !alloc {
                return Ok(0);
            }
            ip.di.indirect = alloc_block(dev)?;
            ip.dirty = true;
        }
        return ptr_slot(dev, ip.di.indirect, n, alloc);
    }
    let n = n - ppb;
    if n >= ppb * ppb {
        return Err(EFBIG);
    }
    if ip.di.dindirect == 0 {
        if !alloc {
            return Ok(0);
        }
        ip.di.dindirect = alloc_block(dev)?;
        ip.dirty = true;
    }
    let mid = ptr_slot(dev, ip.di.dindirect, n / ppb, alloc)?;
    if mid == 0 {
        return Ok(0);
    }
    ptr_slot(dev, mid, n % ppb, alloc)
}

/// Free the blocks listed in an indirect block from index `from` on (recursing
/// `depth` levels). Returns true if the whole table became empty.
fn free_table(dev: u32, table: u32, from: usize, depth: u32) -> KResult<bool> {
    let b = bio::read(dev, table)?;
    let ptrs: Vec<u32> = (0..PTRS_PER_BLOCK as usize).map(|i| block_ptr(b.data(), i)).collect();
    drop(b);
    let ppb = PTRS_PER_BLOCK as usize;
    for (i, &p) in ptrs.iter().enumerate() {
        if p == 0 {
            continue;
        }
        if depth == 0 {
            if i >= from {
                free_block(dev, p)?;
                set_block_ptr(bio::read(dev, table)?.data_mut(), i, 0);
            }
        } else {
            // child table i covers entries [i*ppb, (i+1)*ppb)
            let start = i * ppb;
            if start + ppb <= from {
                continue;
            }
            let sub_from = from.saturating_sub(start);
            if free_table(dev, p, sub_from, depth - 1)? {
                free_block(dev, p)?;
                set_block_ptr(bio::read(dev, table)?.data_mut(), i, 0);
            }
        }
    }
    let b = bio::read(dev, table)?;
    Ok((0..ppb).all(|i| block_ptr(b.data(), i) == 0))
}

/// Cut the file to `size` bytes, freeing whole blocks past the end.
pub fn truncate(ip: &mut Inode, size: u32) -> KResult<()> {
    let dev = ip.dev;
    let keep = size.div_ceil(BSIZE as u32) as usize;
    for i in keep..NDIRECT {
        if ip.di.direct[i] != 0 {
            free_block(dev, ip.di.direct[i])?;
            ip.di.direct[i] = 0;
        }
    }
    let ppb = PTRS_PER_BLOCK as usize;
    if ip.di.indirect != 0 {
        let from = keep.saturating_sub(NDIRECT);
        if free_table(dev, ip.di.indirect, from, 0)? {
            free_block(dev, ip.di.indirect)?;
            ip.di.indirect = 0;
        }
    }
    if ip.di.dindirect != 0 {
        let from = keep.saturating_sub(NDIRECT + ppb);
        if free_table(dev, ip.di.dindirect, from, 1)? {
            free_block(dev, ip.di.dindirect)?;
            ip.di.dindirect = 0;
        }
    }
    // zero the tail of the last partial block so a later extension reads zeros
    if size % BSIZE as u32 != 0 && size < ip.di.size {
        let blk = bmap(ip, size / BSIZE as u32, false)?;
        if blk != 0 {
            let b = bio::read(dev, blk)?;
            b.data_mut()[(size % BSIZE as u32) as usize..].fill(0);
        }
    }
    ip.di.size = size;
    ip.di.mtime = timer::now();
    ip.di.ctime = ip.di.mtime;
    ip.dirty = true;
    Ok(())
}

pub fn read(ip: &mut Inode, off: u32, buf: &mut [u8]) -> KResult<usize> {
    if off >= ip.di.size {
        return Ok(0);
    }
    let n = buf.len().min((ip.di.size - off) as usize);
    let mut done = 0;
    while done < n {
        let pos = off + done as u32;
        let bo = (pos % BSIZE as u32) as usize;
        let chunk = (BSIZE - bo).min(n - done);
        let blk = bmap(ip, pos / BSIZE as u32, false)?;
        if blk == 0 {
            buf[done..done + chunk].fill(0);
        } else {
            let b = bio::read(ip.dev, blk)?;
            buf[done..done + chunk].copy_from_slice(&b.data()[bo..bo + chunk]);
        }
        done += chunk;
    }
    Ok(n)
}

pub fn write(ip: &mut Inode, off: u32, data: &[u8]) -> KResult<usize> {
    let end = off.checked_add(data.len() as u32).ok_or(EFBIG)?;
    if end > MAX_FILE_BLOCKS * BSIZE as u32 {
        return Err(EFBIG);
    }
    let mut done = 0;
    while done < data.len() {
        let pos = off + done as u32;
        let bo = (pos % BSIZE as u32) as usize;
        let chunk = (BSIZE - bo).min(data.len() - done);
        let blk = match bmap(ip, pos / BSIZE as u32, true) {
            Ok(b) => b,
            Err(_) if done > 0 => break, // partial write (e.g. disk full)
            Err(e) => return Err(e),
        };
        let b = bio::read(ip.dev, blk)?;
        b.data_mut()[bo..bo + chunk].copy_from_slice(&data[done..done + chunk]);
        done += chunk;
    }
    let pos = off + done as u32;
    if pos > ip.di.size {
        ip.di.size = pos;
    }
    ip.di.mtime = timer::now();
    ip.di.ctime = ip.di.mtime;
    ip.dirty = true;
    Ok(done)
}

// ---- directories ----------------------------------------------------------------

/// Visit directory entries: f(offset, ino, name) returns true to stop.
pub fn dir_scan(ip: &mut Inode, mut f: impl FnMut(u32, u32, &[u8]) -> bool) -> KResult<()> {
    let size = ip.di.size;
    let mut off = 0u32;
    while off < size {
        let blk = bmap(ip, off / BSIZE as u32, false)?;
        if blk != 0 {
            let b = bio::read(ip.dev, blk)?;
            let data = b.data();
            for e in 0..DIRENTS_PER_BLOCK {
                let o = e * DIRENT_SIZE;
                let pos = off + o as u32;
                if pos >= size {
                    break;
                }
                let ino = dirent_ino(&data[o..]);
                if ino != 0 && f(pos, ino, dirent_name(&data[o..o + DIRENT_SIZE])) {
                    return Ok(());
                }
            }
        }
        off += BSIZE as u32;
    }
    Ok(())
}

pub fn dir_lookup(ip: &mut Inode, name: &[u8]) -> KResult<Option<(u32, u32)>> {
    let mut r = None;
    dir_scan(ip, |off, ino, n| {
        if n == name {
            r = Some((ino, off));
            true
        } else {
            false
        }
    })?;
    Ok(r)
}

pub fn dir_add(ip: &mut Inode, name: &[u8], ino: u32) -> KResult<()> {
    if name.len() > NAME_MAX {
        return Err(ENAMETOOLONG);
    }
    let mut ent = [0u8; DIRENT_SIZE];
    dirent_set(&mut ent, ino, name);
    // reuse a free slot if there is one
    let size = ip.di.size;
    let mut off = 0u32;
    while off < size {
        let blk = bmap(ip, off / BSIZE as u32, false)?;
        if blk != 0 {
            let b = bio::read(ip.dev, blk)?;
            for e in 0..DIRENTS_PER_BLOCK {
                let o = e * DIRENT_SIZE;
                if off + (o as u32) >= size {
                    break;
                }
                if dirent_ino(&b.data()[o..]) == 0 {
                    b.data_mut()[o..o + DIRENT_SIZE].copy_from_slice(&ent);
                    return Ok(());
                }
            }
        }
        off += BSIZE as u32;
    }
    write(ip, size, &ent)?;
    Ok(())
}

/// Overwrite the entry at `off` with `ino` (0 removes it).
pub fn dir_set(ip: &mut Inode, off: u32, ino: u32, name: &[u8]) -> KResult<()> {
    let blk = bmap(ip, off / BSIZE as u32, false)?;
    if blk == 0 {
        return Err(EIO);
    }
    let b = bio::read(ip.dev, blk)?;
    let o = (off % BSIZE as u32) as usize;
    if ino == 0 {
        b.data_mut()[o..o + DIRENT_SIZE].fill(0);
    } else {
        dirent_set(&mut b.data_mut()[o..o + DIRENT_SIZE], ino, name);
    }
    ip.di.mtime = timer::now();
    ip.di.ctime = ip.di.mtime;
    ip.dirty = true;
    Ok(())
}

/// Only `.` and `..` left?
pub fn dir_empty(ip: &mut Inode) -> KResult<bool> {
    let mut empty = true;
    dir_scan(ip, |_, _, n| {
        if n != b"." && n != b".." {
            empty = false;
            true
        } else {
            false
        }
    })?;
    Ok(empty)
}
