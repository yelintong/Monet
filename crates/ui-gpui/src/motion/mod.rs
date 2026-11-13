// ============================================================================
// Motion (Beta) — `ui-egui/src/motion` 的 GPUI 移植。
//
// 本模块**只负责状态、动作与文档写入**；界面分别在 inspector / timeline / easing /
// clip_popover 里，控件一律复用 `crate::widgets`：`template::Input`（带插槽的输入框）、
// `PanelSection` + `field_row`（统一网格）、`Popover`（窗口外壳）、`IconButton`、`Menu`。
// `widgets/` 是 FROZEN，本模块不改其中任何文件。
// ============================================================================
//! Motion (Beta) mode: the animation timeline under the canvas and its inspector.
//!
//! The data model is `figcraft_doc::anim::motion`; the engine exposes the same commands
//! (`motion.keyframe` / `motion.preset` / `motion.timeline`). This module keeps the UI state
//! ([`MotionUi`]), the action queue ([`Queue`] / [`Act`]) and the document edits ([`apply`]),
//! so every panel is a thin retained view over it — exactly the egui split.

pub mod clip_popover;
pub mod easing;
pub mod host;
pub mod inspector;
pub mod keyframe_field;
pub mod timeline;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use figcraft_doc::anim::motion::{self, AnimProperty, AnimValue, Clip, ClipKind, ClipMode, Ease, TimelineSettings};
use figcraft_doc::{Document, NodeId};
use gpui::{Bounds, Pixels, Point};

use crate::app::CanvasApp;

thread_local! {
    /// 当前跨帧手势（clip/layer/keyframe 拖拽、easing 手柄）是否因文档只读 / 预览而被拒。
    /// `Act::BeginEdit` / `Act::EaseBegin` 时按 `begin_interaction` 结果设置，
    /// `EndEdit` / `EaseEnd` 清除；`edit_open` 据此丢弃该手势的每一笔写入。
    /// 与 `timeline.rs` 的 `GRABBED` / `BAR_ORIGIN` 同款单指针 thread_local 约定。
    /// PR#1624 gate #3：只读文档的拖拽编辑也必须整段丢弃、不记 undo。
    static GESTURE_BLOCKED: RefCell<bool> = const { RefCell::new(false) };
}

/// Height of the docked timeline (egui `timeline::HEIGHT`).
pub const HEIGHT: f32 = 309.0;
/// The smallest the docked timeline may be dragged to (egui `timeline::MIN_H`).
pub const MIN_H: f32 = 160.0;
/// Width the right panel grows to while Motion is open (egui `inspector::WIDTH`).
pub const INSPECTOR_W: f32 = 413.0;

// ---------------------------------------------------------------- 选择模型

/// A keyframe the timeline has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedKey {
    pub node: NodeId,
    pub property: AnimProperty,
    pub time_ms: u32,
}

/// 缓动编辑器的写入目标：关键帧段，或动画 clip（egui 的 easing editor 在两处复用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EaseTarget {
    Key(SelectedKey),
    Clip(NodeId, u32),
}

/// A property track picked in the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedTrack {
    pub node: NodeId,
    pub property: AnimProperty,
}

/// One selectable item in the Motion timeline. A layer carries all its children; Shift extends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelItem {
    Layer(NodeId),
    Track(NodeId, AnimProperty),
    Key(NodeId, AnimProperty, u32),
    Segment(NodeId, AnimProperty, u32),
    Clip(NodeId, u32),
}

impl SelItem {
    pub fn node(self) -> NodeId {
        match self {
            SelItem::Layer(n) | SelItem::Track(n, _) | SelItem::Key(n, _, _) | SelItem::Segment(n, _, _) | SelItem::Clip(n, _) => n,
        }
    }
}

/// Which part of a clip bar (or layer summary) a drag grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipEdge {
    Move,
    TrimStart,
    TrimEnd,
}

/// A clip drag in progress: recomputed from `orig` + delta each move, so no drift.
#[derive(Clone, Debug)]
pub struct ClipDrag {
    pub node: NodeId,
    pub id: u32,
    pub edge: ClipEdge,
    pub origin_x: f32,
    pub orig: Clip,
}

/// A layer-summary drag: the layer's whole motion as it was when the drag began.
#[derive(Clone, Debug)]
pub struct LayerDrag {
    pub node: NodeId,
    pub edge: ClipEdge,
    pub origin_x: f32,
    pub lo: u32,
    pub hi: u32,
    pub orig: motion::NodeMotion,
}

/// What an evaluated preview was built from. The key avoids re-evaluating a whole document
/// every frame: the `Arc` pointer changes on every edit, the playhead bits on every seek.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preview {
    pub doc: usize,
    pub page: NodeId,
    pub playhead: u64,
}

/// Timeline state kept between frames (egui `MotionUi`, with egui types replaced by GPUI ones).
#[derive(Default)]
pub struct MotionUi {
    pub timeline_h: f32,
    pub playhead_ms: f64,
    pub playing: bool,
    /// Free-running playback clock, mapped through `TimelineSettings::playback_time`.
    pub clock_ms: f64,
    pub last_tick: Option<Instant>,
    pub selection: BTreeSet<SelItem>,
    pub selected: Option<SelectedKey>,
    pub selected_track: Option<SelectedTrack>,
    pub folded: BTreeSet<NodeId>,
    pub presets_open: bool,
    pub scroll_y: f32,
    pub scrubbing: bool,
    pub drag_duration: Option<u32>,
    pub scroll_x: f32,
    pub selected_clip: Option<(NodeId, u32)>,
    /// The clip whose floating settings window is open.
    pub clip_popover: Option<(NodeId, u32)>,
    /// Where that window was anchored (the clicked row), in window coordinates.
    pub clip_anchor: Option<Bounds<Pixels>>,
    /// A dragged settings window keeps its own top-left.
    pub clip_at: Option<Point<Pixels>>,
    /// 帧间线（segment）的独立缓动浮窗：编辑**这一段**的曲线（`EaseTarget::Key`），
    /// 与 clip 预设窗完全分开（用户 2026-11-12：帧间线设置是「单独的」，不与动画 clip 联动）。
    pub segment_ease: Option<SelectedKey>,
    /// clip 预设窗里 Easing 行的眼睛：折叠/展开**下方曲线编辑器**（不是 clip 本身的显隐）。
    /// 在集合里 = 该 clip 的曲线被折叠；默认（不在集合）= 展开。按 clip 记住各自状态。
    pub easing_collapsed: BTreeSet<(NodeId, u32)>,
    pub easing_target: bool,
    pub preview: Option<(Preview, Arc<Document>)>,
    pub preview_doc: Option<Arc<Document>>,
    pub clip_drag: Option<ClipDrag>,
    pub layer_drag: Option<LayerDrag>,
    /// A press on the ruler / a keyframe drag keeps the canvas previewing.
    pub seek_drag: bool,
}

impl MotionUi {
    /// Advance the playback clock. Returns true when the playhead changed (a repaint is due).
    pub fn advance(&mut self, settings: TimelineSettings) -> bool {
        if !self.playing {
            self.last_tick = None;
            return false;
        }
        let now = Instant::now();
        let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f64() * 1000.0).unwrap_or(0.0);
        self.last_tick = Some(now);
        self.clock_ms += dt;
        let next = settings.playback_time(self.clock_ms as f32) as f64;
        let changed = (next - self.playhead_ms).abs() > 0.01;
        self.playhead_ms = next;
        if settings.finished(self.clock_ms as f32) {
            self.playing = false;
            self.last_tick = None;
        }
        changed
    }

    /// Rebuild the evaluated document whenever the source document or playhead changed.
    pub fn refresh_preview(&mut self, doc: &Arc<Document>, page: NodeId) {
        let key = Preview { doc: Arc::as_ptr(doc) as usize, page, playhead: self.playhead_ms.to_bits() };
        if self.preview.as_ref().is_some_and(|(p, _)| *p == key) {
            return;
        }
        let evaluated = Arc::new(motion::evaluate(doc, page, self.playhead_ms));
        self.preview_doc = Some(evaluated.clone());
        self.preview = Some((key, evaluated));
    }
}

