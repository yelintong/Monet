//! 画布渲染离开 UI 线程（照搬 `ui-egui/src/canvas/render_worker.rs`）。
//!
//! 语义与原文一致：
//! - 最新请求获胜（队列里更旧的直接跳过）；
//! - UI 把每张完成的图**重投影**到当前视图显示，直到下一张到达；
//! - 渲染器保留上一张图（[`figcraft_render::Retained`]）：平移只补未覆盖的条带，
//!   编辑只重画变化的图层区域；
//! - 缩放时无法复用旧图，先出一张粗预览（每边少 [`PREVIEW_FACTOR`] 像素），再精修；
//! - 后台线程按 `PREVIEW_AFTER_MS` 用看门狗补预览，精修图先到就丢弃预览。
//!
//! 与原文的差异只有两处（GPUI 侧无可避免）：
//! 1. `egui::ColorImage` → `Arc<RenderImage>`（GPUI 的贴图来源）；
//! 2. `ctx.request_repaint_of(viewport)` → 唤醒回调（`wake`），由视图层据 `in_flight()` 轮询。
//!
//! # GPU 后端（docs §18）
//!
//! `--features gpu` 编译且 `FIGCRAFT_GPU=1` 时，**精修**那一帧先试 GPU：
//! `gpu::scene_of` 把文档压成纯色/圆角矩形；压不出来（渐变/图片/文字/描边/效果/混合/裁剪/
//! 旋转/半透明…）就原样回退到 `Retained`（CPU）。回退是**整帧**的，不做「一半 GPU 一半 CPU」
//! ——两层图叠起来 z 序会错。粗预览与替身抓取仍走 CPU。

#![allow(dead_code)]

use figcraft_i18n::tr;
use std::sync::Arc;

use figcraft_color::ColorProfile;
use figcraft_doc::{Document, NodeId};
use figcraft_render::{Pixmap, Retained, View, Work};
use gpui::RenderImage;

/// 预览每边少这么多倍像素。
pub const PREVIEW_FACTOR: f64 = 4.0;
/// 拖动替身的像素预算（4MP ≈ 16MB）：一张替身图不大，但大 frame 高缩放时会到 8192²。
const MAX_GRAB_PIXELS: f64 = 4_000_000.0;
/// 快于此（约一帧）的完整渲染不需要预览。
pub const PREVIEW_AFTER_MS: f64 = 12.0;

#[derive(Clone)]
pub struct Job {
    pub doc: Arc<Document>,
    pub page: NodeId,
    pub view: View,
    pub outline: bool,
    pub outline_extras: figcraft_render::Outline,
    pub outlined: Vec<NodeId>,
    pub skip: Vec<NodeId>,
    pub solo: Option<NodeId>,
    pub proof: bool,
    pub dots: Option<figcraft_render::Dots>,
    /// 这次渲染顺带从**同一次**光栅化里裁一块出来（拖动中的图层：见 [`Grab`]）。
    pub grab: Option<GrabAsk>,
    /// 画布提示带（padding / spacing）：铺在所属图层的子层之前。
    pub bands: Vec<figcraft_render::Band>,
    pub uid: u64,
    pub zoom_preview: bool,
    pub at: std::time::Instant,
}

/// 要从这次渲染里裁出来的区域（页面坐标）。
#[derive(Clone, Debug, PartialEq)]
pub struct GrabAsk {
    /// 图层包围盒（页面坐标，`x0 y0 x1 y1`），已按屏幕像素留出阴影余量。
    pub page: [f64; 4],
    /// 抓取那一刻包围盒的左上角：画的时候按之后移动了多少偏移。
    pub at: [f64; 2],
    /// 单独渲染这些图层（透明底），拖动时画布拿它当它们的实时替身。渲染照常从页面往下走，
    /// 所以每一层都还带着自己所在 frame 的裁剪。
    pub ids: Vec<NodeId>,
}

/// 从一次渲染里裁出来的一块图，给画布在拖动时实时画图层用。
pub struct Grab {
    pub image: Arc<RenderImage>,
    /// 这块图覆盖的页面矩形（已裁到渲染范围之内）。
    pub page: [f64; 4],
    /// 抓取那一刻包围盒的左上角（页面坐标）。
    pub at: [f64; 2],
}

pub struct Done {
    /// 可直接上传为纹理的整页图像。
    pub image: Arc<RenderImage>,
    /// 该图显示的视图（请求视图向外吸附到整设备像素后的结果）。
    pub view: View,
    /// 这张图**没有**画哪些图层（画布自己画了它们）。
    pub skip: Vec<NodeId>,
    /// 这次渲染顺带裁出来的图层块（请求了才给）。
    pub grab: Option<Grab>,
    pub ms: f64,
    pub work: Work,
    pub uid: u64,
    pub page: NodeId,
    /// 粗预览，稍后由精修图替换（除非被更新的请求取代）。
    pub coarse: bool,
    pub at: std::time::Instant,
    /// 它回答的请求序号（由 worker 按顺序编号）。
    pub seq: u64,
}

/// 按像素走一遍。用 `align_to_mut` 切成 `[u8; 4]`：`u8` 到 `[u8; 4]` 只要求对齐 1，
/// 每像素不再带边界检查，LLVM 能把这一趟向量化（440 万像素的整页贴图差好几毫秒）。
fn pixels_mut(data: &mut [u8]) -> impl Iterator<Item = &mut [u8; 4]> {
    // SAFETY: `[u8; 4]` 的对齐要求（1）不比 `u8` 严，切片可安全地重新解释。
    let (_, body, _) = unsafe { data.align_to_mut::<[u8; 4]>() };
    body.iter_mut()
}

/// 直通 RGBA → GPUI 纹理要的 **BGRA**（只换 R/B 两个通道）。
pub fn rgba_to_bgra(data: &mut [u8]) {
    for px in pixels_mut(data) {
        px.swap(0, 2);
    }
}

/// tiny-skia 的**预乘** RGBA → GPUI 的**直通** BGRA，反预乘与换序**一趟走完**。
///
/// 整页贴图有 2500×1760 ≈ 440 万像素（17MB），多走一趟就是好几毫秒：拖拽时这点时间直接
/// 变成「内容比选框慢一拍」。
pub fn premul_rgba_to_bgra(data: &mut [u8]) {
    // 这一趟是纯内存搬运：读一整页、写一整页。单核跑不满内存带宽（实测 1080p 要 3.6ms，
    // 占一帧里最贵的一项 CPU 工作），按行切块并行后掉到 1ms 出头。小块不值得开线程。
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4);
    if threads == 1 || data.len() < (1 << 20) {
        premul_rgba_to_bgra_run(data);
        return;
    }
    // 每块的字节数必须是 4 的倍数，否则一个像素会被切到两块里。
    let chunk = (data.len() / threads).div_ceil(4) * 4;
    std::thread::scope(|s| {
        for part in data.chunks_mut(chunk) {
            s.spawn(move || premul_rgba_to_bgra_run(part));
        }
    });
}

fn premul_rgba_to_bgra_run(data: &mut [u8]) {
    for px in pixels_mut(data) {
        match px[3] {
            // 页面底色不透明，整页几乎都走这一支。
            255 => px.swap(0, 2),
            0 => *px = [0, 0, 0, 0],
            a => {
                let a32 = u32::from(a);
                let un = |c: u8| ((u32::from(c) * 255 + a32 / 2) / a32).min(255) as u8;
                let (r, g, b) = (un(px[0]), un(px[1]), un(px[2]));
                *px = [b, g, r, a];
            }
        }
    }
}

/// 把 RGBA 字节包成 GPUI 的 `RenderImage`。
///
/// 通道顺序：tiny-skia 的 pixmap 内部按**预乘** RGBA 存放（`figcraft_render` 文档也这么写），
/// GPUI 的纹理要的是 **BGRA**（`gpui::assets` "in BGRA format"，`elements/img.rs` 解码位图后
/// 也只做 `pixel.swap(0, 2)`，SVG 那条路还会反预乘）。早先这里把 alpha 又乘了一次
/// （当时以为 GPUI 要预乘），整页不透明时看不出来，半透明内容却会被压暗一次。
pub fn wrap_bgra(w: u32, h: u32, buf: Vec<u8>) -> Arc<RenderImage> {
    use image::{Frame, ImageBuffer, Rgba};
    let img = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, buf).unwrap_or_else(|| ImageBuffer::new(w, h));
    Arc::new(RenderImage::new(vec![Frame::new(img)]))
}

/// 直通 RGBA 的字节 → `RenderImage`（换序 + 包装）。
pub fn wrap_rgba(w: u32, h: u32, mut buf: Vec<u8>) -> Arc<RenderImage> {
    rgba_to_bgra(&mut buf);
    wrap_bgra(w, h, buf)
}

/// 预乘 RGBA → 直通 RGBA（就地把每通道除以 alpha）。
pub fn demultiply(data: &mut [u8]) {
    for px in data.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// `profile` 下的文档渲染成屏幕图像（sRGB）。
pub fn screen_image(pix: &Pixmap, profile: ColorProfile) -> Arc<RenderImage> {
    let (w, h) = (pix.width(), pix.height());
    let mut data = pix.data().to_vec();
    demultiply(&mut data);
    if !profile.is_srgb() {
        profile.conversion_to(ColorProfile::Srgb).apply_pixels(&mut data);
    }
    wrap_rgba(w, h, data)
}

/// 软打样：sRGB 显示不了的颜色上打灰色斜纹。
fn mark_outside_srgb(data: &mut [u8], w: u32, clipped: &[bool]) {
    let w = usize::try_from(w.max(1)).unwrap_or(1);
    for (i, (px, out)) in data.as_chunks_mut::<4>().0.iter_mut().zip(clipped).enumerate() {
        if *out && ((i % w) + (i / w)) % 8 < 4 {
            px.copy_from_slice(&[128, 128, 128, 255]);
        }
    }
}

/// 是否把每次渲染的耗时打到 stderr（`FIGCRAFT_RENDER_LOG=1`）。画布调优用，
/// 默认关闭：一次拖拽就几十行。
pub(crate) fn render_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FIGCRAFT_RENDER_LOG").is_some())
}

/// 把 `ask` 的那些图层**单独**渲染成一张透明底的图（拖动时画布拿它当图层的实时替身）。
///
/// 不能从整页贴图上裁：整页是不透明的，裁下来的块带着周围一片背景，拖动时就像把周围
/// 也一起拽走了（用户实测到的就是这一条）。单独渲染只留这些层，周围是透明的，
/// 叠在背景上就是它本来的样子；阴影、描边、圆角也都跟着一起出来。
///
/// 走的是照常从页面往下画的那条路（`Options::only`），不是绕开祖先单独画一个节点：
/// 图层被拖到 frame 边上时，它该被那一半裁掉——用户量过这个行为（半出去的一半消失，
/// 整块拖出去后才不被裁）。`solo` 会绕开裁剪，所以这里不用它。
pub(super) fn grab_layer(doc: &Arc<Document>, page: NodeId, ask: &GrabAsk, scale: f64, bands: &[figcraft_render::Band]) -> Option<Grab> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    // 替身面积预算（4MP ≈ 16MB）：大 frame 在高缩放下能到 8192²（268MB），一张就够呛。
    // 超了就降替身分辨率——它按页面矩形贴上去，分辨率低只是略糊，不会错位。
    let mut scale = scale;
    let mut w = (((ask.page[2] - ask.page[0]) * scale).ceil().max(1.0) as u32).min(8_192);
    let mut h = (((ask.page[3] - ask.page[1]) * scale).ceil().max(1.0) as u32).min(8_192);
    let area = f64::from(w) * f64::from(h);
    if area > MAX_GRAB_PIXELS {
        scale *= (MAX_GRAB_PIXELS / area).sqrt();
        w = (((ask.page[2] - ask.page[0]) * scale).ceil().max(1.0) as u32).min(8_192);
        h = (((ask.page[3] - ask.page[1]) * scale).ceil().max(1.0) as u32).min(8_192);
    }
    let view = View { origin: (ask.page[0], ask.page[1]), scale, width: w, height: h };
    let opts = figcraft_render::Options {
        // `None` 会被页面的底色顶上：这里要的是透明底，只能显式给一个全透明的填充。
        background: Some(figcraft_color::Color::TRANSPARENT),
        only: ask.ids.clone(),
        // 边距/间距带也画进替身：拖动时整页贴图是冻住的，带子只在替身里更新。
        bands: bands.to_vec(),
        lod: true,
        magnified_image_pixels: true,
        ..Default::default()
    };
    // 拖动中的图层要脱离它所在 frame 的裁剪（像被拎起来），否则抓出来的替身就是被裁过的——
    // 用户实测：开始拖动的那一帧还带着 frame 裁剪的痕迹。渲染前临时把这些层所有祖先 frame
    // 的 `clip` 关掉（只动这次渲染用的副本，不动原文档），图层就完整浮出。
    let render_doc: Arc<Document> = {
        let mut d = (**doc).clone();
        let mut to_unclip: Vec<NodeId> = Vec::new();
        for &id in &ask.ids {
            let mut cur = d.node(id).and_then(|n| n.parent);
            while let Some(p) = cur {
                match d.node(p) {
                    Some(n) if n.clip && n.kind.is_frame_like() => {
                        to_unclip.push(p);
                        cur = n.parent;
                    }
                    Some(n) => cur = n.parent,
                    None => break,
                }
            }
        }
        for p in to_unclip {
            if let Ok(m) = d.get_mut(p) {
                m.clip = false;
            }
        }
        Arc::new(d)
    };
    let mut retained = Retained::default();
    let (view, pix, _work) = retained.render(&render_doc, page, &view, &opts)?;
    let mut data = pix.data().to_vec();
    premul_rgba_to_bgra(&mut data);
    let page_rect =
        [view.origin.0, view.origin.1, view.origin.0 + f64::from(view.width) / view.scale, view.origin.1 + f64::from(view.height) / view.scale];
    if std::env::var_os("FIGCRAFT_GRAB_LOG").is_some() {
        let alpha_at = |page_x: f64, page_y: f64| -> u8 {
            let x = ((page_x - view.origin.0) * view.scale).max(0.0) as u32;
            let y = ((page_y - view.origin.1) * view.scale).max(0.0) as u32;
            let i = ((y * pix.width() + x) * 4 + 3) as usize;
            data.get(i).copied().unwrap_or(0)
        };
        eprintln!(
            "[grab] ids={:?} view={:?} at {:?} alpha(at+4)={} alpha(mid)={}",
            ask.ids,
            view,
            ask.at,
            alpha_at(ask.at[0] + 4.0, (ask.page[1] + ask.page[3]) / 2.0),
            alpha_at((ask.page[0] + ask.page[2]) / 2.0, (ask.page[1] + ask.page[3]) / 2.0),
        );
    }
    if std::env::var_os("FIGCRAFT_GRAB_DUMP").is_some() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst) % 8;
        let mut rgba = data.clone();
        rgba_to_bgra(&mut rgba);
        if let Some(img) = image::RgbaImage::from_raw(pix.width(), pix.height(), rgba) {
            let _ = img.save(format!("grab_{n}.png"));
        }
    }
    Some(Grab { image: wrap_bgra(pix.width(), pix.height(), data), page: page_rect, at: ask.at })
}

