//! Charpick: a quick picker for accented letters and symbols, opened with a
//! shortcut (Win+L on Windows, Meta+, on KDE Plasma).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("Charpick runs on Windows and Linux.");

mod data;
mod gfx;
mod picker;
mod settings;
mod theme;
mod util;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod caret;
#[cfg(windows)]
mod d2d;
#[cfg(windows)]
mod diag;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod lockpolicy;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
#[cfg(windows)]
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
#[cfg(windows)]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow, PostMessageW};
#[cfg(windows)]
use windows_core::w;

#[cfg(windows)]
use app::{CMD_DEMO, CMD_HELLO, CMD_QUIT, CMD_RECHECK, CMD_SHORTCUT, CONTROLLER_CLASS, WM_INSTANCE};
use settings::{BackdropChoice, Settings, StyleChoice, ThemeChoice};
#[cfg(windows)]
use util::{APP_KEY, pcwstr, wide};

/// Set in Charpick's key once the one-time Win+L permission was granted, so that
/// uninstalling can take it back.
#[cfg(windows)]
const GRANT_MARKER: &str = "LockAccessGranted";

/// Command line options shared by the platforms.
#[derive(Default)]
pub struct Options {
    /// Started at login: stay quiet.
    pub autostart: bool,
    /// First start after installing: say hello.
    pub welcome: bool,
    /// Open the picker right away.
    pub demo: bool,
    /// Open the shortcut recorder right away.
    pub change_shortcut: bool,
    pub style: Option<StyleChoice>,
    pub theme: Option<ThemeChoice>,
    pub backdrop: Option<BackdropChoice>,
}

impl Options {
    /// Apply the overrides from the command line.
    pub fn apply(&self, s: &mut Settings) {
        if let Some(v) = self.style {
            s.style = v;
        }
        if let Some(v) = self.theme {
            s.theme = v;
        }
        if let Some(v) = self.backdrop {
            s.backdrop = v;
        }
    }
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().any(|a| a.eq_ignore_ascii_case(name));
    let value = |name: &str| {
        args.iter().position(|a| a.eq_ignore_ascii_case(name)).and_then(|i| args.get(i + 1)).map(|v| v.to_ascii_lowercase())
    };

    let opts = Options {
        autostart: arg("--autostart"),
        welcome: arg("--welcome"),
        demo: arg("--demo"),
        change_shortcut: arg("--change-shortcut"),
        style: value("--style").and_then(|v| match v.as_str() {
            "win10" | "10" => Some(StyleChoice::Win10),
            "win11" | "11" => Some(StyleChoice::Win11),
            "kde" | "breeze" | "plasma" => Some(StyleChoice::Kde),
            "auto" => Some(StyleChoice::Auto),
            _ => None,
        }),
        theme: value("--theme").and_then(|v| match v.as_str() {
            "light" => Some(ThemeChoice::Light),
            "dark" => Some(ThemeChoice::Dark),
            "auto" => Some(ThemeChoice::Auto),
            _ => None,
        }),
        backdrop: value("--backdrop").and_then(|v| match v.as_str() {
            "acrylic" | "blur" => Some(BackdropChoice::Acrylic),
            "solid" | "none" => Some(BackdropChoice::Solid),
            "auto" => Some(BackdropChoice::Auto),
            _ => None,
        }),
    };

    #[cfg(feature = "snapshot")]
    if let Some(path) = args.iter().position(|a| a == "--snapshot").and_then(|i| args.get(i + 1)) {
        return snapshot(path, &opts, &value);
    }

    #[cfg(target_os = "linux")]
    return linux::run(&args, opts);
    #[cfg(windows)]
    return run_windows(&args, opts);
}

#[cfg(windows)]
fn run_windows(args: &[String], opts: Options) -> i32 {
    let arg = |name: &str| args.iter().any(|a| a.eq_ignore_ascii_case(name));
    let value = |name: &str| {
        args.iter().position(|a| a.eq_ignore_ascii_case(name)).and_then(|i| args.get(i + 1)).map(|v| v.to_ascii_lowercase())
    };

    // Elevated helper started by `lockpolicy::run_elevated` or the installer.
    for (flag, revoke) in [("--grant-lock-access", false), ("--revoke-lock-access", true)] {
        if arg(flag) {
            let Some(sid) = value(flag) else { return 2 };
            return match lockpolicy::grant(&sid.to_ascii_uppercase(), revoke) {
                Ok(()) => 0,
                Err(_) => 1,
            };
        }
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }

    if arg("--setup-lock") {
        return setup_lock();
    }
    if arg("--uninstall-cleanup") {
        uninstall_cleanup();
        return 0;
    }
    if arg("--quit") {
        quit_running(5000);
        return 0;
    }

    // One instance per session. A second start just talks to the first one.
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\Charpick.SingleInstance")) };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if arg("--log") {
        // Logging needs a fresh start, so this copy replaces the running one.
        if already {
            quit_running(5000);
        }
        diag::start();
    } else if already {
        let cmd = if opts.demo {
            CMD_DEMO
        } else if opts.change_shortcut {
            CMD_SHORTCUT
        } else if opts.autostart {
            return 0;
        } else {
            CMD_HELLO
        };
        tell_running(cmd);
        return 0;
    }

    let code = app::run(opts);
    if let Ok(m) = mutex {
        unsafe {
            let _ = CloseHandle(m);
        }
    }
    code
}

