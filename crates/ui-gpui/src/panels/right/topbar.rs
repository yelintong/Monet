//! 右栏顶栏：`ui-egui/src/panels/right.rs::right_panel` 顶部那 48 pt 一行 + Share 弹层。
//!
//! 照 egui 的顺序与几何还原：
//!
//! ```text
//! [ 头像(你) 头像(别人)… +N ▾ ] ……… [ ▶ ▾ ] [ Share ]
//! ```
//!
//! * 头像 24 pt（egui `R = 12.0`）、间距 20 pt（`STEP`）所以彼此压 4 pt；
//!   自己的头像在会话里点开是「Spotlight me / Stop spotlighting」，别人的点一下跟随、
//!   右键（主人）能移出会话 —— 全走 `collab.*` 命令。
//! * Present 是 42×28 的两段按钮：左边 ▶ 演示（`proto.present`），右边 ▾ 是菜单
//!   （Present / Preview / Close preview，与 egui 同一组命令）。
//! * Share 是强调色按钮（32 pt 高、圆角 6、`accent_on` 文字），点开的是 [`share_card`]：
//!   照 egui `share_popover` 的状态机（未开会话 / 分享中 / 主人 / 客人）给出真能用的动作，
//!   全部推 `collab.*` 命令，文案与 egui 一致。
//!
//! 数据来自 [`super::data::LiveData`]（每帧从 `Session::live()` 现读），UI 不另存状态。

use super::data::{LiveData, PanelData};
use crate::panels::{Cmd, RightFields};
use crate::widgets::{Icon, IconButton, pt, rgba, tokens};
use crate::canvas::geom::Color32;
use gpui::{
    AnyElement, App, Bounds, Div, IntoElement, MouseButton, MouseDownEvent, PathBuilder, Pixels, Point, SharedString, Window, div, prelude::*, px,
};
use serde_json::{Value, json};
use std::rc::Rc;

/// 顶栏高度（egui `allocate_exact_size(vec2(w, 48.0))`）。
pub(crate) const TOP_H: f32 = 48.0;
/// 头像半径（egui `const R: f32 = 12.0`）。
const R: f32 = 12.0;
/// 头像中心间距（egui `const STEP: f32 = 20.0`）。
const STEP: f32 = 20.0;
/// 最多并排画几个别人的头像；多出来的收进 `+N`（egui 按可用宽度算，GPUI 面板宽度固定）。
const MAX_SHOWN: usize = 4;

/// 顶栏整行。
pub(crate) fn top_bar(f: &RightFields, d: &PanelData) -> Div {
    let t = tokens();
    div()
        .flex_none()
        .w_full()
        .h(pt(TOP_H))
        .flex()
        .flex_row()
        .items_center()
        .pl(pt(14.0))
        .pr(pt(14.0))
        // 左：people（你 + 别人 + 人数菜单）。
        .child(people(f, &d.live))
        // 右：Present + Share。
        .child(div().flex_1())
        .child(present(f))
        .child(div().w(pt(8.0)))
        .child(share_button(f, &d.live, &t))
}

