//! az030 address decoding (see MEM_MAP.md).
//!
//! | Address       | What                                                        |
//! |---------------|-------------------------------------------------------------|
//! | `0x0000_0000` | RAM (size from config). ROM overlays reads until BOOT write |
//! | `0xFD00_0000` | VRAM (reserved -> bus error)                                |
//! | `0xFE00_0000` | UART: `+0` DATA, `+4` STATUS                                |
//! | `0xFE00_1000` | SYSCTRL: `+0` BOOT, `+4` ID, `+8` LEDS                      |
//! | `0xFE00_2000` | SCSI (emulator-defined, see scsi.rs)                        |
//! | `0xFE00_3000` | Video registers (reserved -> bus error)                     |
//! | `0xFFF0_0000` | Boot ROM, mirrored across 1 MB                              |
//!
//! Peripheral registers are 32-bit only; byte/word accesses to them, and any access to an
//! unmapped address, raise a bus error.

use crate::scsi::Scsi;
use m68k::AddressBus;
use m68k::core::memory::{BusFault, BusFaultKind};
use std::collections::VecDeque;
use std::io::Write;

const UART_BASE: u32 = 0xFE00_0000;
const SYSCTRL_BASE: u32 = 0xFE00_1000;
const SCSI_BASE: u32 = 0xFE00_2000;
const ROM_BASE: u32 = 0xFFF0_0000;

const UART_TX_READY: u32 = 1 << 0;
const UART_RX_VALID: u32 = 1 << 1;

pub struct Bus {
    pub ram: Vec<u8>,
    rom: Vec<u8>,
    /// ROM visible at address 0 (reset state) until SYSCTRL BOOT is written.
    pub overlay: bool,
    pub uart_rx: VecDeque<u8>,
    pub uart_tx: Vec<u8>,
    sysctrl_id: u32,
    pub leds: u32,
    /// Set when the LEDs change, so the front end can show them.
    pub leds_changed: bool,
    pub scsi: Option<Scsi>,
}

fn fault(address: u32) -> BusFault {
    BusFault { kind: BusFaultKind::BusError, address }
}

impl Bus {
    pub fn new(ram_size: usize, rom: Vec<u8>, sysctrl_id: u32, scsi: Option<Scsi>) -> Self {
        Self {
            ram: vec![0; ram_size],
            rom,
            overlay: true,
            uart_rx: VecDeque::new(),
            uart_tx: Vec::new(),
            sysctrl_id,
            leds: 0,
            leds_changed: false,
            scsi,
        }
    }

    /// Hardware reset: re-enable the ROM overlay. RAM contents survive, as on real DRAM.
    pub fn reset(&mut self) {
        self.overlay = true;
        self.leds = 0;
        self.leds_changed = true;
    }

    #[inline]
    fn rom_byte(&self, addr: u32) -> u8 {
        self.rom[addr as usize % self.rom.len()]
    }

    /// Plain memory (RAM/ROM) read of `N` bytes, or None if this is not memory.
    #[inline]
    fn mem_read<const N: usize>(&self, addr: u32) -> Option<[u8; N]> {
        let a = addr as usize;
        if a + N <= self.ram.len() {
            if self.overlay {
                return Some(std::array::from_fn(|i| self.rom_byte(addr + i as u32)));
            }
            return Some(self.ram[a..a + N].try_into().unwrap());
        }
        if addr >= ROM_BASE {
            return Some(std::array::from_fn(|i| self.rom_byte(addr.wrapping_add(i as u32))));
        }
        None
    }

    /// Plain memory write; Some(()) if handled. ROM writes are silently ignored.
    #[inline]
    fn mem_write(&mut self, addr: u32, bytes: &[u8]) -> Option<()> {
        let a = addr as usize;
        if a + bytes.len() <= self.ram.len() {
            self.ram[a..a + bytes.len()].copy_from_slice(bytes);
            return Some(());
        }
        if addr >= ROM_BASE {
            return Some(());
        }
        None
    }

    fn io_read(&mut self, addr: u32) -> Option<u32> {
        match addr {
            UART_BASE => Some(self.uart_rx.pop_front().map_or(0, u32::from)),
            0xFE00_0004 => {
                let rx = if self.uart_rx.is_empty() { 0 } else { UART_RX_VALID };
                Some(UART_TX_READY | rx)
            }
            SYSCTRL_BASE => Some(0),
            0xFE00_1004 => Some(self.sysctrl_id),
            0xFE00_1008 => Some(self.leds),
            a if (SCSI_BASE..SCSI_BASE + 0x1000).contains(&a) => {
                self.scsi.as_ref()?.read(a - SCSI_BASE)
            }
            _ => None,
        }
    }

    fn io_write(&mut self, addr: u32, val: u32) -> Option<()> {
        match addr {
            UART_BASE => self.uart_tx.push(val as u8),
            0xFE00_0004 => {}
            SYSCTRL_BASE => self.overlay = false,
            0xFE00_1004 => {}
            0xFE00_1008 => {
                if self.leds != val {
                    self.leds = val;
                    self.leds_changed = true;
                }
            }
            a if (SCSI_BASE..SCSI_BASE + 0x1000).contains(&a) => {
                let scsi = self.scsi.as_mut()?;
                if !scsi.write(a - SCSI_BASE, val, &mut self.ram) {
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }

    pub fn flush_tx(&mut self) {
        if !self.uart_tx.is_empty() {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&self.uart_tx);
            let _ = out.flush();
            self.uart_tx.clear();
        }
    }
}

impl AddressBus for Bus {
    fn read_byte(&mut self, a: u32) -> u8 {
        self.try_read_byte(a).unwrap_or(0xFF)
    }
    fn read_word(&mut self, a: u32) -> u16 {
        self.try_read_word(a).unwrap_or(0xFFFF)
    }
    fn read_long(&mut self, a: u32) -> u32 {
        self.try_read_long(a).unwrap_or(0xFFFF_FFFF)
    }
    fn write_byte(&mut self, a: u32, v: u8) {
        let _ = self.try_write_byte(a, v);
    }
    fn write_word(&mut self, a: u32, v: u16) {
        let _ = self.try_write_word(a, v);
    }
    fn write_long(&mut self, a: u32, v: u32) {
        let _ = self.try_write_long(a, v);
    }

    fn try_read_byte(&mut self, a: u32) -> Result<u8, BusFault> {
        self.mem_read::<1>(a).map(|b| b[0]).ok_or(fault(a))
    }
    fn try_read_word(&mut self, a: u32) -> Result<u16, BusFault> {
        self.mem_read::<2>(a).map(u16::from_be_bytes).ok_or(fault(a))
    }
    fn try_read_long(&mut self, a: u32) -> Result<u32, BusFault> {
        if let Some(b) = self.mem_read::<4>(a) {
            return Ok(u32::from_be_bytes(b));
        }
        self.io_read(a).ok_or(fault(a))
    }
    fn try_write_byte(&mut self, a: u32, v: u8) -> Result<(), BusFault> {
        self.mem_write(a, &[v]).ok_or(fault(a))
    }
    fn try_write_word(&mut self, a: u32, v: u16) -> Result<(), BusFault> {
        self.mem_write(a, &v.to_be_bytes()).ok_or(fault(a))
    }
    fn try_write_long(&mut self, a: u32, v: u32) -> Result<(), BusFault> {
        if self.mem_write(a, &v.to_be_bytes()).is_some() {
            return Ok(());
        }
        self.io_write(a, v).ok_or(fault(a))
    }
}
