//! Win11 风格托盘浮窗：用 WinUI 3（XAML Islands）绘制，设置页直接做进浮窗。
//!
//! 走的是「调用系统原生 UI」而非手工仿造：控件、亚克力、圆角、动画全部由
//! WindowsAppRuntime 框架包提供，本进程只增约 150KB，且不引入 .NET。
//!
//! 运行时不存在时（未装框架包的旧系统）`init` 返回 false，调用方回退到原生菜单
//! + Win32 设置窗口（见 settings_window.rs）。
//!
//! # 构建顺序不可调换
//! `Create → XamlSource::Initialize → SetContent → SetSystemBackdrop
//!  → Show → 取 HWND → 窗口外观/失焦监听`
//!
//! 两处顺序踩过坑：
//!  * backdrop 必须在 `SetContent` 之后——内容树为空时挂 backdrop 会**静默段错误**；
//!  * `GetWindowFromWindowId` 必须在 `Show` 之后——窗口未显示时 HWND 尚未实体化，
//!    返回 `E_POINTER`。

use crate::config::{CapslockAction, JapaneseMode};
use std::cell::{Cell, RefCell};
use windows::core::{h, Interface, Ref, Result, BOOL, HSTRING};
use windows::Foundation::{IReference, PropertyValue, TimeSpan, TypedEventHandler};
use windows::Graphics::{RectInt32, SizeInt32};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClientRect, KillTimer, PostQuitMessage, SetForegroundWindow,
    SetTimer, SetWindowPos, SystemParametersInfoW, SPI_GETWORKAREA, PostMessageW,
    SWP_NOACTIVATE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WA_INACTIVE,
    WM_ACTIVATE, WM_USER,
};
use winui3::bootstrap::PackageDependency;
use winui3::Microsoft::UI::Dispatching::DispatcherQueueController;
use winui3::Microsoft::UI::Windowing::{AppWindow, OverlappedPresenter};
use winui3::Microsoft::UI::Xaml::Controls::Primitives::{
    RangeBaseValueChangedEventArgs, RangeBaseValueChangedEventHandler,
};
use winui3::Microsoft::UI::Xaml::Controls::{
    AppBarButton, Border, ColumnDefinition, ComboBox, ComboBoxItem, CommandBarLabelPosition,
    Expander, ExpanderCollapsedEventArgs, ExpanderExpandingEventArgs, FontIcon, Grid, Orientation,
    SelectionChangedEventHandler, Slider, StackPanel, TextBlock, ToggleSwitch,
    XamlControlsResources,
};
use winui3::Microsoft::UI::Xaml::Hosting::{DesktopWindowXamlSource, WindowsXamlManager};
use winui3::Microsoft::UI::Xaml::Media::Animation::{
    CubicEase, DoubleAnimation, EasingMode, Storyboard, Timeline,
};
use winui3::Microsoft::UI::Xaml::Media::{Brush, CompositeTransform, DesktopAcrylicBackdrop};
use winui3::Microsoft::UI::Xaml::{
    Application, CornerRadius, Duration, DurationType, FrameworkElement, GridLength, GridUnitType,
    HorizontalAlignment, LaunchActivatedEventArgs, RoutedEventHandler, Thickness, UIElement,
    VerticalAlignment,
};
use winui3::{XamlApp, XamlAppOverrides};

/// 面板逻辑尺寸（96dpi 基准），高度按下列常量累加得出。
///
/// 不做运行时测量：`DesktopWindowXamlSource` 的内容树量不出可用的高度
/// （手动 `Measure` 早于模板套用、`ActualHeight` 是布局后的值、`DesiredSize` 返回 0），
/// 而各部分的尺寸本就来自 WinUI 主题资源里的固定值，直接累加即可。
/// 改布局时同步改这里。
const PANEL_W: i32 = 320;

/// 设置行高：ComboBox 的 `MinHeight`（32）上下各留 4。
const ROW_H: i32 = 40;
/// 卡片上下内边距各 10，见 make_card。
const CARD_PAD_V: i32 = 10;
/// 普通单行卡片总高（中文锁定/开机自启）：行高 + 上下内边距 + 上下 1px 描边。
/// 描边在内边距外侧（Border 的 Padding 不含 BorderThickness），漏算它会让面板偏矮、
/// 最下面一张卡的底边描边被窗口下缘裁掉。
const CARD_H: i32 = CARD_PAD_V * 2 + ROW_H + 2;
/// 展开区行高：与系统设置展开列表的单行行高一致（如 系统›屏幕›多显示器）。
const EXP_ROW_H: i32 = 48;
/// Expander 展开区水平内边距：模板取 `ExpanderContentPadding` = 16 四边等值，
/// 但上下已被 make_caps_card 里的 `SetPadding(16,0,16,0)` 清零——行高 48 内部
/// 控件居中已自带 8px 空隙，再叠 16px 模板内边距会让首行上方/末行下方的留白
/// 明显大于行间。水平 16 保留（行内容恰好与头部文字左对齐）；
/// 分割线用等值负外边距抵消它，达到系统设置那种通栏分割线的效果。
const EXP_CONTENT_PAD: f64 = 16.0;
/// Expander 展开区总高：3 行 + 2 条分割线 + 内容区底边描边 1（上下内边距已清零）。
const EXPANDED_H: i32 = EXP_ROW_H * 3 + 2 + 1;
/// 标题行：FontSize 14 的单行文本约 20，加下边距 2。
const TITLE_H: i32 = 22;
/// 标题与各卡片间距，见 make_content 里 panel 的 Spacing。
const CARD_GAP: i32 = 8;
/// 内容区上下内边距各 12，见 make_content 里 content 的 Padding。
const CONTENT_PAD_V: i32 = 12;
/// 底栏高度：`AppBarThemeCompactHeight`。
const FOOTER_H: i32 = 48;
/// 内容区底边分隔线 1px。
const SEP_H: i32 = 1;

/// 面板高度 = 内容区 + 分隔线 + 底栏。`expanded` 为 CapsLock 卡片展开与否。
///
/// CapsLock 卡折叠态与普通卡片同为 CARD_H（头部 MinHeight 含自身描边，见
/// make_caps_card），故直接按四张 CARD_H 累加。
const fn panel_h(expanded: bool) -> i32 {
    let caps = CARD_H + if expanded { EXPANDED_H } else { 0 };
    CONTENT_PAD_V * 2 + TITLE_H + CARD_GAP * 4 + CARD_H * 3 + caps + SEP_H + FOOTER_H
}
/// 面板与托盘图标之间的间距（逻辑像素）。
const GAP: i32 = 8;

/// 长按阈值（毫秒）的取值范围与拉条步进。
const THRESH_MIN: f64 = 100.0;
const THRESH_MAX: f64 = 2000.0;
const THRESH_STEP: f64 = 50.0;

/// 下拉框定宽：日文模式下拉框用它；容下「全角英数」且不与开关挤占标签。
///
/// 定宽：系统设置里的下拉框不随布局拉伸，固定宽度让各卡片右侧控件边缘对齐。
const COMBO_W: f64 = 120.0;
/// CapsLock 动作下拉框定宽：需完整容下最长选项「CJK / US 切换」（实测约需 131）。
/// 所在行只有短标签，空间充裕；比 COMBO_W 宽无碍。阈值拉条取同宽，
/// 让展开区内右侧控件的左右边缘全部对齐。
const CAPS_COMBO_W: f64 = 136.0;

