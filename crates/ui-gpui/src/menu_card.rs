//! A menu card placed at a **point** — the shared body of the canvas context menu and the bottom
//! toolbar's flyouts.
//!
//! `widgets/menu.rs::menu_popup` hangs a menu off a trigger's rect (a dropdown's field), and
//! `main_menu.rs` is the multi-level catalog with hover-expanded submenus; neither can put a card
//! at the pointer, and the flyout's rows carry a **tool icon** besides the check column, which
//! `MenuRow` has no field for. So this module owns the one row model both need:
//!
//! ```text
//! ┌ card ─ 圆角 13、内边距 8、t.menu 底、1pt t.border ─────────────┐
//! │  row (28 / 32 高, 圆角 5)                                     │
//! │  ✓ 28 列（有勾选态才留） │ 16pt 图标 │ 标签 ……… 快捷键 │ ▸   │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! Geometry follows `ui-egui/src/menus.rs` (context menu: 28 pt rows) and
//! `ui-egui/src/toolbar.rs::flyout` (32 pt rows, check at 10, icon centred at 32, label from 48,
//! shortcut flush right 8). A row may open a submenu; the open one is supplied by the caller (it
//! is view state, so it survives the frame).

use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::{MENU_TRAIL_GAP, TEXT_PT, pt, rgba, tokens};
use gpui::prelude::*;
use gpui::{AnyElement, App, MouseButton, MouseDownEvent, SharedString, TextRun, Window, div};
pub(crate) use crate::widgets::Width;
use std::rc::Rc;

/// The card's inset (egui `menu_frame`: `Margin::same(8)`). The fill, the 13 pt corners and the
/// shadow come from [`crate::widgets::menu_surface`] — one definition for every popup.
const PAD: f32 = 8.0;
/// A menu item's own radius (egui `rect_filled(r, CornerRadius::same(5), …)`).
const ITEM_RADIUS: f32 = 5.0;
/// The check column: egui's label starts at `r.min.x + 28.0`.
const CHECK_W: f32 = 28.0;
/// The icon column's left edge and the label's, for a row that carries a tool mark.
const ICON_X: f32 = 24.0;
const ICON_SIZE: f32 = 16.0;
const ICON_LABEL_X: f32 = 48.0;
/// A separator's height: `my(4) + 1 + my(4)`.
const SEP_H: f32 = 9.0;

/// 一行菜单的模型：画布右键菜单、工具栏飞出菜单、Actions palette 共用。
/// （2026-11-06 恢复：该文件被并行编辑误删了这一段，而 `lib.rs` / `toolbar.rs` /
/// `context_menu.rs` 都仍按这套 API 调用。）
#[derive(Clone)]
pub struct Item {
    pub id: &'static str,
    /// The value handed to `OnPick` (defaults to `id`). Dynamic rows — an Actions palette
    /// component, style, plugin or widget — carry a runtime key here instead of a static id.
    pub key: SharedString,
    pub label: SharedString,
    pub checked: Option<bool>,
    pub disabled: bool,
    pub icon: Option<Icon>,
    pub trail: Option<SharedString>,
    pub separator_before: bool,
    pub sub: Vec<Item>,
}

impl Item {
    pub fn new(id: &'static str, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            key: SharedString::from(id),
            label: label.into(),
            checked: None,
            disabled: false,
            icon: None,
            trail: None,
            separator_before: false,
            sub: Vec::new(),
        }
    }
    /// Override the value handed to `OnPick` (defaults to `id`).
    pub fn key(mut self, k: impl Into<SharedString>) -> Self {
        self.key = k.into();
        self
    }
    pub fn disabled(mut self, b: bool) -> Self {
        self.disabled = b;
        self
    }
    pub fn checked(mut self, c: Option<bool>) -> Self {
        self.checked = c;
        self
    }
    pub fn icon(mut self, i: Icon) -> Self {
        self.icon = Some(i);
        self
    }
    pub fn trail(mut self, t: impl Into<SharedString>) -> Self {
        self.trail = Some(t.into());
        self
    }
    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
    pub fn sub(mut self, items: Vec<Item>) -> Self {
        self.sub = items;
        self
    }
}

/// 选中一项（送命令 id）与展开子菜单（送行下标）的回调。
pub type OnPick = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type OnSub = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// 标签的左内缩：无勾选列 8；有勾选列时普通行 28、带图标的工具行 48。
fn label_x(item: &Item, has_check: bool) -> f32 {
    if !has_check {
        8.0
    } else if item.icon.is_some() {
        ICON_LABEL_X
    } else {
        CHECK_W
    }
}

