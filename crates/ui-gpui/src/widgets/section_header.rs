// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! `Section Header` — *one* control with two display states, not two controls.
//!
//! The `.fig` ships `Section Header/State=Normal` and `Section Header/State=Dim`; those are the
//! same header at two visual states (an active section vs. an empty/greyed one), so this is a
//! single `SectionHeader` whose `variant` flips the title colour. The leading and trailing slots are
//! reserved up front (the design's `Slot` frame): `lead` sits left of the title, `trail` is pushed
//! to the right edge, and `justify_between` keeps the title hugging the left while `trail` sits at
//! the 8 pt right inset.

use crate::theme::Tokens;
use crate::widgets::{HEADER_GAP, HEADER_H, PAD, PAD_RIGHT, TITLE_PT, dbg_frame, pt, rgba, tokens};
use gpui::{AnyElement, Div, FontWeight, IntoElement, SharedString, div, prelude::*};

/// The two display states of the one `Section Header` control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionHeaderVariant {
    /// The active section's header (full-weight white title).
    Normal,
    /// An empty section's header: the title greys to `text_secondary`.
    Dim,
}

/// A section header: a bold title, an optional leading slot (left of the title) and an optional
/// trailing slot (right-aligned). Mirrors the `ui-egui` `SectionOpts` (`lead`/`buttons`) but as a
/// single composable element.
pub struct SectionHeader {
    title: SharedString,
    variant: SectionHeaderVariant,
    lead: Option<AnyElement>,
    trail: Option<AnyElement>,
}

impl SectionHeader {
    /// A header for `title`, in the `Normal` state, with no slots reserved yet.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), variant: SectionHeaderVariant::Normal, lead: None, trail: None }
    }

    /// Flip the display state (`Normal` ⇄ `Dim`).
    pub fn variant(mut self, variant: SectionHeaderVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Reserve the trailing slot (the design's `Slot` frame, right-aligned at the 8 pt inset).
    pub fn trail(mut self, el: impl IntoElement) -> Self {
        self.trail = Some(el.into_any_element());
        self
    }

    /// Reserve the leading slot, drawn to the left of the title.
    pub fn lead(mut self, el: impl IntoElement) -> Self {
        self.lead = Some(el.into_any_element());
        self
    }
}

impl IntoElement for SectionHeader {
    type Element = Div;

    fn into_element(self) -> Div {
        let t: Tokens = tokens();
        let dim = self.variant == SectionHeaderVariant::Dim;
        let title_color = rgba(if dim { t.text_secondary } else { t.text });

        // Left group: optional lead slot + the title, hugging the left edge.
        let left = div().flex().flex_row().items_center().gap(pt(HEADER_GAP)).when_some(self.lead, |d, lead| d.child(lead)).child(dbg_frame(
            div()
                // 字号三档（9 / 11 / 13）：大标题用 TITLE_PT = 13。
                .text_size(pt(TITLE_PT))
                // 行高按 1.25×字号，让 GPUI 自己在行框里排。
                .line_height(pt(TITLE_PT * 1.25))
                // 用户 2026-11-05：大标题的字重全部用 medium（原来是 bold，显得过重）。
                .font_weight(FontWeight::MEDIUM)
                .text_color(title_color)
                .child(self.title.clone()),
            0xFF2D55,
        ));

        // `spaceBetween` of [left group, trail] reproduces the design's `main=spaceBetween`.
        let mut el =
            div().h(pt(HEADER_H)).flex().flex_row().items_center().justify_between().gap(pt(HEADER_GAP)).pl(pt(PAD)).pr(pt(PAD_RIGHT)).child(left);

        if let Some(trail) = self.trail {
            el = el.child(trail);
        }
        dbg_frame(el, 0x30D158)
    }
}
