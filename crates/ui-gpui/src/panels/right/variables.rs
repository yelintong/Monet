//! 变量编辑器（`ui-egui/src/dialogs/variables_panel.rs` 的 GPUI 移植）。
//!
//! 范围（用户 2026-11-06）：整个面板 —— 顶部 header、左侧 sidebar（集合 / 分组）、右侧表格、
//! 底栏、Floating / Full 两种呈现、以及通用拖拽件 `crate::drag` 接进来的排序。
//!
//! 与右栏其它面板一样，这是**只读快照 + 命令队列**：`CanvasView::panel_data()` 每帧把
//! `Document` 读成 [`VariablesData`]，窗口持有它；所有编辑先乐观更新本地快照、再推 `Cmd`，
//! 由 `sync_panel` 统一送到引擎。窗口内部状态（选中、搜索、菜单、拖拽、输入框）是 retained 的。
//!
//! 这是**表格 / 弹窗**，不是右栏 `PanelSection` 行，所以不受「统一网格 / 禁止手算列宽」约束；
//! 列宽来自 egui 的 `NAME_W = 240` / `ADD_W = 40`，mode 列等分剩余宽度。

use figcraft_i18n::tr;
use crate::canvas::geom::Color32;
use crate::drag::{self, Config as DragCfg, Row as DragRow, Target as DragTarget};
use crate::panels::{Cmd, Cmds};
use crate::widgets::icon::paint as paint_icon;
use crate::widgets::{
    Icon, IconButton, PopoverOpen, Slot, Surface, TextField, number_field, popover_window, pt, rgba,
    search_field, text_field, title_text, tokens,
};
use gpui::{
    AnyElement, App, Bounds, Div, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, SharedString, Size, Stateful, Window, div, point,
    prelude::*, px, size,
};
use crate::panels::right::virtual_list::{VirtualListScrollHandle, v_virtual_list};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Instant;

/// 整卡拖动的按下回调（由通用窗口传入；只绑在头部左侧，不绑整张卡片）。
pub type DragStart = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

// ── egui `variables_panel.rs:25-50` 的常量 ────────────────────────────────────────────────
pub const SIDEBAR_W: f32 = 264.0;
pub const NAME_W: f32 = 240.0;
pub const ADD_W: f32 = 40.0;
pub const ROW_H: f32 = 40.0;
pub const HEAD_ROW_H: f32 = 40.0;
pub const GROUP_H: f32 = 40.0;
pub const GROUP_GAP: f32 = 16.0;
pub const BOTTOM_H: f32 = 40.0;
pub const HEADER_H: f32 = 40.0;
pub const SIDE_ROW_H: f32 = 32.0;
pub const FLOAT_W: f32 = 880.0;
pub const FLOAT_H: f32 = 480.0;
pub const WINDOW_W: f32 = FLOAT_W;
pub const WINDOW_H: f32 = FLOAT_H;
/// egui `Geom::MIN_W/MIN_H`：浮动面板 resize 的下限。
pub const MIN_W: f32 = 480.0;
pub const MIN_H: f32 = 240.0;
/// 列宽 / 侧栏宽的拖动边界（egui `SIDEBAR_MIN/MAX`、`NAME_MIN/MAX`、`MODE_MIN/MAX`）。
pub const SIDEBAR_MIN: f32 = 180.0;
pub const SIDEBAR_MAX: f32 = 420.0;
pub const NAME_MIN: f32 = 120.0;
pub const NAME_MAX: f32 = 420.0;
pub const MODE_MIN: f32 = 100.0;
pub const MODE_MAX: f32 = 420.0;
pub const MODE_W: f32 = 200.0;
/// 水平滚动条的条带高度（egui `HSCROLL_H`）；需要横滚时从行区高度里扣掉。
pub const HSCROLL_H: f32 = 12.0;
const SEARCH_W: f32 = 174.0;
/// 过滤按钮与搜索框拼接后的总宽（egui `SEARCH_W + FILTER_W`）。
const FILTER_W: f32 = 25.0;
const VALUE_FIELD_W: f32 = 150.0;
const TYPES: [(&str, VarKind); 4] = [("Color", VarKind::Color), ("Number", VarKind::Number), ("String", VarKind::String), ("Boolean", VarKind::Boolean)];

/// `figcraft_doc::VariableType` 的显示形态（egui `kind_glyph`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VarKind {
    #[default]
    Color,
    Number,
    String,
    Boolean,
}

impl VarKind {
    fn id(self) -> &'static str {
        match self {
            VarKind::Color => "color",
            VarKind::Number => "number",
            VarKind::String => "string",
            VarKind::Boolean => "boolean",
        }
    }
    fn index(self) -> usize {
        match self {
            VarKind::Color => 0,
            VarKind::Number => 1,
            VarKind::String => 2,
            VarKind::Boolean => 3,
        }
    }
}

/// 一行里某个 mode 的值（egui `value_cell`）。
#[derive(Clone, Debug, PartialEq)]
pub enum VarCell {
    Empty,
    Color { color: Color32, hex: String, alpha: u8 },
    Number(String),
    Text(String),
    Bool(bool),
    Alias(String),
}

