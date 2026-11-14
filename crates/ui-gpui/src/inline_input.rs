//! 无边框「纯输入框」——重命名等就地编辑的唯一实现。
//!
//! 三处界面共用它（egui 版对应 `widgets::rename_field` / `text_field_inline`）：
//!   * 左栏 Pages 行（双击页面名），
//!   * 左栏 Layers 行（双击图层名），
//!   * 画布上顶层帧 / section 的名字标签（双击标签）。
//!
//! 它**只组合** `widgets::TextEdit`（冻结的编辑引擎：caret / 选区 / IME / Enter / 失焦提交），
//! 自己负责三件以前散落在各调用点、因而互不一致的事：
//!
//! 1. **没有边框、没有底色、没有圆角**的裸外观；高度由调用方给出，盒高固定。
//! 2. **统一的焦点生命周期**：打开后持续申请焦点直到真正拿到；只有拿过焦点之后再失焦，
//!    才算编辑结束（否则刚打开就被关掉）。
//! 3. **稳定行盒**：`TextInput` 用「当前文字的 shaped line」的 `ascent + descent` 做垂直居中，
//!    这个度量会随文字内容（空串 / 中文回退字体 / IME 组字串）变化，于是打字时基线会
//!    上下跳一两像素。这里按当帧实际显示串量出度量，给编辑器一个补偿位移，让基线恒定，
//!    与「有边框的输入框」（固定 `FIELD_H` 盒 + 同一字体）对齐。
//!
//! 该文件刻意放在 `src/` 而不是 `src/widgets/`：后者是冻结件，不得改动；本组件只调用它们。

use crate::widgets::TextEdit;
use gpui::{App, Context, Entity, IntoElement, MouseButton, Render, SharedString, Window, div, prelude::*, px};

/// 11pt 与三处被替换的静态标签一致（egui `regular(11.0)` / GPUI `text_size(px(11.0))`）。
const FONT: f32 = 11.0;
/// 参考串：用它量出「基准行盒」，与具体输入内容无关。
const REFERENCE: &str = "Mg";

/// Pages / Layers 行高 32，egui 的重命名框是行内缩 4pt → 24pt。
pub const ROW_H: f32 = 24.0;
/// 画布标签：egui `frame_labels::rename_field` 的 `vec2(w, 16.0)`。
pub const LABEL_H: f32 = 16.0;

/// 一次就地编辑的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditState {
    /// 还在编辑（含等待焦点落地的开头几帧）。
    Editing,
    /// 拿到过焦点后又失焦：调用方应关掉这次重命名。
    Ended,
}

/// 无边框内联输入框。视觉上没有 shell，只有固定高度的一层盒 + 裸 `TextEdit`。
pub struct InlineInput {
    edit: Entity<TextEdit>,
    /// 固定盒高（pt）。Pages / Layers 行用 24，画布标签按 egui 用 16。
    height: f32,
    /// 焦点曾经落到这个 editor 上。
    was_active: bool,
}

impl InlineInput {
    /// 每次重命名都新建一个：复用同一个 `TextEdit` 会留下上一次的草稿
    /// （`set_value` 在编辑态不会动 buffer），导致「第二次双击改成了第一个名字」。
    pub fn build(
        cx: &mut App,
        height: f32,
        value: impl Into<String>,
        on_commit: impl Fn(&str, &mut Window, &mut App) + 'static,
    ) -> Entity<Self> {
        let value = value.into();
        cx.new(|cx| {
            let edit = TextEdit::build(cx, |te, _| {
                te.set_inset_left(0.0);
                te.set_right_pad(0.0);
                te.set_font_size(px(FONT));
                // 空缓冲区默认显示空串，空串的 shaped line 度量与有字时不同；
                // 一个不可见的空格让「删光了」也保持同样的行盒。
                te.set_placeholder(" ");
                te.set_value(value);
                te.set_on_commit(on_commit);
            });
            Self { edit, height, was_active: false }
        })
    }

    /// 点击别处 / 提交草稿：与 `widgets::TextField` 的失焦提交同源。
    pub fn finish_edit(&self, window: &mut Window, cx: &mut App) {
        self.edit.update(cx, |te, cx| {
            te.finish_edit(window, cx);
        });
    }

    /// 每帧在渲染前调用：申请焦点、判断是否已经结束。
    pub fn poll_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> EditState {
        // 只在「还没激活过」时抢焦点：一旦激活过，失焦就代表用户点了别处。未改动的重命名
        // 在失焦时 `TextEdit::commit` 不产生回调，这里若把焦点再抢回来，就会把重命名重新
        // 打开、把后续输入全部截走（点另一个输入框即复现）。
        if !self.was_active && !self.edit.read(cx).is_active(window) {
            self.edit.update(cx, |te, _| te.focus_editor(window));
        }
        let active = self.edit.read(cx).is_active(window);
        if active {
            self.was_active = true;
        }
        if !active && self.was_active { EditState::Ended } else { EditState::Editing }
    }

    /// `TextInput::display` 的等价串（本组件不用 secret / suffix / placeholder 文本）。
    fn shown_text(&self, cx: &App) -> String {
        let te = self.edit.read(cx);
        let c = te.input.buffer.composed();
        if c.composing || !te.input.buffer.is_empty() {
            c.text.replace(['\n', '\r'], " ")
        } else {
            " ".to_string()
        }
    }
}

