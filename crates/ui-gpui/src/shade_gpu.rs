//! 逐层外置材质（`figcraft_render::shade`）的 GPU 后端：一层底图进，一层像素出。
//!
//! 与整页合成（`passes_for` + `children_layer` + `assemble_steps`）的区别只有「输入边界」：
//! 这里的 canvas 是**某一层自己的底图**，`rect_px` 恒为整块，`origin_px` 描述「这块窗口在节点里
//! 的位置」。所以不再需要挖空主图，也不必把子树重渲一遍——子树由 `draw_layer` 的普通递归
//! 画在材质结果之上。
//!
//! 颜色通道：渲染器给的是**直通 RGBA**，而 runner 的 canvas 纹理是 `Bgra8Unorm`，进出一律换序。
#![cfg(feature = "gpu")]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::shade::{premultiplied, ShadeBoard, ShadeRequest, ShaderSpec, Shaded};
use figcraft_render::wgsl_effect as fx;

/// 一页里可见且绑了材质的图层，装成 `ShadeBoard` 需要的形状。
///
/// 与 `wgsl_runner::passes_for` 用的是同一份绑定（`plugin_data` + `wgsl_material` 的解析器），
/// 所以两条通道认的是同一批节点、同一套参数；换 shader 文件不改变节点的匹配方式。
pub fn specs(doc: &Document, page: NodeId) -> HashMap<NodeId, Arc<ShaderSpec>> {
    let mut out = HashMap::new();
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id) else { continue };
        if matches!(n.kind, NodeKind::Page { .. }) || !visible(doc, id) {
            continue;
        }
        let Some(json) = n.plugin_data.get(crate::wgsl_material::BINDING_KEY) else { continue };
        let Some(binding) = crate::wgsl_material::WgslMaterialBinding::from_json(json) else { continue };
        let effect_id = crate::wgsl_material::ensure_loaded(&binding.path);
        out.insert(id, Arc::new(ShaderSpec { effect_id, path: binding.path, params: binding.params }));
    }
    out
}

/// 把这一帧登记的每一层都算出来，装回板上。返回真正交付的层数。
///
/// 一层一次 `composite_steps`：区域就是这个层自己，没有并集、没有跨节点共享的读回缓冲。
/// 调用点在 worker 线程上（不是 UI 线程），所以这里的同步等待不会卡住输入。
pub fn shade_pending(board: &ShadeBoard, runner: &mut crate::wgsl_runner::WgslRunner, doc: &Document) -> usize {
    let requests = board.take_requests();
    if requests.is_empty() {
        return 0;
    }
    let t0 = std::time::Instant::now();
    let mut done = 0usize;
    for req in &requests {
        if crate::canvas::render_worker::render_log() {
            eprintln!(
                "[shade] req node={:?} layer={}x{} node_px={:?} origin_px={:?} bounds={:?} scale={:.3}",
                req.node, req.size.0, req.size.1, req.node_px, req.origin_px, req.bounds, req.scale
            );
        }
        let pass = pass_for(req, doc);
        let (w, h) = req.size;
        // runner 的 canvas 是 BGRA：进去换一次，出来的也换回来。
        let mut canvas = req.backdrop.as_ref().clone();
        crate::canvas::render_worker::rgba_to_bgra(&mut canvas);
        let Some(mut out) = runner.composite_steps(&canvas, w, h, &[crate::wgsl_runner::Step::Pass(&pass)], None) else {
            continue;
        };
        crate::canvas::render_worker::rgba_to_bgra(&mut out);
        let Some(pixmap) = premultiplied(&out, w, h) else { continue };
        board.put(req.node, Shaded { key: req.key, pixmap });
        done += 1;
    }
    if crate::canvas::render_worker::render_log() {
        eprintln!(
            "[shade] {} layers {:.1}ms ok={done}/{}",
            requests.len(),
            t0.elapsed().as_secs_f64() * 1000.0,
            requests.len()
        );
    }
    done
}