/// 左边那一串头像 + 人数菜单的 chevron。
fn people(f: &RightFields, live: &LiveData) -> Div {
    let t = tokens();
    let me_fill = live.me_color.as_deref().map(parse_color).unwrap_or(t.success);
    let shown = live.peers.len().min(MAX_SHOWN);
    let more = live.peers.len().saturating_sub(shown);

    let mut row = div().flex().flex_row().items_center();
    // 自己：会话里点开是 Spotlight 菜单（egui `live.rs::people_bar` 的 `live_me`）。
    let me = avatar(&live.me_name, me_fill, live.spotlighting.then_some(me_fill));
    if live.active {
        let cmds = f.cmds.clone();
        let root = f.root.clone();
        let spotlighting = live.spotlighting;
        row = row.child(div().id("right-me-avatar").cursor_pointer().on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            let p = window.mouse_position();
            let anchor = Bounds { origin: p, size: gpui::size(px(1.0), px(0.0)) };
            let label = if spotlighting { "Stop spotlighting" } else { "Spotlight me" };
            let rows = vec![crate::widgets::MenuRow::new(label)];
            let cmds = cmds.clone();
            let on = !spotlighting;
            let on_pick = Rc::new(move |_: &str, _w: &mut Window, _cx: &mut App| {
                cmds.push(Cmd { id: "collab.spotlight", params: Some(json!({ "on": on })) });
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
        }).child(me));
    } else {
        row = row.child(me);
    }
    for (id, name, color) in live.peers.iter().take(shown) {
        let fill = parse_color(color);
        // 跟随中的那个头像外圈换成他的颜色（egui `circle_stroke(c, R + 2.5, …)`）。
        let ring = (live.follow == Some(*id)).then_some(fill);
        row = row.child(div().ml(pt(STEP - 2.0 * R)).flex().flex_row().items_center().child(avatar(name, fill, ring)));
    }
    if more > 0 {
        row = row.child(
            div()
                .ml(pt(STEP - 2.0 * R))
                .size(pt(2.0 * R))
                .rounded_full()
                .bg(rgba(t.pressed))
                .flex()
                .items_center()
                .justify_center()
                .text_size(pt(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.text))
                .child(SharedString::from(format!("+{more}"))),
        );
    }

    // 人数菜单：你 + 每个人（点一行 = 跟随 / 取消跟随）。
    // 名字是兜底占位（"You"）时不再拼「(you)」，否则读起来是「You (you)」。
    let me_label = if live.me_name == "You" { "You".to_string() } else { format!("{} (you)", live.me_name) };
    let rows: Vec<crate::widgets::MenuRow> = std::iter::once(crate::widgets::MenuRow::new(me_label))
        .chain(live.peers.iter().map(|(id, name, _)| {
            let mut r = crate::widgets::MenuRow::new(name.clone());
            if live.follow == Some(*id) {
                r = r.trail("Following");
            }
            r
        }))
        .collect();
    let ids: Vec<Option<u32>> = std::iter::once(None).chain(live.peers.iter().map(|(id, _, _)| Some(*id))).collect();
    let cmds = f.cmds.clone();
    let root = f.root.clone();
    let menu_rows = rows.clone();
    let chevron = IconButton::new(Icon::ChevronDown).size(24.0).on_click(move |_e, window, cx: &mut App| {
        let Some(root) = root.clone() else { return };
        let p = window.mouse_position();
        // `widgets::menu` 对零尺寸锚点直接不画，所以给 1×0 的锚点（贴在指针下方 4 pt）。
        let anchor = Bounds { origin: p, size: gpui::size(px(1.0), px(0.0)) };
        let ids = ids.clone();
        let cmds = cmds.clone();
        let rows = rows.clone();
        let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
            let Some(pos) = rows.iter().position(|r| r.label.as_ref() == label) else { return };
            let Some(client) = ids.get(pos).copied().flatten() else { return };
            cmds.push(Cmd { id: "collab.follow", params: Some(json!({ "client": client })) });
        });
        root.update(cx, |cv, cx| cv.open_menu(anchor, Some(220.0), menu_rows.clone(), on_pick, cx));
    });
    row.child(div().ml(pt(2.0))).child(chevron)
}

