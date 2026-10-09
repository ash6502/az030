# az030 - a 68030-based homebrew computer
**! AI DISCLAIMER !**

Claude Code was used in the creation of this

# info

| Plan item                     | Status                                   |
|-------------------------------|-------------------------------------------------------|
| 16–50 MHz clock               | CPU clock generated in FPGA (25 MHz from a 50 MHz osc) |
| 128 MB – 1 GB RAM             | 32 KB BRAM stand-in; SDRAM/DDR controller is next     |
| Glue logic in FPGA            | bus bridge, address decode, reset, UART, sysctrl    |
| Video in FPGA                 | Address space reserved (VRAM `0xFD000000`, regs `0xFE003000`) |
| SCSI                          | Address space reserved (`0xFE002000`)                 |
| UNIX-like OS                  | Needs MMU → the 68030 (not 68EC030) was chosen for this |


## Memory map
| Address        | Size   | What                                   |
|----------------|--------|----------------------------------------|
| `0x0000_0000`  | 32 KB* | RAM (*for now). ROM overlays it at reset until SYSCTRL BOOT is written |
| `0xFD00_0000`  | –      | VRAM (reserved)                        |
| `0xFE00_0000`  | 8 B    | UART: `+0` DATA, `+4` STATUS (bit0 tx_ready, bit1 rx_valid) |
| `0xFE00_1000`  | 12 B   | SYSCTRL: `+0` BOOT (any write drops overlay), `+4` ID, `+8` LEDS |
| `0xFE00_2000`  | –      | SCSI (reserved)                        |
| `0xFE00_3000`  | –      | Video registers (reserved)             |
| `0xFFF0_0000`  | 16 KB  | Boot ROM (mirrored across 1 MB)        |

Peripheral registers are **32-bit access only** (`move.l`). Unmapped addresses raise a bus error after 256 FPGA clocks.
