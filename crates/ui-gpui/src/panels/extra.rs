//! 整面板内容（egui `shortcuts_panel.rs` / `versions.rs` / `dev_compare.rs` / `assets_panel.rs`
//! / `icons_panel.rs`）的 GPUI 版，由引擎 `ui_requests` 打开。
//!
//! 数据一律来自引擎：`figcraft_engine::keymap::{KEYS,GESTURES,Group,Platform,display}`、
//! `DocState::versions`、`devcompare::compare`、`library::local_components`。
//! 所有写操作都推引擎命令（`version.*` / `dev.*` / `instance.create` / `style.apply`），
//! 面板本身不改文档。控件全部复用冻结的 `widgets/`。
//!
//! ⚠️ TODO（规划错配，见 parity plan R.1 / R.7 / R.8 / R.9）：egui 把 Versions 放独立面板、
//! Dev compare 做占满工作区的双图对比、Shortcuts 放窗口底部常驻条；GPUI 目前仍把它们塞进
//! 同一个可关闭的居中浮层外壳，**这是已知偏差，不是既定设计**。
//! ✅ Assets / Icons 已按 R.4 搬进左栏 `left_tab`（`layer_tree.rs` 直接调 `assets_body` /
//! `icons_panel::body`），不再经过本文件的 `ExtraKind`；Home 走占满窗口的 `CentralPanel` 分支。
//!
//! 四块现已 1:1 还原：
//! - Dev compare：并排双图 / 叠加分割滑块两种排布、版本下拉切换对比基准、改动层彩色边框
//!   （新增绿 / 移除红 / 改动脉冲色）、点击列表定位选区 + 悬停高亮、Resolve / Close 底栏。
//! - Versions：预览横幅（Back to current / Restore）、保存与重命名内联表单、每行整行点击进
//!   预览、Eye（预览）/Rotate（恢复）/Pencil（重命名）/Trash（删除确认）/Dup 操作、`time_ago`
//!   相对时间、删除二次确认行。
//! - Assets：Components / Styles 切换 tab + 搜索；Components 为本地组件/组件集缩略图网格
//!   （2 列，GPUI 无 flex_wrap），整卡点击 `instance.create`，组件集取默认变体渲染；Styles
//!   列表按类型给徽标（Paint 色板 / Text 图标 / Grid 图标 / Effect 用「fx」文字），点一行把
//!   样式 `style.apply` 到当前选区。
//! - Shortcuts：分组下拉 + 搜索，键位全部来自引擎 `KEYS`/`GESTURES`，一个不手抄。

use figcraft_i18n::tr;
use crate::app::CanvasApp;
use crate::panels::{Cmd, Cmds};
use crate::widgets::{
    Dropdown, Icon, IconButton, MenuRow, PanelSection, SectionHeaderVariant, TextField, field_row, field_row_full, pt, rgba, search_field, tokens,
};
use figcraft_color::{Color, Paint, PaintKind};
use figcraft_doc::NodeKind;
use figcraft_doc::styles::{Style, StyleKind};
use figcraft_engine::components::default_variant;
use figcraft_engine::keymap::{GESTURES, Group, KEYS, Platform};
use gpui::{
    App, Bounds, ClickEvent, Div, Entity, Hsla, ImageSource, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    RenderImage, SharedString, Window, canvas, div, img, prelude::*, px, rgb,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// 打开着的是哪一块面板。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExtraKind {
    Shortcuts,
    Versions,
    /// `dev.compare {id, version, show}` 带过来的目标。
    DevCompare {
        id: Option<figcraft_doc::NodeId>,
        version: Option<String>,
    },
    /// A1: `doc.home` 的首页（Recents / Templates / Getting started）。
    Home,
}

impl ExtraKind {
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Shortcuts => tr("Keyboard shortcuts"),
            Self::Versions => tr("Version history"),
            Self::DevCompare { .. } => tr("Compare changes"),
            Self::Home => tr("Home"),
        }
    }
}

/// `ui_requests` 的一条请求要 UI 做什么（抽成纯函数，便于逐条验证）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UiAction {
    Tab(u8),
    Toggle(ExtraKind),
    Compare {
        id: Option<figcraft_doc::NodeId>,
        version: Option<String>,
    },
    Widths {
        left: Option<f32>,
        right: Option<f32>,
    },
    Palette,
    /// 原型演示 / 浮动预览（engine proto.present / proto.preview）。
    Present(Option<figcraft_doc::NodeId>, bool),
    PreviewClose,
    /// 切到左面板的某个 tab：1 = Assets，2 = Icons（R1/R2）。不再弹居中浮层，
    /// 左面板按 `left_tab` 渲染对应面板（与 egui `LeftTab` 一致）。
    LeftTab(u8),
}

/// egui `handle_ui_requests` 里 GPUI 能落地的那部分（多窗口 / 评论 / 演示 / 对话框
/// 这些 GPUI 还没有对应 UI，返回 `None`，不假装已实现）。`doc.home` 的 Home 已接上。
pub(crate) fn ui_action(kind: &str, data: &Value, tab: u8) -> Option<UiAction> {
    let width = |k: &str| data.get(k).and_then(Value::as_f64).map(|w| (w as f32).clamp(240.0, 500.0));
    Some(match kind {
        "panel.design" => UiAction::Tab(0),
        "panel.prototype" => UiAction::Tab(1),
        "panel.designPrototype" => UiAction::Tab(if tab == 1 { 0 } else { 1 }),
        "panel.devMode" => UiAction::Tab(if tab == 2 { 0 } else { 2 }),
        "panel.shortcuts" => UiAction::Toggle(ExtraKind::Shortcuts),
        "panel.versionHistory" => UiAction::Toggle(ExtraKind::Versions),
        "panel.assets" | "panel.libraries" => UiAction::LeftTab(1),
        "view.devCompare" => UiAction::Compare {
            id: data.get("id").and_then(figcraft_engine::params::id_of),
            version: data.get("version").and_then(Value::as_str).map(str::to_string),
        },
        "panel.widths" => UiAction::Widths { left: width("left"), right: width("right") },
        "palette.actions" => UiAction::Palette,
        "home" => UiAction::Toggle(ExtraKind::Home),
        "present" => UiAction::Present(data.get("start").and_then(figcraft_engine::params::id_of), false),
        "preview" => UiAction::Present(data.get("start").and_then(figcraft_engine::params::id_of), true),
        "preview.close" => UiAction::PreviewClose,
        _ => return None,
    })
}

/// 比较视图的两种排布：并排 / 叠加（带分割滑块）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DevMode {
    SideBySide,
    Overlay,
}

