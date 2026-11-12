//! 通用「尺寸拖动」状态机：左面板宽度（横向）与底部时间轴高度（纵向）共用一份。
//!
//! 抽出来的只是两套拖动**完全相同**的那部分——按下 latch 起点、move 按窗口轴的
//! 相对位移算新尺寸、夹紧、双击复位判定、松手/窗口外收尾。命中区（4px 覆盖条 / 顶边
//! 覆盖条）与可见 1px 描边都**不在这里**，各自就地写在 `lib.rs` 的左栏覆盖条和
//! `motion/timeline.rs` 的顶边覆盖条里。
//!
//! ## 为什么 thread_local（照搬原 `left_divider` / `timeline` 的理由）
//! - 覆盖条每帧重建，若把状态放进它自己 `Rc<RefCell<_>>`，下一次重绘就变全新空状态，
//!   move 看到 `None` 直接早退，表现是「只挪一下就断」。单例 handle 用 thread_local 跨帧存活。
//! - GPUI 逐元素 `on_mouse_move` 只在指针悬停该元素时触发；指针一离开 4px 条（拖到画布 /
//!   面板上）拖拽就断。故 move / up 由根视图 + 窗口级 `on_window_drag_*` 接管，这里提供无
//!   视图引用的 `drag_active` / `route_move` / `end_drag` 供它们调用。
//! - 新尺寸按**窗口轴的相对位移**算（`start_size + sign·(pos − start_pos)`）：面板宽度 / 时间轴
//!   高度一变，分隔条原点也跟着动，用局部坐标会「拖上去→原点反向移→局部位移不变」自我抵消。
//!
//! 两个实例（LEFT / BOTTOM）各自一份状态：方向相反、夹紧范围不同、写入目标不同，但状态机
//! 逐字共用。时间轴原本的 `DragKind::Height` + `H_START/H_START_WIN` 已并入这里，
//! timeline 的 `DRAG` 只保留 seek/duration/clip/layer 等文档相关拖动。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::hold_drag::HoldDrag;

/// 哪条尺寸手柄在拖。`axis`/`sign`/夹紧范围/写入目标由调用方在 `begin` 时传入，本模块不关心。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// 左面板竖分隔条：改 `left_w`，沿窗口 x，sign = +1（右移变宽）。
    LeftPanel,
    /// 底部时间轴顶边：改高度，沿窗口 y，sign = -1（上移变高）。
    BottomTimeline,
}

/// 哪条尺寸边的 4px 热区正被悬停，供**容器自带的那条 1px 描边**变强调色。
/// 与 `Handle` 分开：右栏拖动走旧 `resizing` 状态机，但也有 hover 高亮，故单独一份。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// 左面板右缘（panel | canvas）。
    Left,
    /// 右面板左缘（canvas | panel）。
    Right,
    /// 底部时间轴顶缘。
    Bottom,
}

/// 按下时 latch 的起点。`pos` 是沿拖动轴的窗口坐标（左栏 x、底栏 y）。
#[derive(Clone, Copy, Debug)]
struct DragState {
    start_size: f32,
    sign: f32,
    min: f32,
    max: f32,
    /// 按下 latch → 相对位移 → 收尾，与浮窗拖动共用 [`HoldDrag`]（死区 0：分隔条即时响应）。
    hold: HoldDrag,
}

thread_local! {
    /// 拖拽状态：跨帧持久，不能用 per-frame `Rc`。两个 handle 各自独立，避免互相清零。
    static DRAG: [RefCell<Option<DragState>>; 2] = const {
        [RefCell::new(None), RefCell::new(None)]
    };
    /// 双击复位检测（时间 + 窗口坐标），与两套旧实现同式：400ms 内、4pt 邻域。
    static LAST_PRESS: [RefCell<Option<(Instant, f32, f32)>>; 2] = const {
        [RefCell::new(None), RefCell::new(None)]
    };
    /// 当前悬停的尺寸边（同一时刻至多一条：鼠标只在一个 4px 热区里）。
    /// 由覆盖条的 `on_hover` 写入；容器的 1px 描边在 render 时读它决定常态 / 强调色。
    static HOVER: RefCell<Option<Edge>> = const { RefCell::new(None) };
}

/// 覆盖条 hover 进入 / 离开时调用。`on=false` 只清掉自己，避免 A 的离开误清刚进入的 B。
pub fn set_hover(e: Edge, on: bool) {
    HOVER.with(|h| {
        let mut h = h.borrow_mut();
        if on {
            *h = Some(e);
        } else if *h == Some(e) {
            *h = None;
        }
    });
}

/// 指定边是否正被悬停。
pub fn hovered(e: Edge) -> bool {
    HOVER.with(|h| *h.borrow() == Some(e))
}

#[inline]
fn idx(h: Handle) -> usize {
    match h {
        Handle::LeftPanel => 0,
        Handle::BottomTimeline => 1,
    }
}

/// 是否有任意尺寸拖动正在进行（根视图 / 窗口级据此接管 move / up，并在 `dragging_heavy` 里跳过重活）。
pub fn any_active() -> bool {
    DRAG.with(|slots| slots.iter().any(|s| s.borrow().is_some()))
}

/// 指定 handle 是否正在拖。
pub fn drag_active(h: Handle) -> bool {
    DRAG.with(|slots| slots[idx(h)].borrow().is_some())
}

