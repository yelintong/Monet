//! The floating bottom toolbar — a direct port of `ui-egui/src/toolbar.rs`.
//!
//! The pill sits 12 pt above the canvas's bottom edge, centred on the **canvas** (not the window:
//! the left rail and the two panels shift the canvas off centre). Its metrics are the original's:
//! a 48 pt pill, 32 pt cells 8 pt apart, 8 pt of padding, 13 pt corners, and a 16 pt flyout caret
//! one point off the button it belongs to.
//!
//! ```text
//! [Move▾] [Frame▾] [Shape▾] [Pen▾] [Text▾] [Comment]  [Actions] │ [Draw Design Animation Inspect]
//! ```
//!
//! Every tool group with more than one tool carries a caret; clicking it opens the group's flyout
//! *above* the button ([`crate::menu_card`]), which is the only way to reach the tools that are not
//! the group's shown one. Choosing one both runs `tool.set` and remembers it for that group, so the
//! caret's button keeps showing the last tool picked.
//!
//! Differences from the original, all because the port has no other front-end yet: boards are not
//! supported (the shell only ever opens the built-in sample document), so the board groups and
//! their swatch flyouts are not ported; the Inspect (Dev Mode) segment has no panel to switch to, so
//! it is drawn greyed rather than pretending to work.

use crate::menu_card::Item;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{pt, rgba, tokens};
use figcraft_engine::ToolId;
use gpui::{App, BoxShadow, Div, MouseButton, MouseDownEvent, SharedString, Stateful, Window, div, point, prelude::*, px};
use std::rc::Rc;

/// Every mark in the row is painted in a box this size, so they share one optical size.
const GLYPH: f32 = 18.0;
/// The flyout carets are smaller than the marks (the original's `CARET`).
const CARET: f32 = 14.0;
/// A tool cell's side.
pub(crate) const CELL: f32 = 32.0;
/// The caret's hit box: 16 wide, one point off its button.
const CARET_W: f32 = 16.0;
const CARET_GAP: f32 = 1.0;
/// Between the pill's top-level items (the divider included).
const GAP: f32 = 8.0;
/// The pill's left and right padding.
const PAD: f32 = 8.0;
/// The pill's height.
const PILL_H: f32 = 48.0;
/// The mode switcher's marks: smaller than the tools'.
const MODE_GLYPH: f32 = 14.0;
/// Draw · Design · Animation · Inspect.
const MODE_CELLS: usize = 4;
/// A flyout row's height (`ui-egui/src/toolbar.rs::flyout`: `item_h = 32.0`).
pub(crate) const FLYOUT_ROW_H: f32 = 32.0;

/// A tool group: its key (the flyout's identity), the tools it holds and the mark it shows.
pub(crate) struct Group {
    pub key: &'static str,
    pub tools: &'static [(ToolId, Icon)],
}

impl Group {
    fn has_flyout(&self) -> bool {
        self.tools.len() > 1
    }
}

/// The six groups, in the original's order (`ui-egui/src/toolbar.rs::GROUPS`).
pub(crate) const GROUPS: &[Group] = &[
    Group { key: "move", tools: &[(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand), (ToolId::Scale, Icon::Scale)] },
    Group { key: "frame", tools: &[(ToolId::Frame, Icon::Frame), (ToolId::Section, Icon::Section), (ToolId::Slice, Icon::Slice)] },
    Group {
        key: "shape",
        tools: &[
            (ToolId::Rectangle, Icon::Rectangle),
            (ToolId::Line, Icon::Line),
            (ToolId::Arrow, Icon::Arrow),
            (ToolId::Ellipse, Icon::Ellipse),
            (ToolId::Polygon, Icon::Polygon),
            (ToolId::Star, Icon::Star),
        ],
    },
    Group { key: "pen", tools: &[(ToolId::Pen, Icon::Pen), (ToolId::Pencil, Icon::Pencil), (ToolId::ShapeBuilder, Icon::ShapeBuilder)] },
    Group { key: "text", tools: &[(ToolId::Text, Icon::Text), (ToolId::TextOnPath, Icon::TextOnPath)] },
    Group { key: "comment", tools: &[(ToolId::Comment, Icon::Comment)] },
];

/// The bar's click channel: the view handles every [`Intent`] in one place.
pub(crate) type OnIntent = Rc<dyn Fn(Intent, &mut Window, &mut App)>;

