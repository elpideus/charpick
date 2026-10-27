//! User settings (`%APPDATA%\Charpick\settings.ini` on Windows,
//! `~/.config/charpick/settings.ini` on Linux) and the recently used list.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::util::data_dir;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StyleChoice {
    Auto,
    Win10,
    Win11,
    /// KDE Plasma (Breeze).
    Kde,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackdropChoice {
    Auto,
    Acrylic,
    Solid,
}

/// A key combination such as `Win+L` or `Ctrl+Alt+Space`. Keys are stored as
/// Windows virtual-key codes on every platform.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hotkey {
    pub vk: u8,
    pub mods: u8,
}

pub const MOD_CTRL: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_SHIFT: u8 = 4;
pub const MOD_WIN: u8 = 8;

/// What the Windows / Super key is called on this platform.
pub const WIN_NAME: &str = if cfg!(windows) { "Win" } else { "Meta" };

impl Hotkey {
    pub const NONE: Hotkey = Hotkey { vk: 0, mods: 0 };
    pub const WIN_L: Hotkey = Hotkey { vk: b'L', mods: MOD_WIN };
    /// Meta+, (Plasma already uses Meta+. for its emoji picker).
    pub const META_COMMA: Hotkey = Hotkey { vk: 0xBC, mods: MOD_WIN };

    /// The default shortcut on this platform.
    pub fn default_open() -> Hotkey {
        if cfg!(windows) { Hotkey::WIN_L } else { Hotkey::META_COMMA }
    }

    pub fn parse(s: &str) -> Option<Hotkey> {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("off") {
            return Some(Hotkey::NONE);
        }
        let mut hk = Hotkey::default();
        for part in s.split('+').map(str::trim) {
            let p = part.to_ascii_lowercase();
            match p.as_str() {
                "ctrl" | "control" => hk.mods |= MOD_CTRL,
                "alt" => hk.mods |= MOD_ALT,
                "shift" => hk.mods |= MOD_SHIFT,
                "win" | "windows" | "super" | "meta" | "logo" => hk.mods |= MOD_WIN,
                _ => {
                    if hk.vk != 0 {
                        return None;
                    }
                    hk.vk = key_from_name(&p)?;
                }
            }
        }
        hk.valid().then_some(hk)
    }

    /// A hotkey needs a key and at least one modifier other than Shift.
    pub fn valid(&self) -> bool {
        self.vk != 0 && self.mods & !MOD_SHIFT != 0
    }

    pub fn is_none(&self) -> bool {
        self.vk == 0
    }

    pub fn pack(&self) -> u32 {
        self.vk as u32 | (self.mods as u32) << 8
    }

    pub fn unpack(v: u32) -> Hotkey {
        Hotkey { vk: v as u8, mods: (v >> 8) as u8 }
    }

    pub fn label(&self) -> String {
        if self.is_none() {
            return "none".into();
        }
        self.parts().join("+")
    }

    /// The modifier names and the key name, in the order they are written. A
    /// combination still being typed may have no key yet.
    pub fn parts(&self) -> Vec<String> {
        // Windows writes the Windows key last, Plasma writes Meta first.
        let order = if cfg!(windows) {
            [(MOD_CTRL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, WIN_NAME)]
        } else {
            [(MOD_WIN, WIN_NAME), (MOD_CTRL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift")]
        };
        let mut parts: Vec<String> =
            order.iter().filter(|(bit, _)| self.mods & bit != 0).map(|(_, name)| name.to_string()).collect();
        if self.vk != 0 {
            parts.push(key_name(self.vk));
        }
        parts
    }
}

const NAMED_KEYS: &[(&str, u8)] = &[
    ("space", 0x20),
    ("enter", 0x0D),
    ("tab", 0x09),
    ("insert", 0x2D),
    ("ins", 0x2D),
    ("delete", 0x2E),
    ("del", 0x2E),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("pause", 0x13),
    ("apps", 0x5D),
    ("menu", 0x5D),
    (";", 0xBA),
    ("=", 0xBB),
    (",", 0xBC),
    ("-", 0xBD),
    (".", 0xBE),
    ("/", 0xBF),
    ("`", 0xC0),
    ("[", 0xDB),
    ("\\", 0xDC),
    ("]", 0xDD),
    ("'", 0xDE),
    // Spelled-out names; labels use the symbols above.
    ("comma", 0xBC),
    ("period", 0xBE),
    ("semicolon", 0xBA),
    ("minus", 0xBD),
    ("equal", 0xBB),
    ("slash", 0xBF),
    ("backslash", 0xDC),
    ("apostrophe", 0xDE),
    ("grave", 0xC0),
];

fn key_from_name(p: &str) -> Option<u8> {
    if let Some(&(_, vk)) = NAMED_KEYS.iter().find(|k| k.0 == p) {
        return Some(vk);
    }
    let b = p.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_alphanumeric()) {
        return Some(b[0].to_ascii_uppercase());
    }
    if let Some(n) = p.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x6F + n);
        }
    }
    if let Some(hex) = p.strip_prefix("vk") {
        return u8::from_str_radix(hex, 16).ok();
    }
    None
}

