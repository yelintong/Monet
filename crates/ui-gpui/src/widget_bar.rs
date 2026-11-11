//! The small bar floating over a selected plugin widget that has a property menu: its actions and
//! toggles as buttons, its colour choices as swatches. Each calls `widget.menu`; the labels are the
//! plugin's own.
//!
//! Structure copied from `ui-egui/src/canvas/widget_bar.rs`: the same HEIGHT / CELL / SWATCH / GAP
//! / PAD, the same "above the selection, centred" placement, and the same `widget.menu` payload.

use crate::panels::{Cmd, Cmds};
use crate::widgets::{pt, rgba, tokens};
use figcraft_engine::widgets::{MenuItem, MenuKind, menu_of};
use gpui::{AnyElement, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};

const HEIGHT: f32 = 40.0;
const CELL: f32 = 28.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;
/// A text button's padding on each side.
const PAD: f32 = 8.0;

/// What one item takes across the bar, its trailing gap included (text buttons by their label).
fn item_width(m: &MenuItem) -> f32 {
    match m.kind {
        MenuKind::Color => m.options.len() as f32 * (SWATCH + GAP),
        // GPUI has no synchronous text measure here; 7 px per character is the 11 pt semibold
        // average the egui bar lays out to.
        _ => m.label.chars().count() as f32 * 7.0 + PAD * 2.0 + GAP,
    }
}

/// The bar above the selected widget, while it is the only layer selected, nothing is being dragged
/// or edited and the file can be edited. `None` when there is nothing to show.
pub(crate) fn bar(app: &crate::CanvasApp, cmds: Cmds) -> Option<AnyElement> {
    if app.session.tool.busy() || app.session.active_read_only().is_some() {
        return None;
    }
    let ds = app.session.ds().ok()?;
    let [w] = ds.selection.as_slice() else { return None };
    if ds.text_edit.is_some() {
        return None;
    }
    let w = *w;
    let menu = menu_of(&ds.doc, w);
    if menu.is_empty() {
        return None;
    }
    let b = ds.doc.world_bounds(w)?;
    let tl = app.to_screen(b.x0, b.y0);
    let br = app.to_screen(b.x1, b.y1);
    let widths: Vec<f32> = menu.iter().map(item_width).collect();
    // The items, a divider (8) between each two, and the padding.
    let total = 8.0 + widths.iter().sum::<f32>() + 8.0 * menu.len().saturating_sub(1) as f32 + 4.0;
    let x = (tl.x + (br.x - tl.x) / 2.0 - total / 2.0).max(0.0);
    let y = (tl.y - HEIGHT - 8.0).max(0.0);
    let t = tokens();
    let mut row = div().flex().flex_row().items_center().gap(px(GAP)).px(px(8.0));
    for (i, m) in menu.iter().enumerate() {
        if i > 0 {
            row = row.child(div().w(px(1.0)).h(px(20.0)).bg(rgba(t.border)));
        }
        match m.kind {
            MenuKind::Color => {
                let mut sw = div().flex().flex_row().gap(px(GAP));
                for hex in &m.options {
                    let on = m.value.as_deref() == Some(hex.as_str());
                    let color = crate::canvas::geom::Color32::from_hex(hex).unwrap_or(crate::canvas::geom::Color32::BLACK);
                    let item = m.item.clone();
                    let val = hex.clone();
                    let cmds = cmds.clone();
                    sw = sw.child(
                        div()
                            .id(gpui::ElementId::from(SharedString::from(format!("wb-color-{}-{i}", m.item))))
                            .size(px(SWATCH))
                            .rounded(px(4.0))
                            .bg(rgba(color))
                            .when(on, |d| d.border_2().border_color(rgba(t.text)))
                            .cursor_pointer()
                            .on_click(move |_, _, _| {
                                cmds.push(Cmd { id: "widget.menu", params: Some(json!({"id": w, "item": item.clone(), "value": val.clone()})) });
                            }),
                    );
                }
                row = row.child(sw);
            }
            _ => {
                let on = m.kind == MenuKind::Toggle && m.checked;
                let item = m.item.clone();
                let checked = m.checked;
                let is_toggle = m.kind == MenuKind::Toggle;
                let cmds = cmds.clone();
                let label = m.label.clone();
                row = row.child(
                    div()
                        .id(gpui::ElementId::from(SharedString::from(format!("wb-item-{}", m.item))))
                        .h(px(CELL))
                        .px(px(PAD))
                        .rounded(px(5.0))
                        .flex()
                        .items_center()
                        .text_size(px(11.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(if on { t.accent_on } else { t.text }))
                        .when(on, |d| d.bg(rgba(t.accent)))
                        .hover(|s| s.bg(rgba(t.hover)))
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |_, _, _| {
                            let value = if is_toggle { json!(!checked) } else { Value::Null };
                            cmds.push(Cmd { id: "widget.menu", params: Some(json!({"id": w, "item": item.clone(), "value": value.clone()})) });
                        }),
                );
            }
        }
    }
    Some(
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .h(px(HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(10.0))
            .bg(rgba(t.panel))
            .border_1()
            .border_color(rgba(t.border))
            .child(row)
            .into_any_element(),
    )
}
