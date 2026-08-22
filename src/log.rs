//! 轻量文件日志与 panic 捕获（无第三方依赖）。
//!
//! 日志追加写入 `%APPDATA%\lock-ime\lock-ime.log`；进程崩溃（Rust 的 `panic = "abort"`
//! 会以 `0xC0000409` fail-fast 终止）时，panic 钩子把 panic 调用点 `file:line:col` 与
//! 消息一并写入同一文件 —— 无需调试符号即可定位崩溃位置。

use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn log_dir() -> std::path::PathBuf {
    crate::config::Config::path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

/// 追加一行到日志。全程不 panic（崩溃路径内也安全）。
pub fn write(msg: &str) {
    let mut guard = match LOG_FILE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.is_none() {
        let dir = log_dir();
        let _ = std::fs::create_dir_all(&dir);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("lock-ime.log"))
            .ok();
        *guard = file;
    }
    if let Some(f) = guard.as_mut() {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "[{elapsed}] [{:?}] {msg}", std::thread::current().id());
        let _ = f.flush();
    }
}

/// 登记 panic 钩子：把 panic 调用点与消息写入日志。`panic = "abort"` 下也会先执行钩子。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.clone()))
            .unwrap_or_else(|| "<non-string payload>".to_string());
        write(&format!("PANIC at {loc}: {payload}"));
    }));
}

#[macro_export]
macro_rules! logmsg {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}