/// 渲染 `job` 的 `view`（它自己的视图，或替代它的粗视图）。
fn run(job: &Job, view: &View, retained: &mut Retained, coarse: bool, reuse: Option<&Arc<RenderImage>>) -> Option<Done> {
    let t0 = std::time::Instant::now();
    // 绑了 WGSL 材质的节点在主图里要留空：材质是在这层 CPU 图上做离屏合成，玻璃采样的是
    // 「本帧图像裁下来的那块」。节点自己若还在图里，玻璃就会折射自己的填充色（实测：绿色
    // 矩形的中心被自己的绿色填满、边缘才发白），而不是背后的画面。跳过它之后，合成读到的
    // 就是真正的背景 —— 和参考管线里 `texture0_` = BackdropCache（不含节点本身）一致。
    //
    // 只在绘制用的 `Options::skip` 里扩展，不动 `job.skip`：后者只标记「底图要跳过哪些被拖层」，
    // 由 `GrabAsk` 替身补内容；材质节点属于离屏合成的挖空名单，不能混进 `job.skip` 的语义。
    let mut skip = job.skip.clone();
    // 材质节点在主图里必须留空：材质是在这层 CPU 图上做离屏合成，玻璃采样的是「本帧图像
    // 裁下来的那块」。节点自己若还在图里，玻璃就会折射自己的填充色（实测：绿色矩形的中心
    // 被自己的绿色填满、边缘才发白），而不是背后的画面。拖动期间同样要挖空——效果优先，
    // 材质在拖动中也要完整呈现（代价是拖动帧要等一次 GPU 合成）。
    if cfg!(feature = "gpu") {
        for id in job.doc.subtree(job.page) {
            if job.doc.node(id).is_some_and(|n| n.plugin_data.contains_key(crate::wgsl_material::BINDING_KEY)) {
                skip.push(id);
            }
        }
    }
    let opts = figcraft_render::Options {
        outline_mode: job.outline,
        outline: job.outline_extras,
        outlined: job.outlined.clone(),
        skip,
        solo: job.solo,
        lod: true,
        magnified_image_pixels: true,
        dots: job.dots,
        bands: job.bands.clone(),
        ..Default::default()
    };
    let (view, pix, work) = retained.render(&job.doc, job.page, view, &opts)?;
    let (w, h) = (pix.width(), pix.height());
    // 像素没变（`Retained` 增量缓存命中）：直接复用上一张图，不拷贝、不转换、不新建纹理。
    // 必须放在 `pix.data().to_vec()` **之前**——否则每帧白分配一整页（4.4MP ≈ 17.6MB），
    // 分配峰值叠上 GPUI 那边的纹理，就是那次 `memory allocation of 17598856 bytes failed`。
    if work == Work::Reused
        && job.grab.is_none()
        && let Some(prev) = reuse
    {
        return Some(Done {
            image: prev.clone(),
            grab: None,
            view,
            skip: job.skip.clone(),
            ms: t0.elapsed().as_secs_f64() * 1000.0,
            work,
            uid: job.uid,
            page: job.page,
            coarse,
            at: job.at,
            seq: 0,
        });
    }
    let raster_ms = t0.elapsed().as_secs_f64() * 1000.0;
    // 拖动中的图层：单独渲染一张透明底的替身（见 `grab_layer`）。
    let grab = job.grab.as_ref().and_then(|ask| grab_layer(&job.doc, job.page, ask, view.scale, &job.bands));
    // 软打样要按**原始**（预乘）像素判断哪些颜色 sRGB 显示不了：先算，再取走数据。
    let clipped: Option<Vec<bool>> = job.proof.then(|| job.doc.color_profile.conversion_to(ColorProfile::Srgb).clipped_pixels(pix.data()));
    // 贴图由 `Retained` 持有（它要靠这块缓冲做增量渲染），只能借出来：拷一份再转换。
    let mut data = pix.data().to_vec();
    let image = if clipped.is_some() || !job.doc.color_profile.is_srgb() {
        // 少见路径：反预乘 → 打样/色彩转换（都按 RGBA 通道算）→ 换序。
        demultiply(&mut data);
        if let Some(clipped) = clipped {
            mark_outside_srgb(&mut data, w, &clipped);
        } else {
            job.doc.color_profile.conversion_to(ColorProfile::Srgb).apply_pixels(&mut data);
        }
        rgba_to_bgra(&mut data);
        wrap_bgra(w, h, data)
    } else {
        // 常见路径：反预乘 + 换序一趟走完。
        premul_rgba_to_bgra(&mut data);
        wrap_bgra(w, h, data)
    };
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if render_log() {
        eprintln!(
            "[render] {w}x{h} {ms:.1}ms (栅格 {raster_ms:.1} + 转换 {:.1}) coarse={coarse} skip={} grab={} work={work:?}",
            ms - raster_ms,
            job.skip.len(),
            grab.is_some(),
        );
    }
    Some(Done { image, grab, view, skip: job.skip.clone(), ms, work, uid: job.uid, page: job.page, coarse, at: job.at, seq: 0 })
}

/// `view` 每边少 [`PREVIEW_FACTOR`] 倍像素，至少显示同样大的区域。
pub fn coarse_view(view: &View) -> View {
    let shrink = |n: u32| (f64::from(n) / PREVIEW_FACTOR).ceil().max(1.0) as u32;
    View { origin: view.origin, scale: view.scale / PREVIEW_FACTOR, width: shrink(view.width), height: shrink(view.height) }
}

/// WGSL 材质离屏合成：`Done.image`（straight BGRA）→ 逐绑定节点跑 WGSL → 合成后的 straight BGRA。
///
/// 先做纯 CPU 的 `passes_for`（枚举绑定节点、算导出）——只有真的有节点绑定材质时才惰性创建 wgpu
/// 设备，普通文档零开销。`time` 用进程单调时钟秒数，让材质的活参数（时间动画）每帧推进。
/// 任何失败都保持原 CPU 图——画布永不显示半截材质像素。
///
/// **必须每帧调用**（含 `Work::Reused`）：`last_image` 缓存的是**未合成**的原图，合成只作用于
/// 本帧要显示的 `done.image`；否则复用帧会在已合成的图上再叠一次。
/// 本帧是否存在「老多 pass 契约」的绑定 shader（`fs_glass` + `fs_tile` 入口）。
///
/// 两套通道按 shader 契约分派：老 demo 的 `fs_tile→blur→glass` 链走 `wgsl_composite`；其余所有
/// shader（含原版 `glass.wgsl` 和以后新增的任意材质）走通用 `wgsl_runner`。混用两种契约在一帧里
/// 不常见，故按第一个绑定 shader 决定整帧通道。
#[cfg(feature = "gpu")]
fn frame_uses_legacy_pipeline(doc: &figcraft_doc::Document, page: NodeId) -> bool {
    use figcraft_doc::NodeKind;
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id) else { continue };
        if matches!(n.kind, NodeKind::Page { .. }) {
            continue;
        }
        let Some(json) = n.plugin_data.get(crate::wgsl_material::BINDING_KEY) else { continue };
        let Some(binding) = crate::wgsl_material::WgslMaterialBinding::from_json(json) else { continue };
        let eid = crate::wgsl_material::ensure_loaded(&binding.path);
        let reg = figcraft_render::wgsl_effect::shared();
        let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(scan) = reg.scan(&eid) {
            let has_tile = scan.entries.iter().any(|e| e.name == "fs_tile");
            let has_glass = scan.entries.iter().any(|e| e.name == "fs_glass");
            return has_tile && has_glass;
        }
    }
    false
}

/// 绑了外置材质的**容器**：把它的直接子层单独渲成一张透明底的前景层。
///
/// 主图里材质节点被 `Options::skip` 整个让开（连子树一起），shader 的 backdrop 才是干净的
/// 背景；但容器里的内容不该跟着一起消失。材质合成完之后再把这层按原位置叠回去，顺序与
/// 渲染器里「效果落在填充之后、子层之前」一致：容器既看得见材质，也看得见里面装的东西。
#[cfg(feature = "gpu")]
fn material_foreground(doc: &Document, page: NodeId, view: &View, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut only = Vec::new();
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.children.is_empty() || !n.plugin_data.contains_key(crate::wgsl_material::BINDING_KEY) {
            continue;
        }
        only.extend(n.children.iter().copied());
    }
    if only.is_empty() {
        return None;
    }
    let opts = figcraft_render::Options {
        // `None` 会被页面底色顶上：这里要透明底，只能显式给全透明。
        background: Some(figcraft_color::Color::TRANSPARENT),
        // `only` 会照常从页面往下走，祖先 frame 的裁剪仍然生效，而材质节点自己不画填充。
        only,
        lod: true,
        magnified_image_pixels: true,
        ..Default::default()
    };
    let pix = figcraft_render::render_view_single(doc, page, view, &opts)?;
    if pix.width() != w || pix.height() != h {
        return None;
    }
    // tiny-skia 预乘 RGBA → 直通 RGBA（下面按直通 alpha 做 source-over）。
    let mut rgba = pix.data().to_vec();
    demultiply(&mut rgba);
    Some(rgba)
}

/// Render one material node's **direct children** on a transparent canvas: the layer that belongs
/// above *that* node's material. Returns straight RGBA plus the canvas-pixel box it occupies.
///
/// Only the box those layers can actually ink is rendered. Rasterising the *whole viewport* for
/// every material node was a second full-canvas pass (plus a full-canvas alpha scan) on every
/// frame that touched the view — the single biggest reason a pan or a zoom felt like a stall.
///
/// Nested material nodes are skipped here: each of them runs its own pass with its own backdrop, and
/// picks its own children up in turn.
#[cfg(feature = "gpu")]
fn children_layer(doc: &Document, page: NodeId, view: &View, parent: NodeId, cw: u32, ch: u32) -> Option<(Vec<u8>, [u32; 4])> {
    let n = doc.node(parent)?;
    if n.children.is_empty() {
        return None;
    }
    // Everything under this node **except** nested materials (and their subtrees): those run their
    // own pass with their own backdrop, and pick their own children up in turn. Skipping them by
    // node id instead would drop the whole subtree, children included.
    let mut only = Vec::new();
    collect_without_materials(doc, parent, &mut only);
    if only.is_empty() {
        return None;
    }
    // Their ink, plus room for shadows/blurs that spill past the geometry, clipped to the canvas.
    let ink = doc.bounds_of(&only)?;
    let pad = 64.0 / view.scale.max(1e-9);
    let vp = view.page_rect();
    let x0 = (ink.x0 - pad).max(vp.x0);
    let y0 = (ink.y0 - pad).max(vp.y0);
    let x1 = (ink.x1 + pad).min(vp.x1);
    let y1 = (ink.y1 + pad).min(vp.y1);
    if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
        return None;
    }
    let px = (((x0 - vp.x0) * view.scale).floor() as i64).max(0);
    let py = (((y0 - vp.y0) * view.scale).floor() as i64).max(0);
    let (px, py) = (px as u32, py as u32);
    if px >= cw || py >= ch {
        return None;
    }
    // Whole pixels, and never past the canvas: `Step::Over` keeps the image's own stride, so the
    // rect it is told has to be exactly the image's size.
    let bw = ((((x1 - x0) * view.scale).ceil() as u32).max(1)).min(cw - px);
    let bh = ((((y1 - y0) * view.scale).ceil() as u32).max(1)).min(ch - py);
    let sub = View { origin: (vp.x0 + f64::from(px) / view.scale, vp.y0 + f64::from(py) / view.scale), scale: view.scale, width: bw, height: bh };
    let opts = figcraft_render::Options {
        // `None` 会被页面底色顶上：这里要透明底，只能显式给全透明。
        background: Some(figcraft_color::Color::TRANSPARENT),
        only,
        lod: true,
        magnified_image_pixels: true,
        ..Default::default()
    };
    let pix = figcraft_render::render_view_single(doc, page, &sub, &opts)?;
    let mut rgba = pix.data().to_vec();
    demultiply(&mut rgba);
    Some((rgba, [px, py, pix.width(), pix.height()]))
}

/// Collect a node's descendants, pruning at nested material nodes: their layers are composited by
/// their own pass, at their own place in the stack.
#[cfg(feature = "gpu")]
fn collect_without_materials(doc: &Document, parent: NodeId, out: &mut Vec<NodeId>) {
    let Some(n) = doc.node(parent) else { return };
    for &c in &n.children {
        let Some(child) = doc.node(c) else { continue };
        if child.plugin_data.contains_key(crate::wgsl_material::BINDING_KEY) {
            continue;
        }
        out.push(c);
        collect_without_materials(doc, c, out);
    }
}

/// `front`（直通 RGBA、透明底）盖到 `scene`（直通 RGBA）上。绝大多数像素是透明的，直接跳过。
#[cfg(feature = "gpu")]
fn over_straight(scene: &mut [u8], front: &[u8]) {
    for (d, s) in scene.chunks_exact_mut(4).zip(front.chunks_exact(4)) {
        let sa = s[3];
        if sa == 0 {
            continue;
        }
        if sa == 255 {
            d.copy_from_slice(s);
            continue;
        }
        let a = f64::from(sa) / 255.0;
        let keep = f64::from(d[3]) / 255.0 * (1.0 - a);
        let oa = a + keep;
        if oa <= 0.0 {
            d.fill(0);
            continue;
        }
        for c in 0..3 {
            d[c] = ((f64::from(s[c]) * a + f64::from(d[c]) * keep) / oa).round().min(255.0) as u8;
        }
        d[3] = (oa * 255.0).round().min(255.0) as u8;
    }
}

/// 把一小块直通 BGRA 前景 `front`（`fw×fh`）按 `(fx,fy)` source-over 盖到整页画布 `scene`
/// （`sw×sh`， stride = `sw*4`）上。`front` 的行宽是 `fw*4`，与画布 stride 不同，必须逐行取。
///
/// 逐像素混合规则与 [`over_straight`] 完全一致，只是多了偏移与各自的 stride：这样 CPU 兜底只
/// 需栅格化材质节点墨迹框那一小块，而不是整页，也不必在整页上再扫一遍 alpha。
#[cfg(feature = "gpu")]
fn over_straight_at(scene: &mut [u8], sw: u32, front: &[u8], [fx, fy, fw, fh]: [u32; 4]) {
    let sw = sw as usize;
    let row = (fw as usize) * 4;
    for y in 0..fh as usize {
        let dst = (fy as usize + y) * sw * 4 + fx as usize * 4;
        let src = y * row;
        if dst + row > scene.len() || src + row > front.len() {
            continue;
        }
        over_straight(&mut scene[dst..dst + row], &front[src..src + row]);
    }
}

/// 页面子树里所有绑了外置 WGSL 材质的节点 id。
///
/// 这些正是 [`run`] 在 `cfg!(feature = "gpu")` 下加进 `Options::skip`、从 CPU 底图里挖空的节点——
/// 它们本该由 [`try_apply_wgsl_materials`] 在 GPU 上补回。兜底要用同一份名单把它们画回去。
#[cfg(feature = "gpu")]
fn material_ids(doc: &Document, page: NodeId) -> Vec<NodeId> {
    use figcraft_doc::NodeKind;
    doc.subtree(page)
        .into_iter()
        .filter(|&id| {
            doc.node(id).is_some_and(|n| !matches!(n.kind, NodeKind::Page { .. }) && n.plugin_data.contains_key(crate::wgsl_material::BINDING_KEY))
        })
        .collect()
}

