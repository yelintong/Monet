// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! The one menu control — a floating menu card anchored to its trigger.
//!
//! [`Menu`] is the single primitive every menu is built on: dropdowns, the multi-level main menu,
//! the canvas context menu, the bottom toolbar's flyouts and the Actions palette (as well as the
//! small editor popovers, which use [`Menu::plain`]). It owns only what a menu must: where it
//! anchors, how wide it may become, how it is aligned against the anchor, the content to draw,
//! and the viewport it must not overflow. It keeps no response and touches no business data.
//!
//! The three width policies are the original's `Width`:
//!
//! * [`Width::Fit`] — as wide as its widest row;
//! * [`Width::Min`] — at least `w` (a wider row still wins, so a fixed width never clips);
//! * [`Width::Anchor`] — the width of the rect it is anchored to.
//!
//! Placement follows the original's `RectAlign`: `BottomStart` puts the card's top-left on the
//! anchor's bottom-left, `gap` points between them.

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{pt, rgba, tokens};
use gpui::{AnyElement, Bounds, BoxShadow, Div, ElementId, Pixels, Point, SharedString, div, point, prelude::*, px};

/// How wide a [`Menu`] ends up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    /// As wide as its widest row.
    Fit,
    /// At least `w`; a wider row still wins (so a fixed width never clips its content).
    Min(f32),
    /// The width of the rect it is anchored to — a dropdown matching the field above it.
    Anchor,
}

impl From<f32> for Width {
    fn from(w: f32) -> Self {
        Self::Min(w)
    }
}

/// Where the card sits against its anchor (`egui::RectAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectAlign {
    TopStart,
    TopCenter,
    TopEnd,
    BottomStart,
    BottomCenter,
    BottomEnd,
    LeftStart,
    RightStart,
    Center,
}

/// A generic floating layer.
pub struct Menu {
    anchor: Bounds<Pixels>,
    width: Width,
    align: RectAlign,
    gap: f32,
    menu: bool,
    /// 视口尺寸；>0 时开启上下防溢出与水平贴边。
    vw: f32,
    vh: f32,
    /// 最宽行的内容宽 / 行高总和 / 单行高 —— `Fit` 档据此给**确定宽**，最长项才不会被裁。
    content_w: f32,
    rows_h: f32,
    row_h: f32,
    tag: String,
    measured: bool,
    /// 不进滚动视口的元素（子菜单等，窗口绝对定位）。
    extras: Vec<AnyElement>,
}

impl Menu {
    /// Anchored to `anchor`, as wide as it, `BOTTOM_START` with the design's 4 pt gap — the
    /// defaults the original sets.
    pub fn at(anchor: Bounds<Pixels>) -> Self {
        Self {
            anchor,
            width: Width::Anchor,
            align: RectAlign::BottomStart,
            gap: 4.0,
            menu: true,
            vw: 0.0,
            vh: 0.0,
            content_w: 0.0,
            rows_h: 0.0,
            row_h: crate::menu_metrics::MENU_ROW_H,
            tag: "popup".into(),
            measured: false,
            extras: Vec::new(),
        }
    }

    pub fn width(mut self, w: impl Into<Width>) -> Self {
        self.width = w.into();
        self
    }

    pub fn align(mut self, align: RectAlign) -> Self {
        self.align = align;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// A float with a plain frame (the colour picker's own popover draws its own chrome).
    pub fn plain(mut self) -> Self {
        self.menu = false;
        self
    }

    /// 视口尺寸：开启上下防溢出 + 水平贴边。
    pub fn viewport(mut self, vw: f32, vh: f32) -> Self {
        self.vw = vw;
        self.vh = vh;
        self
    }

    /// 内容测量值：最宽行的内容宽、行高总和、单行高。`Fit` 档据此给确定宽，
    /// 于是「以最宽那一项为标准，其它项左右填充」由这一个控件统一完成。
    pub fn content(mut self, content_w: f32, rows_h: f32, row_h: f32) -> Self {
        self.content_w = content_w;
        self.rows_h = rows_h;
        self.row_h = row_h;
        self.measured = true;
        self
    }

    /// 滚动视口的唯一 id 前缀（多级菜单会同时出现多个）。
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }

    /// 不进滚动视口的绝对定位元素（子菜单）。
    pub fn extras(mut self, extras: Vec<AnyElement>) -> Self {
        self.extras = extras;
        self
    }

