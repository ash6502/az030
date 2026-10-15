//! Behaviour checks against the VERA reference and RTL.

use super::*;

/// A VERA clocked 1:1 with the CPU, so cycles == VERA clocks.
fn vera() -> Vera {
    Vera::new(CLOCK_HZ)
}

fn set_addr(v: &mut Vera, addr: u32, incr: u8) {
    v.write(0x00, addr as u8);
    v.write(0x01, (addr >> 8) as u8);
    v.write(0x02, incr << 4 | (addr >> 16) as u8 & 1);
}

fn poke(v: &mut Vera, addr: u32, bytes: &[u8]) {
    set_addr(v, addr, 1);
    for &b in bytes {
        v.write(0x03, b);
    }
}

/// Run to the start of the next frame's first line, so line 0 renders with the
/// current registers.
fn next_frame(v: &mut Vera) {
    let to_go = (VGA_LINES - v.y) as u64 * VGA_LINE_CLOCKS - v.line_clock;
    v.advance(to_go);
}

fn px(v: &Vera, x: usize, y: usize) -> u32 {
    v.framebuffer()[y * WIDTH + x]
}

fn pal_rgb(v: &Vera, i: usize) -> u32 {
    render::rgb(v.palette[i])
}

#[test]
fn data_port_prefetch_and_increment() {
    let mut v = vera();
    poke(&mut v, 0x100, &[0x11, 0x22, 0x33]);
    set_addr(&mut v, 0x100, 1);
    assert_eq!(v.read(0x03), 0x11);
    assert_eq!(v.read(0x03), 0x22);
    // Port 0 has already fetched 0x102; changing it through port 1 leaves port 0's
    // latch stale, as on hardware.
    v.write(0x05, 1);
    set_addr(&mut v, 0x102, 0);
    v.write(0x04, 0x99);
    v.write(0x05, 0);
    assert_eq!(v.read(0x03), 0x33);
    // Address registers read back with the step applied.
    assert_eq!(v.read(0x00), 0x03);
}

#[test]
fn increments_and_decrement() {
    let mut v = vera();
    set_addr(&mut v, 0x1000, 11); // +40
    v.write(0x03, 1);
    v.write(0x03, 2);
    assert_eq!(v.port[0].addr, 0x1000 + 80);
    v.write(0x02, 15 << 4 | 8); // -640, DECR
    v.write(0x03, 3);
    assert_eq!(v.port[0].addr, 0x1000 + 80 - 640);
    assert_eq!(v.read(0x02), 0xF8);
    // Wraps within 128 KB.
    set_addr(&mut v, 0x1FFFF, 1);
    v.write(0x03, 0);
    assert_eq!(v.port[0].addr, 0);
}

#[test]
fn palette_and_sprite_registers_shadow_vram() {
    let mut v = vera();
    poke(&mut v, PALETTE_BASE + 2 * 5, &[0x34, 0x12]);
    assert_eq!(v.palette[5], 0x234);
    poke(&mut v, SPRITE_BASE + 7, &[0xAB]);
    assert_eq!(v.sprite_attr[7], 0xAB);
    set_addr(&mut v, PALETTE_BASE + 10, 1);
    assert_eq!(v.read(0x03), 0x34); // reads see VRAM

    v.write(0x05, 0x80); // reset
    assert_eq!(v.palette[5], palette::DEFAULT[5]);
    assert_eq!(v.sprite_attr[7], 0);
    set_addr(&mut v, PALETTE_BASE + 10, 1);
    assert_eq!(v.read(0x03), 0x34, "VRAM survives a reset");
}

#[test]
fn scanline_and_interrupts() {
    let mut v = vera();
    // The IRQ line resets to 0, so LINE is already latched from line 0 (as on
    // hardware); acknowledge it before enabling.
    assert_eq!(v.read(0x07) & 7, INT_LINE);
    v.write(0x07, 7);
    v.write(0x08, 100); // IRQ line 100
    v.write(0x06, 0x03); // VSYNC | LINE
    assert!(!v.irq());
    v.advance(100 * VGA_LINE_CLOCKS - 1);
    assert_eq!(v.read(0x08), 99);
    assert!(!v.irq());
    v.advance(1);
    assert_eq!(v.read(0x08), 100);
    assert_eq!(v.read(0x07) & 7, INT_LINE);
    assert!(v.irq());
    v.write(0x07, INT_LINE);
    assert!(!v.irq());

    v.advance(380 * VGA_LINE_CLOCKS); // line 480: vertical blank
    assert_eq!(v.read(0x07) & 7, INT_VSYNC);
    assert!(v.frame_ready);

    v.advance(35 * VGA_LINE_CLOCKS); // line 515 reads as 511 (bit 8 in IEN)
    assert_eq!(v.read(0x08), 0xFF);
    assert_eq!(v.read(0x06) & 0x40, 0x40);

    v.advance(10 * VGA_LINE_CLOCKS); // wrapped to line 0
    assert_eq!(v.read(0x08), 0);
}

