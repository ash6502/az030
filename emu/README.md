# az030 emulator

An emulator for the az030 built on [m68k-rs](https://github.com/benletchford/m68k-rs).
It emulates a 68030 with an FPU (68881/68882), the memory map in `../MEM_MAP.md`, the UART
as your terminal, an optional virtual SCSI bus, and optionally the VERA video module in a
window.

```sh
cargo run --release                      # uses ./az030.toml
cargo run --release -- -c other.toml -v  # -v reports LED changes on stderr
cargo run --release -- --trace 200       # print the last 200 instructions (outside ROM) on exit
cargo run --release -- --vera            # also emulate the VERA and open a window for it
```

The UART connects to the terminal. **Ctrl-] q** quits, **Ctrl-] r** resets the machine, and
**Ctrl-] Ctrl-]** sends a literal Ctrl-]. Ctrl-C goes to the guest.

## Platforms

The emulator builds and runs natively on macOS, Linux and Windows with a plain
`cargo build --release`. On Windows, use Windows Terminal or a recent console host
(Windows 10 1809 or later) so the guest's ANSI escape sequences render. The emulator
switches the console to VT input/output and UTF-8 while it runs, then restores the
original settings on exit.

CI (`.github/workflows/emu.yml`) builds and smoke-tests all four targets: Linux x86-64,
Linux ARM64, Windows x86-64 and macOS. It uploads ready-to-run artifacts.

### Cross-compiling

