//! The style browser: the popover a Fill, Stroke, Effects, Layout grid or Typography section's
//! Styles button opens, laid out like the variable picker. Search; this file's styles, then each
//! enabled library's published ones, grouped by the folders of their `/`-separated names (folders
//! collapse); a list or, for colour and text styles, a grid. Text styles show in their own font,
//! and while the pointer is over one the canvas shows it on the selected text (or the selected
//! chars of the text being edited). Click applies a style (a library's is copied in, linked); a
//! local style's edit button (or a right-click) edits its name, description and values; `+`
//! creates a style from the selection (for text, the selected chars); the detach button unlinks
//! it. Colour slots also list colour variables to bind (this file's, then each enabled library's).

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, GridKind, Node, NodeId, PublishedLibrary, Style, StyleKind, TextStyle, VariableType, trim_num};
use figcraft_engine::library;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::colorpicker::PickerTarget;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::popover::{Hold, Open, Popover, Surface};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, TEXT_PT};

/// The slot's margin inside the shared popover, and the content width left of it.
const PAD: f32 = 8.0;
const W: f32 = 224.0;
/// Height of a list row.
const ROW_H: f32 = 26.0;
/// Height of a text style's list row (its name in its own font).
const TEXT_ROW_H: f32 = 32.0;
/// Side of a swatch in the grid view.
const SWATCH: f32 = 24.0;
/// A text style's tile in the grid view.
const TILE: egui::Vec2 = egui::vec2(54.0, 44.0);
/// Largest size a text style's name previews at in the list (the grid's "Ag" goes to 24).
const LIST_PREVIEW_MAX: f64 = 16.0;

// The Design panel's Local styles list: rows sit inset from the panel's edges with the hover band
// rounded, a folder folds from its chevron, and each type's group opens with a caption row. The
// icon of a row (a colour swatch, a text style's "Ag", a folder's chevron) takes a 16 pt box and
// the name follows 8 pt after it; a nesting level steps in by that same 24 pt, so a folder's
// styles line their icons up under its name.
/// Side inset of a row: the hover band starts and ends here, not at the panel's edges.
const LS_INSET: f32 = 8.0;
/// Where a row's content (and a caption's text) starts from the panel's edge.
const LS_PAD: f32 = 16.0;
/// Height of a group's caption row ("Text styles").
const LS_CAPTION_H: f32 = 32.0;
/// Height of a folder or style row.
const LS_ROW_H: f32 = 24.0;
/// A row's icon box, and the gap between it and the name.
const LS_ICON: f32 = 16.0;
const LS_ICON_GAP: f32 = 8.0;
/// How far a nesting level steps in: an icon box and its gap, so a child's icon sits under the
/// folder's name.
const LS_STEP: f32 = LS_ICON + LS_ICON_GAP;
/// A folder's chevron, inside its 16 pt box.
const LS_CHEVRON: f32 = 12.0;
/// Gap above the first group's caption, and above every later one.
const LS_FIRST_GAP: f32 = 8.0;
const LS_GROUP_GAP: f32 = 16.0;
/// Gap under the whole list.
const LS_BOTTOM_PAD: f32 = 16.0;
/// A row's trailing text (a text style's size and line height) ends this far from the band's edge.
const LS_TRAIL: f32 = 8.0;

/// A style slot on a layer: what a style picker lists and what a style chip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleSlot {
    Fill,
    Stroke,
    Effect,
    Grid,
    Text,
}

impl StyleSlot {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fill" => Some(StyleSlot::Fill),
            "stroke" => Some(StyleSlot::Stroke),
            "effect" => Some(StyleSlot::Effect),
            "grid" => Some(StyleSlot::Grid),
            "text" => Some(StyleSlot::Text),
            _ => None,
        }
    }

    /// The slot's name in `style.apply {target}`, `style.detach {which}` and `variable.bind {field}`.
    pub fn key(self) -> &'static str {
        match self {
            StyleSlot::Fill => "fill",
            StyleSlot::Stroke => "stroke",
            StyleSlot::Effect => "effect",
            StyleSlot::Grid => "grid",
            StyleSlot::Text => "text",
        }
    }

    fn linked(self, n: &Node) -> Option<String> {
        match self {
            StyleSlot::Fill => n.styles.fill.clone(),
            StyleSlot::Stroke => n.styles.stroke.clone(),
            StyleSlot::Effect => n.styles.effect.clone(),
            StyleSlot::Grid => n.styles.grid.clone(),
            StyleSlot::Text => n.styles.text.clone(),
        }
    }

    fn paints(self) -> bool {
        matches!(self, StyleSlot::Fill | StyleSlot::Stroke)
    }

    /// Whether the slot's styles also show as a grid (swatches, "Ag" tiles).
    fn has_grid(self) -> bool {
        self.paints() || self == StyleSlot::Text
    }

    fn title(self) -> &'static str {
        match self {
            StyleSlot::Fill | StyleSlot::Stroke => tr("Color styles"),
            StyleSlot::Effect => tr("Effect styles"),
            StyleSlot::Grid => tr("Grid styles"),
            StyleSlot::Text => tr("Text styles"),
        }
    }

    /// `style.create {type}` for this slot (taken from the layer's fills, strokes, effects, grids
    /// or text).
    fn create_type(self) -> &'static str {
        match self {
            StyleSlot::Fill => "paint",
            StyleSlot::Stroke => "stroke",
            StyleSlot::Effect => "effect",
            StyleSlot::Grid => "grid",
            StyleSlot::Text => "text",
        }
    }

    /// The base of a new style's name ("Color 3").
    fn default_name(self) -> &'static str {
        match self {
            StyleSlot::Fill | StyleSlot::Stroke => "Color",
            StyleSlot::Effect => "Effect",
            StyleSlot::Grid => "Grid",
            StyleSlot::Text => "Text",
        }
    }

    fn holds(self, k: &StyleKind) -> bool {
        matches!(
            (k, self),
            (StyleKind::Paint { .. }, StyleSlot::Fill | StyleSlot::Stroke)
                | (StyleKind::Effect { .. }, StyleSlot::Effect)
                | (StyleKind::Grid { .. }, StyleSlot::Grid)
                | (StyleKind::Text { .. }, StyleSlot::Text)
        )
    }

    fn icon(self) -> Icon {
        match self {
            StyleSlot::Grid => Icon::Grid,
            StyleSlot::Text => Icon::Text,
            _ => Icon::Effects,
        }
    }
}

/// The style being edited, with its name and description as typed.
#[derive(Clone, Debug)]
struct Editing {
    id: String,
    name: String,
    description: String,
    /// Focus the name (and select it) on the next frame: a style that was just created.
    focus: bool,
}

/// The open picker.
#[derive(Clone, Debug)]
pub struct Picker {
    slot: StyleSlot,
    ids: Vec<NodeId>,
    /// Screen y of the button that opened it.
    y: f32,
    query: String,
    edit: Option<Editing>,
}

impl Picker {
    pub fn slot(&self) -> StyleSlot {
        self.slot
    }
}

/// Open the picker for `slot` of `ids`, level with the pointer.
pub fn open(app: &mut FigApp, ui: &Ui, ids: &[NodeId], slot: StyleSlot) {
    let y = ui.input(|i| i.pointer.interact_pos()).map_or(140.0, |p| p.y - 12.0);
    // Opening the style picker closes the colour picker (it would linger beside it).
    crate::colorpicker::close(app);
    app.ui.style_picker_popover = None;
    app.ui.style_picker = Some(Picker { slot, ids: ids.to_vec(), y, query: String::new(), edit: None });
}

/// Open the picker for `slot` of the selection (the control channel), optionally editing `style`.
pub fn open_for_selection(app: &mut FigApp, slot: StyleSlot, style: Option<&str>) {
    let ids = app.session.selection();
    let edit = style.and_then(|s| app.session.doc().ok()?.style(s).map(editing));
    crate::colorpicker::close(app);
    app.ui.style_picker_popover = None;
    app.ui.style_picker = Some(Picker { slot, ids, y: 140.0, query: String::new(), edit });
}

fn editing(s: &Style) -> Editing {
    Editing { id: s.id.clone(), name: s.name.clone(), description: s.description.clone(), focus: false }
}

/// The style `slot` of `ids` links to: `Some(Some(id))` one style, `Some(None)` none, None when
/// they differ. Text goes by chars: the selected chars of the text being edited, else every char.
fn link_of(app: &FigApp, doc: &Document, ids: &[NodeId], slot: StyleSlot) -> Option<Option<String>> {
    if slot != StyleSlot::Text {
        return crate::right::common(doc, ids, |n| slot.linked(n));
    }
    let range = figcraft_engine::textedit::selected_range(&app.session);
    let styles = crate::typography::target_styles(doc, ids, range);
    let mut links = styles.iter().map(|s| s.style_id.clone());
    let Some(first) = links.next() else { return Some(None) };
    links.all(|l| l == first).then_some(first)
}

