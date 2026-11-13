// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// ⛔⛔⛔ 永久性警告（用户 2026-11-05 硬性要求，任何人不得违反）⛔⛔⛔
//
//   右栏（以及任何用 `PanelSection` 的面板）的**行布局只有一种方式：统一网格**。
//
//   1. 任何一行**必须**用本文件的 `field_row` / `field_row_full` / `field_row_full_trailing` /
//      `field_row_h` 构造，
//      交给 `PanelSection::child`。**禁止**在面板里手写 `div().flex().flex_row()...` 来自排
//      一行——那会绕开共享列，必然错位（历史上就是这么错的）。
//   2. **禁止手算列宽**（`flex_none().w(col)` / `w(pt(SLOT))` / 自建 24pt 占位列 等）。
//      列宽只由本文件决定：`1fr | 1fr | 24`（见下方文档）。要几列就用几个 `fields`。
//   3. 网格**只有一列 24pt**（不是两列！）。多画一个 24 会让两列各少 12pt、整行右移
//      ——这是曾经出现过的真实 bug。
//   4. 需要**跨两行**的控件（如 auto-layout 的 3×3 对齐 picker）用 `field_row_h(fields,
//      trailing, height)`：列仍是同一套，只把行高调大。**禁止**改用自由块绕开网格。
//      **行高必须是 `ROW_H` 的整数倍**（两行 = `ROW_H * 2` = 64）。控件自身高度应比行高
//      小 8 pt（上下各留 4 pt），例如 picker = `FIELD_H*2 + GUTTER` = 56，居中放进 64 的行。
//      写成 56 的行高会让它之后的所有行**整体上移 8 pt**——这是真实发生过的垂直错位。
//   5. 一行里的控件个数：`fields` 1~2 个（自动均分两列），`trailing` 放 24pt 图标按钮
//      （每个 24pt，网格会按个数给出 24×n 的列）。不要再往行里塞第三个字段。
//
//   违反以上任意一条 = 破坏整个右栏的对齐，属于**不可接受的回归**。改面板前先读本节。
// ============================================================================
//! `Panel Section` — a layout scheme built from `Section Header`.
//!
//! Mirrors the `.fig`'s `Panel Section`: the 40 pt header on top, then a **single unified grid**
//! body. The body is ONE grid with one set of shared column tracks — not a vertical stack of
//! independent per-row grids — so every row's fields stay in the same columns.
//!
//! The unified grid, left → right:
//!
//! ```text
//! [ field 1fr ] [ field 1fr ] [ 24 pt ]
//! ```
//!
//! * **Field columns** — `flex_1().min_w(0)` for each, i.e. every field column is a `1fr` that
//!   shares the space *equally*. A normal section has **two** equally-divided field columns
//!   (e.g. `X | Y`, `W | H`); a row that supplies only one field simply leaves the second empty.
//! * **Exactly ONE fixed 24 pt column** — `flex_none().w(SLOT * n)`, holding the row's right-hand
//!   actions (the `.fig`'s 24×24 icon buttons). It is *always* present: blank when the row has no
//!   trailing element, otherwise one 24 per trailing element. **There is only one such column —
//!   never a separate "blank 24" plus a "trailing 24".**
//!
//! GPUI 0.2.1's `grid()` only emits uniform columns (`grid_cols(n)` → equal `1fr`), so it cannot
//! express a fixed-width shared column. We therefore build the unified grid **column-major**: a
//! single `flex_row` holding, for each design column, a `flex_col` of that column's cells
//! (`flex_1().min_w(0)` → `1fr`), followed by the fixed 24 pt blank column and the optional fixed
//! 24 pt trailing column. Because the columns are siblings in one row, their tracks are shared and
//! every row aligns — that is the unified grid.
//!
//! ## Full-width rows
//!
//! `ui-egui` lays a dropdown out at `col * 2.0 + 8.0` — both field columns plus their gutter, in a
//! plain `row()` that spans the panel's inner width. A column-major grid cannot let one cell span
//! two columns, so such a row is kept out of the grid and appended under it ([`field_row_full`]):
//! same `PAD .. PAD_RIGHT` inner rect, same `ROW_H`, so it lines up with the grid's rows while
//! using the full `col * 2 + GRID_GAP` width the original gives it.

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::section_header::{SectionHeader, SectionHeaderVariant};
use crate::widgets::{FIELD_H, GRID_GAP, PAD, PAD_RIGHT, RADIUS, ROW_H, SECTION_BOTTOM, SLOT, dbg_frame, pt, rgba, tokens};
use gpui::{
    AnyElement, App, Bounds, Div, Edges, IntoElement, MouseButton, MouseDownEvent, Rgba, SharedString, TextRun, Window, canvas, div, point,
    prelude::*, px, size,
};
use std::rc::Rc;