/// 跨帧的面板状态（egui 存在 `ui.shortcuts_*` / `ui.versions`）。
#[derive(Clone)]
pub(crate) struct ExtraState {
    /// Shortcuts 的分组下标（`Group::ALL`）。
    pub group: Rc<RefCell<usize>>,
    /// Shortcuts 的搜索词（搜索框 Enter 提交）。
    pub query: Rc<RefCell<String>>,
    pub search: Entity<TextField>,
    /// 关闭按钮 / 点遮罩：置位后由 `sync_ui_requests` 在下一帧收掉浮层。
    pub dismiss: Rc<RefCell<bool>>,
    /// Dev compare：并排 / 叠加。
    pub dev_mode: Rc<RefCell<DevMode>>,
    /// Dev compare：对比的版本（`None`：标记 ready 时的快照）。
    pub dev_version: Rc<RefCell<Option<String>>>,
    /// Dev compare：叠加分割滑块位置（0–1）。
    pub dev_split: Rc<RefCell<f32>>,
    /// Dev compare：悬停的改动层（用于高亮其边框）。
    pub dev_hovered: Rc<RefCell<Option<figcraft_doc::NodeId>>>,
    // ── Versions 高级交互状态 ──
    /// 保存/重命名表单：None=关闭；Some(None)=保存新版本；Some(Some(id))=重命名该版本。
    pub ver_form: Rc<RefCell<Option<Option<String>>>>,
    /// 已经预填进文本框的表单目标（避免每帧重置用户输入）。
    pub ver_prefilled: Rc<RefCell<Option<Option<String>>>>,
    /// 等待删除确认的版本 id。
    pub ver_confirm: Rc<RefCell<Option<String>>>,
    pub ver_name: Entity<TextField>,
    pub ver_desc: Entity<TextField>,
    // ── Assets 高级交互状态 ──
    /// false=Components，true=Styles。
    pub assets_tab: Rc<RefCell<bool>>,
    pub assets_query: Rc<RefCell<String>>,
    pub assets_search: Entity<TextField>,
    // ── A1 Home (egui `ui.home_view`) ──
    /// 0 Recents / 1 Online / 2 Templates.
    pub home_section: Rc<RefCell<usize>>,
    pub home_query: Rc<RefCell<String>>,
    pub home_search: Entity<TextField>,
    /// Recents view: `true` grid (cards) / `false` list (egui `home_recents::View`).
    pub home_view_grid: Rc<RefCell<bool>>,
    /// 首页模板卡片的缩略图缓存（`template id → 图`）：渲一次就留着。
    pub template_thumbs: Rc<RefCell<std::collections::HashMap<&'static str, Option<std::sync::Arc<gpui::RenderImage>>>>>,
    /// Recents sort: 0 name / 1 edited (egui `home_recents::SortKey`).
    pub home_sort: Rc<RefCell<u8>>,
    // ── A5 Icons (egui `icons_panel.rs`) ──
    /// 图标面板的搜索词（小写）。
    pub icons_query: Rc<RefCell<String>>,
    pub icons_search: Entity<TextField>,
    /// 图标预览缓存（`font:icon:outline-hash` → 图）：渲一次就留着。
    pub icon_thumbs: Rc<RefCell<std::collections::HashMap<String, Option<std::sync::Arc<gpui::RenderImage>>>>>,
}

impl ExtraState {
    pub(crate) fn new(cx: &mut App, cmds: &Cmds) -> Self {
        let query = Rc::new(RefCell::new(String::new()));
        let search = search_field(cx, 180.0, "", tr("Search shortcuts"));
        let q = query.clone();
        let wake = cmds.clone();
        search.update(cx, move |tf, _| {
            tf.set_on_change(move |text, _, _| {
                *q.borrow_mut() = text.trim().to_lowercase();
                wake.wake();
            });
        });
        let ver_name = TextField::build(cx, 240.0, |tf, _| {
            tf.set_placeholder(tr("Title"));
        });
        let ver_desc = TextField::build(cx, 240.0, |tf, _| {
            tf.set_placeholder(tr("Describe what changed"));
        });
        let assets_query = Rc::new(RefCell::new(String::new()));
        let assets_search = search_field(cx, 200.0, "", tr("Search assets"));
        let aq = assets_query.clone();
        let aw = cmds.clone();
        assets_search.update(cx, move |tf, _| {
            tf.set_on_change(move |text, _, _| {
                *aq.borrow_mut() = text.trim().to_lowercase();
                aw.wake();
            });
        });
        let home_query = Rc::new(RefCell::new(String::new()));
        let home_search = search_field(cx, 220.0, "", tr("Search files and templates"));
        let hq = home_query.clone();
        let hw = cmds.clone();
        home_search.update(cx, move |tf, _| {
            tf.set_on_change(move |text, _, _| {
                *hq.borrow_mut() = text.trim().to_lowercase();
                hw.wake();
            });
        });
        let icons_query = Rc::new(RefCell::new(String::new()));
        let icons_search = search_field(cx, 200.0, "", tr("Search icons"));
        let iq = icons_query.clone();
        let iw = cmds.clone();
        icons_search.update(cx, move |tf, _| {
            tf.set_on_change(move |text, _, _| {
                *iq.borrow_mut() = text.trim().to_lowercase();
                iw.wake();
            });
        });
        Self {
            group: Rc::new(RefCell::new(0usize)),
            query,
            search,
            dismiss: Rc::new(RefCell::new(false)),
            dev_mode: Rc::new(RefCell::new(DevMode::SideBySide)),
            dev_version: Rc::new(RefCell::new(None)),
            dev_split: Rc::new(RefCell::new(0.5)),
            dev_hovered: Rc::new(RefCell::new(None)),
            ver_form: Rc::new(RefCell::new(None)),
            ver_prefilled: Rc::new(RefCell::new(None)),
            ver_confirm: Rc::new(RefCell::new(None)),
            ver_name,
            ver_desc,
            assets_tab: Rc::new(RefCell::new(false)),
            assets_query,
            assets_search,
            home_section: Rc::new(RefCell::new(0usize)),
            home_query,
            home_search,
            home_view_grid: Rc::new(RefCell::new(true)),
            template_thumbs: Rc::new(RefCell::new(std::collections::HashMap::new())),
            home_sort: Rc::new(RefCell::new(1)),
            icons_query,
            icons_search,
            icon_thumbs: Rc::new(RefCell::new(std::collections::HashMap::new())),
        }
    }
}

