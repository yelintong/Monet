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
//! A colour row — one [`Input`] template with **two slots**, never a field containing fields.
//!
//! ```text
//! Field-color (enabled) — `Input::alpha_cell(true)`, the frozen template path
//!   Input::new(width).lead_w(24).trail_w(68).trail_pad(0)
//!   ├── lead   24 pt slot  → the 16 pt colour swatch, click opens the picker
//!   ├── edit   hex / paint label (bare `TextEdit`, no shell of its own)
//!   └── trail  68 pt, laid out by the template as ONE horizontal row of two cells
//!              ├── alpha 44 pt — the opacity number, RIGHT aligned, no right pad
//!              └── unit  24 × 24 — the "%" cell, ResizeLeftRight + drag scrub
//!              the number's last glyph ends exactly at the unit cell's left edge
//!
//! Field-color / Disabled (= bound to a variable)
//!   Input::bare(true) — the trailing slot **stays**: alpha 44 + unit 24 = 68
//!   ├── no shell fill at all, no hover ring
//!   ├── lead   24 pt swatch (no separate click target)
//!   ├── edit   variable name chip
//!   ├── alpha  44 pt, empty — the opacity number is gone in the bound state
//!   └── unit   24 × 24 — the "%" gives way to the **detach button** (`Icon::Detach`)
//!   └── lead / the variable-name chip open the picker; 44 pt alpha stays empty;
//!       the 24 pt unit is detach-only
//! ```
//!
//! 绑定态不解构插槽：`hide_trail` 会让整行宽度跳变，也会把「解绑」推到行外另起一个图标。
//! 解绑按钮因此是**本通用控件尾插槽里的一个 24**，调用方只提供点击后要执行的命令。
//!
//! Sizes from `ui-egui/src/widgets/input.rs` (`UNIT_W`, `ALPHA_NUM_W`, `alpha_of`,
//! `paint_alpha`), unchanged: `ALPHA_NUM_W = 44`, `UNIT_W = 24` (24 × 24),
//! `ALPHA_CELL_W = 68`, `UNIT_PAD = 0`, `SWATCH_W = 16`. In the design's instance the
//! trailing `Slot 2` runs 148…199 and the `Frame 191` unit cell is 175…199, with the `%`
//! centred at 182…192 — 7 pt of the cell's own air on either side, and nothing after it.
//! `paint_alpha` draws the number at `cell.max.x - text_w`: it touches the 24 pt cell,
//! it does not float in the middle of the 44 pt alpha rect.
//! The disabled shape is `widgets.rs::bound_field` + the design's `Field-color / Disabled`:
//! a bound paint shows its variable name and the whole row is the click target, with no
//! opacity editing (`paints.rs` only draws the opacity cell for an unbound solid paint).

use crate::canvas::geom::Color32;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::Slot;
use crate::widgets::input::template::{Input, Parts};
use crate::widgets::input::text_edit::TextEdit;
use crate::widgets::swatch::{paint_name, paint_swatch};
use crate::widgets::{SLOT, TEXT_PT, pt, rgba, tokens};
use figcraft_color::Paint;
use gpui::{App, CursorStyle, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window, div, prelude::*};
use std::rc::Rc;

// `ui-egui/src/widgets/input.rs`'s constants, unchanged.
/// The opacity's number cell: three digits at the field's font (`ALPHA_NUM_W`).
pub const ALPHA_NUM_W: f32 = 44.0;
/// The unit cell — the design's `Frame 191`, 24 pt, with `%` centred in it (`UNIT_W`).
pub const UNIT_W: f32 = 24.0;
/// The whole trailing slot of a field with an opacity: number + unit (`ALPHA_CELL_W`).
pub const ALPHA_CELL_W: f32 = ALPHA_NUM_W + UNIT_W;
/// `Field-color` is all slots: the `%` cell is flush with the field's right edge (`UNIT_PAD`).
pub const UNIT_PAD: f32 = 0.0;
/// The swatch inside its 24 pt cell (`SWATCH_W`).
pub const SWATCH_W: f32 = 16.0;

/// A colour row's value slot: an editable hex, or a label (a gradient, image or pattern).
#[derive(Clone)]
pub enum ColorText {
    Hex(SharedString),
    Label(SharedString),
}

/// The click handler of a whole-frame target (the variable picker anchor).
type ClickHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
/// The 24 pt tail button's handler (detach). Unlike the picker it needs no anchor point.
type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// `Field-color / Disabled` — the state a colour row takes when its colour is bound to a
/// variable. The row keeps no shell, keeps its trailing slot (so the frame's geometry does not
/// jump), swaps the "%" unit for a 24 pt detach button and answers clicks over its whole rect.
struct ColorBound {
    name: SharedString,
    on_click: ClickHandler,
    on_detach: Option<ActionHandler>,
}