/// One body row: its fields (the `1fr` columns of the unified grid) and optional trailing controls.
pub struct FieldRow {
    fields: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
    /// How many of the two field columns the row takes: 1 (a normal row) or 2 (a dropdown or a
    /// `Field-color`, which `ui-egui` lays out at `col * 2.0 + 8.0`). The 24 pt columns stay.
    span: usize,
    /// A row that is not a field track at all — a caption (`sublabels`) or a section-level block.
    /// It goes into the body as it is, with no columns of its own, exactly like `ui-egui`'s
    /// `ui.label` between two `row()`s.
    free: Option<AnyElement>,
    /// Override the row height (`None` = the standard 32 pt `ROW_H`). 用于跨两行的 auto-layout
    /// picker：仍走同一套列（`1fr | 1fr | 24 | 24`），只把行高调成 `FIELD_H*2 + GUTTER`。
    height: Option<f32>,
}

/// Build a body row. `fields` become the `1fr` columns of the unified grid; `trailing` go into the
/// shared 24 pt trailing column on the right-hand side.
pub fn field_row(fields: Vec<impl IntoElement>, trailing: Vec<impl IntoElement>) -> FieldRow {
    FieldRow {
        fields: fields.into_iter().map(|f| f.into_any_element()).collect(),
        trailing: trailing.into_iter().map(|t| t.into_any_element()).collect(),
        span: 1,
        free: None,
        height: None,
    }
}

/// A row whose element spans **both** field columns — `col * 2.0 + 8.0`, the width `ui-egui` gives
/// a dropdown and a `Field-color`. It is still a grid row: the 24 pt blank and trailing columns
/// stay on its right, so it lines up with every other row.
pub fn field_row_full(el: impl IntoElement) -> FieldRow {
    FieldRow { fields: vec![el.into_any_element()], trailing: Vec::new(), span: 2, free: None, height: None }
}

/// A row whose element spans **both** field columns **and** keeps trailing 24 pt icon cells.
/// `ui-egui` builds a paint / effect / grid row as one field of `available_width - 56` followed
/// by an `icon_pair` (two 24 pt buttons 4 apart); on the unified grid that is exactly one
/// spanning field plus a `SLOT * 2` trailing column, so the icons share the same right edge as
/// every other row's trailing column. Nothing is hand-measured here.
pub fn field_row_full_trailing(el: impl IntoElement, trailing: Vec<impl IntoElement>) -> FieldRow {
    FieldRow {
        fields: vec![el.into_any_element()],
        trailing: trailing.into_iter().map(|t| t.into_any_element()).collect(),
        span: 2,
        free: None,
        height: None,
    }
}

/// A row on the **same unified grid** as [`field_row`], but `height` points tall instead of the
/// standard 32 pt. 用于 auto-layout 的 3×3 对齐 picker（一列跨两行）：列宽仍是 `1fr | 1fr | 24 | 24`，
/// 所以它和上面的 W/H 行、下面的 Padding 行严格对齐。
pub fn field_row_h(fields: Vec<impl IntoElement>, trailing: Vec<impl IntoElement>, height: f32) -> FieldRow {
    FieldRow {
        fields: fields.into_iter().map(|f| f.into_any_element()).collect(),
        trailing: trailing.into_iter().map(|t| t.into_any_element()).collect(),
        span: 1,
        free: None,
        height: Some(height),
    }
}