/// 居中浮层（chrome）+ 对应面板的 body。
pub(crate) fn render(kind: &ExtraKind, app: &CanvasApp, cmds: &Cmds, state: &ExtraState, cx: &mut App, close: Rc<dyn Fn()>) -> gpui::Stateful<Div> {
    let t = tokens();
    let body = match kind {
        ExtraKind::Shortcuts => shortcuts_body(state, cmds, cx),
        ExtraKind::Versions => versions_body(app, cmds, state, cx),
        ExtraKind::DevCompare { id, version } => compare_body(app, cmds, state, *id, version.as_deref()),
        ExtraKind::Home => crate::home::body(app, cmds, state),
    };
    if matches!(kind, ExtraKind::Home) {
        // 1:1 with `ui-egui/src/home.rs`: `CentralPanel` fills the window, with the sidebar on
        // the left and the scrolling content on the right — not a 760x520 centred popover.
        // Start BELOW the title bar (TITLE_H): egui's Home is the `CentralPanel`, drawn under the
        // tab bar. A `size_full()` from y=0 covered the window controls, so the title bar
        // disappeared on the Home screen.
        return div()
            .id("home-screen")
            .absolute()
            .top(px(crate::title_bar::TITLE_H))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .bg(rgba(t.panel))
            // ⛔ 不要在这里放关闭按钮：egui 的 Home 是 `CentralPanel`，没有任何「关闭」控件
            // （离开靠点标签栏的首页按钮或某个标签）。之前右上角浮着的那个 ✕ 是浮层外壳的残留。
            .child(body);
    }
    let close_btn = {
        let close = close.clone();
        IconButton::new(Icon::Close).size(24.0).on_click(move |_e, _w, _cx| close())
    };
    div()
        .id("extra-panel-backdrop")
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x00000066))
        .on_mouse_down(gpui::MouseButton::Left, move |_e, _w, _cx| close())
        .child(
            div()
                .id("extra-panel-card")
                .w(px(760.0))
                .h(px(520.0))
                .flex()
                .flex_col()
                .rounded(pt(10.0))
                .bg(rgba(t.panel))
                .border_1()
                .border_color(rgba(t.border))
                .overflow_hidden()
                .on_mouse_down(gpui::MouseButton::Left, |_e, _w, cx: &mut App| cx.stop_propagation())
                .child(
                    div()
                        .flex_none()
                        .h(px(40.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px(pt(16.0))
                        .border_b_1()
                        .border_color(rgba(t.border))
                        .child(
                            div()
                                .text_size(pt(13.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgba(t.text))
                                .child(SharedString::from(kind.title())),
                        )
                        .child(close_btn),
                )
                .child(div().id("extra-body").flex_1().min_h(px(0.0)).overflow_y_scroll().child(body)),
        )
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Shortcuts（egui `shortcuts_panel.rs`）
// ───────────────────────────────────────────────────────────────────────────────────────────────

struct ShortcutRow {
    label: String,
    keys: Vec<String>,
}

/// 按分组 + 搜索词列出快捷键（数据源：引擎 `KEYS` / `GESTURES`，一个键都不手抄）。
fn shortcut_rows(group: Option<Group>, query: &str) -> Vec<ShortcutRow> {
    let q = query.trim().to_lowercase();
    let wanted = |g: Group, label: &str| group.is_none_or(|x| x == g) && (q.is_empty() || label.to_lowercase().contains(&q));
    let platform = Platform::current();
    let mut out: Vec<ShortcutRow> = KEYS
        .iter()
        .map(|b| (b, b.title().to_string()))
        .filter(|(b, label)| wanted(b.group, label))
        .map(|(b, label)| {
            let mut keys: Vec<String> = Vec::with_capacity(2);
            for k in b.keys(platform) {
                let shown = figcraft_engine::keymap::display(k, platform);
                if keys.len() < 2 && !keys.contains(&shown) && !shown.is_empty() {
                    keys.push(shown);
                }
            }
            ShortcutRow { label, keys }
        })
        .collect();
    for (g, label, mac, other) in GESTURES.iter().filter(|(g, label, ..)| wanted(*g, label)) {
        let text = if platform == Platform::Mac { *mac } else { *other };
        let _ = g;
        out.push(ShortcutRow { label: (*label).to_string(), keys: vec![text.to_string()] });
    }
    out
}

fn shortcuts_body(state: &ExtraState, cmds: &Cmds, _cx: &mut App) -> Div {
    let t = tokens();
    let group = (*state.group.borrow()).min(Group::ALL.len() - 1);
    let query = state.query.borrow().clone();
    // 分组下拉：点一下切下一组（浮层不持有根视图，没法用 `open_menu`；12 组循环切换，
    // 搜索框可直接跨组找，功能上等价）。
    let mut group_dd = Dropdown::new(SharedString::from(Group::ALL[group].label()), 160.0, None).fill();
    let store = state.group.clone();
    let wake = cmds.clone();
    group_dd = group_dd.on_click(move |_anchor, _window, _cx| {
        let mut cur = store.borrow_mut();
        *cur = (*cur + 1) % Group::ALL.len();
        drop(cur);
        wake.wake();
    });

    let list = shortcut_rows(if query.is_empty() { Some(Group::ALL[group]) } else { None }, &query);
    let mut body = div().flex().flex_col().w_full().p(pt(16.0)).gap(pt(8.0));
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .w_full()
            .child(group_dd)
            .child(state.search.clone())
            .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(format!("{} shortcuts", list.len())))),
    );
    if list.is_empty() {
        return body.child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("No shortcuts match")));
    }
    for r in list.iter().take(120) {
        let mut line = div().flex().flex_row().items_center().w_full().h(px(26.0)).gap(pt(6.0));
        line = line.child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(r.label.clone())));
        for k in &r.keys {
            line = line.child(
                div()
                    .flex_none()
                    .px(pt(5.0))
                    .py(pt(2.0))
                    .rounded(pt(4.0))
                    .bg(rgba(t.field))
                    .border_1()
                    .border_color(rgba(t.border))
                    .text_size(pt(10.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(k.clone())),
            );
        }
        body = body.child(line);
    }
    body
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// 共用小部件：相对时间、文本按钮、带 stop_propagation 的图标动作
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 把 Unix 秒转成「3 minutes ago」式的相对时间（不依赖 chrono，避免引入新依赖）。
fn time_ago(unix_secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(unix_secs);
    let diff = now.saturating_sub(unix_secs);
    let (n, unit) = if diff < 60 {
        (diff.max(1), "second")
    } else if diff < 3600 {
        (diff / 60, "minute")
    } else if diff < 86400 {
        (diff / 3600, "hour")
    } else if diff < 604800 {
        (diff / 86400, "day")
    } else if diff < 2_629_746 {
        (diff / 604800, "week")
    } else if diff < 31_556_952 {
        (diff / 2_629_746, "month")
    } else {
        (diff / 31_556_952, "year")
    };
    if n <= 1 { format!("{unit} ago") } else { format!("{n} {unit}s ago") }
}

/// 小号文本按钮（主要=accent 底白字，次要=field 底）。
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

