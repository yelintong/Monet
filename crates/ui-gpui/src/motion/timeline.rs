// ============================================================================
// 底部时间轴 —— 逐行照 `ui-egui/src/motion/timeline.rs` 的布局移植。
//
// 几何、控件坐标、行结构、绘制顺序全部对齐原文（行号见各段注释），不是另起一套：
//   L19-41   常量 HEIGHT/MIN_H/HEADER_H/ROW_H/LEFT_W/TIME_PAD/ROW_ICON/NAME_X/KF_R/EDGE_HIT
//   L185-207 header 47pt + body；body_rows → header_row → ruler 的绘制顺序
//   L229-278 header 控件坐标（play@15、add_key@38、时间框@68/108、s@137、loop@160、fold@190）
//   L321-362 ruler 与 header 同高，major/minor 刻度 + "{:.2}s" 标签
//   L365-833 body_rows：选中图层、clips 先 tracks 后、NAME_X=56、eye@LEFT_W-15
//   L824-831 竖分隔线 @LEFT_W、播放头 y0=HEADER_H-6、圆点 r=5.5
//
// 绘制：几何走 `canvas::Painter`（与画布同一套 Op），文字/图标走叠在上面的 div。
// 命中表与绘制同源，root div 的 mouse-down/move/up 反查。
// ============================================================================
//! The docked Motion timeline: ruler, layer rows, property tracks, keyframe diamonds,
//! segments, clip bars and the playhead.

use figcraft_doc::anim::motion::{AnimProperty, TimelineSettings};
use figcraft_doc::{Document, NodeId};
use gpui::{
    AnyElement, App, CursorStyle, Div, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, Window, canvas, div,
    prelude::*, px,
};

use crate::canvas::geom::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, pos2, regular, semibold, vec2};
use crate::canvas::painter::{Label, Painter, text_size};
use crate::motion::{Act, ClipEdge, MotionUi, Queue, SelItem, settings_of};
use crate::theme::stroke;
use crate::widgets::icon::Icon;
use crate::widgets::input::Slot;
use crate::widgets::input::field::text_field;
use crate::widgets::input::text_field::TextField;
use crate::widgets::{pt, rgba, tokens};
use gpui::Entity;

/// The two editable time boxes in the header (egui `time_field`, L294-311). Retained entities:
/// rebuilt every frame they would lose focus and the caret on every keystroke.
pub struct TimeFields {
    cur: Entity<TextField>,
    dur: Entity<TextField>,
}

impl TimeFields {
    pub fn new(cx: &mut App, queue: &Queue) -> Self {
        let q_cur = queue.clone();
        let cur = text_field(cx, 38.0, "0.00", "", Slot::None, Slot::None);
        cur.update(cx, |tf, _| {
            tf.set_numeric("", 0.01);
            tf.set_on_commit(move |s, _, _| {
                if let Ok(v) = s.trim().parse::<f64>() {
                    q_cur.push(Act::Seek((v * 1000.0).max(0.0) as u32));
                }
            });
        });
        let q_dur = queue.clone();
        let dur = text_field(cx, 38.0, "2.00", "", Slot::None, Slot::None);
        dur.update(cx, |tf, _| {
            tf.set_numeric("", 0.01);
            tf.set_on_commit(move |s, _, _| {
                if let Ok(v) = s.trim().parse::<f64>() {
                    q_dur.push(Act::SetDuration((v * 1000.0).clamp(100.0, TimelineSettings::MAX_DURATION_MS as f64) as u32));
                }
            });
        });
        Self { cur, dur }
    }

    /// Push the live values in — but **never over a box the user is typing in** (that would eat
    /// every keystroke and make the field look dead).
    pub fn sync(&self, cx: &mut App, ui: &MotionUi, settings: TimelineSettings, window: &mut Window) {
        let cur_active = self.cur.read(cx).edit().read(cx).is_active(window);
        let dur_active = self.dur.read(cx).edit().read(cx).is_active(window);
        if !cur_active {
            let want = format!("{:.2}", ui.playhead_ms / 1000.0);
            self.cur.update(cx, |tf, _| tf.set_value(want));
        }
        if !dur_active {
            let want = format!("{:.2}", settings.duration_ms as f64 / 1000.0);
            self.dur.update(cx, |tf, _| tf.set_value(want));
        }
    }
}

// egui timeline.rs L19-41 —— 一个数字都不改。
pub const HEADER_H: f32 = 47.0;
pub const ROW_H: f32 = 24.0;
pub const LEFT_W: f32 = 297.0;
pub const TIME_PAD: f32 = 9.5;
/// Row layout: 8 pt padding, a 16 pt fold arrow, 8 pt, a 16 pt kind icon, 8 pt, then the name.
const ROW_PAD: f32 = 8.0;
const ROW_ICON: f32 = 16.0;
const NAME_X: f32 = ROW_PAD + ROW_ICON + 8.0 + ROW_ICON + 8.0; // 56
const KF_R: f32 = 5.0;
const EDGE_HIT: f32 = 8.0;
/// egui `timeline::SCROLL_SPEED` (L41): px/sec the track scrolls while the pointer sits in the
/// duration handle's right-edge scroll zone. At ~60fps that is `SCROLL_SPEED/60` per move event.
const SCROLL_SPEED: f32 = 1200.0;

// ---------------------------------------------------------------- 行模型（egui L43-60）

#[derive(Clone, Copy, Debug)]
enum RowKind {
    Layer,
    Property(AnimProperty),
    Clip,
}

#[derive(Clone, Debug)]
enum RowData {
    Layer { span: Option<(u32, u32)> },
    Property { keys: Vec<u32>, visible: bool },
    Clip { id: u32, start: u32, end: u32 },
}

#[derive(Clone, Debug)]
struct Row {
    node: NodeId,
    kind: RowKind,
    label: String,
    data: RowData,
}

fn prop_of(row: &Row) -> AnimProperty {
    match row.kind {
        RowKind::Property(p) => p,
        _ => AnimProperty::Opacity,
    }
}

/// egui `build_rows` (L141-176): **the selected layers**, clips first, then non-empty tracks.
fn build_rows(doc: &Document, selection: &[NodeId], ui: &MotionUi, page: NodeId, settings: TimelineSettings) -> Vec<Row> {
    let dur = settings.duration_ms;
    let mut rows = Vec::new();
    for id in selection.iter().copied().filter(|n| doc.node(*n).is_some()) {
        let Some(n) = doc.node(id) else { continue };
        rows.push(Row {
            node: id,
            kind: RowKind::Layer,
            label: n.name.clone(),
            data: RowData::Layer { span: n.motion.span_ms(dur) },
        });
        if ui.folded.contains(&id) {
            continue;
        }
        // Clips first, then the keyframe tracks (Figma's order).
        for c in &n.motion.clips {
            rows.push(Row {
                node: id,
                kind: RowKind::Clip,
                label: format!("{} · {}", c.kind.name(), c.mode.label(c.kind)),
                data: RowData::Clip { id: c.id, start: c.delay_ms, end: c.end_ms(dur) },
            });
        }
        for tr in n.motion.tracks.iter().filter(|t| !t.is_empty()) {
            rows.push(Row {
                node: id,
                kind: RowKind::Property(tr.property),
                label: tr.property.name().to_string(),
                data: RowData::Property { keys: tr.keys.iter().map(|k| k.time_ms).collect(), visible: tr.visible },
            });
        }
    }
    let _ = page;
    rows
}

// ---------------------------------------------------------------- 命中表

#[derive(Clone, Copy, Debug)]
enum HitKind {
    Seek,
    Fold(NodeId),
    FoldAll,
    Layer(NodeId),
    Track(NodeId, AnimProperty),
    Key(NodeId, AnimProperty, u32),
    Segment(NodeId, AnimProperty, u32),
    ClipEdge(NodeId, u32, ClipEdge),
    LayerEdge(NodeId, ClipEdge),
    AddKeyAtPlayhead,
    Play,
    Loop,
    /// 帧率按钮：点一下在 24 / 30 / 60 间循环（写 `motion.timeline {fps}`）。
    Fps,
    Duration,
    ToggleKey(NodeId, AnimProperty),
    SeekKey(u32),
    Visible(NodeId),
    TrackVisible(NodeId, AnimProperty),
    Close,
    /// Header export button (egui L267-268): present but a literal no-op there. Must NOT clear
    /// the selection (that is `HitKind::Clear` on the empty lane).
    Export,
    Clear,
    /// 面板顶边：上下拖动改整条时间轴的高度（egui `app/frame.rs::timeline_edge`）。
    ResizeH,
}

#[derive(Clone, Copy, Debug)]
struct Hit {
    rect: Rect,
    kind: HitKind,
}

/// What a press started. Once set, every move is routed by this — **not** by re-hit-testing:
/// the pointer leaves the ruler / bar during a drag, and re-testing is what made the playhead
/// and the duration handle stop dead after two pixels (egui uses `Response::dragged()`).
#[derive(Clone, Copy, Debug)]
enum DragKind {
    Seek,
    Duration,
    /// A clip bar / layer summary: `Act::DragMove` recomputes from the snapshot taken on press.
    Clip,
    Layer,
}

// ⛔ 拖拽的瞬时状态必须**跨帧持久**。`bar()` 每帧都会重新执行（`q.push` → wake → `cx.notify()`
// → 重绘），如果在 `bar()` 里用 `Rc::new(RefCell::new(None))` 存拖拽状态，那么按下（down）设的
// 状态会在紧接着的一次重绘后变成全新的空状态 —— 于是 move 处理器看到 `None` 直接 early return，
// 表现就是「点一下跳一次、拖动完全不跟手」。这正是之前「播放头/时长线/片段拖不动」的真根因。
// 时间轴是单例，用 thread_local 存这四个瞬时状态即可跨帧存活。
thread_local! {
    /// 指针是否在底部 Motion 时间轴里（Space 才播放，否则 Space 仍是画布平移）。
    static POINTER_IN_BAR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_pointer_in_bar(on: bool) {
    POINTER_IN_BAR.with(|c| c.set(on));
}

pub fn pointer_in_bar() -> bool {
    POINTER_IN_BAR.with(|c| c.get())
}

thread_local! {
    /// 时长拖拽的累计横向滚动。⛔ 必须和同一帧算时间用的值**完全相同**：以前时间用上一帧的
    /// scroll、滚动用新值，结果向右拖时先冻住再猛跳，看起来像「拖出去就断了」。
    static DUR_SCROLL: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    /// 上一次推进滚动的时间戳（按真实 dt 算速度，不按帧数）。
    static DUR_LAST: std::cell::RefCell<Option<std::time::Instant>> = const { std::cell::RefCell::new(None) };
    /// 上一次请求悬停刷新的位置格（4pt），避免同格内反复唤醒。
    static HOVER_KEY: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((i32::MIN, i32::MIN)) };
}

