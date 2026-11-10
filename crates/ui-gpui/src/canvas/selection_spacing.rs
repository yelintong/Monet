//! Smart-selection fields and handles. Geometry and document edits belong to the engine.
//!
//! 照搬 `ui-egui/src/canvas/selection_spacing.rs` 的 `draw` / `input` / `sync` / `cancel`
//! （`panel` 留到面板阶段）。后端差异（均已注释）：
//! - egui 的 `ctx.data` 临时状态 → `CanvasApp` 上的 [`State`]；
//! - `ctx.input` 的按键/修饰键 → 调用方显式传入；
//! - 没有 `toast` → 命令出错时直接忽略（原文弹提示）；
//! - `right_tab` / `picker`（面板状态）暂缺 → [`available`] 只查工具与编辑态。

use figcraft_i18n::tr;
use std::sync::Arc;

use figcraft_doc::{Document, NodeId};
use figcraft_engine::ToolId;
use figcraft_engine::selection_reorder::ReorderLayout;
use figcraft_engine::selection_spacing::{Axis, SelectionSpacing, selection_spacing};
use serde_json::json;

use super::geom::*;
use super::handles::{PINK, pink_label};
use super::outline::stroke;
use super::overlays::accent;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::canvas::cursors::Shown;

#[derive(Clone)]
struct Cache {
    doc: Arc<Document>,
    page: NodeId,
    ids: Vec<NodeId>,
    layout: Arc<SelectionSpacing>,
    reorder: Option<Arc<ReorderLayout>>,
}

/// 间距/重排拖拽 + 一份按文档快照的布局缓存（原文按帧缓存，这里按 `Arc` 指针缓存）。
#[derive(Clone, Default)]
pub struct State {
    drag: Option<Drag>,
    cache: Option<Cache>,
}

#[derive(Clone)]
enum Action {
    Spacing { axis: Axis, value: f64, last_value: Option<f64> },
    Reorder { layout: Arc<ReorderLayout>, source: NodeId, target: NodeId, swap: bool },
}

#[derive(Clone)]
struct Drag {
    ids: Vec<NodeId>,
    action: Action,
    start: Pos2,
    uid: u64,
    page: NodeId,
    moved: bool,
}

#[derive(Clone)]
struct Handle {
    value: f64,
    center: Pos2,
    hit: Rect,
}

/// 一份选择/页面的间距计算缓存；文档 `Arc` 换了才重算（原文是「下一帧刷新」）。
fn layout(app: &mut CanvasApp, ids: &[NodeId]) -> Arc<SelectionSpacing> {
    let Ok(ds) = app.session.ds() else { return Arc::new(SelectionSpacing::default()) };
    if let Some(c) = &app.selection_spacing.cache
        && c.page == ds.page
        && c.ids == ids
        && Arc::ptr_eq(&c.doc, &ds.doc)
    {
        return c.layout.clone();
    }
    let layout = Arc::new(selection_spacing(&ds.doc, ids));
    let reorder = ReorderLayout::new(&ds.doc, ids).map(Arc::new);
    app.selection_spacing.cache = Some(Cache { doc: ds.doc.clone(), page: ds.page, ids: ids.to_vec(), layout: layout.clone(), reorder });
    layout
}

fn reorder_layout(app: &mut CanvasApp, ids: &[NodeId]) -> Option<Arc<ReorderLayout>> {
    layout(app, ids);
    app.selection_spacing.cache.as_ref().and_then(|c| c.reorder.clone())
}

