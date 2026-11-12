//! Monet UI on **GPUI** — 画布按 `ui-egui` 的原实现移植。

//!

//! # 架构（与 egui 版一致）

//! ```text

//! 引擎渲染（后台线程，原生像素密度 scale = zoom × ppp）

//!   -> 一张整页贴图（Retained 增量渲染：平移只补条带、编辑只重画变化区域）

//!   -> 画布把它【重投影】到当前视图显示（平移/缩放零重栅格 → 流畅）

//!   -> 叠加层（选中框/手柄/参考线/标尺/标签）用画笔在图元层绘制

//! 输入：指针事件交给引擎 `Session::pointer`（选择、拖拽、缩放、旋转、框选、吸附都在引擎里）

//! ```

//!

//! 与 egui 版的差别只在后端：`egui::Painter` → [`canvas::painter::Painter`]（同名方法，

//! 底层换成 GPUI `paint_quad` / `paint_path`），`egui::TextureHandle` → `RenderImage`，

//! 文字用 GPUI 文本元素（矢量、清晰）。



mod app;

mod canvas;

/// 标签条的纯几何 / 动画状态（上游 `ui-egui/src/tabs.rs` 的可测部分，P0-1）。

mod tabs;

/// 标签条的 GPUI 视图层（P0-1 第 2 步）。

mod tabs_view;

/// 系统级窗口标题区域：按 GPUI Kit 的 TitleBar 契约声明并渲染（含 WindowControlArea）。

mod title_bar;

/// 工作区 Dock 布局（参考 GPUI Kit 的 `DockLayout` / `DockPlacement`）：布局是一份值。

mod dock;

/// 通用「按住拖动」状态机：尺寸拖动 / 浮窗整卡移动 / 对话框拖动共用一份。

mod hold_drag;
mod render_stack;

/// 通用尺寸拖动状态机：左面板宽度 / 底部时间轴高度共用（thread_local + 窗口轴相对位移）。

mod resize_drag;

/// 画布右键菜单（`ui-egui/src/menus.rs` 的 context menu）。

mod context_menu;

pub mod control;

pub mod control_server;

/// Per-node WGSL 材质绑定：数据模型 / 固定目录扫描 / plugin_data 持久化（无 wgpu，恒编译）。

mod wgsl_material;

/// GPU 渲染后端（§18 第 1、2 步）：`--features gpu` 才编译，主路径仍是 CPU。

#[cfg(feature = "gpu")]

pub mod gpu;

/// WGSL 材质离屏合成器（`--features gpu`）：把 CPU 画布当 sceneTex，逐绑定节点跑 WGSL 多 pass，
/// readback 回 `Arc<RenderImage>` 显示路径。gpui 不暴露材质回调，这是唯一可行路线。
#[cfg(feature = "gpu")]

pub mod wgsl_composite;

/// 通用 WGSL 材质 runner（`--features gpu`）：宿主↔shader 数据交换契约，按 binding 变量名注入
/// 数据（任意 uniform / backdrop / fields / sampler），不认具体 shader、不写死 pass 链。新增任意
/// 符合契约的 `.wgsl` 到固定目录即可用，零宿主改动。多 pass 老通道见 `wgsl_composite`。
#[cfg(feature = "gpu")]

pub mod wgsl_runner;

mod main_menu;

/// 点锚定的菜单卡片：右键菜单与工具栏飞出菜单共用的一套行模型。

mod menu_card;

/// B2: the Actions palette's data (tabs, Recents, grouped commands, ranking). No widgets here.
mod palette;

mod widget_bar;

mod home;

mod comments;

mod icons_panel;

mod board_bar;

mod board_picker;

mod open;

/// A6: the Export dialog's bitmap preview, produced by the engine's renderer.
mod export_preview;

/// The completion audit: plan status is derived from code capabilities, not hand-written.
#[cfg(test)]
mod parity_contract;

mod panels;

/// 左面板控件树（Layers 树）：可展开/折叠、选中、显隐/锁定，含父子级拖动排序（复用 `drag` 通用件）。

mod layer_tree;

/// 五个旧弹窗（自动布局 / 描边 / 取色器 / 变量 / 文本样式）在通用窗口上的装配层。

mod popovers;

/// Prototype 演示层（proto.present / proto.preview 的动画播放）。

mod present;

mod preview;

mod theme;

/// 通用列表 / 树拖拽件：把 egui 的 `list_drag` 原语、layers 的拖动效果与 variables groups 的

/// 「左右改层级」整合为一套几何 + 状态机 + GPUI 视觉件，供任何面板复用。

mod drag;

mod drag_manager;

mod drag_router;

/// Motion (Beta) 动画系统：底部时间轴 + 右栏 inspector + easing + clip 设置浮窗。

/// 控件全部复用 `widgets/`（template::Input 带插槽输入框 / PanelSection 统一网格 / Popover 窗口外壳）。

mod motion;

/// 底部悬浮工具栏。

mod toolbar;

/// 用户可见的功能名（命名规范见 `docs/feature-names.md`）：右栏 tab / a11y / 命名回归测试
/// 共用同一份常量，避免 "Dev Mode" 这类退役文案重新混进界面。
mod naming;

/// 通用对话框框架（A19）：所有对话框共用一套数据模型 + 冻结的 `widgets/` 控件 + 命令队列。
mod dialogs;

/// 通用 GPUI 控件：section header、panel section、icon button 以及输入框系列。

pub mod widgets;



pub use app::{CanvasApp, sample_doc};

pub use preview::export_preview;



use figcraft_i18n::tr;
use std::cell::RefCell;

use std::ops::Range;

use std::rc::Rc;

use std::sync::Arc;

use std::time::{Duration, Instant};



use crate::widgets::menu::{MenuRow, menu_popup_in, popup_backdrop};

use figcraft_doc::{CrossAlign, Document, MainAlign, Node, NodeId, NodeKind};

use figcraft_engine::textedit;

use figcraft_engine::tools::{PointerEvent, PointerKind};

use gpui::prelude::*;

use gpui::{

    AnyElement, App, Application, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, ImageSource, KeyDownEvent,

    KeyUpEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,

    UTF16Selection, WeakEntity, Window, WindowBounds, WindowOptions, div, img, px, size,

};



use canvas::geom::*;

use canvas::render_worker::Worker;

use canvas::{CanvasOutput, Painter};



/// 连击判定：两次按下之间不超过这个时间、且几乎没移动，就算双击。

const DOUBLE_CLICK_MS: u64 = 400;

/// 双击时允许的位移（逻辑像素）。

const DOUBLE_CLICK_SLOP: f32 = 4.0;

/// Actions 面板的尺寸（`ui-egui/src/menus/palette.rs` 的卡片宽 320）。

const PALETTE_W: f32 = 320.0;

const PALETTE_H: f32 = 380.0;



/// 主菜单的目录、几何与外观都在 `main_menu` 模块里（照 `ui-egui/src/menus/tree.rs`）。



/// 缩放档位（figma 的步进，与 `cmd/view.rs` 的 `STEPS` 一致）。

const ZOOM_STEPS: &[f64] = &[0.02, 0.03, 0.06, 0.13, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0];



/// GPUI 的键名（`keystroke.key`）→ 引擎 `keymap` 认的规范键名。

/// `keymap::normalize` 只认 `ArrowLeft`/`Space`/`F1` 这一套，GPUI 给的是 `left`/`space`/`f1`。

/// These are protocol strings, not UI copy: never pass them through `tr()`.

/// The catalogs translate "Delete" / "Space" / "Home" / "End" in 23 of 25 languages,

/// after which `keymap::normalize` no longer recognises the key and the whole shortcut

/// silently stops firing (Delete, Home/End, Shift/Cmd+Delete, Space combos).

fn canonical_key(key: &str) -> String {

    let named = match key {

        "left" => "ArrowLeft",

        "right" => "ArrowRight",

        "up" => "ArrowUp",

        "down" => "ArrowDown",

        "space" => "Space",

        "escape" => "Escape",

        "enter" => "Enter",

        "tab" => "Tab",

        "backspace" => "Backspace",

        "delete" => "Delete",

        "pageup" => "PageUp",

        "pagedown" => "PageDown",

        "home" => "Home",

        "end" => "End",

        other => {

            // F1..F12 在 keymap 里是大写。

            if other.len() >= 2 && other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) {

                return other.to_ascii_uppercase();

            }

            other

        }

    };

    named.to_string()

}



/// 跑一条键盘绑定：照搬 `ui-egui/src/shortcuts.rs::run_binding` 的画布分支，

/// 其余都是引擎命令（撤销/重做、删除、复制粘贴、全选、成组……都在 `keymap` 里）。

pub(crate) fn run_binding(app: &mut CanvasApp, id: &str) -> bool {

    match id {

        "select.none" => app.session.escape().is_ok(),

        // 钢笔还没闭合时 Delete 是「删掉上一个锚点」（原文同一分支）。

        "edit.delete" if app.session.tool.pen.is_some() => app.session.pen_finish().is_ok(),

        _ => app.session.execute(id, &serde_json::json!({})).is_ok(),

    }

}



/// Common value of `f` across `ids` (`None` when mixed or empty) — the same helper `ui-egui`'s

/// right panel uses, so multi-selection shows "Mixed" exactly like the original.

fn common<T: PartialEq + Clone>(doc: &Document, ids: &[NodeId], f: impl Fn(&Node) -> T) -> Option<T> {

    let mut it = ids.iter().filter_map(|i| doc.node(*i));

    let first = f(it.next()?);

    if it.all(|n| f(n) == first) { Some(first) } else { None }

}



fn zoom_step(z: f64, up: bool) -> f64 {

    if up {

        ZOOM_STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(256.0)

    } else {

        ZOOM_STEPS.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(0.02)

    }

}



// B2: the palette's rows moved to `palette.rs` — three tabs, Recents, grouped commands,

// prefix-first ranking and a 60-row cap. `render` calls `palette::sections` before borrowing

// `self.popup`; `palette::items` maps the sections onto the shared menu-card row model.



/// 正在拖拽的分隔线：左面板右边缘 / 右面板左边缘。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub(crate) enum ResizeSide {

    Left,

    Right,

}



/// 一个打开的弹出层（下拉菜单或**通用窗口弹窗**）。由根视图持有，渲染在所有面板之上。

/// 数据（而非闭包）在这里存一份，避免把非 `Send` 的闭包长期挂在视图上；

/// 闭包在 `render` 里每帧从数据重建（与 `menu.rs` 的 `menu_popup` 同源）。

enum Popup {

    /// 下拉菜单：`anchor` 是触发控件的矩形（egui 的 `Response::rect`）——菜单位置取它的左下角，

    /// 宽度取 `anchor.width().max(48)`（`ui-egui` 的 `menu_popup` / `MENU_MIN_W`）。

    Menu {

        anchor: Bounds<Pixels>,

        width: Option<f32>,

        rows: Vec<MenuRow>,

        on_pick: std::rc::Rc<dyn Fn(&str, &mut Window, &mut App)>,

        /// 触发按钮每帧上报的真实矩形：右栏滚动 / 面板变宽后，菜单要跟着按钮一起动，

        /// 不能停在点击那一帧的坐标（用户 2026-11-08）。`None` = 静态锚点（旧调用点）。

        probe: Option<std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,

    },

    /// 主菜单：多级、悬停展开。`menu.rs::menu_popup` 是单层菜单、没有悬停回调，

    /// 装不下「悬停展开 + 子菜单贴行顶对齐」，所以单独一个变体（见 `main_menu` 模块）。

    MainMenu(main_menu::MainMenu),

    /// 五个旧弹窗（自动布局 / 描边 / 取色器 / 变量 / 文本样式）：统一挂在

    /// `widgets::popover::Popover` 这个通用窗口上，标题是一个自定义插槽。

    Window(popovers::WindowState),

    /// 画布右键菜单：贴在**当前指针**的画布局部坐标；render 时按当前画布偏移重算窗口坐标，

    /// 窗口缩放 / 左栏宽度变化时菜单跟着内容走，而不是留在原地（`用户 2026-11-06`）。

    Context {

        pos: Pos2,

        items: Vec<menu_card::Item>,

        open_sub: Option<usize>,

        /// 通用菜单行为（安全三角 / 悬停延迟 / 静止交接）：右键菜单与主菜单共用同一份

        /// `widgets::MenuAim`，斜着划向「Copy as」子菜单不会闪、经过的行不抢展开。

        aim: crate::widgets::MenuAim,

        /// 最近一次指针位置（窗口坐标）；定时结算时没有新鼠标事件，用它。

        pointer: Point<Pixels>,

        born: std::time::Instant,

        /// 只保留一个在飞的延迟唤醒，避免每次 mousemove 都堆 timer。

        aim_busy: std::rc::Rc<std::cell::Cell<bool>>,

    },

    /// 工具栏飞出菜单：贴在触发按钮上方。存的是「组 key」而不是写死坐标，render 时按当前画布

    /// 宽度重算（pill 居中、窗口缩放时重新定位）—— 否则缩放后菜单会停在原坐标不动。

    Flyout { key: &'static str, items: Vec<menu_card::Item> },



    /// 画板工具组的「选项」飞出菜单（便签颜色 / 形状 / 印章）：一排格子，不是工具行



    /// （`ui-egui/src/toolbar.rs::options_flyout`）。



    Options { key: &'static str },



    /// 画板的模板选择器（`crate::board_picker`）。



    Templates,

    /// Actions 面板：搜索框 + 命令列表，贴在指针上方。`at` 是打开时的指针（窗口坐标），

    /// render 时按当前画布宽度重算，窗口缩放时跟着动。

    Palette { at: Point<Pixels> },

    /// Share 弹层（右栏顶栏的 Share 按钮）：锚在按钮上，内容每帧按 live 状态重建

    /// （egui `live::share_popover`）。

    Share { anchor: Bounds<Pixels> },

}



/// R4/R5: egui `UiState::rename_label` / `ui.rename` — a rename is a field drawn *on* the

/// label or the layer row, never a centred dialog. Enter / losing focus commits `object.rename`,

/// Escape cancels.

#[derive(Clone)]

struct InlineRename {

    ids: Vec<figcraft_doc::NodeId>,

    target: InlineRenameTarget,

    /// Just opened: give the field the keyboard on the next frame (egui `Rename::start`).

    start: bool,

    /// Frames since opening: the focus request needs a few frames to land before we may

    /// conclude the edit ended (egui keeps the field open only while it has focus).

    frames: u8,

    /// The field has had the keyboard at least once. Only then may losing focus end the rename;

    /// otherwise a focus request that has not landed yet would immediately close the field.

    was_active: bool,

}



#[derive(Clone, Copy, PartialEq)]

enum InlineRenameTarget {

    /// `canvas.renameLabel`: the top-level frame / section name on the canvas.

    Label,

    /// `layers.rename` with one id: the row inside the layers panel.

    Layer,

    /// A row of the Pages list (`page.rename`).

    Page,

}



/// 画布视图：引擎会话 + 画布状态 + 交互状态。

pub struct CanvasView {

    app: CanvasApp,

    /// 顶部 40pt 标签条的状态（Home / 开场动画 / 拖拽），P0-1。

    tabs: tabs_view::TabState,

    worker: Worker,

    /// 键盘焦点（空格平移、Z 缩放工具要收键盘事件）。

    focus: FocusHandle,

    /// 按下后正在拖拽（Move 事件只在拖拽时送给引擎）。

    dragging: bool,

    /// 中键 / 空格 / Ctrl+右键平移：按下点的屏幕位置与当时的视图中心。

    pan: Option<(Pos2, f64, f64)>,

    /// 这次平移是 **Ctrl+右键** 发起的：`on_mouse_move` 必须把 `Right` 当成「仍按住」，

    /// 否则 `route_canvas_drag` 会按「窗口外松手」立刻收尾，右键根本拖不动。

    right_pan: bool,

    /// 指针最后的位置（画布局部坐标）。

    pointer: Option<Pos2>,

    /// 当前修饰键状态（GPUI 不给「当前状态」，只能从事件里记）。

    mods: Modifiers,

    /// 空格按住：平移工具。

    space: bool,

    /// Z 按住：缩放工具（Alt 缩小）。

    zoom_key: bool,

    /// 缩放工具拖拽的起点。

    zoom_drag: Option<Pos2>,

    /// 上一次按下的时间与位置（判双击）。

    last_down: Option<(Instant, Pos2)>,

    /// 左键当前是否按下（光标要变握拳）。

    primary_down: bool,

    /// 控制通道的请求队列（`--control <port>` 时由 [`control_server`] 往里塞）。

    control_queue: control::Queue,

    /// 左 / 右面板宽度（240..=500，默认 280）。

    left_w: f32,

    right_w: f32,

    /// 正在拖拽哪条分隔线（None = 没在拖）。

    resizing: Option<ResizeSide>,

    /// 拖拽起点（窗口 x）与当时的宽度。

    drag_start_x: f32,

    drag_start_w: f32,

    /// 右栏里缓存的有状态字段（跨帧保留编辑状态）。

    fields: panels::RightFields,

    /// 右栏字段提交的写回队列（`(prop, value)`），render 时统一执行 `node.set`。

    pending: panels::Pending,

    /// 上一次同步到右栏的选中 / 字段指纹，避免每帧重复写字段。

    last_geo: Option<(Vec<NodeId>, [Option<f64>; 14])>,

    /// 上一次同步到右栏的 auto-layout 值指纹（gap / padding / 对齐），同样避免每帧重写。

    last_layout: Option<(Vec<NodeId>, panels::LayoutVals)>,

    /// The column width the fields were last laid out at (`col_width(right_w)`).

    last_col: f32,

    /// 当前打开的弹出层（菜单 / 取色器）；`None` 表示没打开。render 时画在最上层。

    popup: Option<Popup>,

    /// 变量编辑器**独立于 `popup`**（egui 的 `app.ui.variables`：不是 dialog，与主菜单等并存）。

    /// 关掉时只把 `variables_open` 置 false、**保留实体**，这样 Full/Floating、窗口位置、

    /// 选中行、分组折叠都能跨开关记住；再点按钮直接复用，不重建 `WindowState`。

    variables_editor: Option<popovers::WindowState>,

    /// 变量面板是否打开 / 是否全屏（Full）。小窗(Floating)时左栏 Variables 按钮不显示选中；

    /// 全屏时 Variables 选中、File/Assets 取消选中（egui `layers.rs::variables_full` 的等价）。

    variables_open: bool,

    /// 会话级记忆：关闭变量面板**不清零**，下次打开恢复上次的呈现模式。

    variables_full: bool,

    /// 拖动弹窗时右栏数据不变，复用上一帧的 PanelData（省掉每帧 collect / 读列表的开销）。

    last_panel: Option<panels::right::data::PanelData>,

    /// 工具栏里打开着的飞出菜单（组的 key）；`None` 表示没开。



    tool_flyout: Option<&'static str>,



    /// 画板的 Design 面板是否展开（`ui-egui` 的 `app.ui.board_panel`）。画板没有原型、也没有



    /// Dev Mode，右栏默认收起，工具栏最右的按钮 / `view.design` 是它的开关。



    board_panel: bool,



    /// 模板选择器画好的预览图，第一次打开时渲染一次就留着（5 个模板各渲一次很贵）。



    tpl_pics: board_picker::Pictures,

    /// 每个工具组上一次选中的工具（egui 存在 `ui.data` 里的 `tb_last`）。

    tool_last: Vec<(&'static str, figcraft_engine::ToolId)>,

    /// Motion 时间线是否展开（Animation 模式）。**默认收起** —— egui `FigApp::new` 是



    /// `motion_open: false`，而且打开任何文件都会把它关掉（用户 2026-11-10）。



    motion: bool,

    /// Motion 的播放头 / 选中 / 折叠 / 预览状态，以及按钮动作队列（retained tree 用）。

    motion_ui: motion::MotionUi,

    motion_q: motion::Queue,

    /// Motion 打开前的右栏宽度（关闭时还原）。

    motion_right_w: Option<f32>,

    /// 右栏 12 个「带关键帧按钮输入框」的跨帧实体。

    motion_fields: motion::keyframe_field::KeyframeFields,

    /// clip 设置浮窗的 7 个数值字段跨帧实体。

    motion_clip_fields: motion::clip_popover::ClipFields,

    /// 时间轴 header 的两个可编辑时间框（当前时间 / 总时长）。

    motion_times: motion::timeline::TimeFields,

    /// Actions 面板的搜索词（按 Enter 提交）与它自己的搜索框。

    palette_query: String,

    /// B2: the active tab (All / Assets / Plugins & widgets). Opening the palette resets it.

    palette_tab: palette::Tab,

    /// The keyboard-highlighted row (palette Up/Down).
    palette_sel: Option<usize>,

    palette_field: Entity<widgets::TextField>,

    /// 四块整面板的浮层（由引擎 `ui_requests` 打开）与其跨帧状态。

    extra: Option<panels::extra::ExtraKind>,

    extra_state: panels::extra::ExtraState,

    /// 左面板当前 tab：0 = File（Pages/Layers），1 = Assets，2 = Icons。
    /// 对应 egui `panels/layers.rs` 的 `LeftTab`；点左栏 rail 或引擎 `panel.assets` /
    /// `ui.set {leftTab}` 切换这里，左面板据此渲染对应面板（见 R 节 R1/R2）。

    left_tab: u8,

    /// 左栏 Pages 段是否展开（egui `app.ui.pages_open`，默认展开）。

    pages_open: std::rc::Rc<std::cell::RefCell<bool>>,



    /// Pages 段的搜索：搜索框是否打开、以及过滤词（egui `page_search` / `page_filter`）。

    page_search: std::rc::Rc<std::cell::RefCell<bool>>,

    page_filter: std::rc::Rc<std::cell::RefCell<String>>,



    /// 文件名的可编辑字段 + 待提交的新名字（`file.rename`）。

    file_name_field: Entity<widgets::TextField>,

    file_rename: std::rc::Rc<std::cell::RefCell<Option<String>>>,



    /// Pages 段的搜索框（边打边过滤）。

    page_search_field: Entity<widgets::TextField>,



    /// 左栏是否收起（egui `ui.left_collapsed`；文件头的面板按钮切换）。

    left_collapsed: bool,



    /// 底部中间的一条临时提示（egui `app.toast`）：导入失败、拖入被拒这类话必须让用户看见，

    /// 不能像以前那样只在 `FC_DEBUG_CMDS` 下打到 stderr。

    toast_msg: Option<(String, std::time::Instant)>,



    /// R4/R5: the inline rename being edited, if any.

    inline_rename: Option<InlineRename>,



    /// One bare `TextEdit` shared by the canvas label, the pages row and the layers row.

    /// The primitive editor has no Input shell: the row itself is the chrome, as in lizard's

    /// `text_field_inline`. Built once so the caret / focus state survives across frames.

    inline_rename_field: Entity<widgets::TextEdit>,



    /// Escape cancelled the inline rename: a late focus-out commit must not write the draft.

    rename_cancel: bool,



    /// `TextEdit::set_on_commit` must not borrow the view back (commit_edits already holds

    /// `&mut CanvasView`). It only drops the result here; `render` drains it next frame.

    inline_rename_pending: std::rc::Rc<std::cell::RefCell<Option<(InlineRenameTarget, Vec<figcraft_doc::NodeId>, String)>>>,



    /// 上一次同步给 `file_name_field` 的 `(uid, name)`：切标签时才回写，免得打字被打断。

    file_name_seen: Option<(u64, String)>,



    /// B3: the board bar's shape grid is open (the shape button toggles it).
    board_shape_open: std::rc::Rc<std::cell::RefCell<bool>>,

    /// B3: which connector popover is open: 0 none, 1 start cap, 2 end cap, 3 line style.
    board_line_open: std::rc::Rc<std::cell::RefCell<u8>>,

    /// 当前打开的通用对话框（A19）。所有对话框共用 `dialogs::DialogState`，这里只存「有没有、
    /// 是哪一个」，控件状态由它自己保留。

    dialog: Option<dialogs::DialogState>,

    /// 原型演示（None = 未打开）。

    present: Option<present::PresentState>,

    /// 左面板控件树（Layers）的展开 / 选中 / 拖动状态。

    layer_tree: layer_tree::LayerTreeState,

    /// 通用窗口级拖拽层（Kit 范式：`canvas` 覆盖层 + `window.on_mouse_event`）。

    /// 每帧在 Paint 阶段注册 move/up/down 三个窗口级监听，把跨元素 / 跨窗口的拖拽

    /// 统一收口到 `WindowDragTarget` 实现，指针移到哪一层拖拽都不中断。

    drag_layer: drag_manager::WindowDragLayer,

    /// 上一帧的 `on_mouse_up` 刚结束过一次拖拽（窗口级已收尾）。根 div 的 up handler

    /// 据此跳过 plain-click 的 guide 命中，避免拖拽收尾那一下重复触发命中逻辑。

    drag_just_ended: bool,

}



impl CanvasView {

    pub fn new(cx: &mut Context<Self>) -> Self {

        // 渲染完成靠 `in_flight()` 轮询驱动（每帧请求动画帧直到拿到结果）。

        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});

        let mut app = CanvasApp::new(sample_doc());

        // 一个文字层：双击它就能编辑（照搬 `text.create` 命令的用法）。

        let _ = app.session.execute("text.create", &serde_json::json!({"text": "Hello", "x": 60.0, "y": 430.0}));

        // 标尺默认关（和引擎一致），这里按用户会做的操作打开（Shift+R）。

        let _ = app.session.execute("view.rulers", &serde_json::json!({"on": true}));

        // 界面语言：与原文 `FigApp::new` 同一句 —— `session.prefs.language` 是 `system` 时，

        // 用系统偏好的语言列表来选目录（`i18n::set` 会挑列表里第一个有目录的）。

        figcraft_i18n::set(&app.session.prefs.language, system_language().as_deref());

        // 右栏字段提交后要唤醒视图去 drain 写回队列（按 Enter 提交时画布不会自己重绘）。

        let (tx, rx) = smol::channel::unbounded::<()>();

        cx.spawn(async move |this, cx| {

            while rx.recv().await.is_ok() {

                if this.update(cx, |_, cx| cx.notify()).is_err() {

                    break;

                }

            }

        })

        .detach();

        // 实时协作的传输层：与 `apps/figcraft/src/main.rs` 同一句（`NetLink` + 唤醒 UI）。

        // 网络线程收到东西就 `try_send` 一帧唤醒 —— 和右栏字段提交用的是同一个通道。

        let live_wake: Arc<dyn Fn() + Send + Sync> = {

            let tx = tx.clone();

            Arc::new(move || {

                let _ = tx.try_send(());

            })

        };

        app.session.live.install(Box::new(figcraft_collab_net::NetLink::new(live_wake)));

        let panel_wake: Rc<dyn Fn()> = Rc::new(move || {

            let _ = tx.try_send(());

        });

        // Motion 动作队列共用同一个唤醒器：按钮 push 之后必有一帧来 drain 并写回引擎。

        let motion_q = motion::Queue::new(panel_wake.clone());

        let motion_fields = motion::keyframe_field::KeyframeFields::new(cx, 0.0, &motion_q);

        let motion_clip_fields = motion::clip_popover::ClipFields::new(cx, &motion_q);

        let motion_times = motion::timeline::TimeFields::new(cx, &motion_q);

        let pending: panels::Pending = Rc::new(RefCell::new(Vec::new()));

        // 右栏字段是有状态实体：只建一次，跨帧复用（不能每帧重建，否则输入状态会丢）。

        let fields = panels::RightFields::new(cx, &pending, &panel_wake);

        // Actions 面板的搜索框：Enter 提交时把搜索词写进视图，列表按它过滤。

        let palette_field = widgets::search_field(cx, 288.0, "", "Search actions");



        // R4/R5: the one inline rename editor — the primitive TextEdit, not the shelled

        // TextField. Its size comes from the row / label that hosts it.

        let inline_rename_field = widgets::TextEdit::build(cx, |te, _| {

            te.set_inset_left(0.0);

            te.set_right_pad(0.0);

            // Same size as the layer label it replaces: two sizes mean two line boxes, and
            // centring each one in the row leaves the text a pixel or two lower.

            te.set_font_size(px(11.0));

        });

        let extra_state = panels::extra::ExtraState::new(cx, &fields.queue());

        let mut view = Self {

            app,

            tabs: tabs_view::TabState::default(),

            worker: Worker::new(wake),

            focus: cx.focus_handle(),

            dragging: false,

            pan: None,

            right_pan: false,

            pointer: None,

            mods: Modifiers::default(),

            space: false,

            zoom_key: false,

            zoom_drag: None,

            last_down: None,

            primary_down: false,

            control_queue: Arc::new(std::sync::Mutex::new(Vec::new())),

            left_w: panels::DEFAULT_PANEL,

            right_w: panels::DEFAULT_PANEL,

            resizing: None,

            drag_start_x: 0.0,

            drag_start_w: 0.0,

            fields,

            pending,

            last_geo: None,

            last_layout: None,

            last_col: 0.0,

            popup: None,

            variables_editor: None,

            variables_open: false,

            variables_full: false,

            last_panel: None,

            tool_flyout: None,



            board_panel: false,



            pages_open: std::rc::Rc::new(std::cell::RefCell::new(true)),

            page_search: std::rc::Rc::new(std::cell::RefCell::new(false)),

            page_filter: std::rc::Rc::new(std::cell::RefCell::new(String::new())),

            file_name_field: widgets::TextField::build(cx, 180.0, |tf, _| {

                tf.set_placeholder(tr("Untitled"));

            }),

            file_rename: std::rc::Rc::new(std::cell::RefCell::new(None)),

            page_search_field: widgets::search_field(cx, 200.0, "", "Search pages"),

            left_collapsed: false,



            toast_msg: None,

            file_name_seen: None,



            tpl_pics: std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new())),

            tool_last: {

                // 每个组默认显示它的第一个工具（egui 的 `remembered(...).unwrap_or(first)`）。

                // Keyed by group key — egui stores one `tb_last` per key, so a board's nine groups

                // and a design file's six groups each remember their own tool.

                let mut last: Vec<(&'static str, figcraft_engine::ToolId)> = Vec::new();

                for g in toolbar::GROUPS.iter().chain(toolbar::BOARD_GROUPS) {

                    if last.iter().any(|(k, _)| *k == g.key) {

                        continue;

                    }

                    let first = g.tools.first().map(|t| t.0).unwrap_or(figcraft_engine::ToolId::Move);

                    last.push((g.key, first));

                }

                last

            },

            motion: false,

            motion_ui: motion::MotionUi::default(),

            motion_q,

            motion_right_w: None,

            motion_fields,

            motion_clip_fields,

            motion_times,

            palette_query: String::new(),

            palette_tab: palette::Tab::All,

            palette_sel: Some(0),

            palette_field,



            inline_rename: None,



            inline_rename_field,



            rename_cancel: false,



            inline_rename_pending: std::rc::Rc::new(std::cell::RefCell::new(None)),

            extra: None,

            extra_state,

            left_tab: 0,

            board_shape_open: std::rc::Rc::new(std::cell::RefCell::new(false)),

            board_line_open: std::rc::Rc::new(std::cell::RefCell::new(0u8)),

            dialog: None,

            present: None,

            layer_tree: layer_tree::LayerTreeState::default(),

            drag_layer: drag_manager::WindowDragLayer::new(),

            drag_just_ended: false,

        };

        // 启动即 Motion 模式（egui `set_open(true)`）：右栏加宽到 inspector 的 413pt，

        // 并记住原宽度，切回 Design 时由 `set_motion_open(false)` 还原。

        view.motion_right_w = Some(view.right_w);

        view.right_w = motion::INSPECTOR_W;

        // 仅供 UI 核对：`FC_DEBUG_POPUP=layout|stroke|color|variables|styles` 启动时直接打开

        // 对应的弹窗（合成数据），省去手工点按钮。生产构建不带这个环境变量时完全不生效。

        if let Ok(kind) = std::env::var("FC_DEBUG_POPUP") {

            // 仅供 UI 核对：加 `_full` 后缀直接以 Full 呈现打开，省去手工切模式。

            if kind == "variables_table_full" {

                view.variables_full = true;

            }

            let data = view.panel_data();

            let queue = view.fields.queue();

            // x is deliberately irrelevant — `WindowState::render` puts every window against the

            // Design panel's left edge. 140 is egui's default `picker_y` for a detached panel.

            let anchor = Point { x: px(0.0), y: px(140.0) };

            let rows = vec![

                crate::popovers::Row::new("Brand/Primary", "variable.bind", serde_json::json!({ "field": "fill" }))

                    .group(tr("Collection"))

                    .trail(tr("Color")),

                crate::popovers::Row::new("Space/Medium", "variable.bind", serde_json::json!({ "field": "gap" })).group(tr("Collection")).trail(tr("Number")),

                crate::popovers::Row::new("Body/Large", "style.apply", serde_json::json!({ "style": "body-large" })).group(tr("This file")).trail(tr("Text")),

            ];

            let w = match kind.as_str() {

                "layout" => Some(popovers::WindowState::layout(queue, &data, cx, anchor)),

                "stroke" => Some(popovers::WindowState::stroke(queue, &data, cx, anchor)),

                "variables" => Some(popovers::WindowState::variables(queue, cx, anchor, rows)),

                "styles" => Some(popovers::WindowState::styles(queue, cx, anchor, tr("Text styles"), rows)),

                // 变量编辑器右侧列表：文档里没有变量时用合成数据，方便核对表格外观。

                "variables_table" | "variables_table_full" => {

                    let t = if data.variables.collections.is_empty() {

                        crate::panels::right::variables::sample()

                    } else {

                        data.variables.clone()

                    };

                    Some(popovers::WindowState::variables_table(queue.clone(), anchor, t))

                }

                "color" => {

                    let on_pick = Rc::new(|_c: Color32, _w: &mut Window, _cx: &mut App| {});

                    Some(popovers::WindowState::color(cx, anchor, Color32::from_rgb(0x2D, 0x9C, 0xDB), on_pick, view.fields.queue()))

                }

                _ => None,

            };

            if let Some(w) = w {

                view.open_window(w, cx);

            }

            // 新控件同法：`FC_DEBUG_POPUP=context|flyout|palette` 开局就打开右键菜单 / 飞出菜单 /

            // Actions，方便在没有鼠标注入的情况下核对渲染。

            if kind == "context" {

                view.open_context_menu(pos2(360.0, 300.0), cx);

            } else if kind == "flyout" {

                view.tool_flyout = Some("shape");

                view.popup = view.flyout_at("shape");

            } else if kind == "palette" {

                view.popup = Some(view.palette_at_pointer(Point { x: px(600.0), y: px(600.0) }));

            }

        }

        // Actions 面板：边打边过滤（egui palette 每敲一个字就重排），Enter 执行高亮行。

        let me = cx.entity();



        view.palette_field.update(cx, |tf, _| {

            // 每改一个字符就重排，并把键盘高亮复位到第一行；与 commit 互不影响。

            tf.set_on_change({

                let me = me.clone();

                move |text, _w, cx| {

                    let q = text.trim().to_string();

                    me.update(cx, |v, cx| {

                        v.palette_query = q;

                        v.palette_sel = Some(0);

                        cx.notify();

                    });

                }

            });

            // `palette_query` 已由 on_change 同步，所以 Enter 直接运行高亮行；

            // 不再有「第一次 Enter 过滤、第二次 Enter 才执行」这种两段式行为。

            tf.set_on_commit(move |_text, _w, cx| {

                me.update(cx, |v, cx| {

                    let items = palette::items(&palette::sections(&v.palette_query, v.palette_tab, &v.app.session));

                    if let Some(item) = v.palette_sel.and_then(|i| items.get(i)) {

                        let key = item.key.to_string();

                        v.pick_palette(&key, cx);

                    }

                });

            });

        });

        view

    }



