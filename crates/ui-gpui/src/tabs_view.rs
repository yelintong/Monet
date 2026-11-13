//! 顶部 40pt 标签条的 GPUI 视图层（P0-1 第 2 步）。
//!
//! 几何 / 动画状态全在 `tabs.rs`（照搬上游 831 行可测部分）；这里只负责把上游
//! `ui-egui/src/tabs.rs::tab_bar` 的**可见行为**用 GPUI Kit 拼出来：
//! Home、文件标签、宽度收缩/溢出、新标签、主题切换、溢出菜单、X / 中键关闭、拖拽重排插入线。
//!
//! 明确**不在本轮**的（对比报告里已标好阶段，避免半成品假完成）：
//! - 演示标签组 / present 状态机（P2）；
//! - 拖离成新窗口（P2：多窗口）；窗口按钮 / 无边框拖窗 / 双击最大化 / 8 向边缘缩放在
//!   P0-2 用 `WindowControlArea` 落地（**更正**：GPUI 0.2.1 有等价能力，不是框架差异）；
//! - ~~未保存关闭三问~~（**A18 已接，2026-11-10**）：关闭走 `file.close {index}`，引擎对脏文档
//!   发 `dialog.unsaved`，由 `dialogs.rs` 的通用三问对话框处理；Alt+close 才带 `force:true` 直接丢弃；
//! - live / 断连标记（P0-2）、a11y（AccessKit，P4）、Home 首页内容（P1）。
//!
//! 坐标口径：`tab_strip` 是**窗口标题栏区域的 child**（kit `TitleBar::new().child(...)`），
//! 它的 (0,0) 就是窗口原点；标签 x、拖拽落点、溢出菜单锚点都直接用窗口坐标。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Bounds, ClickEvent, Div, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Stateful, Window, WindowControlArea, div,
    prelude::*, px, size,
};
use serde_json::json;

use crate::panels::{Cmd, Cmds};
use crate::tabs::{
    BAR_BUTTON, BAR_ICON, CLOSE_ICON, HEIGHT, LIVE_MARK_WIDTH, LiveBadge, OVERFLOW_WIDTH, TabMotion, VisibleTab, live_badge, open_progress,
    preferred_width, tab_drop_position, tab_layout,
};
use crate::title_bar;
use crate::widgets::icon::{self, Icon};
use crate::widgets::{MenuRow, pt, rgba, tokens};
use crate::{CanvasView, panels};

/// 一个文档标签的数据快照：每帧从 `Session.docs` 现读，照右栏 `PanelData` 的模式。
#[derive(Clone, Debug)]
pub(crate) struct TabDoc {
    /// `Session.docs` 里的真实下标（`doc.switch` / `file.close` / `file.moveTab` 都用它）。
    pub index: usize,
    pub uid: u64,
    pub name: String,
    pub board: bool,
    pub dirty: bool,
    /// 上游 `live_mark` / `live_lost`：这个文档自己的 live 会话，每帧从 Session 现读。
    pub live: LiveBadge,
}

pub(crate) fn read_docs(session: &figcraft_engine::Session) -> Vec<TabDoc> {
    session
        .docs
        .iter()
        .enumerate()
        .map(|(index, d)| {
            // `live_of` 给的是「这个 uid 的会话」，但会话还带着它同步的 `doc_uid`；
            // 两个条件都满足才在标签上画徽标（上游 tabs.rs:688/694 的 filter）。
            let live = session.live_of(d.uid);
            TabDoc {
                index,
                uid: d.uid,
                name: d.doc.name.clone(),
                board: d.doc.kind == figcraft_doc::FileKind::Board,
                dirty: d.is_dirty(),
                live: live_badge(live.shared(), live.disconnected(), live.doc_uid, d.uid),
            }
        })
        .collect()
}

/// 拖拽状态由**跨帧共享的 Rc** 持有，而不是在 `drag_router` 的回调里 `Entity::update`：
/// 根视图的 `cx.listener` 已经借出了 `CanvasView`，回调里再 `me.update(cx, …)` 会二次借用同一
/// entity 并 panic。现有 `panels/extra.rs::dev-split` 也是用共享状态 + `wake()` 这个模式。
#[derive(Clone, Default)]
pub(crate) struct TabDragState(Rc<RefCell<DragInner>>);