/// CPU 兜底：把被 GPU 路径挖空的材质节点（连它们的子树）用普通渲染画回画布。
///
/// 触发条件是这一帧本该由 GPU 合成的外置材质**没能补回**（无 adapter、`WgslRunner::new()`
/// 失败、`composite_steps` 返回 `None`、或 shader 没加载导致 `passes` 为空）。此时 CPU 底图里
/// 这些节点连子树一起是空的，什么都不做就是「容器不闪但内容图不见了」。
///
/// 用 `Options::only` 渲这些节点：按 `draw.rs` 的语义，`only` 列出的节点**自身**（普通填充、
/// 圆角、描边）和它们的**整棵子树**都会画，只是没有 WGSL 玻璃——退化成普通矩形，但图完整、
/// 内容可见，与纯 CPU 构建的显示一致。透明底、独立 view，再按 source-over 盖回现有画布。
#[cfg(feature = "gpu")]
fn cpu_restore_materials(doc: &Document, page: NodeId, done: &mut Done) {
    let ids = material_ids(doc, page);
    if ids.is_empty() {
        return;
    }
    let (w, h) = {
        let sz = done.image.size(0);
        (u32::from(sz.width), u32::from(sz.height))
    };
    if w == 0 || h == 0 {
        return;
    }
    let (Some(bgra), view) = (done.image.as_bytes(0), done.view.clone()) else { return };

    // 只栅格化这些材质节点的墨迹框（`only` 会连子树一起画），不再整页重渲。
    // 旧实现用整页 `view` + 单线程 `render_view_single`：无 adapter 的设备上，refine/preview/
    // watchdog/overview/patch 五条路径每帧都付一次单线程整页光栅 + 整页 demultiply + 整页
    // blend，正是「拖动时画布外区域加载慢」在无 GPU 设备上的放大版。这里按 `children_layer`
    // 的成熟做法把 view 裁到墨迹框，并换用 `render_view`（桌面 rayon 分带并行，wasm32 自动
    // 退化为单线程 `render_one`，与旧行为一致——见 render/src/lib.rs:161-175 的 cfg 门控）。
    let Some(ink) = doc.bounds_of(&ids) else { return };
    let pad = 64.0 / view.scale.max(1e-9); // 阴影/模糊外溢留边
    let vp = view.page_rect();
    let x0 = (ink.x0 - pad).max(vp.x0);
    let y0 = (ink.y0 - pad).max(vp.y0);
    let x1 = (ink.x1 + pad).min(vp.x1);
    let y1 = (ink.y1 + pad).min(vp.y1);
    if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
        return;
    }
    // 整像素、不越界：子图的 stride 就是自己的 bw*4，`over_straight_at` 按它逐行取。
    let px = (((x0 - vp.x0) * view.scale).floor() as i64).max(0) as u32;
    let py = (((y0 - vp.y0) * view.scale).floor() as i64).max(0) as u32;
    if px >= w || py >= h {
        return;
    }
    let bw = ((((x1 - x0) * view.scale).ceil() as u32).max(1)).min(w - px);
    let bh = ((((y1 - y0) * view.scale).ceil() as u32).max(1)).min(h - py);
    let sub = View { origin: (vp.x0 + f64::from(px) / view.scale, vp.y0 + f64::from(py) / view.scale), scale: view.scale, width: bw, height: bh };
    let id_count = ids.len();
    let opts = figcraft_render::Options {
        background: Some(figcraft_color::Color::TRANSPARENT),
        only: ids,
        lod: true,
        magnified_image_pixels: true,
        ..Default::default()
    };
    let Some(pix) = figcraft_render::render_view(doc, page, &sub, &opts) else { return };
    if pix.width() != bw || pix.height() != bh {
        return;
    }
    // 只处理墨迹框那一小块：tiny-skia 预乘 RGBA → 直通 RGBA → 画布的 BGRA 顺序。
    let mut front = pix.data().to_vec();
    demultiply(&mut front);
    rgba_to_bgra(&mut front);
    let mut out = bgra.to_vec();
    over_straight_at(&mut out, w, &front, [px, py, bw, bh]);
    done.image = wrap_bgra(w, h, out);
    if render_log() {
        eprintln!(
            "[material] CPU 兜底 {w}x{h} 只栅格化材质框 {bw}x{bh} ({:.1}% 面积), {} 节点",
            (f64::from(bw) * f64::from(bh) / f64::from(w) / f64::from(h) * 100.0),
            id_count
        );
    }
}

/// The offscreen compositors, kept across frames. Split out of [`Renders`] so the preview path (and
/// the page overview) can composite too: a preview without the material is a *different picture*,
/// and swapping the canvas onto it for a few frames is exactly the flash the user sees on every
/// zoom step.
#[cfg(feature = "gpu")]
#[derive(Default)]
pub struct Materials {
    /// The legacy multi-pass compositor (`fs_tile` + `fs_glass` shaders).
    legacy: Option<crate::wgsl_composite::WgslComposite>,
    /// The universal runner (`@fx param` / `Step` contract).
    runner: Option<crate::wgsl_runner::WgslRunner>,
    /// The last composited picture, handed back while its inputs stay byte-identical.
    last_composite: Option<LastComposite>,
}

/// One composited frame: what it was made from, and what it looked like.
///
/// A composite is a pure function of the page image it read (`base`) and of the passes describing
/// the materials over it (`key`). Dragging a layer that nothing else depends on leaves both alone —
/// the page image is the very same `Arc` (`run`'s `Work::Reused` path) and the passes come out of
/// `passes_for` unchanged — so the picture is the same one as last frame's, and the whole
/// composite (children layers, GPU region upload, readback, splice, and the full-page texture
/// upload that follows) can be skipped without changing a pixel.
#[cfg(feature = "gpu")]
struct LastComposite {
    base: Arc<RenderImage>,
    key: u64,
    image: Arc<RenderImage>,
}

/// Everything a composite reads besides its page image, hashed. `time` is not hashed on purpose:
/// it only reaches a shader that **declares** a `time` uniform, and such a pass packs it into
/// `uniforms`, so its key differs every frame and it recomposites — animation keeps running. A
/// shader that ignores `time` packs nothing of it and keeps its key.
#[cfg(feature = "gpu")]
fn composite_key(passes: &[crate::wgsl_runner::RunPass], view: &View, w: u32, h: u32) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut key = std::collections::hash_map::DefaultHasher::new();
    (w, h).hash(&mut key);
    view.origin.0.to_bits().hash(&mut key);
    view.origin.1.to_bits().hash(&mut key);
    view.scale.to_bits().hash(&mut key);
    passes.len().hash(&mut key);
    for p in passes {
        p.effect.hash(&mut key);
        p.node.0.hash(&mut key);
        p.rect_px.hash(&mut key);
        p.radii_px.iter().for_each(|v| v.to_bits().hash(&mut key));
        (p.node_px.0.to_bits(), p.node_px.1.to_bits()).hash(&mut key);
        (p.origin_px.0.to_bits(), p.origin_px.1.to_bits()).hash(&mut key);
        p.field_band.to_bits().hash(&mut key);
        p.field_bytes.hash(&mut key);
        for (name, bytes) in &p.uniforms {
            name.hash(&mut key);
            bytes.hash(&mut key);
        }
    }
    key.finish()
}

/// Last frame's composite, when this frame reads the same page image through the same passes.
#[cfg(feature = "gpu")]
fn reuse_composite(mats: &Materials, base: &Arc<RenderImage>, key: u64) -> Option<Arc<RenderImage>> {
    let last = mats.last_composite.as_ref()?;
    (last.key == key && Arc::ptr_eq(&last.base, base)).then(|| last.image.clone())
}

#[cfg(feature = "gpu")]
fn apply_wgsl_materials(mats: &mut Materials, doc: &Document, page: NodeId, done: &mut Done) {
    if !try_apply_wgsl_materials(mats, doc, page, done) {
        // `run` 在 cfg!(gpu) 下把材质节点连子树一起挖空了。GPU 这一帧没能补回，必须用 CPU 画
        // 回去，否则就是「容器不闪、里面的图不见了」。无 adapter 的设备上这是常事，不是意外。
        cpu_restore_materials(doc, page, done);
    }
}

