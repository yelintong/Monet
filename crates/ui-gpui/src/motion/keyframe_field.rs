// ============================================================================
// 带「添加关键帧」按钮的输入框 —— 从 `widgets::input::template::Input` **延伸**出来的控件。
//
// 用户 2026-11-06 要求：「动画添加关键帧的那个带按钮输入框自己从 template 延伸过来，
// 后面有插槽」。所以这里**不手写 flex 行**：直接调 `Input::new(...).paint(...)`，
//   lead  = Slot::Text("X" / "Y" / "W" …)      → template 自己量宽并居中
//   edit  = Entity<TextEdit>                   → 复用编辑内核（数值、范围、失焦提交）
//   trail = Slot::Custom(菱形按钮工厂)          → 后面的插槽，点击增删关键帧
// `widgets/` 是 FROZEN，本文件不改其中任何一行；菱形复用 `Icon::Component` /
// `Icon::Instance`（实心 / 空心菱形），不是自己画的图形。
// ============================================================================
//! The Motion inspector's keyframe field: one `Input` template with a diamond in its trailing
//! slot. Any panel that needs "a value plus an action button" can reuse [`KeyframeField`].

use figcraft_i18n::tr_noop;
use figcraft_i18n::tr;
use figcraft_doc::anim::motion::AnimProperty;
use figcraft_doc::{Document, NodeId};
use gpui::{App, Context, Div, Entity, MouseButton, Render, Window, div, prelude::*};

use crate::motion::{Act, MotionUi, Queue, has_any_key, has_key, value_of};
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::Slot;
use crate::widgets::input::template::{Input, Parts};
use crate::widgets::input::text_edit::TextEdit;
use crate::widgets::input::slot_child;
use crate::widgets::{FIELD_H, SLOT, pt, rgba, tokens};

/// The inspector's fields, in the reference's order. The last three are the reused Design
/// rows (Fill / Stroke / Effects) that also carry a keyframe diamond.
pub const FIELDS: [(AnimProperty, usize, &str); 12] = [
    (AnimProperty::Position, 0, "X"),
    (AnimProperty::Position, 1, "Y"),
    (AnimProperty::Scale, 0, "X"),
    (AnimProperty::Scale, 1, "Y"),
    (AnimProperty::Rotation, 0, "°"),
    (AnimProperty::Width, 0, "W"),
    (AnimProperty::Height, 0, "H"),
    (AnimProperty::Opacity, 0, "%"),
    (AnimProperty::CornerRadius, 0, tr_noop("R")),
    (AnimProperty::FillColor, 0, tr_noop("Fill")),
    (AnimProperty::StrokeColor, 0, tr_noop("Stroke")),
    (AnimProperty::ShadowBlur, 0, tr_noop("Blur")),
];

/// One field: `Input(lead | edit | trail)` where `trail` is the keyframe diamond.
pub struct KeyframeField {
    width: f32,
    lead: Slot,
    property: AnimProperty,
    idx: usize,
    node: Option<NodeId>,
    /// 播放头正好停在该属性的某个帧上 → 实心菱形（egui 原行为，保留）。
    keyed: bool,
    /// 整条轨道有任意帧 → 插针常亮（空心、accent 描边 + 矩形底），与播放头无关。
    has_any: bool,
    queue: Queue,
    edit: Entity<TextEdit>,
}

impl KeyframeField {
    pub fn build(cx: &mut App, width: f32, property: AnimProperty, idx: usize, lead: &str, queue: Queue) -> Entity<Self> {
        cx.new(|cx| Self {
            width,
            lead: Slot::Text(lead.to_string().into()),
            property,
            idx,
            node: None,
            keyed: false,
            has_any: false,
            queue: queue.clone(),
            edit: TextEdit::build(cx, |e, _| {
                e.set_numeric("", 1.0);
                e.set_inset_left(0.0);
                e.set_right_pad(8.0);
                if matches!(property, AnimProperty::Opacity) {
                    e.set_range(Some(0.0), Some(100.0));
                }
            }),
        })
    }

    /// Refresh the shown value and the diamond's keyed state from the document.
    pub fn sync(&mut self, cx: &mut App, doc: &Document, ui: &MotionUi, node: Option<NodeId>) {
        self.node = node;
        let time = ui.playhead_ms;
        let value = node.and_then(|n| value_of(doc, n, self.property, self.idx, time));
        // 两个独立状态：整条轨道有没有帧（→ 空心常亮）与播放头是否正停在帧上（→ 实心）。
        let has_any = node.is_some_and(|n| has_any_key(doc, n, self.property));
        let keyed = node.is_some_and(|n| has_key(doc, n, self.property, ui.playhead_ms.max(0.0) as u32));
        self.has_any = has_any;
        self.keyed = keyed;
        let text = value.map(|v| format!("{v}")).unwrap_or_default();
        let placeholder = value.is_none();
        let queue = self.queue.clone();
        let (property, idx) = (self.property, self.idx);
        self.edit.update(cx, |e, _| {
            e.set_value(text);
            if placeholder {
                e.set_placeholder(tr("Mixed"));
            }
            if let Some(n) = node {
                e.set_on_commit(move |s, _window, _cx| {
                    if let Ok(v) = s.trim().parse::<f64>() {
                        queue.push(Act::CommitValue { node: n, property, idx, value: v });
                    }
                });
            }
        });
    }
}

