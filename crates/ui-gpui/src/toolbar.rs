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
//! A board document gets its own nine groups ([`BOARD_GROUPS`], the original's order) and its own
//! tail: the templates picker and the Design panel's button, in place of the mode switcher (a board
//! has no prototypes and no Dev Mode). Its one-tool groups (sticky notes, shapes, stamps) open a
//! flyout of **choices** rather than of tools — see [`Group::options`].

use crate::menu_card::Item;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{pt, rgba, tokens};
use figcraft_engine::ToolId;
use figcraft_engine::tools::{DrawLineStyle, DrawStyle};
use figcraft_engine::vectoredit::VTool;
use figcraft_i18n::tr;
use gpui::{App, BoxShadow, Div, ImageSource, MouseButton, MouseDownEvent, SharedString, Stateful, Window, div, img, point, prelude::*, px};
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
/// A flyout row's height: every menu shares `menu_metrics::MENU_ROW_H` (24 pt), which also
/// matches the shape-tool marks' natural 24x24 viewBox.
pub(crate) const FLYOUT_ROW_H: f32 = crate::menu_metrics::MENU_ROW_H;

/// A tool group: its key (the flyout's identity), the tools it holds and the mark it shows.
pub(crate) struct Group {
    pub key: &'static str,
    /// The flyout button's name, for its tooltip / screen readers (`ui-egui`'s `a11y::menu_button`).
    pub name: &'static str,
    pub tools: &'static [(ToolId, Icon)],
    /// What a one-tool group's flyout offers instead of tools (boards: colours, shapes, stamps).
    pub options: Options,
}

/// The choices a board tool's flyout shows (`ui-egui/src/toolbar.rs::Options`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Options {
    None,
    Colors,
    Shapes,
    Stamps,
}

impl Group {
    /// A caret when the group has more than one tool **or** choices to offer — a board's sticky
    /// note, shape and stamp groups are one tool each, but their caret opens the picker.
    fn has_flyout(&self) -> bool {
        self.tools.len() > 1 || self.options != Options::None
    }
}

/// The six groups, in the original's order (`ui-egui/src/toolbar.rs::GROUPS`).
pub(crate) const GROUPS: &[Group] = &[
    Group {
        key: "move",
        name: "Move tools",
        tools: &[(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand), (ToolId::Scale, Icon::Scale)],
        options: Options::None,
    },
    Group {
        key: "frame",
        name: "Frame tools",
        tools: &[(ToolId::Frame, Icon::Frame), (ToolId::Section, Icon::Section), (ToolId::Slice, Icon::Slice)],
        options: Options::None,
    },
    Group {
        key: "shape",
        name: "Shape tools",
        tools: &[
            (ToolId::Rectangle, Icon::Rectangle),
            (ToolId::Line, Icon::Line),
            (ToolId::Arrow, Icon::Arrow),
            (ToolId::Ellipse, Icon::Ellipse),
            (ToolId::Polygon, Icon::Polygon),
            (ToolId::Star, Icon::Star),
        ],
        options: Options::None,
    },
    Group {
        key: "pen",
        name: "Drawing tools",
        tools: &[(ToolId::Pen, Icon::Pen), (ToolId::Pencil, Icon::Pencil), (ToolId::ShapeBuilder, Icon::ShapeBuilder)],
        options: Options::None,
    },
    Group { key: "text", name: "Text", tools: &[(ToolId::Text, Icon::Text), (ToolId::TextOnPath, Icon::TextOnPath)], options: Options::None },
    Group { key: "comment", name: "Comment", tools: &[(ToolId::Comment, Icon::Comment)], options: Options::None },
];

/// A board's tool groups, in the original's order (`ui-egui/src/toolbar.rs::BOARD_GROUPS`).
pub(crate) const BOARD_GROUPS: &[Group] = &[
    Group { key: "bmove", name: "Move tools", tools: &[(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand)], options: Options::None },
    Group {
        key: "ink",
        name: "Drawing tools",
        tools: &[(ToolId::Marker, Icon::Marker), (ToolId::Highlighter, Icon::Highlighter), (ToolId::Eraser, Icon::Eraser)],
        options: Options::None,
    },
    Group { key: "sticky", name: "Sticky note colors", tools: &[(ToolId::Sticky, Icon::Board)], options: Options::Colors },
    Group { key: "bshape", name: "Shapes", tools: &[(ToolId::BoardShape, Icon::Shapes)], options: Options::Shapes },
    Group { key: "connector", name: "Connector", tools: &[(ToolId::Connector, Icon::Connector)], options: Options::None },
    Group { key: "text", name: "Text", tools: &[(ToolId::Text, Icon::Text)], options: Options::None },
    Group { key: "section", name: "Section", tools: &[(ToolId::Section, Icon::Section)], options: Options::None },
    Group { key: "stamp", name: "Stamps", tools: &[(ToolId::Stamp, Icon::Stamp)], options: Options::Stamps },
    Group { key: "comment", name: "Comment", tools: &[(ToolId::Comment, Icon::Comment)], options: Options::None },
];

/// The groups the toolbar shows for the open document: a board's, else the design tools'.
pub(crate) fn groups(board: bool) -> &'static [Group] {
    if board { BOARD_GROUPS } else { GROUPS }
}

/// Group `key`, looked up in both sets: a popup stores only the key, so the renderer needs a way
/// back to the group's options and its tool.
pub(crate) fn group(key: &str) -> Option<&'static Group> {
    GROUPS.iter().chain(BOARD_GROUPS).find(|g| g.key == key)
}

