//! IMM32 后端：通过默认 IME 窗口发送 `WM_IME_CONTROL` 控制开关状态与转换模式。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

const WM_IME_CONTROL: u32 = 0x0283;

const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_SETCONVERSIONMODE: usize = 0x0002;
const IMC_GETOPENSTATUS: usize = 0x0005;
const IMC_SETOPENSTATUS: usize = 0x0006;

/// 取目标窗口对应的默认 IME 窗口句柄。
fn ime_window(hwnd: HWND) -> Option<HWND> {
    let ime = unsafe { ImmGetDefaultIMEWnd(hwnd) };
    if ime.0.is_null() {
        None
    } else {
        Some(ime)
    }
}

fn ime_control(ime: HWND, cmd: usize, value: isize) -> isize {
    unsafe { SendMessageW(ime, WM_IME_CONTROL, WPARAM(cmd), LPARAM(value)).0 }
}

/// 读取开关状态（true = 开 = 中文）。
pub fn get_open_status(hwnd: HWND) -> Option<bool> {
    let ime = ime_window(hwnd)?;
    Some(ime_control(ime, IMC_GETOPENSTATUS, 0) != 0)
}

/// 设置开关状态。中文输入法：开 ≈ 中文模式。
pub fn set_open_status(hwnd: HWND, open: bool) -> bool {
    let Some(ime) = ime_window(hwnd) else {
        return false;
    };
    ime_control(ime, IMC_SETOPENSTATUS, if open { 1 } else { 0 });
    true
}

/// 读取转换模式标志位。
pub fn get_conversion_mode(hwnd: HWND) -> Option<u32> {
    let ime = ime_window(hwnd)?;
    Some(ime_control(ime, IMC_GETCONVERSIONMODE, 0) as u32)
}

/// 设置转换模式标志位（日文：平假名/片假名/英数）。
pub fn set_conversion_mode(hwnd: HWND, mode: u32) -> bool {
    let Some(ime) = ime_window(hwnd) else {
        return false;
    };
    ime_control(ime, IMC_SETCONVERSIONMODE, mode as isize);
    true
}

/// 确保中文输入法处于中文模式：若当前为关（英文直输）则打开。
pub fn ensure_chinese(hwnd: HWND) {
    if get_open_status(hwnd) != Some(true) {
        set_open_status(hwnd, true);
    }
}

/// 确保日文输入法处于目标转换模式：仅当不一致时才设置，避免打断输入。
pub fn ensure_japanese(hwnd: HWND, target_mode: u32) {
    // 日文场景下也需确保 IME 处于开状态，否则转换模式无意义。
    if get_open_status(hwnd) != Some(true) {
        set_open_status(hwnd, true);
    }
    if get_conversion_mode(hwnd) != Some(target_mode) {
        set_conversion_mode(hwnd, target_mode);
    }
}
