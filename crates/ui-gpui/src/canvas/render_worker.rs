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
    let opts = figcraft_render::Options {
        outline_mode: job.outline,
        outline: job.outline_extras,
        outlined: job.outlined.clone(),
        skip: job.skip.clone(),
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

/// 渲染线程在任务之间保留的东西。
pub struct Renders {
    full: Retained,
    preview: Retained,
    /// GPU 后端（§18）：`None` = 没开、没适配器，或这一帧压不出场景。按需创建、跨帧复用。
    #[cfg(feature = "gpu")]
    gpu: Option<crate::gpu::Gpu>,
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
        preview_with(job, &mut self.preview)
    }

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
                return Some(d);
            }
        }
        // 上一张图借给 `run`：像素没变时它直接复用这张，不再分配整整一页。
        let prev = self.last_image.clone();
        let mut done = guarded(&mut self.full, |r| run(job, &job.view, r, false, prev.as_ref()));
        if let Some(d) = done.as_mut() {
            self.last = Some((job.uid, job.page, job.view.scale));
            if d.work == Work::Full {
                self.full_ms = d.ms;
            }
            // 像素没变的那一帧（`Retained` 增量缓存命中）直接复用上一张图：既不分配新 `Vec`，
            // 也不让 GPUI 每帧新建一张 GPU 纹理（那样会逐帧累积内存直到分配失败）。
            if d.work == Work::Reused
                && let Some(prev) = &self.last_image
            {
                d.image = prev.clone();
            }
            self.last_image = Some(d.image.clone());
        }
        done
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

/// 后台渲染工作线程（对应原文的桌面 `Worker`）。
pub struct Worker {
    tx: std::sync::mpsc::Sender<(u64, Job)>,
    rx: std::sync::mpsc::Receiver<Done>,
    submitted: u64,
    finished: Arc<std::sync::atomic::AtomicU64>,
    /// 看门狗正在画补做的预览，或者手上已经有一张要交给 UI 的预览。
    /// 它也算「在飞」：否则补做的预览画好了却没人续帧，要等下一次输入才上台。
    previewing: Arc<std::sync::atomic::AtomicBool>,
    /// 替身请求（拉伸/旋转时每帧一张）：单独一条线程，免得排到整页渲染后面。

    /// 图完成后叫醒 UI。
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Worker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, jobs) = std::sync::mpsc::channel::<(u64, Job)>();
        // 有界：最多 2 张完成图在飞。无界通道在 UI 卡顿时会堆几十张整页图（每张 17.6MB）。
        let (done_tx, rx) = std::sync::mpsc::sync_channel::<Done>(2);
        let finished = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let done_with = finished.clone();
        let (watch, watched) = std::sync::mpsc::channel::<(u64, Job, std::time::Instant)>();
        let refined = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let refined_seen = refined.clone();
        let previewing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let busy = previewing.clone();

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
                        && let Some(d) = preview_with(&job, &mut retained)
                        && still_rendering()
                    {
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
        let spawned = std::thread::Builder::new().name("figcraft-render".into()).stack_size(figcraft_render::RENDER_STACK).spawn(move || {
            let mut renders = Renders::default();
            let mut next = None;
            loop {
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
        });
        if let Err(e) = spawned {
            log::error!("render thread: {e}");
        }

        Self { tx, rx, submitted: 0, finished, previewing, wake }
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