/// 图标动作按钮：点击时阻止冒泡，避免触发所在行的整行点击（egui 的整行预览）。
fn icon_act(icon: Icon, size: f32, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> IconButton {
    IconButton::new(icon).size(size).on_click(move |e, w, cx| {
        cx.stop_propagation();
        on_click(e, w, cx);
    })
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Versions（egui `versions.rs`）
// ───────────────────────────────────────────────────────────────────────────────────────────────

fn versions_body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, cx: &mut App) -> Div {
    let t = tokens();
    let Ok(ds) = app.session.ds() else { return note("No document open.") };

    // 打开 / 收起「保存新版本」表单。
    let vt = state.ver_form.clone();
    let vw = cmds.clone();
    let save_btn = IconButton::new(Icon::Plus).size(24.0).on_click(move |_e, _w, _cx| {
        let mut f = vt.borrow_mut();
        *f = if f.is_some() { None } else { Some(None) };
        vw.wake();
    });
    let mut sec = PanelSection::new(tr("Version history")).header_trail(save_btn);

    // 预览横幅：正在查看某版本时，给出「回到当前 / 恢复此版本」。
    if let Some(p) = &ds.preview {
        let pid = p.id.clone();
        let nm = p.name.clone();
        let pr = cmds.clone();
        let pe = cmds.clone();
        sec = sec.child(field_row_full(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(8.0))
                .w_full()
                .bg(rgba(t.selected_row))
                .rounded(pt(6.0))
                .px(pt(8.0))
                .py(pt(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_col()
                        .child(
                            div()
                                .text_size(pt(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgba(t.text))
                                .child(SharedString::from(format!("Viewing “{nm}”"))),
                        )
                        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(time_ago(p.time)))),
                )
                .child(pill(tr("Back to current"), false, move |_e, _w, _cx| {
                    pr.push(Cmd { id: "version.exitPreview", params: None });
                }))
                .child(pill(tr("Restore"), true, move |_e, _w, _cx| {
                    pe.push(Cmd { id: "version.restore", params: Some(json!({ "id": pid })) });
                })),
        ));
    }

    // 保存 / 重命名表单（标题 + 描述，提交走 file.saveVersion / version.rename）。
    if let Some(editing) = state.ver_form.borrow().clone() {
        if *state.ver_prefilled.borrow() != Some(editing.clone()) {
            let (n, d) = match editing.clone() {
                Some(id) => ds.versions.iter().find(|v| v.id == id).map(|v| (v.name.clone(), v.description.clone())).unwrap_or_default(),
                None => (String::new(), String::new()),
            };
            state.ver_name.update(cx, |tf, _| tf.set_value(n));
            state.ver_desc.update(cx, |tf, _| tf.set_value(d));
            *state.ver_prefilled.borrow_mut() = Some(editing);
        }
        let vt = state.ver_form.clone();
        let vp = state.ver_prefilled.clone();
        let vn = state.ver_name.clone();
        let vd = state.ver_desc.clone();
        let cm = cmds.clone();
        let submit = pill(tr("Save"), true, move |_e, w, cx| {
            let name = vn.read(cx).value(cx).trim().to_string();
            let desc = vd.read(cx).value(cx).trim().to_string();
            match vt.borrow().clone() {
                Some(Some(id)) => cm.push(Cmd { id: "version.rename", params: Some(json!({ "id": id, "name": name, "description": desc })) }),
                Some(None) => cm.push(Cmd { id: "file.saveVersion", params: Some(json!({ "name": name, "description": desc })) }),
                None => {}
            }
            *vt.borrow_mut() = None;
            *vp.borrow_mut() = None;
        });
        let vt2 = state.ver_form.clone();
        let vp2 = state.ver_prefilled.clone();
        let cancel = pill(tr("Cancel"), false, move |_e, _w, _cx| {
            *vt2.borrow_mut() = None;
            *vp2.borrow_mut() = None;
        });
        sec = sec.child(field_row_full(
            div()
                .flex()
                .flex_col()
                .gap(pt(6.0))
                .w_full()
                .child(state.ver_name.clone())
                .child(state.ver_desc.clone())
                .child(div().flex().flex_row().gap(pt(8.0)).child(submit).child(cancel)),
        ));
    }

    if ds.versions.is_empty() {
        return div().w_full().child(sec.child(field_row_full(note("No saved versions yet. Save a version to mark a point you can come back to."))));
    }

    for v in ds.versions.iter().rev() {
        let vid = v.id.clone();
        // 删除确认行。
        if *state.ver_confirm.borrow() == Some(v.id.clone()) {
            let vc = state.ver_confirm.clone();
            let cm = cmds.clone();
            let yes = pill(tr("Delete"), true, move |_e, _w, _cx| {
                cm.push(Cmd { id: "version.delete", params: Some(json!({ "id": vid })) });
                *vc.borrow_mut() = None;
            });
            let vc2 = state.ver_confirm.clone();
            let no = pill(tr("Cancel"), false, move |_e, _w, _cx| {
                *vc2.borrow_mut() = None;
            });
            sec = sec.child(field_row_full(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text))
                            .child(SharedString::from(format!("Delete “{}”? This can't be undone.", v.name))),
                    )
                    .child(yes)
                    .child(no),
            ));
            continue;
        }

        let label = if v.auto { format!("{} (auto)", v.name) } else { v.name.clone() };
        // 整行点击 = 预览（与 egui 一致）；按钮带 stop_propagation 防止冒泡。
        let vcp = vid.clone();
        let cm_preview = cmds.clone();
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .w_full()
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, _cx| {
                cm_preview.push(Cmd { id: "version.preview", params: Some(json!({ "id": vcp })) });
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex_col()
                    .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(label)))
                    .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(time_ago(v.time))))
                    .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(v.description.clone()))),
            );
        let cm1 = cmds.clone();
        let id1 = vid.clone();
        let preview = icon_act(Icon::Eye, 18.0, move |_e, _w, _cx| cm1.push(Cmd { id: "version.preview", params: Some(json!({ "id": id1 })) }));
        let cm2 = cmds.clone();
        let id2 = vid.clone();
        let restore = icon_act(Icon::Rotate, 18.0, move |_e, _w, _cx| cm2.push(Cmd { id: "version.restore", params: Some(json!({ "id": id2 })) }));
        let vt = state.ver_form.clone();
        let vp = state.ver_prefilled.clone();
        let id3 = vid.clone();
        let rename = icon_act(Icon::Pencil, 18.0, move |_e, _w, _cx| {
            *vt.borrow_mut() = Some(Some(id3.clone()));
            *vp.borrow_mut() = None;
        });
        let vc = state.ver_confirm.clone();
        let id4 = vid.clone();
        let del = icon_act(Icon::Trash, 18.0, move |_e, _w, _cx| {
            *vc.borrow_mut() = Some(id4.clone());
        });
        let cm3 = cmds.clone();
        let id5 = vid.clone();
        let dup = pill("Dup", false, move |_e, _w, _cx| cm3.push(Cmd { id: "version.duplicate", params: Some(json!({ "id": id5 })) }));
        sec = sec.child(field_row_full(row.child(preview).child(restore).child(rename).child(dup).child(del)));
    }
    div().w_full().child(sec)
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Dev compare（egui `dev_compare.rs`）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// 叠加分割滑块的拖动几何：`STAGE_RECT` 由 stage 内的 canvas 每帧记下 stage 的 bounds，
/// `DRAGGING` 记是否正在拖（与 timeline 同思路：指针位置是窗口坐标，减原点得局部 x）。
thread_local! {
    static STAGE_RECT: std::cell::RefCell<Option<gpui::Bounds<gpui::Pixels>>> = const { std::cell::RefCell::new(None) };
    static DRAGGING: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
}

