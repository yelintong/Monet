//! Prototype 演示层 —— `ui-egui/src/present/{player,draw}.rs` 的 GPUI 移植（核心播放行为）。
//!
//! 引擎 `proto.present` / `proto.preview` 只发 `ui_requests`；这里由 `sync_ui_requests` 打开。
//! 画面用渲染层的 `figcraft_render::render_node_at(doc, frame, scale, time_ms)` 按时间求值，
//! 所以 **motion clip / 关键帧动画在演示里真的会动**（不只是静态截图）。
//! 点击画面执行当前 frame 的第一条交互（Navigate / ChangeTo / OpenOverlay / Back），
//! 过渡期间用 `figcraft_doc::anim::transitions::poses()` 摆出/淡入两张帧图。
//!
//! 与 egui 的差异：没有独立 Present 窗口 / 设备外壳 / hotspot 高亮 / 缩放档位 / Inspect 侧栏；
//! 这里是一个覆盖窗口的演示层，保留播放、导航、过渡、退出这几条主行为。

use crate::app::CanvasApp;
use crate::canvas::render_worker::screen_image;
use crate::widgets::{Icon, IconButton, pt, rgba, tokens};
use figcraft_doc::proto::{Action, AnimationOp, Transition, Trigger};
use figcraft_doc::{Document, NodeId};
use figcraft_i18n::tr;
use gpui::{Div, ImageSource, MouseButton, SharedString, Stateful, div, img, prelude::*, px};
use std::collections::BTreeMap;
use std::time::Instant;

/// 一个画框（动画组）挂起时记住的时钟。画框之间独立，各自持有播放头与是否在播。
/// current 画框的权威时钟直接落在 `PresentState.time_ms/playing`，这里只存 **非 current** 画框。
#[derive(Clone, Copy, Debug, PartialEq)]
struct MotionClock {
    playing: bool,
    base_ms: f64,
}

/// 对齐 desigma `animation_control` 的 op 语义，作用于一个画框的时钟。
/// `at_ms` 是该画框当前播放头，`dur_ms` 是它的总时长（>=1）。
///
/// * `Play{from_start:false}`：正在播且没到头 → 幂等不动；已到头 → 从头播；否则续播。
/// * `Play{from_start:true}` / `Restart`：从头播。
/// * `Pause`：停在播放头。
/// * `TogglePlay`：播着且没到头 → 暂停；否则播放（到头则从头）。
fn apply_animation_op(op: AnimationOp, cur: MotionClock, at_ms: f64, dur_ms: f64) -> MotionClock {
    let finished = at_ms >= dur_ms;
    let play = |base: f64| MotionClock { playing: true, base_ms: base.min(dur_ms) };
    match op {
        AnimationOp::Play { from_start: false } if cur.playing && !finished => cur,
        AnimationOp::Play { from_start: false } => play(if finished { 0.0 } else { at_ms }),
        AnimationOp::Play { from_start: true } | AnimationOp::Restart => play(0.0),
        AnimationOp::Pause => MotionClock { playing: false, base_ms: at_ms },
        AnimationOp::TogglePlay if cur.playing && !finished => MotionClock { playing: false, base_ms: at_ms },
        AnimationOp::TogglePlay => play(if finished { 0.0 } else { at_ms }),
    }
}

/// 一次演示的运行时状态。
#[derive(Clone, Debug)]
pub struct PresentState {
    pub start: NodeId,
    pub current: NodeId,
    pub history: Vec<NodeId>,
    /// 动画播放头（ms）；传给 `render_node_at` 让 motion clip 按时间求值。
    pub time_ms: f64,
    pub playing: bool,
    pub last: Instant,
    /// 导航过渡：上一帧、开始时刻、时长、类型。
    pub from: Option<NodeId>,
    pub transition_start: Instant,
    pub transition_ms: u32,
    pub transition: Transition,
    /// hotspot 提示（点空白处闪一下可点区域）——这里简化为顶栏开关。
    pub hints: bool,
    pub preview: bool,
    /// 非 current 画框各自的挂起时钟；导航离开 / 进入时存取，落实「画框之间独立」。
    clocks: BTreeMap<NodeId, MotionClock>,
}

impl PresentState {
    pub fn new(start: NodeId, preview: bool) -> Self {
        Self {
            start,
            current: start,
            history: Vec::new(),
            time_ms: 0.0,
            playing: false,
            last: Instant::now(),
            from: None,
            transition_start: Instant::now(),
            transition_ms: 0,
            transition: Transition::Instant,
            hints: true,
            preview,
            clocks: BTreeMap::new(),
        }
    }