#[derive(Default)]
struct DragInner {
    current: Option<TabDrag>,
    /// 本帧可见标签的窗口 x 区间（`tab_drop_position` 的输入）。
    visible: Vec<VisibleTab>,
}

struct TabDrag {
    from: usize,
    start: Point<Pixels>,
    pointer: Point<Pixels>,
    /// 超过 3pt 死区才算拖拽，否则这一下仍是「点击切换标签」。
    moved: bool,
}

impl TabDragState {
    fn begin(&self, from: usize, at: Point<Pixels>) {
        *self.0.borrow_mut() = DragInner { current: Some(TabDrag { from, start: at, pointer: at, moved: false }), visible: Vec::new() };
    }
    fn set_visible(&self, visible: Vec<VisibleTab>) {
        self.0.borrow_mut().visible = visible;
    }
    /// 画插入线要的三个值。
    fn marker(&self) -> Option<(usize, f32)> {
        let b = self.0.borrow();
        let d = b.current.as_ref()?;
        if !d.moved {
            return None;
        }
        tab_drop_position(&b.visible, d.from, f32::from(d.pointer.x)).map(|(_, x)| (d.from, x))
    }
}

/// 跨帧的标签条状态；`home` / `motion` / 拖拽都由 `CanvasView` 持有。
#[derive(Default)]
pub(crate) struct TabState {
    pub home: bool,
    pub motion: TabMotion,
    pub drag: TabDragState,
    started: Option<Instant>,
}

impl TabState {
    fn now(&mut self) -> f64 {
        self.started.get_or_insert_with(Instant::now).elapsed().as_secs_f64()
    }
}

/// 上游 `icons::file_mark(FileKind)`：Design → 钢笔尖，Board → 网格。
fn file_mark(board: bool) -> Icon {
    if board { Icon::ViewGrid } else { Icon::Pen }
}

/// GPUI 没有上游的 `bidi::paint_name`，按标签宽度做字符级省略（中文按全角计）。
fn elide(name: &str, width: f32) -> String {
    let budget = ((width - 72.0).max(0.0) / 7.2).floor() as usize;
    if budget == 0 {
        return String::new();
    }
    let units = |c: char| if c.is_ascii() { 1 } else { 2 };
    let mut used = 0;
    let mut out = String::new();
    for c in name.chars() {
        let u = units(c);
        if used + u > budget {
            out.push('…');
            return out;
        }
        out.push(c);
        used += u;
    }
    out
}

/// 标签条上的一次拖拽：注册到窗口级 `drag_router`（项目铁律：GPUI 0.2.1 无鼠标捕获，
/// 跨元素拖拽必须走覆盖整窗的根 `on_mouse_move` / `on_mouse_up`）。
fn begin_tab_drag(state: &TabDragState, cmds: &Cmds, from: usize, at: Point<Pixels>) {
    state.begin(from, at);
    let on_move = state.clone();
    let on_end = state.clone();
    let wake = cmds.clone();
    let wake_end = cmds.clone();
    crate::drag_router::begin(
        "tab",
        move |pos, _pressed, _window, _cx| {
            if let Some(d) = on_move.0.borrow_mut().current.as_mut() {
                let dx = f32::from(pos.x) - f32::from(d.start.x);
                let dy = f32::from(pos.y) - f32::from(d.start.y);
                d.pointer = pos;
                if dx.hypot(dy) > 3.0 {
                    d.moved = true;
                }
            }
            wake.wake();
        },
        move |_window, _cx| {
            let (from, pointer_x, visible, moved) = {
                let b = on_end.0.borrow();
                let Some(d) = b.current.as_ref() else { return };
                (d.from, f32::from(d.pointer.x), b.visible.clone(), d.moved)
            };
            on_end.0.borrow_mut().current = None;
            if moved
                && let Some((to, _)) = tab_drop_position(&visible, from, pointer_x)
                && to != from
            {
                wake_end.push(Cmd { id: "file.moveTab", params: Some(json!({ "from": from, "to": to })) });
            }
            wake_end.wake();
        },
    );
}