/// 卡片内容高度：上下各 8pt 内边距 + 每行行高 + 每条分隔线 9pt。
pub(crate) fn height(items: &[Item], row_h: f32) -> f32 {
    2.0 * PAD + items.len() as f32 * row_h + items.iter().filter(|i| i.separator_before).count() as f32 * SEP_H
}

/// 最宽一行的文字宽度（标签内缩 + 标签 + 可选的快捷键列）。
fn widest(items: &[Item], has_check: bool, window: &Window) -> f32 {
    let measure = |s: &SharedString| -> f32 {
        let run = TextRun {
            len: s.len(),
            font: window.text_style().font(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(window.text_system().shape_line(s.clone(), pt(TEXT_PT), &[run], None).width)
    };
    items
        .iter()
        .map(|i| {
            let mut w = label_x(i, has_check) + measure(&i.label) + 10.0;
            if let Some(trail) = &i.trail {
                if !trail.is_empty() {
                    w += MENU_TRAIL_GAP + measure(trail) + 16.0;
                }
            }
            w
        })
        .fold(0.0f32, f32::max)
        .max(120.0)
}

/// The rows alone, with no card chrome (the Actions palette puts them under its search field).
///
/// Returns the rows **and** any open submenu cards as two separate vecs: a scrollable card puts the
/// rows inside its scroll viewport, while the submenus stay direct children of the surface (their
/// `absolute` placement is in window coordinates and must not scroll with the rows).
#[allow(clippy::too_many_arguments)]
pub(crate) fn rows(
    items: &[Item],
    row_h: f32,
    has_check: bool,
    open_sub: Option<usize>,
    left: f32,
    top: f32,
    width: f32,
    on_pick: OnPick,
    on_sub: OnSub,
    // 悬停某一行时回调（`MenuAim` 的输入）：子菜单的展开 / 收起由通用状态机裁决，
    // 不再靠点击切换。工具栏飞出 / palette 传空闭包即可。
    on_hover: OnSub,
    window: &Window,
    vh: f32,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let t = tokens();
    let mut out: Vec<AnyElement> = Vec::new();
    let mut subs: Vec<AnyElement> = Vec::new();
    let mut y = top + PAD;
    for (i, item) in items.iter().enumerate() {
        if item.separator_before {
            out.push(div().my(pt(4.0)).h(pt(1.0)).w_full().bg(rgba(t.border)).into_any_element());
            y += SEP_H;
        }
        let key = item.key.clone();
        let has_sub = !item.sub.is_empty();
        let inset = label_x(item, has_check);
        // This row's top — the submenu is pinned to it.
        let row_top = y;

        let mut row = div()
            .relative()
            .h(pt(row_h))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .rounded(pt(ITEM_RADIUS))
            .pl(pt(inset))
            .pr(pt(10.0))
            .text_size(pt(TEXT_PT))
            .text_color(rgba(if item.disabled { t.menu_dim } else { t.menu_text }));
        if !item.disabled {
            row = row.hover(|s| s.bg(rgba(t.menu_hover))).cursor_pointer();
            let hover = on_hover.clone();
            row = row.on_mouse_move(move |_e, window, cx| hover(i, window, cx));
            let pick = on_pick.clone();
            let sub = on_sub.clone();
            row = row.on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                if has_sub {
                    // A submenu row opens its card instead of running a command.
                    sub(i, window, cx);
                } else {
                    pick(key.as_ref(), window, cx);
                }
            });
        }
        // The check mark and the icon are absolutely placed so the label's own inset stays exact.
        if has_check {
            row = row.child(div().absolute().left(pt(6.0)).top(pt(0.0)).h(pt(row_h)).flex().items_center().child(if item.checked == Some(true) {
                "✓"
            } else {
                ""
            }));
        }
        if let Some(icon) = item.icon {
            row = row.child(div().absolute().left(pt(ICON_X)).top(pt(0.0)).h(pt(row_h)).flex().items_center().child(paint_icon(
                icon,
                rgba(if item.disabled { t.menu_dim } else { t.menu_text }),
                ICON_SIZE,
            )));
        }
        row = row.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().whitespace_nowrap().child(item.label.clone()));
        if has_sub {
            row = row.child(div().ml(pt(MENU_TRAIL_GAP)).whitespace_nowrap().text_color(rgba(t.menu_dim)).child("▸"));
        } else if let Some(trail) = &item.trail {
            // `用户 2026-11-06`：无快捷键就不预留占位（避免「项内右侧空一截」）。
            if !trail.is_empty() {
                row = row.child(div().ml(pt(MENU_TRAIL_GAP)).whitespace_nowrap().text_color(rgba(t.menu_dim)).child(trail.clone()));
            }
        }
        out.push(row.into_any_element());
        y += row_h;
        // The open submenu hangs off this row: right of the card, its first row level with this one
        // (egui `submenu_at`: `pos2(r.max.x + gap, r.min.y - 8.0)`).
        if !item.sub.is_empty() && open_sub == Some(i) {
            // 子卡片（目前最多一级）不再继续嵌套瞄准：传空悬停回调，避免行下标串级。
            subs.push(card(
                left + width + 4.0,
                row_top - PAD,
                Width::Min(width),
                row_h,
                &item.sub,
                None,
                on_pick.clone(),
                on_sub.clone(),
                Rc::new(|_, _, _| {}),
                window,
                vh,
            ));
        }
    }
    (out, subs)
}