/// 一个头像：24 pt 的圆 + 首字母（egui `avatar()`：`initials` + `text_on(fill)`）。
/// `ring` 有值时外圈换成那个颜色（跟随中 / 会话里自己）。
fn avatar(name: &str, fill: Color32, ring: Option<Color32>) -> Div {
    let t = tokens();
    div()
        .flex_none()
        .size(pt(2.0 * R))
        .rounded_full()
        .bg(rgba(fill))
        .border_1()
        .border_color(rgba(ring.unwrap_or(t.panel)))
        .flex()
        .items_center()
        .justify_center()
        .text_size(pt(9.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgba(ink_on(fill)))
        .child(SharedString::from(figcraft_engine::live::initials(name)))
}

/// Present：▶（`proto.present`）+ ▾（Present / Preview / Close preview）。
fn present(f: &RightFields) -> Div {
    let t = tokens();
    let play_cmds = f.cmds.clone();
    let play = div()
        .id("right-present-play")
        .w(pt(26.0))
        .h(pt(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _w, _cx| {
            play_cmds.push(Cmd { id: "proto.present", params: Some(json!({})) });
        })
        .child(play_glyph(14.0, t.icon));
    let menu_cmds = f.cmds.clone();
    let root = f.root.clone();
    let menu = div()
        .id("right-present-menu")
        .w(pt(16.0))
        .h(pt(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.hover)))
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            let p = window.mouse_position();
            // `widgets::menu` 对零尺寸锚点直接不画，所以给 1×0 的锚点（贴在指针下方 4 pt）。
        let anchor = Bounds { origin: p, size: gpui::size(px(1.0), px(0.0)) };
            let rows = vec![
                crate::widgets::MenuRow::new("Present").trail(crate::main_menu::shortcut_text("proto.present").unwrap_or_default()),
                crate::widgets::MenuRow::new("Preview"),
                crate::widgets::MenuRow::new("Close preview"),
            ];
            let cmds = menu_cmds.clone();
            let on_pick = Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                let cmd = match label {
                    "Present" => "proto.present",
                    "Preview" => "proto.preview",
                    "Close preview" => "proto.closePreview",
                    _ => return,
                };
                cmds.push(Cmd { id: cmd, params: Some(json!({})) });
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(200.0), rows, on_pick, cx));
        })
        .child(crate::widgets::icon::paint(Icon::ChevronDown, rgba(t.text_secondary), 10.0));
    div().flex().flex_row().items_center().child(play).child(menu)
}

/// Share 按钮：强调色、32 pt 高、圆角 6，宽度跟着文字（egui 同款）。
fn share_button(f: &RightFields, live: &LiveData, t: &crate::theme::Tokens) -> gpui::Stateful<Div> {
    let root = f.root.clone();
    let width = if live.active { 64.0 } else { 56.0 };
    div()
        .id("right-share")
        .h(pt(32.0))
        .min_w(pt(width))
        .px(pt(12.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(6.0))
        .bg(rgba(t.accent))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.accent_hover)))
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _w, cx: &mut App| {
            let Some(root) = root.clone() else { return };
            // `widgets::menu` 对「零尺寸锚点」直接返回 None（不画），所以这里给 1×0 的锚点。
            // y 固定到顶栏下沿（egui 的 popover 贴在 Share 按钮下面，不盖住按钮本身）。
            let origin = Point { x: e.position.x, y: px(TOP_H - 4.0) };
            let anchor = Bounds { origin, size: gpui::size(px(1.0), px(0.0)) };
            root.update(cx, |cv, cx| cv.open_share(anchor, cx));
        })
        .child(
            div()
                .text_size(pt(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(t.accent_on))
                .child("Share"),
        )
}