/// 溢出菜单：所有文档（含未显示的）都能从这里切换，勾选当前文档。
fn open_overflow_menu(me: &Entity<CanvasView>, cmds: &Cmds, docs: &[TabDoc], active: usize, home: bool, at_x: f32, cx: &mut App) {
    let labels: Vec<(String, usize)> = docs.iter().map(|d| (if d.dirty { format!("{} •", d.name) } else { d.name.clone() }, d.index)).collect();
    let rows: Vec<MenuRow> = labels.iter().map(|(label, i)| MenuRow::new(label.clone()).checked(Some(*i == active && !home))).collect();
    let cmds_pick = cmds.clone();
    let me_pick = me.clone();
    let on_pick: Rc<dyn Fn(&str, &mut Window, &mut App)> = Rc::new(move |label: &str, _window, cx| {
        if let Some((_, index)) = labels.iter().find(|(l, _)| l == label) {
            cmds_pick.push(Cmd { id: "doc.switch", params: Some(json!({ "index": index })) });
            me_pick.update(cx, |cv, _| cv.close_home());
        }
    });
    // 通用 Menu 自己按「锚点底边 + 4pt」定位；给 40pt 高的锚点正好落在标签条下方 4pt。
    let anchor = Bounds { origin: Point::new(px(at_x), px(0.0)), size: size(px(OVERFLOW_WIDTH), px(HEIGHT)) };
    me.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
}

