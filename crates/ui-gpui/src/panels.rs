//! The GPUI shell: left rail, left/right panels, bottom animation bar and the floating
//! bottom toolbar.
//!
//! Everything here is built from the reusable controls in [`crate::widgets`] — no new controls
//! are introduced. The right panel restores the egui Design panel's section order; sections whose
//! controls have not been ported yet are shown as greyed (`Dim`) headers with an empty body.

use figcraft_i18n::tr_noop;
use figcraft_i18n::tr;
use crate::CanvasView;
use crate::canvas::geom::Color32;
use crate::widgets::menu::MenuRow;
use crate::widgets::{
    Icon, IconButton, PanelSection, SectionHeader, SectionHeaderVariant, Segmented, Slot, TextEdit, TextField, field_row, field_row_full, pt, rgba,
    sublabel_row, sublabel_row2, tokens,
};
use figcraft_color::BlendMode;
use gpui::{App, Div, Entity, SharedString, Stateful, Window, div, prelude::*, px};
use serde_json::Value;
use std::rc::Rc;

/// 右栏高级设置子模块（描边/效果等的高级弹层占位）。源码抄自 `ui-egui::panels::right`，见各文件。
pub(crate) mod right;
/// 四块整面板（Shortcuts / Versions / Dev compare / Assets），由引擎 `ui_requests` 打开的浮层。
pub(crate) mod extra;

/// A field's width before the panel reports its own (`col_width(280)`).
pub(crate) const DEFAULT_COL: f32 = 108.0;
/// The fixed left icon rail.
pub(crate) const RAIL_W: f32 = 56.0;
/// 左竖栏里图标按钮的边长（`left_rail` 的 `IconButton::size(32.0)`）。
const RAIL_BTN: f32 = 32.0;
/// 左竖栏的上下内边距（`left_rail` 的 `py(pt(8.0))`）。
const RAIL_PAD_Y: f32 = 8.0;

/// 左竖栏**第 1 颗按钮**（主菜单）的矩形。
///
/// 主菜单要贴在这颗按钮的底边往下 4pt（`menu` 的 `gap`），所以需要它的真实位置：
/// 竖栏 56 宽、`border_1`（流内边框，内容盒 54）、上下 `py(8)`、按钮 32×32 居中。
/// `IconButton::on_click` 的 `ClickEvent` 只带指针坐标、不带元素矩形，只能按这套固定
/// 几何算——改竖栏的宽度/内边距/按钮尺寸时，这里必须同步。
pub(crate) fn rail_menu_bounds() -> gpui::Bounds<gpui::Pixels> {
    use gpui::{Bounds, Point, px, size};
    let x = 1.0 + (RAIL_W - 2.0 - RAIL_BTN) / 2.0;
    let y = 1.0 + RAIL_PAD_Y;
    Bounds { origin: Point { x: px(x), y: px(y) }, size: size(px(RAIL_BTN), px(RAIL_BTN)) }
}

/// 指针是否落在左栏 rail 的某颗按钮上（窗口坐标）。
/// 弹层的 backdrop 必须放行这块区域：否则「菜单/变量窗口已开时再点同一颗按钮」会被 backdrop
/// 先一步 dismiss 掉，按钮的 `on_click` 收不到，toggle 就永远只表现为「重新打开」。
pub(crate) fn rail_button_hit(x: f32, y: f32) -> bool {
    // 只判「是否落在左侧竖栏顶部的按钮带」：窗口上方还有一条标题区，按 RAIL_PAD_Y
    // 逐颗按钮算精确矩形会整体偏 ~23pt，让第 2 次点击被 backdrop 吞掉。竖栏顶部
    // 200pt 内都是按钮/间隔，整块放行最稳妥（egui 的 rail 本身就是弹层 Hold 区）。
    (0.0..=RAIL_W).contains(&x) && (0.0..=200.0).contains(&y)
}

/// The bottom animation bar's height (egui's timeline is taller; this keeps the canvas usable).
pub(crate) const ANIM_H: f32 = 220.0;
/// A panel divider's **visible** width: the hairline you actually see. It stays 1 px so the
/// layout geometry (canvas_x / canvas_w, and every hit-test that subtracts them) is untouched.
pub(crate) const HANDLE_W: f32 = 1.0;
/// The divider's **click** width: `1 px line + 1.5 px on each side = 4 px`, centred on the line.
/// The hit area is a separate, top-layer overlay (see `CanvasView::divider_hotspots`) because
/// GPUI hits the most-recently-painted hitbox first — the canvas painted right after the left
/// divider would otherwise steal the 1.5 px that spill onto it.
pub(crate) const HANDLE_HIT_W: f32 = 4.0;
/// Overhang on each side of the visible hairline (`(HANDLE_HIT_W - HANDLE_W) / 2`).
pub(crate) const HANDLE_OVERHANG: f32 = 1.5;

/// Centre x of a divider, in window coordinates. `w` is that dock's current width.
/// Both the visible hairline and the 4 px hit overlay derive from this one source.
pub(crate) fn divider_center_x(side: crate::ResizeSide, viewport_w: f32, w: f32) -> f32 {
    match side {
        crate::ResizeSide::Left => RAIL_W + w + HANDLE_W / 2.0,
        crate::ResizeSide::Right => viewport_w - w - HANDLE_W / 2.0,
    }
}

/// Left edge of the 4 px hit overlay for a divider centred at `cx`.
pub(crate) fn divider_hit_left(cx: f32) -> f32 {
    cx - HANDLE_HIT_W / 2.0
}

#[cfg(test)]
mod divider_tests {
    use super::*;

    #[test]
    fn hit_width_is_line_plus_even_overhang() {
        assert_eq!(HANDLE_HIT_W, HANDLE_W + HANDLE_OVERHANG * 2.0);
        assert_eq!(HANDLE_OVERHANG, (HANDLE_HIT_W - HANDLE_W) / 2.0);
    }

    #[test]
    fn hit_overlay_is_centred_on_the_visible_line() {
        for centre in [56.0 + 280.0 + 0.5, 1280.0 - 280.0 - 0.5] {
            let left = divider_hit_left(centre);
            assert_eq!(left + HANDLE_HIT_W / 2.0, centre, "4 px 热区以可见线为中心");
            // 线本身落在热区正中间，两侧各留 1.5 px。
            assert_eq!(centre - (left + HANDLE_OVERHANG), HANDLE_W / 2.0);
        }
    }

    #[test]
    fn both_dividers_share_one_centre_source() {
        use crate::ResizeSide;
        // 左：rail + 左面板 + 半条线；右：视口宽 − 右面板 − 半条线。
        let left = divider_center_x(ResizeSide::Left, 1280.0, 280.0);
        let right = divider_center_x(ResizeSide::Right, 1280.0, 280.0);
        assert_eq!(left, RAIL_W + 280.0 + HANDLE_W / 2.0);
        assert_eq!(right, 1280.0 - 280.0 - HANDLE_W / 2.0);

        // 两条覆盖条都是 [centre−2, centre+2]，宽 4；可见线 [centre−0.5, centre+0.5] 居中。
        for centre in [left, right] {
            let hit_l = divider_hit_left(centre);
            assert_eq!((hit_l, hit_l + HANDLE_HIT_W), (centre - 2.0, centre + 2.0));
            assert_eq!(centre - hit_l - HANDLE_OVERHANG, HANDLE_W / 2.0);
            assert_eq!(hit_l + HANDLE_OVERHANG + HANDLE_W + HANDLE_OVERHANG, centre + 2.0);
        }
    }
}
/// Panel width limits, per the user's spec.
pub(crate) const MIN_PANEL: f32 = 240.0;
pub(crate) const MAX_PANEL: f32 = 500.0;
pub(crate) const DEFAULT_PANEL: f32 = 280.0;

/// A right-panel field that writes back to the selected node via `node.set`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Prop {
    X,
    Y,
    W,
    H,
    Rotation,
    Opacity,
    Corner,
    /// Ellipse arc start (degrees).
    ArcStart,
    /// Ellipse arc sweep (degrees).
    ArcSweep,
    /// Ellipse arc inner radius, shown as a percentage.
    ArcInner,
    /// Star / polygon point count.
    PointCount,
    /// Star inner radius, shown as a percentage.
    StarRatio,
    /// Text font size.
    FontSize,
    /// Stroke weight.
    StrokeWeight,
}

impl Prop {
    /// The `node.set` property key.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Prop::X => "x",
            Prop::Y => "y",
            Prop::W => "width",
            Prop::H => "height",
            Prop::Rotation => "rotation",
            Prop::Opacity => "opacity",
            Prop::Corner => "cornerRadius",
            Prop::ArcStart => "arcStart",
            Prop::ArcSweep => "arcSweep",
            Prop::ArcInner => "arcInner",
            Prop::PointCount => "pointCount",
            // 引擎里星形的内径比键名是 `innerRadius`（props.rs: "innerRadius" | "ratio"）。
            Prop::StarRatio => "innerRadius",
            Prop::FontSize => "fontSize",
            Prop::StrokeWeight => "strokeWeight",
        }
    }

    /// 输入侧的范围（与 [`Self::map`] 写回文档时的夹紧保持一致）：
    /// 百分比 0..100、尺寸类 ≥0、点数 3..60，其余不限。范围在 `TextEdit` 里统一落地，
    /// 打字 / 方向键 / 拖拽 / 文档回推四条路径都过 `NumberConfig::clamp`。
    pub(crate) fn range(self) -> (Option<f64>, Option<f64>) {
        match self {
            Prop::Opacity | Prop::ArcInner | Prop::StarRatio => (Some(0.0), Some(100.0)),
            Prop::Corner | Prop::FontSize | Prop::StrokeWeight => (Some(0.0), None),
            Prop::PointCount => (Some(3.0), Some(60.0)),
            _ => (None, None),
        }
    }

    /// Display value → document value, mirroring `ui-egui`'s `field_of` maps:
    /// opacity is a percentage clamped to `0..=1`; corner radius never goes negative.
    pub(crate) fn map(self, v: f64) -> f64 {
        match self {
            Prop::Opacity => (v / 100.0).clamp(0.0, 1.0),
            Prop::Corner => v.max(0.0),
            Prop::ArcInner | Prop::StarRatio => (v / 100.0).clamp(0.0, 1.0),
            Prop::PointCount => v.round().clamp(3.0, 60.0),
            _ => v,
        }
    }
}

/// Fields push `(prop, document value)` here on commit; the view drains it and runs `node.set`.
pub(crate) type Pending = std::rc::Rc<std::cell::RefCell<Vec<(Prop, f64)>>>;
/// A panel command raised against the selection: an id plus an optional extra params object that is
/// merged into `{"ids": ids}` when executed. egui runs `app.run(cmd, {"ids": ids})`, but several
/// commands need more — `effect.add {type:"dropShadow"}`, `grid.add`, `layout.add`, … — so the
/// channel carries those extras instead of only a bare command name.
pub(crate) struct Cmd {
    pub id: &'static str,
    pub params: Option<Value>,
}
/// Commands raised by the panel (align, distribute, layout/effect/grid add, …) and run against the
/// selection after the frame — the port of `app.run(cmd, {"ids": ids})` in `ui-egui`'s right panel.
///
/// **The queue owns the view's wake handle**: egui is immediate-mode, so a button's click always
/// gets a repaint and `app.run` happens that same frame. GPUI is retained — `CanvasView::render`
/// only runs when the view is dirty, and `sync_panel` (the only place that drains this queue) is
/// called from `render`. A bare `push` from a plain icon/segmented button therefore left the
/// command sitting in the queue forever; only menu buttons worked, because `close_popup` happens
/// to call `cx.notify()`. Every push now wakes the view, so any button — present or future — gets
/// its command executed on the next frame.
#[derive(Clone)]
pub(crate) struct CmdQueue {
    items: std::rc::Rc<std::cell::RefCell<Vec<Cmd>>>,
    wake: std::rc::Rc<dyn Fn()>,
}

impl CmdQueue {
    pub(crate) fn new(wake: &std::rc::Rc<dyn Fn()>) -> Self {
        Self { items: Default::default(), wake: wake.clone() }
    }

    /// Enqueue and force the frame that runs it.
    pub(crate) fn push(&self, cmd: Cmd) {
        self.items.borrow_mut().push(cmd);
        (self.wake)();
    }

    /// Ask for one more frame without raising an engine command (popup requests use this).
    pub(crate) fn wake(&self) {
        (self.wake)();
    }

    pub(crate) fn drain(&self) -> Vec<Cmd> {
        self.items.borrow_mut().drain(..).collect()
    }

    /// Only for tests: how many commands are still waiting for a frame.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.borrow().len()
    }
}
pub(crate) type Cmds = CmdQueue;

/// A cloneable handle a button captures to open a detached window. It does not hold the root
/// `Entity<CanvasView>`: the root view drains the slot on its next frame, so a click can never be
/// swallowed by a missing `set_root`. `wake()` asks for that frame.
#[derive(Clone)]
pub(crate) struct PopupReqs {
    slot: Rc<std::cell::RefCell<Option<crate::popovers::WindowState>>>,
    cmds: Cmds,
}

impl PopupReqs {
    pub(crate) fn request(&self, w: crate::popovers::WindowState) {
        *self.slot.borrow_mut() = Some(w);
        self.cmds.wake();
    }

    /// DIAGNOSTIC CUT glue: no-op (base PopupReqs lacks open_kind/close_req fields).
    pub(crate) fn sync_open(&self, _current: Option<crate::popovers::WindowKind>) {}
    pub(crate) fn take_close(&self) -> bool {
        false
    }
}

/// Merge a button's extra params into `{"ids": ids}` — the GPUI equivalent of egui's
/// `app.run(cmd, json!({"ids": ids, ...}))`. `sync_panel` calls this once per drained command.
/// Kept as a pure function so the wiring is testable without a GPUI window.
pub(crate) fn cmd_params(ids: &[figcraft_doc::NodeId], extra: Option<Value>) -> Value {
    let mut params = serde_json::json!({ "ids": ids });
    if let Some(extra) = extra
        && let (Some(base), Some(add)) = (params.as_object_mut(), extra.as_object())
    {
        for (k, v) in add {
            base.insert(k.clone(), v.clone());
        }
    }
    params
}

/// The text/number fields the right panel embeds. They are created once and cached so their
/// editing state survives re-renders (GPUI elements are consumed by paint and cannot be rebuilt
/// per frame).
/// 右栏各 section 的显示条件，逐条对应 egui `panels/right.rs::design`：
/// - 无选中 → 只有 Page；
/// - Arc 仅当**全部**选中都是 Ellipse；
/// - Star 仅当全部是 Polygon/Star；
/// - Typography 仅当选中含 Text；
/// - Layout grid 仅当全部是 frame-like。
/// The common auto-layout direction across the selection (`None` = not in auto layout, or mixed).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LayoutDir {
    #[default]
    None,
    Vertical,
    Horizontal,
    Grid,
}

/// Selection's common auto-layout values pushed into the cached fields (selection → panel).
/// All fields are `Option`: `None` = mixed / no value → shown as "Mixed".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LayoutVals {
    pub gap: Option<f64>,
    pub cross_gap: Option<f64>,
    pub padding: Option<[f64; 4]>,
    pub main_align: Option<&'static str>,
    pub cross_align: Option<&'static str>,
    /// `wrap`（换行流）与 `contentAlign`（`spaceBetween` 时交叉轴按分布画）。
    pub wrap: bool,
    pub content_align: Option<&'static str>,
}

