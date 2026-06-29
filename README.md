# lock-ime

常驻后台的 Windows 输入法模式锁定工具。解决「微软中/日文输入法被系统自作主张切到英文/其他子模式」的困扰——理念是 **用英文输入法打英文，而不是在中文输入法里切英文子模式**。

## 功能

1. **中文锁中文模式** —— 切换输入法 / 窗口获得焦点时，把微软中文输入法强制设回中文模式（IME 开状态）。
2. **日文锁平假名** —— 同样时机把日文输入法转换模式设为平假名（可在配置里改为片假名 / 全角英数）。
3. **CapsLock 切输入法（模拟 Mac）** —— **短按** CapsLock 在「英文(US) ↔ 上一个 CJK 输入法」之间切换；**长按**（默认 >300ms）才触发真正的大写锁定。

托盘菜单可随时开关每项功能、切换开机自启、打开配置文件。

## 构建

```sh
cargo build --release
# 产物：target/release/lock-ime.exe
```

无运行时依赖，单 exe。`#![windows_subsystem = "windows"]` 已去掉控制台窗口。

## 运行

直接双击 `lock-ime.exe`，托盘出现蓝色图标即在运行。右键托盘菜单进行设置。

## 配置

首次运行自动生成 `%APPDATA%\lock-ime\config.toml`：

```toml
chinese_lock_enabled = true
japanese_lock_enabled = true
japanese_mode = "hiragana"      # hiragana | katakana | full_width_alnum
capslock_switch_enabled = true
capslock_longpress_ms = 300
autostart = false
```

改完保存后，托盘里的开关会在下次重启程序时同步；功能开关本身实时生效。

## 手动验证

- **功能#1**：切到微软拼音，手动切到英文子模式，点另一个窗口让它获得焦点 → 应自动弹回中文模式。
- **功能#2**：日文输入法切到片假名，重新聚焦输入框 → 弹回平假名（把 `japanese_mode` 改成 `katakana` 后应锁片假名）。
- **功能#3**：在文本框里 **短按** CapsLock → 在英文与中文输入法间切换；**长按** → 锁大写并能连续输入大写字母；确认无按键卡死。
- 退出程序后，所有 hook 解除，CapsLock 恢复系统默认行为。

## 已知限制 / 设计说明

- **现代微软 IME（Win11 22H2+）有时会静默忽略 IMM32 的 `WM_IME_CONTROL`**。本工具 v1 以 IMM32 打底（`no_english_mode` 验证过对微软拼音够用）；若在你的机器上实测失效，需深化 `src/ime/tsf.rs` 的 TSF compartment 路径。
- 焦点事件后会**延迟约 60ms** 再施加模式，规避 Win8+「输入模式按用户全局」对获得焦点时设置的覆盖。
- 中文用 **开/关状态**（开≈中文）、日文用**转换模式**，两者区别对待。
- **第三方输入法（搜狗 / QQ 等）** 可能同时无视 IMM32 与 TSF，不在 v1 保证范围内。

## 模块结构

| 文件 | 职责 |
|---|---|
| `src/main.rs` | 入口、隐藏消息窗口、消息循环、hook 安装/清理 |
| `src/config.rs` | TOML 配置读写 |
| `src/autostart.rs` | HKCU\...\Run 开机自启 |
| `src/lang.rs` | 键盘布局 / 语言检测 |
| `src/ime/imm32.rs` | IMM32 后端：`WM_IME_CONTROL` 控制开关/转换模式 |
| `src/ime/tsf.rs` | TSF compartment 回退（占位，待深化） |
| `src/events.rs` | 焦点/前台 WinEvent hook → 触发锁定 |
| `src/keyboard.rs` | 低级键盘 hook → CapsLock 逻辑 |
| `src/tray.rs` | 托盘图标与菜单 |

## 致谢 / 参考

- [mbbill/no_english_mode](https://github.com/mbbill/no_english_mode) —— 功能 #1 的骨架思路
- [karakaram/alt-ime-ahk](https://github.com/karakaram/alt-ime-ahk) / IME.ahk —— 日文转换模式标志位
- [Linkerin/capswitch](https://github.com/Linkerin/capswitch) —— CapsLock 低级钩子参考
