// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// 2026-11-07 用户明确授权本次解冻：为通用菜单控件加入「安全三角 / 子菜单瞄准意图」
// （已抽到独立行为文件 `widgets/menu_aim.rs`，主菜单与右键菜单共用）。本次只新增能力，
// 上面三条不变量与既有行控件行为一律未动；移植来源为本项目 `ui-egui/src/menus/tree.rs`
// 与 `ui-egui/src/menu_aim_tests.rs`。
//
// 2026-11-08 用户指出「弹出的菜单是有防溢出能力的，为什么有些项还是溢出了」：
// `menu_popup` 从未把视口尺寸和真实内容高度交给 `Menu`，`fit_vertical` / 内部滚动 /
// 水平贴边整条链路一直是关着的，调用方只能用粗糙的估计值手工夹一下。本次新增
// `menu_popup_in`（带视口），把防溢出真正接到控件上；`menu_popup` 保持无视口的旧行为。
// ============================================================================
//! Popover menus + colour picker for the GPUI frontend.
//!
//! Both are *floating layers*: an absolute-positioned frame anchored below a trigger, painted on top
//! of the panel (the root view renders them last). A full-window transparent backdrop catches
//! outside clicks; rows highlight on hover; a click picks an item and dismisses.

use crate::canvas::geom::Color32;
use crate::widgets::input::text_field::TextField;
use crate::widgets::{pt, rgba, tokens};
use gpui::{AnyElement, App, Bounds, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window, div, prelude::*};
use std::rc::Rc;

/// One row of a popover menu: a label, an optional right-aligned trailing text (shortcut), an
/// optional check mark, and an optional divider above it (a menu group).
#[derive(Clone)]
pub struct MenuRow {
    pub label: SharedString,
    pub trail: Option<SharedString>,
    /// `Some(true)` draws the design's check mark (`✓ `), `Some(false)` reserves its width so
    /// plain rows line up with checked ones, `None` is a plain row. Mirrors egui's `menu_check` /
    /// `menu_item_beside_checks`.
    pub checked: Option<bool>,
    /// Draw a 1 pt divider above this row — egui's `ui.separator()` between menu groups.
    pub separator_before: bool,
}

impl MenuRow {
    /// A plain row with just a label (no trailing shortcut text).
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), trail: None, checked: None, separator_before: false }
    }

    /// A check row, or (with `None`) a plain row that keeps the check column's width.
    pub fn checked(mut self, on: Option<bool>) -> Self {
        self.checked = on;
        self
    }

    /// Start a new menu group: a divider above this row.
    pub fn separator(mut self) -> Self {
        self.separator_before = true;
        self
    }

    /// The right-aligned trailing text (a shortcut).
    pub fn trail(mut self, text: impl Into<SharedString>) -> Self {
        self.trail = Some(text.into());
        self
    }
}

/// Build a popover menu frame. `origin` is the popover's top-left in window coordinates; `min_w`
/// is its minimum width (the trigger's width, so it lines up under the field). Rows highlight on
/// hover and, on click, call `on_pick(label)` then `dismiss`.
pub fn menu_popup(
    anchor: Bounds<Pixels>,
    // `Some(w)` overrides the width (`paints.rs` passes `200.0` for the blend menu); `None` is the
    // original's `anchor.rect.width().max(MENU_MIN_W)`.
    width_override: Option<f32>,
    rows: Vec<MenuRow>,
    on_pick: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    menu_popup_in(anchor, width_override, rows, on_pick, dismiss, 0.0, 0.0)
}

/// 菜单防溢出要用的两个测量值：**真实内容高度**（不含卡片 padding）与**最宽行估算宽**。
/// 抽成纯函数，好让「行 28pt / 分隔行 9pt」这条规则有数值断言守着——以前手工估计时
/// 分隔符和卡片 8pt padding 都漏算，靠近屏幕底部时最后几项就溢出去了。
pub(crate) fn menu_metrics(rows: &[MenuRow]) -> (f32, f32) {
    let rows_h = rows.len() as f32 * 28.0
        + rows.iter().filter(|r| r.separator_before).count() as f32 * 9.0;
    let content_w = rows
        .iter()
        .map(|r| {
            let label: f32 = r.label.chars().map(|c| if c.is_ascii() { 6.5 } else { 11.0 }).sum();
            let trail: f32 = r.trail.as_ref().map(|s| s.chars().count() as f32 * 6.5 + 12.0).unwrap_or(0.0);
            label + trail + if r.checked.is_some() { 28.0 } else { 8.0 } + 10.0
        })
        .fold(0.0f32, f32::max);
    (rows_h, content_w)
}

