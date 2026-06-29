//! 焦点/前台切换事件 hook，触发 IME 模式锁定（功能 #1 / #2）。

use crate::ime::{self, imm32};
use crate::lang;
use crate::TIMER_APPLY;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    SetTimer, EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

/// 安装焦点 + 前台切换两个 WinEvent hook。返回的句柄需在退出时 `uninstall`。
pub fn install() -> Vec<HWINEVENTHOOK> {
    let mut hooks = Vec::new();
    for event in [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_FOCUS] {
        let h = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if !h.is_invalid() {
            hooks.push(h);
        }
    }
    hooks
}

/// 卸载 WinEvent hook。
pub fn uninstall(hooks: &[HWINEVENTHOOK]) {
    for h in hooks {
        unsafe {
            let _ = UnhookWinEvent(*h);
        }
    }
}

/// WinEvent 回调：不直接施加，而是排一个一次性延迟 timer，
/// 等焦点稳定（约 60ms）后再在 wndproc 里施加，规避 Win8+ 的「按用户全局」状态覆盖。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    crate::state::with(|st| {
        if !st.hidden_hwnd.is_invalid() {
            // 同一 timer id 重复 SetTimer 会重置计时，天然合并连续焦点事件。
            unsafe {
                SetTimer(st.hidden_hwnd, TIMER_APPLY, 60, None);
            }
        }
    });
}

/// 对当前前台窗口施加 IME 模式锁定（由 WM_TIMER 调用）。
pub fn apply_for_foreground() {
    let hwnd = lang::foreground_window();
    if hwnd.is_invalid() {
        return;
    }
    let layout = lang::window_layout(hwnd);
    let language = lang::primary_lang(layout);

    crate::state::with(|st| match language {
        lang::LANG_ZH_CN if st.config.chinese_lock_enabled => {
            imm32::ensure_chinese(hwnd);
        }
        lang::LANG_JA if st.config.japanese_lock_enabled => {
            let mode = ime::japanese_conversion_mode(st.config.japanese_mode);
            imm32::ensure_japanese(hwnd, mode);
        }
        _ => {}
    });
}
