//! The style picker: name folders, new style names, and the popover drawn headless (list, grid,
//! editor), committing a typed name when it closes.

use egui::{Id, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use super::*;
use crate::{FigApp, NoServices};

#[test]
fn names_split_into_folders() {
    assert_eq!((folder("Brand/Blue/500"), leaf("Brand/Blue/500")), ("Brand/Blue", "500"));
    assert_eq!((folder("Danger"), leaf("Danger")), ("", "Danger"));
    assert_eq!((folder(" Brand / Primary "), leaf(" Brand / Primary ")), ("Brand", "Primary"));
    let mut d = Document::new("d");
    assert_eq!(next_name(&d, "Color"), "Color 1");
    for n in ["Color 1", "Color 2"] {
        d.styles.push(Style {
            id: n.into(),
            name: n.into(),
            description: String::new(),
            kind: StyleKind::Paint { paints: vec![] },
            bound: Default::default(),
        });
    }
    assert_eq!(next_name(&d, "Color"), "Color 3");
    assert!(StyleSlot::Fill.holds(&StyleKind::Paint { paints: vec![] }) && !StyleSlot::Effect.holds(&StyleKind::Paint { paints: vec![] }));
    for s in [StyleSlot::Fill, StyleSlot::Stroke, StyleSlot::Effect, StyleSlot::Grid, StyleSlot::Text] {
        assert_eq!(StyleSlot::parse(s.key()), Some(s));
    }
    assert!(StyleSlot::Text.holds(&StyleKind::Text { style: Default::default() }) && StyleSlot::Text.has_grid());
}

/// The Local styles list's lines as text: a folder's label, or a style's last part, indented by
/// the level it sits at.
fn shape(lines: &[Entry]) -> Vec<String> {
    lines
        .iter()
        .map(|e| match e {
            Entry::Folder { label, depth, .. } => format!("{}{label}", "  ".repeat(*depth)),
            Entry::Style(s, depth) => format!("{}·{}", "  ".repeat(*depth), leaf(&s.name)),
        })
        .collect()
}

#[test]
fn folders_nest_as_deep_as_their_names_go() {
    let st = |id: &str, name: &str| Style {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![] },
        bound: Default::default(),
    };
    let styles = [st("a", "Top"), st("b", "Brand/Primary"), st("c", "Brand/Blue/500"), st("d", "Loose")];
    // A name's folders are folders even when nothing else names them: "Brand" for "Brand/Blue".
    assert_eq!(shape(&entries(styles.iter(), |_| false)), ["·Top", "·Loose", "Brand", "  ·Primary", "  Blue", "    ·500"]);
    // Folding a folder hides its whole branch, not just the styles named directly in it.
    assert_eq!(shape(&entries(styles.iter(), |p| p == "Brand")), ["·Top", "·Loose", "Brand"]);
}

fn frame(ctx: &egui::Context, app: &mut FigApp, t: &mut f64) {
    frame_with(ctx, app, t, vec![]);
}

fn frame_with(ctx: &egui::Context, app: &mut FigApp, t: &mut f64, events: Vec<egui::Event>) {
    *t += 1.0 / 60.0;
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(*t), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    out.textures_delta.clear();
}

