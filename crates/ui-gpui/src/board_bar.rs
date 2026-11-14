//! The small bar floating over a board's selection: colours (`board.setColor`), text sizes
//! (`board.setStyle {fontSize}`), the shape grid (`board.setStyle {shape}`) and the connector part
//! (routing, start/end caps, line style). Structure copied from
//! `ui-egui/src/canvas/board_bar.rs`; `cap_art` from `ui-egui/src/panels/right/stroke.rs`.

use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, icon, rgba, tokens};
use figcraft_doc::{BOARD_COLORS, BoardItem, BoardShape, NodeId, Routing, Stamp, StrokeCap};
use figcraft_engine::board::TEXT_SIZES;
use gpui::{AnyElement, Div, IntoElement, Pixels, Point, SharedString, canvas, div, point, prelude::*, px};
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
/// A start/end point menu's width (egui `CAP_W`).
const CAP_W: f32 = 62.0;
/// The line style menu's width (egui `STYLE_W`).
const STYLE_W: f32 = CELL + 12.0;
/// The weights the line style menu offers (egui `WEIGHTS`).
const WEIGHTS: [f64; 4] = [1.0, 2.0, 4.0, 6.0];
/// A dashed connector's dashes and gaps (egui `DASHES`).
const DASHES: [f64; 2] = [8.0, 6.0];

fn color_of(c: figcraft_color::Color) -> crate::canvas::geom::Color32 {
    let [r, g, b, a] = c.to_rgba8();
    crate::canvas::geom::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The selected connectors' line as the bar shows it (the first one's), and their ids.
#[derive(Clone)]
struct Line {
    routing: Routing,
    start: StrokeCap,
    end: StrokeCap,
    weight: f64,
    dashed: bool,
    ids: Vec<NodeId>,
}

/// The selected board objects' colour / text / shape / connector (egui `has`).
#[derive(Default)]
struct Has {
    color: bool,
    text: bool,
    shape: Option<BoardShape>,
    connector: Option<Line>,
}

fn has(ds: &figcraft_engine::DocState) -> Option<Has> {
    let mut h = Has::default();
    for n in ds.selection.iter().filter_map(|id| ds.doc.node(*id)) {
        match &n.board {
            Some(BoardItem::Sticky { .. } | BoardItem::Ink) => h.color = true,
            Some(BoardItem::Shape { shape }) => {
                h.color = true;
                h.text = true;
                h.shape = Some(*shape);
            }
            Some(BoardItem::Connector(c)) => {
                h.color = true;
                let s = &n.stroke;
                let line = h.connector.get_or_insert_with(|| Line {
                    routing: c.routing,
                    start: s.cap_start,
                    end: s.cap_end,
                    weight: s.weight,
                    dashed: !s.dashes.is_empty(),
                    ids: Vec::new(),
                });
                line.ids.push(n.id);
            }
            Some(BoardItem::Label { .. }) => h.text = true,
            _ => {}
        }
    }
    (h.color || h.text || h.shape.is_some() || h.connector.is_some()).then_some(h)
}

fn at(origin: Point<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    point(origin.x + px(x), origin.y + px(y))
}

/// A shape's outline as a GPUI stroke path, offset to `origin`.
fn shape_path_at(shape: BoardShape, size: f32, origin: Point<Pixels>) -> Option<gpui::Path<Pixels>> {
    use figcraft_geom::kurbo::PathEl;
    let bez = shape.path(f64::from(size), f64::from(size));
    let mut b = gpui::PathBuilder::stroke(px(1.2));
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(at(origin, p.x as f32, p.y as f32)),
            PathEl::LineTo(p) => b.line_to(at(origin, p.x as f32, p.y as f32)),
            PathEl::QuadTo(c, p) => b.curve_to(at(origin, p.x as f32, p.y as f32), at(origin, c.x as f32, c.y as f32)),
            PathEl::CurveTo(c1, c2, p) => {
                b.cubic_bezier_to(at(origin, p.x as f32, p.y as f32), at(origin, c1.x as f32, c1.y as f32), at(origin, c2.x as f32, c2.y as f32))
            }
            PathEl::ClosePath => b.close(),
        }
    }
    b.build().ok()
}

/// A shape drawn as its outline in `ink`, `size` square.
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

