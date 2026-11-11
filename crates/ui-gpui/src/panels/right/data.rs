//! 右栏列表数据的快照（selection → panel）。
//!
//! 权威来源：`crates/ui-egui/src/panels/right/{paints,stroke,effects}.rs`、
//! `panels/{export_panel,selection_colors,section_panel,layout_panel}.rs`。
//!
//! egui 的面板每帧直接读 `Document`；GPUI 是 retained tree，`right_panel` 拿不到 doc，所以
//! 由 `CanvasView::panel_data()` 每帧把这些列表读成快照传给 `right_panel`。所有字段都是已经
//! 格式化好的 `String`（或 `Paint`），面板只负责用通用组件画出来 + 推命令，不再碰 doc。

use figcraft_color::Paint;
use figcraft_doc::{
    styles::StyleKind,
    text::trim_num,
    Document, NodeId, NodeKind,
};
use figcraft_i18n::tr;
use figcraft_i18n::tr_noop;
use serde_json::Value;

/// 面板里最多画多少条网格轨道（egui `layout_panel.rs::MAX_TRACK_ROWS` 同为 24）。
const GRID_TRACK_ROWS: usize = 24;

/// 一个 fill/stroke paint 行（egui `paints.rs::paint_row`）。
#[derive(Clone, Debug, PartialEq)]
pub struct PaintRow {
    pub index: usize,
    pub fills: bool,
    /// paint 本体（`ColorField` 画 swatch 用）。
    pub paint: figcraft_color::Paint,
    /// 纯色显示 hex；渐变/图片显示名字。
    pub text_is_hex: bool,
    pub text: String,
    /// 0–100 的百分数。
    pub opacity: f64,
    pub visible: bool,
    /// paint 颜色绑定的变量名（`Some` 时显示 chip + detach，egui `bound_paint`）。
    pub bound: Option<String>,
    /// `variable.bind` 用的 field key（`fill` / `fills/<i>/color`）。
    pub bind_field: String,
    /// 该字段可绑定的颜色变量名列表（点绑定按钮时弹出的菜单）。
    pub variables: Vec<String>,
}

/// 一个 effect 行（egui `effects.rs::effects`）。
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRow {
    pub index: usize,
    pub label: String,
    pub ty: &'static str,
    pub visible: bool,
    /// 该 effect 的原始 JSON（改数值字段时 clone 后改一个键，推 `effect.set`）。
    pub effect: serde_json::Value,
}

/// 选中节点的 WGSL 材质绑定快照（存在 `plugin_data` 的保留 key，非 `EffectKind`）。
///
/// `path` 为 `None` 表示该节点没有绑定材质；面板因此只在有绑定时才展开参数。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WgslPanelData {
    /// 已绑定的材质文件路径（来自 plugin_data）。
    pub path: Option<String>,
    /// 用户当前的参数值（uniform 路径 → 0..100 刻度的值）。
    pub params: std::collections::BTreeMap<String, f64>,
}

/// 一个 layout grid 行（egui `effects.rs::grids`）。
#[derive(Clone, Debug, PartialEq)]
pub struct GridRow {
    pub index: usize,
    pub label: String,
    pub color: String,
    pub visible: bool,
    /// 整个 grids 数组的 JSON（改类型 / 颜色时替换其中一项，推 `node.set {"grids": …}`）。
    pub grids: serde_json::Value,
}

/// 一个 export setting 行（egui `export_panel.rs::setting_row`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRow {
    pub index: usize,
    pub size: String,
    pub suffix: String,
    pub format: &'static str,
    pub format_label: &'static str,
    pub is_raster: bool,
    /// Dots 弹层（`export_panel.rs::advanced_options` / `print_options`）的当前值，
    /// 用来把菜单项做成「开 / 关」切换，而不是无条件写同一个布尔值。
    pub contents_only: bool,
    pub include_bounding_box: bool,
    pub svg_outline_text: bool,
    pub svg_id_attribute: bool,
    pub svg_simplify_stroke: bool,
    pub vd_compat: &'static str,
    pub bleed: f64,
    pub crop_marks: bool,
    pub bleed_marks: bool,
    pub cmyk: bool,
}

/// 一个 selection color 组（egui `selection_colors.rs::section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionColorRow {
    /// 纯色是 hex，渐变/图片是 `paint_name`（egui 用 dropdown 显示名字）。
    pub hex: String,
    pub solid: bool,
    pub opacity: f64,
    /// 绑定的样式名或变量名（`Some` 时这一行显示 chip + detach，egui `linked_name`）。
    pub linked: Option<String>,
    /// `selection.setPaint` / `selection.selectColor` 用的 targets JSON。由引擎的
    /// `PaintTarget` 序列化而来，自带文本范围的 `range`。
    pub targets: serde_json::Value,
    /// `(显示名, 变量 id)`：这一组**可绑定**的颜色变量。egui `section` 会用
    /// `bind_refusal` / `in_scope` 过滤，否则菜单里点到的变量会被引擎直接拒绝。
    pub variables: Vec<(String, String)>,
    /// 该组原本的完整 Paint（egui `group.paint.clone()`）：改颜色或改透明度都在它上面
    /// 改一个字段再整体提交；`selection.setPaint` 只认 `paint`，顶层 `color`/`opacity`
    /// 会被忽略，直接传 `"#RRGGBB"` 还会把透明度重置成 1。
    pub paint: serde_json::Value,
}

/// Auto layout 的尺寸（egui `layout_panel.rs::dimensions` / `dim_field`）。
/// W/H 数值走通用 `number_field`；sizing 是 Fixed/Hug/Fill 下拉。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DimensionsData {
    pub w: Option<f64>,
    pub h: Option<f64>,
    /// `sizingW` / `sizingH` 的 serde 名（`fixed` / `hug` / `fill`）；`None` = Mixed。
    pub sizing_w: Option<&'static str>,
    pub sizing_h: Option<&'static str>,
    /// 约束比例（`constrainProportions`）。
    pub constrain: bool,
    pub min_w: Option<f64>,
    pub max_w: Option<f64>,
    pub min_h: Option<f64>,
    pub max_h: Option<f64>,
}

/// 选中文本节点的排版快照（egui `typography/fields.rs`）。
///
/// 只在这些节点**全部是文本**时才 `Some`；否则 `None`（右栏不画 Typography 节）。
/// 行高 / 字距的 `None` 表示「Auto」（或 Mixed，v1 两者都显示为空，提交时回写 `"AUTO"`）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextData {
    pub font_family: Option<String>,
    pub font_style: Option<String>,
    /// 字号（px）。
    pub font_size: Option<f64>,
    /// 行高：`(值, 是否百分比)`；`None` = Auto。
    pub line_height: Option<(f64, bool)>,
    /// 字距：`(值, 是否百分比)`；`None` = Auto。
    pub letter_spacing: Option<(f64, bool)>,
    /// 段落间距（px）。
    pub paragraph_spacing: Option<f64>,
    /// 自动缩放：`widthAndHeight` / `height` / `none` / `truncate`；`None` = Mixed。
    pub auto_resize: Option<&'static str>,
    /// 水平对齐：`left` / `center` / `right` / `justified`；`None` = Mixed。
    pub align_h: Option<&'static str>,
    /// 垂直对齐：`top` / `middle` / `bottom`；`None` = Mixed。
    pub align_v: Option<&'static str>,
    // ── Type settings 弹层（egui `typography.rs::type_settings_basics` / `text_style_settings`）──
    // 这些属性没有常驻行，只在「Type settings」窗口里出现，所以跟着快照一起读出来。
    /// 装饰线：`none` / `underline` / `strikethrough`；`None` = Mixed。
    pub decoration: Option<&'static str>,
    /// 装饰线样式：`solid` / `dotted` / `wavy`；`None` = Mixed。
    pub decoration_style: Option<&'static str>,
    /// 装饰线粗细（px）；`None` = Auto（跟字号走）或 Mixed。
    pub decoration_thickness: Option<f64>,
    /// 下划线在字母下伸部处断开。
    pub decoration_skip_ink: Option<bool>,
    /// 大小写：`original` / `upper` / `lower` / `title` / `smallCaps`；`None` = Mixed。
    pub case: Option<&'static str>,
    /// 垂直裁切：`none` / `capHeight`；`None` = Mixed。
    pub leading_trim: Option<&'static str>,
    /// 列表：`none` / `bulleted` / `numbered`；`None` = Mixed。
    pub list_style: Option<&'static str>,
    /// 段落缩进（px）。
    pub paragraph_indent: Option<f64>,
    /// 标点悬挂（开/关）。
    pub hanging_punctuation: Option<bool>,
    /// 列表悬挂（开/关）。
    pub hanging_list: Option<bool>,
    /// 换行样式：`auto` / `balance` / `pretty`；`None` = Mixed。驱动 Basics 页 Wrap style 下拉。
    pub wrap_style: Option<&'static str>,
    /// 最大行数（0 = 不截断）。
    pub max_lines: Option<u32>,
    /// OpenType 特性覆盖（tag → 开/关）；没有覆盖的 tag 不在表里。驱动 Details 页各开关。
    pub features: std::collections::BTreeMap<String, bool>,
    /// 当前字体字面**真实提供**的 GSUB feature tags（egui `face.has_feature` / 引擎
    /// `text.fonts {features:true}` 的同一份能力，走 `figcraft_text::find_face` 同步读取）。
    /// Details 页据此决定每个 feature 开关是可用还是置灰；`kern`/`cpsp` 例外恒可用。
    pub features_supported: std::collections::BTreeSet<String>,
    /// 可变字体的轴（只有单一字体族的可变字面才有）：驱动 Type settings 的 Variable 页。
    pub axes: Vec<AxisRow>,
}

/// 可变字体的一条轴（`figcraft_text::Axis` + 当前值），供 Type settings 的 Variable 页使用。
#[derive(Clone, Debug, PartialEq)]
pub struct AxisRow {
    pub tag: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub value: f64,
}

/// 矢量编辑模式下选中点的快照（egui `vector_panel.rs::points`）。
///
/// 只在该面板真的会画（`vector_edit` 存在）时 `Some`；`count` 为 0 时面板显示提示行。
#[derive(Clone, Debug, PartialEq)]
pub struct VectorData {
    /// 选中的顶点数。
    pub count: usize,
    /// 是否选了顶点**或线段**（egui `points_selected`）：决定 Position / Auto layout / Arc / Star
    /// 是否让位给点面板。
    pub has_selection: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// 仅当选中 > 1 个点时有点的包围盒。
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub radius: Option<f64>,
    /// `None` = Mixed。
    pub mirroring: Option<figcraft_doc::HandleMirroring>,
    /// `None` = Mixed；`Some(None)` = 跟图层一致。
    pub cap: Option<Option<figcraft_doc::StrokeCap>>,
    pub join: Option<Option<figcraft_doc::StrokeJoin>>,
    /// Variable width 工具选中的宽度点（`ve.tool == Width` 时右栏只画 Width point 节）。
    pub width_point: Option<WidthPointData>,
}

/// Variable width 的宽度点（egui `vector_panel.rs::width_point`）。
#[derive(Clone, Debug, PartialEq)]
pub struct WidthPointData {
    /// 点在路径上的位置（0..1）。
    pub t: f64,
    /// 该点的宽度（已乘 stroke.weight）。
    pub width: f64,
    /// 左右两侧各自的宽度。
    pub left: f64,
    pub right: f64,
    /// 该点在 profile 里的下标（提交命令用）。
    pub index: usize,
}

/// 一条 flow 起点（egui `proto_panel.rs::flow_row` 的输入）。
#[derive(Clone, Debug, PartialEq)]
pub struct FlowRow {
    pub start: NodeId,
    pub name: String,
    pub description: String,
}

