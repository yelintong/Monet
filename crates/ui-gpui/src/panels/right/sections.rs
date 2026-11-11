//! 右栏各节的**真实行控件**，逐节照搬 `ui-egui`：
//! - `paints.rs::paint_row` → paint 列表（`ColorField` 颜色输入框 + 绑定 chip/按钮 + 显隐/删除）
//! - `effects.rs::effects` / `grids` → effect / grid 列表
//! - `selection_colors.rs::section` → 选区颜色组（`ColorField` + 绑定按钮）
//! - `export_panel.rs::setting_row` → 导出设置列表
//! - `paints.rs::variable_modes` / `section_panel.rs::section` / `right.rs::header` / `page_section`
//!
//! 所有写回都通过 `Cmd` 通道（`sync_panel` 统一 `execute`），与 egui 的 `app.run/set_props` 同构。
//! 动态列表的输入框由父模块的**编辑器池**提供（retained tree 不能每帧新建 `Entity`）。

use figcraft_i18n::tr_noop;
use figcraft_i18n::tr;
use super::data::{LocalStyleRow, PaintRow, PanelData, ViewData};
use crate::canvas::geom::Color32;
use crate::motion::{Act, Queue};
use crate::panels::{Cmd, ROW_EDITORS, RightFields, RowEditCtx, visibility};
use crate::widgets::{
    ColorField, ColorText, Dropdown, FieldRow, HEADER_H, Icon, IconButton, MenuRow, PAD, PanelSection, SECTION_BOTTOM, SectionHeader,
    SectionHeaderVariant, Segmented, TITLE_PT, field_row, field_row_full, field_row_full_trailing, pt, rgba, sublabel_row, sublabel_row2, swatch,
    tokens, variable_button,
};
use figcraft_color::PaintKind;
use figcraft_doc::anim::motion::AnimProperty;
use figcraft_doc::NodeId;
use gpui::{App, Bounds, MouseButton, Pixels, Point, SharedString, Window, div, prelude::*, px, size};
use serde_json::{Value, json};
use std::rc::Rc;

// ───────────────────────────────────────────────────────────────────────────────────────────────
// 公共小工具
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 推一条命令（等同 egui 的 `app.run(cmd, {"ids": ids, …})`）。
fn push(f: &RightFields, id: &'static str, params: Value) {
    f.cmds.push(Cmd { id, params: Some(params) });
}

/// 右栏一节（Fill / Stroke / Effects / Layout grid / Export …）的两种呈现。
///
/// **空节只有一行置灰头部**：没有 body、没有提示文字、没有底边距 —— 视觉上就是「这一节
/// 死在那里」，只剩标题和右边那排图标（`+` 仍然可点，功能没丢）。
///
/// 为什么空节不能走 `PanelSection`：它的 body 恒为
/// `div().w_full().pb(pt(SECTION_BOTTOM))`（`widgets/panel_section.rs:234`，该文件 FROZEN
/// 改不了），哪怕一行都没有也会垫出 12 pt 空白 —— 于是空节看着「没内容却多一截」。
/// 这里对空态直接返回裸 `SectionHeader`，天然没有 body。
pub(crate) enum Section {
    /// 有内容的完整节：头部 + body 网格。
    Full(PanelSection),
    /// 空节：只有置灰头部。
    Empty(SectionHeader),
}

impl Section {
    /// 空节：置灰标题 + 与有内容时**同一套** header 图标。
    fn empty(title: impl Into<SharedString>, trail: impl IntoElement) -> Self {
        Section::Empty(SectionHeader::new(title).variant(SectionHeaderVariant::Dim).trail(trail))
    }
}

impl IntoElement for Section {
    type Element = gpui::Div;

    fn into_element(self) -> gpui::Div {
        match self {
            Section::Full(sec) => sec.into_element(),
            // ⛔ 空节必须**钉死**成标准 40 pt，且禁止收缩（`flex_none()`）。
            //
            // 右栏 body 是 `flex_1().min_h(px(0.0)).flex_col().overflow_y_scroll()`
            // （`panels.rs::right_panel`）。`min_h(0)` 关掉了自动最小高度，于是直接塞进
            // body 的裸 `SectionHeader` 会被当成**可收缩的 flex 项**：它只有 `height: 40`、
            // 内容只有一行字，内容型最小尺寸 ≈ 文字行高（十几 pt），内容一长就被压扁。
            // `PanelSection` 那层之所以没事，是因为它没有指定高度、高度由内容撑开，
            // 自动最小高度 = 全部内容 → 收缩不了。这里把外层拿掉了，就得自己补上这个保护。
            Section::Empty(header) => div().flex_none().w_full().h(pt(HEADER_H)).child(header).into_element(),
        }
    }
}

/// 认领一个池槽位；超出时夹到最后一个，保证不会 panic。
pub(crate) fn slot(n: &mut usize) -> usize {
    let s = (*n).min(ROW_EDITORS - 1);
    *n += 1;
    s
}

/// 一个「点开菜单选中即推命令」的下拉（egui `menu_popup` + `menu_item`）。
/// `rows`: (显示名, 选中后推的命令 id, 命令额外参数)。`cmds` 句柄 clone 进 `on_pick`。
pub(crate) fn pick_dropdown(f: &RightFields, label: impl Into<SharedString>, width: f32, rows: Vec<(String, &'static str, Value)>) -> Dropdown {
    let menu: Vec<MenuRow> = rows.iter().map(|(l, _, _)| MenuRow::new(l.clone())).collect();
    let mut d = Dropdown::new(label, width, None).fill();
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let rows = rows.clone();
            let cmds = cmds.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some((_, cmd, params)) = rows.iter().find(|(l, _, _)| l == label) {
                    cmds.push(Cmd { id: cmd, params: Some(params.clone()) });
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(width.max(120.0)), menu.clone(), on_pick, cx));
        });
    }
    d
}

/// Zoom / Show 菜单的一行：显示/隐藏开关带 `menu_check` 的勾选列，分组用分隔线。
#[derive(Clone)]
struct ViewMenuRow {
    label: String,
    cmd: &'static str,
    params: Value,
    checked: Option<bool>,
    separator: bool,
}

impl ViewMenuRow {
    fn plain(label: &str, cmd: &'static str) -> Self {
        Self { label: label.into(), cmd, params: json!({}), checked: None, separator: false }
    }

    fn toggle(label: &str, cmd: &'static str, params: Value, on: bool, separator: bool) -> Self {
        Self { label: label.into(), cmd, params, checked: Some(on), separator }
    }
}

/// 带勾选状态的下拉（egui `right.rs` 的 Zoom `menu_popup` + `menu_check`）。
/// `MenuRow` 自己负责 `✓ ` 前缀与对齐，`on_pick` 按**纯文本**匹配，所以这里只存原始 label。
fn checked_dropdown(f: &RightFields, label: impl Into<SharedString>, width: f32, rows: Vec<ViewMenuRow>) -> Dropdown {
    let menu: Vec<MenuRow> = rows
        .iter()
        .map(|r| {
            let mut m = MenuRow::new(r.label.clone());
            if r.separator {
                m = m.separator();
            }
            if let Some(on) = r.checked {
                m = m.checked(Some(on));
            }
            m
        })
        .collect();
    let mut d = Dropdown::new(label, width, None);
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let rows = rows.clone();
            let menu = menu.clone();
            let cmds = cmds.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(r) = rows.iter().find(|r| r.label == label) {
                    cmds.push(Cmd { id: r.cmd, params: Some(r.params.clone()) });
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(width.max(200.0)), menu, on_pick, cx));
        });
    }
    d
}

/// 右栏顶部工具行（egui `right.rs` 的 tabs row + Zoom 菜单）。
/// 这里只接**有引擎命令**的「显示 / 隐藏」开关；Share / Design / Prototype 是弹窗或纯 UI
/// 状态，没有对应命令，按用户约定继续留白（源码见 `reference/right.rs.txt`）。
fn view_rows(v: &ViewData) -> Vec<ViewMenuRow> {
    let mut rows = Vec::new();
    for (label, cmd) in [
        (tr("Zoom in"), "view.zoomIn"),
        (tr("Zoom out"), "view.zoomOut"),
        (tr("Zoom to fit"), "view.zoomFit"),
        (tr("Zoom to 50%"), "view.zoom50"),
        (tr("Zoom to 100%"), "view.zoom100"),
        (tr("Zoom to 200%"), "view.zoom200"),
        (tr("Zoom to selection"), "view.zoomSelection"),
    ] {
        rows.push(ViewMenuRow::plain(label, cmd));
    }
    // egui `pixel_preview`: `{"on": !on, "scale": scale}`，1x / 2x 各自打勾。
    for scale in [1u8, 2u8] {
        let on = v.pixel_preview && v.pixel_scale == scale;
        rows.push(ViewMenuRow::toggle(&format!("Pixel preview {scale}x"), "view.pixelPreview", json!({ "on": !on, "scale": scale }), on, scale == 1));
    }
    // egui `toggles`：这些命令不传参数，引擎 `flag()` 自己按当前值取反。
    for (label, cmd, on, separator) in [
        (tr("Pixel grid"), "view.pixelGrid", v.pixel_grid, true),
        (tr("Snap to pixel grid"), "view.snapPixel", v.snap_pixel, false),
        (tr("Layout grids"), "view.layoutGrids", v.layout_grids, false),
        (tr("Rulers"), "view.rulers", v.rulers, false),
        (tr("Outlines"), "view.outlines", v.outlines, false),
    ] {
        rows.push(ViewMenuRow::toggle(label, cmd, json!({}), on, separator));
    }
    rows
}

pub(crate) fn view_bar(f: &RightFields, d: &PanelData, motion: bool) -> gpui::Div {
    let t = tokens();
    let zoom_label = format!("{}%", (d.view.zoom * 100.0).round() as i64);
    let zoom = checked_dropdown(f, zoom_label, 64.0, view_rows(&d.view));
    // egui `right.rs` 的 tabs 行（Design / Prototype）：激活的那个上底色 `hover`、圆角 5、
    // 半粗 11 pt；未激活的 `text_secondary`、无底色。整行 40 pt 高、下边框。
    // 纯 UI 状态（`app.ui.right_tab`），不进文档，所以只动 `f.right_tab` 并唤醒一帧。
    let tab = |label: &'static str, idx: u8| -> gpui::Stateful<gpui::Div> {
        // 先取值再进闭包，避免 `borrow()` 与 `borrow_mut()` 同语句重叠。
        let active = *f.right_tab.borrow() == idx;
        let tabs = f.right_tab.clone();
        let cmds = f.cmds.clone();
        div()
            .id(SharedString::from(match idx {
                0 => "rtab-design",
                1 => "rtab-prototype",
                _ => "rtab-inspect",
            }))
            .h(pt(24.0))
            .px(pt(8.0))
            .flex()
            .items_center()
            .rounded(pt(5.0))
            .cursor_pointer()
            .when(active, |d| d.bg(rgba(t.hover)))
            .hover(|s| s.bg(rgba(t.hover)))
            .on_mouse_down(MouseButton::Left, move |_e, _w, _cx| {
                {
                    let mut cur = tabs.borrow_mut();
                    if *cur != idx {
                        *cur = idx;
                    }
                }
                cmds.wake();
            })
            .child(
                div()
                    .text_size(pt(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(if active { t.text } else { t.text_secondary }))
                    .child(label),
            )
    };
    // 2026-11-10 用户要求：动画 / 开发模式下，这一栏不显示 Design / Prototype 两个标签，
    // 原位置直接换成文本——动画写「动画」、开发写「开发」；其余模式仍是可点的 Design / Prototype。
    // 开发模式由右栏标签下标 2（set_right_tab(2)）判定；动画模式由 motion 判定。
    let mode_label = |s: &'static str| -> gpui::Div {
        div()
            .h(pt(24.0))
            .px(pt(8.0))
            .flex()
            .items_center()
            .rounded(pt(5.0))
            .text_size(pt(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.text))
            .child(s)
    };
    let tabs = if motion {
        div().flex().flex_row().items_center().gap(pt(2.0)).child(mode_label("动画"))
    } else if f.right_tab() == 2 {
        div().flex().flex_row().items_center().gap(pt(2.0)).child(mode_label("开发"))
    } else {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(2.0))
            // 标签文案仍由 naming::right_tab_label 统一提供（a11y / 命名回归测试继续用它）。
            .child(tab(tr(crate::naming::right_tab_label(0)), 0))
            .child(tab(tr(crate::naming::right_tab_label(1)), 1))
    };
    div()
        .flex_none()
        .w_full()
        .h(pt(40.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(pt(8.0))
        .border_b_1()
        .border_color(rgba(tokens().border))
        .child(tabs)
        .child(zoom)
}

/// 一个图标按钮，点击推命令（egui `icons::button` + `app.run`）。
pub(crate) fn icon_cmd(icon: Icon, cmd_id: &'static str, params: Value, f: &RightFields, active: bool) -> IconButton {
    let cmds = f.cmds.clone();
    IconButton::new(icon).size(24.0).active(active).on_click(move |_e, _w, _cx| {
        cmds.push(Cmd { id: cmd_id, params: Some(params.clone()) });
    })
}

/// 把 sizing 菜单挂到 W/H 字段的 **trail 槽**（egui `number_field_mixed_menu`）：右端一个
/// chevron，点开是 Fixed/Hug/Fill + Add min/max + Remove + Apply variable。不是独立控件。
pub(crate) fn dim_trail_menu(
    f: &RightFields,
    cx: &mut App,
    field: &gpui::Entity<crate::widgets::TextField>,
    horizontal: bool,
    dims: &super::data::DimensionsData,
) {
    let cmds = f.cmds.clone();
    let root = f.root.clone();
    let limit_open = f.limit_open.clone();
    let sizing_key: &'static str = if horizontal { "sizingH" } else { "sizingV" };
    let min_key: &'static str = if horizontal { "minWidth" } else { "minHeight" };
    let max_key: &'static str = if horizontal { "maxWidth" } else { "maxHeight" };
    let (has_min, has_max) = if horizontal {
        (dims.min_w.is_some() || limit_open.borrow()[0], dims.max_w.is_some() || limit_open.borrow()[1])
    } else {
        (dims.min_h.is_some() || limit_open.borrow()[2], dims.max_h.is_some() || limit_open.borrow()[3])
    };
    // egui `dim_field`：Fill/Hug 时 trail 显示对应单词，否则一个 chevron。
    let cur = if horizontal { dims.sizing_w } else { dims.sizing_h };
    let trail_slot = match cur {
        Some("fill") => crate::widgets::Slot::Text(SharedString::from(tr("Fill"))),
        Some("hug") => crate::widgets::Slot::Text(SharedString::from(tr("Hug"))),
        _ => crate::widgets::Slot::Icon(Icon::ChevronDown),
    };
    field.update(cx, move |tf, _| {
        tf.set_trail(trail_slot);
        let cmds = cmds.clone();
        let root = root.clone();
        let limit_open = limit_open.clone();
        tf.set_on_trail(move |pos: Point<Pixels>, _w, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            let mut rows: Vec<MenuRow> = vec![MenuRow::new(tr("Fixed")), MenuRow::new(tr("Hug contents")), MenuRow::new(tr("Fill container"))];
            if !has_min {
                rows.push(MenuRow::new(tr("Add min")));
            }
            if !has_max {
                rows.push(MenuRow::new(tr("Add max")));
            }
            if has_min || has_max {
                rows.push(MenuRow::new(if has_min && has_max {
                    "Remove limits"
                } else if has_min {
                    "Remove min"
                } else {
                    "Remove max"
                }));
            }
            let cmds = cmds.clone();
            let limit_open = limit_open.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                let mut push = |cmd: &'static str, params: Value| {
                    cmds.push(Cmd { id: cmd, params: Some(params) });
                };
                match label {
                    "Fixed" => push("node.set", json!({ "props": { sizing_key: "fixed" } })),
                    "Hug contents" => push("node.set", json!({ "props": { sizing_key: "hug" } })),
                    "Fill container" => push("node.set", json!({ "props": { sizing_key: "fill" } })),
                    "Add min" => {
                        let i = if horizontal { 0 } else { 2 };
                        limit_open.borrow_mut()[i] = true;
                    }
                    "Add max" => {
                        let i = if horizontal { 1 } else { 3 };
                        limit_open.borrow_mut()[i] = true;
                    }
                    "Remove limits" => {
                        push("node.set", json!({ "props": { min_key: Value::Null, max_key: Value::Null } }));
                        let (a, b) = if horizontal { (0, 1) } else { (2, 3) };
                        limit_open.borrow_mut()[a] = false;
                        limit_open.borrow_mut()[b] = false;
                    }
                    "Remove min" => {
                        push("node.set", json!({ "props": { min_key: Value::Null } }));
                        limit_open.borrow_mut()[if horizontal { 0 } else { 2 }] = false;
                    }
                    "Remove max" => {
                        push("node.set", json!({ "props": { max_key: Value::Null } }));
                        limit_open.borrow_mut()[if horizontal { 1 } else { 3 }] = false;
                    }
                    _ => {}
                }
                // 仅翻转 `limit_open` 的项（Add/Remove min/max）没有推命令，必须主动唤醒视图，
                // 否则 min/max 字段行要等到下一次别的重绘才出现。
                cmds.wake();
            });
            let anchor = Bounds { origin: pos, size: size(px(0.0), px(0.0)) };
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
        });
    });
}

