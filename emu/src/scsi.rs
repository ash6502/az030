//! Virtual SCSI host adapter at 0xFE00_2000.
//!
//! The memory map only reserves this block, so this is an emulator-defined, high-level
//! controller: software fills in a target, a CDB and a DMA buffer, writes EXECUTE, and the
//! whole command (selection, data phase, status) completes before the write returns.
//! All registers are 32-bit, like the rest of the peripheral block.
//!
//! | Offset | R/W | Name     | Meaning                                                    |
//! |--------|-----|----------|------------------------------------------------------------|
//! | `0x00` | W   | CMD      | 1 = EXECUTE, 2 = BUS RESET, 3 = CLEAR STATUS               |
//! | `0x04` | R   | STATUS   | bit0 BUSY, bit1 DONE, bit2 NO TARGET (selection timeout),  |
//! |        |     |          | bit3 DMA ERROR, bits 15:8 SCSI status byte (0 GOOD, 2 CHECK)|
//! | `0x08` | RW  | TARGET   | bits 2:0 target ID, bits 10:8 LUN                          |
//! | `0x0C` | RW  | DMA_ADDR | physical RAM address of the data buffer                    |
//! | `0x10` | RW  | DMA_LEN  | size of the data buffer in bytes                           |
//! | `0x14` | R   | XFER     | bytes actually moved by the last command                   |
//! | `0x18` | RW  | CDB_LEN  | 6, 10, 12 or 16                                            |
//! | `0x1C` | R   | ID       | `0x53435349` ("SCSI")                                      |
//! | `0x20` | RW  | CDB0-3   | CDB bytes 0-15, big-endian, four per register              |

use crate::config::DiskConfig;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub const BLOCK: u64 = 512;

const ST_BUSY: u32 = 1 << 0;
const ST_DONE: u32 = 1 << 1;
const ST_NO_TARGET: u32 = 1 << 2;
const ST_DMA_ERR: u32 = 1 << 3;

const GOOD: u8 = 0x00;
const CHECK_CONDITION: u8 = 0x02;

// Sense keys.
const NO_SENSE: u8 = 0x0;
const MEDIUM_ERROR: u8 = 0x3;
const ILLEGAL_REQUEST: u8 = 0x5;
const DATA_PROTECT: u8 = 0x7;

#[derive(Clone, Copy, Default)]
struct Sense {
    key: u8,
    asc: u8,
    ascq: u8,
}

struct Disk {
    file: File,
    blocks: u64,
    read_only: bool,
    sense: Sense,
}

/// Outcome of one command at the SCSI level.
enum Outcome {
    Good(u32),
    Check(Sense),
    DmaError,
}

pub struct Scsi {
    targets: [Option<Disk>; 8],
    status: u32,
    target: u32,
    dma_addr: u32,
    dma_len: u32,
    xfer: u32,
    cdb_len: u32,
    cdb: [u8; 16],
}

impl Scsi {
    pub fn new(disks: &[DiskConfig]) -> Result<Self, String> {
        let mut targets: [Option<Disk>; 8] = Default::default();
        for d in disks {
            let path = &d.image;
            if !path.exists() {
                match d.create_size {
                    Some(size) => {
                        let f = File::create(path)
                            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
                        f.set_len(size.0)
                            .map_err(|e| format!("cannot size {}: {e}", path.display()))?;
                    }
                    None => {
                        return Err(format!(
                            "SCSI ID {}: image {} does not exist (set create_size to make one)",
                            d.id,
                            path.display()
                        ));
                    }
                }
            }
            let file = OpenOptions::new()
                .read(true)
                .write(!d.read_only)
                .open(path)
                .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            targets[d.id as usize] = Some(Disk {
                file,
                blocks: len / BLOCK,
                read_only: d.read_only,
                sense: Sense::default(),
            });
        }
        Ok(Self {
            targets,
            status: 0,
            target: 0,
            dma_addr: 0,
            dma_len: 0,
            xfer: 0,
            cdb_len: 6,
            cdb: [0; 16],
        })
    }

    pub fn read(&self, off: u32) -> Option<u32> {
        Some(match off {
            0x00 => 0,
            0x04 => self.status,
            0x08 => self.target,
            0x0C => self.dma_addr,
            0x10 => self.dma_len,
            0x14 => self.xfer,
            0x18 => self.cdb_len,
            0x1C => 0x5343_5349,
            0x20..=0x2F => {
                let i = (off - 0x20) as usize & !3;
                u32::from_be_bytes(self.cdb[i..i + 4].try_into().unwrap())
            }
            _ => return None,
        })
    }

    /// Register write. `ram` is the machine's RAM, which the controller DMAs into.
    pub fn write(&mut self, off: u32, val: u32, ram: &mut [u8]) -> bool {
        match off {
            0x00 => match val {
                1 => self.execute(ram),
                2 => {
                    self.status = 0;
                    self.xfer = 0;
                    for d in self.targets.iter_mut().flatten() {
                        d.sense = Sense::default();
                        let _ = d.file.flush();
                    }
                }
                3 => self.status = 0,
                _ => {}
            },
            0x08 => self.target = val & 0x707,
            0x0C => self.dma_addr = val,
            0x10 => self.dma_len = val,
            0x18 => self.cdb_len = val & 0x1F,
            0x20..=0x2F => {
                let i = (off - 0x20) as usize & !3;
                self.cdb[i..i + 4].copy_from_slice(&val.to_be_bytes());
            }
            0x04 | 0x14 | 0x1C => {} // read-only, writes ignored
            _ => return false,
        }
        true
    }