impl VarCell {
    /// 值格输入框里显示的文本。
    fn text(&self) -> String {
        match self {
            VarCell::Color { hex, .. } => hex.clone(),
            VarCell::Number(s) | VarCell::Text(s) | VarCell::Alias(s) => s.clone(),
            VarCell::Bool(b) => b.to_string(),
            VarCell::Empty => String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarRow {
    pub id: String,
    /// 文档里的完整名（`Brand/Primary`）；详情面板改名、复制都用它，不能用短名。
    pub full_name: String,
    /// 分组内显示的短名（`rsplit('/')` 的最后一段）。
    pub name: String,
    /// 所属分组（去掉名字前缀），空串 = 未分组。
    pub group: String,
    pub kind: VarKind,
    /// 与集合 `modes` 一一对应。
    pub values: Vec<VarCell>,
    /// 每个 mode 是否为 override（扩展集合继承标记，画左侧 2pt 竖条）。
    pub overrides: Vec<bool>,
    /// 详情面板：描述、发布时隐藏、各平台的代码名、作用范围。
    pub description: String,
    pub hidden: bool,
    pub code: Vec<(String, String)>,
    pub scopes: Option<Vec<String>>,
    /// 扩展集合里继承来的变量：不可拖拽、改名、删除（egui `v.extends.is_some()`）。
    pub inherited: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeSnap {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectionSnap {
    pub id: String,
    pub name: String,
    pub modes: Vec<ModeSnap>,
    /// 文档顺序的全部变量（过滤 / 排序在渲染时做）。
    pub rows: Vec<VarRow>,
    /// 由变量名前缀推出的分组，按字母序。
    pub groups: Vec<String>,
    /// 来源库（`Some` = 库里的只读集合）。
    pub library: Option<String>,
    /// 扩展自哪个集合（egui `coll.extends`：值格右侧多留按钮位、显示 notice_row）。
    pub extends: Option<String>,
    /// 本地集合可编辑；库集合只读。
    pub editable: bool,
}

/// 一个已启用库（egui `View::of(lib)`）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibrarySnap {
    pub id: String,
    pub name: String,
    pub collections: Vec<CollectionSnap>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariablesData {
    pub collections: Vec<CollectionSnap>,
    pub libraries: Vec<LibrarySnap>,
}

fn kind_of(k: figcraft_doc::VariableType) -> VarKind {
    match k {
        figcraft_doc::VariableType::Color => VarKind::Color,
        figcraft_doc::VariableType::Number => VarKind::Number,
        figcraft_doc::VariableType::String => VarKind::String,
        figcraft_doc::VariableType::Boolean => VarKind::Boolean,
    }
}

fn cell_of(vars: &figcraft_doc::Variables, val: Option<&figcraft_doc::VariableValue>) -> VarCell {
    use figcraft_doc::VariableValue as V;
    let Some(val) = val else { return VarCell::Empty };
    match val {
        V::Color(c) => {
            let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            VarCell::Color {
                color: Color32::from_rgba_unmultiplied(ch(c.r), ch(c.g), ch(c.b), ch(c.a)),
                hex: c.to_hex(),
                alpha: (c.a.clamp(0.0, 1.0) * 100.0).round() as u8,
            }
        }
        V::Number(n) => {
            let s = if n.is_finite() && n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n}") };
            VarCell::Number(s)
        }
        V::String(s) => VarCell::Text(s.clone()),
        V::Boolean(b) => VarCell::Bool(*b),
        V::Alias(id) => {
            let name = vars.get(id).map(|v| v.name.clone()).unwrap_or_else(|| format!("→ {id}"));
            VarCell::Alias(name)
        }
        _ => VarCell::Empty,
    }
}

/// 从 `Document` 读快照（本地集合；库变量只读与 extends 继承留给后续）。
pub fn read(doc: &figcraft_doc::Document) -> VariablesData {
    let vars = &doc.variables;
    let collections = vars
        .collections
        .iter()
        .map(|c| {
            let modes: Vec<ModeSnap> = c.modes.iter().map(|m| ModeSnap { id: m.id.clone(), name: m.name.clone() }).collect();
            let mut groups: Vec<String> = Vec::new();
            let rows: Vec<VarRow> = vars
                .variables
                .iter()
                .filter(|v| v.collection == c.id)
                .map(|v| {
                    let (group, short) = match v.name.rsplit_once('/') {
                        Some((g, s)) => (g.to_string(), s.to_string()),
                        None => (String::new(), v.name.clone()),
                    };
                    if !group.is_empty() && !groups.contains(&group) {
                        groups.push(group.clone());
                    }
                    VarRow {
                        id: v.id.clone(),
                        full_name: v.name.clone(),
                        name: short,
                        group,
                        kind: kind_of(v.kind),
                        values: c.modes.iter().map(|m| cell_of(vars, v.values.get(&m.id))).collect(),
                        overrides: c.modes.iter().map(|m| v.overrides.contains(&m.id)).collect(),
                        description: v.description.clone(),
                        hidden: v.hidden_from_publishing,
                        code: ["WEB", "iOS", "ANDROID"]
                            .iter()
                            .map(|k| (k.to_string(), v.code_syntax.get(*k).cloned().unwrap_or_default()))
                            .collect(),
                        scopes: v.scopes.clone(),
                        inherited: v.extends.is_some(),
                    }
                })
                .collect();
            groups.sort();
            CollectionSnap {
                id: c.id.clone(),
                name: c.name.clone(),
                modes,
                rows,
                groups,
                library: None,
                extends: c.extends.clone(),
                editable: true,
            }
        })
        .collect();
    VariablesData { collections, libraries: Vec::new() }
}

/// 已启用库的只读集合（egui `View::of(lib)` + `published_collections`）。
pub fn read_libraries(cache: &figcraft_engine::library::Cache, doc: &figcraft_doc::Document) -> Vec<LibrarySnap> {
    use figcraft_engine::library;
    library::read_libraries(cache, doc)
        .map(|(link, lib)| {
            let vars = &lib.doc.variables;
            let collections = library::published_collections(lib)
                .map(|c| {
                    let modes: Vec<ModeSnap> = c.modes.iter().map(|m| ModeSnap { id: m.id.clone(), name: m.name.clone() }).collect();
                    let mut groups: Vec<String> = Vec::new();
                    let rows: Vec<VarRow> = library::published_variables(lib)
                        .filter(|v| v.collection == c.id)
                        .map(|v| {
                            let (group, short) = match v.name.rsplit_once('/') {
                                Some((g, s)) => (g.to_string(), s.to_string()),
                                None => (String::new(), v.name.clone()),
                            };
                            if !group.is_empty() && !groups.contains(&group) {
                                groups.push(group.clone());
                            }
                            VarRow {
                                id: v.id.clone(),
                                full_name: v.name.clone(),
                                name: short,
                                group,
                                kind: kind_of(v.kind),
                                values: c.modes.iter().map(|m| cell_of(vars, v.values.get(&m.id))).collect(),
                                overrides: c.modes.iter().map(|m| v.overrides.contains(&m.id)).collect(),
                                description: v.description.clone(),
                                hidden: v.hidden_from_publishing,
                                code: ["WEB", "iOS", "ANDROID"]
                                    .iter()
                                    .map(|k| (k.to_string(), v.code_syntax.get(*k).cloned().unwrap_or_default()))
                                    .collect(),
                                scopes: v.scopes.clone(),
                                inherited: v.extends.is_some(),
                            }
                        })
                        .collect();
                    groups.sort();
                    CollectionSnap {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        modes,
                        rows,
                        groups,
                        library: Some(link.id.clone()),
                        extends: c.extends.clone(),
                        editable: false,
                    }
                })
                .collect();
            LibrarySnap { id: link.id.clone(), name: link.name.clone(), collections }
        })
        .collect()
}

/// `FC_DEBUG_POPUP=variables_table` 用的合成数据。
pub fn sample() -> VariablesData {
    let color = |r: u8, g: u8, b: u8, hex: &str| VarCell::Color {
        color: Color32::from_rgb(r, g, b),
        hex: hex.to_string(),
        alpha: 100,
    };
    let row = |id: &str, name: &str, group: &str, kind: VarKind, values: Vec<VarCell>| VarRow {
        id: id.to_string(),
        full_name: if group.is_empty() { name.to_string() } else { format!("{group}/{name}") },
        name: name.to_string(),
        group: group.to_string(),
        kind,
        overrides: vec![false; values.len()],
        description: String::new(),
        hidden: false,
        code: vec![("WEB".into(), String::new()), ("iOS".into(), String::new()), ("ANDROID".into(), String::new())],
        scopes: None,
        inherited: false,
        values,
    };
    VariablesData {
        libraries: Vec::new(),
        collections: vec![CollectionSnap {
            id: "coll-sample".into(),
            name: "Design tokens".into(),
            modes: vec![ModeSnap { id: "m-light".into(), name: "Light".into() }, ModeSnap { id: "m-dark".into(), name: "Dark".into() }],
            library: None,
            extends: None,
            editable: true,
            groups: vec!["Brand".into(), "Text".into()],
            rows: vec![
                row("v-radius", "Radius", "", VarKind::Number, vec![VarCell::Number("8".into()), VarCell::Number("12".into())]),
                row("v-primary", "Primary", "Brand", VarKind::Color, vec![color(0x0D, 0x99, 0xFF, "0D99FF"), color(0x60, 0xA5, 0xFA, "60A5FA")]),
                row("v-on", "On primary", "Brand", VarKind::Color, vec![color(0xFF, 0xFF, 0xFF, "FFFFFF"), VarCell::Alias("Brand/Primary".into())]),
                row("v-font", "Font family", "Text", VarKind::String, vec![VarCell::Text("Inter".into()), VarCell::Text("Inter".into())]),
                row("v-bold", "Use bold", "Text", VarKind::Boolean, vec![VarCell::Bool(true), VarCell::Bool(false)]),
            ],
        }],
    }
}

// ── 编辑器状态 ────────────────────────────────────────────────────────────────────────────

/// 内联菜单（不占用全局 `Popup` 槽：变量窗口本身就是 `Popup::Window`，再开全局菜单会把它顶掉）。
#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub cmd: &'static str,
    pub params: Value,
}

impl MenuItem {
    fn new(label: impl Into<SharedString>, cmd: &'static str, params: Value) -> Self {
        Self { label: label.into(), cmd, params }
    }
}

#[derive(Clone)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
}

/// 一次列表拖拽（集合 / 分组 / 变量行共用），落点由 `crate::drag::resolve` 算。
pub struct ListDragState {
    pub kind: DragList,
    pub from: usize,
    /// 可见行的稳定标识：`ids[gap]` 用来拼 `before` 参数。
    pub ids: Vec<String>,
    pub rows: Vec<DragRow>,
    pub pointer: Point<Pixels>,
    pub target: Option<DragTarget>,
    pub armed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DragList {
    Collections,
    Groups,
    Variables,
}

/// 本地剪贴板：按 **mode 名** 存值（egui `CopiedVariable`），跨集合粘贴才不会串列。
#[derive(Clone, Debug)]
pub struct ClipRow {
    pub name: String,
    pub kind: VarKind,
    pub values: Vec<(String, VarCell)>,
}

/// 行内联编辑（改名 / mode 改名 / 值编辑）用到的 retained `TextField` 实体缓存。
///
/// 虚拟列表的渲染闭包是 `'static`、拿不到 `&mut VariablesEditor`，所以这些实体必须放在
/// `Rc<RefCell<..>>` 里，由闭包共享读写（参照通用虚拟列表的做法，只是把
/// `Entity<V>`/`V: Render` 的约束换成闭包捕获）。
#[derive(Default)]
struct RowCaches {
    rename_field: Option<Entity<TextField>>,
    rename_field_id: Option<String>,
    mode_rename_field: Option<Entity<TextField>>,
    mode_rename_field_id: Option<String>,
    value_editors: HashMap<String, Entity<TextField>>,
    /// sidebar 里集合 / 分组的内联改名框。
    collection_rename: Option<Entity<TextField>>,
    collection_rename_id: Option<String>,
    group_rename: Option<Entity<TextField>>,
    group_rename_id: Option<String>,
}

/// 虚拟列表中的「条目」：分组间隔、分组标题、或一条变量行。
enum RowItem {
    Gap,
    GroupHeader { name: String },
    Row { index: usize, row: VarRow },
}
impl RowItem {
    fn height(&self) -> f32 {
        match self {
            RowItem::Gap => GROUP_GAP,
            RowItem::GroupHeader { .. } => GROUP_H,
            RowItem::Row { .. } => ROW_H,
        }
    }
}

pub struct EditorState {
    /// 当前来源库（`None` = 本文件）。
    pub library: Option<String>,
    pub collection: usize,
    pub query: String,
    pub hidden: [bool; 4],
    pub group: Option<String>,
    pub sidebar: bool,
    pub full: bool,
    pub selected: Option<String>,
    /// 选中的整列（mode id）；与 `selected` 互斥。
    pub selected_mode: Option<String>,
    /// sidebar 里 "All variables" 是否被选中（与按组过滤区分）。
    pub selected_all: bool,
    /// 侧栏宽度与 Collections/Groups 的上下分割比例（egui `sidebar_w` / `sidebar_split`）。
    pub sidebar_w: f32,
    pub sidebar_split: f32,
    pub sidebar_resize: Option<(f32, f32)>,
    pub split_resize: Option<(f32, f32)>,
    /// 表格的水平滚动句柄（12pt 条带读它的 offset 画滑块）。
    pub hscroll: ScrollHandle,
    /// 横向滚动条拖动中：`(按下 x, 起始 offset, 滑块可移动行程, 最大滚动量)`。
    pub hscroll_drag: Option<(f32, f32, f32, f32)>,
    pub rename_collection: Option<String>,
    pub rename_group: Option<String>,
    pub rename: Option<String>,
    /// 正在内联改名的 mode id（`variable.renameMode`）。
    pub rename_mode: Option<String>,
    pub menu: Option<Menu>,
    pub drag: Option<ListDragState>,
    /// mode 列水平拖动：`(起始列下标, 按下 x, 当前 x)`。
    pub mode_drag: Option<(usize, f32, f32)>,
    /// Name 列宽（egui `NAME_W`，可拖 120..420）。
    pub name_w: f32,
    /// Name 列宽拖动中：`(按下 x, 起始宽度)`。
    pub col_resize: Option<(f32, f32)>,
    /// 每个 mode 列的宽度（egui `MODE_W`，可拖 100..420）。
    pub mode_ws: Vec<f32>,
    /// mode 列宽拖动中：`(列下标, 按下 x, 起始宽度)`。
    pub mode_resize: Option<(usize, f32, f32)>,
    /// 本地剪贴板：复制的整行（按 mode 名）。
    pub clipboard: Option<ClipRow>,
    /// 复制的整列：`(变量 id, 值)`，Paste column 时逐条 `variable.set`。
    pub clipboard_column: Option<Vec<(String, VarCell)>>,
    /// 分组显示顺序（egui 的 `group_order`，会话视图偏好，不进文档）。
    pub group_order: Vec<String>,
    /// 被折叠的分组（其下所有变量不在表格里显示）。
    pub collapsed: BTreeSet<String>,
    /// 正在编辑详情的变量 id（egui `details`）。
    pub details: Option<String>,
    /// 用户拖出来的窗口尺寸（None = 默认 FLOAT_W/FLOAT_H）。
    pub size: Option<(f32, f32)>,
    /// resize 拖动中：`(方向位, 按下 x, 按下 y, 起始宽, 起始高)`。
    pub resize: Option<(u8, f32, f32, f32, f32)>,
    /// 双击检测：上次按下的时间与目标 id。
    pub last_down: Option<(Instant, String)>,
    /// mode 表头的双击检测。
    pub last_mode_down: Option<(Instant, String)>,
    /// sidebar 集合 / 分组的双击检测。
    pub last_coll_down: Option<(Instant, usize)>,
    pub last_group_down: Option<(Instant, String)>,
    /// 变量表格的虚拟列表滚动句柄（跨帧保留，滚动位置不丢失）。
    pub scroll: VirtualListScrollHandle,
    /// root 在窗口里的真实矩形（由每帧的 canvas prepaint 写入）。插入线要把窗口坐标换算成
    /// root 局部坐标，而 prepaint 在本帧构建之后才跑——所以这个 `Rc` 必须**跨帧保留**，
    /// 每帧新建一个的话读到的永远是 `None`（插入线因此一直画不出来）。
    pub root_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl EditorState {
    fn new() -> Self {
        Self {
            library: None,
            collection: 0,
            query: String::new(),
            hidden: [false; 4],
            group: None,
            sidebar: true,
            full: false,
            selected: None,
            selected_mode: None,
            selected_all: true,
            sidebar_w: SIDEBAR_W,
            sidebar_split: 0.5,
            sidebar_resize: None,
            split_resize: None,
            hscroll: ScrollHandle::default(),
            hscroll_drag: None,
            rename_collection: None,
            rename_group: None,
            rename: None,
            rename_mode: None,
            menu: None,
            drag: None,
            mode_drag: None,
            name_w: NAME_W,
            col_resize: None,
            mode_ws: Vec::new(),
            mode_resize: None,
            clipboard: None,
            clipboard_column: None,
            group_order: Vec::new(),
            collapsed: BTreeSet::new(),
            details: None,
            size: None,
            resize: None,
            last_down: None,
            last_mode_down: None,
            last_coll_down: None,
            last_group_down: None,
            scroll: VirtualListScrollHandle::new(),
            root_bounds: Rc::new(std::cell::Cell::new(None)),
        }
    }
}

pub type State = Rc<RefCell<EditorState>>;

/// 完整的变量编辑器。`WindowState` 持有它，`render` 每帧调用。
pub struct VariablesEditor {
    pub state: State,
    pub data: VariablesData,
    cmds: Cmds,
    search: Entity<TextField>,
    /// 行内联编辑（改名 / mode 改名 / 值编辑）用到的 retained `TextField` 实体缓存。
    /// 放进 `Rc<RefCell<..>>` 以便虚拟列表的 `'static` 渲染闭包也能读写它们。
    row_caches: Rc<RefCell<RowCaches>>,
    details_fields: HashMap<String, Entity<TextField>>,
    on_full: Rc<dyn Fn(bool, &mut App)>,
    on_close: Rc<dyn Fn(&mut App)>,
}

/// 表格里的分组标题行（egui `group_rows` 的表头）：左 16pt 起、11pt 次要色、底部 1px 线。
/// **没有折叠三角** —— 折叠只在 sidebar 的 Groups 里做（egui 同）。
fn group_header_div(name: &str) -> Div {
    let t = tokens();
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(pt(GROUP_H))
        .flex()
        .items_center()
        .pl(pt(16.0))
        .text_size(pt(11.0))
        .text_color(rgba(t.text_secondary))
        .child(name.to_string())
        .child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)))
}

impl VariablesEditor {
    pub fn new(
        cmds: Cmds,
        data: VariablesData,
        cx: &mut App,
        on_full: Rc<dyn Fn(bool, &mut App)>,
        on_close: Rc<dyn Fn(&mut App)>,
    ) -> Self {
        let state: State = Rc::new(RefCell::new(EditorState::new()));
        let search = search_field(cx, SEARCH_W, "", "Search variables");
        let q = state.clone();
        search.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| q.borrow_mut().query = text.trim().to_lowercase());
        });
        Self {
            state,
            data,
            cmds,
            search,
            row_caches: Rc::new(RefCell::new(RowCaches::default())),
            details_fields: HashMap::new(),
            on_full,
            on_close,
        }
    }

    pub fn is_full(&self) -> bool {
        self.state.borrow().full
    }

    /// 由根视图写回「记住的 Full/Floating」。变量编辑器实体跨开关保留在视图上，
    /// 呈现模式是会话级状态，不该每次重新打开都回到 Floating。
    pub fn set_full(&self, full: bool) {
        self.state.borrow_mut().full = full;
    }

    /// 重新打开时刷新文档变量快照（交互状态仍保留）。
    pub fn set_data(&mut self, data: VariablesData) {
        self.data = data;
    }

    /// 8 个边缘/角 handle 是否正被拖动（窗口级拖拽路由用）。
    pub fn resizing(&self) -> bool {
        self.state.borrow().resize.is_some()
    }

    /// 是否有「内置拖拽」正在进行（列表行拖动 / 列宽 / 侧栏 / 分栏 / 模式列宽）。
    /// 这些由编辑器内部 `on_move` 处理，整卡移动必须让位——否则在表格行上拖动会同时把整张卡也拖走。
    pub fn internal_drag(&self) -> bool {
        let s = self.state.borrow();
        s.drag.is_some()
            || s.mode_drag.is_some()
            || s.col_resize.is_some()
            || s.sidebar_resize.is_some()
            || s.split_resize.is_some()
            || s.mode_resize.is_some()
            || s.hscroll_drag.is_some()
    }

    /// 拖拽中：由**根视图**用窗口坐标推进（GPUI 的元素级 `on_mouse_move` 只在指针悬停其
    /// hitbox 时触发，指针移出卡片就断；与 `motion/timeline.rs` 的 `route_global_move` 同构）。
    /// 这里只改状态；重绘由调用方 `cx.notify()` 负责。
    pub fn route_move(&self, at: Point<Pixels>) {
        Self::on_move(&self.state, at);
    }

    /// 抬起 / Escape：清掉所有进行中的尺寸拖动状态。
    pub fn end_resize(&self) {
        let mut s = self.state.borrow_mut();
        s.resize = None;
        s.col_resize = None;
        s.mode_resize = None;
        s.sidebar_resize = None;
        s.split_resize = None;
        // ⛔ 内部拖动也必须一起清：mouse-up 可能落在窗口外收不到，残留的 `drag` / `hscroll_drag`
        // 会让 `internal_drag()` 永远为真 → 根视图把画布的 move 全抢走。
        s.drag = None;
        s.mode_drag = None;
        s.hscroll_drag = None;
    }

    /// 用户拖出来的窗口尺寸（`WindowState::render` 用它覆盖默认宽高）。
    pub fn size(&self) -> Option<(f32, f32)> {
        self.state.borrow().size
    }

    /// 当前集合的变量总数（标题栏计数 chip）。
    pub fn total(&self) -> usize {
        self.coll().map(|c| c.rows.len()).unwrap_or(0)
    }

    /// 根视图在 `open_window` 时注入 Floating/Full 切换与关闭回调。
    pub fn set_callbacks(&mut self, on_full: Rc<dyn Fn(bool, &mut App)>, on_close: Rc<dyn Fn(&mut App)>) {
        self.on_full = on_full;
        self.on_close = on_close;
    }

    /// 键盘：Escape 先收掉内部菜单 / 改名 / 拖拽（没有内部状态就返回 false，让通用 Escape 关窗口）；
    /// Ctrl/Cmd+C / V 复制粘贴整行。返回 true = 这次按键已被变量面板消费。
    pub fn key(&self, key: &str, accel: bool) -> bool {
        let busy = {
            let s = self.state.borrow();
            s.menu.is_some()
                || s.rename.is_some()
                || s.rename_mode.is_some()
                || s.drag.is_some()
                || s.mode_drag.is_some()
                || s.col_resize.is_some()
                || s.mode_resize.is_some()
                || s.resize.is_some()
        };
        if key == "escape" {
            if busy {
                let mut s = self.state.borrow_mut();
                s.menu = None;
                s.rename = None;
                s.rename_mode = None;
                s.rename_collection = None;
                s.rename_group = None;
                s.drag = None;
                s.mode_drag = None;
                s.col_resize = None;
                s.mode_resize = None;
                s.sidebar_resize = None;
                s.split_resize = None;
                s.resize = None;
                return true;
            }
            // 没有内部状态时：先释放选中 / 关详情，再一次 Escape 才交给外层关窗口。
            let had = {
                let s = self.state.borrow();
                s.selected.is_some() || s.selected_mode.is_some() || s.details.is_some()
            };
            if had {
                let mut s = self.state.borrow_mut();
                s.selected = None;
                s.selected_mode = None;
                s.details = None;
                return true;
            }
            return false;
        }
        if accel && key.eq_ignore_ascii_case("c") {
            Self::copy_row(&self.state, &self.data);
            return true;
        }
        if accel && key.eq_ignore_ascii_case("v") {
            Self::paste_row(&self.state, &self.data, &self.cmds);
            return true;
        }
        false
    }

    /// 导出：系统「另存为」对话框 → `variable.export {format, path}`。
    fn export_dialog(cmds: &Cmds, format: &str) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&format!("variables.{format}"))
            .add_filter(format.to_uppercase(), &[format])
            .save_file()
        else {
            return;
        };
        cmds.push(Cmd {
            id: "variable.export",
            params: Some(json!({ "format": format, "path": path.to_string_lossy().to_string() })),
        });
    }

    /// 导入：系统「打开」对话框 → `variable.import {variables: path}`。
    fn import_dialog(cmds: &Cmds) {
        let Some(path) = rfd::FileDialog::new().add_filter("Variables JSON", &["json"]).pick_file() else {
            return;
        };
        cmds.push(Cmd {
            id: "variable.import",
            params: Some(json!({ "variables": path.to_string_lossy().to_string() })),
        });
    }

    /// 当前来源（本文件 / 库）下的集合切片。
    fn collections<'a>(st: &State, data: &'a VariablesData) -> &'a [CollectionSnap] {
        let lib = st.borrow().library.clone();
        match lib {
            Some(id) => data.libraries.iter().find(|l| l.id == id).map(|l| l.collections.as_slice()).unwrap_or(&[]),
            None => data.collections.as_slice(),
        }
    }

    /// 复制选中行：按 **mode 名** 存值，所以粘到 mode 名不同的集合也不会串列。
    fn copy_row(st: &State, data: &VariablesData) {
        let sel = st.borrow().selected.clone();
        let Some(sel) = sel else { return };
        let list = Self::collections(st, data);
        let Some((c, r)) = list.iter().find_map(|c| c.rows.iter().find(|r| r.id == sel).map(|r| (c, r))) else {
            return;
        };
        let values = c.modes.iter().zip(r.values.iter()).map(|(m, v)| (m.name.clone(), v.clone())).collect();
        st.borrow_mut().clipboard = Some(ClipRow { name: r.full_name.clone(), kind: r.kind, values });
    }

    /// 粘贴 = 在当前集合里**新建**一个变量（`variable.create` 带 `values {modeName: value}`），
    /// 而不是覆盖选中行的值 —— 与 egui `paste()` 一致。
    fn paste_row(st: &State, data: &VariablesData, cmds: &Cmds) {
        let clip = st.borrow().clipboard.clone();
        let Some(clip) = clip else { return };
        let list = Self::collections(st, data);
        let i = st.borrow().collection.min(list.len().saturating_sub(1));
        let Some(c) = list.get(i) else { return };
        if !c.editable {
            return;
        }
        let base = format!("{} copy", clip.name);
        let name = unique(&base, |n| c.rows.iter().any(|r| r.full_name == n));
        let mut values = serde_json::Map::new();
        for (mode, cell) in &clip.values {
            if let Some(v) = cell_value(cell) {
                values.insert(mode.clone(), v);
            }
        }
        cmds.push(Cmd {
            id: "variable.create",
            params: Some(json!({ "collection": c.id, "name": name, "type": clip.kind.id(), "values": Value::Object(values) })),
        });
    }

    /// 复制整列：记下当前可见行在该 mode 下的值。
    fn copy_column(st: &State, data: &VariablesData, mode: &str) {
        let list = Self::collections(st, data);
        let i = st.borrow().collection.min(list.len().saturating_sub(1));
        let Some(c) = list.get(i) else { return };
        let Some(mi) = c.modes.iter().position(|m| m.id == mode) else { return };
        let vals = c.rows.iter().filter_map(|r| r.values.get(mi).map(|v| (r.id.clone(), v.clone()))).collect();
        st.borrow_mut().clipboard_column = Some(vals);
    }

    /// 粘贴整列到目标 mode：每条一个 `variable.set`。
    fn paste_column(st: &State, cmds: &Cmds, mode: &str) {
        let Some(vals) = st.borrow().clipboard_column.clone() else { return };
        for (vid, cell) in vals {
            let Some(v) = cell_value(&cell) else { continue };
            cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mode, "value": v })) });
        }
    }

    fn push(&self, id: &'static str, params: Value) {
        self.cmds.push(Cmd { id, params: Some(params) });
    }

    /// 当前集合（越界时收敛到 0）。
    fn coll(&self) -> Option<&CollectionSnap> {
        let s = self.state.borrow();
        let list = match &s.library {
            Some(lib) => self.data.libraries.iter().find(|l| &l.id == lib).map(|l| l.collections.as_slice()).unwrap_or(&[]),
            None => self.data.collections.as_slice(),
        };
        list.get(s.collection.min(list.len().saturating_sub(1)))
    }

    fn coll_mut(&mut self) -> Option<&mut CollectionSnap> {
        let (lib, i) = {
            let s = self.state.borrow();
            (s.library.clone(), s.collection)
        };
        let list = match lib {
            Some(l) => self.data.libraries.iter_mut().find(|x| x.id == l).map(|x| &mut x.collections)?,
            None => &mut self.data.collections,
        };
        let i = i.min(list.len().saturating_sub(1));
        list.get_mut(i)
    }

    /// 过滤 + 分组排序后的可见变量行（egui `table()` 的 list）。
    fn visible_rows(&self, c: &CollectionSnap) -> Vec<VarRow> {
        let st = self.state.borrow();
        let q = st.query.as_str();
        let mut list: Vec<VarRow> = c
            .rows
            .iter()
            .filter(|v| {
                let in_group = st.group.as_ref().is_none_or(|g| v.group == *g);
                let full_name = if v.group.is_empty() { v.name.clone() } else { format!("{}/{}", v.group, v.name) };
                in_group
                    && !st.hidden[v.kind.index()]
                    && (q.is_empty() || full_name.to_lowercase().contains(q))
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            let pa = st.group_order.iter().position(|g| g == &a.group);
            let pb = st.group_order.iter().position(|g| g == &b.group);
            match (pa, pb) {
                (Some(i), Some(j)) => i.cmp(&j),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.group.cmp(&b.group),
            }
        });
        list
    }

    /// 主体：`body_h` 是通用窗口留给 body 的高度（Full 时是整个视口减去 header）；
    /// `viewport` 供详情浮层用通用 `Popover` 做边界夹取。
    pub fn render(&mut self, cx: &mut App, body_h: f32, viewport: Size<Pixels>, origin: Point<Pixels>, drag_start: Option<DragStart>, card_w: f32) -> AnyElement {
        let t = tokens();
        let st = self.state.clone();
        let cmds = self.cmds.clone();
        let data = self.data.clone();
        let coll = self.coll().cloned();
        let full = st.borrow().full;
        let (sidebar, sidebar_w) = {
            let s = st.borrow();
            (s.sidebar, s.sidebar_w)
        };
        // 表格可用宽度 = **卡片宽度**（不是窗口宽度）减去侧栏与其 1pt 分隔线。
        // 以前误用 `viewport.width`，导致表格/滚动条按整窗宽算，列宽和滚动条全错。
        let table_w = (card_w - if sidebar { sidebar_w + 1.0 } else { 0.0 }).max(120.0);

        // 根：按下任意处关掉内联菜单；松开时提交拖拽。
        // ⛔ 这里**不要**铺一块矩形 `bg(t.panel)`：`variables_table` 的卡片 `pad = 0`，这块矩形会
        // 正好铺满整张卡片、把卡片自己的圆角（`Popover` 的 `rounded(RADIUS)`）整个盖掉 —— 渲染出来
        // 就是直角。egui 的 `Frame::window` 圆角之所以露得出来，正是因为内容层不铺满底色。
        // 底色由外层卡片提供（同为 `t.panel`），这里保持透明即可。
        let mut root = div()
            .relative()
            .w_full()
            .h(pt(body_h))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                move |_e: &MouseDownEvent, _, _| st.borrow_mut().menu = None
            })
            .on_mouse_move({
                let st = st.clone();
                move |e: &MouseMoveEvent, _, _| Self::on_move(&st, e.position)
            })
            .on_mouse_up(MouseButton::Left, {
                let st = st.clone();
                let cmds = cmds.clone();
                let cid = coll.as_ref().map(|c| c.id.clone()).unwrap_or_default();
                let mode_ids: Vec<String> =
                    coll.as_ref().map(|c| c.modes.iter().map(|m| m.id.clone()).collect()).unwrap_or_default();
                move |_e: &MouseUpEvent, _, _| {
                    {
                        let mut s = st.borrow_mut();
                        s.col_resize = None;
                        s.mode_resize = None;
                        s.resize = None;
                        s.sidebar_resize = None;
                        s.split_resize = None;
                        s.hscroll_drag = None;
                    }
                    let d = st.borrow_mut().drag.take();
                    if let Some(d) = d {
                        Self::commit_drag(&st, &d, &cmds);
                    }
                    // mode 列拖动：按**实际列宽**累加，越过下一列宽度就前移/后移一位。
                    let (md, mode_ws) = {
                        let mut s = st.borrow_mut();
                        (s.mode_drag.take(), s.mode_ws.clone())
                    };
                    if let Some((from, x0, x1)) = md {
                        let dx = x1 - x0;
                        if !mode_ids.is_empty() && dx.abs() > 8.0 {
                            let mut to = from;
                            if dx > 0.0 {
                                let mut acc = 0.0;
                                for k in from..mode_ids.len().saturating_sub(1) {
                                    acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                                    if dx >= acc {
                                        to = k + 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                let mut acc = 0.0;
                                for k in (0..from).rev() {
                                    acc += mode_ws.get(k).copied().unwrap_or(MODE_W);
                                    if -dx >= acc {
                                        to = k;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if to != from {
                                if let Some(mid) = mode_ids.get(from) {
                                    cmds.push(Cmd {
                                        id: "variable.moveMode",
                                        params: Some(json!({ "collection": cid, "mode": mid, "to": to })),
                                    });
                                }
                            }
                        }
                    }
                }
            });

        root = root.child(self.header(&st, &data, full, drag_start));
        let body = div().flex().flex_row().flex_1().min_h(pt(0.0)).w_full();
        let body = if sidebar {
            body.child(self.sidebar(&st, &data, cx, body_h))
                .child(div().flex_none().w(pt(1.0)).h_full().bg(rgba(t.border)))
        } else {
            body
        };
        // 表格在窗口里的左缘：卡片原点 + 侧栏宽度（+1pt 分隔线）。
        let table_left = f32::from(origin.x) + if sidebar { sidebar_w + 1.0 } else { 0.0 };
        root = root.child(body.child(self.table(&st, &coll, cx, body_h, table_w, table_left)));
        root = root.child(self.bottom_bar(&st, &coll));
        root = root.child(self.menu_overlay(&st, origin));
        root = root.child(self.details_overlay(&st, cx, viewport));
        // 记录 root 在窗口里的真实矩形：行拖动要画的插入线，落点 y 是窗口坐标，
        // 而插入线挂在 root 这个 relative 容器里，必须换算成 root 局部坐标才画得对。
        // 跨帧保留的 root 矩形（prepaint 写入，下一帧读；每帧新建会永远读到 None）。
        let root_bounds = st.borrow().root_bounds.clone();
        let rb = root_bounds.clone();
        root = root.child(
            gpui::canvas(move |bounds, _, _| rb.set(Some(bounds)), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        if let Some(el) = self.drag_line(&st, root_bounds.get()) {
            root = root.child(el);
        }
        // 8 个 resize handle（四边 + 四角）。方向位：E=1 W=2 S=4 N=8。
        let (sw, sh) = st.borrow().size.unwrap_or((WINDOW_W, WINDOW_H));
        let handles: [(u8, Div); 8] = [
            (1, div().absolute().right(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full()),
            (2, div().absolute().left(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full()),
            (4, div().absolute().bottom(pt(0.0)).left(pt(0.0)).h(pt(6.0)).w_full()),
            (8, div().absolute().top(pt(0.0)).left(pt(0.0)).h(pt(6.0)).w_full()),
            (5, div().absolute().right(pt(0.0)).bottom(pt(0.0)).size(pt(12.0))),
            (6, div().absolute().left(pt(0.0)).bottom(pt(0.0)).size(pt(12.0))),
            (9, div().absolute().right(pt(0.0)).top(pt(0.0)).size(pt(12.0))),
            (10, div().absolute().left(pt(0.0)).top(pt(0.0)).size(pt(12.0))),
        ];
        for (dir, el) in handles {
            let st = st.clone();
            root = root.child(el.on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, _| {
                st.borrow_mut().resize = Some((dir, f32::from(e.position.x), f32::from(e.position.y), sw, sh));
            }));
        }
        let _ = full;
        root.into_any_element()
    }

    // ── header ────────────────────────────────────────────────────────────────────────────
    /// 第二层工具条。**标题与关闭按钮由外层通用 `Popover` 提供**（"Variables" + 计数 chip + X），
    /// 这里不再重复画一个 "Local variables" 标题或第二个 X —— 那会叠成 80pt 的双层 header。
    fn header(&mut self, st: &State, _data: &VariablesData, full: bool, drag_start: Option<DragStart>) -> Div {
        let t = tokens();
        let filtered = st.borrow().hidden.iter().any(|&h| h);
        let mut h = div()
            .flex_none()
            .w_full()
            .h(pt(HEADER_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .pl(pt(16.0))
            // 右侧给浮在右上角的 X 留出 32pt，最后一个工具按钮不会被盖住。
            .pr(pt(48.0))
            .border_b_1()
            .border_color(rgba(t.border));
        // 侧栏显示 / 隐藏（egui `vars_sidebar`）。
        // 标题 + 计数 chip 画进工具条（外层通用 Popover 已改成"整窗一个插槽"，
        // 不再单独占一行标题栏）。
        let title_group = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Variables")))
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
                    .child(format!("{}", self.total())),
            );
        // ⛔ 整卡拖动只绑在**头部左侧**（egui `vars_drag`：标题到工具按钮之前那块），
        // 不是整张卡片。绑整卡会让「点表格行 / 点按钮」也把窗口拖走（用户 2026-11-07）。
        // 标题 + 侧栏开关 + 之后的空白一起放进 `left`（flex_1），整块都能拖。
        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .flex_1()
            .min_w(pt(0.0))
            .child(title_group)
            // 注意：`Icon::Panel` 在 icon.rs 里仍是圆点占位（未实现），用已实现的 `Icon::Panels`。
            .child(IconButton::new(Icon::Panels).size(24.0).on_click({
                let st = st.clone();
                move |_, _, _| {
                    let mut s = st.borrow_mut();
                    s.sidebar = !s.sidebar;
                }
            }));
        if let Some(d) = drag_start {
            left = left.cursor_grab().on_mouse_down(MouseButton::Left, move |e, w, cx| d(e, w, cx));
        }
        h = h.child(left);

        // Import… / Export… 是两个独立按钮（egui header 的 right_to_left 条），不是一个混合菜单。
        let import_cmds = self.cmds.clone();
        h = h.child(
            div()
                .flex_none()
                .h(pt(24.0))
                .px(pt(6.0))
                .flex()
                .items_center()
                .rounded(pt(4.0))
                .cursor_pointer()
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| Self::import_dialog(&import_cmds))
                .child(tr("Import…")),
        );
        h = h.child(
            div()
                .flex_none()
                .h(pt(24.0))
                .px(pt(6.0))
                .flex()
                .items_center()
                .rounded(pt(4.0))
                .cursor_pointer()
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .hover(move |s| s.bg(rgba(t.hover)))
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, _| {
                        st.borrow_mut().menu = Some(Menu {
                            x: f32::from(e.position.x),
                            y: f32::from(e.position.y),
                            items: vec![
                                MenuItem::new("Variables JSON…", "ui.export", json!({ "format": "json" })),
                                MenuItem::new("CSS custom properties…", "ui.export", json!({ "format": "css" })),
                                MenuItem::new("Dart constants for Flutter…", "ui.export", json!({ "format": "dart" })),
                            ],
                        });
                    }
                })
                .child(tr("Export…")),
        );

        // 搜索框 174 + 过滤钮 25 拼成 egui 的 199 宽一体控件。
        h = h.child(
            div()
                .flex_none()
                .w(pt(SEARCH_W + FILTER_W))
                .h(pt(24.0))
                .flex()
                .flex_row()
                .child(div().flex_none().w(pt(SEARCH_W)).child(self.search.clone()))
                .child(IconButton::new(Icon::Eye).size(24.0).active(filtered).on_click({
                    let st = st.clone();
                    move |e, _, _| {
                        let mut s = st.borrow_mut();
                        let open = s.menu.take().is_none();
                        if open {
                            let mut items: Vec<MenuItem> = TYPES
                                .iter()
                                .enumerate()
                                .map(|(i, (name, _))| {
                                    MenuItem::new(format!("{} {}", if s.hidden[i] { "○" } else { "✓" }, name), "ui.filter", json!({ "kind": i }))
                                })
                                .collect();
                            if s.hidden.iter().any(|&h| h) {
                                items.push(MenuItem::new("Show every type", "ui.showAll", json!({})));
                            }
                            s.menu = Some(Menu { x: f32::from(e.position().x), y: f32::from(e.position().y), items });
                        }
                    }
                })),
        );
        // Floating / Full
        h = h.child(IconButton::new(Icon::Frame).size(24.0).active(full).on_click({
            let st = st.clone();
            let on_full = self.on_full.clone();
            move |_, _, cx: &mut App| {
                let v = {
                    let mut s = st.borrow_mut();
                    s.full = !s.full;
                    s.full
                };
                on_full(v, cx);
            }
        }));
        h
    }

    /// 集合内联改名框（egui `start_rename(Rename::Collection)`）：右键 Rename / 双击都进这里，
    /// 不再直接把名字改成 "<旧名> 2"。
    fn collection_rename_field(&self, cx: &mut App, st: &State, id: &str, initial: &str) -> Entity<TextField> {
        {
            let mut c = self.row_caches.borrow_mut();
            if c.collection_rename_id.as_deref() == Some(id) {
                if let Some(e) = &c.collection_rename {
                    return e.clone();
                }
            }
            c.collection_rename = None;
            c.collection_rename_id = Some(id.to_string());
        }
        let f = text_field(cx, 160.0, initial, "Collection", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let st = st.clone();
        let cid = id.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim();
                if !name.is_empty() {
                    cmds.push(Cmd { id: "variable.renameCollection", params: Some(json!({ "collection": cid, "name": name })) });
                }
                st.borrow_mut().rename_collection = None;
            });
        });
        self.row_caches.borrow_mut().collection_rename = Some(f.clone());
        f
    }

    /// 分组内联改名：把组内所有变量的名字前缀整体换掉（egui `rename_group` 的 batch）。
    fn group_rename_field(&self, cx: &mut App, st: &State, c: &CollectionSnap, group: &str, initial: &str) -> Entity<TextField> {
        {
            let mut k = self.row_caches.borrow_mut();
            if k.group_rename_id.as_deref() == Some(group) {
                if let Some(e) = &k.group_rename {
                    return e.clone();
                }
            }
            k.group_rename = None;
            k.group_rename_id = Some(group.to_string());
        }
        let f = text_field(cx, 160.0, initial, "Group", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let st = st.clone();
        let old = group.to_string();
        let prefix = format!("{group}/");
        let members: Vec<(String, String)> = c
            .rows
            .iter()
            .filter(|r| r.full_name.starts_with(&prefix))
            .map(|r| (r.id.clone(), r.full_name[prefix.len()..].to_string()))
            .collect();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim().replace('/', "-");
                if !name.is_empty() && name != old {
                    for (id, rest) in &members {
                        cmds.push(Cmd {
                            id: "variable.update",
                            params: Some(json!({ "variable": id, "name": format!("{name}/{rest}") })),
                        });
                    }
                }
                st.borrow_mut().rename_group = None;
            });
        });
        self.row_caches.borrow_mut().group_rename = Some(f.clone());
        f
    }

    // ── sidebar ───────────────────────────────────────────────────────────────────────────
    fn sidebar(&self, st: &State, data: &VariablesData, cx: &mut App, body_h: f32) -> Stateful<Div> {
        let t = tokens();
        let (cur, group_sel, collapsed, library_sel, sidebar_w, split, renaming_coll, renaming_group, selected_all) = {
            let s = st.borrow();
            (
                s.collection.min(data.collections.len().saturating_sub(1)),
                s.group.clone(),
                s.collapsed.clone(),
                s.library.clone(),
                s.sidebar_w,
                s.sidebar_split,
                s.rename_collection.clone(),
                s.rename_group.clone(),
                s.selected_all,
            )
        };
        // egui `sidebar()`：Collections 标题 34 + 独立滚动列表 + 7pt 可拖分隔条 +
        // Groups 标题 34 + 独立滚动列表。两个区各自滚动，Groups 不会被库列表挤到屏幕外。
        let avail = (body_h - HEADER_H - BOTTOM_H).max(160.0);
        let split_total = (avail - 34.0 - 7.0 - 34.0).max(80.0);
        let collections_h = (split * split_total).clamp(40.0, (split_total - 40.0).max(40.0));
        let groups_h = (split_total - collections_h).max(40.0);
        let mut col = div().id("vars-sidebar").flex_none().w(pt(sidebar_w)).h_full().flex().flex_col().bg(rgba(t.panel));

        // ① Collections 标题 + 新建（unique 命名，不再用时间戳）。
        col = col.child(
            div()
                .flex_none().w_full().h(pt(34.0)).flex().flex_row().items_center().pl(pt(16.0))
                .text_size(pt(11.0)).text_color(rgba(t.text_secondary))
                .child(tr("Collections"))
                .child(div().flex_1().min_w(pt(0.0)))
                .child(IconButton::new(Icon::Plus).id("vars-add-collection").size(24.0).on_click({
                    let cmds = self.cmds.clone();
                    let taken: Vec<String> = data.collections.iter().map(|c| c.name.clone()).collect();
                    move |_, _, _| {
                        let name = unique("Collection", |n| taken.iter().any(|x| x == n));
                        cmds.push(Cmd { id: "variable.createCollection", params: Some(json!({ "name": name, "modes": ["Mode 1"] })) });
                    }
                })),
        );
        // ② Collections 列表（独立滚动）。
        let coll_ids: Vec<String> = data.collections.iter().map(|c| c.id.clone()).collect();
        let mut cl = div().id("var-collections").flex_none().w_full().h(pt(collections_h)).flex().flex_col().overflow_y_scroll();
        for (i, c) in data.collections.iter().enumerate() {
            let active = library_sel.is_none() && i == cur;
            let mut inner = div().w_full().h(pt(24.0)).flex().flex_row().items_center().pl(pt(8.0)).pr(pt(8.0)).rounded(pt(5.0));
            // 高亮块内缩 4pt（egui `hl = r.shrink2(vec2(0,4))`）；选中时不再叠 hover，避免选中色被覆盖。
            if active {
                inner = inner.bg(rgba(t.selected_row));
            } else {
                let hover = t.hover;
                inner = inner.hover(move |s| s.bg(rgba(hover)));
            }
            inner = if renaming_coll.as_deref() == Some(c.id.as_str()) {
                inner.child(self.collection_rename_field(cx, st, &c.id, &c.name))
            } else {
                inner.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(c.name.clone()))
            };
            inner = inner.child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(format!("{}", c.rows.len())));
            cl = cl.child(
                div()
                    .id(SharedString::from(format!("vcoll-{i}")))
                    .flex_none().w_full().h(pt(SIDE_ROW_H)).flex().flex_row().items_center().px(pt(8.0))
                    .on_mouse_down(MouseButton::Left, {
                        let st = st.clone();
                        let ids = coll_ids.clone();
                        let cid = c.id.clone();
                        move |e: &MouseDownEvent, _, _| {
                            let mut s = st.borrow_mut();
                            s.library = None;
                            s.collection = i;
                            s.group = None;
                            s.selected_all = true;
                            s.menu = None;
                            let dbl = s.last_coll_down.as_ref().is_some_and(|(at, j)| *j == i && at.elapsed().as_millis() < 400);
                            s.last_coll_down = Some((Instant::now(), i));
                            if dbl {
                                s.rename_collection = Some(cid.clone());
                            } else {
                                s.drag = Some(Self::begin_drag(DragList::Collections, i, &ids, e.position, &[]));
                            }
                        }
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let st = st.clone();
                        let cid = c.id.clone();
                        move |e: &MouseDownEvent, _, _| {
                            st.borrow_mut().menu = Some(Menu {
                                x: f32::from(e.position.x), y: f32::from(e.position.y),
                                items: vec![
                                    MenuItem::new("Rename collection", "ui.renameCollection", json!({ "collection": cid })),
                                    MenuItem::new("Delete collection", "variable.deleteCollection", json!({ "collection": cid })),
                                ],
                            });
                        }
                    })
                    .child(inner),
            );
        }
        // 已启用库：库标题带图标，集合只读、右键 Extend collection。
        for lib in &data.libraries {
            cl = cl.child(
                div()
                    .flex_none().w_full().h(pt(26.0)).flex().flex_row().items_center().gap(pt(6.0)).pl(pt(22.0))
                    .text_size(pt(11.0)).text_color(rgba(t.text_secondary))
                    .child(paint_icon(Icon::Component, rgba(t.text_secondary), 12.0))
                    .child(lib.name.clone()),
            );
            for (i, lc) in lib.collections.iter().enumerate() {
                let active = library_sel.as_deref() == Some(lib.id.as_str()) && i == cur;
                let lid = lib.id.clone();
                let cid = lc.id.clone();
                let count = lc.rows.len();
                let name = lc.name.clone();
                let mut inner = div().w_full().h(pt(24.0)).flex().flex_row().items_center().pl(pt(26.0)).pr(pt(8.0)).rounded(pt(5.0));
                if active {
                    inner = inner.bg(rgba(t.selected_row));
                } else {
                    let hover = t.hover;
                    inner = inner.hover(move |s| s.bg(rgba(hover)));
                }
                inner = inner
                    .child(div().flex_1().min_w(pt(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(name))
                    .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(format!("{count}")));
                cl = cl.child(
                    div()
                        .flex_none().w_full().h(pt(SIDE_ROW_H)).flex().flex_row().items_center().px(pt(8.0))
                        .on_mouse_down(MouseButton::Left, {
                            let st = st.clone();
                            move |_: &MouseDownEvent, _, _| {
                                let mut s = st.borrow_mut();
                                s.library = Some(lid.clone());
                                s.collection = i;
                                s.group = None;
                                s.selected_all = true;
                                s.query.clear();
                                s.menu = None;
                            }
                        })
                        .on_mouse_down(MouseButton::Right, {
                            let st = st.clone();
                            let lid_r = lib.id.clone();
                            move |e: &MouseDownEvent, _, _| {
                                st.borrow_mut().menu = Some(Menu {
                                    x: f32::from(e.position.x), y: f32::from(e.position.y),
                                    items: vec![MenuItem::new("Extend collection", "library.extendCollection", json!({ "library": lid_r, "collection": cid }))],
                                });
                            }
                        })
                        .child(inner),
                );
            }
        }
        col = col.child(cl);

        // ③ 7pt 可拖分隔条（egui `vars_split`）。
        col = col.child(
            div()
                .flex_none().w_full().h(pt(7.0)).flex().items_center().cursor_pointer()
                .child(div().w_full().h(pt(1.0)).bg(rgba(t.border)))
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, _| {
                        let mut s = st.borrow_mut();
                        s.split_resize = Some((f32::from(e.position.y), s.sidebar_split));
                    }
                }),
        );
        // ④ Groups 标题。
        col = col.child(
            div().flex_none().w_full().h(pt(34.0)).flex().items_center().pl(pt(16.0))
                .text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Groups")),
        );
        let coll_snap = match &library_sel {
            Some(l) => data.libraries.iter().find(|x| &x.id == l).and_then(|x| x.collections.get(cur)).cloned(),
            None => data.collections.get(cur).cloned(),
        };
        let groups = coll_snap.as_ref().map(|c| c.groups.clone()).unwrap_or_default();
        let group_ids: Vec<String> = groups.clone();
        let mut gl = div().id("var-groups").flex_none().w_full().h(pt(groups_h)).flex().flex_col().overflow_y_scroll();
        let mut all_inner = div().w_full().h(pt(24.0)).flex().items_center().pl(pt(8.0)).rounded(pt(5.0));
        if selected_all {
            all_inner = all_inner.bg(rgba(t.selected_row));
        } else {
            let hover = t.hover;
            all_inner = all_inner.hover(move |s| s.bg(rgba(hover)));
        }
        all_inner = all_inner.child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("All variables")));
        gl = gl.child(
            div()
                .flex_none().w_full().h(pt(SIDE_ROW_H)).flex().items_center().px(pt(8.0))
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |_: &MouseDownEvent, _, _| {
                        let mut s = st.borrow_mut();
                        s.group = None;
                        s.selected_all = true;
                        s.menu = None;
                    }
                })
                .child(all_inner),
        );
        for (i, g) in groups.iter().enumerate() {
            // 祖先被折叠 → 子组不画（egui `group_rows` 的 collapsed_groups）。
            if collapsed.iter().any(|c| g.starts_with(&format!("{c}/"))) {
                continue;
            }
            let active = group_sel.as_deref() == Some(g.as_str()) && !selected_all;
            let is_collapsed = collapsed.contains(g);
            let level = g.matches('/').count() + 1;
            let has_children = groups.iter().any(|other| other.starts_with(&format!("{g}/")));
            let short = g.rsplit('/').next().unwrap_or(g).to_string();
            let count = coll_snap.as_ref().map(|c| c.rows.iter().filter(|r| r.group == *g).count()).unwrap_or(0);
            let mut inner = div().w_full().h(pt(24.0)).flex().flex_row().items_center().rounded(pt(5.0));
            if active {
                inner = inner.bg(rgba(t.selected_row));
            } else {
                let hover = t.hover;
                inner = inner.hover(move |s| s.bg(rgba(hover)));
            }
            let x0 = 8.0 + (level - 1) as f32 * 24.0;
            inner = inner.child(
                div()
                    .flex_none().w(pt(24.0)).h_full().flex().items_center().justify_center()
                    .child(
                        if has_children {
                            paint_icon(if is_collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, rgba(t.text_secondary), 12.0).into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    ),
            );
            inner = if renaming_group.as_deref() == Some(g.as_str()) {
                let f = coll_snap.as_ref().map(|c| self.group_rename_field(cx, st, c, g, &short));
                match f {
                    Some(f) => inner.child(f),
                    None => inner.child(short.clone()),
                }
            } else {
                inner.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(short))
            };
            inner = inner.child(div().pr(pt(8.0)).text_size(pt(10.0)).text_color(rgba(t.text_tertiary)).child(format!("{count}")));
            let g_toggle = g.clone();
            let g_sel = g.clone();
            let g_dbl = g.clone();
            let vars: Vec<String> = coll_snap.as_ref().map(|c| c.rows.iter().filter(|r| r.group == *g).map(|r| r.id.clone()).collect()).unwrap_or_default();
            gl = gl.child(
                div()
                    .flex_none().w_full().h(pt(SIDE_ROW_H)).flex().items_center().pl(pt(x0))
                    .on_mouse_down(MouseButton::Left, {
                        let st = st.clone();
                        let ids = group_ids.clone();
                        move |e: &MouseDownEvent, _, _| {
                            let mut s = st.borrow_mut();
                            s.menu = None;
                            let dbl = s.last_group_down.as_ref().is_some_and(|(at, name)| name == &g_dbl && at.elapsed().as_millis() < 400);
                            s.last_group_down = Some((Instant::now(), g_dbl.clone()));
                            if dbl {
                                s.rename_group = Some(g_dbl.clone());
                            } else {
                                s.group = Some(g_sel.clone());
                                s.selected_all = false;
                                s.drag = Some(Self::begin_drag(DragList::Groups, i, &ids, e.position, &[]));
                            }
                        }
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let st = st.clone();
                        move |e: &MouseDownEvent, _, _| {
                            st.borrow_mut().menu = Some(Menu {
                                x: f32::from(e.position.x), y: f32::from(e.position.y),
                                items: vec![
                                    MenuItem::new("Rename group", "ui.renameGroup", json!({ "group": g_toggle })),
                                    MenuItem::new("Delete group", "ui.deleteGroup", json!({ "variables": vars })),
                                ],
                            });
                        }
                    })
                    .child(inner),
            );
        }
        col = col.child(gl);
        // ⑥ 侧栏右缘 6pt 拖动柄（egui `vars_sidebar_edge`，180..420）。
        col.child(
            div()
                .absolute().right(pt(0.0)).top(pt(0.0)).w(pt(6.0)).h_full()
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, _| {
                        let mut s = st.borrow_mut();
                        s.sidebar_resize = Some((f32::from(e.position.x), s.sidebar_w));
                    }
                }),
        )
    }

    // ── table ─────────────────────────────────────────────────────────────────────────────
    fn table(
        &mut self,
        st: &State,
        coll: &Option<CollectionSnap>,
        cx: &mut App,
        body_h: f32,
        table_w: f32,
        table_left: f32,
    ) -> Stateful<Div> {
        let t = tokens();
        let Some(c) = coll else {
            return div()
                .id("vars-empty")
                .flex_1()
                .min_w(pt(0.0))
                .h_full()
                .flex()
                .flex_col()
                .child(
                    div().flex_1().w_full().flex().flex_col().items_center().justify_center().gap(pt(12.0))
                        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Variables store reusable values with modes such as Light and Dark.")))
                        .child(IconButton::new(Icon::Plus).id("vars-add-collection-empty").size(24.0).on_click({
                            let cmds = self.cmds.clone();
                            move |_, _, _| cmds.push(Cmd { id: "variable.createCollection", params: Some(json!({ "name": "Collection 1", "modes": ["Mode 1"] })) })
                        })),
                );
        };
        // 列宽数组与集合的 mode 数对齐，算出一个确定的表格总宽：横向滚动靠它。
        {
            let mut s = st.borrow_mut();
            while s.mode_ws.len() < c.modes.len() {
                s.mode_ws.push(200.0);
            }
            s.mode_ws.truncate(c.modes.len());
        }
        let (name_w, mode_ws) = {
            let s = st.borrow();
            (s.name_w, s.mode_ws.clone())
        };
        // egui `Columns::room`：Name 列先让位，保证 name + add + 一段 mode 区一定放进表格宽度。
        // 以前直接用 `name_w` 并且拿**窗口宽**当表格宽，表格会整块超出卡片。
        let name_eff = name_w.clamp(NAME_MIN, NAME_MAX).min((table_w - ADD_W - 2.0).max(0.0));
        // 只有 mode 列横向滚动：Name / add 列固定。`modes_full` 含每列 1pt 网格线。
        let modes_room = (table_w - name_eff - ADD_W - 2.0).max(0.0);
        let modes_full: f32 = mode_ws.iter().map(|w| w + 1.0).sum();
        let max_sx = (modes_full - (modes_room + 1.0)).max(0.0);
        let sx = (-f32::from(st.borrow().hscroll.offset().x)).clamp(0.0, max_sx);
        let mut col = div().flex_1().min_h(pt(0.0)).w_full().flex().flex_col();
        // 表头（底部分隔线 + 每列右缘 1px 网格线）
        let (selected_mode, has_clip_col) = {
            let s = st.borrow();
            (s.selected_mode.clone(), s.clipboard_column.is_some())
        };
        let mut head = div().relative().flex_none().w_full().h(pt(HEAD_ROW_H)).flex().flex_row().items_center();
        let name_w = name_eff;
        // Name 列：双击拖动柄 = 自动适配到最长完整变量名。
        let auto_name_w = {
            let longest = c.rows.iter().map(|r| r.full_name.chars().count()).max().unwrap_or(0) as f32;
            (longest * 7.0 + 40.0 + 24.0).clamp(NAME_MIN, NAME_MAX)
        };
        head = head.child(
            div()
                .relative()
                .flex_none()
                .w(pt(name_w))
                .h_full()
                .flex()
                .items_center()
                .pl(pt(16.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Name"))
                .child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)))
                // 右缘 6pt 拖动柄；双击自动适配。
                .child(
                    div()
                        .absolute()
                        .right(pt(0.0))
                        .top(pt(0.0))
                        .w(pt(6.0))
                        .h_full()
                        .on_mouse_down(MouseButton::Left, {
                            let st = st.clone();
                            move |e: &MouseDownEvent, _, cx: &mut App| {
                                // 列宽柄是单元格的子节点：不拦冒泡的话，父级 mode 单元格的
                                // on_mouse_down 会把 `last_mode_down` 覆盖成 mode id，双击判定
                                // 永远等不到同一个键 → 双击列边永远不生效（用户 2026-11-07）。
                                cx.stop_propagation();
                                let mut s = st.borrow_mut();
                                let dbl = s.last_mode_down.as_ref().is_some_and(|(at, id)| id == "__name" && at.elapsed().as_millis() < 400);
                                s.last_mode_down = Some((Instant::now(), "__name".into()));
                                if dbl {
                                    s.name_w = auto_name_w;
                                } else {
                                    s.col_resize = Some((f32::from(e.position.x), s.name_w));
                                }
                            }
                        }),
                ),
        );
        let n_modes = c.modes.len();
        // mode 列区：固定宽度 + 裁切，内容按 `sx` 左移（egui `modes_rect` + `sx`）。
        // Name / add 列在它两侧固定不动，所以横向滚动不会把 Name 列一起滚走。
        let mut modes_head = div().flex_none().w(pt(modes_room)).h_full().relative().overflow_hidden();
        let mut modes_inner = div().absolute().left(pt(-sx)).top_0().h_full().flex().flex_row();
        for (mi, m) in c.modes.iter().enumerate() {
            let renaming = st.borrow().rename_mode.as_deref() == Some(m.id.as_str());
            let mw = mode_ws.get(mi).copied().unwrap_or(MODE_W);
            let is_sel = selected_mode.as_deref() == Some(m.id.as_str());
            // 双击列边自动适配：该列最长值文本宽。
            let auto_mw = {
                let longest = c.rows.iter().map(|r| r.values.get(mi).map(|v| v.text().chars().count()).unwrap_or(0)).max().unwrap_or(0) as f32;
                (longest * 7.0 + 24.0).clamp(MODE_MIN, MODE_MAX)
            };
            let mut cell = div()
                .relative()
                .flex_none()
                .w(pt(mw))
                .h_full()
                .flex()
                .items_center()
                .pl(pt(4.0))
                .overflow_hidden()
                .text_size(pt(11.0))
                .text_color(rgba(if is_sel { t.text } else { t.text_secondary }))
                .bg(rgba(if is_sel { t.hover } else { t.panel }));
            cell = if renaming {
                cell.child(VariablesEditor::ensure_mode_rename_field(&self.row_caches, &self.cmds, cx, &m.name, st, &c.id, &m.id))
            } else {
                cell.child(m.name.clone())
            };
            let mid = m.id.clone();
            let mname = m.name.clone();
            let cid = c.id.clone();
            cell = cell
                .on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    let mid = mid.clone();
                    let editable = c.editable;
                    move |e: &MouseDownEvent, _, _| {
                        let mut s = st.borrow_mut();
                        s.menu = None;
                        s.selected = None;
                        s.selected_mode = Some(mid.clone());
                        if !editable {
                            return;
                        }
                        let dbl = s.last_mode_down.as_ref().is_some_and(|(at, id)| id == &mid && at.elapsed().as_millis() < 400);
                        s.last_mode_down = Some((Instant::now(), mid.clone()));
                        if dbl {
                            s.rename_mode = Some(mid.clone());
                        } else {
                            s.rename_mode = None;
                            let x = f32::from(e.position.x);
                            s.mode_drag = Some((mi, x, x));
                        }
                    }
                })
                .on_mouse_down(MouseButton::Right, {
                    let st = st.clone();
                    let mid = mid.clone();
                    let cid = cid.clone();
                    let mname = mname.clone();
                    let editable = c.editable;
                    move |e: &MouseDownEvent, _, _| {
                        if !editable {
                            return;
                        }
                        let mut items = vec![
                            MenuItem::new("Rename mode", "ui.renameMode", json!({ "mode": mid.clone() })),
                            MenuItem::new("Add mode", "variable.addMode", json!({ "collection": cid.clone(), "name": "Mode" })),
                            MenuItem::new("Duplicate mode", "variable.addMode", json!({ "collection": cid.clone(), "name": format!("{} copy", mname), "from": mid.clone() })),
                        ];
                        if mi > 0 {
                            items.push(MenuItem::new("Set as default", "variable.moveMode", json!({ "collection": cid.clone(), "mode": mid.clone(), "to": 0 })));
                            items.push(MenuItem::new("Move left", "variable.moveMode", json!({ "collection": cid.clone(), "mode": mid.clone(), "to": mi - 1 })));
                        }
                        if mi + 1 < n_modes {
                            items.push(MenuItem::new("Move right", "variable.moveMode", json!({ "collection": cid.clone(), "mode": mid.clone(), "to": mi + 1 })));
                        }
                        items.push(MenuItem::new("Copy column", "ui.copyColumn", json!({ "mode": mid.clone() })));
                        if has_clip_col {
                            items.push(MenuItem::new("Paste column", "ui.pasteColumn", json!({ "mode": mid.clone() })));
                        }
                        if n_modes > 1 {
                            items.push(MenuItem::new("Delete mode", "variable.removeMode", json!({ "collection": cid.clone(), "mode": mid.clone() })));
                        }
                        st.borrow_mut().menu = Some(Menu { x: f32::from(e.position.x), y: f32::from(e.position.y), items });
                    }
                });
            cell = cell.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
            cell = cell.child(
                div()
                    .absolute()
                    .right(pt(0.0))
                    .top(pt(0.0))
                    .w(pt(6.0))
                    .h_full()
                    .on_mouse_down(MouseButton::Left, {
                        let st = st.clone();
                        let edge_key = format!("{mid}-edge");
                        move |e: &MouseDownEvent, _, cx: &mut App| {
                            // 同 Name 列：先拦冒泡，父级单元格不会再把 `last_mode_down`
                            // 覆盖成 mode id（否则双击列边自动宽度永远不生效）。
                            cx.stop_propagation();
                            let mut s = st.borrow_mut();
                            let dbl = s.last_mode_down.as_ref().is_some_and(|(at, id)| id == &edge_key && at.elapsed().as_millis() < 400);
                            s.last_mode_down = Some((Instant::now(), edge_key.clone()));
                            if dbl {
                                if let Some(w) = s.mode_ws.get_mut(mi) {
                                    *w = auto_mw;
                                }
                            } else {
                                s.mode_resize = Some((mi, f32::from(e.position.x), mw));
                            }
                        }
                    }),
            );
            modes_inner = modes_inner.child(cell);
        }
        modes_head = modes_head.child(modes_inner);
        head = head.child(modes_head);
        let mut add_cell = div().relative().flex_none().w(pt(ADD_W)).h_full().flex().items_center().justify_center();
        if c.editable {
            add_cell = add_cell.child(IconButton::new(Icon::Plus).id("vars-add-mode").size(24.0).on_click({
                    let cmds = self.cmds.clone();
                    let cid = c.id.clone();
                    move |_, _, _| cmds.push(Cmd { id: "variable.addMode", params: Some(json!({ "collection": cid, "name": "Mode" })) })
                }));
        }
        add_cell = add_cell.child(div().absolute().left_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
        head = head.child(add_cell);
        head = head.child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)));
        col = col.child(head);

        // notice_row：库集合只读 / 扩展集合提示（egui `notice_row`）。
        if !c.editable || c.extends.is_some() {
            let msg = if !c.editable { "Library collection · read-only" } else { "Extended collection" };
            let mut n = div()
                .flex_none()
                .w_full()
                .h(pt(32.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(8.0))
                .pl(pt(16.0))
                .pr(pt(8.0))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(msg);
            if !c.editable {
                let cmds = self.cmds.clone();
                let lib = c.library.clone().unwrap_or_default();
                let cid = c.id.clone();
                n = n.child(div().flex_1().min_w(pt(0.0))).child(
                    div()
                        .h(pt(22.0))
                        .px(pt(8.0))
                        .rounded(pt(4.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .child(tr("Extend collection"))
                        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                            cmds.push(Cmd { id: "library.extendCollection", params: Some(json!({ "library": lib, "collection": cid })) });
                        }),
                );
            }
            col = col.child(n);
        }

        // 行区（虚拟列表：只渲染可视区间内的条目，几千行也不卡）。
        // 底部常驻 12pt 水平滚动条带（egui `HSCROLL_H`），所以行区高度要扣掉它。
        let rows_h = (body_h - HEADER_H - HEAD_ROW_H - BOTTOM_H - HSCROLL_H).max(96.0);
        let visible = self.visible_rows(c);
        let ids: Vec<String> = visible.iter().map(|r| r.id.clone()).collect();

        // 把变量行（含分组间隔 / 分组标题）摊平成一个固定顺序的「条目」序列，
        // 每项高度已知，虚拟列表据此做前缀和 + 二分求可视区间。
        let collapsed = st.borrow().collapsed.clone();
        let mut items: Vec<RowItem> = Vec::new();
        let mut prev_group: Option<String> = None;
        let mut skip = false;
        for (i, r) in visible.iter().enumerate() {
            if prev_group.as_deref() != Some(&r.group) {
                prev_group = Some(r.group.clone());
                skip = collapsed.contains(&r.group);
                if !r.group.is_empty() {
                    items.push(RowItem::Gap);
                    items.push(RowItem::GroupHeader { name: r.group.clone() });
                }
            }
            if skip {
                continue;
            }
            items.push(RowItem::Row { index: i, row: r.clone() });
        }

        // 每个可见变量行在「内容坐标」里的顶边。分组间隔 / 分组标题都占高度，行拖动重建
        // 矩形时必须按真实偏移算，不能假设行是连续等距的——否则带分组的集合一拖就整体错位。
        let row_tops: Rc<Vec<f32>> = Rc::new({
            let mut tops = vec![0.0f32; visible.len()];
            let mut y = 0.0f32;
            for it in &items {
                if let RowItem::Row { index, .. } = it {
                    if let Some(slot) = tops.get_mut(*index) {
                        *slot = y;
                    }
                }
                y += it.height();
            }
            tops
        });

        if visible.is_empty() {
            col = col.child(
                div().flex_none().w_full().h(pt(ROW_H)).flex().items_center().pl(pt(16.0))
                    .text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("No matching variables")),
            );
        } else {
            // 每项尺寸：宽 = 表格可用宽度（行内 Name/add 固定、mode 区自己裁切）；
            // 高 = 该条目类型固定高度。
            let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
                items.iter().map(|it| size(px(table_w), px(it.height()))).collect(),
            );
            let scroll = st.borrow().scroll.clone();
            let st2 = st.clone();
            let cmds2 = self.cmds.clone();
            let caches2 = self.row_caches.clone();
            let c2 = c.clone();
            let ids2 = ids.clone();
            let list = v_virtual_list("vars-rows", &scroll, item_sizes, move |range, _window, cx| {
                let mut out: Vec<AnyElement> = Vec::new();
                for ix in range {
                    let Some(item) = items.get(ix) else { continue };
                    let el: AnyElement = match item {
                        RowItem::Gap => div()
                            .flex_none()
                            .w(px(table_w))
                            .h(px(GROUP_GAP))
                            .into_any_element(),
                        RowItem::GroupHeader { name } => {
                            group_header_div(name).w(px(table_w)).into_any_element()
                        }
                        RowItem::Row { index, row } => VariablesEditor::var_row(
                            &st2, &cmds2, &caches2, &c2, row, *index, &ids2, row_tops.clone(), name_eff, modes_room, sx, cx,
                        )
                        .w(px(table_w))
                        .into_any_element(),
                    };
                    out.push(el);
                }
                out
            })
            .h(pt(rows_h));
            col = col.child(list);
        }
        // 12pt 水平滚动条带（egui `hscroll`）：只覆盖 mode 区（Name 与 add 之间）。
        let hscroll = st.borrow().hscroll.clone();
        let track_start = name_eff + 9.0;
        let track_w = (table_w - ADD_W - 9.0 - track_start).max(1.0);
        let thumb_w = (track_w * (modes_room / modes_full.max(1.0)).clamp(0.12, 1.0)).min(track_w);
        let max_scroll = max_sx.max(1.0);
        let off = sx.clamp(0.0, max_sx);
        let thumb_x = off / max_scroll * (track_w - thumb_w);
        let bar = div()
            .flex_none()
            .w_full()
            .h(pt(HSCROLL_H))
            .relative()
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                let hscroll = hscroll.clone();
                move |e: &MouseDownEvent, _, _| {
                    let rel = f32::from(e.position.x) - table_left - track_start;
                    let on_thumb = rel >= thumb_x && rel <= thumb_x + thumb_w;
                    let mut s = st.borrow_mut();
                    let start = if on_thumb {
                        sx
                    } else {
                        // 点轨道空白：先跳到指针处，再从那里接着拖。
                        let x = rel.clamp(0.0, track_w);
                        let target = (x / (track_w - thumb_w).max(1.0) * max_sx).clamp(0.0, max_sx);
                        hscroll.set_offset(point(px(-target), px(0.0)));
                        target
                    };
                    s.hscroll_drag = Some((f32::from(e.position.x), start, (track_w - thumb_w).max(1.0), max_sx));
                }
            })
            .child(div().absolute().left(pt(track_start)).top(pt(5.0)).w(pt(track_w)).h(pt(2.0)).bg(rgba(t.border)))
            .child(
                div()
                    .absolute()
                    .left(pt(track_start + thumb_x))
                    .top(pt(4.0))
                    .w(pt(thumb_w))
                    .h(pt(4.0))
                    .rounded(pt(2.0))
                    .bg(rgba(t.border_strong)),
            );
        div()
            .id("vars-table")
            .flex_1()
            .min_w(pt(0.0))
            .h_full()
            .flex()
            .flex_col()
            .child(col)
            .child(bar)
    }

    #[allow(clippy::too_many_arguments)]
    fn var_row(
        st: &State,
        cmds: &Cmds,
        caches: &Rc<RefCell<RowCaches>>,
        c: &CollectionSnap,
        r: &VarRow,
        index: usize,
        ids: &[String],
        tops: Rc<Vec<f32>>,
        name_eff: f32,
        modes_room: f32,
        sx: f32,
        cx: &mut App,
    ) -> Div {
        let t = tokens();
        let (selected, renaming, mode_ws, has_clip) = {
            let s = st.borrow();
            (
                s.selected.as_deref() == Some(r.id.as_str()),
                s.rename.as_deref() == Some(r.id.as_str()),
                s.mode_ws.clone(),
                s.clipboard.is_some(),
            )
        };
        // 库变量 / 扩展集合里继承来的变量：不可拖拽、改名、删除。
        let movable = c.editable && !r.inherited;
        let has_override = r.overrides.iter().any(|&o| o);
        let mut row = div().relative().flex_none().w_full().h(pt(ROW_H)).flex().flex_row().items_center();
        row = if selected { row.bg(rgba(t.selected_row)) } else { row.hover(move |s| s.bg(rgba(t.hover))) };

        // 行选择 / 双击改名 / 右键菜单 / 拖拽
        let rid = r.id.clone();
        let rname = r.name.clone();
        row = row
            .on_mouse_down(MouseButton::Left, {
                let st = st.clone();
                let rid = rid.clone();
                let ids = ids.to_vec();
                move |e: &MouseDownEvent, _, _| {
                    let mut s = st.borrow_mut();
                    s.menu = None;
                    s.selected_mode = None;
                    let dbl = s.last_down.as_ref().is_some_and(|(at, id)| id == &rid && at.elapsed().as_millis() < 400);
                    s.last_down = Some((Instant::now(), rid.clone()));
                    s.selected = Some(rid.clone());
                    if dbl && movable {
                        s.rename = Some(rid.clone());
                    } else if s.rename.as_deref() != Some(rid.as_str()) {
                        s.rename = None;
                        if movable {
                            s.drag = Some(Self::begin_drag(DragList::Variables, index, &ids, e.position, tops.as_slice()));
                        }
                    }
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let st = st.clone();
                let rid = rid.clone();
                move |e: &MouseDownEvent, _, _| {
                    let items = if movable {
                        let mut v = vec![
                            MenuItem::new("Edit variable", "ui.details", json!({ "variable": rid.clone() })),
                            MenuItem::new("Rename", "ui.rename", json!({ "variable": rid.clone() })),
                            MenuItem::new("Copy", "ui.copy", json!({})),
                        ];
                        if has_clip {
                            v.push(MenuItem::new("Paste", "ui.paste", json!({})));
                        }
                        v.push(MenuItem::new("Duplicate", "variable.duplicate", json!({ "variable": rid.clone() })));
                        if has_override {
                            v.push(MenuItem::new("Reset overrides", "variable.resetOverride", json!({ "variable": rid.clone() })));
                        }
                        v.push(MenuItem::new("Delete", "variable.delete", json!({ "variable": rid.clone() })));
                        v
                    } else {
                        vec![MenuItem::new("Copy", "ui.copy", json!({}))]
                    };
                    st.borrow_mut().menu = Some(Menu { x: f32::from(e.position.x), y: f32::from(e.position.y), items });
                }
            });

        // Name 列（右缘 1px 网格线）
        let mut name_cell = div()
            .relative()
            .flex_none()
            .w(pt(name_eff))
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_none().w(pt(40.0)).h_full().flex().items_center().pl(pt(17.0)).child(kind_glyph(r)));
        if renaming {
            let ed = Self::ensure_rename_field(caches, cmds, cx, &rname, st, &rid);
            name_cell = name_cell.child(div().flex_1().min_w(pt(0.0)).child(ed));
        } else {
            name_cell = name_cell.child(
                div().flex_1().min_w(pt(0.0)).overflow_hidden().text_size(pt(11.0)).text_color(rgba(t.text)).child(rname.clone()),
            );
        }
        name_cell = name_cell.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
        row = row.child(name_cell);

        // 值格（右缘 1px 网格线；override 竖条 + 重置按钮）
        // mode 区同样固定宽度 + 裁切 + 按 `sx` 左移，Name / add 列固定不动。
        let mut modes_strip = div().flex_none().w(pt(modes_room)).h_full().relative().overflow_hidden();
        let mut modes_inner = div().absolute().left(pt(-sx)).top_0().h_full().flex().flex_row();
        for (mi, m) in c.modes.iter().enumerate() {
            let cell = r.values.get(mi).cloned().unwrap_or(VarCell::Empty);
            let overridden = r.overrides.get(mi).copied().unwrap_or(false);
            let cw = mode_ws.get(mi).copied().unwrap_or(MODE_W);
            let mut cellbox = div()
                .relative()
                .flex_none()
                .w(pt(cw))
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(4.0))
                .pl(pt(4.0))
                .pr(pt(8.0));
            if overridden {
                cellbox = cellbox.child(div().absolute().left(pt(4.0)).top(pt(8.0)).w(pt(2.0)).h(pt(24.0)).bg(rgba(t.accent)));
            }
            cellbox = cellbox.child(Self::value_cell(caches, cmds, st, c, r, m, &cell, cx));
            if overridden && c.editable {
                let vid = r.id.clone();
                let mid = m.id.clone();
                let reset_cmds = cmds.clone();
                cellbox = cellbox.child(IconButton::new(Icon::Detach).size(16.0).on_click(move |_, _, _| {
                    reset_cmds.push(Cmd { id: "variable.resetOverride", params: Some(json!({ "variable": vid, "mode": mid })) });
                }));
            }
            cellbox = cellbox.child(div().absolute().right_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border)));
            modes_inner = modes_inner.child(cellbox);
        }
        modes_strip = modes_strip.child(modes_inner);
        row = row.child(modes_strip);
        // 表尾 40pt 列：Edit variable（打开详情，egui 的 `Icon::Adjust` 在 add 列中心）。
        row = row.child(
            div()
                .relative()
                .flex_none()
                .w(pt(ADD_W))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(IconButton::new(Icon::Pencil).size(20.0).on_click({
                    let st = st.clone();
                    let rid = rid.clone();
                    move |_, _, _| st.borrow_mut().details = Some(rid.clone())
                }))
                .child(div().absolute().left_0().top_0().w(pt(1.0)).h_full().bg(rgba(t.border))),
        );
        row.child(div().absolute().bottom_0().left_0().w_full().h(pt(1.0)).bg(rgba(t.border)))
    }

    fn ensure_rename_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        initial: &str,
        st: &State,
        rid: &str,
    ) -> Entity<TextField> {
        // 换了一行改名时必须重建输入框，否则会沿用上一行的初值。
        if caches.borrow().rename_field_id.as_deref() == Some(rid) {
            if let Some(e) = &caches.borrow().rename_field {
                return e.clone();
            }
        }
        caches.borrow_mut().rename_field = None;
        caches.borrow_mut().rename_field_id = Some(rid.to_string());
        let f = text_field(cx, NAME_W - 80.0, initial, "Name", Slot::None, Slot::None);
        let cmds = cmds.clone();
        let st = st.clone();
        let rid = rid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim();
                if !name.is_empty() {
                    cmds.push(Cmd { id: "variable.update", params: Some(json!({ "variable": rid, "name": name })) });
                }
                let mut s = st.borrow_mut();
                s.rename = None;
                s.last_down = None;
            });
        });
        caches.borrow_mut().rename_field = Some(f.clone());
        f
    }

    /// mode 表头的内联改名框（`variable.renameMode`）。同样按目标 id 缓存，避免串值。
    fn ensure_mode_rename_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        initial: &str,
        st: &State,
        cid: &str,
        mid: &str,
    ) -> Entity<TextField> {
        if caches.borrow().mode_rename_field_id.as_deref() == Some(mid) {
            if let Some(e) = &caches.borrow().mode_rename_field {
                return e.clone();
            }
        }
        caches.borrow_mut().mode_rename_field = None;
        caches.borrow_mut().mode_rename_field_id = Some(mid.to_string());
        let f = text_field(cx, 160.0, initial, "Mode", Slot::None, Slot::None);
        let cmds = cmds.clone();
        let st = st.clone();
        let cid = cid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim();
                if !name.is_empty() {
                    cmds.push(Cmd {
                        id: "variable.renameMode",
                        params: Some(json!({ "collection": cid, "mode": mid, "name": name })),
                    });
                }
                let mut s = st.borrow_mut();
                s.rename_mode = None;
                s.last_mode_down = None;
            });
        });
        caches.borrow_mut().mode_rename_field = Some(f.clone());
        f
    }

    /// 值格的统一底框（24 高 / 圆角 5 / `t.field` 底 / 6pt 内边距）。
    fn value_shell(t: &crate::theme::Tokens) -> Div {
        div()
            .flex_1()
            .min_w(pt(0.0))
            .h(pt(24.0))
            .rounded(pt(5.0))
            .bg(rgba(t.field))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .px(pt(6.0))
            .overflow_hidden()
            .text_size(pt(11.0))
            .text_color(rgba(t.text))
    }

    /// 颜色 swatch（16×16 / 圆角 3 / 1px 边框）。
    fn value_swatch(c: Color32, t: &crate::theme::Tokens) -> Div {
        div().flex_none().size(pt(16.0)).rounded(pt(3.0)).bg(rgba(c)).border_1().border_color(rgba(t.border))
    }

    /// 文本型值域（Color 的 hex / String）。
    fn text_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        initial: &str,
        vid: &str,
        mid: &str,
        kind: VarKind,
        width: f32,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = text_field(cx, width, initial, "", Slot::None, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let value = encode_value(kind, text);
                cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": value })) });
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    /// 数字型值域：用通用 `number_field`（箭头步进 / 拖动 scrub / 相对表达式）。
    fn number_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        value: Option<f64>,
        vid: &str,
        mid: &str,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = number_field(cx, VALUE_FIELD_W, value, "", 1.0, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                if let Ok(n) = text.trim().parse::<f64>() {
                    cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": n })) });
                }
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    /// 颜色 alpha 的百分比域（通用 `number_field`，0..100）。
    fn alpha_value_field(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        cx: &mut App,
        key: &str,
        alpha: u8,
        color: Color32,
        vid: &str,
        mid: &str,
    ) -> Entity<TextField> {
        if let Some(e) = caches.borrow().value_editors.get(key) {
            return e.clone();
        }
        let f = number_field(cx, 64.0, Some(alpha as f64), "%", 1.0, Slot::None);
        let cmds = cmds.clone();
        let vid = vid.to_string();
        let mid = mid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let a = text.trim().trim_end_matches('%').trim().parse::<f64>().unwrap_or(alpha as f64).clamp(0.0, 100.0);
                let a8 = (a / 100.0 * 255.0).round() as u8;
                let hex = format!("#{:02X}{:02X}{:02X}{:02X}", color.0[0], color.0[1], color.0[2], a8);
                cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": hex })) });
            });
        });
        caches.borrow_mut().value_editors.insert(key.to_string(), f.clone());
        f
    }

    fn value_cell(
        caches: &Rc<RefCell<RowCaches>>,
        cmds: &Cmds,
        st: &State,
        c: &CollectionSnap,
        r: &VarRow,
        m: &ModeSnap,
        cell: &VarCell,
        cx: &mut App,
    ) -> AnyElement {
        // 'static 回调要持有命令队列的所有权，不能用本帧的引用。
        let cmds = cmds.clone();
        let t = tokens();
        // 只读（库集合 / 继承变量）统一走静态样式 —— 所有类型都在这里收口，
        // 不会再被 Color / Bool 分支截胡成"看起来可编辑"。
        if !c.editable {
            let mut d = Self::value_shell(&t);
            match cell {
                VarCell::Color { color, hex, alpha } => {
                    d = d.child(Self::value_swatch(*color, &t));
                    d = d.child(hex.clone());
                    d = d.child(div().text_color(rgba(t.text_secondary)).child(format!("{alpha}%")));
                }
                VarCell::Bool(b) => d = d.child(if *b { "True" } else { "False" }),
                VarCell::Alias(name) => {
                    d = d.child(paint_icon(Icon::Arrow, rgba(t.text_secondary), 12.0)).child(name.clone());
                }
                VarCell::Empty => {}
                _ => d = d.child(cell.text()),
            }
            return d.into_any_element();
        }
        match cell {
            VarCell::Color { color, hex, alpha } => {
                // 绝对定位：swatch 左 6、hex 从 28 起、alpha 贴右 4。
                // 用 flex 会让两个固定宽输入框互相挤压 / 重叠（"0D" 盖住 "100%"）。
                let hkey = format!("{}/{}:hex", r.id, m.id);
                let akey = format!("{}/{}:alpha", r.id, m.id);
                div()
                    .w_full()
                    .h(pt(24.0))
                    .rounded(pt(5.0))
                    .bg(rgba(t.field))
                    .relative()
                    .overflow_hidden()
                    .child(div().absolute().left(pt(6.0)).top(pt(4.0)).child(Self::value_swatch(*color, &t)))
                    .child(
                        div()
                            .absolute()
                            .left(pt(28.0))
                            .top(pt(0.0))
                            .w(pt(76.0))
                            .h(pt(24.0))
                            .child(Self::text_value_field(caches, &cmds, cx, &hkey, hex, &r.id, &m.id, VarKind::Color, 76.0)),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(pt(4.0))
                            .top(pt(0.0))
                            .w(pt(64.0))
                            .h(pt(24.0))
                            .child(Self::alpha_value_field(caches, &cmds, cx, &akey, *alpha, *color, &r.id, &m.id)),
                    )
                    .into_any_element()
            }
            VarCell::Bool(b) => {
                let on = *b;
                let vid = r.id.clone();
                let mid = m.id.clone();
                let cmds = cmds.clone();
                let mut d = Self::value_shell(&t).cursor_pointer();
                d = d.child(
                    div()
                        .flex_none()
                        .size(pt(14.0))
                        .rounded(pt(3.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .bg(rgba(if on { t.accent } else { t.panel }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if on {
                            paint_icon(Icon::Check, rgba(Color32::from_rgb(0xFF, 0xFF, 0xFF)), 10.0).into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                );
                d = d.child(if on { "True" } else { "False" });
                d.on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                    cmds.push(Cmd { id: "variable.set", params: Some(json!({ "variable": vid, "mode": mid, "value": !on })) });
                })
                .into_any_element()
            }
            VarCell::Alias(name) => {
                // 别名 chip：点击弹出同类型变量列表（+ Detach）。
                let vid = r.id.clone();
                let mid = m.id.clone();
                let kind = r.kind;
                let candidates: Vec<(String, String)> = c
                    .rows
                    .iter()
                    .filter(|x| x.kind == kind && x.id != vid)
                    .map(|x| (x.id.clone(), x.full_name.clone()))
                    .collect();
                let mut d = Self::value_shell(&t).cursor_pointer();
                d = d.child(
                    div()
                        .flex_1()
                        .min_w(pt(0.0))
                        .h(pt(18.0))
                        .rounded(pt(4.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .flex()
                        .items_center()
                        .gap(pt(4.0))
                        .px(pt(4.0))
                        .child(paint_icon(Icon::Arrow, rgba(t.text_secondary), 12.0))
                        .child(div().flex_1().min_w(pt(0.0)).overflow_hidden().child(name.clone())),
                );
                d.on_mouse_down(MouseButton::Left, {
                    let st = st.clone();
                    move |e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let items: Vec<MenuItem> = candidates
                            .iter()
                            .map(|(id, name)| MenuItem::new(name.clone(), "ui.setAlias", json!({ "variable": vid, "mode": mid, "alias": id })))
                            .collect();
                        st.borrow_mut().menu = Some(Menu { x: f32::from(e.position.x), y: f32::from(e.position.y), items });
                    }
                })
                .into_any_element()
            }
            VarCell::Number(s) => {
                let key = format!("{}/{}:num", r.id, m.id);
                let v = s.trim().parse::<f64>().ok();
                let mut d = Self::value_shell(&t);
                d = d.child(Self::number_value_field(caches, &cmds, cx, &key, v, &r.id, &m.id));
                d.into_any_element()
            }
            VarCell::Text(s) => {
                let key = format!("{}/{}:txt", r.id, m.id);
                let mut d = Self::value_shell(&t);
                d = d.child(Self::text_value_field(caches, &cmds, cx, &key, s, &r.id, &m.id, VarKind::String, VALUE_FIELD_W));
                d.into_any_element()
            }
            VarCell::Empty => Self::value_shell(&t).into_any_element(),
        }
    }

    // ── bottom bar ────────────────────────────────────────────────────────────────────────
    fn bottom_bar(&self, st: &State, coll: &Option<CollectionSnap>) -> Div {
        let t = tokens();
        let Some(c) = coll else { return div().flex_none().w_full().h(pt(BOTTOM_H)).border_t_1().border_color(rgba(t.border)) };
        if !c.editable {
            return div()
                .flex_none()
                .w_full()
                .h(pt(BOTTOM_H))
                .flex()
                .items_center()
                .pl(pt(16.0))
                .border_t_1()
                .border_color(rgba(t.border))
                .text_size(pt(10.0))
                .text_color(rgba(t.text_secondary))
                .child(tr("Library collection (read-only)"));
        }
        // "+ Create variable" 是**整块**可点的按钮（图标 + 文本一起响应），且用 `unique()` 命名，
        // 不会每次都建出同名 "Variable"。
        let taken: Vec<String> = c.rows.iter().map(|r| r.full_name.clone()).collect();
        let name = unique("Variable", |n| taken.iter().any(|x| x == n));
        let cid = c.id.clone();
        let items: Vec<MenuItem> = TYPES
            .iter()
            .map(|(label, k)| MenuItem::new(*label, "variable.create", json!({ "collection": cid, "name": name, "type": k.id(), "value": "0" })))
            .collect();
        div()
            .flex_none()
            .w_full()
            .h(pt(BOTTOM_H))
            .flex()
            .flex_row()
            .items_center()
            .px(pt(16.0))
            .border_t_1()
            .border_color(rgba(t.border))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(4.0))
                    .h(pt(24.0))
                    .px(pt(6.0))
                    .rounded(pt(4.0))
                    .cursor_pointer()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .hover(move |s| s.bg(rgba(t.hover)))
                    .on_mouse_down(MouseButton::Left, {
                        let st = st.clone();
                        move |e: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            st.borrow_mut().menu = Some(Menu {
                                x: f32::from(e.position.x),
                                y: f32::from(e.position.y) - 130.0,
                                items: items.clone(),
                            });
                        }
                    })
                    .child(paint_icon(Icon::Plus, rgba(t.text_secondary), 14.0))
                    .child(tr("Create variable")),
            )
    }

    // ── 内联菜单 ──────────────────────────────────────────────────────────────────────────
    /// 内联菜单。外壳走**通用 `Menu` 控件**（`menu_surface()`：13 圆角 / 8 内边距 / 无边框 /
    /// 标准阴影 / occlude），行高与 `MenuRow` 一致 28pt；只有行的业务回调是变量面板自己的。
    fn menu_overlay(&self, st: &State, origin: Point<Pixels>) -> AnyElement {
        let t = tokens();
        let Some(menu) = st.borrow().menu.clone() else { return div().into_any_element() };
        // `menu.x/y` 是 MouseDown 的**窗口坐标**；这个菜单画在卡片（`relative`）内部，
        // `menu()` 生成的 `.absolute().left/top` 以卡片左上角为原点，故需减掉卡片原点。
        let local_x = menu.x - f32::from(origin.x);
        let local_y = menu.y - f32::from(origin.y);
        let cmds = self.cmds.clone();
        let state = st.clone();
        let data = self.data.clone();
        let mut col = div().flex().flex_col();
        for item in menu.items {
            let label = item.label.clone();
            let cmd = item.cmd;
            let params = item.params.clone();
            let cmds = cmds.clone();
            let state = state.clone();
            let data = data.clone();
            col = col.child(
                div()
                    .flex_none()
                    .w_full()
                    .h(pt(28.0))
                    .flex()
                    .items_center()
                    .px(pt(8.0))
                    .rounded(pt(5.0))
                    .cursor_pointer()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.menu_text))
                    .hover(move |s| s.bg(rgba(t.menu_hover)))
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                        state.borrow_mut().menu = None;
                        match cmd {
                            "ui.filter" => {
                                if let Some(i) = params["kind"].as_u64() {
                                    let k = (i as usize).min(3);
                                    let mut s = state.borrow_mut();
                                    let on = s.hidden[k];
                                    s.hidden[k] = !on;
                                }
                            }
                            "ui.showAll" => state.borrow_mut().hidden = [false; 4],
                            "ui.rename" => {
                                if let Some(v) = params["variable"].as_str() {
                                    state.borrow_mut().rename = Some(v.to_string());
                                }
                            }
                            "ui.renameMode" => {
                                if let Some(v) = params["mode"].as_str() {
                                    state.borrow_mut().rename_mode = Some(v.to_string());
                                }
                            }
                            "ui.renameCollection" => {
                                if let Some(v) = params["collection"].as_str() {
                                    state.borrow_mut().rename_collection = Some(v.to_string());
                                }
                            }
                            "ui.renameGroup" => {
                                if let Some(v) = params["group"].as_str() {
                                    state.borrow_mut().rename_group = Some(v.to_string());
                                }
                            }
                            "ui.details" => {
                                if let Some(v) = params["variable"].as_str() {
                                    state.borrow_mut().details = Some(v.to_string());
                                }
                            }
                            "ui.export" => Self::export_dialog(&cmds, params["format"].as_str().unwrap_or("json")),
                            "ui.import" => Self::import_dialog(&cmds),
                            "ui.setAlias" => {
                                if let (Some(v), Some(m), Some(a)) =
                                    (params["variable"].as_str(), params["mode"].as_str(), params["alias"].as_str())
                                {
                                    cmds.push(Cmd {
                                        id: "variable.set",
                                        params: Some(json!({ "variable": v, "mode": m, "value": { "alias": a } })),
                                    });
                                }
                            }
                            "ui.copy" => Self::copy_row(&state, &data),
                            "ui.paste" => Self::paste_row(&state, &data, &cmds),
                            "ui.copyColumn" => {
                                if let Some(m) = params["mode"].as_str() {
                                    Self::copy_column(&state, &data, m);
                                }
                            }
                            "ui.pasteColumn" => {
                                if let Some(m) = params["mode"].as_str() {
                                    Self::paste_column(&state, &cmds, m);
                                }
                            }
                            "ui.deleteGroup" => {
                                let ids: Vec<String> = params["variables"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                                for id in ids {
                                    cmds.push(Cmd { id: "variable.delete", params: Some(json!({ "variable": id })) });
                                }
                            }
                            _ => cmds.push(Cmd { id: cmd, params: Some(params.clone()) }),
                        }
                    })
                    .child(label),
            );
        }
        crate::widgets::menu(crate::widgets::at_point(local_x, local_y.max(4.0)))
            .gap(0.0)
            .width(crate::widgets::Width::Min(180.0))
            .show(|| col.into_any_element())
            .unwrap_or_else(|| div().into_any_element())
    }

    // ── 拖拽 ──────────────────────────────────────────────────────────────────────────────

    /// 用按下点作为命中行的参考，重建整列的行矩形（`drag::calibrate` 的同构做法）。
    /// 平面列表 `can_into = false`，所以 `resolve` 只会在 Above / Below 之间选，`gap` 即插入缝。
    fn begin_drag(kind: DragList, from: usize, ids: &[String], hit: Point<Pixels>, tops: &[f32]) -> ListDragState {
        let h = match kind {
            DragList::Variables => ROW_H,
            _ => SIDE_ROW_H,
        };
        // 变量表带分组间隔 / 分组标题，行不是等距的：用 `tops`（内容坐标里的真实顶边）
        // 反推整列矩形。指针按下点的窗口 y 减去按下行的内容 y，就得到内容原点的窗口 y；
        // 纵向滚动也因此自动抵消。侧栏两个列表是等距平铺，`tops` 为空，走原公式。
        let from_top = tops.get(from).copied().unwrap_or(from as f32 * h);
        let base_y = f32::from(hit.y) - from_top;
        let rows: Vec<DragRow> = ids
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let y = match tops.get(i) {
                    Some(t) => base_y + *t,
                    None => f32::from(hit.y) - from as f32 * h + i as f32 * h,
                };
                DragRow {
                    id: i as u64,
                    path: i.to_string(),
                    can_into: false,
                    collapsed: false,
                    rect: Bounds { origin: Point { x: px(0.0), y: px(y) }, size: size(px(10000.0), px(h)) },
                }
            })
            .collect();
        ListDragState { kind, from, ids: ids.to_vec(), rows, pointer: hit, target: None, armed: false }
    }

    fn on_move(st: &State, pointer: Point<Pixels>) {
        let mut s = st.borrow_mut();
        // 横向滚动条：按住滑块（或轨道）拖动，按行程把指针位移换算成滚动量。
        if let Some((x0, off0, range, max_scroll)) = s.hscroll_drag {
            let target = (off0 + (f32::from(pointer.x) - x0) / range.max(1.0) * max_scroll).clamp(0.0, max_scroll);
            s.hscroll.set_offset(point(px(-target), px(0.0)));
        }
        if let Some((_, _, cur)) = s.mode_drag.as_mut() {
            *cur = f32::from(pointer.x);
        }
        if let Some((x0, w0)) = s.col_resize {
            s.name_w = (w0 + f32::from(pointer.x) - x0).clamp(NAME_MIN, NAME_MAX);
        }
        if let Some((x0, w0)) = s.sidebar_resize {
            s.sidebar_w = (w0 + f32::from(pointer.x) - x0).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        if let Some((y0, split0)) = s.split_resize {
            // 上下分区的比例：以 ~400pt 的可用高度折算，夹在 5%..95%。
            s.sidebar_split = (split0 + (f32::from(pointer.y) - y0) / 400.0).clamp(0.05, 0.95);
        }
        if let Some((dir, x0, y0, w0, h0)) = s.resize {
            let dx = f32::from(pointer.x) - x0;
            let dy = f32::from(pointer.y) - y0;
            let mut w = w0;
            let mut h = h0;
            if dir & 1 != 0 {
                w = w0 + dx;
            }
            if dir & 2 != 0 {
                w = w0 - dx;
            }
            if dir & 4 != 0 {
                h = h0 + dy;
            }
            if dir & 8 != 0 {
                h = h0 - dy;
            }
            s.size = Some((w.clamp(480.0, 1600.0), h.clamp(300.0, 1200.0)));
        }
        if let Some((i, x0, w0)) = s.mode_resize {
            if let Some(w) = s.mode_ws.get_mut(i) {
                *w = (w0 + f32::from(pointer.x) - x0).clamp(100.0, 420.0);
            }
        }
        let Some(d) = s.drag.as_mut() else { return };
        let dx = (f32::from(pointer.x) - f32::from(d.pointer.x)).abs();
        let dy = (f32::from(pointer.y) - f32::from(d.pointer.y)).abs();
        if !d.armed && dx + dy < 4.0 {
            return;
        }
        d.armed = true;
        d.pointer = pointer;
        d.target = drag::resolve(&d.rows, d.from, pointer, DragCfg::default());
    }

    fn commit_drag(st: &State, d: &ListDragState, cmds: &Cmds) {
        let Some(t) = d.target.as_ref() else { return };
        if t.refused || t.noop || !d.armed {
            return;
        }
        let gap = t.gap.min(d.ids.len());
        let before = d.ids.get(gap).cloned();
        match d.kind {
            DragList::Collections => {
                let from = d.ids.get(d.from).cloned().unwrap_or_default();
                let mut p = json!({ "collection": from });
                if let Some(b) = before {
                    p["before"] = json!(b);
                }
                cmds.push(Cmd { id: "variable.moveCollection", params: Some(p) });
            }
            DragList::Variables => {
                let from = d.ids.get(d.from).cloned().unwrap_or_default();
                let mut p = json!({ "variable": from });
                if let Some(b) = before {
                    p["before"] = json!(b);
                }
                cmds.push(Cmd { id: "variable.move", params: Some(p) });
            }
            DragList::Groups => {
                // egui 的 group_order 是会话视图偏好，不进文档：只重排本地顺序。
                let mut s = st.borrow_mut();
                if let Some(name) = d.ids.get(d.from).cloned() {
                    s.group_order.retain(|g| g != &name);
                    let at = before.and_then(|b| s.group_order.iter().position(|g| g == &b)).unwrap_or(usize::MAX);
                    let len = s.group_order.len();
                    s.group_order.insert(at.min(len), name);
                }
            }
        }
    }

    // ── Edit variable 详情面板 ───────────────────────────────────────────────────────────
    /// 一个详情字段（Name / Description）：commit 时推 `variable.update {field: text}`。
    fn details_field(&mut self, cx: &mut App, key: &str, initial: &str, vid: &str, field: &'static str) -> Entity<TextField> {
        if let Some(e) = self.details_fields.get(key) {
            return e.clone();
        }
        let f = text_field(cx, 260.0, initial, "", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let vid = vid.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let mut p = json!({ "variable": vid });
                p[field] = json!(text);
                cmds.push(Cmd { id: "variable.update", params: Some(p) });
            });
        });
        self.details_fields.insert(key.to_string(), f.clone());
        f
    }

    /// 代码名（WEB / iOS / ANDROID）：`variable.update {codeSyntax: {platform: text}}`。
    fn details_code_field(&mut self, cx: &mut App, key: &str, initial: &str, vid: &str, platform: &str) -> Entity<TextField> {
        if let Some(e) = self.details_fields.get(key) {
            return e.clone();
        }
        let f = text_field(cx, 180.0, initial, "", Slot::None, Slot::None);
        let cmds = self.cmds.clone();
        let vid = vid.to_string();
        let platform = platform.to_string();
        f.update(cx, move |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let mut code = serde_json::Map::new();
                code.insert(platform.clone(), json!(text));
                let mut p = json!({ "variable": vid });
                p["codeSyntax"] = Value::Object(code);
                cmds.push(Cmd { id: "variable.update", params: Some(p) });
            });
        });
        self.details_fields.insert(key.to_string(), f.clone());
        f
    }

    /// 变量详情（egui `details()`）。外壳用**通用 `Popover`**（40pt 标题栏 / 13 圆角 / 标准阴影 /
    /// 统一 X），不再手搓 absolute 卡片。Name 用完整名，改名才不会丢掉分组前缀。
    fn details_overlay(&mut self, st: &State, cx: &mut App, viewport: Size<Pixels>) -> AnyElement {
        let t = tokens();
        // egui：库变量（只读）不显示详情。
        if st.borrow().library.is_some() {
            st.borrow_mut().details = None;
            return div().into_any_element();
        }
        let Some(id) = st.borrow().details.clone() else { return div().into_any_element() };
        let Some(row) = self.data.collections.iter().flat_map(|c| c.rows.iter()).find(|r| r.id == id).cloned() else {
            st.borrow_mut().details = None;
            return div().into_any_element();
        };
        let mut body = div().w_full().flex().flex_col().gap(pt(6.0));
        body = body.child(detail_label("Name"));
        body = body.child(self.details_field(cx, &format!("d-name-{}", row.id), &row.full_name, &row.id, "name"));
        body = body.child(detail_label("Description"));
        body = body.child(self.details_field(cx, &format!("d-desc-{}", row.id), &row.description, &row.id, "description"));
        // Hide from publishing
        let hidden = row.hidden;
        body = body.child(
            div()
                .w_full()
                .h(pt(24.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .size(pt(14.0))
                        .rounded(pt(3.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .bg(rgba(if hidden { t.accent } else { t.panel })),
                )
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Hide from publishing")))
                .on_mouse_down(MouseButton::Left, {
                    let cmds = self.cmds.clone();
                    let vid = row.id.clone();
                    move |_: &MouseDownEvent, _, _| {
                        cmds.push(Cmd {
                            id: "variable.update",
                            params: Some(json!({ "variable": vid, "hiddenFromPublishing": !hidden })),
                        });
                    }
                }),
        );
        body = body.child(detail_label("Code syntax"));
        for (k, v) in row.code.clone() {
            let key = format!("d-code-{}-{}", row.id, k);
            body = body.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(6.0))
                    .child(div().flex_none().w(pt(60.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(k.clone()))
                    .child(self.details_code_field(cx, &key, &v, &row.id, &k)),
            );
        }
        // Scoping：egui `details()` 的 "All supported properties" + 逐项范围。
        body = body.child(detail_label("Scoping: where pickers offer it"));
        let all_scopes = row.scopes.is_none();
        body = body.child(
            div()
                .w_full()
                .h(pt(24.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(6.0))
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .size(pt(14.0))
                        .rounded(pt(3.0))
                        .border_1()
                        .border_color(rgba(t.border_strong))
                        .bg(rgba(if all_scopes { t.accent } else { t.panel })),
                )
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("All supported properties")))
                .on_mouse_down(MouseButton::Left, {
                    let cmds = self.cmds.clone();
                    let vid = row.id.clone();
                    move |_: &MouseDownEvent, _, _| {
                        cmds.push(Cmd {
                            id: "variable.update",
                            params: Some(json!({ "variable": vid, "scopes": if all_scopes { Value::Null } else { json!([]) } })),
                        });
                    }
                }),
        );
        for scope in row.scopes.clone().unwrap_or_default() {
            body = body.child(
                div()
                    .pl(pt(16.0))
                    .h(pt(22.0))
                    .flex()
                    .items_center()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(scope),
            );
        }
        let close_st = st.clone();
        let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_, _| close_st.borrow_mut().details = None);
        popover_window(PopoverOpen::At(point(px(300.0), px(60.0))))
            .width(280.0)
            .pad(16.0)
            .surface(Surface::Panel)
            .title(title_text("Edit variable"))
            .body(body)
            .show(viewport, dismiss)
            .1
    }

    fn drag_line(&self, st: &State, root_bounds: Option<Bounds<Pixels>>) -> Option<AnyElement> {
        let t = tokens();
        let s = st.borrow();
        let d = s.drag.as_ref()?;
        let tg = d.target.as_ref()?;
        if tg.refused || tg.noop || !d.armed {
            return None;
        }
        // 落点几何是**窗口坐标**（行矩形由按下点推出来），而插入线要挂在 root 里：
        // 减去 root 的窗口原点才是局部坐标。x 还要加上侧栏宽度（表格不在 root 左缘）。
        let rb = root_bounds?;
        let ox = f32::from(rb.origin.x);
        let oy = f32::from(rb.origin.y);
        let w = f32::from(rb.size.width);
        let h = f32::from(rb.size.height);
        let table_left = if s.sidebar { s.sidebar_w + 1.0 } else { 0.0 };
        let x0 = table_left + tg.line.x0;
        let x1 = (table_left + tg.line.x1).min(w);
        let y = tg.line.y - oy;
        if y < 0.0 || y > h || x1 <= x0 {
            return None;
        }
        let _ = ox;
        Some(
            div()
                .absolute()
                .left(pt(x0))
                .top(pt(y))
                .w(pt((x1 - x0).max(40.0)))
                .h(pt(2.0))
                .bg(rgba(t.accent))
                .into_any_element(),
        )
    }
}

