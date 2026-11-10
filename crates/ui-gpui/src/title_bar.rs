//! 窗口级标题栏壳 —— 按 **GPUI Kit**（外部组件库 `gpui-kit` v0.7.0）的 `TitleBar` 契约实现。
//!
//! 不引入外部 crate：本项目已有自己的主题 / 命令 / 图标体系，照搬 kit 的**公开契约**即可：
//! - `TitleBar::window_options()` / `title_bar_options()`：窗口配置里把系统标题栏设成透明，
//!   标题栏区域完全交给应用自绘（kit 文档：「replace the default OS title bar」）。本项目对应
//!   [`title_bar_options`]，在 `CanvasView::run()` 里塞进 `WindowOptions.titlebar`。
//! - `TitleBar::new().child(…)`：左侧是应用内容（本项目放 `tabs_view::tab_strip` 的 TabBar 内容），
//!   右侧是窗口控制按钮（kit：Windows/Linux 自绘，按钮固定宽度；macOS 用系统红绿灯 + 80pt 留白）。
//! - kit 常量 `TITLE_BAR_HEIGHT = 34px`，但示例允许 `.h(px(40.))`；本项目按上游 figcraft 的 40pt。
//! - 数值保持上游 `ui-egui/src/tabs.rs`：左留白 8pt、Home 到左轨右缘 56pt、窗口按钮 46pt×3。
//!
//! **为什么不是「在内容区再塞一条」**：GPUI 0.2.1 的 `WindowOptions.titlebar` 一旦设为
//! `appears_transparent`，Windows 端 `hide_title_bar = true`，系统不再画标题栏，客户端区从
//! y=0 起，这条 40pt 容器就是窗口唯一的标题栏（系统仍处理窗口边缘缩放）。之前保留原生标题栏
//! 又在 flex_col 里插一条自绘 div，是两层标题栏，已修正。
//!
//! ## 窗口交互（P0-2，更正 2026-11-08 的旧判断）
//! GPUI 0.2.1 **有** `InteractiveElement::window_control_area(WindowControlArea)`：
//! Windows 平台在 `WM_NCHITTEST` 里把它映射成 `HTCAPTION / HTMINBUTTON / HTMAXBUTTON / HTCLOSE`
//! （`platform/windows/events.rs::handle_hit_test_msg`），随后由平台的非客户区消息处理提供
//! 「按住拖窗 / 双击最大化↔还原 / 右键系统菜单 / 最小化 / 关闭」；8 向边缘缩放则由同一函数里
//! 的 `DefWindowProcW` 命中值（`HTLEFT..HTBOTTOMLEFT`）直接交给系统。
//! 因此报告里「GPUI 无 `StartDrag` / `BeginResize`，属框架差异」**不成立**，也不需要 Win32 FFI。
//!
//! 三个按钮**不再挂 `on_click`**：GPUI 会把 NC down/up 转成客户区 MouseDown/MouseUp 派发，
//! 同时平台层自己执行窗口动作；若再挂 `zoom_window()` 等回调会双重触发，且 Windows 的
//! `zoom()` 只做 `SW_MAXIMIZE`（不还原），会把系统的 `SW_NORMAL` 覆盖掉。
//! `WinRestore` 图标随 [`Window::is_maximized`] 切换，动作交给系统保证是 toggle。
//!
//! 仍未做（别假装完成）：关闭前的未保存三问（P1 对话框框架）、Native/Custom/Inset 三态标题栏、
//! RTL mirrored 的左轨位置。

use gpui::{Div, Point, SharedString, TitlebarOptions, WindowControlArea, div, prelude::*, px};

use crate::widgets::icon::{self, Icon};
use crate::widgets::{pt, rgba, tokens};

/// 标题栏高度（上游 `tabs.rs::HEIGHT`，kit 允许通过 `.h()` 覆盖默认的 34px）。
pub const TITLE_H: f32 = 40.0;
/// 标题栏左留白（上游 `rect.min.x + 8.0`；kit 非 macOS 是 12pt）。
pub const LEFT_PAD: f32 = 8.0;
/// 一个窗口控制按钮的宽度（上游 `windows_buttons::W = 46.0`；kit 是 34pt）。
pub const CONTROL_W: f32 = 46.0;
/// 三个窗口控制按钮的总宽度（上游 `window_buttons()` 返回 `W * 3.0`）。
pub const CONTROLS_W: f32 = CONTROL_W * 3.0;

/// kit 的 `TitleBar::title_bar_options()`：隐藏系统标题栏，macOS 红绿灯留在 (9,9)。
pub fn title_bar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(SharedString::from("FigCraft")),
        appears_transparent: true,
        traffic_light_position: Some(Point::new(px(9.0), px(9.0))),
    }
}

/// 标题栏里留给 `TabBar` 内容的宽度：窗口宽 − 左留白 − 窗口控制按钮。
pub fn content_width(viewport_w: f32) -> f32 {
    (viewport_w - LEFT_PAD - CONTROLS_W).max(0.0)
}

/// 最大化按钮的图标：窗口已最大化时显示还原（上游 `WinRestore`）。
pub fn maximize_icon(maximized: bool) -> Icon {
    if maximized {
        Icon::WinRestore
    } else {
        Icon::WinMaximize
    }
}

/// 一个窗口控制按钮：hover 底色，关闭键 hover 变红（上游 `window_buttons`）。
/// 命中区通过 `window_control_area` 交给平台，动作由系统执行（见文件头）。
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
        .child(icon::paint(icon, ink, 20.0))
}

/// kit 的 `TitleBar::new().child(content)`：内容 + 右侧窗口控制区。
/// `content` 是 `TabBar`（本项目的 `tabs_view::tab_strip`），不在这里关心标签逻辑。
/// `maximized` 由调用方每帧传 `window.is_maximized()`，决定 max 键画方框还是双框。
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
        // 左留白也是拖拽区（不是 padding，padding 不属于任何可标 area 的子 hitbox）。
        .child(
            div()
                .flex_none()
                .w(pt(LEFT_PAD))
                .h_full()
                .window_control_area(WindowControlArea::Drag),
        )
        .child(div().flex_1().min_w(pt(0.0)).h_full().flex().items_center().child(content))
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
        assert_eq!(content_width(0.0), 0.0, "窄窗不产生负宽度");
    }

    #[test]
    fn maximize_button_switches_between_maximize_and_restore() {
        assert_eq!(maximize_icon(false), Icon::WinMaximize);
        assert_eq!(maximize_icon(true), Icon::WinRestore);
    }

    #[test]
    fn title_bar_options_hides_the_system_title_bar() {
        let o = title_bar_options();
        assert!(o.appears_transparent, "自绘标题栏必须让系统标题栏透明");
        assert!(o.title.is_some());
    }
}
