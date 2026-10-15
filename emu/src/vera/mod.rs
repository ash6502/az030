//! VERA video adapter (https://github.com/fvdhoef/vera-module), following the v0.9
//! programmer's reference and the FPGA sources where the reference is silent.
//!
//! The 32 byte-wide VERA registers sit at `0xFE00_3000 + 4 * n` on the az030, one per
//! 32-bit word (the low byte carries the data), because peripheral registers are
//! 32-bit-access only. See `bus.rs`.
//!
//! What is emulated:
//! * VRAM (128 KB) with both auto-incrementing data ports, including the hardware's
//!   read prefetch (a port's DATA register returns the byte fetched when its address was
//!   last set or stepped).
//! * Both layers in every tile and bitmap mode, 128 sprites with Z-depth, flips,
//!   collision masks and the per-line sprite render budget, the palette, and the display
//!   composer (border, active area, H/V scaling).
//! * Scanline-accurate timing and interrupts (VSYNC, LINE, SPRCOL, AFLOW), the SCANLINE
//!   register, and the interlaced field bit for the NTSC/RGB output modes.
//! * The PCM FIFO's fill level and drain rate, so AFLOW and FIFO Full/Empty behave. The
//!   PSG and PCM produce no sound (the emulator has no audio), and the SPI port sees no
//!   SD card (reads return 0xFF).
//!
//! Rendering happens a line at a time as emulated time reaches each scanline, so
//! mid-frame register changes (raster effects) show up where they would on hardware.

mod audio;
pub mod display;
mod palette;
mod render;
#[cfg(test)]
mod tests;

use audio::Audio;

/// VERA's fixed pixel clock.
pub const CLOCK_HZ: u64 = 25_000_000;
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
/// Interrupt priority level of the VERA IRQ line on the az030.
pub const IPL: u8 = 3;

const VRAM_SIZE: usize = 0x2_0000;
const ADDR_MASK: u32 = 0x1_FFFF;
const PSG_BASE: u32 = 0x1_F9C0;
const PALETTE_BASE: u32 = 0x1_FA00;
const SPRITE_BASE: u32 = 0x1_FC00;

const INCREMENTS: [u32; 16] = [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 40, 80, 160, 320, 640];

// ISR / IEN bits
const INT_VSYNC: u8 = 1 << 0;
const INT_LINE: u8 = 1 << 1;
const INT_SPRCOL: u8 = 1 << 2;
const INT_AFLOW: u8 = 1 << 3;

/// Line timing: VGA is 800 clocks x 525 lines; the interlaced modes (NTSC composite and
/// RGB) use 1588-clock lines and 262/263-line fields that cover every other line.
const VGA_LINE_CLOCKS: u64 = 800;
const VGA_LINES: u16 = 525;
const NTSC_LINE_CLOCKS: u64 = 1588;

#[derive(Clone, Copy, Default)]
struct DataPort {
    addr: u32,
    incr: u8,
    decr: bool,
    /// Byte prefetched from `addr`, returned by the next DATA read.
    latch: u8,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Layer {
    pub config: u8,
    pub mapbase: u8,
    pub tilebase: u8,
    pub hscroll: u16,
    pub vscroll: u16,
}

pub struct Vera {
    vram: Vec<u8>,
    palette: [u16; 256],
    sprite_attr: [u8; 1024],
    psg: [u8; 64],

    port: [DataPort; 2],
    addrsel: usize,
    dcsel: bool,

    ien: u8,
    isr: u8,
    irq_line: u16,

    /// DC_VIDEO without the read-only field bit.
    dc_video: u8,
    hscale: u8,
    vscale: u8,
    border: u8,
    hstart: u16,
    hstop: u16,
    vstart: u16,
    vstop: u16,
    layers: [Layer; 2],

    audio: Audio,
    spi_ctrl: u8,
    spi_data: u8,

    // timing
    cpu_hz: u64,
    /// Fractional VERA clocks, in units of 1/cpu_hz.
    frac: u64,
    /// VERA clocks into the current line.
    line_clock: u64,
    /// The scanline counter (y_counter in the RTL).
    y: u16,
    /// Lines left in the current field (interlaced modes).
    field_lines: u16,
    field: bool,

