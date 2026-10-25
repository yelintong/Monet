//! Drag to reorder the Design panel's lists (fills, strokes, effects, layout grids) and the pages
//! list. A row is dragged by its grip, the empty strip left of its fields (a handle shows over the
//! row), a page by its whole row; while dragging, a line shows where it will land and its label
//! follows the pointer, as for layers dragged in the layers panel; dropping moves it with one
//! command (one undo step). The Design panel shows the topmost item first while the model keeps
//! lists bottom first; other lists (an interaction's actions, pages) show their items in model
//! order.

use egui::{AsId, Id, Rect, Sense, Ui, pos2, vec2};

use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular};
use crate::widgets::{self, PAD};

/// The id of list `list` (any hashable key, e.g. `"fills"`).
fn list_id(list: impl AsId) -> Id {
    Id::new(("list_drag", list))
}

/// The row of list `list` being dragged (its model index; temp data).
fn drag_id(list: Id) -> Id {
    list.with("dragging")
}

/// The grip of item `index` of list `list` (tests find rows by it).
#[cfg(test)]
pub(crate) fn grip_id(list: impl AsId, index: usize) -> Id {
    grip_of(list_id(list), index)
}

fn grip_of(list: Id, index: usize) -> Id {
    list.with(("grip", index))
}

/// Where a drop landed: the item's model index, the index it goes to, and whether the pointer was
/// on the far side of a group break (see [`ListDrag::break_at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dropped {
    pub from: usize,
    pub to: usize,
    /// The drop ends the group above the break rather than opening the group below it.
    pub above: bool,
}

impl Dropped {
    /// The move as `(from, to)` model indices (for `fill.move` and friends).
    pub fn pair(self) -> (usize, usize) {
        (self.from, self.to)
    }
}

/// One list's rows in one frame: lay out each row, register it with [`ListDrag::row`], then call
/// [`ListDrag::finish`].
pub struct ListDrag {
    list: Id,
    /// Whether the rows show the model's last item first (the Design panel's paints).
    reversed: bool,
    /// The item being dragged (model index).
    dragging: Option<usize>,
    /// The rows as laid out, top to bottom: model index and the area the row covers (with any
    /// settings open under it).
    rows: Vec<(usize, Rect)>,
    /// What the pointer carries.
    label: Option<String>,
    /// Where a drop is taken, when the caller knows better than half a row around the rows: a
    /// list whose groups open with a caption (the Local styles list) takes a drop anywhere in the
    /// group, so its first and last rows are as easy to reach as the ones between.
    bounds: Option<Rect>,
    /// How far the insertion line stays inside the rows' sides.
    line_inset: f32,
    /// Row indices where another group starts (a folder's rows after another folder's): the gap
    /// there holds the group's header, and which half of it the pointer is in says whether the
    /// drop ends the group above or opens the one below.
    breaks: Vec<usize>,
}

impl ListDrag {
    /// A list shown topmost item (the model's last) first, as the Design panel shows paints.
    pub fn new(ui: &Ui, list: impl AsId) -> Self {
        Self::with_order(ui, list_id(list), true)
    }

    /// A list shown in model order, first item on top (an interaction's actions).
    pub fn in_order(ui: &Ui, list: impl AsId) -> Self {
        Self::with_order(ui, list_id(list), false)
    }

