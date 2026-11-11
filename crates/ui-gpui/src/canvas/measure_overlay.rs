//! 画布上的智能参考线与测量标注：
//! 红色（`guide`）实线/虚线、吸附点上的十字、被量层的框，以及互不重叠、各缩放级别等大的距离标签。

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::theme::Tokens;

/// 标签与它所标的线（或另一个标签）之间的屏幕像素间距。
const GAP: f32 = 4.0;
/// 标签 pill 的内边距。
const PAD: Vec2 = vec2(4.0, 2.0);

/// 画工具叠加层的参考线、测量标签与被量的框；标签避开 `avoid`（选中框的尺寸标签）与其他标签。
pub(super) fn draw(app: &CanvasApp, painter: &mut Painter, t: &Tokens, avoid: Option<Rect>) {
    let ov = &app.session.tool.overlay;
    let line = Stroke::new(1.0, t.guide);
    if let Some([x0, y0, x1, y1]) = ov.measure_box {
        painter.rect_stroke(Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1)), CornerRadius::ZERO, line, StrokeKind::Middle);
    }
    let mut labels: Vec<(Pos2, Pos2, String)> = Vec::new();
    // egui：`ov.guides.iter().chain(crate::dev_mode::saved_lines(app))`。
    // 只有 Inspect 标签（Dev Mode）才把留在画布上的测量也算进来，且与 hover 的距离标签
    // 一起走同一套「互不重叠」排版。
    let saved = super::dev::saved_lines(app);
    for g in ov.guides.iter().chain(saved.iter()) {
        let (a, b) = (app.to_screen(g.x0, g.y0), app.to_screen(g.x1, g.y1));
        if g.x0 == g.x1 && g.y0 == g.y1 {
            for d in [vec2(4.0, 4.0), vec2(4.0, -4.0)] {
                painter.line_segment([a - d, a + d], Stroke::new(1.5, t.guide));
            }
        } else if g.dashed {
            for sh in Shape::dashed_line(&[a, b], line, 3.0, 2.0) {
                painter.add(sh);
            }
        } else {
            painter.line_segment([a, b], line);
        }
        if let Some(l) = &g.label {
            labels.push((a, b, l.clone()));
        }
    }
    let galleys: Vec<_> = labels.iter().map(|(_, _, l)| painter.layout_no_wrap(l.clone(), semibold(10.0), Color32::WHITE)).collect();
    let lines: Vec<(Pos2, Pos2, Vec2)> = labels.iter().zip(&galleys).map(|((a, b, _), g)| (*a, *b, g.size() + PAD * 2.0)).collect();
    for (r, g) in place_labels(&lines, avoid.as_slice()).into_iter().zip(galleys) {
        painter.rect_filled(r, CornerRadius::same(3), t.guide);
        painter.galley(r.min + PAD, g, Color32::WHITE);
    }
}

/// 线 `(a, b, 标签尺寸)` 的标签框（屏幕坐标）：线够长时居中在线，否则沿线上滑，
/// 再退到线旁（横线下方、竖线右侧，之后逐圈向外），直到不碰 `avoid` 与之前放好的标签。
pub(super) fn place_labels(lines: &[(Pos2, Pos2, Vec2)], avoid: &[Rect]) -> Vec<Rect> {
    let mut placed: Vec<Rect> = avoid.to_vec();
    for (a, b, size) in lines {
        let mid = pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let d = *b - *a;
        let horizontal = d.x.abs() >= d.y.abs();
        // 沿线方向与向外方向：单位向量，以及一个标签的步长。
        let (dir, out) = if horizontal { (vec2(1.0, 0.0), vec2(0.0, 1.0)) } else { (vec2(0.0, 1.0), vec2(1.0, 0.0)) };
        let (along, across) = if horizontal { (size.x, size.y) } else { (size.y, size.x) };
        let room = (d.length() - along) / 2.0 - GAP;
        let mut centres: Vec<Pos2> = Vec::new();
        if room >= 0.0 {
            centres.push(mid);
            // 沿线上来回滑动，前提是仍然落在线上。
            let mut n = 1.0;
            while n * (along + GAP) <= room && centres.len() < 32 {
                centres.extend([mid + dir * n * (along + GAP), mid - dir * n * (along + GAP)]);
                n += 1.0;
            }
        }
        for n in 0..32 {
            let off = across / 2.0 + GAP + n as f32 * (across + GAP);
            centres.extend([mid + out * off, mid - out * off]);
        }
        let free = |r: &Rect| !placed.iter().any(|p| p.expand(GAP / 2.0).intersects(*r));
        // 所有候选都被占（一处挤了几十个标签）：叠到已放置内容的下面。
        let r = centres.into_iter().map(|c| Rect::from_center_size(c, *size)).find(free).unwrap_or_else(|| {
            let bottom = placed.iter().map(|p| p.max.y).fold(mid.y, f32::max);
            Rect::from_min_size(pos2(mid.x - size.x / 2.0, bottom + GAP), *size)
        });
        placed.push(r);
    }
    placed.split_off(avoid.len())
}