    /// 推进播放头 / 过渡时钟。返回是否需要继续请求动画帧。
    pub fn advance(&mut self, doc: &Document) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = now;
        if self.playing {
            self.time_ms += dt;
            // Per-frame animation group (用户 2026-11-12)：the current frame owns the timeline;
            // a frame with no entry of its own inherits its page's (settings_of does the fallback).
            let st = crate::motion::settings_of(doc, self.current);
            let dur = st.duration_ms.max(1) as f64;
            if self.time_ms > dur {
                if st.looping {
                    self.time_ms %= dur;
                } else {
                    // D3 Control：one-shot 播到头停在末帧（desigma 的 `finished`），交回 Control
                    // animation —— Restart / Play 才能再次启动。
                    self.time_ms = dur;
                    self.playing = false;
                }
            }
        }
        let in_transition = self.from.is_some() && now.duration_since(self.transition_start).as_secs_f64() * 1000.0 < self.transition_ms as f64;
        if !in_transition {
            self.from = None;
        }
        self.playing || in_transition
    }

    /// 点击兜底：没有命中任何子图层 hotspot 时，执行当前画框自身的第一条交互（保留旧行为——
    /// 画框本身挂的交互点画面任意处都能触发）。命中检测走 [`PresentState::fire`]。
    pub fn click(&mut self, doc: &Document) {
        let Some(it) = doc.node(self.current).and_then(|n| n.interactions.first()).cloned() else { return };
        self.fire_actions(doc, &it);
    }

    /// 命中子图层后，执行 `node` 第 `k` 条交互的 actions（egui `fire` 的 GPUI 版）。
    pub fn fire(&mut self, doc: &Document, node: NodeId, k: usize) {
        let Some(it) = doc.node(node).and_then(|n| n.interactions.get(k)).cloned() else { return };
        self.fire_actions(doc, &it);
    }

    /// 一条交互的执行体：先跑 `Action::Animation` 控制画框时钟（不导航），再执行第一条
    /// 导航类动作切帧。对「画框自身」和「命中的子图层」一视同仁。
    fn fire_actions(&mut self, doc: &Document, it: &figcraft_doc::proto::Interaction) {
        // D3 Control：控制类动作不导航，逐个作用于目标画框（dest 落在画框内也按整框算）。
        for a in &it.actions {
            if let Action::Animation { dest, op } = a {
                self.control_animation(doc, *dest, *op);
            }
        }
        let dest = it.actions.iter().find_map(|a| match a {
            Action::Navigate { dest, .. }
            | Action::ScrollTo { dest, .. }
            | Action::OpenOverlay { dest, .. }
            | Action::SwapOverlay { dest }
            | Action::ChangeTo { dest } => Some(*dest),
            Action::Back => self.history.pop(),
            _ => None,
        });
        let Some(next) = dest.filter(|d| doc.node(*d).is_some()) else { return };
        if next == self.current {
            return;
        }
        // 画框之间独立：离开前存下 current 的时钟，进入时恢复 next 的挂起时钟（没有则 0/不播）。
        self.clocks.insert(self.current, MotionClock { playing: self.playing, base_ms: self.time_ms });
        self.history.push(self.current);
        self.from = Some(self.current);
        self.current = next;
        let c = self.clocks.get(&next).copied().unwrap_or(MotionClock { playing: false, base_ms: 0.0 });
        self.time_ms = c.base_ms;
        self.playing = c.playing;
        self.transition_start = Instant::now();
        self.transition_ms = it.duration_ms;
        self.transition = it.transition;
    }

    /// 把 `op` 作用于 `dest` 所在的画框（动画组）。控制当前画框直接改 time_ms/playing；
    /// 控制其它画框只更新它的挂起时钟，等导航过去时再恢复。
    fn control_animation(&mut self, doc: &Document, dest: NodeId, op: AnimationOp) {
        let scope = doc.top_level_frame(dest).unwrap_or(dest);
        let dur = crate::motion::settings_of(doc, scope).duration_ms.max(1) as f64;
        if scope == self.current {
            let cur = MotionClock { playing: self.playing, base_ms: self.time_ms };
            let next = apply_animation_op(op, cur, self.time_ms, dur);
            self.playing = next.playing;
            self.time_ms = next.base_ms;
        } else {
            let cur = self.clocks.get(&scope).copied().unwrap_or(MotionClock { playing: false, base_ms: 0.0 });
            let next = apply_animation_op(op, cur, cur.base_ms, dur);
            self.clocks.insert(scope, next);
        }
    }

    pub fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.clocks.insert(self.current, MotionClock { playing: self.playing, base_ms: self.time_ms });
            self.from = Some(self.current);
            self.current = prev;
            let c = self.clocks.get(&prev).copied().unwrap_or(MotionClock { playing: false, base_ms: 0.0 });
            self.time_ms = c.base_ms;
            self.playing = c.playing;
            self.transition_start = Instant::now();
            self.transition_ms = 300;
            self.transition = Transition::Instant;
        }
    }
}