#[test]
fn picker_draws_every_view_and_commits_a_typed_name_on_close() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    for (name, c) in [("Brand/Primary", "#0D99FF"), ("Brand/Accent", "#FFC700"), ("Danger", "#F24822")] {
        s.execute("style.create", &json!({"type": "paint", "name": name, "value": [{"type": "solid", "color": c}]})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("style.apply", &json!({"ids": [r], "style": "Brand/Primary", "target": "fill"})).unwrap();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    for slot in [StyleSlot::Fill, StyleSlot::Stroke, StyleSlot::Effect, StyleSlot::Grid, StyleSlot::Text] {
        open_for_selection(&mut app, slot, None);
        for grid in [false, true] {
            app.ui.style_grid = grid;
            for _ in 0..3 {
                frame(&ctx, &mut app, &mut t);
            }
            assert_eq!(app.ui.style_picker.as_ref().map(Picker::slot), Some(slot), "{slot:?} stays open");
        }
    }
    // Folders fold.
    app.ui.style_folds.insert("Brand".into());
    frame(&ctx, &mut app, &mut t);
    // The editor; a name typed there is written when the picker closes (here: a new selection).
    open_for_selection(&mut app, StyleSlot::Fill, Some("Brand/Accent"));
    frame(&ctx, &mut app, &mut t);
    let edit = app.ui.style_picker.as_mut().and_then(|p| p.edit.as_mut()).expect("editing");
    edit.name = "Brand/Gold".into();
    app.run("select.none", json!({}));
    frame(&ctx, &mut app, &mut t);
    assert!(app.ui.style_picker.is_none(), "a new selection closes the picker");
    let doc = app.session.doc().unwrap();
    assert!(doc.style("Brand/Gold").is_some(), "{:?}", doc.styles.iter().map(|s| &s.name).collect::<Vec<_>>());
}

/// A session with a text layer and text styles "Type/Heading" and "Type/Body" (the layer uses Body).
fn text_session() -> (Session, figcraft_doc::NodeId) {
    let mut s = Session::new();
    let t = figcraft_engine::params::id_of(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute(
        "style.create",
        &json!({"type": "text", "name": "Type/Heading", "value": {"fontFamily": "Inter", "fontStyle": "Bold", "fontSize": 32}}),
    )
    .unwrap();
    s.execute("style.create", &json!({"type": "text", "name": "Type/Body", "value": {"fontFamily": "Inter", "fontSize": 14}})).unwrap();
    s.execute("style.apply", &json!({"ids": [t], "style": "Type/Body"})).unwrap();
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    (s, t)
}

#[test]
fn text_styles_preview_on_hover_and_apply_on_click() {
    let (s, t) = text_session();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    open_for_selection(&mut app, StyleSlot::Text, None);
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut time);
    }
    let area = ctx.memory(|m| m.area_rect(Id::new("style_picker_area"))).expect("the picker shows");
    let size = |app: &FigApp| app.session.doc().unwrap().node(t).unwrap().kind.text().unwrap().style.font_size;
    let undo = app.session.ds().unwrap().history.undo.len();
    // Down the popover until a style row previews on the canvas: the heading (listed first).
    let mut over = None;
    for y in (area.min.y as i32..area.max.y as i32).step_by(4) {
        let p = Pos2::new(area.center().x - 40.0, y as f32);
        frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(p)]);
        frame(&ctx, &mut app, &mut time);
        if figcraft_engine::textpreview::active(&app.session) {
            over = Some(p);
            break;
        }
    }
    let p = over.expect("a hovered text style previews");
    assert_eq!(size(&app), 32.0, "the canvas shows the hovered style");
    assert_eq!(app.session.ds().unwrap().history.undo.len(), undo, "a preview is no edit");
    // Away from the popover the preview ends.
    frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(Pos2::new(600.0, 600.0))]);
    frame(&ctx, &mut app, &mut time);
    assert!(!figcraft_engine::textpreview::active(&app.session));
    assert_eq!(size(&app), 14.0);
    // A click applies it: one undo step, and the picker closes.
    let press = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(p)]);
    frame_with(&ctx, &mut app, &mut time, vec![press(true)]);
    frame_with(&ctx, &mut app, &mut time, vec![press(false)]);
    frame(&ctx, &mut app, &mut time);
    assert_eq!(size(&app), 32.0);
    assert_eq!(app.session.ds().unwrap().history.undo.len(), undo + 1);
    assert!(app.ui.style_picker.is_none());
    let doc = app.session.doc().unwrap();
    assert_eq!(doc.node(t).unwrap().styles.text.as_deref(), doc.style("Type/Heading").map(|s| s.id.as_str()));
}

