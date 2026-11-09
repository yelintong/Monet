// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// 2026-11-05 补充：用户随后明确指令「重构 color_field / template 插槽隐藏 / number_field 范围 /
// 失焦提交」，本文件按该指令解冻；以上 INVARIANT 仍保持不变。
// ============================================================================
//! `TextEdit` — `ui-egui`'s **`text_field_edit` / `field_editor`**: the editing lifecycle only.
//!
//! It owns no shell, no slots and no layout. The caller gives it a place to live (`frame.edit` in a
//! [`super::template::Input`], or the number cell of a `Field-color`'s trail) and it paints the
//! value, takes the caret, and commits. That split is what `ui-egui` does:
//!
//! ```text
//! field()        → Input (shell + slots + layout) + text_field_edit into frame.edit
//! color_input()  → Input (shell + slots + layout) + text_field_edit into frame.edit
//!                                              + text_field_edit into frame.alpha
//! ```
//!
//! A colour field is therefore **one** `Input`, not a field containing two fields.

use crate::widgets::input::{EditOutcome, TextAlign, TextInput};
use crate::widgets::{FIELD_H, dbg_frame, pt};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, EntityInputHandler, FocusHandle, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, UTF16Selection, Window, canvas, div, prelude::*, px,
};
use std::ops::Range;
use std::rc::Rc;

type CommitHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// How a number field steps with the arrow keys / scrubs with the pointer, and the range it
/// accepts. `ui-egui`'s `number_field` never clamps at the widget — the callers do it on write —
/// but the design requires the input itself to reject out-of-range values (opacity 0..100,
/// corner ≥ 0, star points 3..60 …), so the clamp lives here in one place.
#[derive(Clone, Copy, Debug)]
pub struct NumberConfig {
    pub step: f64,
    /// Big step (Shift / ten steps) — egui's `key_steps` uses `step * 10` unless a nudge pref is set.
    pub big: f64,
    /// `Empty::Auto`: typing the word "Auto" (or "auto") commits 0.
    pub auto: bool,
    /// 允许的最小值（含）；`None` = 不限制下界。
    pub min: Option<f64>,
    /// 允许的最大值（含）；`None` = 不限制上界。
    pub max: Option<f64>,
}

impl NumberConfig {
    /// A plain config with no range (the old behaviour).
    pub fn new(step: f64) -> Self {
        Self { step, big: step * 10.0, auto: false, min: None, max: None }
    }
    /// Attach an inclusive `[min, max]` range; `None` leaves that side unbounded.
    pub fn with_range(mut self, min: Option<f64>, max: Option<f64>) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    /// Clamp a value into the configured range. Non-finite input stays untouched.
    pub fn clamp(&self, v: f64) -> f64 {
        if !v.is_finite() {
            return v;
        }
        let v = self.min.map_or(v, |lo| v.max(lo));
        self.max.map_or(v, |hi| v.min(hi))
    }
}

/// The editing lifecycle of one field's value.
pub struct TextEdit {
    pub(crate) input: TextInput,
    /// The committed value, and what it was when the current draft began (`kept` in egui).
    value: String,
    kept: String,
    editing: bool,
    /// The click that starts an edit has not released yet: keep select-all laid until it does.
    select_all_pending: bool,
    /// `keep_focus`: the editor element is new on the start frame, ask for focus once it exists.
    focus_pending: bool,
    number: Option<NumberConfig>,
    pub(crate) focus: FocusHandle,
    /// 失焦订阅：焦点离开就立即提交，不再依赖「下一帧恰好重绘到这个 Entity」。
    /// GPUI 会缓存未标脏的嵌套 Entity 的 prepaint（`view.rs:208-214`），把提交只挂在
    /// `render()` 里的失焦轮询上，点画布时会漏掉。
    focus_out: Option<gpui::Subscription>,
    on_commit: Option<CommitHandler>,
    /// Scrub state: the value at drag start and the accumulated amount (egui's `scrub_id`).
    pub(crate) scrub: Option<(f64, f64)>,
    pub(crate) scrub_x: Option<Pixels>,
    last_click: Option<(std::time::Instant, Point<Pixels>, u32)>,
    /// `text_field_edit`'s left inset: `0` when the field has a lead, `8` when it has none.
    inset_left: f32,
    /// `field_editor_in`'s right pad — `8` for a field's value, `0` for a cell that can spare none.
    right_pad: f32,
    /// Right-align the value: the opacity's number sits right-aligned in its 44 pt cell.
    align: TextAlign,
    /// A read-only field never enters edit mode (a display-only combo).
    readonly: bool,
    /// Escape clears this field (a search box).
    search_clear: bool,
}

