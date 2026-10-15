//! PCM FIFO model. No sound is produced; the FIFO drains at the programmed sample rate so
//! FIFO Full/Empty and the AFLOW interrupt behave as on hardware.

/// Usable FIFO capacity (4 KB ring buffer, one slot kept free).
const FIFO_SIZE: u32 = 4095;
/// VERA clocks per output sample (25 MHz / 512 = 48828.125 Hz).
const SAMPLE_CLOCKS: u64 = 512;

#[derive(Default)]
pub(super) struct Audio {
    /// AUDIO_CTRL bits 5:0 (16-bit, stereo, PCM volume).
    ctrl: u8,
    pub rate: u8,
    fifo_len: u32,
    phase: u32,
    clock: u64,
}

impl Audio {
    pub fn ctrl_read(&self) -> u8 {
        let full = (self.fifo_len >= FIFO_SIZE) as u8;
        let empty = (self.fifo_len == 0) as u8;
        full << 7 | empty << 6 | self.ctrl
    }

    pub fn ctrl_write(&mut self, v: u8) {
        if v & 0x80 != 0 {
            self.fifo_len = 0;
        }
        self.ctrl = v & 0x3F;
    }

    pub fn push(&mut self, _byte: u8) {
        if self.fifo_len < FIFO_SIZE {
            self.fifo_len += 1;
        }
    }

    /// AFLOW: less than a quarter full.
    pub fn fifo_low(&self) -> bool {
        self.fifo_len < (FIFO_SIZE + 1) / 4
    }

    pub fn advance(&mut self, clocks: u64) {
        self.clock += clocks;
        let samples = self.clock / SAMPLE_CLOCKS;
        self.clock %= SAMPLE_CLOCKS;
        if self.rate == 0 || self.rate > 128 || self.fifo_len == 0 {
            return;
        }
        // Each output sample adds RATE/128 of an input sample.
        let total = self.phase as u64 + samples * self.rate as u64;
        self.phase = (total % 128) as u32;
        let consumed = total / 128;
        let bytes_per = 1u64 << ((self.ctrl >> 4 & 1) + (self.ctrl >> 5 & 1));
        let bytes = (consumed * bytes_per).min(self.fifo_len as u64 / bytes_per * bytes_per);
        self.fifo_len -= bytes as u32;
    }
}
