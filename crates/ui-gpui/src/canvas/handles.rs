//! 自动布局的 padding / gap 手柄：**照搬** `ui-egui/src/canvas/handles.rs` 里的 `al_zones`、
//! `Drag::Padding`、`Drag::Gap` 与它们的 `draw` / `input`。
//!
//! 行为与原实现一致：选中一个自动布局 frame 后，指针停到某条边距带 / 间距带上时它高亮并显示数值，
//! 按住拖动改 `paddingTop/Right/Bottom/Left`（Shift 同轴两侧一起）或 `gap`/`crossGap`。
//!
//! 高亮不再是叠加层里画色块：带子（底色 + 斜线）作为 `figcraft_render::Band` 交给渲染，
//! 铺在子层**之前**（见 `crates/render/src/band.rs`），这样边界只有一次合成、不会留白细线。
//! 这里只剩数值标签、间距刻度和拖动逻辑。
//!
//! 还没搬：`handles.rs` 里的渐变 / 模糊 / 裁剪手柄（要面板的颜色选择器 `colorpicker::PickerTarget`
//! 与 Dev Mode 的 `RightTab`），等面板搬过来再接。

use figcraft_doc::{ContentAlign, LayoutMode, NodeId};
use figcraft_engine::ToolId;
use figcraft_geom::Point;
use figcraft_i18n::tr;
use serde_json::json;

use super::geom::*;
use super::outline::stroke;
use super::painter::Painter;
use super::rulers::fmt;
use crate::app::CanvasApp;
use crate::canvas::cursors::Shown;

/// Figma 的 spacing 粉（数值标签底色）。
pub(crate) const PINK: Color32 = Color32::from_rgb(0xF2, 0x48, 0x8C);

/// padding 带（自动布局内边距）的蓝：深色主题用 Figma 快照里的深色令牌。
fn pad_colour(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(0x0c, 0x8c, 0xe9) } else { Color32::from_rgb(0x0d, 0x99, 0xff) }
}

/// spacing 带（子项间距）的粉。
fn gap_colour(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(0xf3, 0x16, 0xb0) } else { Color32::from_rgb(0xff, 0x24, 0xbd) }
}