/// What a click on the toolbar asks for. One callback carries them all, so the bar stays a plain
/// element builder and every button reaches the view through the same door.
pub(crate) enum Intent {
    /// Choose the group's shown tool (`tool.set`).
    Tool(&'static str),
    /// Toggle the flyout of group `key`.
    Flyout(&'static str),
    /// Toggle the Actions palette.
    Actions,
    /// Switch to mode `i` of the Draw · Design · Animation · Inspect switcher.
    Mode(usize),
}

/// The bar's natural width: the groups with their carets, Actions, the divider and the mode
/// switcher, one `GAP` apart (`main_width` in the original). The flyout's anchor and the test both
/// need it.
pub(crate) fn width() -> f32 {
    let groups_w: f32 = GROUPS.iter().map(group_w).sum();
    let items = GROUPS.len() as f32 + 3.0;
    PAD * 2.0 + groups_w + CELL + 1.0 + MODE_CELLS as f32 * CELL + GAP * (items - 1.0)
}

/// A group's width: its cell, plus the caret and the point between them when it has a flyout.
fn group_w(g: &Group) -> f32 {
    if g.has_flyout() { CELL + CARET_GAP + CARET_W } else { CELL }
}

/// Group `key`'s left edge **inside the pill** — where its tool cell starts.
pub(crate) fn group_left(key: &str) -> Option<f32> {
    let mut x = PAD;
    for g in GROUPS {
        if g.key == key {
            return Some(x);
        }
        x += group_w(g) + GAP;
    }
    None
}

/// The flyout caret's left edge **inside the pill**: the tool cell plus the point between them.
/// The flyout card is aligned to this, not to the cell (`用户 2026-11-06`：菜单左边和那 16pt 的
/// 点击按钮左边对齐). `None` for a single-tool group, which has no caret.
pub(crate) fn caret_left(key: &str) -> Option<f32> {
    let g = GROUPS.iter().find(|g| g.key == key)?;
    if !g.has_flyout() {
        return None;
    }
    Some(group_left(key)? + CELL + CARET_GAP)
}

/// A cell's top inside the 48 pt pill: the cells are centred, so 8 pt of the pill is above them.
pub(crate) const CELL_TOP: f32 = (PILL_H - CELL) / 2.0;
/// The flyout card's gap above the caret button (`用户 2026-11-06`：在按钮的正上方 4).
pub(crate) const FLYOUT_GAP: f32 = 4.0;

/// Which group the current tool belongs to, and which tool that group shows.
pub(crate) fn shown<'a>(g: &'a Group, current: ToolId, last: &'a [ToolId]) -> (ToolId, Icon) {
    if let Some(hit) = g.tools.iter().find(|(t, _)| *t == current) {
        return *hit;
    }
    let remembered = GROUPS.iter().position(|x| x.key == g.key).and_then(|i| last.get(i)).copied();
    let tool = remembered.filter(|t| g.tools.iter().any(|(x, _)| x == t)).or_else(|| g.tools.first().map(|x| x.0)).unwrap_or(ToolId::Move);
    g.tools.iter().find(|(t, _)| *t == tool).copied().unwrap_or((tool, Icon::Move))
}

/// "Shift+Cmd+K" → "⇧⌘K" on macOS, "Ctrl+Shift+K" elsewhere (`pretty_shortcut`).
pub(crate) fn pretty_shortcut(spec: &str) -> String {
    figcraft_engine::keymap::display(spec, figcraft_engine::keymap::Platform::current())
}

/// A group's flyout rows: one row per tool, the current one checked.
pub(crate) fn flyout_items(key: &str, current: ToolId) -> Vec<Item> {
    let Some(g) = GROUPS.iter().find(|g| g.key == key) else { return Vec::new() };
    g.tools
        .iter()
        .map(|(tool, icon)| {
            Item::new(tool.id(), figcraft_i18n::tr(tool.label())).icon(*icon).checked(Some(*tool == current)).trail(pretty_shortcut(tool.shortcut()))
        })
        .collect()
}

/// The whole pill, absolutely placed inside the shell's top row.
///
/// The wrapper spans **the canvas's own box** and centres the pill in it (the original centres the
/// pill on `canvas.center()`, not on the window: the rail and the two panels shift the canvas off
/// centre). `canvas_left` / `canvas_w` are that box in window coordinates.
pub(crate) fn bar(
    tool: ToolId,
    last: &[ToolId],
    flyout: Option<&'static str>,
    actions_open: bool,
    motion: bool,
    // 右栏 Inspect（Dev Mode）标签是否激活 —— 模式切换器的第 4 段不是死按钮了。
    inspect: bool,
    read_only: Option<&'static str>,
    canvas_left: f32,
    canvas_w: f32,
    on: OnIntent,
) -> Div {
    let t = tokens();
    // No `max_w`: the pill is purely content-sized (auto width). A clamp here would shrink the
    // frame while the `flex_none` cells keep their fixed size, so the controls overflow the pill
    // (`用户 2026-11-06`：框缩回去、控件超出). The wrapper still centres it over the canvas.
    let mut pill = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(GAP))
        .px(pt(PAD))
        .h(pt(PILL_H))
        .rounded(pt(13.0))
        .bg(rgba(t.panel))
        .border_1()
        .border_color(rgba(t.border_strong))
        .shadow(vec![BoxShadow { color: rgba(t.shadow).into(), offset: point(px(0.0), px(3.0)), blur_radius: px(14.0), spread_radius: px(0.0) }])
        // The pill swallows the press: a tool click must not fall through to the canvas below.
        .occlude();

