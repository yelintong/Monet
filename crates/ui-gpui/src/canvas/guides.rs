//! 标尺参考线：从标尺拖出新增，拖参考线移动（Alt 复制），
//! 拖回标尺删除；点击选中（Delete、方向键、右键菜单作用于它）。落在顶层帧上的参考线属于该帧：
//! 存在帧坐标里、跟着帧走、只跨该帧（旋转帧沿自己的轴）。参考线随标尺显示；图层会吸附到它们。
//! 拖拽时按住 Alt 显示参考线到所跨帧中图层各条边的距离。

use figcraft_doc::{Document, NodeId};
use figcraft_engine::guides;
use figcraft_geom::Point;
use serde_json::json;

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;

/// 标尺厚度（与画布标尺一致）。
pub(super) const RULER: f32 = 20.0;
const GUIDE: Color32 = Color32::from_rgb(0xF2, 0x48, 0x22);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GuideDrag {
    /// 从标尺里拖出新参考线。
    New { vertical: bool, frame: Option<NodeId> },
    /// 移动 `owner`（页面或帧）的第 `index` 条；`copy`（按下时的 Alt）另放一条新的。
    Move { owner: NodeId, index: usize, vertical: bool, copy: bool, frame: Option<NodeId> },
}

impl GuideDrag {
    /// Alt 在哪个帧里测量：拖拽开始时选中的那个，否则参考线自己所属的。
    fn frame(self) -> Option<NodeId> {
        match self {
            GuideDrag::New { frame, .. } | GuideDrag::Move { frame, .. } => frame,
        }
    }

    fn vertical(self) -> bool {
        match self {
            GuideDrag::New { vertical, .. } | GuideDrag::Move { vertical, .. } => vertical,
        }
    }
}

/// 参考线落到的位置。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    /// 所属页面或帧。
    pub owner: NodeId,
    pub index: usize,
    pub vertical: bool,
    /// 在 owner 坐标里的偏移（标签显示的就是它；页面参考线就是页面坐标）。
    pub offset: f64,
    /// 帧参考线的页面线段，沿帧自己的轴穿过整帧；None：页面参考线，横跨整个画布。
    pub line: Option<[Point; 2]>,
}

/// 参考线拖拽的当前状态（对应原文的 egui 临时数据）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    pub drag: Option<GuideDrag>,
    pub preview: Option<Placed>,
}

/// 页面的参考线与顶层帧的参考线（帧在后，所以先被命中）。
fn placed_guides(app: &CanvasApp) -> Vec<Placed> {
    let Ok(ds) = app.session.ds() else { return vec![] };
    let doc = &ds.doc;
    let mut out: Vec<Placed> = doc
        .node(ds.page)
        .map(|n| n.guides())
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, g)| Placed { owner: ds.page, index, vertical: g.vertical, offset: g.offset, line: None })
        .collect();
    for f in doc.top_frames(ds.page) {
        let Some(n) = doc.node(f).filter(|n| n.visible) else { continue };
        out.extend(n.guides.iter().enumerate().filter_map(|(index, g)| place(doc, f, index, g.vertical, g.offset)));
    }
    out
}

/// 已放好的参考线在页面轴上的表达（是否竖线、页面偏移），当它沿页面轴时。
fn page_line(app: &CanvasApp, g: &Placed) -> Option<(bool, f64)> {
    match g.line {
        None => Some((g.vertical, g.offset)),
        Some(_) => guides::line_in(app.session.doc().ok()?.world_transform(g.owner), g.vertical, g.offset),
    }
}

/// 选中项所在的顶层帧。
fn selected_frame(app: &CanvasApp) -> Option<NodeId> {
    let ds = app.session.ds().ok()?;
    ds.selection.first().and_then(|i| ds.doc.top_level_frame(*i))
}

/// 帧 `owner` 里偏移 `offset`（帧坐标）的第 `index` 条参考线落在页面的哪里。
fn place(doc: &Document, owner: NodeId, index: usize, vertical: bool, offset: f64) -> Option<Placed> {
    Some(Placed { owner, index, vertical, offset, line: Some(guides::segment(doc, owner, vertical, offset)?) })
}

