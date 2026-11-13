//! Inspect 标签里的 Dev Mode 各节，照搬 egui `ui-egui/src/panels/dev_mode.rs`：
//! `ready_list`（无选中）/ `categories_section`（无选中）/ `annotations_section`（有选中）/
//! `dev_resources_section`（有选中）/ `measurements_section`（两者都有）。
//!
//! 全部用冻结通用控件（`PanelSection` / `field_row*` / `Dropdown` / `IconButton`），动态行文本
//! 走 `RightFields::row_field` 的编辑器池，没有新建任何控件。
//!
//! 语义差异（已汇报）：
//! - egui 的注释便签是 **2 行 multiline 框**，GPUI 的 `TextEdit` 只有单行 → 单行框。
//! - egui 的分类色是 8 格**色板弹窗**，`MenuRow` 只画文字，子菜单又画不了色块 → 8 个 hex 下拉项。
//! - 图标：measure 用 `Line`，ready 的「在专注视图打开」用 `Frame`，link 用 `LinkOn`。

use super::data::{AnnotationRow, DevResourceRow, InspectRow, MeasurementRow, PanelData, ReadyRow};
use super::inspect::local_dropdown;
use super::sections::{icon_cmd, icon_row, pick_dropdown, slot};
use crate::panels::{Cmd, RightFields, RowEditCtx};
use crate::widgets::{Dropdown, Icon, IconButton, MenuRow, PanelSection, SectionHeaderVariant, field_row, field_row_full, pt, rgba, tokens};
use figcraft_i18n::tr;
use figcraft_i18n::tr_noop;
use gpui::{App, Div, IntoElement, SharedString, div, prelude::*, px};
use serde_json::{Value, json};
use std::rc::Rc;

/// egui `FILTERS`：Ready for dev 列表的四种过滤。
const READY_FILTERS: [&str; 4] = [tr_noop("Ready for dev"), tr_noop("Changed"), tr_noop("Completed"), tr_noop("Any status")];

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Ready for dev（无选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 全文件的界手递手状态列表，按过滤器筛，点一行跳过去（egui `ready_list`）。
pub(crate) fn ready_list(f: &RightFields, d: &PanelData) -> PanelSection {
    // 过滤已在数据层用引擎 `devinspect::dev_items(doc, status, None)` 做掉
    // （与 `inspect.readyForDev {status}` 命令同源），这里只负责画。
    let filter = d.dev.ready_filter.min(3);
    let rows: Vec<&ReadyRow> = d.dev.ready.iter().collect();
    let mut sec =
        PanelSection::new(tr("Ready for dev")).variant(if rows.is_empty() { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    let picks: Vec<(String, usize)> = READY_FILTERS.iter().enumerate().map(|(i, l)| (l.to_string(), i)).collect();
    sec = sec.child(field_row_full(local_dropdown(f, READY_FILTERS[filter.min(3)].into(), 0.0, picks, f.dev_filter.clone())));
    if rows.is_empty() {
        return sec.child(field_row_full(note(tr("Nothing here yet. Set a frame's status to list it."))));
    }
    for (i, r) in rows.into_iter().enumerate() {
        sec = sec.child(field_row_full(ready_line(f, r, i)));
    }
    sec
}

/// 一行：状态点 + 名字 + 页码 +（悬停时）在专注视图打开的按钮。
fn ready_line(f: &RightFields, r: &ReadyRow, i: usize) -> gpui::Stateful<Div> {
    let t = tokens();
    let (cr, cg, cb) = r.color;
    let dot = div().flex_none().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(gpui::rgb(((cr as u32) << 16 | (cg as u32) << 8 | cb as u32).into()));
    let focus = icon_cmd(Icon::Frame, "dev.focus", json!({ "id": r.id }), f, false);
    let cmds = f.cmds.clone();
    let id = r.id;
    div()
        .id(SharedString::from(format!("dev-ready-{i}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .w_full()
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
            cmds.push(Cmd { id: "select.set", params: Some(json!({ "ids": [id] })) });
            cmds.push(Cmd { id: "view.zoomSelection", params: Some(json!({})) });
        })
        .child(dot)
        .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(r.name.clone())))
        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(r.page.clone())))
        .child(focus)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Annotations（无选中：分类清单；有选中：这一层的注解）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 无选中：画布上显示哪些注解（过滤器）+ 文件里的分类（改名 / 换色 / 删除 / 新增）。
