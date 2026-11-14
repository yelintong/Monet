//! Layer tree: the layer list in the left panel — expand/collapse, selection, visibility, lock and reordering.
//!
//! This module is only a **consumer** of the generic drag kit in [`crate::drag`]: it turns the
//! visible rows into `drag::Row`s (ancestor-id path, can-take-children, collapsed, window rect),
//! feeds the global pointer to `Drag::update` every frame, and translates the dropped `Target`
//! into one `node.reparent`. Drop geometry, level changes, cycle prevention, the insertion line,
//!
//! the drag label and hover-to-expand all live in the kit. GPUI cannot map an element id back to
//! its layout rect, and `on_drag_move` only reports the bounds of the row it hit; `calibrate`
//! rebuilds the column from that row plus a fixed pitch, so the root-level pointer keeps resolving.

use figcraft_i18n::tr;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Instant;

use figcraft_doc::{BoolOp, Document, NodeId, NodeKind};
use figcraft_engine::booleans::distinct_op;
use figcraft_engine::components::is_slot;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, Div, ElementId, Entity, Modifiers, MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Size,
    Stateful, StatefulInteractiveElement, Window, div, px,
};
use serde_json::json;

use crate::CanvasView;
use crate::drag;
use crate::panels::right::virtual_list::{VirtualList, VirtualListScrollHandle, v_virtual_list};
use crate::widgets::icon::paint as paint_icon;
use crate::widgets::{Icon, IconButton, SectionHeader, SectionHeaderVariant, pt, rgba, tokens};

/// Ordinary ink: a theme token (the light theme's `icon` is 90% black), not a hand-made rgba.
/// Component purple, icon amber, selection and the hidden grey keep their own colours.
fn ink() -> gpui::Rgba {
    rgba(tokens().icon)
}

/// Each nesting level indents by 24 pt (the agreed spec; the reference used 16).
const LAYER_INDENT: f32 = 24.0;
/// The row's inner rect is inset by 8 pt on each side.
const ROW_INSET: f32 = 8.0;
/// Row pitch 32 = a 24 pt block plus 8 pt of gap. A row's block is 24 pt tall and the gap
/// between two separate blocks is 8 pt; blocks never merge into one strip.
const ROW_H: f32 = 32.0;
/// The icon **cell** is 24×24; the glyph inside it stays 16, so the icon does not fill the cell.
const ICON_BOX: f32 = 24.0;
const ICON_GLYPH: f32 = 16.0;
/// The block leaves 4 pt above and below, so it is ROW_H - 8 = 24 tall and neighbours sit 8 apart.
const BLOCK_PAD: f32 = 4.0;

/// Drag payload: the node id of the row being dragged (GPUI drags require `T: 'static`).
struct LayerDrag(pub NodeId);

/// The ghost view GPUI needs while dragging. The visible label is drawn by the kit's
/// `drag::overlay` at the root, so this stays empty to avoid drawing it twice.
struct LayerDragGhost;
impl Render for LayerDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// One visible row of the flattened tree.
#[derive(Clone)]
pub struct LayerRow {
    pub id: NodeId,
    /// Visual depth; 0 is the top level.
    pub depth: usize,
    pub name: String,
    pub icon: Icon,
    /// The row glyph, or `None` to fall back to the generic icon painter.
    pub glyph: Option<crate::widgets::layer_icons::LayerIcon>,
    /// `child_layout.absolute` inside an auto-layout parent: an extra Absolute glyph is layered on.
    pub absolute: bool,
    pub has_children: bool,
    pub can_into: bool,
    pub expanded: bool,
    pub selected: bool,
    /// 0 plain / 1 component family / 2 icon (drives the row's colour).
    pub mark_kind: u8,
    pub visible: bool,
    pub locked: bool,
}

/// Selection marks: the selected row, its descendants, and everything else.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    None,
    Child,
    Selected,
}

/// Selected / descendant / none, which decides each row's band colour.
/// One contiguous run (`Mark != None`) is a **single block**: the light fill joins up, the
/// selected row takes the deeper band, and only the run's first and last rows inset by 4 pt, so
fn marks(rows: &[LayerRow]) -> Vec<Mark> {
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        while open.last().is_some_and(|d| *d >= r.depth) {
            open.pop();
        }
        if r.selected {
            open.push(r.depth);
            out.push(Mark::Selected);
        } else if open.is_empty() {
            out.push(Mark::None);
        } else {
            out.push(Mark::Child);
        }
    }
    out
}

/// The run boundaries: `first[i]` / `last[i]` are the first and last row of the run containing i.
/// Row drawing uses them to decide whether this row rounds and insets at the top / bottom; rows
/// inside a run do not inset, so the whole run reads as one block.
fn selection_runs(m: &[Mark]) -> (Vec<usize>, Vec<usize>) {
    let n = m.len();
    let mut first = vec![0usize; n];
    let mut last = vec![0usize; n];
    let mut i = 0;
    while i < n {
        if m[i] != Mark::None {
            let start = i;
            while i < n && m[i] != Mark::None {
                i += 1;
            }
            let end = i - 1;
            for j in start..=end {
                first[j] = start;
                last[j] = end;
            }
        } else {
            i += 1;
        }
    }
    (first, last)
}

/// An all-icon band takes `icon_layer_row`, an all-component band `component_row`, else
/// `selected_row`. The range is this band (`start..=end`), not the whole run, so in a
/// "selected parent + plain children" run the parent's band keeps the ordinary selected colour.
fn run_color(rows: &[LayerRow], from: usize, to: usize) -> gpui::Rgba {
    let t = tokens();
    let all_icon = rows[from..=to].iter().all(|r| r.mark_kind == 2);
    let all_comp = rows[from..=to].iter().all(|r| r.mark_kind == 1);
    if all_icon {
        rgba(t.icon_layer_row)
    } else if all_comp {
        rgba(t.component_row)
    } else {
        rgba(t.selected_row)
    }
}

