//! 画布：渲染好的整页图像 + 选中/悬停/参考线/标签等叠加层。
//!
//! 结构与 `ui-egui/src/canvas.rs` 一一对应：
//! `show()` 负责「同步视口尺寸 → 请求/轮询渲染 → 画页面底色与贴图 → 画叠加层」，
//! 叠加层各自在子模块里（标尺、手柄、overlays…），全部沿用原实现的画法。

pub mod builder;
pub mod cursors;
/// Dev Mode 画布层（注解气泡 + 已存测量线，egui `dev_mode::draw` / `saved_lines`）。
pub mod dev;
pub mod frame_labels;
pub mod frame_stats;
pub mod geom;
pub mod guides;
pub mod handles;
pub mod measure_overlay;
pub mod outline;
pub mod overlays;
pub mod painter;
pub mod render;
pub mod render_worker;
pub mod rulers;
pub mod selection_spacing;
/// 动态内联 SVG：GPUI 0.2.2 里唯一能旋转的文字（标尺左侧数字）。
pub mod svg_text;
pub mod tiles;

pub use geom::*;
pub use painter::Painter;

use figcraft_i18n::tr;
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::{View, Work};
use gpui::RenderImage;

use crate::app::CanvasApp;
use crate::theme::{Tokens, doc_c32};
use render::JobKey;
use render_worker::{Renders, Worker};

/// 拖动补丁：一张**当帧同步渲染的局部完整场景**，盖在冻结的整页贴图之上。
///
/// 它同时做两件事：用不透明的场景把底图里「被拖图层还停在旧位置」的那一份盖掉（不必再
/// 异步挖空整页），并按当前文档把图层连同**所有效果**画在新位置。走的是和静态画面完全相同
/// 的渲染管线，所以内置阴影/模糊、外置 WGSL 材质、容器的子节点与 frame 裁剪都在，不会丢、
/// 也不会慢半帧。
pub struct Live {
    pub image: Arc<RenderImage>,
    /// 这张补丁覆盖的页面矩形（`x0 y0 x1 y1`）。
    pub page: [f64; 4],
    /// 拖动开始时被拖图层的包围盒：每帧的补丁都要把它包进来，旧残影才擦得干净。
    pub start: [f64; 4],
    /// 这次拖动抓的是哪几层：中途换了选中就作废重来，别把两组图层的旧位置糊在一起。
    pub ids: Vec<NodeId>,
    /// 松手时提交的那张整页渲染序号：它上台之前补丁要一直留着，否则图层会闪没。
    pub release_seq: Option<u64>,
}

/// 画布状态（对应原文的 `CanvasState`）。
pub struct CanvasState {
    /// 画布在窗口中的矩形（逻辑像素）。
    pub rect: Rect,
    pub texture: Option<Arc<RenderImage>>,
    pub tex_view: Option<View>,
    /// 贴图显示的文件与页面：只在它们上绘制。
    pub tex_of: Option<(u64, NodeId)>,
    /// 贴图是粗预览。
    pub tex_coarse: bool,
    /// 贴图回答的请求序号。
    pub tex_seq: u64,
    /// 覆盖整页的低分辨率兜底图（见 [`render_worker::Overview`]），铺在清晰贴图**下面**。
    /// 平移露出新区域时，清晰贴图还没到的那一条立刻有内容，看不见"正在加载"。
    pub overview: Option<(Arc<RenderImage>, View)>,
    /// 兜底图描述的是哪一版内容（`doc 指针 / 页面 / 字体代次`）。
    pub overview_of: Option<(usize, NodeId, u64)>,
    /// 拖动中盖在整页贴图之上的局部场景补丁（见 [`Live`]）。
    pub live: Option<Live>,
    pub(crate) submitted: Option<JobKey>,
    /// 上一帧是否在拖动：松手那一帧用它强制提交一张含最终位置的整页图。
    pub(crate) dragging: bool,
    /// UI 线程自己的材质合成器（拖动补丁当帧同步渲染时用，不与渲染线程抢设备）。
    /// 只有文档里真的绑了外置材质才会惰性创建 wgpu 设备。
    pub patch_renders: Option<Renders>,
    /// 最后一次请求渲染的视图。
    pub(crate) want_view: Option<View>,
    /// 上一次画布渲染做了什么。
    pub render_work: Option<Work>,
    pub preview_latency_ms: Option<f64>,
    pub refine_latency_ms: Option<f64>,
    /// 本帧的页面底色（用于决定叠加层用亮绿还是暗绿）。
    pub page: Color32,
    /// 这一帧被替换下来的图：视图层负责 `cx.drop_image`。
    ///
    /// GPUI 把每张 `RenderImage` 放进 sprite atlas，**只有显式 drop 才移除**；不 drop 的话
    /// 每次渲染/每帧替身都永久占着 GPU 与 CPU 的一份（整页图 17.6MB、替身按面积）。
    /// 缩放、拖动、旋转会持续产图——这就是「图片没被释放」的那条泄漏。
    pub retired: Vec<Arc<RenderImage>>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            texture: None,
            tex_view: None,
            tex_of: None,
            tex_coarse: false,
            tex_seq: 0,
            overview: None,
            overview_of: None,
            live: None,
            submitted: None,
            dragging: false,
            patch_renders: None,
            want_view: None,
            render_work: None,
            preview_latency_ms: None,
            refine_latency_ms: None,
            page: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            retired: Vec::new(),
        }
    }
}

