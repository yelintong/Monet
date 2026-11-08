//! 顶层帧与 section 的画布名称标签（照搬 `ui-egui/src/canvas/frame_labels.rs`）。
//!
//! 帧名贴在上边缘之上，section 名是左上角内部的 pill；组件/图标用各自的身份色。
//! `label_edge` 的几何计算与原实现**逐行一致**（含镜像帧保持正向）。
//!
//! 差异（GPUI 侧适配）：文字用 GPUI 文本元素渲染，不能像 egui 那样把已画出的图元整体旋转，
//! 因此旋转帧的标签仍放在几何算出的角点，但保持水平（记为已知限制）。

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Point;

use super::geom::*;
use super::painter::Painter;
use super::rulers::status_badge;
use crate::app::CanvasApp;
use crate::theme::{COMPONENT, Tokens};

/// 文字身份色（图标层用橙色；这里先留接口）。
pub const ICON: Color32 = Color32::from_rgb(0xFF, 0xB3, 0x4D);

/// 标签贴在屏幕上的位置：起点角、读取方向（弧度，屏幕顺时针）、该方向的边长。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelEdge {
    pub corner: Pos2,
    pub angle: f32,
    pub len: f32,
}

/// 本地角 (0,0)、(w,0)、(0,h) 显示在 `p00`、`p10`、`p01` 时，帧的上边缘作为标签边。
pub fn label_edge(p00: Pos2, p10: Pos2, p01: Pos2) -> LabelEdge {
    let (ex, ey) = (p10 - p00, p01 - p00);
    let len = ex.length();
    if !len.is_finite() || len <= 1e-3 {
        return LabelEdge { corner: p00, angle: 0.0, len: 0.0 };
    }
    let mirrored = ex.x * ey.y - ex.y * ey.x < 0.0;
    let reversed = mirrored && ex.x < 0.0;
    let d = if reversed { -ex } else { ex };
    let up = vec2(d.y, -d.x);
    let bottom = up.dot(ey) > 0.0;
    let corner = p00 + if reversed { ex } else { Vec2::ZERO } + if bottom { ey } else { Vec2::ZERO };
    LabelEdge { corner, angle: d.y.atan2(d.x), len }
}

/// 画 `id`（顶层帧或 section）的标签；返回它在屏幕上的四角（用于命中测试）。
pub fn draw(app: &CanvasApp, painter: &mut Painter, t: &Tokens, doc: &Document, id: NodeId, selected: bool, hovered: bool) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let at = |x: f64, y: f64| {
        let p = w * Point::new(x, y);
        app.to_screen(p.x, p.y)
    };
    let (p00, p01) = (at(0.0, 0.0), at(0.0, n.height));
    let edge = label_edge(p00, at(n.width, 0.0), p01);
    let tl = edge.corner;
    let mark = match n.kind {
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => Some(COMPONENT),
        NodeKind::Icon { .. } => Some(ICON),
        _ => None,
    };
    let col = if let Some(m) = mark {
        m
    } else if selected || hovered {
        app.accent()
    } else {
        t.text_secondary
    };

    let area = if matches!(n.kind, NodeKind::Section { .. }) {
        // section 标题是左上角内部的 pill。
        let hidden = n.hides_contents();
        let fg = if selected { Color32::WHITE } else { t.text };
        let galley = painter.layout_no_wrap(n.name.clone(), semibold(11.0), fg);
        let r = Rect::from_min_size(tl, galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(4), if selected { t.accent } else { t.hover });
        painter.galley(r.min + vec2(8.0, 4.0), galley, fg);
        status_badge(painter, n, pos2(r.max.x + 6.0, r.center().y));
        let _ = hidden;
        r
    } else {
        let x = tl.x;
        // 名字画在帧上边缘之上。
        let galley = painter.layout_no_wrap(n.name.clone(), regular(11.0), col);
        let size = galley.size();
        let end = x + size.x;
        painter.galley(pos2(x, tl.y - 16.0), galley, col);
        status_badge(painter, n, pos2(end + 6.0, tl.y - 9.0));
        Rect::from_min_max(pos2(tl.x, tl.y - 18.0), pos2(end, tl.y - 16.0 + size.y.max(12.0)))
    };
    if n.hides_contents() {
        let height = (p01 - p00).length();
        if edge.len > 120.0 && height > 60.0 {
            let center = at(n.width / 2.0, n.height / 2.0);
            painter.text(center, Align2::CENTER_CENTER, "Contents hidden", regular(11.0), t.text_tertiary);
        }
    }
    Some([area.min, pos2(area.max.x, area.min.y), area.max, pos2(area.min.x, area.max.y)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    /// 与原文测试同款：标签边跟随旋转、镜像时保持正向。
    #[test]
    fn label_edges_follow_rotation_and_stay_upright_when_mirrored() {
        let e = label_edge(pos2(10.0, 20.0), pos2(110.0, 20.0), pos2(10.0, 70.0));
        assert!(close(e.corner, pos2(10.0, 20.0)) && e.angle == 0.0 && (e.len - 100.0).abs() < 1e-3);
        let e = label_edge(pos2(100.0, 0.0), pos2(100.0, 80.0), pos2(50.0, 0.0));
        assert!(close(e.corner, pos2(100.0, 0.0)));
        assert!((e.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let e = label_edge(pos2(100.0, 50.0), pos2(0.0, 50.0), pos2(100.0, 0.0));
        assert!(close(e.corner, pos2(100.0, 50.0)));
        assert!((e.angle.abs() - std::f32::consts::PI).abs() < 1e-5);
        let e = label_edge(pos2(100.0, 0.0), pos2(0.0, 0.0), pos2(100.0, 50.0));
        assert!(close(e.corner, pos2(0.0, 0.0)) && e.angle.abs() < 1e-6, "{e:?}");
        let e = label_edge(pos2(0.0, 50.0), pos2(100.0, 50.0), pos2(0.0, 0.0));
        assert!(close(e.corner, pos2(0.0, 0.0)) && e.angle.abs() < 1e-6, "{e:?}");
        let e = label_edge(pos2(5.0, 5.0), pos2(5.0, 5.0), pos2(5.0, 9.0));
        assert_eq!(e, LabelEdge { corner: pos2(5.0, 5.0), angle: 0.0, len: 0.0 });
        let e = label_edge(pos2(5.0, 5.0), pos2(f32::NAN, 5.0), pos2(5.0, 9.0));
        assert_eq!(e.angle, 0.0);
    }
}
