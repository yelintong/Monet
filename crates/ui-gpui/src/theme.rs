//! 画布用到的主题令牌，取值与 `ui-egui/src/theme.rs` 的**四主题系统完全一致**：
//! 一条中性轴（亮/暗）加一条品牌轴（蓝/绿/黄/粉，来自 `FigCraft.variables.json`）。
//!
//! 品牌轴是 Figma `them` 集合；每个强调色给出 `Color`（主色）与 `Color1`（暗一档，用于
//! hover/pressed），`Color2`/`Color3`（主色 10%/5% 透明，用作选中行底色）。中性轴是 Figma
//! `Variable collection`：`Back 1/2`（面板/画布）、`Border Line`（描边），以及 `Black 1/2/3`——
//! 后三者就是带透明度的黑或白，按导出的原值直接存进令牌，由画布每帧叠在面板上。

#![allow(dead_code)]

use figcraft_i18n::tr;
use crate::canvas::geom::Color32;

/// FigCraft 的品牌强调色。Figma `them` 集合给出蓝/绿/黄/粉；绿沿用现有 FigCraft 身份，故为默认。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Blue,
    Green,
    Yellow,
    Pink,
}

impl Accent {
    /// 解析偏好字符串（"blue"/"green"/"yellow"/"pink"）；其它一律回退到 Green。
    pub fn from_str(s: &str) -> Accent {
        match s.trim().to_ascii_lowercase().as_str() {
            "blue" => Accent::Blue,
            "yellow" => Accent::Yellow,
            "pink" => Accent::Pink,
            _ => Accent::Green,
        }
    }

    /// 该强调色的偏好字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Accent::Blue => "blue",
            Accent::Green => "green",
            Accent::Yellow => "yellow",
            Accent::Pink => "pink",
        }
    }

    /// 人类可读标签（设置控件与菜单用）。
    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => tr("Blue"),
            Accent::Green => tr("Green"),
            Accent::Yellow => tr("Yellow"),
            Accent::Pink => tr("Pink"),
        }
    }

    /// 四个强调色，按设置/菜单顺序。
    pub const ALL: [Accent; 4] = [Accent::Blue, Accent::Green, Accent::Yellow, Accent::Pink];

    /// `(light Color, light Color1, dark Color, dark Color1)`，直接取自 Figma 导出。
    fn palette(self) -> (Color32, Color32, Color32, Color32) {
        match self {
            Accent::Green => (c(0x34, 0xC7, 0x59), c(0x2E, 0xB2, 0x50), c(0x30, 0xD1, 0x58), c(0x2A, 0xBC, 0x4E)),
            Accent::Blue => (c(0x00, 0x88, 0xFF), c(0x00, 0x7A, 0xE5), c(0x00, 0x91, 0xFF), c(0x00, 0x83, 0xE5)),
            Accent::Yellow => (c(0xFF, 0xCC, 0x00), c(0xE5, 0xB8, 0x00), c(0xFF, 0xD6, 0x00), c(0xE5, 0xC1, 0x00)),
            Accent::Pink => (c(0xFF, 0x2D, 0x55), c(0xFF, 0x14, 0x41), c(0xFF, 0x37, 0x5F), c(0xFF, 0x1F, 0x4B)),
        }
    }
}

/// `Color32::from_rgb` 的简写，避免每个调色板项都大写喊一遍。
fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// `rgba(0,0,0,a)` / `rgba(255,255,255,a)` —— 直连（未预乘）的透明度，与 Figma 存储一致。
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// 把 `c` 的透明度换成 `a`（0–255）。
fn with_alpha(c: Color32, a: u8) -> Color32 {
    c.with_alpha(a)
}

/// `x` 作为文档色（alpha 强制不透明）用于对比度计算。
fn to_color(x: Color32) -> figcraft_color::Color {
    figcraft_color::Color::new(x.0[0] as f32 / 255.0, x.0[1] as f32 / 255.0, x.0[2] as f32 / 255.0, 1.0)
}

/// `fg` 落在 `bg` 上的 WCAG 对比度（先把 `fg` 可能带有的透明度叠到 `bg` 上）。
fn contrast(fg: Color32, bg: Color32) -> f32 {
    to_color(over(fg, bg)).contrast(to_color(bg))
}

