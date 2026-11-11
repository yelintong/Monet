// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// 2026-11-05 补充：用户随后明确指令「重构 color_field / template 插槽隐藏 / number_field 范围 /
// 失焦提交」，本文件按该指令解冻；以上 INVARIANT 仍保持不变。
// ============================================================================
//! `number_field` — a numeric variant of [`TextField`], now with an inclusive input range.
//!
//! Shows `value` (formatted) or "Mixed" when `None`; appends `suffix` (e.g. `%`, `°`) while not
//! editing; steps by `step` (Shift = ×10) with the arrow keys; drag-scrubs from the lead. On commit,
//! typing `+4` / `-2` / `*2` / `/2` is evaluated relative to the current value. Every path that
//! produces a number — typing, arrow stepping, scrubbing and a value pushed in from the document —
//! goes through [`NumberConfig::clamp`], so an out-of-range value can never enter the field.

use figcraft_i18n::tr;
use crate::widgets::input::Slot;
use crate::widgets::input::text_field::TextField;
use gpui::{App, Entity, SharedString};

/// A numeric field `width` wide, with no range limit. `value` of `None` shows "Mixed".
/// `lead` reserves the left slot (e.g. a unit icon / the `"X"` label).
pub fn number_field(cx: &mut App, width: f32, value: Option<f64>, suffix: &str, step: f64, lead: Slot) -> Entity<TextField> {
    number_field_ranged(cx, width, value, suffix, step, lead, None, None)
}

/// [`number_field`] with an inclusive `[min, max]` range. `None` leaves that side unbounded;
/// pass `Some(0.0), Some(100.0)` for a percentage, `Some(0.0), None` for a non-negative size.
pub fn number_field_ranged(
    cx: &mut App,
    width: f32,
    value: Option<f64>,
    suffix: &str,
    step: f64,
    lead: Slot,
    min: Option<f64>,
    max: Option<f64>,
) -> Entity<TextField> {
    let shown = value.map(|v| format!("{}", v)).unwrap_or_default();
    let suffix = SharedString::from(suffix.to_string());
    let mixed = value.is_none();
    TextField::build(cx, width, move |tf, _| {
        tf.set_value(shown);
        if mixed {
            tf.set_placeholder(tr("Mixed"));
        }
        tf.set_lead(lead);
        tf.set_numeric(suffix, step);
        tf.set_range(min, max);
    })
}