/// 带**视口**的版本：把 `vw/vh` 和真实内容高度交给通用 `Menu`，启用它自带的
/// 上下防溢出 / 内部滚动 / 水平贴边（`fit_vertical` + `fit_rows`）。
/// 旧路径没传这些，于是 `rows_h` 一直是 0、`Menu::show` 的整条防溢出链路从不生效——
/// 项多或靠近屏幕底部时菜单照样溢出（用户 2026-11-08）。
pub fn menu_popup_in(
    anchor: Bounds<Pixels>,
    width_override: Option<f32>,
    rows: Vec<MenuRow>,
    on_pick: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    vw: f32,
    vh: f32,
) -> AnyElement {
    let t = tokens();
    // `MENU_ROW_H = 28`（`ui-egui/src/menus/tree.rs:351` 的 `allocate_exact_size(vec2(w, 28.0))`）。
    // 原为 24，与 egui 的 28 不一致 —— 2026-11-05 用户规定外框规格时一并纠正。
    let row_h = pt(28.0);
    // `MENU_MIN_W = 48`: a menu is never narrower than this, whatever its trigger.
    let width = width_override.unwrap_or_else(|| f32::from(anchor.size.width).max(48.0));
    // ⛔ 不要在这里加 `.w_full()`：卡片的宽度是 `min_w(width)` + 内容撑开（**不定宽**），
    // `width: 100%` 在不定宽父级里会退化成 auto，于是每一行只有自己文字的宽度、整排
    // 参差不齐（「项没填充满宽度」就是这个）。不加宽度时它是 flex 子项，靠 `align-items:
    // stretch` 撑满卡片内容盒，行再 `w_full` 就正好填满。
    let mut col = div().flex().flex_col();

    // 交给通用 `Menu` 做防溢出用的两个测量值（真实内容高度 / 最宽行估算宽）。
    // 以前这里根本没传，`rows_h` 默认 0，`fit_vertical` 于是从不滚动、也不上移。
    let (rows_h, content_w) = menu_metrics(&rows);

    // 这个菜单里**有**可勾选的行才留 28pt 勾选列；一条都没有就不留（否则左边白空一条缩进）。
    let has_check = rows.iter().any(|r| r.checked.is_some());
    for r in rows {
        if r.separator_before {
            col = col.child(div().my(pt(4.0)).h(pt(1.0)).w_full().bg(rgba(t.border)));
        }
        // ⛔ 勾选列是**固定 28pt 的一列**，不是文字前缀。egui `widgets::menu_row_draw`：
        // 勾号画在 `r.min.x + 12`（12×12），文字起点是 `pos2(r.min.x + 28.0 + indent, …)`。
        // 之前用 `"✓ "` / `"    "` 前缀去顶，勾与不勾的字宽不同、缩进对不上，整列是歪的
        // （用户 2026-11-05 指出）。没有勾选态的行不占这一列。
        let check = if r.checked == Some(true) { "✓" } else { "" };
        let pick_text = r.label.to_string();
        let label = r.label;
        let trail = r.trail;
        let on_pick = on_pick.clone();
        let dismiss = dismiss.clone();
        let mut row = div()
            .h(row_h)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .rounded(pt(5.0))
            .text_color(rgba(t.menu_text))
            .text_size(pt(crate::widgets::TEXT_PT))
            .hover(|s| s.bg(rgba(t.menu_hover)))
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                on_pick(pick_text.as_str(), window, cx);
                dismiss(window, cx);
            });
        if has_check {
            // 勾号靠左 6pt（字形约 12 宽，中心≈12，与 egui 的 `r.min.x + 12` 一致）。
            row = row.child(div().w(pt(28.0)).flex_none().flex().flex_row().items_center().pl(pt(6.0)).child(check));
        } else {
            // 没有勾选列时左边至少留 8pt —— 不能把文字顶到行首。
            row = row.pl(pt(8.0));
        }
        // 标签占满剩余宽度，超长裁掉（egui 把文字裁到 `r.max.x - 8.0`）。
        row = row.child(div().flex_1().min_w(pt(0.0)).overflow_hidden().whitespace_nowrap().child(label)).when_some(trail, |d, tr| {
            d.child(div().ml(pt(crate::widgets::MENU_TRAIL_GAP)).pr(pt(10.0)).whitespace_nowrap().text_color(rgba(t.menu_dim)).child(tr))
        });
        col = col.child(row);
    }
    // The rows go on the one shared `Menu` control; no second popup chrome here.
    // `viewport` + `content` 才把控件自带的防溢出接通：顶部夹紧、水平贴边、装不下时
    // 内部滚动（`Menu::show` → `fit_vertical` / `fit_rows`）。
    crate::widgets::menu(anchor)
        .width(crate::widgets::Width::Min(width))
        .viewport(vw, vh)
        .content(content_w, rows_h, 28.0)
        .show(|| col.into_any_element())
        .unwrap_or_else(|| div().into_any_element())
}