/// 把 `fg`（可带 alpha）叠到 `bg` 上。
fn over(fg: Color32, bg: Color32) -> Color32 {
    let a = fg.0[3] as f32 / 255.0;
    let mix = |x: u8, y: u8| (x as f32 * a + y as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(fg.0[0], bg.0[0]), mix(fg.0[1], bg.0[1]), mix(fg.0[2], bg.0[2]))
}

/// `c` 朝 `target` 线性混合 `t`（0 = `c`，1 = `target`）。
fn toward(c: Color32, target: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.0[0], target.0[0]), m(c.0[1], target.0[1]), m(c.0[2], target.0[2]))
}

/// 在实色 `fill` 上读得清的墨色（黑或白）。
fn ink_on(fill: Color32) -> Color32 {
    if to_color(Color32::WHITE).contrast(to_color(fill)) >= to_color(Color32::BLACK).contrast(to_color(fill)) {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// 把 `accent` 朝黑压暗，直到与 `bg` 达到 `min` 对比度（浅色面板上用的强调文字，以及
/// 白字菜单悬停行）。
fn darken_to_contrast(accent: Color32, bg: Color32, min: f32) -> Color32 {
    let mut c = accent;
    let mut t: f32 = 0.0;
    while t < 1.0 && contrast(c, bg) < min {
        t = (t + 0.1).min(1.0);
        c = toward(accent, Color32::BLACK, t);
    }
    c
}

/// 把 `accent` 朝白提亮，直到与 `bg` 达到 `min` 对比度（深色面板上的强调文字）。
fn lighten_to_contrast(accent: Color32, bg: Color32, min: f32) -> Color32 {
    let mut c = accent;
    let mut t: f32 = 0.0;
    while t < 1.0 && contrast(c, bg) < min {
        t = (t + 0.1).min(1.0);
        c = toward(accent, Color32::WHITE, t);
    }
    c
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub canvas: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub field: Color32,
    /// A field's hover hairline (`ui-egui`'s `field_hover_border`).
    pub field_hover_border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_on: Color32,
    pub selected_row: Color32,
    /// 选中行及其后代组成的整块底色（egui `selected_row_child`）。
    pub selected_row_child: Color32,
    /// 整块全是组件时的 band 色（egui `component_row`）。
    pub component_row: Color32,
    /// 整块全是图标时的 band 色（egui `icon_layer_row`）。
    pub icon_layer_row: Color32,
    pub guide: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub menu: Color32,
    /// Text on a menu / popover (`ui-egui`'s `menu_text`).
    pub menu_text: Color32,
    /// The hovered menu row's fill (egui uses the accent, not the panel's `hover`).
    pub menu_hover: Color32,
    /// A menu row's greyed shortcut text.
    pub menu_dim: Color32,
    pub shadow: Color32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::dark()
    }
}

impl Tokens {
    /// 为某个中性模式 + 某个品牌强调色构建完整令牌集。取值与 egui 的 `base_light` /
    /// `base_dark` 完全一致（不透明文字、暗色画布 1E1E1E、字段 383838），否则两后端观感会不一致。
    pub fn new(dark: bool, accent: Accent) -> Self {
        let panel = if dark { c(0x2C, 0x2C, 0x2C) } else { c(0xFF, 0xFF, 0xFF) };
        let canvas = if dark { c(0x1E, 0x1E, 0x1E) } else { c(0xF5, 0xF5, 0xF5) };
        let field = if dark { c(0x38, 0x38, 0x38) } else { c(0xF5, 0xF5, 0xF5) };
        let border = if dark { c(0x44, 0x44, 0x44) } else { c(0xE6, 0xE6, 0xE6) };
        let border_strong = if dark { c(0x55, 0x55, 0x55) } else { c(0xD9, 0xD9, 0xD9) };
        let text = if dark { c(0xFF, 0xFF, 0xFF) } else { c(0x1E, 0x1E, 0x1E) };
        let text_secondary = if dark { c(0xB3, 0xB3, 0xB3) } else { c(0x6E, 0x6E, 0x6E) };
        let text_tertiary = if dark { c(0x88, 0x88, 0x88) } else { c(0x8A, 0x8A, 0x8A) };
        let icon = if dark { c(0xF5, 0xF5, 0xF5) } else { c(0x1E, 0x1E, 0x1E) };
        let hover = if dark { c(0x38, 0x38, 0x38) } else { c(0xF5, 0xF5, 0xF5) };
        let pressed = if dark { c(0x44, 0x44, 0x44) } else { c(0xE6, 0xE6, 0xE6) };
        let field_hover_border = if dark { c(0x4D, 0x4D, 0x4D) } else { c(0xE6, 0xE6, 0xE6) };

        let (lc, lc1, dc, dc1) = accent.palette();
        let (color, color1) = if dark { (dc, dc1) } else { (lc, lc1) };
        let accent_on = ink_on(color);
        // Green 沿用 egui 的 ACCENT（0x00875F）；其余品牌色按对比度压暗（菜单悬停行上是白字）。
        let menu_hover = if matches!(accent, Accent::Green) { c(0x00, 0x87, 0x5F) } else { darken_to_contrast(color, Color32::WHITE, 4.5) };
        Self {
            dark,
            bg: panel,
            panel,
            canvas,
            border,
            border_strong,
            text,
            text_secondary,
            text_tertiary,
            icon,
            hover,
            pressed,
            field,
            field_hover_border,
            accent: color,
            accent_hover: color1,
            accent_on,
            // 选中行：向白/黑混合（egui 同式），不是透明叠加。
            selected_row: if dark { toward(color, Color32::BLACK, 0.85) } else { toward(color, Color32::WHITE, 0.86) },
            selected_row_child: if dark { c(0x12, 0x37, 0x2D) } else { c(0xF1, 0xFC, 0xF7) },
            component_row: if dark { c(0x3E, 0x30, 0x52) } else { c(0xF1, 0xE9, 0xFF) },
            icon_layer_row: if dark { c(0x4A, 0x38, 0x1C) } else { c(0xFD, 0xF0, 0xDD) },
            guide: c(0xF2, 0x48, 0x22),
            danger: c(0xF2, 0x48, 0x22),
            success: DEV_READY,
            warning: c(0xFF, 0xCD, 0x29),
            menu: c(0x1E, 0x1E, 0x1E),
            menu_text: c(0xFF, 0xFF, 0xFF),
            menu_hover,
            menu_dim: c(0x9A, 0x9A, 0x9A),
            shadow: if dark { Color32::from_black_alpha(90) } else { Color32::from_black_alpha(40) },
        }
    }

    /// 亮主题，绿强调色 —— 历史默认。
    pub fn light() -> Self {
        Self::new(false, Accent::Green)
    }

    /// 暗主题，绿强调色 —— 历史默认。
    pub fn dark() -> Self {
        Self::new(true, Accent::Green)
    }
}

/// 当前强调色的主色（替代旧 `BRAND` 常量）。
pub fn accent(t: &Tokens) -> Color32 {
    t.accent
}

/// 当前强调色的第二档（hover/pressed；替代旧 `ACCENT` 常量）。
pub fn accent_hover(t: &Tokens) -> Color32 {
    t.accent_hover
}

/// 在强调色实心上读得清的墨色（替代旧 `ACCENT_ON` 常量）。
pub fn accent_on(t: &Tokens) -> Color32 {
    t.accent_on
}

/// 组件层紫色。它是产品层色，不属于可换主题的品牌轴，故固定。
pub const COMPONENT: Color32 = Color32::from_rgb(0x97, 0x47, 0xFF);
/// Handoff 状态徽标（其上白字，两主题皆然）。
pub const DEV_READY: Color32 = Color32::from_rgb(0x08, 0x7D, 0x4F);
pub const DEV_CHANGED: Color32 = Color32::from_rgb(0xA7, 0x5B, 0x00);
pub const DEV_COMPLETED: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x73);

/// `figcraft_color::Color` → 画布 `Color32`。
pub fn doc_c32(c: figcraft_color::Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// 画布叠加层的主色：页面深色用亮强调色，浅色用暗一档（对应 `set_canvas_page`）。
pub fn canvas_accent(page: Color32, accent: Color32, accent_hover: Color32) -> Color32 {
    if contrast(accent, page) > contrast(accent_hover, page) { accent } else { accent_hover }
}

pub fn stroke(color: Color32, width: f32) -> crate::canvas::geom::Stroke {
    crate::canvas::geom::Stroke::new(width, color)
}