#[test]
fn chars_linked_to_several_styles_show_mixed() {
    let (mut s, t) = text_session();
    s.execute("style.apply", &json!({"ids": [t], "style": "Type/Heading", "start": 0, "end": 2})).unwrap();
    let app = FigApp::new(s, Box::new(NoServices));
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), None, "mixed");
    let mut app = app;
    // Editing the first two chars: they link to one style.
    figcraft_engine::textedit::begin(&mut app.session, t, Some(0), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut app.session, 2, true).unwrap();
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), Some(doc.style("Type/Heading").map(|s| s.id.clone())));
    // The panel draws the chip ("Mixed" when the edit spans both).
    figcraft_engine::textedit::set_caret(&mut app.session, 4, true).unwrap();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut time);
    }
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), None);
}

#[test]
fn a_click_in_the_colour_picker_keeps_the_style_editor_open() {
    use crate::layers_drag_tests::Ui;
    let mut s = Session::new();
    s.execute("style.create", &json!({"type": "paint", "name": "Brand", "value": [{"type": "solid", "color": "#0D99FF"}]})).unwrap();
    let mut u = Ui::new(s);
    open_for_selection(&mut u.app, StyleSlot::Fill, Some("Brand"));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let id = u.app.session.doc().unwrap().style("Brand").unwrap().id.clone();
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Style { id, index: 0 });
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let cp = u.app.ui.picker_popover.as_ref().expect("colour picker shows").rect;
    let p = cp.center();
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
    assert!(u.app.ui.style_picker.is_some(), "the style editor stays open while the colour picker is used");
    assert!(
        matches!(u.app.ui.picker, Some(crate::colorpicker::PickerTarget::Style { .. })),
        "the colour picker stays open too: {:?}",
        u.app.ui.picker
    );
}

#[test]
fn local_style_drops_join_the_folder_the_line_is_in() {
    let st = |id: &str, name: &str| Style {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![] },
        bound: Default::default(),
    };
    let (a, b, c) = (st("a", "Top"), st("b", "Brand/One"), st("c", "Brand/Two"));
    let mut doc = Document::new("d");
    doc.styles = vec![a.clone(), b.clone(), c.clone()];
    let shown = [&a, &b, &c];
    // `Top` dropped between One and Two: the line is in Brand's rows, so it joins Brand.
    assert_eq!(drop_move(&doc, &shown, 0, 1, false), Some(json!({"styles": ["a"], "before": "c", "name": "Brand/Top"})));
    // Two dropped above Top: out of Brand, before Top.
    assert_eq!(drop_move(&doc, &shown, 2, 0, false), Some(json!({"styles": ["c"], "before": "a", "name": "Two"})));
    // One dropped last: the row above gives the folder, so it stays in Brand, at the end.
    assert_eq!(drop_move(&doc, &shown, 1, 2, false), Some(json!({"styles": ["b"]})));
    // Two dropped on the top half of Brand's header: that ends Top's rows, so it joins Top (and
    // loses the folder from its name) without moving.
    assert_eq!(drop_move(&doc, &shown, 2, 1, true), Some(json!({"styles": ["c"], "before": "b", "name": "Two"})));
    // A folder that already has that last part numbers it instead of repeating the name.
    let (d, e) = (st("d", "Other/One"), st("e", "Brand/Three"));
    doc.styles = vec![b.clone(), e.clone(), d.clone()];
    let shown = [&b, &e, &d];
    assert_eq!(drop_move(&doc, &shown, 2, 1, false), Some(json!({"styles": ["d"], "before": "e", "name": "Brand/One 2"})));
    // The last row of a group dropped on the end of the group above joins it in place: nothing
    // follows, so only its name changes.
    assert_eq!(drop_move(&doc, &shown, 2, 2, true), Some(json!({"styles": ["d"], "name": "Brand/One 2"})));
    assert_eq!(drop_move(&doc, &shown, 5, 0, false), None);
}

