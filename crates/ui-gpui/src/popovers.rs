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
use crate::widgets::menu::menu_popup_in;
use crate::widgets::popover::{self, HEADER_H, Open, Surface, WIDTH, popover_window, title_text};
use crate::widgets::typ_icons::TypIcon;
use crate::widgets::{Icon, IconButton, SegItem, Segmented, Slot, TextField, number_field, pt, rgba, search_field, tokens};
use crate::widgets::{MenuRow, dropdown};
use figcraft_i18n::tr;
use gpui::{
    AnyElement, App, Bounds, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString, Size, Window, div, point,
    prelude::*, px, size,
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
    /// 详细排版设置（egui `typography.rs::type_settings_panel`）：Basics / Details /
    /// Variable 三页签的浮层，正文全部用通用 `Setting` 行（chips + 数字框）搭出来。
    TypeSettings,
    /// 效果设置小窗：点右栏效果行的方框弹出，装该效果的设置项与类型切换。
    EffectSettings,
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
    /// 有值时用排版对话框字形（`typ-*.svg`）画格，label 退为 tooltip/无障碍文本。
    /// Type settings  Basics 页的对齐 / 装饰线 / 大小写 / 列表 / 截断全是图标格。
    pub glyph: Option<crate::widgets::typ_icons::TypIcon>,
}

impl Opt {
    pub fn new(label: impl Into<SharedString>, cmd: &'static str, params: Value) -> Self {
        Self { label: label.into(), cmd, params, glyph: None }
    }
    /// 把这一格改成 lizard 的排版字形（U̲ / AG / 项目符号…），label 仍保留作语义标注。
    pub fn with_glyph(mut self, g: crate::widgets::typ_icons::TypIcon) -> Self {
        self.glyph = Some(g);
        self
    }
    fn item(&self) -> crate::widgets::SegItem {
        match self.glyph {
            Some(g) => crate::widgets::SegItem::Svg(g),
            None => crate::widgets::SegItem::text(self.label.clone()),
        }
    }
}

