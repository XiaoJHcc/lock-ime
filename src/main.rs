//! lock-ime —— 常驻后台的 Windows 输入法模式锁定工具。
//!
//! 功能：
//!  1. 中文输入法在获得焦点/切换时锁定为中文模式；
//!  2. 日文输入法锁定为平假名（可配置）；
//!  3. CapsLock 短按切换输入法（英文 ↔ 上一个 CJK），长按锁大写。
#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod events;
mod ime;
mod keyboard;
mod lang;
mod settings_window;
mod state;
mod tray;

use config::Config;
use state::AppState;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage,
    RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY,
    WM_TIMER, WNDCLASSW,
};

/// 焦点稳定后施加 IME 模式的一次性 timer。
pub const TIMER_APPLY: usize = 1;
/// CapsLock 长按判定 timer。
pub const TIMER_CAPS: usize = 2;

fn main() {
    // 声明 Per-Monitor-V2 DPI 感知：必须在创建任何窗口之前调用，
    // 否则设置窗口会被系统位图拉伸，在高 DPI（如 4K 150%）下发虚。
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let config = Config::load();

    // 与注册表保持一致：每次启动按配置同步自启项。
    autostart::set_autostart(config.autostart);

    let hidden = match create_hidden_window() {
        Some(h) => h,
        None => return,
    };

    state::init(AppState::new(config, hidden));

    // 安装事件 hook 与键盘 hook。
    let win_hooks = events::install();
    let kbd_hook = keyboard::install();

    // 托盘必须在消息循环所在线程创建。
    let tray = tray::Tray::new();

    // 启动时先对当前前台施加一次。
    events::apply_for_foreground();

    run_message_loop(tray.as_ref());

    // 清理。
    keyboard::uninstall(kbd_hook);
    events::uninstall(&win_hooks);
}

/// 创建一个 message-only 隐藏窗口，用于接收 WM_TIMER。
fn create_hidden_window() -> Option<HWND> {
    unsafe {
        let hmodule = GetModuleHandleW(None).ok()?;
        let hinstance = HINSTANCE(hmodule.0);
        let class_name = w!("lock_ime_hidden_window");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("lock-ime"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
        .ok()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            match wparam.0 {
                TIMER_APPLY => {
                    unsafe {
                        let _ = KillTimer(hwnd, TIMER_APPLY);
                    }
                    events::apply_for_foreground();
                }
                TIMER_CAPS => {
                    unsafe {
                        let _ = KillTimer(hwnd, TIMER_CAPS);
                    }
                    keyboard::on_caps_longpress();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn run_message_loop(tray: Option<&tray::Tray>) {
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break; // 0 = WM_QUIT，-1 = 错误。
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 处理托盘菜单事件。
        if let Some(tray) = tray {
            while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                if tray.handle(&event.id) {
                    return; // 退出。
                }
            }
            if settings_window::NEED_REFRESH
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                tray.refresh();
            }
        }
    }
}