    /// Draws the card at its anchor and hands the content a body to fill; `None` when the popup
    /// has no anchor (a point anchor with nothing to hang off).
    pub fn show(self, content: impl FnOnce() -> AnyElement) -> Option<AnyElement> {
        let aw = f32::from(self.anchor.size.width);
        let ah = f32::from(self.anchor.size.height);
        let ax = f32::from(self.anchor.origin.x);
        let ay = f32::from(self.anchor.origin.y);
        if aw <= 0.0 && ah <= 0.0 {
            return None;
        }
        let (left, top) = match self.align {
            RectAlign::BottomStart => (ax, ay + ah + self.gap),
            RectAlign::BottomCenter => (ax + aw / 2.0, ay + ah + self.gap),
            RectAlign::BottomEnd => (ax + aw, ay + ah + self.gap),
            RectAlign::TopStart => (ax, ay - self.gap),
            RectAlign::TopCenter => (ax + aw / 2.0, ay - self.gap),
            RectAlign::TopEnd => (ax + aw, ay - self.gap),
            RectAlign::LeftStart => (ax - self.gap, ay),
            RectAlign::RightStart => (ax + aw + self.gap, ay),
            RectAlign::Center => (ax + aw / 2.0, ay + ah / 2.0),
        };
        // 确定宽：内容宽 + 外框左右各 8。旧调用没给测量值时保持原来的不定宽 / min_w 行为。
        let width = if self.measured {
            let natural = self.content_w + 2.0 * PAD;
            Some(match self.width {
                Width::Fit => natural,
                Width::Min(w) => natural.max(w),
                Width::Anchor => natural.max(aw.max(48.0)),
            })
        } else {
            None
        };
        let left = match (self.vw > 0.0, width) {
            (true, Some(w)) => left.min((self.vw - w - PAD).max(PAD)).max(PAD),
            _ => left,
        };
        let (top, viewport_h, scroll) = if self.vh > 0.0 { fit_vertical(top, self.rows_h, self.vh, self.row_h) } else { (top, self.rows_h, false) };

        let mut card = if self.menu { menu_surface() } else { div() }.absolute().left(pt(left)).top(pt(top));
        card = match width {
            Some(w) => card.w(pt(w)),
            None => match self.width {
                Width::Fit => card,
                Width::Min(w) => card.min_w(pt(w)),
                Width::Anchor => card.min_w(pt(aw)),
            },
        };
        if scroll {
            // 底部 8pt 让位给 24pt 箭头条：不是 8+24=32，而是「原本 8 变成 24」。
            card = card.pb(pt(0.0));
        }
        if self.menu {
            card = card.occlude();
        }
        let body = content();
        Some(card.children(fit_rows(body, viewport_h, scroll, &self.tag)).children(self.extras).into_any_element())
    }
}

/// 卡片外框内边距（仅垂直 `py(pt(8))`；水平 8pt 已移到每行，好让分隔线通到卡片边缘）。
const PAD: f32 = crate::menu_metrics::MENU_PAD;
/// 溢出时底部原本 8pt 的空白变成 24pt 的箭头条。
const AFFORDANCE: f32 = 24.0;

/// 通用上下溢出保护：顶部不低于 8；内容放得下就整体上移给底部留 8；放不下就卡在屏幕底
/// 并内部滚动。`rows_h` 是不含外框 padding 的纯行高总和。
fn fit_vertical(top: f32, rows_h: f32, vh: f32, row_h: f32) -> (f32, f32, bool) {
    let max_top = (vh - rows_h - 2.0 * PAD - PAD).max(PAD);
    let top = top.clamp(PAD, max_top.max(PAD));
    let avail = (vh - top - 2.0 * PAD).max(row_h);
    let scroll = rows_h > avail + 0.5;
    // 滚动时底部 padding 被 `.pb(0)` 去掉，所以视口 = 可用高 - 8 - 24。
    let viewport_h = if scroll { (vh - top - PAD - AFFORDANCE).max(row_h) } else { rows_h };
    (top, viewport_h, scroll)
}

/// 不滚动就直接铺；滚动则套一层隐藏滚动条的视口 + 24pt 居中向下箭头，不出现拖动滑条。
fn fit_rows(body: AnyElement, viewport_h: f32, scroll: bool, tag: &str) -> Vec<AnyElement> {
    if !scroll {
        return vec![body];
    }
    let t = tokens();
    let viewport = div()
        .id(ElementId::from(SharedString::from(format!("popup-viewport-{tag}"))))
        .h(pt(viewport_h))
        .overflow_y_scroll()
        .scrollbar_width(px(0.0))
        .child(body)
        .into_any_element();
    let aff = div()
        .h(pt(AFFORDANCE))
        .flex()
        .items_center()
        .justify_center()
        .child(paint_icon(Icon::ChevronDown, rgba(t.menu_dim), 24.0))
        .into_any_element();
    vec![viewport, aff]
}

/// 直接给坐标的锚点（1×0），供「按当前触发按钮 bounds 每帧重算」的弹窗使用。
pub fn at_point(x: f32, y: f32) -> Bounds<Pixels> {
    Bounds { origin: point(pt(x), pt(y)), size: gpui::size(pt(1.0), pt(0.0)) }
}

/// The surface **every** menu-like card is drawn on — dropdown popups ([`Menu`]), the main
/// menu's levels, the canvas context menu, the toolbar's flyouts and the Actions palette.
///
/// It is the design's `menu_frame` (`ui-egui`'s `menus/tree.rs`): `fill(t.menu)`, 13 pt corners,
/// an 8 pt inset and the menu shadow (offset y 4, blur 16, black α60). **No stroke** — the user
/// removed the hairline on 2026-11-06, and the original never drew one; with `t.menu` and `t.canvas`
/// the same colour in the dark theme, the shadow is what makes a card read as a card.
///
/// One definition, so a change here lands on every popup at once: nothing else may draw a menu's
/// chrome (a row's own 5 pt hover fill is `widgets/menu.rs` / `menu_card`'s business).
pub fn menu_surface() -> Div {
    let t = tokens();
    div().flex().flex_col().py(pt(PAD)).bg(rgba(t.menu)).rounded(pt(13.0)).shadow(vec![BoxShadow {
        color: rgba(crate::canvas::geom::Color32::from_black_alpha(60)).into(),
        offset: point(px(0.0), px(4.0)),
        blur_radius: px(16.0),
        spread_radius: px(0.0),
    }])
}

/// Open a menu anchored to `anchor` (the trigger's bounds) — [`Menu::at`].
pub fn menu(anchor: Bounds<Pixels>) -> Menu {
    Menu::at(anchor)
}

/// The anchor a `Point` gives: a zero-sized rect — for a popup that opens at the pointer.
pub fn point_anchor(p: Point<Pixels>) -> Bounds<Pixels> {
    Bounds { origin: p, size: gpui::size(pt(0.0), pt(0.0)) }
}