// ---------------------------------------------------------------- 动作队列

/// Everything a Motion widget can ask for. Widgets live in a retained tree and only get
/// `&mut App` in their callbacks, so they push an [`Act`] and the root view drains it at the
/// top of the next frame (the same `CmdQueue` pattern the right panel uses).
#[derive(Clone, Debug)]
pub enum Act {
    Fold(NodeId),
    FoldAll,
    Click(SelItem, bool),
    ClearSelection,
    AddKey(NodeId, AnimProperty, u32),
    AddKeyAtPlayhead,
    MoveKey(SelectedKey, u32),
    ToggleKey(NodeId, AnimProperty),
    SetLoop(bool),
    /// 0 = 循环切档（24/30/60），其它 = 指定 fps。
    SetFps(u32),
    Play,
    Seek(u32),
    SetDuration(u32),
    /// 整帧复制（右键时间轴某一时刻）：把 `from` 时刻所有 track 的真实 key（含 easing）
    /// 复制到播放头。走 engine `motion.keyframeCopy`，一个 undo 步。用户 2026-11-12 D2。
    CopyFrame { from: u32 },
    /// 总时长手柄拖动中：实时把快照写进 `drag_duration`（带 snap + 滚动区），落点才真正写文档。
    SetDragDuration(Option<u32>),
    /// 总时长手柄抬起：把 `drag_duration` 提交为真正时长（一次 undo 步），并清掉滚动/快照。
    CommitDuration,
    ToggleVisible(NodeId),
    ToggleTrackVisible(NodeId, AnimProperty),
    DeleteClip(NodeId, u32),
    ToggleClipVisible(NodeId, u32),
    SelectClip(NodeId, u32),
    OpenClipPopover(NodeId, u32),
    CloseClipPopover,
    /// 打开帧间线的**独立缓动浮窗**（只 Curve/Spring 曲线，不带 clip 的 preset 字段）。
    OpenSegmentEase(SelectedKey),
    CloseSegmentEase,
    /// Easing 行的眼睛：折叠/展开该 clip 下方的曲线编辑器（纯 UI，不写文档）。
    ToggleClipEasingFold(NodeId, u32),
    /// 打开时记下触发行的顶边 y（窗口坐标），卡片据此与按钮上边对齐（egui `clip_anchor`）。
    SetClipAnchorY(f32),
    MoveClipPopover(Point<Pixels>),
    SetPresetsOpen(bool),
    AddClip(NodeId, ClipKind),
    AddPreset(NodeId, String),
    SetEase(Ease),
    /// 缓动编辑器：一次离散改动（tab / Reset / 数值 / 预设），自带一次 undo。
    EaseSet(EaseTarget, Ease),
    /// 拖拽手柄：Begin → 多次 Live → End 合并成一个 undo step（egui `EaseChange`）。
    EaseBegin,
    EaseLive(EaseTarget, Ease),
    EaseEnd,
    SetEasingTarget(bool),
    CommitValue { node: NodeId, property: AnimProperty, idx: usize, value: f64 },
    SetClip(NodeId, u32, Box<Clip>),
    ScrollY(f32),
    ScrollX(f32),
    /// 按下即选中、且**不会**因为「再点一次已选中的项」而取消选中。egui 的 Click 只在抬起时
    /// 触发，所以拖起点不会被误取消；GPUI 是按下派发，必须用这个变体保住拖拽起点。
    EnsureSelected(SelItem),
    /// 时间轴顶边拖出来的高度（egui `app/frame.rs::timeline_edge`）。只改 UI 状态，不进 undo。
    SetTimelineHeight(f32),
    SetScrubbing(bool),
    /// A press on a clip bar / layer summary starts a drag; `orig` is the untouched model, so
    /// every move recomputes from it (never accumulate drift, egui `ClipDrag` / `LayerDrag`).
    BeginClipDrag { node: NodeId, id: u32, edge: ClipEdge, origin_x: f32, orig: Box<Clip> },
    BeginLayerDrag { node: NodeId, edge: ClipEdge, origin_x: f32, lo: u32, hi: u32, orig: Box<motion::NodeMotion> },
    /// A pointer move while a drag is in progress; `pxms` is the timeline's px↔ms scale.
    DragMove { x: f32, pxms: f32 },
    EndDrag,
    BeginEdit(&'static str),
    EndEdit,
    /// The header's X: leave Motion mode. Handled by the root view (it owns the panel width),
    /// not by [`apply`], which only sees the session.
    CloseMotion,
}

/// A per-frame action queue. `push` wakes the root view so a queued action is never lost.
#[derive(Clone)]
pub struct Queue {
    items: Rc<RefCell<Vec<Act>>>,
    wake: Rc<dyn Fn()>,
}

impl Queue {
    pub fn new(wake: Rc<dyn Fn()>) -> Self {
        Self { items: Rc::new(RefCell::new(Vec::new())), wake }
    }

    pub fn push(&self, act: Act) {
        self.items.borrow_mut().push(act);
        (self.wake)();
    }

    pub fn drain(&self) -> Vec<Act> {
        std::mem::take(&mut *self.items.borrow_mut())
    }

    /// 请求一帧但不排动作（hover tip / 光标这类纯视觉刷新用）。
    pub fn wake(&self) {
        (self.wake)();
    }
}

/// Open / close Motion mode, widening the right panel to the inspector while it is open.
pub fn set_open(ui: &mut MotionUi, right_w: &mut f32, saved: &mut Option<f32>, open: bool, current: bool) -> bool {
    if open == current {
        return current;
    }
    if open {
        *saved = Some(*right_w);
        *right_w = INSPECTOR_W;
        ui.playing = false;
        ui.last_tick = None;
    } else if let Some(w) = saved.take() {
        *right_w = w;
    }
    open
}

// ---------------------------------------------------------------- 文档读取

/// The animation scope of the current selection: its top-level frame, or the page when nothing
/// is selected (or the selection isn't inside a frame). One frame = one animation group: every
/// component under it shares this scope's timeline, playhead and duration (用户 2026-11-12).
/// Falls back to the page so page-level motion keeps working unchanged.
pub fn scope_of(doc: &Document, selection: &[NodeId], page: NodeId) -> NodeId {
    // 选中节点在画框里 → 那个顶层画框（框内共用一条时间轴）；**不在任何画框里 → 它自己**
    // （用户 2026-11-12：把组件拖出画框后，动画要跟着它单独出来，不能再并进原来那一坨）。
    // 只有没有选中时才回退到页面。
    match selection.first() {
        Some(id) if doc.node(*id).is_some() => doc.top_level_frame(*id).unwrap_or(*id),
        _ => page,
    }
}

/// The selected scope's timeline settings, clamped. A frame with no entry of its own inherits
/// its page's (the lookup lives in `Document::timeline_settings_for`, shared with export).
pub fn settings_of(doc: &Document, scope: NodeId) -> TimelineSettings {
    doc.timeline_settings_for(scope).sanitized()
}

/// The displayed value of `property` at the playhead (`None` = Mixed / colour).
///
/// Rotation 以 **0 为基准**（相对设计态的增量）：引擎的 Rotation 轨道存的是绝对角，
/// 无轨道时 `value_at` 回退到 `base_rotation_deg`（设计画布里转过的角度）。若直接显示，
/// 一打开动画就会看到设计角而不是 0（用户 2026-11-12）。这里减去设计角；写回由
/// [`commit_value`] 成对加回，引擎语义不变。
pub fn value_of(doc: &Document, node: NodeId, property: AnimProperty, idx: usize, time_ms: f64) -> Option<f64> {
    let v = doc.node(node).and_then(|n| motion::value_at(n, property, time_ms))?;
    match v {
        AnimValue::Scalar(x) => {
            let base = if matches!(property, AnimProperty::Rotation) {
                doc.node(node).map(motion::base_rotation_deg).unwrap_or(0.0)
            } else {
                0.0
            };
            Some((x - base) * property.display_factor())
        }
        AnimValue::Vec2(p) => Some(if idx == 0 { p.x } else { p.y }),
        AnimValue::Color(_) => None,
    }
}

/// Whether a keyframe sits exactly at `time` on `property`.
pub fn has_key(doc: &Document, node: NodeId, property: AnimProperty, time: u32) -> bool {
    doc.node(node).and_then(|n| n.motion.track(property)).is_some_and(|tr| tr.key_at(time).is_some())
}

/// 整条属性轨道上是否**存在任意关键帧**（与播放头位置无关）。右侧字段的插针菱形据此常亮
/// （空心），只有播放头正好停在某个帧上时才变实心（用户 2026-11-12「轨道加了帧就亮起」）。
pub fn has_any_key(doc: &Document, node: NodeId, property: AnimProperty) -> bool {
    doc.node(node).and_then(|n| n.motion.track(property)).is_some_and(|tr| !tr.keys.is_empty())
}

/// The nodes on this page that carry any animation (egui `motion::animated_nodes`).
pub fn animated_nodes(doc: &Document, page: NodeId) -> Vec<NodeId> {
    motion::animated_nodes(doc, page)
}

// ---------------------------------------------------------------- 文档写入

/// 一条 motion 命令 = 一个 undo step，且走 engine 命令注册表（architecture.md「one path」）。
/// `begin_interaction` 定义 undo 边界并做只读 / 预览门控（gate #3），`execute` 执行文档修改，
/// `commit` 压栈。`EditCtx` 的直写 helper 逐步收敛到这里。
fn run_cmd(app: &mut CanvasApp, label: &str, cmd: &str, params: serde_json::Value) -> Option<serde_json::Value> {
    if app.session.begin_interaction(label).is_err() {
        return None;
    }
    GESTURE_BLOCKED.with(|b| *b.borrow_mut() = false);
    let out = app.session.execute(cmd, &params).ok();
    let _ = app.session.commit_interaction();
    out
}

fn set_duration(app: &mut CanvasApp, scope: NodeId, duration: u32) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "page": scope.0, "duration": duration }));
}

