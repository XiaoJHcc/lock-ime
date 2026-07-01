//! 设置窗口：用 Win32 原生控件做一个简单的设置页面。

use crate::config::JapaneseMode;
use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, CREATESTRUCTW, CW_USEDEFAULT, DefWindowProcW, DestroyWindow, GetDlgItem,
    GetWindowTextW, HMENU, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowTextW,
    ShowWindow, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_SETFONT,
    WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};

// 控件风格数值常量。
const BS_AUTOCHECKBOX: u32 = 0x0003;
const BS_AUTORADIOBUTTON: u32 = 0x0009;
const BS_GROUPBOX: u32 = 0x0007;
const BS_PUSHBUTTON: u32 = 0x0000;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const SS_LEFT: u32 = 0x00000000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_NUMBER: u32 = 0x2000;
const WS_CHILD: u32 = 0x40000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_BORDER: u32 = 0x00800000;
const WS_TABSTOP: u32 = 0x00010000;
const WS_GROUP: u32 = 0x00020000;
const WS_OVERLAPPED: u32 = 0;
const WS_CAPTION: u32 = 0x00C00000;
const WS_SYSMENU: u32 = 0x00080000;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_UNCHECKED: usize = 0;
const BST_CHECKED: usize = 1;
const BN_CLICKED: u16 = 0;

const ID_CHK_CN: i32 = 1001;
const ID_CHK_JA: i32 = 1002;
const ID_CHK_CAPS: i32 = 1003;
const ID_CHK_AUTO: i32 = 1004;
const ID_RAD_HIRA: i32 = 1005;
const ID_RAD_KATA: i32 = 1006;
const ID_RAD_FULL: i32 = 1007;
const ID_EDIT_LP: i32 = 1008;
const ID_BTN_OK: i32 = 1009;
const ID_BTN_CANCEL: i32 = 1010;

