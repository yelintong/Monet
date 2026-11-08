//! The canvas right-click menu — a port of `ui-egui/src/menus.rs`'s context menu.
//!
//! Right-clicking the canvas first picks the layer under the pointer (and selects it, exactly like
//! a click would: `open_context_menu` in `ui-egui/src/canvas/input.rs`), then opens a card at the
//! pointer. Its rows are the selection's commands — or, on empty canvas, the page's.
//!
//! The row set, their order, the dividers and the Copy-as submenu are the original's. A row whose
//! command this front-end cannot run is **greyed** rather than removed, which is the same signal
//! egui gives for an unavailable command (`available(id)`); the UI-layer commands that live in
//! egui's own clipboard (Copy as CSS / SVG / PNG, Copy link) are the ones that come out grey here.

use crate::app::CanvasApp;
use crate::menu_card::Item;
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Point;
use figcraft_i18n::tr;

/// A context menu row's height (egui allocates 28 pt per row).
pub(crate) const ROW_H: f32 = 28.0;
/// The card's width: wide enough for "Wrap in new section" plus a shortcut (egui's `MENU_W`).
pub(crate) const WIDTH: f32 = 240.0;

/// The marker row that opens the Copy-as submenu (`COPY_AS` in the original).
const COPY_AS: &str = "Copy as";

/// Select what a right-click at `p` (page coordinates) lands on, and return the selection the menu
/// acts on. Mirrors `open_context_menu`: the hit layer is resolved to a select target (a group's
/// child, an instance's main component's parts, …) and becomes the selection when it is not
/// already part of it.
pub(crate) fn target(app: &mut CanvasApp, p: Point) -> Vec<NodeId> {
    if let Ok(ds) = app.session.ds() {
        let (doc, page, sel, zoom) = (ds.doc.clone(), ds.page, ds.selection.clone(), ds.view.zoom.max(1e-6));
        if let Some(leaf) = app.session.hit(&doc, page, p, 2.0 / zoom) {
            let target = figcraft_doc::geometry::select_target(&doc, leaf, &sel, false);
            if !sel.contains(&target) {
                app.session.set_selection(vec![target]);
            }
        }
    }
    app.session.selection()
}

/// Whether a row can run: the engine owns the command. (The original also counts the commands its
/// own UI implements — the clipboard and file dialogs — which this front-end does not have yet.)
fn can_run(id: &str) -> bool {
    !id.is_empty() && figcraft_engine::find_command(id).is_some()
}

fn shortcut_of(id: &str) -> Option<String> {
    let spec = figcraft_engine::catalog::shortcut(id)?;
    let text = figcraft_engine::keymap::display(spec, figcraft_engine::keymap::Platform::current());
    (!text.is_empty()).then_some(text)
}

/// A row with its shortcut on the right; greyed when the command is not there.
fn cmd(id: &'static str, label: &'static str) -> Item {
    let mut r = Item::new(id, label).disabled(!can_run(id));
    if let Some(sc) = shortcut_of(id) {
        r = r.trail(sc);
    }
    r
}

/// The Copy-as submenu: the four clipboard formats the original offers.
fn copy_as() -> Item {
    let sub = [
        ("edit.copyAsCss", tr("Copy as CSS")),
        ("edit.copyAsSvg", tr("Copy as SVG")),
        ("edit.copyAsPng", tr("Copy as PNG")),
        ("edit.copyLink", tr("Copy link to selection")),
    ]
    .into_iter()
    .map(|(id, label)| cmd(id, label))
    .collect();
    cmd(COPY_AS, tr("Copy as")).sub(sub)
}

/// Start a new group: a divider above its first row.
fn group(rows: &mut Vec<Item>, items: Vec<Item>) {
    // A divider belongs **between** groups: the very first group has nothing above it to divide
    // from, so it must not carry one (it would draw a stray line at the card's top padding).
    let first_group = rows.is_empty();
    for (i, mut r) in items.into_iter().enumerate() {
        if i == 0 && !first_group {
            r = r.separator_before();
        }
        rows.push(r);
    }
}

