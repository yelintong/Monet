//! 画布画笔：**与 egui `Painter` 同名同语义**，后端换成 GPUI 的低层绘制 API。
//!
//! 这样原 `ui-egui/src/canvas/*` 里的绘制代码可以原样照搬：
//! `painter.rect_filled(...)` / `line_segment(...)` / `circle_filled(...)` / `text(...)`
//! 全部保留，只是这里改成"先记录、再统一绘制"。
//!
//! - 形状 → `window.paint_quad` / `window.paint_path`（canvas 元素绘制回调里执行）
//! - 文字 → 记录成 `Label`，由视图层用 GPUI 文本元素渲染（矢量、清晰，可精确居中）

#![allow(dead_code)]

use super::geom::*;
use gpui::{Bounds, Corners, PathBuilder, Pixels, Rgba, Window, point, px};

/// `Color32` → GPUI 的直通道颜色。
pub fn to_rgba(c: Color32) -> Rgba {
    Rgba { r: f32::from(c.0[0]) / 255.0, g: f32::from(c.0[1]) / 255.0, b: f32::from(c.0[2]) / 255.0, a: f32::from(c.0[3]) / 255.0 }
}

/// 排版好的一段文字（只需尺寸 + 内容：真正绘制交给 GPUI 文本元素）。
#[derive(Clone, Debug)]
pub struct Galley {
    pub text: String,
    pub font: FontId,
    pub color: Color32,
    pub size: Vec2,
}

impl Galley {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn size(&self) -> Vec2 {
        self.size
    }

    pub fn rect_at(&self, pos: Pos2) -> Rect {
        Rect::from_min_size(pos, self.size)
    }
}

/// 一个待绘制文字：位置与锚点（画布局部坐标）。
#[derive(Clone, Debug)]
pub struct Label {
    pub rect: Rect,
    pub text: String,
    pub font: FontId,
    pub color: Color32,
    pub align: Align2,
    /// `None` = 普通水平文字；`Some(rad)` = 绕 `rect` 中心旋转（标尺左侧数字用 -π/2）。
    /// GPUI 0.2.2 只有 SVG 绘制支持变换，见 `canvas::svg_text`。
    pub angle: Option<f32>,
}

#[derive(Clone, Debug)]
pub enum Op {
    Rect { rect: Rect, radius: CornerRadius, fill: Color32, stroke: Stroke, kind: StrokeKind },
    Line { a: Pos2, b: Pos2, stroke: Stroke },
    Circle { center: Pos2, radius: f32, fill: Color32, stroke: Stroke },
    Poly { points: Vec<Pos2>, closed: bool, fill: Color32, stroke: Stroke },
}

/// 文字宽度估算：画布上的文字都很短（标尺数字、尺寸标注、帧名），
/// 只需足够准的排版尺寸；绘制由 GPUI 矢量文本完成，位置与居中是精确的。
fn advance(c: char, size: f32) -> f32 {
    let k = match c {
        '0'..='9' => 0.556,
        ' ' => 0.28,
        '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 0.28,
        '-' | '/' => 0.34,
        '%' => 0.85,
        '×' => 0.72,
        'A'..='Z' => 0.64,
        'a'..='z' => 0.54,
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.56,
    };
    k * size
}

/// 文字的整体尺寸（宽 × 行高）。
pub fn text_size(text: &str, font: FontId) -> Vec2 {
    let w: f32 = text.chars().map(|c| advance(c, font.size)).sum();
    vec2(w, (font.size * 1.25).max(font.size))
}

/// 记录式画笔：同一套 egui 方法名。
#[derive(Clone, Debug, Default)]
pub struct Painter {
    /// 裁剪矩形（画布局部坐标）；GPUI 的 canvas 元素本身已按元素边界裁剪。
    pub clip: Rect,
    pub ops: Vec<Op>,
    pub labels: Vec<Label>,
}

impl Painter {
    pub fn new(clip: Rect) -> Self {
        Self { clip, ops: Vec::new(), labels: Vec::new() }
    }

    pub fn set_clip_rect(&mut self, clip: Rect) {
        self.clip = clip;
    }

    // ---------------------------------------------------------------- 矩形

    pub fn rect_filled(&mut self, rect: Rect, corner_radius: impl Into<CornerRadius>, fill_color: Color32) {
        self.ops.push(Op::Rect { rect, radius: corner_radius.into(), fill: fill_color, stroke: Stroke::NONE, kind: StrokeKind::Inside });
    }

    pub fn rect_stroke(&mut self, rect: Rect, corner_radius: impl Into<CornerRadius>, stroke: impl Into<Stroke>, kind: StrokeKind) {
        self.ops.push(Op::Rect { rect, radius: corner_radius.into(), fill: Color32::TRANSPARENT, stroke: stroke.into(), kind });
    }

