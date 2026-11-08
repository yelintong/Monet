// ============================================================================
// Clip 设置浮窗 —— `ui-egui/src/motion/clip_popover.rs` 的 GPUI 移植。
//
// 窗口外壳**直接用** `widgets::popover::Popover`（240/264 宽、40 pt header、标题插槽、
// X、13 圆角、阴影、拖标题栏、贴右栏左缘），本文件只填 title / body 两个插槽。
// body 每行仍然只用 `field_row` / `field_row_full` 交给 `PanelSection::child`。
// ============================================================================
//! The clip's floating settings window over the canvas. It is a `Popover` with a `title_text`
//! header and a `PanelSection` body; the view owns open/close and the dragged position.

use std::cell::RefCell;
use std::rc::Rc;

use figcraft_doc::anim::motion::{Clip, ClipKind, ClipMode, Ease};
use figcraft_doc::{Document, NodeId};
use gpui::{AnyElement, App, Entity, Point, SharedString, Size, Window, px};

use crate::motion::{Act, MotionUi, Queue};
use crate::widgets::dropdown::dropdown;
use crate::widgets::input::Slot;
use crate::widgets::input::number_field::number_field;
use crate::widgets::input::text_field::TextField;
use crate::widgets::menu::MenuRow;
use crate::widgets::panel_section::{field_row, field_row_full, panel_section, sublabel_row};
use crate::widgets::popover::{Open as PopoverOpen, popover_window, title_text};

pub const WIDTH: f32 = 264.0;

/// Seven retained numeric fields, paired on the unified grid (Delay | Duration,
/// Amount | From X, From Y | To X, To Y | blank).
pub struct ClipFields {
    fields: Vec<Entity<TextField>>,
    /// The clip as of this frame; a commit clones it, changes one value and pushes `SetClip`.
    draft: Rc<RefCell<Option<Clip>>>,
    node: Option<NodeId>,
    id: u32,
}

const NUM_LABELS: [&str; 7] = ["Delay", "Duration", "Amount", "From X", "From Y", "To X", "To Y"];

/// egui `amount_unit` / `amount_label`：不同动画类型的幅度单位（° / % / 距离）。
fn amount_unit(kind: ClipKind) -> (f32, &'static str) {
    match kind {
        ClipKind::Position => (1.0, ""),
        ClipKind::Rotation => (1.0, "°"),
        _ => (100.0, "%"),
    }
}

fn amount_label(kind: ClipKind) -> &'static str {
    match kind {
        ClipKind::Position => "Distance",
        ClipKind::Rotation => "Angle",
        ClipKind::Opacity => "Opacity",
        ClipKind::Scale => "Scale",
        ClipKind::Size => "Size",
    }
}

/// 当前 clip 下 7 个数值框的标签、可见性和显示倍率。
/// egui 的 Custom 模式才显示 From/To，且只有 Position/Scale/Size 是 vec2（X+Y 两列）。
fn labels_for(c: &Clip) -> ([String; 7], [bool; 7], f32) {
    let (f, suffix) = amount_unit(c.kind);
    let vec2 = matches!(c.kind, ClipKind::Position | ClipKind::Scale | ClipKind::Size);
    let custom = c.mode == ClipMode::Custom;
    let mut labels: [String; 7] = NUM_LABELS.map(str::to_string);
    labels[2] = format!("{}{}", amount_label(c.kind), if suffix.is_empty() { String::new() } else { format!(" {suffix}") });
    let mut visible = [false; 7];
    visible[0] = true; // Delay
    visible[1] = true; // Duration
    visible[2] = true; // Amount
    if custom {
        visible[3] = true; // From X
        visible[4] = vec2; // From Y
        visible[5] = true; // To X
        visible[6] = vec2; // To Y
    }
    (labels, visible, f)
}

impl ClipFields {
    pub fn new(cx: &mut App, queue: &Queue) -> Self {
        let draft = Rc::new(RefCell::new(None::<Clip>));
        let mut fields = Vec::with_capacity(NUM_LABELS.len());
        for (i, label) in NUM_LABELS.iter().enumerate() {
            let d = draft.clone();
            let q = queue.clone();
            let field = number_field(cx, 0.0, None, "", 1.0, Slot::Text(SharedString::from(label.to_string())));
            field.update(cx, |tf, _| {
                tf.set_on_commit(move |s, _window, _cx| {
                    let Ok(v) = s.trim().parse::<f64>() else { return };
                    let Some(mut c) = d.borrow().as_ref().cloned() else { return };
                    // 显示值 = 模型值 * factor（egui amount_unit），提交时反算回去。
                    let f = CLIP_FACTOR.with(|c| c.get()) as f64;
                    let val = v / f;
                    match i {
                        0 => c.delay_ms = v.max(0.0) as u32,
                        1 => c.duration_ms = v.max(1.0) as u32,
                        2 => c.amount = val as f32,
                        3 => c.from.x = val,
                        4 => c.from.y = val,
                        5 => c.to.x = val,
                        _ => c.to.y = val,
                    }
                    if let Some((node, id)) = CLIP_TARGET.with(|c| *c.borrow()) {
                        q.push(Act::SetClip(node, id, Box::new(c)));
                    }
                });
            });
            fields.push(field);
        }
        Self { fields, draft, node: None, id: 0 }
    }

