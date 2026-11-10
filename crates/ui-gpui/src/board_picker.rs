//! The toolbar's templates picker: a card for each board template (with a picture of itself) and
//! one for a table. Structure copied from `ui-egui/src/widgets/board_picker.rs`.
//!
//! The card grid is 3 columns wide; a picture is the template built on a board of its own and drawn
//! small (`figcraft_engine::board_templates::preview` + `figcraft_render::render_area`). The
//! pictures are drawn once, the first time the picker opens, and kept in [`Pictures`] — rendering
//! five boards costs a few hundred milliseconds, which would stall every frame if it were per-open.

use std::collections::HashMap;
use std::rc::Rc;

use figcraft_doc::BOARD_PAGE;
use figcraft_engine::board_templates::TEMPLATES;
use gpui::{AnyElement, Div, ImageSource, RenderImage, SharedString, div, img, prelude::*, px};
use std::cell::RefCell;

use crate::widgets::{Icon, icon, pt, rgba, tokens};

/// The picker's key while it is open (`CanvasView::tool_flyout`).
pub(crate) const KEY: &str = "board_templates";
/// A card's size (egui `CARD`).
const CARD_W: f32 = 136.0;
const CARD_H: f32 = 104.0;
/// The picture's height inside a card (egui `PICTURE_H`).
const PICTURE_H: f32 = 76.0;
/// The grid's columns (egui `COLUMNS`).
const COLUMNS: usize = 3;
/// The card's padding, and the point between cards (egui `PAD` / `GAP`).
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
/// The room the picker's title takes (egui `TITLE_H`).
const TITLE_H: f32 = 26.0;

/// A template's picture, drawn the first time the picker opens. `None` for one that could not be
/// drawn (the card then shows its mark, like the table's does).
pub(crate) type Pictures = Rc<RefCell<HashMap<&'static str, Option<std::sync::Arc<RenderImage>>>>>;

/// The picker's size: as many cards as there are templates plus the table, `COLUMNS` to a row.
pub(crate) fn size() -> (f32, f32) {
    let count = TEMPLATES.len() + 1;
    let rows = count.div_ceil(COLUMNS);
    let w = COLUMNS as f32 * (CARD_W + GAP) - GAP + 2.0 * PAD;
    let h = TITLE_H + rows as f32 * (CARD_H + GAP) - GAP + 2.0 * PAD;
    (w, h)
}

/// A template's picture: the template built on a board of its own and drawn small.
fn picture(id: &str) -> Option<std::sync::Arc<RenderImage>> {
    let doc = figcraft_engine::board_templates::preview(id).ok()?;
    let layers = doc.node(doc.first_page()?)?.children.clone();
    let b = layers.iter().filter_map(|l| figcraft_render::export_bounds(&doc, *l)).reduce(|a, b| a.union(b))?;
    if !(b.width() > 0.0 && b.height() > 0.0) {
        return None;
    }
    let fit = (f64::from(CARD_W - 16.0) / b.width()).min(f64::from(PICTURE_H - 12.0) / b.height());
    let pix = figcraft_render::render_area(&doc, &layers, b, fit.min(1.0))?;
    Some(crate::canvas::render_worker::screen_image(&pix, doc.color_profile))
}