/// 材质进程的单调时钟（秒）。同步合成与异步提交共用同一原点，让同一帧里 `passes_for`
/// 拿到的 `time` 一致（对 `glass.wgsl` 这种无 time uniform 的 shader 无影响，对将来通用
/// `@fx` 时间动画材质才对得上）。
#[cfg(feature = "gpu")]
fn material_time() -> f64 {
    static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    CLOCK.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// 把 z 序的 `passes` 和每个 pass 的子层组装成 steps。同步合成（[`try_apply_wgsl_materials`]）
/// 与异步提交（[`try_submit_wgsl_materials`]）共用这一份，保证两条管线的 pass/over 顺序、
/// 裁剪和子层定位逐字节一致——异步唯一改变的只是「何时读回」，绝不是「画什么」。
#[cfg(feature = "gpu")]
fn assemble_steps<'a>(passes: &'a [crate::wgsl_runner::RunPass], layers: &'a [Option<(Vec<u8>, [u32; 4])>]) -> Vec<crate::wgsl_runner::Step<'a>> {
    let mut steps = Vec::with_capacity(passes.len() * 2);
    for (p, layer) in passes.iter().zip(layers.iter()) {
        steps.push(crate::wgsl_runner::Step::Pass(p));
        if let Some((pixels, rect)) = layer {
            steps.push(crate::wgsl_runner::Step::Over { pixels: pixels.as_slice(), rect: *rect });
        }
    }
    steps
}

/// 试走 GPU 合成。返回 `true` 当且仅当：**画布里本该由外置材质补回的节点，这一帧确实补齐了**
/// （或本来就没有需要补的节点）。返回 `false` 时画布带着挖空的洞，调用方必须走 CPU 兜底。
#[cfg(feature = "gpu")]
fn try_apply_wgsl_materials(mats: &mut Materials, doc: &Document, page: NodeId, done: &mut Done) -> bool {
    let (w, h) = {
        let sz = done.image.size(0);
        (u32::from(sz.width), u32::from(sz.height))
    };
    if w == 0 || h == 0 {
        return true;
    }
    let time = material_time();
    let expected = (w as usize) * (h as usize) * 4;

    // The legacy multi-pass shaders keep their old single-layer path. The universal one composites
    // the whole stack in z order (see below).
    if frame_uses_legacy_pipeline(doc, page) {
        let passes = crate::wgsl_composite::passes_for(doc, page, &done.view, time);
        let front = material_foreground(doc, page, &done.view, w, h);
        if passes.is_empty() && front.is_none() {
            return material_ids(doc, page).is_empty();
        }
        let Some(bgra) = done.image.as_bytes(0) else { return false };
        let mut rgba = bgra.to_vec();
        rgba_to_bgra(&mut rgba);
        let composited = if passes.is_empty() {
            None
        } else {
            if mats.legacy.is_none() {
                mats.legacy = crate::wgsl_composite::WgslComposite::new();
            }
            mats.legacy.as_mut().and_then(|r| r.composite(&rgba, w, h, &passes))
        };
        // composite 失败时 `out` 是挖空底图 + front（子层），材质容器本身仍缺 → 交由兜底补回。
        let ok = composited.is_some();
        let mut out = match composited {
            Some(o) if o.len() >= expected => o,
            _ => rgba,
        };
        if let Some(front) = &front {
            over_straight(&mut out, front);
        }
        rgba_to_bgra(&mut out);
        done.image = wrap_bgra(w, h, out);
        return ok;
    }

    let passes = crate::wgsl_runner::passes_for(doc, page, &done.view, time);
    if passes.is_empty() {
        return material_ids(doc, page).is_empty();
    }
    // 同一张底图 + 同一组 passes：上一帧的合成图就是这一帧的图（拖动、hover 之类的帧间静止）。
    let key = composite_key(&passes, &done.view, w, h);
    if let Some(image) = reuse_composite(mats, &done.image, key) {
        done.image = image;
        return true;
    }
    let base_image = done.image.clone();
    // Straight BGRA in, straight BGRA out — the canvas texture is declared BGRA (see `FORMAT`), so
    // neither direction needs the whole-image channel swap the old path ran twice per frame.
    // 借用而非 `to_vec()`：`composite_steps` 只从这块画布里 crop 材质 region 上传、读回时用它
    // 当 base，全程只读；返回的合成图是 owned，NLL 在这里释放借用后下面就能覆盖 `done.image`。
    // 省掉的是一次整页 `w*h*4` 拷贝（1080p ≈ 8MB、约 1ms），每帧每次合成都省。
    let canvas = match done.image.as_bytes(0) {
        Some(b) => b,
        None => return false,
    };

    // One children layer per material node, rendered **before** the composite so it can be dropped
    // into the stack right above its own material. This is what keeps the z order when materials
    // are nested: the old code rendered every child layer once and pasted the lot on top of the
    // finished image, so an upper pane's children floated above the lower pane's glass entirely.
    let t_layers = std::time::Instant::now();
    let layers: Vec<Option<(Vec<u8>, [u32; 4])>> = passes.iter().map(|p| children_layer(doc, page, &done.view, p.node, w, h)).collect();
    let layers_ms = t_layers.elapsed().as_secs_f64() * 1000.0;

    if mats.runner.is_none() {
        mats.runner = crate::wgsl_runner::WgslRunner::new();
    }
    let Some(runner) = mats.runner.as_mut() else { return false };
    let steps = assemble_steps(&passes, &layers);
    let t_comp = std::time::Instant::now();
    let composited = runner.composite_steps(canvas, w, h, &steps);
    // `composite_steps` returns a full-size buffer even when a shader failed to load or compile: it
    // silently skips that pass's glass draw, still draws the children (`Over`), and reads back. If
    // we only checked the buffer length we would show a container with its contents but no glass —
    // exactly the "效果对不上/内容缺一块" failure. Require **every** pass to have a pipeline; a
    // single missing one means the canvas is incomplete and the caller must restore from the CPU.
    let (drawn, total) = runner.steps_drawn(&steps);
    let ok = composited.as_ref().is_some_and(|out| out.len() >= expected) && drawn == total;
    if ok && let Some(out) = composited {
        done.image = wrap_bgra(w, h, out);
        mats.last_composite = Some(LastComposite { base: base_image, key, image: done.image.clone() });
    }
    if render_log() {
        eprintln!(
            "[material] {w}x{h} passes={} drawn={drawn}/{total} layers={layers_ms:.1}ms composite={:.1}ms ok={ok}",
            passes.len(),
            t_comp.elapsed().as_secs_f64() * 1000.0
        );
    }
    ok
}

/// 一次异步提交的去向。
#[cfg(feature = "gpu")]
enum Submit {
    /// 命令已入队、像素稍后由 [`collect_ready`] 收；`key` 让收齐时记下合成缓存。
    Queued { id: u64, key: u64 },
    /// 输入与上一帧相同：这张合成图直接可用，不必碰 GPU。
    Reused(Arc<RenderImage>),
    /// 本帧不能安全地异步（老多 pass 契约、shader 没编译齐、没有 adapter、读回环满），
    /// 调用方退回同步的 [`apply_wgsl_materials`]（含 CPU 兜底）。
    Sync,
}

/// 尝试把这一帧的材质合成**异步**提交给 GPU：`Queued` 表示命令已入队、像素稍后由
/// [`collect_ready`] 收；`Sync` 表示本帧不能安全地异步，调用方必须退回同步的
/// [`apply_wgsl_materials`]（含 CPU 兜底）。
///
/// 同步与异步共用 [`passes_for`] / [`children_layer`] / [`assemble_steps`]，唯一区别是后者用
/// `submit_async` 而非 `composite_steps`。提交前先用 `ensure_pipelines` 把缺的 shader 编译好并
/// 核对 `drawn == total` —— 否则一个静默跳过 glass draw 的异步帧会把「有子层、没玻璃」的错图
/// 交回 UI，而那时帧早就翻篇了。
#[cfg(feature = "gpu")]
fn try_submit_wgsl_materials(mats: &mut Materials, doc: &Document, page: NodeId, done: &Done) -> Submit {
    let (w, h) = {
        let sz = done.image.size(0);
        (u32::from(sz.width), u32::from(sz.height))
    };
    if w == 0 || h == 0 {
        return Submit::Sync;
    }
    // 老 demo 的 `fs_tile→blur→glass` 多 pass 契约只走同步通道：它每帧一张、本就不值得为它
    // 单开异步状态机，保持与历史逐字节一致。
    if frame_uses_legacy_pipeline(doc, page) {
        return Submit::Sync;
    }
    let passes = crate::wgsl_runner::passes_for(doc, page, &done.view, material_time());
    // 没有 universal-runner pass：交给同步路径判定「本来就无材质」或走 CPU 兜底。
    if passes.is_empty() {
        return Submit::Sync;
    }
    // 底图与 passes 逐字节没变：上一帧的合成图就是这一帧该显示的图，连 GPU 都不用碰。
    let key = composite_key(&passes, &done.view, w, h);
    if let Some(image) = reuse_composite(mats, &done.image, key) {
        return Submit::Reused(image);
    }
    let Some(canvas) = done.image.as_bytes(0) else { return Submit::Sync };
    let layers: Vec<Option<(Vec<u8>, [u32; 4])>> = passes.iter().map(|p| children_layer(doc, page, &done.view, p.node, w, h)).collect();
    if mats.runner.is_none() {
        mats.runner = crate::wgsl_runner::WgslRunner::new();
    }
    let Some(runner) = mats.runner.as_mut() else { return Submit::Sync };
    let steps = assemble_steps(&passes, &layers);
    // 编译 + 判据：与同步 `try_apply_wgsl_materials` 的 `steps_drawn` 检查等价。必须先确保
    // shader 齐全再占用 slot——否则一个静默跳过 glass draw 的异步帧会把「有子层没玻璃」的
    // 错图交回 UI。`submit_async` 内部还会再走一次 `refresh_pipelines`，但那时 cache 已热，
    // 只是 500ms 节流的 stale 检查，几乎免费。
    let (drawn, total) = runner.ensure_pipelines(w, h, &steps);
    if drawn != total || drawn == 0 {
        return Submit::Sync;
    }
    match runner.submit_async(canvas, w, h, &steps) {
        Some(id) => Submit::Queued { id, key },
        None => Submit::Sync,
    }
}

/// 一帧已提交到 GPU、readback 还在飞的完整上下文。收集成功后 `done.image` 从「未合成的
/// CPU 底图」换成「材质 region 已拼回的合成图」，其它字段（view/uid/page/at…）原样带出。
#[cfg(feature = "gpu")]
struct PendingFrame {
    id: u64,
    seq: u64,
    done: Done,
    w: u32,
    h: u32,
    /// The composite key this frame was submitted with; it labels the picture `collect_ready`
    /// builds, so the next frame reading the same inputs can take it as it is.
    key: u64,
}

/// render 线程在「有异步帧在飞」时的自驱间隔。只在 pending 非空时按这个节拍 `recv_timeout`，
/// 其余时间照旧阻塞在 `recv()` 上，空闲零开销。2ms 远短于 60Hz 的 16.7ms，能在一帧内多轮
/// 收集，尽量让 UI 拿到的就是刚提交那帧、而不是还停在上一张。
#[cfg(feature = "gpu")]
const ASYNC_TICK: std::time::Duration = std::time::Duration::from_millis(2);

/// render 线程这一轮从 job 通道里取到了什么。`Tick` 是「有异步帧在飞、只是等够了 2ms 还没
/// 新活」——它必须回到循环顶部继续 `collect_ready`，绝不能当成通道关闭而退出。
#[cfg(feature = "gpu")]
enum Pulled {
    Job((u64, Job)),
    Tick,
    Closed,
}

/// 把一批完成图经 [`Handoff`] 交给 UI，并在真正送出后单调推进水位。
/// 返回 `false` 表示 UI 已断开、render 线程该退出。
#[cfg(feature = "gpu")]
fn deliver(
    handoff: &mut Handoff,
    frames: Vec<Done>,
    done_tx: &std::sync::mpsc::SyncSender<Done>,
    done_with: &std::sync::atomic::AtomicU64,
    refined: &std::sync::atomic::AtomicU64,
    wake: &Arc<dyn Fn() + Send + Sync>,
) -> bool {
    match handoff.send(frames, done_tx) {
        Ok(Some(seq)) => {
            done_with.fetch_max(seq, std::sync::atomic::Ordering::Release);
            refined.fetch_max(seq, std::sync::atomic::Ordering::SeqCst);
            (wake.as_ref())();
            true
        }
        Ok(None) => true,
        Err(()) => false,
    }
}

/// 推进 GPU 回调并把已完成的异步帧收集成可上台的 Done。render 线程是这台 device 唯一的
/// poller，所以**每一轮循环都要调它**（含没有新 job 的 tick 轮）——wgpu 的 `map_async` 回调
/// 只在 `device.poll` 时 fire，漏一轮就有一张图永远收不回来、`finished` 水位卡死。
/// 非阻塞：`poll_ready` 内部用零超时 `Wait` 主动 flush fence，没好的帧留在 `pending`。
#[cfg(feature = "gpu")]
fn collect_ready(mats: &std::sync::Arc<std::sync::Mutex<Materials>>, pending: &mut Vec<PendingFrame>, ready: &mut Vec<Done>) {
    if pending.is_empty() {
        return;
    }
    let mut guard = mats.lock().unwrap_or_else(|e| e.into_inner());
    let Some(runner) = guard.runner.as_mut() else {
        pending.clear();
        return;
    };
    let mut collected = Vec::new();
    for (i, pf) in pending.iter().enumerate() {
        let base = pf.done.image.as_bytes(0);
        // region 外的像素来自这一帧自己的 base（提交时的 CPU 画布），两帧之间绝不串。
        let pixels = base.and_then(|b| runner.poll_ready(pf.id, b));
        if let Some(pixels) = pixels {
            collected.push((i, pixels));
        }
    }
    // 倒序移除，避免前面的帧被收走后索引漂移。
    for (i, pixels) in collected.into_iter().rev() {
        let mut pf = pending.remove(i);
        // 收齐的这张就是「这张底图 + 这组 passes」的合成结果：记下来，下一帧输入没变就直接用它。
        let base = pf.done.image.clone();
        pf.done.image = wrap_bgra(pf.w, pf.h, pixels);
        guard.last_composite = Some(LastComposite { base, key: pf.key, image: pf.done.image.clone() });
        ready.push(pf.done);
    }
}

/// render 线程每一轮（含没有新 job 的 tick 轮）都要做的「推进 GPU 回调 + 交付」。
///
/// **关键**：即使这一轮没有 GPU 帧收齐（`frames` 为空），只要 [`Handoff::deferred`] 里还压着
/// 上一轮通道满时暂存的最新帧，就必须交给 [`deliver`] 重试。否则「deferred 有帧、pending 空、
/// 又没有新 job」时，render 线程会在 2ms tick 上空转——collect_ready 每轮都拿到空 frames，
/// 那张图永远发不出去，`finished` 水位不动、UI 的 `in_flight()` 恒为 true、画面停在上一张、
/// 动画循环空转耗电。这正是缩放时最怕的「卡住不刷新」。
///
/// 返回 `false` 表示 UI 已断开、render 线程该退出。
#[cfg(feature = "gpu")]
fn pump_and_deliver(
    mats: &std::sync::Arc<std::sync::Mutex<Materials>>,
    pending: &mut Vec<PendingFrame>,
    handoff: &mut Handoff,
    done_tx: &std::sync::mpsc::SyncSender<Done>,
    done_with: &std::sync::atomic::AtomicU64,
    refined: &std::sync::atomic::AtomicU64,
    wake: &Arc<dyn Fn() + Send + Sync>,
) -> bool {
    let mut frames = Vec::new();
    collect_ready(mats, pending, &mut frames);
    // 真没事可做才跳过；deferred 非空就一定要走 deliver 把它重试出去。
    if frames.is_empty() && handoff.deferred.is_none() {
        return true;
    }
    deliver(handoff, frames, done_tx, done_with, refined, wake)
}

/// `Renders::refine_async` 的分流结果。
#[cfg(feature = "gpu")]
pub(crate) enum RefineOutcome {
    /// 这一帧当场就有最终图（本无材质 / shader 没齐 / 无 adapter / 读回环满退回同步），
    /// 可立即交给 UI。
    Ready(Done),
    /// 材质已异步提交、像素还在 GPU 上；带上未合成的 CPU 底图作为收集时的 base。
    /// 收集成功后 `collect_ready` 会把材质 region 拼回它、产出可上台的 Done。
    Submitted { id: u64, base: Done, w: u32, h: u32, key: u64 },
    /// 这一帧压根没渲出图。
    None,
}

/// 完成图交给 UI 的中转站。render 线程是唯一写者，`done_tx` 是容量 2 的有界通道。
///
/// 两条铁律：
/// 1. **不回退**：只把 `seq > highest` 的帧交给 UI，乱序晚到的旧帧直接丢。UI 侧 `shows()`
///    也用 `done.seq < tex_seq` 挡，双重保证画面不会倒退。
/// 2. **不丢最新帧**：同一批里只取 seq 最大的那张发送；通道满就暂存在 `deferred`、不推进
///    水位（`in_flight` 保持 true → UI 继续续帧 → 下一轮重试），绝不把更新的一张丢在地板上。
///    返回被成功送出的 seq，调用方据此单调推进 `finished` / `refined`。
#[cfg(feature = "gpu")]
#[derive(Default)]
struct Handoff {
    highest: u64,
    deferred: Option<Done>,
}

#[cfg(feature = "gpu")]
impl Handoff {
    /// 返回 `Ok(Some(seq))` 表示这一轮有新帧进了通道、调用方应把水位推进到 `seq`；
    /// `Ok(None)` 表示没有可送的（空批，或通道满已暂存）；`Err(())` 表示 UI 已断开。
    fn send(&mut self, mut frames: Vec<Done>, tx: &std::sync::mpsc::SyncSender<Done>) -> Result<Option<u64>, ()> {
        if let Some(d) = self.deferred.take() {
            frames.push(d);
        }
        frames.retain(|d| d.seq > self.highest);
        let Some(best) = frames.into_iter().max_by_key(|d| d.seq) else {
            return Ok(None);
        };
        let seq = best.seq;
        match tx.try_send(best) {
            Ok(()) => {
                self.highest = seq;
                Ok(Some(seq))
            }
            // UI 还没腾出通道位：暂存这张最新的，水位不动，下一轮自驱重试。
            Err(std::sync::mpsc::TrySendError::Full(d)) => {
                self.deferred = Some(d);
                Ok(None)
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Err(()),
        }
    }
}

/// Render thread 在任务之间保留的东西。
pub struct Renders {
    full: Retained,
    preview: Retained,
    /// GPU 后端（§18）：`None` = 没开、没适配器，或这一帧压不出场景。按需创建、跨帧复用。
    #[cfg(feature = "gpu")]
    gpu: Option<crate::gpu::Gpu>,
    /// WGSL 材质离屏合成器（仅 `--features gpu`）：CPU 画布 → 逐节点 WGSL 多 pass → readback。
    /// 精修、粗预览、整页兜底图、看门狗补做的预览共用同一套设备（见 [`Materials`]）——多建一个
    /// 设备就多一份几十 MB 的显存和一次上百毫秒的初始化，而它们本来就在抢同一块 GPU。
    #[cfg(feature = "gpu")]
    mats: std::sync::Arc<std::sync::Mutex<Materials>>,
    /// 上一次完整渲染的文档、页面与尺度。
    last: Option<(u64, NodeId, f64)>,
    /// 上一次精修出的整页贴图：像素没变（`Retained` 命中增量缓存）时直接复用，避免每帧
    /// `wrap_bgra` 新分配 + GPUI 每帧新建一张 GPU 纹理——后者会逐帧累积内存直到
    /// `memory allocation of N bytes failed`（本机实测约 55s 后崩）。
    last_image: Option<Arc<RenderImage>>,
    full_ms: f64,
    preview_after_ms: f64,
}

impl Default for Renders {
    fn default() -> Self {
        Self::with_threshold(PREVIEW_AFTER_MS)
    }
}

impl Renders {
    pub fn with_threshold(ms: f64) -> Self {
        Self {
            full: Retained::default(),
            preview: Retained::default(),
            #[cfg(feature = "gpu")]
            gpu: None,
            #[cfg(feature = "gpu")]
            mats: std::sync::Arc::new(std::sync::Mutex::new(Materials::default())),
            last: None,
            last_image: None,
            full_ms: 0.0,
            preview_after_ms: ms,
        }
    }

    fn starts_over(&self, job: &Job) -> bool {
        match self.last {
            None => true,
            Some((uid, page, scale)) => uid != job.uid || page != job.page || scale.to_bits() != job.view.scale.to_bits(),
        }
    }

    fn wants_preview(&self, job: &Job) -> bool {
        job.zoom_preview && self.last.is_some() && self.full_ms >= self.preview_after_ms && self.starts_over(job)
    }

    pub fn wants_late_preview(&self, job: &Job) -> bool {
        job.zoom_preview && self.starts_over(job) && !self.wants_preview(job)
    }

    pub fn preview(&mut self, job: &Job) -> Option<Done> {
        if !self.wants_preview(job) {
            return None;
        }
        let mut d = preview_with(job, &mut self.preview)?;
        // A preview **must** carry the materials as well. `run` keeps a material node's whole
        // subtree out of the base image (the shader needs a clean backdrop), so a preview that
        // skipped the offscreen pass was a picture with the pane *and everything inside it*
        // missing. Swapping the canvas onto that for a few frames, then back, is the flash on
        // every zoom step. At a quarter of the linear scale this costs 1/16 of the pixels.
        #[cfg(feature = "gpu")]
        apply_wgsl_materials(&mut self.mats.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut d);
        Some(d)
    }

    /// 只做 CPU 光栅 + 帧间缓存记账，产出**尚未合成**外置材质的整页图。
    ///
    /// 同步 [`Self::refine`] 与异步 [`Self::refine_async`] 共用这一段，保证两条管线交给材质的
    /// `base` 画布逐字节一致。返回时 `last_image` 已记下这张未合成原图——复用帧据此重新合成，
    /// 绝不在已合成图上二次叠加。
    fn rasterize(&mut self, job: &Job) -> Option<Done> {
        // 上一张图借给 `run`：像素没变时它直接复用这张，不再分配整整一页。
        let prev = self.last_image.clone();
        let mut done = guarded(&mut self.full, |r| run(job, &job.view, r, false, prev.as_ref()))?;
        self.last = Some((job.uid, job.page, job.view.scale));
        if done.work == Work::Full {
            self.full_ms = done.ms;
        }
        // 像素没变的那一帧（`Retained` 增量缓存命中）直接复用上一张图：既不分配新 `Vec`，
        // 也不让 GPUI 每帧新建一张 GPU 纹理（那样会逐帧累积内存直到分配失败）。
        if done.work == Work::Reused
            && let Some(prev) = &self.last_image
        {
            done.image = prev.clone();
        }
        self.last_image = Some(done.image.clone());
        Some(done)
    }

    /// 同步精修：走完整管线（含同步材质合成），供 GPU 场景路径 / `render_now` / 测试用。
    /// 普通拖动走 worker 的异步 `refine_async`，不再经过这条同步路径。
    pub fn refine(&mut self, job: &Job) -> Option<Done> {
        #[cfg(feature = "gpu")]
        if gpu_enabled() {
            if self.gpu.is_none() {
                self.gpu = crate::gpu::Gpu::new();
                if self.gpu.is_none() {
                    eprintln!("[gpu] no adapter: staying on the CPU renderer");
                }
            }
            if let Some(g) = self.gpu.as_mut()
                && let Some(mut d) = run_gpu(job, &job.view, g)
            {
                self.last = Some((job.uid, job.page, job.view.scale));
                if d.work == Work::Full {
                    self.full_ms = d.ms;
                }
                // GPU 出的也是整页图：像素不变时同样复用，不重复分配。
                if d.work == Work::Reused {
                    if let Some(prev) = &self.last_image {
                        d.image = prev.clone();
                    }
                }
                self.last_image = Some(d.image.clone());
                // last_image 已存原图；WGSL 只作用于本帧显示图，复用帧不会二次叠加。
                #[cfg(feature = "gpu")]
                apply_wgsl_materials(&mut self.mats.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut d);
                return Some(d);
            }
        }
        let mut done = self.rasterize(job)?;
        // WGSL 材质合成：CPU 画布当 sceneTex，逐绑定节点跑多 pass，readback 回显示图。
        // 必须在 last_image 赋值之后——缓存始终保留未合成原图，Reused 帧才不会二次叠加。
        #[cfg(feature = "gpu")]
        apply_wgsl_materials(&mut self.mats.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut done);
        Some(done)
    }

    /// 异步精修：CPU 光栅照旧同步做（它是增量缓存的产地、也必须当帧完成），材质合成则优先
    /// 走 `submit_async` —— 提交立即返回，像素由 render 线程自驱 `collect_ready` 稍后收。
    /// 任何不能安全异步的情形（无材质 / 老契约 / shader 没齐 / 无 adapter / 读回环满）都退回
    /// 与 [`Self::refine`] 完全相同的同步通道，所以每个 job 必有确定归宿，不会把图丢在 GPU 上。
    #[cfg(feature = "gpu")]
    pub fn refine_async(&mut self, job: &Job) -> RefineOutcome {
        // `FIGCRAFT_GPU=1` 的实验性 GPU 场景路径保持同步（它自带一台设备、且默认关闭）。
        if gpu_enabled() {
            return match self.refine(job) {
                Some(d) => RefineOutcome::Ready(d),
                None => RefineOutcome::None,
            };
        }
        let mut base = match self.rasterize(job) {
            Some(d) => d,
            None => return RefineOutcome::None,
        };
        let (w, h) = {
            let sz = base.image.size(0);
            (u32::from(sz.width), u32::from(sz.height))
        };
        // 提交与编译判定在同一把 mats 锁内完成，避免 watchdog 线程在两者之间改了 runner 状态。
        let submitted = {
            let mut mats = self.mats.lock().unwrap_or_else(|e| e.into_inner());
            try_submit_wgsl_materials(&mut mats, &job.doc, job.page, &base)
        };
        match submitted {
            Submit::Queued { id, key } => RefineOutcome::Submitted { id, base, w, h, key },
            // 合成输入没变：上一帧那张合成图就是这一帧的画面，没有 readback、没有新纹理。
            Submit::Reused(image) => {
                base.image = image;
                RefineOutcome::Ready(base)
            }
            Submit::Sync => {
                apply_wgsl_materials(&mut self.mats.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut base);
                RefineOutcome::Ready(base)
            }
        }
    }

    /// 当帧同步渲染一小块**完整场景**（不是透明底的图层替身）。
    ///
    /// 走与整页精修图完全相同的 `run` + `apply_wgsl_materials`：内置效果、外置材质、容器
    /// 裁剪、容器子层全都和静态画面一致，只是 `view` 缩到指定区域。每帧新建 `Retained`
    /// 是故意的 —— 局部 view 每帧都在变，增量缓存帮不上忙。
    ///
    /// 生产拖动已改为「底图 latest wins + `GrabAsk` 替身」，不再经过这里；本入口留给
    /// `render_now` 之类的同步局部渲染需求，以及下方验证「局部 view 下材质子节点可见」的回归测试。
    pub fn render_patch(&mut self, job: &Job) -> Option<Done> {
        let mut retained = Retained::default();
        let mut done = guarded(&mut retained, |r| run(job, &job.view, r, false, None))?;
        #[cfg(feature = "gpu")]
        apply_wgsl_materials(&mut self.mats.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut done);
        Some(done)
    }
}

/// 一次局部同步渲染的作业：整页文档 + 局部 view，不抓替身、不打粗预览。
pub(super) fn patch_job(doc: Arc<Document>, page: NodeId, view: View, bands: &[figcraft_render::Band], uid: u64) -> Job {
    Job {
        doc,
        page,
        view,
        outline: false,
        outline_extras: figcraft_render::Outline::default(),
        outlined: Vec::new(),
        skip: Vec::new(),
        solo: None,
        proof: false,
        dots: None,
        grab: None,
        bands: bands.to_vec(),
        uid,
        zoom_preview: false,
        at: std::time::Instant::now(),
    }
}

/// `job` 整个视图的粗预览。
pub fn preview_with(job: &Job, retained: &mut Retained) -> Option<Done> {
    let view = coarse_view(&job.view);
    guarded(retained, |r| run(job, &view, r, true, None))
}

fn answering(d: Done, seq: u64) -> Done {
    Done { seq, ..d }
}

/// 两张完成的渲染里该显示的那张：更新请求的；同一请求里精修的优先。
pub fn better(a: Option<Done>, b: Done) -> Done {
    match a {
        Some(a) if a.seq > b.seq || (a.seq == b.seq && !a.coarse && b.coarse) => a,
        _ => b,
    }
}

/// 在 `retained` 上跑 `f`；panic 后丢掉可能画了一半的图。
fn guarded(retained: &mut Retained, f: impl FnOnce(&mut Retained) -> Option<Done>) -> Option<Done> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(retained))) {
        Ok(d) => d,
        Err(_) => {
            retained.clear();
            None
        }
    }
}

