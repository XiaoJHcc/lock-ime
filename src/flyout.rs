//! Win11 风格托盘浮窗：用 WinUI 3（XAML Islands）绘制，把托盘菜单的条目搬进来。
//!
//! 走的是「调用系统原生 UI」而非手工仿造：控件、亚克力、圆角、动画全部由
//! WindowsAppRuntime 框架包提供，本进程只增约 150KB，且不引入 .NET。
//!
//! 运行时不存在时（未装框架包的旧系统）`init` 返回 false，调用方回退到原生菜单。
//!
//! # 构建顺序不可调换
//! `Create → XamlSource::Initialize → SetContent → SetSystemBackdrop
//!  → Show → 取 HWND → 窗口外观/失焦监听`
//!
//! 两处顺序踩过坑：
//!  * backdrop 必须在 `SetContent` 之后——内容树为空时挂 backdrop 会**静默段错误**；
//!  * `GetWindowFromWindowId` 必须在 `Show` 之后——窗口未显示时 HWND 尚未实体化，
//!    返回 `E_POINTER`。

use crate::config::JapaneseMode;
use std::cell::RefCell;
use windows::core::{h, Interface, Result, HSTRING};
use windows::Foundation::{IReference, PropertyValue};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    PostQuitMessage, SetForegroundWindow, SystemParametersInfoW, SPI_GETWORKAREA,
    PostMessageW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WA_INACTIVE, WM_ACTIVATE, WM_USER,
};
use winui3::bootstrap::PackageDependency;
use winui3::Microsoft::UI::Dispatching::DispatcherQueueController;
use winui3::Microsoft::UI::Windowing::{AppWindow, OverlappedPresenter};
use winui3::Microsoft::UI::Xaml::Controls::{
    AppBarButton, Border, CheckBox, CommandBarLabelPosition, FontIcon, Grid, Orientation,
    RowDefinition, StackPanel, TextBlock, XamlControlsResources,
};
use winui3::Microsoft::UI::Xaml::Hosting::{DesktopWindowXamlSource, WindowsXamlManager};
use winui3::Microsoft::UI::Xaml::Media::{Brush, DesktopAcrylicBackdrop};
use winui3::Microsoft::UI::Xaml::{
    Application, CornerRadius, GridLength, GridUnitType, HorizontalAlignment,
    LaunchActivatedEventArgs, RoutedEventHandler, Thickness, VerticalAlignment,
};
use winui3::{XamlApp, XamlAppOverrides};

/// 面板逻辑尺寸（96dpi 基准），高度按下列常量累加得出。
///
/// 不做运行时测量：`DesktopWindowXamlSource` 的内容树量不出可用的高度
/// （手动 `Measure` 早于模板套用、`ActualHeight` 是布局后的值、`DesiredSize` 返回 0），
/// 而各部分的尺寸本就来自 WinUI 主题资源里的固定值，直接累加即可。
/// 改布局时同步改这里。
const PANEL_W: i32 = 296;

/// 复选框行高：`CheckBox` 模板的 `MinHeight`。
const ROW_H: i32 = 32;
/// 卡片内复选框间距，见 lock_stack 的 Spacing。
const ROW_GAP: i32 = 10;
/// 卡片上下内边距各 12，见 make_card。
const CARD_PAD_V: i32 = 12;
/// 标题行：FontSize 14 的单行文本约 20，加下边距 2。
const TITLE_H: i32 = 22;
/// 内容区上下内边距各 12，见 build 里 content 的 Padding。
const CONTENT_PAD_V: i32 = 12;
/// 标题与卡片间距，见 panel 的 Spacing。
const TITLE_GAP: i32 = 8;
/// 底栏高度：`AppBarThemeCompactHeight`。
const FOOTER_H: i32 = 48;
/// 内容区底边分隔线 1px。
const SEP_H: i32 = 1;
/// 窗口边框余量 1px。
///
/// 经验值，非推导所得：按上列常量精确累加时，卡片底边那 1px 描边仍被窗口下缘吃掉。
/// 合理的怀疑是 DWM 边框覆盖了客户区最外那行像素（本窗口开了 `DWMWA_BORDER_COLOR`），
/// 但没有实测证据，故不在此断言原因。
///
/// 多给这 1px 是安全的：Row0 是 Star，富余空间被内容区吸收，底栏仍旧贴底。
const CHROME_ALLOWANCE: i32 = 1;

/// 面板高度 = 内容区 + 分隔线 + 底栏。
const fn panel_h(rows: i32) -> i32 {
    let card = CARD_PAD_V * 2 + ROW_H * rows + ROW_GAP * (rows - 1);
    CONTENT_PAD_V * 2 + TITLE_H + TITLE_GAP + card + SEP_H + FOOTER_H + CHROME_ALLOWANCE
}
/// 面板与托盘图标之间的间距（逻辑像素）。
const GAP: i32 = 8;

/// 面板中的开关项。
struct Items {
    chinese: CheckBox,
    japanese: CheckBox,
    capslock: CheckBox,
}