/// 节点渲染成可显示图像（与 `control.rs` 同路：`render_node` → `screen_image`）。
fn render_thumb(doc: &figcraft_doc::Document, id: figcraft_doc::NodeId) -> Option<(Arc<RenderImage>, u32, u32)> {
    let area = figcraft_render::export_bounds(doc, id)?;
    let longest = area.width().max(area.height()).max(1.0);
    let scale = (560.0 / longest).clamp(0.05, 2.0);
    let pix = figcraft_render::render_node(doc, id, scale)?;
    let (w, h) = (pix.width(), pix.height());
    let mut data = figcraft_render::to_rgba8(&pix);
    crate::canvas::render_worker::demultiply(&mut data);
    let img = crate::canvas::render_worker::wrap_rgba(w, h, data);
    Some((img, w, h))
}

/// 把指针窗口坐标映射到 stage 内分割比例（0–1）。
fn update_split(state: &ExtraState, pos: Point<gpui::Pixels>) {
    STAGE_RECT.with(|s| {
        if let Some(b) = s.borrow().as_ref() {
            let x = f32::from(pos.x) - f32::from(b.origin.x);
            let w = f32::from(b.size.width).max(1.0);
            *state.dev_split.borrow_mut() = (x / w).clamp(0.0, 1.0);
        }
    });
}

/// 改动层的彩色边框（按 egui：新增绿 / 移除红 / 改动脉冲色；悬停的填充半透明）。
fn layer_boxes(
    before_doc: &figcraft_doc::Document,
    id: figcraft_doc::NodeId,
    changes: &[figcraft_engine::devcompare::LayerChange],
    dw: f32,
    dh: f32,
    hovered: Option<figcraft_doc::NodeId>,
) -> Vec<Div> {
    let Some(area) = figcraft_render::export_bounds(before_doc, id) else { return Vec::new() };
    let sx = dw as f64 / area.width().max(1.0);
    let sy = dh as f64 / area.height().max(1.0);
    let mut out = Vec::new();
    for c in changes.iter().filter(|c| c.depth > 0) {
        let Some(b) = before_doc.world_bounds(c.id) else { continue };
        let col = match c.change {
            figcraft_engine::devcompare::ChangeKind::Added => rgb(0x4caf50),
            figcraft_engine::devcompare::ChangeKind::Removed => rgb(0xe04a4a),
            figcraft_engine::devcompare::ChangeKind::Changed => rgb(0xe8a033),
        };
        let bx = ((b.x0 - area.x0) * sx) as f32;
        let by = ((b.y0 - area.y0) * sy) as f32;
        let bw = (b.width() * sx) as f32;
        let bh = (b.height() * sy) as f32;
        let hot = hovered == Some(c.id);
        let fill = Hsla::from(col).opacity(0.18);
        let mut box_el = div().absolute().top(px(by)).left(px(bx)).w(px(bw.max(1.0))).h(px(bh.max(1.0))).border_1().border_color(col);
        if hot {
            box_el = box_el.bg(fill);
        }
        out.push(box_el);
    }
    out
}