pub(crate) fn categories(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> PanelSection {
    let t = tokens();
    let mut sec = PanelSection::new(tr("Annotations")).header_trail(icon_row(vec![icon_cmd(
        Icon::Plus,
        "annotation.addCategory",
        json!({ "name": category_name(&d.dev.categories) }),
        f,
        false,
    )]));

    // ── 画布过滤器：All categories / No category / 每个分类 ──
    let filter = (*f.dev_cat_filter.borrow()).min(d.dev.categories.len() + 1);
    let mut picks: Vec<(String, usize)> = vec![(tr("All categories").into(), 0), (tr("No category").into(), 1)];
    picks.extend(d.dev.categories.iter().enumerate().map(|(i, c)| (c.name.clone(), i + 2)));
    sec = sec.child(field_row_full(local_dropdown(f, picks[filter].0.clone().into(), 0.0, picks, f.dev_cat_filter.clone())));

    if d.dev.categories.is_empty() {
        return sec.child(field_row_full(note("No categories yet. Add one to group annotations.")));
    }
    sec = sec.child(field_row_full(sublabel(tr("Categories"))));
    for c in &d.dev.categories {
        // 色块（画出来的当前色）+ 名字（可编辑）+ 换色下拉 + 删除。
        let dot = match figcraft_color::Color::from_hex(&c.color) {
            Some(col) => crate::widgets::swatch(figcraft_color::Paint::solid(col), 14.0).into_any_element(),
            None => div().flex_none().w(px(14.0)).h(px(14.0)).rounded(px(4.0)).bg(rgba(t.field)).into_any_element(),
        };
        let colors: Vec<(String, &'static str, Value)> = figcraft_doc::AnnotationCategory::COLORS
            .iter()
            .map(|col| (col.to_hex(), "annotation.editCategory", json!({ "id": c.id, "color": col.to_hex() })))
            .collect();
        let pick_color = pick_dropdown(f, c.color.clone(), 76.0, colors);
        let name = f.row_field(
            cx,
            slot(n),
            &c.name,
            RowEditCtx {
                cmd: "annotation.editCategory",
                params: json!({ "id": c.id }),
                key: "name",
                scale: 1.0,
                string_value: true,
                paint: None,
                paint_color: None,
                nest: None,
                nest_key: None,
            },
        );
        let del = icon_cmd(Icon::Minus, "annotation.removeCategory", json!({ "id": c.id }), f, false);
        sec = sec.child(field_row_full(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .w_full()
                .child(dot)
                .child(div().flex_1().min_w(px(0.0)).child(name))
                .child(pick_color)
                .child(del),
        ));
    }
    sec
}

/// 新分类的名字：`Category N`（第一个没被占用的）。
fn category_name(existing: &[super::data::CategoryRow]) -> String {
    (1..=existing.len() + 1).map(|k| format!("Category {k}")).find(|n| !existing.iter().any(|c| &c.name == n)).unwrap_or_default()
}

/// 有选中：这一层的注记，每条就地可改、可换分类、可钉属性（egui `annotations_section`）。
pub(crate) fn annotations(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    let node = d.dev.node?;
    let empty = d.dev.annotations.is_empty();
    let mut sec = PanelSection::new(tr("Annotations")).variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    for a in &d.dev.annotations {
        sec = sec.child(field_row_full(annotation_head(f, a, node)));
        // 便签本体（egui 是 2 行 multiline，GPUI 只有单行框）。
        let edit = f.row_field(
            cx,
            slot(n),
            &a.label,
            RowEditCtx {
                cmd: "annotation.edit",
                params: json!({ "id": node, "index": a.index }),
                key: "label",
                scale: 1.0,
                string_value: true,
                paint: None,
                paint_color: None,
                nest: None,
                nest_key: None,
            },
        );
        edit.update(cx, |te, _| te.set_placeholder(tr("Add a note for developers")));
        sec = sec.child(field_row_full(div().w_full().child(edit)));
        // 钉住的属性（只读，值随图层变）。
        for p in &a.pins {
            sec = sec.child(field_row_full(pin_row(p)));
        }
        // 钉更多 / 取消钉住。
        if !a.pin_menu.is_empty() {
            let rows: Vec<(String, bool, Value)> = a.pin_menu.clone();
            sec = sec.child(field_row_full(checked_dropdown(f, tr("Pin a property"), rows)));
        }
    }
    // 新注解从一条便签开始，属性之后再钉（egui 注释里的同一句）。
    if d.dev.can_add_annotation {
        let edit = f.row_field(
            cx,
            slot(n),
            "",
            RowEditCtx {
                cmd: "annotation.add",
                params: json!({ "id": node }),
                key: "label",
                scale: 1.0,
                string_value: true,
                paint: None,
                paint_color: None,
                nest: None,
                nest_key: None,
            },
        );
        edit.update(cx, |te, _| te.set_placeholder(tr("Add an annotation for developers")));
        sec = sec.child(field_row_full(div().w_full().child(edit)));
    }
    Some(sec)
}

/// 一条注解的抬头：`Note N`（半粗）+ 分类下拉 + 删除。
fn annotation_head(f: &RightFields, a: &AnnotationRow, node: figcraft_doc::NodeId) -> Div {
    let t = tokens();
    let cats: Vec<(String, &'static str, Value)> = a.category_menu.iter().map(|(l, _, p)| (l.clone(), "annotation.edit", p.clone())).collect();
    let label = a.category.as_ref().map(|c| c.0.clone()).unwrap_or_else(|| tr("No category").into());
    let dd = pick_dropdown(f, label, 116.0, cats);
    let del = icon_cmd(Icon::Minus, "annotation.remove", json!({ "id": node, "index": a.index }), f, false);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .w_full()
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(pt(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.text_secondary))
                .child(SharedString::from(format!("Note {}", a.index + 1))),
        )
        .child(dd)
        .child(del)
}

/// 钉住属性的一行：标签 | 值（可带色块）。
fn pin_row(p: &InspectRow) -> Div {
    let t = tokens();
    let mut value = div().flex().flex_row().items_center().gap(pt(6.0)).flex_1().min_w(px(0.0));
    if let Some(hex) = &p.color
        && let Some(c) = figcraft_color::Color::from_hex(hex)
    {
        value = value.child(crate::widgets::swatch(figcraft_color::Paint::solid(c), 12.0));
    }
    value = value.child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(p.value.clone())));
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .w_full()
        .child(div().w(px(88.0)).flex_none().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(p.label.clone())))
        .child(value)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Dev resources（有选中）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 给开发看的链接：自己加的可改可删，继承自主组件的只读并写明来源（egui `dev_resources_section`）。
