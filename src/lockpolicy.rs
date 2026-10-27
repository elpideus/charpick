//! Taking over Win+L.
//!
//! Windows handles Win+L before any keyboard hook can stop it. The only supported
//! way to keep it from locking is the per-user policy `DisableLockWorkstation`.
//! Charpick sets that policy only while it is needed ("armed"): when Win goes
//! down. It clears it again ("disarmed") after 30 seconds without input, before
//! sleep, when the session locks and when Charpick exits. So idle locks, resume
//! locks and Ctrl+Alt+Del keep working.
//!
//! The policy key is admin-only by default. The installer runs a one-time,
//! elevated step (`--grant-lock-access <SID>`). That step lets the user's own
//! account write that single key.

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo,
    NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS, SE_REGISTRY_KEY, SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID,
    TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_USERS, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SAM_FLAGS, RegCloseKey, RegCreateKeyExW, RegOpenKeyExW,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows_core::{PWSTR, w};

use crate::util::{self, APP_KEY, pcwstr, wide};

const POLICY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";
const VALUE: &str = "DisableLockWorkstation";
/// Set in Charpick's own key while Charpick owns the policy value.
const MARKER: &str = "LockPolicyArmed";

const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;

fn policy_value() -> Option<u32> {
    util::reg_get_dword(HKEY_CURRENT_USER, POLICY_KEY, VALUE)
}

fn owned() -> bool {
    util::reg_get_dword(HKEY_CURRENT_USER, APP_KEY, MARKER) == Some(1)
}

/// Locking is disabled by someone else (an administrator or group policy).
pub fn disabled_by_admin() -> bool {
    policy_value() == Some(1) && !owned()
}

/// The current user may write the policy value.
pub fn writable() -> bool {
    let k = wide(POLICY_KEY);
    let mut h = HKEY::default();
    unsafe {
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, pcwstr(&k), Some(0), KEY_SET_VALUE | KEY_QUERY_VALUE, &mut h);
        if r == ERROR_SUCCESS {
            let _ = RegCloseKey(h);
            true
        } else {
            false
        }
    }
}

/// Stop Windows from locking on Win+L. Returns true when Win+L is now free.
pub fn arm() -> bool {
    match policy_value() {
        Some(1) => true,
        _ => {
            // Write the marker first: if Charpick dies in between, the next start cleans up.
            util::reg_set_dword(HKEY_CURRENT_USER, APP_KEY, MARKER, 1)
                && util::reg_set_dword(HKEY_CURRENT_USER, POLICY_KEY, VALUE, 1)
        }
    }
}

/// Give Win+L back to Windows (only if Charpick set the policy).
pub fn disarm() {
    if owned() {
        if policy_value().is_some() {
            util::reg_delete_value(HKEY_CURRENT_USER, POLICY_KEY, VALUE);
        }
        util::reg_delete_value(HKEY_CURRENT_USER, APP_KEY, MARKER);
    }
}

/// Lock the PC even though Win+L is taken over.
pub fn lock_now() {
    disarm();
    crate::hook::arm_done(false);
    unsafe {
        let _ = LockWorkStation();
    }
}

// ---------------------------------------------------------------------------
// One-time elevated setup

pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut c_void), len, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        out
    }
}

/// Elevated: let `sid` write its own `Policies\System` key.
pub fn grant(sid: &str, revoke: bool) -> Result<(), String> {
    if !sid.starts_with("S-1-") || sid.contains(['\\', '/']) {
        return Err(format!("invalid SID {sid}"));
    }
    unsafe {
        let mut psid = PSID::default();
        let ws = wide(sid);
        ConvertStringSidToSidW(pcwstr(&ws), &mut psid).map_err(|e| format!("SID: {e}"))?;
        let result = (|| {
            let path = wide(&format!(r"{sid}\{POLICY_KEY}"));
            let mut key = HKEY::default();
            let r = RegCreateKeyExW(
                HKEY_USERS,
                pcwstr(&path),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                REG_SAM_FLAGS(KEY_READ.0 | WRITE_DAC),
                None,
                &mut key,
                None,
            );
            if r != ERROR_SUCCESS {
                return Err(format!("opening the policy key failed ({})", r.0));
            }
            let handle = HANDLE(key.0);
            let mut old: *mut ACL = std::ptr::null_mut();
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let r = GetSecurityInfo(
                handle,
                SE_REGISTRY_KEY,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut old),
                None,
                Some(&mut sd),
            );
            if r != ERROR_SUCCESS {
                let _ = RegCloseKey(key);
                return Err(format!("reading permissions failed ({})", r.0));
            }
            let ea = EXPLICIT_ACCESS_W {
                grfAccessPermissions: KEY_QUERY_VALUE.0 | KEY_SET_VALUE.0 | READ_CONTROL,
                grfAccessMode: if revoke { REVOKE_ACCESS } else { GRANT_ACCESS },
                grfInheritance: NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: PWSTR(psid.0 as *mut u16),
                },
            };
            let mut new: *mut ACL = std::ptr::null_mut();
            let r = SetEntriesInAclW(Some(&[ea]), Some(old), &mut new);
            let out = if r != ERROR_SUCCESS {
                Err(format!("building permissions failed ({})", r.0))
            } else {
                let r =
                    SetSecurityInfo(handle, SE_REGISTRY_KEY, DACL_SECURITY_INFORMATION, None, None, Some(new), None);
                LocalFree(Some(HLOCAL(new as *mut c_void)));
                if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("writing permissions failed ({})", r.0)) }
            };
            LocalFree(Some(HLOCAL(sd.0)));
            let _ = RegCloseKey(key);
            out
        })();
        LocalFree(Some(HLOCAL(psid.0)));
        result
    }
}

pub enum Elevation {
    Done,
    Cancelled,
    Failed(String),
}

/// Run `--grant-lock-access` (or `--revoke-lock-access`) elevated and wait for it.
pub fn run_elevated(revoke: bool) -> Elevation {
    let Some(sid) = current_user_sid() else { return Elevation::Failed("could not read the user's SID".into()) };
    let exe = wide(&util::exe_path().to_string_lossy());
    let params = wide(&format!("{} {sid}", if revoke { "--revoke-lock-access" } else { "--grant-lock-access" }));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: pcwstr(&exe),
        lpParameters: pcwstr(&params),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            return if e.code() == ERROR_CANCELLED.to_hresult() {
                Elevation::Cancelled
            } else {
                Elevation::Failed(e.message())
            };
        }
        if info.hProcess.is_invalid() {
            return Elevation::Failed("no process".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if code == 0 { Elevation::Done } else { Elevation::Failed(format!("setup exited with code {code}")) }
    }
}
