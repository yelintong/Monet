//! Comments — the Comments panel and the canvas pins.
//!
//! Structure copied from `crates/ui-egui/src/canvas/comments.rs`:
//!   `CommentUi` (filter / sort / search / open thread) · `panel()` (the list) · `draw()` (pins).
//!
//! All data and commands come from `figcraft_engine::cmd::comment` (`Filter`, `Sort`, `Query`,
//! `query`, `frame_of`, `user_name`) and the document's own `Comment` type — nothing here invents
//! a command or a widget.

use crate::panels::{Cmd, RightFields};
use crate::widgets::{Dropdown, MenuRow, rgba, tokens};
use figcraft_doc::Document;
use figcraft_engine::cmd::comment::{Filter, Query, Sort, frame_of, query};
use gpui::{App, Div, IntoElement, SharedString, Window, div, prelude::*, px};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// The Comments panel's local state (egui `CommentUi`): which threads show, how they are ordered,
/// the search text, and the thread open in a popover.
#[derive(Clone, Debug)]
pub(crate) struct CommentUi {
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    pub open: Option<u64>,
}

impl Default for CommentUi {
    fn default() -> Self {
        // egui starts on Open (`app.ui.comments.filter`), newest activity first.
        Self { filter: Filter::Open, sort: Sort::Date, search: String::new(), open: None }
    }
}

/// One row of the panel's list, formatted once per frame in `CanvasView::sync_panel`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommentRow {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub ago: String,
    pub unread: bool,
    /// `frame · N replies · Resolved` (egui's `meta.join(" · ")`).
    pub meta: String,
}

/// The comments `filter` / `sort` / `search` keep, formatted for the panel.
pub(crate) fn read_rows(doc: &Document, me: &str, filter: Filter, sort: Sort, search: &str) -> Vec<CommentRow> {
    let q = Query { page: None, frame: None, filter, sort, text: search };
    query(doc, &q, me)
        .into_iter()
        .map(|c| {
            let mut meta = Vec::new();
            if let Some(f) = frame_of(doc, c).and_then(|f| doc.node(f)) {
                meta.push(f.name.clone());
            }
            if !c.replies.is_empty() {
                meta.push(format!("{} replies", c.replies.len()));
            }
            if c.resolved {
                meta.push("Resolved".to_string());
            }
            CommentRow {
                id: c.id,
                author: c.first.author.clone(),
                body: c.first.text.clone(),
                ago: ago(c.last_activity()),
                unread: c.is_unread(me),
                meta: meta.join(" · "),
            }
        })
        .collect()
}

/// The Comments panel body (egui `comments::panel`): title + filter/sort menu, then the list, or
/// the empty state.
pub(crate) fn panel(f: &RightFields, rows: &[CommentRow], total: usize) -> Div {
    let t = tokens();
    let cu = f.comment_ui().borrow().clone();
    let mut body = div().flex().flex_col().w_full();
    // Header: "Comments" on the left, the filter/sort menu on the right (egui `Filter` + `Sort`).
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(40.0))
            .px(px(16.0))
            .child(div().flex_1().text_size(px(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child("Comments"))
            .child(filter_button(f, cu.filter, cu.sort)),
    );
    if rows.is_empty() {
        let (title, sub) = if total == 0 {
            ("No comments yet", "Click anywhere on the canvas to leave one.")
        } else {
            ("No matching comments", "Try another search or filter.")
        };
        return body.child(
            div()
                .px(px(16.0))
                .py(px(24.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(px(12.0)).text_color(rgba(t.text_secondary)).child(title))
                .child(div().text_size(px(11.0)).text_color(rgba(t.text_tertiary)).child(sub)),
        );
    }
    for row in rows {
        body = body.child(comment_row(f, row, cu.open == Some(row.id)));
    }
    body
}

/// One thread row: unread dot, avatar, author + age, body, meta (egui's 52 + 16·lines row).
fn comment_row(f: &RightFields, row: &CommentRow, open: bool) -> impl IntoElement {
    let t = tokens();
    let cmds = f.queue();
    let id = row.id;
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(rgba(t.border))
        .when(open, |d| d.bg(rgba(t.selected_row)))
        .cursor_pointer()
        .id(gpui::ElementId::from(SharedString::from(format!("comment-row-{id}"))))
        .on_click(move |_, _, _| cmds.push(Cmd { id: "comment.open", params: Some(json!({ "id": id })) }))
        // Unread dot (egui draws it at x+8; the column keeps the avatar aligned).
        .child(
            div()
                .w(px(8.0))
                .flex_none()
                .flex()
                .justify_center()
                .pt(px(6.0))
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().when(row.unread, |d| d.bg(rgba(t.accent)))),
        )
        .child(avatar(&row.author))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(div().flex_1().text_size(px(12.0)).text_color(rgba(t.text)).child(row.author.clone()))
                        .child(div().text_size(px(11.0)).text_color(rgba(t.text_tertiary)).child(row.ago.clone())),
                )
                .child(div().text_size(px(12.0)).text_color(rgba(t.text)).child(row.body.clone()))
                .when(!row.meta.is_empty(), |d| d.child(div().text_size(px(11.0)).text_color(rgba(t.text_secondary)).child(row.meta.clone()))),
        )
}

