//! 工作区 Dock 布局 —— 参考 GPUI Kit 的 `DockArea` / `DockLayout` / `DockPlacement`。
//!
//! kit 的关键不是多画几条分隔线，而是把布局变成**一份可计算的值**：中心区域 + 左 / 右 / 下
//! 三个 dock，每个区域有稳定尺寸，面板内容与几何计算分离。此前本项目把
//! `RAIL_W + left_w + HANDLE_W`、`vp.height - motion_bar_h()` 这类补偿散落在 render、
//! 命中换算、弹层锚点里，还因为客户区顶部塞了一条自绘标题栏而漏减 40pt，造成画布点击
//! 热区整体偏移。这里把所有几何收拢到同一份 [`DockLayout`]。
//!
//! ## 窗口标题栏不属于 Dock
//! 系统级窗口标题区域（见 [`crate::title_bar`]）由 `WindowOptions.titlebar` 声明，整条通过
//! `WindowControlArea` 交给平台（拖窗 / 双击 / 按钮），是 Root 的第一个 child；TabBar 内容
//! 放在它里面，不是画布里的一条。客户区（Dock）从 y = `TITLEBAR_H` 开始，所以
//! `canvas_origin().y = TITLEBAR_H`，所有命中换算都从这里推导。
//!
//! ```
//! titlebar:   系统级窗口标题区域              (高 titlebar_h, Root 第一层)
//! left dock:  rail + 左面板                   (宽 rail_w + left_w)
//! handle:     左分隔条                       (宽 handle_w)
//! center:     画布                           (宽 center_w, 高 vp.h - titlebar_h - bottom_h)
//! handle:     右分隔条                       (宽 handle_w)
//! right dock: Design / Motion 面板           (宽 right_w)
//! bottom:     Motion 时间轴                  (高 bottom_h，折叠时为 0)
//! ```

use crate::panels;
use crate::title_bar::TITLE_H;

/// 系统级窗口标题区域的高度：客户区（Dock）从它下面开始。
pub const TITLEBAR_H: f32 = TITLE_H;

/// kit 的四个区域。本项目 left dock 由「活动栏 + 左面板」合成，center 是画布与其页签组。
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Placement {
    Left,
    Center,
    Right,
    Bottom,
}

/// 一帧的布局值：纯数据，可在不持有 `Window` / `App` 的情况下计算、比较与测试。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DockLayout {
    pub rail_w: f32,
    pub left_w: f32,
    pub right_w: f32,
    pub handle_w: f32,
    pub titlebar_h: f32,
    pub bottom_h: f32,
}

impl DockLayout {
    /// `left_w` / `right_w` 来自可拖拽的 dock 尺寸，`bottom_h` 来自 bottom dock 的折叠状态。
    pub fn new(left_w: f32, right_w: f32, bottom_h: f32) -> Self {
        Self {
            rail_w: panels::RAIL_W,
            left_w,
            right_w,
            handle_w: panels::HANDLE_W,
            titlebar_h: TITLEBAR_H,
            bottom_h,
        }
    }

    /// center dock 的左边缘（窗口坐标）：rail + left dock + 左分隔条。
    pub fn center_x(&self) -> f32 {
        self.rail_w + self.left_w + self.handle_w
    }

    /// center dock 的宽度：窗口宽减去左 / 右 dock 与两条分隔条。
    pub fn center_w(&self, viewport_w: f32) -> f32 {
        (viewport_w - self.center_x() - self.handle_w - self.right_w).max(1.0)
    }

    /// 画布在窗口中的原点：center 左边缘 + 系统级标题栏高度。
    pub fn canvas_origin(&self) -> (f32, f32) {
        (self.center_x(), self.titlebar_h)
    }

    /// 画布尺寸：center 宽 × (窗口高 − 标题栏 − bottom dock)。
    pub fn canvas_size(&self, viewport_w: f32, viewport_h: f32) -> (f32, f32) {
        (
            self.center_w(viewport_w),
            (viewport_h - self.titlebar_h - self.bottom_h).max(1.0),
        )
    }

    /// 窗口坐标 → 画布局部坐标（引擎里的 `app.canvas.rect` 原点恒为 0）。
    pub fn to_canvas(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.center_x(), y - self.titlebar_h)
    }

    /// right dock 的左边缘（窗口坐标）。
    pub fn right_x(&self, viewport_w: f32) -> f32 {
        (viewport_w - self.right_w).max(0.0)
    }

    /// bottom dock 的顶边（窗口坐标）。
    pub fn bottom_y(&self, viewport_h: f32) -> f32 {
        (viewport_h - self.bottom_h).max(0.0)
    }

    /// 某个区域的尺寸（kit `DockPlacement` 的等价查询）。
    pub fn extent(&self, placement: Placement, viewport_w: f32, viewport_h: f32) -> (f32, f32) {
        match placement {
            Placement::Left => (self.rail_w + self.left_w, viewport_h - self.titlebar_h - self.bottom_h),
            Placement::Center => (self.center_w(viewport_w), viewport_h - self.titlebar_h - self.bottom_h),
            Placement::Right => (self.right_w, viewport_h - self.titlebar_h - self.bottom_h),
            Placement::Bottom => (viewport_w, self.bottom_h),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> DockLayout {
        DockLayout::new(240.0, 320.0, 120.0)
    }

    #[test]
    fn canvas_origin_is_center_left_edge_plus_the_titlebar() {
        let d = layout();
        assert_eq!(d.canvas_origin(), (d.center_x(), TITLEBAR_H));
    }

    #[test]
    fn canvas_size_excludes_titlebar_and_bottom_dock() {
        let d = layout();
        let (w, h) = d.canvas_size(1280.0, 860.0);
        assert_eq!(w, d.center_w(1280.0));
        assert_eq!(h, 860.0 - TITLEBAR_H - 120.0);
        assert_eq!(d.bottom_y(860.0), 740.0);
    }

    #[test]
    fn to_canvas_inverts_the_origin() {
        let d = layout();
        let (ox, oy) = d.canvas_origin();
        assert_eq!(d.to_canvas(ox + 25.0, oy + 40.0), (25.0, 40.0));
    }

    #[test]
    fn a_collapsed_bottom_dock_gives_the_canvas_its_height() {
        let mut d = layout();
        d.bottom_h = 0.0;
        assert_eq!(d.canvas_size(1280.0, 860.0).1, 860.0 - TITLEBAR_H);
        assert_eq!(d.bottom_y(860.0), 860.0);
    }
}
