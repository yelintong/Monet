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
/// Motion 模式的画布叠加层（运动路径 + 变换手柄，desigma `motion/inspector/overlay.rs`）。
pub mod motion_overlay;
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
use render_worker::{Grab, GrabAsk, Renders, Worker};

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
    /// 台上这张贴图对应的文档：叠加层用它，选框才与画面同帧。
    pub tex_doc: Option<Arc<Document>>,
    /// 本帧视口对应的渲染单元清单（接口层接线，暂不驱动渲染）。
    pub units: Vec<tiles::UnitKey>,
    /// 覆盖整页的低分辨率兜底图（见 [`render_worker::Overview`]），铺在清晰贴图**下面**。
    /// 平移露出新区域时，清晰贴图还没到的那一条立刻有内容，看不见"正在加载"。
    pub overview: Option<(Arc<RenderImage>, View)>,
    /// 兜底图描述的是哪一版内容（`doc 指针 / 页面 / 字体代次`）。
    pub overview_of: Option<(usize, NodeId, u64)>,
    pub(crate) submitted: Option<JobKey>,
    /// 上一帧是否在拖动：松手那一帧用它作废渲染指纹，保证最终态一定提交。
    pub(crate) dragging: bool,
    /// 拖动中被拖层的透明底替身：底图跳过这些层，它按当前位置的偏移贴上去。
    pub(crate) drag_grab: Option<Grab>,
    /// 拖动中盖在整页贴图之上的局部场景补丁（见 [`Live`]）。
    pub live: Option<Live>,
    /// 本次拖动开始时的包围盒：补丁每帧都要盖住它，旧位置才不会留在冻结底图上成残影。
    /// 与 `Live` 解耦 —— 拖进容器 / 自动布局重排时 `moving` 会被 `affected_root` 提升，
    /// `live` 因此重建；只把起点存在 `Live.start` 里，那一刻起点就丢了。
    pub(crate) drag_start: Option<[f64; 4]>,
    /// UI 线程自己的材质合成器，供当帧同步的局部渲染用（不与渲染线程抢设备）。
    /// 生产拖动走 worker 的 `GrabAsk` 替身，不占用它；只有文档里真的绑了外置材质
    /// 才会惰性创建 wgpu 设备。
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
            tex_doc: None,
            units: Vec::new(),
            overview: None,
            overview_of: None,
            submitted: None,
            dragging: false,
            drag_grab: None,
            live: None,
            drag_start: None,
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
    /// 拖动中被拖层的透明底替身（见 `CanvasState.drag_grab`）：贴在整页贴图**之上**、
    /// 叠加层**之下**，底图已跳过这些层。第三项是可选裁剪矩形，恒为 `None` —— 容器裁剪
    /// 由渲染器抓替身时按当前文档现算，画布不再手工套裁剪框。
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
pub fn show(app: &mut CanvasApp, worker: &mut Worker, ppp: f32, t: &Tokens, renaming: Option<NodeId>) -> CanvasOutput {
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

    // 本帧渲染单元清单（接口层接线）：按页面坐标的稳定网格 + 缩放档位生成，
    // 让「哪些单元需要重跑」在画布状态里可见；渲染路径暂不消费它。
    // 复用 CanvasState.units 的 Vec：每帧 clear + extend，不再整表重新分配。
    // 身份里带当前页面，同一文档不同页的相同视口不会共用单元。
    let units_input = app.session.ds().ok().map(|ds| {
        let (ox, oy) = ds.view.origin();
        let z = ds.view.zoom.max(1e-9);
        let w = f64::from(rect.width()) / z;
        let h = f64::from(rect.height()) / z;
        let scale = z * f64::from(ppp);
        // 这里传的是 Arc 身份，不是内容版本：doc_mut 在同一个手势里原地改文档时指针不变。
        // 当前渲染路径不消费 units，所以不会漏刷新；接线前必须换成 Document 暴露的
        // 内容 revision（见 docs/canvas-pipeline-plan.md §7.2）。
        let input = tiles::view_units::ViewInput {
            doc_identity: std::sync::Arc::as_ptr(&ds.doc) as u64,
            page: ds.page.0,
            params: 0,
            size: (rect.width() as u32, rect.height() as u32),
            fonts: 0,
            side: tiles::TILE_SIDE,
        };
        ([ox, oy, ox + w, oy + h], scale, input)
    });
    if let Some((view, scale, input)) = units_input {
        tiles::view_units::for_view_into(view, scale, input, &mut app.canvas.units);
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
        // 新一次拖动才作废起点；同一次拖动里 `moving` 怎么变都保留（见 `drag_start_rect`）。
        if !was_dragging {
            app.canvas.drag_start = None;
        }
        let changed = app.canvas.live.as_ref().is_some_and(|l| l.ids != moving);
        if !was_dragging || changed {
            // 换下来的补丁要显式进 retired，否则一直占着 GPUI 的 sprite atlas。
            if let Some(old) = app.canvas.live.take() {
                app.canvas.retired.push(old.image);
            }
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
        // 起点按「整次拖动」记，不随 `moving` 集合变化：拖进容器时 `dragging_into_flow`
        // 把 moving 提升成容器根，`live` 会重建；若这里回退到当前包围盒，补丁就不再
        // 覆盖拖出位置，冻结底图上那份旧影像就成了残影。
        let start = drag_start_rect(app.canvas.drag_start, b, !was_dragging);
        app.canvas.drag_start = Some(start);
        if let Some((image, page)) = patch_for(app, ppp, &moving, start, b, &bands) {
            if let Some(old) = app.canvas.live.take() {
                app.canvas.retired.push(old.image);
            }
            app.canvas.live = Some(Live { image, page, start, ids: moving.clone(), release_seq: None });
        }
    }

    // 拖动期间底图完全冻结（render::request_render 见 moving 非空直接返回）；
    // `grab_ask` 恒为 None —— 被拖层由上面这张当帧同步补丁提供，不再抓一次性替身位图。
    let _ = (moving_bounds, redraw);
    render::request_render(app, worker, ppp, MAX_TEXTURE_SIDE, &moving, None, &bands);
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
        // 抓到替身就换上（旧的交给视图层 drop，别让 GPUI 的 sprite atlas 越堆越多）。
        if let Some(g) = done.grab
            && let Some(old) = app.canvas.drag_grab.replace(g)
        {
            app.canvas.retired.push(old.image);
        }
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
        app.canvas.tex_doc = Some(done.doc.clone());
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

    // 把上次渲染重投影到当前视图。贴图是否属于当前文件/页面，与叠加层共用同一道判定：
    // 文档或页面切换后旧贴图还在台上，但不能让它的几何流到叠加层。
    let mut image = None;
    if let (Some(tex), Some(v), true) = (app.canvas.texture.as_ref(), app.canvas.tex_view.as_ref(), texture_is_current(&app.canvas, shown)) {
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
    draw_overlays(app, &mut painter, t, renaming);

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
        app.canvas.drag_start = None;
    }

    let retired = std::mem::take(&mut app.canvas.retired);
    CanvasOutput { image, overview, live, painter, bg, in_flight, retired }
}


/// 台上贴图是否属于当前显示的文件/页面。图片显示与叠加层必须共用这一判定。
fn texture_is_current(c: &CanvasState, shown: Option<(u64, NodeId)>) -> bool {
    shown.is_some() && c.tex_of == shown
}

/// 叠加层能用的台上贴图文档：贴图与当前显示的文件/页面一致时，才是同帧同基准。
/// 没有清空 `tex_doc` / `tex_of` 的重置路径，文档或页面切换后靠这道闸门挡住上一个文档的几何。
pub(crate) fn on_screen_tex_doc(c: &CanvasState, shown: Option<(u64, NodeId)>) -> Option<Arc<Document>> {
    if texture_is_current(c, shown) {
        c.tex_doc.clone()
    } else {
        None
    }
}

/// 叠加层能采用的台上贴图文档：画面由「本帧显示文档」或「当帧同步补丁」提供时，
/// 台上那张整页贴图是冻结 / 上一帧的旧结果，不能当几何基准。
/// `frame_doc_active` = Motion 预览或播放（`app.display_doc` 有值）；
/// `live` = 拖动补丁还在台上（`app.canvas.live.is_some()`，含松手后补丁未撤）。
/// 任一为真就忽略贴图文档，改由 `overlay_doc` 取 display → source。
pub(crate) fn overlay_texture_doc(
    c: &CanvasState,
    shown: Option<(u64, NodeId)>,
    frame_doc_active: bool,
    live: bool,
) -> Option<Arc<Document>> {
    if frame_doc_active || live {
        None
    } else {
        on_screen_tex_doc(c, shown)
    }
}

/// 叠加层读哪份文档：台上贴图的 > 本帧显示文档 > 源文档。
/// 抽成纯函数，回归「选框与画面同帧」这条契约。
pub(crate) fn overlay_doc(
    tex: Option<Arc<Document>>,
    display: Option<Arc<Document>>,
    source: &Arc<Document>,
) -> Arc<Document> {
    tex.or(display).unwrap_or_else(|| source.clone())
}

/// 画叠加层：标尺 + （后续）选中框/手柄/参考线/标签。
///
/// 抽成公开的入口，`show()` 与无头预览用同一份代码，保证两边完全一致。
pub(crate) fn draw_overlays(app: &mut CanvasApp, painter: &mut Painter, t: &Tokens, renaming: Option<NodeId>) {
    // 粘性悬停：与 `show()` 里那次调用一致（无头预览走这条路径，也保证 labels 正确）。
    handles::update_hover(app);
    // 文档/页面/选择换掉时撤销进行中的间距/重排手势。
    selection_spacing::sync(app);
    // 只跳过「画布不可见」。标尺开关只影响标尺本身（原实现 `overlays.rs` 里
    // 也是各自判断 `view.rulers`），不能因为关掉标尺就丢掉选中框/参考线。
    if !painter.clip.is_positive() {
        return;
    }
    // 叠加层与画面必须同基准，但「画面由谁提供」分三种情况：
    // · 静态、没有当帧补丁：画面就是台上那张整页贴图，用它的文档（与源文档同源）。
    // · 拖动中 / 松手后补丁未撤：`request_render` 把整页贴图完全冻结，画面由 `patch_for`
    //   每帧当帧重绘。此时若还绑 `tex_doc`，选框会钉死在拖动开始那一版，内容却每帧
    //   在动 —— 实测「框不会动、内容会动」正是这里。
    // · Motion 预览 / 播放：`display_doc` 每帧新建，贴图是后台上一帧，绑 `tex_doc`
    //   会让选框慢一帧；渲染请求、补丁与其余叠加层都读本帧文档，选框必须跟它们一致。
    let shown = app.session.ds().map(|ds| (ds.uid, ds.page)).ok();
    let tex_doc = overlay_texture_doc(
        &app.canvas,
        shown,
        app.display_doc.is_some(),
        app.canvas.live.is_some(),
    );
    let display_doc = app.display_doc.clone();
    let (doc, sel) = match app.session.ds() {
        Ok(ds) => (overlay_doc(tex_doc, display_doc, &ds.doc), ds.selection.clone()),
        Err(_) => (Arc::new(Document::new("empty")), Vec::new()),
    };
    // 叠加层（选中框/手柄/尺寸/框选/参考线）——与原实现同一份画法。
    // `overlays` 同时返回每个顶层帧/section 名称标签的矩形；写入 `tool.labels`，
    // 引擎 `label_at` 据此在点击标签时选中/拖拽对应容器（照搬 `ui-egui`）。
    app.session.tool.labels = overlays::overlays(app, painter, t, doc.clone(), renaming);
    // Motion 模式：选中层的运动路径（轨迹曲线）与变换手柄。画在选中框之下、
    // 评论/注解手柄之上（用户 2026-11-12：有动画就显示，播放中隐藏）。
    motion_overlay::draw(app, painter, t);
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
/// 位置吸附设备像素；像素预览或无效 ppp 时原样返回。
/// 与贴图自带的 `view.scale` 无关：缩放中贴图是旧 scale，位置也必须对齐。
fn snap_origin(x: f32, y: f32, ppp: f32, pixel_preview: bool) -> (f32, f32) {
    if pixel_preview || !ppp.is_finite() || ppp <= 0.0 || !x.is_finite() || !y.is_finite() {
        return (x, y);
    }
    ((x * ppp).round() / ppp, (y * ppp).round() / ppp)
}

pub(crate) fn presented_image_rect(tl: Pos2, view: &View, zoom: f64, ppp: f32, pixel_preview: bool) -> Rect {
    let w = (f64::from(view.width) / view.scale * zoom) as f32;
    let h = (f64::from(view.height) / view.scale * zoom) as f32;
    // 位置始终吸附设备像素：缩放过程中贴图自带的 view.scale 还是旧值，若把对齐
    // 绑在「view.scale == zoom * ppp」上，整个手势期间都会退化成半像素定位，表现为
    // 贴图边缘抖动、内容跟不上选框。原生密度只影响清晰度，不影响位置对齐。
    let (x, y) = snap_origin(tl.x, tl.y, ppp, pixel_preview);
    let min = pos2(x, y);
    Rect::from_min_size(min, vec2(w, h))
}

/// 纹理边长上限（与 egui 的 `max_texture_side` 保守值一致）。
/// 贴图单边上限：egui/GPUI 的 wgpu 后端申请的是 `max_texture_dimension_2d: 8192`，
/// 超过就创不出纹理（旧值 16384 只会在 8K 屏上白白多分配一倍内存）。
pub const MAX_TEXTURE_SIDE: u32 = 8_192;
/// 一整页缓冲的像素预算（12MP ≈ 48MB/张）。超了就降渲染分辨率、显示时拉伸：
/// 一页缓冲在链路里同时存在好几份（Retained、转换副本、GPUI 纹理），没有上限时峰值能到 GB。
pub const MAX_CANVAS_PIXELS: u64 = 12_000_000;

/// 一张拖动补丁的像素上限（4MP ≈ 16MB）。补丁每帧重建，拖得很远时矩形会接近整页，
/// 超过就降渲染分辨率：按页面矩形贴上去只是略糊，不会错位。
const MAX_PATCH_PIXELS: f64 = 4_000_000.0;

/// 本帧补丁要盖住的拖动起点：同一次拖动全程保留，哪怕 `moving` 被提升成容器根、
/// `Live` 被重建；只有新手势才用当前包围盒重新记。抽成纯函数便于回归。
fn drag_start_rect(prev: Option<[f64; 4]>, now: figcraft_geom::Rect, new_gesture: bool) -> [f64; 4] {
    let fresh = [now.x0, now.y0, now.x1, now.y1];
    if new_gesture { fresh } else { prev.unwrap_or(fresh) }
}

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


fn patch_for(
    app: &mut CanvasApp,
    ppp: f32,
    moving: &[NodeId],
    start: [f64; 4],
    now: figcraft_geom::Rect,
    bands: &[figcraft_render::Band],
) -> Option<(Arc<RenderImage>, [f64; 4])> {
    let display_doc = app.display_doc.clone();
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
    let doc = display_doc.unwrap_or_else(|| ds.doc.clone());
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

#[cfg(test)]
mod drag_start_tests {
    use super::{drag_start_rect, patch_rect};
    use figcraft_geom::Rect;

    /// 拖进容器时 `moving` 被 `affected_root` 提升成整个容器。起点不能跟着变，
    /// 否则补丁只覆盖容器，拖出位置留在冻结底图上就是「残影」。
    #[test]
    fn start_survives_a_change_of_moving_layers() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let start = drag_start_rect(None, a, true);
        assert_eq!(start, [0.0, 0.0, 10.0, 10.0]);
        let container = Rect::new(100.0, 100.0, 300.0, 300.0);
        assert_eq!(drag_start_rect(Some(start), container, false), start, "同一手势不能丢起点");
        // 新一次拖动才重置。
        assert_eq!(drag_start_rect(Some(start), container, true), [100.0, 100.0, 300.0, 300.0]);
    }

    #[test]
    fn patch_keeps_covering_the_old_position_after_reparent() {
        let start = [0.0, 0.0, 10.0, 10.0];
        let now = Rect::new(200.0, 200.0, 260.0, 260.0);
        let visible = [-1000.0, -1000.0, 1000.0, 1000.0];
        let r = patch_rect(start, now, 4.0, visible).expect("两个位置都要在补丁里");
        assert!(r[0] <= 0.0 && r[1] <= 0.0, "旧位置没被盖住: {r:?}");
        assert!(r[2] >= 260.0 && r[3] >= 260.0, "新位置没被盖住: {r:?}");
        // 起点丢失（回退成当前包围盒）时，旧位置确实落到补丁外 —— 复现那个残影。
        let lost = patch_rect([now.x0, now.y0, now.x1, now.y1], now, 4.0, visible).unwrap();
        assert!(lost[0] > 0.0, "起点丢失时旧位置不再被覆盖");
    }
}

#[cfg(test)]
mod display_doc_tests {
    use super::*;

    fn doc(name: &str) -> Arc<Document> {
        Arc::new(Document::new(name))
    }

    #[test]
    fn overlay_document_prefers_the_on_screen_frame_then_display_then_source() {
        let source = doc("source");
        let display = doc("display");
        let tex = doc("tex");
        assert!(Arc::ptr_eq(&overlay_doc(Some(tex.clone()), Some(display.clone()), &source), &tex));
        assert!(Arc::ptr_eq(&overlay_doc(None, Some(display.clone()), &source), &display));
        assert!(Arc::ptr_eq(&overlay_doc(None, None, &source), &source));
    }

    #[test]
    fn projected_size_follows_the_current_zoom_not_the_texture_scale() {
        // 缩放中贴图 scale 还是 2.0，当前 zoom 已是 1.25：显示尺寸必须按 zoom 换算，
        // 否则贴图会「一半不到就停」。
        let v = figcraft_render::View { origin: (0.0, 0.0), scale: 2.0, width: 1024, height: 768 };
        let r = presented_image_rect(pos2(0.0, 0.0), &v, 1.25, 2.0, false);
        assert!((r.max.x - r.min.x - 640.0).abs() < 0.01, "width {}", r.max.x - r.min.x);
        assert!((r.max.y - r.min.y - 480.0).abs() < 0.01, "height {}", r.max.y - r.min.y);
    }

    #[test]
    fn origin_snapping_does_not_depend_on_the_texture_scale() {
        // 缩放中贴图还是旧 scale；位置吸附必须照常发生。
        assert_eq!(snap_origin(10.3, 20.7, 2.0, false), (10.5, 20.5));
        assert_eq!(snap_origin(10.3, 20.7, 1.0, true), (10.3, 20.7));
        assert_eq!(snap_origin(10.3, 20.7, 0.0, false), (10.3, 20.7));
    }

    #[test]
    fn overlay_texture_document_is_gated_by_the_current_file_and_page() {
        let tex = doc("tex");
        let mut c = CanvasState::default();
        c.tex_doc = Some(tex.clone());
        c.tex_of = Some((7, NodeId(3)));

        // 同一文件同一页面：叠加层可以用台上贴图那份文档。
        let same = on_screen_tex_doc(&c, Some((7, NodeId(3)))).unwrap();
        assert!(Arc::ptr_eq(&same, &tex));
        assert!(texture_is_current(&c, Some((7, NodeId(3)))));

        // 换文件 / 换页面 / 没有当前文档：旧贴图还在台上，但它的几何不能流到叠加层。
        assert!(on_screen_tex_doc(&c, Some((8, NodeId(3)))).is_none());
        assert!(on_screen_tex_doc(&c, Some((7, NodeId(4)))).is_none());
        assert!(on_screen_tex_doc(&c, None).is_none());
        assert!(!texture_is_current(&c, None));

        // 贴图还没上台时（tex_of 为空），即使 shown 也为空也不能误判成「同一帧」。
        c.tex_of = None;
        assert!(on_screen_tex_doc(&c, None).is_none());
    }

    #[test]
    fn overlay_texture_document_is_ignored_while_the_frame_redraws() {
        let tex = doc("stale texture");
        let mut c = CanvasState::default();
        c.tex_doc = Some(tex.clone());
        c.tex_of = Some((7, NodeId(3)));

        // 静态：贴图就是画面，可以用它的文档。
        assert!(overlay_texture_doc(&c, Some((7, NodeId(3))), false, false).is_some());
        // Motion 预览 / 播放：本帧显示文档每帧重建，贴图是后台上一帧。
        assert!(overlay_texture_doc(&c, Some((7, NodeId(3))), true, false).is_none());
        // 拖动（含松手后补丁未撤）：整页贴图冻结，画面由当帧补丁提供。
        assert!(overlay_texture_doc(&c, Some((7, NodeId(3))), false, true).is_none());
        // 两者同时成立也不能回退到旧贴图。
        assert!(overlay_texture_doc(&c, Some((7, NodeId(3))), true, true).is_none());

        // 忽略贴图后，叠加层必须落到本帧显示文档 / 源文档，而不是那张冻结贴图。
        let display = doc("display");
        let source = doc("source");
        let chosen = overlay_doc(
            overlay_texture_doc(&c, Some((7, NodeId(3))), true, true),
            Some(display.clone()),
            &source,
        );
        assert!(Arc::ptr_eq(&chosen, &display));
        let chosen_static = overlay_doc(
            overlay_texture_doc(&c, Some((7, NodeId(3))), false, false),
            None,
            &source,
        );
        assert!(Arc::ptr_eq(&chosen_static, &tex));
    }

    /// P0-6′ 组合矩阵（纯函数层）：状态 × 特效 × 旋转 × 视口缩放 × 贴图 scale。
    /// 这里锁的是同帧契约：文档基准、投影尺寸、设备像素吸附在任意组合下都不变。
    /// 特效的实际像素输出、旋转后的图形是否与选框重合，**仍需实机逐像素比对**，
    /// 本测试不覆盖也不假称覆盖（见 docs/canvas-pipeline-plan.md §7.2）。
    #[test]
    fn p0_6_matrix_keeps_the_same_frame_baseline_in_every_combination() {
        let source = doc("source");
        let preview = doc("preview");
        let tex = doc("tex");
        // 状态：静态编辑 / Motion 预览 / 播放中 / 拖动中。
        // 分别覆盖 overlay_doc 的「源文档 / 显示文档 / 贴图文档」三条路径。
        let states = [("static", false, false), ("preview", true, false), ("playing", true, true), ("dragging", false, true)];
        // 特效档位：无 / 玻璃 / 背景模糊 / 阴影 / 渐变。
        // 特效只改变贴图内容，不改变叠加层该读哪份文档，也不改变投影几何。
        let effects = ["none", "glass", "backdrop-blur", "shadow", "gradient"];
        let rotations = [0.0_f64, 45.0, 90.0, 37.5];
        let zooms = [0.25_f64, 1.0, 4.0];
        let tex_scales = [0.5_f64, 1.0, 2.0, 4.0];
        let mut cases = 0u32;

        for (state, has_display, has_tex) in states {
            for effect in effects {
                for deg in rotations {
                    for zoom in zooms {
                        for tex_scale in tex_scales {
                            cases += 1;
                            let case = format!("{state}/{effect}/{deg}deg/z{zoom}/tex{tex_scale}");
                            let display = has_display.then(|| preview.clone());
                            let tex_doc = has_tex.then(|| tex.clone());
                            let chosen = overlay_doc(tex_doc, display, &source);
                            let expected = if has_tex { &tex } else if has_display { &preview } else { &source };
                            assert!(Arc::ptr_eq(&chosen, expected), "{case}: 文档基准不对");

                            // 旋转后的左上角：吸附必须落在设备像素上，且与特效/状态无关。
                            let rad = deg.to_radians();
                            let x = 100.0 + 30.0 * rad.cos();
                            let y = 100.0 + 30.0 * rad.sin();
                            let (sx, sy) = snap_origin(x as f32, y as f32, 2.0, false);
                            assert_eq!(sx * 2.0, (sx * 2.0).round(), "{case}: x 没吸附到设备像素");
                            assert_eq!(sy * 2.0, (sy * 2.0).round(), "{case}: y 没吸附到设备像素");

                            // 缩放中贴图带旧 scale：显示尺寸按当前 zoom 投影，位置仍与 scale 无关。
                            let view = figcraft_render::View { origin: (0.0, 0.0), scale: tex_scale, width: 1024, height: 768 };
                            let r = presented_image_rect(pos2(x as f32, y as f32), &view, zoom, 2.0, false);
                            let want_w = 1024.0 / tex_scale as f32 * zoom as f32;
                            let want_h = 768.0 / tex_scale as f32 * zoom as f32;
                            assert!((r.max.x - r.min.x - want_w).abs() < 0.01, "{case}: 宽度 {} 应为 {want_w}", r.max.x - r.min.x);
                            assert!((r.max.y - r.min.y - want_h).abs() < 0.01, "{case}: 高度 {} 应为 {want_h}", r.max.y - r.min.y);
                            assert_eq!(r.min.x, sx, "{case}: 位置吸附被贴图 scale 带偏");
                            assert_eq!(r.min.y, sy, "{case}: 位置吸附被贴图 scale 带偏");
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 4 * 5 * 4 * 3 * 4);
    }
}