/// 一行列表编辑器（paint / export / selection-color …）的提交目标。
/// egui 每帧用 `Id::unique((row, i))` 重建临时输入框；GPUI 是 retained tree，输入框必须
/// 缓存，因此用一个**固定大小的编辑器池** + 每帧刷新的上下文表：第 `n` 个编辑器提交时
/// 读 `ctx[n]` 决定写到哪个命令/哪一行。`right_panel` 每帧按顺序认领槽位。
#[derive(Clone, Debug)]
pub(crate) struct RowEditCtx {
    pub cmd: &'static str,
    /// 提交时合并进 params 的固定部分（如 `{"index": 0}`）。
    pub params: Value,
    /// 用户输入的键名。
    pub key: &'static str,
    /// 数值提交时的缩放（opacity 用 100，百分数 → 0..1）。
    pub scale: f64,
    /// `true` 时把输入当字符串写（hex / suffix），否则解析成 f64。
    pub string_value: bool,
    /// `selection.setPaint` 的 opacity：egui 提交的是 `group.paint.clone()` 改 `opacity` 后的
    /// **完整 Paint 对象**（`{"targets", "paint"}`），顶层 `opacity` 键被引擎忽略。
    /// `Some(base)` 时数值写进 `base["opacity"]`，再赋给 `params["paint"]`。
    pub paint: Option<Value>,
    /// `selection.setPaint` 改颜色（hex 输入）：同样必须提交完整 Paint，只把 `color` 换掉。
    /// 传 `"paint": "#RRGGBB"` 虽然能被引擎解析成 solid，但会把这一组原有的 opacity 丢掉。
    /// `Some(base)` 且 `string_value` 时，文本写进 `base["color"]`，再赋给 `params["paint"]`。
    pub paint_color: Option<Value>,
    /// 写进 `params[nest][key]` 而不是 `params[key]`（`instance.setProperties` 要求
    /// `{"properties": {<属性名>: 值}}`）。
    pub nest: Option<&'static str>,
    /// `nest` 的键名需要是运行时字符串时用它（组件属性名是 `String`，不能 `&'static str`）。
    pub nest_key: Option<String>,
}

/// 一个 WGSL 参数输入框提交时所需的全部上下文。持有当前绑定快照，提交时只改这一个参数、
/// 重新序列化整条 binding 并推 `node.set {props:{pluginData}}`——避免多参数并发编辑互相覆盖。
#[derive(Clone, Debug)]
pub(crate) struct WgslParamEdit {
    pub binding: crate::wgsl_material::WgslMaterialBinding,
    /// uniform 路径，如 `U.gain`。
    pub field: String,
    /// 控件值域上下界（`wgsl_effect::ParamSpec`），提交时 clamp。
    pub min: f64,
    pub max: f64,
}

/// WGSL 参数输入框池的大小（一个 shader 很少超过 12 个可调参数）。
pub(crate) const WGSL_PARAM_FIELDS: usize = 12;

/// 池里一个编辑器的数量上限（paint 两列 × 多行 + export + selection-colors，留足余量）。
pub(crate) const ROW_EDITORS: usize = 48;

#[derive(Clone, Copy, Default)]
pub(crate) struct PanelSel {
    pub empty: bool,
    pub all_ellipse: bool,
    pub all_points: bool,
    pub has_text: bool,
    pub all_frame: bool,
    /// Whether every selected node has auto layout (mirrors egui `has_al`).
    pub has_al: bool,
    /// The common layout direction (`None` when not all in auto layout, or values differ → "Mixed").
    pub layout_dir: LayoutDir,
    /// Number of selected layers — egui `right.rs::position_fields` gates the
    /// "Distribute and tidy" button on `sel.len() > 2`.
    pub count: usize,
    /// Any selected node is a vector network — egui `paints.rs::paints` gates the
    /// Fill-rule button on `first_vector(doc, sel).is_some()`.
    pub has_vector: bool,
    /// egui `layout_panel.rs::layout_section`: an instance takes its auto layout from its main,
    /// so its header shows **no** add/remove button at all.
    pub in_instance: bool,
}

/// 选区标题行的判断输入（`lizard right.rs::header`）。全是纯值，方便单测锁住显隐规则。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeaderSel {
    pub is_frame: bool,
    pub is_group: bool,
    pub is_text: bool,
    pub is_component: bool,
    pub is_shape: bool,
    /// 矢量编辑已经选中了顶点 / 线段（lizard `points_selected`）。
    pub points_selected: bool,
    pub count: usize,
}

/// 标题行该画哪套东西（`lizard right.rs::header` 的分支结果）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeaderActions {
    /// 类型名带下拉（Frame / Group）。
    pub type_menu: bool,
    /// 单选 Frame 的「交换方向」按钮。
    pub swap_orientation: bool,
    /// Text 专属的四个按钮；与 `quick` 互斥。
    pub text: bool,
    /// 普通图层那套：Select matching / Component / Mask / Boolean / Edit object。
    pub quick: bool,
    /// 单选形状才有 Edit object（只在 `quick` 里）。
    pub edit_object: bool,
}

/// 右栏按钮的**显隐条件**，逐条照 `ui-egui` / `lizard` 源码。集中成纯函数，避免条件散在各处，
/// 也让「什么时候显示 / 什么时候隐藏」可以被单测锁住。
pub(crate) mod visibility {
    use super::{HeaderActions, HeaderSel, PanelSel};

    /// `lizard right.rs::header`：
    /// * Frame / Group → 类型名下拉；单选 Frame 另有「交换方向」。
    /// * 已选矢量点 → 只留类型名那部分，右侧动作一个都不画（`return`）。
    /// * Component / ComponentSet → 走组件专属分支，普通动作一个都不画。
    /// * 全是 Text → 四个 Text 专属按钮，普通动作一个都不画（`return`）。
    /// * 其余 → 普通图层动作；单选形状额外多一个 Edit object。
    pub fn header_actions(s: &HeaderSel) -> HeaderActions {
        let base = HeaderActions {
            type_menu: kind_menu(s.is_frame, s.is_group),
            swap_orientation: s.is_frame,
            ..HeaderActions::default()
        };
        if s.points_selected || s.is_component {
            return base;
        }
        if s.is_text {
            return HeaderActions { text: true, ..base };
        }
        HeaderActions { quick: true, edit_object: s.is_shape, ..base }
    }

    /// `lizard right.rs::header`：`bool_button.clicked() && sel.len() >= 2` 才执行 `bool.union`。
    pub fn bool_union_enabled(count: usize) -> bool {
        count >= 2
    }

    /// egui `layout_panel.rs::layout_section` 的 `btns`：
    /// `in_instance` → 空；`frames && !has_al` → `+`；`has_al` → `−`；其余 → 空。
    /// 返回该显示的命令 id（`None` = 这个按钮根本不该出现）。
    pub fn layout_header(sel: &PanelSel) -> Option<&'static str> {
        if sel.in_instance {
            return None;
        }
        if sel.all_frame && !sel.has_al {
            return Some("layout.add");
        }
        if sel.has_al {
            return Some("layout.remove");
        }
        None
    }

    /// egui `right.rs::position_fields`: `if sel.len() > 2 { icons::button(DistributeH, …) }`.
    pub fn distribute(sel: &PanelSel) -> bool {
        sel.count > 2
    }

    /// egui `paints.rs::paints`: `let vector = if fills { first_vector(doc, sel) } else { None };
    /// if vector.is_some() { buttons.push(FillRule) }`.
    pub fn fill_rule(fills: bool, has_vector: bool) -> bool {
        fills && has_vector
    }

    /// egui `right.rs::header`: 类型下拉只在 Frame / Group 上存在，其它类型连 chevron 都不画。
    pub fn kind_menu(is_frame: bool, is_group: bool) -> bool {
        is_frame || is_group
    }

    /// egui `section_header(..., empty)`: a header dims when the selection has no list at all
    /// (`None`) or the list it would show is empty (`Some(0)`). Kept as a pure rule for
    /// `visibility_tests`; the sections that call it land with the list rows.
    #[allow(dead_code)]
    pub fn dim_header(rows: Option<usize>) -> bool {
        matches!(rows, None | Some(0))
    }
}

pub(crate) struct RightFields {
    /// Commands the panel raised this frame.
    cmds: Cmds,
    x: Entity<TextField>,
    y: Entity<TextField>,
    w: Entity<TextField>,
    h: Entity<TextField>,
    rotation: Entity<TextField>,
    arc_start: Entity<TextField>,
    arc_sweep: Entity<TextField>,
    arc_ratio: Entity<TextField>,
    star_points: Entity<TextField>,
    star_ratio: Entity<TextField>,
    opacity: Entity<TextField>,
    corner: Entity<TextField>,
    font_size: Entity<TextField>,
    stroke_weight: Entity<TextField>,
    /// 排版数值字段（仅全部选中为文本时用到）：行高 / 字距 / 段落间距。
    line_height: Entity<TextField>,
    letter_spacing: Entity<TextField>,
    paragraph_spacing: Entity<TextField>,
    /// Scale 工具（K）的三个字段：W / H / 倍数；`scale_ctx` 是提交所需包围盒 + 支点。
    scale_w: Entity<TextField>,
    scale_h: Entity<TextField>,
    scale_k: Entity<TextField>,
    scale_ctx: std::rc::Rc<std::cell::RefCell<crate::panels::right::data::ScaleData>>,
    /// Frame 预设面板各分类的展开状态（egui 存在 `ui.data` 的临时值，默认只开 Phone）。
    preset_open: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>>,
    /// Selection colors 节的展开态，键是 `ds.uid`（lizard
    /// `Id::new(("selection_colors_open", ds.uid))`）：**每个文档各自记住**开合状态，
    /// 缺省即折叠；折叠时头部右侧显示最多三个色块 + `+N`。
    selection_colors_open: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<u64, bool>>>,
    /// 该节头部是否悬停（lizard 只在悬停时画 chevron、把标题提亮）。
    selection_colors_hover: std::rc::Rc<std::cell::RefCell<bool>>,
    /// 右栏当前标签：0 = Design，1 = Prototype，2 = Inspect（egui `app.ui.right_tab`）。纯 UI 状态，不进文档。
    right_tab: std::rc::Rc<std::cell::RefCell<u8>>,
    /// A3: the Comments panel's local state (filter / sort / search / open thread).
    comment_ui: crate::comments::CommentState,
    /// Inspect 标签的三个选择：代码语言下标、颜色格式下标、Web 语言是否用 rem。
    inspect_lang: std::rc::Rc<std::cell::RefCell<usize>>,
    inspect_fmt: std::rc::Rc<std::cell::RefCell<usize>>,
    inspect_rem: std::rc::Rc<std::cell::RefCell<bool>>,
    /// Prototype 面板里展开编辑的那条交互（行下标）；`None` = 都没展开。
    proto_open: std::rc::Rc<std::cell::RefCell<Option<usize>>>,
    /// WGSL 参数输入框池：retained tree 不能每帧新建，预建固定个数，按当前 shader 的参数数认领。
    wgsl_param_fields: Vec<Entity<TextField>>,
    /// 每个参数字段提交时的运行时上下文（当前绑定快照 + uniform 路径 + 控件范围）。
    wgsl_param_ctx: std::rc::Rc<std::cell::RefCell<Vec<Option<WgslParamEdit>>>>,
    /// Inspect 标签的 Dev Mode 纯 UI 状态：Ready for dev 的过滤器下标、注释分类过滤器
    /// （0 = 全部 / 1 = 无分类 / 2+i = 第 i 个分类）、开发资源「正在添加」与「正在编辑第几条」。
    dev_filter: std::rc::Rc<std::cell::RefCell<usize>>,
    dev_cat_filter: std::rc::Rc<std::cell::RefCell<usize>>,
    dev_res_add: std::rc::Rc<std::cell::RefCell<bool>>,
    dev_res_edit: std::rc::Rc<std::cell::RefCell<Option<usize>>>,
    /// Inspect 标签「Assets」里每个资产选中的导出格式下标（egui 的 `AssetUi::formats`，
    /// 按会话保留；`Vec` 避免要求 `NodeId: Ord`）。
    asset_fmt: std::rc::Rc<std::cell::RefCell<Vec<(figcraft_doc::NodeId, usize)>>>,
    /// 矢量点面板的五个数值字段（X / Y / W / H / 圆角半径）。
    vpt_x: Entity<TextField>,
    vpt_y: Entity<TextField>,
    vpt_w: Entity<TextField>,
    vpt_h: Entity<TextField>,
    vpt_radius: Entity<TextField>,
    /// Fill colour row: a colour dot (lead) + editable hex. Clicking the dot opens the colour picker.
    /// The `Field-color`'s hex value: a bare editor (`text_field_edit`), not a field.
    fill_hex: Entity<TextEdit>,
    /// Fill opacity at the end of the colour row (mirrors egui's `color_field` opacity field).
    /// The `Field-color`'s opacity number, in the trail's 44 pt alpha cell.
    fill_opacity: Entity<TextEdit>,
    /// The paint the fill row currently shows, so a re-render does not rewrite the same slots.
    /// `None` until the selection's fill has been read once.
    last_fill: std::cell::RefCell<Option<figcraft_color::Paint>>,
    /// Stroke colour row: a colour dot (lead) + editable hex, mirroring the fill row.
    stroke_hex: Entity<TextEdit>,
    /// Stroke opacity filler for the `ColorField` alpha cell (the main stroke row has none).
    stroke_opacity: Entity<TextEdit>,
    /// The paint the stroke row currently shows.
    last_stroke: std::cell::RefCell<Option<figcraft_color::Paint>>,
    /// Auto layout — component-backed subset (gap / padding / alignment). Rest is placeholder.
    gap: Entity<TextField>,
    cross_gap: Entity<TextField>,
    /// Dimensions 的 min/max（`layout_panel.rs::limit_row_v`）。
    dim_min_w: Entity<TextField>,
    dim_max_w: Entity<TextField>,
    dim_min_h: Entity<TextField>,
    dim_max_h: Entity<TextField>,
    pad_t: Entity<TextField>,
    pad_r: Entity<TextField>,
    pad_b: Entity<TextField>,
    pad_l: Entity<TextField>,
    /// 默认（非 individual）的 padding 两字段：水平 / 垂直（`paddingHorizontal` / `paddingVertical`）。
    pad_h: Entity<TextField>,
    pad_v: Entity<TextField>,
    /// 「独立内边距」开关（egui `indep_padding` 的临时标记）。
    pad_individual: std::rc::Rc<std::cell::RefCell<bool>>,
    /// Shared source of truth for the 4 padding values, so one field's edit re-pushes the whole array.
    pad: std::rc::Rc<std::cell::RefCell<[f64; 4]>>,
    /// Alignment dropdown current labels (serde names) for the main and cross axes.
    align_main_label: std::cell::RefCell<SharedString>,
    align_cross_label: std::cell::RefCell<SharedString>,
    /// 列表行的缓存编辑器池（paint hex / opacity、export suffix、selection-color hex …）。
    row_editors: Vec<Entity<TextEdit>>,
    /// 每个池槽位当前对应的提交目标；`right_panel` 每帧刷新。
    row_ctx: std::rc::Rc<std::cell::RefCell<Vec<Option<RowEditCtx>>>>,
    /// 上次写入池的显示值；文档未变时不重写，避免打断正在输入的内容。
    last_rows: std::cell::RefCell<Vec<Option<String>>>,
    /// Dimensions 的 min/max 行是否显示（`[minW, maxW, minH, maxH]`）。egui 用 `limit_open`
    /// 临时标记：只有已设值、或刚从 sizing 菜单里 Add 的行才显示，默认隐藏。
    limit_open: std::rc::Rc<std::cell::RefCell<[bool; 4]>>,
    /// The blend-mode dropdown's current label (egui's `dropdown` row). Clicking opens the menu.
    blend_label: std::cell::RefCell<SharedString>,
    /// Blend-mode row reveal (egui `paints.rs::appearance`'s `blend_open` local flag): the
    /// Appearance header's blend button toggles this, and the blend dropdown row shows only when set.
    blend_open: std::rc::Rc<std::cell::RefCell<bool>>,
    /// The root view, set once after construction so fields can open menus through it.
    root: Option<Entity<CanvasView>>,
    /// Shared slot for detached-window requests. Buttons put a `WindowState` here and the root
    /// view takes it on the next frame. This deliberately does **not** go through
    /// `Entity<CanvasView>`: a `let Some(root) = ... else { return }` guard silently swallows the
    /// click whenever the one-shot `set_root` did not land, which is the "button does nothing" bug.
    popup_slot: Rc<std::cell::RefCell<Option<crate::popovers::WindowState>>>,
    /// 全部编辑器实体（含所有 TextField 的内层 editor、颜色行 editor、列表行池）。
    /// 点画布时统一 `finish_edit`，保证草稿不会因为「焦点转移但视图没重绘」而丢失。
    editors: Vec<Entity<TextEdit>>,
    // 绑定态（`Field-color / Disabled`）不在这里存副本：`PanelData::PaintRow.bound` 是每帧从
    // Document 读出的权威快照，`right::sections::paints` 直接据此构造 `ColorField::bound()`。
}