/// Prototype settings 的快照（egui `proto_panel.rs::settings`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ProtoSettingsData {
    pub device: String,
    /// `PrototypeSettings::DEVICES` 的名字，外加 "None" / "Custom size"。
    pub devices: Vec<&'static str>,
    /// 该 device 能横竖旋转（手机 / 平板）。
    pub rotates: bool,
    pub landscape: bool,
    /// 当前 device 的机身色（设备没有颜色时为空）。
    pub color: String,
    pub colors: Vec<(&'static str, [u8; 3])>,
    /// 自定义尺寸（`Some` 时才画 Size 行）。
    pub custom: Option<(f64, f64)>,
    pub responsive: bool,
    pub background: String,
}

/// 交互列表的一行（egui `proto_panel.rs::interactions`）：触发器 + 动作摘要，
/// 外加展开编辑所需的原始值（`proto.interaction.set` 直接提交这些 JSON）。
#[derive(Clone, Debug, PartialEq)]
pub struct InteractionRow {
    pub trigger: String,
    pub summary: String,
    /// trigger 的 serde 名（`onClick` / `afterDelay` / `keyPress` …）。
    pub trigger_kind: String,
    /// 完整 trigger（含 delay 的 ms、按键的 key）。
    pub trigger_json: Value,
    pub delay_ms: u32,
    /// transition / easing 的显示名与完整 JSON。
    pub transition: String,
    pub transition_json: Value,
    pub easing: String,
    pub easing_json: Value,
    pub duration_ms: u32,
    /// 每条动作的摘要与完整 JSON。
    pub actions: Vec<(String, Value)>,
}

/// Scroll behavior 节（egui `proto_panel.rs::scroll_section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollData {
    /// 父级是 frame-like（画 Position 行）。
    pub in_frame: bool,
    /// 自己是 frame-like（画 Overflow 行）。
    pub is_frame: bool,
    /// `scrolls` / `fixed` / `sticky`。
    pub position: &'static str,
    /// `none` / `horizontal` / `vertical` / `both`。
    pub overflow: &'static str,
}

/// Prototype 面板的全部数据（无选中 = flows + settings；有选中 = flow / interactions / scroll）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProtoData {
    pub flows: Vec<FlowRow>,
    pub settings: ProtoSettingsData,
    /// `Some(inner)` = 选中节点是顶层 frame；`inner` 是它的 flow 行（还没有起点时 `None`）。
    pub frame_flow: Option<Option<FlowRow>>,
    /// `None` = 多选且交互不一致（Mixed）。
    pub interactions: Option<Vec<InteractionRow>>,
    /// 本页顶层 frame（egui `proto_panel.rs::frames_for`）：新交互的默认跳转目标从这里挑。
    pub frames: Vec<(NodeId, String)>,
    pub scroll: Option<ScrollData>,
}

impl Default for ProtoSettingsData {
    fn default() -> Self {
        Self {
            device: tr("None").into(),
            devices: Vec::new(),
            rotates: false,
            landscape: false,
            color: String::new(),
            colors: Vec::new(),
            custom: None,
            responsive: false,
            background: String::new(),
        }
    }
}

/// 读 Prototype 面板的数据（egui `proto_panel.rs::prototype` 的三段）。
pub fn read_proto(doc: &Document, ids: &[NodeId], page: Option<NodeId>) -> ProtoData {
    use figcraft_doc::proto::{Action, PrototypeSettings};
    let mut p = ProtoData::default();
    let page_kind = page.and_then(|pg| doc.node(pg)).map(|n| &n.kind);
    let (flows, settings) = match page_kind {
        Some(figcraft_doc::NodeKind::Page { flows, prototype, .. }) => (flows.clone(), prototype.clone()),
        _ => (Vec::new(), PrototypeSettings::default()),
    };
    let flow_row = |f: &figcraft_doc::proto::Flow| FlowRow { start: f.start, name: f.name.clone(), description: f.description.clone() };
    p.flows = flows.iter().map(flow_row).collect();
    if let Some(pg) = page {
        p.frames = doc.top_frames(pg).into_iter().map(|id| (id, node_name(doc, id))).collect();
    }

    // ── Prototype settings ──
    let model = settings.model();
    let mut devices: Vec<&'static str> = vec![tr_noop("None")];
    devices.extend(PrototypeSettings::DEVICES.iter().map(|d| d.name));
    devices.push(PrototypeSettings::CUSTOM_DEVICE);
    let custom = (settings.device == PrototypeSettings::CUSTOM_DEVICE).then(|| {
        let d = model.unwrap_or(PrototypeSettings::DEVICES[0]);
        (d.width, d.height)
    });
    p.settings = ProtoSettingsData {
        device: settings.device.clone(),
        devices,
        rotates: model.is_some_and(|d| d.rotates),
        landscape: settings.landscape(),
        color: settings.device_color().map(|c| c.name.to_string()).unwrap_or_default(),
        colors: model.map(|m| m.colors.iter().map(|c| (c.name, c.rgb)).collect()).unwrap_or_default(),
        custom,
        responsive: settings.responsive,
        background: settings.background.to_hex(),
    };

    if ids.is_empty() {
        return p;
    }

    let first = ids.first().copied();
    // ── Flow starting point（顶层 frame 才有）──
    if let Some(id) = first
        && doc.top_level_frame(id) == Some(id)
    {
        let mine = flows.iter().find(|f| f.start == id).map(flow_row);
        p.frame_flow = Some(mine);
    }

    // ── Interactions ──
    let lists: Vec<Option<&Vec<figcraft_doc::proto::Interaction>>> = ids.iter().map(|i| doc.node(*i).map(|n| &n.interactions)).collect();
    let same = lists.first().copied().flatten().filter(|l| lists.iter().all(|x| *x == Some(*l)));
    p.interactions = same.map(|list| {
        list.iter()
            .map(|it| {
                let mut parts: Vec<String> = Vec::new();
                let mut actions: Vec<(String, Value)> = Vec::new();
                for a in &it.actions {
                    let label = match a {
                        Action::Navigate { dest, .. } => format!("Navigate to {}", node_name(doc, *dest)),
                        Action::ScrollTo { dest, .. } => format!("Scroll to {}", node_name(doc, *dest)),
                        Action::OpenOverlay { dest, .. } => format!("Open overlay {}", node_name(doc, *dest)),
                        Action::SwapOverlay { dest } => format!("Swap overlay to {}", node_name(doc, *dest)),
                        Action::ChangeTo { dest } => format!("Change to {}", node_name(doc, *dest)),
                        Action::OpenUrl { url, .. } if !url.is_empty() => format!("Open link {url}"),
                        other => other.label().to_string(),
                    };
                    parts.push(label.clone());
                    actions.push((label, serde_json::to_value(a).unwrap_or(Value::Null)));
                }
                InteractionRow {
                    trigger: it.trigger.label(),
                    summary: parts.join(", "),
                    trigger_kind: serde_json::to_value(&it.trigger)
                        .ok()
                        .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
                        .unwrap_or_else(|| "onClick".into()),
                    trigger_json: serde_json::to_value(&it.trigger).unwrap_or(Value::Null),
                    delay_ms: it.delay_ms,
                    transition: it.transition.label().to_string(),
                    transition_json: serde_json::to_value(&it.transition).unwrap_or(Value::Null),
                    easing: it.easing.label().to_string(),
                    easing_json: serde_json::to_value(&it.easing).unwrap_or(Value::Null),
                    duration_ms: it.duration_ms,
                    actions,
                }
            })
            .collect()
    });

    // ── Scroll behavior ──
    if let Some(n) = first.and_then(|i| doc.node(i)) {
        let in_frame = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.kind.is_frame_like());
        let is_frame = n.kind.is_frame_like();
        if in_frame || is_frame {
            use figcraft_doc::proto::{Overflow, ScrollPosition};
            p.scroll = Some(ScrollData {
                in_frame,
                is_frame,
                position: match n.scroll_position {
                    ScrollPosition::Scrolls => "scrolls",
                    ScrollPosition::Fixed => "fixed",
                    ScrollPosition::Sticky => "sticky",
                },
                overflow: match n.overflow {
                    Overflow::None => "none",
                    Overflow::Horizontal => "horizontal",
                    Overflow::Vertical => "vertical",
                    Overflow::Both => "both",
                },
            });
        }
    }
    p
}

/// Inspect 标签的一行属性（egui `right/inspect.rs::prop_row`）。
#[derive(Clone, Debug, PartialEq)]
pub struct InspectRow {
    pub label: String,
    pub value: String,
    /// 颜色值的十六进制（有值时画色块）。
    pub color: Option<String>,
    /// 这一行来自哪个样式 / 变量（右端 chip）。
    pub token: Option<String>,
}

/// Inspect 标签的一组属性（egui `inspect_section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct InspectSection {
    pub title: &'static str,
    pub rows: Vec<InspectRow>,
}

/// 「Connect to code」已连上时的填写内容（egui `code_connect::CodeLink`）。
#[derive(Clone, Debug, PartialEq)]
pub struct CodeLinkRow {
    pub import_path: String,
    pub name: String,
    pub template: String,
}

/// Inspect 标签里的一项可导出资产（egui `devinspect::Asset` + `inspect_assets` 的行）。
///
/// 缩略图不是 UI 自己光栅化的：`asset_thumb` 调 `figcraft_render::render_node`（和 egui
/// `assets_panel::thumbnail_pixmap` 同一个渲染 crate），这里只把 `Pixmap` 转成
/// GPUI 纹理要的 BGRA 字节；`inspect.rs` 用 `render_worker::wrap_bgra` 包一下显示。
#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    pub id: NodeId,
    pub name: String,
    /// 已有导出设置的摘要（"PNG 2x"…），格式下拉默认取第一条。
    pub exports: Vec<String>,
    /// 该资产的第一条导出设置对应的格式下标（`ExportFormat::ALL`）。
    pub format_index: usize,
    /// `(宽, 高, BGRA 字节)`；渲染不出来时为空。
    pub thumb: Option<(u32, u32, Vec<u8>)>,
}

/// Inspect 标签（Dev Mode 的手递手信息：属性 + 代码）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct InspectData {
    pub sections: Vec<InspectSection>,
    /// 生成的代码（按当前语言）。
    pub code: String,
    /// 语言 display 名。
    pub lang_label: &'static str,
    /// 当前语言 id（下拉选中用）。
    pub lang: &'static str,
    /// 可选语言（label, id）。
    pub langs: Vec<(&'static str, &'static str)>,
    /// 该语言是 Web（才给 px / rem 切换）。
    pub web: bool,
    /// Color format 的显示名与 id。
    pub fmt_label: &'static str,
    pub fmt: &'static str,
    pub formats: Vec<(&'static str, &'static str)>,
    /// 选中图层里可导出的资产。
    pub assets: Vec<AssetRow>,
    /// 「Connect to code」：可连的图层（组件 / 组件集 / 实例）与已连的内容。
    pub connect_owner: Option<NodeId>,
    pub connectable: bool,
    pub link: Option<CodeLinkRow>,
}

