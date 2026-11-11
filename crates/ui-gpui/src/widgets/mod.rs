// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// 2026-11-07 用户明确授权本次解冻：通用「安全三角 / 子菜单瞄准意图」抽到独立行为文件
// `widgets/menu_aim.rs`，主菜单与右键菜单共用；本文件只补 `pub mod` 与 `pub use`，
// 三条不变量与既有控件行为均未改。
// ============================================================================
//! Monet's shared GPUI controls.
//!
//! Size and spacing conventions used across these widgets: the panel grid is `16 | … | 8`
//! (16 pt left inset, 8 pt right inset), rows sit on a 32 pt rhythm (a 24 pt field centred in
//! `ROW_H`, with no extra row gap), the header is 40 pt tall,
//! fields are 24 pt tall, icon slots are 24 pt, and the icon glyph itself is 16 pt.

use crate::canvas::geom::Color32;
use crate::theme::Tokens;
use gpui::{Pixels, Rgba, px};

pub mod checkbox;
pub mod color_field;
pub mod dropdown;
pub mod icon;
pub mod icon_button;
pub mod layer_icons;
pub mod input;
pub mod menu;
/// 通用菜单行为文件（安全三角 / 瞄准意图 / 悬停延迟）：主菜单、右键菜单共用同一份状态机。
pub mod menu_aim;
pub mod menu_popup;
pub mod panel_section;
/// 新增通用控件（2026-11-05）：颜色选择器等弹窗背后的通用窗口，标题为插槽。
pub mod popover;
pub mod section_header;
/// 新增通用控件（2026-11-11）：多选器 `Segmented`，按设计文件里的 `Segmented` 组件集重做，
/// 带「有选中 / 无选中」两种状态，图标格与文字格共用一套几何。
pub mod segmented;
pub mod swatch;
/// 排版对话框字形（照 lizard `typ-*.svg` 1:1 移植）。独立的 `typ:` 资产前缀，
/// 冻结的 `Icon` 枚举不动；与 `Segmented` 共用 `svg().path().text_color()` 管线。
pub mod typ_icons;
pub mod variable;

pub use checkbox::Checkbox;
pub use color_field::{ColorField, ColorText, paint_label};
pub use dropdown::{DropLead, Dropdown, MenuAnchor, dropdown, dropdown_lead, menu_anchor, set_active_menu};
pub use menu::{MenuRow, color_picker, menu_popup, popup_backdrop};
pub use menu_aim::{AimAction, MENU_AIM_ENABLED, MENU_AIM_REST, MENU_HOVER_DELAY, MenuAim, toward};
pub use menu_popup::{Menu, RectAlign, Width, at_point, menu, menu_surface, point_anchor};
pub use swatch::{Swatch, checker, paint_name, paint_swatch, swatch};
pub use variable::{Bound, bound_field, variable_button};

pub use icon::Icon;
pub use icon_button::IconButton;
pub use input::field::{combo_field, search_field, text_field};
pub use input::number_field::number_field;
pub use input::text_edit::NumberConfig;
pub use input::{EditBuffer, EditOutcome, Slot, TextInput, char_to_utf16_str, utf16_to_char_str};
pub use input::{TextEdit, TextField};
pub use panel_section::{
    FieldRow, PanelSection, field_row, field_row_full, field_row_full_trailing, field_row_h, panel_section, segmented, sublabel_row, sublabel_row2,
    sublabels,
};
// `HEADER_H` is already this module's 40 pt header constant; the popover shares that value.
pub use popover::{
    CLOSE, CLOSE_RIGHT, Open as PopoverOpen, Popover, RADIUS as POPOVER_RADIUS, SLOT_PAD, Surface, WIDTH as POPOVER_WIDTH, popover_window, title_text,
};
pub use section_header::{SectionHeader, SectionHeaderVariant};
pub use segmented::{SegItem, Segmented};

/// The panel's left inset: 16 pt.
pub const PAD: f32 = 16.0;
/// Between a menu row's label and its right-aligned shortcut (or a submenu row's chevron):
/// 16 pt, specified by the user on 2026-11-06.
///
/// A **minimum**, not a fixed gutter: the label is `flex_1`, so a long label elides and still
/// leaves 16 pt clear of the shortcut instead of running into it.
pub const MENU_TRAIL_GAP: f32 = 16.0;
/// The panel's right inset: 8 pt.
pub const PAD_RIGHT: f32 = 8.0;
/// Between the panel's grid **columns** (the horizontal gutter).
pub const GRID_GAP: f32 = 8.0;
/// Between the panel grid's **rows**: zero.
///
/// `ui-egui`'s right panel sets `item_spacing = vec2(8.0, 0.0)` (`panels/right.rs`), so rows sit
/// directly on top of each other and the 32 pt `ROW_H` alone is the row pitch. A 24 pt field
/// centred in a 32 pt row already leaves 4 pt above and below — an extra row gap would double the
/// visual distance and break the design's 32 pt rhythm.
pub const ROW_GAP: f32 = 0.0;
/// Space between a section's grid (its `Row 32` stack) and the divider below it.
///
/// The `.fig`'s `Panel Section` has `gap=0` / `pad=0`; its 12 pt `Caption` row is a child, not a
/// margin. This is the explicit breathing room requested for the right panel: every section keeps
/// its grid 12 pt clear of the following separator.
pub const SECTION_BOTTOM: f32 = 12.0;