#[test]
fn local_styles_list_draws_by_type_and_folder() {
    let mut s = Session::new();
    for (ty, name, value) in
        [("paint", "Brand/Primary", json!(["#0D99FF"])), ("paint", "Danger", json!(["#F24822"])), ("text", "Type/Body", json!({"fontSize": 14}))]
    {
        s.execute("style.create", &json!({"type": ty, "name": name, "value": value})).unwrap();
    }
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    // Nothing selected: the Design panel shows the page, with Local styles; a folder rename in
    // progress draws its field.
    ctx.data_mut(|d| d.insert_temp(folder_rename_id(), "Brand".to_string()));
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut t);
    }
    edit_local_for_test(&mut app, &ctx);
    frame(&ctx, &mut app, &mut t);
    let editing = app.ui.style_picker.as_ref().and_then(|p| p.edit.as_ref()).map(|e| e.name.clone());
    assert_eq!(editing.as_deref(), Some("Danger"), "a local style opens in the style editor");
}

fn edit_local_for_test(app: &mut FigApp, ctx: &egui::Context) {
    let mut out = ctx.run_ui(RawInput::default(), |ui| edit_local(app, ui, StyleSlot::Fill, "Danger"));
    out.textures_delta.clear();
}

/// The `+`s the Local styles list draws, as the 16 pt box each one's arms span: the section's, one
/// on each type's caption, and one on every folder's header. Their right edge sits 8 pt in from the
/// panel's, and each is centred on the row it belongs to.
#[test]
fn local_styles_pluses_reach_the_panels_right_edge() {
    let mut s = Session::new();
    // "Base" is a style of its own; "Brand/Blue/500" names two folders, each holding a style.
    for name in ["Base", "Brand/Blue/500"] {
        s.execute("style.create", &json!({"type": "paint", "name": name, "value": "#0D99FF"})).unwrap();
    }
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let mut out = None;
    for _ in 0..3 {
        out = Some(frame_shapes(&ctx, &mut app, &mut t));
    }
    let shapes = out.expect("frames drawn");
    let icon = Tokens::get(&ctx).icon_secondary;
    // One on the section, one on the Color styles caption, one on each of the two folders.
    let pluses = pluses(&shapes, icon);
    assert_eq!(pluses.len(), 4, "{pluses:?}");
    // The panel's content edge: the separators drawn inside it reach it. The toolbar's own rule
    // runs the whole window, so only the right half's lines count. A separator's stroke is drawn
    // to the edge and so ends half a point past the rows, which the allowance below covers.
    let edge = segments(&shapes, Tokens::get(&ctx).border).iter().filter(|s| s.0.x > 720.0).flat_map(|s| [s.0.x, s.1.x]).fold(f32::MIN, f32::max);
    let text = |s: &str| -> Pos2 {
        let (_, p, size) = crate::live_tests::texts(&shapes).into_iter().find(|(t, _, _)| t == s).unwrap_or_else(|| panic!("no text {s:?}"));
        p + size / 2.0
    };
    for (at, box_) in &pluses {
        assert!((box_ - 16.0).abs() < 0.01, "a + is 16 pt: {box_}");
        let right = at.x + box_ / 2.0;
        assert!((edge - right - 8.0).abs() < 1.0, "a + ends 8 pt in from the panel's edge: {right} of {edge}");
    }
    // The rows that carry a `+`: the section's header, the type's caption, and each folder's. A
    // style's own row carries none, which the count above pins down.
    let mut want: Vec<f32> = [tr("Local styles"), StyleSlot::Fill.title()].iter().map(|s| text(s).y).collect();
    want.extend(["Brand", "Blue"].map(|f| text(f).y));
    let mut got: Vec<f32> = pluses.iter().map(|(at, _)| at.y).collect();
    want.sort_by(|a, b| a.total_cmp(b));
    got.sort_by(|a, b| a.total_cmp(b));
    assert_eq!(want.len(), got.len(), "one + per row that takes one");
    for (w, g) in want.iter().zip(&got) {
        // A galley's box sits a point or so off its glyphs, so "on the row" allows a couple of
        // points: the rows are 24 pt apart, so a `+` on the wrong one is far outside that.
        assert!((w - g).abs() < 2.0, "a + sits on its row: {g} vs {w}");
    }
}