impl Items {
    /// 从当前配置回写勾选态与日文标签。
    ///
    /// 只能在控件已进可视化树（`SetContent` 之后）时调用，原因见 `make_checkbox`。
    fn sync(&self) {
        let Some((cn, ja, caps, mode)) = crate::state::with(|st| {
            (
                st.config.chinese_lock_enabled,
                st.config.japanese_lock_enabled,
                st.config.capslock_switch_enabled,
                st.config.japanese_mode,
            )
        }) else {
            return;
        };
        for (cb, v) in [(&self.chinese, cn), (&self.japanese, ja), (&self.capslock, caps)] {
            if let Ok(b) = boxed_bool(v) {
                let _ = cb.SetIsChecked(&b);
            }
        }
        let _ = set_content_text(&self.japanese, japanese_label(mode));
    }
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
    visible: bool,
    /// 上次收起的时刻，用于识别「点托盘收起」这一手势，见 `toggle_at`。
    hidden_at: Option<std::time::Instant>,
}

thread_local! {
    static FLYOUT: RefCell<Option<Flyout>> = const { RefCell::new(None) };
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

/// 从当前配置同步勾选态与日文标签（设置窗口改动后调用）。
pub fn refresh() {
    FLYOUT.with(|c| {
        if let Some(f) = c.borrow().as_ref() {
            f.items.sync();
        }
    });
}

impl Flyout {
    fn hide(&mut self) {
        let _ = self.win.Hide();
        self.visible = false;
        self.hidden_at = Some(std::time::Instant::now());
    }

    /// 依托盘图标位置定位并显示。坐标全程用物理像素，与 `tray_rect` 一致。
    fn show_at(&mut self, tray_rect: tray_icon::Rect) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as i32;
        let s = |v: i32| v * dpi / 96;
        let (pw, ph) = (s(PANEL_W), s(panel_h(3)));

        // ResizeClient 而非 Resize：后者设的是**窗口外框**（含 DWM 边框），
        // 客户区会比要求的矮一圈。两者都吃**物理像素**，而上面的常量是逻辑像素，
        // 故按 DPI 换算——100% 下数值相等掩盖了这点，150% 下窗口会整体偏小。
        // 放在 show_at 而非 build：建窗时窗口未显示，拿不到所在显示器的 DPI。
        if let Err(e) = self.win.ResizeClient(SizeInt32 {
            Width: pw,
            Height: ph,
        }) {
            crate::logmsg!("flyout: ResizeClient failed 0x{:08X}", e.code().0);
        }

        // 右边缘与托盘图标对齐，底边贴在图标上方。
        let icon_right = tray_rect.position.x as i32 + tray_rect.size.width as i32;
        let mut x = icon_right - pw;
        let mut y = tray_rect.position.y as i32 - ph - s(GAP);

        // 钳制到工作区，避免被任务栏遮挡或跑出屏幕（任务栏在侧边/顶部时同样成立）。
        if let Some(wa) = work_area() {
            x = x.clamp(wa.left, (wa.right - pw).max(wa.left));
            y = y.clamp(wa.top, (wa.bottom - ph).max(wa.top));
        }
        let _ = self.win.Move(PointInt32 { X: x, Y: y });

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

fn japanese_label(mode: JapaneseMode) -> &'static str {
    match mode {
        JapaneseMode::Hiragana => "日文锁平假名",
        JapaneseMode::Katakana => "日文锁片假名",
        JapaneseMode::FullWidthAlnum => "日文锁全角英数",
    }
}

/// XAML 的三态 `IsChecked` 要 `IReference<bool>`：先装箱成 IInspectable 再 cast。
fn boxed_bool(v: bool) -> Result<IReference<bool>> {
    PropertyValue::CreateBoolean(v)?.cast()
}

/// XAML 的 `Content` 是 `IInspectable`，字符串需装箱后再设。
fn set_content_text(cb: &CheckBox, text: &str) -> Result<()> {
    let s: HSTRING = text.into();
    cb.SetContent(&PropertyValue::CreateString(&s)?)
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
        Top: 12.0,
        Right: 14.0,
        Bottom: 12.0,
    })?;
    Ok(card)
}