/// The text style the targeted chars of `ids` link to (see [`link_of`]).
pub fn text_link(app: &FigApp, doc: &Document, ids: &[NodeId]) -> Option<Option<String>> {
    link_of(app, doc, ids, StyleSlot::Text)
}

/// The applied style's name (click opens the picker) with a detach button; for text, "Mixed"
/// when the chars link to different styles (or some to none).
pub fn chip(app: &mut FigApp, ui: &mut Ui, doc: &Document, ids: &[NodeId], slot: StyleSlot) {
    let t = Tokens::get(ui.ctx());
    let (name, mixed) = match link_of(app, doc, ids, slot) {
        Some(Some(id)) => (doc.style(&id).map_or_else(|| tr("Missing style").to_string(), |st| st.name.clone()), false),
        None if slot == StyleSlot::Text => (tr("Mixed").to_string(), true),
        _ => return,
    };
    let mut open_it = false;
    crate::right::row(ui, 32.0, |ui| {
        let w = (ui.available_width() - 32.0).max(40.0);
        let (r, resp) = ui.allocate_exact_size(vec2(w, widgets::FIELD_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        let icon = if slot == StyleSlot::Text { Icon::Text } else { Icon::Styles };
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0)), icon, t.icon_secondary);
        let col = if mixed { t.text_secondary } else { t.text };
        crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 26.0, r.center().y), &name, regular(11.0), col, r.width() - 32.0);
        let tip = if mixed { tr("The selection uses several text styles: click to apply one") } else { tr("Change style") };
        open_it = resp.on_hover_text(tip).clicked();
        let detach = if mixed { tr("Detach styles") } else { tr("Detach style") };
        if icons::button(ui, Icon::Detach, 24.0, false, detach).clicked() {
            app.run("style.detach", json!({"ids": ids, "which": slot.key()}));
        }
    });
    if open_it {
        open(app, ui, ids, slot);
    }
}

/// A listed style: by its library's id (None: this file's) and its id there.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    library: Option<String>,
    style: String,
}

impl Key {
    /// What `local` (a style in this document) is: a library style's copy, or this file's own.
    fn of_local(doc: &Document, local: &str) -> Key {
        match doc.libraries.style_import(local) {
            Some(r) => Key { library: Some(r.library.clone()), style: r.key.clone() },
            None => Key { library: None, style: local.to_string() },
        }
    }

    /// The command (and its params) applying this style to `ids`.
    fn apply(&self, ids: &[NodeId], slot: StyleSlot) -> (&'static str, Value) {
        let mut q = match &self.library {
            Some(lib) => json!({"library": lib, "style": self.style, "ids": ids}),
            None => json!({"ids": ids, "style": self.style}),
        };
        if slot.paints() {
            q["target"] = json!(slot.key());
        }
        (if self.library.is_some() { "library.applyStyle" } else { "style.apply" }, q)
    }
}

/// What the user asked the picker to do this frame.
enum Act {
    Apply(Key),
    Bind(crate::variable_ui::Pick),
    Create,
    Detach,
    Edit(String),
    Back,
    EditPaint(String, usize),
    /// Unbind the variable on a field of a style.
    Unbind(String, String),
    /// A text style's new definition (from the editor's fields).
    SetText(String, TextStyle),
    UpdateFromSelection(String),
    Delete(String),
    ToggleFolder(String),
    ToggleView,
}

/// What a listing reports besides acts: the style under the pointer, and the preview budget.
struct Frame {
    act: Option<Act>,
    hover: Option<Key>,
    /// Text previews still allowed to render this frame.
    budget: usize,
}

/// An icon button on the dark popover.
fn menu_icon(ui: &mut Ui, rect: Rect, icon: Icon, tip: &str, on: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.id().with(("style_picker_btn", tip)), Sense::click()).tip(tip);
    if on || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.menu_hover);
    }
    icons::paint(ui.painter(), rect.shrink(5.0), icon, t.menu_text);
    resp
}

/// A text button on the dark popover.
fn menu_button(ui: &mut Ui, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.menu_text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, widgets::FIELD_H), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(5), if resp.hovered() { t.menu_hover } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.menu_dim), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(11.0), t.menu_text);
    resp
}

/// A dim caption on the popover.
fn caption(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).font(regular(10.0)).color(t.menu_dim));
}

/// The folder part of a style name ("Brand/Blue/500" → "Brand/Blue"), empty at the top level.
fn folder(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(f, _)| f.trim())
}

/// The last part of a style name ("Brand/Blue/500" → "500").
fn leaf(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, l)| l.trim())
}

/// `base N` with the smallest N no style of the document is named.
fn next_name(doc: &Document, base: &str) -> String {
    (1..).map(|n| format!("{base} {n}")).find(|n| doc.styles.iter().all(|s| &s.name != n)).unwrap_or_else(|| base.to_string())
}

/// Styles by folder, folders in order of first appearance, top-level styles first.
fn by_folder<'a>(styles: impl Iterator<Item = &'a Style>) -> Vec<(&'a str, Vec<&'a Style>)> {
    let mut groups: Vec<(&str, Vec<&Style>)> = Vec::new();
    for s in styles {
        let f = folder(&s.name);
        match groups.iter_mut().find(|(g, _)| *g == f) {
            Some((_, list)) => list.push(s),
            None => groups.push((f, vec![s])),
        }
    }
    groups.sort_by_key(|(f, _)| !f.is_empty());
    groups
}

/// One line of the Local styles list, in the order it is drawn.
enum Entry<'a> {
    /// A folder's header, `depth` levels in (0 at the top): its whole path and the part shown.
    Folder { path: &'a str, label: &'a str, depth: usize },
    /// A style, `depth` levels in.
    Style(&'a Style, usize),
}

impl Entry<'_> {
    /// How many levels in the line sits (0 at the top).
    fn depth(&self) -> usize {
        match *self {
            Entry::Folder { depth, .. } | Entry::Style(_, depth) => depth,
        }
    }
}

/// A folder of the Local styles tree: its path, the part of it shown, the styles named directly in
/// it and the folders under it (in the order their styles first name them).
struct Branch<'a> {
    path: &'a str,
    label: &'a str,
    styles: Vec<&'a Style>,
    kids: Vec<usize>,
}

/// The styles as a tree of `/`-separated folders, flattened for the list: the styles with no
/// folder first, then each folder's header followed by its own styles and the folders under it.
///
/// Folders come from the names alone, so a name can nest as deep as it likes ("Brand/Blue/500"
/// makes "Brand" and "Brand/Blue"), and a folder that only exists as a prefix is one too.
/// `folded` hides a folder's whole branch.
fn entries<'a>(styles: impl Iterator<Item = &'a Style>, folded: impl Fn(&str) -> bool) -> Vec<Entry<'a>> {
    // The folders in the order their styles first name them, top level first.
    let mut groups: Vec<(&'a str, Vec<&'a Style>)> = Vec::new();
    for s in styles {
        let f = folder(&s.name);
        match groups.iter_mut().find(|(g, _)| *g == f) {
            Some((_, list)) => list.push(s),
            None => groups.push((f, vec![s])),
        }
    }
    groups.sort_by_key(|(f, _)| !f.is_empty());
    // The tree: the root (no path) holds the styles with no folder and the topmost folders.
    let mut nodes: Vec<Branch<'a>> = vec![Branch { path: "", label: "", styles: Vec::new(), kids: Vec::new() }];
    for (path, list) in groups {
        let i = branch_of(&mut nodes, path);
        nodes[i].styles = list;
    }
    let mut out = Vec::new();
    for s in &nodes[0].styles {
        out.push(Entry::Style(s, 0));
    }
    let top = std::mem::take(&mut nodes[0].kids);
    for kid in top {
        flatten(&nodes, kid, 0, &folded, &mut out);
    }
    out
}

/// The branch of `path`, made along with its parents when the tree doesn't have it yet.
fn branch_of<'a>(nodes: &mut Vec<Branch<'a>>, path: &'a str) -> usize {
    if let Some(i) = nodes.iter().position(|n| n.path == path) {
        return i;
    }
    let (parent, label) = path.rsplit_once('/').map_or(("", path), |(p, l)| (p, l));
    let parent = branch_of(nodes, parent);
    nodes.push(Branch { path, label, styles: Vec::new(), kids: Vec::new() });
    let i = nodes.len() - 1;
    nodes[parent].kids.push(i);
    i
}

/// `node`'s header and everything under it, top to bottom (nothing under a folded folder).
fn flatten<'a>(nodes: &[Branch<'a>], node: usize, depth: usize, folded: &impl Fn(&str) -> bool, out: &mut Vec<Entry<'a>>) {
    let n = &nodes[node];
    out.push(Entry::Folder { path: n.path, label: n.label, depth });
    if folded(n.path) {
        return;
    }
    for s in &n.styles {
        out.push(Entry::Style(s, depth + 1));
    }
    for kid in &n.kids {
        flatten(nodes, *kid, depth + 1, folded, out);
    }
}