/// The `+`s drawn in `shapes`, as the centre and the 16 pt box each one's arms span: a `+` is two
/// segments crossing at their middle, one vertical and one horizontal.
fn pluses(shapes: &[egui::Shape], icon: egui::Color32) -> Vec<(Pos2, f32)> {
    let mut out: Vec<(Pos2, f32)> = Vec::new();
    for a in segments(shapes, icon) {
        for b in segments(shapes, icon) {
            let (v, h) = if a.0.x == a.1.x && b.0.y == b.1.y { (a, b) } else { continue };
            let mid = |s: (Pos2, Pos2)| s.0 + (s.1 - s.0) / 2.0;
            if (mid(v) - mid(h)).length() < 0.01 {
                // The arms span 11 of the 16 grid units, so the box is that much wider than they are.
                let arm = (v.1.y - v.0.y).abs().max((h.1.x - h.0.x).abs());
                out.push((mid(v), arm * 16.0 / 11.0));
            }
        }
    }
    out
}

/// The `+`s ask for a new style: a folder's makes one of the type, in that folder, and the
/// section's asks which type in a menu. Either way the new style opens in the editor.
#[test]
fn local_styles_pluses_make_a_style_and_open_it() {
    let mut s = Session::new();
    for name in ["Base", "Brand/Blue/500"] {
        s.execute("style.create", &json!({"type": "paint", "name": name, "value": "#0D99FF"})).unwrap();
    }
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let icon = Tokens::get(&ctx).icon_secondary;
    // The `+`s top to bottom: the section's, the caption's, the Brand folder's, the Blue folder's
    // (Blue sits inside Brand, as its name says).
    let rows = |app: &mut FigApp, t: &mut f64| -> Vec<Pos2> {
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_shapes(&ctx, app, t);
        }
        let mut at: Vec<Pos2> = pluses(&shapes, icon).into_iter().map(|(c, _)| c).collect();
        at.sort_by(|a, b| a.y.total_cmp(&b.y));
        at
    };
    let names = |app: &FigApp| -> Vec<(String, &'static str)> {
        app.session.doc().unwrap().styles.iter().map(|s| (s.name.clone(), s.kind.type_name())).collect()
    };
    // A `+` makes `want` of type `t` and opens it in the editor.
    let made = |app: &mut FigApp, t: &mut f64, which: usize, want: &str, kind: &str| {
        let at = rows(app, t);
        assert!(at.len() > 3, "the section, the caption and both folders: {at:?}");
        click_at(&ctx, app, t, at[which]);
        let made = names(app);
        assert!(made.iter().any(|(n, k)| n == want && *k == kind), "{want} of {kind}: {made:?}");
        assert_eq!(editing(app).as_deref(), Some(want), "{want} opens in the editor");
    };

    // A folder's `+` makes one of the type, in that folder: no menu to pick from. The deepest
    // folder first, then the one it sits in, so the list grows under both.
    made(&mut app, &mut t, 3, "Brand/Blue/Color 1", "paint");
    made(&mut app, &mut t, 2, "Brand/Color 1", "paint");
    // A type's caption `+` makes one of that type, at the top level.
    made(&mut app, &mut t, 1, "Color 1", "paint");
    // The section's `+` asks which type: its menu lists them all, and one makes that type.
    let at = rows(&mut app, &mut t);
    click_at(&ctx, &mut app, &mut t, at[0]);
    let menu = frame_shapes(&ctx, &mut app, &mut t);
    let item = crate::live_tests::texts(&menu)
        .into_iter()
        .find(|(t, _, _)| t == StyleSlot::Grid.title())
        .map(|(_, p, size)| p + size / 2.0)
        .unwrap_or_else(|| panic!("no {} item in the menu", StyleSlot::Grid.title()));
    click_at(&ctx, &mut app, &mut t, item);
    let made = names(&app);
    assert!(made.iter().any(|(n, k)| n == "Grid 1" && *k == StyleSlot::Grid.create_type()), "{made:?}");
    assert_eq!(editing(&app).as_deref(), Some("Grid 1"), "Grid 1 opens in the editor");
}