/// A connector routing icon: straight, elbow or curved, drawn in `ink`.
fn routing_cell(routing: Routing, active: bool, ink: gpui::Rgba) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height);
            let (x0, x1) = (5.0, w - 5.0);
            let y = h / 2.0;
            let mut b = gpui::PathBuilder::stroke(px(if active { 2.0 } else { 1.5 }));
            b.move_to(at(o, x0, y));
            match routing {
                Routing::Straight => b.line_to(at(o, x1, y)),
                Routing::Elbow => {
                    let mx = (x0 + x1) / 2.0;
                    b.line_to(at(o, mx, y));
                    b.line_to(at(o, mx, 7.0));
                    b.line_to(at(o, x1, 7.0));
                }
                Routing::Curved => {
                    let mx = (x0 + x1) / 2.0;
                    b.cubic_bezier_to(at(o, x1, y), at(o, mx, y - 8.0), at(o, mx, y + 8.0));
                }
            }
            if let Ok(p) = b.build() {
                window.paint_path(p, ink);
            }
        },
    )
    .size(px(CELL))
}

/// A line `weight` thick, dashed or not, across the middle (egui `paint_line`).
fn line_sample(weight: f64, dashed: bool, ink: gpui::Rgba) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let w = f32::from(bounds.size.width);
            let y = f32::from(bounds.size.height) / 2.0;
            let mut b = gpui::PathBuilder::stroke(px((weight as f32).clamp(1.0, 5.0)));
            if dashed {
                b = b.dash_array(&[px(5.0), px(3.0)]);
            }
            b.move_to(at(o, 2.0, y));
            b.line_to(at(o, w - 2.0, y));
            if let Ok(p) = b.build() {
                window.paint_path(p, ink);
            }
        },
    )
    .size_full()
}

/// A connector start/end cap drawn like egui `cap_art`: the stroke and its cap shape.
fn cap_art(cap: StrokeCap, start: bool, ink: gpui::Rgba) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let w = f32::from(bounds.size.width);
            let y = f32::from(bounds.size.height) / 2.0;
            let (left, right) = (w - 58.0, w - 14.0);
            let (tip, tail, d) = if start { (left, right, -1.0) } else { (right, left, 1.0) };
            let pt = |back: f32, dy: f32| at(o, tip - d * back, y + dy);
            let weight = if cap == StrokeCap::None { 4.0 } else { 1.5 };
            // The stroke from the plain end into the cap.
            let mut lb = gpui::PathBuilder::stroke(px(weight));
            lb.move_to(at(o, tail, y));
            lb.line_to(pt(0.0, 0.0));
            if let Ok(p) = lb.build() {
                window.paint_path(p, ink);
            }
            // A filled polygon: a triangle or a diamond.
            let mut fill = |pts: &[(f32, f32)]| {
                let mut b = gpui::PathBuilder::fill();
                for (i, (bx, by)) in pts.iter().enumerate() {
                    if i == 0 {
                        b.move_to(pt(*bx, *by));
                    } else {
                        b.line_to(pt(*bx, *by));
                    }
                }
                b.close();
                if let Ok(p) = b.build() {
                    window.paint_path(p, ink);
                }
            };
            match cap {
                StrokeCap::None => {}
                StrokeCap::Round => fill(&[(1.0, -2.0), (3.0, -2.0), (3.0, 2.0), (1.0, 2.0)]),
                StrokeCap::Square => fill(&[(-2.0, -2.0), (0.0, -2.0), (0.0, 2.0), (-2.0, 2.0)]),
                StrokeCap::LineArrow => fill(&[(5.5, -5.5), (0.0, 0.0), (5.5, 5.5), (4.0, 0.0)]),
                StrokeCap::TriangleArrow => fill(&[(0.0, 0.0), (8.0, -4.5), (8.0, 4.5)]),
                StrokeCap::ReversedTriangle => fill(&[(8.0, 0.0), (0.0, -4.5), (0.0, 4.5)]),
                StrokeCap::CircleFilled => fill(&[(0.0, -4.0), (4.0, 0.0), (0.0, 4.0), (-4.0, 0.0)]),
                StrokeCap::DiamondFilled => fill(&[(0.0, 0.0), (5.0, -5.0), (10.0, 0.0), (5.0, 5.0)]),
            }
        },
    )
    .w(px(CAP_W))
    .h(px(CELL))
}

