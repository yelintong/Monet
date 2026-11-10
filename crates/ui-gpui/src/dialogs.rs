//! The **one** dialog framework for the GPUI front-end (A19 in `docs/gpui-egui-parity-plan.md`).
//!
//! `ui-egui` has a separate hand-written window per dialog (`dialogs/export.rs`, `menu_input.rs`,
//! `missing_fonts.rs`, …). Here a dialog is **data**: a title, a message, a list of fields and a
//! list of buttons. Every dialog therefore shares the same chrome, field grid, input controls,
//! dismiss/Escape behaviour and command dispatch — adding a dialog is adding a `spec()`, not a new
//! control.
//!
//! Reused, frozen controls (nothing in `widgets/` is modified):
//! * `widgets::popover::Popover` — the window shell, centred with `Open::At`, with its X button;
//! * `widgets::menu::popup_backdrop` — click-outside dismissal;
//! * `field_row` / `field_row_full` — the unified `1fr | 1fr | 24` grid (no hand-rolled rows);
//! * `text_field` / `number_field`, `tokens()` — inputs and theme;
//! * `panels::CmdQueue` — the same command queue the right panel uses, so the engine sees the
//!   exact same command ids and params as in `ui-egui`.
//!
//! Programmatic calls never open dialogs: commands only *request* UI (`Session::request_ui`), and
//! `sync_ui_requests` maps those requests through [`ui_action`]. Menu items that need input are
//! mapped through [`menu_action`] before the command is executed.
//!
//! No field pretends to be wired: a choice either runs a real engine command on pick
//! (`commands`) or contributes a value to the button's command params (`values`).

