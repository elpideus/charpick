//! Low-level keyboard and mouse hooks, running on their own thread so that the
//! UI never delays keystrokes.
//!
//! While the picker is hidden the keyboard hook only looks for the hotkeys. While
//! it is visible, the picker never takes focus (like the emoji panel), so the hook
//! routes navigation and typing to it and keeps them away from the focused app.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering::Relaxed};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, ToUnicodeEx, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HC_ACTION, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
    WM_NCLBUTTONDOWN, WM_NCRBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
};

use crate::diag::{self, diag};
use crate::settings::{Hotkey, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};

/// `dwExtraInfo` of input we inject ourselves, so the hook lets it through.
pub const MAGIC: usize = 0x4348_5250;

// Messages posted to the controller window.
/// wParam: 1 when opened with the taken-over Win+L, which needs the lock policy set.
pub const WM_HK_SHOW: u32 = WM_APP + 1;
/// wParam: one of the `HIDE_*` reasons.
pub const WM_HK_HIDE: u32 = WM_APP + 2;
/// wParam: virtual key, lParam: `KEY_*` flags.
pub const WM_HK_KEY: u32 = WM_APP + 3;
/// wParam: a Unicode scalar value typed into the search.
pub const WM_HK_CHAR: u32 = WM_APP + 4;
/// wParam: bit 0 = Shift is down, bit 1 = Caps Lock was pressed.
pub const WM_HK_MODS: u32 = WM_APP + 5;
/// wParam: low word = wheel delta, bit 16 = horizontal. lParam: screen point.
pub const WM_HK_WHEEL: u32 = WM_APP + 6;
pub const WM_HK_ARM: u32 = WM_APP + 7;
pub const WM_HK_LOCK: u32 = WM_APP + 8;
/// While recording a shortcut. wParam: virtual key, 0 when only the modifiers
/// changed. lParam: the `MOD_*` bits held.
pub const WM_HK_REC: u32 = WM_APP + 9;

pub const HIDE_TOGGLE: usize = 0;
pub const HIDE_SHORTCUT: usize = 1;
pub const HIDE_CLICK_OUTSIDE: usize = 2;

pub const KEY_SHIFT: isize = 1;
pub const KEY_CTRL: isize = 2;

// Thread messages understood by the hook thread.
const WM_MOUSE_HOOK_ON: u32 = WM_APP + 40;
const WM_MOUSE_HOOK_OFF: u32 = WM_APP + 41;

/// The picker is on screen.
pub static VISIBLE: AtomicBool = AtomicBool::new(false);
/// The picker is recording a new shortcut: every key goes to it.
pub static RECORDING: AtomicBool = AtomicBool::new(false);
pub static HOTKEY: AtomicU32 = AtomicU32::new(0);
pub static LOCK_HOTKEY: AtomicU32 = AtomicU32::new(0);
/// Win+L may be taken over (the hotkey is Win+L and the lock policy is writable).
pub static WINL_ENABLED: AtomicBool = AtomicBool::new(false);
/// The lock policy is currently set, so Windows will not lock on Win+L.
pub static ARMED: AtomicBool = AtomicBool::new(false);
static ARM_PENDING: AtomicBool = AtomicBool::new(false);
static NOTIFY: AtomicIsize = AtomicIsize::new(0);
static THREAD: AtomicU32 = AtomicU32::new(0);
static RECT_L: AtomicI32 = AtomicI32::new(0);
static RECT_T: AtomicI32 = AtomicI32::new(0);
static RECT_R: AtomicI32 = AtomicI32::new(0);
static RECT_B: AtomicI32 = AtomicI32::new(0);

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_CAPITAL: u32 = 0x14;
const VK_ESCAPE: u32 = 0x1B;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2E;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_PACKET: u32 = 0xE7;
/// Unassigned key injected to stop the Start menu from opening on Win release.
const VK_MASK: u16 = 0xE8;
/// How long a key-down seen by the hook counts as held without a newer one. A held
/// key autorepeats at least once a second (the longest repeat delay), so this only
/// expires for a key-up the hook never saw, on the lock screen for example.
const HELD_MS: u32 = 1500;

#[derive(Default)]
struct State {
    /// Key-ups to swallow because the matching key-down was swallowed.
    swallow_up: Vec<bool>,
    /// Keys whose key-down went through the hook and that are not released yet.
    seen_down: Vec<bool>,
    /// Time of each key's last key-down (or autorepeat) the hook saw.
    down_time: Vec<u32>,
    /// Modifiers held down since a hotkey fired. They are ignored until released,
    /// so a still-held Win does not turn the next arrow key into a Win+Arrow.
    sticky: u8,
    dead: Option<char>,
    mouse_hook: Option<HHOOK>,
}

