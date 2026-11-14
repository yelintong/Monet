//! 系统级窗口标题区域 —— 按 **GPUI Kit** 的 `TitleBar` 契约实现（不引入外部 crate）。
//!
//! kit 的写法（`examples/window_title/src/main.rs` / docs/components/title-bar）：
//! ```
//! let options = TitleBar::window_options();          // 窗口配置：声明标题栏区域
//! v_flex().size_full()
//!     .child(TitleBar::new().child(tab_bar_content)) // 内容塞进标题栏
//!     .child(div().id("window-body").size_full())    // 内容区在标题栏下面
//! ```
//! 所以这条标题栏**不是**客户区里再画的一条普通控件，而是窗口的标题栏区域：
//! - [`title_bar_options`] 等价于 `TitleBar::window_options()` 里的 `titlebar` 配置：
//!   `appears_transparent = true` → Windows `hide_title_bar = true`，系统不再画默认标题栏，
//!   这块区域交给应用渲染（kit 文档：「replace the default OS title bar」）。
//! - 整条区域用 `InteractiveElement::window_control_area(WindowControlArea::Drag)` 注册给
//!   平台；Windows 在 `WM_NCHITTEST` 里把它映射成 `HTCAPTION`，于是系统直接提供
//!   「按住拖窗 / 双击最大化↔还原 / 右键系统菜单」。三个按钮各映射
//!   `HTMINBUTTON / HTMAXBUTTON / HTCLOSE`，动作同样由系统执行，不再挂 `on_click`
//!   （否则双重触发，且 `zoom()` 只最大化不还原）。
//! - 标签条等内容是 `TitleBar::new().child(...)` 的 child，**放在标题栏里**，不是画布里的一条。
//!
//! 几何：标题栏占窗口 y ∈ [0, `TITLE_H`)，Dock 内容区从 `TITLE_H` 起。画布命中与浮层坐标
//! 统一由 [`crate::dock::DockLayout`] 推导（`canvas_origin().y = TITLE_H`），不再出现
//! 「漏减标题栏高度 → 画布点击热区整体偏移」。

use figcraft_i18n::tr;
use gpui::{Div, Point, SharedString, TitlebarOptions, WindowControlArea, div, prelude::*, px};

use crate::widgets::icon::{self, Icon};
use crate::widgets::{pt, rgba, tokens};

/// 窗口标题栏高度（kit 默认 34px，示例允许 `.h(px(40.))`；本项目的 TabBar 用 40pt）。
pub const TITLE_H: f32 = 40.0;
/// 标题栏内容左留白（上游 `tabs.rs`：Home 键与左轨右缘对齐）。
pub const LEFT_PAD: f32 = 8.0;
/// 一个窗口控制按钮的宽度（上游 `windows_buttons::W = 46.0`）。
pub const CONTROL_W: f32 = 46.0;
/// 三个窗口控制按钮的总宽度。
pub const CONTROLS_W: f32 = CONTROL_W * 3.0;

/// kit `TitleBar::title_bar_options()`：声明窗口标题、隐藏默认系统标题栏，
/// macOS 红绿灯留在 (9,9)。放进 `WindowOptions.titlebar`。
pub fn title_bar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(SharedString::from(tr("Monet"))),
        appears_transparent: true,
        traffic_light_position: Some(Point::new(px(9.0), px(9.0))),
    }
}

/// 标题栏里留给 TabBar 内容的宽度：窗口宽 − 左留白 − 三个窗口控制按钮。
pub fn content_width(viewport_w: f32) -> f32 {
    (viewport_w - LEFT_PAD - CONTROLS_W).max(0.0)
}

/// 最大化按钮图标：已最大化时画还原（上游 `WinRestore`）。
pub fn maximize_icon(maximized: bool) -> Icon {
    if maximized { Icon::WinRestore } else { Icon::WinMaximize }
}

/// 一个窗口控制按钮：hover 底色，关闭键 hover 变红。命中区交给平台，动作由系统执行。
fn control_button(id: &'static str, icon: Icon, close: bool, area: WindowControlArea) -> gpui::Stateful<Div> {
    let t = tokens();
    let hover = if close { rgba(t.danger) } else { rgba(t.hover) };
    let ink = if close { gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } } else { rgba(t.icon) };
    div()
        .id(SharedString::from(id))
        .flex_none()
        .w(pt(CONTROL_W))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .window_control_area(area)
        // Fallback: if the OS does not take the area, a plain click still works.
        .on_click(move |_e, window, _cx| match area {
            WindowControlArea::Min => window.minimize_window(),
            WindowControlArea::Max => window.zoom_window(),
            WindowControlArea::Close => window.remove_window(),
            WindowControlArea::Drag => {}
        })
        .child(icon::paint(icon, ink, 20.0))
}

/// kit `TitleBar::new().child(content)`：整条注册成系统标题栏拖拽区，
/// 左留白与内容空白区也标 Drag（覆盖子 hitbox 的空隙），右侧三个按钮各归系统。
pub fn title_bar(content: impl IntoElement, maximized: bool) -> Div {
    let min = control_button("title-min", Icon::WinMinimize, false, WindowControlArea::Min);
    let max = control_button("title-max", maximize_icon(maximized), false, WindowControlArea::Max);
    let close = control_button("title-close", Icon::Close, true, WindowControlArea::Close);

    div()
        .flex_none()
        .w_full()
        .h(pt(TITLE_H))
        .flex()
        .flex_row()
        .items_center()
        .bg(rgba(tokens().panel))
        // Do NOT put `Drag` on the whole bar: that makes the OS treat the entire title bar as
        // HTCAPTION and swallows clicks on the Home button and the tabs. Only the left padding and
        // a short strip before the window controls are drag regions.
        .child(div().flex_none().w(pt(LEFT_PAD)).h_full().window_control_area(WindowControlArea::Drag))
        .child(div().flex_1().min_w(pt(0.0)).h_full().flex().items_center().child(content))
        .child(div().flex_none().w(pt(8.0)).h_full().window_control_area(WindowControlArea::Drag))
        .child(min)
        .child(max)
        .child(close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_width_reserves_the_shell_padding_and_three_controls() {
        assert_eq!(content_width(1280.0), 1280.0 - LEFT_PAD - CONTROLS_W);
        assert_eq!(CONTROLS_W, 3.0 * CONTROL_W);
        assert_eq!(content_width(0.0), 0.0);
    }

    #[test]
    fn maximize_button_switches_between_maximize_and_restore() {
        assert_eq!(maximize_icon(false), Icon::WinMaximize);
        assert_eq!(maximize_icon(true), Icon::WinRestore);
    }

    #[test]
    fn title_bar_options_declares_the_window_titlebar_region() {
        let o = title_bar_options();
        assert!(o.appears_transparent, "kit 契约：隐藏默认系统标题栏，区域交给应用渲染");
        assert!(o.title.is_some());
        assert!(o.traffic_light_position.is_some());
    }
}