/// A text style's definition, if `s` is one.
fn text_def(s: &Style) -> Option<&TextStyle> {
    match &s.kind {
        StyleKind::Text { style } => Some(style),
        _ => None,
    }
}

/// "32/Auto": a text style's size and line height.
fn text_meta(def: &TextStyle) -> String {
    format!("{}/{}", trim_num(def.font_size), def.line_height.label())
}

/// A text style's sample (`text` in its own font at most `max` big), painted from `left_center`;
/// falls back to plain UI text until the preview renders (or when the font is missing).
fn text_sample(ui: &Ui, def: &TextStyle, text: &str, max: f64, left_center: egui::Pos2, max_w: f32, max_h: f32, budget: &mut usize) {
    let t = Tokens::get(ui.ctx());
    let shown = TextStyle { font_size: def.font_size.clamp(6.0, max), ..def.clone() };
    match crate::font_preview::texture(ui.ctx(), &shown, text, budget) {
        Some(tex) => {
            crate::font_preview::paint(ui.painter(), &tex, left_center, max_w, max_h, t.menu_text);
        }
        None => {
            crate::bidi::paint_name(ui.painter(), left_center, text, regular(11.0), t.menu_text, max_w);
        }
    }
}

/// Draw a style's preview (its first paint, or the slot's icon) centred in `r`.
fn preview(ui: &Ui, r: Rect, s: &Style, slot: StyleSlot) {
    let t = Tokens::get(ui.ctx());
    match &s.kind {
        StyleKind::Paint { paints } => {
            let p = paints.last().cloned().unwrap_or_else(|| Paint::solid(figcraft_color::Color::TRANSPARENT));
            widgets::paint_swatch(ui.painter(), r, &p);
        }
        _ => icons::paint(ui.painter(), r, slot.icon(), t.menu_text),
    }
}

/// Click, right-click and hover on a listed style become acts.
fn react(resp: &Response, key: Key, editable: bool, out: &mut Frame) {
    if resp.hovered() {
        out.hover = Some(key.clone());
    }
    if resp.secondary_clicked() && editable {
        out.act = Some(Act::Edit(key.style));
    } else if resp.clicked() {
        out.act = Some(Act::Apply(key));
    }
}

/// One style in the list: preview (a text style: its name in its own font and its size), name,
/// a check when applied and an edit button on hover.
fn style_row(ui: &mut Ui, s: &Style, key: Key, slot: StyleSlot, applied: bool, editable: bool, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let h = if slot == StyleSlot::Text { TEXT_ROW_H } else { ROW_H };
    let (r, resp) = ui.allocate_exact_size(vec2(W, h), Sense::click());
    let edit_r = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(22.0, 22.0));
    let hovered = resp.hovered() || ui.rect_contains_pointer(edit_r);
    if hovered {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    match text_def(s) {
        Some(def) => {
            let meta = text_meta(def);
            let meta_w = ui.painter().layout_no_wrap(meta.clone(), regular(10.0), t.menu_dim).size().x;
            ui.painter().text(
                pos2(edit_r.min.x - 4.0, r.center().y),
                Align2::RIGHT_CENTER,
                meta,
                regular(10.0),
                if hovered { t.menu_text } else { t.menu_dim },
            );
            let max_w = edit_r.min.x - 12.0 - meta_w - (r.min.x + 8.0);
            text_sample(ui, def, leaf(&s.name), LIST_PREVIEW_MAX, pos2(r.min.x + 8.0, r.center().y), max_w, h - 4.0, &mut out.budget);
        }
        None => {
            preview(ui, Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0)), s, slot);
            crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 26.0, r.center().y), leaf(&s.name), regular(11.0), t.menu_text, W - 64.0);
        }
    }
    // A style with variables bound inside it is marked (its users follow their own modes).
    if !s.bound.is_empty() {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(edit_r.min.x - 10.0, r.center().y), vec2(11.0, 11.0)), Icon::Variable, t.menu_dim);
    }
    if hovered && editable {
        if menu_icon(ui, edit_r, Icon::Adjust, tr("Edit style"), false).clicked() {
            out.act = Some(Act::Edit(s.id.clone()));
            return;
        }
    } else if applied {
        icons::paint(ui.painter(), edit_r.shrink(5.0), Icon::Check, t.menu_text);
    }
    let tip = match (s.description.is_empty(), text_def(s)) {
        (true, None) if !s.bound.is_empty() => Some(format!("{}\n{}", s.name, tr("Uses variables: each layer shows them in its own mode"))),
        (true, None) => None,
        (true, Some(def)) => Some(format!("{}\n{} {} · {}", s.name, def.font_family, def.font_style, text_meta(def))),
        (false, _) => Some(s.description.clone()),
    };
    let resp = match tip {
        Some(tip) => resp.on_hover_text(tip),
        None => resp,
    };
    react(&resp, key, editable, out);
}

/// Colour styles as swatches and text styles as "Ag" tiles in their own font (names on hover).
fn style_grid(ui: &mut Ui, list: &[(&Style, Key)], applied: Option<&Key>, editable: impl Fn(&Key) -> bool, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let text = list.first().is_some_and(|(s, _)| text_def(s).is_some());
    let cell = if text { TILE } else { vec2(SWATCH, SWATCH) };
    let per_row = ((W - 8.0 + 6.0) / (cell.x + 6.0)).floor().max(1.0) as usize;
    for chunk in list.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            for (s, key) in chunk {
                let (r, resp) = ui.allocate_exact_size(cell, Sense::click());
                match text_def(s) {
                    Some(def) => {
                        let fill = if resp.hovered() { t.menu_hover } else { Color32::from_white_alpha(14) };
                        ui.painter().rect_filled(r, CornerRadius::same(5), fill);
                        let sample_w = ui.painter().layout_no_wrap("Ag".into(), regular(18.0), t.menu_text).size().x;
                        text_sample(
                            ui,
                            def,
                            "Ag",
                            24.0,
                            pos2(r.center().x - sample_w / 2.0, r.center().y),
                            r.width() - 6.0,
                            r.height() - 4.0,
                            &mut out.budget,
                        );
                    }
                    None => preview(ui, r, s, StyleSlot::Fill),
                }
                if applied == Some(key) {
                    ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(5), Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
                } else if resp.hovered() && !text {
                    ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(4), Stroke::new(1.0, t.menu_text), egui::StrokeKind::Inside);
                }
                let tip = match text_def(s) {
                    Some(def) => format!("{}\n{} {} · {}", s.name, def.font_family, def.font_style, text_meta(def)),
                    None => s.name.clone(),
                };
                let resp = resp.on_hover_text(tip);
                react(&resp, key.clone(), editable(key), out);
            }
        });
        ui.add_space(2.0);
    }
}

/// The styles a source offers (this file's, or a library's published ones) that the slot holds
/// and the search matches.
fn offered<'a>(doc: &'a Document, lib: Option<&'a PublishedLibrary>, slot: StyleSlot, q: &str) -> Vec<&'a Style> {
    let matches = |s: &&Style| slot.holds(&s.kind) && (q.is_empty() || s.name.to_lowercase().contains(q));
    match lib {
        Some(l) => l.doc.styles.iter().filter(|s| l.manifest.style(&s.id).is_some_and(|x| x.published)).filter(matches).collect(),
        None => library::local_styles(doc).filter(matches).collect(),
    }
}