    pub fn rect(&mut self, rect: Rect, corner_radius: impl Into<CornerRadius>, fill_color: Color32, stroke: impl Into<Stroke>, kind: StrokeKind) {
        self.ops.push(Op::Rect { rect, radius: corner_radius.into(), fill: fill_color, stroke: stroke.into(), kind });
    }

    // ---------------------------------------------------------------- 线

    pub fn line_segment(&mut self, points: [Pos2; 2], stroke: impl Into<Stroke>) {
        let stroke = stroke.into();
        if stroke.width <= 0.0 || stroke.color.a() == 0 {
            return;
        }
        self.ops.push(Op::Line { a: points[0], b: points[1], stroke });
    }

    // ---------------------------------------------------------------- 圆

    pub fn circle_filled(&mut self, center: Pos2, radius: f32, fill_color: Color32) {
        self.ops.push(Op::Circle { center, radius, fill: fill_color, stroke: Stroke::NONE });
    }

    pub fn circle_stroke(&mut self, center: Pos2, radius: f32, stroke: impl Into<Stroke>) {
        self.ops.push(Op::Circle { center, radius, fill: Color32::TRANSPARENT, stroke: stroke.into() });
    }

    pub fn circle(&mut self, center: Pos2, radius: f32, fill_color: Color32, stroke: impl Into<Stroke>) {
        self.ops.push(Op::Circle { center, radius, fill: fill_color, stroke: stroke.into() });
    }

    // ---------------------------------------------------------------- 多段线

    pub fn add_shape(&mut self, points: Vec<Pos2>, closed: bool, fill: Color32, stroke: impl Into<Stroke>) {
        self.ops.push(Op::Poly { points, closed, fill, stroke: stroke.into() });
    }

    /// egui 的 `painter.add(Shape::…)`。
    pub fn add(&mut self, shape: Shape) {
        match shape {
            Shape::LineSegment { points, stroke } => self.line_segment(points, stroke),
            Shape::Circle { center, radius, fill, stroke } => self.circle(center, radius, fill, stroke),
            Shape::Rect { rect, corner_radius, fill, stroke, stroke_kind } => self.rect(rect, corner_radius, fill, stroke, stroke_kind),
            Shape::Path { points, closed, fill, stroke } => self.add_shape(points, closed, fill, stroke),
        }
    }

    // ---------------------------------------------------------------- 文字

    pub fn layout_no_wrap(&self, text: impl Into<String>, font: FontId, color: Color32) -> Galley {
        let text = text.into();
        let size = text_size(&text, font);
        Galley { text, font, color, size }
    }

    pub fn text(&mut self, pos: Pos2, anchor: Align2, text: impl Into<String>, font: FontId, color: Color32) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        let size = text_size(&text, font);
        let rect = anchor.anchor_size(pos, size);
        self.labels.push(Label { rect, text, font, color, align: anchor, angle: None });
    }

    /// 在 `pos`（左上角）贴一段已排好的文字。
    pub fn galley(&mut self, pos: Pos2, galley: Galley, fallback_color: Color32) {
        let color = if fallback_color.a() > 0 { fallback_color } else { galley.color };
        self.labels.push(Label { rect: Rect::from_min_size(pos, galley.size), text: galley.text, font: galley.font, color, align: Align2::LEFT_TOP, angle: None });
    }

    // ---------------------------------------------------------------- 输出

    /// 把记录的形状画进 GPUI 场景；`origin` 是画布左上角在窗口中的位置。
    pub fn paint(&self, window: &mut Window, origin: Pos2) {
        let o = origin;
        for op in &self.ops {
            match op {
                Op::Rect { rect, radius, fill, stroke, kind } => {
                    let (r, bw) = match kind {
                        StrokeKind::Inside => (*rect, stroke.width),
                        StrokeKind::Middle => (rect.expand(stroke.width / 2.0), stroke.width),
                        StrokeKind::Outside => (rect.expand(stroke.width), stroke.width),
                    };
                    if !r.is_positive() {
                        continue;
                    }
                    let mut q = gpui::fill(bounds_of(r, o), to_rgba(*fill)).corner_radii(corners(*radius));
                    if bw > 0.0 && stroke.color.a() > 0 {
                        q = q.border_widths(px(bw)).border_color(to_rgba(stroke.color));
                    }
                    window.paint_quad(q);
                }
                Op::Line { a, b, stroke } => paint_line(window, *a + o.to_vec2(), *b + o.to_vec2(), stroke),
                Op::Circle { center, radius, fill, stroke } => {
                    paint_circle(window, *center + o.to_vec2(), *radius, *fill, *stroke);
                }
                Op::Poly { points, closed, fill, stroke } => {
                    let pts: Vec<Pos2> = points.iter().map(|p| *p + o.to_vec2()).collect();
                    paint_poly(window, &pts, *closed, *fill, *stroke);
                }
            }
        }
    }
}

