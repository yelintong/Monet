// ============================================================================
// Motion inspector（右栏）——`ui-egui/src/motion/inspector.rs` 的 GPUI 移植。
//
// ⛔ 统一网格：每一行都只用 `field_row` / `field_row_full` / `sublabel_row` 构造，交给
//    `PanelSection::child`。本文件**没有**任何 `div().flex().flex_row()` 手排的行，也不手算列宽。
//    列宽由 `widgets/panel_section.rs` 独占决定（`1fr | 1fr | 24`，只有一列 24）。
// 窗口：clip 的设置浮窗在 `clip_popover.rs`，复用 `Popover` 窗口外壳。
// ============================================================================
//! The right panel while Motion is open. Sections follow the Design panel: Animation,
//! Transform, Layout, Appearance — every animatable value is a [`KeyframeField`] (one
//! `Input` template whose trailing slot carries the add/remove-keyframe diamond).

use figcraft_i18n::tr;
use figcraft_doc::anim::motion::ClipKind;
use figcraft_doc::anim::motion::PresetGroup;
use figcraft_doc::{Document, NodeId};
use gpui::{AnyElement, App, Div, MouseButton, SharedString, Stateful, Window, div, prelude::*, px};

use crate::motion::keyframe_field::{FIELDS, KeyframeField, KeyframeFields};
use crate::motion::{Act, MotionUi, Queue};
use crate::widgets::icon::Icon;
use crate::widgets::icon_button::IconButton;
use crate::widgets::menu::MenuRow;
use crate::widgets::panel_section::{field_row, field_row_full, panel_section, sublabel_row, sublabel_row2};
use crate::widgets::{pt, rgba, tokens};

/// Empty trailing column of a grid row (`field_row`'s generics need a concrete element type).
fn no_trailing() -> Vec<AnyElement> {
    Vec::new()
}