/// CapsLock 动作下拉框的三个选项，下标即 ComboBox 的 SelectedIndex。
const CAPS_ACTIONS: [CapslockAction; 3] = [
    CapslockAction::CjkUs,
    CapslockAction::Cycle,
    CapslockAction::CapsLock,
];
const CAPS_ACTION_LABELS: [&str; 3] = ["CJK / US 切换", "正常循环", "大写锁定"];

fn action_index(a: CapslockAction) -> i32 {
    CAPS_ACTIONS.iter().position(|x| *x == a).unwrap_or(0) as i32
}

fn action_at(idx: i32) -> CapslockAction {
    CAPS_ACTIONS.get(idx.max(0) as usize).copied().unwrap_or_default()
}

fn japanese_index(mode: JapaneseMode) -> i32 {
    match mode {
        JapaneseMode::Hiragana => 0,
        JapaneseMode::Katakana => 1,
        JapaneseMode::FullWidthAlnum => 2,
    }
}

fn japanese_at(idx: i32) -> JapaneseMode {
    match idx {
        0 => JapaneseMode::Hiragana,
        1 => JapaneseMode::Katakana,
        _ => JapaneseMode::FullWidthAlnum,
    }
}

/// 面板中的设置控件。
struct Items {
    chinese: ToggleSwitch,
    japanese: ToggleSwitch,
    japanese_mode: ComboBox,
    caps_card: Expander,
    caps_short: ComboBox,
    caps_long: ComboBox,
    threshold_slider: Slider,
    autostart: ToggleSwitch,
}

impl Items {
    /// 从当前配置回写所有控件。
    ///
    /// 只能在控件已进可视化树（`SetContent` 之后）调用：与旧的 CheckBox 勾标同理，
    /// 程序化写入需在模板套用后进行；写入期间用 `with_syncing` 压住事件回调，
    /// 否则 SetIsOn/SetSelectedIndex 触发的 Toggled/SelectionChanged 会把刚读出来的值
    /// 又写回配置（无害但落盘一次）。
    fn sync(&self) {
        let Some((cn, ja, ja_mode, auto, short, long, lp)) = crate::state::with(|st| {
            (
                st.config.chinese_lock_enabled,
                st.config.japanese_lock_enabled,
                st.config.japanese_mode,
                st.config.autostart,
                st.config.capslock_short_action,
                st.config.capslock_long_action,
                st.config.capslock_longpress_ms,
            )
        }) else {
            return;
        };
        with_syncing(|| {
            let _ = self.chinese.SetIsOn(cn);
            let _ = self.japanese.SetIsOn(ja);
            let _ = self.japanese_mode.SetSelectedIndex(japanese_index(ja_mode));
            let _ = self.caps_short.SetSelectedIndex(action_index(short));
            let _ = self.caps_long.SetSelectedIndex(action_index(long));
            let _ = self.threshold_slider.SetValue2(lp as f64);
            let _ = self.autostart.SetIsOn(auto);
        });
    }
}

/// ease-out cubic。
fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

struct Flyout {
    // 以下三个句柄必须保活到进程结束：drop 会卸载运行时/XAML 上下文。
    _dep: PackageDependency,
    _dqc: DispatcherQueueController,
    _mgr: WindowsXamlManager,
    win: AppWindow,
    _src: DesktopWindowXamlSource,
    hwnd: HWND,
    items: Items,
    /// 根容器：收起时切为贴底（Bottom），下部内容屏幕位置在塌缩前后天然不变。
    root: StackPanel,
    /// 上部内容组（标题+卡1+卡2）与 CapsLock 卡的 RenderTransform：
    /// 展开时由 storyboard 驱动（合成线程插值），呈现「上面部分上移」。
    header_xform: CompositeTransform,
    caps_xform: CompositeTransform,
    /// 展开位移动画的 storyboard（收起/隐藏时若还在播要停掉）。
    expand_sb: Option<Storyboard>,
    visible: bool,
    /// 上次显示时锚定的托盘图标矩形，展开/收起时据此重新定位。
    anchor: Option<tray_icon::Rect>,
    /// 上次收起的时刻，用于识别「点托盘收起」这一手势，见 `toggle_at`。
    hidden_at: Option<std::time::Instant>,
    /// 收起动画的起点时刻（None = 无收起动画；展开由 storyboard 自驱，无需 timer）。
    collapse_start: Option<std::time::Instant>,
}

thread_local! {
    static FLYOUT: RefCell<Option<Flyout>> = const { RefCell::new(None) };
    /// 程序化写控件期间置位，控件事件回调据此跳过（防写回与双向同步回环）。
    static SYNCING: Cell<bool> = const { Cell::new(false) };
}

fn is_syncing() -> bool {
    SYNCING.with(|c| c.get())
}

fn with_syncing(f: impl FnOnce()) {
    SYNCING.with(|c| c.set(true));
    f();
    SYNCING.with(|c| c.set(false));
}

/// 浮窗是否可用（WindowsAppRuntime 已就位且面板已建好）。
pub fn is_available() -> bool {
    FLYOUT.with(|f| f.borrow().is_some())
}

/// 启动时调用一次：引导运行时并预建面板。返回 false 表示需回退到原生菜单。
///
/// 预建而非按需建，是为了把约 40ms 的首帧开销挪到启动阶段，
/// 让点击托盘时只剩 `Show`（约 10ms，无感）。
pub fn init() -> bool {
    match build() {
        Ok(f) => {
            FLYOUT.with(|c| *c.borrow_mut() = Some(f));
            crate::logmsg!("flyout: WinUI 3 panel ready");
            true
        }
        Err(e) => {
            crate::logmsg!(
                "flyout: unavailable (0x{:08X}), fallback to native menu",
                e.code().0
            );
            false
        }
    }
}

/// 托盘被点击：面板已显示则隐藏，否则移到托盘图标上方并显示。
///
/// `tray_rect` 为 `Shell_NotifyIconGetRect` 返回的**物理像素**矩形。
pub fn toggle_at(tray_rect: tray_icon::Rect) {
    /// 「刚刚收起」的判定窗口。
    ///
    /// 面板显示时点托盘，任务栏会先抢走焦点，`WM_ACTIVATE` 抢在托盘事件之前
    /// 把面板收起；等托盘事件到达，`visible` 已是 false，单看它会把这一下
    /// 判成「打开」，于是面板闪一下又弹回来。落在这个窗口内的托盘点击
    /// 视为那次收起的后续，不再打开。
    ///
    /// 取 300ms：足够覆盖失焦到托盘事件之间的间隔（实测在几毫秒量级，
    /// 但要留出系统繁忙时的余量），又短于人有意「关掉再打开」的最快节奏。
    const DISMISS_GRACE: std::time::Duration = std::time::Duration::from_millis(300);

    FLYOUT.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(f) = borrow.as_mut() else { return };
        if f.visible {
            f.hide();
            return;
        }
        if f.hidden_at.is_some_and(|t| t.elapsed() < DISMISS_GRACE) {
            return; // 这一下点击就是刚才那次收起的起因。
        }
        f.show_at(tray_rect);
    });
}

