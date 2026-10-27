//! Small helpers shared by every module: Unicode folding and paths, plus (on
//! Windows) wide strings, registry access and Windows version detection.

#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(windows)]
pub const APP_NAME: &str = "Charpick";

/// Canonical decomposition (`è` → `e` + U+0300).
pub fn nfd(s: &str) -> String {
    #[cfg(windows)]
    return win::normalize(s, windows::Win32::Globalization::NormalizationD);
    #[cfg(not(windows))]
    return unicode_normalization::UnicodeNormalization::nfd(s).collect();
}

/// Canonical composition (`e` + U+0300 → `è`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn nfc(s: &str) -> String {
    #[cfg(windows)]
    return win::normalize(s, windows::Win32::Globalization::NormalizationC);
    #[cfg(not(windows))]
    return unicode_normalization::UnicodeNormalization::nfc(s).collect();
}

fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// Lowercase and strip diacritics, so that `Română` and `romana` compare equal.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in nfd(s).chars() {
        if is_mark(c) {
            continue;
        }
        match c {
            'ß' | 'ẞ' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            'ø' | 'Ø' => out.push('o'),
            'ł' | 'Ł' => out.push('l'),
            'đ' | 'Đ' | 'ð' | 'Ð' => out.push('d'),
            'ħ' | 'Ħ' => out.push('h'),
            'ı' => out.push('i'),
            'þ' | 'Þ' => out.push_str("th"),
            'ĳ' | 'Ĳ' => out.push_str("ij"),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

#[cfg(not(windows))]
pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("charpick"))
}

