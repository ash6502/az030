//! Host terminal: the UART console. Puts the console in raw mode (when it is a terminal) and
//! feeds keystrokes to the emulator from a background thread.
//!
//! Unix (macOS, Linux) uses termios; Windows uses the console API with VT input/output so
//! the guest sees the same bytes (Enter = CR, Ctrl-C = 0x03, arrows as ANSI escapes) and its
//! ANSI output renders.

use std::sync::mpsc::{self, Receiver, Sender};

pub use imp::RawMode;

/// Spawn a thread that forwards console input bytes. The channel closes on EOF.
pub fn spawn_input(raw: &RawMode) -> Receiver<u8> {
    let (tx, rx) = mpsc::channel();
    let console = raw.is_tty();
    std::thread::spawn(move || imp::read_input(console, tx));
    rx
}

fn forward(tx: &Sender<u8>, bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| tx.send(b).is_ok())
}

/// Read stdin with std until EOF (pipes, files, Unix terminals).
fn read_stdin(tx: Sender<u8>) {
    use std::io::Read;
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 256];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if !forward(&tx, &buf[..n]) {
                    return;
                }
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;

    pub struct RawMode {
        saved: Option<libc::termios>,
    }

    impl RawMode {
        /// Enable raw-ish input: no line buffering, no echo, no signals (Ctrl-C goes to the
        /// guest). Output processing is left on so the guest's "\r\n" and the host's
        /// messages behave.
        pub fn enable() -> Self {
            unsafe {
                if libc::isatty(0) == 0 {
                    return Self { saved: None };
                }
                let mut t: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut t) != 0 {
                    return Self { saved: None };
                }
                let saved = t;
                t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
                t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR | libc::ISTRIP);
                t.c_cc[libc::VMIN] = 1;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(0, libc::TCSANOW, &t);
                Self { saved: Some(saved) }
            }
        }

        pub fn is_tty(&self) -> bool {
            self.saved.is_some()
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            if let Some(t) = self.saved {
                unsafe {
                    libc::tcsetattr(0, libc::TCSANOW, &t);
                }
            }
        }
    }

    pub fn read_input(_console: bool, tx: Sender<u8>) {
        read_stdin(tx)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleCP, GetConsoleMode, GetConsoleOutputCP,
        GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleMode,
        SetConsoleOutputCP,
    };

    const CP_UTF8: u32 = 65001;

    struct Saved {
        input: HANDLE,
        in_mode: CONSOLE_MODE,
        output: Option<(HANDLE, CONSOLE_MODE)>,
        in_cp: u32,
        out_cp: u32,
    }

    pub struct RawMode {
        saved: Option<Saved>,
    }

    fn console_mode(h: HANDLE) -> Option<CONSOLE_MODE> {
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut mode = 0;
        (unsafe { GetConsoleMode(h, &mut mode) } != 0).then_some(mode)
    }

    impl RawMode {
        /// Turn off line input, echo and Ctrl-C processing (Ctrl-C goes to the guest), and
        /// switch the console to VT sequences and UTF-8 in both directions.
        pub fn enable() -> Self {
            unsafe {
                let input = GetStdHandle(STD_INPUT_HANDLE);
                let Some(in_mode) = console_mode(input) else {
                    return Self { saved: None };
                };
                let raw_in = (in_mode
                    & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                if SetConsoleMode(input, raw_in) == 0 {
                    return Self { saved: None };
                }
                // Output may be redirected while input is still the console.
                let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
                let output = console_mode(stdout).map(|mode| {
                    SetConsoleMode(
                        stdout,
                        mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                    );
                    (stdout, mode)
                });
                let (in_cp, out_cp) = (GetConsoleCP(), GetConsoleOutputCP());
                SetConsoleCP(CP_UTF8);
                SetConsoleOutputCP(CP_UTF8);
                Self { saved: Some(Saved { input, in_mode, output, in_cp, out_cp }) }
            }
        }

        pub fn is_tty(&self) -> bool {
            self.saved.is_some()
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            if let Some(s) = &self.saved {
                unsafe {
                    SetConsoleMode(s.input, s.in_mode);
                    if let Some((h, mode)) = s.output {
                        SetConsoleMode(h, mode);
                    }
                    SetConsoleCP(s.in_cp);
                    SetConsoleOutputCP(s.out_cp);
                }
            }
        }
    }

    pub fn read_input(console: bool, tx: Sender<u8>) {
        if !console {
            return read_stdin(tx);
        }
        // std's console reader treats a leading Ctrl-Z as end of file, so read raw bytes
        // with ReadFile instead; with VT input on, keys arrive as the bytes a Unix
        // terminal would send.
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut buf = [0u8; 256];
        loop {
            let mut n = 0u32;
            let ok = unsafe {
                ReadFile(input, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut())
            };
            if ok == 0 || !forward(&tx, &buf[..n as usize]) {
                return;
            }
        }
    }
}