thread_local! {
    static DRAG: std::cell::RefCell<Option<DragKind>> = const { std::cell::RefCell::new(None) };
    static GRABBED: std::cell::RefCell<Option<crate::motion::SelectedKey>> = const { std::cell::RefCell::new(None) };
    static LAST_PRESS: std::cell::RefCell<Option<(std::time::Instant, f32, f32)>> = const { std::cell::RefCell::new(None) };
    static BEGAN: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    /// Window-space top-left of the timeline bar. Captured from the canvas paint closure each
    /// frame and read by the root view's window-wide move handler so a drag keeps routing once
    /// the pointer leaves the bar (gpui's per-element `on_mouse_move` only fires while hovered).
    static BAR_ORIGIN: std::cell::RefCell<Point<Pixels>> = const { std::cell::RefCell::new(Point::new(px(0.0), px(0.0))) };
    /// Live geometry the move router needs, refreshed every frame in `bar()`:
    /// (scale px/ms, scroll_x, track_right, frame_ms, duration_ms).
    static GEOM: std::cell::RefCell<(f32, f32, f32, f32, u32)> =
        const { std::cell::RefCell::new((1.0, 0.0, 9999.0, 1000.0 / 60.0, 2000)) };
}

/// Push a text label with the same anchoring the canvas painter uses.
fn text(labels: &mut Vec<Label>, at: Pos2, align: Align2, s: impl Into<String>, font: FontId, color: Color32) {
    let s = s.into();
    let rect = align.anchor_size(at, text_size(&s, font));
    labels.push(Label { rect, text: s, font, color, align });
}

