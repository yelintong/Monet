// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ----------------------------------------------------------------------------
// 2026-11-06 解冻（用户指令）：「照搬EGUI还原底部工具栏功能…检查下拉菜单是否由template衍生，
// 并改template禁用样式作为下拉菜单的普通样式，并去掉背景颜色」。
// 本文件因此按指令重写：
//   * 下拉按钮**不再自己拼一个 div**，改为走 `input/template.rs` 的 `Input` 模板（lead | edit |
//     trail 三槽一次布局），与文本框/数字框/颜色行同源；
//   * 普通态取模板的**禁用态样式**（`bare`：无填充、无壳），即**去掉背景颜色**；
//   * hover 只加一圈绝对定位的 1pt 发丝（不参与布局、不改变底色），egui 的 `field_hover_border`。
// 以上三条是本次改动的目标，后续不要退回「自己画背景」的写法。
// ============================================================================
//! The dropdown button — the [`Input`] template in its **bare** (no-fill) state.
//!
//! The design's `Field / Disabled` look is this control's *normal* look: no background, no shell,
//! only the lead, the label and the trailing chevron, with one hairline while hovered. That is the
//! template's `bare` path — the same layout the text/number/colour fields are built on, so the
//! lead slot, the value's inset and the chevron all sit on the design's 24 pt rhythm instead of
//! being hand-positioned. Metrics (`ui-egui`'s `widgets::dropdown`):
//!
//! * **lead** — a 16 pt glyph or a 9 pt colour dot, in the template's 24 pt lead slot;
//! * **label** — the editable middle, elided when the dropdown is narrower than its text;
//! * **chevron** — 12 pt, in the template's 24 pt trail slot (flush with the field's right edge).

use crate::canvas::geom::Color32;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::template::{Input, Parts};
use crate::widgets::{LEAD_PAD, RADIUS, SLOT, TEXT_PT, pt, rgba, tokens};
use gpui::{App, MouseButton, MouseDownEvent, Pixels, SharedString, Window, div, prelude::*};
use std::rc::Rc;

/// The lead a dropdown may carry (`ui-egui`'s `DropLead`).
#[derive(Clone, Copy)]
pub enum DropLead {
    /// A 14 pt glyph.
    Icon(Icon),
    /// A 4.5 pt-radius colour dot.
    Dot(Color32),
}

/// The click callback: it receives the button's own rect (the menu's anchor).
pub type OnClick = Rc<dyn Fn(gpui::Bounds<Pixels>, &mut Window, &mut App)>;

/// A dropdown-looking button showing `label`, `width` points wide.
pub struct Dropdown {
    label: SharedString,
    width: f32,
    /// `true`：填满所在的网格单元（`w_full`），不按像素定宽——面板里**不允许手算列宽**。
    fill: bool,
    lead: Option<DropLead>,
    on_click: Option<OnClick>,
}

impl Dropdown {
    /// `ui-egui::dropdown`: `width` is required (`col * 2.0 + 8.0` in the panel), the lead optional.
    pub fn new(label: impl Into<SharedString>, width: f32, lead: Option<DropLead>) -> Self {
        Self { label: label.into(), width, fill: false, lead, on_click: None }
    }

    /// 填满所在网格单元（`w_full`）：列宽由统一网格的 `flex_1` 决定，调用方不传宽度。
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Called on a click, with **the button's own rect** — `ui-egui` passes the field's `Response`
    /// to `menu_popup`, whose width and position both come from that rect, not from the pointer.
    pub fn on_click(mut self, f: impl Fn(gpui::Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl IntoElement for Dropdown {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let width = self.width;
        let on_click = self.on_click;
        // The rect `ui-egui` reads off the `Response` (`anchor.rect`).
        let hit = Rc::new(std::cell::Cell::new(None::<gpui::Bounds<Pixels>>));
        let hit_click = hit.clone();

        // ONE template: lead | edit | trail, measured by `layout` (no window needed — the slots'
        // widths are the design's 24 pt cell and an 8 pt empty lead).
        let input = Input::new(width).bare(true);
        let lead_w = if self.lead.is_some() { SLOT } else { LEAD_PAD };
        let frame = input.layout(lead_w, SLOT);

        let lead = self.lead.map(|lead| match lead {
            DropLead::Icon(i) => div().flex_none().size(pt(16.0)).child(paint_icon(i, rgba(t.text_secondary), 16.0)).into_any_element(),
            // A 9 pt dot centred in the 24 pt slot, as the original draws it.
            DropLead::Dot(c) => div().flex_none().size(pt(9.0)).rounded(pt(5.0)).bg(rgba(c)).into_any_element(),
        });
        let label = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(pt(TEXT_PT))
            .text_color(rgba(t.text))
            .child(self.label.clone())
            .into_any_element();
        let chevron = div().flex_none().size(pt(12.0)).child(paint_icon(Icon::ChevronDown, rgba(t.text_secondary), 12.0)).into_any_element();

        let mut el = input.paint(&frame, Parts { lead, edit: Some(label), trail: Some(chevron), ..Default::default() });
        el = if self.fill { el } else { el.w(pt(width)) };
        // The permanent hairline: an absolute overlay, so it neither repaints a background nor takes
        // part in layout (the shell's own 1 pt border would shrink the content box by 1). `用户
        // 2026-11-06`：普通态也要有描边（像选中态那样常态化显示），所以常态就是 `border_strong`，
        // 不再透明；hover 时换成 `field_hover_border`。背景色由 `bare` 模板已去掉，这里不动。
        el = el
            .group("dropdown-bare")
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded(pt(RADIUS))
                    .border_1()
                    .border_color(rgba(t.border_strong))
                    .group_hover("dropdown-bare", |s| s.border_color(rgba(t.field_hover_border))),
            )
            // The anchor: the card's width and position both come from this rect.
            .child(
                gpui::canvas(
                    move |bounds, _window, _cx| {
                        hit.set(Some(bounds));
                    },
                    |_bounds, _, _window, _cx| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| {
                if let (Some(f), Some(rect)) = (on_click.as_ref(), hit_click.get()) {
                    f(rect, window, cx);
                }
            });
        el
    }
}

/// `ui-egui::dropdown`: a dropdown with an optional **icon** lead.
pub fn dropdown(label: impl Into<SharedString>, width: f32, lead: Option<Icon>) -> Dropdown {
    dropdown_lead(label, width, lead.map(DropLead::Icon))
}

/// `ui-egui::dropdown_lead`: the same button with an icon **or a colour dot** before the label.
pub fn dropdown_lead(label: impl Into<SharedString>, width: f32, lead: Option<DropLead>) -> Dropdown {
    Dropdown::new(label, width, lead)
}
