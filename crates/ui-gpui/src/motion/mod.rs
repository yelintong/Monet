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

/// The page's timeline settings (clamped).
pub fn settings_of(doc: &Document, page: NodeId) -> TimelineSettings {
    doc.timelines.get(&page).copied().unwrap_or_default().sanitized()
}

/// The displayed value of `property` at the playhead (`None` = Mixed / colour).
pub fn value_of(doc: &Document, node: NodeId, property: AnimProperty, idx: usize, time_ms: f64) -> Option<f64> {
    let v = doc.node(node).and_then(|n| motion::value_at(n, property, time_ms))?;
    match v {
        AnimValue::Scalar(x) => Some(x * property.display_factor()),
        AnimValue::Vec2(p) => Some(if idx == 0 { p.x } else { p.y }),
        AnimValue::Color(_) => None,
    }
}

/// Whether a keyframe sits exactly at `time` on `property`.
pub fn has_key(doc: &Document, node: NodeId, property: AnimProperty, time: u32) -> bool {
    doc.node(node).and_then(|n| n.motion.track(property)).is_some_and(|tr| tr.key_at(time).is_some())
}

/// The nodes on this page that carry any animation (egui `motion::animated_nodes`).
pub fn animated_nodes(doc: &Document, page: NodeId) -> Vec<NodeId> {
    motion::animated_nodes(doc, page)
}

// ---------------------------------------------------------------- 文档写入

/// One undoable document edit. Every Motion write goes through here so the engine's undo
/// stack and dirty tracking stay correct.
fn edit<F>(app: &mut CanvasApp, label: &str, ids: &[NodeId], f: F)
where
    F: FnOnce(&mut Document),
{
    let _ = app.session.begin_interaction(label);
    edit_open(app, ids, f);
    let _ = app.session.commit_interaction();
}

/// Write into an **already open** gesture (a drag): no `begin`/`commit` here, so the whole
/// drag stays one undo step. `Act::BeginEdit` / `Act::EndEdit` own its lifetime.
fn edit_open<F>(app: &mut CanvasApp, ids: &[NodeId], f: F)
where
    F: FnOnce(&mut Document),
{
    if let Ok(doc) = app.session.doc_mut() {
        f(doc);
    }
    if let Ok(doc) = app.session.doc_mut() {
        doc.mark_dirty(ids.to_vec());
    }
}

fn set_duration(app: &mut CanvasApp, page: NodeId, duration: u32) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "duration": duration }));
    let _ = page;
}

fn set_loop(app: &mut CanvasApp, looping: bool) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "loop": looping }));
}

/// 写当前页的 fps（引擎 `motion.timeline {fps}`，影响逐帧步长与吸附）。
fn set_fps(app: &mut CanvasApp, fps: u32) {
    let _ = app.session.execute("motion.timeline", &serde_json::json!({ "fps": fps }));
}

pub fn add_key(app: &mut CanvasApp, node: NodeId, property: AnimProperty, time: u32) {
    edit(app, "Add keyframe", &[node], |doc| {
        let value = doc.node(node).and_then(|n| motion::keyframed_value_at(n, property, time as f64));
        if let (Some(value), Some(n)) = (value, doc.node_mut(node)) {
            n.motion.ensure_track(property).set_key(time, value, None);
        }
    });
}

