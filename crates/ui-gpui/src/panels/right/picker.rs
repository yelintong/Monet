//! 3×3 对齐选择器 —— 逐行照搬 `ui-egui/src/panels/layout_panel.rs::picker`。
//!
//! egui 用 `ui.painter()` 画：一个 `t.field` 底的圆角药丸，里面 3×3 个格子，每个格子要么画
//! 一组 2 pt 的横/竖条（当前对齐态），要么画一个 1 pt 灰点（未选中）。点击格子写
//! `node.set {"props": {"mainAlign": …, "crossAlign": …}}`。
//!
//! GPUI 侧用 `canvas` 元素做同样的绘制（`window.paint_quad`），格子的点击用 9 个绝对定位的
//! `div` 命中区——两者叠在同一个 `relative` 容器里。栅格模式（`LayoutMode::Grid`）画网格预览。

use crate::canvas::geom::Color32;
use crate::panels::{Cmd, LayoutDir, LayoutVals, RightFields};
use crate::widgets::{FIELD_H, GRID_GAP, pt, rgba, tokens};
use gpui::{App, Bounds, Corners, MouseButton, Pixels, Window, div, fill, point, prelude::*, px, size};
use serde_json::{Value, json};

// ── 源码常量（`layout_panel.rs`），一个不改 ──────────────────────────────────────────────
/// 字形盒 10 pt、条厚 2 pt、换行流里一项长 4 pt。
const GLYPH_BOX: f32 = 10.0;
const THICK: f32 = 2.0;
const ITEM: f32 = 4.0;
/// 不换行时三条的长度（按主轴对齐：起始 / 居中 / 末尾）。
const NOWRAP_LEN: [[f32; 3]; 3] = [[7.0, 10.0, 5.0], [5.0, 10.0, 5.0], [5.0, 10.0, 7.0]];
/// 自动间距（主轴分布）的格子索引。
const SPREAD: usize = 9;
/// 空槽（零宽或零高即不画），让字形固定是 5 条数组。
type Bar = (f32, f32, f32, f32);
const HOLE: Bar = (0.0, 0.0, 0.0, 0.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlowShape {
    NowrapV,
    NowrapH,
    WrapV,
    WrapH,
}

impl FlowShape {
    fn of(dir: LayoutDir, wrap: bool) -> Self {
        match (dir == LayoutDir::Vertical, wrap) {
            (true, false) => Self::NowrapV,
            (true, true) => Self::WrapV,
            (false, false) => Self::NowrapH,
            (false, true) => Self::WrapH,
        }
    }
    fn vertical(self) -> bool {
        matches!(self, Self::NowrapV | Self::WrapV)
    }
}

/// 当前选择态：两轴各自选中的格子（0..3 或 [`SPREAD`]），以及交叉轴是否按分布画。
#[derive(Clone, Copy, Debug)]
struct State {
    main: usize,
    cross: usize,
    spread_cross: bool,
}

impl State {
    fn of(l: &LayoutVals) -> Self {
        let main = match l.main_align {
            Some("center") => 1,
            Some("max") => 2,
            Some("spaceBetween") | Some("spaceAround") | Some("spaceEvenly") => SPREAD,
            _ => 0,
        };
        let cross = match l.cross_align {
            Some("center") => 1,
            Some("max") => 2,
            _ => 0,
        };
        Self { main, cross, spread_cross: l.wrap && l.content_align == Some("spaceBetween") }
    }

    /// 点 (main, cross) 会选成的状态：自动间距保持主轴分布，只改交叉轴。
    fn pick(self, main: usize, cross: usize) -> Self {
        let main = if self.main == SPREAD { SPREAD } else { main };
        Self { main, cross, spread_cross: self.spread_cross }
    }

    /// 该格子画什么；`None` = 保持灰点。
    fn bars(self, shape: FlowShape, main: usize, cross: usize) -> Option<[Bar; 5]> {
        if !self.spread_cross && cross != self.cross {
            return None;
        }
        let cross = if self.spread_cross { 1 } else { self.cross };
        if self.main == SPREAD {
            let len = if main == 1 { GLYPH_BOX * 0.5 } else { GLYPH_BOX };
            return Some(spread_bar(shape, cross, len));
        }
        (main == self.main).then(|| packed_bars(shape, main, cross))
    }
}

/// 水平流主轴沿列、垂直流主轴沿行。
fn axes(xi: usize, yi: usize, horizontal: bool) -> (usize, usize) {
    if horizontal { (xi, yi) } else { (yi, xi) }
}

/// 打包流的字形（10 pt 盒内）。
fn packed_bars(shape: FlowShape, main: usize, cross: usize) -> [Bar; 5] {
    let c = cross as f32;
    let mut bars = [HOLE; 5];
    match shape {
        FlowShape::NowrapV => {
            for (i, len) in NOWRAP_LEN[main].into_iter().enumerate() {
                bars[i] = ((GLYPH_BOX - len) * 0.5 * c, i as f32 * ITEM, len, THICK);
            }
        }
        FlowShape::NowrapH => {
            for (i, len) in NOWRAP_LEN[main].into_iter().enumerate() {
                bars[i] = (i as f32 * ITEM, (GLYPH_BOX - len) * 0.5 * c, THICK, len);
            }
        }
        FlowShape::WrapV => {
            bars[0] = (0.0, 0.0, ITEM, THICK);
            bars[1] = (6.0, 0.0, ITEM, THICK);
            bars[2] = (0.0, 4.0, ITEM, THICK);
            bars[3] = (6.0, 4.0, ITEM, THICK);
            bars[4] = (c * 3.0, 8.0, ITEM, THICK);
        }
        FlowShape::WrapH => {
            bars[0] = (0.0, 0.0, THICK, ITEM);
            bars[1] = (4.0, 0.0, THICK, ITEM);
            bars[2] = (8.0, 0.0, THICK, ITEM);
            bars[3] = (main as f32 * 2.0, 6.0, THICK, ITEM);
            bars[4] = (main as f32 * 2.0 + 4.0, 6.0, THICK, ITEM);
        }
    }
    bars
}

/// 自动间距的标记：一条横贯格子的条，长度 `len`，沿对齐轴滑动。
fn spread_bar(shape: FlowShape, cross: usize, len: f32) -> [Bar; 5] {
    let mut bars = [HOLE; 5];
    let off = (GLYPH_BOX - len) * 0.5 * cross as f32;
    let mid = (GLYPH_BOX - THICK) * 0.5;
    bars[0] = if shape.vertical() { (off, mid, len, THICK) } else { (mid, off, THICK, len) };
    bars
}

/// 画一个填充矩形（`window.paint_quad`），圆角 `r`。
fn rect(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: impl Into<gpui::Rgba>, r: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let corners = Corners { top_left: px(r), top_right: px(r), bottom_right: px(r), bottom_left: px(r) };
    let q = fill(Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }, color.into()).corner_radii(corners);
    window.paint_quad(q);
}

