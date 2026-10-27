//! Opt-in diagnostic log (`Charpick --log`), written to `%APPDATA%\Charpick\debug.log`.
//!
//! It records modifier keys, the shortcut keys and the picker's own events. Other
//! keys are logged as `other`, so the log never contains what was typed.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Sender, channel};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
static TX: OnceLock<Sender<String>> = OnceLock::new();

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

pub fn path() -> PathBuf {
    crate::util::data_dir().join("debug.log")
}

/// Start logging. Lines are written by a thread of their own, so logging never
/// delays the keyboard hook.
pub fn start() -> bool {
    let Ok(mut file) = std::fs::File::create(path()) else { return false };
    let (tx, rx) = channel::<String>();
    let _ = START.set(Instant::now());
    let _ = TX.set(tx);
    let spawned = std::thread::Builder::new().name("diag".into()).spawn(move || {
        for line in rx {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    });
    ENABLED.store(spawned.is_ok(), Relaxed);
    spawned.is_ok()
}

pub fn write(msg: String) {
    if let Some(tx) = TX.get() {
        let ms = START.get().map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
        let _ = tx.send(format!("{ms:10.1}  {msg}"));
    }
}

/// `diag!("format", args..)`: a log line, formatted only while logging.
macro_rules! diag {
    ($($t:tt)*) => {
        if $crate::diag::enabled() {
            $crate::diag::write(format!($($t)*));
        }
    };
}
pub(crate) use diag;

/// A loggable name for a key: modifiers, navigation keys and the shortcut keys by
/// name, anything else as `other`.
pub fn key_name(vk: u32, shortcut_keys: &[u8]) -> String {
    let name = match vk {
        0x5B => "LWin",
        0x5C => "RWin",
        0x10 => "Shift",
        0xA0 => "LShift",
        0xA1 => "RShift",
        0x11 => "Ctrl",
        0xA2 => "LCtrl",
        0xA3 => "RCtrl",
        0x12 => "Alt",
        0xA4 => "LAlt",
        0xA5 => "RAlt",
        0x14 => "CapsLock",
        0x0D => "Enter",
        0x1B => "Esc",
        0x09 => "Tab",
        0x08 => "Backspace",
        0x2E => "Delete",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0xE7 => "Packet",
        0xE8 => "Mask(E8)",
        0xFF => "FF",
        0x07 => "07",
        v if v <= 0xFF && shortcut_keys.contains(&(v as u8)) => {
            return match char::from_u32(v) {
                Some(c) if c.is_ascii_alphanumeric() => format!("{c}"),
                _ => format!("vk{v:02X}"),
            };
        }
        _ => "other",
    };
    name.into()
}