/// 一个图标按钮，点击打开菜单，选中项推对应命令（egui `icons::button` + `menu_popup`）。
fn menu_icon(f: &RightFields, icon: Icon, width: f32, rows: Vec<(String, &'static str, Value)>, active: bool) -> IconButton {
    let menu: Vec<MenuRow> = rows.iter().map(|(l, _, _)| MenuRow::new(l.clone())).collect();
    let cmds = f.cmds.clone();
    let root = f.root.clone();
    IconButton::new(icon).size(24.0).active(active).on_click(move |_e, window, cx: &mut App| {
        let Some(root) = root.clone() else { return };
        let p = window.mouse_position();
        let anchor = Bounds { origin: p, size: size(px(0.0), px(0.0)) };
        let rows = rows.clone();
        let cmds = cmds.clone();
        let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
            if let Some((_, cmd, params)) = rows.iter().find(|(l, _, _)| l == label) {
                cmds.push(Cmd { id: cmd, params: Some(params.clone()) });
            }
        });
        root.update(cx, |cv, cx| cv.open_menu(anchor, Some(width.max(160.0)), menu.clone(), on_pick, cx));
    })
}

/// 变量 / 文本样式这类「列表选择」弹窗：统一用 `widgets::popover` 的通用窗口承载，
/// 不再退化成一个小下拉菜单（egui 里它们本来就是带标题栏的 detached panel）。
#[derive(Clone, Copy)]
enum ListWindow {
    Variables,
    Styles,
}

fn open_list_window(
    popups: crate::panels::PopupReqs,
    queue: crate::panels::Cmds,
    cx: &mut App,
    anchor: Point<Pixels>,
    kind: ListWindow,
    title: SharedString,
    rows: Vec<(String, &'static str, Value)>,
) {
    let trail = match kind {
        ListWindow::Variables => tr("Variable"),
        ListWindow::Styles => tr("Style"),
    };
    let list: Vec<crate::popovers::Row> =
        rows.into_iter().map(|(label, cmd, params)| crate::popovers::Row::new(label, cmd, params).group(tr("This file")).trail(trail)).collect();
    let w = match kind {
        ListWindow::Variables => crate::popovers::WindowState::variables(queue, cx, anchor, list),
        ListWindow::Styles => crate::popovers::WindowState::styles(queue, cx, anchor, title, list),
    };
    // No `Entity<CanvasView>` guard: the root view drains the slot on its next frame.
    popups.request(w);
}

/// A header icon that opens the detached Variables window (egui `variable_ui::picker`) instead
/// of a little dropdown. These are the visible buttons in the Appearance / Typography headers —
/// wiring the window only to the small in-row binding button left them looking unconnected.
fn variables_window_button(f: &RightFields, icon: Icon, rows: Vec<(String, &'static str, Value)>) -> IconButton {
    let root = f.popups();
    let queue = f.queue();
    IconButton::new(icon).size(24.0).on_click(move |_e, window, cx: &mut App| {
        open_list_window(
            root.clone(),
            queue.clone(),
            cx,
            window.mouse_position(),
            ListWindow::Variables,
            SharedString::from(tr("Variables")),
            rows.clone(),
        );
    })
}

/// 一行的 24 pt 图标按钮组（header 的 trailing 槽）。
pub(crate) fn icon_row(items: Vec<IconButton>) -> gpui::Div {
    let mut d = div().flex().flex_row().items_center().gap(pt(4.0));
    for i in items {
        d = d.child(i);
    }
    d
}

/// 某类样式的「Styles」菜单（egui `style_picker::open` 的等价：列出该类样式并 `style.apply`）。
fn style_menu(f: &RightFields, d: &PanelData, ty: &'static str, target: Option<&'static str>, icon: Icon) -> IconButton {
    // egui `style_picker::apply`: a fill/stroke slot also sends `target` (`"fill"` / `"stroke"`),
    // so the engine knows which paint list the style applies to. Text/effect/grid slots do not.
    let rows: Vec<(String, &'static str, Value)> = d
        .styles
        .iter()
        .filter(|s| s.ty == ty)
        .map(|s| {
            let mut q = json!({ "style": s.id });
            if let Some(t) = target {
                q["target"] = json!(t);
            }
            (s.name.clone(), "style.apply", q)
        })
        .collect();
    // egui 的 style_picker 是一个带标题栏的 detached panel（不是下拉菜单）：
    // 这里用同一个通用窗口恢复它，标题栏的插槽里放「新建样式」按钮。
    let title = match ty {
        "text" => tr("Text styles"),
        "paint" => tr("Color styles"),
        "effect" => tr("Effect styles"),
        "grid" => tr("Grid styles"),
        _ => tr("Styles"),
    };
    let root = f.popups();
    let queue = f.queue();
    IconButton::new(icon).size(24.0).on_click(move |_e, window, cx: &mut App| {
        open_list_window(root.clone(), queue.clone(), cx, window.mouse_position(), ListWindow::Styles, SharedString::from(title), rows.clone());
    })
}

/// egui `style_picker::chip`：已链接共享样式时，在节的**标题下面**多画一行
/// 「类型图标 + 样式名 + Detach」。`None` = 该 slot 没链接样式 → 整行不画
/// （egui 里是 `_ => return`，不占位）。
pub(crate) fn style_chip(
    f: &RightFields,
    name: Option<&String>,
    slot: &'static str,
) -> Option<crate::widgets::panel_section::FieldRow> {
    let name = name?;
    let t = tokens();
    let cmds = f.cmds.clone();
    let body = div()
        .w_full()
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .child(
            div()
                .flex_none()
                .w(pt(14.0))
                .h(pt(14.0))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(Icon::Dot, rgba(t.text_secondary), 14.0)),
        )
        .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(name.clone())));
    let detach = IconButton::new(Icon::Detach).size(24.0).on_click(move |_e, _w, _cx| {
        cmds.push(Cmd { id: "style.detach", params: Some(json!({ "which": slot })) });
    });
    Some(field_row_full_trailing(body, vec![detach]))
}