pub(crate) fn dev_resources(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> Option<PanelSection> {
    let node = d.dev.node?;
    let adding = *f.dev_res_add.borrow();
    let editing = *f.dev_res_edit.borrow();
    let empty = d.dev.dev_resources.is_empty();
    let plus = {
        let flag = f.dev_res_add.clone();
        let cmds = f.cmds.clone();
        IconButton::new(Icon::Plus).size(24.0).active(adding).on_click(move |_e, _w, _cx| {
            {
                let mut cur = flag.borrow_mut();
                *cur = !*cur;
            }
            cmds.wake();
        })
    };
    let mut sec = PanelSection::new(tr("Dev resources"))
        .variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![plus]));

    for r in &d.dev.dev_resources {
        // 正在改这一条：名字 + 链接 + 完成。
        if let (Some(i), Some(e)) = (r.index, editing)
            && i == e
        {
            let name = f.row_field(
                cx,
                slot(n),
                &r.name,
                RowEditCtx {
                    cmd: "devResource.edit",
                    params: json!({ "id": node, "index": i }),
                    key: "name",
                    scale: 1.0,
                    string_value: true,
                    paint: None,
                    paint_color: None,
                    nest: None,
                    nest_key: None,
                },
            );
            let url = f.row_field(
                cx,
                slot(n),
                &r.url,
                RowEditCtx {
                    cmd: "devResource.edit",
                    params: json!({ "id": node, "index": i }),
                    key: "url",
                    scale: 1.0,
                    string_value: true,
                    paint: None,
                    paint_color: None,
                    nest: None,
                    nest_key: None,
                },
            );
            let flag = f.dev_res_edit.clone();
            let cmds = f.cmds.clone();
            let done = IconButton::new(Icon::Check).size(24.0).on_click(move |_e, _w, _cx| {
                *flag.borrow_mut() = None;
                cmds.wake();
            });
            sec = sec.child(field_row_full(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .w_full()
                    .child(div().flex_1().min_w(px(0.0)).child(name))
                    .child(div().flex_1().min_w(px(0.0)).child(url))
                    .child(done),
            ));
            continue;
        }
        sec = sec.child(field_row_full(resource_line(f, r, node)));
    }

    if adding && d.dev.can_add_resource {
        let url = f.row_field(
            cx,
            slot(n),
            "",
            RowEditCtx {
                cmd: "devResource.add",
                params: json!({ "id": node }),
                key: "url",
                scale: 1.0,
                string_value: true,
                paint: None,
                paint_color: None,
                nest: None,
                nest_key: None,
            },
        );
        url.update(cx, |te, _| te.set_placeholder(tr("Paste a link (https://…)")));
        sec = sec.child(field_row_full(div().w_full().child(url)));
    } else if empty && d.dev.can_add_resource {
        sec = sec.child(field_row_full(note(tr("Link docs, code or tickets for developers."))));
    }
    Some(sec)
}

