<p align="center">
  <img src="img/lock-ime-logo-128.png" alt="Lock IME Logo" width="64">
</p>

# Lock IME

**将微软中文输入法锁定为中文模式，防止自动切换至英文。**

顺便也实现了日文输入法锁定，以及 Mac 风格的 CapsLock 切换输入法。


## 功能

1. **中文锁中文模式** —— 切换输入法 / 窗口获得焦点时，把微软中文输入法强制设回中文模式。
2. **日文锁平假名** —— 同样时机把日文输入法转换模式设为平假名（或片假名/罗马字等）。*（测试性功能，不一定好用）*
3. **CapsLock 切输入法（模拟 Mac）** —— **短按** CapsLock 在「英文(US) ↔ 上一个 CJK 输入法」之间切换；**长按**（默认 >300ms）才触发真正的大写锁定。

托盘菜单可随时开关每项功能、切换开机自启。


## 运行

直接双击 `lock-ime.exe`，托盘出现 🀄 图标即在运行。右键托盘菜单进行设置。


## 构建

```sh
cargo build --release
# 产物：target/release/lock-ime.exe
```

无运行时依赖，单 exe。`#![windows_subsystem = "windows"]` 已去掉控制台窗口。


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
