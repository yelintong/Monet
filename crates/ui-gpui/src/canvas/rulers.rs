//! 标尺与帧状态徽标（照搬 `ui-egui/src/canvas/rulers.rs`）。
//!
//! 画法、刻度密度（`figcraft_engine::guides::ruler_step`）、选区跨度高亮、
//! 数字格式（`fmt`）全部与原实现一致。

use figcraft_doc::{Document, Node, NodeId, NodeKind};

use super::geom::*;
use super::painter::Painter;
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

pub(super) fn rulers(app: &CanvasApp, painter: &mut Painter, t: &Tokens, doc: &Document, sel: &[NodeId]) {
    let r = app.canvas.rect;
    let size = 20.0;
    painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), size)), CornerRadius::ZERO, t.panel);
    painter.rect_filled(Rect::from_min_size(r.min, vec2(size, r.height())), CornerRadius::ZERO, t.panel);
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let step = figcraft_engine::guides::ruler_step(zoom);
    let p0 = app.to_page(r.min);
    let p1 = app.to_page(r.max);
    let accent = app.accent();
    // 高亮选区的跨度。
    if let Some(b) = doc.bounds_of(sel) {
        let (a, bb) = (app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
        painter.rect_filled(Rect::from_min_max(pos2(a.x, r.min.y), pos2(bb.x, r.min.y + size)), CornerRadius::ZERO, accent.gamma_multiply(0.15));
        painter.rect_filled(Rect::from_min_max(pos2(r.min.x, a.y), pos2(r.min.x + size, bb.y)), CornerRadius::ZERO, accent.gamma_multiply(0.15));
    }
    let mut x = (p0.x / step).floor() * step;
    while x <= p1.x {
        let s = app.to_screen(x, 0.0).x;
        painter.line_segment([pos2(s, r.min.y + size - 6.0), pos2(s, r.min.y + size)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(s + 2.0, r.min.y + 3.0), Align2::LEFT_TOP, fmt(x), regular(9.0), t.text_secondary);
        x += step;
    }
    let mut y = (p0.y / step).floor() * step;
    while y <= p1.y {
        let s = app.to_screen(0.0, y).y;
        painter.line_segment([pos2(r.min.x + size - 6.0, s), pos2(r.min.x + size, s)], stroke(t.text_tertiary, 1.0));
        painter.text(pos2(r.min.x + 3.0, s + 2.0), Align2::LEFT_TOP, fmt(y), regular(9.0), t.text_secondary);
        y += step;
    }
    painter.line_segment([pos2(r.min.x, r.min.y + size), pos2(r.max.x, r.min.y + size)], stroke(t.border, 1.0));
    painter.line_segment([pos2(r.min.x + size, r.min.y), pos2(r.min.x + size, r.max.y)], stroke(t.border, 1.0));
}

/// 标尺数字格式（与原文一致：两位小数四舍五入，整数显示整数）。
pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}
