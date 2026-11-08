//! 何时重渲染、留多少预渲染边、哪张完成图可以上台（照搬 `ui-egui/src/canvas/render.rs`）。
//!
//! 原文逻辑一行未改，只把 `app` 类型换成 GPUI 侧的 [`CanvasApp`]，`web_time::Instant`
//! 换成 `std::time::Instant`，并略去 board 点阵（`dots`：GPUI 侧暂未接 board）。

#![allow(dead_code)]

use std::sync::Arc;

use figcraft_doc::NodeId;
use figcraft_render::View;

use super::render_worker::{Done, GrabAsk, Job, Worker};
use super::{CanvasState, MAX_CANVAS_PIXELS};
use crate::app::CanvasApp;

#[derive(Clone, PartialEq)]
pub(crate) struct JobKey {
    doc: usize,
    page: NodeId,
    origin: (i64, i64),
    scale: i64,
    size: (u32, u32),
    outline: bool,
    outline_extras: figcraft_render::Outline,
    outlined: Vec<NodeId>,
    skip: Vec<NodeId>,
    solo: Option<NodeId>,
    /// 要顺手单独渲染的图层替身（页面矩形 + 抓取时的原点 + 图层，都 ×1000 取整 / 原值）。
    grab: Option<([i64; 6], Vec<i64>)>,
    /// 字体到位后文字要重新排版（系统字体后台加载）。
    fonts: u64,
    proof: bool,
    /// 提示带（padding / spacing）的指纹：换一条就要重画。
    bands: u64,
}

/// 替身请求的指纹：请求的页面矩形 + 抓取原点 + 图层。它变了才值得重抓。
fn grab_key(g: &GrabAsk) -> ([i64; 6], Vec<i64>) {
    (
        [
            (g.page[0] * 1000.0) as i64,
            (g.page[1] * 1000.0) as i64,
            (g.page[2] * 1000.0) as i64,
            (g.page[3] * 1000.0) as i64,
            (g.at[0] * 1000.0) as i64,
            (g.at[1] * 1000.0) as i64,
        ],
        g.ids.iter().map(|id| id.0 as i64).collect(),
    )
}

/// 一张完成的图能否上台：必须是当前显示的文件与页面，且不是盖在同尺度精修图上的粗预览。
pub(crate) fn shows(c: &CanvasState, done: &Done, shown: Option<(u64, NodeId)>) -> bool {
    if shown != Some((done.uid, done.page)) {
        return false;
    }
    // 更旧的请求永远不覆盖台上那张：看门狗会插队发预览，晚到的旧渲染会把内容拉回去
    // （拖动时表现为「内容回到上一个位置」，和选框错开）。
    if done.seq < c.tex_seq {
        return false;
    }
    let refined = !c.tex_coarse && c.tex_of == shown;
    if done.coarse && refined && done.seq <= c.tex_seq {
        return false;
    }
    let refined_here = refined
        && c.tex_view
            .as_ref()
            .zip(c.want_view.as_ref())
            .is_some_and(|(have, want)| have.scale.to_bits() == want.scale.to_bits() && covers(have, want));
    !(done.coarse && refined_here)
}

/// `outer` 是否完整显示了 `inner` 所显示的页面区域。
fn covers(outer: &figcraft_render::View, inner: &figcraft_render::View) -> bool {
    let (o, i) = (outer.page_rect(), inner.page_rect());
    let eps = 1e-6 / inner.scale.max(1e-9);
    o.x0 <= i.x0 + eps && o.y0 <= i.y0 + eps && o.x1 >= i.x1 - eps && o.y1 >= i.y1 - eps
}

/// 提示带的指纹（进 `JobKey`）：同一组带子不重画，换一条就重画。
fn bands_key(bands: &[figcraft_render::Band]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for b in bands {
        b.owner.hash(&mut h);
        for v in b.rect {
            v.to_bits().hash(&mut h);
        }
        b.fill.hash(&mut h);
        if let Some(s) = &b.stripe {
            s.m.to_bits().hash(&mut h);
            s.color.hash(&mut h);
            s.period.to_bits().hash(&mut h);
            s.width.to_bits().hash(&mut h);
            for v in s.origin {
                v.to_bits().hash(&mut h);
            }
        }
    }
    // 空组和「没有带子」也要区分开不了？可以：空组的指纹是 0，没有带子时也走这里。
    h.finish()
}

/// 这次渲染要不要顺手抓一张替身。
///
/// 只有**平移的第一步**要：那一张替身和同一次渲染的贴图是同一份文档，之后只按位移摆放，
/// 所以替身和选框永远同一帧。拉伸/旋转由 `canvas::show` 当帧自己抓（图层像素每帧都在变），
/// 这里再带一张只会在渲染回来时把新替身换成旧的——内容就又跟外框差半步了。
fn wants_grab(redraw: bool, skipping: bool) -> bool {
    !(redraw || skipping)
}

