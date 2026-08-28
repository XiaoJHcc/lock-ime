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

/// CapsLock 短按/长按的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapslockAction {
    /// CJK ↔ 英文 二态切换。
    CjkUs,
    /// 顺序循环切换（等同 Win+Space）。
    Cycle,
    /// 大写锁定（合成一次真正的 CapsLock，即系统默认行为）。
    CapsLock,
}

impl Default for CapslockAction {
    fn default() -> Self {
        CapslockAction::CjkUs
    }
}

/// 旧配置中 CapsLock 短按的切换表现。**仅供迁移旧配置反序列化**，新代码用 `CapslockAction`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapslockSwitchMode {
    CjkUs,
    Cycle,
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
    /// 功能#3：CapsLock 短按动作。
    pub capslock_short_action: CapslockAction,
    /// CapsLock 长按动作。
    pub capslock_long_action: CapslockAction,
    /// CapsLock 长按多少毫秒判定为长按。
    pub capslock_longpress_ms: u64,
    /// 开机自启。
    pub autostart: bool,
    /// 旧配置迁移用，仅反序列化；落盘时不写回。
    #[serde(skip_serializing)]
    capslock_switch_enabled: Option<bool>,
    /// 旧配置迁移用，仅反序列化；落盘时不写回。
    #[serde(skip_serializing)]
    capslock_switch_mode: Option<CapslockSwitchMode>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            chinese_lock_enabled: true,
            japanese_lock_enabled: true,
            japanese_mode: JapaneseMode::Hiragana,
            capslock_short_action: CapslockAction::CjkUs,
            capslock_long_action: CapslockAction::CapsLock,
            capslock_longpress_ms: 300,
            autostart: false,
            capslock_switch_enabled: None,
            capslock_switch_mode: None,
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
            Ok(text) => toml::from_str(&text)
                .map(|cfg| Self::migrate(cfg))
                .unwrap_or_else(|_| {
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

    /// CapsLock 功能是否生效：短按与长按都是「大写锁定」时等同系统默认行为，无需拦截。
    pub fn capslock_active(&self) -> bool {
        !(self.capslock_short_action == CapslockAction::CapsLock
            && self.capslock_long_action == CapslockAction::CapsLock)
    }

    /// 旧配置（capslock_switch_enabled/mode）迁移到 short/long 动作字段，并落盘一次。
    ///
    /// 旧字段只会出现在旧版本写出的文件里（新版本序列化时已跳过），故文件里出现旧字段
    /// 即可断定新字段缺席、正在使用默认值，直接按旧语义覆盖：
    /// enabled=true → 短按=旧 mode、长按=大写锁定；enabled=false → 两者皆大写锁定。
    fn migrate(mut cfg: Config) -> Config {
        let legacy = match (cfg.capslock_switch_enabled, cfg.capslock_switch_mode) {
            (Some(enabled), mode) => Some((enabled, mode.unwrap_or(CapslockSwitchMode::CjkUs))),
            (None, Some(mode)) => Some((true, mode)),
            (None, None) => None,
        };
        if let Some((enabled, mode)) = legacy {
            if enabled {
                cfg.capslock_short_action = match mode {
                    CapslockSwitchMode::CjkUs => CapslockAction::CjkUs,
                    CapslockSwitchMode::Cycle => CapslockAction::Cycle,
                };
                cfg.capslock_long_action = CapslockAction::CapsLock;
            } else {
                cfg.capslock_short_action = CapslockAction::CapsLock;
                cfg.capslock_long_action = CapslockAction::CapsLock;
            }
            cfg.capslock_switch_enabled = None;
            cfg.capslock_switch_mode = None;
            let _ = cfg.save();
        }
        cfg
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
