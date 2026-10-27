//! Linux front end, made for KDE Plasma on Wayland: the picker is a layer-shell
//! panel, the shortcut is a Plasma global shortcut (KGlobalAccel), characters are
//! typed through KWin's fake input protocol, and the tray icon is a
//! StatusNotifierItem.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static LOG: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

pub fn logging() -> bool {
    LOG.load(Ordering::Relaxed)
}

pub fn log_line(args: std::fmt::Arguments) {
    eprintln!("{:9.3} {args}", START.get_or_init(Instant::now).elapsed().as_secs_f64());
}

/// Print to stderr when started with `--log`.
macro_rules! log {
    ($($arg:tt)*) => {
        if $crate::linux::logging() {
            $crate::linux::log_line(format_args!($($arg)*));
        }
    };
}

mod dbus;
mod ipc;
pub mod kde;
pub mod paint;
mod protocols;
mod tray;
mod wl;

use calloop::channel::Event;
use calloop::timer::{TimeoutAction, Timer};

use crate::settings::{Hotkey, Settings, StyleChoice, ThemeChoice};
use dbus::KdeShortcut;
use tray::Tray;
use wl::State;

/// How often settings.ini and the Plasma color scheme are checked for changes.
const WATCH_EVERY: Duration = Duration::from_secs(2);
/// How long "Shortcut saved" stays up.
const SAVED_FOR: Duration = Duration::from_millis(1400);

/// Work for the main loop, from the tray menu, D-Bus and later starts of Charpick.
#[derive(Debug)]
pub enum Cmd {
    Toggle,
    Show,
    Hello,
    Quit,
    ChangeShortcut,
    Style(StyleChoice),
    Theme(ThemeChoice),
    Autostart(bool),
    ClearRecent,
    EditSettings,
    Help,
    /// The shortcut was changed in System Settings.
    KdeShortcut(KdeShortcut),
    /// Plasma's shortcut service restarted.
    Reregister,
}

const USAGE: &str = "\
Usage: charpick [option]

  (none)              start Charpick (or greet, when it is already running)
  --toggle            open or close the picker
  --show              open the picker
  --change-shortcut   choose a new shortcut
  --quit              stop the running Charpick
  --autostart         start quietly (used at login)
  --style auto|kde|win11|win10
  --theme auto|light|dark
  --backdrop auto|blur|solid
  --log               print what happens to stderr";

const HELP: &str = "\
Press the shortcut (Meta+, unless you changed it) while typing in any app.