    /// Sprite collisions accumulated this frame / reported for the last frame.
    cur_collisions: u8,
    frame_collisions: u8,

    framebuffer: Vec<u32>,
    scratch: render::Scratch,
    /// Set at the start of vertical blank; the front end clears it after presenting.
    pub frame_ready: bool,
}

impl Vera {
    /// `cpu_hz` is the CPU clock, used to convert executed cycles into VERA clocks.
    pub fn new(cpu_hz: u64) -> Self {
        let mut v = Self::power_on(vec![0; VRAM_SIZE], vec![0; WIDTH * HEIGHT], cpu_hz.max(1));
        v.start_frame();
        v
    }

    /// Register state after reset, keeping the given VRAM and framebuffer storage.
    fn power_on(vram: Vec<u8>, framebuffer: Vec<u32>, cpu_hz: u64) -> Self {
        Self {
            vram,
            palette: palette::DEFAULT,
            sprite_attr: [0; 1024],
            psg: [0; 64],
            port: [DataPort::default(); 2],
            addrsel: 0,
            dcsel: false,
            ien: 0,
            isr: 0,
            irq_line: 0,
            dc_video: 0,
            hscale: 128,
            vscale: 128,
            border: 0,
            hstart: 0,
            hstop: 640,
            vstart: 0,
            vstop: 480,
            layers: [Layer::default(); 2],
            audio: Audio::default(),
            spi_ctrl: 0,
            spi_data: 0,
            cpu_hz,
            frac: 0,
            line_clock: 0,
            y: 0,
            field_lines: 0,
            field: false,
            cur_collisions: 0,
            frame_collisions: 0,
            framebuffer,
            scratch: render::Scratch::default(),
            frame_ready: false,
        }
    }

    /// Register reset (the az030 reset line, or CTRL bit 7). VRAM keeps its contents; the
    /// palette returns to its defaults and the sprite attributes clear, like the FPGA
    /// reconfiguring.
    pub fn reset(&mut self) {
        let vram = std::mem::take(&mut self.vram);
        let mut framebuffer = std::mem::take(&mut self.framebuffer);
        framebuffer.fill(0);
        *self = Self::power_on(vram, framebuffer, self.cpu_hz);
        self.start_frame();
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }

    /// The IRQ line (active when an enabled interrupt is pending).
    pub fn irq(&self) -> bool {
        self.status() & self.ien != 0
    }

    fn status(&self) -> u8 {
        let aflow = if self.audio.fifo_low() { INT_AFLOW } else { 0 };
        self.isr & 7 | aflow
    }

    fn out_mode(&self) -> u8 {
        self.dc_video & 3
    }

    fn interlaced(&self) -> bool {
        self.out_mode() & 2 != 0
    }

    // ---------------------------------------------------------------- registers