/// The last frame's shapes, with the Design panel showing the document's Local styles.
fn frame_shapes(ctx: &egui::Context, app: &mut FigApp, t: &mut f64) -> Vec<egui::Shape> {
    plus_frame(ctx, app, t, Vec::new())
}

/// One frame carrying `events`, and the shapes it painted (popups included).
fn plus_frame(ctx: &egui::Context, app: &mut FigApp, t: &mut f64, events: Vec<egui::Event>) -> Vec<egui::Shape> {
    *t += 1.0 / 60.0;
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(*t), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    out.textures_delta.clear();
    out.shapes.into_iter().flat_map(|c| crate::live_tests::flat(std::slice::from_ref(&c))).collect()
}

/// A left click at `p`: the pointer goes there, presses, and lets go.
fn click_at(ctx: &egui::Context, app: &mut FigApp, t: &mut f64, p: Pos2) {
    for pressed in [true, false] {
        plus_frame(
            ctx,
            app,
            t,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ],
        );
    }
}

/// The style the Local styles editor has open, by name.
fn editing(app: &FigApp) -> Option<String> {
    app.ui.style_picker.as_ref().and_then(|p| p.edit.as_ref()).map(|e| e.name.clone())
}

/// Straight segments in `c`.
fn segments(shapes: &[egui::Shape], c: egui::Color32) -> Vec<(Pos2, Pos2)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, stroke } if stroke.color == c => Some((points[0], points[1])),
            _ => None,
        })
        .collect()
}

/// Commits `typed` in the text style editor's field `field` ("ts_lh" / "ts_ls") with Enter and
/// returns the redefined style, if any.
fn commit_text_field(def: &TextStyle, field: &str, typed: &str) -> Option<TextStyle> {
    let ctx = egui::Context::default();
    ctx.data_mut(|d| d.insert_temp(Id::new((field, "S")).with("editing"), typed.to_string()));
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))), events: vec![enter], ..Default::default() };
    let mut f = Frame { act: None, hover: None, budget: 0 };
    let doc = Document::new("d");
    let style = Style {
        id: "S".into(),
        name: "S".into(),
        description: String::new(),
        kind: StyleKind::Text { style: def.clone() },
        bound: Default::default(),
    };
    let mut out = ctx.run_ui(input, |ui| text_properties(ui, &doc, &style, def, &mut f));
    out.textures_delta.clear();
    match f.act {
        Some(Act::SetText(id, d)) if id == "S" => Some(d),
        _ => None,
    }
}

#[test]
fn text_style_editor_commits_line_height_and_letter_spacing() {
    use figcraft_doc::{LetterSpacing, LineHeight};
    let def = TextStyle { line_height: LineHeight::Auto, letter_spacing: LetterSpacing::Percent(0.0), ..Default::default() };
    let lh = |def: &TextStyle, s: &str| commit_text_field(def, "ts_lh", s).map(|d| d.line_height);
    let ls = |def: &TextStyle, s: &str| commit_text_field(def, "ts_ls", s).map(|d| d.letter_spacing);
    assert_eq!(lh(&def, "150%"), Some(LineHeight::Percent(150.0)));
    assert_eq!(lh(&def, "24px"), Some(LineHeight::Pixels(24.0)));
    assert_eq!(lh(&def, "24"), Some(LineHeight::Pixels(24.0)));
    let pct = TextStyle { line_height: LineHeight::Percent(120.0), ..def.clone() };
    assert_eq!(lh(&pct, "130"), Some(LineHeight::Percent(130.0)), "a bare number keeps percent");
    assert_eq!(lh(&pct, "Auto"), Some(LineHeight::Auto));
    assert_eq!(ls(&def, "2%"), Some(LetterSpacing::Percent(2.0)));
    assert_eq!(ls(&def, "1.5px"), Some(LetterSpacing::Pixels(1.5)));
    assert_eq!(ls(&def, "3"), Some(LetterSpacing::Percent(3.0)));
    // Only the edited property changes.
    let d = commit_text_field(&def, "ts_lh", "150%").expect("redefined");
    assert_eq!(TextStyle { line_height: def.line_height, ..d }, def);
    assert_eq!(lh(&def, "abc"), None);
}

