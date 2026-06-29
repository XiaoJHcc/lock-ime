//! 系统托盘图标与菜单。

use crate::autostart;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _tray: TrayIcon,
    _menu: Menu,
    chinese: CheckMenuItem,
    japanese: CheckMenuItem,
    capslock: CheckMenuItem,
    autostart: CheckMenuItem,
    open_config: MenuItem,
    quit: MenuItem,
}

/// 生成一个简单的 32x32 托盘图标（蓝底白「中」框），避免外部资源文件。
fn make_icon() -> Option<Icon> {
    const N: usize = 32;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let i = (y * N + x) * 4;
            let border = x < 2 || x >= N - 2 || y < 2 || y >= N - 2;
            let inner = x >= 10 && x < 22 && y >= 6 && y < 26;
            let (r, g, b) = if border {
                (20, 90, 200)
            } else if inner {
                (255, 255, 255)
            } else {
                (30, 110, 230)
            };
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32).ok()
}

impl Tray {
    /// 依据当前配置创建托盘。
    pub fn new() -> Option<Tray> {
        let (cn, ja, caps, auto) = crate::state::with(|st| {
            (
                st.config.chinese_lock_enabled,
                st.config.japanese_lock_enabled,
                st.config.capslock_switch_enabled,
                st.config.autostart,
            )
        })?;

        let chinese = CheckMenuItem::new("中文锁中文模式", true, cn, None);
        let japanese = CheckMenuItem::new("日文锁平假名", true, ja, None);
        let capslock = CheckMenuItem::new("CapsLock 切换输入法", true, caps, None);
        let autostart_item = CheckMenuItem::new("开机自启", true, auto, None);
        let open_config = MenuItem::new("打开配置文件", true, None);
        let quit = MenuItem::new("退出", true, None);

        let menu = Menu::new();
        let _ = menu.append(&chinese);
        let _ = menu.append(&japanese);
        let _ = menu.append(&capslock);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&autostart_item);
        let _ = menu.append(&open_config);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);

        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_tooltip("lock-ime");
        if let Some(icon) = make_icon() {
            builder = builder.with_icon(icon);
        }
        let tray = builder.build().ok()?;

        Some(Tray {
            _tray: tray,
            _menu: menu,
            chinese,
            japanese,
            capslock,
            autostart: autostart_item,
            open_config,
            quit,
        })
    }

    /// 处理一次菜单事件。返回 true 表示请求退出程序。
    pub fn handle(&self, id: &MenuId) -> bool {
        if id == self.quit.id() {
            return true;
        }
        if id == self.open_config.id() {
            let path = crate::config::Config::path();
            // 确保文件存在后用默认程序打开。
            crate::state::with(|st| {
                let _ = st.config.save();
            });
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &path.to_string_lossy()])
                .spawn();
            return false;
        }

        if id == self.chinese.id() {
            let v = self.chinese.is_checked();
            crate::state::with(|st| {
                st.config.chinese_lock_enabled = v;
                let _ = st.config.save();
            });
        } else if id == self.japanese.id() {
            let v = self.japanese.is_checked();
            crate::state::with(|st| {
                st.config.japanese_lock_enabled = v;
                let _ = st.config.save();
            });
        } else if id == self.capslock.id() {
            let v = self.capslock.is_checked();
            crate::state::with(|st| {
                st.config.capslock_switch_enabled = v;
                let _ = st.config.save();
            });
        } else if id == self.autostart.id() {
            let v = self.autostart.is_checked();
            autostart::set_autostart(v);
            crate::state::with(|st| {
                st.config.autostart = v;
                let _ = st.config.save();
            });
        }
        false
    }
}
