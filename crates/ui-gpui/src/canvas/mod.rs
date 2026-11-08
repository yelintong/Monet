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

pub use geom::*;
pub use painter::Painter;

use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::{View, Work};
use gpui::RenderImage;

use crate::app::CanvasApp;
use crate::theme::{Tokens, doc_c32};
use render::JobKey;
use render_worker::Worker;

/// 拖动中的图层：从渲染里裁下来的那一块，画布自己画它。
///
/// 贴图那边用 `Options::skip` 把这几层去掉，这一块图就顶替它们——所以选框（叠加层现算）
/// 和图层的像素是**同一帧**动，不再慢一个渲染往返。和 Figma 拖动时移动图层位图的道理一样。
pub struct Live {
    pub image: Arc<RenderImage>,
    /// 这块图覆盖的页面矩形（`x0 y0 x1 y1`，已裁到渲染范围之内）。
    pub page: [f64; 4],
    /// 抓取那一刻包围盒的左上角：画的时候按之后移动了多少偏移。
    pub at: [f64; 2],
    /// 抓的是这几层（拖拽中换了一组就作废重抓）。
    pub ids: Vec<NodeId>,
    /// 这几层最后一次的包围盒（页面坐标）：松手后还要用它把替身画到原处，直到新贴图到位。
    pub bounds: [f64; 4],
    /// 替身是**每帧重抓**的（拉伸/旋转）：图层自己的像素在变，位图不能靠平移跟。
    /// 这种替身按它抓下来的那一块就位，不做「当前包围盒 − 抓取包围盒」的平移——
    /// 否则上一帧的旧图会被挪到这一帧的新位置上，看着就是跑偏。
    pub reshape: bool,
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
    /// 当前这张贴图**没有**画哪些图层。只有这张贴图确实让开了拖动中的图层，画布才画那一块：
    /// 否则贴图里还留着它们在旧位置的一份，叠上去就是重影。
    pub tex_skip: Vec<NodeId>,
    /// 拖动中由画布自己画的图层（见 [`Live`]）。
    pub live: Option<Live>,
    /// 最后一次单独重抓替身时请求的矩形（拉伸/旋转每帧一次）：同一个就不必再抓。
    pub(crate) grabbed: Option<([i64; 6], Vec<i64>)>,
    pub(crate) submitted: Option<JobKey>,
    /// 拖动开始时已经提交过一次「让开被拖图层」的整页渲染（拖动期间最多一次）。
    pub(crate) skip_pending: bool,
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
            tex_skip: Vec::new(),
            live: None,
            grabbed: None,
            submitted: None,
            skip_pending: false,
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
    /// 拖动中自己画的图层：贴在整页贴图**之上**、叠加层**之下**（和 Figma 的层次一样）。
    /// 第三项是它被所在 frame 裁掉的那一块（屏幕矩形，画布局部坐标）：替身要贴着移动，
    /// 而裁剪框本身不动，所以裁剪只能画的时候套上去，不能烘进图里。
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

    // 拖动中的图层（照搬 Figma 的办法：贴图让开它们，画布自己画这一块）。
    // 平移：抓一次，之后只换位置（替身和选框天然同一帧）。
    // 拉伸/旋转：图层自己的像素每帧都在变，替身**当帧重抓**（见下面 `redraw` 那一段）。
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
    // 并进 `moving`。替身于是画整个受影响的框——`Options::only` 连同子层、`skip` 连同子层——
    // 父级 hug、兄弟位移都与选框同一帧；否则只有被拉的那一层动，父级和内容要等一次异步渲染（果冻）。
    // 旋转不重排（只改自己的几何），保持只画自己，免得把旋转后溢出框外的部分裁进框里。
    if redraw
        && app.session.tool.live_reflows()
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
    // 只有「换了一组图层」才作废替身；松手后（`moving` 空）留着它，等一张含这些层的新贴图
    // 到位再丢（见文件末尾）——否则图层会从画面上消失一下（闪）。
    if let Some(live) = &app.canvas.live
        && !moving.is_empty()
        && live.ids != moving
    {
        app.canvas.live = None;
    }

    // 自动布局的 padding / spacing 提示带：交给渲染铺（在子层之前）。悬停到哪条就只给哪条。
    // 先刷新粘性悬停（指针还在上一条里就锁定它，离开才重判），再据此出带子。
    handles::update_hover(app);
    let bands = handles::bands_for_render(app);

    // 替身**当帧**抓：拉伸/旋转每帧重抓（几何在变），平移只在第一帧抓一次（之后按位移摆放）。
    // 先抓再请求渲染：`request_render` 看到 `live` 已在，就让贴图跳过这些层——平移的第一步
    // 也不会先出现「图层没动」的一两帧（果冻），替身与选框同一帧、同一份文档。
    //
    // 拉伸/旋转的替身每帧都在变——交给后台线程的话，画出来的永远是上一帧的几何，而选框是这一帧
    // 的，两者差一点点，看起来就是内容贴着外框轻微跑偏（用户实测）。单块光栅只要 1–2ms，
    // 直接在当帧抓。
    if !moving.is_empty()
        && (redraw || app.canvas.live.is_none())
        && let Some(b) = moving_bounds
    {
        let grab = app.session.ds().ok().and_then(|ds| {
            let zoom = ds.view.zoom.max(1e-6);
            // 阴影/描边会超出几何包围盒：和渲染那边一样按屏幕像素留一圈。
            let pad = 32.0 / zoom;
            let ask = render_worker::GrabAsk { page: [b.x0 - pad, b.y0 - pad, b.x1 + pad, b.y1 + pad], at: [b.x0, b.y0], ids: moving.clone() };
            let scale = zoom * f64::from(ppp);
            render_worker::grab_layer(&ds.doc, ds.page, &ask, scale, &bands)
        });
        if let Some(g) = grab {
            let at = g.at;
            if let Some(old) = app.canvas.live.take() {
                app.canvas.retired.push(old.image);
            }
            app.canvas.live = Some(Live { image: g.image, page: g.page, at, ids: moving.clone(), bounds: [b.x0, b.y0, b.x1, b.y1], reshape: redraw });
        }
    }

