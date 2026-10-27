//! Finds the text caret of the focused application.
//!
//! Runs on its own COM (MTA) thread because the accessibility calls are cross
//! process and can be slow or hang in a misbehaving app. The result is posted
//! back to the controller window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation, IAccessible, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, IUIAutomationTextRange, TextPatternRangeEndpoint_Start, TextUnit_Character,
    UIA_TextPattern2Id, UIA_TextPatternId,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, OBJID_CARET, PostMessageW,
};
use windows_core::Interface;

pub const WM_CARET: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 20;

#[derive(Clone, Copy, Debug)]
pub struct CaretInfo {
    /// Caret rectangle in physical screen pixels.
    pub rect: RECT,
}

struct Request {
    id: usize,
    foreground: HWND,
}

unsafe impl Send for Request {}

pub struct CaretWorker {
    tx: Sender<Request>,
    busy: Arc<AtomicBool>,
}

impl CaretWorker {
    pub fn start(notify: HWND) -> CaretWorker {
        let (tx, rx) = channel::<Request>();
        let busy = Arc::new(AtomicBool::new(false));
        let busy2 = busy.clone();
        let notify = notify.0 as isize;
        std::thread::Builder::new()
            .name("caret".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                let mut uia: Option<IUIAutomation> = None;
                while let Ok(req) = rx.recv() {
                    let info = find_caret(req.foreground, &mut uia);
                    busy2.store(false, Relaxed);
                    let boxed = info.map(|i| Box::into_raw(Box::new(i)));
                    let lp = boxed.map_or(0, |p| p as isize);
                    unsafe {
                        if PostMessageW(Some(HWND(notify as *mut _)), WM_CARET, WPARAM(req.id), LPARAM(lp)).is_err()
                            && let Some(p) = boxed
                        {
                            drop(Box::from_raw(p));
                        }
                    }
                }
            })
            .expect("caret thread");
        CaretWorker { tx, busy }
    }

    /// Ask for the caret position. Returns false when the worker is still stuck
    /// on an earlier request.
    pub fn request(&self, id: usize, foreground: HWND) -> bool {
        if self.busy.swap(true, Relaxed) {
            return false;
        }
        self.tx.send(Request { id, foreground }).is_ok()
    }
}

/// Take ownership of the result posted with `WM_CARET`.
pub fn take_result(lp: LPARAM) -> Option<CaretInfo> {
    (lp.0 != 0).then(|| unsafe { *Box::from_raw(lp.0 as *mut CaretInfo) })
}

fn valid(r: &RECT) -> bool {
    // Some apps report a caret at the origin when they have none.
    (r.right > r.left || r.bottom > r.top) && !(r.left == 0 && r.top == 0)
}

fn find_caret(fg: HWND, uia: &mut Option<IUIAutomation>) -> Option<CaretInfo> {
    if fg.is_invalid() {
        return None;
    }
    unsafe {
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gti = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        let have_gti = GetGUIThreadInfo(tid, &mut gti).is_ok();

        // 1. The classic system caret (Win32 edit controls, Notepad, many classic apps).
        if have_gti && !gti.hwndCaret.is_invalid() {
            let rc = gti.rcCaret;
            if rc.bottom > rc.top {
                let r = client_rect_to_screen(gti.hwndCaret, rc);
                if valid(&r) {
                    return Some(CaretInfo { rect: r });
                }
            }
        }

        // 2. MSAA caret object (Chromium, Firefox, Electron apps).
        let focus = if have_gti && !gti.hwndFocus.is_invalid() { gti.hwndFocus } else { fg };
        if let Some(r) = msaa_caret(focus) {
            return Some(CaretInfo { rect: r });
        }

        // 3. UI Automation text pattern (UWP, WinUI, WPF, Office, Windows Terminal).
        if uia.is_none() {
            *uia = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
            if let Some(a) = uia.as_ref().and_then(|a| a.cast::<IUIAutomation2>().ok()) {
                let _ = a.SetConnectionTimeout(1500);
                let _ = a.SetTransactionTimeout(400);
            }
        }
        if let Some(a) = uia.as_ref()
            && let Some(r) = uia_caret(a)
        {
            return Some(CaretInfo { rect: r });
        }
    }
    None
}