/// The card at `(left, top)` (window coordinates).
///
/// `sizing` is the shared `Menu` control's width policy (`用户 2026-11-06`): `Width::Min(f)` 固定
/// 下限但内容更宽时以最宽那项为准；`Width::Fit` 就是最宽那项；`Width::Anchor` 跟随触发控件。
/// 宽度恒为确定值，所以每行的 `w_full` 都填充到同一宽度。
///
/// When `top + content` would run past the viewport bottom, the card pins its bottom to `vh` and
/// scrolls internally — a 24 pt affordance (a centred 24×24 down chevron) replaces the 8 pt bottom
/// padding, and the scrollbar is hidden; none of the items ever show a drag thumb.
#[allow(clippy::too_many_arguments)]
pub(crate) fn card(
    left: f32,
    top: f32,
    sizing: Width,
    row_h: f32,
    items: &[Item],
    open_sub: Option<usize>,
    on_pick: OnPick,
    on_sub: OnSub,
    on_hover: OnSub,
    window: &Window,
    vh: f32,
) -> AnyElement {
    let has_check = items.iter().any(|r| r.checked.is_some());
    let content_w = widest(items, has_check, window);
    let rows_h = height(items, row_h) - 2.0 * PAD;
    let w = match sizing {
        Width::Fit => content_w + 2.0 * PAD,
        Width::Min(f) => (content_w + 2.0 * PAD).max(f),
        Width::Anchor => (content_w + 2.0 * PAD).max(48.0),
    };
    let (row_els, sub_els) = rows(items, row_h, has_check, open_sub, left, top, w, on_pick, on_sub, on_hover, window, vh);
    // 唯一的浮层控件：宽度三档、锚点跟随、上下防溢出与滚动全在 `Menu::show` 里。
    crate::widgets::menu(crate::widgets::at_point(left, top))
        .gap(0.0)
        .width(sizing)
        .viewport(0.0, vh) // left 已由调用方按当前窗口算好
        .content(content_w, rows_h, row_h)
        .tag(format!("card-{}-{}", left as i32, top as i32))
        .extras(sub_els)
        .show(|| div().flex().flex_col().children(row_els).into_any_element())
        .unwrap_or_else(|| div().into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flyout sits above its trigger, so the card's height must be predictable from the rows:
    /// chrome 8 + 8, one 32 pt row each, a divider 9 above its row.
    #[test]
    fn card_height_is_chrome_plus_rows() {
        let rows = vec![Item::new("a", "A"), Item::new("b", "B"), Item::new("c", "C").separator_before()];
        assert_eq!(height(&rows, 32.0), 16.0 + 3.0 * 32.0 + 9.0);
        assert_eq!(height(&[], 32.0), 16.0);
    }

    /// The label's inset: a checked row puts it after the 28 pt check column, a tool row after the
    /// 16 pt icon (which is centred at 32 under egui's flyout metrics).
    #[test]
    fn label_inset_follows_the_check_and_icon_columns() {
        let plain = Item::new("a", "A");
        assert_eq!(label_x(&plain, false), 8.0);
        assert_eq!(label_x(&plain, true), CHECK_W);
        let tool = Item::new("b", "B").icon(Icon::Move);
        assert_eq!(label_x(&tool, true), ICON_LABEL_X);
        assert_eq!(ICON_X + ICON_SIZE, 40.0);
    }

}
