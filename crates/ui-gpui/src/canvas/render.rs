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

/// 提交一次整页渲染请求（内容没变、缓存又盖得住时直接短路）。
///
/// **拖动期间照常提交**，不冻结底图。原工程就是这么做的：每完成一张就上台，`poll` 取最新
/// 的那张（`latest wins`）。代价是内容可能比选框晚一两帧，换来的是**没有「必须擦掉的历史
/// 像素」**——残影没有产生的余地。曾经的「冻结底图 + 当帧同步补丁」是为了消灭那一两帧滞后，
/// 但它把正确性押在「每帧算准覆盖范围」上：容器 Hug 变宽又缩回、`moving` 集合中途变化、
/// `start` 被重置，每一步算错都会留下半块残影。
pub(super) fn request_render(
    app: &mut CanvasApp,
    worker: &mut Worker,
    ppp: f32,
    max_texture_side: u32,
    skip: &[NodeId],
    grab_ask: Option<GrabAsk>,
    bands: &[figcraft_render::Band],
) {
    let display_doc = app.display_doc.clone();
    // 拖动期间底图完全冻结：被拖层由画布每帧当帧同步渲染的局部完整场景补丁提供
    // （见 canvas/mod.rs 的 patch_for）。提交异步整页图会让内容比选框晚一两帧，
    // 且旧位置会留下残影；这里一张都不提交，松手后再由 submitted=None 强制补一张。
    if !skip.is_empty() {
        return;
    }
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
    // 本帧显示文档由 CanvasApp 显式给出（Motion 预览时是求值后的文档）。
    let doc = display_doc.unwrap_or_else(|| ds.doc.clone());
    let outlined = if ds.view.outlines_selection { ds.selection.clone() } else { vec![] };
    let proof = ds.view.proof_srgb && !doc.color_profile.is_srgb();
    let outline_extras = figcraft_render::Outline { hidden: ds.view.outline_hidden, bounds: ds.view.outline_bounds, light: false };
    // 渲染范围超出可见视口：平移时旧贴图仍盖住屏幕，worker 同时补新露出的内容。
    let margin = overscan_margin(visible_w, visible_h, max_texture_side);
    let origin = (ox, oy);
    let visible = (visible_w, visible_h);
    let requested = (ox - f64::from(margin) / scale, oy - f64::from(margin) / scale);
    let size = (visible_w.saturating_add(margin * 2).min(max_texture_side), visible_h.saturating_add(margin * 2).min(max_texture_side));
    // 拖动时底图**跳过被拖的层**，由替身（`grab`）单独提供它的内容：
    // 两者合起来才是完整画面，不会重复画同一层，也不需要"擦掉旧位置"。
    let key = JobKey {
        doc: Arc::as_ptr(&doc) as usize,
        page: ds.page,
        origin: ((requested.0 * 1000.0) as i64, (requested.1 * 1000.0) as i64),
        scale: (scale * 10000.0) as i64,
        size,
        outline: ds.view.outlines,
        outline_extras,
        outlined: outlined.clone(),
        skip: skip.to_vec(),
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
        skip: skip.to_vec(),
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
    use super::{cached_render_covers, overscan_margin};

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

