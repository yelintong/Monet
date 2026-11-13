//! 页面之上的叠加层（照搬 `ui-egui/src/canvas/overlays.rs` 的画法与顺序）：
//! 像素网格 → 布局网格与帧标签 → hover 轮廓 → 布尔操作数 → 蒙版/帧/切图轮廓 →
//! 拖放目标 → 选中轮廓与变换框（含连接线端点、旋转原点）→ 框选 → 钢笔/橡皮筋/套索/橡皮 →
//! 吸附参考线与尺寸。

use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::tools::{ToolId, selection_box, selection_frame};
use figcraft_geom::Point;

use super::geom::*;
use super::outline::{draw_outline, node_quad, outline_colour, quad, stroke};
use super::painter::Painter;
use super::rulers::fmt;
use crate::app::CanvasApp;
use crate::theme::{COMPONENT, Tokens, stroke as s};

/// 三次贝塞尔在 `t` 处的点（GPUI 画笔没有贝塞尔图元，段要采样成折线）。
fn cubic_at(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    pos2(p0.x * w0 + p1.x * w1 + p2.x * w2 + p3.x * w3, p0.y * w0 + p1.y * w1 + p2.y * w2 + p3.y * w3)
}

/// 矢量编辑叠加层（照抄 `ui-egui/src/canvas/overlays.rs::vector_overlay`）：
/// 贝塞尔段（采样成折线）+ 手柄线 + 方形点（hover 放大、选中 accent）。
///
/// 用户 2026-11-12：进入矢量编辑后必须能看到每个节点的手柄、能点中它们。
fn vector_overlay(
    app: &CanvasApp,
    painter: &mut Painter,
    o: &figcraft_engine::vectoredit::EditOverlay,
    hover: Option<figcraft_engine::vectoredit::VHit>,
    accent: Color32,
) {
    use figcraft_engine::vectoredit::VHit;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    let hover_seg = match hover {
        Some(VHit::Segment { seg, .. }) => Some(seg),
        _ => None,
    };
    // 段：三次贝塞尔采样成折线。
    for (i, seg) in o.segments.iter().enumerate() {
        let w = if o.selected_segments.contains(&i) {
            3.0
        } else if hover_seg == Some(i) {
            2.5
        } else {
            1.5
        };
        let (p0, p1, p2, p3) = (sp(&seg[0]), sp(&seg[1]), sp(&seg[2]), sp(&seg[3]));
        let mut prev = p0;
        for step in 1..=24 {
            let q = cubic_at(p0, p1, p2, p3, step as f32 / 24.0);
            painter.line_segment([prev, q], s(accent, w));
            prev = q;
        }
    }
    // 手柄线 + 端点圆（hover 放大、选中 accent）。
    for h in &o.handles {
        let a = sp(&h.vertex);
        let b = sp(&h.handle);
        painter.line_segment([a, b], s(accent, 1.0));
        let on = o.selected_handles.contains(&(h.seg, h.start));
        let hovered = hover == Some(VHit::Handle { seg: h.seg, start: h.start });
        let r = if hovered { 4.5 } else { 3.5 };
        painter.circle_filled(b, r, if on { accent } else { Color32::WHITE });
        painter.circle_stroke(b, r, s(accent, 1.0));
    }
    // 顶点：方形（hover 放大、选中 accent）。
    for (i, v) in o.vertices.iter().enumerate() {
        let p = sp(v);
        let hovered = matches!(hover, Some(VHit::Vertex { index }) if index == i);
        let half = if hovered { 4.5 } else { 3.5 };
        let r = Rect::from_min_max(pos2(p.x - half, p.y - half), pos2(p.x + half, p.y + half));
        let on = o.selected_vertices.contains(&i);
        painter.rect_filled(r, CornerRadius::same(2), if on { accent } else { Color32::WHITE });
        painter.rect_stroke(r, CornerRadius::same(2), s(accent, 1.0), StrokeKind::Inside);
    }
}

/// 选中/叠加层主色（与原文 `accent(painter)` 一致：页面深色用亮绿，浅色用对比绿）。
pub(super) fn accent(app: &CanvasApp) -> Color32 {
    app.accent()
}