/// `Field-color / Disabled`（= 变量绑定态）的**全部**绑定行为都收在 `ColorField` 里：
/// 整框点击 → 打开变量菜单换绑；尾插槽那个 24 × 24 按钮 → 解绑（原先是 "%" 单元）。
/// 调用方只给解绑要推的命令，不再在行里另挂一个绑定/解绑 chip。
/// `selection_targets = Some(..)` 时（选区颜色组）换绑推 `selection.bindColorVariable`，
/// 解绑推 `selection.detachColor` —— egui `selection_colors.rs` 用的是这一组专用命令。
fn bound_color_field(
    f: &RightFields,
    cf: ColorField,
    name: &str,
    field: &str,
    vars: &[String],
    selection_targets: Option<Value>,
    detach: (&'static str, Value),
) -> ColorField {
    let root = f.popups();
    let queue = f.queue();
    let vars = vars.to_vec();
    let field = field.to_string();
    let cmds = f.cmds.clone();
    cf.bound(name.to_string(), move |pos, _window, cx| {
        let rows = vars
            .iter()
            .map(|v| {
                if let Some(targets) = &selection_targets {
                    (v.clone(), "selection.bindColorVariable", json!({ "targets": targets, "variable": v }))
                } else {
                    (v.clone(), "variable.bind", json!({ "field": field, "variable": v }))
                }
            })
            .collect();
        open_list_window(root.clone(), queue.clone(), cx, pos, ListWindow::Variables, SharedString::from(tr("Variables")), rows);
    })
    .on_detach(move |_window, _cx| {
        cmds.push(Cmd { id: detach.0, params: Some(detach.1.clone()) });
    })
}

/// Selection colors 的绑定按钮（`selection_colors.rs::section`）：未绑定时一个 Styles 菜单
/// 列出颜色样式（`selection.applyColorStyle`）与颜色变量（`selection.bindColorVariable`）；
/// 已绑定时解绑按钮在 `ColorField` 的 24 pt 尾插槽里，这里保留一个空的 24 占位，
/// 让绑定/未绑定两种行的两个 24 槽严格同位（egui 的这一格本来就是图标列）。
fn selection_bind(f: &RightFields, row: &super::data::SelectionColorRow, d: &PanelData) -> gpui::Div {
    let t = tokens();
    let targets = row.targets.clone();
    if row.linked.is_some() {
        return div().size(pt(24.0));
    }
    let mut rows: Vec<(String, &'static str, Value)> = d
        .styles
        .iter()
        .filter(|s| s.ty == "paint")
        .map(|s| (s.name.clone(), "selection.applyColorStyle", json!({ "targets": targets, "style": s.id })))
        .collect();
    // egui `section`: 只列作用域内可绑定的变量，并提交变量 **id**（不是显示名）。
    rows.extend(row.variables.iter().map(|(name, id)| (name.clone(), "selection.bindColorVariable", json!({ "targets": targets, "variable": id }))));
    let root = f.popups();
    let queue = f.queue();
    div()
        .size(pt(24.0))
        .flex()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(variable_button())
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_e, window, cx: &mut App| {
            open_list_window(
                root.clone(),
                queue.clone(),
                cx,
                window.mouse_position(),
                ListWindow::Variables,
                SharedString::from(tr("Variables")),
                rows.clone(),
            );
        })
}

/// 颜色行的 swatch 点击 → 打开取色器（egui `toggle_picker`）。选色写进 `key` 后推 `cmd`。
/// `fill.set`/`stroke.set`/`page.setBackground` 用 `color`；`selection.setPaint` 必须提交
/// **完整 Paint**：`base_paint = Some(group.paint)` 时只把 `color` 换掉，保留 opacity / blend /
/// visible，否则一个 `"#RRGGBB"` 会把整组的透明度重置成 1。
fn color_pick(
    f: &RightFields,
    cf: ColorField,
    current: Color32,
    cmd: &'static str,
    params: Value,
    key: &'static str,
    base_paint: Option<Value>,
) -> ColorField {
    let popups = f.popups();
    let cmds = f.cmds.clone();
    cf.on_swatch(move |pos: Point<Pixels>, _window, cx: &mut App| {
        let on_pick = Rc::new({
            let cmds = cmds.clone();
            let params = params.clone();
            let base_paint = base_paint.clone();
            move |c: Color32, _w: &mut Window, _cx: &mut App| {
                let mut p = params.clone();
                if let Some(base) = &base_paint {
                    let mut paint = base.clone();
                    paint["color"] = json!(c.to_hex());
                    p["paint"] = paint;
                } else {
                    p[key] = json!(c.to_hex());
                }
                cmds.push(Cmd { id: cmd, params: Some(p) });
            }
        });
        // Synchronous request, no detached async `spawn` and no root-Entity guard: the old
        // `cx.spawn(...).detach()` delayed the picker by a frame and could be dropped entirely.
        let w = crate::popovers::WindowState::color(cx, pos, current, on_pick, cmds.clone());
        popups.request(w);
    })
}

/// paint 行的 swatch（`paints.rs::paint_row`）：选色推 `fill.set`/`stroke.set`，带 `index`。
fn paint_swatch(f: &RightFields, row: &PaintRow, cf: ColorField) -> ColorField {
    let current = row.paint.solid_color().map(crate::widgets::swatch::color32).unwrap_or(Color32::BLACK);
    let cmd = if row.fills { "fill.set" } else { "stroke.set" };
    color_pick(f, cf, current, cmd, json!({ "index": row.index }), "color", None)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Fill / Stroke —— `paints.rs::paints` + `paint_row`
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// paint 列表节。`ColorField` 是「颜色输入框」，它自己的 24 pt 尾插槽同时是绑定/解绑按钮；
/// 行尾两个 24 pt 槽固定给显隐与删除，不再另挂绑定 chip。
pub(crate) fn paints(
    f: &RightFields,
    d: &PanelData,
    fills: bool,
    has_vector: bool,
    row_w: f32,
    col: f32,
    n: &mut usize,
    _window: &mut Window,
    cx: &mut App,
    // Motion mode: when `Some`, this is the layer whose first solid paint keys its colour at the
    // playhead (egui `paints.rs` passes `keyframe: Some(layer)`). The Design panel passes `None`.
    // `keyed` is the precomputed keyframe-at-playhead state for that paint's colour property.
    keyframe_layer: Option<NodeId>,
    keyed: bool,
    motion_q: &Queue,
) -> Section {
    let title = if fills { tr("Fill") } else { tr("Stroke") };
    let add_cmd = if fills { "fill.add" } else { "stroke.add" };
    let list = if fills { d.fills.clone() } else { d.strokes.clone() };
    // Header icons（egui `paints.rs::paints` 的 `[Styles, (FillRule), Plus]`）：
    // Styles → 样式菜单；FillRule → 填充规则；Plus → 添加。
    // egui: `let vector = if fills { first_vector(doc, sel) } else { None };
    //        if vector.is_some() { buttons.push(FillRule) }` —— 只有选中矢量网络才画。
    // 此前这里 `if fills` 无条件画，普通矩形/文本也顶着一个无效的填充规则按钮。
    let mut icons = vec![style_menu(f, d, "paint", Some(if fills { "fill" } else { "stroke" }), Icon::Dot)];
    if visibility::fill_rule(fills, has_vector) {
        icons.push(menu_icon(
            f,
            Icon::ViewGrid,
            160.0,
            vec![
                ("Non-zero".into(), "vector.fillRule", json!({ "rule": "nonZero" })),
                ("Even-odd".into(), "vector.fillRule", json!({ "rule": "evenOdd" })),
            ],
            false,
        ));
    }
    icons.push(icon_cmd(Icon::Plus, add_cmd, json!({}), f, false));
    // 空 / Mixed（多选混合，`list == None`）：**只有置灰头部**——不写提示文字、不给 body、
    // 也就没有 `PanelSection` 那 12 pt 底边距。`+` 图标照旧可点，功能没少。
    let Some(rows) = list.filter(|v| !v.is_empty()) else {
        return Section::empty(title, icon_row(icons));
    };
    let mut sec = PanelSection::new(title).header_trail(icon_row(icons));
    // egui `paints.rs`：header 之后、paint 行之前先画链接样式的 chip（`style_picker::chip`）。
    if let Some(chip) = style_chip(f, d.style_chips[if fills { 0 } else { 1 }].as_ref(), if fills { "fill" } else { "stroke" }) {
        sec = sec.child(chip);
    }
    // egui 倒序画（最上层 paint 在最前）。只有**第一道实心 paint** 的 colour 可关键帧化
    // （egui `paints.rs` 的 `first_solid`）。Motion 模式下那一行行尾多出一个菱形按钮。
    let first_solid = rows.iter().position(|r| matches!(r.paint.kind, PaintKind::Solid { .. }));
    for (idx, row) in rows.iter().enumerate().rev() {
        let cmd = if fills { "fill.set" } else { "stroke.set" };
        let index = row.index;
        let hex_slot = slot(n);
        let op_slot = slot(n);
        let hex_ed = f.row_field(
            cx,
            hex_slot,
            &row.text,
            RowEditCtx { cmd, params: json!({ "index": index }), key: "color", scale: 1.0, string_value: true, paint: None, nest: None, nest_key: None, paint_color: None },
        );
        let op_ed = f.row_field(
            cx,
            op_slot,
            &format!("{}", row.opacity.round() as i64),
            RowEditCtx { cmd, params: json!({ "index": index }), key: "opacity", scale: 100.0, string_value: false, paint: None, nest: None, nest_key: None, paint_color: None },
        );
        let text = if row.text_is_hex {
            ColorText::Hex(SharedString::from(row.text.clone()))
        } else {
            ColorText::Label(SharedString::from(row.text.clone()))
        };
        // 颜色输入框跨两列（w_full），行尾只有两个 24 pt 槽，同 egui `paint_row` 的
        // `available_width - 56` + `icon_pair`。绑定/解绑一律由 ColorField 自己的尾插槽承担。
        let cf = ColorField::new(cx, row.paint.clone(), text, row_w, hex_ed, op_ed);
        let cf = paint_swatch(f, row, cf);
        // 变量绑定态 = 设计稿 `Field-color / Disabled`：无背景、整框可点换绑、透明度数值消失，
        // 24 pt 尾插槽换成解绑按钮；未绑定态该插槽仍是 100 / "%"。
        let cf = match row.bound.as_deref() {
            Some(name) => bound_color_field(
                f,
                cf,
                name,
                &row.bind_field,
                &row.variables,
                None,
                ("variable.bind", json!({ "field": row.bind_field.clone(), "variable": Value::Null })),
            ),
            None => cf,
        };
        let eye = icon_cmd(if row.visible { Icon::Eye } else { Icon::EyeOff }, cmd, json!({ "index": index, "visible": !row.visible }), f, false);
        let minus = icon_cmd(Icon::Minus, if fills { "fill.remove" } else { "stroke.remove" }, json!({ "index": index }), f, false);
        // Motion mode: the first solid paint's row carries a keyframe diamond (egui `diamond()`).
        // Clicking toggles a keyframe for this paint's colour at the playhead. The keyed fill state
        // is precomputed by the host from the motion playhead.
        let mut trailing = vec![eye, minus];
        if keyframe_layer.is_some() && first_solid == Some(idx) {
            let q = motion_q.clone();
            let layer = keyframe_layer.unwrap();
            let prop = if fills { AnimProperty::FillColor } else { AnimProperty::StrokeColor };
            let diamond = IconButton::new(if keyed { Icon::Component } else { Icon::Instance })
                .on_click(move |_, _, _| q.push(Act::ToggleKey(layer, prop)));
            trailing.push(diamond);
        }
        sec = sec.child(field_row_full_trailing(cf, trailing));
    }
    // Stroke 第二行：描边对齐下拉 + 线宽字段 + 独立边宽 + 高级描边（egui `paints.rs`：
    // `dropdown(align, half)` 之后紧跟 `number_field_mixed("strokeWeight", …, weight_w)`）。
    // ⚠️ 混合模式（Blend）**不在这里**：egui 把它放在 `appearance` 节里，由 header 的
    // blend 开关（`f.blend_open`）展开下拉；Fill 的 `paints` 里没有独立混合模式行。
    // 之前这里把 blend 当成 Fill 末尾常驻行重复画了一次，正是「填充下面多出一块」的根因。
    if !fills {
        if let Some(align) = d.stroke_align {
            let rows = [
                (tr("Inside"), "node.set", json!({"props": {"strokeAlign": "inside"}})),
                (tr("Center"), "node.set", json!({"props": {"strokeAlign": "center"}})),
                (tr("Outside"), "node.set", json!({"props": {"strokeAlign": "outside"}})),
            ]
            .into_iter()
            .map(|(l, c, p)| (l.to_string(), c, p))
            .collect();
            // `IndependentCorners`（独立边宽，`strokeSides`）+ `Dots`（高级描边样式）。
            // egui `paints.rs`: `let w = weight.unwrap_or(1.0)` —— 每条边的权重来自当前
            // 描边宽度，硬编码 1.0 会把一条 8pt 的描边点一下就变成 1pt。
            let w = d.stroke_weight.unwrap_or(1.0);
            let sides = menu_icon(
                f,
                Icon::AbsolutePosition,
                200.0,
                vec![
                    (tr("All").into(), "node.set", json!({ "props": { "strokeSides": Value::Null } })),
                    (tr("Top").into(), "node.set", json!({ "props": { "strokeSides": [w, 0.0, 0.0, 0.0] } })),
                    (tr("Right").into(), "node.set", json!({ "props": { "strokeSides": [0.0, w, 0.0, 0.0] } })),
                    (tr("Bottom").into(), "node.set", json!({ "props": { "strokeSides": [0.0, 0.0, w, 0.0] } })),
                    (tr("Left").into(), "node.set", json!({ "props": { "strokeSides": [0.0, 0.0, 0.0, w] } })),
                    (tr("Custom").into(), "node.set", json!({ "props": { "strokeSides": [w, w, w, w] } })),
                ],
                false,
            );
            // Advanced stroke (egui `right/stroke.rs::show_stroke_settings`)：线宽线形、
            // dash/gap、端帽、连接、宽度配置与 Brush，恢复成带标题栏的「Stroke」通用窗口。
            let popups = f.popups();
            let stroke_queue = f.queue();
            let stroke_data = d.clone();
            let dots = IconButton::new(Icon::Menu).size(24.0).on_click(move |_e, window, cx: &mut App| {
                let anchor = window.mouse_position();
                let w = crate::popovers::WindowState::stroke(stroke_queue.clone(), &stroke_data, cx, anchor);
                popups.request(w);
            });
            // 第二行 = 描边对齐下拉 + **线宽字段** + 独立边宽 + 高级描边（egui `paints.rs`:
            // `dropdown(align, half)` 之后紧跟 `number_field_mixed("strokeWeight", …, weight_w)`）。
            sec = sec.child(field_row(
                vec![pick_dropdown(f, align, col, rows).into_any_element(), f.stroke_weight.clone().into_any_element()],
                vec![sides, dots],
            ));
        }
    }
    Section::Full(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Effects —— `effects.rs::effects`
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// WGSL 材质下拉行 + 选中文件的参数输入框，并入 Effects section 的统一网格。
/// 已绑定 WGSL 材质时，在 Effects 节内就地画：
/// * 一行**和普通效果行同款的下拉**：显示当前材质名，点开列固定目录里每个 `.wgsl`（就地换材质）
///   以及一条「None」（解绑）；行尾仍是 24 pt 的 `−`（与 effect 行删除键同位置）。
/// * 该 shader 扫描出的参数，逐项画成「左边独立名称 + 右边数值输入框」（名称不再塞进输入框）。
///
/// 未绑定材质时返回空 `Vec`，Effects 节就是原来的样子（零回归）——选择入口在 `+` 菜单里也有
/// 一份，所以不绑定也能加材质；绑定后这里多一个可换材质的下拉。
fn wgsl_material_rows(
    f: &RightFields,
    d: &PanelData,
    col: f32,
    cx: &mut App,
) -> Vec<crate::widgets::FieldRow> {
    let Some(path) = d.wgsl.path.as_deref() else { return Vec::new() };
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());

    let mut out = Vec::new();
    // 下拉：与 `+` 菜单同源（每次现扫固定目录），选中即 `node.set` 换绑定；末尾 `None` 解绑。
    // 这样外置材质和普通效果一样，行首就是一个下拉，而不是一段只能看的纯文本。
    let mut items: Vec<(String, &'static str, Value)> = crate::wgsl_material::scan_material_dir()
        .iter()
        .map(|wf| {
            let p = wf.path.to_string_lossy().into_owned();
            (wf.name.clone(), "node.set", crate::wgsl_material::write_binding_params(Some(&crate::wgsl_material::binding_for(&p))))
        })
        .collect();
    if items.is_empty() {
        // 目录里没有文件（或目录不存在）：下拉至少保留当前这一条，不让它变成一个空菜单。
        items.push((name.clone(), "node.set", crate::wgsl_material::write_binding_params(Some(
            &crate::wgsl_material::binding_for(path)))));
    }
    items.push((tr("None").to_string(), "node.set", crate::wgsl_material::write_binding_params(None)));
    let dd = pick_dropdown(f, name, col, items);
    let minus = icon_cmd(Icon::Minus, "node.set", crate::wgsl_material::write_binding_params(None), f, false);
    out.push(field_row_full_trailing(dd, vec![minus]));

    // 该 shader 的参数在下面就地展开：名称是**输入框前面的独立文本**，值才进输入框。
    let specs = crate::wgsl_material::params_for(path);
    if specs.is_empty() {
        return out;
    }
    out.push(sublabel_row(tr("WGSL Material Parameters")));
    let binding = crate::wgsl_material::WgslMaterialBinding {
        path: path.to_string(),
        params: d.wgsl.params.clone(),
    };
    for (slot, spec) in specs.iter().take(crate::panels::WGSL_PARAM_FIELDS).enumerate() {
        let value = d.wgsl.params.get(&spec.field).copied().unwrap_or(spec.default);
        let field = f.wgsl_param_field(cx, slot, spec, value, &binding);
        let label = div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(pt(crate::widgets::TEXT_PT))
            .text_color(rgba(tokens().text_secondary))
            .child(spec.label.clone());
        out.push(field_row(
            vec![label.into_any_element(), field.into_any_element()],
            Vec::<gpui::AnyElement>::new(),
        ));
    }
    out
}

/// Effects 标题栏的 `+`：点开是一个「添加效果」菜单 —— 先列 7 种内置效果（`effect.add`），
/// 一条分隔线后列出固定目录里每个 `.wgsl` 文件（`node.set` 写 plugin_data 绑定）。
///
/// 这是用户定的交互：WGSL 不另起一行下拉，而是**和内置效果同处一个菜单**，有多少个文件就
/// 有多少个菜单项；选中文件后其参数才在下方就地展开（由 `d.wgsl.path` 驱动，见 `effects()`）。
/// 菜单项每次点开时现扫固定目录，增删文件立刻反映，无需重编译。
fn add_effect_menu(f: &RightFields) -> IconButton {
    let mut b = IconButton::new(Icon::Plus).size(24.0);
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        b = b.on_click(move |_e, window, cx: &mut App| {
            // `on_click` 是 `Fn`，每次点击都要给 on_pick 一份独立的队列句柄，故这里重新 clone
            // （不能直接把外层的 `cmds` 移进内层闭包）。与 `pick_dropdown` 同构。
            let cmds = cmds.clone();
            // 7 种内置效果：与 `Effect::TYPES` 同源，选中即加一条该类型效果。
            let mut items: Vec<(String, &'static str, Value)> = figcraft_doc::Effect::TYPES
                .iter()
                .map(|(label, ty)| (label.to_string(), "effect.add", json!({ "type": ty })))
                .collect();
            // 内置效果项数：第一个材质项前面画一条分隔线，把它和内置效果分组。
            let types_len = items.len();
            // 固定目录材质：每个文件一项，选中写完整的 binding（含默认参数）到 plugin_data。
            for wf in crate::wgsl_material::scan_material_dir() {
                let binding = crate::wgsl_material::binding_for(&wf.path.to_string_lossy());
                items.push((wf.name.clone(), "node.set", crate::wgsl_material::write_binding_params(Some(&binding))));
            }
            // `separator_before` 只在行上方画一条线、该行仍可点，故分隔标记挂在第一个材质项上，
            // 不引入不可点的「标题行」（那种行被点中会按 label 匹配、可能误触发命令）。
            let rows: Vec<MenuRow> = items
                .iter()
                .enumerate()
                .map(|(i, (l, _, _))| {
                    let r = MenuRow::new(l.clone());
                    if i == types_len { r.separator() } else { r }
                })
                .collect();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some((_, cmd, params)) = items.iter().find(|(l, _, _)| l == label) {
                    cmds.push(Cmd { id: cmd, params: Some(params.clone()) });
                }
            });
            let p = window.mouse_position();
            let anchor = Bounds { origin: p, size: size(px(1.0), px(0.0)) };
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(180.0), rows, on_pick, cx));
        });
    }
    b
}