/// 一张覆盖**整页**的低分辨率图，铺在清晰贴图下面。
///
/// 平移时清晰贴图只覆盖「视口 + 一圈预渲染边」，越过预渲染边的条带在新图到达之前是**空的**
/// ——画布只能露出页面底色，等几十毫秒后那张图才补上。用户看到的就是「拖过去才开始加载」。
/// 这张图在空闲时一次画好并一直留着：新露出的条带立刻有一份正确的（只是糊一点的）内容，
/// 清晰图到了再盖上去。没有任何一帧是空白，加载过程于是不可见。
pub struct Overview {
    pub image: Arc<RenderImage>,
    pub view: View,
    /// 这张图描述的是哪一版内容：`(doc 指针, 页面, 字体代次)`。变了就该重画。
    pub key: (usize, NodeId, u64),
}

/// 一张兜底图最多这么多像素（≈6.4MB）。整页再大也只是更糊，不会更慢。
const OVERVIEW_PIXELS: f64 = 1_600_000.0;

/// 两次重画兜底图之间的最小间隔：空闲时按这个节奏跟上文档改动，不至于每敲一个字都重画一遍。
const OVERVIEW_GAP: std::time::Duration = std::time::Duration::from_millis(120);

/// 整页兜底图的视图：整页内容 + 一点余量，缩到像素预算之内。
pub fn overview_view(doc: &Document, page: NodeId) -> Option<View> {
    let b = doc.page_content_bounds(page)?;
    let pad = 64.0;
    let (x0, y0, x1, y1) = (b.x0 - pad, b.y0 - pad, b.x1 + pad, b.y1 + pad);
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (OVERVIEW_PIXELS / (w * h)).sqrt().min(1.0).max(1e-3);
    let fit = |n: f64| ((n * scale).ceil() as u32).clamp(1, 8_192);
    Some(View { origin: (x0, y0), scale, width: fit(w), height: fit(h) })
}

/// 画这张兜底图：和整页贴图走同一条路（让开材质节点、再由材质合成叠回去），所以玻璃与
/// 容器里的内容在糊的那一版里也是在的。
#[cfg(feature = "gpu")]
fn build_overview(job: &Job, retained: &mut Retained, mats: &mut Materials) -> Option<Overview> {
    let view = overview_view(&job.doc, job.page)?;
    let sub = Job {
        doc: job.doc.clone(),
        page: job.page,
        view,
        outline: false,
        outline_extras: figcraft_render::Outline::default(),
        outlined: Vec::new(),
        skip: Vec::new(),
        solo: None,
        proof: false,
        dots: None,
        grab: None,
        bands: Vec::new(),
        uid: job.uid,
        zoom_preview: false,
        at: std::time::Instant::now(),
    };
    let mut done = run(&sub, &sub.view, retained, false, None)?;
    apply_wgsl_materials(mats, &sub.doc, sub.page, &mut done);
    Some(Overview { image: done.image, view: done.view, key: (Arc::as_ptr(&sub.doc) as usize, sub.page, figcraft_text::layout_generation()) })
}

/// 没有 `gpu` feature 时同样给一张兜底图（只是不带外置材质）。
#[cfg(not(feature = "gpu"))]
fn build_overview(job: &Job, retained: &mut Retained) -> Option<Overview> {
    let view = overview_view(&job.doc, job.page)?;
    let sub = Job {
        doc: job.doc.clone(),
        page: job.page,
        view,
        outline: false,
        outline_extras: figcraft_render::Outline::default(),
        outlined: Vec::new(),
        skip: Vec::new(),
        solo: None,
        proof: false,
        dots: None,
        grab: None,
        bands: Vec::new(),
        uid: job.uid,
        zoom_preview: false,
        at: std::time::Instant::now(),
    };
    let done = run(&sub, &sub.view, retained, false, None)?;
    Some(Overview { image: done.image, view: done.view, key: (Arc::as_ptr(&sub.doc) as usize, sub.page, figcraft_text::layout_generation()) })
}

