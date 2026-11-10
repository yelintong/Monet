//! The small bar floating over a board's selection: its colours first (`board.setColor`), then the
//! text sizes (`board.setStyle {fontSize}`). Structure copied from `ui-egui/src/canvas/board_bar.rs`.
//!
//! The shape grid (outlines) and the connector routing / caps / line style are the remaining parts
//! of the same file; they need shape-outline and line-sample drawing.

use crate::panels::{Cmd, Cmds};
use crate::widgets::{pt, rgba, tokens};
use figcraft_doc::{BOARD_COLORS, BoardItem};
use figcraft_engine::board::TEXT_SIZES;
use gpui::{AnyElement, IntoElement, SharedString, div, prelude::*, px};
use serde_json::json;

const HEIGHT: f32 = 40.0;
const CELL: f32 = 28.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;
/// The room each part takes after its controls (the divider and the gap around it).
const PART_END: f32 = 8.0;

fn color_of(c: figcraft_color::Color) -> crate::canvas::geom::Color32 {
    let [r, g, b, a] = c.to_rgba8();
    crate::canvas::geom::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// What the selected board objects take from the bar (egui `has`): colours for stickies / ink /
/// shapes / connectors, text sizes for shapes and labels.
struct Has {
    color: bool,
    text: bool,
}

fn has(ds: &figcraft_engine::DocState) -> Option<Has> {
    let mut h = Has { color: false, text: false };
    for n in ds.selection.iter().filter_map(|id| ds.doc.node(*id)) {
        match &n.board {
            Some(BoardItem::Sticky { .. } | BoardItem::Ink) => h.color = true,
            Some(BoardItem::Shape { .. }) => {
                h.color = true;
                h.text = true;
            }
            Some(BoardItem::Connector(_)) => h.color = true,
            Some(BoardItem::Label { .. }) => h.text = true,
            _ => {}
        }
    }
    (h.color || h.text).then_some(h)
}

/// A thin separator between two parts.
fn divider(t: &crate::theme::Tokens) -> impl IntoElement {
    div().flex_none().w(px(1.0)).h(px(20.0)).mx(px(2.0)).bg(rgba(t.border))
}

/// The bar above a board selection, while nothing is being dragged and the file can be edited.
pub(crate) fn bar(app: &crate::CanvasApp, cmds: Cmds) -> Option<AnyElement> {
    if app.session.tool.busy() || app.session.active_read_only().is_some() {
        return None;
    }
    if !app.session.doc().is_ok_and(figcraft_engine::board::is_board) {
        return None;
    }
    let ds = app.session.ds().ok()?;
    let h = has(&ds)?;
    let b = ds.doc.bounds_of(&ds.selection)?;
    let tl = app.to_screen(b.x0, b.y0);
    let br = app.to_screen(b.x1, b.y1);
    // The bar's width: the colours, a divider + the sizes, and the padding.
    let mut w = 8.0;
    if h.color {
        w += BOARD_COLORS.len() as f32 * (SWATCH + GAP) + PART_END;
    }
    if h.text {
        w += TEXT_SIZES.len() as f32 * (CELL + 2.0) + PART_END;
    }
    let x = (tl.x + (br.x - tl.x) / 2.0 - w / 2.0).max(0.0);
    let y = (tl.y - 14.0 - HEIGHT).max(0.0);
    let t = tokens();
    let mut row = div().flex().flex_row().items_center().gap(px(GAP)).px(px(8.0));
    if h.color {
        for (i, (name, c)) in BOARD_COLORS.iter().enumerate() {
            let name = name.to_string();
            let cmds = cmds.clone();
            row = row.child(
                div()
                    .id(gpui::ElementId::from(SharedString::from(format!("bb-color-{i}"))))
                    .size(px(SWATCH))
                    .rounded(px(4.0))
                    .bg(rgba(color_of(*c)))
                    .border_1()
                    .border_color(rgba(t.border))
                    .cursor_pointer()
                    .on_click(move |_, _, _| cmds.push(Cmd { id: "board.setColor", params: Some(json!({ "color": name })) })),
            );
        }
    }
    if h.color && h.text {
        row = row.child(divider(&t));
    }
    if h.text {
        for (label, size) in TEXT_SIZES {
            let cmds = cmds.clone();
            row = row.child(
                div()
                    .id(gpui::ElementId::from(SharedString::from(format!("bb-size-{label}"))))
                    .size(px(CELL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(t.text))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(t.hover)))
                    .child(SharedString::from(label))
                    .on_click(move |_, _, _| cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ "fontSize": size })) })),
            );
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
