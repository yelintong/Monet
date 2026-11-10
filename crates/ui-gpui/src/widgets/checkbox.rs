//! `Checkbox` — a binary choice, matching the GPUI Kit component
//! (<https://gpui-kit.com/zh-CN/component/checkbox/>) and drawn like the egui `checkbox`
//! (`ui-egui/src/widgets.rs::checkbox`).
//!
//! API (same shape as `gpui_kit::component::checkbox::Checkbox`):
//! ```ignore
//! Checkbox::new("accept-terms")
//!     .label("Accept terms and conditions")
//!     .checked(self.accept)
//!     .on_change(cx.listener(|this, checked, _, cx| {
//!         this.accept = *checked;
//!         cx.notify();
//!     }))
//! ```
//! `on_change` (and its compatibility alias `on_click`) receive the **new** value; the owner
//! stores it and notifies. This is a new common widget and does not modify any frozen file.

use crate::widgets::{Icon, icon, pt, rgba, tokens};
use gpui::{App, ClickEvent, Div, ElementId, IntoElement, SharedString, Stateful, Window, div, prelude::*, px};
use std::cell::Cell;

/// The box's side (egui `vec2(14.0, 14.0)`).
const BOX: f32 = 14.0;
/// The box's corner radius (egui `CornerRadius::same(3)`).
const RADIUS: f32 = 3.0;
/// The row's height (egui `FIELD_H`).
const ROW_H: f32 = 24.0;
/// The row never grows past this (egui `ui.available_width().min(200.0)`).
const MAX_W: f32 = 200.0;

thread_local! {
    static SEQ: Cell<usize> = const { Cell::new(0) };
}

fn next_stable_id() -> ElementId {
    let n = SEQ.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    ElementId::from(("checkbox", n))
}

/// The new value is delivered here (the kit's `on_change`).
type ChangeHandler = Box<dyn Fn(&bool, &mut Window, &mut App)>;

/// A labelled checkbox. `checked` is the current value; `on_change` reports the value it should
/// become when the user toggles it.
pub struct Checkbox {
    id: Option<ElementId>,
    label: Option<SharedString>,
    checked: bool,
    disabled: bool,
    text_pt: f32,
    on_change: Option<ChangeHandler>,
}

impl Checkbox {
    /// `id` identifies the element (like the kit's `Checkbox::new("id")`).
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: Some(id.into()), label: None, checked: false, disabled: false, text_pt: 12.0, on_change: None }
    }

    /// The visible label to the right of the box.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The current value.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Dim the row and stop reporting changes.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The label's size (the kit's `Sizable`: `text_xs` 11 · `text_sm` 12 · `text_base` 13 ·
    /// `text_lg` 14).
    pub fn text_xs(mut self) -> Self {
        self.text_pt = 11.0;
        self
    }
    pub fn text_sm(mut self) -> Self {
        self.text_pt = 12.0;
        self
    }
    pub fn text_base(mut self) -> Self {
        self.text_pt = 13.0;
        self
    }
    pub fn text_lg(mut self) -> Self {
        self.text_pt = 14.0;
        self
    }

    /// Called with the **new** value when the user toggles the box.
    pub fn on_change(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// Compatibility alias for [`Self::on_change`] (the kit keeps both names for one callback).
    pub fn on_click(self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change(f)
    }
}

impl IntoElement for Checkbox {
    type Element = Stateful<Div>;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let id = self.id.unwrap_or_else(next_stable_id);
        let checked = self.checked;
        let disabled = self.disabled;
        let text_pt = self.text_pt;
        let label = self.label;
        // The click reports the value the box should become (`!checked`), unless disabled.
        let handler = self.on_change;
        let click = handler.map(|cb| {
            move |_e: &ClickEvent, window: &mut Window, cx: &mut App| {
                if !disabled {
                    cb(&!checked, window, cx);
                }
            }
        });
        let text_color = if disabled { t.text_tertiary } else { t.text };

        div()
            .id(id)
            .w_full()
            .max_w(px(MAX_W))
            .h(px(ROW_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .when(!disabled, |d| d.cursor_pointer())
            // The 14×14 box.
            .child(
                div()
                    .w(px(BOX))
                    .h(px(BOX))
                    .flex_none()
                    .rounded(pt(RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(checked, |d| d.bg(rgba(if disabled { t.text_tertiary } else { t.accent })))
                    .when(!checked, |d| d.border_1().border_color(rgba(t.border_strong)))
                    .when(checked, |d| d.child(icon::paint(Icon::Check, rgba(t.accent_on), 11.0).into_any_element())),
            )
            .when_some(label, |d, label| d.child(div().text_size(px(text_pt)).text_color(rgba(text_color)).child(label)))
            .when_some(click, |d, cb| d.on_click(cb))
    }
}