/// 画布上所有叠加层。返回每个顶层帧/section 名称标签的屏幕矩形（页面坐标），
/// 供引擎在点击标签时选中/拖拽对应容器（`ToolState::labels`，照搬 `ui-egui`）。
pub(super) fn overlays(app: &CanvasApp, painter: &mut Painter, t: &Tokens) -> Vec<(NodeId, [Point; 4])> {
    let display_doc = app.display_doc.clone();
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let doc = display_doc.unwrap_or_else(|| ds.doc.clone());
    let page = ds.page;
    let mut labels = Vec::new();
    let sel = ds.selection.clone();
    let zoom = ds.view.zoom;
    let view = ds.view.clone();
    let accent = accent(app);

    // ── 高缩放时的像素网格（400% 起）。
    if view.pixel_grid_shown() {
        let r = app.canvas.rect;
        let p0 = app.to_page(r.min);
        let p1 = app.to_page(r.max);
        let col = Color32::from_black_alpha(18);
        let mut x = p0.x.floor();
        while x <= p1.x {
            let sx = app.to_screen(x, 0.0).x;
            painter.line_segment([pos2(sx, r.min.y), pos2(sx, r.max.y)], s(col, 1.0));
            x += 1.0;
        }
        let mut y = p0.y.floor();
        while y <= p1.y {
            let sy = app.to_screen(0.0, y).y;
            painter.line_segment([pos2(r.min.x, sy), pos2(r.max.x, sy)], s(col, 1.0));
            y += 1.0;
        }
    }

    // ── 布局网格与帧标签（顶层帧，含 section 内的帧），按 z 序。
    let focus = ds.focused();
    for c in super::rulers::boards(&doc, page).iter().filter(|c| focus.is_none_or(|f| f == **c)) {
        let Some(n) = doc.node(*c) else { continue };
        let Some(b) = doc.world_bounds(*c) else { continue };
        // 屏幕外的帧既不画网格也不画标签（标签在帧外）。
        let on_screen = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(20.0);
        if !on_screen.intersects(app.canvas.rect) {
            continue;
        }
        if view.layout_grids && !n.grids.is_empty() && figcraft_layout::grids_shown(&doc, *c) {
            let w = doc.world_transform(*c);
            for g in n.grids.iter().filter(|g| g.visible) {
                let col = crate::theme::doc_c32(g.color);
                for line in figcraft_layout::grid_lines(g, n.width, n.height) {
                    let to_screen = |q: figcraft_geom::Point| {
                        let p = w * q;
                        app.to_screen(p.x, p.y)
                    };
                    match line {
                        figcraft_layout::GridLine::Line { a, b } => {
                            painter.line_segment([to_screen(a), to_screen(b)], s(col, 1.0));
                        }
                        figcraft_layout::GridLine::SlashLine(pts) => {
                            let v: Vec<Pos2> = pts.into_iter().map(to_screen).collect();
                            if v.len() >= 2 {
                                painter.add_shape(v, false, Color32::TRANSPARENT, s(col, 1.0));
                            }
                        }
                        figcraft_layout::GridLine::Band(r) => {
                            let (a, b) = (to_screen(figcraft_geom::Point::new(r.x0, r.y0)), to_screen(figcraft_geom::Point::new(r.x1, r.y1)));
                            painter.rect_filled(Rect::from_two_pos(a, b), CornerRadius::ZERO, col);
                        }
                        figcraft_layout::GridLine::Slash(poly) => {
                            let v: Vec<Pos2> = poly.into_iter().map(to_screen).collect();
                            if v.len() >= 3 {
                                painter.add_shape(v, true, col, Stroke::NONE);
                            }
                        }
                        figcraft_layout::GridLine::Ring { center, radius, width } => {
                            let c = to_screen(center);
                            let rim = to_screen(figcraft_geom::Point::new(center.x + radius, center.y));
                            let r = (rim - c).length();
                            let scale = if radius.abs() > f64::EPSILON { r / radius as f32 } else { 1.0 };
                            painter.circle_stroke(c, r, s(col, if width > 0.0 { (width as f32 * scale).max(1.0) } else { 1.0 }));
                        }
                    }
                }
            }
        }
        let labelled = focus.is_none() && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. })) && !n.is_board_atom();
        if labelled && let Some(q) = super::frame_labels::draw(app, painter, t, &doc, *c, sel.contains(c), ds.hover == Some(*c)) {
            labels.push((*c, q.map(|p| app.to_page(p))));
        }
    }

    // ── Hover 轮廓（测量时改为用红色标出被量的框）。
    if let Some(h) = ds.hover
        && !sel.contains(&h)
        && matches!(app.session.tool.tool, ToolId::Move | ToolId::Scale)
        && app.session.prefs.highlight_on_hover
        && app.session.tool.overlay.measure_box.is_none()
    {
        // 所有节点（含自动布局容器）的描边统一走源码的 draw_outline（选中色/线宽一致）。
        // 自动布局的边距/间距带不在这里画：它是 `handles` 的 hover 手柄（原实现同一处），
        // 悬停到哪条带子才亮哪条，选中时不会常驻。
        draw_outline(app, painter, &doc, h, stroke(outline_colour(&doc, h, accent), 2.0));
    }

    // ── 布尔组内部：每个操作数的轮廓，便于看清实时结果是怎么来的。
    let operands = figcraft_engine::booleans::outlined_operands(&doc, &sel, ds.vector_edit.as_ref().map(|ve| ve.id));
    for id in operands.iter().filter(|o| !sel.contains(o)) {
        draw_outline(app, painter, &doc, *id, stroke(outline_colour(&doc, *id, accent).gamma_multiply(0.6), 1.0));
    }

    // ── 蒙版轮廓（蒙版本体不绘制）、帧轮廓、切图（虚线），由 View 菜单控制。
    if view.mask_outlines || view.frame_outlines || view.show_slices {
        for id in doc.subtree(page) {
            let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
            if view.mask_outlines && n.is_mask {
                draw_outline(app, painter, &doc, id, stroke(accent, 1.0));
            } else if view.frame_outlines && n.kind.is_frame_like() {
                draw_outline(app, painter, &doc, id, stroke(t.text_secondary, 1.0));
            } else if view.show_slices
                && matches!(n.kind, NodeKind::Slice)
                && let Some(q) = node_quad(app, &doc, id)
            {
                for sh in Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(t.text_secondary, 1.0), 4.0, 3.0) {
                    painter.add(sh);
                }
            }
        }
    }

    // ── 拖放目标 / 插入指示 / 网格单元。
    let ov = &app.session.tool.overlay;
    if let Some(d) = ov.drop_target
        && let Some(q) = node_quad(app, &doc, d)
    {
        quad(painter, q, stroke(if figcraft_engine::components::is_slot(&doc, d) { COMPONENT } else { accent }, 1.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.insert_line {
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], s(accent, 2.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.drop_cell {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::same(2), accent.gamma_multiply(0.12));
        let pts = [r.min, pos2(r.max.x, r.min.y), r.max, pos2(r.min.x, r.max.y), r.min];
        for sh in Shape::dashed_line(&pts, stroke(accent, 1.5), 4.0, 3.0) {
            painter.add(sh);
        }
    }

    // ── 矢量编辑：区域斜线剖面（在段与点下面）+ 点/段/手柄。
    if let Some(ve) = ds.vector_edit.as_ref() {
        super::builder::region_hatch(app, painter);
        // 照抄 `ui-egui/src/canvas/overlays.rs:154-175`：先画段，再画手柄，最后画点。
        // hover 由引擎在指针事件里算好（`tool.overlay.vector_hover`），这里只读。
        let ov = &app.session.tool.overlay;
        let hover = ov.vector_hover.filter(|_| ov.vector_hover_on.unwrap_or(ve.id) == ve.id);
        if let Some(o) = figcraft_engine::vectoredit::overlay(&doc, ve) {
            vector_overlay(app, painter, &o, hover, accent);
        }
    }

    // ── 选中：每个选中层的真实轮廓 + 变换框。
    // 选中框的尺寸标签，测量标签要避开它。
    let mut size_label = None;
    let editing = ds.text_edit.is_some();
    // 运动模式播放中：选中边框隐藏（物体在这一帧已不在选中框的位置，留在原地只会造成误导；
    // gizmo / 运动路径也已在播放时藏起，这里保持一致，用户 2026-11-12）。
    if !sel.is_empty() && !editing && ds.vector_edit.is_none() && !app.motion_overlay.playing {
        let col = if sel.iter().any(|i| outline_colour(&doc, *i, accent) == COMPONENT) { COMPONENT } else { accent };
        for id in &sel {
            // 选中描边统一走 draw_outline（源码选中色、1px）。
            draw_outline(app, painter, &doc, *id, stroke(col, 1.0));
        }
        if app.session.tool.tool != ToolId::ShapeBuilder
            && let Some(b) = selection_box(&doc, &sel)
            && let Some((_, w, h)) = selection_frame(&doc, &sel)
        {
            let is_line = sel.len() == 1 && matches!(sel.first().and_then(|i| doc.node(*i)).map(|n| &n.kind), Some(NodeKind::Line));
            // 选中的连接线：手柄就是它的两个端点（可拖到别的对象上）。
            let connector = figcraft_engine::board::selected_connector_ends(&doc, &sel).map(|(_, ends)| ends);
            let mut label = if is_line { fmt(w) } else { format!("{} × {}", fmt(w), fmt(h)) };
            let scale = app.session.tool.overlay.scale;
            if let Some(hint) = scale {
                label = format!("{label} · {}x", fmt(hint.factor.abs()));
            }
            // 屏幕上小于手柄的层只显示轮廓（手柄仍在框外应答）。
            let tiny = figcraft_engine::tools::box_sides_px(&b, zoom).1 < figcraft_engine::tools::SMALL_BOX_PX;
            let measuring = app.session.tool.overlay.measure_box.is_some();
            let handles =
                if measuring || app.session.tool.dev_mode || connector.is_some() || sel.iter().any(|i| figcraft_engine::tools::locked_in(&doc, *i)) {
                    0
                } else if is_line {
                    2
                } else if tiny {
                    0
                } else {
                    4
                };
            size_label = transform_box(
                app,
                painter,
                b.map(|p| app.to_screen(p.x, p.y)),
                col,
                sel.len() > 1,
                handles,
                (!measuring && connector.is_none()).then_some(label),
            );
            for end in connector.into_iter().flatten().filter(|_| !app.session.tool.dev_mode) {
                let c = app.to_screen(end.x, end.y);
                painter.circle_filled(c, 5.0, Color32::WHITE);
                painter.circle_stroke(c, 5.0, stroke(col, 1.5));
            }
            // 圆弧手柄 + 圆角/星形比例手柄：与原实现同处、同顺序（变换框之后、测量时隐藏）。
            if !measuring {
                arc_overlay(app, painter, col);
                shape_overlay(app, painter, col);
            }
            if app.session.tool.tool == ToolId::Scale {
                // 缩放时保持不动的点：拖拽中的支点，否则锚点。
                let pivot = scale
                    .map(|h| figcraft_geom::Point::new(h.pivot[0], h.pivot[1]))
                    .or_else(|| figcraft_engine::tools::scale_anchor_point(&doc, &sel, app.session.tool.scale_anchor));
                if let Some(p) = pivot {
                    let c = app.to_screen(p.x, p.y);
                    painter.circle_filled(c, 4.0, Color32::WHITE);
                    painter.circle_stroke(c, 4.0, stroke(col, 1.5));
                    painter.circle_filled(c, 1.5, col);
                }
            }
            // 旋转原点（Alt+R 显示时）：一个带十字的环。
            if let Some(o) = ds.origin_handle() {
                let c = app.to_screen(o.x, o.y);
                painter.circle_filled(c, 5.0, Color32::WHITE);
                painter.circle_stroke(c, 5.0, stroke(col, 1.5));
                for d in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
                    painter.line_segment([c - d * 10.0, c - d * 5.0], stroke(col, 1.5));
                    painter.line_segment([c + d * 5.0, c + d * 10.0], stroke(col, 1.5));
                }
                painter.circle_filled(c, 1.5, col);
            }
        }
    }

    // ── 形状构建器：各区域边与剖面。
    if app.session.tool.tool == ToolId::ShapeBuilder
        && let Some(arr) = figcraft_engine::shapebuilder::current(&app.session)
    {
        super::builder::builder_overlay(app, painter, t, &arr);
    }

    // ── 缩放工具拖出的区域。
    if let (Some(start), Some(p)) = (app.zoom_drag, app.pointer) {
        let area = Rect::from_two_pos(start, p);
        if area.width() > 4.0 || area.height() > 4.0 {
            painter.rect_filled(area, CornerRadius::ZERO, accent.gamma_multiply(0.1));
            painter.rect_stroke(area, CornerRadius::ZERO, stroke(accent, 1.0), StrokeKind::Inside);
        }
    }

    // ── 框选框。
    if let Some([x0, y0, x1, y1]) = ov.marquee {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.1));
        painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent, 1.0), StrokeKind::Inside);
    }

    // ── 钢笔/铅笔路径。
    if !ov.pen.is_empty() {
        let pts: Vec<Pos2> = ov.pen.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if app.session.tool.tool == ToolId::Pencil {
            // 将要生成的笔画：用它的颜色与粗细（缩到很小时至少 1px）。
            let (color, weight) = figcraft_engine::tools::PENCIL_STROKE;
            painter.add_shape(pts, false, Color32::TRANSPARENT, s(crate::theme::doc_c32(color), ((weight * zoom) as f32).max(1.0)));
        } else {
            for [x0, y0, x1, y1] in &ov.pen_handles {
                bezier_handle(painter, app.to_screen(*x0, *y0), app.to_screen(*x1, *y1), 3.5, Color32::WHITE);
            }
            for p in &pts {
                painter.circle_filled(*p, 3.5, Color32::WHITE);
                painter.circle_stroke(*p, 3.5, stroke(accent, 1.0));
            }
            if let Some([x0, y0, x1, y1]) = ov.rubber {
                let (a, b) = (app.to_screen(x0, y0), app.to_screen(x1, y1));
                // 单击将生成的线段：沿上一个锚点的出方向手柄（如果有）。
                match ov.rubber_handle {
                    Some([hx, hy]) => {
                        let h = app.to_screen(hx, hy);
                        painter.add_shape(vec![a, h, b, b], false, Color32::TRANSPARENT, stroke(accent, 1.0));
                    }
                    None => {
                        painter.line_segment([a, b], stroke(accent, 1.0));
                    }
                }
            }
        }
    } else if let Some([x0, y0, x1, y1]) = ov.rubber {
        // 正在画的连接线，或拖动连接线端点：从起点到指针。
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent, 1.5));
    }

    // ── 橡皮擦笔刷（矢量编辑模式）。
    if let Some([x, y, r]) = ov.brush {
        let c = app.to_screen(x, y);
        let rs = (r * zoom) as f32;
        painter.circle_stroke(c, rs, stroke(Color32::WHITE, 3.0));
        painter.circle_stroke(c, rs, stroke(accent, 1.5));
    }

    // ── 套索（矢量编辑模式）。
    if ov.lasso.len() > 1 {
        let mut pts: Vec<Pos2> = ov.lasso.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if let Some(first) = pts.first().copied() {
            pts.push(first);
        }
        for sh in Shape::dashed_line(&pts, stroke(accent, 1.0), 4.0, 3.0) {
            painter.add(sh);
        }
    }

    // ── 文字编辑：光标与选中。
    if let Some(te) = ds.text_edit.clone()
        && let Some(text) = super::outline::TextOnScreen::of(&doc, te.id)
    {
        if let Some(q) = node_quad(app, &doc, te.id) {
            quad(painter, q, stroke(accent, 1.0));
        }
        let (a, b) = (te.caret.min(te.anchor), te.caret.max(te.anchor));
        if a != b {
            for q in text.selection(app, a, b) {
                painter.add(Shape::convex_polygon(q.to_vec(), accent.gamma_multiply(0.3), Stroke::NONE));
            }
        }
        painter.line_segment(text.caret(app, te.caret, zoom), stroke(t.text, 1.0));
    }

    // ── 智能参考线与测量标注（标签避开尺寸标签）。
    super::measure_overlay::draw(app, painter, t, size_label);

    // ── 标尺参考线（在标尺下面）。
    super::guides::draw(app, painter, &app.guides);

    // ── 指针：自绘字形（旋转、钢笔）画在最上面。
    if let (Some(super::cursors::Shown::Drawn(g)), Some(at)) = (app.cursor, app.pointer) {
        super::cursors::paint(painter, at, g);
    }
    labels
}

