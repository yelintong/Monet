//! egui 几何/颜色子集：**同名同语义**。
//!
//! 目的：原 `crates/ui-egui/src/canvas/*` 的画布代码（标尺、叠加层、手柄、吸附…）
//! 可以直接**照搬**过来，只需把 `use egui::{...}` 换成 `use super::geom::{...}`，
//! 不用改写任何几何/颜色运算逻辑。

#![allow(dead_code)]

use std::ops::{Add, Div, Mul, Neg, Sub};

// ---------------------------------------------------------------- Pos2 / Vec2

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Pos2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

pub const fn pos2(x: f32, y: f32) -> Pos2 {
    Pos2 { x, y }
}

pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Pos2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn to_vec2(self) -> Vec2 {
        vec2(self.x, self.y)
    }

    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn floor(self) -> Self {
        pos2(self.x.floor(), self.y.floor())
    }

    pub fn round(self) -> Self {
        pos2(self.x.round(), self.y.round())
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        pos2(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }

    pub fn angle_from(self, other: Self) -> f32 {
        (other.y - self.y).atan2(other.x - self.x)
    }

    /// 绕 `pivot` 旋转 `angle` 弧度。
    pub fn rotate_about(self, pivot: Self, angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let (dx, dy) = (self.x - pivot.x, self.y - pivot.y);
        pos2(pivot.x + dx * c - dy * s, pivot.y + dx * s + dy * c)
    }
}