/// Like [`nf_set`], but for line-height / letter-spacing style fields whose value may be `Auto`
/// (an empty edit → `"AUTO"`) or a percent (`"120%"`). Routes through the command channel.
fn nf_spacing(
    cx: &mut App,
    lead: &'static str,
    step: f64,
    key: &'static str,
    cmds: &Cmds,
    wake: &std::rc::Rc<dyn Fn()>,
) -> Entity<TextField> {
    let cmds = cmds.clone();
    let wake = wake.clone();
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric("", step);
        tf.set_on_commit(move |text, _window, _cx| {
            let t = text.trim();
            let val = if t.is_empty() {
                serde_json::json!("AUTO")
            } else if let Ok(v) = t.trim_end_matches('%').trim().parse::<f64>() {
                if t.ends_with('%') {
                    serde_json::json!(format!("{}%", v))
                } else {
                    serde_json::json!(v)
                }
            } else {
                return;
            };
            cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { key: val } })) });
            wake();
        });
    })
}

/// A labelled number field: the label is a leading [`Slot::Text`], the unit a suffix.
/// `set_fill(true)` makes it stretch to its grid cell (`1fr`/fill) instead of a fixed width.
/// A `Some(prop)` wires the field to `node.set` (panel → canvas).
#[allow(clippy::too_many_arguments)]
fn nf(
    cx: &mut App,
    lead: &'static str,
    val: f64,
    suffix: &str,
    step: f64,
    prop: Option<Prop>,
    pending: &Pending,
    wake: &std::rc::Rc<dyn Fn()>,
) -> Entity<TextField> {
    let shown = format!("{val}");
    let suffix = SharedString::from(suffix.to_string());
    let pending = pending.clone();
    let wake = wake.clone();
    // `number_field(..., col, ...)`: the width is the panel's column, never inferred.
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_value(shown);
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric(suffix, step);
        if let Some(prop) = prop {
            let (lo, hi) = prop.range();
            tf.set_range(lo, hi);
            tf.set_on_commit(move |text, _window, _cx| {
                if let Ok(v) = text.trim().parse::<f64>() {
                    pending.borrow_mut().push((prop, v));
                    wake();
                }
            });
        }
    })
}

/// Like [`nf`], but writes a single `node.set {"<key>": value}` on commit — for auto-layout scalar
/// props (e.g. `gap`) that have no dedicated `Prop` variant. Routes through the command channel.
fn nf_set(
    cx: &mut App,
    lead: &'static str,
    val: f64,
    suffix: &str,
    step: f64,
    key: &'static str,
    cmds: &Cmds,
    wake: &std::rc::Rc<dyn Fn()>,
) -> Entity<TextField> {
    let shown = format!("{val}");
    let suffix = SharedString::from(suffix.to_string());
    let cmds = cmds.clone();
    let wake = wake.clone();
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_value(shown);
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric(suffix, step);
        tf.set_on_commit(move |text, _window, _cx| {
            if let Ok(v) = text.trim().parse::<f64>() {
                // `node.set` reads props from the nested `props` object; `Cmd.params` is merged
                // at top level by `sync_panel`, so wrap the key here.
                cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { key: v } })) });
                wake();
            }
        });
    })
}

/// A field that pushes one command with `{key: value}` (vector point edits: `vector.edit.position`
/// / `.resize` / `.cornerRadius` all take the edited key and act on the current point selection).
fn nf_cmd(cx: &mut App, lead: &'static str, step: f64, cmd: &'static str, key: &'static str, min_zero: bool, cmds: &Cmds) -> Entity<TextField> {
    let cmds = cmds.clone();
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric("", step);
        let cmds = cmds.clone();
        tf.set_on_commit(move |text, _window, _cx| {
            let Ok(v) = text.trim().parse::<f64>() else { return };
            let v = if min_zero { v.max(0.0) } else { v };
            cmds.push(Cmd { id: cmd, params: Some(serde_json::json!({ key: v })) });
        });
    })
}

/// One of the Scale tool's three fields (egui `scale_panel.rs`).
///
/// `axis`: 0 = W, 1 = H, 2 = factor. W/H divide by the current bounds to get a factor; all three
/// then push `transform.scale {factor, cx, cy}` — the pivot comes from the anchor that the canvas
/// and `tool.scaleAnchor` share, so strokes / effects / radii scale with the layers.
fn nf_scale(
    cx: &mut App,
    lead: &'static str,
    step: f64,
    cmds: &Cmds,
    ctx: std::rc::Rc<std::cell::RefCell<crate::panels::right::data::ScaleData>>,
    axis: u8,
) -> Entity<TextField> {
    let cmds = cmds.clone();
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric("", step);
        // 倍数字段从 1 起（相对当前尺寸）；W / H 每帧由 `sections::scale` 灌入当前包围盒。
        if axis == 2 {
            tf.set_value("1");
        }
        let ctx = ctx.clone();
        let cmds = cmds.clone();
        tf.set_on_commit(move |text, _window, _cx| {
            let Ok(v) = text.trim().parse::<f64>() else { return };
            let c = *ctx.borrow();
            let factor = match axis {
                0 => {
                    if c.w <= 1e-9 {
                        return;
                    }
                    v / c.w
                }
                1 => {
                    if c.h <= 1e-9 {
                        return;
                    }
                    v / c.h
                }
                _ => v,
            };
            // `scale_about`: 非有限 / 非正 / 等于 1 都不产生命令（egui 同款过滤）。
            if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() <= 1e-9 {
                return;
            }
            let factor = factor.clamp(0.01, 100.0);
            cmds.push(Cmd {
                id: "transform.scale",
                params: Some(serde_json::json!({ "factor": factor, "cx": c.cx, "cy": c.cy })),
            });
        });
    })
}

/// One padding field (one of the 4 sides). On commit it updates its slot in the shared array and
/// re-pushes `node.set {"padding": [t, r, b, l]}` (serde order, matching egui's `padding_row`).
fn nf_pad(
    cx: &mut App,
    lead: &'static str,
    idx: usize,
    pad: &std::rc::Rc<std::cell::RefCell<[f64; 4]>>,
    cmds: &Cmds,
    wake: &std::rc::Rc<dyn Fn()>,
) -> Entity<TextField> {
    let cmds = cmds.clone();
    let wake = wake.clone();
    let pad2 = pad.clone();
    let pad_commit = pad.clone();
    TextField::build(cx, DEFAULT_COL, move |tf, _| {
        tf.set_value(format!("{}", pad2.borrow()[idx]));
        tf.set_lead(Slot::Text(SharedString::from(lead)));
        tf.set_numeric("", 1.0);
        tf.set_on_commit(move |text, _window, _cx| {
            if let Ok(v) = text.trim().parse::<f64>() {
                let mut arr = pad_commit.borrow().clone();
                arr[idx] = v;
                *pad_commit.borrow_mut() = arr;
                cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { "padding": arr.to_vec() } })) });
                wake();
            }
        });
    })
}