/// A caption row — `ui-egui`'s `widgets::sublabels(ui, "Alignment", None)`: 12 pt tall, no columns.
pub fn sublabel_row(text: impl Into<SharedString>) -> FieldRow {
    FieldRow { fields: Vec::new(), trailing: Vec::new(), span: 0, free: Some(sublabels(text).into_any_element()), height: None }
}

/// A panel section: the reserved header plus a body of rows laid out as one unified grid.
pub struct PanelSection {
    title: SharedString,
    variant: SectionHeaderVariant,
    header_lead: Option<AnyElement>,
    header_trail: Option<AnyElement>,
    bg: Option<Rgba>,
    rows: Vec<FieldRow>,
}

impl PanelSection {
    /// A section titled `title`, `Normal` header by default.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), variant: SectionHeaderVariant::Normal, header_lead: None, header_trail: None, bg: None, rows: Vec::new() }
    }

    /// Grey the header (an empty section).
    pub fn variant(mut self, variant: SectionHeaderVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Leading slot for the header (drawn left of the title).
    pub fn header_lead(mut self, el: impl IntoElement) -> Self {
        self.header_lead = Some(el.into_any_element());
        self
    }

    /// Trailing slot for the header (e.g. an `+` / chevron icon-button group).
    pub fn header_trail(mut self, el: impl IntoElement) -> Self {
        self.header_trail = Some(el.into_any_element());
        self
    }

    /// A solid background (the design's white panel body; in the dark app use `t.panel`).
    pub fn background(mut self, bg: Rgba) -> Self {
        self.bg = Some(bg);
        self
    }

    /// Append one body row (built with [`field_row`]).
    pub fn child(mut self, row: FieldRow) -> Self {
        self.rows.push(row);
        self
    }
}

/// Convenience constructor matching the egui `section()` call shape.
pub fn panel_section(title: impl Into<SharedString>) -> PanelSection {
    PanelSection::new(title)
}

/// 把若干 `FieldRow` 铺成统一网格的行容器。`PanelSection` 和「无标题的内容块」共用这一份，
/// 免得调用方为了塞进 `Div::child` 又手抄一遍列宽。
fn render_rows(rows: Vec<FieldRow>) -> Div {
    let mut body = div().flex().flex_col().w_full().gap(pt(0.0));
    for row in rows {
        let FieldRow { fields, trailing, span, free, height } = row;
        if let Some(el) = free {
            body = body.child(el);
            continue;
        }
        let row_h = height.unwrap_or(ROW_H);
        let mut line =
            dbg_frame(div().flex().flex_row().items_center().w_full().h(pt(row_h)).gap(pt(GRID_GAP)).pl(pt(PAD)).pr(pt(PAD_RIGHT)), 0x0A84FF);
        let supplied = fields.len();
        for f in fields {
            line = line.child(dbg_frame(div().flex_1().min_w(pt(0.0)).h_full().flex().items_center().child(f), 0x30D158));
        }
        if span < 2 {
            for _ in supplied..2 {
                line = line.child(dbg_frame(div().flex_1().min_w(pt(0.0)).h_full(), 0x30D158));
            }
        }
        // **One** 24 pt column per trailing element — a separate "blank 24" plus "trailing 24"
        // would shift every row. Each trailing element gets its OWN `SLOT` (24 pt) cell, centred:
        // that is the `field_row_full_trailing` contract ("two 24 pt buttons 4 apart"). Without
        // per-element slots, two 20 pt icons sit flush (centres 20 apart) instead of 24 apart,
        // losing the 4 pt gutter — the clip row's eye/minus looked stuck together (user 2026-11-12).
        // Single-icon rows are unchanged (their icon just centres in the same 24 pt).
        let count = trailing.len().max(1) as f32;
        let mut cell = div().flex_none().w(pt(SLOT * count)).h_full().flex().flex_row().items_center();
        for t in trailing {
            cell = cell.child(dbg_frame(div().w(pt(SLOT)).h_full().flex().items_center().justify_center().child(t), 0xBF5AF2));
        }
        line = line.child(cell);
        body = body.child(line);
    }
    body
}