/// Post a command to the running instance. Returns false when there is none.
#[cfg(windows)]
fn tell_running(cmd: usize) -> bool {
    unsafe {
        match FindWindowW(CONTROLLER_CLASS, None) {
            Ok(h) if !h.is_invalid() => PostMessageW(Some(h), WM_INSTANCE, WPARAM(cmd), LPARAM(0)).is_ok(),
            _ => false,
        }
    }
}

/// Ask the running instance to quit and wait for it (used by the installer).
#[cfg(windows)]
fn quit_running(timeout_ms: u32) {
    unsafe {
        let Ok(h) = FindWindowW(CONTROLLER_CLASS, None) else { return };
        if h.is_invalid() {
            return;
        }
        let _ = PostMessageW(Some(h), WM_INSTANCE, WPARAM(CMD_QUIT), LPARAM(0));
        let mut waited = 0;
        while waited < timeout_ms && IsWindow(Some(h)).as_bool() {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 50;
        }
    }
}

/// Installer task: grant the Win+L permission (asks for elevation) and tell the
/// running instance to pick it up.
#[cfg(windows)]
fn setup_lock() -> i32 {
    if lockpolicy::writable() {
        // Already set up (an update or a reinstall).
        tell_running(CMD_RECHECK);
        return 0;
    }
    match lockpolicy::run_elevated(false) {
        lockpolicy::Elevation::Done => {
            util::reg_set_dword(HKEY_CURRENT_USER, APP_KEY, GRANT_MARKER, 1);
            tell_running(CMD_RECHECK);
            0
        }
        lockpolicy::Elevation::Cancelled => 1,
        lockpolicy::Elevation::Failed(e) => {
            util::error_box(None, &format!("Could not set up Win+L for Charpick.\n\n{e}"));
            1
        }
    }
}

/// Uninstaller: stop Charpick and undo everything it changed.
#[cfg(windows)]
fn uninstall_cleanup() {
    quit_running(5000);
    lockpolicy::disarm();
    util::set_autostart(false);
    if util::reg_get_dword(HKEY_CURRENT_USER, APP_KEY, GRANT_MARKER) == Some(1) {
        // Declining the prompt leaves the (harmless) permission in place.
        let _ = lockpolicy::run_elevated(true);
    }
    let key = wide(APP_KEY);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, pcwstr(&key));
    }
}

/// `--snapshot <file.png> [--style ..] [--theme ..] [--query ..] [--upper] [--dpi ..] [--hover ..] [--select ..] [--prompt]`
#[cfg(feature = "snapshot")]
fn snapshot(path: &str, opts: &Options, value: &dyn Fn(&str) -> Option<String>) -> i32 {
    let mut s = Settings::load();
    opts.apply(&mut s);
    let flag = |name: &str| std::env::args().any(|a| a == name);

    #[cfg(windows)]
    let Ok(mut gfx) = d2d::Gfx::new(windows::Win32::Foundation::HWND::default()) else { return 1 };
    #[cfg(windows)]
    let theme = theme::Theme::resolve(&s, &util::system_look(), &|f| gfx.font_exists(f));
    #[cfg(target_os = "linux")]
    let fonts = linux::paint::font_families();
    #[cfg(target_os = "linux")]
    let theme = theme::Theme::resolve(&s, &linux::kde::system_look(true), &|f| fonts.contains(&f.to_lowercase()));

    let mut p = picker::Picker::new(data::Catalog::build(), &s, theme);
    #[cfg(target_os = "linux")]
    {
        p.own_frame = true;
    }
    p.recent = ["à", "è", "ș", "ț", "ă", "€"].iter().filter_map(|k| p.catalog.entry_by_text(k)).collect();
    if let Some(dpi) = value("--dpi").and_then(|v| v.parse().ok()) {
        p.set_dpi(dpi);
    }
    p.reset();
    for c in value("--query").unwrap_or_default().chars() {
        p.type_char(c);
    }
    p.upper = flag("--upper");
    for _ in 0..value("--select").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0) {
        p.navigate(picker::Nav::Right);
    }
    if let Some(i) = value("--hover").and_then(|v| v.parse().ok()) {
        p.set_hover_cell(i);
    }
    if flag("--prompt") {
        p.prompt = Some(picker::Prompt::record_shortcut(&s.hotkey, settings::Hotkey { vk: 0, mods: settings::MOD_CTRL | settings::MOD_ALT }));
    }

    #[cfg(windows)]
    let result = gfx.snapshot(&p, path).map_err(|e| e.to_string());
    #[cfg(target_os = "linux")]
    let result = linux::paint::snapshot(&p, path);
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