    fn with_order(ui: &Ui, list: Id, reversed: bool) -> Self {
        let id = drag_id(list);
        let mut dragging: Option<usize> = ui.data(|d| d.get_temp(id));
        // Escape cancels (and goes no further: it doesn't deselect or close what the list is in); a
        // drag whose release this list never saw (it wasn't shown) is over.
        if dragging.is_some()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || !(i.pointer.primary_down() || i.pointer.any_released()))
        {
            ui.data_mut(|d| d.remove::<usize>(id));
            dragging = None;
        }
        Self { list, reversed, dragging, rows: Vec::new(), label: None, bounds: None, line_inset: PAD, breaks: Vec::new() }
    }

    /// Take drops anywhere in `bounds` (elsewhere: half a row around the rows). A list whose
    /// groups open with a caption passes the whole group, so its head and tail are full targets.
    pub fn bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Keep the insertion line `inset` inside the rows' sides (the rows may sit inset themselves).
    pub fn line_inset(mut self, inset: f32) -> Self {
        self.line_inset = inset;
        self
    }

    /// Row `index` starts another group: the rows above and below it are separated by a header
    /// (a folder's name), and a drop in that gap has to say which side it belongs to — the end of
    /// the group above or the start of the group below.
    pub fn break_at(&mut self, index: usize) {
        self.breaks.push(index);
    }

    /// Item `index` (model order) was laid out with its main row at `header`, covering `extent`
    /// (the header plus anything open under it). Its grip starts a drag carrying `label`.
    pub fn row(&mut self, ui: &mut Ui, index: usize, header: Rect, extent: Rect, label: impl FnOnce() -> String) {
        let grip = Rect::from_min_max(header.min, pos2(header.min.x + PAD, header.max.y));
        let resp = ui.interact(grip, grip_of(self.list, index), Sense::drag());
        // The handle shows while the pointer is over the row (or the row is being dragged).
        let shown = match self.dragging {
            Some(d) => d == index,
            None => ui.rect_contains_pointer(header),
        };
        if shown {
            let t = Tokens::get(ui.ctx());
            let icon = Rect::from_center_size(grip.center(), vec2(12.0, 12.0));
            // Over the panel, hiding any guide line drawn under the rows (a conditional block's).
            ui.painter().rect_filled(icon, 0.0, t.panel);
            icons::paint(ui.painter(), icon, Icon::DragHandle, t.icon_secondary);
        }
        if resp.hovered() && self.dragging.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        self.row_by(&resp, index, extent, label);
    }

    /// Item `index` laid out over `extent`, dragged by the whole row (`resp` senses drags; the
    /// pages list).
    pub fn row_by(&mut self, resp: &egui::Response, index: usize, extent: Rect, label: impl FnOnce() -> String) {
        if resp.drag_started() {
            resp.ctx.data_mut(|d| d.insert_temp(drag_id(self.list), index));
            self.dragging = Some(index);
        }
        if self.dragging == Some(index) {
            self.label = Some(label());
        }
        self.rows.push((index, extent));
    }

    /// After the rows: while dragging, the insertion line and the dragged label; on release over
    /// the list, the move (for `fill.move` and friends).
    pub fn finish(self, ui: &mut Ui) -> Option<Dropped> {
        let from = self.dragging?;
        let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_released()));
        let target = pos.and_then(|p| drop_slot(&self.rows, from, p, self.reversed, self.bounds, &self.breaks));
        if let Some((_, y, _)) = target
            && let Some(span) = self.span()
        {
            let (x0, x1) = (span.min.x + self.line_inset, span.max.x - self.line_inset);
            widgets::insertion_line(ui.painter(), x0, x1.max(x0 + 1.0), y);
        }
        if let (Some(p), Some(label)) = (pos, self.label) {
            let t = Tokens::get(ui.ctx());
            let galley = ui.painter().layout_no_wrap(label, regular(11.0), t.text);
            widgets::drag_ghost(ui.ctx(), p, galley);
        }
        if !released {
            return None;
        }
        ui.data_mut(|d| d.remove::<usize>(drag_id(self.list)));
        target.map(|(to, _, above)| Dropped { from, to, above })
    }

    /// The area all rows cover.
    fn span(&self) -> Option<Rect> {
        self.rows.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b))
    }
}