/// 隐藏面板（失焦、执行菜单项后调用）。
pub fn hide() {
    FLYOUT.with(|c| {
        if let Some(f) = c.borrow_mut().as_mut() {
            f.hide();
        }
    });
}

/// 从当前配置同步全部控件（设置窗口改动后调用）。
pub fn refresh() {
    FLYOUT.with(|c| {
        if let Some(f) = c.borrow().as_ref() {
            f.items.sync();
        }
    });
}

/// CapsLock 卡片展开/收起：驱动窗口动画（保持右缘与底边锚定）。
///
/// 核心约束：XAML 岛场景下**逐帧 resize HWND 必然抽搐**——窗口尺寸走 DWM
/// 合成，内容走 XAML swapchain，两条管线不同步，内容每帧重绘都滞后一拍。
/// 因此窗口只在状态切换时一次到位，动画全部放在内容层（渲染级 RenderTransform
/// 与模板 clip，均为合成线程处理）。
///
///  * 展开——窗口一次长高到位（顶边外原本是透明区，瞬现的只是空白亚克力）；
///    「标题+卡1+卡2+CapsLock 卡」的 RenderTransform 由 **storyboard**（合成线程
///    按 vsync 插值，非 UI 线程逐帧 push）从 +EXPANDED_H 缓动到 0，呈现「上面
///    部分上移」；展开区由模板 clip 揭开；下部内容布局位置不变，天然钉住。
///  * 收起——内容贴底（模板塌缩前后下部内容屏幕位置不变）；塌缩（0.2s）落定后
///    窗口逐帧收缩，期间 XAML 岛子窗口**保持全高、贴父窗口底边**（父窗口相当于
///    视口，顶边下移只是裁掉岛的上部空白）——岛尺寸不变，内容零重绘，
///    底栏与卡片纹丝不动。
///
/// 勿回退的弯路：逐帧 resize + 内容跟随（管线不同步抽搐）；UI 线程 16ms timer
/// 逐帧 SetTranslateY（消息循环帧间隔不均，动画抖动）；ImplicitAnimations
/// （下部内容在屏幕上平移而非钉住）。
fn on_expand_state(expanded: bool) {
    FLYOUT.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(f) = borrow.as_mut() else { return };
        if !f.visible || f.anchor.is_none() {
            return;
        }
        if expanded {
            f.stop_collapse();
            let _ = f.root.SetVerticalAlignment(VerticalAlignment::Top);
            let anchor = f.anchor.unwrap();
            f.place(&anchor, f.phys_h(true), IslandPos::Fill); // 窗口一次到位
            f.play_expand_slide();
        } else {
            f.stop_expand_slide(); // 展开动画在播则停（TranslateY 归位 0）
            let _ = f.root.SetVerticalAlignment(VerticalAlignment::Bottom);
            f.collapse_start = Some(std::time::Instant::now());
            crate::state::with(|st| unsafe {
                SetTimer(Some(st.hidden_hwnd), crate::TIMER_FLYOUT_ANIM, 16, None);
            });
        }
    });
}

/// 收起动画帧（hidden 窗口 WM_TIMER 周期驱动，见 main.rs）。
pub fn on_anim_tick() {
    FLYOUT.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(f) = borrow.as_mut() else { return };
        f.collapse_tick();
    });
}

impl Flyout {
    /// 物理高度（按窗口所在 DPI 换算）。
    fn phys_h(&self, expanded: bool) -> i32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as i32;
        panel_h(expanded) * dpi / 96
    }

    fn hide(&mut self) {
        self.stop_collapse();
        self.stop_expand_slide();
        let _ = self.win.Hide();
        self.visible = false;
        self.hidden_at = Some(std::time::Instant::now());
    }

    /// 依托盘图标位置定位并显示。坐标全程用物理像素，与 `tray_rect` 一致。
    fn show_at(&mut self, tray_rect: tray_icon::Rect) {
        // 保险：停掉上次未播完的动画，位移补偿与对齐复位。
        self.stop_collapse();
        self.stop_expand_slide();
        let _ = self.root.SetVerticalAlignment(VerticalAlignment::Top);
        self.layout_at(&tray_rect, self.items.caps_card.IsExpanded().unwrap_or(false));
        self.anchor = Some(tray_rect);

        // 定好位再显示，避免弹出瞬间先在上一次的旧位置闪一下。
        if self.win.ShowWithActivation(true).is_err() {
            return;
        }
        self.visible = true;

        // ShowWithActivation 抢不过任务栏的前台锁定：托盘点击后前台仍是
        // explorer，面板始终非激活，失焦判定会立刻把它收起。必须显式夺取。
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// 按展开状态确定尺寸并定位：右缘对齐托盘图标，底边贴在图标上方。
    fn layout_at(&self, tray_rect: &tray_icon::Rect, expanded: bool) {
        self.place(tray_rect, self.phys_h(expanded), IslandPos::Fill);
    }

    /// 把窗口放到指定物理高度：宽不变，右缘对齐托盘图标，底边贴在图标上方。
    ///
    /// 位置与尺寸用一次 `MoveAndResize` 原子完成——拆成 ResizeClient + Move 两步时，
    /// DWM 可能在两步之间合成出「尺寸已变、位置未变」的中间帧。
    fn place(&self, tray_rect: &tray_icon::Rect, ph: i32, island: IslandPos) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as i32;
        let s = |v: i32| v * dpi / 96;
        let pw = s(PANEL_W);

        let icon_right = tray_rect.position.x as i32 + tray_rect.size.width as i32;
        let mut x = icon_right - pw;
        let mut y = tray_rect.position.y as i32 - ph - s(GAP);

        // 钳制到工作区，避免被任务栏遮挡或跑出屏幕（任务栏在侧边/顶部时同样成立）。
        if let Some(wa) = work_area() {
            x = x.clamp(wa.left, (wa.right - pw).max(wa.left));
            y = y.clamp(wa.top, (wa.bottom - ph).max(wa.top));
        }
        // 本窗口无标题栏/边框（BorderAndTitleBar 已关），外框即客户区，
        // MoveAndResize 的尺寸语义差异在此不成立。
        if let Err(e) = self.win.MoveAndResize(RectInt32 {
            X: x,
            Y: y,
            Width: pw,
            Height: ph,
        }) {
            crate::logmsg!("flyout: MoveAndResize failed 0x{:08X}", e.code().0);
        }
        unsafe { position_island(self.hwnd, island, self.phys_h(true)) }
    }

    /// 展开位移动画：storyboard 驱动两个 RenderTransform 的 TranslateY
    /// 从 +EXPANDED_H 缓动到 0（合成线程插值，不经消息循环）。
    fn play_expand_slide(&mut self) {
        if let Some(sb) = &self.expand_sb {
            let _ = sb.Stop();
        }
        match self.build_expand_slide() {
            Ok(sb) => self.expand_sb = Some(sb),
            Err(e) => {
                crate::logmsg!("flyout: expand slide failed 0x{:08X}", e.code().0);
                self.set_ty(0.0); // 退化：直接归位，无动画
                self.expand_sb = None;
            }
        }
    }

    fn build_expand_slide(&self) -> Result<Storyboard> {
        let sb = Storyboard::new()?;
        for xform in [&self.header_xform, &self.caps_xform] {
            let anim = DoubleAnimation::new()?;
            anim.SetFrom(
                &PropertyValue::CreateDouble(f64::from(EXPANDED_H))?.cast::<IReference<f64>>()?,
            )?;
            anim.SetTo(&PropertyValue::CreateDouble(0.0)?.cast::<IReference<f64>>()?)?;
            anim.cast::<Timeline>()?.SetDuration(Duration {
                TimeSpan: TimeSpan { Duration: 333 * 10_000 }, // 对齐模板 ExpandDown
                Type: DurationType::TimeSpan,
            })?;
            let ease = CubicEase::new()?;
            ease.SetEasingMode(EasingMode::EaseOut)?;
            anim.SetEasingFunction(&ease)?;
            Storyboard::SetTarget(&anim, xform)?;
            Storyboard::SetTargetProperty(&anim, h!("TranslateY"))?;
            sb.Children()?.Append(&anim)?;
        }
        sb.Begin()?;
        Ok(sb)
    }

    fn stop_expand_slide(&mut self) {
        if let Some(sb) = self.expand_sb.take() {
            let _ = sb.Stop();
        }
        self.set_ty(0.0);
    }

    /// 收起动画帧：前 210ms 等模板塌缩落定，之后逐帧收缩窗口（岛贴底不动）。
    fn collapse_tick(&mut self) {
        let Some(start) = self.collapse_start else { return };
        let Some(anchor) = self.anchor else {
            self.stop_collapse();
            return;
        };
        let el = start.elapsed().as_secs_f64();
        if el < 0.21 {
            return; // 内容滑出（0.167s）+ 塌缩（0.2s）落定前不动窗口
        }
        let t = ((el - 0.21) / 0.167).min(1.0); // 对齐模板 CollapseDown 时长
        let done = t >= 1.0;
        // 收缩期间岛保持全高贴底（内容零重绘）；落定帧恢复铺满。
        let island = if done { IslandPos::Fill } else { IslandPos::AnchorBottom };
        let collapsed = self.phys_h(false);
        let delta = self.phys_h(true) - collapsed;
        self.place(
            &anchor,
            collapsed + (delta as f64 * (1.0 - ease_out(t))).round() as i32,
            island,
        );
        if done {
            self.stop_collapse();
        }
    }

    fn stop_collapse(&mut self) {
        if self.collapse_start.take().is_some() {
            crate::state::with(|st| unsafe {
                let _ = KillTimer(Some(st.hidden_hwnd), crate::TIMER_FLYOUT_ANIM);
            });
        }
    }

    /// 设置上部内容组与 CapsLock 卡的渲染级垂直位移（逻辑像素）。
    fn set_ty(&self, ty: f64) {
        let _ = self.header_xform.SetTranslateY(ty);
        let _ = self.caps_xform.SetTranslateY(ty);
    }
}

