mod bus;
mod config;
mod scsi;
mod terminal;
mod timer;
mod vera;

use bus::Bus;
use config::Config;
use m68k::{CpuCore, CpuType};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

/// Ctrl-] : emulator escape key (like telnet).
const ESCAPE: u8 = 0x1D;

const USAGE: &str = "\
az030 emulator

usage:
  az030-emu [-c CONFIG] [-v] [--max-cycles N]
  az030-emu mkdisk IMAGE SIZE [--boot PROGRAM.bin --load ADDR [--entry ADDR]]

options:
  -c, --config FILE   machine config (default: az030.toml)
  -v, --verbose       report LED changes and CPU halts on stderr
  --max-cycles N      stop after N CPU cycles (for scripted runs)
  --trace N           keep the last N executed instructions (outside ROM) and print
                      them on exit
  --stop-at ADDR      stop when the PC reaches ADDR (hex; useful with --trace)
  --dump ADDR:LEN     hex-dump physical memory when the emulator stops
  --profile FILE      sample the PC and write a histogram (address and count per line) to FILE
  --vera              emulate the VERA video module at 0xFE003000 and show its output in a
                      window; keys typed in the window go to the UART
  --vera-scale N      initial window size: 1, 2 (default) or 4 times 640x480
  --vera-snapshot FILE
                      emulate the VERA (with or without --vera) and save its last complete
                      frame to FILE as a PNG on exit

