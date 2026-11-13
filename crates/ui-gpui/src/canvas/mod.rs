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
            overview: None,
            overview_of: None,
            submitted: None,
            dragging: false,
            drag_grab: None,
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

    // 拖动**不再冻结底图**，也不再生成局部补丁：照常提交整页渲染，谁先完成谁上台
    // （`latest wins`）。这是原工程的模型——内容最多比选框晚一两帧，但屏幕上永远没有
    // 「必须擦掉的历史像素」，残影因此没有产生的余地。曾经的「冻结 + 当帧同步补丁」
    // 是为了消灭那一两帧滞后，代价是把正确性押在「每帧算准覆盖范围」上：容器 Hug 变宽
    // 又缩回、`moving` 集合中途变化、`start` 被重置，任一步算错就留下半块残影。
    // 拖动状态只用来标记「刚松手」：底图不再冻结，松手后只需作废一次指纹，
    // 确保最终态一定提交（拖动中更早的请求可能已被更新的覆盖）。
    // 被拖的层：底图要跳过它们，由抓到的那张透明底替身单独提供内容。
    let drag_ids: Vec<NodeId> = app.session.tool.live_layers().map(|(ids, _)| ids).unwrap_or_default();
    let dragging_now = !drag_ids.is_empty();
    let started = !app.canvas.dragging && dragging_now;
    let released = app.canvas.dragging && !dragging_now;
    app.canvas.dragging = dragging_now;
    if started || released {
        if let Some(old) = app.canvas.drag_grab.take() {
            app.canvas.retired.push(old.image);
        }
    }
    if released {
        app.canvas.submitted = None;
    }

    // 自动布局的 padding / spacing 提示带：交给渲染铺（在子层之前）。悬停到哪条就只给哪条。
    // 先刷新粘性悬停（指针还在上一条里就锁定它，离开才重判），再据此出带子。
    handles::update_hover(app);
    let bands = handles::bands_for_render(app);

    // 刚进入拖动：请 worker 顺手抓一张被拖层的**透明底**替身（只抓一次，之后复用按偏移平移）。
    // 替身由同一次渲染顺带裁出，所以它和底图来自同一版文档，不会错位。
    let grab_ask = started
        .then(|| {
            app.session.ds().ok().and_then(|ds| ds.doc.bounds_of(&drag_ids)).map(|b| {
                // 阴影/模糊会越过包围盒：留 32 页面单位，替身才不会被切掉一圈。
                let pad = 32.0;
                GrabAsk { page: [b.x0 - pad, b.y0 - pad, b.x1 + pad, b.y1 + pad], at: [b.x0, b.y0], ids: drag_ids.clone() }
            })
        })
        .flatten();

    // 拖动期间照常请求整页（`latest wins`）：整页跳过被拖层，替身负责它。
    render::request_render(app, worker, ppp, MAX_TEXTURE_SIDE, &drag_ids, grab_ask, &bands);

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
    draw_overlays(app, &mut painter, t);

    // 替身：底图已经跳过了被拖层，它按「当前包围盒 − 抓取时的左上角」的偏移贴上去。
    // 拖动中只改这个偏移，不再产生任何光栅工作。
    let live = {
        let grab = app.canvas.drag_grab.as_ref();
        let bounds = if dragging_now { app.session.ds().ok().and_then(|ds| ds.doc.bounds_of(&drag_ids)) } else { None };
        match (grab, bounds) {
            (Some(g), Some(b)) => {
                let (dx, dy) = (b.x0 - g.at[0], b.y0 - g.at[1]);
                let p0 = app.to_screen(g.page[0] + dx, g.page[1] + dy);
                let p1 = app.to_screen(g.page[2] + dx, g.page[3] + dy);
                Some((g.image.clone(), Rect::from_min_max(p0, p1).translate(-rect.min.to_vec2()), None))
            }
            _ => None,
        }
    };

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
    // 选框跟台上那张贴图用同一份文档：贴图是后台某一帧的结果，叠加层若读当帧文档
    // 就会「框在跑、内容也在跑」。没有上台贴图时退回本帧显示文档，再到源文档。
    let shown = app.session.ds().map(|ds| (ds.uid, ds.page)).ok();
    let tex_doc = on_screen_tex_doc(&app.canvas, shown);
    let display_doc = app.display_doc.clone();
    let (doc, sel) = match app.session.ds() {
        Ok(ds) => (overlay_doc(tex_doc, display_doc, &ds.doc), ds.selection.clone()),
        Err(_) => (Arc::new(Document::new("empty")), Vec::new()),
    };
    // 叠加层（选中框/手柄/尺寸/框选/参考线）——与原实现同一份画法。
    // `overlays` 同时返回每个顶层帧/section 名称标签的矩形；写入 `tool.labels`，
    // 引擎 `label_at` 据此在点击标签时选中/拖拽对应容器（照搬 `ui-egui`）。
    app.session.tool.labels = overlays::overlays(app, painter, t, doc.clone());
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
