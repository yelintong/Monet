// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! `Icon Button` — implemented directly (click + style + inner icon), not reusing the existing
//! button.
//!
//! The `.fig` ships four states: `Default` / `Hover` / `Pressed` / `Active`. **The `Default` fill
//! (`#E96161`) is a debug placeholder and must be ignored** — a real icon button starts transparent
//! and only paints a background on interaction. So the mapping is:
//!   * no interaction      → transparent
//!   * hover               → `t.hover`      (`#383838`)
//!   * mouse-down / press  → `t.pressed`    (`#454545`)
//!   * `active` (open/sel) → `t.selected_row` (`#173F36`, brand-tinted)
//!
//! The 16 pt glyph is centred; compact (≤21 pt) buttons draw it at 12 pt like the egui original.
//!
//! `active` (the pressed-style pseudo-class) is only available on a *stateful* element, so every
//! button carries a stable `ElementId`; callers may override it, otherwise a process-unique id is
//! assigned so hover/press state resolves correctly.

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{ICON, RADIUS, SLOT, pt, rgba, tokens};
use gpui::{App, ClickEvent, Div, ElementId, IntoElement, Stateful, Window, div, prelude::*};

/// The click callback; kept as an alias so the struct field stays readable.
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

std::thread_local! {
    /// Per-frame sequence used to hand every icon button a *stable* `ElementId`.
    ///
    /// 根因（2026-11-05 实锤）：`render()` 每帧都重建按钮，而旧实现每帧 `fetch_add` 一个新 id
    /// → 同一个按钮每帧 id 都不同。GPUI 点击靠 mouse-down / mouse-up 之间比对元素 `ElementId`，
    /// id 每帧变就永远对不上 → `on_click` **永不触发**，表现为「按钮点了没反应」。右栏 22 个
    /// icon 按钮里只有显式 `.id(...)` 的那颗能点，正是这个原因。改成每帧从 0 重新编号、按
    /// 构建顺序分配，同一状态下每帧得到同一组 id（稳定），同帧内彼此不同（不撞、不串 hover）。
    static ICON_BTN_SEQ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reset the per-frame id sequence. Call once at the very top of the frame's `render`, before any
/// icon button is built, so the ids are deterministic per frame (hence stable across frames for an
/// unchanged UI).
pub(crate) fn reset_icon_btn_ids() {
    ICON_BTN_SEQ.with(|c| c.set(0));
}

/// The next stable id for an icon button built this frame.
fn next_stable_id() -> ElementId {
    let n = ICON_BTN_SEQ.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    ElementId::from(("icon-btn", n))
}

/// A 24×24 (or 32×32) icon button drawn straight from the design: a rounded square that lights up
/// on hover/press/active, with a 16 pt glyph in the middle and an `on_click` handler.
///
/// **The red `Default` fill from the `.fig` is intentionally not used** — see module docs.
pub struct IconButton {
    icon: Icon,
    size: f32,
    active: bool,
    /// Stable id the caller set via `.id(...)`; `None` lets GPUI derive a stable id from the
    /// element's position in the tree. A per-frame fresh id (the old `next_icon_btn_id()`) broke
    /// click handling: GPUI matches the mouse-down and mouse-up targets by `ElementId`, so a
    /// different id every frame meant the two never matched and `on_click` never fired.
    custom_id: Option<ElementId>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    /// A 24 pt icon button for `icon`.
    pub fn new(icon: Icon) -> Self {
        Self { icon, size: SLOT, active: false, custom_id: None, on_click: None }
    }

    /// The button's square side in points (24 or 32; the design's two sizes).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Toggle the "selected/active" look (an open option, a pressed toggle).
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Override the stable element id (for anchoring menus, matching the egui `ui.scope_id()`).
    /// Without this, GPUI assigns a stable id from the button's position in the tree — which is what
    /// makes clicks work. Only set an explicit id when you need an identity that survives structural
    /// changes (e.g. anchoring a popup to the button).
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.custom_id = Some(id.into());
        self
    }

    /// What happens on click.
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl IntoElement for IconButton {
    type Element = Stateful<Div>;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let glyph = if self.size >= 22.0 { ICON } else { 12.0 };
        let glyph_color = rgba(if self.active { t.accent } else { t.icon });
        // 显式 `.id(...)` 优先（锚定弹窗用）；否则用本帧稳定序号。
        // 一定要 `.id()`：GPUI 0.2.1 里 `hover`/`active`/`on_click` 只挂在 `Stateful<Div>` 上，
        // 而 `.id()` 正是把 `Div` 升级成 `Stateful<Div>` 的唯一公开入口。
        let id = self.custom_id.unwrap_or_else(next_stable_id);

        crate::widgets::dbg_frame(div(), 0x0A84FF)
            .id(id)
            .size(pt(self.size))
            .rounded(pt(RADIUS))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(self.active, |d| d.bg(rgba(t.selected_row)))
            .when(!self.active, |d| d.hover(|s| s.bg(rgba(t.hover))).active(|s| s.bg(rgba(t.pressed))))
            .child(paint_icon(self.icon, glyph_color, glyph).into_any_element())
            .when_some(self.on_click, |d, cb| d.on_click(cb))
    }
}
