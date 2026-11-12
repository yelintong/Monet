//! 通用「按住拖动」行为 —— **一次编写，全局复用**。
//!
//! 此前三类手势各写了一套完全相同的状态机：
//!   * 分隔条左右 / 上下拖改视口尺寸（[`crate::resize_drag`]）；
//!   * 变量窗口及其余浮窗拖头部栏改位置（[`crate::popovers::WindowState`]）；
//!   * 通用对话框拖头部栏（[`crate::dialogs::DialogState`]）。
//!
//! 它们共有的只有这一段：按下 latch 起点与目标原点、越过死区才真正起拖、每次 move 都按
//! **相对按下点**算（绝不累积漂移）、松手 / 窗口外松手 / 未按住却移动一律收尾。
//! 夹紧范围、写入目标、命中区、事件路由各归原模块 —— 本模块不碰视图、不碰业务。
//!
//! 与 `drag_router` / `drag_manager` 的分工：那两个负责把**窗口级事件**送进视图；本模块负责
//! 一次手势的**状态**。状态是纯数据，可以嵌进任何 retained 结构体，也可以放进 thread_local。
//!
//! ## 为什么 `place` 用「按下时的原点 + 位移」
//! 拖动中卡片原点每帧都在变；若用「当前原点 + 位移」，位移会被反复叠加，窗口越拖越飞。
//! latch 时记住按下那一刻的原点，之后每一步都从它算绝对位置（等价于旧代码的
//! `grab = press − origin; place = at − grab`）。

use gpui::{Pixels, Point, px};

/// 一次移动的结果。`place` 已是绝对位置，调用方拿去夹紧即可。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// 相对按下点的位移（不累积）。
    pub delta: Point<Pixels>,
    /// 按下时原点 + 位移。
    pub place: Point<Pixels>,
}

/// 一次「按住 → 移动 → 松开」手势的状态机。
#[derive(Clone, Copy, Debug)]
pub struct HoldDrag {
    press: Option<Point<Pixels>>,
    origin: Point<Pixels>,
    latched: bool,
    dead_zone: f32,
}

impl Default for HoldDrag {
    fn default() -> Self {
        Self::new(3.0)
    }
}

impl HoldDrag {
    /// `dead_zone` 是曼哈顿距离阈值（保持旧代码的 `dx.abs() + dy.abs() < 3.0` 语义）。
    /// 分隔条不需要死区，传 0 即可。
    pub fn new(dead_zone: f32) -> Self {
        Self {
            press: None,
            origin: Point { x: px(0.0), y: px(0.0) },
            latched: false,
            dead_zone: dead_zone.max(0.0),
        }
    }

    /// 按下：只记状态、**不重绘**。GPUI 靠 `ElementId` 匹配同一次 click 的 down/up，
    /// 在两者之间重绘会让头部按钮点不动（2026-11-07 实测）。
    pub fn press(&mut self, at: Point<Pixels>, origin: Point<Pixels>) {
        self.press = Some(at);
        self.origin = origin;
        self.latched = false;
    }

    /// 已按下但还没越过死区（纯点击，不是拖动）。
    pub fn pending(&self) -> bool {
        self.press.is_some() && !self.latched
    }

    /// pending 状态下的按下点；已起拖 / 未按下返回 `None`（替代旧代码里 `Option<Point>` 的 pending 字段）。
    pub fn pending_point(&self) -> Option<Point<Pixels>> {
        if self.pending() { self.press } else { None }
    }

    /// 跳过死区、立即 latch（旧 `start_drag()` 的语义，供测试与显式起拖用）。
    pub fn latch(&mut self) -> bool {
        if self.press.is_some() {
            self.latched = true;
            true
        } else {
            false
        }
    }

    /// 已越过死区，真的在拖。
    pub fn active(&self) -> bool {
        self.latched
    }

    /// 任意状态（pending 或 active），供「取消拖拽 / 是否占用指针」判断。
    pub fn busy(&self) -> bool {
        self.press.is_some()
    }

    /// 推进一次移动。死区内或未按下 → `None`；否则 latch 并返回位移与新的绝对位置。
    pub fn advance(&mut self, at: Point<Pixels>) -> Option<Step> {
        let p = self.press?;
        let dx = f32::from(at.x) - f32::from(p.x);
        let dy = f32::from(at.y) - f32::from(p.y);
        if !self.latched {
            if dx.abs() + dy.abs() < self.dead_zone {
                return None;
            }
            self.latched = true;
        }
        let delta = Point { x: px(dx), y: px(dy) };
        Some(Step {
            delta,
            place: Point {
                x: px(f32::from(self.origin.x) + dx),
                y: px(f32::from(self.origin.y) + dy),
            },
        })
    }

    /// 单轴推进（分隔条只关心 x 或 y）：返回沿轴的相对位移。
    /// 按下时用 [`HoldDrag::press_axis`] 配对，二者都把轴值放在 `x` 上。
    pub fn advance_axis(&mut self, value: f32) -> Option<f32> {
        let at = Point { x: px(value), y: px(0.0) };
        self.advance(at).map(|s| f32::from(s.delta.x))
    }

    /// 单轴按下：把轴坐标包成点，目标原点取 0（分隔条只需要位移，不需要 place）。
    pub fn press_axis(&mut self, value: f32) {
        self.press(Point { x: px(value), y: px(0.0) }, Point { x: px(0.0), y: px(0.0) });
    }

    /// 松手 / Escape / 窗口外松手：幂等收尾，返回之前是否在忙。
    pub fn end(&mut self) -> bool {
        let was = self.busy();
        self.press = None;
        self.latched = false;
        was
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point<Pixels> {
        Point { x: px(x), y: px(y) }
    }

    #[test]
    fn a_press_alone_is_not_a_drag_and_the_dead_zone_is_manhattan() {
        let mut d = HoldDrag::default();
        d.press(p(100.0, 100.0), p(10.0, 20.0));
        assert!(d.pending());
        assert!(!d.active());
        assert_eq!(d.advance(p(101.0, 101.0)), None, "1+1=2 < 3 仍在死区");
        assert_eq!(d.advance(p(103.0, 100.0)), Some(Step { delta: p(3.0, 0.0), place: p(13.0, 20.0) }));
        assert!(d.active());
    }

    #[test]
    fn every_step_is_relative_to_the_press_never_cumulative() {
        let mut d = HoldDrag::new(0.0);
        d.press(p(100.0, 100.0), p(10.0, 20.0));
        assert_eq!(d.advance(p(150.0, 130.0)).unwrap().place, p(60.0, 50.0));
        assert_eq!(d.advance(p(160.0, 130.0)).unwrap().place, p(70.0, 50.0));
        assert_eq!(d.advance(p(160.0, 200.0)).unwrap().place, p(70.0, 120.0));
    }

    #[test]
    fn release_is_idempotent_and_clears_every_state() {
        let mut d = HoldDrag::default();
        d.press(p(0.0, 0.0), p(0.0, 0.0));
        d.advance(p(50.0, 0.0));
        assert!(d.end());
        assert!(!d.busy() && !d.pending() && !d.active());
        assert_eq!(d.advance(p(100.0, 0.0)), None);
        assert!(!d.end());
    }

    #[test]
    fn the_axis_mode_feeds_size_handles() {
        let mut d = HoldDrag::new(0.0);
        d.press_axis(336.5);
        assert_eq!(d.advance_axis(396.5), Some(60.0));
        assert_eq!(d.advance_axis(296.5), Some(-40.0));
        assert!(d.end());
        assert_eq!(d.advance_axis(500.0), None);
    }
}
