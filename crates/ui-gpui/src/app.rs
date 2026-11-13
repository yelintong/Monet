//! 画布宿主：引擎会话 + 画布状态 + 主题令牌。
//!
//! 对应原文 `FigApp` 里画布用到的部分（`session` / `canvas` / `to_screen` / `to_page`），
//! 让搬过来的画布代码不用改调用点。

use figcraft_doc::color::{Color, Paint};
use figcraft_doc::{AutoLayout, Document, LayoutMode, NodeId, NodeKind};
use figcraft_engine::Session;
use figcraft_geom::Point;
use figcraft_i18n::tr;

use crate::canvas::CanvasState;
use crate::canvas::cursors::Shown;
use crate::canvas::geom::*;
use crate::canvas::guides;
use crate::theme::{Tokens, canvas_accent};

pub struct CanvasApp {
    pub session: Session,
    pub canvas: CanvasState,
    pub theme: Tokens,
    /// 指针在画布里的位置（画布局部坐标），自绘光标/缩放框要用。
    pub pointer: Option<Pos2>,
    /// 缩放工具正在拖的起点。
    pub zoom_drag: Option<Pos2>,
    /// 本帧指针该显示什么（None：指针不在画布上）。
    pub cursor: Option<Shown>,
    /// 标尺/参考线自己要的光标（覆盖工具光标）。
    pub guide_cursor: Option<Shown>,
    /// 参考线拖拽状态。
    pub(crate) guides: guides::State,
    /// 自动布局 padding/gap 手柄的拖拽状态。
    pub(crate) handles: crate::canvas::handles::State,
    /// 帧统计（`ui.inspect` 的 `framesDrawn`）。
    pub stats: crate::canvas::frame_stats::FrameStats,
    /// 多选间距/重排手柄的拖拽与布局缓存。
    pub(crate) selection_spacing: crate::canvas::selection_spacing::State,
    /// 手柄要的光标（覆盖工具光标），每个事件重设、每帧取用一次。
    pub(crate) spacing_cursor: Option<Shown>,
    /// 每逻辑像素几个设备像素（窗口的 scale factor）：描边吸附到设备像素要用。
    pub ppp: f32,
    /// Dev Mode：右栏 Inspect 标签是否激活（egui `dev_mode::active()` 的 `right_tab` 一半）。
    /// 每帧由 `CanvasView::render` 从 `RightFields::right_tab` 同步过来，画布与预览共用。
    pub dev_inspect: bool,
    /// 画布上注解的分类过滤器（egui `CategoryFilter`）：
    /// `None` = 全部 / `Some("")` = 无分类 / `Some(id)` = 只看这个分类。
    pub dev_cat_filter: Option<String>,
    /// Motion 模式的画布叠加层（运动路径 + 变换手柄）：每帧由 `CanvasView::sync_motion`
    /// 从 Motion 状态同步过来（`None`/失活 = 不画）。
    pub motion_overlay: crate::canvas::motion_overlay::State,
    /// Motion gizmo 的跨帧拖动句柄（`State` 每帧重建，放不下）。
    pub motion_gizmo: crate::canvas::motion_overlay::Interaction,
}

impl CanvasApp {
    pub fn new(doc: Document) -> Self {
        let mut app = Self {
            session: Session::with_document(doc),
            canvas: CanvasState::default(),
            theme: Tokens::dark(),
            pointer: None,
            zoom_drag: None,
            cursor: None,
            guide_cursor: None,
            guides: guides::State::default(),
            handles: crate::canvas::handles::State::default(),
            stats: crate::canvas::frame_stats::FrameStats::default(),
            selection_spacing: crate::canvas::selection_spacing::State::default(),
            spacing_cursor: None,
            ppp: 1.0,
            dev_inspect: false,
            dev_cat_filter: None,
            motion_overlay: crate::canvas::motion_overlay::State::default(),
            motion_gizmo: crate::canvas::motion_overlay::Interaction::default(),
        };
        // 预选一个自动布局容器：启动即可见 hatch（紧贴轮廓 + padding 蓝斜线 + spacing 粉斜线），
        // 不用先手动点选，方便核对"选中=1px / 悬停=2px"与边距斜线管线。
        if let Ok(ds) = app.session.ds()
            && let Some(id) = ds.doc.find_by_name("AutoLayout · H", Some(ds.page)).into_iter().next()
        {
            app.session.set_selection(vec![id]);
        }
        app
    }

    /// 页面 → 屏幕（逻辑点）。
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let Ok(ds) = self.session.ds() else { return Pos2::ZERO };
        let (sx, sy) = ds.view.page_to_screen(x, y);
        pos2(self.canvas.rect.min.x + sx as f32, self.canvas.rect.min.y + sy as f32)
    }

    /// 屏幕 → 页面。
    pub fn to_page(&self, p: Pos2) -> Point {
        let Ok(ds) = self.session.ds() else { return Point::ZERO };
        let (x, y) = ds.view.screen_to_page(f64::from(p.x - self.canvas.rect.min.x), f64::from(p.y - self.canvas.rect.min.y));
        Point::new(x, y)
    }

    /// 画布叠加层的主色（页面深色时用亮品牌绿）。
    pub fn accent(&self) -> Color32 {
        canvas_accent(self.canvas.page, self.theme.accent, self.theme.accent_hover)
    }

    /// 当前页面。
    pub fn page(&self) -> Option<NodeId> {
        self.session.ds().ok().map(|d| d.page)
    }

    /// 当前选中。
    pub fn selection(&self) -> Vec<NodeId> {
        self.session.ds().map(|d| d.selection.clone()).unwrap_or_default()
    }
}

