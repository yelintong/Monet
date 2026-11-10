//! 控件树（Layers 树）：左面板的图层列表，可展开/折叠、选中、显隐/锁定、父子级拖动排序。
//!
//! 这一份只是 [`crate::drag`] 通用拖拽件的一个**消费方**：把可见行翻成 `drag::Row`
//! （`/` 分隔的祖先 id 链当层级、容器能否容纳子节点、是否折叠、窗口矩形），每帧把全局指针
//! 交给 `Drag::update`，松手把 `Target` 翻译成一次 `node.reparent`。所有落点几何、三区判定、
//! 左右改层级、防环、插入线 / 拖拽标签 / 悬停展开都在通用件里，这里不再有第二份实现。
//!
//! GPUI 0.2.1 没有「元素 id → 布局矩形」查询，`on_drag_move` 只给命中那一行的 `bounds`；
//! 通用件的 `calibrate` 用它加固定行高重建整列，于是根层的全局 `on_mouse_move` 在指针移到
//! 行间 / 列表外时也能继续解析落点（旧实现只在命中行里更新，指针一离开落点就冻住）。

use figcraft_i18n::tr;
use std::collections::BTreeSet;
use std::time::Instant;

use figcraft_doc::{BoolOp, Document, NodeId, NodeKind};
use figcraft_engine::booleans::distinct_op;
use figcraft_engine::components::is_slot;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, Div, ElementId, MouseButton, MouseDownEvent, Pixels, Point, Render,
    SharedString, Size, Stateful, StatefulInteractiveElement, Window, div, px,
};
use serde_json::json;

use crate::drag;
use crate::widgets::icon::paint as paint_icon;
use crate::widgets::{Icon, IconButton, SectionHeader, SectionHeaderVariant, pt, rgba, tokens};
use crate::CanvasView;

/// 每行的视觉缩进（与控件树自己的 `depth * INDENT` 对齐）。
const LAYER_INDENT: f32 = 16.0;
/// 行内左右各缩 8pt（egui layers 的 `r.shrink2(vec2(8,0))`）。
const ROW_INSET: f32 = 8.0;
const ROW_H: f32 = 32.0;

/// 拖拽载荷：被拖的那一行的节点 id（gpui 内建拖拽要求 `T: 'static`）。
struct LayerDrag(pub NodeId);

/// 拖拽时跟随光标的占位视图 —— GPUI 内建 drag 需要一个 ghost 视图；真正的标签由通用件
/// `drag::overlay` 画在根层（位置/样式与 egui `drag_ghost` 一致），这里留空避免重复。
struct LayerDragGhost;
impl Render for LayerDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// 一行控件树（已展开成扁平列表后的单个可见行）。
#[derive(Clone)]
pub struct LayerRow {
    pub id: NodeId,
    /// 视觉深度，0 = 顶层。
    pub depth: usize,
    pub name: String,
    pub icon: Icon,
    pub has_children: bool,
    pub can_into: bool,
    pub expanded: bool,
    pub selected: bool,
    /// 0 普通 / 1 组件族 / 2 图标（egui `layers.rs` 的 is_comp / is_icon）。
    pub mark_kind: u8,
    pub visible: bool,
    pub locked: bool,
}

/// egui `layers.rs::Mark`：选中行 = Selected；选中行的后代 = Child；其余 = None。
#[derive(Clone, Copy, PartialEq)]
enum Mark { None, Child, Selected }

/// 选中行及其后代算一个连续 run，整块只画一次（egui `marks` / `selection_runs`）。
fn marks(rows: &[LayerRow]) -> Vec<Mark> {
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        while open.last().is_some_and(|d| *d >= r.depth) { open.pop(); }
        if r.selected { open.push(r.depth); out.push(Mark::Selected); }
        else if open.is_empty() { out.push(Mark::None); }
        else { out.push(Mark::Child); }
    }
    out
}

fn selection_runs(m: &[Mark]) -> (Vec<usize>, Vec<usize>) {
    let n = m.len();
    let mut first = vec![0usize; n];
    let mut last = vec![0usize; n];
    let mut i = 0;
    while i < n {
        if m[i] != Mark::None {
            let start = i;
            while i < n && m[i] != Mark::None { i += 1; }
            let end = i - 1;
            for j in start..=end { first[j] = start; last[j] = end; }
        } else { i += 1; }
    }
    (first, last)
}

