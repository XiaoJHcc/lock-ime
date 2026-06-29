//! 低级键盘 hook：CapsLock 短按切输入法、长按锁大写（功能 #3）。

use crate::lang;
use crate::TIMER_CAPS;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_CAPITAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KillTimer, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, WH_KEYBOARD_LL, WM_INPUTLANGCHANGEREQUEST, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const VK_CAPITAL_U32: u32 = VK_CAPITAL.0 as u32;

/// 安装低级键盘 hook。
pub fn install() -> HHOOK {
    unsafe {
        // 低级 hook 的 hmod 可为 None（回调在本模块内）。
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0).unwrap_or_default()
    }
}

/// 卸载低级键盘 hook。
pub fn uninstall(hook: HHOOK) {
    if !hook.is_invalid() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let vk = kb.vkCode;
    let msg = wparam.0 as u32;
    let injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;

    // 只关心 CapsLock。
    if vk != VK_CAPITAL_U32 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // 读取开关与自注入标志。
    let (enabled, injecting) = crate::state::with(|st| {
        (st.config.capslock_switch_enabled, st.injecting)
    })
    .unwrap_or((false, false));

    // 功能关闭，或这是我们自己合成的注入事件 → 放行系统默认处理。
    if !enabled || (injected && injecting) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

    let hidden = crate::state::with(|st| st.hidden_hwnd).unwrap_or_default();

    if is_down {
        let longpress_ms = crate::state::with(|st| {
            if !st.caps_pending {
                st.caps_pending = true;
                st.caps_consumed = false;
                Some(st.config.capslock_longpress_ms)
            } else {
                None // 按住时的自动重复，忽略。
            }
        })
        .flatten();

        if let Some(ms) = longpress_ms {
            if !hidden.is_invalid() {
                unsafe {
                    SetTimer(hidden, TIMER_CAPS, ms as u32, None);
                }
            }
        }
        // 吞掉物理 CapsLock 按下，避免立即翻转大写状态。
        return LRESULT(1);
    }

    if is_up {
        if !hidden.is_invalid() {
            unsafe {
                let _ = KillTimer(hidden, TIMER_CAPS);
            }
        }
        let do_switch = crate::state::with(|st| {
            let short_press = st.caps_pending && !st.caps_consumed;
            st.caps_pending = false;
            st.caps_consumed = false;
            short_press
        })
        .unwrap_or(false);

        if do_switch {
            toggle_input_language();
        }
        // 吞掉物理 CapsLock 抬起。
        return LRESULT(1);
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 长按计时到点（在 wndproc 的 WM_TIMER 中调用）：合成一次真正的 CapsLock 按键以翻转大写状态。
pub fn on_caps_longpress() {
    let should = crate::state::with(|st| {
        if st.caps_pending && !st.caps_consumed {
            st.caps_consumed = true;
            st.injecting = true;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);

    if should {
        synth_capslock();
        crate::state::with(|st| st.injecting = false);
    }
}

/// 合成 CapsLock 按下+抬起（注入事件，hook 会因 injecting 标志放行）。
fn synth_capslock() {
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CAPITAL,
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CAPITAL,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [down, up];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// 在英文(US) 与上一个 CJK 输入法之间切换前台窗口的键盘布局。
fn toggle_input_language() {
    let hwnd: HWND = lang::foreground_window();
    if hwnd.is_invalid() {
        return;
    }
    let current = lang::window_layout(hwnd);
    let current_lang = lang::primary_lang(current);

    let target = crate::state::with(|st| {
        if current_lang == lang::LANG_EN_US {
            // 当前英文 → 切到记住的 CJK；没有则挑一个已安装的非英文布局。
            st.last_cjk_hkl.or_else(lang::pick_non_english_layout)
        } else {
            // 当前非英文 → 记住它，切到英文。
            st.last_cjk_hkl = Some(current);
            lang::english_layout()
        }
    })
    .flatten();

    if let Some(hkl) = target {
        unsafe {
            // 向前台窗口请求切换键盘布局；lParam 传 HKL。
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(hkl.0 as isize),
            );
        }
    }
}
