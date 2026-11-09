// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! The input control series for the GPUI frontend.
//!
//! * [`EditBuffer`] is the shared editing kernel (caret, selection, grapheme/word motion, IME
//!   composition, UTF-16 conversion). It is backend-agnostic and has no `Session` dependency.
//! * [`TextInput`] is the *most primitive* GPUI input: `shape_line` + `closest_index_for_x` for
//!   hit-testing, `x_for_index` for caret/selection quads, `ElementInputHandler` for the platform
//!   IME.
//! * [`text_field::TextField`] is the **field shell**: fill, radius, hover/focus border, lead/trail
//!   [`Slot`]s, the 8 pt editor inset, commit-on-Enter/blur, Escape, select-all-on-focus and the
//!   number-field behaviours.
//!
//! One `Slot` vocabulary covers every input (icon, short label, colour dot, custom content).

pub mod edit_buffer;
pub mod field;
pub mod number_field;
pub mod template;
pub mod text_edit;
pub mod text_field;
pub mod text_input;

pub use edit_buffer::{Composed, EditBuffer, EditOutcome, char_to_utf16_str, utf16_to_char_str};
pub use text_edit::{NumberConfig, TextEdit, fmt_num};
pub use text_field::TextField;
pub use text_input::{TextAlign, TextInput};

use crate::canvas::geom::Color32;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::text_input::centred_line_origin;
use crate::widgets::{ICON, TEXT_PT, pt, rgba, tokens};
use gpui::{AnyElement, Bounds, Edges, SharedString, TextRun, canvas, div, point, prelude::*, px, size};
use std::rc::Rc;

/// A slot at the leading or trailing end of a field: nothing, a short label, an icon, a colour dot,
/// a paint swatch, or arbitrary content.
///
/// Like `ui-egui::Slot` this is a **description**, not a one-shot element: it is `Clone`, and
/// `Custom` holds a factory that is re-invoked on every frame (GPUI elements are consumed by paint,
/// so they cannot be stored and re-used). That lets a field keep its slots across re-renders.
#[derive(Clone)]
pub enum Slot {
    None,
    Text(SharedString),
    Icon(Icon),
    /// A solid colour dot (`Field-color`): a 14 pt rounded square with a hairline.
    Dot(Color32),
    Custom(Rc<dyn Fn() -> AnyElement>),
}

impl Slot {
    /// Whether the slot should be drawn (everything except `None`).
    pub fn has_content(&self) -> bool {
        !matches!(self, Slot::None)
    }

    /// Arbitrary slot content. `f` is called again on each frame to rebuild the element.
    pub fn custom(f: impl Fn() -> AnyElement + 'static) -> Self {
        Slot::Custom(Rc::new(f))
    }
}

/// Build the inner content of `slot` for this frame. Borrows `slot` so the description survives
/// re-renders; a `Custom` factory is called once per frame.
pub fn slot_child(slot: &Slot) -> Option<AnyElement> {
    let t = tokens();
    match slot {
        Slot::None => None,
        // A short text lead (`X`, `Y`, `W`, `R`): shaped in a canvas and centred in the 24 pt slot
        // like the editor (`centred_line_origin`). Under `FC_DEBUG_LAYOUT` it gets the same 1 px red
        // border as the section title's `dbg_frame` — drawn around the glyph box, not the slot.
        Slot::Text(s) => {
            let text = s.clone();
            let color = rgba(t.text_secondary).into();
            let dbg = std::env::var_os("FC_DEBUG_LAYOUT").is_some();
            Some(
                canvas(
                    move |bounds, window, _| {
                        let run = TextRun {
                            len: text.len(),
                            font: window.text_style().font(),
                            color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(text.clone(), pt(TEXT_PT), &[run], None);
                        let (origin, height) = centred_line_origin(bounds, &line, pt(TEXT_PT), window);
                        (line, origin, height, bounds.size.width, bounds.origin.y, bounds.size.height)
                    },
                    move |_bounds, (line, origin, height, width, by, bh), window, _cx| {
                        // The slot's 24 pt cell is a frame the content sits in the middle of, so a
                        // short lead (`X`, `W`, `R`) is centred in it rather than pinned 8 pt in.
                        let x = origin.x + (width - line.width) * 0.5;
                        let _ = line.paint(point(x, origin.y), height, window, _cx);
                        if dbg {
                            // ⛔ INVARIANT — DO NOT CHANGE (用户硬性要求 "全部按照字形来").
                            // The debug frame MUST hug the glyph's INK BOX (height from
                            // `typographic_bounds`, centred on the field centre), NEVER the line box
                            // (`ascent + descent`) that `centred_line_origin` returns for *painting*.
                            // The line-box frame is taller (carries ascent/descent leading) and its
                            // centre ≠ glyph centre, so it looked "bigger than the glyph". Captions
                            // (`caption_label`) use the exact same glyph basis, so the two red frames
                            // stay pixel-consistent. Do NOT "fix" this back to the line box — that
                            // was the original bug.
                            let field_center = by + bh * 0.5;
                            let gh = line
                                .runs
                                .first()
                                .and_then(|run| window.text_system().typographic_bounds(run.font_id, pt(TEXT_PT), '0').ok())
                                .map(|g| g.size.height)
                                .unwrap_or(height);
                            let gy = field_center - gh * 0.5;
                            let mut boxq = gpui::fill(Bounds { origin: point(x, gy), size: size(line.width, gh) }, gpui::transparent_black());
                            boxq.border_widths = Edges::all(px(1.0));
                            boxq.border_color = gpui::rgb(0xFF2D55).into();
                            window.paint_quad(boxq);
                        }
                    },
                )
                .size_full()
                .into_any_element(),
            )
        }
        Slot::Icon(i) => Some(paint_icon(*i, rgba(t.icon), ICON).into_any_element()),
        Slot::Dot(c) => Some(div().size(pt(14.0)).rounded(pt(3.0)).bg(rgba(*c)).border_1().border_color(rgba(t.border)).into_any_element()),
        Slot::Custom(f) => Some(f()),
    }
}