    if let Some(why) = read_only {
        let label = if why == figcraft_engine::live::PUBLISHING { figcraft_i18n::tr("Publishing…") } else { figcraft_i18n::tr("View only") };
        for (id, icon) in [(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand)] {
            let active = tool == id;
            let on = on.clone();
            pill = pill.child(tool_cell(id.id(), icon, active, tip_for(id), move |w, cx| on(Intent::Tool(id.id()), w, cx)));
        }
        pill = pill.child(divider());
        let badge = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(6.0))
            .h(pt(26.0))
            .px(pt(12.0))
            .rounded(pt(13.0))
            .bg(rgba(t.field))
            .text_size(pt(crate::widgets::TEXT_PT))
            .text_color(rgba(t.text))
            .child(paint_icon(Icon::Eye, rgba(t.text_secondary), 14.0))
            .child(label);
        return centred(pill.child(badge), canvas_left, canvas_w);
    }

    // ── the tool groups ─────────────────────────────────────────────────────────────────────
    for g in GROUPS {
        let in_group = g.tools.iter().any(|(t, _)| *t == tool);
        let (id, icon) = shown(g, tool, last);
        let key = g.key;
        let on_tool = on.clone();
        let cell = tool_cell(id.id(), icon, in_group, tip_for(id), move |w, cx| on_tool(Intent::Tool(id.id()), w, cx));
        let mut group = div().flex().flex_row().items_center().gap(pt(CARET_GAP)).child(cell);
        if g.has_flyout() {
            let open = flyout == Some(key);
            let on_fly = on.clone();
            let caret = div()
                .id(("caret", GROUPS.iter().position(|x| x.key == key).unwrap_or(0) as u64))
                .flex_none()
                .w(pt(CARET_W))
                .h(pt(CELL))
                .flex()
                .items_center()
                .justify_center()
                .rounded(pt(4.0))
                .cursor_pointer()
                .hover(|s| s.bg(rgba(if open { t.pressed } else { t.hover })))
                .when(open, |d| d.bg(rgba(t.pressed)))
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_fly(Intent::Flyout(key), w, cx))
                .child(paint_icon(Icon::ChevronUp, rgba(t.text_secondary), CARET));
            group = group.child(caret);
        }
        pill = pill.child(group);
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────────────
    let on_actions = on.clone();
    pill = pill.child(
        div()
            .id("toolbar-actions")
            .flex_none()
            .size(pt(CELL))
            .flex()
            .items_center()
            .justify_center()
            .rounded(pt(5.0))
            .cursor_pointer()
            .when(actions_open, |d| d.bg(rgba(t.accent)))
            .when(!actions_open, |d| d.hover(|s| s.bg(rgba(t.hover))))
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_actions(Intent::Actions, w, cx))
            .child(paint_icon(Icon::Actions, rgba(if actions_open { t.accent_on } else { t.icon }), GLYPH)),
    );

    pill = pill.child(divider());

    // ── the mode switcher: Draw · Design · Animation · 开发 ──────────────────────────────────
    // Draw is Design with the pencil; Animation is the Motion timeline; 开发 (Dev Mode) opens the
    // right-panel inspector (Inspect 标签 + ui_requests 通道), reached here or via Shift+D.
    let active = if motion {
        Some(2)
    } else if inspect {
        Some(3)
    } else if tool == ToolId::Pencil {
        Some(0)
    } else {
        Some(1)
    };
    let modes = [(Icon::Pencil, "Draw"), (Icon::Frame, "Design"), (Icon::Animation, "Animation"), (Icon::Code, "开发")];
    let mut seg = div().flex_none().w(pt(MODE_CELLS as f32 * CELL)).h(pt(CELL)).flex().flex_row().items_center().rounded(pt(5.0)).bg(rgba(t.field));
    for (i, (icon, name)) in modes.iter().enumerate() {
        // 四段全部可点：Inspect 现在有面板（右栏 Inspect 标签 + ui_requests 通道）。
        let is_active = active == Some(i);
        let on_mode = on.clone();
        let mut cell = div().id(("mode", i)).relative().flex_none().w(pt(CELL)).h(pt(CELL)).flex().items_center().justify_center().rounded(pt(5.0));
        cell = cell
            .cursor_pointer()
            .when(is_active, |d| d.child(div().absolute().left(pt(2.0)).top(pt(2.0)).size(pt(28.0)).rounded(pt(3.0)).bg(rgba(t.panel))))
            .when(!is_active, |d| d.hover(|s| s.bg(rgba(t.hover))))
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_mode(Intent::Mode(i), w, cx));
        let color = if is_active { t.accent } else { t.text_secondary };
        // The four names are the mode switcher's own labels (`ui-egui`'s `on_hover_text(tip)`);
        // 四段的 tip 与 egui `on_hover_text` 一致。
        let tip: SharedString = figcraft_i18n::tr(name).to_string().into();
        let cell = cell.tooltip(move |_window, cx| cx.new(|_| Tip { text: tip.clone() }).into());
        seg = seg.child(cell.child(paint_icon(*icon, rgba(color), MODE_GLYPH)));
    }
    placed(pill.child(seg), canvas_left, canvas_w)
}

