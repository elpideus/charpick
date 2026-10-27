//! The session bus: Plasma's global shortcut service (KGlobalAccel) and desktop
//! notifications.
//!
//! The shortcut lives in two places, `settings.ini` and KDE's own list (System
//! Settings > Keyboard > Shortcuts). A change in either one is copied to the other.

use std::collections::HashMap;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;

use super::Cmd;
use crate::settings::{Hotkey, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};

const SERVICE: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";
const COMPONENT: &str = "charpick";
const ACTION: &str = "toggle";

// KGlobalAccel::SetShortcutFlag
const SET_PRESENT: u32 = 2;
const NO_AUTOLOADING: u32 = 4;
const IS_DEFAULT: u32 = 8;

/// A list of key sequences as KGlobalAccel sends it: each a struct of four Qt key codes.
type Sequences = Vec<(Vec<i32>,)>;

const QT_SHIFT: i32 = 0x0200_0000;
const QT_CTRL: i32 = 0x0400_0000;
const QT_ALT: i32 = 0x0800_0000;
const QT_META: i32 = 0x1000_0000;
const QT_KEYPAD: i32 = 0x2000_0000;
const QT_MODS: i32 = QT_SHIFT | QT_CTRL | QT_ALT | QT_META | QT_KEYPAD;

/// Virtual keys and their Qt key codes, besides letters, digits and F keys.
const QT_KEYS: &[(u8, i32)] = &[
    (0x08, 0x0100_0003), // Backspace
    (0x09, 0x0100_0001), // Tab
    (0x0D, 0x0100_0004), // Return
    (0x13, 0x0100_0008), // Pause
    (0x1B, 0x0100_0000), // Escape
    (0x20, 0x20),        // Space
    (0x21, 0x0100_0016), // Page Up
    (0x22, 0x0100_0017), // Page Down
    (0x23, 0x0100_0011), // End
    (0x24, 0x0100_0010), // Home
    (0x25, 0x0100_0012), // Left
    (0x26, 0x0100_0013), // Up
    (0x27, 0x0100_0014), // Right
    (0x28, 0x0100_0015), // Down
    (0x2D, 0x0100_0006), // Insert
    (0x2E, 0x0100_0007), // Delete
    (0x5D, 0x0100_0055), // Menu
    (0xBA, b';' as i32),
    (0xBB, b'=' as i32),
    (0xBC, b',' as i32),
    (0xBD, b'-' as i32),
    (0xBE, b'.' as i32),
    (0xBF, b'/' as i32),
    (0xC0, b'`' as i32),
    (0xDB, b'[' as i32),
    (0xDC, b'\\' as i32),
    (0xDD, b']' as i32),
    (0xDE, b'\'' as i32),
];

/// The Qt key code of a shortcut.
pub fn to_qt(hk: Hotkey) -> Option<i32> {
    let key = match hk.vk {
        b'A'..=b'Z' | b'0'..=b'9' => hk.vk as i32,
        0x70..=0x87 => 0x0100_0030 + (hk.vk - 0x70) as i32,
        vk => QT_KEYS.iter().find(|k| k.0 == vk)?.1,
    };
    let mut code = key;
    for (bit, qt) in [(MOD_SHIFT, QT_SHIFT), (MOD_CTRL, QT_CTRL), (MOD_ALT, QT_ALT), (MOD_WIN, QT_META)] {
        if hk.mods & bit != 0 {
            code |= qt;
        }
    }
    Some(code)
}

/// The shortcut for a Qt key code, when Charpick can name it.
pub fn from_qt(code: i32) -> Option<Hotkey> {
    let key = code & !QT_MODS;
    let vk = match key {
        0x41..=0x5A | 0x30..=0x39 => key as u8,
        0x61..=0x7A => (key - 0x20) as u8,
        0x0100_0030..=0x0100_0047 => (0x70 + key - 0x0100_0030) as u8,
        _ => QT_KEYS.iter().find(|k| k.1 == key)?.0,
    };
    let mut mods = 0;
    for (bit, qt) in [(MOD_SHIFT, QT_SHIFT), (MOD_CTRL, QT_CTRL), (MOD_ALT, QT_ALT), (MOD_WIN, QT_META)] {
        if code & qt != 0 {
            mods |= bit;
        }
    }
    Some(Hotkey { vk, mods })
}