impl RightFields {
    pub(crate) fn new(cx: &mut App, pending: &Pending, wake: &std::rc::Rc<dyn Fn()>) -> Self {
        let cmds: Cmds = CmdQueue::new(wake);
        let n = |cx: &mut App, lead: &'static str, val: f64, suffix: &str, step: f64, prop: Option<Prop>| {
            nf(cx, lead, val, suffix, step, prop, pending, wake)
        };
        let pad_rc = std::rc::Rc::new(std::cell::RefCell::new([10.0; 4]));
        // 列表行编辑器池：每个槽位提交时读 `row_ctx[slot]` 决定命令与行号。
        let row_ctx: std::rc::Rc<std::cell::RefCell<Vec<Option<RowEditCtx>>>> =
            std::rc::Rc::new(std::cell::RefCell::new((0..ROW_EDITORS).map(|_| None).collect()));
        let mut row_editors = Vec::with_capacity(ROW_EDITORS);
        for slot in 0..ROW_EDITORS {
            let ctx = row_ctx.clone();
            let cmds = cmds.clone();
            row_editors.push(TextEdit::build(cx, move |te, _| {
                te.set_right_pad(8.0);
                te.set_on_commit(move |text, _window, _cx| {
                    let Some(c) = ctx.borrow().get(slot).cloned().flatten() else { return };
                    let mut params = c.params.clone();
                    // `nest` = 写进子对象（`instance.setProperties` 的 `properties`）。
                    let put = |params: &mut Value, key: &str, v: Value| match c.nest {
                        Some(nest) => {
                            let key = c.nest_key.as_deref().unwrap_or(key);
                            if !params[nest].is_object() {
                                params[nest] = serde_json::json!({});
                            }
                            params[nest][key] = v;
                        }
                        None => params[key] = v,
                    };
                    if c.string_value {
                        if let Some(mut paint) = c.paint_color.clone() {
                            paint["color"] = Value::String(text.trim().to_string());
                            params["paint"] = paint;
                        } else {
                            put(&mut params, c.key, Value::String(text.trim().to_string()));
                        }
                    } else if let Ok(v) = text.trim().trim_end_matches('%').trim().parse::<f64>() {
                        if let Some(mut paint) = c.paint.clone() {
                            // egui `selection_colors.rs`: clone the group's paint, change its
                            // opacity, and send the whole object as `paint`.
                            paint["opacity"] = serde_json::json!(v / c.scale);
                            params["paint"] = paint;
                        } else {
                            put(&mut params, c.key, serde_json::json!(v / c.scale));
                        }
                    } else {
                        return;
                    }
                    // `CmdQueue::push` wakes the view; no separate wake() is needed here.
                    cmds.push(Cmd { id: c.cmd, params: Some(params) });
                });
            }));
        }
        // WGSL 材质参数输入框池：retained tree 不能每帧新建 Entity。每帧由
        // `sections::effects` 认领槽位（写入 `wgsl_param_ctx[slot]`），提交时只改一个参数、
        // 重新序列化整条 binding 并推 `node.set`，避免多参数并发编辑互相覆盖。
        let wgsl_param_ctx: std::rc::Rc<std::cell::RefCell<Vec<Option<WgslParamEdit>>>> =
            std::rc::Rc::new(std::cell::RefCell::new((0..WGSL_PARAM_FIELDS).map(|_| None).collect()));
        let mut wgsl_param_fields = Vec::with_capacity(WGSL_PARAM_FIELDS);
        for slot in 0..WGSL_PARAM_FIELDS {
            let ctx = wgsl_param_ctx.clone();
            let cmds = cmds.clone();
            wgsl_param_fields.push(TextField::build(cx, DEFAULT_COL, move |tf, _| {
                tf.set_numeric("", 1.0);
                let ctx = ctx.clone();
                let cmds = cmds.clone();
                tf.set_on_commit(move |text, _window, _cx| {
                    // 未认领的槽位（shader 参数数 < 池大小）在切换绑定后不能沿用旧目标。
                    let Some(Some(e)) = ctx.borrow().get(slot).cloned() else { return };
                    let Ok(mut v) = text.trim().parse::<f64>() else { return };
                    if e.min <= e.max {
                        v = v.clamp(e.min, e.max);
                    }
                    let mut binding = e.binding;
                    binding.params.insert(e.field, v);
                    cmds.push(Cmd { id: "node.set", params: Some(crate::wgsl_material::write_binding_params(Some(&binding))) });
                });
            }));
        }
        let scale_ctx = std::rc::Rc::new(std::cell::RefCell::new(crate::panels::right::data::ScaleData::default()));
        let mut fields = Self {
            x: n(cx, "X", 0.0, "", 1.0, Some(Prop::X)),
            y: n(cx, "Y", 0.0, "", 1.0, Some(Prop::Y)),
            w: n(cx, "W", 100.0, "", 1.0, Some(Prop::W)),
            h: n(cx, "H", 100.0, "", 1.0, Some(Prop::H)),
            rotation: n(cx, tr("R"), 0.0, "°", 1.0, Some(Prop::Rotation)),
            arc_start: n(cx, "S", 0.0, "°", 1.0, Some(Prop::ArcStart)),
            arc_sweep: n(cx, "W", 360.0, "°", 1.0, Some(Prop::ArcSweep)),
            arc_ratio: n(cx, tr("R"), 100.0, "%", 1.0, Some(Prop::ArcInner)),
            star_points: n(cx, "P", 5.0, "", 1.0, Some(Prop::PointCount)),
            star_ratio: n(cx, tr("R"), 50.0, "%", 1.0, Some(Prop::StarRatio)),
            opacity: n(cx, "O", 100.0, "%", 1.0, Some(Prop::Opacity)),
            corner: n(cx, "C", 0.0, "", 1.0, Some(Prop::Corner)),
            font_size: n(cx, "S", 16.0, "", 1.0, Some(Prop::FontSize)),
            stroke_weight: n(cx, "W", 1.0, "", 1.0, Some(Prop::StrokeWeight)),
            // 排版数值字段：行高 / 字距支持 Auto（空→"AUTO"）与百分比；段落间距纯 px。
            line_height: nf_spacing(cx, "Lh", 1.0, "lineHeight", &cmds, wake),
            letter_spacing: nf_spacing(cx, "Ls", 0.1, "letterSpacing", &cmds, wake),
            paragraph_spacing: nf_set(cx, "P", 0.0, "", 1.0, "paragraphSpacing", &cmds, wake),
            // Scale 工具的三字段：W / H 按当前包围盒换算倍数，倍数直接提交。
            scale_w: nf_scale(cx, "W", 1.0, &cmds, scale_ctx.clone(), 0),
            scale_h: nf_scale(cx, "H", 1.0, &cmds, scale_ctx.clone(), 1),
            scale_k: nf_scale(cx, "×", 0.1, &cmds, scale_ctx.clone(), 2),
            scale_ctx,
            preset_open: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            selection_colors_open: std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new())),
            selection_colors_hover: std::rc::Rc::new(std::cell::RefCell::new(false)),
            right_tab: std::rc::Rc::new(std::cell::RefCell::new(0u8)),
            comment_ui: std::rc::Rc::new(std::cell::RefCell::new(crate::comments::CommentUi::default())),
            inspect_lang: std::rc::Rc::new(std::cell::RefCell::new(0usize)),
            inspect_fmt: std::rc::Rc::new(std::cell::RefCell::new(0usize)),
            inspect_rem: std::rc::Rc::new(std::cell::RefCell::new(false)),
            proto_open: std::rc::Rc::new(std::cell::RefCell::new(None)),
            // WGSL 材质（另一条并行开发线新增的字段）：这里先给可编译的默认值，
            // 参数输入框池由该功能自己按需预建。
            wgsl_param_fields,
            wgsl_param_ctx,
            dev_filter: std::rc::Rc::new(std::cell::RefCell::new(0usize)),
            dev_cat_filter: std::rc::Rc::new(std::cell::RefCell::new(0usize)),
            dev_res_add: std::rc::Rc::new(std::cell::RefCell::new(false)),
            dev_res_edit: std::rc::Rc::new(std::cell::RefCell::new(None)),
            asset_fmt: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            // 矢量点面板：命令由引擎按当前 `vector_edit` 的选区执行，参数只带值。
            vpt_x: nf_cmd(cx, "X", 1.0, "vector.edit.position", "x", false, &cmds),
            vpt_y: nf_cmd(cx, "Y", 1.0, "vector.edit.position", "y", false, &cmds),
            vpt_w: nf_cmd(cx, "W", 1.0, "vector.edit.resize", "width", true, &cmds),
            vpt_h: nf_cmd(cx, "H", 1.0, "vector.edit.resize", "height", true, &cmds),
            vpt_radius: nf_cmd(cx, tr("R"), 1.0, "vector.edit.cornerRadius", "radius", true, &cmds),
            // Fill colour row: `color_input` = ONE `Field-color` shell, and its hex and opacity are
            // bare editors (`text_field_edit`) — not fields of their own.
            fill_hex: TextEdit::build(cx, |te, _| {
                te.set_value("#3366FF");
                // `field_editor`: the value stops 8 pt short of the cell's end.
                te.set_right_pad(8.0);
            }),
            // Fill opacity (%): bound to `opacity` like the Appearance row's opacity field.
            fill_opacity: TextEdit::build(cx, |te, _cx| {
                let pending = pending.clone();
                let wake = wake.clone();
                te.set_numeric("", 1.0);
                // 透明度只允许 0..100：打字、方向键、24 pt "%" 拖拽、文档回推都夹在这里。
                te.set_range(Some(0.0), Some(100.0));
                te.set_value("100");
                // `paint_alpha`: the number is right-aligned in its 44 pt cell.
                te.set_align(crate::widgets::input::TextAlign::Right);
                te.set_on_commit(move |text, _window, _cx| {
                    if let Ok(v) = text.trim().parse::<f64>() {
                        pending.borrow_mut().push((Prop::Opacity, v));
                        wake();
                    }
                });
            }),
            // Stroke colour row: `color_input` = ONE `Field-color` shell; its hex is a bare editor.
            stroke_hex: TextEdit::build(cx, |te, _| {
                te.set_value("#000000");
                te.set_right_pad(8.0);
            }),
            // Stroke opacity filler (unused in the main row; only satisfies ColorField's alpha cell).
            stroke_opacity: TextEdit::build(cx, |te, _cx| {
                te.set_numeric("", 1.0);
                te.set_range(Some(0.0), Some(100.0));
                te.set_value("100");
                te.set_align(crate::widgets::input::TextAlign::Right);
            }),
            // ── Auto layout：gap / padding[4] / 主轴+交叉轴对齐（用现成 TextField + Dropdown）──
            gap: nf_set(cx, tr("Gap"), 10.0, "", 1.0, "gap", &cmds, wake),
            cross_gap: nf_set(cx, tr("Gap"), 10.0, "", 1.0, "crossGap", &cmds, wake),
            dim_min_w: nf_set(cx, tr("Min W"), 0.0, "", 1.0, "minWidth", &cmds, wake),
            dim_max_w: nf_set(cx, tr("Max W"), 0.0, "", 1.0, "maxWidth", &cmds, wake),
            dim_min_h: nf_set(cx, tr("Min H"), 0.0, "", 1.0, "minHeight", &cmds, wake),
            dim_max_h: nf_set(cx, tr("Max H"), 0.0, "", 1.0, "maxHeight", &cmds, wake),
            pad: pad_rc.clone(),
            pad_t: nf_pad(cx, "T", 0, &pad_rc, &cmds, wake),
            pad_r: nf_pad(cx, tr("R"), 1, &pad_rc, &cmds, wake),
            pad_b: nf_pad(cx, "B", 2, &pad_rc, &cmds, wake),
            pad_l: nf_pad(cx, "L", 3, &pad_rc, &cmds, wake),
            pad_h: nf_set(cx, "H", 10.0, "", 1.0, "paddingHorizontal", &cmds, wake),
            pad_v: nf_set(cx, "V", 10.0, "", 1.0, "paddingVertical", &cmds, wake),
            pad_individual: std::rc::Rc::new(std::cell::RefCell::new(false)),
            align_main_label: std::cell::RefCell::new(SharedString::from("min")),
            align_cross_label: std::cell::RefCell::new(SharedString::from("min")),
            // Blend-mode dropdown: a button that opens the blend menu on click (egui `dropdown`).
            blend_label: std::cell::RefCell::new(SharedString::from(tr("Normal"))),
            blend_open: std::rc::Rc::new(std::cell::RefCell::new(false)),
            root: None,
            popup_slot: Rc::new(std::cell::RefCell::new(None)),
            last_fill: std::cell::RefCell::new(None),
            last_stroke: std::cell::RefCell::new(None),
            row_editors,
            row_ctx,
            last_rows: std::cell::RefCell::new((0..ROW_EDITORS).map(|_| None).collect()),
            limit_open: std::rc::Rc::new(std::cell::RefCell::new([false; 4])),
            cmds,
            editors: Vec::new(),
        };
        // 点画布时统一失焦提交：收集所有缓存的编辑器实体（TextField 的内层 + 颜色行 + 行池）。
        let mut editors: Vec<Entity<TextEdit>> = [
            &fields.x,
            &fields.y,
            &fields.w,
            &fields.h,
            &fields.rotation,
            &fields.arc_start,
            &fields.arc_sweep,
            &fields.arc_ratio,
            &fields.star_points,
            &fields.star_ratio,
            &fields.opacity,
            &fields.corner,
            &fields.font_size,
            &fields.stroke_weight,
            &fields.line_height,
            &fields.letter_spacing,
            &fields.paragraph_spacing,
            &fields.scale_w,
            &fields.scale_h,
            &fields.scale_k,
            &fields.gap,
            &fields.cross_gap,
            &fields.pad_t,
            &fields.pad_r,
            &fields.pad_b,
            &fields.pad_l,
        ]
        .iter()
        .map(|f| f.read(cx).edit())
        .collect();
        editors.push(fields.fill_hex.clone());
        editors.push(fields.fill_opacity.clone());
        editors.push(fields.stroke_hex.clone());
        editors.push(fields.stroke_opacity.clone());
        editors.extend(fields.row_editors.iter().cloned());
        // WGSL 参数输入框也要在失焦 / 点画布时统一提交草稿。
        editors.extend(fields.wgsl_param_fields.iter().map(|tf| tf.read(cx).edit()));
        fields.editors = editors;
        fields
    }

    /// 点画布 / 任何整窗失焦：把仍在编辑的草稿统一提交。
    /// `TextEdit` 自己也订阅了 focus-out，这里是双保险，保证画布点击当帧就写回文档。
    pub(crate) fn commit_all(&self, window: &mut Window, cx: &mut App) {
        for e in &self.editors {
            e.update(cx, |e, cx| {
                e.finish_edit(window, cx);
            });
        }
    }

    /// Take the commands raised since the last frame.
    pub(crate) fn take_cmds(&self) -> Vec<Cmd> {
        self.cmds.drain()
    }

    /// 通用窗口弹层也走同一条命令通道（点选一行/一个设置立刻唤醒下一帧执行）。
    /// The canvas view, for opening a menu from a panel control (the comments filter).
    pub(crate) fn root(&self) -> Option<Entity<CanvasView>> {
        self.root.clone()
    }

    /// A3: the Comments panel's local state.
    pub(crate) fn comment_ui(&self) -> crate::comments::CommentState {
        self.comment_ui.clone()
    }

    pub(crate) fn queue(&self) -> Cmds {
        self.cmds.clone()
    }

    /// Inspect 标签的三个选择：代码语言下标、颜色格式下标、Web 语言是否用 rem。
    pub(crate) fn inspect_sel(&self) -> (usize, usize, bool) {
        (*self.inspect_lang.borrow(), *self.inspect_fmt.borrow(), *self.inspect_rem.borrow())
    }

    /// 右栏当前标签（0 = Design / 1 = Prototype / 2 = Inspect）。画布的 Dev Mode 行为要读它。
    pub(crate) fn right_tab(&self) -> u8 {
        *self.right_tab.borrow()
    }

    /// 引擎 `ui_requests` 切右栏标签（panel.design / panel.prototype / panel.devMode）。
    pub(crate) fn set_right_tab(&self, tab: u8) {
        if *self.right_tab.borrow() != tab {
            *self.right_tab.borrow_mut() = tab;
            self.cmds.wake();
        }
    }

    /// Inspect 标签 Assets 行的「导出格式」本地选择表（egui `AssetUi::formats`）。
    pub(crate) fn asset_formats(&self) -> std::rc::Rc<std::cell::RefCell<Vec<(figcraft_doc::NodeId, usize)>>> {
        self.asset_fmt.clone()
    }

    /// 「画布上显示哪些注解」的过滤器下标：0 全部 / 1 无分类 / 2+i 第 i 个分类。
    /// 翻成 category id 由 CanvasView 拿着 doc 的分类表做（右栏不持有 doc）。
    pub(crate) fn dev_cat_filter_index(&self) -> usize {
        *self.dev_cat_filter.borrow()
    }

    /// Ready for dev 列表的过滤器下标（egui `FILTERS` 的四项）。
    pub(crate) fn dev_ready_filter(&self) -> usize {
        (*self.dev_filter.borrow()).min(3)
    }

    /// 每帧开头清空池的上下文（未认领的槽位不能沿用上一帧的目标）。
    pub(crate) fn begin_rows(&self) {
        for c in self.row_ctx.borrow_mut().iter_mut() {
            *c = None;
        }
        // WGSL 参数池同理：切换到参数更少的材质时，多余槽位不能沿用上一次的提交目标。
        for c in self.wgsl_param_ctx.borrow_mut().iter_mut() {
            *c = None;
        }
    }

    /// 认领一个 WGSL 材质参数输入框：绑定本帧的提交目标（当前 binding 快照 + uniform 路径 +
    /// 值域），并把该参数当前的显示值 / 标签 / 步长 / 范围推入池里的 `TextField`。
    ///
    /// 提交时只改这一个参数、把整条 binding 重新序列化推 `node.set`，所以另一个参数并发编辑
    /// 时不会互相覆盖（彼此持有的是同一帧的完整快照，引擎按顺序 apply）。值每帧都推：
    /// `TextEdit::set_value` 对相同值恒为 no-op，且用户正在输入的草稿不会被覆盖。
    pub(crate) fn wgsl_param_field(
        &self,
        cx: &mut App,
        slot: usize,
        spec: &figcraft_render::wgsl_effect::ParamSpec,
        value: f64,
        binding: &crate::wgsl_material::WgslMaterialBinding,
    ) -> Entity<TextField> {
        let slot = slot.min(WGSL_PARAM_FIELDS - 1);
        if let Some(c) = self.wgsl_param_ctx.borrow_mut().get_mut(slot) {
            *c = Some(WgslParamEdit {
                binding: binding.clone(),
                field: spec.field.clone(),
                min: spec.min,
                max: spec.max,
            });
        }
        if let Some(e) = self.wgsl_param_fields.get(slot) {
            let shown = format!("{}", value);
            let step = spec.step;
            let (lo, hi) = (spec.min, spec.max);
            e.update(cx, |tf, _| {
                // 参数的名称画在输入框**前面**（`wgsl_material_rows` 的独立文本列），
                // 不再塞进输入框的 lead 槽位 —— 塞进去时名字和数值挤在同一个框里。
                tf.set_lead(Slot::None);
                tf.set_value(shown);
                tf.set_numeric("", step);
                tf.set_range(Some(lo), Some(hi));
            });
        }
        self.wgsl_param_fields[slot].clone()
    }

    /// 认领一个列表行编辑器：绑定本帧的提交目标，且只在显示值变化时写入。
    /// 返回可用于 `ColorField` 等组件的 `TextEdit` 实体。
    pub(crate) fn row_field(&self, cx: &mut App, slot: usize, value: &str, ctx: RowEditCtx) -> Entity<TextEdit> {
        if let Some(c) = self.row_ctx.borrow_mut().get_mut(slot) {
            *c = Some(ctx);
        }
        let mut last = self.last_rows.borrow_mut();
        if last.get(slot).and_then(|v| v.as_deref()) != Some(value) {
            if let Some(e) = self.row_editors.get(slot) {
                let v = value.to_string();
                e.update(cx, |te, _| te.set_value(v.clone()));
            }
            if last.len() <= slot {
                last.resize(slot + 1, None);
            }
            last[slot] = Some(value.to_string());
        }
        self.row_editors[slot].clone()
    }

    /// Push the selected nodes' common values into the cached fields (selection → panel).
    /// `None` = mixed / no value, shown as "Mixed" like egui's `number_field_mixed`.
    pub(crate) fn set_values(&self, cx: &mut App, v: &[Option<f64>; 14]) {
        let items: [(&Entity<TextField>, Option<f64>); 14] = [
            (&self.x, v[0]),
            (&self.y, v[1]),
            (&self.w, v[2]),
            (&self.h, v[3]),
            (&self.rotation, v[4]),
            (&self.opacity, v[5]),
            (&self.corner, v[6]),
            (&self.arc_start, v[7]),
            (&self.arc_sweep, v[8]),
            (&self.arc_ratio, v[9]),
            (&self.star_points, v[10]),
            (&self.star_ratio, v[11]),
            (&self.font_size, v[12]),
            (&self.stroke_weight, v[13]),
        ];
        for (e, val) in items {
            e.update(cx, |tf, _| match val {
                Some(val) => {
                    tf.set_placeholder("");
                    tf.set_value(fmt_num(val));
                }
                None => {
                    tf.set_value("");
                    tf.set_placeholder(tr("Mixed"));
                }
            });
        }
    }

    /// Sync the typography number fields from the panel snapshot (called each frame while the
    /// selection is all-text). `None` blanks the field: the on-commit maps an empty line-height /
    /// letter-spacing edit back to `"AUTO"`; paragraph spacing and font size use the "Mixed" hint.
    pub(crate) fn set_text_values(&self, cx: &mut App, t: &crate::panels::right::data::TextData) {
        let mut set_spacing = |e: &Entity<TextField>, val: Option<(f64, bool)>| {
            e.update(cx, |tf, _| match val {
                Some((v, false)) => {
                    tf.set_placeholder("");
                    tf.set_value(format!("{v}"));
                }
                Some((v, true)) => {
                    tf.set_placeholder("");
                    tf.set_value(format!("{v}%"));
                }
                None => {
                    tf.set_value("");
                    tf.set_placeholder(tr("Auto"));
                }
            });
        };
        set_spacing(&self.line_height, t.line_height);
        set_spacing(&self.letter_spacing, t.letter_spacing);
        self.paragraph_spacing.update(cx, |tf, _| match t.paragraph_spacing {
            Some(v) => {
                tf.set_placeholder("");
                tf.set_value(format!("{v}"));
            }
            None => {
                tf.set_value("");
                tf.set_placeholder(tr("Mixed"));
            }
        });
        // 字号走 `Prop::FontSize` 的 retained 字段，这里从快照回填（几何链只把它设成 None）。
        self.font_size.update(cx, |tf, _| match t.font_size {
            Some(v) => {
                tf.set_placeholder("");
                tf.set_value(format!("{v}"));
            }
            None => {
                tf.set_value("");
                tf.set_placeholder(tr("Mixed"));
            }
        });
    }

    /// Sync the vector-point panel's fields (X / Y / W / H / radius) from the snapshot.
    /// `None` shows "Mixed" like egui's `number_field_mixed`.
    pub(crate) fn set_vector_values(&self, cx: &mut App, v: &crate::panels::right::data::VectorData) {
        fn set_opt(cx: &mut App, e: &Entity<TextField>, val: Option<f64>) {
            e.update(cx, |tf, _| match val {
                Some(n) => {
                    tf.set_placeholder("");
                    tf.set_value(format!("{n}"));
                }
                None => {
                    tf.set_value("");
                    tf.set_placeholder(tr("Mixed"));
                }
            });
        }
        set_opt(cx, &self.vpt_x, v.x);
        set_opt(cx, &self.vpt_y, v.y);
        set_opt(cx, &self.vpt_w, v.w);
        set_opt(cx, &self.vpt_h, v.h);
        set_opt(cx, &self.vpt_radius, v.radius);
    }

    /// Keep the root view reachable from every frame's button callbacks. `set_root` runs once at
    /// startup; if that update never landed (window not ready, `update` returned Err), every
    /// `let Some(root) = ... else { return }` guard in the panel silently swallows the click —
    /// the button looks connected but does nothing. Re-asserting it once per frame makes those
    /// guards harmless. Only fills the hole; it never replaces a root that is already wired.
    pub(crate) fn ensure_root(&mut self, root: Entity<CanvasView>) {
        if self.root.is_none() {
            self.root = Some(root);
        }
    }

    /// Handle for button callbacks that open one of the five detached windows.
    pub(crate) fn popups(&self) -> PopupReqs {
        PopupReqs { slot: self.popup_slot.clone(), cmds: self.cmds.clone() }
    }

    /// Take the window a button requested; the root view checks this once per frame.
    pub(crate) fn take_window(&self) -> Option<crate::popovers::WindowState> {
        self.popup_slot.borrow_mut().take()
    }

    /// Wire the popover-triggering slots. Called once after `CanvasView` exists, so the field
    /// callbacks can reach the root view to open menus / colour pickers. Mirrors egui's
    /// `paints.rs` blend dropdown and `color_field` dot.
    pub(crate) fn set_root(&mut self, root: Entity<CanvasView>, cx: &mut App) {
        self.root = Some(root.clone());

        // Fill colour row: the swatch's click opens the picker (`ColorField::on_swatch`, wired in
        // `right_panel`); here only the hex's commit needs the root — editing it writes the paint.
        let hex = self.fill_hex.clone();
        let root_h = root.clone();
        hex.update(cx, move |te, _| {
            let root_commit = root_h.clone();
            te.set_on_commit(move |text, _w, cx| {
                if let Some(c) = Color32::from_hex(text) {
                    root_commit.update(cx, |cv, cx| cv.apply_fill(c, cx));
                }
            });
        });

        // Stroke colour row: editing the hex writes the stroke paint (egui `color_field` dot/hex).
        let shex = self.stroke_hex.clone();
        let sroot = root.clone();
        shex.update(cx, move |te, _| {
            let root_commit = sroot.clone();
            te.set_on_commit(move |text, _w, cx| {
                if let Some(c) = Color32::from_hex(text) {
                    root_commit.update(cx, |cv, cx| cv.apply_stroke(c, cx));
                }
            });
        });
    }

    /// Push the panel's column width into every number field (`col_width(w)` on each frame in
    /// `ui-egui`; here once per resize).
    pub(crate) fn set_col(&self, col: f32, cx: &mut App) {
        let items = [
            &self.x,
            &self.y,
            &self.w,
            &self.h,
            &self.rotation,
            &self.arc_start,
            &self.arc_sweep,
            &self.arc_ratio,
            &self.star_points,
            &self.star_ratio,
            &self.opacity,
            &self.corner,
            &self.font_size,
            &self.stroke_weight,
            &self.gap,
            &self.cross_gap,
            &self.dim_min_w,
            &self.dim_max_w,
            &self.dim_min_h,
            &self.dim_max_h,
            &self.pad_t,
            &self.pad_r,
            &self.pad_b,
            &self.pad_l,
            &self.pad_h,
            &self.pad_v,
        ];
        for e in items {
            e.update(cx, |tf, _| tf.set_width(col));
        }
    }

    /// Push the selection's common auto-layout values into the cached fields (selection → panel).
    /// `None` = mixed / no value, shown as "Mixed" like egui's `number_field_mixed`.
    /// `main_align` / `cross_align` are the serde names shown in the alignment dropdowns.
    pub(crate) fn set_layout_values(
        &self,
        cx: &mut App,
        gap: Option<f64>,
        cross_gap: Option<f64>,
        padding: Option<[f64; 4]>,
        main_align: Option<&'static str>,
        cross_align: Option<&'static str>,
    ) {
        let mut fmt = |e: &Entity<TextField>, v: Option<f64>| {
            e.update(cx, |tf, _| match v {
                Some(val) => {
                    tf.set_placeholder("");
                    tf.set_value(fmt_num(val));
                }
                None => {
                    tf.set_value("");
                    tf.set_placeholder(tr("Mixed"));
                }
            });
        };
        fmt(&self.gap, gap);
        fmt(&self.cross_gap, cross_gap);
        fmt(&self.pad_t, padding.map(|p| p[0]));
        fmt(&self.pad_r, padding.map(|p| p[1]));
        fmt(&self.pad_b, padding.map(|p| p[2]));
        fmt(&self.pad_l, padding.map(|p| p[3]));
        // 默认视图的两个字段：左右相等才有值（否则 Mixed），上下同理（egui `spacing`）。
        fmt(&self.pad_h, padding.and_then(|p| (p[1] == p[3]).then_some(p[1])));
        fmt(&self.pad_v, padding.and_then(|p| (p[0] == p[2]).then_some(p[0])));
        // Keep the shared padding array in sync, so a later edit re-pushes all four current sides.
        if let Some(p) = padding {
            *self.pad.borrow_mut() = p;
        }
        *self.align_main_label.borrow_mut() = align_label("mainAlign", main_align);
        *self.align_cross_label.borrow_mut() = align_label("crossAlign", cross_align);
    }

    /// Dimensions 的 min/max 字段（`layout_panel.rs::limit_row_v`）。
    pub(crate) fn set_dim_limits(&self, cx: &mut App, min_w: Option<f64>, max_w: Option<f64>, min_h: Option<f64>, max_h: Option<f64>) {
        let mut fmt = |e: &Entity<TextField>, v: Option<f64>| {
            e.update(cx, |tf, _| match v {
                Some(val) => {
                    tf.set_placeholder("");
                    tf.set_value(fmt_num(val));
                }
                None => {
                    tf.set_value("");
                    tf.set_placeholder(tr("Mixed"));
                }
            });
        };
        fmt(&self.dim_min_w, min_w);
        fmt(&self.dim_max_w, max_w);
        fmt(&self.dim_min_h, min_h);
        fmt(&self.dim_max_h, max_h);
    }

    /// Update the blend-mode dropdown's label (after a pick / selection sync).
    pub(crate) fn set_blend(&self, label: &str, _cx: &mut App) {
        *self.blend_label.borrow_mut() = SharedString::from(label.to_string());
    }

    /// Update the fill colour row after a pick: the row shows a solid paint.
    pub(crate) fn set_fill_color(&self, c: Color32, cx: &mut App) {
        let [r, g, b, a] = c.0;
        *self.last_fill.borrow_mut() = Some(figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(r, g, b, a)));
        self.fill_hex.update(cx, |tf, _| {
            tf.set_readonly(false);
            tf.set_value(c.to_hex());
        });
    }

    /// Update the fill colour row from the document's paint. The row itself is a `ColorField`
    /// (`Field-color`): the swatch shows the paint (gradient / image / pattern included), the value
    /// is the hex for a solid paint and the paint's label otherwise.
    pub(crate) fn set_fill_paint(&self, paint: &figcraft_color::Paint, cx: &mut App) {
        if self.last_fill.borrow().as_ref() == Some(paint) {
            return;
        }
        *self.last_fill.borrow_mut() = Some(paint.clone());
        let solid = paint.solid_color();
        self.fill_hex.update(cx, |tf, _| {
            tf.set_readonly(solid.is_none());
            tf.set_value(match solid {
                Some(c) => c.to_hex(),
                None => crate::widgets::swatch::paint_name(paint),
            });
        });
    }

    /// The paint the row shows (the default before a selection is read).
    fn fill_paint(&self) -> figcraft_color::Paint {
        self.last_fill.borrow().clone().unwrap_or_else(|| figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(0x33, 0x66, 0xFF, 255)))
    }

    /// Update the stroke colour row after a pick: the row shows a solid paint.
    pub(crate) fn set_stroke_color(&self, c: Color32, cx: &mut App) {
        let [r, g, b, a] = c.0;
        *self.last_stroke.borrow_mut() = Some(figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(r, g, b, a)));
        self.stroke_hex.update(cx, |tf, _| {
            tf.set_readonly(false);
            tf.set_value(c.to_hex());
        });
    }

    /// Update the stroke colour row from the document's paint (gradient / image / pattern included).
    pub(crate) fn set_stroke_paint(&self, paint: &figcraft_color::Paint, cx: &mut App) {
        if self.last_stroke.borrow().as_ref() == Some(paint) {
            return;
        }
        *self.last_stroke.borrow_mut() = Some(paint.clone());
        let solid = paint.solid_color();
        self.stroke_hex.update(cx, |tf, _| {
            tf.set_readonly(solid.is_none());
            tf.set_value(match solid {
                Some(c) => c.to_hex(),
                None => crate::widgets::swatch::paint_name(paint),
            });
        });
    }

    /// The paint the stroke row shows (the default before a selection is read).
    fn stroke_paint(&self) -> figcraft_color::Paint {
        self.last_stroke.borrow().clone().unwrap_or_else(|| figcraft_color::Paint::solid(figcraft_color::Color::from_rgba8(0x33, 0x66, 0xFF, 255)))
    }
}

