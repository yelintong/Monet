//! Component / Instance 面板（egui `ui-egui/src/panels/component_panel.rs::instance_section`）。
//!
//! Design 标签里，header 之后、Position 之前：
//! - **Instance**：标题（Instance / Nested instance）+ Reset / Detach 按钮、主组件 swap 下拉、
//!   主组件描述、Expose 开关、组件属性（变体值 / 布尔 / 文本 / 实例互换）。
//! - **Component / Component set**：描述编辑。
//!
//! 全部走冻结通用控件与列表行编辑器池，没有新建控件。

use figcraft_i18n::tr;
use super::data::{ComponentData, PanelData};
use super::sections::{icon_cmd, icon_row, pick_dropdown, slot};
use crate::panels::{RightFields, RowEditCtx};
use crate::widgets::{
    Icon, IconButton, PanelSection, SectionHeaderVariant, field_row, field_row_full, pt, rgba, sublabel_row, tokens,
};
use gpui::{App, Div, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};

/// Component / Instance 节（`None` = 选中的不是组件相关节点）。
pub(crate) fn section(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    let c = d.component.as_ref()?;
    if c.is_instance {
        return Some(instance_section(f, c, cx, n));
    }
    if !c.editable_description {
        return None;
    }
    // 组件 / 组件集：只有描述可编辑。
    let mut sec = PanelSection::new(c.title);
    if !c.description.is_empty() || c.editable_description {
        let field = f.row_field(
            cx,
            slot(n),
            &c.description,
            RowEditCtx {
                cmd: "node.set",
                params: json!({}),
                key: "description",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: Some("props"),
                nest_key: None,
                paint_color: None,
            },
        );
        sec = sec.child(field_row_full(field));
    }
    Some(sec)
}

fn instance_section(f: &RightFields, c: &ComponentData, cx: &mut App, n: &mut usize) -> PanelSection {
    let t = tokens();
    let reset = icon_cmd(Icon::Rotate, "component.resetInstance", json!({}), f, false);
    let mut trail = vec![reset];
    if !c.nested {
        trail.push(icon_cmd(Icon::Detach, "component.detach", json!({}), f, false));
    }
    let mut sec = PanelSection::new(SharedString::from(c.title)).header_trail(icon_row(trail));

    // ── 主组件（swap 下拉）+ 「去主组件 / 恢复主组件」──
    let swap_rows: Vec<(String, &'static str, Value)> = c
        .components
        .iter()
        .map(|(id, name)| (name.clone(), "instance.swap", json!({ "component": id })))
        .collect();
    let main_dd = pick_dropdown(f, c.main_name.clone(), 0.0, swap_rows);
    let mut main_trail: Vec<IconButton> = Vec::new();
    if c.missing {
        main_trail.push(icon_cmd(Icon::Component, "component.restoreMain", json!({}), f, true));
    } else {
        main_trail.push(icon_cmd(Icon::Component, "component.goToMain", json!({}), f, false));
    }
    sec = sec.child(field_row(vec![main_dd.into_any_element()], main_trail));

    // ── 主组件描述（次要文字）──
    if !c.description.is_empty() {
        sec = sec.child(field_row_full(
            div()
                .w_full()
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(SharedString::from(c.description.clone())),
        ));
    }

    // ── 嵌在主组件里的实例：把属性暴露给实例 ──
    if c.nested_in_main {
        let expose = icon_cmd(
            Icon::Check,
            "component.exposeInstance",
            json!({ "exposed": !c.exposed }),
            f,
            c.exposed,
        );
        sec = sec.child(field_row(vec![plain_label(tr("Expose properties to instances"))], vec![expose]));
    }

    // ── 组件属性 ──
    if !c.props.is_empty() {
        sec = sec.child(sublabel_row(tr("Properties")));
    }
    for p in &c.props {
        let label = div()
            .w(px(84.0))
            .flex_none()
            .text_size(pt(11.0))
            .text_color(rgba(t.text_secondary))
            .child(SharedString::from(p.name.clone()));
        let control: gpui::AnyElement = match p.kind {
            "boolean" => {
                let on = p.value == "true";
                let btn = icon_cmd(
                    Icon::Check,
                    "instance.setProperties",
                    json!({ "properties": { p.name.clone(): !on } }),
                    f,
                    on,
                );
                div().flex().flex_row().items_center().child(btn).into_any_element()
            }
            "text" => f
                .row_field(
                    cx,
                    slot(n),
                    &p.value,
                    RowEditCtx {
                        cmd: "instance.setProperties",
                        params: json!({}),
                        key: "value",
                        scale: 1.0,
                        string_value: true,
                        paint: None,
                        nest: Some("properties"),
                        nest_key: Some(p.name.clone()),
                        paint_color: None,
                    },
                )
                .into_any_element(),
            "variant" | "swap" => {
                // options 已经是（显示名, 提交值）。
                let rows: Vec<(String, &'static str, Value)> = p
                    .options
                    .iter()
                    .map(|(label, value)| {
                        (label.clone(), "instance.setProperties", json!({ "properties": { p.name.clone(): value.clone() } }))
                    })
                    .collect();
                pick_dropdown(f, p.value.clone(), 0.0, rows).into_any_element()
            }
            // Slot：填充走挑选组件填进去，此轮先显示名字（见报告）。
            _ => div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(tr("Slot"))).into_any_element(),
        };
        let line = div().flex().flex_row().items_center().gap(pt(8.0)).w_full().child(label).child(div().flex_1().min_w(px(0.0)).child(control));
        sec = sec.child(field_row_full(line));
    }
    sec
}

/// 占满字段列的普通文字。
fn plain_label(text: &'static str) -> Div {
    let t = tokens();
    div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(text))
}
