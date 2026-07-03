//! 键盘布局 / 输入法语言检测。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardLayoutList, LoadKeyboardLayoutW, ACTIVATE_KEYBOARD_LAYOUT_FLAGS,
    HKL,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// 简体中文主语言 ID。
pub const LANG_ZH_CN: u16 = 0x0804;
/// 日文主语言 ID。
pub const LANG_JA: u16 = 0x0411;
/// 英文(美国) 主语言 ID。
pub const LANG_EN_US: u16 = 0x0409;

/// 从 HKL 取低 16 位主语言 ID。
pub fn primary_lang(hkl: HKL) -> u16 {
    (hkl.0 as usize as u16) & 0xFFFF
}

/// 当前前台窗口句柄。
pub fn foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

/// 取某窗口所属线程的键盘布局。
pub fn window_layout(hwnd: HWND) -> HKL {
    unsafe {
        let tid = GetWindowThreadProcessId(hwnd, None);
        GetKeyboardLayout(tid)
    }
}

/// 取前台窗口的键盘布局。
pub fn foreground_layout() -> HKL {
    window_layout(foreground_window())
}

/// 加载指定 KLID（如 "00000409"）对应的键盘布局。
pub fn load_layout(klid: &str) -> Option<HKL> {
    let wide: Vec<u16> = OsStr::new(klid)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let hkl =
        unsafe { LoadKeyboardLayoutW(PCWSTR(wide.as_ptr()), ACTIVATE_KEYBOARD_LAYOUT_FLAGS(0)) }
            .ok()?;
    if hkl.0.is_null() {
        None
    } else {
        Some(hkl)
    }
}

/// 英文(美国) 布局。
pub fn english_layout() -> Option<HKL> {
    load_layout("00000409")
}

/// 当前已安装（已加载）的键盘布局列表。
pub fn installed_layouts() -> Vec<HKL> {
    unsafe {
        let count = GetKeyboardLayoutList(None);
        if count <= 0 {
            return Vec::new();
        }
        let mut list = vec![HKL::default(); count as usize];
        let got = GetKeyboardLayoutList(Some(&mut list));
        list.truncate(got.max(0) as usize);
        list
    }
}

/// 从已安装布局中挑一个非英文（CJK 优先）的布局，作为二态切换的默认目标。
pub fn pick_non_english_layout() -> Option<HKL> {
    let layouts = installed_layouts();
    layouts
        .iter()
        .find(|h| matches!(primary_lang(**h), LANG_ZH_CN | LANG_JA))
        .or_else(|| layouts.iter().find(|h| primary_lang(**h) != LANG_EN_US))
        .copied()
}