/// 缩略图缓存（egui `AssetUi::thumbs` 按 `ds.revision` 失效）。GPUI 的 `read_inspect` 每帧都跑，
/// 不缓存就会把每个资产反复光栅化；这里按 `(revision, id, 目标边长)` 存 BGRA 字节。
thread_local! {
    static ASSET_THUMBS: std::cell::RefCell<std::collections::HashMap<(u64, NodeId, u32), Option<(u32, u32, Vec<u8>)>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn cached_asset_thumb(doc: &Document, id: NodeId, side_px: f32, revision: u64) -> Option<(u32, u32, Vec<u8>)> {
    let key = (revision, id, side_px as u32);
    ASSET_THUMBS.with(|cache| {
        if let Some(v) = cache.borrow().get(&key) {
            return v.clone();
        }
        let v = asset_thumb(doc, id, side_px);
        let mut m = cache.borrow_mut();
        if m.len() > 512 {
            m.clear();
        }
        m.insert(key, v.clone());
        v
    })
}

/// 一个资产的 40pt 缩略图：与 egui `assets_panel::thumbnail_pixmap` 逐行一致，
/// 光栅化交给 `figcraft_render`（引擎/渲染层），这里只做色彩管理 + RGBA→BGRA 的字节换序。
fn asset_thumb(doc: &Document, id: NodeId, side_px: f32) -> Option<(u32, u32, Vec<u8>)> {
    let slice = doc.node(id).filter(|n| matches!(n.kind, NodeKind::Slice));
    let b = if slice.is_some() { doc.world_bounds(id)? } else { figcraft_render::export_bounds(doc, id)? };
    let side = b.width().max(b.height());
    if !(side.is_finite() && side > 0.0) {
        return None;
    }
    let scale = (f64::from(side_px) / side).min(2.0);
    let pix = if let Some(n) = slice {
        let setting = n.exports.first().cloned().unwrap_or_default();
        let layers = figcraft_engine::cmd::file::drawn_layers(doc, &[id], &setting);
        figcraft_render::render_area(doc, &layers, b, scale)?
    } else {
        figcraft_render::render_node(doc, id, scale)?
    };
    // 与 `canvas::render_worker::screen_image` 相同：tiny-skia 是预乘 RGBA，
    // GPUI 纹理要直通 BGRA；非 sRGB 文档先转 sRGB。
    let mut buf = pix.data().to_vec();
    crate::canvas::render_worker::demultiply(&mut buf);
    if !doc.color_profile.is_srgb() {
        doc.color_profile.conversion_to(figcraft_color::ColorProfile::Srgb).apply_pixels(&mut buf);
    }
    crate::canvas::render_worker::rgba_to_bgra(&mut buf);
    Some((pix.width(), pix.height(), buf))
}

/// 读 Inspect 标签的数据。
pub fn read_inspect(
    doc: &Document,
    ids: &[NodeId],
    lang: figcraft_engine::codegen::CodeLang,
    fmt: figcraft_engine::devinspect::ColorFormat,
    rem: bool,
    revision: u64,
) -> InspectData {
    use figcraft_engine::codegen::CodeLang;
    use figcraft_engine::devinspect;
    let Some(first) = ids.first().copied() else { return InspectData::default() };
    let sections = devinspect::properties_with(doc, first, fmt)
        .into_iter()
        .map(|s| InspectSection {
            title: s.title,
            rows: s.rows
                .into_iter()
                .map(|p| InspectRow { label: p.label, value: p.value, color: p.color, token: p.token })
                .collect(),
        })
        .collect();
    let raw = figcraft_engine::codegen::code_for(doc, first, lang);
    let code = if rem && lang.is_web() { figcraft_engine::codegen::px_to_rem(&raw, 16.0) } else { raw };
    InspectData {
        sections,
        code,
        lang_label: lang.label(),
        lang: lang.id(),
        langs: CodeLang::ALL.iter().map(|l| (l.label(), l.id())).collect(),
        web: lang.is_web(),
        fmt_label: fmt.label(),
        fmt: fmt.id(),
        formats: devinspect::ColorFormat::ALL.iter().map(|f| (f.label(), f.id())).collect(),
        assets: devinspect::assets(doc, ids)
            .iter()
            .map(|a| {
                let idx = a
                    .settings
                    .first()
                    .and_then(|s| figcraft_doc::ExportFormat::ALL.iter().position(|f| *f == s.format))
                    .unwrap_or(0);
                AssetRow {
                    id: a.id,
                    name: a.name.clone(),
                    exports: a.exports.clone(),
                    format_index: idx,
                    thumb: cached_asset_thumb(doc, a.id, 40.0, revision),
                }
            })
            .collect(),
        connect_owner: figcraft_engine::code_connect::owner(doc, first),
        connectable: figcraft_engine::code_connect::language(lang.id()).is_some(),
        link: figcraft_engine::code_connect::link(doc, first, lang).map(|l| CodeLinkRow {
            import_path: l.import_path.clone(),
            name: l.name.clone(),
            template: l.template.clone(),
        }),
    }
}

// ───────────────────────── Dev Mode（Inspect 标签里除属性/代码之外的全部节） ─────────────────────────
//
// 权威来源：`crates/ui-egui/src/panels/dev_mode.rs` 的
// `annotations_section` / `dev_resources_section` / `measurements_section` /
// `categories_section` / `ready_list`。它们每帧直接读 doc，这里同样读成快照。

/// 一条注释（egui `annotations_section` 的每一项）。
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationRow {
    pub index: usize,
    pub label: String,
    /// 分类名与色值（`None` = 无分类）。
    pub category: Option<(String, String)>,
    /// 分类下拉：`(显示名, 色点 hex, 选中后要推的参数)`。第一项是「No category」。
    pub category_menu: Vec<(String, Option<String>, Value)>,
    /// 钉住属性的当前值（只读，egui `prop_row`）；图层没有的显示 "—"。
    pub pins: Vec<InspectRow>,
    /// 「Pin a property」菜单：`(标签, 是否已钉, 选中后要推的参数)`。
    pub pin_menu: Vec<(String, bool, Value)>,
}

/// 一条开发资源（egui `dev_resources_section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct DevResourceRow {
    /// 自己加的可改可删（`Some(index)`）；继承自主组件的只读。
    pub index: Option<usize>,
    pub name: String,
    pub url: String,
    /// 继承来源的名字（`Some` 时第二行显示 `From <主组件>`）。
    pub from: Option<String>,
}

/// 一条留在画布上的测量（egui `measurement_row`）。
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementRow {
    pub from: NodeId,
    pub from_name: String,
    pub to_name: Option<String>,
    /// `None` = 沿边测量（用 `side_label`）。
    pub side: Option<&'static str>,
    /// 它显示的距离（`"12 · 24"`）。
    pub value: String,
    pub index: usize,
}

/// Ready for dev 列表的一项（egui `ready_list`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ReadyRow {
    pub id: NodeId,
    pub name: String,
    pub page: String,
    pub changed: bool,
    /// 状态点的颜色（egui `canvas::status_label` 的三色）。
    pub color: (u8, u8, u8),
}

/// 注释分类（egui `categories_section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// Inspect 标签里 Dev Mode 各节的数据。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DevModeData {
    /// 注释 / 开发资源所属的图层（单选的第一个）。
    pub node: Option<NodeId>,
    /// 当前选区（「Between the selected layers」要前两个）。
    pub sel: Vec<NodeId>,
    /// Ready for dev 的过滤器下标（egui `FILTERS`，与 `inspect.readyForDev {status}` 同源）。
    pub ready_filter: usize,
    pub annotations: Vec<AnnotationRow>,
    pub can_add_annotation: bool,
    pub dev_resources: Vec<DevResourceRow>,
    pub can_add_resource: bool,
    pub measurements: Vec<MeasurementRow>,
    /// 「To the layer around it」可用（父层不是页面）。
    pub can_measure_around: bool,
    /// 「Between the selected layers」可用（选了 ≥2 个）。
    pub can_measure_between: bool,
    pub ready: Vec<ReadyRow>,
    pub categories: Vec<CategoryRow>,
}

/// `figcraft_doc::Side` 的英文标签（egui `side_label`，i18n 未接前用英文）。
pub fn side_label(side: figcraft_doc::Side) -> &'static str {
    use figcraft_doc::Side;
    match side {
        Side::Top => tr("Along the top"),
        Side::Right => tr("Along the right"),
        Side::Bottom => tr("Along the bottom"),
        Side::Left | Side::Auto => tr("Along the left"),
    }
}

/// 状态的三种颜色（照抄 egui `theme.rs` 的 `DEV_READY` / `DEV_CHANGED` / `DEV_COMPLETED`）。
const DEV_READY: (u8, u8, u8) = (0x08, 0x7D, 0x4F);
const DEV_CHANGED: (u8, u8, u8) = (0xA7, 0x5B, 0x00);
const DEV_COMPLETED: (u8, u8, u8) = (0x6E, 0x6E, 0x73);

/// 读 Dev Mode 各节。`ready` / `categories` 与选区无关；其余只在有选中时有值。
pub fn read_dev(doc: &Document, ids: &[NodeId], page: Option<NodeId>, ready_filter: usize) -> DevModeData {
    use figcraft_engine::{annotations, dev_measurements, dev_resources, devinspect};
    let mut d = DevModeData::default();
    d.ready_filter = ready_filter;
    // ── Ready for dev：过滤交给引擎（`dev_items` 就是 `inspect.readyForDev` 命令的实现），
    //    UI 不再自己拿全量再 match。`changed` 那条再按 egui 的 `retain` 收一次。
    let (status, changed_only) = match ready_filter {
        0 => (Some(figcraft_doc::DevStatus::ReadyForDev), false),
        1 => (Some(figcraft_doc::DevStatus::ReadyForDev), true),
        2 => (Some(figcraft_doc::DevStatus::Completed), false),
        _ => (None, false),
    };
    d.ready = devinspect::dev_items(doc, status, None)
        .into_iter()
        .filter(|i| i.changed || !changed_only)
        .map(|i| ReadyRow {
            id: i.id,
            name: i.name,
            page: node_name(doc, i.page),
            changed: i.changed,
            color: match (i.status, i.changed) {
                (_, true) => DEV_CHANGED,
                (figcraft_doc::DevStatus::ReadyForDev, _) => DEV_READY,
                (figcraft_doc::DevStatus::Completed, _) => DEV_COMPLETED,
                (figcraft_doc::DevStatus::None, _) => DEV_COMPLETED,
            },
        })
        .collect();
    d.categories = annotations::categories(doc)
        .iter()
        .map(|c| CategoryRow { id: c.id.clone(), name: c.name.clone(), color: c.color.to_hex() })
        .collect();

    let first = ids.first().copied().filter(|id| doc.node(*id).is_some_and(|n| !n.is_page()));
    // ── Measurements：当前页上、碰到选区的那些（egui `refresh_measurements` + filter）──
    if let Some(page) = page {
        d.measurements = dev_measurements::on_page(doc, page)
            .into_iter()
            .filter(|m| first.is_none_or(|id| m.touches(id)))
            .map(|m| MeasurementRow {
                from: m.from,
                from_name: node_name(doc, m.from),
                to_name: m.measurement.to.map(|t| node_name(doc, t)),
                side: m.measurement.side.map(side_label),
                value: m.labels().join(" · "),
                index: m.index,
            })
            .collect();
    }
    d.can_measure_between = ids.len() >= 2;
    d.sel = ids.to_vec();

    let Some(id) = first else { return d };
    let Some(n) = doc.node(id) else { return d };
    d.can_measure_around = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| !p.is_page());

    // ── Annotations ──
    let sections = devinspect::properties(doc, id);
    let cats = annotations::categories(doc);
    d.annotations = n
        .annotations
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let mut category_menu: Vec<(String, Option<String>, Value)> =
                vec![(tr("No category").into(), None, serde_json::json!({ "id": id, "index": index, "category": null }))];
            for c in cats {
                category_menu.push((
                    c.name.clone(),
                    Some(c.color.to_hex()),
                    serde_json::json!({ "id": id, "index": index, "category": c.id }),
                ));
            }
            let pin_menu = annotations::PINNABLE
                .iter()
                .filter(|p| a.properties.iter().any(|q| q == p.id) || annotations::applies(&sections, p))
                .map(|p| {
                    let on = a.properties.iter().any(|q| q == p.id);
                    let mut list = a.properties.clone();
                    match list.iter().position(|q| q == p.id) {
                        Some(i) => {
                            list.remove(i);
                        }
                        None => list.push(p.id.to_string()),
                    }
                    (p.label.to_string(), on, serde_json::json!({ "id": id, "index": index, "properties": list }))
                })
                .collect();
            AnnotationRow {
                index,
                label: a.label.clone(),
                category: annotations::category_of(doc, a).map(|c| (c.name.clone(), c.color.to_hex())),
                category_menu,
                pins: annotations::pinned(doc, id, a)
                    .into_iter()
                    .map(|(p, v)| match v {
                        Some(prop) => InspectRow { label: prop.label, value: prop.value, color: prop.color, token: prop.token },
                        None => InspectRow { label: p.label.to_string(), value: "—".into(), color: None, token: None },
                    })
                    .collect(),
                pin_menu,
            }
        })
        .collect();
    d.can_add_annotation = n.annotations.len() < annotations::MAX_PER_LAYER;
    d.node = Some(id);

    // ── Dev resources ──
    d.dev_resources = dev_resources::list(doc, id)
        .into_iter()
        .map(|l| DevResourceRow {
            index: l.index,
            name: l.resource.name.clone(),
            url: l.resource.url.clone(),
            from: l.from.and_then(|f| doc.node(f)).map(|m| m.name.clone()),
        })
        .collect();
    d.can_add_resource =
        !figcraft_engine::components::inside_instance(doc, id) && n.dev_resources.len() < dev_resources::MAX_PER_LAYER;
    d
}

