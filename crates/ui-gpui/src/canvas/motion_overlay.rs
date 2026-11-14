//! Motion 模式的画布叠加层：选中层的**运动路径**（轨迹曲线）与**变换手柄**（gizmo）。
//!
//! 移植自 desigma `crates/desigma-app/src/motion/inspector/overlay.rs`（`paint_overlay`），
//! 并按用户 2026-11-12 的规格补齐交互：
//! - 路径：**只由空间信息**决定（关键帧位置 + 关键帧空间手柄），有手柄走三次贝塞尔、
//!   没手柄走直线，采样用的是**几何参数**而不是时间——缓动/弹簧只决定物体沿这条线走多快，
//!   绝不改线的形状（用户 2026-11-14）。
//!   **轨迹钉死在静态文档坐标系**（父链/尺寸取未评估的源文档），拖播放头时不会跟着漂。
//! - 手柄：圆环（旋转）/ 绿箭头（沿局部 X 移动）/ 粉箭头（沿局部 Y 移动）/ 箭头根部两个
//!   缩放块。两根轴按**动画 Rotation 相对设计角的增量**旋转：设计里斜 45°、动画增量为 0
//!   时箭头仍是屏幕正方向（用户 2026-11-12），只有在动画里转了箭头才跟着转。
//! - 悬停/拖拽：圆环 hover 蓝环变粗、箭头 hover 变长、块 hover 高亮；拖动写播放头关键帧
//!   （`motion.autoKey`），一次拖动 = 一个 undo step。
//!
//! 显隐：Motion 模式打开、不在播放、且有选中层时才画/才接管输入。

use std::sync::Arc;

use figcraft_doc::anim::motion::{self, AnimProperty, AnimValue};
use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Affine, Point};
use gpui::CursorStyle;

use super::geom::*;
use super::painter::Painter;
use crate::app::CanvasApp;
use crate::canvas::cursors::Shown;
use crate::theme::{Tokens, stroke as s};

/// gizmo 的 Y 轴（粉）与 X 轴（绿）——desigma `AXIS_Y` / `AXIS_X`。
const AXIS_Y: Color32 = Color32::from_rgb(0xF2, 0x4E, 0x7B);
const AXIS_X: Color32 = Color32::from_rgb(0x3D, 0xD6, 0x8C);
/// 旋转环的蓝色（desigma `theme::CANVAS_SELECTION = 0x0D99FF`，源码常量，非截图测绘）。
const RING_BLUE: Color32 = Color32::from_rgb(0x0D, 0x99, 0xFF);
/// 手柄圆环半径（desigma `RING_R`）。
const RING_R: f32 = 33.0;
/// 白色是蓝色环的**描边**：这一侧露出的白边宽度恒定，不随 hover 变化（用户 2026-11-12 定 1）。
const RING_OUTLINE_W: f32 = 1.0;
/// 蓝色环（环带）宽度：常态 2 / hover 变宽。变宽时把白描边整体往外挤。
const RING_W: f32 = 2.0;
const RING_W_HOVER: f32 = 3.0;
/// 轴箭头长度：常态 30 / hover 35（用户 2026-11-12 指定）。
const ARROW_LEN: f32 = 30.0;
const ARROW_LEN_HOVER: f32 = 35.0;
/// 缩放块半边长：常态 / hover（箭头与圆环相接处的两个块）。
const BLOCK_R: f32 = 4.0;
const BLOCK_R_HOVER: f32 = 5.5;
/// 命中容差（屏幕像素）。
const HIT_PAD: f32 = 6.0;
/// 缩放拖动灵敏度：沿轴每拖 100 页面像素，scale 变 1.0。
const SCALE_PER_PX: f64 = 1.0 / 100.0;

/// 关键帧空间手柄的哪一端。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// 这一段**离开**本帧时的切线方向（下一段的起点控制点）。
    Out,
    /// 这一段**进入**本帧时的切线方向（上一段的终点控制点）。
    In,
}

/// gizmo 上可命中/可拖动的部件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// 圆环：拖动改动画 Rotation（相对设计角的增量）。
    Ring,
    /// 中心菱形：拖动改**动画里的 Position**（自由 2D）。
    /// 菱形以外、圆环以外的正常拖动仍归画布（改整体/静态位置）。
    Center,
    /// 运动路径上的**空间控制点**（某个关键帧）：拖动改**该时刻的 Position**，
    /// 也就是把运动轨迹在空间里拉过去（用户 2026-11-12）。
    Keyframe(u32),
    /// 选中关键帧的**空间贝塞尔手柄端点**：拖动改该端切线（曲线形状），拖回关键帧中心则删除
    /// （归直）。照矢量路径编辑的贝塞尔手柄交互（用户 2026-11-12）。
    KeyframeHandle(u32, End),
    /// 绿箭头：沿旋转后的局部 X 轴移动 Position。
    ArrowX,
    /// 粉箭头：沿旋转后的局部 Y 轴移动 Position。
    ArrowY,
    /// 绿箭头根部的块：X 方向缩放（箭头正方向 = 放大）。
    ScaleX,
    /// 粉箭头根部的块：Y 方向缩放。
    ScaleY,
}

/// 一次拖动的持久状态。`State`（每帧重建）放不下， Lives in `CanvasApp::motion_gizmo`。
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub part: Part,
    pub node: NodeId,
    /// 按下点（画布局部像素）。
    pub start: Pos2,
    /// gizmo 中心（画布局部像素），旋转的轴心。
    pub center: Pos2,
    /// 旋转后的屏幕单位轴（局部 +X：向左基准；局部 +Y：向上基准）。
    pub axis_x: Vec2,
    pub axis_y: Vec2,
    /// 按下时的设计角 / 动画旋转增量 / 播放头。
    pub base_rot: f64,
    pub start_delta: f64,
    /// 父级世界变换**线性部分之逆**（2×2，行主序 `[a,b,c,d]`）。屏幕位移除以 zoom 得页面
    /// 位移，再乘它才得到节点局部（父坐标系）位移——父级旋转/缩放时 Center/Keyframe 拖动
    /// 方向才正确。父级无变换时为单位阵，与旧行为一致（CodeRabbit）。
    pub parent_inv: [f64; 4],
    pub time: u32,
    /// 按下时各属性的动画值（快照，避免拖动中累积漂移）。
    pub start_pos: figcraft_geom::Vec2,
    pub start_scale: figcraft_geom::Vec2,
    /// 按下时指针相对中心的角度（弧度，屏幕坐标顺时针为正）。
    pub start_angle: f64,
    /// Ctrl 按下时拖路径关键帧 = 拉**空间贝塞尔手柄**（对称），而不是移动该帧。
    pub bend: bool,
}

/// gizmo 交互状态：跨帧存活，唯一放拖动句柄的地方。
#[derive(Clone, Default)]
pub(crate) struct Interaction {
    pub drag: Option<Drag>,
    /// 被选中的路径关键帧 `(层, time_ms)`：选中后显示它的空间手柄，可 Ctrl 拖出曲线
    /// （照矢量路径编辑的交互模式，用户 2026-11-12）。
    pub selected_key: Option<(NodeId, u32)>,
}

/// 叠加层每帧的输入。`CanvasView::sync_motion` 从 Motion 状态同步过来。
#[derive(Default, Clone)]
pub struct State {
    /// Motion 模式打开 **且** 不在播放、且有选中层 → 画 + 接管输入。
    pub active: bool,
    /// 是否正在播放（即使 active 为 false 时也可能在播放，例如播放中被别人接管输入）。
    /// 画布上的**选中边框**在播放时应当隐藏（用户 2026-11-12：播放中选中框还留在原地）。
    pub playing: bool,
    /// 播放头（ms）：手柄落在这帧的位置；落在关键帧上时那颗菱形画实心。
    pub playhead_ms: f64,
    /// 路径采样步长用的帧率。
    pub fps: u32,
    /// 时间轴总时长（ms）：算 clip 的结束时间要用（`span_ms`）。
    pub duration_ms: u32,
    /// 右栏准星按钮把手柄关掉了（desigma `motion_ins.gizmo_hidden`）。
    pub gizmo_hidden: bool,
    /// 播放头下评估过的文档：gizmo 中心按它算，所以手柄跟着当前帧的层走。
    /// **轨迹不用它**（轨迹必须钉死在静态坐标系）。
    pub preview: Option<Arc<Document>>,
    /// **静态源文档**（未求值）。渲染期间 `ds.doc` 会被临时换成 `preview`（lib.rs 的
    /// `motion_preview_doc` 替换），若直接读它，画手柄时拿到"求值后文档"、鼠标命中时拿到
    /// "静态文档"，两边的角度基准与手柄几何就错开（热区偏、设计角串进动画）。
    /// 轨迹、角度基准、命中一律只用这一份；只有 gizmo 中心用 `preview`。
    pub src: Option<Arc<Document>>,
}

impl State {
    /// 显隐：打开 Motion 且不在播放，具体有没有选中层由 `draw` / `input` 再判。
    pub fn new(
        open: bool,
        playing: bool,
        playhead_ms: f64,
        fps: u32,
        duration_ms: u32,
        preview: Option<Arc<Document>>,
        src: Option<Arc<Document>>,
    ) -> Self {
        Self { active: open && !playing, playing, playhead_ms, fps, duration_ms, gizmo_hidden: false, preview, src }
    }
}

/// `id` 父级的世界变换（不含它自己的 transform）。
fn parent_world(doc: &Document, id: NodeId) -> Affine {
    doc.node(id).and_then(|n| n.parent).map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY)
}