console keys:
  Ctrl-] q   quit           Ctrl-] r   reset
  Ctrl-] Ctrl-]   send a literal Ctrl-]
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.first().map(String::as_str) == Some("mkdisk") {
        mkdisk(&args[1..])
    } else {
        run(&args)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("az030-emu: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_num(s: &str) -> Result<u64, String> {
    config::parse_size(s)
}

fn run(args: &[String]) -> Result<(), String> {
    let mut config_path = PathBuf::from("az030.toml");
    let mut verbose = false;
    let mut max_cycles: Option<u64> = None;
    let mut trace_len = 0usize;
    let mut stop_at: Option<u32> = None;
    let mut profile: Option<String> = None;
    let mut samples: std::collections::HashMap<u32, u64> = Default::default();
    let mut dumps: Vec<(u32, u32)> = Vec::new();
    let mut vera_window = false;
    let mut vera_scale = 2u32;
    let mut vera_snapshot: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-c" | "--config" => {
                config_path = it.next().ok_or("--config needs a file")?.into();
            }
            "-v" | "--verbose" => verbose = true,
            "--max-cycles" => {
                max_cycles = Some(parse_num(it.next().ok_or("--max-cycles needs N")?)?);
            }
            "--dump" => {
                let a = it.next().ok_or("--dump needs ADDR:LEN")?;
                let (addr, len) = a.split_once(':').ok_or("--dump needs ADDR:LEN")?;
                let addr = u32::from_str_radix(addr.trim_start_matches("0x"), 16).map_err(|_| "bad --dump address")?;
                dumps.push((addr, parse_num(len)? as u32));
            }
            "--stop-at" => {
                let a = it.next().ok_or("--stop-at needs an address")?;
                let a = a.trim_start_matches("0x");
                stop_at = Some(u32::from_str_radix(a, 16).map_err(|_| "bad --stop-at address")?);
            }
            "--trace" => {
                trace_len = parse_num(it.next().ok_or("--trace needs N")?)? as usize;
            }
            "--vera" => vera_window = true,
            "--vera-scale" => {
                vera_scale = parse_num(it.next().ok_or("--vera-scale needs 1, 2 or 4")?)? as u32;
            }
            "--vera-snapshot" => {
                vera_snapshot = Some(it.next().ok_or("--vera-snapshot needs a file")?.clone());
            }
            "--profile" => {
                profile = Some(it.next().ok_or("--profile needs a file")?.clone());
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            _ => return Err(format!("unknown argument {a:?}\n\n{USAGE}")),
        }
    }

    let cfg = Config::load(&config_path)?;
    let rom = std::fs::read(&cfg.rom.file)
        .map_err(|e| format!("cannot read ROM {}: {e}", cfg.rom.file.display()))?;
    if rom.is_empty() || rom.len() as u64 > config::MAX_ROM {
        return Err(format!("ROM image must be 1 byte to 1 MB (got {})", rom.len()));
    }
    let scsi = if cfg.scsi.enabled { Some(scsi::Scsi::new(&cfg.scsi.disks)?) } else { None };
    let timer = cfg.timer.enabled.then(timer::Timer::new);
    let mut bus = Bus::new(cfg.machine.ram_size.0 as usize, rom, cfg.machine.sysctrl_id, scsi, timer);

    let cpu_hz = (cfg.machine.clock_mhz * 1e6).round() as u64;
    if vera_window || vera_snapshot.is_some() {
        bus.vera = Some(vera::Vera::new(cpu_hz));
    }
    // Open the window before the terminal goes raw, so an error prints normally.
    let mut display = if vera_window { Some(vera::display::Display::open(vera_scale)?) } else { None };
    let mut last_frame: Vec<u32> = Vec::new();

    let mut cpu = CpuCore::new();
    cpu.set_cpu_type(CpuType::M68030);
    cpu.fpu_present = true;
    cpu.reset(&mut bus);

    let raw = terminal::RawMode::enable();
    if raw.is_tty() {
        eprint!("az030: {} RAM, {} MHz. Ctrl-] q to quit, Ctrl-] r to reset.\r\n",
            human(cfg.machine.ram_size.0), cfg.machine.clock_mhz);
    }
    let input = terminal::spawn_input(&raw);

    // Run in 1 ms slices of emulated time, servicing the console in between.
    let clock_hz = cfg.machine.clock_mhz * 1e6;
    let slice = ((clock_hz / 1000.0) as i32).max(100);
    let start = Instant::now();
    let mut total_cycles: u64 = 0;
    let mut escape = false;
    let mut input_open = true;
    let mut was_halted = false;
    let mut trace: std::collections::VecDeque<(u32, u16, [u32; 16])> = Default::default();

    'outer: loop {
        let slice_start = total_cycles;
        let mut vera_synced = false;
        if cpu.is_halted() {
            if !was_halted && verbose {
                eprint!("\r\n[az030] CPU halted (double bus fault) at PC={:08X}\r\n", cpu.pc);
            }
            was_halted = true;
            total_cycles += slice as u64;
        } else if trace_len > 0 {
            let mut done = 0i32;
            while done < slice && !cpu.is_halted() {
                if stop_at == Some(cpu.pc) {
                    eprint!("\r\n[az030] reached {:08X}\r\n", cpu.pc);
                    break 'outer;
                }
                // ROM code is not traced, so the log shows the program, not the monitor.
                if cpu.pc < 0xFFF0_0000 {
                    if trace.len() == trace_len {
                        trace.pop_front();
                    }
                    trace.push_back((cpu.pc, cpu.get_sr(), cpu.dar));
                }
                // a handler that declines everything: traps take their real exceptions
                struct NoHle;
                impl m68k::HleHandler for NoHle {}
                done += match cpu.step_with_hle_handler(&mut bus, &mut NoHle) {
                    m68k::StepResult::Ok { cycles } => cycles.max(1),
                    _ => 4,
                };
            }
            total_cycles += done as u64;
        } else if profile.is_some() {
            // execute in small steps and sample the PC after each
            let mut done = 0i32;
            while done < slice && !cpu.is_halted() {
                done += cpu.execute(&mut bus, 200).max(1);
                *samples.entry(cpu.pc).or_insert(0) += 1;
            }
            total_cycles += done as u64;
        } else {
            // With the VERA on, stop at every scanline so its interrupts and raster
            // timing land where they would on hardware.
            let mut done = 0i32;
            while done < slice && !cpu.is_halted() {
                let mut chunk = slice - done;
                if let Some(v) = &bus.vera {
                    chunk = chunk.min(v.cycles_to_next_line().clamp(1, i32::MAX as u64) as i32);
                }
                let ran = cpu.execute(&mut bus, chunk).max(1);
                if let Some(v) = &mut bus.vera {
                    v.advance(ran as u64);
                    cpu.set_irq(bus.irq_level());
                }
                done += ran;
            }
            total_cycles += done as u64;
            vera_synced = true;
        }
        if !vera_synced && let Some(v) = &mut bus.vera {
            v.advance(total_cycles - slice_start);
        }
        bus.flush_tx();

        if let Some(t) = &mut bus.timer {
            t.advance((total_cycles as f64 / cfg.machine.clock_mhz) as u64);
            if t.power_off {
                break 'outer;
            }
        }
        cpu.set_irq(bus.irq_level());

        if bus.leds_changed && verbose {
            eprint!("\r\n[az030] LEDS = {:08b}\r\n", bus.leds & 0xFF);
        }
        bus.leds_changed = false;

        // Gather input: (byte, whether Ctrl-] acts as the escape key for it).
        let mut incoming: Vec<(u8, bool)> = Vec::new();
        while input_open {
            match input.try_recv() {
                Ok(b) => incoming.push((b, raw.is_tty())),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => input_open = false,
            }
        }
        if let Some(v) = &mut bus.vera {
            if v.frame_ready && vera_snapshot.is_some() {
                last_frame.clear();
                last_frame.extend_from_slice(v.framebuffer());
            }
            match &mut display {
                Some(d) => {
                    incoming.extend(d.update(v).into_iter().map(|b| (b, true)));
                    if !d.is_open() {
                        break 'outer;
                    }
                }
                None => v.frame_ready = false,
            }
        }
        for (b, escapable) in incoming {
            match b {
                b if escape => {
                    escape = false;
                    match b {
                        b'q' | b'Q' => break 'outer,
                        b'r' | b'R' => {
                            eprint!("\r\n[az030] reset\r\n");
                            bus.reset();
                            cpu.reset(&mut bus);
                            was_halted = false;
                        }
                        ESCAPE => bus.uart_rx.push_back(ESCAPE),
                        _ => eprint!("\r\n[az030] Ctrl-] then: q quit, r reset\r\n"),
                    }
                }
                ESCAPE if escapable => escape = true,
                b => bus.uart_rx.push_back(b),
            }
        }

        if max_cycles.is_some_and(|m| total_cycles >= m) {
            break;
        }
        if cfg.machine.throttle {
            let target = Duration::from_secs_f64(total_cycles as f64 / clock_hz);
            let now = start.elapsed();
            if target > now {
                std::thread::sleep(target - now);
            }
        }
    }
    bus.flush_tx();
    drop(raw);
    if let Some(path) = &vera_snapshot {
        let fb = if last_frame.is_empty() { bus.vera.as_ref().unwrap().framebuffer() } else { &last_frame };
        vera::display::save_png(path, fb)?;
    }
    if let Some(f) = &profile {
        let mut v: Vec<(u32, u64)> = samples.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let text: String = v.iter().map(|(pc, n)| format!("{pc:08x} {n}\n")).collect();
        std::fs::write(f, text).map_err(|e| format!("{f}: {e}"))?;
    }
    for &(addr, len) in &dumps {
        for row in (0..len).step_by(16) {
            let a = addr.wrapping_add(row) as usize;
            let bytes: Vec<String> =
                (0..16).map(|i| bus.ram.get(a + i).map_or("--".into(), |b| format!("{b:02X}"))).collect();
            eprintln!("{:08X}: {}", a, bytes.join(" "));
        }
    }
    for (pc, sr, r) in &trace {
        eprintln!(
            "{pc:08X} SR={sr:04X} D={:08X} {:08X} {:08X} {:08X} {:08X} {:08X} {:08X} {:08X} A={:08X} {:08X} {:08X} {:08X} {:08X} {:08X} {:08X} {:08X}",
            r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8], r[9], r[10], r[11], r[12], r[13], r[14], r[15]
        );
    }
    if verbose {
        eprintln!("\n[az030] stopped after {total_cycles} cycles, PC={:08X}", cpu.pc);
    }
    Ok(())
}