pub fn key_name(vk: u8) -> String {
    if let Some(&(name, _)) = NAMED_KEYS.iter().find(|k| k.1 == vk) {
        let mut c = name.chars();
        return match c.next() {
            Some(f) if f.is_alphabetic() => f.to_uppercase().chain(c).collect(),
            _ => name.to_string(),
        };
    }
    match vk {
        b'0'..=b'9' | b'A'..=b'Z' => (vk as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("vk{vk:02X}"),
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub style: StyleChoice,
    pub theme: ThemeChoice,
    pub backdrop: BackdropChoice,
    pub hotkey: Hotkey,
    pub lock_hotkey: Hotkey,
    pub order: Vec<String>,
    pub recent_count: usize,
    pub close_on_click: bool,
    pub caps_lock_uppercase: bool,
    pub gpu: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: StyleChoice::Auto,
            theme: ThemeChoice::Auto,
            backdrop: BackdropChoice::Auto,
            hotkey: Hotkey::default_open(),
            // Only used while Win+L is taken over (Windows).
            lock_hotkey: if cfg!(windows) { Hotkey { vk: b'L', mods: MOD_CTRL | MOD_WIN } } else { Hotkey::NONE },
            order: ["it", "ro", "de", "fr", "es", "pt", "*"].iter().map(|s| s.to_string()).collect(),
            recent_count: 16,
            close_on_click: false,
            caps_lock_uppercase: true,
            gpu: false,
        }
    }
}

#[cfg(windows)]
const TEMPLATE: &str = "\
; Charpick settings. Changes are picked up the next time the picker opens.
; Lines starting with ; are comments.

; Look of the picker: auto (match Windows), win10, win11
style = {style}

; Colors: auto (follow Windows), light, dark
theme = {theme}

; Background: auto (acrylic when transparency effects are on), acrylic, solid
backdrop = {backdrop}

; Shortcut that opens the picker. Examples: Win+L, Ctrl+Alt+Space, Win+Shift+K.
; Win+L replaces the Windows lock shortcut and needs the one-time setup from the installer.
hotkey = {hotkey}

; Shortcut that locks the PC while Win+L is taken over (none to disable).
lock_hotkey = {lock_hotkey}

; Order of the sections. Use language or section codes, * means \"everything else\".
; Languages: it ro de fr es pt ca nl sv no da fi is pl cs sk hu hr sl tr et lv lt sq mt cy ga eo vi el
; Symbols:   punct currency math numbers misc
order = {order}

; How many recently used characters to remember (0 hides the list).
recent_count = {recent_count}

; Keep the picker open after clicking a character (Enter always closes it).
close_on_click = {close_on_click}

; Caps Lock also switches letters to uppercase (Shift always does).
caps_lock_uppercase = {caps_lock_uppercase}

; Render with the GPU instead of the CPU. The CPU is lighter for a window this small.
gpu = {gpu}
";

#[cfg(not(windows))]
const TEMPLATE: &str = "\
; Charpick settings. Changes apply within a few seconds.
; Lines starting with ; are comments.

; Look of the picker: auto (KDE Plasma / Breeze), kde, win11, win10
style = {style}

; Colors: auto (follow the desktop color scheme), light, dark
theme = {theme}

; Background: auto (blurred when the compositor supports it), blur, solid
backdrop = {backdrop}

; Shortcut that opens the picker. Examples: Meta+Comma, Ctrl+Alt+Space, Meta+Shift+K.
; none means no shortcut: open the picker from the tray icon.
; On KDE Plasma it is registered as a global shortcut, so it can also be changed in
; System Settings > Keyboard > Shortcuts > Charpick, and changes made there show up here.
hotkey = {hotkey}

; Order of the sections. Use language or section codes, * means \"everything else\".
; Languages: it ro de fr es pt ca nl sv no da fi is pl cs sk hu hr sl tr et lv lt sq mt cy ga eo vi el
; Symbols:   punct currency math numbers misc
order = {order}

; How many recently used characters to remember (0 hides the list).
recent_count = {recent_count}

; Caps Lock also switches letters to uppercase (Shift always does).
caps_lock_uppercase = {caps_lock_uppercase}
";

const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