impl Render for KeyframeField {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens();
        let keyed = self.keyed;
        let has_any = self.has_any;
        let node = self.node;
        let property = self.property;
        let queue = self.queue.clone();
        let on_color = rgba(t.accent);
        let off_color = rgba(t.text_secondary);
        let hover = rgba(t.hover);
        let selected = rgba(t.selected_row);

        // The trailing slot is a factory (GPUI elements are consumed every frame). It builds the
        // 24 pt diamond button. Three states (用户 2026-11-12「轨道有帧就亮起、悬停到帧才实心」):
        //   · 无帧            → 空心 Instance + text_secondary，无矩形底（熄灭）
        //   · 有帧、播放头不在帧 → 空心 Instance + accent + 矩形底（常亮，轨道已有帧）
        //   · 播放头正停在帧上  → 实心 Component + accent + 矩形底（egui 原行为，保持不变）
        let trail = Slot::custom(move || {
            let icon = if keyed { Icon::Component } else { Icon::Instance };
            // 有帧就上 accent；只有完全无帧才用暗色。
            let colour = if has_any { on_color } else { off_color };
            let q = queue.clone();
            let cell = div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(pt(crate::widgets::RADIUS))
                .cursor_pointer()
                // 整条轨道有帧就给矩形底（不再只在 keyed 时给）。
                .when(has_any, |d| d.bg(selected))
                .hover(move |s| s.bg(hover))
                .on_mouse_down(MouseButton::Left, move |_, _window, _cx| {
                    if let Some(n) = node {
                        q.push(Act::ToggleKey(n, property));
                    }
                })
                .child(paint_icon(icon, colour, 12.0));
            cell.into_any_element()
        });

        // ONE layout — the `Input` template owns lead/edit/trail widths. No hand-rolled flex row,
        // no computed column widths: the template measures the slots and fills the rest with edit.
        let input = Input::new(self.width)
            .lead(self.lead.clone())
            .trail(trail)
            .trail_w(SLOT)
            .numeric(true)
            .lead_click(false);
        let frame = input.measure(window);
        let edit = self.edit.clone().into_any_element();
        let el: Div = input.paint(&frame, Parts { lead: slot_child(&self.lead), edit: Some(edit), trail: None, alpha: None, unit: None });
        el.h(pt(FIELD_H))
    }
}

/// The retained set of keyframe fields. Created once in `CanvasView::new`, like `RightFields`,
/// so typing / caret state survives between frames.
pub struct KeyframeFields {
    fields: Vec<Entity<KeyframeField>>,
    /// The last column width pushed to the fields. The `Input` template positions its lead / edit /
    /// trail slots from `width`, so a `0.0` width (the value they were built with) puts the diamond
    /// at a negative x and leaves the edit 0-wide — the fields look empty and can't be clicked.
    /// We recompute `col_width(right_w)` each frame (same formula as the Design panel) and push it.
    last_col: std::cell::Cell<f32>,
}

impl KeyframeFields {
    pub fn new(cx: &mut App, width: f32, queue: &Queue) -> Self {
        let fields = FIELDS
            .iter()
            .map(|(p, i, lead)| KeyframeField::build(cx, width, *p, *i, lead, queue.clone()))
            .collect();
        Self { fields, last_col: std::cell::Cell::new(width) }
    }

    /// Push the panel's column width into every keyframe field (mirrors `RightFields::set_col`).
    /// Called from `inspector::panel` with the Motion inspector's own width, so the X/Y/W/H inputs
    /// and their diamonds get real geometry instead of a zero-width frame.
    pub fn set_col(&self, col: f32, cx: &mut App) {
        if (col - self.last_col.get()).abs() < 0.5 {
            return;
        }
        self.last_col.set(col);
        for f in &self.fields {
            f.update(cx, |field, _| field.width = col);
        }
    }

    pub fn get(&self, property: AnimProperty, idx: usize) -> Option<Entity<KeyframeField>> {
        FIELDS
            .iter()
            .position(|(p, i, _)| *p == property && *i == idx)
            .and_then(|i| self.fields.get(i).cloned())
    }

    /// Push the document's values into every field for this frame.
    pub fn sync(&self, cx: &mut App, doc: &Document, ui: &MotionUi, node: Option<NodeId>) {
        for f in &self.fields {
            f.update(cx, |field, cx| field.sync(cx, doc, ui, node));
        }
    }

}
