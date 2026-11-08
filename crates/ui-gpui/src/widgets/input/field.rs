// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! Convenience constructors over [`TextField`]: the shared field, the search box, and the combo box.

use crate::widgets::icon::Icon;
use crate::widgets::input::Slot;
use crate::widgets::input::text_field::TextField;
use gpui::{App, Entity, Pixels, Point, SharedString, Window};

/// A plain single-line text field: `value`, optional `placeholder`, and optional lead/trail slots.
pub fn text_field(cx: &mut App, width: f32, value: &str, placeholder: &str, lead: Slot, trail: Slot) -> Entity<TextField> {
    // Convert the `&str` references to owned values before the `'static` build closure captures them.
    let value = value.to_string();
    let placeholder = SharedString::from(placeholder.to_string());
    TextField::build(cx, width, move |tf, _| {
        tf.set_value(value);
        tf.set_placeholder(placeholder);
        tf.set_lead(lead);
        tf.set_trail(trail);
    })
}

/// A search box: a leading search icon and — while `value` is non-empty — a trailing clear icon that
/// wipes the field.
pub fn search_field(cx: &mut App, width: f32, value: &str, placeholder: &str) -> Entity<TextField> {
    let value = value.to_string();
    let placeholder = SharedString::from(placeholder.to_string());
    TextField::build(cx, width, move |tf, _| {
        tf.set_value(value);
        tf.set_placeholder(placeholder);
        tf.set_lead(Slot::Icon(Icon::Search));
        tf.set_search_clear(true);
    })
}

/// A combo box: a trailing chevron whose click fires `on_trail(pos, ..)` (the caller opens the menu
/// at `pos` and reads the picked option through its own channel).
pub fn combo_field(
    cx: &mut App,
    width: f32,
    value: &str,
    placeholder: &str,
    on_trail: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
) -> Entity<TextField> {
    let value = value.to_string();
    let placeholder = SharedString::from(placeholder.to_string());
    TextField::build(cx, width, move |tf, _| {
        tf.set_value(value);
        tf.set_placeholder(placeholder);
        tf.set_trail(Slot::Icon(Icon::ChevronDown));
        tf.set_on_trail(on_trail);
    })
}
