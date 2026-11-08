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
//! `TextField` — the *field shell*: the [`Input`] template's slots plus one [`TextEdit`] in the
//! middle. Ported from `ui-egui`'s `field` / `number_field_impl` / `search_field` / `combo_field`.
//!
//! ```text
//! TextField = Input (lead | edit | trail) + TextEdit (the value)
//! ```
//!
//! The shell owns **no editing state**: the value, the draft, the caret, the scrub and the commit
//! all live in the [`TextEdit`]. That is exactly `ui-egui`'s split — `field()` lays out and draws,
//! `text_field_edit()` edits — and it is what lets a colour field put a bare editor in its own
//! `Input` without nesting a second field.
//!
//! Configuration is queued (`pending`) and handed to the editor at the top of the render, so the
//! shell's setters stay `&mut self` exactly as `ui-egui`'s builder methods are.
//!
//! ## Behaviour checklist (mirrors `ui-egui/src/widgets.rs`)
//! * `field_shell`: `t.field` fill, radius 5, ACCENT border while editing, `t.border` on hover.
//! * `Slot::width`: `Text` = `(8 + measured).min(64).max(24)`; `Icon`/`Dot`/`Custom` = 24.
//! * `text_field_edit`'s inset: `0` after a lead, `8` when the field has none; `8` at the end.
//! * `number_field_impl`: the lead is the scrub grip (Shift ×10, Alt ×0.1).
//! * `search_field`: a clear icon while non-empty; Escape clears and closes the popover.
//! * `combo_field`: the trailing chevron's click opens the caller's menu.

use crate::widgets::icon::Icon;
use crate::widgets::input::Slot;
use crate::widgets::input::text_edit::{NumberConfig, TextEdit};
use crate::widgets::{FIELD_INSET, SLOT, pt};
use gpui::{
    App, Context, CursorStyle, Entity, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Window, div, prelude::*,
};
use std::rc::Rc;

/// Handler that opens a popover (menu / colour picker), with the click position as its anchor.
type PopupHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
type CommitHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Settings applied to the editor at the top of the next render.
#[derive(Default)]
struct Pending {
    value: Option<String>,
    placeholder: Option<SharedString>,
    secret: Option<bool>,
    dim: Option<bool>,
    numeric: Option<(SharedString, f64)>,
    number_cfg: Option<NumberConfig>,
    /// 输入范围 `(min, max)`，`None` 的那一侧不限制。`TextField` 只排队，由 editor 落地。
    range: Option<(Option<f64>, Option<f64>)>,
    readonly: Option<bool>,
    search_clear: Option<bool>,
    on_commit: Option<CommitHandler>,
}

/// The field shell with optional lead/trail slots around one [`TextEdit`].
pub struct TextField {
    width: f32,
    fill: bool,
    /// No shell at all: the row is drawn by a parent control (a `Field-color`).
    bare: bool,
    lead: Slot,
    trail: Slot,
    /// The trailing slot's click handler (opens a menu / popover at the click position).
    on_trail: Option<PopupHandler>,
    /// The leading slot's click handler (a colour dot opens the picker at the click position).
    on_lead: Option<PopupHandler>,
    /// Escape clears this field (and closes its popover).
    search_clear: bool,
    /// The value's editing lifecycle — the only place the text lives.
    edit: Entity<TextEdit>,
    pending: Pending,
}

impl TextField {
    pub fn new(cx: &mut App, width: f32) -> Self {
        Self {
            width,
            fill: false,
            bare: false,
            lead: Slot::None,
            trail: Slot::None,
            on_trail: None,
            on_lead: None,
            search_clear: false,
            edit: TextEdit::build(cx, |_, _| {}),
            pending: Pending::default(),
        }
    }

    pub fn build(cx: &mut App, width: f32, f: impl FnOnce(&mut TextField, &mut App)) -> Entity<Self> {
        cx.new(|cx| {
            let mut s = Self::new(cx, width);
            f(&mut s, cx);
            s
        })
    }