/// `$XDG_CONFIG_HOME/charpick` (`~/.config/charpick`), created on demand.
#[cfg(not(windows))]
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("charpick");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::path::PathBuf;

    use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, POINT, RECT};
    use windows::Win32::Globalization::NormalizeString;
    use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, REG_DWORD, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY, RRF_RT_REG_DWORD, RegDeleteKeyValueW,
        RegGetValueW, RegSetKeyValueW,
    };
    use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MessageBoxW,
    };
    use windows_core::{PCWSTR, s, w};

    use crate::gfx::Color;
    use crate::theme::{Kind, SystemLook, rgba};

    use super::APP_NAME;

    pub const APP_KEY: &str = r"Software\Charpick";

    /// Null terminated UTF-16 copy of `s`.
    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn pcwstr(v: &[u16]) -> PCWSTR {
        PCWSTR(v.as_ptr())
    }

    /// Copy `s` into a fixed size UTF-16 buffer, truncating and null terminating.
    pub fn copy_wide(dst: &mut [u16], s: &str) {
        let mut n = 0;
        for (d, c) in dst.iter_mut().zip(s.encode_utf16()) {
            *d = c;
            n += 1;
        }
        let n = n.min(dst.len() - 1);
        dst[n] = 0;
    }

    pub(super) fn normalize(s: &str, form: windows::Win32::Globalization::NORM_FORM) -> String {
        let src: Vec<u16> = s.encode_utf16().collect();
        if src.is_empty() {
            return String::new();
        }
        unsafe {
            let mut len = NormalizeString(form, &src, None);
            for _ in 0..4 {
                if len <= 0 {
                    break;
                }
                let mut buf = vec![0u16; len as usize];
                let n = NormalizeString(form, &src, Some(&mut buf));
                if n > 0 {
                    buf.truncate(n as usize);
                    return String::from_utf16_lossy(&buf);
                }
                // Buffer estimate was too small: the negative result is the new estimate.
                len = -n;
            }
        }
        s.to_string()
    }

    pub fn exe_path() -> PathBuf {
        let mut buf = vec![0u16; 1024];
        loop {
            let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
            if n < buf.len() {
                return PathBuf::from(String::from_utf16_lossy(&buf[..n]));
            }
            buf.resize(buf.len() * 2, 0);
        }
    }

    /// `%APPDATA%\Charpick`, created on demand.
    pub fn data_dir() -> PathBuf {
        let base = unsafe {
            match SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) {
                Ok(p) => {
                    let s = p.to_string().unwrap_or_default();
                    windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const c_void));
                    PathBuf::from(s)
                }
                Err(_) => std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir),
            }
        };
        let dir = base.join(APP_NAME);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    // ---------------------------------------------------------------------------
    // Registry

    pub fn reg_get_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let (k, v) = (wide(key), wide(value));
        let mut data = 0u32;
        let mut size = 4u32;
        let r = unsafe {
            RegGetValueW(
                root,
                pcwstr(&k),
                pcwstr(&v),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut data as *mut u32 as *mut c_void),
                Some(&mut size),
            )
        };
        (r == ERROR_SUCCESS).then_some(data)
    }

    pub fn reg_get_binary(root: HKEY, key: &str, value: &str) -> Option<Vec<u8>> {
        let (k, v) = (wide(key), wide(value));
        let mut size = 0u32;
        unsafe {
            if RegGetValueW(root, pcwstr(&k), pcwstr(&v), RRF_RT_ANY, None, None, Some(&mut size)) != ERROR_SUCCESS {
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            let mut ty = REG_VALUE_TYPE(0);
            let r = RegGetValueW(
                root,
                pcwstr(&k),
                pcwstr(&v),
                RRF_RT_ANY,
                Some(&mut ty),
                Some(buf.as_mut_ptr() as *mut c_void),
                Some(&mut size),
            );
            (r == ERROR_SUCCESS).then(|| {
                buf.truncate(size as usize);
                buf
            })
        }
    }

    pub fn reg_get_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let bytes = reg_get_binary(root, key, value)?;
        let words: Vec<u16> =
            bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&c| c != 0).collect();
        Some(String::from_utf16_lossy(&words))
    }

    pub fn reg_set_dword(root: HKEY, key: &str, value: &str, data: u32) -> bool {
        let (k, v) = (wide(key), wide(value));
        let bytes = data.to_le_bytes();
        unsafe {
            RegSetKeyValueW(root, pcwstr(&k), pcwstr(&v), REG_DWORD.0, Some(bytes.as_ptr() as *const c_void), 4)
                == ERROR_SUCCESS
        }
    }

    pub fn reg_set_string(root: HKEY, key: &str, value: &str, data: &str) -> bool {
        let (k, v, d) = (wide(key), wide(value), wide(data));
        unsafe {
            RegSetKeyValueW(
                root,
                pcwstr(&k),
                pcwstr(&v),
                REG_SZ.0,
                Some(d.as_ptr() as *const c_void),
                (d.len() * 2) as u32,
            ) == ERROR_SUCCESS
        }
    }

    pub fn reg_delete_value(root: HKEY, key: &str, value: &str) -> bool {
        let (k, v) = (wide(key), wide(value));
        let r = unsafe { RegDeleteKeyValueW(root, pcwstr(&k), pcwstr(&v)) };
        r == ERROR_SUCCESS || r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
    }

    // ---------------------------------------------------------------------------
    // Autostart

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn autostart_enabled() -> bool {
        reg_get_string(HKEY_CURRENT_USER, RUN_KEY, APP_NAME).is_some()
    }

    pub fn set_autostart(on: bool) -> bool {
        if on {
            let cmd = format!("\"{}\" --autostart", exe_path().display());
            reg_set_string(HKEY_CURRENT_USER, RUN_KEY, APP_NAME, &cmd)
        } else {
            reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, APP_NAME)
        }
    }

    // ---------------------------------------------------------------------------
    // System information

    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }

    /// The real OS build number (not subject to version lies).
    pub fn os_build() -> u32 {
        static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *BUILD.get_or_init(|| unsafe {
            let Ok(ntdll) = LoadLibraryW(w!("ntdll.dll")) else { return 19041 };
            let Some(f) = GetProcAddress(ntdll, s!("RtlGetVersion")) else { return 19041 };
            let f: unsafe extern "system" fn(*mut OsVersionInfo) -> i32 = std::mem::transmute(f);
            let mut info: OsVersionInfo = std::mem::zeroed();
            info.size = std::mem::size_of::<OsVersionInfo>() as u32;
            if f(&mut info) == 0 { info.build } else { 19041 }
        })
    }

    pub fn is_windows11() -> bool {
        os_build() >= 22000
    }

    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    /// Apps use the dark theme (Settings → Personalization → Colors).
    pub fn system_dark_mode() -> bool {
        reg_get_dword(HKEY_CURRENT_USER, PERSONALIZE, "AppsUseLightTheme") == Some(0)
    }

    /// The taskbar uses the dark theme (for the tray icon).
    pub fn taskbar_dark() -> bool {
        reg_get_dword(HKEY_CURRENT_USER, PERSONALIZE, "SystemUsesLightTheme") != Some(1)
    }

    pub fn transparency_enabled() -> bool {
        reg_get_dword(HKEY_CURRENT_USER, PERSONALIZE, "EnableTransparency") != Some(0)
    }

    pub fn high_contrast() -> bool {
        use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
        unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some(&mut hc as *mut _ as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_ok()
                && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
        }
    }

    /// Screen coordinates of the mouse cursor.
    pub fn cursor_pos() -> POINT {
        let mut pt = POINT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        }
        pt
    }

    pub fn rect_contains(r: &RECT, x: i32, y: i32) -> bool {
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    }

    fn message_box(hwnd: Option<HWND>, text: &str, style: MESSAGEBOX_STYLE) {
        let (t, c) = (wide(text), wide(APP_NAME));
        unsafe {
            MessageBoxW(hwnd, pcwstr(&t), pcwstr(&c), style | MB_OK);
        }
    }

    pub fn info_box(hwnd: Option<HWND>, text: &str) {
        message_box(hwnd, text, MB_ICONINFORMATION);
    }

    pub fn error_box(hwnd: Option<HWND>, text: &str) {
        message_box(hwnd, text, MB_ICONERROR);
    }

    // ---------------------------------------------------------------------------
    // Look

    /// Accent colors for dark and light backgrounds, from the accent palette.
    fn accent_colors() -> Option<(Color, Color)> {
        const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
        if let Some(p) = reg_get_binary(HKEY_CURRENT_USER, KEY, "AccentPalette").filter(|p| p.len() >= 32) {
            let at = |i: usize| rgba(p[i * 4], p[i * 4 + 1], p[i * 4 + 2], 255);
            // Palette entries: light3, light2, light1, base, dark1, dark2, dark3.
            return Some((at(1), at(4)));
        }
        let abgr = reg_get_dword(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")?;
        let c = rgba(abgr as u8, (abgr >> 8) as u8, (abgr >> 16) as u8, 255);
        Some((c, c))
    }

    /// What the theme needs to know about the Windows settings.
    pub fn system_look() -> SystemLook {
        SystemLook {
            auto_kind: if is_windows11() { Kind::Win11 } else { Kind::Win10 },
            dark: system_dark_mode(),
            accent: accent_colors(),
            transparency: transparency_enabled(),
            high_contrast: high_contrast(),
            system_backdrop: os_build() >= 22621,
            blur: false,
            desktop: None,
            font: Some("Segoe UI".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold("Română"), "romana");
        assert_eq!(fold("ȘȚ"), "st");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(nfc("e\u{0301}"), "é");
    }
}