/// egui `widgets::elide` — cut a long name to a fixed number of glyphs.
fn elide(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// egui `pxms` (L316-318).
fn scale_of(track_x: f32, right: f32, duration_ms: u32) -> f32 {
    ((right - track_x - TIME_PAD - 16.0).max(40.0)) / duration_ms.max(1) as f32
}

/// egui `tick_step` (L1315-1329).
fn tick_step(duration_ms: f64) -> f64 {
    let raw = (duration_ms / 6.0).max(1.0);
    let p = 10f64.powf(raw.log10().floor());
    let n = raw / p;
    let m = if n <= 1.0 { 1.0 } else if n <= 2.0 { 2.0 } else if n <= 5.0 { 5.0 } else { 10.0 };
    (m * p).max(10.0)
}

/// Build the drawn ops, text labels, icon layer and hit table in one pass.
#[allow(clippy::too_many_arguments)]
fn build(
    doc: &Document,
    ui: &MotionUi,
    selection: &[NodeId],
    page: NodeId,
    width: f32,
    height: f32,
    settings: TimelineSettings,
    // 指针在 bar 里的局部坐标（`None` = 指针不在时间轴上）。egui 的 `rect_contains_pointer`
    // 悬停效果（hover 底色、眼睛图标、菱形变亮）都由它驱动；GPUI 没有逐元素 hover 状态。
    hover: Option<Pos2>,
) -> (Painter, Vec<Label>, Vec<Hit>, Vec<(Rect, Icon, Color32)>) {
    let t = tokens();
    let mut p = Painter::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height)));
    let mut labels = Vec::new();
    let mut hits = Vec::new();
    let mut icons = Vec::new();

    // egui L188: the whole panel is one flat panel-coloured rect.
    p.rect_filled(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height)), CornerRadius::ZERO, t.panel);

    let track_x = LEFT_W;
    let track_right = width;
    let scale = scale_of(track_x, track_right, settings.duration_ms);
    let scroll_x = ui.scroll_x;
    // egui L327 / L426: x_of(time) = track_x + TIME_PAD + time*scale - scroll.
    let x_of = |time: f64| track_x + TIME_PAD + (time as f32) * scale - scroll_x;
    let time_of = |x: f32| (((x - track_x - TIME_PAD + scroll_x) / scale).max(0.0)) as f64;

    // The playhead x and the top of its visible line (egui L829: `y0 = header_top + HEADER_H - 6`).
    // Computed once here so both the full-height seek band (below) and the draw (later) agree, and
    // so the line is grabbable across its whole height, not just the top ruler band.
    let playhead_x = x_of(ui.playhead_ms);
    let playhead_top = HEADER_H - 6.0;

    // ── body_rows FIRST (egui L201-202: scrolled rows never bleed into header/ruler) ─────
    let body_top = HEADER_H;
    let rows = build_rows(doc, selection, ui, page, settings);
    // egui L439: rows_top = body.min.y + 8 - scroll_y.
    let rows_top = body_top + 8.0 - ui.scroll_y;

    // egui L374-377: a press on the empty lane clears the selection; registered before rows.
    hits.push(Hit {
        rect: Rect::from_min_max(pos2(track_x, body_top), pos2(track_right, height)),
        kind: HitKind::Clear,
    });

    // egui only makes the TOP ruler band seekable (L352), but the drawn playhead line runs the full
    // height of the panel — so the user sees a line but can only grab its top 47px. Make the visible
    // portion of that line grabbable across its whole height. Registered before the rows so row /
    // keyframe / segment hits (pushed later) still win where they exist, and it only fills the empty
    // lane around the line. Guarded to the on-screen track so it never steals clicks from the
    // left-column row controls when the playhead is scrolled out of view.
    if playhead_x >= track_x && playhead_x <= track_right {
        hits.push(Hit {
            rect: Rect::from_min_max(pos2(playhead_x - 6.0, playhead_top), pos2(playhead_x + 6.0, height)),
            kind: HitKind::Seek,
        });
    }

    // egui L429-436: past the duration the track area is shaded.
    let shown = ui.drag_duration.unwrap_or(settings.duration_ms);
    let end_x = x_of(shown as f64);
    if end_x < track_right {
        p.rect_filled(
            Rect::from_min_max(pos2(end_x.max(track_x), body_top), pos2(track_right, height)),
            CornerRadius::ZERO,
            t.border.gamma_multiply(0.35),
        );
        p.line_segment([pos2(end_x, body_top), pos2(end_x, height)], stroke(t.border_strong, 1.0));
    }

    for (i, row) in rows.iter().enumerate() {
        let y = rows_top + i as f32 * ROW_H;
        if y + ROW_H < body_top || y > height {
            continue;
        }
        let cy = y + ROW_H / 2.0;
        let rect = Rect::from_min_max(pos2(0.0, y), pos2(track_right, y + ROW_H));
        // egui `ui.rect_contains_pointer(rect)`：行悬停时该行才画眼睛、菱形变亮。
        let row_hovered = hover.is_some_and(|p| rect.contains(p));
        // egui L450-491: selected layer = deep block, its children = lighter block.
        let layer_sel = ui.selection.contains(&SelItem::Layer(row.node));
        let track_sel = matches!(row.data, RowData::Property { .. })
            && ui.selection.contains(&SelItem::Track(row.node, prop_of(row)));
        let clip_sel = match &row.data {
            RowData::Clip { id, .. } => ui.selection.contains(&SelItem::Clip(row.node, *id)),
            _ => false,
        };
        let bg = match &row.data {
            RowData::Layer { .. } => layer_sel.then_some(t.selected_row),
            RowData::Property { .. } => {
                if layer_sel { Some(t.selected_row_child) } else { track_sel.then_some(t.selected_row) }
            }
            RowData::Clip { .. } => {
                if layer_sel { Some(t.selected_row_child) } else { clip_sel.then_some(t.selected_row) }
            }
        };
        if let Some(bg) = bg {
            p.rect_filled(rect, CornerRadius::ZERO, bg);
        }

        // egui L493-494: eye centre at LEFT_W - 15, name at NAME_X (56).
        let eye = Rect::from_center_size(pos2(LEFT_W - 15.0, cy), vec2(16.0, 16.0));
        let name_x = NAME_X;

        match &row.data {
            RowData::Layer { span } => {
                // egui L498-505: fold chevron centre at ROW_PAD + ROW_ICON/2 (=16).
                let c = pos2(ROW_PAD + ROW_ICON / 2.0, cy);
                icons.push((
                    Rect::from_center_size(c, vec2(10.0, 10.0)),
                    if ui.folded.contains(&row.node) { Icon::ChevronRight } else { Icon::ChevronDown },
                    t.text_secondary,
                ));
                hits.push(Hit { rect: Rect::from_center_size(c, vec2(ROW_ICON, ROW_ICON)), kind: HitKind::Fold(row.node) });
                // egui L507-510: the row click covers the LEFT column only.
                hits.push(Hit {
                    rect: Rect::from_min_max(pos2(0.0, y), pos2(LEFT_W, y + ROW_H)),
                    kind: HitKind::Layer(row.node),
                });
                text(
                    &mut labels,
                    pos2(name_x, cy),
                    Align2::LEFT_CENTER,
                    elide(&row.label, 28),
                    semibold(10.5),
                    t.text,
                );
                // egui L636-641: the eye only shows while the row is hovered（悬停才出现）。
                if row_hovered {
                    icons.push((eye, if doc.node(row.node).is_some_and(|n| n.visible) { Icon::Eye } else { Icon::EyeOff }, t.text_secondary));
                }
                hits.push(Hit { rect: eye, kind: HitKind::Visible(row.node) });

                // egui L515-530: layer summary bar, 22 pt tall, accent @45%, white end grips.
                if let Some((a, b)) = span {
                    let x0 = x_of(*a as f64);
                    let x1 = x_of(*b as f64).max(x0 + 3.0);
                    let bar = Rect::from_min_max(pos2(x0, cy - 11.0), pos2(x1, cy + 11.0));
                    p.rect_filled(bar, CornerRadius::same(4), t.accent.gamma_multiply(0.45));
                    if x1 - x0 >= 20.0 {
                        for gx in [x0 + 4.0, x1 - 6.0] {
                            p.rect_filled(Rect::from_min_size(pos2(gx, cy - 7.0), vec2(2.0, 14.0)), CornerRadius::same(1), Color32::WHITE);
                        }
                    }
                    let edge = EDGE_HIT.min((x1 - x0).max(1.0) / 2.0);
                    hits.push(Hit { rect: bar, kind: HitKind::LayerEdge(row.node, ClipEdge::Move) });
                    hits.push(Hit { rect: Rect::from_min_max(pos2(x0, bar.min.y), pos2(x0 + edge, bar.max.y)), kind: HitKind::LayerEdge(row.node, ClipEdge::TrimStart) });
                    hits.push(Hit { rect: Rect::from_min_max(pos2(x1 - edge, bar.min.y), pos2(x1, bar.max.y)), kind: HitKind::LayerEdge(row.node, ClipEdge::TrimEnd) });
                }
            }
            RowData::Property { keys, visible } => {
                // 命中优先级：整行「选轨道 / 清空」必须**先**入表（`hit_at` 取最后一个匹配），
                // 否则它会把后面更具体的关键帧、segment 命中全遮住 —— 那正是「菱形拖不动、
                // 两帧之间的线选不中」的根因。egui 里 `rresp` 也是先创建、seg/key 后创建。
                hits.push(Hit {
                    rect: Rect::from_min_max(pos2(0.0, y), pos2(LEFT_W, y + ROW_H)),
                    kind: HitKind::Track(row.node, prop_of(row)),
                });
                hits.push(Hit { rect, kind: HitKind::Track(row.node, prop_of(row)) });
                text(&mut labels, pos2(name_x, cy), Align2::LEFT_CENTER, elide(&row.label, 9), regular(10.0), if *visible { t.text_secondary } else { t.text_tertiary });
                // egui L600-631: prev / diamond / next at +95 / +109 / +123.
                let pr = Rect::from_center_size(pos2(95.0, cy), vec2(14.0, ROW_H));
                let dr = Rect::from_center_size(pos2(109.0, cy), vec2(14.0, ROW_H));
                let nr = Rect::from_center_size(pos2(123.0, cy), vec2(14.0, ROW_H));
                let now = ui.playhead_ms.max(0.0) as u32;
                let prev = keys.iter().copied().filter(|k| *k < now).max();
                let next = keys.iter().copied().filter(|k| *k > now).min();
                let prev_col = if prev.is_some() { t.text_secondary } else { t.text_tertiary };
                let next_col = if next.is_some() { t.text_secondary } else { t.text_tertiary };
                icons.push((Rect::from_center_size(pr.center(), vec2(10.0, 10.0)), Icon::ChevronLeft, prev_col));
                icons.push((Rect::from_center_size(nr.center(), vec2(10.0, 10.0)), Icon::ChevronRight, next_col));
                if let Some(v) = prev { hits.push(Hit { rect: pr, kind: HitKind::SeekKey(v) }); }
                if let Some(v) = next { hits.push(Hit { rect: nr, kind: HitKind::SeekKey(v) }); }
                hits.push(Hit { rect: dr, kind: HitKind::ToggleKey(row.node, prop_of(row)) });
                let has_keys = !keys.is_empty();
                // egui L617：没打过关键帧的轨道，鼠标悬到菱形上变亮（表示可点）。
                let d_hover = hover.is_some_and(|p| dr.contains(p));
                let dcol = if has_keys { t.accent } else if d_hover { t.icon } else { t.text_secondary };
                let on_key = keys.contains(&now);
                let dc = dr.center();
                p.add_shape(
                    vec![dc + vec2(0.0, -3.5), dc + vec2(3.5, 0.0), dc + vec2(0.0, 3.5), dc + vec2(-3.5, 0.0)],
                    true,
                    if on_key { t.accent } else { Color32::TRANSPARENT },
                    Stroke::new(1.0, dcol),
                );
                // egui L632-635: the property's value at the playhead, at +137.
                let prop = prop_of(row);
                if let Some(v) = doc.node(row.node).and_then(|n| figcraft_doc::anim::motion::value_at(n, prop, ui.playhead_ms)) {
                    let s = match v {
                        figcraft_doc::anim::motion::AnimValue::Scalar(x) => format!("{:.2}{}", x * prop.display_factor(), prop.unit()),
                        figcraft_doc::anim::motion::AnimValue::Vec2(q) => format!("{:.1}, {:.1}", q.x, q.y),
                        figcraft_doc::anim::motion::AnimValue::Color(c) => {
                            let [r, g, b, _] = c.to_rgba8();
                            format!("#{r:02X}{g:02X}{b:02X}")
                        }
                    };
                    text(&mut labels, pos2(137.0, cy), Align2::LEFT_CENTER, elide(&s, 14), regular(10.0), t.text_secondary);
                }
                icons.push((eye, if *visible { Icon::Eye } else { Icon::EyeOff }, t.text_secondary));
                hits.push(Hit { rect: eye, kind: HitKind::TrackVisible(row.node, prop) });

                // egui L643-644: baseline.
                let alpha = if *visible { 1.0 } else { 0.35 };
                p.line_segment([pos2(track_x + TIME_PAD, cy), pos2(track_right, cy)], stroke(t.border.gamma_multiply(alpha), 1.0));
                // egui L669-696: segment links + easing badge at the midpoint of a selected one.
                for w in keys.windows(2) {
                    let (xa, xb) = (x_of(w[0] as f64), x_of(w[1] as f64));
                    let seg_sel = ui.selection.contains(&SelItem::Segment(row.node, prop, w[0]));
                    // 悬停到两帧之间那条线：加粗并提亮（egui 没有这层反馈，但这里明显更需要，
                    // 否则那条细线看起来不可选）。
                    let seg_hover = hover.is_some_and(|p| {
                        p.x >= xa - 2.0 && p.x <= xb + 2.0 && (p.y - cy).abs() <= 6.0
                    });
                    let line = if seg_sel {
                        stroke(t.accent, 2.0)
                    } else if seg_hover {
                        stroke(t.accent.gamma_multiply(0.9), 2.0)
                    } else {
                        stroke(t.accent.gamma_multiply(0.35 * alpha), 1.0)
                    };
                    let off = KF_R.min((xb - xa) / 2.0);
                    p.line_segment([pos2(xa + off, cy), pos2(xb - off, cy)], line);
                    if seg_sel && xb - xa > 24.0 {
                        let gr = Rect::from_center_size(pos2((xa + xb) / 2.0, cy), vec2(16.0, 16.0));
                        p.rect_filled(gr, CornerRadius::same(3), t.panel);
                        p.rect_stroke(gr, CornerRadius::same(3), stroke(t.accent, 1.0), crate::canvas::geom::StrokeKind::Inside);
                        // egui L687-688 / mini_curve: 该段的 easing 采样折线画在徽标里。
                        if let Some(ease) = doc.node(row.node)
                            .and_then(|n| n.motion.track(prop))
                            .and_then(|tr| tr.key_at(w[0]))
                            .map(|k| k.easing)
                        {
                            let (lo, hi) = ease.value_range();
                            let span = (hi - lo).abs().max(1e-6);
                            let pts: Vec<Pos2> = ease.sample(10).iter().map(|q| {
                                let x = gr.min.x + 2.0 + q[0].clamp(0.0, 1.0) * (gr.width() - 4.0);
                                let y = gr.max.y - 2.0 - ((q[1] - lo) / span).clamp(0.0, 1.0) * (gr.height() - 4.0);
                                pos2(x, y)
                            }).collect();
                            p.add_shape(pts, false, Color32::TRANSPARENT, stroke(t.text, 1.0));
                        }
                    }
                    hits.push(Hit {
                        rect: Rect::from_min_max(pos2(xa, cy - 5.0), pos2(xb, cy + 5.0)),
                        kind: HitKind::Segment(row.node, prop, w[0]),
                    });
                }
                // egui L697-730: keyframe diamonds, hollow unless selected.
                for (ki, &time) in keys.iter().enumerate() {
                    let c = pos2(x_of(time as f64), cy);
                    let picked = ui.selection.contains(&SelItem::Key(row.node, prop, time));
                    let k_hover = hover.is_some_and(|p| {
                        (p.x - c.x).abs() <= KF_R + 3.0 && (p.y - cy).abs() <= KF_R + 3.0
                    });
                    let fill = if picked {
                        t.accent.gamma_multiply(alpha)
                    } else if k_hover {
                        // 悬停时菱形内部半亮：既不遮住空心造型，又明确「这里能抓」。
                        t.accent.gamma_multiply(0.35 * alpha)
                    } else {
                        Color32::TRANSPARENT
                    };
                    p.add_shape(
                        vec![c + vec2(0.0, -KF_R), c + vec2(KF_R, 0.0), c + vec2(0.0, KF_R), c + vec2(-KF_R, 0.0)],
                        true,
                        fill,
                        Stroke::new(1.0, t.accent.gamma_multiply(alpha)),
                    );
                    hits.push(Hit {
                        rect: Rect::from_center_size(c, vec2(KF_R * 2.0 + 6.0, KF_R * 2.0 + 6.0)),
                        kind: HitKind::Key(row.node, prop, time),
                    });
                    let _ = ki;
                }
                // 整行的「双击加关键帧 / 点空白清空」命中已在上面（关键帧与 segment 之下）。
            }
            RowData::Clip { id, start, end } => {
                text(&mut labels, pos2(name_x, cy), Align2::LEFT_CENTER, elide(&row.label, 24), regular(10.0), t.text_secondary);
                // egui L740-742: 悬停该行才画眼睛。
                if row_hovered {
                    icons.push((eye, Icon::Eye, t.text_secondary));
                }
                // egui L743-760: clip bar, accent @36% + accent outline, white grips, centred label.
                let a = x_of(*start as f64);
                let b = x_of(*end as f64).max(a + 6.0);
                let bar = Rect::from_min_max(pos2(a, cy - 11.0), pos2(b, cy + 11.0));
                p.rect_filled(bar, CornerRadius::same(4), t.accent.gamma_multiply(0.36));
                p.rect_stroke(bar, CornerRadius::same(4), stroke(t.accent, 1.0), crate::canvas::geom::StrokeKind::Inside);
                if b - a >= 20.0 {
                    for gx in [a + 4.0, b - 6.0] {
                        p.rect_filled(Rect::from_min_size(pos2(gx, cy - 7.0), vec2(2.0, 14.0)), CornerRadius::same(1), Color32::WHITE);
                    }
                }
                let kind = row.label.split(" · ").next().unwrap_or(&row.label);
                text(&mut labels, bar.center(), Align2::CENTER_CENTER, elide(kind, 8), regular(9.0), t.accent_on);
                let edge = EDGE_HIT.min((b - a).max(1.0) / 2.0);
                hits.push(Hit { rect: bar, kind: HitKind::ClipEdge(row.node, *id, ClipEdge::Move) });
                hits.push(Hit { rect: Rect::from_min_max(pos2(a, bar.min.y), pos2(a + edge, bar.max.y)), kind: HitKind::ClipEdge(row.node, *id, ClipEdge::TrimStart) });
                hits.push(Hit { rect: Rect::from_min_max(pos2(b - edge, bar.min.y), pos2(b, bar.max.y)), kind: HitKind::ClipEdge(row.node, *id, ClipEdge::TrimEnd) });
            }
        }
    }

    // egui L387-390: the duration handle is a ±8 pt band at the committed duration's x that
    // spans the WHOLE body — this is the "line" the user drags to retime the timeline.
    // Registered after the rows so it wins over a row's own press inside that band.
    let grab_x = x_of(settings.duration_ms as f64);
    hits.push(Hit {
        rect: Rect::from_min_max(pos2(grab_x - 8.0, body_top), pos2(grab_x + 8.0, height)),
        kind: HitKind::Duration,
    });

    // egui L825: vertical divider after the left column, over the whole body.
    p.line_segment([pos2(track_x, body_top), pos2(track_x, height)], stroke(t.border, 1.0));

    // ── header_row (egui L229-278) ───────────────────────────────────────────
    // The header has NO title text: only controls. `at(cx, w, h)` centres on cx.
    let cy = HEADER_H / 2.0;
    let at = |cx: f32, w: f32, h: f32| Rect::from_center_size(pos2(cx, cy), vec2(w, h));

    // Play / Pause (egui L236): gpui's `Icon` enum has no Play/Pause glyph, so draw it directly
    // — a right-pointing triangle while paused (click → play), two bars while playing. Matches the
    // egui button's hit region exactly.
    let play_r = at(15.0, 22.0, 24.0);
    hits.push(Hit { rect: play_r, kind: HitKind::Play });
    {
        let c = play_r.center();
        if ui.playing {
            let bw = 3.0;
            let bh = 11.0;
            let gap = 3.0;
            p.rect_filled(Rect::from_min_max(pos2(c.x - gap - bw, c.y - bh / 2.0), pos2(c.x - gap, c.y + bh / 2.0)), CornerRadius::same(2), t.icon);
            p.rect_filled(Rect::from_min_max(pos2(c.x + gap, c.y - bh / 2.0), pos2(c.x + gap + bw, c.y + bh / 2.0)), CornerRadius::same(2), t.icon);
        } else {
            let s = 6.0;
            p.add_shape(vec![pos2(c.x - s * 0.6, c.y - s), pos2(c.x + s, c.y), pos2(c.x - s * 0.6, c.y + s)], true, t.icon, Stroke::new(0.0, t.icon));
        }
    }

    // The rest of the header controls. Export (egui L267-268) is a no-op button there, so it maps
    // to `HitKind::Export` (does nothing) — NOT `Clear`, which would wipe the user's selection.
    let controls: [(Rect, HitKind, Icon); 5] = [
        (at(38.0, 22.0, 24.0), HitKind::AddKeyAtPlayhead, Icon::Instance),
        (at(160.0, 22.0, 24.0), HitKind::Loop, if settings.looping { Icon::LinkOn } else { Icon::LinkOff }),
        (at(190.0, 22.0, 24.0), HitKind::FoldAll, Icon::ChevronDown),
        (Rect::from_center_size(pos2(track_right - 46.0, cy), vec2(24.0, 24.0)), HitKind::Export, Icon::Arrow),
        (Rect::from_center_size(pos2(track_right - 18.0, cy), vec2(24.0, 24.0)), HitKind::Close, Icon::Close),
    ];
    for (r, kind, icon) in controls {
        // egui `header_button`: 悬停给 5pt 圆角底色。
        if hover.is_some_and(|p| r.contains(p)) {
            p.rect_filled(r, CornerRadius::same(5), t.hover);
        }
        icons.push((Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.icon));
        hits.push(Hit { rect: r, kind });
    }
    // 帧率：egui 没有 UI 入口但引擎 `motion.timeline {fps}` 支持；这里给一个可见的循环按钮。
    let fps_r = at(220.0, 34.0, 22.0);
    if hover.is_some_and(|p| fps_r.contains(p)) {
        p.rect_filled(fps_r, CornerRadius::same(5), t.hover);
    }
    hits.push(Hit { rect: fps_r, kind: HitKind::Fps });
    text(&mut labels, fps_r.center(), Align2::CENTER_CENTER, format!("{}", settings.fps), regular(9.0), t.text_secondary);
    // egui L243-257: two editable time boxes + the "s" unit label. The boxes are real
    // `TextField`s (drawn by `field_layers`), so no hit region is registered for them here —
    // a hit would swallow the click that focuses the editor.
    // egui L254: 悬停「s」单位格给底色。
    let unit_r = at(137.0, 20.0, 20.0);
    if hover.is_some_and(|p| unit_r.contains(p)) {
        p.rect_filled(unit_r, CornerRadius::same(4), t.hover);
    }
    text(&mut labels, unit_r.center(), Align2::CENTER_CENTER, "s", regular(10.0), t.text_secondary);
    // egui L276-277: separator under the header and after the left column (header only).
    p.line_segment([pos2(0.0, HEADER_H), pos2(track_right, HEADER_H)], stroke(t.border, 1.0));
    p.line_segment([pos2(LEFT_W, 0.0), pos2(LEFT_W, HEADER_H)], stroke(t.border, 1.0));

    // ── ruler (egui L321-362): the RIGHT half of the SAME header band ────────
    let ruler = Rect::from_min_max(pos2(track_x, 0.0), pos2(track_right, HEADER_H));
    hits.push(Hit { rect: ruler, kind: HitKind::Seek });
    let dur_ms = settings.duration_ms.max(1) as f64;
    let step = tick_step(dur_ms).max(1.0);
    let step_u = step as u32;
    let minor = if step_u % 5 == 0 { step_u / 5 } else if step_u % 2 == 0 { step_u / 2 } else { step_u };
    let base = ruler.max.y;
    let mut tick = 0u32;
    let end = dur_ms + step + (scroll_x.max(0.0) / scale) as f64;
    while (tick as f64) <= end {
        let x = x_of(tick as f64);
        if x >= ruler.min.x && x <= ruler.max.x {
            let major = tick % step_u == 0;
            let h = if major { 6.0 } else { 3.0 };
            p.line_segment([pos2(x, base - h), pos2(x, base)], stroke(if major { t.text_tertiary } else { t.border }, 1.0));
            if major {
                text(&mut labels, pos2(x + 3.0, 17.0), Align2::LEFT_CENTER, format!("{:.2}s", tick as f64 / 1000.0), regular(10.0), t.text_secondary);
            }
        }
        tick = tick.saturating_add(minor.max(1));
    }

    // ── playhead LAST (egui L826-831) ────────────────────────────────────────
    let px_head = playhead_x;
    let y0 = playhead_top;
    p.line_segment([pos2(px_head, y0), pos2(px_head, height)], stroke(t.accent, 1.0));
    p.circle_filled(pos2(px_head, y0 + 1.0), 5.5, t.accent);

    // 顶边那条 1px 描边由 timeline 容器自身的 border-top 唯一绘制（见 `bar()` 的 div），
    // 不再在 canvas 里重复 `line_segment`，也不再强插 hover accent 线。
    // 顶边高度手柄的命中交给 `bar()` div 上那条 4px GPUI 覆盖条（能向上溢出到画布侧），
    // 拖动状态机与左面板共用 `resize_drag::Handle::BottomTimeline`。
    let _ = time_of;
    (p, labels, hits, icons)
}