impl TextEdit {
    pub fn new(cx: &mut App) -> Self {
        Self {
            input: TextInput::new(),
            value: String::new(),
            kept: String::new(),
            editing: false,
            select_all_pending: false,
            focus_pending: false,
            number: None,
            focus: cx.focus_handle(),
            focus_out: None,
            on_commit: None,
            scrub: None,
            scrub_x: None,
            last_click: None,
            inset_left: 0.0,
            right_pad: 0.0,
            align: TextAlign::Left,
            readonly: false,
            search_clear: false,
        }
    }

    pub fn build(cx: &mut App, f: impl FnOnce(&mut TextEdit, &mut App)) -> gpui::Entity<Self> {
        cx.new(|cx| {
            let mut s = Self::new(cx);
            f(&mut s, cx);
            s
        })
    }

    // ── configuration ───────────────────────────────────────────────────────────────────────
    /// Push the document's value in. A value that did not change is a no-op, and a field the user
    /// is typing into keeps its draft: `ui-egui` redraws the same number every frame without ever
    /// touching the editor's text, so a push that overwrote a live draft would be a bug there too.
    pub fn set_value(&mut self, v: impl Into<String>) {
        let v = v.into();
        // 外部推来的文档值也要先夹进范围：否则一个越界的文档值会绕过输入限制显示出来。
        // 非数值（hex、paint 名、"Mixed"）parse 失败，原样保留。
        let v = match self.number {
            Some(cfg) => v.parse::<f64>().ok().map(|n| fmt_num(cfg.clamp(n))).unwrap_or(v),
            None => v,
        };
        if v == self.value {
            return;
        }
        self.value = v;
        self.kept = self.value.clone();
        if self.editing {
            return;
        }
        self.input.buffer.set_text(self.value.clone());
    }
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.input.set_placeholder(p);
    }
    pub fn set_secret(&mut self, s: bool) {
        self.input.set_secret(s);
    }
    pub fn set_dim(&mut self, d: bool) {
        self.input.set_dim(d);
    }
    /// `text_field_edit`'s inset: `0` after a lead, `8` when the field has none.
    pub fn set_inset_left(&mut self, px: f32) {
        self.inset_left = px;
    }
    /// `field_editor_in`'s right pad.
    pub fn set_right_pad(&mut self, px: f32) {
        self.right_pad = px;
    }
    pub fn set_align(&mut self, a: TextAlign) {
        self.align = a;
        self.input.set_align(a);
    }
    pub fn set_readonly(&mut self, b: bool) {
        self.readonly = b;
    }
    pub fn set_search_clear(&mut self, b: bool) {
        self.search_clear = b;
    }
    /// Ported from `number_field_bindable`: suffix + step (big step defaults to `step * 10`).
    pub fn set_numeric(&mut self, suffix: impl Into<SharedString>, step: f64) {
        self.input.set_suffix(suffix);
        self.number = Some(NumberConfig::new(step));
    }
    pub fn set_number_config(&mut self, cfg: NumberConfig) {
        self.number = Some(cfg);
    }
    /// 设置允许的输入范围（含端点），并把当前值夹进去。
    /// 在 `set_numeric` / `set_number_config` **之后**调用，否则会被后写的配置覆盖。
    pub fn set_range(&mut self, min: Option<f64>, max: Option<f64>) {
        let cfg = self.number.get_or_insert_with(|| NumberConfig::new(1.0));
        cfg.min = min;
        cfg.max = max;
        let cfg = *cfg;
        if let Ok(v) = self.value.parse::<f64>() {
            let next = fmt_num(cfg.clamp(v));
            self.value = next.clone();
            self.kept = next.clone();
            if !self.editing {
                self.input.buffer.set_text(next);
            }
        }
    }
    pub fn set_on_commit(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.on_commit = Some(Rc::new(f));
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn is_empty(&self) -> bool {
        self.input.buffer.is_empty()
    }
    pub fn is_readonly(&self) -> bool {
        self.readonly
    }
    /// The field takes a number: its lead drags, Arrows step, typing parses `+ - * /`.
    pub fn is_numeric(&self) -> bool {
        self.number.is_some()
    }
    /// The editor owns the keyboard this frame (the shell wears the accent hairline).
    pub fn is_active(&self, window: &Window) -> bool {
        self.editing && self.focus.is_focused(window)
    }
    /// Take the keyboard without a click — a caller that just opened a panel around this editor
    /// (the Actions palette) puts the caret in it, so typing filters instead of running canvas
    /// shortcuts.
    pub fn focus_editor(&mut self, window: &mut Window) {
        self.begin_edit(window, false);
    }
    /// The lead's press: arm a scrub from the current value (`Sense::drag` on the grip).
    pub fn start_scrub(&mut self) {
        self.scrub = Some((self.value.parse::<f64>().unwrap_or(0.0), 0.0));
        self.scrub_x = None;
    }

    // ── edit lifecycle (egui's `text_field_edit`) ───────────────────────────────────────────
    fn begin_edit(&mut self, window: &mut Window, select_all: bool) {
        let was = self.editing;
        self.editing = true;
        if !was {
            self.kept = self.value.clone();
            self.input.buffer.set_text(self.value.clone());
        }
        if select_all {
            self.input.buffer.select_all();
            self.select_all_pending = true;
        }
        // `keep_focus`: the editor element is new this frame, ask again once it exists.
        self.focus.focus(window);
        self.focus_pending = !self.focus.is_focused(window);
    }

    /// `select_all_on_focus`: lay select-all again until the pointer is released. A drag or a
    /// typed key cancels it, so a drag selects what it covers instead.
    fn tick_select_all(&mut self) {
        if self.select_all_pending && !self.input.buffer.composing() {
            self.input.buffer.select_all();
        }
    }

    /// Commit only when the draft differs from `kept` — egui: "a field left as it was commits
    /// nothing: no undo step, and a rounded display isn't written back".
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.editing {
            return false;
        }
        self.editing = false;
        self.select_all_pending = false;
        let draft = self.input.buffer.text().to_string();
        let value = self.resolve_number(&draft);
        let changed = value != self.kept;
        self.value = value.clone();
        self.kept = value.clone();
        self.input.buffer.set_text(value.clone());
        if !changed {
            return false;
        }
        if let Some(cb) = self.on_commit.clone() {
            cb(&value, window, cx);
        }
        true
    }

    /// 外部要求立即结束编辑并提交（例如点画布时由面板统一调用）。
    /// 没有在编辑时是 no-op，重复调用也安全。
    pub fn finish_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit(window, cx)
    }

    fn cancel(&mut self, window: &mut Window) {
        let saved = self.kept.clone();
        self.input.buffer.set_text(saved);
        self.editing = false;
        self.select_all_pending = false;
        window.blur();
    }

    /// egui's `typed`: absolute value, or `+ - * /` applied to the value at edit start.
    /// `Empty::Auto` also accepts the word "auto".
    fn resolve_number(&self, text: &str) -> String {
        resolve_number_with(self.number, &self.kept, text)
    }

    /// ArrowUp/Down step, both while idle (`on_field`) and while typing. The stepped value
    /// stays selected so typing replaces it (egui's `selectall` temp).
    fn step_number(&mut self, dir: f64, big: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = self.number else { return };
        let by = (if big { cfg.big } else { cfg.step }) * dir;
        let base = if self.editing { self.input.buffer.text().to_string() } else { self.value.clone() };
        let cur = self.resolve_number(&base).parse::<f64>().unwrap_or(0.0);
        let next = fmt_num(cfg.clamp(cur + by));
        self.input.buffer.set_text(next.clone());
        if self.editing {
            self.input.buffer.select_all();
        }
        self.value = next.clone();
        self.kept = next.clone();
        if let Some(cb) = self.on_commit.clone() {
            cb(&next, window, cx);
        }
        cx.notify();
    }

    /// egui's `scrub_amount` / `scrubbed`: Shift ×10, Alt ×0.1, quantised to whole steps.
    pub(crate) fn scrub_by(&mut self, dx: f64, shift: bool, alt: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = self.number else { return };
        let rate = match (shift, alt) {
            (true, false) => 10.0,
            (false, true) => 0.1,
            _ => 1.0,
        };
        let (v0, amount) = self.scrub.unwrap_or((self.value.parse::<f64>().unwrap_or(0.0), 0.0));
        let amount = amount + dx * cfg.step * rate;
        self.scrub = Some((v0, amount));
        let quantum = if alt { cfg.step * 0.1 } else { cfg.step };
        let raw = if quantum > 0.0 && quantum.is_finite() { v0 + (amount / quantum).round() * quantum } else { v0 };
        let next = fmt_num(cfg.clamp(raw));
        self.value = next.clone();
        self.input.buffer.set_text(next.clone());
        if let Some(cb) = self.on_commit.clone() {
            cb(&next, window, cx);
        }
        cx.notify();
    }
}