/// Drag and expansion state for the tree, kept on `CanvasView` across frames.
pub struct LayerTreeState {
    /// Container nodes that are expanded (have children and are open).
    pub expanded: BTreeSet<NodeId>,
    /// The rows rendered this frame.
    pub rows: Vec<LayerRow>,
    /// The same rows translated for the drag kit (window rect and path included).
    pub drag_rows: Vec<drag::Row>,
    /// The drag in progress; set on `on_drag`, cleared on release or Escape.
    pub drag: Option<drag::Drag>,
    /// A press that has not passed the threshold yet — it may become a click or a drag. The
    /// modifiers are kept so multi-select can use them.
    pub candidate: Option<(NodeId, Point<Pixels>, Modifiers)>,
    /// Start of the shift range: the last row that was selected outright (plain or toggled).
    pub anchor: Option<NodeId>,
    /// The last `(row index, real window rect)` reported by `on_drag_move`, used to rebuild the
    pub calibrated: Option<(usize, Bounds<Pixels>)>,
    /// The last global pointer position (window space): draws the drag label and resolves drops.
    pub pointer: Point<Pixels>,
    /// Scroll handle of the virtual list, kept across frames.
    pub scroll: VirtualListScrollHandle,
    clock: Instant,
}

impl Default for LayerTreeState {
    fn default() -> Self {
        Self {
            expanded: BTreeSet::new(),
            rows: Vec::new(),
            drag_rows: Vec::new(),
            drag: None,
            candidate: None,
            anchor: None,
            calibrated: None,
            pointer: Point::new(px(0.0), px(0.0)),
            scroll: VirtualListScrollHandle::new(),
            clock: Instant::now(),
        }
    }
}