/// A thin separator between two parts.
// ── choices: what a board tool's flyout offers (ui-egui/src/canvas/board_bar.rs) ─────────────

/// A choice a board tool's flyout offers: a sticky note's colour, a shape, or a stamp.
pub(crate) struct Choice {
    /// The label as the catalogue has it (English); translate with `tr` when drawing.
    pub label: &'static str,
    /// What running the tool with this choice asks for.
    pub params: serde_json::Value,
    /// Whether the tool is already on this choice.
    pub on: bool,
    pub mark: Mark,
}

/// How a choice draws itself.
#[derive(Clone, Copy)]
pub(crate) enum Mark {
    Color(figcraft_color::Color),
    Shape(BoardShape),
    Stamp(Stamp),
}

/// The choices `options` offers, with the tool's current one marked (`choices` in the original).
pub(crate) fn choices(options: crate::toolbar::Options, board: &figcraft_engine::board::BoardTool) -> Vec<Choice> {
    use crate::toolbar::Options;
    match options {
        Options::Colors => BOARD_COLORS
            .iter()
            .map(|(n, c)| Choice { label: color_label(n), params: json!({ "color": n }), on: board.sticky == *c, mark: Mark::Color(*c) })
            .collect(),
        Options::Shapes => BoardShape::ALL
            .iter()
            .map(|b| Choice { label: b.label(), params: json!({ "shape": b.id() }), on: board.shape == *b, mark: Mark::Shape(*b) })
            .collect(),
        Options::Stamps => Stamp::ALL
            .iter()
            .map(|b| Choice { label: b.label(), params: json!({ "stamp": b.id() }), on: board.stamp == *b, mark: Mark::Stamp(*b) })
            .collect(),
        Options::None => Vec::new(),
    }
}

/// A board colour's name as the UI says it (`color_label` in the original).
fn color_label(name: &str) -> &'static str {
    match name {
        "yellow" => "Yellow",
        "orange" => "Orange",
        "red" => "Red",
        "pink" => "Pink",
        "violet" => "Violet",
        "blue" => "Blue",
        "green" => "Green",
        _ => "Gray",
    }
}

/// A choice's mark drawn in a `size` square: a colour chip, a shape's outline, or a stamp badge
/// (`paint_choice` in the original).
pub(crate) fn choice_mark(mark: Mark, size: f32, ink: gpui::Rgba) -> AnyElement {
    match mark {
        Mark::Color(c) => {
            let t = tokens();
            div().size(px(size)).rounded(px(4.0)).bg(rgba(color_of(c))).border_1().border_color(rgba(t.border)).into_any_element()
        }
        Mark::Shape(s) => canvas(
            move |_, _, _| (),
            move |bounds, _, window, _| {
                if let Some(path) = shape_path_at(s, size - 2.0, at(bounds.origin, 1.0, 1.0)) {
                    window.paint_path(path, ink);
                }
            },
        )
        .size(px(size))
        .into_any_element(),
        Mark::Stamp(s) => {
            let parts: Vec<(figcraft_geom::BezPath, bool)> =
                s.mark().iter().filter_map(|m| figcraft_geom::BezPath::from_svg(m.d).ok().map(|b| (b, m.filled))).collect();
            let disc = color_of(s.color());
            canvas(
                move |_, _, _| (),
                move |bounds, _, window, _| {
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    let r = w.min(h) / 2.0;
                    let o = point(bounds.origin.x + px(w / 2.0 - r), bounds.origin.y + px(h / 2.0 - r));
                    // A square whose four corners are r **is** the same circle, and it is the one
                    // primitive a filled Bézier circle can't spoil (see `canvas::painter`).
                    let quad = gpui::fill(gpui::Bounds { origin: o, size: gpui::size(px(r * 2.0), px(r * 2.0)) }, rgba(disc))
                        .corner_radii(gpui::Corners { top_left: px(r), top_right: px(r), bottom_right: px(r), bottom_left: px(r) });
                    window.paint_quad(quad);
                    outline_marks(&parts, figcraft_doc::STAMP_SIZE, r * 2.0, gpui::rgb(0xffffff), 3.0, o, window);
                },
            )
            .size(px(size))
            .into_any_element()
        }
    }
}