// ---------------------------------------------------------------- 图层

/// Text labels: single unbreakable lines (a fixed `w()` makes GPUI wrap them).
fn text_layers(labels: &[Label]) -> Vec<AnyElement> {
    labels
        .iter()
        .map(|l| {
            let weight = match l.font.weight {
                crate::canvas::geom::FontWeight::Bold => gpui::FontWeight::BOLD,
                crate::canvas::geom::FontWeight::SemiBold => gpui::FontWeight::SEMIBOLD,
                _ => gpui::FontWeight::NORMAL,
            };
            div()
                .absolute()
                .left(px(l.rect.min.x))
                .top(px(l.rect.min.y))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_size(pt(l.font.size))
                .font_weight(weight)
                .text_color(rgba(l.color))
                .child(SharedString::from(l.text.clone()))
                .into_any_element()
        })
        .collect()
}

fn icon_layers(icons: &[(Rect, Icon, Color32)]) -> Vec<AnyElement> {
    icons
        .iter()
        .map(|(r, icon, colour)| {
            div()
                .absolute()
                .left(px(r.min.x))
                .top(px(r.min.y))
                .w(px(r.width()))
                .h(px(r.height()))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(*icon, rgba(*colour), r.width().min(14.0)))
                .into_any_element()
        })
        .collect()
}

// ---------------------------------------------------------------- 视图