/// 纯函数形式的 `typed`：绝对值，或以编辑开始时的值为基准计算 `+ - * /`。
/// 抽出来是为了让「范围夹紧」可以在没有 `Entity<TextEdit>` 的单元测试里直接验证。
fn resolve_number_with(number: Option<NumberConfig>, kept: &str, text: &str) -> String {
    let Some(cfg) = number else { return text.to_string() };
    let t = text.trim();
    if t.is_empty() {
        return text.to_string();
    }
    if cfg.auto && t.eq_ignore_ascii_case("auto") {
        return "0".to_string();
    }
    let cur = kept.parse::<f64>().ok();
    let parsed = if let Some(rest) = t.strip_prefix('+') {
        cur.and_then(|c| rest.parse::<f64>().ok().map(|x| c + x))
    } else if let Some(rest) = t.strip_prefix('-') {
        cur.and_then(|c| rest.parse::<f64>().ok().map(|x| c - x))
    } else if let Some(rest) = t.strip_prefix('*') {
        cur.and_then(|c| rest.parse::<f64>().ok().map(|x| c * x))
    } else if let Some(rest) = t.strip_prefix('/') {
        cur.and_then(|c| rest.parse::<f64>().ok().filter(|x| *x != 0.0).map(|x| c / x))
    } else {
        t.parse::<f64>().ok()
    };
    parsed.map(|v| fmt_num(cfg.clamp(v))).unwrap_or_else(|| text.to_string())
}

