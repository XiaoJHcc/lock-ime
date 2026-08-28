//! 焦点/前台切换事件 hook，触发 IME 模式锁定（功能 #1 / #2）。

use crate::ime::{self, imm32};
use crate::lang;
use crate::TIMER_APPLY;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, SetTimer, EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
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
                SetTimer(Some(st.hidden_hwnd), TIMER_APPLY, 60, None);
            }
        }
    });
}

/// 焦点控件是否为密码输入框（标准 EDIT 控件带 ES_PASSWORD 样式）。
///
/// 密码框必须放行：强制中文模式会让密码打出中文，且系统本就要求密码框走英文直输。
/// 注：只能识别原生 Win32 EDIT 控件；浏览器/Electron 自绘的密码框识别不到。
fn is_password_focus(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetGUIThreadInfo, GetWindowLongPtrW, GWL_STYLE, GUITHREADINFO,
    };
    const ES_PASSWORD: isize = 0x0020;

    let tid = unsafe { GetWindowThreadProcessId(hwnd, None) };
    if tid == 0 {
        return false;
    }
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(tid, &mut info) }.is_err() {
        return false;
    }
    let focus = info.hwndFocus;
    if focus.is_invalid() || focus.0.is_null() {
        return false;
    }
    let mut class = [0u16; 32];
    let len = unsafe { GetClassNameW(focus, &mut class) };
    if len <= 0 {
        return false;
    }
    // 标准 EDIT 控件类名（ASCII，不区分大小写）。
    let edit: [u16; 4] = [b'E' as u16, b'd' as u16, b'i' as u16, b't' as u16];
    let is_edit = len as usize == edit.len()
        && class[..4]
            .iter()
            .zip(edit.iter())
            .all(|(a, b)| (*a as u8).to_ascii_lowercase() == (*b as u8).to_ascii_lowercase());
    if !is_edit {
        return false;
    }
    let style = unsafe { GetWindowLongPtrW(focus, GWL_STYLE) };
    style & ES_PASSWORD != 0
}

/// 对当前前台窗口施加 IME 模式锁定（由 WM_TIMER / 切换校验循环 / 看门狗调用）。
///
/// 幂等：仅当当前状态与锁定目标不一致时才真正下发设置，周期轮询也不会打断输入。
pub fn apply_for_foreground() {
    let hwnd = lang::foreground_window();
    if hwnd.is_invalid() {
        return;
    }
    // 密码框不锁定，保持英文直输。
    if is_password_focus(hwnd) {
        return;
    }
    let layout = lang::window_layout(hwnd);
    let language = lang::primary_lang(layout);

    // 锁内只取配置、立即还锁：ensure_* 会向目标线程的 IME 窗口 SendMessage，
    // 跨线程等待期间本线程会重入处理待发消息（键盘 hook / 定时器回调等），
    // 若借用还在手上，再次 state::with 即双重 borrow_mut 直接 panic
    // （日志中实测出现过 RefCell already borrowed 崩溃）。
    enum Lock {
        Chinese,
        Japanese(u32),
    }
    let job = crate::state::with(|st| match language {
        lang::LANG_ZH_CN if st.config.chinese_lock_enabled => Some(Lock::Chinese),
        lang::LANG_JA if st.config.japanese_lock_enabled => Some(Lock::Japanese(
            ime::japanese_conversion_mode(st.config.japanese_mode),
        )),
        _ => None,
    })
    .flatten();

    match job {
        Some(Lock::Chinese) => imm32::ensure_chinese(hwnd),
        Some(Lock::Japanese(mode)) => imm32::ensure_japanese(hwnd, mode),
        None => {}
    }
}
