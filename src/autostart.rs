//! 开机自启：读写 HKCU\Software\Microsoft\Windows\CurrentVersion\Run。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "lock-ime";

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn open_run_key() -> Option<HKEY> {
    let subkey = wide(RUN_KEY);
    let mut hkey = HKEY::default();
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if rc == ERROR_SUCCESS {
        Some(hkey)
    } else {
        None
    }
}

/// 当前可执行文件路径（带引号，便于含空格的路径）。
fn exe_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.to_string_lossy()))
}

/// 设置或清除开机自启。
pub fn set_autostart(enabled: bool) -> bool {
    let Some(hkey) = open_run_key() else {
        return false;
    };
    let name = wide(VALUE_NAME);
    let ok = if enabled {
        let Some(cmd) = exe_command() else {
            unsafe { let _ = RegCloseKey(hkey); }
            return false;
        };
        let data = wide(&cmd);
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
        };
        let rc = unsafe {
            RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes))
        };
        rc == ERROR_SUCCESS
    } else {
        let rc = unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) };
        // 值本就不存在也算成功。
        rc == ERROR_SUCCESS || true
    };
    unsafe { let _ = RegCloseKey(hkey); }
    ok
}
