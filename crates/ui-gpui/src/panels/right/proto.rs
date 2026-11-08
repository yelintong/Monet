//! Prototype 标签（egui `ui-egui/src/panels/proto_panel.rs` 的 GPUI 版）。
//!
//! 三段结构照抄 egui `prototype()`：
//! - **无选中**：`Flow starting points`（起点列表）+ `Prototype settings`（设备 / 旋转 / 机身色 /
//!   自定义尺寸 / Responsive / 背景）。
//! - **有选中**：`Flow starting point`（仅顶层 frame）+ `Interactions` + `Scroll behavior`。
//!
//! 全部走冻结通用控件（`PanelSection` / `field_row*` / `Dropdown` / `IconButton` / `swatch`）；
//! 动态行文本用 `RightFields::row_field` 的编辑器池，没有新建任何控件。
//!
//! 图标差异（冻结 `Icon` 集没有 egui 的 `Play` / `Preview`）：play 用 `ChevronRight`、
//! preview 用 `Eye`、连接字形用 `LinkOn`，语义一一对应。

use super::data::{FlowRow, InteractionRow, PanelData, ProtoData, ScrollData};
use super::sections::{icon_cmd, icon_row, pick_dropdown, slot};
use crate::panels::{RightFields, RowEditCtx};
use crate::widgets::{
    Icon, IconButton, PanelSection, SectionHeaderVariant, field_row, field_row_full, field_row_full_trailing, pt, rgba, sublabel_row,
    sublabel_row2, swatch, tokens,
};
use gpui::{App, Div, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};

/// Prototype 标签整页（`right_panel` 在 `right_tab == 1` 时调用）。
pub(crate) fn panel(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Div {
    let p = &d.proto;
    let mut body = div().flex().flex_col().w_full();
    if d.header.is_none() {
        // ── 无选中：flows + settings（egui `prototype()` 前半段）──
        body = body.child(flows_section(f, p, cx, n)).child(crate::panels::sep()).child(settings_section(f, p, cx, n));
        return body;
    }
    // ── 有选中：flow（顶层 frame 才有）+ interactions + scroll ──
    if let Some(mine) = &p.frame_flow {
        body = body.child(flow_point_section(f, mine.as_ref(), cx, n)).child(crate::panels::sep());
    }
    body = body.child(interactions_section(f, p, cx, n));
    if let Some(scroll) = &p.scroll {
        body = body.child(crate::panels::sep()).child(scroll_section(f, scroll));
    }
    body
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Flow starting points（无选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn flows_section(f: &RightFields, p: &ProtoData, cx: &mut App, n: &mut usize) -> PanelSection {
    let empty = p.flows.is_empty();
    let plus = icon_cmd(Icon::Plus, "proto.flow.add", json!({}), f, false);
    let mut sec = PanelSection::new("Flow starting points")
        .variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![plus]));
    if empty {
        return sec.child(field_row_full(note("Select a frame and add a starting point to present it as a flow.")));
    }
    for flow in &p.flows {
        sec = sec.child(field_row_full(flow_line(f, flow, cx, n)));
        if !flow.description.trim().is_empty() {
            sec = sec.child(field_row_full(desc_line(&flow.description)));
        }
    }
    sec
}

/// 一条 flow 行：play / 名字（可编辑）/ preview / remove（egui `flow_row`）。
fn flow_line(f: &RightFields, flow: &FlowRow, cx: &mut App, n: &mut usize) -> Div {
    let play = icon_cmd(Icon::ChevronRight, "proto.present", json!({ "start": flow.start }), f, false);
    let preview = icon_cmd(Icon::Eye, "proto.preview", json!({ "start": flow.start }), f, false);
    let remove = icon_cmd(Icon::Minus, "proto.flow.remove", json!({ "start": flow.start }), f, false);
    let edit = f.row_field(
        cx,
        slot(n),
        &flow.name,
        RowEditCtx {
            cmd: "proto.flow.rename",
            params: json!({ "start": flow.start }),
            key: "name",
            scale: 1.0,
            string_value: true,
            paint: None,
            nest: None, nest_key: None, paint_color: None,
        },
    );
    div().flex().flex_row().items_center().gap(pt(6.0)).w_full().child(play).child(div().flex_1().min_w(px(0.0)).child(edit)).child(preview).child(remove)
}