/// `id` 的布局位置为 `pos` 时，它中心在屏幕上的位置。
///
/// `doc` 传**静态源文档**：父链变换与尺寸都不带播放头评估，于是同一条轨迹在不同播放头下
/// 采样出的点完全一致 —— 轨迹钉死不动（用户 2026-11-12）。
fn centre_at(app: &CanvasApp, doc: &Document, id: NodeId, pos: figcraft_geom::Vec2) -> Option<Pos2> {
    let n = doc.node(id)?;
    let (w, h) = n.size();
    let c = parent_world(doc, id) * Point::new(pos.x + w * 0.5, pos.y + h * 0.5);
    Some(app.to_screen(c.x, c.y))
}

/// `id` 在 `t_ms` 的运动路径上的点（关键帧 + clip 预设都算），静态父链。
///
/// **不含缓动**（见 [`position_value`]）：轨迹、菱形、手柄锚点全用同一个空间量。
fn path_point(app: &CanvasApp, src: &Document, id: NodeId, t_ms: f64) -> Option<Pos2> {
    centre_at(app, src, id, position_value(src, id, t_ms)?)
}

/// `id` 在 `t_ms` 的 Position **值**（页面坐标，含 clip 位移），不转屏幕。
///
/// 用 [`motion::path_position_at`]（**取掉缓动**）：画轨迹时段的端点也只取空间量，
/// 缓动/弹簧的过冲不会把线拉出关键帧（用户 2026-11-14）。
fn position_value(src: &Document, id: NodeId, t_ms: f64) -> Option<figcraft_geom::Vec2> {
    Some(motion::path_position_at(src.node(id)?, t_ms))
}

/// 关键帧 `t_ms` 那一端的**非零空间手柄**（相对该关键帧值的页面偏移）。
///
/// 「精确为 (0,0)」按直线处理（照矢量路径的 `Segment::is_straight`）——零手柄的三次贝塞尔
/// **不等于**线性插值，会把直线画弯（用户 2026-11-12）。
fn handle_at(src: &Document, id: NodeId, t_ms: f64, end: End) -> Option<[f64; 2]> {
    let k = src.node(id)?.motion.track(AnimProperty::Position)?.key_at(t_ms as u32)?;
    let h = match end {
        End::Out => k.out_handle,
        End::In => k.in_handle,
    }?;
    (h[0] != 0.0 || h[1] != 0.0).then_some(h)
}

/// gizmo 中心：层在**当前播放头**画在哪（含自身/clip 位移），手柄跟着它走。
fn gizmo_centre(app: &CanvasApp, shown: &Document, id: NodeId) -> Option<Pos2> {
    let n = shown.node(id)?;
    let (w, h) = n.size();
    let c = shown.world_transform(id) * Point::new(w * 0.5, h * 0.5);
    Some(app.to_screen(c.x, c.y))
}

/// 2D 向量旋转（kurbo `Affine::rotate` 同款矩阵，屏幕 y 向下时正角 = 顺时针）。
fn rot(v: Vec2, a_rad: f64) -> Vec2 {
    let (s, c) = a_rad.sin_cos();
    vec2((f64::from(v.x) * c - f64::from(v.y) * s) as f32, (f64::from(v.x) * s + f64::from(v.y) * c) as f32)
}

/// 一个选中层的手柄几何。
struct Geo {
    node: NodeId,
    center: Pos2,
    /// 动画 Rotation 相对设计角的增量（度，Figma 逆时针为正）。设计 45°、动画 0 时为 0。
    delta_deg: f64,
    /// 屏幕单位轴：局部 +X 以「向左」为基准，局部 +Y 以「向上」为基准，再按增量旋转。
    axis_x: Vec2,
    axis_y: Vec2,
    base_rot: f64,
    /// 运动路径上的关键帧点：`(time_ms, 屏幕位置)`，可拖（改该时刻的空间位置）。
    keys: Vec<(u32, Pos2)>,
    /// 选中关键帧的**空间贝塞尔手柄端点**（屏幕位置）：只有选中的帧才画 / 可拖
    /// （照矢量路径交互），用于在曲线上拉出 / 调整 / 删除切线。
    handle_ends: Vec<(u32, End, Pos2)>,
}

/// 算出所有「有动画」的选中层的手柄几何。
fn geos(app: &CanvasApp) -> Vec<Geo> {
    let st = &app.motion_overlay;
    if !st.active || st.gizmo_hidden {
        return Vec::new();
    }
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    if ds.selection.is_empty() {
        return Vec::new();
    }
    // 静态源文档：渲染期 `ds.doc` 会被换成 preview，直接读它会让角度基准与轨迹父链
    // 跟着播放头漂（用户 2026-11-12）。只有手柄中心用求值后的 `shown`。
    let src = st.src.clone().unwrap_or_else(|| ds.doc.clone());
    let shown = st.preview.clone().unwrap_or_else(|| src.clone());
    let t = st.playhead_ms;
    let mut out = Vec::new();
    for id in &ds.selection {
        let Some(node) = src.node(*id).filter(|n| !n.motion.is_empty()) else { continue };
        let Some(center) = gizmo_centre(app, &shown, *id) else { continue };
        let base_rot = motion::base_rotation_deg(node);
        // 关键：动画 Rotation 是相对设计角的增量。没有 Rotation 轨道/clip 时 value_at 回退到
        // base_rotation_deg，于是 delta=0 → 箭头是屏幕正方向（用户 2026-11-12）。
        let r = motion::value_at(node, AnimProperty::Rotation, t).and_then(|v| v.scalar()).unwrap_or(base_rot);
        let delta_deg = r - base_rot;
        // Figma 逆时针为正；屏幕 y 向下，故施加 kurbo 的 -delta。
        let a = -delta_deg.to_radians();
        // 运动路径上的关键帧点（屏幕位置），供拖动改空间位置。
        let keys: Vec<(u32, Pos2)> = node
            .motion
            .track(AnimProperty::Position)
            .map(|tr| tr.keys.iter().filter_map(|k| path_point(app, &src, *id, f64::from(k.time_ms)).map(|p| (k.time_ms, p))).collect())
            .unwrap_or_default();
        // 选中关键帧的空间贝塞尔手柄端点（屏幕位置）：只有选中的帧才画 / 可拖（照矢量路径交互）。
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6) as f32;
        let mut handle_ends = Vec::new();
        if let Some((sn, st_time)) = app.motion_gizmo.selected_key {
            if sn == *id {
                if let Some(k) = node.motion.track(AnimProperty::Position).and_then(|tr| tr.key_at(st_time)) {
                    let c = path_point(app, &src, *id, f64::from(st_time)).unwrap_or(center);
                    for (which, h) in [(End::Out, k.out_handle), (End::In, k.in_handle)] {
                        if let Some(h) = h {
                            let hp = pos2(c.x + h[0] as f32 * zoom, c.y + h[1] as f32 * zoom);
                            handle_ends.push((st_time, which, hp));
                        }
                    }
                }
            }
        }
        out.push(Geo {
            node: *id,
            center,
            delta_deg,
            // 绿箭头朝上、粉箭头朝右（用户 2026-11-12 更正：之前左右/上下弄反了）。
            axis_x: rot(vec2(0.0, -1.0), a), // 绿箭头基准：向上
            axis_y: rot(vec2(1.0, 0.0), a),  // 粉箭头基准：向右
            base_rot,
            keys,
            handle_ends,
        });
    }
    out
}

/// 点到线段的距离（屏幕像素）。
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 <= 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// 命中测试：块 → 箭头 → 圆环（块压在箭头起点、箭头起点压在圆环上，故这个顺序）。
fn hit(g: &Geo, p: Pos2) -> Option<Part> {
    // ① 运动路径上的关键帧点**最优先**：拖它 = 改该时刻的空间位置（用户 2026-11-12）。
    // 命中半径 = 菱形实际大小（6 + 1）。不能再大：播放头停在关键帧上时它与中心菱形重合，
    // 半径一大，用户拖物体本体（想整体移动）就被它抢走（用户 2026-11-12）。
    for (t, c) in &g.keys {
        if p.distance(*c) <= 6.0 + 1.0 {
            return Some(Part::Keyframe(*t));
        }
    }
    // ③ 选中关键帧的空间贝塞尔手柄端点：点中可拖动改曲线；拖回关键帧中心则删除（归直）。
    // 手柄画在选中帧上，且离关键帧中心有一段距离，落在关键帧 7px 命中环之外，故放在关键帧之后。
    for (t, end, hp) in &g.handle_ends {
        if p.distance(*hp) <= 5.0 + 1.0 {
            return Some(Part::KeyframeHandle(*t, *end));
        }
    }
    // ② 中心菱形：拖它 = 改动画里的 Position。**命中范围收到菱形本身**（4.5 + 2），
    // 不能像以前那样 4.5 + HIT_PAD(6) = 10.5：它就画在物体中心，范围一大，
    // 用户拖物体本体（想整体移动）就被它抢走（用户 2026-11-12："正常拖动被删了"）。
    if p.distance(g.center) <= 4.5 + 2.0 {
        return Some(Part::Center);
    }
    let sx = g.center + g.axis_x * RING_R;
    let sy = g.center + g.axis_y * RING_R;
    if p.distance(sx) <= BLOCK_R + HIT_PAD {
        return Some(Part::ScaleX);
    }
    if p.distance(sy) <= BLOCK_R + HIT_PAD {
        return Some(Part::ScaleY);
    }
    let tipx = g.center + g.axis_x * (RING_R + ARROW_LEN_HOVER);
    let tipy = g.center + g.axis_y * (RING_R + ARROW_LEN_HOVER);
    if seg_dist(p, sx, tipx) <= HIT_PAD {
        return Some(Part::ArrowX);
    }
    if seg_dist(p, sy, tipy) <= HIT_PAD {
        return Some(Part::ArrowY);
    }
    if (p.distance(g.center) - RING_R).abs() <= HIT_PAD {
        return Some(Part::Ring);
    }
    None
}

