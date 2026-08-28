//! 系统托盘图标与菜单。

use crate::autostart;
use crate::config::{CapslockAction, JapaneseMode};
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _tray: TrayIcon,
    /// 原生菜单，仅在浮窗不可用时创建；可用时托盘不挂菜单，右键也交给浮窗。
    menu: Option<NativeMenu>,
}

/// 回退路径用的原生菜单及其条目。
struct NativeMenu {
    _menu: Menu,
    chinese: CheckMenuItem,
    japanese: CheckMenuItem,
    capslock: CheckMenuItem,
    autostart: CheckMenuItem,
    open_settings: MenuItem,
    quit: MenuItem,
}

fn japanese_label(mode: JapaneseMode) -> &'static str {
    match mode {
        JapaneseMode::Hiragana => "日文锁平假名",
        JapaneseMode::Katakana => "日文锁片假名",
        JapaneseMode::FullWidthAlnum => "日文锁全角英数",
    }
}

// 托盘图标资源：构建期由 img/lock-ime-logo.png 最近邻放大生成的像素画，
// 与应用图标同源（见 build.rs）。tray_meta.rs 定义 TRAY_W / TRAY_H。
include!(concat!(env!("OUT_DIR"), "/tray_meta.rs"));
const TRAY_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray_rgba.bin"));

fn make_icon() -> Option<Icon> {
    Icon::from_rgba(TRAY_RGBA.to_vec(), TRAY_W, TRAY_H).ok()
}

impl Tray {
    /// 依据当前配置创建托盘。浮窗可用时不建原生菜单。
    pub fn new() -> Option<Tray> {
        let mut builder = TrayIconBuilder::new().with_tooltip("lock-ime");
        if let Some(icon) = make_icon() {
            builder = builder.with_icon(icon);
        }

        // 挂了菜单，右键会被 tray-icon 的 TrackPopupMenu 抢先接管，收不到
        // TrayIconEvent；不挂则左右键都只发事件，由主循环转给浮窗。
        let menu = if crate::flyout::is_available() {
            builder = builder.with_menu_on_left_click(false);
            None
        } else {
            let m = NativeMenu::new()?;
            builder = builder.with_menu(Box::new(m._menu.clone()));
            Some(m)
        };

        Some(Tray { _tray: builder.build().ok()?, menu })
    }

    /// 从当前配置同步菜单勾选状态与日文标签。浮窗路径下无菜单可同步。
    pub fn refresh(&self) {
        if let Some(m) = &self.menu {
            m.refresh();
        }
    }

    /// 处理一次原生菜单事件。返回 true 表示请求退出程序。
    pub fn handle(&self, id: &MenuId) -> bool {
        self.menu.as_ref().is_some_and(|m| m.handle(id))
    }
}

impl NativeMenu {
    fn new() -> Option<NativeMenu> {
        let (cn, ja, caps, auto, ja_mode) = crate::state::with(|st| {
            (
                st.config.chinese_lock_enabled,
                st.config.japanese_lock_enabled,
                st.config.capslock_active(),
                st.config.autostart,
                st.config.japanese_mode,
            )
        })?;

        let chinese = CheckMenuItem::new("中文锁中文模式", true, cn, None);
        let japanese = CheckMenuItem::new(japanese_label(ja_mode), true, ja, None);
        let capslock = CheckMenuItem::new("CapsLock 切换输入法", true, caps, None);
        let autostart_item = CheckMenuItem::new("开机自启", true, auto, None);
        let open_settings = MenuItem::new("设置…", true, None);
        let quit = MenuItem::new("退出", true, None);

        let menu = Menu::new();
        let _ = menu.append(&chinese);
        let _ = menu.append(&japanese);
        let _ = menu.append(&capslock);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&autostart_item);
        let _ = menu.append(&open_settings);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);

        Some(NativeMenu {
            _menu: menu,
            chinese,
            japanese,
            capslock,
            autostart: autostart_item,
            open_settings,
            quit,
        })
    }

    fn refresh(&self) {
        let (cn, ja, caps, auto, ja_mode) = crate::state::with(|st| {
            (
                st.config.chinese_lock_enabled,
                st.config.japanese_lock_enabled,
                st.config.capslock_active(),
                st.config.autostart,
                st.config.japanese_mode,
            )
        })
        .unwrap_or_default();
        self.chinese.set_checked(cn);
        self.japanese.set_checked(ja);
        self.japanese.set_text(japanese_label(ja_mode));
        self.capslock.set_checked(caps);
        self.autostart.set_checked(auto);
    }

    fn handle(&self, id: &MenuId) -> bool {
        if id == self.quit.id() {
            return true;
        }
        if id == self.open_settings.id() {
            crate::settings_window::open();
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
                // 菜单项只有开/关两态，对短按/长按双动作是有损映射：
                // 开 = 恢复默认（短按 CJK/US、长按大写锁定），关 = 两者皆大写锁定
                // （等同系统默认行为）。精细配置走设置窗口或配置文件。
                let (short, long) = if v {
                    (CapslockAction::CjkUs, CapslockAction::CapsLock)
                } else {
                    (CapslockAction::CapsLock, CapslockAction::CapsLock)
                };
                st.config.capslock_short_action = short;
                st.config.capslock_long_action = long;
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