fn on_off(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        data_dir().join("settings.ini")
    }

    pub fn modified() -> Option<SystemTime> {
        std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok()
    }

    /// Load the settings file, writing a commented default file when missing.
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            s.save();
            return s;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            let lv = v.to_ascii_lowercase();
            match k.as_str() {
                "style" => {
                    s.style = match lv.as_str() {
                        "win10" | "10" => StyleChoice::Win10,
                        "win11" | "11" => StyleChoice::Win11,
                        "kde" | "breeze" | "plasma" => StyleChoice::Kde,
                        _ => StyleChoice::Auto,
                    }
                }
                "theme" => {
                    s.theme = match lv.as_str() {
                        "light" => ThemeChoice::Light,
                        "dark" => ThemeChoice::Dark,
                        _ => ThemeChoice::Auto,
                    }
                }
                "backdrop" => {
                    s.backdrop = match lv.as_str() {
                        "acrylic" | "blur" => BackdropChoice::Acrylic,
                        "solid" | "none" => BackdropChoice::Solid,
                        _ => BackdropChoice::Auto,
                    }
                }
                "hotkey" => {
                    // On Linux "none" leaves the picker to the tray icon and `Charpick --toggle`.
                    if let Some(h) = Hotkey::parse(v).filter(|h| !h.is_none() || cfg!(not(windows))) {
                        s.hotkey = h;
                    }
                }
                "lock_hotkey" => {
                    if let Some(h) = Hotkey::parse(v) {
                        s.lock_hotkey = h;
                    }
                }
                "order" => {
                    let list: Vec<String> =
                        v.split([',', ' ']).map(str::trim).filter(|x| !x.is_empty()).map(|x| x.to_ascii_lowercase()).collect();
                    if !list.is_empty() {
                        s.order = list;
                    }
                }
                "recent_count" => {
                    if let Ok(n) = v.parse::<usize>() {
                        s.recent_count = n.min(64);
                    }
                }
                "close_on_click" => s.close_on_click = parse_bool(v).unwrap_or(s.close_on_click),
                "caps_lock_uppercase" => s.caps_lock_uppercase = parse_bool(v).unwrap_or(s.caps_lock_uppercase),
                "gpu" => s.gpu = parse_bool(v).unwrap_or(s.gpu),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let text = TEMPLATE
            .replace(
                "{style}",
                match self.style {
                    StyleChoice::Auto => "auto",
                    StyleChoice::Win10 => "win10",
                    StyleChoice::Win11 => "win11",
                    StyleChoice::Kde => "kde",
                },
            )
            .replace(
                "{theme}",
                match self.theme {
                    ThemeChoice::Auto => "auto",
                    ThemeChoice::Light => "light",
                    ThemeChoice::Dark => "dark",
                },
            )
            .replace(
                "{backdrop}",
                match self.backdrop {
                    BackdropChoice::Auto => "auto",
                    BackdropChoice::Acrylic => if cfg!(windows) { "acrylic" } else { "blur" },
                    BackdropChoice::Solid => "solid",
                },
            )
            .replace("{hotkey}", &self.hotkey.label())
            .replace("{lock_hotkey}", &self.lock_hotkey.label())
            .replace("{order}", &self.order.join(", "))
            .replace("{recent_count}", &self.recent_count.to_string())
            .replace("{close_on_click}", on_off(self.close_on_click))
            .replace("{caps_lock_uppercase}", on_off(self.caps_lock_uppercase))
            .replace("{gpu}", on_off(self.gpu))
            .replace('\n', NEWLINE);
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(Self::path(), text);
    }
}

/// Recently used characters, most recent first, stored one per line.
pub struct Recent {
    pub items: Vec<String>,
    dirty: bool,
}

impl Recent {
    fn path() -> PathBuf {
        data_dir().join("recent.txt")
    }

    pub fn load() -> Recent {
        let items = std::fs::read_to_string(Self::path())
            .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        Recent { items, dirty: false }
    }

    pub fn push(&mut self, key: &str, max: usize) {
        self.items.retain(|i| i != key);
        self.items.insert(0, key.to_string());
        self.items.truncate(max.max(1));
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.dirty = true;
        self.flush();
    }

    pub fn flush(&mut self) {
        if self.dirty {
            self.dirty = false;
            let _ = std::fs::write(Self::path(), self.items.join(NEWLINE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        assert_eq!(Hotkey::parse("Win+L"), Some(Hotkey::WIN_L));
        assert_eq!(Hotkey::parse("ctrl + win + l").unwrap().mods, MOD_CTRL | MOD_WIN);
        assert_eq!(Hotkey::parse("Ctrl+Alt+Space").unwrap().vk, 0x20);
        assert_eq!(Hotkey::parse("F9"), None);
        assert_eq!(Hotkey::parse("Win+F9").unwrap().vk, 0x78);
        assert_eq!(Hotkey::parse("none"), Some(Hotkey::NONE));
        let ctrl_win_l = if cfg!(windows) { "Ctrl+Win+L" } else { "Meta+Ctrl+L" };
        assert_eq!(Hotkey::parse("Ctrl+Win+L").unwrap().label(), ctrl_win_l);
        assert_eq!(Hotkey::parse("Ctrl+Alt+Space").unwrap().label(), "Ctrl+Alt+Space");
        assert_eq!(Hotkey::parse("Meta+,"), Some(Hotkey::META_COMMA));
        assert_eq!(Hotkey::parse("super+comma"), Some(Hotkey::META_COMMA));
        assert_eq!(Hotkey::parse(&Hotkey::META_COMMA.label()), Some(Hotkey::META_COMMA));
        assert_eq!(Hotkey { vk: 0, mods: MOD_CTRL | MOD_ALT }.parts(), ["Ctrl", "Alt"]);
    }
}
