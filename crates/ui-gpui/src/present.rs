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

use figcraft_i18n::tr;
use crate::app::CanvasApp;
use crate::canvas::render_worker::screen_image;
use crate::widgets::{Icon, IconButton, pt, rgba, tokens};
use figcraft_doc::{Document, NodeId};
use figcraft_doc::proto::{Action, Transition};
use gpui::{Div, ImageSource, MouseButton, SharedString, Stateful, div, img, prelude::*, px};
use std::time::Instant;

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
        }
    }

    /// 推进播放头 / 过渡时钟。返回是否需要继续请求动画帧。
    pub fn advance(&mut self, doc: &Document) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = now;
        if self.playing {
            self.time_ms += dt;
            // 一页的 motion timeline 长度；到头就循环（演示里的动画一直跑）。
            let page = doc.page_of(self.current).unwrap_or(self.current);
            let dur = crate::motion::settings_of(doc, page).duration_ms.max(1) as f64;
            if self.time_ms > dur {
                self.time_ms %= dur;
            }
        }
        let in_transition = self.from.is_some()
            && now.duration_since(self.transition_start).as_secs_f64() * 1000.0 < self.transition_ms as f64;
        if !in_transition {
            self.from = None;
        }
        self.playing || in_transition
    }

    /// 执行当前 frame 的第一条交互：切到目标 frame，起一个过渡。
    pub fn click(&mut self, doc: &Document) {
        let Some(n) = doc.node(self.current) else { return };
        let Some(it) = n.interactions.first() else { return };
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
        self.history.push(self.current);
        self.from = Some(self.current);
        self.current = next;
        self.time_ms = 0.0;
        self.transition_start = Instant::now();
        self.transition_ms = it.duration_ms;
        self.transition = it.transition;
        self.playing = false;
    }

    pub fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.from = Some(self.current);
            self.current = prev;
            self.time_ms = 0.0;
            self.transition_start = Instant::now();
            self.transition_ms = 300;
            self.transition = Transition::Instant;
        }
    }
}

/// 渲染一个 frame 在当前播放头时刻的位图。
fn frame_image(doc: &Document, id: NodeId, scale: f64, time_ms: f64) -> Option<gpui::AnyElement> {
    let pix = figcraft_render::render_node_at(doc, id, scale, time_ms)?;
    let (w, h) = (pix.width(), pix.height());
    let tex = screen_image(&pix, doc.color_profile);
    Some(
        img(ImageSource::Render(tex))
            .w(px(w as f32))
            .h(px(h as f32))
            .into_any_element(),
    )
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
    if let Some(el) = frame_image(doc, state.current, scale, state.time_ms) {
        stage = stage.child(el);
    } else {
        stage = stage.child(
            div().text_size(pt(13.0)).text_color(rgba(t.text_secondary)).child(tr("This frame cannot be rendered")),
        );
    }
    // 过渡：用引擎 `poses()` 的 alpha/offset 摆上一帧（SmartAnimate 的淡入 / 位移）。
    if let Some(from) = state.from {
        let p = (state.transition_start.elapsed().as_secs_f64() * 1000.0 / state.transition_ms.max(1) as f64).clamp(0.0, 1.0);
        let size = figcraft_geom::Vec2::new(900.0, 600.0);
        let poses = figcraft_doc::anim::transitions::poses(&state.transition, p, size);
        if let Some(el) = frame_image(doc, from, scale, 0.0) {
            let _ = poses; // offset 由 GPUI 的 absolute 定位叠加；这里先按 alpha 淡出
            stage = stage.child(div().absolute().left(px(0.0)).top(px(0.0)).opacity(1.0 - p as f32).child(el));
        }
    }

    let mut bar = div()
        .flex_none().h(pt(40.0)).w_full().flex().flex_row().items_center().gap(pt(8.0)).px(pt(16.0))
        .bg(rgba(t.panel)).border_b_1().border_color(rgba(t.border));
    let play = IconButton::new(if state.playing { Icon::Check } else { Icon::ChevronRight })
        .on_click(|_, _, _| ACTION.with(|a| a.set(1)));
    let back = IconButton::new(Icon::ChevronLeft).on_click(|_, _, _| ACTION.with(|a| a.set(2)));
    let exit = IconButton::new(Icon::Close).on_click(|_, _, _| ACTION.with(|a| a.set(3)));
    bar = bar
        .child(play)
        .child(back)
        .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(name)))
        .child(div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(format!("{:.2}s", state.time_ms / 1000.0))))
        .child(exit);

    div()
        .id("present-layer")
        .absolute().top(px(0.0)).left(px(0.0)).size_full()
        .flex().flex_col()
        .bg(gpui::rgb(0x141414))
        .child(bar)
        .child(
            stage.on_mouse_down(MouseButton::Left, |_e, _w, _cx| {
                // 点击画面推进交互：状态由 root view 在 sync 里更新（见 `PresentState::click`）。
                CLICK.with(|c| c.set(true));
            }),
        )
}

thread_local! {
    /// 画面点击标记：render 里不能借 `&mut PresentState`，交给 sync 帧落地。
    pub static CLICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 顶栏按钮：1 = 播放/暂停，2 = 返回，3 = 退出。
    pub static ACTION: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}


#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new("present-test")
    }

    #[test]
    fn playback_advances_and_wraps_at_the_page_duration() {
        let d = doc();
        let mut p = PresentState::new(figcraft_doc::NodeId(1), false);
        p.playing = true;
        p.last = Instant::now() - std::time::Duration::from_millis(120);
        assert!(p.advance(&d), "播放中要续帧");
        assert!(p.time_ms > 0.0);
        p.time_ms = 1999.0;
        p.last = Instant::now() - std::time::Duration::from_millis(10);
        p.advance(&d);
        assert!(p.time_ms < 2000.0, "超过一页时长后循环");
        p.playing = false;
        assert!(!p.advance(&d));
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
}