/// 一次画布渲染的产物：贴图（图像 + 屏幕矩形，画布局部坐标）+ 记录下来的绘制指令。
pub struct CanvasOutput {
    pub image: Option<(Arc<RenderImage>, Rect)>,
    /// 整页兜底图（低分辨率）：铺在 `image` **下面**，只在 `image` 没盖住的地方露出来。
    pub overview: Option<(Arc<RenderImage>, Rect)>,
    /// 拖动补丁（不透明的局部场景）：贴在整页贴图**之上**、叠加层**之下**。
    /// 第三项保留给视图层的可选裁剪矩形；新方案里恒为 `None` —— frame 裁剪由渲染器在
    /// 生成补丁时按当前文档现算，不需要画布再手工套一个不动的裁剪框。
    pub live: Option<(Arc<RenderImage>, Rect, Option<Rect>)>,
    pub painter: Painter,
    /// 页面底色。由视图层当作容器背景铺在**贴图下面**——不能进 `painter.ops`，
    /// 否则它会被画在贴图之上、把整页内容盖住。
    pub bg: Color32,
    /// 还有渲染没上台：视图层据此续帧，直到一切都画出来。
    pub in_flight: bool,
    /// 本帧换下来的图（替身、整页贴图）：视图层逐个 `cx.drop_image`，否则永远留在 atlas 里。
    pub retired: Vec<Arc<RenderImage>>,
}

