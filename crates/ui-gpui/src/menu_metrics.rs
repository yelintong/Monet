//! 所有菜单共用的一套规格（用户 2026-11-14）：行高 24、分隔线通边、白色百分比墨色。
//!
//! 画布右键菜单、工具栏飞出菜单、Actions palette、主菜单、下拉菜单都读这里，不再各自写
//! 28/32/8/9 或 `t.border` / `t.menu_text`。卡片左右各有 [`MENU_PAD`] 的内边距，所以分隔线
//! 必须从内容区左边缘往左挪一个 `MENU_PAD`、再按**卡片完整宽度**画，才能真正通到两边。

use crate::canvas::geom::Color32;
use crate::widgets::{pt, rgba};
use gpui::{IntoElement, Rgba, div, prelude::*};

/// Every menu row, in points.
pub(crate) const MENU_ROW_H: f32 = 24.0;
/// A separator's total height: 4 pt above + 1 pt rule + 4 pt below.
pub(crate) const MENU_SEP_H: f32 = 9.0;
/// The card's inset; the divider cancels it with a negative margin.
pub(crate) const MENU_PAD: f32 = 8.0;

fn white(pct: f32) -> Rgba {
    rgba(Color32::from_white_alpha((255.0 * pct).round() as u8))
}

/// Labels, check marks and leading icons: white 90%.
pub(crate) fn ink() -> Rgba {
    white(0.9)
}

/// Trailing shortcut hints and the submenu chevron: white 70%.
pub(crate) fn trail_ink() -> Rgba {
    white(0.7)
}

/// Dividers: white 10%.
pub(crate) fn divider_ink() -> Rgba {
    white(0.1)
}

/// A 1 pt rule that reaches the card's left and right edges. `card_w` is the **full** card
/// width (including both paddings); the line starts one `MENU_PAD` to the left of the content
/// box, so a parent padding can never leave a gap at either end.
pub(crate) fn divider(card_w: f32) -> impl IntoElement {
    div()
        .h(pt(MENU_SEP_H))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .child(div().ml(pt(-MENU_PAD)).w(pt(card_w)).h(pt(1.0)).flex_none().bg(divider_ink()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_row_is_24_and_the_palette_is_white_percentages() {
        assert_eq!(MENU_ROW_H, 24.0);
        assert_eq!(MENU_SEP_H, 9.0);
        assert!((ink().a - 0.9).abs() < 0.01);
        assert!((trail_ink().a - 0.7).abs() < 0.01);
        assert!((divider_ink().a - 0.1).abs() < 0.01);
        assert_eq!(ink().r, 1.0);
    }
}