fn human(n: u64) -> String {
    match n {
        n if n >= 1 << 30 && n % (1 << 30) == 0 => format!("{}G", n >> 30),
        n if n >= 1 << 20 && n % (1 << 20) == 0 => format!("{}M", n >> 20),
        n if n % (1 << 10) == 0 => format!("{}K", n >> 10),
        n => format!("{n} bytes"),
    }
}

/// Create a raw SCSI disk image, optionally with a program the boot ROM can load.
///
/// Boot block (LBA 0) layout, all big-endian:
///   0x00  "AZ30BOOT"
///   0x08  load address
///   0x0C  entry address
///   0x10  program length in 512-byte blocks (program starts at LBA 1)
///   0x14  program length in bytes
fn mkdisk(args: &[String]) -> Result<(), String> {
    let mut pos = Vec::new();
    let (mut boot, mut load, mut entry) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--boot" => boot = Some(PathBuf::from(it.next().ok_or("--boot needs a file")?)),
            "--load" => load = Some(parse_num(it.next().ok_or("--load needs an address")?)?),
            "--entry" => entry = Some(parse_num(it.next().ok_or("--entry needs an address")?)?),
            _ => pos.push(a.clone()),
        }
    }
    let [image, size] = pos.as_slice() else { return Err(USAGE.into()) };
    let size = parse_num(size)?;
    if size % scsi::BLOCK != 0 || size < 2 * scsi::BLOCK {
        return Err("disk size must be a multiple of 512 and at least 1K".into());
    }
    let mut disk = vec![0u8; size as usize];
    if let Some(boot) = boot {
        let prog = std::fs::read(&boot).map_err(|e| format!("{}: {e}", boot.display()))?;
        let load = load.ok_or("--boot requires --load")? as u32;
        let entry = entry.map_or(load, |e| e as u32);
        let blocks = prog.len().div_ceil(scsi::BLOCK as usize);
        if (1 + blocks) as u64 * scsi::BLOCK > size {
            return Err("program does not fit on the disk".into());
        }
        disk[0..8].copy_from_slice(b"AZ30BOOT");
        disk[8..12].copy_from_slice(&load.to_be_bytes());
        disk[12..16].copy_from_slice(&entry.to_be_bytes());
        disk[16..20].copy_from_slice(&(blocks as u32).to_be_bytes());
        disk[20..24].copy_from_slice(&(prog.len() as u32).to_be_bytes());
        disk[512..512 + prog.len()].copy_from_slice(&prog);
    }
    std::fs::write(image, &disk).map_err(|e| format!("{image}: {e}"))?;
    println!("wrote {image} ({})", human(size));
    Ok(())
}
