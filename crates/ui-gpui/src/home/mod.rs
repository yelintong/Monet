//! The Home screen (A1): a 240 pt sidebar (wordmark, search, Recents / Online / Templates) and the
//! main area (action cards, recent files, Getting started).
//!
//! Structure copied from `crates/ui-egui/src/home.rs`. The `Recents` grid/sort and the Online
//! section are the parts of `home/home_recents.rs` / `home/online.rs` still to come; this file is
//! where they land.

use figcraft_i18n::tr_noop;
use crate::CanvasApp;
use crate::panels::extra::ExtraState;
use crate::panels::{Cmd, Cmds};
use crate::widgets::{Icon, pt, rgba, tokens};
use figcraft_i18n::tr;
use gpui::{Div, MouseButton, SharedString, div, prelude::*, px};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// `ui-egui/src/home.rs::TIPS`: the six Getting-started rows (title, one-line hint).
const HOME_TIPS: [(&str, &str); 6] = [
    (tr_noop("Draw a frame"), tr_noop("Pick the Frame tool, then drag on the canvas or choose a device size")),
    (tr_noop("Add auto layout"), tr_noop("Select layers and they space themselves as their content changes")),
    (tr_noop("Make a component"), tr_noop("Turn a layer into a component, reuse it, and change every copy at once")),
    (tr_noop("Prototype"), tr_noop("Link frames on the Prototype tab, then present the flow")),
    (tr_noop("Share with a code"), tr_noop("Start sharing with the Share button, then pass on the code it gives")),
    (tr_noop("Export"), tr_noop("Export frames and layers as PNG, JPG, SVG or PDF")),
];

