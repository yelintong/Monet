// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! The variable-binding widgets — the port of `panels/variable_ui.rs:340-401` and
//! `widgets.rs:538`'s `bound_field`, built on the [`Input`] template.
//!
//! A bound field is the template's **read-only shape**: the lead, the variable's name in a chip as
//! the middle, the detach icon as the 24 pt trail (`trail_hover_only`, so it stays out of the way
//! until the pointer is on the row). The chip itself is `frame.edit` inset 2 pt left/right and 3 pt
//! top/bottom, radius 4, `t.hover` fill with a `t.border` hairline, the name 8 pt inside it.

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::Slot;
use crate::widgets::input::template::{Frame, Input};
use crate::widgets::{TEXT_PT, pt, rgba, tokens};
use gpui::{Div, SharedString, Window, div, prelude::*};

/// The "Apply variable" button: 18×18, centred 11 pt from the row's right edge, `Icon::Variable`
/// inset 3 (`variable_ui.rs:379-391`). The caller places it.
pub fn variable_button() -> Div {
    let t = tokens();
    div().size(pt(18.0)).rounded(pt(4.0)).flex().items_center().justify_center().hover(|s| s.bg(rgba(t.hover))).child(paint_icon(
        Icon::Variable,
        rgba(t.text_secondary),
        12.0,
    ))
}

/// A field bound to the variable `name` (`widgets.rs:538`): the lead, the chip in the middle, the
/// detach icon as the trail. Returns the frame and the row; the chip is already in it.
pub fn bound_field(window: &mut Window, width: f32, lead: Slot, name: &SharedString) -> (Frame, Div) {
    let t = tokens();
    let input = Input::new(width).lead(lead).trail(Slot::Icon(Icon::Detach)).trail_w(crate::widgets::SLOT).trail_hover_only(true);
    let frame = input.measure(window);

    // The chip is the middle inset 2 pt left/right and 3 pt top/bottom — the row's height, so it
    // is given as padding on the edit cell rather than a rect of its own.
    let short = name.rsplit('/').next().unwrap_or(name.as_ref()).to_string();
    let chip = div()
        .w_full()
        .h_full()
        .rounded(pt(4.0))
        .bg(rgba(t.hover))
        .border_1()
        .border_color(rgba(t.border))
        .pl(pt(8.0))
        .pr(pt(8.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(pt(TEXT_PT))
        .text_color(rgba(t.text))
        .child(short);
    let edit = div().size_full().flex().items_center().px(pt(2.0)).py(pt(3.0)).child(chip).into_any_element();
    let el = input.paint(&frame, crate::widgets::input::template::Parts { lead: None, edit: Some(edit), trail: None, alpha: None, unit: None });
    (frame, el)
}

/// Alongside `bound_field`: whether `field` of the selection is bound (`bound_name`), the value a
/// `bindable` control stands in for. Kept as a type so the panel can branch without touching the
/// document itself.
#[derive(Clone, Debug, Default)]
pub struct Bound {
    /// The variable's full name, `None` when unbound.
    pub name: Option<SharedString>,
}

impl Bound {
    pub fn is_bound(&self) -> bool {
        self.name.is_some()
    }
}
