//! Figma-style compact controls: number fields with scrub labels, text fields, dropdowns,
//! segmented buttons, colour swatches and section headers.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use crate::a11y::{self, Tip};
pub use crate::field_math::{Rel, Typed, eval, parse, stepped};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
pub(crate) mod board_picker;
pub mod colorpicker;
pub mod field_math;
pub mod font_preview;
pub mod style_picker;

pub const ROW_H: f32 = 32.0;
pub const FIELD_H: f32 = 24.0;
pub const PAD: f32 = 16.0;
/// A section's right margin: 8. The panel's grid is `16 | 88 | 8 | 88 | 8 | 24 | 8`: a 240 pt
/// panel, its content 16 in from the left and 8 from the right, two 88 pt columns, an 8 pt gutter
/// between them and 8 + 24 + 8 for the buttons at a row's end. FigCraft's own, from the panel as it
/// is drawn: the columns are equal, the buttons sit on the same 24 pt rhythm as the icon buttons
/// the panel already has.
pub const PAD_RIGHT: f32 = 8.0;
/// Between the panel's grid columns.
pub const GUTTER: f32 = 8.0;
/// A field's inset before its label, and the room its value keeps at the field's end.
const LEAD_PAD: f32 = 8.0;
/// An icon lead's slot: the 24 pt rhythm the panel's buttons sit on. The whole slot is the scrub
/// target — not only the 14 pt glyph drawn in it.
const LEAD_ICON: f32 = 24.0;
const VALUE_W: f32 = 32.0;
/// The panel's type, in points: a caption under a field row, the labels and values in the rows, and
/// the type of the selection above them. Two points apart, so 12 sits halfway between 10 and 14.
pub const CAPTION_PT: f32 = 10.0;
pub const TEXT_PT: f32 = 12.0;
pub const TITLE_PT: f32 = 14.0;

/// What a numeric field's label shows: text ("X") or an icon with the field's name (for screen
/// readers: an icon doesn't say which property it is).
#[derive(Clone, Copy)]
pub enum Lead<'a> {
    Text(&'a str),
    Icon(Icon, &'a str),
    None,
    /// No lead, only a name for screen readers (the label is beside the field).
    Named(&'a str),
}

impl Lead<'_> {
    /// The field's name for screen readers ("W" is "Width").
    pub fn name(&self) -> &str {
        match self {
            Lead::Text("W") => tr("Width"),
            Lead::Text("H") => tr("Height"),
            Lead::Text(s) => s,
            Lead::Icon(_, name) | Lead::Named(name) => name,
            Lead::None => "",
        }
    }
}

/// What a number field shows when it has no single value, and whether that word types 0.
#[derive(Clone, Copy)]
pub enum Empty<'a> {
    /// Dim text that is only a label ("Mixed", or "Auto" where 0 means something else).
    Text(&'a str),
    /// Auto, in the UI's language: 0 is the field's automatic value, so the word types 0.
    Auto,
}

impl<'a> Empty<'a> {
    /// The text shown when the field has no value.
    fn text(self) -> &'a str {
        match self {
            Empty::Text(s) => s,
            Empty::Auto => tr("Auto"),
        }
    }
}

/// Result of a field edit.
pub enum FieldEdit {
    /// Committed a new value (Enter, Tab or focus loss; an arrow key's step).
    Set(f64),
    /// Scrubbed by dragging the label (live; `.1` is true when the drag ended).
    Scrub(f64, bool),
}

/// What a field over several layers gives ([`number_field_mixed`]): one value for all of them, or
/// (while it shows no single value: "Mixed") an edit relative to each layer's own (`+10`, `*2`, an
/// arrow key's step).
pub enum Edit {
    All(FieldEdit),
    Each(Rel),
}

impl Edit {
    /// For a field that has no value per layer to work from: `+10` is 10, as typed into an empty
    /// field, and other relative edits are nothing.
    pub fn settled(self) -> Option<FieldEdit> {
        match self {
            Edit::All(e) => Some(e),
            Edit::Each(Rel::Add(v)) => Some(FieldEdit::Set(v)),
            Edit::Each(_) => None,
        }
    }
}

/// The nudge amounts the arrow keys step pixel fields by (plain, with Shift): the preferences'
/// small and big nudge. Set once a frame.
pub fn set_nudge(ctx: &egui::Context, small: f64, big: f64) {
    ctx.data_mut(|d| d.insert_temp(Id::unique("field_nudge"), (small, big)));
}

/// What an arrow key (with Shift: `.1`) and a pixel of scrubbing step a field by: a field in
/// pixels (no unit, whole steps) takes the nudge preferences from the keys, every other field
/// its own step and ten of them (1% and 10% of opacity, 1° and 10° of rotation).
fn key_steps(ctx: &egui::Context, suffix: &str, step: f64) -> (f64, f64) {
    let nudge =
        ctx.data(|d| d.get_temp::<(f64, f64)>(Id::unique("field_nudge"))).filter(|(s, b)| s.is_finite() && b.is_finite() && *s > 0.0 && *b > 0.0);
    match nudge {
        Some(n) if suffix.is_empty() && step == 1.0 => n,
        _ => (step, step * 10.0),
    }
}

/// How far a scrub of `dx` points moves a value whose field steps by `step`: 1 step a point, ten
/// with Shift, a tenth with Alt (fine).
pub fn scrub_amount(dx: f64, step: f64, shift: bool, alt: bool) -> f64 {
    dx * step
        * match (shift, alt) {
            (true, false) => 10.0,
            (false, true) => 0.1,
            _ => 1.0,
        }
}

/// The value a scrub that began at `from` and has moved `amount` (see [`scrub_amount`]) shows:
/// whole steps of `quantum` away from where it began.
pub fn scrubbed(from: f64, amount: f64, quantum: f64) -> f64 {
    if quantum > 0.0 && quantum.is_finite() { stepped(from, (amount / quantum).round(), quantum) } else { from }
}

/// The shared label-scrub gesture, including changing modifier rates and raw edge motion.
/// Callers supply the hit area and apply live values until the final `Scrub(_, true)`.
pub(crate) fn scrub(ui: &Ui, id: Id, response: &Response, value: Option<f64>, step: f64) -> Option<FieldEdit> {
    let v = value?;
    // Where the scrub began and how far it has taken the value (the modifiers can change on
    // the way: each stretch counts at its own rate).
    let scrub_id = id.with("scrub");
    if response.dragged() || response.drag_stopped() {
        let began = response.drag_started();
        let (v0, mut amount): (f64, f64) = ui.data(|d| d.get_temp(scrub_id)).filter(|_| !began).unwrap_or((v, 0.0));
        let (dx, shift, alt) = ui.input(|i| {
            let p = &i.pointer;
            // As it begins: all the way from the press (a drag is one only after a few points).
            let moved = if began { p.hover_pos().zip(p.press_origin()).map_or(0.0, |(at, from)| at.x - from.x) } else { p.delta().x };
            // With the pointer stopped at the edge of the screen (or of a window that fills
            // it) the mouse's own motion goes on, where the platform reports it: the scrub
            // does too. Elsewhere the scrub just stops there.
            let raw = p.motion().map_or(0.0, |m| m.x / i.pixels_per_point);
            let at_edge = moved == 0.0 && p.hover_pos().is_none_or(|at| at.x <= 1.0 || at.x >= i.content_rect().max.x - 1.0);
            (f64::from(if at_edge { raw } else { moved }), i.modifiers.shift, i.modifiers.alt)
        });
        // The frame the button goes up moves nothing: the scrub ends where the last one left it.
        if response.dragged() {
            amount += scrub_amount(dx, step, shift, alt);
            ui.data_mut(|d| d.insert_temp(scrub_id, (v0, amount)));
        }
        let quantum = if alt { step * 0.1 } else { step };
        return Some(FieldEdit::Scrub(scrubbed(v0, amount, quantum), response.drag_stopped()));
    }
    None
}

pub(crate) fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

/// What a number field's variable button did.
pub enum BindEvent {
    /// Open the variable picker below this point.
    Open(egui::Pos2),
    /// Detach the bound variable.
    Detach,
}

/// A number field `width` wide. `value` None shows "Mixed". `suffix` (e.g. "%" or "°") is shown
/// after the number. Typing supports simple math (`10+5`, `*2`).
pub fn number_field(ui: &mut Ui, id: Id, lead: Lead, value: Option<f64>, suffix: &str, width: f32, step: f64) -> Option<FieldEdit> {
    number_field_mixed(ui, id, lead, value, suffix, width, step)?.settled()
}

/// [`number_field`] over several layers: without a single value (`value` None, "Mixed") a relative
/// edit comes back as one for each layer ([`Edit::Each`]).
pub fn number_field_mixed(ui: &mut Ui, id: Id, lead: Lead, value: Option<f64>, suffix: &str, width: f32, step: f64) -> Option<Edit> {
    number_field_bindable(ui, id, lead, value, suffix, width, step, None, Empty::Text(tr("Mixed"))).0
}

/// [`number_field`] that shows `empty` (dim) for no value instead of "Mixed" (e.g. "Auto"): there
/// is no value to edit relative to, so `+4` is 4.
pub fn number_field_or(ui: &mut Ui, id: Id, value: Option<f64>, empty: &str, width: f32, step: f64) -> Option<FieldEdit> {
    number_field_impl(ui, id, Lead::None, value, "", width, step, None, Empty::Text(empty), false, None).0?.settled()
}