use crate::panels::{Cmd, Cmds};
use crate::widgets::menu::popup_backdrop;
use crate::widgets::popover::{Open, popover_window, title_text};
use crate::widgets::{Slot, TextField, field_row, field_row_full, number_field, pt, rgba, text_field, tokens};
use gpui::{
    AnyElement, App, Div, Entity, ImageSource, MouseButton, MouseDownEvent, Pixels, Point, RenderImage, SharedString, Size, Window, div, img,
    prelude::*, px, size,
};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Which dialog to open. The data that came with the request is passed separately to [`spec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A6 — format / scale / destination.
    Export,
    /// A7 — preferences (theme, interface size, panel widths).
    Preferences,
    /// A11 — About / credits.
    About,
    /// A20 — find and replace.
    Find,
    /// A18 — the Don't Save / Cancel / Save question.
    Unsaved,
    /// A8 + A17 — one text value, used by batch rename and menu-input commands.
    Rename,
    /// A10 — colour profile.
    ColorProfile,
    /// A13 — missing fonts.
    MissingFonts,
    /// A12 — import report.
    ImportReport,
    /// A15 — sampled colour.
    SampledColor,
    /// A4 — plugins.
    Plugins,
    /// A23 — save as.
    SaveAs,
    /// A9 — branch.
    Branch,
    /// A14 — updates.
    Updates,
    /// Anything else: a message with a single OK button.
    Message,
}

impl Kind {
    /// The one list of dialogs. `parity_contract`'s capability audit and this module's own
    /// shape test both read it, so a new dialog cannot be added without being audited.
    pub(crate) const ALL: [Kind; 15] = [
        Kind::Export,
        Kind::Preferences,
        Kind::About,
        Kind::Find,
        Kind::Unsaved,
        Kind::Rename,
        Kind::ColorProfile,
        Kind::MissingFonts,
        Kind::ImportReport,
        Kind::SampledColor,
        Kind::Plugins,
        Kind::SaveAs,
        Kind::Branch,
        Kind::Updates,
        Kind::Message,
    ];
}

/// A field of a dialog. `key` is the name the engine command receives in its params object.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Field {
    Text {
        key: &'static str,
        label: &'static str,
        value: String,
    },
    Number {
        key: &'static str,
        label: &'static str,
        value: f64,
    },
    Choice {
        key: &'static str,
        label: &'static str,
        options: Vec<&'static str>,
        /// Optional JSON values sent instead of the label (numeric scale, lower-case format, …).
        values: Option<Vec<Value>>,
        /// Optional engine command per option; picking one runs it immediately.
        commands: Option<Vec<&'static str>>,
        /// Optional params for that command, one per option (`{"language": "zh-Hans"}`).
        /// Without these the command runs with no parameters.
        command_params: Option<Vec<Value>>,
        active: usize,
    },
    /// A read-only line of prose (reports, hints, contributor lists).
    Note {
        text: String,
    },
    /// A read-only bitmap preview. The image is produced by the engine's renderer, never by a
    /// new UI control: `render` puts the project's existing `img(ImageSource::Render(..))`
    /// inside one `field_row_full`.
    Preview {
        label: &'static str,
    },
}

impl Field {
    /// A plain choice whose label is its value.
    fn choice(key: &'static str, label: &'static str, options: Vec<&'static str>, active: usize) -> Self {
        Self::Choice { key, label, options, values: None, commands: None, command_params: None, active }
    }
    /// Give the choice engine values (numbers, ids, …) instead of the labels.
    fn values(mut self, v: Vec<Value>) -> Self {
        if let Self::Choice { values, .. } = &mut self {
            *values = Some(v);
        }
        self
    }
    /// Every option runs a real engine command as soon as it is picked.
    fn commands(mut self, c: Vec<&'static str>) -> Self {
        if let Self::Choice { commands, .. } = &mut self {
            *commands = Some(c);
        }
        self
    }
    /// The params each of those commands takes (one entry per option).
    fn with_params(mut self, p: Vec<Value>) -> Self {
        if let Self::Choice { command_params, .. } = &mut self {
            *command_params = Some(p);
        }
        self
    }
}

/// What a button does. `Submit` sends one engine command, with `base` merged with every field
/// value keyed by its `key`. `Close` only dismisses (Cancel / OK / X / Escape).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Close,
    Submit {
        id: &'static str,
        base: Value,
    },
    /// Send this engine command with no parameters — field values are deliberately not merged.
    /// `view.panelWidths` with an empty object IS the engine's "reset to default" behaviour.
    Run {
        id: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Button {
    pub label: &'static str,
    pub primary: bool,
    pub action: Action,
}

impl Button {
    fn close(label: &'static str) -> Self {
        Self { label, primary: false, action: Action::Close }
    }
    fn primary(label: &'static str, id: &'static str, base: Value) -> Self {
        Self { label, primary: true, action: Action::Submit { id, base } }
    }
    fn submit(label: &'static str, id: &'static str, base: Value) -> Self {
        Self { label, primary: false, action: Action::Submit { id, base } }
    }
    fn run(label: &'static str, id: &'static str) -> Self {
        Self { label, primary: false, action: Action::Run { id } }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spec {
    pub title: String,
    pub message: Option<String>,
    pub fields: Vec<Field>,
    pub buttons: Vec<Button>,
    pub width: f32,
}

impl Spec {
    fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), message: None, fields: Vec::new(), buttons: Vec::new(), width: 360.0 }
    }
    fn message(mut self, text: impl Into<String>) -> Self {
        self.message = Some(text.into());
        self
    }
    fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
    fn field(mut self, f: Field) -> Self {
        self.fields.push(f);
        self
    }
    fn button(mut self, b: Button) -> Self {
        self.buttons.push(b);
        self
    }
    /// Every dialog has a way out, even one built by mistake.
    fn with_cancel(mut self) -> Self {
        if !self.buttons.iter().any(|b| b.action == Action::Close) {
            self.buttons.push(Button::close("Cancel"));
        }
        self
    }
}

/// Merge a command's static params with `{ key: value }` for every field. Field values win over
/// `base` so a dialog can override a default; non-object `base` is treated as `{}`.
pub(crate) fn merge_params(base: &Value, values: &[(String, Value)]) -> Value {
    let mut out = match base {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    for (key, value) in values {
        out.insert(key.clone(), value.clone());
    }
    Value::Object(out)
}

/// Engine `ui_requests` (and a few menu commands that need input) → a dialog kind.
/// Kept a pure function so every mapping has a regression test and no UI state is touched here.
pub(crate) fn ui_action(kind: &str, _data: &Value) -> Option<Kind> {
    Some(match kind {
        "dialog.preferences" | "app.preferences" => Kind::Preferences,
        "dialog.about" | "help.about" => Kind::About,
        "dialog.plugins" | "plugins.manage" => Kind::Plugins,
        "dialog.unsaved" => Kind::Unsaved,
        "dialog.colorProfile" => Kind::ColorProfile,
        "dialog.missingFonts" => Kind::MissingFonts,
        "dialog.importReport" | "dialog.import" => Kind::ImportReport,
        "dialog.createFromColor" | "dialog.sampledColor" => Kind::SampledColor,
        "dialog.saveAs" => Kind::SaveAs,
        "dialog.branch" => Kind::Branch,
        "dialog.updates" => Kind::Updates,
        "dialog.export" => Kind::Export,
        "dialog.find" | "edit.findReplace" => Kind::Find,
        "dialog.batchRename" => Kind::Rename,
        // The three "…" commands in the menu catalog need an input before they can run.
        "file.export" | "file.exportFramesPdf" | "file.exportSvg" => Kind::Export,
        "layers.rename" | "canvas.renameLabel" | "board.editText" => Kind::Rename,
        "dialog.message" | "dialog.notice" => Kind::Message,
        // An unknown future `dialog.*` request is still shown as a notice rather than swallowed.
        _ if kind.starts_with("dialog.") => Kind::Message,
        _ => return None,
    })
}

/// Menu commands that open a dialog **instead of** being executed (they need parameters first).
/// Executing e.g. `file.export` with `{}` would export with defaults and never show a UI.
pub(crate) fn menu_action(cmd: &str) -> Option<Kind> {
    match cmd {
        // `app.preferences` / `help.about` / `plugins.manage` raise a `dialog.*` request themselves.
        "edit.findReplace" => Some(Kind::Find),
        "file.export" | "file.exportFramesPdf" | "file.exportSvg" => Some(Kind::Export),
        _ => None,
    }
}

/// Build the dialog's data. One arm per dialog; no per-dialog rendering code anywhere else.
pub(crate) fn spec(kind: Kind, data: &Value) -> Spec {
    let str_field = |key: &str, d: &Value, default: &str| -> String { d.get(key).and_then(Value::as_str).unwrap_or(default).to_string() };
    match kind {
        Kind::Export => Spec::new("Export")
            .width(400.0)
            .message("Export the current selection. Choose a format and a scale, then pick Export.")
            .field(
                Field::choice("format", "Format", vec!["PNG", "JPG", "SVG", "PDF", "FIG"], 0)
                    .values(vec![json!("png"), json!("jpg"), json!("svg"), json!("pdf"), json!("fig")]),
            )
            .field(
                Field::choice("scale", "Scale", vec!["1x", "2x", "3x", "0.5x"], 0)
                    .values(vec![json!(1.0), json!(2.0), json!(3.0), json!(0.5)]),
            )
            .field(Field::Text { key: "path", label: "File name", value: str_field("path", data, "") })
            .field(Field::Preview { label: "Nothing to preview: select a layer first." })
            .button(Button::primary("Export", "file.export", json!({})))
            .with_cancel(),
        Kind::Preferences => Spec::new("Preferences")
            .width(420.0)
            .message("Theme and interface size take effect immediately. Panel widths are applied on Apply.")
            .field(
                Field::choice("theme", "Theme", vec!["Dark", "Light", "System"], 0)
                    .commands(vec!["view.themeDark", "view.themeLight", "view.themeSystem"]),
            )
            .field(
                Field::choice("uiScale", "Interface size", vec!["Default", "Larger", "Smaller"], 0)
                    .commands(vec!["pref.uiScaleReset", "pref.uiScaleUp", "pref.uiScaleDown"]),
            )
            .field(
                Field::choice("language", "Language", vec!["System", "English", "简体中文", "日本語", "Deutsch", "Français"], 0)
                    .commands(vec!["pref.language"; 6])
                    .with_params(vec![
                        json!({"language": "system"}),
                        json!({"language": "en"}),
                        json!({"language": "zh-Hans"}),
                        json!({"language": "ja"}),
                        json!({"language": "de"}),
                        json!({"language": "fr"}),
                    ]),
            )
            .field(
                Field::choice("wheel", "Scroll wheel", vec!["Zoom", "Scroll"], 0)
                    .commands(vec!["pref.scrollWheelZoom", "pref.scrollWheelZoom"])
                    .with_params(vec![json!({"on": true}), json!({"on": false})]),
            )
            .field(Field::Number { key: "left", label: "Left panel width", value: 280.0 })
            .field(Field::Number { key: "right", label: "Right panel width", value: 280.0 })
            .button(Button::primary("Apply", "view.panelWidths", json!({})))
            .button(Button::run("Reset widths", "view.panelWidths"))
            .with_cancel(),
        Kind::About => Spec::new("About FigCraft")
            .width(420.0)
            .message(concat!(
                "Interface design and prototyping. A free, open-source, clean-room design tool ",
                "written from scratch in Rust, part of the ArtCraft Crafting Apps."
            ))
            .field(Field::Note { text: format!("Version {}", env!("CARGO_PKG_VERSION")) })
            .field(Field::Note { text: "Dual-licensed MIT OR Apache-2.0.".to_string() })
            .field(Field::Note { text: "About ▸ Contributors are compiled from contributors/contributors.json.".to_string() })
            .button(Button::close("Close")),
        Kind::Find => Spec::new("Find and replace")
            .width(400.0)
            .field(Field::Text { key: "find", label: "Find", value: String::new() })
            .field(Field::Text { key: "replace", label: "Replace with", value: String::new() })
            .button(Button::primary("Replace all", "edit.findReplace", json!({})))
            .button(Button::submit("Find next", "edit.findReplace", json!({ "next": true })))
            .with_cancel(),
        Kind::Unsaved => {
            let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
            Spec::new("Unsaved changes")
                .width(380.0)
                .message(format!("Document {index} has unsaved changes. Save before closing?"))
                .button(Button::submit("Don't Save", "file.close", json!({ "index": index, "force": true })))
                .button(Button::close("Cancel"))
                // `file.save` 保存的是当前活动文档；sync_ui_requests 在弹出本对话框前
                // 已经 `doc.switch` 到被关闭的标签，所以这里不需要（引擎也不接受）index。
                .button(Button::primary("Save", "file.save", json!({})))
        }
        Kind::Rename => Spec::new("Rename")
            .width(360.0)
            .field(Field::Text { key: "name", label: "Name", value: str_field("name", data, "") })
            .button(Button::primary("Rename", "object.rename", json!({})))
            .with_cancel(),
        Kind::ColorProfile => Spec::new("Colour profile")
            .width(380.0)
            .message("Choose the colour space used for display and export.")
            .field(
                Field::choice("profile", "Profile", vec!["sRGB", "Display P3"], 0)
                    .values(vec![json!("srgb"), json!("displayP3")]),
            )
            .button(Button::primary("Apply", "file.colorProfile", json!({})))
            .with_cancel(),
        Kind::MissingFonts => Spec::new("Missing fonts")
            .width(400.0)
            .message("This file uses fonts that are not installed. Font substitution has no engine command yet, so this is a notice rather than a fake control.")
            .field(Field::Note { text: str_field("font", data, "Unknown font") })
            .button(Button::close("Close")),
        Kind::ImportReport => Spec::new("Import report")
            .width(440.0)
            .message(str_field("message", data, "The file was imported. Review the warnings below."))
            .field(Field::Note { text: str_field("warnings", data, "No warnings.") })
            .button(Button::close("Close")),
        Kind::SampledColor => Spec::new("Sampled colour")
            .width(360.0)
            .message("The eyedropper sampled this colour. Creating a style from it is not wired yet.")
            .field(Field::Note { text: str_field("color", data, "#000000") })
            .button(Button::close("Close")),
        Kind::Plugins => Spec::new("Plugins")
            .width(440.0)
            .message("Browse and manage plugins. Use the Actions palette to run one.")
            .field(Field::Text { key: "query", label: "Search", value: String::new() })
            .field(Field::Note { text: "Installed plugins appear in the Actions palette under Plugins & widgets.".to_string() })
            .button(Button::close("Close")),
        Kind::SaveAs => Spec::new("Save as")
            .width(400.0)
            .field(Field::Text { key: "path", label: "File name", value: str_field("path", data, "") })
            .button(Button::primary("Save", "file.saveAs", json!({})))
            .with_cancel(),
        Kind::Branch => Spec::new("Branch")
            .width(400.0)
            .message("Create a branch to work on changes separately from the main file.")
            .field(Field::Text { key: "name", label: "Branch name", value: String::new() })
            .button(Button::primary("Create", "branch.create", json!({})))
            .with_cancel(),
        Kind::Updates => Spec::new("Updates")
            .width(400.0)
            .message("The updater needs the craft-update upgrade (C2) before it can check or install. This is a notice until then.")
            .field(Field::Note { text: "No update command is registered by the engine yet.".to_string() })
            .button(Button::close("Close")),
        Kind::Message => {
            let title = str_field("title", data, "FigCraft");
            let message = str_field("message", data, "The command asked the interface to show this message.");
            Spec::new(title).width(360.0).message(message).button(Button::close("OK"))
        }
    }
}

/// A rendered field: the spec plus its retained state. GPUI is a retained tree, so an input's
/// `Entity` must be created once (in [`DialogState::new`]) and reused every frame.
#[derive(Clone)]
struct Rendered {
    field: Field,
    input: Option<Entity<TextField>>,
    active: Rc<RefCell<usize>>,
}

/// What a button needs to read the fields at click time (not a stale snapshot from render time).
#[derive(Clone)]
enum Source {
    Text(&'static str, Entity<TextField>),
    Number(&'static str, Entity<TextField>),
    Choice(&'static str, Vec<&'static str>, Option<Vec<Value>>, Rc<RefCell<usize>>),
    Note,
}

fn collect_sources(sources: &[Source], cx: &App) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for s in sources {
        match s {
            Source::Text(key, e) => out.push((key.to_string(), Value::String(e.read(cx).value(cx)))),
            Source::Number(key, e) => {
                let raw = e.read(cx).value(cx);
                let value = raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
                out.push((key.to_string(), value.map(|v| json!(v)).unwrap_or(Value::Null)));
            }
            Source::Choice(key, options, values, active) => {
                let i = *active.borrow();
                let value = match values {
                    Some(v) => v.get(i).cloned().unwrap_or(Value::Null),
                    None => Value::String(options.get(i).copied().unwrap_or("").to_string()),
                };
                out.push((key.to_string(), value));
            }
            Source::Note => {}
        }
    }
    out
}

/// One open dialog. The root view holds `Option<DialogState>` and rebuilds only the element tree.
pub(crate) struct DialogState {
    spec: Spec,
    rendered: Vec<Rendered>,
    cmds: Cmds,
    dismiss: Rc<RefCell<bool>>,
    /// Produced by `figcraft_render` in `CanvasView::open_dialog`; `None` shows the fallback label.
    preview: Option<Arc<RenderImage>>,
}

impl DialogState {
    pub(crate) fn new(kind: Kind, data: &Value, cmds: &Cmds, cx: &mut App, preview: Option<Arc<RenderImage>>) -> Self {
        let spec = spec(kind, data);
        let mut rendered = Vec::with_capacity(spec.fields.len());
        for f in &spec.fields {
            let (input, active) = match f {
                Field::Text { value, label, .. } => {
                    (Some(text_field(cx, 200.0, value, label, Slot::None, Slot::None)), Rc::new(RefCell::new(0usize)))
                }
                Field::Number { value, .. } => (Some(number_field(cx, 200.0, Some(*value), "", 1.0, Slot::None)), Rc::new(RefCell::new(0usize))),
                Field::Choice { active, .. } => (None, Rc::new(RefCell::new(*active))),
                Field::Note { .. } | Field::Preview { .. } => (None, Rc::new(RefCell::new(0usize))),
            };
            rendered.push(Rendered { field: f.clone(), input, active });
        }
        Self { spec, rendered, cmds: cmds.clone(), dismiss: Rc::new(RefCell::new(false)), preview }
    }

    /// The root view polls this once per frame (same mechanism as `ExtraState::dismiss`).
    pub(crate) fn take_dismiss(&self) -> bool {
        let was = *self.dismiss.borrow();
        *self.dismiss.borrow_mut() = false;
        was
    }

    fn close_fn(&self) -> Rc<dyn Fn(&mut Window, &mut App)> {
        let flag = self.dismiss.clone();
        let cmds = self.cmds.clone();
        Rc::new(move |_window, _cx| {
            *flag.borrow_mut() = true;
            cmds.wake();
        })
    }

    fn sources(&self) -> Vec<Source> {
        self.rendered
            .iter()
            .map(|r| match &r.field {
                Field::Text { key, .. } => r.input.clone().map(|e| Source::Text(key, e)).unwrap_or(Source::Note),
                Field::Number { key, .. } => r.input.clone().map(|e| Source::Number(key, e)).unwrap_or(Source::Note),
                Field::Choice { key, options, values, .. } => Source::Choice(key, options.clone(), values.clone(), r.active.clone()),
                Field::Note { .. } | Field::Preview { .. } => Source::Note,
            })
            .collect()
    }

    /// Backdrop + centred window. Everything is built from the frozen shared controls.
    pub(crate) fn render(&self, viewport: Size<Pixels>) -> Vec<AnyElement> {
        let t = tokens();
        let close = self.close_fn();
        let sources = self.sources();
        let mut body = div().w_full().flex().flex_col().gap(pt(8.0));

        if let Some(msg) = &self.spec.message {
            body = body.child(div().w_full().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(msg.clone())));
        }

        for r in &self.rendered {
            let label_text = match &r.field {
                Field::Text { label, .. } | Field::Number { label, .. } | Field::Choice { label, .. } => *label,
                Field::Preview { label } => *label,
                Field::Note { .. } => "",
            };
            let label = || {
                div()
                    .h(pt(24.0))
                    .flex()
                    .items_center()
                    .text_size(pt(9.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(label_text))
                    .into_any_element()
            };
            let row = match &r.field {
                Field::Text { .. } | Field::Number { .. } => {
                    let control = r.input.clone().map(|e| e.into_any_element()).unwrap_or_else(|| div().into_any_element());
                    field_row(vec![label(), control], Vec::<AnyElement>::new())
                }
                Field::Choice { options, commands, command_params, .. } => {
                    let chips = choice_chips(options, commands.as_deref(), command_params.as_deref(), r.active.clone(), self.cmds.clone());
                    field_row(vec![label(), chips.into_any_element()], Vec::<AnyElement>::new())
                }
                Field::Preview { label } => {
                    field_row_full(div().w_full().h(pt(112.0)).flex().items_center().justify_center().bg(rgba(t.field)).rounded(pt(6.0)).child(
                        match &self.preview {
                            Some(image) => img(ImageSource::Render(image.clone())).w(px(160.0)).h(px(100.0)).into_any_element(),
                            None => div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(*label).into_any_element(),
                        },
                    ))
                }
                Field::Note { text } => field_row_full(
                    div()
                        .w_full()
                        .h(pt(24.0))
                        .flex()
                        .items_center()
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .child(SharedString::from(text.clone())),
                ),
            };
            body = body.child(row);
        }

        // Two buttons per grid row; a last odd button takes a full row. No hand-rolled row.
        for pair in self.spec.buttons.chunks(2) {
            let mut els: Vec<AnyElement> = Vec::with_capacity(pair.len());
            for b in pair {
                let flag = self.dismiss.clone();
                let cmds_btn = self.cmds.clone();
                let sources_btn = sources.clone();
                let action = b.action.clone();
                els.push(
                    dialog_button(b.label, b.primary, move |_: &MouseDownEvent, _, cx: &mut App| {
                        match &action {
                            Action::Submit { id, base } => {
                                let params = merge_params(base, &collect_sources(&sources_btn, cx));
                                cmds_btn.push(Cmd { id, params: Some(params) });
                            }
                            Action::Run { id } => cmds_btn.push(Cmd { id, params: None }),
                            Action::Close => {}
                        }
                        *flag.borrow_mut() = true;
                        cmds_btn.wake();
                    })
                    .into_any_element(),
                );
            }
            let row = if els.len() == 2 {
                field_row(els, Vec::<AnyElement>::new())
            } else {
                field_row_full(els.pop().unwrap_or_else(|| div().into_any_element()))
            };
            body = body.child(row);
        }

        let preview_extra = self.spec.fields.iter().filter(|f| matches!(f, Field::Preview { .. })).count() as f32 * 80.0;
        let height = 40.0
            + 32.0
            + self.spec.message.as_ref().map(|_| 32.0).unwrap_or(0.0)
            + self.spec.fields.len() as f32 * 32.0
            + preview_extra
            + self.spec.buttons.len().div_ceil(2) as f32 * 32.0;
        let at = Point {
            x: px(((f32::from(viewport.width) - self.spec.width) * 0.5).max(0.0)),
            y: px(((f32::from(viewport.height) - height) * 0.5).max(0.0)),
        };
        let card = popover_window(Open::At(at))
            .width(self.spec.width)
            .height(height)
            .title(title_text(self.spec.title.clone()))
            .body(body)
            .show(size(viewport.width, viewport.height), close.clone());
        vec![popup_backdrop(close), card.1]
    }
}

/// A row of option chips. It lives *inside* one `field_row` cell — the row's columns still belong
/// to the unified grid, this is only the control drawn in the second column. Picking an option
/// that names a command runs it immediately (the generic "a choice is a command" behaviour).
fn choice_chips(
    options: &[&'static str],
    commands: Option<&[&'static str]>,
    params: Option<&[Value]>,
    active: Rc<RefCell<usize>>,
    queue: Cmds,
) -> Div {
    let t = tokens();
    let mut row = div().w_full().flex().flex_row().gap(pt(4.0));
    for (i, option) in options.iter().enumerate() {
        let on = *active.borrow() == i;
        let flag = active.clone();
        let cmds = queue.clone();
        let command = commands.and_then(|c| c.get(i)).copied();
        let command_params = params.and_then(|p| p.get(i)).cloned();
        let chip = div()
            .flex_1()
            .min_w(px(0.0))
            .h(pt(24.0))
            .rounded(pt(5.0))
            .bg(rgba(if on { t.accent } else { t.field }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(pt(9.0))
            .text_color(rgba(if on { t.accent_on } else { t.text }))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                *flag.borrow_mut() = i;
                if let Some(id) = command {
                    cmds.push(Cmd { id, params: command_params.clone() });
                } else {
                    cmds.wake();
                }
            })
            .child(SharedString::from(*option));
        row = row.child(chip);
    }
    row
}

/// A labelled button. `on_click` is the generic mouse-down handler shared by every dialog button.
fn dialog_button(label: &'static str, primary: bool, on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Div {
    let t = tokens();
    div()
        .w_full()
        .h(pt(24.0))
        .rounded(pt(6.0))
        .bg(rgba(if primary { t.accent } else { t.field }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(pt(11.0))
        .text_color(rgba(if primary { t.accent_on } else { t.text }))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, on_click)
        .child(SharedString::from(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_kind_maps_to_a_dialog() {
        let empty = json!({});
        assert_eq!(ui_action("dialog.preferences", &empty), Some(Kind::Preferences));
        assert_eq!(ui_action("app.preferences", &empty), Some(Kind::Preferences));
        assert_eq!(ui_action("dialog.about", &empty), Some(Kind::About));
        assert_eq!(ui_action("dialog.unsaved", &json!({"index": 2})), Some(Kind::Unsaved));
        assert_eq!(ui_action("dialog.missingFonts", &empty), Some(Kind::MissingFonts));
        assert_eq!(ui_action("dialog.createFromColor", &json!({"color": "#ff0000"})), Some(Kind::SampledColor));
        assert_eq!(ui_action("layers.rename", &empty), Some(Kind::Rename));
        assert_eq!(ui_action("edit.findReplace", &empty), Some(Kind::Find));
        assert_eq!(ui_action("dialog.unknownFutureThing", &empty), Some(Kind::Message));
        assert_eq!(ui_action("shape.rectangle", &empty), None, "a normal command is not a dialog");
    }

    #[test]
    fn menu_commands_that_need_input_do_not_execute_directly() {
        assert_eq!(menu_action("file.export"), Some(Kind::Export));
        assert_eq!(menu_action("edit.findReplace"), Some(Kind::Find));
        assert_eq!(menu_action("app.preferences"), None, "the engine raises the request itself");
        assert_eq!(menu_action("view.toggleUi"), None);
    }

    #[test]
    fn specs_share_one_shape_and_one_way_out() {
        let empty = json!({});
        for kind in [
            Kind::Export,
            Kind::Preferences,
            Kind::About,
            Kind::Find,
            Kind::Unsaved,
            Kind::Rename,
            Kind::ColorProfile,
            Kind::MissingFonts,
            Kind::ImportReport,
            Kind::SampledColor,
            Kind::Plugins,
            Kind::SaveAs,
            Kind::Branch,
            Kind::Updates,
            Kind::Message,
        ] {
            let s = spec(kind, &empty);
            assert!(!s.title.is_empty(), "{kind:?} has no title");
            assert!(!s.buttons.is_empty(), "{kind:?} has no buttons");
            assert!(s.buttons.iter().any(|b| b.action == Action::Close), "{kind:?} cannot be dismissed");
            assert!(!s.title.contains("Dev Mode"), "the retired label may not appear in a dialog");
        }
    }

    #[test]
    fn export_and_preferences_carry_real_engine_commands() {
        let export = spec(Kind::Export, &json!({}));
        assert!(export.fields.iter().any(|f| matches!(f, Field::Choice { key: "format", .. })));
        assert!(export.buttons.iter().any(|b| matches!(&b.action, Action::Submit { id: "file.export", .. })));
        // The engine's `file.export` wants a numeric scale, not the "2x" label.
        assert!(export.fields.iter().any(|f| matches!(
            f,
            Field::Choice { key: "scale", values: Some(v), .. } if v.get(1) == Some(&json!(2.0))
        )));
        let prefs = spec(Kind::Preferences, &json!({}));
        assert!(prefs.buttons.iter().any(|b| matches!(&b.action, Action::Submit { id: "view.panelWidths", .. })));
        assert!(prefs.fields.iter().any(|f| matches!(
            f,
            Field::Choice { commands: Some(c), .. } if c.first() == Some(&"view.themeDark")
        )));
        let unsaved = spec(Kind::Unsaved, &json!({"index": 3}));
        assert_eq!(unsaved.buttons.len(), 3, "Don't Save / Cancel / Save");
        assert!(unsaved.buttons.iter().any(|b| b.label == "Cancel" && b.action == Action::Close));
        assert!(unsaved.message.as_deref().unwrap_or("").contains('3'));
    }

    #[test]
    fn params_merge_field_values_over_defaults() {
        let merged =
            merge_params(&json!({"scale": 1, "path": "default.png"}), &[("scale".to_string(), json!(2.0)), ("path".to_string(), json!("out.png"))]);
        assert_eq!(merged["scale"], json!(2.0), "field values win");
        assert_eq!(merged["path"], json!("out.png"));
        assert_eq!(merge_params(&Value::Null, &[]), json!({}), "a non-object base becomes an empty object");
    }
}