fn bounds_of(r: Rect, o: Pos2) -> Bounds<Pixels> {
    Bounds { origin: point(px(r.min.x + o.x), px(r.min.y + o.y)), size: gpui::size(px(r.width().max(0.0)), px(r.height().max(0.0))) }
}

fn corners(r: CornerRadius) -> Corners<Pixels> {
    Corners { top_left: px(f32::from(r.nw)), top_right: px(f32::from(r.ne)), bottom_right: px(f32::from(r.se)), bottom_left: px(f32::from(r.sw)) }
}

/// 一条线：正交线走 quad（像素对齐、锐利），斜线走四边形路径（抗锯齿）。
fn paint_line(window: &mut Window, a: Pos2, b: Pos2, stroke: &Stroke) {
    let w = stroke.width.max(0.0);
    if w <= 0.0 || stroke.color.a() == 0 {
        return;
    }
    let color = to_rgba(stroke.color);
    if (a.x - b.x).abs() < 0.01 {
        let x = a.x - w / 2.0;
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        window.paint_quad(gpui::fill(Bounds { origin: point(px(x), px(y0)), size: gpui::size(px(w.max(1.0)), px((y1 - y0).max(1.0))) }, color));
        return;
    }
    if (a.y - b.y).abs() < 0.01 {
        let y = a.y - w / 2.0;
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        window.paint_quad(gpui::fill(Bounds { origin: point(px(x0), px(y)), size: gpui::size(px((x1 - x0).max(1.0)), px(w.max(1.0))) }, color));
        return;
    }
    let d = b - a;
    let n = vec2(-d.y, d.x).normalized() * (w / 2.0);
    let pts = vec![a + n, b + n, b - n, a - n];
    paint_poly(window, &pts, true, stroke.color, Stroke::NONE);
}

fn paint_circle(window: &mut Window, center: Pos2, radius: f32, fill: Color32, stroke: Stroke) {
    if radius <= 0.0 {
        return;
    }
    if fill.a() > 0 {
        // ⛔ 不要用 `PathBuilder` 贝塞尔画圆：GPUI 0.2.1 下 `paint_path` 对曲线路径的填充会画成
        // 一坨（播放头的圆头就是这样）。正方形 + 四角半径 = r 的 `paint_quad` 是同一个圆，且是
        // 全项目到处在用的最可靠图元。
        let r = radius;
        let rect = Bounds { origin: point(px(center.x - r), px(center.y - r)), size: gpui::size(px(r * 2.0), px(r * 2.0)) };
        let quad = gpui::fill(rect, to_rgba(fill)).corner_radii(Corners {
            top_left: px(r),
            top_right: px(r),
            bottom_right: px(r),
            bottom_left: px(r),
        });
        window.paint_quad(quad);
    }
    if stroke.width > 0.0 && stroke.color.a() > 0 {
        // 描边必须是**一条闭合的 stroke 路径**：以前逐段 `paint_line` 拼几十个独立四边形，
        // 段与段之间没有 join，宽描边会碎成一段一段的弧块（动画 gizmo 的 5pt 白色底环最
        // 明显，半径越大越碎）。lyon 的 StrokeTessellator 对闭合路径会自动补 join，得到
        // 连续的一整圈；用折线近似而非贝塞尔，避开 GPUI 0.2.1 对曲线路径填充的问题。
        let segments = ((radius * 2.5) as usize).clamp(16, 96);
        let mut b = PathBuilder::stroke(px(stroke.width));
        for i in 0..segments {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            let p = point(px(center.x + radius * t.cos()), px(center.y + radius * t.sin()));
            if i == 0 {
                b.move_to(p);
            } else {
                b.line_to(p);
            }
        }
        b.close();
        if let Ok(path) = b.build() {
            window.paint_path(path, to_rgba(stroke.color));
        }
    }
}

fn paint_poly(window: &mut Window, pts: &[Pos2], closed: bool, fill: Color32, stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }
    if fill.a() > 0 && pts.len() >= 3 {
        let mut pb = PathBuilder::fill();
        pb.move_to(point(px(pts[0].x), px(pts[0].y)));
        for p in &pts[1..] {
            pb.line_to(point(px(p.x), px(p.y)));
        }
        if closed {
            pb.close();
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, to_rgba(fill));
        }
    }
    if stroke.width > 0.0 && stroke.color.a() > 0 {
        let n = if closed { pts.len() } else { pts.len().saturating_sub(1) };
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            paint_line(window, a, b, &stroke);
        }
    }
}