    /// 选中节点的公共值快照（多选走 `common()`：不一致即 `None` → "Mixed"）。

    /// 顺序：`[x, y, w, h, rotation°, opacity%, cornerRadius]`。

    /// X/Y 用 `doc.frame_relative`（与 egui 的 `x_of` / `y_of` 一致：组不占坐标系）。

    /// 右栏 section 的显示条件（照抄 egui `design()` 的判定）。

    fn panel_sel(&self) -> panels::PanelSel {

        let Ok(ds) = self.app.session.ds() else {

            return panels::PanelSel { empty: true, ..Default::default() };

        };

        if ds.selection.is_empty() {

            return panels::PanelSel { empty: true, ..Default::default() };

        }

        let nodes: Vec<&Node> = ds.selection.iter().filter_map(|i| ds.doc.node(*i)).collect();

        let has_al = !nodes.is_empty() && nodes.iter().all(|n| n.layout.is_some());

        let layout_dir = if has_al {

            let first = nodes.first().and_then(|n| n.layout.as_ref().map(|l| l.mode));

            if nodes.iter().all(|n| n.layout.as_ref().map(|l| l.mode) == first) {

                match first {

                    Some(figcraft_doc::LayoutMode::Vertical) => panels::LayoutDir::Vertical,

                    Some(figcraft_doc::LayoutMode::Horizontal) => panels::LayoutDir::Horizontal,

                    Some(figcraft_doc::LayoutMode::Grid) => panels::LayoutDir::Grid,

                    _ => panels::LayoutDir::None,

                }

            } else {

                panels::LayoutDir::None

            }

        } else {

            panels::LayoutDir::None

        };

        // 按钮显隐条件（照 egui 对应函数逐个算，不让装配层自己猜）。

        let in_instance = ds.selection.iter().any(|i| figcraft_engine::components::owning_instance(&ds.doc, *i).is_some());

        let has_vector = nodes.iter().any(|n| matches!(n.kind, NodeKind::Vector { .. }));

        panels::PanelSel {

            empty: false,

            all_ellipse: !nodes.is_empty() && nodes.iter().all(|n| matches!(n.kind, NodeKind::Ellipse { .. })),

            all_points: !nodes.is_empty() && nodes.iter().all(|n| matches!(n.kind, NodeKind::Polygon { .. } | NodeKind::Star { .. })),

            has_text: nodes.iter().any(|n| matches!(n.kind, NodeKind::Text(_))),

            all_frame: !nodes.is_empty() && nodes.iter().all(|n| n.kind.is_frame_like()),

            has_al,

            layout_dir,

            count: ds.selection.len(),

            has_vector,

            in_instance,

        }

    }



    fn selected_values(&self) -> Option<(Vec<NodeId>, [Option<f64>; 14])> {

        let ds = self.app.session.ds().ok()?;

        let ids = ds.selection.clone();

        if ids.is_empty() {

            return None;

        }

        let doc: &Document = &ds.doc;

        let r2 = |v: f64| (v * 100.0).round() / 100.0;

        let x = common(doc, &ids, |n| r2(doc.frame_relative(n.id).as_coeffs()[4]));

        let y = common(doc, &ids, |n| r2(doc.frame_relative(n.id).as_coeffs()[5]));

        let w = common(doc, &ids, |n| r2(n.width));

        let h = common(doc, &ids, |n| r2(n.height));

        let rot = common(doc, &ids, |n| r2(figcraft_geom::rotation_degrees(n.transform)));

        let op = common(doc, &ids, |n| (f64::from(n.opacity) * 100.0).round());

        let corner = common(doc, &ids, |n| r2(n.corners.uniform().unwrap_or(0.0)));

        // 只有对应形状才有值；类型不一致 → None（显示 Mixed）。

        let arc_start = common(doc, &ids, |n| match &n.kind {

            NodeKind::Ellipse { start, .. } => Some(r2(start.to_degrees())),

            _ => None,

        })

        .flatten();

        let arc_sweep = common(doc, &ids, |n| match &n.kind {

            NodeKind::Ellipse { sweep, .. } => Some(r2(sweep.to_degrees())),

            _ => None,

        })

        .flatten();

        let arc_inner = common(doc, &ids, |n| match &n.kind {

            NodeKind::Ellipse { inner, .. } => Some((inner * 100.0).round()),

            _ => None,

        })

        .flatten();

        let points = common(doc, &ids, |n| match &n.kind {

            NodeKind::Star { count, .. } | NodeKind::Polygon { count } => Some(f64::from(*count)),

            _ => None,

        })

        .flatten();

        let star_ratio = common(doc, &ids, |n| match &n.kind {

            NodeKind::Star { inner_ratio, .. } => Some((inner_ratio * 100.0).round()),

            _ => None,

        })

        .flatten();

        let stroke = common(doc, &ids, |n| r2(n.stroke.weight));

        Some((ids, [x, y, w, h, rot, op, corner, arc_start, arc_sweep, arc_inner, points, star_ratio, None, stroke]))

    }



    /// Selection's common auto-layout values for the right panel's Auto layout section.

    /// Mirrors `common()` like `selected_values`: `None` = mixed / no auto layout → "Mixed".

    /// 右栏列表数据的快照（照 egui 面板每帧读 `Document` 的同构做法）。

    fn panel_data(&self) -> panels::right::data::PanelData {

        let Ok(ds) = self.app.session.ds() else { return Default::default() };

        let page = self.app.session.page().ok();

        let mut d = panels::right::data::read(&ds.doc, &ds.selection, page, self.fields.dev_ready_filter());

        // lizard `Id::new(("selection_colors_open", ds.uid))`：展开态按文档 uid 分档。
        d.doc_uid = ds.uid;

        // A3: the Comments panel's list, from the panel's own filter/sort/search.
        let comment_me = figcraft_engine::cmd::comment::user_name(&self.app.session);
        {
            let cu = self.fields.comment_ui();
            let cu = cu.borrow();
            d.comments = comments::read_rows(&ds.doc, &comment_me, cu.filter, cu.sort, &cu.search);
        }
        d.comment_me = comment_me;
        d.comment_tool = self.app.session.tool.tool == figcraft_engine::tools::ToolId::Comment;
        d.comments_total = ds.doc.comments.len();

        // 变量编辑器还要列已启用库的只读集合（egui `View::of(lib)`）。

        d.variables.libraries = panels::right::variables::read_libraries(&self.app.session.libraries, &ds.doc);

        // Zoom / Show 菜单的勾选状态来自引擎的 ViewState，不从 UI 另存一份。

        d.view = panels::right::data::ViewData::read(&ds.view);

        // 右栏顶栏（头像 / Present / Share）的 live 快照：同样每帧从引擎现读。

        d.live = panels::right::data::LiveData::read(&self.app.session);

        // Frame 工具：Design 标签整页换成 Frame 预设（egui 同款分支）。

        d.frame_tool = self.app.session.tool.tool == figcraft_engine::tools::ToolId::Frame;

        // Inspect 标签（Dev Mode）：属性分组 + 生成的代码，按当前语言 / 颜色格式 / rem 计算。

        if !ds.selection.is_empty() {

            use figcraft_engine::codegen::CodeLang;

            use figcraft_engine::devinspect::ColorFormat;

            let (li, fi, rem) = self.fields.inspect_sel();

            let lang = CodeLang::ALL.get(li).copied().unwrap_or_default();

            let fmt = ColorFormat::ALL.get(fi).copied().unwrap_or_default();

            d.inspect = Some(panels::right::data::read_inspect(&ds.doc, &ds.selection, lang, fmt, rem, ds.revision));

        }

        // 矢量编辑：选中的点驱动 Points 节（替换 Position / Auto layout / Arc / Star）。

        if let Some(ve) = ds.vector_edit.as_ref() {

            d.vector = Some(panels::right::data::read_vector(&ds.doc, ve));

        }

        // Scale 工具激活时，右栏用 Scale 节替换 Position（egui `right.rs` 同款分支）。

        if self.app.session.tool.tool == figcraft_engine::tools::ToolId::Scale

            && let Some(b) = ds.doc.bounds_of(&ds.selection)

        {

            let (ax, ay) = self.app.session.tool.scale_anchor;

            let (ax, ay) = (ax.min(2), ay.min(2));

            // `scale_about`: 支点 = 包围盒按锚点在 0 / ¼ / ½ 处取值（锚点 0/1/2 → 0.0/0.5/1.0）。

            let (fx, fy) = (f64::from(ax) / 2.0, f64::from(ay) / 2.0);

            d.scale = Some(panels::right::data::ScaleData {

                w: b.width(),

                h: b.height(),

                anchor: (ax, ay),

                cx: b.x0 + b.width() * fx,

                cy: b.y0 + b.height() * fy,

            });

        }

        d

    }



    fn selected_layout(&self) -> Option<(Vec<NodeId>, panels::LayoutVals)> {

        let ds = self.app.session.ds().ok()?;

        let ids = ds.selection.clone();

        if ids.is_empty() {

            return None;

        }

        let doc: &Document = &ds.doc;

        let main_name = |m: MainAlign| match m {

            MainAlign::Min => "min",

            MainAlign::Center => "center",

            MainAlign::Max => "max",

            MainAlign::SpaceBetween => "spaceBetween",

            MainAlign::SpaceAround => "spaceAround",

            MainAlign::SpaceEvenly => "spaceEvenly",

        };

        let cross_name = |c: CrossAlign| match c {

            CrossAlign::Min => "min",

            CrossAlign::Center => "center",

            CrossAlign::Max => "max",

            CrossAlign::Baseline => "baseline",

        };

        let content_name = |c: figcraft_doc::ContentAlign| match c {

            figcraft_doc::ContentAlign::Auto => "auto",

            figcraft_doc::ContentAlign::SpaceBetween => "spaceBetween",

        };

        let v = panels::LayoutVals {

            gap: common(doc, &ids, |n| n.layout.as_ref().map(|l| l.gap)).flatten(),

            cross_gap: common(doc, &ids, |n| n.layout.as_ref().map(|l| l.cross_gap)).flatten(),

            padding: common(doc, &ids, |n| n.layout.as_ref().map(|l| l.padding)).flatten(),

            main_align: common(doc, &ids, |n| n.layout.as_ref().map(|l| main_name(l.main_align))).flatten(),

            cross_align: common(doc, &ids, |n| n.layout.as_ref().map(|l| cross_name(l.cross_align))).flatten(),

            wrap: common(doc, &ids, |n| n.layout.as_ref().map(|l| l.wrap)).flatten().unwrap_or(false),

            content_align: common(doc, &ids, |n| n.layout.as_ref().map(|l| content_name(l.content_align))).flatten(),

        };

        Some((ids, v))

    }



    /// 画布 ↔ 右栏联动（照抄 egui）：

    /// 1. 右栏字段提交 → `node.set`（`ids` 数组，多选一起改）；

    /// 2. 选中 / 几何变化 → 把公共值刷进缓存字段（不一致显示 "Mixed"）。

    /// 提交右栏所有正在编辑的草稿，并**立刻**把命令跑掉。

    ///

    /// ⛔ 只调 `commit_all` 是不够的：提交只是把命令塞进队列，真正执行要等下一帧的

    /// `sync_panel`；而点画布这一下会在同一帧里把选区清空，等下一帧执行时

    /// `node.set` 已经拿不到 ids 了 —— 表现就是「输入完点画布，值没提交进去」。

    /// 所以这里在事件处理中、选区还活着的时候就把队列抽干执行。

    fn commit_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {

        self.fields.commit_all(window, cx);

        // Inline rename: clicking anywhere outside the bare editor commits it. The callback

        // only fills the pending queue (it must not borrow the view back), so `render` executes

        // the command on the next frame.

        let editor = self.inline_rename_field.clone();

        editor.update(cx, |te, cx| { te.finish_edit(window, cx); });

        if self.inline_rename.is_some() && self.inline_rename_pending.borrow().is_none() {

            // Unchanged text produces no commit, but the field still closes (egui's rule).

            self.inline_rename = None;

        }

        self.sync_panel(cx);

    }



    /// 引擎 `ui_requests` 通道（egui `FigApp::handle_ui_requests` 的 GPUI 等价）。

    ///

    /// `view.devMode` / `help.shortcuts` / `file.versionHistory` / `view.devCompare` 等命令只往

    /// `session.ui_requests` 里放请求；UI 每帧取走，决定切哪个右栏标签、开哪块面板。

    /// 接上这条通道后，引擎 keymap 里的 `Shift+D`（Dev Mode）、`Shift+E`、`Ctrl+Shift+?`

    /// 才真正有 UI 反应（以前 GPUI 完全不消费这些请求，我只能用右栏状态硬同步）。

    fn sync_ui_requests(&mut self, cx: &mut Context<Self>) {

        if *self.extra_state.dismiss.borrow() {

            *self.extra_state.dismiss.borrow_mut() = false;

            self.extra = None;

        }

        // 通用对话框：X / 遮罩 / 按钮统一置位，下一帧由这里收掉（与 four extra 面板同一机制）。

        if self.dialog.as_ref().is_some_and(|d| d.take_dismiss()) {

            self.dialog = None;

        }

        let reqs = std::mem::take(&mut self.app.session.ui_requests);

        if reqs.is_empty() {

            return;

        }

        for r in reqs {

            // R4/R5/R6 (egui `app/commands.rs:135,252,255`): these are inline edits or a direct

            // engine call, never a centred dialog. Catch them **before** `dialogs::ui_action`.

            match r.kind.as_str() {

                "canvas.renameLabel" => {

                    if let Some(id) = r.data.get("id").and_then(figcraft_engine::params::id_of) {

                        self.begin_inline_rename(vec![id], InlineRenameTarget::Label, cx);

                    }

                    continue;

                }

                "layers.rename" => {

                    let ids: Vec<figcraft_doc::NodeId> = r.data.get("ids")

                        .and_then(serde_json::Value::as_array).into_iter().flatten()

                        .filter_map(figcraft_engine::params::id_of).collect();

                    if ids.len() > 1 {

                        // egui `dialogs::open_batch_rename`: only a multi-selection is a dialog.

                        self.open_dialog(dialogs::Kind::Rename, &r.data, cx);

                    } else if let Some(&id) = ids.first() {

                        // egui `layers::start_rename`: open the ancestors, select the layer,

                        // then put the field in its row.

                        let ancestors: Vec<_> = self.app.session.ds().ok()

                            .map(|ds| ds.doc.ancestors(id)).unwrap_or_default();

                        for a in ancestors { self.layer_tree.expanded.insert(a); }

                        let _ = self.app.session.execute("select.set", &serde_json::json!({ "ids": [id] }));

                        self.begin_inline_rename(vec![id], InlineRenameTarget::Layer, cx);

                    }

                    continue;

                }

                "board.editText" => {

                    if let Some(id) = r.data.get("id").and_then(figcraft_engine::params::id_of)

                        && let Err(e) = figcraft_engine::board::edit_text(&mut self.app.session, id)

                    {

                        self.toast(e.to_string(), cx);

                    }

                    continue;

                }

                _ => {}

            }

            // A19：所有 `dialog.*` 请求（以及少数需要输入的命令）先走通用对话框框架。
            // 框架返回 None 的请求继续交给下面四块整面板的 `ui_action`，互不遮挡。

            if let Some(kind) = dialogs::ui_action(&r.kind, &r.data) {

                // A18：`file.close` 拒绝脏文档时带上被关闭标签的下标。先切到那个标签，
                // 否则三问里的 Save 会保存当前活动文档（可能不是用户要关的那个）。

                if kind == dialogs::Kind::Unsaved {

                    if let Some(i) = r.data.get("index").and_then(serde_json::Value::as_u64) {

                        let _ = self.app.session.execute("doc.switch", &serde_json::json!({ "index": i }));

                    }

                }

                self.open_dialog(kind, &r.data, cx);

                continue;

            }

            // `ui.set` 是控制通道改 UI 状态的入口（egui control.rs:1112）：motion 开关、

            // playhead、rightTab、fps。它不走引擎命令，直接落到视图状态。

            if r.kind == "ui.set" {

                if let Some(on) = r.data.get("motion").and_then(serde_json::Value::as_bool) {

                    self.set_motion_open(on, cx);

                }

                if let Some(t) = r.data.get("playhead").and_then(serde_json::Value::as_f64) {

                    self.motion_ui.playhead_ms = t.max(0.0);

                }

                if let Some(tab) = r.data.get("rightTab").and_then(serde_json::Value::as_str) {

                    let idx = match tab { "prototype" => 1, "inspect" | "devMode" => 2, _ => 0 };

                    self.fields.set_right_tab(idx);

                }

                // `ui.set {leftTab}`：egui 用它切左栏的 File / Assets / Icons。GPUI 把这几块
                // 收进左面板的 tab（见 R 节 R1/R2），不再弹浮层：`set` 语义是幂等的，不是 toggle。
                if let Some(tab) = r.data.get("leftTab").and_then(serde_json::Value::as_str) {

                    match tab {

                        "icons" => { self.left_tab = 2; self.extra = None; }

                        "assets" => { self.left_tab = 1; self.extra = None; }

                        // `file`（以及未知值）就是图层面板：收起浮层并回到 tab 0。
                        _ => {

                            self.left_tab = 0;

                            // R.4: Assets / Icons are left-panel tabs now, not `ExtraKind`

                            // variants. `ui.set {leftTab}` is the only place that closes them.

                        }

                    }

                }

                if let Some(fps) = r.data.get("fps").and_then(serde_json::Value::as_u64) {

                    // Route to the selected frame's scope, matching the in-UI fps writes.
                    let scope = self.app.session.ds().ok().map(|ds| {
                        let sel = self.app.session.selection();
                        crate::motion::scope_of(&ds.doc, &sel, ds.page)
                    });
                    let params = match scope {
                        Some(s) => serde_json::json!({ "fps": fps, "page": s.0 }),
                        None => serde_json::json!({ "fps": fps }),
                    };
                    let _ = self.app.session.execute("motion.timeline", &params);

                }

                // 控制通道打开 / 关闭变量面板（截图与端到端验证用；等价于点左栏 Variables）。

                if let Some(on) = r.data.get("variables").and_then(serde_json::Value::as_bool) {

                    if on && !self.variables_open {

                        let data = self.panel_data();

                        let win = popovers::WindowState::variables_table(

                            self.fields.queue(),

                            Point { x: px(0.0), y: px(100.0) },

                            data.variables,

                        );

                        self.open_window(win, cx);

                    } else if !on && self.variables_open {

                        self.close_variables(cx);

                    }

                }

                if let Some(full) = r.data.get("variablesFull").and_then(serde_json::Value::as_bool) {

                    self.set_variables_full(full, cx);

                }

                continue;

            }

            let Some(action) = panels::extra::ui_action(&r.kind, &r.data, self.fields.right_tab()) else { continue };

            match action {

                panels::extra::UiAction::Tab(tab) => self.fields.set_right_tab(tab),

                panels::extra::UiAction::Toggle(kind) => self.toggle_extra(kind),

                // `panel.assets` / `panel.libraries`：切到左面板的 Assets tab（R1），不弹浮层。
                panels::extra::UiAction::LeftTab(tab) => { self.left_tab = tab; self.extra = None; },

                panels::extra::UiAction::Compare { id, version } => {

                    self.extra = Some(panels::extra::ExtraKind::DevCompare { id, version: version.clone() });

                    *self.extra_state.dev_mode.borrow_mut() = panels::extra::DevMode::SideBySide;

                    *self.extra_state.dev_version.borrow_mut() = version.clone();

                    *self.extra_state.dev_split.borrow_mut() = 0.5;

                    *self.extra_state.dev_hovered.borrow_mut() = None;

                }

                panels::extra::UiAction::Widths { left, right } => {

                    let reset = left.is_none() && right.is_none();

                    let default = panels::DEFAULT_PANEL;

                    self.left_w = left.unwrap_or(if reset { default } else { self.left_w });

                    self.right_w = right.unwrap_or(if reset { default } else { self.right_w });

                }

                panels::extra::UiAction::Palette => {

                    let p = self.palette_at_pointer(gpui::point(px(400.0), px(300.0)));

                    self.popup = Some(p);

                }

                panels::extra::UiAction::Present(start, preview) => {

                    let id = start.or_else(|| {

                        self.app.session.ds().ok().and_then(|ds| ds.doc.top_frames(ds.page).into_iter().next())

                    });

                    if let Some(id) = id {

                        self.present = Some(present::PresentState::new(id, preview));

                    }

                }

                panels::extra::UiAction::PreviewClose => self.present = None,

            }

        }

    }



    /// Text 选区标题行的「Create link」：`lizard menus::invoke("text.link")` 先弹 URL 输入框，
    /// 由对话框按钮把 `{"url": …}` 交给 `text.link`。右栏按钮直接调这个入口。
    pub(crate) fn open_link_dialog(&mut self, cx: &mut Context<Self>) {
        self.open_dialog(dialogs::Kind::Link, &serde_json::json!({}), cx);
    }