impl Render for TextEdit {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 失焦提交：焦点一离开就由 GPUI 的 focus-out 事件直接 commit，不依赖「下一次恰好
        // 重绘到这个 Entity」。GPUI 对未标脏的嵌套 Entity 会复用缓存的 prepaint，只把提交
        // 放在 render 里轮询焦点，点画布（焦点移到画布 FocusHandle）时就会漏提交。
        if self.focus_out.is_none() {
            let focus = self.focus.clone();
            self.focus_out = Some(cx.on_focus_out(&focus, window, |this, _e, window, cx| {
                if this.finish_edit(window, cx) {
                    cx.notify();
                }
            }));
        }
        // `text_field_edit`: losing focus commits (Enter commits in key()). 保留这条兜底，
        // 供没有触发 focus-out 事件（如窗口失活）的情况。
        if self.editing && !self.focus.is_focused(window) && !self.focus_pending {
            self.commit(window, cx);
        }
        if self.focus_pending && self.focus.is_focused(window) {
            self.focus_pending = false;
        }
        let focused = self.editing && self.focus.is_focused(window);
        if focused {
            self.tick_select_all();
        }

        let entity = cx.entity();
        let focus = self.focus.clone();
        let cursor = self.editing.then_some(CursorStyle::IBeam);
        let mut el = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .pl(pt(self.inset_left))
            .pr(pt(self.right_pad))
            .child(self.input.element(entity, focus, focused, cursor));

        // ── keyboard: `track_focus` registers the handle in the element tree, which is what
        //    routes keystrokes here. ──────────────────────────────────────────────────────────
        el = el.track_focus(&self.focus);

