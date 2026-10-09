# az030 emulator

An emulator for the az030 built on [m68k-rs](https://github.com/benletchford/m68k-rs).
It emulates a 68030 with an FPU (68881/68882), the memory map in `../MEM_MAP.md`, the UART
as your terminal, and an optional virtual SCSI bus.

```sh
cargo run --release                      # uses ./az030.toml
cargo run --release -- -c other.toml -v  # -v reports LED changes on stderr
cargo run --release -- --trace 200       # print the last 200 instructions (outside ROM) on exit
```

The UART connects to the terminal. **Ctrl-] q** quits, **Ctrl-] r** resets the machine, and
**Ctrl-] Ctrl-]** sends a literal Ctrl-]. Ctrl-C goes to the guest.

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
* Peripheral registers accept 32-bit accesses only. Byte/word accesses to them and all
  accesses to unmapped space (including the reserved VRAM and video blocks) raise a bus error.

Not emulated yet: the VERA/video, PS/2 keyboard, the floppy, and Ethernet. None of them
have registers in the memory map.

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