From any host, [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) with
[Zig](https://ziglang.org) on `PATH` covers Linux and Windows without a separate C toolchain:

```sh
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu    # Linux x86-64
cargo zigbuild --release --target aarch64-unknown-linux-gnu   # Linux ARM64 (Raspberry Pi etc.)
cargo zigbuild --release --target x86_64-pc-windows-gnu       # Windows x86-64 (.exe)
```

Binaries land in `target/<target>/release/`. Ship `az030.toml` and `../rom/boot.bin` with
them, keeping the same relative layout, or edit `rom.file` in the config.

## Config (`az030.toml`)

| Key                     | Meaning                                                    |
|-------------------------|------------------------------------------------------------|
| `machine.ram_size`      | `"32K"`, `"128M"`, `"1G"`, or bytes. 4K–1G                 |
| `machine.clock_mhz`     | Emulated clock, used to pace emulation (default 25)        |
| `machine.throttle`      | `false` runs as fast as possible                           |
| `machine.sysctrl_id`    | Value of the SYSCTRL ID register (default `0x415A3330`, "AZ30") |
| `rom.file`              | Raw boot ROM image, up to 1 MB, mirrored over `0xFFF00000+` |
| `scsi.enabled`          | `false` leaves `0xFE002000` unmapped (bus error)           |
| `timer.enabled`         | `false` leaves `0xFE004000` unmapped (default `true`)      |
| `scsi.host_id`          | Host adapter ID (default 7)                                |
| `[[scsi.disk]]`         | `id`, `image`, `read_only`, `create_size`                  |

Relative paths resolve against the config file's directory.

## Emulated hardware

* **RAM** at 0, sized from the config. At reset the ROM overlays it for reads (writes still
  reach RAM) until anything is written to SYSCTRL BOOT.
* **UART** at `0xFE000000`: `+0` DATA, `+4` STATUS (bit0 tx_ready, always set; bit1 rx_valid).
* **SYSCTRL** at `0xFE001000`: `+0` BOOT, `+4` ID, `+8` LEDS.
* **ROM** at `0xFFF00000`, mirrored across 1 MB. Writes are ignored.
* **VERA** at `0xFE003000`, only with `--vera` or `--vera-snapshot` (see below).
* Peripheral registers accept 32-bit accesses only. Byte/word accesses to them and all
  accesses to unmapped space (including the reserved VRAM block, and the VERA block when
  the VERA is off) raise a bus error.

Not emulated yet: the PS/2 keyboard, the floppy, and Ethernet. None of them have
registers in the memory map.

## VERA (`0xFE003000`, opt-in)

The [VERA](https://github.com/fvdhoef/vera-module) is emulated only when asked for on the
command line, so the emulator stays a plain terminal program by default:

| Option                  | Effect                                                           |
|-------------------------|------------------------------------------------------------------|
| `--vera`                | Emulate the VERA and show its 640x480 output in a window         |
| `--vera-scale N`        | Initial window size, 1, 2 (default) or 4 times; it can be resized |
| `--vera-snapshot FILE`  | Emulate the VERA (window or not) and save its last frame as a PNG on exit |

Closing the window quits the emulator. The az030's PS/2 keyboard has no registers yet, so
keys typed into the window go to the **UART**, as the bytes a VT100-style terminal sends
(Enter = CR, Backspace = DEL, arrows = `ESC [ A`-`D`, Ctrl+letter = control codes). Ctrl-]
works there as it does in the terminal. The UART output still goes to the terminal.

**Register mapping.** VERA register `n` (0-31, as numbered in the
[programmer's reference](https://github.com/fvdhoef/vera-module/blob/master/doc/VERA%20Programmer's%20Reference.md))
is at `0xFE003000 + 4*n`. Like every az030 peripheral it is accessed as a long; the data is
the low byte. So `ADDRx_L` is `0xFE003000`, `DATA0` is `0xFE00300C`, `CTRL` is
`0xFE003014`, `DC_VIDEO` is `0xFE003024`, and so on. The VERA IRQ line is autovectored at
**IPL 3** (vector 27), below the UART (4) and the timer tick (6). VRAM is reached only
through the data ports, as on the real module, so the `0xFD000000` VRAM window stays
reserved.

**Emulated:** the reference v0.9 register set, with details checked against the FPGA
sources:
* VRAM, both data ports with every increment, DECR and read prefetch;
* both layers in all tile and bitmap modes;
* 128 sprites with Z-depth priority, flips, collision masks, 1024-pixel wraparound and the
  per-line render budget;
* the palette, border, active area and H/V scaling;
* scanline-accurate VSYNC/LINE/SPRCOL/AFLOW interrupts and SCANLINE, and interlaced field
  timing in the NTSC/RGB output modes (composite with chroma disabled shows in greyscale);
* the PCM FIFO's level and drain rate.

Rendering happens line by line in emulated time, so mid-frame register changes take effect
at the right scanline.

**Not emulated:**
* Sound: the PSG and PCM accept writes but produce no audio.
* The SD card on the SPI port: none is attached, so reads return `0xFF`.
* NTSC colour artefacts.
* VRAM's random power-on contents: it starts zeroed.
* The sprite budget is an approximation of the hardware's fetch timing.

**Demo:** `rom/examples/vera_demo.s` exercises text, tiles, sprites, scaling and both
interrupts. To run it:

```sh
cargo run --release -- mkdisk disks/vera.img 1M --boot ../rom/examples/vera_demo.bin --load 0x10000
```

Then add `disks/vera.img` as a `[[scsi.disk]]` and run with `--vera`. In the demo, `z`
toggles 2x zoom, `c` toggles composite (chroma off), and `q` returns to the monitor.

## Timer / interrupt controller (`0xFE004000`)

Also emulator-defined, so a preemptive OS has a clock. It provides a periodic tick, a
microsecond counter, a wall-clock RTC and interrupt enables. Interrupts are
autovectored: the tick is IPL 6 (vector 30), UART receive is IPL 4 (vector 28).
Levels are re-evaluated once per millisecond of emulated time. Registers are 32-bit.

| Offset | R/W | Register | Meaning                                                   |
|--------|-----|----------|-----------------------------------------------------------|
| `0x00` | RW  | CTRL     | bit0 tick enable                                          |
| `0x04` | RW  | PERIOD   | tick period in µs (100–1000000, default 10000 = 100 Hz)   |
| `0x08` | RW  | STATUS   | bit0 tick pending; write 1 to acknowledge                 |
| `0x0C` | R   | USEC_LO  | µs since reset, low word (reading it latches USEC_HI)     |
| `0x10` | R   | USEC_HI  | high word                                                 |
| `0x14` | R   | RTC      | host wall clock, seconds since 1970-01-01 UTC             |
| `0x18` | RW  | INTEN    | bit0 tick → IPL 6, bit1 UART RX data → IPL 4              |
| `0x1C` | R   | INTPEND  | bit0 tick, bit1 UART RX (pending and enabled)             |
| `0x20` | W   | POWER    | write `0x504F4646` ("POFF") to power off (emulator exits) |
| `0x24` | R   | ID       | `0x54494D52` ("TIMR")                                     |

## Virtual SCSI (`0xFE002000`)

The memory map only reserves this block, so the emulator defines a simple DMA host adapter
for it. Set the target, CDB and buffer, then write EXECUTE. The command finishes (DONE set)
before the write returns. Registers are 32-bit.

| Offset | R/W | Register | Meaning                                                      |
|--------|-----|----------|--------------------------------------------------------------|
| `0x00` | W   | CMD      | 1 EXECUTE, 2 BUS RESET, 3 CLEAR STATUS                       |
| `0x04` | R   | STATUS   | bit0 BUSY, bit1 DONE, bit2 NO TARGET, bit3 DMA ERROR, bits 15:8 SCSI status (0 GOOD, 2 CHECK CONDITION) |
| `0x08` | RW  | TARGET   | bits 2:0 ID, bits 10:8 LUN                                   |
| `0x0C` | RW  | DMA_ADDR | RAM address of the data buffer                               |
| `0x10` | RW  | DMA_LEN  | Buffer size in bytes                                         |
| `0x14` | R   | XFER     | Bytes moved by the last command                              |
| `0x18` | RW  | CDB_LEN  | 6/10/12/16                                                   |
| `0x1C` | R   | ID       | `0x53435349` ("SCSI")                                        |
| `0x20`–`0x2F` | RW | CDB | CDB bytes 0–15, big-endian, four per register          |

Disks are direct-access devices with 512-byte blocks. They support TEST UNIT READY,
REQUEST SENSE, INQUIRY, MODE SENSE(6), READ CAPACITY(10), READ/WRITE(6/10), VERIFY,
SYNCHRONIZE CACHE, START STOP UNIT, PREVENT ALLOW and FORMAT UNIT (a no-op). Any other
opcode returns CHECK CONDITION / ILLEGAL REQUEST.

## Boot disks

```sh
cargo run --release -- mkdisk disks/hello.img 1M --boot ../rom/examples/hello.bin --load 0x10000
```

This writes a boot block at LBA 0 (all fields big-endian), with the program from LBA 1 on:

| Offset | Field                         |
|--------|-------------------------------|
| `0x00` | `"AZ30BOOT"`                  |
| `0x08` | load address                  |
| `0x0C` | entry address (`--entry`, defaults to the load address) |
| `0x10` | program length in blocks      |
| `0x14` | program length in bytes       |

The boot ROM reads the program to the load address and calls the entry point with
`d0` = SCSI ID and `d1` = RAM size. See `../rom/boot.s` for the monitor commands and the
TRAP #15 system calls.