/// 跑一遍画布（对应原文 `canvas::show`）。
pub fn show(app: &mut CanvasApp, worker: &mut Worker, ppp: f32, t: &Tokens) -> CanvasOutput {
    let rect = app.canvas.rect;

    // 视口尺寸同步：首次定尺寸时缩放到合适大小；之后**锚定左上角**。
    //
    // `origin()` 是 `cx - vw/2/z`，所以只改 `vw/vh` 会让整幅画随视口宽度平移——拖右栏时
    // 内容被"挤着跑"就是这么来的。改尺寸时把 `cx/cy` 反算回去，保持左上角的世界坐标不变，
    // 画面就钉在原地（右栏变宽只是少看见右边一截，内容不动）。
    if let Ok(ds) = app.session.ds_mut()
        && ((ds.view.vw - f64::from(rect.width())).abs() > 0.5 || (ds.view.vh - f64::from(rect.height())).abs() > 0.5)
    {
        let first = ds.view.vw == 1200.0 && ds.view.vh == 800.0;
        let (ox, oy) = ds.view.origin();
        let z = ds.view.zoom.max(1e-9);
        ds.view.vw = f64::from(rect.width());
        ds.view.vh = f64::from(rect.height());
        if first {
            ds.zoom_to_fit();
        } else {
            ds.view.cx = ox + ds.view.vw / 2.0 / z;
            ds.view.cy = oy + ds.view.vh / 2.0 / z;
        }
    }

    // 页面底色（贴图滞后/局部时也铺在底下）。
    let bg = app
        .session
        .ds()
        .ok()
        .and_then(|ds| match ds.doc.node(ds.page).map(|n| &n.kind) {
            Some(NodeKind::Page { background, .. }) => Some(doc_c32(*background)),
            _ => None,
        })
        .unwrap_or(t.canvas);
    app.canvas.page = bg;

    // 拖动中的图层：整页贴图冻住不动，画布当帧自己重绘那一小块完整场景（见 `patch_for`）。
    // 平移、拉伸、旋转一律每帧重算 —— 图层像素本来就在变，位图平移那套会跟效果脱节。
    let (mut moving, mut redraw) =
        app.session.tool.live_layers().map(|(ids, draw)| (ids, draw == figcraft_engine::LiveDraw::Reshape)).unwrap_or_default();
    // 复用 resize 的 live-layer reshape 路径：拖间距/边距手柄时，把受影响的图层也并进 `moving`
    // 并置 `redraw`（间距手柄本就是 `arrange.spacing` 指令下放改文档）。这样画布走和 resize 完全
    // 一致的「整页让位 + 当帧重抓替身」，内容贴着框走——否则整页只能异步重渲，背景跟不上框（果冻）。
    if let Some((ids, r)) = app.selection_spacing.live_layers() {
        for id in ids {
            if !moving.contains(&id) {
                moving.push(id);
            }
        }
        redraw |= r;
    }
    // 拖自动布局的 padding / gap 手柄：和 resize 走同一条 live 路径（当帧重抓替身）。
    // 不进 live 的话每个指针事件只能等一次整页异步渲染——带子动了、内容和背景还没跟上。
    if let Some((ids, r)) = app.handles.live_layers() {
        for id in ids {
            if !moving.contains(&id) {
                moving.push(id);
            }
        }
        redraw |= r;
    }
    // 拉伸会重排父级与兄弟：把被拉的那一层所在的、会随它变化的框（自动布局父级、跟随子级的组）
    // 并进 `moving`，补丁于是重绘整个受影响的框，父级 hug、兄弟位移都与选框同一帧。
    // 旋转不重排（引擎 `live_reflows()` 只对 Resize 为真），`moving` 只含被转的几层；
    // 每帧按当前变换重绘局部场景，不需要旧替身那套 reshape 标志与手工裁剪。
    //
    // 拖入/重排自动布局（Move）同样每帧 reflow 兄弟：引擎 `drag.rs` 在 live 阶段就 `reparent` 并把
    // 被拖层插入目标容器的 `children`，auto layout 随即把兄弟推到新位置。但 Move 走
    // `LiveDraw::Translate`、`live_reflows()` 为假，`moving` 因此只含被拖层，`moving_bounds` 只盖它
    // 自己——被推动的兄弟「旧位置」留在冻结底图上成残影、「新位置」落在补丁外直接不显示，就是
    // 「拖进自动布局有东西不渲染」的根因。引擎每帧把插入指示线写进 `overlay.insert_line`，它是
    // 「正处于 auto-layout flow」的可靠信号（同容器内重排时 `drop_target` 为 None 但 insert_line 仍
    // 为 Some，所以必须用它、不能用 drop_target）。此时和 Resize 一样提升 `affected_root`：被拖层
    // 的 parent 已被 reparent 成目标容器（同容器重排则本就是它），沿父链爬到该容器，补丁整块重绘，
    // 兄弟的旧/新位一次盖全，与选框同一帧。
    let dragging_into_flow = !redraw
        && app.session.tool.overlay.insert_line.is_some();
    if (redraw && app.session.tool.live_reflows() || dragging_into_flow)
        && let Ok(ds) = app.session.ds()
    {
        let mut roots: Vec<NodeId> = Vec::new();
        for id in &moving {
            let r = affected_root(&ds.doc, *id);
            if !roots.contains(&r) {
                roots.push(r);
            }
        }
        moving = roots;
    }
    let moving_bounds = if moving.is_empty() { None } else { app.session.ds().ok().and_then(|ds| ds.doc.bounds_of(&moving)) };
    // 新一次拖动：旧补丁记的「起始位置」不属于这次操作，作废重来；同一次拖动的连续帧保留
    // `start`，旧位置的残影才一直被盖住。松手后补丁留到新的整页贴图上台（见文件末尾）。
    let was_dragging = app.canvas.dragging;
    let released = was_dragging && moving.is_empty();
    if !moving.is_empty() {
        let changed = app.canvas.live.as_ref().is_some_and(|l| l.ids != moving);
        if !was_dragging || changed {
            app.canvas.live = None;
        }
        app.canvas.dragging = true;
    } else if released {
        app.canvas.dragging = false;
        // 拖动期间 `request_render` 一次都没提交过；作废指纹，强制提交一张含最终位置的整页图。
        app.canvas.submitted = None;
    }

    // 自动布局的 padding / spacing 提示带：交给渲染铺（在子层之前）。悬停到哪条就只给哪条。
    // 先刷新粘性悬停（指针还在上一条里就锁定它，离开才重判），再据此出带子。
    handles::update_hover(app);
    let bands = handles::bands_for_render(app);

    // 局部场景补丁：每帧**当帧同步**渲染「旧位置 ∪ 新位置 + 效果余量」那一块不透明的完整场景。
    // 它和整页贴图走同一条渲染管线（内置效果、外置 WGSL 材质、容器裁剪与子节点都在），所以
    // 效果与选框天然同一帧；底图里停在旧位置的那份被这块不透明场景直接盖掉，无需异步挖空。
    if let Some(b) = moving_bounds {
        let start = app.canvas.live.as_ref().map(|l| l.start).unwrap_or([b.x0, b.y0, b.x1, b.y1]);
        if let Some((image, page)) = patch_for(app, ppp, &moving, start, b, &bands) {
            if let Some(old) = app.canvas.live.take() {
                app.canvas.retired.push(old.image);
            }
            app.canvas.live = Some(Live { image, page, start, ids: moving.clone(), release_seq: None });
        }
    }

    render::request_render(app, worker, ppp, MAX_TEXTURE_SIDE, &moving, moving_bounds, redraw, &bands);
    // 松手这一帧刚提交的那张整页图：补丁一直留到它上台。
    if released && let Some(l) = app.canvas.live.as_mut() {
        l.release_seq = Some(worker.submitted());
    }

    // 每张完成的图都上台（按顺序完成；拖拽时每帧都有编辑，否则永远看不到）。
    // 顺序要紧：**先**读完成计数、**再** poll。worker 先发结果再记账，所以计数相等时
    // 结果必然已经在通道里；反过来（先 poll 后读）会把「刚发出去还没被读到」的那张
    // 判成「还在渲染」，于是这一帧不再续帧，画面就停在上一张上——拖拽时内容落后、
    // 松手后定格，都是这个洞。
    let finished = worker.finished();
    let shown = app.session.ds().map(|ds| (ds.uid, ds.page)).ok();
    if let Some(done) = worker.poll()
        && render::shows(&app.canvas, &done, shown)
    {
        if !done.coarse {
            app.canvas.render_work = Some(done.work);
        }
        let latency = Some(done.at.elapsed().as_secs_f64() * 1000.0);
        if render_worker::render_log() {
            eprintln!(
                "[canvas] {} seq={} {:.1}ms end-to-end {:.1}ms",
                if done.coarse { "coarse" } else { "refine" },
                done.seq,
                done.ms,
                latency.unwrap_or(0.0)
            );
        }
        if done.coarse {
            app.canvas.preview_latency_ms = latency;
        } else {
            app.canvas.refine_latency_ms = latency;
        }
        if let Some(old) = app.canvas.texture.replace(done.image) {
            app.canvas.retired.push(old);
        }
        app.canvas.tex_view = Some(done.view);
        app.canvas.tex_of = Some((done.uid, done.page));
        app.canvas.tex_coarse = done.coarse;
        app.canvas.tex_seq = done.seq;
    }
    if let Some(o) = worker.poll_overview() {
        if let Some(old) = app.canvas.overview.replace((o.image, o.view)) {
            app.canvas.retired.push(old.0);
        }
        app.canvas.overview_of = Some(o.key);
    }
    let in_flight = finished < worker.submitted();

    // 底色交给视图层做容器背景（见 `CanvasOutput::bg` 的注释）。
    let mut painter = Painter::new(rect);

    // 把上次渲染重投影到当前视图。
    let mut image = None;
    if let (Some(tex), Some(v), true) = (app.canvas.texture.as_ref(), app.canvas.tex_view.as_ref(), shown.is_some() && app.canvas.tex_of == shown) {
        let tl = app.to_screen(v.origin.0, v.origin.1);
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
        let pixel_preview = app.session.ds().ok().and_then(|ds| ds.view.pixel_preview_scale()) == Some(v.scale);
        let r = presented_image_rect(tl, v, zoom, ppp, pixel_preview);
        image = Some((tex.clone(), r.translate(-rect.min.to_vec2())));
    }

    // 整页兜底图：铺在清晰贴图**下面**（`image` 之后画，会盖住它没覆盖的部分）。
    let overview = app.canvas.overview.as_ref().map(|(tex, v)| {
        let tl = app.to_screen(v.origin.0, v.origin.1);
        let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
        let r = presented_image_rect(tl, v, zoom, ppp, false);
        (tex.clone(), r.translate(-rect.min.to_vec2()))
    });

    // 叠加层（标尺；后续的选中框/手柄/参考线/标签也在这一层）。
    draw_overlays(app, &mut painter, t);

    // 拖动补丁按它这一帧覆盖的页面矩形摆放：不透明的完整场景同时擦掉旧位置、画出新位置。
    // 裁剪不再手工套 —— 补丁本身就是渲染器按当前文档（含 frame clip）画出来的。
    let live = app.canvas.live.as_ref().map(|l| {
        let p0 = app.to_screen(l.page[0], l.page[1]);
        let p1 = app.to_screen(l.page[2], l.page[3]);
        let r = Rect::from_min_max(p0, p1).translate(-rect.min.to_vec2());
        (l.image.clone(), r, None)
    });

    // 松手后不立刻丢掉补丁：等那张含最终位置的整页贴图真的上台再丢，否则图层会闪一下。
    // `!in_flight()` 是兜底 —— 万一那张图被 `shows()` 拒掉，不能把补丁永远挂在画布上。
    let release_seq = app.canvas.live.as_ref().and_then(|l| l.release_seq);
    if let Some(seq) = release_seq
        && (app.canvas.tex_seq >= seq || !worker.in_flight())
    {
        app.canvas.live = None;
    }

    let retired = std::mem::take(&mut app.canvas.retired);
    CanvasOutput { image, overview, live, painter, bg, in_flight, retired }
}