/// 一行也能独立成一个元素：右栏的 `header()` / `section()` 刻意不要区块标题，
/// 直接把 `field_row(...)` 塞进 `Div::child`（以前 `FieldRow` 只能喂给
/// `PanelSection::child`，于是这两处一直编译不过）。
impl IntoElement for FieldRow {
    type Element = Div;

    fn into_element(self) -> Div {
        render_rows(vec![self])
    }
}

impl IntoElement for PanelSection {
    type Element = Div;

    fn into_element(self) -> Div {
        let mut header = SectionHeader::new(self.title.clone()).variant(self.variant);
        if let Some(lead) = self.header_lead {
            header = header.lead(lead);
        }
        if let Some(trail) = self.header_trail {
            header = header.trail(trail);
        }

        let body = render_rows(self.rows);

        // `SECTION_BOTTOM` (12 pt) is the shared control's gap between the rows and the divider
        // that `right_panel` draws after each section.
        let body = div().w_full().pb(pt(SECTION_BOTTOM)).child(body);

        let mut el = div().flex().flex_col().w_full().gap(pt(0.0));
        if let Some(bg) = self.bg {
            el = el.bg(bg);
        }
        el.child(header).child(body)
    }
}

/// Paint a 10 pt caption label, bottom-left inside its 12 pt cell, and — under `FC_DEBUG_LAYOUT` —
/// outline the **glyph** box (not the line box), so it matches the slot leads (`Input::slot_child`)
/// and the section title. The old `dbg_frame(div().child(text))` wrapped the text element's line box,
/// which carries leading above the glyph, so the red frame sat wide of the ink.
fn caption_label(text: SharedString) -> AnyElement {
    let t = tokens();
    let color = rgba(t.text_secondary).into();
    let dbg = std::env::var_os("FC_DEBUG_LAYOUT").is_some();
    canvas(
        move |bounds, window, _| {
            let run =
                TextRun { len: text.len(), font: window.text_style().font(), color, background_color: None, underline: None, strikethrough: None };
            let line = window.text_system().shape_line(text.clone(), px(crate::widgets::CAPTION_PT), &[run], None);
            // Bottom-align the line box in the 12 pt caption cell, exactly as `items_end` did before.
            let line_h = line.ascent + line.descent;
            let origin_y = bounds.origin.y + bounds.size.height - line_h;
            (line, px(crate::widgets::CAPTION_PT), bounds.origin.x, origin_y, bounds.size.height, line_h)
        },
        move |_bounds, (line, fs, x, origin_y, _cell_h, line_h), window, _cx| {
            let _ = line.paint(point(x, origin_y), line_h, window, _cx);
            if dbg {
                let center = origin_y + line_h * 0.5;
                let gh = line
                    .runs
                    .first()
                    .and_then(|run| window.text_system().typographic_bounds(run.font_id, fs, '0').ok())
                    .map(|g| g.size.height)
                    .unwrap_or(line_h);
                let gy = center - gh * 0.5;
                let mut boxq = gpui::fill(Bounds { origin: point(x, gy), size: size(line.width, gh) }, gpui::transparent_black());
                boxq.border_widths = Edges::all(px(1.0));
                boxq.border_color = gpui::rgb(0xFF2D55).into();
                window.paint_quad(boxq);
            }
        },
    )
    .size_full()
    .into_any_element()
}

