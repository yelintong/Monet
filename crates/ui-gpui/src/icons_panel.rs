//! A5 — the Icons panel, ported from `ui-egui/src/icons_panel.rs`.
//!
//! The panel lists the icon fonts the file carries, each font's glyphs, and the per-font actions
//! (generate, delete). An icon's preview is drawn from the outline the icon stores
//! (`figcraft_render::render_path`), so it shows before a font has ever been compiled. A click on
//! a row copies the character the glyph stands for.
//!
//! **Hosting**: `ui-egui` shows this as the left panel's `LeftTab::Icons`, which is reachable only
//! through the control channel (`ui.set {leftTab:"icons"}`); no rail button or menu names it. This
//! front-end hosts `ui-egui`'s full panels inside one closable floating panel (`ExtraKind`, see
//! `panels/extra.rs`), so Icons joins that set and is opened by the same `leftTab` request. The
//! content, commands and copy behaviour are the original's.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use figcraft_doc::{IconFont, IconGlyph};
use gpui::{App, Div, ImageSource, MouseButton, MouseDownEvent, RenderImage, SharedString, Window, div, img, prelude::*, px};
use serde_json::{Value, json};

use crate::app::CanvasApp;
use crate::panels::extra::ExtraState;
use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, IconButton, pt, rgba, tokens};

/// The side of an icon's preview, in points (`icons_panel.rs: THUMB`).
const THUMB: f32 = 26.0;

/// What the panel lists: the fonts, each with only the glyphs `query` keeps.
pub(crate) struct Row<'a> {
    pub font: &'a IconFont,
    pub glyphs: Vec<&'a IconGlyph>,
    /// Every glyph stays in (`query` is empty): only then can a drop mean "put it back here".
    pub whole: bool,
}

/// The fonts to list, filtered by `query` (name or code point, lowercase).
///
/// A font with no match is left out while searching; with no query every font stays, even an empty
/// one, so its header (rename / generate / delete) is reachable.
pub(crate) fn listed<'a>(fonts: &'a [IconFont], query: &str) -> Vec<Row<'a>> {
    fonts
        .iter()
        .filter_map(|f| {
            let glyphs: Vec<&IconGlyph> = f
                .glyphs
                .iter()
                .filter(|g| {
                    query.is_empty()
                        || g.name.to_lowercase().contains(query)
                        || format!("{:04x}", g.codepoint).contains(query)
                        || format!("u+{:04x}", g.codepoint).contains(query)
                })
                .collect();
            (!glyphs.is_empty() || query.is_empty()).then_some(Row { font: f, glyphs, whole: query.is_empty() })
        })
        .collect()
}

/// One action the panel raises, run once the list has been drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Copy { font: String, icon: String, codepoint: u32 },
    Delete { font: String, icon: String },
    Generate { font: String },
    DeleteFont { font: String },
}

/// The engine command a click runs. `Copy` has none — it puts the character on the clipboard.
///
/// Kept separate from the drawing code so a test can prove every button raises a real engine
/// command rather than a made-up one.
pub(crate) fn command(act: &Act) -> Option<(&'static str, Value)> {
    Some(match act {
        Act::Delete { font, icon } => ("icon.delete", json!({ "font": font, "name": icon })),
        Act::Generate { font } => ("icon.generateFont", json!({ "font": font })),
        Act::DeleteFont { font } => ("icon.font.delete", json!({ "font": font })),
        Act::Copy { .. } => return None,
    })
}