/// Build the Motion right panel.
pub fn panel(
    cx: &mut App,
    doc: &Document,
    ui: &mut MotionUi,
    queue: &Queue,
    fields: &KeyframeFields,
    selection: &[NodeId],
    // The root view, for the "+" menu (the same `open_menu` the Design panel uses).
    root: gpui::Entity<crate::CanvasView>,
    // The Design panel's Fill / Stroke / Effects sections, built by the host from `PanelData`.
    design_sections: Vec<AnyElement>,
    panel_w: f32,
    _vp: gpui::Size<gpui::Pixels>,
    _window: &mut Window,
) -> Stateful<Div> {
    let t = tokens();
    // The keyframe fields' `Input` template positions its slots from `width`; without this they
    // stay at the 0.0 they were built with (empty, unclickable). Same column formula as the
    // Design panel's `set_col`, computed from the Motion inspector's own width.
    let col = ((panel_w - crate::widgets::PAD - crate::widgets::PAD_RIGHT - crate::widgets::GRID_GAP * 2.0 - crate::widgets::SLOT) / 2.0).max(40.0);
    fields.set_col(col, cx);
    fields.sync(cx, doc, ui, selected_layer(doc, ui, selection));

    let mut body = div().id("motion-inspector-body").flex_1().min_h(px(0.0)).w_full().flex().flex_col().overflow_y_scroll();
    let wrap = |body: Stateful<Div>| -> Stateful<Div> {
        div().id("motion-inspector").flex_none().h_full().w_full().flex().flex_col().bg(rgba(t.panel)).child(body)
    };

    if ui.easing_target {
        body = body.child(super::easing::panel(cx, doc, ui, queue, _window));
        return wrap(body);
    }

    let Some(layer) = selected_layer(doc, ui, selection) else {
        body = body.child(
            div().px(pt(16.0)).py(pt(12.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Select a layer")),
        );
        return wrap(body);
    };

    // ── Layer name header (40 pt) ──────────────────────────────────────────────
    let name = doc.node(layer).map(|n| n.name.clone()).unwrap_or_default();
    body = body.child(
        div()
            .h(pt(40.0))
            .w_full()
            .flex()
            .items_center()
            .pl(pt(16.0))
            .text_size(pt(13.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.text))
            .child(SharedString::from(name)),
    );

    // ── Animation section: presets toggle + "+" menu ──────────────────────────
    // egui L62-89: the "+" opens a menu of the FIVE `ClipKind`s (the library button beside it
    // holds the 41 presets). Previously this button silently added a Position clip — wrong.
    let q = queue.clone();
    let presets_open = ui.presets_open;
    let plus_q = queue.clone();
    let plus_root = root.clone();
    let plus = IconButton::new(Icon::Plus).on_click(move |_e, window, cx| {
        let anchor = gpui::Bounds { origin: window.mouse_position(), size: gpui::size(px(1.0), px(1.0)) };
        let rows: Vec<MenuRow> = ClipKind::ALL.iter().map(|k| MenuRow::new(k.name())).collect();
        let q = plus_q.clone();
        let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
            if let Some(kind) = ClipKind::ALL.iter().find(|k| k.name() == label) {
                q.push(Act::AddClip(layer, *kind));
            }
        });
        plus_root.update(cx, |cv, cx| cv.open_menu(anchor, None, rows, on_pick, cx));
    });
    let anim_section = panel_section("Animation")
        .header_trail(
            div()
                .flex()
                .flex_row()
                .gap(pt(4.0))
                .child(IconButton::new(Icon::ComponentSet).active(presets_open).on_click(move |_, _, _| q.push(Act::SetPresetsOpen(!presets_open))))
                .child(plus),
        )
        .child(clips_rows(doc, ui, queue, layer));

    body = body.child(anim_section);
    if presets_open {
        body = body.child(presets_rows(queue, layer));
    }
    body = body.child(separator());

    // ── Transform ─────────────────────────────────────────────────────────────
    body = body.child(
        panel_section("Transform")
            .child(sublabel_row(tr("Position")))
            .child(field_row(vec![field(fields, 0).into_any_element(), field(fields, 1).into_any_element()], no_trailing()))
            .child(sublabel_row(tr("Scale")))
            .child(field_row(vec![field(fields, 2).into_any_element(), field(fields, 3).into_any_element()], no_trailing()))
            .child(sublabel_row(tr("Rotation")))
            .child(field_row(vec![field(fields, 4).into_any_element()], no_trailing())),
    );
    body = body.child(separator());

    // ── Layout: W | H + proportions lock as the trailing 24 pt control ─────────
    let locked = doc.node(layer).is_some_and(|n| n.constrain_proportions);
    body = body.child(
        panel_section("Layout")
            .child(sublabel_row2("Width", "Height"))
            .child(field_row(
                vec![field(fields, 5).into_any_element(), field(fields, 6).into_any_element()],
                vec![IconButton::new(if locked { Icon::LinkOn } else { Icon::LinkOff }).active(locked).into_any_element()],
            )),
    );
    body = body.child(separator());

    // ── Appearance ────────────────────────────────────────────────────────────
    body = body.child(
        panel_section("Appearance")
            .child(sublabel_row2("Opacity", "Corner radius"))
            .child(field_row(vec![field(fields, 7).into_any_element(), field(fields, 8).into_any_element()], no_trailing())),
    );
    body = body.child(separator());

    // ── Fill / Stroke / Effects ────────────────────────────────────────────────
    // egui `inspector.rs` L101-114: "Fill, Stroke and Effects are the Design panel's own
    // rows … so there is nothing to re-implement here". It calls `paints(...)` /
    // `effects(...)` directly. These are the SAME `right::sections` elements, precomputed
    // by the host from `PanelData`; no hand-made "Mixed" placeholder rows.
    for (i, sec) in design_sections.into_iter().enumerate() {
        body = body.child(separator());
        body = body.child(div().w_full().child(sec));
        let _ = i;
    }

    wrap(body)
}

