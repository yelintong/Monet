// ============================================================================
// 新增通用控件（2026-11-05 组装）— 从 `ui-egui/src/popover.rs` 迁移。
// INVARIANT（禁止破坏）：
//   - 标题位置是**插槽**（`title(impl IntoElement)`），不是 `String`；任何调用方都能放自定义内容。
//   - 卡片规格：240 宽 / 圆角 13 / 默认 pad 16 / X 24×24 浮在右上角（右 8、上 8）。
//   - 本控件只画「窗口外壳」，不碰业务数据、不决定自己何时打开关闭。
//
// 2026-11-07 用户明确授权本次修改：**整个小窗口就是一个插槽**（除了右上角 X 的区域），
// 不再有独立的 40pt 标题栏 + 下面另起一栏 body；`title()` 只是插槽里的第一块内容，
// 拖动绑在整张卡片上（输入框会 `stop_propagation`，不会误拖）。
//
// 2026-11-12 用户再次明确结构（本次修改只落实这三条，不改动架构）：
//   1. 头部栏 40pt 高的整块区域，除右上角 X 占的区域外，**整体就是一个插槽**（不是一行文字）；
//   2. 底下放东西的是**第二个插槽**，紧贴头部栏；8pt 是这个 body 插槽**内部各项之间**的间距
//      （由 [`popover_body`] 统一给出），不是两个插槽之间的间距；
//   3. 拖动是控件的**通用能力**，统一绑在头部栏插槽上，不是每个调用方各写一套。
//   头部栏内容垂直居中；X 仍浮在右上角、不占布局行。
// ============================================================================
//! The chrome every detached panel shares — the GPUI port of `ui-egui/src/popover.rs`.
//!
//! This is the window **behind** the colour picker (and the layout settings, stroke settings,
//! variable picker and style picker): a card with a 40 pt header, a hairline under it, the theme's
//! popup shadow and 13 pt corners, wrapped around one body slot.
//!
//! The one deliberate change against the egui original: **the header's title is a slot**, not a
//! `String`. `title()` takes any element — plain text via [`title_text`], or a swatch + label, a
//! tab strip, a button, anything. The X, the drag handle, the hairline and the body slot stay put.
//!
//! Placement mirrors the original: [`Open::Anchor`] puts the card's right edge on the anchor and
//! lifts it when its bottom would pass the screen; [`Open::At`] puts its top-left exactly there.
//! `at()` overrides both (a dragged card). Everything is clamped inside the viewport.

use crate::canvas::geom::Color32;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{pt, rgba, tokens};
use figcraft_i18n::tr;
use gpui::{
    AnyElement, App, BoxShadow, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString, Size, Window, div, point,
    prelude::*, px,
};
use std::rc::Rc;

/// The default width: the colour picker's, and the one every detached panel shares.
pub const WIDTH: f32 = 240.0;
/// The header's height: the whole 40 pt band is the title slot (minus the X's corner).
pub const HEADER_H: f32 = 40.0;
/// The vertical gap between **items inside the body slot** (2026-11-12).
pub const ITEM_GAP: f32 = 8.0;
/// The title starts this far from the panel's left edge, the X ends this far from its right one.
pub const TITLE_LEFT: f32 = 16.0;
pub const CLOSE_RIGHT: f32 = 8.0;
/// The X's button (the icon is inset in it).
pub const CLOSE: f32 = 24.0;
/// The body slot's default margin inside the panel.
pub const SLOT_PAD: f32 = 16.0;
/// A lifted popover keeps this much screen under it; it stays [`EDGE`] inside the others.
pub const SCREEN_GAP: f32 = 16.0;
pub const EDGE: f32 = 8.0;
/// The panel's corners.
pub const RADIUS: f32 = 13.0;

/// The surface a popover draws on: the panels' own, or the dark one the style picker uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Panel,
    Menu,
}

/// Where a popover opens when the caller has not put it down itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Open {
    /// Its right edge on `anchor.x` (the right panel's left edge), its top on `anchor.y`, lifted
    /// when that would run its bottom past the screen's bottom.
    Anchor(Point<Pixels>),
    /// Its top-left exactly here.
    At(Point<Pixels>),
}