/// 仅用于建立带样式 provider 的 Application 上下文；窗口由本模块自己建，
/// 故 OnLaunched 无需做任何事（也不会调用 Application::Start）。
struct NullApp;

impl XamlAppOverrides for NullApp {
    fn OnLaunched(
        &self,
        _base: &Application,
        _args: Option<&LaunchActivatedEventArgs>,
    ) -> Result<()> {
        Ok(())
    }
}

/// 子类化标识。同一窗口可挂多个子类，靠这个 id 区分。
const SUBCLASS_ID: usize = 1;

/// 自定义消息：收起浮窗。
///
/// 用于把 `Hide` 挪出 `WM_ACTIVATE` 的处理过程——在窗口过程内部同步调
/// `AppWindow::Hide` 会重入窗口管理逻辑。
const WM_FLYOUT_DISMISS: u32 = WM_USER + 1;

/// 浮窗窗口过程的子类化钩子：失焦即收起。
///
/// `WM_ACTIVATE` 的 wParam 低位为 `WA_INACTIVE` 表示本窗口正在失去激活。
/// 这是 Win32 层面的判据，不经 WinUI 的激活树，因而不受
/// 「Hide 之后再也回不到 Activated」那个行为的影响。
///
/// 收起动作经 `WM_FLYOUT_DISMISS` 绕一手，原因见该常量。
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_ACTIVATE && (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_FLYOUT_DISMISS, WPARAM(0), LPARAM(0));
        }
    }
    if msg == WM_FLYOUT_DISMISS {
        hide();
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// 给浮窗加上 Win11 的圆角与边框。
///
/// `OverlappedPresenter::CreateForContextMenu` 只提供行为，视觉外框要自己向 DWM 要。
/// 这是官方文档 apply-rounded-corners 的 Example 4「Rounding the corners of a menu」
/// 所用的方案：
///  * `DWMWCP_ROUND` —— 标准圆角（8px），与系统输入法面板、右键菜单等浮窗一致。
///    半径由 DWM 内部规定、不可指定数值：ROUND=8、ROUNDSMALL=4，只能二选一，
///    这也是系统所有窗口圆角能保持统一的原因；
///  * `DWMWA_BORDER_COLOR` = `DWMWA_COLOR_DEFAULT` —— 交还系统绘制边框，
///    亮暗主题下自动取对应颜色。
///
/// 必须在 HWND 实体化之后调用。失败不致命（旧系统退化为直角无边框）。
fn apply_window_chrome(hwnd: HWND) {
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
        let color = DWMWA_COLOR_DEFAULT;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &color as *const _ as *const _,
            std::mem::size_of_val(&color) as u32,
        );
    }
}

/// 主显示器工作区（已扣除任务栏），物理像素。
fn work_area() -> Option<RECT> {
    let mut rc = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok().then_some(rc)
}

/// XAML 岛子窗口的摆位方式。
#[derive(Clone, Copy)]
enum IslandPos {
    /// 铺满父窗口客户区（常规状态）。
    Fill,
    /// 岛保持展开态全高、贴父窗口底边：父窗口收缩时相当于视口，顶边下移
    /// 只是裁掉岛的上部（贴底布局下那里是空白）。岛尺寸不变 → 内容零重绘
    /// 零重排，这是收起动画底栏不抽搐的关键。
    AnchorBottom,
}

/// 摆放 XAML 岛的输入站点子窗口。
///
/// `DesktopWindowXamlSource` 的内容宿主在一个子窗口（InputSiteWindowClass）里，
/// 它只在首次显示时取父窗口客户区尺寸；之后窗口尺寸变化它也不跟随，
/// 需要手动摆放。`island_h` 为 AnchorBottom 模式下的岛高（物理像素）。
/// 正常情况只有一个子窗口；重复调用无害。
unsafe fn position_island(hwnd: HWND, mode: IslandPos, island_h: i32) {
    let mut rc = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
        return;
    }
    let layout = IslandLayout {
        rc,
        mode,
        island_h,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(enum_child_layout),
            LPARAM(&layout as *const IslandLayout as isize),
        );
    }
}

struct IslandLayout {
    rc: RECT,
    mode: IslandPos,
    island_h: i32,
}