thread_local! {
    static WND: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// 设置窗口确定后置位，主消息循环检测后调用 `Tray::refresh` 并清除。
pub static NEED_REFRESH: AtomicBool = AtomicBool::new(false);

/// 打开设置窗口；已打开则前置。
pub fn open() {
    WND.with(|c| {
        if let Some(h) = c.get() {
            unsafe {
                let _ = SetForegroundWindow(h);
            }
            return;
        }
        unsafe {
            let hmodule = GetModuleHandleW(None).ok().unwrap();
            let hinst = HINSTANCE(hmodule.0);
            let class_name = w!("lock_ime_settings");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);
            if let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("lock-ime 设置"),
                WINDOW_STYLE(WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                400,
                None,
                None,
                hinst,
                None,
            ) {
                c.set(Some(hwnd));
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            unsafe {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let hinst = cs.hInstance;
                let hfont = GetStockObject(DEFAULT_GUI_FONT);
                let make = |class: PCWSTR,
                            text: PCWSTR,
                            style: u32,
                            x: i32,
                            y: i32,
                            w: i32,
                            h: i32,
                            id: i32|
                 -> HWND {
                    let child = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        text,
                        WINDOW_STYLE(style),
                        x,
                        y,
                        w,
                        h,
                        hwnd,
                        HMENU(id as usize as *mut _),
                        hinst,
                        None,
                    )
                    .unwrap_or(HWND(null_mut()));
                    let _ = SendMessageW(
                        child,
                        WM_SETFONT,
                        WPARAM(hfont.0 as usize),
                        LPARAM(1),
                    );
                    child
                };

                let btn = |s: u32| s | WS_CHILD | WS_VISIBLE;
                let ctl = |s: u32| s | WS_CHILD | WS_VISIBLE | WS_TABSTOP;

                make(w!("BUTTON"), w!("功能"), btn(BS_GROUPBOX), 10, 5, 370, 140, 1000);
                make(
                    w!("BUTTON"),
                    w!("中文输入法锁定为中文模式"),
                    btn(BS_AUTOCHECKBOX),
                    25,
                    30,
                    330,
                    22,
                    ID_CHK_CN,
                );
                make(
                    w!("BUTTON"),
                    w!("日文输入法锁定转换模式"),
                    btn(BS_AUTOCHECKBOX),
                    25,
                    55,
                    330,
                    22,
                    ID_CHK_JA,
                );
                make(
                    w!("BUTTON"),
                    w!("CapsLock 短按切换输入法"),
                    btn(BS_AUTOCHECKBOX),
                    25,
                    80,
                    330,
                    22,
                    ID_CHK_CAPS,
                );
                make(
                    w!("BUTTON"),
                    w!("开机自启"),
                    btn(BS_AUTOCHECKBOX),
                    25,
                    105,
                    330,
                    22,
                    ID_CHK_AUTO,
                );

                make(
                    w!("BUTTON"),
                    w!("日文转换模式"),
                    btn(BS_GROUPBOX),
                    10,
                    155,
                    370,
                    120,
                    1000,
                );
                make(
                    w!("BUTTON"),
                    w!("平假名（全角）"),
                    ctl(BS_AUTORADIOBUTTON | WS_GROUP),
                    25,
                    180,
                    330,
                    22,
                    ID_RAD_HIRA,
                );
                make(
                    w!("BUTTON"),
                    w!("片假名（全角）"),
                    ctl(BS_AUTORADIOBUTTON),
                    25,
                    205,
                    330,
                    22,
                    ID_RAD_KATA,
                );
                make(
                    w!("BUTTON"),
                    w!("全角英数"),
                    ctl(BS_AUTORADIOBUTTON),
                    25,
                    230,
                    330,
                    22,
                    ID_RAD_FULL,
                );

                make(
                    w!("STATIC"),
                    w!("CapsLock 长按判定 (毫秒)"),
                    btn(SS_LEFT),
                    25,
                    290,
                    200,
                    20,
                    1000,
                );
                make(
                    w!("EDIT"),
                    w!("300"),
                    ctl(WS_BORDER | ES_AUTOHSCROLL | ES_NUMBER),
                    230,
                    288,
                    80,
                    22,
                    ID_EDIT_LP,
                );

                make(
                    w!("BUTTON"),
                    w!("确定"),
                    ctl(BS_DEFPUSHBUTTON | WS_GROUP),
                    190,
                    325,
                    90,
                    28,
                    ID_BTN_OK,
                );
                make(
                    w!("BUTTON"),
                    w!("取消"),
                    ctl(BS_PUSHBUTTON),
                    290,
                    325,
                    90,
                    28,
                    ID_BTN_CANCEL,
                );

                load_config_to_controls(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            if code == BN_CLICKED {
                if id == ID_BTN_OK {
                    if apply_controls(hwnd) {
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                } else if id == ID_BTN_CANCEL {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            WND.with(|c| {
                if c.get() == Some(hwnd) {
                    c.set(None);
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn is_checked(hwnd: HWND, id: i32) -> bool {
    unsafe {
        let h = GetDlgItem(hwnd, id).unwrap_or(HWND(null_mut()));
        SendMessageW(h, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize == BST_CHECKED
    }
}

fn set_checked(hwnd: HWND, id: i32, v: bool) {
    unsafe {
        let h = GetDlgItem(hwnd, id).unwrap_or(HWND(null_mut()));
        let _ = SendMessageW(
            h,
            BM_SETCHECK,
            WPARAM(if v { BST_CHECKED } else { BST_UNCHECKED }),
            LPARAM(0),
        );
    }
}

fn load_config_to_controls(hwnd: HWND) {
    let (cn, ja, caps, auto, mode, lp) = crate::state::with(|st| {
        (
            st.config.chinese_lock_enabled,
            st.config.japanese_lock_enabled,
            st.config.capslock_switch_enabled,
            st.config.autostart,
            st.config.japanese_mode,
            st.config.capslock_longpress_ms,
        )
    })
    .unwrap_or_default();

    set_checked(hwnd, ID_CHK_CN, cn);
    set_checked(hwnd, ID_CHK_JA, ja);
    set_checked(hwnd, ID_CHK_CAPS, caps);
    set_checked(hwnd, ID_CHK_AUTO, auto);
    set_checked(
        hwnd,
        match mode {
            JapaneseMode::Hiragana => ID_RAD_HIRA,
            JapaneseMode::Katakana => ID_RAD_KATA,
            JapaneseMode::FullWidthAlnum => ID_RAD_FULL,
        },
        true,
    );

    unsafe {
        if let Ok(h) = GetDlgItem(hwnd, ID_EDIT_LP) {
            let s = HSTRING::from(lp.to_string());
            let _ = SetWindowTextW(h, PCWSTR(s.as_ptr()));
        }
    }
}

fn apply_controls(hwnd: HWND) -> bool {
    let cn = is_checked(hwnd, ID_CHK_CN);
    let ja = is_checked(hwnd, ID_CHK_JA);
    let caps = is_checked(hwnd, ID_CHK_CAPS);
    let auto = is_checked(hwnd, ID_CHK_AUTO);
    let mode = if is_checked(hwnd, ID_RAD_HIRA) {
        JapaneseMode::Hiragana
    } else if is_checked(hwnd, ID_RAD_KATA) {
        JapaneseMode::Katakana
    } else {
        JapaneseMode::FullWidthAlnum
    };
    let lp = unsafe {
        let h = GetDlgItem(hwnd, ID_EDIT_LP).unwrap_or(HWND(null_mut()));
        let mut buf = [0u16; 32];
        let n = GetWindowTextW(h, &mut buf[..]);
        let s = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        s.trim().parse::<u64>().unwrap_or(300)
    };

    crate::autostart::set_autostart(auto);
    crate::state::with(|st| {
        st.config.chinese_lock_enabled = cn;
        st.config.japanese_lock_enabled = ja;
        st.config.japanese_mode = mode;
        st.config.capslock_switch_enabled = caps;
        st.config.capslock_longpress_ms = lp;
        st.config.autostart = auto;
        let _ = st.config.save();
    });
    NEED_REFRESH.store(true, Ordering::Relaxed);
    true
}