    // ── configuration (queued; the editor receives it on the next frame) ───────────────────
    pub fn set_value(&mut self, v: impl Into<String>) {
        self.pending.value = Some(v.into());
    }
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.pending.placeholder = Some(p.into());
    }
    pub fn set_secret(&mut self, s: bool) {
        self.pending.secret = Some(s);
    }
    pub fn set_dim(&mut self, d: bool) {
        self.pending.dim = Some(d);
    }
    pub fn set_numeric(&mut self, suffix: impl Into<SharedString>, step: f64) {
        self.pending.numeric = Some((suffix.into(), step));
    }
    pub fn set_number_config(&mut self, cfg: NumberConfig) {
        self.pending.number_cfg = Some(cfg);
    }
    /// 输入值范围（含端点）；`None` 的一侧不限制。必须在 `set_numeric` 之后调用。
    pub fn set_range(&mut self, min: Option<f64>, max: Option<f64>) {
        self.pending.range = Some((min, max));
    }
    pub fn set_on_commit(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.pending.on_commit = Some(Rc::new(f));
    }
    pub fn set_readonly(&mut self, b: bool) {
        self.pending.readonly = Some(b);
    }
    pub fn set_lead(&mut self, s: Slot) {
        self.lead = s;
    }
    pub fn set_trail(&mut self, s: Slot) {
        self.trail = s;
    }
    pub fn set_search_clear(&mut self, b: bool) {
        self.search_clear = b;
        self.pending.search_clear = Some(b);
    }
    /// Draw no shell at all: the row is embedded in a parent control's own frame.
    pub fn set_bare(&mut self, b: bool) {
        self.bare = b;
    }
    pub fn set_fill(&mut self, b: bool) {
        self.fill = b;
    }
    /// The field's width. The painted row is `w_full`, so a stretched container still fills.
    pub fn set_width(&mut self, w: f32) {
        self.width = w;
    }
    /// Set the trailing slot's click handler.
    pub fn set_on_trail(&mut self, f: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) {
        self.on_trail = Some(Rc::new(f));
    }
    /// Set the leading slot's click handler (used by the colour dot in a colour field).
    pub fn set_on_lead(&mut self, f: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) {
        self.on_lead = Some(Rc::new(f));
    }
    pub fn value(&self, cx: &App) -> String {
        self.edit.read(cx).value().to_string()
    }
    /// The editor itself — a caller that wants the value without the shell takes this.
    pub fn edit(&self) -> Entity<TextEdit> {
        self.edit.clone()
    }

    /// Flush everything the setters queued into the editor.
    fn sync(&mut self, cx: &mut App) {
        let p = std::mem::take(&mut self.pending);
        self.edit.update(cx, |e, _| {
            if let Some(v) = p.value {
                e.set_value(v);
            }
            if let Some(s) = p.placeholder {
                e.set_placeholder(s);
            }
            if let Some(s) = p.secret {
                e.set_secret(s);
            }
            if let Some(d) = p.dim {
                e.set_dim(d);
            }
            if let Some((suffix, step)) = p.numeric {
                e.set_numeric(suffix, step);
            }
            if let Some(cfg) = p.number_cfg {
                e.set_number_config(cfg);
            }
            // 范围必须在 numeric / number_config 之后应用，否则会被覆盖成「无限制」。
            if let Some((min, max)) = p.range {
                e.set_range(min, max);
            }
            if let Some(b) = p.readonly {
                e.set_readonly(b);
            }
            if let Some(b) = p.search_clear {
                e.set_search_clear(b);
            }
            if let Some(cb) = p.on_commit {
                e.set_on_commit(move |t, w, cx| cb(t, w, cx));
            }
        });
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);

        let numeric = self.edit.read(cx).is_numeric();
        let active = self.edit.read(cx).is_active(window);
        let empty = self.edit.read(cx).is_empty();

        // `Input::show_at` (`input.rs:237`): ONE layout — lead | edit | trail.
        let trail = if self.search_clear && !empty { Slot::Icon(Icon::Close) } else { self.trail.clone() };
        // 只要字段带 trail（内容或点击回调），就**固定预留一个 24 pt 槽**——否则
        // `frame.trail.w == 0`，`set_trail` 放进来的 chevron/文字根本画不出来。
        // 文字（Hug / Fill）要多留 8 pt，免得顶着右边框。
        let is_text_trail = matches!(&trail, Slot::Text(_));
        let trail_w = if self.on_trail.is_some() || trail.has_content() { if is_text_trail { SLOT + 8.0 } else { SLOT } } else { 0.0 };
        let mut input = super::template::Input::new(self.width)
            .lead(self.lead.clone())
            .trail(trail.clone())
            .trail_w(trail_w)
            .numeric(numeric)
            .lead_click(self.on_lead.is_some() || numeric)
            .active(active)
            .bare(self.bare);
        // A number with no label keeps 8 pt of blank as its grip (`text_field_edit`'s inset).
        if !self.lead.has_content() && numeric {
            input = input.lead_w(FIELD_INSET);
        }
        let frame = input.measure(window);

        // `text_field_edit`: no left inset after a lead, 8 when the field has none, 8 at the end.
        // A `Field-color`'s cells are the exception: one shared shell, no text inset at the start.
        let pad_l = if self.bare || frame.lead.w > 0.0 { 0.0 } else { FIELD_INSET };
        let pad_r = if self.bare { 0.0 } else { FIELD_INSET };
        self.edit.update(cx, |e, _| {
            e.set_inset_left(pad_l);
            e.set_right_pad(pad_r);
        });

        // ── the lead: a grip for a number, a click for a colour dot, nothing else ──────────
        let lead = (frame.lead.w > 0.0).then(|| {
            let mut d = div().size_full().flex().items_center().justify_center();
            if let Some(content) = super::slot_child(&self.lead) {
                d = d.child(content);
            }
            if numeric {
                let edit = self.edit.clone();
                d = d.cursor(CursorStyle::ResizeLeftRight).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_this, _e: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                        edit.update(cx, |e, cx| {
                            e.start_scrub();
                            cx.notify();
                        });
                    }),
                );
            } else if self.on_lead.is_some() {
                let cb = self.on_lead.clone();
                d = d.cursor_pointer().on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(cb) = cb.as_ref() {
                        cb(e.position, window, cx);
                    }
                });
            }
            d.into_any_element()
        });

        // ── the middle: the editor entity itself, filling the row's remainder ────────────
        let edit = self.edit.clone().into_any_element();

        // ── the trailing slot: a pure trigger ───────────────────────────────────────────────
        let clear = self.search_clear && !empty;
        let trail_el = if frame.trail.w > 0.0 && (self.on_trail.is_some() || clear) {
            let on_trail = self.on_trail.clone();
            let editor = self.edit.clone();
            // 文字槽靠右排并留 8 pt（不顶边框）；图标槽居中。
            let mut cell = div().size_full().flex().items_center().cursor_pointer();
            cell = if is_text_trail { cell.justify_end().pr(pt(8.0)) } else { cell.justify_center() };
            let mut cell = cell.on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if clear {
                    editor.update(cx, |te, cx| {
                        te.set_value("");
                        cx.notify();
                    });
                }
                if let Some(cb) = on_trail.as_ref() {
                    cb(e.position, window, cx);
                }
            });
            // 画出 trail 槽的内容（chevron / 图标 / 文字），否则槽是空的。
            if let Some(content) = super::slot_child(&trail) {
                cell = cell.child(content);
            }
            Some(cell.into_any_element())
        } else {
            None
        };

        input.paint(&frame, super::template::Parts { lead, edit: Some(edit), trail: trail_el, alpha: None, unit: None })
    }
}