/// 一张拖动补丁的像素上限（4MP ≈ 16MB）。补丁每帧重建，拖得很远时矩形会接近整页，
/// 超过就降渲染分辨率：按页面矩形贴上去只是略糊，不会错位。
const MAX_PATCH_PIXELS: f64 = 4_000_000.0;

/// 补丁要覆盖的页面矩形：起始包围盒 ∪ 当前包围盒 + 效果余量，再裁到可见视口。
/// 两块都包住是关键 —— 少包旧位置就留下原地残影，少包新位置就把效果切掉半个。
fn patch_rect(start: [f64; 4], now: figcraft_geom::Rect, pad: f64, visible: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        (start[0].min(now.x0) - pad).max(visible[0]),
        (start[1].min(now.y0) - pad).max(visible[1]),
        (start[2].max(now.x1) + pad).min(visible[2]),
        (start[3].max(now.y1) + pad).min(visible[3]),
    ];
    (r[2] - r[0] >= 0.5 && r[3] - r[1] >= 0.5).then_some(r)
}

#[cfg(test)]
mod patch_tests {
    use super::patch_rect;
    use figcraft_geom::Rect;

    /// 内置效果的残影就是「旧位置没被盖住」。补丁必须同时盖住拖动起点和当前位置。
    #[test]
    fn the_patch_covers_both_the_ghost_and_the_new_position() {
        let start = [0.0, 0.0, 100.0, 100.0];
        let now = Rect::new(300.0, 20.0, 400.0, 120.0);
        let r = patch_rect(start, now, 16.0, [-1000.0, -1000.0, 2000.0, 2000.0]).expect("a patch");
        assert!(r[0] <= start[0] && r[1] <= start[1] && r[2] >= start[2] && r[3] >= start[3], "旧位置要被盖住: {r:?}");
        assert!(r[0] <= now.x0 && r[1] <= now.y0 && r[2] >= now.x1 && r[3] >= now.y1, "新位置要画得下: {r:?}");
    }