        // ── scrub: window-level move/up, so it keeps tracking outside the field ────────────
        if self.scrub.is_some() {
            let entity = cx.entity();
            el = el.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let mover = entity.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            let shift = window.modifiers().shift;
                            let alt = window.modifiers().alt;
                            mover.update(cx, |this, cx| {
                                // 窗口外松手：没有 `on_mouse_up` 会送到这里，下一帧 move 才带
                                // 着「按键已松开」回来。立刻结束 scrub，否则回窗后还会继续乱跳。
                                if e.pressed_button != Some(MouseButton::Left) {
                                    this.scrub = None;
                                    this.scrub_x = None;
                                    cx.notify();
                                    return;
                                }
                                if this.scrub.is_none() {
                                    return;
                                }
                                match this.scrub_x {
                                    Some(prev) => {
                                        let dx = f64::from(e.position.x - prev);
                                        if dx != 0.0 {
                                            this.scrub_x = Some(e.position.x);
                                            this.scrub_by(dx, shift, alt, window, cx);
                                        }
                                    }
                                    // First move after the press seeds the origin (no jump).
                                    None => this.scrub_x = Some(e.position.x),
                                }
                            });
                        });
                        let upper = entity.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            upper.update(cx, |this, cx| {
                                if this.scrub.take().is_some() {
                                    this.scrub_x = None;
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .size_full(),
            );
        }

        // ── pointer: click positions the caret, drag selects, double-click selects a word,
        //    triple-click selects the line ───────────────────────────────────────────────────
        el = el
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    if this.readonly {
                        // 只读字段不算「点进输入框」：让事件继续冒泡，由根视图统一提交别的草稿。
                        return;
                    }
                    // 点输入框自己 = 进入/继续编辑，**不要**让根视图把这次按下当成「点别处」
                    // 而把编辑提交掉（否则刚点进来就被收走，表现为点不进编辑态）。
                    cx.stop_propagation();
                    let now = std::time::Instant::now();
                    let count = match this.last_click {
                        Some((at, p, n))
                            if now.duration_since(at).as_millis() < 400
                                && (p.x - e.position.x).abs() < px(6.0)
                                && (p.y - e.position.y).abs() < px(6.0) =>
                        {
                            n + 1
                        }
                        _ => 1,
                    };
                    this.last_click = Some((now, e.position, count));
                    let idx = this.input.index_for_point(e.position);
                    match count {
                        1 => {
                            let starting = !this.editing;
                            this.begin_edit(window, starting && !e.modifiers.shift);
                            if !starting {
                                if e.modifiers.shift {
                                    this.input.buffer.select_to(idx);
                                } else {
                                    this.input.buffer.move_to(idx);
                                }
                            }
                            this.input.buffer.set_selecting(true);
                        }
                        2 => {
                            this.begin_edit(window, false);
                            this.input.buffer.select_word(idx);
                            this.select_all_pending = false;
                        }
                        _ => {
                            this.begin_edit(window, false);
                            this.input.buffer.select_line();
                            this.select_all_pending = false;
                        }
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                if this.scrub.is_some() {
                    return;
                }
                if this.input.buffer.is_selecting() {
                    let idx = this.input.index_for_point(e.position);
                    this.input.buffer.select_to(idx);
                    if !this.input.buffer.is_all_selected() {
                        this.select_all_pending = false;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, cx| {
                    this.input.buffer.set_selecting(false);
                    this.select_all_pending = false;
                    this.scrub = None;
                    this.scrub_x = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, cx| {
                    this.input.buffer.set_selecting(false);
                    this.select_all_pending = false;
                    this.scrub = None;
                    this.scrub_x = None;
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                let ctrl = m.platform || m.control;
                // GPUI names the arrows `up`/`down`/`left`/`right`; `EditBuffer` and egui use
                // `arrowup`… Without this the arrow branch never matched.
                let key = match e.keystroke.key.as_str() {
                    "left" => "arrowleft",
                    "right" => "arrowright",
                    "up" => "arrowup",
                    "down" => "arrowdown",
                    k => k,
                };
                if ctrl {
                    match key {
                        "a" => this.input.buffer.select_all(),
                        "c" => {
                            let r = this.input.buffer.range();
                            let s: String = this.input.buffer.text().chars().skip(r.start).take(r.end - r.start).collect();
                            if !s.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(s));
                            }
                        }
                        "x" => {
                            let r = this.input.buffer.range();
                            let s: String = this.input.buffer.text().chars().skip(r.start).take(r.end - r.start).collect();
                            if !s.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(s));
                                this.input.buffer.insert("");
                            }
                        }
                        "v" => {
                            let text = cx.read_from_clipboard().and_then(|i| i.text().map(|s| s.to_string()));
                            if let Some(s) = text {
                                this.input.buffer.insert(&s);
                            }
                        }
                        _ => return,
                    }
                    this.select_all_pending = false;
                    cx.notify();
                    return;
                }
                // Arrow stepping for number fields (egui's `arrow_step`), idle or typing.
                if this.number.is_some() {
                    match key {
                        "arrowup" => {
                            this.step_number(1.0, m.shift, window, cx);
                            return;
                        }
                        "arrowdown" => {
                            this.step_number(-1.0, m.shift, window, cx);
                            return;
                        }
                        _ => {}
                    }
                }
                // Escape clears a search box (and closes its popover in egui), else cancels.
                if key == "escape" && this.search_clear {
                    this.input.buffer.set_text("");
                    this.value.clear();
                    this.kept.clear();
                    this.editing = false;
                    window.blur();
                    if let Some(cb) = this.on_commit.clone() {
                        cb("", window, cx);
                    }
                    cx.notify();
                    return;
                }
                // Printable characters belong to the platform input handler.
                if let Some(ch) = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ') {
                    if !this.editing {
                        this.begin_edit(window, false);
                        this.input.buffer.set_text("");
                        this.input.buffer.key(&ch.to_string(), false, false);
                        this.select_all_pending = false;
                        cx.notify();
                    }
                    return;
                }
                if !this.editing {
                    let ch = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ');
                    if ch.is_some() {
                        this.begin_edit(window, false);
                        this.input.buffer.set_text("");
                    }
                }
                let word = m.alt;
                let ch = e.keystroke.key_char.as_ref().and_then(|s| s.chars().next()).filter(|c| *c >= ' ');
                let typed = ch.map(|c| c.to_string()).unwrap_or_else(|| key.to_string());
                match this.input.buffer.key(&typed, m.shift, word) {
                    EditOutcome::Submit => {
                        this.commit(window, cx);
                        window.blur();
                    }
                    EditOutcome::Cancel => this.cancel(window),
                    EditOutcome::Changed => {
                        this.select_all_pending = false;
                        cx.notify();
                    }
                    EditOutcome::None => {}
                }
            }));
        dbg_frame(el, 0xFF453A)
    }
}

