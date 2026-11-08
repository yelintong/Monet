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
//! The thin input template — the port of `ui-egui/src/widgets/input.rs`'s `Input` / `Frame`.
//!
//! ```text
//! Root (horizontal, vertically centred)
//! ├── Lead   the content or the ability: an icon, a label, a colour dot, a grip
//! ├── Edit   the editable middle — it takes whatever the two slots leave
//! └── Trail  the pure trigger: a click area with no background of its own
//! ```
//!
//! The layout exists once ([`Input::show`]): the lead keeps its width, the middle takes **all** of
//! what is left, the trail keeps its width. Every control that looks like an input derives from
//! it — [`super::text_field::TextField`] (a text editor), the number field, the colour field and
//! the menu field — and adds only three things: what fills the slots, how the middle parses and
//! paints, which abilities the slots have.
//!
//! GPUI has no `Ui::allocate_exact_size`, so the run is a flex row instead: the slots are
//! `flex_none` at their measured widths and the middle is `flex_1`. That is the one difference
//! that matters — the middle fills the container's remaining width whatever it is, where the
//! original got the same result by handing the editor `[lead.max, trail.min]` of a known rect.

use crate::canvas::geom::Color32;
use crate::widgets::input::{Slot, slot_child};
use crate::widgets::{LEAD_PAD, RADIUS, SLOT, pt, rgba, tokens};
use gpui::{AnyElement, CursorStyle, Div, TextRun, Window, div, prelude::*};

/// A rect in the field's own coordinates (origin at the field's top-left).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
}

/// The laid-out parts of an input (`Frame`, `input.rs:331`).
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// The whole field.
    pub rect: Rect,
    /// The leading slot (the grip's rect for a number).
    pub lead: Rect,
    /// The editable middle.
    pub edit: Rect,
    /// The opacity's number, when the field keeps one ([`Input::alpha_cell`]): the part of the
    /// trailing slot left of the unit cell, or a zero rect for a field without it.
    pub alpha: Rect,
    /// The trailing slot.
    pub trail: Rect,
    /// A text lead too long for its slot was cut.
    pub lead_elided: bool,
}

/// The elements the caller paints into the template's parts. Every one is optional: a slot with
/// nothing supplied falls back to its [`Slot`] content, and the middle is simply empty.
#[derive(Default)]
pub struct Parts {
    /// The lead's content *with its behaviour* (a grip, a dot that opens the picker). Its width is
    /// still the slot's, so the middle starts at the same x as the painted value does.
    pub lead: Option<AnyElement>,
    /// The editable middle.
    pub edit: Option<AnyElement>,
    /// The trailing slot's content with its behaviour (a chevron that opens the menu).
    pub trail: Option<AnyElement>,
    /// The opacity's number cell (`alpha_cell` only).
    pub alpha: Option<AnyElement>,
    /// The opacity's unit cell (`alpha_cell` only) — 24 × 24, flush with the right edge.
    pub unit: Option<AnyElement>,
}

/// What an input is and what its slots may do. One layout.
pub struct Input {
    /// The whole field's width, for the [`Frame`] arithmetic. The painted row is `w_full`, so a
    /// stretched container still fills.
    pub width: f32,
    /// The row height, `FIELD_H` unless a shorter row is asked for.
    pub height: f32,
    pub lead: Slot,
    /// The lead's width when the content does not decide it.
    pub lead_w: Option<f32>,
    pub trail: Slot,
    /// The trail's width when the content does not decide it (`alpha_cell` sets it).
    pub trail_w: Option<f32>,
    /// The gap the trail keeps to the field's right edge.
    pub trail_pad: f32,
    /// The trail is drawn only while the field is hovered.
    pub trail_hover_only: bool,
    /// The lead drags the value (`Sense::drag`, a resize cursor).
    pub numeric: bool,
    /// The field ends in an opacity's number beside the unit: `ALPHA_CELL_W` is taken out of the
    /// middle for it, so the value never paints over the number.
    pub alpha_cell: bool,
    /// The lead answers clicks (a colour dot that opens the picker).
    pub lead_click: bool,
    /// The middle is being typed into: it wears the accent hairline.
    pub active: bool,
    /// No shell at all: the field is embedded in a parent control's own frame.
    pub bare: bool,
    /// The fill; `t.field` unless a field asks for its own colour.
    pub fill: Option<Color32>,
    /// The unit cell's width, for the alpha rect (`UNIT_W`).
    pub unit_w: f32,
    /// **整个前插槽**不渲染、也不占宽度（`Field-color` 绑定变量时只剩 edit 一格）。
    /// 注意：这是「插槽整体消失」，不是把内容清空后留一条空列。
    pub hide_lead: bool,
    /// **整个尾插槽**不渲染、也不占宽度：连 alpha 数值格、24 pt 单位格、trail_pad 一起消失，
    /// edit 自动吃掉这部分宽度。设计稿 `Field-color / Disabled`（= 变量绑定态）用的就是它。
    pub hide_trail: bool,
}