/// The docked timeline.
#[allow(clippy::too_many_arguments)]
pub fn bar(
    cx: &mut App,
    doc: &Document,
    ui: &mut MotionUi,
    queue: &Queue,
    selection: &[NodeId],
    times: &TimeFields,
    page: NodeId,
    width: f32,
    window: &mut Window,
) -> Stateful<Div> {
    let t = tokens();
    let h = if ui.timeline_h > 0.0 { ui.timeline_h } else { crate::motion::HEIGHT };
    let settings = settings_of(doc, page);
    times.sync(cx, ui, settings, window);
    // 悬停：用**上一帧**记下的 bar 原点把窗口坐标换成局部坐标（指针是实时的，差一帧无感）。
    // 以前这里是「把命中项的文字画成一个跟着鼠标跑的黑色方块」，既不是 tooltip 的正确行为，
    // 也挡住了内容 —— 已删除，改成 egui 那套真悬停反馈（底色 / 眼睛 / 菱形变亮）。
    let bar_o = BAR_ORIGIN.with(|o| *o.borrow());
    let mp = window.mouse_position();
    let hover = {
        let local = pos2(f32::from(mp.x) - f32::from(bar_o.x), f32::from(mp.y) - f32::from(bar_o.y));
        (local.x >= 0.0 && local.x <= width && local.y >= 0.0 && local.y <= h).then_some(local)
    };
    let (painter, labels, hits, icons) = build(doc, ui, selection, page, width, h, settings, hover);
    let ops = painter.ops.clone();

    // GPUI mouse events arrive in WINDOW coordinates, but `build`'s hit table and the canvas
    // painter both work in the bar's LOCAL space (origin at the bar's top-left). The bar is not
    // at the window origin, so every press must be offset by the bar's real origin before it is
    // fed to `hit_at`. We capture that origin from the canvas paint closure, which receives it as
    // `b.origin` each frame — robust to any layout / resize / docks around the bar.
    let bar_origin = std::rc::Rc::new(std::cell::RefCell::new(Point::new(px(0.0), px(0.0))));

    let cursor_hits = hits.clone();
    let cursor_w = width;
    let cursor_h = h;
    let shapes: AnyElement = canvas(
        |_, _, _| (),
        {
            let bar_origin = bar_origin.clone();
            let ch = cursor_hits;
            let cw = cursor_w;
            let chh = cursor_h;
            move |b, _, window, _cx| {
                *bar_origin.borrow_mut() = b.origin;
                BAR_ORIGIN.with(|o| *o.borrow_mut() = b.origin);
                // Cursor feedback (egui L391-393 / L549-552 / L782-785 set `set_cursor_icon` for the
                // draggable bits). GPUI has no per-element cursor, so resolve it per frame from the
                // live pointer against the hit table during paint. Only when the pointer is actually
                // over the bar — when it leaves, we push nothing so the rest of the window keeps its
                // own cursors (e.g. the editable time boxes' text cursor).
                if drag_active() {
                    window.set_window_cursor_style(CursorStyle::ClosedHand);
                } else {
                    let mp = window.mouse_position();
                    let local = pos2(
                        f32::from(mp.x) - f32::from(b.origin.x),
                        f32::from(mp.y) - f32::from(b.origin.y),
                    );
                    if local.x >= 0.0 && local.x <= cw && local.y >= 0.0 && local.y <= chh {
                        // 顶边 4px 高度手柄的命中覆盖条落在 local y ∈ [0, 1.5]（其余 1.5px
                        // 溢出到画布侧）。覆盖条用 hitbox 级 ResizeUpDown，但整窗级 cursor
                        // 优先级更高，会把 hint 盖成 seek / clip 的光标 —— 这里在热区带内
                        // 也显式登记 ResizeUpDown，保证与上方的覆盖条一致。
                        if local.y <= crate::panels::HANDLE_HIT_W / 2.0 - crate::panels::HANDLE_W / 2.0 {
                            window.set_window_cursor_style(CursorStyle::ResizeUpDown);
                        } else if let Some(hit) = hit_at(&ch, local) {
                            window.set_window_cursor_style(cursor_for(hit.kind));
                        }
                    }
                }
                let p = Painter { clip: Rect::NOTHING, ops: ops.clone(), labels: Vec::new() };
                p.paint(window, pos2(f32::from(b.origin.x), f32::from(b.origin.y)));
            }
        },
    )
    .absolute()
    .size_full()
    .into_any_element();

    let q_down = queue.clone();
    let scale = scale_of(LEFT_W, width, settings.duration_ms);
    let scrolling_x = ui.scroll_x;
    let looping = settings.looping;
    let drag_doc = doc.clone();
    let drag_duration = settings.duration_ms;
    // Refresh the live geometry the window-wide move router (root view) reads, so a drag keeps
    // routing after the pointer leaves the bar and so frame-snapping/scroll-zone math stays in sync.
    GEOM.with(|g| *g.borrow_mut() = (scale, scrolling_x, width, settings.frame_ms(), settings.duration_ms));

    div()
        .id("motion-timeline")
        .flex_none()
        .w_full()
        .h(pt(h))
        .relative()
        .bg(rgba(t.panel))
        // 唯一的 1px 边线绘制在**上面**：只保留顶边描边，左右下三边不画（与左右面板
        // `border_r_1`/`border_l_1` 同一套做法）。hover 或拖拽中变强调色，状态由顶边 4px
        // 覆盖条的 on_hover 写入 `resize_drag::hovered`。
        .border_t_1()
        .border_color(rgba(if crate::resize_drag::hovered(crate::resize_drag::Edge::Bottom)
            || crate::resize_drag::drag_active(crate::resize_drag::Handle::BottomTimeline)
        {
            t.accent
        } else {
            t.border
        }))
        .child(shapes)
        .children(icon_layers(&icons))
        .children(text_layers(&labels))
        // The two editable time boxes (egui `time_field`), absolutely placed in the header.
        .child(
            div()
                .absolute()
                .left(px(49.0))
                .top(px(11.5))
                .w(px(38.0))
                .child(times.cur.clone()),
        )
        .child(
            div()
                .absolute()
                .left(px(89.0))
                .top(px(11.5))
                .w(px(38.0))
                .child(times.dur.clone()),
        )
        // 顶边高度手柄：4px 命中覆盖条（顶边描边为中心，上下各 1.5px），absolute 且
        // 后绘制，才能压住上方画布那 1.5px。latch 进与左面板共用的 `resize_drag` 状态机；
        // 整窗 move/up 由根视图 + 窗口级接管，指针移出 bar 仍连续。
        .child({
            let q = queue.clone();
            let bh = h;
            // `.id()` 升成 Stateful<Div> 才能挂 `on_hover`（只在 StatefulInteractiveElement 上）。
            div()
                .id("timeline-top-hit")
                .absolute()
                .left(px(0.0))
                .top(px(-(crate::panels::HANDLE_HIT_W / 2.0 + crate::panels::HANDLE_W / 2.0)))
                .w_full()
                .h(px(crate::panels::HANDLE_HIT_W))
                .cursor(CursorStyle::ResizeUpDown)
                // hover 状态写进 thread_local，让顶边那条 1px 描边变强调色（与左右面板一致）。
                // 这里没有 CanvasView 的 Context，直接 set + window.refresh() 触发重绘。
                .on_hover(move |hovering: &bool, window: &mut Window, _: &mut App| {
                    crate::resize_drag::set_hover(crate::resize_drag::Edge::Bottom, *hovering);
                    window.refresh();
                })
                .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    // 覆盖条压在画布 / bar 内容之上，这一下只归顶边手柄：画布不得开始框选。
                    cx.stop_propagation();
                    let tallest = (f32::from(window.viewport_size().height) - 200.0).max(crate::motion::MIN_H);
                    let double = crate::resize_drag::begin(
                        crate::resize_drag::Handle::BottomTimeline,
                        -1.0, // 上移（y 减小）→ 变高
                        bh,
                        f32::from(e.position.y),
                        crate::motion::MIN_H,
                        tallest,
                        f32::from(e.position.x),
                        f32::from(e.position.y),
                    );
                    if double {
                        q.push(Act::SetTimelineHeight(crate::motion::HEIGHT));
                        crate::resize_drag::end_drag(crate::resize_drag::Handle::BottomTimeline);
                    }
                })
        })
        .on_mouse_down(MouseButton::Left, {
            let hits = hits.clone();
            let bo = bar_origin.clone();
            move |e: &MouseDownEvent, _window, _cx| {
                // Window coords → bar-local coords (see `bar_origin` above). Without this offset the
                // whole timeline is dead: every hit test misses because the bar sits at the window's
                // bottom, not at (0,0).
                let o = *bo.borrow();
                let pos = pos2(f32::from(e.position.x) - f32::from(o.x), f32::from(e.position.y) - f32::from(o.y));
                GRABBED.with(|g| *g.borrow_mut() = None);
                BEGAN.with(|b| *b.borrow_mut() = false);
                let now = std::time::Instant::now();
                let double = LAST_PRESS.with(|lp| {
                    lp.borrow().is_some_and(|(t, x, y)| {
                        now.duration_since(t) < std::time::Duration::from_millis(400) && (x - pos.x).abs() < 4.0 && (y - pos.y).abs() < 4.0
                    })
                });
                LAST_PRESS.with(|lp| *lp.borrow_mut() = Some((now, pos.x, pos.y)));
                let hit = hit_at(&hits, pos);
                // 顶边高度手柄已移到 bar div 上那条 4px GPUI 覆盖条（latch 进 resize_drag），
                // 不再走这条 canvas 命中表。`double` 仍传给 `begin_drag` 供关键帧等双击用。
                begin_drag(&q_down, hit, pos, scale, looping, scrolling_x, &drag_doc, drag_duration, e.modifiers.shift, double);
            }
        })
        // 只在指针于时间轴内移动时请求一帧（悬停反馈按实时指针位置绘制）。拖拽本身仍由根视图
        // 的窗口级 move/up 接管：gpui 的逐元素处理器只在悬停时触发，指针一离开就断。
        .on_mouse_move({
            let hits = hits.clone();
            let bo = bar_origin.clone();
            let wake_q = queue.clone();
            move |e: &MouseMoveEvent, _window, _cx| {
                let o = *bo.borrow();
                let pos = pos2(f32::from(e.position.x) - f32::from(o.x), f32::from(e.position.y) - f32::from(o.y));
                // 位置按 4pt 格取整：同一格里移动不唤醒，跨格才请求一帧（悬停反馈够用且不空转）。
                let key = ((pos.x / 4.0) as i32, (pos.y / 4.0) as i32);
                let changed = HOVER_KEY.with(|c| c.replace(key) != key);
                let _ = &hits;
                if changed {
                    wake_q.wake();
                }
            }
        })
        .on_scroll_wheel({
            let q = queue.clone();
            let scroll = ui.scroll_y;
            let scroll_x = ui.scroll_x;
            move |e: &ScrollWheelEvent, _window, _cx| {
                let (dx, dy) = match e.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(p) => (p.x * 30.0, p.y * 30.0),
                };
                if e.modifiers.shift || dx.abs() > dy.abs() {
                    q.push(Act::ScrollX(scroll_x + dx.abs().max(dy.abs())));
                } else {
                    q.push(Act::ScrollY(scroll - dy));
                }
            }
        })
}

fn hit_at(hits: &[Hit], p: Pos2) -> Option<Hit> {
    hits.iter().rev().find(|h| h.rect.contains(p)).copied()
}

/// egui sets `set_cursor_icon` for the draggable bits (L391-393 duration, L549-552 layer summary,
/// L782-785 clip). GPUI has no per-element cursor, so we resolve it per frame from the hovered hit
/// region during paint (see the canvas closure in `bar`). GPUI has no `Move` cursor, so a middle
/// (slide) grip uses `ClosedHand` (grab) as the closest proxy.
fn cursor_for(kind: HitKind) -> CursorStyle {
    match kind {
        HitKind::ResizeH => CursorStyle::ResizeUpDown,
        HitKind::Duration => CursorStyle::ResizeLeftRight,
        HitKind::Seek => CursorStyle::ResizeLeftRight,
        HitKind::LayerEdge(_, edge) | HitKind::ClipEdge(_, _, edge) => match edge {
            ClipEdge::Move => CursorStyle::ClosedHand,
            ClipEdge::TrimStart | ClipEdge::TrimEnd => CursorStyle::ResizeLeftRight,
        },
        HitKind::Key(..) => CursorStyle::PointingHand,
        HitKind::Play
        | HitKind::Loop
        | HitKind::Fps
        | HitKind::Fold(_)
        | HitKind::FoldAll
        | HitKind::AddKeyAtPlayhead
        | HitKind::ToggleKey(..)
        | HitKind::SeekKey(_)
        | HitKind::Visible(_)
        | HitKind::TrackVisible(..)
        | HitKind::Layer(_)
        | HitKind::Track(..)
        | HitKind::Close
        | HitKind::Export
        | HitKind::Segment(..) => CursorStyle::PointingHand,
        HitKind::Clear => CursorStyle::Arrow,
    }
}

/// egui `time_of(p.x)` = (x - track_x - TIME_PAD + scroll) / scale.
fn time_at(x: f32, scale: f32, scroll_x: f32) -> u32 {
    (((x - LEFT_W - TIME_PAD + scroll_x) / scale.max(0.0001)).max(0.0)) as u32
}

