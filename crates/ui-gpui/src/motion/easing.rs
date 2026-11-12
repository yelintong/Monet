//! Easing 编辑器 —— `ui-egui/src/motion/easing.rs` 的 GPUI 移植。
//!
//! 同一个 `editor()` 在两处复用：右栏 Inspector（选中关键帧段时）和 clip 设置浮窗。
//! 功能与 egui 对齐：Curve | Spring 双 tab、贝塞尔两个可拖控制点、弹簧峰值手柄、
//! 数值输入 / Reset / Exact envelope / Mass、物理读数、预设列表，以及「整段拖拽一个 undo step」。
//!
//! 拖拽用**相对位移**（按下点 → 全局 move）而不是元素局部坐标：GPUI 的 per-element
//! `on_mouse_move` 只在悬停时触发，所以 move/up 由根视图的窗口级事件路由进来，
//! 与 `timeline.rs` 的拖拽同一套做法。控件全部复用冻结的 `widgets/`。

use figcraft_i18n::tr;
use crate::canvas::geom::{Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use crate::canvas::painter::Painter;
use crate::motion::{Act, EaseTarget, MotionUi, Queue};
use crate::theme::stroke;
use crate::widgets::panel_section::{field_row, field_row_full, panel_section, sublabel_row2};
use crate::widgets::{Icon, IconButton, Slot, TextField, pt, rgba, text_field, tokens};
use figcraft_doc::Document;
use figcraft_doc::anim::motion::{
    DEFAULT_ELASTICITY, ELASTICITY_RANGE, Ease, SpringAlgorithm, spring_angular_frequency, spring_damping_ratio,
    spring_stiffness_damping, unit_mass,
};
use gpui::{App, Bounds, ContentMask, Div, Entity, MouseButton, Pixels, Point, SharedString, Window, canvas, div, prelude::*, px};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const GRAPH_H: f32 = 144.0;
/// 图形里的方形绘图边长（相对位移换算控制点用）。
const PLOT: f32 = 120.0;
const CONTROL_W: f32 = 124.0;

/// 一个进行中的手柄拖拽。`ease` 是按下瞬间的原值，每次 move 都从它重算，绝不累积漂移。
struct Drag {
    handle: u8, // 0/1 = 贝塞尔两个控制点，2 = 弹簧峰值
    x0: f32,
    y0: f32,
    ease: Ease,
    target: EaseTarget,
    queue: Queue,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// 数值输入框跨帧复用（panel / clip 各一套），否则每帧新建会丢焦点。
    static FIELDS: RefCell<HashMap<&'static str, Entity<TextField>>> = RefCell::new(HashMap::new());
}

pub fn drag_active() -> bool {
    DRAG.with(|d| d.borrow().is_some())
}

/// 根视图的窗口级 move 路由：拖拽离开图形区域后仍然继续（egui `Response::dragged()`）。
pub fn route_move(pos: Point<Pixels>) {
    DRAG.with(|d| {
        let mut slot = d.borrow_mut();
        let Some(g) = slot.as_mut() else { return };
        let (dx, dy) = (f32::from(pos.x) - g.x0, f32::from(pos.y) - g.y0);
        let next = match g.handle {
            0 | 1 => {
                let Some(mut c) = g.ease.control_points() else { return };
                let i = (g.handle as usize) * 2;
                // x 不能让时间倒流；y 允许过冲（CSS 语义）。
                c[i] = (c[i] + dx / PLOT).clamp(0.0, 1.0);
                c[i + 1] = (c[i + 1] - dy / PLOT).clamp(-0.5, 1.5);
                Ease::bezier(c[0], c[1], c[2], c[3])
            }
            _ => {
                let e = g.ease.elasticity().unwrap_or(DEFAULT_ELASTICITY);
                let algo = g.ease.spring_algorithm().unwrap_or_default();
                let mass = g.ease.mass().unwrap_or_else(unit_mass);
                let gain = ELASTICITY_RANGE.1 / PLOT;
                Ease::spring_with(e - dy * gain, algo, mass)
            }
        };
        g.queue.push(Act::EaseLive(g.target, next));
    });
}

/// 根视图的窗口级 up：一次拖拽收成一个 undo step。
pub fn end_drag() {
    DRAG.with(|d| {
        if let Some(g) = d.borrow_mut().take() {
            g.queue.push(Act::EaseEnd);
        }
    });
}

fn default_ease(spring: bool) -> Ease {
    if spring { Ease::SpringGentle } else { Ease::Linear }
}

fn fmt_num(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// `"0.1, 0.2, 0.9, 1"` → CustomBezier（egui `parse_values`）。
fn parse_values(s: &str) -> Option<Ease> {
    let nums: Vec<f32> = s.split([',', ' ', ';'])
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().parse::<f32>())
        .collect::<Result<_, _>>().ok()?;
    if nums.len() != 4 || nums.iter().any(|v| !v.is_finite()) { return None; }
    Some(Ease::bezier(nums[0], nums[1], nums[2], nums[3]))
}

/// 曲线图（Painter 记录形状，交给 `gpui::canvas` 重放）。
fn graph(painter: &mut Painter, rect: Rect, ease: Ease, t: crate::theme::Tokens) {
    painter.rect_stroke(rect, CornerRadius::ZERO, stroke(t.border, 1.0), StrokeKind::Inside);
    let plot = Rect::from_center_size(rect.center(), vec2(PLOT, PLOT));
    let to_s = |x: f32, y: f32| pos2(plot.min.x + x * plot.width(), plot.max.y - y * plot.height());
    for v in [0.0, 1.0] {
        painter.line_segment([to_s(0.0, v), to_s(1.0, v)], stroke(t.border, 1.0));
    }
    if let Some(c) = ease.control_points().filter(|_| !matches!(ease, Ease::Hold)) {
        painter.line_segment([to_s(0.0, 0.0), to_s(c[0], c[1])], stroke(t.accent, 1.0));
        painter.line_segment([to_s(1.0, 1.0), to_s(c[2], c[3])], stroke(t.accent, 1.0));
    }
    let (lo, hi) = ease.value_range();
    let span = (hi - lo).abs().max(1e-6);
    let samples = ease.sample(64);
    let pts: Vec<Pos2> = if ease.is_spring() {
        let (a, b) = (lo.min(0.0), hi.max(1.0));
        samples.iter().map(|q| pos2(
            rect.min.x + 12.0 + q[0].clamp(0.0, 1.0) * (rect.width() - 24.0),
            rect.max.y - 12.0 - ((q[1] - a) / (b - a).max(1e-3)).clamp(0.0, 1.0) * (rect.height() - 24.0),
        )).collect()
    } else {
        samples.iter().map(|q| pos2(
            rect.min.x + q[0].clamp(0.0, 1.0) * rect.width(),
            rect.max.y - ((q[1] - lo) / span).clamp(0.0, 1.0) * rect.height(),
        )).collect()
    };
    painter.add_shape(pts, false, Color32::TRANSPARENT, Stroke::new(1.5, t.text));
}

/// 某个手柄在图形里的位置（用于放绝对定位的命中格）。
fn handle_pos(rect: Rect, ease: Ease, handle: u8) -> Option<Pos2> {
    let plot = Rect::from_center_size(rect.center(), vec2(PLOT, PLOT));
    if handle < 2 {
        let c = ease.control_points()?;
        let i = (handle as usize) * 2;
        return Some(pos2(plot.min.x + c[i] * plot.width(), plot.max.y - c[i + 1] * plot.height()));
    }
    let (lo, hi) = ease.value_range();
    let (a, b) = (lo.min(0.0), hi.max(1.0));
    let mut best = (1.0, 1.0);
    for q in ease.sample(96) {
        if q[1] > best.1 + 1e-4 { best = (q[0], q[1]); }
    }
    Some(pos2(
        rect.min.x + 12.0 + best.0 * (rect.width() - 24.0),
        rect.max.y - 12.0 - ((best.1 - a) / (b - a).max(1e-3)).clamp(0.0, 1.0) * (rect.height() - 24.0),
    ))
}

/// 复用的数值框（跨帧保留焦点）。
fn numeric(cx: &mut App, key: &'static str, value: &str, on_commit: Rc<dyn Fn(String)>) -> Entity<TextField> {
    FIELDS.with(|m| {
        let mut map = m.borrow_mut();
        let field = map.entry(key)
            .or_insert_with(|| text_field(cx, CONTROL_W, value, "", Slot::None, Slot::None))
            .clone();
        field.update(cx, |tf, _| {
            tf.set_value(value.to_string());
            let f = on_commit.clone();
            tf.set_on_commit(move |s, _, _| f(s.to_string()));
        });
        field
    })
}

/// 缓动编辑器（egui `easing::editor`）：tab + 可拖手柄 + 数值/弹簧参数 + 预设。
/// `salt` 区分 inspector("panel") 与 clip("clip") 的数值框缓存；`show_done` 只给 inspector。
pub fn editor(cx: &mut App, ease: Ease, target: EaseTarget, queue: &Queue, salt: &'static str, show_done: bool, width: f32) -> Div {
    let t = tokens();
    let spring = ease.is_spring();
    let mut body = div().flex().flex_col().w_full().gap(pt(8.0));
    let mut grid = panel_section(tr("Easing"));

    // ── Curve | Spring tabs ──
    let mut tabs = div().flex().flex_row().w_full().h(pt(24.0)).rounded(pt(6.0)).bg(rgba(t.field));
    for (i, label) in ["Curve", "Spring"].iter().enumerate() {
        let on = (i == 1) == spring;
        let q = queue.clone();
        let tab = div()
            .flex_1().h_full().flex().items_center().justify_center().rounded(pt(5.0)).cursor_pointer()
            .when(on, |d| d.bg(rgba(t.panel)))
            .when(!on, |d| d.hover(|s| s.bg(rgba(t.hover))))
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                if !on { q.push(Act::EaseSet(target, default_ease(i == 1))); }
            })
            .text_size(pt(11.0))
            .text_color(rgba(if on { t.text } else { t.text_tertiary }))
            .child(SharedString::from(*label));
        tabs = tabs.child(tab);
    }
    body = body.child(tabs);

    // ── 图形 + 手柄 ──
    // The curve's coordinates are baked from this rect, so the canvas has to be exactly this
    // wide — and the width depends on where the editor sits (card body vs the inspector panel,
    // which pads twice). The caller passes it.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, GRAPH_H));
    let mut painter = Painter::new(rect);
    graph(&mut painter, rect, ease, t);
    let ops = painter.ops.clone();
    let mut graph_box = div()
        .id(SharedString::from(format!("ease-graph-{salt}")))
        .relative().w(pt(width)).h(pt(GRAPH_H))
        .child(
            canvas(
                |_, _, _| (),
                move |b, _, window, _cx| {
                    let p = Painter { clip: Rect::NOTHING, ops: ops.clone(), labels: Vec::new() };
                    p.paint(window, pos2(f32::from(b.origin.x), f32::from(b.origin.y)));
                },
            ).w(pt(width)).h(pt(GRAPH_H)),
        );
    let handles: Vec<u8> = if spring { vec![2] } else if ease.control_points().is_some() { vec![0, 1] } else { vec![] };
    for h in handles {
        if let Some(pos) = handle_pos(rect, ease, h) {
            let q = queue.clone();
            graph_box = graph_box.child(
                div()
                    .absolute().left(px(pos.x - 9.0)).top(px(pos.y - 9.0)).w(px(18.0)).h(px(18.0)).cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |e, _, _| {
                        q.push(Act::EaseBegin);
                        DRAG.with(|d| *d.borrow_mut() = Some(Drag {
                            handle: h,
                            x0: f32::from(e.position.x),
                            y0: f32::from(e.position.y),
                            ease,
                            target,
                            queue: q.clone(),
                        }));
                    }),
            );
        }
    }
    grid = grid.child(field_row_full(graph_box));

    // ── 数值 / 弹簧参数 ──
    if spring {
        let el = ease.elasticity().unwrap_or(DEFAULT_ELASTICITY);
        let algo = ease.spring_algorithm().unwrap_or_default();
        let mass = ease.mass().unwrap_or_else(unit_mass);
        let q_vals = queue.clone();
        let f = numeric(cx, if salt == "clip" { "clip-elasticity" } else { "panel-elasticity" }, &fmt_num(el), Rc::new(move |s| {
            if let Ok(v) = s.trim().parse::<f32>() { q_vals.push(Act::EaseSet(target, Ease::spring_with(v, algo, mass))); }
        }));
        grid = grid.child(field_row(vec![
            div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Elasticity")).into_any_element(),
            f.into_any_element(),
        ], Vec::<IconButton>::new()));
        let (z, w0) = (spring_damping_ratio(el), spring_angular_frequency(el, algo));
        let (k, c) = spring_stiffness_damping(el, mass, algo);
        grid = grid.child(field_row_full(
            // 与弹窗一致：长串物理读数单行裁切，不撑破右栏网格。
            div().w_full().overflow_hidden().whitespace_nowrap().truncate()
                .text_size(pt(9.5)).text_color(rgba(t.text_tertiary))
                .child(SharedString::from(format!("ζ {z:.3} · ω0 {w0:.1} · k {k:.0} · c {c:.1}"))),
        ));
        let q_env = queue.clone();
        let exact = algo == SpringAlgorithm::Tight;
        grid = grid.child(field_row(vec![
            div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Exact envelope")).into_any_element(),
            IconButton::new(Icon::Check).active(exact).on_click(move |_, _, _| {
                let next = if exact { SpringAlgorithm::Envelope } else { SpringAlgorithm::Tight };
                q_env.push(Act::EaseSet(target, Ease::spring_with(el, next, mass)));
            }).into_any_element(),
        ], Vec::<IconButton>::new()));
        let q_mass = queue.clone();
        let mf = numeric(cx, if salt == "clip" { "clip-mass" } else { "panel-mass" }, &fmt_num(mass), Rc::new(move |s| {
            if let Ok(v) = s.trim().parse::<f32>() { q_mass.push(Act::EaseSet(target, Ease::spring_with(el, algo, v.clamp(0.01, 100.0)))); }
        }));
        grid = grid.child(field_row(vec![
            div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Mass")).into_any_element(),
            mf.into_any_element(),
        ], Vec::<IconButton>::new()));
    } else if let Some(c) = ease.control_points() {
        let txt = c.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(", ");
        let q = queue.clone();
        let f = numeric(cx, if salt == "clip" { "clip-values" } else { "panel-values" }, &txt, Rc::new(move |s| {
            if let Some(e) = parse_values(&s) { q.push(Act::EaseSet(target, e)); }
        }));
        let q_reset = queue.clone();
        grid = grid.child(field_row(vec![f.into_any_element()], vec![
            IconButton::new(Icon::Rotate).on_click(move |_, _, _| q_reset.push(Act::EaseSet(target, Ease::Linear))),
        ]));
    }

    body = body.child(grid);

    // The presets live in the card's Easing dropdown (`ease_row` builds its menu from the same
    // `Ease::PRESETS`). Laying all twelve out again as rows here is what pushed the card past the
    // window; only the "Done" row of the old section is still needed.
    if show_done {
        let mut sec = panel_section(tr("Easing"));
        let q = queue.clone();
        sec = sec.child(field_row_full(
            div()
                .h(pt(24.0)).w_full().flex().items_center().justify_center().rounded(pt(5.0))
                .bg(rgba(t.field)).cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _, _| q.push(Act::SetEasingTarget(false)))
                .text_size(pt(11.0)).text_color(rgba(t.text)).child(tr("Done")),
        ));
        body = body.child(sec);
    }
    body
}