    #[test]
    fn the_patch_stays_inside_the_visible_viewport() {
        let r = patch_rect([0.0, 0.0, 100.0, 100.0], Rect::new(0.0, 0.0, 100.0, 100.0), 16.0, [20.0, 20.0, 80.0, 80.0]).expect("a patch");
        assert_eq!(r, [20.0, 20.0, 80.0, 80.0]);
    }

    #[test]
    fn a_layer_dragged_entirely_out_of_view_needs_no_patch() {
        assert!(patch_rect([0.0, 0.0, 10.0, 10.0], Rect::new(0.0, 0.0, 10.0, 10.0), 4.0, [1000.0, 1000.0, 1100.0, 1100.0]).is_none());
    }

    /// 旋转（`Gesture::Rotate` → `LiveDraw::Reshape`）会把世界 AABB 撑大（45° 的正方形约 1.41 倍）。
    /// 冻结底图里仍是原始角度那一份，补丁必须把「旧 AABB」与「转动后的大 AABB」一起包住；
    /// 每帧只换一张补丁、不叠上一帧，所以中间角度不需要额外留范围。
    #[test]
    fn a_turn_that_grows_the_bounds_still_covers_the_original_ghost() {
        let start = [0.0, 0.0, 100.0, 100.0];
        let turned = Rect::new(-21.0, -21.0, 121.0, 121.0);
        let r = patch_rect(start, turned, 8.0, [-1000.0, -1000.0, 2000.0, 2000.0]).expect("a patch");
        assert!(r[0] <= start[0] && r[1] <= start[1] && r[2] >= start[2] && r[3] >= start[3], "旧角度要被盖住: {r:?}");
        assert!(r[0] <= turned.x0 && r[1] <= turned.y0 && r[2] >= turned.x1 && r[3] >= turned.y1, "新角度要画得下: {r:?}");
        assert!(r[2] - r[0] > 100.0, "转过之后补丁要跟着变大: {r:?}");
    }
}