/// 部件的悬停光标。
fn cursor_for(part: Part, g: &Geo) -> Shown {
    match part {
        Part::Center | Part::Keyframe(_) | Part::KeyframeHandle(_, _) => Shown::System(gpui::CursorStyle::ClosedHand),
        Part::Ring => Shown::Drawn(crate::canvas::cursors::Glyph::Rotate(g.delta_deg as f32)),
        Part::ArrowX | Part::ScaleX => {
            Shown::System(if g.axis_x.x.abs() >= g.axis_x.y.abs() { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown })
        }
        Part::ArrowY | Part::ScaleY => {
            Shown::System(if g.axis_y.x.abs() >= g.axis_y.y.abs() { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown })
        }
    }
}

/// 菱形四角（中心 `c`、半径 `r`）。
fn diamond(c: Pos2, r: f32) -> Vec<Pos2> {
    vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)]
}

/// 正方形四角（缩放块，屏幕轴对齐）。
fn square(c: Pos2, r: f32) -> Vec<Pos2> {
    vec![c + vec2(-r, -r), c + vec2(r, -r), c + vec2(r, r), c + vec2(-r, r)]
}

/// 带箭头的轴线：一根杆 + 一个「倒 V」的线条箭头（不是实心三角），整体带白色描边。
///
/// 用户 2026-11-12 的规格：
/// - 两条分叉与杆成 **45°**（回折距离与侧移相等）；
/// - **圆头**：首尾与箭头都不是平头，端点补圆；
/// - 白色描边先打底（粗 1.5pt）再压彩色细线。
fn arrow(p: &mut Painter, from: Pos2, to: Pos2, col: Color32, w: f32) {
    let d = (to - from).normalized();
    let n = vec2(-d.y, d.x);
    let len = 7.0;
    // 45°：沿杆回折 len，同时侧移 len。
    let a1 = to - d * len + n * len;
    let a2 = to - d * len - n * len;
    let ow = w + 1.5;
    let segs = [(from, to), (a1, to), (a2, to)];
    // 白色描边打底（圆头）。
    for (a, b) in segs {
        p.line_segment([a, b], s(Color32::WHITE, ow));
        p.circle_filled(a, ow / 2.0, Color32::WHITE);
        p.circle_filled(b, ow / 2.0, Color32::WHITE);
    }
    // 彩色细线压在白底上（圆头）。
    for (a, b) in segs {
        p.line_segment([a, b], s(col, w));
        p.circle_filled(a, w / 2.0, col);
        p.circle_filled(b, w / 2.0, col);
    }
}

/// 画一个层的运动路径与手柄。
fn paint(painter: &mut Painter, app: &CanvasApp, st: &State, src: &Document, g: &Geo, hovered: Option<Part>, selected: Option<(NodeId, u32)>) {
    let node = src.node(g.node).expect("geo's node exists");
    let Some((t0, t1)) = node.motion.span_ms(st.duration_ms) else { return };
    let accent = app.accent();

    // ── 轨迹：静态父链 + 各时刻的 Position。拖播放头时这条线钉死不动。
    //
    // **轨迹只由空间信息决定**：段的两端取关键帧位置，段内的形状只由两端的**空间手柄**
    // 决定，采样参数是几何参数 u∈0..1，**不是时间**。缓动（EaseOutBack 的 y=1.45、
    // EaseInOutBack 的 y=-0.5/1.4、弹簧）只决定物体沿这条线走多快，改缓动不该改线的形状；
    // 以前按时间采样、把 `easing.ease(p)` 当贝塞尔参数，过冲的 p 会把线拉出关键帧之外
    // （用户 2026-11-14：「动画的路径线应该和时间/缓动曲线无关」）。
    let key_times: Vec<f64> =
        node.motion.track(AnimProperty::Position).map(|tr| tr.keys.iter().map(|k| f64::from(k.time_ms)).collect()).unwrap_or_default();
    if key_times.is_empty() {
        // 纯 clip：位移由 clip 的时间函数给出，只能按帧率采样（clip 自身的几何）。
        let step = (1000.0 / f64::from(st.fps.max(1))).max(1.0);
        let mut pts: Vec<Pos2> = Vec::new();
        let mut at = f64::from(t0);
        while at < f64::from(t1) {
            pts.extend(path_point(app, src, g.node, at));
            at += step;
        }
        pts.extend(path_point(app, src, g.node, f64::from(t1)));
        for w in pts.windows(2) {
            if w[0].distance(w[1]) > 0.5 {
                painter.line_segment([w[0], w[1]], s(accent, 1.0));
            }
        }
    } else {
        let mut times = key_times;
        times.push(f64::from(t0));
        times.push(f64::from(t1));
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        times.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        // 每段按几何参数均匀取 24 点：没手柄 = 直线；有手柄 = 沿三次贝塞尔（与物体实际
        // 走的空间曲线一致，用户 2026-11-12「改完手柄后对不上」）。
        const STEPS: usize = 24;
        for pair in times.windows(2) {
            let (ta, tb) = (pair[0], pair[1]);
            if tb - ta < 0.5 {
                continue;
            }
            let (Some(va), Some(vb)) = (position_value(src, g.node, ta), position_value(src, g.node, tb)) else { continue };
            let (Some(pa), Some(pb)) = (centre_at(app, src, g.node, va), centre_at(app, src, g.node, vb)) else { continue };
            let (ho, hi) = (handle_at(src, g.node, ta, End::Out), handle_at(src, g.node, tb, End::In));
            if ho.is_none() && hi.is_none() {
                if pa.distance(pb) > 0.01 {
                    painter.line_segment([pa, pb], s(accent, 1.0));
                }
                continue;
            }
            let (h1, h2) = (ho.unwrap_or([0.0, 0.0]), hi.unwrap_or([0.0, 0.0]));
            let (x0, y0, x3, y3) = (va.x, va.y, vb.x, vb.y);
            let (x1, y1) = (x0 + h1[0], y0 + h1[1]);
            let (x2, y2) = (x3 + h2[0], y3 + h2[1]);
            let mut prev = pa;
            for i in 1..=STEPS {
                let u = i as f64 / STEPS as f64;
                let t = 1.0 - u;
                let (w0, w1, w2, w3) = (t * t * t, 3.0 * t * t * u, 3.0 * t * u * u, u * u * u);
                let v = figcraft_geom::Vec2::new(
                    x0 * w0 + x1 * w1 + x2 * w2 + x3 * w3,
                    y0 * w0 + y1 * w1 + y2 * w2 + y3 * w3,
                );
                let Some(q) = centre_at(app, src, g.node, v) else { continue };
                painter.line_segment([prev, q], s(accent, 1.0));
                prev = q;
            }
        }
    }

    // 关键帧菱形：clip 预设没有离散帧，所以只有轨道上的帧有；播放头正落在帧上时画实心。
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6) as f32;
    for k in node.motion.track(AnimProperty::Position).map(|tr| tr.keys.as_slice()).unwrap_or_default() {
        let Some(c) = path_point(app, src, g.node, f64::from(k.time_ms)) else { continue };
        let on = (f64::from(k.time_ms) - st.playhead_ms).abs() < 0.5;
        // hover / 选中：放大 + 实心，告诉用户「这里可以拖」。
        let hot = matches!(hovered, Some(Part::Keyframe(t)) if t == k.time_ms);
        let sel = matches!(selected, Some((n, t)) if n == g.node && t == k.time_ms);
        let fill = if on || hot || sel { accent } else { Color32::WHITE };
        let r = if hot || sel { 7.5 } else { 6.0 };
        painter.add_shape(diamond(c, r), true, fill, s(accent, 1.5));
        // 选中的帧：画它的**空间手柄**（照矢量路径的贝塞尔手柄样式），可拖端点改曲线 / 拖回节点删。
        if sel {
            for (which, h) in [(End::Out, k.out_handle), (End::In, k.in_handle)] {
                if let Some(h) = h {
                    let end = pos2(c.x + h[0] as f32 * zoom, c.y + h[1] as f32 * zoom);
                    let hot = matches!(hovered, Some(Part::KeyframeHandle(_, e)) if e == which);
                    painter.line_segment([c, end], s(accent, 1.0));
                    let r = if hot { 4.5 } else { 3.5 };
                    painter.circle_filled(end, r, Color32::WHITE);
                    painter.circle_stroke(end, r, s(accent, 1.0));
                }
            }
        }
    }

    // ── 手柄：圆环 + 两根轴箭头 + 箭头根部的两个缩放块 + 中心菱形。
    let c = g.center;
    let sx = c + g.axis_x * RING_R;
    let sy = c + g.axis_y * RING_R;
    let ring_hot = hovered == Some(Part::Ring);
    // 白色是蓝色环的**描边**，宽度恒为 RING_OUTLINE_W；蓝色环 hover 变宽时，描边被整体
    // 往外/往内挤（半径随 blue_w 走），但描边自身宽度不变。不能像以前那样把白色 5pt
    // 圆环和蓝色圆环叠在同一半径上——那会让 hover 时露出的白边宽度随蓝环变宽而变窄。
    let blue_w = if ring_hot { RING_W_HOVER } else { RING_W };
    let outline_r = blue_w / 2.0 + RING_OUTLINE_W / 2.0;
    // ⛔ 不要填整圆（以前这里有个 `circle_filled(RING_R, 淡蓝 0.12)`）：它把圆内涂成半透明蓝，
    // 和画布背景对不上，看起来「中间有一圈颜色」（用户 2026-11-12）。环就是环，中间透空。
    painter.circle_stroke(c, RING_R - outline_r, s(Color32::WHITE, RING_OUTLINE_W));
    painter.circle_stroke(c, RING_R + outline_r, s(Color32::WHITE, RING_OUTLINE_W));
    painter.circle_stroke(c, RING_R, s(RING_BLUE, blue_w));

    let len_x = if hovered == Some(Part::ArrowX) { ARROW_LEN_HOVER } else { ARROW_LEN };
    let len_y = if hovered == Some(Part::ArrowY) { ARROW_LEN_HOVER } else { ARROW_LEN };
    arrow(painter, sx, c + g.axis_x * (RING_R + len_x), AXIS_X, 1.5);
    arrow(painter, sy, c + g.axis_y * (RING_R + len_y), AXIS_Y, 1.5);

    // 两个缩放块：蓝色圆角矩形（箭头正方向 = 放大），hover 时放大 + 白描边。
    paint_block(painter, sx, hovered == Some(Part::ScaleX));
    paint_block(painter, sy, hovered == Some(Part::ScaleY));

    painter.add_shape(diamond(c, 4.5), true, Color32::WHITE, s(RING_BLUE, 1.25));
}

