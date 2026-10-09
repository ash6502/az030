//! The az030 OS kernel.
//!
//! A small UNIX-like kernel for the az030 (MC68030 + MC68882): preemptive
//! multitasking with protected address spaces (68030 PMMU), a UNIX file system
//! (azfs) on SCSI disks, pipes, signals, a terminal line discipline, and ELF
//! executables. See `os/README.md` for the big picture.

#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod console;
mod arch;
mod bio;
mod exec;
mod file;
mod fs;
mod mem;
mod pipe;
mod proc;
mod scsi;
mod signal;
mod syscall;
mod timer;
mod trap;
mod tty;
mod util;
mod vm;

use core::panic::PanicInfo;

pub const VERSION: &str = "0.1";

/// Written by the bootloader (see `os/boot/boot.s`).
#[repr(C)]
pub struct BootInfo {
    pub magic: u32,
    pub ram_size: u32,
    pub boot_id: u32,
    pub root_lba: u32,
    pub root_blocks: u32,
    pub kernel_start: u32,
    pub kernel_size: u32,
    pub cmdline: [u8; 128],
}

const BOOT_MAGIC: u32 = 0x424F_4F54;

/// Kernel command line options.
pub struct Config {
    /// SCSI ID the system was booted from.
    pub boot_id: u32,
    pub root_dev: u32,
    pub init: alloc::string::String,
    pub quiet: bool,
}

pub static CONFIG: util::Global<Option<Config>> = util::Global::new(None);

pub fn config() -> &'static Config {
    CONFIG.get().as_ref().unwrap()
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain(bi: &BootInfo) -> ! {
    console::init();
    if bi.magic != BOOT_MAGIC {
        panic!("bad boot info");
    }
    arch::init();
    kprintln!();
    kprintln!("az030 UNIX {} (MC68030)", VERSION);
    mem::init(bi.ram_size);
    let cmdline = util::cstr(&bi.cmdline);
    let mut cfg = Config { boot_id: bi.boot_id, root_dev: azsys::makedev(azsys::dev::SD_MAJOR, bi.boot_id), init: "/sbin/init".into(), quiet: false };
    for opt in cmdline.split_whitespace() {
        match opt.split_once('=') {
            Some(("root", v)) => {
                if let Some(id) = v.strip_prefix("sd").and_then(|n| n.parse::<u32>().ok()) {
                    cfg.root_dev = azsys::makedev(azsys::dev::SD_MAJOR, id);
                }
            }
            Some(("init", v)) => cfg.init = v.into(),
            None if opt == "quiet" => cfg.quiet = true,
            _ => kprintln!("kernel: ignoring option {}", opt),
        }
    }
    *CONFIG.get() = Some(cfg);
    vm::init();
    arch::init_fpu();
    timer::init();
    tty::init();
    scsi::init();
    bio::init(bi.ram_size);
    fs::init(bi.root_lba, bi.root_blocks);
    proc::start_init()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::irq_off();
    console::emergency();
    kprintln!();
    kprintln!("kernel panic: {}", info);
    if let Some(p) = proc::current_opt() {
        kprintln!("  in process {} ({})", p.pid, p.name());
    }
    kprintln!("System halted.");
    arch::halt()
}