// ---- 子图层命中检测（egui `Place` / `proto_run::hotspots` / `pick` 的 GPUI 版）----
//
// 演示画面是一整张位图，画框内的按钮不是独立 DOM 节点，点击只能靠坐标反算命中。
// 位图由 GPUI flex 居中，图片在窗口里的 origin 依赖窗口尺寸；render 时拿不到，故把渲染
// 固有量（scale / 图片像素尺寸 / 图片 page 原点）存进 [`PLACE`]，点击时结合
// `window.viewport_size()` 现算居中偏移，再反算成 page 坐标，交给 engine 的 hotspots 命中。

/// 顶栏高度（px）。本项目 `pt(p)=px(p)`，命中换算与布局共用这一处，避免两处硬编码漂移。
const BAR_H: f64 = 40.0;

/// 演示画面的 page ↔ screen 映射固有量（对齐 egui `Place`，但 origin 在点击时按窗口现算）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    /// page 单位 → 图片像素。
    scale: f64,
    /// 位图像素尺寸（= 图片在窗口里的逻辑宽高，`.w(px(w)).h(px(h))`）。
    img_w: f64,
    img_h: f64,
    /// 图片左上角对应的 page 坐标（`world_render_bounds` 的 x0/y0）。
    page_x0: f64,
    page_y0: f64,
}

impl Place {
    /// 图片左上角在窗口的坐标。stage = 顶栏以下的整块区域，图片在 stage 中 flex 居中。
    /// `bar_h_px` = 顶栏的物理像素高（`pt(40)` × scale_factor）。
    fn origin(self, vp_w: f64, vp_h: f64, bar_h_px: f64) -> (f64, f64) {
        let stage_h = (vp_h - bar_h_px).max(0.0);
        ((vp_w - self.img_w) / 2.0, bar_h_px + (stage_h - self.img_h) / 2.0)
    }

    /// 窗口点击点 → 图片内的 page 坐标。`None` = 缩放非法或点在图片外。
    fn screen_to_page(self, click: (f64, f64), vp: (f64, f64), bar_h_px: f64) -> Option<(f64, f64)> {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return None;
        }
        let (ox, oy) = self.origin(vp.0, vp.1, bar_h_px);
        let (lx, ly) = (click.0 - ox, click.1 - oy);
        if lx < 0.0 || ly < 0.0 || lx > self.img_w || ly > self.img_h {
            return None;
        }
        Some((self.page_x0 + lx / self.scale, self.page_y0 + ly / self.scale))
    }
}

/// 命中：`spots`（engine hotspots，page 单位，bottom-most first）里 top-most 且含 page 点、
/// 带指定 trigger 的层，返回 `(节点, 该层匹配 trigger 的交互下标 k)`。
/// 对齐 egui `input.rs` 的 `hot.rev().filter(contains)` + `pick(find_trigger)`：
/// 嵌套取最上层、有交互但无该 trigger 的层让点击穿透、无交互层/隐藏层已被 hotspots 过滤。
fn pick_hotspot(doc: &Document, spots: &[(NodeId, figcraft_geom::Rect)], at: (f64, f64), trigger: Trigger) -> Option<(NodeId, usize)> {
    let p = figcraft_geom::Point::new(at.0, at.1);
    spots.iter().rev().find_map(|(id, r)| {
        if !r.contains(p) {
            return None;
        }
        doc.node(*id)?.interactions.iter().position(|it| it.trigger == trigger).map(|k| (*id, k))
    })
}

/// 一次点击的完整命中链：窗口点 → page 点 → evaluate（与渲染同一份 doc，动画位移后位置也对）
/// → engine hotspots → top-most 命中。纯读，可在单测里复现；点击是低频事件，evaluate 重算一次足够。
fn hit_test(
    doc: &Document,
    scope: NodeId,
    root: NodeId,
    place: Place,
    click: (f64, f64),
    vp: (f64, f64),
    bar_h_px: f64,
    time_ms: f64,
) -> Option<(NodeId, usize)> {
    let page_pt = place.screen_to_page(click, vp, bar_h_px)?;
    let evaluated = figcraft_doc::anim::motion::evaluate(doc, scope, time_ms);
    let spots = figcraft_engine::proto_run::hotspots(&evaluated, root);
    pick_hotspot(&evaluated, &spots, page_pt, Trigger::OnClick)
}

