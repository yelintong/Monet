//! 图层轮廓（照搬 `ui-egui/src/canvas/outline.rs`）。
//!
//! hover / 选中 / 蒙版 / 布尔操作数的轮廓：椭圆、矢量、多边形、星形走**真实几何轮廓**
//! （`figcraft_render::node_outline` 展平后逐段画），其余走包围盒四边形。

use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Point;

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::theme::COMPONENT;

pub(super) fn stroke(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

/// 一个文字层摆在画布上的排版：光标与选中的字符画在哪里。
pub(crate) struct TextOnScreen {
    pub layout: Arc<figcraft_text::TextLayout>,
    /// 文字有多少个字符。
    pub chars: usize,
    /// 排版在框内的竖直偏移（竖直对齐）。
    dy: f64,
    world: figcraft_geom::Affine,
}

impl TextOnScreen {
    /// `doc` 里图层 `id` 的文字（None：不是文字层）。
    pub(crate) fn of(doc: &Document, id: NodeId) -> Option<Self> {
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let (layout, dy) = figcraft_text::layout_node(n)?;
        Some(Self { layout, chars: t.char_count(), dy, world: doc.world_transform(id) })
    }

    /// 排版点 `(x, y)` 在屏幕上的位置（沿路径排的文字也走这条路）。
    pub(crate) fn at(&self, app: &CanvasApp, x: f64, y: f64) -> Pos2 {
        let q = self.layout.warp_point(x, y);
        let p = self.world * Point::new(q.x, q.y + self.dy);
        app.to_screen(p.x, p.y)
    }

    /// 字符 `a..b` 的高亮：每行一个屏幕四边形。
    pub(crate) fn selection(&self, app: &CanvasApp, a: usize, b: usize) -> Vec<[Pos2; 4]> {
        let rects = self.layout.selection_rects(a, b);
        rects.iter().map(|r| [self.at(app, r.x0, r.y0), self.at(app, r.x1, r.y0), self.at(app, r.x1, r.y1), self.at(app, r.x0, r.y1)]).collect()
    }

    /// 第 `index` 个字符前的光标：屏幕上的上下两点。
    pub(crate) fn caret(&self, app: &CanvasApp, index: usize, zoom: f64) -> [Pos2; 2] {
        let (cx, li) = self.layout.caret(index);
        match self.layout.lines.get(li).or(self.layout.lines.last()) {
            Some(line) => [self.at(app, cx, line.top), self.at(app, cx, line.top + line.height)],
            None => {
                let p = self.at(app, 0.0, 0.0);
                [p, p + vec2(0.0, 14.0 * zoom as f32)]
            }
        }
    }
}

/// 节点在屏幕空间的定向轮廓（四角，跟随旋转/镜像）。
pub(crate) fn node_quad(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let corners = [Point::new(0.0, 0.0), Point::new(n.width, 0.0), Point::new(n.width, n.height), Point::new(0.0, n.height)];
    Some(corners.map(|c| {
        let p = w * c;
        app.to_screen(p.x, p.y)
    }))
}

/// 一条闭合四边形轮廓。
pub(super) fn quad(painter: &mut Painter, q: [Pos2; 4], s: Stroke) {
    painter.add(Shape::closed_line(q.to_vec(), s));
}

/// 轮廓颜色：组件 / 组件集 / 实例（及其内部所有图层）用组件紫，其余用画布叠加色。
pub(super) fn outline_colour(doc: &Document, id: NodeId, green: Color32) -> Color32 {
    use figcraft_engine::components::{owning_component, owning_instance};
    let set = matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::ComponentSet { .. }));
    if set || owning_component(doc, id).is_some() || owning_instance(doc, id).is_some() { COMPONENT } else { green }
}