fn set_loop(app: &mut CanvasApp, scope: NodeId, looping: bool) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "page": scope.0, "loop": looping }));
}

/// 写当前作用域（画框）的 fps（引擎 `motion.timeline {fps}`，影响逐帧步长与吸附）。
fn set_fps(app: &mut CanvasApp, scope: NodeId, fps: u32) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "page": scope.0, "fps": fps }));
}

/// 收敛到 engine 命令 `motion.keyframe`（PR#1624）：命令内部取 `keyframed_value_at`
/// 作为新 key 的值，与本地旧闭包等价，且统一了 undo 边界与只读门控。
pub fn add_key(app: &mut CanvasApp, node: NodeId, property: AnimProperty, time: u32) {
    run_cmd(app, "Add keyframe", "motion.keyframe", serde_json::json!({
        "ids": [node.0], "property": property.name(), "time": time,
    }));
}

pub fn remove_key(app: &mut CanvasApp, node: NodeId, property: AnimProperty, time: u32) {
    run_cmd(app, "Delete keyframe", "motion.keyframeDelete", serde_json::json!({
        "node": node.0, "property": property.name(), "time": time,
    }));
}

pub fn toggle_key(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, property: AnimProperty) {
    let time = ui.playhead_ms.max(0.0) as u32;
    let doc = app.session.doc().ok();
    let on = doc.is_some_and(|d| has_key(d, node, property, time));
    if on {
        remove_key(app, node, property, time);
    } else {
        add_key(app, node, property, time);
    }
}

/// 收敛到 engine 命令 `motion.keyframeMove`（PR#1624）：命令把目标时刻夹在相邻 key 之间，
/// 保证两个 key 永不共享同一时刻（本地旧版无夹紧，会拖出编辑器无法按时寻址的重叠 key）。
/// 拖拽手势的 undo 边界仍由 `Act::BeginEdit`/`EndEdit` 的 interaction 拥有，故用 execute 直发。
pub fn move_key(app: &mut CanvasApp, key: SelectedKey, time: u32) {
    let _ = app.session.execute("motion.keyframeMove", &serde_json::json!({
        "node": key.node.0, "property": key.property.name(), "time": key.time_ms, "to": time,
    }));
}

/// Writes a typed value into `property` at the playhead (the inspector's number fields).
/// Goes through `motion.autoKey` (Figma/desigma semantics): if the property already has a track,
/// a key is set at the playhead; otherwise the node's static property changes and no animation is
/// invented. The current value (key at playhead, else the static base) seeds `next` so a static
/// edit is an absolute value, not relative to zero. `value` shape is validated by the command.
pub fn commit_value(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, property: AnimProperty, idx: usize, value: f64) {
    let time = ui.playhead_ms.max(0.0) as u32;
    let current = app
        .session
        .doc()
        .ok()
        .and_then(|doc| doc.node(node))
        .and_then(|n| n.motion.track(property))
        .and_then(|tr| tr.key_at(time))
        .map(|k| k.value)
        .or_else(|| app.session.doc().ok().and_then(|doc| doc_node_value(doc, node, property, time as f64)));
    // 与 `value_of` 成对：右栏 Rotation 显示的是"相对设计角的增量、0 基准"，
    // 而引擎轨道 / `motion.autoKey` 收的是绝对角，写回时必须加回设计角。
    let base_rot = if matches!(property, AnimProperty::Rotation) {
        app.session.doc().ok().and_then(|doc| doc.node(node)).map(motion::base_rotation_deg).unwrap_or(0.0)
    } else {
        0.0
    };
    let next = match current {
        Some(AnimValue::Vec2(p)) => AnimValue::Vec2(if idx == 0 {
            figcraft_geom::Vec2::new(value, p.y)
        } else {
            figcraft_geom::Vec2::new(p.x, value)
        }),
        Some(AnimValue::Color(c)) => AnimValue::Color(c),
        None if matches!(property, AnimProperty::Position | AnimProperty::Scale) => AnimValue::Vec2(figcraft_geom::Vec2::new(
            if idx == 0 { value } else { 0.0 },
            if idx == 1 { value } else { 0.0 },
        )),
        _ => AnimValue::Scalar((value / property.display_factor().max(1e-6)) + base_rot),
    };
    let Ok(value) = serde_json::to_value(next) else { return };
    run_cmd(app, "Edit animation value", "motion.autoKey", serde_json::json!({
        "ids": [node.0], "property": property.name(), "time": time, "value": value,
    }));
}

/// Borrow-safe helper: `commit_value` already holds `&mut Document`, so re-read through the node
/// id rather than trying to keep a second borrow alive.
fn doc_node_value(doc: &Document, node: NodeId, prop: AnimProperty, time: f64) -> Option<AnimValue> {
    doc.node(node).and_then(|n| motion::keyframed_value_at(n, prop, time))
}

/// 把 ease 写进关键帧段或 clip，走 engine 命令（PR#1624）：
///   - Key 目标 → `motion.keyframeEase {node, property, time, ease}`；
///   - Clip 目标 → 读整条 clip、改 easing、发 `motion.clipUpdate`。
/// `in_gesture=true`（EaseLive 拖拽跨帧）时不 begin/commit，undo 由外层 interaction 拥有，
/// 并受 `GESTURE_BLOCKED` 门控；`false`（EaseSet 一次性）走 `run_cmd` 自带 undo 边界。
fn write_ease(app: &mut CanvasApp, target: EaseTarget, ease: Ease, in_gesture: bool) {
    if in_gesture && GESTURE_BLOCKED.with(|b| *b.borrow()) {
        return
    }
    let Ok(ease_json) = serde_json::to_value(ease) else { return };
    let (cmd, params) = match target {
        EaseTarget::Key(k) => (
            "motion.keyframeEase",
            serde_json::json!({"node": k.node.0, "property": k.property.name(), "time": k.time_ms, "ease": ease_json}),
        ),
        EaseTarget::Clip(node, id) => {
            let Some(mut clip) = app.session.doc().ok().and_then(|d| d.node(node)).and_then(|n| n.motion.clip(id)).cloned() else {
                return
            };
            clip.easing = ease;
            let Ok(clip) = serde_json::to_value(clip) else { return };
            ("motion.clipUpdate", serde_json::json!({"node": node.0, "id": id, "clip": clip}))
        }
    };
    if in_gesture {
        let _ = app.session.execute(cmd, &params);
    } else {
        run_cmd(app, "Ease curve", cmd, params);
    }
}