/// 后台渲染工作线程（对应原文的桌面 `Worker`）。
pub struct Worker {
    tx: std::sync::mpsc::Sender<(u64, Job)>,
    rx: std::sync::mpsc::Receiver<Done>,
    /// 整页兜底图（见 [`Overview`]）。和完成图分开两条通道：它不是「这一张请求的答案」，
    /// 晚到也不该顶掉任何人的台。
    overview_rx: std::sync::mpsc::Receiver<Overview>,
    submitted: u64,
    finished: Arc<std::sync::atomic::AtomicU64>,
    /// 看门狗正在画补做的预览，或者手上已经有一张要交给 UI 的预览。
    /// 它也算「在飞」：否则补做的预览画好了却没人续帧，要等下一次输入才上台。
    previewing: Arc<std::sync::atomic::AtomicBool>,
    /// 图完成后叫醒 UI。
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Worker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, jobs) = std::sync::mpsc::channel::<(u64, Job)>();
        // 有界：最多 2 张完成图在飞。无界通道在 UI 卡顿时会堆几十张整页图（每张 17.6MB）。
        let (done_tx, rx) = std::sync::mpsc::sync_channel::<Done>(2);
        // 整页兜底图单独一条通道（只留最新的一张）。
        let (overview_tx, overview_rx) = std::sync::mpsc::sync_channel::<Overview>(1);
        let finished = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let done_with = finished.clone();
        let (watch, watched) = std::sync::mpsc::channel::<(u64, Job, std::time::Instant)>();
        let refined = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let refined_seen = refined.clone();
        let previewing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let busy = previewing.clone();

        #[cfg(feature = "gpu")]
        let mat_share: std::sync::Arc<std::sync::Mutex<Materials>> = std::sync::Arc::new(std::sync::Mutex::new(Materials::default()));
        #[cfg(feature = "gpu")]
        let mats_watch = mat_share.clone();
        #[cfg(feature = "gpu")]
        let mats_render = mat_share.clone();

        let _wake_preview = wake.clone();
        let (preview_tx, preview_wake) = (done_tx.clone(), wake.clone());
        let watchdog = std::thread::Builder::new()
            .name("figcraft-render-preview".into())
            // 渲染按层递归，深文档会吃掉默认 2 MiB 栈（上游 #1082 的修法）。
            .stack_size(figcraft_render::RENDER_STACK)
            .spawn(move || {
                let mut retained = Retained::default();
                let after = std::time::Duration::from_secs_f64(PREVIEW_AFTER_MS / 1000.0);
                while let Ok((mut seq, mut job, mut started)) = watched.recv() {
                    loop {
                        match watched.recv_timeout(after.saturating_sub(started.elapsed())) {
                            Ok(newer) => (seq, job, started) = newer,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    // 从看它一眼开始忙，到预览交出去、叫醒 UI 为止，这样 `in_flight` 一直成立；
                    // 精修图先到的话（`refined` 已记账）这里看到就什么也不交。
                    busy.store(true, std::sync::atomic::Ordering::SeqCst);
                    let still_rendering = || refined_seen.load(std::sync::atomic::Ordering::SeqCst) < seq;
                    let mut sent = true;
                    if still_rendering()
                        && let Some(mut d) = preview_with(&job, &mut retained)
                        && still_rendering()
                    {
                        // 补做的预览也得带材质，理由同 `Renders::preview`：不带材质的预览是另一张图。
                        #[cfg(feature = "gpu")]
                        apply_wgsl_materials(&mut mats_watch.lock().unwrap_or_else(|e| e.into_inner()), &job.doc, job.page, &mut d);
                        sent = preview_tx.try_send(answering(d, seq)).is_ok();
                        preview_wake();
                    }
                    busy.store(false, std::sync::atomic::Ordering::SeqCst);
                    if !sent {
                        break;
                    }
                }
            });
        if let Err(e) = watchdog {
            log::error!("render preview thread: {e}");
        }

        let wake_done = wake.clone();
        let overview_wake = wake.clone();
        let spawned = std::thread::Builder::new().name("figcraft-render".into()).stack_size(figcraft_render::RENDER_STACK).spawn(move || {
            let mut renders = Renders::default();
            #[cfg(feature = "gpu")]
            {
                renders.mats = mats_render;
            }
            // 整页兜底图：一份自留的 `Retained`（它和清晰图用的尺度完全不同，共用会把增量缓存
            // 每次打废），以及「已经画过哪一版内容」和上次画它的时刻。
            let mut overview_retained = Retained::default();
            #[allow(unused_mut)]
            let mut overview_key: Option<(usize, NodeId, u64)> = None;
            let mut overview_at = std::time::Instant::now();
            let mut next = None;
            // 阶段 2b：异步材质帧的在飞队列 + 交给 UI 前的去重/暂存中转。两者都只在本线程碰。
            #[cfg(feature = "gpu")]
            let mut pending: Vec<PendingFrame> = Vec::new();
            #[cfg(feature = "gpu")]
            let mut handoff = Handoff::default();
            loop {
                // ── 自驱核心：每一轮都先推进 GPU 回调、收集已完成的异步帧并交给 UI ──
                // 没有新 job 的 tick 轮也要走这里，否则 map_async 回调没人 poll、那张图永远收不回。
                // `pump_and_deliver` 还保证「通道满暂存的 deferred 帧」在 pending 空时也会被重试送出，
                // 水位只在真送出后推进——漏掉这一点就会让缩放后的最后一帧卡在 render 线程。
                #[cfg(feature = "gpu")]
                if !pump_and_deliver(&renders.mats, &mut pending, &mut handoff, &done_tx, &done_with, &refined, &wake_done) {
                    break;
                }

                // ── 取下一张 job ──
                // 有异步帧在飞时用 2ms 短 tick 自驱（绝不阻塞等新活，那样异步帧就卡在 GPU 上了）；
                // 队列空时照旧阻塞在 recv()，空闲零开销。preview / overview / watchdog 一键不变。
                #[cfg(feature = "gpu")]
                let pulled = if let Some(x) = next.take() {
                    Pulled::Job(x)
                } else if pending.is_empty() && handoff.deferred.is_none() {
                    match jobs.recv() {
                        Ok(x) => Pulled::Job(x),
                        Err(_) => Pulled::Closed,
                    }
                } else {
                    match jobs.recv_timeout(ASYNC_TICK) {
                        Ok(x) => Pulled::Job(x),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Pulled::Tick,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Pulled::Closed,
                    }
                };
                #[cfg(feature = "gpu")]
                let (mut seq, mut job) = match pulled {
                    Pulled::Job(x) => x,
                    Pulled::Tick => continue, // 回去继续 collect_ready / pump
                    Pulled::Closed => break,
                };
                #[cfg(not(feature = "gpu"))]
                let Some((mut seq, mut job)) = next.take().or_else(|| jobs.recv().ok()) else { break };
                while let Ok(newer) = jobs.try_recv() {
                    (seq, job) = newer;
                }
                if let Some(d) = renders.preview(&job) {
                    if matches!(done_tx.try_send(answering(d, seq)), Err(std::sync::mpsc::TrySendError::Disconnected(_))) {
                        break;
                    }
                    wake_done();
                    if let Ok(newer) = jobs.try_recv() {
                        next = Some(newer);
                        continue;
                    }
                }
                if renders.wants_late_preview(&job) {
                    let _ = watch.send((seq, job.clone(), std::time::Instant::now()));
                }

                #[cfg(feature = "gpu")]
                {
                    // 精修走异步优先：能安全异步就提交后立即回来继续收/取活，CPU 光栅那几毫秒
                    // 仍当帧做完（`rasterize`），只有 GPU readback 被推迟。退回同步的帧走同一条
                    // `deliver`，与旧路径逐像素一致。
                    match renders.refine_async(&job) {
                        RefineOutcome::Submitted { id, base, w, h, key } => {
                            pending.push(PendingFrame { id, seq, done: answering(base, seq), w, h, key });
                        }
                        RefineOutcome::Ready(d) => {
                            if !deliver(&mut handoff, vec![answering(d, seq)], &done_tx, &done_with, &refined, &wake_done) {
                                break;
                            }
                        }
                        RefineOutcome::None => {}
                    }
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let done = renders.refine(&job);
                    refined.store(seq, std::sync::atomic::Ordering::Release);
                    if let Some(d) = done
                        && matches!(done_tx.try_send(answering(d, seq)), Err(std::sync::mpsc::TrySendError::Disconnected(_)))
                    {
                        break;
                    }
                    wake_done();
                    done_with.store(seq, std::sync::atomic::Ordering::Release);
                }

                // 队列空了才画兜底图：它的唯一用途是「有一份总在届时的低清底」，晚几毫秒无妨，
                // 但绝不能把下一次请求往后推。
                if !jobs.try_recv().is_ok_and(|newer| {
                    next = Some(newer);
                    true
                }) && overview_at.elapsed() >= OVERVIEW_GAP
                {
                    let due = (Arc::as_ptr(&job.doc) as usize, job.page, figcraft_text::layout_generation());
                    if overview_key != Some(due) {
                        #[cfg(feature = "gpu")]
                        let built = build_overview(&job, &mut overview_retained, &mut renders.mats.lock().unwrap_or_else(|e| e.into_inner()));
                        #[cfg(not(feature = "gpu"))]
                        let built = build_overview(&job, &mut overview_retained);
                        if let Some(o) = built {
                            overview_key = Some(o.key);
                            overview_at = std::time::Instant::now();
                            let _ = overview_tx.try_send(o);
                            overview_wake();
                        }
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("render thread: {e}");
        }

        Self { tx, rx, overview_rx, submitted: 0, finished, previewing, wake }
    }

    pub fn submit(&mut self, job: Job) {
        if self.tx.send((self.submitted + 1, job)).is_ok() {
            self.submitted += 1;
        }
    }

    /// 还有提交过的请求在渲染、没被 poll 到，或者看门狗的补做预览正忙（它也算在飞）。
    pub fn in_flight(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::SeqCst) < self.submitted || self.previewing.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 已经交代完的请求数。**先读它、再 [`Self::poll`]**：worker 总是「先发结果、再记账」，
    /// 所以读到它和 [`Self::submitted`] 相等，就保证所有结果都已经躺在通道里，
    /// `poll` 一定拿得到——不会出现「图好了、却没人在画」的那一帧。
    pub fn finished(&self) -> u64 {
        self.finished.load(std::sync::atomic::Ordering::Acquire)
    }

    /// 已提交的请求数。
    pub fn submitted(&self) -> u64 {
        self.submitted
    }

    /// 自上次轮询以来完成的渲染里最好的那张。
    pub fn poll(&mut self) -> Option<Done> {
        let mut best = None;
        while let Ok(d) = self.rx.try_recv() {
            best = Some(better(best, d));
        }
        best
    }

    /// 最新的一张整页兜底图（没有新的就 `None`）。
    pub fn poll_overview(&mut self) -> Option<Overview> {
        let mut best = None;
        while let Ok(o) = self.overview_rx.try_recv() {
            best = Some(o);
        }
        best
    }

    pub fn wake(&self) -> &Arc<dyn Fn() + Send + Sync> {
        &self.wake
    }
}

/// `FIGCRAFT_GPU=1` 才走 GPU（还要这个构建带了 `gpu` feature）。
#[cfg(feature = "gpu")]
fn gpu_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FIGCRAFT_GPU").is_ok_and(|v| v != "0" && !v.is_empty()))
}

/// 用 GPU 画这一帧；压不出场景（或这个作业带着 CPU 专属的活）时返回 `None`，调用方回退。
///
/// 作业里这些字段 GPU 路径都不做，直接让出去：轮廓模式、只画某层、跳过某些层、
/// 画布提示带、拖动替身抓取、点阵、软打样。
#[cfg(feature = "gpu")]
fn run_gpu(job: &Job, view: &View, gpu: &mut crate::gpu::Gpu) -> Option<Done> {
    if job.outline || job.solo.is_some() || !job.skip.is_empty() || !job.bands.is_empty() || job.grab.is_some() || job.dots.is_some() || job.proof {
        return None;
    }
    let t0 = std::time::Instant::now();
    let scene = crate::gpu::scene_of(&job.doc, job.page, view)?;
    // `Gpu::render` 给的正是**预乘** RGBA（和 tiny-skia 的 Pixmap 同一种排布）。
    let mut data = gpu.render(&scene);
    if data.len() != (view.width as usize) * (view.height as usize) * 4 {
        return None;
    }
    premul_rgba_to_bgra(&mut data);
    let image = wrap_bgra(view.width, view.height, data);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if render_log() {
        eprintln!("[render] {}x{} {ms:.1}ms gpu rects={}", view.width, view.height, scene.rects.len());
    }
    Some(Done {
        image,
        grab: None,
        view: view.clone(),
        skip: job.skip.clone(),
        ms,
        work: Work::Full,
        uid: job.uid,
        page: job.page,
        coarse: false,
        at: job.at,
        seq: 0,
    })
}

/// 同步从零渲染一次（测试/截图用）。
pub fn render_now(job: &Job) -> Option<Done> {
    run(job, &job.view, &mut Retained::default(), false, None)
}

#[cfg(test)]
mod live_clip_tests {
    use super::*;
    /// The canvas's stand-in for a layer dragged half out of a frame must be cut off by the frame
    /// the way the page render cuts it (the app's own sample document).
    #[test]
    fn the_stand_in_is_clipped_by_the_frame_around_it() {
        let mut doc = crate::app::sample_doc();
        let page = doc.pages[0];
        let frame = doc.node(page).and_then(|n| n.children.first().copied()).expect("Frame 1");
        let blue = doc.node(frame).and_then(|n| n.children.first().copied()).expect(tr("Blue"));
        // Drag it left so half of it is past the frame's left edge.
        let t = doc.get_mut(blue).expect("a node");
        t.transform = figcraft_geom::Affine::translate((-110.0, 0.0)) * t.transform;

        let bounds = doc.bounds_of(&[blue]).expect("bounds");
        let pad = 32.0;
        let page_rect = [bounds.x0 - pad, bounds.y0 - pad, bounds.x1 + pad, bounds.y1 + pad];
        let scale = 1.745;
        let w = ((page_rect[2] - page_rect[0]) * scale).ceil() as u32;
        let h = ((page_rect[3] - page_rect[1]) * scale).ceil() as u32;
        let view = View { origin: (page_rect[0], page_rect[1]), scale, width: w, height: h };
        let opts = figcraft_render::Options {
            background: Some(figcraft_color::Color::TRANSPARENT),
            only: vec![blue],
            lod: true,
            magnified_image_pixels: true,
            ..Default::default()
        };
        let doc = Arc::new(doc);
        // Both paths: the plain one and the retained one the grab uses.
        let plain = figcraft_render::render_view(&doc, page, &view, &opts).expect("a render");
        let mut retained = Retained::default();
        let (view2, kept, _) = retained.render(&doc, page, &view, &opts).expect("a render");
        let at = |v: &View, src: &Pixmap, page_x: f64, page_y: f64| {
            let x = ((page_x - v.origin.0) * v.scale) as u32;
            let y = ((page_y - v.origin.1) * v.scale) as u32;
            src.pixel(x, y).map(|c| c.alpha()).unwrap_or(0)
        };
        assert_eq!(at(&view, &plain, 20.0, 150.0), 0, "plain: left of the frame cut off");
        assert!(at(&view, &plain, 100.0, 150.0) > 0, "plain: inside the frame drawn");
        assert_eq!(at(&view2, kept, 20.0, 150.0), 0, "retained: left of the frame cut off");
        assert!(at(&view2, kept, 100.0, 150.0) > 0, "retained: inside the frame drawn");
    }
}

/// 外置材质挂在**容器**上时，容器里装的东西必须还在。
///
/// 旧实现把材质节点整个塞进 `Options::skip`（连子树一起挖空）再在整页图上做离屏合成，
/// 于是容器子节点永远不画；掉帧时连材质层本身都跟着没了。这些测试钉住「前景层」这条新路：
/// 容器子节点单独渲成透明层，材质合成之后叠回去，静态图与拖动补丁都走它。
#[cfg(all(test, feature = "gpu"))]
mod material_foreground_tests {
    use super::*;
    use figcraft_color::{Color, Paint};
    use figcraft_doc::NodeKind;

    /// 一个绑了（故意加载不到的）外置材质的 Frame，里面放一个红色矩形。
    /// 材质 shader 是否存在不影响前景层：只要 `plugin_data` 里有绑定，子层就得回来。
    fn glass_container() -> (Document, NodeId) {
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Glass", 0.0, 0.0, 100.0, 100.0).expect("a frame");
        let binding = crate::wgsl_material::WgslMaterialBinding::new("missing.wgsl".to_string());
        doc.get_mut(frame).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let chip = doc.create(frame, NodeKind::Rectangle, "Chip", 20.0, 20.0, 60.0, 60.0).expect("a chip");
        doc.get_mut(chip).expect("the chip").fills = vec![Paint::solid(Color::from_hex("FF2020").expect("hex"))];
        (doc, page)
    }

    fn view() -> View {
        View { origin: (0.0, 0.0), scale: 1.0, width: 100, height: 100 }
    }

    #[test]
    fn a_material_container_keeps_its_children_in_the_foreground() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_container();
        let front = material_foreground(&doc, page, &view(), 100, 100).expect("a foreground layer");
        let alpha = |x: u32, y: u32| front[((y * 100 + x) * 4 + 3) as usize];
        assert!(alpha(50, 50) > 200, "子节点中心必须画出来");
        assert_eq!(alpha(5, 5), 0, "容器自身的填充不能混进前景层");
    }

    #[test]
    fn a_plain_material_shape_needs_no_foreground() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).expect("a rectangle");
        let binding = crate::wgsl_material::WgslMaterialBinding::new("missing.wgsl".to_string());
        doc.get_mut(rect).expect("the rectangle").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        assert!(material_foreground(&doc, page, &view(), 100, 100).is_none());
    }

    #[test]
    fn the_foreground_is_composited_over_the_material() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut scene = vec![0u8; 4];
        scene.copy_from_slice(&[0, 0, 255, 255]); // straight RGBA：不透明蓝
        over_straight(&mut scene, &[255, 0, 0, 255]); // 不透明红
        assert_eq!(scene, [255, 0, 0, 255], "不透明子层完全盖住材质");
        let mut scene = vec![0u8; 4];
        scene.copy_from_slice(&[0, 0, 255, 255]);
        over_straight(&mut scene, &[255, 0, 0, 128]); // 半透明红
        assert!(scene[0] > 100 && scene[2] > 100, "半透明子层要混合: {scene:?}");
    }

    /// region 化兜底的落位契约：材质节点偏在画布右下角时，容器矩形和红色子节点必须画在
    /// **原来的世界坐标**，画布其它地方保持透明。这条钉住墨迹框偏移 / 子图 stride / pad 的计算——
    /// 一旦 view 裁剪或 `over_straight_at` 的行偏移算错，红子会跑到左上角或整块错位。
    #[test]
    fn cpu_fallback_lands_an_offset_material_at_its_world_position() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, NodeKind::Frame, "Glass", 100.0, 100.0, 60.0, 60.0).expect("a frame");
        let binding = crate::wgsl_material::WgslMaterialBinding::new("missing.wgsl".to_string());
        doc.get_mut(frame).expect("frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let chip = doc.create(frame, NodeKind::Rectangle, "Chip", 10.0, 10.0, 40.0, 40.0).expect("a chip");
        doc.get_mut(chip).expect("chip").fills = vec![Paint::solid(Color::from_hex("FF2020").expect("hex"))];

        let (w, h) = (200u32, 200u32);
        let mut done = Done {
            image: wrap_bgra(w, h, vec![0u8; (w * h * 4) as usize]),
            view: View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h },
            skip: Vec::new(),
            grab: None,
            ms: 0.0,
            work: figcraft_render::Work::Full,
            uid: 0,
            page,
            coarse: false,
            at: std::time::Instant::now(),
            seq: 0,
        };
        cpu_restore_materials(&doc, page, &mut done);
        let bgra = done.image.as_bytes(0).expect("pixels");
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [bgra[i], bgra[i + 1], bgra[i + 2], bgra[i + 3]]
        };
        // 红色子节点在世界坐标 (110..150, 110..150)，中心 (130,130)。
        assert!(at(130, 130)[3] > 200, "红子必须画在偏移后的世界坐标: {:?}", at(130, 130));
        assert!(at(130, 130)[2] > 200 && at(130, 130)[0] < 60, "子节点保持红色（BGRA）: {:?}", at(130, 130));
        // 容器自身矩形（Frame 默认白填充）在 (100,100) 附近、红子之外。
        assert!(at(105, 105)[3] > 0, "容器矩形要在自己的位置: {:?}", at(105, 105));
        // 墨迹框之外（画布左上）必须仍全透明——region 化不能污染框外，也不能整页重画。
        assert_eq!(at(10, 10), [0, 0, 0, 0], "框外保持透明，证明只画了材质墨迹框: {:?}", at(10, 10));
        assert_eq!(at(190, 190), [0, 0, 0, 0], "右下框外同样透明: {:?}", at(190, 190));
    }

    #[test]
    fn material_ids_lists_only_bound_nodes_and_skips_the_page() {
        let (doc, page) = glass_container();
        let ids = material_ids(&doc, page);
        assert_eq!(ids.len(), 1, "只有容器绑了材质");
        assert!(!ids.contains(&page), "Page 不能算作材质节点");
        let frame = ids[0];
        assert_eq!(doc.node(frame).map(|n| n.name.as_str()), Some("Glass"));
    }

    /// 回退的核心契约：一张**已被挖空**的画布（材质节点连子树都不在），CPU 兜底必须把容器
    /// 自身的矩形**和**里面的红色子节点一起画回来。少了容器矩形就是「图浮在空中/边框没了」，
    /// 少了子节点就是用户报的「容器不闪但内容图不见了」。
    #[test]
    fn cpu_fallback_restores_the_container_and_its_children() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_container();
        let mut done = crate::canvas::render_worker::Done {
            image: wrap_bgra(100, 100, vec![0u8; 100 * 100 * 4]),
            view: view(),
            skip: Vec::new(),
            grab: None,
            ms: 0.0,
            work: figcraft_render::Work::Full,
            uid: 0,
            page,
            coarse: false,
            at: std::time::Instant::now(),
            seq: 0,
        };
        cpu_restore_materials(&doc, page, &mut done);
        let bgra = done.image.as_bytes(0).expect("pixels");
        let at = |x: u32, y: u32| {
            let i = ((y * 100 + x) * 4) as usize;
            [bgra[i], bgra[i + 1], bgra[i + 2], bgra[i + 3]]
        };
        assert!(at(50, 50)[3] > 200, "红色子节点必须画回来（容器中心）: {:?}", at(50, 50));
        assert!(at(50, 50)[2] > 200 && at(50, 50)[0] < 60, "子节点是红色（BGRA）: {:?}", at(50, 50));
        assert!(at(10, 10)[3] > 0, "容器自身的矩形也要画回来，不只是浮空的子节点: {:?}", at(10, 10));
    }

    /// 完整链路兜底：编译带 gpu feature、但这一帧 GPU 没能补回材质（这里用加载不到的 shader
    /// 恒定触发，等价于「带 gpu 的二进制跑在没有 adapter 的设备上」）。`apply_wgsl_materials`
    /// 必须在返回前把被 `run` 挖空的容器和子节点用 CPU 画回，绝不能留下一张挖空的画布。
    /// 测试机有无 adapter 都能过：missing.wgsl 恒走失败分支，不依赖真实 GPU。
    #[test]
    fn apply_wgsl_materials_falls_back_when_gpu_cannot_restore() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_container();
        // 一张「已被挖空」的画布：材质节点连子树都不在里面，全透明。
        let mut done = Done {
            image: wrap_bgra(100, 100, vec![0u8; 100 * 100 * 4]),
            view: view(),
            skip: Vec::new(),
            grab: None,
            ms: 0.0,
            work: figcraft_render::Work::Full,
            uid: 0,
            page,
            coarse: false,
            at: std::time::Instant::now(),
            seq: 0,
        };
        let mut mats = Materials::default();
        apply_wgsl_materials(&mut mats, &doc, page, &mut done);
        let bgra = done.image.as_bytes(0).expect("pixels");
        let at = |x: u32, y: u32| {
            let i = ((y * 100 + x) * 4) as usize;
            [bgra[i], bgra[i + 1], bgra[i + 2], bgra[i + 3]]
        };
        assert!(at(50, 50)[3] > 200, "GPU 补不回时红色子节点必须被兜底画回: {:?}", at(50, 50));
        assert!(at(50, 50)[2] > 200 && at(50, 50)[0] < 60, "子节点保持红色: {:?}", at(50, 50));
        assert!(at(10, 10)[3] > 0, "容器自身矩形也要回来: {:?}", at(10, 10));
    }

    /// 关键回归：局部同步渲染走的是同一条 `run` + `apply_wgsl_materials`，
    /// 所以材质容器里的子节点不会因为 view 被裁小就消失。
    #[test]
    fn the_drag_patch_shows_a_material_container_children() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_container();
        let job = patch_job(Arc::new(doc), page, view(), &[], 1);
        let done = Renders::default().render_patch(&job).expect("a drag patch");
        let bgra = done.image.as_bytes(0).expect("pixels");
        // `Retained::render` 会把 view 向外吸附到整设备像素（100 → 101），行距要用真实图宽。
        let sz = done.image.size(0);
        let (w, h) = (u32::from(sz.width), u32::from(sz.height));
        let i = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(bgra[i + 2] > 200 && bgra[i] < 60, "红色子节点要出现在补丁中心: {:?}", &bgra[i..i + 4]);
    }
}