/// 拖动期间要不要**完全不渲**整页贴图。
///
/// 拖动时被拖的图层（连同被拉伸重排的父级/兄弟）由画布当帧自己画（`Live`），底图保持
/// 拖动前那一张、把它的旧位置用背景盖住即可。异步渲染永远比选框晚一到两帧，是
/// 「内容/背景和框脱开」的来源；而每帧同步渲一张又会反复分配整页缓冲（内存）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Hold,
    Async,
}

fn drag_plan(skip: &[NodeId]) -> Plan {
    if skip.is_empty() { Plan::Async } else { Plan::Hold }
}

fn drag_holds(skip: &[NodeId]) -> bool {
    drag_plan(skip) == Plan::Hold
}

pub(super) fn request_render(
    app: &mut CanvasApp,
    worker: &mut Worker,
    ppp: f32,
    max_texture_side: u32,
    moving: &[NodeId],
    moving_bounds: Option<figcraft_geom::Rect>,
    redraw: bool,
    bands: &[figcraft_render::Band],
) {
    let Ok(ds) = app.session.ds() else { return };
    let rect = app.canvas.rect;
    let (mut ox, mut oy) = ds.view.origin();
    let (scale, visible_w, visible_h) = if let Some(k) = ds.view.pixel_preview_scale() {
        // 像素预览：从整数像素原点按每页面单位 k 像素渲染，放大显示且不平滑。
        (ox, oy) = ((ox * k).floor() / k, (oy * k).floor() / k);
        let z = ds.view.zoom.max(1e-9) / k;
        (k, (f64::from(rect.width()) / z).ceil() as u32 + 2, (f64::from(rect.height()) / z).ceil() as u32 + 2)
    } else {
        (ds.view.zoom * f64::from(ppp), (rect.width() * ppp).ceil().max(1.0) as u32, (rect.height() * ppp).ceil().max(1.0) as u32)
    };
    // 像素预算：超了就**降渲染分辨率**（显示时拉伸），宁可略糊也不让峰值上 GB。
    // 贴图、`Retained` 的缓冲、转换副本、GPUI 纹理，一条链路上同尺寸的整页缓冲有好几份。
    let pixels = u64::from(visible_w) * u64::from(visible_h);
    let (scale, visible_w, visible_h) = if pixels > MAX_CANVAS_PIXELS {
        let k = (MAX_CANVAS_PIXELS as f64 / pixels as f64).sqrt();
        (scale * k, (f64::from(visible_w) * k).ceil().max(1.0) as u32, (f64::from(visible_h) * k).ceil().max(1.0) as u32)
    } else {
        (scale, visible_w, visible_h)
    };
    let doc = ds.doc.clone();
    let outlined = if ds.view.outlines_selection { ds.selection.clone() } else { vec![] };
    let proof = ds.view.proof_srgb && !doc.color_profile.is_srgb();
    let outline_extras = figcraft_render::Outline { hidden: ds.view.outline_hidden, bounds: ds.view.outline_bounds, light: false };
    // 渲染范围超出可见视口：平移时旧贴图仍盖住屏幕，worker 同时补新露出的内容。
    let margin = overscan_margin(visible_w, visible_h, max_texture_side);
    let origin = (ox, oy);
    let visible = (visible_w, visible_h);
    let requested = (ox - f64::from(margin) / scale, oy - f64::from(margin) / scale);
    let size = (visible_w.saturating_add(margin * 2).min(max_texture_side), visible_h.saturating_add(margin * 2).min(max_texture_side));
    // 拖动中的图层：贴图里让开它们（画布自己画那一块），并且**这一次**渲染顺手把它单独渲染出来。
    // 顺序不能反：先在不 skip 的这一张上拿到替身，下一张才把它去掉。
    let skipping = app.canvas.live.is_some();
    let skip: Vec<NodeId> = if skipping { moving.to_vec() } else { vec![] };
    // 替身是一次单独渲染：多选也好，复制拖动也好，都抓同一张（`Options::only` 认一组）。
    //
    // **替身只能有一个写入方**。平移：这一次渲染顺手抓一张（`at` 与这张贴图同一份文档），
    // 之后只按位移摆放——所以替身和选框永远同一帧，不会跑偏。
    // 拉伸/旋转：替身由 `canvas::show` **当帧**自己抓（图层像素每帧都变，走渲染就得慢一帧）。
    // 这里要是也带一张，渲染回来时 `done.grab` 会把当帧那张换成更旧的一张——内容就又跟外框
    // 差半步了。所以 `redraw` 时不请求 grab。
    let grab_ask = if wants_grab(redraw, skipping) {
        moving_bounds.map(|b| {
            // 阴影/描边会超出几何包围盒：按屏幕像素留一圈，免得拖动时被切掉。
            let pad = 32.0 / ds.view.zoom.max(1e-6);
            GrabAsk { page: [b.x0 - pad, b.y0 - pad, b.x1 + pad, b.y1 + pad], at: [b.x0, b.y0], ids: moving.to_vec() }
        })
    } else {
        None
    };
    let key = JobKey {
        // 拖拽期间被拖的图层让开了，它的几何再怎么变都不影响贴图：指纹固定，
        // 免得每帧都白重渲一次（那样替身永远等不到一张干净的背景，拖动时就会拖影）。
        doc: if skipping { 0 } else { Arc::as_ptr(&doc) as usize },
        page: ds.page,
        origin: ((requested.0 * 1000.0) as i64, (requested.1 * 1000.0) as i64),
        scale: (scale * 10000.0) as i64,
        size,
        outline: ds.view.outlines,
        outline_extras,
        outlined: outlined.clone(),
        skip: skip.clone(),
        solo: ds.focused(),
        fonts: figcraft_text::layout_generation(),
        proof,
        grab: grab_ask.as_ref().map(grab_key),
        // 提示带换了（悬停到另一条、拖出数值）就得重画：它们铺在图层的子层之前。
        bands: bands_key(bands),
    };
    if std::env::var_os("FIGCRAFT_KEY_LOG").is_some()
        && let Some(prev) = &app.canvas.submitted
        && prev != &key
    {
        eprintln!(
            "[key-diff] doc:{}=>{} origin:{:?}=>{:?} scale:{}==>{} size:{:?}=>{:?} bands:{}==>{} fonts:{}==>{} outline:{}=>{} solo:{:?}=>{:?} skip:{:?}=>{:?} grab:{:?}=>{:?}",
            prev.doc,
            key.doc,
            prev.origin,
            key.origin,
            prev.scale,
            key.scale,
            prev.size,
            key.size,
            prev.bands,
            key.bands,
            prev.fonts,
            key.fonts,
            prev.outline,
            key.outline,
            prev.solo,
            key.solo,
            prev.skip,
            key.skip,
            prev.grab,
            key.grab
        );
    }
    // 拉伸/旋转的替身由画布**当帧**自己抓（见 `canvas::show` 里的说明），
    // 这里不必再走后台线程；贴图只要让开它们的那一张到位就够了。
    if let Some(previous) = &app.canvas.submitted {
        let previous_origin = (previous.origin.0 as f64 / 1000.0, previous.origin.1 as f64 / 1000.0);
        let same_content = JobKey { origin: previous.origin, size: previous.size, ..key.clone() } == *previous;
        let covers_now = cached_render_covers(previous_origin, previous.size, origin, visible, scale);
        if same_content && covers_now {
            return;
        }
    }
    if app.canvas.submitted.as_ref() == Some(&key) {
        return;
    }
    let job = Job {
        doc,
        page: ds.page,
        view: View { origin: requested, scale, width: size.0, height: size.1 },
        outline: ds.view.outlines,
        outline_extras,
        outlined,
        skip,
        solo: key.solo,
        proof,
        dots: None,
        grab: grab_ask,
        bands: bands.to_vec(),
        uid: ds.uid,
        zoom_preview: ds.view.pixel_preview_scale().is_none(),
        at: std::time::Instant::now(),
    };
    app.canvas.want_view = Some(job.view.clone());
    // 拖动中的图层（平移也好、拉伸/旋转也好）：整页贴图**让开**它们一次就够——被拖的那几层
    // 由画布当帧自己画，其余内容在拖动过程中不变。所以这里同步渲一张（当帧就能上屏），
    // 之后整个拖动过程一次都不再提交。
    //
    // 异步渲染永远比选框晚一到两帧，那就是「内容/背景和框脱开」的来源；同步这一张把它一次
    // 抹掉：贴图、替身、选框来自同一份文档的同一帧。
    // 拖动中：整页贴图只需要**让开**被拖的图层一次——之后底图冻住，被拖的那几层由替身
    // 当帧画。所以拖动期间最多只提交**一次**渲染（`skip_pending` 到贴图到手为止）。
    //
    // 以前是每个指针事件提一张（padding 手柄更是连替身都没有），于是：底图永远比带子晚一到
    // 两帧（果冻），而且一次拖动就是几十张整页缓冲（17.6MB/张）——反复拖就是把内存吃光。
    if drag_holds(&job.skip) {
        if app.canvas.tex_skip != job.skip && !app.canvas.skip_pending {
            app.canvas.skip_pending = true;
            app.canvas.want_view = Some(job.view.clone());
            worker.submit(job);
        }
        app.canvas.submitted = Some(key);
        return;
    }
    app.canvas.skip_pending = false;
    worker.submit(job);
    app.canvas.submitted = Some(key);
}