/// One line of a settings window (Layout / Stroke).
#[derive(Clone)]
pub(crate) enum Setting {
    Caption {
        tab: u8,
        text: SharedString,
    },
    /// A labelled row of choices; `active` is the raised cell. `None` = nothing chosen
    /// (a mixed selection, or a value this node does not have) — the design's `State=Rest`.
    Options {
        tab: u8,
        label: SharedString,
        active: Option<usize>,
        options: Vec<Opt>,
    },
    /// 定宽的图标分段行（Type settings Basics 页）：左标签 + 右侧 SVG 字形条，整条 `bar_w` 宽、
    /// 格子均分；`options` 带 `glyph` 时画 lizard 的 `typ-*.svg`。`drill_in` 是行尾的 `>` 按钮
    /// （文本修饰用），点击把 `active tab` 切到目标页签。
    IconOptions {
        tab: u8,
        label: SharedString,
        active: Option<usize>,
        bar_w: f32,
        options: Vec<Opt>,
        /// 行尾 `>` 按钮：与紧跟其后的 `ExpandedRows.flag` 共用同一份标志，点击翻转即展开/收起。
        drill_in: Option<Rc<RefCell<bool>>>,
    },
    /// A labelled numeric field; its commit was wired to the command queue at construction.
    Number {
        tab: u8,
        label: SharedString,
        field: Entity<TextField>,
    },
    /// Type settings 顶部的「预览」大框（lizard `type_settings_basics` 的 `previewBox`）。
    /// 所有页签共用、只画一次；内容是选中文本的字面，灰底、11 pt、次要色。
    Preview {
        text: SharedString,
    },
    /// Details 页的粗体分组标题（lizard `details_section`：10.5 pt semibold + 下方一条分隔线）。
    GroupTitle {
        tab: u8,
        text: SharedString,
    },
    /// 一条 1 px 分隔线（Basics 页预览框 / 上下两组之间）。
    Divider {
        tab: u8,
    },
    /// 标签 + 当前值 + 通用下拉（Wrap style：自动 / 平衡 / 美观）。
    /// 触发按钮用通用 `widgets::dropdown`（Input 模板同源），弹层用通用 `menu_popup_in`
    /// （`Menu::show`：菜单表面 / hover / 防溢出 / 滚动）。浮层占着根视图唯一的 `popup` 槽，
    /// 开不了第二个 `Popup::Menu`，所以菜单作为浮层 `render()` 多返回的一个**窗口级兄弟元素**
    /// 挂在根视图的 `els` 上（与浮层卡片同级，因此用窗口坐标、不被滚动裁剪）。
    Dropdown {
        tab: u8,
        label: SharedString,
        /// 当前显示值，选中后就地更新。
        current: Rc<RefCell<SharedString>>,
        /// 触发按钮的窗口矩形（`Dropdown::on_click` 回填），作为通用菜单的锚点。
        anchor: Rc<RefCell<Option<Bounds<Pixels>>>>,
        options: Vec<Opt>,
    },
    /// Details 页 `details_toggle` 的 `— / ✓` 开关行（hanging / features）。
    /// `on` 是当前值（retained，点一次翻转并发对应命令），`supported=false` 时画禁用态。
    Toggle {
        tab: u8,
        label: SharedString,
        on: Rc<RefCell<bool>>,
        supported: bool,
        on_cmd: Opt,
        off_cmd: Opt,
    },
    /// 由行尾 `>` 展开的一组子行（`decoration_details`：样式 / 粗细 / 跳过下伸部）。
    /// `flag` 与 `Setting::IconOptions.drill_in` 共用；false 时整组不渲染，点 `>` 翻转后显示。
    ExpandedRows {
        tab: u8,
        flag: Rc<RefCell<bool>>,
        rows: Vec<Setting>,
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
    /// 通用「按住拖动」状态：按下 / 3pt 死区 / latch / 相对位移 / 收尾。
    /// 与尺寸分隔条（`resize_drag`）、通用对话框（`dialogs`）共用同一份 `hold_drag` 实现。
    /// 按下本身**不重绘**：GPUI 靠 ElementId 匹配 click 的 down/up，重绘会让头部按钮点不动。
    hold: crate::hold_drag::HoldDrag,
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
    /// 浮层内通用下拉的菜单卡片（窗口坐标，作为 `render()` 的兄弟元素返回）。每帧清空重建。
    overlays: Rc<RefCell<Vec<AnyElement>>>,
    /// 当前浮层的关闭回调（`render()` 每帧写入），供菜单 `dismiss` 之外复用。
    dismiss: Rc<RefCell<Option<Rc<dyn Fn(&mut Window, &mut App)>>>>,
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
            hold: crate::hold_drag::HoldDrag::default(),
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
            overlays: Rc::new(RefCell::new(Vec::new())),
            dismiss: Rc::new(RefCell::new(None)),
        }
    }

    // ── ① 自动布局设置（egui `panels/layout_panel.rs::settings`, 标题 "Layout"）───────────────
    pub(crate) fn layout(cmds: Cmds, data: &PanelData, cx: &mut App, anchor: Point<Pixels>) -> Self {
        let mut w = Self::base(WindowKind::Layout, tr("Layout"), cmds, Open::Anchor(anchor));
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
                label: tr("Strokes").into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("Included"), "node.set", json!({ "props": { "strokesIncluded": true } })),
                    Opt::new(tr("Excluded"), "node.set", json!({ "props": { "strokesIncluded": false } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: tr("Canvas stacking").into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("Last on top"), "node.set", json!({ "props": { "reverseZ": false } })),
                    Opt::new(tr("First on top"), "node.set", json!({ "props": { "reverseZ": true } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: "Text baselines".into(),
                active: Some(usize::from(text_baseline)),
                options: vec![
                    Opt::new(tr("Off"), "node.set", json!({ "props": { "crossAlign": "min" } })),
                    Opt::new(tr("Align"), "node.set", json!({ "props": { "crossAlign": "baseline" } })),
                ],
            },
            Setting::Options {
                tab: 0,
                label: "Gap mode".into(),
                active: Some(gap_active(main, wrap)),
                options: if wrap {
                    vec![
                        Opt::new(tr("Fixed"), "node.set", json!({ "props": { "contentAlign": "auto" } })),
                        Opt::new(tr("Auto"), "node.set", json!({ "props": { "contentAlign": "spaceBetween" } })),
                    ]
                } else {
                    vec![
                        Opt::new(tr("Fixed"), "node.set", json!({ "props": { "mainAlign": "min" } })),
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
        let mut w = Self::base(WindowKind::Stroke, tr("Stroke"), cmds.clone(), Open::Anchor(anchor));
        w.pad = 12.0;
        w.height = 320.0;
        w.tabs = Some((vec!["Basic".into(), tr("Brush").into()], Rc::new(RefCell::new(0u8))));
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
            Opt::new(tr("Uniform"), "stroke.widthProfile", json!({ "preset": "uniform" })),
            Opt::new(tr("Wedge"), "stroke.widthProfile", json!({ "preset": "wedge" })),
            Opt::new(tr("Taper"), "stroke.widthProfile", json!({ "preset": "taper" })),
            Opt::new(tr("Eye"), "stroke.widthProfile", json!({ "preset": "eye" })),
        ];
        let settings = vec![
            Setting::Caption { tab: 0, text: tr("Stroke style").into() },
            Setting::Options {
                tab: 0,
                label: tr("Pattern").into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("Solid"), "node.set", json!({ "props": { "dashes": [] } })),
                    Opt::new(tr("Dash"), "node.set", json!({ "props": { "dashes": [weight * 4.0, weight * 4.0] } })),
                ],
            },
            Setting::Number { tab: 0, label: tr("Dash").into(), field: dash_field },
            Setting::Number { tab: 0, label: tr("Gap").into(), field: gap_field },
            Setting::Options {
                tab: 0,
                label: tr("Join").into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("Miter"), "node.set", json!({ "props": { "strokeJoin": "miter" } })),
                    Opt::new(tr("Bevel"), "node.set", json!({ "props": { "strokeJoin": "bevel" } })),
                    Opt::new(tr("Round"), "node.set", json!({ "props": { "strokeJoin": "round" } })),
                ],
            },
            Setting::Number { tab: 0, label: tr("Miter angle").into(), field: miter_field },
            Setting::Options {
                tab: 0,
                label: "End caps".into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("None"), "node.set", json!({ "props": { "strokeCap": "butt" } })),
                    Opt::new(tr("Round"), "node.set", json!({ "props": { "strokeCap": "round" } })),
                    Opt::new(tr("Square"), "node.set", json!({ "props": { "strokeCap": "square" } })),
                ],
            },
            Setting::Options { tab: 0, label: tr("Width profile").into(), active: Some(0), options: profiles.clone() },
            Setting::Caption { tab: 1, text: tr("Brush").into() },
            Setting::Options {
                tab: 1,
                label: tr("Brush").into(),
                active: Some(0),
                options: vec![
                    Opt::new(tr("None"), "stroke.brush", json!({ "preset": Value::Null })),
                    Opt::new(tr("Stretch"), "stroke.brush", json!({ "preset": "stretch" })),
                    Opt::new(tr("Scatter"), "stroke.brush", json!({ "preset": "scatter" })),
                ],
            },
            Setting::Options { tab: 1, label: tr("Width profile").into(), active: Some(0), options: profiles },
        ];
        w.settings = Rc::new(RefCell::new(settings));
        w
    }

    // ── ④ 变量弹窗（egui `panels/variable_ui.rs::picker`, 标题 "Variables"）─────────────────
    pub(crate) fn variables(cmds: Cmds, cx: &mut App, anchor: Point<Pixels>, rows: Vec<Row>) -> Self {
        let mut w = Self::base(WindowKind::Variables, tr("Variables"), cmds, Open::Anchor(anchor));
        w.surface = Surface::Menu;
        w.pad = 8.0;
        w.height = 300.0;
        w.rows = rows;
        w.add_search(cx, "Search variables");
        w
    }

    // ── ⑤ 详细排版设置（egui `typography.rs::type_settings_panel` + `type_settings_basics`）──
    //
    // egui 的这个浮层是「三页签 + 头部关闭按钮」的 Compact 面板：
    // `Basics`（对齐 / 装饰线 / 大小写 / 垂直裁切 / 列表 / 段落间距）、
    // `Details`（装饰线样式 / 粗细 / 跳过下伸部 / 大小写）、
    // `Variable`（可变字体的轴）。
    //
    // 这里**不新建任何控件**：页签用通用「设置窗口」自带的那条页签，行用 `Setting::Options`
    // （chips）与 `Setting::Number`（数字框）搭；正文与页签的排版完全交给 `settings_element`。
    // 与 egui 的两处差异都来自控件库的现状，不是另起一套：
    //   * egui 的装饰线 / 大小写 / 对齐是**图标**分段控件，通用 chips 只有文字（图标集里没有
    //     `TypeDialog*` 这些字形），所以这里用文字 chips；
    //   * `Variable` 页在 egui 里是滑杆；控件库没有滑杆，改用每条轴一个数字框（同样是通用控件）。
    // 第三页只在字体**确实是可变字体**时出现 —— egui 是把它画成禁用页签，通用页签没有禁用态，
    // 于是缺省就不出现，避免留一个点不动的死页签。
    pub(crate) fn type_settings(cmds: Cmds, data: &PanelData, cx: &mut App, anchor: Point<Pixels>) -> Self {
        let mut w = Self::base(WindowKind::TypeSettings, tr("Type settings"), cmds, Open::Anchor(anchor));
        // 卡片 pad=8：标题行（页签）从 y=8 起，与右上角 X（top 8 / 24 高，中心 y=20）垂直居中对齐。
        // 正文另在 settings_element 里补 px(4)，把左右视觉内边距补回 12（见那里的 kind 判断）。
        w.pad = 8.0;
        // Basics 短、Details 长：高度用于抬升/夹取，正文交给滚动容器按内容延展。
        w.height = 520.0;
        let t = data.text.clone().unwrap_or_default();
        let has_variable = !t.axes.is_empty();
        let mut tabs = vec![tr("Basics").into(), tr("Details").into()];
        if has_variable {
            tabs.push(tr("Variable").into());
        }
        w.tabs = Some((tabs, Rc::new(RefCell::new(0u8))));

        // 「文本修饰」行尾 `>` 展开的装饰线明细，与该行的 drill_in 共用同一标志。
        let deco_flag: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let mut settings: Vec<Setting> = Vec::new();

        // ── Basics：预览框（三页签共用，Variable 页不画）───────────────────────────────
        settings.push(Setting::Preview { text: tr("Preview").into() });
        settings.push(Setting::Divider { tab: 0 });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("Alignment").into(),
            active: match t.align_h {
                Some("left") => Some(0),
                Some("center") => Some(1),
                Some("right") => Some(2),
                Some("justified") => Some(3),
                _ => None,
            },
            bar_w: 96.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("Align left"), "node.set", json!({ "props": { "textAlign": "left" } })).with_glyph(TypIcon::AlignLeft),
                Opt::new(tr("Align center"), "node.set", json!({ "props": { "textAlign": "center" } })).with_glyph(TypIcon::AlignCenter),
                Opt::new(tr("Align right"), "node.set", json!({ "props": { "textAlign": "right" } })).with_glyph(TypIcon::AlignRight),
                Opt::new(tr("Justify"), "node.set", json!({ "props": { "textAlign": "justified" } })).with_glyph(TypIcon::Justify),
            ],
        });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("Decoration").into(),
            active: match t.decoration {
                Some("none") => Some(0),
                Some("underline") => Some(1),
                Some("strikethrough") => Some(2),
                _ => None,
            },
            bar_w: 72.0,
            drill_in: Some(deco_flag.clone()),
            options: vec![
                Opt::new(tr("None"), "node.set", json!({ "props": { "textDecoration": "none" } })).with_glyph(TypIcon::None),
                Opt::new(tr("Underline"), "node.set", json!({ "props": { "textDecoration": "underline" } })).with_glyph(TypIcon::Underline),
                Opt::new(tr("Strikethrough"), "node.set", json!({ "props": { "textDecoration": "strikethrough" } })).with_glyph(TypIcon::Strike),
            ],
        });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("Letter case").into(),
            active: match t.case {
                Some("original") => Some(0),
                Some("upper") => Some(1),
                Some("lower") => Some(2),
                Some("title") => Some(3),
                Some("smallCaps") => Some(4),
                _ => None,
            },
            bar_w: 120.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("As typed"), "node.set", json!({ "props": { "textCase": "original" } })).with_glyph(TypIcon::None),
                Opt::new(tr("Uppercase"), "node.set", json!({ "props": { "textCase": "upper" } })).with_glyph(TypIcon::Upper),
                Opt::new(tr("Lowercase"), "node.set", json!({ "props": { "textCase": "lower" } })).with_glyph(TypIcon::Lower),
                Opt::new(tr("Title case"), "node.set", json!({ "props": { "textCase": "title" } })).with_glyph(TypIcon::Title),
                Opt::new(tr("Small caps"), "node.set", json!({ "props": { "textCase": "smallCaps" } })).with_glyph(TypIcon::SmallCaps),
            ],
        });
        settings.push(Setting::Divider { tab: 0 });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("Vertical trim").into(),
            active: match t.leading_trim {
                Some("capHeight") => Some(1),
                Some("none") => Some(0),
                _ => None,
            },
            bar_w: 48.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("Standard"), "node.set", json!({ "props": { "leadingTrim": "none" } })).with_glyph(TypIcon::TrimStandard),
                Opt::new(tr("Cap height"), "node.set", json!({ "props": { "leadingTrim": "capHeight" } })).with_glyph(TypIcon::TrimCap),
            ],
        });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("List style").into(),
            active: match t.list_style {
                Some("none") => Some(0),
                Some("bulleted") => Some(1),
                Some("numbered") => Some(2),
                _ => None,
            },
            bar_w: 72.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("No list"), "node.set", json!({ "props": { "listStyle": "none" } })).with_glyph(TypIcon::None),
                Opt::new(tr("Bulleted list"), "node.set", json!({ "props": { "listStyle": "bulleted" } })).with_glyph(TypIcon::ListBullets),
                Opt::new(tr("Numbered list"), "node.set", json!({ "props": { "listStyle": "numbered" } })).with_glyph(TypIcon::ListNumbered),
            ],
        });
        let spacing = number_field(cx, 72.0, t.paragraph_spacing, "", 1.0, Slot::None);
        wire_set(&spacing, cx, &w.cmds, "paragraphSpacing", 0.0, 10_000.0);
        settings.push(Setting::Number { tab: 0, label: tr("Paragraph spacing").into(), field: spacing });
        settings.push(Setting::IconOptions {
            tab: 0,
            label: tr("Truncate text").into(),
            active: match t.auto_resize {
                Some("truncate") => Some(1),
                _ => None,
            },
            bar_w: 48.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("Don't truncate"), "node.set", json!({ "props": { "textAutoResize": "none" } })).with_glyph(TypIcon::None),
                Opt::new(tr("Truncate with an ellipsis"), "node.set", json!({ "props": { "textAutoResize": "truncate" } }))
                    .with_glyph(TypIcon::Truncate),
            ],
        });
        settings.push(Setting::Dropdown {
            tab: 0,
            label: tr("Wrap style").into(),
            current: Rc::new(RefCell::new(
                match t.wrap_style {
                    Some("balance") => tr("Balance"),
                    Some("pretty") => tr("Pretty"),
                    _ => tr("Automatic"),
                }
                .into(),
            )),
            anchor: Rc::new(RefCell::new(None)),
            options: vec![
                Opt::new(tr("Automatic"), "node.set", json!({ "props": { "wrapStyle": "auto" } })),
                Opt::new(tr("Balance"), "node.set", json!({ "props": { "wrapStyle": "balance" } })),
                Opt::new(tr("Pretty"), "node.set", json!({ "props": { "wrapStyle": "pretty" } })),
            ],
        });

        // 「文本修饰 >」展开：装饰线样式 / 粗细 / 跳过下伸部（lizard `decoration_details`）。
        let thickness = number_field(cx, 72.0, t.decoration_thickness, "", 0.5, Slot::None);
        wire_set(&thickness, cx, &w.cmds, "textDecorationThickness", 0.0, 1_000.0);
        settings.push(Setting::ExpandedRows {
            tab: 0,
            flag: deco_flag.clone(),
            rows: vec![
                Setting::Caption { tab: 0, text: tr("Decoration style").into() },
                Setting::Options {
                    tab: 0,
                    label: SharedString::from(""),
                    active: match t.decoration_style {
                        Some("solid") => Some(0),
                        Some("dotted") => Some(1),
                        Some("wavy") => Some(2),
                        _ => None,
                    },
                    options: vec![
                        Opt::new(tr("Solid"), "node.set", json!({ "props": { "textDecorationStyle": "solid" } })),
                        Opt::new(tr("Dotted"), "node.set", json!({ "props": { "textDecorationStyle": "dotted" } })),
                        Opt::new(tr("Wavy"), "node.set", json!({ "props": { "textDecorationStyle": "wavy" } })),
                    ],
                },
                Setting::Number { tab: 0, label: tr("Thickness").into(), field: thickness },
                Setting::Toggle {
                    tab: 0,
                    label: tr("Skip ink").into(),
                    on: Rc::new(RefCell::new(t.decoration_skip_ink.unwrap_or(true))),
                    supported: true,
                    on_cmd: Opt::new(tr("On"), "node.set", json!({ "props": { "textDecorationSkipInk": true } })),
                    off_cmd: Opt::new(tr("Off"), "node.set", json!({ "props": { "textDecorationSkipInk": false } })),
                },
            ],
        });

        // ── Details：lizard `type_settings_details` 的六个分组 ────────────────────────
        settings.push(Setting::GroupTitle { tab: 1, text: tr("Indentation").into() });
        settings.push(Setting::Toggle {
            tab: 1,
            label: tr("Hanging punctuation").into(),
            on: Rc::new(RefCell::new(t.hanging_punctuation.unwrap_or(false))),
            supported: true,
            on_cmd: Opt::new(tr("On"), "node.set", json!({ "props": { "hangingPunctuation": true } })),
            off_cmd: Opt::new(tr("Off"), "node.set", json!({ "props": { "hangingPunctuation": false } })),
        });
        settings.push(Setting::Toggle {
            tab: 1,
            label: tr("Hanging list").into(),
            on: Rc::new(RefCell::new(t.hanging_list.unwrap_or(false))),
            supported: true,
            on_cmd: Opt::new(tr("On"), "node.set", json!({ "props": { "hangingList": true } })),
            off_cmd: Opt::new(tr("Off"), "node.set", json!({ "props": { "hangingList": false } })),
        });
        let indent = number_field(cx, 72.0, t.paragraph_indent, "", 1.0, Slot::None);
        wire_set(&indent, cx, &w.cmds, "paragraphIndent", 0.0, 10_000.0);
        settings.push(Setting::Number { tab: 1, label: tr("Paragraph indent").into(), field: indent });
        settings.push(Setting::Divider { tab: 1 });

        settings.push(Setting::GroupTitle { tab: 1, text: tr("Letter case").into() });
        settings.push(Setting::IconOptions {
            tab: 1,
            label: tr("Letter case").into(),
            active: match t.case {
                Some("original") => Some(0),
                Some("upper") => Some(1),
                Some("lower") => Some(2),
                Some("title") => Some(3),
                Some("smallCaps") => Some(4),
                _ => None,
            },
            bar_w: 120.0,
            drill_in: None,
            options: vec![
                Opt::new(tr("As typed"), "node.set", json!({ "props": { "textCase": "original" } })).with_glyph(TypIcon::None),
                Opt::new(tr("Uppercase"), "node.set", json!({ "props": { "textCase": "upper" } })).with_glyph(TypIcon::Upper),
                Opt::new(tr("Lowercase"), "node.set", json!({ "props": { "textCase": "lower" } })).with_glyph(TypIcon::Lower),
                Opt::new(tr("Title case"), "node.set", json!({ "props": { "textCase": "title" } })).with_glyph(TypIcon::Title),
                Opt::new(tr("Small caps"), "node.set", json!({ "props": { "textCase": "smallCaps" } })).with_glyph(TypIcon::SmallCaps),
            ],
        });
        // OpenType feature 分组完全由扫描结果 `features_supported` 驱动：字体没有的 feature
        // 根本不生成行（不是置灰）。本地只持有一份「分组依据」表，回答「扫描到这个 tag 时
        // 归到哪组 / 显示什么名字 / 默认开关」，不决定显示哪些。
        settings.push(Setting::Divider { tab: 1 });
        settings.extend(Self::type_feature_groups(1, &t.features, &t.features_supported));

        // ── Variable（只有可变字体才有这一页）──────────────────────────────────────────
        for axis in t.axes.iter() {
            let field = number_field(cx, 72.0, Some(axis.value), "", 1.0, Slot::None);
            let tag = axis.tag.clone();
            let cmds = w.cmds.clone();
            field.update(cx, move |tf, _| {
                let tag = tag.clone();
                tf.set_on_commit(move |text, _, _| {
                    if let Ok(v) = text.trim().parse::<f64>() {
                        cmds.push(Cmd { id: "node.set", params: Some(json!({ "props": { "variationSettings": { tag.clone(): v } } })) });
                    }
                });
            });
            settings.push(Setting::Number { tab: 2, label: axis.name.clone().into(), field });
        }
        if has_variable {
            settings.push(Setting::Options {
                tab: 2,
                label: tr("Variations").into(),
                active: Some(0),
                options: vec![Opt::new(tr("Reset to style"), "node.set", json!({ "props": { "variationSettings": Value::Null } }))],
            });
        }
        w.settings = Rc::new(RefCell::new(settings));
        w
    }

    // OpenType feature 面板是**数据驱动**的：显示哪些行完全由字体的扫描结果
    // `features_supported`（`face.feature_tags()`）决定，字体没有的 feature 根本不出现。
    // 本地只持有一份「分组依据」表：把扫描到的 tag 归到 lizard 的分组、给显示名和默认开关。
    // 这些 tag -> 组 / 显示名 / 默认值是**本地知识**（分组依据），不是显示清单。
    //
    // 命令增量写 `text.setFeatures {features:{tag:bool}}`，不会互相覆盖整张 features 表
    // （与 node.set 的清空重建语义不同）。开关的 `on` 优先取文档显式覆盖，否则用默认值；
    // 既然行是从扫描结果生成的，它就一定被字体支持，`supported` 恒真、永不变灰。
    const FEATURE_CATALOG: &'static [(&'static str, &'static str, &'static str, bool)] = &[
        //  tag     显示名                        归属分组 (tr key)      默认
        ("case", "Case-sensitive forms", "Letter case|dialog", false),
        ("cpsp", "Capital spacing", "Letter case|dialog", false),
        ("tnum", "Tabular numbers", "Numbers", false),
        ("frac", "Fractions", "Numbers", false),
        ("zero", "Slashed zero", "Numbers", false),
        ("liga", "Standard ligatures", "Letter forms", true),
        ("dlig", "Discretionary ligatures", "Letter forms", false),
        ("calt", "Contextual alternates", "Letter forms", true),
        ("ordn", "Ordinals", "Letter forms", false),
        ("smcp", "Small capitals", "Letter forms", false),
        ("ss01", "Stylistic set 1", "Stylistic sets", false),
        ("ss02", "Stylistic set 2", "Stylistic sets", false),
        ("ss03", "Stylistic set 3", "Stylistic sets", false),
        ("ss04", "Stylistic set 4", "Stylistic sets", false),
        ("ss05", "Stylistic set 5", "Stylistic sets", false),
        ("kern", "Kerning pairs", "Horizontal spacing", true),
    ];
    /// 分组的显示顺序（与 lizard `type_settings_details` 一致）；空组不显示。
    const FEATURE_GROUP_ORDER: &'static [&'static str] =
        &["Letter case|dialog", "Numbers", "Letter forms", "Stylistic sets", "Horizontal spacing", "Other features"];

    fn type_feature_groups(
        tab: u8,
        features: &std::collections::BTreeMap<String, bool>,
        features_supported: &std::collections::BTreeSet<String>,
    ) -> Vec<Setting> {
        use std::collections::BTreeMap;
        // ① 把扫描到的每个 tag 归进组（本地表只提供归属 / 显示名 / 默认值）。
        //    显示清单 = features_supported ∩ 表；表外但扫描到的 tag 归「Other features」。
        let mut grouped: BTreeMap<&'static str, Vec<Setting>> = BTreeMap::new();
        let mut others: Vec<Setting> = Vec::new();
        for tag in features_supported.iter() {
            let known = Self::FEATURE_CATALOG.iter().find(|(t, _, _, _)| *t == tag.as_str());
            let (label, group, default_on): (SharedString, &'static str, bool) = match known {
                Some((_, name, grp, def)) => (tr(*name).into(), *grp, *def),
                // 字体有、但本地没有可信中文名的 feature：用 tag 本身，归「其它」。
                None => (SharedString::from(tag.clone()), "Other features", false),
            };
            let on = features.get(tag).copied().unwrap_or(default_on);
            let key = tag.clone();
            let row = Setting::Toggle {
                tab,
                label,
                on: Rc::new(RefCell::new(on)),
                supported: true, // 来自扫描结果，必然支持
                on_cmd: Opt::new(tr("On"), "text.setFeatures", json!({ "features": { (key.clone()): true } })),
                off_cmd: Opt::new(tr("Off"), "text.setFeatures", json!({ "features": { (key): false } })),
            };
            if known.is_some() {
                grouped.entry(group).or_default().push(row);
            } else {
                others.push(row);
            }
        }
        if !others.is_empty() {
            grouped.insert("Other features", others);
        }
        // ② 按固定组序输出；组里没有任何行（字体不支持该组所有 feature）就整组不出现。
        let mut out = Vec::new();
        for group in Self::FEATURE_GROUP_ORDER {
            let Some(rows) = grouped.remove(group) else { continue };
            if rows.is_empty() {
                continue;
            }
            out.push(Setting::GroupTitle { tab, text: tr(*group).into() });
            out.extend(rows);
            out.push(Setting::Divider { tab });
        }
        out
    }
    // ── ⑥ 变量编辑器右侧列表（egui `dialogs/variables_panel.rs::table()`）─────────────────
    // 只搬右边的表格：左侧集合/分组 sidebar、顶部搜索、编辑与拖拽都还没做。
    // 标题用集合名（egui 的浮动面板标题），右侧放变量总数的计数 chip。
    pub(crate) fn variables_table(cmds: Cmds, anchor: Point<Pixels>, data: VariablesData) -> Self {
        let mut w = Self::base(WindowKind::VariablesTable, tr("Variables"), cmds, Open::Anchor(anchor));
        w.width = variables::WINDOW_W;
        w.height = variables::WINDOW_H;
        // 表格自己控制左右 inset，卡片 body 不再加通用 16 pt 内边距。
        w.pad = 0.0;
        w.variables_data = Some(data);
        w
    }

    /// 效果设置小窗：点效果行的方框弹出。egui `effects.rs::effect_settings` 是**向下 inline
    /// 展开**设置项，这里按用户 2026-11-13 的要求把设置项装进 detached 小窗。
    ///
    /// 类型切换也搬进小窗顶部，且用 `Effect::of_type` 生成**全新默认对象**。原实现是 clone
    /// 旧 effect JSON 只改 `type`：旧类型的字段（offset / spread / size / density …）会残留，
    /// 被引擎 `refuse_ignored_effect_keys` 判为 unknown properties 后整条 `effect.set` 拒收，
    /// 命令静默失效 —— 这就是「有一些效果点了、选中图层在画布中没有效果」的根因。
    /// `effect.add` 走 `Effect::of_type`，所以从 `+` 菜单添加一直是正常的。
    pub(crate) fn effect_settings(
        cmds: Cmds,
        cx: &mut App,
        anchor: Point<Pixels>,
        index: usize,
        label: SharedString,
        effect: Value,
    ) -> Self {
        let mut w = Self::base(WindowKind::EffectSettings, label.clone(), cmds.clone(), Open::Anchor(anchor));
        w.width = 248.0;
        w.height = 320.0;
        w.pad = 12.0;
        let ty = effect.get("type").and_then(Value::as_str).unwrap_or("dropShadow").to_string();
        let mut settings: Vec<Setting> = Vec::new();
        // ① 类型切换：每项都带一份干净的、只属于该类型的默认 effect。
        let options: Vec<Opt> = figcraft_doc::Effect::TYPES
            .iter()
            .map(|(l, t)| {
                let clean = figcraft_doc::Effect::of_type(*t)
                    .and_then(|e| serde_json::to_value(e).ok())
                    .unwrap_or(Value::Null);
                Opt::new(tr(*l), "effect.set", json!({ "index": index, "effect": clean }))
            })
            .collect();
        let current: SharedString = figcraft_doc::Effect::TYPES
            .iter()
            .find(|(_, t)| *t == ty)
            .map(|(l, _)| tr(*l).into())
            .unwrap_or_else(|| label.clone());
        settings.push(Setting::Dropdown {
            tab: 0,
            label: tr("Type").into(),
            current: Rc::new(RefCell::new(current)),
            anchor: Rc::new(RefCell::new(None)),
            options,
        });
        // ② 该效果的具体设置项（egui effect_settings 的数值 / 开关 / 枚举）。
        settings.extend(effect_setting_rows(cx, &cmds, index, &effect, &ty));
        w.settings = Rc::new(RefCell::new(settings));
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
        self.hold.busy()
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
        // 整卡移动：3pt 死区 / latch / 相对位移 / 收尾全在通用 `hold_drag` 里
        // （egui `click_and_drag`）；这里只把算出的绝对位置写回 `place`。
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
        let mut w = Self::base(WindowKind::Color, tr("Custom"), cmds, Open::Anchor(origin));
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

    // ── drag：状态机在 `hold_drag`，这里只把结果写回 `place` ─────────────────────────────
    /// 按下：只记状态、**不重绘**（GPUI 靠 ElementId 匹配 click 的 down/up）。
    pub(crate) fn press(&mut self, at: Point<Pixels>) {
        self.hold.press(at, self.origin);
    }
    /// 已按下但还没越过 3pt 死区时的按下点。
    pub(crate) fn pending(&self) -> Option<Point<Pixels>> {
        self.hold.pending_point()
    }
    /// 显式 latch（跳过死区），供测试与需要立即起拖的调用方。
    pub(crate) fn start_drag(&mut self) {
        self.hold.latch();
    }
    pub(crate) fn dragging(&self) -> bool {
        self.hold.active()
    }
    /// 通用状态机给出「按下时原点 + 相对位移」的绝对位置，再按视口夹紧（egui 每步都 clamp）。
    /// 返回 true 仅当卡片确实移动了。
    pub(crate) fn move_drag(&mut self, at: Point<Pixels>) -> bool {
        let Some(step) = self.hold.advance(at) else { return false };
        let p = popover::clamp(step.place, self.width, self.height, self.viewport);
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
        self.hold.end();
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
                    .child(title_text(tr("Custom")))
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
                        cmds.push(Cmd { id: "style.create", params: Some(json!({ "type": "text", "name": tr("Text style") })) });
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
            WindowKind::Layout | WindowKind::Stroke | WindowKind::EffectSettings => title_text(self.title.clone()).into_any_element(),
            // Type settings 的标题栏就是三页签（截图顶部没有单独的 "Type settings" 标题文字），
            // 与 lizard `type_settings_panel` 把 Tabs 放进 header 一致。
            WindowKind::TypeSettings => {
                if let Some((names, active)) = &self.tabs {
                    let tab = *active.borrow();
                    // 页签胶囊：整体垂直居中、靠左排、块间 4；每块高 24、文字左右 8，
                    // 只有选中块有底色（与参考截图一致：未选中的「基础/变量」是透明底）。
                    let mut row = div().w_full().flex().flex_row().items_center().gap(pt(4.0));
                    for (i, name) in names.iter().enumerate() {
                        let a = active.clone();
                        let wake = self.cmds.clone();
                        let on = i as u8 == tab;
                        let mut cell = div()
                            .flex_none()
                            .h(pt(24.0))
                            .px(pt(8.0))
                            .rounded(pt(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(pt(11.0))
                            .text_color(rgba(if on { t.text } else { t.text_secondary }))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                *a.borrow_mut() = i as u8;
                                wake.wake();
                            });
                        if on {
                            cell = cell.bg(rgba(t.selected_row));
                        }
                        row = row.child(cell.child(name.clone()));
                    }
                    row.into_any_element()
                } else {
                    title_text(self.title.clone()).into_any_element()
                }
            }
        }
    }

    // ── body slot ───────────────────────────────────────────────────────────────────────────
    /// `body_h` = 通用窗口留给 body 的高度（Full 时是整个视口减去 header）。
    fn body_element(&mut self, cx: &mut App, body_h: f32, card_w: f32, viewport: Size<Pixels>) -> AnyElement {
        match self.kind {
            WindowKind::Color => self.color_body(),
            WindowKind::Layout | WindowKind::Stroke | WindowKind::TypeSettings | WindowKind::EffectSettings => self.settings_element(cx),
            WindowKind::Variables | WindowKind::TextStyles => self.list_element(cx),
            WindowKind::VariablesTable => {
                if self.editor.is_none() {
                    if let Some(data) = self.variables_data.take() {
                        let editor = VariablesEditor::new(self.cmds.clone(), data, cx, self.var_on_full.clone(), self.var_on_close.clone());
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
        // Type settings 卡片 pad 为了让标题栏与 X 垂直居中改成 8；正文这里补左右各 4，
        // 把视觉内边距补回设计的 12（Layout / Stroke 仍是 pad 12，不加）。
        // body 插槽里的项间距统一由 `popover_body` 给（8pt），不在每个弹窗各写一个 gap。
        // TypeSettings 的 pad 少 4，这里补回左右各 4 的视觉内边距。
        let mut col = if self.kind == WindowKind::TypeSettings {
            crate::widgets::popover::popover_body().px(pt(4.0))
        } else {
            crate::widgets::popover::popover_body()
        };
        // Type settings 的页签已画在标题栏插槽里；Layout / Stroke 仍沿用正文顶部的页签行。
        if self.kind != WindowKind::TypeSettings {
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
        }
        let tab = self.tabs.as_ref().map(|(_, a)| *a.borrow()).unwrap_or(0);
        let list = self.settings.borrow();
        for setting in list.iter() {
            if let Some(el) = self.render_setting(setting, tab) {
                col = col.child(el);
            }
        }
        let _ = cx;
        // Details 正文很长（六个分组 + 十几条开关），限高滚动；Basics 短，不触发滚动条。
        // `overflow_y_scroll` 属于 `StatefulInteractiveElement`，需要一个元素 id，且调用后类型
        // 变成 `Stateful<Div>`，不能再赋回 `Div` 的 `col`，故在这里单独收尾。
        if self.kind == WindowKind::TypeSettings {
            col.id("type-settings-scroll").max_h(pt(468.0)).overflow_y_scroll().into_any_element()
        } else {
            col.into_any_element()
        }
    }

    /// 渲染设置窗口的一行；`ExpandedRows` 会递归调用它画被 `>` 展开的子行。
    fn render_setting(&self, setting: &Setting, tab: u8) -> Option<AnyElement> {
        let t = tokens();
        let type_ui = self.kind == WindowKind::TypeSettings;
        match setting {
            Setting::Caption { tab: st, text } => {
                if *st != tab {
                    return None;
                }
                Some(div().w_full().pt(pt(6.0)).pb(pt(2.0)).text_size(pt(10.0)).text_color(rgba(t.text)).child(text.clone()).into_any_element())
            }
            Setting::GroupTitle { tab: st, text } => {
                if *st != tab {
                    return None;
                }
                Some(
                    div()
                        .w_full()
                        .pt(pt(8.0))
                        .pb(pt(4.0))
                        .text_size(pt(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(t.text))
                        .child(text.clone())
                        .into_any_element(),
                )
            }
            Setting::Divider { tab: st } => {
                if *st != tab {
                    return None;
                }
                Some(div().w_full().h(px(1.0)).my(pt(4.0)).bg(rgba(t.border)).into_any_element())
            }
            Setting::Preview { text } => {
                // 预览框为 Basics / Details 共用；Variable 页只画可变字体轴。
                if tab == 2 {
                    return None;
                }
                Some(
                    div()
                        .w_full()
                        .h(pt(120.0))
                        .rounded(pt(5.0))
                        .bg(rgba(t.hover))
                        .flex()
                        .items_start()
                        .p(pt(16.0))
                        .text_size(pt(11.0))
                        .text_color(rgba(t.text_secondary))
                        .child(text.clone())
                        .into_any_element(),
                )
            }
            Setting::Number { tab: st, label, field } => {
                if *st != tab {
                    return None;
                }
                if type_ui {
                    let control = div().w(pt(72.0)).h(pt(24.0)).flex().items_center().child(field.clone()).into_any_element();
                    Some(type_row(label, &t, control))
                } else {
                    Some(
                        div()
                            .w_full()
                            .h(pt(28.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(6.0))
                            .child(div().flex_none().w(pt(76.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(label.clone()))
                            .child(div().flex_1().child(field.clone()))
                            .into_any_element(),
                    )
                }
            }
            Setting::Options { tab: st, label, active, options } => {
                if *st != tab {
                    return None;
                }
                let seg = self.options_segmented(options, *active, label);
                if !type_ui {
                    // Layout / Stroke：沿用原来的「放不下就标签上移」自适应行。
                    let inline_w = 134.0_f32;
                    let per_cell = inline_w / options.len().max(1) as f32;
                    let stacked = options.iter().any(|o| est_text_w(&o.label) + 10.0 > per_cell);
                    let cap = div().text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(label.clone());
                    return Some(if stacked {
                        div().w_full().flex().flex_col().gap(pt(4.0)).py(pt(2.0)).child(cap).child(seg).into_any_element()
                    } else {
                        div()
                            .w_full()
                            .min_h(pt(24.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(6.0))
                            .child(div().flex_none().w(pt(76.0)).text_size(pt(9.0)).text_color(rgba(t.text_secondary)).child(label.clone()))
                            .child(div().flex_1().min_w(px(0.0)).child(seg))
                            .into_any_element()
                    });
                }
                let bar = options.iter().map(|o| est_text_w(&o.label) + 16.0).sum::<f32>().max(48.0);
                let seg = seg.fixed_bar(bar);
                if label.is_empty() { Some(seg.into_any_element()) } else { Some(type_row(label, &t, seg.into_any_element())) }
            }
            Setting::IconOptions { tab: st, label, active, bar_w, options, drill_in } => {
                if *st != tab {
                    return None;
                }
                let mut seg = Segmented::new(options.iter().map(|o| o.item()).collect()).active(*active).fixed_bar(*bar_w);
                let cmds = self.cmds.clone();
                let settings = self.settings.clone();
                let opts = options.clone();
                let key = label.clone();
                seg = seg.on_pick(move |i, _, _| {
                    let Some(opt) = opts.get(i) else { return };
                    cmds.push(Cmd { id: opt.cmd, params: Some(opt.params.clone()) });
                    // 抬起格同步（命令引发的重绘会读到新值）。
                    if let Ok(mut list) = settings.try_borrow_mut() {
                        for s in list.iter_mut() {
                            if let Setting::IconOptions { label: l, active: a, .. } = s {
                                if *l == key {
                                    *a = Some(i);
                                }
                            }
                        }
                    }
                });
                let mut control: AnyElement = seg.into_any_element();
                if let Some(flag) = drill_in {
                    let f = flag.clone();
                    let wake = self.cmds.clone();
                    let chev = IconButton::new(Icon::ChevronRight).size(16.0).on_click(move |_, _, _| {
                        let mut b = f.borrow_mut();
                        *b = !*b;
                        drop(b);
                        wake.wake();
                    });
                    control = div().flex().flex_row().items_center().gap(pt(4.0)).child(control).child(chev).into_any_element();
                }
                Some(type_row(label, &t, control))
            }
            Setting::Dropdown { tab: st, label, current, anchor, options } => {
                if *st != tab {
                    return None;
                }
                let key = format!("type-settings-{}", label);
                let cur = current.borrow().clone();
                // 触发按钮：通用 `widgets::dropdown`（Input 模板同源 / bare 常态 / 发丝描边 / chevron），
                // 不再自己拼 div。点击把按钮窗口矩形回填为菜单锚点。
                let at = anchor.clone();
                let wake = self.cmds.clone();
                let trigger = dropdown(cur, 96.0, None).menu(key).on_click(move |rect, _window, _cx| {
                    *at.borrow_mut() = Some(rect);
                    wake.wake();
                });

                // 菜单：通用 `menu_popup_in`（`Menu::show` 的菜单表面 / hover / 勾选列 / 防溢出）。
                // 浮层占着根视图唯一的 popup 槽，故菜单由 `render()` 作为窗口级兄弟元素返回。
                let shown = *anchor.borrow();
                if let Some(rect) = shown {
                    let rows: Vec<MenuRow> =
                        options.iter().map(|o| MenuRow::new(o.label.clone()).checked(Some(o.label == *current.borrow()))).collect();
                    let opts = options.clone();
                    let cmds = self.cmds.clone();
                    let cur_state = current.clone();
                    let at2 = anchor.clone();
                    let wake2 = self.cmds.clone();
                    // 效果小窗：切换类型后下面那批数值行还属于旧类型，拿旧键写回会被引擎拒收。
                    // 选中即关窗，重新点开时按新类型重建设置项；其它窗口的行为不变。
                    let close_window = if self.kind == WindowKind::EffectSettings {
                        self.dismiss.borrow().clone()
                    } else {
                        None
                    };
                    let on_pick = Rc::new(move |picked: &str, w: &mut Window, cx: &mut App| {
                        if let Some(o) = opts.iter().find(|o| o.label == picked) {
                            *cur_state.borrow_mut() = o.label.clone();
                            cmds.push(Cmd { id: o.cmd, params: Some(o.params.clone()) });
                        }
                        *at2.borrow_mut() = None;
                        wake2.wake();
                        if let Some(cb) = &close_window {
                            cb(w, cx);
                        }
                    });
                    let at3 = anchor.clone();
                    let wake3 = self.cmds.clone();
                    let dismiss = Rc::new(move |_w: &mut Window, _cx: &mut App| {
                        *at3.borrow_mut() = None;
                        wake3.wake();
                    });
                    // 3 项的 Wrap style 不会溢出，vw/vh 传 0 关掉防溢出即可。
                    let menu = menu_popup_in(rect, None, rows, on_pick, dismiss, 0.0, 0.0);
                    self.overlays.borrow_mut().push(menu);
                }
                Some(type_row(label, &t, trigger.into_any_element()))
            }
            Setting::Toggle { tab: st, label, on, supported, on_cmd, off_cmd } => {
                if *st != tab {
                    return None;
                }
                let idx = if *on.borrow() { 1 } else { 0 };
                let mut seg = Segmented::new(vec![SegItem::Svg(TypIcon::None), SegItem::Icon(Icon::Check)]).active(Some(idx)).fixed_bar(48.0);
                if *supported {
                    let cmds = self.cmds.clone();
                    let state = on.clone();
                    let oc = on_cmd.clone();
                    let fc = off_cmd.clone();
                    seg = seg.on_pick(move |i, _, _| {
                        let val = i == 1;
                        *state.borrow_mut() = val;
                        let opt = if val { &oc } else { &fc };
                        cmds.push(Cmd { id: opt.cmd, params: Some(opt.params.clone()) });
                    });
                }
                let row = type_row(label, &t, seg.into_any_element());
                Some(if *supported { row } else { div().w_full().opacity(0.45).child(row).into_any_element() })
            }
            Setting::ExpandedRows { tab: st, flag, rows } => {
                if *st != tab || !*flag.borrow() {
                    return None;
                }
                let mut body = div().w_full().flex().flex_col().gap(pt(2.0)).pl(pt(2.0));
                for row in rows {
                    if let Some(el) = self.render_setting(row, tab) {
                        body = body.child(el);
                    }
                }
                Some(body.into_any_element())
            }
        }
    }

    /// 文字分段条：抬起格同步写回 retained `active`（命令引发的重绘会读到新值）。
    fn options_segmented(&self, options: &[Opt], active: Option<usize>, key: &SharedString) -> Segmented {
        let seg = Segmented::new(options.iter().map(|o| o.item()).collect()).active(active);
        let cmds = self.cmds.clone();
        let settings = self.settings.clone();
        let opts = options.to_vec();
        let key = key.clone();
        seg.on_pick(move |i, _, _| {
            let Some(opt) = opts.get(i) else { return };
            cmds.push(Cmd { id: opt.cmd, params: Some(opt.params.clone()) });
            if let Ok(mut list) = settings.try_borrow_mut() {
                for s in list.iter_mut() {
                    if let Setting::Options { label: l, active: a, .. } = s {
                        if *l == key {
                            *a = Some(i);
                        }
                    }
                }
            }
        })
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
        // 浮层内通用下拉的菜单每帧重建：先清空上一帧，body_element 里的 Dropdown 会重新登记。
        self.overlays.borrow_mut().clear();
        *self.dismiss.borrow_mut() = Some(dismiss.clone());
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
        // 菜单卡片与浮层卡片同挂窗口根（els），所以菜单用窗口坐标，不被浮层的滚动裁剪。
        let mut els = vec![card];
        els.extend(self.overlays.borrow_mut().drain(..));
        els
    }
}

/// 估算一段选项文字在 9.5 pt 下的宽度（CJK 一字约一个字号宽，拉丁约 0.55 字号）。
///
/// 只用于判断「这一行的多选器要不要改成上下堆叠」，所以宁可估宽一点：估算偏大只会多加一行，
/// 估算偏小就会把字切掉。
fn est_text_w(s: &str) -> f32 {
    s.chars().map(|c| if c.is_ascii() { 5.5 } else { 10.0 }).sum()
}

/// Type settings 的一行：左标签 + 弹性留白 + 右控件（贴合参考截图的右对齐布局）。
fn type_row(label: &SharedString, t: &crate::theme::Tokens, control: AnyElement) -> AnyElement {
    div()
        .w_full()
        .min_h(pt(24.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(6.0))
        .child(div().flex_none().min_w(px(0.0)).text_size(pt(10.0)).text_color(rgba(t.text)).child(label.clone()))
        .child(div().flex_1().min_w(px(0.0)))
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// 把一个数字框的提交接到 `node.set { props: { key: value } }`（Type settings 的数值行共用）。
///
/// 提交值按 `[min, max]` 夹紧 —— egui 那边是 `value.clamp(...)`，越界的键入不该写进文档。
/// 空文本视为「不动」：`Auto` 类的值（装饰线粗细）本来就允许留空。
/// clone 一份 effect JSON，只改一个顶层键，作为 `effect.set` 的 `effect` 参数。
/// 所有键都属于当前类型，不会触发引擎的 unknown properties 拒收。
fn effect_with(effect: &Value, key: &str, value: Value) -> Value {
    let mut e = effect.clone();
    if let Some(o) = e.as_object_mut() {
        o.insert(key.to_string(), value);
    }
    e
}

/// 一条数值设置项：显示值 = 存储值 × `scale`（密度 / 强度这类 0..1 用 100 显示成百分比），
/// 提交时反算、夹紧，然后只改一个键推 `effect.set`。`at = Some(i)` 用于 `offset: [x, y]`。
#[allow(clippy::too_many_arguments)]
fn effect_number(
    cx: &mut App,
    cmds: &Cmds,
    index: usize,
    effect: &Value,
    key: &'static str,
    at: Option<usize>,
    label: &'static str,
    suffix: &str,
    step: f64,
    scale: f64,
    min: f64,
    max: f64,
) -> Setting {
    let raw = match at {
        Some(i) => effect.get(key).and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64),
        None => effect.get(key).and_then(Value::as_f64),
    }
    .unwrap_or(0.0);
    let field = number_field(cx, 72.0, Some(raw * scale), suffix, step, Slot::None);
    let base = effect.clone();
    let queue = cmds.clone();
    field.update(cx, move |tf, _| {
        tf.set_on_commit(move |text, _, _| {
            let Ok(v) = text.trim().parse::<f64>() else { return };
            if !v.is_finite() {
                return;
            }
            let v = (v / scale).clamp(min, max);
            let mut e = base.clone();
            let Some(o) = e.as_object_mut() else { return };
            match at {
                Some(i) => {
                    let Some(arr) = o.get_mut(key).and_then(Value::as_array_mut) else { return };
                    if let Some(cell) = arr.get_mut(i) {
                        *cell = json!(v);
                    }
                }
                None => {
                    o.insert(key.to_string(), json!(v));
                }
            }
            queue.push(Cmd { id: "effect.set", params: Some(json!({ "index": index, "effect": e })) });
        });
    });
    Setting::Number { tab: 0, label: SharedString::from(label), field }
}

/// 按效果类型生成设置项（与 egui `effects.rs::effect_settings` 的字段对应；颜色需取色器，
/// 暂不搬入小窗）。所有写回都走 `effect.set`，参数只含当前类型自己的键。
fn effect_setting_rows(cx: &mut App, cmds: &Cmds, index: usize, effect: &Value, ty: &str) -> Vec<Setting> {
    let mut out: Vec<Setting> = Vec::new();
    macro_rules! n {
        ($key:expr, $at:expr, $label:expr, $suffix:expr, $step:expr, $scale:expr, $min:expr, $max:expr) => {
            out.push(effect_number(cx, cmds, index, effect, $key, $at, $label, $suffix, $step, $scale, $min, $max));
        };
    }
    macro_rules! tgl {
        ($key:expr, $label:expr) => {{
            let cur = effect.get($key).and_then(Value::as_bool).unwrap_or(false);
            let on_v = effect_with(effect, $key, json!(true));
            let off_v = effect_with(effect, $key, json!(false));
            out.push(Setting::Toggle {
                tab: 0,
                label: SharedString::from($label),
                on: Rc::new(RefCell::new(cur)),
                supported: true,
                on_cmd: Opt::new("On", "effect.set", json!({ "index": index, "effect": on_v })),
                off_cmd: Opt::new("Off", "effect.set", json!({ "index": index, "effect": off_v })),
            });
        }};
    }
    macro_rules! choice {
        ($key:expr, $label:expr, $values:expr) => {{
            let vals: &[(&str, &str)] = $values;
            let cur = effect.get($key).and_then(Value::as_str).unwrap_or("").to_string();
            let opts: Vec<Opt> = vals
                .iter()
                .map(|(l, v)| Opt::new(*l, "effect.set", json!({ "index": index, "effect": effect_with(effect, $key, json!(*v)) })))
                .collect();
            let active = vals.iter().position(|(_, v)| *v == cur.as_str());
            out.push(Setting::Options { tab: 0, label: SharedString::from($label), active, options: opts });
        }};
    }
    match ty {
        "dropShadow" | "innerShadow" => {
            n!("offset", Some(0), "X", "", 1.0, 1.0, -10_000.0, 10_000.0);
            n!("offset", Some(1), "Y", "", 1.0, 1.0, -10_000.0, 10_000.0);
            n!("radius", None, "Blur", "", 1.0, 1.0, 0.0, 10_000.0);
            n!("spread", None, "Spread", "", 1.0, 1.0, -10_000.0, 10_000.0);
            if ty == "dropShadow" {
                tgl!("show_behind", "Show behind transparent areas");
            }
        }
        "layerBlur" | "backgroundBlur" => {
            choice!("blur_type", "Blur type", &[("Uniform", "uniform"), ("Progressive", "progressive")]);
            n!("radius", None, "Blur", "", 1.0, 1.0, 0.0, 10_000.0);
            n!("start_radius", None, "Start", "", 1.0, 1.0, 0.0, 10_000.0);
        }
        "noise" => {
            choice!("mode", "Mode", &[("Mono", "mono"), ("Duo", "duo"), ("Multi", "multi")]);
            n!("size", None, "Size", "", 0.1, 1.0, 0.01, 1_000.0);
            n!("density", None, "Density", "%", 1.0, 100.0, 0.0, 1.0);
            n!("opacity", None, "Opacity", "%", 1.0, 100.0, 0.0, 1.0);
        }
        "texture" => {
            n!("size", None, "Size", "", 0.1, 1.0, 0.01, 1_000.0);
            n!("radius", None, "Radius", "", 1.0, 1.0, 0.0, 10_000.0);
            tgl!("clip_to_shape", "Clip to shape");
        }
        "glass" => {
            n!("light_angle", None, "Light angle", "\u{00b0}", 1.0, 1.0, -180.0, 180.0);
            n!("light_intensity", None, "Intensity", "%", 1.0, 100.0, 0.0, 1.0);
            n!("refraction", None, "Refraction", "%", 1.0, 100.0, 0.0, 1.0);
            n!("depth", None, "Depth", "", 1.0, 1.0, 1.0, 1_000.0);
            n!("dispersion", None, "Dispersion", "%", 1.0, 100.0, 0.0, 1.0);
            n!("frost", None, "Frost", "", 1.0, 1.0, 0.0, 100.0);
            n!("splay", None, "Splay", "%", 1.0, 100.0, 0.0, 1.0);
        }
        _ => {}
    }
    out
}

fn wire_set(field: &Entity<TextField>, cx: &mut App, cmds: &Cmds, key: &'static str, min: f64, max: f64) {
    let queue = cmds.clone();
    field.update(cx, move |tf, _| {
        tf.set_on_commit(move |text, _, _| {
            let Ok(v) = text.trim().parse::<f64>() else { return };
            if !v.is_finite() {
                return;
            }
            let v = v.clamp(min, max);
            queue.push(Cmd { id: "node.set", params: Some(json!({ "props": { key: v } })) });
        });
    });
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
        let r = Row::new("Brand/Primary", "variable.bind", json!({ "field": "fill" })).group(tr("Collection")).trail(tr("Color"));
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
        let mut w = WindowState::base(WindowKind::Layout, tr("Layout"), Cmds::new(&wake), Open::Anchor(Point { x: px(0.0), y: px(0.0) }));
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
        let mut w = WindowState::variables_table(
            Cmds::new(&wake),
            Point { x: px(100.0), y: px(100.0) },
            VariablesData { collections: Vec::new(), libraries: Vec::new() },
        );
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

    // ── Type settings（Basics / Details 1:1 布局重写）─────────────────────────────

    #[test]
    fn opt_glyph_becomes_an_svg_cell_and_plain_label_stays_text() {
        let g = Opt::new("Align left", "node.set", json!({})).with_glyph(crate::widgets::typ_icons::TypIcon::AlignLeft);
        assert!(matches!(g.item(), SegItem::Svg(crate::widgets::typ_icons::TypIcon::AlignLeft)));
        let plain = Opt::new("Auto", "node.set", json!({}));
        assert!(matches!(plain.item(), SegItem::Text(ref t) if t == &SharedString::from("Auto")));
    }

    // 把 data-driven 的分组结果摊平成 (kind, tag-ish) 便于断言：只数 GroupTitle / Toggle / Divider。
    fn group_summary(settings: &[Setting]) -> (Vec<String>, usize, usize) {
        let mut titles = Vec::new();
        let mut toggles = 0;
        let mut dividers = 0;
        for s in settings {
            match s {
                Setting::GroupTitle { text, .. } => titles.push(text.to_string()),
                Setting::Toggle { .. } => toggles += 1,
                Setting::Divider { .. } => dividers += 1,
                _ => panic!("feature groups should only contain title/toggle/divider"),
            }
        }
        (titles, toggles, dividers)
    }

    #[test]
    fn feature_rows_are_driven_by_what_the_font_scan_returns() {
        let features = std::collections::BTreeMap::new();
        // 扫描只报 3 个 tag：liga(Letter forms) / tnum(Numbers) / ss03(Stylistic sets)。
        let supported = ["liga", "tnum", "ss03"].iter().map(|s| s.to_string()).collect();
        let rows = WindowState::type_feature_groups(1, &features, &supported);
        let (titles, toggles, dividers) = group_summary(&rows);
        assert_eq!(toggles, 3, "只有扫描到的 3 个 tag 生成开关，其余一律不出现");
        assert_eq!(titles.len(), 3, "只显示有行的组");
        assert_eq!(dividers, 3, "每组收尾一条分隔线");
        // 组序与 lizard 一致：Letter case → Numbers → Letter forms → …；这里只出现
        // Numbers / Letter forms / Stylistic sets 三组，跳过空组（Letter case / Horizontal / Other）。
        assert!(titles.iter().any(|t| t.contains("Number") || t.contains("数字")), "missing Numbers: {:?}", titles);
        assert!(titles.iter().any(|t| t.contains("Letter forms") || t.contains("字母形式")), "missing Letter forms: {:?}", titles);
        assert!(titles.iter().any(|t| t.contains("Stylistic") || t.contains("样式集")), "missing Stylistic sets: {:?}", titles);
    }

    #[test]
    fn a_tag_the_font_lacks_produces_no_row_at_all() {
        let features = std::collections::BTreeMap::new();
        let supported = ["tnum"].iter().map(|s| s.to_string()).collect(); // 只有 tnum
        let rows = WindowState::type_feature_groups(1, &features, &supported);
        let (_, toggles, _) = group_summary(&rows);
        assert_eq!(toggles, 1, "liga/frac/kern 等字体没有的 feature 根本不生成行（不是置灰）");
        // 每个生成出来的行都必须 supported=true（它来自扫描结果）。
        for s in &rows {
            if let Setting::Toggle { supported, .. } = s {
                assert!(*supported, "数据驱动生成的行不可能 unsupported");
            }
        }
    }

    #[test]
    fn scanned_tag_outside_the_catalog_is_grouped_as_other() {
        let features = std::collections::BTreeMap::new();
        let supported = ["aalt", "liga"].iter().map(|s| s.to_string()).collect(); // aalt 不在本地表
        let rows = WindowState::type_feature_groups(1, &features, &supported);
        let (titles, toggles, _) = group_summary(&rows);
        assert_eq!(toggles, 2);
        assert!(titles.iter().any(|t| t.contains("Other") || t.contains("其它") || t.contains("其他")), "表外 tag 归 Other features: {:?}", titles);
    }

    #[test]
    fn set_features_writes_one_tag_incrementally() {
        let features = std::collections::BTreeMap::new();
        let supported = ["liga", "tnum"].iter().map(|s| s.to_string()).collect();
        let rows = WindowState::type_feature_groups(1, &features, &supported);
        // 按命令里的 tag 定位 liga（组序里 Numbers 在 Letter forms 前，不能用索引）。
        let liga = rows
            .iter()
            .find_map(|s| match s {
                Setting::Toggle { on_cmd, .. } if on_cmd.params["features"].get("liga").is_some() => Some(s),
                _ => None,
            })
            .expect("liga toggle must exist (it was scanned)");
        let Setting::Toggle { on, on_cmd, off_cmd, .. } = liga else { unreachable!() };
        assert_eq!(on_cmd.cmd, "text.setFeatures");
        assert_eq!(off_cmd.cmd, "text.setFeatures");
        assert_eq!(on_cmd.params["features"].as_object().unwrap().len(), 1, "每次只增量写一个 tag");
        assert!(*on.borrow(), "liga 默认开");
    }

    #[test]
    fn document_override_wins_over_the_catalog_default() {
        let mut features = std::collections::BTreeMap::new();
        features.insert("liga".to_string(), false); // 文档显式关掉（表里 liga 默认开）
        let supported = ["liga"].iter().map(|s| s.to_string()).collect();
        let rows = WindowState::type_feature_groups(1, &features, &supported);
        let Setting::Toggle { on, .. } = &rows[1] else { panic!("expected toggle after title") };
        assert!(!*on.borrow(), "文档覆盖应盖过默认开");
    }

    /// 效果设置小窗的数值行按这些键写回。键名必须与 `serde_json::to_value(Effect)` 一致，
    /// 否则 `effect.set` 会被引擎 `refuse_ignored_effect_keys` 判成 unknown properties 拒收。
    #[test]
    fn effect_setting_keys_match_the_serialized_effect() {
        let cases: &[(&str, figcraft_doc::Effect, &[&str])] = &[
            ("dropShadow", figcraft_doc::Effect::drop_shadow(), &["type", "offset", "radius", "spread", "show_behind", "visible"]),
            ("layerBlur", figcraft_doc::Effect::layer_blur(), &["type", "radius", "blur_type", "start_radius"]),
            ("noise", figcraft_doc::Effect::noise(), &["type", "mode", "size", "density", "opacity"]),
            ("texture", figcraft_doc::Effect::texture(), &["type", "size", "radius", "clip_to_shape"]),
            ("glass", figcraft_doc::Effect::glass(), &["type", "light_angle", "light_intensity", "refraction", "depth", "dispersion", "frost", "splay"]),
        ];
        for (name, e, keys) in cases {
            let v = serde_json::to_value(e).unwrap();
            for k in *keys {
                assert!(v.get(k).is_some(), "{name} JSON missing `{k}`: {v}");
            }
        }
    }

    /// 类型切换必须生成干净对象：旧实现 clone 旧 effect JSON 只改 `type`，
    /// 残留的 offset / spread / color 会让整条 `effect.set` 静默失效。
    #[test]
    fn switching_effect_type_drops_the_old_types_fields() {
        let old = serde_json::to_value(figcraft_doc::Effect::drop_shadow()).unwrap();
        assert!(old.get("offset").is_some() && old.get("spread").is_some());
        let clean = serde_json::to_value(figcraft_doc::Effect::of_type("glass").unwrap()).unwrap();
        assert_eq!(clean.get("type").and_then(Value::as_str), Some("glass"));
        for k in ["offset", "spread", "color"] {
            assert!(clean.get(k).is_none(), "clean glass must not carry shadow field `{k}`: {clean}");
        }
        // 小窗里的类型切换用的正是 `Effect::of_type`，逐类型都不残留别类的键。
        for (_, ty) in figcraft_doc::Effect::TYPES {
            let v = serde_json::to_value(figcraft_doc::Effect::of_type(ty).unwrap()).unwrap();
            assert_eq!(v.get("type").and_then(Value::as_str), Some(ty), "type tag for {ty}");
            assert!(v.as_object().unwrap().len() < 16, "{ty} carries too many fields: {v}");
        }
    }

}