/// A generic detached window. Build one with [`popover_window`] / [`Popover::new`], give it a
/// [`Popover::title`] slot and a [`Popover::body`] slot, then [`Popover::show`] it.
pub struct Popover {
    open: Open,
    /// `Some` once the header has been dragged (egui's `Place::At`).
    at: Option<Point<Pixels>>,
    width: f32,
    /// The card's estimated height — the anchor's lift and the clamp both need it before layout.
    height: f32,
    pad: f32,
    surface: Surface,
    title: Option<AnyElement>,
    body: Option<AnyElement>,
    on_drag: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    /// 整卡 `on_mouse_move` 的回调：把卡片上的移动路由到根视图的拖拽状态机，使「按下后指针移出
    /// 最初元素」时拖拽仍连续（与根 `on_mouse_move` 接管同构；编辑器 body 自身的 `on_mouse_move`
    /// 只处理内置拖拽，不负责整卡移动，这里补上窗口级路由）。
    on_move: Option<Rc<dyn Fn(&MouseMoveEvent, &mut Window, &mut App)>>,
    /// 卡片上的 **up** 收尾：弹窗是 `occlude()` 浮层，指针停在其上方时根 div 的 up 被吞，
    /// 松手必须能在卡片这一层收到，否则拖拽状态残留、「放不下」。与 `on_move` 对称。
    on_up: Option<Rc<dyn Fn(&gpui::MouseUpEvent, &mut Window, &mut App)>>,
    /// 全屏呈现（egui `Frame::NONE`）：不画圆角/阴影、不加内边距，位置也不被 `clamp` 夹离。
    flush: bool,
}

impl Popover {
    /// A popover that opens at `open`, on the panel surface, 240 pt wide with the default pad.
    pub fn new(open: Open) -> Self {
        Self {
            open,
            at: None,
            width: WIDTH,
            height: HEADER_H + SLOT_PAD * 2.0,
            pad: SLOT_PAD,
            surface: Surface::Panel,
            title: None,
            body: None,
            on_drag: None,
            on_move: None,
            on_up: None,
            flush: false,
        }
    }

    /// Fullscreen presentation (egui `Frame::NONE`): no card padding, no rounded corners, no shadow,
    /// and the position is used as-is (not clamped) — the full view is flush against the editor area.
    pub fn flush(mut self, flush: bool) -> Self {
        self.flush = flush;
        self
    }