    /// A19：打开一个通用对话框。菜单命令与引擎 `ui_requests` 都走这一个入口，
    /// 所有对话框共用 `DialogState` 的壳 / 网格 / 控件 / 命令队列 / 关闭行为。
    fn open_dialog(&mut self, kind: dialogs::Kind, data: &serde_json::Value, cx: &mut Context<Self>) {

        // A19 SaveAs: a native "Save as…" picker (like egui's `pick_file`), not a fake path field.
        if kind == dialogs::Kind::SaveAs {
            if let Some(path) = rfd::FileDialog::new().set_file_name("Untitled.figcraft").add_filter(tr("Monet"), &["figcraft"]).save_file() {
                let _ = self.app.session.execute("file.saveAs", &serde_json::json!({ "path": path.to_string_lossy() }));
            }
            if matches!(self.popup, Some(Popup::MainMenu(_))) {
                self.close_popup(cx);
            }
            cx.notify();
            return;
        }

        let cmds = self.fields.queue();

        // A6: only the Export dialog needs a bitmap, and it comes from the engine renderer.

        let preview = (kind == dialogs::Kind::Export).then(|| export_preview::make(&self.app.session)).flatten();

        // A7: the Preferences form shows the live values; its request carries none, so snapshot
        // the prefs into `data` (the spec is a pure function of it).
        let data: std::borrow::Cow<'_, serde_json::Value> = match kind {
            dialogs::Kind::Preferences => {
                let p = &self.app.session.prefs;
                std::borrow::Cow::Owned(serde_json::json!({ "prefs": {
                    "keep_tool": p.keep_tool, "highlight_on_hover": p.highlight_on_hover,
                    "rename_duplicates": p.rename_duplicates, "show_dimensions": p.show_dimensions,
                    "number_keys_opacity": p.number_keys_opacity, "reduce_motion": p.reduce_motion,
                    "auto_save": p.auto_save, "start_on_home": p.start_on_home,
                    "user_name": p.user_name, "publish_server": p.publish_server,
                    "nudge_small": p.nudge_small, "nudge_big": p.nudge_big,
                    "theme": p.theme, "language": p.language,
                }}))
            }
            dialogs::Kind::Plugins => {
                // A4: the Installed list is the live `prefs.plugins`, not a stub.
                let plugins: Vec<serde_json::Value> = self
                    .app
                    .session
                    .prefs
                    .plugins
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "id": p.manifest.id, "name": p.manifest.name,
                            "version": p.manifest.version, "description": p.manifest.description,
                        })
                    })
                    .collect();
                std::borrow::Cow::Owned(serde_json::json!({ "plugins": plugins }))
            }
            dialogs::Kind::Branch => {
                // A19: the branch list is the live `branch.list`, not a stub.
                let branches = self
                    .app
                    .session
                    .execute("branch.list", &serde_json::json!({}))
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                std::borrow::Cow::Owned(serde_json::json!({ "branches": branches }))
            }
            _ => std::borrow::Cow::Borrowed(data),
        };

        let root = cx.entity();

        self.dialog = Some(dialogs::DialogState::new(kind, data.as_ref(), &cmds, cx, preview, root));

        // 从主菜单点开的对话框要把菜单本身收掉，否则菜单会盖在对话框上。

        if matches!(self.popup, Some(Popup::MainMenu(_))) {

            self.close_popup(cx);

        }

        cx.notify();

    }

    /// Show the Home screen (the tab strip's Home button): the Home panel plus the tab state.
    pub(crate) fn show_home(&mut self) {

        self.tabs.home = true;

        // A leftover `dismiss` (set when a previous extra panel was closed) would make
        // `sync_ui_requests` clear `extra` on the very next frame — reset it so Home stays.
        *self.extra_state.dismiss.borrow_mut() = false;

        self.extra = Some(panels::extra::ExtraKind::Home);

    }

    /// Leave the Home screen when a file tab is picked (the Home panel is the `extra` one).
    pub(crate) fn close_home(&mut self) {

        self.tabs.home = false;

        if matches!(self.extra, Some(panels::extra::ExtraKind::Home)) {

            self.extra = None;

        }

    }

    fn toggle_extra(&mut self, kind: panels::extra::ExtraKind) {

        if self.extra.as_ref() == Some(&kind) {

            self.extra = None;

        } else {

            self.extra = Some(kind);

        }

    }



    /// 演示层：消费顶栏按钮 / 画面点击，推进播放头与过渡时钟。

    fn sync_present(&mut self) {

        let action = present::ACTION.with(|a| a.replace(0));

        if action == 3 {

            self.present = None;

            return;

        }

        let doc = self.app.session.doc().ok().cloned();

        if let (Some(st), Some(doc)) = (self.present.as_mut(), doc.as_ref()) {

            match action {

                1 => st.playing = !st.playing,

                2 => st.back(),

                _ => {}

            }

            // 画面点击：命中子图层 hotspot 就执行它自己的交互（含 Action::Animation），
            // 没命中回退到画框自身第一条交互。坐标换算与命中都在 present 侧完成。
            present::consume_click(doc, st);

            st.advance(doc);

        }

    }



    /// Dev Mode（右栏 Inspect 标签）→ 画布 / 引擎的每帧同步，照 egui `sync_dev_mode`。

    ///

    /// 进入 Inspect 时结束正在编辑的文字 / 矢量、丢掉手势（Dev Mode 什么都不许改），

    /// 并把「画布显示哪些注解」的过滤器从右栏下标翻成 category id（分类没了就退回全部）。

    fn sync_dev_mode(&mut self) {

        let dev = self.fields.right_tab() == 2;

        if dev && !self.app.session.tool.dev_mode {

            let _ = self.app.session.drop_gesture();

            let editing = self.app.session.ds().map(|d| (d.text_edit.is_some(), d.vector_edit.is_some())).unwrap_or_default();

            for (on, cmd) in [(editing.0, "text.edit.end"), (editing.1, "vector.edit.end")] {

                if on {

                    let _ = self.app.session.execute(cmd, &serde_json::json!({}));

                }

            }

        }

        self.app.session.tool.dev_mode = dev;

        self.app.dev_inspect = dev;

        self.app.dev_cat_filter = match self.fields.dev_cat_filter_index() {

            0 => None,

            1 => Some(String::new()),

            i => self.app.session.ds().ok().and_then(|ds| {

                figcraft_engine::annotations::categories(&ds.doc).get(i.saturating_sub(2)).map(|c| c.id.clone())

            }),

        };

    }



    /// 聚焦视图顶栏（egui `dev_mode::focus_bar`）：名字 / 状态 / Compare changes /

    /// 上一个·下一个 ready-for-dev / 退出。只在 `dev.focus` 选中了一个 frame 且不在版本预览时画。

    fn dev_focus_bar(&self) -> Option<gpui::Stateful<gpui::Div>> {

        let ds = self.app.session.ds().ok()?;

        let id = ds.focused()?;

        if ds.preview.is_some() {

            return None;

        }

        let n = ds.doc.node(id)?;

        let t = &self.app.theme;

        let cmds = self.fields.queue();

        let list: Vec<NodeId> = figcraft_engine::devinspect::dev_items(&ds.doc, Some(figcraft_doc::DevStatus::ReadyForDev), None)

            .into_iter()

            .map(|i| i.id)

            .collect();

        let at = list.iter().position(|i| *i == id);

        let can_step = list.len() > usize::from(at.is_some());

        let pos = at.map_or_else(|| format!("{} ready", list.len()), |k| format!("{} of {}", k + 1, list.len()));



        // 一行里：图标 / 名字 / 状态徽标 / 按钮。

        let mut row = div()

            .flex()

            .flex_row()

            .items_center()

            .gap(widgets::pt(6.0))

            .px(widgets::pt(10.0))

            .py(widgets::pt(5.0))

            .rounded(widgets::pt(8.0))

            .bg(widgets::rgba(t.panel))

            .border_1()

            .border_color(widgets::rgba(t.border))

            .child(widgets::icon::paint(widgets::Icon::Frame, widgets::rgba(t.icon), 14.0))

            .child(

                div()

                    .text_size(widgets::pt(11.0))

                    .font_weight(gpui::FontWeight::SEMIBOLD)

                    .text_color(widgets::rgba(t.text))

                    .child(gpui::SharedString::from(n.name.clone())),

            );

        if let Some((label, col)) = canvas::rulers::status_label(n) {

            row = row.child(

                div()

                    .px(widgets::pt(5.0))

                    .py(widgets::pt(2.0))

                    .rounded(widgets::pt(4.0))

                    .bg(widgets::rgba(col))

                    .text_size(widgets::pt(10.0))

                    .font_weight(gpui::FontWeight::SEMIBOLD)

                    .text_color(gpui::white())

                    .child(gpui::SharedString::from(label)),

            );

        }

        if n.dev_status == figcraft_doc::DevStatus::ReadyForDev && n.dev_changed {

            let c = cmds.clone();

            row = row.child(

                widgets::Dropdown::new(gpui::SharedString::from(tr("Compare changes")), 0.0, None).fill().on_click(move |_a, _w, _cx| {

                    c.push(panels::Cmd { id: "dev.compare", params: Some(serde_json::json!({ "id": id, "show": true })) });

                }),

            );

        }

        if can_step {

            for (icon, dir, tip) in [

                (widgets::Icon::ChevronLeft, "previous", tr("Previous ready for dev")),

                (widgets::Icon::ChevronRight, "next", tr("Next ready for dev")),

            ] {

                let c = cmds.clone();

                let _ = tip;

                row = row.child(widgets::IconButton::new(icon).size(24.0).on_click(move |_e, _w, _cx| {

                    c.push(panels::Cmd { id: "dev.focus", params: Some(serde_json::json!({ "direction": dir })) });

                }));

            }

            row = row.child(

                div().text_size(widgets::pt(11.0)).text_color(widgets::rgba(t.text_secondary)).child(gpui::SharedString::from(pos)),

            );

        }

        let c = cmds.clone();

        row = row.child(widgets::IconButton::new(widgets::Icon::Close).size(24.0).on_click(move |_e, _w, _cx| {

            c.push(panels::Cmd { id: "dev.unfocus", params: Some(serde_json::json!({})) });

        }));

        Some(

            div()

                .id("dev-focus-bar")

                .absolute()

                .top(px(12.0))

                .left(px(0.0))

                .right(px(0.0))

                .flex()

                .justify_center()

                .on_mouse_down(MouseButton::Left, |_e, _w, cx: &mut App| cx.stop_propagation())

                .child(row),

        )

    }



    fn sync_panel(&mut self, cx: &mut Context<Self>) -> panels::right::data::PanelData {

        // Commands the panel raised (align, distribute): `ui-egui` runs them against the selection

        // the moment a button is clicked, which in a retained tree is the frame after.

        let cmds = self.fields.take_cmds();

        if !cmds.is_empty() {

            let ids = self.app.session.ds().ok().map(|d| d.selection.clone()).unwrap_or_default();

            for c in cmds {

                // egui ran `app.run(cmd, {"ids": ids})`; extra command params (e.g.

                // `effect.add {type:"dropShadow"}`) are merged into that object here.

                // No `if ids.is_empty() { skip }`: that guard used to *drain and discard* every

                // command raised with nothing selected, and it also blocked commands that do not

                // need a selection at all (`page.setBackground`, `file.exportLayers`, …).

                // egui hands the command to the engine unconditionally; the engine decides.

                let params = panels::cmd_params(&ids, c.params);

                if let Err(e) = self.app.session.execute(c.id, &params) {

                    // Do not swallow it silently: a button that looks dead may actually be a

                    // rejected command. `FC_DEBUG_CMDS=1` shows the exact id/params/error.

                    if std::env::var_os("FC_DEBUG_CMDS").is_some() {

                        eprintln!("[panel cmd] {} {} -> {}", c.id, params, e);

                    }

                }

            }

            // Command ran: the panel's snapshot, the canvas and the header all depend on the

            // document, so ask for the frame that redraws them.

            cx.notify();

        }



        let items: Vec<(panels::Prop, f64)> = self.pending.borrow_mut().drain(..).collect();

        if !items.is_empty() {

            let ids = self.app.session.ds().ok().map(|d| d.selection.clone()).unwrap_or_default();

            if !ids.is_empty() {

                for (prop, v) in items {

                    let mut props = serde_json::Map::new();

                    props.insert(prop.key().to_string(), serde_json::json!(prop.map(v)));

                    let _ = self.app.session.execute("node.set", &serde_json::json!({"ids": ids, "props": props}));

                }

                cx.notify();

            }

        }



        // 一帧只读一次整份文档：`panel_data()` 会 `data::read` 整棵文档 + 变量库，

        // 以前这里连调 3 次、render 再调 1 次 = 每帧 4 次全量读，拖动时直接拖垮帧率。

        let data = self.panel_data();



        let values = self.selected_values();

        if values != self.last_geo {

            self.last_geo = values.clone();

            if let Some((_, v)) = &values {

                self.fields.set_values(cx, v);

            }

        }



        // Auto layout（gap / cross-gap / padding / 主轴+交叉轴对齐）→ 右栏缓存字段。

        let layout = self.selected_layout();

        if layout != self.last_layout {

            self.last_layout = layout.clone();

            if let Some((_, v)) = &layout {

                self.fields.set_layout_values(cx, v.gap, v.cross_gap, v.padding, v.main_align, v.cross_align);

            }

        }

        // Dimensions 的 min/max（`limit_row_v`）。

        if let Some(dims) = &data.dimensions {

            self.fields.set_dim_limits(cx, dims.min_w, dims.max_w, dims.min_h, dims.max_h);

        }



        // 排版数值字段（字号 / 行高 / 字距 / 段落间距）：仅全选文本时回填。

        if let Some(t) = &data.text {

            self.fields.set_text_values(cx, t);

        }

        // 矢量点面板的数值字段（X / Y / W / H / 圆角）：仅在矢量编辑模式回填。

        if let Some(v) = &data.vector {

            self.fields.set_vector_values(cx, v);

        }



        // 字段宽度 = `col_width(right_w)`；面板拖拽后推一次。

        let col = ((self.right_w - crate::widgets::PAD - crate::widgets::PAD_RIGHT - crate::widgets::GRID_GAP * 2.0 - crate::widgets::SLOT) / 2.0)

            .max(40.0);

        if (col - self.last_col).abs() > 0.5 {

            self.last_col = col;

            self.fields.set_col(col, cx);

        }



        // 填充色行（egui `color_field`）：读选中节点第一个可见 fill —— 纯色显示 14pt dot，

        // 渐变 / 图片 / 图案显示颜色条（swatch）。

        let paint = self.app.session.ds().ok().and_then(|ds| {

            let id = *ds.selection.first()?;

            let n = ds.doc.node(id)?;

            n.fills.iter().find(|p| p.visible).cloned()

        });

        if let Some(p) = paint {

            self.fields.set_fill_paint(&p, cx);

        }



        // 描边色行（egui `paints.rs` `color_field`）：读选中节点第一个可见 stroke —— 纯色显示颜色点，

        // 渐变 / 图片 / 图案显示颜色条（swatch）。

        let stroke = self.app.session.ds().ok().and_then(|ds| {

            let id = *ds.selection.first()?;

            let n = ds.doc.node(id)?;

            n.strokes.iter().find(|p| p.visible).cloned()

        });

        if let Some(p) = stroke {

            self.fields.set_stroke_paint(&p, cx);

        }

        data

    }



    /// 一条分隔条的 **4px 命中覆盖条**：以可见 1px 线为中心，左右各 `HANDLE_OVERHANG`。
    /// 必须作为 `top_row` 的**最后**一个 child 绘制：GPUI「后绘制的 hitbox 先命中」
    /// （`window.rs::hit_test` 逆序遍历 + `dispatch_mouse_event` 的 bubble 逆序），
    /// 这样溢出到画布那 1.5px 才不会被紧随其后的画布抢走。它 `absolute` 定位、不占
    /// flex 宽度；`stop_propagation` 防止同一次按下再触发画布的框选 / 面板点击。
    /// hover 经 `on_hover` 写进 `resize_drag::hovered`，点亮所在容器自带的那条 1px 描边。
    fn divider_hotspot(&self, side: ResizeSide, viewport_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let centre = panels::divider_center_x(side, viewport_w, match side {
            ResizeSide::Left => self.left_w,
            ResizeSide::Right => self.right_w,
        });
        let hover_edge = match side {
            ResizeSide::Left => crate::resize_drag::Edge::Left,
            ResizeSide::Right => crate::resize_drag::Edge::Right,
        };
        // `.id()` 把裸 Div 升成 Stateful<Div>，才能挂 `on_hover`（只在 StatefulInteractiveElement
        // 上）。左右各用不同 id，避免 element id 冲突；id 内容不参与命中几何。
        let hit_id: gpui::SharedString = match side {
            ResizeSide::Left => "divider-hit-left".into(),
            ResizeSide::Right => "divider-hit-right".into(),
        };
        let hit = div()
            .id(hit_id)
            .absolute()
            .left(px(panels::divider_hit_left(centre)))
            .top(px(0.0))
            .w(px(panels::HANDLE_HIT_W))
            .h_full()
            // hover 光标用 **hitbox 级**，绝不用整窗级 `set_window_cursor_style`：后者会压掉
            // 右栏输入框前导格的 `ResizeLeftRight`（见 lib.rs:4139 / 4341 的回归记录）。
            // 拖动中的 `ClosedHand` 整窗锁由 root 的 paint-only canvas 在 `resize_drag::any_active()` 时处理。
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            // hover 状态写进 thread_local，让对应容器的 1px 描边变强调色。gpui 的 group_hover
            // 依赖 group hitbox 先 push，而覆盖条后绘制 → 跨兄弟查不到，故走状态 + notify。
            .on_hover(cx.listener(move |_this: &mut CanvasView, hovering: &bool, _window, cx| {
                crate::resize_drag::set_hover(hover_edge, *hovering);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut CanvasView, e: &MouseDownEvent, window, cx| {
                    // 热区压在画布 / 面板之上，这一下必须只归分隔条：否则溢出到画布的
                    // 1.5px 会同时启动框选。
                    cx.stop_propagation();
                    match side {
                        ResizeSide::Left => {
                            // 左栏：与底栏顶边共用 `resize_drag` 状态机（thread_local + 窗口 x
                            // 相对位移）。move/up 由根视图 + 窗口级接管，双击复位走同一处判定。
                            let x = f32::from(e.position.x);
                            let double = crate::resize_drag::begin(
                                crate::resize_drag::Handle::LeftPanel,
                                1.0, // 右移变宽
                                this.left_w,
                                x,
                                panels::MIN_PANEL,
                                panels::MAX_PANEL,
                                x,
                                f32::from(e.position.y),
                            );
                            if double {
                                this.left_w = panels::DEFAULT_PANEL;
                                crate::resize_drag::end_drag(crate::resize_drag::Handle::LeftPanel);
                                cx.notify();
                                return;
                            }
                        }
                        ResizeSide::Right => {
                            // 右栏维持现状：沿用 `resizing` 状态机 + 根 div 内联路由。
                            if e.click_count >= 2 {
                                this.right_w = panels::DEFAULT_PANEL;
                                this.resizing = None;
                                cx.notify();
                                return;
                            }
                            this.resizing = Some(side);
                            this.drag_start_x = f32::from(e.position.x);
                            this.drag_start_w = this.right_w;
                        }
                    }
                    this.commit_edits(window, cx);
                    this.focus.focus(window);
                    cx.notify();
                }),
            );
        hit
    }



    /// 让控制通道的请求驱动的这一帧（对应原文的 `ctx.request_repaint()`）。

    ///

    /// GPUI 的 `AsyncApp`/`WeakEntity` 都不是 `Send`，所以不能把唤醒闭包直接交给别的线程去

    /// `notify`。这里走一条 `smol::channel`：服务线程 `try_send`（任意线程、非阻塞），

    /// 前台任务 `recv().await` 拿到后再 `cx.notify()`——和 egui 的 `request_repaint` 同语义。

    fn control_wake(cx: &mut Context<Self>) -> control::Wake {

        let (tx, rx) = smol::channel::unbounded::<()>();

        cx.spawn(async move |this, cx| {

            while rx.recv().await.is_ok() {

                if this.update(cx, |_, cx| cx.notify()).is_err() {

                    break;

                }

            }

        })

        .detach();

        Arc::new(move || {

            let _ = tx.try_send(());

        })

    }



    /// 窗口坐标 → 画布局部坐标。

    ///

    /// `app.canvas.rect` 的原点恒为 0（画布内部一律用局部坐标），窗口里的偏移来自

    /// 左侧的导轨 + 左面板 + 手柄；这里把它减掉。

    /// B9：`view.toggleUi` 写 `ui_hidden`，`view.minimize` 写 `ui_minimized`；两条命令和
    /// 快捷键早就绑好，界面层只差把这两个标志读出来。
    fn ui_flags(&self) -> (bool, bool) {

        self.app.session.ds().map(|d| (d.view.ui_hidden, d.view.ui_minimized)).unwrap_or((false, false))

    }

    /// 本帧的 Dock 值：隐藏 / 收起侧栏时把对应宽度置 0，**不改 `self.left_w` / `right_w`**，
    /// 于是恢复时宽度原样回来。画布几何、命中换算与弹层锚点全取这一份值，不会互相漂移。
    fn dock(&self) -> dock::DockLayout {

        let (hidden, minimized) = self.ui_flags();

        let left = if hidden || minimized { 0.0 } else { self.left_w };

        let no_selection = self.app.session.ds().map(|d| d.selection.is_empty()).unwrap_or(true);

        // 画板：左栏 / 竖条 / 动画栏全收起，右栏也只在 `board_panel` 打开时出现



        // （用户 2026-11-10：「画板模式下把左右栏都关掉，动画也关掉，包括侧面的那个竖条」）。



        let board = self.is_board_doc();



        let board_hidden = board && !self.board_panel;



        let left = if board || self.left_collapsed { 0.0 } else { left };



        let right = if hidden || board_hidden || (minimized && no_selection) { 0.0 } else { self.right_w };

        let bottom = if hidden { 0.0 } else { self.motion_bar_h() };

        dock::DockLayout::new(left, right, bottom)

    }

    fn local(&self, p: gpui::Point<Pixels>) -> Pos2 {

        let dock = self.dock();

        pos2(f32::from(p.x) - dock.center_x(), f32::from(p.y) - dock.titlebar_h)

    }



    /// 本帧指针该显示什么（照搬 `canvas.rs` 末尾那段：平移 > 缩放工具 > 工具光标）。

    fn cursor_now(&self) -> canvas::cursors::Shown {

        // 标尺/参考线自己的光标优先（原来由 guides::input 直接设）。

        if let Some(c) = self.app.guide_cursor {

            return c;

        }

        // 自动布局手柄的悬停光标（原实现 `ctx.set_cursor_icon`）。

        if let Some(c) = self.app.spacing_cursor {

            return c;

        }

        let panning = self.space || self.pan.is_some();

        let down = self.primary_down || self.pan.is_some();

        canvas::cursors::on_canvas(panning, down, self.zoom_key.then_some(self.mods.alt), self.app.session.tool.overlay.cursor)

    }



    /// 标尺/参考线优先接管指针（原 `input.rs` 里 guides::input 在工具之前）。

    fn guide_input(&mut self, pos: Pos2, pressed: bool, released: bool, alt: bool) -> bool {

        self.app.guide_cursor = None;

        canvas::guides::input(&mut self.app, pos, pressed, released, alt)

    }



    /// 把一次指针事件交给引擎（对应原文 `pointer_event` + `session.pointer`）。

    fn send(&mut self, kind: PointerKind, pos: Pos2, mods: &Modifiers) {

        let p = self.app.to_page(pos);

        let zoom = self.app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);

        let ev = PointerEvent {

            kind,

            x: p.x,

            y: p.y,

            sx: f64::from(pos.x),

            sy: f64::from(pos.y),

            shift: mods.shift,

            alt: mods.alt,

            // `cmd` 与 egui 的 `modifiers.command` 同义：macOS 是 Cmd、Windows/Linux 是 **Ctrl**。
            // 只传 `platform` 的话，Windows 上按 Ctrl 不算 command，引擎的 Bend 工具
            // （`vectoredit/pointer.rs:567` 判 `ev.cmd`）永远不会触发 —— 用户按住 Ctrl 拉不出曲线。
            cmd: mods.platform || mods.control,

            ctrl: mods.control,

            zoom,

            pressure: None,

        };

        self.pointer = Some(pos);

        let _ = self.app.session.pointer(&ev);

    }



    /// 滚轮：Ctrl/Cmd（或捏合）以指针为锚点缩放；否则平移（照搬 `input.rs` 的分支）。

    fn on_wheel(&mut self, e: &ScrollWheelEvent) {

        let pos = self.local(e.position);

        self.pointer = Some(pos);

        let (dx, dy) = match e.delta {

            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),

            ScrollDelta::Lines(p) => (p.x * 30.0, p.y * 30.0),

        };

        let zooming = e.modifiers.control || e.modifiers.platform;

        let anchor = self.app.to_page(pos);

        let Ok(ds) = self.app.session.ds_mut() else { return };

        if zooming {

            // 与 egui 的 `zoom_delta()` 同向同量级：滚轮向上（dy>0）放大。

            let factor = (f64::from(dy) * 0.0025).exp();

            let z = ds.view.zoom * factor;

            ds.view.set_zoom_at(z, (anchor.x, anchor.y));

        } else {

            ds.view.scroll(f64::from(dx), f64::from(dy), e.modifiers.shift);

        }

        ds.revision += 1;

    }



    /// 缩放工具（照搬 `cmd/view.rs::zoom_at` 与 `input.rs::zoom_tool`）：

    /// 点一下缩放一档，拖出区域则把那块填满视图。

    fn finish_zoom(&mut self, start: Pos2, end: Pos2, out: bool) {

        let area = Rect::from_two_pos(start, end);

        let a = self.app.to_page(area.min);

        let b = self.app.to_page(area.max);

        let Ok(ds) = self.app.session.ds_mut() else { return };

        if area.width() > 4.0 && area.height() > 4.0 {

            let (x, y, w, h) = (a.x.min(b.x), a.y.min(b.y), (b.x - a.x).abs(), (b.y - a.y).abs());

            ds.view.fit(figcraft_geom::Rect::new(x, y, x + w, y + h), 256.0);

        } else {

            let z = zoom_step(ds.view.zoom, !out);

            ds.view.set_zoom_at(z, (b.x, b.y));

        }

        ds.revision += 1;

    }



    // ── 弹出层（下拉菜单 / 取色器）：与 egui 的 `menu_popup` / `colorpicker` 一致 ──────────────

    // 打开后画在最上层（root 视图在 `render` 末尾把 backdrop + 弹层作为最后的孩子渲染），

    // 透明 backdrop 拦截外部点击来关闭。



    /// **所有下拉弹窗的唯一入口**（用户 2026-11-08：跟随按钮 / 宽度取按钮真实宽度 /

    /// 下方 4pt 收口成通用能力）。

    ///

    /// `anchor` 是 `Dropdown::menu(key)` 登记后拿到的 `MenuAnchor`：它跨帧保留按钮矩形，

    /// 所以右栏滚动、面板变宽、弹窗拖动时菜单都跟着按钮走。`rect` 是点击那一帧的矩形

    ///（`Dropdown::on_click` 给的），用来给探针播种，之后由 prepaint 每帧刷新。

    /// `min_width = None` 表示「宽度就取按钮真实宽度」；`Some(w)` 只作为不得更窄的下限。

    fn open_dropdown_menu(

        &mut self,

        anchor: Option<crate::widgets::dropdown::MenuAnchor>,

        rect: Bounds<Pixels>,

        min_width: Option<f32>,

        rows: Vec<MenuRow>,

        on_pick: std::rc::Rc<dyn Fn(&str, &mut Window, &mut App)>,

        cx: &mut Context<Self>,

    ) {

        // ① 零尺寸锚点补成 1×0。`open_menu(pos, …)` 的调用点（W/H 字段尾插槽的 sizing 菜单、
        //    图标按钮菜单、标题行的 More 菜单）给的是一个「点」，而通用 `Menu::show` 把
        //    「宽高都为 0」当作没有锚点、直接返回 None —— 菜单于是**点了什么都不弹**
        //    （用户 2026-11-11 指出「尺寸的下拉菜单点不开」）。补 1 pt 宽后落位规则不变：
        //    仍然从点击点起、按「锚点底 + 4」向下展开，水平贴边由 `Menu` 自己夹紧。

        let rect = if rect.size.width <= px(0.0) && rect.size.height <= px(0.0) {

            Bounds { origin: rect.origin, size: size(px(1.0), px(0.0)) }

        } else {

            rect

        };

        if let Some(a) = &anchor {

            a.seed(rect);

            crate::widgets::dropdown::set_active_menu(Some((a.key().to_string(), f32::from(rect.origin.x))));

        } else {

            crate::widgets::dropdown::set_active_menu(None);

        }

        self.popup = Some(Popup::Menu {

            anchor: rect,

            width: min_width.or_else(|| Some(f32::from(rect.size.width))),

            rows,

            on_pick,

            probe: anchor.map(|a| a.cell()),

        });

        cx.notify();

    }



    /// 静态锚点的旧调用点：统一走同一个入口，只是没有跟随探针。

    fn open_menu(

        &mut self,

        anchor: Bounds<Pixels>,

        width: Option<f32>,

        rows: Vec<MenuRow>,

        on_pick: std::rc::Rc<dyn Fn(&str, &mut Window, &mut App)>,

        cx: &mut Context<Self>,

    ) {

        self.open_dropdown_menu(None, anchor, width, rows, on_pick, cx);

    }



    /// 打开 Share 弹层（右栏顶栏的 Share 按钮；内容在 render 里按 live 状态重建）。

    fn open_share(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {

        self.popup = Some(Popup::Share { anchor });

        cx.notify();

    }



    // ── 底部工具栏 · 画布右键菜单 ────────────────────────────────────────────────────────────



    /// 跑一条命令（工具栏与右键菜单共用）：`ui-egui` 里是 `app.run(id, params)`。

    fn run_command(&mut self, id: &str, params: serde_json::Value, cx: &mut Context<Self>) {

        // 打开 / 新建 / 关闭文件后，动画面板默认收起：Motion 是设计文件的编辑模式，不是常驻栏。



        // （egui `FigApp::new` 的 `motion_open: false`；用户 2026-11-10：「打开任何东西的时候



        // 默认应该是要把这个动画关掉的」。）



        if id.starts_with("file.open") || id.starts_with("file.new") || id == "file.close" || id == "file.saveAs" {



            self.set_motion_open(false, cx);



        }



        // 需要**系统文件对话框**的命令（egui `FigApp::open_dialog` / `import_dialog`）：

        // 引擎只认 `path`（或 `data` + `name`），路径得由原生选择器给。

        // 扩展名表来自 `crate::open`，与 `ui-egui/src/open.rs` / `import_dialog` 同源。

        let picker: Option<(&str, &[&str], &str)> = match id {

            "file.open" => Some((tr("Open"), crate::open::OPEN_EXTENSIONS, "file.open")),

            "file.import" => Some((tr("Import"), crate::open::IMPORT_EXTENSIONS, "file.import")),

            "file.placeImage" => Some((tr("Place image"), crate::open::IMAGE_EXTENSIONS, "file.placeImage")),

            _ => None,

        };

        if let Some((title, exts, cmd)) = picker {

            // 一个筛选器列出全部扩展名（每个扩展名一个筛选器会变成一长串重复项）。

            let mut dialog = rfd::FileDialog::new().set_title(title).add_filter(title, exts);

            if let Some(path) = dialog.pick_file() {

                let params = serde_json::json!({ "path": path.to_string_lossy() });

                if self.execute_reporting(cmd, params, cx) {

                    cx.notify();

                }

            }

            return;

        }

        if let Err(e) = self.app.session.execute(id, &params) {

            // 不做无声吞掉：`FC_DEBUG_CMDS=1` 时把 id / 参数 / 错误打出来（同右栏那条通道）。

            if std::env::var_os("FC_DEBUG_CMDS").is_some() {

                eprintln!("[ui cmd] {id} {params} -> {e}");

            }

        }

        cx.notify();

    }



    /// 工具栏的一次点击（`toolbar::Intent`）。`window` 用来取指针位置（飞出菜单挂在 caret 上方）

    /// 以及把焦点交给 Actions 的搜索框。

    /// 当前文档是不是画板（`figcraft_engine::board::is_board`）。工具栏按它换成

    /// `toolbar::BOARD_GROUPS`（`ui-egui/src/toolbar.rs` 的同一套九组），画板选择条也按它决定

    /// 显不显示。

    fn is_board_doc(&self) -> bool {

        self.app.session.doc().is_ok_and(figcraft_engine::board::is_board)

    }



    /// 底部中间弹一条提示，4 秒后自己消失（egui `app.toast`）。

    /// Open the inline rename field over a canvas label or a layers row.

    /// egui `frame_labels::rename_field` / `layers::start_rename`.

    fn begin_inline_rename(&mut self, ids: Vec<figcraft_doc::NodeId>, target: InlineRenameTarget, cx: &mut Context<Self>) {

        let name = self.app.session.ds().ok()

            .and_then(|ds| ids.first().and_then(|i| ds.doc.node(*i)).map(|n| n.name.clone()))

            .unwrap_or_default();

        // A NEW primitive editor for every rename. Reusing one TextEdit keeps the previous

        // draft in its buffer (`set_value` does not touch the buffer while editing), which made

        // every double click rename the row to the first row's name.

        let pending = self.inline_rename_pending.clone();

        let ids_for_cb = ids.clone();

        let editor = widgets::TextEdit::build(cx, |te, _| {

            te.set_inset_left(0.0);

            te.set_right_pad(0.0);

            te.set_value(name);

            te.set_on_commit(move |text, _w, _cx| {

                *pending.borrow_mut() = Some((target, ids_for_cb.clone(), text.trim().to_string()));

            });

        });

        self.inline_rename_field = editor;

        self.inline_rename = Some(InlineRename { ids, target, start: true, frames: 0, was_active: false });

        self.rename_cancel = false;

        cx.notify();

    }



    /// R4: where the field sits for a canvas label. `tool.labels` holds the label quads in *page*

    /// coordinates; `to_screen` turns them into window coordinates for an absolutely positioned

    /// element. Recomputed every frame so panning / zooming keeps the field on its label.

    fn label_rename_rect(&self, id: Option<figcraft_doc::NodeId>) -> Option<(f32, f32, f32)> {

        let id = id?;

        let name = self.app.session.ds().ok()

            .and_then(|ds| ds.doc.node(id).map(|n| n.name.clone())).unwrap_or_default();

        let Some(quad) = self.app.session.tool.labels.iter()

            .find(|(i, _)| *i == id).map(|(_, q)| *q)

        else {

            // `tool.labels` is only filled while the label is actually drawn (egui has the same

            // dependency). If it is missing, fall back to the frame's own top-left corner so

            // an inline rename is never an invisible field that has grabbed the keyboard.

            let ds = self.app.session.ds().ok()?;

            let n = ds.doc.node(id)?;

            let w = ds.doc.world_transform(id);

            let p = w * figcraft_geom::Point::new(0.0, 0.0);

            let s = self.app.to_screen(p.x, p.y);

            let w = (name.chars().count() as f32 * 6.5 + 24.0).clamp(80.0, 480.0);

            return Some((s.x, s.y - 20.0, w));

        };

        let mut min_x = f32::MAX;

        let mut max_x = f32::MIN;

        let mut max_y = f32::MIN;

        for p in quad {

            let s = self.app.to_screen(p.x, p.y);

            min_x = min_x.min(s.x);

            max_x = max_x.max(s.x);

            max_y = max_y.max(s.y);

        }

        if min_x == f32::MAX { return None; }

        let w = (name.chars().count() as f32 * 6.5 + 24.0).max(max_x - min_x).clamp(80.0, 480.0);

        Some((min_x, max_y - 16.0, w))

    }



    pub(crate) fn toast(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {

        let text = text.into();

        if text.is_empty() {

            return;

        }

        self.toast_msg = Some((text, std::time::Instant::now()));

        // 只在过期那一刻唤醒一次：不为一条提示持续重绘。

        let me = cx.entity();

        cx.spawn(async move |_, cx| {

            gpui::Timer::after(std::time::Duration::from_secs(4)).await;

            let _ = me.update(cx, |cv, cx| {

                cv.toast_msg = None;

                cx.notify();

            });

        })

        .detach();

        cx.notify();

    }



    /// 跑一条命令；失败时把引擎的原话弹出来（egui 的 `app.run` + `show_error`）。

    fn execute_reporting(&mut self, id: &str, params: serde_json::Value, cx: &mut Context<Self>) -> bool {

        match self.app.session.execute(id, &params) {

            Ok(_) => true,

            Err(e) => {

                if std::env::var_os("FC_DEBUG_CMDS").is_some() {

                    eprintln!("[ui cmd] {id} {params} -> {e}");

                }

                self.toast(e.to_string(), cx);

                false

            }

        }

    }



    /// 一次拖入（`ui-egui/src/canvas/input.rs::import_dropped` 与 `home.rs::dropped` 的合并）：

    ///

    /// * **文档**（`.figcraft` / `.fig` / `.jam` / `.penpot` / `.sketch` / JSON）—— 每个都

    ///   `file.open` 成一个**新文件**，不管落在首页还是画布上（`is_document`）。

    /// * **图片与 SVG** —— 编辑器里 `file.import` 落在**放下那一点**（页面坐标，引擎自己分流：

    ///   SVG 变框架、图片放图片；`.figcraft` 之类才会追加页面）。

    /// * 首页只接受能打开的类型，其余按原文提示 "Drop a .figcraft, …"。

    pub(crate) fn drop_files(&mut self, paths: &[std::path::PathBuf], at: Point<Pixels>, cx: &mut Context<Self>) {

        if paths.is_empty() {

            return;

        }

        let on_home = self.tabs.home;

        let pt = self.app.to_page(pos2(f32::from(at.x), f32::from(at.y)));

        let (mut opened, mut imported, mut refused) = (0usize, 0usize, 0usize);

        for path in paths {

            if crate::open::is_document(path) {

                if self.execute_reporting("file.open", serde_json::json!({ "path": path.to_string_lossy() }), cx) {

                    opened += 1;

                }

                continue;

            }

            if on_home {

                if crate::open::opens(path) {

                    if self.execute_reporting("file.open", serde_json::json!({ "path": path.to_string_lossy() }), cx) {

                        opened += 1;

                    }

                } else {

                    refused += 1;

                }

                continue;

            }

            // 编辑器里：落到指针那一点。引擎按扩展名自己选「SVG → 框架 / 图片 → 图片 /

            // 文档 → 追加页面」，这里不重复判断。

            let params = serde_json::json!({ "path": path.to_string_lossy(), "x": pt.x, "y": pt.y });

            if self.execute_reporting("file.import", params, cx) {

                imported += 1;

            }

        }

        if opened > 0 {

            // 打开文件后离开首页（egui `app.ui.home = false`）。

            self.close_home();

        }

        if refused > 0 {

            self.toast(figcraft_i18n::tr("Drop a .figcraft, .fig, .jam, .sketch, .json, .svg or image file."), cx);

        }

        if imported > 0 {

            cx.notify();

        }

    }



    /// 画板工具组的一个选项被选中：`tool.{id}` 带上它的参数（颜色 / 形状 / 印章），



    /// 然后关掉菜单（`ui-egui/src/toolbar.rs::options_flyout`）。



    fn pick_board_option(&mut self, id: &str, params: serde_json::Value, cx: &mut Context<Self>) {



        self.tool_flyout = None;



        self.popup = None;



        self.run_command(id, params, cx);



    }



    /// 模板选择器的一张卡被点了：`board.template` 或 `board.table`。



    fn pick_template(&mut self, i: usize, cx: &mut Context<Self>) {



        let (id, params) = crate::board_picker::command_for(i);



        self.tool_flyout = None;



        self.popup = None;



        self.run_command(id, params, cx);



    }



    fn on_intent(&mut self, intent: toolbar::Intent, window: &mut Window, cx: &mut Context<Self>) {

        use figcraft_engine::ToolId;

        let mouse = window.mouse_position();

        match intent {

            toolbar::Intent::Tool(id) => {

                // A tool picked from the row closes the flyout: it could otherwise hang over the

                // cell that just changed.

                self.tool_flyout = None;

                if matches!(self.popup, Some(Popup::Flyout { .. })) {

                    self.popup = None;

                }

                self.run_command("tool.set", serde_json::json!({ "tool": id }), cx);

            }

            toolbar::Intent::Flyout(key) => {

                let open = self.tool_flyout != Some(key);

                self.tool_flyout = open.then_some(key);

                self.popup = if open { self.flyout_at(key) } else { None };

                cx.notify();

            }

            toolbar::Intent::Templates => {



                let open = self.tool_flyout != Some(crate::board_picker::KEY);



                self.tool_flyout = open.then_some(crate::board_picker::KEY);



                self.popup = open.then_some(Popup::Templates);



                cx.notify();



            }



            toolbar::Intent::DesignPanel => {



                // 画板上 `view.design` 是 Design 面板的开关（`ui-egui/src/app/commands.rs`）。



                self.run_command("view.design", serde_json::json!({}), cx);



                cx.notify();



            }



            toolbar::Intent::Actions => {

                let open = !matches!(self.popup, Some(Popup::Palette { .. }));

                self.popup = open.then(|| self.palette_at_pointer(mouse));

                if open {

                    // Put the caret in the search box, so typing filters instead of running

                    // canvas shortcuts.

                    let edit = self.palette_field.read(cx).edit();

                    edit.update(cx, |te, _| te.focus_editor(window));

                }

                cx.notify();

            }

            toolbar::Intent::Mode(i) => {

                let tool = self.app.session.tool.tool;

                match i {

                    // Draw: the pencil, with the timeline put away.

                    0 => {

                        self.set_motion_open(false, cx);

                        self.fields.set_right_tab(0);

                        self.run_command("tool.set", serde_json::json!({ "tool": "pencil" }), cx);

                    }

                    // Design: the move tool (leaving the pencil). Also returns the right panel to
                    // the Design tab so the Dev-Mode inspector (tab 2) does not stay on screen.

                    1 => {

                        self.set_motion_open(false, cx);

                        self.fields.set_right_tab(0);

                        if tool == ToolId::Pencil {

                            self.run_command("tool.set", serde_json::json!({ "tool": "move" }), cx);

                        }

                        cx.notify();

                    }

                    // Animation toggles the Motion timeline; a pen path ends, as in the original.

                    // `set_motion_open` also widens the right panel to the inspector (413 pt)

                    // and restores the previous width when closing.

                    2 => {

                        let open = !self.motion;

                        self.set_motion_open(open, cx);

                        if self.motion && tool == ToolId::Pen {

                            self.run_command("tool.set", serde_json::json!({ "tool": "move" }), cx);

                        }

                        cx.notify();

                    }

                    // Inspect (Dev Mode)：关 Motion，切右栏 Inspect 标签（egui pick_mode(3)）。

                    3 => {

                        self.set_motion_open(false, cx);

                        self.fields.set_right_tab(2);

                        cx.notify();

                    }

                    _ => {}

                }

            }

        }

    }



    /// The canvas's bottom edge in window coordinates minus the pill's own box: the flyout and the

    /// palette both hang off the pill, 8 pt above it.

    fn pill_top(&self) -> f32 {

        // `canvas.rect` is the canvas's own rectangle (origin 0): its height ends at the top row's

        // bottom, and the pill sits 12 pt above that with a 48 pt body.

        self.app.canvas.rect.height() - 12.0 - 48.0

    }



    /// The group's flyout, aligned to **its 16 pt caret button**: the card's left edge is the

    /// caret's left edge, and its bottom edge sits `FLYOUT_GAP` (4 pt) above the button's top edge.

    /// (egui uses the tool cell's left and an 8 pt gap above the pill; the user re-positioned it on

    /// 2026-11-06.)

    fn flyout_at(&self, key: &'static str) -> Option<Popup> {

        // Tick the tool the group *shows* — its current tool when the group holds the current one,

        // else the group's remembered/first tool — rather than the raw current tool. The current

        // tool belongs to exactly one group, so checking it left every other group's flyout with no

        // tick at all (用户：「点击第 2 个连个勾都没有」: the cell shows Frame, the flyout ticked

        // nothing because Move isn't in the frame group).

        let current = self.app.session.tool.tool;

        // The set the pill actually drew: a board has its own nine groups, so a board flyout must

        // look its group up there or it would come back empty.

        let groups = toolbar::groups(self.is_board_doc());

        let ticked = groups

            .iter()

            .find(|g| g.key == key)

            .map(|g| toolbar::shown(g, current, &self.tool_last).0)

            .unwrap_or(current);

        // 便签颜色 / 形状 / 印章：只有一格的组，它的 caret 打开的是「选项」而不是工具行。



        if toolbar::group(key).is_some_and(|g| g.options != toolbar::Options::None) {



            return Some(Popup::Options { key });



        }



        let items = toolbar::flyout_items(groups, key, ticked);

        if items.is_empty() {

            return None;

        }

        // Only the group key is frozen; the card's geometry is recomputed in `render` from the live

        // canvas size, so it tracks the centred pill as the window resizes (`用户 2026-11-06`).

        Some(Popup::Flyout { key, items })

    }



    /// The Actions palette, centred over the pointer and 8 pt above the pill.

    fn palette_at_pointer(&mut self, mouse: Point<Pixels>) -> Popup {

        // Opening afresh lands on All (egui `set_palette`). The query is deliberately kept:

        // GPUI's search field only commits on Enter, so clearing just the model would leave

        // stale text in the field.

        self.palette_tab = palette::Tab::All;

        self.palette_sel = Some(0);

        Popup::Palette { at: mouse }

    }



    /// The canvas right-click menu at `pos` (canvas-local): select what is under it first, then

    /// open the card at the pointer (`ui-egui`'s `open_context_menu`).

    fn open_context_menu(&mut self, pos: Pos2, cx: &mut Context<Self>) {

        let page = self.app.to_page(pos);

        let ids = context_menu::target(&mut self.app, page);

        let doc = self.app.session.doc().ok();

        let items = context_menu::items(doc, &ids);

        self.tool_flyout = None;

        // Store the click point in **canvas-local** coordinates; `render` converts back to window

        // coordinates from the *current* canvas offset, so the menu follows the canvas when the

        // window is resized or the left panel width changes (`用户 2026-11-06`).

        self.popup = Some(Popup::Context {

            pos,

            items,

            open_sub: None,

            aim: crate::widgets::MenuAim::new(),

            pointer: Point { x: px(0.0), y: px(0.0) },

            born: std::time::Instant::now(),

            aim_busy: std::rc::Rc::new(std::cell::Cell::new(false)),

        });

        cx.notify();

    }



    /// A flyout row chosen: `tool.set`, and the tool is remembered for its group so the caret's

    /// button keeps showing it (`ui-egui`'s `tb_last`).

    fn pick_flyout(&mut self, id: &str, cx: &mut Context<Self>) {

        if let Some(tool) = figcraft_engine::ToolId::ALL.iter().copied().find(|t| t.id() == id)

            && let Some(g) = toolbar::groups(self.is_board_doc()).iter().find(|g| g.tools.iter().any(|(t, _)| *t == tool))

        {

            // Remember it for its group (egui's `tb_last`, keyed by group key).

            match self.tool_last.iter_mut().find(|(k, _)| *k == g.key) {

                Some(slot) => slot.1 = tool,

                None => self.tool_last.push((g.key, tool)),

            }

        }

        self.tool_flyout = None;

        self.popup = None;

        self.run_command("tool.set", serde_json::json!({ "tool": id }), cx);

    }



    /// A context-menu row's command: "Paste here" pastes at the click point, everything else runs

    /// as-is (egui's `menus.rs` has the same special case).

    fn pick_context(&mut self, id: &str, cx: &mut Context<Self>) {

        let params = if id == "edit.paste" {

            // The click point in page coordinates: the menu was opened at it, so the pointer still

            // is (a click on a row does not move the pointer out of the menu).

            match self.pointer {

                Some(p) => {

                    let page = self.app.to_page(p);

                    serde_json::json!({ "x": page.x, "y": page.y })

                }

                None => serde_json::json!({}),

            }

        } else {

            serde_json::json!({})

        };

        self.popup = None;

        self.run_command(id, params, cx);

    }



    /// B2: run an Actions palette row by its key and remember it in Recents (newest first).

    fn pick_palette(&mut self, key: &str, cx: &mut Context<Self>) {

        let r = &mut self.app.session.prefs.recent_actions;

        if !r.first().is_some_and(|f| f == key) {

            r.retain(|k| k != key);

            r.insert(0, key.to_string());

            r.truncate(figcraft_engine::MAX_RECENT_ACTIONS);

        }

        self.popup = None;

        if let Some(id) = key.strip_prefix("command:") {

            self.run_command(id, serde_json::json!({}), cx);

        } else if let Some(rest) = key.strip_prefix("plugin:") {

            if let Some((pid, cid)) = rest.split_once('/') {

                self.run_command("plugin.run", serde_json::json!({ "id": pid, "command": cid }), cx);

            }

        } else if let Some(pid) = key.strip_prefix("widget:") {

            self.run_command("widget.insert", serde_json::json!({ "plugin": pid }), cx);

        } else if let Some(node) = key.strip_prefix("component:") {

            let (x, y) = self.app.session.ds().ok().map(|ds| (ds.view.cx, ds.view.cy)).unwrap_or((0.0, 0.0));

            if let Ok(n) = node.parse::<u64>() {

                self.run_command("instance.place", serde_json::json!({ "component": n, "x": x, "y": y }), cx);

            }

        } else if let Some(style) = key.strip_prefix("style:") {

            self.run_command("style.apply", serde_json::json!({ "style": style }), cx);

        }

        cx.notify();

    }

    /// 左侧竖栏第 1 项（`Icon::Menu`）点开的主菜单。

    ///

    /// 目录来自 `figcraft_engine::catalog::MENU`（11 个顶层菜单、226 条命令），几何与交互照

    /// `ui-egui/src/menus/tree.rs`：**悬停**展开子菜单、子卡片贴父卡片右边 4pt 并与那一行齐平。

    /// 具体实现在 `main_menu` 模块。

    fn open_main_menu(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {

        let checked = |id: &str| figcraft_engine::cmd::view::checked(&self.app.session, id);

        let first = main_menu::top_level();

        let _ = checked; // 顶层只有菜单名，不需要勾选态

        self.popup = Some(Popup::MainMenu(main_menu::MainMenu::new(anchor, first)));

        cx.notify();

    }



    /// 主菜单里悬停一行：交给通用控件 `MenuAim` 裁决，展开 / 收起它的子菜单。

    /// 安全三角内的行会被屏蔽；悬停延迟与静止超时靠 `schedule_aim` 安排一次延迟重绘。

    fn hover_menu(&mut self, level: usize, row: usize, pointer: Point<Pixels>, cx: &mut Context<Self>) {

        let checked = |id: &str| figcraft_engine::cmd::view::checked(&self.app.session, id);

        let mut wait = None;

        if let Some(Popup::MainMenu(m)) = &mut self.popup {

            wait = m.hover(level, row, pointer, |prefix| main_menu::build_level(prefix, &checked));

        }

        cx.notify();

        self.schedule_aim(wait, cx);

    }



    /// 延迟唤醒后的结算：没有鼠标事件也要让悬停延迟到点、或让三角内静止超时后交出控制权。

    fn settle_menu(&mut self, cx: &mut Context<Self>) {

        let checked = |id: &str| figcraft_engine::cmd::view::checked(&self.app.session, id);

        let mut wait = None;

        if let Some(Popup::MainMenu(m)) = &mut self.popup {

            wait = m.settle(|prefix| main_menu::build_level(prefix, &checked));

        }

        cx.notify();

        self.schedule_aim(wait, cx);

    }



    /// 只保留一个在飞的延迟唤醒；醒来后再按 `next_wakeup` 续上下一次。

    fn schedule_aim(&mut self, wait: Option<f64>, cx: &mut Context<Self>) {

        let Some(wait) = wait.filter(|w| w.is_finite() && *w > 0.0) else {

            return;

        };

        let Some(Popup::MainMenu(m)) = &self.popup else {

            return;

        };

        if m.aim_busy.get() {

            return;

        }

        m.aim_busy.set(true);

        let flag = m.aim_busy.clone();

        let delay = Duration::from_secs_f64(wait.min(0.5));

        cx.spawn(async move |this, cx| {

            smol::Timer::after(delay).await;

            flag.set(false);

            let _ = this.update(cx, |cv, cx| cv.settle_menu(cx));

        })

        .detach();

    }



    /// 右键菜单的几何（窗口坐标）：整张卡片、展开行的触发行、以及子菜单面板。

    /// 与 `menu_card::rows` 的摆放一致（子卡片在 `left + width + 4`、`row_top - PAD`）。

    fn context_geom(

        pos: Pos2,

        canvas_left: f32,

        vw: f32,

        vh: f32,

        items: &[menu_card::Item],

        open_sub: Option<usize>,

    ) -> (Bounds<Pixels>, Option<Bounds<Pixels>>, Option<Bounds<Pixels>>) {

        let m = 8.0f32;

        let w = context_menu::WIDTH;

        let row_h = context_menu::ROW_H;

        let h = menu_card::height(items, row_h);

        let x = (f32::from(pos.x) + canvas_left).min((vw - w - m).max(m)).max(m);

        let y = f32::from(pos.y).min((vh - h - m).max(m)).max(m);

        let card = Bounds { origin: Point { x: px(x), y: px(y) }, size: size(px(w), px(h)) };

        let mut top = y + 8.0;

        let mut trigger = None;

        let mut panel = None;

        for (i, item) in items.iter().enumerate() {

            if item.separator_before {

                top += 9.0;

            }

            if open_sub == Some(i) && !item.sub.is_empty() {

                trigger = Some(Bounds { origin: Point { x: px(x), y: px(top) }, size: size(px(w), px(row_h)) });

                let ph = menu_card::height(&item.sub, row_h);

                panel = Some(Bounds { origin: Point { x: px(x + w + 4.0), y: px(top - 8.0) }, size: size(px(w), px(ph)) });

            }

            top += row_h;

        }

        (card, trigger, panel)

    }



    /// 右键菜单某行被悬停：把当前几何喂给通用 `MenuAim`，由它裁决子菜单展开 / 收起。

    /// 安全三角内经过的行不会抢走已展开的子菜单；等待时间交给 `schedule_context_aim`。

    fn hover_context(&mut self, row: usize, pointer: Point<Pixels>, vw: f32, vh: f32, cx: &mut Context<Self>) {

        let canvas_left = panels::RAIL_W + self.left_w + panels::HANDLE_W;

        let mut wait = None;

        {

            let Some(Popup::Context { pos, items, open_sub, aim, pointer: p, born, .. }) = &mut self.popup else {

                return;

            };

            *p = pointer;

            let now = born.elapsed().as_secs_f64();

            let (card, trigger, panel) = Self::context_geom(*pos, canvas_left, vw, vh, items, *open_sub);

            aim.set_geometry(0, card, trigger, panel);

            let is_sub = items.get(row).is_some_and(|it| !it.sub.is_empty());

            match aim.hover(0, row, is_sub, pointer, now) {

                crate::widgets::AimAction::Open(_, r) => *open_sub = Some(r),

                crate::widgets::AimAction::Close(_) => *open_sub = None,

                crate::widgets::AimAction::Hold => {}

            }

            // 展开状态可能变了：按新的 open_sub 重新喂几何（子菜单面板的位置）。

            let (card, trigger, panel) = Self::context_geom(*pos, canvas_left, vw, vh, items, *open_sub);

            aim.set_geometry(0, card, trigger, panel);

            wait = aim.next_wakeup(now);

        }

        cx.notify();

        self.schedule_context_aim(wait, cx);

    }



    /// 延迟唤醒：没有新的鼠标事件时也要结算悬停延迟 / 三角内静止超时。

    fn settle_context(&mut self, cx: &mut Context<Self>) {

        let mut wait = None;

        if let Some(Popup::Context { open_sub, aim, pointer, born, .. }) = &mut self.popup {

            let now = born.elapsed().as_secs_f64();

            for action in aim.settle(*pointer, now) {

                match action {

                    crate::widgets::AimAction::Open(_, r) => *open_sub = Some(r),

                    crate::widgets::AimAction::Close(_) => *open_sub = None,

                    crate::widgets::AimAction::Hold => {}

                }

            }

            wait = aim.next_wakeup(now);

        }

        cx.notify();

        self.schedule_context_aim(wait, cx);

    }



    /// 与主菜单 `schedule_aim` 同构：只保留一个在飞的延迟唤醒。

    fn schedule_context_aim(&mut self, wait: Option<f64>, cx: &mut Context<Self>) {

        let Some(wait) = wait.filter(|w| w.is_finite() && *w > 0.0) else {

            return;

        };

        let Some(Popup::Context { aim_busy, .. }) = &self.popup else {

            return;

        };

        if aim_busy.get() {

            return;

        }

        aim_busy.set(true);

        let flag = aim_busy.clone();

        let delay = Duration::from_secs_f64(wait.min(0.5));

        cx.spawn(async move |this, cx| {

            smol::Timer::after(delay).await;

            flag.set(false);

            let _ = this.update(cx, |cv, cx| cv.settle_context(cx));

        })

        .detach();

    }



    /// 主菜单里点一行：命令项执行并关掉菜单；子菜单项只把它的下级展开（通常悬停已经开了）。

    fn click_menu(&mut self, level: usize, row: usize, pointer: Point<Pixels>, cx: &mut Context<Self>) {

        let id = match &mut self.popup {

            Some(Popup::MainMenu(m)) => m.click(level, row),

            _ => None,

        };

        let Some(id) = id else {

            self.hover_menu(level, row, pointer, cx);

            return;

        };

        // ⛔ 四个强调色共用一条命令 `pref.accent`，菜单 id 是 `pref.accent.<颜色>`。

        // 之前把**菜单 id 当命令名**直接 execute（`execute("pref.accent.pink")`）→ 引擎报

        // UnknownCommand，被 `let _ =` 吞掉 → 点颜色毫无反应（用户 2026-11-05 指出「颜色没绑」）。

        // 必须换成真正的命令名 + `{"accent": …}` 参数（同 egui `menus.rs::invoke`）。

        let (cmd, params) = match id.strip_prefix("pref.accent.") {

            Some(accent) => ("pref.accent", serde_json::json!({ "accent": accent })),

            None => (id, serde_json::json!({})),

        };

        // A19：带 "…" 的菜单命令先要输入。直接 execute 会用默认参数静默导出 / 查找，
        // 界面上什么都不会出现；先开通用对话框，由它的按钮带字段值提交同一条命令。

        if let Some(kind) = dialogs::menu_action(cmd) {

            self.open_dialog(kind, &params, cx);

            return;

        }

        if let Err(e) = self.app.session.execute(cmd, &params) {

            eprintln!("[menu] {cmd}: {e}");

        }

        self.close_popup(cx);

    }



    /// 打开取色器弹窗。`current` 是当前色，弹层内嵌一个可编辑十六进制框；`on_pick` 在选色后调用。

    /// 与其余四个旧弹窗一样，外壳是 `widgets::popover` 的通用窗口（标题插槽 = 色点 + "Custom"）。

    fn open_color(

        &mut self,

        origin: Point<Pixels>,

        current: Color32,

        on_pick: std::rc::Rc<dyn Fn(Color32, &mut Window, &mut App)>,

        cx: &mut Context<Self>,

    ) {

        let w = popovers::WindowState::color(cx, origin, current, on_pick, self.fields.queue());

        self.open_window(w, cx);

    }



    /// 打开任意一个通用窗口弹窗（自动布局 / 描边 / 变量 / 文本样式 / 取色器）。

    fn open_window(&mut self, mut w: popovers::WindowState, cx: &mut Context<Self>) {

        // 结构性：变量编辑器**不进 `self.popup` 单槽**。egui 里它是常驻 `app.ui.variables`

        // 的一个编辑器（Full 是页面、Floating 是 Area），与主菜单/dialog 并存；塞进单槽会导致

        // 「点主菜单把它替换掉」「全屏遮罩吞掉 rail 点击」「每次重开都重建、状态全丢」。

        if w.kind() == popovers::WindowKind::VariablesTable {

            let me_full = cx.entity();

            let me_close = cx.entity();

            w.set_variables_callbacks(

                Rc::new(move |full: bool, cx: &mut App| {

                    me_full.update(cx, |cv, cx| cv.set_variables_full(full, cx));

                }),

                Rc::new(move |cx: &mut App| {

                    me_close.update(cx, |cv, cx| cv.close_variables(cx));

                }),

            );

            // 恢复会话记住的 Full/Floating，而不是一律 Floating。

            w.set_variables_full(self.variables_full);

            self.variables_editor = Some(w);

            self.variables_open = true;

            cx.notify();

            return;

        }

        self.popup = Some(Popup::Window(w));

        cx.notify();

    }



    /// 关闭变量编辑器：只隐藏，**保留实体和呈现模式**（下次打开恢复）。

    fn close_variables(&mut self, cx: &mut Context<Self>) {

        // 关掉时清掉可能挂着的拖动状态，否则隐藏的实体仍会让 `popup_dragging()` 为真。

        if let Some(w) = &mut self.variables_editor {

            // 连编辑器内部的拖动状态一起清（行排序 / 列宽 / 滚动条）：残留会让

            // `window_drag_active()` 永远为真，关闭后画布拖动会被根视图抢走。

            w.variables_end_drag();

        }

        self.variables_open = false;

        cx.notify();

    }



    /// 当前承载拖动状态的窗口：`popup` 里的通用窗口，或**打开中的**变量编辑器。

    ///

    /// ⛔ 关闭后的变量编辑器实体仍然保留（记住位置/尺寸/选中），但它的拖动状态**不能**再参与

    /// 窗口级路由：`internal_drag()` 里任何一个残留（例如滚动条 `hscroll_drag` 没被 mouse-up 清掉）

    /// 都会让 `window_drag_active()` 永远为真，根视图于是把**画布的**每次 move 都抢走并

    /// `stop_propagation()`——表现就是「页面原本的拖动一塌糊涂」。所以这里必须看 `variables_open`。

    fn active_window_mut(&mut self) -> Option<&mut popovers::WindowState> {

        if let Some(Popup::Window(w)) = &mut self.popup {

            return Some(w);

        }

        if !self.variables_open {

            return None;

        }

        self.variables_editor.as_mut()

    }

    fn active_window(&self) -> Option<&popovers::WindowState> {

        if let Some(Popup::Window(w)) = &self.popup {

            return Some(w);

        }

        if !self.variables_open {

            return None;

        }

        self.variables_editor.as_ref()

    }



    // 通用窗口的标题栏可拖动：按下时记下抓取偏移，移动时改 place，松开结束。

    fn begin_popup_drag(&mut self, at: Point<Pixels>) {

        if let Some(w) = self.active_window_mut() {

            // Record the press only — no drag, no repaint. Repainting on every press is what made

            // the header buttons stop firing: GPUI matches a click's down/up by `ElementId`, and a

            // repaint between them can shift the id. The drag latches in `move_popup_drag` once the

            // pointer clears a small dead zone (egui's `click_and_drag`).

            w.press(at);

        } else if let Some(d) = &mut self.dialog {

            d.press(at);

        }

    }

    fn move_popup_drag(&mut self, at: Point<Pixels>, pressed: bool) -> bool {

        // 统一路由：resize（editor 按住边缘 handle）改窗口尺寸，否则整卡移动。**每次移动都直接

        // 改状态并让调用方 notify，绝不节流**：

        //   * GPUI 自己会把同一帧内的多次 `cx.notify()` 合并成一次渲染（按 vsync），所以「每个

        //     move 都 notify」根本不会导致多渲染；

        //   * 反过来，手写的 12ms 节流会平白加一帧延迟，并且**丢掉松手前最后一次移动**——表现

        //     出来就是「窗口非常卡、完全不跟鼠标」。用户 2026-11-07 实测确认这是逻辑错误，

        //     不是渲染慢，根因就是这段节流。

        self.active_window_mut().is_some_and(|w| w.variables_route_move(at)) || self.dialog.as_mut().is_some_and(|d| d.route_move(at))

    }

    /// 尺寸拖动的**窗口级接管**：左面板宽 + 底栏时间轴高共用 `resize_drag`。
    ///
    /// root 的 `on_mouse_move` 与 `on_window_drag_move` 都调这一份（以前两处逐字重复）。
    /// 指针移出 4px 覆盖条 / 根 div 后逐元素 move 停发，靠窗口轴相对位移续算；窗口外松手时
    /// 下一帧 move 不带左键，在这里收尾，避免状态残留。返回是否消费了这次移动。
    fn route_size_drag(&mut self, pos: Point<Pixels>, pressed: bool) -> bool {

        if !crate::resize_drag::any_active() {

            return false;

        }

        if !pressed {

            crate::resize_drag::end_any();

        } else {

            if let Some(w) = crate::resize_drag::route_move(
                crate::resize_drag::Handle::LeftPanel, f32::from(pos.x)) {
                self.left_w = w;
            }
            if let Some(h) = crate::resize_drag::route_move(
                crate::resize_drag::Handle::BottomTimeline, f32::from(pos.y)) {
                self.motion_q.push(crate::motion::Act::SetTimelineHeight(h));
            }

        }

        true

    }

    /// 指针在**没有按住左键**的情况下移动：一律收掉卡片拖动。

    ///

    /// 这是「必须一直按住才拖」的保证：GPUI 在指针移出窗口后松手时不会补发 `mouse_up`，

    /// 只靠 up 事件收尾的话，`grab` / `pending` 会留到下一次按下——于是松手之后光是移动鼠标，

    /// 卡片就又开始跟着走（用户 2026-11-08 明确指出这个行为不对）。每次 move 带着真实的按键

    /// 状态，就永远不会「没按住还拖」。

    fn cancel_drag_if_not_held(&mut self, held: bool) -> bool {

        if held {

            return false;

        }

        let clip_drag = crate::motion::clip_popover::drag_active();
        if clip_drag {
            // 窗口外松手后指针回来、左键没按住：收掉 clip 卡片拖动，绝不「没按住还跟着走」。
            crate::motion::clip_popover::end_drag();
        }

        let busy = self.window_drag_active() || self.popup_dragging() || self.popup_pending();

        if busy {

            self.end_popup_drag();

        }

        busy || clip_drag

    }

    /// True while a detached window's header is held. The canvas skips its hover hit-testing

    /// during a drag (that is the expensive part that made dragging feel sticky).

    fn popup_dragging(&self) -> bool {

        self.active_window().is_some_and(|w| w.dragging()) || self.dialog.as_ref().is_some_and(|d| d.dragging())

    }

    /// True while a press on a popover is held but hasn't moved enough to count as a drag.

    fn popup_pending(&self) -> bool {

        self.active_window().is_some_and(|w| w.pending().is_some()) || self.dialog.as_ref().is_some_and(|d| d.pending())

    }

    /// 正在做「高频、只改几何」的拖动：整卡移动 / 画布平移 / 缩放框 / 左右工具栏拖宽 /

    /// 控件树拖动 / 时间轴拖动。这些帧里右栏内容不会变，`sync_panel`（读整份文档、收集列表）

    /// 每帧重跑一遍是画布/工具栏拖动发涩的主要来源——拖动期间复用上一帧的 `PanelData`。

    fn dragging_heavy(&self) -> bool {

        self.popup_dragging()

            || self.window_drag_active()

            || self.pan.is_some()

            || self.zoom_drag.is_some()

            || self.resizing.is_some()

            || crate::resize_drag::any_active()

            || self.layer_tree.drag.is_some()

            || crate::motion::timeline::drag_active()

            || crate::motion::easing::drag_active()

            || crate::motion::clip_popover::drag_active()

    }

    /// 是否有「窗口级」拖拽正在进行——即任何需要由 `on_window_drag_*` 统一接管的拖拽。

    /// 根 div 的 hover/命中 handler 据此跳过，避免与窗口级双发；窗口级 handler 自身也用它

    /// 做无拖拽时的早退（几乎零开销）。覆盖：画布平移/缩放/Move、多选间距/变换手柄、面板 resize、

    /// 控件树拖动、整卡移动（含 resize handle）、时间轴/easing 拖拽、可复用拖拽路由器、Ctrl+右键平移。

    fn any_window_drag_active(&self) -> bool {

        self.dragging

            || self.pan.is_some()

            || self.zoom_drag.is_some()

            || self.resizing.is_some()

            || crate::resize_drag::any_active()

            || self.layer_tree.drag.is_some()

            || self.window_drag_active()

            || self.right_pan

            || crate::drag_router::is_active()

            || crate::motion::timeline::drag_active()

            || crate::motion::easing::drag_active()

            || self.app.selection_spacing.is_dragging()

            || self.app.handles.drag.is_some()

    }

    fn end_popup_drag(&mut self) {

        if let Some(w) = self.active_window_mut() {

            // 同时清掉整卡移动（pending/grab）与 resize（editor 内部状态）。

            w.variables_end_drag();

        }

        if let Some(d) = &mut self.dialog {

            d.end_drag();

        }

    }

    /// 画布级拖拽（平移 / 缩放 / 框选拖移 / 间距手柄 / padding 手柄）的**窗口级**路由。

    ///

    /// 这些拖拽原本只在画布元素的 `on_mouse_move` 里推进，指针移到画布外的面板、或拖出窗口

    /// 就断；窗口外松手收不到 `on_mouse_up`，状态残留。把「继续拖拽」与「按键已松开→收尾」都搬

    /// 到窗口根视图（覆盖整窗），指针离开画布/窗口也能连续，窗口外松手也能干净结束（用户

    /// 2026-11-08 指出的监听失效）。返回 true 表示事件被拖拽消费，调用方应跳过 hover 处理。

    fn route_canvas_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {

        let active = self.dragging

            || self.pan.is_some()

            || self.zoom_drag.is_some()

            || self.app.selection_spacing.is_dragging()

            || self.app.handles.drag.is_some()

            || self.app.motion_gizmo.drag.is_some();

        if !active {

            return false;

        }

        // 窗口外松手：下一帧 move 带「按键已松开」回来，在这里收尾——每一类画布拖拽都按

        // 它们原本 `on_mouse_up` 的语义结束（缩放要 finish_zoom、框选/变换手柄要用「抬起」

        // 语义写回、frame 拖动要发 Up），否则回窗后还会继续平移 / 拖手柄，或改动没落库。

        // Ctrl+右键平移时 `Right` 也算「按住」，否则一移动就被判成松手（用户 2026-11-08）。

        let held = e.pressed_button == Some(MouseButton::Left)

            || (self.right_pan && e.pressed_button == Some(MouseButton::Right));

        if !held {

            let pos = self.local(e.position);

            let was_dragging = self.dragging;

            self.pan = None;

            self.right_pan = false;

            let zoom_start = self.zoom_drag.take();

            self.dragging = false;

            if let Some(start) = zoom_start {

                self.finish_zoom(start, pos, e.modifiers.alt);

            }

            if was_dragging {

                self.send(PointerKind::Up, pos, &e.modifiers);

            }

            if self.app.selection_spacing.is_dragging() {

                let _ = canvas::selection_spacing::input(

                    &mut self.app,

                    Some(pos),

                    true,

                    self.space,

                    false,

                    false,

                    true,

                    e.modifiers.platform || e.modifiers.control,

                    e.modifiers.alt,

                    e.modifiers.shift,

                );

            }

            if self.app.motion_gizmo.drag.is_some() {

                let _ = canvas::motion_overlay::input(&mut self.app, pos, false, false, true, false);

            }

            if self.app.handles.drag.is_some() {

                let _ = canvas::handles::input(&mut self.app, pos, false, false, true, e.modifiers.shift);

            }

            cx.notify();

            return true;

        }

        // 继续拖拽（与画布元素原处理逻辑一致，但由窗口级接管）。

        let pos = self.local(e.position);

        self.mods = e.modifiers;

        self.pointer = Some(pos);

        if let Some((p0, cx0, cy0)) = self.pan {

            if let Ok(ds) = self.app.session.ds_mut() {

                let z = ds.view.zoom.max(1e-6);

                ds.view.cx = cx0 - f64::from(pos.x - p0.x) / z;

                ds.view.cy = cy0 - f64::from(pos.y - p0.y) / z;

                ds.revision += 1;

            }

            cx.notify();

            return true;

        }

        if self.zoom_drag.is_some() {

            cx.notify();

            return true;

        }

        if self.app.motion_gizmo.drag.is_some() {

            let _ = canvas::motion_overlay::input(&mut self.app, pos, false, self.primary_down, false, e.modifiers.control);

            cx.notify();

            return true;

        }

        if self.dragging {

            self.send(PointerKind::Move, pos, &e.modifiers);

            cx.notify();

            return true;

        }

        if self.app.selection_spacing.is_dragging() {

            let _ = canvas::selection_spacing::input(

                &mut self.app,

                Some(pos),

                true,

                self.space,

                false,

                self.primary_down,

                false,

                e.modifiers.platform || e.modifiers.control,

                e.modifiers.alt,

                e.modifiers.shift,

            );

            cx.notify();

            return true;

        }

        if self.app.handles.drag.is_some() {

            let _ = canvas::handles::input(&mut self.app, pos, false, self.primary_down, false, e.modifiers.shift);

            cx.notify();

            return true;

        }

        true

    }

    /// 是否有窗口级拖拽正在进行（整卡移动 pending/dragging，或变量编辑器的 resize handle 按住）。

    /// 根 `on_mouse_move` 据此接管，使拖拽在指针移出「最初按下那个元素」后依然连续——

    /// 否则像 motion 时间轴一样，指针一离开 body，body 自己的 `on_mouse_move` 就停发、拖拽断掉。

    fn window_drag_active(&self) -> bool {

        self.active_window().is_some_and(|w| w.variables_active())

    }



    /// 端到端测试读取：变量卡渲染后的原点（窗口坐标）。

    #[cfg(test)]

    pub(crate) fn test_variables_origin(&self) -> Option<Point<Pixels>> {

        self.variables_editor.as_ref().map(|w| w.test_origin())

    }

    /// 端到端测试读取：拖拽后的卡片落点（None = 还没拖动过）。

    #[cfg(test)]

    pub(crate) fn test_variables_place(&self) -> Option<Point<Pixels>> {

        self.variables_editor.as_ref().and_then(|w| w.test_place())

    }

    /// 仅供端到端测试：打开一张 Floating 变量面板（绕过左栏 rail 点击路径）。

    /// 放这里是因为 `fields` / `open_window` 都是私有，测试模块（同级）访问不到。

    #[cfg(test)]

    pub(crate) fn test_open_variables_floating(&mut self, cx: &mut Context<Self>) {

        let data = crate::panels::right::variables::VariablesData { collections: Vec::new(), libraries: Vec::new() };

        let anchor = Point::new(px(100.0), px(100.0));

        let win = popovers::WindowState::variables_table(self.fields.queue(), anchor, data);

        self.open_window(win, cx);

    }

    /// 端到端测试读取：resize 后的卡片尺寸（None = 还没改过尺寸）。

    #[cfg(test)]

    pub(crate) fn test_variables_size(&self) -> Option<(f32, f32)> {

        self.variables_editor.as_ref().and_then(|w| w.test_size())

    }

    /// 端到端测试读取：左面板当前宽度。

    #[cfg(test)]

    pub(crate) fn test_left_w(&self) -> f32 {

        self.left_w

    }

    /// 端到端测试读取：右面板当前宽度。

    #[cfg(test)]

    pub(crate) fn test_right_w(&self) -> f32 {

        self.right_w

    }

    /// 端到端测试读取：当前是否正在拖拽某条面板分隔条（None = 没有）。

    #[cfg(test)]

    pub(crate) fn test_resizing(&self) -> Option<ResizeSide> {

        // 左栏已迁到 `resize_drag` 的 thread_local 状态机（与 timeline 顶边共用）；右栏仍用
        // `self.resizing`。测试辅助把两者统一成同一个 `ResizeSide` 视图，保持既有断言语义。
        if crate::resize_drag::drag_active(crate::resize_drag::Handle::LeftPanel) {
            Some(ResizeSide::Left)
        } else {
            self.resizing
        }

    }



    /// 关闭当前弹层（backdrop 点击 / 菜单项选中 / 取色后都会调用）。

    fn close_popup(&mut self, cx: &mut Context<Self>) {

        crate::widgets::dropdown::set_active_menu(None);

        // A flyout's open mark lives on the view, not in the popup: closing either way (outside

        // click, Escape, a pick) must clear it, or the caret keeps reading as open.

        self.tool_flyout = None;

        // 变量编辑器独立于 popup：关闭主菜单/取色器等不影响它，`variables_full` 是会话记忆，

        // 绝不能在这里清零（否则每次重新打开都退回 Floating）。

        if self.popup.take().is_some() {

            cx.notify();

        }

    }



    /// 变量面板在 Floating / Full 之间切换（header 的展开按钮用）。

    /// 左栏选中态由 `variables_full` 驱动，所以切换后要重画一帧。

    fn set_variables_full(&mut self, full: bool, cx: &mut Context<Self>) {

        self.variables_full = full;

        // 同步回实体：卡片几何由 `WindowState::variables_full()` 读 editor 状态决定。

        if let Some(w) = &mut self.variables_editor {

            w.set_variables_full(full);

        }

        cx.notify();

    }



    /// 应用混合模式（egui `paints.rs` 的 `set_props {"blend": label}`）。

    fn apply_blend(&mut self, label: &str, cx: &mut Context<Self>) {

        let ids = self.app.session.ds().ok().map(|d| d.selection.clone()).unwrap_or_default();

        if !ids.is_empty() {

            let mut props = serde_json::Map::new();

            props.insert("blend".into(), serde_json::json!(label));

            let _ = self.app.session.execute("node.set", &serde_json::json!({ "ids": ids, "props": props }));

            cx.notify();

        }

        self.fields.set_blend(label, cx);

    }



    /// 应用填充色（egui `paints.rs` 的 `node.set {"fill": "#RRGGBB"}`）。

    fn apply_fill(&mut self, color: Color32, cx: &mut Context<Self>) {

        let ids = self.app.session.ds().ok().map(|d| d.selection.clone()).unwrap_or_default();

        if !ids.is_empty() {

            let mut props = serde_json::Map::new();

            props.insert("fill".into(), serde_json::json!(color.to_hex()));

            let _ = self.app.session.execute("node.set", &serde_json::json!({ "ids": ids, "props": props }));

            cx.notify();

        }

        self.fields.set_fill_color(color, cx);

    }



    /// 应用描边色（egui `paints.rs` 的 `node.set {"stroke": "#RRGGBB"}`，与 `apply_fill` 同构）。

    fn apply_stroke(&mut self, color: Color32, cx: &mut Context<Self>) {

        let ids = self.app.session.ds().ok().map(|d| d.selection.clone()).unwrap_or_default();

        if !ids.is_empty() {

            let mut props = serde_json::Map::new();

            props.insert("stroke".into(), serde_json::json!(color.to_hex()));

            let _ = self.app.session.execute("node.set", &serde_json::json!({ "ids": ids, "props": props }));

            cx.notify();

        }

        self.fields.set_stroke_color(color, cx);

    }

}



impl Render for CanvasView {

    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {

        // Inline rename commits land here (see `inline_rename_pending`): Enter, focus-out or a

        // click elsewhere all go through the same queue, and the editor is closed either way.

        let committed = self.inline_rename_pending.borrow_mut().take();

        if let Some((target, ids, name)) = committed {

            if self.rename_cancel {

                self.rename_cancel = false;

            } else if !name.is_empty() && !ids.is_empty() {

                if target == InlineRenameTarget::Page {

                    self.run_command("page.rename", serde_json::json!({ "id": ids[0], "name": name }), cx);

                } else {

                    self.run_command("object.rename", serde_json::json!({ "ids": ids, "name": name }), cx);

                }

            }

            self.inline_rename = None;

        }

        // 帧统计：一帧画一次（原文 `FrameStats::record(ctx)` 同处）。

        self.app.stats.record();

        // i18n: the language can change from the Preferences dialog, the palette or a menu; sync
        // the catalog every frame so the change takes effect at once (an atomic store, cheap).
        figcraft_i18n::set(&self.app.session.prefs.language, system_language().as_deref());

        // 每帧渲染开始时把 icon 按钮的稳定 id 序号归零。右栏/弹窗里的 icon 按钮每帧重建，

        // 只有按固定构建顺序重新编号，同一按钮才能在 mouse-down / mouse-up 两帧之间拿到同一个

        // `ElementId`——GPUI 靠它匹配点击，id 每帧变则 `on_click` 永不触发（按钮"点不动"）。

        crate::widgets::icon_button::reset_icon_btn_ids();

        // Motion：先落地按钮动作（写回引擎 / 改播放头），再画这一帧，否则点「添加关键帧」

        // 要等到下一帧才看得见。播放中的续帧也由这里推进时钟。

        self.sync_motion(cx);

        // 引擎 ui_requests：先消费命令留下的 UI 请求（切标签 / 开面板），再同步 Dev Mode。

        self.sync_ui_requests(cx);

        self.sync_present();

        // Dev Mode（Inspect 标签）：每帧把右栏标签 / 注解过滤器同步给画布与引擎工具状态。

        // egui 在 `FigApp::sync_dev_mode()` 做同一件事（进入时结束文字/矢量编辑、丢掉手势）。

        self.sync_dev_mode();

        // 控制通道：在真实的一帧里把排到的请求答掉（与原文 `drain(app, ctx)` 同处）。

        // 有请求时再画一帧，让回复反映画出来的 UI。

        if control::drain(&mut self.app, &self.control_queue) {

            cx.notify();

        }

        // 画布 ↔ 右栏：先写回提交的字段，再把选中/几何刷回面板。

        // 一帧只读一次文档：`sync_panel` 现在把这次读到的 `PanelData` 返回，整帧复用；

        // 拖动时直接复用上一帧（`last_panel`），不再每帧全量读文档。

        let panel_data = if self.dragging_heavy() {

            self.last_panel.clone().unwrap_or_else(|| self.panel_data())

        } else {

            let p = self.sync_panel(cx);

            self.last_panel = Some(p.clone());

            p

        };

        // 每帧确认右栏字段能找到根视图（见 `RightFields::ensure_root`）。启动时那次

        // set_root 若因时序没落地，所有开窗按钮都会被 `let Some(root) = … else { return }`

        // 静默吞掉 —— 现象就是「按钮点了没反应」。

        self.fields.ensure_root(cx.entity());

        // 菜单关掉（或被别的弹层顶掉）后，别让按钮继续写跨帧探针。

        if !matches!(&self.popup, Some(Popup::Menu { .. })) {

            crate::widgets::dropdown::set_active_menu(None);

        }

        // 触发按钮的选中态 / 「再次点击关闭」一律由**真实 popup 状态**驱动，不靠按钮自己记：

        // 每帧把当前窗口写回共享槽，于是 backdrop / Escape / X 关窗后按钮同步熄灭。

        let popup_reqs = self.fields.popups();

        if popup_reqs.take_close() {

            self.close_popup(cx);

            if self.variables_open {

                self.close_variables(cx);

            }

        }

        let current_kind = match &self.popup {

            Some(Popup::Window(w)) => Some(w.kind()),

            _ => self.variables_open.then_some(popovers::WindowKind::VariablesTable),

        };

        popup_reqs.sync_open(current_kind);

        // 右栏按钮把「要打开的窗口」放进共享槽，这里每帧取走。按钮因此不需要持有根视图

        // Entity，也就不会被 `let Some(root) = … else { return }` 静默吞掉。

        if let Some(w) = self.fields.take_window() {

            if std::env::var_os("FC_DEBUG_POPUP_BTN").is_some() {

                eprintln!("[popup-btn] frame took a window request");

            }

            // 走统一入口，变量窗口才能拿到 Full / Close 回调。

            self.open_window(w, cx);

        }

        let vp = window.viewport_size();

        // 画布被左右面板夹在中间：按固定布局反推它的尺寸（顺序见下方根 div 树：

        // 导轨 56 | 左面板 left_w | 手柄 4 | 画布 | 手柄 4 | 右面板 right_w）。

        // 注意：画布内部（overlay/labels/painter）都以**画布局部坐标**（原点 0）工作，

        // `Painter::paint` 再叠加画布元素在窗口中的原点——所以这里只给尺寸，min 保持 0。

        // Dock 布局值：本帧所有几何（画布原点 / 尺寸 / 弹层锚点）都从这一份值推导。
        // B9：两个标志只影响「画不画」与 Dock 宽度，不改控件本身，也不改记住的面板宽度。
        let (ui_hidden, ui_minimized) = self.ui_flags();
        // 画板文档的工具栏换成一整套画板工具组，宽度和飞出菜单的锚点都跟着变；左右栏与



        // 动画栏在画板上都默认收起。`self.popup` 下面要被可变借用，所以提前取好。



        let board = self.is_board_doc();



        // 画板没有图层树 / 竖条 / 动画面板：整条侧栏让位给画布（用户 2026-11-10）。



        let show_sidebars = !ui_hidden && !ui_minimized && !board;



        let show_right = !ui_hidden

            && (!board || self.board_panel)

            && (!ui_minimized || self.app.session.ds().map(|d| !d.selection.is_empty()).unwrap_or(false));
        let dock = self.dock();
        let canvas_x = dock.center_x();

        let canvas_w = dock.center_w(f32::from(vp.width));

        // 只扣系统级标题栏区域与 bottom dock；Dock 内容区从 y = TITLE_H 起。
        let canvas_h = dock.canvas_size(f32::from(vp.width), f32::from(vp.height)).1;

        if std::env::var_os("FC_DEBUG_SIZE").is_some() {

            eprintln!(

                "[size] vp={:?} canvas_x={} canvas_w={} canvas_h={} left={} right={}",

                (f32::from(vp.width), f32::from(vp.height)),

                canvas_x,

                canvas_w,

                canvas_h,

                self.left_w,

                self.right_w

            );

        }

        self.app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(canvas_w, canvas_h));

        // 焦点不再每帧强抢：只有点画布时才聚焦（见下方左键 on_mouse_down），

        // 否则右栏输入框永远拿不到焦点、打不进字。



        // 主题随偏好实时重建（主菜单切亮/暗与强调色后立即生效）。

        // ⛔ 光算出 `Tokens` 不够：所有控件是从 `widgets::tokens()` 取颜色的，必须把这一帧的

        // 令牌写进去，否则界面永远停在写死的暗色（`tokens()` 以前直接返回 `Tokens::dark()`）。

        let dark = match self.app.session.prefs.theme.as_str() {

            "light" => false,

            "dark" => true,

            // 跟随系统：gpui 的窗口外观（`WindowAppearance`）。

            _ => matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark),

        };

        let accent = crate::theme::Accent::from_str(&self.app.session.prefs.accent);

        self.app.theme = crate::theme::Tokens::new(dark, accent);

        crate::widgets::set_tokens(self.app.theme);



        let t = self.app.theme;

        let ppp = window.scale_factor();

        // 描边按设备像素吸附（见 `outline::crisp_rect`）。

        self.app.ppp = ppp;



        let shown = self.cursor_now();

        // 画布光标必须只在指针真的落在画布矩形里时生效 —— 原文 `canvas.rs` 的门控是

        // `own_cursor && resp.hovered()`。`self.pointer` 只由画布自己的 mouse / wheel 事件写入，

        // 指针移出画布后没有任何事件会把它清空，直接拿它当「悬停」等于「曾经进过画布就永远悬停」。

        // 这一点在 egui 里只是多画一个光标，在 GPUI 里却是致命的：`set_window_cursor_style`

        // 是**整窗**级且优先级高于所有元素的 `.cursor()`（见 window.rs 的文档与

        // `Frame::cursor_style` 的 `fold_while`：遇到 hitbox_id == None 直接 Done），

        // 于是右栏数字输入框前导 24pt 格的 `ResizeLeftRight` 会被永久压掉。

        let mouse = self.local(window.mouse_position());

        let over_canvas = window.is_window_hovered() && mouse.x >= 0.0 && mouse.x < canvas_w && mouse.y >= 0.0 && mouse.y < canvas_h;

        // 手柄光标每个事件重设、每帧取一次：指针移开带子后不该留着。

        self.app.spacing_cursor = None;

        self.app.cursor = over_canvas.then_some(shown);

        self.app.pointer = over_canvas.then_some(mouse);

        self.app.zoom_drag = self.zoom_drag;



        // 松开键已经抬起、但这次抬起没落在画布上：结束参考线拖动（原文每帧调一次）。

        if !self.primary_down {

            canvas::guides::end_released(&mut self.app);

        }

        // Motion 播放头预览（egui `motion_preview` 的等价）：这一帧让画布画「在 playhead

        // 求值后的文档」，画完立刻还原真实文档。引擎 / 撤销栈 / 选中状态全程不受影响，

        // 因为替换的只是 `DocState.doc` 这一份 `Arc<Document>`，且同一帧内还原。

        let saved_doc = self.motion_preview_doc().and_then(|preview| {

            let ds = self.app.session.ds_mut().ok()?;

            Some(std::mem::replace(&mut ds.doc, preview))

        });

        let CanvasOutput { image, overview, live, painter, bg, in_flight, retired } = canvas::show(&mut self.app, &mut self.worker, ppp, &t);

        if let Some(real) = saved_doc

            && let Ok(ds) = self.app.session.ds_mut()

        {

            ds.doc = real;

        }



        // 换下来的图必须显式还给 GPUI：它把每张 `RenderImage` 放进 sprite atlas，

        // 不 drop 就永远占着（缩放/拖动/旋转会持续产图，这是内存一直涨的那条）。

        for old in retired {

            cx.drop_image(old, Some(window));

        }



        // 还有渲染在飞 → 在 paint 回调里续帧（对应 egui 的 request_repaint）。

        // 画布的工具光标改成下面画布 div 上的 `.cursor(style)`（hitbox 级）：整窗光标会压掉

        // 左右面板所有控件自己的光标，包括数字输入框 24pt 前导格的左右箭头。

        let style = shown.icon();



        // 贴图：整页图像重投影到当前视图。

        let mut layers: Vec<gpui::AnyElement> = Vec::new();

        // 整页兜底图垫在最下面：清晰贴图盖住的部分看不见它，没盖住的那条露出来的是它。
        // 平移时于是没有一帧是空白 —— 新区域"一拖过去就已经在那儿"了。

        if let Some((tex, r)) = overview {

            layers.push(

                img(ImageSource::Render(tex))

                    .absolute()

                    .left(px(r.min.x))

                    .top(px(r.min.y))

                    .w(px(r.width().max(1.0)))

                    .h(px(r.height().max(1.0)))

                    .into_any_element(),

            );

        }

        if let Some((tex, r)) = image {

            layers.push(

                img(ImageSource::Render(tex))

                    .absolute()

                    .left(px(r.min.x))

                    .top(px(r.min.y))

                    .w(px(r.width().max(1.0)))

                    .h(px(r.height().max(1.0)))

                    .into_any_element(),

            );

        }



        // 拖动中的图层：贴着整页贴图、压在叠加层之下——它和选框同一帧动，

        // 不用等下一个渲染往返（贴图那边已经把这几层让开了）。

        // 裁剪框不跟着图走，所以在这里套：图层移到 frame 边上，该没的那一半立刻就没。

        if let Some((tex, r, clip)) = live {

            let picture = || img(ImageSource::Render(tex.clone())).absolute().w(px(r.width().max(1.0))).h(px(r.height().max(1.0)));

            match clip {

                Some(c) if c.width() > 0.0 && c.height() > 0.0 => layers.push(

                    div()

                        .absolute()

                        .left(px(c.min.x))

                        .top(px(c.min.y))

                        .w(px(c.width()))

                        .h(px(c.height()))

                        .overflow_hidden()

                        .child(picture().left(px(r.min.x - c.min.x)).top(px(r.min.y - c.min.y)))

                        .into_any_element(),

                ),

                _ => layers.push(picture().left(px(r.min.x)).top(px(r.min.y)).into_any_element()),

            }

        }



        // 图元层：记录下来的绘制指令交给 GPUI 低层 API。

        let ops = painter.ops.clone();

        // 文字编辑时把 IME 输入处理器挂到本视图：组合（preedit）由它接管，

        // 引擎 `textedit` 已经把组合/提交/删除周边都实现好了，这里只做 GPUI ↔ 引擎的桥。

        let text_editing = self.app.session.ds().is_ok_and(|d| d.text_edit.is_some());

        let ime_focus = self.focus.clone();

        let ime_entity = cx.entity();

        // Motion 播放中：画布必须每帧续帧，否则 `advance()` 只推进一帧就停（播放按钮点了不走）。

        let motion_playing = self.motion && self.motion_ui.playing;

        let present_playing = self.present.as_ref().is_some_and(|p| p.playing || p.from.is_some());

        // 时长手柄停在时间轴右端滚动区时，指针不动也要继续伸长（egui `request_repaint`）。

        // 必须在这里推动作：paint 阶段推队列会重入唤醒，render 里推是安全点。

        let duration_ticking = if self.primary_down && crate::motion::timeline::duration_drag_active() {

            let mp = window.mouse_position();

            let o = crate::motion::timeline::bar_origin();

            crate::motion::timeline::tick_duration(

                &self.motion_q,

                pos2(f32::from(mp.x) - f32::from(o.x), f32::from(mp.y) - f32::from(o.y)),

            )

        } else {

            false

        };

        let shapes = gpui::canvas(

            |_, _, _| (),

            move |b, _, window, cx| {

                // 这里**不能**调 `window.set_window_cursor_style(style)`：它是整窗级、优先级

                // 高于一切 hitbox 光标的（GPUI `window.rs`），会让右栏 24pt 前导格的

                // `ResizeLeftRight`、IBeam、PointingHand 全部失效。画布光标改由画布 div 的

                // `.cursor(style)` 按 hitbox 悬停生效，和 egui 的 `resp.hovered()` 等价。

                // IME：每帧在 paint 阶段重新登记输入处理器（与 GPUI 的文本元素同一机制）。

                if text_editing {

                    window.handle_input(&ime_focus, ElementInputHandler::new(b, ime_entity.clone()), cx);

                }

                let p = Painter { clip: Rect::NOTHING, ops: ops.clone(), labels: Vec::new() };

                p.paint(window, pos2(f32::from(b.origin.x), f32::from(b.origin.y)));

                // 续帧只能在 request_layout/prepaint/paint 里调，所以放这里。

                // 播放中同样要续帧：动画的每一帧都是新的一帧画面。

                if in_flight || motion_playing || present_playing || duration_ticking {

                    window.request_animation_frame();

                }

            },

        )

        .absolute()

        .size_full();

        layers.push(shapes.into_any_element());

        // A25: the plugin widget's property bar, above the selected widget.
        if let Some(bar) = widget_bar::bar(&self.app, self.fields.queue()) {
            layers.push(bar);
        }

        // B3: the board selection's bar (colours first), above the selected board objects.
        if let Some(bar) = board_bar::bar(&self.app, self.fields.queue(), &self.board_shape_open, &self.board_line_open) {
            layers.push(bar);
        }



        // 文字层：GPUI 矢量文本，位置/居中由标签矩形决定。

        let mut texts: Vec<gpui::AnyElement> = Vec::new();

        for l in &painter.labels {

            // 旋转标签（左标尺数字）：GPUI 的文字管线没有变换矩阵，只有 SVG 绘图有。
            // 把整段文字做成内联 SVG 后绕自身中心转 -90°，等价于 egui 的
            // `TextShape::with_angle_and_anchor(-π/2, CENTER_CENTER)`。
            if let Some(angle) = l.angle {
                let key = canvas::svg_text::text_key(&l.text, l.font.size, l.rect.width(), l.rect.height());
                let rotated = gpui::svg()
                    .absolute()
                    .left(px(l.rect.min.x))
                    .top(px(l.rect.min.y - widgets::TEXT_NUDGE))
                    .w(px(l.rect.width().max(1.0)))
                    .h(px(l.rect.height().max(1.0)))
                    .text_color(canvas::painter::to_rgba(l.color))
                    .path(key)
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(angle)));
                texts.push(rotated.into_any_element());
                continue;
            }

            let mut el = div()

                .absolute()

                .left(px(l.rect.min.x))

                // 与控件文字同一套基线修正，标签与几何不脱节。

                .top(px(l.rect.min.y - widgets::TEXT_NUDGE))

                .w(px(l.rect.width().max(1.0)))

                .h(px(l.rect.height().max(1.0)))

                .flex();

            el = match l.align.y {

                0 => el.items_start(),

                2 => el.items_end(),

                _ => el.items_center(),

            };

            el = match l.align.x {

                0 => el.justify_start(),

                2 => el.justify_end(),

                _ => el.justify_center(),

            };

            texts.push(

                el.child(

                    div()

                        .text_size(px(l.font.size))

                        .text_color(canvas::painter::to_rgba(l.color))

                        // 标签框可能很窄（左侧标尺只有 20px），换行会把 "-150" 拆成竖排。

                        .whitespace_nowrap()

                        .child(l.text.clone()),

                )

                .into_any_element(),

            );

        }



        let canvas = div()

            .flex_1()

            .min_w(px(0.0))

            .h_full()

            .relative()

            // 工具光标（含自绘字形时的 `CursorStyle::None`）只挂在**画布自己的 hitbox** 上：

            // 指针在画布里才生效，移进右栏就自动让位给控件的 `.cursor()`。

            // `Frame::cursor_style` 只挑选「被悬停的 hitbox」的请求，正好对应 egui

            // `canvas.rs` 里的 `resp.hovered()`；`Shown::Drawn` 时系统光标隐藏、字形自绘。

            .cursor(style)

            // 画布内的贴图/图元/标签都是 `absolute`，GPUI 默认不裁剪：页面贴图比视口大时

            // 会直接画到左面板上（"工具栏被画布嵌进去一截"、多出来的色块）。裁到画布边界。

            .overflow_hidden()

            .bg(canvas::painter::to_rgba(bg))

            .track_focus(&self.focus)

            .children(layers)

            .children(texts)

            // 聚焦视图顶栏（egui `dev_mode::focus_bar`），浮在画布上方居中。

            .children(self.dev_focus_bar())

            // 左键按下：平移 / 缩放工具 / 双击 / 交给引擎的 Down。

            .on_mouse_down(

                MouseButton::Left,

                cx.listener(|this: &mut CanvasView, e: &MouseDownEvent, window, cx| {

                    // 点画布 = 整窗失焦：先把右栏所有仍在编辑的草稿提交，再把焦点给画布。

                    // 只靠 TextEdit::render() 轮询 focus 会漏：GPUI 对未标脏的嵌套 Entity

                    // 复用缓存 prepaint，焦点转移的那一帧不一定会重绘到那个 Entity；

                    // focus-out 订阅之外再统一兜一次，保证点画布当帧就写回文档。

                    this.commit_edits(window, cx);

                    // 点画布才把焦点给画布（不再每帧强抢，右栏输入框才能保留焦点）。

                    this.focus.focus(window);

                    this.mods = e.modifiers;

                    let pos = this.local(e.position);

                    this.pointer = Some(pos);

                    this.primary_down = true;

                    if !this.space && !this.zoom_key && this.guide_input(pos, true, false, e.modifiers.alt) {

                        cx.notify();

                        return;

                    }

                    // 多选间距/重排手柄：原文 `selection_spacing::input` 在 `handles::input` 之前。

                    if !this.space

                        && !this.zoom_key

                        && canvas::selection_spacing::input(

                            &mut this.app,

                            Some(pos),

                            true,

                            this.space,

                            true,

                            true,

                            false,

                            e.modifiers.platform || e.modifiers.control,

                            e.modifiers.alt,

                            e.modifiers.shift,

                        )

                    {

                        cx.notify();

                        return;

                    }

                    // Motion gizmo：圆环 / 箭头 / 缩放块的命中与起拖，优先于设计态手柄。
                    if !this.space && !this.zoom_key && canvas::motion_overlay::input(&mut this.app, pos, true, true, false, e.modifiers.control) {

                        cx.notify();

                        return;

                    }

                    if !this.space && !this.zoom_key && canvas::handles::input(&mut this.app, pos, true, true, false, e.modifiers.shift) {

                        cx.notify();

                        return;

                    }

                    if this.space {

                        if let Ok(ds) = this.app.session.ds() {

                            this.pan = Some((pos, ds.view.cx, ds.view.cy));

                        }

                    } else if this.zoom_key {

                        this.zoom_drag = Some(pos);

                    } else {

                        // 双击判定（原实现在同一帧直接发 DoubleClick，不发 Down）。

                        let now = Instant::now();

                        let double = this.last_down.is_some_and(|(at, p)| {

                            now.duration_since(at) <= Duration::from_millis(DOUBLE_CLICK_MS) && (pos - p).length() <= DOUBLE_CLICK_SLOP

                        });

                        this.last_down = Some((now, pos));

                        if double {

                            this.dragging = false;

                            this.send(PointerKind::DoubleClick, pos, &e.modifiers);

                        } else {

                            this.dragging = true;

                            this.send(PointerKind::Down, pos, &e.modifiers);

                        }

                    }

                    cx.notify();

                }),

            )

            // 移动：拖拽进度已统一由窗口级 `on_window_drag_move` 接管（见 `WindowDragTarget`

            // 实现）；这里只做 hover 命中，拖拽进行中由 `any_window_drag_active()` 提前跳过。

            .on_mouse_move(cx.listener(|this: &mut CanvasView, e: &MouseMoveEvent, _window, cx| {

                // A window-header drag must not run the canvas hover hit-testing (guide lines,

                // selection spacing, handles, pointer send). Those walks are the expensive part

                // of every move event and made dragging the popover feel sticky.

                if this.popup_dragging() {

                    return;

                }

                // 拖左右面板分隔线时，指针就在画布边缘上：画布每次 move 都跑一遍

                // hover 命中（guide / spacing / handles / send Hover）纯属白干，还会

                // 让拖宽发涩。resize 期间直接跳过画布的 hover 处理。

                if this.resizing.is_some() {

                    return;

                }

                this.mods = e.modifiers;

                let pos = this.local(e.position);

                this.pointer = Some(pos);

                // 控件树拖动中：用全局指针重算落点（行间 / 列表外也能更新），跳过画布自身的

                // Hover / 选中派发。几何由通用件的 `calibrate` 从命中行矩形重建。

                if this.layer_tree.drag.is_some() {

                    this.layer_on_global_move(e.position, cx);

                    return;

                }

                if let Some((p0, cx0, cy0)) = this.pan {

                    if let Ok(ds) = this.app.session.ds_mut() {

                        let z = ds.view.zoom.max(1e-6);

                        ds.view.cx = cx0 - f64::from(pos.x - p0.x) / z;

                        ds.view.cy = cy0 - f64::from(pos.y - p0.y) / z;

                        ds.revision += 1;

                    }

                    cx.notify();

                    return;

                }

                if this.zoom_drag.is_some() {

                    cx.notify();

                    return;

                }

                if !this.dragging && this.guide_input(pos, false, false, e.modifiers.alt) {

                    cx.notify();

                    return;

                }

                if !this.dragging

                    && canvas::selection_spacing::input(

                        &mut this.app,

                        Some(pos),

                        true,

                        this.space,

                        false,

                        this.primary_down,

                        false,

                        e.modifiers.platform || e.modifiers.control,

                        e.modifiers.alt,

                        e.modifiers.shift,

                    )

                {

                    cx.notify();

                    return;

                }

                if canvas::motion_overlay::input(&mut this.app, pos, false, this.primary_down, false, e.modifiers.control) {

                    cx.notify();

                    return;

                }

                if canvas::handles::input(&mut this.app, pos, false, this.primary_down, false, e.modifiers.shift) {

                    cx.notify();

                    return;

                }

                let kind = if this.dragging { PointerKind::Move } else { PointerKind::Hover };

                this.send(kind, pos, &e.modifiers);

                cx.notify();

            }))

            // 抬起：画布级拖拽收尾（pan / zoom / 框选 / 变换手柄 / Move）已统一由窗口级

            // `on_window_drag_up` 接管，指针移出画布 / 窗口也不中断，故此处只做兜底复位。

            .on_mouse_up(

                MouseButton::Left,

                cx.listener(|this: &mut CanvasView, e: &MouseUpEvent, _window, cx| {

                    this.mods = e.modifiers;

                    let pos = this.local(e.position);

                    this.pointer = Some(pos);

                    this.primary_down = false;

                    // 控件树拖动中：提交落点（或把按下候选当作选中），不再跑画布自身的 up 逻辑。

                    if this.layer_tree.drag.is_some() {

                        this.layer_drag_up(cx);

                        cx.notify();

                        return;

                    }

                    if this.pan.take().is_some() {

                        cx.notify();

                        return;

                    }

                    if let Some(start) = this.zoom_drag.take() {

                        this.finish_zoom(start, pos, e.modifiers.alt);

                        cx.notify();

                        return;

                    }

                    if this.app.selection_spacing.is_dragging() {

                        let _ = canvas::selection_spacing::input(

                            &mut this.app,

                            Some(pos),

                            true,

                            this.space,

                            false,

                            false,

                            true,

                            e.modifiers.platform || e.modifiers.control,

                            e.modifiers.alt,

                            e.modifiers.shift,

                        );

                        cx.notify();

                        return;

                    }

                    if this.app.motion_gizmo.drag.is_some() {

                        let _ = canvas::motion_overlay::input(&mut this.app, pos, false, false, true, false);

                        cx.notify();

                        return;

                    }

                    if this.app.handles.drag.is_some() {

                        let _ = canvas::handles::input(&mut this.app, pos, false, false, true, e.modifiers.shift);

                        cx.notify();

                        return;

                    }

                    if !this.dragging && this.guide_input(pos, false, true, e.modifiers.alt) {

                        cx.notify();

                        return;

                    }

                    if this.dragging {

                        this.dragging = false;

                        this.send(PointerKind::Up, pos, &e.modifiers);

                    }

                    cx.notify();

                }),

            )

            .on_scroll_wheel(cx.listener(|this: &mut CanvasView, e: &ScrollWheelEvent, _window, cx| {

                this.mods = e.modifiers;

                this.on_wheel(e);

                cx.notify();

            }))

            // 右键：Ctrl/Cmd + 右键 = 平移画布；否则打开画布上下文菜单

            //（`ui-egui` 的 `open_context_menu`）。用户 2026-11-08：Ctrl+右键拖不动。

            .on_mouse_down(

                MouseButton::Right,

                cx.listener(|this: &mut CanvasView, e: &MouseDownEvent, window, cx| {

                    let pos = this.local(e.position);

                    this.pointer = Some(pos);

                    this.commit_edits(window, cx);

                    if e.modifiers.control || e.modifiers.platform {

                        if let Ok(ds) = this.app.session.ds() {

                            this.pan = Some((pos, ds.view.cx, ds.view.cy));

                        }

                        this.right_pan = true;

                        cx.notify();

                        cx.stop_propagation();

                        return;

                    }

                    this.focus.focus(window);

                    this.open_context_menu(pos, cx);

                    // 不许再走画布命中：右键这一下不属于工具的按下。

                    cx.stop_propagation();

                }),

            )

            .on_mouse_up(

                MouseButton::Right,

                cx.listener(|this: &mut CanvasView, _e: &MouseUpEvent, _window, cx| {

                    if this.pan.take().is_some() {

                        this.right_pan = false;

                        cx.notify();

                    }

                }),

            )

            // 中键平移（原实现同款）。

            .on_mouse_down(

                MouseButton::Middle,

                cx.listener(|this: &mut CanvasView, e: &MouseDownEvent, _window, cx| {

                    let pos = this.local(e.position);

                    this.pointer = Some(pos);

                    if let Ok(ds) = this.app.session.ds() {

                        this.pan = Some((pos, ds.view.cx, ds.view.cy));

                    }

                    this.right_pan = false;

                    cx.notify();

                }),

            )

            .on_mouse_up(

                MouseButton::Middle,

                cx.listener(|this: &mut CanvasView, _e: &MouseUpEvent, _window, cx| {

                    this.pan = None;

                    this.right_pan = false;

                    cx.notify();

                }),

            )

            // 键盘：空格 = 平移工具，Z = 缩放工具（Alt 缩小）。

            .on_key_down(cx.listener(|this: &mut CanvasView, e: &KeyDownEvent, _window, cx| {

                this.mods = e.keystroke.modifiers;

                // R4/R5: Escape cancels an inline rename (egui `Rename` keeps the old name);

                // it must not reach the engine or leave a late focus-out commit behind.

                if e.keystroke.key.as_str() == "escape" && this.inline_rename.is_some() {

                    this.inline_rename = None;

                    this.inline_rename_pending.borrow_mut().take();

                    this.rename_cancel = true;

                    cx.notify();

                    return;

                }

                // ── Motion 时间轴快捷键（egui timeline.rs:1093-1158）──

                // Space 播放(仅指针在时间轴)、,/. 逐帧、Home/End 跳首尾、

                // Delete/Backspace 删选中的关键帧/片段并吞掉键，避免全局删掉整个图层。

                if this.motion && !this.app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false) {

                    let key = e.keystroke.key.as_str();

                    let settings = this.app.session.ds().ok()

                        .map(|ds| {
                            let sel = this.app.session.selection();
                            let scope = crate::motion::scope_of(&ds.doc, &sel, ds.page);
                            crate::motion::settings_of(&ds.doc, scope)
                        })

                        .unwrap_or_default();

                    let step = 1000.0 / settings.fps.max(1) as f64;

                    let handled = match key {

                        "space" if crate::motion::timeline::pointer_in_bar() => {

                            this.motion_ui.playing = !this.motion_ui.playing;

                            this.motion_ui.last_tick = None;

                            if this.motion_ui.playing {

                                this.motion_ui.clock_ms = this.motion_ui.playhead_ms;

                            }

                            true

                        }

                        "comma" => { this.motion_ui.playhead_ms = (this.motion_ui.playhead_ms - step).max(0.0); true }

                        "period" => { this.motion_ui.playhead_ms += step; true }

                        "home" => { this.motion_ui.playhead_ms = 0.0; true }

                        "end" => { this.motion_ui.playhead_ms = settings.duration_ms as f64; true }

                        "delete" | "backspace" if !this.motion_ui.selection.is_empty() => {

                            let sel = this.motion_ui.selection.clone();

                            for item in sel {

                                match item {

                                    crate::motion::SelItem::Key(n, p, t) | crate::motion::SelItem::Segment(n, p, t) => {

                                        crate::motion::remove_key(&mut this.app, n, p, t);

                                    }

                                    crate::motion::SelItem::Clip(n, c) => crate::motion::remove_clip(&mut this.app, n, c),

                                    _ => {}

                                }

                            }

                            let _ = crate::motion::apply(&mut this.app, &mut this.motion_ui, vec![crate::motion::Act::ClearSelection]);

                            cx.stop_propagation();

                            true

                        }

                        _ => false,

                    };

                    if handled {

                        cx.notify();

                        return;

                    }

                }

                // 变量编辑器优先：Escape 先收内部菜单/改名/拖拽（没有内部状态才让通用 Escape

                // 关窗口），Ctrl/Cmd+C/V 复制粘贴整行。它独立于 `popup`，单独取实体。

                if this.variables_open {

                    if let Some(w) = &this.variables_editor {

                        let m = e.keystroke.modifiers;

                        if w.variables_key(&e.keystroke.key, m.platform || m.control) {

                            cx.notify();

                            return;

                        }

                    }

                }

                // B2: the Actions palette's Up/Down move the highlighted row; Enter (the search
                // field's commit) runs it.
                if matches!(&this.popup, Some(Popup::Palette { .. })) {
                    let key = e.keystroke.key.as_str();
                    if key == "up" || key == "down" {
                        let count = palette::items(&palette::sections(&this.palette_query, this.palette_tab, &this.app.session)).len();
                        if count > 0 {
                            let cur = this.palette_sel.unwrap_or(0);
                            this.palette_sel = Some(if key == "up" { cur.saturating_sub(1) } else { (cur + 1).min(count - 1) });
                            cx.notify();
                        }
                        return;
                    }
                }

                // 通用拖拽：Escape 取消（egui `ListDrag::with_order` 的语义），既不当成点击选中，

                // 也不把这一次按键送给引擎。

                if e.keystroke.key == "escape" && this.layer_cancel_drag(cx) {

                    return;

                }

                // 演示层：Escape 退出（egui present 的 Esc 语义）。

                if e.keystroke.key == "escape" && this.present.is_some() {

                    this.present = None;

                    cx.notify();

                    return;

                }

                // A19：通用对话框的 Escape 与 X / 遮罩一致，都是「只关窗、不改文档」。

                if e.keystroke.key == "escape" && this.dialog.is_some() {

                    this.dialog = None;

                    cx.notify();

                    return;

                }

                // 四块整面板：Escape 关掉浮层（egui 的 window/dialog 同款）。

                if e.keystroke.key == "escape" && this.extra.is_some() {

                    this.extra = None;

                    cx.notify();

                    return;

                }

                // 变量编辑器：Escape 关闭（保留实体与 Full/Floating 记忆）。

                if e.keystroke.key == "escape" && this.variables_open {

                    this.close_variables(cx);

                    return;

                }

                // 通用窗口弹层：Escape 只关窗口（egui `popover::show` 的 Escape 语义），

                // 不把这一次按键再送给引擎或文字编辑。

                if e.keystroke.key == "escape" && matches!(&this.popup, Some(Popup::Window(_))) {

                    this.close_popup(cx);

                    cx.notify();

                    return;

                }

                // 右键菜单 / 飞出菜单 / Actions：Escape 关掉它们，而不是把这次按键送给引擎

                // （否则会先 `select.none` 清掉选区，egui 那边 Escape 只关菜单）。

                if e.keystroke.key == "escape"

                    && matches!(

                        &this.popup,

                        Some(

                            Popup::Context { .. }

                                | Popup::Flyout { .. }

                                | Popup::Options { .. }

                                | Popup::Templates

                                | Popup::Palette { .. }

                                | Popup::Menu { .. }

                                | Popup::Share { .. }

                        )

                    )

                {

                    this.popup = None;

                    this.tool_flyout = None;

                    cx.notify();

                    return;

                }

                // 正在编辑文字：按键给引擎的文本编辑（照搬 `shortcuts.rs::text_keys` 的按键名）。

                if this.app.session.ds().is_ok_and(|d| d.text_edit.is_some()) {

                    let m = e.keystroke.modifiers;

                    // Text edit: Cmd/Ctrl+Z(-Shift) ends the edit, then undo/redo.

                    if (m.platform || m.control) && e.keystroke.key.as_str() == "z" {

                        let _ = figcraft_engine::textedit::end(&mut this.app.session);

                        let id = if m.shift { "edit.redo" } else { "edit.undo" };

                        let _ = this.app.session.execute(id, &serde_json::json!({}));

                        cx.notify();

                        return;

                    }

                    let word = m.control || m.alt;

                    let line = m.platform || m.control;

                    let named = match e.keystroke.key.as_str() {

                        "backspace" => Some("Backspace"),

                        "delete" => Some("Delete"),

                        "enter" => Some("Enter"),

                        "escape" => Some("Escape"),

                        "tab" => Some("Tab"),

                        "left" => Some("Left"),

                        "right" => Some("Right"),

                        "up" => Some("Up"),

                        "down" => Some("Down"),

                        "home" => Some("Home"),

                        "end" => Some("End"),

                        _ => None,

                    };

                    if let Some(name) = named {

                        let _ = figcraft_engine::textedit::key(&mut this.app.session, name, m.shift, word, line);

                    } else if !m.platform

                        && !m.control

                        && let Some(c) = e.keystroke.key_char.clone()

                    {

                        // 正在 IME 组合（preedit 已建立）时，文字由 `EntityInputHandler` 接管，

                        // 这里不再插字，避免重复（见 `impl EntityInputHandler for CanvasView`）。

                        let composing = this.app.session.ds().ok().and_then(|d| d.text_edit.as_ref()).is_some_and(|te| te.preedit.is_some());

                        if !composing {

                            let _ = figcraft_engine::textedit::insert(&mut this.app.session, &c);

                        }

                    }

                    cx.notify();

                    return;

                }

                // 命令快捷键：键名规范化后查引擎的 keymap（和原文 `shortcuts.rs` 同一条路）。

                // Ctrl/Cmd+Z、Shift+Ctrl+Z、Delete、Ctrl+C/V/D、Ctrl+A……都在这张表里。

                let held = figcraft_engine::keymap::Mods {

                    shift: e.keystroke.modifiers.shift,

                    alt: e.keystroke.modifiers.alt,

                    cmd: e.keystroke.modifiers.platform,

                    ctrl: e.keystroke.modifiers.control,

                };

                let key = canonical_key(&e.keystroke.key);

                let binding = figcraft_engine::keymap::lookup(

                    figcraft_engine::keymap::Platform::current(),

                    figcraft_engine::keymap::Context::Canvas,

                    &key,

                    held,

                )

                .map(|b| b.id);

                if let Some(id) = binding

                    && run_binding(&mut this.app, id)

                {

                    cx.notify();

                    return;

                }

                match e.keystroke.key.as_str() {

                    "space" => this.space = true,

                    "z" if !e.keystroke.modifiers.platform => this.zoom_key = true,

                    _ => {}

                }

                cx.notify();

            }))

            .on_key_up(cx.listener(|this: &mut CanvasView, e: &KeyUpEvent, _window, cx| {

                this.mods = e.keystroke.modifiers;

                match e.keystroke.key.as_str() {

                    "space" => this.space = false,

                    "z" => {

                        this.zoom_key = false;

                        // 松开 Z 时如果还拖着缩放框，就当点击收尾。

                        if let Some(start) = this.zoom_drag.take() {

                            let end = this.pointer.unwrap_or(start);

                            this.finish_zoom(start, end, this.mods.alt);

                        }

                    }

                    _ => {}

                }

                cx.notify();

            }));



        // `FC_DEBUG_LAYOUT=1`：给每个区域描一圈彩色边，方便肉眼核对布局边界。

        let dbg = crate::widgets::debug_layout();



        // `panel_data` 已在帧首算好（拖动时复用上一帧），这里直接沿用，不再重复读文档。



        // ── 主菜单入口：左侧竖栏第 1 项（`Icon::Menu`）点击后，在它下方弹出下拉主菜单。

        //    不是常驻横条——菜单只在这一下点击之后出现。

        //    锚点 = 那颗按钮的**真实矩形**：`menu` 是「锚点左下角 + gap(4)」，

        //    所以菜单顶边正好落在按钮底边往下 4pt。──

        let me_menu = cx.entity();

        let menu_anchor = panels::rail_menu_bounds();

        // 第 1 次打开，第 2 次关闭（用户 2026-11-06）。backdrop 已放行 rail 按钮区域，

        // 所以第二次点击一定能到这里，而不会被全屏遮罩先 dismiss 掉。

        let on_menu = move |_e: &gpui::ClickEvent, _w: &mut Window, cx: &mut App| {

            me_menu.update(cx, |cv, cx| {

                if matches!(cv.popup, Some(Popup::MainMenu(_))) {

                    cv.close_popup(cx);

                } else {

                    cv.open_main_menu(menu_anchor, cx);

                }

            });

        };



        // 左栏 rail 第 4 项 = Variables（egui `panels/layers.rs` 的 `Icon::Variables` 按钮）。

        // 点它打开变量编辑器窗口；当前该窗口只承载「右边的列表」。锚点只取 y（`anchored_open`

        // 会把 x 改成 Design 面板左缘），所以用鼠标位置的高度即可。

        let me_vars = cx.entity();

        // 已经是变量窗口时再点按钮 = 关闭，绝不二次弹出（否则会重建 WindowState，

        // 丢掉拖动后的位置 / 选中行）。用户 2026-11-06。

        let on_variables = move |_e: &gpui::ClickEvent, w: &mut Window, cx: &mut App| {

            let y = w.mouse_position().y;

            me_vars.update(cx, |cv, cx| {

                // ⛔ 已打开就**什么都不做**：浮动小窗点出来就点出来，再点一下不关闭

                // （用户 2026-11-08 明确要求）。关闭走小窗的 X 或左栏 File。

                if cv.variables_open {

                    return;

                }

                if cv.variables_editor.is_some() {

                    // 复用实体：egui 的 Editor 常驻 `app.ui.variables`，不会二次重建。

                    let data = cv.panel_data();

                    if let Some(win) = &mut cv.variables_editor {

                        win.set_variables_full(cv.variables_full);

                        win.set_variables_data(data.variables);

                    }

                    cv.variables_open = true;

                    cx.notify();

                    return;

                }

                let data = cv.panel_data();

                let win = popovers::WindowState::variables_table(

                    cv.fields.queue(),

                    Point { x: px(0.0), y },

                    data.variables,

                );

                // 不再强制 `variables_full = false`：会话记住上次是 Full 还是 Floating。

                cv.open_window(win, cx);

            });

        };



        // 左栏 rail 第 2 项 = File。egui `layers.rs`: tab 点击会 `app.ui.variables = None`，

        // 所以 Full 时点 File 能「切换回去」（退出变量编辑器，回到普通视图）。

        let me_file = cx.entity();

        let on_file = move |_e: &gpui::ClickEvent, _w: &mut Window, cx: &mut App| {

            me_file.update(cx, |cv, cx| {

                if cv.variables_open {

                    cv.close_variables(cx);

                }

                // 点 File tab：回到图层视图（R1）。
                cv.left_tab = 0;

            });

        };

        // 左栏 rail 第 3 项 = Assets。egui `layers.rs` 的 `LeftTab::Assets` 分支：点它切到左面板
        // 的 Assets tab（R1），不再弹居中浮层。
        let me_assets = cx.entity();

        let on_assets = move |_e: &gpui::ClickEvent, _w: &mut Window, cx: &mut App| {

            me_assets.update(cx, |cv, cx| {

                if cv.left_tab == 1 {

                    cv.left_tab = 0; // 再次点击收起回图层

                } else {

                    cv.left_tab = 1;

                }

                cv.extra = None;

                cx.notify();

            });

        };




        // 外壳：顶部行（导轨 | 左面板 | 手柄 | 画布 | 手柄 | 右面板 | 悬浮 pill）+ 底部动画栏。

        // 底部工具栏（`toolbar`）：工具组 + 飞出菜单、Actions、模式切换。放最后 → GPUI 按

        // children 顺序绘制，天然盖在左右栏之上，所以窄窗口里它浮在面板之上而不被裁掉。

        let on_intent = {

            let me = cx.entity();

            let cb: std::rc::Rc<dyn Fn(toolbar::Intent, &mut Window, &mut App)> = std::rc::Rc::new(move |intent, w, cx| {

                me.update(cx, |cv, cx| cv.on_intent(intent, w, cx));

            });

            cb

        };

        let bar = toolbar::bar(

            self.app.session.tool.tool,

            &self.tool_last,

            self.tool_flyout,

            matches!(self.popup, Some(Popup::Palette { .. })),

            self.motion,

            self.fields.right_tab() == 2,

            self.app.session.active_read_only(),

            canvas_x,

            canvas_w,

            on_intent,

            board,

            self.board_panel,

        );

        // Motion 打开时右栏换成 Motion inspector（宽 413）；否则仍是 Design 面板。

        // 两者都返回 `Stateful<Div>`，宽度由同一个 `right_w` 决定。

        // egui 里 Motion 只占「标签行以下」：48 pt 顶栏与 Design/Prototype 标签**仍然在**，

        // 所以这里把同样两行摆在 Motion inspector 上面，而不是整块换掉。

        let right_el = if self.motion {

            let head = div()

                .flex_none()

                .w_full()

                .flex()

                .flex_col()

                .child(panels::right::topbar::top_bar(&self.fields, &panel_data))

                .child(panels::right::sections::view_bar(&self.fields, &panel_data, true));

            let t = crate::widgets::tokens();

            let right_hot = crate::resize_drag::hovered(crate::resize_drag::Edge::Right)
                || self.resizing == Some(ResizeSide::Right);

            div()

                .id("right-motion")

                .flex_none()

                .h_full()

                .flex()

                .flex_col()

                .bg(crate::widgets::rgba(t.panel))

                // 与画布之间那条唯一的 1px 分隔线 = 容器自身左缘描边（与底栏 border_t_1、
                // 左面板 border_r_1 同一套做法，不再有独立 drag_handle 元素）。宽度 = right_w +
                // HANDLE_W 吸收掉被删的 1px 占位，canvas 几何 / 命中不变。只画左缘一条，hover
                // 或拖拽中变强调色；绝不加四边 border_1（那会与分隔条并排成双线 + 内缩伪影）。
                .border_l_1()

                .border_color(crate::widgets::rgba(if right_hot { t.accent } else { t.border }))

                .child(head)

                .child(self.render_motion_inspector(window, cx).flex_1().min_h(px(0.0)))

                .w(px(self.right_w + panels::HANDLE_W))

        } else {

            // 普通右面板同理：宽度 +HANDLE_W 吸收被删的 1px 占位，容器自身左缘画那条线。
            // 注意 right_panel 内部还有 view_bar 的 border_b 等**内容**分隔线，那些不是容器
            // 外框、保持不动；这里只在外层 wrap 上加左缘描边。
            let t = crate::widgets::tokens();

            let right_hot = crate::resize_drag::hovered(crate::resize_drag::Edge::Right)
                || self.resizing == Some(ResizeSide::Right);

            panels::right_panel(&self.fields, self.panel_sel(), &panel_data, self.right_w, window, cx, self.motion)

                .border_l_1()

                .border_color(crate::widgets::rgba(if right_hot { t.accent } else { t.border }))

                .w(px(self.right_w + panels::HANDLE_W))

        };

        // ── 系统级窗口标题区域（kit `TitleBar::new().child(TabBar)`）────────────────────
        // 标签条塞在标题栏里，不是画布里的一条；标题栏是 Root 的第一个 child，
        // 客户区（Dock）从 y = TITLE_H 起，几何由 dock::DockLayout 统一推导。
        let tab_docs = tabs_view::read_docs(&self.app.session);
        let tabs_active = self.app.session.active;
        let tabs_dark = self.app.theme.dark;
        let tabs_reduce_motion = self.app.session.prefs.reduce_motion;
        let viewport_w = f32::from(vp.width);
        let content_w = title_bar::content_width(viewport_w);
        let tabs_cmds = self.fields.queue();
        let tabs_me = cx.entity();
        let tab_bar = title_bar::title_bar(
            tabs_view::tab_strip(
                &mut self.tabs,
                &tab_docs,
                tabs_active,
                tabs_dark,
                tabs_reduce_motion,
                content_w,
                &tabs_cmds,
                &tabs_me,
            ),
            window.is_maximized(),
        );



        // 新标签的 0.18s 入场动画期间要连续重绘，直到 opening 队列清空。

        if !self.tabs.motion.opening().is_empty() {

            cx.notify();

        }



        let top_row = div()

            .flex()

            .flex_row()

            .flex_1()

            .min_w(px(0.0))

            // flex 项默认 `min-height: auto`：右栏的 section 内容比画布高时，会把这一行

            // 撑到内容高度，画布（`h_full`）跟着变高、把底部动画栏顶出窗口。

            .min_h(px(0.0))

            .overflow_hidden()

            .w_full()

            .relative()

            // 标签页头部栏（系统 TitleBar，y ∈ [0, TITLE_H)）与下方工作区之间的 1pt 分隔线
            // （用户 2026-11-12）。用 `border_t` 而非插入一个 h(1) child：border 在盒子内，
            // 不会把 top_row 下推 1pt 打乱 Dock 几何（`canvas_origin().y = TITLE_H`，命中/浮层
            // 坐标都依赖它）。隐藏 / 收起 UI 时没有标签栏，这条线也不画。
            .when(!ui_hidden && !ui_minimized, |d| d.border_t_1().border_color(crate::widgets::rgba(crate::widgets::tokens().border)))

            // B9：隐藏 UI 时左右侧栏、工具栏、分隔条命中区全部让位给画布；
            // 收起界面（minimize）时保留工具栏，右栏只在有选区时出现（egui `view.minimize` 语义）。
            .when(show_sidebars, |d| d
                .child(dbg_tint(panels::left_rail(on_menu, on_variables, on_file, on_assets, self.left_tab, self.variables_open, self.variables_full), dbg, 0xE5484D))

                .when(!self.left_collapsed, |d| {

                    d.child(dbg_tint(self.render_left_panel(window, cx).w(px(self.left_w + panels::HANDLE_W)), dbg, 0xF5A524))

                }))

            .child(dbg_tint(canvas, dbg, 0x30A46C))

            .when(show_right, |d| d.child(dbg_tint(right_el, dbg, 0x0091FF)))

            .when(!ui_hidden, |d| d.child(bar))

            // 4px 命中覆盖条必须最后绘制：后绘制的 hitbox 先命中，才压得住画布 / 面板。
            .when(show_sidebars, |d| d
                .child(self.divider_hotspot(ResizeSide::Left, viewport_w, cx))
                .child(self.divider_hotspot(ResizeSide::Right, viewport_w, cx)));



        // 弹出层画在最上层：backdrop + 弹层作为 root 的最后 children。root 设为 `relative`，

        // 使 `absolute()` 子节点以窗口原点（0,0）为基准，正好对应 `open_*` 收到的窗口坐标。

        let mut popup_el: Vec<AnyElement> = Vec::new();

        // 先把 Actions 面板要用的两个字段读出来：下面 `&mut self.popup` 会把 self 整体借走。

        let palette_field = self.palette_field.clone();

        let palette_query = self.palette_query.clone();

        // B2: sections are computed here, before `&mut self.popup` borrows all of `self`.

        let palette_tab = self.palette_tab;

        let palette_sections = palette::sections(&palette_query, palette_tab, &self.app.session);

        let palette_items = palette::items(&palette_sections);

        let palette_sel = self.palette_sel;

        let palette_count = palette_sections.iter().map(|s| s.rows.len()).sum::<usize>();

        // Share 弹层同理：live 快照与命令队列也要在 `&mut self.popup` 之前取好。

        let share_live = panels::right::data::LiveData::read(&self.app.session);

        let share_cmds = self.fields.queue();

        // 画板工具组的选项菜单按这份快照画「当前选中的那一格」；模板预览图也从这里取。

        let board_tool = self.app.session.tool.board.clone();

        let tpl_pics = self.tpl_pics.clone();

        if let Some(p) = &mut self.popup {

            let me = cx.entity();

            let dismiss = std::rc::Rc::new({

                let me = me.clone();

                move |_window: &mut Window, cx: &mut App| me.update(cx, |cv, cx| cv.close_popup(cx))

            });

            // 通用窗口的标题栏按下：只**记录**按下点，不拖动、也不重绘（`cx.notify()` 会让按钮的

            // `ElementId` 在按下/抬起之间变化，`on_click` 就对不上、表现为「按钮点了没反应」）。

            // 真正开始拖动在 `move_popup_drag`：指针移出 3pt 死区后才 latch（egui `click_and_drag`）。

            let drag_start = std::rc::Rc::new({

                let me = me.clone();

                move |e: &MouseDownEvent, _window: &mut Window, cx: &mut App| {

                    me.update(cx, |cv, _cx| {

                        cv.begin_popup_drag(e.position);

                    });

                }

            });

            let drag_move = std::rc::Rc::new({

                let me = me.clone();

                move |e: &MouseMoveEvent, _window: &mut Window, cx: &mut App| {

                    // 卡片自身的 on_mouse_move 也要 notify：一旦根视图那层因为某种原因没接到

                    // move（如 cursor_grab 只把事件送到卡片），这里仍能驱动重绘，窗口才跟手。

                    let held = e.pressed_button == Some(MouseButton::Left);

                    me.update(cx, |cv, cx| {

                        // 没按住左键的移动只能「收尾」，绝不能继续拖（用户 2026-11-08）。

                        if cv.cancel_drag_if_not_held(held) || cv.move_popup_drag(e.position, held) {

                            cx.notify();

                        }

                    });

                }

            });

            let vp = window.viewport_size();

            let vw = f32::from(vp.width);

            let vh = f32::from(vp.height);

            let m = 8.0f32;

            // Canvas geometry (read once, at the current size) so every popup recomputes its position

            // on each render and follows the canvas as the window resizes / panels change

            // (`用户 2026-11-06`：缩放窗口时飞出菜单、主菜单、右键菜单、Actions 面板都要跟着动).

            let canvas_left = dock.center_x();

            let canvas_w = self.app.canvas.rect.width();

            let canvas_h = self.app.canvas.rect.height();

            let bar_w = toolbar::width(toolbar::groups(board), board);

            // 画布原点 y = 标题栏高度；弹层挂在 root 上用窗口坐标，必须加回这个原点，
            // 否则 flyout / palette 整体上移 40pt。
            let pill_top = dock.titlebar_h + canvas_h - 12.0 - 48.0;

            // The Design panel's left edge: the one x every detached window opens against.

            let right_edge = px(vw - self.right_w);

            // `ui-egui/src/popover.rs` Hold rule: the panel that opened the popover is held out,

            // so pressing another swatch / field there keeps editing instead of closing it.

            let backdrop = if matches!(p, Popup::Window(_)) {

                let d = dismiss.clone();

                div()

                    .absolute()

                    // 结构性：遮罩从 rail 右缘开始，绝不覆盖左栏。否则 GPUI 把 mouse-down 只投递给

                    // 最上层命中的 hitbox，rail 的按钮收不到按下/抬起，`on_click` 永远不触发 ——

                    // 表现为「变量面板 Full 时点 File 切不回去」「点主菜单行为异常」。

                    .left(px(panels::RAIL_W))

                    .size_full()

                    .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {

                        let x = f32::from(e.position.x);

                        let y = f32::from(e.position.y);

                        if x < f32::from(right_edge) && !panels::rail_button_hit(x, y) {

                            d(window, cx);

                        }

                    })

                    .into_any_element()

            } else if matches!(p, Popup::Context { .. }) {

                // Right-clicking somewhere else moves the context menu there, like the original

                // (`open_context_menu` runs again on the new layer); a left press just closes it.

                let d = dismiss.clone();

                let me2 = me.clone();

                div()

                    .absolute()

                    .size_full()

                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| d(window, cx))

                    .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _window, cx| {

                        let at = e.position;

                    me2.update(cx, |cv, cx| {

                        let local = cv.local(at);

                        cv.open_context_menu(local, cx);

                    });

                    })

                    .into_any_element()

            } else if matches!(p, Popup::MainMenu(_)) {

                // 主菜单同样放行 rail 按钮：第 2 次点 Menu 要由 on_menu 关闭，

                // 否则全屏遮罩会吞掉这次点击，看起来像「关不掉 / 又打开」。

                let d = dismiss.clone();

                div()

                    .absolute()

                    .left(px(panels::RAIL_W))

                    .size_full()

                    .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {

                        let x = f32::from(e.position.x);

                        let y = f32::from(e.position.y);

                        if !panels::rail_button_hit(x, y) {

                            d(window, cx);

                        }

                    })

                    .into_any_element()

            } else {

                popup_backdrop(dismiss.clone())

            };

            let mut els: Vec<AnyElement> = Vec::new();

            match p {

                Popup::Menu { anchor, width, rows, on_pick, probe } => {

                    // 每帧取触发按钮的当前矩形（滚动 / resize / 弹窗拖动后仍跟着按钮走）。

                    let live = probe.as_ref().and_then(|p| p.get()).unwrap_or(*anchor);

                    // 宽度下限 = 按钮真实宽度（旧代码把调用方传的 0 → 120 当菜单宽，

                    // 而按钮是网格铺满的 160/200，菜单与下拉框不齐）。

                    let w = f32::from(live.size.width).max(width.unwrap_or(0.0)).max(48.0);

                    // 位置与防溢出全部交给通用 `Menu`：它自带「按钮底 + 4」、上下防溢出、

                    // 内部滚动、水平贴边。以前在这里手工夹 x/y 用的是估计高度，

                    // 漏了分隔符和 8pt 内边距，项多 / 贴屏幕底部时照样溢出。

                    els.push(menu_popup_in(live, Some(w), rows.clone(), on_pick.clone(), dismiss.clone(), vw, vh));

                }

                Popup::MainMenu(mm) => {

                    // Re-anchor to the rail button from the *current* layout every render, so the

                    // menu tracks it if the window / panels change (`用户 2026-11-06`).

                    mm.anchor = panels::rail_menu_bounds();

                    let hover = {

                        let me = me.clone();

                        std::rc::Rc::new(move |level: usize, row: usize, w: &mut Window, cx: &mut App| {

                            let p = w.mouse_position();

                            me.update(cx, |cv, cx| cv.hover_menu(level, row, p, cx));

                        })

                    };

                    let click = {

                        let me = me.clone();

                        std::rc::Rc::new(move |level: usize, row: usize, w: &mut Window, cx: &mut App| {

                            let p = w.mouse_position();

                            me.update(cx, |cv, cx| cv.click_menu(level, row, p, cx));

                        })

                    };

                    els.extend(main_menu::render(mm, vw, vh, hover, click));

                }

                Popup::Window(w) => els.extend(w.render(cx, vp, dismiss.clone(), drag_start, drag_move.clone(), right_edge)),

                Popup::Share { anchor } => {

                    // 锚在 Share 按钮的左下角，贴边夹紧（同 Popup::Menu 的做法）。

                    let w = 300.0_f32;

                    let h = 220.0_f32;

                    let x = px(f32::from(anchor.origin.x).min(vw - w - m).max(m));

                    let y = px(f32::from(anchor.origin.y).min(vh - h - m).max(m));

                    let clamped = Bounds { origin: Point { x, y }, size: anchor.size };

                    let card = panels::right::topbar::share_card(clamped, &share_live, &share_cmds, dismiss.clone());

                    els.push(card);

                }

                Popup::Context { pos, items, open_sub, .. } => {

                    // Clamp against the window's right/bottom edge (egui's `constrain(true)`). The

                    // point is canvas-local; convert to window coords from the *current* canvas

                    // offset so the menu tracks the canvas on resize / panel-width change.

                    let w = context_menu::WIDTH;

                    let h = menu_card::height(items, context_menu::ROW_H);

                    let wx = f32::from(pos.x) + canvas_left;

                    let wy = f32::from(pos.y);

                    let x = wx.min((vw - w - m).max(m)).max(m);

                    let y = wy.min((vh - h - m).max(m)).max(m);

                    let pick = {

                        let me = me.clone();

                        std::rc::Rc::new(move |id: &str, _w: &mut Window, cx: &mut App| {

                            me.update(cx, |cv, cx| cv.pick_context(id, cx));

                        })

                    };

                    // 点击子菜单行：直接展开（悬停通常已经开好，这里只兜底）。

                    let on_sub = {

                        let me = me.clone();

                        std::rc::Rc::new(move |row: usize, _w: &mut Window, cx: &mut App| {

                            me.update(cx, |cv, cx| {

                                if let Some(Popup::Context { open_sub, .. }) = &mut cv.popup {

                                    if *open_sub != Some(row) {

                                        *open_sub = Some(row);

                                        cx.notify();

                                    }

                                }

                            });

                        })

                    };

                    // 悬停：交给通用 `MenuAim`，安全三角内经过的行不会抢走已展开的子菜单。

                    let on_hover = {

                        let me = me.clone();

                        std::rc::Rc::new(move |row: usize, win: &mut Window, cx: &mut App| {

                            let p = win.mouse_position();

                            let vp = win.viewport_size();

                            let (vw_, vh_) = (f32::from(vp.width), f32::from(vp.height));

                            me.update(cx, |cv, cx| cv.hover_context(row, p, vw_, vh_, cx));

                        })

                    };

                    els.push(menu_card::card(x, y, menu_card::Width::Min(w), context_menu::ROW_H, items, *open_sub, pick, on_sub, on_hover, window, vh));

                }

                Popup::Flyout { key, items } => {

                    // Geometry is recomputed from the current canvas size every render, so the card

                    // tracks the centred pill as the window resizes (`用户 2026-11-06`). It is **as

                    // wide as its content** — no fixed width; only its height comes from the rows.

                    let pill_left = canvas_left + (canvas_w - bar_w) / 2.0;

                    let caret = toolbar::caret_left(toolbar::groups(board), *key).unwrap_or(0.0);

                    let left = pill_left + caret;

                    let bottom = pill_top + toolbar::CELL_TOP - toolbar::FLYOUT_GAP;

                    let h = menu_card::height(items, toolbar::FLYOUT_ROW_H);

                    let x = left.max(m);

                    let y = (bottom - h).max(m);

                    let pick = {

                        let me = me.clone();

                        std::rc::Rc::new(move |id: &str, _w: &mut Window, cx: &mut App| {

                            me.update(cx, |cv, cx| cv.pick_flyout(id, cx));

                        })

                    };

                    let hover: std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)> = std::rc::Rc::new(|_, _, _| {});

                    els.push(menu_card::card(x, y, menu_card::Width::Fit, toolbar::FLYOUT_ROW_H, items, None, pick, hover, std::rc::Rc::new(|_, _, _| {}), window, vh));

                }

                Popup::Options { key } => {



                    // 一排格子贴在 caret 上方：便签颜色、形状或印章，当前那个有底色



                    // （`ui-egui/src/toolbar.rs::options_flyout`）。



                    if let Some(g) = toolbar::group(key) {



                        let items = crate::board_bar::choices(g.options, &board_tool);



                        let (cell, pad) = (32.0f32, 8.0f32);



                        let w = items.len() as f32 * (cell + 4.0) - 4.0 + 2.0 * pad;



                        let h = cell + 2.0 * pad;



                        let pill_left = canvas_left + (canvas_w - bar_w) / 2.0;



                        let caret = toolbar::caret_left(toolbar::groups(board), key).unwrap_or(0.0);



                        let x = (pill_left + caret).max(m);



                        let y = (pill_top + toolbar::CELL_TOP - toolbar::FLYOUT_GAP - h).max(m);



                        let t = widgets::tokens();



                        let cmd = format!("tool.{}", g.tools.first().map(|x| x.0.id()).unwrap_or("move"));



                        let mut row = div().flex().flex_row().gap(px(4.0)).px(px(pad)).py(px(pad));



                        for (i, c) in items.into_iter().enumerate() {



                            let me = me.clone();



                            let id = cmd.clone();



                            let label: gpui::SharedString = tr(c.label).into();



                            let on = c.on;



                            row = row.child(



                                div()



                                    .id(("bb-opt", i))



                                    .size(px(cell))



                                    .flex()



                                    .items_center()



                                    .justify_center()



                                    .rounded(px(5.0))



                                    .cursor_pointer()



                                    .when(on, |d| d.bg(widgets::rgba(t.menu_hover)))



                                    .hover(|s| s.bg(widgets::rgba(t.menu_hover)))



                                    .tooltip(move |_w, cx| cx.new(|_| toolbar::Tip { text: label.clone() }).into())



                                    .child(crate::board_bar::choice_mark(c.mark, cell - 10.0, widgets::rgba(t.menu_text)))



                                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _w, cx| {



                                        let params = c.params.clone();



                                        let id = id.clone();



                                        me.update(cx, |cv, cx| cv.pick_board_option(&id, params, cx));



                                    }),



                            );



                        }



                        let card = div()



                            .absolute()



                            .left(px(x))



                            .top(px(y))



                            .w(px(w))



                            .h(px(h))



                            .rounded(px(13.0))



                            .bg(widgets::rgba(t.menu))



                            .border_1()



                            .border_color(widgets::rgba(t.border_strong))



                            .shadow(vec![gpui::BoxShadow {



                                color: widgets::rgba(t.shadow).into(),



                                offset: gpui::point(px(0.0), px(3.0)),



                                blur_radius: px(14.0),



                                spread_radius: px(0.0),



                            }])



                            .occlude()



                            .child(row);



                        els.push(card.into_any_element());



                    }



                }



                Popup::Templates => {



                    // 模板选择器：右端对齐工具栏右端，再夹进画布



                    // （`ui-egui/src/widgets/board_picker.rs::picker`）。



                    let (w, h) = crate::board_picker::size();



                    let anchor = canvas_left + (canvas_w + bar_w) / 2.0;



                    let x = (anchor - w).min(canvas_left + canvas_w - w - 8.0).max(canvas_left + 8.0).max(m);



                    let y = (pill_top - 8.0 - h).max(m);



                    let me = me.clone();



                    let on_card: std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)> = std::rc::Rc::new(move |i, _w, cx| {

                        me.update(cx, |cv, cx| cv.pick_template(i, cx));

                    });

                    let card = crate::board_picker::picker(&tpl_pics, on_card);



                    els.push(div().absolute().left(px(x)).top(px(y)).child(card).into_any_element());



                }



                Popup::Palette { at } => {

                    let w = PALETTE_W;

                    let h = PALETTE_H;

                    // Recompute from the *current* pill position so the palette follows on resize.

                    let left = (f32::from(at.x) - PALETTE_W / 2.0).max(8.0);

                    let bottom = pill_top - 8.0;

                    let x = left.min((vw - w - m).max(m)).max(m);

                    let y = (bottom - h).max(m);

                    let t = widgets::tokens();

                    let pick = {

                        let me = me.clone();

                        std::rc::Rc::new(move |key: &str, _w: &mut Window, cx: &mut App| {

                            me.update(cx, |cv, cx| cv.pick_palette(key, cx));

                        })

                    };

                    let hover: std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)> = std::rc::Rc::new(|_, _, _| {});

                    let (pal_rows, _pal_subs) = menu_card::rows(

                        &palette_items,

                        context_menu::ROW_H,

                        false,

                        None,

                        palette_sel,

                        x,

                        y + 40.0,

                        w - 16.0,

                        pick,

                        hover,

                        std::rc::Rc::new(|_, _, _| {}),

                        window,

                        vh,

                    );

                    let list = div()

                        .id("palette-list")

                        .flex_1()

                        .min_h(px(0.0))

                        .w_full()

                        .overflow_y_scroll()

                        .children(if palette_count == 0 {

                            vec![div()

                                .p(widgets::pt(8.0))

                                .text_size(widgets::pt(12.0))

                                .text_color(widgets::rgba(t.menu_dim))

                                .child(tr("No actions match this search."))

                                .into_any_element()]

                        } else {

                            pal_rows

                        });

                    // B2: All / Assets / Plugins & widgets. Fixed panel height, so switching

                    // tabs never resizes it; only the active chip carries a fill.

                    let tabs = {

                        let me = me.clone();

                        div()

                            .flex()

                            .flex_row()

                            .gap(widgets::pt(4.0))

                            .w_full()

                            .children(palette::Tab::ALL.iter().map(|tab| {

                                let tab = *tab;

                                let active = tab == palette_tab;

                                let me = me.clone();

                                let mut chip = div()

                                    .px(widgets::pt(8.0))

                                    .py(widgets::pt(4.0))

                                    .rounded(widgets::pt(6.0))

                                    .text_size(widgets::pt(12.0))

                                    .text_color(widgets::rgba(if active { t.menu_text } else { t.menu_dim }))

                                    .cursor_pointer()

                                    .child(tab.label())

                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {

                                        me.update(cx, |cv, cx| {

                                            cv.palette_tab = tab;

                                            cx.notify();

                                        });

                                    });

                                if active {

                                    chip = chip.bg(widgets::rgba(t.menu_hover));

                                }

                                chip.into_any_element()

                            }))

                    };

                    els.push(

                        div()

                            .absolute()

                            .left(px(x))

                            .top(px(y))

                            .w(px(w))

                            .h(px(h))

                            .flex()

                            .flex_col()

                            .gap(widgets::pt(8.0))

                            .p(widgets::pt(8.0))

                            .bg(widgets::rgba(t.menu))

                            .rounded(widgets::pt(13.0))

                            .border_1()

                            .border_color(widgets::rgba(t.border))

                            .occlude()

                            .child(palette_field.clone())

                            .child(tabs)

                            .child(list)

                            .into_any_element(),

                    );

                }

            }

            popup_el.push(backdrop);

            popup_el.extend(els);

        }



        // ── 变量编辑器（结构性：独立于 `popup`，egui `app.ui.variables`）────────────────────

        // 不画 backdrop：egui 的 Floating 是 `egui::Area`（没有「点外面就关」），Full 更是一块页面。

        // 这样左栏 rail 不被全屏遮罩吞掉（Full 时点 File 能切回），主菜单也不会把它替换掉。

        // Floating 带圆角/阴影/可拖；Full 从 rail 右缘贴边铺满（Frame::NONE）。

        let variables_full = self.variables_open && self.variables_full;

        let variables_el: Vec<AnyElement> = if self.variables_open {

            if let Some(w) = &mut self.variables_editor {

                // 每帧把最新文档快照写回编辑器。命令（新建集合 / 加 mode / 改名 / 删除 …）只改

                // 文档，不刷新快照的话面板永远停在「打开那一刻」——按钮点了、命令也执行了，

                // 但列表没有任何变化，看起来就是「加号点击无效」（用户 2026-11-07 实测）。

                w.set_variables_data(panel_data.variables.clone());

                let me = cx.entity();

                let dismiss = Rc::new(move |_w: &mut Window, cx: &mut App| {

                    me.update(cx, |cv, cx| cv.close_variables(cx));

                });

                let me_drag = cx.entity();

                let drag_start = Rc::new({

                    let me_drag = me_drag.clone();

                    move |e: &MouseDownEvent, _w: &mut Window, cx: &mut App| {

                        me_drag.update(cx, |cv, _cx| cv.begin_popup_drag(e.position));

                    }

                });

                let drag_move = Rc::new(move |e: &MouseMoveEvent, _w: &mut Window, cx: &mut App| {

                    // 同通用窗口：卡片自身 move 也 notify，保证根视图没接到时窗口仍跟手。

                    let held = e.pressed_button == Some(MouseButton::Left);

                    me_drag.update(cx, |cv, cx| {

                        if cv.cancel_drag_if_not_held(held) || cv.move_popup_drag(e.position, held) {

                            cx.notify();

                        }

                    });

                });

                let vp2 = window.viewport_size();

                let edge = px(f32::from(vp2.width) - self.right_w);

                w.render(cx, vp2, dismiss, drag_start, drag_move, edge)

            } else {

                Vec::new()

            }

        } else {

            Vec::new()

        };



        // 四块整面板（Shortcuts / Versions / Dev compare / Assets）：由 `ui_requests` 打开，

        // 画在所有弹层之上；关闭走 ExtraState::dismiss，下一帧由 sync_ui_requests 收掉。

        let extra_el: Vec<AnyElement> = match &self.extra {

            Some(kind) => {

                let dismiss = self.extra_state.dismiss.clone();

                let wake = self.fields.queue();

                let close: Rc<dyn Fn()> = Rc::new(move || {

                    *dismiss.borrow_mut() = true;

                    wake.wake();

                });

                let cmds = self.fields.queue();

                vec![panels::extra::render(kind, &self.app, &cmds, &self.extra_state, cx, close).into_any_element()]

            }

            None => Vec::new(),

        };



        let present_el: Vec<AnyElement> = self.present.as_ref()

            .map(|st| present::render(&self.app, st).into_any_element())

            .into_iter().collect();

        // A19：通用对话框（遮罩 + 居中窗口）画在所有面板 / 弹层 / 整面板之上。

        let dialog_el: Vec<AnyElement> = self.dialog.as_mut().map(|d| d.render(vp)).unwrap_or_default();



        // 拖着文件悬在窗口上（egui `open.rs::drop_overlay`）：整窗一圈强调色 + 中间一句话，

        // 说明松手会发生什么。GPUI 把外部拖入翻译成一次内部拖拽，而本工程只有图层树用

        // `on_drag`，所以「有活动拖拽、又不是在拖图层」就是外部文件。

        let drop_el: Option<AnyElement> = (cx.has_active_drag() && self.layer_tree.drag.is_none()).then(|| {

            let t = widgets::tokens();

            let mut tint = widgets::rgba(t.accent);

            tint.a = 0.12;

            let label: gpui::SharedString =

                if self.tabs.home { figcraft_i18n::tr("Drop to open") } else { figcraft_i18n::tr("Drop to import") }.into();

            div()

                .absolute()

                .left(px(4.0))

                .top(px(4.0))

                .right(px(4.0))

                .bottom(px(4.0))

                .rounded(px(8.0))

                .bg(tint)

                .border_2()

                .border_color(widgets::rgba(t.accent))

                .flex()

                .items_center()

                .justify_center()

                .child(

                    div()

                        .px(px(16.0))

                        .py(px(8.0))

                        .rounded(px(8.0))

                        .bg(widgets::rgba(t.panel))

                        .border_1()

                        .border_color(widgets::rgba(t.accent))

                        .text_size(px(14.0))

                        .font_weight(gpui::FontWeight::SEMIBOLD)

                        .text_color(widgets::rgba(t.text))

                        .child(label),

                )

                .into_any_element()

        });



        // 一条临时提示（egui `app.toast`）：底部中间、圆角卡片，4 秒后自己消失。

        let toast_el: Option<AnyElement> = self.toast_msg.as_ref().map(|(text, _)| {

            let t = widgets::tokens();

            div()

                .absolute()

                .bottom(px(72.0))

                .left(px(0.0))

                .right(px(0.0))

                .flex()

                .flex_row()

                .justify_center()

                .child(

                    div()

                        .px(px(14.0))

                        .py(px(8.0))

                        .rounded(px(8.0))

                        .bg(widgets::rgba(t.menu))

                        .border_1()

                        .border_color(widgets::rgba(t.border_strong))

                        .shadow(vec![gpui::BoxShadow {

                            color: widgets::rgba(t.shadow).into(),

                            offset: gpui::point(px(0.0), px(3.0)),

                            blur_radius: px(14.0),

                            spread_radius: px(0.0),

                        }])

                        .text_size(px(12.0))

                        .text_color(widgets::rgba(t.menu_text))

                        .child(gpui::SharedString::from(text.clone())),

                )

                .into_any_element()

        });



        // R4/R5: the inline rename field. A canvas label gets an absolutely positioned field

        // over the label; a layers row renders this same field *inside the row* (see

        // `layer_tree::render_layer_tree`), so there is no separate overlay for that case.

        let mut rename_dead = false;

        if let Some(r) = self.inline_rename.as_mut() {

            let editor = self.inline_rename_field.clone();

            // GPUI can only focus an element once it has been mounted. Keep asking for the

            // first few frames instead of just the first one (egui `Rename::start`).

            let active_before = editor.read(cx).is_active(window);

            if r.start || !active_before {

                r.start = false;

                editor.update(cx, |te, _| te.focus_editor(window));

            }

            r.frames = r.frames.saturating_add(1);

            let active = editor.read(cx).is_active(window);

            if active {

                r.was_active = true;

            } else if r.was_active {

                // egui keeps the field open only while it has focus.

                rename_dead = true;

            }

        }

        if rename_dead { self.inline_rename = None; }

        let inline_el: Vec<AnyElement> = match &self.inline_rename {

            Some(r) if r.target == InlineRenameTarget::Label => {

                match self.label_rename_rect(r.ids.first().copied()) {

                    Some((x, y, w)) => {

                        let t = widgets::tokens();

                        vec![div()

                            .absolute()

                            .left(px(x))

                            .top(px(y))

                            .w(px(w))

                            .h(px(24.0))

                            .on_mouse_down(MouseButton::Left, |_e, _w, cx| cx.stop_propagation())

                            .bg(widgets::rgba(t.menu))

                            .border_1()

                            .border_color(widgets::rgba(t.accent))

                            .child(self.inline_rename_field.clone())

                            .into_any_element()]

                    }

                    None => Vec::new(),

                }

            }

            _ => Vec::new(),

        };



        // 整个窗口都是拖放目标：GPUI 把 OS 的文件拖入翻译成一次内部拖拽 + 抬起，

        // `on_drop::<ExternalPaths>` 就是落点。落点坐标用 `window.mouse_position()`（Submit 时

        // 已更新），首页与画布的取舍在 `drop_files` 里。

        let drop_target = cx.entity();

        div()



            .size_full()



            .flex()



            .flex_col()



            .overflow_hidden()



            .relative()



            .on_drop::<gpui::ExternalPaths>(move |paths: &gpui::ExternalPaths, window, cx| {

                let at = window.mouse_position();

                let paths = paths.paths().to_vec();

                drop_target.update(cx, |cv, cx| cv.drop_files(&paths, at, cx));

            })

            // 全局字体：MiSans（系统已装）。GPUI 按 family 名解析；子元素与自绘的

            // `window.text_style()` 都继承这一层。

            .font_family("MiSans")

            // ⛔ 点窗口**任意位置**都先结束输入框的编辑（原来只有点画布 / 分隔线才提交，

            // 于是点了别处输入框仍然保持「聚焦 + 全选」的样子，退不出去）。

            // 输入框自己按下时会 `cx.stop_propagation()`，所以点进输入框不会走到这里。

            .on_mouse_down(

                MouseButton::Left,

                cx.listener(|this: &mut CanvasView, _e: &MouseDownEvent, window, cx| {

                    this.commit_edits(window, cx);

                }),

            )

            // B9：Hide UI 与 Minimize UI 都收起导航标签条（`view.minimize` 收导航栏）。

            .when(!ui_hidden && !ui_minimized, |d| d.child(tab_bar))

            .child(top_row)

            // 结构性：变量面板 Full 占「面板 + 画布」区域，底部动画时间轴属于这块，

            // 不能再延伸过来盖住 Full（egui 的 full 由主布局给出整块 panel，Motion 让位）。

            .when(self.motion && !variables_full && !ui_hidden, |d| d.child(dbg_tint(self.render_motion_timeline(window, cx), dbg, 0x8E4EC6)))

            // 变量编辑器在弹层之下、主布局之上：Full 时 rail 仍可点，主菜单等弹层盖在它上面。

            .children(variables_el)

            .children(extra_el)

            .children(present_el)

            .children(drop_el)



            .children(dialog_el)



            .children(toast_el)



            .children(inline_el)

            // Motion 的 clip 设置浮窗：复用 `Popover` 窗口外壳。它排在 `popup_el` **之前**：
            // 卡片里的 Easing 是下拉菜单，菜单必须画在卡片之上，否则被卡片自己盖住
            // （用户 2026-11-12「下拉菜单会被窗口遮挡」）。

            .children(if self.motion && !variables_full { self.motion_clip_popover_layer(cx, vp) } else { Vec::new() })

            // Popups (dropdown menus, the main menu, the Actions palette) are drawn AFTER the
            // dialog layer, so a dropdown inside a dialog is not covered by it. This is the one
            // place the layer order lives, so every dropdown everywhere gets it.
            .children(popup_el)

            // 通用拖拽的视觉层（插入线 / Into 整行描边 / 跟随光标的标签）画在最上面，

            // 用窗口坐标绝对定位，覆盖所有面板。

            .children(self.layer_overlay())

            // 左分隔条拖动中的整窗光标锁定（与 timeline 顶边同构）：只在 paint 阶段、
            // 只在 drag_active 期间调 `set_window_cursor_style(ClosedHand)`，松手即不再登记，
            // 光标自动回退到各元素的 hitbox 级 `.cursor()`（不压右栏输入框）。
            // 1×1 absolute canvas，不接任何鼠标事件、不注册 hitbox，不影响命中顺序。
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    |_, _, window, _| {
                        if crate::resize_drag::any_active() {
                            window.set_window_cursor_style(gpui::CursorStyle::ClosedHand);
                        }
                    },
                )
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .w(px(1.0))
                .h(px(1.0)),
            )

            // 容器级 move：分隔条 resize / 整卡移动 / 控件树 / 时间轴 / easing / 可复用路由器

            // 的拖拽进度已统一由窗口级 `on_window_drag_move` 接管；这里只更新「时间轴指针是否在

            // bar 内」的 hover 标记，拖拽进行中由 `any_window_drag_active()` 提前跳过。

            .on_mouse_move(cx.listener(|this: &mut CanvasView, e: &MouseMoveEvent, window, cx| {

                // Space 在 Motion 时间轴里才播放：时间轴是窗口底部、高 motion_bar_h() 的那条。

                let vh = f32::from(window.viewport_size().height);

                crate::motion::timeline::set_pointer_in_bar(

                    this.motion && f32::from(e.position.y) >= vh - this.motion_bar_h(),

                );

                // Clip 设置弹窗的头部栏拖动：与 easing/timeline 同构，模块自持状态，
                // 根 div 覆盖整窗，指针移出卡片仍连续。不能用 drag_router 的 pressed 判断
                // （GPUI move 事件通常不带 pressed_button → 拖一下就 end）。
                if crate::motion::clip_popover::drag_active() {
                    crate::motion::clip_popover::route_move(e.position);
                    cx.notify();
                    return;
                }

                // Easing 曲线手柄拖拽：同样由窗口级 move 接管（相对位移，不需要元素原点）。

                if crate::motion::easing::drag_active() {

                    crate::motion::easing::route_move(e.position);

                    cx.notify();

                }

                // Motion 时间轴拖拽延续：bar 自己的 on_mouse_move 只在指针悬停其命中框时触发，

                // 一旦指针移出 bar（例如移到上方画布上）拖拽就断。根 div 覆盖整窗、永远悬停，故在此接管。

                if crate::motion::timeline::drag_active() {

                    if e.pressed_button != Some(MouseButton::Left) {

                        // 按钮在窗口外松开：优雅结束（与弹窗拖拽同理），避免卡死。

                        crate::motion::timeline::end_drag(&this.motion_q);

                    } else {

                        crate::motion::timeline::route_global_move(&this.motion_q, e.position);

                    }

                    cx.notify();

                    cx.stop_propagation();

                    return;

                }

                // 通用窗口拖动优先于分隔线拖动。`window_drag_active()` = 按下后标题栏整卡移动

                // （含越过 3pt 死区前）或变量编辑器的 resize handle 正按住——都要在此接管，

                // 指针移出最初元素后拖拽才不会断（与 motion/timeline 的 route_global_move 同理）。

                if this.window_drag_active() {

                    // GPUI only delivers mouse events inside the window. If the button was

                    // released outside, no `on_mouse_up` arrives; the next move with no button

                    // held ends the drag instead of leaving the card glued to the pointer.

                    if e.pressed_button != Some(MouseButton::Left) {

                        this.end_popup_drag();

                        cx.notify();

                        cx.stop_propagation();

                        return;

                    }

                    if this.move_popup_drag(e.position, e.pressed_button == Some(MouseButton::Left)) {

                        cx.notify();

                    }

                    cx.stop_propagation();

                    return;

                }

                // 控件树拖动中：用全局指针重算落点，跳过分隔线拖动逻辑。

                if this.layer_tree.drag.is_some() {

                    this.layer_on_global_move(e.position, cx);

                    return;

                }

                // 尺寸拖动的窗口级接管：与 `on_window_drag_move` 共用同一份 `route_size_drag`。
                if this.route_size_drag(e.position, e.pressed_button == Some(MouseButton::Left)) {
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }

                // 右分隔条维持现状（`resizing` 状态机，左栏已迁到 thread_local）。
                let Some(side) = this.resizing else { return };

                let dx = f32::from(e.position.x) - this.drag_start_x;

                let w = match side {

                    ResizeSide::Left => this.drag_start_w + dx,

                    ResizeSide::Right => this.drag_start_w - dx,

                };

                let w = w.clamp(panels::MIN_PANEL, panels::MAX_PANEL);

                match side {

                    ResizeSide::Left => this.left_w = w,

                    ResizeSide::Right => this.right_w = w,

                }

                cx.notify();

            }))

            // 抬起：面板 resize / 窗口整卡移动 / 控件树 / 时间轴 / easing / 可复用路由器的收尾

            // 已全部由窗口级 `on_window_drag_up` 接管（指针移出面板 / 窗口也不中断），此处只做兜底。

            .on_mouse_up(

                MouseButton::Left,

                cx.listener(|this: &mut CanvasView, _e: &MouseUpEvent, _window, cx| {

                    // Clip 弹窗头部栏抬起：`WindowDragLayer` 没挂，窗口级 up 不触发，
                    // 收尾必须落在这对真正生效的根 div move/up 上（与 easing/timeline 同式）。
                    if crate::motion::clip_popover::drag_active() {

                        crate::motion::clip_popover::end_drag();

                        cx.notify();

                        cx.stop_propagation();

                        return;

                    }

                    // Easing 手柄抬起：整段拖拽收成一个 undo step。

                    if crate::motion::easing::drag_active() {

                        crate::motion::easing::end_drag();

                        cx.notify();

                        cx.stop_propagation();

                        return;

                    }

                    // Motion 拖拽在指针移出 bar 时由根 move 接管，up 也须在此结束（bar 的 up 在指针离开时不触发）。

                    if crate::motion::timeline::drag_active() {

                        crate::motion::timeline::end_drag(&this.motion_q);

                        cx.notify();

                        cx.stop_propagation();

                        return;

                    }

                    // 左分隔条收尾（与 timeline 同式，优先于控件树 / 面板内联路由）。
                    // thread_local 收尾幂等；窗口外松手时这里不触发，由根 move 的下一帧兜底。
                    let was_resize_drag = crate::resize_drag::end_any();

                    // 控件树拖动结束（或左键抬起当作选中）：提交落点 / 选中。

                    this.layer_drag_up(cx);

                    let was_window = this.window_drag_active();

                    this.end_popup_drag();

                    let was_resize = this.resizing.take().is_some();

                    if was_window || was_resize || was_resize_drag {

                        cx.notify();

                    }

                }),

            )

    }

}



