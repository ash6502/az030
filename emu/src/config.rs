//! Machine configuration, loaded from a TOML file (see `az030.toml`).

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Highest RAM size the memory map allows (RAM must stay below VRAM at 0xFD00_0000;
/// the board tops out at 1 GB anyway).
pub const MAX_RAM: u64 = 1 << 30;
/// The ROM window is 1 MB; images are mirrored across it.
pub const MAX_ROM: u64 = 1 << 20;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub machine: MachineConfig,
    pub rom: RomConfig,
    #[serde(default)]
    pub scsi: ScsiConfig,
    #[serde(default)]
    pub timer: TimerConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TimerConfig {
    /// When false the timer block at 0xFE004000 is unmapped (bus error).
    pub enabled: bool,
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MachineConfig {
    /// RAM size: an integer number of bytes, or a string such as "32K", "128M", "1G".
    pub ram_size: Size,
    /// CPU clock in MHz, used to pace emulation to real time.
    pub clock_mhz: f64,
    /// When false, run as fast as the host allows.
    pub throttle: bool,
    /// Value returned by the SYSCTRL ID register.
    pub sysctrl_id: u32,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            ram_size: Size(128 << 20),
            clock_mhz: 25.0,
            throttle: true,
            sysctrl_id: 0x415A_3330, // "AZ30"
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomConfig {
    /// Boot ROM image (raw binary). Relative paths are resolved against the config file.
    pub file: PathBuf,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScsiConfig {
    /// When false the SCSI register block is unmapped (bus error), like on the current FPGA build.
    pub enabled: bool,
    /// SCSI ID of the host adapter (default 7).
    pub host_id: Option<u8>,
    #[serde(rename = "disk")]
    pub disks: Vec<DiskConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskConfig {
    /// Target ID, 0-7.
    pub id: u8,
    /// Raw disk image file. Relative paths are resolved against the config file.
    pub image: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    /// Create the image with this size if it does not exist yet.
    #[serde(default)]
    pub create_size: Option<Size>,
}

/// A byte count that accepts K/M/G suffixes in the config file.
#[derive(Debug, Clone, Copy)]
pub struct Size(pub u64);

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(n) => Ok(Size(n)),
            Raw::Str(s) => parse_size(&s).map(Size).map_err(serde::de::Error::custom),
        }
    }
}

pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&s[..s.len() - 1], 1u64 << 10),
        Some('M') => (&s[..s.len() - 1], 1 << 20),
        Some('G') => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let n: u64 = if let Some(hex) = num.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        num.trim().parse()
    }
    .map_err(|_| format!("invalid size {s:?}"))?;
    n.checked_mul(mult).ok_or_else(|| format!("size {s:?} overflows"))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut cfg: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        cfg.rom.file = base.join(&cfg.rom.file);
        for d in &mut cfg.scsi.disks {
            d.image = base.join(&d.image);
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), String> {
        let ram = self.machine.ram_size.0;
        if ram < 4096 || ram > MAX_RAM || ram % 4096 != 0 {
            return Err(format!(
                "machine.ram_size must be a multiple of 4K between 4K and 1G (got {ram})"
            ));
        }
        if self.machine.clock_mhz <= 0.0 {
            return Err("machine.clock_mhz must be positive".into());
        }
        let host = self.scsi.host_id.unwrap_or(7);
        if host > 7 {
            return Err("scsi.host_id must be 0-7".into());
        }
        let mut seen = [false; 8];
        for d in &self.scsi.disks {
            if d.id > 7 || d.id == host {
                return Err(format!("scsi disk id {} is invalid or collides with the host", d.id));
            }
            if std::mem::replace(&mut seen[d.id as usize], true) {
                return Err(format!("scsi disk id {} used twice", d.id));
            }
        }
        Ok(())
    }
}