/// The card for template `i`, or for the table when `i` is past the last template. The caller owns
/// the click (a card is not a button: the whole card is the target).
pub(crate) fn card(pics: &Pictures, i: usize) -> gpui::Stateful<Div> {
    let t = tokens();
    let tpl = TEMPLATES.get(i);
    if let Some(tpl) = tpl {
        pics.borrow_mut().entry(tpl.id).or_insert_with(|| picture(tpl.id));
    }
    let label: SharedString = match tpl {
        Some(tpl) => figcraft_i18n::tr(tpl.name).into(),
        None => figcraft_i18n::tr("Table").into(),
    };
    let mut card = div()
        .id(("board-template", i as u64))
        .w(pt(CARD_W))
        .h(pt(CARD_H))
        .rounded(pt(8.0))
        .p(pt(4.0))
        .flex()
        .flex_col()
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.menu_hover)));
    // The picture's frame: the board's own page colour, with the template drawn inside it.
    let mut frame = div()
        .w_full()
        .h(pt(PICTURE_H))
        .rounded(pt(6.0))
        .bg(rgba(page_color()))
        .border_1()
        .border_color(rgba(t.border))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden();
    let tex = tpl.and_then(|tpl| pics.borrow().get(tpl.id).cloned().flatten());
    frame = match tex {
        Some(tex) => frame.child(
            img(ImageSource::Render(tex))
                .max_w(px(CARD_W - 8.0 - 12.0))
                .max_h(px(PICTURE_H - 12.0)),
        ),
        // The table (and a template whose picture couldn't be drawn): its mark.
        None => frame.child(icon::paint(if tpl.is_some() { Icon::Templates } else { Icon::Table }, rgba(t.icon), 32.0)),
    };
    card = card.child(frame).child(
        div()
            .mt(pt(2.0))
            .h(pt(CARD_H - PICTURE_H - 8.0 - 2.0))
            .flex()
            .items_center()
            .text_size(pt(crate::widgets::TEXT_PT))
            .text_color(rgba(t.menu_text))
            .child(label),
    );
    card
}

/// What a click on card `i` runs: `board.template` for a template, `board.table` for the table.
pub(crate) fn command_for(i: usize) -> (&'static str, serde_json::Value) {
    match TEMPLATES.get(i) {
        Some(tpl) => ("board.template", serde_json::json!({ "name": tpl.id })),
        None => ("board.table", serde_json::json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid is 3 cards wide and as tall as the templates need, plus the table's card.
    #[test]
    fn the_grid_fits_every_template_and_the_table() {
        let count = TEMPLATES.len() + 1;
        assert_eq!(count, 6, "five templates and a table");
        let rows = count.div_ceil(COLUMNS);
        assert_eq!(rows, 2);
        let (w, h) = size();
        assert_eq!(w, COLUMNS as f32 * (CARD_W + GAP) - GAP + 2.0 * PAD);
        assert_eq!(h, TITLE_H + rows as f32 * (CARD_H + GAP) - GAP + 2.0 * PAD);
    }

    /// A card runs the template it shows, and the last card adds a table.
    #[test]
    fn a_card_runs_its_template_or_a_table() {
        for (i, tpl) in TEMPLATES.iter().enumerate() {
            let (id, params) = command_for(i);
            assert_eq!(id, "board.template");
            assert_eq!(params["name"], serde_json::json!(tpl.id));
        }
        let (id, params) = command_for(TEMPLATES.len());
        assert_eq!(id, "board.table");
        assert_eq!(params, serde_json::json!({}));
    }
}

/// The board page colour as the renderer's colour type.
fn page_color() -> crate::canvas::geom::Color32 {
    let [r, g, b, a] = BOARD_PAGE.to_rgba8();
    crate::canvas::geom::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The picker card itself: the title row and the grid. `on_card` is called with the card's index.
pub(crate) fn picker(pics: &Pictures, on_card: Rc<dyn Fn(usize, &mut gpui::Window, &mut gpui::App)>) -> AnyElement {
    let t = tokens();
    let (w, h) = size();
    let count = TEMPLATES.len() + 1;
    let mut grid = div().flex().flex_row().flex_wrap().gap(pt(GAP)).w(pt(COLUMNS as f32 * (CARD_W + GAP) - GAP));
    for i in 0..count {
        let on_card = on_card.clone();
        grid = grid.child(
            card(pics, i)
                .on_mouse_down(gpui::MouseButton::Left, move |_: &gpui::MouseDownEvent, w, cx| on_card(i, w, cx)),
        );
    }
    div()
        .w(pt(w))
        .h(pt(h))
        .p(pt(PAD))
        .rounded(pt(13.0))
        .bg(rgba(t.menu))
        .border_1()
        .border_color(rgba(t.border_strong))
        .shadow(vec![gpui::BoxShadow {
            color: rgba(t.shadow).into(),
            offset: gpui::point(px(0.0), px(3.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
        }])
        .occlude()
        .child(
            div()
                .h(pt(TITLE_H))
                .flex()
                .items_center()
                .pl(pt(2.0))
                .text_size(pt(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.menu_text))
                .child(SharedString::from(figcraft_i18n::tr("Templates"))),
        )
        .child(grid)
        .into_any_element()
}