/// 通用窗口级拖拽派发（Kit 范式：`canvas` 覆盖层 + `window.on_mouse_event`）。

///

/// 把分散在两个根 div 上的「拖拽续帧 / 收尾」逻辑统一收口到这里：指针无论停在哪个子元素、

/// 是否移出最初按下那个元素、甚至拖出窗口（OS 捕获范围内），事件都由窗口级监听接住，拖拽

/// 不再断裂。hover / 命中（guide / spacing / handles / send）仍由各根 div 的 `on_mouse_move`

/// 负责，但拖拽进行中会跳过（见 `any_window_drag_active`）。

impl drag_manager::WindowDragTarget for CanvasView {

    fn on_window_drag_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {

        // 画布级拖拽（平移 / 缩放 / 拖拽 Move / 多选间距手柄 / 变换手柄）沿用既有 route_canvas_drag。

        if self.route_canvas_drag(e, cx) {

            return;

        }

        // 可复用拖拽路由器（dev 对比分割条、tab 拖拽等）：按下时在 on_mouse_down 里 begin，这里接管。

        // 按真实按键状态收尾，覆盖「指针滑出窗口后松手」导致的监听失效。

        if crate::drag_router::route_move(e.position, e.pressed_button == Some(MouseButton::Left), window, cx) {

            return;

        }

        // Easing 曲线手柄拖拽（相对位移，不需要元素原点）。

        if crate::motion::easing::drag_active() {

            if e.pressed_button != Some(MouseButton::Left) {

                crate::motion::easing::end_drag();

            } else {

                crate::motion::easing::route_move(e.position);

            }

            cx.notify();

            return;

        }

        // Motion 时间轴拖拽延续：bar 自己的 move 只在指针悬停其命中框时触发，移到画布上就断；

        // 窗口级 move 接管，指针移出 bar 仍连续。

        if crate::motion::timeline::drag_active() {

            if e.pressed_button != Some(MouseButton::Left) {

                crate::motion::timeline::end_drag(&self.motion_q);

            } else {

                crate::motion::timeline::route_global_move(&self.motion_q, e.position);

            }

            cx.notify();

            return;

        }

        // 尺寸拖动窗口级接管：与 root 的 `on_mouse_move` 共用同一份 `route_size_drag`。
        if self.route_size_drag(e.position, e.pressed_button == Some(MouseButton::Left)) {
            cx.notify();
            return;
        }

        // 整卡移动 / 变量编辑器 resize handle：按下后标题栏 on_mouse_down 进入 pending/dragging，

        // 指针移出最初元素后拖拽不中断（与 motion/timeline 同理）。

        if self.window_drag_active() {

            if self.cancel_drag_if_not_held(e.pressed_button == Some(MouseButton::Left)) {

                cx.notify();

                return;

            }

            if self.move_popup_drag(e.position, e.pressed_button == Some(MouseButton::Left)) {

                cx.notify();

            }

            return;

        }

        // 控件树（Layers）拖动：用全局指针重算落点，跳过分隔线逻辑。

        if self.layer_tree.drag.is_some() {

            if e.pressed_button != Some(MouseButton::Left) {

                self.layer_drag_up(cx);

            } else {

                self.layer_on_global_move(e.position, cx);

            }

            return;

        }

        // 面板分隔条：move 改宽度（夹紧 240..=500）；处理「窗口外松手」——没按住就结束。

        if let Some(side) = self.resizing {

            if e.pressed_button != Some(MouseButton::Left) {

                self.resizing = None;

                cx.notify();

                return;

            }

            if side != ResizeSide::Right {

                return;

            }

            let dx = f32::from(e.position.x) - self.drag_start_x;

            let w = match side {

                ResizeSide::Left => self.drag_start_w + dx,

                ResizeSide::Right => self.drag_start_w - dx,

            };

            let w = w.clamp(panels::MIN_PANEL, panels::MAX_PANEL);

            match side {

                ResizeSide::Left => self.left_w = w,

                ResizeSide::Right => self.right_w = w,

            }

            cx.notify();

            return;

        }

    }



