// ============================================================================
// Motion ↔ CanvasView 接入层。把主视图需要的动作集中在这里，`lib.rs` 只留几行调用。
// 和 `layer_tree.rs` 一样，用 `impl CanvasView` 在子模块里访问根视图的状态。
// ============================================================================
//! Glue between the Motion state machine and the root view: drain actions each frame, advance
//! playback, build the inspector / timeline / clip window from reusable widgets.


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
        let selection = self.app.session.selection();
        let settings = self
            .app
            .session
            .ds()
            .ok()
            .map(|ds| motion::settings_of(&ds.doc, motion::scope_of(&ds.doc, &selection, ds.page)))
            .unwrap_or_default();
        if self.motion_ui.advance(settings) {
            cx.notify();
        }
        if self.motion {
            if let Ok(ds) = self.app.session.ds() {
                let doc = ds.doc.clone();
                // Evaluate only the selected frame's subtree so all its components play together
                // on one timeline; page scope (nothing in a frame selected) stays unchanged.
                let scope = motion::scope_of(&doc, &selection, ds.page);
                self.motion_ui.refresh_preview(&doc, scope);
            }
        } else {
            self.motion_ui.preview_doc = None;
            self.motion_ui.preview = None;
        }
        // 画布上的运动路径 + 变换手柄：Motion 模式里只要有 `Position` 动画就画，
        // 播放过程中藏起来（用户 2026-11-12）——播放时看的是成品，不停留 Controls。
        // 静态源文档必须在"临时替换 ds.doc 为 preview"之前取：sync_motion 在 render 开头跑，
        // 此刻 ds.doc 还是未求值的真实文档。轨迹 / 角度基准 / 命中都靠它保持同一坐标系。
        let src_doc = self.app.session.ds().ok().map(|ds| ds.doc.clone());
        self.app.motion_overlay = crate::canvas::motion_overlay::State::new(
            self.motion,
            self.motion_ui.playing,
            self.motion_ui.playhead_ms,
            settings.fps,
            settings.duration_ms,
            self.motion_ui.preview_doc.clone(),
            src_doc,
        );
        // 让引擎的命中/悬停按「画面上显示的那份」算：动画模式播放头停在中间时，物体画在
        // 动画位置，命中若仍读未求值的 ds.doc 就点不中（用户 2026-11-12）。
        // 播放态不设置：播放时预览文档每帧重建，若把它交给命中/拖拽/缩放/旋转工具，
        // 手势起点与后续帧的几何基准会一直变，表现为「图形乱跑」。
        // 写操作不受影响——引擎的 `display_doc` 只用于命中几何，编辑仍走 `ds.doc`。
        self.app.session.display_doc = if self.motion && !self.motion_ui.playing {
            self.motion_ui.preview_doc.clone()
        } else {
            None
        };
        // 画布这一帧显示的文档：渲染请求与叠加层都读它，ds.doc 不再被临时替换。
        self.app.display_doc = self.motion_ui.preview_doc.clone();
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
        // ── 组件专属段（用户 2026-11-12）──
        // Motion 右栏不能只有一套通用面板：选中什么组件，就长什么面板。这里与设计面板用同一批
        // section（文本 → Typography、画框 → Auto layout / Layout grid、实例 → Component、
        // 椭圆 → Arc、星形 → Star…），只是可动画字段仍走关键帧控件。
        let mut before: Vec<AnyElement> = Vec::new();
        if let Some(c) = crate::panels::right::component::section(&self.fields, &data, cx, &mut n) {
            before.push(c.into_any_element());
        }
        if let Some(s) = crate::panels::right::sections::section(&self.fields, &data, 0.0) {
            before.push(s.into_any_element());
        }
        // Auto layout 的附加行（方向 / 对齐 / 间距 / 内边距 / 网格轨道）。Motion 的
        // Layout 段已经用关键帧字段画了 W/H，这里只要「自动布局」那部分，所以不重复尺寸行。
        if sel.has_al {
            before.push(crate::panels::auto_layout_section(&self.fields, sel, &data, cx, &mut n, false, false));
        }
        if sel.all_ellipse {
            before.push(crate::panels::arc_section(&self.fields).into_any_element());
        }
        if sel.all_points {
            before.push(crate::panels::star_section(&self.fields).into_any_element());
        }
        if let Some(m) = crate::panels::right::sections::variable_modes(&self.fields, &data, 0.0) {
            before.push(m.into_any_element());
        }
        if sel.has_text {
            before.push(crate::panels::right::sections::typography(&self.fields, &data).into_any_element());
        }
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
        let keyframe = layer.map(|l| (l, keyed_effects.clone(), &self.motion_q));
        let effects = crate::panels::right::sections::effects(&self.fields, &data, 0.0, cx, keyframe)
            .into_element()
            .into_any_element();
        // Fill / Stroke / Effects 之后的组件专属段：Layout grid（画框）+ Export（所有图层）。
        let mut after: Vec<AnyElement> = Vec::new();
        if sel.all_frame {
            after.push(crate::panels::right::sections::grids(&self.fields, &data, 0.0).into_element().into_any_element());
        }
        after.push(crate::panels::right::sections::export(&self.fields, &data, 0.0, &mut n, cx).into_element().into_any_element());
        let mut design_sections = before;
        design_sections.extend([fills, strokes, effects]);
        design_sections.extend(after);
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
        let selection = self.app.session.selection();
        // The bottom timeline belongs to the selected frame (one animation group), not the whole
        // page. `bar`'s `page` parameter is now this scope; build_rows still lists `selection`.
        let scope = motion::scope_of(&doc, &selection, self.app.session.ds().ok().map(|d| d.page).unwrap_or(figcraft_doc::NodeId(0)));
        motion::timeline::bar(
            cx,
            &doc,
            &mut self.motion_ui,
            &self.motion_q,
            &selection,
            &self.motion_times,
            scope,
            width,
            window,
        )
    }

    /// The clip settings floating window (a `Popover`), or nothing when closed.
    pub(crate) fn motion_clip_popover_layer(&mut self, cx: &mut Context<Self>, vp: gpui::Size<gpui::Pixels>) -> Vec<AnyElement> {
        let doc = self.app.session.doc().ok().cloned().unwrap_or_else(|| Document::new("motion"));
        let root = cx.entity();
        // `right_w` 在这里直接取（self 就是 CanvasView）。绝不能在 `panel()` 里 `root.read(cx)`：
        // 那是渲染期，CanvasView 正被 update，再读自己会 panic
        // `cannot read CanvasView while it is already being updated`（entity_map.rs:138）。
        let right_w = self.right_w;
        motion::clip_popover::panel(cx, &doc, &self.motion_ui, &self.motion_q, &mut self.motion_clip_fields, vp, root, right_w)
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