/// 缩放块：箭头与圆相接处的**蓝色圆角矩形**操作手柄（用户 2026-11-12）。
///
/// 坐标 `round()` 到整数像素：以前用 `add_shape` 画四边形，抗锯齿让边缘发糊
/// （用户说"像素没对齐"）。圆角矩形走 `rect_filled` 的 `CornerRadius`，边缘清晰。
fn paint_block(p: &mut Painter, at: Pos2, hot: bool) {
    let half = if hot { BLOCK_R_HOVER } else { BLOCK_R };
    // 对齐整数像素（+0.5 让 1px 描边落在像素中心）。
    let cx = at.x.round() + 0.5;
    let cy = at.y.round() + 0.5;
    let rect = Rect::from_min_max(pos2(cx - half, cy - half), pos2(cx + half, cy + half));
    // 用户 2026-11-12：**白底 + 蓝色描边**（之前画反成蓝底白边）。hover 时描边加粗。
    let stroke_w = if hot { 2.0 } else { 1.0 };
    p.rect_filled(rect, CornerRadius::same(2), Color32::WHITE);
    p.rect_stroke(rect, CornerRadius::same(2), s(RING_BLUE, stroke_w), StrokeKind::Inside);
}

/// 画每个选中层的运动路径与手柄。
pub(super) fn draw(app: &CanvasApp, painter: &mut Painter, _t: &Tokens) {
    let st = &app.motion_overlay;
    if !st.active {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    // 空选：整块叠加层（轨迹 + 手柄）都不画（用户 2026-11-12）。
    if ds.selection.is_empty() {
        return;
    }
    // 轨迹钉死在静态文档坐标系：`ds.doc` 在渲染期已被临时换成 preview，用它的话
    // `parent_world` 会带上播放头求值后的变换，线条就跟着手柄一起动。
    let src = st.src.clone().unwrap_or_else(|| ds.doc.clone());
    let drag = &app.motion_gizmo.drag;
    let selected = app.motion_gizmo.selected_key;
    for g in geos(app) {
        // 拖动中锁定高亮为被拖部件（指针会离开部件），否则按当前指针现算 hover。
        let hovered = drag.as_ref().filter(|d| d.node == g.node).map(|d| d.part).or_else(|| app.pointer.and_then(|p| hit(&g, p)));
        paint(painter, app, st, &src, &g, hovered, selected);
    }
}

/// 把一次拖动的最新位置写回动画（播放头关键帧）。
fn apply(app: &mut CanvasApp, drag: &Drag, pos: Pos2) {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0).max(1e-6);
    let d = (pos - drag.start) / zoom as f32;
    // Ctrl + 拖路径关键帧 = 拉**对称**空间手柄（out = d，in = -d），前后两段一起弯。
    // 这是照矢量路径编辑的 Bend 行为做的（用户 2026-11-12）。
    if drag.bend
        && let Part::Keyframe(t) = drag.part
    {
        let (ox, oy) = (f64::from(d.x), f64::from(d.y));
        for (end, h) in [("out", [ox, oy]), ("in", [-ox, -oy])] {
            let _ = app.session.execute(
                "motion.keyframeHandle",
                &serde_json::json!({
                    "node": drag.node.0,
                    "property": AnimProperty::Position.name(),
                    "time": t,
                    "end": end,
                    "handle": h,
                }),
            );
        }
        return;
    }
    // 拖空间贝塞尔手柄端点。绘制端点在 `geos()` 里是 `c + handle*zoom`（`c` = 选中帧在屏幕上的
    // 中心，由 `path_point` 算出），所以**写入也必须以同一个 `c` 为原点**，否则端点会「跳一下」：
    // 屏幕位移 `(pos - c)` 除以 `zoom` 即得与绘制同基准的页面偏移。拖回中心（页面距离 < 4）→
    // 删除（null，曲线回到直线），照矢量路径「手柄拖回锚点即归直」的交互（用户 2026-11-12）。
    if let Part::KeyframeHandle(t, end) = drag.part {
        let src = app.session.ds().ok().map(|d| d.doc.clone());
        let c_screen = src.as_ref().and_then(|s| path_point(app, s, drag.node, f64::from(t))).unwrap_or(drag.center);
        let dx = pos.x - c_screen.x;
        let dy = pos.y - c_screen.y;
        let hx = dx as f64 / zoom as f64;
        let hy = dy as f64 / zoom as f64;
        let dist = ((dx * dx + dy * dy) as f64).sqrt() / zoom as f64;
        let handle: Option<[f64; 2]> = if dist < 4.0 { None } else { Some([hx, hy]) };
        let end_str = match end {
            End::Out => "out",
            End::In => "in",
        };
        let _ = app.session.execute(
            "motion.keyframeHandle",
            &serde_json::json!({
                "node": drag.node.0,
                "property": AnimProperty::Position.name(),
                "time": t,
                "end": end_str,
                "handle": handle,
            }),
        );
        return;
    }
    let (prop, value): (AnimProperty, AnimValue) = match drag.part {
        Part::Ring => {
            // 屏幕角差（顺时针为正）→ 动画增量（逆时针为正）取负。
            // 引擎 `eval.rs` 的 Rotation 轨道存的是**绝对角**（它自己算
            // `r - base_rotation_deg(base)` 得到增量），所以必须写回 `base_rot + delta`，
            // 否则画面会平白多转一个设计角。gizmo 的"0 基准"体现在 `Geo::delta_deg`
            // （显示/箭头朝向 = 动画绝对角 − 设计角），不在关键帧数值上。
            let now = f64::from((pos - drag.center).angle());
            let delta = drag.start_delta - (now - drag.start_angle).to_degrees();
            (AnimProperty::Rotation, AnimValue::Scalar(drag.base_rot + delta))
        }
        Part::Center | Part::Keyframe(_) => {
            // 中心菱形 / 路径关键帧点：自由 2D 位移 → Position（不沿轴）。
            // 中心菱形写在播放头；关键帧点写回**它自己的时间**（见下面的 `time`）。
            // `d` 是页面空间位移；乘父级线性变换之逆才得到局部（父坐标系）位移，否则父级
            // 旋转/缩放时拖拽方向会偏（90° 父级下水平拖变竖直，CodeRabbit）。父级无变换时
            // `parent_inv` 为单位阵 → 与旧行为完全一致。
            let [ia, ib, ic, idd] = drag.parent_inv;
            let dx = f64::from(d.x);
            let dy = f64::from(d.y);
            let local = figcraft_geom::Vec2::new(ia * dx + ic * dy, ib * dx + idd * dy);
            let p = drag.start_pos + local;
            (AnimProperty::Position, AnimValue::Vec2(p))
        }
        Part::ArrowX | Part::ArrowY => {
            let axis = if drag.part == Part::ArrowX { drag.axis_x } else { drag.axis_y };
            let along = f64::from(d.x * axis.x + d.y * axis.y);
            let p = drag.start_pos + figcraft_geom::Vec2::new(f64::from(axis.x) * along, f64::from(axis.y) * along);
            (AnimProperty::Position, AnimValue::Vec2(p))
        }
        Part::ScaleX | Part::ScaleY => {
            let axis = if drag.part == Part::ScaleX { drag.axis_x } else { drag.axis_y };
            let along = f64::from(d.x * axis.x + d.y * axis.y) * SCALE_PER_PX;
            let v = if drag.part == Part::ScaleX { drag.start_scale.x } else { drag.start_scale.y };
            let scaled = (v + along).max(0.05);
            let n = if drag.part == Part::ScaleX {
                figcraft_geom::Vec2::new(scaled, drag.start_scale.y)
            } else {
                figcraft_geom::Vec2::new(drag.start_scale.x, scaled)
            };
            (AnimProperty::Scale, AnimValue::Vec2(n))
        }
        // 手柄端点已在上方分支 return；这里兜底，理论不可达。
        Part::KeyframeHandle(_, _) => return,
    };
    let Ok(value) = serde_json::to_value(value) else { return };
    // 关键帧点写回**它自己的时间**；其余部件写在播放头。
    let time = match drag.part {
        Part::Keyframe(t) => t,
        _ => drag.time,
    };
    // gizmo 是「在播放头打动画帧」的工具：即便该属性还没有轨道也要建一条（`motion.keyframe`
    // 会 ensure_track + set_key）。不能用 `motion.autoKey` —— 它对无轨道属性只改静态 transform，
    // 那会让「拖一下旋转环/缩放块」看不到任何动画轨道，与手柄语义相悖。
    // undo 边界由 `input` 在按下/抬起时的 begin_interaction / commit_interaction 统一拥有。
    let _ =
        app.session.execute("motion.keyframe", &serde_json::json!({ "ids": [drag.node.0], "property": prop.name(), "time": time, "value": value }));
}