#[allow(clippy::too_many_arguments)]
fn dispatch_down(
    q: &Queue,
    hit: Option<Hit>,
    pos: Pos2,
    scale: f32,
    looping: bool,
    scroll_x: f32,
    doc: &Document,
    duration: u32,
    shift: bool,
    double: bool,
) -> Option<DragKind> {
    let Some(h) = hit else { return None };
    match h.kind {
        HitKind::Seek => {
            q.push(Act::Seek(time_at(pos.x, scale, scroll_x)));
            return Some(DragKind::Seek);
        }
        HitKind::Fold(n) => q.push(Act::Fold(n)),
        HitKind::FoldAll => q.push(Act::FoldAll),
        HitKind::Play => q.push(Act::Play),
        HitKind::Loop => q.push(Act::SetLoop(!looping)),
        HitKind::Fps => q.push(Act::SetFps(0)),
        HitKind::AddKeyAtPlayhead => q.push(Act::AddKeyAtPlayhead),
        HitKind::Duration => {
            // egui L394-397: reset scroll, latch the live `drag_duration` to the current length and
            // open ONE undo step (the `BeginEdit` is pushed by `begin_drag` since this is a mutate
            // drag). The doc is only written on release via `Act::CommitDuration`.
            q.push(Act::ScrollX(0.0));
            DUR_SCROLL.with(|s| s.set(0.0));
            DUR_LAST.with(|l| *l.borrow_mut() = Some(std::time::Instant::now()));
            q.push(Act::SetDragDuration(Some(duration)));
            return Some(DragKind::Duration);
        }
        // 顶边高度手柄已迁到 bar div 的 4px GPUI 覆盖条 + `resize_drag`，canvas 命中表
        // 不再产生 `ResizeH`；保留此臂仅为 `HitKind` 穷尽匹配。
        HitKind::ResizeH => {}
        HitKind::ToggleKey(n, p) => q.push(Act::ToggleKey(n, p)),
        HitKind::SeekKey(t) => q.push(Act::Seek(t)),
        HitKind::Layer(n) => q.push(Act::Click(SelItem::Layer(n), shift)),
        HitKind::Track(n, p) => {
            // egui L581-592：左列选轨道；轨道区双击（这里 Shift 也可以）在该时间加关键帧；
            // 点空白轨道区则是「清空选中」——不是选轨道。
            if pos.x > LEFT_W {
                if double || shift {
                    q.push(Act::AddKey(n, p, time_at(pos.x, scale, scroll_x)));
                } else {
                    q.push(Act::ClearSelection);
                }
            } else {
                q.push(Act::Click(SelItem::Track(n, p), shift));
            }
        }
        HitKind::Key(n, p, t) => {
            let item = SelItem::Key(n, p, t);
            // Shift = 扩展选择；普通按下 = 直接选中（不再「再点一次取消」，否则拖拽起点会被清掉）。
            if shift {
                q.push(Act::Click(item, true));
            } else {
                q.push(Act::EnsureSelected(item));
            }
            // 拖关键帧时画布要跟着预览（egui `kresp.dragged()` → `scrubbing = true`）。
            q.push(Act::SetScrubbing(true));
            GRABBED.with(|g| *g.borrow_mut() = Some(crate::motion::SelectedKey { node: n, property: p, time_ms: t }));
        }
        HitKind::Segment(n, p, t) => {
            // egui L700-702：点中两帧之间的线就是「选中这一段」——不再顺带把起点关键帧也选上。
            let item = SelItem::Segment(n, p, t);
            if shift {
                q.push(Act::Click(item, true));
            } else {
                q.push(Act::EnsureSelected(item));
            }
        }
        HitKind::ClipEdge(node, id, edge) => {
            q.push(Act::Click(SelItem::Clip(node, id), shift));
            q.push(Act::OpenClipPopover(node, id));
            if let Some(orig) = doc.node(node).and_then(|n| n.motion.clips.iter().find(|c| c.id == id)).cloned() {
                q.push(Act::BeginClipDrag { node, id, edge, origin_x: pos.x, orig: Box::new(orig) });
                return Some(DragKind::Clip);
            }
        }
        HitKind::LayerEdge(node, edge) => {
            q.push(Act::Click(SelItem::Layer(node), shift));
            if let Some(orig) = doc.node(node).map(|n| n.motion.clone()) {
                // egui anchors the time-stretch to the layer's own span, not the whole timeline.
                let (lo, hi) = doc.node(node).and_then(|n| n.motion.span_ms(duration)).unwrap_or((0, duration));
                q.push(Act::BeginLayerDrag { node, edge, origin_x: pos.x, lo, hi, orig: Box::new(orig) });
                return Some(DragKind::Layer);
            }
        }
        HitKind::Visible(n) => q.push(Act::ToggleVisible(n)),
        HitKind::TrackVisible(n, p) => q.push(Act::ToggleTrackVisible(n, p)),
        HitKind::Close => q.push(Act::CloseMotion),
        // egui L267-268: the export button is a no-op there — leave the selection intact.
        HitKind::Export => {}
        HitKind::Clear => q.push(Act::ClearSelection),
    }
    None
}

/// Press logic shared by the real `on_mouse_down` handler and the regression tests.
///
/// Hit-tests, pushes the initial `Act`, and latches the drag / key state into the cross-frame
/// `thread_local`s (see the long note on `DRAG`/`GRABBED`). Returns whether an undo step was
/// opened — only the doc-mutating drags (clip bar / layer summary / keyframe grab) do; a plain
/// click, a ruler seek or a duration drag must NOT (egui records no interaction for those).
fn begin_drag(
    q: &Queue,
    hit: Option<Hit>,
    pos: Pos2,
    scale: f32,
    looping: bool,
    scroll_x: f32,
    doc: &Document,
    duration: u32,
    shift: bool,
    double: bool,
) -> bool {
    GRABBED.with(|g| *g.borrow_mut() = None);
    BEGAN.with(|b| *b.borrow_mut() = false);
    let started = dispatch_down(q, hit, pos, scale, looping, scroll_x, doc, duration, shift, double);
    // Only the doc-mutating drags open an undo step. The duration handle is one of them
    // (egui opens "Animation duration" on drag start, commits on release), so it is included;
    // a plain click, a ruler seek or a keyframe grab must NOT.
    let mutate = matches!(started, Some(DragKind::Clip) | Some(DragKind::Layer) | Some(DragKind::Duration)) || GRABBED.with(|g| g.borrow().is_some());
    if mutate {
        // 名称照 egui：拖关键帧是 "Move keyframe"，拖时长是 "Animation duration"，拖片段/图层是
        // "Move animation"。高度拖拽不写文档，所以不算 edit。
        let label = if GRABBED.with(|g| g.borrow().is_some()) {
            "Move keyframe"
        } else if matches!(started, Some(DragKind::Duration)) {
            "Animation duration"
        } else {
            "Move animation"
        };
        q.push(Act::BeginEdit(label));
        BEGAN.with(|b| *b.borrow_mut() = true);
    }
    DRAG.with(|d| *d.borrow_mut() = started);
    mutate
}

/// Move routing shared by the real `on_mouse_move` handler and the regression tests.
///
/// Reads the drag / key state latched by `begin_drag`. ⛔ Must NOT re-hit-test: the pointer
/// leaves the ruler / bar mid-drag, and re-testing is what made the playhead and duration handle
/// die after two pixels in the original egui port.
/// Nearest frame boundary to `t_ms` (mirrors egui `snap_to_frame`, which is `(t/frame_ms).round()*frame_ms`).
fn snap_frame(t_ms: f32, frame_ms: f32) -> u32 {
    ((t_ms / frame_ms).round() * frame_ms) as u32
}

fn route_drag_move(q: &Queue, scale: f32, scroll_x: f32, pos: Pos2) {
    let dragging = DRAG.with(|d| d.borrow().is_some()) || GRABBED.with(|g| g.borrow().is_some());
    if !dragging {
        return;
    }
    // Geometry for the frame snap and the duration scroll zone (refreshed every frame in `bar()`).
    let (_, _, track_right, frame_ms, dur_ms) = GEOM.with(|g| *g.borrow());
    match DRAG.with(|d| *d.borrow()) {
        Some(DragKind::Seek) => {
            // egui L360: snap to frame, clamp to the current duration.
            q.push(Act::SetScrubbing(true));
            let t_ms = time_at(pos.x, scale, scroll_x);
            let s = (t_ms as f32).clamp(0.0, dur_ms as f32);
            q.push(Act::Seek(snap_frame(s, frame_ms)));
        }
        Some(DragKind::Duration) => advance_duration(q, scale, track_right, frame_ms, pos.x),
        // Clip / layer bars: `DragMove` recomputes from the snapshot taken on press.
        Some(DragKind::Clip) | Some(DragKind::Layer) => {
            q.push(Act::DragMove { x: pos.x, pxms: scale });
        }
        None => {}
    }
    if let Some(k) = GRABBED.with(|g| *g.borrow()) {
        // A grabbed keyframe snaps to the frame grid while dragging (egui L382 `snap`).
        q.push(Act::SetScrubbing(true));
        let s = (time_at(pos.x, scale, scroll_x) as f32).clamp(0.0, dur_ms as f32);
        let snapped = snap_frame(s, frame_ms);
        q.push(Act::MoveKey(k, snapped));
        // 关键帧已经搬到新时间：抓取状态必须跟着走。否则下一个 move 去找原时间上的键
        // 找不到，`move_key` 直接空转 —— 表现就是「只挪一下，然后怎么拖都不动」。
        if snapped != k.time_ms {
            GRABBED.with(|g| *g.borrow_mut() = Some(crate::motion::SelectedKey { time_ms: snapped, ..k }));
        }
    }
}

/// 时长手柄的一次推进（move 事件和「按住不动」的续帧都走这里，保证两条路径完全一致）。
///
/// 关键点：**算时间用的 scroll 就是这一帧要写回去的 scroll**。以前时间用上一帧的 scroll、
/// 滚动用新值，于是指针停在右端时「先冻住、再猛跳」，看起来就是拖出去以后跟鼠标断开了。
fn advance_duration(q: &Queue, scale: f32, track_right: f32, frame_ms: f32, x: f32) {
    let now = std::time::Instant::now();
    let dt = DUR_LAST.with(|l| {
        let mut slot = l.borrow_mut();
        let prev = slot.replace(now);
        prev.map(|p| now.duration_since(p).as_secs_f32().clamp(0.0, 0.05)).unwrap_or(0.0)
    });
    // The last 48 pt of the track is the scroll zone, ramped 0→1,速度按真实 dt 算（px/s）。
    let zone = 48.0_f32.min(track_right - LEFT_W - TIME_PAD).max(1.0);
    let ramp = if x > track_right - zone { ((x - (track_right - zone)) / zone).clamp(0.0, 1.0) } else { 0.0 };
    let scroll = if ramp > 0.0 {
        let s = (DUR_SCROLL.with(|s| s.get()) + ramp * SCROLL_SPEED * dt).max(0.0);
        DUR_SCROLL.with(|v| v.set(s));
        s
    } else {
        DUR_SCROLL.with(|s| s.get())
    };
    if ramp > 0.0 {
        q.push(Act::ScrollX(scroll));
    }
    let t = time_at(x.min(track_right), scale, scroll) as f32;
    let snapped = (t / frame_ms.max(0.1)).round() * frame_ms;
    let ms = snapped.clamp(100.0, TimelineSettings::MAX_DURATION_MS as f32) as u32;
    q.push(Act::SetDragDuration(Some(ms)));
}