/// 重排环：屏幕上的圆心与命中半径（8px 上限，占小图层四分之一）。
fn ring(app: &CanvasApp, b: figcraft_geom::Rect) -> Option<(Pos2, f32)> {
    let zoom = app.session.ds().map_or(1.0, |d| d.view.zoom);
    let side = ((b.x1 - b.x0).min(b.y1 - b.y0) * zoom) as f32;
    (side >= 16.0).then(|| (app.to_screen((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5), (side / 4.0).min(8.0)))
}

fn bounds(doc: &Document, ids: &[NodeId]) -> Option<figcraft_geom::Rect> {
    ids.iter().filter_map(|id| doc.world_bounds(*id)).reduce(|a, b| a.union(b))
}

fn handles(app: &CanvasApp, axis: &Axis) -> Vec<Handle> {
    let Ok(doc) = app.session.doc() else { return vec![] };
    let cross = |b: figcraft_geom::Rect| if axis.vertical { (b.x0, b.x1) } else { (b.y0, b.y1) };
    let mut out = Vec::new();
    for pair in axis.tracks.windows(2) {
        let Some((a, b)) = pair.first().zip(pair.get(1)) else { continue };
        let (Some(ab), Some(bb)) = (bounds(doc, &a.ids), bounds(doc, &b.ids)) else { continue };
        let gap = axis.start(bb) - axis.end(ab);
        let middle = (axis.start(bb) + axis.end(ab)) * 0.5;
        // 相邻两列的交叉跨度求交：一次扫描，不两两比较。
        let mut aa: Vec<_> = a.ids.iter().filter_map(|id| doc.world_bounds(*id)).map(cross).collect();
        let mut bs: Vec<_> = b.ids.iter().filter_map(|id| doc.world_bounds(*id)).map(cross).collect();
        aa.sort_by(|a, b| a.0.total_cmp(&b.0));
        bs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut i, mut j) = (0, 0);
        while let (Some(a), Some(b)) = (aa.get(i), bs.get(j)) {
            let (lo, hi) = (a.0.max(b.0), a.1.min(b.1));
            if hi > lo {
                let c = (lo + hi) * 0.5;
                let center = if axis.vertical { app.to_screen(c, middle) } else { app.to_screen(middle, c) };
                let size = if axis.vertical { vec2(24.0, 10.0) } else { vec2(10.0, 24.0) };
                out.push(Handle { value: gap, center, hit: Rect::from_center_size(center, size) });
            }
            if a.1 <= b.1 {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    out
}

/// 原文还查 `right_tab == Design` 与 `picker.is_none()`；GPUI 还没有面板，先只查工具/编辑态。
fn available(app: &CanvasApp) -> bool {
    app.session.tool.tool == ToolId::Move
        && !app.session.tool.busy()
        && app.session.ds().is_ok_and(|d| d.text_edit.is_none() && d.vector_edit.is_none())
}

/// `widgets::fmt_num` 的原样拷贝（原文在 `ui-egui/src/widgets.rs:178`）。
fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

fn start_drag(app: &mut CanvasApp, ids: Vec<NodeId>, p: Pos2, action: Action) {
    let label = match action {
        Action::Spacing { .. } => tr("Spacing"),
        Action::Reorder { .. } => tr("Reorder selection"),
    };
    if app.session.begin_interaction(label).is_ok() {
        let (uid, page) = match app.session.ds() {
            Ok(ds) => (ds.uid, ds.page),
            Err(_) => return,
        };
        app.selection_spacing.drag = Some(Drag { ids, action, start: p, uid, page, moved: false });
    }
}

impl State {
    /// 是否正在拖间距/重排（指针抬起时要把它收尾）。
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// 拖间距/边距手柄、且已经挪动时，返回应作为 live 替身**当帧重抓**的图层。
    ///
    /// 直接复用 resize 的 live-layer reshape 路径：间距手柄每帧 `arrange.spacing` 指令下放改文档，
    /// 内容随之移动；把它并进 `moving` + `redraw=true`，画布就走和 resize 完全相同的
    /// 「整页让位 + 当帧重抓替身」——内容贴着框走，不再异步整页重渲导致背景跟不上框（果冻）。
    /// 没在拖、或还没挪动时返回 `None`，画布走普通整页渲染。
    pub fn live_layers(&self) -> Option<(Vec<NodeId>, bool)> {
        match &self.drag {
            Some(d) if d.moved => Some((d.ids.clone(), true)),
            _ => None,
        }
    }
}

pub fn cancel(app: &mut CanvasApp) -> bool {
    if let Some(d) = app.selection_spacing.drag.take() {
        figcraft_engine::selection_spacing::cancel_gesture(&mut app.session, d.uid);
        true
    } else {
        false
    }
}

/// 文档/页面/选择/工具换掉时撤销手势（原文 `sync`）。
pub fn sync(app: &mut CanvasApp) {
    let Some(d) = app.selection_spacing.drag.as_ref() else { return };
    let same = app.session.ds().is_ok_and(|ds| ds.uid == d.uid && ds.page == d.page && ds.selection == d.ids && ds.interaction.is_some());
    let ok = same && available(app);
    if !ok {
        cancel(app);
    }
}

/// 指针处理；返回 true 表示这一下归间距/重排手柄。
#[allow(clippy::too_many_arguments)]
pub fn input(
    app: &mut CanvasApp,
    pos: Option<Pos2>,
    hovered: bool,
    space: bool,
    pressed: bool,
    down: bool,
    released: bool,
    command: bool,
    alt: bool,
    shift: bool,
) -> bool {
    if let Some(mut drag) = app.selection_spacing.drag.clone() {
        let same =
            app.session.ds().is_ok_and(|ds| ds.uid == drag.uid && ds.page == drag.page && ds.selection == drag.ids && ds.interaction.is_some());
        if !same {
            cancel(app);
            return true;
        }
        if let Some(p) = pos {
            match &mut drag.action {
                Action::Spacing { axis, value: original, last_value } => {
                    let zoom = app.session.ds().map_or(1.0, |d| d.view.zoom).max(1e-6);
                    let distance = if axis.vertical { p.y - drag.start.y } else { p.x - drag.start.x };
                    drag.moved |= distance.abs() >= 3.0;
                    if drag.moved {
                        // Alt 0.1 / Shift 10 / 默认 1 的步进（原文 quantum）。
                        let quantum = if alt {
                            0.1
                        } else if shift {
                            10.0
                        } else {
                            1.0
                        };
                        let value = (((*original + f64::from(distance) / zoom) / quantum).round() * quantum).clamp(-1e6, 1e6);
                        let key = if axis.vertical { "vertical" } else { "horizontal" };
                        if *last_value != Some(value)
                            && app.session.execute("arrange.spacing", &json!({"ids": drag.ids, key: value, "fromGesture": true})).is_ok()
                        {
                            *last_value = Some(value);
                        }
                    }
                }
                Action::Reorder { layout, source, target, swap } if app.canvas.rect.contains(p) => {
                    drag.moved |= (p - drag.start).length() >= 3.0;
                    // 命中原始槽位，不认移动中的预览：否则指针不动也会在两种排列间来回切。
                    let dist = |r: figcraft_geom::Rect| {
                        let c = app.to_screen((r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5);
                        (c - p).length()
                    };
                    let nearest = layout.slots.iter().min_by(|a, b| dist(a.1).total_cmp(&dist(b.1))).map(|(id, _)| *id).unwrap_or(*source);
                    if drag.moved
                        && (nearest != *target || command != *swap)
                        && app
                            .session
                            .execute(
                                "arrange.reorder",
                                &json!({"ids": drag.ids, "source": source, "target": nearest, "swap": command, "fromGesture": true}),
                            )
                            .is_ok()
                    {
                        *target = nearest;
                        *swap = command;
                    }
                }
                // 在面板/窗口外松开：保留最后一次预览，不选新槽位。
                Action::Reorder { .. } => {}
            }
        }
        app.spacing_cursor = Some(Shown::System(match &drag.action {
            Action::Spacing { axis, .. } if axis.vertical => gpui::CursorStyle::ResizeUpDown,
            Action::Spacing { .. } => gpui::CursorStyle::ResizeLeftRight,
            Action::Reorder { .. } => gpui::CursorStyle::ClosedHand,
        }));
        if released || !down {
            let _ = app.session.commit_interaction();
            app.selection_spacing.drag = None;
        } else {
            app.selection_spacing.drag = Some(drag);
        }
        return true;
    }
    if !hovered || space || !available(app) {
        return false;
    }
    let Some(p) = pos else { return false };
    let ids = app.session.selection();
    if let Some(reorder) = reorder_layout(app, &ids) {
        for (id, b) in &reorder.slots {
            if ring(app, *b).is_some_and(|(center, r)| (center - p).length() <= r) {
                app.spacing_cursor = Some(Shown::System(gpui::CursorStyle::OpenHand));
                if pressed {
                    let source = *id;
                    start_drag(app, ids, p, Action::Reorder { layout: reorder, source, target: source, swap: false });
                }
                return true;
            }
        }
    }
    if command {
        return false;
    }
    let layout = layout(app, &ids);
    let axes: Vec<Axis> = [layout.horizontal.as_ref(), layout.vertical.as_ref()].into_iter().flatten().cloned().collect();
    for axis in axes {
        let vertical = axis.vertical;
        for h in handles(app, &axis) {
            if !h.hit.contains(p) {
                continue;
            }
            app.spacing_cursor = Some(Shown::System(if vertical { gpui::CursorStyle::ResizeUpDown } else { gpui::CursorStyle::ResizeLeftRight }));
            if pressed {
                start_drag(app, ids, p, Action::Spacing { value: h.value, last_value: axis.value, axis });
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三个同排 50 宽的方块，间距 30：横轴应当只有一条、值 30。
    fn setup() -> (CanvasApp, Vec<NodeId>) {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        let mut ids = Vec::new();
        for x in [0.0, 80.0, 160.0] {
            let v = app.session.execute("shape.rectangle", &json!({"x": x, "y": 900.0, "width": 50.0, "height": 50.0})).expect("rect");
            ids.push(serde_json::from_value(v["id"].clone()).expect("id"));
        }
        let _ = app.session.set_selection(ids.clone());
        (app, ids)
    }

    #[test]
    fn horizontal_spacing_reads_the_uniform_gap() {
        let (mut app, ids) = setup();
        let l = layout(&mut app, &ids);
        assert_eq!(l.horizontal.as_ref().and_then(|a| a.value), Some(30.0));
        assert!(l.vertical.is_none(), "one row: no vertical spacing");
    }

    #[test]
    fn dragging_a_spacing_handle_writes_arrange_spacing() {
        let (mut app, ids) = setup();
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        app.session.tool.tool = ToolId::Move;
        let before = app.session.doc().ok().and_then(|d| d.node(ids[1])).map(|n| n.transform);
        // 第一列右边缘 50、第二列左边缘 80：手柄在 x=65。
        let start = app.to_screen(65.0, 925.0);
        assert!(input(&mut app, Some(start), true, false, true, true, false, false, false, false), "press on the handle");
        assert!(app.selection_spacing.is_dragging(), "press starts a spacing drag");
        let moved = app.to_screen(85.0, 925.0);
        let _ = input(&mut app, Some(moved), true, false, false, true, false, false, false, false);
        let after = app.session.doc().ok().and_then(|d| d.node(ids[1])).map(|n| n.transform);
        assert_ne!(before, after, "the second column follows the new spacing");
    }
}

/// 画重排环与间距手柄（原文 `draw`）。
pub fn draw(app: &mut CanvasApp, painter: &mut Painter) {
    let dragging = app.selection_spacing.drag.is_some();
    if !available(app) && !dragging {
        return;
    }
    let (doc, selection) = match app.session.ds() {
        Ok(ds) => (ds.doc.clone(), ds.selection.clone()),
        Err(_) => return,
    };
    let hover = app.pointer;
    let Some(b) = bounds(&doc, &selection) else { return };
    let rect = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(12.0);
    if !dragging && !hover.is_some_and(|p| rect.contains(p)) {
        return;
    }
    let reorder = match app.selection_spacing.drag.as_ref().map(|d| &d.action) {
        Some(Action::Reorder { layout, .. }) => Some(layout.clone()),
        Some(Action::Spacing { .. }) => None,
        None => reorder_layout(app, &selection),
    };
    if let Some(layout) = reorder {
        for (id, _) in &layout.slots {
            let Some((center, r)) = doc.world_bounds(*id).and_then(|b| ring(app, b)) else { continue };
            let hot = hover.is_some_and(|p| (center - p).length() <= r);
            let active = matches!(app.selection_spacing.drag.as_ref().map(|d| &d.action), Some(Action::Reorder { source, .. }) if source == id);
            if active {
                painter.circle_filled(center, 4.5, PINK);
            } else {
                painter.circle_stroke(center, 4.5, stroke(PINK, if hot { 2.5 } else { 1.5 }));
            }
        }
    }
    if let Some(Drag { action: Action::Reorder { layout, target, .. }, moved: true, .. }) = &app.selection_spacing.drag {
        if let Some((_, b)) = layout.slots.iter().find(|(id, _)| id == target) {
            let r = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(3.0);
            painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent(app), 2.0), StrokeKind::Outside);
        }
        return;
    }
    let axes: Vec<Axis> = if let Some(Drag { action: Action::Spacing { axis, .. }, .. }) = &app.selection_spacing.drag {
        vec![axis.clone()]
    } else {
        let l = layout(app, &selection);
        [l.horizontal.clone(), l.vertical.clone()].into_iter().flatten().collect()
    };
    for axis in axes {
        for h in handles(app, &axis) {
            let hot = hover.is_some_and(|p| h.hit.contains(p)) || dragging;
            let by = if axis.vertical { vec2(10.0, 0.0) } else { vec2(0.0, 10.0) };
            painter.line_segment([h.center - by, h.center + by], stroke(PINK, if hot { 3.0 } else { 2.0 }));
            if hot {
                let at = h.center + if axis.vertical { vec2(18.0, 0.0) } else { vec2(0.0, -22.0) };
                pink_label(painter, fmt_num(h.value), at);
            }
        }
    }
}