impl EntityInputHandler for TextEdit {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        self.input.input_text_for_range(range, adjusted)
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        self.input.input_selected_range()
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.input.input_marked_range()
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.input.input_unmark();
        cx.notify();
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.input.input_replace(range, text);
        self.select_all_pending = false;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.input_replace_and_mark(range, new_text, selected);
        self.select_all_pending = false;
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.input.input_bounds_for_range(range, element_bounds)
    }
    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.input.input_character_index_for_point(point)
    }
}

/// `fmt_num` from `ui-egui`: two decimals, `-0` and float noise removed, non-finite → empty.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

/// `FIELD_H` is the row height the editor fills; kept here so callers can size a cell.
pub const EDIT_H: f32 = FIELD_H;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_num_matches_egui() {
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(-0.0001), "0");
        assert_eq!(fmt_num(12.3456), "12.35");
        assert!(fmt_num(f64::NAN).is_empty());
    }

    /// 输入侧范围：百分比 0..100、尺寸 ≥0、点数 3..60，未配置时与原行为一致。
    #[test]
    fn number_range_clamps_both_sides() {
        let pct = NumberConfig::new(1.0).with_range(Some(0.0), Some(100.0));
        assert_eq!(pct.clamp(-12.0), 0.0);
        assert_eq!(pct.clamp(250.0), 100.0);
        assert_eq!(pct.clamp(37.5), 37.5);

        let nonneg = NumberConfig::new(1.0).with_range(Some(0.0), None);
        assert_eq!(nonneg.clamp(-8.0), 0.0);
        assert_eq!(nonneg.clamp(1000.0), 1000.0);

        let points = NumberConfig::new(1.0).with_range(Some(3.0), Some(60.0));
        assert_eq!(points.clamp(1.0), 3.0);
        assert_eq!(points.clamp(99.0), 60.0);

        let plain = NumberConfig::new(1.0);
        assert_eq!(plain.clamp(-999.0), -999.0);
        assert_eq!(plain.clamp(f64::INFINITY), f64::INFINITY);
    }

    /// 打字（含 `+ - * /` 表达式）最终也要落在范围内，而不是只在写回文档时夹紧。
    #[test]
    fn resolve_number_applies_the_range() {
        let cfg = NumberConfig::new(1.0).with_range(Some(0.0), Some(100.0));
        let s = |v: &str, kept: &str, number| resolve_number_with(number, kept, v);
        assert_eq!(s("250", "100", Some(cfg)), "100");
        assert_eq!(s("-150", "100", Some(cfg)), "0");
        assert_eq!(s("-30", "100", Some(cfg)), "70");
        assert_eq!(s("+50", "80", Some(cfg)), "100");
        assert_eq!(s("*3", "80", Some(cfg)), "100");
        assert_eq!(s("/2", "80", Some(cfg)), "40");
        assert_eq!(s("/0", "80", Some(cfg)), "/0");
        assert_eq!(s("  ", "80", Some(cfg)), "  ");
        assert_eq!(s("250", "100", None), "250");
    }
}
