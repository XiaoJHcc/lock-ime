//! TSF compartment 回退后端（占位）。
//!
//! 现代微软 IME（Win11 22H2+）有时会静默忽略 IMM32 的 `WM_IME_CONTROL`。架构上正确的
//! 控制方式是 TSF compartment：
//!   - `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION` —— 转换模式（平假名/片假名/英数…）
//!   - `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` —— 开/关
//!
//! 大致流程：`CoCreateInstance(CLSID_TF_ThreadMgr)` → 取 `ITfCompartmentMgr` →
//! `GetCompartment(guid)` → `SetValue(tid, VT_I4)`；并 advise `ITfCompartmentEventSink`
//! 监听外部切换后重新施加。compartment 为「按线程」语义，需在焦点线程上初始化。
//!
//! v1 先以 IMM32 打底；当在目标机器上实测发现 IMM32 被忽略时，在此深化实现。

/// 占位：TSF 后端是否可用。当前恒为 false，调用方据此回退到 IMM32。
#[allow(dead_code)]
pub fn available() -> bool {
    false
}