/// The style list: header, search, this file's and the libraries' styles by folder, and (colour
/// slots) colour variables.
fn listing(app: &FigApp, ui: &mut Ui, doc: &Document, p: &mut Picker, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let slot = p.slot;
    let link = link_of(app, doc, &p.ids, slot);
    let applied = link.clone().flatten().map(|id| Key::of_local(doc, &id));
    let grid = slot.has_grid() && app.ui.style_grid;
    // The title is the popover's; this row keeps the view toggle, detach and create.
    let (hr, _) = ui.allocate_exact_size(vec2(W, 24.0), Sense::hover());
    let mut x = hr.max.x - 24.0;
    let mut slot_rect = || {
        let r = Rect::from_min_size(pos2(x, hr.min.y), vec2(24.0, 24.0));
        x -= 26.0;
        r
    };
    let create_tip = if slot == StyleSlot::Text { tr("Create style from the selected text") } else { tr("Create style") };
    if menu_icon(ui, slot_rect(), Icon::Plus, create_tip, false).clicked() {
        out.act = Some(Act::Create);
    }
    if link != Some(None) && menu_icon(ui, slot_rect(), Icon::Detach, tr("Detach style"), false).clicked() {
        out.act = Some(Act::Detach);
    }
    if slot.has_grid() {
        let (icon, tip) = if grid { (Icon::ViewList, tr("Show as list")) } else { (Icon::ViewGrid, tr("Show as grid")) };
        if menu_icon(ui, slot_rect(), icon, tip, false).clicked() {
            out.act = Some(Act::ToggleView);
        }
    }
    ui.add_space(4.0);
    let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tr("Search")).font(regular(11.0)).desired_width(W));
    crate::widgets::select_all_on_focus(ui, &r, &p.query);
    if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
        r.request_focus();
    }
    ui.add_space(4.0);
    let q = p.query.trim().to_lowercase();
    let libs = crate::variable_ui::libraries(app, doc);
    let mut sources: Vec<(Option<&PublishedLibrary>, Vec<&Style>)> = vec![(None, offered(doc, None, slot, &q))];
    for l in &libs {
        let list = offered(doc, Some(l), slot, &q);
        if !list.is_empty() {
            sources.push((Some(l.as_ref()), list));
        }
    }
    let colors = if slot.paints() {
        // Variables scoped to every selected layer's fill (or stroke), as the Design panel's pickers offer.
        let scopes: Vec<_> = p.ids.iter().filter_map(|id| figcraft_engine::cmd::variables::field_scope(doc, *id, slot.key())).collect();
        crate::variable_ui::groups(doc, &libs, |_, v| {
            v.kind == VariableType::Color && (q.is_empty() || v.name.to_lowercase().contains(&q)) && scopes.iter().all(|s| v.in_scope(s))
        })
    } else {
        Vec::new()
    };
    egui::ScrollArea::vertical().id_salt("style_picker_scroll").max_height(320.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if sources.iter().all(|(_, l)| l.is_empty()) {
            let any = library::local_styles(doc).any(|s| slot.holds(&s.kind));
            let hint = match (any, slot) {
                (true, _) => tr("No matching styles"),
                (false, StyleSlot::Text) => tr("No text styles yet. Click + to create one from the selected text."),
                (false, _) => tr("No styles yet. Click + to create one from the selection."),
            };
            caption(ui, hint);
        }
        for (lib, list) in &sources {
            if let Some(l) = lib {
                ui.add_space(6.0);
                caption(ui, &l.manifest.name);
            } else if list.is_empty() {
                continue;
            } else if sources.len() > 1 {
                caption(ui, tr("This file"));
            }
            let lib_id = lib.map(|l| l.manifest.id.clone());
            for (f, styles) in by_folder(list.iter().copied()) {
                let fold = match &lib_id {
                    Some(id) => format!("{id}/{f}"),
                    None => f.to_string(),
                };
                if !f.is_empty() {
                    // Searching shows every match, folders open.
                    let closed = q.is_empty() && app.ui.style_folds.contains(&fold);
                    let (r, resp) = ui.allocate_exact_size(vec2(W, ROW_H), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
                    }
                    let chevron = Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(10.0, 10.0));
                    icons::paint(ui.painter(), chevron, if closed { Icon::ChevronRight } else { Icon::Chevron }, t.menu_dim);
                    let label = f.split('/').map(str::trim).collect::<Vec<_>>().join(" / ");
                    crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 22.0, r.center().y), &label, regular(11.0), t.menu_dim, W - 26.0);
                    if resp.clicked() && q.is_empty() {
                        out.act = Some(Act::ToggleFolder(fold.clone()));
                    }
                    if closed {
                        continue;
                    }
                }
                let keyed: Vec<(&Style, Key)> = styles.into_iter().map(|s| (s, Key { library: lib_id.clone(), style: s.id.clone() })).collect();
                // Library styles are read-only here.
                let editable = |k: &Key| k.library.is_none();
                if grid {
                    ui.add_space(4.0);
                    style_grid(ui, &keyed, applied.as_ref(), editable, out);
                } else {
                    for (s, key) in keyed {
                        let on = applied.as_ref() == Some(&key);
                        let can_edit = editable(&key);
                        style_row(ui, s, key, slot, on, can_edit, out);
                    }
                }
            }
        }
        if !colors.is_empty() {
            ui.add_space(6.0);
            caption(ui, tr("Color variables"));
        }
        for (lib, c, vars) in &colors {
            crate::variable_ui::group_caption(ui, *lib, c);
            let all = lib.map_or(&doc.variables, |l| &l.doc.variables);
            for v in vars {
                if crate::variable_ui::variable_row(ui, all, v, W) {
                    out.act = Some(Act::Bind(crate::variable_ui::Pick { library: lib.map(|l| l.manifest.id.clone()), id: v.id.clone() }));
                }
            }
        }
    });
}

/// One property row in the editor (a paint, an effect, a grid); paints open the colour picker.
fn property_row(ui: &mut Ui, label: &str, paint: Option<&Paint>, slot: StyleSlot) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(W, ROW_H), if paint.is_some() { Sense::click() } else { Sense::hover() });
    if paint.is_some() && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let pr = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0));
    match paint {
        Some(p) => widgets::paint_swatch(ui.painter(), pr, p),
        None => icons::paint(ui.painter(), pr, slot.icon(), t.menu_text),
    }
    ui.painter().text(pos2(r.min.x + 26.0, r.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.menu_text);
    paint.is_some() && resp.clicked()
}

/// The variable on `field` of the style a [`property_row`] just drew: its name at the row's end
/// (a click unbinds it), or, with the pointer over the row, the button that opens the variable
/// picker for it. True when the pointer is on it (the row's own click isn't meant then).
fn binding_cell(ui: &mut Ui, doc: &Document, style: &Style, field: &str, kind: figcraft_doc::VariableType, out: &mut Frame) -> bool {
    let t = Tokens::get(ui.ctx());
    let row = ui.min_rect();
    let row = Rect::from_min_max(pos2(row.min.x, row.max.y - ROW_H), row.max);
    let Some(name) = style.bound.get(field).map(|v| doc.variables.get(v).map_or(v.as_str(), |x| x.name.as_str())) else {
        let b = Rect::from_center_size(pos2(row.max.x - 13.0, row.center().y), vec2(20.0, 20.0));
        if ui.rect_contains_pointer(row) && menu_icon(ui, b, Icon::Variable, tr("Apply variable"), false).clicked() {
            crate::variable_ui::open_style(ui, &style.id, field, kind, row.left_bottom());
        }
        return ui.rect_contains_pointer(b);
    };
    let id = Id::new(("style_binding", &style.id, field));
    let shown = widgets::elide_font(ui, leaf(name), 90.0, regular(10.0));
    let w = ui.painter().layout_no_wrap(shown.clone(), regular(10.0), t.menu_dim).size().x + 22.0;
    let chip = Rect::from_min_max(pos2(row.max.x - w - 4.0, row.min.y + 4.0), pos2(row.max.x - 4.0, row.max.y - 4.0));
    let resp =
        ui.interact(chip, id, Sense::click()).on_hover_text(crate::i18n::trf("Bound to {name}. Click to detach the variable.", &[("name", &name)]));
    ui.painter().rect_filled(chip, CornerRadius::same(4), if resp.hovered() { t.menu_hover } else { t.menu });
    ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.menu_dim.gamma_multiply(0.4)), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(chip.min.x + 9.0, chip.center().y), vec2(11.0, 11.0)), Icon::Variable, t.menu_dim);
    ui.painter().text(pos2(chip.min.x + 18.0, chip.center().y), Align2::LEFT_CENTER, shown, regular(10.0), t.menu_text);
    if resp.clicked() {
        out.act = Some(Act::Unbind(style.id.clone(), field.to_string()));
    }
    resp.hovered()
}

/// The field of effect `fx` (item `i` of an effect style) a variable binds from its row, and its
/// type: a shadow's colour, a blur's radius.
fn effect_binding(fx: &figcraft_doc::Effect, i: usize) -> Option<(String, figcraft_doc::VariableType)> {
    use figcraft_doc::{EffectKind as K, VariableType as V};
    match fx.kind {
        K::DropShadow(_) | K::InnerShadow(_) => Some((format!("effects/{i}/color"), V::Color)),
        K::LayerBlur(_) | K::BackgroundBlur(_) => Some((format!("effects/{i}/radius"), V::Number)),
        _ => None,
    }
}

/// A layout grid's summary ("Columns 12", "Grid 8").
fn grid_label(g: &figcraft_doc::LayoutGrid) -> String {
    match g.kind {
        GridKind::Grid => trf("Grid {size}", &[("size", &g.size)]),
        GridKind::Columns => trf("Columns {count}", &[("count", &g.count)]),
        GridKind::Rows => trf("Rows {count}", &[("count", &g.count)]),
        GridKind::Diagonal => trf("45° {count}", &[("count", &g.count)]),
        GridKind::Rings => trf("Rings {count}", &[("count", &g.count)]),
    }
}

/// The chip in `rect` of the variable `name` bound to `field` of text style `style`. A click on it
/// opens the variable picker to change it, and its detach button unbinds it.
#[allow(clippy::too_many_arguments)]
fn text_binding(ui: &mut Ui, style: &str, field: &str, kind: VariableType, rect: Rect, name: &str, lead: widgets::Lead, out: &mut Frame) {
    match widgets::bound_field(ui, Id::new(("ts_bound", style, field)), lead, rect, name) {
        Some(widgets::BindEvent::Open(at)) => crate::variable_ui::open_style(ui, style, field, kind, at),
        Some(widgets::BindEvent::Detach) => out.act = Some(Act::Unbind(style.to_string(), field.to_string())),
        None => {}
    }
}