/// 右栏 Inspector 的 Easing 页（egui `easing::panel`）：Back + 标题 + 当前名 + editor。
pub fn panel(cx: &mut App, doc: &Document, ui: &MotionUi, queue: &Queue, _window: &mut Window) -> Div {
    let t = tokens();
    let Some(k) = ui.selected else {
        return div().w_full().p(pt(16.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Select a segment"));
    };
    let current = doc.node(k.node)
        .and_then(|n| n.motion.track(k.property))
        .and_then(|tr| tr.key_at(k.time_ms))
        .map(|key| key.easing)
        .unwrap_or(Ease::Linear);
    let back = {
        let q = queue.clone();
        IconButton::new(Icon::ChevronLeft).on_click(move |_, _, _| q.push(Act::SetEasingTarget(false)))
    };
    let mut grid = panel_section(tr("Easing"));
    let mut body = div().flex().flex_col().w_full();
    grid = grid.child(field_row(vec![
        back.into_any_element(),
        div().flex_1().text_size(pt(11.5)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text))
            .child(tr("Easing")).into_any_element(),
    ], Vec::<IconButton>::new()));
    grid = grid.child(sublabel_row2("Curve", current.name().to_string()));
    body = body.child(grid);
    body.child(div().w_full().child(editor(cx, current, EaseTarget::Key(k), queue, "panel", true, 176.0)))
}


// ─────────────────────────────────────────────────────────────────────────────
// Clip 设置浮窗专用的缓动编辑器。
//
// 布局严格照 egui PR#1624 `motion/easing.rs` 的 `editor()` / `editor_height()` /
// `curve_graph()` / `spring_graph()`：弹窗内容宽 240pt（264 卡片 − 12×2 pad），
// editor 直接占满这个宽度，tabs 24 → 8 → graph 144 → 8 → values 24/112/0。
//
// 之前的写法把 144pt 的图表塞进 `field_row_full` 的 32pt 网格行（上下各溢出 56pt，
// 压住 "Easing" 标题和数值框），还把 208pt 宽的画布塞进带两层 padding 的 cell
// （实际只有 128pt，横向画出卡片）。这里不用网格行，图表自己占满内容宽并裁剪。
// ─────────────────────────────────────────────────────────────────────────────

/// `easing::editor_height`: 24 tabs + 8 + 144 graph + 8 + values。
pub fn clip_editor_height(ease: Ease) -> f32 {
    let values = if ease.is_spring() {
        112.0
    } else if ease.control_points().is_some() {
        24.0
    } else {
        0.0
    };
    24.0 + 8.0 + GRAPH_H + 8.0 + values
}

/// 图表：`graph()` 填 6pt 圆角的 field 底；曲线 / 控制点连线都在 rect 内裁剪
/// （egui 用 `painter.with_clip_rect(rect)`，GPUI 在 canvas paint 里用 content mask）。
fn clip_graph(painter: &mut Painter, rect: Rect, ease: Ease, t: crate::theme::Tokens) {
    painter.rect_filled(rect, CornerRadius::same(6), t.field);
    if ease.is_spring() {
        // `spring_graph`: plot = rect 内缩左右 20 / 上下 28。
        let (lo, hi) = ease.value_range();
        let (a, b) = (lo.min(0.0), hi.max(1.0));
        let plot = Rect::from_min_max(
            pos2(rect.min.x + 20.0, rect.min.y + 28.0),
            pos2(rect.max.x - 20.0, rect.max.y - 28.0),
        );
        let to_s = |tm: f32, v: f32| {
            pos2(plot.min.x + tm * plot.width(), plot.max.y - (v - a) / (b - a).max(1e-3) * plot.height())
        };
        for v in [0.0, 1.0] {
            painter.line_segment([to_s(0.0, v), to_s(1.0, v)], stroke(t.border, 1.0));
        }
        let pts: Vec<Pos2> = ease.sample(96).iter().map(|q| to_s(q[0], q[1])).collect();
        painter.add_shape(pts, false, Color32::TRANSPARENT, Stroke::new(1.5, t.text));
    } else {
        // `curve_graph`: 正方形 plot 居中，边长 min(高−24, 宽−24)，给过冲的手柄留白。
        let side = (rect.height() - 24.0).min(rect.width() - 24.0).max(24.0);
        let plot = Rect::from_center_size(rect.center(), vec2(side, side));
        let to_s = |x: f32, y: f32| pos2(plot.min.x + x * plot.width(), plot.max.y - y * plot.height());
        for v in [0.0, 1.0] {
            painter.line_segment([to_s(0.0, v), to_s(1.0, v)], stroke(t.border, 1.0));
        }
        if let Some(c) = ease.control_points().filter(|_| !matches!(ease, Ease::Hold)) {
            painter.line_segment([to_s(0.0, 0.0), to_s(c[0], c[1])], stroke(t.accent, 1.0));
            painter.line_segment([to_s(1.0, 1.0), to_s(c[2], c[3])], stroke(t.accent, 1.0));
        }
        let pts: Vec<Pos2> = ease.sample(64).iter().map(|q| to_s(q[0], q[1])).collect();
        painter.add_shape(pts, false, Color32::TRANSPARENT, Stroke::new(1.5, t.text));
    }
}

/// 手柄在图表里的位置，坐标系与 `clip_graph` 完全一致。
fn clip_handle_pos(rect: Rect, ease: Ease, handle: u8) -> Option<Pos2> {
    if handle < 2 {
        let c = ease.control_points()?;
        let side = (rect.height() - 24.0).min(rect.width() - 24.0).max(24.0);
        let plot = Rect::from_center_size(rect.center(), vec2(side, side));
        let i = (handle as usize) * 2;
        return Some(pos2(plot.min.x + c[i] * plot.width(), plot.max.y - c[i + 1] * plot.height()));
    }
    let (lo, hi) = ease.value_range();
    let (a, b) = (lo.min(0.0), hi.max(1.0));
    let plot = Rect::from_min_max(
        pos2(rect.min.x + 20.0, rect.min.y + 28.0),
        pos2(rect.max.x - 20.0, rect.max.y - 28.0),
    );
    let mut best = (1.0, 1.0);
    for q in ease.sample(96) {
        if q[1] > best.1 + 1e-4 {
            best = (q[0], q[1]);
        }
    }
    Some(pos2(
        plot.min.x + best.0 * plot.width(),
        plot.max.y - (best.1 - a) / (b - a).max(1e-3) * plot.height(),
    ))
}

/// 可指定宽度的数值框（PR 里 clip 的 values 行是 `w − 28`，不是右栏的 124）。
/// 与 `numeric` 共用同一个跨帧缓存，用不同 key 避免和 inspector 抢焦点。
fn numeric_w(cx: &mut App, key: &'static str, value: &str, width: f32, on_commit: Rc<dyn Fn(String)>) -> Entity<TextField> {
    FIELDS.with(|m| {
        let mut map = m.borrow_mut();
        let field = map.entry(key)
            .or_insert_with(|| text_field(cx, width, value, "", Slot::None, Slot::None))
            .clone();
        field.update(cx, |tf, _| {
            tf.set_value(value.to_string());
            let f = on_commit.clone();
            tf.set_on_commit(move |s, _, _| f(s.to_string()));
        });
        field
    })
}

/// 弹窗里的完整 easing 编辑器（不含 Easing 下拉行，那一行由 `clip_popover` 画）。
pub fn clip_editor(cx: &mut App, ease: Ease, target: EaseTarget, queue: &Queue, width: f32) -> Div {
    let t = tokens();
    let spring = ease.is_spring();
    let mut body = div().flex().flex_col().w(pt(width)).gap(pt(8.0));

    // 1) Curve | Spring：用通用 `Segmented`（24pt 等宽双格），不再手写一套 tabs。
    let q_tabs = queue.clone();
    let tabs = crate::widgets::segmented::Segmented::new(vec![
        crate::widgets::segmented::SegItem::text(tr("Curve")),
        crate::widgets::segmented::SegItem::text(tr("Spring")),
    ])
    .active(Some(if spring { 1 } else { 0 }))
    .on_pick(move |i, _, _| {
        if (i == 1) != spring { q_tabs.push(Act::EaseSet(target, default_ease(i == 1))); }
    })
    .build();
    body = body.child(tabs);

    // 2) graph：宽 = 弹窗内容宽，高 144；绘制用 content mask 裁在图表框内，
    //    y 过冲的控制点线 / 弹簧响应都不会画到框外。
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, GRAPH_H));
    let mut painter = Painter::new(rect);
    clip_graph(&mut painter, rect, ease, t);
    let ops = painter.ops.clone();
    let name = SharedString::from(ease.name().to_string());
    let mut graph_box = div()
        .id("clip-ease-graph")
        .relative().w(pt(width)).h(pt(GRAPH_H))
        .child(
            canvas(
                |_, _, _| (),
                move |b, _, window, _cx| {
                    let bounds = Bounds { origin: b.origin, size: b.size };
                    let p = Painter { clip: Rect::NOTHING, ops: ops.clone(), labels: Vec::new() };
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        p.paint(window, pos2(f32::from(b.origin.x), f32::from(b.origin.y)));
                    });
                },
            ).w(pt(width)).h(pt(GRAPH_H)),
        )
        // PR `curve_graph`: 曲线名字画在图表左上角 (8, 6)，9pt tertiary。
        .child(
            div().absolute().left(px(8.0)).top(px(6.0))
                .text_size(pt(9.0)).text_color(rgba(t.text_tertiary)).child(name),
        );
    let handles: Vec<u8> = if spring { vec![2] } else if ease.control_points().is_some() { vec![0, 1] } else { vec![] };
    for h in handles {
        if let Some(pos) = clip_handle_pos(rect, ease, h) {
            let q = queue.clone();
            graph_box = graph_box.child(
                div()
                    .absolute().left(px(pos.x - 9.0)).top(px(pos.y - 9.0)).w(px(18.0)).h(px(18.0)).cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |e, _, _| {
                        q.push(Act::EaseBegin);
                        DRAG.with(|d| *d.borrow_mut() = Some(Drag {
                            handle: h,
                            x0: f32::from(e.position.x),
                            y0: f32::from(e.position.y),
                            ease,
                            target,
                            queue: q.clone(),
                        }));
                    }),
            );
        }
    }
    body = body.child(graph_box);

    // 3) values：curve = 一个 `w − 28` 的数值框 + 靠右 24pt Reset（PR `editor` L223-237）；
    //    spring = Elasticity / 物理读数 / Exact envelope / Mass，共 112pt。
    if spring {
        let el = ease.elasticity().unwrap_or(DEFAULT_ELASTICITY);
        let algo = ease.spring_algorithm().unwrap_or_default();
        let mass = ease.mass().unwrap_or_else(unit_mass);
        let q_vals = queue.clone();
        let f = numeric_w(cx, "clip2-elasticity", &fmt_num(el), CONTROL_W, Rc::new(move |s| {
            if let Ok(v) = s.trim().parse::<f32>() { q_vals.push(Act::EaseSet(target, Ease::spring_with(v, algo, mass))); }
        }));
        let (z, w0) = (spring_damping_ratio(el), spring_angular_frequency(el, algo));
        let (k, c) = spring_stiffness_damping(el, mass, algo);
        let q_env = queue.clone();
        let exact = algo == SpringAlgorithm::Tight;
        let q_mass = queue.clone();
        let mf = numeric_w(cx, "clip2-mass", &fmt_num(mass), CONTROL_W, Rc::new(move |s| {
            if let Ok(v) = s.trim().parse::<f32>() { q_mass.push(Act::EaseSet(target, Ease::spring_with(el, algo, v.clamp(0.01, 100.0)))); }
        }));
        // 24 + 8 + 16 + 8 + 24 + 8 + 24 = 112，正好等于 `editor_height` 的 spring 段。
        body = body.child(
            div().flex().flex_col().w_full().h(pt(112.0)).gap(pt(8.0))
                .child(
                    div().flex().flex_row().w_full().h(pt(24.0)).items_center()
                        .child(div().flex_1().min_w(px(0.0)).pl(pt(4.0)).overflow_hidden().whitespace_nowrap().truncate().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Elasticity")))
                        .child(f.into_any_element()),
                )
                .child(
                    // 物理读数长串：单行并按要求裁掉溢出，绝不撑破卡片。
                    div().w_full().h(pt(16.0)).pl(pt(4.0)).pr(pt(4.0))
                        .overflow_hidden().whitespace_nowrap().truncate()
                        .text_size(pt(9.5)).text_color(rgba(t.text_tertiary))
                        .child(SharedString::from(format!("ζ {z:.3} · ω0 {w0:.1} · k {k:.0} · c {c:.1}"))),
                )
                .child(
                    // 二元选择器 → 通用 `Checkbox`（自带 14×14 方框 + 标签 + 24pt 行高）。
                    crate::widgets::checkbox::Checkbox::new("clip-exact-envelope")
                        .label(tr("Exact envelope"))
                        .checked(exact)
                        .text_xs()
                        .on_change(move |next, _, _| {
                            let algo = if *next { SpringAlgorithm::Tight } else { SpringAlgorithm::Envelope };
                            q_env.push(Act::EaseSet(target, Ease::spring_with(el, algo, mass)));
                        }),
                )
                .child(
                    div().flex().flex_row().w_full().h(pt(24.0)).items_center()
                        .child(div().flex_1().min_w(px(0.0)).pl(pt(4.0)).overflow_hidden().whitespace_nowrap().truncate().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Mass")))
                        .child(mf.into_any_element()),
                ),
        );
    } else if let Some(c) = ease.control_points() {
        let txt = c.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(", ");
        let q = queue.clone();
        let f = numeric_w(cx, "clip2-values", &txt, width - 28.0, Rc::new(move |s| {
            if let Some(e) = parse_values(&s) { q.push(Act::EaseSet(target, e)); }
        }));
        let q_reset = queue.clone();
        body = body.child(
            div().flex().flex_row().w_full().h(pt(24.0)).items_center().gap(pt(4.0))
                .child(f.into_any_element())
                .child(IconButton::new(Icon::Rotate).on_click(move |_, _, _| q_reset.push(Act::EaseSet(target, Ease::Linear))).into_any_element()),
        );
    }
    body
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Act, EaseTarget, Queue, SelectedKey};
    use gpui::{point, px};
    use std::rc::Rc;

    #[test]
    fn parse_values_builds_a_custom_bezier() {
        assert_eq!(parse_values("0.1, 0.2, 0.9, 1"), Some(Ease::bezier(0.1, 0.2, 0.9, 1.0)));
        assert_eq!(parse_values("0.1 0.2 0.9 1"), Some(Ease::bezier(0.1, 0.2, 0.9, 1.0)));
        assert_eq!(parse_values("1,2,3"), None);
        assert_eq!(parse_values("a,b,c,d"), None);
    }

    /// 手柄拖拽：从按下瞬间的原值按相对位移重算，且整段只排 Live（Begin/End 由按钮与 root up 负责）。
    #[test]
    fn handle_drag_writes_live_eases_from_the_original() {
        let queue = Queue::new(Rc::new(|| {}));
        let target = EaseTarget::Key(SelectedKey {
            node: figcraft_doc::NodeId(1),
            property: figcraft_doc::anim::motion::AnimProperty::Opacity,
            time_ms: 0,
        });
        let ease = Ease::Linear;
        DRAG.with(|d| *d.borrow_mut() = Some(Drag {
            handle: 0,
            x0: 100.0,
            y0: 100.0,
            ease,
            target,
            queue: queue.clone(),
        }));
        assert!(drag_active());
        route_move(point(px(130.0), px(100.0)));
        let acts = queue.drain();
        assert_eq!(acts.len(), 1);
        match acts[0] {
            Act::EaseLive(t, e) => {
                assert_eq!(t, target);
                assert!(matches!(e, Ease::CustomBezier(_)), "拖出自定义曲线: {e:?}");
            }
            ref other => panic!("expected EaseLive, got {other:?}"),
        }
        // 抬起收尾（推 EaseEnd）。
        end_drag();
        assert!(!drag_active());
        assert!(matches!(queue.drain()[0], Act::EaseEnd));
    }

    /// 弹窗编辑器的高度严格等于 egui PR#1624 `easing::editor_height` 的三档。
    #[test]
    fn clip_editor_height_matches_the_reference() {
        // 24 tabs + 8 + 144 graph + 8 + values(24 / 112 / 0)
        assert_eq!(clip_editor_height(Ease::Linear), 208.0);
        assert_eq!(clip_editor_height(Ease::Hold), 184.0);
        assert_eq!(clip_editor_height(Ease::SpringGentle), 296.0);
        // 264 卡片 − 12×2 pad = 图表实际绘制宽度。
        assert_eq!(crate::motion::clip_popover::WIDTH - 24.0, 240.0);
    }

    /// 弹窗图表的坐标系以 240×144 为基准，手柄不会因旧版 208 宽画布而错位。
    #[test]
    fn clip_graph_and_handles_use_the_card_width() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, GRAPH_H));
        let t = tokens();
        let mut p = Painter::new(rect);
        clip_graph(&mut p, rect, Ease::Linear, t);
        assert!(p.ops.len() >= 3, "底框 + 两条 guide + 曲线");
        // 线性曲线的两端必须正好落在 120pt 方形 plot 的对角，而不是画布左右边缘。
        let poly = p.ops.iter().rev().find_map(|op| match op {
            crate::canvas::painter::Op::Poly { points, .. } => Some(points.clone()),
            _ => None,
        }).expect("曲线折线");
        assert!(poly.len() >= 64, "64 段采样");
        let (x0, x1) = (poly.first().unwrap().x, poly.last().unwrap().x);
        assert!((x0 - 60.0).abs() < 0.01 && (x1 - 180.0).abs() < 0.01,
            "plot 居中在 240 宽卡片里: {x0} .. {x1}");
        let h = clip_handle_pos(rect, Ease::Linear, 0).unwrap();
        assert!((h.x - 60.0).abs() < 0.01 && (h.y - 132.0).abs() < 0.01,
            "plot 上下各留 12pt，第一个控制点落在 (60, 132): {h:?}");
    }

    /// 弹簧峰值手柄改 elasticity，不改 algorithm / mass。
    #[test]
    fn spring_handle_changes_elasticity() {
        let queue = Queue::new(Rc::new(|| {}));
        let target = EaseTarget::Clip(figcraft_doc::NodeId(2), 3);
        let ease = Ease::SpringGentle;
        DRAG.with(|d| *d.borrow_mut() = Some(Drag { handle: 2, x0: 0.0, y0: 100.0, ease, target, queue: queue.clone() }));
        route_move(point(px(0.0), px(80.0)));
        match queue.drain()[0] {
            Act::EaseLive(_, e) => assert!(e.is_spring() && e.elasticity().is_some()),
            ref other => panic!("expected EaseLive, got {other:?}"),
        }
        end_drag();
    }
}