/// 带子的 RGBA（未预乘）：底色 α=0.15、斜线 α=0.60（Figma 快照 CSS 的 `.xljh79v`）。
fn rgba(c: Color32, a: f32) -> [u8; 4] {
    [c.0[0], c.0[1], c.0[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8]
}

/// 正在拖的那条手柄。
#[derive(Clone)]
pub enum Drag {
    /// Padding side 0 top, 1 right, 2 bottom, 3 left.
    Padding { id: NodeId, side: usize, start: Pos2, value: f64 },
    /// A gap band: `key` is the prop it edits (`gap` or `crossGap`), `vertical` the drag axis.
    Gap { id: NodeId, start: Pos2, value: f64, vertical: bool, key: &'static str },
}

/// 当前粘性悬停的那条带子（padding 用 side、gap 用它在 `gaps` 里的下标定位）。
///
/// 用「粘性」而不是每帧按指针位置重判：一旦指针落上某条带子，只要它还在这条带子里，
/// 就一直高亮这一条；只有指针离开它之后才重新判定下一条。否则移到相邻/角落时带子会来回跳。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Hover {
    Padding { id: NodeId, side: usize },
    Gap { id: NodeId, index: usize },
}

#[derive(Clone, Default)]
pub struct State {
    pub drag: Option<Drag>,
    /// 这一次拖动已经把数值改过了（没过阈值、只是按了一下不算）。
    pub moved: bool,
    /// 粘性悬停：见 [`Hover`]。
    pub hover: Option<Hover>,
}

impl State {
    /// 拖动 padding / gap 手柄时，画布要**当帧**自己画哪一层、以及是不是每帧重抓。
    ///
    /// 和 `Gesture::Resize` 那条 live 路径同形：返回自动布局 frame 本身（它也是 band 的
    /// owner），`true` = 每帧重抓（子层跟着重排，像素每帧都在变）。
    /// 不进 live 的话，每个指针事件只能等一次整页异步渲染——背景落后于带子，
    /// 反复拖动还会一张接一张地分配整页缓冲（内存）。
    pub fn live_layers(&self) -> Option<(Vec<NodeId>, bool)> {
        let drag = self.drag.as_ref()?;
        if !self.moved {
            return None;
        }
        let id = match drag {
            Drag::Padding { id, .. } | Drag::Gap { id, .. } => *id,
        };
        Some((vec![id], true))
    }
}

/// 刷新「悬停哪条带子」：粘性判定（指针还在上一条里面就锁定，离开才重判）。
///
/// 必须在 `bands_for_render` / `draw` 之前调用（每帧一次，`&mut CanvasApp`）。
pub(crate) fn update_hover(app: &mut CanvasApp) {
    // 拖动中由被拖的那条接管，悬停态清空。
    if app.handles.drag.is_some() {
        app.handles.hover = None;
        return;
    }
    let Some(z) = al_zones(app) else {
        app.handles.hover = None;
        return;
    };
    let Some(pointer) = app.pointer else {
        app.handles.hover = None;
        return;
    };
    // 还在上一条里面 → 不动。
    let still_in = match &app.handles.hover {
        Some(Hover::Padding { id, side }) if *id == z.id => z.bands.iter().find(|(s, _, _)| *s == *side).is_some_and(|(_, r, _)| r.contains(pointer)),
        Some(Hover::Gap { id, index }) if *id == z.id => z.gaps.get(*index).is_some_and(|g| g.rect.contains(pointer)),
        _ => false,
    };
    if still_in {
        return;
    }
    // 离开上一条 → 重新判定（可空）。先 padding 后 gap，与 `input` 的命中顺序一致。
    app.handles.hover = None;
    for (side, r, _) in &z.bands {
        if r.contains(pointer) {
            app.handles.hover = Some(Hover::Padding { id: z.id, side: *side });
            return;
        }
    }
    for (i, g) in z.gaps.iter().enumerate() {
        if g.rect.contains(pointer) {
            app.handles.hover = Some(Hover::Gap { id: z.id, index: i });
            return;
        }
    }
}

#[derive(Clone)]
struct GapZone {
    rect: Rect,
    key: &'static str,
    value: f64,
    vertical: bool,
}

struct Zones {
    id: NodeId,
    bands: Vec<(usize, Rect, f64)>,
    gaps: Vec<GapZone>,
    /// 网格 frame：要画虚线的轨道边界（屏幕坐标线段）。
    tracks: Vec<[Pos2; 2]>,
}

impl CanvasApp {
    /// 画布 → 页面：手柄几何都在屏幕坐标里算，数值标签上的原点锚也一样。
    fn screen(&self, x: f64, y: f64) -> Pos2 {
        self.to_screen(x, y)
    }
}

/// 选中层若是一个自动布局 frame，返回它的边距带 / 间距带 / 轨道线（屏幕坐标）。
fn al_zones(app: &CanvasApp) -> Option<Zones> {
    let ds = app.session.ds().ok()?;
    if ds.selection.len() != 1 || ds.text_edit.is_some() || ds.vector_edit.is_some() {
        return None;
    }
    let id = *ds.selection.first()?;
    let doc = &ds.doc;
    let n = doc.node(id)?;
    let al = n.layout.as_ref()?;
    // Only for unrotated frames (axis-aligned bands).
    let w = doc.world_transform(id);
    if figcraft_geom::rotation_degrees(w).abs() > 0.01 {
        return None;
    }
    let to = |x: f64, y: f64| {
        let p = w * Point::new(x, y);
        app.screen(p.x, p.y)
    };
    let band = |x0: f64, y0: f64, x1: f64, y1: f64| Rect::from_two_pos(to(x0, y0), to(x1, y1));
    let [pt, pr, pb, pl] = al.padding;
    let (fw, fh) = (n.width, n.height);
    let bands = vec![
        (0, band(0.0, 0.0, fw, pt.max(4.0)), pt),
        (1, band(fw - pr.max(4.0), 0.0, fw, fh), pr),
        (2, band(0.0, fh - pb.max(4.0), fw, fh), pb),
        (3, band(0.0, 0.0, pl.max(4.0), fh), pl),
    ];
    // An auto gap shows (and is dragged from) the distance the layout gives it; the drag makes it a
    // fixed gap again, as typing one does.
    let (auto_main, auto_cross) =
        (al.main_align.is_auto_gap() && al.mode != LayoutMode::Grid, al.wrap && al.content_align == ContentAlign::SpaceBetween);
    let main = |rect: Rect, vertical: bool, dist: f64| GapZone { rect, key: "gap", value: if auto_main { dist } else { al.gap }, vertical };
    let cross =
        |rect: Rect, vertical: bool, dist: f64| GapZone { rect, key: "crossGap", value: if auto_cross { dist } else { al.cross_gap }, vertical };
    let mut gaps = Vec::new();
    let mut tracks = Vec::new();
    if let Some(g) = figcraft_layout::grid_geometry(doc, id) {
        // Grid: bands between columns (column gap) and between rows (row gap), and the tracks.
        let (top, bottom) = (g.rows.first().map(|r| r.0).unwrap_or(pt), g.rows.last().map(|r| r.1).unwrap_or(fh - pb));
        let (left, right) = (g.columns.first().map(|c| c.0).unwrap_or(pl), g.columns.last().map(|c| c.1).unwrap_or(fw - pr));
        for pair in g.columns.windows(2) {
            if let [a, b] = pair {
                gaps.push(main(band(a.1, top, b.0, bottom), false, al.gap));
            }
        }
        for pair in g.rows.windows(2) {
            if let [a, b] = pair {
                gaps.push(cross(band(left, a.1, right, b.0), true, al.cross_gap));
            }
        }
        for c in &g.columns {
            tracks.push([to(c.0, top), to(c.0, bottom)]);
            tracks.push([to(c.1, top), to(c.1, bottom)]);
        }
        for r in &g.rows {
            tracks.push([to(left, r.0), to(right, r.0)]);
            tracks.push([to(left, r.1), to(right, r.1)]);
        }
    } else {
        // Worked out along x: a vertical flow is a horizontal one turned on its side (`flip`
        // swaps the axes, `fband` swaps them back).
        let vertical = al.mode == LayoutMode::Vertical;
        let flip = |r: figcraft_geom::Rect| if vertical { figcraft_geom::Rect::new(r.y0, r.x0, r.y1, r.x1) } else { r };
        let fband = |x0: f64, y0: f64, x1: f64, y1: f64| if vertical { band(y0, x0, y1, x1) } else { band(x0, y0, x1, y1) };
        // Main-axis length and padding, cross-axis length and padding.
        let (mw, mp0, mp1, cw, cp0, cp1) = if vertical { (fh, pt, pb, fw, pl, pr) } else { (fw, pl, pr, fh, pt, pb) };
        let boxes: Vec<figcraft_geom::Rect> = figcraft_layout::flow_children(doc, id)
            .into_iter()
            .filter_map(|c| doc.node(c))
            .map(|c| flip(figcraft_geom::transform_rect_bbox(c.transform, c.local_rect())))
            .collect();
        // Lines: a wrapped item starts before the end of the previous one.
        let mut lines: Vec<Vec<figcraft_geom::Rect>> = Vec::new();
        for b in boxes {
            match lines.last_mut() {
                Some(line) if !al.wrap || line.last().is_some_and(|a| b.x0 >= a.x1 - 1e-6) => line.push(b),
                _ => lines.push(vec![b]),
            }
        }
        let span = |line: &[figcraft_geom::Rect]| line.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| (a.min(r.y0), b.max(r.y1)));
        for line in &lines {
            let (y0, y1) = if al.wrap { span(line) } else { (cp0, cw - cp1) };
            for pair in line.windows(2) {
                if let [ra, rb] = pair {
                    gaps.push(main(fband(ra.x1, y0, rb.x0, y1), vertical, rb.x0 - ra.x1));
                }
            }
        }
        for pair in lines.windows(2) {
            if let [a, b] = pair {
                gaps.push(cross(fband(mp0, span(a).1, mw - mp1, span(b).0), !vertical, span(b).0 - span(a).1));
            }
        }
    }
    gaps.retain(|g| g.rect.width() > 0.5 && g.rect.height() > 0.5);
    Some(Zones { id, bands, gaps, tracks })
}