/// The pill's left edge in window coordinates. **The one formula** both the pill's own placement
/// ([`placed`]) and the flyout's anchor (`flyout_at`) use — so they can never disagree when the
/// window narrows (`用户 2026-11-06`：窗口缩小一点，弹窗位置就不对了). The pill is content-sized and
/// is never clamped, so a narrow canvas just makes this offset negative (it overflows, centred).
pub(crate) fn pill_left(canvas_left: f32, canvas_w: f32) -> f32 {
    canvas_left + (canvas_w - width()) / 2.0
}

/// Place the pill at its exact left edge instead of relying on flex `justify_center`: the flyout
/// anchor and the pill then share [`pill_left`] by construction.
fn placed(pill: Div, canvas_left: f32, canvas_w: f32) -> Div {
    let left = pill_left(canvas_left, canvas_w);
    div().absolute().left(px(left)).bottom(px(12.0)).child(pill)
}

/// Wrap `pill` in a row that spans the canvas's box and centres it: the pill lands on the canvas's
/// middle whatever its content's width, and 12 pt above the canvas's bottom edge.
fn centred(pill: Div, canvas_left: f32, canvas_w: f32) -> Div {
    div().absolute().left(px(canvas_left)).w(px(canvas_w.max(0.0))).bottom(px(12.0)).flex().flex_row().justify_center().child(pill)
}

/// A one-line tooltip: the tool's name and its shortcut, on the menu's own card colours
/// (`ui-egui` shows `"Rectangle  R"` under the pointer).
struct Tip {
    text: SharedString,
}

impl gpui::Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let t = tokens();
        div()
            .px(pt(8.0))
            .py(pt(4.0))
            .rounded(pt(4.0))
            .bg(rgba(t.menu))
            .border_1()
            .border_color(rgba(t.border))
            .text_size(pt(crate::widgets::TEXT_PT))
            .text_color(rgba(t.menu_text))
            .child(self.text.clone())
    }
}