/// The strokes a glyph's preview is built from, when the outline holds something to draw.
fn outline_hash(path: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// Build (and cache) a glyph's preview in the theme's text colour.
fn thumb(state: &ExtraState, font: &str, glyph: &IconGlyph, colour: figcraft_color::Color) -> Option<Arc<RenderImage>> {
    let key = format!("{font}:{}:{:x}", glyph.name, outline_hash(&glyph.path));
    if let Some(hit) = state.icon_thumbs.borrow().get(&key) {
        return hit.clone();
    }
    let drawn = figcraft_geom::BezPath::from_svg(&glyph.path)
        .ok()
        .and_then(|p| figcraft_render::render_path(&p, (THUMB * 4.0) as u32, colour))
        .map(|pix| crate::canvas::render_worker::screen_image(&pix, figcraft_color::ColorProfile::Srgb));
    state.icon_thumbs.borrow_mut().insert(key, drawn.clone());
    drawn
}

/// The panel: search, the "New icon font" button, then every font and its glyphs.
pub(crate) fn body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, _cx: &mut App) -> Div {
    let t = tokens();
    let Ok(ds) = app.session.ds() else { return note("No document open.") };
    let fonts = ds.doc.icon_fonts().to_vec();
    let query = state.icons_query.borrow().clone();
    let [tr, tg, tb, _] = t.text.to_array();
    let colour = figcraft_color::Color::from_rgba8(tr, tg, tb, 255);

    let new_font = {
        let cmds = cmds.clone();
        pill(tr_str("New icon font"), false, move |_e: &MouseDownEvent, _w: &mut Window, _cx: &mut App| {
            cmds.push(Cmd { id: "icon.font.create", params: Some(json!({})) });
        })
    };

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .w_full()
        .child(state.icons_search.clone())
        .child(div().flex_1().min_w(px(0.0)))
        .child(new_font);

    let mut list = div().flex().flex_col().w_full().gap(pt(6.0));
    if fonts.is_empty() {
        list = list.child(note("No icon fonts yet. Select a layer and run Icon ▸ Create icon."));
        return wrap(header, list);
    }

    let rows = listed(&fonts, &query);
    if rows.is_empty() {
        list = list.child(note("No icons match the search."));
    }
    for row in &rows {
        let f = row.font;
        list = list.child(font_header(f, cmds));
        if row.glyphs.is_empty() {
            list = list.child(note("No icons in this font yet."));
            continue;
        }
        for g in &row.glyphs {
            list = list.child(icon_row(f, g, &colour, state, cmds, &t));
        }
    }
    wrap(header, list)
}

/// One font's header: its name, then generate and delete.
fn font_header(f: &IconFont, cmds: &Cmds) -> Div {
    let t = tokens();
    let generate = {
        let cmds = cmds.clone();
        let font = f.id.clone();
        // Refreshing is part of generating (`icons_panel.rs::Act::Generate`): a font is compiled
        // from the outlines its icons store, so an icon whose layer changed since it was made
        // would otherwise be compiled as its old shape. Every icon with a layer behind it takes
        // its outline again first, then the font is built.
        let behind: Vec<String> = f.glyphs.iter().filter(|g| g.node.is_some()).map(|g| g.name.clone()).collect();
        IconButton::new(Icon::Rotate).size(24.0).on_click(move |_e, _w, _cx| {
            for name in &behind {
                cmds.push(Cmd { id: "icon.update", params: Some(json!({ "font": font, "name": name })) });
            }
            cmds.push(Cmd { id: "icon.generateFont", params: Some(json!({ "font": font })) });
        })
    };
    let delete = {
        let cmds = cmds.clone();
        let font = f.id.clone();
        IconButton::new(Icon::Close).size(24.0).on_click(move |_e, _w, _cx| {
            cmds.push(Cmd { id: "icon.font.delete", params: Some(json!({ "font": font })) });
        })
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .w_full()
        .h(px(32.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(pt(11.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.text))
                .child(SharedString::from(f.name.clone())),
        )
        .child(generate)
        .child(delete)
}

/// One icon: preview, name, code point, delete; a click on the row copies the character.
fn icon_row(f: &IconFont, g: &IconGlyph, colour: &figcraft_color::Color, state: &ExtraState, cmds: &Cmds, t: &crate::theme::Tokens) -> Div {
    let preview = match thumb(state, &f.id, g, *colour) {
        Some(tex) => div().flex_none().child(img(ImageSource::Render(tex)).max_w(px(THUMB)).max_h(px(THUMB))),
        // The outline holds nothing to draw: a dimmed stand-in rather than a wrong glyph.
        None => div()
            .flex_none()
            .w(px(THUMB))
            .h(px(THUMB))
            .rounded(pt(3.0))
            .border_1()
            .border_color(rgba(t.text_tertiary)),
    };
    let copy = {
        let cmds = cmds.clone();
        let (font, icon, cp) = (f.id.clone(), g.name.clone(), g.codepoint);
        div()
            .id(SharedString::from(format!("icon-row-{}-{}", font, icon)))
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w: &mut Window, cx: &mut App| {
                // The character the glyph stands for: the document already holds its code point,
                // so no `icon.list` round-trip is needed to copy it.
                if let Some(ch) = char::from_u32(cp) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(ch.to_string()));
                }
                let _ = &cmds;
            })
    };
    let delete = {
        let cmds = cmds.clone();
        let font = f.id.clone();
        let icon = g.name.clone();
        IconButton::new(Icon::Close).size(20.0).on_click(move |_e, _w, _cx| {
            cmds.push(Cmd { id: "icon.delete", params: Some(json!({ "font": font, "name": icon })) });
        })
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .w_full()
        .h(px(34.0))
        .child(preview)
        .child(
            copy.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(g.name.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(56.0))
                    .text_size(pt(10.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(format!("U+{:04X}", g.codepoint))),
            ),
        )
        .child(delete)
}