/// 指针处理；返回 `true` 表示这一下归 gizmo（画布不要再当选择/拖拽）。
///
/// 与 `canvas::handles::input` 同款三段式：`pressed` / `down` / `released` 由 GPUI 的三个
/// 回调分别传入；窗口级拖拽由 `route_canvas_drag` / `on_window_drag_up` 接管。
pub(crate) fn input(app: &mut CanvasApp, pos: Pos2, pressed: bool, down: bool, released: bool, ctrl: bool) -> bool {
    let st = app.motion_overlay.clone();
    if !st.active || st.gizmo_hidden {
        return false;
    }
    let Ok(ds) = app.session.ds() else { return false };
    if ds.selection.is_empty() {
        app.motion_gizmo.drag = None;
        return false;
    }

    // ① 一次拖动进行中：推进 / 收尾。
    if let Some(drag) = app.motion_gizmo.drag.clone() {
        if down {
            apply(app, &drag, pos);
        }
        // ⚠️ 只有明确的 `released` 才收尾。窗口级 move 常带 `down=false`，用 `!down` 收尾会
        // 把刚起的拖当场清掉 —— 表现就是「hover 有高亮、但按下去拖不动」（用户 2026-11-12）。
        if released {
            let _ = app.session.commit_interaction();
            app.motion_gizmo.drag = None;
        }
        return true;
    }

    // ② 命中：悬停反馈 + 光标；按下则起一次拖动。
    let time = st.playhead_ms.max(0.0) as u32;
    for g in geos(app) {
        let Some(part) = hit(&g, pos) else { continue };
        app.spacing_cursor = Some(cursor_for(part, &g));
        if !pressed {
            return false; // 悬停：设光标，但不吞掉画布的 hover。
        }
        // 点中路径关键帧 → 记为选中（持久），选中后画它的手柄；Ctrl 拖则拉手柄。
        if let Part::Keyframe(t) = part {
            app.motion_gizmo.selected_key = Some((g.node, t));
        }
        let Some(node) = ds.doc.node(g.node) else { return false };
        // ⚠️ 必须用 `keyframed_value_at`（**只算轨道、不含 clip**），不能用 `value_at`：
        // `value_at` 含 clip 位移，而 `motion.keyframe` 写的是 track 值，引擎求值时又会把
        // clip 叠加一次 → 写入值多带一份 clip 偏移，表现为「一按箭头先往反方向跳一下，
        // 之后才在错误基准上正常拖动」（用户 2026-11-12）。
        let start_pos = match part {
            // 拖路径关键帧 / 它的手柄端点：基准都是该帧自己的值，不是播放头的插值。
            Part::Keyframe(t) | Part::KeyframeHandle(t, _) => node
                .motion
                .track(AnimProperty::Position)
                .and_then(|tr| tr.key_at(t))
                .and_then(|k| k.value.vec2())
                .unwrap_or_else(|| motion::base_position(node)),
            _ => motion::keyframed_value_at(node, AnimProperty::Position, st.playhead_ms)
                .and_then(|v| v.vec2())
                .unwrap_or_else(|| motion::base_position(node)),
        };
        let start_scale = motion::keyframed_value_at(node, AnimProperty::Scale, st.playhead_ms)
            .and_then(|v| v.vec2())
            .unwrap_or(figcraft_geom::Vec2::new(1.0, 1.0));
        // 以下三项都读 `node` / `ds`（借用 session），必须在 `begin_interaction`（可变借用）
        // 之前算好，否则与 `ds` 的不可变借用冲突。
        // 旋转增量用**关键帧**值（不含 clip）：clip 求值时另加，存进关键帧会叠加两次（CodeRabbit）。
        let start_delta = motion::keyframed_value_at(node, AnimProperty::Rotation, st.playhead_ms)
            .and_then(|v| v.scalar())
            .map(|r| r - g.base_rot)
            .unwrap_or(0.0);
        // 父级线性变换之逆：把「页面空间」位移转成节点局部（父坐标系）位移。
        let src = st.src.clone().unwrap_or_else(|| ds.doc.clone());
        let pc = parent_world(&src, g.node).inverse().as_coeffs();
        if app.session.begin_interaction("Transform").is_err() {
            return false;
        }
        app.motion_gizmo.drag = Some(Drag {
            part,
            node: g.node,
            start: pos,
            center: g.center,
            axis_x: g.axis_x,
            axis_y: g.axis_y,
            base_rot: g.base_rot,
            start_delta,
            parent_inv: [pc[0], pc[1], pc[2], pc[3]],
            time,
            start_pos,
            start_scale,
            start_angle: f64::from((pos - g.center).angle()),
            // Ctrl + 拖路径关键帧 = 拉空间手柄（对称），其余部件不受影响。
            bend: ctrl && matches!(part, Part::Keyframe(_)),
        });
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::painter::Op;
    use figcraft_doc::anim::motion::Ease;
    use figcraft_doc::{Document, NodeKind};

    /// `Card(0,0,200,200)` 里一个 `Dot(10,10,40,40)`，Position 0ms (10,10) → 1000ms (110,60)。
    fn app_with_motion() -> (CanvasApp, NodeId) {
        let mut doc = Document::new("motion-overlay");
        let page = doc.first_page().expect("page");
        let card = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 200.0, 200.0).expect("card");
        let dot = doc.create(card, NodeKind::Rectangle, "Dot", 10.0, 10.0, 40.0, 40.0).expect("dot");
        if let Some(n) = doc.node_mut(dot) {
            let tr = n.motion.ensure_track(AnimProperty::Position);
            tr.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(10.0, 10.0)), Some(Ease::Linear));
            tr.set_key(1000, AnimValue::Vec2(figcraft_geom::Vec2::new(110.0, 60.0)), Some(Ease::Linear));
        }
        let mut app = CanvasApp::new(doc);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        if let Ok(ds) = app.session.ds_mut() {
            ds.view.vw = 800.0;
            ds.view.vh = 600.0;
            ds.view.zoom = 1.0;
        }
        app.session.set_selection(vec![dot]);
        (app, dot)
    }

    fn preview_at(app: &CanvasApp, t_ms: f64) -> Arc<Document> {
        let ds = app.session.ds().expect("ds");
        Arc::new(motion::evaluate(&ds.doc, ds.page, t_ms))
    }

    fn on(app: &mut CanvasApp, t: f64) {
        let src = app.session.ds().expect("ds").doc.clone();
        app.motion_overlay = State::new(true, false, t, 60, 2000, Some(preview_at(app, t)), Some(src));
    }

    #[test]
    fn empty_selection_draws_nothing() {
        let (mut app, _dot) = app_with_motion();
        app.session.set_selection(Vec::new());
        on(&mut app, 0.0);
        let mut painter = Painter::new(app.canvas.rect);
        let theme = app.theme;
        draw(&app, &mut painter, &theme);
        assert!(painter.ops.is_empty(), "空选不画轨迹/手柄：{} 条指令", painter.ops.len());
        assert!(geos(&app).is_empty());
    }

    #[test]
    fn playing_hides_the_overlay() {
        assert!(State::new(true, false, 0.0, 60, 2000, None, None).active, "Motion 打开、没在播放 → 画");
        assert!(!State::new(true, true, 0.0, 60, 2000, None, None).active, "播放中 → 藏起来");
        assert!(!State::new(false, false, 0.0, 60, 2000, None, None).active, "不在 Motion 模式 → 不画");
    }

    #[test]
    fn path_is_static_across_playheads() {
        // 轨迹钉死：换播放头、换 preview，采样点不变（父链/尺寸取未评估文档）。
        let (mut app, dot) = app_with_motion();
        on(&mut app, 0.0);
        let src = app.session.ds().unwrap().doc.clone();
        let a0 = path_point(&app, &src, dot, 0.0).expect("t0");
        let b0 = path_point(&app, &src, dot, 1000.0).expect("t1");
        on(&mut app, 1000.0);
        let src2 = app.session.ds().unwrap().doc.clone();
        let a1 = path_point(&app, &src2, dot, 0.0).expect("t0 again");
        let b1 = path_point(&app, &src2, dot, 1000.0).expect("t1 again");
        assert_eq!(a0, a1, "播放头变化不该移动轨迹起点");
        assert_eq!(b0, b1, "播放头变化不该移动轨迹终点");
        assert!((b0.x - a0.x).abs() > 1.0, "Position 关键帧确实画出了一条线");
    }

    /// 轨迹线**只由空间信息决定**：把同一段的缓动换成会过冲的 back / 弹簧，画出来的点
    /// 必须逐点相同（用户 2026-11-14：「动画的路径线应该和时间/缓动曲线无关」）。
    ///
    /// 回归锁：以前按时间采样、把 `easing.ease(p)` 当贝塞尔参数，过冲的 p（EaseOutBack 的
    /// y=1.45、EaseInOutBack 的 y=-0.5/1.4、弹簧振荡）会把线拉出关键帧之外。
    #[test]
    fn path_shape_ignores_the_easing_curve() {
        /// 画一遍，取轨迹线（accent 色、1pt 的 `Line`）的所有端点；顺带给出两端的理论位置。
        fn drawn(ease: Ease) -> (Vec<Pos2>, Pos2, Pos2) {
            let (mut app, dot) = app_with_motion();
            {
                let ds = app.session.ds_mut().expect("ds");
                let tr = Arc::make_mut(&mut ds.doc).node_mut(dot).unwrap().motion.ensure_track(AnimProperty::Position);
                tr.keys.iter_mut().find(|k| k.time_ms == 0).unwrap().easing = ease;
            }
            on(&mut app, 500.0);
            let src = app.session.ds().expect("ds").doc.clone();
            let a = path_point(&app, &src, dot, 0.0).expect("起点");
            let b = path_point(&app, &src, dot, 1000.0).expect("终点");
            app.pointer = Some(pos2(-100.0, -100.0)); // 别 hover 出手柄线
            let mut painter = Painter::new(app.canvas.rect);
            let theme = app.theme;
            draw(&app, &mut painter, &theme);
            let accent = app.accent();
            let mut pts = Vec::new();
            for o in &painter.ops {
                if let Op::Line { a, b, stroke } = o
                    && stroke.color == accent
                    && (stroke.width - 1.0).abs() < 1e-6
                {
                    pts.push(*a);
                    pts.push(*b);
                }
            }
            (pts, a, b)
        }

        let bbox = |pts: &[Pos2]| {
            pts.iter().fold((f32::MAX, f32::MIN, f32::MAX, f32::MIN), |(x0, x1, y0, y1), p| {
                (x0.min(p.x), x1.max(p.x), y0.min(p.y), y1.max(p.y))
            })
        };

        let (linear, a, b) = drawn(Ease::Linear);
        assert!(!linear.is_empty(), "轨迹要画出来，否则这条测试没有约束力");
        for ease in [Ease::EaseInOutBack, Ease::EaseOutBack, Ease::SpringBouncy, Ease::EaseIn] {
            let (pts, _, _) = drawn(ease);
            assert_eq!(pts, linear, "{ease:?} 不该改变轨迹的点（形状/长度都与缓动无关）");
        }
        // 没有空间手柄 → 直线，且恰好止于两个关键帧，不外溢。
        let (lx0, lx1, ly0, ly1) = bbox(&linear);
        let (ex0, ex1, ey0, ey1) = bbox(&[a, b]);
        for (got, want, axis) in [(lx0, ex0, "x0"), (lx1, ex1, "x1"), (ly0, ey0, "y0"), (ly1, ey1, "y1")] {
            assert!((got - want).abs() < 0.01, "轨迹 {axis} 应与关键帧端点一致：{got} vs {want}");
        }
    }

    /// 有空间手柄时轨迹仍然与缓动无关（贝塞尔形状来自手柄，采样参数是几何参数）。
    #[test]
    fn curved_path_shape_ignores_the_easing_curve() {
        fn drawn(ease: Ease) -> Vec<Pos2> {
            let (mut app, dot) = app_with_motion();
            {
                let ds = app.session.ds_mut().expect("ds");
                let tr = Arc::make_mut(&mut ds.doc).node_mut(dot).unwrap().motion.ensure_track(AnimProperty::Position);
                {
                    let k = tr.keys.iter_mut().find(|k| k.time_ms == 0).unwrap();
                    k.easing = ease;
                    // 只给 out 端一个手柄：段内曲线明显偏离弦（对称 S 形会在中点落回弦上，
                    // 测不出「手柄有没有生效」）。
                    k.out_handle = Some([40.0, -80.0]);
                }
            }
            on(&mut app, 500.0);
            app.pointer = Some(pos2(-100.0, -100.0));
            let mut painter = Painter::new(app.canvas.rect);
            let theme = app.theme;
            draw(&app, &mut painter, &theme);
            let accent = app.accent();
            let mut pts = Vec::new();
            for o in &painter.ops {
                if let Op::Line { a, b, stroke } = o
                    && stroke.color == accent
                    && (stroke.width - 1.0).abs() < 1e-6
                {
                    pts.push(*a);
                    pts.push(*b);
                }
            }
            pts
        }

        let linear = drawn(Ease::Linear);
        assert!(linear.len() > 4, "有手柄时该按贝塞尔采样出多个点：{}", linear.len());
        for ease in [Ease::EaseInOutBack, Ease::SpringBouncy] {
            assert_eq!(drawn(ease), linear, "{ease:?} 不该改变曲线形状");
        }
        // 曲线：点集必须明显偏离两端连线（否则手柄没生效，前两条等式断言会失去意义）。
        let (a, b) = (linear[0], *linear.last().unwrap());
        let chord = b - a;
        let dev = linear
            .iter()
            .map(|p| (*p - a).dot(vec2(-chord.y, chord.x)).abs() / a.distance(b).max(1e-6))
            .fold(0.0f32, f32::max);
        assert!(dev > 5.0, "手柄应把轨迹拉弯，实际最大偏离弦 {dev}px");
    }

    #[test]
    fn design_rotation_does_not_rotate_the_arrows() {
        let (mut app, dot) = app_with_motion();
        {
            let ds = app.session.ds_mut().expect("ds");
            if let Some(n) = Arc::make_mut(&mut ds.doc).node_mut(dot) {
                n.transform = Affine::rotate((-45.0_f64).to_radians()); // Figma +45°(逆时针)
            }
        }
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        assert!(g.delta_deg.abs() < 1e-6, "没有动画旋转 → 增量为 0：{}", g.delta_deg);
        assert!((g.axis_x - vec2(0.0, -1.0)).length() < 1e-6, "绿箭头仍向上：{:?}", g.axis_x);
        assert!((g.axis_y - vec2(1.0, 0.0)).length() < 1e-6, "粉箭头仍向右：{:?}", g.axis_y);
    }

    /// 动画里旋转 45°（相对设计角）→ 两根轴跟着转 45°。
    #[test]
    fn animation_rotation_rotates_the_arrows() {
        let (mut app, dot) = app_with_motion();
        {
            let ds = app.session.ds_mut().expect("ds");
            if let Some(n) = Arc::make_mut(&mut ds.doc).node_mut(dot) {
                n.transform = Affine::rotate((-45.0_f64).to_radians()); // 设计 +45°
                let tr = n.motion.ensure_track(AnimProperty::Rotation);
                // value_at = 绝对角；设计 45 + 动画增量 45 = 90。
                tr.set_key(0, AnimValue::Scalar(90.0), Some(Ease::Linear));
            }
        }
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        assert!((g.delta_deg - 45.0).abs() < 1e-6, "动画增量 45°：{}", g.delta_deg);
        // rot((-1,0), -45°) = (-√2/2, √2/2)：向左基准转 45° 逆时针 → 指向左下。
        // 绿箭头基准向上 (0,-1)，再逆时针转 45° → 指向左上。
        assert!(g.axis_x.x < -0.6 && g.axis_x.y < -0.6, "绿箭头跟转：{:?}", g.axis_x);
    }

    #[test]
    fn hit_tests_all_six_parts() {
        let (mut app, _dot) = app_with_motion();
        // 停在两帧之间：否则中心菱形正好压在 0ms 关键帧上，会先命中 `Keyframe(0)`。
        on(&mut app, 500.0);
        let g = geos(&app).pop().expect("one gizmo");
        assert_eq!(hit(&g, g.center), Some(Part::Center), "中心菱形改动画位置");
        assert_eq!(hit(&g, g.center + g.axis_x * (RING_R + ARROW_LEN_HOVER)), Some(Part::ArrowX));
        assert_eq!(hit(&g, g.center + g.axis_y * (RING_R + ARROW_LEN_HOVER)), Some(Part::ArrowY));
        assert_eq!(hit(&g, g.center + g.axis_x * RING_R), Some(Part::ScaleX));
        assert_eq!(hit(&g, g.center + g.axis_y * RING_R), Some(Part::ScaleY));
        // 正下方：避开「绿轴向上、粉轴向右」，才落在环上而不是块上。
        assert_eq!(hit(&g, g.center + vec2(0.0, RING_R)), Some(Part::Ring));
        assert_eq!(hit(&g, g.center + vec2(120.0, 120.0)), None);
        // 菱形外、环外的正常拖动仍归画布（改整体位置）。
        assert_eq!(hit(&g, g.center + vec2(16.0, 0.0)), None);
    }

    /// 没有空间手柄的 Position 轨道，`value_at` 必须严格在直线上（不能有贝塞尔式的弯曲）。
    /// 这是"线是弯的"最底层的判据：如果这条都过不了，问题在引擎；过了就说明在绘制。
    #[test]
    fn a_position_track_without_handles_is_a_straight_line() {
        use figcraft_doc::anim::motion::{Ease, Track};
        let mut tr = Track::new(AnimProperty::Position);
        tr.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(0.0, 0.0)), Some(Ease::Linear));
        tr.set_key(1000, AnimValue::Vec2(figcraft_geom::Vec2::new(100.0, 100.0)), Some(Ease::Linear));
        for (t, want) in [(250.0, 25.0), (500.0, 50.0), (750.0, 75.0)] {
            let Some(AnimValue::Vec2(m)) = tr.value_at(t) else { panic!("track has keys") };
            assert!((m.x - want).abs() < 1e-6 && (m.y - want).abs() < 1e-6, "t={t} 应在 ({want},{want})：{m:?}");
        }
    }

    /// 手柄是 `Some([0,0])`（Ctrl 只点了一下）时也必须严格直线——照矢量路径 `is_straight`。
    #[test]
    fn a_zero_handle_is_still_a_straight_line() {
        use figcraft_doc::anim::motion::{Ease, Track};
        let mut tr = Track::new(AnimProperty::Position);
        tr.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(0.0, 0.0)), Some(Ease::Linear));
        tr.set_key(1000, AnimValue::Vec2(figcraft_geom::Vec2::new(100.0, 100.0)), Some(Ease::Linear));
        tr.keys[0].out_handle = Some([0.0, 0.0]);
        tr.keys[1].in_handle = Some([0.0, 0.0]);
        for (t, want) in [(250.0, 25.0), (500.0, 50.0), (750.0, 75.0)] {
            let Some(AnimValue::Vec2(m)) = tr.value_at(t) else { panic!("track has keys") };
            assert!((m.x - want).abs() < 1e-6 && (m.y - want).abs() < 1e-6, "t={t} 零手柄应在 ({want},{want})：{m:?}");
        }
    }

    /// 拖**运动路径上的关键帧点** = 改该时刻的空间 Position（用户 2026-11-12）。
    #[test]
    fn dragging_a_path_keyframe_moves_that_moment() {
        let (mut app, dot) = app_with_motion();
        on(&mut app, 500.0);
        let g = geos(&app).pop().expect("one gizmo");
        let (t0, p0) = *g.keys.first().expect("0ms 关键帧");
        assert_eq!(t0, 0);
        assert!(input(&mut app, p0, true, true, false, false), "关键帧点接管起拖");
        assert_eq!(app.motion_gizmo.drag.as_ref().unwrap().part, Part::Keyframe(0));
        let to = p0 + vec2(20.0, 10.0);
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        let v = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Position))
            .and_then(|tr| tr.key_at(0))
            .map(|k| k.value);
        let Some(AnimValue::Vec2(p)) = v else { panic!("0ms 该有 Position 关键帧") };
        // 0ms 原值 (10,10)，屏幕拖 (20,10)（zoom=1）→ (30,20)。
        assert!((p.x - 30.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3, "关键帧空间位置：{p:?}");
    }

    /// 选中关键帧后，它的空间贝塞尔手柄端点必须「可点中、可拖出改曲线、可拖回中心删除（归直）」。
    /// Bug 1 / Bug 2 的回归锁：之前手柄画得出却点不中 → 既不能弯也不能删（用户 2026-11-12）。
    #[test]
    fn keyframe_handle_is_hittable_draggable_and_deletable() {
        let (mut app, dot) = app_with_motion();
        // 选中 0ms 关键帧并给它一个 out 手柄。
        on(&mut app, 0.0);
        app.motion_gizmo.selected_key = Some((dot, 0));
        {
            let ds = app.session.ds_mut().expect("ds");
            let tr = Arc::make_mut(&mut ds.doc).node_mut(dot).unwrap().motion.ensure_track(AnimProperty::Position);
            tr.keys.iter_mut().find(|k| k.time_ms == 0).unwrap().out_handle = Some([30.0, 0.0]);
        }
        on(&mut app, 0.0); // 重新取 src/preview，让 geos 带上手柄端点
        let g = geos(&app).pop().expect("one gizmo");
        let (ht, hend, hp) = g.handle_ends.iter().find(|(t, e, _)| *t == 0 && *e == End::Out).expect("out 手柄端点应被画出");
        assert_eq!(*ht, 0);
        assert_eq!(*hend, End::Out);
        // ① 点中手柄端点
        assert_eq!(hit(&g, *hp), Some(Part::KeyframeHandle(0, End::Out)), "手柄端点必须可点中");
        // ② 拖回关键帧中心 → 删除（归直）
        assert!(input(&mut app, *hp, true, true, false, false), "手柄端点接管起拖");
        assert_eq!(app.motion_gizmo.drag.as_ref().unwrap().part, Part::KeyframeHandle(0, End::Out));
        let center = g.center; // 关键帧在屏幕上的中心
        input(&mut app, center, false, true, false, false);
        input(&mut app, center, false, false, true, false);
        let handle = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Position))
            .and_then(|tr| tr.key_at(0))
            .and_then(|k| k.out_handle);
        assert!(handle.is_none(), "拖回中心应删除 out 手柄（曲线归直）：{:?}", handle);
        // ③ 重新给手柄，拖离中心 → 写回与绘制同基准的相对偏移
        {
            let ds = app.session.ds_mut().expect("ds");
            let tr = Arc::make_mut(&mut ds.doc).node_mut(dot).unwrap().motion.ensure_track(AnimProperty::Position);
            tr.keys.iter_mut().find(|k| k.time_ms == 0).unwrap().out_handle = Some([30.0, 0.0]);
        }
        app.motion_gizmo.selected_key = Some((dot, 0));
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        let hp = g.handle_ends.iter().find(|(t, e, _)| *t == 0 && *e == End::Out).expect("out 手柄端点").2;
        assert!(input(&mut app, hp, true, true, false, false));
        // 拖到中心右方 40px（zoom=1 → 页面偏移也是 40）
        let to = g.center + vec2(40.0, 0.0);
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        let handle = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Position))
            .and_then(|tr| tr.key_at(0))
            .and_then(|k| k.out_handle);
        let Some(h) = handle else { panic!("拖离中心应写回 out 手柄") };
        assert!((h[0] - 40.0).abs() < 1.0 && (h[1] - 0.0).abs() < 1.0, "手柄跟随指针偏移（与绘制同基准）：{:?}", h);
    }

    /// 中心菱形拖动 = 自由 2D 位移写进动画 Position；`start_pos` 只取轨道值（不含 clip），
    /// 所以第一次写入不会「跳」。
    #[test]
    fn dragging_the_center_diamond_keys_animation_position() {
        let (mut app, dot) = app_with_motion();
        on(&mut app, 500.0);
        let g = geos(&app).pop().expect("one gizmo");
        assert!(input(&mut app, g.center, true, true, false, false), "菱形接管起拖");
        assert_eq!(app.motion_gizmo.drag.as_ref().unwrap().part, Part::Center);
        let to = g.center + vec2(24.0, -12.0);
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        let v = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Position))
            .and_then(|tr| tr.key_at(500))
            .map(|k| k.value);
        let Some(AnimValue::Vec2(p)) = v else { panic!("播放头该有 Position 关键帧") };
        // 500ms 轨道插值 (60,35)，自由 2D 拖 24,-12 → (84,23)。
        assert!((p.x - 84.0).abs() < 1e-3 && (p.y - 23.0).abs() < 1e-3, "自由 2D 位移：{p:?}");
    }

    #[test]
    fn drag_arrow_x_keys_position_at_playhead() {
        let (mut app, dot) = app_with_motion();
        on(&mut app, 500.0);
        let g = geos(&app).pop().expect("one gizmo");
        // 点在箭头中段，避开根部缩放块的容差（块命中优先于箭头）。
        let press = g.center + g.axis_x * (RING_R + ARROW_LEN - 4.0);
        assert!(input(&mut app, press, true, true, false, false), "按下绿箭头 → gizmo 接管");
        let drag = app.motion_gizmo.drag.clone().expect("drag started");
        assert_eq!(drag.part, Part::ArrowX);
        // 沿绿箭头（向左）拖 30px → Position.x 减 30（zoom=1）。
        let to = press + g.axis_x * 30.0;
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        assert!(app.motion_gizmo.drag.is_none(), "抬起收尾");
        let v = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Position))
            .and_then(|tr| tr.key_at(500))
            .map(|k| k.value);
        let Some(AnimValue::Vec2(p)) = v else { panic!("播放头该有 Position 关键帧") };
        // 500ms 的插值起点 x=60/y=35；绿箭头朝上，沿它拖 30px → y 减 30，X 轴不动。
        assert!((p.x - 60.0).abs() < 1e-3, "X 轴不动：{p:?}");
        assert!((p.y - 5.0).abs() < 1e-3, "起点 y=35，向上拖 30 → 5：{p:?}");
    }

    #[test]
    fn drag_scale_block_keys_scale_positive_is_bigger() {
        let (mut app, dot) = app_with_motion();
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        let press = g.center + g.axis_x * RING_R;
        assert!(input(&mut app, press, true, true, false, false));
        assert_eq!(app.motion_gizmo.drag.as_ref().unwrap().part, Part::ScaleX);
        // 沿绿箭头正方向拖 50px → Scale.x = 1 + 50/100 = 1.5（放大）。
        let to = press + g.axis_x * 50.0;
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        let v = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Scale))
            .and_then(|tr| tr.key_at(0))
            .map(|k| k.value);
        let Some(AnimValue::Vec2(sc)) = v else { panic!("该有 Scale 关键帧") };
        assert!((sc.x - 1.5).abs() < 1e-3, "正方向 = 放大：{sc:?}");
        assert!((sc.y - 1.0).abs() < 1e-3, "另一轴不动：{sc:?}");
    }

    #[test]
    fn drag_ring_keys_rotation_delta() {
        let (mut app, dot) = app_with_motion();
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        // 从圆环**正下方**按下（避开向上/向右两根轴），顺时针拖 90° → 动画增量 -90°。
        let press = g.center + vec2(0.0, RING_R);
        assert!(input(&mut app, press, true, true, false, false));
        assert_eq!(app.motion_gizmo.drag.as_ref().unwrap().part, Part::Ring);
        let to = g.center + vec2(-RING_R, 0.0); // 下方 → 左方 = 屏幕顺时针 90°
        input(&mut app, to, false, true, false, false);
        input(&mut app, to, false, false, true, false);
        let v = app
            .session
            .doc()
            .ok()
            .and_then(|d| d.node(dot))
            .and_then(|n| n.motion.track(AnimProperty::Rotation))
            .and_then(|tr| tr.key_at(0))
            .map(|k| k.value);
        let Some(AnimValue::Scalar(r)) = v else { panic!("该有 Rotation 关键帧") };
        assert!((r + 90.0).abs() < 1e-2, "顺时针拖 → 动画增量 -90°（绝对角写 -90）：{r}");
    }

    #[test]
    fn draw_paints_the_path_keyframe_diamonds_and_the_gizmo() {
        let (mut app, _dot) = app_with_motion();
        on(&mut app, 1000.0);
        app.pointer = Some(pos2(-100.0, -100.0)); // 别 hover 到部件上
        let mut painter = Painter::new(app.canvas.rect);
        let theme = app.theme;
        draw(&app, &mut painter, &theme);

        let quad = |o: &Op| matches!(o, Op::Poly { points, .. } if points.len() == 4);
        // 两个关键帧菱形 + 手柄中心菱形 + 两个缩放块 = 5 个四边形。
        // 两个关键帧菱形 + 手柄中心菱形 = 3 个四边形（缩放块已改成圆角矩形，走 Op::Rect）。
        assert_eq!(painter.ops.iter().filter(|o| quad(o)).count(), 3);
        let ring = painter.ops.iter().filter(|o| matches!(o, Op::Circle { radius, .. } if *radius == RING_R)).count();
        // 半径恰为 RING_R 的是淡蓝填充 + 蓝色环；两条白描边在 RING_R ± outline_r。
        assert_eq!(ring, 1, "半径 RING_R 的只有蓝环一条（整圆淡蓝填充已删，用户 2026-11-12）");
        let accent = app.accent();
        let filled = painter.ops.iter().filter(|o| matches!(o, Op::Poly { points, fill, .. } if points.len() == 4 && *fill == accent)).count();
        assert_eq!(filled, 1, "只有播放头处那颗关键帧画实心");
    }

    /// hover 圆环：蓝色环从 RING_W(1.5) 变粗到 RING_W_HOVER(3.0)，白色**描边**
    /// 宽度恒为 RING_OUTLINE_W 不变、只是被变宽的蓝环挤到更外侧（用户 2026-11-12）。
    /// 叠两个同心圆环、或让 hover 改变白边宽度，都是错的。
    #[test]
    fn hovering_the_ring_thickens_only_the_blue_band() {
        let (mut app, _dot) = app_with_motion();
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        let strokes = |a: &CanvasApp| -> (Vec<(f32, f32)>, Vec<f32>) {
            let mut p = Painter::new(a.canvas.rect);
            let t = a.theme;
            draw(a, &mut p, &t);
            let mut white = Vec::new();
            let mut blue = Vec::new();
            for o in &p.ops {
                if let Op::Circle { radius, stroke, .. } = o
                    && stroke.width > 0.0
                {
                    if stroke.color == Color32::WHITE {
                        white.push((*radius, stroke.width));
                    } else if stroke.color == RING_BLUE {
                        blue.push(stroke.width);
                    }
                }
            }
            (white, blue)
        };
        app.pointer = Some(g.center + vec2(0.0, RING_R)); // 环正下方（避开上/右两轴）
        let (white_hot, blue_hot) = strokes(&app);
        app.pointer = Some(pos2(-100.0, -100.0));
        let (white_cold, blue_cold) = strokes(&app);

        assert_eq!(white_cold.len(), 2, "内外两条白描边：{white_cold:?}");
        assert!(white_cold.iter().all(|(_, w)| *w == RING_OUTLINE_W), "常态白描边宽度：{white_cold:?}");
        assert!(white_hot.iter().all(|(_, w)| *w == RING_OUTLINE_W), "hover 时白描边宽度**不变**：{white_hot:?}");
        assert_eq!(blue_cold, vec![RING_W], "常态蓝环细");
        assert_eq!(blue_hot, vec![RING_W_HOVER], "hover 蓝环变宽");

        // 蓝环变宽 → 外侧白描边半径变大、内侧变小，即"被挤出去"。
        let outer = |w: &Vec<(f32, f32)>| w.iter().map(|(r, _)| *r).fold(0.0f32, f32::max);
        let inner = |w: &Vec<(f32, f32)>| w.iter().map(|(r, _)| *r).fold(f32::MAX, f32::min);
        assert!(outer(&white_hot) > outer(&white_cold), "外描边被挤到更外侧：{white_hot:?} vs {white_cold:?}");
        assert!(inner(&white_hot) < inner(&white_cold), "内描边被挤到更内侧：{white_hot:?} vs {white_cold:?}");
    }

    /// hover 绿箭头：箭头线段真的变长，且两种长度下尖端都可命中。
    #[test]
    fn hovering_an_arrow_lengthens_it() {
        let (mut app, _dot) = app_with_motion();
        on(&mut app, 0.0);
        let g = geos(&app).pop().expect("one gizmo");
        // 绿箭头是唯一水平向左的 Line（轨迹线也有 Line，但方向不同），按其颜色筛。
        // 箭头现在是「杆 + 两条倒 V 斜线」共 3 条，取**最长的那条**（杆）作为长度。
        let green_line_len = |a: &CanvasApp| -> f32 {
            let mut p = Painter::new(a.canvas.rect);
            let t = a.theme;
            draw(a, &mut p, &t);
            p.ops
                .iter()
                .filter_map(|o| match o {
                    Op::Line { a, b, stroke } if stroke.color == AXIS_X => Some(a.distance(*b)),
                    _ => None,
                })
                .fold(0.0f32, f32::max)
        };
        app.pointer = Some(pos2(-100.0, -100.0));
        let cold = green_line_len(&app);
        app.pointer = Some(g.center + g.axis_x * (RING_R + ARROW_LEN - 4.0)); // hover 绿箭头
        let hot = green_line_len(&app);
        assert!((cold - ARROW_LEN).abs() < 0.01, "常态箭头长 {ARROW_LEN}：{cold}");
        assert!((hot - ARROW_LEN_HOVER).abs() < 0.01, "hover 箭头长 {ARROW_LEN_HOVER}：{hot}");
        assert!(hot > cold, "hover 让箭头变长");
    }

    #[test]
    fn draw_is_silent_while_playing() {
        let (mut app, _dot) = app_with_motion();
        let src = app.session.ds().expect("ds").doc.clone();
        app.motion_overlay = State::new(true, true, 1000.0, 60, 2000, Some(preview_at(&app, 1000.0)), Some(src));
        let mut painter = Painter::new(app.canvas.rect);
        let theme = app.theme;
        draw(&app, &mut painter, &theme);
        assert!(painter.ops.is_empty(), "播放中不画路径/手柄：{} 条指令", painter.ops.len());
    }

    #[test]
    fn a_clip_only_animation_still_gets_a_path_and_a_gizmo() {
        let mut doc = Document::new("motion-overlay-clip");
        let page = doc.first_page().expect("page");
        let card = doc.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 200.0, 200.0).expect("card");
        let dot = doc.create(card, NodeKind::Rectangle, "Dot", 10.0, 10.0, 40.0, 40.0).expect("dot");
        let mut app = CanvasApp::new(doc);
        app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        if let Ok(ds) = app.session.ds_mut() {
            ds.view.vw = 800.0;
            ds.view.vh = 600.0;
            ds.view.zoom = 1.0;
        }
        app.session.execute("motion.preset", &serde_json::json!({"preset": "Burst", "ids": [dot.0]})).expect("preset");
        assert!(app.session.ds().unwrap().doc.node(dot).is_some_and(|n| n.motion.track(AnimProperty::Position).is_none()));
        app.session.set_selection(vec![dot]);
        on(&mut app, 400.0);

        let mut painter = Painter::new(app.canvas.rect);
        let theme = app.theme;
        draw(&app, &mut painter, &theme);

        assert!(painter.ops.iter().any(|o| matches!(o, Op::Line { .. })), "clip 也要有轨迹线");
        let ring = painter.ops.iter().filter(|o| matches!(o, Op::Circle { radius, .. } if *radius == RING_R)).count();
        assert_eq!(ring, 1, "clip 也要有手柄蓝环（整圆填充已删）");
        let quad = |o: &Op| matches!(o, Op::Poly { points, .. } if points.len() == 4);
        // clip 没有离散帧：只剩手柄中心菱形 1 个四边形（缩放块是圆角矩形，走 Op::Rect）。
        assert_eq!(painter.ops.iter().filter(|o| quad(o)).count(), 1);
    }
}