/// 当帧渲染拖动补丁：范围 = `start`（拖动开始时的包围盒）∪ `now`（这一帧的包围盒）+ 效果余量，
/// 裁到可见视口后，走与整页贴图**完全相同**的 `run` + 材质合成。
///
/// 旧路径的「透明底替身」有两个先天缺陷：它绕开了 `Options::skip` 挖空整页时连根删掉的容器
/// 子树，也拿不到外置材质所需的干净 backdrop。这里直接渲染一个不透明的局部场景，两个问题
/// 一起消失，而且阴影 / 模糊 / 描边的外溢只受这里留的 `pad` 限制，不会被位图边界切断。
fn patch_for(
    app: &mut CanvasApp,
    ppp: f32,
    moving: &[NodeId],
    start: [f64; 4],
    now: figcraft_geom::Rect,
    bands: &[figcraft_render::Band],
) -> Option<(Arc<RenderImage>, [f64; 4])> {
    let Ok(ds) = app.session.ds() else { return None };
    let zoom = ds.view.zoom.max(1e-6);
    // 阴影 / 模糊 / 材质折射会越过几何包围盒：按效果 reach 与屏幕像素留一圈。
    let mut pad = 64.0 / zoom;
    for id in moving {
        if let Some(n) = ds.doc.node(*id) {
            for e in &n.effects {
                pad = pad.max(e.reach() * 1.5 + 24.0 / zoom);
            }
            // 外置 WGSL 材质的折射 / 霜化会越过几何边界，按屏幕像素再多留一圈。
            if n.plugin_data.contains_key(crate::wgsl_material::BINDING_KEY) {
                pad = pad.max(96.0 / zoom);
            }
        }
    }
    let (ox, oy) = ds.view.origin();
    let visible = [
        ox,
        oy,
        ox + f64::from(app.canvas.rect.width()) / zoom,
        oy + f64::from(app.canvas.rect.height()) / zoom,
    ];
    let Some(rect) = patch_rect(start, now, pad, visible) else { return None };
    let doc = ds.doc.clone();
    let page = ds.page;
    let uid = ds.uid;

    let mut scale = zoom * f64::from(ppp);
    // 单边不得超过纹理上限：先按边长收尺度，再按面积收，避免把补丁硬截断成缺一块。
    let side = f64::from(MAX_TEXTURE_SIDE);
    scale = scale.min(side / (rect[2] - rect[0])).min(side / (rect[3] - rect[1]));
    let size = |k: f64| {
        (
            ((rect[2] - rect[0]) * k).ceil().max(1.0) as u32,
            ((rect[3] - rect[1]) * k).ceil().max(1.0) as u32,
        )
    };
    let (mut w, mut h) = size(scale);
    let area = f64::from(w) * f64::from(h);
    if area > MAX_PATCH_PIXELS {
        scale *= (MAX_PATCH_PIXELS / area).sqrt();
        (w, h) = size(scale);
    }
    let view = View { origin: (rect[0], rect[1]), scale, width: w, height: h };
    let job = render_worker::patch_job(doc, page, view, bands, uid);
    let renders = app.canvas.patch_renders.get_or_insert_with(Renders::default);
    let done = renders.render_patch(&job)?;
    // `Retained::render` 会把 view 吸附到整设备像素，图上台的位置要用它返回的 view 与真实图尺寸。
    let v = &done.view;
    let sz = done.image.size(0);
    let (iw, ih) = (u32::from(sz.width), u32::from(sz.height));
    let page_rect = [
        v.origin.0,
        v.origin.1,
        v.origin.0 + f64::from(iw) / v.scale,
        v.origin.1 + f64::from(ih) / v.scale,
    ];
    Some((done.image, page_rect))
}

/// 拉伸 `id` 时，它的父级与兄弟也会跟着变：返回**最小的那个把它们全包住**的图层——从 `id`
/// 往上爬，只要父级是自动布局（会重排子层）或跟随子级定尺寸的组（`bounds_follow_children`）
/// 就继续爬。补丁按这个框的包围盒重绘整块场景，父级 hug 与兄弟位移就和选框同一帧。
/// 旋转不走这里（引擎 `live_reflows()` 只对 Resize 为真）：它只改自己的几何，
/// `moving` 就只含这几层，补丁范围由 `start ∪ now` 决定，不必把框外内容无谓包进来。
fn affected_root(doc: &Document, id: NodeId) -> NodeId {
    let mut root = id;
    let mut cur = doc.node(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        let Some(pn) = doc.node(p) else { break };
        if !(pn.layout.is_some() || pn.kind.bounds_follow_children()) {
            break;
        }
        root = p;
        cur = pn.parent;
    }
    root
}

/// 拖动中的图层会被它所在的、会裁剪内容的 frame 切掉：返回那块裁剪区域（页面坐标）。
///
/// 替身是**整层**的一张图，平移时只换位置——图里烘的是抓取那一刻的裁剪，图层一移出去就对不上了
/// （用户实测：半出一半还在，正好是要裁掉的那一半）。所以裁剪按**当前**位置现算，画的时候套上去。
///
/// 多层各自在不同 frame 里时取并集：宁可少裁，也不要凭空把某一层裁没。
///
/// **拖出超过自身宽度一半就弹出去**：水平方向上被 frame 切掉的部分超过图层自身宽度的一半时，
/// 整组替身**不再裁切**（连同还没出框的兄弟一起放出），免得一半在框里、一半被生生切平。
/// 拖回框内（水平重叠回到一半以上）则恢复裁切。镜像方向同理：向右拖出去同样以一半为界。
///
/// 拉伸/旋转不做「弹出去」（`pop_out = false`）：那条规则是给**平移**用的判断，缩放时图层
/// 自身的宽度就在变，拿它当分母会让一块正常缩小的图层突然整块不裁。
#[allow(dead_code)] // 旧「位图替身」路径的裁剪规则；新补丁由渲染器自己裁，保留仅供回归测试对照。
fn live_clip(doc: &Document, ids: &[NodeId], pop_out: bool) -> Option<[f64; 4]> {
    let mut out: Option<[f64; 4]> = None;
    for id in ids {
        let mut clip: Option<[f64; 4]> = None;
        let mut cur = doc.node(*id).and_then(|n| n.parent);
        while let Some(p) = cur {
            let Some(n) = doc.node(p) else { break };
            if n.clip && n.kind.is_frame_like() {
                let b = figcraft_geom::transform_rect_bbox(doc.world_transform(p), n.local_rect());
                let r = [b.x0, b.y0, b.x1, b.y1];
                clip = Some(match clip {
                    None => r,
                    Some(c) => [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]), c[3].min(r[3])],
                });
            }
            cur = n.parent;
        }
        // 有一层谁都不裁：整组就不能裁（只裁它以外的层会露馅）。
        let c = clip?;
        // 水平方向拖出超过自身宽度一半 → 弹出去，整组不再裁切。
        if pop_out && let Some(lb) = doc.world_bounds(*id) {
            let w = lb.x1 - lb.x0;
            if w > 1e-6 {
                let overlap = (lb.x1.min(c[2]) - lb.x0.max(c[0])).max(0.0);
                if overlap < 0.5 * w {
                    return None;
                }
            }
        }
        out = Some(match out {
            None => c,
            Some(o) => [o[0].min(c[0]), o[1].min(c[1]), o[2].max(c[2]), o[3].max(c[3])],
        });
    }
    out
}

