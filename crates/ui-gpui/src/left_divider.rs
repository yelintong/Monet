//! 左面板分隔条的窗口级拖拽状态机 —— 与 `motion/timeline.rs` 的顶边拖动**同构**。
//!
//! 为什么单独成模块 + thread_local（照搬 timeline 的理由，逐条对应）：
//! - `divider_hotspot()` 每帧重建，若在它里面用 `Rc<RefCell<…>>` 存拖拽状态，按下设的状态
//!   会在紧接着的重绘后变成全新空状态 → move 看到 `None` 直接早退，表现是「只挪一下就断」。
//!   时间轴单例、左分隔条也单例，故用 thread_local 跨帧存活。
//! - GPUI 的逐元素 `on_mouse_move` 只在指针悬停该元素时触发；指针一离开 4px 覆盖条（拖到
//!   画布 / 面板上）拖拽就断。故 move / up 由根视图 + 窗口级 `on_window_drag_*` 接管，这里
//!   提供无视图引用的 `drag_active` / `route_move` / `end_drag` 供它们调用。
//! - 宽度按**窗口 x 的相对位移**算（`start_w + (win_x − start_x)`），与 timeline 顶边用
//!   窗口 y 同理：面板宽度一变，分隔条原点也跟着动，用局部坐标会自我抵消、永远拖不动。
//!
//! 与 timeline 有意不同的两点（结果一致、更安全）：
//! - **命中带**仍由 `divider_hotspot` 的 4px GPUI 覆盖条承担（1px 线 + 左右各 1.5px，
//!   已真鼠标实测），本模块不碰命中，只管按下之后的状态 / 路由。
//! - **光标**：hover 用覆盖条上的 hitbox 级 `.cursor(ResizeLeftRight)`；拖动中用整窗级
//!   `ClosedHand`（`cursor()`，仅 paint 阶段、仅 active 期间）。不把「整窗光标」扩散到
//!   hover 常态，否则会压掉右栏输入框的 `ResizeLeftRight`（见 lib.rs:4139 / 4341 的坑）。
//!
//! 右面板分隔条维持现状（`CanvasView.resizing = Some(Right)` + 根 div 内联路由），本模块
//! 只服务 Left —— 用户本次只要求「左栏和底下一样」。

use crate::panels;

/// 按下时 latch 的拖拽起点。`start_w` 是当时的左面板宽度，`start_x` 是窗口 x。
#[derive(Clone, Copy, Debug, PartialEq)]
struct DragState {
    start_w: f32,
    start_x: f32,
}

thread_local! {
    /// 拖拽状态：跨帧持久，不能用 per-frame `Rc`。
    static DRAG: std::cell::RefCell<Option<DragState>> = const { std::cell::RefCell::new(None) };
    /// 双击复位检测（时间 + 窗口坐标），与 timeline 的 LAST_PRESS 同式。
    static LAST_PRESS: std::cell::RefCell<Option<(std::time::Instant, f32, f32)>> =
        const { std::cell::RefCell::new(None) };
}

/// 是否正在拖左分隔条（窗口级 / 根视图据此接管 move / up，并在 `dragging_heavy` 里跳过重活）。
pub fn drag_active() -> bool {
    DRAG.with(|d| d.borrow().is_some())
}

/// 按下：latch 起点。返回 true 表示双击（按下方据此复位到 `DEFAULT_PANEL`）。
///
/// 双击判定与 timeline 一致：400ms 内、同一 4pt 邻域。GPUI 元素虽带 `click_count`，这里不
/// 依赖它——拖拽可能让两次 click 落在不同元素 id 上，`click_count` 会丢；时间 + 坐标更稳，
/// 且与 timeline 顶边双击同构。
pub fn begin(start_w: f32, win_x: f32, win_y: f32) -> bool {
    DRAG.with(|d| *d.borrow_mut() = Some(DragState { start_w, start_x: win_x }));
    let now = std::time::Instant::now();
    let double = LAST_PRESS.with(|lp| {
        lp.borrow().is_some_and(|(t, x, y)| {
            now.duration_since(t) < std::time::Duration::from_millis(400)
                && (x - win_x).abs() < 4.0
                && (y - win_y).abs() < 4.0
        })
    });
    LAST_PRESS.with(|lp| *lp.borrow_mut() = Some((now, win_x, win_y)));
    double
}

/// 窗口级 move：按窗口 x 的相对位移算新宽度，夹紧 `MIN_PANEL..=MAX_PANEL`。
/// 非拖拽态返回 `None`（调用方据此让位给其它拖拽）。
pub fn route_move(win_x: f32) -> Option<f32> {
    DRAG.with(|d| {
        d.borrow().map(|s| (s.start_w + (win_x - s.start_x)).clamp(panels::MIN_PANEL, panels::MAX_PANEL))
    })
}

/// 收尾：清拖拽状态。返回 true 表示刚才确实在拖（供 `drag_just_ended` 用）。
/// 幂等：没在拖也安全，供根 `on_mouse_up` / 窗口级 `on_window_drag_up` 两处都调。
pub fn end_drag() -> bool {
    DRAG.with(|d| d.borrow_mut().take().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_is_relative_to_window_x_and_clamped() {
        assert!(!drag_active(), "测试线程不应继承别的拖拽状态");
        assert_eq!(begin(280.0, 336.5, 430.0), false, "首次按下不是双击");
        assert!(drag_active());
        // 往右拖 60pt -> 加宽 60；相对位移与面板当前宽度无关。
        assert_eq!(route_move(336.5 + 60.0), Some(340.0));
        // 往左拖到下限、往右拖到上限都夹紧。
        assert_eq!(route_move(0.0), Some(panels::MIN_PANEL));
        assert_eq!(route_move(9999.0), Some(panels::MAX_PANEL));
        assert!(end_drag());
        assert!(!drag_active());
        assert_eq!(route_move(400.0), None, "收尾后不再路由");
        assert!(!end_drag(), "end_drag 幂等");
    }

    #[test]
    fn double_click_within_400ms_and_4pt() {
        assert!(!end_drag(), "确保从干净状态开始");
        assert_eq!(begin(280.0, 100.0, 200.0), false);
        assert!(end_drag());
        // 同一位置、紧接着再按 -> 双击。
        assert_eq!(begin(280.0, 101.0, 201.0), true, "4pt 内的第二次按下算双击");
        assert!(end_drag());
        // 位置偏离 4pt 以上不算双击。
        assert_eq!(begin(280.0, 200.0, 200.0), false);
        assert!(end_drag());
    }

    #[test]
    fn width_is_independent_of_panel_origin_shift() {
        // 模拟拖动中面板原点随宽度变化：route 只认「起点宽度 + 窗口 x 位移」，
        // 不读当前 left_w，所以即使布局每帧把分隔条挪走，宽度仍按指针相对位移连续增长。
        assert!(!end_drag());
        begin(280.0, 336.5, 430.0);
        let w1 = route_move(346.5).unwrap(); // +10
        let w2 = route_move(356.5).unwrap(); // +20（不是基于 w1 再叠，是相对起点）
        let w3 = route_move(376.5).unwrap(); // +40
        assert_eq!((w1, w2, w3), (290.0, 300.0, 320.0), "始终相对起点，不自我累积 / 抵消");
        end_drag();
    }
}