    fn on_window_drag_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {

        self.mods = e.modifiers;

        let pos = self.local(e.position);

        self.pointer = Some(pos);

        self.primary_down = false;



        // Clip 弹窗头部栏拖动收尾：必须在画布级分支之前清掉，否则松手落在画布上时
        // 状态残留、下次移动鼠标卡片又跟着走。幂等。
        if crate::motion::clip_popover::drag_active() {
            crate::motion::clip_popover::end_drag();
        }

        // 左分隔条收尾（thread_local，与 timeline 同式）：必须在画布级分支之前清掉，
        // 否则指针落在画布参考线 / 标尺区时 `guide_input` 提前 return，状态残留卡在拖拽态。

        if crate::resize_drag::end_any() {

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        // 右分隔条收尾：与画布级拖拽（pan / zoom / handles / guide / 选区间距）互斥，

        // 必须在这些分支之前清掉。否则指针落在画布参考线 / 标尺区时 `guide_input` 提前 return，

        // `resizing` 残留、面板卡在拖拽态（右分隔条正好贴着画布右标尺，松手即触发）。

        if self.resizing.take().is_some() {

            self.drag_just_ended = true;

            cx.notify();

            return;

        }



        // 可复用拖拽路由器收尾（dev 对比分割条等）。

        if crate::drag_router::end(window, cx) {

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        // Easing 手柄抬起：整段拖拽收成一个 undo step。

        if crate::motion::easing::drag_active() {

            crate::motion::easing::end_drag();

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        // Motion 拖拽结束（指针移出 bar 时由窗口级 move 接管，up 也须窗口级结束）。

        if crate::motion::timeline::drag_active() {

            crate::motion::timeline::end_drag(&self.motion_q);

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        // 控件树拖动结束（或左键抬起当作选中）。

        if self.layer_tree.drag.is_some() {

            self.layer_drag_up(cx);

            self.drag_just_ended = true;

            // 继续往下处理画布 / 窗口 / 分隔线收尾（与根 div 原逻辑一致：不中途 return）。

        }

        // 画布级拖拽结束：pan / zoom / 框选 / 变换手柄 / 拖拽 Move。

        if self.pan.take().is_some() {

            self.right_pan = false;

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        if let Some(start) = self.zoom_drag.take() {

            self.finish_zoom(start, pos, e.modifiers.alt);

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        if self.app.selection_spacing.is_dragging() {

            let _ = canvas::selection_spacing::input(

                &mut self.app,

                Some(pos),

                true,

                self.space,

                false,

                false,

                true,

                e.modifiers.platform || e.modifiers.control,

                e.modifiers.alt,

                e.modifiers.shift,

            );

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        if self.app.motion_gizmo.drag.is_some() {

            let _ = canvas::motion_overlay::input(&mut self.app, pos, false, false, true, false);

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        if self.app.handles.drag.is_some() {

            let _ = canvas::handles::input(&mut self.app, pos, false, false, true, e.modifiers.shift);

            self.drag_just_ended = true;

            cx.notify();

            return;

        }

        if !self.dragging && self.guide_input(pos, false, true, e.modifiers.alt) {

            cx.notify();

            return;

        }

        if self.dragging {

            self.dragging = false;

            self.send(PointerKind::Up, pos, &e.modifiers);

            self.drag_just_ended = true;

        }

        // 整卡移动 / 变量编辑器 resize 收尾。

        let was_window = self.window_drag_active();

        self.end_popup_drag();

        let was_resize = self.resizing.take().is_some();

        // 左分隔条收尾（thread_local，幂等；三条 up 路径都兜一次，防状态残留）。
        let was_resize_drag = crate::resize_drag::end_any();

        if was_window || was_resize || was_resize_drag {

            self.drag_just_ended = true;

            cx.notify();

        }

    }



    fn on_window_drag_down(&mut self, _e: &MouseDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {

        // 按下起拖由各元素自身的 `on_mouse_down` 负责（画布在 canvas div、面板在各自控件、

        // 标题栏在弹层头部），它们在按下时写入拖拽状态；本方法留空，避免与元素级 down 重复。

        // 窗口级 down 监听仍每帧注册，便于未来把「起拖」也收口到此处。

    }

}



/// 应用入口。

pub fn run() {

    // 文档文字的量算与渲染都走 `figcraft_text`；它的字体库要显式扫一次系统字体，

    // 否则文字宽度会是 0/1px（画出来就是一条竖线）。egui 版在启动时也是这么做的。

    figcraft_text::scan_system_fonts();

    // `--control <port>`：开本地控制通道（与 `cargo run -p figcraft -- --control 7979` 同义）。

    let control_port = control_port_from_args(std::env::args());

    // 图标走 GPUI 的 SVG 管线（`svg().path(...)` / `paint_svg`），必须由 `AssetSource` 提供字节。

    // GPUI 默认资源源是 `()`（load 恒返回 None），不注册图标就会静默不画。

    // 这是 kit `Icon` 依赖的资源注册契约，本地实现、不引入 gpui-kit crate。

    // `AppAssets` = 原有 IconAssets + 动态内联 SVG 文字（旋转标尺数字）。
    let app = Application::new().with_assets(crate::canvas::svg_text::AppAssets);

    app.run(move |cx: &mut App| {

        let bounds = Bounds::centered(None, size(px(1280.0), px(860.0)), cx);

        // kit `TitleBar::window_options()` 的等价物：声明窗口标题栏区域
        // （appears_transparent = true → Windows hide_title_bar = true，系统不画默认标题栏）。
        // 标题栏由 title_bar.rs 渲染并整条通过 WindowControlArea 交给系统管理；
        // TabBar 内容放在标题栏里，Dock 内容区从 y = TITLE_H 起。
        let options = WindowOptions {

            window_bounds: Some(WindowBounds::Windowed(bounds)),

            titlebar: Some(title_bar::title_bar_options()),

            ..Default::default()

        };

        let view = match cx.open_window(options, |_window, cx| cx.new(CanvasView::new)) {

            Ok(view) => Some(view),

            Err(e) => {

                eprintln!("open_window: {e}");

                None

            }

        };

        // 把根视图交给右栏字段：它们需要通过根视图来打开弹出层（下拉菜单 / 取色器）。

        // 必须在控制服务器之外执行 —— 否则不带 `--control` 启动时字段永远连不上弹层。

        if let Some(view) = &view {

            let _ = view.update(cx, |this, _window, cx| {

                let root = cx.entity();

                this.fields.set_root(root, cx);

                // The field setters don't notify on their own: repaint once so the wired

                // chevron / colour dot show up before the first interaction.

                cx.notify();

            });

        }

        if let (Some(port), Some(view)) = (control_port, view) {

            let _ = view.update(cx, |this, _window, cx| {

                let queue = this.control_queue.clone();

                let wake = CanvasView::control_wake(cx);

                control_server::start(port, queue, wake);

            });

        }

        cx.activate(true);

    });

}



/// `FC_DEBUG_LAYOUT` 开着时给元素描一圈彩色边（不影响布局，border 是内描边）。

fn dbg_tint<E: gpui::Styled>(el: E, on: bool, color: u32) -> E {

    if on { el.border_1().border_color(gpui::rgb(color)) } else { el }

}



/// `--control <port>` / `--control=<port>`（照搬 `apps/figcraft` 的同名选项）。

/// The system's preferred UI languages, comma separated ("zh-CN,en-US"), or `None` when the

/// platform can't say.

///

/// `figcraft_i18n::set` picks the first language in that list that has a catalog, so `"system"`

/// in the preferences resolves to the interface language the OS is actually in. `ui-egui` reads

/// the same list through `apps/figcraft`'s `Services::system_language`; the GPUI front-end runs

/// on its own (the crate is excluded from the workspace), so it asks the OS directly.

fn system_language() -> Option<String> {

    use std::sync::OnceLock;

    static CACHE: OnceLock<Option<String>> = OnceLock::new();

    CACHE.get_or_init(probe_system_language).clone()

}



#[cfg(target_os = "windows")]

fn probe_system_language() -> Option<String> {

    // `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, …)` is what

    // `GlobalizationPreferences::Languages()` wraps: a double-null-terminated list of BCP-47 tags.

    unsafe extern "system" {

        fn GetUserPreferredUILanguages(flags: u32, count: *mut u32, buffer: *mut u16, size: *mut u32) -> i32;

    }

    const MUI_LANGUAGE_NAME: u32 = 0x8;

    let mut count: u32 = 0;

    let mut size: u32 = 0;

    // First call: how big the buffer must be.

    let ok = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, std::ptr::null_mut(), &mut size) };

    if ok == 0 || size == 0 {

        return None;

    }

    let mut buf = vec![0u16; size as usize];

    let ok = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut size) };

    if ok == 0 {

        return None;

    }

    let list: Vec<String> = buf

        .split(|c| *c == 0)

        .filter(|w| !w.is_empty())

        .map(String::from_utf16_lossy)

        .filter(|s| !s.is_empty())

        .collect();

    (!list.is_empty()).then(|| list.join(","))

}



#[cfg(target_os = "macos")]

fn probe_system_language() -> Option<String> {

    // `defaults` prints a list: ( "es-ES", en ).

    let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;

    let list: Vec<String> = String::from_utf8_lossy(&out.stdout)

        .split(['(', ',', ')'])

        .map(|s| s.trim().trim_matches('"').to_string())

        .filter(|s| !s.is_empty())

        .collect();

    (!list.is_empty()).then(|| list.join(","))

}



#[cfg(all(unix, not(target_os = "macos")))]

fn probe_system_language() -> Option<String> {

    // gettext's `LANGUAGE` priority list ("fr:de"), then the locale ("fr_FR.UTF-8").

    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX");

    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(var);

    let list: Vec<String> = var("LANGUAGE").map(|l| l.replace(':', ",")).into_iter().chain(locale).collect();

    (!list.is_empty()).then(|| list.join(","))

}



#[cfg(not(any(windows, unix)))]

fn probe_system_language() -> Option<String> {

    None

}



fn control_port_from_args(args: impl Iterator<Item = String>) -> Option<u16> {

    let mut args = args.skip(1);

    while let Some(a) = args.next() {

        if let Some(rest) = a.strip_prefix("--control=") {

            return rest.parse().ok();

        }

        if a == "--control" {

            return args.next().and_then(|p| p.parse().ok());

        }

    }

    None

}



// ───────────────────────────── IME 输入处理器 ─────────────────────────────

//

// GPUI 把 IME 组合事件（候选、上屏、删除周边）交给 `EntityInputHandler`；引擎那边

// 已经把组合/提交/删除周边按字符实现好（`figcraft_engine::textedit` 的 `preedit` /

// `insert_ime_commit` / `delete_surrounding_*` / `set_caret` …），这里只做

// 「GPUI 的 UTF-16 坐标 ↔ 引擎的字符坐标」桥。

//

// 组合（preedit）在引擎里是「UI 专用、还没进文档」的一段文字：IME 看到的

// 「合成中文本」= 节点正文在 caret 处插入 preedit。这样 `marked_text_range` /

// `text_for_range` 自洽，平台每次 `replace_and_mark_text_in_range` 都把整段 preedit

// 换成新文字（引擎 `preedit` 直接覆盖），`unmark_text` 把 preedit 提交进文档。



/// 一段文本的 UTF-16 码元总数（一个字符占 1 或 2 个码元）。

/// 仅测试使用（转换的生产路径在 `widgets::input` 的 `EditBuffer` 里）。

#[allow(dead_code)]

fn utf16_len(s: &str) -> usize {

    s.chars().map(|c| c.len_utf16()).sum()

}



// The canvas IME bridge and the panel's `EditBuffer` share one UTF-16 <-> character conversion,

// so emoji/CJK caret offsets cannot drift between the two editors.

use crate::widgets::{char_to_utf16_str as char_to_utf16, utf16_to_char_str as utf16_to_char};



/// IME 视角下的「合成中文本」：节点正文 + 在 caret 处插入的 preedit，并给出各角色的

/// 字符索引（caret / anchor / preedit 区间）。组合中 anchor = caret = 组合光标。

struct Composed {

    text: String,

    caret: usize,

    anchor: usize,

    mark_start: usize,

    mark_end: usize,

    composing: bool,

}



fn composed(app: &CanvasApp) -> Option<Composed> {

    let ds = app.session.ds().ok()?;

    let te = ds.text_edit.as_ref()?;

    let t = ds.doc.node(te.id).and_then(|n| n.kind.text())?;

    let base = t.characters.clone();

    let at = te.caret.min(t.char_count());

    match te.preedit.as_ref() {

        Some(pe) => {

            let n = pe.text.chars().count();

            let cursor = at + pe.cursor.min(n);

            let text: String = base.chars().take(at).chain(pe.text.chars()).chain(base.chars().skip(at)).collect();

            Some(Composed { text, caret: cursor, anchor: cursor, mark_start: at, mark_end: at + n, composing: true })

        }

        None => {

            let (a, b) = (te.caret.min(te.anchor), te.caret.max(te.anchor));

            Some(Composed { text: base, caret: te.caret, anchor: te.anchor, mark_start: a, mark_end: b, composing: false })

        }

    }

}



impl EntityInputHandler for CanvasView {

    fn text_for_range(

        &mut self,

        range: Range<usize>,

        adjusted_range: &mut Option<Range<usize>>,

        _: &mut Window,

        _: &mut Context<Self>,

    ) -> Option<String> {

        let c = composed(&self.app)?;

        let len = c.text.chars().count();

        let a = utf16_to_char(&c.text, range.start);

        let mut b = utf16_to_char(&c.text, range.end);

        if b > len {

            b = len;

        }

        let out: String = c.text.chars().skip(a).take(b.saturating_sub(a)).collect();

        *adjusted_range = Some(char_to_utf16(&c.text, a)..char_to_utf16(&c.text, b));

        Some(out)

    }



    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {

        let c = composed(&self.app)?;

        if c.composing {

            // 组合中把「当前选择」报成整段 preedit：平台下次替换就精准命中组合。

            Some(UTF16Selection { range: char_to_utf16(&c.text, c.mark_start)..char_to_utf16(&c.text, c.mark_end), reversed: false })

        } else {

            let (a, b) = (c.caret.min(c.anchor), c.caret.max(c.anchor));

            Some(UTF16Selection { range: char_to_utf16(&c.text, a)..char_to_utf16(&c.text, b), reversed: c.caret > c.anchor })

        }

    }



    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {

        let c = composed(&self.app)?;

        if c.composing { Some(char_to_utf16(&c.text, c.mark_start)..char_to_utf16(&c.text, c.mark_end)) } else { None }

    }



    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {

        // 提交组合：把 preedit 以上屏方式写进文档（引擎会清掉 preedit）。

        let pe = self.app.session.ds().ok().and_then(|d| d.text_edit.clone()).and_then(|te| te.preedit).map(|pe| pe.text);

        if let Some(text) = pe {

            let _ = textedit::insert_ime_commit(&mut self.app.session, &text);

        }

    }



    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, _: &mut Context<Self>) {

        // 组合中上屏：交给 insert_ime_commit（清 preedit）；否则普通输入交给 insert。

        let composing = self.app.session.ds().ok().and_then(|d| d.text_edit.as_ref()).is_some_and(|te| te.preedit.is_some());

        if composing {

            let _ = textedit::insert_ime_commit(&mut self.app.session, text);

        } else {

            let _ = textedit::insert(&mut self.app.session, text);

        }

    }



    fn replace_and_mark_text_in_range(

        &mut self,

        _: Option<Range<usize>>,

        new_text: &str,

        new_selected_range: Option<Range<usize>>,

        _: &mut Window,

        _: &mut Context<Self>,

    ) {

        // IME 组合更新：引擎的 preedit 直接覆盖成新文字；光标落在 new_selected_range 末尾。

        let cursor = new_selected_range.map(|r| utf16_to_char(new_text, r.end)).unwrap_or_else(|| new_text.chars().count());

        let _ = textedit::preedit(&mut self.app.session, new_text, Some(cursor));

    }



    fn bounds_for_range(&mut self, _: Range<usize>, element_bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {

        // 候选窗定位：优先给文字层屏幕矩形，否则退回画布矩形。

        let ds = self.app.session.ds().ok()?;

        let te = ds.text_edit.as_ref()?;

        match ds.doc.world_bounds(te.id) {

            Some(b) => {

                let min = self.app.to_screen(b.x0, b.y0);

                let max = self.app.to_screen(b.x1, b.y1);

                Some(Bounds::from_corners(Point::new(px(min.x), px(min.y)), Point::new(px(max.x), px(max.y))))

            }

            None => Some(element_bounds),

        }

    }



    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {

        // 没有字形级命中：返回 None，平台沿用当前 caret（点选定位交给引擎的 pointer 事件）。

        None

    }

}



#[cfg(test)]

mod key_tests {

    use super::*;

    use figcraft_engine::keymap::{self, Mods, Platform};



    fn bind(platform: Platform, key: &str, m: Mods) -> Option<&'static str> {

        keymap::lookup(platform, keymap::Context::Canvas, &canonical_key(key), m).map(|b| b.id)

    }



    /// 撤销/重做：macOS 是 Cmd+Z / Shift+Cmd+Z，Windows/Linux 是 Ctrl+Z / Shift+Ctrl+Z。

    #[test]

    fn undo_and_redo_follow_the_engine_keymap() {

        assert_eq!(bind(Platform::Mac, "z", Mods { cmd: true, ..Default::default() }), Some("edit.undo"));

        assert_eq!(bind(Platform::Mac, "z", Mods { cmd: true, shift: true, ..Default::default() }), Some("edit.redo"));

        assert_eq!(bind(Platform::Other, "z", Mods { ctrl: true, ..Default::default() }), Some("edit.undo"));

        assert_eq!(bind(Platform::Other, "z", Mods { ctrl: true, shift: true, ..Default::default() }), Some("edit.redo"));

        // 不带修饰键的 z 是缩放工具，不是撤销。

        assert_eq!(bind(Platform::Other, "z", Mods::default()), None);

    }



    /// GPUI 的键名要能对上 keymap 的规范名，否则 Delete/方向键/F 键全都查不到。

    #[test]

    fn gpui_key_names_reach_the_keymap() {

        assert_eq!(canonical_key("delete"), "Delete");

        assert_eq!(canonical_key("backspace"), "Backspace");

        assert_eq!(canonical_key("left"), "ArrowLeft");

        assert_eq!(canonical_key("space"), "Space");

        assert_eq!(canonical_key("f5"), "F5");

        assert_eq!(canonical_key("a"), "a");

        assert_eq!(bind(Platform::Other, "delete", Mods::default()), Some("edit.delete"));

        assert_eq!(bind(Platform::Other, "a", Mods { ctrl: true, ..Default::default() }), Some("select.all"));

        assert_eq!(bind(Platform::Other, "d", Mods { ctrl: true, ..Default::default() }), Some("edit.duplicate"));

    }

}



/// 右键菜单交给通用 `MenuAim` 的几何：子菜单卡片必须贴在展开行右侧、与该行顶部对齐

/// （`row_top - PAD`），三角才画在正确的位置。

#[cfg(test)]

mod context_geom_tests {

    use super::*;

    use crate::menu_card::Item;



    fn items() -> Vec<Item> {

        vec![

            Item::new("a", tr("Copy")),

            Item::new("b", "Paste"),

            Item::new("copy.as", tr("Copy as")).sub(vec![Item::new("x", "CSS")]),

        ]

    }



    #[test]

    fn submenu_panel_sits_right_of_the_open_row() {

        let pos = Pos2 { x: 0.0, y: 0.0 };

        let (card, trigger, panel) = CanvasView::context_geom(pos, 0.0, 1000.0, 800.0, &items(), Some(2));

        let trigger = trigger.expect("the Copy as row is the trigger");

        let panel = panel.expect("an open submenu has a panel");

        assert_eq!(f32::from(card.origin.x), 8.0);

        assert_eq!(f32::from(card.origin.y), 8.0);

        assert_eq!(f32::from(trigger.origin.y), 8.0 + 8.0 + 2.0 * context_menu::ROW_H);

        assert_eq!(f32::from(panel.origin.x), f32::from(card.origin.x) + context_menu::WIDTH + 4.0);

        assert_eq!(f32::from(panel.origin.y), f32::from(trigger.origin.y) - 8.0);

    }

}



/// Alt 拖动 = 复制一份再拖（引擎在 `drag.rs` 的 Down 分支里做，UI 只要把 alt 传进去）。

/// 这条测试用 `send()` 同款的 `PointerEvent` 走一遍，确认「按 alt 拖」真的会复制。

#[test]

fn alt_dragging_a_layer_duplicates_it() {

    use figcraft_engine::Session;

    let mut s = Session::new();

    let page = s.ds().expect("session").page;

    let id = serde_json::from_value::<figcraft_doc::NodeId>(

        s.execute("shape.rectangle", &serde_json::json!({"x": 0, "y": 0, "width": 50, "height": 50})).expect("rect")["id"].clone(),

    )

    .expect("id");

    s.execute("select.set", &serde_json::json!({"ids": [id]})).expect("select");

    let ev = |kind: PointerKind, x: f64, y: f64, alt: bool| PointerEvent {

        kind,

        x,

        y,

        sx: x,

        sy: y,

        shift: false,

        alt,

        cmd: false,

        ctrl: false,

        zoom: 1.0,

        pressure: None,

    };

    // 在图形内部按下（Alt），往右拖 50，松手。

    s.pointer(&ev(PointerKind::Down, 25.0, 25.0, true)).expect("down");

    s.pointer(&ev(PointerKind::Move, 75.0, 25.0, true)).expect("move");

    s.pointer(&ev(PointerKind::Up, 75.0, 25.0, true)).expect("up");

    // 原件留着，副本被拖走：页面上两个矩形，位置不同。

    let ds = s.ds().expect("session");

    let ids = ds.doc.node(page).map(|p| p.children.clone()).unwrap_or_default();

    assert_eq!(ids.len(), 2, "Alt+drag leaves the original and a copy");

    let xs: Vec<f64> = ids.iter().filter_map(|id| ds.doc.world_bounds(*id).map(|b| b.x0)).collect();

    assert!(xs.contains(&0.0), "the original stayed put: {xs:?}");

    assert!(xs.iter().any(|x| (*x - 50.0).abs() < 1.0), "the copy followed the pointer: {xs:?}");

}



#[cfg(test)]

mod ime_tests {

    use super::*;

    use figcraft_doc::NodeKind;



    /// 找页面上的文本层；示例文档里 `text.create` 加的那个 "Hello" 若不在就现建一个。

    fn text_id(app: &mut CanvasApp) -> figcraft_doc::NodeId {

        {

            let ds = app.session.ds().expect("session");

            let page = ds.page;

            if let Some(id) = ds.doc.subtree(page).into_iter().find(|id| ds.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) {

                return id;

            }

        }

        let v = app.session.execute("text.create", &serde_json::json!({"text": "Hello", "x": 10.0, "y": 10.0})).expect("create text");

        serde_json::from_value(v["id"].clone()).expect("text id")

    }



    #[test]

    fn utf16_offsets_round_trip_across_char_and_surrogate() {

        // UTF-16 只有「BMP 之外的字符」（如 emoji U+1F600）才用代理对占 2 个码元；

        // CJK 统一表意文字（中=U+4E2D、文=U+6587）都在 BMP 内，每字符只占 1 个码元。

        // 这里混入一个 astral 字符，才能真正走到「双码元 / 代理对」分支并验证往返。

        let mut s = String::new();

        s.push('a'); // U+0061，1 个 UTF-16 码元

        s.push('\u{1f600}'); // 😀 U+1F600，astral，2 个码元（代理对）

        s.push('b'); // 1

        s.push('\u{4e2d}'); // 中，BMP，1 个码元

        s.push('c'); // 1

        // 共 5 个字符、6 个 UTF-16 码元。

        assert_eq!(utf16_len(&s), 6);

        assert_eq!(char_to_utf16(&s, 0), 0);

        assert_eq!(char_to_utf16(&s, 1), 1); // 😀 起始

        assert_eq!(char_to_utf16(&s, 2), 3); // 过完 😀（占 2 个码元）

        assert_eq!(char_to_utf16(&s, 3), 4); // 过完 中

        assert_eq!(char_to_utf16(&s, 4), 5);

        assert_eq!(char_to_utf16(&s, 5), 6);

        // 反向：UTF-16 偏移 → 字符索引，落在代理对中间要归到该字符起点。

        assert_eq!(utf16_to_char(&s, 0), 0);

        assert_eq!(utf16_to_char(&s, 1), 1); // 😀 第一个码元

        assert_eq!(utf16_to_char(&s, 2), 1); // 😀 第二个码元（代理对内）

        assert_eq!(utf16_to_char(&s, 3), 2);

        assert_eq!(utf16_to_char(&s, 4), 3); // 中

        assert_eq!(utf16_to_char(&s, 5), 4);

        assert_eq!(utf16_to_char(&s, 6), 5);

    }



    #[test]

    fn composed_exposes_preedit_as_the_marked_range() {

        let mut app = CanvasApp::new(sample_doc());

        let id = text_id(&mut app);

        // 在末尾开始编辑（caret = 5，选择收拢），不选中。

        textedit::begin(&mut app.session, id, Some(5), false).expect("begin");

        // 组合「中文」，IME 光标停在「中」之后（字符索引 1）。

        textedit::preedit(&mut app.session, "中文", Some(1)).expect("preedit");



        let c = composed(&app).expect("composed");

        assert!(c.composing, "preedit 激活 → composing");

        // 「Hello」5 字符，preedit 插在 caret(5) 处，占 2 字符。

        assert_eq!(c.mark_start, 5);

        assert_eq!(c.mark_end, 7);

        assert_eq!(c.caret, 6, "IME 光标 = at + 组合内光标(1)");

        // marked_text_range 应为这段的 UTF-16 区间。

        let m = char_to_utf16(&c.text, c.mark_start)..char_to_utf16(&c.text, c.mark_end);

        assert_eq!(m, 5..7, "「中文」是 2 个 BMP 字符 = 2 个 UTF-16 码元");

        // 组合中节点正文还没动（preedit 是 UI 专用）。

        let node_text = app.session.ds().unwrap().doc.node(id).unwrap().kind.text().unwrap().characters.clone();

        assert_eq!(node_text, "Hello");

    }



    #[test]

    fn unmark_commits_preedit_into_the_document() {

        let mut app = CanvasApp::new(sample_doc());

        let id = text_id(&mut app);

        textedit::begin(&mut app.session, id, Some(5), false).expect("begin");

        textedit::preedit(&mut app.session, "XY", Some(2)).expect("preedit");

        // 模拟 `unmark_text`：把 preedit 以上屏方式写进文档。

        let pe = app.session.ds().unwrap().text_edit.clone().unwrap().preedit.unwrap().text;

        textedit::insert_ime_commit(&mut app.session, &pe).expect("commit");

        let node_text = app.session.ds().unwrap().doc.node(id).unwrap().kind.text().unwrap().characters.clone();

        assert_eq!(node_text, "HelloXY", "preedit 提交后进入文档");

        // 提交后组合清空。

        assert!(app.session.ds().unwrap().text_edit.as_ref().unwrap().preedit.is_none());

    }

}



/// 变量窗口拖拽的端到端验证：真实鼠标事件（按下→移动→抬起）必须真的把整张卡拖动起来。

/// 这证明修复链路成立——**头部左侧** `on_mouse_down`（记 press）→ 根视图 `on_mouse_move`

/// （指针移出卡片仍接管）→ `move_popup_drag` 改 `place`——而不是只在状态机层自测。

#[cfg(test)]

mod popup_drag_tests {

    use super::*;

    use crate::panels::right::variables::{WINDOW_H, WINDOW_W};

    use gpui::{MouseButton, Modifiers, Point, TestAppContext, px};



    #[gpui::test]

    async fn variables_card_drag_moves_window_via_real_mouse_events(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));



        // 打开 Floating 变量面板（绕过左栏 rail 点击，直接走根视图的 open_window）。

        view.update(window, |cv, cx| {

            cv.test_open_variables_floating(cx);

        });



        // 触发一次重绘，让卡片 origin（窗口坐标）算出来，之后才能对卡片落点下手。

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let origin = view

            .update(window, |cv, _cx| cv.test_variables_origin())

            .expect("variables card should be open and rendered");



        // 在**头部左侧**按下（不是 body）：整卡拖动只绑头部，body 里点任何地方都不该拖窗口。

        let press = Point::new(origin.x + px(60.0), origin.y + px(20.0));

        window.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());



        assert!(

            view.update(window, |cv, _cx| cv.test_variables_place()).is_none(),

            "press alone must not move the card (it is still mid dead-zone / pending)"

        );



        // 按住左键越过 3pt 死区并移动 (200,200)→(230,230)，差值 (30,30)。根视图 on_mouse_move

        // 接管：window_drag_active() 为真 → move_popup_drag → place 改变。

        let moved = Point::new(press.x + px(30.0), press.y + px(30.0));

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());



        let place = view

            .update(window, |cv, _cx| cv.test_variables_place())

            .expect("drag should have relocated the card");

        let dx = f32::from(place.x) - f32::from(origin.x);

        let dy = f32::from(place.y) - f32::from(origin.y);

        // place = clamp(press + (30,30) - grab, …)，而 grab = press - origin，故 place ≈ origin + (30,30)。

        assert!((dx - 30.0).abs() < 2.0, "card moved {dx}px in x, expected ~30");

        assert!((dy - 30.0).abs() < 2.0, "card moved {dy}px in y, expected ~30");



        // 抬起：拖拽结束，但窗口停在落点（place 不回零，否则下次打开又回到原位）。

        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        let place_after = view

            .update(window, |cv, _cx| cv.test_variables_place())

            .expect("place persists after mouse up");

        assert!(

            (f32::from(place_after.x) - f32::from(place.x)).abs() < 0.01

                && (f32::from(place_after.y) - f32::from(place.y)).abs() < 0.01,

            "place must stay put after the drag ends"

        );

    }