/// 一层 → 一次 GPU pass。几何导出与 `passes_for` 逐字段对齐：两条通道对同一节点必须打包出
/// 同一份 uniform，否则「逐层」与「整页」的像素没法比对。
fn pass_for(req: &ShadeRequest, doc: &Document) -> crate::wgsl_runner::RunPass {
    let (rw_px, rh_px) = req.size;
    let sc = req.scale.max(1e-9);
    let [bx0, by0, bx1, by1] = req.bounds;
    let (dw, dh) = ((bx1 - bx0).max(0.0), (by1 - by0).max(0.0));
    let (cx, cy) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0);
    let radii_doc = req.radii_px.map(|v| f64::from(v) / sc);
    let uniform_corners = radii_doc.iter().all(|v| (v - radii_doc[0]).abs() < 1e-6);

    let mut ex = fx::EngineExports::default();
    ex.set("resolution", vec![f64::from(rw_px), f64::from(rh_px)]);
    ex.set("nodeSize", vec![dw, dh]);
    ex.set("quad", vec![dw / 2.0, dh / 2.0, cx, cy]);
    ex.set("canvasSize", vec![f64::from(rw_px) / sc, f64::from(rh_px) / sc]);
    ex.set("nodeOrigin", vec![bx0, by0]);
    ex.set("nodeSpaceCenterPoint", vec![cx, cy]);
    ex.set("cornerRadii", radii_doc.to_vec());
    ex.set("cornerSmoothings", vec![req.smoothing; 4]);
    ex.set("hasUniformCorners", vec![if uniform_corners { 1.0 } else { 0.0 }]);
    ex.set("dpr", vec![sc]);
    ex.set("nodeSizePx", vec![f64::from(req.node_px.0), f64::from(req.node_px.1)]);
    ex.set("regionOriginPx", vec![f64::from(req.origin_px.0), f64::from(req.origin_px.1)]);
    ex.set("scale", vec![1.0]);
    ex.set("time", vec![material_time()]);

    let params: &BTreeMap<String, f64> = &req.spec.params;
    let (uniforms, field_band) = crate::wgsl_runner::pack_uniforms(&req.spec.effect_id, params, &ex, dw, dh, sc);
    let field_bytes = crate::wgsl_runner::prepare_fields(doc, req.node, rw_px, rh_px, sc, req.origin_px);
    crate::wgsl_runner::RunPass {
        effect: req.spec.effect_id.clone(),
        node: req.node,
        // 这一层就是整块 canvas：区域从 (0,0) 起，尺寸即纹理尺寸。
        rect_px: [0, 0, rw_px, rh_px],
        radii_px: req.radii_px,
        node_px: req.node_px,
        origin_px: req.origin_px,
        field_bytes,
        field_band,
        uniforms,
        dpr: sc as f32,
    }
}

/// 这些材质在**自己那层之外**还要读多少设备像素 —— 分带渲染靠它决定每条带多渲染多少行。
///
/// 材质的空间 reach 就是它的透镜带：`depth`（文档单位，已经由 [`figcraft_doc::Glass::
/// depth_reach_doc`] 换算过）加一单位的 bevel，乘回 `scale` 变设备像素。不把它算进 overlap，
/// 每条带就会在带边界处采到被截断的底图：每条带各画一遍自己的边缘高光，接缝处留下亮带。
/// 材质的阅读距离完全由参数决定，与底图无关，所以这里只需要遍历绑定，不需要底图像素。
pub fn reach_px(doc: &Document, page: NodeId, scale: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for (id, spec) in specs(doc, page) {
        let Some(n) = doc.node(id) else { continue };
        let short = n.width.min(n.height).max(1.0);
        let depth = spec
            .params
            .iter()
            .find(|(k, _)| k.as_str() == "depth" || k.ends_with(".depth"))
            .map(|(_, v)| *v)
            .unwrap_or(20.0);
        worst = worst.max(figcraft_doc::Glass::depth_reach_doc(depth, short));
    }
    if worst <= 0.0 {
        return 0.0;
    }
    (worst + 1.0) * scale.max(1e-9) + 4.0
}

fn visible(doc: &Document, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(nid) = cur {
        let Some(n) = doc.node(nid) else { return false };
        if !n.visible {
            return false;
        }
        cur = n.parent;
    }
    true
}

/// 与整页通道共用的材质时钟（同一原点，`time` 才对得上）。
fn material_time() -> f64 {
    static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    CLOCK.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}