/// Where item `from` lands when dropped at `p` over `rows` (top to bottom; the model's last item
/// first when `reversed`): its new model index, the insertion line's y and whether it ends the
/// group above a `breaks` gap. None outside the list (`bounds`, or half a row of slack above and
/// below the rows) or where nothing would change.
fn drop_slot(
    rows: &[(usize, Rect)],
    from: usize,
    p: egui::Pos2,
    reversed: bool,
    bounds: Option<Rect>,
    breaks: &[usize],
) -> Option<(usize, f32, bool)> {
    let (first, last) = (rows.first()?.1, rows.last()?.1);
    let slack = first.height().min(last.height()) / 2.0;
    let area = bounds.unwrap_or_else(|| {
        Rect::from_min_max(pos2(first.min.x.min(last.min.x), first.min.y - slack), pos2(first.max.x.max(last.max.x), last.max.y + slack))
    });
    if !area.contains(p) {
        return None;
    }
    let n = rows.len();
    let shown = rows.iter().position(|(i, _)| *i == from)?;
    // The gap under the pointer: how many rows sit above it.
    let gap = rows.iter().filter(|(_, r)| r.center().y < p.y).count();
    // A gap that holds a group's header: its top half ends the group above, its bottom half opens
    // the group below. Both are the same place in the list, but they belong to different groups —
    // which is why a drop there is a move even when the row keeps its place: it changes group.
    let above = gap > 0 && gap < n && breaks.contains(&gap) && p.y < (rows[gap - 1].1.max.y + rows[gap].1.min.y) / 2.0;
    // The row above the gap is the dragged one (it goes back where it was), or the one below is.
    let keeps_place = gap == shown || gap == shown + 1;
    // …unless the drop crosses the header: the first row of a group dropped in the top half of the
    // header ends the group above, the last row of a group dropped in its bottom half opens the one
    // below. Either way the row stays where it is but joins the other group.
    let crosses = breaks.contains(&gap) && if gap == shown { above } else { !above };
    if keeps_place && !crosses {
        return None;
    }
    // The line sits on the edge of the row the drop belongs to: over the group above's last row,
    // or under the header, over the group below's first — never across the header.
    let y = match rows.get(gap) {
        Some((_, below)) if !above => below.min.y,
        _ => rows[gap - 1].1.max.y,
    };
    // Shown position once moved, then its model index.
    let at = if gap > shown { gap - 1 } else { gap };
    Some((if reversed { n - 1 - at } else { at }, y, above))
}

/// Keep per-row flags (`Id::new((salt, index))`, e.g. an open settings section) with their rows
/// when item `from` of a `len`-long list moves to `to`.
pub fn move_flags(ui: &Ui, salt: &'static str, len: usize, from: usize, to: usize) {
    ui.data_mut(|d| {
        let flags: Vec<Option<bool>> = (0..len).map(|i| d.get_temp::<bool>(Id::new((salt, i)))).collect();
        for (i, f) in flags.into_iter().enumerate() {
            let j = figcraft_engine::cmd::lists::moved_index(i, from, to);
            match f {
                Some(v) => {
                    d.insert_temp(Id::new((salt, j)), v);
                }
                None => d.remove::<bool>(Id::new((salt, j))),
            }
        }
    });
}

