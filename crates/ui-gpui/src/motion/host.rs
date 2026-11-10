// ============================================================================
// Motion ↔ CanvasView 接入层。把主视图需要的动作集中在这里，`lib.rs` 只留几行调用。
// 和 `layer_tree.rs` 一样，用 `impl CanvasView` 在子模块里访问根视图的状态。
// ============================================================================
//! Glue between the Motion state machine and the root view: drain actions each frame, advance
//! playback, build the inspector / timeline / clip window from reusable widgets.

use std::sync::Arc;

use figcraft_doc::Document;
use gpui::{AnyElement, Context, Div, IntoElement, Stateful, Window};

use crate::CanvasView;
use crate::motion;

impl CanvasView {
    /// Drain the Motion action queue, advance playback and refresh the evaluated preview.
    /// Call once at the top of `render`, next to `sync_panel`.
    pub(crate) fn sync_motion(&mut self, cx: &mut Context<Self>) {
        let mut acts = self.motion_q.drain();
        // The header's X leaves Motion mode; only the root view can resize the right panel.
        let close = acts.iter().any(|a| matches!(a, motion::Act::CloseMotion));
        acts.retain(|a| !matches!(a, motion::Act::CloseMotion));
        if motion::apply(&mut self.app, &mut self.motion_ui, acts) {
            cx.notify();
        }
        if close {
            self.set_motion_open(false, cx);
            self.motion_ui.playing = false;
        }
        let settings = self
            .app
            .session
            .ds()
            .ok()
            .map(|ds| motion::settings_of(&ds.doc, ds.page))
            .unwrap_or_default();
        if self.motion_ui.advance(settings) {
            cx.notify();
        }
        if self.motion {
            if let Ok(ds) = self.app.session.ds() {
                let doc = ds.doc.clone();
                let page = ds.page;
                self.motion_ui.refresh_preview(&doc, page);
            }
        } else {
            self.motion_ui.preview_doc = None;
            self.motion_ui.preview = None;
        }
    }

    /// The document the canvas should draw while Motion is previewing a playhead position.
    pub(crate) fn motion_preview_doc(&self) -> Option<Arc<Document>> {
        self.motion_ui.preview_doc.clone()
    }

    /// The Motion right panel (replaces the Design panel while Motion is open).
    pub(crate) fn render_motion_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let vp = window.viewport_size();
        let doc = self.app.session.doc().ok().cloned().unwrap_or_else(|| Document::new("motion"));
        let selection = self.app.session.selection();

        // egui `inspector.rs` L101-114 reuses the Design panel's Fill / Stroke / Effects rows
        // verbatim (`paints(app, ui, &doc, &sel, true/false, Some(layer))`, `effects(...)`).
        // Build the SAME `right::sections` elements from this frame's `PanelData` — not a
        // hand-written placeholder row. `FILL` is the right panel's "let the grid decide" width.
        let data = self.panel_data();
        let sel = self.panel_sel();
        self.fields.begin_rows();
        let mut n = 0usize;
        // Motion mode: the Fill / Stroke sections carry a keyframe diamond on the layer's first
        // solid paint. Compute the precomputed "keyed at playhead" state for each colour property
        // (egui passes `keyframe: Some(layer)` to `paints`); the diamond toggles a keyframe there.
        let layer = selection.first().copied().filter(|id| doc.node(*id).is_some());
        let playhead = self.motion_ui.playhead_ms.max(0.0) as u32;
        let fill_keyed = layer.is_some_and(|l| crate::motion::has_key(&doc, l, figcraft_doc::anim::motion::AnimProperty::FillColor, playhead));
        let stroke_keyed = layer.is_some_and(|l| crate::motion::has_key(&doc, l, figcraft_doc::anim::motion::AnimProperty::StrokeColor, playhead));
        let fills = crate::panels::right::sections::paints(&self.fields, &data, true, sel.has_vector, 0.0, 0.0, &mut n, window, cx, layer, fill_keyed, &self.motion_q)
            .into_element()
            .into_any_element();
        let strokes = crate::panels::right::sections::paints(&self.fields, &data, false, sel.has_vector, 0.0, 0.0, &mut n, window, cx, layer, stroke_keyed, &self.motion_q)
            .into_element()
            .into_any_element();
        let keyed_effects: Vec<figcraft_doc::anim::motion::AnimProperty> = layer
            .map(|l| {
                [
                    figcraft_doc::anim::motion::AnimProperty::ShadowBlur,
                    figcraft_doc::anim::motion::AnimProperty::BlurRadius,
                ]
                .into_iter()
                .filter(|p| crate::motion::has_key(&doc, l, *p, playhead))
                .collect()
            })
            .unwrap_or_default();
        let effects = layer
            .map(|l| {
                crate::panels::right::sections::effects(&self.fields, &data, 0.0, Some((l, keyed_effects.clone(), &self.motion_q)))
                    .into_element()
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                crate::panels::right::sections::effects(&self.fields, &data, 0.0, None)
                    .into_element()
                    .into_any_element()
            });
        let design_sections = vec![fills, strokes, effects];
        let root = cx.entity();

        motion::inspector::panel(
            cx,
            &doc,
            &mut self.motion_ui,
            &self.motion_q,
            &self.motion_fields,
            &selection,
            root,
            design_sections,
            self.right_w,
            vp,
            window,
        )
    }

    /// The bottom timeline (replaces `panels::animation_bar()`).
    pub(crate) fn render_motion_timeline(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let _ = cx;
        let vp = window.viewport_size();
        let width = f32::from(vp.width);
        let doc = self.app.session.doc().ok().cloned().unwrap_or_else(|| Document::new("motion"));
        let page = self.app.session.ds().ok().map(|d| d.page).unwrap_or(figcraft_doc::NodeId(0));
        let selection = self.app.session.selection();
        motion::timeline::bar(
            cx,
            &doc,
            &mut self.motion_ui,
            &self.motion_q,
            &selection,
            &self.motion_times,
            page,
            width,
            window,
        )
    }

    /// The clip settings floating window (a `Popover`), or nothing when closed.
    pub(crate) fn motion_clip_popover_layer(&mut self, cx: &mut Context<Self>, vp: gpui::Size<gpui::Pixels>) -> Vec<AnyElement> {
        let doc = self.app.session.doc().ok().cloned().unwrap_or_else(|| Document::new("motion"));
        let root = cx.entity();
        motion::clip_popover::panel(cx, &doc, &self.motion_ui, &self.motion_q, &mut self.motion_clip_fields, vp, root)
    }

    /// Open / close Motion mode: widen the right panel to the inspector width and back.
    pub(crate) fn set_motion_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let current = self.motion;
        self.motion = motion::set_open(&mut self.motion_ui, &mut self.right_w, &mut self.motion_right_w, open, current);
        cx.notify();
    }

    /// The docked timeline's height for this frame.
    ///
    /// A board has no animation, so the timeline never takes the dock's height there — even if
    /// `motion` was left open on a design file before switching (用户 2026-11-10).
    pub(crate) fn motion_bar_h(&self) -> f32 {
        if self.motion && !self.is_board_doc() {
            if self.motion_ui.timeline_h > 0.0 {
                // 上限交给拖拽时按视口夹紧（`timeline::route_drag_move` 的 Height 分支），
                // 这里不能再卡在默认高度上，否则「往上拖变高」永远看不出效果。
                self.motion_ui.timeline_h.max(motion::MIN_H)
            } else {
                motion::HEIGHT
            }
        } else {
            0.0
        }
    }

}
