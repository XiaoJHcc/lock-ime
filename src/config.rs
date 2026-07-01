//! 配置文件：读写 `%APPDATA%\lock-ime\config.toml`。

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 日文输入法要锁定的转换模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JapaneseMode {
    /// 平假名（全角）。
    Hiragana,
    /// 片假名（全角）。
    Katakana,
    /// 全角英数。
    FullWidthAlnum,
}

impl Default for JapaneseMode {
    fn default() -> Self {
        JapaneseMode::Hiragana
    }
}

/// CapsLock 短按的切换表现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapslockSwitchMode {
    /// CJK ↔ 英文 二态切换。
    CjkUs,
    /// 顺序循环切换（等同 Win+Space）。
    Cycle,
}

impl Default for CapslockSwitchMode {
    fn default() -> Self {
        CapslockSwitchMode::CjkUs
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 功能#1：中文输入法获得焦点/切换时锁定为中文模式。
    pub chinese_lock_enabled: bool,
    /// 功能#2：日文输入法锁定为指定转换模式。
    pub japanese_lock_enabled: bool,
    /// 日文锁定的目标模式。
    pub japanese_mode: JapaneseMode,
    /// 功能#3：CapsLock 短按切换输入法。
    pub capslock_switch_enabled: bool,
    /// CapsLock 短按的切换表现。
    pub capslock_switch_mode: CapslockSwitchMode,
    /// CapsLock 长按多少毫秒判定为「锁大写」。
    pub capslock_longpress_ms: u64,
    /// 开机自启。
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            chinese_lock_enabled: true,
            japanese_lock_enabled: true,
            japanese_mode: JapaneseMode::Hiragana,
            capslock_switch_enabled: true,
            capslock_switch_mode: CapslockSwitchMode::CjkUs,
            capslock_longpress_ms: 300,
            autostart: false,
        }
    }
}

impl Config {
    /// 配置文件完整路径。
    pub fn path() -> PathBuf {
        if let Some(dirs) = ProjectDirs::from("", "", "lock-ime") {
            dirs.config_dir().join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    /// 加载配置；文件不存在或解析失败时返回默认值并尝试写回默认文件。
    pub fn load() -> Config {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|_| {
                let cfg = Config::default();
                let _ = cfg.save();
                cfg
            }),
            Err(_) => {
                let cfg = Config::default();
                let _ = cfg.save();
                cfg
            }
        }
    }

    /// 保存配置到磁盘。
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&path, text)
    }
}