/// Keep per-row flags (`Id::new((salt, index))`) with their rows when item `removed` of a
/// `len`-long list goes; its own flag goes with it.
pub fn remove_flags(ui: &Ui, salt: &'static str, len: usize, removed: usize) {
    ui.data_mut(|d| {
        let flags: Vec<Option<bool>> = (0..len).map(|i| d.get_temp::<bool>(Id::new((salt, i)))).collect();
        for i in 0..len {
            d.remove::<bool>(Id::new((salt, i)));
        }
        for (i, f) in flags.into_iter().enumerate() {
            if let (Some(j), Some(v)) = (figcraft_engine::cmd::lists::removed_index(i, removed), f) {
                d.insert_temp(Id::new((salt, j)), v);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Vec<(usize, Rect)> {
        // Shown topmost first: model index n-1 at the top.
        (0..n).map(|k| (n - 1 - k, Rect::from_min_size(pos2(0.0, k as f32 * 32.0), vec2(240.0, 32.0)))).collect()
    }

    #[test]
    fn drop_slots_map_to_model_indices() {
        let r = rows(3);
        // Drag the bottom item (model 0, shown last) above the top one: it becomes the top (2).
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 4.0), true, None, &[]), Some((2, 0.0, false)));
        // Between the first and second rows: model 1.
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 40.0), true, None, &[]), Some((1, 32.0, false)));
        // Around itself: no move.
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 70.0), true, None, &[]), None);
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 95.0), true, None, &[]), None);
        // The top item to the bottom: model 0.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 95.0), true, None, &[]), Some((0, 96.0, false)));
        // Outside the list, or an item it doesn't show.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 200.0), true, None, &[]), None);
        assert_eq!(drop_slot(&r, 2, pos2(400.0, 95.0), true, None, &[]), None);
        assert_eq!(drop_slot(&r, 7, pos2(100.0, 4.0), true, None, &[]), None);
        assert_eq!(drop_slot(&[], 0, pos2(0.0, 0.0), true, None, &[]), None);
    }

    #[test]
    fn a_given_area_takes_drops_over_the_whole_group() {
        // Rows 0..96 with a caption above them (the group's header) and room below.
        let r = rows(3);
        let area = Rect::from_min_max(pos2(0.0, -32.0), pos2(240.0, 128.0));
        // Over the caption, far above the first row: the head of the list.
        assert_eq!(drop_slot(&r, 0, pos2(100.0, -20.0), false, Some(area), &[]), Some((0, 0.0, false)));
        // Well below the last row: the tail.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 120.0), false, Some(area), &[]), Some((2, 96.0, false)));
        // The area still bounds the drop: just outside it nothing happens.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 140.0), false, Some(area), &[]), None);
        // Dropping a row where it already is stays put, however wide the area.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, -20.0), false, Some(area), &[]), None);
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 120.0), false, Some(area), &[]), None);
    }

    #[test]
    fn lists_in_model_order_drop_at_the_shown_slot() {
        // Shown in model order: model index k at row k.
        let r: Vec<(usize, Rect)> = (0..3).map(|k| (k, Rect::from_min_size(pos2(0.0, k as f32 * 32.0), vec2(240.0, 32.0)))).collect();
        // The first item below the last: model 2.
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 95.0), false, None, &[]), Some((2, 96.0, false)));
        // The last item above the first: model 0.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 4.0), false, None, &[]), Some((0, 0.0, false)));
        // The first between the second and third: model 1.
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 70.0), false, None, &[]), Some((1, 64.0, false)));
        assert_eq!(drop_slot(&r, 1, pos2(100.0, 40.0), false, None, &[]), None);
    }

    /// Four rows with a group's header (24 pt) between the second and the third: `breaks` holds
    /// the shown index of the first row under the header.
    fn grouped() -> (Vec<(usize, Rect)>, [usize; 1]) {
        let row = |i: usize, y: f32| (i, Rect::from_min_size(pos2(0.0, y), vec2(240.0, 24.0)));
        (vec![row(0, 0.0), row(1, 24.0), row(2, 72.0), row(3, 96.0)], [2])
    }

    #[test]
    fn a_header_belongs_to_the_group_under_it_but_its_top_half_to_the_one_above() {
        let (r, breaks) = grouped();
        // The header spans 48..72: its top half is the end of the group above, its bottom half the
        // head of the group below — and the line sits on the row edge either way, not in between.
        assert_eq!(drop_slot(&r, 3, pos2(100.0, 55.0), false, None, &breaks), Some((2, 48.0, true)));
        assert_eq!(drop_slot(&r, 0, pos2(100.0, 65.0), false, None, &breaks), Some((1, 72.0, false)));
    }

    #[test]
    fn a_drop_across_a_header_moves_the_row_even_where_it_stays() {
        let (r, breaks) = grouped();
        // The last row of a group dropped on the next group's head: same place, other folder.
        assert_eq!(drop_slot(&r, 1, pos2(100.0, 65.0), false, None, &breaks), Some((1, 72.0, false)));
        // …and the first row of a group on the group above's tail.
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 55.0), false, None, &breaks), Some((2, 48.0, true)));
        // On their own side of the header they stay put.
        assert_eq!(drop_slot(&r, 1, pos2(100.0, 55.0), false, None, &breaks), None);
        assert_eq!(drop_slot(&r, 2, pos2(100.0, 65.0), false, None, &breaks), None);
    }
}