pub fn remove_key(app: &mut CanvasApp, node: NodeId, property: AnimProperty, time: u32) {
    edit(app, "Delete keyframe", &[node], |doc| {
        if let Some(n) = doc.node_mut(node) {
            if let Some(tr) = n.motion.track_mut(property) {
                tr.remove_key(time);
            }
            n.motion.normalize();
        }
    });
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

pub fn move_key(app: &mut CanvasApp, key: SelectedKey, time: u32) {
    edit_open(app, &[key.node], |doc| {
        if let Some(n) = doc.node_mut(key.node) {
            let mut m = n.motion.clone();
            let value = m.track(key.property).and_then(|tr| tr.key_at(key.time_ms)).map(|k| k.value);
            let ease = m.track(key.property).and_then(|tr| tr.key_at(key.time_ms)).map(|k| k.easing);
            if let Some(v) = value {
                if let Some(tr) = m.track_mut(key.property) {
                    tr.remove_key(key.time_ms);
                }
                m.ensure_track(key.property).set_key(time, v, ease);
                m.normalize();
                n.motion = m;
            }
        }
    });
}

/// Writes a typed value into `property` at the playhead (the inspector's number fields).
pub fn commit_value(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, property: AnimProperty, idx: usize, value: f64) {
    let time = ui.playhead_ms.max(0.0) as u32;
    edit(app, "Edit animation value", &[node], |doc| {
        // Read the current value BEFORE taking `&mut node`: `doc_node_value` needs an immutable
        // borrow of `doc`, which cannot coexist with the mutable one below.
        let current = doc
            .node(node)
            .and_then(|n| n.motion.track(property))
            .and_then(|tr| tr.key_at(time))
            .map(|k| k.value)
            .or_else(|| doc_node_value(doc, node, property, time as f64));
        let Some(n) = doc.node_mut(node) else { return };
        let mut m = n.motion.clone();
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
            _ => AnimValue::Scalar(value / property.display_factor().max(1e-6)),
        };
        m.ensure_track(property).set_key(time, next, None);
        m.normalize();
        n.motion = m;
    });
}

/// Borrow-safe helper: `commit_value` already holds `&mut Document`, so re-read through the node
/// id rather than trying to keep a second borrow alive.
fn doc_node_value(doc: &Document, node: NodeId, prop: AnimProperty, time: f64) -> Option<AnimValue> {
    doc.node(node).and_then(|n| motion::keyframed_value_at(n, prop, time))
}

/// 把 ease 写进关键帧段或 clip（不碰 undo 栈，由调用方决定）。
fn write_ease(doc: &mut Document, target: EaseTarget, ease: Ease) {
    match target {
        EaseTarget::Key(k) => {
            if let Some(n) = doc.node_mut(k.node)
                && let Some(tr) = n.motion.track_mut(k.property)
                && let Some(i) = tr.key_index(k.time_ms)
            {
                tr.keys[i].easing = ease;
            }
        }
        EaseTarget::Clip(node, id) => {
            if let Some(n) = doc.node_mut(node)
                && let Some(c) = n.motion.clips.iter_mut().find(|c| c.id == id)
            {
                c.easing = ease;
                c.sanitize();
            }
        }
    }
}

fn ease_ids(target: EaseTarget) -> Vec<NodeId> {
    vec![match target {
        EaseTarget::Key(k) => k.node,
        EaseTarget::Clip(n, _) => n,
    }]
}

pub fn set_ease(app: &mut CanvasApp, key: SelectedKey, ease: Ease) {
    edit(app, "Ease curve", &[key.node], |doc| {
        if let Some(n) = doc.node_mut(key.node)
            && let Some(tr) = n.motion.track_mut(key.property)
            && let Some(i) = tr.key_index(key.time_ms)
        {
            tr.keys[i].easing = ease;
        }
    });
}

pub fn add_clip(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, kind: ClipKind) {
    let start = ui.playhead_ms.max(0.0) as u32;
    edit(app, "Add animation", &[node], |doc| {
        if let Some(n) = doc.node_mut(node) {
            let mut c = Clip::new(kind, ClipMode::In);
            c.id = n.motion.next_clip_id();
            c.delay_ms = start;
            n.motion.clips.push(c);
            n.motion.normalize();
        }
    });
}

pub fn add_preset(app: &mut CanvasApp, ui: &MotionUi, node: NodeId, name: &str) {
    let start = ui.playhead_ms.max(0.0) as u32;
    let Some(preset) = motion::find_preset(name) else { return };
    edit(app, "Add animation preset", &[node], |doc| {
        if let Some(n) = doc.node_mut(node) {
            let first = n.motion.next_clip_id();
            n.motion.clips.extend(preset.instantiate(start, first));
            n.motion.normalize();
        }
    });
}

pub fn remove_clip(app: &mut CanvasApp, node: NodeId, id: u32) {
    edit(app, "Remove animation", &[node], |doc| {
        if let Some(n) = doc.node_mut(node) {
            n.motion.clips.retain(|c| c.id != id);
            n.motion.normalize();
        }
    });
}