/// band 里全图标 → `icon_layer_row`，全组件 → `component_row`，否则 `selected_row`。
/// 范围就是这条 band 自己（`start..=end`），不是整个 run —— egui `layers.rs` 传的是 band 的
/// `start, end`，所以「选中父 + 普通子」的 run 里，父那条 band 仍是普通选中色。
fn run_color(rows: &[LayerRow], from: usize, to: usize) -> gpui::Rgba {
    let t = tokens();
    let all_icon = rows[from..=to].iter().all(|r| r.mark_kind == 2);
    let all_comp = rows[from..=to].iter().all(|r| r.mark_kind == 1);
    if all_icon { rgba(t.icon_layer_row) }
    else if all_comp { rgba(t.component_row) }
    else { rgba(t.selected_row) }
}

/// 控件树的拖动与展开状态，挂在 `CanvasView` 上跨帧保留。
pub struct LayerTreeState {
    /// 已展开的容器节点（有子节点且被展开）。
    pub expanded: BTreeSet<NodeId>,
    /// 当前帧渲染出的可见行（渲染用）。
    pub rows: Vec<LayerRow>,
    /// 同一批行翻成通用件的输入（含窗口矩形与路径）。
    pub drag_rows: Vec<drag::Row>,
    /// 正在进行的拖拽（`on_drag` 起手时置位，松手 / Escape 清空）。
    pub drag: Option<drag::Drag>,
    /// 按下但未越过阈值的候选（可能是点击选中，也可能起手拖拽）。
    pub candidate: Option<(NodeId, Point<Pixels>)>,
    /// 最近一次 `on_drag_move` 报告的（行索引, 真实窗口矩形），用于每帧重建整列几何。
    pub calibrated: Option<(usize, Bounds<Pixels>)>,
    /// 最后一次全局指针位置（窗口坐标）——画拖拽标签、解析落点都用它。
    pub pointer: Point<Pixels>,
    clock: Instant,
}

impl Default for LayerTreeState {
    fn default() -> Self {
        Self {
            expanded: BTreeSet::new(),
            rows: Vec::new(),
            drag_rows: Vec::new(),
            drag: None,
            candidate: None,
            calibrated: None,
            pointer: Point::new(px(0.0), px(0.0)),
            clock: Instant::now(),
        }
    }
}