/// Convert a caret rectangle in `hwnd`'s client coordinates to physical screen
/// pixels, accounting for DPI virtualized (unaware or system aware) windows.
unsafe fn client_rect_to_screen(hwnd: HWND, rc: RECT) -> RECT {
    unsafe {
        let mut origin = POINT::default();
        let _ = ClientToScreen(hwnd, &mut origin);
        let mon = MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let wdpi = GetDpiForWindow(hwnd).max(96);
        let s = dx as f32 / wdpi as f32;
        let f = |v: i32| (v as f32 * s).round() as i32;
        RECT {
            left: origin.x + f(rc.left),
            top: origin.y + f(rc.top),
            right: origin.x + f(rc.right),
            bottom: origin.y + f(rc.bottom),
        }
    }
}

unsafe fn msaa_caret(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut ptr = std::ptr::null_mut();
        AccessibleObjectFromWindow(hwnd, OBJID_CARET.0 as u32, &IAccessible::IID, &mut ptr).ok()?;
        if ptr.is_null() {
            return None;
        }
        let acc = IAccessible::from_raw(ptr);
        let child = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    Anonymous: VARIANT_0_0_0 { lVal: 0 },
                    ..Default::default()
                }),
            },
        };
        let (mut l, mut t, mut w, mut h) = (0, 0, 0, 0);
        acc.accLocation(&mut l, &mut t, &mut w, &mut h, &child).ok()?;
        let r = RECT { left: l, top: t, right: l + w.max(1), bottom: t + h };
        (h > 0 && valid(&r)).then_some(r)
    }
}

unsafe fn rects_of(range: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        let sa: *mut SAFEARRAY = range.GetBoundingRectangles().ok()?;
        if sa.is_null() {
            return None;
        }
        let mut out = None;
        let lo = SafeArrayGetLBound(sa, 1).unwrap_or(0);
        let hi = SafeArrayGetUBound(sa, 1).unwrap_or(-1);
        let n = (hi - lo + 1).max(0) as usize;
        let mut data = std::ptr::null_mut();
        if n >= 4 && SafeArrayAccessData(sa, &mut data).is_ok() {
            let v = std::slice::from_raw_parts(data as *const f64, n);
            // Use the last rectangle: for a multi-line selection the caret is at its end.
            let base = (n / 4 - 1) * 4;
            let (x, y, w, h) = (v[base], v[base + 1], v[base + 2], v[base + 3]);
            if h > 0.0 {
                out = Some(RECT {
                    left: x as i32,
                    top: y as i32,
                    right: (x + w.max(1.0)) as i32,
                    bottom: (y + h) as i32,
                });
            }
            let _ = SafeArrayUnaccessData(sa);
        }
        let _ = SafeArrayDestroy(sa);
        out
    }
}

/// Bounding box of a degenerate (empty) range: measure the character before it.
unsafe fn degenerate_caret(range: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        if let Some(r) = rects_of(range) {
            return Some(r);
        }
        let probe = range.Clone().ok()?;
        if probe.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()? != 0
            && let Some(r) = rects_of(&probe)
        {
            // The caret sits after that character.
            return Some(RECT { left: r.right, top: r.top, right: r.right + 1, bottom: r.bottom });
        }
        let probe = range.Clone().ok()?;
        probe.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
        rects_of(&probe).map(|r| RECT { right: r.left + 1, ..r })
    }
}

unsafe fn uia_caret(uia: &IUIAutomation) -> Option<RECT> {
    unsafe {
        let el = uia.GetFocusedElement().ok()?;
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
            let mut active = windows_core::BOOL(0);
            if let Ok(range) = p.GetCaretRange(&mut active)
                && let Some(r) = degenerate_caret(&range)
            {
                return Some(r);
            }
        }
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            && let Ok(sel) = p.GetSelection()
            && sel.Length().unwrap_or(0) > 0
            && let Ok(range) = sel.GetElement(0)
            && let Some(r) = degenerate_caret(&range)
        {
            return Some(r);
        }
        // A focused element without text support (a search box in a custom UI):
        // anchor at its bottom-left corner when it is reasonably small.
        let r = el.CurrentBoundingRectangle().ok()?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        (w > 0 && h > 0 && h < 120 && w < 2000).then(|| RECT {
            left: r.left + 4,
            top: r.top,
            right: r.left + 5,
            bottom: r.bottom,
        })
    }
}