/// 给颜色叠加 α（0..1）。
fn with_alpha(c: Color32, a: f32) -> Color32 {
    let a8 = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], a8)
}

/// 斜线的锚点：节点世界原点（图案随节点走，不随平移滑）。
fn origin_of(app: &CanvasApp, id: NodeId) -> Pos2 {
    let Ok(ds) = app.session.ds() else { return Pos2::ZERO };
    let p = ds.doc.world_transform(id) * Point::new(0.0, 0.0);
    app.to_screen(p.x, p.y)
}

/// 数值标签：粉底白字（原实现的 `pink_label`）。
pub(crate) fn pink_label(painter: &mut Painter, label: String, center: Pos2) {
    let font = semibold(10.0);
    let size = super::painter::text_size(&label, font);
    let rect = Rect::from_center_size(center, size + vec2(8.0, 4.0));
    painter.rect_filled(rect, CornerRadius::same(3), PINK);
    painter.text(rect.min + vec2(4.0, 2.0), Align2::LEFT_TOP, label, font, Color32::WHITE);
}

/// 这一帧交给渲染的提示带（**页面坐标**）：指针所在的那一条，或者正在拖的那一条。
///
/// 为什么交给渲染而不是画在叠加层：带子的边缘和子层的边缘是同一条线。画在贴图之上时，
/// 子层在贴图里的抗锯齿边缘已经把画框底色混进边界那一列，带子只盖到一半 —— 看着一条白细线。
/// 铺在子层之前，那一列只有一次合成（见 `crates/render/src/band.rs`）。
pub fn bands_for_render(app: &CanvasApp) -> Vec<figcraft_render::Band> {
    let Some(z) = al_zones(app) else { return Vec::new() };
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let zoom = ds.view.zoom.max(1e-6);
    let dark = app.theme.dark;
    // 屏幕上的 6px 周期、2px 线宽 → 页面单位（图案随视图缩放保持屏幕上的疏密）。
    let period = 6.0 * std::f64::consts::SQRT_2 / zoom;
    let width = 2.0 / zoom;
    let anchor = app.to_page(origin_of(app, z.id));
    let origin = [anchor.x, anchor.y];
    let mut out = Vec::new();
    let push = |out: &mut Vec<figcraft_render::Band>, r: Rect, col: Color32, m: f32| {
        let a = app.to_page(r.min);
        let b = app.to_page(r.max);
        out.push(figcraft_render::Band {
            owner: z.id,
            rect: [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)],
            fill: rgba(col, 0.15),
            stripe: Some(figcraft_render::Stripe { m, color: rgba(col, 0.60), period, width, origin }),
        });
    };
    // 正在拖的那条：跟手显示，优先于悬停。
    if let Some(Drag::Padding { side, .. }) = app.handles.drag.as_ref()
        && let Some((_, r, _)) = z.bands.iter().find(|(s, _, _)| *s == *side)
    {
        push(&mut out, *r, pad_colour(dark), if *side % 2 == 0 { 1.0 } else { -1.0 });
    }
    if let Some(Drag::Gap { key, .. }) = app.handles.drag.as_ref() {
        for g in &z.gaps {
            if g.key == *key {
                push(&mut out, g.rect, gap_colour(dark), 1.0);
            }
        }
    }
    // 没在拖 → 显示粘性悬停的那一条（指针离开上一条后才重判，不会在带子间乱跳）。
    if app.handles.drag.is_none() {
        match &app.handles.hover {
            Some(Hover::Padding { id, side }) if *id == z.id => {
                if let Some((_, r, _)) = z.bands.iter().find(|(s, _, _)| *s == *side) {
                    push(&mut out, *r, pad_colour(dark), if *side % 2 == 0 { 1.0 } else { -1.0 });
                }
            }
            Some(Hover::Gap { id, index }) if *id == z.id => {
                if let Some(g) = z.gaps.get(*index) {
                    push(&mut out, g.rect, gap_colour(dark), 1.0);
                }
            }
            _ => {}
        }
    }
    out
}