impl CanvasView {
    /// 左面板：标题 + 控件树滚动区。
    ///
    /// 宽度由调用方给 `left_w + HANDLE_W`：多出的 1px 是画布侧那条分隔线，容器**自身右缘**
    /// 用 1px `border_r_1` 画它（与底栏 `border_t_1` 同一套做法，不再有独立的 `drag_handle`
    /// 元素）。上/下/左三边一律不画，避免与标题栏下沿、时间轴顶边叠成双线。border-box 内描边
    /// 不改布局宽度：`canvas_x` / 命中几何仍按 `left_w + HANDLE_W` 算，一像素不差。
    /// hover 或拖拽中这条线变强调色（`resize_drag::hovered` 由 4px 覆盖条写入）。
    pub(crate) fn render_left_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = tokens();
        let divider_hot = crate::resize_drag::hovered(crate::resize_drag::Edge::Left)
            || crate::resize_drag::drag_active(crate::resize_drag::Handle::LeftPanel);
        // 文件头那个改名框提交的新名字（`file.rename`）。
        // 先把待提交的名字取出来（`RefMut` 的临时值活到语句结束，不能包着 `&mut self` 调用）。
        let renamed = self.file_rename.borrow_mut().take();
        if let Some(name) = renamed {
            self.run_command("file.rename", serde_json::json!({ "name": name }), cx);
        }
        self.sync_file_name(cx);
        // egui `layers::left_panel`：**三段** —— 64 pt 文件头、Pages 段、Layers 段。
        // 以前只有「Layers 标题 + 树」，文件头和页面列表整个没有（用户 2026-11-10）。
        div()
            .id("left-panel")
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgba(t.panel))
            .border_r_1()
            .border_color(rgba(if divider_hot { t.accent } else { t.border }))
            .child(self.file_header(cx))
            .child(self.pages_section(cx))
            .child(SectionHeader::new(tr("Layers")).variant(SectionHeaderVariant::Normal))
            .child(self.render_layer_tree(cx))
    }

    /// 把文档名回写进文件头的输入框：只在**换了标签**（uid 变化）或名字真的变了时写，
    /// 否则每帧 `set_value` 会把正在输入的内容打断。
    fn sync_file_name(&mut self, cx: &mut Context<Self>) {
        let Some(ds) = self.app.session.ds().ok() else { return };
        let (uid, name) = (ds.uid, ds.doc.name.clone());
        if self.file_name_seen.as_ref() == Some(&(uid, name.clone())) {
            return;
        }
        let field = self.file_name_field.clone();
        field.update(cx, |tf, _| tf.set_value(name.clone()));
        self.file_name_seen = Some((uid, name));
    }

    /// 左栏的文件头（egui `layers::left_panel` 开头那段 64 pt）：文件名（可改）+ `Drafts` + 收起按钮。
    fn file_header(&mut self, cx: &mut Context<Self>) -> Div {
        let t = tokens();
        let field = self.file_name_field.clone();
        // 提交（Enter / 失焦）→ 排队一个 `file.rename`，由下一帧的左栏取走。
        let pending = self.file_rename.clone();
        field.update(cx, |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    *pending.borrow_mut() = Some(name);
                }
            });
        });
        let me = cx.entity();
        div()
            .flex_none()
            .w_full()
            .h(pt(64.0))
            .flex()
            .flex_col()
            .px(pt(14.0))
            .pt(pt(12.0))
            .border_b_1()
            .border_color(rgba(t.border))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .child(div().flex_1().min_w(px(0.0)).child(field))
                    .child(IconButton::new(Icon::Panels).size(24.0).on_click(move |_e, _w, cx| {
                        me.update(cx, |cv, cx| {
                            cv.left_collapsed = true;
                            cx.notify();
                        });
                    })),
            )
            .child(div().mt(pt(2.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Drafts")))
    }

    /// Pages 段（egui `layers::pages`）：40 pt 标题行（`Pages` + 搜索 + 加页）+ 页行。
    /// 页行 32 pt：选中页有底色；分隔页显示一条横线（egui 的 `divider` 页）。
    fn pages_section(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = tokens();
        let Some(ds) = self.app.session.ds().ok() else { return div().id("pages-section") };
        let pages: Vec<(NodeId, String, bool)> =
            ds.doc.pages.iter().filter_map(|p| ds.doc.node(*p).map(|n| (*p, n.name.clone(), ds.doc.is_divider(*p)))).collect();
        let cur = ds.page;
        let open = *self.pages_open.borrow();
        let searching = *self.page_search.borrow();
        let filter = self.page_filter.borrow().trim().to_lowercase();
        let me = cx.entity();

        // ── 40 pt 标题行 ────────────────────────────────────────────────────────────────
        let open_slot = self.pages_open.clone();
        let search_slot = self.page_search.clone();
        let filter_slot = self.page_filter.clone();
        let me_add = me.clone();
        let head = div()
            .flex_none()
            .w_full()
            .h(pt(40.0))
            .flex()
            .flex_row()
            .items_center()
            .pl(pt(16.0))
            .pr(pt(8.0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, {
                let me = me.clone();
                move |_e, _, cx| {
                    let next = !*open_slot.borrow();
                    *open_slot.borrow_mut() = next;
                    me.update(cx, |_, cx| cx.notify());
                }
            })
            .child(div().flex_1().text_size(pt(11.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("Pages")))
            .child(IconButton::new(Icon::Search).size(24.0).active(searching).on_click({
                let me = me.clone();
                move |_e, _, cx| {
                    let next = !*search_slot.borrow();
                    *search_slot.borrow_mut() = next;
                    if next {
                        filter_slot.borrow_mut().clear();
                    }
                    me.update(cx, |_, cx| cx.notify());
                }
            }))
            .child(IconButton::new(Icon::Plus).size(24.0).on_click(move |_e, _, cx| {
                me_add.update(cx, |cv, cx| cv.run_command("page.add", serde_json::json!({}), cx));
            }));

        let mut sec = div().id("pages-section").flex_none().w_full().flex().flex_col().child(head);
        if !open {
            return sec;
        }
        if searching {
            // 边打边过滤：`set_on_change`（不是 Enter）。
            let field = self.page_search_field.clone();
            let slot = self.page_filter.clone();
            let me_field = me.clone();
            field.update(cx, |tf, _| {
                tf.set_on_change(move |text, _, cx| {
                    *slot.borrow_mut() = text.trim().to_string();
                    me_field.update(cx, |_, cx| cx.notify());
                });
            });
            sec = sec.child(div().px(pt(8.0)).pb(pt(4.0)).child(field));
        }
        let mut list = div().id("pages-list").w_full().flex().flex_col().max_h(px(220.0)).overflow_y_scroll();
        for (id, name, divider) in pages {
            if !filter.is_empty() && (divider || !name.to_lowercase().contains(&filter)) {
                continue;
            }
            let is_cur = id == cur;
            let me_row = me.clone();
            let row = div()
                .id(ElementId::from(SharedString::from(format!("page-{}", id.0))))
                .flex_none()
                .w_full()
                .h(pt(32.0))
                .px(pt(8.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_e, _, cx| {
                    me_row.update(cx, |cv, cx| cv.run_command("page.select", serde_json::json!({ "id": id.0 }), cx));
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .rounded(pt(5.0))
                        .px(pt(8.0))
                        .py(pt(4.0))
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text))
                        .when(is_cur, |d| d.bg(rgba(t.selected_row)))
                        .when(!is_cur, |d| d.hover(|s| s.bg(rgba(t.hover))))
                        .child(if divider { SharedString::from("—") } else { SharedString::from(name) }),
                );
            list = list.child(row);
        }
        sec.child(list)
    }

    fn render_layer_tree(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let (rows, drag_rows) = self.collect_layer_rows();
        self.layer_tree.rows = rows.clone();
        self.layer_tree.drag_rows = drag_rows;
        let drag = self.layer_tree.drag.as_ref().map(|d| d.from_id);
        let marked = marks(&rows);
        let (first, last) = selection_runs(&marked);
        let t = tokens();
        let mut overlays: Vec<AnyElement> = Vec::new();
        // ① 整个 run 一块 `selected_row_child`，四角圆 5，上下各缩 4。
        for i in 0..rows.len() {
            if marked[i] == Mark::None || i != first[i] { continue; }
            let from = first[i]; let to = last[i];
            let h = (to - from + 1) as f32 * ROW_H;
            overlays.push(
                div().absolute().left(pt(0.0)).top(pt(from as f32 * ROW_H))
                    .w_full().h(pt(h))
                    .px(pt(ROW_INSET)).pt(pt(4.0)).pb(pt(4.0))
                    .child(div().size_full().rounded(pt(5.0)).bg(rgba(t.selected_row_child)))
                    .into_any_element(),
            );
        }
        // ② run 内连续的 Selected 行 = band。egui `layers.rs`：只有落在 run 上下两端时才有
        //    对应两角的圆角（`CornerRadius { nw/ne: edge(start == from), sw/se: edge(end == to) }`），
        //    band 之间的接缝保持直角，否则会变成一堆叠在一起的小圆角块。
        let mut k = 0;
        while k < rows.len() {
            if marked[k] != Mark::Selected { k += 1; continue; }
            let start = k;
            while k < rows.len() && marked[k] == Mark::Selected { k += 1; }
            let end = k - 1;
            let from = first[start]; let to = last[start];
            let tp = if start == from { 4.0 } else { 0.0 };
            let bp = if end == to { 4.0 } else { 0.0 };
            let h = (end - start + 1) as f32 * ROW_H - tp - bp;
            let mut band = div().size_full().bg(run_color(&rows, start, end));
            if start == from { band = band.rounded_t(px(5.0)); }
            if end == to { band = band.rounded_b(px(5.0)); }
            overlays.push(
                div().absolute().left(pt(0.0)).top(pt(start as f32 * ROW_H + tp))
                    .w_full().h(pt(h)).px(pt(ROW_INSET))
                    .child(band)
                    .into_any_element(),
            );
        }
        let mut divs: Vec<Stateful<Div>> = Vec::with_capacity(rows.len());
        for (index, r) in rows.iter().enumerate() {
            divs.push(self.layer_row(index, r, drag, marked[index], cx));
        }
        div()
            .id("layer-scroll")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(overlays)
            .children(divs)
    }

    fn layer_row(
        &mut self,
        index: usize,
        r: &LayerRow,
        dragging: Option<u64>,
        mark: Mark,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = tokens();
        let is_dragging = dragging == Some(r.id.0);
        let row_h = pt(ROW_H);
        let indent = r.depth as f32 * LAYER_INDENT;

        // egui `layers.rs`：行本身是透明的；选中 / 悬停是「行左右各缩 8、上下各缩 4」的
        // 圆角 5 独立块（`hl = r.shrink2(vec2(8,0)).shrink2(vec2(0,4))`），
        // 不是整行铺满 32 高的粗方块。选中 → `t.selected_row`，悬停 → `t.hover` 实心、无描边。
        // hover 由整行 hitbox 决定（`resp.contains_pointer()`），所以挂在 group 上。
        let group: SharedString = format!("layer-row-{}", r.id.0).into();

        let name_color = if r.visible { rgba(t.text) } else { rgba(t.text_tertiary) };
        let id = r.id;
        let name = r.name.clone();
        let icon = r.icon;
        let has_children = r.has_children;
        let expanded = r.expanded;
        let visible = r.visible;
        let locked = r.locked;

        // 行实体的稳定 id：gpui 的内建拖拽（`on_drag` / `on_drag_move`）只能挂在 Stateful 上。
        let row_id = ElementId::from(SharedString::from(format!("layer-row-{}", id.0)));
        let me = cx.entity();

        let chevron = if has_children {
            div()
                .w(px(16.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this: &mut CanvasView, _e: &MouseDownEvent, _w: &mut Window, cx: &mut Context<Self>| {
                        this.layer_toggle_expand(id, cx);
                    }),
                )
                .child(paint_icon(if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, rgba(t.text_secondary), 16.0))
        } else {
            div().w(px(16.0))
        };

        let eye = div()
            .w(px(18.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut CanvasView, _e: &MouseDownEvent, _w: &mut Window, cx: &mut Context<Self>| {
                    this.layer_toggle_visible(id, cx);
                }),
            )
            .child(paint_icon(if visible { Icon::Eye } else { Icon::EyeOff }, rgba(if visible { t.text_secondary } else { t.text_tertiary }), 16.0));

        let lock = div()
            .w(px(18.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut CanvasView, _e: &MouseDownEvent, _w: &mut Window, cx: &mut Context<Self>| {
                    this.layer_toggle_lock(id, cx);
                }),
            )
            .child(paint_icon(if locked { Icon::Lock } else { Icon::Unlock }, rgba(if locked { t.accent } else { t.text_tertiary }), 16.0));

        // 起手拖拽：立刻置位（egui `drag_started`），并记住整组被拖的行与跟随光标的标签。
        let me_start = me.clone();
        // 移动：把命中行的真实窗口矩形交给通用件校准，再用全局指针重算落点。
        let me_move = me.clone();

        // 选中/父子集整块由 overlay 统一画；只有未标记的行才有 hover 块（egui 同式）。
        let highlight = div().size_full().rounded(pt(5.0));
        let highlight = if mark == Mark::None {
            highlight.group_hover(group.clone(), |s| s.bg(rgba(t.hover)))
        } else {
            highlight
        };

        div()
            .group(group)
            .id(row_id)
            .relative()
            .h(row_h)
            .w_full()
            .flex()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            // 内缩层：占位与行完全重合，用 padding 把高亮块挤成 `r.shrink2(8,0).shrink2(0,4)`。
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .px(pt(ROW_INSET))
                    .pt(pt(4.0))
                    .pb(pt(4.0))
                    .child(highlight),
            )
            .when(is_dragging, |d| d.opacity(0.5))
            .on_drag(LayerDrag(id), move |_val, _offset, _window, app| {
                me_start.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                    this.begin_layer_drag(id, index, cx);
                });
                app.new(|_| LayerDragGhost)
            })
            .on_drag_move::<LayerDrag>(move |ev, _window, app| {
                let dragging = ev.drag(app).0;
                let b = ev.bounds;
                let pointer = ev.event.position;
                me_move.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                    this.on_layer_drag_move(dragging, index, b, pointer, cx);
                });
            })
            .child(div().w(px(indent)))
            .child(chevron)
            .child(div().w(px(16.0)).h(px(16.0)).child(paint_icon(icon, rgba(t.text), 16.0)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this: &mut CanvasView, e: &MouseDownEvent, _w: &mut Window, _cx: &mut Context<Self>| {
                            this.layer_on_row_mouse_down(id, e);
                        }),
                    )
                    .child(div().child(name).text_size(px(11.0)).text_color(name_color).truncate()),
            )
            .child(eye)
            .child(lock)
    }

    /// 把当前页的节点展开成可见行（DFS，**z 序最上的在最前**，与 egui `listed_rows` 一致：
    /// children 是 bottom-first，所以逆序遍历才是 top-first）。
    ///
    /// `stack.pop()` 取的是**最后**压入的元素，因此入栈必须按 children **正序**（bottom
    /// first），弹出来才是 top-first。之前这里入栈用了 `.rev()`，两次反转抵消，把列表变成了
    /// bottom-first；而提交按「列表上方 = children 里更靠后」算，两者正好相反 —— 这就是
    /// 「插入线在这里、节点却跑到另一头」的根因（用户 2026-11-06）。
    /// 同时产出通用件的 `drag::Row`：路径用祖先 id 链，重名也不会串。
    fn collect_layer_rows(&mut self) -> (Vec<LayerRow>, Vec<drag::Row>) {
        let Ok(ds) = self.app.session.ds() else { return (Vec::new(), Vec::new()) };
        let doc = &ds.doc;
        let page = ds.page;
        let Some(page_n) = doc.node(page) else { return (Vec::new(), Vec::new()) };
        let mut visual = Vec::new();
        let mut drag_rows: Vec<drag::Row> = Vec::new();
        let mut stack: Vec<(NodeId, usize, String)> = page_n.children.iter().map(|&c| (c, 0, String::new())).collect();
        while let Some((id, depth, parent_path)) = stack.pop() {
            let Some(n) = doc.node(id) else { continue };
            let has_children = !n.children.is_empty();
            let expanded = self.layer_tree.expanded.contains(&id);
            let path = if parent_path.is_empty() { id.0.to_string() } else { format!("{parent_path}/{}", id.0) };
            let can_into = n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. });
            let mark_kind = if matches!(n.kind, NodeKind::Icon { .. }) {
                2
            } else if matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. }) {
                1
            } else { 0 };
            visual.push(LayerRow {
                id,
                depth,
                name: n.name.clone(),
                icon: Self::node_icon(doc, id),
                has_children,
                can_into,
                expanded,
                selected: ds.selection.contains(&id),
                mark_kind,
                visible: n.visible,
                locked: n.locked,
            });
            drag_rows.push(drag::Row {
                id: id.0,
                path: path.clone(),
                can_into,
                collapsed: has_children && !expanded,
                rect: Bounds {
                    origin: Point::new(px(0.0), px(0.0)),
                    size: Size::new(px(240.0), px(ROW_H)),
                },
            });
            if has_children && expanded {
                // 正序入栈：pop 取最后一个 → children[last]（z 最高）先画。
                for &c in n.children.iter() {
                    stack.push((c, depth + 1, path.clone()));
                }
            }
        }
        // 每帧重建行；用最近一次命中行的真实矩形重新校准整列几何。
        if let Some((hit, b)) = self.layer_tree.calibrated {
            drag::calibrate(&mut drag_rows, hit, b);
        }
        (visual, drag_rows)
    }

    /// 通用件的层级配置：level 1 的左边界是行区内缩 8pt，每层 16pt（与视觉缩进一致）。
    fn layer_config(rows: &[drag::Row]) -> drag::Config {
        let base = rows.first().map(|r| f32::from(r.rect.origin.x) + ROW_INSET).unwrap_or(ROW_INSET);
        drag::Config { base, indent: LAYER_INDENT }
    }

    /// 节点类型 → 图标（照搬 egui `node_icon` 的映射，用 gpui 现有图标凑齐）。
    fn node_icon(doc: &Document, id: NodeId) -> Icon {
        let Some(n) = doc.node(id) else { return Icon::Rectangle };
        if let Some(op) = distinct_op(doc, id) {
            return match op {
                BoolOp::Union => Icon::Union,
                BoolOp::Subtract => Icon::Subtract,
                BoolOp::Intersect => Icon::Intersect,
                BoolOp::Exclude => Icon::Exclude,
            };
        }
        match &n.kind {
            NodeKind::Page { .. } => Icon::Page,
            NodeKind::Frame if is_slot(doc, id) => Icon::Slot,
            NodeKind::Frame => Icon::Frame,
            NodeKind::Group => Icon::Group,
            NodeKind::Section { .. } => Icon::Section,
            NodeKind::Rectangle => Icon::Rectangle,
            NodeKind::Ellipse { .. } => Icon::Ellipse,
            NodeKind::Polygon { .. } => Icon::Polygon,
            NodeKind::Star { .. } => Icon::Star,
            NodeKind::Line => Icon::Line,
            NodeKind::Vector { .. } => Icon::Vector,
            NodeKind::Text(_) => Icon::Text,
            NodeKind::BooleanOp { .. } => Icon::Frame,
            NodeKind::Component { .. } => Icon::Component,
            NodeKind::ComponentSet { .. } => Icon::ComponentSet,
            NodeKind::Instance { .. } => Icon::Instance,
            NodeKind::Icon { .. } => Icon::IconKind,
            NodeKind::Slice => Icon::Slice,
        }
    }

    // ── 拖拽事件 ────────────────────────────────────────────────────────────────

    /// `on_drag` 起手：整组一起拖（egui `dragged_layers`），标签写名字或「N layers」。
    fn begin_layer_drag(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        let Ok(ds) = self.app.session.ds() else { return };
        let moving: Vec<NodeId> = if ds.selection.contains(&id) && ds.selection.len() > 1 {
            ds.selection.iter().copied().collect()
        } else {
            vec![id]
        };
        let label = if moving.len() == 1 {
            ds.doc.node(id).map(|n| n.name.clone()).unwrap_or_default()
        } else {
            format!("{} layers", moving.len())
        };
        self.layer_tree.drag = Some(drag::Drag::start(index, id.0, label));
        // 起手后不再是「点击选中」。
        self.layer_tree.candidate = None;
        cx.notify();
    }

    /// `on_drag_move`：GPUI 只给命中行的矩形；交给通用件校准整列，再用全局指针解析。
    fn on_layer_drag_move(
        &mut self,
        dragging: NodeId,
        index: usize,
        bounds: Bounds<Pixels>,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.layer_tree.drag.as_ref().map(|d| d.from_id) != Some(dragging.0) {
            return;
        }
        self.layer_tree.calibrated = Some((index, bounds));
        self.layer_tree.pointer = pointer;
        drag::calibrate(&mut self.layer_tree.drag_rows, index, bounds);
        self.update_layer_drop(cx);
    }

    /// 根层全局 `on_mouse_move`：指针移到行间 / 列表外也能继续解析落点（旧实现冻在这里）。
    pub(crate) fn layer_on_global_move(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self.layer_tree.drag.is_none() {
            return;
        }
        self.layer_tree.pointer = pointer;
        self.update_layer_drop(cx);
    }

    /// 每帧只解析落点来画线，**不应用**；悬停折叠容器够久会请求展开。
    fn update_layer_drop(&mut self, cx: &mut Context<Self>) {
        let mut d = match self.layer_tree.drag.take() {
            Some(d) => d,
            None => return,
        };
        let rows = std::mem::take(&mut self.layer_tree.drag_rows);
        let cfg = Self::layer_config(&rows);
        let now = self.layer_tree.clock.elapsed().as_secs_f64();
        let expand = d.update(&rows, self.layer_tree.pointer, cfg, now);
        self.layer_tree.drag_rows = rows;
        self.layer_tree.drag = Some(d);
        if let Some(eid) = expand {
            self.layer_tree.expanded.insert(NodeId(eid));
            let (visual, drag_rows) = self.collect_layer_rows();
            self.layer_tree.rows = visual;
            self.layer_tree.drag_rows = drag_rows;
        }
        cx.notify();
    }

    /// Escape 取消拖拽（egui `ListDrag::with_order` 的 Escape 语义）；不触发选中、不送引擎。
    pub(crate) fn layer_cancel_drag(&mut self, cx: &mut Context<Self>) -> bool {
        if self.layer_tree.drag.is_none() {
            return false;
        }
        self.layer_tree.drag = None;
        self.layer_tree.calibrated = None;
        self.layer_tree.candidate = None;
        cx.notify();
        true
    }

    /// 根层拖拽视觉：插入线 / Into 整行描边 / 跟随光标的标签，窗口坐标绝对定位。
    pub(crate) fn layer_overlay(&self) -> Vec<AnyElement> {
        match &self.layer_tree.drag {
            Some(d) => drag::overlay(d, self.layer_tree.pointer),
            None => Vec::new(),
        }
    }

    // ── 事件 ────────────────────────────────────────────────────────────────────

    fn layer_on_row_mouse_down(&mut self, id: NodeId, e: &MouseDownEvent) {
        self.layer_tree.candidate = Some((id, e.position));
    }

    fn layer_toggle_expand(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if self.layer_tree.expanded.contains(&id) {
            self.layer_tree.expanded.remove(&id);
        } else {
            self.layer_tree.expanded.insert(id);
        }
        cx.notify();
    }

    fn layer_toggle_visible(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("object.toggleVisible", &json!({ "ids": [id] }));
        cx.notify();
    }

    fn layer_toggle_lock(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("object.toggleLock", &json!({ "ids": [id] }));
        cx.notify();
    }

    fn layer_select(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("select.set", &json!({ "ids": [id] }));
        cx.notify();
    }

    /// 根 `on_mouse_up` 里调用：有拖拽则**只在这一刻**提交落点，否则把按下候选当作点击选中。
    pub(crate) fn layer_drag_up(&mut self, cx: &mut Context<Self>) {
        if let Some(mut d) = self.layer_tree.drag.take() {
            if let Some(target) = d.take()
                && !target.refused
                && !target.noop
            {
                self.layer_commit_drop(&d, &target, cx);
            }
            self.layer_tree.calibrated = None;
            cx.notify();
        } else if let Some((cid, _)) = self.layer_tree.candidate.take() {
            self.layer_select(cid, cx);
            cx.notify();
        }
    }

    /// 把通用件的落点翻译成**一次** `node.reparent`（一个 undo step）。
    ///
    /// - `Into`：parent = anchor，落到第一个子。
    /// - 其余：层级由指针 x 决定。`parent` = anchor 路径的第 `level-1` 段祖先，
    ///   `boundary` = 第 `level` 段（浅层级就是包含 anchor 的那棵子树的根）。
    ///   `after` 落在整棵子树之后（浅层级强制），否则落在它前面。
    /// - 列表自上而下是 z 序逆序（`collect_layer_rows` 已修成 top-first）：
    ///   列表更下方 = children 里更靠前，所以 `after` → `gap = idx`，
    ///   `!after` → `gap = idx + 1`（egui `layers.rs` 的注释）。
    /// - `after` / `level` 与通用件画插入线用的 `gap` 是同一个落点：线画在整棵子树的
    ///   首行之上或末行之下，提交就把节点放到那道缝里（用户 2026-11-06 的位置不匹配修复）。
    fn layer_commit_drop(&mut self, d: &drag::Drag, target: &drag::Target, cx: &mut Context<Self>) {
        let dragging = NodeId(d.from_id);
        let Ok(ds) = self.app.session.ds() else { return };
        let doc = &ds.doc;
        let page = ds.page;
        let moving: Vec<NodeId> = if ds.selection.contains(&dragging) && ds.selection.len() > 1 {
            ds.selection.iter().copied().collect()
        } else {
            vec![dragging]
        };
        // 用 id 反查而不是 `drag_rows[target.anchor]`：松手那一帧若正好发生了悬停展开，
        // 可见行会重排，索引就不再是 resolve 时的那一行。
        let Some(anchor_row) = self.layer_tree.drag_rows.iter().find(|r| r.id == target.anchor_id) else { return };
        let anchor_path = anchor_row.path.clone();

        if target.zone == drag::Zone::Into {
            // egui layers：Into 只给 `ids` + `parent`，**不带 index** —— 引擎把不带 index 的
            // reparent 放到最上面（on top）。传 0 反而会落到 siblings 的最底（bottom first）。
            let _ = self
                .app
                .session
                .execute("node.reparent", &json!({ "ids": moving, "parent": NodeId(target.anchor_id) }));
            cx.notify();
            return;
        }

        let level = target.level;
        let (parent, boundary) = if level <= 1 {
            (Some(page), path_id(anchor_path.split('/').next()))
        } else {
            let pp = drag::parent_path_at(&anchor_path, level);
            (path_id(pp.rsplit('/').next()), path_id(anchor_path.split('/').nth(level - 1)))
        };
        let (Some(parent), Some(boundary)) = (parent, boundary) else { return };
        let Some(parent_n) = doc.node(parent) else { return };
        let idx = doc.index_in_parent(boundary).unwrap_or(0);
        let gap = if target.after { idx } else { idx + 1 };
        // 被拖的行若本来就在 gap 之前，会随移出而腾出位置（egui layers 的 `below`）。
        let below = parent_n.children.iter().take(gap).filter(|c| moving.contains(c)).count();
        let index = gap.saturating_sub(below);
        let _ = self
            .app
            .session
            .execute("node.reparent", &json!({ "ids": moving, "parent": parent, "index": index }));
        cx.notify();
    }
}

fn path_id(seg: Option<&str>) -> Option<NodeId> {
    seg.and_then(|s| s.parse::<u64>().ok()).map(NodeId)
}