/// 时长手柄是否处于「指针停在滚动区、需要继续推进」的状态（续帧用）。
pub fn duration_drag_active() -> bool {
    matches!(DRAG.with(|d| *d.borrow()), Some(DragKind::Duration))
}

/// 指针停住不动时也要继续推进时长滚动（egui 用 `request_repaint`）：返回 true 表示本帧
/// 又推了一次动作，调用方应当再请求一帧。
pub fn tick_duration(q: &Queue, pos: Pos2) -> bool {
    if !duration_drag_active() {
        return false;
    }
    let (scale, _, track_right, frame_ms, _) = GEOM.with(|g| *g.borrow());
    let zone = 48.0_f32.min(track_right - LEFT_W - TIME_PAD).max(1.0);
    if pos.x <= track_right - zone {
        return false;
    }
    advance_duration(q, scale, track_right, frame_ms, pos.x);
    true
}

// ---------------------------------------------------------------- 根视图接管（窗口级拖拽延续）

/// Window-space → bar-local conversion for a move arriving at the root view (which spans the whole
/// window, so its `on_mouse_move` fires even when the pointer has left the timeline bar). Reads the
/// bar origin captured each frame and routes through the shared [`route_drag_move`].
pub fn route_global_move(q: &Queue, window_pos: Point<Pixels>) {
    // 顶边高度手柄已迁到 `resize_drag::Handle::BottomTimeline`，由根视图 / 窗口级
    // 通用路由处理；这里只负责 timeline 自身的其它拖拽（seek / duration / clip / layer）。
    let o = BAR_ORIGIN.with(|o| *o.borrow());
    let (scale, scroll_x, _, _, _) = GEOM.with(|g| *g.borrow());
    let pos = pos2(f32::from(window_pos.x) - f32::from(o.x), f32::from(window_pos.y) - f32::from(o.y));
    route_drag_move(q, scale, scroll_x, pos);
}

/// 时间轴在窗口里的左上角（上一帧记下的值，根视图做窗口坐标换算用）。
pub fn bar_origin() -> Point<Pixels> {
    BAR_ORIGIN.with(|o| *o.borrow())
}

/// Whether a Motion timeline drag is currently in progress (so the root view can take over the move).
pub fn drag_active() -> bool {
    DRAG.with(|d| d.borrow().is_some()) || GRABBED.with(|g| g.borrow().is_some())
}