/// 一个填充的 ▶（egui 用 `closed_line` 画的三角形；GPUI 这里自绘三角形，不新增图标）。
fn play_glyph(size: f32, color: Color32) -> impl IntoElement {
    let fill = rgba(color);
    gpui::canvas(
        move |bounds, _window, _cx| bounds,
        move |bounds, _rl, window, _cx| {
            let o = bounds.origin;
            let s = f32::from(bounds.size.width) / 16.0;
            let at = |x: f32, y: f32| Point::new(px(f32::from(o.x) + x * s), px(f32::from(o.y) + y * s));
            let mut p = PathBuilder::fill();
            p.add_polygon(&[at(3.0, 2.0), at(13.0, 8.0), at(3.0, 14.0)], true);
            if let Ok(path) = p.build() {
                window.paint_path(path, fill);
            }
        },
    )
    .size(pt(size))
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// Share 弹层（egui `live.rs::share_popover` 的状态机）
// ───────────────────────────────────────────────────────────────────────────────────────────────

/// Share 弹层的内容。照 egui `share_popover`：标题 + 状态行 + 按状态给出的动作。
///
/// 与 egui 的差异只在「控件形态」：egui 是一整块带输入框的卡片，GPUI 这里是同一张
/// `menu` 卡片里的行按钮（输入框那套 join/端口/密钥属于高级设置，尚未搬）。
pub(crate) fn share_card(anchor: Bounds<Pixels>, live: &LiveData, cmds: &crate::panels::Cmds, dismiss: Rc<dyn Fn(&mut Window, &mut App)>) -> AnyElement {
    let t = tokens();
    let title = if live.active { "Live session" } else { "Share this file" };
    let mut col = div().flex().flex_col().gap(pt(2.0)).w(pt(284.0));
    col = col.child(
        div()
            .px(pt(8.0))
            .pt(pt(4.0))
            .pb(pt(2.0))
            .text_size(pt(12.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgba(t.menu_text))
            .child(title),
    );
    if let Some(line) = status_line(&live) {
        col = col.child(
            div()
                .px(pt(8.0))
                .pb(pt(4.0))
                .text_size(pt(11.0))
                .text_color(rgba(if live.status == "failed" { t.danger } else { t.menu_dim }))
                .child(SharedString::from(line)),
        );
    }

    let push = |cmd: &'static str, params: Value| {
        let cmds = cmds.clone();
        let dismiss = dismiss.clone();
        move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cmds.push(Cmd { id: cmd, params: Some(params.clone()) });
            dismiss(window, cx);
        }
    };
    // ── 不在会话里：开分享 ───────────────────────────────────────────────────────
    if !live.active {
        if live.can_host {
            col = col.child(menu_row("Share on your network", None, push("collab.share", json!({ "via": "local" }))));
        }
        if live.server_set {
            col = col.child(menu_row("Share online", None, push("collab.share", json!({ "via": "online" }))));
        }
        if !live.can_host && !live.server_set {
            col = col.child(note("Sharing is off in this build."));
        }
    } else if live.shared {
        // ── 分享中：码 / 链接 / 停止 ─────────────────────────────────────────────
        if let Some(code) = &live.code {
            col = col.child(
                div()
                    .px(pt(8.0))
                    .py(pt(6.0))
                    .text_size(pt(if live.online { 20.0 } else { 15.0 }))
                    .text_color(rgba(t.menu_text))
                    .child(SharedString::from(code.clone())),
            );
        }
        if let Some(code) = live.code.clone() {
            col = col.child(copy_row("Copy code", &code, cmds, &dismiss));
        }
        if let Some(link) = live.link.clone() {
            col = col.child(copy_row("Copy link", &link, cmds, &dismiss));
        }
        col = col.child(menu_row("Stop sharing", None, push("collab.stop", json!({}))));
    } else if live.host {
        // ── 主人、没在分享码：给链接，能停会话 ───────────────────────────────────
        if let Some(link) = live.info_link.clone() {
            col = col.child(copy_row("Copy invite link", &link, cmds, &dismiss));
        }
        col = col.child(menu_row("Stop session", None, push("collab.stop", json!({}))));
    } else {
        // ── 客人：说明能做什么，能离开 ──────────────────────────────────────────
        if live.view_only {
            col = col.child(note("View only · your edits are refused"));
        }
        col = col.child(menu_row("Leave session", None, push("collab.leave", json!({}))));
    }

    // 底部：egui 的 Export… 行还没搬（GPUI 没有导出预览窗口），这里就不放假按钮。
    crate::widgets::menu(anchor)
        .width(crate::widgets::Width::Min(300.0))
        .show(|| col.into_any_element())
        .unwrap_or_else(|| div().into_any_element())
}

/// 弹层里的状态行（egui `share_popover` 里那个 `(line, col)` 的 `line`）。
fn status_line(live: &LiveData) -> Option<String> {
    let others = live.peers.len();
    let people = match others {
        0 => "nobody else here yet".to_string(),
        1 => "1 person joined".to_string(),
        n => format!("{n} people joined"),
    };
    let line = match (live.status, live.shared, live.host, live.view_only) {
        ("off", ..) => String::new(),
        ("failed", ..) => live.reason.clone().unwrap_or_else(|| "Couldn't connect".into()),
        ("connecting", ..) => "Starting to share…".to_string(),
        ("reconnecting", ..) => "Connection lost, reconnecting…".to_string(),
        ("live", false, true, _) => format!("Live · {people}"),
        ("live", false, false, _) => "Joined".to_string(),
        ("live", true, _, _) if live.online => format!("Sharing online · {people}"),
        ("live", true, ..) => format!("Sharing on your network · {people}"),
        _ => String::new(),
    };
    (!line.is_empty()).then_some(line)
}