/// `FC_DEBUG_LAYOUT=1` 时给控件描一圈彩边：每个盒子的边界一眼可见，方便核对布局。
/// border 是内描边，不影响尺寸。
///
/// ⛔ 只认「明确开启」：`FC_DEBUG_LAYOUT=0` / `false` / 空串都算关。以前用 `var_os(..).is_some()`，
/// 于是 `FC_DEBUG_LAYOUT=0` 反而把整窗彩边打开（用户 2026-11-07 让关掉时才发现）。
pub fn debug_layout() -> bool {
    // 缓存：`dbg_frame` 每帧对**每个控件**都要问一次，`std::env::var` 每次都要拿全局环境锁
    // 并分配 String——上千个控件 × 60fps 是实打实的开销。启动读一次即可。
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("FC_DEBUG_LAYOUT") {
        Ok(v) => {
            let v = v.trim();
            !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        }
        Err(_) => false,
    })
}

pub fn dbg_frame<E: gpui::Styled>(el: E, color: u32) -> E {
    if debug_layout() { el.border_1().border_color(gpui::rgb(color)) } else { el }
}

/// 字形视觉居中修正（逻辑像素）。GPUI 的行盒按 ascent+descent 居中，而 descent 下沿的
/// 留白比 ascent 上沿多，于是文字看起来偏下约 1pt。所有自绘文字统一上移这么多。
pub const TEXT_NUDGE: f32 = 1.0;
/// A text lead's own left padding inside its slot (`ui-egui`'s `LEAD_PAD`): a `Slot::Text` is
/// `LEAD_PAD + measured` wide, clamped to 16..64 — not a fixed 24 pt square.
pub const LEAD_PAD: f32 = 8.0;
/// Inset of a field's inner input box from its leading/trailing slot.
///
/// Ported from `ui-egui`'s `field_editor` (`rect.shrink2(vec2(8.0, 0.0))`): the editable text
/// sits 8 pt clear of a 24 pt icon slot, and the same 8 pt is reserved at the trailing end.
pub const FIELD_INSET: f32 = 8.0;
/// A section header's height.
pub const HEADER_H: f32 = 40.0;
/// A panel body row's height (the design's `Row 32`).
pub const ROW_H: f32 = 32.0;
/// The gap between a header's title and its trailing slot.
pub const HEADER_GAP: f32 = 4.0;
/// A text/number field's height.
pub const FIELD_H: f32 = 24.0;
/// Field 内部水平 AL 的 gap（设计稿 `Field`: gap=4）。
pub const FIELD_GAP: f32 = 4.0;
/// The square a lead/trail slot (or an icon button) occupies.
pub const SLOT: f32 = 24.0;
/// The glyph drawn inside a 24/32 pt slot.
pub const ICON: f32 = 16.0;
/// Corner radius shared by fields and icon buttons (`CornerRadius::same(5)` in the egui original).
pub const RADIUS: f32 = 5.0;
/// The header title: Inter 11 Bold (the design's `Title` style).
///
/// 字号只有三档（用户 2026-11-05 定）：**9 小字 / 11 正文 / 13 标题**。
/// 任何新控件都必须落在这三档里，不要再出现 10 / 11.5 / 12 / 14 这类中间值。
pub const TITLE_PT: f32 = 13.0;
/// Field / number / combo text size (egui's `TEXT_PT`) —— 正文档 11。
pub const TEXT_PT: f32 = 11.0;
/// 小字档：节内的说明文字（`sublabels`）、弹窗里的字段标签。
pub const CAPTION_PT: f32 = 9.0;

thread_local! {
    /// 当前这一帧的主题令牌。
    ///
    /// ⛔ 以前这里是写死的 `Tokens::dark()`——于是**整个 gpui 界面根本不跟随主题**：
    /// 亮/暗、强调色怎么切，所有面板、菜单、弹层都还是那套暗色常量（用户 2026-11-05 发现
    /// 「颜色一个都没绑」）。现在由 `CanvasView::render` 每帧从 `session.prefs`
    /// 重建 `Tokens` 后 `set_tokens` 写进来，`tokens()` 读它。
    static CURRENT: std::cell::Cell<Tokens> = std::cell::Cell::new(Tokens::dark());
}

/// 这一帧的主题令牌（所有控件都从这里取颜色）。
pub fn tokens() -> Tokens {
    CURRENT.with(|t| t.get())
}

/// 由根视图在每帧 render 开头调用：把从偏好重建好的主题设成本帧的当前主题。
pub fn set_tokens(t: Tokens) {
    CURRENT.with(|c| c.set(t));
}

/// `Color32` (canvas token) → GPUI `Rgba`.
pub fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.0;
    Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

/// `f32` points → GPUI pixels.
pub fn pt(p: f32) -> Pixels {
    px(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two spacings requested for the right panel, kept as named constants so a future
    /// "tidy up" cannot silently collapse them back to the `.fig`'s `gap=0` / `pad=0`.
    #[test]
    fn panel_spacing_constants() {
        assert_eq!(SECTION_BOTTOM, 12.0, "grid → divider");
        assert_eq!(FIELD_INSET, 8.0, "lead slot → inner input box");
        assert_eq!(SLOT, 24.0, "an icon slot is 24 pt");
        assert_eq!(GRID_GAP, 8.0, "between grid columns");
        assert_eq!(ROW_GAP, 0.0, "between grid rows: none, ROW_H is the pitch");
        // egui's `field_editor`: edit_rect starts after the slot, then shrinks 8 pt on each side.
        assert_eq!(SLOT + FIELD_INSET, 32.0);
        assert_eq!(PAD, 16.0);
        assert_eq!(PAD_RIGHT, 8.0);
        assert_eq!(ROW_H, 32.0);
        assert_eq!(FIELD_H, 24.0);
    }
}
