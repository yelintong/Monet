//! The small bar floating over a board's selection: colours (`board.setColor`), text sizes
//! (`board.setStyle {fontSize}`) and the shape grid (`board.setStyle {shape}`). Structure copied
//! from `ui-egui/src/canvas/board_bar.rs`.
//!
//! The connector part (routing / caps / line style) is the remaining section of the same file.

use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, icon, pt, rgba, tokens};
use figcraft_doc::{BOARD_COLORS, BoardItem, BoardShape};
use figcraft_engine::board::TEXT_SIZES;
use gpui::{AnyElement, IntoElement, SharedString, canvas, div, prelude::*, px};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

const HEIGHT: f32 = 40.0;
const CELL: f32 = 28.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;
/// The room each part takes after its controls (the divider and the gap around it).
const PART_END: f32 = 8.0;
/// The shape grid's columns (egui `SHAPE_COLUMNS`).
const SHAPE_COLUMNS: usize = 6;

fn color_of(c: figcraft_color::Color) -> crate::canvas::geom::Color32 {
    let [r, g, b, a] = c.to_rgba8();
    crate::canvas::geom::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The selected board objects' colour / text / shape (egui `has`).
struct Has {
    color: bool,
    text: bool,
    shape: Option<BoardShape>,
}

fn has(ds: &figcraft_engine::DocState) -> Option<Has> {
    let mut h = Has { color: false, text: false, shape: None };
    for n in ds.selection.iter().filter_map(|id| ds.doc.node(*id)) {
        match &n.board {
            Some(BoardItem::Sticky { .. } | BoardItem::Ink) => h.color = true,
            Some(BoardItem::Shape { shape }) => {
                h.color = true;
                h.text = true;
                h.shape = Some(*shape);
            }
            Some(BoardItem::Connector(_)) => h.color = true,
            Some(BoardItem::Label { .. }) => h.text = true,
            _ => {}
        }
    }
    (h.color || h.text || h.shape.is_some()).then_some(h)
}

/// A shape's outline as a GPUI stroke path, offset to `origin` (kurbo `BezPath` → `PathBuilder`).
fn shape_path_at(shape: BoardShape, size: f32, origin: gpui::Point<gpui::Pixels>) -> Option<gpui::Path<gpui::Pixels>> {
    use figcraft_geom::kurbo::PathEl;
    let bez = shape.path(f64::from(size), f64::from(size));
    let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));
    let pt = |x: f64, y: f64| gpui::point(px(ox + x as f32), px(oy + y as f32));
    let mut b = gpui::PathBuilder::stroke(px(1.2));
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(pt(p.x, p.y)),
            PathEl::LineTo(p) => b.line_to(pt(p.x, p.y)),
            PathEl::QuadTo(c, p) => b.curve_to(pt(p.x, p.y), pt(c.x, c.y)),
            PathEl::CurveTo(c1, c2, p) => b.cubic_bezier_to(pt(p.x, p.y), pt(c1.x, c1.y), pt(c2.x, c2.y)),
            PathEl::ClosePath => b.close(),
        }
    }
    b.build().ok()
}

/// A shape drawn as its outline in `ink`, `size` square (egui `paint_choice` `Mark::Shape`).
fn shape_cell(shape: BoardShape, size: f32, ink: gpui::Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if let Some(path) = shape_path_at(shape, size, bounds.origin) {
                window.paint_path(path, ink);
            }
        },
    )
    .size(px(size))
}

/// A thin separator between two parts.
fn divider(t: &crate::theme::Tokens) -> impl IntoElement {
    div().flex_none().w(px(1.0)).h(px(20.0)).mx(px(2.0)).bg(rgba(t.border))
}

/// The bar above a board selection, while nothing is being dragged and the file can be edited.
pub(crate) fn bar(app: &crate::CanvasApp, cmds: Cmds, shape_open: &Rc<RefCell<bool>>) -> Option<AnyElement> {
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
    let mut w = 8.0;
    if h.color {
        w += BOARD_COLORS.len() as f32 * (SWATCH + GAP) + PART_END;
    }
    if h.shape.is_some() {
        w += CELL + 12.0 + GAP + PART_END;
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
    if let Some(shape) = h.shape {
        if h.color {
            row = row.child(divider(&t));
        }
        // The shape button: the current shape + a chevron; it toggles the grid below.
        let open = shape_open.clone();
        let is_open = *shape_open.borrow();
        row = row.child(
            div()
                .id("bb-shape")
                .h(px(CELL))
                .px(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .rounded(px(5.0))
                .hover(|s| s.bg(rgba(t.hover)))
                .cursor_pointer()
                .child(shape_cell(shape, 18.0, rgba(t.icon)))
                .child(icon::paint(Icon::ChevronDown, rgba(t.icon), 12.0))
                .on_click(move |_, _, _| *open.borrow_mut() = !*open.borrow()),
        );
        if is_open {
            row = row.child(divider(&t));
        }
    }
    if h.text {
        if h.color || h.shape.is_some() {
            row = row.child(divider(&t));
        }
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

    let mut root = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .child(
            div()
                .h(px(HEIGHT))
                .flex()
                .flex_row()
                .items_center()
                .rounded(px(10.0))
                .bg(rgba(t.panel))
                .border_1()
                .border_color(rgba(t.border))
                .child(row),
        );
    // The shape grid, above the bar: every shape's outline (egui's `SHAPE_COLUMNS` grid).
    if *shape_open.borrow() {
        if let Some(current) = h.shape {
            let mut grid = div().flex().flex_wrap().gap(px(2.0)).p(px(6.0)).rounded(px(8.0)).bg(rgba(t.menu)).border_1().border_color(rgba(t.border));
            for s in BoardShape::ALL {
                let cmds = cmds.clone();
                let open = shape_open.clone();
                grid = grid.child(
                    div()
                        .id(gpui::ElementId::from(SharedString::from(format!("bb-shape-{}", s.id()))))
                        .size(px(CELL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .when(s == current, |d| d.bg(rgba(t.selected_row)))
                        .hover(|d| d.bg(rgba(t.hover)))
                        .cursor_pointer()
                        .child(shape_cell(s, 20.0, rgba(t.icon)))
                        .on_click(move |_, _, _| {
                            cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ "shape": s.id() })) });
                            *open.borrow_mut() = false;
                        }),
                );
            }
            let _ = SHAPE_COLUMNS;
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(HEIGHT + 4.0))
                    .left(px(0.0))
                    .w(px(SHAPE_COLUMNS as f32 * (CELL + 2.0) + 12.0))
                    .child(grid),
            );
        }
    }
    Some(root.into_any_element())
}