/// [`number_field`] that can bind a variable: `binding` None = not bindable, `Some(None)` = shows
/// a variable button on hover, `Some(Some(name))` = bound (shows the name; click to change, the
/// detach button unbinds). No value shows `empty` ("Mixed", "Auto"); [`Empty::Auto`] also takes
/// the word for Auto as 0.
pub fn number_field_bindable(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: Empty<'_>,
) -> (Option<Edit>, Option<BindEvent>) {
    let (edit, bind, _) = number_field_impl(ui, id, lead, value, suffix, width, step, binding, empty, false, None);
    (edit, bind)
}

/// [`number_field_bindable`] whose value reads dimmed (50% ink) while it stays editable: a size
/// the layout computes (a Fill child's) is shown that way, and typing turns it into a fixed one.
pub fn number_field_bindable_dim(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: Empty<'_>,
    dim: bool,
) -> (Option<Edit>, Option<BindEvent>) {
    let (edit, bind, _) = number_field_impl(ui, id, lead, value, suffix, width, step, binding, empty, dim, None);
    (edit, bind)
}

/// [`number_field_bindable_dim`] whose own right-end button opens a menu instead of the variable
/// picker: `menu` is `Some(word)` to draw a chevron there (the word takes its place while the
/// pointer is away). The last value is that button's response, for anchoring the menu.
#[allow(clippy::too_many_arguments)]
pub fn number_field_bindable_menu(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: Empty<'_>,
    dim: bool,
    menu: Option<Option<&str>>,
) -> (Option<Edit>, Option<BindEvent>, Option<egui::Response>) {
    number_field_impl(ui, id, lead, value, suffix, width, step, binding, empty, dim, menu)
}

/// A save admitted for another document cannot make this reused field commit its cached draft.
/// Drop only this editor's transient state, before a natural focus-loss branch can read it.
fn cancel_retargeted_field_save(ui: &mut Ui, id: Id) {
    let editor = id.with("te");
    if !crate::shortcuts::take_field_save_cancellation(ui.ctx(), editor) {
        return;
    }
    let select_all = select_all_id(editor);
    ui.data_mut(|d| {
        d.remove::<String>(id);
        d.remove::<String>(id.with("editing"));
        d.remove::<String>(id.with("kept"));
        d.remove::<String>(id.with("buf"));
        d.remove::<bool>(id.with("selectall"));
        d.remove::<bool>(select_all);
        d.remove::<egui::text_edit::TextEditState>(editor);
    });
    ui.memory_mut(|m| m.surrender_focus(editor));
}

#[allow(clippy::too_many_arguments)]
fn number_field_impl(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: Empty<'_>,
    dim: bool,
    menu: Option<Option<&str>>,
) -> (Option<Edit>, Option<BindEvent>, Option<egui::Response>) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    if let Some(Some(name)) = binding {
        return (None, bound_field(ui, id, lead, rect, name), None);
    }
    let mut out: Option<Edit> = None;
    // The lead is the whole scrub target: 8 points of blank before a value with no label, the
    // icon's 24 pt slot with one (the glyph is only 14 of those points, and all 24 drag).
    let lead_w = match lead {
        Lead::None | Lead::Named(_) => LEAD_PAD,
        Lead::Text(s) => text_lead_width(ui, s, width),
        Lead::Icon(..) => LEAD_ICON,
    };
    let lead_rect = Rect::from_min_size(rect.min, vec2(lead_w, FIELD_H));
    let edit_rect = Rect::from_min_max(pos2(rect.min.x + lead_w, rect.min.y), rect.max);
    let editing_id = id.with("editing");
    let mut editing: Option<String> = ui.data(|d| d.get_temp(editing_id));
    let field_resp = ui.interact(edit_rect, id.with("field"), Sense::click());
    a11y::number(&field_resp, lead.name(), value, suffix, empty.text());
    a11y::ring_around(ui, rect, &field_resp);
    // The label scrubs with the pointer; the keyboard stops at the value only.
    let lead_resp = ui.interact(lead_rect, id.with("lead"), Sense::CLICK | Sense::DRAG);
    let hovered = field_resp.hovered() || lead_resp.hovered() || editing.is_some();
    field_shell(ui, rect, editing.is_some(), hovered);
    match lead {
        Lead::Text(s) => {
            let galley = crate::bidi::name_galley(ui.painter(), s, regular(TEXT_PT), t.text_secondary, lead_w - LEAD_PAD);
            // A label cut short shows whole on hover.
            if galley.elided {
                lead_resp.clone().on_hover_text(s);
            }
            ui.painter().galley(pos2(lead_rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0), galley, t.text_secondary);
        }
        Lead::Icon(i, _) => paint_slot(ui, Slot::Icon(i), lead_rect),
        Lead::None | Lead::Named(_) => {}
    }
    if lead_resp.hovered() || lead_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if let Some(edit) = scrub(ui, id, &lead_resp, value, step) {
        out = Some(Edit::All(edit));
    }
    let (key_step, key_big) = key_steps(ui.ctx(), suffix, step);
    // The keyboard is on the field (also in a window that isn't in front: keys reach one only
    // through the control channel then).
    let on_field = ui.memory(|m| m.has_focus(field_resp.id));
    // What ArrowUp or ArrowDown pressed this frame steps by (Shift: the big step).
    let arrow_step = |ui: &Ui| {
        ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key: egui::Key::ArrowUp, pressed: true, modifiers, .. } => Some(if modifiers.shift { key_big } else { key_step }),
                egui::Event::Key { key: egui::Key::ArrowDown, pressed: true, modifiers, .. } => {
                    Some(-if modifiers.shift { key_big } else { key_step })
                }
                _ => None,
            })
        })
    };
    // A relative edit of no single value goes to each layer's own.
    let set = |v: f64| Edit::All(FieldEdit::Set(v));
    let typed = |text: &str| {
        // A field where 0 is its automatic value takes the word for Auto as input too: the
        // placeholder's own text (so it is what the field shows, in the UI's language) or the
        // English word.
        if let Empty::Auto = empty {
            let t = text.trim();
            if t == empty.text() || t.eq_ignore_ascii_case("auto") {
                return Some(set(0.0));
            }
        }
        match (parse(text, suffix == "%")?, value) {
            (Typed::Value(v), _) => Some(set(v)),
            (Typed::Rel(r), Some(cur)) => r.apply(cur).map(set),
            (Typed::Rel(r), None) => Some(Edit::Each(r)),
        }
    };
    // With the keyboard on the field (Tab) and nothing typed yet, the arrows step it as it is.
    if editing.is_none()
        && on_field
        && let Some(by) = arrow_step(ui)
    {
        out = Some(value.map_or(Edit::Each(Rel::Add(by)), |v| set(stepped(v, 1.0, by))));
    }
    // Typing on a field the keyboard reached (Tab) edits it: the editor takes the typed text.
    let typed_now = editing.is_none() && on_field && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
    let started = editing.is_none() && (field_resp.clicked() || typed_now);
    // The text the field would commit to no effect: what it showed when the edit began, or the
    // value the arrows last stepped it to.
    let kept_id = id.with("kept");
    if started {
        editing = Some(if typed_now { String::new() } else { value.map(fmt_num).unwrap_or_default() });
        ui.data_mut(|d| {
            d.insert_temp(editing_id, editing.clone().unwrap_or_default());
            d.insert_temp(kept_id, value.map(fmt_num).unwrap_or_default());
            d.insert_temp(id.with("selectall"), !typed_now);
        });
        ui.memory_mut(|m| m.request_focus(id.with("te")));
    }
    // A computed size (a Fill child's) reads at 50% ink: dimmer than a value, but not as faint as
    // the "Mixed" placeholder. It stays editable, and the editor keeps that colour.
    let dim_col = dim.then(|| if t.dark { Color32::from_white_alpha(128) } else { Color32::from_black_alpha(128) });
    match editing {
        Some(mut text) => {
            let mut te = egui::TextEdit::singleline(&mut text)
                .id(id.with("te"))
                .frame(egui::Frame::NONE)
                // No margin: the value is painted at `edit_rect.min.x` and centred on the row, and
                // the editor's own 4/2 margin would shift it right and down on the click.
                .margin(egui::Margin::ZERO)
                .font(regular(TEXT_PT))
                .desired_width(edit_rect.width() - 6.0)
                .vertical_align(egui::Align::Center);
            if let Some(c) = dim_col {
                te = te.text_color(c);
            }
            // The editor sits centred on the row, exactly where the painted value was.
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let r = child.add(te);
            a11y::own_focus(&r);
            keep_focus(&r, started);
            crate::shortcuts::release_field_for_save(&r);
            if ui.data(|d| d.get_temp::<bool>(id.with("selectall"))).unwrap_or(false)
                && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id.with("te"))
            {
                let ccursor = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                state.cursor.set_char_range(Some(ccursor));
                state.store(ui.ctx(), id.with("te"));
                ui.data_mut(|d| d.remove::<bool>(id.with("selectall")));
            }
            // The Enter that started the edit (from the keyboard) doesn't end it.
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if ui.memory(|m| m.has_focus(r.id))
                && let Some(by) = arrow_step(ui)
            {
                // From what is typed (arithmetic too); no single value: each layer's own steps.
                match typed(&text) {
                    Some(Edit::All(FieldEdit::Set(v))) => {
                        let nv = stepped(v, 1.0, by);
                        text = fmt_num(nv);
                        // The stepped value stays selected: typing replaces it.
                        ui.data_mut(|d| {
                            d.insert_temp(kept_id, text.clone());
                            d.insert_temp(id.with("selectall"), true);
                        });
                        out = Some(set(nv));
                    }
                    _ if value.is_none() => out = Some(Edit::Each(Rel::Add(by))),
                    _ => {}
                }
            }
            if esc {
                // Back to what it was: nothing is committed. The Escape ends this edit only.
                a11y::spend_escape(ui);
                ui.data_mut(|d| d.remove::<String>(editing_id));
                ui.memory_mut(|m| m.surrender_focus(id.with("te")));
            } else if r.lost_focus() || enter {
                // Enter, Tab or a click elsewhere commits what was typed (a field left as it was
                // commits nothing: no undo step, and a rounded display isn't written back).
                let kept: String = ui.data(|d| d.get_temp(kept_id)).unwrap_or_default();
                if text != kept
                    && let Some(e) = typed(&text)
                {
                    out = Some(e);
                }
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else {
                ui.data_mut(|d| d.insert_temp(editing_id, text));
            }
        }
        None => {
            let s = match value {
                Some(v) => format!("{}{}", fmt_num(v), suffix),
                None => empty.text().to_string(),
            };
            let col = dim_col.unwrap_or(if value.is_some() { t.text } else { t.text_secondary });
            // A value too long for its room ends in "…", so a cut number never reads as another.
            crate::bidi::paint_name(ui.painter(), pos2(edit_rect.min.x, rect.center().y), &s, regular(TEXT_PT), col, edit_rect.width());
        }
    }
    // The field's own right-end button: the sizing menu's chevron when `menu` is set (a fill shows
    // its word there until the pointer comes back), else the variable button over a bindable field.
    let mut bind = None;
    let mut menu_resp = None;
    if let Some(word) = menu {
        let w = if word.is_some() { 26.0 } else { 18.0 };
        let br = Rect::from_center_size(pos2(rect.max.x - w / 2.0 - 3.0, rect.center().y), vec2(w, 18.0));
        let bresp = ui.interact(br, id.with("sizing_menu"), Sense::click());
        if bresp.hovered() {
            ui.painter().rect_filled(br, CornerRadius::same(4), t.hover);
        }
        if let Some(word) = word.filter(|_| !bresp.hovered()) {
            let g = ui.painter().layout_no_wrap(word.to_string(), regular(11.0), t.text_secondary);
            ui.painter().galley(pos2(br.center().x - g.size().x / 2.0, br.center().y - g.size().y / 2.0), g, t.text_secondary);
        } else {
            icons::paint(ui.painter(), Rect::from_center_size(br.center(), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
        }
        menu_resp = Some(bresp);
    } else if binding.is_some() && ui.data(|d| d.get_temp::<String>(editing_id)).is_none() {
        let br = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(18.0, 18.0));
        let bresp = ui.interact(br, id.with("bind"), Sense::click());
        if bresp.has_focus() || rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(egui::Pos2::new(f32::NAN, f32::NAN))) {
            if bresp.hovered() {
                ui.painter().rect_filled(br, CornerRadius::same(4), t.hover);
            }
            icons::paint(ui.painter(), br.shrink(3.0), Icon::Variable, t.icon_secondary);
        }
        let bresp = bresp.tip(tr("Apply variable"));
        if bresp.clicked() {
            bind = Some(BindEvent::Open(rect.left_bottom()));
        }
    }
    (out, bind, menu_resp)
}