/// 每边留 96 像素余量：下一次渲染要在平移触到缓存边缘之前开始。
fn cached_render_covers(buffer_origin: (f64, f64), buffer_size: (u32, u32), view_origin: (f64, f64), view_size: (u32, u32), scale: f64) -> bool {
    if !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    let guard_x = 96.0_f64.min(f64::from(buffer_size.0.saturating_sub(view_size.0)) / 4.0) / scale;
    let guard_y = 96.0_f64.min(f64::from(buffer_size.1.saturating_sub(view_size.1)) / 4.0) / scale;
    view_origin.0 >= buffer_origin.0 + guard_x
        && view_origin.1 >= buffer_origin.1 + guard_y
        && view_origin.0 + f64::from(view_size.0) / scale <= buffer_origin.0 + f64::from(buffer_size.0) / scale - guard_x
        && view_origin.1 + f64::from(view_size.1) / scale <= buffer_origin.1 + f64::from(buffer_size.1) / scale - guard_y
}

/// 额外渲染面积限制在可见面积的约 1.6 倍。
fn overscan_margin(width: u32, height: u32, max_texture_side: u32) -> u32 {
    width.min(height).saturating_div(8).min(128).min(max_texture_side.saturating_sub(width.max(height)) / 2)
}

#[cfg(test)]
mod tests {
    use super::{cached_render_covers, drag_holds, overscan_margin};
    use figcraft_doc::NodeId;