/// The bar's click channel: the view handles every [`Intent`] in one place.
pub(crate) type OnIntent = Rc<dyn Fn(Intent, &mut Window, &mut App)>;

/// What a click on the toolbar asks for. One callback carries them all, so the bar stays a plain
/// element builder and every button reaches the view through the same door.
#[derive(Clone)]
pub(crate) enum Intent {
    /// Choose the group's shown tool (`tool.set`).
    Tool(&'static str),
    /// Toggle the flyout of group `key`.
    Flyout(&'static str),
    /// Toggle the Actions palette.
    Actions,
    /// Toggle the board's templates picker (`crate::board_picker`).
    Templates,
    /// Show or hide the Design panel on a board (`view.design`).
    DesignPanel,
    /// Switch to mode `i` of the Draw · Design · Animation · Inspect switcher.
    Mode(usize),
    /// `vector.edit.tool`: pick one of `VTool::ALL` in the vector edit toolbar.
    VectorTool(&'static str),
    /// `vector.edit.end`: leave vector edit mode (the toolbar's close button).
    VectorDone,
    /// Toggle the vector edit row's "More" dropdown (Shape builder / Variable width).
    VectorMore,
    /// Draw mode's illustrated Brush nib: pencil + its ink brush preset.
    DrawBrush,
    /// Draw toolbar: open the stroke colour picker.
    DrawColor,
    /// Draw toolbar: `tool.drawStyle` weight / lineStyle / brush preset.
    DrawWeight(f64),
    DrawLine(&'static str),
    DrawPreset(Option<&'static str>),
    /// Draw toolbar: Pen's "Done drawing" → `tool.set move`.
    DrawDone,
}

/// Draw mode replaces the Pen group with lizard's illustrated nib strip
/// (`ui-egui/src/toolbar.rs::draw_brushes`): three 54 pt cells between two hairlines, 182 pt
/// in total, inserted right after the Move group.
const DRAW_NIB_CELL: f32 = 54.0;
const DRAW_SECTION_W: f32 = 182.0;

/// The three nib illustrations, decoded once into GPUI `RenderImage`s. They are embedded with
/// `include_bytes!`, so no runtime file path is needed (the icon AssetSource is frozen).
fn draw_art() -> &'static [std::sync::Arc<gpui::RenderImage>; 3] {
    static ART: std::sync::OnceLock<[std::sync::Arc<gpui::RenderImage>; 3]> = std::sync::OnceLock::new();
    ART.get_or_init(|| {
        let files: [&[u8]; 3] = [
            include_bytes!("../assets/draw/draw-pen.png"),
            include_bytes!("../assets/draw/draw-brush.png"),
            include_bytes!("../assets/draw/draw-pencil.png"),
        ];
        std::array::from_fn(|i| {
            let rgba = image::load_from_memory(files[i]).expect("draw nib PNG").to_rgba8();
            let (w, h) = rgba.dimensions();
            crate::canvas::render_worker::wrap_rgba(w, h, rgba.into_raw())
        })
    })
}

/// The Draw-only part of the main pill: Pen / Brush / Pencil as illustrated nibs. This crate has
/// no `ToolId::Brush`, so Brush activates the pencil with its ink brush preset (the same
/// `pencil_brush` / `tool.drawStyle` path the Draw toolbar already uses).
fn draw_section(tool: ToolId, brush: bool, on: &OnIntent) -> Div {
    let t = tokens();
    let art = draw_art();
    let entries: [(&str, bool, Intent); 3] = [
        ("pen", tool == ToolId::Pen, Intent::Tool("pen")),
        ("brush", brush, Intent::DrawBrush),
        ("pencil", tool == ToolId::Pencil && !brush, Intent::Tool("pencil")),
    ];
    let mut row = div().flex_none().w(pt(DRAW_SECTION_W)).h(pt(PILL_H)).flex().flex_row().items_center().justify_between().px(pt(8.0)).child(divider());
    for (i, (id, active, intent)) in entries.into_iter().enumerate() {
        let tex = art[i].clone();
        let (iw, ih) = (art[i].size(0).width.0 as f32, art[i].size(0).height.0 as f32);
        let w = 60.0 * iw / ih;
        let on_click = on.clone();
        let label: SharedString = tr(match id { "pen" => "Pen", "brush" => "Brush", _ => "Pencil" }).into();
        let cell = div()
            .id(("draw-nib", i as u64))
            .relative()
            .flex_none()
            .w(pt(DRAW_NIB_CELL))
            .h(pt(PILL_H))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            // The selected nib sits on a raised 5 pt rounded plate, with its art lifted 8 pt.
            .when(active, |d| {
                d.child(div().absolute().left(pt(4.0)).top(pt(4.0)).w(pt(46.0)).h(pt(40.0)).rounded(pt(5.0)).bg(rgba(t.pressed)))
            })
            .when(!active, |d| d.hover(|s| s.bg(rgba(t.hover))))
            .tooltip(move |_window, cx| cx.new(|_| Tip { text: label.clone() }).into())
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_click(intent.clone(), w, cx))
            .child(
                div()
                    .relative()
                    .top(px(if active { -8.0 } else { 0.0 }))
                    .w(px(w))
                    .h(px(60.0))
                    .child(img(ImageSource::Render(tex)).w(px(w)).h(px(60.0))),
            );
        row = row.child(cell);
    }
    row.child(divider())
}

/// The bar's natural width: the groups with their carets, Actions, the divider and the mode
/// switcher, one `GAP` apart (`main_width` in the original). In Draw mode the Pen group is
/// replaced by the 182 pt nib strip, so the pill grows by `DRAW_SECTION_W - pen_group_w`.
/// The flyout's anchor and the test both need it.
pub(crate) fn width(groups: &[Group], board: bool, draw_mode: bool) -> f32 {
    let groups_w: f32 = groups.iter().map(group_w).sum();
    let items = groups.len() as f32 + 3.0;
    // A board swaps the 4-cell mode switcher for two cells: the templates picker and the Design
    // panel's button. (`ui-egui`'s `main_width` still counts only the Design panel's cell — its
    // templates button is built but never mounted, so nothing ever needed the extra cell. The port
    // does mount it, so the tail is both cells and the point between them.)
    let tail = if board { 2.0 * CELL + GAP } else { MODE_CELLS as f32 * CELL };
    let draw_extra = if draw_mode && !board {
        DRAW_SECTION_W - groups.iter().find(|g| g.key == "pen").map(group_w).unwrap_or(0.0)
    } else {
        0.0
    };
    PAD * 2.0 + groups_w + CELL + 1.0 + tail + GAP * (items - 1.0) + draw_extra
}

/// A group's width: its cell, plus the caret and the point between them when it has a flyout.
fn group_w(g: &Group) -> f32 {
    if g.has_flyout() { CELL + CARET_GAP + CARET_W } else { CELL }
}

/// Group `key`'s left edge **inside the pill** — where its tool cell starts.
pub(crate) fn group_left(groups: &[Group], key: &str) -> Option<f32> {
    let mut x = PAD;
    for g in groups {
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
pub(crate) fn caret_left(groups: &[Group], key: &str) -> Option<f32> {
    let g = groups.iter().find(|g| g.key == key)?;
    if !g.has_flyout() {
        return None;
    }
    Some(group_left(groups, key)? + CELL + CARET_GAP)
}

/// A cell's top inside the 48 pt pill: the cells are centred, so 8 pt of the pill is above them.
pub(crate) const CELL_TOP: f32 = (PILL_H - CELL) / 2.0;
/// The flyout card's gap above the caret button (`用户 2026-11-06`：在按钮的正上方 4).
pub(crate) const FLYOUT_GAP: f32 = 4.0;

/// Which group the current tool belongs to, and which tool that group shows.
///
/// `last` is keyed by **group key** — exactly like egui's `tb_last`, which stores one
/// `egui::Id::unique(("tb_last", key))` per group. Keying by position would make a board's 9
/// groups and a design file's 6 groups share slots and remember each other's tools.
pub(crate) fn shown(g: &Group, current: ToolId, last: &[(&'static str, ToolId)]) -> (ToolId, Icon) {
    if let Some(hit) = g.tools.iter().find(|(t, _)| *t == current) {
        return *hit;
    }
    let remembered = last.iter().find(|(k, _)| *k == g.key).map(|(_, t)| *t);
    let tool = remembered.filter(|t| g.tools.iter().any(|(x, _)| x == t)).or_else(|| g.tools.first().map(|x| x.0)).unwrap_or(ToolId::Move);
    g.tools.iter().find(|(t, _)| *t == tool).copied().unwrap_or((tool, Icon::Move))
}

/// "Shift+Cmd+K" → "⇧⌘K" on macOS, "Ctrl+Shift+K" elsewhere (`pretty_shortcut`).
pub(crate) fn pretty_shortcut(spec: &str) -> String {
    figcraft_engine::keymap::display(spec, figcraft_engine::keymap::Platform::current())
}

/// A group's flyout rows: one row per tool, the current one checked.
pub(crate) fn flyout_items(groups: &[Group], key: &str, current: ToolId) -> Vec<Item> {
    let Some(g) = groups.iter().find(|g| g.key == key) else { return Vec::new() };
    g.tools
        .iter()
        .map(|(tool, icon)| {
            let is_current = *tool == current;
            Item::new(tool.id(), figcraft_i18n::tr(tool.label()))
                .icon(*icon)
                .checked(Some(is_current))
                // 当前工具那一行把 hover 底色常亮（用户 2026-11-14：只有勾、没有颜色）。
                .active_fill(is_current)
                .trail(pretty_shortcut(tool.shortcut()))
        })
        .collect()
}

/// The whole pill, absolutely placed inside the shell's top row.
///
/// The wrapper spans **the canvas's own box** and centres the pill in it (the original centres the
/// pill on `canvas.center()`, not on the window: the rail and the two panels shift the canvas off
/// centre). `canvas_left` / `canvas_w` are that box in window coordinates.
/// 模式切换器高亮哪一段。Draw 只看真实模式状态，不看当前工具：Design 里点出铅笔/钢笔
/// 仍然是 Design（否则会误亮 Draw，并把 Draw 专属的宽度栏带出来）。
fn active_mode(motion: bool, inspect: bool, draw_mode: bool) -> Option<usize> {
    if motion {
        Some(2)
    } else if inspect {
        Some(3)
    } else if draw_mode {
        Some(0)
    } else {
        Some(1)
    }
}

pub(crate) fn bar(
    tool: ToolId,
    last: &[(&'static str, ToolId)],
    flyout: Option<&'static str>,
    actions_open: bool,
    motion: bool,
    // 右栏 Inspect（Dev Mode）标签是否激活 —— 模式切换器的第 4 段不是死按钮了。
    inspect: bool,
    read_only: Option<&'static str>,
    canvas_left: f32,
    canvas_w: f32,
    on: OnIntent,
    board: bool,
    // 画板的 Design 面板是否展开（`ui-egui` 的 `app.ui.board_panel`）。
    board_panel: bool,
    // 真正的 Draw 模式（模式切换器第 0 段）。**不**从「当前工具 == Pencil」推断：
    // Design 模式里点出铅笔/钢笔只是换工具，不能把模式显示成 Draw、更不能弹出 Draw 专属栏。
    draw_mode: bool,
    // 铅笔已挂上笔刷预设（`pencil_brush`）：本仓没有 `ToolId::Brush`，用它点亮 Brush 笔尖。
    draw_brush: bool,
) -> Div {
    let t = tokens();
    let groups = groups(board);
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
    for g in groups {
        // Draw mode drops the Pen group entirely and puts lizard's illustrated nib strip right
        // after Move (`ui-egui/src/toolbar.rs::main_toolbar` + `draw_brushes`).
        if draw_mode && !board && g.key == "pen" {
            continue;
        }
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
                .id(("caret", groups.iter().position(|x| x.key == key).unwrap_or(0) as u64))
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
                .tooltip({
                    // 组的名字（egui `a11y::menu_button(&cresp, tr(g.name), open)`）。
                    let name: SharedString = tr(g.name).into();
                    move |_window, cx| cx.new(|_| Tip { text: name.clone() }).into()
                })
                .child(paint_icon(Icon::ChevronUp, rgba(t.text_secondary), CARET));
            group = group.child(caret);
        }
        pill = pill.child(group);
        if draw_mode && !board && g.key == "move" {
            pill = pill.child(draw_section(tool, draw_brush, &on));
        }
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

    if board {
        // ── a board's tail: templates, then the Design panel's button ────────────────────────
        // A board has no prototypes and no Dev Mode, and its Design panel starts hidden; this
        // button is the only way to bring it back (`ui-egui/src/toolbar.rs::main_toolbar`).
        let on_tpl = on.clone();
        let open = flyout == Some(crate::board_picker::KEY);
        pill = pill.child(tool_cell("board_templates", Icon::Templates, open, tr("Templates").into(), move |w, cx| on_tpl(Intent::Templates, w, cx)));
        let on_panel = on.clone();
        pill = pill
            .child(tool_cell("board_panel", Icon::Panels, board_panel, tr("Design panel").into(), move |w, cx| on_panel(Intent::DesignPanel, w, cx)));
        return placed(pill, groups, board, draw_mode, canvas_left, canvas_w);
    }

    // ── the mode switcher: Draw · Design · Animation · 开发 ──────────────────────────────────
    // Draw is Design with the pencil; Animation is the Motion timeline; 开发 (Dev Mode) opens the
    // right-panel inspector (Inspect 标签 + ui_requests 通道), reached here or via Shift+D.
    let active = active_mode(motion, inspect, draw_mode);
    let modes = [(Icon::Pencil, tr("Draw")), (Icon::Frame, tr("Design")), (Icon::Animation, tr("Animation")), (Icon::Code, "开发")];
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
    placed(pill.child(seg), groups, board, draw_mode, canvas_left, canvas_w)
}

/// The pill's left edge in window coordinates. **The one formula** both the pill's own placement
/// ([`placed`]) and the flyout's anchor (`flyout_at`) use — so they can never disagree when the
/// window narrows (`用户 2026-11-06`：窗口缩小一点，弹窗位置就不对了). The pill is content-sized and
/// is never clamped, so a narrow canvas just makes this offset negative (it overflows, centred).
pub(crate) fn pill_left(groups: &[Group], board: bool, draw_mode: bool, canvas_left: f32, canvas_w: f32) -> f32 {
    canvas_left + (canvas_w - width(groups, board, draw_mode)) / 2.0
}

/// Place the pill at its exact left edge instead of relying on flex `justify_center`: the flyout
/// anchor and the pill then share [`pill_left`] by construction.
fn placed(pill: Div, groups: &[Group], board: bool, draw_mode: bool, canvas_left: f32, canvas_w: f32) -> Div {
    let left = pill_left(groups, board, draw_mode, canvas_left, canvas_w);
    div().absolute().left(px(left)).bottom(px(12.0)).child(pill)
}

/// Wrap `pill` in a row that spans the canvas's box and centres it: the pill lands on the canvas's
/// middle whatever its content's width, and 12 pt above the canvas's bottom edge.
fn centred(pill: Div, canvas_left: f32, canvas_w: f32) -> Div {
    div().absolute().left(px(canvas_left)).w(px(canvas_w.max(0.0))).bottom(px(12.0)).flex().flex_row().justify_center().child(pill)
}

/// A labelled button in one of the two extra toolbars (40 pt tall surfaces).
fn extra_button(label: impl Into<SharedString>, active: bool, intent: Intent, on: OnIntent) -> Div {
    let t = tokens();
    let mut d = div()
        .flex_none()
        .h(pt(28.0))
        .px(pt(10.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .text_size(pt(crate::widgets::TEXT_PT))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on(intent.clone(), w, cx))
        .child(label.into());
    d = if active {
        d.bg(rgba(t.accent)).text_color(rgba(t.accent_on))
    } else {
        d.text_color(rgba(t.text)).hover(|s| s.bg(rgba(t.hover)))
    };
    d
}

/// The Draw mode's own toolbar, 8 pt above the main pill.
///
/// Ported from lizard `ui-egui/src/toolbar.rs::drawing_toolbar`: a stroke swatch, the weight,
/// the Pencil's line style and brush presets, and the Pen's Done. Lizard uses dropdowns for the
/// last two; here they are segmented buttons so no extra popup state is needed, but every button
/// runs the real `tool.drawStyle` / `tool.pencilBrush` command.
pub(crate) fn drawing_bar(
    tool: ToolId,
    style: &DrawStyle,
    brush: Option<&str>,
    canvas_left: f32,
    canvas_w: f32,
    on: OnIntent,
) -> Div {
    let t = tokens();
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(4.0))
        .px(pt(8.0))
        .h(pt(40.0))
        .rounded(pt(13.0))
        .bg(rgba(t.panel))
        .border_1()
        .border_color(rgba(t.border_strong))
        .shadow(vec![BoxShadow {
            color: rgba(t.shadow).into(),
            offset: point(px(0.0), px(3.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
        }])
        .occlude();

    // Stroke colour → the shared colour picker, writing back with `tool.drawStyle`.
    let on_color = on.clone();
    let c = crate::widgets::swatch::color32(style.color());
    row = row.child(
        div()
            .flex_none()
            .size(pt(24.0))
            .rounded(pt(4.0))
            .bg(rgba(c))
            .border_1()
            .border_color(rgba(t.border_strong))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_color(Intent::DrawColor, w, cx)),
    );

    // Weight: − / value / + (lizard uses a NumberField plus a Slider).
    let weight = style.weight(tool);
    row = row
        .child(extra_button("−", false, Intent::DrawWeight((weight - 1.0).max(0.1)), on.clone()))
        .child(
            div()
                .flex_none()
                .w(pt(56.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(pt(crate::widgets::TEXT_PT))
                .text_color(rgba(t.text))
                .child(SharedString::from(format!("{weight:.1} px"))),
        )
        .child(extra_button("+", false, Intent::DrawWeight((weight + 1.0).min(500.0)), on.clone()));

    if tool == ToolId::Pencil {
        row = row.child(divider());
        for (ls, label) in [(DrawLineStyle::Solid, "Solid"), (DrawLineStyle::Dashed, "Dashed"), (DrawLineStyle::Dotted, "Dotted")] {
            row = row.child(extra_button(label, style.line_style == ls, Intent::DrawLine(ls.id()), on.clone()));
        }
        row = row.child(divider());
        let current = brush.unwrap_or("");
        for (id, label) in [("", "Plain"), ("taper", "Taper"), ("ink", "Ink"), ("bristle", "Bristle")] {
            let preset = if id.is_empty() { None } else { Some(id) };
            row = row.child(extra_button(label, current == id, Intent::DrawPreset(preset), on.clone()));
        }
    }
    if tool == ToolId::Pen {
        row = row.child(divider());
        row = row.child(extra_button(figcraft_i18n::tr("Done drawing").to_string(), false, Intent::DrawDone, on.clone()));
    }

    // 12 pt above the canvas bottom + the 48 pt main pill + an 8 pt gap.
    div()
        .absolute()
        .left(px(canvas_left))
        .w(px(canvas_w.max(0.0)))
        .bottom(px(12.0 + PILL_H + 8.0))
        .flex()
        .flex_row()
        .justify_center()
        .child(row)
}

/// The vector-edit row, transcribed from the saved Figma editor's `secondary_toolbelt`
/// (`aria-label="Vector editing"` in `Figma – Figma.html`) and its plaintext
/// `figma_app-*.min.css.br`. The saved DOM gives the exact order and grouping:
///
/// ```text
/// [icon 移动] [icon 套索工具] │ [icon 油漆桶] [icon 弯曲工具] [icon 剪切] [icon Erase] │ [更多 ▾] │ [✕]
/// ```
///
/// Container (`.secondary_toolbelt--secondaryToolbeltContainer`): 40 pt tall, 13 pt corners,
/// 8 pt padding, 8 pt gaps. Buttons (`.topLevelButtonSecondaryPadding`): no padding of their own,
/// a 24 pt icon + a nowrap label with 8 pt trailing padding, 5 pt corners; the selected one keeps
/// the hover fill lit (as the toolbar flyout does). The hairline dividers are 1 pt wide and
/// stretch the full 40 pt height. The bar sits 8 pt above the 48 pt main pill — Figma's 96 pt
/// root over a toolbelt pinned at `bottom:12px` with `height:48px` yields the same 8 pt gap.
///
/// Figma's own Lasso / Paint bucket / Bend glyphs have no counterpart in the frozen icon set
/// (`src/widgets/icon.rs`), so the nearest existing marks stand in; that file is untouched.
/// Pen and Variable width — the two edit tools Figma hides behind "More" — live in that dropdown.
const VECTOR_MAIN: [(VTool, Icon, &'static str); 6] = [
    (VTool::Move, Icon::Move, "Move"),
    (VTool::Lasso, Icon::ShapeBuilder, "Lasso"),
    (VTool::Paint, Icon::Marker, "Paint bucket"),
    (VTool::Bend, Icon::Animation, "Bend"),
    (VTool::Cut, Icon::Slice, "Cut"),
    (VTool::Eraser, Icon::Eraser, "Eraser"),
];

/// "More" carries **Shape builder + Variable width**, not Pen. Figma's own bundle
/// (`81504-…min.js.br`) declares `J=[k.NLJ.SHAPE_BUILDER, k.NLJ.VECTOR_VAR_WIDTH_POINT]`
/// and renders `g===SHAPE_BUILDER && H`, `g===VECTOR_VAR_WIDTH_POINT && N` next to the More menu;
/// the saved DOM's six visible buttons contain no Pen at all. lizard's Pen+Width was the wrong
/// source — copying it put a pen in More and hid the shape builder.
///
/// Shape builder is the engine's existing `ToolId::ShapeBuilder` (`tool.set shapeBuilder`,
/// `canvas/shapebuilder.rs`); variable width stays an edit-mode tool (`vector.edit.tool width`).
fn vector_more_items() -> [(&'static str, Icon, &'static str, &'static str, Intent); 2] {
    [
        ("shapeBuilder", Icon::ShapeBuilder, "Shape builder", ToolId::ShapeBuilder.shortcut(), Intent::Tool("shapeBuilder")),
        ("width", Icon::Variable, "Variable width", VTool::Width.shortcut(), Intent::VectorTool("width")),
    ]
}

/// A 1 pt hairline that reaches the container's outer edges (40 pt tall), cancelling its 8 pt
/// padding — Figma uses `align-self:stretch` with `margin-top/-bottom:-8px`.
fn vector_divider() -> Div {
    let t = tokens();
    div().flex_none().w(pt(1.0)).h(pt(40.0)).bg(rgba(t.border))
}

/// One icon + label button of the vector-edit row.
fn vector_tool_button(i: usize, tool: VTool, icon: Icon, label: &'static str, current: VTool, on: &OnIntent) -> Stateful<Div> {
    let t = tokens();
    let active = tool == current;
    let id = tool.id();
    let on_click = on.clone();
    let tip: SharedString = format!("{}  {}", tr(label), pretty_shortcut(tool.shortcut())).into();
    let ink = if active { t.menu_text } else { t.text };
    let glyph = if active { t.menu_text } else { t.icon };
    let mut btn = div()
        .id(("vtool", i as u64))
        .flex_none()
        .h(pt(24.0))
        .flex()
        .items_center()
        .rounded(pt(5.0))
        .text_size(pt(crate::widgets::TEXT_PT))
        .text_color(rgba(ink))
        .cursor_pointer()
        .tooltip(move |_window, cx| cx.new(|_| Tip { text: tip.clone() }).into())
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_click(Intent::VectorTool(id), w, cx))
        .child(paint_icon(icon, rgba(glyph), 24.0))
        .child(div().pr(pt(8.0)).child(tr(label)));
    if active {
        btn.bg(rgba(t.menu_hover))
    } else {
        btn.hover(|s| s.bg(rgba(t.hover)))
    }
}

/// "More" — Figma's `aria-haspopup=menu` button: a text label plus a 16 pt chevron.
fn vector_more_button(open: bool, on: &OnIntent) -> Stateful<Div> {
    let t = tokens();
    let on_click = on.clone();
    let mut btn = div()
        .id("vector-edit-more")
        .flex_none()
        .h(pt(24.0))
        .flex()
        .items_center()
        .gap(pt(2.0))
        .rounded(pt(5.0))
        .text_size(pt(crate::widgets::TEXT_PT))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_click(Intent::VectorMore, w, cx))
        .child(div().pl(pt(4.0)).child(tr("More")))
        .child(paint_icon(Icon::ChevronDown, rgba(if open { t.menu_text } else { t.icon }), 16.0));
    if open {
        btn.bg(rgba(t.menu_hover)).text_color(rgba(t.menu_text))
    } else {
        btn.text_color(rgba(t.text)).hover(|s| s.bg(rgba(t.hover)))
    }
}

/// The dropdown 8 pt above the bar, right-aligned to it, in the shared menu-card colours.
fn vector_more_card(current: VTool, on: &OnIntent) -> Div {
    let t = tokens();
    let mut card = crate::widgets::menu_surface().absolute().bottom(pt(48.0)).right(pt(0.0)).w(pt(184.0)).gap(pt(2.0));
    for (i, (id, icon, label, shortcut, intent)) in vector_more_items().into_iter().enumerate() {
        let on_click = on.clone();
        // Only the edit-mode tool can be lit from `VTool`; shape builder is a main tool.
        let active = id == "width" && current == VTool::Width;
        let mut r = div()
            .id(("vtool-more", i as u64))
            .flex_none()
            .h(pt(24.0))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(pt(8.0))
            .px(pt(4.0))
            .rounded(pt(5.0))
            .text_size(pt(crate::widgets::TEXT_PT))
            .text_color(rgba(t.menu_text))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_click(intent.clone(), w, cx))
            .child(paint_icon(icon, rgba(t.menu_text), 16.0))
            .child(tr(label))
            .child(div().flex_1())
            .child(div().text_color(rgba(t.menu_dim)).child(pretty_shortcut(shortcut)));
        r = if active { r.bg(rgba(t.menu_hover)) } else { r.hover(|s| s.bg(rgba(t.menu_hover))) };
        card = card.child(r);
    }
    card
}

/// A second toolbar **above** the main pill while a vector is being edited, laid out like Figma's
/// `Vector editing` secondary toolbelt. Each tool button runs `vector.edit.tool`; the close
/// button runs `vector.edit.end`; `more_open` shows the two hidden edit tools.
pub(crate) fn vector_edit_bar(current: VTool, more_open: bool, canvas_left: f32, canvas_w: f32, on: OnIntent) -> Div {
    let t = tokens();
    let mut row = div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(pt(8.0))
        .p(pt(8.0))
        .h(pt(40.0))
        .rounded(pt(13.0))
        .bg(rgba(t.panel))
        .border_1()
        .border_color(rgba(t.border_strong))
        .shadow(vec![BoxShadow {
            color: rgba(t.shadow).into(),
            offset: point(px(0.0), px(3.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
        }])
        .occlude();

    // Group 1: Move / Lasso. Group 2: Paint bucket / Bend / Cut / Erase.
    for (i, (tool, icon, label)) in VECTOR_MAIN.into_iter().enumerate() {
        if i == 2 {
            row = row.child(vector_divider());
        }
        row = row.child(vector_tool_button(i, tool, icon, label, current, &on));
    }
    row = row.child(vector_divider()).child(vector_more_button(more_open, &on)).child(vector_divider());

    let on_done = on.clone();
    let done_tip: SharedString = format!("{}  Esc", tr("Done editing")).into();
    row = row.child(
        div()
            .id("vector-edit-done")
            .flex_none()
            .size(pt(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(pt(5.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(t.hover)))
            .tooltip(move |_window, cx| cx.new(|_| Tip { text: done_tip.clone() }).into())
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, w, cx| on_done(Intent::VectorDone, w, cx))
            .child(paint_icon(Icon::Close, rgba(t.icon), 24.0)),
    );

    if more_open {
        row = row.child(vector_more_card(current, &on));
    }

    // 12 pt above the canvas bottom + the 48 pt main pill + an 8 pt gap.
    div()
        .absolute()
        .left(px(canvas_left))
        .w(px(canvas_w.max(0.0)))
        .bottom(px(12.0 + PILL_H + 8.0))
        .flex()
        .flex_row()
        .justify_center()
        .child(row)
}

/// A one-line tooltip: the tool's name and its shortcut, on the menu's own card colours
/// (`ui-egui` shows `"Rectangle  R"` under the pointer).
pub(crate) struct Tip {
    pub(crate) text: SharedString,
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
    let color = rgba(if active { t.accent_on } else { t.icon });
    // Shape-tool marks are 24 pt vectors: let them occupy their own size instead of forcing them
    // into GLYPH, which would scale their (shared 1 pt) stroke up or down.
    let mark = if crate::widgets::icon::is_shape_tool(icon) {
        crate::widgets::icon::paint_tool(icon, color).into_any_element()
    } else {
        paint_icon(icon, color, GLYPH).into_any_element()
    };
    cell.child(mark)
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
        assert_eq!(width(GROUPS, false, false), expected);
        assert_eq!(width(GROUPS, false, false), 518.0);
    }

    /// A group shows its own tool when the current tool is in it, else the remembered one, else its
    /// first tool.
    #[test]
    fn a_group_shows_the_current_then_the_remembered_tool() {
        let move_group = &GROUPS[0];
        assert_eq!(shown(move_group, ToolId::Hand, &[]).0, ToolId::Hand);
        let last = [("move", ToolId::Scale)];
        assert_eq!(shown(move_group, ToolId::Rectangle, &last).0, ToolId::Scale, "remembered");
        assert_eq!(shown(move_group, ToolId::Rectangle, &[]).0, ToolId::Move, "first");
    }

    /// The flyout lists every tool of its group and checks the current one.
    #[test]
    fn flyout_checks_the_current_tool() {
        let rows = flyout_items(GROUPS, "shape", ToolId::Ellipse);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.iter().filter(|r| r.checked == Some(true)).count(), 1);
        assert_eq!(rows[3].id, "ellipse");
        assert!(rows.iter().all(|r| r.icon.is_some()));
        assert!(flyout_items(GROUPS, "nope", ToolId::Move).is_empty());
    }

    /// The checked tool keeps the hover fill lit (用户 2026-11-14：只有勾、没有颜色)。
    #[test]
    fn the_checked_tool_keeps_the_hover_fill_lit() {
        let rows = flyout_items(GROUPS, "shape", ToolId::Ellipse);
        assert_eq!(rows.iter().filter(|r| r.active_fill).count(), 1);
        assert!(rows.iter().all(|r| r.active_fill == (r.checked == Some(true))));
        // 其它菜单构造的是普通 Item —— 不显式打开就没有任何行常亮。
        assert!(!crate::menu_card::Item::new("plain", "Plain").active_fill);
    }

    /// Every group's flyout ticks the tool its own cell shows — even the groups that do not hold
    /// the current tool. Checking the raw current tool left every non-active group's flyout with no
    /// tick at all (`用户 2026-11-06`：点第 1 个有勾、点第 2 个没有勾).
    #[test]
    fn every_groups_flyout_ticks_the_tool_its_cell_shows() {
        // Default toolbar: Move is current, and each group remembers its first tool.
        let current = ToolId::Move;
        let last: Vec<(&'static str, ToolId)> = GROUPS.iter().map(|g| (g.key, g.tools[0].0)).collect();
        for g in GROUPS {
            let shown_tool = shown(g, current, &last).0;
            let rows = flyout_items(GROUPS, g.key, shown_tool);
            assert_eq!(rows.iter().filter(|r| r.checked == Some(true)).count(), 1, "group `{}` must tick exactly one row", g.key);
            assert!(
                rows.iter().any(|r| r.id == shown_tool.id() && r.checked == Some(true)),
                "group `{}` must tick the tool its cell shows ({})",
                g.key,
                shown_tool.id()
            );
            // Sanity: the old behaviour (ticking the raw current tool) leaves other groups blank.
            if !g.tools.iter().any(|(t, _)| *t == current) {
                assert!(
                    flyout_items(GROUPS, g.key, current).iter().all(|r| r.checked != Some(true)),
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
        assert_eq!(caret_left(GROUPS, "move"), Some(PAD + CELL + CARET_GAP));
        assert_eq!(group_left(GROUPS, "move"), Some(PAD));
        // frame sits one group further right (its width, the gap, then its own cell).
        assert_eq!(caret_left(GROUPS, "frame"), Some(PAD + group_w(&GROUPS[0]) + GAP + CELL + CARET_GAP));
        // `comment` is a single-tool group: no caret, no flyout.
        assert_eq!(caret_left(GROUPS, "comment"), None);
        assert_eq!(caret_left(GROUPS, "nope"), None);
        // The cells are centred in the 48 pt pill, so 8 pt of it is above a button's top edge; the
        // card's bottom edge sits 4 pt above that.
        assert_eq!(CELL_TOP, 8.0);
        assert_eq!(FLYOUT_GAP, 4.0);
        assert_eq!(CELL_TOP - FLYOUT_GAP, 4.0);
    }

    /// A board document gets its own nine groups, in the original's order
    /// (`ui-egui/src/toolbar.rs::BOARD_GROUPS`), and a different pill width than a design file.
    #[test]
    fn a_board_swaps_in_its_own_groups() {
        // `GROUPS` / `BOARD_GROUPS` are `const`s, so each use is its own promoted slice: compare
        // the contents, not the addresses.
        let keys_of = |gs: &[Group]| gs.iter().map(|g| g.key).collect::<Vec<_>>();
        assert_eq!(keys_of(groups(false)), keys_of(GROUPS));
        assert_eq!(keys_of(groups(true)), keys_of(BOARD_GROUPS));
        assert_eq!(BOARD_GROUPS.len(), 9);
        let keys: Vec<&str> = BOARD_GROUPS.iter().map(|g| g.key).collect();
        assert_eq!(keys, ["bmove", "ink", "sticky", "bshape", "connector", "text", "section", "stamp", "comment"]);
        // A caret for the multi-tool groups (`bmove`, `ink`) and for the one-tool groups that offer
        // choices instead (`sticky` → colours, `bshape` → shapes, `stamp` → stamps).
        let carets: Vec<&str> = BOARD_GROUPS.iter().filter(|g| g.has_flyout()).map(|g| g.key).collect();
        assert_eq!(carets, ["bmove", "ink", "sticky", "bshape", "stamp"]);
        // The board pill is wider than the design pill (nine groups instead of six, even though
        // its tail is two cells instead of four), so `width` must take the set.
        assert_ne!(width(BOARD_GROUPS, true, false), width(GROUPS, false, false));
        // Every board group's flyout (where it has one) ticks exactly one row.
        let last: Vec<(&'static str, ToolId)> = BOARD_GROUPS.iter().map(|g| (g.key, g.tools[0].0)).collect();
        for g in BOARD_GROUPS {
            let shown_tool = shown(g, ToolId::Move, &last).0;
            let rows = flyout_items(BOARD_GROUPS, g.key, shown_tool);
            assert_eq!(rows.len(), g.tools.len(), "group `{}` rows", g.key);
            assert_eq!(rows.iter().filter(|r| r.checked == Some(true)).count(), 1, "group `{}` tick", g.key);
        }
    }

    /// The vector edit row follows the saved Figma `Vector editing` secondary toolbelt:
    /// Move/Lasso │ Paint/Bend/Cut/Erase │ More │ Close. Figma's More is
    /// Shape builder + Variable width — Pen is not in this toolbar at all.
    #[test]
    fn vector_edit_row_uses_the_figma_groups() {
        let main: Vec<&str> = VECTOR_MAIN.iter().map(|(t, _, _)| t.id()).collect();
        assert_eq!(main, ["move", "lasso", "paint", "bend", "cut", "eraser"]);
        let more: Vec<&str> = vector_more_items().iter().map(|(id, _, _, _, _)| *id).collect();
        assert_eq!(more, ["shapeBuilder", "width"]);
        assert!(!main.contains(&"pen") && !more.contains(&"pen"));
        assert!(more.contains(&"width"), "variable width stays reachable");
    }

    /// Draw is a mode, not "the current tool happens to be a pencil": Design + Pen/Pencil
    /// must stay on the Design segment (and must not show the Draw width toolbar).
    #[test]
    fn draw_mode_is_a_mode_not_a_tool() {
        assert_eq!(active_mode(false, false, true), Some(0));
        assert_eq!(active_mode(false, false, false), Some(1));
        assert_eq!(active_mode(true, false, true), Some(2));
        assert_eq!(active_mode(false, true, false), Some(3));
    }

    /// Draw replaces the Pen group with the 182 pt nib strip: the pill grows by that strip
    /// minus the Pen group's own cell (the number of items is unchanged).
    #[test]
    fn draw_mode_widens_the_pill_by_the_nib_strip() {
        let pen = group_w(GROUPS.iter().find(|g| g.key == "pen").unwrap());
        let base = width(GROUPS, false, false);
        assert_eq!(width(GROUPS, false, true) - base, DRAW_SECTION_W - pen);
        // A board has no Draw mode, so its width is untouched.
        assert_eq!(width(BOARD_GROUPS, true, true), width(BOARD_GROUPS, true, false));
    }

    /// The three nib illustrations really decode into GPUI images. Draw mode calls this on its
    /// first frame, so a broken embedded PNG would panic the app instead of showing the strip.
    #[test]
    fn draw_nib_art_decodes() {
        let art = draw_art();
        for (i, (w, h)) in [(159u32, 324u32), (148, 319), (215, 340)].into_iter().enumerate() {
            assert_eq!(art[i].size(0).width.0, w as i32, "nib {i} width");
            assert_eq!(art[i].size(0).height.0, h as i32, "nib {i} height");
        }
    }
}