/// The width a text label takes at the start of a field `field_w` wide: the label as drawn plus
/// its padding, leaving the value room for a few characters (a longer label ends in an ellipsis).
pub(crate) fn text_lead_width(ui: &Ui, label: &str, field_w: f32) -> f32 {
    let text = ui.painter().layout_no_wrap(label.to_string(), regular(TEXT_PT), Color32::PLACEHOLDER).size().x;
    (LEAD_PAD + text).min(field_w - VALUE_W).max(24.0)
}

/// A field bound to a variable: the lead, then the variable's name in a chip; the chip opens the
/// picker and the detach button unbinds.
/// A field showing the variable `name` bound to it in `rect`: click to change, detach to unbind.
pub fn bound_field(ui: &mut Ui, id: Id, lead: Lead, rect: Rect, name: &str) -> Option<BindEvent> {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id.with("bound"), Sense::click());
    a11y::combo(&resp, lead.name(), name, false);
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    let lead_w = match lead {
        Lead::None | Lead::Named(_) => 4.0,
        Lead::Text(s) => {
            let w = text_lead_width(ui, s, rect.width());
            crate::bidi::paint_name(ui.painter(), pos2(rect.min.x + 8.0, rect.center().y), s, regular(TEXT_PT), t.text_secondary, w - 12.0);
            w
        }
        Lead::Icon(i, _) => {
            paint_slot(ui, Slot::Icon(i), Rect::from_min_size(rect.min, vec2(24.0, rect.height())));
            24.0
        }
    };
    let dr = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(18.0, 18.0));
    let chip = Rect::from_min_max(pos2(rect.min.x + lead_w - 2.0, rect.min.y + 3.0), pos2(dr.min.x - 2.0, rect.max.y - 3.0));
    ui.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
    ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    // Grouped names ("size/card") show their last part; the tooltip has the full name.
    let short = name.rsplit('/').next().unwrap_or(name);
    let galley = ui.painter().layout_no_wrap(short.to_string(), regular(TEXT_PT), t.text);
    ui.painter().with_clip_rect(chip.shrink2(vec2(5.0, 0.0))).galley(pos2(chip.min.x + 5.0, chip.center().y - galley.size().y / 2.0), galley, t.text);
    let dresp = ui.interact(dr, id.with("detach"), Sense::click()).tip(tr("Detach variable"));
    if resp.hovered() || dresp.hovered() || dresp.has_focus() {
        if dresp.hovered() {
            ui.painter().rect_filled(dr, CornerRadius::same(4), t.hover);
        }
        icons::paint(ui.painter(), dr.shrink(3.0), Icon::Detach, t.icon_secondary);
    }
    if dresp.clicked() {
        return Some(BindEvent::Detach);
    }
    let resp = resp.on_hover_text(name);
    resp.clicked().then(|| BindEvent::Open(rect.left_bottom()))
}

/// A text field that commits on Enter / focus loss. Returns the new text when committed.
/// A single-line text input: see [`field`] for the unified builder (this is `field` with no
/// leading or trailing slot).
pub fn text_field(ui: &mut Ui, id: Id, value: &str, width: f32, placeholder: &str) -> Option<String> {
    field(
        ui,
        FieldOpts { id, value, width, placeholder, name: placeholder, secret: false, lead: Slot::None, trail: Slot::None, trail_suffix: "trail" },
    )
    .0
}

/// [`text_field`] without a placeholder, named `name` for screen readers (its label is beside it).
pub fn text_field_named(ui: &mut Ui, id: Id, value: &str, width: f32, name: &str) -> Option<String> {
    field(ui, FieldOpts { id, value, width, placeholder: "", name, secret: false, lead: Slot::None, trail: Slot::None, trail_suffix: "trail" }).0
}

/// [`text_field_named`] for a secret (a server key): it shows dots, typing included, and screen
/// readers hear dots too.
pub fn secret_field_named(ui: &mut Ui, id: Id, value: &str, width: f32, name: &str) -> Option<String> {
    field(ui, FieldOpts { id, value, width, placeholder: "", name, secret: true, lead: Slot::None, trail: Slot::None, trail_suffix: "trail" }).0
}

/// A multi-line text field across the panel (descriptions, notes) that commits when it loses
/// focus: returns the new text then, when it changed. The draft lives only while it has focus.
pub fn multiline_field(ui: &mut Ui, id: Id, value: &str, hint: &str, rows: usize) -> Option<String> {
    cancel_retargeted_field_save(ui, id);
    let t = Tokens::get(ui.ctx());
    let te = id.with("te");
    let focused = ui.memory(|m| m.has_focus(te));
    let mut text: String = if focused { ui.data(|d| d.get_temp(id)).unwrap_or_else(|| value.to_string()) } else { value.to_string() };
    let mut done = false;
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        let w = ui.available_width() - PAD;
        let r = egui::Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id(te)
                    .frame(egui::Frame::NONE)
                    .font(regular(TEXT_PT))
                    .hint_text(hint)
                    .desired_rows(rows)
                    .desired_width(w - 12.0),
            )
        });
        crate::shortcuts::release_field_for_save(&r.inner);
        done = r.inner.lost_focus();
    });
    if done {
        ui.data_mut(|d| d.remove::<String>(id));
        return (text != value).then_some(text);
    }
    if ui.memory(|m| m.has_focus(te)) {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    None
}

/// An editable field with a menu of `options` behind the chevron at its right (export sizes…).
/// Returns the typed text (on Enter / focus loss) or the option picked.
/// An editable field with a menu of `options` behind the chevron at its right (export sizes…):
/// returns the typed text (on Enter / focus loss) or the option picked. The chevron is the field's
/// trailing slot ([`Slot::Icon`]); its response anchors the menu.
pub fn combo_field(ui: &mut Ui, id: Id, value: &str, width: f32, options: &[String]) -> Option<String> {
    let (mut out, chev) = field(
        ui,
        FieldOpts {
            id,
            value,
            width,
            placeholder: "",
            name: tr("Presets"),
            secret: false,
            lead: Slot::None,
            trail: Slot::Icon(Icon::Chevron),
            trail_suffix: "menu",
        },
    );
    if let Some(resp) = chev {
        a11y::combo(&resp, tr("Presets"), value, a11y::menu_open(&resp));
        crate::right::menu_popup(&resp).show(|ui| {
            for o in options {
                if crate::right::menu_item(ui, o, "") {
                    out = Some(o.clone());
                    ui.close();
                }
            }
        });
    }
    out
}