/// 一行可点的菜单行（28 pt，悬停高亮）。
fn menu_row(label: &'static str, trail: Option<String>, on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let t = tokens();
    div()
        .h(pt(28.0))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .px(pt(8.0))
        .rounded(pt(5.0))
        .text_size(pt(crate::widgets::TEXT_PT))
        .text_color(rgba(t.menu_text))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(t.menu_hover)))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(div().flex_1().min_w(pt(0.0)).whitespace_nowrap().child(label))
        .when_some(trail, |d, tr| {
            d.child(div().ml(pt(8.0)).text_color(rgba(t.menu_dim)).whitespace_nowrap().child(SharedString::from(tr)))
        })
}

/// 「Copy …」一行：点了把 `text` 放进剪贴板再关掉弹层。
fn copy_row(label: &'static str, text: &str, cmds: &crate::panels::Cmds, dismiss: &Rc<dyn Fn(&mut Window, &mut App)>) -> impl IntoElement {
    let text = text.to_string();
    let dismiss = dismiss.clone();
    let cmds = cmds.clone();
    menu_row(label, None, move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
        cmds.wake();
        dismiss(window, cx);
    })
}

/// 弹层里的一行说明文字。
fn note(text: &str) -> impl IntoElement {
    let t = tokens();
    div()
        .px(pt(8.0))
        .py(pt(4.0))
        .text_size(pt(11.0))
        .text_color(rgba(t.menu_dim))
        .child(SharedString::from(text.to_string()))
}

/// `#RRGGBB` → `Color32`（会话里的人色；解析不了就退回中灰）。
fn parse_color(s: &str) -> Color32 {
    figcraft_color::Color::from_hex_or_name(s)
        .map(crate::widgets::swatch::color32)
        .unwrap_or(Color32::from_rgb(0x9B, 0x9B, 0x9B))
}

/// 头像上的字色：底色亮就用黑字（egui `text_on`）。
fn ink_on(fill: Color32) -> Color32 {
    let [r, g, b, _] = fill.0;
    let lum = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if lum > 140.0 { Color32::BLACK } else { Color32::WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(status: &'static str, shared: bool, host: bool) -> LiveData {
        LiveData { status, shared, host, ..LiveData::default() }
    }

    /// egui `live.rs::share_popover` 的状态行：同一组状态必须给出同一句话，
    /// 否则「分享中 / 未分享 / 已加入」在弹层里会读不出来。
    #[test]
    fn the_status_line_follows_the_session_state() {
        assert_eq!(status_line(&live("off", false, false)), None);
        assert_eq!(status_line(&live("connecting", false, true)).as_deref(), Some("Starting to share…"));
        assert_eq!(status_line(&live("reconnecting", false, false)).as_deref(), Some("Connection lost, reconnecting…"));
        assert_eq!(status_line(&live("live", false, true)).as_deref(), Some("Live · nobody else here yet"));
        assert_eq!(status_line(&live("live", true, true)).as_deref(), Some("Sharing on your network · nobody else here yet"));
        assert_eq!(status_line(&live("live", false, false)).as_deref(), Some("Joined"));
    }

    /// 在线分享用另一句话（egui 的 `Sharing online · {people}`）。
    #[test]
    fn online_sharing_says_so() {
        let mut l = live("live", true, true);
        l.online = true;
        l.peers = vec![(7, "Ada".into(), "#0D99FF".into())];
        assert_eq!(status_line(&l).as_deref(), Some("Sharing online · 1 person joined"));
    }

    /// 头像上的字母是 `live::initials` 的（和 egui 同一个函数），空名字也有兜底。
    #[test]
    fn avatar_ink_is_readable_on_the_session_colour() {
        assert_eq!(ink_on(Color32::from_rgb(0x0D, 0x99, 0xFF)), Color32::WHITE);
        assert_eq!(ink_on(Color32::from_rgb(0xFF, 0xFF, 0xFF)), Color32::BLACK);
        assert_eq!(parse_color("#0D99FF"), Color32::from_rgb(0x0D, 0x99, 0xFF));
        assert_eq!(parse_color("not a colour"), Color32::from_rgb(0x9B, 0x9B, 0x9B));
    }
}