/// Component 面板的一行属性（egui `component_panel.rs::instance_props`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentProp {
    pub name: String,
    /// `boolean` / `text` / `swap` / `variant` / `slot`
    pub kind: &'static str,
    /// 当前值（bool 用 "true"/"false"，swap 用组件名，variant 用值名）。
    pub value: String,
    /// 可选值：variant 的候选值、swap 的候选（显示名 + 提交值，逗号 id）。
    pub options: Vec<(String, Value)>,
}

/// Component / Instance 面板（egui `component_panel::instance_section`）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ComponentData {
    pub is_instance: bool,
    /// 嵌在另一个实例里的实例（不给 detach）。
    pub nested: bool,
    /// 主组件已被删除。
    pub missing: bool,
    pub title: &'static str,
    pub main_name: String,
    pub main_id: Option<NodeId>,
    /// swap 菜单的候选（id, name）。
    pub components: Vec<(NodeId, String)>,
    pub description: String,
    pub exposed: bool,
    /// 嵌在主组件里的实例：可「Expose properties to instances」。
    pub nested_in_main: bool,
    pub props: Vec<ComponentProp>,
    /// 组件 / 组件集：描述可编辑。
    pub editable_description: bool,
}

/// 读 Component 面板的数据。
pub fn read_component(doc: &Document, ids: &[NodeId]) -> Option<ComponentData> {
    use figcraft_engine::components;
    let [id] = ids else { return None };
    let n = doc.node(*id)?;
    match &n.kind {
        figcraft_doc::NodeKind::Instance { component, props } => {
            let set = components::variant_set(doc, *component);
            let missing = !doc.contains(*component);
            let main_name = set
                .or(Some(*component))
                .and_then(|c| doc.node(c))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr("Main component deleted").into());
            let nested = components::inside_instance(doc, *id);
            let components = list_components(doc);
            let mut out = ComponentData {
                is_instance: true,
                nested,
                missing,
                title: if nested { tr("Nested instance") } else { tr("Instance") },
                main_name,
                main_id: Some(*component),
                components,
                description: String::new(),
                exposed: n.exposed,
                nested_in_main: !nested && components::owning_component(doc, *id).is_some_and(|c| c != *id),
                props: Vec::new(),
                editable_description: false,
            };
            // 主组件（或变体集）的描述。
            out.description = [Some(*component), set]
                .into_iter()
                .flatten()
                .filter_map(|c| match doc.node(c).map(|x| &x.kind) {
                    Some(figcraft_doc::NodeKind::Component { description, .. } | figcraft_doc::NodeKind::ComponentSet { description }) => {
                        Some(description.trim().to_string())
                    }
                    _ => None,
                })
                .find(|d| !d.is_empty())
                .unwrap_or_default();
            // 变体属性。
            if let Some(set) = set {
                let current = doc.node(*component).map(|c| components::variant_props(&c.name)).unwrap_or_default();
                for (k, vals) in components::variant_options(doc, set) {
                    out.props.push(ComponentProp {
                        name: k.clone(),
                        kind: "variant",
                        value: current.get(&k).cloned().unwrap_or_default(),
                        options: vals.into_iter().map(|v| (v.clone(), serde_json::json!(v))).collect(),
                    });
                }
            }
            // bool / text / swap / slot 属性。
            if let figcraft_doc::NodeKind::Component { props: defs, .. } = &doc.node(*component)?.kind {
                for (k, d) in defs {
                    let v = props.get(k).cloned().unwrap_or_else(|| components::prop_default_value(d));
                    match d {
                        figcraft_doc::PropDef::Boolean { .. } => out.props.push(ComponentProp {
                            name: k.clone(),
                            kind: "boolean",
                            value: v.as_bool().unwrap_or(false).to_string(),
                            options: Vec::new(),
                        }),
                        figcraft_doc::PropDef::Text { .. } => out.props.push(ComponentProp {
                            name: k.clone(),
                            kind: "text",
                            value: v.as_str().unwrap_or_default().to_string(),
                            options: Vec::new(),
                        }),
                        figcraft_doc::PropDef::InstanceSwap { preferred, .. } => {
                            let options = preferred
                                .iter()
                                .chain(out.components.iter().map(|(i, _)| i))
                                .filter_map(|c| doc.node(*c).map(|x| (x.name.clone(), serde_json::json!(c.0))))
                                .collect();
                            let cur = figcraft_engine::params::id_of(&v);
                            out.props.push(ComponentProp {
                                name: k.clone(),
                                kind: "swap",
                                value: cur.and_then(|c| doc.node(c)).map(|x| x.name.clone()).unwrap_or_else(|| tr("None").into()),
                                options,
                            });
                        }
                        // Slot 的填充走 `slot_fill`（挑选组件填进去），此轮只标注出来。
                        figcraft_doc::PropDef::Slot { .. } => out.props.push(ComponentProp {
                            name: k.clone(),
                            kind: "slot",
                            value: String::new(),
                            options: Vec::new(),
                        }),
                        figcraft_doc::PropDef::Variant { .. } => {}
                    }
                }
            }
            Some(out)
        }
        figcraft_doc::NodeKind::ComponentSet { description } => Some(ComponentData {
            title: tr("Component set"),
            description: description.clone(),
            editable_description: true,
            ..Default::default()
        }),
        figcraft_doc::NodeKind::Component { description, .. } => Some(ComponentData {
            title: tr("Component"),
            description: description.clone(),
            editable_description: true,
            ..Default::default()
        }),
        _ => None,
    }
}

/// 文件里的本地组件与组件集（swap 菜单的候选）。
fn list_components(doc: &Document) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    for page in doc.pages.clone() {
        for id in doc.subtree(page) {
            if let Some(n) = doc.node(id)
                && matches!(n.kind, figcraft_doc::NodeKind::Component { .. } | figcraft_doc::NodeKind::ComponentSet { .. })
            {
                out.push((id, n.name.clone()));
            }
        }
    }
    out
}

fn node_name(doc: &Document, id: NodeId) -> String {
    doc.node(id).map(|n| n.name.clone()).unwrap_or_else(|| "?".into())
}

/// 读 `vector_edit` 的选中点（engine `vectoredit` 的 API 与 egui `vector_panel` 完全相同）。
pub fn read_vector(doc: &Document, ve: &figcraft_engine::vectoredit::VectorEdit) -> VectorData {
    use figcraft_engine::vectoredit;
    fn same<T: PartialEq>(mut it: impl Iterator<Item = T>) -> Option<T> {
        let first = it.next()?;
        it.all(|x| x == first).then_some(first)
    }
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let net = vectoredit::network(doc, ve.id);
    let pts = net.map(|n| vectoredit::selected_points(n, ve)).unwrap_or_default();
    let pos = vectoredit::selection_position(doc, ve);
    let b = vectoredit::transform::point_box(doc, ve).filter(|b| b.points.len() > 1);
    let (bw, bh) = match &b {
        Some(b) => (Some(round(b.width)), Some(round(b.height))),
        None => (None, None),
    };
    let vertex = |i: &usize| net.and_then(|n| n.vertices.get(*i));
    // Variable width 工具：选中的宽度点（egui `width_point`）。
    let width_point = if ve.tool == vectoredit::VTool::Width {
        let stroke = doc.node(ve.id).map(|n| &n.stroke);
        let weight = stroke.map_or(1.0, |s| s.weight);
        ve.width_point
            .and_then(|i| Some((i, *stroke?.profile()?.points.get(i)?)))
            .map(|(index, p)| {
                let (l, r) = p.sides();
                WidthPointData {
                    t: round(p.t * 100.0),
                    width: round(p.scale * weight),
                    left: round(l * weight / 2.0),
                    right: round(r * weight / 2.0),
                    index,
                }
            })
    } else {
        None
    };
    VectorData {
        count: pts.len(),
        has_selection: !ve.vertices.is_empty() || !ve.segments.is_empty(),
        x: pos.map(|p| round(p.x)),
        y: pos.map(|p| round(p.y)),
        w: bw,
        h: bh,
        radius: same(pts.iter().map(|v| vertex(v).map(|x| x.radius))).flatten().map(round),
        mirroring: net.and_then(|n| same(pts.iter().map(|v| n.mirroring(*v)))),
        cap: same(pts.iter().map(|v| vertex(v).map(|x| x.cap))).flatten(),
        join: same(pts.iter().map(|v| vertex(v).map(|x| x.join))).flatten(),
        width_point,
    }
}

/// Scale 工具（K）的面板快照（egui `scale_panel.rs`）。
///
/// `cx` / `cy` 是 `scale_about` 算好的画布支点（按锚点取包围盒的 0 / ¼ / ½ 位置），
/// 提交 `transform.scale` 时直接用，不在 UI 层重算。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaleData {
    pub w: f64,
    pub h: f64,
    /// 锚点列 / 行（0 = start, 1 = center, 2 = end）。
    pub anchor: (u8, u8),
    pub cx: f64,
    pub cy: f64,
}

/// 网格 auto layout 的一条轨道（egui `layout_panel.rs::tracks` 的一行）。
#[derive(Clone, Debug, PartialEq)]
pub struct TrackRow {
    /// `sizing` 的 serde 名：`fixed` / `hug` / `fill`。
    pub sizing: &'static str,
    /// Fill 时是 fr 份数；Fixed 时是 px。
    pub value: f64,
    /// 实际布局尺寸（px）——Fixed / Hug 的输入框显示它，Fill 显示 fr。
    pub size: f64,
}

/// 网格 frame 的行列轨道（仅单选一个 grid frame 时 `Some`）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GridTracksData {
    pub columns: Vec<TrackRow>,
    pub rows: Vec<TrackRow>,
}

/// 变量模式的一行（egui `paints.rs::variable_modes`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ModeRow {
    pub collection: String,
    /// 命令用的集合 id（egui `paints.rs::variable_modes` 的 `local`），不是显示名。
    pub collection_id: String,
    pub current: String,
    pub options: Vec<String>,
    /// 与 `options` 一一对应的 `mode` 参数：Auto → null，其余 → mode id（`json!(m.id)`）。
    pub option_ids: Vec<serde_json::Value>,
}

/// Section/Component 的状态（egui `section_panel.rs::section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct SectionData {
    pub is_section: bool,
    pub status: String,
    pub changed: bool,
    pub hide_contents: bool,
    pub color_hex: Option<String>,
}