impl Input {
    pub fn new(width: f32) -> Self {
        Self {
            width,
            height: crate::widgets::FIELD_H,
            lead: Slot::None,
            lead_w: None,
            trail: Slot::None,
            trail_w: None,
            trail_pad: 0.0,
            trail_hover_only: false,
            numeric: false,
            alpha_cell: false,
            lead_click: false,
            active: false,
            bare: false,
            fill: None,
            unit_w: 24.0,
            hide_lead: false,
            hide_trail: false,
        }
    }

    pub fn lead(mut self, slot: Slot) -> Self {
        self.lead = slot;
        self
    }
    pub fn lead_w(mut self, w: f32) -> Self {
        self.lead_w = Some(w);
        self
    }
    pub fn trail(mut self, slot: Slot) -> Self {
        self.trail = slot;
        self
    }
    pub fn trail_w(mut self, w: f32) -> Self {
        self.trail_w = Some(w);
        self
    }
    pub fn trail_pad(mut self, p: f32) -> Self {
        self.trail_pad = p;
        self
    }
    pub fn trail_hover_only(mut self, yes: bool) -> Self {
        self.trail_hover_only = yes;
        self
    }
    pub fn numeric(mut self, yes: bool) -> Self {
        self.numeric = yes;
        self
    }
    pub fn alpha_cell(mut self, yes: bool) -> Self {
        self.alpha_cell = yes;
        self
    }
    pub fn lead_click(mut self, yes: bool) -> Self {
        self.lead_click = yes;
        self
    }
    pub fn active(mut self, yes: bool) -> Self {
        self.active = yes;
        self
    }
    pub fn bare(mut self, yes: bool) -> Self {
        self.bare = yes;
        self
    }
    pub fn fill(mut self, c: Color32) -> Self {
        self.fill = Some(c);
        self
    }
    /// 隐藏整个前插槽（宽度归零，内容不渲染）。
    pub fn hide_lead(mut self, yes: bool) -> Self {
        self.hide_lead = yes;
        self
    }
    /// 隐藏整个尾插槽（宽度归零，alpha / unit / trail / pad 全部不渲染）。
    pub fn hide_trail(mut self, yes: bool) -> Self {
        self.hide_trail = yes;
        self
    }

    /// `Input::show_at` (`input.rs:237`), first half: measure the slots into a [`Frame`], so the
    /// caller can build its parts against the same rects the paint will use.
    pub fn measure(&self, window: &mut Window) -> Frame {
        // `number_field_impl`'s lead width: `Lead::None => LEAD_PAD` — an empty lead still reserves
        // 8 pt (the number's grip / the value's left inset), it is never 0.
        let lead_w = if self.hide_lead {
            0.0
        } else {
            self.lead_w.unwrap_or_else(|| match &self.lead {
                Slot::None => LEAD_PAD,
                _ => slot_width(&self.lead, window),
            })
        };
        let trail_w = if self.hide_trail {
            0.0
        } else {
            self.trail_w.unwrap_or_else(|| if matches!(self.trail, Slot::None) { 0.0 } else { slot_width(&self.trail, window) })
        };
        self.layout(lead_w, trail_w)
    }

    /// The rects alone (`Frame`): `lead = [0, lead_w]`, `trail = [width - pad - trail_w, trail_w]`,
    /// `edit = [lead.max, trail.min]`, `alpha = [trail.min, trail.max - unit_w]`.
    pub fn layout(&self, lead_w: f32, trail_w: f32) -> Frame {
        // 隐藏的插槽连宽度都不占：调用方就算还按老尺寸传参，edit 也必须吃掉整行。
        let (lead_w, trail_w) = (if self.hide_lead { 0.0 } else { lead_w }, if self.hide_trail { 0.0 } else { trail_w });
        // 隐藏尾插槽时 trail_pad 也一起消失：paint() 不会再画那个空白格，layout() 也不能
        // 把它从 edit 的宽度里扣掉，否则 Frame 与真正画出来的 flex 行对不上。
        let trail_pad = if self.hide_trail { 0.0 } else { self.trail_pad };
        let height = self.height;
        let lead = Rect { x: 0.0, y: 0.0, w: lead_w, h: height };
        let trail = Rect { x: (self.width - trail_pad - trail_w).max(lead.right()), y: 0.0, w: trail_w, h: height };
        let edit = Rect { x: lead.right(), y: 0.0, w: (trail.x - lead.right()).max(0.0), h: height };
        let alpha = if self.alpha_cell && trail_w > self.unit_w {
            Rect { x: trail.x, y: 0.0, w: (trail.w - self.unit_w).max(0.0), h: height }
        } else {
            Rect::default()
        };
        Frame { rect: Rect { x: 0.0, y: 0.0, w: self.width, h: height }, lead, edit, alpha, trail, lead_elided: false }
    }