/// 阶段 2b：refine 走「CPU 光栅同步 + 材质异步 readback」时的正确性回归。
///
/// 钉住三条契约：
/// 1. [`Handoff`] 不回退、不丢最新帧 —— 纯逻辑，任何平台（只要开了 `gpu` feature）都跑；
/// 2. 异步收集回来的像素与同步 `refine` 逐字节一致 —— 有真实 adapter 时验证；无 adapter 时
///    `refine_async` 合法退回同步，同样要求一致；
/// 3. 有材质时异步帧确实把玻璃画进了未合成底图（不是「有子层、没玻璃」的透传）。
#[cfg(all(test, feature = "gpu"))]
mod async_refine_tests {
    use super::*;

    /// 只带序号的最小 Done，供 Handoff 排序 / 水位测试用。
    fn done_with_seq(seq: u64) -> Done {
        let page = figcraft_doc::Document::default().pages[0];
        Done {
            image: wrap_bgra(2, 2, vec![0u8; 2 * 2 * 4]),
            view: View { origin: (0.0, 0.0), scale: 1.0, width: 2, height: 2 },
            skip: Vec::new(),
            grab: None,
            ms: 0.0,
            work: Work::Full,
            uid: seq,
            page,
            coarse: false,
            at: std::time::Instant::now(),
            seq,
        }
    }

    /// 不回退：一批乱序帧只发最新那张；晚到的旧帧既不上台、也不把水位拉回去。
    #[test]
    fn handoff_never_rewinds_and_sends_only_the_newest_of_a_batch() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Done>(2);
        let mut h = Handoff::default();

        assert_eq!(h.send(vec![done_with_seq(3), done_with_seq(1), done_with_seq(2)], &tx), Ok(Some(3)));
        assert_eq!(rx.recv().expect("a frame").seq, 3, "only the newest frame of the batch crosses to the UI");

        // 晚到的旧帧被 retain 掉，水位停在 3。
        assert_eq!(h.send(vec![done_with_seq(1), done_with_seq(2)], &tx), Ok(None));
        assert!(rx.try_recv().is_err(), "stale frames never reach the UI");
        assert_eq!(h.highest, 3, "the water mark stays at the highest seq already sent");

        // 严格更新的帧照常放行。
        assert_eq!(h.send(vec![done_with_seq(4)], &tx), Ok(Some(4)));
        assert_eq!(rx.recv().expect("a frame").seq, 4);
        assert_eq!(h.highest, 4);
    }

    /// 不丢最新帧：通道满时暂存这一批里 seq 最大的那张、水位一步不动；更晚到的更新帧替换
    /// 暂存帧；UI 腾出通道后靠下一轮自驱重试补发，一张不丢；已被水位盖过的帧再晚到也丢。
    #[test]
    fn handoff_defers_latest_under_backpressure_and_retries() {
        // 容量 1，先塞一张进去模拟「UI 还没取走上一帧」。
        let (tx, rx) = std::sync::mpsc::sync_channel::<Done>(1);
        tx.try_send(done_with_seq(1)).expect("seed the channel");
        let mut h = Handoff::default();

        assert_eq!(h.send(vec![done_with_seq(5), done_with_seq(4)], &tx), Ok(None));
        assert_eq!(h.highest, 0, "nothing was sent, so the water mark does not move");
        assert_eq!(h.deferred.as_ref().map(|d| d.seq), Some(5), "the newest frame is parked, not dropped");

        // 仍然满：又来一张更新的，deferred 必须被 7 替换，绝不能留着旧的 5。
        assert_eq!(h.send(vec![done_with_seq(7)], &tx), Ok(None));
        assert_eq!(h.deferred.as_ref().map(|d| d.seq), Some(7), "a newer frame supersedes the deferred one");
        assert_eq!(h.highest, 0);

        // UI 取走占位那张，通道腾出空位。
        assert_eq!(rx.recv().expect("the seeded frame").seq, 1);

        // 下一轮自驱重试：暂存的 7 补发出去。
        assert_eq!(h.send(vec![], &tx), Ok(Some(7)));
        assert_eq!(rx.recv().expect("the deferred newest").seq, 7);
        assert_eq!(h.highest, 7);
        assert!(h.deferred.is_none(), "the deferred slot is cleared once delivered");

        // 已经比水位旧的帧即使再晚到也直接丢，绝不回退。
        assert_eq!(h.send(vec![done_with_seq(6)], &tx), Ok(None));
        assert!(rx.try_recv().is_err(), "a frame older than the water mark is dropped");
    }

    /// 阶段 2b 卡死回归：pending 空、deferred 有帧、通道又满时，render 线程的 tick 轮**仍必须**
    /// 反复重试送出那张暂存帧，否则它永远到不了 UI、`in_flight()` 恒 true、画面冻在上一张。
    /// `pump_and_deliver` 把「collect（这里是空）+ 交付 deferred」合并成一步，杜绝被 `frames.is_empty()`
    /// 短路跳过。这里不碰 GPU：pending 空时 `collect_ready` 直接返回，连 runner 都不需要。
    #[test]
    fn a_deferred_frame_is_retried_when_no_gpu_frame_is_in_flight() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Done>(1);
        tx.try_send(done_with_seq(1)).expect("seed the channel");
        let mut h = Handoff::default();
        // 通道满 → 最新帧 3 被暂存，水位不动。
        assert_eq!(h.send(vec![done_with_seq(3)], &tx), Ok(None));
        assert_eq!(h.highest, 0);

        let mats = std::sync::Arc::new(std::sync::Mutex::new(Materials::default()));
        let mut pending: Vec<PendingFrame> = Vec::new(); // 没有任何 GPU 帧在飞
        let done_with = std::sync::atomic::AtomicU64::new(0);
        let refined = std::sync::atomic::AtomicU64::new(0);
        let wake = std::sync::Arc::new(|| {}) as Arc<dyn Fn() + Send + Sync>;

        // 通道仍满：这一轮空转也不能把 deferred 弄丢、不能推进水位。
        assert!(pump_and_deliver(&mats, &mut pending, &mut h, &tx, &done_with, &refined, &wake));
        assert_eq!(h.deferred.as_ref().map(|d| d.seq), Some(3), "still parked, not dropped");
        assert_eq!(done_with.load(std::sync::atomic::Ordering::SeqCst), 0, "water mark does not move");

        // UI 取走占位帧，下一轮 tick（依旧 pending 空）必须把暂存的 3 补发出去。
        assert_eq!(rx.recv().expect("the seeded frame").seq, 1);
        assert!(pump_and_deliver(&mats, &mut pending, &mut h, &tx, &done_with, &refined, &wake));
        assert_eq!(rx.recv().expect("the deferred frame is retried").seq, 3);
        assert_eq!(done_with.load(std::sync::atomic::Ordering::SeqCst), 3, "water mark advances once really sent");
        assert_eq!(refined.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert!(h.deferred.is_none(), "the deferred slot is cleared");
    }

    fn glass_bound_square() -> (figcraft_doc::Document, NodeId) {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let r = doc.create(page, figcraft_doc::NodeKind::Rectangle, "R", 20.0, 20.0, 100.0, 100.0).expect("a rectangle");
        doc.get_mut(r).expect("the rectangle").fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex("FF2020").expect("hex"))];
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(r).expect("the rectangle").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        (doc, page)
    }

    fn job_for(doc: figcraft_doc::Document, page: NodeId) -> Job {
        Job {
            doc: Arc::new(doc),
            page,
            view: View { origin: (0.0, 0.0), scale: 1.0, width: 140, height: 140 },
            outline: false,
            outline_extras: figcraft_render::Outline::default(),
            outlined: Vec::new(),
            skip: Vec::new(),
            solo: None,
            proof: false,
            dots: None,
            grab: None,
            bands: Vec::new(),
            uid: 1,
            zoom_preview: false,
            at: std::time::Instant::now(),
        }
    }

    fn pixels(d: &Done) -> Vec<u8> {
        d.image.as_bytes(0).map(|b| b.to_vec()).unwrap_or_default()
    }

    /// 按 render 线程的方式把一张已提交的异步帧收集成可上台的 Done，带 2000 轮上限防止坏驱动死等。
    fn collect_one(renders: &Renders, id: u64, base: Done, w: u32, h: u32, key: u64) -> Done {
        let seq = base.seq.max(1);
        let mut pending = vec![PendingFrame { id, seq, done: answering(base, seq), w, h, key }];
        let mats = renders.mats.clone();
        let mut ready = Vec::new();
        for _ in 0..2000 {
            collect_ready(&mats, &mut pending, &mut ready);
            if ready.len() == 1 {
                break;
            }
            std::thread::yield_now();
        }
        assert!(pending.is_empty(), "the submitted frame is collected and releases its readback slot");
        assert_eq!(ready.len(), 1, "a submitted async frame becomes exactly one displayable Done");
        ready.remove(0)
    }

    /// 阶段 2b 的载荷正确性：异步收集回来的像素必须与同步 `refine` 逐字节一致。
    /// 有 adapter 时走 `Submitted`；无 adapter 时 `refine_async` 合法退回 `Ready`，同样要求一致。
    #[test]
    fn async_refine_is_pixel_identical_to_sync_refine() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_bound_square();
        let job = job_for(doc, page);

        // 同一台 Renders（同一 device）：先拿同步结果，再走异步，排除设备间差异。
        let mut renders = Renders::default();
        let sync = pixels(&renders.refine(&job).expect("a sync render"));

        let outcome = renders.refine_async(&job);
        let (async_px, submitted) = match outcome {
            RefineOutcome::Submitted { id, base, w, h, key } => (pixels(&collect_one(&renders, id, base, w, h, key)), true),
            RefineOutcome::Ready(d) => (pixels(&d), false),
            RefineOutcome::None => panic!("a bound document must produce a picture"),
        };

        assert_eq!(async_px.len(), sync.len(), "same canvas size");
        let mut diff = 0usize;
        for (a, b) in sync.iter().zip(async_px.iter()) {
            if a != b {
                diff += 1;
            }
        }
        assert_eq!(diff, 0, "async refine must be pixel-for-pixel identical to sync (submitted={submitted}): {diff} bytes differ");
        if !submitted {
            eprintln!("no GPU adapter — async refine took the synchronous fallback (still byte-identical)");
        }
    }

    /// 有真实 adapter 时，带材质的帧必须真的走 `Submitted`，且收集后玻璃像素确实画进了未合成
    /// 底图——防止 `ensure_pipelines` 漏判时「只画子层、透传没玻璃」的错图被 async 帧带上台。
    #[test]
    fn submitted_async_refine_actually_draws_the_material() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (doc, page) = glass_bound_square();
        let job = job_for(doc, page);

        let mut renders = Renders::default();
        let (id, base, w, h, key) = match renders.refine_async(&job) {
            RefineOutcome::Submitted { id, base, w, h, key } => (id, base, w, h, key),
            RefineOutcome::Ready(_) => {
                eprintln!("no GPU adapter — skipping the submitted-branch assertion");
                return;
            }
            RefineOutcome::None => panic!("a bound document must produce a picture"),
        };

        let before = pixels(&base);
        let done = collect_one(&renders, id, base, w, h, key);
        let after = pixels(&done);
        assert_ne!(after, before, "the material composite must change the uncomposited base — glass was really drawn, not passed through");
    }

    /// 拖动一层时画布自己画它、底图 skip 它：底图与 passes 逐字节没变，就绝不能再把这一帧送去
    /// GPU——送一次就是一次 readback + 一张新纹理 + 一次整页重传，而画面一个像素都不会变。
    /// 这条把 A1（被 skip 的层不算 damage → 底图 `Reused`）与 A2（合成复用）串起来验证。
    #[test]
    fn a_drag_frame_hands_back_the_same_composited_picture() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (mut doc, page) = glass_bound_square();
        let dragged = doc.create(page, figcraft_doc::NodeKind::Rectangle, "Drag", 4.0, 4.0, 12.0, 12.0).expect("a rectangle");
        let mut job = job_for(doc.clone(), page);
        job.skip = vec![dragged];

        let mut renders = Renders::default();
        let first = match renders.refine_async(&job) {
            RefineOutcome::Ready(d) => d.image,
            RefineOutcome::Submitted { id, base, w, h, key } => collect_one(&renders, id, base, w, h, key).image,
            RefineOutcome::None => panic!("a bound document must produce a picture"),
        };

        // 拖动只挪「画布自己画的那一层」：它在底图里被 skip，材质那一层一动没动。
        doc.get_mut(dragged).expect("the dragged layer").transform = figcraft_geom::Affine::translate((18.0, 12.0));
        job.doc = Arc::new(doc);
        match renders.refine_async(&job) {
            RefineOutcome::Ready(d) => {
                assert!(Arc::ptr_eq(&d.image, &first), "the drag frame must reuse the composited picture, not build another one");
            }
            RefineOutcome::Submitted { .. } => panic!("the composite cannot have changed, yet it was submitted to the GPU again"),
            RefineOutcome::None => panic!("a bound document must produce a picture"),
        }
    }
}