/// 空选区时的 Page 节（egui `right.rs::page_section`）。
#[derive(Clone, Debug, PartialEq)]
pub struct PageData {
    pub background: String,
    pub profile: String,
    pub profiles: Vec<String>,
    pub local_variables: usize,
    pub local_styles: usize,
}

/// Selection header（`lizard right.rs::header` 的全部判断输入）。
///
/// 这一行不是普通区块：选中 Frame / Group / Text / 形状 / 组件时，右侧那排快捷动作**整套
/// 换掉**。所以把判断所需的原值都快照下来，交给 `panels::visibility::header_actions` 这个
/// 纯函数决定显隐，面板自己不再散写条件。
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderData {
    pub label: String,
    /// 单选 Frame（lizard `is_frame`）。
    pub is_frame: bool,
    /// 选区全是 Group（lizard `is_group`）。
    pub is_group: bool,
    /// 选区全是 Text（lizard `is_text`）。
    pub is_text: bool,
    /// 单选 Component / ComponentSet（lizard `header` 开头的组件专属分支）。
    pub is_component: bool,
    /// 单选形状（lizard `edit_id` → `vector.edit.begin`）。
    pub is_shape: bool,
    /// 布尔组当前运算（lizard `booleans::shown_op`）：决定布尔按钮图标与菜单勾选项。
    pub bool_op: Option<figcraft_doc::BoolOp>,
    /// 选区数量（lizard：Union 按钮只有 `sel.len() >= 2` 才执行）。
    pub count: usize,
    /// 「More actions」菜单行：`(显示名, 命令 id, 上方要分隔线)`。
    /// 取自画布右键菜单的顶层可运行命令（lizard `text.more` 打开的就是它）。
    pub more_actions: Vec<(String, &'static str, bool)>,
}

/// 画布「显示 / 隐藏」开关的快照（egui `right.rs` 的 Zoom 菜单读 `app.session.ds().view`）。
/// 右栏顶部菜单要照 `menu_check` 打勾，状态必须从引擎同一份 `ViewState` 读，不能在 UI 里另存一份。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewData {
    pub zoom: f64,
    pub pixel_preview: bool,
    pub pixel_scale: u8,
    pub pixel_grid: bool,
    pub snap_pixel: bool,
    pub layout_grids: bool,
    pub rulers: bool,
    pub outlines: bool,
}

impl Default for ViewData {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pixel_preview: false,
            pixel_scale: 1,
            pixel_grid: true,
            snap_pixel: true,
            layout_grids: true,
            rulers: false,
            outlines: false,
        }
    }
}

impl ViewData {
    pub fn read(v: &figcraft_engine::ViewState) -> Self {
        Self {
            zoom: v.zoom,
            pixel_preview: v.pixel_preview,
            pixel_scale: v.pixel_scale,
            pixel_grid: v.pixel_grid,
            snap_pixel: v.snap_pixel,
            layout_grids: v.layout_grids,
            rulers: v.rulers,
            outlines: v.outlines,
        }
    }
}

/// 一个本地样式（egui `style_picker::local_styles` 的逐条快照）。
///
/// 比旧的 `(id, name, ty)` 三元组多带了「画色板用的 paint」与「文本样式预览用的 meta」，
/// 这样右栏 Local styles 节能 1:1 照搬 egui 的圆形色板 / "Ag" + "32/Auto" 预览，而不用回读 doc。
#[derive(Clone, Debug, PartialEq)]
pub struct LocalStyleRow {
    pub id: String,
    pub name: String,
    /// `StyleKind::type_name()`：text / paint / effect / grid。
    pub ty: &'static str,
    /// Paint 样式：最后一个 paint（egui `paint_dot` 画的就是它），色板用。
    pub paint: Option<Paint>,
    /// 文本样式："32/Auto" 预览（egui 的 `text_meta`）；其余类型为 `None`。
    pub text_meta: Option<String>,
    pub description: String,
}

/// 右栏所有列表的一次快照。`None` 的 Vec 表示「多选且值不一致」（egui 显示 Mixed / 提示点 + 替换）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PanelData {
    pub live: LiveData,
    pub header: Option<HeaderData>,
    pub fills: Option<Vec<PaintRow>>,
    pub strokes: Option<Vec<PaintRow>>,
    pub stroke_align: Option<&'static str>,
    /// 选中节点的共同描边宽度（egui `paints.rs` 的 `weight.unwrap_or(1.0)`）：
    /// 「Individual strokes」菜单里每个边的权重必须用它，不能硬编码 1.0。
    pub stroke_weight: Option<f64>,
    pub effects: Option<Vec<EffectRow>>,
    /// WGSL 材质绑定（单选时来自该节点的 plugin_data；多选不一致时按「无绑定」处理）。
    pub wgsl: WgslPanelData,
    pub grids: Option<Vec<GridRow>>,
    pub exports: Option<Vec<ExportRow>>,
    pub selection_colors: Vec<SelectionColorRow>,
    /// 当前文档快照的 uid（lizard `ds.uid`）：Selection colors 的展开态按文档分档存，
    /// 切换文档时各自记住自己的开合状态，而不是共用一个全局布尔。
    pub doc_uid: u64,
    pub variable_modes: Vec<ModeRow>,
    pub section: Option<SectionData>,
    pub page: Option<PageData>,
    /// Auto layout 的 W/H + sizing + 约束比例 + min/max。
    pub dimensions: Option<DimensionsData>,
    /// 排版快照（仅全部选中都是文本节点时 `Some`）；驱动 Typography 节。
    pub text: Option<TextData>,
    /// 网格 auto layout 的轨道快照（仅单选一个 grid frame 时 `Some`）。
    pub grid_tracks: Option<GridTracksData>,
    /// Scale 工具激活时的快照；`Some` 时右栏用 Scale 节**替换** Position 节。
    pub scale: Option<ScaleData>,
    /// Frame 工具（F）激活时，Design 标签整页换成 Frame 预设（egui `right.rs` 的
    /// `RightTab::Design if tool == Frame => frame_presets::panel`）。
    pub frame_tool: bool,
    /// 矢量编辑模式的选中点；`Some` 时点面板**替换** Position / Auto layout / Arc / Star。
    pub vector: Option<VectorData>,
    /// 选中节点是否都是「无自动布局（或自身 absolute）的 frame 子层」——只有这种才显示 Constraints 子节。
    pub in_frame: bool,
    /// Constraints 子节的水平 / 垂直约束（egui `right.rs::position_fields`）。`None` = Mixed。
    pub constraints_h: Option<&'static str>,
    pub constraints_v: Option<&'static str>,
    /// Auto layout 的对齐快照（3×3 picker 用）。
    pub layout: crate::panels::LayoutVals,
    /// 数值字段的绑定名（`field key → variable name`），egui `variable_ui::bound_name`。
    pub bound: std::collections::BTreeMap<String, String>,
    /// 可绑定的变量名（`variable_button` 菜单用，按类型分）。
    pub color_variables: Vec<String>,
    pub number_variables: Vec<String>,
    pub bool_variables: Vec<String>,
    pub text_variables: Vec<String>,
    /// 文档里的本地样式（egui `style_picker::local_styles` 的逐条快照），供右栏 Local styles 节
    /// 与各处 Styles 菜单、颜色绑定菜单渲染 / 推命令用。
    pub styles: Vec<LocalStyleRow>,
    /// 节点当前链接的共享样式**名字**，顺序固定 `[fill, stroke, effect, grid]`
    /// （egui `style_picker::chip` 的 `link_of`）。`None` = 该 slot 没有链接样式 → 不画 chip 行；
    /// 多选且链接不一致时也按「没链接」算（egui 的 `common` 语义）。
    pub style_chips: [Option<String>; 4],
    /// 变量编辑器的完整快照（egui `variables_panel.rs`）。
    pub variables: crate::panels::right::variables::VariablesData,
    /// Appearance 的可见性（`object.toggleVisible`）。
    pub visible: Option<bool>,
    /// 画布辅助元素的显示 / 隐藏开关（右栏顶部 Zoom / Show 菜单）。
    pub view: ViewData,
    /// Prototype 标签的全部数据（flows / settings / interactions / scroll）。
    pub proto: ProtoData,
    /// Inspect 标签（Dev Mode）的属性与代码；`None` = 无选中。
    pub inspect: Option<InspectData>,
    /// Inspect 标签里 Dev Mode 的其余节：注释 / 开发资源 / 测量 / 就绪列表 / 分类。
    pub dev: DevModeData,
    /// Component / Instance 面板；`None` = 选中的不是组件相关节点。
    pub component: Option<ComponentData>,
    /// A3: the Comments panel's rows (already filtered/sorted/searched).
    pub comments: Vec<crate::comments::CommentRow>,
    /// A3: the current user (for unread dots).
    pub comment_me: String,
    /// A3: whether the Comment tool is active (the panel shows the Comments list instead).
    pub comment_tool: bool,
    /// A3: every comment on every page, for the "No comments yet" vs "No matching comments" state.
    pub comments_total: usize,
}

/// `common()`：选中节点的某个值全一致才 `Some`，否则 `None`（egui `right.rs::common`）。
fn common<T: PartialEq + Clone>(doc: &Document, ids: &[NodeId], f: impl Fn(&figcraft_doc::Node) -> T) -> Option<T> {
    let mut it = ids.iter().filter_map(|i| doc.node(*i));
    let first = f(it.next()?);
    if it.all(|n| f(n) == first) { Some(first) } else { None }
}

/// 文本节点的 `TextData`；非文本返回 `None`（Type settings 的若干属性都只对文本有意义）。
fn text_data_of(n: &figcraft_doc::Node) -> Option<&figcraft_doc::TextData> {
    match &n.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

/// 可变字体的轴（egui `typography.rs::variable_face`）：**单一**字体族 + 单一字面，且该字面确实是
/// 可变字体时，才列出它的轴。值取第一个选中文本的 `axis_values`（已按 style 的 named instance /
/// 自定义 variations 解析并夹紧），与 egui 的 `values.get(index).unwrap_or(axis.default)` 一致。
/// 当前选中文本所用字面的 GSUB feature tags（egui `details_feature` 的 `face.has_feature`）。
/// 与 `variable_axes` 同源：`find_face` 同步查字体库，单一字体族 + 单一字面才有意义。
/// 找不到 face（如字体未安装 / 多选不同字体）时返回空集，UI 侧按「全部置灰」处理。
fn face_feature_tags(doc: &Document, ids: &[NodeId]) -> std::collections::BTreeSet<String> {
    let text_of = text_data_of;
    let family = common(doc, ids, |n| text_of(n).map(|t| t.style.font_family.clone())).flatten();
    let style_name = common(doc, ids, |n| text_of(n).map(|t| t.style.font_style.clone())).flatten();
    let (Some(family), Some(style_name)) = (family, style_name) else {
        return std::collections::BTreeSet::new();
    };
    let Some(face) = figcraft_text::find_face(&family, &style_name) else {
        return std::collections::BTreeSet::new();
    };
    // 字体库找不到该族时会退回默认字面；那不是「这个字体的 features」
    if !face.family.eq_ignore_ascii_case(&family) {
        return std::collections::BTreeSet::new();
    }
    face.feature_tags().into_iter().collect()
}

fn variable_axes(doc: &Document, ids: &[NodeId]) -> Vec<AxisRow> {
    let text_of = text_data_of;
    let family = common(doc, ids, |n| text_of(n).map(|t| t.style.font_family.clone())).flatten();
    let style_name = common(doc, ids, |n| text_of(n).map(|t| t.style.font_style.clone())).flatten();
    let (Some(family), Some(style_name)) = (family, style_name) else { return Vec::new() };
    let Some(face) = figcraft_text::find_face(&family, &style_name) else { return Vec::new() };
    // 字体库找不到该族时会退回默认字面；那不是「这个字体的轴」。
    if !face.is_variable() || !face.family.eq_ignore_ascii_case(&family) {
        return Vec::new();
    }
    let values = ids.first().and_then(|i| doc.node(*i)).and_then(text_of).map(|t| face.axis_values(&t.style)).unwrap_or_default();
    face.axes
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.hidden)
        .map(|(index, a)| AxisRow {
            tag: a.tag.clone(),
            name: if a.name.trim().is_empty() { figcraft_text::axis_label(&a.tag).to_string() } else { a.name.clone() },
            min: a.min,
            max: a.max,
            default: a.default,
            value: values.get(index).copied().unwrap_or(a.default),
        })
        .collect()
}