#[test]
fn cpu_clock_conversion() {
    let mut v = Vera::new(50_000_000); // 50 MHz CPU: 2 CPU cycles per VERA clock
    assert_eq!(v.cycles_to_next_line(), 2 * VGA_LINE_CLOCKS);
    v.advance(2 * VGA_LINE_CLOCKS - 1);
    assert_eq!(v.read(0x08), 0);
    assert_eq!(v.cycles_to_next_line(), 1);
    v.advance(1);
    assert_eq!(v.read(0x08), 1);
}

#[test]
fn text_mode_layer() {
    let mut v = vera();
    // Glyph 'A' row 0 = 0b1000_0001 at tile base 0x8000; map at 0.
    poke(&mut v, 0x8000 + b'A' as u32 * 8, &[0x81]);
    poke(&mut v, 0, &[b'A', 0x52]); // bg 5, fg 2
    v.write(0x14, 0x00); // 1bpp, 32x32 map
    v.write(0x15, 0);
    v.write(0x16, ((0x8000u32 >> 11) << 2) as u8);
    v.write(0x09, 0x21); // layer 1, VGA
    next_frame(&mut v);
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 2));
    assert_eq!(px(&v, 1, 0), pal_rgb(&v, 5));
    assert_eq!(px(&v, 7, 0), pal_rgb(&v, 2));

    // Horizontal scroll by 1 moves the glyph left.
    v.write(0x17, 1);
    next_frame(&mut v);
    assert_eq!(px(&v, 6, 0), pal_rgb(&v, 2));
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 5));
}

#[test]
fn tile_mode_palette_offset_and_flip() {
    let mut v = vera();
    // 4bpp 8x8 tile 1 at 0x10000: row 0 = colours 1,2,0,...
    poke(&mut v, 0x10000 + 32, &[0x12, 0x00, 0x00, 0x03]);
    poke(&mut v, 0, &[1, 0x30]); // tile 1, palette offset 3
    v.write(0x0D, 0x02); // L0: 4bpp tiles
    v.write(0x0E, 0);
    v.write(0x0F, ((0x10000u32 >> 11) << 2) as u8);
    v.write(0x09, 0x11);
    next_frame(&mut v);
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 0x31));
    assert_eq!(px(&v, 1, 0), pal_rgb(&v, 0x32));
    assert_eq!(px(&v, 2, 0), pal_rgb(&v, 0), "colour 0 is transparent");
    // H-flip mirrors the row.
    poke(&mut v, 0, &[1, 0x34]);
    next_frame(&mut v);
    assert_eq!(px(&v, 7, 0), pal_rgb(&v, 0x31));
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 0x33));
}

#[test]
fn bitmap_mode_and_scaling() {
    let mut v = vera();
    poke(&mut v, 0, &[7, 8, 9]);
    poke(&mut v, 320, &[10]); // second row of a 320-wide 8bpp bitmap
    v.write(0x0D, 0x07); // L0: 8bpp bitmap, 320 wide
    v.write(0x0F, 0);
    v.write(0x09, 0x11);
    v.write(0x0A, 64); // 2x
    v.write(0x0B, 64);
    next_frame(&mut v);
    v.advance(2 * VGA_LINE_CLOCKS);
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 7));
    assert_eq!(px(&v, 1, 0), pal_rgb(&v, 7));
    assert_eq!(px(&v, 2, 0), pal_rgb(&v, 8));
    assert_eq!(px(&v, 2, 1), pal_rgb(&v, 8));
    assert_eq!(px(&v, 0, 2), pal_rgb(&v, 10));
}

#[test]
fn border_and_active_area() {
    let mut v = vera();
    v.write(0x05, 2); // DCSEL
    v.write(0x09, 8 >> 2); // HSTART 8
    v.write(0x0B, 4 >> 1); // VSTART 4
    v.write(0x05, 0);
    v.write(0x0C, 2); // border colour
    v.write(0x09, 0x01); // VGA, no layers
    next_frame(&mut v);
    v.advance(8 * VGA_LINE_CLOCKS);
    assert_eq!(px(&v, 7, 5), pal_rgb(&v, 2));
    assert_eq!(px(&v, 8, 3), pal_rgb(&v, 2));
    assert_eq!(px(&v, 8, 5), pal_rgb(&v, 0));
    assert_eq!(v.read(0x09), 0x01);
    v.write(0x05, 2);
    assert_eq!(v.read(0x09), 8 >> 2);
}