    /// `width` points wide instead of [`WIDTH`].
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(1.0);
        self
    }

    /// The card's estimated height (used only to lift / clamp it before the first layout).
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(HEADER_H);
        self
    }

    /// `pad` points of margin around the body slot instead of the default 16.
    pub fn pad(mut self, pad: f32) -> Self {
        self.pad = pad.max(0.0);
        self
    }

    /// Drawn on the dark surface the style browser uses.
    pub fn surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }

    /// Its top-left at `pos` instead of where [`Open`] would put it — a dragged card.
    pub fn at(mut self, pos: Point<Pixels>) -> Self {
        self.at = Some(pos);
        self
    }

    /// **The title slot.** Anything can go here: [`title_text`] for the original's 11 pt semibold
    /// label, or a swatch + label + buttons. The X and the drag handle are not part of the slot.
    pub fn title(mut self, el: impl IntoElement) -> Self {
        self.title = Some(el.into_any_element());
        self
    }

    /// The body slot, inset by `pad` on every side (egui's `Frame::inner_margin`).
    pub fn body(mut self, el: impl IntoElement) -> Self {
        self.body = Some(el.into_any_element());
        self
    }

    /// Called when the header (everything but the X) is pressed — the caller owns the drag state,
    /// since a retained tree keeps the card's position between frames.
    pub fn on_drag_start(mut self, f: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_drag = Some(Rc::new(f));
        self
    }

    /// 整卡移动 / resize 的窗口级 `on_mouse_move` 路由：把卡片上的移动事件交给根视图的拖拽状态机
    /// （GPUI 元素级 `on_mouse_move` 只在指针悬停其命中框时触发，指针移出卡片后拖拽会断——这里在
    /// 卡片这一层补一个，保证 move 一路路由到 `CanvasView::move_popup_drag`）。
    pub fn on_drag_move(mut self, f: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_move = Some(Rc::new(f));
        self
    }

    /// 整卡 `on_mouse_up` 的回调：与 `on_drag_move` 对称。弹窗 `occlude()` 会吞掉根 div 的 up，
    /// 指针停在卡片上方松手时只有卡片自己能收到——拖拽收尾（`end_drag`）必须绑在这里，
    /// 否则状态残留、鼠标一动卡片又跟着走（「放不下」）。
    pub fn on_drag_up(mut self, f: impl Fn(&gpui::MouseUpEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_up = Some(Rc::new(f));
        self
    }

    /// Where this card sits in `viewport`, after lift and clamp. Exposed so a drag can seed its
    /// state from the exact same origin the card is drawn at.
    pub fn origin(&self, viewport: Size<Pixels>) -> Point<Pixels> {
        let p = match self.at {
            Some(p) => p,
            None => match self.open {
                Open::At(p) => p,
                Open::Anchor(a) => open_pos(a, self.width, self.height, viewport),
            },
        };
        // Fullscreen sits flush at the editor area's origin; clamping to EDGE (8) would push it off
        // and, being viewport-wide, overflow the right/bottom. So fullscreen keeps the raw position.
        if self.flush { p } else { clamp(p, self.width, self.height, viewport) }
    }

    /// Draws the card and returns `(origin, element)`. `dismiss` is wired to the X; Escape and
    /// outside presses stay with the view that owns the popup (the egui original does the same).
    ///
    /// 2026-11-12 结构：**两个插槽**。40pt 的头部栏除右上角 X 占的区域外整块是 title 插槽
    /// （内容垂直居中、通用拖动绑在这里），下面 8pt 处是 body 插槽。X 浮在右上角不占布局行。
    pub fn show(self, viewport: Size<Pixels>, dismiss: Rc<dyn Fn(&mut Window, &mut App)>) -> (Point<Pixels>, AnyElement) {
        let origin = self.origin(viewport);
        let t = tokens();
        let dark = self.surface == Surface::Menu;
        let fill = if dark { t.menu } else { t.panel };
        let hover = if dark { Color32::from_white_alpha(20) } else { t.hover };

        // 全屏时没有内边距（egui `Frame::NONE` + `inner_margin(0)`）。
        let pad = if self.flush { 0.0 } else { self.pad };
        // 卡片 = 两个插槽。头部栏**贴卡片边缘**（egui 的 40pt header 不吃 body 的 pad），
        // 只有 body 插槽带 pad；X 绝对定位、(40−24)/2 垂直居中于头部栏，标题与 X 才同一水平线。
        // 两个插槽之间**没有**额外间距；8pt 是 body 插槽内部各项之间、由 [`popover_body`] 给。
        let mut stack = div().w_full().flex().flex_col();
        if let Some(title) = self.title {
            // 头部栏插槽：整块 40pt 都是插槽，内容垂直居中；左 16、右让开 X 的 24 + 8。
            let mut head = div().w_full().h(pt(HEADER_H)).flex().items_center().pl(pt(TITLE_LEFT)).pr(pt(CLOSE + CLOSE_RIGHT)).child(title);
            // ⛔ 通用拖动能力只绑在头部栏插槽，不是整张卡片：绑整卡会让点 body 里任何
            // 地方都开始拖窗口（用户 2026-11-07 指出）。X 自己 stop_propagation。
            if let Some(f) = self.on_drag.clone() {
                head = head.cursor_grab().on_mouse_down(MouseButton::Left, move |e, window, cx| f(e, window, cx));
            }
            stack = stack.child(head);
        }
        if let Some(body) = self.body {
            stack = stack.child(div().w_full().flex().flex_col().p(pt(pad)).child(body));
        }

        // X 浮在右上角，不占布局行。
        let close = div()
            .absolute()
            .right(pt(CLOSE_RIGHT))
            // 24×24 的 X 垂直居中于 40pt 头部栏：(40 − 24) / 2 = 8。
            .top(pt((HEADER_H - CLOSE) / 2.0))
            .size(pt(CLOSE))
            .rounded(pt(crate::widgets::RADIUS))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |s| s.bg(rgba(hover)))
            .on_mouse_down(MouseButton::Left, {
                let dismiss = dismiss.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    dismiss(window, cx);
                }
            })
            .child(paint_icon(Icon::Close, rgba(if dark { t.menu_text } else { t.icon }), 12.0));

        // 拖动：整张卡片（除 X 以外）都是拖动区。
        let mut card =
            div().absolute().left(pt(f32::from(origin.x))).top(pt(f32::from(origin.y))).w(pt(self.width)).flex().flex_col().bg(rgba(fill)).occlude();
        // 全屏 = egui `Frame::NONE`：不要圆角、不要阴影，贴边铺满编辑器区。
        if !self.flush {
            card = card.rounded(pt(RADIUS)).shadow(vec![BoxShadow {
                color: rgba(t.shadow).into(),
                offset: point(px(0.0), px(4.0)),
                blur_radius: px(16.0),
                spread_radius: px(0.0),
            }]);
        }
        // 拖动区已绑在标题行（见上），整张卡片不再接收拖动按下。
        // 整卡 move 路由：与 `on_drag_start` 同构，把卡片上的移动交给根视图的拖拽状态机。
        // 不 `stop_propagation`——body 的内置拖拽回调先跑，再冒泡到根；这里只是补一层保险，
        // 且 `move_popup_drag` 对「无拖拽中」是幂等 no-op，重复触发无害。
        if let Some(f) = self.on_move.clone() {
            card = card.on_mouse_move(move |e: &MouseMoveEvent, window, cx| f(e, window, cx));
        }
        // 卡片层 up：弹窗 occlude 吞掉根 div 的 up，松手停在卡片上方时靠这里收尾，
        // 与根 div up / cancel_drag_if_not_held 三路都调同一个幂等 end_drag。
        if let Some(f) = self.on_up.clone() {
            card = card.on_mouse_up(MouseButton::Left, move |e: &gpui::MouseUpEvent, window, cx| f(e, window, cx));
        }
        (origin, card.child(stack).child(close).into_any_element())
    }
}