/// Compact number for a field (`-0`, float noise and long tails removed).
fn fmt_num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == 0.0 {
        "0".to_string()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// A row of icon buttons for a section header's single trailing slot.
fn trail(icons: &[Icon]) -> Div {
    let mut d = div().flex().flex_row().items_center().gap(pt(4.0));
    for &icon in icons {
        d = d.child(IconButton::new(icon).size(24.0));
    }
    d
}

/// egui `blend_menu(ui, true)`: every mode in `ALL` order (pass-through included), with a divider
/// above the first row of each following group — after Normal / ColorBurn / ColorDodge /
/// HardLight / Exclusion.
pub(crate) fn blend_rows() -> Vec<MenuRow> {
    let mut rows: Vec<MenuRow> = Vec::new();
    let mut divider = false;
    for b in BlendMode::ALL {
        let mut row = MenuRow::new(b.label());
        if divider {
            row = row.separator();
        }
        rows.push(row);
        divider = matches!(b, BlendMode::Normal | BlendMode::ColorBurn | BlendMode::ColorDodge | BlendMode::HardLight | BlendMode::Exclusion);
    }
    rows
}

/// egui `position_fields`'s "Distribute and tidy" menu: the three commands run against the
/// selection (`distribute.horizontal` / `distribute.vertical` / `arrange.tidy`), each with
/// `{"ids": ids}`. Shown only when 3+ layers are selected, like the egui original.
fn distribute_rows() -> Vec<MenuRow> {
    vec![MenuRow::new(tr("Distribute horizontal spacing")), MenuRow::new(tr("Distribute vertical spacing")), MenuRow::new(tr("Tidy up"))]
}

/// Main-axis alignment options: `MainAlign` serde names → labels. egui's 3×3 picker sets
/// `mainAlign` / `crossAlign`; here the same values come from two `Dropdown`s (no picker widget).
const MAIN_ALIGN: &[(&str, &str)] = &[
    ("Packed", "min"),
    (tr_noop("Center"), "center"),
    (tr_noop("End"), "max"),
    (tr_noop("Space between"), "spaceBetween"),
    (tr_noop("Space around"), "spaceAround"),
    (tr_noop("Space evenly"), "spaceEvenly"),
];
/// Cross-axis alignment options: `CrossAlign` serde names → labels.
const CROSS_ALIGN: &[(&str, &str)] = &[("Packed", "min"), (tr_noop("Center"), "center"), (tr_noop("End"), "max"), ("Baseline", "baseline")];

fn align_rows(kind: &'static str) -> Vec<MenuRow> {
    let opts = if kind == "mainAlign" { MAIN_ALIGN } else { CROSS_ALIGN };
    opts.iter().map(|(label, _)| MenuRow::new(*label)).collect()
}

/// Look up the serde value for a picked alignment label (used by `right_panel`'s on_pick).
fn align_value(kind: &'static str, label: &str) -> &'static str {
    let opts = if kind == "mainAlign" { MAIN_ALIGN } else { CROSS_ALIGN };
    opts.iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or("min")
}

/// The label shown on an alignment dropdown for a given serde value (`"Mixed"` when absent).
fn align_label(kind: &'static str, value: Option<&'static str>) -> SharedString {
    let opts = if kind == "mainAlign" { MAIN_ALIGN } else { CROSS_ALIGN };
    SharedString::from(value.and_then(|v| opts.iter().find(|(_, sv)| *sv == v).map(|(l, _)| *l)).unwrap_or(tr("Mixed")).to_string())
}

/// The 1 pt divider between sections.
fn sep() -> Div {
    let t = tokens();
    // `flex_none`: a 1 pt rule in a column whose content is taller than the panel would
    // otherwise be shrunk to zero height — that is why the section dividers disappeared.
    crate::widgets::dbg_frame(div().flex_none().h(pt(1.0)).w_full().bg(rgba(t.border)), 0x8E8E93)
}

/// A section with `title`, an optional greyed (`Dim`) header and trailing icon buttons.
/// Rows are added by the caller via [`crate::widgets::field_row`].
fn section(title: &'static str, dim: bool, icons: &[Icon]) -> PanelSection {
    let s = PanelSection::new(title);
    let s = if dim { s.variant(SectionHeaderVariant::Dim) } else { s };
    if icons.is_empty() { s } else { s.header_trail(trail(icons)) }
}

/// 左栏 rail 三项的高亮状态（File / Assets / Variables），**严格互斥**。
/// 变量全屏时只高亮 Variables；否则按 `left_tab`：0=File、1=Assets、2=File（Icons 归文件视图，
/// rail 暂无独立 Icons 按钮）。抽成纯函数是为了穷举所有组合做回归测试。
pub(crate) fn rail_active(left_tab: u8, variables_open: bool, variables_full: bool) -> (bool, bool, bool) {
    let variables = variables_open && variables_full;
    let file = !variables && (left_tab == 0 || left_tab == 2);
    let assets = !variables && left_tab == 1;
    (file, assets, variables)
}

#[cfg(test)]
mod rail_active_tests {
    use super::rail_active;

    #[test]
    fn the_three_tabs_are_never_both_active() {
        for lt in 0..=4u8 {
            for vo in [false, true] {
                for vf in [false, true] {
                    let (f, a, v) = rail_active(lt, vo, vf);
                    assert!(
                        (f as u8) + (a as u8) + (v as u8) <= 1,
                        "left_tab={lt} open={vo} full={vf} -> file={f} assets={a} vars={v}"
                    );
                }
            }
        }
    }

    /// 用户 2026-11-11 报的 bug：切到组件库后 File 必须灭，不能两个都选。
    #[test]
    fn switching_to_assets_turns_file_off() {
        assert_eq!(rail_active(0, false, false), (true, false, false));
        assert_eq!(rail_active(1, false, false), (false, true, false));
        assert_eq!(rail_active(2, false, false), (true, false, false));
    }

    /// 用户 2026-11-08 要求：浮动小窗不算选中，左栏仍高亮 File。
    #[test]
    fn floating_variables_keeps_the_file_tab_active() {
        assert_eq!(rail_active(0, true, false), (true, false, false));
        assert_eq!(rail_active(1, true, false), (false, true, false));
    }

    /// `variables_full` 是会话记忆，关掉后高亮必须回到 File/Assets，且全屏时压过 left_tab。
    #[test]
    fn full_variables_wins_and_session_memory_does_not_leak() {
        assert_eq!(rail_active(0, true, true), (false, false, true));
        assert_eq!(rail_active(1, true, true), (false, false, true));
        assert_eq!(rail_active(0, false, true), (true, false, false));
        assert_eq!(rail_active(1, false, true), (false, true, false));
    }
}

/// The fixed 56 pt left icon rail (skeleton — placeholder glyphs only).
pub(crate) fn left_rail(
    on_menu: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    on_variables: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    // 点 File：egui `layers.rs` 里 tab 点击会 `app.ui.variables = None`，Full 时据此切回普通视图。
    on_file: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    // 点 Assets：切到左面板的 Assets tab（R1），不再弹浮层。
    on_assets: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    // 左栏当前 tab（0=File / 1=Assets / 2=Icons）与变量编辑器的开 / 全屏状态。
    // 三项高亮统一由 `rail_active` 派生（含穷举回归测试），调用点不再各算一份：
    //   * **浮动小窗不算选中**——小窗开着时左栏仍高亮 File（用户 2026-11-08）；
    //   * 只有全屏变量面板才高亮 Variables；
    //   * 关闭后 `variables_full` 作为会话记忆保留，须与 `variables_open` 取与；
    //   * 点 Assets 后 File 必须灭（用户 2026-11-11「切到组件库两个都选」）。
    left_tab: u8,
    variables_open: bool,
    variables_full: bool,
) -> Div {
    let t = tokens();
    let (file_active, assets_active, variables_tab) = rail_active(left_tab, variables_open, variables_full);
    div()
        .flex_none()
        .w(pt(RAIL_W))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(pt(RAIL_PAD_Y))
        .bg(rgba(t.panel))
        // 与左面板之间的那条静态 1px 分隔线（用户：侧栏与左工具栏的分隔线要保留）。
        // 用 rail 自身右缘描边、只此一条；border-box 内描边不改布局宽度，RAIL_W 命中几何不变。
        .border_r_1()
        .border_color(rgba(t.border))
        // ① 主菜单：Logo 一格 32×32（egui `layers::rail` 的第一个 `lr`）。
        // 尺寸/内边距必须与 `rail_menu_bounds()` 一致（菜单锚点按那套几何算）。
        .child(IconButton::new(Icon::Logo).id("rail-menu").size(RAIL_BTN).on_click(on_menu))
        // ② 16 pt 分隔带，中间一条 1 px 横线（egui：`y += 16.0` 之前画 `line_segment`）。
        .child(
            div()
                .flex_none()
                .h(pt(16.0))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(div().w(pt(16.0)).h(pt(1.0)).bg(rgba(t.border))),
        )
        // ③ File / Assets / Variables：每项 56 pt —— 32×32 图标格 + 下面 9 pt 的文字标签。
        //    egui 的 rail 是**带文字**的，之前这里只有图标（而且三个都是占位圆点）。
        .child(rail_item(
            "rail-file",
            Icon::Page,
            tr("File"),
            file_active,
            Some(Box::new(on_file)),
        ))
        .child(rail_item("rail-assets", Icon::Assets, tr("Assets"), assets_active, Some(Box::new(on_assets))))
        .child(rail_item(
            "rail-variables",
            Icon::Variables,
            tr("Variables"),
            variables_tab,
            Some(Box::new(on_variables)),
        ))
}

/// 竖条的一项：56 pt 高的整列可点，32×32 的图标格（悬停/选中底色）+ 下面 9 pt 的文字
/// （`ui-egui/src/panels/layers.rs::rail`：`ir` 32×32、文字在 `y + 47`）。
fn rail_item(
    id: &'static str,
    icon: Icon,
    label: &'static str,
    active: bool,
    on_click: Option<Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App)>>,
) -> Stateful<Div> {
    let t = tokens();
    let group: SharedString = format!("{id}-grp").into();
    let cell = div()
        .flex_none()
        .size(pt(RAIL_BTN))
        .rounded(pt(5.0))
        .flex()
        .items_center()
        .justify_center()
        .when(active, |d| d.bg(rgba(t.selected_row)))
        .when(!active, |d| d.group_hover(group.clone(), |s| s.bg(rgba(t.hover))))
        .child(crate::widgets::icon::paint(icon, rgba(if active { t.accent } else { t.icon }), 16.0));
    let mut item = div()
        .id(id)
        .flex_none()
        .w_full()
        .h(pt(56.0))
        .flex()
        .flex_col()
        .items_center()
        .pt(pt(4.0))
        .cursor_pointer()
        .group(group)
        .child(cell)
        .child(div().mt(pt(3.0)).text_size(pt(9.0)).text_color(rgba(t.text)).child(tr(label)));
    if let Some(f) = on_click {
        item = item.on_click(move |e, w, cx| f(e, w, cx));
    }
    item
}

/// The left panel (skeleton — header only). The caller sets its width.
pub(crate) fn left_panel() -> Stateful<Div> {
    let t = tokens();
    div()
        .id("left-panel")
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .bg(rgba(t.panel))
        .child(SectionHeader::new(tr("Layers")).variant(SectionHeaderVariant::Normal))
}

/// The right design panel, mirroring the egui Design panel's section order.
///
/// `data` 是每帧从 `Document` 读出的列表快照；`window` / `cx` 供 `ColorField` 与
/// `bound_field`（绑定 chip）这类需要测量/缓存的通用组件使用。
#[allow(clippy::too_many_arguments)]
pub(crate) fn right_panel(
    f: &RightFields,
    sel: PanelSel,
    data: &right::data::PanelData,
    panel_w: f32,
    window: &mut Window,
    cx: &mut App,
    motion: bool,
) -> Stateful<Div> {
    let t = tokens();
    // 不再手算任何列宽：字段/下拉/颜色行一律 `w_full`，列宽完全由统一网格的 `flex_1` 决定。
    // `FILL` 是传给需要宽度参数组件的哨兵值（它们内部都走 `w_full`）。
    const FILL: f32 = 0.0;
    let col = FILL;
    let row_w = FILL;
    let _ = panel_w;
    // egui 的右栏顶栏（tabs + Zoom 菜单）固定在 ScrollArea **之外**：外层不滚动，
    // 只有下面的 section 列表滚动。否则顶栏会跟着内容滚走，显隐开关就找不到了。
    let mut body = div().id("right-body").flex_1().min_h(px(0.0)).w_full().flex().flex_col().overflow_y_scroll();
    let wrap = |body: Stateful<Div>| -> Stateful<Div> {
        let t = tokens();
        div()
            .id("right-panel")
            .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(rgba(t.panel))
            // 48 pt 顶栏（头像 / Present / Share）：egui `right.rs:103-127` 画在 tabs 之上，
            // 且 Motion 模式下同样保留。以前只有 lib.rs 的 Motion 分支画了它，普通右栏漏掉，
            // 于是退出 Motion 后整条头像/分享栏就「消失」了。这里补回，与 Motion 分支同序。
            .child(right::topbar::top_bar(f, data))
            // 「显示 / 隐藏」开关无论有无选中都在：view.* 是文档级命令，不需要选区。
            .child(right::sections::view_bar(f, data, motion))
            .child(body)
    };

    // 列表行编辑器池：每帧清空上下文，然后各节按顺序认领槽位。
    f.begin_rows();
    let mut n = 0usize;

    // A3: the Comment tool shows the Comments list instead of the design sections
    // (egui `right.rs`: `if tool == Comment { crate::comments::panel(...); return; }`).
    if data.comment_tool {
        body = body.child(crate::comments::panel(f, &data.comments, data.comments_total));
        return wrap(body);
    }

    // ── Prototype / Inspect 标签：整页交给对应面板（egui `RightTab::Prototype` / `RightTab::Inspect`）──
    match *f.right_tab.borrow() {
        1 => {
            body = body.child(right::proto::panel(f, data, cx, &mut n));
            return wrap(body);
        }
        2 => {
            body = body.child(right::inspect::panel(f, data, cx, &mut n));
            return wrap(body);
        }
        _ => {}
    }

    // ── 无选中：Page（egui `page_section`）──────────────────────────────────────
    if sel.empty {
        // `view_bar` 已有下边框，这里不再补一条线。
        // egui `page_section` 是三节（Page / Local variables / Local styles），节间各一条线。
        let secs = right::sections::page(f, data, row_w, &mut n, cx);
        let last = secs.len().saturating_sub(1);
        for (i, sec) in secs.into_iter().enumerate() {
            body = body.child(sec);
            if i < last {
                body = body.child(sep());
            }
        }
        return wrap(body);
    }

    // ── Frame 工具（F）：Design 标签整页换成 Frame 预设（egui `right.rs`:
    //    `RightTab::Design if tool == Frame => frame_presets::panel`）──────────────
    if data.frame_tool {
        body = body.child(right::sections::frame_presets(f, data));
        return wrap(body);
    }

    // 预计算需要 `window` / `cx` 的节（避免链式构造里的重复可变借用）。
    let header_sec = right::sections::header(f, data, col);
    // Component / Instance（egui `header` 末尾的 `component_panel::instance_section`）。
    let component_sec = right::component::section(f, data, cx, &mut n);
    let section_sec = right::sections::section(f, data, row_w);
    let modes_sec = right::sections::variable_modes(f, data, row_w);
    // Design panel: not in Motion mode, so paint rows carry no keyframe diamond. The motion queue
    // is a placeholder (never used — `keyframe_layer` is `None`), keeping the shared `paints` call
    // shape identical to the Motion inspector's.
    let motion_q = crate::motion::Queue::new(std::rc::Rc::new(|| {}));
    let fills_sec = right::sections::paints(f, data, true, sel.has_vector, row_w, col, &mut n, window, cx, None, false, &motion_q);
    let strokes_sec = right::sections::paints(f, data, false, sel.has_vector, row_w, col, &mut n, window, cx, None, false, &motion_q);
    let effects_sec = right::sections::effects(f, data, col, cx, None);
    let sel_colors_sec = right::sections::selection_colors(f, data, row_w, col, &mut n, window, cx);
    let grids_sec = right::sections::grids(f, data, col);
    let export_sec = right::sections::export(f, data, col, &mut n, cx);

    // ⛔ 分隔线的规则：**一条线属于「它上面那一节」的结尾**，且只有那一节真的存在时才加。
    //  - `view_bar` 自己有 `border_b_1`，所以 body 开头**不再补线**（否则顶部两条）。
    //  - 可选节（header / section / variable modes / selection colors）返回 `None` 时，
    //    如果照样补一条，就会和上一节结尾那条挨在一起 —— 表现为两条分隔线贴在一起。
    if let Some(h) = header_sec {
        body = body.child(h).child(sep());
    }
    if let Some(c) = component_sec {
        body = body.child(c).child(sep());
    }
    if let Some(s) = section_sec {
        body = body.child(s).child(sep());
    }
    // ── Position / Scale / Points ───────────────────────────────────────────────
    // 矢量编辑选了顶点或线段时，点面板**替换** Position（egui `design`：`match vector_edit
    // { Some(ve) => vector_panel::points(...), None => { … position … } }`）。
    let points_selected = data.vector.as_ref().is_some_and(|v| v.has_selection);
    body = body
        .child(if points_selected {
            right::sections::vector_points(f, data, cx, &mut n)
        } else if data.scale.is_some() {
            right::sections::scale(f, data, cx)
        } else {
            // `position_fields`: Alignment captions sit above the 32 pt field track; the six icons are
            // two three-cell segments; each fills one field column of the unified grid (no fixed
            // width — they flow with the columns, so they stay aligned with the X/Y rows below).
            let bar = |items: &[(Icon, &'static str)], base: usize| {
                let cmds = f.cmds.clone();
                Segmented::icons(items).on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
                    const NAMES: [&str; 6] = ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom"];
                    cmds.push(Cmd { id: NAMES[(base + i).min(5)], params: None });
                })
            };
            let h = bar(&[(Icon::AlignLeft, tr("Align left")), (Icon::AlignHCenter, tr("Align horizontal centers")), (Icon::AlignRight, tr("Align right"))], 0);
            let v = bar(&[(Icon::AlignTop, tr("Align top")), (Icon::AlignVCenter, tr("Align vertical centers")), (Icon::AlignBottom, tr("Align bottom"))], 3);
            // egui `right.rs::position_fields`: `if sel.len() > 2 { DistributeH }`。
            // 少于 3 个图层时这个按钮**根本不画**，不是画出来再禁用。此前这里无条件显示，
            // 是「按钮什么时候显示 / 什么时候隐藏没接好」最直白的一处。
            let distribute_btn = visibility::distribute(&sel).then(|| {
                let dist_cmds = f.cmds.clone();
                let dist_root = f.root.clone();
                IconButton::new(Icon::DistributeH).size(24.0).on_click(move |_e, window, cx| {
                    let Some(root) = dist_root.clone() else { return };
                    let p = window.mouse_position();
                    let anchor = gpui::Bounds { origin: p, size: gpui::size(px(0.0), px(0.0)) };
                    let dc = dist_cmds.clone();
                    let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                        let cmd = match label {
                            "Distribute horizontal spacing" => "distribute.horizontal",
                            "Distribute vertical spacing" => "distribute.vertical",
                            "Tidy up" => "arrange.tidy",
                            _ => return,
                        };
                        dc.push(Cmd { id: cmd, params: None });
                    });
                    root.update(cx, |cv, cx| cv.open_menu(anchor, Some(220.0), distribute_rows(), on_pick, cx));
                })
            });
            // `rotate_row`: the three 24 pt cells under "Rotation" (`position_fields`'s
            // `widgets::segmented`): Rotate 90° / Flip H / Flip V — each a `object.*` command.
            let rot_cmds = f.cmds.clone();
            let rotate = Segmented::icons(&[
                (Icon::Rotate, tr("Rotate 90° right")),
                (Icon::FlipH, tr("Flip horizontal")),
                (Icon::FlipV, tr("Flip vertical")),
            ])
            .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
                const C: [&str; 3] = ["object.rotateRight", "object.flipH", "object.flipV"];
                rot_cmds.push(Cmd { id: C[i.min(2)], params: None });
            });
            let mut pos_sec = PanelSection::new(tr("Position"));
            if let Some(b) = distribute_btn {
                pos_sec = pos_sec.header_trail(b);
            }
            let pos_sec = pos_sec
                .child(sublabel_row(tr("Alignment")))
                .child(field_row(vec![h.into_any_element(), v.into_any_element()], Vec::<IconButton>::new()))
                .child(sublabel_row(tr("Position")))
                .child(field_row(vec![f.x.clone().into_any_element(), f.y.clone().into_any_element()], Vec::<IconButton>::new()))
                .child(sublabel_row(tr("Rotation")))
                .child(field_row(vec![f.rotation.clone().into_any_element(), rotate.into_any_element()], Vec::<IconButton>::new()));

            // ── Constraints（egui `right.rs::position_fields`：仅 frame 子层、自身无自动布局或为
            // absolute 时才显示；`in_frame` 的逻辑与 egui `common` 后的 `all(...)` 完全一致）。──
            // 两列各一个下拉：水平 left/right/leftRight/center/scale、垂直 top/bottom/topBottom/center/scale；
            // 选中即 `node.set {"props":{"constraints":{"horizontal"/"vertical":v}}}`（等价于 egui 的
            // `app.set_props(&ids, json!({"constraints":{"horizontal": v}}))`，`node.set` 由 Cmd 通道附带
            // 选区 ids 执行）。`None` = Mixed，下拉显示 "Mixed"。
            let pos_sec = if data.in_frame {
                let pretty_h = |v: Option<&'static str>| -> &'static str {
                    match v {
                        Some("leftRight") => tr("Left & right"),
                        Some(s) => s,
                        None => tr("Mixed"),
                    }
                };
                let pretty_v = |v: Option<&'static str>| -> &'static str {
                    match v {
                        Some("topBottom") => tr("Top & bottom"),
                        Some(s) => s,
                        None => tr("Mixed"),
                    }
                };
                let h_opts: Vec<(String, &'static str, Value)> = vec![
                    (tr("Left").into(), "node.set", serde_json::json!({ "props": { "constraints": { "horizontal": "left" } } })),
                    (tr("Right").into(), "node.set", serde_json::json!({ "props": { "constraints": { "horizontal": "right" } } })),
                    (tr("Left & right").into(), "node.set", serde_json::json!({ "props": { "constraints": { "horizontal": "leftRight" } } })),
                    (tr("Center").into(), "node.set", serde_json::json!({ "props": { "constraints": { "horizontal": "center" } } })),
                    (tr("Scale").into(), "node.set", serde_json::json!({ "props": { "constraints": { "horizontal": "scale" } } })),
                ];
                let v_opts: Vec<(String, &'static str, Value)> = vec![
                    (tr("Top").into(), "node.set", serde_json::json!({ "props": { "constraints": { "vertical": "top" } } })),
                    (tr("Bottom").into(), "node.set", serde_json::json!({ "props": { "constraints": { "vertical": "bottom" } } })),
                    (tr("Top & bottom").into(), "node.set", serde_json::json!({ "props": { "constraints": { "vertical": "topBottom" } } })),
                    (tr("Center").into(), "node.set", serde_json::json!({ "props": { "constraints": { "vertical": "center" } } })),
                    (tr("Scale").into(), "node.set", serde_json::json!({ "props": { "constraints": { "vertical": "scale" } } })),
                ];
                pos_sec
                    .child(sublabel_row(tr("Constraints")))
                    .child(field_row(
                        vec![
                            right::sections::pick_dropdown(f, pretty_h(data.constraints_h), col, h_opts),
                            right::sections::pick_dropdown(f, pretty_v(data.constraints_v), col, v_opts),
                        ],
                        Vec::<IconButton>::new(),
                    ))
            } else {
                pos_sec
            };
            pos_sec
        })
        .child(sep())
        // ── Auto layout（egui `layout_panel.rs::layout_section` / `direction` / `spacing` / `alignment`）──
        // 用现成通用组件实现：±添加/移除、方向 segmented、gap/cross-gap 两个 TextField、
        // padding 四个 TextField、主轴/交叉轴对齐两个 Dropdown；W/H sizing 走 `dim_trail_menu`
        // 的 trail 菜单（Fixed/Hug/Fill + min/max），min/max 用 `any_min/any_max` 行，3×3 对齐
        // picker 由 `right::picker::alignment_picker` 自绘。grid tracks 见 `grid_track_rows`。
        // 矢量点被选中时整节让位给点面板（egui `if !points_selected { layout_section(…) }`）。
        .child(if points_selected {
            div().into_any_element()
        } else {
            // egui `layout_panel.rs::layout_section` 的 `btns` 三分支：in_instance → 不画；
            // frames && !has_al → `+`；has_al → `−`；其余 → 不画。此前无条件画 `+`/`−`。
            // ChevronDown 在 egui 的 layout header 里**根本不存在**，之前是上一轮多画的、
            // 连 on_click 都没有的幽灵按钮；高级设置入口本来由下面的 `layout_dots` 承担，故移除。
            let layout_btn = visibility::layout_header(&sel).map(|id| {
                let cmds = f.cmds.clone();
                IconButton::new(if id == "layout.remove" { Icon::Minus } else { Icon::Plus }).size(20.0).on_click(move |_e, _window, _cx| {
                    cmds.push(Cmd { id, params: None });
                })
            });
            let mut sec = PanelSection::new(if sel.has_al { tr("Auto layout") } else { tr("Layout") });
            if let Some(b) = layout_btn {
                sec = sec.header_trail(b);
            }

            // ── 顺序照 `layout_section`：① 方向（仅 has_al）② Dimensions（无条件）③ 对齐/间距（仅 has_al）──

            // ① 方向切换（egui `direction`，标题 "Layout" 下）。
            if sel.has_al {
                let active = match sel.layout_dir {
                    LayoutDir::Vertical => Some(0),
                    LayoutDir::Horizontal => Some(1),
                    LayoutDir::Grid => Some(2),
                    LayoutDir::None => None,
                };
                let dir_cmds = f.cmds.clone();
                let dir = Segmented::icons(&[
                    (Icon::AlignVCenter, tr("Vertical")),
                    (Icon::AlignHCenter, tr("Horizontal")),
                    (Icon::ViewGrid, tr("Grid")),
                ])
                .active(active)
                .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
                    let m = match i {
                        0 => "vertical",
                        1 => "horizontal",
                        2 => "grid",
                        _ => return,
                    };
                    dir_cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { "layout": m } })) });
                });
                sec = sec.child(sublabel_row(tr("Layout"))).child(field_row_full(dir));
            }

            // ② Dimensions（egui `dimensions`：**无条件**显示）。
            let dims = data.dimensions.clone().unwrap_or_default();
            right::sections::dim_trail_menu(f, cx, &f.w, true, &dims);
            right::sections::dim_trail_menu(f, cx, &f.h, false, &dims);
            let lock_cmds = f.cmds.clone();
            let lock_on = dims.constrain;
            let constrain =
                IconButton::new(if lock_on { Icon::LinkOn } else { Icon::LinkOff }).size(24.0).active(lock_on).on_click(move |_e, _w, _cx| {
                    lock_cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { "constrainProportions": !lock_on } })) });
                });
            sec = sec.child(sublabel_row(tr("Dimensions"))).child(field_row(vec![f.w.clone(), f.h.clone()], vec![constrain]));
            // 选中文字时的「调整大小」三格（用户 2026-11-11：从 Typography 段移到 Layout 段）。
            // 值仍是 `textAutoResize`，与 Typography 之前那个下拉同一条命令。
            if let Some(t) = data.text.as_ref() {
                let active = match t.auto_resize {
                    Some("widthAndHeight") => Some(0usize),
                    Some("height") => Some(1),
                    Some("none") => Some(2),
                    _ => None,
                };
                let cmds = f.cmds.clone();
                let seg = crate::widgets::segmented::Segmented::new(vec![
                    crate::widgets::segmented::SegItem::text(tr("Auto width")),
                    crate::widgets::segmented::SegItem::text(tr("Auto height")),
                    crate::widgets::segmented::SegItem::text(tr("Fixed")),
                ])
                .active(active)
                .on_pick(move |i: usize, _w: &mut Window, _cx: &mut App| {
                    let v = match i { 0 => "widthAndHeight", 1 => "height", 2 => "none", _ => return };
                    cmds.push(Cmd { id: "node.set", params: Some(serde_json::json!({ "props": { "textAutoResize": v } })) });
                });
                sec = sec.child(sublabel_row(tr("Resizing"))).child(field_row_full(seg));
            }
            // Min/Max：仅当该轴已设值、或刚从 sizing 菜单 Add 时才显示（egui `limit_row_v`）。
            let lo = *f.limit_open.borrow();
            let any_min = dims.min_w.is_some() || dims.min_h.is_some() || lo[0] || lo[2];
            let any_max = dims.max_w.is_some() || dims.max_h.is_some() || lo[1] || lo[3];
            if any_max {
                let left = if dims.max_w.is_some() || lo[1] { f.dim_max_w.clone().into_any_element() } else { div().into_any_element() };
                let right = if dims.max_h.is_some() || lo[3] { f.dim_max_h.clone().into_any_element() } else { div().into_any_element() };
                sec = sec.child(sublabel_row2(tr("Max W"), tr("Max H"))).child(field_row(vec![left, right], Vec::<IconButton>::new()));
            }
            if any_min {
                let left = if dims.min_w.is_some() || lo[0] { f.dim_min_w.clone().into_any_element() } else { div().into_any_element() };
                let right = if dims.min_h.is_some() || lo[2] { f.dim_min_h.clone().into_any_element() } else { div().into_any_element() };
                sec = sec.child(sublabel_row2(tr("Min W"), tr("Min H"))).child(field_row(vec![left, right], Vec::<IconButton>::new()));
            }

            // ③ 对齐 picker + 间距 + 内边距（egui `alignment` / `spacing`，仅 has_al）。
            if sel.has_al {
                // egui `alignment`：picker 一列跨两行，右侧是 gap 字段（1 个或 2 个），再右边一个 24。
                let picker = right::picker::alignment_picker(f, sel.layout_dir, &data.layout);
                // egui `alignment` 的标题：栅格时左标题写 "Grid"，否则 "Align"；右侧恒为 "Spacing"。
                let left_cap = if sel.layout_dir == LayoutDir::Grid { tr("Grid") } else { tr("Align") };
                // `two_gaps = mode == Grid || wrap`：只有这时才显示两个 gap；普通流只有一个 gap
                // （`gap_and_mode`），第二个字段位置留空。
                let two_gaps = sel.layout_dir == LayoutDir::Grid || data.layout.wrap;
                // 布局（用户 2026-11-05 明确要求，纯统一网格、零 cell 内包裹）：
                // 左列 = 对齐 picker，本身**跨两行**（`field_row_h` 把行高撑到 `ROW_H*2`=64，
                // picker 56 在其中上下各留 4 pt）；右列 = 间距，每个字段**各占一行**（gap 第一行、
                // cross_gap 第二行，仅 Grid / wrap 时），左列用空占位。picker 在左、跨两行，
                // gap / cross_gap 在右、各自一行 —— 完全由 `field_row` / `field_row_h` 表达，
                // 不在任何单元格里再套 `flex_col`。
                // picker 跨两行、在左；间距在右列、gap 占第一行、cross_gap 占第二行，二者与
                // picker **并排在同一个两行高的块里**（而不是堆到 picker 下方）。统一网格的
                // `field_row_h` 是一行两 cell、整行高 64；右列要分两行只能在该 cell 内用 `flex_col`
                // 排 gap / cross_gap（这是表达「右列两行」的标准方式，不是多余包裹）；`items_start`
                // 让仅一个 gap 时只占第一行、下方空出第二行。
                // 右列：gap / cross_gap 各自占一个 `ROW_H`(32) 高的网格行，内部把 24pt 的字段
                // **垂直居中**（上下各留 4pt）——这是「32 网格 / 24 内容」的标准对齐，不能 `items_start`
                // 把内容整体顶上去。`flex_col` + `items_start` 让整列从顶部排成两个 32 行。
                let right_col = if two_gaps {
                    div()
                        .w_full()
                        .h_full()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(pt(0.0))
                        .child(div().w_full().h(pt(crate::widgets::ROW_H)).flex().items_center().child(f.gap.clone().into_any_element()))
                        .child(div().w_full().h(pt(crate::widgets::ROW_H)).flex().items_center().child(f.cross_gap.clone().into_any_element()))
                } else {
                    div()
                        .w_full()
                        .h_full()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(pt(0.0))
                        .child(div().w_full().h(pt(crate::widgets::ROW_H)).flex().items_center().child(f.gap.clone().into_any_element()))
                };
                // Gap 行右侧那颗 dots（egui `layout_panel::settings_toggle` 的 "Advanced"）：
                // 打开的是通用窗口弹层「Layout」，不再是旧版的一串下拉菜单。
                let popups = f.popups();
                let settings_queue = f.queue();
                let settings_data = data.clone();
                let layout_dots = IconButton::new(Icon::Menu).id("layout-settings-dots").size(24.0).on_click(move |_e, window, cx: &mut App| {
                    if std::env::var_os("FC_DEBUG_POPUP_BTN").is_some() {
                        eprintln!("[popup-btn] layout dots clicked");
                    }
                    let anchor = window.mouse_position();
                    let w = crate::popovers::WindowState::layout(settings_queue.clone(), &settings_data, cx, anchor);
                    popups.request(w);
                });
                sec = sec.child(sublabel_row2(left_cap, tr("Spacing"))).child(crate::widgets::field_row_h(
                    vec![picker, right_col],
                    vec![layout_dots],
                    crate::widgets::ROW_H * 2.0,
                ));

                // Padding（egui `spacing`）：默认水平/垂直两字段；点右侧按钮切「独立内边距」→
                // 四个字段（L/T、R/B，按 egui 的 sides 顺序）。
                let individual = *f.pad_individual.borrow();
                let toggle_cmds = f.cmds.clone();
                let pad_flag = f.pad_individual.clone();
                let toggle = IconButton::new(Icon::AbsolutePosition).size(24.0).active(individual).on_click(move |_e, _w, _cx| {
                    let new_val = !*pad_flag.borrow();
                    *pad_flag.borrow_mut() = new_val;
                    // 翻转后必须唤醒视图重绘，否则右栏读到的还是旧 `pad_individual`，四个内边距字段不展开。
                    toggle_cmds.wake();
                });
                sec = sec.child(sublabel_row(tr("Padding")));
                if individual {
                    sec = sec
                        .child(field_row(vec![f.pad_l.clone(), f.pad_t.clone()], Vec::<IconButton>::new()))
                        .child(field_row(vec![f.pad_r.clone(), f.pad_b.clone()], vec![toggle]));
                } else {
                    sec = sec.child(field_row(vec![f.pad_h.clone(), f.pad_v.clone()], vec![toggle]));
                }
            }
            // ④ 网格轨道（egui `layout_panel.rs::tracks`，只在 grid frame 上出现）：
            // 每轴一组「Column sizes / Row sizes」+ 每条轨道一行（序号 / 尺寸 / sizing 下拉）。
            sec = right::sections::grid_track_rows(f, data, cx, &mut n, sec);
            sec.into_any_element()
        })
        .child(sep());

    // ── Arc：仅当全部选中都是 Ellipse（原版 `arc_section` 返回 false 就不画） ──────
    if sel.all_ellipse && !points_selected {
        body = body
            .child(
                section(tr("Arc"), false, &[])
                    .child(field_row(vec![f.arc_start.clone(), f.arc_sweep.clone()], Vec::<IconButton>::new()))
                    .child(field_row(vec![f.arc_ratio.clone()], Vec::<IconButton>::new())),
            )
            .child(sep());
    }

    // ── Star：仅当全部选中都是 Polygon/Star（原版 `points_section`） ──────────────
    if sel.all_points && !points_selected {
        body = body
            .child(section(tr("Star"), false, &[]).child(field_row(vec![f.star_points.clone(), f.star_ratio.clone()], Vec::<IconButton>::new())))
            .child(sep());
    }

    body = body
        // ── Appearance（egui `paints.rs::appearance`：Eye / Variable / Blend 三个 header 图标）──
        .child(right::sections::appearance(f, data, row_w, &f.opacity, &f.corner))
        .child(sep());
    // ── Variable modes（egui `paints.rs::variable_modes`）：没有变量时**不留线** ──────────
    if let Some(m) = modes_sec {
        body = body.child(m).child(sep());
    }

    // ── Typography：仅当选中含 Text（原版 `sel.iter().any(Text)`） ────────────────
    if sel.has_text {
        body = body.child(right::sections::typography(f, data)).child(sep());
    }

    body = body
        // ── Fill / Stroke（egui `paints.rs::paint_row`：paint 列表 + ColorField 颜色输入框
        //    + variable_button/bound_field 绑定按钮 + 显隐/删除 + 添加） ──
        .child(fills_sec)
        .child(sep())
        .child(strokes_sec)
        .child(sep())
        // ── Effects（egui `effects.rs::effects`：类型下拉 + 显隐/删除 + 添加）──
        .child(effects_sec)
        .child(sep());
    // ── Selection colors（egui `selection_colors.rs::section`）：只有一组颜色时整节不画，
    //    也**不留线**（否则它和 Effects 结尾那条线会挨成两条）。
    if let Some(s) = sel_colors_sec {
        body = body.child(s).child(sep());
    }

    // ── Layout grid：仅当全部选中都是 frame-like（原版 `all(is_frame_like)`）──────
    if sel.all_frame {
        body = body.child(grids_sec).child(sep());
    }

    // ── Export（egui `export_panel.rs::setting_row`）──────────────────────────────
    wrap(body.child(export_sec))
}