/// The grid's caption row above a field row (`widgets::sublabels`): a 10 pt caption, 12 pt tall,
/// `t.text_secondary`, left-aligned on the panel's 16 pt inset. The caption's glyph gets `pt(4)` of
/// breathing room above it (the user wants the block opened up a little), and the debug border
/// (only with `FC_DEBUG_LAYOUT`) hugs the glyph itself — not a box around the whole 16 pt-inset strip.
pub fn sublabels(text: impl Into<SharedString>) -> Div {
    let t = tokens();
    div()
        .h(px(12.0))
        .mt(pt(4.0))
        .w_full()
        .flex()
        .items_end()
        .pl(px(PAD))
        .pr(px(PAD_RIGHT))
        .text_size(px(crate::widgets::CAPTION_PT))
        .text_color(rgba(t.text_secondary))
        .child(caption_label(text.into()))
}

/// A two-column caption row (`widgets::sublabels(ui, a, Some(b))`): the left and right labels sit
/// above the two field columns of the row below them — e.g. Appearance's "Opacity" / "Corner radius".
/// Each label gets its own glyph-tight red debug frame when `FC_DEBUG_LAYOUT` is set, and the row
/// keeps `pt(4)` of breathing room above the glyphs.
pub fn sublabels2(left: impl Into<SharedString>, right: impl Into<SharedString>) -> Div {
    let cap = |s: SharedString| div().flex_1().min_w(px(0.0)).child(caption_label(s));
    div()
        .h(px(12.0))
        .mt(pt(4.0))
        .w_full()
        .flex()
        .flex_row()
        .items_end()
        .gap(pt(GRID_GAP))
        .pl(px(PAD))
        .pr(px(PAD_RIGHT))
        .child(cap(left.into()))
        .child(cap(right.into()))
        // The same fixed 24 pt blank column every field row keeps after its two field columns
        // (`panel_section.rs` line 175), so the two captions share the field rows' tracks instead
        // of overrunning the right inset by `GRID_GAP + SLOT` and landing on `PAD_RIGHT`.
        .child(div().flex_none().w(pt(SLOT)))
}

/// A two-column caption row, wrapped as a [`FieldRow`] free row.
pub fn sublabel_row2(left: impl Into<SharedString>, right: impl Into<SharedString>) -> FieldRow {
    FieldRow { fields: Vec::new(), trailing: Vec::new(), span: 0, free: Some(sublabels2(left, right).into_any_element()), height: None }
}

/// A segmented control (`widgets::segmented`): `items.len()` equal cells on a `t.field` bar that
/// fills the unified-grid column it is placed in (like every other field — `w_full`, not a
/// hand-computed width), so it stays aligned with the text fields in neighbouring rows. The active
/// cell is raised on `t.panel` with a hairline; a hovered cell sits on `t.hover`.
pub fn segmented(items: &[(Icon, &'static str)], active: Option<usize>, on_pick: Option<Rc<dyn Fn(usize, &mut Window, &mut App)>>) -> Div {
    let t = tokens();
    let mut bar = div().w_full().h(px(FIELD_H)).rounded(px(RADIUS)).bg(rgba(t.field)).flex().flex_row().items_center();
    for (i, (icon, _tip)) in items.iter().enumerate() {
        let on = active == Some(i);
        let mut c = div().flex_1().min_w(px(0.0)).h(px(FIELD_H)).flex().items_center().justify_center();
        if on {
            c = c.p(px(2.0)).child(
                div()
                    .size_full()
                    .rounded(px(4.0))
                    .bg(rgba(t.panel))
                    .border_1()
                    .border_color(rgba(t.border))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(paint_icon(*icon, rgba(t.icon), 14.0)),
            );
        } else {
            c = c.rounded(px(4.0)).hover(|s| s.bg(rgba(t.hover))).child(paint_icon(*icon, rgba(t.text_secondary), 14.0));
        }
        if let Some(f) = on_pick.clone() {
            c = c.cursor_pointer().on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| f(i, window, cx));
        }
        bar = bar.child(c);
    }
    bar
}