/// Build a colour picker popover — the port of `ui-egui/src/widgets/colorpicker.rs`'s body:
/// a 240 pt saturation/value square, a 240 × 28 hue row (the bar is `[48, 240-16]` wide) and a
/// 240 × 20 alpha row, then the hex field and the page colours.
///
/// The two sliders sit at `+48` because the eyedropper button owns the first 36 pt of their rows.
/// The hue bar is twelve two-stop gradients (egui meshes twelve too); the alpha bar is the
/// checkerboard under a transparent→opaque gradient.
pub fn color_picker(
    origin: Point<Pixels>,
    current: Color32,
    hex_field: Entity<TextField>,
    on_pick: Rc<dyn Fn(Color32, &mut Window, &mut App)>,
    dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    let t = tokens();
    let c = figcraft_color::Color::from_rgba8(current.0[0], current.0[1], current.0[2], current.0[3]);
    let hsb = figcraft_color::Hsb::from_color(c);
    let hue_c = hsb_color(hsb.h, 1.0, 1.0);

    // 240 pt of content, 12 pt padding each side (`colorpicker.rs` allocates 240-wide rows).
    let mut col = div()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(pt(240.0 + 24.0))
        .flex()
        .flex_col()
        .gap(pt(4.0))
        .p(pt(12.0))
        .bg(rgba(t.menu))
        .rounded(pt(6.0))
        .border_1()
        .border_color(rgba(t.border))
        .occlude();

    // ── saturation / value square, 240 × 240 ────────────────────────────────────────────
    // White → hue horizontally, then transparent → black vertically (egui's two meshes).
    let sv_hit = Rc::new(std::cell::Cell::new(None::<gpui::Bounds<Pixels>>));
    let sv_click = sv_hit.clone();
    let hue_for_pick = hsb.h;
    let on_sv = on_pick.clone();
    let sv = div()
        .relative()
        .w(pt(240.0))
        .h(pt(240.0))
        .child(
            gpui::canvas(
                move |bounds, _window, _cx| {
                    sv_hit.set(Some(bounds));
                },
                |_bounds, _, _window, _cx| {},
            )
            .size_full(),
        )
        .child(div().absolute().size_full().bg(gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(gpui::white(), 0.0),
            gpui::linear_color_stop(rgba(hue_c), 1.0),
        )))
        .child(div().absolute().size_full().bg(gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(gpui::transparent_black(), 0.0),
            gpui::linear_color_stop(gpui::black(), 1.0),
        )))
        // The knob: a 12 pt ring, white inside a black hairline.
        .child(
            div()
                .absolute()
                .left(pt(hsb.s * 240.0 - 6.0))
                .top(pt((1.0 - hsb.b) * 240.0 - 6.0))
                .size(pt(12.0))
                .rounded(pt(6.0))
                .border_2()
                .border_color(gpui::white()),
        )
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
            if let Some(b) = sv_click.get() {
                let x0 = f32::from(b.origin.x);
                let y0 = f32::from(b.origin.y);
                let w = f32::from(b.size.width).max(1.0);
                let h = f32::from(b.size.height).max(1.0);
                let s = ((f32::from(e.position.x) - x0) / w).clamp(0.0, 1.0);
                let v = 1.0 - ((f32::from(e.position.y) - y0) / h).clamp(0.0, 1.0);
                let picked = hsb_color(hue_for_pick, s, v);
                on_sv(picked, window, cx);
            }
        });
    col = col.child(sv);

    // ── hue row: 240 × 28, the bar from +48 to -16 (176 wide), 12 two-stop segments ──────
    let hue_hit = Rc::new(std::cell::Cell::new(None::<gpui::Bounds<Pixels>>));
    let hue_click = hue_hit.clone();
    let on_hue = on_pick.clone();
    let seg = 176.0 / 12.0;
    let mut hue_bar = div().relative().w(pt(176.0)).h(pt(12.0)).flex().flex_row().rounded(pt(6.0)).overflow_hidden().child(
        gpui::canvas(
            move |bounds, _window, _cx| {
                hue_hit.set(Some(bounds));
            },
            |_bounds, _, _window, _cx| {},
        )
        .size_full(),
    );
    for i in 0..12 {
        let a = i as f32 / 12.0;
        let b = (i + 1) as f32 / 12.0;
        hue_bar = hue_bar.child(div().flex_none().w(pt(seg)).h_full().bg(gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(rgba(hsb_color(a * 360.0, 1.0, 1.0)), 0.0),
            gpui::linear_color_stop(rgba(hsb_color(b * 360.0, 1.0, 1.0)), 1.0),
        )));
    }
    let hue_row = div()
        .relative()
        .w(pt(240.0))
        .h(pt(28.0))
        .flex()
        .items_center()
        .pl(pt(48.0))
        .pr(pt(16.0))
        .child(hue_bar.on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
            if let Some(b) = hue_click.get() {
                let x0 = f32::from(b.origin.x);
                let w = f32::from(b.size.width).max(1.0);
                let h = ((f32::from(e.position.x) - x0) / w).clamp(0.0, 1.0) * 360.0;
                let picked = hsb_color(h, hsb.s, hsb.b);
                on_hue(picked, window, cx);
            }
        }))
        .child(
            div()
                .absolute()
                .left(pt(48.0 + hsb.h / 360.0 * 176.0 - 6.0))
                .top(pt(14.0 - 6.0))
                .size(pt(12.0))
                .rounded(pt(6.0))
                .border_2()
                .border_color(gpui::white()),
        );
    col = col.child(hue_row);

    // ── alpha row: 240 × 20, the bar from +48 to -16, checker under transparent→opaque ───
    let alpha_hit = Rc::new(std::cell::Cell::new(None::<gpui::Bounds<Pixels>>));
    let alpha_click = alpha_hit.clone();
    let on_alpha = on_pick.clone();
    let alpha_bar = div()
        .relative()
        .w(pt(176.0))
        .h(pt(12.0))
        .rounded(pt(6.0))
        .overflow_hidden()
        .child(
            gpui::canvas(
                move |bounds, _window, _cx| {
                    alpha_hit.set(Some(bounds));
                },
                move |bounds, _, window, _cx| crate::widgets::swatch::checker(window, bounds, 4.0),
            )
            .size_full(),
        )
        .child(div().absolute().size_full().bg(gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(gpui::transparent_black(), 0.0),
            gpui::linear_color_stop(rgba(current.with_alpha(255)), 1.0),
        )))
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
            if let Some(b) = alpha_click.get() {
                let x0 = f32::from(b.origin.x);
                let w = f32::from(b.size.width).max(1.0);
                let a = ((f32::from(e.position.x) - x0) / w).clamp(0.0, 1.0);
                let picked = current.with_alpha((a * 255.0).round() as u8);
                on_alpha(picked, window, cx);
            }
        });
    let alpha_row = div().relative().w(pt(240.0)).h(pt(20.0)).flex().items_center().pl(pt(48.0)).pr(pt(16.0)).child(alpha_bar).child(
        div()
            .absolute()
            .left(pt(48.0 + (f32::from(current.0[3]) / 255.0) * 176.0 - 6.0))
            .top(pt(10.0 - 6.0))
            .size(pt(12.0))
            .rounded(pt(6.0))
            .border_2()
            .border_color(gpui::white()),
    );
    col = col.child(alpha_row);

    // The hex field, as before.
    col = col.child(div().w(pt(240.0)).h(pt(28.0)).flex().items_center().child(hex_field));

    // Page colours: a small preset palette (Figma's "Document colors" row).
    let palette: [Color32; 12] = [
        Color32::from_rgb(0x00, 0x00, 0x00),
        Color32::from_rgb(0xFF, 0xFF, 0xFF),
        Color32::from_rgb(0xEB, 0x4D, 0x4D),
        Color32::from_rgb(0xF5, 0xA6, 0x23),
        Color32::from_rgb(0x4C, 0xAF, 0x50),
        Color32::from_rgb(0x2D, 0x9C, 0xDB),
        Color32::from_rgb(0x5B, 0x8D, 0xE6),
        Color32::from_rgb(0x9B, 0x59, 0xB6),
        Color32::from_rgb(0x34, 0x49, 0x5E),
        Color32::from_rgb(0x7F, 0x8C, 0x8D),
        Color32::from_rgb(0xE9, 0x1E, 0x63),
        Color32::from_rgb(0x00, 0xBF, 0xA5),
    ];
    let mut grid = div().w(pt(240.0)).grid().grid_cols(6).gap(pt(6.0));
    for c in palette {
        let on_pick = on_pick.clone();
        let dismiss = dismiss.clone();
        grid = grid.child(
            div()
                .size(pt(20.0))
                .rounded(pt(3.0))
                .bg(rgba(c))
                .border_1()
                .border_color(rgba(t.border))
                .hover(|s| s.border_color(rgba(t.text)))
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                    on_pick(c, window, cx);
                    dismiss(window, cx);
                }),
        );
    }
    col.child(grid).into_any_element()
}