    pub fn read(&mut self, reg: u8) -> u8 {
        let p = &self.port[self.addrsel];
        match reg & 0x1F {
            0x00 => p.addr as u8,
            0x01 => (p.addr >> 8) as u8,
            0x02 => p.incr << 4 | (p.decr as u8) << 3 | (p.addr >> 16) as u8 & 1,
            0x03 => self.data_access(0, None),
            0x04 => self.data_access(1, None),
            0x05 => (self.dcsel as u8) << 1 | self.addrsel as u8,
            0x06 => {
                let scan = self.scanline();
                ((self.irq_line >> 8) as u8 & 1) << 7 | ((scan >> 8) as u8 & 1) << 6 | self.ien
            }
            0x07 => self.frame_collisions << 4 | self.status(),
            0x08 => self.scanline() as u8,
            0x09 if !self.dcsel => (self.field as u8) << 7 | self.dc_video,
            0x0A if !self.dcsel => self.hscale,
            0x0B if !self.dcsel => self.vscale,
            0x0C if !self.dcsel => self.border,
            0x09 => (self.hstart >> 2) as u8,
            0x0A => (self.hstop >> 2) as u8,
            0x0B => (self.vstart >> 1) as u8,
            0x0C => (self.vstop >> 1) as u8,
            r @ 0x0D..=0x1A => {
                let (l, off) = if r < 0x14 { (&self.layers[0], r - 0x0D) } else { (&self.layers[1], r - 0x14) };
                match off {
                    0 => l.config,
                    1 => l.mapbase,
                    2 => l.tilebase,
                    3 => l.hscroll as u8,
                    4 => (l.hscroll >> 8) as u8,
                    5 => l.vscroll as u8,
                    _ => (l.vscroll >> 8) as u8,
                }
            }
            0x1B => self.audio.ctrl_read(),
            0x1C => self.audio.rate,
            0x1D => 0,
            0x1E => {
                let d = self.spi_data;
                if self.spi_ctrl & 4 != 0 {
                    self.spi_transfer(0xFF); // auto-tx
                }
                d
            }
            _ => self.spi_ctrl & 7, // never busy: transfers complete instantly
        }
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        match reg & 0x1F {
            0x00 => self.set_addr(|p| p.addr = p.addr & !0xFF | v as u32),
            0x01 => self.set_addr(|p| p.addr = p.addr & !0xFF00 | (v as u32) << 8),
            0x02 => self.set_addr(|p| {
                p.incr = v >> 4;
                p.decr = v & 8 != 0;
                p.addr = p.addr & 0xFFFF | (v as u32 & 1) << 16;
            }),
            0x03 => {
                self.data_access(0, Some(v));
            }
            0x04 => {
                self.data_access(1, Some(v));
            }
            0x05 => {
                if v & 0x80 != 0 {
                    self.reset();
                    return;
                }
                self.dcsel = v & 2 != 0;
                self.addrsel = (v & 1) as usize;
            }
            0x06 => {
                self.irq_line = self.irq_line & 0xFF | (v as u16 >> 7) << 8;
                self.ien = v & 0x0F;
            }
            0x07 => self.isr &= !(v & 7),
            0x08 => self.irq_line = self.irq_line & 0x100 | v as u16,
            0x09 if !self.dcsel => self.dc_video = v & 0x77,
            0x0A if !self.dcsel => self.hscale = v,
            0x0B if !self.dcsel => self.vscale = v,
            0x0C if !self.dcsel => self.border = v,
            0x09 => self.hstart = (v as u16) << 2,
            0x0A => self.hstop = (v as u16) << 2,
            0x0B => self.vstart = (v as u16) << 1,
            0x0C => self.vstop = (v as u16) << 1,
            r @ 0x0D..=0x1A => {
                let (l, off) =
                    if r < 0x14 { (&mut self.layers[0], r - 0x0D) } else { (&mut self.layers[1], r - 0x14) };
                match off {
                    0 => l.config = v,
                    1 => l.mapbase = v,
                    2 => l.tilebase = v,
                    3 => l.hscroll = l.hscroll & 0xF00 | v as u16,
                    4 => l.hscroll = l.hscroll & 0xFF | (v as u16 & 0xF) << 8,
                    5 => l.vscroll = l.vscroll & 0xF00 | v as u16,
                    _ => l.vscroll = l.vscroll & 0xFF | (v as u16 & 0xF) << 8,
                }
            }
            0x1B => self.audio.ctrl_write(v),
            0x1C => self.audio.rate = v,
            0x1D => self.audio.push(v),
            0x1E => self.spi_transfer(v),
            _ => self.spi_ctrl = v & 7,
        }
    }

    /// No SD card is attached: MISO idles high, so every transfer reads back 0xFF.
    fn spi_transfer(&mut self, _tx: u8) {
        self.spi_data = 0xFF;
    }

    /// Update the selected port's address registers, then prefetch from the new address.
    fn set_addr(&mut self, f: impl FnOnce(&mut DataPort)) {
        let p = &mut self.port[self.addrsel];
        f(p);
        p.latch = self.vram[p.addr as usize];
    }

