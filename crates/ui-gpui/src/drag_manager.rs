//! 通用窗口级拖拽层 —— 直接照搬 gpui-kit（`longbridge/gpui-kit`）的拖拽范式。
//!
//! ## Kit 怎么做的
//!
//! Kit 没有「通用 DragController」：每个可拖拽 widget 在自己的 `paint` 里调
//! `window.on_mouse_event(MouseMoveEvent / MouseUpEvent / MouseDownEvent)`，用一份共享状态
//! 标记「正在拖拽」，靠 `phase.bubble()` 决定是否接管。监听**每帧**在 Paint 阶段重注册
//! （GPUI 每帧清空 `next_frame.mouse_listeners`），这样指针无论停在哪个子元素上、是否移出
//! 最初按下那个元素、甚至拖出窗口（OS 捕获范围内），事件都到得了。
//!
//! 本项目 0.2.1/0.2.2 没有鼠标捕获（`capture_mouse`/`on_mouse_up_outside`），所以「在元素
//! `on_mouse_down` 起拖、靠元素 `on_mouse_move` 推进」必然断：指针一离开元素（拖到相邻面板、
//! 拖出卡片、拖到窗口外）拖拽就冻结，窗口外松手还残留状态。窗口级监听是 Kit 验证过的根治方案。
//!
//! ## 本模块提供的「通用件」
//!
//! - [`WindowDragTarget`]：任何想用窗口级拖拽的视图实现它，把 move / up / down 三类事件的
//!   派发写进 `on_window_drag_*` 三个方法；拖拽状态仍由视图自己持有（与 Kit 一致）。
//! - [`WindowDragLayer::layer`]：返回一个 `canvas` 覆盖层，其 paint 闭包在 **Paint 阶段**运行，
//!   每帧注册一次 move / up / down 三个窗口级监听，转发给 `me` 指向视图的 [`WindowDragTarget`]
//!   实现。拖拽不活跃时三个 handler 按 `phase` / 各自 `active` 判断后立刻 return，几乎零开销。

use gpui::{IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, WeakEntity, Window, canvas};

/// 实现该 trait 的视图即可获得窗口级拖拽派发。三个方法签名与 GPUI 的
/// `on_mouse_event` 回调一致（`ev` 是事件，`window`/`cx` 用于改写视图状态）。
pub trait WindowDragTarget: 'static + Sized {
    fn on_window_drag_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut gpui::Context<Self>);
    fn on_window_drag_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut gpui::Context<Self>);
    fn on_window_drag_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut gpui::Context<Self>);
}

/// 通用窗口级拖拽层。无状态：真实拖拽状态由视图（`WindowDragTarget` 实现者）持有，
/// 本结构只负责每帧在 Paint 阶段挂上三个窗口级监听并把事件转发到视图。
pub struct WindowDragLayer;

impl WindowDragLayer {
    pub fn new() -> Self {
        Self
    }

    /// 返回一个覆盖层元素：它的 paint 闭包在 Paint 阶段运行，每帧注册一次窗口级
    /// move / up / down 监听，转发给 `me` 指向视图的 [`WindowDragTarget`] 实现。
    ///
    /// 用 `canvas` 元素是因为它的 paint 闭包本身就处于 Paint 阶段——`window.on_mouse_event`
    /// 内部有 `debug_assert_paint()`，必须在 paint 阶段调用（Kit 的 `root.rs` 同样是
    /// 用 `canvas(.., move |_, _, window, _| { window.on_mouse_event(...) })` 绕过这个限制）。
    pub fn layer<E: WindowDragTarget>(&self, me: WeakEntity<E>) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |_, _, window, _cx| {
                let me_move = me.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    if let Some(me) = me_move.upgrade() {
                        me.update(cx, |v, cx| v.on_window_drag_move(ev, window, cx));
                    }
                });
                let me_up = me.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    if let Some(me) = me_up.upgrade() {
                        me.update(cx, |v, cx| v.on_window_drag_up(ev, window, cx));
                    }
                });
                let me_down = me.clone();
                window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    if let Some(me) = me_down.upgrade() {
                        me.update(cx, |v, cx| v.on_window_drag_down(ev, window, cx));
                    }
                });
            },
        )
    }
}