fn compare_body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, id: Option<figcraft_doc::NodeId>, version: Option<&str>) -> Div {
    let t = tokens();
    let Ok(ds) = app.session.ds() else { return note("No document open.") };
    let doc = &ds.doc;
    let target = id.or_else(|| ds.selection.first().copied()).or_else(|| ds.focused());
    let Some(id) = target else {
        return note("Select a frame or section marked ready for dev to compare it.");
    };
    // 版本：面板内选中的优先（ExtraState），否则打开时带进来的。
    let chosen = state.dev_version.borrow().clone().or_else(|| version.map(str::to_string));
    let version_doc = chosen.as_ref().and_then(|v| ds.versions.iter().find(|x| &x.id == v).and_then(|x| x.document().ok()));
    let (cmp, before_doc) = match &version_doc {
        Some(b) => (figcraft_engine::devcompare::compare_with(b, doc, id), Some(b.as_ref().clone())),
        None if chosen.is_some() => (figcraft_engine::devcompare::empty(doc, id), None),
        None => (figcraft_engine::devcompare::compare(doc, id), figcraft_engine::devcompare::before_doc(doc, id)),
    };
    let after = render_thumb(doc, id);
    let before = before_doc.as_ref().and_then(|b| render_thumb(b, id));

    let mut body = div().flex().flex_col().w_full().h_full().gap(pt(8.0)).p(pt(12.0));
    let summary = format!("{} · {} added · {} removed · {} modified", cmp.name, cmp.added, cmp.removed, cmp.modified);
    body = body.child(div().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(summary)));
    if let Some(v) = &chosen {
        if let Some(name) = ds.versions.iter().find(|x| &x.id == v).map(|x| x.name.clone()) {
            body = body
                .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(format!("Compared to version {name}"))));
        }
    }

    // 控制行：Compare with + 并排/叠加切换。
    let opts: Vec<Option<String>> = std::iter::once(None).chain(ds.versions.iter().map(|v| Some(v.id.clone()))).collect();
    let cur_idx = opts.iter().position(|o| o == &chosen).unwrap_or(0);
    let cur_name =
        chosen.as_ref().and_then(|v| ds.versions.iter().find(|x| &x.id == v).map(|x| x.name.clone())).unwrap_or_else(|| tr("When marked ready").into());
    let vstate = state.dev_version.clone();
    let vwake = cmds.clone();
    let mut version_dd = Dropdown::new(SharedString::from(cur_name), 200.0, None).fill();
    version_dd = version_dd.on_click(move |_anchor, _window, _cx| {
        let next = (cur_idx + 1) % opts.len().max(1);
        *vstate.borrow_mut() = opts[next].clone();
        vwake.wake();
    });
    let mstate = state.dev_mode.clone();
    let mwake = cmds.clone();
    let mode_label = match *mstate.borrow() {
        DevMode::SideBySide => tr("Side by side"),
        DevMode::Overlay => tr("Overlay"),
    };
    let mut mode_dd = Dropdown::new(SharedString::from(mode_label), 130.0, None).fill();
    mode_dd = mode_dd.on_click(move |_anchor, _window, _cx| {
        let next = match *mstate.borrow() {
            DevMode::SideBySide => DevMode::Overlay,
            DevMode::Overlay => DevMode::SideBySide,
        };
        *mstate.borrow_mut() = next;
        mwake.wake();
    });
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .w_full()
            .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(tr("Compare with")))
            .child(version_dd)
            .child(mode_dd),
    );

    // 主区：左图 / 右改动列表。
    let mode = *state.dev_mode.borrow();
    let split = *state.dev_split.borrow();
    let hovered = *state.dev_hovered.borrow();
    let (aw, ah) = after.as_ref().map(|(_, w, h)| (*w as f32, *h as f32)).unwrap_or((1.0, 1.0));
    let max_w = 360.0;
    let max_h = 300.0;
    let s = (max_w / aw.max(1.0)).min(max_h / ah.max(1.0)).clamp(0.01, 2.0);
    let dw = (aw * s).max(1.0);
    let dh = (ah * s).max(1.0);

    let sstate = state.clone();
    let swake = cmds.clone();
    let stage = div()
        .id("compare-stage")
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .relative()
        .on_mouse_down(MouseButton::Left, {
            let st = sstate.clone();
            let w = swake.clone();
            move |e: &MouseDownEvent, _window, _cx| {
                DRAGGING.with(|d| *d.borrow_mut() = true);
                update_split(&st, e.position);
                w.wake();
            }
        })
        .on_mouse_move({
            let st = sstate.clone();
            let w = swake.clone();
            move |e: &MouseMoveEvent, _window, _cx| {
                if DRAGGING.with(|d| *d.borrow()) {
                    update_split(&st, e.position);
                    w.wake();
                }
            }
        })
        .on_mouse_up(MouseButton::Left, move |_e: &MouseUpEvent, _window, _cx| {
            DRAGGING.with(|d| *d.borrow_mut() = false);
        })
        .child(
            canvas(
                |_, _, _| (),
                move |b, _, _, _| {
                    STAGE_RECT.with(|s| *s.borrow_mut() = Some(b.clone()));
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
        .child({
            let mut images = div().relative().flex().items_center().justify_center().w_full().h_full();
            let after_el = after.as_ref().map(|(tex, _, _)| img(ImageSource::Render(tex.clone())).w(px(dw)).h(px(dh)));
            let before_el = before.as_ref().map(|(tex, _, _)| img(ImageSource::Render(tex.clone())).w(px(dw)).h(px(dh)));
            match mode {
                DevMode::SideBySide => {
                    if let (Some(b), Some(a)) = (before_el, after_el) {
                        images = images.flex_row().gap(pt(16.0)).child(b).child(a);
                    }
                }
                DevMode::Overlay => {
                    if let Some(a) = after_el {
                        images = images.child(a);
                        if let Some(b) = before_el {
                            images = images.child(div().absolute().top(px(0.0)).left(px(0.0)).w(px(dw * split)).h(px(dh)).overflow_hidden().child(b));
                        }
                        images =
                            images.child(div().absolute().top(px(0.0)).left(px(dw * split - 1.0)).w(px(1.0)).h(px(dh)).bg(rgba(t.accent)).child(
                                div().absolute().top(px(dh / 2.0 - 8.0)).left(px(-5.0)).w(px(12.0)).h(px(16.0)).rounded(px(6.0)).bg(rgba(t.accent)),
                            ));
                    }
                }
            }
            if let Some(bd) = before_doc.as_ref() {
                for box_el in layer_boxes(bd, id, &cmp.changes, dw, dh, hovered) {
                    images = images.child(box_el);
                }
            }
            images
        });

    let hstate = state.dev_hovered.clone();
    let mut list = div().id("compare-changes").flex_none().w(px(300.0)).h_full().min_h(px(0.0)).flex().flex_col().overflow_y_scroll().child(
        div()
            .text_size(pt(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.text))
            .child(SharedString::from(format!("{} changes", cmp.changes.len()))),
    );
    for c in &cmp.changes {
        let cid = c.id;
        let c2 = cmds.clone();
        let h2 = hstate.clone();
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .w_full()
            .min_h(px(24.0))
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _window, _cx| {
                *h2.borrow_mut() = Some(cid);
                c2.push(Cmd { id: "select.set", params: Some(json!({ "ids": [cid] })) });
            })
            .on_mouse_move({
                let h3 = hstate.clone();
                move |_e: &MouseMoveEvent, _window, _cx| {
                    *h3.borrow_mut() = Some(cid);
                }
            });
        let tag_col = match c.change {
            figcraft_engine::devcompare::ChangeKind::Added => rgb(0x4caf50),
            figcraft_engine::devcompare::ChangeKind::Removed => rgb(0xe04a4a),
            _ => rgb(0xe8a033),
        };
        row = row.child(div().flex_none().text_size(pt(9.0)).text_color(tag_col).child(SharedString::from(c.change.label().to_string())));
        row = row.child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(c.name.clone())));
        list = list.child(row);
        for p in &c.properties {
            list = list.child(
                div()
                    .pl(px(16.0))
                    .text_size(pt(10.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(format!("{} · {}", p.section, p.text))),
            );
        }
    }

    body = body.child(div().flex().flex_row().w_full().flex_1().min_h(px(0.0)).gap(pt(8.0)).child(stage).child(list));

    // 底栏：Resolve + Close。
    let resolve = icon_cmd(Icon::Check, "dev.resolveChange", json!({ "id": id }), cmds);
    let dstate = state.dismiss.clone();
    let close_btn = IconButton::new(Icon::Close).size(24.0).on_click(move |_e, _w, _cx| *dstate.borrow_mut() = true);
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .child(resolve)
            .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(tr("Mark changes as reviewed")))
            .child(close_btn),
    );
    body
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Assets（egui `assets_panel.rs`）
// ───────────────────────────────────────────────────────────────────────────────────────────────