impl CanvasView {
    /// The left panel: a section header plus the scrolling tree.
    ///
    /// The caller passes `left_w + HANDLE_W`: the extra pixel is the divider on the canvas side,
    /// drawn as a 1 px `border_r_1` on this container's right edge. Top, bottom and left draw
    /// nothing, so the header edge and the timeline edge do not double up. The inset border does
    /// not change the layout width: `canvas_x` and hit geometry still use `left_w + HANDLE_W`.
    /// The line turns into the accent colour while hovered or dragged.
    pub(crate) fn render_left_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = tokens();
        let divider_hot =
            crate::resize_drag::hovered(crate::resize_drag::Edge::Left) || crate::resize_drag::drag_active(crate::resize_drag::Handle::LeftPanel);
        // The name committed by the file header's rename field (`file.rename`).
        // Take it out first: the `RefMut` temporary lives to the end of the statement and cannot wrap a `&mut self` call.
        let renamed = self.file_rename.borrow_mut().take();
        if let Some(name) = renamed {
            self.run_command("file.rename", serde_json::json!({ "name": name }), cx);
        }
        self.sync_file_name(cx);
        // Left-panel tab: Assets / Icons render into the panel itself, not a floating card.
        match self.left_tab {
            1 => {
                let cmds = self.fields.queue();
                return div()
                    .id("left-panel")
                    .flex_none()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(rgba(t.panel))
                    .border_r_1()
                    .border_color(rgba(if divider_hot { t.accent } else { t.border }))
                    .child(SectionHeader::new(tr("Assets")).variant(SectionHeaderVariant::Normal))
                    .child(crate::panels::extra::assets_body(&self.app, &cmds, &self.extra_state, cx));
            }
            2 => {
                let cmds = self.fields.queue();
                return div()
                    .id("left-panel")
                    .flex_none()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(rgba(t.panel))
                    .border_r_1()
                    .border_color(rgba(if divider_hot { t.accent } else { t.border }))
                    .child(SectionHeader::new(tr("Icons")).variant(SectionHeaderVariant::Normal))
                    .child(crate::icons_panel::body(&self.app, &cmds, &self.extra_state, cx));
            }
            _ => {}
        }
        // Three parts: a 64 pt file header, the Pages section and the Layers section.
        // Previously this was only a Layers header plus the tree.
        div()
            .id("left-panel")
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgba(t.panel))
            .border_r_1()
            .border_color(rgba(if divider_hot { t.accent } else { t.border }))
            .child(self.file_header(cx))
            .child(self.pages_section(cx))
            // Rule between the Pages section and the Layers section.
            .child(div().flex_none().w_full().h(pt(1.0)).bg(rgba(t.border)))
            .child(SectionHeader::new(tr("Layers")).variant(SectionHeaderVariant::Normal))
            .child(self.render_layer_tree(cx))
    }

    /// Write the document name into the header field: only when the tab changed (new uid) or the
    /// name really changed, otherwise `set_value` every frame would interrupt typing.
    fn sync_file_name(&mut self, cx: &mut Context<Self>) {
        let Some(ds) = self.app.session.ds().ok() else { return };
        let (uid, name) = (ds.uid, ds.doc.name.clone());
        if self.file_name_seen.as_ref() == Some(&(uid, name.clone())) {
            return;
        }
        let field = self.file_name_field.clone();
        field.update(cx, |tf, _| tf.set_value(name.clone()));
        self.file_name_seen = Some((uid, name));
    }

    /// The left panel's file header (64 pt): editable file name, the drafts label, a collapse button.
    fn file_header(&mut self, cx: &mut Context<Self>) -> Div {
        let t = tokens();
        let field = self.file_name_field.clone();
        // Enter or losing focus queues a `file.rename`, picked up by the panel next frame.
        let pending = self.file_rename.clone();
        field.update(cx, |tf, _| {
            tf.set_on_commit(move |text, _, _| {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    *pending.borrow_mut() = Some(name);
                }
            });
        });
        let me = cx.entity();
        div()
            .flex_none()
            .w_full()
            .h(pt(64.0))
            .flex()
            .flex_col()
            .px(pt(14.0))
            .pt(pt(12.0))
            .border_b_1()
            .border_color(rgba(t.border))
            .child(div().flex().flex_row().items_center().gap(pt(6.0)).child(div().flex_1().min_w(px(0.0)).child(field)).child(
                IconButton::new(Icon::Panels).size(24.0).on_click(move |_e, _w, cx| {
                    me.update(cx, |cv, cx| {
                        cv.left_collapsed = true;
                        cx.notify();
                    });
                }),
            ))
            .child(div().mt(pt(2.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr("Drafts")))
    }

    /// Pages section: a 40 pt header (title, search, add) followed by the page rows.
    /// A page row is 32 pt; the current page has a fill and a divider page draws a line.
    fn pages_section(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = tokens();
        let Some(ds) = self.app.session.ds().ok() else { return div().id("pages-section") };
        let pages: Vec<(NodeId, String, bool)> =
            ds.doc.pages.iter().filter_map(|p| ds.doc.node(*p).map(|n| (*p, n.name.clone(), ds.doc.is_divider(*p)))).collect();
        let cur = ds.page;
        let open = *self.pages_open.borrow();
        let searching = *self.page_search.borrow();
        let filter = self.page_filter.borrow().trim().to_lowercase();
        let me = cx.entity();

        // ── 40 pt header row ────────────────────────────────────────────────────────────────
        // 用与 Layers/Assets/Icons **同一个**通用 `SectionHeader`（TITLE_PT=13 / MEDIUM /
        // HEADER_H=40 / pl PAD=16 / pr PAD_RIGHT=8）。以前这里是手写 `h(40)+pl(16)+pr(8)` +
        // `text_size(11).SEMIBOLD`，字号 11 vs 13、字重 SEMIBOLD vs MEDIUM，和 Layers 明显不一致
        // （用户 2026-11-12 指出 Pages/Layers 字体大小不同）。折叠点击绑外层，搜索/加号进 trail
        // 并各自 stop_propagation，避免点按钮误触发折叠。
        let open_slot = self.pages_open.clone();
        let search_slot = self.page_search.clone();
        let filter_slot = self.page_filter.clone();
        let me_add = me.clone();
        let trail = div()
            .flex()
            .flex_row()
            .gap(pt(4.0))
            .child(IconButton::new(Icon::Search).size(24.0).active(searching).on_click({
                let me = me.clone();
                move |_e, _, cx: &mut App| {
                    cx.stop_propagation();
                    let next = !*search_slot.borrow();
                    *search_slot.borrow_mut() = next;
                    if next {
                        filter_slot.borrow_mut().clear();
                    }
                    me.update(cx, |_, cx| cx.notify());
                }
            }))
            .child(IconButton::new(Icon::Plus).size(24.0).on_click(move |_e, _, cx: &mut App| {
                cx.stop_propagation();
                me_add.update(cx, |cv, cx| cv.run_command("page.add", serde_json::json!({}), cx));
            }));
        let head = div()
            .flex_none()
            .w_full()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, {
                let me = me.clone();
                move |_e, _, cx| {
                    let next = !*open_slot.borrow();
                    *open_slot.borrow_mut() = next;
                    me.update(cx, |_, cx| cx.notify());
                }
            })
            .child(SectionHeader::new(tr("Pages")).variant(SectionHeaderVariant::Normal).trail(trail));

        let mut sec = div().id("pages-section").flex_none().w_full().flex().flex_col().child(head);
        if !open {
            return sec;
        }
        if searching {
            // Filter as you type (`set_on_change`, not Enter).
            let field = self.page_search_field.clone();
            let slot = self.page_filter.clone();
            let me_field = me.clone();
            field.update(cx, |tf, _| {
                tf.set_on_change(move |text, _, cx| {
                    *slot.borrow_mut() = text.trim().to_string();
                    me_field.update(cx, |_, cx| cx.notify());
                });
            });
            // A height is what makes the field's `items_center` do anything: without one the
            // box is as tall as its content and the glyphs drift to the bottom.
            sec = sec.child(div().px(pt(8.0)).pb(pt(4.0)).h(pt(24.0)).child(field));
        }
        // lizard `pages()`: `resp.double_clicked()` starts an inline rename of that page row.
        let renaming_page = self.inline_rename.as_ref().filter(|r| r.target == crate::InlineRenameTarget::Page).and_then(|r| r.ids.first().copied());
        let rename_editor = self.inline_rename_field.clone();
        let mut list = div().id("pages-list").w_full().flex().flex_col().max_h(px(220.0)).overflow_y_scroll();
        for (id, name, divider) in pages {
            if !filter.is_empty() && (divider || !name.to_lowercase().contains(&filter)) {
                continue;
            }
            let is_cur = id == cur;
            let editing = renaming_page == Some(id) && !divider;
            let name_cell = if editing {
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(pt(24.0))
                    .px(pt(8.0))
                    // A press inside the editor must not bubble to the root's commit_edits.
                    .on_mouse_down(MouseButton::Left, |_e, _w, cx| cx.stop_propagation())
                    .child(rename_editor.clone())
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .rounded(pt(5.0))
                    .px(pt(8.0))
                    .py(pt(4.0))
                    .text_size(pt(11.0))
                    .text_color(ink())
                    .when(is_cur, |d| d.bg(rgba(t.selected_row)))
                    .when(!is_cur, |d| d.hover(|s| s.bg(rgba(t.hover))))
                    .child(if divider { SharedString::from("—") } else { SharedString::from(name) })
                    .into_any_element()
            };
            let me_row = me.clone();
            let row = div()
                .id(ElementId::from(SharedString::from(format!("page-{}", id.0))))
                .flex_none()
                .w_full()
                .h(pt(32.0))
                .px(pt(8.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                    if e.click_count >= 2 && !divider {
                        me_row.update(cx, |cv, cx| {
                            cv.begin_inline_rename(vec![id], crate::InlineRenameTarget::Page, cx);
                        });
                        // The second press of a double click must not bubble to the root either, or commit_edits would close the editor that was just opened.
                        cx.stop_propagation();
                    } else {
                        me_row.update(cx, |cv, cx| {
                            cv.run_command("page.select", serde_json::json!({ "id": id.0 }), cx);
                        });
                    }
                })
                .child(name_cell);
            list = list.child(row);
        }
        sec.child(list)
    }

    fn render_layer_tree(&mut self, cx: &mut Context<Self>) -> VirtualList {
        let (rows, drag_rows) = self.collect_layer_rows();
        // Keep the full row list for dragging and hit testing; only the viewport is rendered.
        // `drag::calibrate` rebuilds the column from a hit row plus the fixed pitch, so virtualisation does not change drop resolution.
        self.layer_tree.rows = rows.clone();
        self.layer_tree.drag_rows = drag_rows;
        let drag = self.layer_tree.drag.as_ref().map(|d| d.from_id);
        let marked = marks(&rows);
        let (first, last) = selection_runs(&marked);
        // The solid colour of each row's band; `None` for descendant / unmarked rows.
        let mut band: Vec<Option<gpui::Rgba>> = vec![None; rows.len()];
        let mut k = 0;
        while k < rows.len() {
            if marked[k] != Mark::Selected {
                k += 1;
                continue;
            }
            let start = k;
            while k < rows.len() && marked[k] == Mark::Selected {
                k += 1;
            }
            let end = k - 1;
            let color = run_color(&rows, start, end);
            for j in start..=end {
                band[j] = Some(color);
            }
        }
        // The per-row render snapshot (`'static` closures only get `&mut App`).
        let view: Rc<Vec<(LayerRow, Mark, Option<gpui::Rgba>, bool, bool)>> = Rc::new(
            rows.into_iter()
                .zip(marked)
                .zip(band)
                .enumerate()
                .map(|(i, ((r, m), b))| {
                    let in_run = m != Mark::None;
                    (r, m, b, in_run && i == first[i], in_run && i == last[i])
                })
                .collect(),
        );
        let n = view.len();
        let row_w = px(self.left_w + crate::panels::HANDLE_W);
        let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new((0..n).map(|_| Size::new(row_w, px(ROW_H))).collect());
        // R5: the one row being renamed renders the shared inline field instead of its name.
        // The virtual list's render closure only gets `&mut App`, so the (id, field) pair has
        // to be captured here, alongside the `Entity<CanvasView>` the row callbacks use.
        let rename: Option<(NodeId, Entity<crate::widgets::TextEdit>)> = self
            .inline_rename
            .as_ref()
            .filter(|r| r.target == crate::InlineRenameTarget::Layer)
            .and_then(|r| r.ids.first().copied())
            .map(|id| (id, self.inline_rename_field.clone()));
        let entity = cx.entity();
        let scroll = self.layer_tree.scroll.clone();
        v_virtual_list("layer-scroll", &scroll, item_sizes, move |range, _window, _app| {
            let mut out: Vec<AnyElement> = Vec::with_capacity(range.end - range.start);
            for ix in range {
                let Some((r, mark, band_color, run_top, run_bottom)) = view.get(ix) else { continue };
                let field = rename.as_ref().and_then(|(id, f)| (*id == r.id).then(|| f.clone()));
                out.push(Self::layer_row(&entity, ix, r, drag, *mark, band_color.clone(), *run_top, *run_bottom, field).into_any_element());
            }
            out
        })
        .flex_1()
        .min_h(px(0.0))
        .w_full()
    }

    /// One 16 pt glyph: lizard's original SVG when there is a LayerIcon, otherwise the
    /// generic Icon painter. An absolutely-positioned child in an auto-layout parent gets
    /// the Absolute glyph layered underneath, exactly as `layers()` does.
    fn layer_glyph_element(r: &LayerRow) -> gpui::AnyElement {
        use crate::widgets::layer_icons::{self, LayerIcon};
        let main = match r.glyph {
            Some(g) => layer_icons::paint(g, ink(), ICON_GLYPH).into_any_element(),
            None => paint_icon(r.icon, ink(), ICON_GLYPH).into_any_element(),
        };
        if !r.absolute {
            return main;
        }
        div()
            .relative()
            .w(px(ICON_GLYPH))
            .h(px(ICON_GLYPH))
            .child(div().absolute().top_0().left_0().size_full().child(layer_icons::paint(LayerIcon::Absolute, ink(), ICON_GLYPH)))
            .child(div().absolute().top_0().left_0().size_full().child(main))
            .into_any_element()
    }

    fn layer_row(
        entity: &Entity<CanvasView>,
        index: usize,
        r: &LayerRow,
        dragging: Option<u64>,
        mark: Mark,
        band_color: Option<gpui::Rgba>,
        run_top: bool,
        run_bottom: bool,
        rename_field: Option<Entity<crate::widgets::TextEdit>>,
    ) -> Stateful<Div> {
        let t = tokens();
        let is_dragging = dragging == Some(r.id.0);
        let row_h = pt(ROW_H);
        let indent = r.depth as f32 * LAYER_INDENT;

        // The row itself is transparent; selection and hover are a separate rounded block inset
        // 8 pt on each side and 4 pt above and below, not a 32 pt-tall slab covering the row.
        // Selected rows use `t.selected_row`, hovered rows a solid `t.hover` with no outline.
        // Hover comes from the whole row's hitbox, so it is bound to the group.
        let group: SharedString = format!("layer-row-{}", r.id.0).into();

        // Row colours: component members and slots use the component colour, selected rows use
        // `text`, ordinary rows the theme's 90% black, hidden rows `text_tertiary`.
        // The theme's 90% black is `t.icon`; hidden rows fall back to `text_tertiary`.
        let component_member = r.mark_kind != 0;
        let mark_col = if r.mark_kind == 2 { t.icon_layer } else { t.component };
        let layer_secondary = ink();
        let name_color = if component_member {
            rgba(mark_col)
        } else if r.visible {
            rgba(t.text)
        } else {
            rgba(t.text_tertiary)
        };
        let icon_color = if component_member || r.icon == Icon::Slot {
            rgba(mark_col)
        } else if r.visible {
            layer_secondary
        } else {
            rgba(t.text_tertiary)
        };
        let id = r.id;
        let name = r.name.clone();
        let has_children = r.has_children;
        let expanded = r.expanded;
        let visible = r.visible;
        let locked = r.locked;

        // A stable id for the row entity: GPUI's built-in drag needs a stateful element.
        let row_id = ElementId::from(SharedString::from(format!("layer-row-{}", id.0)));
        let me = entity.clone();

        let chevron = if has_children {
            div()
                .w(px(ICON_BOX))
                .h(px(ICON_BOX))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, {
                    let me = me.clone();
                    move |_e: &MouseDownEvent, _w: &mut Window, app: &mut App| {
                        me.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                            this.layer_toggle_expand(id, cx);
                        });
                    }
                })
                .child(paint_icon(if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, ink(), ICON_GLYPH))
        } else {
            div().w(px(ICON_BOX))
        };

        let eye = div()
            .w(px(ICON_BOX))
            .h(px(ICON_BOX))
            .flex()
            .items_center()
            .justify_center()
            // lizard `show_eye = over || hidden`: a visible layer's eye appears on hover only.
            .when(visible, |d| d.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
            .on_mouse_down(MouseButton::Left, {
                let me = me.clone();
                move |_e: &MouseDownEvent, _w: &mut Window, app: &mut App| {
                    me.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                        this.layer_toggle_visible(id, cx);
                    });
                }
            })
            .child(paint_icon(if visible { Icon::Eye } else { Icon::EyeOff }, ink(), ICON_GLYPH));

        let lock = div()
            .w(px(ICON_BOX))
            .h(px(ICON_BOX))
            .flex()
            .items_center()
            .justify_center()
            // lizard `show_lock = over || locked`: an unlocked layer's padlock appears on hover only.
            .when(!locked, |d| d.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
            .on_mouse_down(MouseButton::Left, {
                let me = me.clone();
                move |_e: &MouseDownEvent, _w: &mut Window, app: &mut App| {
                    me.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                        this.layer_toggle_lock(id, cx);
                    });
                }
            })
            .child(paint_icon(if locked { Icon::Lock } else { Icon::Unlock }, ink(), ICON_GLYPH));

        // Drag start: set the state immediately and remember the group and its label.
        let me_start = me.clone();
        // Move: hand the hit row's real window rect to the kit, then resolve with the pointer.
        let me_move = me.clone();

        // One run is one block: its first and last rows inset by 4 pt (block height 24) and the
        // rows between them do not, so a parent's subtree reads as one light strip; separate
        // runs end up 8 pt apart. The selected parent takes the solid band, its children the
        // lighter shade, so the two levels stay distinguishable. Hovered rows inset too, otherwise
        let (inset_top, inset_bottom) = if mark == Mark::None {
            (BLOCK_PAD, BLOCK_PAD)
        } else {
            (if run_top { BLOCK_PAD } else { 0.0 }, if run_bottom { BLOCK_PAD } else { 0.0 })
        };
        let highlight = if mark == Mark::None {
            // Hover is the theme's light accent tint, not a neutral grey.
            div().size_full().rounded(pt(5.0)).group_hover(group.clone(), |s| s.bg(rgba(t.selected_row_child)))
        } else {
            let mut h = div().size_full().bg(band_color.unwrap_or(rgba(t.selected_row_child)));
            if run_top {
                h = h.rounded_t(pt(5.0));
            }
            if run_bottom {
                h = h.rounded_b(pt(5.0));
            }
            h
        };

        // R5 (egui `layers.rs::rename_field`): the name slot of the row being renamed is the
        // input field itself. No hit handler while editing, so clicking the field does not
        // re-select the layer underneath it.
        let name_cell = if let Some(field) = rename_field {
            div()
                .flex_1()
                .min_w(px(0.0))
                // Full row height, exactly like the label it replaces: a 24 pt box inside a 32 pt
                // row only lines up if the row centres it, and one extra box is one more place
                // for the text to drift. Same box, same size, same centre.
                .h_full()
                .flex()
                .items_center()
                .on_mouse_down(MouseButton::Left, |_e, _w, cx| cx.stop_propagation())
                .child(field)
        } else {
            div()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex()
                .items_center()
                .on_mouse_down(MouseButton::Left, {
                    let me = me.clone();
                    move |e: &MouseDownEvent, _w: &mut Window, app: &mut App| {
                        me.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                            // lizard `layers.rs:1142`: a double click starts an inline rename
                            // of this row; a single click still goes through the selection logic.
                            if e.click_count >= 2 {
                                this.begin_inline_rename(vec![id], crate::InlineRenameTarget::Layer, cx);
                                // Same as above: this press opens the rename, it is not a click elsewhere.
                                cx.stop_propagation();
                            } else {
                                this.layer_on_row_mouse_down(id, e);
                            }
                        });
                    }
                })
                .child(div().child(name).text_size(px(11.0)).text_color(name_color).truncate())
        };

        div()
            .group(group)
            .id(row_id)
            .relative()
            .flex_none()
            .h(row_h)
            .w_full()
            .flex()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            // The inset layer: an overlay the size of the row whose padding squeezes the highlight into an 8/4 inset block.
            .child(div().absolute().top_0().left_0().size_full().px(pt(ROW_INSET)).pt(pt(inset_top)).pb(pt(inset_bottom)).child(highlight))
            .when(is_dragging, |d| d.opacity(0.5))
            .on_drag(LayerDrag(id), move |_val, _offset, _window, app| {
                me_start.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                    this.begin_layer_drag(id, index, cx);
                });
                app.new(|_| LayerDragGhost)
            })
            .on_drag_move::<LayerDrag>(move |ev, _window, app| {
                let dragging = ev.drag(app).0;
                let b = ev.bounds;
                let pointer = ev.event.position;
                me_move.update(app, |this: &mut CanvasView, cx: &mut Context<CanvasView>| {
                    this.on_layer_drag_move(dragging, index, b, pointer, cx);
                });
            })
            .child(div().w(px(indent)))
            .child(chevron)
            .child(div().w(px(ICON_BOX)).h(px(ICON_BOX)).flex().items_center().justify_center().child(Self::layer_glyph_element(r)))
            .child(name_cell)
            .child(eye)
            .child(lock)
    }

    /// Flatten the current page into visible rows (DFS, **topmost in z first**).
    /// Children are stored bottom-first, so the stack is filled in order and popped from the end.
    ///
    /// `stack.pop()` takes the **last** pushed element, so children are pushed in their natural
    /// order and come out top-first. Pushing them reversed cancelled out and produced a
    /// bottom-first list, while the drop logic assumed top-first — that mismatch is why the
    /// insertion line and the dropped node ended up at opposite ends.
    /// The same pass produces the kit's `drag::Row`s, keyed by the ancestor-id path.
    fn collect_layer_rows(&mut self) -> (Vec<LayerRow>, Vec<drag::Row>) {
        let Ok(ds) = self.app.session.ds() else { return (Vec::new(), Vec::new()) };
        let doc = &ds.doc;
        let page = ds.page;
        let Some(page_n) = doc.node(page) else { return (Vec::new(), Vec::new()) };
        let mut visual = Vec::new();
        let mut drag_rows: Vec<drag::Row> = Vec::new();
        let mut stack: Vec<(NodeId, usize, String)> = page_n.children.iter().map(|&c| (c, 0, String::new())).collect();
        while let Some((id, depth, parent_path)) = stack.pop() {
            let Some(n) = doc.node(id) else { continue };
            let has_children = !n.children.is_empty();
            let expanded = self.layer_tree.expanded.contains(&id);
            let path = if parent_path.is_empty() { id.0.to_string() } else { format!("{parent_path}/{}", id.0) };
            let can_into = n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. });
            let mark_kind = if matches!(n.kind, NodeKind::Icon { .. }) {
                2
            } else if matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. }) {
                1
            } else {
                0
            };
            let icon = Self::node_icon(doc, id);
            visual.push(LayerRow {
                id,
                depth,
                name: n.name.clone(),
                icon,
                glyph: Self::layer_glyph(doc, id, icon).0,
                absolute: Self::layer_glyph(doc, id, icon).1,
                has_children,
                can_into,
                expanded,
                selected: ds.selection.contains(&id),
                mark_kind,
                visible: n.visible,
                locked: n.locked,
            });
            drag_rows.push(drag::Row {
                id: id.0,
                path: path.clone(),
                can_into,
                collapsed: has_children && !expanded,
                rect: Bounds { origin: Point::new(px(0.0), px(0.0)), size: Size::new(px(240.0), px(ROW_H)) },
            });
            if has_children && expanded {
                // Push in natural order: `pop` takes the last child (highest z) first.
                for &c in n.children.iter() {
                    stack.push((c, depth + 1, path.clone()));
                }
            }
        }
        // Rebuild the rows every frame; recalibrate the column from the last hit row's rect.
        if let Some((hit, b)) = self.layer_tree.calibrated {
            drag::calibrate(&mut drag_rows, hit, b);
        }
        (visual, drag_rows)
    }

    /// The kit's level configuration: level 1 starts 8 pt inside the row, each level adds 16 pt.
    fn layer_config(rows: &[drag::Row]) -> drag::Config {
        let base = rows.first().map(|r| f32::from(r.rect.origin.x) + ROW_INSET).unwrap_or(ROW_INSET);
        drag::Config { base, indent: LAYER_INDENT }
    }

    /// Node kind to icon.
    /// The complete glyph decision for a row: mask group, elliptical mask and plain mask come
    /// first, then the icon-to-glyph table, and anything not in that table (booleans, slots,
    /// component sets, pages, sections, slices) falls back to the generic icon painter.
    /// The second value marks an absolutely-positioned child, which gets an extra Absolute glyph.
    fn layer_glyph(doc: &Document, id: NodeId, icon: Icon) -> (Option<crate::widgets::layer_icons::LayerIcon>, bool) {
        use crate::widgets::layer_icons::LayerIcon as L;
        let Some(n) = doc.node(id) else { return (None, false) };
        let absolute = n.child_layout.absolute && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
        let mask_group = matches!(n.kind, NodeKind::Group) && n.children.iter().any(|c| doc.node(*c).is_some_and(|node| node.is_mask));
        let glyph = if mask_group {
            Some(L::Mask)
        } else if n.is_mask && icon == Icon::Ellipse {
            Some(L::EllipseMask)
        } else if n.is_mask {
            Some(L::Mask)
        } else {
            match icon {
                Icon::Frame => Some(L::Frame),
                Icon::LayoutH => Some(L::LayoutH),
                Icon::LayoutV => Some(L::LayoutV),
                Icon::LayoutGrid => Some(L::LayoutGrid),
                Icon::Text => Some(L::Text),
                Icon::Ellipse => Some(L::Ellipse),
                Icon::Component => Some(L::Component),
                Icon::Instance => Some(L::Instance),
                Icon::Vector => Some(L::Vector),
                Icon::Image => Some(L::Image),
                Icon::Star => Some(L::Star),
                Icon::Polygon => Some(L::Polygon),
                Icon::Arrow => Some(L::Arrow),
                Icon::Line => Some(L::Line),
                Icon::Rectangle => Some(L::Rectangle),
                Icon::Group => Some(L::Group),
                _ => None,
            }
        };
        (glyph, absolute)
    }

    /// Node kind to icon, one arm per kind.
    /// Four cases are easy to miss: auto-layout frames, image-filled rectangles, arrow-ended lines
    fn node_icon(doc: &Document, id: NodeId) -> Icon {
        let Some(n) = doc.node(id) else { return Icon::Rectangle };
        // An operand combining by an operation other than its group's shows that operation.
        if let Some(op) = distinct_op(doc, id) {
            return match op {
                BoolOp::Union => Icon::Union,
                BoolOp::Subtract => Icon::Subtract,
                BoolOp::Intersect => Icon::Intersect,
                BoolOp::Exclude => Icon::Exclude,
            };
        }
        match &n.kind {
            NodeKind::Page { .. } => Icon::Page,
            NodeKind::Frame if is_slot(doc, id) => Icon::Slot,
            NodeKind::Frame => {
                if let Some(l) = &n.layout {
                    match l.mode {
                        figcraft_doc::LayoutMode::Vertical => Icon::LayoutV,
                        figcraft_doc::LayoutMode::Grid => Icon::LayoutGrid,
                        _ => Icon::LayoutH,
                    }
                } else {
                    Icon::Frame
                }
            }
            NodeKind::Group => Icon::Group,
            NodeKind::Section { .. } => Icon::Section,
            NodeKind::Rectangle => {
                if n.fills.iter().any(|f| matches!(f.kind, figcraft_color::PaintKind::Image(_))) {
                    Icon::Image
                } else {
                    Icon::Rectangle
                }
            }
            NodeKind::Ellipse { .. } => Icon::Ellipse,
            NodeKind::Polygon { .. } => Icon::Polygon,
            NodeKind::Star { .. } => Icon::Star,
            NodeKind::Line => {
                if n.stroke.cap_end.is_arrow() {
                    Icon::Arrow
                } else {
                    Icon::Line
                }
            }
            NodeKind::Vector { .. } => Icon::Vector,
            NodeKind::Text(_) => Icon::Text,
            NodeKind::BooleanOp { op } => match op {
                BoolOp::Union => Icon::Union,
                BoolOp::Subtract => Icon::Subtract,
                BoolOp::Intersect => Icon::Intersect,
                BoolOp::Exclude => Icon::Exclude,
            },
            NodeKind::Component { .. } => Icon::Component,
            NodeKind::ComponentSet { .. } => Icon::ComponentSet,
            NodeKind::Instance { .. } => Icon::Instance,
            // The document model has an icon node type; it keeps its own glyph.
            NodeKind::Icon { .. } => Icon::IconKind,
            NodeKind::Slice => Icon::Slice,
        }
    }

    // ── Drag events ──────────────────────────────────────────────────────────────

    /// `on_drag`: drag the whole selection, with the name or "N layers" as the label.
    fn begin_layer_drag(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        let Ok(ds) = self.app.session.ds() else { return };
        let moving: Vec<NodeId> =
            if ds.selection.contains(&id) && ds.selection.len() > 1 { ds.selection.iter().copied().collect() } else { vec![id] };
        let label = if moving.len() == 1 { ds.doc.node(id).map(|n| n.name.clone()).unwrap_or_default() } else { format!("{} layers", moving.len()) };
        self.layer_tree.drag = Some(drag::Drag::start(index, id.0, label));
        // Once a drag starts it is no longer a click.
        self.layer_tree.candidate = None;
        cx.notify();
    }

    /// `on_drag_move`: GPUI only reports the hit row's rect; the kit calibrates the column.
    fn on_layer_drag_move(&mut self, dragging: NodeId, index: usize, bounds: Bounds<Pixels>, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self.layer_tree.drag.as_ref().map(|d| d.from_id) != Some(dragging.0) {
            return;
        }
        self.layer_tree.calibrated = Some((index, bounds));
        self.layer_tree.pointer = pointer;
        drag::calibrate(&mut self.layer_tree.drag_rows, index, bounds);
        self.update_layer_drop(cx);
    }

    /// Root-level `on_mouse_move`: keeps resolving drops between rows and outside the list.
    pub(crate) fn layer_on_global_move(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self.layer_tree.drag.is_none() {
            return;
        }
        self.layer_tree.pointer = pointer;
        self.update_layer_drop(cx);
    }

    /// Resolve the drop every frame to draw the line, **without applying** it; hovering a
    fn update_layer_drop(&mut self, cx: &mut Context<Self>) {
        let mut d = match self.layer_tree.drag.take() {
            Some(d) => d,
            None => return,
        };
        let rows = std::mem::take(&mut self.layer_tree.drag_rows);
        let cfg = Self::layer_config(&rows);
        let now = self.layer_tree.clock.elapsed().as_secs_f64();
        let expand = d.update(&rows, self.layer_tree.pointer, cfg, now);
        self.layer_tree.drag_rows = rows;
        self.layer_tree.drag = Some(d);
        if let Some(eid) = expand {
            self.layer_tree.expanded.insert(NodeId(eid));
            let (visual, drag_rows) = self.collect_layer_rows();
            self.layer_tree.rows = visual;
            self.layer_tree.drag_rows = drag_rows;
        }
        cx.notify();
    }

    /// Escape cancels the drag: no selection change, nothing sent to the engine.
    pub(crate) fn layer_cancel_drag(&mut self, cx: &mut Context<Self>) -> bool {
        if self.layer_tree.drag.is_none() {
            return false;
        }
        self.layer_tree.drag = None;
        self.layer_tree.calibrated = None;
        self.layer_tree.candidate = None;
        cx.notify();
        true
    }

    /// Root-level drag visuals: insertion line, whole-row outline for Into, and the label.
    pub(crate) fn layer_overlay(&self) -> Vec<AnyElement> {
        match &self.layer_tree.drag {
            Some(d) => drag::overlay(d, self.layer_tree.pointer),
            None => Vec::new(),
        }
    }

    // ── Events ──────────────────────────────────────────────────────────────────

    fn layer_on_row_mouse_down(&mut self, id: NodeId, e: &MouseDownEvent) {
        // Keep the modifiers: on release they decide plain select / toggle / range.
        self.layer_tree.candidate = Some((id, e.position, e.modifiers));
    }

    fn layer_toggle_expand(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if self.layer_tree.expanded.contains(&id) {
            self.layer_tree.expanded.remove(&id);
        } else {
            self.layer_tree.expanded.insert(id);
        }
        cx.notify();
    }

    fn layer_toggle_visible(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("object.toggleVisible", &json!({ "ids": [id] }));
        cx.notify();
    }

    fn layer_toggle_lock(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("object.toggleLock", &json!({ "ids": [id] }));
        cx.notify();
    }

    /// Selecting from a row click:
    /// - plain click: replace the selection with `[id]` and make `id` the shift anchor;
    /// - cmd/ctrl: toggle `id` in the selection and move the anchor;
    /// - shift: select the visible range from the anchor (if still selected) to `id`, merging
    ///   with the current selection when cmd is held, otherwise replacing it.
    fn layer_click(&mut self, id: NodeId, m: Modifiers, cx: &mut Context<Self>) {
        let Ok(ds) = self.app.session.ds() else { return };
        let order: Vec<NodeId> = self.layer_tree.rows.iter().map(|r| r.id).collect();
        let sel: Vec<NodeId> = ds.selection.iter().copied().collect();
        let pos = |x: NodeId| order.iter().position(|o| *o == x);
        let anchor = self.layer_tree.anchor.filter(|a| sel.contains(a)).and_then(|a| pos(a));
        let (ai, bi) = (anchor, pos(id));
        // The range needs both the anchor and the clicked row in the visible order.
        let mut set_anchor = false;
        let new_sel = if m.shift && ai.is_some() && bi.is_some() {
            let (lo, hi) = (ai.unwrap().min(bi.unwrap()), ai.unwrap().max(bi.unwrap()));
            let mut s: Vec<NodeId> = if m.platform || m.control { sel.clone() } else { Vec::new() };
            for x in order.get(lo..=hi).unwrap_or_default() {
                if !s.contains(x) {
                    s.push(*x);
                }
            }
            s
        } else if m.shift || m.platform || m.control {
            let mut s = sel.clone();
            if let Some(p) = s.iter().position(|x| *x == id) {
                s.remove(p);
            } else {
                s.push(id);
            }
            set_anchor = true;
            s
        } else {
            set_anchor = true;
            vec![id]
        };
        if set_anchor {
            self.layer_tree.anchor = Some(id);
        }
        let _ = self.app.session.execute("select.set", &json!({ "ids": new_sel }));
        cx.notify();
    }

    fn layer_select(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let _ = self.app.session.execute("select.set", &json!({ "ids": [id] }));
        cx.notify();
    }

    /// Called from the root `on_mouse_up`: a drag commits its drop **here and only here**;
    pub(crate) fn layer_drag_up(&mut self, cx: &mut Context<Self>) {
        if let Some(mut d) = self.layer_tree.drag.take() {
            if let Some(target) = d.take()
                && !target.refused
                && !target.noop
            {
                self.layer_commit_drop(&d, &target, cx);
            }
            self.layer_tree.calibrated = None;
            cx.notify();
        } else if let Some((cid, _, m)) = self.layer_tree.candidate.take() {
            self.layer_click(cid, m, cx);
        }
    }

    /// Translate the kit's drop into a single `node.reparent` (one undo step).
    ///
    /// - `Into`: the parent is the anchor and the node becomes its first child.
    /// - otherwise the level comes from the pointer's x. `parent` is the ancestor at `level-1`
    ///   of the anchor path, `boundary` the segment at `level`.
    ///   `after` places the node after that whole subtree, otherwise before it.
    /// - The list runs top-first, which is the reverse of the z order: lower in the list means
    ///   earlier among the children, so `after` maps to `gap = idx` and `!after` to `idx + 1`.
    ///   Both use the same gap the insertion line is drawn at, so the node lands where the line was.
    ///   (This is the placement fix that stopped nodes landing at the wrong end of the list.)
    ///
    fn layer_commit_drop(&mut self, d: &drag::Drag, target: &drag::Target, cx: &mut Context<Self>) {
        let dragging = NodeId(d.from_id);
        let Ok(ds) = self.app.session.ds() else { return };
        let doc = &ds.doc;
        let page = ds.page;
        let moving: Vec<NodeId> =
            if ds.selection.contains(&dragging) && ds.selection.len() > 1 { ds.selection.iter().copied().collect() } else { vec![dragging] };
        // Look the row up by id instead of by index: if a hover-expand fired on the release
        // frame, the visible rows were reordered and the index no longer matches.
        let Some(anchor_row) = self.layer_tree.drag_rows.iter().find(|r| r.id == target.anchor_id) else { return };
        let anchor_path = anchor_row.path.clone();

        if target.zone == drag::Zone::Into {
            // Into passes only `ids` + `parent`, with **no index**: the engine puts an index-less
            // reparent on top. Passing 0 would drop it to the bottom of the siblings instead.
            let _ = self.app.session.execute("node.reparent", &json!({ "ids": moving, "parent": NodeId(target.anchor_id) }));
            cx.notify();
            return;
        }

        let level = target.level;
        let (parent, boundary) = if level <= 1 {
            (Some(page), path_id(anchor_path.split('/').next()))
        } else {
            let pp = drag::parent_path_at(&anchor_path, level);
            (path_id(pp.rsplit('/').next()), path_id(anchor_path.split('/').nth(level - 1)))
        };
        let (Some(parent), Some(boundary)) = (parent, boundary) else { return };
        let Some(parent_n) = doc.node(parent) else { return };
        let idx = doc.index_in_parent(boundary).unwrap_or(0);
        let gap = if target.after { idx } else { idx + 1 };
        // A dragged row that sat before the gap frees its slot as it leaves.
        let below = parent_n.children.iter().take(gap).filter(|c| moving.contains(c)).count();
        let index = gap.saturating_sub(below);
        let _ = self.app.session.execute("node.reparent", &json!({ "ids": moving, "parent": parent, "index": index }));
        cx.notify();
    }
}

fn path_id(seg: Option<&str>) -> Option<NodeId> {
    seg.and_then(|s| s.parse::<u64>().ok()).map(NodeId)
}