/// The whole Home screen: sidebar + main.
pub(crate) fn body(app: &CanvasApp, cmds: &Cmds, state: &ExtraState) -> Div {
    let t = tokens();
    let section = *state.home_section.borrow();
    let q = state.home_query.borrow().to_lowercase();

    // ── 240 pt sidebar: wordmark, file/template search, three section tabs ──────
    let mut side = div().flex_none().w(px(240.0)).h_full().flex().flex_col().gap(pt(8.0)).p(pt(12.0)).border_r_1().border_color(rgba(t.border));
    side = side.child(
        div()
            .h(px(40.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(10.0))
            .child(crate::widgets::icon::paint(Icon::Logo, rgba(t.accent), 24.0))
            .child(div().text_size(pt(16.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("FigCraft"))),
    );
    side = side.child(state.home_search.clone());
    for (i, (icon, label)) in [(Icon::Page, tr("Recents")), (Icon::LinkOn, tr("Online")), (Icon::ViewGrid, tr("Templates"))].into_iter().enumerate() {
        let on = section == i;
        let slot = state.home_section.clone();
        let wake = cmds.clone();
        side = side.child(
            div()
                .h(px(32.0))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(pt(10.0))
                .px(pt(10.0))
                .rounded(pt(5.0))
                .bg(rgba(if on { t.selected_row } else { t.panel }))
                .hover(|s| s.bg(rgba(t.hover)))
                .cursor_pointer()
                .child(crate::widgets::icon::paint(icon, rgba(t.icon), 16.0))
                .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(tr(label)))
                .on_mouse_down(MouseButton::Left, move |_e, _, _| {
                    *slot.borrow_mut() = i;
                    wake.wake();
                }),
        );
    }

    // ── main: heading + cards + files + tips, scrolled like the egui CentralPanel ─
    let mut main = div().id("home-main").flex_1().min_w(px(0.0)).h_full().flex().flex_col().gap(pt(16.0)).p(pt(24.0)).overflow_y_scroll();
    match section {
        2 => {
            main = main.child(heading(tr("Templates")));
            let mut row = div().flex().flex_row().flex_wrap().gap(pt(12.0));
            row = row.child(action_card(Icon::Frame, tr("New design file"), tr("Start from a blank canvas"), "file.new", cmds));
            row = row.child(action_card(Icon::Rectangle, tr("New board"), tr("Sticky notes, shapes and connectors"), "file.newBoard", cmds));
            row = row.child(action_card(Icon::ViewGrid, tr("Start from a template"), tr("Start from a template"), "file.newFromTemplate", cmds));
            main = main.child(row);
        }
        1 => {
            main = main.child(heading(tr("Online")));
            main = main.child(note(tr("Online files come from the engine's online section; this front-end has no local list yet.")));
        }
        _ => {
            main = main.child(heading(tr("Recents")));
            let mut cards = div().flex().flex_row().flex_wrap().gap(pt(12.0));
            cards = cards.child(action_card(Icon::Frame, tr("New design file"), tr("Start from a blank canvas"), "file.new", cmds));
            cards = cards.child(action_card(Icon::Rectangle, tr("New board"), tr("Sticky notes, shapes and connectors"), "file.newBoard", cmds));
            cards = cards.child(action_card(Icon::Page, tr("Open file"), tr("Open .figcraft, .svg or images"), "file.open", cmds));
            main = main.child(cards);
            if !app.session.prefs.getting_started_dismissed {
                main = main.child(subheading(tr("Getting started")));
                let mut tips = div().w_full().flex().flex_col().gap(pt(8.0)).p(pt(16.0)).rounded(pt(8.0)).bg(rgba(t.field));
                for (i, (title, line)) in HOME_TIPS.iter().enumerate() {
                    tips =
                        tips.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(pt(10.0))
                                .child(div().size(pt(20.0)).rounded(pt(10.0)).bg(rgba(t.panel)).flex().items_center().justify_center().child(
                                    div().text_size(pt(10.0)).text_color(rgba(t.text_secondary)).child(SharedString::from((i + 1).to_string())),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(tr(title)))
                                        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(tr(line))),
                                ),
                        );
                }
                main = main.child(tips);
            }
            // Toolbar: "Recent files" + a grid/list toggle + a name/edited sort
            // (egui `home_recents::View` + `Sort`).
            let grid = *state.home_view_grid.borrow();
            main = main.child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(pt(8.0))
                    .child(
                        div().flex_1().text_size(pt(13.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("Recent files")),
                    )
                    .child(cycle_bool(if grid { tr("Grid") } else { tr("List") }, state.home_view_grid.clone(), cmds))
                    .child(cycle_u8(if *state.home_sort.borrow() == 0 { tr("Name") } else { tr("Edited") }, state.home_sort.clone(), 2, cmds)),
            );
            let mut files: Vec<String> =
                app.session.prefs.recent_files.iter().filter(|p| q.is_empty() || p.to_lowercase().contains(&q)).cloned().collect();
            if *state.home_sort.borrow() == 0 {
                files.sort_by_key(|p| std::path::Path::new(p).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default());
            } else {
                files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
            }
            if files.is_empty() {
                main = main.child(note(tr("No recent files yet. Open or save a file to see it here.")));
            } else if grid {
                let mut g = div().w_full().flex().flex_row().flex_wrap().gap(pt(12.0));
                for path in files.iter().take(12) {
                    g = g.child(recent_card(path, cmds));
                }
                main = main.child(g);
            } else {
                for path in files.iter().take(12) {
                    main = main.child(recent_row(path, cmds));
                }
            }
        }
    }
    div().flex().flex_row().w_full().h_full().child(side).child(main)
}

fn cycle_bool(label: &'static str, slot: Rc<RefCell<bool>>, cmds: &Cmds) -> Div {
    let t = tokens();
    let wake = cmds.clone();
    div()
        .h(px(24.0))
        .px(pt(8.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .bg(rgba(t.panel))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .text_size(pt(11.0))
        .text_color(rgba(t.text))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            // ⛔ 先读再写：`*slot.borrow_mut() = !*slot.borrow()` 是一条表达式，左侧的 `RefMut`
            // 还活着时右侧又借了一次，直接 panic（RefCell already borrowed），整个 app 会崩。
            let next = !*slot.borrow();
            *slot.borrow_mut() = next;
            wake.wake();
        })
}