/// 画手柄（原 `handles::draw` 的自动布局部分）。
pub fn draw(app: &CanvasApp, painter: &mut Painter, _pointer: Option<Pos2>) {
    let tool_move = app.session.tool.tool == ToolId::Move && !app.session.tool.busy();
    let dragging = app.handles.drag.as_ref();
    if !tool_move && dragging.is_none() {
        return;
    }
    let Some(z) = al_zones(app) else { return };
    let active_side = match dragging {
        Some(Drag::Padding { side, .. }) => Some(*side),
        _ => None,
    };
    for seg in &z.tracks {
        // 轨道线：原实现是粉色虚线（网格 frame 的行列边界）。
        painter.line_segment(*seg, stroke(with_alpha(PINK, 0.6), 1.0));
    }
    for (side, r, v) in &z.bands {
        // 拖动中显示被拖的那条；否则只显示粘性悬停的那一条（用 `hover`，不再逐条 `contains` 指针）。
        let hot = active_side == Some(*side) || matches!(&app.handles.hover, Some(Hover::Padding { id, side: s }) if *id == z.id && *s == *side);
        if hot {
            // 底色与斜线由渲染层铺（在子层之前）；这里只放数值标签。
            pink_label(painter, fmt(*v), r.center());
        }
    }
    let drag_key = match dragging {
        Some(Drag::Gap { key, .. }) => Some(*key),
        _ => None,
    };
    for (i, g) in z.gaps.iter().enumerate() {
        let r = g.rect;
        let hot = drag_key == Some(g.key) || matches!(&app.handles.hover, Some(Hover::Gap { id, index }) if *id == z.id && *index == i);
        if hot {
            let c = r.center();
            painter.line_segment([pos2(c.x - 3.0, c.y), pos2(c.x + 3.0, c.y)], stroke(PINK, 1.0));
            pink_label(painter, fmt(g.value), c);
        }
    }
}

/// 指针处理；返回 true 表示这一下归手柄（画布不要再把它当选择/拖拽）。
///
/// `pressed` 是「这一事件按下了左键」，`released` 是「这一事件松开了左键」，
/// `down` 是「左键当前按着」——由 GPUI 的三个回调分别传进来（egui 那边是同一个 `input()`）。
pub fn input(app: &mut CanvasApp, pos: Pos2, pressed: bool, down: bool, released: bool, shift: bool) -> bool {
    if let Some(drag) = app.handles.drag.clone() {
        if down {
            apply(app, &drag, pos, shift);
            app.spacing_cursor = Some(Shown::System(gpui::CursorStyle::ClosedHand));
        }
        if released || !down {
            let _ = app.session.commit_interaction();
            app.handles.drag = None;
        }
        return true;
    }
    let tool_move = app.session.tool.tool == ToolId::Move && !app.session.tool.busy();
    if !tool_move {
        return false;
    }
    let Some(z) = al_zones(app) else { return false };
    for (side, r, v) in &z.bands {
        if !r.contains(pos) {
            continue;
        }
        app.spacing_cursor = Some(Shown::System(if *side % 2 == 0 { gpui::CursorStyle::ResizeUpDown } else { gpui::CursorStyle::ResizeLeftRight }));
        if pressed {
            let _ = app.session.begin_interaction(tr("Padding"));
            app.handles.moved = false;
            app.handles.drag = Some(Drag::Padding { id: z.id, side: *side, start: pos, value: *v });
            return true;
        }
        // 原实现：只是悬停时不接管指针（把 hover 留给画布），按下才算归手柄。
        return false;
    }
    for g in &z.gaps {
        if !g.rect.contains(pos) {
            continue;
        }
        app.spacing_cursor = Some(Shown::System(if g.vertical { gpui::CursorStyle::ResizeUpDown } else { gpui::CursorStyle::ResizeLeftRight }));
        if pressed {
            let _ = app.session.begin_interaction(tr("Gap"));
            app.handles.moved = false;
            app.handles.drag = Some(Drag::Gap { id: z.id, start: pos, value: g.value, vertical: g.vertical, key: g.key });
            return true;
        }
        return false;
    }
    false
}