/// 形状类节点（椭圆/矢量/多边形/星形/布尔组）的轮廓路径，屏幕坐标。
fn outline_shape(app: &CanvasApp, doc: &Document, id: NodeId) -> Option<Vec<Vec<Pos2>>> {
    let n = doc.node(id)?;
    if !matches!(
        n.kind,
        NodeKind::Ellipse { .. } | NodeKind::Vector { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::BooleanOp { .. }
    ) {
        return None;
    }
    let (path, _) = figcraft_render::node_outline(doc, n)?;
    let w = doc.world_transform(id);
    let mut out = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    // 展平容差按屏幕像素给（0.5px），缩放越大越精细。
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    figcraft_geom::kurbo::flatten(path.iter(), 0.5 / zoom, |el| {
        use figcraft_geom::kurbo::PathEl::*;
        match el {
            MoveTo(p) => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            LineTo(p) => {
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            ClosePath => {
                if let Some(f) = cur.first().copied() {
                    cur.push(f);
                }
            }
            _ => {}
        }
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    Some(out)
}

/// 图层在画布上的轮廓：形状类走真实几何，其余走包围盒。
pub(super) fn draw_outline(app: &CanvasApp, painter: &mut Painter, doc: &Document, id: NodeId, s: Stroke) {
    match outline_shape(app, doc, id) {
        Some(lines) => {
            for l in lines {
                painter.add(Shape::line(l, s));
            }
        }
        None => {
            if let Some(q) = node_quad(app, doc, id) {
                // 轴对齐的矩形：吸附到**设备像素**画四条填充带。描一条 1px 的路径时，边落在
                // 分数坐标上会被抗锯齿摊成两条半透明的线——四条边各自落点不同，看起来就是
                // "线粗细不均匀"。Figma 的选中框永远是一个设备像素、永远实心。
                if let Some(r) = axis_aligned(q)
                    && crisp_rect(painter, r, s.width, app.ppp, s.color)
                {
                    return;
                }
                quad(painter, q, s);
            }
        }
    }
}

/// 四边形若轴对齐，返回它的矩形（旋转过的不算）。
pub(super) fn axis_aligned(q: [Pos2; 4]) -> Option<Rect> {
    const EPS: f32 = 0.01;
    let [a, b, c, d] = q;
    let horizontal = (a.y - b.y).abs() < EPS && (d.y - c.y).abs() < EPS;
    let vertical = (a.x - d.x).abs() < EPS && (b.x - c.x).abs() < EPS;
    (horizontal && vertical).then(|| Rect::from_min_max(pos2(a.x.min(c.x), a.y.min(c.y)), pos2(a.x.max(c.x), a.y.max(c.y))))
}

/// 沿矩形画 `width`（逻辑像素）粗的描边，四条边都吸附到设备像素网格上。
///
/// 线宽取**整数个设备像素**（至少 1），矩形边先向外扩半个线宽（和描路径一样是居中描边），
/// 再把四条边取整到设备像素——于是每条边都恰好覆盖整列/整行像素，粗细恒定、边缘实心。
/// `ppp` 不合法时返回 false，调用方退回描路径。
pub(super) fn crisp_rect(painter: &mut Painter, r: Rect, width: f32, ppp: f32, col: Color32) -> bool {
    if !(ppp.is_finite() && ppp > 0.0 && width.is_finite() && width > 0.0) {
        return false;
    }
    // 整数个设备像素（至少 1），再换回逻辑像素。
    let t = (width * ppp).round().clamp(1.0, 64.0) / ppp;
    let half = t / 2.0;
    let snap = |v: f32| (v * ppp).round() / ppp;
    let (x0, y0) = (snap(r.min.x - half), snap(r.min.y - half));
    let (x1, y1) = (snap(r.max.x + half), snap(r.max.y + half));
    if x1 - x0 < t || y1 - y0 < t {
        return false;
    }
    let band = |a: Pos2, b: Pos2| Rect::from_min_max(a, b);
    // 上下两条通长，左右两条只补中间，四角不重叠（半透明颜色叠两次会更深）。
    painter.rect_filled(band(pos2(x0, y0), pos2(x1, y0 + t)), CornerRadius::ZERO, col);
    painter.rect_filled(band(pos2(x0, y1 - t), pos2(x1, y1)), CornerRadius::ZERO, col);
    painter.rect_filled(band(pos2(x0, y0 + t), pos2(x0 + t, y1 - t)), CornerRadius::ZERO, col);
    painter.rect_filled(band(pos2(x1 - t, y0 + t), pos2(x1, y1 - t)), CornerRadius::ZERO, col);
    true
}

#[cfg(test)]
mod crisp_tests {
    use super::*;
    use crate::canvas::painter::Op;

    fn rects(p: &Painter) -> Vec<Rect> {
        p.ops
            .iter()
            .filter_map(|op| match op {
                Op::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// 选中框：1px 的线在 1.75 的缩放下必须落在整设备像素上，四条边一样厚。
    #[test]
    fn a_one_pixel_outline_lands_on_whole_device_pixels() {
        let ppp = 1.75f32;
        let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0)));
        let r = Rect::from_min_max(pos2(10.37, 20.61), pos2(110.83, 80.42));
        assert!(crisp_rect(&mut p, r, 1.0, ppp, Color32::WHITE));
        let q = rects(&p);
        assert_eq!(q.len(), 4, "four bands");
        for band in &q {
            for v in [band.min.x, band.min.y, band.max.x, band.max.y] {
                let d = v * ppp;
                assert!((d - d.round()).abs() < 1e-3, "{v} is not on a device pixel");
            }
        }
        // 四条边厚度一致，且都是整数个设备像素（1×1.75 = 1.75 → 取整为 2 个设备像素）。
        let t = (1.0f32 * ppp).round() / ppp;
        let heights = [band_height(&q[0]), band_height(&q[1]), band_width(&q[2]), band_width(&q[3])];
        for h in heights {
            assert!((h - t).abs() < 1e-4, "thickness {h} is not {t} (an even number of device pixels)");
            let dev = h * ppp;
            assert!((dev - dev.round()).abs() < 1e-3, "{dev} device pixels is not a whole number");
        }
    }

    fn band_height(r: &Rect) -> f32 {
        r.max.y - r.min.y
    }
    fn band_width(r: &Rect) -> f32 {
        r.max.x - r.min.x
    }

    /// 旋转过的四边形不做吸附，退回描路径（否则会画成轴对齐的框）。
    #[test]
    fn a_rotated_quad_is_not_taken_for_a_rectangle() {
        let square = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0), pos2(0.0, 10.0)];
        assert!(axis_aligned(square).is_some());
        let turned = [pos2(0.0, 5.0), pos2(5.0, 0.0), pos2(10.0, 5.0), pos2(5.0, 10.0)];
        assert!(axis_aligned(turned).is_none());
    }

    /// 非法缩放/线宽不画（返回 false，调用方退回描路径）。
    #[test]
    fn absurd_scales_fall_back() {
        let mut p = Painter::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)));
        let r = Rect::from_min_max(pos2(1.0, 1.0), pos2(9.0, 9.0));
        assert!(!crisp_rect(&mut p, r, 1.0, 0.0, Color32::WHITE));
        assert!(!crisp_rect(&mut p, r, 1.0, f32::NAN, Color32::WHITE));
        assert!(!crisp_rect(&mut p, r, 0.0, 1.0, Color32::WHITE));
        assert!(p.ops.is_empty());
    }
}