/// A round avatar with the author's initial (egui `avatar` + `author_color`).
fn avatar(name: &str) -> Div {
    let color = author_color(name);
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .w(px(24.0))
        .h(px(24.0))
        .flex_none()
        .rounded_full()
        .bg(rgba(color))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.0))
        .text_color(rgba(crate::canvas::geom::Color32::WHITE))
        .child(initial)
}

/// The filter/sort button: a dropdown whose menu sets the local filter/sort or marks all read
/// (egui's `Adjust` icon + `menu_popup` with `Filter::ALL`, `Sort::ALL`, "Mark all as read").
fn filter_button(f: &RightFields, filter: Filter, sort: Sort) -> Dropdown {
    let rows: Vec<MenuRow> = {
        let mut v: Vec<MenuRow> = Filter::ALL.iter().map(|x| MenuRow::new(x.label()).checked(Some(*x == filter))).collect();
        v.extend(Sort::ALL.iter().map(|x| MenuRow::new(x.label()).separator().checked(Some(*x == sort))));
        v.push(MenuRow::new("Mark all as read"));
        v
    };
    let mut d = Dropdown::new(filter.label(), 0.0, None).fill();
    if let Some(root) = f.root() {
        let ui = f.comment_ui();
        let cmds = f.queue();
        d = d.on_click(move |anchor, _window, cx: &mut App| {
            let ui = ui.clone();
            let cmds = cmds.clone();
            let menu = rows.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(f) = Filter::ALL.iter().find(|x| x.label() == label) {
                    ui.borrow_mut().filter = *f;
                } else if let Some(s) = Sort::ALL.iter().find(|x| x.label() == label) {
                    ui.borrow_mut().sort = *s;
                } else if label == "Mark all as read" {
                    cmds.push(Cmd { id: "comment.markRead", params: Some(json!({})) });
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), menu, on_pick, cx));
        });
    }
    d
}

/// Pins on the canvas: the current page's threads, the open one on top (egui `pins` + `paint_pin`).
pub(crate) fn draw(app: &crate::CanvasApp, painter: &mut crate::canvas::painter::Painter, t: &crate::theme::Tokens) {
    let Ok(ds) = app.session.ds() else { return };
    let me = figcraft_engine::cmd::comment::user_name(&app.session);
    let q = Query { page: Some(ds.page), ..Query::default() };
    let mut list = query(&ds.doc, &q, &me);
    list.reverse();
    for c in list {
        let (x, y) = c.position(&ds.doc);
        let at = app.to_screen(x, y);
        let size = 28.0;
        let rect =
            crate::canvas::geom::Rect::from_min_max(crate::canvas::geom::pos2(at.x, at.y - size), crate::canvas::geom::pos2(at.x + size, at.y));
        let fill = if c.resolved { t.text_tertiary } else { t.accent };
        painter.rect_filled(rect, crate::canvas::geom::CornerRadius::same(8), fill);
        painter.text(
            crate::canvas::geom::pos2(rect.min.x + size / 2.0, rect.min.y + size / 2.0),
            crate::canvas::geom::Align2::CENTER_CENTER,
            c.id.to_string(),
            crate::canvas::geom::regular(12.0),
            crate::canvas::geom::Color32::WHITE,
        );
    }
}

/// A stable colour per author (egui `author_color`): the name's hash into a small palette.
pub(crate) fn author_color(name: &str) -> crate::canvas::geom::Color32 {
    const PALETTE: [&str; 6] = ["#4C8DFF", "#E5484D", "#30A46C", "#F76B15", "#8E4EC6", "#E93D82"];
    let mut h: u64 = 0;
    for b in name.bytes() {
        h = h.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    let hex = PALETTE[(h % PALETTE.len() as u64) as usize];
    crate::canvas::geom::Color32::from_hex(hex).unwrap_or(crate::canvas::geom::Color32::WHITE)
}

/// `now` / `3m` / `2h` / `5d` for a Unix timestamp (egui `ago`).
pub(crate) fn ago(created: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let d = now.saturating_sub(created);
    if created == 0 {
        String::new()
    } else if d < 60 {
        "now".to_string()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

/// A local UI-state cell for the comment panel (used by `RightFields`).
pub(crate) type CommentState = Rc<RefCell<CommentUi>>;
