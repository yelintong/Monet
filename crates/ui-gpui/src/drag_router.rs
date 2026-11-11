//! 可复用的窗口级拖拽路由器。
//!
//! ## 为什么需要它
//!
//! GPUI 0.2.1 **没有鼠标捕获**（没有 `capture_mouse` / `on_mouse_up_outside` / `SetCapture`）。
//! 因此「在元素 `on_mouse_down` 里开始拖，靠元素自身的 `on_mouse_move` / `on_mouse_up` 推进」的
//! 写法有两个必然的坑：
//!
//! 1. **元素级 `on_mouse_move` 只在指针悬停该元素时触发**——指针一离开元素（例如拖到相邻面板、
//!    拖出卡片），拖拽就当场冻结。
//! 2. **窗口外松手不会收到 `on_mouse_up`**——拖拽状态残留，指针下次回到窗口还会继续乱拖 / 误拖。
//!
//! ## 用法（与 timeline / 弹窗拖拽同源，这里抽成一处可复用）
//!
//! 任何拖拽在 `on_mouse_down` 里 [`begin`] 注册 `on_move` / `on_end` 两个闭包（闭包捕获各自模块的
//! 状态，无需持有视图句柄），再由**窗口根视图**的 `on_mouse_move` / `on_mouse_up` 统一调用
//! [`route_move`] / [`end`]。
//!
//! [`route_move`] 用事件里**真实的按键状态**判断：没按住左键（即「窗口外松手」后指针回到窗口的
//! 那一帧）就立刻 [`end`]；按住才推进 `on_move`。单槽即可——同一时刻只有一个指针在拖。
//!
//! 已有正确实现的拖拽（motion 时间轴、弹窗整卡移动）不必改动；新拖拽和遗漏的旧拖拽都走这里。

use gpui::{App, Point, Pixels, Window};

/// 一个已注册的拖拽。`on_move` 收到指针位置 + 真实按键状态；`on_end` 在松手 / 窗口外松手时提交。
struct Entry {
    kind: &'static str,
    on_move: Box<dyn FnMut(Point<Pixels>, bool, &mut Window, &mut App)>,
    on_end: Box<dyn FnOnce(&mut Window, &mut App)>,
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<Entry>> = const { std::cell::RefCell::new(None) };
}

/// 注册一个窗口级拖拽。同一时刻只允许一个（指针只有一个）。
pub fn begin(
    kind: &'static str,
    on_move: impl FnMut(Point<Pixels>, bool, &mut Window, &mut App) + 'static,
    on_end: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    ACTIVE.with(|a| *a.borrow_mut() = Some(Entry { kind, on_move: Box::new(on_move), on_end: Box::new(on_end) }));
}

/// 当前是否有拖拽在进行。
pub fn is_active() -> bool {
    ACTIVE.with(|a| a.borrow().is_some())
}

/// 当前拖拽的种类（调试 / 去重用）。
pub fn kind() -> Option<&'static str> {
    ACTIVE.with(|a| a.borrow().as_ref().map(|e| e.kind))
}

/// 窗口根视图的 `on_mouse_move` 调用。返回 `true` 表示事件已被拖拽消费。
///
/// - 当前没有拖拽 → 返回 `false`，调用方继续走别的路由。
/// - 有拖拽但按键已松开（窗口外松手）→ 立刻 [`end`]，返回 `true`。
/// - 有拖拽且仍按住 → 推进 `on_move`，返回 `true`。
pub fn route_move(pos: Point<Pixels>, pressed: bool, window: &mut Window, cx: &mut App) -> bool {
    if !is_active() {
        return false;
    }
    if !pressed {
        end(window, cx);
        return true;
    }
    ACTIVE.with(|a| {
        if let Some(e) = a.borrow_mut().as_mut() {
            (e.on_move)(pos, pressed, window, cx);
        }
    });
    true
}

/// 窗口根视图的 `on_mouse_up` 调用。结束当前拖拽（若有）并提交，返回 `true` 表示被消费。
pub fn end(window: &mut Window, cx: &mut App) -> bool {
    let e = ACTIVE.with(|a| a.borrow_mut().take());
    if let Some(e) = e {
        (e.on_end)(window, cx);
        true
    } else {
        false
    }
}
