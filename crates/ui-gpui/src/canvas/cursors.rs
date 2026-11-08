//! 指针在画布上的样子（照搬 `ui-egui/src/canvas/cursors.rs` 的映射与自绘字形）。
//!
//! 引擎给出 `Cursor`（默认/十字/移动/缩放角度/旋转/抓手/文字/钢笔/复制/手型/禁行…），
//! 这里换成系统光标；旋转与钢笔是 FigCraft 自绘的小字形（系统光标隐藏）。

use figcraft_engine::tools::{Cursor, PenCursor};
use gpui::CursorStyle;

use super::geom::*;
use super::painter::Painter;

/// FigCraft 自绘的光标字形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    /// 沿方向（度，0 = 右，90 = 下）的弧形双箭头：从框心指向角。
    Rotate(f32),
    Pen(PenCursor),
}

/// 指针当前显示什么。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    System(CursorStyle),
    Drawn(Glyph),
}

/// 沿 `degrees`（0 = 水平，90 = 垂直）缩放的最近系统光标（屏幕角度，45 度从左上到右下）。
pub fn resize_icon(degrees: f64) -> CursorStyle {
    let a = degrees.rem_euclid(180.0);
    if !(22.5..157.5).contains(&a) {
        CursorStyle::ResizeLeftRight
    } else if a < 67.5 {
        CursorStyle::ResizeUpLeftDownRight
    } else if a < 112.5 {
        CursorStyle::ResizeUpDown
    } else {
        CursorStyle::ResizeUpRightDownLeft
    }
}

/// 引擎的 `cursor` 显示成什么。
pub fn of(cursor: Cursor) -> Shown {
    use Shown::{Drawn, System};
    match cursor {
        Cursor::Default => System(CursorStyle::Arrow),
        Cursor::Crosshair => System(CursorStyle::Crosshair),
        // GPUI 没有「四向移动」光标，用箭头代替。
        Cursor::Move => System(CursorStyle::Arrow),
        Cursor::Resize(a) if a.is_finite() => System(resize_icon(a)),
        Cursor::Resize(_) => System(CursorStyle::Arrow),
        Cursor::Rotate(a) => Drawn(Glyph::Rotate(if a.is_finite() { a.rem_euclid(360.0) as f32 } else { 0.0 })),
        Cursor::Grab => System(CursorStyle::OpenHand),
        Cursor::Grabbing => System(CursorStyle::ClosedHand),
        Cursor::Text => System(CursorStyle::IBeam),
        Cursor::Pen(p) => Drawn(Glyph::Pen(p)),
        Cursor::Copy => System(CursorStyle::DragCopy),
        Cursor::Pointer => System(CursorStyle::PointingHand),
        // 吸管/评论针是图标字形（要图标字体），暂用箭头。
        Cursor::Eyedropper => System(CursorStyle::Arrow),
        Cursor::Comment => System(CursorStyle::Arrow),
        Cursor::NotAllowed => System(CursorStyle::OperationNotAllowed),
    }
}

/// 画布上显示什么：平移（空格或中键：开手/按下时握拳）与缩放工具（Z；`Some(true)`：Alt 缩小）
/// 优先于工具自己的光标。
pub fn on_canvas(panning: bool, button_down: bool, zoom_out: Option<bool>, cursor: Cursor) -> Shown {
    match (panning, zoom_out) {
        (true, _) => Shown::System(if button_down { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }),
        (false, Some(_)) => Shown::System(CursorStyle::Crosshair),
        (false, None) => of(cursor),
    }
}

impl Shown {
    /// 要向系统申请的光标（自绘字形时隐藏系统光标）。
    pub fn icon(self) -> CursorStyle {
        match self {
            Shown::System(i) => i,
            Shown::Drawn(_) => CursorStyle::None,
        }
    }
}

const INK: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
const EDGE: Color32 = Color32::WHITE;

/// 一条线，下面先垫一条浅色边（页面上任何底色都看得清）。
fn line(painter: &mut Painter, pts: &[Pos2]) {
    painter.add_shape(pts.to_vec(), false, Color32::TRANSPARENT, Stroke::new(3.5, EDGE));
    painter.add_shape(pts.to_vec(), false, Color32::TRANSPARENT, Stroke::new(1.5, INK));
}

/// 圆弧上从 `a0` 到 `a1`（弧度）的点。
fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    (0..=12).map(|i| a0 + (a1 - a0) * i as f32 / 12.0).map(|a| c + vec2(a.cos(), a.sin()) * r).collect()
}

/// 把 `glyph` 画在热区 `at` 上。
pub fn paint(painter: &mut Painter, at: Pos2, glyph: Glyph) {
    match glyph {
        Glyph::Rotate(degrees) => {
            // 从框心向外鼓起的弧，穿过指针，两端各一个箭头。
            let (dir, r, half) = (degrees.to_radians(), 11.0, 0.8);
            let c = at - vec2(dir.cos(), dir.sin()) * r;
            let pts = arc(c, r, dir - half, dir + half);
            line(painter, &pts);
            for (end, sign) in [(dir - half, -1.0), (dir + half, 1.0)] {
                let tip = c + vec2(end.cos(), end.sin()) * r;
                // 沿弧在该端的方向，继续向前。
                let t = vec2(-end.sin(), end.cos()) * sign;
                let n = vec2(end.cos(), end.sin());
                line(painter, &[tip - t * 4.5 + n * 3.5, tip + t * 1.0, tip - t * 4.5 - n * 3.5]);
            }
        }
        Glyph::Pen(state) => {
            // 中心敞开的十字（落点看得见），右下角表示一次点击会做什么。
            for (a, b) in [
                (vec2(-8.0, 0.0), vec2(-2.5, 0.0)),
                (vec2(2.5, 0.0), vec2(8.0, 0.0)),
                (vec2(0.0, -8.0), vec2(0.0, -2.5)),
                (vec2(0.0, 2.5), vec2(0.0, 8.0)),
            ] {
                line(painter, &[at + a, at + b]);
            }
            let b = at + vec2(10.0, 10.0);
            match state {
                PenCursor::New => {}
                PenCursor::Close => {
                    let mut ring = arc(b, 3.0, 0.0, std::f32::consts::TAU);
                    ring.push(pos2(b.x + 3.0, b.y));
                    line(painter, &ring);
                }
                PenCursor::Add => {
                    line(painter, &[b - vec2(3.5, 0.0), b + vec2(3.5, 0.0)]);
                    line(painter, &[b - vec2(0.0, 3.5), b + vec2(0.0, 3.5)]);
                }
                PenCursor::Bend => line(painter, &arc(b + vec2(-3.5, 3.5), 7.0, -std::f32::consts::FRAC_PI_2, 0.0)),
            }
        }
    }
}