/// A text style's typography in the editor: a sample, then font, style, size, line height and
/// letter spacing; a change redefines the style (every linked char follows). Each field takes a
/// variable: the font and style menus start with Apply variable, the other fields show a variable
/// button on hover, and a bound field shows its variable instead of its value.
fn text_properties(ui: &mut Ui, doc: &Document, style: &Style, def: &TextStyle, out: &mut Frame) {
    let id = style.id.as_str();
    let bound = |field: &str| style.bound.get(field).map(|v| doc.variables.get(v).map_or_else(|| v.clone(), |x| x.name.clone()));
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(W, 40.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(5), Color32::from_white_alpha(14));
    text_sample(ui, def, "Ag", 28.0, pos2(r.min.x + 10.0, r.center().y), 60.0, r.height() - 4.0, &mut out.budget);
    let summary = format!("{} {}\n{}", def.font_family, def.font_style, text_meta(def));
    let g = ui.painter().layout(summary, regular(10.5), t.menu_dim, W - 80.0);
    ui.painter().galley(pos2(r.min.x + 72.0, r.center().y - g.size().y / 2.0), g, t.menu_dim);
    ui.add_space(6.0);
    let set = |out: &mut Frame, f: &dyn Fn(&mut TextStyle)| {
        let mut d = def.clone();
        f(&mut d);
        if d != *def {
            out.act = Some(Act::SetText(id.to_string(), d));
        }
    };
    let half = (W - 8.0) / 2.0;
    // The dropdowns' menus open outside the dark scope (they're regular popups).
    let (family_resp, style_resp) = crate::theme::on_dark(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        let family_resp = match bound("fontFamily") {
            Some(name) => {
                let (r, _) = ui.allocate_exact_size(vec2(W, widgets::FIELD_H), Sense::hover());
                text_binding(ui, id, "fontFamily", VariableType::String, r, &name, widgets::Lead::Named(tr("Font")), out);
                None
            }
            None => Some(widgets::dropdown(ui, &def.font_family, W, None).on_hover_text(tr("Font"))),
        };
        let mut style_resp = None;
        ui.horizontal(|ui| {
            match bound("fontStyle") {
                Some(name) => {
                    let (r, _) = ui.allocate_exact_size(vec2(half, widgets::FIELD_H), Sense::hover());
                    text_binding(ui, id, "fontStyle", VariableType::String, r, &name, widgets::Lead::Named(tr("Font style")), out);
                }
                None => style_resp = Some(widgets::dropdown(ui, &def.font_style, half, None).on_hover_text(tr("Font style"))),
            }
            let size = bound("fontSize");
            let (edit, ev) = widgets::number_field_bindable(
                ui,
                Id::new(("ts_size", id)),
                widgets::Lead::Text(tr("Size")),
                Some(def.font_size),
                "",
                half,
                1.0,
                Some(size.as_deref()),
                widgets::Empty::Text(tr("Mixed")),
            );
            if let Some(widgets::FieldEdit::Set(v) | widgets::FieldEdit::Scrub(v, true)) = edit.and_then(|e| e.settled()) {
                set(out, &|d| d.font_size = v.clamp(1.0, 5000.0));
            }
            match ev {
                Some(widgets::BindEvent::Open(at)) => crate::variable_ui::open_style(ui, id, "fontSize", VariableType::Number, at),
                Some(widgets::BindEvent::Detach) => out.act = Some(Act::Unbind(id.to_string(), "fontSize".to_string())),
                None => {}
            }
        });
        ui.horizontal(|ui| {
            let area = Rect::from_min_size(ui.cursor().min, vec2(half, widgets::FIELD_H));
            match bound("lineHeight") {
                Some(name) => {
                    let (r, _) = ui.allocate_exact_size(area.size(), Sense::hover());
                    text_binding(ui, id, "lineHeight", VariableType::Number, r, &name, widgets::Lead::Named(tr("Line height")), out);
                }
                None => {
                    if let Some(v) = widgets::text_field(ui, Id::new(("ts_lh", id)), &def.line_height.label(), half, tr("Line height"))
                        && let Some(lh) =
                            crate::typography::parse_line_height(&v, Some(def.line_height)).and_then(|v| crate::typography::line_height_of(&v))
                    {
                        set(out, &|d| d.line_height = lh);
                    }
                    if crate::variable_ui::variable_button(ui, area, Id::new(("ts_bind", id, "lineHeight"))) {
                        crate::variable_ui::open_style(ui, id, "lineHeight", VariableType::Number, area.left_bottom());
                    }
                }
            }
            let area = Rect::from_min_size(ui.cursor().min, vec2(half, widgets::FIELD_H));
            match bound("letterSpacing") {
                Some(name) => {
                    let (r, _) = ui.allocate_exact_size(area.size(), Sense::hover());
                    text_binding(ui, id, "letterSpacing", VariableType::Number, r, &name, widgets::Lead::Named(tr("Letter spacing")), out);
                }
                None => {
                    if let Some(v) = widgets::text_field(ui, Id::new(("ts_ls", id)), &def.letter_spacing.label(), half, tr("Letter spacing"))
                        && let Some(ls) = crate::typography::parse_letter_spacing(&v, Some(def.letter_spacing))
                            .and_then(|v| crate::typography::letter_spacing_of(&v))
                    {
                        set(out, &|d| d.letter_spacing = ls);
                    }
                    if crate::variable_ui::variable_button(ui, area, Id::new(("ts_bind", id, "letterSpacing"))) {
                        crate::variable_ui::open_style(ui, id, "letterSpacing", VariableType::Number, area.left_bottom());
                    }
                }
            }
        });
        (family_resp, style_resp)
    });
    if let Some(family_resp) = family_resp {
        crate::right::menu_popup(&family_resp).width(260.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if crate::right::menu_item(ui, tr("Apply variable"), "") {
                crate::variable_ui::open_style(ui, id, "fontFamily", VariableType::String, family_resp.rect.left_bottom());
                ui.close();
                return;
            }
            ui.separator();
            if let Some(f) = crate::typography::font_picker(ui, Some(&def.font_family), family_resp.clicked()).picked {
                set(out, &|d| {
                    d.font_family = f.clone();
                    let styles = figcraft_text::family_styles(&f);
                    if !styles.is_empty() && !styles.contains(&d.font_style) {
                        d.set_font_style(styles.iter().find(|s| s.eq_ignore_ascii_case("Regular")).or(styles.first()).cloned().unwrap_or_default());
                    }
                });
                ui.close();
            }
        });
    }
    if let Some(sr) = style_resp {
        crate::right::menu_popup(&sr).width(200.0).show(|ui| {
            if crate::right::menu_item(ui, tr("Apply variable"), "") {
                crate::variable_ui::open_style(ui, id, "fontStyle", VariableType::String, sr.rect.left_bottom());
                ui.close();
                return;
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for s in figcraft_text::family_styles(&def.font_family) {
                    let mark = if def.font_style == s { "✓ " } else { "   " };
                    if crate::right::menu_item(ui, &format!("{mark}{s}"), "") {
                        set(out, &|d| d.set_font_style(s.clone()));
                        ui.close();
                    }
                }
            });
        });
    }
}