/// 按下：latch 起点与夹紧范围。返回 true 表示双击（调用方据此复位到默认值并 `end_drag`）。
///
/// 双击判定不依赖 GPUI 的 `click_count`：拖拽可能让两次 click 落在不同元素 id 上导致它丢失；
/// 时间 + 窗口坐标更稳，且与两套旧实现逐字一致。
pub fn begin(
    h: Handle,
    sign: f32,
    start_size: f32,
    start_pos: f32,
    min: f32,
    max: f32,
    win_x: f32,
    win_y: f32,
) -> bool {
    let i = idx(h);
    DRAG.with(|slots| {
        let mut hold = HoldDrag::new(0.0);
        hold.press_axis(start_pos);
        *slots[i].borrow_mut() = Some(DragState { start_size, sign, min, max, hold });
    });
    let now = Instant::now();
    let double = LAST_PRESS.with(|slots| {
        slots[i].borrow().is_some_and(|(t, x, y)| {
            now.duration_since(t) < Duration::from_millis(400)
                && (x - win_x).abs() < 4.0
                && (y - win_y).abs() < 4.0
        })
    });
    LAST_PRESS.with(|slots| *slots[i].borrow_mut() = Some((now, win_x, win_y)));
    double
}

/// 窗口级 move：按沿轴相对位移算并夹紧新尺寸。非拖拽态返回 `None`（调用方据此让位给其它拖拽）。
/// `pos` 传左栏的窗口 x 或底栏的窗口 y。
pub fn route_move(h: Handle, pos: f32) -> Option<f32> {
    DRAG.with(|slots| {
        let mut slot = slots[idx(h)].borrow_mut();
        let s = slot.as_mut()?;
        // 位移由通用按住拖动状态机给出（始终相对按下点，不随原点移动自我抵消）。
        let delta = s.hold.advance_axis(pos)?;
        Some((s.start_size + s.sign * delta).clamp(s.min, s.max))
    })
}

/// 收尾：清该 handle 的拖拽状态。返回 true 表示刚才确实在拖（供 `drag_just_ended` 用）。
/// 幂等：没在拖也安全，供根 `on_mouse_up` / 窗口级 `on_window_drag_up` 多处都调。
pub fn end_drag(h: Handle) -> bool {
    DRAG.with(|slots| slots[idx(h)].borrow_mut().take().is_some())
}

/// 收尾所有尺寸拖动（三条 up 兜底路径统一调），返回是否有任意一个刚结束。
pub fn end_any() -> bool {
    DRAG.with(|slots| slots.iter().fold(false, |acc, s| acc | s.borrow_mut().take().is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_is_relative_to_axis_and_clamped() {
        assert!(!end_any(), "测试线程不应继承别的拖拽状态");
        // 左栏：sign +1，右移 +60 → 加宽 60。
        assert_eq!(begin(Handle::LeftPanel, 1.0, 280.0, 336.5, 240.0, 500.0, 336.5, 430.0), false);
        assert!(drag_active(Handle::LeftPanel));
        assert_eq!(route_move(Handle::LeftPanel, 336.5 + 60.0), Some(340.0));
        assert_eq!(route_move(Handle::LeftPanel, 0.0), Some(240.0));
        assert_eq!(route_move(Handle::LeftPanel, 9999.0), Some(500.0));
        assert!(end_drag(Handle::LeftPanel));
        assert!(!drag_active(Handle::LeftPanel));
        assert_eq!(route_move(Handle::LeftPanel, 400.0), None, "收尾后不再路由");
        assert!(!end_drag(Handle::LeftPanel), "end_drag 幂等");
    }

    #[test]
    fn bottom_sign_is_inverted_and_uses_window_y() {
        assert!(!end_any());
        // 底栏：sign -1，上移（y 减小）60 → 变高 60。
        assert_eq!(begin(Handle::BottomTimeline, -1.0, 309.0, 600.0, 160.0, 800.0, 400.0, 600.0), false);
        assert_eq!(route_move(Handle::BottomTimeline, 540.0), Some(369.0));
        assert_eq!(route_move(Handle::BottomTimeline, 700.0), Some(209.0));
        assert_eq!(route_move(Handle::BottomTimeline, 5000.0), Some(160.0));
        end_any();
    }

    #[test]
    fn width_independent_of_origin_shift() {
        assert!(!end_any());
        begin(Handle::LeftPanel, 1.0, 280.0, 336.5, 240.0, 500.0, 336.5, 430.0);
        let w1 = route_move(Handle::LeftPanel, 346.5).unwrap();
        let w2 = route_move(Handle::LeftPanel, 356.5).unwrap();
        let w3 = route_move(Handle::LeftPanel, 376.5).unwrap();
        assert_eq!((w1, w2, w3), (290.0, 300.0, 320.0), "始终相对起点，不自我累积 / 抵消");
        end_any();
    }

    #[test]
    fn handles_are_independent_and_double_click_is_per_handle() {
        assert!(!end_any());
        // 左栏先按下，底栏再按下：两者互不干扰。
        begin(Handle::LeftPanel, 1.0, 280.0, 100.0, 240.0, 500.0, 100.0, 200.0);
        begin(Handle::BottomTimeline, -1.0, 309.0, 600.0, 160.0, 800.0, 400.0, 600.0);
        assert!(drag_active(Handle::LeftPanel));
        assert!(drag_active(Handle::BottomTimeline));
        assert_eq!(route_move(Handle::LeftPanel, 160.0), Some(340.0));
        assert_eq!(route_move(Handle::BottomTimeline, 560.0), Some(349.0));
        // 双击：同一 handle 400ms 内、4pt 邻域的第二次按下。
        assert_eq!(begin(Handle::LeftPanel, 1.0, 280.0, 101.0, 240.0, 500.0, 101.0, 201.0), true);
        assert_eq!(begin(Handle::LeftPanel, 1.0, 280.0, 200.0, 240.0, 500.0, 200.0, 200.0), false, "偏离 >4pt 不算双击");
        assert!(end_any());
        assert!(!any_active());
    }
}