    fn execute(&mut self, ram: &mut [u8]) {
        self.status = ST_BUSY;
        self.xfer = 0;
        let id = (self.target & 7) as usize;
        let lun = (self.target >> 8) & 7;
        let cdb = self.cdb;
        let (dma_addr, dma_len) = (self.dma_addr as usize, self.dma_len as usize);

        let Some(disk) = self.targets[id].as_mut() else {
            self.status = ST_DONE | ST_NO_TARGET;
            return;
        };

        // Resolve the DMA window up front; commands that move no data never touch it.
        let buf = dma_addr
            .checked_add(dma_len)
            .filter(|&end| end <= ram.len())
            .map(|end| &mut ram[dma_addr..end]);

        let outcome = disk.command(&cdb, lun, buf);
        self.status = match outcome {
            Outcome::Good(n) => {
                self.xfer = n;
                disk.sense = Sense::default();
                ST_DONE | (GOOD as u32) << 8
            }
            Outcome::Check(s) => {
                disk.sense = s;
                ST_DONE | (CHECK_CONDITION as u32) << 8
            }
            Outcome::DmaError => ST_DONE | ST_DMA_ERR,
        };
    }
}

fn check(key: u8, asc: u8) -> Outcome {
    Outcome::Check(Sense { key, asc, ascq: 0 })
}

/// Copy `data` (data-in phase) into the DMA buffer, truncated to its size like an
/// allocation length.
fn data_in(buf: Option<&mut [u8]>, data: &[u8]) -> Outcome {
    let Some(buf) = buf else { return Outcome::DmaError };
    let n = data.len().min(buf.len());
    buf[..n].copy_from_slice(&data[..n]);
    Outcome::Good(n as u32)
}

impl Disk {
    fn command(&mut self, cdb: &[u8; 16], lun: u32, buf: Option<&mut [u8]>) -> Outcome {
        let op = cdb[0];
        if lun != 0 {
            if op == 0x12 {
                // INQUIRY to a missing LUN: peripheral qualifier 3, "not supported".
                let mut d = [0u8; 36];
                d[0] = 0x7F;
                return data_in(buf, &d);
            }
            return check(ILLEGAL_REQUEST, 0x25);
        }
        match op {
            0x00 | 0x1B | 0x1E | 0x2F | 0x04 => Outcome::Good(0), // TUR, START STOP, PREVENT, VERIFY, FORMAT
            0x03 => {
                // REQUEST SENSE (fixed format)
                let mut d = [0u8; 18];
                d[0] = 0x70;
                d[2] = self.sense.key;
                d[7] = 10;
                d[12] = self.sense.asc;
                d[13] = self.sense.ascq;
                let alloc = if cdb[4] == 0 { 4 } else { cdb[4] as usize };
                let n = alloc.min(d.len());
                let r = data_in(buf, &d[..n]);
                self.sense = Sense { key: NO_SENSE, asc: 0, ascq: 0 };
                r
            }
            0x12 => {
                let mut d = [0u8; 36];
                d[2] = 2; // SCSI-2
                d[3] = 2; // response data format
                d[4] = 31;
                d[8..16].copy_from_slice(b"AZ030   ");
                d[16..32].copy_from_slice(b"VIRTUAL DISK    ");
                d[32..36].copy_from_slice(b"0.1 ");
                let n = (cdb[4] as usize).min(d.len());
                data_in(buf, &d[..n])
            }
            0x1A => {
                // MODE SENSE(6): header + one block descriptor, no pages.
                let mut d = [0u8; 12];
                d[0] = 11;
                d[2] = if self.read_only { 0x80 } else { 0 };
                d[3] = 8;
                let blocks = self.blocks.min(0xFF_FFFF) as u32;
                d[5..8].copy_from_slice(&blocks.to_be_bytes()[1..]);
                d[9..12].copy_from_slice(&(BLOCK as u32).to_be_bytes()[1..]);
                let n = (cdb[4] as usize).min(d.len());
                data_in(buf, &d[..n])
            }
            0x25 => {
                let mut d = [0u8; 8];
                let last = self.blocks.saturating_sub(1).min(0xFFFF_FFFF) as u32;
                d[..4].copy_from_slice(&last.to_be_bytes());
                d[4..].copy_from_slice(&(BLOCK as u32).to_be_bytes());
                data_in(buf, &d)
            }
            0x35 => match self.file.sync_data() {
                Ok(()) => Outcome::Good(0),
                Err(_) => check(MEDIUM_ERROR, 0x0C),
            },
            0x08 | 0x0A | 0x28 | 0x2A => {
                let (lba, count) = if op & 0x20 == 0 {
                    let lba = u32::from_be_bytes([0, cdb[1] & 0x1F, cdb[2], cdb[3]]) as u64;
                    let count = if cdb[4] == 0 { 256 } else { cdb[4] as u64 };
                    (lba, count)
                } else {
                    let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap()) as u64;
                    (lba, u16::from_be_bytes([cdb[7], cdb[8]]) as u64)
                };
                let write = op & 0x02 != 0;
                if lba + count > self.blocks {
                    return check(ILLEGAL_REQUEST, 0x21);
                }
                if write && self.read_only {
                    return check(DATA_PROTECT, 0x27);
                }
                let bytes = (count * BLOCK) as usize;
                if bytes == 0 {
                    return Outcome::Good(0);
                }
                let Some(buf) = buf.filter(|b| b.len() >= bytes) else {
                    return Outcome::DmaError;
                };
                let buf = &mut buf[..bytes];
                let io = self.file.seek(SeekFrom::Start(lba * BLOCK)).and_then(|_| {
                    if write { self.file.write_all(buf) } else { self.file.read_exact(buf) }
                });
                match io {
                    Ok(()) => Outcome::Good(bytes as u32),
                    Err(_) => check(MEDIUM_ERROR, if write { 0x0C } else { 0x11 }),
                }
            }
            _ => check(ILLEGAL_REQUEST, 0x20),
        }
    }
}