/// 选中椭圆的圆弧手柄（悬停/拖动中的那个略大，Start 中间带一个小点），
/// 拖动时在旁边显示读数 —— 照搬 `ui-egui/src/canvas/overlays.rs` 的 `arc_overlay`。
fn arc_overlay(app: &CanvasApp, painter: &mut Painter, col: Color32) {
    use figcraft_engine::arc_handles::ArcHandle;
    let Some(hs) = app.session.arc_handles() else { return };
    let ov = &app.session.tool.overlay;
    for h in [ArcHandle::Ratio, ArcHandle::Start, ArcHandle::Sweep] {
        let Some([x, y]) = hs.at(h) else { continue };
        let c = app.to_screen(x, y);
        let r = if ov.arc_hover == Some(h) { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
        if h == ArcHandle::Start {
            painter.circle_filled(c, 1.5, col);
        }
    }
    if let Some(hint) = &ov.arc {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, hint.label.clone(), col);
    }
}

/// 选中形状的圆角手柄与星形的比例手柄（悬停/拖动中的那个略大），拖动时在旁边显示数值。
///
/// 照搬 `ui-egui/src/canvas/overlays.rs` 的 `shape_overlay`：几何由引擎
/// `session.shape_handles()` 给（页面坐标），这里只负责白底彩边的圆点和读数 pill。
fn shape_overlay(app: &CanvasApp, painter: &mut Painter, col: Color32) {
    use figcraft_engine::shape_handles::ShapeHandle;
    let ov = &app.session.tool.overlay;
    for (h, [x, y]) in app.session.shape_handles() {
        let r = if ov.shape_hover == Some(h) { 4.5 } else { 3.5 };
        let c = app.to_screen(x, y);
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
    }
    if let Some(hint) = &ov.shape {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        let text = match hint.handle {
            ShapeHandle::Radius { .. } => format!("{} {}", figcraft_i18n::tr("Radius"), hint.value),
            ShapeHandle::Ratio => format!("{} {}%", figcraft_i18n::tr("Ratio"), hint.value),
        };
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, text, col);
    }
}