/// 一条资源：链接图标 + 名字（点击打开）+ 来源或网址 +（自己的）改 / 删。
fn resource_line(f: &RightFields, r: &DevResourceRow, node: figcraft_doc::NodeId) -> Div {
    let t = tokens();
    let below = r.from.clone().map(|m| format!("From {m}")).unwrap_or_else(|| r.url.clone());
    let url_open = r.url.clone();
    let link = div()
        .id(SharedString::from(format!("devres-{}", r.index.map(|i| i.to_string()).unwrap_or_else(|| r.url.clone()))))
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .flex_col()
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, cx: &mut App| {
            // 只有网页链接才打开（egui `doc_urls::openable` 同一道闸）。
            if figcraft_engine::doc_urls::openable(&url_open) {
                cx.open_url(&url_open);
            }
        })
        .child(div().text_size(pt(11.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.accent)).child(SharedString::from(r.name.clone())))
        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(below)));
    let mut row = div().flex().flex_row().items_center().gap(pt(6.0)).w_full().child(link);
    if let Some(i) = r.index {
        // 自己的可改可删（继承来的没有这两个按钮）。
        let flag = f.dev_res_edit.clone();
        let cmds = f.cmds.clone();
        let pen = IconButton::new(Icon::Pencil).size(24.0).on_click(move |_e, _w, _cx| {
            *flag.borrow_mut() = Some(i);
            cmds.wake();
        });
        row = row.child(pen).child(icon_cmd(Icon::Minus, "devResource.remove", json!({ "id": node, "index": i }), f, false));
    }
    row
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Measurements（有选中 = 与它有关的；无选中 = 本页全部）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 留在画布上的测量：点一行选中起点，「−」删掉；有选中时给一个「加一条」菜单
/// （egui `measurements_section`）。
pub(crate) fn measurements(f: &RightFields, d: &PanelData) -> Option<PanelSection> {
    let dev = &d.dev;
    if dev.node.is_none() && dev.measurements.is_empty() {
        return None;
    }
    let empty = dev.measurements.is_empty();
    let mut sec = PanelSection::new(tr("Measurements")).variant(if empty { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    for m in &dev.measurements {
        sec = sec.child(field_row_full(measure_line(f, m)));
    }
    let Some(node) = dev.node else { return Some(sec) };
    // 「加一条」：两选之间 / 到外层 / 沿某一条边。
    let mut rows: Vec<(String, &'static str, Value)> = Vec::new();
    if dev.can_measure_between
        && let [a, b] = dev.sel.as_slice()
    {
        rows.push((tr("Between the selected layers").into(), "devMode.addMeasurement", json!({ "from": a, "to": b })));
    }
    if dev.can_measure_around {
        rows.push((tr("To the layer around it").into(), "devMode.addMeasurement", json!({ "from": node })));
    }
    for (label, side) in
        [(tr("Along the top"), "top"), (tr("Along the right"), "right"), (tr("Along the bottom"), "bottom"), (tr("Along the left"), "left")]
    {
        rows.push((label.into(), "devMode.addMeasurement", json!({ "from": node, "side": side })));
    }
    Some(sec.child(field_row(vec![pick_dropdown(f, tr("Add measurement"), 0.0, rows).into_any_element()], Vec::<IconButton>::new())))
}

/// 一条测量：图标 + 测的是什么 + 距离 + 删除。
fn measure_line(f: &RightFields, m: &MeasurementRow) -> Div {
    let t = tokens();
    let what = match (&m.to_name, m.side) {
        (Some(to), _) => format!("{} → {to}", m.from_name),
        (None, Some(side)) => format!("{} · {side}", m.from_name),
        (None, None) => m.from_name.clone(),
    };
    let from = m.from;
    let cmds = f.cmds.clone();
    let body = div()
        .id(SharedString::from(format!("measure-{}-{}", m.index, what)))
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .flex_col()
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| {
            cmds.push(Cmd { id: "select.set", params: Some(json!({ "ids": [from] })) });
        })
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(what)))
        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(m.value.clone())));
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .w_full()
        .child(div().flex_none().child(crate::widgets::icon::paint(Icon::Line, rgba(t.icon), 14.0)))
        .child(body)
        .child(icon_cmd(Icon::Minus, "devMode.removeMeasurement", json!({ "from": m.from, "index": m.index }), f, false))
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// 小工具
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 勾选型下拉（`MenuRow::checked`）：钉属性菜单要看得出哪几条已经钉了。
fn checked_dropdown(f: &RightFields, label: &'static str, rows: Vec<(String, bool, Value)>) -> Dropdown {
    let menu: Vec<MenuRow> = rows.iter().map(|(l, on, _)| MenuRow::new(l.clone()).checked(Some(*on))).collect();
    let mut d = Dropdown::new(SharedString::from(label), 0.0, None).fill();
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let rows = rows.clone();
            let cmds = cmds.clone();
            let menu = menu.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut gpui::Window, _cx: &mut App| {
                // `MenuRow` 的标签带 `✓ ` 前缀，按去掉前缀后的文本找回原行。
                let key = label.trim_start_matches("✓ ").trim();
                if let Some((_, _, params)) = rows.iter().find(|(l, _, _)| l == key) {
                    cmds.push(Cmd { id: "annotation.edit", params: Some(params.clone()) });
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(160.0), menu.clone(), on_pick, cx));
        });
    }
    d
}

/// 一行小标题。
fn sublabel(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(10.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

/// 一行次要提示文字。
fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}
