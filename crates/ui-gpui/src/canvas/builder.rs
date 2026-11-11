//! 区域填充的斜线剖面（照搬 `ui-egui/src/canvas/builder.rs`）：矢量编辑里悬停/选中的区域、
//! 形状构建器里可合并/可删除的区域，都用斜线剖面标出来，线条带浅色晕边，压在任何填色上都看得清。

use figcraft_engine::shapebuilder::Arrangement;

use super::geom::*;
use super::outline::stroke;
use super::overlays::accent;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::theme::Tokens;

/// 剖线的宽度（屏幕像素）。
const REGION_HATCH_WIDTH: f32 = 1.4;

/// 矢量编辑：选中与悬停的区域画剖面（在段与点下面；引擎负责排版与裁剪）。
pub(super) fn region_hatch(app: &CanvasApp, painter: &mut Painter) {
    let Ok(ds) = app.session.ds() else { return };
    let (ox, oy) = ds.view.page_to_screen(0.0, 0.0);
    let (z, min) = (ds.view.zoom, app.canvas.rect.min);
    let view = figcraft_geom::Affine::new([z, 0.0, 0.0, z, f64::from(min.x) + ox, f64::from(min.y) + oy]);
    let r = painter.clip;
    let clip = figcraft_geom::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y));
    let col = accent(app);
    for [a, b] in app.session.vector_region_hatch(view, Some(clip)) {
        painter.line_segment([pos2(a.x as f32, a.y as f32), pos2(b.x as f32, b.y as f32)], stroke(col, REGION_HATCH_WIDTH));
    }
}

/// 形状构建器：每个区域的边，悬停/已收集的区域画剖面（松开要删掉时用危险色）。
pub(super) fn builder_overlay(app: &CanvasApp, painter: &mut Painter, t: &Tokens, arr: &Arrangement) {
    let ov = &app.session.tool.overlay;
    let accent = accent(app);
    let screen = |r: &figcraft_engine::shapebuilder::Region| -> Vec<Vec<Pos2>> {
        r.polys.iter().filter_map(|poly| clean_poly(poly.iter().map(|p| app.to_screen(p.x, p.y)))).collect()
    };
    // 边要在任何填色上都看得清：细亮线下垫一层浅色晕。
    for r in &arr.regions {
        for poly in screen(r) {
            for seg in segments(&poly) {
                painter.add(Shape::line(seg.to_vec(), stroke(Color32::from_white_alpha(150), 2.5)));
                painter.add(Shape::line(seg.to_vec(), stroke(accent, 1.0)));
            }
        }
    }
    let col = if ov.builder_remove { t.danger } else { accent };
    for r in ov.builder_regions.iter().filter_map(|i| arr.regions.get(*i)) {
        hatched(painter, &screen(r), col);
    }
}

/// 区域轮廓的屏幕坐标，画之前先清理：丢掉非有限点、重复点（末点回到首点、细缝折返的点）
/// 以及亚像素细缝（反正看不见）。
pub(crate) fn clean_poly(points: impl IntoIterator<Item = Pos2>) -> Option<Vec<Pos2>> {
    let mut out: Vec<Pos2> = Vec::new();
    for q in points {
        if !(q.x.is_finite() && q.y.is_finite()) || out.last().is_some_and(|l| l.distance(q) < 0.01) {
            continue;
        }
        out.push(q);
    }
    if out.len() > 1 && out.first().is_some_and(|f| f.distance(*out.last().unwrap_or(f)) < 0.01) {
        out.pop();
    }
    let n = out.len();
    let twice_area: f32 = (0..n).map(|k| out[k].x * out[(k + 1) % n].y - out[(k + 1) % n].x * out[k].y).sum();
    (n > 2 && twice_area.abs() > 0.4).then_some(out)
}

/// 闭合多边形拆成逐段。逐段画而不是画闭合路径：细缝折返处求斜接会除以零法线，
/// 会在画布上拉出一根尖刺。
fn segments(poly: &[Pos2]) -> impl Iterator<Item = [Pos2; 2]> + '_ {
    let n = poly.len();
    (0..n).map(move |k| [poly[k], poly[(k + 1) % n]])
}

/// 用 `col` 斜线剖面并描边的多边形（即将被填充、合并或删除的区域）。
pub(super) fn hatched(painter: &mut Painter, polys: &[Vec<Pos2>], col: Color32) {
    let halo = Color32::from_white_alpha(200);
    for l in hatch(polys, 6.0) {
        painter.line_segment(l, stroke(halo, 3.0));
        painter.line_segment(l, stroke(col, 1.25));
    }
    for poly in polys {
        for seg in segments(poly) {
            painter.add(Shape::line(seg.to_vec(), stroke(halo, 4.0)));
            painter.add(Shape::line(seg.to_vec(), stroke(col, 2.0)));
        }
    }
}

/// 多边形内间隔 `gap` 的 45° 斜线（奇偶规则），屏幕坐标。
fn hatch(polys: &[Vec<Pos2>], gap: f32) -> Vec<[Pos2; 2]> {
    // 先把斜线转成水平再扫描，最后转回去。
    let rot = |p: Pos2| pos2((p.x + p.y) * std::f32::consts::FRAC_1_SQRT_2, (p.y - p.x) * std::f32::consts::FRAC_1_SQRT_2);
    let back = |x: f32, y: f32| pos2((x - y) * std::f32::consts::FRAC_1_SQRT_2, (x + y) * std::f32::consts::FRAC_1_SQRT_2);
    let edges: Vec<(Pos2, Pos2)> = polys
        .iter()
        .flat_map(|poly| {
            let pts: Vec<Pos2> = poly.iter().map(|p| rot(*p)).collect();
            let n = pts.len();
            (0..n).filter_map(move |k| Some((*pts.get(k)?, *pts.get((k + 1) % n)?)))
        })
        .collect();
    let (y0, y1) = edges.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (a, b)| (lo.min(a.y).min(b.y), hi.max(a.y).max(b.y)));
    let mut out = Vec::new();
    if !(y0.is_finite() && y1.is_finite()) || (y1 - y0) / gap > 4000.0 {
        return out;
    }
    let mut y = (y0 / gap).ceil() * gap;
    while y < y1 {
        let mut xs: Vec<f32> = edges
            .iter()
            .filter(|(a, b)| (a.y <= y && y < b.y) || (b.y <= y && y < a.y))
            .map(|(a, b)| a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y))
            .collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            if let [a, b] = pair {
                out.push([back(*a, y), back(*b, y)]);
            }
        }
        y += gap;
    }
    out
}