#[cfg(test)]
mod pop_out_tests {
    use super::*;
    use figcraft_geom::Affine;

    /// 取样文档里的 `Blue`（Frame 1 的左上角子矩形，宽 220）。
    fn blue(doc: &Document) -> NodeId {
        let page = doc.pages[0];
        let frame = doc.node(page).and_then(|n| n.children.first().copied()).expect("Frame 1");
        doc.node(frame).and_then(|n| n.children.first().copied()).expect(tr("Blue"))
    }

    fn shift(doc: &mut Document, id: NodeId, dx: f64) {
        let t = doc.node(id).expect("node").transform;
        doc.get_mut(id).expect("node").transform = Affine::translate((dx, 0.0)) * t;
    }

    #[test]
    fn exactly_half_out_stays_clipped() {
        let mut doc = crate::app::sample_doc();
        let b = blue(&doc);
        // Blue 世界 x∈[80,300]（frame 局部 40，frame 本身在 40）。左移 150 → [−70,150]，
        // 与 frame 左缘 40 的水平重叠恰好 110 = 宽 220 的一半 → 仍然裁切。
        shift(&mut doc, b, -150.0);
        assert!(live_clip(&doc, &[b], true).is_some(), "恰好一半出框：仍然裁切");
    }

    #[test]
    fn past_half_out_pops_out() {
        let mut doc = crate::app::sample_doc();
        let b = blue(&doc);
        // 左移 200 → [−120,100]，与 frame 重叠仅 60/220 ≈ 0.27，超过一半在框外 → 弹出去。
        shift(&mut doc, b, -200.0);
        assert!(live_clip(&doc, &[b], true).is_none(), "超过一半出框：弹出去，不再裁切");
    }

    #[test]
    fn dragged_right_past_half_also_pops_out() {
        let mut doc = crate::app::sample_doc();
        let b = blue(&doc);
        // 右移 300 → [380,580]，与 frame 右缘 460 的重叠仅 80/220 ≈ 0.36 → 同样弹出去。
        shift(&mut doc, b, 300.0);
        assert!(live_clip(&doc, &[b], true).is_none(), "向右拖出超过一半：同样弹出去");
    }

    #[test]
    fn back_inside_re_clips() {
        let mut doc = crate::app::sample_doc();
        let b = blue(&doc);
        // 右移 10：仍在框内，水平重叠充足 → 恢复裁切。
        shift(&mut doc, b, 10.0);
        assert!(live_clip(&doc, &[b], true).is_some(), "拖回框内：恢复裁切");
    }

    /// Regression for "dragging into an auto layout leaves some items unrendered". The live patch
    /// must be lifted to the whole flow container so reflowed siblings' old and new positions are
    /// both repainted in one frame. That lift walks up through `affected_root`; once the engine
    /// `reparent`s the dragged layer into the flow (which it does every live frame, see
    /// `engine/src/canvas/tools/drag.rs`), `affected_root` must reach that container. This pins the
    /// exact dependency the fix in `show()` relies on, without needing a full CanvasApp/gesture.
    #[test]
    fn affected_root_reaches_the_auto_layout_container_after_drop_in() {
        use figcraft_doc::NodeKind;
        let mut doc = crate::app::sample_doc();
        let page = doc.pages[0];

        // Find an auto-layout container and a plain rectangle that is not already one of its items.
        let page_children: Vec<NodeId> = doc.node(page).map(|n| n.children.clone()).unwrap_or_default();
        let container = page_children.iter().copied()
            .find(|id| doc.node(*id).is_some_and(|n| n.layout.is_some()))
            .expect("sample_doc has an auto-layout frame");
        let mover = page_children.iter().copied()
            .find(|id| *id != container && matches!(doc.node(*id).map(|n| &n.kind), Some(NodeKind::Rectangle { .. })))
            .expect("sample_doc has a plain rectangle to drag in");
        assert_eq!(affected_root(&doc, mover), mover, "a loose rectangle has no reflowing ancestor");

        // The engine reparents the dragged layer into the flow before the canvas builds its patch.
        doc.reparent(mover, container, None).expect("reparent into the flow");
        assert_eq!(doc.node(mover).and_then(|n| n.parent), Some(container), "the layer now lives in the flow");
        assert_eq!(affected_root(&doc, mover), container,
            "after drop-in the patch must lift to the whole flow, covering every reflowed sibling");
    }
}