•  Type to search: a letter (\"s\"), a name (\"e grave\", \"cedilla\"), a symbol (\"euro\") or a language (\"romanian\", \"română\", \"ro\").
•  Arrow keys, Page Up/Down and Home/End move around; Tab and Shift+Tab jump between languages.
•  Enter or a click types the character and closes the picker.
•  Hold Shift (or turn on Caps Lock) for uppercase letters.
•  Esc clears the search, then closes.

To use another shortcut, choose \"Change shortcut…\" in the tray menu and press the new one. It can also be changed in System Settings > Keyboard > Shortcuts > Charpick.";

/// The shortcut that settings.ini and Plasma last agreed on.
fn sync_path() -> PathBuf {
    crate::util::data_dir().join("kde-shortcut")
}

fn autostart_path() -> PathBuf {
    kde::config_home().join("autostart").join("charpick.desktop")
}

/// Quote a path for the `Exec` key of a desktop file. The quoting backslashes
/// are themselves escaped, because the value is also a desktop-file string.
fn exec_quote(path: &str) -> String {
    let mut out = String::from('"');
    for c in path.chars() {
        match c {
            '\\' => out.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                out.push_str(r"\\");
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn set_autostart(on: bool) -> std::io::Result<()> {
    let path = autostart_path();
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = crate::util::exe_path();
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Charpick\nComment=Accented letters and symbols\n\
         Exec={} --autostart\nIcon=accessories-character-map\nTerminal=false\n\
         X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input\n",
        exec_quote(&exe.to_string_lossy())
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// Start a program without waiting for it.
fn spawn(program: &str, args: &[&str]) -> bool {
    match std::process::Command::new(program).args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(e) => {
            log!("app   cannot start {program}: {e}");
            false
        }
    }
}

impl State {
    pub(super) fn command(&mut self, cmd: Cmd) {
        log!("cmd   {cmd:?}");
        match cmd {
            Cmd::Toggle => self.toggle(),
            Cmd::Show => self.show(false),
            Cmd::Hello => self.greet(),
            Cmd::Quit => self.quit(),
            Cmd::ChangeShortcut => self.record_shortcut(),
            Cmd::Style(s) => {
                self.opts.style = None;
                self.settings.style = s;
                self.save_settings();
            }
            Cmd::Theme(t) => {
                self.opts.theme = None;
                self.settings.theme = t;
                self.save_settings();
            }
            Cmd::Autostart(on) => {
                if let Err(e) = set_autostart(on) {
                    self.notify("Could not change starting at login", &e.to_string());
                }
                self.update_tray();
            }
            Cmd::ClearRecent => {
                self.recent.clear();
                self.update_tray();
            }
            Cmd::EditSettings => {
                if !Settings::path().exists() {
                    self.settings.save();
                }
                let path = Settings::path();
                if !spawn("xdg-open", &[&path.to_string_lossy()]) {
                    self.notify("Charpick settings", &path.to_string_lossy());
                }
            }
            Cmd::Help => {
                if !spawn("kdialog", &["--title", "How to use Charpick", "--msgbox", HELP]) {
                    self.notify("How to use Charpick", HELP);
                }
            }
            Cmd::KdeShortcut(k) => self.adopt(k),
            Cmd::Reregister => self.register_shortcut(),
        }
    }

    fn save_settings(&mut self) {
        self.settings.save();
        self.settings_time = Settings::modified();
        self.apply_settings();
        self.update_tray();
    }

    fn quit(&mut self) {
        self.hide();
        if let Some(b) = &self.bus {
            b.set_inactive();
        }
        if let Some(t) = self.tray.take() {
            t.shutdown();
        }
        ipc::remove();
        self.exit = true;
    }

    /// The shortcut as it is shown to people.
    fn shortcut_label(&self) -> String {
        self.kde_label.clone().unwrap_or_else(|| self.settings.hotkey.label())
    }

    fn greet(&self) {
        let label = self.shortcut_label();
        let text = if label == "none" {
            "Open it from the tray icon, or give it a shortcut with \"Change shortcut…\" in the tray menu.".to_string()
        } else if self.bus.is_none() {
            format!(
                "Plasma's shortcut service is not available, so {label} does nothing. Bind \"charpick --toggle\" to a shortcut in your desktop's settings."
            )
        } else {
            format!("Press {label} in any text field to pick accented letters and symbols. Hold Shift for uppercase.")
        };
        self.notify("Charpick is ready", &text);
    }

    pub(super) fn update_tray(&self) {
        let Some(t) = &self.tray else { return };
        let hotkey = if self.kde_label.is_some() { None } else { Some(self.settings.hotkey) };
        let label = self.shortcut_label();
        let (style, theme, has_recent) = (self.settings.style, self.settings.theme, !self.recent.items.is_empty());
        let autostart = autostart_path().exists();
        t.update(move |t: &mut Tray| {
            t.hotkey = hotkey;
            t.hotkey_label = label;
            t.style = style;
            t.theme = theme;
            t.has_recent = has_recent;
            t.autostart = autostart;
        });
    }

    // --- The shortcut, shared with Plasma ----------------------------------------

    /// Register with Plasma's shortcut service and agree on the shortcut. When
    /// settings.ini changed since the last agreement it wins; otherwise Plasma's
    /// own wins (it may have been changed in System Settings meanwhile).
    pub(super) fn register_shortcut(&mut self) {
        let Some(bus) = &self.bus else { return };
        if let Err(e) = bus.register() {
            log!("dbus  cannot register the shortcut: {e}");
            return;
        }
        let agreed = std::fs::read_to_string(sync_path()).ok().and_then(|s| Hotkey::parse(&s));
        self.push_shortcut(agreed != Some(self.settings.hotkey));
    }

    /// Hand the shortcut in settings.ini to Plasma.
    fn push_shortcut(&mut self, force: bool) {
        let Some(bus) = &self.bus else { return };
        let wanted = self.settings.hotkey;
        match bus.set_shortcut(wanted, force) {
            Ok(k) => {
                log!("dbus  shortcut {} → {}", wanted.label(), k.label());
                if force && !wanted.is_none() && k.hotkey() != Some(wanted) {
                    self.notify(
                        &format!("{} is already in use", wanted.label()),
                        "Choose another shortcut with \"Change shortcut…\" in Charpick's tray menu, or in System Settings > Keyboard > Shortcuts.",
                    );
                }
                self.adopt(k);
            }
            Err(e) => log!("dbus  cannot set the shortcut: {e}"),
        }
    }

    /// Take over the shortcut that Plasma uses now.
    fn adopt(&mut self, k: KdeShortcut) {
        match k.hotkey() {
            Some(hk) => {
                self.kde_label = None;
                if hk != self.settings.hotkey {
                    self.settings.hotkey = hk;
                    self.settings.save();
                    self.settings_time = Settings::modified();
                }
            }
            // Plasma allows keys that settings.ini cannot name; leave the file alone.
            None => self.kde_label = Some(k.label()),
        }
        self.mark_synced();
        self.update_tray();
    }

    fn mark_synced(&mut self) {
        self.synced = Some(self.settings.hotkey);
        let _ = std::fs::write(sync_path(), self.settings.hotkey.label());
    }

    /// settings.ini or the color scheme may have changed.
    fn watch(&mut self) {
        if !self.visible() {
            self.refresh();
        }
        if self.synced.is_some_and(|s| s != self.settings.hotkey) {
            self.push_shortcut(true);
        }
    }

    /// The recorder got a valid combination.
    pub(super) fn recorded(&mut self, hk: Hotkey) {
        if let Some(bus) = &self.bus {
            if !bus.available(hk) {
                self.picker.record_warn(hk.parts(), "Already used by another shortcut");
                return;
            }
            match bus.set_shortcut(hk, true) {
                Ok(k) if k.hotkey() == Some(hk) => {}
                Ok(_) => {
                    let _ = bus.set_shortcut(self.settings.hotkey, true);
                    self.picker.record_warn(hk.parts(), "Already used by another shortcut");
                    return;
                }
                Err(e) => {
                    log!("dbus  cannot set the shortcut: {e}");
                    self.picker.record_warn(hk.parts(), "Plasma did not accept this shortcut");
                    return;
                }
            }
        }
        log!("app   new shortcut {}", hk.label());
        self.settings.hotkey = hk;
        self.settings.save();
        self.settings_time = Settings::modified();
        self.kde_label = None;
        self.mark_synced();
        self.update_tray();

        self.recording = false;
        if let Some(b) = &self.bus {
            b.block_shortcuts(false);
        }
        self.picker.record_saved(hk);
        if self.bus.is_none() {
            self.picker.record_warn(hk.parts(), "Saved, but Plasma's shortcut service is not running");
        }
        self.timers.close = self
            .handle
            .insert_source(Timer::from_duration(SAVED_FOR), |_, _, st| {
                st.timers.close = None;
                st.hide();
                TimeoutAction::Drop
            })
            .ok();
    }
}

pub fn run(args: &[String], opts: crate::Options) -> i32 {
    let arg = |name: &str| args.iter().any(|a| a.eq_ignore_ascii_case(name));
    if arg("--log") {
        LOG.store(true, Ordering::Relaxed);
        START.get_or_init(Instant::now);
    }
    if arg("--help") || arg("-h") {
        println!("{USAGE}");
        return 0;
    }
    if arg("--quit") {
        ipc::quit_running(Duration::from_secs(5));
        return 0;
    }
    let request = if arg("--toggle") {
        Some(Cmd::Toggle)
    } else if arg("--show") || opts.demo {
        Some(Cmd::Show)
    } else if opts.change_shortcut {
        Some(Cmd::ChangeShortcut)
    } else if opts.autostart {
        None
    } else {
        Some(Cmd::Hello)
    };

    // One Charpick per session: a later start hands its request over and exits.
    let listener = match ipc::bind() {
        Ok(l) => Some(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let name = match request {
                Some(Cmd::Toggle) => "toggle",
                Some(Cmd::Show) => "show",
                Some(Cmd::ChangeShortcut) => "change-shortcut",
                Some(_) => "hello",
                None => return 0,
            };
            ipc::send(name);
            return 0;
        }
        Err(e) => {
            log!("ipc   cannot listen at {}: {e}", ipc::socket_path().display());
            None
        }
    };

    let (mut event_loop, mut state) = match wl::connect(opts) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("Charpick: {e}");
            if let Some(b) = dbus::Bus::connect() {
                b.notify("Charpick cannot start", &e);
            }
            if listener.is_some() {
                ipc::remove();
            }
            return 1;
        }
    };

    let (tx, rx) = calloop::channel::channel::<Cmd>();
    let _ = event_loop.handle().insert_source(rx, |ev, _, st: &mut State| {
        if let Event::Msg(cmd) = ev {
            st.command(cmd);
        }
    });
    if let Some(l) = listener {
        ipc::serve(l, tx.clone());
    }

    state.bus = dbus::Bus::connect();
    if let Some(b) = &state.bus {
        b.listen(tx.clone());
    }
    state.register_shortcut();

    let tray = Tray {
        tx: tx.clone(),
        hotkey: None,
        hotkey_label: String::new(),
        style: state.settings.style,
        theme: state.settings.theme,
        autostart: false,
        has_recent: false,
    };
    state.tray = match ksni::blocking::TrayMethods::spawn(tray) {
        Ok(h) => Some(h),
        Err(e) => {
            log!("tray  not available: {e}");
            None
        }
    };
    state.update_tray();

    let _ = event_loop.handle().insert_source(Timer::from_duration(WATCH_EVERY), |_, _, st: &mut State| {
        st.watch();
        TimeoutAction::ToDuration(WATCH_EVERY)
    });

    if let Some(cmd) = request {
        state.command(cmd);
    }

    let signal = event_loop.get_signal();
    let result = event_loop.run(None, &mut state, |st| {
        if st.exit {
            signal.stop();
        }
    });
    state.recent.flush();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Charpick: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::exec_quote;

    #[test]
    fn desktop_exec_quoting() {
        assert_eq!(exec_quote("/home/a/.local/bin/charpick"), r#""/home/a/.local/bin/charpick""#);
        assert_eq!(exec_quote(r#"/x y/$a`b"c\d%e"#), r#""/x y/\\$a\\`b\\"c\\\\d%%e""#);
    }
}