pub fn toggle_clip_visible(app: &mut CanvasApp, node: NodeId, id: u32) {
    edit(app, "Show/hide animation", &[node], |doc| {
        if let Some(c) = doc.node_mut(node).and_then(|n| n.motion.clips.iter_mut().find(|c| c.id == id)) {
            c.visible = !c.visible;
        }
    });
}

pub fn set_clip(app: &mut CanvasApp, node: NodeId, id: u32, clip: Clip) {
    edit(app, "Edit animation clip", &[node], |doc| write_clip(doc, node, id, clip));
}

/// Write one clip into the document. Shared by the settings window (its own undo step) and
/// by the live clip drag (inside the gesture opened by `Act::BeginEdit`).
fn write_clip(doc: &mut Document, node: NodeId, id: u32, clip: Clip) {
    if let Some(c) = doc.node_mut(node).and_then(|n| n.motion.clips.iter_mut().find(|c| c.id == id)) {
        let mut next = clip;
        next.sanitize();
        *c = next;
    }
}

fn clip_dragged(app: &mut CanvasApp, ui: &mut MotionUi, node: NodeId, id: u32, edge: ClipEdge, dx_ms: i64) {
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
    c.sanitize();
    edit_open(app, &[node], |doc| write_clip(doc, node, id, c));
}

fn layer_dragged(app: &mut CanvasApp, ui: &mut MotionUi, node: NodeId, edge: ClipEdge, dx_ms: i64) {
    let Some(drag) = ui.layer_drag.clone() else { return };
    let (lo, hi) = (drag.lo as f64, drag.hi.max(drag.lo + 1) as f64);
    edit_open(app, &[node], |doc| {
        if let Some(n) = doc.node_mut(node) {
            let mut m = drag.orig.clone();
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
            n.motion = m;
        }
    });
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
    let page = app.session.ds().ok().map(|d| d.page);
    let doc = app.session.doc().ok().cloned();
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
            Act::SetLoop(on) => set_loop(app, on),
            Act::SetFps(v) => {
                let cur = app.session.ds().ok().map(|ds| settings_of(&ds.doc, ds.page).fps).unwrap_or(60);
                let next = if v == 0 { match cur { 24 => 30, 30 => 60, _ => 24 } } else { v.clamp(1, figcraft_doc::anim::motion::TimelineSettings::MAX_FPS) };
                set_fps(app, next);
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
                edit(app, "Show/hide layer", &[n], |doc| {
                    if let Some(node) = doc.node_mut(n) {
                        node.visible = !node.visible;
                    }
                });
            }
            Act::ToggleTrackVisible(n, p) => {
                edit(app, "Show/hide track", &[n], |doc| {
                    if let Some(node) = doc.node_mut(n)
                        && let Some(tr) = node.motion.track_mut(p)
                    {
                        tr.visible = !tr.visible;
                    }
                });
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
            }
            Act::CloseClipPopover => ui.clip_popover = None,
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
                let ids = ease_ids(target);
                edit(app, "Ease curve", &ids, |doc| write_ease(doc, target, ease));
            }
            Act::EaseBegin => {
                let _ = app.session.begin_interaction("Ease curve");
            }
            Act::EaseLive(target, ease) => {
                let ids = ease_ids(target);
                edit_open(app, &ids, |doc| write_ease(doc, target, ease));
            }
            Act::EaseEnd => {
                let _ = app.session.commit_interaction();
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
                let _ = app.session.begin_interaction(label);
            }
            Act::EndEdit => {
                let _ = app.session.commit_interaction();
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

    /// The inspector's number field commits at the playhead, not at t=0.
    #[test]
    fn commit_value_writes_key_at_playhead() {
        let (mut app, node) = app_with_node();
        let ui = MotionUi { playhead_ms: 750.0, ..Default::default() };
        commit_value(&mut app, &ui, node, AnimProperty::Width, 0, 240.0);
        assert!(matches!(key_at(&app, node, AnimProperty::Width, 750), Some(AnimValue::Scalar(v)) if (v - 240.0).abs() < 1e-9));
        assert!(key_at(&app, node, AnimProperty::Width, 0).is_none(), "no stray keyframe at t=0");
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