/// [`Popover::new`].
pub fn popover_window(open: Open) -> Popover {
    Popover::new(open)
}

/// The default **title slot**: the original's 11 pt semibold label. Callers that want something
/// richer simply pass their own element to [`Popover::title`].
pub fn title_text(text: impl Into<SharedString>) -> gpui::Div {
    // egui 用 `elide_font` 按可用宽度裁标题；GPUI 这边保持同一效果：不换行 + 溢出隐藏。
    // `h_full + items_center` 让标题在 40pt 头部栏插槽里真正垂直居中（只靠外层 items_center
    // 时，文本自身的行盒 leading 会让字看起来偏上/偏下 1~2pt）。
    div()
        .h_full()
        .flex()
        .items_center()
        .text_size(pt(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgba(tokens().text))
        .whitespace_nowrap()
        .overflow_hidden()
        .child(text.into())
}

/// **body 插槽的统一容器**：把弹窗里的每一项 `.child(...)` 进来，各项之间固定 8pt。
///
/// 8pt 只写在这一处，所有弹窗复用；不要在每个调用方自己写 `gap()` 或手算行距。
/// 头部栏插槽与 body 插槽之间**没有**额外间距（见 [`Popover::show`]）。
pub fn popover_body() -> gpui::Div {
    div().w_full().flex().flex_col().gap(pt(ITEM_GAP))
}

/// The opening position: the right edge on `anchor.x`, the top on `anchor.y` — lifted, when that
/// runs its bottom past the screen's bottom, to keep [`SCREEN_GAP`] under it.
pub fn open_pos(anchor: Point<Pixels>, width: f32, height: f32, screen: Size<Pixels>) -> Point<Pixels> {
    let bottom = f32::from(screen.height) - SCREEN_GAP;
    let y = if f32::from(anchor.y) + height > bottom { bottom - height } else { f32::from(anchor.y) };
    Point { x: px(f32::from(anchor.x) - width), y: px(y) }
}

/// Keeps the panel inside the screen: whole when it fits, its top-left corner when it doesn't.
pub fn clamp(pos: Point<Pixels>, width: f32, height: f32, screen: Size<Pixels>) -> Point<Pixels> {
    let x0 = EDGE;
    let y0 = EDGE;
    let hi_x = (f32::from(screen.width) - width - EDGE).max(x0);
    let hi_y = (f32::from(screen.height) - height - EDGE).max(y0);
    Point { x: px(f32::from(pos.x).clamp(x0, hi_x)), y: px(f32::from(pos.y).clamp(y0, hi_y)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: f32, h: f32) -> Size<Pixels> {
        gpui::size(px(w), px(h))
    }

    #[test]
    fn anchor_puts_the_right_edge_on_the_anchor() {
        let p = open_pos(Point { x: px(900.0), y: px(120.0) }, WIDTH, 200.0, screen(1200.0, 800.0));
        assert_eq!(f32::from(p.x), 900.0 - WIDTH);
        assert_eq!(f32::from(p.y), 120.0);
    }

    #[test]
    fn anchor_lifts_a_tall_card_off_the_screen_bottom() {
        let p = open_pos(Point { x: px(900.0), y: px(700.0) }, WIDTH, 200.0, screen(1200.0, 800.0));
        assert_eq!(f32::from(p.y), 800.0 - SCREEN_GAP - 200.0);
    }

    #[test]
    fn a_dragged_card_is_clamped_not_clipped() {
        let p = clamp(Point { x: px(-50.0), y: px(900.0) }, WIDTH, 200.0, screen(1200.0, 800.0));
        assert_eq!(f32::from(p.x), EDGE);
        assert_eq!(f32::from(p.y), 800.0 - 200.0 - EDGE);
    }

    #[test]
    fn the_title_is_a_slot_not_a_string() {
        // A custom title (a swatch-like box + a label) must be accepted by `title()` unchanged.
        let custom = div().flex().flex_row().gap(pt(4.0)).child(div().size(pt(8.0)).bg(rgba(Color32::from_rgb(1, 2, 3)))).child(tr("Custom"));
        let pop = popover_window(Open::At(Point { x: px(20.0), y: px(20.0) })).title(custom).body(div());
        assert!(pop.title.is_some(), "title() stores an arbitrary element");
        assert!(pop.body.is_some());
        assert_eq!(pop.width, WIDTH);
        assert_eq!(pop.pad, SLOT_PAD);
    }
}