    /// Refresh every field from the currently open clip.
    pub fn sync(&mut self, cx: &mut App, doc: &Document, ui: &MotionUi) {
        let Some((node, id)) = ui.clip_popover else {
            *self.draft.borrow_mut() = None;
            return;
        };
        self.node = Some(node);
        self.id = id;
        CLIP_TARGET.with(|c| *c.borrow_mut() = Some((node, id)));
        let Some(c) = doc.node(node).and_then(|n| n.motion.clips.iter().find(|c| c.id == id)).cloned() else {
            *self.draft.borrow_mut() = None;
            return;
        };
        *self.draft.borrow_mut() = Some(c.clone());
        let (labels, _visible, factor) = labels_for(&c);
        CLIP_FACTOR.with(|f| f.set(factor));
        let values = [
            c.delay_ms as f64,
            c.duration_ms as f64,
            c.amount as f64 * factor as f64,
            c.from.x * factor as f64,
            c.from.y * factor as f64,
            c.to.x * factor as f64,
            c.to.y * factor as f64,
        ];
        for ((f, v), label) in self.fields.iter().zip(values).zip(labels) {
            let label = label.clone();
            f.update(cx, |tf, _| {
                tf.set_lead(Slot::Text(SharedString::from(label)));
                tf.set_value(format!("{v}"));
            });
        }
    }

    fn field(&self, i: usize) -> Entity<TextField> {
        self.fields[i].clone()
    }
}

thread_local! {
    /// 当前 clip 的显示倍率（`amount_unit`）；'static 提交闭包读它。
    static CLIP_FACTOR: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
    /// The `(node, clip id)` the number fields commit to. The fields are retained entities whose
    /// callbacks are `'static`, so they cannot hold the frame's `(node, id)` directly.
    static CLIP_TARGET: RefCell<Option<(NodeId, u32)>> = const { RefCell::new(None) };
}

/// `field_row`'s empty `Vec<impl IntoElement>` needs a concrete element type.
fn none() -> Vec<AnyElement> {
    Vec::new()
}

