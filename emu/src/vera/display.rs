//! Host front end for the VERA: a window showing the 640x480 output, keyboard input
//! from that window, and PNG snapshots.
//!
//! The az030's PS/2 keyboard has no registers in the memory map yet, so keys typed into
//! the window go to the UART, as the bytes a VT100-style terminal would send. That way
//! the window works as a console for software that draws its own text with the VERA.

use super::{HEIGHT, Vera, WIDTH};
use minifb::{InputCallback, Key, Scale, ScaleMode, Window, WindowOptions};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Don't present frames faster than the host display can show them.
const MIN_FRAME_TIME: Duration = Duration::from_millis(15);
/// How often to service window events when no frame is due.
const EVENT_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Default)]
struct KeyState {
    bytes: Vec<u8>,
    ctrl_l: bool,
    ctrl_r: bool,
}

/// Receives keyboard events from minifb in the order they happen.
struct Keys(Rc<RefCell<KeyState>>);

impl InputCallback for Keys {
    /// Printable text (minifb never reports control characters here).
    fn add_char(&mut self, c: u32) {
        let mut k = self.0.borrow_mut();
        if k.ctrl_l || k.ctrl_r {
            return; // Ctrl combinations are produced in set_key_state
        }
        if let Some(ch) = char::from_u32(c).filter(|ch| !ch.is_control()) {
            let mut buf = [0; 4];
            k.bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }

    fn set_key_state(&mut self, key: Key, down: bool) {
        let mut k = self.0.borrow_mut();
        match key {
            Key::LeftCtrl => k.ctrl_l = down,
            Key::RightCtrl => k.ctrl_r = down,
            _ => {}
        }
        if !down {
            return;
        }
        let ctrl = k.ctrl_l || k.ctrl_r;
        let seq: &[u8] = match key {
            Key::Enter | Key::NumPadEnter => b"\r",
            Key::Backspace => b"\x7f",
            Key::Tab => b"\t",
            Key::Escape => b"\x1b",
            Key::Up => b"\x1b[A",
            Key::Down => b"\x1b[B",
            Key::Right => b"\x1b[C",
            Key::Left => b"\x1b[D",
            Key::Home => b"\x1b[H",
            Key::End => b"\x1b[F",
            Key::Insert => b"\x1b[2~",
            Key::Delete => b"\x1b[3~",
            Key::PageUp => b"\x1b[5~",
            Key::PageDown => b"\x1b[6~",
            _ if ctrl => {
                let code = match key {
                    k if (Key::A..=Key::Z).contains(&k) => (k as u8 - Key::A as u8) + 1,
                    Key::Space | Key::Key2 => 0x00,
                    Key::LeftBracket => 0x1B,
                    Key::Backslash => 0x1C,
                    Key::RightBracket => 0x1D,
                    Key::Key6 => 0x1E,
                    Key::Minus => 0x1F,
                    _ => return,
                };
                k.bytes.push(code);
                return;
            }
            _ => return,
        };
        k.bytes.extend_from_slice(seq);
    }
}

pub struct Display {
    window: Window,
    keys: Rc<RefCell<KeyState>>,
    last_frame: Instant,
    last_events: Instant,
}

impl Display {
    /// Open the window at `scale` times 640x480 (1, 2 or 4). It can be resized freely.
    pub fn open(scale: u32) -> Result<Self, String> {
        let scale = match scale {
            1 => Scale::X1,
            2 => Scale::X2,
            4 => Scale::X4,
            _ => return Err("--vera-scale must be 1, 2 or 4".into()),
        };
        let opts = WindowOptions {
            resize: true,
            scale,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        };
        let mut window = Window::new("az030 VERA", WIDTH, HEIGHT, opts)
            .map_err(|e| format!("cannot open the VERA window: {e}"))?;
        window.set_target_fps(0); // the emulator paces itself
        let keys = Rc::new(RefCell::new(KeyState::default()));
        window.set_input_callback(Box::new(Keys(keys.clone())));
        let now = Instant::now();
        Ok(Self { window, keys, last_frame: now, last_events: now })
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// Show the frame VERA just finished, if there is one and it is time for it, and
    /// service window events. Returns the bytes typed into the window.
    pub fn update(&mut self, vera: &mut Vera) -> Vec<u8> {
        let now = Instant::now();
        if vera.frame_ready {
            vera.frame_ready = false;
            if now - self.last_frame >= MIN_FRAME_TIME {
                // Presenting also processes events.
                let _ = self.window.update_with_buffer(vera.framebuffer(), WIDTH, HEIGHT);
                self.last_frame = now;
                self.last_events = now;
            }
        }
        if now - self.last_events >= EVENT_INTERVAL {
            self.window.update();
            self.last_events = now;
        }
        std::mem::take(&mut self.keys.borrow_mut().bytes)
    }
}

/// Save a framebuffer as an RGB PNG.
pub fn save_png(path: &str, fb: &[u32]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH as u32, HEIGHT as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = fb.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect();
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&data))
        .map_err(|e| format!("{path}: {e}"))
}