/// 收敛到 engine 命令 `motion.keyframeEase`（PR#1624）：设置段起点帧的缓动。
pub fn set_ease(app: &mut CanvasApp, key: SelectedKey, ease: Ease) {
    let Ok(ease) = serde_json::to_value(ease) else { return };
    run_cmd(app, "Ease curve", "motion.keyframeEase", serde_json::json!({
        "node": key.node.0, "property": key.property.name(), "time": key.time_ms, "ease": ease,
    }));
}

/// 收敛到 engine 命令 `motion.clip`（PR#1624）：命令内部取 next_clip_id 并 normalize。
pub fn add_clip(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, kind: ClipKind) {
    let start = ui.playhead_ms.max(0.0) as u32;
    run_cmd(app, "Add animation", "motion.clip", serde_json::json!({
        "kind": kind.name(), "mode": "In", "time": start, "ids": [node.0],
    }));
}

/// 收敛到 engine 命令 `motion.preset`（本地已有；PR#1624 的同名命令内部就是这段逻辑）。
pub fn add_preset(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, name: &str) {
    let start = ui.playhead_ms.max(0.0) as u32;
    run_cmd(app, "Add animation preset", "motion.preset", serde_json::json!({
        "preset": name, "at": start, "ids": [node.0],
    }));
}

/// 收敛到 `motion.clipDelete`（PR#1624）。
pub fn remove_clip(app: &mut CanvasApp, node: NodeId, id: u32) {
    run_cmd(app, "Remove animation", "motion.clipDelete", serde_json::json!({"node": node.0, "id": id}));
}

/// 收敛到 `motion.clipUpdate`（PR#1624 inspector::toggle_clip_visible）：读整条 clip、
/// 翻转 visible、序列化整条发命令（命令侧 sanitize + normalize）。
pub fn toggle_clip_visible(app: &mut CanvasApp, node: NodeId, id: u32) {
    let Some(mut clip) = app.session.doc().ok().and_then(|d| d.node(node)).and_then(|n| n.motion.clip(id)).cloned() else {
        return
    };
    clip.visible = !clip.visible;
    let Ok(clip) = serde_json::to_value(clip) else { return };
    run_cmd(app, "Show/hide animation", "motion.clipUpdate", serde_json::json!({"node": node.0, "id": id, "clip": clip}));
}

/// 收敛到 `motion.clipUpdate`（PR#1624 clip_popover 的唯一写入口）。
pub fn set_clip(app: &mut CanvasApp, node: NodeId, id: u32, clip: Clip) {
    let Ok(clip) = serde_json::to_value(clip) else { return };
    run_cmd(app, "Edit animation clip", "motion.clipUpdate", serde_json::json!({"node": node.0, "id": id, "clip": clip}));
}

/// 拖拽跨帧把整条 clip 写回：走 `motion.clipUpdate` 命令（PR#1624），但**不** begin/commit——
/// undo 边界由 `Act::BeginEdit`/`EndEdit` 的 interaction 拥有，整段拖拽仍是一步。
/// `GESTURE_BLOCKED`（gate #3）为真时（只读 / 预览）丢弃这一笔。
fn clip_dragged(app: &mut CanvasApp, ui: &mut MotionUi, node: NodeId, id: u32, edge: ClipEdge, dx_ms: i64) {
    if GESTURE_BLOCKED.with(|b| *b.borrow()) {
        return
    }
    let Some(drag) = ui.clip_drag.clone() else { return };
    let orig = &drag.orig;
    let mut c = orig.clone();
    match edge {
        ClipEdge::Move => c.delay_ms = (orig.delay_ms as i64 + dx_ms).max(0) as u32,
        ClipEdge::TrimStart => {
            let end = (orig.delay_ms + orig.duration_ms) as i64;
            let start = (orig.delay_ms as i64 + dx_ms).clamp(0, end - 1);
            c.delay_ms = start as u32;
            c.duration_ms = (end - start).max(1) as u32;
        }
        ClipEdge::TrimEnd => c.duration_ms = (orig.duration_ms as i64 + dx_ms).max(1) as u32,
    }
    c.id = id;
    c.sanitize();
    let Ok(clip) = serde_json::to_value(c) else { return };
    let _ = app.session.execute("motion.clipUpdate", &serde_json::json!({"node": node.0, "id": id, "clip": clip}));
}

fn layer_dragged(app: &mut CanvasApp, ui: &mut MotionUi, node: NodeId, edge: ClipEdge, dx_ms: i64) {
    if GESTURE_BLOCKED.with(|b| *b.borrow()) {
        return
    }
    let Some(drag) = ui.layer_drag.clone() else { return };
    let (lo, hi) = (drag.lo as f64, drag.hi.max(drag.lo + 1) as f64);
    // 算出整条 motion（纯计算，不碰 doc），收敛到 `motion.replace` 命令（PR#1624）。
    // undo 边界由 BeginEdit/EndEdit 的 interaction 拥有，故直发 execute 不 begin/commit。
    let mut m = drag.orig.clone();
    {
            match edge {
                ClipEdge::Move => {
                    for tr in &mut m.tracks {
                        for k in &mut tr.keys {
                            k.time_ms = (k.time_ms as i64 + dx_ms).max(0) as u32;
                        }
                    }
                    for c in &mut m.clips {
                        c.delay_ms = (c.delay_ms as i64 + dx_ms).max(0) as u32;
                    }
                }
                ClipEdge::TrimStart | ClipEdge::TrimEnd => {
                    let denom = (hi - lo).max(1.0);
                    let (anchor, scale) = if edge == ClipEdge::TrimEnd {
                        let new_hi = (hi + dx_ms as f64).max(lo + 1.0);
                        (lo, (new_hi - lo) / denom)
                    } else {
                        let new_lo = (lo + dx_ms as f64).clamp(0.0, hi - 1.0);
                        (hi, (hi - new_lo) / denom)
                    };
                    for tr in &mut m.tracks {
                        for k in &mut tr.keys {
                            k.time_ms = (anchor + (k.time_ms as f64 - anchor) * scale).max(0.0) as u32;
                        }
                    }
                    for c in &mut m.clips {
                        c.delay_ms = (anchor + (c.delay_ms as f64 - anchor) * scale).max(0.0) as u32;
                        c.duration_ms = ((c.duration_ms as f64) * scale).max(1.0) as u32;
                    }
                }
            }
        m.normalize();
        let Ok(motion) = serde_json::to_value(m) else { return };
        let _ = app.session.execute("motion.replace", &serde_json::json!({"node": node.0, "motion": motion}));
    }
}

// ---------------------------------------------------------------- 动作落地

/// Reconcile the single-item caches the inspector / easing panel read from `selection`.
fn reconcile(ui: &mut MotionUi) {
    ui.selected = ui.selection.iter().find_map(|s| match s {
        SelItem::Key(n, p, t) | SelItem::Segment(n, p, t) => Some(SelectedKey { node: *n, property: *p, time_ms: *t }),
        _ => None,
    });
    ui.selected_track = ui.selection.iter().find_map(|s| match s {
        SelItem::Track(n, p) => Some(SelectedTrack { node: *n, property: *p }),
        _ => None,
    });
    ui.selected_clip = ui.selection.iter().find_map(|s| match s {
        SelItem::Clip(n, c) => Some((*n, *c)),
        _ => None,
    });
}