/// 贝塞尔控制点：小方块。
fn bezier_handle(painter: &mut Painter, a: Pos2, b: Pos2, size: f32, col: Color32) {
    painter.line_segment([a, b], stroke(col.gamma_multiply(0.6), 1.0));
    let r = Rect::from_center_size(b, vec2(size, size));
    painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
    painter.rect_stroke(r, CornerRadius::same(1), stroke(Color32::from_gray(0), 1.0), StrokeKind::Inside);
}

/// 变换框（屏幕四角 tl、tr、br、bl）：`outline` 时画轮廓，前 `handles` 个角手柄，
/// 显示尺寸时在下方画尺寸 `label`（返回尺寸 pill 的矩形）。
fn transform_box(
    app: &CanvasApp,
    painter: &mut Painter,
    q: [Pos2; 4],
    col: Color32,
    outline: bool,
    handles: usize,
    label: Option<String>,
) -> Option<Rect> {
    if outline {
        // 变换框也按设备像素吸附（见 `outline::crisp_rect`）：直接描 1px 路径时四条边
        // 落点不同，会一条粗一条细。
        let crisp = super::outline::axis_aligned(q).is_some_and(|r| super::outline::crisp_rect(painter, r, 1.0, app.ppp, col));
        if !crisp {
            quad(painter, q, stroke(col, 1.0));
        }
    }
    for c in q.iter().take(handles) {
        let r = Rect::from_center_size(*c, vec2(8.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(r, CornerRadius::same(1), stroke(col, 1.0), StrokeKind::Inside);
    }
    if let Some(label) = label.filter(|_| app.session.prefs.show_dimensions) {
        let bottom = q.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let cx = q.iter().map(|p| p.x).sum::<f32>() / 4.0;
        return Some(pill(painter, pos2(cx, bottom + 9.0), Align2::CENTER_TOP, label, col));
    }
    None
}

/// 画布读数：`col` 底色的 pill 上白字（返回 pill 区域）。
pub(super) fn pill(painter: &mut Painter, at: Pos2, align: Align2, text: String, col: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text, semibold(11.0), Color32::WHITE);
    let r = align.anchor_size(at, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), under_white(col));
    painter.galley(r.min + vec2(4.0, 2.0), galley, Color32::WHITE);
    r
}

/// 白字下面的底色需要压暗才够对比（对应 `theme::under_white`）。
fn under_white(col: Color32) -> Color32 {
    let d = |v: u8| (f32::from(v) * 0.55) as u8;
    Color32::from_rgb(d(col.0[0]), d(col.0[1]), d(col.0[2]))
}

/// 一条闭合四边形轮廓（供其它模块用）。
pub(super) fn _quad(painter: &mut Painter, q: [Pos2; 4], st: Stroke) {
    quad(painter, q, st);
}