    /// DATA0/DATA1 access: return the prefetched byte (reads) or store `write`, then step
    /// the port's address and prefetch from it.
    fn data_access(&mut self, n: usize, write: Option<u8>) -> u8 {
        let p = self.port[n];
        if let Some(v) = write {
            self.vram_write(p.addr, v);
        }
        let step = INCREMENTS[p.incr as usize];
        let addr = if p.decr { p.addr.wrapping_sub(step) } else { p.addr.wrapping_add(step) } & ADDR_MASK;
        let latch = self.vram[addr as usize];
        let port = &mut self.port[n];
        port.addr = addr;
        port.latch = latch;
        p.latch
    }

    /// VRAM write; the top 1.5 KB also updates the write-only PSG, palette and sprite
    /// attribute registers (reads always see plain VRAM).
    fn vram_write(&mut self, addr: u32, v: u8) {
        self.vram[addr as usize] = v;
        if addr >= SPRITE_BASE {
            self.sprite_attr[(addr - SPRITE_BASE) as usize] = v;
        } else if addr >= PALETTE_BASE {
            let i = ((addr - PALETTE_BASE) >> 1) as usize;
            self.palette[i] = if addr & 1 == 0 {
                self.palette[i] & 0xF00 | v as u16
            } else {
                self.palette[i] & 0x0FF | (v as u16 & 0xF) << 8
            };
        } else if addr >= PSG_BASE {
            self.psg[(addr - PSG_BASE) as usize] = v;
        }
    }

    // ---------------------------------------------------------------- timing

    fn line_clocks(&self) -> u64 {
        if self.interlaced() { NTSC_LINE_CLOCKS } else { VGA_LINE_CLOCKS }
    }

    /// SCANLINE as software reads it: pegged at 511 past the 9-bit range.
    fn scanline(&self) -> u16 {
        self.y.min(0x1FF)
    }

    /// CPU cycles until the next scanline starts.
    pub fn cycles_to_next_line(&self) -> u64 {
        let remaining = (self.line_clocks().saturating_sub(self.line_clock) * self.cpu_hz)
            .saturating_sub(self.frac);
        remaining.div_ceil(CLOCK_HZ)
    }

    /// Advance VERA time by `cpu_cycles` CPU cycles, running every scanline that starts.
    pub fn advance(&mut self, cpu_cycles: u64) {
        self.frac += cpu_cycles * CLOCK_HZ;
        let clocks = self.frac / self.cpu_hz;
        self.frac %= self.cpu_hz;
        self.audio.advance(clocks);
        self.line_clock += clocks;
        while self.line_clock >= self.line_clocks() {
            self.line_clock -= self.line_clocks();
            self.next_line();
        }
    }

    fn start_frame(&mut self) {
        if self.interlaced() {
            self.field = !self.field;
            self.y = self.field as u16;
            self.field_lines = if self.field { 262 } else { 263 };
        } else {
            self.field = false;
            self.y = 0;
            self.field_lines = VGA_LINES;
        }
        self.start_line();
    }

    fn next_line(&mut self) {
        self.field_lines -= 1;
        if self.field_lines == 0 {
            self.start_frame();
        } else {
            self.y += if self.interlaced() { 2 } else { 1 };
            self.start_line();
        }
    }

    /// Work done as scanline `self.y` begins: interrupts, rendering, vertical blank.
    fn start_line(&mut self) {
        let hit = if self.interlaced() {
            self.y >> 1 == self.irq_line >> 1
        } else {
            self.y == self.irq_line
        };
        if hit {
            self.isr |= INT_LINE;
        }
        let y = self.y as usize;
        if y < HEIGHT {
            self.render_line(y);
        }
        // First line of vertical blank (480, or 481 in the odd field).
        if y >= HEIGHT && y < HEIGHT + 2 && (y == HEIGHT || self.interlaced()) {
            self.isr |= INT_VSYNC;
            self.frame_collisions = self.cur_collisions;
            if self.cur_collisions != 0 {
                self.isr |= INT_SPRCOL;
            }
            self.cur_collisions = 0;
            self.frame_ready = true;
        }
    }
}