    /// 在 body（表格/按钮区域）按下并拖动，**不能**把整张卡拖走。

    /// 用户 2026-11-07：拖动区绑整卡后「点哪里都可以开始拖」，这里把「只有头部能拖」锁死。

    #[gpui::test]

    async fn pressing_the_body_does_not_drag_the_card(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        view.update(window, |cv, cx| {

            cv.test_open_variables_floating(cx);

        });

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());

        let origin = view

            .update(window, |cv, _cx| cv.test_variables_origin())

            .expect("variables card should be open and rendered");

        // body 深处（表格区），不是头部。

        let press = Point::new(origin.x + px(200.0), origin.y + px(200.0));

        window.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());

        let moved = Point::new(press.x + px(30.0), press.y + px(30.0));

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());

        assert!(

            view.update(window, |cv, _cx| cv.test_variables_place()).is_none(),

            "a press in the body must not relocate the card"

        );

    }



    /// 端到端验证 resize：真实鼠标在「东向 handle」按下→右拖，卡片宽度应增大、高度不变，

    /// 且抬起后尺寸保留。这覆盖了原需求里"也不能改变大小"那一半，证明根视图接管对 resize 同样生效。

    #[gpui::test]

    async fn variables_card_resize_handle_changes_size_via_real_mouse_events(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));



        view.update(window, |cv, cx| {

            cv.test_open_variables_floating(cx);

        });

        // 重绘：算 origin，并建好编辑器（含 8 个 resize handle）。

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let origin = view

            .update(window, |cv, _cx| cv.test_variables_origin())

            .expect("variables card should be open and rendered");

        let w0 = WINDOW_W;

        let h0 = WINDOW_H;

        assert_eq!(w0, 880.0);

        assert_eq!(h0, 480.0);



        // 东向 handle：卡片右缘 6px 竖条（right:0, w:6, h_full）。点中它按下 → 记 resize。

        let press = Point::new(origin.x + px(w0 - 3.0), origin.y + px(h0 / 2.0));

        window.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());

        assert!(

            view.update(window, |cv, _cx| cv.test_variables_size()).is_none(),

            "size must not change on the press alone"

        );



        // 按住左键往右拖 50px：整卡加宽 50（dir & 1 → w = w0 + dx）。

        let moved = Point::new(press.x + px(50.0), press.y);

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());



        let size = view

            .update(window, |cv, _cx| cv.test_variables_size())

            .expect("resize should have set a size");

        let dw = size.0 - w0;

        let dh = size.1 - h0;

        assert!((dw - 50.0).abs() < 2.0, "card widened by {dw}px, expected ~50");

        assert!(dh.abs() < 0.5, "height must stay {h0} when dragging east-only, got {}", size.1);



        // 松手：resize 状态清空，但尺寸保留。

        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        let size_after = view

            .update(window, |cv, _cx| cv.test_variables_size())

            .expect("size persists after mouse up");

        assert!(

            (size_after.0 - size.0).abs() < 0.01 && (size_after.1 - size.1).abs() < 0.01,

            "size must stay put after the resize ends"

        );

    }

}