/// [`text_field`] drawn into `rect`; `name` names it for screen readers.
/// A slot at the leading or trailing end of a field: nothing, a short label, an icon, a colour
/// dot, or arbitrary content drawn by `Custom`. One builder ([`field`]) then covers "icon-or-text
/// lead" and "text-or-icon trail" without a separate function per combination.
#[derive(Clone, Copy)]
pub enum Slot<'a> {
    None,
    Text(&'a str),
    Icon(Icon),
    Dot(Color32),
    Custom(&'a dyn Fn(&mut Ui)),
}

impl Slot<'_> {
    /// The slot's width at the end of a `FIELD_H` field.
    fn width(self, ui: &Ui) -> f32 {
        match self {
            Slot::None => 0.0,
            Slot::Text(s) => {
                (LEAD_PAD + ui.painter().layout_no_wrap(s.to_string(), regular(TEXT_PT), Color32::PLACEHOLDER).size().x).min(64.0).max(16.0)
            }
            // The design's slot is 24 points wide whatever it holds (frame29's `Slot` frame).
            Slot::Icon(_) | Slot::Dot(_) | Slot::Custom(_) => LEAD_ICON,
        }
    }
}

/// Options for [`field`].
pub struct FieldOpts<'a> {
    pub id: Id,
    pub value: &'a str,
    pub width: f32,
    pub placeholder: &'a str,
    pub name: &'a str,
    pub secret: bool,
    pub lead: Slot<'a>,
    pub trail: Slot<'a>,
    /// The trailing slot's response id suffix (defaults to `"trail"`). A combo keeps `"menu"`, the
    /// id its dropdown anchor has always had.
    pub trail_suffix: &'static str,
}

/// The background and border every input wears: the field fill, the accent while `editing`, a
/// hairline while `hovered`. Shared by the text fields and the number fields so the two look the
/// same wherever they sit side by side.
fn field_shell(ui: &mut Ui, rect: Rect, editing: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if editing {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
}

/// A single-line text input with an optional leading and trailing slot (icon, text, dot or custom
/// content): the unified builder behind [`text_field`], [`text_field_named`],
/// [`secret_field_named`] and [`combo_field`]. The middle is the editable text; `lead`/`trail` fill
/// the insets. Returns the committed text and, when a trailing slot was drawn, its response (so a
/// chevron can anchor a menu).
pub fn field(ui: &mut Ui, opts: FieldOpts) -> (Option<String>, Option<Response>) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(opts.width, FIELD_H), Sense::hover());
    let lead_w = opts.lead.width(ui);
    let trail_w = opts.trail.width(ui);
    let edit_rect = Rect::from_min_max(pos2(rect.min.x + lead_w, rect.min.y), pos2(rect.max.x - trail_w, rect.max.y));
    let active = ui.data(|d| d.get_temp::<String>(opts.id.with("editing"))).is_some();
    field_shell(ui, rect, active, ui.rect_contains_pointer(rect));
    paint_slot(ui, opts.lead, Rect::from_min_size(rect.min, vec2(lead_w, FIELD_H)));
    let out = text_field_edit(ui, opts.id, opts.value, edit_rect, opts.placeholder, opts.name, opts.secret);
    let mut trail_resp = None;
    if !matches!(opts.trail, Slot::None) {
        let tr = Rect::from_min_size(pos2(rect.max.x - trail_w, rect.min.y), vec2(trail_w, FIELD_H));
        let resp = ui.interact(tr, opts.id.with(opts.trail_suffix), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(tr.shrink(2.0), CornerRadius::same(4), t.hover);
        }
        paint_slot(ui, opts.trail, tr);
        trail_resp = Some(resp);
    }
    (out, trail_resp)
}

/// Paint `slot` centred in `area` (or, for `Custom`, let it draw).
fn paint_slot(ui: &mut Ui, slot: Slot, area: Rect) {
    let t = Tokens::get(ui.ctx());
    match slot {
        Slot::None => {}
        Slot::Text(s) => {
            let galley = crate::bidi::name_galley(ui.painter(), s, regular(TEXT_PT), t.text_secondary, area.width());
            ui.painter().galley(pos2(area.min.x + 8.0, area.center().y - galley.size().y / 2.0), galley, t.text_secondary);
        }
        Slot::Icon(i) => {
            // A 16 pt glyph in the 24 pt slot, as the design's `Icon` instance is.
            icons::paint(ui.painter(), Rect::from_center_size(area.center(), vec2(16.0, 16.0)), i, t.icon_secondary);
        }
        Slot::Dot(c) => {
            // The design's colour dot (`Field-color`): a 14 pt rounded square with a hairline,
            // not a circle.
            let dot = Rect::from_center_size(area.center(), vec2(14.0, 14.0));
            ui.painter().rect_filled(dot, CornerRadius::same(3), c);
            ui.painter().rect_stroke(dot, CornerRadius::same(3), Stroke::new(1.0, t.border), StrokeKind::Inside);
        }
        Slot::Custom(f) => {
            f(ui);
        }
    }
}

/// A search box: a leading search icon, a live single-line text input, and (while `text` is
/// non-empty) a trailing clear icon. The leading and trailing icons share the unified [`Slot`]
/// painting with [`field`], so a search box is the same "lead slot + input + trail slot" control
/// as every other input — the one place the product hand-rolled an icon beside a field.
/// `width`/`height` size the box; `focus` pulls the keyboard in on the frame it is set (the box
/// just opened). Escape clears `text` and, when `open` is `Some`, also closes the box.
pub fn search_field(ui: &mut Ui, id: Id, width: f32, height: f32, hint: &str, text: &mut String, focus: bool, open: Option<&mut bool>) {
    let t = Tokens::get(ui.ctx());
    let (fr, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    // The field background is inset within `fr` (as the original hand-rolled box did); only the
    // leading/trailing icon painting is unified through [`Slot`].
    let field = Rect::from_min_max(fr.min + vec2(12.0, 2.0), fr.max - vec2(12.0, 6.0));
    ui.painter().rect_filled(field, CornerRadius::same(5), t.field);
    paint_slot(ui, Slot::Icon(Icon::Search), Rect::from_min_size(field.min, vec2(24.0, height)));
    let te_rect = Rect::from_min_max(pos2(field.min.x + 24.0, field.center().y - 7.0), pos2(field.max.x - 6.0, field.center().y + 7.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(te_rect));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(regular(11.0))
            .hint_text(hint)
            .align(egui::Align2::LEFT_CENTER)
            .desired_width(te_rect.width()),
    );
    if focus {
        r.request_focus();
    }
    select_all_on_focus(ui, &r, text);
    if r.has_focus() {
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
    }
    // Consume Escape so it does not bubble to global handlers; closing is opt-in via `open`.
    if (r.has_focus() || r.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        if let Some(o) = open {
            *o = false;
        }
        text.clear();
    }
    if !text.is_empty() {
        let cr = Rect::from_center_size(pos2(field.max.x - 5.0, field.center().y), vec2(16.0, 16.0));
        let cres = ui.interact(cr, id.with("clear"), Sense::click()).tip(tr("Clear"));
        if cres.hovered() {
            ui.painter().rect_filled(cr, CornerRadius::same(4), t.hover);
        }
        paint_slot(ui, Slot::Icon(Icon::Close), cr);
        if cres.clicked() {
            text.clear();
        }
    }
}

/// Options for [`menu_row`].
pub struct MenuRowOpts<'a> {
    /// The row's label.
    pub label: &'a str,
    /// Whether the row can be clicked (disabled rows dim and never report a click).
    pub enabled: bool,
    /// Draws a check at the leading slot (and tells screen readers) when no `lead` is set.
    pub checked: bool,
    /// Pixels the label starts further in (nested layers).
    pub indent: f32,
    /// A leading slot — an icon, check or dot — drawn at the row's left. When `None` and
    /// `checked`, a check is drawn there instead.
    pub lead: Slot<'a>,
    /// A trailing slot — an icon or dot — drawn at the row's right. When `None`, `shortcut` is
    /// drawn as trailing text instead.
    pub trail: Slot<'a>,
    /// A trailing shortcut hint, drawn as text when `trail` is `None`.
    pub shortcut: &'a str,
}