/// The bottom animation bar (skeleton — header only).
pub(crate) fn animation_bar() -> Div {
    let t = tokens();
    div()
        .flex_none()
        .w_full()
        .h(pt(ANIM_H))
        .flex()
        .flex_col()
        .bg(rgba(t.panel))
        .child(
            div()
                .h(pt(47.0))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .px(pt(16.0))
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Animation"))),
        )
        .child(div().flex_1().w_full())
}

/// The floating bottom toolbar lives in [`crate::toolbar`] — it is the real pill (tool groups with
/// their flyouts, Actions, the mode switcher), not a row of placeholder icon buttons.
#[cfg(test)]
mod cmd_queue_tests {
    use super::*;
    use std::cell::Cell;

    /// Regression for "every right-panel button is dead": in a retained tree a command that is
    /// only pushed is never drained, because `sync_panel` runs from `render`, and `render` only
    /// runs when the view is dirty. Push must wake; drain must consume exactly once.
    #[test]
    fn push_wakes_the_view_and_drains_once() {
        let wakes = std::rc::Rc::new(Cell::new(0usize));
        let counter = wakes.clone();
        let wake: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || counter.set(counter.get() + 1));
        let q = CmdQueue::new(&wake);

        assert_eq!(q.len(), 0);
        q.push(Cmd { id: "align.left", params: None });
        q.push(Cmd { id: "effect.add", params: Some(serde_json::json!({ "type": "dropShadow" })) });
        assert_eq!(wakes.get(), 2, "each push must wake the retained view");
        assert_eq!(q.len(), 2);