impl Add<Vec2> for Pos2 {
    type Output = Pos2;
    fn add(self, rhs: Vec2) -> Pos2 {
        pos2(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Add for Pos2 {
    type Output = Pos2;
    fn add(self, rhs: Pos2) -> Pos2 {
        pos2(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub<Vec2> for Pos2 {
    type Output = Pos2;
    fn sub(self, rhs: Vec2) -> Pos2 {
        pos2(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Sub for Pos2 {
    type Output = Vec2;
    fn sub(self, rhs: Pos2) -> Vec2 {
        vec2(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Pos2 {
    type Output = Pos2;
    fn mul(self, rhs: f32) -> Pos2 {
        pos2(self.x * rhs, self.y * rhs)
    }
}

impl From<(f32, f32)> for Pos2 {
    fn from((x, y): (f32, f32)) -> Self {
        pos2(x, y)
    }
}

impl From<Pos2> for (f32, f32) {
    fn from(p: Pos2) -> Self {
        (p.x, p.y)
    }
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const X: Self = Self { x: 1.0, y: 0.0 };
    pub const Y: Self = Self { x: 0.0, y: 1.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 0.0 { vec2(self.x / len, self.y / len) } else { Self::ZERO }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        vec2(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        vec2(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: f32) -> Vec2 {
        vec2(self.x * rhs, self.y * rhs)
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    fn div(self, rhs: f32) -> Vec2 {
        vec2(self.x / rhs, self.y / rhs)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        vec2(-self.x, -self.y)
    }
}

// ---------------------------------------------------------------------- Rect

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub min: Pos2,
    pub max: Pos2,
}

impl Default for Rect {
    fn default() -> Self {
        Self::NOTHING
    }
}

impl Rect {
    /// 空矩形（min = +inf, max = -inf），与 egui 一致。
    pub const NOTHING: Self = Self { min: Pos2::new(f32::INFINITY, f32::INFINITY), max: Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY) };

    pub const fn from_min_max(min: Pos2, max: Pos2) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: Pos2, size: Vec2) -> Self {
        Self { min, max: pos2(min.x + size.x, min.y + size.y) }
    }

    pub fn from_two_pos(a: Pos2, b: Pos2) -> Self {
        Self { min: pos2(a.x.min(b.x), a.y.min(b.y)), max: pos2(a.x.max(b.x), a.y.max(b.y)) }
    }

    pub fn from_center_size(center: Pos2, size: Vec2) -> Self {
        Self::from_min_size(pos2(center.x - size.x / 2.0, center.y - size.y / 2.0), size)
    }

    /// 包住一串点的矩形（egui 的 `Rect::from_points`）。
    pub fn from_points(points: &[Pos2]) -> Self {
        let mut r = Self::NOTHING;
        for p in points {
            r = r.union(Self::from_min_max(*p, *p));
        }
        r
    }

    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn size(self) -> Vec2 {
        vec2(self.width(), self.height())
    }

    pub fn center(self) -> Pos2 {
        pos2((self.min.x + self.max.x) / 2.0, (self.min.y + self.max.y) / 2.0)
    }

    pub fn left(self) -> f32 {
        self.min.x
    }

    pub fn right(self) -> f32 {
        self.max.x
    }

    pub fn top(self) -> f32 {
        self.min.y
    }

    pub fn bottom(self) -> f32 {
        self.max.y
    }

    pub fn contains(self, p: Pos2) -> bool {
        self.min.x <= p.x && p.x <= self.max.x && self.min.y <= p.y && p.y <= self.max.y
    }

    /// 两矩形是否相交（egui `Rect::intersects`）。
    pub fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x && other.min.x <= self.max.x && self.min.y <= other.max.y && other.min.y <= self.max.y
    }

    pub fn is_positive(self) -> bool {
        self.max.x > self.min.x && self.max.y > self.min.y
    }

    pub fn is_finite(self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }

    pub fn expand(self, amount: f32) -> Self {
        Self { min: pos2(self.min.x - amount, self.min.y - amount), max: pos2(self.max.x + amount, self.max.y + amount) }
    }

    pub fn shrink(self, amount: f32) -> Self {
        self.expand(-amount)
    }

    pub fn expand2(self, amount: Vec2) -> Self {
        Self { min: pos2(self.min.x - amount.x, self.min.y - amount.y), max: pos2(self.max.x + amount.x, self.max.y + amount.y) }
    }

    pub fn translate(self, by: Vec2) -> Self {
        Self { min: self.min + by, max: self.max + by }
    }

    pub fn intersect(self, other: Self) -> Self {
        Self {
            min: pos2(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: pos2(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: pos2(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: pos2(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    pub fn clamp(self, p: Pos2) -> Pos2 {
        pos2(p.x.clamp(self.min.x, self.max.x), p.y.clamp(self.min.y, self.max.y))
    }

    /// 四条边（左上 → 右上 → 右下 → 左下）。
    pub fn corners(self) -> [Pos2; 4] {
        [self.min, pos2(self.max.x, self.min.y), self.max, pos2(self.min.x, self.max.y)]
    }
}

impl Add<Vec2> for Rect {
    type Output = Rect;
    fn add(self, rhs: Vec2) -> Rect {
        self.translate(rhs)
    }
}

// ------------------------------------------------------------------- Color32

/// 直通道 RGBA（非预乘），与 egui 的构造/使用方式一致。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color32(pub [u8; 4]);

impl Color32 {
    pub const TRANSPARENT: Self = Self([0, 0, 0, 0]);
    pub const BLACK: Self = Self([0, 0, 0, 255]);
    pub const WHITE: Self = Self([255, 255, 255, 255]);
    pub const RED: Self = Self([255, 0, 0, 255]);
    pub const GREEN: Self = Self([0, 255, 0, 255]);
    pub const BLUE: Self = Self([0, 0, 255, 255]);
    pub const GRAY: Self = Self([128, 128, 128, 255]);

    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    pub const fn from_rgba_unmultiplied(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    pub const fn from_rgba_premultiplied(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    pub const fn from_gray(v: u8) -> Self {
        Self([v, v, v, 255])
    }

    pub const fn from_black_alpha(a: u8) -> Self {
        Self([0, 0, 0, a])
    }

    pub const fn from_white_alpha(a: u8) -> Self {
        Self([255, 255, 255, a])
    }

    pub fn a(self) -> u8 {
        self.0[3]
    }

    pub fn to_array(self) -> [u8; 4] {
        self.0
    }

    /// 与 egui 一致：四个通道等比例缩小（用于做半透明高亮）。
    pub fn gamma_multiply(self, factor: f32) -> Self {
        let f = |v: u8| ((f32::from(v) * factor).round().clamp(0.0, 255.0)) as u8;
        Self([f(self.0[0]), f(self.0[1]), f(self.0[2]), f(self.0[3])])
    }

    pub fn linear_multiply(self, factor: f32) -> Self {
        self.gamma_multiply(factor)
    }

    /// Replace the alpha channel, keeping RGB (egui's `with_alpha`).
    pub fn with_alpha(self, a: u8) -> Self {
        let mut c = self.0;
        c[3] = a;
        Self(c)
    }

    /// Parse `"#RGB"` / `"#RRGGBB"` / `"#RRGGBBAA"` (the leading `#` is optional). Returns `None`
    /// on any malformed input — callers fall back to the previous value.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        let h = s.strip_prefix('#').unwrap_or(s);
        let b = h.as_bytes();
        let parse2 = |i: usize| {
            let hi = (b[i] as char).to_digit(16)?;
            let lo = (b[i + 1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        };
        match b.len() {
            3 => Some(Self([parse2(0)? * 17, parse2(1)? * 17, parse2(2)? * 17, 255])),
            6 => Some(Self([parse2(0)?, parse2(2)?, parse2(4)?, 255])),
            8 => Some(Self([parse2(0)?, parse2(2)?, parse2(4)?, parse2(6)?])),
            _ => None,
        }
    }

    /// `"#RRGGBB"` (opaque — alpha is dropped).
    pub fn to_hex(self) -> String {
        let [r, g, b, _] = self.0;
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    }

    /// `"#RRGGBBAA"` (alpha included).
    pub fn to_hex_alpha(self) -> String {
        let [r, g, b, a] = self.0;
        format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
    }
}

// ------------------------------------------------------------------- Stroke

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: Color32,
}

impl Stroke {
    pub const NONE: Self = Self { width: 0.0, color: Color32::TRANSPARENT };

    pub const fn new(width: f32, color: Color32) -> Self {
        Self { width, color }
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self::NONE
    }
}

/// 描边相对矩形的位置（egui 0.29+ 的 `StrokeKind`）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum StrokeKind {
    Inside,
    #[default]
    Middle,
    Outside,
}

// -------------------------------------------------------------- CornerRadius

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CornerRadius {
    pub nw: u8,
    pub ne: u8,
    pub sw: u8,
    pub se: u8,
}

impl CornerRadius {
    pub const ZERO: Self = Self { nw: 0, ne: 0, sw: 0, se: 0 };

    pub const fn same(radius: u8) -> Self {
        Self { nw: radius, ne: radius, sw: radius, se: radius }
    }
}

impl From<u8> for CornerRadius {
    fn from(v: u8) -> Self {
        Self::same(v)
    }
}

// -------------------------------------------------------------------- Align2

/// 文字锚点（0=左/上，1=右/下），与 egui 的 `Align2` 同名常量一致。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Align2 {
    pub x: u8,
    pub y: u8,
}

impl Align2 {
    pub const LEFT_TOP: Self = Self { x: 0, y: 0 };
    pub const CENTER_TOP: Self = Self { x: 1, y: 0 };
    pub const RIGHT_TOP: Self = Self { x: 2, y: 0 };
    pub const LEFT_CENTER: Self = Self { x: 0, y: 1 };
    pub const CENTER_CENTER: Self = Self { x: 1, y: 1 };
    pub const RIGHT_CENTER: Self = Self { x: 2, y: 1 };
    pub const LEFT_BOTTOM: Self = Self { x: 0, y: 2 };
    pub const CENTER_BOTTOM: Self = Self { x: 1, y: 2 };
    pub const RIGHT_BOTTOM: Self = Self { x: 2, y: 2 };

    /// 把一个 `size` 大小的框放到锚点 `pos` 上。
    pub fn anchor_size(self, pos: Pos2, size: Vec2) -> Rect {
        let x = match self.x {
            0 => pos.x,
            2 => pos.x - size.x,
            _ => pos.x - size.x / 2.0,
        };
        let y = match self.y {
            0 => pos.y,
            2 => pos.y - size.y,
            _ => pos.y - size.y / 2.0,
        };
        Rect::from_min_size(pos2(x, y), size)
    }

    /// 把 `rect` 对齐进 `frame`。
    pub fn anchor_rect(self, frame: Rect) -> Rect {
        self.anchor_size(frame.min, frame.size())
    }

    /// 在 `rect` 内摆放 `size`。
    pub fn align_size_within_rect(self, size: Vec2, rect: Rect) -> Rect {
        let x = match self.x {
            0 => rect.min.x,
            2 => rect.max.x - size.x,
            _ => rect.min.x + (rect.width() - size.x) / 2.0,
        };
        let y = match self.y {
            0 => rect.min.y,
            2 => rect.max.y - size.y,
            _ => rect.min.y + (rect.height() - size.y) / 2.0,
        };
        Rect::from_min_size(pos2(x, y), size)
    }
}

// -------------------------------------------------------------------- 字体

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    SemiBold,
    Bold,
}

/// egui `FontId` 的等价：只保留字号与字重（GPUI 侧映射为 font_size / font_weight）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FontId {
    pub size: f32,
    pub weight: FontWeight,
}

impl FontId {
    pub const fn new(size: f32, weight: FontWeight) -> Self {
        Self { size, weight }
    }
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontWeight::Normal)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontWeight::SemiBold)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontWeight::Bold)
}

// -------------------------------------------------------------------- Shape

/// egui `Shape` 中画布用到的子集（照搬代码直接可用）。
#[derive(Clone, Debug)]
pub enum Shape {
    LineSegment {
        points: [Pos2; 2],
        stroke: Stroke,
    },
    Circle {
        center: Pos2,
        radius: f32,
        fill: Color32,
        stroke: Stroke,
    },
    Rect {
        rect: Rect,
        corner_radius: CornerRadius,
        fill: Color32,
        stroke: Stroke,
        stroke_kind: StrokeKind,
    },
    /// 折线（`closed` 时闭合），只描边。
    Path {
        points: Vec<Pos2>,
        closed: bool,
        fill: Color32,
        stroke: Stroke,
    },
}

impl Shape {
    pub fn line_segment(points: [Pos2; 2], stroke: impl Into<Stroke>) -> Self {
        Self::LineSegment { points, stroke: stroke.into() }
    }

    pub fn circle_filled(center: Pos2, radius: f32, fill: Color32) -> Self {
        Self::Circle { center, radius, fill, stroke: Stroke::NONE }
    }

    pub fn circle_stroke(center: Pos2, radius: f32, stroke: impl Into<Stroke>) -> Self {
        Self::Circle { center, radius, fill: Color32::TRANSPARENT, stroke: stroke.into() }
    }

    pub fn closed_line(points: Vec<Pos2>, stroke: impl Into<Stroke>) -> Self {
        Self::Path { points, closed: true, fill: Color32::TRANSPARENT, stroke: stroke.into() }
    }

    /// 开放折线（egui 的 `Shape::line`）。
    pub fn line(points: Vec<Pos2>, stroke: impl Into<Stroke>) -> Self {
        Self::Path { points, closed: false, fill: Color32::TRANSPARENT, stroke: stroke.into() }
    }

    /// 折线虚线（按段长切分），与 egui 的 `Shape::dashed_line` 同语义。
    pub fn dashed_line(points: &[Pos2], stroke: Stroke, dash_length: f32, gap: f32) -> Vec<Shape> {
        let dash = dash_length.max(0.5);
        let gap = gap.max(0.0);
        let mut out = Vec::new();
        let mut carry = 0.0_f32; // 下一段起点在整条折线中的偏移（相对段首）
        let mut drawing = true;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let d = b - a;
            let len = d.length();
            if len <= 0.0 {
                continue;
            }
            let dir = d / len;
            let mut t = 0.0;
            while t < len {
                let step = if drawing { dash } else { gap } - carry;
                let e = (t + step).min(len);
                if drawing {
                    out.push(Shape::line_segment([a + dir * t, a + dir * e], stroke));
                }
                carry += e - t;
                if carry >= (if drawing { dash } else { gap }) {
                    carry = 0.0;
                    drawing = !drawing;
                }
                t = e;
            }
        }
        out
    }

    pub fn convex_polygon(points: Vec<Pos2>, fill: Color32, stroke: impl Into<Stroke>) -> Self {
        Self::Path { points, closed: true, fill, stroke: stroke.into() }
    }
}