/// A square tool button: accent solid when active, grey on hover (`ui-egui::tool_cell`).
fn tool_cell(id: &'static str, icon: Icon, active: bool, tip: SharedString, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    let t = tokens();
    let mut cell = div()
        .id(id)
        .flex_none()
        .size(pt(CELL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .cursor_pointer()
        .tooltip(move |_window, cx| cx.new(|_| Tip { text: tip.clone() }).into())
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_click(w, cx));
    // 当前工具格：accent 实心；其余格子悬停为灰底。不加描边（圆角外圈的淡边是「虚边」）。
    cell = if active { cell.bg(rgba(t.accent)) } else { cell.hover(|s| s.bg(rgba(t.hover))) };
    cell.child(paint_icon(icon, rgba(if active { t.accent_on } else { t.icon }), GLYPH))
}

/// "Rectangle  R": the tooltip text for a tool (`ui-egui`'s `format!("{}  {}", label, shortcut)`).
fn tip_for(tool: ToolId) -> SharedString {
    format!("{}  {}", figcraft_i18n::tr(tool.label()), pretty_shortcut(tool.shortcut())).into()
}

/// A 1 pt separator, as tall as the pill's content (`divider`).
fn divider() -> Div {
    let t = tokens();
    div().flex_none().w(pt(1.0)).h(pt(CELL)).bg(rgba(t.border))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pill's width is the groups (with their carets), Actions, the divider and the mode
    /// switcher, one 8 pt gap apart — the original's `main_width` arithmetic.
    #[test]
    fn width_matches_the_original_arithmetic() {
        // move/frame/shape/pen/text carry carets: 49 each; comment is a single 32 cell.
        let groups: f32 = 5.0 * (CELL + CARET_GAP + CARET_W) + CELL;
        let items = GROUPS.len() as f32 + 3.0;
        let expected = PAD * 2.0 + groups + CELL + 1.0 + 4.0 * CELL + GAP * (items - 1.0);
        assert_eq!(width(), expected);
        assert_eq!(width(), 518.0);
    }

    /// A group shows its own tool when the current tool is in it, else the remembered one, else its
    /// first tool.
    #[test]
    fn a_group_shows_the_current_then_the_remembered_tool() {
        let move_group = &GROUPS[0];
        assert_eq!(shown(move_group, ToolId::Hand, &[]).0, ToolId::Hand);
        let last = [ToolId::Scale];
        assert_eq!(shown(move_group, ToolId::Rectangle, &last).0, ToolId::Scale, "remembered");
        assert_eq!(shown(move_group, ToolId::Rectangle, &[]).0, ToolId::Move, "first");
    }

    /// The flyout lists every tool of its group and checks the current one.
    #[test]
    fn flyout_checks_the_current_tool() {
        let rows = flyout_items("shape", ToolId::Ellipse);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.iter().filter(|r| r.checked == Some(true)).count(), 1);
        assert_eq!(rows[3].id, "ellipse");
        assert!(rows.iter().all(|r| r.icon.is_some()));
        assert!(flyout_items("nope", ToolId::Move).is_empty());
    }

    /// Every group's flyout ticks the tool its own cell shows — even the groups that do not hold
    /// the current tool. Checking the raw current tool left every non-active group's flyout with no
    /// tick at all (`用户 2026-11-06`：点第 1 个有勾、点第 2 个没有勾).
    #[test]
    fn every_groups_flyout_ticks_the_tool_its_cell_shows() {
        // Default toolbar: Move is current, and each group remembers its first tool.
        let current = ToolId::Move;
        let last: Vec<ToolId> = GROUPS.iter().map(|g| g.tools[0].0).collect();
        for g in GROUPS {
            let shown_tool = shown(g, current, &last).0;
            let rows = flyout_items(g.key, shown_tool);
            assert_eq!(
                rows.iter().filter(|r| r.checked == Some(true)).count(),
                1,
                "group `{}` must tick exactly one row",
                g.key
            );
            assert!(
                rows.iter().any(|r| r.id == shown_tool.id() && r.checked == Some(true)),
                "group `{}` must tick the tool its cell shows ({})",
                g.key,
                shown_tool.id()
            );
            // Sanity: the old behaviour (ticking the raw current tool) leaves other groups blank.
            if !g.tools.iter().any(|(t, _)| *t == current) {
                assert!(
                    flyout_items(g.key, current).iter().all(|r| r.checked != Some(true)),
                    "precondition: group `{}` has no row matching the global current tool",
                    g.key
                );
            }
        }
    }

    /// The flyout card hangs off **the 16 pt caret**, not off the tool cell: its left edge is the
    /// caret's, and its bottom edge is 4 pt above the caret button's top edge (user, 2026-11-06).
    #[test]
    fn the_flyout_hangs_off_the_caret() {
        // move is the first group: 8 pt of padding, then the 32 pt cell, then the 1 pt gap.
        assert_eq!(caret_left("move"), Some(PAD + CELL + CARET_GAP));
        assert_eq!(group_left("move"), Some(PAD));
        // frame sits one group further right (its width, the gap, then its own cell).
        assert_eq!(caret_left("frame"), Some(PAD + group_w(&GROUPS[0]) + GAP + CELL + CARET_GAP));
        // `comment` is a single-tool group: no caret, no flyout.
        assert_eq!(caret_left("comment"), None);
        assert_eq!(caret_left("nope"), None);
        // The cells are centred in the 48 pt pill, so 8 pt of it is above a button's top edge; the
        // card's bottom edge sits 4 pt above that.
        assert_eq!(CELL_TOP, 8.0);
        assert_eq!(FLYOUT_GAP, 4.0);
        assert_eq!(CELL_TOP - FLYOUT_GAP, 4.0);
    }
}
