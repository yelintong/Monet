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
            // Brand mark: the three anchor dots in red over the blue path (the mono version is
            // used by the rail / main menu). The renderer only keeps alpha, so each layer is
            // tinted separately instead of relying on the SVG's own colours.
            .child(
                gpui::svg()
                    .path("brand/monet-dots.svg")
                    .w(px(24.0))
                    .h(px(24.0))
                    .text_color(gpui::Hsla::from(gpui::rgb(0x007BE5))),
            )
            .child(div().text_size(pt(16.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(tr("Monet"))),
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
            // egui `home.rs::templates`：标题 + 卡片网格（248×234，缩略图 148 高），
            // 点一张就 `file.newFromTemplate`。以前这里是三张动作卡，根本不是模板。
            main = main.child(heading(tr("Templates")));
            let shown: Vec<&figcraft_engine::templates::Template> = figcraft_engine::templates::TEMPLATES
                .iter()
                .filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q) || x.description.to_lowercase().contains(&q))
                .collect();
            if shown.is_empty() {
                main = main.child(note(tr("No templates match your search")));
            } else {
                let mut grid = div().w_full().flex().flex_row().flex_wrap().gap(pt(20.0));
                for tpl in shown {
                    grid = grid.child(template_card(state, tpl, cmds));
                }
                main = main.child(grid);
            }
        }
        1 => {
            // egui `home/online.rs::section`：标题 + 说明 + 本机在线文件列表（空态另说）。
            main = main.child(heading(tr("Online files")));
            main = main.child(note(tr(
                "The files you shared or published online from this computer. They stay on their server until you remove them.",
            )));
            let files = &app.session.prefs.online_files;
            if files.is_empty() {
                main = main.child(note(tr("Nothing online yet. Use Share on a file to put it online and it is listed here.")));
            } else {
                let mut list = div().w_full().flex().flex_col();
                for f in files.iter().take(40) {
                    list = list.child(
                        div()
                            .w_full()
                            .h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(pt(10.0))
                            .px(pt(10.0))
                            .rounded(pt(5.0))
                            .child(crate::widgets::icon::paint(Icon::LinkOn, rgba(t.icon), 16.0))
                            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_size(pt(12.0)).text_color(rgba(t.text)).child(
                                SharedString::from(f.name.clone()),
                            ))
                            .child(div().text_size(pt(11.0)).text_color(rgba(t.text_tertiary)).child(SharedString::from(f.server.clone()))),
                    );
                }
                main = main.child(list);
            }
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

/// A template card (egui `home.rs::template_card`): a 148 pt thumbnail over its name and one
/// line about it, 248 × 234. Clicking it opens a new document made from the template.
fn template_card(state: &ExtraState, tpl: &figcraft_engine::templates::Template, cmds: &Cmds) -> gpui::Stateful<Div> {
    let t = tokens();
    let thumb = template_thumb(state, tpl);
    let c = cmds.clone();
    let id = tpl.id;
    let name = figcraft_i18n::tr(tpl.name).to_string();
    let name_for_cmd = name.clone();
    let mut card = div()
        .id(gpui::ElementId::from(SharedString::from(format!("home-tpl-{}", tpl.id))))
        .flex_none()
        .w(px(248.0))
        .h(px(234.0))
        .flex()
        .flex_col()
        .rounded(pt(8.0))
        .bg(rgba(t.field))
        .border_1()
        .border_color(rgba(t.border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, move |_e, _, _| {
            c.push(Cmd { id: "file.newFromTemplate", params: Some(json!({ "template": id, "name": name_for_cmd.clone() })) });
        });
    // The picture: the template drawn small, or its mark when it couldn't be drawn.
    let mut pic = div()
        .w_full()
        .h(px(148.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(t.panel))
        .border_b_1()
        .border_color(rgba(t.border));
    pic = match thumb {
        Some(tex) => pic.child(gpui::img(gpui::ImageSource::Render(tex)).max_w(px(226.0)).max_h(px(148.0))),
        None => pic.child(crate::widgets::icon::paint(Icon::Frame, rgba(t.icon), 32.0)),
    };
    card = card.child(pic).child(
        div()
            .flex()
            .flex_col()
            .gap(pt(4.0))
            .p(pt(12.0))
            .child(div().text_size(pt(12.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgba(t.text)).child(SharedString::from(name)))
            .child(
                div()
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text_secondary))
                    .child(SharedString::from(figcraft_i18n::tr(tpl.description).to_string())),
            ),
    );
    card
}

/// A template's thumbnail, drawn once and kept on the view (five documents' worth of rendering
/// is far too slow to do per frame).
fn template_thumb(state: &ExtraState, tpl: &figcraft_engine::templates::Template) -> Option<std::sync::Arc<gpui::RenderImage>> {
    if let Some(hit) = state.template_thumbs.borrow().get(tpl.id) {
        return hit.clone();
    }
    let drawn = draw_template_thumb(tpl);
    state.template_thumbs.borrow_mut().insert(tpl.id, drawn.clone());
    drawn
}

/// `ui-egui/src/home.rs::thumbnail`: the template built on a page of its own and drawn to fit
/// a 226 × 126 area, cropped to its top when it is much taller than it is wide.
fn draw_template_thumb(tpl: &figcraft_engine::templates::Template) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let doc = tpl.build(figcraft_color::Color::WHITE).ok()?;
    let frames = doc.node(doc.first_page()?)?.children.clone();
    let bounds = frames.iter().filter_map(|id| figcraft_render::export_bounds(&doc, *id)).reduce(|a, b| a.union(b))?;
    if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
        return None;
    }
    let (w, h) = (226.0f64, 126.0f64);
    let area = figcraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1.min(bounds.y0 + bounds.width() * h / w * 1.5));
    let scale = (w / area.width()).min(h / area.height());
    let pix = crate::render_stack::on_render_stack(|| figcraft_render::render_area(&doc, &frames, area, scale.min(1.0)))??;
    Some(crate::canvas::render_worker::screen_image(&pix, doc.color_profile))
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