/// Render the settings window. Returns the card element (already positioned) for the root view.
pub fn panel(
    cx: &mut App,
    doc: &Document,
    ui: &MotionUi,
    queue: &Queue,
    fields: &mut ClipFields,
    vp: Size<gpui::Pixels>,
    root: gpui::Entity<crate::CanvasView>,
) -> Vec<AnyElement> {
    let Some((node, id)) = ui.clip_popover else { return Vec::new() };
    fields.sync(cx, doc, ui);
    let Some(c) = doc.node(node).and_then(|n| n.motion.clips.iter().find(|c| c.id == id)).cloned() else {
        return Vec::new()
    };
    let q = queue.clone();

    // ── Enum rows: each opens a MENU of its options on click (egui `menu_popup` + `menu_check`),
    //    not a click-to-cycle. The chosen value is applied via `Act::SetClip`, same as egui. ──────
    let kind_row = dropdown(SharedString::from(c.kind.name()), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let menu: Vec<MenuRow> = ClipKind::ALL.iter().map(|k| MenuRow::new(k.name())).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(k) = ClipKind::ALL.iter().find(|k| k.name() == label) {
                        let mut next = c.clone();
                        next.kind = *k;
                        next.direction = next.kind.default_direction();
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), menu, on_pick, cx));
        }
    });
    let mode_row = dropdown(SharedString::from(c.mode.label(c.kind)), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        let kind = c.kind;
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let modes = [ClipMode::In, ClipMode::Out, ClipMode::Custom];
            let menu: Vec<MenuRow> = modes.iter().map(|m| MenuRow::new(m.label(kind))).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(m) = modes.iter().find(|m| m.label(kind).as_str() == label) {
                        let mut next = c.clone();
                        next.mode = *m;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(160.0), menu, on_pick, cx));
        }
    });
    let dir_row = dropdown(SharedString::from(c.direction.label(c.mode)), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        let mode = c.mode;
        let dirs = c.kind.directions().to_vec();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let dirs = dirs.clone();
            let menu: Vec<MenuRow> = dirs.iter().map(|dd| MenuRow::new(dd.label(mode))).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(dd) = dirs.iter().find(|dd| dd.label(mode).as_str() == label) {
                        let mut next = c.clone();
                        next.direction = *dd;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(160.0), menu, on_pick, cx));
        }
    });
    let ease_row = dropdown(SharedString::from(c.easing.name()), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let menu: Vec<MenuRow> = Ease::PRESETS.iter().map(|(name, _)| MenuRow::new(*name)).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some((_, e)) = Ease::PRESETS.iter().find(|(name, _)| *name == label) {
                        let mut next = c.clone();
                        next.easing = *e;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), menu, on_pick, cx));
        }
    });

    let (_labels, visible, _factor) = labels_for(&c);
    let mut sec = panel_section("Clip")
        .child(crate::widgets::panel_section::sublabel_row2("Animation", "Type"))
        .child(field_row(vec![kind_row, mode_row], none()))
        .child(crate::widgets::panel_section::sublabel_row2("Direction", "Easing"))
        .child(field_row(vec![dir_row, ease_row], none()));
    // 数值行按可见性两两配对（Custom 才显示 From/To，vec2 类型才有 Y）。
    let shown: Vec<usize> = (0..7).filter(|i| visible[*i]).collect();
    for pair in shown.chunks(2) {
        let row: Vec<_> = pair.iter().map(|i| fields.field(*i)).collect();
        sec = sec.child(field_row(row, none()));
    }
    // egui clip_popover.rs:281 内嵌同一个 Curve|Spring easing 编辑器。
    sec = sec.child(field_row_full(
        crate::motion::easing::editor(cx, c.easing, crate::motion::EaseTarget::Clip(node, id), queue, "clip", false),
    ));

    // Visible toggle, as the row's single 24 pt trailing control. The next value is computed
    // outside the `Fn` callback so no captured state has to be mutated.
    let q = queue.clone();
    let mut toggled = c.clone();
    toggled.visible = !toggled.visible;
    sec = sec.child(field_row(none(), vec![crate::widgets::icon_button::IconButton::new(if c.visible {
        crate::widgets::icon::Icon::Eye
    } else {
        crate::widgets::icon::Icon::EyeOff
    })
    .active(c.visible)
    .on_click(move |_, _, _| q.push(Act::SetClip(node, id, Box::new(toggled.clone()))))]));

    let title = format!("{} · {}", c.kind.name(), c.mode.label(c.kind));
    let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |_, _| q.push(Act::CloseClipPopover)
    });
    let drag: Rc<dyn Fn(&gpui::MouseDownEvent, &mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |e, _, _| q.push(Act::MoveClipPopover(e.position))
    });

    let anchor = ui.clip_anchor.map(|b| b.origin).unwrap_or(Point { x: px(0.0), y: px(100.0) });
    let open = if let Some(p) = ui.clip_at {
        PopoverOpen::At(p)
    } else {
        PopoverOpen::Anchor(Point { x: px(f32::from(anchor.x)), y: px(f32::from(anchor.y)) })
    };
    let pop = popover_window(open)
        .width(WIDTH)
        .pad(12.0)
        .height(420.0)
        .title(title_text(title))
        .body(sec)
        .on_drag_start(move |e, w, cx| drag(e, w, cx));
    let (_origin, card) = pop.show(vp, dismiss);
    vec![card]
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_kind_and_custom_mode() {
        let mut c = Clip::new(ClipKind::Opacity, ClipMode::In);
        let (labels, visible, factor) = labels_for(&c);
        assert_eq!(factor, 100.0, "透明度用百分数");
        assert_eq!(labels[2], "Opacity %");
        assert_eq!(visible, [true, true, true, false, false, false, false], "In 模式不显示 From/To");

        c.kind = ClipKind::Position;
        c.mode = ClipMode::Custom;
        let (labels, visible, factor) = labels_for(&c);
        assert_eq!(factor, 1.0, "位移用 px");
        assert_eq!(labels[2], "Distance");
        assert_eq!(visible, [true, true, true, true, true, true, true], "Custom + vec2 显示 X/Y");

        c.kind = ClipKind::Rotation;
        let (labels, visible, _f) = labels_for(&c);
        assert_eq!(labels[2], "Angle °");
        assert_eq!(visible[4], false, "旋转不是 vec2，没有 From Y");
        assert_eq!(visible[6], false);
    }
}