/// Draw a frame92-style menu row into `r` for `resp` (both allocated by the caller) and return the
/// width the menu should grow to. `hov` is the row's highlight: the caller decides it, since it may
/// depend on menu-level submenu state (such as `plain_row_hover`) that this helper does not own.
/// `lead`/`trail` take a `Slot`; when they are `None`, a `checked` row draws a check at the left and
/// a non-empty `shortcut` draws at the right.
pub fn menu_row_draw(ui: &mut Ui, r: Rect, resp: &Response, opts: MenuRowOpts, hov: bool) -> f32 {
    let t = Tokens::get(ui.ctx());
    a11y::menu_item(resp, opts.label, opts.shortcut, opts.checked.then_some(true));
    if !opts.enabled {
        a11y::disabled(resp);
    }
    a11y::own_focus(resp);
    let hov = opts.enabled && hov;
    if hov {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let col = if opts.enabled { t.menu_text } else { t.menu_dim };
    // Leading slot, or a check when checked and no lead was given.
    if !matches!(opts.lead, Slot::None) {
        paint_slot(ui, opts.lead, Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(24.0, 28.0)));
    } else if opts.checked {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(12.0, 12.0)), Icon::Check, col);
    }
    let galley = ui.painter().layout_no_wrap(opts.label.to_string(), regular(12.0), col);
    let keys = (!opts.shortcut.is_empty()).then(|| {
        ui.painter().layout_no_wrap(crate::toolbar::pretty_shortcut(opts.shortcut), regular(12.0), if hov { t.menu_text } else { t.menu_dim })
    });
    let need = 28.0 + opts.indent + galley.size().x + keys.as_ref().map_or(8.0, |k| 16.0 + k.size().x + 10.0);
    // Trailing slot, or the shortcut text when no trail was given.
    if matches!(opts.trail, Slot::None) {
        if let Some(k) = keys {
            ui.painter().galley(pos2(r.max.x - 10.0 - k.size().x, r.center().y - k.size().y / 2.0), k, col);
        }
    } else {
        paint_slot(ui, opts.trail, Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(24.0, 28.0)));
    }
    let text_rect = Rect::from_min_max(pos2(r.min.x + 28.0 + opts.indent, r.min.y), pos2(r.max.x - 8.0, r.max.y));
    ui.painter().with_clip_rect(text_rect).galley(pos2(text_rect.min.x, r.center().y - galley.size().y / 2.0), galley, col);
    need
}

/// A menu row allocated at the current cursor — the common case, highlighted on hover or keyboard
/// focus. Returns whether it was clicked and the width the menu should grow to.
pub fn menu_row(ui: &mut Ui, opts: MenuRowOpts) -> (bool, f32) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let hov = resp.hovered() || resp.has_focus();
    let need = menu_row_draw(ui, r, &resp, opts, hov);
    (resp.clicked(), need)
}

/// [`text_field`] drawn into `edit_rect`: the field's background and slots are drawn by the
/// caller ([`field`]); `name` names it for screen readers.
fn text_field_edit(ui: &mut Ui, id: Id, value: &str, edit_rect: Rect, placeholder: &str, name: &str, secret: bool) -> Option<String> {
    cancel_retargeted_field_save(ui, id);
    let t = Tokens::get(ui.ctx());
    let editing_id = id.with("editing");
    let mut editing: Option<String> = ui.data(|d| d.get_temp(editing_id));
    let resp = ui.interact(edit_rect, id.with("click"), Sense::click());
    let shown = if secret { std::borrow::Cow::Owned("•".repeat(value.chars().count())) } else { std::borrow::Cow::Borrowed(value) };
    a11y::text(&resp, name, &shown);
    let started = editing.is_none() && resp.clicked();
    if started {
        editing = Some(value.to_string());
        ui.memory_mut(|m| m.request_focus(id.with("te")));
        select_all_next(ui, id.with("te"), value);
    }
    let mut out = None;
    match editing {
        Some(mut text) => {
            let r = field_editor(ui, edit_rect, id.with("te"), &mut text, "", secret);
            a11y::own_focus(&r);
            keep_focus(&r, started);
            crate::shortcuts::release_field_for_save(&r);
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if esc {
                a11y::spend_escape(ui);
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else if r.lost_focus() || enter {
                out = Some(text);
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else {
                ui.data_mut(|d| d.insert_temp(editing_id, text));
            }
        }
        None => {
            let (s, col) = if value.is_empty() { (placeholder, t.text_tertiary) } else { (&*shown, t.text) };
            // Long text stays inside the field.
            let painter = ui.painter().with_clip_rect(edit_rect.shrink2(vec2(8.0, 0.0)));
            painter.text(pos2(edit_rect.min.x + 8.0, edit_rect.center().y), Align2::LEFT_CENTER, s, regular(TEXT_PT), col);
        }
    }
    out
}

/// The editor inside a field's `rect`: one line of text in the field's font, without a frame of
/// its own (the field draws it), showing `hint` while empty; `password` shows dots.
fn field_editor(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str, password: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    // The editor sits centred on the row, as the painted value does: its own margin would push
    // the text right and down when the click starts editing.
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(regular(TEXT_PT))
            .hint_text(egui::RichText::new(hint).font(regular(TEXT_PT)).color(t.text_tertiary))
            .desired_width(rect.width() - 16.0),
    );
    select_all_on_focus(ui, &r, text);
    r
}

/// A text field that is typed into in place: `text` changes as it is typed (the caller decides
/// what Enter does). As tall as a button and drawn like [`text_field`], so the two line up in a
/// row. Returns the editor's response (name it for screen readers with `a11y::text`).
pub fn text_input(ui: &mut Ui, id: Id, text: &mut String, width: f32, hint: &str) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    let r = field_editor(ui, rect, id, text, hint, false);
    field_shell(ui, rect, r.has_focus(), ui.rect_contains_pointer(rect));
    r
}

/// Focus for a field's editor that appears on the click that starts editing: egui takes focus
/// away from a widget a click lands outside of, and a widget that is new this frame isn't under
/// the pointer yet, so the focus is asked for again once it exists.
fn keep_focus(r: &Response, started: bool) {
    if started && !r.has_focus() {
        r.request_focus();
    }
}

/// Selects all of a one-line editor's `text` when it gains the keyboard (a click, Tab or a focus
/// request), so what is typed replaces the value, as in Figma's fields. Call it after adding the
/// editor (`r` is its response). egui puts the caret where the press landed, so the selection is
/// laid again until the button is released; a drag from the press selects what it covers instead.
pub fn select_all_on_focus(ui: &Ui, r: &Response, text: &str) {
    let pending = select_all_id(r.id);
    if r.gained_focus() {
        ui.data_mut(|d| d.insert_temp(pending, true));
    }
    if !ui.data(|d| d.get_temp::<bool>(pending)).unwrap_or(false) {
        return;
    }
    // Not focused yet (the click that starts an edit can land before the editor exists): wait.
    if !r.has_focus() {
        return;
    }
    let (down, dragged) = ui.input(|i| (i.pointer.any_down(), i.pointer.total_drag_delta().is_some_and(|d| d.length() > 6.0)));
    if down && dragged {
        ui.data_mut(|d| d.remove::<bool>(pending));
        return;
    }
    // A key typed in the frame the edit started already replaced the selection laid beforehand.
    if r.changed() {
        ui.data_mut(|d| d.remove::<bool>(pending));
        return;
    }
    select_all(ui, r.id, text);
    ui.ctx().request_repaint();
    if !down {
        ui.data_mut(|d| d.remove::<bool>(pending));
    }
}

/// Has [`select_all_on_focus`] select all of editor `te_id` from now on (an edit that starts
/// with the editor already focused, which then gains no focus). Call it before adding the editor:
/// the text is selected at once, so a key typed in the frame the edit starts (the frame after a
/// double-click or F2 is often the one with the first key) replaces it.
fn select_all_next(ui: &Ui, te_id: Id, text: &str) {
    ui.data_mut(|d| d.insert_temp(select_all_id(te_id), true));
    select_all(ui, te_id, text);
}

/// Selects all of `text` in editor `te_id`.
fn select_all(ui: &Ui, te_id: Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), te_id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    state.cursor.set_char_range(Some(all));
    state.store(ui.ctx(), te_id);
}

fn select_all_id(te_id: Id) -> Id {
    te_id.with("select_all_on_focus")
}

/// What a dropdown shows before its label.
#[derive(Clone, Copy)]
pub enum DropLead {
    Icon(Icon),
    /// A colour dot (a category's colour).
    Dot(Color32),
}

/// A dropdown-looking button showing `label`; returns the response (open a menu on click).
pub fn dropdown(ui: &mut Ui, label: &str, width: f32, lead: Option<Icon>) -> Response {
    dropdown_lead(ui, label, width, lead.map(DropLead::Icon))
}

/// [`dropdown`] with an icon or a colour dot before the label.
pub fn dropdown_lead(ui: &mut Ui, label: &str, width: f32, lead: Option<DropLead>) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::click());
    a11y::combo(&resp, "", label, a11y::menu_open(&resp));
    let bg = t.field;
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    let mut x = rect.min.x + 8.0;
    match lead {
        Some(DropLead::Icon(i)) => {
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(14.0, 14.0)), i, t.icon_secondary);
            x += 18.0;
        }
        Some(DropLead::Dot(c)) => {
            ui.painter().circle_filled(pos2(x + 5.0, rect.center().y), 4.5, c);
            x += 16.0;
        }
        None => {}
    }
    // One line: a label longer than the field (a variable font style like "SemiBold SemiCondensed",
    // a long translation) ends with an ellipsis and shows in full on hover.
    let shown = elide(ui, label, rect.max.x - x - 20.0);
    let galley = ui.painter().layout_no_wrap(shown.clone(), regular(TEXT_PT), t.text);
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, t.text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.max.x - 10.0, rect.center().y), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    if shown != label { resp.on_hover_text(label) } else { resp }
}

/// `s` shortened with an ellipsis to fit `max_w` on one line at the UI's 11 px.
pub fn elide(ui: &Ui, s: &str, max_w: f32) -> String {
    elide_font(ui, s, max_w, regular(TEXT_PT))
}