/// paint 绑定 key（照抄 `engine::cmd::lists::paint_binding_key` 的语义）。
fn paint_bind_key(n: &figcraft_doc::Node, fills: bool, index: usize) -> String {
    let prefix = if fills { "fills" } else { "strokes" };
    let indexed = format!("{prefix}/{index}/color");
    if n.bound.keys().any(|k| k == &indexed) {
        indexed
    } else if index == 0 && n.bound.keys().any(|k| k == if fills { "fill" } else { "stroke" }) {
        (if fills { "fill" } else { "stroke" }).to_string()
    } else {
        indexed
    }
}

fn paint_rows(doc: &Document, ids: &[NodeId], fills: bool) -> Option<Vec<PaintRow>> {
    let list = common(doc, ids, |n| if fills { n.fills.clone() } else { n.strokes.clone() })?;
    let color_vars: Vec<String> =
        doc.variables.variables.iter().filter(|v| v.kind == figcraft_doc::VariableType::Color).map(|v| v.name.clone()).collect();
    Some(
        list.iter()
            .enumerate()
            .map(|(index, p)| {
                let solid = p.solid_color();
                let (text_is_hex, text) = match solid {
                    Some(c) => (true, c.to_hex()),
                    None => (false, crate::widgets::paint_name(p).to_string()),
                };
                let key = ids.first().and_then(|id| doc.node(*id)).map(|n| paint_bind_key(n, fills, index)).unwrap_or_default();
                let bound = ids
                    .first()
                    .and_then(|id| doc.node(*id))
                    .and_then(|n| n.bound.get(&key))
                    .and_then(|v| doc.variables.get(v))
                    .map(|v| v.name.clone());
                PaintRow {
                    index,
                    fills,
                    paint: p.clone(),
                    text_is_hex,
                    text,
                    opacity: f64::from(p.opacity) * 100.0,
                    visible: p.visible,
                    bound,
                    bind_field: key,
                    variables: color_vars.clone(),
                }
            })
            .collect(),
    )
}