unsafe extern "system" fn enum_child_layout(child: HWND, lparam: LPARAM) -> BOOL {
    let l = unsafe { &*(lparam.0 as *const IslandLayout) };
    let (w, h) = (l.rc.right - l.rc.left, l.rc.bottom - l.rc.top);
    let (y, h) = match l.mode {
        IslandPos::Fill => (0, h),
        // 贴底：岛高固定，顶部（客户区高 - 岛高，可能为负）超出部分被父窗口裁掉。
        IslandPos::AnchorBottom => (h - l.island_h, l.island_h),
    };
    unsafe {
        let _ = SetWindowPos(child, None, 0, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    BOOL(1)
}

/// 从 Application 资源字典取主题画刷并应用。
///
/// 这些键在亮/暗主题下自动解析成不同的值，配色随系统主题切换，无需自维护调色板。
/// 查不到时只记日志、不算失败：控件退化为无背景/无描边仍可用，
/// 为一个配色让整个面板建不起来不划算。
fn set_brush<F>(key: &str, apply: F) -> Result<()>
where
    F: FnOnce(&Brush) -> Result<()>,
{
    let boxed = PropertyValue::CreateString(&HSTRING::from(key))?;
    let brush = Application::Current()
        .and_then(|a| a.Resources())
        .and_then(|r| r.Lookup(&boxed))
        .and_then(|v| v.cast::<Brush>());
    match brush {
        Ok(b) => apply(&b),
        Err(e) => {
            crate::logmsg!("flyout: brush '{}' missing 0x{:08X}", key, e.code().0);
            Ok(())
        }
    }
}

/// Win11 设置页那种卡片：圆角 + 描边 + 主题背景。
///
/// 三个键都来自 WinUI 主题资源，与「系统 › 屏幕」里的卡片同源：
///  * `CardBackgroundFillColorDefaultBrush` —— 卡片底色
///  * `CardStrokeColorDefaultBrush` —— 1px 描边
///  * 圆角 8 —— 对应 `OverlayCornerRadius` 档位。设置页里的卡片、快速设置面板里的
///    分组块用的都是这一档；`ControlCornerRadius`（4）是按钮/输入框那种控件级圆角，
///    用在卡片上会明显偏小。
fn make_card() -> Result<Border> {
    let card = Border::new()?;
    set_brush("CardBackgroundFillColorDefaultBrush", |b| {
        card.SetBackground(b)
    })?;
    set_brush("CardStrokeColorDefaultBrush", |b| card.SetBorderBrush(b))?;
    card.SetBorderThickness(Thickness {
        Left: 1.0,
        Top: 1.0,
        Right: 1.0,
        Bottom: 1.0,
    })?;
    card.SetCornerRadius(CornerRadius {
        TopLeft: 8.0,
        TopRight: 8.0,
        BottomRight: 8.0,
        BottomLeft: 8.0,
    })?;
    card.SetPadding(Thickness {
        Left: 14.0,
        Top: f64::from(CARD_PAD_V),
        Right: 14.0,
        Bottom: f64::from(CARD_PAD_V),
    })?;
    Ok(card)
}

/// 卡片左侧的单行标签。
fn make_label(text: &str) -> Result<TextBlock> {
    let tb = TextBlock::new()?;
    tb.SetText(&HSTRING::from(text))?;
    tb.SetVerticalAlignment(VerticalAlignment::Center)?;
    Ok(tb)
}

/// 「左标签 + 右控件」的设置行：Star/Auto 两列，行高 `min_h`。
fn make_setting_row(label: &str, min_h: f64) -> Result<Grid> {
    let row = Grid::new()?;
    row.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
    row.SetMinHeight(min_h)?;
    for t in [GridUnitType::Star, GridUnitType::Auto] {
        let col = ColumnDefinition::new()?;
        col.SetWidth(GridLength {
            Value: 1.0,
            GridUnitType: t,
        })?;
        row.ColumnDefinitions()?.Append(&col)?;
    }
    let tb = make_label(label)?;
    Grid::SetColumn(&tb, 0)?;
    row.Children()?.Append(&tb)?;
    Ok(row)
}

/// 把控件放进设置行右列（垂直居中、靠右，防止列比控件宽时贴左）。
fn set_row_control(row: &Grid, ctl: &UIElement) -> Result<()> {
    let fe = ctl.cast::<FrameworkElement>()?;
    fe.SetVerticalAlignment(VerticalAlignment::Center)?;
    fe.SetHorizontalAlignment(HorizontalAlignment::Right)?;
    Grid::SetColumn(&fe, 1)?;
    row.Children()?.Append(&fe)?;
    Ok(())
}

/// 单开关卡片（中文锁定 / 开机自启）：左标签 + 右开关。
fn make_switch_card(label: &str, sw: &ToggleSwitch) -> Result<Border> {
    let row = make_setting_row(label, f64::from(ROW_H))?;
    set_row_control(&row, &sw.cast()?)?;
    let card = make_card()?;
    card.SetChild(&row)?;
    Ok(card)
}

/// 日文锁定卡片：左标签，右侧 ComboBox + ToggleSwitch 并排。
fn make_japanese_card(items: &Items) -> Result<Border> {
    let row = make_setting_row("日文锁定", f64::from(ROW_H))?;
    let right = StackPanel::new()?;
    right.SetOrientation(Orientation::Horizontal)?;
    right.SetSpacing(8.0)?;
    let sw = items.japanese.cast::<FrameworkElement>()?;
    sw.SetVerticalAlignment(VerticalAlignment::Center)?;
    right.Children()?.Append(&items.japanese_mode)?;
    right.Children()?.Append(&sw)?;
    set_row_control(&row, &right.cast()?)?;
    let card = make_card()?;
    card.SetChild(&row)?;
    Ok(card)
}

/// 展开区行间的通栏分割线。
///
/// 用 1px 高的 Border 画线。负水平外边距抵消模板展开区内边距（EXP_CONTENT_PAD），
/// 使线画满卡片整宽——系统设置展开列表的分割线就是通栏的。
///
/// 颜色取 `CardStrokeColorDefaultBrush` 而不是 Divider 系列：模板里头部底边那条线
/// （展开时 header 与内容区间的分隔）用的就是卡片描边色（ExpanderHeaderBorderBrush），
/// 行间分割线取同一键才能与之无色差；且该键在本工具支持的旧运行时上也存在
/// （实测 `DividerFillColorDefaultBrush` 在部分运行时版本上 Lookup 失败）。
fn make_row_divider() -> Result<Border> {
    let line = Border::new()?;
    set_brush("CardStrokeColorDefaultBrush", |b| line.SetBackground(b))?;
    line.SetHeight(1.0)?;
    line.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
    line.SetMargin(Thickness {
        Left: -EXP_CONTENT_PAD,
        Top: 0.0,
        Right: -EXP_CONTENT_PAD,
        Bottom: 0.0,
    })?;
    Ok(line)
}

/// CapsLock 卡片：Expander 自带右侧展开箭头，展开区是短按/长按/阈值三行。
///
/// 不套 make_card 的 Border：Expander 默认模板本身就是设置页那种卡片
/// （header + 展开内容），卡片外观（底色/描边/圆角）直接设在 Expander 上。
fn make_caps_card(items: &Items) -> Result<Expander> {
    let ex = &items.caps_card;
    // 折叠态头部默认高 48（主题资源 ExpanderMinHeight），比普通卡片（CARD_H=60）矮。
    // 模板里头部 ToggleButton 与内容 Border 的 MinHeight 都绑定 Expander 自身的
    // MinHeight，头部内容垂直居中（ExpanderHeaderVerticalContentAlignment=Center），
    // 故直接抬 MinHeight 即可让折叠态与普通卡片等高，且文字/箭头保持居中；
    // 展开区自然高度（EXPANDED_H）远大于 60，该 MinHeight 对展开态无影响。
    ex.SetMinHeight(f64::from(CARD_H))?;
    ex.SetHeader(&make_label("CapsLock切换输入法")?)?;
    // 实测默认模板下 Expander 不横向撑满 StackPanel（缩到内容宽），必须显式 Stretch。
    ex.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
    ex.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;
    set_brush("CardBackgroundFillColorDefaultBrush", |b| ex.SetBackground(b))?;
    set_brush("CardStrokeColorDefaultBrush", |b| ex.SetBorderBrush(b))?;
    ex.SetBorderThickness(Thickness {
        Left: 1.0,
        Top: 1.0,
        Right: 1.0,
        Bottom: 1.0,
    })?;
    ex.SetCornerRadius(CornerRadius {
        TopLeft: 8.0,
        TopRight: 8.0,
        BottomRight: 8.0,
        BottomLeft: 8.0,
    })?;
    // 展开区上下内边距清零（模板默认 ExpanderContentPadding=16 四边等值）：
    // 行高 48 内部控件居中已自带 8px 空隙，再叠 16px 会让首行上方/末行下方
    // 留白明显大于行间。水平 16 保留，与头部文字左对齐。模板里该 Padding 只被
    // 展开区内容 Border 使用（TemplateBinding），头部 padding 是独立的静态资源，
    // 不受影响。
    ex.SetPadding(Thickness {
        Left: 16.0,
        Top: 0.0,
        Right: 16.0,
        Bottom: 0.0,
    })?;

    // 展开区：三行（行高 EXP_ROW_H），行间通栏分割线，与系统设置的展开列表同形。
    // 头部与第一行之间的分割线由模板自带，无需再加；末行之后也没有。
    // 行本身不加外边距——模板展开区自带 16px 内边距，行内容恰好与头部文字左对齐；
    // 分割线则用负外边距抵消该内边距，画满卡片整宽。
    let rows = StackPanel::new()?;

    let short_row = make_setting_row("短按", f64::from(EXP_ROW_H))?;
    set_row_control(&short_row, &items.caps_short.cast()?)?;
    rows.Children()?.Append(&short_row)?;

    rows.Children()?.Append(&make_row_divider()?)?;

    let long_row = make_setting_row("长按", f64::from(EXP_ROW_H))?;
    set_row_control(&long_row, &items.caps_long.cast()?)?;
    rows.Children()?.Append(&long_row)?;

    rows.Children()?.Append(&make_row_divider()?)?;

    // 阈值行：标签 + 定宽拉条（与下拉框同宽，右缘对齐；
    // 数值由拉条拖动时的工具提示显示，不放输入框）。
    let slider = items.threshold_slider.clone();
    slider.SetWidth(CAPS_COMBO_W)?;
    let thresh_row = make_setting_row("长按阈值", f64::from(EXP_ROW_H))?;
    set_row_control(&thresh_row, &slider.cast()?)?;
    rows.Children()?.Append(&thresh_row)?;

    ex.SetContent(&rows)?;

    // 展开/收起时调整窗口尺寸（见 on_expand_state）。Expanding/Collapsed 事件
    // 都在模板 storyboard 启动的同一帧触发（Expander::OnIsExpandedPropertyChanged
    // 里 RaiseCollapsedEvent 紧跟 UpdateExpandState，并非播完才触发——「Collapsed
    // 在动画后触发」是此前的误读），窗口操作与内容动画天然同步。
    //
    // 曾改用 IsExpanded 属性变化回调（RegisterPropertyChangedCallback）：
    // 一注册整个 XAML 岛渲染白屏（PrintWindow/实际绘制均无内容），弃用。
    ex.Expanding(&TypedEventHandler::new(
        |_: Ref<'_, Expander>, _: Ref<'_, ExpanderExpandingEventArgs>| {
            on_expand_state(true);
            Ok(())
        },
    ))?;
    ex.Collapsed(&TypedEventHandler::new(
        |_: Ref<'_, Expander>, _: Ref<'_, ExpanderCollapsedEventArgs>| {
            on_expand_state(false);
            Ok(())
        },
    ))?;
    Ok(items.caps_card.clone())
}

/// 底栏：右对齐的退出图标按钮。
///
/// 不设背景、不设边框——背景即浮窗基底（亚克力本身），分隔线归上方内容区的底边。
///
/// 左右内边距与内容区取同一个值：`ContentDialog` 模板里 `CommandSpace.Padding`
/// 和内容区 Padding 绑的是同一个键 `ContentDialogPadding`，底栏并非通栏无边距；
/// 少了它，悬停底板会贴到浮窗边缘。上下不留，由 `FOOTER_H` 给高度即可。
fn make_footer() -> Result<Border> {
    let bar = StackPanel::new()?;
    bar.SetOrientation(Orientation::Horizontal)?;
    bar.SetHorizontalAlignment(HorizontalAlignment::Right)?;
    bar.SetVerticalAlignment(VerticalAlignment::Center)?;

    // U+E711 Cancel，取自 Segoe Fluent Icons，与系统底栏同款字形。
    let quit = make_command_button("\u{E711}", "退出")?;
    quit.Click(&RoutedEventHandler::new(|_, _| {
        // 回调跑在消息循环所在线程，直接投 WM_QUIT 即可。
        unsafe { PostQuitMessage(0) };
        Ok(())
    }))?;
    bar.Children()?.Append(&quit)?;

    let footer = Border::new()?;
    footer.SetMinHeight(f64::from(FOOTER_H))?;
    let pad = f64::from(CONTENT_PAD_V);
    footer.SetPadding(Thickness {
        Left: pad,
        Top: 0.0,
        Right: pad,
        Bottom: 0.0,
    })?;
    footer.SetChild(&bar)?;
    Ok(footer)
}

/// 底栏图标按钮。用 `AppBarButton` 而非自绘 Button，是因为命令栏这一档的
/// 尺寸、悬停/按下反馈、图标字号在 Win11 里由一组主题资源统一规定
/// （`AppBarThemeMinHeight` = 48、图标 16pt、`SymbolThemeFontFamily`），
/// AppBarButton 的默认模板正是这些资源的消费者——系统各处底栏之所以高度一致，
/// 靠的就是它，自己写死数值必然对不上。
///
/// `LabelPosition = Collapsed` 收起文字标签只留图标，与设置浮窗、快速设置面板
/// 底部那排图标同形；文字退化为悬停提示，语义不丢。
///
/// **图标字号不要覆盖**。模板把图标套在 `Height=16` 的 Viewbox 里
/// （`AppBarButton_themeresources.xaml:366`，高度取 `AppBarButtonContentHeight`），
/// Viewbox 是按**整个文本框**等比缩放到 16，不是按字号。`FontIcon` 默认字号 20
/// （`icon.cpp` 的 `g_ClientCoreFontSize`），由 Viewbox 压到 16 —— 归一化已经做完了。
/// 手动设成 16 只会让自然文本框变小、被 Viewbox 反向放大，图标显著偏大。
///
/// 唯一覆盖的是宽度：模板默认 68 是为文字标签预留的，标签收起后只剩 16px 图标，
/// 68 宽会让两个图标间距远大于系统底栏。取 40 是因为模板的悬停底板
/// （`AppBarButtonInnerBorder`）边距为 `2,6,2,6`：40 宽 × 48 高（
/// `AppBarThemeCompactHeight`）扣掉后正好是 **36×36 的正方形**悬停区，
/// 与系统底栏图标按钮同形。换任何别的宽度，悬停区都会变成长方形。
fn make_command_button(glyph: &str, label: &str) -> Result<AppBarButton> {
    let b = AppBarButton::new()?;
    let icon = FontIcon::new()?;
    icon.SetGlyph(&HSTRING::from(glyph))?;
    b.SetIcon(&icon)?;
    b.SetLabel(&HSTRING::from(label))?;
    b.SetLabelPosition(CommandBarLabelPosition::Collapsed)?;
    b.SetWidth(40.0)?;
    Ok(b)
}

/// 内容区：标题 + 四张设置卡片，底边带分隔线。
///
/// 抬亮的是**内容区**而非底栏，这是照 `ContentDialog` 模板的归属：
/// 内容区 `Background = ContentDialogTopOverlay`（→ `LayerFillColorAltBrush`），
/// 底栏 `Background = {TemplateBinding Background}` 即对话框基底、不做抬亮，
/// 视觉上是「上亮下透」。浮窗坐在亚克力上，故换成 `LayerOnAcrylic` 那一支。
///
/// 分隔线同样归内容区：模板里 `BorderThickness="0,0,0,1"` 挂在内容区**底边**。
///
/// 额外返回上部内容组（标题+卡1+卡2）的引用：展开动画时它与 CapsLock 卡一起
/// 做「上面部分上移」的位移过渡（见 on_expand_state / anim_tick）。
fn make_content(items: &Items) -> Result<(Border, StackPanel)> {
    let panel = StackPanel::new()?;
    panel.SetSpacing(f64::from(CARD_GAP))?;

    let title = TextBlock::new()?;
    title.SetText(h!("lock-ime"))?;
    title.SetFontSize(14.0)?;
    title.SetFontWeight(windows::UI::Text::FontWeights::SemiBold()?)?;
    title.SetMargin(Thickness {
        Left: 2.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 2.0,
    })?;

    // 上部内容打包成组：展开动画期间整组位移（间距与外层一致，布局无差别）。
    let header_group = StackPanel::new()?;
    header_group.SetSpacing(f64::from(CARD_GAP))?;
    header_group.Children()?.Append(&title)?;
    header_group
        .Children()?
        .Append(&make_switch_card("中文锁定", &items.chinese)?)?;
    header_group
        .Children()?
        .Append(&make_japanese_card(items)?)?;
    panel.Children()?.Append(&header_group)?;

    panel.Children()?.Append(&make_caps_card(items)?)?;
    panel
        .Children()?
        .Append(&make_switch_card("开机自启", &items.autostart)?)?;

    let content = Border::new()?;
    set_brush("LayerOnAcrylicFillColorDefaultBrush", |b| {
        content.SetBackground(b)
    })?;
    set_brush("CardStrokeColorDefaultBrush", |b| content.SetBorderBrush(b))?;
    content.SetBorderThickness(Thickness {
        Left: 0.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 1.0,
    })?;
    let pad = f64::from(CONTENT_PAD_V);
    content.SetPadding(Thickness {
        Left: pad,
        Top: pad,
        Right: pad,
        Bottom: pad,
    })?;
    content.SetChild(&panel)?;
    Ok((content, header_group))
}

/// 把配置写回并落盘。各控件事件共用，避免重复。
fn apply<F: FnOnce(&mut crate::config::Config)>(f: F) {
    crate::state::with(|st| {
        f(&mut st.config);
        let _ = st.config.save();
    });
}

/// 长按阈值写入：拉到边界外的值按范围钳制。
fn apply_threshold(v: f64) {
    let ms = v.round().clamp(THRESH_MIN, THRESH_MAX) as u64;
    apply(|c| c.capslock_longpress_ms = ms);
}

/// 建一个只读下拉框（只能选不能输）：定宽、垂直居中。
///
/// 居中：显式设 Center 是因为横向 StackPanel 会把子元素在纵向上 Stretch
/// （日文锁定卡片里下拉框被拉到开关那么高、文字顶在上边）。
fn make_combo(labels: &[&str], width: f64) -> Result<ComboBox> {
    let cb = ComboBox::new()?;
    let items = cb.Items()?;
    for t in labels {
        let item = ComboBoxItem::new()?;
        item.SetContent(&PropertyValue::CreateString(&HSTRING::from(*t))?)?;
        items.Append(&item)?;
    }
    cb.SetWidth(width)?;
    cb.SetVerticalAlignment(VerticalAlignment::Center)?;
    Ok(cb)
}

fn make_toggle() -> Result<ToggleSwitch> {
    let sw = ToggleSwitch::new()?;
    // 默认样式带 MinWidth≈156（为 On/Off 文本预留），会让所在 Auto 列吃掉标签的宽度，
    // 卡片 2 里甚至把「日文锁定」挤没。清掉，让列宽等于开关实际宽度。
    sw.SetMinWidth(0.0)?;
    Ok(sw)
}

macro_rules! trystep {
    ($label:expr, $e:expr) => {{
        match $e {
            Ok(v) => v,
            Err(e) => {
                crate::logmsg!("flyout: step '{}' failed 0x{:08X}", $label, e.code().0);
                return Err(e);
            }
        }
    }};
}

fn build() -> Result<Flyout> {
    trystep!(
        "init_apartment",
        winui3::init_apartment(winui3::ApartmentType::SingleThreaded)
    );
    // 挂载系统已装的 WindowsAppRuntime 框架包；缺失时在此返回 Err。
    let dep = trystep!("bootstrap", PackageDependency::initialize());
    let dqc = trystep!(
        "dispatcher",
        DispatcherQueueController::CreateOnCurrentThread()
    );
    let dq = dqc.DispatcherQueue()?;
    // compose 必须在 WindowsXamlManager 之前：它内部创建的
    // XamlControlsXamlMetaDataProvider 才是 WinUI 控件样式（圆角、Fluent 外观）的来源。
    // 反过来先 Initialize，manager 会自建一个不带 provider 的 Application，
    // 控件就退化成无主题的方角样式。
    trystep!("application", XamlApp::compose(NullApp));
    let mgr = trystep!("xaml_mgr", WindowsXamlManager::InitializeForCurrentThread());

    // CreateForContextMenu：WinUI 专为上下文菜单/浮窗提供的 presenter。
    // 它只管行为（无标题栏、不进 Alt+Tab、不抢激活），**不负责视觉外框**——
    // 圆角与边框归 DWM 管，见下方 Show 之后的 apply_window_chrome。
    let presenter = trystep!("presenter", OverlappedPresenter::CreateForContextMenu());
    let win = trystep!("appwindow", AppWindow::CreateWithPresenter(&presenter));
    win.AssociateWithDispatcherQueue(&dq)?;
    win.SetTitle(h!("lock-ime"))?;
    win.SetIsShownInSwitchers(false)?; // 不进 Alt+Tab / 任务栏
    // 有意覆盖 CreateForContextMenu 的预设（该方法文档的配置表里此项为 false）：
    // 托盘浮窗要压在任务栏之上，不置顶会被任务栏盖住。
    presenter.SetIsAlwaysOnTop(true)?;
    let _ = presenter.SetBorderAndTitleBar(false, false);
    win.ResizeClient(SizeInt32 {
        Width: PANEL_W,
        Height: panel_h(false),
    })?;

    let src = trystep!("xaml_source", DesktopWindowXamlSource::new());
    trystep!("xaml_init", src.Initialize(win.Id()?));

    // 根容器用顶对齐 StackPanel 而非 Star/Auto Grid：展开/收起的窗口高度动画
    // 逐帧 resize 时，内容各元素的位置不随容器高度变化（顶对齐零重排），
    // 这是动画不抽搐的关键——Grid 的 Star 行会让底栏每帧跟随窗口底边重排，
    // XAML 岛的布局滞后一拍，表现为内容位置疯狂抽搐。
    let root = trystep!("root", StackPanel::new());

    // 必须在任何 make_card / theme_brush 之前：控件模板与主题画刷都在这份字典里。
    // 合并到 Application 级而非 root，theme_brush 走的正是 Application::Current().Resources()。
    // 前提是 XamlApp::compose 已在 WindowsXamlManager 之前建立带元数据 provider
    // 的 Application，否则此处激活失败返回 E_FAIL。
    match (XamlControlsResources::new(), Application::Current()) {
        (Ok(res), Ok(app)) => {
            app.Resources()?.MergedDictionaries()?.Append(&res)?;
            crate::logmsg!("flyout: XamlControlsResources merged into Application");
        }
        (Err(e), _) | (_, Err(e)) => {
            crate::logmsg!("flyout: XamlControlsResources failed 0x{:08X}", e.code().0)
        }
    }

    // 各控件的选中态先留默认，与值一并在 SetContent 之后由 sync 落实（原因见 Items::sync）。
    let threshold_slider = Slider::new()?;
    threshold_slider.SetMinimum(THRESH_MIN)?;
    threshold_slider.SetMaximum(THRESH_MAX)?;
    threshold_slider.SetStepFrequency(THRESH_STEP)?;

    let items = Items {
        chinese: make_toggle()?,
        japanese: make_toggle()?,
        japanese_mode: make_combo(&["平假名", "片假名", "全角英数"], COMBO_W)?,
        caps_card: Expander::new()?,
        caps_short: make_combo(&CAPS_ACTION_LABELS, CAPS_COMBO_W)?,
        caps_long: make_combo(&CAPS_ACTION_LABELS, CAPS_COMBO_W)?,
        threshold_slider,
        autostart: make_toggle()?,
    };
    bind_controls(&items)?;

    let (content, header_group) = make_content(&items)?;
    root.Children()?.Append(&content)?;

    let footer = make_footer()?;
    root.Children()?.Append(&footer)?;

    // 展开动画期间做「上面部分上移」的渲染级位移（RenderTransform 不影响布局，
    // 逐帧设置即时生效，见 anim_tick）。
    let header_xform = CompositeTransform::new()?;
    header_group.SetRenderTransform(&header_xform)?;
    let caps_xform = CompositeTransform::new()?;
    items.caps_card.SetRenderTransform(&caps_xform)?;

    trystep!("set_content", src.SetContent(&root));

    // 初值必须等到内容树挂上宿主、模板套用之后再写，详见 Items::sync。
    items.sync();

    // 亚克力必须在 SetContent 之后；失败不致命（旧系统降级为纯色）。
    if let Err(e) = DesktopAcrylicBackdrop::new().and_then(|b| src.SetSystemBackdrop(&b)) {
        crate::logmsg!("flyout: acrylic unavailable (0x{:08X})", e.code().0);
    }

    // Show 一次让 HWND 实体化，随后立即隐藏——首帧开销在启动时付掉。
    trystep!("show", win.Show());
    let hwnd = unsafe { winui3::interop::GetWindowFromWindowId(win.Id()?)? };
    apply_window_chrome(hwnd);

    // 失焦即收起。走 Win32 的 WM_ACTIVATE 而非 WinUI 的 InputActivationListener：
    // 后者在本场景下不可用——窗口一旦 Hide 过，ShowWithActivation 就再也无法让它
    // 回到 Activated（实测每次 Show 后 State 恒为 Deactivated），
    // 于是失焦不再产生状态跳变，事件只在进程首次显示时触发过一次。
    // 与关闭方式无关，toggle 关闭同样如此。
    //
    // 子类化而非改窗口过程：AppWindow 的 HWND 由 WinUI 创建并持有自己的窗口过程，
    // SetWindowSubclass 是官方为这种「插一手别人的窗口」提供的接口，
    // 消息会先过我们、再交还原过程。
    unsafe {
        // 调试截图模式（LOCK_IME_DEBUG_FLYOUT）下不挂失焦钩子，面板常驻便于截取。
        #[cfg(debug_assertions)]
        let debug_show = std::env::var_os("LOCK_IME_DEBUG_FLYOUT").is_some();
        #[cfg(not(debug_assertions))]
        let debug_show = false;
        if !debug_show {
            let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0);
        }
    }

    win.Hide()?;

    Ok(Flyout {
        _dep: dep,
        _dqc: dqc,
        _mgr: mgr,
        win,
        _src: src,
        hwnd,
        items,
        root,
        header_xform,
        caps_xform,
        expand_sb: None,
        visible: false,
        anchor: None,
        hidden_at: None,
        collapse_start: None,
    })
}

/// 把控件事件绑到配置写入。所有回调先查 `is_syncing`：程序化写值（sync）不触发配置回写。
fn bind_controls(items: &Items) -> Result<()> {
    bind_switch(&items.chinese, |v| {
        apply(|c| c.chinese_lock_enabled = v)
    })?;
    bind_switch(&items.japanese, |v| {
        apply(|c| c.japanese_lock_enabled = v)
    })?;
    bind_switch(&items.autostart, |v| {
        crate::autostart::set_autostart(v);
        apply(|c| c.autostart = v);
    })?;

    bind_combo(&items.japanese_mode, |i| {
        apply(|c| c.japanese_mode = japanese_at(i))
    })?;
    bind_combo(&items.caps_short, |i| {
        apply(|c| c.capslock_short_action = action_at(i))
    })?;
    bind_combo(&items.caps_long, |i| {
        apply(|c| c.capslock_long_action = action_at(i))
    })?;

    items.threshold_slider.ValueChanged(&RangeBaseValueChangedEventHandler::new(
        move |_, args: Ref<'_, RangeBaseValueChangedEventArgs>| {
            if is_syncing() {
                return Ok(());
            }
            apply_threshold(args.ok()?.NewValue()?);
            Ok(())
        },
    ))?;
    Ok(())
}

fn bind_switch<F: Fn(bool) + Send + 'static>(sw: &ToggleSwitch, f: F) -> Result<()> {
    let sw2 = sw.clone();
    sw.Toggled(&RoutedEventHandler::new(move |_, _| {
        if !is_syncing() {
            f(sw2.IsOn().unwrap_or(false));
        }
        Ok(())
    }))?;
    Ok(())
}

fn bind_combo<F: Fn(i32) + Send + 'static>(cb: &ComboBox, f: F) -> Result<()> {
    let cb2 = cb.clone();
    cb.SelectionChanged(&SelectionChangedEventHandler::new(move |_, _| {
        if !is_syncing() {
            f(cb2.SelectedIndex().unwrap_or(0));
        }
        Ok(())
    }))?;
    Ok(())
}