/// [`elide`] in `font`.
pub fn elide_font(ui: &Ui, s: &str, max_w: f32, font: egui::FontId) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x <= max_w;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| format!("{}…", chars.iter().take(n).collect::<String>().trim_end()))
        .find(|c| fits(c))
        .unwrap_or_else(|| "…".into())
}

/// [`elide_font`] cut at its start, keeping its end: a path keeps its file and the folders
/// nearest it.
pub fn elide_start_font(ui: &Ui, s: &str, max_w: f32, font: egui::FontId) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x <= max_w;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| format!("…{}", chars[chars.len() - n..].iter().collect::<String>().trim_start()))
        .find(|c| fits(c))
        .unwrap_or_else(|| "…".into())
}

/// A row of icon buttons; returns the index clicked.
pub fn segmented(ui: &mut Ui, items: &[(Icon, &str)], active: Option<usize>, cell: f32) -> Option<usize> {
    let names: Vec<&str> = items.iter().map(|(_, tip)| *tip).collect();
    segments(ui, &names, active, cell, true, |ui, r, i, on, t| {
        if let Some((icon, _)) = items.get(i) {
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), *icon, if on { t.icon } else { t.icon_secondary });
        }
    })
}

/// A choice between a few labelled options across the available width (the active one raised);
/// returns the index clicked. A label too long for its cell ends with an ellipsis.
pub fn segmented_text(ui: &mut Ui, labels: &[&str], active: usize) -> Option<usize> {
    let cell = (ui.available_width() / labels.len().max(1) as f32).floor();
    segments(ui, labels, Some(active), cell, false, |ui, r, i, on, t| {
        let label = labels.get(i).copied().unwrap_or_default();
        let font = if on { semibold(TEXT_PT) } else { regular(TEXT_PT) };
        let shown = elide_font(ui, label, r.width() - 12.0, font.clone());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, shown, font, if on { t.text } else { t.text_secondary });
    })
}

/// The cells of a segmented control, each `cell` wide and named by `names` (shown on hover when
/// `tips`), drawn by `paint(ui, cell, index, active, tokens)`; returns the index clicked.
fn segments(
    ui: &mut Ui,
    names: &[&str],
    active: Option<usize>,
    cell: f32,
    tips: bool,
    paint: impl Fn(&Ui, Rect, usize, bool, &Tokens),
) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let w = cell * names.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let mut out = None;
    for (i, name) in names.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x + i as f32 * cell, rect.min.y), vec2(cell, FIELD_H));
        let mut resp = ui.interact(r, ui.scope_id().with(("seg", i, *name)), Sense::click());
        if tips {
            resp = resp.on_hover_text(*name);
        }
        a11y::toggle(&resp, name, active == Some(i));
        if active == Some(i) {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(r.shrink(2.0), CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.hover);
        }
        paint(ui, r, i, active == Some(i), &t);
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// A row that shows or hides what is under it: a chevron and `label`; returns whether it was
/// clicked.
pub fn disclosure(ui: &mut Ui, label: &str, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(TEXT_PT), t.text_secondary);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, FIELD_H), Sense::click());
    a11y::disclosure(&resp, label, open);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let chevron = Rect::from_center_size(pos2(rect.min.x + 8.0, rect.center().y), vec2(10.0, 10.0));
    icons::paint(ui.painter(), chevron, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
    ui.painter().galley(pos2(rect.min.x + 18.0, rect.center().y - galley.size().y / 2.0), galley, t.text_secondary);
    resp.clicked()
}

/// A section header ("Fill", "Stroke"…) with optional right-side icon buttons; returns which
/// button was clicked. `dim` greys the title (empty sections).
pub fn section_header(ui: &mut Ui, title: &str, buttons: &[(Icon, &str)], dim: bool) -> Option<usize> {
    section_header_resp(ui, title, buttons, dim).0
}

/// [`section_header`], with the header's own response for a right-click menu.
pub fn section_header_resp(ui: &mut Ui, title: &str, buttons: &[(Icon, &str)], dim: bool) -> (Option<usize>, egui::Response) {
    section_header_impl(ui, title, Slot::None, buttons, dim)
}

/// The shared section-header body: an optional leading slot (frame29's marker), the title and the
/// trailing icon buttons. See [`section`] for the general form with children.
fn section_header_impl(ui: &mut Ui, title: &str, lead: Slot, buttons: &[(Icon, &str)], dim: bool) -> (Option<usize>, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    // Only an empty section's header acts as its `+` button; elsewhere it is a right-click target
    // alone, so it keeps the normal cursor.
    if resp.hovered() && !dim {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    let lead_w = lead.width(ui);
    if lead_w > 0.0 {
        paint_slot(ui, lead, Rect::from_min_size(pos2(rect.min.x + PAD, rect.min.y), vec2(lead_w, rect.height())));
    }
    ui.painter().text(
        pos2(rect.min.x + PAD + lead_w, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        regular(TEXT_PT),
        if dim { t.text_secondary } else { t.text },
    );
    let mut out = None;
    let mut x = rect.max.x - PAD_RIGHT;
    for (i, (icon, tip)) in buttons.iter().enumerate().rev() {
        let r = Rect::from_min_size(pos2(x - 24.0, rect.center().y - 12.0), vec2(24.0, 24.0));
        let resp = ui.interact(r, ui.scope_id().with(("hdr", title, i)), Sense::click()).tip(*tip);
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), r.shrink(6.0), *icon, t.icon);
        if resp.clicked() {
            out = Some(i);
        }
        x -= 28.0;
    }
    // An empty section's whole header adds its item, not only its `+` button.
    if out.is_none() && dim && resp.clicked() {
        out = buttons.iter().position(|(icon, _)| *icon == Icon::Plus);
    }
    (out, resp)
}

/// Options for [`section`].
pub struct SectionOpts<'a> {
    /// The section's title.
    pub title: &'a str,
    /// A leading slot before the title — frame29's marker, or an icon.
    pub lead: Slot<'a>,
    /// Trailing icon buttons; the clicked one's index is returned.
    pub buttons: &'a [(Icon, &'a str)],
    /// Greys the title (an empty section).
    pub dim: bool,
}

/// A section: the frame29-style header — an optional leading marker, the title and trailing icon
/// buttons — and, when `body` is given, its content below. The general form behind
/// [`section_header`], for "a title and the rows it owns" (the same style in both side panels).
/// Returns which header button was clicked.
pub fn section(ui: &mut Ui, opts: SectionOpts, body: impl FnOnce(&mut Ui)) -> Option<usize> {
    let (clicked, _) = section_header_impl(ui, opts.title, opts.lead, opts.buttons, opts.dim);
    body(ui);
    clicked
}

/// Runs `add` as the group of buttons at a row's right end: two 24 pt buttons 4 pt apart, the last
/// of them ending at the section's 8 pt margin, whatever the row has before it. (The columns around
/// them keep the 8 pt gutter.)
pub fn tight_end<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let group = 24.0 * 2.0 + 4.0;
    let avail = ui.available_rect_before_wrap();
    let rect = Rect::from_min_max(pos2((avail.max.x - group).max(avail.min.x), avail.min.y), avail.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 4.0;
    // The row already gave the group its height (`block` allocated it): the cursor moves on only
    // past the group's width, so the next row keeps the row's own height.
    add(&mut child)
}

/// Options for [`icon_button_opts`].
pub struct IconButtonOpts<'a> {
    pub icon: Icon,
    /// The button's square, in points.
    pub size: f32,
    /// Whether it shows the selected look (an open option, a pressed toggle).
    pub active: bool,
    pub tip: &'a str,
}

/// An icon button: hover, pressed and selected backgrounds, the glyph at its native size, a
/// tooltip and (when `active`) a pressed toggle for screen readers. **The one implementation**
/// behind [`icon_button`], [`icon_button_at`] and [`crate::icons::button`].
pub fn icon_button_opts(ui: &mut Ui, opts: IconButtonOpts) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(opts.size, opts.size), Sense::click());
    paint_icon_button(ui, rect, resp, opts)
}

/// Paint an icon button whose rect and response the caller made: the shared body of every icon
/// button.
fn paint_icon_button(ui: &mut Ui, rect: Rect, resp: Response, opts: IconButtonOpts) -> Response {
    let t = Tokens::get(ui.ctx());
    let bg = if opts.active {
        t.selected_row
    } else if resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    let col = if opts.active { t.accent_text } else { t.icon };
    // The glyph at its native 16 px in the usual buttons, smaller in compact ones.
    let glyph = if opts.size >= 22.0 { 16.0 } else { 12.0 };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(glyph, glyph)), opts.icon, col);
    if opts.tip.is_empty() {
        return resp;
    }
    let resp = crate::a11y::Tip::tip(resp, opts.tip);
    // Shown pressed (an open option): a pressed toggle to screen readers.
    if opts.active {
        crate::a11y::toggle(&resp, opts.tip, true);
    }
    resp
}

/// An icon button at `rect` (hover highlight, tooltip); true when clicked.
pub fn icon_button_at(ui: &mut Ui, id: Id, rect: Rect, icon: Icon, tip: &str) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    paint_icon_button(ui, rect, resp, IconButtonOpts { icon, size: rect.width(), active: false, tip }).clicked()
}

/// A 24×24 icon button laid out in place; true when clicked.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> bool {
    icon_button_opts(ui, IconButtonOpts { icon, size: 24.0, active: false, tip }).clicked()
}