/// `owner`（页面或帧）里过页面点 `p` 的第 `index` 条参考线；`snap` 时偏移取整像素，否则到百分位。
fn through(app: &CanvasApp, owner: NodeId, index: usize, vertical: bool, p: Point, snap: bool) -> Option<Placed> {
    let ds = app.session.ds().ok()?;
    let v = guides::offset_at(&ds.doc, owner, vertical, p)?;
    let offset = if snap { v.round() } else { (v * 100.0).round() / 100.0 };
    if owner == ds.page { Some(Placed { owner, index, vertical, offset, line: None }) } else { place(&ds.doc, owner, index, vertical, offset) }
}

/// 从标尺拖出（`vertical`：左侧标尺）、在页面点 `p` 放下的参考线：落在下面的顶层帧里就用帧坐标、
/// 沿更接近标尺的那条轴，否则落在页面上。
fn drop_target(app: &CanvasApp, p: Point, vertical: bool, snap: bool) -> Option<Placed> {
    let ds = app.session.ds().ok()?;
    match guides::drop_owner(&ds.doc, ds.page, p) {
        f if f != ds.page => through(app, f, usize::MAX, guides::frame_axis(&ds.doc, f, vertical), p, snap),
        page => through(app, page, usize::MAX, vertical, p, snap),
    }
}

/// 参考线命令的参数：帧参考线要点名帧，页面参考线不用。
fn owned(owner: NodeId, page: Option<NodeId>, mut params: serde_json::Value) -> serde_json::Value {
    if Some(owner) != page
        && let Some(o) = params.as_object_mut()
    {
        o.insert("frame".into(), json!(owner));
    }
    params
}

fn rulers_on(app: &CanvasApp) -> bool {
    app.session.ds().is_ok_and(|d| d.view.rulers)
}

/// 参考线在屏幕上的线段：页面参考线横跨画布 `r`，帧参考线横跨它的帧。
fn segment(app: &CanvasApp, g: &Placed, r: Rect) -> [Pos2; 2] {
    match g.line {
        Some([a, b]) => [app.to_screen(a.x, a.y), app.to_screen(b.x, b.y)],
        None if g.vertical => {
            let x = app.to_screen(g.offset, 0.0).x;
            [pos2(x, r.min.y), pos2(x, r.max.y)]
        }
        None => {
            let y = app.to_screen(0.0, g.offset).y;
            [pos2(r.min.x, y), pos2(r.max.x, y)]
        }
    }
}