/// A session with the text style "Body" (Inter Regular 14) and number and string variables to
/// bind to its fields; `bind` binds them all first.
fn text_variables_session(bind: bool) -> crate::layers_drag_tests::Ui {
    let mut s = Session::new();
    s.execute("style.create", &json!({"type": "text", "name": "Body", "value": {"fontFamily": "Inter", "fontStyle": "Regular", "fontSize": 14}}))
        .unwrap();
    s.execute("variable.createCollection", &json!({"name": "Type", "modes": ["Base"]})).unwrap();
    for (name, ty, value) in [
        ("Body size", "number", json!(18)),
        ("Body leading", "number", json!(24)),
        ("Body tracking", "number", json!(1)),
        ("Body font", "string", json!("Inter")),
        ("Body weight", "string", json!("Bold")),
    ] {
        s.execute("variable.create", &json!({"collection": "Type", "name": name, "type": ty, "values": {"Base": value}})).unwrap();
    }
    if bind {
        for (field, var) in [
            ("fontSize", "Body size"),
            ("lineHeight", "Body leading"),
            ("letterSpacing", "Body tracking"),
            ("fontFamily", "Body font"),
            ("fontStyle", "Body weight"),
        ] {
            s.execute("style.bind", &json!({"style": "Body", "field": field, "variable": var})).unwrap();
        }
    }
    let mut u = crate::layers_drag_tests::Ui::new(s);
    open_for_selection(&mut u.app, StyleSlot::Text, Some("Body"));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    assert!(u.app.ui.style_picker.as_ref().is_some_and(|p| p.edit.is_some()), "the text style editor is open");
    u
}

/// The left edge of the style editor's content (its "Edit style" title starts 30 px in).
fn editor_left(u: &mut crate::layers_drag_tests::Ui) -> f32 {
    let mut left = None;
    for c in u.frame_output(vec![]).shapes {
        if let egui::Shape::Text(t) = &c.shape
            && t.galley.text() == "Edit style"
        {
            // The shared popover's title sits 16 in from its edge, its content 8 (its padding).
            left = Some(t.pos.x - 8.0);
        }
    }
    left.expect("the editor's title")
}

/// The texts drawn in one frame, with their centers.
fn drawn_texts(u: &mut crate::layers_drag_tests::Ui) -> Vec<(String, Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.size() / 2.0)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in u.frame_output(vec![]).shapes {
        walk(&c.shape, &mut out);
    }
    out
}