/// A non-interactive `icon` of `size` px (for slots and inline use): allocates its square and
/// returns the response so it lays out like any widget. For a clickable icon use [`icon_button`].
pub fn icon(ui: &mut Ui, icon: Icon, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let t = Tokens::get(ui.ctx());
    icons::paint(ui.painter(), rect, icon, t.icon_secondary);
    resp
}

/// A bar floating at the top centre of the `canvas` area (a version preview, the focus view):
/// `add` lays out its contents in a row.
pub fn canvas_banner<R>(ctx: &egui::Context, id: Id, canvas: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ctx);
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(pos2(canvas.center().x, canvas.min.y + 12.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(8))
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin { left: 14, right: 8, top: 8, bottom: 8 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        add(ui)
                    })
                    .inner
                })
                .inner
        })
        .inner
}

/// A vertical scroll area that never scrolls by dragging its contents: its rows are dragged
/// themselves (layers, pages, fills, effects, styles). With drag-to-scroll on (egui's default
/// where a touch screen is reported) a row drag also scrolls the list, so the rows move under the
/// pointer and its head and tail can't be reached — the wheel and the scroll bar still scroll.
pub fn scroll_area() -> egui::ScrollArea {
    use egui::containers::scroll_area::{DragScroll, ScrollSource};
    egui::ScrollArea::vertical().scroll_source(ScrollSource { drag: DragScroll::Never, ..Default::default() })
}

/// Where a dragged row will land: an accent line across `x0..x1` at `y`.
pub fn insertion_line(painter: &egui::Painter, x0: f32, x1: f32, y: f32) {
    painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(2.0, crate::theme::ACCENT));
}

/// What a drag carries, following the pointer at `at`: `galley` in a small outlined box above
/// everything else, with the grabbing hand.
pub fn drag_ghost(ctx: &egui::Context, at: egui::Pos2, galley: std::sync::Arc<egui::Galley>) {
    let t = Tokens::get(ctx);
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("drag_ghost")));
    let r = Rect::from_min_size(at + vec2(14.0, 10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(r, CornerRadius::same(5), t.panel);
    painter.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), StrokeKind::Inside);
    painter.galley(r.min + vec2(8.0, 4.0), galley, t.text);
}

pub fn separator(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, t.border));
}

/// Paint a checkerboard (transparency) in `rect`.
pub fn checker(painter: &egui::Painter, rect: Rect, cell: f32) {
    checker_rounded(painter, rect, cell, 0);
}

/// [`checker`] whose corners round by `radius`, so a swatch's transparent background stays inside
/// the swatch's rounded border.
fn checker_rounded(painter: &egui::Painter, rect: Rect, cell: f32, radius: u8) {
    painter.rect_filled(rect, CornerRadius::same(radius), Color32::WHITE);
    let nx = (rect.width() / cell).ceil() as i32;
    let ny = (rect.height() / cell).ceil() as i32;
    for y in 0..ny {
        for x in 0..nx {
            if (x + y) % 2 == 0 {
                continue;
            }
            let r = Rect::from_min_size(pos2(rect.min.x + x as f32 * cell, rect.min.y + y as f32 * cell), vec2(cell, cell)).intersect(rect);
            // The corner cells round their outer corner alone.
            let cr = CornerRadius {
                nw: if x == 0 && y == 0 { radius } else { 0 },
                ne: if x == nx - 1 && y == 0 { radius } else { 0 },
                sw: if x == 0 && y == ny - 1 { radius } else { 0 },
                se: if x == nx - 1 && y == ny - 1 { radius } else { 0 },
            };
            painter.rect_filled(r, cr, Color32::from_gray(0xE6));
        }
    }
}

/// A paint swatch (with checkerboard under transparent colours).
pub fn swatch(ui: &mut Ui, paint: &figcraft_color::Paint, size: f32) -> Response {
    swatch_named(ui, paint, size, tr("Color"))
}

/// [`swatch`] with the screen reader's name for the field it belongs to, so a panel's swatches
/// are told apart from every other colour control (and from each other).
pub fn swatch_named(ui: &mut Ui, paint: &figcraft_color::Paint, size: f32, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    // The value is only spelled out while a screen reader reads the tree.
    let value = if a11y::active(&resp.ctx) { paint_name(paint) } else { String::new() };
    a11y::color(&resp, label, &value);
    paint_swatch(ui.painter(), rect, paint);
    resp
}

/// A paint as a screen reader says it: "#FF0000", "#FF0000 50%", "Gradient", "Image", "Pattern".
pub fn paint_name(paint: &figcraft_color::Paint) -> String {
    use figcraft_color::PaintKind;
    match &paint.kind {
        PaintKind::Solid { color } => {
            let hex = color.to_hex();
            let alpha = color.a * paint.opacity;
            if alpha < 0.999 { format!("{hex} {}%", (alpha * 100.0).round()) } else { hex }
        }
        PaintKind::Gradient(_) => tr("Gradient").into(),
        PaintKind::Image(_) => tr("Image").into(),
        PaintKind::Pattern(_) => tr("Pattern").into(),
    }
}

pub fn paint_swatch(painter: &egui::Painter, rect: Rect, paint: &figcraft_color::Paint) {
    let t = Tokens::get(painter.ctx());
    use figcraft_color::PaintKind;
    let clip = painter.with_clip_rect(rect);
    // The swatch's own 2 pt rounding: every fill wears it, or the colour pokes out past the
    // rounded border at the four corners.
    let round = CornerRadius::same(2);
    let left_round = CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 };
    let right_round = CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 };
    checker_rounded(&clip, rect, rect.width() / 4.0, 2);
    match &paint.kind {
        PaintKind::Solid { color } => {
            // Left half opaque, right half with alpha (like Figma's swatches).
            let c = crate::theme::doc_c32(painter.ctx(), color.with_alpha(color.a * paint.opacity));
            let opaque = crate::theme::doc_c32(painter.ctx(), color.with_alpha(1.0));
            clip.rect_filled(Rect::from_min_max(rect.min, pos2(rect.center().x, rect.max.y)), left_round, opaque);
            clip.rect_filled(Rect::from_min_max(pos2(rect.center().x, rect.min.y), rect.max), right_round, c);
            if paint.opacity >= 0.999 && color.a >= 0.999 {
                clip.rect_filled(rect, round, opaque);
            }
        }
        PaintKind::Gradient(g) => {
            let n = 16;
            for i in 0..n {
                let c = crate::theme::doc_c32(painter.ctx(), g.sample(i as f32 / (n - 1) as f32));
                let x0 = rect.min.x + rect.width() * i as f32 / n as f32;
                let x1 = rect.min.x + rect.width() * (i + 1) as f32 / n as f32;
                // Only the ends round their outer corners; the seam between segments stays square.
                let cr = if i == 0 {
                    left_round
                } else if i == n - 1 {
                    right_round
                } else {
                    CornerRadius::ZERO
                };
                clip.rect_filled(Rect::from_min_max(pos2(x0, rect.min.y), pos2(x1, rect.max.y)), cr, c);
            }
        }
        PaintKind::Image(_) | PaintKind::Pattern(_) => {
            let icon = if matches!(paint.kind, PaintKind::Image(_)) { Icon::Image } else { Icon::ViewGrid };
            clip.rect_filled(rect, round, Color32::from_gray(0xB3));
            icons::paint(&clip, rect.shrink(2.0), icon, Color32::WHITE);
        }
    }
    painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

/// A colour row's value slot: an editable hex, or a label (a style, variable, gradient or image).
pub enum ColorText<'a> {
    Hex(&'a str),
    Label(&'a str),
}

/// What a [`color_field`]'s user did.
pub struct ColorField {
    /// The dot in the lead slot: click it to open the picker.
    pub dot: Response,
    /// A committed hex (the value slot was editable).
    pub hex: Option<String>,
    /// The value slot's response, for anchoring a menu (a label row).
    pub label: Option<Response>,
    /// The opacity edit (live while scrubbing).
    pub opacity: Option<FieldEdit>,
}

/// A colour row, as the design's `Field-color`: the 24 pt slot with a 14 pt dot, the value, and
/// the opacity at the row's end (52 pt). The caller draws its own buttons after it.
pub fn color_field(ui: &mut Ui, id: Id, paint: &figcraft_color::Paint, text: ColorText<'_>, width: f32, opacity_pct: f64) -> ColorField {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    field_shell(ui, rect, false, ui.rect_contains_pointer(rect));
    // The dot in its 24 pt slot: a solid paint as the rounded square, anything else as a swatch.
    let slot = Rect::from_min_size(rect.min, vec2(LEAD_ICON, FIELD_H));
    let dot = ui.interact(slot, id.with("dot"), Sense::click());
    match paint.solid_color() {
        Some(c) => paint_slot(ui, Slot::Dot(crate::theme::doc_c32(ui.ctx(), c.with_alpha(paint.opacity))), slot),
        None => paint_swatch(ui.painter(), slot.shrink(5.0), paint),
    }
    // The value: an editable hex, or a label the caller opens a menu from.
    let op_w = 52.0;
    let value_r = Rect::from_min_max(pos2(slot.max.x, rect.min.y), pos2(rect.max.x - op_w, rect.max.y));
    let mut out = ColorField { dot, hex: None, label: None, opacity: None };
    match text {
        ColorText::Hex(v) => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(value_r));
            out.hex = text_field_inline(&mut child, id.with("hex"), v, value_r.shrink2(vec2(0.0, 4.0)), regular(11.0));
        }
        ColorText::Label(v) => {
            ui.painter().text(pos2(value_r.min.x, rect.center().y), Align2::LEFT_CENTER, v, regular(11.0), t.text);
            out.label = Some(ui.interact(value_r, id.with("label"), Sense::click()));
        }
    }
    // The opacity at the end, on the same field (its own background is the row's).
    let op_r = Rect::from_min_max(pos2(rect.max.x - op_w, rect.min.y), rect.max);
    ui.painter().line_segment([pos2(op_r.min.x, rect.min.y + 4.0), pos2(op_r.min.x, rect.max.y - 4.0)], Stroke::new(1.0, t.panel));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(op_r));
    out.opacity = number_field(&mut child, id.with("op"), Lead::None, Some(opacity_pct.round()), "%", op_w, 1.0);
    out
}