/// 拖动中的位移 → 命令（原 `apply` 的 Padding / Gap 两支）。
fn apply(app: &mut CanvasApp, drag: &Drag, pos: Pos2, shift: bool) {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6);
    match drag {
        Drag::Padding { id, side, start, value } => {
            let d = f64::from(match side {
                0 => pos.y - start.y,
                1 => start.x - pos.x,
                2 => start.y - pos.y,
                _ => pos.x - start.x,
            }) / zoom;
            let raw = value + d;
            let key = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"].get(*side).copied().unwrap_or("paddingTop");
            // Shift drags both sides of the axis together.
            // 行为与仓库原版一致：padding = 起点值 + 位移，只夹到 >= 0。
            let v = raw.max(0.0).round();
            // 取整后与文档里的现值相同就不发命令：重复的 Move 事件不该换来一份新文档 + 一次渲染。
            let now = app.session.doc().ok().and_then(|d| d.node(*id).and_then(|n| n.layout.as_ref()).map(|l| l.padding[*side])).unwrap_or(f64::NAN);
            if (now - v).abs() < 0.5 {
                return;
            }
            app.handles.moved = true;
            let props =
                if shift { if side % 2 == 0 { json!({"paddingVertical": v}) } else { json!({ "paddingHorizontal": v }) } } else { json!({ key: v }) };
            let _ = app.session.execute("node.set", &json!({"ids": [id], "props": props}));
        }
        Drag::Gap { id, start, value, vertical, key } => {
            let d = f64::from(if *vertical { pos.y - start.y } else { pos.x - start.x }) / zoom;
            let v = (value + d).round().max(0.0);
            // Dragging an auto gap fixes it at what the drag makes of the distance it had.
            let mut props = json!({ *key: v });
            match (*key, app.session.doc().ok().and_then(|d| d.node(*id)).and_then(|n| n.layout.as_ref())) {
                ("crossGap", Some(l)) if l.content_align == ContentAlign::SpaceBetween => props["contentAlign"] = json!("auto"),
                // 原实现：`MainAlign::Min` 的序列化是 `"min"`；网格不走自动间距。
                ("gap", Some(l)) if l.main_align.is_auto_gap() && l.mode != LayoutMode::Grid => props["mainAlign"] = json!("min"),
                _ => {}
            }
            app.handles.moved = true;
            let _ = app.session.execute("node.set", &json!({"ids": [id], "props": props}));
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// 拖 padding / gap 手柄时，画布要把它当成 resize 那样的 live 图层来当帧重抓；
    /// 只是按了一下（没过阈值）不算，否则会平白多抓一张。
    #[test]
    fn dragging_a_padding_band_reports_the_frame_as_live() {
        let mut st = State::default();
        assert!(st.live_layers().is_none(), "没在拖：什么都不抢");
        st.drag = Some(Drag::Padding { id: NodeId(7), side: 0, start: pos2(0.0, 0.0), value: 8.0 });
        assert!(st.live_layers().is_none(), "刚按下、还没动：不抢画布");
        st.moved = true;
        let (ids, redraw) = st.live_layers().expect("拖动中：这一帧由画布自己画");
        assert_eq!(ids, vec![NodeId(7)]);
        assert!(redraw, "子层跟着重排：每帧都要重抓");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::{AutoLayout, Document, NodeKind};
    use figcraft_engine::Session;

    fn app_with_padding(pad: [f64; 4]) -> (CanvasApp, NodeId) {
        let mut doc = Document::new("handles");
        let page = doc.first_page().expect("page");
        let f = doc.create(page, NodeKind::Frame, "AL", 0.0, 0.0, 200.0, 100.0).expect("frame");
        if let Ok(n) = doc.get_mut(f) {
            n.layout = Some(AutoLayout { mode: LayoutMode::Horizontal, gap: 10.0, padding: pad, ..Default::default() });
        }
        let c = doc.create(f, NodeKind::Rectangle, "item", 40.0, 40.0, 40.0, 20.0).expect("child");
        let _ = c;
        let mut app = CanvasApp::new(doc);
        let _ = app.session.set_selection(vec![f]);
        (app, f)
    }

    /// 四条 padding 带按源码的 `pt.max(4.0)` 撑开：padding 为 0 时仍有 4px 可抓。
    #[test]
    fn padding_bands_keep_a_four_pixel_grip() {
        let (mut app, _) = app_with_padding([0.0; 4]);
        // `CanvasState` 的默认 rect 是 Rect::NOTHING（±inf）：真机上每帧由视口填，测试里要自己给。
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let z = al_zones(&app).expect("zones for an auto layout frame");
        assert_eq!(z.bands.len(), 4);
        for (_, r, v) in &z.bands {
            assert!(*v == 0.0);
            assert!(r.width() > 0.5 && r.height() > 0.5, "a {r:?} band is grabbable");
        }
    }

    /// 拖动一条 padding 带 = `paddingTop` 加同样的量（原 `apply` 的算法）。
    #[test]
    fn dragging_the_top_band_writes_the_padding() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        let start = pos2(20.0, 5.0);
        let drag = Drag::Padding { id: f, side: 0, start, value: 8.0 };
        app.handles.drag = Some(drag.clone());
        apply(&mut app, &drag, pos2(20.0, 15.0), false);
        let pad = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.layout.as_ref()).map(|l| l.padding);
        assert_eq!(pad.map(|p| p[0]), Some(18.0), "top padding follows the drag");
    }

    /// 悬停做成「粘性」：落在底边距后，移到角落（仍属底边距）不跳到左/右侧；
    /// 只有离开底边距进入左带才重判为 left；移到中心空白则清空。
    #[test]
    fn hover_is_sticky_until_leaving_the_band() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        // 200×100 的 frame、padding 8：bands 为 top/right/bottom/left，其中
        // bottom 横贯整宽（含左下角落）、left 占满整高。坐标取屏幕空间（与指针一致）。
        let z = al_zones(&app).expect("zones");
        let band_rect = |side: usize| z.bands.iter().find(|(s, _, _)| *s == side).map(|(_, r, _)| *r).expect("band");
        let bottom = band_rect(2);
        let left = band_rect(3);

        // 悬停底边距（side 2）：取底边距正中。
        let on_bottom = bottom.center();
        app.pointer = Some(on_bottom);
        update_hover(&mut app);
        assert_eq!(app.handles.hover, Some(Hover::Padding { id: f, side: 2 }));

        // 移到左下角落（同时落在 left 与 bottom 内）：仍锁定 bottom，不跳到 left。
        let corner = pos2(left.min.x + 1.0, bottom.max.y - 1.0);
        assert!(bottom.contains(corner) && left.contains(corner), "角落点应两带重叠");
        app.pointer = Some(corner);
        update_hover(&mut app);
        assert_eq!(app.handles.hover, Some(Hover::Padding { id: f, side: 2 }), "角落仍锁定底边距，不应跳到相邻带");

        // 离开底边距、进入左侧带（side 3）区域：重判为 left。
        let on_left = pos2(left.min.x + 1.0, left.center().y);
        assert!(left.contains(on_left) && !bottom.contains(on_left), "左侧点不应落在底边距内");
        app.pointer = Some(on_left);
        update_hover(&mut app);
        assert_eq!(app.handles.hover, Some(Hover::Padding { id: f, side: 3 }));

        // 移到中心空白（既不在 padding 也不在 gap）：无悬停。
        let center = app.to_screen(100.0, 50.0);
        app.pointer = Some(center);
        update_hover(&mut app);
        assert_eq!(app.handles.hover, None);

        // 拖动中清空悬停，由被拖的带子接管。
        app.handles.drag = Some(Drag::Padding { id: f, side: 0, start: on_bottom, value: 8.0 });
        app.pointer = Some(on_bottom);
        update_hover(&mut app);
        assert_eq!(app.handles.hover, None, "拖动时悬停态应清空");
        app.handles.drag = None;
    }

    /// 悬停到带子上不接管指针（原实现 `input()` 只在按下时 `return true`），
    /// 让画布继续把 hover 交给引擎；按下才归手柄。
    #[test]
    fn hover_over_a_band_does_not_own_the_pointer() {
        let (mut app, _) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.session.tool.tool = ToolId::Move;
        let z = al_zones(&app).expect("zones");
        let on_top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(!input(&mut app, on_top, false, false, false, false), "hover must not swallow the pointer");
        assert!(app.spacing_cursor.is_some(), "hover still shows the resize cursor");
        assert!(input(&mut app, on_top, true, true, false, false), "press owns the pointer");
        assert!(app.handles.drag.is_some(), "press starts the padding drag");
    }

    /// 拖动自动间距（`mainAlign` 是 auto gap）后把它固定成 `min`（源码的 `"min"`，
    /// 不是 `"start"`）；网格不参与。
    #[test]
    fn dragging_an_auto_gap_fixes_it_at_min_align() {
        let mut doc = Document::new("gap");
        let page = doc.first_page().expect("page");
        let f = doc.create(page, NodeKind::Frame, "AL", 0.0, 0.0, 200.0, 100.0).expect("frame");
        if let Ok(n) = doc.get_mut(f) {
            n.layout = Some(AutoLayout {
                mode: LayoutMode::Horizontal,
                main_align: figcraft_doc::MainAlign::SpaceBetween,
                gap: 10.0,
                padding: [8.0; 4],
                ..Default::default()
            });
        }
        let mut app = CanvasApp::new(doc);
        let _ = app.session.set_selection(vec![f]);
        let drag = Drag::Gap { id: f, start: pos2(0.0, 0.0), value: 10.0, vertical: false, key: "gap" };
        apply(&mut app, &drag, pos2(14.0, 0.0), false);
        let align = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.layout.as_ref()).map(|l| l.main_align);
        assert_eq!(align, Some(figcraft_doc::MainAlign::Min), "auto gap drag pins mainAlign to min");
    }

    /// 整条 input → apply 路径：拖底边距带，只有底边距变，顶边距保持。
    #[test]
    fn input_drag_on_bottom_band_writes_only_the_bottom() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.session.tool.tool = ToolId::Move;
        let z = al_zones(&app).expect("zones");
        let bottom = z.bands.iter().find(|(s, _, _)| *s == 2).map(|(_, r, _)| r.center()).expect("bottom band");
        assert!(input(&mut app, bottom, true, true, false, false), "down on the bottom band");
        let moved = pos2(bottom.x, bottom.y - 10.0);
        assert!(input(&mut app, moved, false, true, false, false), "move up 10");
        let pad = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.layout.as_ref()).map(|l| l.padding);
        assert_eq!(pad.map(|p| p[2]), Some(18.0), "bottom padding grows");
        assert_eq!(pad.map(|p| p[0]), Some(8.0), "top padding untouched without shift");
    }

    /// Shift 拖同一条带子：同轴两侧一起变（源码 `paddingVertical`）。
    #[test]
    fn input_shift_drag_moves_both_sides_of_the_axis() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.session.tool.tool = ToolId::Move;
        let z = al_zones(&app).expect("zones");
        let top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(input(&mut app, top, true, true, false, false), "down on the top band");
        let moved = pos2(top.x, top.y + 10.0);
        assert!(input(&mut app, moved, false, true, false, true), "move down 10 with shift");
        let pad = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.layout.as_ref()).map(|l| l.padding);
        assert_eq!(pad, Some([18.0, 8.0, 18.0, 8.0]), "shift moves both sides of the axis");
    }

    /// 改边距后子层必须跟着重排（`normalize` 的 layout_frame）——否则看着就是「拖了没反应」。
    #[test]
    fn padding_edit_reflows_the_children() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        let child = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.children.first().copied()).expect("child");
        let before = app.session.doc().ok().and_then(|d| d.node(child)).map(|n| (n.transform, n.width, n.height));
        let drag = Drag::Padding { id: f, side: 0, start: pos2(20.0, 5.0), value: 8.0 };
        apply(&mut app, &drag, pos2(20.0, 25.0), false);
        let after = app.session.doc().ok().and_then(|d| d.node(child)).map(|n| (n.transform, n.width, n.height));
        assert_ne!(before, after, "the child must follow the new padding (layout reflow)");
    }

    /// 自适应（Hug）的自动布局 frame：加顶边距会改变 frame 的高度（另一条边跟着动）。
    #[test]
    fn hugging_frame_resizes_with_the_top_padding() {
        let mut doc = Document::new("hug");
        let page = doc.first_page().expect("page");
        let f = doc.create(page, NodeKind::Frame, "AL", 0.0, 0.0, 200.0, 100.0).expect("frame");
        if let Ok(n) = doc.get_mut(f) {
            n.layout = Some(AutoLayout { mode: LayoutMode::Vertical, gap: 10.0, padding: [8.0; 4], ..Default::default() });
            n.child_layout.sizing_v = figcraft_doc::Sizing::Hug;
        }
        let _ = doc.create(f, NodeKind::Rectangle, "item", 8.0, 8.0, 40.0, 20.0).expect("child");
        let mut app = CanvasApp::new(doc);
        let _ = app.session.set_selection(vec![f]);
        let before = app.session.doc().ok().and_then(|d| d.node(f)).map(|n| n.height).unwrap_or(0.0);
        let drag = Drag::Padding { id: f, side: 0, start: pos2(20.0, 5.0), value: 8.0 };
        apply(&mut app, &drag, pos2(20.0, 25.0), false);
        let after = app.session.doc().ok().and_then(|d| d.node(f)).map(|n| n.height).unwrap_or(0.0);
        assert_ne!(after, before, "a hugging frame resizes with its top padding ({before} -> {after})");
    }

    /// 真实手势路径（begin_interaction + execute）：拖顶边距带必须把子层往下挤。
    #[test]
    fn input_drag_on_top_band_pushes_the_children_down() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.session.tool.tool = ToolId::Move;
        let child = app.session.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.children.first().copied()).expect("child");
        let before = app.session.doc().ok().and_then(|d| d.node(child)).map(|n| n.transform);
        let z = al_zones(&app).expect("zones");
        let top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(input(&mut app, top, true, true, false, false), "down");
        assert!(input(&mut app, pos2(top.x, top.y + 10.0), false, true, false, false), "move down 10");
        let after = app.session.doc().ok().and_then(|d| d.node(child)).map(|n| n.transform);
        assert_ne!(before, after, "the child must be pushed down by the new top padding");
    }

    /// 拖边距必须换掉文档 Arc：渲染层用 `Arc::as_ptr` 当键，指针不变就不会重画（画面就冻住）。
    #[test]
    fn padding_drag_changes_the_doc_arc() {
        let (mut app, f) = app_with_padding([8.0; 4]);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.session.tool.tool = ToolId::Move;
        let ptr = |app: &CanvasApp| app.session.ds().ok().map(|d| std::sync::Arc::as_ptr(&d.doc) as usize);
        let p0 = ptr(&app);
        let z = al_zones(&app).expect("zones");
        let top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(input(&mut app, top, true, true, false, false), "down");
        assert!(input(&mut app, pos2(top.x, top.y + 10.0), false, true, false, false), "move");
        let p1 = ptr(&app);
        let _ = f;
        assert_ne!(p0, p1, "the document Arc must change so the canvas re-renders");
    }

    /// 用**应用实际打开的示例文档**复现：拖 AutoLayout · H 的顶边距，子层必须跟着动。
    #[test]
    fn sample_auto_layout_drag_reflows_the_children() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1294.0, 897.0));
        app.session.tool.tool = ToolId::Move;
        let (f, child, before) = {
            let ds = app.session.ds().expect("session");
            let f = ds.selection.first().copied().expect("sample preselects AutoLayout · H");
            let child = ds.doc.node(f).and_then(|n| n.children.first().copied()).expect("child");
            let before = ds.doc.node(child).map(|n| n.transform);
            (f, child, before)
        };
        let z = al_zones(&app).expect("zones for the preselected auto layout frame");
        let top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(input(&mut app, top, true, true, false, false), "down");
        assert!(input(&mut app, pos2(top.x, top.y + 20.0), false, true, false, false), "move down 20");
        let after = app.session.ds().expect("session").doc.node(child).map(|n| n.transform);
        let _ = f;
        assert_ne!(before, after, "the sample's child must be pushed down by the new padding");
    }

    /// 示例里的自动布局 frame 是 Hug：拖顶边距必须把 frame 撑高（盒子变大），而不是原地裁掉内容。
    #[test]
    fn sample_hug_frame_grows_with_padding() {
        let mut app = CanvasApp::new(crate::app::sample_doc());
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1294.0, 897.0));
        app.session.tool.tool = ToolId::Move;
        let f = app.session.ds().expect("session").selection.first().copied().expect("preselected");
        let before = app.session.ds().ok().and_then(|d| d.doc.node(f)).map(|n| n.height).unwrap_or(0.0);
        let z = al_zones(&app).expect("zones");
        let top = z.bands.iter().find(|(s, _, _)| *s == 0).map(|(_, r, _)| r.center()).expect("top band");
        assert!(input(&mut app, top, true, true, false, false), "down");
        assert!(input(&mut app, pos2(top.x, top.y + 20.0), false, true, false, false), "move down 20");
        let after = app.session.ds().ok().and_then(|d| d.doc.node(f)).map(|n| n.height).unwrap_or(0.0);
        assert!(after > before, "the sample's hugging frame must grow ({before} -> {after})");
    }

    /// 选中的矩形露出四个圆角手柄（几何由引擎 `shape_handles` 给，叠加层只负责画）。
    #[test]
    fn shape_handles_show_four_corner_radii_on_a_rectangle() {
        let mut doc = Document::new("shape");
        let page = doc.first_page().expect("page");
        let r = doc.create(page, NodeKind::Rectangle, "rect", 0.0, 0.0, 200.0, 200.0).expect("rect");
        let mut app = CanvasApp::new(doc);
        let _ = app.session.set_selection(vec![r]);
        app.session.tool.tool = ToolId::Move;
        let hs = app.session.shape_handles();
        assert_eq!(hs.len(), 4, "a rectangle has one radius handle inside each corner");
        assert!(hs.iter().all(|(h, _)| matches!(*h, figcraft_engine::shape_handles::ShapeHandle::Radius { .. })));
    }
}