        let got = q.drain();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "align.left");
        assert_eq!(got[0].params, None);
        assert_eq!(got[1].id, "effect.add");
        assert_eq!(got[1].params.as_ref().unwrap()["type"], "dropShadow");
        assert_eq!(q.len(), 0, "drain must not leave anything for the next frame");
        assert!(q.drain().is_empty(), "a second drain runs nothing");
        assert_eq!(wakes.get(), 2, "drain itself must not schedule another frame");
    }

    #[test]
    fn cmd_params_merges_button_extras_over_ids() {
        let ids = vec![figcraft_doc::NodeId(7)];
        let p = cmd_params(&ids, Some(serde_json::json!({ "index": 2, "type": "dropShadow" })));
        assert_eq!(p["ids"], serde_json::json!([7]));
        assert_eq!(p["index"], serde_json::json!(2));
        assert_eq!(p["type"], "dropShadow");
        // A button may pass its own explicit `ids`; the extra object wins.
        let overridden = cmd_params(&ids, Some(serde_json::json!({ "ids": [] })));
        assert_eq!(overridden["ids"], serde_json::json!([]));
        assert_eq!(cmd_params(&ids, None), serde_json::json!({ "ids": [7] }));
    }

    /// A misspelt id is exactly the "button does nothing" failure: `Session::execute` answers
    /// `UnknownCommand`, and the old `sync_panel` swallowed every error. This keeps the ids the
    /// right panel raises in sync with the engine's command catalogue.
    #[test]
    fn panel_command_ids_exist_in_the_engine() {
        use figcraft_engine::cmd::find_command;
        const IDS: &[&str] = &[
            "fill.add",
            "fill.set",
            "fill.remove",
            "stroke.add",
            "stroke.set",
            "stroke.remove",
            "vector.fillRule",
            "stroke.widthProfile",
            "stroke.brush",
            "node.set",
            "effect.add",
            "effect.set",
            "effect.remove",
            "grid.add",
            "grid.remove",
            "export.add",
            "export.update",
            "export.remove",
            "file.exportLayers",
            "file.colorProfile",
            "selection.setPaint",
            "selection.selectColor",
            "selection.applyColorStyle",
            "selection.bindColorVariable",
            "selection.detachColor",
            "variable.bind",
            "variable.setMode",
            "style.apply",
            "dev.resolveChange",
            "section.toggleContents",
            "object.resizeToFit",
            "object.toggleVisible",
            "object.convertToFrame",
            "object.convertToGroup",
            "frame.swapOrientation",
            "component.create",
            "object.mask",
            "vector.flatten",
            "bool.union",
            "bool.subtract",
            "bool.intersect",
            "bool.exclude",
            "page.setBackground",
            "align.left",
            "align.right",
            "align.top",
            "align.bottom",
            "align.hcenter",
            "align.vcenter",
            "distribute.horizontal",
            "distribute.vertical",
            "arrange.spacing",
            "arrange.tidy",
            "arrange.reorder",
            "layout.add",
            "layout.remove",
            "object.flipH",
            "object.flipV",
            "object.rotateRight",
            "edit.delete",
            "edit.duplicate",
            "edit.undo",
            "edit.redo",
            "select.all",
            "select.none",
            "select.set",
            // 右栏顶部 Zoom 菜单的「显示 / 隐藏」开关（文档级命令，无选区也要能执行）。
            "view.rulers",
            "view.pixelGrid",
            "view.snapPixel",
            "view.layoutGrids",
            "view.outlines",
            "view.pixelPreview",
            "view.zoomIn",
            "view.zoomOut",
            "view.zoomFit",
            "view.zoom50",
            "view.zoom100",
            "view.zoom200",
            "view.zoomSelection",
            "guide.add",
            "guide.move",
            "guide.remove",
            "guide.select",
            "shape.rectangle",
            "text.create",
            "node.create",
            "node.get",
            "transform.nudge",
            "transform.nudgeSize",
        ];
        for id in IDS {
            assert!(find_command(id).is_some(), "right panel raises unknown command `{id}`");
        }
    }

    /// The Zoom menu's show/hide rows are document-level commands: `sync_panel` used to drop
    /// every command raised without a selection, which made exactly these toggles dead.
    /// Execute the merged params against a real Session so a wrong key is caught here.
    #[test]
    fn view_show_hide_toggles_change_the_engine() {
        use figcraft_engine::Session;
        let mut s = Session::new();
        let before = s.ds().unwrap().view.pixel_grid;
        let p = cmd_params(&[], Some(serde_json::json!({})));
        s.execute("view.pixelGrid", &p).unwrap();
        assert_eq!(s.ds().unwrap().view.pixel_grid, !before, "view.pixelGrid must flip the flag");

        let p = cmd_params(&[], Some(serde_json::json!({ "on": true, "scale": 2 })));
        s.execute("view.pixelPreview", &p).unwrap();
        let v = &s.ds().unwrap().view;
        assert!(v.pixel_preview, "`on` must reach the engine");
        assert_eq!(v.pixel_scale, 2, "`scale` must reach the engine");

        let p = cmd_params(&[], Some(serde_json::json!({})));
        s.execute("view.zoom200", &p).unwrap();
        assert_eq!(s.ds().unwrap().view.zoom, 2.0);
    }

    /// The menu's check marks come from the engine's ViewState, not a second UI copy.
    #[test]
    fn view_data_mirrors_the_engine_view_state() {
        let mut v = figcraft_engine::ViewState::default();
        v.zoom = 1.5;
        v.pixel_preview = true;
        v.pixel_scale = 2;
        v.rulers = true;
        v.outlines = true;
        v.pixel_grid = false;
        let d = crate::panels::right::data::ViewData::read(&v);
        assert_eq!(d.zoom, 1.5);
        assert!(d.pixel_preview && d.rulers && d.outlines);
        assert_eq!(d.pixel_scale, 2);
        assert!(!d.pixel_grid);
    }
}