    /// The shell and the parts, in one flex row. The middle is `flex_1`, so it fills **all** the
    /// room the slots leave — the row is `w_full`, not the nominal width.
    pub fn paint(&self, frame: &Frame, parts: Parts) -> Div {
        let t = tokens();
        let mut root = div().relative().w_full().h(pt(self.height)).flex().flex_row().items_center();
        if !self.bare {
            root = root.rounded(pt(RADIUS)).bg(rgba(self.fill.unwrap_or(t.field)));
            // The 1 px shell is an **absolute overlay**, not a flow `border_1()`: a flow border
            // shrinks the content box by 1 on each side (24 → 22), so the slots could never be the
            // design's 24 × 24 — and the value would sit 1 px right of where the slot ends. The
            // overlay hugs the edge without taking part in layout, exactly `ui-egui`'s
            // `StrokeKind::Inside`. Hover goes through the group because a child cannot read the
            // parent's `hover()` state.
            let mut ring = div().absolute().top_0().left_0().size_full().rounded(pt(RADIUS)).border_1();
            if self.active {
                ring = ring.border_color(rgba(t.accent));
            } else {
                root = root.group("field-shell");
                ring = ring.border_color(rgba(t.field)).group_hover("field-shell", |s| s.border_color(rgba(t.field_hover_border)));
            }
            root = root.child(ring);
        }

        // Lead: its own width, never stretched. A slot that reserves width with nothing in it (a
        // number's 8 pt grip) still takes the room — the middle must start where the value does.
        if !self.hide_lead && frame.lead.w > 0.0 {
            // `lead_rect = vec2(lead_w, FIELD_H)`: the slot is the **full** field height, and it
            // spans the full width from the field's edge — the shell's 1 px stroke is an overlay
            // (above), so it neither shrinks the slot to 22 nor pushes it 1 px inward. Ask for the
            // height explicitly so a future flow border cannot reintroduce the 24 × 22 slot.
            let mut cell =
                crate::widgets::dbg_frame(div().flex_none().w(pt(frame.lead.w)).h(pt(self.height)).flex().items_center().justify_center(), 0xFFB020);
            // `number_field_impl`: the cursor is **the lead's**, not the field's — the 24 pt slot
            // is the whole scrub target (`lead_resp`), so the resize arrow must appear exactly
            // where a drag actually changes the value and nowhere else. Set it on the cell (not
            // only on its content) so the full 24 × 24 reads as draggable, debug frame included.
            if self.numeric {
                cell = cell.cursor(CursorStyle::ResizeLeftRight);
            } else if self.lead_click {
                cell = cell.cursor(CursorStyle::PointingHand);
            }
            if let Some(content) = parts.lead.or_else(|| slot_child(&self.lead)) {
                cell = cell.child(content);
            }
            root = root.child(cell);
        }

        // Edit: everything the slots leave. `min_w(0)` lets it shrink below its content, so a long
        // value elides instead of pushing the trail out of the field.
        let mut edit = div().flex_1().min_w(pt(0.0)).h_full();
        if let Some(e) = parts.edit {
            edit = edit.child(e);
        }
        root = root.child(edit);

        // 尾插槽：整个插槽是一个整体。`hide_trail` 时连同 alpha 数值格、24 pt 单位格、
        // trail_pad 一起消失，而不是留一条空列 —— 变量绑定 / 禁用态要求「插槽整体消失」。
        if !self.hide_trail {
            if self.alpha_cell {
                // The opacity's number, then its unit cell (`alpha_of`'s `[trail.max.x - UNIT_W, 24]`).
                let alpha_w = (frame.trail.w - self.unit_w).max(0.0);
                let mut a = div().flex_none().w(pt(alpha_w)).h(pt(self.height)).flex().items_center();
                if let Some(e) = parts.alpha {
                    a = a.child(e);
                }
                root = root.child(crate::widgets::dbg_frame(a, 0xFFB020));
                let mut u = div().flex_none().w(pt(self.unit_w)).h(pt(self.height)).flex().items_center().justify_center();
                if let Some(e) = parts.unit {
                    u = u.child(e);
                }
                root = root.child(crate::widgets::dbg_frame(u, 0xFFB020));
            } else if frame.trail.w > 0.0 {
                // ONE slot: whatever `Parts.trail` supplies fills it. `Field-color` does NOT go
                // through here — it uses `alpha_cell` above (44 pt number + 24 × 24 "%"), the
                // frozen design where the number is flush against the unit cell.
                let mut cell = crate::widgets::dbg_frame(
                    div().flex_none().w(pt(frame.trail.w)).h(pt(self.height)).flex().items_center().justify_center(),
                    0xFFB020,
                );
                if let Some(content) = parts.trail.or_else(|| slot_child(&self.trail)) {
                    cell = cell.child(content);
                }
                root = root.child(cell);
            }

            // `UNIT_PAD`: nothing after the unit in a `Field-color`; a plain field keeps what it asked.
            if self.trail_pad > 0.0 {
                root = root.child(div().flex_none().w(pt(self.trail_pad)).h(pt(self.height)));
            }
        }
        root
    }
}

