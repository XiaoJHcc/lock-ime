//! 低级键盘 hook：CapsLock 短按切输入法、长按锁大写（功能 #3）。

use crate::config::CapslockSwitchMode;
use crate::lang;
use crate::state::SwitchGoal;
use crate::TIMER_CAPS;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_CAPITAL, VK_LWIN, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KillTimer, PostMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WH_KEYBOARD_LL, WM_INPUTLANGCHANGEREQUEST, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const VK_CAPITAL_U32: u32 = VK_CAPITAL.0 as u32;

/// 直切后 / 兜底按键后回读校验的等待毫秒数。越短手感越快，但过短可能在系统尚未
/// 完成切换前就回读到旧布局、误判为「未生效」而多按一次。30ms 在实测下兼顾灵敏与稳。
const SWITCH_VERIFY_MS: u32 = 30;

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
            let mode = crate::state::with(|st| st.config.capslock_switch_mode)
                .unwrap_or(CapslockSwitchMode::CjkUs);
            match mode {
                CapslockSwitchMode::CjkUs => toggle_input_language(),
                CapslockSwitchMode::Cycle => cycle_input_language(),
            }
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

/// 合成一次 Win+Space（按下 LWin → 按下 Space → 抬起 Space → 抬起 LWin）。
///
/// 这是**系统级**输入法切换：与用户手按 Win+Space 完全等价，故能覆盖 IMM32/`WM_INPUTLANGCHANGEREQUEST`
/// 静默忽略的应用（Electron/Chromium，如 POPO）。注入期间置 `injecting`，让本 hook 放行自身合成事件。
///
/// 仅作**兜底**：正常应用走 HKL 直切（瞬时、可定向），只有直切没生效时才退到这里。
fn synth_win_space() {
    let set = crate::state::with(|st| st.injecting = true);
    if set.is_none() {
        return;
    }
    let mk = |vk, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        mk(VK_LWIN, false),
        mk(VK_SPACE, false),
        mk(VK_SPACE, true),
        mk(VK_LWIN, true),
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
    crate::state::with(|st| st.injecting = false);
}

/// HKL 直切：向前台窗口 `PostMessageW(WM_INPUTLANGCHANGEREQUEST)` 请求定向切到指定布局。
///
/// 原生 Win32 应用（记事本/浏览器/任务管理器）**瞬时生效、可定向**，无论装几个输入法都一步到位；
/// Electron/Chromium 会忽略它——由校验循环兜底到 Win+Space。
fn activate_layout_direct(hkl: HKL) {
    let hwnd = lang::foreground_window();
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            hwnd,
            WM_INPUTLANGCHANGEREQUEST,
            WPARAM(0),
            LPARAM(hkl.0 as isize),
        );
    }
}

/// 判断切换目标是否已达成（回读前台布局后调用）。
fn goal_satisfied(goal: SwitchGoal, current: HKL) -> bool {
    match goal {
        SwitchGoal::ReachLang(l) => lang::primary_lang(current) == l,
        SwitchGoal::LeaveHkl(orig) => current.0 as isize != orig,
    }
}

/// 启动一次切换：先尝试 HKL 直切，再排校验 tick。`budget` 为兜底阶段最多补按 Win+Space 的次数。
fn begin_switch(goal: SwitchGoal, direct: Option<HKL>, budget: u8) {
    if let Some(hkl) = direct {
        activate_layout_direct(hkl);
    }
    crate::state::with(|st| {
        st.switch_goal = Some(goal);
        st.switch_remaining = budget;
    });
    let hidden = crate::state::with(|st| st.hidden_hwnd).unwrap_or_default();
    if !hidden.is_invalid() {
        // 给系统一点时间完成直切后再回读校验。
        unsafe {
            SetTimer(hidden, crate::TIMER_SWITCH, SWITCH_VERIFY_MS, None);
        }
    }
}

/// 校验循环（WM_TIMER 调用）：回读前台布局；已到位则收尾，否则退到 Win+Space 兜底并继续校验。
pub fn on_switch_tick() {
    let Some(goal) = crate::state::with(|st| st.switch_goal).flatten() else {
        return;
    };

    let current = lang::foreground_layout();
    if goal_satisfied(goal, current) {
        crate::state::with(|st| {
            st.switch_goal = None;
            st.switch_remaining = 0;
        });
        return;
    }

    // 直切未生效（Electron 类应用）→ 合成 Win+Space 兜底，仍有余量则继续校验。
    let again = crate::state::with(|st| {
        if st.switch_remaining > 0 {
            st.switch_remaining -= 1;
            true
        } else {
            st.switch_goal = None; // 用尽次数仍未到位：放弃，避免死循环。
            false
        }
    })
    .unwrap_or(false);

    if again {
        synth_win_space();
        let hidden = crate::state::with(|st| st.hidden_hwnd).unwrap_or_default();
        if !hidden.is_invalid() {
            unsafe {
                SetTimer(hidden, crate::TIMER_SWITCH, SWITCH_VERIFY_MS, None);
            }
        }
    }
}

/// 顺序循环切换（等同 Win+Space）：直切到下一个已安装布局，兜底仅补按一次。
fn cycle_input_language() {
    let current = lang::foreground_layout();
    let mut layouts = lang::installed_layouts();
    layouts.dedup();
    if layouts.len() < 2 {
        return;
    }
    let idx = layouts.iter().position(|h| h.0 == current.0);
    let next = match idx {
        Some(i) => layouts[(i + 1) % layouts.len()],
        None => layouts[0],
    };
    // 循环语义只是「前进一格」：目标为「布局发生变化」，兜底补按一次即可。
    begin_switch(SwitchGoal::LeaveHkl(current.0 as isize), Some(next), 1);
}

/// 在英文(US) 与上一个 CJK 输入法之间二态切换：HKL 直切优先，Win+Space 兜底。
fn toggle_input_language() {
    let current = lang::foreground_layout();
    let current_lang = lang::primary_lang(current);

    let target = crate::state::with(|st| {
        if current_lang == lang::LANG_EN_US {
            // 当前英文 → 切回记住的上一个 CJK；没有则挑一个已安装的非英文布局。
            st.last_cjk_hkl.or_else(lang::pick_non_english_layout)
        } else {
            // 当前非英文 → 记住它，切到英文。
            st.last_cjk_hkl = Some(current);
            lang::english_layout()
        }
    })
    .flatten();

    let Some(target) = target else {
        return;
    };
    let target_lang = lang::primary_lang(target);
    if target_lang == current_lang {
        return;
    }

    // 兜底阶段最多按已安装布局数次 Win+Space，直到轮到目标语言。
    let budget = lang::installed_layouts().len().clamp(1, 8) as u8;
    begin_switch(SwitchGoal::ReachLang(target_lang), Some(target), budget);
}