/// flow 的描述（名字下面缩进一行的次要文字）。
fn desc_line(text: &str) -> Div {
    let t = tokens();
    div().w_full().pl(pt(30.0)).text_size(pt(10.5)).text_color(rgba(t.text_secondary)).child(SharedString::from(text.to_string()))
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Prototype settings（无选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn settings_section(f: &RightFields, p: &ProtoData, cx: &mut App, n: &mut usize) -> PanelSection {
    let s = &p.settings;
    let preview = icon_cmd(Icon::Eye, "proto.preview", json!({}), f, false);
    let mut sec = PanelSection::new("Prototype settings").header_trail(icon_row(vec![preview]));

    // ── Device + 旋转（只有会转的设备才给旋转按钮）──
    sec = sec.child(sublabel_row("Device"));
    let devices: Vec<(String, &'static str, Value)> =
        s.devices.iter().map(|d| (d.to_string(), "proto.settings", json!({ "device": d }))).collect();
    let device_dd = pick_dropdown(f, s.device.clone(), 0.0, devices);
    let mut trail: Vec<IconButton> = Vec::new();
    if s.rotates {
        let next = if s.landscape { "portrait" } else { "landscape" };
        trail.push(icon_cmd(Icon::Rotate, "proto.settings", json!({ "rotation": next }), f, s.landscape));
    }
    sec = sec.child(field_row(vec![device_dd.into_any_element()], trail));

    // ── 机身色 ──
    if !s.colors.is_empty() {
        sec = sec.child(sublabel_row("Color"));
        let rows: Vec<(String, &'static str, Value)> =
            s.colors.iter().map(|(name, _)| (name.to_string(), "proto.settings", json!({ "color": name }))).collect();
        let rgb = s.colors.iter().find(|(name, _)| *name == s.color).map(|(_, c)| *c).unwrap_or([0, 0, 0]);
        let dot = swatch(figcraft_color::Paint::solid(figcraft_color::Color::from_rgb8_const(rgb[0], rgb[1], rgb[2])), 16.0);
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(dot).child(pick_dropdown(f, s.color.clone(), 0.0, rows));
        sec = sec.child(field_row_full(line));
    }

    // ── 自定义尺寸（W/H + 交换）──
    if let Some((w, h)) = s.custom {
        sec = sec.child(sublabel_row("Size"));
        let wf = f.row_field(cx, slot(n), &num(w), RowEditCtx { cmd: "proto.settings", params: json!({}), key: "width", scale: 1.0, string_value: false, paint: None, nest: None, nest_key: None, paint_color: None });
        let hf = f.row_field(cx, slot(n), &num(h), RowEditCtx { cmd: "proto.settings", params: json!({}), key: "height", scale: 1.0, string_value: false, paint: None, nest: None, nest_key: None, paint_color: None });
        let swap = icon_cmd(Icon::Rotate, "proto.settings", json!({ "width": h, "height": w }), f, false);
        sec = sec.child(field_row(vec![wf.into_any_element(), hf.into_any_element()], vec![swap]));
    }

    // ── Responsive ──
    let resp = icon_cmd(Icon::Check, "proto.settings", json!({ "responsive": !s.responsive }), f, s.responsive);
    sec = sec.child(field_row(vec![plain_label("Responsive")], vec![resp]));

    // ── Background（色块 + hex 输入）──
    sec = sec.child(sublabel_row("Background"));
    let bg = &s.background;
    let color = figcraft_color::Color::from_hex_or_name(bg).unwrap_or(figcraft_color::Color::from_rgb8_const(0x1E, 0x1E, 0x1E));
    let hex = f.row_field(
        cx,
        slot(n),
        bg,
        RowEditCtx {
            cmd: "proto.settings",
            params: json!({}),
            key: "background",
            scale: 1.0,
            string_value: true,
            paint: None,
            nest: None, nest_key: None, paint_color: None,
        },
    );
    let dot = swatch(figcraft_color::Paint::solid(color), 16.0);
    let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(dot).child(hex);
    sec.child(field_row_full(line))
}

fn num(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Flow starting point（有选中、且是顶层 frame）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn flow_point_section(f: &RightFields, mine: Option<&FlowRow>, cx: &mut App, n: &mut usize) -> PanelSection {
    let is_start = mine.is_some();
    let mut buttons: Vec<IconButton> = Vec::new();
    if !is_start {
        // egui：还没成为起点时给「Preview this frame」+「Add starting point」。
        buttons.push(icon_cmd(Icon::Eye, "proto.preview", json!({}), f, false));
        buttons.push(icon_cmd(Icon::Plus, "proto.flow.add", json!({}), f, false));
    }
    let mut sec = PanelSection::new("Flow starting point")
        .variant(if is_start { SectionHeaderVariant::Normal } else { SectionHeaderVariant::Dim })
        .header_trail(icon_row(buttons));
    if let Some(flow) = mine {
        sec = sec.child(field_row_full(flow_line(f, flow, cx, n)));
        // 描述输入框（egui `flow_section` 第二行）。
        let desc = f.row_field(
            cx,
            slot(n),
            &flow.description,
            RowEditCtx {
                cmd: "proto.flow.rename",
                params: json!({ "start": flow.start }),
                key: "description",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None, nest_key: None, paint_color: None,
            },
        );
        sec = sec.child(field_row_full(desc));
    }
    sec
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Interactions
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn interactions_section(f: &RightFields, p: &ProtoData, cx: &mut App, n: &mut usize) -> PanelSection {
    let Some(list) = p.interactions.as_ref() else {
        // 多选且交互不一致（egui `mixed_interactions`）：「+」把全部换成一条点击导航，
        // 「Remove all」清空。
        let replace = json!([
            { "command": "proto.interaction.clear", "params": {} },
            { "command": "proto.interaction.add", "params": { "trigger": { "type": "onClick" }, "action": default_navigate(p) } },
        ]);
        let plus = icon_cmd(Icon::Plus, "command.batch", json!({ "label": "Replace interactions", "commands": replace }), f, false);
        let mut sec = PanelSection::new("Interactions").header_trail(icon_row(vec![plus]));
        sec = sec.child(field_row_full(note("Mixed interactions: click + to replace them")));
        let remove = icon_cmd(Icon::Minus, "proto.interaction.clear", json!({}), f, false);
        return sec.child(field_row(vec![plain_label("Remove all")], vec![remove]));
    };
    let empty = list.is_empty();
    // 「+」= 加一条默认点击导航（egui `action_of("Navigate to")`，没有别的 frame 时退回 Back）。
    let plus = icon_cmd(
        Icon::Plus,
        "proto.interaction.add",
        json!({ "trigger": { "type": "onClick" }, "action": default_navigate(p) }),
        f,
        false,
    );
    let mut sec = PanelSection::new("Interactions")
        .variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![plus]));
    if empty {
        return sec.child(field_row_full(note("Add an interaction to make this layer react.")));
    }
    for (i, it) in list.iter().enumerate() {
        let remove = icon_cmd(Icon::Minus, "proto.interaction.remove", json!({ "index": i }), f, false);
        sec = sec.child(interaction_line(f, it, i, remove));
        if *f.proto_open.borrow() == Some(i) {
            for row in editor_rows(f, it, i, cx, n) {
                sec = sec.child(row);
            }
        }
    }
    sec
}

/// 展开的交互编辑器（egui `proto_panel.rs::editor_popover` 的 GPUI 就地版）：
/// 触发器 / 延迟 / 转场 / 时长 / 缓动 + 动作列表。
fn editor_rows(f: &RightFields, it: &InteractionRow, i: usize, cx: &mut App, n: &mut usize) -> Vec<crate::widgets::FieldRow> {
    let mut out = Vec::new();
    // ── Trigger ──
    out.push(sublabel_row("Trigger"));
    let triggers: Vec<(&'static str, &'static str, Value)> = vec![
        ("On click", "onClick", json!({ "type": "onClick" })),
        ("While hovering", "onHover", json!({ "type": "onHover" })),
        ("While pressing", "onPress", json!({ "type": "onPress" })),
        ("On drag", "onDrag", json!({ "type": "onDrag" })),
        ("Mouse enter", "mouseEnter", json!({ "type": "mouseEnter" })),
        ("Mouse leave", "mouseLeave", json!({ "type": "mouseLeave" })),
        ("Mouse down", "mouseDown", json!({ "type": "mouseDown" })),
        ("Mouse up", "mouseUp", json!({ "type": "mouseUp" })),
        ("After delay", "afterDelay", json!({ "type": "afterDelay", "ms": 300 })),
        ("Key / Gamepad", "keyPress", json!({ "type": "keyPress", "key": "Space" })),
    ];
    let rows: Vec<(String, &'static str, Value)> =
        triggers.iter().map(|(label, _, v)| (label.to_string(), "proto.interaction.set", json!({ "index": i, "trigger": v }))).collect();
    out.push(field_row_full(pick_dropdown(f, it.trigger.clone(), 0.0, rows)));

    // ── After delay 的毫秒 / 鼠标手势后的延迟 ──
    let needs_ms = it.trigger_kind == "afterDelay";
    let gesture_delay = matches!(it.trigger_kind.as_str(), "mouseEnter" | "mouseLeave" | "mouseDown" | "mouseUp");
    if needs_ms {
        let field = f.row_field(
            cx,
            slot(n),
            &format!("{}", ms_of(&it.trigger_json)),
            RowEditCtx {
                cmd: "proto.interaction.set",
                params: json!({ "index": i, "trigger": it.trigger_json.clone() }),
                key: "ms",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: Some("trigger"),
                nest_key: None,
                paint_color: None,
            },
        );
        out.push(sublabel_row("Delay (ms)"));
        out.push(field_row_full(field));
    } else if gesture_delay {
        let field = f.row_field(
            cx,
            slot(n),
            &format!("{}", it.delay_ms),
            RowEditCtx {
                cmd: "proto.interaction.set",
                params: json!({ "index": i }),
                key: "delayMs",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        out.push(sublabel_row("Delay (ms)"));
        out.push(field_row_full(field));
    }

    // ── Actions：每条一行（摘要 + 删除），另有「+ Navigate to」加一条 ──
    out.push(sublabel_row(if it.actions.len() > 1 { "Actions" } else { "Action" }));
    for (k, (label, action)) in it.actions.iter().enumerate() {
        // 删掉第 k 条：把剩下的整组写回（`proto.interaction.set {actions}`）。
        let mut rest: Vec<Value> = it.actions.iter().map(|(_, a)| a.clone()).collect();
        rest.remove(k);
        let remove = icon_cmd(Icon::Minus, "proto.interaction.set", json!({ "index": i, "actions": rest }), f, false);
        let line = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .w_full()
            .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(tokens().text)).child(SharedString::from(label.clone())));
        out.push(field_row_full_trailing(line, vec![remove]));
    }
    // 追加一条导航（目标 = 本 frame 之外的第一个 frame）。
    let add_params = match it.actions.first().and_then(|(_, a)| a.get("dest")).cloned() {
        Some(dest) => json!({ "index": i, "action": { "type": "navigate", "dest": dest } }),
        None => json!({ "index": i, "action": { "type": "back" } }),
    };
    let add = icon_cmd(Icon::Plus, "proto.interaction.set", add_params, f, false);
    out.push(field_row_full_trailing(plain_label("Add action"), vec![add]));

    // ── Transition / Easing / Duration ──
    out.push(sublabel_row("Transition"));
    let transitions: [(&'static str, Value); 8] = [
        ("Instant", json!({ "type": "instant" })),
        ("Dissolve", json!({ "type": "dissolve" })),
        ("Smart animate", json!({ "type": "smartAnimate" })),
        ("Move in", json!({ "type": "moveIn", "direction": "left" })),
        ("Move out", json!({ "type": "moveOut", "direction": "left" })),
        ("Push", json!({ "type": "push", "direction": "left" })),
        ("Slide in", json!({ "type": "slideIn", "direction": "left" })),
        ("Slide out", json!({ "type": "slideOut", "direction": "left" })),
    ];
    let trows: Vec<(String, &'static str, Value)> =
        transitions.iter().map(|(l, v)| (l.to_string(), "proto.interaction.set", json!({ "index": i, "transition": v }))).collect();
    out.push(field_row_full(pick_dropdown(f, it.transition.clone(), 0.0, trows)));

    out.push(sublabel_row("Easing"));
    let easings = ["Linear", "Ease in", "Ease out", "Ease in and out", "Ease in back", "Ease out back", "Ease in and out back", "Gentle", "Quick", "Bouncy", "Slow"];
    let erows: Vec<(String, &'static str, Value)> = easings
        .iter()
        .zip([
            json!("linear"),
            json!("easeIn"),
            json!("easeOut"),
            json!("easeInOut"),
            json!("easeInBack"),
            json!("easeOutBack"),
            json!("easeInOutBack"),
            json!("gentle"),
            json!("quick"),
            json!("bouncy"),
            json!("slow"),
        ])
        .map(|(l, v)| (l.to_string(), "proto.interaction.set", json!({ "index": i, "easing": v })))
        .collect();
    out.push(field_row_full(pick_dropdown(f, it.easing.clone(), 0.0, erows)));

    out.push(sublabel_row("Duration (ms)"));
    let dur = f.row_field(
        cx,
        slot(n),
        &format!("{}", it.duration_ms),
        RowEditCtx {
            cmd: "proto.interaction.set",
            params: json!({ "index": i }),
            key: "durationMs",
            scale: 1.0,
            string_value: false,
            paint: None,
            nest: None,
            nest_key: None,
            paint_color: None,
        },
    );
    out.push(field_row_full(dur));
    out
}

/// `afterDelay` trigger 里的毫秒。
fn ms_of(trigger: &Value) -> u64 {
    trigger.get("ms").and_then(Value::as_u64).unwrap_or(300)
}

/// 一条交互行：连接字形 + 触发器（半粗）/ 动作摘要（次要）+ 删除；点行展开编辑器。
fn interaction_line(f: &RightFields, it: &InteractionRow, i: usize, remove: IconButton) -> crate::widgets::FieldRow {
    let t = tokens();
    let open = *f.proto_open.borrow() == Some(i);
    let glyph = IconButton::new(Icon::LinkOn).size(20.0);
    // 点文字区域展开 / 收起编辑器（egui 是点开出浮窗，GPUI 就地展开，功能等价）。
    let toggler = f.proto_open.clone();
    let cmds = f.cmds.clone();
    let text = div()
        .id(SharedString::from(format!("proto-it-{i}")))
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .cursor_pointer()
        .rounded(pt(5.0))
        .when(open, |d| d.bg(rgba(t.selected_row)))
        .hover(|s| s.bg(rgba(t.hover)))
        .px(pt(4.0))
        .py(pt(2.0))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
            {
                let mut cur = toggler.borrow_mut();
                *cur = if *cur == Some(i) { None } else { Some(i) };
            }
            cmds.wake();
        })
        .child(div().text_size(pt(11.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(it.trigger.clone())))
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(it.summary.clone())));
    field_row(vec![glyph.into_any_element(), text.into_any_element()], vec![remove])
}

/// 新交互的默认动作：跳到本 frame 之外的第一个 frame，没有就 Back（egui `action_of`）。
fn default_navigate(p: &ProtoData) -> Value {
    let own = p.frame_flow.as_ref().and_then(|_| p.frames.first().map(|(id, _)| *id));
    let dest = p.frames.iter().map(|(id, _)| *id).find(|f| Some(*f) != own).or_else(|| p.frames.first().map(|(id, _)| *id));
    match dest {
        Some(id) => json!({ "type": "navigate", "dest": id }),
        None => json!({ "type": "back" }),
    }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Scroll behavior
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn scroll_section(f: &RightFields, s: &ScrollData) -> PanelSection {
    let mut sec = PanelSection::new("Scroll behavior");
    if s.in_frame {
        let rows: Vec<(String, &'static str, Value)> = [
            ("Scroll with parent", "scrolls"),
            ("Fixed", "fixed"),
            ("Sticky", "sticky"),
        ]
        .into_iter()
        .map(|(l, v)| (l.to_string(), "node.set", json!({ "props": { "scrollPosition": v } })))
        .collect();
        let label = match s.position {
            "fixed" => "Fixed",
            "sticky" => "Sticky",
            _ => "Scroll with parent",
        };
        sec = sec.child(field_row(vec![pick_dropdown(f, label, 0.0, rows).into_any_element()], Vec::<IconButton>::new()));
    }
    if s.is_frame {
        let rows: Vec<(String, &'static str, Value)> = [
            ("No scrolling", "none"),
            ("Horizontal", "horizontal"),
            ("Vertical", "vertical"),
            ("Both directions", "both"),
        ]
        .into_iter()
        .map(|(l, v)| (l.to_string(), "node.set", json!({ "props": { "overflow": v } })))
        .collect();
        let label = match s.overflow {
            "horizontal" => "Horizontal",
            "vertical" => "Vertical",
            "both" => "Both directions",
            _ => "No scrolling",
        };
        sec = sec.child(field_row(vec![pick_dropdown(f, label, 0.0, rows).into_any_element()], Vec::<IconButton>::new()));
    }
    sec
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// 小工具
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 一行次要提示文字（egui `widgets::note`）。
fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

/// 一行普通文字（占满字段列，`Remove all` / `Responsive` 用）。
fn plain_label(text: &'static str) -> Div {
    let t = tokens();
    div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(text))
}
