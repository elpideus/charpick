//! The controller: tray icon, the picker window, hotkey and caret messages,
//! typing the chosen character, recording a new shortcut and the Win+L lock policy.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::Ordering::Relaxed;
use std::time::SystemTime;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DWMWINDOWATTRIBUTE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows::Win32::System::RemoteDesktop::{NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Controls::{LIM_SMALL, LoadIconMetric, MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetLastInputInfo, LASTINPUTINFO, ReleaseCapture, SetCapture, TME_LEAVE,
    TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEKEYBOARD,
    RegisterRawInputDevices,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONUSERCLICK, NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, GUID, PCWSTR, s, w};

use crate::Options;
use crate::caret::{self, CaretWorker, WM_CARET};
use crate::d2d::Gfx;
use crate::data::Catalog;
use crate::diag::{self, diag};
use crate::hook;
use crate::lockpolicy::{self, Elevation};
use crate::picker::{Action, Nav, Picker, Recorded};
use crate::settings::{Hotkey, Recent, Settings, StyleChoice, ThemeChoice};
use crate::theme::{Backdrop, Kind, Theme};
use crate::util::{self, APP_KEY, APP_NAME, copy_wide, pcwstr, wide};

pub const CONTROLLER_CLASS: PCWSTR = w!("CharpickController");
const PICKER_CLASS: PCWSTR = w!("CharpickPicker");

/// Posted by a second instance. wParam is one of the `CMD_*` values.
pub const WM_INSTANCE: u32 = WM_APP + 30;
pub const CMD_HELLO: usize = 0;
pub const CMD_QUIT: usize = 1;
pub const CMD_RECHECK: usize = 2;
pub const CMD_DEMO: usize = 3;
pub const CMD_SHORTCUT: usize = 4;

const WM_TRAY: u32 = WM_APP + 10;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

// Controller timers.
const T_CARET: usize = 1;
const T_IDLE: usize = 2;
const T_THEME: usize = 3;
/// Close the recorder after showing the saved shortcut.
const T_SAVED: usize = 4;
// Picker timers.
const T_BLINK: usize = 10;
const T_ANIM: usize = 11;

/// How long to wait for the caret position before opening at the mouse pointer.
const CARET_TIMEOUT_MS: u32 = 200;
/// Give Win+L back to Windows after this much time without input.
const IDLE_DISARM_MS: u32 = 30_000;

const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);
const VK_SHIFT: i32 = 0x10;
const VK_CAPITAL: i32 = 0x14;

// Tray menu commands.
const ID_LOCK: u32 = 10;
const ID_SHORTCUT: u32 = 15;
const ID_STYLE_AUTO: u32 = 20;
const ID_STYLE_10: u32 = 21;
const ID_STYLE_11: u32 = 22;
const ID_STYLE_KDE: u32 = 23;
const ID_THEME_AUTO: u32 = 30;
const ID_THEME_LIGHT: u32 = 31;
const ID_THEME_DARK: u32 = 32;
const ID_WINL_SETUP: u32 = 40;
const ID_AUTOSTART: u32 = 50;
const ID_CLEAR_RECENT: u32 = 60;
const ID_SETTINGS: u32 = 70;
const ID_HELP: u32 = 80;
const ID_QUIT: u32 = 99;

/// What Win+L does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WinL {
    /// The picker uses another hotkey.
    Off,
    /// Win+L is wanted but the one-time permission is missing.
    NeedsSetup,
    /// Locking is already disabled by an administrator, so Win+L is always free.
    ByPolicy,
    /// Charpick sets and clears the lock policy as needed.
    Ready,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BalloonAction {
    None,
    SetupWinL,
}