/// 读取当前选区的完整快照。
pub fn read(doc: &Document, ids: &[NodeId], page: Option<NodeId>, ready_filter: usize) -> PanelData {
    let mut d = PanelData::default();
    // 变量编辑器的列表跟选区无关，两种分支都要有（`ids.is_empty()` 会提前 return）。
    d.variables = crate::panels::right::variables::read(doc);
    // Prototype 标签的数据跟选区无关地先算好：`ids.is_empty()` 会提前 return。
    d.proto = read_proto(doc, ids, page);
    // Inspect 标签的 Dev Mode 各节：`ready` / `categories` 也与选区无关。
    d.dev = read_dev(doc, ids, page, ready_filter);
    // 本地样式清单同样与选区无关：空选区时 Page 节的「Local styles」要逐条列出来
    // （egui `page_section` → `style_picker::local_styles`），所以必须在提前 return 之前填。
    d.styles = doc
        .styles
        .iter()
        .map(|s| {
            // 色板 / 文本预览：本地样式节要 1:1 照搬 egui 的色板与 "Ag" + "32/Auto" 预览，
            // 这里预存好，避免面板再回读 doc。逻辑与 egui `paint_dot` / `text_meta` 对齐。
            let (paint, text_meta) = match &s.kind {
                StyleKind::Paint { paints } => (paints.last().cloned(), None),
                StyleKind::Text { style, .. } => (None, Some(format!("{}/{}", trim_num(style.font_size), style.line_height.label()))),
                _ => (None, None),
            };
            LocalStyleRow {
                id: s.id.to_string(),
                name: s.name.clone(),
                ty: s.kind.type_name(),
                paint,
                text_meta,
                description: s.description.clone(),
            }
        })
        .collect();
    if ids.is_empty() {
        // ── Page（`page_section`）：背景色 / 色彩配置 / 本地变量与样式数量 ──
        let bg = page
            .and_then(|p| doc.node(p))
            .and_then(|n| match &n.kind {
                figcraft_doc::NodeKind::Page { background, .. } => Some(*background),
                _ => None,
            })
            .unwrap_or(figcraft_color::Color::DEFAULT_PAGE);
        d.page = Some(PageData {
            background: bg.to_hex(),
            profile: doc.color_profile.label().to_string(),
            profiles: figcraft_color::ColorProfile::ALL.iter().map(|p| p.label().to_string()).collect(),
            local_variables: doc.variables.variables.len(),
            local_styles: doc.styles.len(),
        });
        return d;
    }

    let first = ids.first().and_then(|i| doc.node(*i));
    let kinds = common(doc, ids, |n| n.kind.label().to_string());
    // lizard `right.rs::header` 的五个判断输入，全部只读当前文档 / 选区。
    let is_text = ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))));
    let is_shape = ids.len() == 1 && first.is_some_and(|n| n.kind.is_shape());
    let is_component =
        ids.len() == 1 && first.is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }));
    // More actions：右键菜单的顶层行（子菜单 / 不可运行的命令跳过，分隔线顺延到下一行）。
    let mut more_actions: Vec<(String, &'static str, bool)> = Vec::new();
    let mut carry_sep = false;
    for it in crate::context_menu::items(Some(doc), ids) {
        let sep = it.separator_before || carry_sep;
        if it.sub.is_empty() && !it.disabled && !it.id.is_empty() {
            more_actions.push((it.label.to_string(), it.id, sep));
            carry_sep = false;
        } else {
            carry_sep |= sep;
        }
    }
    d.header = Some(HeaderData {
        label: match &kinds {
            Some(l) if ids.len() == 1 => l.clone(),
            Some(l) => format!("{l}s"),
            None => tr("Mixed").into(),
        },
        is_frame: ids.len() == 1 && first.is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Frame)),
        is_group: ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Group))),
        is_text,
        is_component,
        is_shape,
        bool_op: figcraft_engine::booleans::shown_op(doc, ids),
        count: ids.len(),
        more_actions,
    });

    // ── 已链接的共享样式名（egui `style_picker::chip` → `link_of` → `n.styles.<slot>`）──
    // 多选时只有全部一致才算「有链接」（`common` 返回 `None` 即 Mixed → 不画）。
    {
        let names = common(doc, ids, |n| n.styles.clone());
        d.style_chips = match names {
            Some(r) => {
                let name = |id: &Option<figcraft_doc::styles::StyleId>| id.as_deref().and_then(|i| doc.style(i)).map(|s| s.name.clone());
                [name(&r.fill), name(&r.stroke), name(&r.effect), name(&r.grid)]
            }
            None => [None, None, None, None],
        };
    }

    // ── Component / Instance（egui `component_panel::instance_section`）──
    d.component = read_component(doc, ids);

    d.fills = paint_rows(doc, ids, true);
    d.strokes = paint_rows(doc, ids, false);
    d.stroke_align = common(doc, ids, |n| n.stroke.align).map(|a| match a {
        figcraft_doc::StrokeAlign::Inside => tr("Inside"),
        figcraft_doc::StrokeAlign::Center => tr("Center"),
        figcraft_doc::StrokeAlign::Outside => tr("Outside"),
    });
    d.stroke_weight = common(doc, ids, |n| n.stroke.weight);

    // ── Effects（`effects.rs::effects`） ──
    d.effects = common(doc, ids, |n| n.effects.clone()).map(|list| {
        list.iter()
            .enumerate()
            .map(|(index, e)| EffectRow {
                index,
                label: e.label().to_string(),
                ty: match e.kind {
                    figcraft_doc::EffectKind::DropShadow(_) => "dropShadow",
                    figcraft_doc::EffectKind::InnerShadow(_) => "innerShadow",
                    figcraft_doc::EffectKind::LayerBlur(_) => "layerBlur",
                    figcraft_doc::EffectKind::BackgroundBlur(_) => "backgroundBlur",
                    figcraft_doc::EffectKind::Noise(_) => "noise",
                    figcraft_doc::EffectKind::Texture(_) => "texture",
                    figcraft_doc::EffectKind::Glass(_) => "glass",
                },
                visible: e.visible,
                effect: serde_json::to_value(e).unwrap_or(serde_json::Value::Null),
            })
            .collect()
    });

    // ── WGSL 材质绑定（plugin_data 保留 key，非 EffectKind）──
    // 单选时读该节点的绑定；多选时只有全部一致才显示，避免给多个节点误改同一个材质。
    d.wgsl = crate::wgsl_material::read_binding_of(doc, ids)
        .map(|b| WgslPanelData { path: Some(b.path), params: b.params })
        .unwrap_or_default();

    // ── Layout grids（`effects.rs::grids`） ──
    d.grids = common(doc, ids, |n| n.grids.clone()).map(|list| {
        let grids_json = serde_json::to_value(&list).unwrap_or(serde_json::Value::Array(vec![]));
        list.iter()
            .enumerate()
            .map(|(index, g)| GridRow { index, label: grid_label(g), color: g.color.to_hex(), visible: g.visible, grids: grids_json.clone() })
            .collect()
    });

    // ── Export（`export_panel.rs::section`） ──
    d.exports = common(doc, ids, |n| n.exports.clone()).map(|list| {
        list.iter()
            .enumerate()
            .map(|(index, e)| ExportRow {
                index,
                size: e.size.label(),
                suffix: e.suffix.clone(),
                format: e.format.ext(),
                format_label: e.format.label(),
                is_raster: e.format.is_raster(),
                contents_only: e.contents_only,
                include_bounding_box: e.include_bounding_box,
                svg_outline_text: e.svg_outline_text,
                svg_id_attribute: e.svg_id_attribute,
                svg_simplify_stroke: e.svg_simplify_stroke,
                vd_compat: e.vd_compat.key(),
                bleed: e.print.bleed,
                crop_marks: e.print.crop_marks,
                bleed_marks: e.print.bleed_marks,
                cmyk: e.print.cmyk,
            })
            .collect()
    });

    // ── Selection colors（`selection_colors.rs::section`，至少两组才显示） ──
    d.selection_colors = selection_colors(doc, ids);

    // ── Variable modes（`paints.rs::variable_modes`，仅 frame-like） ──
    if ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())) {
        d.variable_modes = doc
            .variables
            .collections
            .iter()
            .filter(|c| !c.modes.is_empty())
            .map(|c| {
                let own = first.and_then(|n| n.var_modes.get(&c.id)).cloned();
                let inherited = c.modes.first().map(|m| m.name.clone()).unwrap_or_default();
                let current = own
                    .as_ref()
                    .and_then(|m| c.modes.iter().find(|x| x.id == *m).map(|x| x.name.clone()))
                    .unwrap_or_else(|| format!("Auto ({inherited})"));
                let mut options = vec![format!("Auto ({inherited})")];
                options.extend(c.modes.iter().map(|m| m.name.clone()));
                // egui sends `collection: <collection id>` and `mode: <mode id>` (or null for
                // Auto). Display names are for the menu only.
                let mut option_ids = vec![serde_json::Value::Null];
                option_ids.extend(c.modes.iter().map(|m| serde_json::json!(m.id)));
                ModeRow { collection: c.name.clone(), collection_id: c.id.clone(), current, options, option_ids }
            })
            .collect();
    }

    // ── Section / Dev status（`section_panel.rs::section`）──
    // egui 只在「所有选中节点都能带 handoff status」时画这一节；普通矩形/子 frame 不画。
    if ids.iter().all(|i| figcraft_engine::devinspect::has_status(doc, *i)) {
        let is_section = ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Section { .. })));
        let status = common(doc, ids, |n| (n.dev_status, n.dev_changed))
            .map(|(s, changed)| if changed && s == figcraft_doc::DevStatus::ReadyForDev { tr("Changed").to_string() } else { s.label().to_string() });
        d.section = status.map(|status| SectionData {
            is_section,
            status,
            changed: ids.iter().any(|i| doc.node(*i).is_some_and(|n| n.dev_status == figcraft_doc::DevStatus::ReadyForDev && n.dev_changed)),
            hide_contents: ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.hides_contents())),
            color_hex: common(doc, ids, |n| n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex())).flatten(),
        });
    }

    // ── 绑定信息（所有字段共用的 `variable_button` 菜单数据） ──
    for (k, v) in first.map(|n| &n.bound).into_iter().flatten() {
        if let Some(var) = doc.variables.get(v) {
            d.bound.insert(k.clone(), var.name.clone());
        }
    }
    d.color_variables = doc.variables.variables.iter().filter(|v| v.kind == figcraft_doc::VariableType::Color).map(|v| v.name.clone()).collect();
    d.number_variables = doc.variables.variables.iter().filter(|v| v.kind == figcraft_doc::VariableType::Number).map(|v| v.name.clone()).collect();
    d.bool_variables = doc.variables.variables.iter().filter(|v| v.kind == figcraft_doc::VariableType::Boolean).map(|v| v.name.clone()).collect();
    d.text_variables = doc.variables.variables.iter().filter(|v| v.kind == figcraft_doc::VariableType::String).map(|v| v.name.clone()).collect();
    d.visible = common(doc, ids, |n| n.visible);

    // ── Auto layout 的对齐快照（picker 用，与 `CanvasView::selected_layout` 同构）──
    {
        let main_name = |m: figcraft_doc::MainAlign| match m {
            figcraft_doc::MainAlign::Min => "min",
            figcraft_doc::MainAlign::Center => "center",
            figcraft_doc::MainAlign::Max => "max",
            figcraft_doc::MainAlign::SpaceBetween => "spaceBetween",
            figcraft_doc::MainAlign::SpaceAround => "spaceAround",
            figcraft_doc::MainAlign::SpaceEvenly => "spaceEvenly",
        };
        let cross_name = |c: figcraft_doc::CrossAlign| match c {
            figcraft_doc::CrossAlign::Min => "min",
            figcraft_doc::CrossAlign::Center => "center",
            figcraft_doc::CrossAlign::Max => "max",
            figcraft_doc::CrossAlign::Baseline => "baseline",
        };
        let content_name = |c: figcraft_doc::ContentAlign| match c {
            figcraft_doc::ContentAlign::Auto => "auto",
            figcraft_doc::ContentAlign::SpaceBetween => "spaceBetween",
        };
        d.layout = crate::panels::LayoutVals {
            gap: common(doc, ids, |n| n.layout.as_ref().map(|l| l.gap)).flatten(),
            cross_gap: common(doc, ids, |n| n.layout.as_ref().map(|l| l.cross_gap)).flatten(),
            padding: common(doc, ids, |n| n.layout.as_ref().map(|l| l.padding)).flatten(),
            main_align: common(doc, ids, |n| n.layout.as_ref().map(|l| main_name(l.main_align))).flatten(),
            cross_align: common(doc, ids, |n| n.layout.as_ref().map(|l| cross_name(l.cross_align))).flatten(),
            wrap: common(doc, ids, |n| n.layout.as_ref().map(|l| l.wrap)).flatten().unwrap_or(false),
            content_align: common(doc, ids, |n| n.layout.as_ref().map(|l| content_name(l.content_align))).flatten(),
        };
    }

    // ── Dimensions（`layout_panel.rs::dimensions`）：W/H + sizing + 约束比例 + min/max ──
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    let sizing_name = |s: figcraft_doc::Sizing| match s {
        figcraft_doc::Sizing::Fixed => "fixed",
        figcraft_doc::Sizing::Hug => "hug",
        figcraft_doc::Sizing::Fill => "fill",
    };
    d.dimensions = Some(DimensionsData {
        w: common(doc, ids, |n| r2(n.width)),
        h: common(doc, ids, |n| r2(n.height)),
        sizing_w: common(doc, ids, |n| sizing_name(n.child_layout.sizing_h)),
        sizing_h: common(doc, ids, |n| sizing_name(n.child_layout.sizing_v)),
        constrain: common(doc, ids, |n| n.constrain_proportions).unwrap_or(false),
        min_w: common(doc, ids, |n| n.child_layout.min_w).flatten(),
        max_w: common(doc, ids, |n| n.child_layout.max_w).flatten(),
        min_h: common(doc, ids, |n| n.child_layout.min_h).flatten(),
        max_h: common(doc, ids, |n| n.child_layout.max_h).flatten(),
    });

    // ── Constraints（`right.rs::position_fields` 的 Constraints 子节）──
    let in_frame = ids.iter().all(|i| {
        doc.node(*i).is_some_and(|n| {
            n.parent
                .and_then(|p| doc.node(p))
                .is_some_and(|p| p.kind.is_frame_like() && (p.layout.is_none() || n.child_layout.absolute))
        })
    });
    let constraints_h = common(doc, ids, |n| match n.constraints.horizontal {
        figcraft_doc::HConstraint::Left => "left",
        figcraft_doc::HConstraint::Right => "right",
        figcraft_doc::HConstraint::LeftRight => "leftRight",
        figcraft_doc::HConstraint::Center => "center",
        figcraft_doc::HConstraint::Scale => "scale",
    });
    let constraints_v = common(doc, ids, |n| match n.constraints.vertical {
        figcraft_doc::VConstraint::Top => "top",
        figcraft_doc::VConstraint::Bottom => "bottom",
        figcraft_doc::VConstraint::TopBottom => "topBottom",
        figcraft_doc::VConstraint::Center => "center",
        figcraft_doc::VConstraint::Scale => "scale",
    });
    d.in_frame = in_frame;
    d.constraints_h = constraints_h;
    d.constraints_v = constraints_v;

    // ── Typography（`typography/fields.rs`）：全部选中都是文本节点才画这一节 ──
    let text = if ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) {
        fn text_of(n: &figcraft_doc::Node) -> Option<&figcraft_doc::TextData> {
            match &n.kind {
                NodeKind::Text(t) => Some(t),
                _ => None,
            }
        }
        Some(TextData {
            font_family: common(doc, ids, |n| text_of(n).map(|t| t.style.font_family.clone())).flatten(),
            font_style: common(doc, ids, |n| text_of(n).map(|t| t.style.font_style.clone())).flatten(),
            font_size: common(doc, ids, |n| text_of(n).map(|t| r2(t.style.font_size))).flatten(),
            // `None` = Auto（或 Mixed）：字段显示为空、提交回写 `"AUTO"`。
            line_height: common(doc, ids, |n| -> Option<(f64, bool)> {
                text_of(n).and_then(|t| match t.style.line_height {
                    figcraft_doc::LineHeight::Auto => None,
                    figcraft_doc::LineHeight::Pixels(v) => Some((v, false)),
                    figcraft_doc::LineHeight::Percent(p) => Some((p, true)),
                })
            })
            .flatten(),
            // `LetterSpacing` 没有 Auto 变体（默认 `Percent(0.0)`），所以恒有值。
            letter_spacing: common(doc, ids, |n| -> Option<(f64, bool)> {
                text_of(n).and_then(|t| match t.style.letter_spacing {
                    figcraft_doc::LetterSpacing::Pixels(v) => Some((v, false)),
                    figcraft_doc::LetterSpacing::Percent(p) => Some((p, true)),
                })
            })
            .flatten(),
            paragraph_spacing: common(doc, ids, |n| text_of(n).map(|t| t.paragraph_spacing)).flatten(),
            auto_resize: common(doc, ids, |n| {
                text_of(n).map(|t| match t.auto_resize {
                    figcraft_doc::TextAutoResize::WidthAndHeight => "widthAndHeight",
                    figcraft_doc::TextAutoResize::Height => "height",
                    figcraft_doc::TextAutoResize::None => "none",
                    figcraft_doc::TextAutoResize::Truncate => "truncate",
                })
            })
            .flatten(),
            align_h: common(doc, ids, |n| {
                text_of(n).map(|t| match t.align_h {
                    figcraft_doc::TextAlignH::Left => "left",
                    figcraft_doc::TextAlignH::Center => "center",
                    figcraft_doc::TextAlignH::Right => "right",
                    figcraft_doc::TextAlignH::Justified => "justified",
                })
            })
            .flatten(),
            align_v: common(doc, ids, |n| {
                text_of(n).map(|t| match t.align_v {
                    figcraft_doc::TextAlignV::Top => "top",
                    figcraft_doc::TextAlignV::Middle => "middle",
                    figcraft_doc::TextAlignV::Bottom => "bottom",
                })
            })
            .flatten(),
            // ── Type settings 用的那一组（egui `type_settings_basics` / `text_style_settings`）──
            decoration: common(doc, ids, |n| {
                text_of(n).map(|t| match t.style.decoration {
                    figcraft_doc::TextDecoration::None => "none",
                    figcraft_doc::TextDecoration::Underline => "underline",
                    figcraft_doc::TextDecoration::Strikethrough => "strikethrough",
                })
            })
            .flatten(),
            decoration_style: common(doc, ids, |n| {
                text_of(n).map(|t| match t.style.decoration_look.as_ref().map(|l| l.style) {
                    None | Some(figcraft_doc::DecorationStyle::Solid) => "solid",
                    Some(figcraft_doc::DecorationStyle::Dotted) => "dotted",
                    Some(figcraft_doc::DecorationStyle::Wavy) => "wavy",
                })
            })
            .flatten(),
            // 粗细 `Auto`（= 跟字号走）当空值处理，与 egui 的 `NumberField::empty(Auto)` 一致。
            decoration_thickness: common(doc, ids, |n| {
                text_of(n).and_then(|t| match t.style.decoration_look.as_ref().map(|l| l.thickness) {
                    Some(figcraft_doc::LineHeight::Pixels(v)) => Some(r2(v)),
                    _ => None,
                })
            })
            .flatten(),
            decoration_skip_ink: common(doc, ids, |n| text_of(n).map(|t| t.style.decoration_look.as_ref().is_some_and(|l| l.skip_ink))).flatten(),
            case: common(doc, ids, |n| {
                text_of(n).map(|t| match t.style.case {
                    figcraft_doc::TextCase::Original => "original",
                    figcraft_doc::TextCase::Upper => "upper",
                    figcraft_doc::TextCase::Lower => "lower",
                    figcraft_doc::TextCase::Title => "title",
                    figcraft_doc::TextCase::SmallCaps => "smallCaps",
                })
            })
            .flatten(),
            leading_trim: common(doc, ids, |n| {
                text_of(n).map(|t| match t.leading_trim {
                    figcraft_doc::LeadingTrim::None => "none",
                    figcraft_doc::LeadingTrim::CapHeight => "capHeight",
                })
            })
            .flatten(),
            list_style: common(doc, ids, |n| {
                text_of(n).map(|t| match t.list {
                    figcraft_doc::ListStyle::None => "none",
                    figcraft_doc::ListStyle::Bulleted => "bulleted",
                    figcraft_doc::ListStyle::Numbered => "numbered",
                })
            })
            .flatten(),
            // ── Details 页 / Wrap style 用的那一组 ──
            paragraph_indent: common(doc, ids, |n| text_of(n).map(|t| r2(t.paragraph_indent))).flatten(),
            hanging_punctuation: common(doc, ids, |n| text_of(n).map(|t| t.hanging_punctuation)).flatten(),
            hanging_list: common(doc, ids, |n| text_of(n).map(|t| t.hanging_list)).flatten(),
            wrap_style: common(doc, ids, |n| {
                text_of(n).map(|t| match t.wrap_style {
                    figcraft_doc::WrapStyle::Auto => "auto",
                    figcraft_doc::WrapStyle::Balance => "balance",
                    figcraft_doc::WrapStyle::Pretty => "pretty",
                })
            })
            .flatten(),
            max_lines: common(doc, ids, |n| text_of(n).map(|t| t.max_lines)).flatten(),
            // 只有所有选中节点的特性覆盖完全一致才带出具体 tag，否则空表 = 「全默认 / Mixed」。
            features: common(doc, ids, |n| text_of(n).map(|t| t.features.clone())).flatten().unwrap_or_default(),
            // 当前字面真实提供的 OpenType features：Details 页按此置灰 / 补漏。
            features_supported: face_feature_tags(doc, ids),
            // 可变字体的轴：只有**单一**字体族 + 单一字面才谈得上「这个字体的轴」（egui
            // `variable_face`）；值取该字面的 `axis_values`，缺省退回轴的默认值。
            axes: variable_axes(doc, ids),
        })
    } else {
        None
    };
    d.text = text;

    // ── 网格轨道（egui `layout_panel.rs::tracks`）：只在单选一个 grid frame 时出现 ──
    d.grid_tracks = ids.first().and_then(|id| {
        let layout = doc.node(*id)?.layout.as_ref()?;
        let g = figcraft_layout::grid_geometry(doc, *id)?;
        let track_rows = |specs: &[figcraft_doc::GridTrack], ranges: &[(f64, f64)]| -> Vec<TrackRow> {
            ranges
                .iter()
                .enumerate()
                .take(GRID_TRACK_ROWS)
                .map(|(i, (a, b))| {
                    let t = specs.get(i).copied().unwrap_or_default();
                    TrackRow {
                        sizing: match t.sizing {
                            figcraft_doc::Sizing::Fixed => "fixed",
                            figcraft_doc::Sizing::Hug => "hug",
                            figcraft_doc::Sizing::Fill => "fill",
                        },
                        value: t.value,
                        size: ((b - a) * 100.0).round() / 100.0,
                    }
                })
                .collect()
        };
        Some(GridTracksData {
            columns: track_rows(&layout.column_tracks, &g.columns),
            rows: track_rows(&layout.row_tracks, &g.rows),
        })
    });

    d
}