pub(crate) fn effects(
    f: &RightFields,
    d: &PanelData,
    col: f32,
    cx: &mut App,
    // Motion 模式：`(layer, 该属性在 playhead 已有关键帧, 动作队列)`；Design 传 None。
    // egui effects.rs 给每个 shadow/blur 数值字段一个菱形；GPUI 的行是「类型下拉 + 显隐 + 删除」，
    // 这里给每条 effect 行一个菱形，按 effect 类型映射到 ShadowBlur / BlurRadius。
    keyframe: Option<(NodeId, Vec<figcraft_doc::anim::motion::AnimProperty>, &Queue)>,
) -> Section {
    // `[Styles, Plus]`；`+` 点开是「添加效果」菜单：7 种内置效果 + 分隔线 + 固定目录里每个
    // `.wgsl` 文件项（有多少文件就多少项）。选中文件写 plugin_data 绑定，其参数才在下方展开。
    // WGSL 与内置效果同处一个菜单，不再另起独立下拉行。
    let icons = vec![style_menu(f, d, "effect", None, Icon::Dot), add_effect_menu(f)];

    // WGSL 材质：只在「已绑定」时于下方就地展开参数输入框（选中文件由上面的 `+` 菜单写入）。
    // 空态（0 效果、未绑定）连参数行都不画，Effects 节就是一行置灰头部 + 可点的 `+`。
    let wgsl_rows = wgsl_material_rows(f, d, col, cx);

    let Some(rows) = d.effects.clone().filter(|v| !v.is_empty()) else {
        // 没有普通效果、但已绑定材质时，Effects 节仍要画出来以承载其参数行；
        // 未绑定时这里是空节（置灰头部 + 可点的 `+`，`+` 菜单里能选内置效果或 wgsl 文件）。
        if !wgsl_rows.is_empty() {
            let mut sec = PanelSection::new(tr("Effects")).header_trail(icon_row(icons));
            if let Some(chip) = style_chip(f, d.style_chips[2].as_ref(), "effect") {
                sec = sec.child(chip);
            }
            for r in wgsl_rows {
                sec = sec.child(r);
            }
            return Section::Full(sec);
        }
        return Section::empty(tr("Effects"), icon_row(icons));
    };
    let mut sec = PanelSection::new(tr("Effects")).header_trail(icon_row(icons));
    // egui `effects.rs`：`style_picker::chip(StyleSlot::Effect)` 紧跟在 header 后面。
    if let Some(chip) = style_chip(f, d.style_chips[2].as_ref(), "effect") {
        sec = sec.child(chip);
    }
    // 材质行放在普通效果之前：选中文件→下方参数展开，视觉上和「加一条效果→展开设置」一致。
    for r in wgsl_rows {
        sec = sec.child(r);
    }
    for row in rows.iter().rev() {
        let types: Vec<(String, &'static str, Value)> = figcraft_doc::Effect::TYPES
            .iter()
            .map(|(label, ty)| {
                let mut e = row.effect.clone();
                if let Some(o) = e.as_object_mut() {
                    o.insert("type".into(), json!(ty));
                }
                (label.to_string(), "effect.set", json!({ "index": row.index, "effect": e }))
            })
            .collect();
        let dd = pick_dropdown(f, row.label.clone(), col, types);
        let mut e2 = row.effect.clone();
        if let Some(o) = e2.as_object_mut() {
            o.insert("visible".into(), json!(!row.visible));
        }
        // 显隐按钮必须反映该 effect 当前的 `visible`，否则用户看不出点它会在显示/隐藏之间
        // 切到哪一边（egui `effects.rs` 用 Eye / EyeOff）。原先这里写了 Dot / Close——图标集
        // 里明明有 `Icon::Eye` / `Icon::EyeOff`，用圆点冒充眼睛是错的，已改正。
        let eye = icon_cmd(if row.visible { Icon::Eye } else { Icon::EyeOff }, "effect.set", json!({ "index": row.index, "effect": e2 }), f, false);
        let minus = icon_cmd(Icon::Minus, "effect.remove", json!({ "index": row.index }), f, false);
        let mut trailing = vec![eye, minus];
        if let Some((layer, keyed, q)) = &keyframe {
            use figcraft_doc::anim::motion::AnimProperty;
            let ty = row.effect.get("type").and_then(|v| v.as_str()).unwrap_or_default();
            let prop = match ty {
                "dropShadow" | "innerShadow" => AnimProperty::ShadowBlur,
                _ => AnimProperty::BlurRadius,
            };
            let on = keyed.contains(&prop);
            let queue = (*q).clone();
            let layer = *layer;
            trailing.push(
                IconButton::new(if on { Icon::Component } else { Icon::Instance })
                    .on_click(move |_, _, _| queue.push(Act::ToggleKey(layer, prop))),
            );
        }
        sec = sec.child(field_row_full_trailing(dd, trailing));
    }
    Section::Full(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Selection colors —— `selection_colors.rs::section`
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// lizard `selection_colors.rs::header`：折叠时最多显示 3 个色块，多的收进 `+N`。
fn collapsed_swatch_count(n: usize) -> usize {
    n.min(3)
}
fn collapsed_extra(n: usize) -> usize {
    n.saturating_sub(3)
}

/// lizard `Id::new(("selection_colors_open", ds.uid))` 的 GPUI 等价物：
/// 状态按文档 uid 存，缺省折叠，点一次只翻转当前文档那一档。
type OpenByDoc = std::collections::HashMap<u64, bool>;
fn open_for_doc(map: &OpenByDoc, uid: u64) -> bool {
    *map.get(&uid).unwrap_or(&false)
}
fn toggle_doc(map: &mut OpenByDoc, uid: u64) {
    let next = !open_for_doc(map, uid);
    map.insert(uid, next);
}

/// 折叠头部的 16 pt 色块（lizard `widgets::paint_swatch` 的纯色分支）。
///
/// 冻结的 `widgets::paint_swatch` **无条件**先画 `checker_rounded`，四角会露出白色/浅色
/// 像素，看上去像「四个圆角溢出色块」（用户 2026-11-11 指出）。lizard 对
/// `opaque_solid`（`opacity>=1 && color.a>=1`）**根本不画棋盘**，只画一块圆角 3 的色，
/// 最后 `rect_stroke(rect, same(3), 1pt border, Inside)`。这里在业务侧照该规则重画，
/// 不改冻结组件；渐变/图片/图案仍回退到原组件，外层圆角+裁剪兜住溢出。
fn collapsed_swatch(paint: figcraft_color::Paint, side: f32) -> gpui::AnyElement {
    let t = tokens();
    let solid = match &paint.kind {
        PaintKind::Solid { color } => Some(color.clone()),
        _ => None,
    };
    let Some(color) = solid else {
        // 渐变 / 图片 / 图案沿用冻结组件的全部行为，外层圆角 + 裁剪兜住四角溢出。
        return div()
            .flex_none()
            .w(pt(side))
            .h(pt(side))
            .rounded(px(3.0))
            .overflow_hidden()
            .child(swatch(paint, side))
            .into_any_element();
    };
    let opaque = rgba(crate::widgets::swatch::color32(color.with_alpha(1.0)));
    let tinted = rgba(crate::widgets::swatch::color32(color.with_alpha(color.a * paint.opacity)));
    let is_opaque = paint.opacity >= 0.999 && color.a >= 0.999;
    // 用 GPUI 原生 `div` 背景，而不是 canvas 里自绘 `paint_quad`：quad 的圆角抗锯齿会在
    // 四角留下彩色半透明像素，正是「四个圆角溢出色块」。半透明时直接把带 alpha 的色
    // 叠在面板白底上（lizard 左不透明/右带 alpha 的 Figma 分半在 16pt 上无差别）。
    // ⚠️ 不画 `border_1()`：1pt 浅灰描边压在圆角填充的抗锯齿上，四角会出现一圈
    // 「浅色虚线尖角」（用户 2026-11-11 指出的「四个圆角溢出色块」）。lizard 的
    // 1pt Inside 描边在 16pt 色块上肉眼几乎不可见，去掉后与原文观感一致且四角干净。
    div()
        .flex_none()
        .w(pt(side))
        .h(pt(side))
        .rounded(px(3.0))
        .bg(if is_opaque { opaque } else { tinted })
        .into_any_element()
}

/// Selection colors 节（`lizard selection_colors.rs::section`）的折叠样式 1:1：
///
/// * 默认**折叠**：40 pt 可点头部，标题 `text_secondary`；悬停时提亮并显示左侧 chevron；
/// * 折叠态右侧并排最多 3 个 16 pt 色块（间距 8），再多一个 `+N`；
/// * 点标题展开，才画原来的颜色行；展开态 chevron 朝下。
///
/// 头部不用冻结的 `SectionHeader`（它没有整行点击 / hover 状态），但展开后的每一行仍由
/// `field_row_full_trailing` 构造，走同一套统一网格。
pub(crate) fn selection_colors(
    f: &RightFields,
    d: &PanelData,
    row_w: f32,
    _col: f32,
    n: &mut usize,
    _window: &mut Window,
    cx: &mut App,
) -> Option<gpui::AnyElement> {
    if d.selection_colors.len() < 2 {
        return None;
    }
    let t = tokens();
    // lizard `Id::new(("selection_colors_open", ds.uid))`：开合状态**按文档 uid 分档**，
    // 切到另一个文档时各自保留自己的状态，缺省折叠。
    let uid = d.doc_uid;
    let open = open_for_doc(&f.selection_colors_open.borrow(), uid);
    let hovered = *f.selection_colors_hover.borrow();
    // lizard：`hovered && dark` → 白；`hovered` → text；否则 text_secondary。
    let ink = if hovered && t.dark {
        Color32::WHITE
    } else if hovered {
        t.text
    } else {
        t.text_secondary
    };

    // ── 头部 ────────────────────────────────────────────────────────────────
    // lizard `theme::property_section_title()` = medium(ui_font_size(10.5))；GPUI 的
    // `SectionHeader` 把这一档固化成 TITLE_PT=13 / MEDIUM / 1.25 行高。折叠节标题必须
    // 与上面的「填充 / 效果 / 描边」用同一套排版，不能自己写一个 10.5 pt 的小字。
    let title = div()
        .flex_none()
        .text_size(pt(TITLE_PT))
        .line_height(pt(TITLE_PT * 1.25))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgba(ink))
        .child(tr("Selection colors"));
    // lizard 从 `rect.right() - PAD` 起从右往左摆：`+N` 最右，色块在其左，间隔 8 pt。
    let mut right = div().flex_none().flex().flex_row().items_center().gap(pt(8.0));
    if !open {
        for row in d.selection_colors.iter().take(collapsed_swatch_count(d.selection_colors.len())) {
            let color = figcraft_color::Color::from_hex(&row.hex).unwrap_or(figcraft_color::Color::WHITE);
            let paint = serde_json::from_value::<figcraft_color::Paint>(row.paint.clone())
                .unwrap_or_else(|_| figcraft_color::Paint::solid(color));
            right = right.child(collapsed_swatch(paint, 16.0));
        }
        let extra = collapsed_extra(d.selection_colors.len());
        if extra > 0 {
            // lizard `regular(11.0)` → ui_font_size(11)=12.94，GPUI 对应 TITLE_PT 这一档
            // （字重是 regular，不是标题的 medium），颜色仍是 text_secondary。
            right = right.child(
                div()
                    .flex_none()
                    .text_size(pt(TITLE_PT))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(format!("+{extra}"))),
            );
        }
    }
    let open_cell = f.selection_colors_open.clone();
    let hover_cell = f.selection_colors_hover.clone();
    let cmds = f.cmds.clone();
    let mut head = div()
        .id("selection-colors-header")
        .relative()
        .flex_none()
        .w_full()
        .h(pt(HEADER_H))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .pl(pt(PAD))
        // lizard header 右侧也从 `rect.right() - PAD` 起算，左右都是 16 pt。
        .pr(pt(PAD))
        .cursor_pointer()
        .child(title)
        .child(right)
        .on_click(move |_e, _window, _cx| {
            let mut by_doc = open_cell.borrow_mut();
            toggle_doc(&mut by_doc, uid);
            drop(by_doc);
            cmds.wake();
        })
        .on_hover(move |hovering: &bool, window: &mut Window, _cx: &mut App| {
            *hover_cell.borrow_mut() = *hovering;
            window.refresh();
        });
    if hovered {
        // 悬停才画：10 pt chevron 落在 `PAD - 10` 的位置，展开朝下 / 折叠朝右。
        let icon = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        head = head.child(
            div()
                .absolute()
                .left(pt(PAD - 15.0))
                .top(pt((HEADER_H - 10.0) / 2.0))
                .w(pt(10.0))
                .h(pt(10.0))
                .child(crate::widgets::icon::paint(icon, rgba(ink), 10.0)),
        );
    }
    let mut sec = div().flex().flex_col().w_full().child(head);
    if !open {
        return Some(sec.into_any_element());
    }

    // ── 展开态：原来的颜色行（lizard 在 `add_enabled_ui(!interacting, …)` 里画）──
    let mut body = div().flex().flex_col().w_full().pb(pt(SECTION_BOTTOM));
    for row in d.selection_colors.iter() {
        let targets = row.targets.clone();
        let hex_slot = slot(n);
        let op_slot = slot(n);
        let hex_ed = f.row_field(
            cx,
            hex_slot,
            &row.hex,
            RowEditCtx {
                cmd: "selection.setPaint",
                params: json!({ "targets": targets }),
                key: "paint",
                scale: 1.0,
                string_value: true,
                paint: None,
                // 改 hex 也要 clone 整组 Paint 再换 `color`（egui `section`）：只发一个
                // `"#RRGGBB"` 会让 `paint_of` 造出 opacity=1 的新 paint，把透明度抹掉。
                nest: None, nest_key: None, paint_color: Some(row.paint.clone()),
            },
        );
        let op_ed = f.row_field(
            cx,
            op_slot,
            &format!("{}", row.opacity.round() as i64),
            RowEditCtx {
                cmd: "selection.setPaint",
                params: json!({ "targets": targets }),
                key: "opacity",
                scale: 100.0,
                string_value: false,
                paint: Some(row.paint.clone()),
                nest: None, nest_key: None, paint_color: None,
            },
        );
        // egui `section`: 纯色显示 hex 且可编辑；渐变/图片只显示名字，点它开取色器，
        // 所以 hex 编辑器必须只读，否则会把一个名字当成颜色提交。
        if !row.solid {
            hex_ed.update(cx, |te, _| te.set_readonly(true));
        }
        let color = figcraft_color::Color::from_hex(&row.hex).unwrap_or(figcraft_color::Color::WHITE);
        // egui `section` 直接拿 `group.paint` 画色块：这里也必须用该组原本的完整 Paint，
        // 否则 `Paint::solid(color)` 会把透明度抹成 1、把渐变/图片退成纯色（甚至白色）。
        let paint = serde_json::from_value::<figcraft_color::Paint>(row.paint.clone()).unwrap_or_else(|_| figcraft_color::Paint::solid(color));
        let text =
            if row.solid { ColorText::Hex(SharedString::from(row.hex.clone())) } else { ColorText::Label(SharedString::from(row.hex.clone())) };
        // 取色器的初值同样取自完整 paint（渐变取其代表色），不是从 hex 反解。
        let current = paint.solid_color().map(crate::widgets::swatch::color32).unwrap_or(Color32::BLACK);
        let cf = ColorField::new(cx, paint, text, row_w, hex_ed, op_ed);
        // swatch 点击开取色器（`selection_colors.rs::toggle`）：`selection.setPaint` 用 `paint` 键。
        let cf = color_pick(f, cf, current, "selection.setPaint", json!({ "targets": targets }), "paint", Some(row.paint.clone()));
        // 选区颜色组绑定到变量时同样走 `Field-color / Disabled`：整框可点换绑，24 pt
        // 尾插槽换成解绑按钮（`selection.detachColor`）。换绑菜单只列这组作用域内的变量。
        let var_names: Vec<String> = row.variables.iter().map(|(name, _)| name.clone()).collect();
        let cf = match row.linked.as_deref() {
            Some(name) => bound_color_field(
                f,
                cf,
                name,
                "fill",
                &var_names,
                Some(targets.clone()),
                ("selection.detachColor", json!({ "targets": targets.clone() })),
            ),
            None => cf,
        };
        let bind = selection_bind(f, row, d).into_any_element();
        let select = icon_cmd(Icon::Search, "selection.selectColor", json!({ "targets": targets }), f, false).into_any_element();
        // 两个 24 pt 尾槽：Styles/变量（绑定态留空占位） + 选中同色图层。
        body = body.child(field_row_full_trailing(cf, vec![bind, select]).into_element());
    }
    Some(sec.child(body).into_any_element())
}

#[cfg(test)]
mod selection_colors_tests {
    use super::{collapsed_extra, collapsed_swatch_count, open_for_doc, toggle_doc, OpenByDoc};

    /// lizard `header`：折叠时 3 个色块，多出来的收进 `+N`。
    #[test]
    fn collapsed_header_shows_three_swatches_and_the_rest_as_a_count() {
        // 少于两组根本不画这一节，但纯函数仍给出安全值。
        assert_eq!(collapsed_swatch_count(0), 0);
        assert_eq!(collapsed_extra(0), 0);
        for n in [2, 3] {
            assert_eq!(collapsed_swatch_count(n), n);
            assert_eq!(collapsed_extra(n), 0);
        }
        assert_eq!(collapsed_swatch_count(7), 3);
        assert_eq!(collapsed_extra(7), 4);
        assert_eq!(collapsed_extra(4), 1);
    }

    /// lizard `Id::new(("selection_colors_open", ds.uid))`：两个文档各自记住开合状态。
    #[test]
    fn open_state_is_keyed_by_document_uid() {
        let mut map = OpenByDoc::new();
        // 缺省折叠。
        assert!(!open_for_doc(&map, 1));
        assert!(!open_for_doc(&map, 2));
        toggle_doc(&mut map, 1);
        assert!(open_for_doc(&map, 1));
        assert!(!open_for_doc(&map, 2), "opening doc 1 must not open doc 2");
        toggle_doc(&mut map, 2);
        assert!(open_for_doc(&map, 1));
        assert!(open_for_doc(&map, 2));
        toggle_doc(&mut map, 1);
        assert!(!open_for_doc(&map, 1));
        assert!(open_for_doc(&map, 2), "closing doc 1 must not close doc 2");
    }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Layout grid —— `effects.rs::grids`
// ───────────────────────────────────────────────────────────────────────────────────────────────

pub(crate) fn grids(f: &RightFields, d: &PanelData, col: f32) -> Section {
    // `[Styles, Plus]`；egui `effects.rs::grids` 的 `+` 直接加一条默认网格（`grid.add {ids}`），
    // 类型再在行内下拉改。GPUI 不要在 header 弹类型菜单，直接加默认网格（与源码一致）。
    let icons = vec![style_menu(f, d, "grid", None, Icon::Dot), icon_cmd(Icon::Plus, "grid.add", json!({}), f, false)];
    // 空 / Mixed：只有置灰头部（同 Fill / Stroke）。
    let Some(rows) = d.grids.clone().filter(|v| !v.is_empty()) else {
        return Section::empty(tr("Layout grid"), icon_row(icons));
    };
    let mut sec = PanelSection::new(tr("Layout grid")).header_trail(icon_row(icons));
    // egui `effects.rs::grids`：`style_picker::chip(StyleSlot::Grid)` 紧跟 header。
    if let Some(chip) = style_chip(f, d.style_chips[3].as_ref(), "grid") {
        sec = sec.child(chip);
    }
    for row in rows.iter().rev() {
        let kinds: Vec<(String, &'static str, Value)> = ["grid", "columns", "rows", "diagonal", "rings"]
            .into_iter()
            .map(|k| {
                let mut arr = row.grids.clone();
                if let Some(item) = arr.as_array_mut().and_then(|a| a.get_mut(row.index)) {
                    if let Some(o) = item.as_object_mut() {
                        o.insert("kind".into(), json!(k));
                    }
                }
                (k.to_string(), "node.set", json!({ "props": { "grids": arr } }))
            })
            .collect();
        let dd = pick_dropdown(f, row.label.clone(), col, kinds);
        let mut arr = row.grids.clone();
        if let Some(item) = arr.as_array_mut().and_then(|a| a.get_mut(row.index)) {
            if let Some(o) = item.as_object_mut() {
                o.insert("visible".into(), json!(!row.visible));
            }
        }
        // 同 effect 行：显隐图标按该 grid 的 `visible` 切换，不能恒定一个 Dot —— 而且这里
        // 本来就该是 `Eye` / `EyeOff`（egui `effects.rs::grids`），不是圆点/叉。
        let eye = icon_cmd(if row.visible { Icon::Eye } else { Icon::EyeOff }, "node.set", json!({ "props": { "grids": arr } }), f, false);
        let minus = icon_cmd(Icon::Minus, "grid.remove", json!({ "index": row.index }), f, false);
        sec = sec.child(field_row_full_trailing(dd, vec![eye, minus]));
    }
    Section::Full(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Export —— `export_panel.rs::section` / `setting_row`
// ───────────────────────────────────────────────────────────────────────────────────────────────

pub(crate) fn export(f: &RightFields, d: &PanelData, col: f32, n: &mut usize, cx: &mut App) -> Section {
    // egui `export_panel.rs::section`: `mixed = shared.is_none() && sel.len() > 1`。
    // mixed 时 `+` 是「整体替换成一条默认设置」（set_props exports），不是 export.add；
    // tip 也从 "Add export setting" 变成 "Replace mixed export settings"。
    let mixed = d.exports.is_none();
    let replace = json!({ "props": { "exports": [figcraft_doc::ExportSetting::default()] } });
    let plus =
        if mixed { icon_cmd(Icon::Plus, "node.set", replace.clone(), f, false) } else { icon_cmd(Icon::Plus, "export.add", json!({}), f, false) };
    // 空 / Mixed：只有置灰头部（同 Fill / Stroke）。mixed 时 `+` 已经是「整体替换成一条默认设置」。
    let Some(rows) = d.exports.clone().filter(|v| !v.is_empty()) else {
        return Section::empty(tr("Export"), plus);
    };
    let mut sec = PanelSection::new(tr("Export")).header_trail(plus);
    let tick = |label: &str, on: bool| format!("{label}{}", if on { " ✓" } else { "" });
    for row in rows.iter() {
        let index = row.index;
        // suffix / format / remove —— 照 `setting_row` 顺序。size 与高级选项放进 Dots 菜单，
        // 避免往同一行再塞一个 `w_full` 下拉把 suffix 挤没（列宽由统一网格负责）。
        let suffix_slot = slot(n);
        let suffix = f.row_field(
            cx,
            suffix_slot,
            &row.suffix,
            RowEditCtx {
                cmd: "export.update",
                params: json!({ "index": index }),
                key: "suffix",
                scale: 1.0,
                string_value: true,
                paint: None,
                nest: None, nest_key: None, paint_color: None,
            },
        );
        let formats: Vec<(String, &'static str, Value)> =
            [("PNG", "png"), ("JPG", "jpg"), ("SVG", "svg"), ("PDF", "pdf"), ("WebP", "webp"), ("XML", "xml")]
                .into_iter()
                .map(|(l, ext)| (l.to_string(), "export.update", json!({ "index": index, "format": ext })))
                .collect();
        let dd = pick_dropdown(f, row.format_label, col, formats);

        // Dots（`export_panel.rs::advanced_options` + `print_options`）：之前这一行只有
        // format/suffix/remove，size、contentsOnly、SVG/PDF/XML 选项全都没有按钮可点。
        let mut opts: Vec<(String, &'static str, Value)> = Vec::new();
        if row.is_raster {
            // egui `setting_row`: SCALES 的 label + 512w/512h，`export.update {"size": label}`。
            for v in figcraft_doc::ExportSize::SCALES {
                let label = figcraft_doc::ExportSize::Scale { value: v }.label();
                opts.push((format!("Size {label}"), "export.update", json!({ "index": index, "size": label })));
            }
            for s in ["512w", "512h"] {
                opts.push((format!("Size {s}"), "export.update", json!({ "index": index, "size": s })));
            }
        }
        opts.push((
            tick(tr("Ignore overlapping layers"), row.contents_only),
            "export.update",
            json!({ "index": index, "contentsOnly": !row.contents_only }),
        ));
        opts.push((
            tick(tr("Include bounding box"), row.include_bounding_box),
            "export.update",
            json!({ "index": index, "includeBoundingBox": !row.include_bounding_box }),
        ));
        if row.format == "svg" {
            opts.push((
                tick(tr("Outline text"), row.svg_outline_text),
                "export.update",
                json!({ "index": index, "svgOutlineText": !row.svg_outline_text }),
            ));
            opts.push((
                tick(tr("Include id attribute"), row.svg_id_attribute),
                "export.update",
                json!({ "index": index, "svgIdAttribute": !row.svg_id_attribute }),
            ));
            opts.push((
                tick("Simplify strokes", row.svg_simplify_stroke),
                "export.update",
                json!({ "index": index, "svgSimplifyStroke": !row.svg_simplify_stroke }),
            ));
        }
        if row.format == "xml" {
            opts.push((
                tick(tr("Include id attribute"), row.svg_id_attribute),
                "export.update",
                json!({ "index": index, "svgIdAttribute": !row.svg_id_attribute }),
            ));
            for (label, key) in [
                ("Compatibility: best effort", "bestEffort"),
                ("Compatibility: exact only", "exact"),
                ("Compatibility: ignore unsupported", "ignore"),
            ] {
                opts.push((tick(label, row.vd_compat == key), "export.update", json!({ "index": index, "vdCompat": key })));
            }
        }
        if row.format == "pdf" {
            // egui `print_params`: bleed / cropMarks / bleedMarks / cmyk 都走 `export.update`。
            let bleed = if row.bleed > 0.0 { json!(0) } else { json!("3mm") };
            opts.push((tick("3 mm bleed", row.bleed > 0.0), "export.update", json!({ "index": index, "bleed": bleed })));
            opts.push((tick(tr("Crop marks"), row.crop_marks), "export.update", json!({ "index": index, "cropMarks": !row.crop_marks })));
            opts.push((tick(tr("Bleed marks"), row.bleed_marks), "export.update", json!({ "index": index, "bleedMarks": !row.bleed_marks })));
            opts.push((tick("CMYK", row.cmyk), "export.update", json!({ "index": index, "cmyk": !row.cmyk })));
        }
        let dots = menu_icon(f, Icon::Menu, 220.0, opts, false);
        let minus = icon_cmd(Icon::Minus, "export.remove", json!({ "index": index }), f, false);
        let line = div().flex().flex_row().items_center().gap(pt(4.0)).child(dd).child(suffix).child(dots).child(minus);
        sec = sec.child(field_row_full(line));
    }
    // 「Export N layers」（egui `wide_button` + `export_layers`）：egui 有普通导出与
    // Inspect 资源导出两条路径，Web 端多个文件打成 zip。原来只有一个硬编码 `{}` 的按钮。
    if !rows.is_empty() {
        let export_all = menu_icon(
            f,
            Icon::Plus,
            220.0,
            vec![
                (tr("Export layers").into(), "file.exportLayers", json!({})),
                ("Export assets".into(), "file.exportLayers", json!({ "assets": true })),
                ("Export layers as ZIP".into(), "file.exportLayers", json!({ "zip": true })),
                ("Export assets as ZIP".into(), "file.exportLayers", json!({ "assets": true, "zip": true })),
            ],
            false,
        );
        sec = sec.child(field_row(vec![SharedString::from("Export all layers")], vec![export_all]));
    }
    Section::Full(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Variable modes / Section / Header / Page
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// Appearance 的 header 图标（egui `paints.rs::appearance`：Eye/Variable/Blend）。
pub(crate) fn appearance(
    f: &RightFields,
    d: &PanelData,
    row_w: f32,
    opacity: &gpui::Entity<crate::widgets::TextField>,
    corner: &gpui::Entity<crate::widgets::TextField>,
) -> PanelSection {
    let visible = d.visible.unwrap_or(true);
    let eye = icon_cmd(if visible { Icon::Eye } else { Icon::EyeOff }, "object.toggleVisible", json!({}), f, false);
    let bind = variables_window_button(
        f,
        Icon::Variable,
        d.bool_variables.iter().map(|v| (v.clone(), "variable.bind", json!({ "field": "visible", "variable": v }))).collect(),
    );
    // egui `paints.rs::appearance`：blend 按钮（索引 2）**不推命令**——它只翻转本地 `blend_open`
    // 标记来展开/收起下方的 blend-mode 下拉行；blend 值由那一行里的下拉选定（→ `node.set`）。
    // GPUI 必须一致：点这个按钮只是切换显隐，而不是直接弹 `node.set` 菜单。
    let blend_cmds = f.cmds.clone();
    let blend_open_ref = f.blend_open.clone();
    let blend_is_open = *blend_open_ref.borrow();
    let blend_open_for_click = blend_open_ref.clone();
    let blend = IconButton::new(Icon::Settings).size(24.0).active(blend_is_open).on_click(move |_e, _w, _cx| {
        let open = !*blend_open_for_click.borrow();
        *blend_open_for_click.borrow_mut() = open;
        blend_cmds.wake();
    });
    let mut sec = PanelSection::new(tr("Appearance"))
        .header_trail(icon_row(vec![eye, bind, blend]))
        .child(sublabel_row2(tr("Opacity"), tr("Corner radius")))
        .child(field_row(vec![opacity.clone(), corner.clone()], Vec::<IconButton>::new()));
    // 仅当 header 的 blend 开关打开时才显示 blend-mode 下拉行（egui `blend_open`）。
    if *f.blend_open.borrow() {
        // 显示走 `tr`，**写回值必须是英文 serde 名**（`blend` 只认 `Normal` / `Multiply` …）。
        let rows: Vec<(String, &'static str, Value)> = crate::panels::blend_rows()
            .into_iter()
            .map(|r| (tr(&r.label).to_string(), "node.set", json!({ "props": { "blend": r.label } })))
            .collect();
        let shown = tr(&f.blend_label.borrow()).to_string();
        sec = sec.child(field_row_full(pick_dropdown(f, shown, row_w, rows)));
    }
    sec
}

/// Typography 的 header 图标（egui `typography.rs`：Variable/Styles）。
/// Typography（egui `typography.rs` → `typography/fields.rs::font_rows` + `paragraph_rows`）。
///
/// 逐行照搬 egui：字体族 / 字重 / 字号 / 行高 / 字距 / 段落间距 + 自动缩放 + 水平·垂直对齐。
/// 写回都是 `node.set {"props": {…}}`（等价 egui 的 `apply_style` → `commit_text`）。
///
/// ⚠️ 与 egui 的一处差异：egui 的自动缩放与两组对齐是 `segmented`，图标是 `Hug`/`LayoutV`/
/// `Fixed`/`Dots`/`TextAlign*`，但冻结的 [`Icon`] 集里**没有这些图标**，而 `segmented` 只收
/// `(Icon, tooltip)`。按「缺组件不新建、仅此处用就内联」的约定，这里改用纯文本 `Dropdown`。
pub(crate) fn typography(f: &RightFields, d: &PanelData) -> PanelSection {
    let t = d.text.clone().unwrap_or_default();
    let var = variables_window_button(
        f,
        Icon::Variable,
        d.text_variables.iter().map(|v| (v.clone(), "variable.bind", json!({ "field": "characters", "variable": v }))).collect(),
    );
    let mut sec = PanelSection::new(tr("Typography")).header_trail(icon_row(vec![var, style_menu(f, d, "text", None, Icon::Dot)]));

    // ── 字体族（整行）──
    let families: Vec<(String, &'static str, Value)> = figcraft_text::family_names()
        .into_iter()
        .map(|fam| {
            let q = json!({ "props": { "fontFamily": fam.clone() } });
            (fam, "node.set", q)
        })
        .collect();
    sec = sec.child(field_row_full(pick_dropdown(
        f,
        t.font_family.clone().unwrap_or_else(|| tr("Mixed").into()),
        240.0,
        families,
    )));

    // ── 字重 / 样式 + 字号 ──
    // 样式列表取决于字体族（egui `family_styles(fam)`）；没选族时 egui 提示 "Pick one font
    // family first"，这里用下拉的当前标签表达同一件事（菜单为空即点不开选项）。
    let styles: Vec<(String, &'static str, Value)> = match &t.font_family {
        Some(fam) => figcraft_text::family_styles(fam)
            .into_iter()
            .map(|s| {
                let q = json!({ "props": { "fontStyle": s.clone() } });
                (s, "node.set", q)
            })
            .collect(),
        None => Vec::new(),
    };
    let style_label = match t.font_style.clone() {
        Some(s) => s,
        None if styles.is_empty() => "Pick a font family".to_string(),
        None => tr("Mixed").to_string(),
    };
    sec = sec.child(field_row(
        vec![pick_dropdown(f, style_label, 160.0, styles).into_any_element(), f.font_size.clone().into_any_element()],
        Vec::<IconButton>::new(),
    ));

    // ── 行高 + 字距（池化字段，值由 `set_text_values` 每帧灌入）──
    sec = sec.child(field_row(
        vec![f.line_height.clone().into_any_element(), f.letter_spacing.clone().into_any_element()],
        Vec::<IconButton>::new(),
    ));

    // ── 段落间距（P）──
    // 用户 2026-11-11：文字段只留 LH / LS / P；自动缩放（Auto width…）从这段移除，
    // 选中文字时它属于 Layout 段的「调整大小 + 三格选择器」。
    sec = sec.child(field_row(
        vec![f.paragraph_spacing.clone().into_any_element()],
        Vec::<IconButton>::new(),
    ));

    // ── 水平 / 垂直对齐：两个通用 `Segmented`（egui `paragraph_rows` 的两个分段控件）──
    //
    // 图标集里没有 `TypeDialogJustify`，所以这里只放 3 格；`justified`（两端对齐）仍然是合法值，
    // 这时两格都不抬起 —— 正好用上 Segmented 的「无选中」状态（设计文件 `State=Rest`），
    // 而且 Justify 在 Type settings 的 Alignment 行里可选。
    let h_cmds = f.cmds.clone();
    let h = Segmented::icons(&[
        (Icon::AlignLeft, tr("Align left")),
        (Icon::AlignHCenter, tr("Align center")),
        (Icon::AlignRight, tr("Align right")),
    ])
    .active(match t.align_h {
        Some("left") => Some(0),
        Some("center") => Some(1),
        Some("right") => Some(2),
        _ => None,
    })
    .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
        const V: [&str; 3] = ["left", "center", "right"];
        h_cmds.push(Cmd { id: "node.set", params: Some(json!({ "props": { "textAlign": V[i.min(2)] } })) });
    });
    let v_cmds = f.cmds.clone();
    let v = Segmented::icons(&[
        (Icon::AlignTop, tr("Align top")),
        (Icon::AlignVCenter, tr("Align middle")),
        (Icon::AlignBottom, tr("Align bottom")),
    ])
    .active(match t.align_v {
        Some("top") => Some(0),
        Some("middle") => Some(1),
        Some("bottom") => Some(2),
        _ => None,
    })
    .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
        const V: [&str; 3] = ["top", "middle", "bottom"];
        v_cmds.push(Cmd { id: "node.set", params: Some(json!({ "props": { "textAlignVertical": V[i.min(2)] } })) });
    });
    sec.child(field_row(
        vec![h.into_any_element(), v.into_any_element()],
        // egui `paragraph_rows` 末尾那个 `Icon::TypeSettings` 按钮：点开详细排版设置
        // （`type_settings_panel`，Basics / Details / Variable）。原实现把这一行做成了「两个
        // 对齐下拉」就结束，缺了这个入口（用户 2026-11-11 指出）。
        vec![type_settings_button(f, d)],
    ))
}

/// 「Type settings」入口（egui `typography.rs::paragraph_rows` 末尾的 `Icon::TypeSettings`）。
///
/// egui 把它放在对齐那一行的行尾、24 pt；点开的是一个**独立的浮层**（带页签与关闭按钮），
/// 不是下拉菜单 —— 所以这里走通用 `WindowState`（`popovers::type_settings`）。
///
/// 图标用 `Icon::Menu`（三条横线 =「更多选项」）：egui 的 `Icon::TypeSettings` 字形在冻结的
/// 图标集里没有对应项，而 `Icon::Settings` 目前只是占位圆点（不是齿轮），画出来认不出。
fn type_settings_button(f: &RightFields, d: &PanelData) -> IconButton {
    let popups = f.popups();
    let queue = f.queue();
    let data = d.clone();
    IconButton::new(Icon::Menu).size(24.0).on_click(move |_e, window, cx: &mut App| {
        popups.request(crate::popovers::WindowState::type_settings(queue.clone(), &data, cx, window.mouse_position()));
    })
}

/// 矢量编辑模式下选中点的面板（egui `vector_panel.rs::points`）。
///
/// 只在 `data.vector` 存在时出现，**替换** Position / Auto layout / Arc / Star。`count == 0` 时
/// 只有一行提示（egui `widgets::note`）。写回全部走 `vector.edit.*`，引擎按当前点选区执行。
pub(crate) fn vector_points(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize) -> PanelSection {
    use figcraft_doc::{HandleMirroring, StrokeCap, StrokeJoin};
    let Some(v) = d.vector.as_ref() else { return PanelSection::new(tr("Points")) };
    // Variable width 工具：右栏只画 Width point 节（egui `points` 开头同款分支）。
    if let Some(w) = &v.width_point {
        let round = |x: f64| x;
        let mut sec = PanelSection::new(tr("Width point"));
        sec = sec.child(sublabel_row2(tr("Position"), tr("Width")));
        let tf = f.row_field(
            cx,
            slot(n),
            &format!("{}", w.t),
            RowEditCtx {
                cmd: "vector.edit.widthPoint",
                params: json!({ "index": w.index }),
                key: "t",
                scale: 100.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let wf = f.row_field(
            cx,
            slot(n),
            &format!("{}", round(w.width)),
            RowEditCtx {
                cmd: "vector.edit.widthPoint",
                params: json!({ "index": w.index }),
                key: "width",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        sec = sec.child(field_row(vec![tf, wf], Vec::<IconButton>::new()));
        sec = sec.child(sublabel_row2(tr("Left side"), tr("Right side")));
        let lf = f.row_field(
            cx,
            slot(n),
            &format!("{}", round(w.left)),
            RowEditCtx {
                cmd: "vector.edit.widthPoint",
                params: json!({ "index": w.index, "side": "left" }),
                key: "width",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        let rf = f.row_field(
            cx,
            slot(n),
            &format!("{}", round(w.right)),
            RowEditCtx {
                cmd: "vector.edit.widthPoint",
                params: json!({ "index": w.index, "side": "right" }),
                key: "width",
                scale: 1.0,
                string_value: false,
                paint: None,
                nest: None,
                nest_key: None,
                paint_color: None,
            },
        );
        return sec.child(field_row(vec![lf, rf], Vec::<IconButton>::new()));
    }
    let mut sec = PanelSection::new(if v.count > 1 { tr("Points") } else { tr("Point") });
    if v.count == 0 {
        return sec.child(field_row_full(
            div().text_size(pt(11.0)).text_color(rgba(tokens().text_secondary)).child(SharedString::from(
                tr("Select points to edit their position, radius and mirroring"),
            )),
        ));
    }
    sec = sec
        .child(sublabel_row(tr("Position")))
        .child(field_row(vec![f.vpt_x.clone(), f.vpt_y.clone()], Vec::<IconButton>::new()));
    if v.w.is_some() || v.h.is_some() {
        sec = sec
            .child(sublabel_row(tr("Dimensions")))
            .child(field_row(vec![f.vpt_w.clone(), f.vpt_h.clone()], Vec::<IconButton>::new()));
    }
    sec = sec
        .child(sublabel_row(tr("Corner radius")))
        .child(field_row(vec![f.vpt_radius.clone()], Vec::<IconButton>::new()));

    // Handle mirroring（egui `menu_popup` + `HandleMirroring::id`）。
    let mirror_label = match v.mirroring {
        Some(HandleMirroring::None) => tr("No mirroring"),
        Some(HandleMirroring::Angle) => tr("Mirror angle"),
        Some(HandleMirroring::AngleAndLength) => tr("Mirror angle and length"),
        None => tr("Mixed"),
    };
    let mirror_rows: Vec<(String, &'static str, Value)> = [HandleMirroring::None, HandleMirroring::Angle, HandleMirroring::AngleAndLength]
        .into_iter()
        .map(|m| {
            let label = match m {
                HandleMirroring::None => tr("No mirroring"),
                HandleMirroring::Angle => tr("Mirror angle"),
                HandleMirroring::AngleAndLength => tr("Mirror angle and length"),
            };
            (label.to_string(), "vector.edit.mirroring", json!({ "mode": m.id() }))
        })
        .collect();
    sec = sec.child(sublabel_row(tr("Mirroring"))).child(field_row_full(pick_dropdown(f, mirror_label, 200.0, mirror_rows)));

    // 这些点上的端点 / 连接样式（`Some(None)` = 跟图层一致，`None` = Mixed）。
    let mut cap_rows: Vec<(String, &'static str, Value)> = vec![(tr_noop("Same as layer").into(), "vector.edit.strokeCap", json!({ "cap": Value::Null }))];
    for c in StrokeCap::ALL {
        cap_rows.push((c.label().to_string(), "vector.edit.strokeCap", json!({ "cap": c })));
    }
    let mut join_rows: Vec<(String, &'static str, Value)> =
        vec![(tr("Same as layer").into(), "vector.edit.strokeJoin", json!({ "join": Value::Null }))];
    let join_name = |j: StrokeJoin| match j {
        StrokeJoin::Miter => tr("Miter"),
        StrokeJoin::Bevel => tr("Bevel"),
        StrokeJoin::Round => tr("Round"),
    };
    for j in StrokeJoin::ALL {
        join_rows.push((join_name(j).to_string(), "vector.edit.strokeJoin", json!({ "join": j })));
    }
    let cap_label = match v.cap {
        None => tr("Mixed"),
        Some(None) => tr("Same as layer"),
        Some(Some(c)) => c.label(),
    };
    let join_label = match v.join {
        None => tr("Mixed"),
        Some(None) => tr("Same as layer"),
        Some(Some(j)) => join_name(j),
    };
    sec.child(sublabel_row2(tr("End point"), tr("Join"))).child(field_row(
        vec![
            pick_dropdown(f, cap_label, 140.0, cap_rows).into_any_element(),
            pick_dropdown(f, join_label, 140.0, join_rows).into_any_element(),
        ],
        Vec::<IconButton>::new(),
    ))
}

/// Frame 工具（F）的预设面板（egui `frame_presets.rs`）。
///
/// 按 `FRAME_PRESETS` 的分类折叠列表：点分类折展、点预设推 `frame.create` + 切回 Move +
/// 缩放到新选区。展开状态存在 `RightFields`（egui 存在 `ui.data` 临时值里），默认只开 Phone。
pub(crate) fn frame_presets(f: &RightFields, _d: &PanelData) -> PanelSection {
    use figcraft_engine::cmd::create::FRAME_PRESETS;
    let mut sec = PanelSection::new(tr("Frame"));
    let mut cats: Vec<&'static str> = Vec::new();
    for (c, ..) in FRAME_PRESETS {
        if !cats.contains(c) {
            cats.push(c);
        }
    }
    let fmt = |v: f64| if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
    for cat in cats {
        let open = {
            let map = f.preset_open.borrow();
            map.iter().find(|(c, _)| *c == cat).map(|(_, o)| *o).unwrap_or(cat == tr("Phone"))
        };
        let toggle_cmds = f.cmds.clone();
        let map = f.preset_open.clone();
        let head = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h_full()
            .gap(pt(6.0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_e, _w, _cx| {
                {
                    let mut m = map.borrow_mut();
                    match m.iter_mut().find(|(c, _)| *c == cat) {
                        Some(e) => e.1 = !e.1,
                        None => m.push((cat, true)),
                    }
                }
                toggle_cmds.wake();
            })
            .child(IconButton::new(if open { Icon::ChevronDown } else { Icon::ChevronRight }).size(16.0))
            .child(div().text_size(pt(11.0)).child(SharedString::from(cat)));
        sec = sec.child(field_row_full(head));
        if !open {
            continue;
        }
        for (_, name, pw, ph) in FRAME_PRESETS.iter().filter(|(c, ..)| *c == cat) {
            let cmds = f.cmds.clone();
            let label = SharedString::from(*name);
            let size = SharedString::from(format!("{}×{}", fmt(*pw), fmt(*ph)));
            let preset = SharedString::from(*name);
            let line = div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h_full()
                .gap(pt(6.0))
                .pl(pt(16.0))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_e, _w, _cx| {
                    cmds.push(Cmd { id: "frame.create", params: Some(json!({ "preset": preset.clone() })) });
                    cmds.push(Cmd { id: "tool.set", params: Some(json!({ "tool": "move" })) });
                    cmds.push(Cmd { id: "view.zoomSelection", params: Some(json!({})) });
                })
                .child(div().flex_1().text_size(pt(11.0)).child(label))
                .child(div().text_size(pt(11.0)).text_color(rgba(tokens().text_secondary)).child(size));
            sec = sec.child(field_row_full(line));
        }
    }
    sec
}

/// Scale 工具面板（egui `scale_panel.rs`）：Dimensions（W/H）+ 倍数 + 3×3 锚点。
///
/// W/H/倍数的字段是 `RightFields` 里的池化 `TextField`，提交时经 `transform.scale` 生效
/// （描边、效果、圆角、文字都随图层一起缩放）；锚点点击推 `tool.scaleAnchor`。
pub(crate) fn scale(f: &RightFields, d: &PanelData, cx: &mut App) -> PanelSection {
    let mut sec = PanelSection::new(tr("Scale"));
    let Some(s) = d.scale else { return sec };
    // 每帧把当前包围盒 / 支点写进提交上下文，供 W / H 字段的 `on_commit` 换算倍数。
    *f.scale_ctx.borrow_mut() = s;
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    f.scale_w.update(cx, |tf, _| tf.set_value(format!("{}", r2(s.w))));
    f.scale_h.update(cx, |tf, _| tf.set_value(format!("{}", r2(s.h))));
    sec = sec
        .child(sublabel_row(tr("Dimensions")))
        .child(field_row(vec![f.scale_w.clone(), f.scale_h.clone()], Vec::<IconButton>::new()))
        .child(sublabel_row2(tr("Scale"), tr("Anchor")))
        .child(field_row(
            vec![f.scale_k.clone().into_any_element(), anchor_picker(f, s.anchor).into_any_element()],
            Vec::<IconButton>::new(),
        ));
    sec
}

/// 3×3 锚点点阵（egui `scale_panel.rs::anchor_picker`）。
///
/// 只在 Scale 节用一处，按「仅此处用就内联」的约定不抽成通用控件；48 pt 方框 + 9 个 16 pt
/// 命中格，选中的点画成 7 pt 实心 accent 方块，其余 4 pt 灰点。点击推 `tool.scaleAnchor`。
fn anchor_picker(f: &RightFields, cur: (u8, u8)) -> gpui::Div {
    let bg = tokens().field;
    let border = tokens().border;
    let accent = tokens().accent;
    let dim = tokens().text_secondary;
    let mut grid =
        div().w(pt(48.0)).h(pt(48.0)).rounded(pt(5.0)).bg(rgba(bg)).border_1().border_color(rgba(border)).flex().flex_col().items_center().justify_center();
    for row in 0..3u8 {
        let mut line = div().flex().flex_row().items_center();
        for col in 0..3u8 {
            let on = (col, row) == cur;
            let cmds = f.cmds.clone();
            let size = if on { 7.0 } else { 4.0 };
            line = line.child(
                div()
                    .w(pt(16.0))
                    .h(pt(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_e, _w, _cx| {
                        cmds.push(Cmd {
                            id: "tool.scaleAnchor",
                            params: Some(json!({ "anchor": figcraft_engine::tools::anchor_name((col, row)) })),
                        });
                    })
                    .child(div().w(pt(size)).h(pt(size)).rounded(pt(if on { 2.0 } else { 1.0 })).bg(rgba(if on { accent } else { dim }))),
            );
        }
        grid = grid.child(line);
    }
    grid
}

/// 网格 auto layout 的行列轨道（egui `layout_panel.rs::tracks`）。
///
/// 每轴一组：`Column sizes` / `Row sizes` 标题 + 每条轨道一行（序号 + 尺寸 + sizing 下拉）。
/// Fill 轨道的数值是 fr 份数，Fixed / Hug 显示实际 px 尺寸 —— 与 egui 完全一致。
/// 数值框复用行编辑器池（`f.row_field`）：轨道条数随设计变化，不能每帧新建 retained 实体。
/// 写回：`layout.setTrack {axis, index, sizing, value}`（改尺寸）或省略 `value`（改 sizing）。
pub(crate) fn grid_track_rows(f: &RightFields, d: &PanelData, cx: &mut App, n: &mut usize, mut sec: PanelSection) -> PanelSection {
    let Some(t) = d.grid_tracks.as_ref() else { return sec };
    if t.columns.is_empty() && t.rows.is_empty() {
        return sec;
    }
    for (axis, tracks, caption) in [("columns", &t.columns, tr("Column sizes")), ("rows", &t.rows, tr("Row sizes"))] {
        if tracks.is_empty() {
            continue;
        }
        sec = sec.child(sublabel_row(caption));
        for (i, track) in tracks.iter().enumerate() {
            // ego `tracks`: Fill 编辑 fr；Fixed / Hug 编辑（并显示）px。
            let shown = if track.sizing == "fill" { track.value } else { track.size };
            let s = slot(n);
            let edit = f.row_field(
                cx,
                s,
                &format!("{shown}"),
                RowEditCtx {
                    cmd: "layout.setTrack",
                    params: json!({ "axis": axis, "index": i, "sizing": track.sizing }),
                    key: "value",
                    scale: 1.0,
                    string_value: false,
                    paint: None,
                    nest: None, nest_key: None, paint_color: None,
                },
            );
            let sizes: Vec<(String, &'static str, Value)> = [(tr_noop("Fixed"), "fixed"), (tr_noop("Hug contents"), "hug"), (tr_noop("Fill (fr)"), "fill")]
                .into_iter()
                .map(|(l, v)| (l.to_string(), "layout.setTrack", json!({ "axis": axis, "index": i, "sizing": v })))
                .collect();
            let label = match track.sizing {
                "fixed" => tr("Fixed"),
                "hug" => tr("Hug"),
                _ => tr("Fill"),
            };
            let line = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .child(div().w(pt(18.0)).child(SharedString::from(format!("{}", i + 1))))
                .child(div().flex_1().min_w(px(0.0)).child(edit))
                .child(pick_dropdown(f, label, 140.0, sizes));
            sec = sec.child(field_row_full(line));
        }
    }
    sec
}

pub(crate) fn variable_modes(f: &RightFields, d: &PanelData, row_w: f32) -> Option<PanelSection> {
    if d.variable_modes.is_empty() {
        return None;
    }
    let t = tokens();
    let mut sec = PanelSection::new(tr("Variable modes"));
    for m in &d.variable_modes {
        // egui `paints.rs::variable_modes`: both the collection and the mode are sent as **ids**,
        // not their display names; `Auto` is `mode: null`.
        let rows: Vec<(String, &'static str, Value)> = m
            .options
            .iter()
            .zip(m.option_ids.iter())
            .map(|(o, id)| (o.clone(), "variable.setMode", json!({ "collection": m.collection_id, "mode": id.clone() })))
            .collect();
        // egui `paints.rs::variable_modes`：集合名是 `regular(11.0)` + `t.text`，模式下拉在右。
        //
        // 之前这里是一行手写的 `div().flex().flex_row()`：名字没给字号（继承到面板正文的
        // 13 pt+，比整栏高一行，看着「肿」），后面的下拉又带 `w_full`——在 `flex_row` 里
        // `w_full` 是父宽的 100%，于是整行被名字顶宽、下拉溢出右边界，左右两侧完全不对等。
        // 现在走统一网格：左列 `1fr` 放名字（11 pt、超长裁切），右列 `1fr` 放模式下拉。
        let name = div()
            .w_full()
            .overflow_hidden()
            .text_size(pt(11.0))
            .text_color(rgba(t.text))
            .child(SharedString::from(m.collection.clone()));
        let pick = pick_dropdown(f, m.current.clone(), row_w, rows);
        sec = sec.child(field_row(
            vec![name.into_any_element(), pick.into_any_element()],
            Vec::<gpui::AnyElement>::new(),
        ));
    }
    Some(sec)
}

pub(crate) fn section(f: &RightFields, d: &PanelData, row_w: f32) -> Option<PanelSection> {
    let s = d.section.as_ref()?;
    // egui `section_panel::section`: the "Section color" block only exists for a Section layer
    // (`is_section`); a rectangle / text / frame has no "Section" section at all. It was drawn
    // for every layer here, which is the stray "Section" row the user saw.
    if !s.is_section {
        return None;
    }
    let mut sec = PanelSection::new(tr("Section"));
    if s.is_section {
        // 8 个 Section color 色板（`section_panel.rs::SECTION_COLORS`）。
        let colors = [
            (tr("White"), "#FFFFFF"),
            (tr("Gray"), "#EDEDED"),
            (tr("Red"), "#FFE0DE"),
            (tr("Orange"), "#FFE8D1"),
            (tr("Yellow"), "#FFF4C2"),
            (tr("Green"), "#DDF5E3"),
            (tr("Blue"), "#DCEBFF"),
            (tr("Violet"), "#EBE3FF"),
        ];
        let mut swatches = div().flex().flex_row().items_center().gap(pt(6.0));
        for (_, hex) in colors {
            let c = Color32::from_hex(hex).unwrap_or(Color32::WHITE);
            let cmds = f.cmds.clone();
            let hex = hex.to_string();
            swatches = swatches.child(
                div().size(pt(18.0)).rounded(pt(9.0)).bg(rgba(c)).border_1().border_color(rgba(tokens().border)).cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    move |_e, _w, _cx| {
                        cmds.push(Cmd { id: "node.set", params: Some(json!({ "props": { "fills": [hex] } })) });
                    },
                ),
            );
        }
        sec = sec.child(sublabel_row(tr("Section color"))).child(field_row_full(swatches));
        sec = sec.child(field_row(
            vec![pick_dropdown(
                f,
                if s.hide_contents { "Hide contents ✓" } else { tr("Hide contents") },
                row_w,
                vec![
                    ("Show contents".into(), "section.toggleContents", json!({ tr("hidden"): false })),
                    (tr("Hide contents").into(), "section.toggleContents", json!({ tr("hidden"): true })),
                ],
            )],
            Vec::<IconButton>::new(),
        ));
    }
    sec = sec.child(sublabel_row(tr("Status"))).child(field_row(
        vec![pick_dropdown(
            f,
            s.status.clone(),
            row_w,
            vec![
                (tr("No status").into(), "node.set", json!({ "props": { "devStatus": "none" } })),
                (tr("Ready for dev").into(), "node.set", json!({ "props": { "devStatus": "readyForDev" } })),
                (tr("Completed").into(), "node.set", json!({ "props": { "devStatus": "completed" } })),
            ],
        )],
        Vec::<IconButton>::new(),
    ));
    if s.changed {
        sec = sec.child(field_row(
            vec![pick_dropdown(f, "Resolve — back to Ready for dev", row_w, vec![(tr("Resolve").into(), "dev.resolveChange", json!({}))])],
            Vec::<IconButton>::new(),
        ));
    }
    sec = sec.child(field_row(
        vec![pick_dropdown(f, tr("Resize to fit"), row_w, vec![(tr("Resize to fit").into(), "object.resizeToFit", json!({}))])],
        Vec::<IconButton>::new(),
    ));
    Some(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Selection header —— `lizard right.rs::header` 的判断规则 1:1
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 标题行菜单里的一个行模型：lizard `menu_item` / `menu_check` / `Menu::separator`。
/// `cmd` 为空 = 当前已选中的勾选项，点了不推命令（egui `if to_frame != frame`）。
#[derive(Clone)]
struct HeadRow {
    label: String,
    cmd: &'static str,
    params: Value,
    /// `Some(on)` = 勾选行 / 保留勾选列；`None` = 普通行。
    checked: Option<bool>,
    separator: bool,
}

impl HeadRow {
    fn plain(label: impl Into<String>, cmd: &'static str, params: Value) -> Self {
        Self { label: label.into(), cmd, params, checked: None, separator: false }
    }
    fn check(label: impl Into<String>, cmd: &'static str, params: Value, on: bool) -> Self {
        Self { checked: Some(on), ..Self::plain(label, cmd, params) }
    }
    /// egui `menu_item_beside_checks`：普通行，但要占住勾选列。
    fn beside_checks(label: impl Into<String>, cmd: &'static str, params: Value) -> Self {
        Self { checked: Some(false), ..Self::plain(label, cmd, params) }
    }
    fn sep(mut self) -> Self {
        self.separator = true;
        self
    }
}

fn head_menu(rows: &[HeadRow]) -> Vec<MenuRow> {
    rows.iter()
        .map(|r| {
            let mut m = MenuRow::new(r.label.clone());
            if let Some(on) = r.checked {
                m = m.checked(Some(on));
            }
            if r.separator {
                m = m.separator();
            }
            m
        })
        .collect()
}

/// 选中一行就推它的命令（lizard `menu_popup(&resp).show(...)`）。
fn head_pick(rows: Vec<HeadRow>, cmds: crate::panels::Cmds) -> Rc<dyn Fn(&str, &mut Window, &mut App)> {
    Rc::new(move |label, _w, _cx| {
        if let Some(r) = rows.iter().find(|r| r.label == label) {
            if !r.cmd.is_empty() {
                cmds.push(Cmd { id: r.cmd, params: Some(r.params.clone()) });
            }
        }
    })
}

/// 类型名的文字下拉（lizard `header` 的 `lr`：86×32）。Frame / Group 才有；Frame 的菜单
/// 还接着 `FRAME_PRESETS`，每个分类前一条分隔线（`kind_items` + preset 列表）。
fn head_dropdown(f: &RightFields, label: String, rows: Vec<HeadRow>) -> Dropdown {
    let menu = head_menu(&rows);
    let mut d = Dropdown::new(label, 86.0, None);
    if let Some(root) = f.root.clone() {
        let cmds = f.cmds.clone();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let on_pick = head_pick(rows.clone(), cmds.clone());
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(260.0), menu.clone(), on_pick, cx));
        });
    }
    d
}

/// 一个图标按钮，点开标题行的菜单（lizard `menu_popup(&resp)`）。
fn head_menu_button(f: &RightFields, icon: Icon, side: f32, width: f32, rows: Vec<HeadRow>) -> IconButton {
    let menu = head_menu(&rows);
    let cmds = f.cmds.clone();
    let root = f.root.clone();
    IconButton::new(icon).size(side).on_click(move |_e, window, cx: &mut App| {
        let Some(root) = root.clone() else { return };
        let anchor = Bounds { origin: window.mouse_position(), size: size(px(0.0), px(0.0)) };
        let on_pick = head_pick(rows.clone(), cmds.clone());
        root.update(cx, |cv, cx| cv.open_menu(anchor, Some(width), menu.clone(), on_pick, cx));
    })
}

/// 「Create link」：lizard `menus::invoke("text.link")` 先弹输入框拿 URL，再执行 `text.link`。
/// GPUI 的通用单值对话框（A8/A17 同款）就是那个输入框。
fn link_button(f: &RightFields) -> IconButton {
    let root = f.root.clone();
    IconButton::new(Icon::LinkOn).size(24.0).on_click(move |_e, _window, cx: &mut App| {
        let Some(root) = root.clone() else { return };
        root.update(cx, |cv, cx| cv.open_link_dialog(cx));
    })
}

/// 选区标题行（lizard `right.rs::header`）。
///
/// ⛔ **一行 40 pt、没有区块标题**：左边 86 pt 的类型名（Frame / Group 可下拉互转），
/// 右边按选中类型整套换按钮。所有显隐都走 [`crate::panels::visibility::header_actions`]，
/// 规则逐条对应 lizard 的 `is_frame / is_group / is_text / edit_id / points_selected`：
///
/// * 单选 Frame → 类型下拉里还有 `FRAME_PRESETS`（分类分隔）+ 右侧「交换方向」；
/// * 选区全是 Text → 四个专属按钮（Select matching / Create link / Create component / More），
///   **不画**普通图层的布尔 / 蒙版按钮；
/// * 单选 Component / ComponentSet，或矢量编辑已选点 → 右侧动作一个都不画；
/// * 其余 → Select matching / Create component / Mask / 布尔按钮 + 布尔菜单 / Edit object（单选形状）。
///
/// 与 `view_bar` 一样是**独立自由行**：lizard 用 `allocate_exact_size(w, 40)` 后绝对定位左右
/// 两块，冻结的统一网格（`field_row_h` 的 2×1fr + 24pt 列）放不下 6 个动作格 + 86 pt 标签。
pub(crate) fn header(f: &RightFields, d: &PanelData, col: f32) -> Option<gpui::Div> {
    let h = d.header.as_ref()?;
    let _ = col;
    let t = tokens();
    let sel = crate::panels::HeaderSel {
        is_frame: h.is_frame,
        is_group: h.is_group,
        is_text: h.is_text,
        is_component: h.is_component,
        is_shape: h.is_shape,
        points_selected: d.vector.as_ref().is_some_and(|v| v.has_selection),
        count: h.count,
    };
    let a = visibility::header_actions(&sel);

    // ── 左：类型名 / 类型下拉 ─────────────────────────────────────────────────────
    let lead: gpui::AnyElement = if a.type_menu {
        // `kind_items`: 勾选项点了什么都不做，只有转换成另一种才推命令。
        let mut rows = vec![
            HeadRow::check(tr("Frame"), if h.is_frame { "" } else { "object.convertToFrame" }, json!({}), h.is_frame),
            HeadRow::check(tr("Group"), if h.is_group { "" } else { "object.convertToGroup" }, json!({}), h.is_group),
        ];
        if h.is_frame {
            use figcraft_engine::cmd::create::FRAME_PRESETS;
            let mut cat = "";
            for (c, name, w, hh) in FRAME_PRESETS {
                let mut r = HeadRow::plain(*name, "node.set", json!({ "props": { "width": w, "height": hh } }));
                if *c != cat {
                    r = r.sep();
                    cat = c;
                }
                rows.push(r);
            }
        }
        head_dropdown(f, tr(&h.label).to_string(), rows).into_any_element()
    } else {
        // 其它类型没有下拉，连 chevron 都不画：就是一段 13 pt 半粗文字。
        div()
            .text_size(pt(13.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.text))
            .child(SharedString::from(tr(&h.label).to_string()))
            .into_any_element()
    };

    // ── 右：按选中类型整套换按钮 ───────────────────────────────────────────────
    let mut actions: Vec<gpui::AnyElement> = Vec::new();
    // 「交换方向」在 Frame 上挂到动作列最左（lizard 把它对齐到 4 个动作格 + chevron 的左边）。
    if a.swap_orientation {
        actions.push(icon_cmd(Icon::Rotate, "frame.swapOrientation", json!({}), f, false).into_any_element());
    }
    if a.text {
        // lizard `is_text` 分支的四个按钮，左→右顺序照抄。
        actions.push(icon_cmd(Icon::Search, "select.matching", json!({}), f, false).into_any_element());
        actions.push(link_button(f).into_any_element());
        actions.push(icon_cmd(Icon::Component, "component.create", json!({}), f, false).into_any_element());
        let more: Vec<HeadRow> = h
            .more_actions
            .iter()
            .map(|(label, cmd, sep)| {
                let r = HeadRow::plain(label.clone(), cmd, json!({}));
                if *sep { r.sep() } else { r }
            })
            .collect();
        actions.push(head_menu_button(f, Icon::Menu, 24.0, 240.0, more).into_any_element());
    } else if a.quick {
        // lizard：从右往左布局，所以这里左→右是
        // Select matching / Create component / Mask / Union / 布尔 chevron / Edit object。
        actions.push(icon_cmd(Icon::Search, "select.matching", json!({}), f, false).into_any_element());
        actions.push(icon_cmd(Icon::Component, "component.create", json!({}), f, false).into_any_element());
        actions.push(icon_cmd(Icon::Image, "object.mask", json!({}), f, false).into_any_element());
        let bool_icon = match h.bool_op {
            Some(figcraft_doc::BoolOp::Subtract) => Icon::Subtract,
            Some(figcraft_doc::BoolOp::Intersect) => Icon::Intersect,
            Some(figcraft_doc::BoolOp::Exclude) => Icon::Exclude,
            _ => Icon::Union,
        };
        actions.push(
            if visibility::bool_union_enabled(h.count) {
                icon_cmd(bool_icon, "bool.union", json!({}), f, false)
            } else {
                // lizard：少于两个图层时点了也不执行，按钮只是当前运算的状态图标。
                IconButton::new(bool_icon)
            }
            .into_any_element(),
        );
        let cur = h.bool_op.map(|o| o.label());
        let mut bool_rows: Vec<HeadRow> = Vec::new();
        for (label, cmd) in [(tr("Union"), "bool.union"), (tr("Subtract"), "bool.subtract"), (tr("Intersect"), "bool.intersect"), (tr("Exclude"), "bool.exclude")] {
            let r = match cur {
                Some(c) => HeadRow::check(label, cmd, json!({}), c == label),
                None => HeadRow::plain(label, cmd, json!({})),
            };
            bool_rows.push(r);
        }
        // Flatten：有当前运算时用 `menu_item_beside_checks` 占住勾选列。
        let flatten = if cur.is_some() {
            HeadRow::beside_checks(tr("Flatten"), "vector.flatten", json!({}))
        } else {
            HeadRow::plain(tr("Flatten"), "vector.flatten", json!({}))
        };
        bool_rows.push(flatten.sep());
        actions.push(head_menu_button(f, Icon::ChevronDown, 16.0, 180.0, bool_rows).into_any_element());
        if a.edit_object {
            actions.push(icon_cmd(Icon::Pen, "vector.edit.begin", json!({}), f, false).into_any_element());
        }
    }

    Some(
        div()
            .flex_none()
            .w_full()
            .h(pt(40.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(pt(4.0))
            .pl(pt(8.0))
            .pr(pt(8.0))
            .child(div().flex_none().w(pt(86.0)).flex().items_center().child(lead))
            .child(div().flex().flex_row().items_center().gap(pt(4.0)).children(actions)),
    )
}

/// 样式名的「叶子」部分（去掉 "Brand/" 前缀），egui 的 `leaf(&s.name)`。
fn leaf_name(name: &str) -> SharedString {
    SharedString::from(name.rsplit('/').next().unwrap_or(name).to_string())
}

/// 文件夹路径：名字中最后一个 "/" 之前的部分（egui `style_picker::folder`）。
fn ls_folder(name: &str) -> String {
    name.rsplit_once('/').map_or_else(|| String::new(), |(p, _)| p.to_string())
}

/// `entries()` 产出的扁平树的一行（egui `style_picker::local_styles` 的 Folder / Style 行）。
enum LsLine {
    Folder { label: String, depth: usize },
    /// `idx` 是样式在 `list` 中的下标；`depth` 是缩进层级。
    Style(usize, usize),
}

/// 文件夹树节点（egui `style_picker::entries` 的 `Branch`）。
struct LsBranch {
    path: String,
    label: String,
    styles: Vec<usize>,
    kids: Vec<usize>,
}

/// 按名字里的 "/" 把样式组织成文件夹树，扁平化成「先文件夹头、后其下样式、再递归子文件夹」的列表。
/// 与 egui `entries` / `branch_of` / `flatten` 一一对应；`folded` 返回 true 的文件夹整支收起
/// （本遍不做折叠态，始终展开）。
fn ls_entries(list: &[&LocalStyleRow], folded: &dyn Fn(&str) -> bool) -> Vec<LsLine> {
    // 按文件夹分组（无文件夹的放到 groups[0] 的 "" 组）。
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let f = ls_folder(&s.name);
        match groups.iter_mut().find(|(g, _)| *g == f) {
            Some((_, idx)) => idx.push(i),
            None => groups.push((f, vec![i])),
        }
    }
    // 有文件夹的排后面（egui `sort_by_key(|(f, _)| !f.is_empty())`：空目录最前）。
    groups.sort_by_key(|(f, _)| !f.is_empty());
    let mut nodes: Vec<LsBranch> = vec![LsBranch { path: String::new(), label: String::new(), styles: Vec::new(), kids: Vec::new() }];
    for (path, idx) in groups {
        let ni = ls_branch_of(&mut nodes, &path);
        nodes[ni].styles = idx;
    }
    let mut out = Vec::new();
    for &i in &nodes[0].styles {
        out.push(LsLine::Style(i, 0));
    }
    let top = std::mem::take(&mut nodes[0].kids);
    for kid in top {
        ls_flatten(&nodes, kid, 0, folded, &mut out);
    }
    out
}

/// 取得（或递归建出）`path` 对应的分支（egui `branch_of`）。
fn ls_branch_of(nodes: &mut Vec<LsBranch>, path: &str) -> usize {
    if let Some(i) = nodes.iter().position(|n| n.path == path) {
        return i;
    }
    let (parent, label) = path.rsplit_once('/').map_or(("", path), |(p, l)| (p, l));
    let parent = ls_branch_of(nodes, parent);
    nodes.push(LsBranch { path: path.to_string(), label: label.to_string(), styles: Vec::new(), kids: Vec::new() });
    let i = nodes.len() - 1;
    nodes[parent].kids.push(i);
    i
}

/// 一个分支的头 + 其下全部内容（egui `flatten`）。
fn ls_flatten(nodes: &[LsBranch], node: usize, depth: usize, folded: &dyn Fn(&str) -> bool, out: &mut Vec<LsLine>) {
    let n = &nodes[node];
    out.push(LsLine::Folder { label: n.label.clone(), depth });
    if folded(&n.path) {
        return;
    }
    for &i in &n.styles {
        out.push(LsLine::Style(i, depth + 1));
    }
    for &kid in &n.kids {
        ls_flatten(nodes, kid, depth + 1, folded, out);
    }
}

/// 类型分组的标题行（egui `local_styles` 的 caption：32 pt、类型名 + 右侧 `+`）。
fn local_style_caption(title: SharedString, plus: IconButton) -> FieldRow {
    let t = tokens();
    let body = div()
        .w_full()
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .pl(pt(8.0))
        .pr(pt(8.0))
        .child(div().text_size(pt(11.0)).font_weight(gpui::FontWeight::MEDIUM).text_color(rgba(t.text_secondary)).child(title))
        .child(div().flex_1().min_w(px(0.0)))
        .child(plus);
    field_row_full(body)
}

/// 文件夹头行（egui `local_styles` 的 Folder 行：chevron + 文件夹名，按层级缩进）。
/// 本遍不做折叠态，始终展开（chevron 朝下）。
fn local_style_folder(label: &str, depth: usize) -> FieldRow {
    let t = tokens();
    let body = div()
        .w_full()
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .pl(pt(24.0 * depth as f32))
        .pr(pt(8.0))
        .cursor_pointer()
        .child(crate::widgets::icon::paint(Icon::ChevronDown, rgba(t.text_secondary), 12.0))
        .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(label.to_string())));
    field_row_full(div().w_full().h_full().rounded(pt(5.0)).hover(|s| s.bg(rgba(t.hover))).child(body))
}

/// 类型分组标题栏的 `+`：弹出「建哪种样式」菜单（egui `local_styles` 的头部 `+`）。
fn local_styles_add_button(f: &RightFields) -> IconButton {
    // (菜单标题, style.create 的 type, 默认名) —— 与 egui `StyleSlot::{title,create_type,default_name}` 对齐。
    // 必须是 `static`：菜单回调会逃出本函数，借本地数组会活不过函数体。
    static ITEMS: &[(&str, &str, &str)] = &[
        ("Text styles", "text", "Text"),
        ("Color styles", "paint", "Color"),
        ("Effect styles", "effect", "Effect"),
        ("Grid styles", "grid", "Grid"),
    ];
    let root = f.root.clone();
    let cmds = f.cmds.clone();
    IconButton::new(Icon::Plus)
        .size(24.0)
        .on_click(move |_e, window, cx: &mut App| {
            let rows: Vec<MenuRow> = ITEMS.iter().map(|r| MenuRow::new(r.0)).collect();
            let Some(root) = root.clone() else { return };
            let anchor = Bounds { origin: window.mouse_position(), size: size(px(0.0), px(0.0)) };
            // `on_click` 是 `Fn`，捕获的 `cmds` 不能直接 move 进内层回调，先克隆一份。
            let cmds = cmds.clone();
            root.update(cx, move |cv, cx| {
                cv.open_menu(
                    anchor,
                    Some(200.0),
                    rows,
                    Rc::new(move |label: &str, _w, _cx| {
                        if let Some(r) = ITEMS.iter().find(|r| r.0 == label) {
                            let (_, ctype, dname) = *r;
                            cmds.push(Cmd { id: "style.create", params: Some(json!({ "type": ctype, "name": dname })) });
                        }
                    }),
                    cx,
                );
            });
        })
}

/// Page 节「Local styles」里的一行（egui `style_picker::local_styles` 的 Style 行，1:1）：
/// 左侧徽标 16×16 —— paint 用圆形色板、text 用 "Ag" + 右侧 "32/Auto" 预览、effect/grid 用类型图标；
/// 整行悬停高亮（圆角 t.hover 块，与行同高、内缩 8px）、左键把样式应用到选区、右键 Duplicate / Delete；
/// 文件夹层级用 `depth * 24px` 缩进（egui `LS_STEP`）。
fn local_style_row(f: &RightFields, s: &LocalStyleRow, depth: usize, ty: &'static str) -> FieldRow {
    let t = tokens();
    let cmds = f.cmds.clone();
    let sid = s.id.clone();
    let name = leaf_name(&s.name);
    // 左侧徽标（egui `paint_dot` / "Ag" / `slot.icon()`）。
    let badge: gpui::AnyElement = match &s.paint {
        Some(p) => div().flex_none().size(pt(16.0)).rounded_full().overflow_hidden().child(swatch(p.clone(), 16.0)).into_any_element(),
        None if ty == "text" => div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .min_w(pt(16.0))
            .text_size(pt(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.text))
            .child("Ag")
            .into_any_element(),
        None => {
            let icon = match ty {
                "effect" => Icon::Star,
                "grid" => Icon::ViewGrid,
                _ => Icon::Text,
            };
            div()
                .flex_none()
                .size(pt(16.0))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(icon, rgba(t.text_secondary), 14.0))
                .into_any_element()
        }
    };
    // 整行内容：层级缩进 + 8px 内缩，悬停高亮。
    let mut body = div()
        .w_full()
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .pl(pt(24.0 * depth as f32))
        .pr(pt(8.0))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, {
            let cmds = cmds.clone();
            let sid = sid.clone();
            move |_e, _w, _cx| {
                cmds.push(Cmd { id: "style.apply", params: Some(json!({ "style": sid })) });
            }
        })
        .child(badge);
    // 名字占满中间，文本样式的 "32/Auto" 贴行尾（egui 的 meta 右对齐在 `r.max.x - LS_TRAIL`）。
    body = body.child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(name));
    if let Some(meta) = &s.text_meta {
        body = body.child(div().flex_none().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(meta.clone()));
    }
    // 右键：Duplicate / Delete（egui 的 context_popup）。
    let cmds_r = cmds.clone();
    let root = f.root.clone();
    let dup_label: String = tr("Duplicate").to_string();
    let del_label: String = tr("Delete").to_string();
    body = body.on_mouse_down(MouseButton::Right, move |e: &gpui::MouseDownEvent, _w, cx: &mut App| {
        let cmds_r = cmds_r.clone();
        let sid = sid.clone();
        // `on_mouse_down` 的监听是 `Fn`（可多次触发），不能把捕获的 String 移出去，
        // 每次调用先克隆成本地变量再 move 进菜单回调。
        let dup = dup_label.clone();
        let del = del_label.clone();
        let rows = vec![MenuRow::new(dup.clone()).separator(), MenuRow::new(del.clone())];
        let Some(root) = root.clone() else { return };
        let anchor = gpui::Bounds::new(e.position, gpui::size(px(0.0), px(0.0)));
        root.update(cx, move |cv, cx| {
            cv.open_menu(
                anchor,
                Some(160.0),
                rows,
                Rc::new(move |label: &str, _w, _cx| {
                    let id = if dup.as_str() == label {
                        "style.duplicate"
                    } else if del.as_str() == label {
                        "style.delete"
                    } else {
                        return;
                    };
                    cmds_r.push(Cmd { id, params: Some(json!({ "style": sid })) });
                }),
                cx,
            );
        });
    });
    // 悬停高亮整行（egui `rect_filled(r, CornerRadius::same(5), t.hover)`）。
    field_row_full(div().w_full().h_full().rounded(pt(5.0)).hover(|s| s.bg(rgba(t.hover))).child(body))
}

/// 空选区时的右栏（egui `right.rs::page_section`）：**三个独立小节**，各自一个 40 pt 标题：
/// ① Page（背景色 / Color profile）② Local variables ③ Local styles。
///
/// 之前这三块被塞进同一个 `PanelSection::new(tr("Page"))` 里，「Local variables」和「Local styles」
/// 只是 `sublabel_row2` 的两个小字，样式列表更是压根没画——所以 Local styles 看着「没出现在
/// 该出现的位置」。现在按 egui 拆成独立节，节与节之间由调用方补分隔线。
pub(crate) fn page(f: &RightFields, d: &PanelData, row_w: f32, n: &mut usize, cx: &mut App) -> Vec<PanelSection> {
    let Some(p) = d.page.as_ref() else { return Vec::new() };
    let mut sec = PanelSection::new(tr("Page"));
    let hex_slot = slot(n);
    let op_slot = slot(n);
    let hex_ed = f.row_field(
        cx,
        hex_slot,
        &p.background,
        RowEditCtx { cmd: "page.setBackground", params: json!({}), key: "color", scale: 1.0, string_value: true, paint: None, nest: None, nest_key: None, paint_color: None },
    );
    let op_ed = f.row_field(
        cx,
        op_slot,
        "100",
        RowEditCtx {
            cmd: "page.setBackground",
            params: json!({}),
            key: "opacity",
            scale: 100.0,
            string_value: false,
            paint: None,
            nest: None, nest_key: None, paint_color: None,
        },
    );
    // egui `right.rs::page_section`: the trailing "100 %" is a **label**, not an editable field.
    // Keep the slot's look but make it read-only, so clicking it cannot raise the bogus
    // `page.setBackground {"opacity": …}` (the engine only reads `color`).
    op_ed.update(cx, |te, _| te.set_readonly(true));
    let color = figcraft_color::Color::from_hex(&p.background).unwrap_or(figcraft_color::Color::WHITE);
    let cf = ColorField::new(cx, figcraft_color::Paint::solid(color), ColorText::Hex(SharedString::from(p.background.clone())), row_w, hex_ed, op_ed);
    // swatch → picker (`toggle_picker(PickerTarget::PageBackground)`).
    let cf = color_pick(f, cf, Color32::from_hex(&p.background).unwrap_or(Color32::WHITE), "page.setBackground", json!({}), "color", None);
    sec = sec.child(field_row_full(cf));
    // egui keeps this as local UI state and opens a dialog; GPUI has no dialog yet, so the
    // engine's `file.colorProfile` command is the equivalent. `node.set {"colorProfile": …}`
    // was rejected: `colorProfile` is not a node property.
    let profiles: Vec<(String, &'static str, Value)> =
        figcraft_color::ColorProfile::ALL.iter().map(|cp| (cp.label().to_string(), "file.colorProfile", json!({ "profile": cp.id() }))).collect();
    sec = sec.child(field_row_full(pick_dropdown(f, p.profile.clone(), row_w, profiles)));
    let mut out = vec![sec];

    // ── ② Local variables（egui：独立标题 + 「Open variables (N)」下拉；空时标题置灰）──
    let mut vars = PanelSection::new(tr("Local variables"))
        .variant(if p.local_variables == 0 { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal });
    let table = d.variables.clone();
    let popups = f.popups();
    let queue = f.queue();
    vars = vars.child(field_row_full(
        Dropdown::new(format!("Open variables ({})", p.local_variables), row_w, None)
            .fill()
            .on_click(move |bounds, _, _| {
                popups.request(crate::popovers::WindowState::variables_table(queue.clone(), bounds.origin, table.clone()));
            }),
    ));
    out.push(vars);

    // ── ③ Local styles（egui `style_picker::local_styles`）：按 Text / Color / Effect / Grid
    //       分成四组，每组一个小标题（带 `+`） + 该类型的样式行（含文件夹树）；空文档只有置灰标题。──
    let mut styles_sec = PanelSection::new(tr("Local styles"))
        .variant(if p.local_styles == 0 { SectionHeaderVariant::Dim } else { SectionHeaderVariant::Normal })
        .header_trail(icon_row(vec![local_styles_add_button(f)]));
    // 分组顺序与 egui `StyleSlot` 一致：Text / Fill(Color) / Effect / Grid。
    let groups: &[(&str, &str, &str)] = &[
        ("text", "Text styles", "Text"),
        ("paint", "Color styles", "Color"),
        ("effect", "Effect styles", "Effect"),
        ("grid", "Grid styles", "Grid"),
    ];
    for (ty, cap_key, dname) in groups {
        let list: Vec<&LocalStyleRow> = d.styles.iter().filter(|s| s.ty == *ty).collect();
        if list.is_empty() {
            continue;
        }
        // 每组的小标题 + 该组的 `+`（直接建这种类型的样式；egui 同位置的 `+` 也是这个行为）。
        let cmds = f.cmds.clone();
        let ctype = *ty;
        let dname = *dname;
        let plus = IconButton::new(Icon::Plus).size(16.0).on_click(move |_anchor, _w, _cx| {
            cmds.push(Cmd { id: "style.create", params: Some(json!({ "type": ctype, "name": dname })) });
        });
        styles_sec = styles_sec.child(local_style_caption(SharedString::from(tr(cap_key)), plus));
        // 文件夹树（本遍展开全部）：先文件夹头、后其下样式、再递归子文件夹。
        for line in ls_entries(&list, &|_| false) {
            match line {
                LsLine::Folder { label, depth, .. } => styles_sec = styles_sec.child(local_style_folder(&label, depth)),
                LsLine::Style(idx, depth) => {
                    let s = &list[idx];
                    styles_sec = styles_sec.child(local_style_row(f, s, depth, *ty));
                }
            }
        }
    }
    out.push(styles_sec);
    out
}

#[cfg(test)]
mod view_menu_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `menu_check` 给勾选行加 `✓ ` / 四个空格前缀，`menu_popup` 在 on_pick 时 trim 掉。
    /// 所以行 label 本身不能带前后空格、也不能重复，否则会选中错的那条命令。
    #[test]
    fn view_rows_are_unique_trimmed_and_registered() {
        let rows = view_rows(&ViewData::default());
        let mut seen = BTreeSet::new();
        for r in &rows {
            assert_eq!(r.label.trim(), r.label, "menu label `{}` would break on_pick trim", r.label);
            assert!(seen.insert(r.label.clone()), "duplicate menu label `{}`", r.label);
            assert!(figcraft_engine::cmd::find_command(r.cmd).is_some(), "view menu raises unknown command `{}`", r.cmd);
        }
        assert_eq!(rows.len(), 14, "7 zoom + 2 pixel preview + 5 show/hide toggles");
        assert_eq!(rows.iter().filter(|r| r.checked.is_some()).count(), 7, "only show/hide rows get a check column");
        assert_eq!(rows.iter().filter(|r| r.separator).count(), 2, "egui separates the zoom / preview / toggle groups");
    }

    /// 1x / 2x 不能同时打勾，且 `on` 必须是当前值的反相（引擎 `pixel_preview` 读 `on`）。
    #[test]
    fn pixel_preview_rows_toggle_the_current_scale() {
        let v = ViewData { pixel_preview: true, pixel_scale: 2, ..Default::default() };
        let rows = view_rows(&v);
        let previews: Vec<&ViewMenuRow> = rows.iter().filter(|r| r.cmd == "view.pixelPreview").collect();
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[0].checked, Some(false));
        assert_eq!(previews[0].params["on"], serde_json::json!(true));
        assert_eq!(previews[1].checked, Some(true));
        assert_eq!(previews[1].params["on"], serde_json::json!(false));
        assert_eq!(previews[1].params["scale"], serde_json::json!(2));
    }
}