    render::request_render(app, worker, ppp, MAX_TEXTURE_SIDE, &moving, moving_bounds, redraw, &bands);

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
        app.canvas.tex_skip = done.skip.clone();
        // 拖动图层的那一块（跟着这次渲染一起裁出来的）：下一帧起贴图里让开它们。
        if let Some(grab) = done.grab {
            let at = grab.at;
            if let Some(old) = app.canvas.live.take() {
                app.canvas.retired.push(old.image);
            }
            app.canvas.live = Some(Live {
                image: grab.image,
                page: grab.page,
                at,
                ids: moving.clone(),
                bounds: moving_bounds.map_or([at[0], at[1], at[0], at[1]], |b| [b.x0, b.y0, b.x1, b.y1]),
                reshape: redraw,
            });
        }
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

    // 叠加层（标尺；后续的选中框/手柄/参考线/标签也在这一层）。
    draw_overlays(app, &mut painter, t);

    // 拖动中的图层：那一块图按「当前包围盒 − 抓取时的包围盒」平移。
    // 只有台上这张贴图确实让开了它，才画：贴图里还留着它在旧位置的一份时叠上去就是重影。
    if let (Some(l), Some(b)) = (app.canvas.live.as_mut(), moving_bounds)
        && !l.reshape
    {
        l.bounds = [b.x0, b.y0, b.x1, b.y1];
    }
    // 拖动中的图层按它所在的、裁切内容的 frame 实时裁切（拖出框外被切平；水平方向拖出
    // 超过自身宽度一半则整体弹出，见 `live_clip`）。替身位图本身已是完整图层——`grab_layer`
    // 抓取时临时关掉了祖先 frame 的 `clip`，所以初始帧不会带着 frame 提前裁出的残缺，裁切
    // 一律按**实时位置**现算（移动后立刻跟手）。
    let clip_doc = app.session.ds().ok().map(|ds| ds.doc.clone());
    let live = app.canvas.live.as_ref().and_then(|l| {
        if app.canvas.tex_skip != l.ids || l.ids.is_empty() {
            return None;
        }
        // 平移：整张图按「现在在哪 − 抓的时候在哪」挪。拉伸/旋转：图是这一帧刚重抓的，
        // 就按它自己那一块放（再叠加位移只会把旧几何挪到新位置，看着跑偏）。
        let (dx, dy) = if l.reshape {
            (0.0, 0.0)
        } else {
            let b = l.bounds;
            (b[0] - l.at[0], b[1] - l.at[1])
        };
        let p0 = app.to_screen(l.page[0] + dx, l.page[1] + dy);
        let p1 = app.to_screen(l.page[2] + dx, l.page[3] + dy);
        let r = Rect::from_min_max(p0, p1).translate(-rect.min.to_vec2());
        let clip = clip_doc.as_ref().and_then(|doc| live_clip(doc, &l.ids, !l.reshape)).map(|c| {
            let q0 = app.to_screen(c[0], c[1]);
            let q1 = app.to_screen(c[2], c[3]);
            Rect::from_min_max(q0, q1).translate(-rect.min.to_vec2())
        });
        Some((l.image.clone(), r, clip))
    });

    if std::env::var_os("FIGCRAFT_LIVE_LOG").is_some() {
        let (view, sel) = match app.session.ds() {
            Ok(d) => {
                let sel = d.selection.first().copied().and_then(|id| {
                    let n = d.doc.node(id)?;
                    let b = d.doc.world_bounds(id)?;
                    let p0 = app.to_screen(b.x0, b.y0);
                    let p1 = app.to_screen(b.x1, b.y1);
                    Some((n.name.clone(), p0.x.round(), p0.y.round(), p1.x.round(), p1.y.round()))
                });
                (Some((d.view.zoom, d.view.cx, d.view.cy)), sel)
            }
            Err(_) => (None, None),
        };
        eprintln!(
            "[live] view={view:?} rect={:?} redraw={redraw} moving={} bounds={:?} sel={sel:?} live={:?} tex_skip={} drawn={}",
            (app.canvas.rect.min.x, app.canvas.rect.min.y),
            moving.len(),
            moving_bounds.map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())),
            app.canvas.live.as_ref().map(|l| (l.page, l.at, l.reshape)),
            app.canvas.tex_skip.len(),
            live.is_some(),
        );
    }

    // 松手后不立刻丢掉替身：等一张**含图层**的新贴图到位再丢，否则中间那十几毫秒
    // 图层会从画面上消失一下。
    if moving.is_empty() && app.canvas.tex_skip.is_empty() {
        app.canvas.live = None;
    }

    let retired = std::mem::take(&mut app.canvas.retired);
    CanvasOutput { image, live, painter, bg, in_flight, retired }
}

/// 拉伸 `id` 时，它的父级与兄弟也会跟着变：返回**最小的那个把它们全包住**的图层——从 `id`
/// 往上爬，只要父级是自动布局（会重排子层）或跟随子级定尺寸的组（`bounds_follow_children`）
/// 就继续爬。替身按这个框抓（`only`/`skip` 都带上整棵子树），父级 hug 与兄弟位移就和选框
/// 同一帧，不会慢一次渲染。旋转不走这里（它只改自己的几何，不该把框外的部分裁进来）。
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
        doc.node(frame).and_then(|n| n.children.first().copied()).expect("Blue")
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