fn sprite(v: &mut Vera, n: u32, addr: u32, x: u16, y: u16, z: u8, mask: u8) {
    let a = [
        (addr >> 5) as u8,
        (addr >> 13) as u8 & 0xF | 0x80, // 8bpp
        x as u8,
        (x >> 8) as u8,
        y as u8,
        (y >> 8) as u8,
        mask << 4 | z << 2,
        0x00, // 8x8, palette offset 0
    ];
    poke(v, SPRITE_BASE + n * 8, &a);
}

#[test]
fn sprite_priority_and_collisions() {
    let mut v = vera();
    poke(&mut v, 0x1000, &[0x20; 64]); // solid 8x8 of colour 0x20
    poke(&mut v, 0x1040, &[0x30; 64]); // solid 8x8 of colour 0x30
    sprite(&mut v, 0, 0x1000, 0, 0, 1, 0b0011);
    sprite(&mut v, 1, 0x1040, 4, 0, 1, 0b0001);
    sprite(&mut v, 2, 0x1040, 100, 0, 3, 0b0100);
    sprite(&mut v, 3, 0x1000, 100, 0, 1, 0b0100);
    v.write(0x09, 0x41); // sprites, VGA
    next_frame(&mut v);
    // Same Z: the lower-numbered sprite wins.
    assert_eq!(px(&v, 5, 0), pal_rgb(&v, 0x20));
    assert_eq!(px(&v, 9, 0), pal_rgb(&v, 0x30));
    // Higher Z wins regardless of order.
    assert_eq!(px(&v, 100, 0), pal_rgb(&v, 0x30));

    v.advance(480 * VGA_LINE_CLOCKS); // vertical blank latches collisions
    assert_eq!(v.read(0x07) >> 4, 0b0101);
    assert_eq!(v.read(0x07) & INT_SPRCOL, INT_SPRCOL);
}

#[test]
fn sprite_wraps_at_1024() {
    let mut v = vera();
    poke(&mut v, 0x1000, &[0x20; 64]);
    sprite(&mut v, 0, 0x1000, 1020, 1020, 3, 0);
    v.write(0x09, 0x41);
    next_frame(&mut v);
    assert_eq!(px(&v, 3, 3), pal_rgb(&v, 0x20));
    assert_eq!(px(&v, 4, 3), pal_rgb(&v, 0));
}

#[test]
fn layer_priority_with_sprites() {
    let mut v = vera();
    poke(&mut v, 0, &[1, 2]); // L0 8bpp bitmap pixels
    poke(&mut v, 0x1000, &[0x40; 64]);
    sprite(&mut v, 0, 0x1000, 0, 0, 1, 0); // behind layer 0
    v.write(0x0D, 0x07);
    v.write(0x0F, 0);
    v.write(0x09, 0x51); // sprites + L0
    next_frame(&mut v);
    assert_eq!(px(&v, 0, 0), pal_rgb(&v, 1));
    assert_eq!(px(&v, 2, 0), pal_rgb(&v, 0x40), "sprite shows through transparent L0");
}

#[test]
fn audio_fifo() {
    let mut v = vera();
    assert_eq!(v.read(0x1B) & 0xC0, 0x40); // empty
    v.write(0x06, INT_AFLOW);
    assert!(v.irq(), "AFLOW is a level: an empty FIFO asserts it");
    for _ in 0..1024 {
        v.write(0x1D, 0);
    }
    assert!(!v.irq());
    v.write(0x1C, 128); // 48.8 kHz, 8-bit mono
    v.advance(512 * 10);
    assert!(v.irq(), "drained below a quarter");
    v.write(0x1B, 0x80); // FIFO reset
    assert_eq!(v.read(0x1B) & 0x40, 0x40);
}

/// Full 9-bit SCANLINE (bit 8 lives in IEN).
fn scanline(v: &mut Vera) -> u16 {
    v.read(0x08) as u16 | (v.read(0x06) as u16 >> 6 & 1) << 8
}

#[test]
fn interlaced_timing() {
    let mut v = vera();
    v.write(0x09, 0x02); // NTSC
    while scanline(&mut v) > 1 {
        v.advance(NTSC_LINE_CLOCKS); // run to the start of a field
    }
    let field = v.read(0x09) >> 7;
    let first = scanline(&mut v);
    v.advance(NTSC_LINE_CLOCKS);
    assert_eq!(scanline(&mut v), first + 2, "every other line");
    assert_eq!(first & 1, field as u16);
    // A field is 262 or 263 lines of 1588 clocks; the next one has the other parity.
    while scanline(&mut v) > 1 {
        v.advance(NTSC_LINE_CLOCKS);
    }
    assert_eq!(v.read(0x09) >> 7, field ^ 1);
    assert_eq!(scanline(&mut v) & 1, (field ^ 1) as u16);
}
