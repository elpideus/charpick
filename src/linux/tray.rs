//! The icon in the system tray (StatusNotifierItem) and its menu. The menu runs
//! on its own thread and sends what was chosen to the main loop.

use calloop::channel::Sender;
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Category, MenuItem, ToolTip};

use super::Cmd;
use crate::settings::{Hotkey, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN, StyleChoice, ThemeChoice, key_name};

const STYLES: [(StyleChoice, &str); 4] = [
    (StyleChoice::Auto, "Automatic (KDE Plasma)"),
    (StyleChoice::Kde, "KDE Plasma"),
    (StyleChoice::Win11, "Windows 11"),
    (StyleChoice::Win10, "Windows 10"),
];

const THEMES: [(ThemeChoice, &str); 3] =
    [(ThemeChoice::Auto, "Follow the color scheme"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")];

/// What the menu shows. The main loop keeps it up to date.
pub struct Tray {
    pub tx: Sender<Cmd>,
    /// The shortcut, when Charpick can name it in settings.ini.
    pub hotkey: Option<Hotkey>,
    /// The shortcut as text ("none" when there is none).
    pub hotkey_label: String,
    pub style: StyleChoice,
    pub theme: ThemeChoice,
    pub autostart: bool,
    pub has_recent: bool,
}

impl Tray {
    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

/// The shortcut the way menus show it: `[["Super", ","]]`.
fn menu_shortcut(hk: Option<Hotkey>) -> Vec<Vec<String>> {
    let Some(hk) = hk.filter(|h| !h.is_none()) else { return Vec::new() };
    let mut keys = Vec::new();
    for (bit, name) in [(MOD_CTRL, "Control"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Super")] {
        if hk.mods & bit != 0 {
            keys.push(name.to_string());
        }
    }
    keys.push(key_name(hk.vk));
    vec![keys]
}

fn item(label: &str, cmd: fn() -> Cmd) -> MenuItem<Tray> {
    StandardItem { label: label.into(), activate: Box::new(move |t: &mut Tray| t.send(cmd())), ..Default::default() }.into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "charpick".into()
    }

    fn title(&self) -> String {
        "Charpick".into()
    }

    fn icon_name(&self) -> String {
        "accessories-character-map".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn tool_tip(&self) -> ToolTip {
        let description = if self.hotkey_label == "none" {
            "Accented letters and symbols".into()
        } else {
            format!("Press {} to pick a character", self.hotkey_label)
        };
        ToolTip { title: "Charpick".into(), description, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Cmd::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let style = STYLES.iter().position(|s| s.0 == self.style).unwrap_or(0);
        let theme = THEMES.iter().position(|t| t.0 == self.theme).unwrap_or(0);
        let mut menu = vec![
            StandardItem {
                label: "Open Charpick".into(),
                shortcut: menu_shortcut(self.hotkey),
                activate: Box::new(|t: &mut Tray| t.send(Cmd::Show)),
                ..Default::default()
            }
            .into(),
        ];
        if self.hotkey.is_none_or(|h| h.is_none()) {
            // No shortcut, or one set in System Settings that the menu cannot show.
            let label = format!("Shortcut: {}", self.hotkey_label);
            menu.push(StandardItem { label, enabled: false, ..Default::default() }.into());
        }
        menu.extend([
            item("Change shortcut…", || Cmd::ChangeShortcut),
            MenuItem::Separator,
            SubMenu {
                label: "Style".into(),
                submenu: vec![
                    RadioGroup {
                        selected: style,
                        select: Box::new(|t: &mut Tray, i| {
                            t.style = STYLES[i].0;
                            t.send(Cmd::Style(STYLES[i].0));
                        }),
                        options: STYLES.iter().map(|s| RadioItem { label: s.1.into(), ..Default::default() }).collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Colors".into(),
                submenu: vec![
                    RadioGroup {
                        selected: theme,
                        select: Box::new(|t: &mut Tray, i| {
                            t.theme = THEMES[i].0;
                            t.send(Cmd::Theme(THEMES[i].0));
                        }),
                        options: THEMES.iter().map(|s| RadioItem { label: s.1.into(), ..Default::default() }).collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Start at login".into(),
                checked: self.autostart,
                activate: Box::new(|t: &mut Tray| {
                    t.autostart = !t.autostart;
                    t.send(Cmd::Autostart(t.autostart));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Clear recently used".into(),
                enabled: self.has_recent,
                activate: Box::new(|t: &mut Tray| {
                    t.has_recent = false;
                    t.send(Cmd::ClearRecent);
                }),
                ..Default::default()
            }
            .into(),
            item("Edit settings…", || Cmd::EditSettings),
            item("How to use", || Cmd::Help),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Tray| t.send(Cmd::Quit)),
                ..Default::default()
            }
            .into(),
        ]);
        menu
    }
}
