//! What the canvas draws itself while a gesture runs (`ToolState::live_layers`): the layers the
//! gesture moves, stretches or turns — including Alt-drag's copies and a many-layer selection.

use serde_json::json;

use crate::Session;
use crate::LiveDraw;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn id(v: serde_json::Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 100, "height": 80})).unwrap())
}

fn down(s: &mut Session, at: (f64, f64), alt: bool) {
    let mut ev = PointerEvent::at(PointerKind::Down, at.0, at.1, 1.0);
    ev.alt = alt;
    s.pointer(&ev).unwrap();
}

fn at(s: &mut Session, kind: PointerKind, to: (f64, f64)) {
    s.pointer(&PointerEvent::at(kind, to.0, to.1, 1.0)).unwrap();
}

#[test]
fn a_move_lives_in_the_layers_it_moves() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    s.set_selection(vec![a]);
    down(&mut s, (50.0, 40.0), false);
    at(&mut s, PointerKind::Move, (60.0, 50.0));
    assert_eq!(s.tool.live_layers(), Some((vec![a], LiveDraw::Translate)));
    at(&mut s, PointerKind::Up, (60.0, 50.0));
    assert_eq!(s.tool.live_layers(), None, "nothing is being dragged any more");
}

#[test]
fn a_many_layer_move_lives_in_all_of_them() {
    let mut s = Session::new();
    let (a, b) = (rect(&mut s, 0.0, 0.0), rect(&mut s, 200.0, 0.0));
    s.set_selection(vec![a, b]);
    down(&mut s, (50.0, 40.0), false);
    at(&mut s, PointerKind::Move, (60.0, 50.0));
    let (ids, draw) = s.tool.live_layers().expect("a multi-selection drag draws itself too");
    assert_eq!(ids, vec![a, b]);
    assert_eq!(draw, LiveDraw::Translate);
}

#[test]
fn an_alt_drag_lives_in_the_copies_it_makes() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    s.set_selection(vec![a]);
    down(&mut s, (50.0, 40.0), true);
    at(&mut s, PointerKind::Move, (60.0, 50.0));
    let (first, draw) = s.tool.live_layers().expect("the copy is what the drag shows");
    assert_eq!(draw, LiveDraw::Translate);
    assert_ne!(first, vec![a], "the original stays put; the canvas draws the copy");
    assert_eq!(first, s.selection(), "the copy is what got selected");
    // The copies are made afresh from the snapshot at every event, and keep their ids.
    at(&mut s, PointerKind::Move, (80.0, 70.0));
    let (second, _) = s.tool.live_layers().unwrap();
    assert_eq!(second, first, "the copy keeps its id, so the canvas keeps one stand-in");
}

#[test]
fn a_resize_and_a_turn_redraw_the_layer_every_frame() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    s.set_selection(vec![a]);
    // The bottom-right handle of a 100×80 box.
    down(&mut s, (100.0, 80.0), false);
    at(&mut s, PointerKind::Move, (140.0, 120.0));
    assert_eq!(s.tool.live_layers(), Some((vec![a], LiveDraw::Reshape)));
    at(&mut s, PointerKind::Up, (140.0, 120.0));

    // A turn: the rotate handle sits a little outside a corner.
    let b = s.doc().unwrap().world_bounds(a).unwrap();
    s.set_selection(vec![a]);
    down(&mut s, (b.x1 + 8.0, b.y0 - 8.0), false);
    at(&mut s, PointerKind::Move, (b.x1 + 6.0, b.y0 - 20.0));
    assert_eq!(s.tool.live_layers(), Some((vec![a], LiveDraw::Reshape)));
}

#[test]
fn only_a_resize_reflows_its_parent_and_siblings() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);

    // A move only shifts the bitmap: nothing around it reflows.
    s.set_selection(vec![a]);
    down(&mut s, (50.0, 40.0), false);
    at(&mut s, PointerKind::Move, (60.0, 50.0));
    assert!(!s.tool.live_reflows(), "a move doesn't reflow");
    at(&mut s, PointerKind::Up, (60.0, 50.0));

    // A resize reflows: a hugging frame grows with the child, the layout pushes siblings along.
    let b = s.doc().unwrap().world_bounds(a).unwrap();
    s.set_selection(vec![a]);
    down(&mut s, (b.x1, b.y1), false);
    at(&mut s, PointerKind::Move, (b.x1 + 40.0, b.y1 + 40.0));
    assert!(s.tool.live_reflows(), "a resize reflows");
    at(&mut s, PointerKind::Up, (b.x1 + 40.0, b.y1 + 40.0));

    // A turn only changes the layer's own geometry.
    let c = s.doc().unwrap().world_bounds(a).unwrap();
    s.set_selection(vec![a]);
    down(&mut s, (c.x1 + 8.0, c.y0 - 8.0), false);
    at(&mut s, PointerKind::Move, (c.x1 + 6.0, c.y0 - 20.0));
    assert!(!s.tool.live_reflows(), "a turn doesn't reflow");
}

#[test]
fn dragging_a_layer_half_out_of_a_frame_leaves_it_in_the_frame() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 100, "y": 100, "width": 300, "height": 300})).unwrap());
    let a = id(s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100, "parent": f})).unwrap());
    s.set_selection(vec![a]);
    down(&mut s, (200.0, 200.0), false);
    // Pointer out past the frame's left edge; half the layer is still inside it.
    at(&mut s, PointerKind::Move, (60.0, 200.0));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(a).unwrap().parent, Some(f), "still a child of the frame, so still clipped by it");
    let b = doc.world_bounds(a).unwrap();
    assert!(b.x0 < 100.0 && b.x1 > 100.0, "half out: {b:?}");
}