/// 渲染一个 frame 在当前播放头时刻的位图。
///
/// M1（用户 2026-11-12）：先按 per-frame 作用域把关键帧 / clip 动画求值进一个 scratch doc，
/// 再交给渲染层——否则 `render_node_at` 的 `time` 只喂给 animated GIF 选帧，关键帧预设在
/// 演示里根本不会动（文件头原先的承诺落空）。求值根 = `id` 所属的最外层画框（一个动画组），
/// 只烘焙该画框子树，画框内所有组件按同一个 `time_ms` 一起动 =「严格一起播」。
fn frame_image(doc: &Document, id: NodeId, scale: f64, time_ms: f64) -> Option<(gpui::AnyElement, Place)> {
    let scope = doc.top_level_frame(id).unwrap_or(id);
    let evaluated = figcraft_doc::anim::motion::evaluate(doc, scope, time_ms);
    let pix = figcraft_render::render_node_at(&evaluated, id, scale, time_ms)?;
    let (w, h) = (pix.width(), pix.height());
    // 位图左上角对应的 page 原点 = 渲染区域 bounds 的左上角（render_node_at 用同一 bounds）。
    let bounds = figcraft_render::export_bounds(&evaluated, id)?;
    let place = Place { scale, img_w: w as f64, img_h: h as f64, page_x0: bounds.x0, page_y0: bounds.y0 };
    let tex = screen_image(&pix, doc.color_profile);
    Some((img(ImageSource::Render(tex)).w(px(w as f32)).h(px(h as f32)).into_any_element(), place))
}

/// 覆盖率层（全屏）：顶栏 + 帧图 +（过渡时）上一帧。
pub fn render(app: &CanvasApp, state: &PresentState) -> Stateful<Div> {
    let t = tokens();
    let Ok(ds) = app.session.ds() else {
        return div().id("present-layer").size_full();
    };
    let doc = &ds.doc;
    let name = doc.node(state.current).map(|n| n.name.clone()).unwrap_or_default();
    // 缩到窗口里能放下（演示里帧通常比窗口大）。
    let scale = 1.0f64.min(900.0 / doc.node(state.current).map(|n| n.width.max(1.0)).unwrap_or(900.0));

    let mut stage = div().flex_1().min_h(px(0.0)).w_full().flex().items_center().justify_center().relative();
    if let Some((el, place)) = frame_image(doc, state.current, scale, state.time_ms) {
        // 把当前画面的映射固有量交给点击回调（render 借不到 &mut state，和 CLICK/ACTION 同套路）。
        PLACE.with(|p| p.set(Some(place)));
        stage = stage.child(el);
    } else {
        PLACE.with(|p| p.set(None));
        stage = stage.child(div().text_size(pt(13.0)).text_color(rgba(t.text_secondary)).child(tr("This frame cannot be rendered")));
    }
    // 过渡：用引擎 `poses()` 的 alpha/offset 摆上一帧（SmartAnimate 的淡入 / 位移）。
    if let Some(from) = state.from {
        let p = (state.transition_start.elapsed().as_secs_f64() * 1000.0 / state.transition_ms.max(1) as f64).clamp(0.0, 1.0);
        let size = figcraft_geom::Vec2::new(900.0, 600.0);
        let poses = figcraft_doc::anim::transitions::poses(&state.transition, p, size);
        if let Some((el, _)) = frame_image(doc, from, scale, 0.0) {
            let _ = poses; // offset 由 GPUI 的 absolute 定位叠加；这里先按 alpha 淡出
            stage = stage.child(div().absolute().left(px(0.0)).top(px(0.0)).opacity(1.0 - p as f32).child(el));
        }
    }

    let mut bar = div()
        .flex_none()
        .h(px(BAR_H as f32))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .px(pt(16.0))
        .bg(rgba(t.panel))
        .border_b_1()
        .border_color(rgba(t.border));
    let play = IconButton::new(if state.playing { Icon::Check } else { Icon::ChevronRight }).on_click(|_, _, _| ACTION.with(|a| a.set(1)));
    let back = IconButton::new(Icon::ChevronLeft).on_click(|_, _, _| ACTION.with(|a| a.set(2)));
    let exit = IconButton::new(Icon::Close).on_click(|_, _, _| ACTION.with(|a| a.set(3)));
    bar = bar
        .child(play)
        .child(back)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(pt(11.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.text))
                .child(SharedString::from(name)),
        )
        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(format!("{:.2}s", state.time_ms / 1000.0))))
        .child(exit);

    div().id("present-layer").absolute().top(px(0.0)).left(px(0.0)).size_full().flex().flex_col().bg(gpui::rgb(0x141414)).child(bar).child(
        stage.on_mouse_down(MouseButton::Left, |e, window, _cx| {
            // 记下点击的窗口点与视口尺寸；sync 帧据此反算 page 坐标并做子图层命中。
            // 顶栏 `h(pt(40))`：本项目的 `pt(p)=px(p)`，即 1pt=1px，故物理高就是 40px。
            let vp = window.viewport_size();
            let pos = (f64::from(e.position.x), f64::from(e.position.y));
            let vp = (f64::from(vp.width), f64::from(vp.height));
            let bar_h = BAR_H;
            CLICK_POS.with(|c| c.set(Some((pos, vp, bar_h))));
            CLICK.with(|c| c.set(true));
        }),
    )
}