/// Editing one style: name, description, its values (paints open the colour picker; text styles
/// have their typography), update from the selection, delete.
fn editor(ui: &mut Ui, doc: &Document, p: &mut Picker, out: &mut Frame) {
    let slot = p.slot;
    let has_selection = !p.ids.is_empty();
    let Some(e) = p.edit.as_mut() else { return };
    let Some(style) = doc.style(&e.id).cloned() else {
        out.act = Some(Act::Back);
        return;
    };
    let (hr, _) = ui.allocate_exact_size(vec2(W, 24.0), Sense::hover());
    if menu_icon(ui, Rect::from_min_size(hr.min, vec2(24.0, 24.0)), Icon::ChevronLeft, tr("Back to styles"), false).clicked() {
        out.act = Some(Act::Back);
    }
    ui.add_space(6.0);
    caption(ui, tr("Name"));
    let r = ui.add(egui::TextEdit::singleline(&mut e.name).font(regular(11.0)).desired_width(W));
    if e.focus {
        r.request_focus();
        let mut state = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(e.name.chars().count()));
        state.cursor.set_char_range(Some(all));
        egui::TextEdit::store_state(ui.ctx(), r.id, state);
        e.focus = false;
    }
    crate::widgets::select_all_on_focus(ui, &r, &e.name);
    ui.add_space(6.0);
    caption(ui, tr("Description"));
    ui.add(egui::TextEdit::multiline(&mut e.description).font(regular(11.0)).desired_width(W).desired_rows(2));
    ui.add_space(8.0);
    caption(ui, tr("Properties"));
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match &style.kind {
            StyleKind::Paint { paints } if paints.is_empty() => caption(ui, tr("No paints")),
            StyleKind::Paint { paints } => {
                // Top of the list paints on top: show in reverse like the Fill section.
                for (i, paint) in paints.iter().enumerate().rev() {
                    let label = format!("{}  {:.0}%", tr(&paint.label()), paint.opacity * 100.0);
                    let clicked = property_row(ui, &label, Some(paint), slot);
                    // A solid paint's colour takes a variable.
                    let on_binding = matches!(paint.kind, PaintKind::Solid { .. })
                        && binding_cell(ui, doc, &style, &format!("fills/{i}/color"), figcraft_doc::VariableType::Color, out);
                    if clicked && !on_binding {
                        out.act = Some(Act::EditPaint(style.id.clone(), i));
                    }
                }
            }
            StyleKind::Effect { effects } => {
                for (i, fx) in effects.iter().enumerate().rev() {
                    property_row(ui, tr(fx.label()), None, slot);
                    if let Some((field, kind)) = effect_binding(fx, i) {
                        binding_cell(ui, doc, &style, &field, kind, out);
                    }
                }
            }
            StyleKind::Grid { grids } => {
                for (i, g) in grids.iter().enumerate() {
                    property_row(ui, &grid_label(g), None, slot);
                    binding_cell(ui, doc, &style, &format!("layoutGrids/{i}/size"), figcraft_doc::VariableType::Number, out);
                }
            }
            StyleKind::Text { style: def } => text_properties(ui, doc, &style, def, out),
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if has_selection && menu_button(ui, tr("Update from selection")).on_hover_text(tr("Take the values from the first selected layer")).clicked()
        {
            out.act = Some(Act::UpdateFromSelection(style.id.clone()));
        }
        if menu_button(ui, tr("Delete")).on_hover_text(tr("Delete the style (layers keep their values)")).clicked() {
            out.act = Some(Act::Delete(style.id.clone()));
        }
    });
}

/// Write the typed name and description of the style being edited, when they changed.
fn commit_edit(app: &mut FigApp, doc: &Document, e: &Editing) {
    let Some(s) = doc.style(&e.id) else { return };
    let name = e.name.trim();
    let mut q = json!({"style": e.id});
    if !name.is_empty() && name != s.name {
        q["name"] = json!(name);
    }
    if e.description != s.description {
        q["description"] = json!(e.description);
    }
    if q.as_object().is_some_and(|o| o.len() > 1) {
        app.run("style.update", q);
    }
}

/// The picker popover (call once per frame).
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut p) = app.ui.style_picker.take() else { return };
    let Ok(doc) = app.session.ds().map(|d| d.text_preview.clone().unwrap_or_else(|| d.doc.clone())) else { return };
    // A new selection closes the picker (it acts on the layers it was opened for).
    if app.session.selection() != p.ids {
        if let Some(e) = &p.edit {
            commit_edit(app, &doc, e);
        }
        app.ui.style_picker_popover = None;
        return;
    }
    let screen = ctx.content_rect();
    // Left of the Design panel; further left while the colour picker (which sits there) is open.
    let mut x = screen.max.x - app.ui.right_width - W - 32.0;
    if app.ui.picker.is_some() {
        x -= 260.0;
    }
    let y = p.y.clamp(screen.min.y + 48.0, (screen.max.y - 460.0).max(screen.min.y + 48.0));
    let mut out = Frame { act: None, hover: None, budget: 4 };
    // The title and the X are the shared popover's now; the list keeps its tools row.
    let title = if p.edit.is_some() { tr("Edit style") } else { p.slot.title() };
    let mut pop = app.ui.style_picker_popover.take().unwrap_or_else(|| Popover::new("style_picker_area", title).pad(PAD).surface(Surface::Menu));
    pop.title = title.to_string();
    // Clicks in the colour picker editing this picker's style, in the variable picker binding one
    // of its fields, or in a menu the editor opened, keep it open.
    let picker_rect =
        app.ui.picker.as_ref().filter(|t| matches!(t, PickerTarget::Style { .. })).and(ctx.memory(|m| m.area_rect(Id::new("color_picker"))));
    let variables = crate::variable_ui::picker_rect(ctx);
    let holds: Vec<Rect> = picker_rect.into_iter().chain(variables).collect();
    let closed = pop.show(ctx, Open::At(pos2(x, y)), Hold { rects: &holds, escape: variables.is_some() || picker_rect.is_some() }, |ui| {
        if p.edit.is_some() {
            editor(ui, &doc, &mut p, &mut out);
        } else {
            listing(app, ui, &doc, &mut p, &mut out);
        }
    });
    let mut close = closed;
    let ids = p.ids.clone();
    let slot = p.slot;
    // The canvas shows the text style under the pointer on the selected text.
    if slot == StyleSlot::Text
        && !close
        && out.act.is_none()
        && let Some(key) = &out.hover
    {
        let (cmd, params) = key.apply(&ids, slot);
        app.preview_text(ctx, cmd, params);
    }
    match out.act {
        Some(Act::Apply(key)) => {
            let (cmd, params) = key.apply(&ids, slot);
            app.run(cmd, params);
            close = true;
        }
        Some(Act::Bind(pick)) => {
            crate::variable_ui::bind(app, &ids, slot.key(), &pick);
            close = true;
        }
        Some(Act::Detach) => {
            app.run("style.detach", json!({"ids": ids, "which": slot.key()}));
        }
        Some(Act::Create) => {
            let name = next_name(&doc, slot.default_name());
            let made = app.run("style.create", json!({"type": slot.create_type(), "name": name, "from": ids.first()}));
            if let Some(id) = made.as_ref().and_then(|v| v["id"].as_str()) {
                p.query.clear();
                p.edit = Some(Editing { id: id.to_string(), name, description: String::new(), focus: true });
            }
        }
        Some(Act::Edit(id)) => p.edit = doc.style(&id).map(editing),
        Some(Act::Back) => {
            if let Some(e) = p.edit.take() {
                commit_edit(app, &doc, &e);
            }
        }
        Some(Act::EditPaint(id, index)) => {
            if let Some(e) = &p.edit {
                commit_edit(app, &doc, e);
            }
            // No swatch to sit on here (the click came from the editor's list): the picker takes
            // its usual height rather than the one a swatch opened it at before.
            app.open_picker(PickerTarget::Style { id, index }, None);
        }
        Some(Act::Unbind(id, field)) => {
            app.run("style.bind", json!({"style": id, "field": field}));
        }
        Some(Act::SetText(id, def)) => {
            app.run("style.update", json!({"style": id, "value": def}));
        }
        Some(Act::UpdateFromSelection(id)) => {
            app.run("style.update", json!({"style": id, "from": ids.first()}));
        }
        Some(Act::Delete(id)) => {
            app.run("style.delete", json!({"style": id}));
            p.edit = None;
        }
        Some(Act::ToggleFolder(f)) => {
            if !app.ui.style_folds.remove(&f) {
                app.ui.style_folds.insert(f);
            }
        }
        Some(Act::ToggleView) => app.ui.style_grid = !app.ui.style_grid,
        None => {}
    }
    if close {
        if let Some(e) = &p.edit {
            commit_edit(app, &doc, e);
        }
        if matches!(app.ui.picker, Some(PickerTarget::Style { .. })) {
            app.ui.picker = None;
        }
        app.ui.style_picker_popover = None;
    } else {
        app.ui.style_picker = Some(p);
        app.ui.style_picker_popover = Some(pop);
    }
}

/// Open the style editor on this file's style `id` (from the Local styles list).
fn edit_local(app: &mut FigApp, ui: &Ui, slot: StyleSlot, id: &str) {
    let y = ui.input(|i| i.pointer.interact_pos()).map_or(140.0, |p| p.y - 12.0);
    let edit = app.session.doc().ok().and_then(|d| d.style(id).map(editing));
    crate::colorpicker::close(app);
    app.ui.style_picker_popover = None;
    app.ui.style_picker = Some(Picker { slot, ids: app.session.selection(), y, query: String::new(), edit });
}

/// The name `moved` takes in `folder`: its last part there, numbered when that name is taken
/// ("Sunset/600 2") so a drop never leaves two styles sharing one.
fn moved_name(doc: &Document, moved: &Style, folder: &str) -> String {
    let leaf = leaf(&moved.name);
    let named = |l: &str| if folder.is_empty() { l.to_string() } else { format!("{folder}/{l}") };
    let taken = |n: &str| doc.styles.iter().any(|s| s.id != moved.id && s.name == n);
    let first = named(leaf);
    if !taken(&first) {
        return first;
    }
    (2..).map(|k| named(&format!("{leaf} {k}"))).find(|n| !taken(n)).unwrap_or(first)
}