thread_local! {
    static STATE: RefCell<State> =
        RefCell::new(State { swallow_up: vec![false; 256], seen_down: vec![false; 256], down_time: vec![0; 256], ..Default::default() });
}

fn post(msg: u32, wp: usize, lp: isize) {
    let hwnd = HWND(NOTIFY.load(Relaxed) as *mut _);
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wp), LPARAM(lp));
    }
}

fn down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

fn modifiers() -> u8 {
    let mut m = 0;
    if down(VK_CONTROL) {
        m |= MOD_CTRL;
    }
    if down(VK_MENU) {
        m |= MOD_ALT;
    }
    if down(VK_SHIFT) {
        m |= MOD_SHIFT;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m |= MOD_WIN;
    }
    m
}

/// Whether the hook saw `vk` go down and not up.
fn held(st: &State, vk: u32, time: u32) -> bool {
    st.seen_down[vk as usize] && time.wrapping_sub(st.down_time[vk as usize]) <= HELD_MS
}

/// `MOD_WIN` when the hook saw Win go down and not up. Windows may not know yet:
/// some keyboard tools hold the Win key back and send it only just before the next
/// key, after this hook has already looked at that key.
fn seen_win(st: &State, time: u32) -> u8 {
    if held(st, VK_LWIN, time) || held(st, VK_RWIN, time) { MOD_WIN } else { 0 }
}

/// The modifiers the hook saw go down and not up. Unlike `modifiers`, this is
/// right for a modifier event the hook is looking at right now.
fn held_mods(st: &State, time: u32) -> u8 {
    let mut m = 0;
    for (left, right, bit) in
        [(VK_LCONTROL, VK_RCONTROL, MOD_CTRL), (VK_LMENU, VK_RMENU, MOD_ALT), (VK_LSHIFT, VK_RSHIFT, MOD_SHIFT), (VK_LWIN, VK_RWIN, MOD_WIN)]
    {
        if held(st, left, time) || held(st, right, time) {
            m |= bit;
        }
    }
    m
}

fn modifier_bit(vk: u32) -> u8 {
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => MOD_SHIFT,
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => MOD_CTRL,
        VK_MENU | VK_LMENU | VK_RMENU => MOD_ALT,
        VK_LWIN | VK_RWIN => MOD_WIN,
        _ => 0,
    }
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: MAGIC } },
    }
}

