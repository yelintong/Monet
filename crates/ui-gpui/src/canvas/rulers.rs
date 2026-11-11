//! 标尺与帧状态徽标（照搬 `ui-egui/src/canvas/rulers.rs`）。
//!
//! 画法、刻度密度（`figcraft_engine::guides::ruler_step`）、选区跨度高亮、
//! 数字格式（`fmt`）全部与原实现一致。

use figcraft_doc::{Document, Node, NodeId, NodeKind};

use super::geom::*;
use super::painter::{Galley, Label, Painter, text_size};
use crate::app::CanvasApp;
use crate::theme::{DEV_CHANGED, DEV_COMPLETED, DEV_READY, Tokens, stroke};

/// 会得到画布标签与布局网格的图层，按 z 序：页面可见的顶层图层，
/// 以及「内容显示」的 section 内部的可见子层（section 可以嵌套）。
/// section 排在其子层之后，于是它的标题 pill 会盖住（并接住点击）靠近其左上角的帧标签。
pub(super) fn boards(doc: &Document, page: NodeId) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<NodeId>) {
        let Some(n) = doc.node(id) else { return };
        for c in &n.children {
            let Some(cn) = doc.node(*c) else { continue };
            if !cn.visible {
                continue;
            }
            if matches!(cn.kind, NodeKind::Section { contents_hidden: false }) && depth < 32 {
                walk(doc, *c, depth + 1, out);
            }
            out.push(*c);
        }
    }
    let mut out = Vec::new();
    walk(doc, page, 0, &mut out);
    out
}

/// 帧/section 的交付状态与其徽标颜色。
pub(crate) fn status_label(n: &Node) -> Option<(&'static str, Color32)> {
    match n.dev_status {
        figcraft_doc::DevStatus::None => None,
        // 原文走 `tr(...)`：徽标文字要跟着界面语言走，不能写死英文。
        figcraft_doc::DevStatus::ReadyForDev if n.dev_changed => Some((figcraft_i18n::tr("Changed"), DEV_CHANGED)),
        figcraft_doc::DevStatus::ReadyForDev => Some((figcraft_i18n::tr("Ready for dev"), DEV_READY)),
        figcraft_doc::DevStatus::Completed => Some((figcraft_i18n::tr("Completed"), DEV_COMPLETED)),
    }
}

/// 帧/section 的交付状态，做成以 `at` 左侧居中为基准的小 pill；返回 pill 区域。
pub(crate) fn status_badge(painter: &mut Painter, n: &Node, at: Pos2) -> Option<Rect> {
    let (label, fill) = status_label(n)?;
    let galley = painter.layout_no_wrap(label, semibold(10.0), Color32::WHITE);
    let size = galley.size() + vec2(10.0, 4.0);
    let r = Rect::from_min_size(pos2(at.x, at.y - galley.size().y / 2.0 - 2.0), size);
    painter.rect_filled(r, CornerRadius::same(4), fill);
    painter.galley(r.min + vec2(5.0, 2.0), galley, Color32::WHITE);
    Some(r)
}

/// 侧标尺上的一个数字（lizard `vertical_ruler_label`）。
///
/// lizard 用 `TextShape::with_angle_and_anchor(-π/2, CENTER_CENTER)`：先把**整段**文字按水平
/// 排版放在中心点四周，再绕中心转 -90°。GPUI 0.2.2 的文字管线把变换矩阵写死成 unit，
/// 但 `Svg::with_transformation` 可以旋转 SVG 绘图（kit 的 `Icon::rotate` 同一条路），
/// 所以这里记录**一个**带 `angle=-π/2` 的 Label，由 `lib.rs` 换成内联 SVG 文字后旋转。
/// 碰撞盒按「旋转后」计算：宽 = 行高，高 = 原文本宽，与 lizard 一致。
#[derive(Clone)]
struct VLabel {
    center: Pos2,
    text: String,
    font: FontId,
    color: Color32,
}

impl VLabel {
    fn new(center: Pos2, text: String, font: FontId, color: Color32) -> Self {
        Self { center, text, font, color }
    }

    /// 旋转前水平文字框：宽 = 文本宽，高 = 行高；绕自身中心旋转。
    fn unrotated(&self) -> Rect {
        Rect::from_center_size(self.center, text_size(&self.text, self.font))
    }

    /// 旋转后文本的视觉包围盒：宽 = 行高，高 = 原文本宽。
    fn bounds(&self) -> Rect {
        let w = text_size(&self.text, self.font).x;
        let h = text_size("0", self.font).y;
        Rect::from_center_size(self.center, vec2(h, w))
    }

    fn paint(&self, painter: &mut Painter) {
        painter.labels.push(Label {
            rect: self.unrotated(),
            text: self.text.clone(),
            font: self.font,
            color: self.color,
            align: Align2::CENTER_CENTER,
            angle: Some(-std::f32::consts::FRAC_PI_2),
        });
    }
}