/// 「按钮什么时候显示 / 什么时候隐藏」的回归测试：每条都对应 ui-egui 里的一处
/// `if`（或 `section_header(..., empty)`），防止装配层再次退化成「无条件全画」。
#[cfg(test)]
mod visibility_tests {
    use super::{HeaderSel, PanelSel, visibility};

    /// `lizard right.rs::header` 的分支：Frame / Group / Text / 形状 / 组件 / 已选矢量点。
    #[test]
    fn header_actions_follow_lizard_visibility_rules() {
        let a = |s: HeaderSel| visibility::header_actions(&s);
        // 普通矩形：类型名没有下拉，普通动作齐；单选形状多一个 Edit object。
        let plain = a(HeaderSel::default());
        assert!(!plain.type_menu && !plain.swap_orientation && !plain.text);
        assert!(plain.quick && !plain.edit_object);
        let shape = a(HeaderSel { is_shape: true, count: 1, ..HeaderSel::default() });
        assert!(shape.quick && shape.edit_object, "a single shape shows Edit object");
        // Frame：类型下拉 + 交换方向 + 普通动作。
        let frame = a(HeaderSel { is_frame: true, count: 1, ..HeaderSel::default() });
        assert!(frame.type_menu && frame.swap_orientation && frame.quick && !frame.text);
        // Group：类型下拉，没有交换方向。
        let group = a(HeaderSel { is_group: true, count: 1, ..HeaderSel::default() });
        assert!(group.type_menu && !group.swap_orientation && group.quick);
        // Text：四个专属按钮，普通动作一个都不画（与 quick 互斥）。
        let text = a(HeaderSel { is_text: true, count: 1, ..HeaderSel::default() });
        assert!(text.text && !text.quick && !text.edit_object && !text.type_menu);
        // 多选文本同样走 Text 分支。
        assert!(a(HeaderSel { is_text: true, count: 3, ..HeaderSel::default() }).text);
        // 组件 / 已选矢量点：右侧动作全隐藏，但 Frame 的交换方向仍在（lizard 在 return 之前画）。
        let component = a(HeaderSel { is_component: true, count: 1, ..HeaderSel::default() });
        assert!(!component.quick && !component.text);
        let points = a(HeaderSel { points_selected: true, is_frame: true, count: 1, ..HeaderSel::default() });
        assert!(!points.quick && !points.text && points.type_menu && points.swap_orientation);
        // 顺序：矢量点 / 组件优先于 Text；Text 优先于普通动作。
        let mixed = a(HeaderSel { points_selected: true, is_text: true, count: 1, ..HeaderSel::default() });
        assert!(!mixed.quick && !mixed.text);
    }

    #[test]
    fn bool_union_needs_at_least_two_layers() {
        assert!(!visibility::bool_union_enabled(0));
        assert!(!visibility::bool_union_enabled(1));
        assert!(visibility::bool_union_enabled(2));
    }

    #[test]
    fn distribute_only_for_three_or_more_layers() {
        let mut s = PanelSel::default();
        for n in 0..=4 {
            s.count = n;
            assert_eq!(visibility::distribute(&s), n > 2, "egui gates DistributeH on sel.len() > 2 (count={n})");
        }
    }

    #[test]
    fn fill_rule_only_for_the_fill_section_of_a_vector() {
        assert!(visibility::fill_rule(true, true), "a vector's Fill shows the fill-rule button");
        assert!(!visibility::fill_rule(false, true), "the Stroke section never shows it");
        assert!(!visibility::fill_rule(true, false), "a plain rectangle has no filled regions to set a rule on");
        assert!(!visibility::fill_rule(false, false));
    }

    #[test]
    fn layout_header_button_follows_egui_visibility() {
        // egui `layout_panel.rs::layout_section` 的 `btns` 规则（严格对齐源码）：
        // `in_instance` → 无按钮；`frames && !has_al` → `+`；`has_al` → `−`；
        // 其余（含普通非 frame 图层）→ 无按钮。
        let mut s = PanelSel::default();
        assert_eq!(visibility::layout_header(&s), None, "a non-frame without auto layout shows no button (matches egui)");
        s.all_frame = true;
        assert_eq!(visibility::layout_header(&s), Some("layout.add"), "frames without auto layout show +");
        s.has_al = true;
        assert_eq!(visibility::layout_header(&s), Some("layout.remove"), "auto layout shows -");
        s.in_instance = true;
        assert_eq!(visibility::layout_header(&s), None, "an instance takes its layout from the main: no button");
    }

    /// 端到端：验证引擎 `layout.add` / `layout.remove` 在真实选区上确实加上 / 去掉自动布局。
    /// 注意：header 按钮的**显示**严格按 egui（`frames && !has_al` 才显示 `+`），但引擎命令本身
    /// 支持非 frame 节点（先 wrap 成 frame 再加），这里直接驱动命令以证明行为正确。
    #[test]
    fn layout_add_and_remove_roundtrip_on_real_selection() {
        use crate::app::{CanvasApp, sample_doc};
        let mut app = CanvasApp::new(sample_doc());
        // 选一个还没自动布局的普通矩形 Blue（CanvasApp 默认预选了 AutoLayout·H，这里覆盖）。
        let blue = {
            let ds = app.session.ds().unwrap();
            ds.doc.find_by_name("Blue", Some(ds.page)).into_iter().next().unwrap()
        };
        app.session.set_selection(vec![blue]);
        assert!(app.session.ds().unwrap().doc.node(blue).unwrap().layout.is_none(), "Blue has no auto layout before");

        // 与 header `+` 按钮完全一致的调用：`cmds.push(Cmd { id: "layout.add", params: None })`，
        // 引擎用当前选区作为目标。
        app.session.execute("layout.add", &serde_json::json!({})).expect("layout.add must accept the current selection");

        // 引擎把非 frame 节点先 `wrap` 成一个新 frame，再把自动布局挂到**那个 frame**上
        // （见 engine `structure.rs::layout_add`），与 Figma「选中矩形 → 加自动布局 → 自动套一层 frame」一致。
        // 所以 Blue 本身不改、被收进新 frame；新 frame 才是真正带 layout 的节点。
        let fid = *app.selection().first().expect("layout.add selects the new frame");
        assert_ne!(fid, blue, "layout.add wraps the rectangle in a new frame");
        let f = app.session.ds().unwrap().doc.node(fid).unwrap();
        assert!(f.kind.is_frame_like(), "the wrapper is a frame");
        assert!(f.layout.is_some(), "the wrapper frame gained auto layout");
        assert_eq!(app.session.ds().unwrap().doc.node(blue).unwrap().parent, Some(fid), "Blue is now a child of the auto-layout frame");

        // "-" 把自动布局从 frame 上摘掉（frame 本身保留，Blue 仍在里面）。
        app.session.execute("layout.remove", &serde_json::json!({})).expect("layout.remove must accept the current selection");
        assert!(app.session.ds().unwrap().doc.node(fid).unwrap().layout.is_none(), "after layout.remove the frame no longer has auto layout");
    }

    #[test]
    fn kind_menu_only_for_frame_or_group() {
        assert!(!visibility::kind_menu(false, false), "other layer types have no Frame/Group convert dropdown");
        assert!(visibility::kind_menu(true, false));
        assert!(visibility::kind_menu(false, true));
        assert!(visibility::kind_menu(true, true));
    }

    #[test]
    fn empty_or_mixed_sections_are_dimmed() {
        assert!(visibility::dim_header(None), "a mixed selection has no list");
        assert!(visibility::dim_header(Some(0)), "an empty list dims the header");
        assert!(!visibility::dim_header(Some(1)));
        assert!(!visibility::dim_header(Some(7)));
    }

    /// The command each conditional button raises must exist, otherwise the button is
    /// visible but clicking it is rejected by `Session::execute`.
    #[test]
    fn conditional_buttons_raise_registered_commands() {
        use figcraft_engine::cmd::find_command;
        for id in [
            "distribute.horizontal",
            "distribute.vertical",
            "arrange.tidy",    // DistributeH menu
            "vector.fillRule", // Fill-rule menu
            "layout.add",
            "layout.remove", // Auto layout header
            "object.convertToFrame",
            "object.convertToGroup",
            "frame.swapOrientation", // kind menu
            "fill.add",
            "stroke.add",
            "effect.add",
            "grid.add",
            "export.add", // dim-header whole-row click
            "node.set",   // mixed Export replacement
        ] {
            assert!(find_command(id).is_some(), "conditional button raises unknown command `{id}`");
        }
    }
}