/// A readable name for any Qt key code, such as `Meta+Ò`.
pub fn qt_label(code: i32) -> String {
    if let Some(hk) = from_qt(code) {
        return hk.label();
    }
    let mut parts = Hotkey { vk: 0, mods: from_qt((code & QT_MODS) | b'A' as i32).map_or(0, |h| h.mods) }.parts();
    let key = (code & !QT_MODS) as u32;
    parts.push(match char::from_u32(key) {
        Some(c) if key < 0x0100_0000 && !c.is_control() => c.to_uppercase().collect(),
        _ => format!("key {key:#x}"),
    });
    parts.join("+")
}

fn action_id() -> Vec<&'static str> {
    vec![COMPONENT, ACTION, "Charpick", "Open the character picker"]
}

fn sequences(code: Option<i32>) -> Sequences {
    code.map(|c| vec![(vec![c, 0, 0, 0],)]).unwrap_or_default()
}

/// The first key of the first sequence, the only part Charpick uses.
fn first_key(seqs: &Sequences) -> Option<i32> {
    seqs.iter().filter_map(|s| s.0.first().copied()).find(|&k| k != 0)
}

/// What KDE ended up using.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdeShortcut(pub Option<i32>);

impl KdeShortcut {
    pub fn hotkey(self) -> Option<Hotkey> {
        match self.0 {
            None => Some(Hotkey::NONE),
            Some(k) => from_qt(k),
        }
    }

    pub fn label(self) -> String {
        self.0.map(qt_label).unwrap_or_else(|| "none".into())
    }
}

pub struct Bus {
    conn: Connection,
}

impl Bus {
    pub fn connect() -> Option<Bus> {
        match Connection::session() {
            Ok(conn) => Some(Bus { conn }),
            Err(e) => {
                log!("dbus  no session bus: {e}");
                None
            }
        }
    }