/// WGSL 材质必须真的出现在**精修图**里：`refine` 是画布实际显示的出口。
///
/// 这条测试跨越了「面板写 plugin_data」与「屏幕像素」之间的整条缝：少了 `apply_wgsl_materials`
/// 这一步，`passes_for` 与 `composite` 各自的单测都能通过，而用户看到的仍然是一张没有材质的图。
#[cfg(all(test, feature = "gpu"))]
mod material_apply_tests {
    use super::*;

    fn job_for(doc: figcraft_doc::Document, page: NodeId) -> Job {
        job_for_at_scale(doc, page, 1.0)
    }

    fn job_for_at_scale(doc: figcraft_doc::Document, page: NodeId, scale: f64) -> Job {
        let side = (140.0 * scale).ceil() as u32;
        Job {
            doc: std::sync::Arc::new(doc),
            page,
            view: View { origin: (0.0, 0.0), scale, width: side, height: side },
            outline: false,
            outline_extras: figcraft_render::Outline::default(),
            outlined: Vec::new(),
            skip: Vec::new(),
            solo: None,
            proof: false,
            dots: None,
            grab: None,
            bands: Vec::new(),
            uid: 1,
            zoom_preview: false,
            at: std::time::Instant::now(),
        }
    }

    fn red_square() -> (figcraft_doc::Document, NodeId, NodeId) {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let r = doc.create(page, figcraft_doc::NodeKind::Rectangle, "R", 20.0, 20.0, 100.0, 100.0).expect("a rectangle");
        doc.get_mut(r).expect("the rectangle").fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex("FF2020").expect("hex"))];
        (doc, page, r)
    }

    /// Pixels of the refined image, straight BGRA.
    fn pixels(d: &Done) -> Vec<u8> {
        d.image.as_bytes(0).map(|b| b.to_vec()).unwrap_or_default()
    }

    #[test]
    fn a_bound_material_changes_the_refined_image() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (mut doc, page, r) = red_square();
        let plain = pixels(&super::Renders::default().refine(&job_for(doc.clone(), page)).expect("a render"));
        assert!(!plain.is_empty(), "the CPU render produced pixels");

        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(r).expect("the rectangle").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let bound = pixels(&super::Renders::default().refine(&job_for(doc, page)).expect("a render"));

        assert_eq!(bound.len(), plain.len(), "same canvas size");
        assert_ne!(bound, plain, "the bound .wgsl material must reach the refined image the canvas shows");
    }

    /// The pane must refract what is **behind** it, not its own fill. Before the layer was kept
    /// out of the base image, a red rectangle's middle came back red (its own colour) and only the
    /// rim went pale — the pane looked like a tinted blob instead of glass.
    #[test]
    fn the_pane_does_not_refract_its_own_fill() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (mut doc, page, r) = red_square();
        let plain_done = super::Renders::default().refine(&job_for(doc.clone(), page)).expect("a render");
        // The view snaps outward to whole device pixels, so read the stride from the image.
        let w = usize::from(plain_done.image.size(0).width);
        let plain = pixels(&plain_done);
        let at = |buf: &[u8], x: usize, y: usize| -> [u8; 4] {
            let i = (y * w + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let centre = at(&plain, 70, 70);
        assert!(centre[2] > 200, "the plain image really has the red fill at the centre: {centre:?}");

        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(r).expect("the rectangle").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let bound_done = super::Renders::default().refine(&job_for(doc, page)).expect("a render");
        let bound = pixels(&bound_done);
        let glass = at(&bound, 70, 70);
        // BGRA: the red fill has a low blue channel (0x20), the backdrop is neutral. Through the
        // pane the centre must read as backdrop, not as the layer's own red.
        assert!(
            glass[0] > centre[0].saturating_add(100),
            "the pane's centre is the backdrop, not the layer's own red fill: {glass:?} (plain {centre:?})"
        );
    }

    /// Zooming the canvas must not change the glass. The Figma panel `Depth` is a document-unit
    /// value; the shader turns it into device px via `dpr` (= view.scale), so the refraction band
    /// keeps the same width *relative to the node* at every zoom. This test renders the SAME bound
    /// document at 1x and 2x through the real material channel (`Renders::refine`), maps the 2x
    /// image back onto the 1x grid by document coordinates, and requires the two to coincide.
    /// A background with a hard colour edge (yellow stripe) sits behind the pane so a drift in the
    /// bend band would visibly move the refracted fringe — a plain backdrop could not catch it.
    #[test]
    fn the_glass_look_is_invariant_under_zoom() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let solid = |hex| vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex(hex).expect("hex"))];
        let bg = doc.create(page, figcraft_doc::NodeKind::Rectangle, "BG", 0.0, 0.0, 140.0, 140.0).expect("bg");
        doc.get_mut(bg).expect("bg").fills = solid("3040C0"); // blue canvas
        let stripe = doc.create(page, figcraft_doc::NodeKind::Rectangle, "S", 60.0, 20.0, 20.0, 100.0).expect("stripe");
        doc.get_mut(stripe).expect("stripe").fills = solid("F0D020"); // yellow edge behind the pane
        let pane = doc.create(page, figcraft_doc::NodeKind::Rectangle, "P", 20.0, 20.0, 100.0, 100.0).expect("pane");
        doc.get_mut(pane).expect("pane").fills = solid("FF2020"); // red glass on top
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(pane).expect("pane").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());

        let render = |scale: f64| -> (usize, Vec<u8>) {
            let d = super::Renders::default().refine(&job_for_at_scale(doc.clone(), page, scale)).expect("a render");
            let w = usize::from(d.image.size(0).width);
            (w, pixels(&d))
        };
        let (w1, a) = render(1.0);
        let (w2, b) = render(2.0);
        // The view snaps outward to whole device pixels, so 2x is approximately — not exactly —
        // double the 1x stride (e.g. 141 -> 281). Use the real ratio for the coordinate mapping.
        assert!((w2 as f64 / w1 as f64 - 2.0).abs() < 0.05, "2x view ~doubles the stride: {w1} -> {w2}");

        // Compare the two images at matching document points via each image's own stride, so the
        // outward pixel snapping cannot bias the mapping. Sample the centre of each 2x block to
        // keep AA at block borders from skewing the comparison.
        let at = |buf: &[u8], x: usize, y: usize, w: usize| -> [u8; 4] {
            let i = (y * w + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let mut differing = 0usize;
        let mut total = 0usize;
        for y in 0..w1 {
            for x in 0..w1 {
                let p = at(&a, x, y, w1);
                let qx = ((x as f64 + 0.5) * w2 as f64 / w1 as f64).floor() as usize;
                let qy = ((y as f64 + 0.5) * w2 as f64 / w1 as f64).floor() as usize;
                let q = at(&b, qx.min(w2 - 1), qy.min(w2 - 1), w2);
                let d = p.iter().zip(q.iter()).map(|(u, v)| u.abs_diff(*v)).max().unwrap_or(0);
                if d > 40 {
                    differing += 1;
                }
                total += 1;
            }
        }
        let frac = differing as f64 / total as f64;
        // Only resampling/AA along edges should differ. If the bend band drifted with zoom the
        // refracted fringe would move and this fraction would be several times larger.
        assert!(frac < 0.03, "the glass look must not drift with zoom: {:.2}% of pixels differ by >40 ({} / {})", frac * 100.0, differing, total);
    }

    /// Unbinding takes the material off again — no stale pixels stay on the canvas.
    #[test]
    fn unbinding_restores_the_plain_image() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (mut doc, page, r) = red_square();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(r).expect("the rectangle").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let bound = pixels(&super::Renders::default().refine(&job_for(doc.clone(), page)).expect("a render"));

        doc.get_mut(r).expect("the rectangle").plugin_data.remove(crate::wgsl_material::BINDING_KEY);
        let plain = pixels(&super::Renders::default().refine(&job_for(doc, page)).expect("a render"));
        assert_ne!(bound, plain, "binding changed the image");
    }
    /// 预览必须带材质。`run` 把材质节点的**整棵子树**让开（shader 要干净的 backdrop），
    /// 所以以前那张粗预览里既没有玻璃、也没有容器里的东西 —— 缩放时画布切到它、再切回来，
    /// 就是用户看到的「效果闪、内容物闪」。这里同一份作业各走一遍「不带材质」与「预览」，
    /// 要求预览里那片红色子节点还在。
    #[test]
    fn a_preview_carries_the_material_and_its_children() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let solid = |hex| vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex(hex).expect("hex"))];
        let bg = doc.create(page, figcraft_doc::NodeKind::Rectangle, "BG", 0.0, 0.0, 140.0, 140.0).expect("bg");
        doc.get_mut(bg).expect("bg").fills = solid("3040C0");
        let frame = doc.create(page, figcraft_doc::NodeKind::Frame, "Glass", 20.0, 20.0, 100.0, 100.0).expect("frame");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(frame).expect("frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        let chip = doc.create(frame, figcraft_doc::NodeKind::Rectangle, "Chip", 10.0, 10.0, 80.0, 80.0).expect("chip");
        doc.get_mut(chip).expect("chip").fills = solid("E01818");

        let mut job = job_for_at_scale(doc.clone(), page, 2.0);
        job.zoom_preview = true;
        // 阈值 0：这里不测「值不值得出预览」，只测预览的内容。
        let mut renders = super::Renders::with_threshold(0.0);
        let _ = renders.refine(&job_for_at_scale(doc.clone(), page, 1.0));
        let preview = renders.preview(&job).expect("a preview");
        let raw = preview_with(&job, &mut Retained::default()).expect("a plain preview");
        let a = preview.image.as_bytes(0).map(|b| b.to_vec()).expect("pixels");
        let b = raw.image.as_bytes(0).map(|b| b.to_vec()).expect("pixels");
        assert_eq!(a.len(), b.len(), "same coarse size");
        assert_ne!(a, b, "the preview must not be the material-less picture");
        let w = usize::from(preview.image.size(0).width);
        let h = usize::from(preview.image.size(0).height);
        let at = |buf: &[u8], x: usize, y: usize| -> [u8; 4] {
            let i = (y * w + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let (cx, cy) = (w / 2, h / 2);
        let p = at(&a, cx, cy);
        let q = at(&b, cx, cy);
        // BGRA：红子节点 = 蓝色通道高、绿色通道低。
        assert!(p[2] > 150 && p[1] < 90, "the child is in the preview: {p:?}");
        assert!(!(q[2] > 150 && q[1] < 90), "and it was missing without the material pass: {q:?}");
    }

    /// 子层只渲染它自己那一块。以前每个材质节点都把整个视口再光栅化一遍（外加一次全图 alpha
    /// 扫描）—— 平移时这笔开销就是「拖过去才开始加载」的主因。
    #[test]
    fn a_children_layer_renders_only_the_box_it_needs() {
        let _gpu = crate::wgsl_runner::GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let frame = doc.create(page, figcraft_doc::NodeKind::Frame, "Glass", 0.0, 0.0, 100.0, 100.0).expect("frame");
        let binding = crate::wgsl_material::WgslMaterialBinding::new("missing.wgsl".to_string());
        doc.get_mut(frame).expect("frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
        doc.create(frame, figcraft_doc::NodeKind::Rectangle, "Chip", 20.0, 20.0, 40.0, 40.0).expect("chip");

        let view = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 400 };
        let (px, rect) = children_layer(&doc, page, &view, frame, 400, 400).expect("a layer");
        assert!(rect[2] < 400 && rect[3] < 400, "only the children's box, not the viewport: {rect:?}");
        assert!(rect[0] <= 20 && rect[1] <= 20, "the box starts at the child: {rect:?}");
        assert!(rect[0] + rect[2] >= 60 && rect[1] + rect[3] >= 60, "and reaches its end: {rect:?}");
        assert_eq!(px.len(), (rect[2] as usize) * (rect[3] as usize) * 4, "row stride matches the rect");
    }

    /// 整页兜底图：必须覆盖整页内容，且不超过像素预算 —— 它每换一版都要重画一次。
    #[test]
    fn the_overview_covers_the_page_within_its_budget() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        doc.create(page, figcraft_doc::NodeKind::Frame, "F", 100.0, 200.0, 2500.0, 1760.0).expect("frame");
        let v = overview_view(&doc, page).expect("an overview");
        let b = doc.page_content_bounds(page).expect("bounds");
        assert!(v.origin.0 <= b.x0 && v.origin.1 <= b.y0, "covers the top-left: {v:?}");
        assert!(
            v.origin.0 + f64::from(v.width) / v.scale >= b.x1 && v.origin.1 + f64::from(v.height) / v.scale >= b.y1,
            "covers the bottom-right: {v:?}"
        );
        assert!(u64::from(v.width) * u64::from(v.height) <= OVERVIEW_PIXELS as u64 * 2, "within the pixel budget: {v:?}");
    }
}