thread_local! {
    /// 画面点击标记：render 里不能借 `&mut PresentState`，交给 sync 帧落地。
    pub static CLICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 点击的窗口点、视口尺寸、顶栏物理高（见 on_mouse_down）。
    pub static CLICK_POS: std::cell::Cell<Option<((f64, f64), (f64, f64), f64)>> = const { std::cell::Cell::new(None) };
    /// 当前画面的 page↔screen 映射固有量，由 render 每帧写好，点击回调读取。
    pub static PLACE: std::cell::Cell<Option<Place>> = const { std::cell::Cell::new(None) };
    /// 顶栏按钮：1 = 播放/暂停，2 = 返回，3 = 退出。
    pub static ACTION: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// 消费一次画面点击（由 lib.rs 的 sync 帧调用）：命中子图层就执行它的交互，没命中则回退到
/// 画框自身的第一条交互（保留旧行为）。返回本帧是否真的有一次点击要处理。
pub fn consume_click(doc: &Document, st: &mut PresentState) -> bool {
    let clicked = CLICK.with(|c| c.replace(false));
    if !clicked {
        CLICK_POS.with(|c| c.set(None));
        return false;
    }
    let pos = CLICK_POS.with(|c| c.replace(None));
    let place = PLACE.with(|p| p.get());
    match (pos, place) {
        // 过渡期画面叠了 from 帧（且 PLACE 只记 current），精细命中会对错位，回退到画框兜底交互。
        (Some(((cx, cy), (vw, vh), bar_h)), Some(place)) if st.from.is_none() => {
            let scope = doc.top_level_frame(st.current).unwrap_or(st.current);
            if let Some((node, k)) = hit_test(doc, scope, st.current, place, (cx, cy), (vw, vh), bar_h, st.time_ms) {
                st.fire(doc, node, k);
            } else {
                // 点在空白处 / 无命中：仍给画框自身挂的交互一次机会（egui 是闪 hotspot）。
                st.click(doc);
            }
        }
        _ => st.click(doc),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new("present-test")
    }

    #[test]
    fn playback_advances_and_wraps_at_the_page_duration() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        // D3：loop=false 到头停帧，回绕只发生在 looping=true。这里显式设一个循环页。
        let mut d = doc();
        let page = d.first_page().unwrap();
        let _ = d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 5.0, 5.0).unwrap();
        d.timelines.insert(page, TimelineSettings { duration_ms: 2000, fps: 30, looping: true });
        let mut p = PresentState::new(page, false);
        p.playing = true;
        p.last = Instant::now() - std::time::Duration::from_millis(120);
        assert!(p.advance(&d), "播放中要续帧");
        assert!(p.time_ms > 0.0);
        p.time_ms = 1999.0;
        p.last = Instant::now() - std::time::Duration::from_millis(10);
        p.advance(&d);
        assert!(p.time_ms < 2000.0, "循环页超过时长后回绕");
        p.playing = false;
        assert!(!p.advance(&d));
    }

    /// D3 Control：one-shot（loop=false）播到头停在末帧并退出播放态，不再续帧。
    #[test]
    fn one_shot_stops_at_the_end_and_stops_requesting_frames() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        let mut d = Document::new("present-one-shot");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 60, looping: false });
        let mut p = PresentState::new(frame, false);
        p.playing = true;
        // 第一步：小步进，离末帧还远 → 仍要续帧。
        p.time_ms = 100.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        assert!(p.advance(&d), "到达末帧前仍要续帧");
        assert!(p.time_ms > 100.0 && p.time_ms < 500.0, "播放头在末帧之前推进");
        // 第二步：跨过末帧 → 停在 500ms 并退出播放态。
        p.time_ms = 480.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        p.advance(&d);
        assert!((p.time_ms - 500.0).abs() < 1e-6, "停在画框末帧 500ms");
        assert!(!p.playing, "one-shot 播完退出播放态");
        assert!(!p.advance(&d), "停帧后不再请求动画帧");
    }

    /// M1（用户 2026-11-12）：演示播放按**当前画框自己的**时间轴时长回绕（画框没设才回退
    /// 页面），不再无条件用整页时长。这是 per-frame 动画组在演示侧的落点。
    #[test]
    fn playback_wraps_at_the_frame_duration_not_the_page() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::TimelineSettings;
        let mut d = Document::new("present-frame");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.timelines.insert(page, TimelineSettings { duration_ms: 2000, fps: 30, looping: false });
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 60, looping: true });
        let mut p = PresentState::new(frame, false);
        p.playing = true;
        p.time_ms = 480.0;
        p.last = Instant::now() - std::time::Duration::from_millis(40);
        p.advance(&d);
        assert!(p.time_ms < 500.0, "必须按画框的 500ms 回绕，而不是页面的 2000ms（否则会跑到 ~520）");
    }

    /// M1 地基：motion::evaluate 把画框子树的关键帧烘焙进 scratch doc，frame_image 依赖它让
    /// 演示里的组件真的随时间动（而不只是 GIF）。这里验证求值链对当前画框作用域成立。
    #[test]
    fn evaluate_bakes_the_frames_subtree_at_time() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::{AnimProperty, AnimValue, Ease};
        let mut d = Document::new("present-eval");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let dot = d.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        let n = d.node_mut(dot).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Opacity);
        tr.set_key(0, AnimValue::Scalar(1.0), Some(Ease::bezier(0.0, 0.0, 1.0, 1.0)));
        tr.set_key(500, AnimValue::Scalar(0.0), None);
        let at0 = figcraft_doc::anim::motion::evaluate(&d, frame, 0.0);
        let at250 = figcraft_doc::anim::motion::evaluate(&d, frame, 250.0);
        let o0 = at0.node(dot).unwrap().opacity as f64;
        let o250 = at250.node(dot).unwrap().opacity as f64;
        assert!((o0 - 1.0).abs() < 1e-3, "t=0 不透明度 1");
        assert!(o250 < 0.9 && o250 > 0.1, "t=250 关键帧被求值成中间值，演示里会动");
    }

    #[test]
    fn back_returns_to_the_previous_frame() {
        let mut p = PresentState::new(figcraft_doc::NodeId(1), false);
        p.history.push(figcraft_doc::NodeId(1));
        p.current = figcraft_doc::NodeId(2);
        p.back();
        assert_eq!(p.current, figcraft_doc::NodeId(1));
        assert_eq!(p.from, Some(figcraft_doc::NodeId(2)));
    }

    // ---- D3 Control：Action::Animation 作用于画框时钟 ----

    fn clock(playing: bool, base_ms: f64) -> MotionClock {
        MotionClock { playing, base_ms }
    }

    #[test]
    fn animation_op_resume_is_idempotent_while_playing() {
        // Play{from_start:false} 正在播且没到头 → 幂等不动（desigma 语义）。
        let cur = clock(true, 250.0);
        assert_eq!(apply_animation_op(AnimationOp::RESUME, cur, 250.0, 1000.0), cur);
    }

    #[test]
    fn animation_op_resume_restarts_a_finished_one_shot() {
        // 到头的一次性动画再 resume → 从头播。
        let cur = clock(false, 1000.0);
        let next = apply_animation_op(AnimationOp::RESUME, cur, 1000.0, 1000.0);
        assert!(next.playing && (next.base_ms - 0.0).abs() < 1e-6);
    }

    #[test]
    fn animation_op_from_start_and_restart_rewind() {
        let cur = clock(true, 700.0);
        for op in [AnimationOp::Play { from_start: true }, AnimationOp::Restart] {
            let next = apply_animation_op(op, cur, 700.0, 1000.0);
            assert!(next.playing, "{op:?} 应处于播放态");
            assert!((next.base_ms - 0.0).abs() < 1e-6, "{op:?} 应回到 0");
        }
    }

    #[test]
    fn animation_op_pause_keeps_the_playhead() {
        let cur = clock(true, 333.0);
        let next = apply_animation_op(AnimationOp::Pause, cur, 333.0, 1000.0);
        assert!(!next.playing && (next.base_ms - 333.0).abs() < 1e-6);
    }

    #[test]
    fn animation_op_toggle_play_flips_both_ways() {
        // 播着 → 暂停在播放头。
        let playing = clock(true, 400.0);
        let paused = apply_animation_op(AnimationOp::TogglePlay, playing, 400.0, 1000.0);
        assert!(!paused.playing && (paused.base_ms - 400.0).abs() < 1e-6);
        // 暂停 → 从播放头续播。
        let resumed = apply_animation_op(AnimationOp::TogglePlay, paused, 400.0, 1000.0);
        assert!(resumed.playing && (resumed.base_ms - 400.0).abs() < 1e-6);
    }

    /// 造一个带 `Action::Animation` 点击交互的画框；dest 落在画框内（验证按整框作用域算）。
    fn frame_with_animation_click(op: AnimationOp) -> (Document, figcraft_doc::NodeId) {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-control");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
        let btn = d.create(frame, NodeKind::Rectangle, "Btn", 0.0, 0.0, 10.0, 10.0).unwrap();
        // 既有 Present 架构：`click()` 读当前画框自身的 `interactions.first()`，故交互挂在画框上；
        // dest 指向子图层 btn，验证 control_animation 用 top_level_frame 把作用域解析回整框。
        d.get_mut(frame).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Animation { dest: btn, op }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        d.timelines.insert(frame, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        (d, frame)
    }

    #[test]
    fn click_dispatches_animation_op_to_the_current_frame() {
        let (d, frame) = frame_with_animation_click(AnimationOp::Play { from_start: true });
        let mut p = PresentState::new(frame, false);
        p.time_ms = 800.0; // 一个正在播、已过半的画框
        p.playing = true;
        p.click(&d);
        assert!(p.playing, "Play 后处于播放态");
        assert!((p.time_ms - 0.0).abs() < 1e-6, "from_start 把当前画框播放头拉回 0");
        assert_eq!(p.current, frame, "控制类动作不导航");
    }

    #[test]
    fn click_toggle_play_pauses_without_navigating() {
        let (d, frame) = frame_with_animation_click(AnimationOp::TogglePlay);
        let mut p = PresentState::new(frame, false);
        p.time_ms = 250.0;
        p.playing = true;
        p.click(&d);
        assert!(!p.playing, "TogglePlay 暂停正在播的画框");
        assert!((p.time_ms - 250.0).abs() < 1e-6, "暂停保留播放头");
    }

    /// 画框之间独立：dest 指向另一个画框时只更新它的挂起时钟，当前画框与播放态不受影响。
    #[test]
    fn animation_of_another_frame_is_kept_in_its_own_clock() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-two-frames");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 200.0, 0.0, 100.0, 100.0).unwrap();
        for f in [a, b] {
            d.timelines.insert(f, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        }
        d.get_mut(a).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Animation { dest: b, op: AnimationOp::Play { from_start: true } }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        let mut p = PresentState::new(a, false);
        p.time_ms = 120.0;
        p.playing = true;
        p.click(&d);
        // 当前画框 A 的时钟没被 B 的控制影响。
        assert!(p.playing && (p.time_ms - 120.0).abs() < 1e-6, "控制 B 不动 A 的播放头");
        assert_eq!(p.current, a, "不导航");
        // B 的挂起时钟被置成「从头播」。
        assert_eq!(p.clocks.get(&b), Some(&clock(true, 0.0)), "B 独立记住自己的时钟");
    }

    /// 导航到另一个画框时恢复它挂起的时钟（画框之间独立、来回不丢播放头）。
    #[test]
    fn navigation_restores_a_frames_suspended_clock() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("present-nav-clock");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 200.0, 0.0, 100.0, 100.0).unwrap();
        for f in [a, b] {
            d.timelines.insert(f, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: false });
        }
        // B 的动画被预先起播、此刻挂起在 600ms。
        let mut p = PresentState::new(a, false);
        p.clocks.insert(b, clock(true, 600.0));
        d.get_mut(a).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Navigate { dest: b, preserve_scroll: false }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        p.click(&d);
        assert_eq!(p.current, b, "导航到 B");
        assert!(p.playing && (p.time_ms - 600.0).abs() < 1e-6, "恢复 B 挂起的播放头与播放态");
    }

    // ---- 子图层命中检测（点按钮真正触发它自己的交互 / Action::Animation）----

    fn place(scale: f64, w: f64, h: f64, x0: f64, y0: f64) -> Place {
        Place { scale, img_w: w, img_h: h, page_x0: x0, page_y0: y0 }
    }

    #[test]
    fn place_screen_to_page_unprojects_the_centred_image() {
        // 窗口 1000×800，顶栏物理高 40，图片 400×200 居中于 stage(40..800)。
        let pl = place(2.0, 400.0, 200.0, 100.0, 50.0);
        let vp = (1000.0, 800.0);
        let bar = 40.0;
        // 图片左上角 = ((1000-400)/2, 40 + (760-200)/2) = (300, 320)。
        assert_eq!(pl.origin(vp.0, vp.1, bar), (300.0, 320.0));
        // 点图片左上角 → page (100, 50)。
        let tl = pl.screen_to_page((300.0, 320.0), vp, bar).unwrap();
        assert!((tl.0 - 100.0).abs() < 1e-9 && (tl.1 - 50.0).abs() < 1e-9, "图片左上角映射回 page 原点");
        // 点图片内 (100, 50) 像素 → page 位移 (50, 25)。
        let mid = pl.screen_to_page((400.0, 370.0), vp, bar).unwrap();
        assert!((mid.0 - 150.0).abs() < 1e-9 && (mid.1 - 75.0).abs() < 1e-9, "按 scale 反投影到 page");
        // 点在图片外 → None。
        assert!(pl.screen_to_page((100.0, 100.0), vp, bar).is_none(), "图片外的点击不算");
        assert!(pl.screen_to_page((700.1, 320.0), vp, bar).is_none(), "图片右边界外不算");
    }

    #[test]
    fn pick_hotspot_prefers_topmost_and_clicks_through_non_matching_layers() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("pick");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 200.0, 200.0).unwrap();
        let btn = d.create(frame, NodeKind::Rectangle, "Btn", 10.0, 10.0, 30.0, 30.0).unwrap();
        let cover = d.create(frame, NodeKind::Rectangle, "Cover", 10.0, 10.0, 30.0, 30.0).unwrap();
        let it = |actions: Vec<Action>| Interaction {
            trigger: Trigger::OnClick,
            actions,
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        };
        // 两个重叠层都是 hotspot，cover 后画（更上层）。
        d.get_mut(frame).unwrap().interactions.push(it(vec![Action::Back]));
        d.get_mut(btn).unwrap().interactions.push(it(vec![Action::Back]));
        d.get_mut(cover).unwrap().interactions.push(Interaction { trigger: Trigger::MouseDown, actions: vec![Action::Back], ..it(vec![]) });
        let spots = figcraft_engine::proto_run::hotspots(&d, frame);
        // cover 只有 MouseDown（无 OnClick）→ 点击穿透到更下面的 btn（最上层的 OnClick）。
        let hit = pick_hotspot(&d, &spots, (20.0, 20.0), Trigger::OnClick);
        assert_eq!(hit, Some((btn, 0)), "最上层带 OnClick 的按钮胜出，MouseDown-only 的盖层穿透");
    }

    /// 造一份演示：Frame(0,0,400x200) 内含 Btn(50,60,100x40)，Btn 挂一个 TogglePlay 的
    /// Action::Animation；画框时间轴 1000ms。返回 (doc, frame, btn, 该 frame 的 Place)。
    fn frame_with_animated_button() -> (Document, NodeId, NodeId, Place) {
        use figcraft_doc::NodeKind;
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let mut d = Document::new("hit-demo");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 400.0, 200.0).unwrap();
        let btn = d.create(frame, NodeKind::Rectangle, "PlayBtn", 50.0, 60.0, 100.0, 40.0).unwrap();
        d.get_mut(btn).unwrap().interactions.push(Interaction {
            trigger: Trigger::OnClick,
            actions: vec![Action::Animation { dest: frame, op: AnimationOp::TogglePlay }],
            transition: Transition::Instant,
            duration_ms: 300,
            easing: Easing::EaseOut,
            delay_ms: 0,
        });
        d.timelines.insert(frame, figcraft_doc::anim::motion::TimelineSettings { duration_ms: 1000, fps: 60, looping: true });
        // 1:1 缩放，图片 400×200，page 原点 (0,0)。窗口 1000×800、顶栏 40 → 图片 origin (300, 320)。
        (d, frame, btn, place(1.0, 400.0, 200.0, 0.0, 0.0))
    }

    #[test]
    fn clicking_a_child_button_toggles_the_frames_animation() {
        let (d, frame, btn, pl) = frame_with_animated_button();
        let mut p = PresentState::new(frame, false);
        p.playing = true; // 画框动画正在播
        // 按钮中心 page (100, 80) → 窗口 origin(300,320) + (100,80) = (400, 400)。
        let hit = hit_test(&d, frame, frame, pl, (400.0, 400.0), (1000.0, 800.0), 40.0, p.time_ms);
        assert_eq!(hit, Some((btn, 0)), "点在子按钮上必须命中它，而不是画框自身");
        p.fire(&d, btn, 0);
        assert!(!p.playing, "命中按钮后执行它的 Action::Animation(TogglePlay) → 画框动画暂停");
    }

    #[test]
    fn clicking_outside_the_button_does_not_hit_it() {
        let (d, frame, _btn, pl) = frame_with_animated_button();
        // 点画框右上角空白（page 350,20 → 窗口 650,340），不在按钮 range 内。
        let hit = hit_test(&d, frame, frame, pl, (650.0, 340.0), (1000.0, 800.0), 40.0, 0.0);
        assert!(hit.is_none(), "按钮外的空白不应命中该按钮");
    }

    /// 关键：命中用**与渲染同一份 evaluated doc**，所以按钮被关键帧位移后，新位置能命中、旧位置不能。
    #[test]
    fn hit_test_follows_the_buttons_animated_displacement() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::anim::motion::{AnimProperty, AnimValue, Ease};
        use figcraft_doc::proto::{Easing, Interaction, Trigger};
        let (mut d, frame, btn, pl) = frame_with_animated_button();
        // 给按钮加位移关键帧：t=0 在 (50,60)，t=500 平移到 (250,60)（右移 200）。
        let tr = d.node_mut(btn).unwrap().motion.ensure_track(AnimProperty::Position);
        tr.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(50.0, 60.0)), Some(Ease::Linear));
        tr.set_key(500, AnimValue::Vec2(figcraft_geom::Vec2::new(250.0, 60.0)), None);
        let at_end = |t, pt| hit_test(&d, frame, frame, pl, pt, (1000.0, 800.0), 40.0, t);
        // t=500：按钮在 x∈[250,350]，page 中心 (300,80) → 窗口 (600,400) 命中；旧中心 (400,400) 落空。
        assert_eq!(at_end(500.0, (600.0, 400.0)), Some((btn, 0)), "位移后的新位置能命中");
        assert!(at_end(500.0, (400.0, 400.0)).is_none(), "位移前的旧位置不再命中");
        // t=0：按钮还在原位，旧中心能命中，新位置落空。
        assert_eq!(at_end(0.0, (400.0, 400.0)), Some((btn, 0)), "t=0 原位仍能命中");
    }
}