/// `Slot::width` (`widgets.rs:693`): `None` = 0; `Text` = `(LEAD_PAD + measured).min(64).max(24)`;
/// `Icon` / `Dot` / `Custom` = the 24 pt `LEAD_ICON` slot. **Every slot is at least 24 × 24** —
/// the design's `Slot` frame — whatever it holds.
pub fn slot_width(slot: &Slot, window: &mut Window) -> f32 {
    match slot {
        Slot::None => 0.0,
        Slot::Text(s) => {
            let run = TextRun {
                len: s.len(),
                font: window.text_style().font(),
                color: rgba(tokens().text_secondary).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(s.clone(), pt(crate::widgets::TEXT_PT), &[run], None);
            (crate::widgets::LEAD_PAD + f32::from(line.width)).min(64.0).max(SLOT)
        }
        Slot::Icon(_) | Slot::Dot(_) | Slot::Custom(_) => SLOT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Field-color / Disabled`：尾插槽（透明度数值 + 24 pt "%" + pad）整体消失，
    /// 宽度归零，edit 吃掉整行；而不是留一条空列。
    #[test]
    fn hidden_slots_take_no_width() {
        let width = 200.0;
        let normal = Input::new(width).lead_w(24.0).trail_w(68.0);
        let f = normal.layout(24.0, 68.0);
        assert_eq!(f.lead.w, 24.0);
        assert_eq!(f.trail.w, 68.0);
        assert_eq!(f.edit.w, width - 24.0 - 68.0);

        // 禁用态只隐藏尾插槽：前插槽（色块）保留，edit 吃掉尾部的 68 pt。
        let bound = Input::new(width).lead_w(24.0).trail_w(68.0).hide_trail(true);
        let f = bound.layout(24.0, 68.0);
        assert_eq!(f.lead.w, 24.0);
        assert_eq!(f.trail.w, 0.0);
        assert_eq!(f.edit.w, width - 24.0);

        // 调用方即使仍按老尺寸传参，隐藏的插槽也不能占宽度（layout 与 measure 同规则）。
        let both = Input::new(width).lead_w(24.0).trail_w(68.0).trail_pad(6.0).hide_lead(true).hide_trail(true);
        let f = both.layout(24.0, 68.0);
        assert_eq!(f.lead.w, 0.0);
        assert_eq!(f.trail.w, 0.0);
        assert_eq!(f.edit.w, width);
    }

    /// `ui-egui::alpha_of` / `paint_alpha`：数字格的右边界就是 24 × 24 "%" 单元的左边界，
    /// 单元本身 flush 到字段右边缘（`UNIT_PAD = 0`）。数字右对齐后必然紧贴那个框。
    #[test]
    fn alpha_cell_touches_the_unit_cell() {
        let width = 200.0;
        let input = Input::new(width).lead_w(24.0).trail_w(68.0).trail_pad(0.0).alpha_cell(true);
        let f = input.layout(24.0, 68.0);
        assert_eq!(f.trail.w, 68.0);
        assert_eq!(f.trail.x + f.trail.w, width); // UNIT_PAD = 0
        assert_eq!(f.alpha.w, 44.0);
        assert_eq!(f.alpha.x + f.alpha.w, f.trail.x + f.trail.w - 24.0);
        assert_eq!(f.alpha.x + f.alpha.w + 24.0, width);
    }
}