/// A small checkbox with label.
pub fn checkbox(ui: &mut Ui, on: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(200.0), FIELD_H), Sense::click());
    a11y::checkbox(&resp, label, on);
    let b = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 7.0), vec2(14.0, 14.0));
    if on {
        ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
        icons::paint(ui.painter(), b.shrink(1.5), Icon::Check, t.accent_on);
    } else {
        ui.painter().rect_stroke(b, CornerRadius::same(3), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    }
    ui.painter().text(pos2(b.max.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(TEXT_PT), t.text);
    resp.clicked()
}

/// A radio button with label; returns whether it was clicked.
pub fn radio(ui: &mut Ui, on: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(240.0), FIELD_H), Sense::click());
    a11y::radio(&resp, label, on);
    let c = pos2(rect.min.x + 7.0, rect.center().y);
    if on {
        ui.painter().circle_filled(c, 7.0, t.accent);
        ui.painter().circle_filled(c, 2.5, t.accent_on);
    } else {
        ui.painter().circle_stroke(c, 6.5, Stroke::new(1.0, if resp.hovered() { crate::theme::ACCENT } else { t.border_strong }));
    }
    ui.painter().text(pos2(rect.min.x + 22.0, rect.center().y), Align2::LEFT_CENTER, label, regular(TEXT_PT), t.text);
    resp.clicked()
}

/// A text button: `primary` is filled with the accent colour, otherwise outlined. A disabled
/// button is grey with dim text (still readable) and never reports clicks.
pub fn button(ui: &mut Ui, label: &str, primary: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(TEXT_PT), t.text);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, FIELD_H), sense);
    a11y::button(&resp, label);
    let (fill, text) = match (enabled, primary, resp.hovered()) {
        (false, true, _) => (t.field, t.text_tertiary),
        (false, false, _) => (Color32::TRANSPARENT, t.text_tertiary),
        (true, true, false) => (t.accent, t.accent_on),
        (true, true, true) => (t.accent_hover, t.accent_on),
        (true, false, false) => (Color32::TRANSPARENT, t.text),
        (true, false, true) => (t.hover, t.text),
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    if !primary {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(TEXT_PT), text);
    resp
}

/// Text in a row.
pub fn label(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text).font(regular(TEXT_PT)).color(color));
}

/// Text shown in `rect` that turns into an editor on click (or when its editor has focus, e.g.
/// after a double-click rename). Returns the new text on Enter / focus loss.
pub fn text_field_inline(ui: &mut Ui, id: Id, value: &str, rect: Rect, font: egui::FontId) -> Option<String> {
    cancel_retargeted_field_save(ui, id);
    let t = Tokens::get(ui.ctx());
    let te_id = id.with("te");
    let buf_id = id.with("buf");
    let focused = ui.memory(|m| m.has_focus(te_id));
    let mut buf: Option<String> = ui.data(|d| d.get_temp(buf_id));
    let resp = ui.interact(rect, id.with("click"), Sense::click());
    a11y::text(&resp, tr("Name"), value);
    let started = buf.is_none() && (focused || resp.clicked());
    if started {
        buf = Some(value.to_string());
        ui.memory_mut(|m| m.request_focus(te_id));
        select_all_next(ui, te_id, value);
    }
    match buf {
        Some(mut text) => {
            ui.painter().rect_filled(rect.expand(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(4), Stroke::new(1.0, crate::theme::ACCENT), StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            let r = crate::bidi::single_line(&mut child, te_id, &mut text, font, rect.width());
            a11y::own_focus(&r);
            keep_focus(&r, started);
            crate::shortcuts::release_field_for_save(&r);
            // A rename or a hex edit starts with the whole text selected.
            select_all_on_focus(ui, &r, &text);
            // The Enter that started the edit (from the keyboard) doesn't end it.
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if esc {
                a11y::spend_escape(ui);
                ui.data_mut(|d| d.remove::<String>(buf_id));
                ui.memory_mut(|m| m.surrender_focus(te_id));
                None
            } else if r.lost_focus() || enter {
                ui.data_mut(|d| d.remove::<String>(buf_id));
                ui.memory_mut(|m| m.surrender_focus(te_id));
                (text != value && !text.trim().is_empty()).then_some(text)
            } else {
                ui.data_mut(|d| d.insert_temp(buf_id, text));
                None
            }
        }
        None => {
            if resp.hovered() {
                ui.painter().rect_filled(rect.expand(2.0), CornerRadius::same(4), t.hover);
            }
            crate::bidi::paint_name(ui.painter(), rect.left_center(), value, font, t.text, rect.width());
            None
        }
    }
}

/// [`text_field_inline`] for a rename in progress: it takes the keyboard, with the whole name
/// selected, when `start`, and the rename is over once it lets go. Returns the new name once
/// committed and whether the rename goes on.
pub fn rename_field(ui: &mut Ui, id: Id, value: &str, rect: Rect, font: egui::FontId, start: bool) -> (Option<String>, bool) {
    let te_id = id.with("te");
    if start {
        ui.data_mut(|d| d.remove::<String>(id.with("buf")));
        ui.memory_mut(|m| m.request_focus(te_id));
    }
    let out = text_field_inline(ui, id, value, rect, font);
    let open = out.is_none() && ui.memory(|m| m.has_focus(te_id));
    (out, open)
}

/// Tab (`Some(false)`) or Shift+Tab (`Some(true)`) pressed this frame, taken from the input and
/// from egui's focus traversal, for a field that moves on to the next item itself.
pub fn take_tab(ui: &Ui) -> Option<bool> {
    let back = ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
            Some(true)
        } else {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab).then_some(false)
        }
    })?;
    ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    Some(back)
}

/// A short secondary note (an empty list's hint) that wraps inside the panel's side padding.
pub fn note(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(PAD as i8, 6)).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(text).font(regular(TEXT_PT)).color(color)).wrap());
    });
}

/// The width of a settings row's control column.
pub const SETTING_CONTROL: f32 = 120.0;

/// Draws a row's two trailing icon buttons 4 points apart (a control keeps 8), returning the
/// second's response. The gap after a widget is fixed when it is laid out, so the tighter spacing
/// has to be in place before the first icon; the row's own spacing comes back afterwards.
pub fn icon_pair(ui: &mut Ui, first: impl FnOnce(&mut Ui), second: impl FnOnce(&mut Ui) -> Response) -> Response {
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.x = 4.0;
    first(ui);
    let out = second(ui);
    ui.spacing_mut().item_spacing = spacing;
    out
}

/// A row in a settings popover: its label on the left, as wide as the row leaves, and one control
/// on the right, [`SETTING_CONTROL`] points wide. Rows stack vertically; this is the horizontal
/// part only.
pub fn setting_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let control = Rect::from_min_size(pos2(rect.max.x - SETTING_CONTROL, rect.center().y - 12.0), vec2(SETTING_CONTROL, 24.0));
    let shown = elide_font(ui, label, (control.min.x - rect.min.x - 8.0).max(0.0), regular(11.0));
    ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, shown, regular(11.0), t.text_secondary);
    let mut child = ui.new_child(UiBuilder::new().max_rect(control).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing = vec2(8.0, 0.0);
    add(&mut child);
}

/// A small grey label above a row of fields ("Position", "Dimensions"…). With two labels the
/// second starts at the second field column.
pub fn sublabels(ui: &mut Ui, a: &str, b: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    // The grid's caption row, above a field row's 32 pt track: a 10 pt caption, 12 pt tall.
    let (r, _) = ui.allocate_exact_size(vec2(w, 12.0), Sense::hover());
    let col = col_width(w);
    // Each label fits its column on one line (long translations end with an ellipsis, in full on hover).
    let a_w = if b.is_some() { col } else { w - PAD - PAD_RIGHT };
    for (i, (text, x, max_w)) in [(a, PAD, a_w)].into_iter().chain(b.map(|b| (b, PAD + col + GUTTER, w - col - GUTTER - PAD - PAD_RIGHT))).enumerate()
    {
        let shown = elide_font(ui, text, max_w, regular(CAPTION_PT));
        let at = ui.painter().text(pos2(r.min.x + x, r.max.y), Align2::LEFT_BOTTOM, &shown, regular(CAPTION_PT), t.text_secondary);
        if shown != text {
            ui.interact(at, ui.scope_id().with(("sublabel", i)), Sense::hover()).on_hover_text(text);
        }
    }
}

/// Width of one of the two field columns in a panel `w` wide (leaving the icon column).
pub fn col_width(w: f32) -> f32 {
    ((w - PAD - PAD_RIGHT - GUTTER * 2.0 - 24.0) / 2.0).max(40.0)
}
