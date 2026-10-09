| Address        | Size   | Purpose                                |
|----------------|--------|----------------------------------------|
| `0x0000_0000`  | 32 KB* | RAM (*for now). ROM overlays it at reset until SYSCTRL BOOT is written |
| `0xFD00_0000`  | –      | VRAM (reserved)                        |
| `0xFE00_0000`  | 8 B    | UART: `+0` DATA, `+4` STATUS (bit0 tx_ready, bit1 rx_valid) |
| `0xFE00_1000`  | 12 B   | SYSCTRL: `+0` BOOT (any write drops overlay), `+4` ID, `+8` LEDS |
| `0xFE00_2000`  | –      | SCSI (reserved)                        |
| `0xFE00_3000`  | –      | Video registers (reserved)             |
| `0xFFF0_0000`  | 16 KB  | Boot ROM (mirrored across 1 MB)        |
 