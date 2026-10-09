//! Timer / interrupt controller at 0xFE00_4000 (emulator-defined, like the SCSI block).
//!
//! The memory map has no interrupt sources yet, so this block supplies what a
//! preemptive OS needs: a periodic tick, a microsecond clock, a wall-clock RTC, and
//! interrupt enables for the tick and UART receive. Interrupts are autovectored.
//! All registers are 32-bit.
//!
//! | Offset | R/W | Name    | Meaning                                                  |
//! |--------|-----|---------|----------------------------------------------------------|
//! | `0x00` | RW  | CTRL    | bit0 tick enable                                         |
//! | `0x04` | RW  | PERIOD  | tick period in microseconds (100-1000000, default 10000) |
//! | `0x08` | RW  | STATUS  | bit0 tick pending; write 1 to clear                      |
//! | `0x0C` | R   | USEC_LO | microseconds since reset, low word (latches USEC_HI)     |
//! | `0x10` | R   | USEC_HI | high word, as latched by the last USEC_LO read           |
//! | `0x14` | R   | RTC     | host wall clock, seconds since 1970-01-01 UTC            |
//! | `0x18` | RW  | INTEN   | bit0 tick -> IPL 6, bit1 UART RX data -> IPL 4           |
//! | `0x1C` | R   | INTPEND | bit0 tick, bit1 UART RX (pending and enabled)            |
//! | `0x20` | W   | POWER   | write `0x504F4646` ("POFF") to power off                 |
//! | `0x24` | R   | ID      | `0x54494D52` ("TIMR")                                    |

pub const TICK_IPL: u8 = 6;
pub const UART_IPL: u8 = 4;
const POWER_OFF_MAGIC: u32 = 0x504F_4646;

pub struct Timer {
    ctrl: u32,
    period_us: u32,
    pending: bool,
    next_tick: u64,
    inten: u32,
    now_us: u64,
    latched_hi: u32,
    pub power_off: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            period_us: 10_000,
            pending: false,
            next_tick: 0,
            inten: 0,
            now_us: 0,
            latched_hi: 0,
            power_off: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self { now_us: self.now_us, ..Self::new() };
    }

    /// Move emulated time forward to `now_us`, raising the tick if a period elapsed.
    pub fn advance(&mut self, now_us: u64) {
        self.now_us = now_us;
        if self.ctrl & 1 != 0 && now_us >= self.next_tick {
            self.pending = true;
            // Skip missed ticks instead of delivering a burst.
            let p = self.period_us as u64;
            self.next_tick = now_us + p - (now_us - self.next_tick) % p;
        }
    }

    /// Interrupt level to present to the CPU.
    pub fn irq_level(&self, uart_rx: bool) -> u8 {
        if self.pending && self.inten & 1 != 0 {
            TICK_IPL
        } else if uart_rx && self.inten & 2 != 0 {
            UART_IPL
        } else {
            0
        }
    }

    pub fn read(&mut self, off: u32, uart_rx: bool) -> Option<u32> {
        Some(match off {
            0x00 => self.ctrl,
            0x04 => self.period_us,
            0x08 => self.pending as u32,
            0x0C => {
                self.latched_hi = (self.now_us >> 32) as u32;
                self.now_us as u32
            }
            0x10 => self.latched_hi,
            0x14 => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as u32),
            0x18 => self.inten,
            0x1C => (self.pending && self.inten & 1 != 0) as u32 | ((uart_rx && self.inten & 2 != 0) as u32) << 1,
            0x20 => 0,
            0x24 => 0x5449_4D52,
            _ => return None,
        })
    }

    pub fn write(&mut self, off: u32, val: u32) -> Option<()> {
        match off {
            0x00 => {
                if self.ctrl & 1 == 0 && val & 1 != 0 {
                    self.next_tick = self.now_us + self.period_us as u64;
                }
                self.ctrl = val & 1;
            }
            0x04 => self.period_us = val.clamp(100, 1_000_000),
            0x08 => {
                if val & 1 != 0 {
                    self.pending = false;
                }
            }
            0x18 => self.inten = val & 3,
            0x20 => {
                if val == POWER_OFF_MAGIC {
                    self.power_off = true;
                }
            }
            0x0C | 0x10 | 0x14 | 0x1C | 0x24 => {}
            _ => return None,
        }
        Some(())
    }
}