/// The menu's rows for `ids` (the selection the menu was opened on).
pub(crate) fn items(doc: Option<&Document>, ids: &[NodeId]) -> Vec<Item> {
    let mut rows: Vec<Item> = Vec::new();
    if ids.is_empty() {
        // Empty canvas: the page's own commands, no dividers before the first one.
        let mut first = true;
        for (id, label) in [
            ("edit.paste", tr("Paste here")),
            ("select.all", tr("Select all")),
            ("view.toggleUi", tr("Show/Hide UI")),
            ("view.zoomFit", tr("Zoom to fit")),
            ("view.rulers", tr("Rulers")),
            ("view.pixelGrid", tr("Pixel grid")),
        ] {
            let mut r = cmd(id, label);
            if !first {
                r = r.separator_before();
            }
            first = false;
            rows.push(r);
        }
        return rows;
    }

    group(
        &mut rows,
        vec![cmd("edit.copy", tr("Copy")), cmd("edit.paste", tr("Paste here")), cmd("edit.pasteReplace", tr("Paste to replace")), copy_as()],
    );
    group(
        &mut rows,
        vec![
            cmd("arrange.front", tr("Bring to front")),
            cmd("arrange.forward", tr("Bring forward")),
            cmd("arrange.backward", tr("Send backward")),
            cmd("arrange.back", tr("Send to back")),
        ],
    );
    let mut structure = vec![
        cmd("object.group", tr("Group selection")),
        cmd("object.frameSelection", tr("Frame selection")),
        cmd("object.wrapInSection", tr("Wrap in new section")),
    ];
    // A frame/section in the selection converts; the row sits right under "Wrap in new section".
    if let Some(convert) = convert_row(doc, ids) {
        structure.push(convert);
    }
    structure.push(cmd("object.ungroup", tr("Ungroup")));
    structure.push(cmd("vector.flatten", tr("Flatten")));
    structure.push(cmd("vector.outlineStroke", tr("Outline stroke")));
    structure.push(cmd("object.mask", tr("Use as mask")));
    group(&mut rows, structure);
    let mut components = vec![cmd("layout.add", tr("Add auto layout"))];
    components.push(component_row(doc, ids).unwrap_or_else(|| cmd("component.create", tr("Create component"))));
    if doc.is_some_and(|d| figcraft_engine::cmd::component::can_combine_variants(d, ids)) {
        components.push(cmd("component.combineVariants", tr("Combine as variants")));
    }
    components.push(cmd("component.detach", tr("Detach instance")));
    group(&mut rows, components);
    group(&mut rows, vec![cmd("icon.create", tr("Create icon")), cmd("icon.detach", tr("Detach icon"))]);
    let mut state = vec![cmd("object.toggleVisible", tr("Show/Hide")), cmd("object.toggleLock", tr("Lock/Unlock"))];
    if let Some(contents) = section_row(doc, ids) {
        state.push(contents);
    }
    group(&mut rows, state);
    group(&mut rows, vec![cmd("object.flipH", tr("Flip horizontal")), cmd("object.flipV", tr("Flip vertical"))]);
    group(&mut rows, vec![cmd("edit.delete", tr("Delete"))]);
    rows
}

/// Frames on the page (or in a section) convert to sections, and sections to frames.
fn convert_row(doc: Option<&Document>, ids: &[NodeId]) -> Option<Item> {
    let doc = doc?;
    let nodes: Vec<&figcraft_doc::Node> = ids.iter().filter_map(|i| doc.node(*i)).collect();
    if nodes.is_empty() {
        return None;
    }
    let on_board =
        |n: &figcraft_doc::Node| n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || matches!(p.kind, NodeKind::Section { .. }));
    if nodes.iter().all(|n| matches!(n.kind, NodeKind::Section { .. })) {
        Some(cmd("object.convertToFrame", tr("Convert to frame")))
    } else if nodes.iter().all(|n| matches!(n.kind, NodeKind::Frame) && on_board(n)) {
        Some(cmd("object.convertToSection", tr("Convert to section")))
    } else {
        None
    }
}

/// "Create component" becomes "Go to main component" when every selected layer is an instance.
fn component_row(doc: Option<&Document>, ids: &[NodeId]) -> Option<Item> {
    let doc = doc?;
    ids.iter()
        .all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. })))
        .then(|| cmd("component.goToMain", tr("Go to main component")))
}

/// Sections in the selection add a contents toggle naming what it will do.
fn section_row(doc: Option<&Document>, ids: &[NodeId]) -> Option<Item> {
    let doc = doc?;
    let states: Vec<bool> =
        ids.iter().filter_map(|i| doc.node(*i)).filter(|n| matches!(n.kind, NodeKind::Section { .. })).map(|n| n.hides_contents()).collect();
    if states.is_empty() {
        return None;
    }
    let label = if states.iter().all(|h| *h) { tr("Show section contents") } else { tr("Hide section contents") };
    Some(cmd("section.toggleContents", label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(rows: &[Item]) -> Vec<&'static str> {
        rows.iter().filter(|r| !r.id.is_empty()).map(|r| r.id).collect()
    }

    /// Empty canvas: the page's own commands, in the original's order, with no divider above the
    /// first row.
    #[test]
    fn empty_canvas_offers_the_page_commands() {
        let rows = items(None, &[]);
        assert_eq!(ids(&rows), ["edit.paste", "select.all", "view.toggleUi", "view.zoomFit", "view.rulers", "view.pixelGrid"]);
        assert!(!rows[0].separator_before);
        assert!(rows[1..].iter().all(|r| r.separator_before));
    }

    /// A selection: copy/paste, a Copy-as submenu, the arrange/group/component blocks and Delete —
    /// with the dividers the original puts between them.
    #[test]
    fn a_selection_offers_the_layer_commands() {
        let rows = items(None, &[NodeId(1)]);
        let list = ids(&rows);
        assert_eq!(list.first(), Some(&"edit.copy"));
        assert_eq!(list.last(), Some(&"edit.delete"));
        assert!(!rows[0].separator_before, "no divider above the first row");
        assert!(list.contains(&COPY_AS));
        let copy_as_row = rows.iter().find(|r| r.id == COPY_AS).expect("copy as");
        assert_eq!(copy_as_row.sub.len(), 4);
        assert!(copy_as_row.sub.iter().all(|r| r.disabled), "the clipboard formats are UI-side");
        assert_eq!(rows.iter().filter(|r| r.separator_before).count(), 7, "eight groups, seven dividers between them");
    }

    /// A row is greyed when the engine doesn't own its command.
    #[test]
    fn an_unknown_command_is_greyed() {
        assert!(!can_run("edit.copyAsCss"));
        assert!(can_run("edit.copy"));
        assert!(can_run("view.rulers"));
    }
}