/// 建一个只有文案、未定勾选态的复选框。
///
/// **不在这里设 `IsChecked`**：勾标是 `AnimatedIcon`，由 `CheckedNormal` 视觉状态
/// 把 `AnimatedIcon.State` 切到 `NormalOn` 来播动画呈现
/// （`CheckBox_themeresources.xaml:400`）。控件尚未进可视化树时模板还没套用，
/// `CheckGlyph` 不存在，这次状态转换无处落地；等模板套用后图标停在初值 `NormalOff`
/// （同文件 `:604`）上，而 `IsChecked` 已是 true——于是勾标显示为动画中间帧（横线），
/// 直到鼠标经过触发一次真正的状态转换才补上。
///
/// 勾选态统一在 `SetContent` 之后由 `sync` 写入，那时模板已套用。
fn make_checkbox(text: &str) -> Result<CheckBox> {
    let cb = CheckBox::new()?;
    set_content_text(&cb, text)?;
    Ok(cb)
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

/// 内容区：标题 + 开关卡片，底边带分隔线。
///
/// 抬亮的是**内容区**而非底栏，这是照 `ContentDialog` 模板的归属：
/// 内容区 `Background = ContentDialogTopOverlay`（→ `LayerFillColorAltBrush`），
/// 底栏 `Background = {TemplateBinding Background}` 即对话框基底、不做抬亮，
/// 视觉上是「上亮下透」。浮窗坐在亚克力上，故换成 `LayerOnAcrylic` 那一支。
///
/// 分隔线同样归内容区：模板里 `BorderThickness="0,0,0,1"` 挂在内容区**底边**。
fn make_content(items: &Items) -> Result<Border> {
    let panel = StackPanel::new()?;
    panel.SetSpacing(8.0)?;

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
    panel.Children()?.Append(&title)?;

    let stack = StackPanel::new()?;
    stack.SetSpacing(f64::from(ROW_GAP))?;
    for cb in [&items.chinese, &items.japanese, &items.capslock] {
        stack.Children()?.Append(cb)?;
    }
    let card = make_card()?;
    card.SetChild(&stack)?;
    panel.Children()?.Append(&card)?;

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
    Ok(content)
}

/// 底栏：右对齐的退出、设置两个图标按钮。
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

    // U+E711 Cancel、U+E713 Setting，取自 Segoe Fluent Icons，与系统底栏同款字形。
    let quit = make_command_button("\u{E711}", "退出")?;
    quit.Click(&RoutedEventHandler::new(|_, _| {
        // 回调跑在消息循环所在线程，直接投 WM_QUIT 即可。
        unsafe { PostQuitMessage(0) };
        Ok(())
    }))?;
    bar.Children()?.Append(&quit)?;

    let settings = make_command_button("\u{E713}", "设置")?;
    settings.Click(&RoutedEventHandler::new(|_, _| {
        hide();
        crate::settings_window::open();
        Ok(())
    }))?;
    bar.Children()?.Append(&settings)?;

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

/// 把配置写回并落盘。各开关共用，避免四份重复。
fn apply<F: FnOnce(&mut crate::config::Config)>(f: F) {
    crate::state::with(|st| {
        f(&mut st.config);
        let _ = st.config.save();
    });
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
        Height: panel_h(3),
    })?;

    let src = trystep!("xaml_source", DesktopWindowXamlSource::new());
    trystep!("xaml_init", src.Initialize(win.Id()?));

    let root = trystep!("grid", Grid::new());

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
    // 两行：Row0 内容区、Row1 底栏。
    // Star 而非 Auto：窗口若略高于内容自然高度（字体回退、DPI 取整），
    // 多出的空隙归内容区吸收，底栏始终贴底。
    for h in [GridUnitType::Star, GridUnitType::Auto] {
        let r = RowDefinition::new()?;
        r.SetHeight(GridLength {
            Value: 1.0,
            GridUnitType: h,
        })?;
        root.RowDefinitions()?.Append(&r)?;
    }

    // 日文项的文案随配置变，这里给默认值占位，与勾选态一并由下方的 sync 落实。
    let items = Items {
        chinese: make_checkbox("中文锁中文模式")?,
        japanese: make_checkbox(japanese_label(JapaneseMode::default()))?,
        capslock: make_checkbox("CapsLock 切换输入法")?,
    };
    // 勾选后只写配置、不关面板，便于连续切换多个开关。
    bind(&items.chinese, |v| apply(|c| c.chinese_lock_enabled = v))?;
    bind(&items.japanese, |v| apply(|c| c.japanese_lock_enabled = v))?;
    bind(&items.capslock, |v| {
        apply(|c| c.capslock_switch_enabled = v)
    })?;

    let content = make_content(&items)?;
    Grid::SetRow(&content, 0)?;
    root.Children()?.Append(&content)?;

    let footer = make_footer()?;
    Grid::SetRow(&footer, 1)?;
    root.Children()?.Append(&footer)?;

    trystep!("set_content", src.SetContent(&root));

    // 勾选态必须等到内容树挂上宿主、模板套用之后再写，否则勾标停在动画中间帧。
    // 详见 make_checkbox 的说明。
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
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0);
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
        visible: false,
        hidden_at: None,
    })
}

/// 把 CheckBox 的勾选/取消绑到同一个写配置闭包上。
fn bind<F: Fn(bool) + Clone + Send + 'static>(cb: &CheckBox, f: F) -> Result<()> {
    let on = f.clone();
    cb.Checked(&RoutedEventHandler::new(move |_, _| {
        on(true);
        Ok(())
    }))?;
    cb.Unchecked(&RoutedEventHandler::new(move |_, _| {
        f(false);
        Ok(())
    }))?;
    Ok(())
}
