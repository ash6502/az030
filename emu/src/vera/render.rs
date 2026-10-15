//! Line renderer: the two layer renderers, the sprite renderer and the display composer,
//! following fpga/source/graphics/*.v.
//!
//! Layers and sprites draw into 640-entry line buffers in "layer space"; the composer
//! then walks the 640 output pixels, applying the active area, border and H/V scaling.

use super::{ADDR_MASK, HEIGHT, Layer, Vera, WIDTH};

/// Line buffers (1024 entries, like the hardware's 10-bit indexes).
pub(super) struct Scratch {
    layer: [[u8; 1024]; 2],
    sprite_color: [u8; 1024],
    sprite_z: [u8; 1024],
    sprite_mask: [u8; 1024],
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            layer: [[0; 1024]; 2],
            sprite_color: [0; 1024],
            sprite_z: [0; 1024],
            sprite_mask: [0; 1024],
        }
    }
}

/// Sprite renderer time budget per line, in VERA clocks (render_time in the RTL).
const SPRITE_BUDGET: i32 = 798;

/// Map output pixel `n` of the active area to a layer-space coordinate. The hardware's
/// scaled counter stops advancing once it reaches `limit`.
fn scaled(n: usize, scale: u8, limit: usize) -> usize {
    if scale == 0 {
        return 0;
    }
    let scale = scale as usize;
    let first_past = (limit << 7).div_ceil(scale);
    (n.min(first_past) * scale) >> 7
}

/// Pixel `idx` of a packed row at `base` with `bpp` bits per pixel (leftmost pixel in
/// the most significant bits).
#[inline]
fn pixel(vram: &[u8], base: u32, idx: u32, bpp: u32) -> u8 {
    let bit = idx * bpp;
    let byte = vram[((base + (bit >> 3)) & ADDR_MASK) as usize];
    let shift = 8 - bpp - (bit & 7);
    (byte >> shift) & ((1u16 << bpp) - 1) as u8
}

/// Palette offset rule shared by layers and sprites: colors 1-15 move to the selected
/// 16-color block, 0 (transparent) and 16-255 are left alone.
#[inline]
fn offset(color: u8, pal: u8) -> u8 {
    if (1..16).contains(&color) { color | pal << 4 } else { color }
}

#[inline]
pub(super) fn rgb(c: u16) -> u32 {
    let r = (c >> 8 & 0xF) as u32 * 17;
    let g = (c >> 4 & 0xF) as u32 * 17;
    let b = (c & 0xF) as u32 * 17;
    r << 16 | g << 8 | b
}

/// Luma only, for composite output with chroma disabled.
#[inline]
fn grey(c: u32) -> u32 {
    let (r, g, b) = (c >> 16 & 0xFF, c >> 8 & 0xFF, c & 0xFF);
    let y = (r * 299 + g * 587 + b * 114) / 1000;
    y << 16 | y << 8 | y
}

fn render_layer(vram: &[u8], l: &Layer, line: u32, out: &mut [u8; 1024]) {
    let depth = l.config & 3;
    let bpp = 1u32 << depth;
    let tilebase = (l.tilebase as u32 & 0xFC) << 9;
    let wide = l.tilebase & 1 != 0;
    let tall = l.tilebase & 2 != 0;

    if l.config & 4 != 0 {
        // Bitmap: no scrolling; H-Scroll (11:8) is the palette offset. The renderer reads
        // 640 pixels linearly, so a 320-wide bitmap runs into the next row past x = 320.
        let width = if wide { 640 } else { 320 };
        let base = tilebase + line * (width * bpp / 8);
        let pal = (l.hscroll >> 8) as u8 & 0xF;
        for (x, o) in out[..WIDTH].iter_mut().enumerate() {
            let c = pixel(vram, base, x as u32, bpp);
            *o = if depth == 0 { c } else { offset(c, pal) };
        }
        return;
    }

    let tile_w = if wide { 16 } else { 8 };
    let tile_h = if tall { 16 } else { 8 };
    let map_w = 32u32 << (l.config >> 4 & 3);
    let map_h = 32u32 << (l.config >> 6 & 3);
    let mapbase = (l.mapbase as u32) << 9;
    let row_bytes = tile_w * bpp / 8;
    let tile_bytes = row_bytes * tile_h;
    let t256c = l.config & 8 != 0;

    let y = (line + l.vscroll as u32) & 0xFFF;
    let map_row = (y / tile_h) & (map_h - 1);
    let row = y % tile_h;

    let mut sx = l.hscroll as u32 & 0xFFF;
    let mut x = 0;
    while x < WIDTH {
        let map_col = (sx / tile_w) & (map_w - 1);
        let entry = mapbase + (map_row * map_w + map_col) * 2;
        let b0 = vram[(entry & ADDR_MASK) as usize];
        let b1 = vram[((entry + 1) & ADDR_MASK) as usize];
        let first = sx % tile_w;
        let n = ((tile_w - first) as usize).min(WIDTH - x);

        if depth == 0 {
            // 1 bpp text: byte 0 is the character, byte 1 the colors.
            let base = tilebase + b0 as u32 * tile_bytes + row * row_bytes;
            let (fg, bg) = if t256c { (b1, 0) } else { (b1 & 0xF, b1 >> 4) };
            for i in 0..n {
                out[x + i] = if pixel(vram, base, first + i as u32, 1) != 0 { fg } else { bg };
            }
        } else {
            let tile = b0 as u32 | (b1 as u32 & 3) << 8;
            let hflip = b1 & 4 != 0;
            let vflip = b1 & 8 != 0;
            let pal = b1 >> 4;
            let r = if vflip { tile_h - 1 - row } else { row };
            let base = tilebase + tile * tile_bytes + r * row_bytes;
            for i in 0..n {
                let c = first + i as u32;
                let c = if hflip { tile_w - 1 - c } else { c };
                out[x + i] = offset(pixel(vram, base, c, bpp), pal);
            }
        }
        x += n;
        sx = (sx + n as u32) & 0xFFF;
    }
}