fn field(fields: &KeyframeFields, index: usize) -> gpui::Entity<KeyframeField> {
    let (p, i, _) = FIELDS[index];
    fields.get(p, i).unwrap_or_else(|| fields.get(FIELDS[0].0, FIELDS[0].1).expect("KeyframeFields is built from FIELDS"))
}

fn separator() -> Div {
    div().flex_none().w_full().h(pt(1.0)).bg(rgba(tokens().border))
}

/// egui `inspector::selected_layer`: the FIRST node of the canvas selection, not "some node
/// that happens to have animation". Previously this fell back to `animated_nodes`, so a plain
/// document (no animation yet) rendered an empty white panel — exactly the state in which the
/// user needs the inspector most (to ADD the first keyframe / clip).
/// The timeline's own selection wins when it points at a layer; otherwise use the canvas one.
fn selected_layer(doc: &Document, ui: &MotionUi, selection: &[NodeId]) -> Option<NodeId> {
    let from_canvas = selection.first().copied().filter(|id| doc.node(*id).is_some());
    let from_timeline = ui.selection.iter().next().map(|s| s.node()).filter(|id| doc.node(*id).is_some());
    from_canvas.or(from_timeline)
}

/// The clips list, all as full-width grid rows.
fn clips_rows(doc: &Document, ui: &MotionUi, queue: &Queue, layer: NodeId) -> crate::widgets::panel_section::FieldRow {
    let t = tokens();
    let clips: Vec<_> = doc.node(layer).map(|n| n.motion.clips.clone()).unwrap_or_default();
    let mut col = div().flex().flex_col().w_full();
    if clips.is_empty() {
        col = col.child(div().py(pt(4.0)).pl(pt(16.0)).text_size(pt(9.0)).text_color(rgba(t.text_tertiary)).child(tr("None")));
    }
    for c in clips {
        let selected = ui.selected_clip == Some((layer, c.id));
        let q = queue.clone();
        let q2 = queue.clone();
        let q3 = queue.clone();
        let label = format!("{} · {}", c.kind.name(), c.mode.label(c.kind));
        let row = div()
            .h(pt(24.0))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .rounded(pt(5.0))
            .pl(pt(8.0))
            .bg(rgba(if selected { t.selected_row } else { t.field }))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                q.push(Act::SelectClip(layer, c.id));
                q.push(Act::OpenClipPopover(layer, c.id));
            })
            .child(div().flex_1().min_w(px(0.0)).text_size(pt(11.0)).text_color(rgba(t.text)).child(SharedString::from(label)))
            .child(IconButton::new(if c.visible { Icon::Eye } else { Icon::EyeOff }).size(20.0).on_click(move |_, _, _| q2.push(Act::ToggleClipVisible(layer, c.id))))
            .child(IconButton::new(Icon::Minus).size(20.0).on_click(move |_, _, _| q3.push(Act::DeleteClip(layer, c.id))));
        col = col.child(row);
    }
    field_row_full(col)
}

fn presets_rows(queue: &Queue, layer: NodeId) -> Div {
    let t = tokens();
    let mut col = div().flex().flex_col().w_full().px(pt(16.0)).pb(pt(12.0));
    for group in PresetGroup::ALL {
        col = col.child(div().pt(pt(4.0)).pb(pt(4.0)).text_size(pt(9.0)).text_color(rgba(t.text_tertiary)).child(group.name()));
        for preset in figcraft_doc::anim::motion::presets_in(group) {
            let q = queue.clone();
            let name = preset.name.to_string();
            col = col.child(
                div()
                    .h(pt(24.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .rounded(pt(5.0))
                    .pl(pt(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgba(t.hover)))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| q.push(Act::AddPreset(layer, name.clone())))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(preset.name)),
            );
        }
    }
    col
}