/// `Hsb` → `Color32` at `s = b = 1` (a hue), or at an explicit saturation/value.
fn hsb_color(h: f32, s: f32, b: f32) -> Color32 {
    crate::widgets::swatch::color32(figcraft_color::Hsb { h, s, b }.to_color())
}

/// The full-window transparent backdrop that closes the popover on an outside click.
pub fn popup_backdrop(dismiss: Rc<dyn Fn(&mut Window, &mut App)>) -> AnyElement {
    div().absolute().size_full().on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| dismiss(window, cx)).into_any_element()
}

// 子菜单「安全三角」/ 瞄准意图已抽到独立行为文件 `widgets/menu_aim.rs`（通用，任何菜单共用）。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_count_toward_the_height_the_overflow_guard_uses() {
        let rows = vec![
            MenuRow::new("Fit"),
            MenuRow::new("Fill").separator(),
            MenuRow::new("100%").checked(Some(true)),
        ];
        let (rows_h, content_w) = menu_metrics(&rows);
        // 3×28 + 一条分隔行 9（my(4)+1+my(4)）。旧的手工估计只有 rows*28+16，
        // 既漏了分隔行、又把卡片 padding 当成了行数，靠近屏幕底部时最后几项会溢出。
        assert_eq!(rows_h, 3.0 * 28.0 + 9.0);
        assert!(content_w > 0.0, "最宽行必须有内容宽，否则卡片会被压到 16pt、文字全被裁掉");
    }

    #[test]
    fn a_long_label_widens_the_content_estimate() {
        let short = menu_metrics(&[MenuRow::new("Fit")]).1;
        let long = menu_metrics(&[MenuRow::new("Zoom to selection 100%")]).1;
        assert!(long > short, "长标签要能把卡片撑宽，不然会被 overflow_hidden 裁掉");
        // 中文按全角 11pt 估宽，不该被当成 6.5pt 的窄 ASCII。
        let cn = menu_metrics(&[MenuRow::new("显示所有类型")]).1;
        let en = menu_metrics(&[MenuRow::new("Show all")]).1;
        assert!(cn > en, "中文按全角估宽：{cn} 应大于 {en}");
    }

    #[test]
    fn an_empty_menu_has_no_height() {
        assert_eq!(menu_metrics(&[]).0, 0.0);
    }
}