/// Inject a do-nothing key so that releasing Win does not open the Start menu.
fn inject_mask() {
    let inputs = [key_input(VK_MASK, 0, KEYBD_EVENT_FLAGS(0)), key_input(VK_MASK, 0, KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Type `text` into the focused window.
pub fn send_text(text: &str) {
    let mut inputs = Vec::with_capacity(text.len() * 4);
    for unit in text.encode_utf16() {
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Map a spacing dead-key character to its combining mark.
fn combining_for(dead: char) -> Option<char> {
    Some(match dead {
        '`' => '\u{0300}',
        '´' | '\'' => '\u{0301}',
        '^' | 'ˆ' => '\u{0302}',
        '~' | '˜' => '\u{0303}',
        '¯' => '\u{0304}',
        '˘' => '\u{0306}',
        '˙' => '\u{0307}',
        '¨' | '"' => '\u{0308}',
        '°' | '˚' => '\u{030A}',
        '˝' => '\u{030B}',
        'ˇ' => '\u{030C}',
        '¸' => '\u{0327}',
        '˛' => '\u{0328}',
        _ => return None,
    })
}

enum Typed {
    Text(String),
    Dead,
    Nothing,
}

fn translate(vk: u32, scan: u32, st: &mut State) -> Typed {
    let mut ks = [0u8; 256];
    for k in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU] {
        if down(k) {
            ks[k as usize] = 0x80;
        }
    }
    ks[VK_CAPITAL as usize] = unsafe { GetKeyState(VK_CAPITAL as i32) as u8 & 1 };
    let mut buf = [0u16; 8];
    let n = unsafe {
        let tid = GetWindowThreadProcessId(GetForegroundWindow(), None);
        // Flag 4: do not touch the keyboard state, so the focused app's dead keys keep working.
        ToUnicodeEx(vk, scan, &ks, &mut buf, 4, Some(GetKeyboardLayout(tid)))
    };
    if n < 0 {
        st.dead = String::from_utf16_lossy(&buf[..1]).chars().next();
        return Typed::Dead;
    }
    let s: String = String::from_utf16_lossy(&buf[..n as usize]).chars().filter(|c| !c.is_control()).collect();
    if s.is_empty() {
        st.dead = None;
        return Typed::Nothing;
    }
    let Some(dead) = st.dead.take() else { return Typed::Text(s) };
    if s == " " {
        return Typed::Text(dead.to_string());
    }
    if let Some(mark) = combining_for(dead) {
        let mut chars = s.chars();
        if let (Some(base), None) = (chars.next(), chars.clone().next()) {
            let composed = crate::util::nfc(&format!("{base}{mark}"));
            if composed.chars().count() == 1 {
                return Typed::Text(composed);
            }
        }
    }
    Typed::Text(format!("{dead}{s}"))
}

/// Decide what to do with a key event. Returns true to swallow it.
fn on_key(k: &KBDLLHOOKSTRUCT, msg: u32, st: &mut State) -> bool {
    let vk = k.vkCode;
    if vk > 0xFF {
        return false;
    }
    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let visible = VISIBLE.load(Relaxed);
    let modbit = modifier_bit(vk);
    let hk = Hotkey::unpack(HOTKEY.load(Relaxed));
    let lk = Hotkey::unpack(LOCK_HOTKEY.load(Relaxed));
    // A key-down of a key that is already held is an autorepeat.
    let repeat = held(st, vk, k.time);
    st.seen_down[vk as usize] = is_down;
    if is_down {
        st.down_time[vk as usize] = k.time;
        if !repeat {
            // Only set here when the last key-up went to the lock screen.
            st.swallow_up[vk as usize] = false;
        }
    }
    diag!(
        "hook  {:<4} {:<9} {}t={} | win: async={} seen={} | armed={} pending={} visible={}",
        if is_down { "down" } else { "up" },
        diag::key_name(vk, &[hk.vk, lk.vk]),
        if k.flags.0 & LLKHF_INJECTED.0 != 0 { "injected " } else { "" },
        k.time,
        down(VK_LWIN) || down(VK_RWIN),
        seen_win(st, k.time) != 0,
        ARMED.load(Relaxed),
        ARM_PENDING.load(Relaxed),
        visible,
    );

    if RECORDING.load(Relaxed) {
        return record_key(vk, is_down, repeat, modbit, k.time, st);
    }

    // Win+L is ours once the policy is set, or while it is being set: a quick Win+L
    // can arrive before the controller has finished writing it.
    let takeover = hk == Hotkey::WIN_L && WINL_ENABLED.load(Relaxed);
    let available = hk != Hotkey::WIN_L || ARMED.load(Relaxed) || (takeover && ARM_PENDING.load(Relaxed));

    if !is_down {
        if modbit != 0 {
            st.sticky &= !modbit;
            if visible && modbit == MOD_SHIFT {
                post_mods(vk, false, false);
            }
        }
        return std::mem::replace(&mut st.swallow_up[vk as usize], false);
    }

    if modbit != 0 {
        if modbit == MOD_WIN && WINL_ENABLED.load(Relaxed) && !ARMED.load(Relaxed) && !ARM_PENDING.swap(true, Relaxed) {
            // Set the lock policy as soon as Win goes down, before L follows.
            diag!("hook  arm requested");
            post(WM_HK_ARM, 0, 0);
        }
        if visible && modbit == MOD_SHIFT {
            post_mods(vk, true, false);
        }
        return false;
    }
    if vk == VK_CAPITAL {
        if visible {
            post_mods(vk, down(VK_SHIFT), true);
        }
        return false;
    }

    let win_held_back = seen_win(st, k.time) != 0 && !(down(VK_LWIN) || down(VK_RWIN));
    let mods = modifiers() | seen_win(st, k.time);

    if !hk.is_none() && vk == hk.vk as u32 && mods == hk.mods && available {
        if repeat {
            return true;
        }
        diag!("hook  shortcut: {}{}", if visible { "close" } else { "open" }, if win_held_back { " (Win held back)" } else { "" });
        // When another program holds Win back, let L go on so that it sends Win now.
        // Windows then sees Win+L, which the lock policy turns into nothing. Keeping
        // the L could leave Win held back until release, and that program may then
        // replay it as a Win tap, which opens the Start menu.
        let pass = win_held_back && hk == Hotkey::WIN_L && ARMED.load(Relaxed);
        if !pass {
            st.swallow_up[vk as usize] = true;
            if mods & MOD_WIN != 0 {
                inject_mask();
            }
        }
        st.sticky = mods;
        if visible {
            post(WM_HK_HIDE, HIDE_TOGGLE, 0);
        } else {
            post(WM_HK_SHOW, takeover as usize, 0);
        }
        return !pass;
    }
    if !hk.is_none() && vk == hk.vk as u32 && !repeat {
        diag!("hook  shortcut key not taken: mods={mods:#x} (want {:#x}) available={available}", hk.mods);
    }
    if !lk.is_none() && vk == lk.vk as u32 && mods == lk.mods && WINL_ENABLED.load(Relaxed) {
        if !repeat {
            diag!("hook  lock shortcut");
            st.swallow_up[vk as usize] = true;
            if mods & MOD_WIN != 0 {
                inject_mask();
            }
            post(WM_HK_LOCK, 0, 0);
        }
        return true;
    }
    if !visible {
        return false;
    }

    // The picker is open: route the key to it.
    let active = mods & !st.sticky;
    let ctrl = active & MOD_CTRL != 0;
    let alt = active & MOD_ALT != 0;
    let altgr = mods & MOD_CTRL != 0 && down(VK_RMENU);
    if active & MOD_WIN != 0 || (alt && !altgr) || (ctrl && !altgr && vk != VK_BACK) {
        // A shortcut meant for something else: get out of the way.
        diag!("hook  other shortcut: closing");
        post(WM_HK_HIDE, HIDE_SHORTCUT, 0);
        return false;
    }
    let mut flags = 0;
    if mods & MOD_SHIFT != 0 {
        flags |= KEY_SHIFT;
    }
    if ctrl {
        flags |= KEY_CTRL;
    }
    match vk {
        VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_TAB | VK_RETURN | VK_ESCAPE
        | VK_BACK | VK_DELETE => {
            st.swallow_up[vk as usize] = true;
            post(WM_HK_KEY, vk as usize, flags);
            return true;
        }
        VK_PACKET => {
            // Unicode input injected by touch keyboards and similar tools.
            st.swallow_up[vk as usize] = true;
            if let Some(c) = char::from_u32(k.scanCode) {
                post(WM_HK_CHAR, c as usize, 0);
            }
            return true;
        }
        _ => {}
    }
    match translate(vk, k.scanCode, st) {
        Typed::Text(s) => {
            for c in s.chars() {
                post(WM_HK_CHAR, c as usize, 0);
            }
            st.swallow_up[vk as usize] = true;
            true
        }
        Typed::Dead => {
            st.swallow_up[vk as usize] = true;
            true
        }
        // Function, media and other non-text keys keep working normally.
        Typed::Nothing => false,
    }
}

/// Report a key to the shortcut recorder and keep it from the focused app.
/// Modifiers pass, so that Windows keeps track of them.
fn record_key(vk: u32, is_down: bool, repeat: bool, modbit: u8, time: u32, st: &mut State) -> bool {
    if modbit != 0 {
        if !(is_down && repeat) {
            post(WM_HK_REC, 0, held_mods(st, time) as isize);
        }
        return false;
    }
    if !is_down {
        return std::mem::replace(&mut st.swallow_up[vk as usize], false);
    }
    let mods = held_mods(st, time);
    st.swallow_up[vk as usize] = true;
    if mods & MOD_WIN != 0 {
        inject_mask();
    }
    if !repeat {
        post(WM_HK_REC, vk as usize, mods as isize);
    }
    true
}

/// Tell the picker the uppercase state may have changed.
fn post_mods(vk: u32, is_down: bool, caps: bool) {
    let left = if vk == VK_LSHIFT { is_down } else { down(VK_LSHIFT) };
    let right = if vk == VK_RSHIFT { is_down } else { down(VK_RSHIFT) };
    let mut wp = 0;
    if left || right {
        wp |= 1;
    }
    if caps {
        wp |= 2;
    }
    post(WM_HK_MODS, wp, 0);
}

unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if k.dwExtraInfo != MAGIC {
            let swallow = STATE.with(|s| match s.try_borrow_mut() {
                Ok(mut st) => on_key(k, wp.0 as u32, &mut st),
                Err(_) => false,
            });
            if swallow {
                diag!("hook        swallowed");
                return LRESULT(1);
            }
        } else if k.vkCode == VK_MASK as u32 {
            diag!("hook  {} Mask(E8) (Charpick's own)", if wp.0 as u32 == WM_KEYDOWN { "down" } else { "up" });
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && VISIBLE.load(Relaxed) {
        let m = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let r = RECT { left: RECT_L.load(Relaxed), top: RECT_T.load(Relaxed), right: RECT_R.load(Relaxed), bottom: RECT_B.load(Relaxed) };
        let inside = crate::util::rect_contains(&r, m.pt.x, m.pt.y);
        match wp.0 as u32 {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN => {
                if !inside {
                    post(WM_HK_HIDE, HIDE_CLICK_OUTSIDE, 0);
                }
            }
            msg @ (WM_MOUSEWHEEL | WM_MOUSEHWHEEL) if inside => {
                // Route the wheel to the picker whatever the "scroll inactive windows" setting.
                let delta = (m.mouseData >> 16) as u16 as usize;
                let horizontal = if msg == WM_MOUSEHWHEEL { 1 << 16 } else { 0 };
                let pt = (m.pt.x as u16 as u32 | (m.pt.y as u16 as u32) << 16) as i32 as isize;
                post(WM_HK_WHEEL, delta | horizontal, pt);
                return LRESULT(1);
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// Unpack the screen point posted with `WM_HK_WHEEL`.
pub fn wheel_point(lp: LPARAM) -> POINT {
    POINT { x: lp.0 as u16 as i16 as i32, y: (lp.0 >> 16) as u16 as i16 as i32 }
}

/// Start the hook thread. Hook messages are posted to `notify`.
pub fn start(notify: HWND) {
    NOTIFY.store(notify.0 as isize, Relaxed);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hooks".into())
        .stack_size(256 * 1024)
        .spawn(move || unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            let module = GetModuleHandleW(None).ok().map(|m| m.into());
            let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            let _ = tx.send(GetCurrentThreadId());
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    WM_MOUSE_HOOK_ON => STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.mouse_hook.is_none() {
                            st.mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0).ok();
                        }
                    }),
                    WM_MOUSE_HOOK_OFF => STATE.with(|s| {
                        if let Some(h) = s.borrow_mut().mouse_hook.take() {
                            let _ = UnhookWindowsHookEx(h);
                        }
                    }),
                    _ => {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            STATE.with(|s| {
                if let Some(h) = s.borrow_mut().mouse_hook.take() {
                    let _ = UnhookWindowsHookEx(h);
                }
            });
            if let Ok(kb) = kb {
                let _ = UnhookWindowsHookEx(kb);
            }
        })
        .expect("hook thread");
    if let Ok(id) = rx.recv() {
        THREAD.store(id, Relaxed);
    }
}

fn post_thread(msg: u32) {
    let id = THREAD.load(Relaxed);
    if id != 0 {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(id, msg, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn stop() {
    post_thread(windows::Win32::UI::WindowsAndMessaging::WM_QUIT);
}

/// Publish the picker's state and screen rectangle to the hooks.
pub fn set_visible(visible: bool, rect: RECT) {
    RECT_L.store(rect.left, Relaxed);
    RECT_T.store(rect.top, Relaxed);
    RECT_R.store(rect.right, Relaxed);
    RECT_B.store(rect.bottom, Relaxed);
    VISIBLE.store(visible, Relaxed);
    post_thread(if visible { WM_MOUSE_HOOK_ON } else { WM_MOUSE_HOOK_OFF });
}

pub fn set_hotkeys(open: Hotkey, lock: Hotkey) {
    HOTKEY.store(open.pack(), Relaxed);
    LOCK_HOTKEY.store(lock.pack(), Relaxed);
}

/// The controller finished an arm request.
pub fn arm_done(armed: bool) {
    ARMED.store(armed, Relaxed);
    ARM_PENDING.store(false, Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State { swallow_up: vec![false; 256], seen_down: vec![false; 256], down_time: vec![0; 256], ..Default::default() }
    }

    #[test]
    fn held_keys_expire() {
        let mut st = state();
        // Win down at t=8453437, L 469 ms later (from a log where another program held Win back).
        st.seen_down[VK_LWIN as usize] = true;
        st.down_time[VK_LWIN as usize] = 8_453_437;
        assert_eq!(seen_win(&st, 8_453_906), MOD_WIN);
        // A Win key-up that went to the lock screen stops counting.
        assert_eq!(seen_win(&st, 8_453_437 + HELD_MS + 1), 0);
        // The tick count wraps around after 49.7 days.
        st.down_time[VK_LWIN as usize] = u32::MAX - 10;
        assert_eq!(seen_win(&st, 20), MOD_WIN);
        st.seen_down[VK_LWIN as usize] = false;
        assert_eq!(seen_win(&st, 20), 0);
    }
}