pub(crate) fn assets_body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState, _cx: &mut App) -> Div {
    let t = tokens();
    let Ok(ds) = app.session.ds() else { return note("No document open.") };
    let doc = &ds.doc;
    let q = state.assets_query.borrow().clone();

    // 顶部：Components / Styles 切换 + 搜索框。
    let on_components = *state.assets_tab.borrow() == false;
    let on_styles = *state.assets_tab.borrow() == true;
    let tab_components = {
        let st = state.assets_tab.clone();
        let wake = cmds.clone();
        pill(tr("Components"), on_components, move |_e, _w, _cx| {
            *st.borrow_mut() = false;
            wake.wake();
        })
    };
    let tab_styles = {
        let st = state.assets_tab.clone();
        let wake = cmds.clone();
        pill(tr("Styles"), on_styles, move |_e, _w, _cx| {
            *st.borrow_mut() = true;
            wake.wake();
        })
    };
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .w_full()
        .child(tab_components)
        .child(tab_styles)
        .child(div().flex_1().min_w(px(0.0)))
        .child(state.assets_search.clone());

    let body = if on_components { assets_components(doc, cmds, &q) } else { assets_styles(doc, cmds, &q) };
    wrap_header(body, header)
}

/// 把标题栏 + 搜索接到 body 前面（避免每个分支重复）。
fn wrap_header(body: Div, header: Div) -> Div {
    div().flex().flex_col().w_full().gap(pt(12.0)).p(pt(16.0)).child(header).child(body)
}

/// Components tab：本地组件/组件集缩略图网格（2 列，GPUI 无 flex_wrap），点一下创建实例。
fn assets_components(doc: &figcraft_doc::Document, cmds: &Cmds, q: &str) -> Div {
    let comps: Vec<figcraft_doc::NodeId> = figcraft_engine::library::local_components(doc)
        .into_iter()
        .filter(|id| doc.node(*id).is_some_and(|n| n.name.to_lowercase().contains(q)))
        .collect();
    let mut grid = div().flex().flex_col().w_full().gap(pt(10.0));
    if comps.is_empty() {
        return grid.child(note(if q.is_empty() {
            "No local components. Create one with Component ▸ Create component."
        } else {
            "No components match your search."
        }));
    }
    for chunk in comps.chunks(2) {
        let mut row = div().flex().flex_row().w_full().gap(pt(10.0));
        for id in chunk {
            row = row.child(component_tile(doc, cmds, *id));
        }
        // 末行若只有一列，补一个占位保持两列等高对齐。
        if chunk.len() == 1 {
            row = row.child(div().flex_1().min_w(px(0.0)));
        }
        grid = grid.child(row);
    }
    grid
}

/// 单个组件缩略图卡片：渲染缩略图 + 名称，整卡点击创建实例。
fn component_tile(doc: &figcraft_doc::Document, cmds: &Cmds, id: figcraft_doc::NodeId) -> Div {
    let t = tokens();
    let Some(n) = doc.node(id) else { return div() };
    let shown = if matches!(n.kind, NodeKind::ComponentSet { .. }) { default_variant(doc, id).unwrap_or(id) } else { id };
    let thumb = render_thumb(doc, shown);
    let name = n.name.clone();
    let is_set = matches!(n.kind, NodeKind::ComponentSet { .. });

    // 缩略图区域：渲染成功则等比缩放进 120×120 框，失败则灰底占位。
    let thumb_el = match thumb {
        Some((tex, w, h)) => {
            let (w, h) = (w as f32, h as f32);
            let longest = w.max(h).max(1.0);
            let scale = 120.0 / longest;
            div().child(img(ImageSource::Render(tex)).w(px(w * scale)).h(px(h * scale)))
        }
        None => div().size_full().bg(rgba(t.field_hover_border)),
    };

    let c = cmds.clone();
    let create = move |_e: &MouseDownEvent, _w: &mut Window, _cx: &mut App| {
        c.push(Cmd { id: "instance.create", params: Some(json!({ "component": id })) });
    };
    div()
        .flex_1()
        .min_w(px(0.0))
        .rounded(pt(8.0))
        .border_1()
        .border_color(rgba(t.border))
        .bg(rgba(t.panel))
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, create.clone())
        .cursor_pointer()
        .child(div().h(px(120.0)).flex().items_center().justify_center().p(pt(8.0)).bg(rgba(t.field)).child(thumb_el))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .w_full()
                .px(pt(8.0))
                .py(pt(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_size(pt(11.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgba(t.text))
                        .child(SharedString::from(name)),
                )
                .child(div().text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(if is_set {
                    "Set"
                } else {
                    tr("Component")
                }))),
        )
}

/// Styles tab：本地样式列表，按类型给图标/色板，点一行把样式应用到当前选区。
fn assets_styles(doc: &figcraft_doc::Document, cmds: &Cmds, q: &str) -> Div {
    let t = tokens();
    let styles: Vec<&Style> = figcraft_engine::library::local_styles(doc).filter(|s| s.name.to_lowercase().contains(q)).collect();
    let mut list = div().flex().flex_col().w_full().gap(pt(2.0));
    if styles.is_empty() {
        return list.child(note(if q.is_empty() { "No local styles." } else { "No styles match your search." }));
    }
    for s in styles {
        let name = s.name.clone();
        let id = serde_json::to_value(&s.id).unwrap_or(Value::Null);
        let c = cmds.clone();
        let sid = id.clone();
        let apply = move |_e: &MouseDownEvent, _w: &mut Window, _cx: &mut App| {
            c.push(Cmd { id: "style.apply", params: Some(json!({ "style": sid })) });
        };
        let badge = style_badge(s);
        list = list.child(
            div()
                .w_full()
                .h(px(32.0))
                .rounded(pt(6.0))
                .on_mouse_down(MouseButton::Left, apply)
                .cursor_pointer()
                .hover(|h| h.bg(rgba(t.hover)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(pt(8.0))
                        .w_full()
                        .h_full()
                        .px(pt(8.0))
                        .child(badge)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_size(pt(11.0))
                                .text_color(rgba(t.text))
                                .child(SharedString::from(name)),
                        )
                        .child(div().text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(s.kind.type_name()))),
                ),
        );
    }
    list
}