/// 行盒补偿量（pt）：把「随内容变化的基线」拉回参考基线。
///
/// `TextInput` 绘制时 `centred_line_origin` 给出
/// `origin.y = box.y + (H - (A + D)) / 2`，`paint_line` 的基线是 `origin.y + A`
/// （`line_height == A + D`，padding 为 0）。要把基线锁在参考值
/// `box.y + (H - (A0 + D0)) / 2 + A0`，只需把整个编辑器盒子下移
/// `((A + D) - (A0 + D0)) / 2 + (A0 - A)`。
pub fn stable_top_offset(ascent: f32, descent: f32, ref_ascent: f32, ref_descent: f32) -> f32 {
    ((ascent + descent) - (ref_ascent + ref_descent)) * 0.5 + (ref_ascent - ascent)
}

/// 画布标签重命名框的矩形（egui `frame_labels::rename_field`），纯几何、可回归测试。
///
/// `min_x / max_x / max_y` 是标签四角经 `to_screen` 得到的**画布局部坐标**；本项目
/// `app.canvas.rect` 原点恒为 0，绝对定位的浮层挂在整窗根视图上，因此必须补 `origin_*`。
/// 调用方给的 `text_w` 是 11pt 下真实量算出的名字宽度（不再是字符数估算）。
pub fn label_rename_rect(min_x: f32, max_x: f32, max_y: f32, text_w: f32, origin_x: f32, origin_y: f32) -> (f32, f32, f32) {
    let w = (text_w + 24.0).max(max_x - min_x).clamp(80.0, 480.0);
    (min_x + origin_x, max_y - 16.0 + origin_y, w)
}

/// 标签 quad 缺失时的兜底位置（帧左上角上方 20pt），同样要补画布窗口原点。
pub fn label_rename_fallback(x: f32, y: f32, text_w: f32, origin_x: f32, origin_y: f32) -> (f32, f32, f32) {
    let w = (text_w + 24.0).clamp(80.0, 480.0);
    (x + origin_x, y - 20.0 + origin_y, w)
}

impl Render for InlineInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown_text(cx);
        let line = window.text_system().shape_line(SharedString::from(shown.clone()), px(FONT), &[window.text_style().to_run(shown.len())], None);
        let reference =
            window.text_system().shape_line(SharedString::from(REFERENCE), px(FONT), &[window.text_style().to_run(REFERENCE.len())], None);
        let drift = stable_top_offset(
            f32::from(line.ascent),
            f32::from(line.descent),
            f32::from(reference.ascent),
            f32::from(reference.descent),
        );
        div()
            .relative()
            .w_full()
            .h(px(self.height))
            // 点输入框自己不能冒泡到根视图的「点别处就提交」。
            .on_mouse_down(MouseButton::Left, |_e, _w, cx| cx.stop_propagation())
            .child(div().relative().w_full().h_full().top(px(drift)).child(self.edit.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_metrics_need_no_correction() {
        assert_eq!(stable_top_offset(9.0, 3.0, 9.0, 3.0), 0.0);
    }

    /// 关键回归：字体度量变了，补偿后的基线必须逐像素不动。
    /// baseline = box.y + (H - (A+D))/2 + A + drift，H 取 24。
    #[test]
    fn corrected_baseline_is_constant_across_font_metrics() {
        let h = 24.0;
        let (a0, d0) = (9.0, 3.0);
        let expected = (h - (a0 + d0)) * 0.5 + a0;
        for (a, d) in [(11.0, 3.0), (8.0, 2.0), (10.0, 4.0), (0.0, 0.0), (13.0, 5.0)] {
            let drift = stable_top_offset(a, d, a0, d0);
            let baseline = (h - (a + d)) * 0.5 + a + drift;
            assert!((baseline - expected).abs() < 1e-4, "metrics ({a}, {d}) moved the baseline by {}", baseline - expected);
        }
    }

    /// 空串 / 占位空格走同一套度量：`shown_text` 用空格代替空缓冲区。
    #[test]
    fn reference_is_a_single_line_without_newlines() {
        assert!(!REFERENCE.contains('\n') && !REFERENCE.is_empty());
    }

    /// 画布标签输入框位置：必须补上 dock 给的画布窗口原点（原来漏了，输入框整体偏到左栏）。
    #[test]
    fn label_rename_rect_adds_the_canvas_window_origin() {
        let (x, y, w) = label_rename_rect(100.0, 300.0, 240.0, 60.0, 300.0, 40.0);
        assert_eq!((x, y, w), (400.0, 264.0, 200.0));
        // 同一标签在原点为 0 的画布里，x/y 正好少掉轨道 + 左栏 + 标题栏。
        assert_eq!(label_rename_rect(100.0, 300.0, 240.0, 60.0, 0.0, 0.0), (100.0, 224.0, 200.0));
    }

    /// 宽度逐项照抄 egui：`(text_w + 24).max(area.width).clamp(80, 480)`。
    #[test]
    fn label_rename_rect_width_uses_measured_text_and_label_bounds() {
        assert_eq!(label_rename_rect(0.0, 10.0, 30.0, 2.0, 0.0, 0.0).2, 80.0, "短名字至少 80pt");
        assert_eq!(label_rename_rect(0.0, 400.0, 30.0, 2.0, 0.0, 0.0).2, 400.0, "标签比文字宽时取标签宽");
        assert_eq!(label_rename_rect(0.0, 10.0, 30.0, 900.0, 0.0, 0.0).2, 480.0, "长名字夹到 480pt");
        assert_eq!(label_rename_rect(0.0, 10.0, 30.0, 100.0, 0.0, 0.0).2, 124.0);
    }

    #[test]
    fn label_rename_fallback_also_adds_the_origin() {
        assert_eq!(label_rename_fallback(50.0, 200.0, 60.0, 300.0, 40.0), (350.0, 220.0, 84.0));
    }
}