impl Vera {
    pub(super) fn render_line(&mut self, y: usize) {
        let vactive = y >= self.vstart as usize && y < self.vstop as usize;
        let video = self.dc_video;
        let (l0_on, l1_on, spr_on) = (video & 0x10 != 0, video & 0x20 != 0, video & 0x40 != 0);

        if vactive {
            let line = scaled(y - self.vstart as usize, self.vscale, HEIGHT) as u32 & 0x1FF;
            let Vera { vram, layers, scratch, .. } = self;
            for (n, on) in [l0_on, l1_on].into_iter().enumerate() {
                if on {
                    render_layer(vram, &layers[n], line, &mut scratch.layer[n]);
                }
            }
            // The sprite renderer runs whether or not sprites are displayed, so
            // collisions are still detected.
            self.render_sprites(line);
        }

        let row = &mut self.framebuffer[y * WIDTH..(y + 1) * WIDTH];
        let mode = video & 3;
        if mode == 0 {
            row.fill(0);
            return;
        }
        let mono = mode == 2 && video & 4 != 0;
        let s = &self.scratch;
        let (hstart, hstop) = (self.hstart as usize, self.hstop as usize);
        for (x, px) in row.iter_mut().enumerate() {
            let color = if !vactive || x < hstart || x >= hstop {
                self.border
            } else {
                let sx = scaled(x - hstart, self.hscale, WIDTH);
                let sl = sx & 1023;
                let (sc, sz) = (s.sprite_color[sl], s.sprite_z[sl]);
                let l0 = if sx < WIDTH { s.layer[0][sx] } else { 0 };
                let l1 = if sx < WIDTH { s.layer[1][sx] } else { 0 };
                let mut c = 0;
                if spr_on && sc != 0 && sz == 1 {
                    c = sc;
                }
                if l0_on && l0 != 0 {
                    c = l0;
                }
                if spr_on && sc != 0 && sz == 2 {
                    c = sc;
                }
                if l1_on && l1 != 0 {
                    c = l1;
                }
                if spr_on && sc != 0 && sz == 3 {
                    c = sc;
                }
                c
            };
            let c = rgb(self.palette[color as usize]);
            *px = if mono { grey(c) } else { c };
        }
    }

    /// Render the sprites crossing layer line `line` into the sprite line buffer.
    /// Lower-numbered sprites win among equal Z-depths; a higher Z-depth always wins.
    fn render_sprites(&mut self, line: u32) {
        let Vera { vram, sprite_attr, scratch: s, cur_collisions, .. } = self;
        s.sprite_color.fill(0);
        s.sprite_z.fill(0);
        s.sprite_mask.fill(0);

        // Approximate the hardware's clock budget: 1 clock per sprite examined, 1 to
        // start a sprite, 2 per 32-bit fetch and 1 per pixel written.
        let mut budget = SPRITE_BUDGET;
        for a in sprite_attr.chunks_exact(8) {
            budget -= 1;
            if budget <= 0 {
                break;
            }
            let z = a[6] >> 2 & 3;
            if z == 0 {
                continue;
            }
            let h = 8u32 << (a[7] >> 6 & 3);
            let ydiff = line.wrapping_sub(a[4] as u32 | (a[5] as u32 & 3) << 8) & 0x3FF;
            if ydiff >= h {
                continue;
            }
            let w = 8u32 << (a[7] >> 4 & 3);
            let bpp = if a[1] & 0x80 != 0 { 8 } else { 4 };
            let per_fetch = 32 / bpp;
            let addr = (a[0] as u32) << 5 | (a[1] as u32 & 0xF) << 13;
            let x0 = a[2] as u32 | (a[3] as u32 & 3) << 8;
            let (hflip, vflip) = (a[6] & 1 != 0, a[6] & 2 != 0);
            let mask = a[6] >> 4;
            let pal = a[7] & 0xF;
            let r = if vflip { h - 1 - ydiff } else { ydiff };
            let base = addr + r * w * bpp / 8;

            budget -= 1;
            for px in 0..w {
                if px % per_fetch == 0 {
                    budget -= 2;
                }
                budget -= 1;
                if budget < 0 {
                    return;
                }
                let c = if hflip { w - 1 - px } else { px };
                let color = pixel(vram, base, c, bpp);
                if color == 0 {
                    continue;
                }
                let color = offset(color, pal);
                let i = ((x0 + px) & 0x3FF) as usize;
                if i < WIDTH && mask != 0 {
                    *cur_collisions |= s.sprite_mask[i] & mask;
                }
                if s.sprite_color[i] == 0 || z > s.sprite_z[i] {
                    s.sprite_color[i] = color;
                    s.sprite_z[i] = z;
                    s.sprite_mask[i] |= mask;
                }
            }
        }
    }
}