/// egui `unique()`：重名时加 ` 2`、` 3`…（新建集合 / 变量 / mode 都用它）。
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    for i in 2..10000 {
        let s = format!("{base} {i}");
        if !taken(&s) {
            return s;
        }
    }
    base.to_string()
}

/// 详情面板的小标题。
fn detail_label(text: &'static str) -> Div {
    let t = tokens();
    div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(text)
}

/// 剪贴板里的值 → `variable.set` 的 value（egui `value_json` 的等价）。
fn cell_value(c: &VarCell) -> Option<Value> {
    match c {
        VarCell::Color { hex, .. } => Some(json!(format!("#{hex}"))),
        VarCell::Number(s) => s.trim().parse::<f64>().ok().map(|n| json!(n)),
        VarCell::Text(s) => Some(json!(s)),
        VarCell::Bool(b) => Some(json!(b)),
        VarCell::Alias(name) => Some(json!({ "alias": name })),
        VarCell::Empty => None,
    }
}

/// 输入框文本 → `variable.set` 的 value（egui `value_json` / `var_value` 的等价）。
fn encode_value(kind: VarKind, text: &str) -> Value {
    match kind {
        VarKind::Number => text.trim().parse::<f64>().map(|n| json!(n)).unwrap_or_else(|_| json!(0.0)),
        VarKind::Boolean => json!(text.trim().eq_ignore_ascii_case("true")),
        // hex 统一成 `#RRGGBB` / `#RRGGBBAA`（用户可能带或不带 `#`）。
        VarKind::Color => json!(format!("#{}", text.trim().trim_start_matches('#'))),
        VarKind::String => json!(text.trim()),
    }
}

/// egui `kind_glyph`：Color = 色块、Number = `#`、String = Text 图标、Boolean = 小开关。
fn kind_glyph(r: &VarRow) -> AnyElement {
    let t = tokens();
    match r.kind {
        VarKind::Color => {
            let c = r
                .values
                .iter()
                .find_map(|v| match v {
                    VarCell::Color { color, .. } => Some(*color),
                    _ => None,
                })
                .unwrap_or(Color32::GRAY);
            div().flex_none().size(pt(14.0)).rounded(pt(3.0)).bg(rgba(c)).border_1().border_color(rgba(t.border)).into_any_element()
        }
        VarKind::Number => div()
            .flex_none()
            .size(pt(14.0))
            .flex()
            .items_center()
            .justify_center()
            .text_size(pt(12.0))
            .text_color(rgba(t.text_secondary))
            .child("#")
            .into_any_element(),
        VarKind::String => paint_icon(Icon::Text, rgba(t.text_secondary), 14.0).into_any_element(),
        VarKind::Boolean => div()
            .flex_none()
            .w(pt(14.0))
            .h(pt(8.0))
            .rounded(pt(4.0))
            .border_1()
            .border_color(rgba(t.text_secondary))
            .into_any_element(),
    }
}