/// End the in-progress drag from the root view: commit the duration (if any), close the undo step
/// and clear the drag/key state. Mirrors the bar's old `on_mouse_up`, but reachable even when the
/// pointer is outside the bar (incl. released outside the window).
pub fn end_drag(q: &Queue) {
    let kind = DRAG.with(|d| *d.borrow());
    GRABBED.with(|g| *g.borrow_mut() = None);
    DRAG.with(|d| *d.borrow_mut() = None);
    DUR_SCROLL.with(|s| s.set(0.0));
    if matches!(kind, Some(DragKind::Duration)) {
        q.push(Act::CommitDuration);
    }
    if BEGAN.with(|b| std::mem::replace(&mut *b.borrow_mut(), false)) {
        q.push(Act::EndEdit);
    }
    q.push(Act::EndDrag);
    q.push(Act::SetScrubbing(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::sample_doc;
    use std::rc::Rc;

    /// The timeline's drag state lives in `thread_local`s, not per-frame `Rc`s. Reset them so a
    /// test does not inherit state from a previous one (the statics are process-wide).
    fn reset_drag_state() {
        DRAG.with(|d| *d.borrow_mut() = None);
        GRABBED.with(|g| *g.borrow_mut() = None);
        BEGAN.with(|b| *b.borrow_mut() = false);
    }

    fn doc() -> Document {
        sample_doc()
    }

    /// Regression test for the "handles won't drag at all" bug. The root cause was that the
    /// drag state (`DRAG`/`GRABBED`) was stored in a `Rc<RefCell<_>>` created fresh inside
    /// `bar()` every frame. `q.push` triggers a wake → re-render → `bar()` runs again → a brand
    /// new empty `Rc`, so `on_mouse_move` read `None` and returned early — the playhead / duration
    /// handle / clips never followed the pointer. The fix keeps the state in `thread_local`, which
    /// survives the per-frame rebuild.
    ///
    /// This test reproduces exactly that: press (latches `DRAG`), then **simulate the per-frame
    /// rebuild by NOT touching the thread-locals** (the old code would have reset them), then move.
    /// If the drag state were lost, `route_drag_move` would push nothing and the assertion fails.
    #[test]
    fn playhead_drag_survives_per_frame_rebuild() {
        reset_drag_state();
        let q = Queue::new(Rc::new(|| {}));
        // A Seek hit covering everything: pressing anywhere starts dragging the playhead.
        let hits = vec![Hit { rect: Rect { min: pos2(-9999.0, -9999.0), max: pos2(9999.0, 9999.0) }, kind: HitKind::Seek }];
        let pos = pos2(300.0, 10.0);

        // 1) Press: latches DRAG = Seek and pushes one initial Act::Seek.
        let mutate = begin_drag(&q, hit_at(&hits, pos), pos, 1.0, false, 0.0, &doc(), 2000, false, false);
        assert!(!mutate, "a playhead seek is not a doc-mutating drag, no undo should open");
        assert!(matches!(DRAG.with(|d| *d.borrow()), Some(DragKind::Seek)), "press must latch DRAG = Seek");
        assert_eq!(q.drain().len(), 1, "press pushed one initial Act::Seek");

        // 2) Simulate the re-render (bar() runs again): the old per-frame Rc would reset DRAG to
        //    None here. The thread-local is untouched — this is what the fix relies on.

        // 3) Move: must route by the latched drag state and push Act::Seek, not re-hit-test.
        let q_move = Queue::new(Rc::new(|| {}));
        route_drag_move(&q_move, 1.0, 0.0, pos2(360.0, 10.0));
        let acts = q_move.drain();
        assert!(
            acts.iter().any(|a| matches!(a, Act::Seek(_))),
            "a move must actually push an Act::Seek, otherwise it is the 'won't drag' bug"
        );
        assert!(
            acts.iter().any(|a| matches!(a, Act::SetScrubbing(true))),
            "拖播放头要把 scrubbing 打开（画布要跟着预览）"
        );
    }

    /// The duration handle is a drag (latches DRAG = Duration) and, after a simulated per-frame
    /// rebuild, still routes the move to `Act::SetDragDuration` (the live preview). Like egui it
    /// opens ONE undo step (BeginEdit on press, EndEdit on release), so `mutate` is now true.
    #[test]
    fn duration_drag_routes_after_rebuild() {
        reset_drag_state();
        let q = Queue::new(Rc::new(|| {}));
        let hits = vec![Hit { rect: Rect { min: pos2(-9999.0, -9999.0), max: pos2(9999.0, 9999.0) }, kind: HitKind::Duration }];
        let pos = pos2(400.0, 10.0);
        let mutate = begin_drag(&q, hit_at(&hits, pos), pos, 1.0, false, 0.0, &doc(), 2000, false, false);
        assert!(mutate, "duration drag opens ONE undo step (BeginEdit on press, EndEdit on release)");
        assert!(BEGAN.with(|b| *b.borrow()), "BEGAN must be true for a duration drag (balanced Begin/EndEdit)");
        assert!(matches!(DRAG.with(|d| *d.borrow()), Some(DragKind::Duration)));

        let q_move = Queue::new(Rc::new(|| {}));
        route_drag_move(&q_move, 1.0, 0.0, pos2(420.0, 10.0));
        let acts = q_move.drain();
        assert!(matches!(acts.first(), Some(Act::SetDragDuration(_))), "a duration move should push Act::SetDragDuration (live preview)");
    }

    /// With no latched drag state, a move must be a complete no-op (no accidental edits).
    #[test]
    fn move_without_drag_is_a_noop() {
        reset_drag_state();
        let q = Queue::new(Rc::new(|| {}));
        route_drag_move(&q, 1.0, 0.0, pos2(123.0, 10.0));
        assert!(q.drain().is_empty(), "a move with no drag state must push nothing");
    }

    /// The root view takes over a drag once the pointer leaves the bar (gpui's per-element move
    /// handler only fires while hovered). This regression test proves the window→bar coordinate
    /// conversion in `route_global_move` still routes a DURATION drag when the pointer has moved
    /// ABOVE the bar (window y smaller than the bar's origin): the geometry thread-locals are read
    /// and a `SetDragDuration` is pushed, so "drag the total time, mouse leaves the bar → it dies"
    /// stays fixed.
    #[test]
    fn global_move_routes_duration_drag_when_pointer_leaves_bar() {
        reset_drag_state();
        // The bar sits at window y = 600 (bottom of the window); scale 1 px/ms, no scroll.
        BAR_ORIGIN.with(|o| *o.borrow_mut() = Point::new(px(0.0), px(600.0)));
        GEOM.with(|g| *g.borrow_mut() = (1.0, 0.0, 1000.0, 1000.0 / 60.0, 2000));
        let q = Queue::new(Rc::new(|| {}));
        let hits = vec![Hit {
            rect: Rect { min: pos2(-9999.0, -9999.0), max: pos2(9999.0, 9999.0) },
            kind: HitKind::Duration,
        }];
        // Press at window (420, 610) → bar-local (420, 10): latches DRAG = Duration.
        begin_drag(&q, hit_at(&hits, pos2(420.0, 10.0)), pos2(420.0, 10.0), 1.0, false, 0.0, &doc(), 2000, false, false);
        assert!(drag_active(), "a duration press must latch a drag");

        // Pointer now ABOVE the bar at window (450, 540) → bar-local (450, -60). Must still route.
        let qm = Queue::new(Rc::new(|| {}));
        route_global_move(&qm, Point::new(px(450.0), px(540.0)));
        let acts = qm.drain();
        assert!(
            matches!(acts.first(), Some(Act::SetDragDuration(_))),
            "a duration drag must keep routing even when the pointer leaves the bar"
        );
    }

    /// 一份带 Position 轨道、两个关键帧（0ms / 500ms）的文档。
    fn doc_with_keys() -> (Document, NodeId) {
        use figcraft_engine::Session;
        let mut s = Session::with_document(Document::new("timeline-test"));
        let v = s
            .execute("shape.rectangle", &serde_json::json!({ "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0 }))
            .expect("rect");
        let id: NodeId = serde_json::from_value(v["id"].clone()).expect("id");
        s.execute("select.set", &serde_json::json!({ "ids": [id] })).expect("select");
        for t in [0u32, 500] {
            s.execute("motion.keyframe", &serde_json::json!({ "property": "Position", "time": t }))
                .expect("keyframe");
        }
        let doc = s.doc().expect("doc").clone();
        assert!(doc.node(id).is_some_and(|n| !n.motion.tracks.is_empty()), "测试文档应有轨道");
        (doc, id)
    }

    struct Geom {
        doc: Document,
        id: NodeId,
        settings: TimelineSettings,
        scale: f32,
        width: f32,
        key_y: f32,
        x0: f32,
    }

    fn geom() -> Geom {
        let (doc, id) = doc_with_keys();
        let settings = settings_of(&doc, doc.pages[0]);
        let width = 1200.0;
        let scale = scale_of(LEFT_W, width, settings.duration_ms);
        // 行序：Layer 在 55，Position 轨道在 79..103（ROW_H = 24）。
        let key_y = HEADER_H + 8.0 + ROW_H * 1.5;
        Geom { doc, id, settings, scale, width, key_y, x0: LEFT_W + TIME_PAD }
    }

    fn build_hits(g: &Geom, hover: Option<Pos2>) -> Vec<Hit> {
        let mut ui = MotionUi::default();
        ui.selection.insert(SelItem::Layer(g.id));
        let (_, _, hits, _) = build(&g.doc, &ui, &[g.id], g.doc.pages[0], g.width, 309.0, g.settings, hover);
        hits
    }

    /// 回归：整行「选轨道 / 清空」的命中**不能**压住关键帧和 segment。
    /// 之前它被最后注册，而 `hit_at` 取最后一个匹配 → 菱形和两帧之间的线全都点不到，
    /// 表现就是「帧拖不动、线选不中」。
    #[test]
    fn keyframe_and_segment_win_over_the_row_lane_hit() {
        reset_drag_state();
        let g = geom();
        let hits = build_hits(&g, None);
        let at_key = hit_at(&hits, pos2(g.x0 + 1.0, g.key_y)).map(|h| h.kind);
        assert!(matches!(at_key, Some(HitKind::Key(id, AnimProperty::Position, 0)) if id == g.id), "按在菱形上必须是关键帧命中，实得 {at_key:?}");

        let mid_x = g.x0 + 250.0 * g.scale;
        let at_seg = hit_at(&hits, pos2(mid_x, g.key_y)).map(|h| h.kind);
        assert!(matches!(at_seg, Some(HitKind::Segment(id, AnimProperty::Position, 0)) if id == g.id), "两帧之间的线必须可选，实得 {at_seg:?}");

        // 空白轨道区仍然是「清空选中」，不是选轨道。
        let q = Queue::new(Rc::new(|| {}));
        let empty = pos2(g.x0 + 600.0, g.key_y);
        let hit = hit_at(&hits, empty);
        begin_drag(&q, hit, empty, g.scale, false, 0.0, &g.doc, g.settings.duration_ms, false, false);
        assert!(q.drain().iter().any(|a| matches!(a, Act::ClearSelection)), "点空白轨道应清空选中");
    }

    /// 回归：拖关键帧要能**连续**跟手。以前第一次 MoveKey 之后 `GRABBED` 还指着旧时间，
    /// 下一次移动找不到那个键 → `move_key` 空转，看起来就是「帧只挪一下就不动了」。
    #[test]
    fn keyframe_drag_keeps_moving_after_the_first_step() {
        reset_drag_state();
        let g = geom();
        let hits = build_hits(&g, None);
        let start = pos2(g.x0 + 1.0, g.key_y);
        let q = Queue::new(Rc::new(|| {}));
        begin_drag(&q, hit_at(&hits, start), start, g.scale, false, 0.0, &g.doc, g.settings.duration_ms, false, false);
        assert!(GRABBED.with(|x| x.borrow().is_some()), "按下关键帧要抓住它");
        assert!(q.drain().iter().any(|a| matches!(a, Act::BeginEdit("Move keyframe"))), "拖关键帧开一次 undo");
        GEOM.with(|gg| *gg.borrow_mut() = (g.scale, 0.0, g.width, g.settings.frame_ms(), g.settings.duration_ms));

        let mut last = 0u32;
        for step in 1..=3u32 {
            let qm = Queue::new(Rc::new(|| {}));
            route_drag_move(&qm, g.scale, 0.0, pos2(g.x0 + 120.0 * step as f32, g.key_y));
            let moved = qm.drain().into_iter().find_map(|a| match a {
                Act::MoveKey(k, t) => Some((k.time_ms, t)),
                _ => None,
            });
            let (from, to) = moved.expect("每一步都要继续推 MoveKey");
            assert!(to > last, "第 {step} 步必须继续往右挪（上一目标 {last}，这次 {to}）");
            assert_eq!(from, last, "第二次起要从「上一次的目标时间」搬，而不是原时间");
            last = to;
            assert_eq!(GRABBED.with(|x| x.borrow().map(|k| k.time_ms)), Some(to), "抓取状态要跟到新时间");
        }
    }

    /// 回归：时长手柄向右拖。算时间用的 scroll 必须和这一帧写回去的 scroll 完全一致，
    /// 否则会出现「冻住 → 猛跳」（用户看到的「向右不是正常人」）。这里直接给定累计滚动，
    /// 断言时长按同一个 scroll 换算，并且随指针右移单调增长。
    #[test]
    fn duration_drag_right_uses_the_same_scroll_it_pushes() {
        reset_drag_state();
        let g = geom();
        GEOM.with(|gg| *gg.borrow_mut() = (g.scale, 0.0, g.width, g.settings.frame_ms(), g.settings.duration_ms));
        let q = Queue::new(Rc::new(|| {}));
        let hits = build_hits(&g, None);
        let handle = pos2(g.x0 + g.settings.duration_ms as f32 * g.scale, g.key_y);
        begin_drag(&q, hit_at(&hits, handle), handle, g.scale, false, 0.0, &g.doc, g.settings.duration_ms, false, false);
        assert!(matches!(DRAG.with(|d| *d.borrow()), Some(DragKind::Duration)));
        q.drain();

        // 模拟已经累积了 200px 的滚动，指针仍在右端：时长必须把这段滚动算进去。
        DUR_SCROLL.with(|s| s.set(200.0));
        DUR_LAST.with(|l| *l.borrow_mut() = Some(std::time::Instant::now()));
        let qm = Queue::new(Rc::new(|| {}));
        route_drag_move(&qm, g.scale, 0.0, pos2(g.width, g.key_y));
        let acts = qm.drain();
        let scrolled = acts.iter().find_map(|a| match a {
            Act::ScrollX(x) => Some(*x),
            _ => None,
        });
        assert!(scrolled.is_some_and(|x| x >= 200.0), "向右拖必须继续推进滚动");
        let ms = acts.iter().find_map(|a| match a {
            Act::SetDragDuration(v) => *v,
            _ => None,
        }).expect("必须推 SetDragDuration");
        let expect = ((g.width - LEFT_W - TIME_PAD + 200.0) / g.scale) as f32;
        assert!((ms as f32 - expect).abs() < g.settings.frame_ms() * 2.0, "时长要按同一个 scroll 换算：期望≈{expect:.0}ms，实得 {ms}ms");
        assert!(ms > g.settings.duration_ms, "在右端往外拖应该变长");
    }

    /// 回归：时长手柄向左拖是线性的（指针往左 = 变短），不应该被滚动区影响。
    #[test]
    fn duration_drag_left_is_linear() {
        reset_drag_state();
        let g = geom();
        GEOM.with(|gg| *gg.borrow_mut() = (g.scale, 0.0, g.width, g.settings.frame_ms(), g.settings.duration_ms));
        let q = Queue::new(Rc::new(|| {}));
        let hits = build_hits(&g, None);
        let handle = pos2(g.x0 + g.settings.duration_ms as f32 * g.scale, g.key_y);
        begin_drag(&q, hit_at(&hits, handle), handle, g.scale, false, 0.0, &g.doc, g.settings.duration_ms, false, false);
        q.drain();
        DUR_SCROLL.with(|s| s.set(0.0));
        DUR_LAST.with(|l| *l.borrow_mut() = Some(std::time::Instant::now()));

        let mut prev = f32::MAX;
        for dx in [700.0f32, 600.0, 500.0] {
            let qm = Queue::new(Rc::new(|| {}));
            route_drag_move(&qm, g.scale, 0.0, pos2(dx, g.key_y));
            let ms = qm.drain().into_iter().find_map(|a| match a {
                Act::SetDragDuration(v) => v,
                _ => None,
            }).expect("向左也要推时长");
            assert!((ms as f32) < prev, "指针左移时长要变短（{prev} → {ms}）");
            prev = ms as f32;
        }
    }

    /// 悬停：行悬停才画眼睛、菱形变亮 —— 这是本次补上的 egui 悬停反馈（以前只有一个跟着
    /// 鼠标跑的黑色小方块，既不是 tooltip 也挡内容，已删除）。
    #[test]
    fn hover_drives_the_eye_and_the_diamond() {
        reset_drag_state();
        let g = geom();
        let no_hover = build_hits(&g, None);
        let with_hover = build_hits(&g, Some(pos2(g.x0 + 1.0, g.key_y)));
        // 命中表本身不受 hover 影响（悬停只改绘制），但命中项必须仍然稳定。
        assert_eq!(no_hover.len(), with_hover.len());
        // 眼睛的命中区一直在（否则悬停出现后点不到）；绘制由 hover 决定，见 build 内 row_hovered。
        let layer_y = HEADER_H + 8.0 + ROW_H * 0.5;
        let eye = hit_at(&with_hover, pos2(LEFT_W - 15.0, layer_y)).map(|h| h.kind);
        assert!(matches!(eye, Some(HitKind::Visible(id)) if id == g.id), "图层行的眼睛命中区要一直在，实得 {eye:?}");
        let track_eye = hit_at(&with_hover, pos2(LEFT_W - 15.0, g.key_y)).map(|h| h.kind);
        assert!(matches!(track_eye, Some(HitKind::TrackVisible(id, AnimProperty::Position)) if id == g.id), "轨道的眼睛命中区也要在，实得 {track_eye:?}");
    }
}

