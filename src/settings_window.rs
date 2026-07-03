//! 设置窗口：用 Win32 原生控件做一个简单的设置页面。

use crate::config::{CapslockSwitchMode, JapaneseMode};
use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontW, DeleteObject, HGDIOBJ};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, CREATESTRUCTW, CW_USEDEFAULT, DefWindowProcW, DestroyWindow, GetDlgItem,
    GetWindowTextW, HMENU, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowPos,
    SetWindowTextW, ShowWindow, SET_WINDOW_POS_FLAGS, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_SETFONT, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};

// SetWindowPos 标志。
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

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
const ID_RAD_CJKUS: i32 = 1011;
const ID_RAD_CYCLE: i32 = 1012;

thread_local! {
    static WND: Cell<Option<HWND>> = const { Cell::new(None) };
    // 设置窗口的字体句柄（HFONT.0 as isize），WM_DESTROY 时释放。
    static FONT: Cell<isize> = const { Cell::new(0) };
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
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(16usize as *mut _), // COLOR_BTNFACE + 1
                ..Default::default()
            };
            RegisterClassW(&wc);
            if let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("Lock IME 设置"),
                WINDOW_STYLE(WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                420,
                470,
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

                // 按窗口 DPI 缩放（96 = 100%）。所有坐标以 96-dpi 逻辑像素书写。
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                let s = move |v: i32| v * dpi / 96;

                // 用 ClearType 雅黑替换发虚的位图 DEFAULT_GUI_FONT。
                let hfont = CreateFontW(
                    -(9 * dpi / 72), // 9pt
                    0,
                    0,
                    0,
                    400, // FW_NORMAL
                    0,
                    0,
                    0,
                    1, // DEFAULT_CHARSET
                    0,
                    0,
                    5, // CLEARTYPE_QUALITY
                    0,
                    w!("Microsoft YaHei UI"),
                );
                FONT.with(|c| c.set(hfont.0 as isize));

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
                        s(x),
                        s(y),
                        s(w),
                        s(h),
                        hwnd,
                        HMENU(id as usize as *mut _),
                        hinst,
                        None,
                    )
                    .unwrap_or(HWND(null_mut()));
                    let _ = SendMessageW(child, WM_SETFONT, WPARAM(hfont.0 as usize), LPARAM(1));
                    child
                };

                let btn = |s: u32| s | WS_CHILD | WS_VISIBLE;
                let ctl = |s: u32| s | WS_CHILD | WS_VISIBLE | WS_TABSTOP;

                // ---- 统一网格（96-dpi 逻辑像素）----
                // 分组框：x=12, 宽=356（窗口客户区宽 380，左右各留 12）。
                // 组内：复选框 x=26；单选/标签缩进 x=40；行高 20，按钮高 28。
                const GX: i32 = 12;
                const GW: i32 = 356;
                const CX: i32 = 26; // 复选框左
                const CW_: i32 = GW - (CX - GX) - 12; // 组内控件宽
                const RX: i32 = 40; // 单选/标签缩进

                // 区1：中文
                make(w!("BUTTON"), w!("中文"), btn(BS_GROUPBOX), GX, 10, GW, 52, 1000);
                make(w!("BUTTON"), w!("锁定为中文模式"), btn(BS_AUTOCHECKBOX), CX, 34, CW_, 20, ID_CHK_CN);

                // 区2：日文
                make(w!("BUTTON"), w!("日文"), btn(BS_GROUPBOX), GX, 72, GW, 100, 1000);
                make(w!("BUTTON"), w!("锁定转换模式"), btn(BS_AUTOCHECKBOX), CX, 96, CW_, 20, ID_CHK_JA);
                // 转换模式三选一横排，等距。
                make(w!("BUTTON"), w!("平假名"), ctl(BS_AUTORADIOBUTTON | WS_GROUP), RX, 128, 100, 20, ID_RAD_HIRA);
                make(w!("BUTTON"), w!("片假名"), ctl(BS_AUTORADIOBUTTON), RX + 108, 128, 100, 20, ID_RAD_KATA);
                make(w!("BUTTON"), w!("全角英数"), ctl(BS_AUTORADIOBUTTON), RX + 216, 128, 100, 20, ID_RAD_FULL);

                // 区3：CapsLock
                make(w!("BUTTON"), w!("CapsLock"), btn(BS_GROUPBOX), GX, 182, GW, 128, 1000);
                make(w!("BUTTON"), w!("短按切换输入法"), btn(BS_AUTOCHECKBOX), CX, 206, CW_, 20, ID_CHK_CAPS);
                // 切换表现两选一。
                make(w!("BUTTON"), w!("CJK / US 切换"), ctl(BS_AUTORADIOBUTTON | WS_GROUP), RX, 238, 150, 20, ID_RAD_CJKUS);
                make(w!("BUTTON"), w!("正常循环"), ctl(BS_AUTORADIOBUTTON), RX + 160, 238, 150, 20, ID_RAD_CYCLE);
                // 长按阈值：标签与输入框同一行、垂直居中对齐。
                make(w!("STATIC"), w!("长按大写锁定 阈值（毫秒）"), btn(SS_LEFT), RX, 276, 190, 20, 1000);
                make(w!("EDIT"), w!("300"), ctl(WS_BORDER | ES_AUTOHSCROLL | ES_NUMBER), RX + 194, 274, 70, 24, ID_EDIT_LP);

                // 区4：开机自启
                make(w!("BUTTON"), w!("开机自启"), btn(BS_GROUPBOX), GX, 320, GW, 52, 1000);
                make(w!("BUTTON"), w!("随 Windows 启动"), btn(BS_AUTOCHECKBOX), CX, 344, CW_, 20, ID_CHK_AUTO);

                // 底部按钮，右对齐到分组框右缘。
                make(w!("BUTTON"), w!("确定"), ctl(BS_DEFPUSHBUTTON | WS_GROUP), 176, 388, 92, 28, ID_BTN_OK);
                make(w!("BUTTON"), w!("取消"), ctl(BS_PUSHBUTTON), 276, 388, 92, 28, ID_BTN_CANCEL);

                load_config_to_controls(hwnd);

                // 按 DPI 调整窗口外框，使客户区正好容纳网格（380 x 430 逻辑像素）。
                let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
                let mut rc = RECT { left: 0, top: 0, right: s(380), bottom: s(430) };
                let _ = AdjustWindowRectExForDpi(
                    &mut rc,
                    WINDOW_STYLE(style),
                    false,
                    WINDOW_EX_STYLE(0),
                    dpi as u32,
                );
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SET_WINDOW_POS_FLAGS(SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE),
                );
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
            FONT.with(|c| {
                let v = c.get();
                if v != 0 {
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(v as *mut _));
                    }
                    c.set(0);
                }
            });
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
    let (cn, ja, caps, auto, mode, caps_mode, lp) = crate::state::with(|st| {
        (
            st.config.chinese_lock_enabled,
            st.config.japanese_lock_enabled,
            st.config.capslock_switch_enabled,
            st.config.autostart,
            st.config.japanese_mode,
            st.config.capslock_switch_mode,
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
    set_checked(
        hwnd,
        match caps_mode {
            CapslockSwitchMode::CjkUs => ID_RAD_CJKUS,
            CapslockSwitchMode::Cycle => ID_RAD_CYCLE,
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
    let caps_mode = if is_checked(hwnd, ID_RAD_CYCLE) {
        CapslockSwitchMode::Cycle
    } else {
        CapslockSwitchMode::CjkUs
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
        st.config.capslock_switch_mode = caps_mode;
        st.config.capslock_longpress_ms = lp;
        st.config.autostart = auto;
        let _ = st.config.save();
    });
    NEED_REFRESH.store(true, Ordering::Relaxed);
    true
}