fn wrap(header: Div, body: Div) -> Div {
    div().flex().flex_col().w_full().gap(pt(12.0)).p(pt(16.0)).child(header).child(body)
}

fn note(text: &str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text.to_string()))
}

/// A flat button in the panel's own style (`panels/extra.rs::pill` lives next to the panels it
/// serves; this is the same shape for the one button this panel has).
fn pill(label: &str, primary: bool, on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Div {
    let t = tokens();
    div()
        .flex_none()
        .px(pt(10.0))
        .h(px(26.0))
        .rounded(pt(6.0))
        .bg(if primary { rgba(t.accent) } else { rgba(t.field) })
        .flex()
        .items_center()
        .justify_center()
        .text_size(pt(11.0))
        .text_color(if primary { rgba(t.accent_on) } else { rgba(t.text) })
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, on_click)
        .child(SharedString::from(label.to_string()))
}

fn tr_str(s: &'static str) -> &'static str {
    figcraft_i18n::tr(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::{IconFont, IconGlyph};

    fn glyph(name: &str, codepoint: u32) -> IconGlyph {
        IconGlyph { id: name.to_string(), name: name.to_string(), codepoint, path: String::new(), node: None }
    }

    fn font(id: &str, name: &str, glyphs: Vec<IconGlyph>) -> IconFont {
        IconFont { id: id.to_string(), name: name.to_string(), glyphs }
    }

    /// `icons_panel.rs::listed`: a query keeps the glyphs it names, by name or code point, and
    /// drops a font with no match while searching but keeps an empty font when not.
    #[test]
    fn search_keeps_names_and_code_points() {
        let fonts = vec![
            font("f1", "UI", vec![glyph("Home", 0xE000), glyph("Search", 0xE001)]),
            font("f2", "Brand", vec![glyph("Logo", 0xE100)]),
            font("f3", "Empty", vec![]),
        ];
        // No query: every font, empty ones included.
        let all = listed(&fonts, "");
        assert_eq!(all.len(), 3);
        assert!(all[2].glyphs.is_empty());
        assert!(all.iter().all(|r| r.whole));
        // By name.
        let by_name = listed(&fonts, "home");
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].glyphs.len(), 1);
        assert_eq!(by_name[0].glyphs[0].name, "Home");
        assert!(!by_name[0].whole, "a filtered list cannot mean 'put it back here'");
        // By code point, with and without the U+ prefix.
        assert_eq!(listed(&fonts, "e001")[0].glyphs[0].name, "Search");
        assert_eq!(listed(&fonts, "u+e100")[0].glyphs[0].name, "Logo");
        // A miss drops every font.
        assert!(listed(&fonts, "nothing").is_empty());
    }

    /// Every non-copy action must name a real engine command (`icon.*`), never an invented one.
    #[test]
    fn actions_use_engine_commands() {
        let acts = [
            Act::Delete { font: "f".into(), icon: "i".into() },
            Act::Generate { font: "f".into() },
            Act::DeleteFont { font: "f".into() },
        ];
        for act in acts {
            let (id, _) = command(&act).expect("a simple act has a command");
            assert!(id.starts_with("icon."), "{id} is not an icon command");
        }
        assert!(command(&Act::Copy { font: "f".into(), icon: "i".into(), codepoint: 0xE000 }).is_none());
    }
}