/// 一条 32pt 方形按钮（新标签 / 主题）：上游 `BAR_BUTTON=32` / `BAR_ICON=20`。
/// `IconButton` 内部把字形锁在 16pt，这里按 #1729 的规范用 Kit 的 `icon::paint` 画 20pt。
fn bar_button(icon: Icon, t: crate::theme::Tokens, id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(pt(BAR_BUTTON))
        .h(pt(BAR_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .hover(move |s| s.bg(rgba(t.hover)))
        .child(icon::paint(icon, rgba(t.icon), BAR_ICON))
}

/// 画 40pt 标签条，并把本帧的可见标签矩形同步进拖拽状态。
#[allow(clippy::too_many_arguments)]
pub(crate) fn tab_strip(
    state: &mut TabState,
    docs: &[TabDoc],
    active: usize,
    dark: bool,
    reduce_motion: bool,
    // 标题栏壳留给 TabBar 内容的宽度（已扣掉左留白与右侧窗口按钮）。
    content_w: f32,
    cmds: &Cmds,
    me: &Entity<CanvasView>,
    // 必须是**具体类型**：Rust 2024 的 `impl Trait` 会捕获 `&mut state` 的生命周期，
    // 根视图在后面还要借 `self.tabs` / `self.render_left_panel(...)`，会直接 E0502/E0499。
) -> Stateful<Div> {
    let t = tokens();
    let now = state.now();
    state.motion.begin_frame(docs.iter().map(|d| d.uid), now, reduce_motion);
    let opening = state.motion.opening().to_vec();
    // 上游还会排除「正在演示」的情况；GPUI 的 present 是覆盖层、不是标签（P2）。
    let home_active = state.home || docs.is_empty();

    // 窗口标题栏壳（`title_bar.rs`）负责左留白 8pt 和右侧 3×46pt 窗口按钮；这里只画
    // TabBar 内容。x 一律用**窗口坐标**（拖拽落点、溢出菜单锚点都是窗口坐标），
    // 插入线是内容区的 absolute 子元素，画的时候要减回 content_x0。
    let content_x0 = title_bar::LEFT_PAD;
    let home_w = (panels::RAIL_W - content_x0).max(0.0);
    let mut x = content_x0 + home_w;
    // 上游 `room = rect.max.x - buttons_w - 70 - x - 34`；`content_w` 已扣掉壳的两侧，
    // 于是等价 `content_w - 70 - home_w - 34`（新标签 / 主题 / 空白区留位）。
    let room = (content_w - 70.0 - home_w - 34.0).max(0.0);
    let preferred: Vec<f32> = docs.iter().map(|d| preferred_width(&d.name, d.live.visible(), d.dirty)).collect();
    let active_tab = docs.iter().position(|d| d.index == active);
    let layout = tab_layout(&preferred, room, active_tab);

    // Home 键（active / hover 底色 + 20pt 图标）。
    let me_home = me.clone();
    let home = div()
        .id("tab-home")
        .flex_none()
        .w(pt(home_w))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .when(home_active, move |d| d.bg(rgba(t.pressed)))
        .hover(move |s| s.bg(rgba(t.hover)))
        .on_click(move |_e: &ClickEvent, _window, cx: &mut App| {
            me_home.update(cx, |cv, cx| {
                cv.show_home();
                cx.notify();
            });
        })
        .child(icon::paint(Icon::Home, rgba(t.icon), BAR_ICON))
        .into_any_element();

    // 这是 kit `TitleBar::new().child(...)` 的 child：高度 / 底色 / 窗口按钮都由壳负责。
    let mut strip = div()
        .id("tab-strip")
        .flex_1()
        .min_w(pt(0.0))
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .relative()
        .child(home)
        .child(div().flex_none().w(pt(1.0)).h_full().bg(rgba(t.border)));

    let mut visible: Vec<VisibleTab> = Vec::with_capacity(layout.widths.len());
    for (slot, &w) in layout.widths.iter().enumerate() {
        let d = &docs[layout.first + slot];
        let is_active = d.index == active && !state.home;
        let progress = open_progress(&opening, d.uid, now);
        let tab_x = x;
        let right = x + w;
        visible.push((d.index, (tab_x, right)));

        let cmds_switch = cmds.clone();
        let me_switch = me.clone();
        let cmds_close = cmds.clone();
        let cmds_middle = cmds.clone();
        let drag = state.drag.clone();
        let drag_cmds = cmds.clone();
        let index = d.index;
        let uid = d.uid;
        let body_w = (w - 32.0).max(0.0);
        // 上游 `bidi::paint_name` 的可用宽度也要扣掉 live 标记与未保存点，否则名字会压到徽标上。
        let name_w = (w - if d.live.visible() { LIVE_MARK_WIDTH } else { 0.0 } - if d.dirty { 12.0 } else { 0.0 }).max(0.0);
        let name = elide(&d.name, name_w);
        let badge = d.live;

        let body = div()
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .w(pt(body_w))
            .min_w(pt(0.0))
            .pl(pt(10.0))
            .gap(pt(6.0))
            .overflow_hidden()
            .id(("tab-body", uid as usize))
            .on_click(move |_e: &ClickEvent, _window, cx: &mut App| {
                cmds_switch.push(Cmd { id: "doc.switch", params: Some(json!({ "index": index })) });
                me_switch.update(cx, |cv, _| cv.close_home());
            })
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _window, _cx| {
                begin_tab_drag(&drag, &drag_cmds, index, e.position);
            })
            .child(icon::paint(file_mark(d.board), rgba(t.accent), BAR_ICON))
            .child(
                div()
                    .flex_1()
                    .min_w(pt(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(pt(12.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(name)),
            )
            .when(d.dirty, |d| d.child(div().flex_none().text_size(pt(12.0)).text_color(rgba(t.text)).child("•")))
            .when(badge.visible(), move |d| {
                // 上游 `paint_tab`：先判 lost（danger），否则按 shared 画 online::mark。
                // `widgets/icon.rs` 是 FROZEN 文件，本次不新增 LiveShared / LivePrivate 变体，
                // 复用现有的 LinkOn / LinkOff 表达「别人能进 / 只有自己」；要与上游图标一致，
                // 需用户明确授权解冻后再补变体。
                let (glyph, color) = if badge.lost {
                    (Icon::LiveDisconnected, t.danger)
                } else if badge.shared == Some(true) {
                    (Icon::LinkOn, t.success)
                } else {
                    (Icon::LinkOff, t.text_secondary)
                };
                d.child(icon::paint(glyph, rgba(color), BAR_ICON))
            });

        // 关闭区独占 32pt，不压在可拖拽的 body 上（上游 `close_r`）。
        let close = div()
            .flex_none()
            .w(pt(32.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |s| s.bg(rgba(t.pressed)))
            // A18：普通关闭不带 force，让引擎判断文档是否脏 —— 脏了会发 `dialog.unsaved`，
            // 由通用对话框弹 Don't Save / Cancel / Save。只有 Alt+close 才直接丢弃
            // （`e.modifiers.alt`），与 ui-egui 的 Alt+close 语义一致。
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _window, cx: &mut App| {
                cmds_close.push(Cmd { id: "file.close", params: Some(json!({ "index": index, "force": e.modifiers.alt })) });
                cx.stop_propagation();
            })
            .on_mouse_down(MouseButton::Middle, move |e: &MouseDownEvent, _window, cx: &mut App| {
                cmds_middle.push(Cmd { id: "file.close", params: Some(json!({ "index": index, "force": e.modifiers.alt })) });
                cx.stop_propagation();
            })
            .child(icon::paint(Icon::Close, rgba(t.text_secondary), CLOSE_ICON));

        // 入场动画：0.18s 渐显（上游的 -28pt 滑入需要绝对定位，会脱离 flex 布局，本轮保留渐显）。
        let tab = div()
            .flex_none()
            .w(pt(w))
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .when(is_active, move |d| d.bg(rgba(t.pressed)))
            .when(!is_active, move |d| d.hover(move |s| s.bg(rgba(t.hover))))
            .opacity(progress)
            .child(body)
            .child(close);
        strip = strip.child(tab).child(div().flex_none().w(pt(1.0)).h(pt(HEIGHT - 20.0)).bg(rgba(t.border)));
        x = right;
    }
    state.drag.set_visible(visible.clone());

    // 溢出 chevron + 菜单（菜单的防溢出 / 400pt 内部滚动由通用 `menu_popup_in` 负责）。
    if layout.overflow {
        let chevron_x = x;
        let me_menu = me.clone();
        let cmds_menu = cmds.clone();
        let docs_menu = docs.to_vec();
        let home_menu = state.home;
        strip = strip.child(
            div()
                .id("tab-overflow")
                .flex_none()
                .w(pt(OVERFLOW_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_click(move |_e: &ClickEvent, _window, cx: &mut App| {
                    open_overflow_menu(&me_menu, &cmds_menu, &docs_menu, active, home_menu, chevron_x, cx);
                })
                .child(icon::paint(Icon::ChevronDown, rgba(t.icon), BAR_ICON)),
        );
        x += OVERFLOW_WIDTH;
    }

    // 新标签 +。
    let cmds_new = cmds.clone();
    let me_new = me.clone();
    strip = strip.child(div().flex_none().h_full().flex().items_center().pl(pt(6.0)).child(bar_button(Icon::Plus, t, "tab-new").on_click(
        move |_e: &ClickEvent, _window, cx: &mut App| {
            cmds_new.push(Cmd { id: "file.new", params: Some(json!({})) });
            me_new.update(cx, |cv, _| cv.close_home());
        },
    )));

    // 空白区 = 系统标题栏的拖拽区（上游 `drag_free_space()` 的 `StartDrag`）。
    // 壳整条已标 `WindowControlArea::Drag`；这里再标一次，保证这块 flex_1 hitbox
    // 命中时平台仍按 HTCAPTION 处理，标签 / Home / 按钮的子 hitbox 不受影响。
    strip = strip.child(div().flex_1().min_w(pt(0.0)).h_full().window_control_area(WindowControlArea::Drag));

    // 主题切换（engine `view.themeLight/Dark` 改 prefs，render 每帧据此重建 Tokens）。
    let cmds_theme = cmds.clone();
    strip = strip.child(div().flex_none().h_full().flex().items_center().pr(pt(10.0)).child(
        bar_button(if dark { Icon::Sun } else { Icon::Moon }, t, "tab-theme").on_click(move |_e: &ClickEvent, _window, _cx| {
            cmds_theme.push(Cmd { id: if dark { "view.themeLight" } else { "view.themeDark" }, params: Some(json!({})) });
        }),
    ));

    // 拖拽插入线（3pt 竖线，画在标签条内、所有标签之上）。
    if let Some((_, at)) = state.drag.marker() {
        strip = strip.child(div().absolute().left(px(at - content_x0)).top(pt(5.0)).w(pt(3.0)).h(pt(HEIGHT - 10.0)).bg(rgba(t.accent)));
    }
    strip
}