/// 3×3 对齐选择器。宽度**填满所在列**（`w_full`，列宽由外部 `flex_1` 决定）；高度
/// `FIELD_H * 2 + GUTTER`（跨两行）。
pub(crate) fn alignment_picker(f: &RightFields, dir: LayoutDir, l: &LayoutVals) -> gpui::Div {
    let t = tokens();
    let height = FIELD_H * 2.0 + GRID_GAP;
    let shape = FlowShape::of(dir, l.wrap);
    let state = State::of(l);
    let horizontal = dir != LayoutDir::Vertical;
    let is_grid = dir == LayoutDir::Grid;

    // 绘制层：药丸底 + 3×3 条/点（或栅格预览）。
    let accent = t.accent;
    let field = t.field;
    let grey = t.text_secondary;
    let grid_cols: f32 = 2.0; // 栅格预览：列数由 `dims` 的 gap 形状粗略代表
    let canvas = gpui::canvas(
        move |_bounds, _window, _cx| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _cx| {
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (w, h) = (f32::from(bounds.size.width).max(1.0), f32::from(bounds.size.height).max(1.0));
            rect(window, x0, y0, w, h, rgba(field), 5.0);
            if is_grid {
                // 栅格预览：2 列 × 2 行的白格 + 居中「rows × cols」略（文字用 GPUI 元素另画）。
                let inner = 3.0;
                let (ix, iy, iw, ih) = (x0 + inner, y0 + inner, w - inner * 2.0, h - inner * 2.0);
                let gap = 2.0;
                let cols = grid_cols.max(1.0);
                let cw = (iw - (cols - 1.0) * gap) / cols;
                let ch = (ih - gap) / 2.0;
                for r in 0..2 {
                    for c in 0..(cols as usize) {
                        rect(window, ix + c as f32 * (cw + gap), iy + r as f32 * (ch + gap), cw, ch, rgba(Color32::WHITE), 2.0);
                    }
                }
                return;
            }
            let inset = h / 12.0;
            let (gx, gy, gw, gh) = (x0, y0 + inset, w, h - inset * 2.0);
            let (cw, ch) = (gw / 3.0, gh / 3.0);
            for yi in 0..3 {
                for xi in 0..3 {
                    let (main_i, cross_i) = axes(xi, yi, horizontal);
                    let cx = gx + (xi as f32 + 0.5) * cw;
                    let cy = gy + (yi as f32 + 0.5) * ch;
                    if let Some(bars) = state.bars(shape, main_i, cross_i) {
                        let bx = cx - GLYPH_BOX * 0.5;
                        let by = cy - GLYPH_BOX * 0.5;
                        for (bx_, by_, bw, bh) in bars {
                            rect(window, bx + bx_, by + by_, bw, bh, rgba(accent), 1.0);
                        }
                    } else {
                        // 1 pt 灰点（半径 1 → 2×2）。
                        rect(window, cx - 1.0, cy - 1.0, 2.0, 2.0, rgba(grey), 1.0);
                    }
                }
            }
        },
    )
    .absolute()
    .size_full();

    // 9 个命中格：3×3 弹性网格铺满药丸（与画布同样上下 inset 1/12），点击写 mainAlign/crossAlign。
    let inset = height / 12.0;
    let mut grid = div().absolute().size_full().pt(px(inset)).pb(px(inset)).flex().flex_col();
    let main_now = l.main_align;
    let is_spread = state.main == SPREAD;
    for yi in 0..3 {
        let mut row = div().flex().flex_row().flex_1().min_h(px(0.0));
        for xi in 0..3 {
            let cmds = f.cmds.clone();
            let (main_i, cross_i) = axes(xi, yi, horizontal);
            row = row.child(div().flex_1().min_w(px(0.0)).h_full().cursor_pointer().on_mouse_down(MouseButton::Left, move |_e, _w, _cx| {
                let main: Value = if is_spread { json!(main_now.unwrap_or("min")) } else { json!(["min", "center", "max"][main_i]) };
                let cross = ["min", "center", "max"][cross_i];
                cmds.push(Cmd { id: "node.set", params: Some(json!({ "props": { "mainAlign": main, "crossAlign": cross } })) });
            }));
        }
        grid = grid.child(row);
    }
    div().relative().w_full().h(pt(height)).child(canvas).child(grid)
}

/// 让 `RightFields` 的私有 `cmds` 在 picker 里可用（同模块树，直接字段访问即可）。
const _: fn() = || {};
#[allow(dead_code)]
fn _assert_cmds(_: &RightFields) {
    let _ = &|f: &RightFields| f.cmds.clone();
}

/// 占位：确保 `App` 被引用（`on_mouse_down` 的闭包签名用到）。
#[allow(dead_code)]
fn _app(_: &mut App) {}
