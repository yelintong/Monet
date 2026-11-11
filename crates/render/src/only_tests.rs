//! `Options::only`: the layers a canvas draws itself while they are dragged, rendered on their own
//! — but down the page as usual, so a layer dragged half out of a clipping frame is still cut off
//! by it (what the editor showed before the canvas took the layer over).

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind};

use crate::{Options, View, render_view};

fn pixel(p: &crate::Pixmap, x: u32, y: u32) -> [u8; 4] {
    p.pixel(x, y).map(|c| c.demultiply()).map(|c| [c.red(), c.green(), c.blue(), c.alpha()]).unwrap_or_default()
}

fn solid(d: &mut Document, id: figcraft_doc::NodeId, hex: &str) {
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
}

#[test]
fn only_draws_the_listed_layers_and_nothing_else() {
    let mut d = Document::new("only");
    let page = d.first_page().unwrap();
    let a = d.create(page, NodeKind::Rectangle, "A", 10.0, 10.0, 50.0, 50.0).unwrap();
    let b = d.create(page, NodeKind::Rectangle, "B", 100.0, 10.0, 50.0, 50.0).unwrap();
    let c = d.create(page, NodeKind::Rectangle, "C", 190.0, 10.0, 50.0, 50.0).unwrap();
    for (id, hex) in [(a, "#FF0000"), (b, "#00FF00"), (c, "#0000FF")] {
        solid(&mut d, id, hex);
    }
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 260, height: 80 };
    let bg = Color::DEFAULT_PAGE.to_rgba8();

    let two = render_view(&d, page, &view, &Options { only: vec![a, c], ..Default::default() }).unwrap();
    assert_eq!(pixel(&two, 30, 30), [255, 0, 0, 255], "A drawn");
    assert_eq!(pixel(&two, 220, 30), [0, 0, 255, 255], "C drawn");
    assert_eq!(pixel(&two, 120, 30), bg, "B left out");
}

#[test]
fn only_draws_a_listed_container_and_its_children() {
    // A container dragged or resized draws as one stand-in: `only: [frame]` must paint the frame
    // **and what is inside it**, not the frame alone (the bug where a scaled container's contents
    // vanished).
    let mut d = Document::new("container");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 20.0, 20.0, 100.0, 100.0).unwrap();
    let child = d.create(frame, NodeKind::Rectangle, "Child", 30.0, 30.0, 40.0, 40.0).unwrap();
    let other = d.create(page, NodeKind::Rectangle, "Other", 200.0, 20.0, 40.0, 40.0).unwrap();
    solid(&mut d, frame, "#FFFFFF");
    solid(&mut d, child, "#FF0000");
    solid(&mut d, other, "#00FF00");
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 300, height: 160 };
    let bg = Color::DEFAULT_PAGE.to_rgba8();

    let only = render_view(&d, page, &view, &Options { only: vec![frame], ..Default::default() }).unwrap();
    assert_eq!(pixel(&only, 25, 25), [255, 255, 255, 255], "the container's own fill is drawn");
    assert_eq!(pixel(&only, 60, 60), [255, 0, 0, 255], "the child inside the container is drawn");
    assert_eq!(pixel(&only, 210, 40), bg, "a layer outside the container is left out");
}

#[test]
fn only_still_clips_a_layer_by_the_frame_it_is_in() {
    let mut d = Document::new("clip");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 0.0, 0.0, 100.0, 100.0).unwrap();
    // Half out of the frame on the right.
    let child = d.create(frame, NodeKind::Rectangle, "Child", 50.0, 10.0, 100.0, 80.0).unwrap();
    solid(&mut d, child, "#FF0000");
    d.get_mut(frame).unwrap().clip = true;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 120 };
    let bg = Color::DEFAULT_PAGE.to_rgba8();

    let only = render_view(&d, page, &view, &Options { only: vec![child], ..Default::default() }).unwrap();
    assert_eq!(pixel(&only, 60, 50), [255, 0, 0, 255], "the half inside the frame");
    assert_eq!(pixel(&only, 120, 50), bg, "the half past the frame's edge is cut off");
    // The frame itself isn't one of the layers drawn: its own fill stays out.
    assert_eq!(pixel(&only, 10, 110), bg, "the frame's own paint isn't in the stand-in");

    // Out of the frame (where a drag leaves it), the same layer shows whole.
    d.reparent(child, page, None).unwrap();
    let out = render_view(&d, page, &view, &Options { only: vec![child], ..Default::default() }).unwrap();
    assert_eq!(pixel(&out, 120, 50), [255, 0, 0, 255], "nothing clips it on the page");
}

#[test]
fn only_clips_with_the_view_off_the_page_origin() {
    let mut d = Document::new("clip2");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 40.0, 40.0, 420.0, 320.0).unwrap();
    // Sticking out of the frame to the left, the way a drag takes it.
    let child = d.create(frame, NodeKind::Rectangle, "Child", -110.0, 40.0, 220.0, 140.0).unwrap();
    solid(&mut d, child, "#FF0000");
    d.get_mut(frame).unwrap().clip = true;
    // The grab's view: the layer's own bounds, off the page origin.
    let view = View { origin: (-90.0, 60.0), scale: 1.0, width: 300, height: 200 };
    let bg = Color::DEFAULT_PAGE.to_rgba8();
    let only = render_view(&d, page, &view, &Options { only: vec![child], ..Default::default() }).unwrap();
    // Page x = view.origin.0 + x. Frame starts at 40 → x = 130.
    assert_eq!(pixel(&only, 100, 70), bg, "left of the frame: cut off");
    assert_eq!(pixel(&only, 160, 70), [255, 0, 0, 255], "inside the frame: drawn");
}