/// 样式类型徽标：Paint 用色板（首个纯色填充），Text/Grid 用图标，Effect 用「fx」文字徽标。
fn style_badge(s: &Style) -> Div {
    fn boxed(el: impl IntoElement) -> Div {
        div().flex_none().w(px(18.0)).h(px(18.0)).flex().items_center().justify_center().child(el)
    }
    let t = tokens();
    match &s.kind {
        StyleKind::Paint { paints } => {
            let col = paints
                .first()
                .and_then(|p| match &p.kind {
                    PaintKind::Solid { color } => Some(*color),
                    _ => None,
                })
                .unwrap_or(Color::DEFAULT_SHAPE);
            boxed(
                div()
                    .size(px(14.0))
                    .rounded(px(3.0))
                    .bg(gpui::Rgba { r: col.r, g: col.g, b: col.b, a: col.a })
                    .border_1()
                    .border_color(rgba(t.border)),
            )
        }
        StyleKind::Text { .. } => boxed(IconButton::new(Icon::Text).size(14.0)),
        StyleKind::Grid { .. } => boxed(IconButton::new(Icon::ViewGrid).size(14.0)),
        StyleKind::Effect { .. } => {
            boxed(div().text_size(pt(9.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text_secondary)).child(SharedString::from("fx")))
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────────────────────────

fn icon_cmd(icon: Icon, id: &'static str, params: Value, cmds: &Cmds) -> IconButton {
    let c = cmds.clone();
    IconButton::new(icon).size(24.0).on_click(move |_e, _w, _cx| c.push(Cmd { id, params: Some(params.clone()) }))
}

/// A1 Home — 1:1 with `ui-egui/src/home.rs`: a 240 pt sidebar (wordmark, file search, section
/// tabs) and a scrolling main area with `action_card`s, the recent-file list and the six tips.
/// All data and actions come from the engine (`prefs.recent_files`, `prefs.getting_started_dismissed`,
/// `file.new` / `file.newBoard` / `file.open` / `file.openRecent` / `file.newFromTemplate`).
fn heading(text: &'static str) -> Div {
    div().w_full().text_size(pt(20.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(tokens().text)).child(text)
}

fn subheading(text: &'static str) -> Div {
    div().w_full().text_size(pt(13.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(tokens().text)).child(text)
}

/// `ui-egui/src/home.rs::action_card`: 260x72, a 32 pt accent icon tile, title + sub line.
fn action_card(icon: Icon, title: &'static str, sub: &'static str, id: &'static str, cmds: &Cmds) -> Div {
    let t = tokens();
    let c = cmds.clone();
    div()
        .flex_none()
        .w(px(260.0))
        .h(px(72.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(12.0))
        .px(pt(16.0))
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(
            div()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(pt(6.0))
                .bg(rgba(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(icon, rgba(t.accent_on), 16.0)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(pt(4.0))
                .child(div().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(title))
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(sub)),
        )
        .on_mouse_down(MouseButton::Left, move |_e, _, _| c.push(Cmd { id, params: None }))
}


fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().p(pt(16.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_engine::Session;
    use serde_json::json;

    /// D 类通道的地基：这些命令确实只发 ui_requests，UI 每帧取走即可接线。
    #[test]
    fn engine_commands_emit_ui_requests() {
        let mut s = Session::with_document(figcraft_doc::Document::new("ui-req"));
        for (cmd, want) in [
            ("view.devMode", "panel.devMode"),
            ("view.designPrototype", "panel.designPrototype"),
            ("view.design", "panel.design"),
            ("view.prototype", "panel.prototype"),
            ("help.shortcuts", "panel.shortcuts"),
            ("file.versionHistory", "panel.versionHistory"),
            ("view.assets", "panel.assets"),
            ("view.devMode", "panel.devMode"),
        ] {
            let _ = s.execute(cmd, &json!({}));
            let last = s.ui_requests.last().map(|r| r.kind.as_str()).unwrap_or("");
            assert_eq!(last, want, "{cmd} 应发出 {want}");
        }
    }

    /// Shortcuts 面板不自己抄键位：全部来自引擎 keymap，且分组 / 搜索生效。
    #[test]
    fn shortcuts_come_from_the_engine_keymap() {
        let all = shortcut_rows(None, "");
        assert!(all.len() > 50, "引擎 KEYS + GESTURES 应给出完整清单");
        assert!(all.iter().any(|r| r.label.to_lowercase().contains("dev mode")), "Shift+D 在表里");
        assert!(all.iter().any(|r| !r.keys.is_empty()), "至少有一行带 keycap");
        let essential = shortcut_rows(Some(Group::Essential), "");
        assert!(!essential.is_empty() && essential.len() < all.len());
        let searched = shortcut_rows(None, "shortcuts");
        assert!(!searched.is_empty());
    }

    /// kind → UI 行为的分派逐条可验证（D 类通道的核心）。
    #[test]
    fn ui_request_dispatch_covers_every_panel() {
        let empty = json!({});
        assert_eq!(ui_action("panel.design", &empty, 2), Some(UiAction::Tab(0)));
        assert_eq!(ui_action("panel.prototype", &empty, 0), Some(UiAction::Tab(1)));
        assert_eq!(ui_action("panel.devMode", &empty, 0), Some(UiAction::Tab(2)), "Shift+D 进 Inspect");
        assert_eq!(ui_action("panel.devMode", &empty, 2), Some(UiAction::Tab(0)), "再按一次回 Design");
        assert_eq!(ui_action("panel.designPrototype", &empty, 0), Some(UiAction::Tab(1)));
        assert_eq!(ui_action("panel.shortcuts", &empty, 0), Some(UiAction::Toggle(ExtraKind::Shortcuts)));
        assert_eq!(ui_action("panel.versionHistory", &empty, 0), Some(UiAction::Toggle(ExtraKind::Versions)));
        assert_eq!(ui_action("view.assets", &empty, 0), None, "Assets 请求是 panel.assets，不是命令名");
        assert_eq!(ui_action("panel.assets", &empty, 0), Some(UiAction::LeftTab(1)), "Assets 切到左面板 tab，不弹浮层（R1）");
        assert_eq!(ui_action("panel.libraries", &empty, 0), Some(UiAction::LeftTab(1)));
        assert_eq!(ui_action("view.devCompare", &json!({"version": "v1"}), 0), Some(UiAction::Compare { id: None, version: Some("v1".into()) }));
        assert_eq!(ui_action("panel.widths", &json!({"left": 320.0}), 0), Some(UiAction::Widths { left: Some(320.0), right: None }));
        assert_eq!(ui_action("palette.actions", &empty, 0), Some(UiAction::Palette));
        assert_eq!(ui_action("home", &empty, 0), Some(UiAction::Toggle(ExtraKind::Home)));
        assert_eq!(
            ui_action("present", &json!({"start": 7}), 0),
            Some(UiAction::Present(Some(figcraft_doc::NodeId(7)), false)),
            "proto.present → 演示层"
        );
        assert_eq!(ui_action("preview", &json!({"start": 7}), 0), Some(UiAction::Present(Some(figcraft_doc::NodeId(7)), true)));
        assert_eq!(ui_action("preview.close", &empty, 0), Some(UiAction::PreviewClose));
    }

    /// 四块面板的标题 / 变体都能构造（防止 ui_requests 分派时漏分支）。
    #[test]
    fn every_extra_kind_has_a_title() {
        for k in [
            ExtraKind::Shortcuts,
            ExtraKind::Versions,
            ExtraKind::DevCompare { id: None, version: Some("v1".into()) },
            ExtraKind::Home,
        ] {
            assert!(!k.title().is_empty());
        }
    }
}