/// Apply every queued action: UI state changes here, document writes through the helpers above.
/// Returns whether the view should repaint.
pub fn apply(app: &mut CanvasApp, ui: &mut MotionUi, acts: Vec<Act>) -> bool {
    if acts.is_empty() {
        return false;
    }
    let doc = app.session.doc().ok().cloned();
    // One animation group = one frame. All timeline settings writes and the FoldAll scope go to
    // the selected component's top-level frame, falling back to the page when none is selected.
    let page = doc.as_ref().map(|d| scope_of(d, &app.session.selection(), app.session.ds().ok().map(|s| s.page).unwrap_or(figcraft_doc::NodeId(0))));
    for act in acts {
        match act {
            Act::Fold(n) => {
                if !ui.folded.remove(&n) {
                    ui.folded.insert(n);
                }
            }
            Act::FoldAll => {
                if ui.folded.is_empty() {
                    if let (Some(d), Some(pg)) = (&doc, page) {
                        ui.folded.extend(animated_nodes(d, pg));
                    }
                } else {
                    ui.folded.clear();
                }
            }
            Act::EnsureSelected(item) => {
                ui.selection.clear();
                ui.selection.insert(item);
                reconcile(ui);
                ui.easing_target = matches!(item, SelItem::Segment(..)) && ui.selected.is_some();
            }
            Act::Click(item, shift) => {
                if shift {
                    if !ui.selection.remove(&item) {
                        ui.selection.insert(item);
                    }
                } else if ui.selection.len() == 1 && ui.selection.contains(&item) {
                    ui.selection.clear();
                } else {
                    ui.selection.clear();
                    ui.selection.insert(item);
                    if let SelItem::Layer(n) = item {
                        if let Some(d) = &doc
                            && let Some(node) = d.node(n)
                        {
                            for tr in &node.motion.tracks {
                                ui.selection.insert(SelItem::Track(n, tr.property));
                                for k in &tr.keys {
                                    ui.selection.insert(SelItem::Key(n, tr.property, k.time_ms));
                                }
                            }
                            for c in &node.motion.clips {
                                ui.selection.insert(SelItem::Clip(n, c.id));
                            }
                        }
                    }
                }
                reconcile(ui);
                if matches!(item, SelItem::Segment(..)) {
                    ui.easing_target = ui.selected.is_some();
                } else {
                    ui.easing_target = false;
                }
            }
            Act::ClearSelection => {
                ui.selection.clear();
                reconcile(ui);
                ui.easing_target = false;
            }
            Act::AddKey(n, p, t) => add_key(app, n, p, t),
            Act::CopyFrame { from } => {
                // Copy every real key at `from` onto the playhead, for the selected layers.
                // `motion.keyframeCopy` handles value+easing fidelity and one-step undo; `run_cmd`
                // wraps it in a single interaction (consistent with the other keyframe edits).
                let ids = app.session.selection();
                if !ids.is_empty() {
                    let to = ui.playhead_ms.max(0.0) as u32;
                    run_cmd(app, "Copy frame", "motion.keyframeCopy", serde_json::json!({
                        "from": from, "to": to, "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>(),
                    }));
                }
            }
            Act::AddKeyAtPlayhead => {
                let t = ui.playhead_ms.max(0.0) as u32;
                let ids = app.session.selection();
                let props: Vec<AnimProperty> = doc
                    .as_ref()
                    .and_then(|d| ids.first().and_then(|id| d.node(*id)))
                    .map(|n| n.motion.tracks.iter().map(|tr| tr.property).collect())
                    .unwrap_or_else(|| vec![AnimProperty::Position]);
                for id in ids {
                    for p in &props {
                        let _ = app.session.execute("motion.keyframe", &serde_json::json!({ "property": p.name(), "time": t, "ids": [id] }));
                    }
                }
            }
            Act::MoveKey(k, t) => {
                move_key(app, k, t);
                // 选中项跟着关键帧一起搬。否则拖完以后高亮留在旧时间上、缓动面板指向一个
                // 已经不存在的键（`ui.selected` 是 inspector / easing 的唯一来源）。
                if ui.selection.remove(&SelItem::Key(k.node, k.property, k.time_ms)) {
                    ui.selection.insert(SelItem::Key(k.node, k.property, t));
                }
                if ui.selection.remove(&SelItem::Segment(k.node, k.property, k.time_ms)) {
                    ui.selection.insert(SelItem::Segment(k.node, k.property, t));
                }
                if ui.selected == Some(k) {
                    ui.selected = Some(crate::motion::SelectedKey { time_ms: t, ..k });
                }
            }
            Act::ToggleKey(n, p) => toggle_key(app, ui, n, p),
            Act::SetLoop(on) => {
                if let Some(pg) = page {
                    set_loop(app, pg, on);
                }
            }
            Act::SetFps(v) => {
                let cur = doc.as_ref().and_then(|d| page.map(|pg| settings_of(d, pg).fps)).unwrap_or(60);
                let next = if v == 0 { match cur { 24 => 30, 30 => 60, _ => 24 } } else { v.clamp(1, figcraft_doc::anim::motion::TimelineSettings::MAX_FPS) };
                if let Some(pg) = page {
                    set_fps(app, pg, next);
                }
            }
            Act::Play => {
                ui.playing = !ui.playing;
                ui.last_tick = None;
                if ui.playing {
                    ui.clock_ms = ui.playhead_ms;
                }
            }
            Act::Seek(t) => {
                ui.playhead_ms = t as f64;
                ui.clock_ms = t as f64;
            }
            Act::SetDuration(d) => {
                if let Some(pg) = page {
                    set_duration(app, pg, d);
                }
                ui.drag_duration = None;
            }
            Act::SetDragDuration(d) => {
                ui.drag_duration = d;
                if d.is_none() {
                    ui.scroll_x = 0.0;
                }
            }
            Act::CommitDuration => {
                if let Some(ms) = ui.drag_duration {
                    if let Some(pg) = page {
                        set_duration(app, pg, ms);
                    }
                }
                ui.drag_duration = None;
                ui.scroll_x = 0.0;
            }
            Act::ToggleVisible(n) => {
                // 层显隐走通用命令 `object.toggleVisible`（与 layer_tree 一致），不再闭包直写。
                run_cmd(app, "Show/hide layer", "object.toggleVisible", serde_json::json!({"ids": [n.0]}));
            }
            Act::ToggleTrackVisible(n, p) => {
                // 收敛到 `motion.track`（PR#1624）：不带 visible 参数 = 翻转。
                run_cmd(app, "Show/hide track", "motion.track", serde_json::json!({"node": n.0, "property": p.name()}));
            }
            Act::DeleteClip(n, c) => {
                remove_clip(app, n, c);
                if ui.clip_popover == Some((n, c)) {
                    ui.clip_popover = None;
                }
            }
            Act::ToggleClipVisible(n, c) => toggle_clip_visible(app, n, c),
            Act::SelectClip(n, c) => {
                ui.selected_clip = Some((n, c));
                ui.selection.clear();
                ui.selection.insert(SelItem::Clip(n, c));
                ui.easing_target = false;
            }
            Act::OpenClipPopover(n, c) => {
                let open = ui.clip_popover == Some((n, c));
                ui.clip_popover = if open { None } else { Some((n, c)) };
                ui.selected_clip = Some((n, c));
                ui.easing_target = false;
                // 与帧间线浮窗互斥：开 clip 窗就关段窗。
                ui.segment_ease = None;
                // 每次**重新打开**都回到锚点：顶边与触发的那一条动画行齐高、右缘贴右栏左缘。
                // 不清的话上次的拖动落点 `clip_at` 会让 `Open::At` 盖掉 `Open::Anchor`。
                if !open {
                    ui.clip_at = None;
                }
            }
            Act::CloseClipPopover => ui.clip_popover = None,
            Act::OpenSegmentEase(k) => {
                // 段缓动窗是**独立**的：同一时刻只开一个浮窗，开段窗就关 clip 窗。
                let open = ui.segment_ease == Some(k);
                ui.segment_ease = if open { None } else { Some(k) };
                ui.clip_popover = None;
                ui.clip_at = None;
                // 浮窗编辑，不切右栏 Easing 页（那是没有浮窗时的旧路径）。
                ui.easing_target = false;
                // 锚点沿用「右栏左缘 − 8 / 触发点上方」：SetClipAnchorY 由 timeline 点击前 push。
            }
            Act::CloseSegmentEase => ui.segment_ease = None,
            Act::ToggleClipEasingFold(n, c) => {
                // 眼睛折叠/展开该 clip 的曲线编辑器：纯 UI 状态，不碰 clip.visible。
                if !ui.easing_collapsed.remove(&(n, c)) {
                    ui.easing_collapsed.insert((n, c));
                }
            }
            Act::SetClipAnchorY(y) => {
                // egui `inspector.rs:156`: 记下触发行的 rect；这里只需要顶边 y，
                // x 由 `clip_popover::panel` 按「右栏左缘 − 8」给出。清除上次的拖动落点，
                // 否则 `Open::At` 会盖掉锚点、把卡片停在时间轴的点击处。
                ui.clip_anchor = Some(gpui::Bounds {
                    origin: gpui::Point { x: gpui::px(0.0), y: gpui::px(y) },
                    size: gpui::size(gpui::px(0.0), gpui::px(0.0)),
                });
                ui.clip_at = None;
            }
            Act::MoveClipPopover(p) => ui.clip_at = Some(p),
            Act::SetPresetsOpen(on) => ui.presets_open = on,
            Act::AddClip(n, kind) => {
                add_clip(app, ui, n, kind);
                if let Some(d) = app.session.doc().ok()
                    && let Some(node) = d.node(n)
                    && let Some(c) = node.motion.clips.last()
                {
                    ui.clip_popover = Some((n, c.id));
                    ui.selected_clip = Some((n, c.id));
                }
            }
            Act::AddPreset(n, name) => {
                add_preset(app, ui, n, &name);
                if let Some(d) = app.session.doc().ok()
                    && let Some(node) = d.node(n)
                    && let Some(c) = node.motion.clips.last()
                {
                    ui.clip_popover = Some((n, c.id));
                    ui.selected_clip = Some((n, c.id));
                }
            }
            Act::SetEase(ease) => {
                if let Some(k) = ui.selected {
                    set_ease(app, k, ease);
                }
            }
            Act::EaseSet(target, ease) => {
                write_ease(app, target, ease, false);
            }
            Act::EaseBegin => {
                // gate #3：只读 / 预览下 begin 失败 → 标记本手势被拒，后续 EaseLive 全部丢弃。
                let blocked = app.session.begin_interaction("Ease curve").is_err();
                GESTURE_BLOCKED.with(|b| *b.borrow_mut() = blocked);
            }
            Act::EaseLive(target, ease) => {
                write_ease(app, target, ease, true);
            }
            Act::EaseEnd => {
                if !GESTURE_BLOCKED.with(|b| *b.borrow()) {
                    let _ = app.session.commit_interaction();
                }
                GESTURE_BLOCKED.with(|b| *b.borrow_mut() = false);
            }
            Act::SetEasingTarget(on) => ui.easing_target = on,
            Act::CommitValue { node, property, idx, value } => commit_value(app, ui, node, property, idx, value),
            Act::SetClip(n, c, clip) => set_clip(app, n, c, *clip),
            Act::ScrollY(y) => ui.scroll_y = y.max(0.0),
            Act::ScrollX(x) => ui.scroll_x = x.max(0.0),
            Act::SetTimelineHeight(h) => ui.timeline_h = h.max(crate::motion::MIN_H),
            Act::SetScrubbing(on) => ui.scrubbing = on,
            Act::BeginClipDrag { node, id, edge, origin_x, orig } => {
                ui.clip_drag = Some(ClipDrag { node, id, edge, origin_x, orig: *orig });
                ui.layer_drag = None;
                ui.scrubbing = true;
            }
            Act::BeginLayerDrag { node, edge, origin_x, lo, hi, orig } => {
                ui.layer_drag = Some(LayerDrag { node, edge, origin_x, lo, hi, orig: *orig });
                ui.clip_drag = None;
                ui.scrubbing = true;
            }
            Act::DragMove { x, pxms } => {
                // Recompute from `orig` every move: no accumulated drift (egui's rule).
                let to_ms = |dx_px: f32| (dx_px / pxms.max(0.0001)) as i64;
                if let Some(d) = ui.clip_drag.clone() {
                    ui.seek_drag = true;
                    clip_dragged(app, ui, d.node, d.id, d.edge, to_ms(x - d.origin_x));
                } else if let Some(d) = ui.layer_drag.clone() {
                    ui.seek_drag = true;
                    layer_dragged(app, ui, d.node, d.edge, to_ms(x - d.origin_x));
                }
            }
            Act::EndDrag => {
                ui.clip_drag = None;
                ui.layer_drag = None;
                ui.seek_drag = false;
                ui.scrubbing = false;
            }
            Act::BeginEdit(label) => {
                // gate #3：begin 失败（read-only / preview）→ 本手势所有 edit_open 都丢弃。
                let blocked = app.session.begin_interaction(label).is_err();
                GESTURE_BLOCKED.with(|b| *b.borrow_mut() = blocked);
            }
            Act::EndEdit => {
                if !GESTURE_BLOCKED.with(|b| *b.borrow()) {
                    let _ = app.session.commit_interaction();
                }
                GESTURE_BLOCKED.with(|b| *b.borrow_mut() = false);
            }
            // The root view handles it in `sync_motion` (it owns the panel width).
            Act::CloseMotion => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_the_playhead_rebuilds_the_preview_document() {
        let doc = Arc::new(Document::new("motion-preview"));
        let page = doc.pages[0];
        let mut ui = MotionUi::default();
        ui.refresh_preview(&doc, page);
        let first = ui.preview_doc.as_ref().map(|d| Arc::as_ptr(d) as usize);
        ui.playhead_ms = 500.0;
        ui.refresh_preview(&doc, page);
        let second = ui.preview_doc.as_ref().map(|d| Arc::as_ptr(d) as usize);
        assert!(first.is_some(), "the first refresh produces a document");
        assert_ne!(first, second, "a playhead change must produce a new document identity");
    }
    use crate::app::{CanvasApp, sample_doc};
    use figcraft_doc::NodeKind;

    fn app_with_node() -> (CanvasApp, NodeId) {
        let mut doc = sample_doc();
        let page = doc.first_page().expect("sample doc has a page");
        let id = doc.create(page, NodeKind::Rectangle, "Motion test", 10.0, 20.0, 100.0, 50.0).expect("create node");
        (CanvasApp::new(doc), id)
    }

    fn key_at(app: &CanvasApp, node: NodeId, prop: AnimProperty, time: u32) -> Option<AnimValue> {
        app.session
            .doc()
            .ok()
            .and_then(|d| d.node(node))
            .and_then(|n| n.motion.track(prop))
            .and_then(|tr| tr.key_at(time))
            .map(|k| k.value)
    }

    /// 用户 2026-11-12 的动画模型：画框 = 一个动画单位，框内所有组件共用一条时间轴。
    /// `scope_of` 把当前选中解析成作用域：框内组件/框本身 → 那个顶层画框；**画框外的组件 →
    /// 它自己**（拖出画框后动画跟着它独立，不再并进整页/原画框那一组）；只有空选 → 页面。
    #[test]
    fn scope_of_resolves_the_selected_frame_or_the_node_itself() {
        let mut doc = Document::new("scope");
        let page = doc.first_page().unwrap();
        let frame = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let inside = doc.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        let nested = doc.create(frame, NodeKind::Frame, "Inner", 4.0, 4.0, 40.0, 40.0).unwrap();
        let deep = doc.create(nested, NodeKind::Rectangle, "Deep", 6.0, 6.0, 8.0, 8.0).unwrap();
        let on_page = doc.create(page, NodeKind::Rectangle, "Loose", 120.0, 0.0, 8.0, 8.0).unwrap();
        // Selecting a component, a nested frame's child, or the frame itself → the top frame.
        assert_eq!(scope_of(&doc, &[inside], page), frame, "component → its top frame");
        assert_eq!(scope_of(&doc, &[deep], page), frame, "nested child → outermost frame");
        assert_eq!(scope_of(&doc, &[frame], page), frame, "frame → itself");
        // A component directly on the page is its own unit (dragged out of a frame), not the page.
        assert_eq!(scope_of(&doc, &[on_page], page), on_page, "page-level component → itself");
        assert_eq!(scope_of(&doc, &[], page), page, "empty selection → page");
    }

    /// 拖出画框后，时间轴只列这个组件自己的动画（不再拉进整页的其它动画）。
    #[test]
    fn a_component_dragged_out_of_a_frame_animates_on_its_own() {
        let mut doc = Document::new("scope");
        let page = doc.first_page().unwrap();
        let frame = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let inside = doc.create(frame, NodeKind::Rectangle, "Inner", 2.0, 2.0, 8.0, 8.0).unwrap();
        let sibling = doc.create(frame, NodeKind::Rectangle, "Sibling", 20.0, 2.0, 8.0, 8.0).unwrap();
        for id in [frame, inside, sibling] {
            let tr = doc.node_mut(id).unwrap().motion.ensure_track(AnimProperty::Position);
            tr.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(0.0, 0.0)), None);
            tr.set_key(500, AnimValue::Vec2(figcraft_geom::Vec2::new(10.0, 0.0)), None);
        }
        // 画框里时：三条轨道共用画框的时间轴。
        assert_eq!(scope_of(&doc, &[inside], page), frame);
        assert_eq!(animated_nodes(&doc, scope_of(&doc, &[inside], page)), vec![frame, inside, sibling]);
        // 把它拖到页面上（reparent），动画跟着它自己走，不再和原来那一坨同组。
        doc.reparent(inside, page, None).unwrap();
        assert_eq!(scope_of(&doc, &[inside], page), inside, "dragged-out layer is its own scope");
        assert_eq!(animated_nodes(&doc, scope_of(&doc, &[inside], page)), vec![inside], "only itself on its timeline");
        assert_eq!(animated_nodes(&doc, scope_of(&doc, &[sibling], page)), vec![frame, sibling], "the frame keeps its own group");
    }

    /// 选中画框内组件时，时间轴设置取该画框自己的；画框没单独设就继承页面。
    #[test]
    fn settings_of_follows_the_frame_scope() {
        use figcraft_doc::anim::motion::TimelineSettings;
        let mut doc = Document::new("scope");
        let page = doc.first_page().unwrap();
        let frame = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let inside = doc.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        let page_tl = TimelineSettings { duration_ms: 1000, fps: 30, looping: false };
        let frame_tl = TimelineSettings { duration_ms: 300, fps: 60, looping: true };
        doc.timelines.insert(page, page_tl);
        let scope = scope_of(&doc, &[inside], page);
        assert_eq!(settings_of(&doc, scope), page_tl, "frame with no entry inherits the page");
        doc.timelines.insert(frame, frame_tl);
        assert_eq!(settings_of(&doc, scope), frame_tl, "frame's own entry wins");
    }

    /// PR#1624 gate #3：只读 / 版本预览文档下，inspector / easing / clip 的写入必须整笔丢弃，
    /// 既不写文档也不留 undo step。以前 `edit()` 忽略 `begin_interaction` 的 Err 照样写。
    #[test]
    fn a_refusing_document_drops_motion_edits() {
        let (mut app, node) = app_with_node();
        // 造一个版本并进入只读预览（version.preview 会把 DocState 置为只读）。
        app.session.execute("file.saveVersion", &serde_json::json!({"name": "v1"})).unwrap();
        app.session.execute("version.preview", &serde_json::json!({"index": 0})).unwrap();

        let before = app.session.doc().unwrap().clone();
        apply(&mut app, &mut MotionUi::default(), vec![Act::AddKey(node, AnimProperty::Opacity, 0)]);
        apply(&mut app, &mut MotionUi::default(), vec![Act::ToggleKey(node, AnimProperty::Position)]);
        let after = app.session.doc().unwrap().clone();
        assert_eq!(before, after, "只读预览下 apply 不改变文档（连一个 undo step 都不留）");
        assert!(after.node(node).unwrap().motion.track(AnimProperty::Opacity).is_none());
    }

    /// 帧间线的独立缓动窗与 clip 预设窗**互斥**：同一时刻只开一个，且开段窗不切右栏
    /// Easing 页（`easing_target` 必须保持 false，否则又串回 preset 面板）。回归用户
    /// 2026-11-12「点两帧之间的线弹不出曲线窗」的修复。
    #[test]
    fn segment_ease_and_clip_popover_are_mutually_exclusive() {
        let (mut app, node) = app_with_node();
        let mut ui = MotionUi::default();
        let k = SelectedKey { node, property: AnimProperty::Position, time_ms: 0 };

        apply(&mut app, &mut ui, vec![Act::OpenSegmentEase(k)]);
        assert_eq!(ui.segment_ease, Some(k), "段窗打开");
        assert_eq!(ui.clip_popover, None, "开段窗必须关 clip 窗");
        assert!(!ui.easing_target, "浮窗编辑，不切右栏 Easing 页");

        // 开 clip 窗要反过来关段窗（同一时刻只一个浮窗）。
        apply(&mut app, &mut ui, vec![Act::OpenClipPopover(node, 1)]);
        assert_eq!(ui.clip_popover, Some((node, 1)), "clip 窗打开");
        assert_eq!(ui.segment_ease, None, "开 clip 窗必须关段窗");

        // 再点同一个段 = 切换关闭。
        apply(&mut app, &mut ui, vec![Act::OpenSegmentEase(k)]);
        apply(&mut app, &mut ui, vec![Act::OpenSegmentEase(k)]);
        assert_eq!(ui.segment_ease, None, "再点同一段关闭段窗");

        apply(&mut app, &mut ui, vec![Act::CloseSegmentEase]);
        assert_eq!(ui.segment_ease, None, "Close 收尾幂等");
    }

    /// 整条轨道有任意帧 → `has_any_key` 为真（右侧插针空心常亮），与播放头位置无关；
    /// `has_key` 仍只在播放头正停在帧上为真（→ 实心）。这是两个独立状态。
    #[test]
    fn has_any_key_is_independent_of_playhead() {
        let (mut app, node) = app_with_node();
        add_key(&mut app, node, AnimProperty::Position, 200);
        let doc = app.session.doc().unwrap();
        assert!(has_any_key(doc, node, AnimProperty::Position), "轨道有帧就为真");
        assert!(!has_key(doc, node, AnimProperty::Position, 0), "播放头在 t=0 不在帧上");
        assert!(has_key(doc, node, AnimProperty::Position, 200), "播放头停在帧上才为真");
        assert!(!has_any_key(doc, node, AnimProperty::Scale), "另一条轨道无帧");
    }

    /// Easing 行的眼睛只切 `easing_collapsed`（曲线区折叠），绝不写 `clip.visible`。
    #[test]
    fn easing_eye_folds_curve_without_touching_clip_visible() {
        let (mut app, node) = app_with_node();
        let mut ui = MotionUi::default();
        let mut c = Clip::new(ClipKind::Position, ClipMode::In);
        c.id = 9;
        app.session.doc_mut().unwrap().node_mut(node).unwrap().motion.clips.push(c.clone());

        assert!(!ui.easing_collapsed.contains(&(node, 9)), "默认展开");
        apply(&mut app, &mut ui, vec![Act::ToggleClipEasingFold(node, 9)]);
        assert!(ui.easing_collapsed.contains(&(node, 9)), "点眼睛折叠");
        apply(&mut app, &mut ui, vec![Act::ToggleClipEasingFold(node, 9)]);
        assert!(!ui.easing_collapsed.contains(&(node, 9)), "再点展开");
        // clip.visible 始终未被这个 act 改动。
        assert!(app.session.doc().unwrap().node(node).unwrap().motion.clips[0].visible, "眼睛不碰 clip.visible");
    }

    /// Adding / removing / toggling a keyframe writes the document through the engine's undo
    /// path, and `has_key` reads back exactly what was written (not a UI-only flag).
    #[test]
    fn keyframe_add_remove_toggle_roundtrip() {
        let (mut app, node) = app_with_node();
        let ui = MotionUi::default();
        assert!(!has_key(app.session.doc().unwrap(), node, AnimProperty::Opacity, 0));

        add_key(&mut app, node, AnimProperty::Opacity, 0);
        assert!(has_key(app.session.doc().unwrap(), node, AnimProperty::Opacity, 0));
        assert!(matches!(key_at(&app, node, AnimProperty::Opacity, 0), Some(AnimValue::Scalar(_))));

        remove_key(&mut app, node, AnimProperty::Opacity, 0);
        assert!(!has_key(app.session.doc().unwrap(), node, AnimProperty::Opacity, 0));

        toggle_key(&mut app, &ui, node, AnimProperty::Opacity);
        assert!(has_key(app.session.doc().unwrap(), node, AnimProperty::Opacity, 0), "toggle adds when absent");
        toggle_key(&mut app, &ui, node, AnimProperty::Opacity);
        assert!(!has_key(app.session.doc().unwrap(), node, AnimProperty::Opacity, 0), "toggle removes when present");
    }

    /// Figma/desigma auto-key semantics: editing a property that has NO track changes its static
    /// value and must NOT invent an animation.
    #[test]
    fn commit_value_edits_static_value_when_no_track() {
        let (mut app, node) = app_with_node();
        let ui = MotionUi { playhead_ms: 750.0, ..Default::default() };
        commit_value(&mut app, &ui, node, AnimProperty::Width, 0, 240.0);
        assert!((app.session.doc().unwrap().node(node).unwrap().width - 240.0).abs() < 1e-9, "static width written");
        assert!(app.session.doc().unwrap().node(node).unwrap().motion.track(AnimProperty::Width).is_none(), "no animation created");
    }

    /// With a track already present, the inspector's number field commits a key at the playhead,
    /// not at t=0 (PR#1624 behaviour preserved for animated properties).
    #[test]
    fn commit_value_writes_key_at_playhead() {
        let (mut app, node) = app_with_node();
        // Establish a Width track (one key at 0) so auto-key takes the animated branch.
        {
            let doc = app.session.doc_mut().unwrap();
            doc.node_mut(node).unwrap().motion.ensure_track(AnimProperty::Width).set_key(0, AnimValue::Scalar(100.0), None);
        }
        let ui = MotionUi { playhead_ms: 750.0, ..Default::default() };
        commit_value(&mut app, &ui, node, AnimProperty::Width, 0, 240.0);
        assert!(matches!(key_at(&app, node, AnimProperty::Width, 750), Some(AnimValue::Scalar(v)) if (v - 240.0).abs() < 1e-9));
        assert!(key_at(&app, node, AnimProperty::Width, 0).is_some(), "the seed key at t=0 survives");
        assert!((app.session.doc().unwrap().node(node).unwrap().width - 100.0).abs() < 1e-9, "static value untouched for animated property");
    }

    /// 整帧复制（用户 2026-11-12 D2）：Act::CopyFrame 把右键时刻所有 track 的真实 key
    /// （含 easing）复制到播放头，写的是选中层，一个 undo 步。
    #[test]
    fn copy_frame_duplicates_keys_at_playhead() {
        let (mut app, node) = app_with_node();
        let ease = Ease::bezier(0.1, 0.2, 0.3, 0.4);
        {
            let doc = app.session.doc_mut().unwrap();
            let n = doc.node_mut(node).unwrap();
            n.motion.ensure_track(AnimProperty::Opacity).set_key(0, AnimValue::Scalar(0.3), Some(ease));
            n.motion.ensure_track(AnimProperty::Width).set_key(0, AnimValue::Scalar(80.0), None);
        }
        app.session.set_selection(vec![node]);
        let mut ui = MotionUi { playhead_ms: 500.0, ..Default::default() };
        apply(&mut app, &mut ui, vec![Act::CopyFrame { from: 0 }]);
        let doc = app.session.doc().unwrap();
        let n = doc.node(node).unwrap();
        let op = n.motion.track(AnimProperty::Opacity).unwrap();
        let copied = op.key_at(500).expect("Opacity key copied to playhead");
        assert!((copied.value.scalar().unwrap() - 0.3).abs() < 1e-9, "value copied");
        assert_eq!(copied.easing, ease, "easing round-trips");
        assert!(n.motion.track(AnimProperty::Width).unwrap().key_at(500).is_some(), "every track copied");
        assert!((op.key_at(0).unwrap().value.scalar().unwrap() - 0.3).abs() < 1e-9, "source frame untouched");
    }

    /// A clip drag recomputes from the untouched `orig` on every move: two moves with the same
    /// delta must NOT accumulate drift (egui's `ClipDrag` rule).
    #[test]
    fn clip_drag_never_accumulates_drift() {
        let (mut app, node) = app_with_node();
        let mut ui = MotionUi::default();
        let mut orig = Clip::new(ClipKind::Position, ClipMode::In);
        orig.id = 7;
        orig.delay_ms = 100;
        orig.duration_ms = 300;
        {
            let doc = app.session.doc_mut().unwrap();
            let n = doc.node_mut(node).unwrap();
            n.motion.clips.push(orig.clone());
        }
        ui.clip_drag = Some(ClipDrag { node, id: 7, edge: ClipEdge::Move, origin_x: 0.0, orig: orig.clone() });
        assert_eq!(app.session.doc().unwrap().node(node).unwrap().motion.clips.len(), 1, "one clip pushed");
        assert_eq!(app.session.doc().unwrap().node(node).unwrap().motion.clips[0].id, 7, "the pushed clip carries id 7");

        clip_dragged(&mut app, &mut ui, node, 7, ClipEdge::Move, 50);
        clip_dragged(&mut app, &mut ui, node, 7, ClipEdge::Move, 50);
        let delay = app.session.doc().unwrap().node(node).unwrap().motion.clips[0].delay_ms;
        assert_eq!(delay, 150, "same delta twice must leave the same result, not 200");

        // A NEW gesture snapshots the clip as it is NOW (delay 150), not the original 100.
        // Trimming the right edge only changes duration, never delay.
        let current = app.session.doc().unwrap().node(node).unwrap().motion.clips[0].clone();
        ui.clip_drag = Some(ClipDrag { node, id: 7, edge: ClipEdge::TrimEnd, origin_x: 0.0, orig: current });
        clip_dragged(&mut app, &mut ui, node, 7, ClipEdge::TrimEnd, 80);
        let c = &app.session.doc().unwrap().node(node).unwrap().motion.clips[0];
        assert_eq!(c.delay_ms, 150);
        assert_eq!(c.duration_ms, 380);
    }

    /// Motion widens the right panel to the inspector and restores the previous width on close.
    #[test]
    fn set_open_restores_the_right_width() {
        let mut ui = MotionUi::default();
        let mut right_w = 280.0;
        let mut saved = None;
        assert!(set_open(&mut ui, &mut right_w, &mut saved, true, false));
        assert_eq!(right_w, INSPECTOR_W);
        assert_eq!(saved, Some(280.0));
        assert!(!set_open(&mut ui, &mut right_w, &mut saved, false, true));
        assert_eq!(right_w, 280.0);
        assert_eq!(saved, None);
    }

    /// The evaluated preview changes with the playhead and is cached otherwise.
    #[test]
    fn preview_is_cached_by_doc_and_playhead() {
        let (app, _node) = app_with_node();
        let mut ui = MotionUi::default();
        let (doc, page) = {
            let ds = app.session.ds().unwrap();
            (ds.doc.clone(), ds.page)
        };
        ui.refresh_preview(&doc, page);
        let first = ui.preview.as_ref().map(|(k, _)| *k).unwrap();
        ui.refresh_preview(&doc, page);
        assert_eq!(ui.preview.as_ref().map(|(k, _)| *k), Some(first), "unchanged frame reuses the cache");
        ui.playhead_ms = 500.0;
        ui.refresh_preview(&doc, page);
        assert_ne!(ui.preview.as_ref().map(|(k, _)| *k), Some(first), "a new playhead re-evaluates");
        assert!(ui.preview_doc.is_some());
    }
}

