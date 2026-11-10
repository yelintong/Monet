//! The five detached panels, all assembled on the one shared window control.
//!
//! `ui-egui` builds these on `popover::Popover`: the colour picker, the layout settings, the
//! stroke settings, the variable picker and the style picker. This module is the GPUI assembly of
//! the same five, using [`crate::widgets::popover::Popover`] — whose header title is a **slot**, so
//! each window can put its own content there (a swatch, a create button, a count chip, …).
//!
//! The window owns only chrome, placement and drag. Bodies are built from the shared widget
//! library (search field, number fields, icon buttons, text), and every action goes through the
//! very same `CmdQueue` the right panel uses, so the engine sees the exact same commands.

use crate::canvas::geom::Color32;
use crate::panels::right::data::PanelData;
use crate::panels::right::variables::{self, VariablesData, VariablesEditor};
use crate::panels::{Cmd, Cmds};
use crate::widgets::popover::{self, HEADER_H, Open, Surface, WIDTH, popover_window, title_text};
use crate::widgets::{Icon, IconButton, Slot, TextField, number_field, pt, rgba, search_field, tokens};
use gpui::{
    AnyElement, App, Bounds, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString, Size, Window, div, point, prelude::*, px,
    size,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// Half of a 24 pt icon button. Callers hand us the press point; the placement rule wants the
/// button's top (`ui-egui/src/popover.rs`: "its top on `y` (the button's top)").
const BUTTON_HALF: f32 = 12.0;

/// Which of the five old popovers is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowKind {
    Layout,
    Stroke,
    Color,
    Variables,
    TextStyles,
    /// 变量编辑器的右侧列表（egui `dialogs/variables_panel.rs::table()`）。
    /// 与 `Variables`（变量**绑定选择器**弹窗）是两个不同的窗口。
    VariablesTable,
}

/// One clickable row of a list window (a variable, a style, a create action).
#[derive(Clone)]
pub(crate) struct Row {
    pub group: Option<SharedString>,
    pub label: SharedString,
    pub trail: Option<SharedString>,
    pub cmd: &'static str,
    pub params: Value,
}

impl Row {
    pub fn new(label: impl Into<SharedString>, cmd: &'static str, params: Value) -> Self {
        Self { group: None, label: label.into(), trail: None, cmd, params }
    }
    pub fn group(mut self, name: impl Into<SharedString>) -> Self {
        self.group = Some(name.into());
        self
    }
    pub fn trail(mut self, text: impl Into<SharedString>) -> Self {
        self.trail = Some(text.into());
        self
    }
}

/// One option of a setting's segmented choice (egui's `menu_item` behind a `dropdown`).
#[derive(Clone)]
pub(crate) struct Opt {
    pub label: SharedString,
    pub cmd: &'static str,
    pub params: Value,
}

impl Opt {
    pub fn new(label: impl Into<SharedString>, cmd: &'static str, params: Value) -> Self {
        Self { label: label.into(), cmd, params }
    }
}

/// One line of a settings window (Layout / Stroke).
#[derive(Clone)]
pub(crate) enum Setting {
    Caption {
        tab: u8,
        text: SharedString,
    },
    /// A labelled row of option chips; `active` is the highlighted one.
    Options {
        tab: u8,
        label: SharedString,
        active: usize,
        options: Vec<Opt>,
    },
    /// A labelled numeric field; its commit was wired to the command queue at construction.
    Number {
        tab: u8,
        label: SharedString,
        field: Entity<TextField>,
    },
}

struct ColorState {
    current: Color32,
    hex: Entity<TextField>,
    on_pick: Rc<dyn Fn(Color32, &mut Window, &mut App)>,
}

/// Everything an open detached window needs. Kept as data on the root view (a retained tree), not
/// rebuilt into a fresh element tree from a closure every frame.
pub(crate) struct WindowState {
    kind: WindowKind,
    title: SharedString,
    open: Open,
    width: f32,
    pad: f32,
    surface: Surface,
    height: f32,
    /// `Some` once the header has been dragged (egui's `Place::At`).
    place: Option<Point<Pixels>>,
    origin: Point<Pixels>,
    /// Last viewport the card was laid out in. A drag clamps every step to it, exactly like
    /// egui's `clamp(at - grab, size, screen)`, so a pointer leaving the window cannot park the
    /// card outside it.
    viewport: Size<Pixels>,
    /// Pointer minus origin at the press, while dragging.
    grab: Option<Point<Pixels>>,
    /// Where the pointer pressed, before it has moved far enough to count as a drag. A press alone
    /// must NOT start a drag (and must not repaint): GPUI matches a click's down/up by `ElementId`,
    /// so repainting on every press is what made header buttons stop responding. The drag only
    /// latches once the pointer moves past a small threshold — egui's `click_and_drag`.
    pending: Option<Point<Pixels>>,
    /// 标题栏 / 头部左侧的拖动回调（整卡拖动）。**只绑在头部左侧**，不是整张卡片：
    /// 绑整卡会让「点表格行、点按钮」都开始拖窗口（用户 2026-11-07 指出）。
    drag_start: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    cmds: Cmds,
    rows: Vec<Row>,
    settings: Rc<RefCell<Vec<Setting>>>,
    tabs: Option<(Vec<SharedString>, Rc<RefCell<u8>>)>,
    search: Option<Entity<TextField>>,
    query: Rc<RefCell<String>>,
    color: Option<ColorState>,
    /// 完整变量编辑器（只有 `VariablesTable` 用）：持有只读快照 + retained 交互状态。
    /// 延迟到 `body_element` 才创建 —— 那里才拿得到 `&mut App` 来建搜索框 / 值输入框。
    editor: Option<VariablesEditor>,
    variables_data: Option<VariablesData>,
    /// 视图级记住的 Full/Floating。`editor` 是延迟到 `body_element` 才建的，所以切换模式的
    /// 调用可能发生在建 editor 之前 —— 必须把它存下来，建好后再灌进去，否则 Full 永远不生效。
    editor_full: Option<bool>,
    var_on_full: Rc<dyn Fn(bool, &mut App)>,
    var_on_close: Rc<dyn Fn(&mut App)>,
}

impl WindowState {
    /// DIAGNOSTIC CUT glue: no-op anchor adjustment (base WindowState lacks exact_top).
    pub(crate) fn anchor_to_button_top(&mut self, _top: Pixels) {}

    /// 供根视图判断当前是否是某个特定窗口（例如变量面板已开时再点按钮就不要二次弹出）。
    pub(crate) fn kind(&self) -> WindowKind {
        self.kind
    }