/// SVG path data outlines on a `size`-unit box, drawn into a `box_size` square at `origin`: each
/// part filled, or stroked `width` box units wide (`ui-egui/src/icons/draw.rs::paint_outlines`).
fn outline_marks(
    parts: &[(figcraft_geom::BezPath, bool)],
    size: f64,
    box_size: f32,
    ink: gpui::Rgba,
    width: f32,
    origin: Point<Pixels>,
    window: &mut gpui::Window,
) {
    use figcraft_geom::kurbo::PathEl;
    let s = box_size / size as f32;
    let o = point(origin.x + px(box_size / 2.0 - size as f32 * s / 2.0), origin.y + px(box_size / 2.0 - size as f32 * s / 2.0));
    for (path, filled) in parts {
        let mut b = if *filled { gpui::PathBuilder::fill() } else { gpui::PathBuilder::stroke(px((width * s).max(0.5))) };
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => b.move_to(at(o, p.x as f32 * s, p.y as f32 * s)),
                PathEl::LineTo(p) => b.line_to(at(o, p.x as f32 * s, p.y as f32 * s)),
                PathEl::QuadTo(c, p) => b.curve_to(at(o, p.x as f32 * s, p.y as f32 * s), at(o, c.x as f32 * s, c.y as f32 * s)),
                PathEl::CurveTo(c1, c2, p) => b.cubic_bezier_to(
                    at(o, p.x as f32 * s, p.y as f32 * s),
                    at(o, c1.x as f32 * s, c1.y as f32 * s),
                    at(o, c2.x as f32 * s, c2.y as f32 * s),
                ),
                PathEl::ClosePath => b.close(),
            }
        }
        if let Ok(p) = b.build() {
            window.paint_path(p, ink);
        }
    }
}

fn divider(t: &crate::theme::Tokens) -> impl IntoElement {
    div().flex_none().w(px(1.0)).h(px(20.0)).mx(px(2.0)).bg(rgba(t.border))
}

/// A small toggle button (`cell` square) with `el` inside.
fn cell_button(id: impl Into<gpui::ElementId>, el: impl IntoElement) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(CELL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(tokens().hover)))
        .child(el)
}