/// 画叠加层：标尺 + （后续）选中框/手柄/参考线/标签。
///
/// 抽成公开的入口，`show()` 与无头预览用同一份代码，保证两边完全一致。
pub(crate) fn draw_overlays(app: &mut CanvasApp, painter: &mut Painter, t: &Tokens) {
    // 粘性悬停：与 `show()` 里那次调用一致（无头预览走这条路径，也保证 labels 正确）。
    handles::update_hover(app);
    // 文档/页面/选择换掉时撤销进行中的间距/重排手势。
    selection_spacing::sync(app);
    // 只跳过「画布不可见」。标尺开关只影响标尺本身（原实现 `overlays.rs` 里
    // 也是各自判断 `view.rulers`），不能因为关掉标尺就丢掉选中框/参考线。
    if !painter.clip.is_positive() {
        return;
    }
    let (doc, sel) = match app.session.ds() {
        Ok(ds) => (ds.doc.clone(), ds.selection.clone()),
        Err(_) => (Arc::new(Document::new("empty")), Vec::new()),
    };
    // 叠加层（选中框/手柄/尺寸/框选/参考线）——与原实现同一份画法。
    // `overlays` 同时返回每个顶层帧/section 名称标签的矩形；写入 `tool.labels`，
    // 引擎 `label_at` 据此在点击标签时选中/拖拽对应容器（照搬 `ui-egui`）。
    app.session.tool.labels = overlays::overlays(app, painter, t);
    // A3: comment pins, over the overlays (egui `comments::draw` after `overlays()`).
    crate::comments::draw(app, painter, t);
    // 注解 callout：egui 在 `overlays()` 之后、`handles::draw()` 之前画（Inspect 标签才画）。
    dev::draw_annotations(app, painter, t);
    // 自动布局的 padding/gap 手柄（原 `show()` 里 `crate::handles::draw` 就在 overlays 之后）。
    handles::draw(app, painter, app.pointer);
    // 多选间距/重排手柄（原 `canvas.rs` 在 `handles::draw` 之后调 `selection_spacing::draw`）。
    selection_spacing::draw(app, painter);
    // 标尺（View ▸ Rulers）。
    if app.session.ds().map(|d| d.view.rulers).unwrap_or(true) {
        rulers::rulers(app, painter, t, doc.as_ref(), &sel);
    }
}

/// 贴图的屏幕矩形；原生密度的渲染对齐到设备像素，避免线性过滤把相邻纹素混在一起。
pub(crate) fn presented_image_rect(tl: Pos2, view: &View, zoom: f64, ppp: f32, pixel_preview: bool) -> Rect {
    let w = (f64::from(view.width) / view.scale * zoom) as f32;
    let h = (f64::from(view.height) / view.scale * zoom) as f32;
    let native_density = !pixel_preview
        && ppp.is_finite()
        && ppp > 0.0
        && zoom.is_finite()
        && view.scale.is_finite()
        && view.scale > 0.0
        && (view.scale - zoom * f64::from(ppp)).abs() <= view.scale.abs().max(1.0) * 1e-9;
    let min = if native_density { pos2((tl.x * ppp).round() / ppp, (tl.y * ppp).round() / ppp) } else { tl };
    Rect::from_min_size(min, vec2(w, h))
}

/// 纹理边长上限（与 egui 的 `max_texture_side` 保守值一致）。
/// 贴图单边上限：egui/GPUI 的 wgpu 后端申请的是 `max_texture_dimension_2d: 8192`，
/// 超过就创不出纹理（旧值 16384 只会在 8K 屏上白白多分配一倍内存）。
pub const MAX_TEXTURE_SIDE: u32 = 8_192;
/// 一整页缓冲的像素预算（12MP ≈ 48MB/张）。超了就降渲染分辨率、显示时拉伸：
/// 一页缓冲在链路里同时存在好几份（Retained、转换副本、GPUI 纹理），没有上限时峰值能到 GB。
pub const MAX_CANVAS_PIXELS: u64 = 12_000_000;