    /// Base state; the five constructors below fill in the body model.
    fn base(kind: WindowKind, title: impl Into<SharedString>, cmds: Cmds, open: Open) -> Self {
        Self {
            kind,
            title: title.into(),
            open,
            width: WIDTH,
            pad: popover::SLOT_PAD,
            surface: Surface::Panel,
            height: 240.0,
            place: None,
            origin: Point { x: px(0.0), y: px(0.0) },
            viewport: size(px(0.0), px(0.0)),
            grab: None,
            pending: None,
            drag_start: None,
            cmds,
            rows: Vec::new(),
            settings: Rc::new(RefCell::new(Vec::new())),
            tabs: None,
            search: None,
            query: Rc::new(RefCell::new(String::new())),
            color: None,
            editor: None,
            variables_data: None,
            editor_full: None,
            var_on_full: Rc::new(|_, _| {}),
            var_on_close: Rc::new(|_| {}),
        }
    }

    // ── ① 自动布局设置（egui `panels/layout_panel.rs::settings`, 标题 "Layout"）───────────────
    pub(crate) fn layout(cmds: Cmds, data: &PanelData, cx: &mut App, anchor: Point<Pixels>) -> Self {
        let mut w = Self::base(WindowKind::Layout, "Layout", cmds, Open::Anchor(anchor));
        w.pad = 12.0;
        w.height = 200.0;
        let wrap = data.layout.wrap;
        let main = data.layout.main_align.unwrap_or("min");
        let gap_active = |main: &str, wrap: bool| -> usize {
            if wrap {
                if data.layout.content_align == Some("spaceBetween") { 1 } else { 0 }
            } else {
                match main {
                    "spaceBetween" => 1,
                    "spaceEvenly" => 2,
                    "spaceAround" => 3,
                    _ => 0,
                }
            }
        };
        let text_baseline = data.layout.cross_align == Some("baseline");
        let settings = vec![
            Setting::Options {
                tab: 0,
                label: "Strokes".into(),
                active: 0,
                options: vec![
                    Opt::new("Included", "node.set", json!({ "props": { "strokesIncluded": true } })),
                    Opt::new("Excluded", "node.set", json!({ "props": { "strokesIncluded": false } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: "Canvas stacking".into(),
                active: 0,
                options: vec![
                    Opt::new("Last on top", "node.set", json!({ "props": { "reverseZ": false } })),
                    Opt::new("First on top", "node.set", json!({ "props": { "reverseZ": true } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: "Text baselines".into(),
                active: usize::from(text_baseline),
                options: vec![
                    Opt::new("Off", "node.set", json!({ "props": { "crossAlign": "min" } })),
                    Opt::new("Align", "node.set", json!({ "props": { "crossAlign": "baseline" } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: "Gap mode".into(),
                active: gap_active(main, wrap),
                options: if wrap {
                    vec![
                        Opt::new("Fixed", "node.set", json!({ "props": { "contentAlign": "auto" } })),
                        Opt::new("Auto", "node.set", json!({ "props": { "contentAlign": "spaceBetween" } })),
                    ]
                } else {
                    vec![
                        Opt::new("Fixed", "node.set", json!({ "props": { "mainAlign": "min" } })),
                        Opt::new("Between", "node.set", json!({ "props": { "mainAlign": "spaceBetween" } })),
                        Opt::new("Evenly", "node.set", json!({ "props": { "mainAlign": "spaceEvenly" } })),
                        Opt::new("Around", "node.set", json!({ "props": { "mainAlign": "spaceAround" } })),
                    ]
                },
            },
        ];
        w.settings = Rc::new(RefCell::new(settings));
        let _ = cx; // no cached editor in this window yet
        w
    }

    // ── ② 描边高级设置（egui `panels/right/stroke.rs::stroke_settings_body`, 标题 "Stroke"）───
    pub(crate) fn stroke(cmds: Cmds, data: &PanelData, cx: &mut App, anchor: Point<Pixels>) -> Self {
        let mut w = Self::base(WindowKind::Stroke, "Stroke", cmds.clone(), Open::Anchor(anchor));
        w.pad = 12.0;
        w.height = 320.0;
        w.tabs = Some((vec!["Basic".into(), "Brush".into()], Rc::new(RefCell::new(0u8))));
        let weight = data.stroke_weight.unwrap_or(1.0);
        let dash = Rc::new(RefCell::new([weight * 4.0, weight * 4.0]));
        let mk_number = |cx: &mut App, value: f64, apply: Rc<dyn Fn(f64) -> Value>| -> Entity<TextField> {
            let f = number_field(cx, 100.0, Some(value), "", 1.0, Slot::None);
            let queue = cmds.clone();
            f.update(cx, move |tf, _| {
                tf.set_on_commit(move |text, _, _| {
                    if let Ok(v) = text.trim().parse::<f64>() {
                        queue.push(Cmd { id: "node.set", params: Some(apply(v)) });
                    }
                });
            });
            f
        };
        let d = dash.clone();
        let dash_value = d.borrow()[0];
        let dash_field = mk_number(
            cx,
            dash_value,
            Rc::new(move |v| {
                let mut pair = *d.borrow();
                pair[0] = v.max(0.0);
                *d.borrow_mut() = pair;
                json!({ "props": { "dashes": pair } })
            }),
        );
        let g = dash.clone();
        let gap_value = g.borrow()[1];
        let gap_field = mk_number(
            cx,
            gap_value,
            Rc::new(move |v| {
                let mut pair = *g.borrow();
                pair[1] = v.max(0.0);
                *g.borrow_mut() = pair;
                json!({ "props": { "dashes": pair } })
            }),
        );
        let miter_field = mk_number(cx, 28.6, Rc::new(|v| json!({ "props": { "miterAngle": v } })));
        let profiles = vec![
            Opt::new("Uniform", "stroke.widthProfile", json!({ "preset": "uniform" })),
            Opt::new("Wedge", "stroke.widthProfile", json!({ "preset": "wedge" })),
            Opt::new("Taper", "stroke.widthProfile", json!({ "preset": "taper" })),
            Opt::new("Eye", "stroke.widthProfile", json!({ "preset": "eye" })),
        ];
        let settings = vec![
            Setting::Caption { tab: 0, text: "Stroke style".into() },
            Setting::Options {
                tab: 0,
                label: "Pattern".into(),
                active: 0,
                options: vec![
                    Opt::new("Solid", "node.set", json!({ "props": { "dashes": [] } })),
                    Opt::new("Dash", "node.set", json!({ "props": { "dashes": [weight * 4.0, weight * 4.0] } })),
                ],
            },
            Setting::Number { tab: 0, label: "Dash".into(), field: dash_field },
            Setting::Number { tab: 0, label: "Gap".into(), field: gap_field },
            Setting::Options {
                tab: 0,
                label: "Join".into(),
                active: 0,
                options: vec![
                    Opt::new("Miter", "node.set", json!({ "props": { "strokeJoin": "miter" } })),
                    Opt::new("Bevel", "node.set", json!({ "props": { "strokeJoin": "bevel" } })),
                    Opt::new("Round", "node.set", json!({ "props": { "strokeJoin": "round" } })),
                ],
            },
            Setting::Number { tab: 0, label: "Miter angle".into(), field: miter_field },
            Setting::Options {
                tab: 0,
                label: "End caps".into(),
                active: 0,
                options: vec![
                    Opt::new("None", "node.set", json!({ "props": { "strokeCap": "butt" } })),
                    Opt::new("Round", "node.set", json!({ "props": { "strokeCap": "round" } })),
                    Opt::new("Square", "node.set", json!({ "props": { "strokeCap": "square" } })),
                ],
            },
            Setting::Options { tab: 0, label: "Width profile".into(), active: 0, options: profiles.clone() },
            Setting::Caption { tab: 1, text: "Brush".into() },
            Setting::Options {
                tab: 1,
                label: "Brush".into(),
                active: 0,
                options: vec![
                    Opt::new("None", "stroke.brush", json!({ "preset": Value::Null })),
                    Opt::new("Stretch", "stroke.brush", json!({ "preset": "stretch" })),
                    Opt::new("Scatter", "stroke.brush", json!({ "preset": "scatter" })),
                ],
            },
            Setting::Options { tab: 1, label: "Width profile".into(), active: 0, options: profiles },
        ];
        w.settings = Rc::new(RefCell::new(settings));
        w
    }

    // ── ④ 变量弹窗（egui `panels/variable_ui.rs::picker`, 标题 "Variables"）─────────────────
    pub(crate) fn variables(cmds: Cmds, cx: &mut App, anchor: Point<Pixels>, rows: Vec<Row>) -> Self {
        let mut w = Self::base(WindowKind::Variables, "Variables", cmds, Open::Anchor(anchor));
        w.surface = Surface::Menu;
        w.pad = 8.0;
        w.height = 300.0;
        w.rows = rows;
        w.add_search(cx, "Search variables");
        w
    }

    // ── ⑤ 变量编辑器右侧列表（egui `dialogs/variables_panel.rs::table()`）─────────────────
    // 只搬右边的表格：左侧集合/分组 sidebar、顶部搜索、编辑与拖拽都还没做。
    // 标题用集合名（egui 的浮动面板标题），右侧放变量总数的计数 chip。
    pub(crate) fn variables_table(cmds: Cmds, anchor: Point<Pixels>, data: VariablesData) -> Self {
        let mut w = Self::base(WindowKind::VariablesTable, "Variables", cmds, Open::Anchor(anchor));
        w.width = variables::WINDOW_W;
        w.height = variables::WINDOW_H;
        // 表格自己控制左右 inset，卡片 body 不再加通用 16 pt 内边距。
        w.pad = 0.0;
        w.variables_data = Some(data);
        w
    }

    /// 根视图注入 Floating/Full 切换与关闭回调（编辑器尚未创建时先存着）。
    pub(crate) fn set_variables_callbacks(&mut self, on_full: Rc<dyn Fn(bool, &mut App)>, on_close: Rc<dyn Fn(&mut App)>) {
        self.var_on_full = on_full;
        self.var_on_close = on_close;
        if let Some(e) = &mut self.editor {
            e.set_callbacks(self.var_on_full.clone(), self.var_on_close.clone());
        }
    }

    fn total_variables(&self) -> usize {
        if let Some(e) = &self.editor {
            return e.total();
        }
        self.variables_data.as_ref().and_then(|d| d.collections.first()).map(|c| c.rows.len()).unwrap_or(0)
    }

    /// 变量编辑器是否处于全屏呈现（左栏选中态、卡片几何都看它）。
    pub(crate) fn variables_full(&self) -> bool {
        // `render()` 在 `body_element` 之前就要决定几何，首帧 editor 还没建，必须读显式状态。
        self.editor_full.unwrap_or_else(|| self.editor.as_ref().is_some_and(|e| e.is_full()))
    }

    /// 由根视图把「记住的 Full/Floating」写回。editor 可能尚未创建，故先存字段，
    /// `body_element` 建好后再灌进去（`variables_full()` 也以这个字段为准）。
    pub(crate) fn set_variables_full(&mut self, full: bool) {
        self.editor_full = Some(full);
        if let Some(e) = &self.editor {
            e.set_full(full);
        }
    }

    /// 变量窗口是否有拖拽正在进行（整卡移动 / 8 个 resize handle）。
    pub(crate) fn variables_active(&self) -> bool {
        self.pending.is_some()
            || self.grab.is_some()
            || self.editor.as_ref().is_some_and(|e| e.resizing())
            // 编辑器内部的拖拽（行排序 / 列宽 / 侧栏 / 分栏 / 模式列 / 横向滚动条）同样要由
            // **根视图**接管：GPUI 元素级的 `on_mouse_move` 只在指针悬停该元素时触发，
            // 指针一移出卡片拖拽就断（用户 2026-11-07 明确指出这个监听方式不对）。
            || self.editor.as_ref().is_some_and(|e| e.internal_drag())
    }

    /// resize handle 优先：按下落在边缘时，不让整卡拖动抢走这次手势。
    pub(crate) fn variables_resizing(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.resizing())
    }

    /// 由根视图用**窗口坐标**推进变量窗口的所有拖拽：resize 改 `size`，整卡移动改 `place`。
    /// 返回是否真的变了（调用方据此 `cx.notify()`，配合节流避免每帧整树重绘导致拖动发涩）。
    ///
    /// resize 优先且独占：一旦某个边缘/角 handle 按下（`editor.resizing()`），本次手势只改尺寸，
    /// 绝不启动整卡移动——否则按下右下角会被同时解读成「移动 + 缩放」，窗口乱跳。
    pub(crate) fn variables_route_move(&mut self, at: Point<Pixels>) -> bool {
        if self.editor.as_ref().is_some_and(|e| e.resizing()) {
            if let Some(e) = &self.editor {
                e.route_move(at);
            }
            return true;
        }
        // 内置拖拽（行 / 列宽 / 侧栏 / 分栏 / 模式列宽 / 横向滚动条）进行中：整卡不要动，
        // 但**必须由根视图把移动转给编辑器的 `route_move`**——否则指针一离开卡片，
        // 编辑器自己的 `on_mouse_move` 就不再触发，拖拽会当场冻结。
        if self.editor.as_ref().is_some_and(|e| e.internal_drag()) {
            if let Some(e) = &self.editor {
                e.route_move(at);
            }
            return true;
        }
        // 整卡移动：越过 3pt 死区才 latch，纯点击不拖（egui `click_and_drag`）。
        if !self.dragging() {
            let Some(p) = self.pending else { return false };
            let dx = (f32::from(at.x) - f32::from(p.x)).abs();
            let dy = (f32::from(at.y) - f32::from(p.y)).abs();
            if dx + dy < 3.0 {
                return false;
            }
            self.start_drag();
        }
        self.move_drag(at)
    }

    /// 抬起 / Escape：结束整卡拖动与 resize。
    pub(crate) fn variables_end_drag(&mut self) {
        self.end_drag();
        if let Some(e) = &self.editor {
            e.end_resize();
        }
    }

    /// 重新打开变量编辑器时刷新文档快照，但保留交互状态（选中 / 分组折叠 / 窗口位置）。
    pub(crate) fn set_variables_data(&mut self, data: VariablesData) {
        if let Some(e) = &mut self.editor {
            e.set_data(data);
        } else {
            self.variables_data = Some(data);
        }
    }

    /// 根视图的 `on_key_down` 转发给变量编辑器（Escape / Ctrl-Cmd+C/V）。
    pub(crate) fn variables_key(&self, key: &str, accel: bool) -> bool {
        if self.kind != WindowKind::VariablesTable {
            return false;
        }
        self.editor.as_ref().is_some_and(|e| e.key(key, accel))
    }

    // ── ⑥ 文本样式选择弹窗（egui `widgets/style_picker.rs`, 标题 "Text styles"）───────────────
    pub(crate) fn styles(cmds: Cmds, cx: &mut App, anchor: Point<Pixels>, title: impl Into<SharedString>, rows: Vec<Row>) -> Self {
        let mut w = Self::base(WindowKind::TextStyles, title, cmds, Open::Anchor(anchor));
        w.surface = Surface::Menu;
        w.pad = 8.0;
        w.height = 320.0;
        w.rows = rows;
        w.add_search(cx, "Search styles");
        w
    }

    // ── ③ 颜色选择器弹窗（egui `widgets/colorpicker.rs`, 标题 "Custom"）────────────────────
    pub(crate) fn color(
        cx: &mut App,
        origin: Point<Pixels>,
        current: Color32,
        on_pick: Rc<dyn Fn(Color32, &mut Window, &mut App)>,
        cmds: Cmds,
    ) -> Self {
        let mut w = Self::base(WindowKind::Color, "Custom", cmds, Open::Anchor(origin));
        // egui 的取色器用 `pad(0)` 让 240 pt 的内容贴住窗口；这里同样给 12 pt 内边距，
        // 内容自己按 240 pt 排（SV 方块 / 色相 / 透明度 / Hex / 页面色板）。
        w.width = 264.0;
        w.pad = 12.0;
        w.height = HEADER_H + 348.0;
        let hex = TextField::build(cx, 240.0, |tf, _| {
            tf.set_value(current.to_hex());
            tf.set_placeholder("Hex");
        });
        let commit = on_pick.clone();
        hex.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, window, cx| {
                if let Some(c) = Color32::from_hex(text) {
                    commit(c, window, cx);
                }
            });
        });
        w.color = Some(ColorState { current, hex, on_pick });
        w
    }

    fn add_search(&mut self, cx: &mut App, hint: &str) {
        let query = self.query.clone();
        let field = search_field(cx, self.width - 2.0 * self.pad, "", hint);
        field.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| *query.borrow_mut() = text.to_lowercase());
        });
        self.search = Some(field);
    }

    // ── drag (the view owns the pointer events; this is just its state) ──────────────────────
    /// Record a press on the card. This does **not** start a drag and does **not** repaint: a press
    /// that never moves past the threshold must stay a click, or header buttons stop firing (GPUI
    /// matches a click's down/up by `ElementId`, and a repaint between them can shift the id).
    pub(crate) fn press(&mut self, at: Point<Pixels>) {
        self.pending = Some(at);
    }
    /// The press point while the pointer is held but has not moved far enough to be a drag.
    pub(crate) fn pending(&self) -> Option<Point<Pixels>> {
        self.pending
    }
    /// Latch the drag: from here the card follows the pointer. Seeds `grab` from the *press* so the
    /// card does not jump to the pointer.
    pub(crate) fn start_drag(&mut self) {
        if let Some(at) = self.pending.take() {
            self.grab = Some(Point { x: px(f32::from(at.x) - f32::from(self.origin.x)), y: px(f32::from(at.y) - f32::from(self.origin.y)) });
        }
    }
    pub(crate) fn dragging(&self) -> bool {
        self.grab.is_some()
    }
    /// Returns true only when the card actually moved. egui clamps **every** drag step to the
    /// screen (`self.place = Place::At(clamp(at - grab, size, screen))`); without that clamp a
    /// pointer that leaves the window parks the card outside it, and the next enter jumps it.
    /// Clamping here also keeps `place` in the same coordinate space as `origin` (which `show()`
    /// clamps), so the grab offset never drifts between frames.
    pub(crate) fn move_drag(&mut self, at: Point<Pixels>) -> bool {
        let Some(g) = self.grab else { return false };
        let raw = Point { x: px(f32::from(at.x) - f32::from(g.x)), y: px(f32::from(at.y) - f32::from(g.y)) };
        let p = popover::clamp(raw, self.width, self.height, self.viewport);
        if let Some(old) = self.place {
            let dx = f32::from(p.x) - f32::from(old.x);
            let dy = f32::from(p.y) - f32::from(old.y);
            if dx.abs() < 0.5 && dy.abs() < 0.5 {
                return false;
            }
        }
        self.place = Some(p);
        true
    }
    pub(crate) fn end_drag(&mut self) {
        self.grab = None;
        self.pending = None;
    }

    /// 仅供端到端测试读取拖拽后的卡片原点（窗口坐标）。
    #[cfg(test)]
    pub(crate) fn test_origin(&self) -> Point<Pixels> {
        self.origin
    }
    /// 仅供端到端测试读取拖拽后的卡片落点（None = 尚未拖动）。
    #[cfg(test)]
    pub(crate) fn test_place(&self) -> Option<Point<Pixels>> {
        self.place
    }
    /// 仅供端到端测试读取 resize 后的卡片尺寸（None = 还没改过尺寸）。
    #[cfg(test)]
    pub(crate) fn test_size(&self) -> Option<(f32, f32)> {
        self.editor.as_ref().and_then(|e| e.size())
    }

    // ── header title slot ───────────────────────────────────────────────────────────────────
    fn title_slot(&self) -> AnyElement {
        let t = tokens();
        match self.kind {
            // A custom title: the current colour's swatch beside the usual label.
            WindowKind::Color => {
                let c = self.color.as_ref().map(|c| c.current).unwrap_or(Color32::BLACK);
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .child(div().size(pt(12.0)).rounded(pt(3.0)).bg(rgba(c)).border_1().border_color(rgba(t.border)))
                    .child(title_text("Custom"))
                    .into_any_element()
            }
            // A custom title with a live control: create a style straight from the header.
            WindowKind::TextStyles => {
                let cmds = self.cmds.clone();
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(4.0))
                    .child(div().flex_1().child(title_text(self.title.clone())))
                    .child(IconButton::new(Icon::Plus).size(20.0).on_click(move |_, _, _| {
                        cmds.push(Cmd { id: "style.create", params: Some(json!({ "type": "text", "name": "Text style" })) });
                    }))
                    .into_any_element()
            }
            // A custom title with a count chip.
            WindowKind::Variables => {
                let n = self.rows.len();
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .child(title_text(self.title.clone()))
                    .child(
                        div()
                            .px(pt(5.0))
                            .h(pt(16.0))
                            .rounded(pt(4.0))
                            .bg(rgba(t.hover))
                            .flex()
                            .items_center()
                            .text_size(pt(9.0))
                            .text_color(rgba(t.text_secondary))
                            .child(format!("{n}")),
                    )
                    .into_any_element()
            }
            // 变量编辑器把「标题 + 计数」画进自己的工具条：整窗一个插槽，顶部只有一行。
            WindowKind::VariablesTable => div().into_any_element(),
            WindowKind::Layout | WindowKind::Stroke => title_text(self.title.clone()).into_any_element(),
        }
    }

    // ── body slot ───────────────────────────────────────────────────────────────────────────
    /// `body_h` = 通用窗口留给 body 的高度（Full 时是整个视口减去 header）。
    fn body_element(&mut self, cx: &mut App, body_h: f32, card_w: f32, viewport: Size<Pixels>) -> AnyElement {
        match self.kind {
            WindowKind::Color => self.color_body(),
            WindowKind::Layout | WindowKind::Stroke => self.settings_element(cx),
            WindowKind::Variables | WindowKind::TextStyles => self.list_element(cx),
            WindowKind::VariablesTable => {
                if self.editor.is_none() {
                    if let Some(data) = self.variables_data.take() {
                        let editor = VariablesEditor::new(
                            self.cmds.clone(),
                            data,
                            cx,
                            self.var_on_full.clone(),
                            self.var_on_close.clone(),
                        );
                        // 把在 editor 建立之前就收到的 Full/Floating 记住值灌进去。
                        if let Some(full) = self.editor_full {
                            editor.set_full(full);
                        }
                        self.editor = Some(editor);
                    }
                }
                // 卡片在窗口中的原点：面板内部的右键菜单用**窗口坐标**记录按下点，而菜单元素
                // 挂在卡片这个 `relative` 容器里，需要换算成卡片局部坐标，否则弹出位置会整体偏移。
                let origin = self.origin;
                let drag_start = self.drag_start.clone();
                match &mut self.editor {
                    Some(e) => e.render(cx, body_h, viewport, origin, drag_start, card_w),
                    None => div().into_any_element(),
                }
            }
        }
    }

    /// 取色器的主体：直接画在通用窗口的 body 插槽里（不再套一层旧取色器卡片）。
    /// 尺寸沿用 `ui-egui::colorpicker`：240 pt 宽的行、12 段色相、棋盘透明度、Hex、页面色板。
    fn color_body(&self) -> AnyElement {
        let Some(c) = &self.color else { return div().into_any_element() };
        let t = tokens();
        let current = c.current;
        let hsb = figcraft_color::Hsb::from_color(figcraft_color::Color::from_rgba8(current.0[0], current.0[1], current.0[2], current.0[3]));
        let hue = hsb.h;
        let pick = c.on_pick.clone();

        // ① 饱和度 / 明度方块：两个线性渐变 quad（底层 白→色相，上层 透明→黑）。
        //    原来是 24×18 = 432 个逐格 quad，再叠上百个棋盘格 —— 拖动这张卡片时每帧要画
        //    上千个 quad，是「拖动卡」的主要来源。GPUI 的 linear_gradient 一次就能画整块。
        //    prepaint 记下盒子在窗口中的位置，点击时按相对坐标取 s / v。
        let sv_hit: Rc<std::cell::Cell<Option<Bounds<Pixels>>>> = Rc::new(std::cell::Cell::new(None));
        let sv_prepaint = sv_hit.clone();
        let sv_click = sv_hit.clone();
        let sv_pick = pick.clone();
        let hue_c = hsb_color(hue, 1.0, 1.0);
        let sv = div()
            .w(pt(240.0))
            .h(pt(180.0))
            .rounded(pt(4.0))
            .overflow_hidden()
            .child(
                gpui::canvas(
                    move |bounds, _, _| sv_prepaint.set(Some(bounds)),
                    move |bounds, _, window, _| {
                        let white = gpui::Hsla::from(rgba(Color32::from_rgb(255, 255, 255)));
                        let pure = gpui::Hsla::from(rgba(hue_c));
                        let clear = gpui::transparent_black();
                        let black = gpui::Hsla::from(rgba(Color32::from_rgb(0, 0, 0)));
                        // 90° = 左→右（饱和度 0→1），180° = 上→下（明度 1→0）。
                        window.paint_quad(gpui::fill(
                            bounds,
                            gpui::linear_gradient(90.0, gpui::linear_color_stop(white, 0.0), gpui::linear_color_stop(pure, 1.0)),
                        ));
                        window.paint_quad(gpui::fill(
                            bounds,
                            gpui::linear_gradient(180.0, gpui::linear_color_stop(clear, 0.0), gpui::linear_color_stop(black, 1.0)),
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                let Some(b) = sv_click.get() else { return };
                let s = ((f32::from(e.position.x) - f32::from(b.origin.x)) / f32::from(b.size.width).max(1.0)).clamp(0.0, 1.0);
                let v = 1.0 - ((f32::from(e.position.y) - f32::from(b.origin.y)) / f32::from(b.size.height).max(1.0)).clamp(0.0, 1.0);
                sv_pick(hsb_color(hue, s, v), window, cx);
            });

        // ② 色相条：6 段渐变（每段两个色标），整条命中按 x 比例取色相。
        //    12 个带独立 hitbox 的 div 换成 6 个 quad，重绘和命中都轻得多。
        let hue_hit: Rc<std::cell::Cell<Option<Bounds<Pixels>>>> = Rc::new(std::cell::Cell::new(None));
        let hue_prepaint = hue_hit.clone();
        let hue_click = hue_hit.clone();
        let hue_pick = pick.clone();
        let hue_row = div()
            .w(pt(240.0))
            .h(pt(20.0))
            .rounded(pt(4.0))
            .overflow_hidden()
            .cursor_pointer()
            .child(
                gpui::canvas(
                    move |bounds, _, _| hue_prepaint.set(Some(bounds)),
                    move |bounds, _, window, _| {
                        let x0 = f32::from(bounds.origin.x);
                        let y0 = f32::from(bounds.origin.y);
                        let w = f32::from(bounds.size.width).max(1.0);
                        let h = f32::from(bounds.size.height).max(1.0);
                        for i in 0..6 {
                            let a = hsb_color(i as f32 * 60.0, 1.0, 1.0);
                            let b = hsb_color((i + 1) as f32 * 60.0, 1.0, 1.0);
                            window.paint_quad(gpui::fill(
                                Bounds { origin: point(px(x0 + w * i as f32 / 6.0), px(y0)), size: size(px(w / 6.0), px(h)) },
                                gpui::linear_gradient(
                                    90.0,
                                    gpui::linear_color_stop(gpui::Hsla::from(rgba(a)), 0.0),
                                    gpui::linear_color_stop(gpui::Hsla::from(rgba(b)), 1.0),
                                ),
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                let Some(b) = hue_click.get() else { return };
                let f = ((f32::from(e.position.x) - f32::from(b.origin.x)) / f32::from(b.size.width).max(1.0)).clamp(0.0, 1.0);
                hue_pick(hsb_color(f * 360.0, 1.0, 1.0), window, cx);
            });

        // ③ 透明度：8pt 棋盘 + 一个 透明→当前色 的水平渐变 quad（原来 24 段 + 4pt 棋盘）。
        let alpha_hit: Rc<std::cell::Cell<Option<Bounds<Pixels>>>> = Rc::new(std::cell::Cell::new(None));
        let alpha_prepaint = alpha_hit.clone();
        let alpha_click = alpha_hit.clone();
        let alpha = div()
            .w(pt(240.0))
            .h(pt(20.0))
            .rounded(pt(4.0))
            .overflow_hidden()
            .child(
                gpui::canvas(
                    move |bounds, _, _| alpha_prepaint.set(Some(bounds)),
                    move |bounds, _, window, _| {
                        crate::widgets::swatch::checker(window, bounds, 8.0);
                        let opaque = gpui::Hsla::from(rgba(current.with_alpha(255)));
                        window.paint_quad(gpui::fill(
                            bounds,
                            gpui::linear_gradient(
                                90.0,
                                gpui::linear_color_stop(gpui::transparent_black(), 0.0),
                                gpui::linear_color_stop(opaque, 1.0),
                            ),
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, {
                let p = pick.clone();
                move |e, window, cx| {
                    let Some(b) = alpha_click.get() else { return };
                    let a = ((f32::from(e.position.x) - f32::from(b.origin.x)) / f32::from(b.size.width).max(1.0)).clamp(0.0, 1.0);
                    p(current.with_alpha((a * 255.0).round() as u8), window, cx);
                }
            });

        // ④ Hex 输入框（提交即写回）。
        // ⑤ 页面色板：12 个纯色块，点一个即应用。
        let palette: [Color32; 12] = [
            Color32::from_rgb(0x00, 0x00, 0x00),
            Color32::from_rgb(0xFF, 0xFF, 0xFF),
            Color32::from_rgb(0xEB, 0x4D, 0x4D),
            Color32::from_rgb(0xF5, 0xA6, 0x23),
            Color32::from_rgb(0x4C, 0xAF, 0x50),
            Color32::from_rgb(0x2D, 0x9C, 0xDB),
            Color32::from_rgb(0x5B, 0x8D, 0xE6),
            Color32::from_rgb(0x9B, 0x59, 0xB6),
            Color32::from_rgb(0x34, 0x49, 0x5E),
            Color32::from_rgb(0x7F, 0x8C, 0x8D),
            Color32::from_rgb(0xE9, 0x1E, 0x63),
            Color32::from_rgb(0x00, 0xBF, 0xA5),
        ];
        let mut grid = div().w(pt(240.0)).grid().grid_cols(6).gap(pt(6.0));
        for col in palette {
            let p = pick.clone();
            grid = grid.child(
                div()
                    .size(pt(20.0))
                    .rounded(pt(3.0))
                    .bg(rgba(col))
                    .border_1()
                    .border_color(rgba(t.border))
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgba(t.text)))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| p(col, window, cx)),
            );
        }
        div()
            .w(pt(240.0))
            .flex()
            .flex_col()
            .gap(pt(6.0))
            .child(sv)
            .child(hue_row)
            .child(alpha)
            .child(div().w(pt(240.0)).h(pt(28.0)).child(c.hex.clone()))
            .child(grid)
            .into_any_element()
    }

    fn list_element(&self, cx: &mut App) -> AnyElement {
        let t = tokens();
        let mut col = div().w_full().flex().flex_col().gap(pt(2.0));
        if let Some(search) = &self.search {
            col = col.child(div().w_full().h(pt(28.0)).child(search.clone()));
        }
        let q = self.query.borrow().trim().to_lowercase();
        let mut shown = 0usize;
        let mut last_group: Option<SharedString> = None;
        for row in &self.rows {
            if !q.is_empty() && !row.label.to_lowercase().contains(&q) {
                continue;
            }
            if row.group != last_group {
                if let Some(g) = &row.group {
                    col = col.child(div().mt(pt(4.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(g.clone()));
                }
                last_group = row.group.clone();
            }
            let cmds = self.cmds.clone();
            let cmd = row.cmd;
            let params = row.params.clone();
            let trail = row.trail.clone();
            col = col.child(
                div()
                    .h(pt(24.0))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(pt(8.0))
                    .rounded(pt(4.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.menu_text))
                    .hover(|s| s.bg(rgba(t.menu_hover)))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        cmds.push(Cmd { id: cmd, params: Some(params.clone()) });
                    })
                    .child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(row.label.clone()))
                    .children(trail.map(|tr| div().text_color(rgba(t.menu_dim)).child(tr))),
            );
            shown += 1;
        }
        if shown == 0 {
            col = col.child(div().py(pt(12.0)).text_size(pt(11.0)).text_color(rgba(t.menu_dim)).child(if self.rows.is_empty() {
                "No items yet."
            } else {
                "No matching items"
            }));
        }
        let _ = cx;
        col.into_any_element()
    }

    fn settings_element(&self, cx: &mut App) -> AnyElement {
        let t = tokens();
        let mut col = div().w_full().flex().flex_col().gap(pt(4.0));
        if let Some((names, active)) = &self.tabs {
            let tab = *active.borrow();
            let mut row = div().w_full().h(pt(24.0)).flex().flex_row().gap(pt(4.0));
            for (i, name) in names.iter().enumerate() {
                let a = active.clone();
                let name = name.clone();
                row = row.child(
                    div()
                        .flex_1()
                        .h_full()
                        .rounded(pt(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(pt(11.0))
                        .text_color(rgba(if i as u8 == tab { t.accent } else { t.text_secondary }))
                        .bg(rgba(if i as u8 == tab { t.selected_row } else { t.field }))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, _| *a.borrow_mut() = i as u8)
                        .child(name),
                );
            }
            col = col.child(row);
        }
        let tab = self.tabs.as_ref().map(|(_, a)| *a.borrow()).unwrap_or(0);
        for setting in self.settings.borrow().iter() {
            match setting {
                Setting::Caption { tab: st, text } => {
                    if *st != tab {
                        continue;
                    }
                    col = col.child(div().mt(pt(4.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(text.clone()));
                }
                Setting::Options { tab: st, label, active, options } => {
                    if *st != tab {
                        continue;
                    }
                    let mut row = div()
                        .w_full()
                        .min_h(pt(24.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(pt(6.0))
                        .child(div().flex_none().w(pt(76.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(label.clone()));
                    let mut chips = div().flex_1().flex().flex_row().flex_wrap().gap(pt(4.0));
                    for (i, opt) in options.iter().enumerate() {
                        let cmds = self.cmds.clone();
                        let opt = opt.clone();
                        let settings = self.settings.clone();
                        let index = *active;
                        let label = label.clone();
                        chips = chips.child(
                            div()
                                .h(pt(20.0))
                                .px(pt(5.0))
                                .rounded(pt(4.0))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_size(pt(9.0))
                                .text_color(rgba(if i == index { t.accent } else { t.text }))
                                .bg(rgba(if i == index { t.selected_row } else { t.field }))
                                .hover(|s| s.bg(rgba(t.hover)))
                                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                    cmds.push(Cmd { id: opt.cmd, params: Some(opt.params.clone()) });
                                    // Keep the highlighted chip in sync (the command's wake redraws us).
                                    if let Ok(mut list) = settings.try_borrow_mut() {
                                        for s in list.iter_mut() {
                                            if let Setting::Options { label: l, active, .. } = s
                                                && *l == label
                                            {
                                                *active = i;
                                            }
                                        }
                                    }
                                })
                                .child(opt.label.clone()),
                        );
                    }
                    row = row.child(chips);
                    col = col.child(row);
                }
                Setting::Number { tab: st, label, field, .. } => {
                    if *st != tab {
                        continue;
                    }
                    col = col.child(
                        div()
                            .w_full()
                            .h(pt(28.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(6.0))
                            .child(div().flex_none().w(pt(76.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(label.clone()))
                            .child(div().flex_1().child(field.clone())),
                    );
                }
            }
        }
        let _ = cx;
        col.into_any_element()
    }

    /// Draws the shared window and (for the colour picker) the picker's body over its slot.
    /// Returns the elements in paint order; the root view puts them above its backdrop.
    pub(crate) fn render(
        &mut self,
        cx: &mut App,
        viewport: Size<Pixels>,
        dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
        drag: Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>,
        drag_move: Rc<dyn Fn(&MouseMoveEvent, &mut Window, &mut App)>,
        right_edge: Pixels,
    ) -> Vec<AnyElement> {
        self.viewport = viewport;
        // 变量编辑器把拖动绑在**自己的头部左侧**（不是整张卡片），所以要把回调传进 body。
        self.drag_start = Some(drag.clone());
        let open = anchored_open(self.open, right_edge);
        // Full 呈现：egui 的 `variables_panel::full(panel)` —— 只占「面板 + 画布」那块（左侧 rail
        // 保留），`Frame::NONE` 贴边铺满；Floating 用构造时的宽高与拖拽位置。
        let full = self.kind == WindowKind::VariablesTable && self.variables_full();
        // 变量窗口允许用户拖 8 个 handle 改尺寸（`EditorState.size`）。
        let (base_w, base_h) = self.editor.as_ref().and_then(|e| e.size()).unwrap_or((self.width, self.height));
        // Full 从左侧 rail 的右缘开始，宽 = 视口 − rail，这样左栏图标不会消失。
        let rail = if full { crate::panels::RAIL_W } else { 0.0 };
        let width = if full { (f32::from(viewport.width) - rail).max(1.0) } else { base_w };
        let height = if full { f32::from(viewport.height) } else { base_h };
        // Full 是 `Frame::NONE`：内边距 0、无圆角/阴影；Floating 保留通用 16（变量表构造时是 0）。
        let pad = if full { 0.0 } else { self.pad };
        let body_h = if full { f32::from(viewport.height) } else { height - 2.0 * self.pad };
        let at = if full {
            // 贴着 rail 右缘，保留左侧图标栏（egui 的 full 就是「面板+画布」那块）。
            Some(Point { x: px(rail), y: px(0.0) })
        } else {
            self.place
        };
        // 先算**当帧**卡片原点：面板内部右键菜单记的是窗口坐标，body_element 要用它换算成
        // 卡片局部坐标。origin() 只依赖 open/at/width/height/flush，可用一个 probe 提前求值，
        // 否则首帧或拖动后会差一帧。
        let mut probe = popover_window(open).width(width).pad(pad).flush(full).height(height);
        if let Some(p) = at {
            probe = probe.at(p);
        }
        self.origin = probe.origin(viewport);
        let mut pop = popover_window(open)
            .width(width)
            .pad(pad)
            .flush(full)
            .surface(self.surface)
            .height(height)
            .title(self.title_slot())
            .body(self.body_element(cx, body_h.max(120.0), width, viewport))
            .on_drag_start(move |e, window, cx| drag(e, window, cx))
            .on_drag_move(move |e, window, cx| drag_move(e, window, cx));
        if let Some(p) = at {
            pop = pop.at(p);
        }
        let (origin, card) = pop.show(viewport, dismiss);
        self.origin = origin;
        vec![card]
    }
}

/// Placement rule from `ui-egui/src/popover.rs`: the card's right edge sits on the Design
/// panel's left edge, its top on the button that opened it — "lifted so its bottom keeps off the
/// screen's bottom". Callers hand us the press point; only its y survives (moved up to the
/// button's top). The pointer's x must never decide where a detached window opens.
fn anchored_open(open: Open, right_edge: Pixels) -> Open {
    match open {
        Open::Anchor(a) => Open::Anchor(Point { x: right_edge, y: px((f32::from(a.y) - BUTTON_HALF).max(0.0)) }),
        other => other,
    }
}

/// `Hsb` → `Color32`（与 `widgets/menu.rs::hsb_color` 同式，取色器主体自己算色用）。
fn hsb_color(h: f32, s: f32, b: f32) -> Color32 {
    crate::widgets::swatch::color32(figcraft_color::Hsb { h, s, b }.to_color())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_kinds_are_distinct() {
        let kinds = [WindowKind::Layout, WindowKind::Stroke, WindowKind::Color, WindowKind::Variables, WindowKind::TextStyles];
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                assert_eq!(a == b, i == j);
            }
        }
    }

    #[test]
    fn a_row_carries_its_command() {
        let r = Row::new("Brand/Primary", "variable.bind", json!({ "field": "fill" })).group("Collection").trail("Color");
        assert_eq!(r.cmd, "variable.bind");
        assert_eq!(r.group.as_ref().map(|g| g.to_string()), Some("Collection".to_string()));
        assert_eq!(r.params["field"], "fill");
    }

    #[test]
    fn every_window_opens_against_the_design_panel_not_the_pointer() {
        // The press was out in the right panel; only its y (button top) may survive.
        let open = anchored_open(Open::Anchor(Point { x: px(2589.0), y: px(503.0) }), px(980.0));
        match open {
            Open::Anchor(p) => {
                assert_eq!(f32::from(p.x), 980.0, "right edge belongs on the panel edge");
                assert_eq!(f32::from(p.y), 503.0 - BUTTON_HALF, "top belongs on the button top");
            }
            _ => panic!("an anchor must stay an anchor"),
        }
        // All five kinds are built as anchors now; none pins its top-left to the pointer.
        assert!(matches!(open, Open::Anchor(_)));
    }

    #[test]
    fn a_drag_never_parks_the_card_outside_the_window() {
        let wake: Rc<dyn Fn()> = Rc::new(|| {});
        let mut w = WindowState::base(WindowKind::Layout, "Layout", Cmds::new(&wake), Open::Anchor(Point { x: px(0.0), y: px(0.0) }));
        w.viewport = size(px(1200.0), px(800.0));
        w.width = WIDTH;
        w.height = 200.0;
        w.origin = Point { x: px(400.0), y: px(100.0) };
        w.press(Point { x: px(420.0), y: px(120.0) });
        w.start_drag();
        // Pointer leaves the window and comes back at an absurd coordinate: the card stays put.
        assert!(w.move_drag(Point { x: px(99_999.0), y: px(-99_999.0) }));
        let p = w.place.expect("drag recorded a place");
        assert!(f32::from(p.x) <= 1200.0 - WIDTH - 8.0);
        assert!(f32::from(p.y) >= 8.0);
        // A drag that has ended (e.g. the button was released outside) must not move the card.
        w.end_drag();
        assert!(!w.move_drag(Point { x: px(700.0), y: px(400.0) }));
    }

    /// The Floating variables window routes its drag through `variables_route_move` (what the root
    /// `on_mouse_move` calls via `move_popup_drag`). A press only records the grab origin; a move
    /// inside the 3pt dead zone must not latch, and a move past it must relocate `place`.
    #[test]
    fn variables_route_move_latches_past_dead_zone_and_relocates() {
        let wake: Rc<dyn Fn()> = Rc::new(|| {});
        let mut w = WindowState::variables_table(Cmds::new(&wake), Point { x: px(100.0), y: px(100.0) }, VariablesData { collections: Vec::new(), libraries: Vec::new() });
        w.viewport = size(px(1280.0), px(860.0));
        w.origin = Point { x: px(100.0), y: px(100.0) };
        // Press records the grab origin; no drag yet.
        w.press(Point { x: px(120.0), y: px(120.0) });
        assert!(w.variables_active(), "pending must mark the window active");
        // A 2px move stays inside the 3pt dead zone: must not latch, must not move.
        assert!(!w.variables_route_move(Point { x: px(121.0), y: px(121.0) }), "still inside dead zone");
        assert!(!w.dragging());
        assert!(w.pending().is_some());
        // A 100px move clears the dead zone: latches, then relocates `place`.
        let at = Point { x: px(220.0), y: px(220.0) };
        assert!(w.variables_route_move(at), "card should move");
        assert!(w.dragging());
        assert!(w.pending().is_none(), "start_drag consumed pending");
        let p = w.place.expect("place set");
        // grab = press - origin = (20,20); raw = at - grab = (200,200); inside viewport → unchanged.
        assert!((f32::from(p.x) - 200.0).abs() < 0.5, "place.x ≈ 200, got {}", f32::from(p.x));
        assert!((f32::from(p.y) - 200.0).abs() < 0.5, "place.y ≈ 200, got {}", f32::from(p.y));
        // Ending the drag clears every transient state (matches `variables_end_drag`).
        w.variables_end_drag();
        assert!(!w.dragging());
        assert!(w.pending().is_none());
        assert!(!w.variables_active());
    }
}