#[cfg(test)]
mod tests {
    use super::place_labels;
    use crate::canvas::geom::{Pos2, Rect, Vec2, pos2, vec2};

    const SIZE: Vec2 = vec2(24.0, 14.0);

    fn no_overlaps(r: &[Rect]) -> bool {
        r.iter().enumerate().all(|(i, a)| r.iter().skip(i + 1).all(|b| !a.intersects(*b)))
    }

    #[test]
    fn long_lines_carry_their_label_in_the_middle() {
        let r = place_labels(&[(pos2(0.0, 50.0), pos2(200.0, 50.0), SIZE), (pos2(50.0, 0.0), pos2(50.0, 300.0), SIZE)], &[]);
        assert_eq!(r[0].center(), pos2(100.0, 50.0));
        assert_eq!(r[1].center(), pos2(50.0, 150.0));
    }

    #[test]
    fn short_lines_put_the_label_beside_them() {
        let r = place_labels(&[(pos2(0.0, 50.0), pos2(10.0, 50.0), SIZE), (pos2(50.0, 0.0), pos2(50.0, 6.0), SIZE)], &[]);
        assert!(r[0].min.y > 50.0, "below a short horizontal line: {:?}", r[0]);
        assert!(r[1].min.x > 50.0, "right of a short vertical line: {:?}", r[1]);
    }

    #[test]
    fn labels_never_overlap_even_when_crowded() {
        // 缩得很远：所有线都挤到一处。
        let crowd: Vec<(Pos2, Pos2, Vec2)> =
            (0..100).map(|i| (pos2(10.0, 10.0), pos2(10.0 + (i % 2) as f32 * 0.5, 10.0 + (i % 3) as f32 * 0.5), SIZE)).collect();
        assert!(no_overlaps(&place_labels(&crowd, &[])));
        // 相隔几像素的平行线。
        let close: Vec<(Pos2, Pos2, Vec2)> = (0..6).map(|i| (pos2(0.0, i as f32 * 3.0), pos2(100.0, i as f32 * 3.0), SIZE)).collect();
        let r = place_labels(&close, &[]);
        assert!(no_overlaps(&r));
        assert_eq!(r[0].center(), pos2(50.0, 0.0), "the first keeps its place");
    }

    #[test]
    fn labels_keep_clear_of_the_size_label() {
        let size = Rect::from_center_size(pos2(50.0, 100.0), vec2(60.0, 16.0));
        // 长线把标签沿自己滑开，绕过尺寸标签。
        let r = place_labels(&[(pos2(50.0, 80.0), pos2(50.0, 200.0), SIZE)], &[size]);
        assert_eq!(r.len(), 1);
        assert!(!r[0].intersects(size) && r[0].center().x == 50.0, "{:?}", r[0]);
        // 短线挪到旁边。
        let r = place_labels(&[(pos2(50.0, 90.0), pos2(50.0, 110.0), SIZE)], &[size]);
        assert!(!r[0].intersects(size), "{:?}", r[0]);
    }
}
