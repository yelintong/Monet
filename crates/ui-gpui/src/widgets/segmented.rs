//! `Segmented` — the shared multi-choice selector.
//!
//! Ported from the `Segmented` component in the design file (`Monet-UI-ui-vars.fig`,
//! Components page, component set `Segmented`): one 24 pt bar, equal-width cells, and a
//! **raised** cell for the current choice.
//!
//! The design ships four variants (`Size=48/88/184` × `State=Rest/Active`); the geometry is the
//! same in all of them, only the cell count differs:
//!
//! * bar: 24 pt tall, radius 5, fill `field` (`#F5F5F5`), no border, cells laid out with no gap;
//! * cell: full bar height, radius 4, **no fill** when at rest;
//! * the chosen cell: `panel` fill (`#FFFFFF`) + 1 pt inside hairline in `border` (`#E6E6E6`);
//! * the glyph keeps the same colour in both states — the raised cell alone says which one is on
//!   (rest glyphs are black at 90 %, i.e. the panel's `icon` colour).
//!
//! **No choice** is a real state, not a missing feature: `active(None)` draws every cell at rest.
//! Everything that only sometimes has a value (a mixed selection, a property the current node
//! does not have) uses it, exactly like the design's `State=Rest` master.
//!
//! Cells hold either a glyph or a short text label — the design's masters are icon-only, and the
//! text form is the same control with the icon slot replaced (the settings popovers need words:
//! `On`/`Off`, `Solid`/`Dotted`/`Wavy`, …).

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{FIELD_H, RADIUS, TEXT_PT, pt, rgba, tokens};
use gpui::{AnyElement, App, Div, MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*, px};
use std::rc::Rc;

/// Radius of the raised cell (the bar is `RADIUS` = 5; the design uses 4 inside it).
const CELL_RADIUS: f32 = 4.0;

/// One cell of a [`Segmented`].
#[derive(Clone, Debug, PartialEq)]
pub enum SegItem {
    /// A glyph, centred in the cell (the design's masters).
    Icon(Icon),
    /// A short word, centred in the cell (the text form of the same control).
    Text(SharedString),
    /// A dialog typography glyph (U̲ / S̶ / AG / list bullets …), loaded through the `typ:`
    /// asset prefix. Same geometry as `Icon`, only the SVG source differs — see `typ_icons`.
    Svg(crate::widgets::typ_icons::TypIcon),
}

impl SegItem {
    pub fn icon(i: Icon) -> Self {
        Self::Icon(i)
    }
    pub fn text(s: impl Into<SharedString>) -> Self {
        Self::Text(s.into())
    }
    /// A typography-dialog glyph, from the lizard `typ-*.svg` set.
    pub fn svg(i: crate::widgets::typ_icons::TypIcon) -> Self {
        Self::Svg(i)
    }
}

type PickHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The shared multi-choice selector. See the module docs for the design it ports.
pub struct Segmented {
    items: Vec<SegItem>,
    active: Option<usize>,
    height: f32,
    /// Stretch to the parent's width (panel rows) instead of one 24 pt cell per item.
    fill: bool,
    /// 固定整条 bar 的总宽（Type settings 对话框里每条是定宽：对齐 96、装饰 72、大小写 120…）。
    /// `Some` 时忽略 `fill`，格子仍 `flex_1` 均分这段定宽。
    fixed_w: Option<f32>,
    on_pick: Option<PickHandler>,
}

impl Segmented {
    /// A selector over `items`, filling the parent's width.
    pub fn new(items: Vec<SegItem>) -> Self {
        Self { items, active: None, height: FIELD_H, fill: true, fixed_w: None, on_pick: None }
    }

    /// 构造一组排版对话框字形（4 个对齐、3 个装饰线……）。格宽由 `.fixed_bar()` 决定。
    pub fn glyphs(items: &[crate::widgets::typ_icons::TypIcon]) -> Self {
        Self::new(items.iter().copied().map(SegItem::Svg).collect())
    }

    /// 固定整条 bar 的总宽；格子 `flex_1` 均分。用于对话框里 lizard 指定的定宽分段条。
    pub fn fixed_bar(mut self, w: f32) -> Self {
        self.fixed_w = Some(w);
        self.fill = false;
        self
    }

    /// The icon form: `(icon, tooltip)`. The tooltip is accepted for call-site parity with the
    /// old `panel_section::segmented`; the control has no tooltip layer of its own yet.
    pub fn icons(items: &[(Icon, &'static str)]) -> Self {
        Self::new(items.iter().map(|(i, _)| SegItem::Icon(*i)).collect())
    }

    /// Which cell is raised. `None` = nothing selected (the design's `State=Rest` master).
    pub fn active(mut self, index: Option<usize>) -> Self {
        self.active = index;
        self
    }

    /// Cell height; 24 by default (the design's only size in the panel).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    /// Size each cell to the bar's height instead of stretching (`n × 24` wide).
    pub fn hugging(mut self) -> Self {
        self.fill = false;
        self
    }

    /// Called with the picked cell's index. Cells are only clickable when this is set.
    pub fn on_pick(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }

    /// Build the element (`Div` so callers can `.flex_none()` / `.w(...)` it like any row child).
    pub fn build(self) -> Div {
        let t = tokens();
        let mut bar = div()
            .h(pt(self.height))
            .rounded(pt(RADIUS))
            .bg(rgba(t.field))
            .flex()
            .flex_row()
            .items_center();
        if let Some(w) = self.fixed_w {
            bar = bar.w(pt(w)).flex_none();
        } else if self.fill {
            bar = bar.w_full();
        }
        for (i, item) in self.items.iter().enumerate() {
            let chosen = self.active == Some(i);
            let mut cell = div()
                .h_full()
                .rounded(pt(CELL_RADIUS))
                .flex()
                .items_center()
                .justify_center()
                // 等宽格：`flex_1` 时按份数分，`hugging` 时每格一个 24 pt 方。
                .min_w(px(0.0))
                .overflow_hidden();
            cell = if self.fill || self.fixed_w.is_some() { cell.flex_1() } else { cell.w(pt(self.height)) };
            // 激活格「抬起」：白底 + 1 pt 内侧描边（设计里两种状态的图标同色，靠这块白底示意）。
            if chosen {
                cell = cell.bg(rgba(t.panel)).border_1().border_color(rgba(t.border));
            } else if self.on_pick.is_some() {
                // 设计没有 hover 变体；用 pressed 做一层极浅的水洗，让可点的格子有反馈。
                cell = cell.hover(|s| s.bg(rgba(t.pressed)));
            }
            let content: AnyElement = match item {
                SegItem::Icon(icon) => {
                    let glyph = if self.height <= 21.0 { 12.0 } else { crate::widgets::ICON };
                    paint_icon(*icon, rgba(t.icon), glyph).into_any_element()
                }
                SegItem::Svg(typ) => {
                    let glyph = if self.height <= 21.0 { 14.0 } else { 16.0 };
                    super::typ_icons::paint(*typ, rgba(t.icon), glyph).into_any_element()
                }
                SegItem::Text(s) => div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(pt(TEXT_PT - 1.5))
                    .text_color(rgba(t.text))
                    .child(s.clone())
                    .into_any_element(),
            };
            cell = cell.child(content);
            if let Some(f) = self.on_pick.clone() {
                cell = cell.cursor_pointer().on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| f(i, window, cx));
            }
            bar = bar.child(cell);
        }
        bar
    }
}

impl IntoElement for Segmented {
    type Element = Div;

    fn into_element(self) -> Self::Element {
        self.build()
    }
}