    fn kga<B>(&self, method: &str, body: &B) -> zbus::Result<zbus::message::Message>
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn.call_method(Some(SERVICE), PATH, Some(IFACE), method, body)
    }

    /// Announce the action and its default shortcut.
    pub fn register(&self) -> zbus::Result<()> {
        self.kga("doRegister", &(action_id(),))?;
        self.kga("setShortcutKeys", &(action_id(), sequences(to_qt(Hotkey::default_open())), IS_DEFAULT))?;
        Ok(())
    }

    /// Set the shortcut. Without `force`, a shortcut KDE already knows for the
    /// action wins (it may have been changed in System Settings meanwhile).
    /// Keys used by another shortcut are left out by KDE.
    pub fn set_shortcut(&self, hk: Hotkey, force: bool) -> zbus::Result<KdeShortcut> {
        let code = if hk.is_none() { None } else { to_qt(hk) };
        let flags = SET_PRESENT | if force { NO_AUTOLOADING } else { 0 };
        let reply = self.kga("setShortcutKeys", &(action_id(), sequences(code), flags))?;
        let seqs: Sequences = reply.body().deserialize()?;
        Ok(KdeShortcut(first_key(&seqs)))
    }

    /// Whether `hk` is free, or already Charpick's.
    pub fn available(&self, hk: Hotkey) -> bool {
        let Some(code) = to_qt(hk) else { return false };
        self.kga("globalShortcutAvailable", &((vec![code, 0, 0, 0],), COMPONENT))
            .and_then(|r| r.body().deserialize::<bool>())
            .unwrap_or(true)
    }

    /// Hold back every global shortcut, so the recorder receives them.
    pub fn block_shortcuts(&self, block: bool) {
        if let Err(e) = self.kga("blockGlobalShortcuts", &(block,)) {
            log!("dbus  blockGlobalShortcuts({block}) failed: {e}");
        }
    }

    /// Charpick is going away: its shortcut does nothing until the next start.
    pub fn set_inactive(&self) {
        let _ = self.kga("setInactive", &(action_id(),));
    }

    /// Show a desktop notification.
    pub fn notify(&self, summary: &str, body: &str) {
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert("desktop-entry", Value::from("charpick"));
        let r = self.conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("Charpick", 0u32, "accessories-character-map", summary, body, Vec::<&str>::new(), hints, -1i32),
        );
        if let Err(e) = r {
            log!("dbus  notification failed: {e}");
        }
    }

    /// Forward shortcut presses and changes made in System Settings.
    pub fn listen(&self, tx: calloop::channel::Sender<Cmd>) {
        let conn = self.conn.clone();
        let rules = [
            format!(
                "type='signal',path='/component/{COMPONENT}',interface='org.kde.kglobalaccel.Component',member='globalShortcutPressed'"
            ),
            format!("type='signal',path='{PATH}',interface='{IFACE}',member='yourShortcutsChanged'"),
            format!(
                "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='{SERVICE}'"
            ),
        ];
        for rule in rules {
            let conn = conn.clone();
            let tx = tx.clone();
            let spawned = std::thread::Builder::new().name("dbus-signals".into()).spawn(move || {
                let Ok(rule) = zbus::MatchRule::try_from(rule.as_str()) else { return };
                let iter = match MessageIterator::for_match_rule(rule, &conn, Some(16)) {
                    Ok(i) => i,
                    Err(e) => {
                        log!("dbus  cannot listen: {e}");
                        return;
                    }
                };
                for msg in iter.flatten() {
                    let header = msg.header();
                    let member = header.member().map(|m| m.to_string()).unwrap_or_default();
                    let body = msg.body();
                    let cmd = match member.as_str() {
                        "globalShortcutPressed" => match body.deserialize::<(String, String, i64)>() {
                            Ok((c, a, _)) if c == COMPONENT && a == ACTION => Some(Cmd::Toggle),
                            _ => None,
                        },
                        "yourShortcutsChanged" => match body.deserialize::<(Vec<String>, Sequences)>() {
                            Ok((id, seqs))
                                if id.first().map(String::as_str) == Some(COMPONENT)
                                    && id.get(1).map(String::as_str) == Some(ACTION) =>
                            {
                                Some(Cmd::KdeShortcut(KdeShortcut(first_key(&seqs))))
                            }
                            _ => None,
                        },
                        "NameOwnerChanged" => match body.deserialize::<(String, String, String)>() {
                            // The shortcut service (re)started: register again.
                            Ok((_, _, new)) if !new.is_empty() => Some(Cmd::Reregister),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(cmd) = cmd
                        && tx.send(cmd).is_err()
                    {
                        return;
                    }
                }
            });
            if let Err(e) = spawned {
                log!("dbus  cannot start a listener: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_codes() {
        // Meta+, as Plasma stores it.
        assert_eq!(to_qt(Hotkey::META_COMMA), Some(0x1000_002C));
        assert_eq!(from_qt(0x1000_002C), Some(Hotkey::META_COMMA));
        // Ctrl+Alt+Space, Meta+Shift+K, Alt+F2 and Ctrl+Alt+Delete round-trip.
        for s in ["Ctrl+Alt+Space", "Meta+Shift+K", "Alt+F2", "Ctrl+Alt+Delete", "Meta+]", "Ctrl+Alt+7"] {
            let hk = Hotkey::parse(s).unwrap();
            assert_eq!(from_qt(to_qt(hk).unwrap()), Some(hk), "{s}");
        }
        // Meta+ò cannot be written in settings.ini but still gets a name.
        assert_eq!(from_qt(QT_META | 0xF2), None);
        assert_eq!(qt_label(QT_META | 0xF2), format!("{}+Ò", crate::settings::WIN_NAME));
        assert_eq!(KdeShortcut(None).hotkey(), Some(Hotkey::NONE));
    }
}