/// The bar above a board selection, while nothing is being dragged and the file can be edited.
pub(crate) fn bar(app: &crate::CanvasApp, cmds: Cmds, shape_open: &Rc<RefCell<bool>>, line_open: &Rc<RefCell<u8>>) -> Option<AnyElement> {
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
    if let Some(line) = &h.connector {
        w += 3.0 * (CELL + 2.0) + 8.0 + 2.0 * CAP_W + 2.0 + 8.0 + STYLE_W + PART_END;
        let _ = line;
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
        let open = shape_open.clone();
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
                .on_click(move |_, _, _| {
                    // 先读再写（同 `home::cycle_bool`：一条表达式里会 RefCell 双重借用 panic）。
                    let next = !*open.borrow();
                    *open.borrow_mut() = next;
                }),
        );
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
    // ── The connector part: routing, start/end caps, line style (egui `line_part`) ──
    if let Some(line) = &h.connector {
        if h.color || h.shape.is_some() || h.text {
            row = row.child(divider(&t));
        }
        for (i, (routing, id)) in [(Routing::Straight, "straight"), (Routing::Elbow, "elbow"), (Routing::Curved, "curved")].into_iter().enumerate() {
            let cmds = cmds.clone();
            let active = line.routing == routing;
            row = row.child(
                cell_button(("bb-line", i), routing_cell(routing, active, rgba(if active { t.accent } else { t.icon })))
                    .when(active, |d| d.bg(rgba(t.selected_row)))
                    .on_click(move |_, _, _| cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ "style": id })) })),
            );
        }
        // Start / end cap menus.
        for (slot, start, cap) in [(1u8, true, line.start), (2u8, false, line.end)] {
            let open = line_open.clone();
            let is_open = *line_open.borrow() == slot;
            row = row.child(
                cell_button(("bb-cap", slot as usize), cap_art(cap, start, rgba(t.icon)))
                    .when(is_open, |d| d.bg(rgba(t.selected_row)))
                    .w(px(CAP_W))
                    .on_click(move |_, _, _| {
                        let mut o = open.borrow_mut();
                        *o = if *o == slot { 0 } else { slot };
                    }),
            );
        }
        // The line style menu (solid / dash / weights).
        let open = line_open.clone();
        let is_open = *line_open.borrow() == 3;
        row = row.child(
            cell_button("bb-style", line_sample(line.weight, line.dashed, rgba(t.icon)))
                .when(is_open, |d| d.bg(rgba(t.selected_row)))
                .w(px(STYLE_W))
                .on_click(move |_, _, _| {
                    let mut o = open.borrow_mut();
                    *o = if *o == 3 { 0 } else { 3 };
                }),
        );
    }

    let mut root = div().absolute().left(px(x)).top(px(y)).child(
        div().h(px(HEIGHT)).flex().flex_row().items_center().rounded(px(10.0)).bg(rgba(t.panel)).border_1().border_color(rgba(t.border)).child(row),
    );
    // The shape grid, above the bar.
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
            root = root.child(div().absolute().bottom(px(HEIGHT + 4.0)).left(px(0.0)).w(px(SHAPE_COLUMNS as f32 * (CELL + 2.0) + 12.0)).child(grid));
        }
    }
    // The connector cap / style popovers, above the bar.
    let open = *line_open.borrow();
    if open != 0 {
        if let Some(line) = &h.connector {
            let mut pop = div().flex().flex_col().gap(px(2.0)).p(px(6.0)).rounded(px(8.0)).bg(rgba(t.menu)).border_1().border_color(rgba(t.border));
            match open {
                1 | 2 => {
                    let start = open == 1;
                    for cap in StrokeCap::ALL {
                        let cmds = cmds.clone();
                        let ids = line.ids.clone();
                        let key = if start { "startCap" } else { "endCap" };
                        let close = line_open.clone();
                        pop = pop.child(
                            div()
                                .id(gpui::ElementId::from(SharedString::from(format!("bb-cap-{}-{}", start, cap.label()))))
                                .h(px(CELL))
                                .flex()
                                .items_center()
                                .rounded(px(5.0))
                                .hover(|s| s.bg(rgba(t.hover)))
                                .cursor_pointer()
                                .child(cap_art(cap, start, rgba(t.icon)))
                                .on_click(move |_, _, _| {
                                    let _ = &ids;
                                    cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ key: cap })) });
                                    *close.borrow_mut() = 0;
                                }),
                        );
                    }
                }
                _ => {
                    for (dashed, label) in [(false, "Solid"), (true, "Dash")] {
                        let cmds = cmds.clone();
                        let close = line_open.clone();
                        pop = pop.child(
                            div()
                                .id(gpui::ElementId::from(SharedString::from(format!("bb-dash-{dashed}"))))
                                .h(px(CELL))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(5.0))
                                .hover(|s| s.bg(rgba(t.hover)))
                                .cursor_pointer()
                                .child(div().w(px(44.0)).h(px(CELL)).child(line_sample(2.0, dashed, rgba(t.icon))))
                                .child(div().text_size(px(11.0)).text_color(rgba(t.text)).child(SharedString::from(label)))
                                .on_click(move |_, _, _| {
                                    let dashes: &[f64] = if dashed { &DASHES } else { &[] };
                                    cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ "dashes": dashes })) });
                                    *close.borrow_mut() = 0;
                                }),
                        );
                    }
                    for w in WEIGHTS {
                        let cmds = cmds.clone();
                        let close = line_open.clone();
                        pop = pop.child(
                            div()
                                .id(gpui::ElementId::from(SharedString::from(format!("bb-weight-{w}"))))
                                .h(px(CELL))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(5.0))
                                .hover(|s| s.bg(rgba(t.hover)))
                                .cursor_pointer()
                                .child(div().w(px(44.0)).h(px(CELL)).child(line_sample(w, false, rgba(t.icon))))
                                .child(div().text_size(px(11.0)).text_color(rgba(t.text)).child(SharedString::from(format!("{w}"))))
                                .on_click(move |_, _, _| {
                                    cmds.push(Cmd { id: "board.setStyle", params: Some(json!({ "strokeWeight": w })) });
                                    *close.borrow_mut() = 0;
                                }),
                        );
                    }
                }
            }
            root = root.child(div().absolute().bottom(px(HEIGHT + 4.0)).left(px(0.0)).w(px(200.0)).child(pop));
        }
    }
    Some(root.into_any_element())
}