    /// 拖动期间一次都不提交：底图只有一张，替身当帧画——否则异步渲染晚一两帧，
    /// 每次拖动还要新分配一整页缓冲。
    #[test]
    fn a_drag_never_submits_a_page_render() {
        let ids = vec![NodeId(2)];
        assert!(drag_holds(&ids), "拖住一个图层：不渲染");
        assert!(!drag_holds(&[]), "没有拖动：正常渲染");
    }

    #[test]
    fn pan_uses_cached_pixels_until_the_refill_boundary() {
        let margin = overscan_margin(1000, 800, 16_383);
        assert_eq!(margin, 100);
        let buffer = (-f64::from(margin), -f64::from(margin));
        let size = (1000 + 2 * margin, 800 + 2 * margin);
        assert!(cached_render_covers(buffer, size, (0.0, 0.0), (1000, 800), 1.0));
        assert!(cached_render_covers(buffer, size, (50.0, 0.0), (1000, 800), 1.0));
        assert!(!cached_render_covers(buffer, size, (80.0, 0.0), (1000, 800), 1.0));
        assert!(!cached_render_covers(buffer, size, (-80.0, 0.0), (1000, 800), 1.0));
        assert!(cached_render_covers((-24.0, -24.0), (1048, 848), (10.0, 0.0), (1000, 800), 1.0));
        assert!(!cached_render_covers((-24.0, -24.0), (1048, 848), (19.0, 0.0), (1000, 800), 1.0));
    }

    #[test]
    fn overscan_stays_within_texture_and_pixel_budget() {
        for (width, height, limit) in [(1000, 800, 16_383), (390, 844, 16_383), (2560, 1440, 16_383), (1000, 800, 1060)] {
            let margin = overscan_margin(width, height, limit);
            assert!(width + 2 * margin <= limit || width > limit);
            assert!(height + 2 * margin <= limit || height > limit);
            let rendered = u64::from(width + 2 * margin) * u64::from(height + 2 * margin);
            let visible = u64::from(width) * u64::from(height);
            assert!(rendered * 5 <= visible * 8);
        }
    }
}

#[cfg(test)]
mod grab_tests {
    use super::*;

    /// 替身只有一个写入方：平移的第一步要抓，其余情况一律不抓。
    #[test]
    fn only_the_first_translate_render_carries_a_stand_in() {
        assert!(wants_grab(false, false), "平移的第一步：这一张贴图顺手抓替身");
        assert!(!wants_grab(false, true), "已经在替身里了：贴图让开就够");
        assert!(!wants_grab(true, false), "拉伸/旋转：替身由画布当帧自己抓");
        assert!(!wants_grab(true, true), "拉伸/旋转：绝不能再让渲染覆盖当帧那张");
    }
}