/// One drawn row of the Local styles list: a folder caption or a style. Both take a drop, which is
/// what lets a style be dropped between two folders — into a folder of its own, or above the first
/// folder, which sends it back to the root.
enum Slot<'a> {
    Header { path: &'a str, rect: Rect },
    Style { index: usize, style: &'a Style, rect: Rect },
}

impl Slot<'_> {
    /// The row's band, used for the drop's side and the pointer's target.
    fn rect(&self) -> Rect {
        match self {
            Slot::Header { rect, .. } | Slot::Style { rect, .. } => *rect,
        }
    }
}

/// Where a drop at `at` lands: the line's y (for the insertion line) and the `style.move` query.
///
/// The rows are the entries, captions included, so every folder is a target on both of its sides:
/// the top half of a caption ends the group above it, its bottom half opens the folder under it —
/// which is what lets a style be dropped into a folder that holds only deeper folders, or into one
/// that is folded. A style row's either half ends it (the line sits under it), so a style dropped
/// next to its own place keeps its order. `before` is the first style under the line, the dragged
/// one never counting.
fn drop_move(doc: &Document, slots: &[Slot<'_>], from: usize, at: Pos2) -> Option<(f32, Value)> {
    let (fp, moved) = slots.iter().enumerate().find_map(|(i, s)| match s {
        Slot::Style { index, style, .. } if *index == from => Some((i, *style)),
        _ => None,
    })?;
    // The line: under the row the pointer is in, over the top half of a caption (which ends the
    // group above it), or above the first row.
    let (line, y) = match slots.iter().filter(|s| s.rect().min.y <= at.y).count().checked_sub(1) {
        None => (0, slots.first()?.rect().min.y),
        Some(i) => {
            let r = slots[i].rect();
            let over_caption = matches!(slots[i], Slot::Header { .. }) && at.y < r.center().y;
            match (over_caption, slots.get(i + 1)) {
                (true, _) => (i, r.min.y),
                (false, Some(next)) => (i + 1, next.rect().min.y),
                (false, None) => (i + 1, r.max.y),
            }
        }
    };
    let before = slots.iter().skip(line).find_map(|s| match s {
        Slot::Style { style, .. } if style.id != moved.id => Some(style.id.clone()),
        _ => None,
    });
    // The folder the style joins: the row under the line's own; a caption's, the group above it (a
    // style's folder, a caption's folder, or the root above the first caption); the list's end,
    // the last row's own.
    let into = match slots.get(line) {
        Some(Slot::Style { style, .. }) => folder(&style.name).to_string(),
        Some(Slot::Header { .. }) => match line.checked_sub(1).map(|i| &slots[i]) {
            Some(Slot::Style { style, .. }) => folder(&style.name).to_string(),
            Some(Slot::Header { path, .. }) => path.to_string(),
            None => String::new(),
        },
        None => match slots.last()? {
            Slot::Style { style, .. } => folder(&style.name).to_string(),
            Slot::Header { path, .. } => path.to_string(),
        },
    };
    // A style that lands where it already is is no move at all.
    if (line == fp || line == fp + 1) && folder(&moved.name) == into {
        return None;
    }
    let name = moved_name(doc, moved, &into);
    let mut q = json!({"styles": [moved.id]});
    if let Some(b) = before {
        q["before"] = json!(b);
    }
    if name != moved.name {
        q["name"] = json!(name);
    }
    Some((y, q))
}

/// The id of the folder caption being renamed in the Local styles list (temp data).
fn folder_rename_id() -> Id {
    Id::new("local_style_folder_rename")
}

/// The folders folded in the Local styles list (temp data), as `slot/folder`.
fn folded_id() -> Id {
    Id::new("local_style_folded")
}

/// A Local styles row: the allocated full-width band, cut to the list's side insets.
fn inset_row(row: Rect) -> Rect {
    Rect::from_min_max(pos2(row.min.x + LS_INSET, row.min.y), pos2(row.max.x - LS_INSET, row.max.y))
}

/// Where a row's icon box starts: the list's padding plus one step per nesting level.
fn icon_x(row: Rect, level: f32) -> f32 {
    row.min.x + (LS_PAD - LS_INSET) + level * LS_STEP
}

/// A 16 pt `+` at a row's right end, 8 pt in from the panel's edge; true when it is clicked. Drawn
/// after the row it sits on, so it takes the click where the two overlap.
fn row_plus(ui: &mut Ui, id: Id, row: Rect, tip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let r = Rect::from_min_size(pos2(row.max.x - LS_INSET - LS_ICON, row.center().y - LS_ICON / 2.0), vec2(LS_ICON, LS_ICON));
    let resp = ui.interact(r, id, Sense::click()).tip(tip);
    if resp.hovered() {
        ui.painter().rect_filled(r.expand(4.0), CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), r, Icon::Plus, t.icon_secondary);
    resp
}

/// A style's colour as a round swatch centred at `c`: solid, or a gradient spread around the rim.
fn paint_dot(ui: &Ui, c: egui::Pos2, r: f32, paint: &Paint) {
    let t = Tokens::get(ui.ctx());
    let stops: Vec<Color32> = match &paint.kind {
        PaintKind::Solid { color } => vec![crate::theme::doc_c32(ui.ctx(), color.with_alpha(color.a * paint.opacity))],
        PaintKind::Gradient(g) => (0..=4).map(|i| crate::theme::doc_c32(ui.ctx(), g.sample(i as f32 / 4.0))).collect(),
        PaintKind::Image(_) | PaintKind::Pattern(_) => {
            let icon = if matches!(paint.kind, PaintKind::Image(_)) { Icon::Image } else { Icon::ViewGrid };
            ui.painter().circle_filled(c, r, Color32::from_gray(0xB3));
            icons::paint(ui.painter(), Rect::from_center_size(c, vec2(r, r)), icon, Color32::WHITE);
            ui.painter().circle_stroke(c, r - 0.5, Stroke::new(1.0, t.border));
            return;
        }
    };
    match stops.as_slice() {
        [] => {}
        [one] => {
            ui.painter().circle_filled(c, r, *one);
        }
        many => {
            // A fan of rim vertices, each sampling the gradient along its own direction.
            const N: usize = 32;
            let at = |v: f32| {
                let x = v.clamp(0.0, 1.0) * (many.len() - 1) as f32;
                let i = (x.floor() as usize).min(many.len() - 2);
                many[i].lerp_to_gamma(many[i + 1], x - i as f32)
            };
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(c, at(0.5));
            for k in 0..=N {
                let a = k as f32 / N as f32 * std::f32::consts::TAU;
                let d = vec2(a.cos(), a.sin());
                mesh.colored_vertex(c + d * r, at(0.5 + (d.x + d.y) * 0.35));
            }
            for k in 1..=N as u32 {
                mesh.add_triangle(0, k, k + 1);
            }
            ui.painter().add(egui::Shape::mesh(mesh));
        }
    }
    ui.painter().circle_stroke(c, r - 0.5, Stroke::new(1.0, t.border));
}

/// The Design panel's Local styles (shown with nothing selected): this file's styles by type
/// (text, colour, effect, grid), then by folder. Rows sit inset from the panel's edges with the
/// hover band rounded; a folder folds from its chevron. A click edits a style; right-click offers
/// edit, duplicate and delete; a row dragged by itself moves where the line shows, into the folder
/// of the row it lands above (renamed, numbered if that name is taken: see [`drop_move`]);
/// right-click on a folder renames it (and the styles in it).
pub fn local_styles(app: &mut FigApp, ui: &mut Ui, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    let renaming: Option<String> = ui.data(|d| d.get_temp(folder_rename_id()));
    let mut folded: std::collections::HashSet<String> = ui.data(|d| d.get_temp(folded_id())).unwrap_or_default();
    let folded_before = folded.clone();
    let mut run: Option<(&'static str, Value)> = None;
    let mut edit: Option<(StyleSlot, String)> = None;
    // A `+` asked for a new style: its slot, and the folder it lands in ("" at the top level).
    let mut create: Option<(StyleSlot, String)> = None;
    let mut groups = 0;
    // The section's header, with the `+` that asks which type of style to make.
    let any = library::local_styles(doc).next().is_some();
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().text(
        pos2(head.min.x + LS_PAD, head.center().y),
        Align2::LEFT_CENTER,
        tr("Local styles"),
        regular(TEXT_PT),
        if any { t.text } else { t.text_secondary },
    );
    let add = row_plus(ui, ui.id().with("ls_add"), head, tr("Add style"));
    // The section's `+` has no type of its own: it asks which one to make.
    crate::right::menu_popup(&add).show(|ui| {
        for slot in [StyleSlot::Text, StyleSlot::Fill, StyleSlot::Effect, StyleSlot::Grid] {
            if crate::right::menu_item(ui, tr(slot.title()), "") {
                create = Some((slot, String::new()));
                ui.close();
            }
        }
    });
    for slot in [StyleSlot::Text, StyleSlot::Fill, StyleSlot::Effect, StyleSlot::Grid] {
        let list: Vec<&Style> = library::local_styles(doc).filter(|s| slot.holds(&s.kind)).collect();
        if list.is_empty() {
            continue;
        }
        ui.add_space(if groups == 0 { LS_FIRST_GAP } else { LS_GROUP_GAP });
        groups += 1;
        let w = ui.available_width();
        let (caption, _) = ui.allocate_exact_size(vec2(w, LS_CAPTION_H), Sense::hover());
        ui.painter().text(pos2(caption.min.x + LS_PAD, caption.center().y), Align2::LEFT_CENTER, slot.title(), regular(11.0), t.text_secondary);
        // This group's `+` makes one of its type: no menu to pick from.
        if row_plus(ui, ui.id().with(("ls_add_slot", slot.key())), caption, tr("Add style")).clicked() {
            create = Some((slot, String::new()));
        }
        // One drag for the whole type: a style can be dropped in another folder (it then takes
        // that folder's name), so the groups aren't separate lists.
        let mut drag = crate::list_drag::ListDrag::in_order(ui, ("local_styles", slot.key()));
        let mut slots: Vec<Slot> = Vec::new();
        let mut styles = 0;
        let lines = entries(list.into_iter(), |path| folded.contains(&format!("{}/{}", slot.key(), path)));
        for line in &lines {
            let depth = line.depth() as f32;
            let w = ui.available_width();
            match line {
                Entry::Folder { path, label, .. } => {
                    let key = format!("{}/{}", slot.key(), path);
                    let open = !folded.contains(&key);
                    let (row, resp) = ui.allocate_exact_size(vec2(w, LS_ROW_H), Sense::click());
                    let r = inset_row(row);
                    let x = icon_x(r, depth);
                    // The `+` at the row's right end keeps the name out of its 16 pt box.
                    let text_r = Rect::from_min_max(pos2(x + LS_STEP, r.min.y + 2.0), pos2(r.max.x - LS_ICON - LS_ICON_GAP, r.max.y - 2.0));
                    if renaming.as_deref() == Some(*path) {
                        // Focused as it is first drawn: a focus asked for before its field is drawn
                        // isn't in the screen reader tree (UI Automation aborts the app, #224).
                        let start = ui.data_mut(|d| d.remove_temp::<bool>(folder_rename_id().with("start"))).unwrap_or(false);
                        let (out, still_open) = widgets::rename_field(ui, folder_rename_id().with(*path), path, text_r, regular(11.0), start);
                        if let Some(name) = out {
                            run = Some(("style.renameFolder", json!({"folder": path, "name": name})));
                        }
                        if !still_open {
                            ui.data_mut(|d| d.remove::<String>(folder_rename_id()));
                        }
                    } else {
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
                        }
                        let chevron = Rect::from_center_size(pos2(x + LS_ICON / 2.0, r.center().y), vec2(LS_CHEVRON, LS_CHEVRON));
                        icons::paint(ui.painter(), chevron, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
                        // The path names the folder; the last part is what the tree shows.
                        let resp = resp.tip(*path);
                        crate::bidi::paint_name(
                            ui.painter(),
                            pos2(x + LS_STEP, r.center().y),
                            label,
                            regular(11.0),
                            t.text_secondary,
                            text_r.width(),
                        );
                        // This folder's `+` makes one of the group's type, in this folder.
                        if row_plus(ui, ui.id().with(("ls_add_in", slot.key(), path)), row, tr("Add style")).clicked() {
                            create = Some((slot, path.to_string()));
                        }
                        crate::right::context_popup(&resp).show(|ui| {
                            if crate::right::menu_item(ui, tr("Rename folder"), "") {
                                ui.data_mut(|d| d.insert_temp(folder_rename_id(), path.to_string()));
                                ui.data_mut(|d| d.insert_temp(folder_rename_id().with("start"), true));
                                ui.close();
                            }
                        });
                        if resp.clicked() {
                            if open {
                                folded.insert(key.clone());
                            } else {
                                folded.remove(&key);
                            }
                        }
                    }
                    // The caption takes no drag of its own, but a drop over it has to say which
                    // side it belongs to, which is why it is registered as a header.
                    slots.push(Slot::Header { path, rect: r });
                    drag.header(r);
                }
                Entry::Style(s, _) => {
                    let index = styles;
                    styles += 1;
                    let (row, resp) = ui.allocate_exact_size(vec2(w, LS_ROW_H), Sense::click_and_drag());
                    let r = inset_row(row);
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
                    }
                    let x = icon_x(r, depth);
                    let icon_r = Rect::from_min_size(pos2(x, r.center().y - LS_ICON / 2.0), vec2(LS_ICON, LS_ICON));
                    let name_x = x + LS_STEP;
                    let mut name_w = r.max.x - LS_TRAIL - name_x;
                    match &s.kind {
                        StyleKind::Paint { paints } => {
                            let p = paints.last().cloned().unwrap_or_else(|| Paint::solid(figcraft_color::Color::TRANSPARENT));
                            paint_dot(ui, icon_r.center(), LS_ICON / 2.0, &p);
                        }
                        _ => match text_def(s) {
                            Some(def) => {
                                ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, "Ag", semibold(11.0), t.text);
                                let meta = ui.painter().text(
                                    pos2(r.max.x - LS_TRAIL, r.center().y),
                                    Align2::RIGHT_CENTER,
                                    text_meta(def),
                                    regular(11.0),
                                    t.text_tertiary,
                                );
                                name_w -= meta.width() + 8.0;
                            }
                            None => icons::paint(ui.painter(), icon_r, slot.icon(), t.icon_secondary),
                        },
                    }
                    crate::bidi::paint_name(ui.painter(), pos2(name_x, r.center().y), leaf(&s.name), regular(11.0), t.text, name_w.max(0.0));
                    // The whole row drags (a style row is too narrow for a grip beside its swatch).
                    drag.row_by(&resp, index, r, || leaf(&s.name).to_string());
                    slots.push(Slot::Style { index, style: s, rect: r });
                    let tip = if s.description.is_empty() { s.name.clone() } else { format!("{}\n{}", s.name, s.description) };
                    let resp = resp.tip(tip);
                    if resp.clicked() {
                        edit = Some((slot, s.id.clone()));
                    }
                    crate::right::context_popup(&resp).show(|ui| {
                        if crate::right::menu_item(ui, tr("Edit style"), "") {
                            edit = Some((slot, s.id.clone()));
                            ui.close();
                        }
                        if crate::right::menu_item(ui, tr("Duplicate"), "") {
                            run = Some(("style.duplicate", json!({"style": s.id})));
                            ui.close();
                        }
                        if crate::right::menu_item(ui, tr("Delete"), "") {
                            run = Some(("style.delete", json!({"style": s.id})));
                            ui.close();
                        }
                    });
                }
            }
        }
        // The whole type group is a target — its caption and its folders' headers included — so
        // its first and last rows are as easy to reach as the ones between. The rows decide the
        // drop (see `drop_move`), so the list ends at the last one of them, style or folder.
        if let Some(last) = slots.last() {
            let area = Rect::from_min_max(pos2(last.rect().min.x, caption.min.y), pos2(last.rect().max.x, last.rect().max.y + LS_ROW_H / 2.0));
            if let Some(q) = drag.bounds(area).line_inset(0.0).finish_in(ui, |from, at| drop_move(doc, &slots, from, at)) {
                run = Some(("style.move", q));
            }
        }
    }
    if groups > 0 {
        ui.add_space(LS_BOTTOM_PAD);
    }
    if folded != folded_before {
        ui.data_mut(|d| d.insert_temp(folded_id(), folded.clone()));
    }
    if let Some((cmd, q)) = run {
        app.run(cmd, q);
    }
    if let Some((slot, folder)) = create {
        let q = new_style_params(doc, &app.session.selection(), slot, &folder);
        if let Some(id) = app.run("style.create", q).as_ref().and_then(|v| v["id"].as_str()) {
            // Straight into the editor: a style just made has nothing but its default values.
            edit_local(app, ui, slot, id);
        }
    }
    if let Some((slot, id)) = edit {
        edit_local(app, ui, slot, &id);
    }
}

/// `style.create` params for a new `slot` style in `folder` ("" at the top level), named after the
/// slot ("Brand/Color 3"). A style made while a layer is selected takes that layer's values; with
/// nothing selected it takes the slot's own starting value, so the `+` always works.
fn new_style_params(doc: &Document, ids: &[NodeId], slot: StyleSlot, folder: &str) -> Value {
    let base = if folder.is_empty() { slot.default_name().to_string() } else { format!("{folder}/{}", slot.default_name()) };
    let mut q = json!({"type": slot.create_type(), "name": next_name(doc, &base)});
    match ids.first() {
        Some(from) => q["from"] = json!(from),
        None => {
            q["value"] = match slot {
                StyleSlot::Fill | StyleSlot::Stroke => json!("#000000"),
                StyleSlot::Text => json!({"fontSize": 16.0}),
                StyleSlot::Effect | StyleSlot::Grid => json!([]),
            }
        }
    }
    q
}

#[cfg(test)]
#[path = "style_picker_tests.rs"]
mod tests;