/// A `Field-color` row, `width` points wide — one [`Input`] with a lead and a trail slot.
pub struct ColorField {
    paint: Paint,
    text: ColorText,
    width: f32,
    /// The hex value's bare editor, sitting in `frame.edit`.
    hex: Entity<TextEdit>,
    /// The opacity's number, sitting in the trail's 44 pt alpha cell.
    opacity: Entity<TextEdit>,
    on_swatch: Option<ClickHandler>,
    /// `Some` = 变量绑定 / 禁用态。
    bound: Option<ColorBound>,
}

impl ColorField {
    /// `hex` and `opacity` are **editors**, not fields: the row draws the one shell they live in.
    ///
    /// `ui-egui`'s `paint_alpha`: the opacity's number is right-aligned in its 44 pt cell with
    /// **no** right inset — it must end exactly where the 24 pt "%" cell begins. The row editor
    /// pool gives every editor the generic 8 pt `field_editor` inset, so the colour row takes
    /// that off its own opacity editor and aligns it right. This only touches this row's editor.
    pub fn new(cx: &mut App, paint: Paint, text: ColorText, width: f32, hex: Entity<TextEdit>, opacity: Entity<TextEdit>) -> Self {
        opacity.update(cx, |te, _| {
            te.set_align(crate::widgets::input::TextAlign::Right);
            te.set_right_pad(0.0);
        });
        Self { paint, text, width, hex, opacity, on_swatch: None, bound: None }
    }

    /// A click on the swatch — the picker opens at that position (`frame.lead_resp`).
    /// Ignored in the bound/disabled state, where the whole frame is the click target.
    pub fn on_swatch(mut self, f: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_swatch = Some(Rc::new(f));
        self
    }

    /// 禁用态 = 变量绑定态：不画背景、尾插槽保留、透明度数值消失、变量名 chip 占中间，
    /// 整框可点击以打开/更换变量。
    pub fn bound(mut self, name: impl Into<SharedString>, on_click: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.bound = Some(ColorBound { name: name.into(), on_click: Rc::new(on_click), on_detach: None });
        self
    }

    /// 绑定态尾插槽（原本 24 × 24 的 "%" 单元）里放一个 24 pt 解绑按钮。
    /// 控件只负责画按钮和拦下这次点击，执行什么命令由调用方给。
    pub fn on_detach(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        if let Some(b) = self.bound.as_mut() {
            b.on_detach = Some(Rc::new(f));
        }
        self
    }

    /// Alias for [`Self::bound`] — the design calls this state "Disabled".
    pub fn disabled(self, name: impl Into<SharedString>, on_click: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.bound(name, on_click)
    }
}

impl IntoElement for ColorField {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let bound = self.bound.is_some();
        // 两种状态都用同一个 68 pt 尾插槽：绑定态只是把 44 pt 数值格留空、把 24 × 24
        // 单元换成解绑按钮。插槽不消失，颜色行的宽度与右边缘就不会跳。
        let trail_w = ALPHA_CELL_W;
        let detach = self.bound.as_ref().and_then(|b| b.on_detach.clone());
        // 绑定态「换绑」点击只交给 lead 与变量名 chip；44 pt alpha 纯留白，
        // 24 pt unit 只放解绑按钮。各 hitbox 互不重叠，不靠传播顺序抢点击。
        let picker = self.bound.as_ref().map(|b| b.on_click.clone());

        // ONE template: lead | edit | trail. The trailing slot is the template's OWN
        // `alpha_cell` row (44 pt number + 24 × 24 unit) — the frozen design, not a hand-built
        // replacement layout. `bare` removes the shell (so the disabled row has no background
        // at all); the slot itself is NEVER hidden.
        let input = Input::new(self.width)
            .lead(Slot::None)
            .lead_w(SLOT)
            .trail(Slot::None)
            .trail_w(trail_w)
            .trail_pad(UNIT_PAD)
            .alpha_cell(true)
            .bare(bound);
        let frame = input.layout(SLOT, trail_w);