/// 一份含一个 Frame 与两个真实几何节点的示例文档。
pub fn sample_doc() -> Document {
    let mut doc = Document::new("gpui-canvas");
    let page = doc.pages[0];

    let frame = doc.create(page, NodeKind::Frame, "Frame 1", 40.0, 40.0, 420.0, 320.0).unwrap_or(page);

    let blue = doc.create(frame, NodeKind::Rectangle, tr("Blue"), 40.0, 40.0, 220.0, 140.0).unwrap_or(frame);
    if let Some(n) = doc.node_mut(blue) {
        // 替换（不是 push）：`Node::new` 已给矩形一个默认 `DEFAULT_SHAPE` 填充，push 会留下它。
        n.fills = vec![Paint::solid(Color::from_rgba8(0x33, 0x88, 0xFF, 255))];
    }
    let red = doc.create(frame, NodeKind::Rectangle, tr("Red"), 170.0, 160.0, 150.0, 110.0).unwrap_or(frame);
    if let Some(n) = doc.node_mut(red) {
        n.fills = vec![Paint::solid(Color::from_rgba8(0xFF, 0x55, 0x55, 255))];
    }
    // 两个**同色相邻**的色块（共用一条边，不留缝不叠压）：用来检查拼接缝——
    // 同一块纯色被拆成两次绘制（分块/部分重绘/贴图边缘）时，中缝会露出一条比两侧
    // 略深或略浅的 1px 线。放大缩小、平移前后都盯这条缝看。
    let seam = Color::from_rgba8(0x2F, 0xA3, 0x6B, 255);
    for (name, x, w) in [("Seam Left", 40.0, 240.0), ("Seam Right", 280.0, 240.0)] {
        let n = doc.create(page, NodeKind::Rectangle, name, x, 380.0, w, 140.0).unwrap_or(page);
        if let Some(n) = doc.node_mut(n) {
            n.fills = vec![Paint::solid(seam)];
        }
    }
    // 页面上两条参考线（标尺拖出来的那种），方便无头预览核对画法。
    if let Some(p) = doc.node_mut(page) {
        p.guides_mut().push(figcraft_doc::node::Guide { vertical: true, offset: 60.0 });
        p.guides_mut().push(figcraft_doc::node::Guide { vertical: false, offset: 30.0 });
    }

    // 两个自动布局容器：用来验证"紧贴轮廓（选中 1px / 悬停 2px）+ padding 蓝斜线 +
    // spacing 粉斜线"管线。先用普通 Frame + 显式坐标摆好（即使布局引擎不重排也能看对），
    // 再挂上 `layout` 让 `is_auto_layout` 为真、触发 hatch。
    let al_h = doc.create(page, NodeKind::Frame, "AutoLayout · H", 40.0, 560.0, 480.0, 180.0).unwrap_or(page);
    if let Some(n) = doc.node_mut(al_h) {
        n.layout = Some(AutoLayout { mode: LayoutMode::Horizontal, gap: 24.0, padding: [32.0, 32.0, 32.0, 32.0], ..Default::default() });
        // 自适应（Hug）：拖 padding 时 frame 跟着内容+内边距变大变小（Figma 的 Hug 行为）。
        n.child_layout.sizing_h = figcraft_doc::Sizing::Hug;
        n.child_layout.sizing_v = figcraft_doc::Sizing::Hug;
    }
    // 子层坐标是**父容器内**的局部坐标（`create` 把 x/y 写进 transform）：padding 32 + gap 24，
    // 110×116 的三个块落在 32 / 166 / 300。写成页面坐标会被父级 `clip` 整块裁掉（内容不渲染）。
    for (i, (x, fill)) in [(32.0, 0x33_88_FF), (166.0, 0x33_CC_88), (300.0, 0xCC_88_33)].into_iter().enumerate() {
        let c = doc.create(al_h, NodeKind::Rectangle, &format!("H·item {i}"), x, 32.0, 110.0, 116.0).unwrap_or(al_h);
        if let Some(n) = doc.node_mut(c) {
            n.fills = vec![Paint::solid(Color::from_rgba8((fill >> 16) as u8, (fill >> 8) as u8, fill as u8, 255))];
        }
    }

    let al_v = doc.create(page, NodeKind::Frame, "AutoLayout · V", 560.0, 40.0, 200.0, 420.0).unwrap_or(page);
    if let Some(n) = doc.node_mut(al_v) {
        n.layout = Some(AutoLayout { mode: LayoutMode::Vertical, gap: 24.0, padding: [32.0, 32.0, 32.0, 32.0], ..Default::default() });
        n.child_layout.sizing_h = figcraft_doc::Sizing::Hug;
        n.child_layout.sizing_v = figcraft_doc::Sizing::Hug;
    }
    for (i, (y, fill)) in [(32.0, 0x88_33_CC), (146.0, 0xCC_33_88), (260.0, 0x33_CC_CC)].into_iter().enumerate() {
        let c = doc.create(al_v, NodeKind::Rectangle, &format!("V·item {i}"), 32.0, y, 136.0, 90.0).unwrap_or(al_v);
        if let Some(n) = doc.node_mut(c) {
            n.fills = vec![Paint::solid(Color::from_rgba8((fill >> 16) as u8, (fill >> 8) as u8, fill as u8, 255))];
        }
    }

    doc
}
