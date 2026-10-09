//! Host terminal: the UART console. Puts stdin in raw mode (when it is a tty) and feeds
//! keystrokes to the emulator from a background thread.

use std::io::Read;
use std::sync::mpsc::{self, Receiver};

pub struct RawMode {
    saved: Option<libc::termios>,
}

impl RawMode {
    /// Enable raw-ish input: no line buffering, no echo, no signals (Ctrl-C goes to the guest).
    /// Output processing is left on so the guest's "\r\n" and the host's messages behave.
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

/// Spawn a thread that forwards stdin bytes. The channel closes on EOF.
pub fn spawn_input() -> Receiver<u8> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 256];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if tx.send(b).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    rx
}