        // ── lead: the 16 pt swatch centred in its 24 pt cell (`slot_swatch`) ───────────────
        let paint = self.paint.clone();
        let swatch = div()
            .size(pt(SWATCH_W))
            .child(gpui::canvas(|_bounds, _window, _cx| {}, move |bounds, _, window, _cx| paint_swatch(window, bounds, &paint)).size_full());
        let mut lead = div().size_full().flex().items_center().justify_center().child(swatch);
        if !bound {
            if let Some(f) = self.on_swatch {
                lead = lead.cursor_pointer().on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| f(e.position, window, cx));
            }
        } else if let Some(f) = picker.clone() {
            lead = lead.cursor_pointer().on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| f(e.position, window, cx));
        }
        let lead_el = crate::widgets::dbg_frame(lead, 0xFFB020).into_any_element();

        // ── edit: hex / label (enabled) or the variable name chip (disabled) ──────────────
        let edit = if let Some(b) = &self.bound {
            let short = b.name.rsplit('/').next().unwrap_or(b.name.as_ref()).to_string();
            let chip = div()
                .size_full()
                .rounded(pt(4.0))
                .bg(rgba(t.hover))
                .border_1()
                .border_color(rgba(t.border))
                .pl(pt(8.0))
                .pr(pt(8.0))
                .flex()
                .items_center()
                .overflow_hidden()
                .text_size(pt(TEXT_PT))
                .text_color(rgba(t.text))
                .child(short);
            let mut cell = div().size_full().flex().items_center().px(pt(2.0)).py(pt(3.0)).child(chip);
            if let Some(f) = picker.clone() {
                cell = cell.cursor_pointer().on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| f(e.position, window, cx));
            }
            cell.into_any_element()
        } else {
            match &self.text {
                ColorText::Hex(_) => self.hex.clone().into_any_element(),
                ColorText::Label(v) => {
                    div().size_full().flex().items_center().text_size(pt(TEXT_PT)).text_color(rgba(t.text)).child(v.clone()).into_any_element()
                }
            }
        };

        // ── trail: the template's own two cells (`alpha_of`) ──────────────────────────────
        // `frame.alpha` is `[trail.min, trail.max - 24]`; `frame.trail`'s last 24 × 24 is the
        // unit. The number is right-aligned there (see `new`), so its last glyph touches the
        // unit's left edge; the `%` is centred in its own 24 pt cell, flush with the row's end.
        // `ui.interact(unit, ..., Sense::CLICK | Sense::DRAG)` + `ResizeHorizontal`: the press
        // arms the number editor's scrub, whose window-level tracking keeps the drag alive
        // outside the cell. The unit content fills the whole 24 × 24, so the cursor and the
        // press cover the whole slot, not just the glyph.
        let (alpha, unit) = if bound {
            // 44 pt 数值格保持空白；24 × 24 单元换成解绑按钮（`Icon::Detach`，与
            // `variable.rs::bound_field` 的 detach 图标同一套）。换绑只由 lead 与
            // 变量名 chip 负责，尾插槽里不会混入第二个点击目标。
            let unit = detach.clone().map(|on_detach| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(pt(4.0))
                    .hover(|s| s.bg(rgba(t.hover)))
                    .child(paint_icon(Icon::Detach, rgba(t.text_secondary), 12.0))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        on_detach(window, cx);
                    })
                    .into_any_element()
            });
            // 44 pt 数值格依旧**不画任何东西**（绑定态的透明百分比消失），但它与 24 pt
            // unit 同属保留下来的 68 pt 尾插槽：整条插槽都是解绑热区，图标只画在 unit 里。
            // 这样图标若与 24 pt 槽的 hitbox 有一两个像素的偏差，按钮也照样点得到。
            let alpha = detach.map(|on_detach| {
                div()
                    .size_full()
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        on_detach(window, cx);
                    })
                    .into_any_element()
            });
            (alpha, unit)
        } else {
            let opacity = self.opacity.clone();
            let unit = div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(pt(TEXT_PT))
                .text_color(rgba(t.text))
                .child("%")
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    opacity.update(cx, |e, cx| {
                        e.start_scrub();
                        cx.notify();
                    });
                })
                .into_any_element();
            (Some(self.opacity.clone().into_any_element()), Some(unit))
        };

        // Disabled: only lead and the variable-name chip open the picker; the 44 pt alpha is
        // empty and the 24 pt unit owns detach. No frame-level click target, so the two actions
        // can never share a hitbox (and no overlay can silently cover the unit again).
        input.paint(&frame, Parts { lead: Some(lead_el), edit: Some(edit), trail: None, alpha, unit })
    }
}

/// The label a non-solid paint shows in the value slot (`ui-egui`'s `paint_label`).
pub fn paint_label(paint: &Paint) -> String {
    paint_name(paint)
}

/// `figcraft_color::Color` → `Color32`, for the panels that read a document paint.
pub fn color32(c: figcraft_color::Color) -> Color32 {
    crate::widgets::swatch::color32(c)
}
