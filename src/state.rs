//! 进程内共享状态。
//!
//! 所有 hook 回调（WinEvent / 低级键盘）以及隐藏窗口的 wndproc 都运行在主线程上，
//! 因此用 `thread_local! + RefCell` 持有可变状态即可，无需跨线程同步。

use crate::config::Config;
use std::cell::RefCell;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;

/// 一次切换的目标判定：校验循环据此判断「是否已到位」。
#[derive(Clone, Copy)]
pub enum SwitchGoal {
    /// 二态切换：前台主语言 ID 达到该值即完成。
    ReachLang(u16),
    /// 循环切换：前台布局 HKL 只要不再等于该原始值（发生了移动）即完成。
    LeaveHkl(isize),
}

pub struct AppState {
    pub config: Config,
    /// 隐藏的消息窗口，用于接收 WM_TIMER 等。
    pub hidden_hwnd: HWND,

    // ---- CapsLock 状态机 ----
    /// 物理 CapsLock 已按下并被吞掉，等待判定短按/长按。
    pub caps_pending: bool,
    /// 本次按下已被作为「锁大写」消费（长按已触发）。
    pub caps_consumed: bool,
    /// 正在合成注入按键，键盘 hook 应放行自己注入的事件。
    pub injecting: bool,

    // ---- 输入法切换（HKL 直切优先 + Win+Space 兜底 + 定时器校验）----
    /// 二态切换时记住的「上一个 CJK 键盘布局」，用于切回。
    pub last_cjk_hkl: Option<HKL>,
    /// 进行中的切换目标判定（None = 无进行中的切换）。
    pub switch_goal: Option<SwitchGoal>,
    /// 兜底阶段还可再合成几次 Win+Space（防死循环上限）。
    pub switch_remaining: u8,
}

impl AppState {
    pub fn new(config: Config, hidden_hwnd: HWND) -> Self {
        AppState {
            config,
            hidden_hwnd,
            caps_pending: false,
            caps_consumed: false,
            injecting: false,
            last_cjk_hkl: None,
            switch_goal: None,
            switch_remaining: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// 初始化全局状态（仅主线程调用一次）。
pub fn init(state: AppState) {
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

/// 访问可变状态；未初始化时返回 None。
pub fn with<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}