fn text_at(texts: &[(String, Pos2)], s: &str) -> Pos2 {
    texts
        .iter()
        .find(|(t, _)| t == s)
        .map(|(_, p)| *p)
        .unwrap_or_else(|| panic!("no text {s:?} in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
}

/// Hover `p` (variable buttons show on hover), then press and release there.
fn click(u: &mut crate::layers_drag_tests::Ui, p: Pos2) {
    u.frame(vec![egui::Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// The name of the variable on `field` of the style "Body".
fn bound_to(u: &crate::layers_drag_tests::Ui, field: &str) -> Option<String> {
    let doc = u.app.session.doc().unwrap();
    doc.style("Body").unwrap().bound.get(field).map(|v| doc.variables.get(v).map_or_else(|| v.clone(), |x| x.name.clone()))
}

#[test]
fn text_style_editor_shows_bound_fields_as_variables_and_detaches_them() {
    let mut u = text_variables_session(true);
    let left = editor_left(&mut u);
    let texts = drawn_texts(&mut u);
    let half = (W - 8.0) / 2.0;
    // (field, variable shown in its place, right edge of the field: full width or half)
    let fields = [
        ("fontFamily", "Body font", W),
        ("fontStyle", "Body weight", half),
        ("fontSize", "Body size", W),
        ("lineHeight", "Body leading", half),
        ("letterSpacing", "Body tracking", W),
    ];
    let at: Vec<f32> = fields.iter().map(|(_, name, _)| text_at(&texts, name).y).collect();
    for ((field, name, right), y) in fields.iter().zip(at) {
        assert_eq!(bound_to(&u, field).as_deref(), Some(*name));
        // The chip's detach button sits at its right end.
        click(&mut u, Pos2::new(left + right - 11.0, y));
        assert_eq!(bound_to(&u, field), None, "{field} is detached");
    }
    assert!(u.app.ui.style_picker.is_some(), "the editor stays open");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn text_style_editor_binds_variables_from_its_fields() {
    let mut u = text_variables_session(false);
    let left = editor_left(&mut u);
    let texts = drawn_texts(&mut u);
    let half = (W - 8.0) / 2.0;
    let size_row = text_at(&texts, "Size").y;
    let spacing = {
        let doc = u.app.session.doc().unwrap();
        let Some(StyleKind::Text { style: def }) = doc.style("Body").map(|s| &s.kind) else { panic!("a text style") };
        def.letter_spacing.label()
    };
    let in_editor: Vec<_> = texts.iter().filter(|(_, p)| p.x > left && p.x < left + W).cloned().collect();
    let spacing_row = text_at(&in_editor, &spacing).y;
    // Size, line height and letter spacing: the variable button at the field's right end.
    for (field, name, x, y) in [
        ("fontSize", "Body size", left + W - 11.0, size_row),
        ("lineHeight", "Body leading", left + half - 11.0, spacing_row),
        ("letterSpacing", "Body tracking", left + W - 11.0, spacing_row),
    ] {
        click(&mut u, Pos2::new(x, y));
        assert!(crate::variable_ui::picker_open(&u.ctx), "{field}: the variable picker opens");
        let texts = drawn_texts(&mut u);
        assert!(!texts.iter().any(|(t, _)| t == "Body font"), "{field} offers number variables only");
        click(&mut u, text_at(&texts, name));
        assert_eq!(bound_to(&u, field).as_deref(), Some(name));
    }
    // Font and font style: Apply variable at the top of their menus.
    for (field, shown, name) in [("fontFamily", "Inter", "Body font"), ("fontStyle", "Regular", "Body weight")] {
        // The menu's label, not the same name in the Local styles list behind the editor.
        let texts: Vec<_> = drawn_texts(&mut u).into_iter().filter(|(_, p)| p.x > left && p.x < left + W).collect();
        click(&mut u, text_at(&texts, shown));
        let texts = drawn_texts(&mut u);
        click(&mut u, text_at(&texts, "Apply variable"));
        assert!(crate::variable_ui::picker_open(&u.ctx), "{field}: the variable picker opens");
        let texts = drawn_texts(&mut u);
        // "Body size" shows once, on the size field's chip: the picker doesn't list it.
        assert_eq!(texts.iter().filter(|(t, _)| t == "Body size").count(), 1, "{field} offers string variables only");
        click(&mut u, text_at(&texts, name));
        assert_eq!(bound_to(&u, field).as_deref(), Some(name));
    }
    let doc = u.app.session.doc().unwrap();
    let Some(StyleKind::Text { style: def }) = doc.style("Body").map(|s| &s.kind) else { panic!("a text style") };
    assert_eq!((def.font_size, def.font_style.as_str()), (18.0, "Bold"), "the style takes the variables' values");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}