struct App {
    ctl: HWND,
    win: HWND,
    hinst: HINSTANCE,
    gfx: Gfx,
    picker: Picker,
    settings: Settings,
    settings_time: Option<SystemTime>,
    opts: Options,
    recent: Recent,
    caret: CaretWorker,
    req: usize,
    pending: bool,
    visible: bool,
    /// The picker shows the shortcut recorder.
    recording: bool,
    target: HWND,
    /// The picker's screen rectangle in physical pixels.
    rect: RECT,
    shift: bool,
    caps: bool,
    winl: WinL,
    taskbar_created: u32,
    tray_icon: HICON,
    balloon_action: BalloonAction,
    fg_hook: Option<HWINEVENTHOOK>,
    tracking: bool,
    anim_timer: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` on the application state. Returns None when the state is already in
/// use further up the stack (a message sent while handling another one).
fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn loword(v: isize) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}

fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

// ---------------------------------------------------------------------------
// Window decoration

fn dwm_set(win: HWND, attr: DWMWINDOWATTRIBUTE, value: u32) {
    unsafe {
        let _ = DwmSetWindowAttribute(win, attr, &value as *const u32 as *const c_void, 4);
    }
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

/// Acrylic blur through the undocumented accent policy (works on Windows 10 and 11).
fn set_accent(win: HWND, acrylic: bool, gradient: u32) {
    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let f: SetWca = std::mem::transmute(f);
        let mut policy = AccentPolicy { state: if acrylic { 4 } else { 0 }, flags: 0, gradient, animation: 0 };
        let mut data = CompositionData {
            attribute: 19,
            data: &mut policy as *mut AccentPolicy as *mut c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        let _ = f(win, &mut data);
    }
}

fn resolve_theme(settings: &Settings, gfx: &Gfx) -> Theme {
    Theme::resolve(settings, &util::system_look(), &|f| gfx.font_exists(f))
}

fn apply_window_style(win: HWND, t: &Theme) {
    dwm_set(win, DWMWA_USE_IMMERSIVE_DARK_MODE, t.c.dark as u32);
    dwm_set(win, DWMWA_TRANSITIONS_FORCEDISABLED, 1);
    if util::is_windows11() {
        let round = t.kind != Kind::Win10;
        dwm_set(
            win,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            if round { DWMWCP_ROUND.0 as u32 } else { DWMWCP_DONOTROUND.0 as u32 },
        );
        // The Windows 10 look draws its own square border.
        dwm_set(win, DWMWA_BORDER_COLOR, if round { 0xFFFF_FFFF } else { 0xFFFF_FFFE });
    }
    let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(win, &margins);
    }
    if util::os_build() >= 22621 {
        let backdrop = if t.backdrop == Backdrop::System { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
        dwm_set(win, DWMWA_SYSTEMBACKDROP_TYPE, backdrop.0 as u32);
    }
    set_accent(win, t.backdrop == Backdrop::Accent, t.accent_gradient());
}

fn cloak(win: HWND, on: bool) {
    dwm_set(win, DWMWA_CLOAK, on as u32);
}

fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

// ---------------------------------------------------------------------------
// Lock policy helpers that need no application state

fn disarm_policy(ctl: HWND) {
    if hook::WINL_ENABLED.load(Relaxed) && hook::ARMED.load(Relaxed) {
        lockpolicy::disarm();
        hook::arm_done(false);
    }
    unsafe {
        let _ = KillTimer(Some(ctl), T_IDLE);
    }
}

fn idle_check(ctl: HWND) {
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut li).as_bool() && GetTickCount().wrapping_sub(li.dwTime) >= IDLE_DISARM_MS {
            diag!("ctl   idle: disarming");
            disarm_policy(ctl);
        }
    }
}

// ---------------------------------------------------------------------------

impl App {
    fn scale(&self) -> f32 {
        self.picker.dpi() as f32 / 96.0
    }

    fn load_settings(&mut self) {
        let mut s = Settings::load();
        self.opts.apply(&mut s);
        self.settings = s;
        self.settings_time = Settings::modified();
    }

    fn save_settings(&mut self) {
        self.settings.save();
        self.settings_time = Settings::modified();
    }

    /// Apply the current settings and Windows theme everywhere.
    fn apply_settings(&mut self) {
        let theme = resolve_theme(&self.settings, &self.gfx);
        self.picker.apply_settings(&self.settings, theme);
        apply_window_style(self.win, &self.picker.theme);
        hook::set_hotkeys(self.settings.hotkey, self.settings.lock_hotkey);
        self.update_winl();
        self.tray_update();
    }

    fn update_winl(&mut self) {
        let new = if self.settings.hotkey != Hotkey::WIN_L {
            WinL::Off
        } else if lockpolicy::disabled_by_admin() {
            WinL::ByPolicy
        } else if lockpolicy::writable() {
            WinL::Ready
        } else {
            WinL::NeedsSetup
        };
        if new != WinL::Ready && self.winl == WinL::Ready {
            lockpolicy::disarm();
        }
        let armed = match new {
            WinL::ByPolicy => true,
            WinL::Ready => self.winl == WinL::Ready && hook::ARMED.load(Relaxed),
            _ => false,
        };
        hook::WINL_ENABLED.store(new == WinL::Ready, Relaxed);
        hook::arm_done(armed);
        self.winl = new;
    }

    // --- Showing and hiding --------------------------------------------------

    fn begin_show(&mut self) {
        if self.visible || self.pending {
            diag!("app   already {}", if self.visible { "visible" } else { "opening" });
            return;
        }
        if Settings::modified() != self.settings_time {
            self.load_settings();
            self.apply_settings();
        }
        self.target = unsafe { GetForegroundWindow() };
        self.shift = key_down(VK_SHIFT);
        self.caps = unsafe { GetKeyState(VK_CAPITAL) } & 1 != 0;
        self.picker.upper = self.upper();
        let recent: Vec<u32> = self
            .recent
            .items
            .iter()
            .filter_map(|k| self.picker.catalog.entry_by_text(k))
            .take(self.settings.recent_count)
            .collect();
        self.picker.recent = recent;
        self.picker.reset();
        self.pending = true;
        // Route keys to the picker right away, so fast typing is not lost.
        hook::set_visible(true, RECT::default());
        self.req = self.req.wrapping_add(1);
        diag!("app   opening: target {}", window_class(self.target));
        if self.caret.request(self.req, self.target) {
            unsafe {
                SetTimer(Some(self.ctl), T_CARET, CARET_TIMEOUT_MS, None);
            }
        } else {
            self.show_at(None);
        }
    }

    fn caret_done(&mut self, id: usize, info: Option<caret::CaretInfo>) {
        diag!(
            "app   caret {} (request {id}, current {}, pending={})",
            if info.is_some() { "found" } else { "not found" },
            self.req,
            self.pending
        );
        if self.pending && id == self.req {
            self.show_at(info.map(|i| i.rect));
        }
    }

    /// Open the picker next to `anchor` (the caret), or next to the mouse pointer.
    fn show_at(&mut self, anchor: Option<RECT>) {
        unsafe {
            let _ = KillTimer(Some(self.ctl), T_CARET);
        }
        self.pending = false;
        let anchor = anchor.unwrap_or_else(|| {
            let p = util::cursor_pos();
            RECT { left: p.x, top: p.y, right: p.x + 8, bottom: p.y + 16 }
        });
        {
            let (work, dx) = Self::monitor(&anchor);
            self.picker.set_dpi(dx);
            let (w, h) = self.picker.size_px(dx);
            let s = dx as f32 / 96.0;
            let (gx, gy) = ((8.0 * s).round() as i32, (6.0 * s).round() as i32);
            // Slightly to the right of the caret and just below its line; flip to the
            // left or above when there is no room.
            let mut x = anchor.right + gx;
            if x + w > work.right {
                x = anchor.left - gx - w;
            }
            let mut y = anchor.bottom + gy;
            if y + h > work.bottom {
                y = anchor.top - gy - h;
            }
            x = x.clamp(work.left, (work.right - w).max(work.left));
            y = y.clamp(work.top, (work.bottom - h).max(work.top));
            self.present(x, y, w, h);
        }
    }

    /// The work area and DPI of the monitor showing most of `r`.
    fn monitor(r: &RECT) -> (RECT, u32) {
        unsafe {
            let mon = MonitorFromRect(r, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            (mi.rcWork, dx)
        }
    }

    /// Put the picker on screen at `x`, `y` (physical pixels).
    fn present(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            cloak(self.win, true);
            let _ = SetWindowPos(self.win, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            // Draw as an active window so that the acrylic backdrop stays on.
            SendMessageW(self.win, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
            self.visible = true;
            self.picker.caret_on = true;
            self.gfx.render(&self.picker);
            cloak(self.win, false);
            hook::set_visible(true, self.rect);
            diag!("app   shown at {x},{y} {w}x{h}");
            self.start_blink();
            if self.fg_hook.is_none() {
                let h = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(on_foreground),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                self.fg_hook = (!h.is_invalid()).then_some(h);
            }
        }
    }

    /// Open the shortcut recorder in the middle of the screen with the mouse pointer.
    fn record_shortcut(&mut self) {
        self.hide();
        if Settings::modified() != self.settings_time {
            self.load_settings();
            self.apply_settings();
        }
        self.picker.reset();
        self.picker.record_start(self.settings.hotkey);
        self.recording = true;
        hook::RECORDING.store(true, Relaxed);
        self.target = unsafe { GetForegroundWindow() };
        let p = util::cursor_pos();
        let (work, dpi) = Self::monitor(&RECT { left: p.x, top: p.y, right: p.x + 1, bottom: p.y + 1 });
        self.picker.set_dpi(dpi);
        let (w, h) = self.picker.size_px(dpi);
        diag!("app   recording a shortcut");
        self.present((work.left + work.right - w) / 2, (work.top + work.bottom - h) / 2, w, h);
    }

    /// A key reached the shortcut recorder.
    fn recorded(&mut self, vk: u32, mods: u8) {
        if !self.recording {
            return;
        }
        match self.picker.record(Hotkey { vk: vk as u8, mods }) {
            Recorded::Pending => self.redraw(),
            Recorded::Cancel => self.hide(),
            Recorded::Done(hk) => {
                diag!("app   new shortcut {}", hk.label());
                self.recording = false;
                hook::RECORDING.store(false, Relaxed);
                self.settings.hotkey = hk;
                self.save_settings();
                self.apply_settings();
                self.picker.record_saved(hk);
                self.redraw();
                unsafe {
                    SetTimer(Some(self.ctl), T_SAVED, 1400, None);
                }
                if self.winl == WinL::NeedsSetup {
                    self.greet(false);
                }
            }
        }
    }

    fn hide(&mut self) {
        let was = self.visible || self.pending;
        if was {
            diag!("app   hidden (was {})", if self.visible { "visible" } else { "opening" });
        }
        self.pending = false;
        self.recording = false;
        self.picker.prompt = None;
        hook::RECORDING.store(false, Relaxed);
        hook::set_visible(false, RECT::default());
        unsafe {
            let _ = KillTimer(Some(self.ctl), T_CARET);
            let _ = KillTimer(Some(self.ctl), T_SAVED);
            if self.visible {
                self.visible = false;
                let _ = ShowWindow(self.win, SW_HIDE);
                let _ = KillTimer(Some(self.win), T_BLINK);
                let _ = KillTimer(Some(self.win), T_ANIM);
                self.anim_timer = false;
                self.tracking = false;
                if let Some(h) = self.fg_hook.take() {
                    let _ = UnhookWinEvent(h);
                }
            }
        }
        if was {
            self.recent.flush();
        }
    }

    fn start_blink(&mut self) {
        let ms = unsafe { GetCaretBlinkTime() };
        if ms > 0 && ms != u32::MAX {
            unsafe {
                SetTimer(Some(self.win), T_BLINK, ms, None);
            }
        }
    }

    fn redraw(&mut self) {
        if !self.visible {
            return;
        }
        // Keep the search caret solid while something happens.
        self.picker.caret_on = true;
        self.start_blink();
        unsafe {
            let _ = InvalidateRect(Some(self.win), None, false);
        }
        if self.picker.animating() && !self.anim_timer {
            self.anim_timer = true;
            unsafe {
                SetTimer(Some(self.win), T_ANIM, 10, None);
            }
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Redraw => self.redraw(),
            Action::Close => self.hide(),
            Action::Insert { text, key, close } => {
                if close {
                    self.hide();
                }
                hook::send_text(&text);
                if self.settings.recent_count > 0 {
                    self.recent.push(&key, self.settings.recent_count);
                }
                if close {
                    self.recent.flush();
                } else {
                    self.redraw();
                }
            }
        }
    }

    // --- Input -----------------------------------------------------------------

    fn upper(&self) -> bool {
        self.shift ^ (self.caps && self.settings.caps_lock_uppercase)
    }

    fn key(&mut self, vk: u32, flags: isize) {
        if !self.visible && !self.pending {
            return;
        }
        let shift = flags & hook::KEY_SHIFT != 0;
        let ctrl = flags & hook::KEY_CTRL != 0;
        let p = &mut self.picker;
        let action = match vk {
            0x25 => p.navigate(Nav::Left),
            0x27 => p.navigate(Nav::Right),
            0x26 => p.navigate(Nav::Up),
            0x28 => p.navigate(Nav::Down),
            0x24 => p.navigate(Nav::Home),
            0x23 => p.navigate(Nav::End),
            0x21 => p.navigate(Nav::PageUp),
            0x22 => p.navigate(Nav::PageDown),
            0x09 => p.navigate(if shift { Nav::PrevSection } else { Nav::NextSection }),
            0x0D => p.enter(),
            0x1B => p.escape(),
            0x08 => p.backspace(ctrl),
            0x2E => p.clear_query(),
            _ => Action::None,
        };
        self.act(action);
    }

    fn typed(&mut self, c: u32) {
        if !self.visible && !self.pending {
            return;
        }
        if let Some(c) = char::from_u32(c) {
            let a = self.picker.type_char(c);
            self.act(a);
        }
    }

    fn mods(&mut self, wp: usize) {
        self.shift = wp & 1 != 0;
        if wp & 2 != 0 {
            self.caps = !self.caps;
        }
        let upper = self.upper();
        if upper != self.picker.upper {
            self.picker.upper = upper;
            self.redraw();
        }
    }

    fn wheel(&mut self, wp: usize, lp: LPARAM) {
        if !self.visible {
            return;
        }
        let delta = wp as u16 as i16;
        let horizontal = wp & (1 << 16) != 0;
        let pt = hook::wheel_point(lp);
        let y = (pt.y - self.rect.top) as f32 / self.scale();
        let a = self.picker.wheel(delta, horizontal, y);
        self.act(a);
        self.refresh_hover();
    }

    /// Recompute the hovered cell after the content moved under the pointer.
    fn refresh_hover(&mut self) {
        let p = util::cursor_pos();
        if util::rect_contains(&self.rect, p.x, p.y) {
            let s = self.scale();
            let a = self.picker.mouse_move((p.x - self.rect.left) as f32 / s, (p.y - self.rect.top) as f32 / s);
            self.act(a);
        }
    }

    fn mouse(&mut self, msg: u32, lp: LPARAM) {
        if !self.visible {
            return;
        }
        let s = self.scale();
        let (x, y) = (loword(lp.0) as f32 / s, hiword(lp.0) as f32 / s);
        let action = unsafe {
            match msg {
                WM_MOUSEMOVE => {
                    if !self.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: self.win,
                            dwHoverTime: 0,
                        };
                        self.tracking = TrackMouseEvent(&mut tme).is_ok();
                    }
                    self.picker.mouse_move(x, y)
                }
                WM_LBUTTONDOWN => {
                    SetCapture(self.win);
                    self.picker.mouse_down(x, y)
                }
                WM_LBUTTONUP => {
                    let _ = ReleaseCapture();
                    self.picker.mouse_up(x, y)
                }
                _ => Action::None,
            }
        };
        self.act(action);
    }

    fn mouse_leave(&mut self) {
        self.tracking = false;
        let a = self.picker.mouse_leave();
        self.act(a);
    }

    fn picker_timer(&mut self, id: usize) {
        match id {
            T_BLINK => {
                self.picker.caret_on = !self.picker.caret_on;
                unsafe {
                    let _ = InvalidateRect(Some(self.win), None, false);
                }
            }
            T_ANIM => {
                let more = self.picker.tick();
                unsafe {
                    let _ = InvalidateRect(Some(self.win), None, false);
                }
                if !more {
                    self.anim_timer = false;
                    unsafe {
                        let _ = KillTimer(Some(self.win), T_ANIM);
                    }
                    self.refresh_hover();
                }
            }
            _ => {}
        }
    }

    // --- Tray icon ---------------------------------------------------------------

    fn tip(&self) -> String {
        match self.winl {
            WinL::NeedsSetup => format!("{APP_NAME}: Win+L needs a one-time setup (right-click)"),
            _ => format!("{APP_NAME}: press {} in a text field", self.settings.hotkey.label()),
        }
    }

    fn tray_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.ctl,
            uID: 1,
            ..Default::default()
        }
    }

    fn load_tray_icon(&mut self) {
        // Resource 2 is drawn for dark taskbars, 3 for light ones.
        let id = if util::taskbar_dark() { 2usize } else { 3 };
        unsafe {
            if let Ok(icon) = LoadIconMetric(Some(self.hinst), PCWSTR(id as *const u16), LIM_SMALL) {
                if !self.tray_icon.is_invalid() {
                    let _ = DestroyIcon(self.tray_icon);
                }
                self.tray_icon = icon;
            }
        }
    }

    fn tray_add(&mut self) {
        self.load_tray_icon();
        let mut nid = self.tray_data();
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.tray_icon;
        copy_wide(&mut nid.szTip, &self.tip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn tray_update(&mut self) {
        self.load_tray_icon();
        let mut nid = self.tray_data();
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.hIcon = self.tray_icon;
        copy_wide(&mut nid.szTip, &self.tip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn tray_remove(&self) {
        let nid = self.tray_data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    fn balloon(&mut self, title: &str, text: &str, action: BalloonAction) {
        self.balloon_action = action;
        let mut nid = self.tray_data();
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
        copy_wide(&mut nid.szInfoTitle, title);
        copy_wide(&mut nid.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn greet(&mut self, welcome: bool) {
        match self.winl {
            WinL::NeedsSetup => self.balloon(
                "Win+L needs a one-time setup",
                "Click here to let Win+L open Charpick. Windows will ask for administrator approval once.",
                BalloonAction::SetupWinL,
            ),
            _ if welcome => {
                let text = format!(
                    "Press {} in any text field to pick accented letters and symbols. Hold Shift for uppercase.",
                    self.settings.hotkey.label()
                );
                self.balloon("Charpick is ready", &text, BalloonAction::None);
            }
            _ => {}
        }
    }

    fn build_menu(&self) -> Option<HMENU> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let add = |m: HMENU, flags: MENU_ITEM_FLAGS, id: u32, text: &str| {
                let t = wide(text);
                let _ = AppendMenuW(m, flags, id as usize, pcwstr(&t));
            };
            let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };

            add(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                &format!("{APP_NAME}: {} opens the picker", self.settings.hotkey.label()),
            );
            add(menu, MF_STRING, ID_SHORTCUT, "Change shortcut…");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            match self.winl {
                WinL::Ready => {
                    let lk = self.settings.lock_hotkey;
                    let label =
                        if lk.is_none() { "Lock this PC".to_string() } else { format!("Lock this PC\t{}", lk.label()) };
                    add(menu, MF_STRING, ID_LOCK, &label);
                }
                WinL::NeedsSetup => add(menu, MF_STRING, ID_WINL_SETUP, "Let Win+L open Charpick…"),
                _ => {}
            }

            let auto_style = if util::is_windows11() { "Automatic (Windows 11)" } else { "Automatic (Windows 10)" };
            if let Ok(sub) = CreatePopupMenu() {
                add(sub, MF_STRING, ID_STYLE_AUTO, auto_style);
                add(sub, MF_STRING, ID_STYLE_10, "Windows 10");
                add(sub, MF_STRING, ID_STYLE_11, "Windows 11");
                add(sub, MF_STRING, ID_STYLE_KDE, "KDE Plasma");
                let cur = match self.settings.style {
                    StyleChoice::Auto => ID_STYLE_AUTO,
                    StyleChoice::Win10 => ID_STYLE_10,
                    StyleChoice::Win11 => ID_STYLE_11,
                    StyleChoice::Kde => ID_STYLE_KDE,
                };
                let _ = CheckMenuRadioItem(sub, ID_STYLE_AUTO, ID_STYLE_KDE, cur, MF_BYCOMMAND.0);
                let t = wide("Style");
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, pcwstr(&t));
            }
            if let Ok(sub) = CreatePopupMenu() {
                add(sub, MF_STRING, ID_THEME_AUTO, "Automatic");
                add(sub, MF_STRING, ID_THEME_LIGHT, "Light");
                add(sub, MF_STRING, ID_THEME_DARK, "Dark");
                let cur = match self.settings.theme {
                    ThemeChoice::Auto => ID_THEME_AUTO,
                    ThemeChoice::Light => ID_THEME_LIGHT,
                    ThemeChoice::Dark => ID_THEME_DARK,
                };
                let _ = CheckMenuRadioItem(sub, ID_THEME_AUTO, ID_THEME_DARK, cur, MF_BYCOMMAND.0);
                let t = wide("Theme");
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, pcwstr(&t));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(menu, MF_STRING | check(util::autostart_enabled()), ID_AUTOSTART, "Start with Windows");
            let no_recent = if self.recent.items.is_empty() { MF_GRAYED } else { MF_ENABLED };
            add(menu, MF_STRING | no_recent, ID_CLEAR_RECENT, "Clear recently used");
            add(menu, MF_STRING, ID_SETTINGS, "Edit settings…");
            add(menu, MF_STRING, ID_HELP, "How to use");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(menu, MF_STRING, ID_QUIT, "Quit Charpick");
            Some(menu)
        }
    }

    fn set_style(&mut self, style: StyleChoice) {
        self.opts.style = None;
        self.settings.style = style;
        self.save_settings();
        self.apply_settings();
    }

    fn set_theme(&mut self, theme: ThemeChoice) {
        self.opts.theme = None;
        self.settings.theme = theme;
        self.save_settings();
        self.apply_settings();
    }

    fn shutdown(&mut self) {
        self.hide();
        self.recent.flush();
        if self.winl == WinL::Ready {
            lockpolicy::disarm();
        }
        hook::WINL_ENABLED.store(false, Relaxed);
        hook::arm_done(false);
        self.tray_remove();
    }
}

// ---------------------------------------------------------------------------
// Commands that may show modal UI run without holding the application state.

fn show_menu(ctl: HWND, x: i32, y: i32) {
    with(|a| a.hide());
    let Some(menu) = with(|a| a.build_menu()).flatten() else { return };
    unsafe {
        let _ = SetForegroundWindow(ctl);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 { TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align;
        let cmd = TrackPopupMenuEx(menu, flags.0, x, y, ctl, None);
        let _ = PostMessageW(Some(ctl), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if cmd.0 > 0 {
            command(ctl, cmd.0 as u32);
        }
    }
}

const HELP: &str = "\
Press the hotkey (Win+L by default) while typing in any app.

•  Type to search: a letter (\"s\"), a name (\"e grave\", \"cedilla\"), a symbol (\"euro\") or a language (\"romanian\", \"română\", \"ro\").
•  Arrow keys, Page Up/Down and Home/End move around; Tab and Shift+Tab jump between languages.
•  Enter types the character and closes the picker. Clicking types it and keeps the picker open.
•  Hold Shift (or turn on Caps Lock) for uppercase letters.
•  Esc clears the search, then closes.

To use another shortcut, choose \"Change shortcut…\" in the tray menu and press the new one.

While Win+L opens Charpick, lock the PC with Ctrl+Win+L or from the tray menu.";

fn command(ctl: HWND, id: u32) {
    match id {
        ID_LOCK => lockpolicy::lock_now(),
        ID_SHORTCUT => _ = with(|a| a.record_shortcut()),
        ID_STYLE_AUTO => _ = with(|a| a.set_style(StyleChoice::Auto)),
        ID_STYLE_10 => _ = with(|a| a.set_style(StyleChoice::Win10)),
        ID_STYLE_11 => _ = with(|a| a.set_style(StyleChoice::Win11)),
        ID_STYLE_KDE => _ = with(|a| a.set_style(StyleChoice::Kde)),
        ID_THEME_AUTO => _ = with(|a| a.set_theme(ThemeChoice::Auto)),
        ID_THEME_LIGHT => _ = with(|a| a.set_theme(ThemeChoice::Light)),
        ID_THEME_DARK => _ = with(|a| a.set_theme(ThemeChoice::Dark)),
        ID_WINL_SETUP => setup_winl(ctl),
        ID_AUTOSTART => {
            let on = !util::autostart_enabled();
            if !util::set_autostart(on) {
                util::error_box(None, "Could not change the startup setting.");
            }
        }
        ID_CLEAR_RECENT => _ = with(|a| a.recent.clear()),
        ID_SETTINGS => open_settings(),
        ID_HELP => util::info_box(None, HELP),
        ID_QUIT => unsafe {
            let _ = DestroyWindow(ctl);
        },
        _ => {}
    }
}

fn setup_winl(_ctl: HWND) {
    match lockpolicy::run_elevated(false) {
        Elevation::Done => {
            util::reg_set_dword(windows::Win32::System::Registry::HKEY_CURRENT_USER, APP_KEY, "LockAccessGranted", 1);
            with(|a| {
                a.update_winl();
                a.tray_update();
                if a.winl == WinL::Ready {
                    a.balloon(
                        "Win+L now opens Charpick",
                        "Lock the PC with Ctrl+Win+L or from the tray menu.",
                        BalloonAction::None,
                    );
                }
            });
        }
        Elevation::Cancelled => {}
        Elevation::Failed(e) => util::error_box(None, &format!("Could not set up Win+L.\n\n{e}")),
    }
}

fn open_settings() {
    let path = Settings::path();
    if !path.exists() {
        Settings::default().save();
    }
    let p = wide(&path.to_string_lossy());
    unsafe {
        let r = ShellExecuteW(None, w!("open"), pcwstr(&p), None, None, SW_SHOWNORMAL);
        if r.0 as isize <= 32 {
            ShellExecuteW(None, w!("open"), w!("notepad.exe"), pcwstr(&p), None, SW_SHOWNORMAL);
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostics (only while logging)

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n > 0 { String::from_utf16_lossy(&buf[..n as usize]) } else { "(none)".into() }
}

/// Log a raw keyboard event: modifier and shortcut keys by name, others as `other`.
fn log_raw_input(lp: LPARAM) {
    if !diag::enabled() {
        return;
    }
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let n = unsafe {
        GetRawInputData(
            HRAWINPUT(lp.0 as _),
            RID_INPUT,
            Some(&mut raw as *mut _ as *mut _),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if n == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    let kb = unsafe { raw.data.keyboard };
    let hk = Hotkey::unpack(hook::HOTKEY.load(Relaxed));
    let lk = Hotkey::unpack(hook::LOCK_HOTKEY.load(Relaxed));
    let up = kb.Flags as u32 & RI_KEY_BREAK != 0;
    diag!(
        "raw   {:<9} {}{}",
        diag::key_name(kb.VKey as u32, &[hk.vk, lk.vk]),
        if up { "up" } else { "down" },
        if raw.header.hDevice.is_invalid() { " (no device: injected)" } else { "" }
    );
}

// ---------------------------------------------------------------------------
// Window procedures

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with(|a| {
        // The recorder stays until a click outside: opening it from the tray menu
        // moves the foreground around.
        if (a.visible || a.pending) && !a.recording && hwnd != a.target && hwnd != a.win && hwnd != a.ctl {
            diag!("app   foreground changed to {}: closing", window_class(hwnd));
            a.hide();
        }
    });
}

unsafe extern "system" fn ctl_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            hook::WM_HK_SHOW => {
                // The arm request was posted first, so it has been handled. If it failed,
                // Windows is locking: stay hidden.
                let armed = hook::ARMED.load(Relaxed);
                diag!("ctl   show request (win+l={}, armed={armed})", wp.0);
                if wp.0 == 0 || armed {
                    with(|a| a.begin_show());
                }
            }
            hook::WM_HK_HIDE => {
                diag!(
                    "ctl   hide request ({})",
                    ["shortcut again", "other shortcut", "click outside"].get(wp.0).unwrap_or(&"?")
                );
                with(|a| a.hide());
            }
            hook::WM_HK_KEY => _ = with(|a| a.key(wp.0 as u32, lp.0)),
            hook::WM_HK_CHAR => _ = with(|a| a.typed(wp.0 as u32)),
            hook::WM_HK_MODS => _ = with(|a| a.mods(wp.0)),
            hook::WM_HK_WHEEL => _ = with(|a| a.wheel(wp.0, lp)),
            hook::WM_HK_REC => _ = with(|a| a.recorded(wp.0 as u32, lp.0 as u8)),
            hook::WM_HK_ARM => {
                let t = std::time::Instant::now();
                let ok = lockpolicy::arm();
                hook::arm_done(ok);
                diag!("ctl   armed={ok} in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
                if ok {
                    SetTimer(Some(hwnd), T_IDLE, 5000, None);
                }
            }
            hook::WM_HK_LOCK => {
                with(|a| a.hide());
                lockpolicy::lock_now();
            }
            WM_CARET => {
                let info = caret::take_result(lp);
                with(|a| a.caret_done(wp.0, info));
            }
            WM_TIMER => match wp.0 {
                T_CARET => {
                    let _ = KillTimer(Some(hwnd), T_CARET);
                    diag!("ctl   caret timeout");
                    with(|a| {
                        if a.pending {
                            a.show_at(None)
                        }
                    });
                }
                T_IDLE => idle_check(hwnd),
                T_SAVED => {
                    let _ = KillTimer(Some(hwnd), T_SAVED);
                    with(|a| a.hide());
                }
                T_THEME => {
                    let _ = KillTimer(Some(hwnd), T_THEME);
                    with(|a| {
                        // The style (and so the size) only changes from the tray menu or on
                        // the next open, so an open picker can simply be redrawn.
                        a.apply_settings();
                        a.redraw();
                    });
                }
                _ => {}
            },
            WM_TRAY => match (lp.0 & 0xFFFF) as u32 {
                WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT => {
                    show_menu(hwnd, loword(wp.0 as isize), hiword(wp.0 as isize))
                }
                NIN_BALLOONUSERCLICK => {
                    let action = with(|a| std::mem::replace(&mut a.balloon_action, BalloonAction::None));
                    if action == Some(BalloonAction::SetupWinL) {
                        setup_winl(hwnd);
                    }
                }
                _ => {}
            },
            WM_INSTANCE => match wp.0 {
                CMD_QUIT => {
                    let _ = DestroyWindow(hwnd);
                }
                CMD_RECHECK => {
                    _ = with(|a| {
                        a.update_winl();
                        a.tray_update();
                        if a.winl == WinL::Ready {
                            a.balloon(
                                "Win+L now opens Charpick",
                                "Lock the PC with Ctrl+Win+L or from the tray menu.",
                                BalloonAction::None,
                            );
                        }
                    })
                }
                CMD_DEMO => _ = with(|a| a.begin_show()),
                CMD_SHORTCUT => _ = with(|a| a.record_shortcut()),
                _ => _ = with(|a| a.greet(true)),
            },
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                if msg == WM_SETTINGCHANGE && lp.0 != 0 {
                    diag!("ctl   setting change \"{}\"", PCWSTR(lp.0 as *const u16).to_string().unwrap_or_default());
                } else {
                    diag!("ctl   theme message {msg:#x}");
                }
                // These arrive in bursts: apply once things settle.
                SetTimer(Some(hwnd), T_THEME, 300, None);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            WM_DISPLAYCHANGE => {
                diag!("ctl   display change");
                with(|a| a.hide());
            }
            WM_INPUT => {
                log_raw_input(lp);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            WM_WTSSESSION_CHANGE => {
                diag!("ctl   session change {}", wp.0);
                // Lock, disconnect or log off: Win+L must lock again from here on.
                if matches!(
                    wp.0 as u32,
                    WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT | WTS_SESSION_LOGOFF
                ) {
                    with(|a| a.hide());
                    disarm_policy(hwnd);
                }
            }
            WM_POWERBROADCAST => {
                diag!("ctl   power event {}", wp.0);
                match wp.0 as u32 {
                    PBT_APMSUSPEND => {
                        with(|a| a.hide());
                        disarm_policy(hwnd);
                    }
                    PBT_POWERSETTINGCHANGE if lp.0 != 0 => {
                        let ps = &*(lp.0 as *const POWERBROADCAST_SETTING);
                        if ps.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && ps.DataLength >= 1 && ps.Data[0] == 0 {
                            disarm_policy(hwnd);
                        }
                    }
                    _ => {}
                }
                return LRESULT(1);
            }
            WM_QUERYENDSESSION => return LRESULT(1),
            WM_ENDSESSION => {
                if wp.0 != 0 {
                    with(|a| a.shutdown());
                    // Restart Manager (an installer) asks to close: exit instead of waiting to be killed.
                    if lp.0 as u32 & ENDSESSION_CLOSEAPP != 0 {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            WM_DESTROY => {
                with(|a| a.shutdown());
                PostQuitMessage(0);
            }
            _ => {
                let taskbar = with(|a| a.taskbar_created).unwrap_or(0);
                if taskbar != 0 && msg == taskbar {
                    // Explorer restarted: the tray icon is gone.
                    with(|a| a.tray_add());
                    return LRESULT(0);
                }
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
        }
        LRESULT(0)
    }
}

unsafe extern "system" fn picker_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // The whole window is client area, painted by Direct2D.
            WM_NCCALCSIZE if wp.0 != 0 => LRESULT(0),
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            // Always look active so that the system backdrop does not fall back to a flat color.
            WM_NCACTIVATE => DefWindowProcW(hwnd, msg, WPARAM(1), LPARAM(-1)),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                with(|a| {
                    if a.visible {
                        a.gfx.render(&a.picker)
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                with(|a| a.mouse(msg, lp));
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|a| a.mouse_leave());
                LRESULT(0)
            }
            WM_TIMER => {
                with(|a| a.picker_timer(wp.0));
                LRESULT(0)
            }
            WM_DPICHANGED => {
                // The picker is placed and sized for its monitor before it is shown.
                let dpi = (wp.0 & 0xFFFF) as u32;
                with(|a| a.picker.set_dpi(dpi));
                LRESULT(0)
            }
            WM_CLOSE => {
                with(|a| a.hide());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// ---------------------------------------------------------------------------

/// Let the tray menu follow the dark theme (undocumented uxtheme ordinals 135/136).
fn allow_dark_menus() {
    if util::os_build() < 18362 {
        return;
    }
    unsafe {
        let Ok(ux) = windows::Win32::System::LibraryLoader::LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(ux, windows_core::PCSTR(135 as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            // 1 = AllowDark: follow the system setting.
            set_preferred_app_mode(1);
        }
        if let Some(f) = GetProcAddress(ux, windows_core::PCSTR(136 as *const u8)) {
            let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
            flush_menu_themes();
        }
    }
}

pub fn run(opts: Options) -> i32 {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let icon = LoadIconW(Some(hinst), PCWSTR(std::ptr::without_provenance(1))).unwrap_or_default();
        let ctl_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(ctl_proc),
            hInstance: hinst,
            hIcon: icon,
            lpszClassName: CONTROLLER_CLASS,
            ..Default::default()
        };
        let picker_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(picker_proc),
            hInstance: hinst,
            hIcon: icon,
            hCursor: cursor,
            lpszClassName: PICKER_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&ctl_class);
        RegisterClassExW(&picker_class);

        let Ok(ctl) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CONTROLLER_CLASS,
            w!("Charpick"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        ) else {
            util::error_box(None, "Charpick could not start (controller window).");
            return 1;
        };
        let Ok(win) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            PICKER_CLASS,
            w!("Charpick"),
            WS_POPUP | WS_THICKFRAME,
            0,
            0,
            360,
            450,
            None,
            None,
            Some(hinst),
            None,
        ) else {
            util::error_box(None, "Charpick could not start (picker window).");
            return 1;
        };

        allow_dark_menus();

        let mut settings = Settings::load();
        opts.apply(&mut settings);
        let gfx = match Gfx::new(win) {
            Ok(g) => g,
            Err(e) => {
                util::error_box(None, &format!("Charpick could not start Direct2D.\n\n{}", e.message()));
                return 1;
            }
        };
        let picker = Picker::new(Catalog::build(), &settings, resolve_theme(&settings, &gfx));

        // A previous run that crashed may have left Win+L disabled.
        lockpolicy::disarm();

        let welcome = opts.welcome;
        let greet = opts.welcome || !opts.autostart;
        let demo = opts.demo;
        let record = opts.change_shortcut;
        let app = App {
            ctl,
            win,
            hinst,
            gfx,
            picker,
            settings,
            settings_time: Settings::modified(),
            opts,
            recent: Recent::load(),
            caret: CaretWorker::start(ctl),
            req: 0,
            pending: false,
            visible: false,
            recording: false,
            target: HWND::default(),
            rect: RECT::default(),
            shift: false,
            caps: false,
            winl: WinL::Off,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            tray_icon: HICON::default(),
            balloon_action: BalloonAction::None,
            fg_hook: None,
            tracking: false,
            anim_timer: false,
        };
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        if diag::enabled() {
            // Raw input shows what the keyboard sent, even keys Windows keeps from the hook.
            let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: ctl };
            let raw = RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_ok();
            diag!(
                "start Charpick {} on Windows build {}, remote session={}, raw input={raw}",
                env!("CARGO_PKG_VERSION"),
                util::os_build(),
                GetSystemMetrics(SM_REMOTESESSION) != 0
            );
        }
        hook::start(ctl);
        with(|a| {
            apply_window_style(a.win, &a.picker.theme);
            hook::set_hotkeys(a.settings.hotkey, a.settings.lock_hotkey);
            a.update_winl();
            diag!("start hotkey {}, Win+L {:?}", a.settings.hotkey.label(), a.winl);
            a.tray_add();
            // Create the Direct2D resources now so that the first open is instant.
            a.gfx.prepare(&a.picker);
            if greet {
                a.greet(welcome);
            }
        });
        let _ = WTSRegisterSessionNotification(ctl, NOTIFY_FOR_THIS_SESSION);
        let _ = RegisterPowerSettingNotification(
            windows::Win32::Foundation::HANDLE(ctl.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        if demo {
            let _ = PostMessageW(Some(ctl), WM_INSTANCE, WPARAM(CMD_DEMO), LPARAM(0));
        } else if record {
            let _ = PostMessageW(Some(ctl), WM_INSTANCE, WPARAM(CMD_SHORTCUT), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }

        hook::stop();
        APP.with(|cell| cell.borrow_mut().take());
        0
    }
}
