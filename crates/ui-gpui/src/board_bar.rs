//! The small bar floating over a board's selection: its colours first (`board.setColor`), then the
//! shape / text-size / connector parts. Structure copied from `ui-egui/src/canvas/board_bar.rs`.
//!
//! This file starts with the container and the colour part; the shape grid, text sizes and
//! connector routing/caps/style are the next parts to add here.

use crate::panels::{Cmd, Cmds};
use crate::widgets::{pt, rgba, tokens};
use figcraft_doc::{BOARD_COLORS, BoardItem};
use gpui::{AnyElement, IntoElement, div, prelude::*, px};
use serde_json::json;

const HEIGHT: f32 = 40.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;

fn color_of(c: figcraft_color::Color) -> crate::canvas::geom::Color32 {
    let [r, g, b, a] = c.to_rgba8();
    crate::canvas::geom::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Whether the selection has anything the bar's colour part changes.
fn has_color(ds: &figcraft_engine::DocState) -> bool {
    ds.selection.iter().filter_map(|id| ds.doc.node(*id)).any(|n| {
        matches!(
            n.board,
            Some(BoardItem::Sticky { .. } | BoardItem::Ink | BoardItem::Shape { .. } | BoardItem::Connector(_))
        )
    })
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
    if !has_color(&ds) {
        return None;
    }
    let b = ds.doc.bounds_of(&ds.selection)?;
    let tl = app.to_screen(b.x0, b.y0);
    let br = app.to_screen(b.x1, b.y1);
    let w = 8.0 + BOARD_COLORS.len() as f32 * (SWATCH + GAP) + 8.0;
    let x = (tl.x + (br.x - tl.x) / 2.0 - w / 2.0).max(0.0);
    let y = (tl.y - 14.0 - HEIGHT).max(0.0);
    let t = tokens();
    let mut row = div().flex().flex_row().items_center().gap(px(GAP)).px(px(8.0));
    for (i, (name, c)) in BOARD_COLORS.iter().enumerate() {
        let name = name.to_string();
        let cmds = cmds.clone();
        row = row.child(
            div()
                .id(gpui::ElementId::from(gpui::SharedString::from(format!("bb-color-{i}"))))
                .size(px(SWATCH))
                .rounded(px(4.0))
                .bg(rgba(color_of(*c)))
                .border_1()
                .border_color(rgba(t.border))
                .cursor_pointer()
                .hover(|s| s.border_2())
                .on_click(move |_, _, _| cmds.push(Cmd { id: "board.setColor", params: Some(json!({ "color": name })) })),
        );
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
