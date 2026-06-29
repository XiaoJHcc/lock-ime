//! 输入法模式控制抽象。
//!
//! v1 以 IMM32（`WM_IME_CONTROL`）打底；TSF compartment 作为可扩展回退（见 `tsf` 模块）。

pub mod imm32;
pub mod tsf;

use crate::config::JapaneseMode;

/// IME 转换模式标志位（与 IMM32 `IME_CMODE_*` 一致）。
pub const IME_CMODE_ALPHANUMERIC: u32 = 0x0000;
pub const IME_CMODE_NATIVE: u32 = 0x0001;
pub const IME_CMODE_KATAKANA: u32 = 0x0002;
pub const IME_CMODE_FULLSHAPE: u32 = 0x0008;

/// 把配置里的日文模式映射成转换模式标志位。
pub fn japanese_conversion_mode(mode: JapaneseMode) -> u32 {
    match mode {
        // 平假名（全角）。
        JapaneseMode::Hiragana => IME_CMODE_NATIVE | IME_CMODE_FULLSHAPE,
        // 片假名（全角）。
        JapaneseMode::Katakana => IME_CMODE_NATIVE | IME_CMODE_KATAKANA | IME_CMODE_FULLSHAPE,
        // 全角英数。
        JapaneseMode::FullWidthAlnum => IME_CMODE_ALPHANUMERIC | IME_CMODE_FULLSHAPE,
    }
}