/// 离 `pos` 几点以内的参考线（帧参考线与后放的优先）。
fn guide_at(app: &CanvasApp, pos: Pos2) -> Option<Placed> {
    let r = app.canvas.rect;
    placed_guides(app).into_iter().rev().find(|g| {
        let [a, b] = segment(app, g, r);
        let ab = b - a;
        let t = if ab.length_sq() > 1e-6 { ((pos - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
        (a + ab * t).distance(pos) <= 3.0
    })
}

/// 拖参考线穿过它自己时的缩放光标，按它屏幕上的走向。
fn cursor_across(app: &CanvasApp, g: &Placed, r: Rect) -> gpui::CursorStyle {
    let [a, b] = segment(app, g, r);
    let d = b - a;
    // 线方向按八分之一圈取整（屏幕 y 向下）：0 横向、2 纵向。
    let eighths = (d.y.atan2(d.x).to_degrees().rem_euclid(180.0) + 22.5) / 45.0;
    match eighths as u8 % 4 {
        0 => gpui::CursorStyle::ResizeUpDown,
        1 => gpui::CursorStyle::ResizeUpRightDownLeft,
        2 => gpui::CursorStyle::ResizeLeftRight,
        _ => gpui::CursorStyle::ResizeUpLeftDownRight,
    }
}

fn cross_cursor(vertical: bool) -> gpui::CursorStyle {
    if vertical { gpui::CursorStyle::ResizeLeftRight } else { gpui::CursorStyle::ResizeUpDown }
}

/// 指针处理；事件属于标尺或参考线时返回 true（状态存在 `app.guides` 里）。
pub(crate) fn input(app: &mut CanvasApp, pos: Pos2, pressed: bool, released: bool, alt: bool) -> bool {
    let mut st = app.guides;
    let hit = input_inner(app, &mut st, pos, pressed, released, alt);
    app.guides = st;
    hit
}

fn input_inner(app: &mut CanvasApp, st: &mut State, pos: Pos2, pressed: bool, released: bool, alt: bool) -> bool {
    if !rulers_on(app) {
        return false;
    }
    let r = app.canvas.rect;
    if let Some(drag) = st.drag {
        let vertical = drag.vertical();
        let p = app.to_page(pos);
        let snap = app.session.ds().map(|d| d.view.snap_pixel).unwrap_or(true);
        let at = match drag {
            GuideDrag::New { vertical, .. } => drop_target(app, p, vertical, snap),
            GuideDrag::Move { owner, index, vertical, .. } => through(app, owner, index, vertical, p, snap),
        };
        // Alt：参考线到所跨帧里各图层边的距离。
        let frame = drag.frame().or_else(|| at.map(|g| g.owner).filter(|o| app.session.ds().is_ok_and(|d| d.page != *o)));
        app.session.tool.overlay.guides = match (alt && !released, frame, at.and_then(|g| page_line(app, &g))) {
            (true, Some(f), Some((v, off))) => app.session.doc().map(|d| figcraft_engine::distance::from_guide(d, f, v, off)).unwrap_or_default(),
            _ => Vec::new(),
        };
        st.preview = at;
        app.guide_cursor = Some(super::cursors::Shown::System(at.map_or(cross_cursor(vertical), |g| cursor_across(app, &g, r))));
        if released {
            st.drag = None;
            st.preview = None;
            // 落在自己的标尺上（往下走的线看左侧标尺）：不放参考线（被移动的那条被删掉）。
            let down = at.map_or(vertical, |g| {
                let [a, b] = segment(app, &g, r);
                (b.y - a.y).abs() >= (b.x - a.x).abs()
            });
            let on_ruler = if down { pos.x < r.min.x + RULER } else { pos.y < r.min.y + RULER };
            let page = app.session.ds().map(|d| d.page).ok();
            let before = app.session.doc().ok().and_then(|d| match drag {
                GuideDrag::Move { owner, index, .. } => d.node(owner).and_then(|n| n.guides().get(index)).map(|g| g.offset),
                GuideDrag::New { .. } => None,
            });
            let run = |app: &mut CanvasApp, name: &str, params: serde_json::Value| {
                let _ = app.session.execute(name, &params);
            };
            match (drag, on_ruler, at) {
                (GuideDrag::New { .. } | GuideDrag::Move { copy: true, .. }, false, Some(g)) if Some(g.offset) != before => {
                    run(app, "guide.add", owned(g.owner, page, json!({"vertical": g.vertical, "offset": g.offset})));
                }
                (GuideDrag::Move { owner, index, copy: false, .. }, true, _) => {
                    run(app, "guide.remove", owned(owner, page, json!({"index": index})));
                }
                // 没移动的点击保持原样（只是选中）。
                (GuideDrag::Move { owner, index, copy: false, .. }, false, Some(g)) if Some(g.offset) != before => {
                    run(app, "guide.move", owned(owner, page, json!({"index": index, "offset": g.offset})));
                }
                _ => {}
            }
        }
        return true;
    }
    let in_top = pos.y < r.min.y + RULER && pos.x > r.min.x + RULER;
    let in_left = pos.x < r.min.x + RULER && pos.y > r.min.y + RULER;
    if in_top || in_left {
        app.guide_cursor = Some(super::cursors::Shown::System(cross_cursor(in_left)));
        if pressed {
            let frame = selected_frame(app);
            st.drag = Some(GuideDrag::New { vertical: in_left, frame });
        }
        return true;
    }
    if pos.x < r.min.x + RULER || pos.y < r.min.y + RULER {
        return true;
    }
    // 已有参考线用移动工具拖动（编辑文字/矢量时不抢）。
    let idle = app.session.tool.tool == figcraft_engine::tools::ToolId::Move
        && app.session.ds().is_ok_and(|d| d.text_edit.is_none() && d.vector_edit.is_none());
    if idle && let Some(g) = guide_at(app, pos) {
        app.guide_cursor = Some(super::cursors::Shown::System(cursor_across(app, &g, r)));
        let page = app.session.ds().map(|d| d.page).ok();
        if pressed {
            // 点之前选中的帧才是 Alt 要测量的那个。
            let frame = selected_frame(app).or((Some(g.owner) != page).then_some(g.owner));
            let _ = app.session.execute("guide.select", &owned(g.owner, page, json!({"index": g.index})));
            st.drag = Some(GuideDrag::Move { owner: g.owner, index: g.index, vertical: g.vertical, copy: alt, frame });
        }
        return true;
    }
    false
}

/// 鼠标已经松开、但 `input` 没接到这次松开（在画布外松手，或松开时指针压在别的控件上）：
/// 把这次拖动丢掉，免得下一次点击接着把它当成「还在拖」去加/移参考线。
///
/// 原文 `ui-egui/src/canvas/guides.rs::end_released` 每帧调一次；GPUI 侧漏了这一步。
pub(crate) fn end_released(app: &mut CanvasApp) {
    if app.guides.drag.is_none() && app.guides.preview.is_none() {
        return;
    }
    app.guides.drag = None;
    app.guides.preview = None;
}

/// 画页面与帧的参考线（在标尺下面），以及正在拖的那条和它的位置读数。
pub(super) fn draw(app: &CanvasApp, painter: &mut Painter, st: &State) {
    if !rulers_on(app) {
        return;
    }
    let r = app.canvas.rect;
    let moving = match st.drag {
        Some(GuideDrag::Move { owner, index, copy: false, .. }) => Some((owner, index)),
        _ => None,
    };
    let hover = if st.drag.is_none() { app.pointer.filter(|p| r.contains(*p)).and_then(|p| guide_at(app, p)) } else { None };
    let selected = app.session.ds().ok().and_then(|d| d.selected_guide());
    for g in placed_guides(app) {
        if Some((g.owner, g.index)) == moving {
            continue;
        }
        let hovered = hover == Some(g) || selected == Some((g.owner, g.index));
        painter.line_segment(segment(app, &g, r), Stroke::new(if hovered { 2.0 } else { 1.0 }, GUIDE));
        if hovered {
            label(app, painter, r, &g);
        }
    }
    if let Some(g) = st.preview {
        painter.line_segment(segment(app, &g, r), Stroke::new(1.0, GUIDE));
        label(app, painter, r, &g);
    }
}

/// 参考线的偏移（页面或帧坐标），标在它靠标尺的那头。
fn label(app: &CanvasApp, painter: &mut Painter, r: Rect, g: &Placed) {
    let galley = painter.layout_no_wrap(super::rulers::fmt(g.offset), semibold(10.0), Color32::WHITE);
    let size = galley.size() + vec2(8.0, 4.0);
    let [a, _] = segment(app, g, r);
    // 画布比标尺还窄/矮时 `min > max`，那就把标签留在标尺边上。
    let (lo_x, lo_y) = (r.min.x + RULER, r.min.y + RULER);
    let at = pos2(a.x.clamp(lo_x, r.max.x.max(lo_x)) + 4.0, a.y.clamp(lo_y, r.max.y.max(lo_y)) + 4.0);
    let tag = Rect::from_min_size(at, size);
    painter.rect_filled(tag, CornerRadius::same(3), GUIDE);
    painter.galley(tag.min + vec2(4.0, 2.0), galley, Color32::WHITE);
}
