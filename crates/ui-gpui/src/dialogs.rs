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

use crate::CanvasView;
use crate::panels::{Cmd, Cmds};
use crate::widgets::dropdown::dropdown;
use crate::widgets::menu::popup_backdrop;
use crate::widgets::popover::{Open, clamp, popover_window, title_text};
use crate::widgets::{Checkbox, MenuRow, Slot, TextField, field_row, field_row_full, number_field, pt, rgba, text_field, tokens};
use figcraft_i18n::tr;
use gpui::{
    AnyElement, App, Bounds, Div, Entity, ImageSource, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, RenderImage, SharedString, Size,
    Window, div, img, prelude::*, px, size,
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
    /// A menu-input command: `text.link` needs the URL before it can run
    /// (`lizard menus.rs::invoke` → `MenuInput::Link`).
    Link,
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
    pub(crate) const ALL: [Kind; 16] = [
        Kind::Export,
        Kind::Preferences,
        Kind::About,
        Kind::Find,
        Kind::Unsaved,
        Kind::Rename,
        Kind::Link,
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
    /// A section caption with the hairline above it (`ui-egui/dialogs/export.rs`'s `caption`
    /// + `divider`). Data only — the row stays inside the shared `field_row_full`.
    Section {
        title: &'static str,
    },
    /// A clickable preset row (`row_tip` in the egui Export dialog): clicking runs one engine
    /// command with its params and closes the dialog, exactly like the original.
    Action {
        label: &'static str,
        id: &'static str,
        params: Value,
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
    /// Like `Submit`, but merges only the named fields. A "Find" button sends `find` without the
    /// empty `replace` — otherwise the engine would replace every match with nothing.
    SubmitFields {
        id: &'static str,
        base: Value,
        fields: Vec<&'static str>,
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
    /// A submit button that merges only `fields` (a Find button without a Replace field).
    fn submit_only(label: &'static str, id: &'static str, base: Value, fields: Vec<&'static str>) -> Self {
        Self { label, primary: false, action: Action::SubmitFields { id, base, fields } }
    }
    fn primary_only(label: &'static str, id: &'static str, base: Value, fields: Vec<&'static str>) -> Self {
        Self { label, primary: true, action: Action::SubmitFields { id, base, fields } }
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
            self.buttons.push(Button::close(tr("Cancel")));
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
        // R4/R5/R6: these three are handled by `CanvasView::sync_ui_requests` as inline edits —
        // egui never shows a centred dialog for them. Only a real batch rename is a dialog.
        "dialog.batchRename" => Kind::Rename,
        // The three "…" commands in the menu catalog need an input before they can run.
        "file.export" | "file.exportFramesPdf" | "file.exportSvg" => Kind::Export,
        "dialog.textLink" | "text.link" => Kind::Link,
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
        // `text.link` needs a URL; the Text header's Create link button opens the same dialog.
        "text.link" => Some(Kind::Link),
        _ => None,
    }
}

/// Build the dialog's data. One arm per dialog; no per-dialog rendering code anywhere else.
pub(crate) fn spec(kind: Kind, data: &Value) -> Spec {
    let str_field = |key: &str, d: &Value, default: &str| -> String { d.get(key).and_then(Value::as_str).unwrap_or(default).to_string() };
    match kind {
        Kind::Export => {
            // 1:1 with `ui-egui/src/dialogs/export.rs`: caption + divider + eight preset rows,
            // PDF print options and the whole-file row. It is NOT an invented format/scale form.
            let preset = |label, format: &str, scale| Field::Action {
                label,
                id: "file.export",
                params: json!({"format": format, "scale": scale}),
            };
            Spec::new(tr("Export"))
                .width(320.0)
                .field(Field::Section { title: tr("With the layers' export settings") })
                .field(Field::Action { label: tr("Export layers"), id: "file.exportLayers", params: json!({}) })
                .field(Field::Section { title: tr("The selection as one file") })
                .field(preset("PNG @1x", "png", 1.0))
                .field(preset("PNG @2x", "png", 2.0))
                .field(preset("PNG @3x", "png", 3.0))
                .field(preset("JPG @2x", "jpg", 2.0))
                .field(preset("WebP @2x", "webp", 2.0))
                .field(preset("SVG", "svg", 1.0))
                .field(preset("PDF", "pdf", 1.0))
                .field(preset("Vector Drawable", "vectordrawable", 1.0))
                .field(Field::Section { title: tr("PDF print options") })
                .field(Field::Preview { label: tr("Nothing to preview: select a layer first.") })
                .field(Field::Section { title: tr("The whole file") })
                .field(Field::Action { label: tr(".fig (experimental)"), id: "file.export", params: json!({"format": "fig"}) })
                .field(Field::Note { text: tr("Other apps may not open every .fig file exported here.").to_string() })
                .with_cancel()
        }
        Kind::Preferences => {
            // 1:1 with `ui-egui/src/dialogs.rs::preferences`: the same checkbox list, name, server,
            // nudge, interface size, theme and language. `data["prefs"]` carries the live values
            // (injected by `open_dialog`). Accent and the graphics backend have no engine command,
            // so they are not faked here.
            let p = &data["prefs"];
            let on = |k: &str| p[k].as_bool().unwrap_or(false);
            let text = |k: &str| p[k].as_str().unwrap_or("").to_string();
            let toggle = |key: &'static str, label: &'static str, cmd: &'static str, on: bool| {
                Field::choice(key, label, vec!["On", tr("Off")], if on { 0 } else { 1 }).commands(vec![cmd, cmd])
            };
            let theme = text("theme");
            let theme_active = ["light", "dark", "system"].iter().position(|t| theme.eq_ignore_ascii_case(t)).unwrap_or(2);
            let lang = text("language");
            let lang_codes = ["system", "en", "zh-Hans", "ja", "de", "fr"];
            let lang_active = lang_codes.iter().position(|c| lang.eq_ignore_ascii_case(c)).unwrap_or(0);
            Spec::new(tr("Preferences"))
                .width(360.0)
                .field(toggle("keepTool", tr("Keep tool selected after use"), "pref.keepTool", on("keep_tool")))
                .field(toggle("highlightHover", tr("Highlight layers on hover"), "pref.highlightHover", on("highlight_on_hover")))
                .field(toggle("renameDuplicates", tr("Rename duplicated layers"), "pref.renameDuplicates", on("rename_duplicates")))
                .field(toggle("showDimensions", tr("Show dimensions on objects"), "pref.showDimensions", on("show_dimensions")))
                .field(toggle("numberKeysOpacity", tr("Use number keys for opacity"), "pref.numberKeysOpacity", on("number_keys_opacity")))
                .field(toggle("reduceMotion", tr("Reduce motion"), "pref.reduceMotion", on("reduce_motion")))
                .field(toggle("autoSave", tr("Save files automatically"), "pref.autoSave", on("auto_save")))
                .field(toggle("startOnHome", tr("Start on the Home screen"), "pref.startOnHome", on("start_on_home")))
                .field(Field::Text { key: "name", label: tr("Your name in comments"), value: text("user_name") })
                .button(Button::submit_only(tr("Set name"), "pref.userName", json!({}), vec!["name"]))
                .field(Field::Text { key: "url", label: tr("Server for online sharing"), value: text("publish_server") })
                .button(Button::submit_only(tr("Set server"), "pref.publishServer", json!({}), vec!["url"]))
                .button(Button::run(tr("Reset"), "pref.publishServer"))
                .field(Field::Number { key: "small", label: tr("Small nudge"), value: p["nudge_small"].as_f64().unwrap_or(1.0) })
                .field(Field::Number { key: "big", label: tr("Big nudge"), value: p["nudge_big"].as_f64().unwrap_or(10.0) })
                .button(Button::submit_only(tr("Apply nudge"), "pref.nudge", json!({}), vec!["small", "big"]))
                .field(
                    Field::choice("uiScale", tr("Interface size"), vec![tr("Default"), "Larger", "Smaller"], 0)
                        .commands(vec!["pref.uiScaleReset", "pref.uiScaleUp", "pref.uiScaleDown"]),
                )
                .field(
                    Field::choice("theme", tr("Theme"), vec![tr("Light"), tr("Dark"), tr("System")], theme_active)
                        .commands(vec!["view.themeLight", "view.themeDark", "view.themeSystem"]),
                )
                .field(
                    Field::choice("language", tr("Language"), vec![tr("System"), "English", "简体中文", "日本語", "Deutsch", "Français"], lang_active)
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
                .with_cancel()
        }
        Kind::About => Spec::new(tr("About Monet"))
            .width(420.0)
            .message(concat!(
                "Interface design and prototyping. A free, open-source, clean-room design tool ",
                "written from scratch in Rust, part of the ArtCraft Crafting Apps."
            ))
            .field(Field::Note { text: format!("Version {}", env!("CARGO_PKG_VERSION")) })
            .field(Field::Note { text: tr("Dual-licensed MIT OR Apache-2.0.").to_string() })
            .field(Field::Note { text: tr("About ▸ Contributors are compiled from contributors/contributors.json.").to_string() })
            .button(Button::close(tr("Close"))),
        Kind::Find => Spec::new(tr("Find and replace"))
            .width(400.0)
            .field(Field::Text { key: "find", label: tr("Find"), value: String::new() })
            .field(Field::Text { key: "replace", label: tr("Replace with"), value: String::new() })
            // egui: "Find" searches (no `replace`, so nothing is changed) including layer names;
            // "Replace all" replaces in text layers only.
            .button(Button::submit_only(tr("Find"), "edit.findReplace", json!({ "layerNames": true }), vec!["find"]))
            .button(Button::primary_only(tr("Replace all"), "edit.findReplace", json!({ "layerNames": false }), vec!["find", "replace"]))
            .with_cancel(),
        Kind::Unsaved => {
            let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
            Spec::new(tr("Unsaved changes"))
                .width(380.0)
                .message(format!("Document {index} has unsaved changes. Save before closing?"))
                .button(Button::submit(tr("Don't Save"), "file.close", json!({ "index": index, "force": true })))
                .button(Button::close(tr("Cancel")))
                // `file.save` 保存的是当前活动文档；sync_ui_requests 在弹出本对话框前
                // 已经 `doc.switch` 到被关闭的标签，所以这里不需要（引擎也不接受）index。
                .button(Button::primary(tr("Save"), "file.save", json!({})))
        }
        Kind::Rename => Spec::new(tr("Rename"))
            .width(360.0)
            .field(Field::Text { key: "name", label: tr("Name"), value: str_field("name", data, "") })
            // A batch rename carries the ids; `object.rename` needs them explicitly. A menu-driven
            // batch rename has none and falls back to the selection (engine `req_targets`).
            .button(Button::primary(tr("Rename"), "object.rename", json!({ "ids": data.get("ids").cloned().unwrap_or(json!([])) })))
            .with_cancel(),
        // `lizard dialogs.rs::MenuInput::Link`: 一个 URL 输入框，提交时由按钮把它作为
        // `{"url": …}` 交给 `text.link`（引擎只认这个键）。
        Kind::Link => Spec::new(tr("Create link"))
            .width(360.0)
            .field(Field::Text { key: "url", label: tr("URL"), value: str_field("url", data, "https://") })
            .button(Button::primary(tr("Create link"), "text.link", json!({})))
            .with_cancel(),
        Kind::ColorProfile => Spec::new(tr("Color profile"))
            .width(380.0)
            .message(tr("Choose the colour space used for display and export."))
            .field(
                Field::choice("profile", tr("Profile"), vec!["sRGB", "Display P3"], 0)
                    .values(vec![json!("srgb"), json!("displayP3")]),
            )
            .button(Button::primary(tr("Apply"), "file.colorProfile", json!({})))
            .with_cancel(),
        Kind::MissingFonts => Spec::new(tr("Missing fonts"))
            .width(400.0)
            .message(tr("This file uses fonts that are not installed. Font substitution has no engine command yet, so this is a notice rather than a fake control."))
            .field(Field::Note { text: str_field("font", data, "Unknown font") })
            .button(Button::close(tr("Close"))),
        Kind::ImportReport => Spec::new(tr("Import report"))
            .width(440.0)
            .message(str_field("message", data, "The file was imported. Review the warnings below."))
            .field(Field::Note { text: str_field("warnings", data, "No warnings.") })
            .button(Button::close(tr("Close"))),
        Kind::SampledColor => Spec::new(tr("Sampled colour"))
            .width(360.0)
            .message(tr("The eyedropper sampled this colour. Creating a style from it is not wired yet."))
            .field(Field::Note { text: str_field("color", data, "#000000") })
            .button(Button::close(tr("Close"))),
        Kind::Plugins => {
            // 1:1 with `ui-egui/src/dialogs/plugins_ui.rs::installed_tab`: the installed plugins
            // (name + summary + Remove), then "Import plugin from manifest…". The Browse tab fetches
            // a plugin directory over the network (a background task), which is not wired here.
            let list = data["plugins"].as_array().cloned().unwrap_or_default();
            let mut spec = Spec::new(tr("Plugins")).width(440.0).field(Field::Section { title: tr("Installed") });
            if list.is_empty() {
                spec = spec.field(Field::Note {
                    text: tr("No plugins yet. Import a development plugin from its figcraft-plugin.json.").to_string(),
                });
            }
            for p in &list {
                let id = p["id"].as_str().unwrap_or("").to_string();
                let name = p["name"].as_str().unwrap_or("").to_string();
                let version = p["version"].as_str().unwrap_or("").to_string();
                let desc = p["description"].as_str().unwrap_or("").to_string();
                let title = if version.is_empty() { name.clone() } else { format!("{name}  {version}") };
                spec = spec.field(Field::Note { text: title });
                if !desc.is_empty() {
                    spec = spec.field(Field::Note { text: desc });
                }
                spec = spec.field(Field::Action { label: tr("Remove"), id: "plugin.remove", params: json!({ "id": id }) });
            }
            spec.button(Button::run(tr("Import plugin from manifest…"), "plugin.import")).button(Button::close(tr("Close")))
        }
        Kind::SaveAs => Spec::new(tr("Save as"))
            .width(400.0)
            .field(Field::Text { key: "path", label: tr("File name"), value: str_field("path", data, "") })
            .button(Button::primary(tr("Save"), "file.saveAs", json!({})))
            .with_cancel(),
        Kind::Branch => {
            // 1:1 with `ui-egui/src/dialogs/branch_ui.rs`: the branch list (each opens or archives)
            // above the create field. The side-by-side review window is a separate large piece.
            let list = data["branches"].as_array().cloned().unwrap_or_default();
            let mut spec = Spec::new(tr("Branch"))
                .width(440.0)
                .message(tr("Create a branch to work on changes separately from the main file."))
                .field(Field::Text { key: "name", label: tr("Branch name"), value: String::new() })
                .button(Button::primary(tr("Create"), "branch.create", json!({})));
            if !list.is_empty() {
                spec = spec.field(Field::Section { title: tr("Branches") });
                for b in &list {
                    let id = b["id"].as_str().unwrap_or("").to_string();
                    let name = b["name"].as_str().unwrap_or("").to_string();
                    let state = b["state"].as_str().unwrap_or("");
                    let label = if state == "active" || state.is_empty() { name.clone() } else { format!("{name}  ({state})") };
                    spec = spec
                        .field(Field::Note { text: label })
                        .field(Field::Action { label: tr("Open"), id: "branch.open", params: json!({ "id": id }) })
                        .field(Field::Action { label: tr("Archive"), id: "branch.archive", params: json!({ "id": id }) });
                }
            }
            spec.with_cancel()
        }
        Kind::Updates => Spec::new(tr("Update"))
            .width(400.0)
            .message(tr("The updater needs the craft-update upgrade (C2) before it can check or install. This is a notice until then."))
            .field(Field::Note { text: tr("No update command is registered by the engine yet.").to_string() })
            .button(Button::close(tr("Close"))),
        Kind::Message => {
            let title = str_field("title", data, tr("Monet"));
            let message = str_field("message", data, "The command asked the interface to show this message.");
            Spec::new(title).width(360.0).message(message).button(Button::close(tr("Done")))
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
    /// The root view: dropdown menus and the shared drag state machine both live there.
    root: Entity<CanvasView>,
    // 通用「按住拖动」状态：与浮窗、尺寸分隔条共用 `hold_drag`，不再自己写一套。
    viewport: Size<Pixels>,
    place: Option<Point<Pixels>>,
    hold: crate::hold_drag::HoldDrag,
}

fn centered(viewport: Size<Pixels>, width: f32, height: f32) -> Point<Pixels> {
    Point { x: px(((f32::from(viewport.width) - width) * 0.5).max(0.0)), y: px(((f32::from(viewport.height) - height) * 0.5).max(0.0)) }
}

impl DialogState {
    pub(crate) fn new(kind: Kind, data: &Value, cmds: &Cmds, cx: &mut App, preview: Option<Arc<RenderImage>>, root: Entity<CanvasView>) -> Self {
        let spec = spec(kind, data);
        let mut rendered = Vec::with_capacity(spec.fields.len());
        for f in &spec.fields {
            let (input, active) = match f {
                Field::Text { value, label, .. } => {
                    (Some(text_field(cx, 200.0, value, label, Slot::None, Slot::None)), Rc::new(RefCell::new(0usize)))
                }
                Field::Number { value, .. } => (Some(number_field(cx, 200.0, Some(*value), "", 1.0, Slot::None)), Rc::new(RefCell::new(0usize))),
                Field::Choice { active, .. } => (None, Rc::new(RefCell::new(*active))),
                Field::Note { .. } | Field::Preview { .. } | Field::Section { .. } | Field::Action { .. } => (None, Rc::new(RefCell::new(0usize))),
            };
            rendered.push(Rendered { field: f.clone(), input, active });
        }
        Self {
            spec,
            rendered,
            cmds: cmds.clone(),
            dismiss: Rc::new(RefCell::new(false)),
            preview,
            root,
            viewport: size(px(0.0), px(0.0)),
            place: None,
            hold: crate::hold_drag::HoldDrag::default(),
        }
    }

    /// 通用按住拖动：按下 / 3pt 死区 / 相对位移 / 收尾全在 `hold_drag`，
    /// 这里只把算出的绝对位置按视口夹紧并写回 `place`。
    pub(crate) fn press(&mut self, at: Point<Pixels>) {
        let origin = self.place.unwrap_or_else(|| centered(self.viewport, self.spec.width, self.height()));
        self.hold.press(at, origin);
    }
    pub(crate) fn dragging(&self) -> bool {
        self.hold.active()
    }
    pub(crate) fn pending(&self) -> bool {
        self.hold.pending()
    }
    pub(crate) fn end_drag(&mut self) {
        self.hold.end();
    }
    pub(crate) fn route_move(&mut self, at: Point<Pixels>) -> bool {
        let Some(step) = self.hold.advance(at) else { return false };
        let p = clamp(step.place, self.spec.width, self.height(), self.viewport);
        if let Some(old) = self.place
            && (f32::from(p.x) - f32::from(old.x)).abs() < 0.5
            && (f32::from(p.y) - f32::from(old.y)).abs() < 0.5
        {
            return false;
        }
        self.place = Some(p);
        true
    }

    fn height(&self) -> f32 {
        let extra: f32 = self
            .spec
            .fields
            .iter()
            .map(|f| match f {
                Field::Preview { .. } => 80.0,
                Field::Section { .. } => 18.0,
                _ => 0.0,
            })
            .sum();
        40.0 + 32.0
            + self.spec.message.as_ref().map(|_| 32.0).unwrap_or(0.0)
            + self.spec.fields.len() as f32 * 32.0
            + extra
            + self.spec.buttons.len().div_ceil(2) as f32 * 32.0
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
                Field::Note { .. } | Field::Preview { .. } | Field::Section { .. } | Field::Action { .. } => Source::Note,
            })
            .collect()
    }

    /// Backdrop + a **movable** window. Every piece is a shared control: `Popover` (title drag,
    /// X), the `field_row` grid, `dropdown`, `text_field` / `number_field` and `dialog_button`.
    pub(crate) fn render(&mut self, viewport: Size<Pixels>) -> Vec<AnyElement> {
        let t = tokens();
        let close = self.close_fn();
        let sources = self.sources();
        self.viewport = viewport;
        let height = self.height();
        let at = self.place.unwrap_or_else(|| centered(viewport, self.spec.width, height));
        // 项间距统一走通用弹窗的 body 容器（8pt 只定义一次）。
        let mut body = crate::widgets::popover::popover_body();

        if let Some(msg) = &self.spec.message {
            body = body.child(div().w_full().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(msg.clone())));
        }

        for r in &self.rendered {
            let label_text = match &r.field {
                Field::Text { label, .. } | Field::Number { label, .. } | Field::Choice { label, .. } => *label,
                Field::Preview { label } | Field::Section { title: label } | Field::Action { label, .. } => *label,
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
                // A two-option choice is a binary choice: draw the kit Checkbox, not a dropdown
                // (egui uses `widgets::checkbox` for on/off preferences).
                Field::Choice { label, options, commands, command_params, .. } if options.len() == 2 => {
                    let active = r.active.clone();
                    let checked = *active.borrow() == 0;
                    let cmds = self.cmds.clone();
                    let cmd = commands.as_ref().and_then(|c| c.first()).copied();
                    let params = command_params.clone();
                    let cb = Checkbox::new(SharedString::from(format!("dialog-choice-{label}"))).label(*label).checked(checked).on_change(
                        move |&next, _, _| {
                            *active.borrow_mut() = if next { 0 } else { 1 };
                            if let Some(cmd) = cmd {
                                let p = params
                                    .as_ref()
                                    .and_then(|v| if next { v.first() } else { v.get(1) })
                                    .cloned()
                                    .unwrap_or_else(|| serde_json::json!({}));
                                cmds.push(Cmd { id: cmd, params: Some(p) });
                            }
                        },
                    );
                    field_row_full(cb)
                }
                Field::Choice { options, commands, command_params, .. } => {
                    // 1:1 with ui-egui: a real dropdown, not a row of chips.
                    let control = dropdown_field(
                        options,
                        commands.as_deref(),
                        command_params.as_deref(),
                        r.active.clone(),
                        self.root.clone(),
                        self.cmds.clone(),
                    );
                    field_row(vec![label(), control], Vec::<AnyElement>::new())
                }
                Field::Section { title } => field_row_full(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(pt(8.0))
                        .child(div().h(pt(1.0)).w_full().bg(rgba(t.border)))
                        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(*title)),
                ),
                Field::Action { label, id, params } => {
                    let flag = self.dismiss.clone();
                    let cmds = self.cmds.clone();
                    let id = *id;
                    let params = params.clone();
                    field_row_full(
                        div()
                            .w_full()
                            .h(pt(30.0))
                            .pl(pt(10.0))
                            .rounded(pt(5.0))
                            .flex()
                            .items_center()
                            .text_size(pt(12.0))
                            .text_color(rgba(t.text))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(t.hover)))
                            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                                cmds.push(Cmd { id, params: Some(params.clone()) });
                                *flag.borrow_mut() = true;
                                cmds.wake();
                            })
                            .child(*label),
                    )
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
                            Action::SubmitFields { id, base, fields } => {
                                let all = collect_sources(&sources_btn, cx);
                                let keep: Vec<(String, Value)> = all.into_iter().filter(|(k, _)| fields.contains(&k.as_str())).collect();
                                let params = merge_params(base, &keep);
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

        let root_start = self.root.clone();
        let root_move = self.root.clone();
        let card = popover_window(Open::At(at))
            .width(self.spec.width)
            .height(height)
            .title(title_text(self.spec.title.clone()))
            .body(body)
            .on_drag_start(move |e: &MouseDownEvent, _, cx: &mut App| {
                root_start.update(cx, |cv, _| {
                    if let Some(d) = &mut cv.dialog {
                        d.press(e.position);
                    }
                });
            })
            .on_drag_move(move |e: &MouseMoveEvent, _, cx: &mut App| {
                let held = e.pressed_button == Some(MouseButton::Left);
                root_move.update(cx, |cv, cx| {
                    let moved = if !held {
                        if let Some(d) = &mut cv.dialog {
                            d.end_drag();
                            true
                        } else {
                            false
                        }
                    } else if let Some(d) = &mut cv.dialog {
                        d.route_move(e.position)
                    } else {
                        false
                    };
                    if moved {
                        cx.notify();
                    }
                });
            })
            .show(size(viewport.width, viewport.height), close.clone());
        vec![popup_backdrop(close), card.1]
    }
}

/// A real dropdown control inside one `field_row` cell. `CanvasView::open_menu` owns the popup's
/// width / placement; picking an option updates the retained selection and (when the field names
/// engine commands) runs that option's command with its own params. 1:1 with
/// `ui-egui/src/dialogs.rs`'s `widgets::dropdown` + `right::menu_popup`.
fn dropdown_field(
    options: &[&'static str],
    commands: Option<&[&'static str]>,
    params: Option<&[Value]>,
    active: Rc<RefCell<usize>>,
    root: Entity<CanvasView>,
    queue: Cmds,
) -> AnyElement {
    let current = options.get(*active.borrow()).copied().unwrap_or("");
    let opts = options.to_vec();
    let cmd_list = commands.map(|c| c.to_vec());
    let param_list = params.map(|p| p.to_vec());
    dropdown(SharedString::from(current), 0.0, None)
        .fill()
        .on_click(move |anchor: Bounds<Pixels>, _w: &mut Window, cx: &mut App| {
            let rows: Vec<MenuRow> = opts.iter().enumerate().map(|(i, o)| MenuRow::new(*o).checked(Some(i == *active.borrow()))).collect();
            let act = active.clone();
            let opts = opts.clone();
            let cmd_list = cmd_list.clone();
            let param_list = param_list.clone();
            let q = queue.clone();
            let root = root.clone();
            let on_pick = Rc::new(move |picked: &str, _w: &mut Window, _cx: &mut App| {
                let Some(i) = opts.iter().position(|o| *o == picked) else { return };
                *act.borrow_mut() = i;
                if let Some(id) = cmd_list.as_ref().and_then(|c| c.get(i)) {
                    q.push(Cmd { id, params: param_list.as_ref().and_then(|p| p.get(i)).cloned() });
                } else {
                    q.wake();
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, None, rows, on_pick, cx));
        })
        .into_any_element()
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
        // R4/R5/R6: inline edits, not centred dialogs.
        assert_eq!(ui_action("layers.rename", &empty), None);
        assert_eq!(ui_action("canvas.renameLabel", &empty), None);
        assert_eq!(ui_action("board.editText", &empty), None);
        assert_eq!(ui_action("dialog.batchRename", &empty), Some(Kind::Rename));
        assert_eq!(ui_action("dialog.textLink", &empty), Some(Kind::Link));
        assert_eq!(ui_action("text.link", &empty), Some(Kind::Link));
        assert_eq!(ui_action("edit.findReplace", &empty), Some(Kind::Find));
        assert_eq!(ui_action("dialog.unknownFutureThing", &empty), Some(Kind::Message));
        assert_eq!(ui_action("shape.rectangle", &empty), None, "a normal command is not a dialog");
    }

    #[test]
    fn menu_commands_that_need_input_do_not_execute_directly() {
        assert_eq!(menu_action("file.export"), Some(Kind::Export));
        assert_eq!(menu_action("edit.findReplace"), Some(Kind::Find));
        assert_eq!(menu_action("text.link"), Some(Kind::Link), "Create link needs a URL first");
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
            Kind::Link,
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
        // 1:1 with the egui Export dialog: caption sections + eight clickable presets.
        assert!(export.fields.iter().any(|f| matches!(f, Field::Section { .. })));
        let presets: Vec<&str> = export
            .fields
            .iter()
            .filter_map(|f| match f {
                Field::Action { label, .. } => Some(*label),
                _ => None,
            })
            .collect();
        assert!(presets.contains(&"PNG @2x") && presets.contains(&".fig (experimental)"), "{presets:?}");
        assert!(presets.len() >= 8, "all the original presets are listed");
        let prefs = spec(Kind::Preferences, &json!({}));
        assert!(prefs.buttons.iter().any(|b| matches!(&b.action, Action::SubmitFields { id: "pref.nudge", .. })));
        assert!(prefs.fields.iter().any(|f| matches!(
            f,
            Field::Choice { commands: Some(c), .. } if c.contains(&"view.themeDark")
        )));
        assert!(prefs.fields.iter().any(|f| matches!(
            f,
            Field::Choice { commands: Some(c), .. } if c.first() == Some(&"pref.keepTool")
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