/// 左/右面板分隔条 resize 的端到端自测：覆盖「菜单快速滑动跳针」同类问题——

/// 即高频 `WM_MOUSEMOVE` 被系统合并、只交付最新一条时，拖拽会不会断裂 / 卡死 / 残留状态。

///

/// 与菜单不同，面板宽度由**绝对指针位置** `drag_start_w + (e.position.x - drag_start_x)` 算出，

/// 所以合并后宽度只跳到真实指针处、不会卡在错误中间态（这正是 Kit 范式下 resize 天然抗合并的原因）。

/// 这里用真实鼠标事件逐一验证：慢拖、镜像侧、快速跳针、窗口外松手、上下夹紧。

#[cfg(test)]

mod panel_resize_tests {

    use super::*;

    use crate::panels::{HANDLE_HIT_W, HANDLE_OVERHANG, HANDLE_W, MAX_PANEL, MIN_PANEL, RAIL_W};

    use gpui::{MouseButton, Modifiers, Point, Size, TestAppContext, px};



    /// 左分隔条中心 x = RAIL_W + left_w + HANDLE_W/2；右分隔条中心 x = vw - right_w - HANDLE_W/2。

    fn handle_x(side: ResizeSide, vw: f32, w: f32) -> f32 {
        panels::divider_center_x(side, vw, w)
    }