fn grid_label(g: &figcraft_doc::LayoutGrid) -> String {
    use figcraft_doc::GridKind;
    let trim = |v: f64| format!("{v}");
    match g.kind {
        GridKind::Grid => format!("Grid ({})", trim(g.size)),
        GridKind::Columns if g.count == 0 => tr("Auto columns").into(),
        GridKind::Rows if g.count == 0 => tr("Auto rows").into(),
        GridKind::Diagonal if g.count == 0 => "Auto 45° lines".into(),
        GridKind::Rings if g.count == 0 => "Auto rings".into(),
        GridKind::Columns => format!("{} columns", g.count.max(1)),
        GridKind::Rows => format!("{} rows", g.count.max(1)),
        GridKind::Diagonal => format!("{} lines at 45°", g.count.max(1)),
        GridKind::Rings => format!("{} rings", g.count.max(1)),
    }
}

/// 选区颜色分组。**直接用引擎的 `collect`**，与 egui `selection_colors::colors` 是同一份数据。
///
/// 之前这里自己按 `(hex, opacity)` 扫 `n.fills / n.strokes` 分组，带来三个实打实的接线问题：
/// - `linked` 恒为 `None` → 已绑定的样式/变量行永远显示成「未绑定」，detach 分支不可达；
/// - `targets` 没有 `range` → 文本局部填色不在任何一组里，颜色按钮影响不到它；
/// - 分组没有考虑 mask / BooleanOp / 子层 / 不可见 paint，和引擎自己的选区模型不一致。
/// `collect` 已经把这些都算好了（含 `style` / `variable` id 和 `PaintTarget::range`）。
fn selection_colors(doc: &Document, ids: &[NodeId]) -> Vec<SelectionColorRow> {
    use figcraft_doc::VariableType;
    use figcraft_engine::selection_colors::{bind_refusal, collect, in_scope};
    collect(doc, ids)
        .into_iter()
        .map(|g| {
            let solid = g.paint.solid_color();
            let hex = match solid {
                Some(c) => c.to_hex(),
                None => crate::widgets::paint_name(&g.paint).to_string(),
            };
            // egui `section`: 样式名优先，其次变量名。两者都是 **id**，要换成显示名。
            let linked = g
                .style
                .as_ref()
                .and_then(|id| doc.style(id).map(|s| s.name.clone()))
                .or_else(|| g.variable.as_ref().and_then(|id| doc.variables.get(id).map(|v| v.name.clone())));
            // egui `section`: 只列作用域内、引擎肯接受的颜色变量，并且提交 **id**。
            let variables = doc
                .variables
                .variables
                .iter()
                .filter(|v| v.kind == VariableType::Color && bind_refusal(doc, &g.targets, &v.id).is_none() && in_scope(doc, &g.targets, &v.id))
                .map(|v| (v.name.clone(), v.id.clone()))
                .collect();
            let op = f64::from(g.paint.opacity) * 100.0;
            SelectionColorRow {
                hex,
                solid: solid.is_some(),
                opacity: op,
                linked,
                targets: serde_json::to_value(&g.targets).unwrap_or_else(|_| serde_json::json!([])),
                paint: serde_json::to_value(&g.paint).unwrap_or(serde_json::Value::Null),
                variables,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_engine::Session;
    use serde_json::json;

    fn rect_doc() -> (Session, NodeId) {
        let mut s = Session::with_document(Document::new("export-test"));
        let v = s.execute("shape.rectangle", &json!({ "x": 0.0, "y": 0.0, "width": 64.0, "height": 64.0 })).expect("rect");
        let id: NodeId = serde_json::from_value(v["id"].clone()).expect("id");
        // 给它一个导出设置，才算是 Inspect 的 asset。
        s.execute("export.add", &json!({ "ids": [id] })).expect("export.add");
        (s, id)
    }

    /// 每一种 `Effect::TYPES` 加进去后都必须变成一行面板数据 —— 少一行，用户看到的就是
    /// 「点了没反应」。这条覆盖菜单里的全部 7 种，不只是阴影。
    #[test]
    fn every_effect_type_becomes_a_panel_row() {
        let mut s = Session::with_document(Document::new("fx-rows"));
        let v = s.execute("shape.rectangle", &json!({ "x": 0.0, "y": 0.0, "width": 64.0, "height": 64.0 })).expect("rect");
        let id: NodeId = serde_json::from_value(v["id"].clone()).expect("id");
        for (_, ty) in figcraft_doc::Effect::TYPES {
            s.execute("effect.add", &json!({ "ids": [id], "type": ty })).expect("effect.add");
        }
        let doc = s.doc().expect("doc");
        let d = read(doc, &[id], None, 0);
        let rows = d.effects.expect("rows for a single selection");
        assert_eq!(rows.len(), figcraft_doc::Effect::TYPES.len(), "one row per type");
        for (row, (label, ty)) in rows.iter().zip(figcraft_doc::Effect::TYPES) {
            assert_eq!(row.label, label, "the row shows the type's label");
            assert_eq!(row.ty, ty, "the row carries the type the menu adds");
            assert_eq!(row.effect.get("type").and_then(|v| v.as_str()), Some(ty), "serialised effect keeps its type");
        }
    }

    /// 导出是引擎级能力：只调 `file.export {ids, format, path}`，文件就该由引擎写到磁盘。
    /// UI 不需要（也不应该）自己光栅化 / 编码。
    #[test]
    fn export_writes_a_file_without_any_ui_code() {
        let (mut s, id) = rect_doc();
        let path = std::env::temp_dir().join(format!("figcraft_gpui_export_{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let r = s.execute("file.export", &json!({ "ids": [id], "format": "png", "path": path.to_string_lossy() }));
        assert!(r.is_ok(), "file.export failed: {r:?}");
        assert!(path.exists(), "引擎应把文件写到 path");
        assert!(std::fs::metadata(&path).map(|m| m.len() > 100).unwrap_or(false), "导出的 PNG 不是空文件");
        let _ = std::fs::remove_file(&path);
    }

    /// 缩略图同样是渲染 crate 的产物（`figcraft_render::render_node`），不是 UI 自绘。
    /// Ready 列表的四种过滤器交给引擎 `dev_items(doc, status, None)`（= `inspect.readyForDev`），
    /// UI 不再拿全量自己筛。
    #[test]
    fn ready_filter_is_applied_by_the_engine_query() {
        let mut s = Session::with_document(Document::new("ready-test"));
        let mk = |s: &mut Session, x: f64| -> NodeId {
            let v = s.execute("frame.create", &json!({ "x": x, "y": 0.0, "width": 100.0, "height": 100.0 })).expect("frame");
            serde_json::from_value(v["id"].clone()).expect("id")
        };
        let a = mk(&mut s, 0.0);
        let b = mk(&mut s, 300.0);
        s.execute("node.set", &json!({ "ids": [a], "props": { "devStatus": "readyForDev" } })).expect("status a");
        s.execute("node.set", &json!({ "ids": [b], "props": { "devStatus": "completed" } })).expect("status b");
        let doc = s.doc().expect("doc");
        let page = doc.pages[0];
        assert_eq!(read_dev(doc, &[], Some(page), 0).ready.len(), 1, "Ready for dev");
        assert_eq!(read_dev(doc, &[], Some(page), 2).ready.len(), 1, "Completed");
        assert_eq!(read_dev(doc, &[], Some(page), 3).ready.len(), 2, "Any status");
    }

    #[test]
    fn asset_thumb_comes_from_the_render_crate() {
        let (s, id) = rect_doc();
        let doc = s.doc().expect("doc");
        let (w, h, buf) = asset_thumb(doc, id, 40.0).expect("thumbnail");
        assert!(w > 0 && h > 0);
        assert_eq!(buf.len(), (w * h * 4) as usize, "BGRA 四通道字节");
        assert!(buf.iter().any(|b| *b != 0), "渲染出了实际像素");
        let d = read_inspect(doc, &[id], figcraft_engine::codegen::CodeLang::ALL[0], figcraft_engine::devinspect::ColorFormat::ALL[0], false, 0);
        assert_eq!(d.assets.len(), 1);
        assert_eq!(d.assets[0].id, id);
        assert!(d.assets[0].thumb.is_some(), "Assets 行带着缩略图");
    }
}


// DIAGNOSTIC CUT (isolate data.rs): stub LiveData so per-frame `read` is neutralized.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct LiveData {
    pub me_name: String,
    pub me_color: Option<String>,
    pub peers: Vec<(u32, String, String)>,
    pub active: bool,
    pub status: &'static str,
    pub reason: Option<String>,
    pub host: bool,
    pub follow: Option<u32>,
    pub spotlighting: bool,
    pub can_remove: bool,
    pub can_host: bool,
    pub code: Option<String>,
    pub link: Option<String>,
    pub info_link: Option<String>,
    pub shared: bool,
    pub online: bool,
    pub server_set: bool,
    pub view_only: bool,
}

impl LiveData {
    pub fn read(_session: &figcraft_engine::Session) -> Self {
        Self::default()
    }
}