/// 让带 4~5 位数字的刻度标签之间留够距离（lizard `ruler_major_step`）。
fn ruler_major_step(zoom: f64) -> f64 {
    let mut step = figcraft_engine::guides::ruler_step(zoom);
    while step * zoom < 85.0 {
        let base = 10.0_f64.powf(step.log10().floor());
        let part = step / base;
        step = if part < 1.5 {
            2.0 * base
        } else if part < 3.5 {
            5.0 * base
        } else {
            10.0 * base
        };
    }
    step
}

/// 标尺（lizard `canvas.rs::rulers` 1:1）：主/次刻度、自适应标签间距、选区跨度两端坐标、
/// 标签避让。egui 用 `with_clip_rect` 把上/左标尺各自裁开；GPUI 的记录式画笔没有子画笔，
/// 改为在写 op / label 之前做同样的边界判断（视觉等价，也不会污染全局 clip）。
pub(super) fn rulers(app: &CanvasApp, painter: &mut Painter, t: &Tokens, doc: &Document, sel: &[NodeId]) {
    let r = app.canvas.rect;
    let size = 20.0;
    painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), size)), CornerRadius::ZERO, t.panel);
    painter.rect_filled(Rect::from_min_size(r.min, vec2(size, r.height())), CornerRadius::ZERO, t.panel);
    let top = Rect::from_min_max(pos2(r.min.x + size, r.min.y), pos2(r.max.x, r.min.y + size));
    let side = Rect::from_min_max(pos2(r.min.x, r.min.y + size), pos2(r.min.x + size, r.max.y));
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6);
    let major_step = ruler_major_step(zoom);
    let minor_step = major_step / 5.0;
    let p0 = app.to_page(r.min);
    let selected = doc.bounds_of(sel).map(|b| {
        let a = app.to_screen(b.x0, b.y0);
        let bb = app.to_screen(b.x1, b.y1);
        ((b.x0, b.y0), (b.x1, b.y1), a, bb)
    });
    let selection_color = app.accent();
    let selected_label_color = selection_color.gamma_multiply(0.9);
    let number_color = t.text_secondary.gamma_multiply(0.82);
    let muted_number_color = t.text_tertiary.gamma_multiply(0.85);
    let tick_color = t.text_tertiary.gamma_multiply(0.8);
    let mut top_edges: Vec<(Rect, Galley)> = Vec::new();
    let mut side_edges: Vec<VLabel> = Vec::new();

    if let Some(((x0, y0), (x1, y1), a, b)) = selected {
        let fill = selection_color.gamma_multiply(0.15);
        let top_fill = Rect::from_min_max(pos2(a.x, top.top()), pos2(b.x, top.bottom())).intersect(top);
        let side_fill = Rect::from_min_max(pos2(side.left(), a.y), pos2(side.right(), b.y)).intersect(side);
        if top_fill.is_positive() {
            painter.rect_filled(top_fill, CornerRadius::ZERO, fill);
        }
        if side_fill.is_positive() {
            painter.rect_filled(side_fill, CornerRadius::ZERO, fill);
        }
        // 上标尺：跨度两端写坐标，并在刻度带上画一条 7 pt 的强调刻度。
        for (value, at, before) in [(x0, a.x, true), (x1, b.x, false)] {
            let galley = painter.layout_no_wrap(fmt(value), regular(9.0), selected_label_color);
            let x = if before { at - 5.0 - galley.size().x } else { at + 5.0 };
            let rect = Rect::from_min_size(pos2(x, top.top() + 2.0), galley.size());
            top_edges.push((rect, galley));
            if at >= top.left() && at <= top.right() {
                painter.line_segment([pos2(at, top.bottom() - 7.0), pos2(at, top.bottom())], stroke(selected_label_color, 1.0));
            }
        }
        // 左标尺：同样的两端坐标，文字竖排。
        for (value, at, before) in [(y0, a.y, true), (y1, b.y, false)] {
            let text = fmt(value);
            let length = text_size(&text, regular(9.0)).x;
            let center_y = if before { at - 5.0 - length / 2.0 } else { at + 5.0 + length / 2.0 };
            side_edges.push(VLabel::new(pos2(side.left() + 7.5, center_y), text, regular(9.0), selected_label_color));
            if at >= side.top() && at <= side.bottom() {
                painter.line_segment([pos2(side.right() - 7.0, at), pos2(side.right(), at)], stroke(selected_label_color, 1.0));
            }
        }
    }

    // ── 上标尺：次刻度 3 pt / 主刻度 6 pt，只有主刻度写数字 ──────────────────
    let horizontal_ticks = ((f64::from(top.width()) / (minor_step * zoom)).ceil() as usize + 2).min(10_000);
    let first_x = (p0.x / minor_step).floor() as i64;
    for offset in 0..horizontal_ticks {
        let index = first_x.saturating_add(offset as i64);
        let value = index as f64 * minor_step;
        let x = app.to_screen(value, 0.0).x;
        if x > top.right() + 1.0 {
            break;
        }
        let major = index.rem_euclid(5) == 0;
        let tick = if major { 6.0 } else { 3.0 };
        if x >= top.left() && x <= top.right() {
            painter.line_segment([pos2(x, top.bottom() - tick), pos2(x, top.bottom())], stroke(tick_color, 1.0));
        }
        if major {
            let color = if selected.is_some_and(|(_, _, a, b)| (a.x..b.x).contains(&x)) { muted_number_color } else { number_color };
            let galley = painter.layout_no_wrap(fmt(value), regular(9.0), color);
            let rect = Rect::from_min_size(pos2(x - galley.size().x / 2.0, top.top() + 2.0), galley.size());
            if rect.left() >= top.left() + 2.0 && rect.right() <= top.right() - 2.0 && !top_edges.iter().any(|(edge, _)| edge.expand(3.0).intersects(rect)) {
                painter.galley(rect.min, galley, color);
            }
        }
    }

    // ── 左标尺 ───────────────────────────────────────────────────────────────
    let vertical_ticks = ((f64::from(side.height()) / (minor_step * zoom)).ceil() as usize + 2).min(10_000);
    let first_y = (p0.y / minor_step).floor() as i64;
    for offset in 0..vertical_ticks {
        let index = first_y.saturating_add(offset as i64);
        let value = index as f64 * minor_step;
        let y = app.to_screen(0.0, value).y;
        if y > side.bottom() + 1.0 {
            break;
        }
        let major = index.rem_euclid(5) == 0;
        let tick = if major { 6.0 } else { 3.0 };
        if y >= side.top() && y <= side.bottom() {
            painter.line_segment([pos2(side.right() - tick, y), pos2(side.right(), y)], stroke(tick_color, 1.0));
        }
        if major {
            let color = if selected.is_some_and(|(_, _, a, b)| (a.y..b.y).contains(&y)) { muted_number_color } else { number_color };
            let label = VLabel::new(pos2(side.left() + 7.5, y), fmt(value), regular(9.0), color);
            let bounds = label.bounds();
            if bounds.top() >= side.top() + 2.0
                && bounds.bottom() <= side.bottom() - 2.0
                && !side_edges.iter().any(|edge| edge.bounds().expand(3.0).intersects(bounds))
            {
                label.paint(painter);
            }
        }
    }

    // 选区两端的坐标最后画（盖在刻度数字之上，与 egui 的顺序一致）。
    for (rect, galley) in top_edges {
        if rect.left() >= top.left() && rect.right() <= top.right() {
            painter.galley(rect.min, galley, selected_label_color);
        }
    }
    for label in side_edges {
        let bounds = label.bounds();
        if bounds.top() >= side.top() && bounds.bottom() <= side.bottom() {
            label.paint(painter);
        }
    }

    painter.line_segment([pos2(r.min.x, r.min.y + size), pos2(r.max.x, r.min.y + size)], stroke(t.border, 1.0));
    painter.line_segment([pos2(r.min.x + size, r.min.y), pos2(r.min.x + size, r.max.y)], stroke(t.border, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// lizard `ruler_major_step`：标签间距至少 85 pt，5 的倍数是主刻度。
    #[test]
    fn major_step_keeps_labels_at_least_85_points_apart() {
        for zoom in [0.1_f64, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
            let step = ruler_major_step(zoom);
            assert!(step > 0.0 && step.is_finite(), "zoom {zoom} produced {step}");
            assert!(step * zoom >= 85.0, "zoom {zoom}: {step} px labels are too dense");
            assert!((step * zoom / 5.0) > 0.0);
        }
    }

    /// 旋转标签：碰撞盒是旋转后尺寸，实际绘制仍是「一整段文字绕中心 -90°」。
    #[test]
    fn vertical_label_is_one_rotated_galley_not_stacked_digits() {
        let font = regular(9.0);
        let label = VLabel::new(pos2(10.0, 100.0), "1000".into(), font, Color32::WHITE);
        let b = label.bounds();
        assert_eq!(b.width(), text_size("0", font).y);
        assert_eq!(b.height(), text_size("1000", font).x);
        let u = label.unrotated();
        assert_eq!(u.width(), text_size("1000", font).x);
        assert_eq!(u.height(), text_size("0", font).y);
        assert_eq!(u.center(), b.center());
        let mut p = Painter::default();
        label.paint(&mut p);
        assert_eq!(p.labels.len(), 1, "the whole number rotates as one piece");
        assert_eq!(p.labels[0].text, "1000");
        assert_eq!(p.labels[0].angle, Some(-std::f32::consts::FRAC_PI_2));
        assert_eq!(p.labels[0].align, Align2::CENTER_CENTER);
    }
}

/// 标尺数字格式（与原文一致：两位小数四舍五入，整数显示整数）。
pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}