    /// 4px 热区实测：以可见 1px 线为中心，左右各溢出 `HANDLE_OVERHANG`(1.5px)。
    /// 打在 [centre−2, centre+2] 内（含两侧溢出带）都要能启动 resize；打到
    /// [centre±2.5]（覆盖条外，属于画布 / 面板）则不该由分隔条接管。
    /// 这同时证明「后绘制的覆盖条压得住相邻的画布」——否则溢出那 1.5px 会被画布抢走。
    #[gpui::test]
    async fn divider_hit_area_is_four_px_centred_on_the_hairline(cx: &mut TestAppContext) {
        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));
        window.simulate_resize(Size::new(px(1280.0), px(860.0)));
        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());

        let left0 = view.update(window, |cv, _cx| cv.test_left_w());
        let centre = handle_x(ResizeSide::Left, 1280.0, left0);
        let hy = px(430.0);

        // 覆盖条共 4px：中心、左侧溢出带、右侧溢出带各打一下，都应启动 resize。
        for dx in [-HANDLE_OVERHANG + 0.2, 0.0, HANDLE_OVERHANG - 0.2] {
            window.simulate_mouse_up(Point::new(px(centre + dx), hy), MouseButton::Left, Modifiers::default());
            window.simulate_mouse_down(Point::new(px(centre + dx), hy), MouseButton::Left, Modifiers::default());
            assert_eq!(
                view.update(window, |cv, _cx| cv.test_resizing()),
                Some(ResizeSide::Left),
                "dx={dx} 落在 4px 热区内应启动 resize"
            );
        }

        // 覆盖条外 2.5px（> HANDLE_HIT_W/2 = 2.0）：归画布，不该由分隔条接管。
        window.simulate_mouse_up(Point::new(px(centre + 2.5), hy), MouseButton::Left, Modifiers::default());
        window.simulate_mouse_down(Point::new(px(centre + 2.5), hy), MouseButton::Left, Modifiers::default());
        assert_eq!(
            view.update(window, |cv, _cx| cv.test_resizing()),
            None,
            "4px 热区外不该启动 resize"
        );

        // 常量自洽：1 + 1.5×2 = 4。
        assert_eq!(HANDLE_HIT_W, HANDLE_W + HANDLE_OVERHANG * 2.0);
    }



    #[gpui::test]

    async fn left_panel_resize_slow_drag_updates_width_and_persists(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        // 触发一次布局，让分隔条位置算出来。

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let left0 = view.update(window, |cv, _cx| cv.test_left_w());

        assert_eq!(left0, 280.0, "left panel starts at DEFAULT_PANEL 280");



        let hx = px(handle_x(ResizeSide::Left, 1280.0, left0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());

        assert_eq!(

            view.update(window, |cv, _cx| cv.test_resizing()),

            Some(ResizeSide::Left),

            "press on left handle starts resize"

        );



        // 按住左键往右拖 60px：左面板应加宽 60。

        let moved = Point::new(hx + px(60.0), hy);

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());

        let left1 = view.update(window, |cv, _cx| cv.test_left_w());

        assert!((left1 - 340.0).abs() < 2.0, "left panel widened to {left1}, expected ~340");



        // 松手：resize 清空，宽度保留。

        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        assert_eq!(

            view.update(window, |cv, _cx| cv.test_resizing()),

            None,

            "resize cleared after mouse up"

        );

        let left2 = view.update(window, |cv, _cx| cv.test_left_w());

        assert!((left2 - left1).abs() < 0.01, "width persists after mouse up");

    }



    #[gpui::test]

    async fn right_panel_resize_drag_updates_width_and_persists(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let right0 = view.update(window, |cv, _cx| cv.test_right_w());

        // 启动默认 Motion 模式：右栏初始宽度 = motion::INSPECTOR_W（413），不是 DEFAULT_PANEL。

        // 这里不写死，直接以实际初值推导期望，避免与产品默认耦合。



        let hx = px(handle_x(ResizeSide::Right, 1280.0, right0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());

        assert_eq!(

            view.update(window, |cv, _cx| cv.test_resizing()),

            Some(ResizeSide::Right),

            "press on right handle starts resize"

        );



        // 右面板：往左拖 50px → dx = -50 → w = right0 - (-50) = right0 + 50（右面板加宽）。

        let moved = Point::new(hx - px(50.0), hy);

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());

        let right1 = view.update(window, |cv, _cx| cv.test_right_w());

        assert!((right1 - (right0 + 50.0)).abs() < 2.0, "right panel widened to {right1}, expected ~{}", right0 + 50.0);



        // 松手：状态清空、宽度保留。

        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        assert_eq!(view.update(window, |cv, _cx| cv.test_resizing()), None);

        let right2 = view.update(window, |cv, _cx| cv.test_right_w());

        assert!((right2 - right1).abs() < 0.01, "right width persists after mouse up");

    }



    /// 模拟 `WM_MOUSEMOVE` 合并：按下后**直接**跳到远点、中间一步都不发。

    /// 若 resize 用相对增量累加，这里会漏掉中间位移导致宽度不对；本实现用绝对指针位置，

    /// 跳针后宽度正好落在最终指针处——这正是它**不会**像菜单那样「卡在错误中间态」的原因。

    #[gpui::test]

    async fn left_panel_resize_fast_jump_tracks_final_position(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let left0 = view.update(window, |cv, _cx| cv.test_left_w());

        let hx = px(handle_x(ResizeSide::Left, 1280.0, left0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());



        // 一步跳 120px，无中间 move。

        let jumped = Point::new(hx + px(120.0), hy);

        window.simulate_mouse_move(jumped, MouseButton::Left, Modifiers::default());

        let left1 = view.update(window, |cv, _cx| cv.test_left_w());

        assert!((left1 - 400.0).abs() < 2.0, "jump drag landed at {left1}, expected ~400 (absolute tracking)");



        window.simulate_mouse_up(jumped, MouseButton::Left, Modifiers::default());

        assert_eq!(view.update(window, |cv, _cx| cv.test_resizing()), None);

    }



    /// 窗口外松手：指针移出窗口后系统不再上报 `pressed_button`，最后一条 move 的

    /// `pressed_button == None` 应让 resize 立刻收尾（不残留、不冻结），宽度停在最后一次

    /// 仍按住时的位置，而非继续跳变。

    #[gpui::test]

    async fn left_panel_resize_release_outside_window_clears_state(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let left0 = view.update(window, |cv, _cx| cv.test_left_w());

        let hx = px(handle_x(ResizeSide::Left, 1280.0, left0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());



        // 先按住拖 40px（仍在窗口内、按键上报）。

        let held = Point::new(hx + px(40.0), hy);

        window.simulate_mouse_move(held, MouseButton::Left, Modifiers::default());

        let left_held = view.update(window, |cv, _cx| cv.test_left_w());

        assert!((left_held - 320.0).abs() < 2.0, "after held drag width ~320, got {left_held}");



        // 再「移出窗口」：按键不再上报（pressed_button = None）。

        let outside = Point::new(hx + px(80.0), hy);

        window.simulate_mouse_move(outside, None, Modifiers::default());

        assert_eq!(

            view.update(window, |cv, _cx| cv.test_resizing()),

            None,

            "resizing must end when the button is no longer reported held (pointer left window)"

        );

        // 宽度停在最后一次仍按住的位置（320），不因「窗口外」那条 move 继续变。

        let left_out = view.update(window, |cv, _cx| cv.test_left_w());

        assert!((left_out - 320.0).abs() < 0.01, "width frozen at last held value, got {left_out}");



        // 真正松手也不该再触发任何状态。

        window.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());

        assert_eq!(view.update(window, |cv, _cx| cv.test_resizing()), None);

    }



    #[gpui::test]

    async fn left_panel_resize_clamps_to_max(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let left0 = view.update(window, |cv, _cx| cv.test_left_w());

        let hx = px(handle_x(ResizeSide::Left, 1280.0, left0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());



        // 往右拖 400px：280 + 400 = 680 > MAX(500) → 夹紧 500。

        let moved = Point::new(hx + px(400.0), hy);

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());

        let left1 = view.update(window, |cv, _cx| cv.test_left_w());

        assert_eq!(left1, MAX_PANEL, "clamped to MAX_PANEL {MAX_PANEL}, got {left1}");



        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        assert_eq!(view.update(window, |cv, _cx| cv.test_resizing()), None);

    }



    #[gpui::test]

    async fn left_panel_resize_clamps_to_min(cx: &mut TestAppContext) {

        let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));

        window.simulate_resize(Size::new(px(1280.0), px(860.0)));

        window.simulate_mouse_move(Point::new(px(0.0), px(0.0)), None, Modifiers::default());



        let left0 = view.update(window, |cv, _cx| cv.test_left_w());

        let hx = px(handle_x(ResizeSide::Left, 1280.0, left0));

        let hy = px(430.0);

        window.simulate_mouse_down(Point::new(hx, hy), MouseButton::Left, Modifiers::default());



        // 往左拖 200px：280 - 200 = 80 < MIN(240) → 夹紧 240。

        let moved = Point::new(hx - px(200.0), hy);

        window.simulate_mouse_move(moved, MouseButton::Left, Modifiers::default());

        let left1 = view.update(window, |cv, _cx| cv.test_left_w());

        assert_eq!(left1, MIN_PANEL, "clamped to MIN_PANEL {MIN_PANEL}, got {left1}");



        window.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

        assert_eq!(view.update(window, |cv, _cx| cv.test_resizing()), None);

    }

}



#[cfg(test)]
mod keyboard_audit_tests {
    use super::canonical_key;
    use figcraft_engine::keymap::{self, Context, Platform};

    /// Reverse a keymap canonical key into the name GPUI puts in `Keystroke::key`.
    fn gpui_key(canonical: &str) -> String {
        match canonical {
            "Delete" => "delete",
            "Backspace" => "backspace",
            "Enter" => "enter",
            "Escape" => "escape",
            "Tab" => "tab",
            "Space" => "space",
            "Home" => "home",
            "End" => "end",
            "PageUp" => "pageup",
            "PageDown" => "pagedown",
            "ArrowLeft" => "left",
            "ArrowRight" => "right",
            "ArrowUp" => "up",
            "ArrowDown" => "down",
            other => return other.to_ascii_lowercase(),
        }
        .to_string()
    }

    /// Every canvas binding must survive `canonical_key` and still resolve to its own command.
    #[test]
    fn audit_every_canvas_binding() {
        // Key names are protocol strings now: independent of the UI language. Never touch
        // the global locale here, or tests running in parallel read translated labels.
        for (gpu, want) in [("delete", "Delete"), ("space", "Space"), ("home", "Home"), ("end", "End"), ("enter", "Enter")] {
            assert_eq!(canonical_key(gpu), want, "key names must not be localised");
        }
        let platform = Platform::current();
        let mut ok = 0usize;
        let mut broken: Vec<String> = Vec::new();
        for b in keymap::KEYS {
            if !matches!(b.context, Context::Canvas | Context::Both) {
                continue;
            }
            for spec in b.keys(platform) {
                let Some((mods, key)) = keymap::parse(spec) else { continue };
                let canon = canonical_key(&gpui_key(key));
                let hit = keymap::lookup(platform, Context::Canvas, &canon, mods).map(|f| f.id);
                if hit == Some(b.id) {
                    ok += 1;
                } else {
                    broken.push(format!("{}: {} -> {:?} -> {:?}", b.id, spec, canon, hit));
                }
            }
        }
        eprintln!("[audit] ok={ok} broken={}\n{}", broken.len(), broken.join("\n"));
        assert!(broken.is_empty(), "{} bindings broken", broken.len());
    }
}

#[cfg(test)]
mod keyboard_e2e_tests;