fn cycle_u8(label: &'static str, slot: Rc<RefCell<u8>>, n: u8, cmds: &Cmds) -> Div {
    let t = tokens();
    let wake = cmds.clone();
    div()
        .h(px(24.0))
        .px(pt(8.0))
        .rounded(pt(4.0))
        .flex()
        .items_center()
        .bg(rgba(t.panel))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .text_size(pt(11.0))
        .text_color(rgba(t.text))
        .child(SharedString::from(label))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            // 同上：先读再写。
            let next = (*slot.borrow() + 1) % n.max(1);
            *slot.borrow_mut() = next;
            wake.wake();
        })
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn edited_ago(path: &str) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else { return String::new() };
    let Ok(since) = modified.elapsed() else { return String::new() };
    let d = since.as_secs();
    if d < 60 {
        tr("Just now").to_string()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

fn recent_card(path: &str, cmds: &Cmds) -> Div {
    let t = tokens();
    let name = file_name(path);
    let edited = edited_ago(path);
    let c = cmds.clone();
    let p = path.to_string();
    div()
        .flex_none()
        .w(px(200.0))
        .h(px(154.0))
        .flex()
        .flex_col()
        .gap(pt(6.0))
        .p(pt(10.0))
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .child(
            div().w_full().h(px(96.0)).rounded(pt(6.0)).bg(rgba(t.panel)).flex().items_center().justify_center().child(crate::widgets::icon::paint(
                Icon::Page,
                rgba(t.icon),
                28.0,
            )),
        )
        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(SharedString::from(name)))
        .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(edited)))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.openRecent", params: Some(json!({ "path": p.clone() })) });
        })
}

fn recent_row(path: &str, cmds: &Cmds) -> Div {
    let t = tokens();
    let name = file_name(path);
    let c = cmds.clone();
    let p = path.to_string();
    div()
        .w_full()
        .h(px(36.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(10.0))
        .px(pt(10.0))
        .rounded(pt(5.0))
        .hover(|s| s.bg(rgba(t.hover)))
        .cursor_pointer()
        .child(crate::widgets::icon::paint(Icon::Page, rgba(t.icon), 16.0))
        .child(div().text_size(pt(12.0)).text_color(rgba(t.text)).child(SharedString::from(name)))
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.openRecent", params: Some(json!({ "path": p.clone() })) });
        })
}

fn note(text: &'static str) -> Div {
    let t = tokens();
    div().w_full().p(pt(16.0)).text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(text))
}

fn heading(text: &'static str) -> Div {
    div().w_full().text_size(pt(20.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(tokens().text)).child(SharedString::from(text))
}

fn subheading(text: &'static str) -> Div {
    div().w_full().text_size(pt(13.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(tokens().text)).child(SharedString::from(text))
}

/// `ui-egui/src/home.rs::action_card`: 260x72, a 32 pt accent icon tile, title + sub line.
fn action_card(icon: Icon, title: &'static str, sub: &'static str, id: &'static str, cmds: &Cmds) -> Div {
    let t = tokens();
    let c = cmds.clone();
    div()
        .flex_none()
        .w(px(260.0))
        .h(px(72.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(12.0))
        .px(pt(16.0))
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .child(
            div()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(pt(6.0))
                .bg(rgba(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::widgets::icon::paint(icon, rgba(t.accent_on), 16.0)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(pt(4.0))
                .child(div().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(title)))
                .child(div().text_size(pt(11.0)).text_color(rgba(t.text_secondary)).child(SharedString::from(sub))),
        )
        .on_mouse_down(MouseButton::Left, move |_e, _, _| c.push(Cmd { id, params: None }))
}
