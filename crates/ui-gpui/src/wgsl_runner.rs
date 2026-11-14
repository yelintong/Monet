//! Universal WGSL material runner for the GPUI canvas (`--features gpu` only).
//!
//! This is the general host↔shader data-exchange contract. It does **not** know any particular
//! shader: any `.wgsl` dropped in the material directory can use it, and adding more shaders needs
//! zero host changes. The host promises a fixed set of resources; a shader opts into what it needs
//! simply by declaring a binding with the matching **variable name**, or a uniform field with the
//! matching **field name**.
//!
//! # The contract
//!
//! | binding / field name | what the host supplies |
//! |---|---|
//! | `uniform` var (any name: `U`, `u`, `Uniforms`, …) | packed per-node bytes: generic exports by field name (`nodeSize`, `quad`, `cornerRadii`, `resolution`, `dpr`, `time`, …) + the shader's `@fx param` control values |
//! | `texture_2d backdrop` / `sceneTex` / `scene` / `blurIn` / (any other texture) | the node's **backdrop**: the already-rendered canvas pixels under the node rectangle |
//! | `texture_2d fields` | a generated SDF field: `R` = normalised edge distance (0 at centre, 1 at the bend depth), `GB` = inward unit normal encoded `0.5*(n+1)`, `A` = coverage |
//! | `sampler` (any name) | a standard linear / clamp-to-edge sampler |
//! | fragment entry (any name) | one fullscreen-triangle pass drawn into the node rectangle (viewport + scissor), source-over alpha blended |
//!
//! `resolution` is injected as the node rectangle's **device-pixel** size — the natural, universal
//! meaning a shader needs for texel-sized effects (blur steps, refraction offsets, rim width).
//!
//! The CPU canvas is uploaded once as a shared scene texture, each node's backdrop is cropped from
//! it, and each material pass is drawn into an offscreen output seeded with the scene, then read
//! back to straight RGBA. One shared wgpu device/queue is cached across frames; per-node pipelines
//! are cached by (shader hash, canvas size). Any failure degrades to the untouched CPU image.

use std::collections::{BTreeMap, HashMap};

use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Affine, Point};
use figcraft_render::wgsl_effect as fx;

/// The canvas format. **BGRA**, because that is what the app hands in and takes back: the CPU
/// renderer produces BGRA and GPUI uploads BGRA. Declaring the texture BGRA means the two full-canvas
/// RGBA↔BGRA passes the old code ran every frame (two `w*h*4` byte swaps, on the render thread,
/// before and after the GPU work) simply do not exist — the sampler swizzles for us.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// A pass's children arrive as straight **RGBA** from the CPU rasteriser. Sampling an RGBA texture
/// and writing into the BGRA canvas keeps the colours in the right channels without a per-pixel swap.
const OVERLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The `fields` texture: `R` = signed distance in pixels (negative inside), `GB` = inward normal,
/// `A` = coverage. Float so a big pane keeps a pixel-accurate distance field.
const FIELDS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const COPY_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// One GPU at a time while testing. Each `WgslRunner` owns its own device, and several devices
/// compositing at once makes the pixel comparisons flaky (the app has exactly one, so this is a
/// test-harness concern, not a product one).
#[cfg(test)]
pub(crate) static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One node's universal material pass, prepared host-side (pure math, no GPU) so it is testable.
#[derive(Clone, Debug)]
pub struct RunPass {
    /// Registry id the shader is loaded under.
    pub effect: String,
    /// Selected node (cache key).
    pub node: NodeId,
    /// Node rectangle in device pixels `[x, y, w, h]` — the pass viewport/scissor.
    pub rect_px: [u32; 4],
    /// Corner radii in **device pixels** `[tl, tr, br, bl]` — fallback for analytic rectangles.
    pub radii_px: [f32; 4],
    /// The node's own size in device pixels, and where this window sits inside it: `rect_px` is
    /// the node **clipped to the canvas**, so a shader that needs the shape's centre (splay) or its
    /// distance field must be told the window is not the whole node.
    pub node_px: (f32, f32),
    pub origin_px: (f32, f32),
    /// Pre-built `fields` texture bytes (R/GB/A, `rect_px[2] × rect_px[3]`). `None` makes the
    /// compositor fall back to the analytic rounded-rect field. `Some` carries the **real outline**
    /// (text, vector path, ellipse, boolean …) so an arbitrary shape gets correct coverage,
    /// edge distance and inward normals without the shader knowing what kind of node it is.
    pub field_bytes: Option<Vec<u8>>,
    /// How far from the edge the analytic field has to be exact, in device pixels. Beyond it the
    /// field is filled with a constant (clearly inside / clearly outside), because no shader reads
    /// a distance larger than its own lens reach. Computing an exact distance for every pixel of a
    /// 1080p pane cost 42 ms per rebuild — and a zoom rebuilds it every frame.
    pub field_band: f32,
    /// Packed uniform buffers, one per `var<uniform>` declaration: `(var_name, bytes)`.
    pub uniforms: Vec<(String, Vec<u8>)>,
    /// Device pixels per document unit for the frame this pass belongs to (the view's scale).
    ///
    /// The compositor needs it to size the backdrop's frost pyramid: a material's blur radius is in
    /// **document** units, so the mip level it asks for is `log2(radius × dpr)` — a level budget
    /// fixed at "8" is only enough while `dpr ≈ 1` and silently truncates the frost at every zoom
    /// above that.
    pub dpr: f32,
}

/// Compute the universal material passes for a rendered page: enumerate bound + visible nodes,
/// derive geometry, and pack each node's uniforms from generic exports + its stored `@fx param`s.
/// No GPU, no shader-specific logic — the same for every material.
pub fn passes_for(doc: &Document, page: NodeId, view: &figcraft_render::View, time: f64) -> Vec<RunPass> {
    use figcraft_doc::NodeKind;
    let (cw, ch) = (f64::from(view.width), f64::from(view.height));
    if cw < 1.0 || ch < 1.0 {
        return Vec::new();
    }
    let sc = view.scale.max(1e-6);
    let inv = 1.0 / sc;
    let (canvas_w, canvas_h) = (cw * inv, ch * inv);
    let xf_view = view.transform();

    let mut out = Vec::new();
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id) else { continue };
        if matches!(n.kind, NodeKind::Page { .. }) || !node_visible(doc, id) {
            continue;
        }
        let Some(json) = n.plugin_data.get(crate::wgsl_material::BINDING_KEY) else { continue };
        let Some(binding) = crate::wgsl_material::WgslMaterialBinding::from_json(json) else { continue };
        let Some(r) = doc.bounds_of(&[id]) else { continue };
        let (w, h) = (r.width(), r.height());
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let radii = n.corners.radii;
        let smoothing = n.corners.smoothing;
        let uniform_corners = radii.iter().all(|v| (v - radii[0]).abs() < 1e-6);

        // Screen rect in device pixels (the same transform the CPU renderer used), kept inside
        // the canvas: a node that hangs over the viewport edge must not produce a rect the GPU
        // crop would run past the texture's end.
        let (x0, y0) = map(xf_view, r.x0, r.y0);
        let (x1, y1) = map(xf_view, r.x1, r.y1);
        let (nx0, ny0) = (x0.min(x1), y0.min(y1));
        let (nx1, ny1) = (x0.max(x1), y0.max(y1));
        let (sx0, sy0) = (nx0.max(0.0), ny0.max(0.0));
        let (sx1, sy1) = (nx1.min(cw), ny1.min(ch));
        if sx1 - sx0 <= 1.0 || sy1 - sy0 <= 1.0 {
            continue;
        }
        // The window **covers whole pixels**: `floor` its start, `ceil` its end.
        //
        // Truncating (`as u32`) made the window up to a pixel narrower than the node, while the
        // shader normalises its uv to the window. The backdrop was therefore rescaled by up to
        // 1/width — a different factor on every frame of a zoom — which is exactly the shimmer you
        // see inside a container while scaling it. Covering whole pixels makes uv and screen pixels
        // a strict 1:1 map again, so the sampled position moves continuously with the node.
        let (px0, py0) = (sx0.floor(), sy0.floor());
        let (px1, py1) = (sx1.ceil().min(cw), sy1.ceil().min(ch));
        if px1 - px0 < 1.0 || py1 - py0 < 1.0 {
            continue;
        }
        let rect_px = [px0 as u32, py0 as u32, (px1 - px0) as u32, (py1 - py0) as u32];
        let rw = f64::from(rect_px[2]);
        let rh = f64::from(rect_px[3]);
        let radii_px = radii.map(|v| (v * sc).max(0.0) as f32);
        // The window is the node **clipped to the canvas**, so the node's own geometry has to be
        // described in the node's pixel frame, not the window's: otherwise a pane zoomed past the
        // viewport gets its rounded corners and its lens band drawn on the *viewport* edge. The
        // origin stays fractional — that sub-pixel part is what keeps the distance field continuous.
        let node_px = ((nx1 - nx0) as f32, (ny1 - ny0) as f32);
        let origin_px = (rect_px[0] as f32 - nx0 as f32, rect_px[1] as f32 - ny0 as f32);

        // Generic exports: every field below is a stable, shader-agnostic name. A shader uses
        // whichever it declares in its uniform struct; the rest are simply ignored by the packer.
        let mut ex = fx::EngineExports::default();
        ex.set("resolution", vec![rw, rh]); // node region size in device pixels
        ex.set("nodeSize", vec![w, h]); // node size in document units
        ex.set("quad", vec![w / 2.0, h / 2.0, cx, cy]);
        ex.set("canvasSize", vec![canvas_w, canvas_h]);
        ex.set("nodeOrigin", vec![r.x0, r.y0]);
        ex.set("nodeSpaceCenterPoint", vec![cx, cy]);
        ex.set("cornerRadii", radii.to_vec());
        ex.set("cornerSmoothings", vec![smoothing; 4]);
        ex.set("hasUniformCorners", vec![if uniform_corners { 1.0 } else { 0.0 }]);
        ex.set("dpr", vec![sc]);
        ex.set("nodeSizePx", vec![f64::from(node_px.0), f64::from(node_px.1)]);
        ex.set("regionOriginPx", vec![f64::from(origin_px.0), f64::from(origin_px.1)]);
        ex.set("scale", vec![1.0]);
        ex.set("time", vec![time]);

        let eid = crate::wgsl_material::ensure_loaded(&binding.path);
        let (uniforms, field_band) = pack_uniforms(&eid, &binding.params, &ex, w, h, sc);

        let field_bytes = prepare_fields(doc, id, rect_px[2], rect_px[3], sc, origin_px);
        out.push(RunPass { effect: eid, node: id, rect_px, radii_px, node_px, origin_px, field_bytes, field_band, uniforms, dpr: sc as f32 });
    }
    out
}

/// Pack every `var<uniform>` a shader declares, from `ex` (the generic exports) and `params` (the
/// node's control values), and return the `fields` band the pass needs.
///
/// One node's params never leak into the global registry: the registry is cloned, so two layers on
/// the same shader keep their own values. Both material channels (the whole-page compositor and the
/// per-layer one) go through here, so a node looks the same whichever draws it.
pub(crate) fn pack_uniforms(
    eid: &str,
    params: &BTreeMap<String, f64>,
    ex: &fx::EngineExports,
    node_w: f64,
    node_h: f64,
    sc: f64,
) -> (Vec<(String, Vec<u8>)>, f32) {
    let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
    let mut local = reg.clone();
    for (field, value) in params {
        let _ = local.set_param(eid, field, *value);
    }
    // The field only has to resolve as far out as the shader can look, and the shader reads
    // `depth` **after** this node's own overrides are applied. Ask the registry for the value
    // that will actually be uploaded, not the binding: a material whose depth the panel set
    // to 98 would otherwise get a band sized for 20 and a wrong "deep inside" fill between.
    let depth_field = local.params(eid).into_iter()
        .find(|p| p.field == "depth" || p.field.ends_with(".depth"))
        .map(|p| p.field.clone());
    let reach = depth_field.as_ref()
        .and_then(|f| local.param_value(eid, f))
        .unwrap_or(20.0);
    // Depth is a panel notch (1..100), not a pixel length. Figma turns it into a node-space
    // thickness upstream (see `figma_depth_coefficient.html` §1.5/§5); the CPU glass and this
    // external material must share that one conversion or the same document looks different
    // on the two paths. `node_w`/`node_h` are document-unit node bounds, the same domain as
    // `reach`, so write the converted **document-unit** thickness back into `u.depth`: the
    // shader's unchanged `clamp(u.depth, 1, 100) * dpr` then lands on exactly the CPU's lens band.
    // Small notches that already fit pass through untouched, so the panel's low end and the
    // default keep their exact old look; only "small node + high notch", which cannot
    // physically fit, is clamped. `local` is a clone, so no other node / the global registry
    // is affected. `band` keeps using the raw notch (an upper bound), which is a safe margin
    // >= the clamped reach, leaving the region/backdrop mapping byte-identical to before.
    let reach_doc = figcraft_doc::Glass::depth_reach_doc(reach, node_w.min(node_h));
    if let Some(f) = depth_field.as_ref() {
        let _ = local.set_param(eid, f, reach_doc);
    }
    let band = (reach.max(1.0) * sc + 2.0 * sc + 8.0) as f32;
    let vars: Vec<String> = local.scan(eid)
        .map(|s| s.uniforms.keys().cloned().collect())
        .unwrap_or_default();
    let packed = vars.into_iter()
        .filter_map(|var| local.uniform_bytes_for(eid, &var, ex).ok().map(|b| (var, b)))
        .collect();
    (packed, band)
}

fn map(m: Affine, x: f64, y: f64) -> (f64, f64) {
    let p = m * Point::new(x, y);
    (p.x, p.y)
}

fn node_visible(doc: &Document, id: NodeId) -> bool {
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

/// Build the `fields` texture bytes for one node, in the exact R/GB/A encoding every material
/// shader reads (`R` = signed distance in pixels, negative inside; `GB` = inward normal;
/// `A` = coverage).
///
/// A plain rectangle keeps the analytic rounded-rect field — byte-for-byte the historic path, so
/// its glass look cannot shift. **Every other node kind** (text, vector path, ellipse, polygon,
/// star, line, boolean, icon, instance …) is rasterised from its *real outline* through the
/// same exact-Euclidean-distance + smoothed-normal pipeline the built-in CPU glass reads, so a
/// shader needs no change and no per-shape knowledge to sit correctly on arbitrary artwork.
/// `None` = "no outline" and falls back to the analytic rectangle.
pub(crate) fn prepare_fields(doc: &Document, id: NodeId, rw: u32, rh: u32, sc: f64, origin_px: (f32, f32)) -> Option<Vec<u8>> {
    use figcraft_doc::NodeKind;
    let n = doc.node(id)?;
    if matches!(n.kind, NodeKind::Rectangle) {
        return None; // analytic path, unchanged
    }
    let outline = figcraft_render::node_outline(doc, n)?;
    // Local outline -> the node's device-pixel frame, then the window's own origin. `bounds_of` is
    // the same world box the CPU renderer rasterised, so this maps the outline onto the backdrop
    // exactly even when the window is only the part of the node inside the viewport.
    let r = doc.bounds_of(&[id])?;
    let xf = Affine::translate((f64::from(-origin_px.0), f64::from(-origin_px.1)))
        * Affine::scale(sc)
        * Affine::translate((-r.x0, -r.y0))
        * doc.world_transform(id);
    let bytes = figcraft_render::glass::outline_fields_half(&outline, xf, rw, rh, 2)?;
    Some(bytes)
}

/// Generate the universal `fields` texture for a rounded rectangle: `R` = signed distance to the
/// node's edge in device pixels (negative inside), `GB` = inward unit normal encoded
/// `0.5*(n+1)`, `A` = coverage (antialiased by the signed distance).
///
/// `w` × `h` is the **window** (the node clipped to the canvas) and `node` × `origin` say where
/// that window sits inside the node's own pixel frame: `origin` is the window's top-left corner in
/// node pixels. A window that is not the whole node must not be treated as one, or a pane zoomed
/// full screen grows rounded corners and a lens band along the viewport edge.
///
/// This is a clean-room closed-form rounded-rectangle SDF and is shared by **every** shader that
/// declares a `fields` binding — no per-shader knowledge. Self-contained so it is unit-testable.
pub fn fields_for_rect(w: u32, h: u32, node: (f32, f32), origin: (f32, f32), radii_px: [f32; 4]) -> Vec<u8> {
    fields_for_rect_banded(w, h, node, origin, radii_px, f32::INFINITY)
}

/// `fields_for_rect`, but only resolving the distance **within `band` pixels of the edge**.
///
/// A material stops reading the distance once it is past its own reach — the lens is gone, the light
/// is gone, the coverage is settled — so a pixel far inside or far outside only needs to say "far".
/// The earlier version ran the exact distance for every pixel of the window: 38 ms for a 1080p pane,
/// on the render thread, and a zoom rebuilds the field on **every frame** because the sub-pixel
/// window origin moves. That single loop was the worst part of the frame.
///
/// The inner block is written as whole rows copied from two templates (one all-outside, one with the
/// settled core), so the bulk of a big window costs a handful of `memcpy`s instead of millions of
/// per-pixel stores, and only the band around the edge runs the distance function.
///
/// A `band` of infinity keeps the historic exact field (used by the tests that check the encoding).
pub fn fields_for_rect_banded(w: u32, h: u32, node: (f32, f32), origin: (f32, f32), radii_px: [f32; 4], band: f32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 8) as usize];
    if w == 0 || h == 0 || node.0 <= 0.0 || node.1 <= 0.0 {
        return out;
    }
    let half = (node.0 * 0.5, node.1 * 0.5);
    let most = half.0.min(half.1);
    let r = radii_px.map(|v| if v.is_finite() { v.clamp(0.0, most) } else { 0.0 });
    // A corner's distance is decided by the radius, not by the band, so the band has to be at least
    // as wide as the largest corner, or a rounded pane would get a wrong "deep inside" fill.
    let b = if band.is_finite() { band.max(r.iter().fold(0.0f32, |a, v| a.max(*v))) } else { f32::INFINITY };

    let stride = (w * 8) as usize;
    let cx = half.0 - origin.0 - 0.5;

    // With a band: two row templates carry the whole settled area.
    let banded = b.is_finite() && b < half.0 && b < half.1;
    let mut row_out: Vec<u8> = Vec::new();
    let mut row_core: Vec<u8> = Vec::new();
    let mut core = 0u32..0u32;
    if banded {
        row_out = vec![0u8; stride];
        write_run(&mut row_out, 0, w, b + 1.0, 0.0);
        // The core between the two side bands, filled once: it does not depend on the row.
        let (lo, hi) = (cx - half.0 + b, cx + half.0 - b);
        let (lo, hi) = (lo.max(0.0), hi.min(w as f32));
        core = if hi > lo { (lo.floor() as u32)..((hi.ceil() as u32).min(w)) } else { 0..0 };
        row_core = row_out.clone();
        write_run(&mut row_core, core.start, core.end, -b - 1.0, 1.0);
    }

    for y in 0..h {
        let qy = origin.1 + y as f32 + 0.5 - half.1;
        let row_start = (y as usize) * stride;
        // Which pixels of this row still need an exact distance.
        let (mut from, mut to) = (0u32, w);
        if banded {
            if qy.abs() > half.1 + b {
                out[row_start..row_start + stride].copy_from_slice(&row_out);
                continue; // the whole row is far outside
            }
            if qy.abs() < half.1 - b {
                out[row_start..row_start + stride].copy_from_slice(&row_core);
                from = 0;
                to = core.start; // left side band
                exact_run(&mut out, y, w, from, to, origin, half, &r);
                from = core.end;
                to = w; // right side band
                exact_run(&mut out, y, w, from, to, origin, half, &r);
                continue;
            }
            // The top/bottom band: the row's distance is decided by `qy` everywhere, so all of it
            // has to be exact. Start from the outside template (coverage 0) and overwrite.
            out[row_start..row_start + stride].copy_from_slice(&row_out);
        }
        exact_run(&mut out, y, w, from, to, origin, half, &r);
    }
    out
}

/// Write `len` pixels of one settled value into a row buffer.
fn write_run(row: &mut [u8], x0: u32, x1: u32, sd: f32, coverage: f32) {
    if x1 <= x0 {
        return;
    }
    let mut px = [0u8; 8];
    figcraft_render::glass::half_at(&mut px[0..2], sd);
    figcraft_render::glass::half_at(&mut px[2..4], 0.5);
    figcraft_render::glass::half_at(&mut px[4..6], 0.5);
    figcraft_render::glass::half_at(&mut px[6..8], coverage);
    for i in x0..x1 {
        let at = (i as usize) * 8;
        row[at..at + 8].copy_from_slice(&px);
    }
}

/// Exact distance + inward normal + coverage for `x0..x1` of row `y`.
fn exact_run(out: &mut [u8], y: u32, w: u32, x0: u32, x1: u32, origin: (f32, f32), half: (f32, f32), r: &[f32; 4]) {
    if x1 <= x0 {
        return;
    }
    for x in x0..x1 {
        let qx = origin.0 + x as f32 + 0.5 - half.0;
        let qy = origin.1 + y as f32 + 0.5 - half.1;
        let corner = if qx < 0.0 && qy < 0.0 {
            r[0]
        } else if qx >= 0.0 && qy < 0.0 {
            r[1]
        } else if qx >= 0.0 && qy >= 0.0 {
            r[2]
        } else {
            r[3]
        };
        let (sd, nx, ny) = rounded_rect_sdf(qx, qy, half.0, half.1, corner);
        let coverage = (0.5 - sd).clamp(0.0, 1.0);
        let i = ((y as usize * w as usize + x as usize) * 8) as usize;
        figcraft_render::glass::half_at(&mut out[i..i + 2], sd);
        figcraft_render::glass::half_at(&mut out[i + 2..i + 4], nx * 0.5 + 0.5);
        figcraft_render::glass::half_at(&mut out[i + 4..i + 6], ny * 0.5 + 0.5);
        figcraft_render::glass::half_at(&mut out[i + 6..i + 8], coverage);
    }
}

fn rounded_rect_sdf(qx: f32, qy: f32, hx: f32, hy: f32, r: f32) -> (f32, f32, f32) {
    let sx = if qx < 0.0 { -1.0 } else { 1.0 };
    let sy = if qy < 0.0 { -1.0 } else { 1.0 };
    let kx = qx.abs() - hx + r;
    let ky = qy.abs() - hy + r;
    let (sd, gx, gy) = if kx > 0.0 && ky > 0.0 {
        let len = (kx * kx + ky * ky).sqrt().max(1e-6);
        (len - r, sx * kx / len, sy * ky / len)
    } else if kx > ky {
        (kx - r, sx, 0.0)
    } else {
        (ky - r, 0.0, sy)
    };
    // Outward gradient is (gx, gy); inward normal is its negation.
    (sd, -gx, -gy)
}

/// Snapshot of a shader's declared resources, read from the host-agnostic scan.
#[derive(Clone, Debug)]
struct ShaderInfo {
    hash: u64,
    code: String,
    vertex: String,
    fragments: Vec<String>,
    /// `(group, binding, name, ty, is_uniform)` for every declared resource.
    bindings: Vec<(u32, u32, String, String, bool)>,
}

fn shader_info(id: &str) -> Option<ShaderInfo> {
    use std::hash::{Hash, Hasher};
    let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
    let scan = reg.scan(id)?;
    let code = reg.source_code(id)?.to_string();
    let vertex = scan.entries.iter().find(|e| e.stage == "vertex").map(|e| e.name.clone()).or_else(|| scan.generic_entries.first().cloned())?;
    let fragments: Vec<String> = scan.entries.iter().filter(|e| e.stage == "fragment").map(|e| e.name.clone()).collect();
    if fragments.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    let bindings = scan.bindings.iter().map(|b| (b.group, b.binding, b.name.clone(), b.ty.clone(), b.uniform)).collect();
    Some(ShaderInfo { hash: hasher.finish(), code, vertex, fragments, bindings })
}

/// A compiled per-(shader,canvas-size) pipeline and its reusable GPU resources.
struct Compiled {
    hash: u64,
    canvas: (u32, u32),
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    layouts: Vec<wgpu::BindGroupLayout>,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    /// Uniform buffers by variable name.
    ubufs: Vec<(String, wgpu::Buffer)>,
    /// Which binding slot each uniform variable uses (group, binding).
    ubind: Vec<(String, u32, u32)>,
    /// Texture/sampler bindings: `(group, binding, kind)` where kind is "backdrop" | "fields" | "sampler".
    resource_binds: Vec<(u32, u32, &'static str)>,
}

fn is_fields_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("fields") || name.eq_ignore_ascii_case("sdf")
}
fn is_sampler_ty(ty: &str) -> bool {
    ty.trim() == "sampler" || ty.contains("sampler")
}

fn compile(device: &wgpu::Device, info: &ShaderInfo, canvas: (u32, u32), format: wgpu::TextureFormat) -> Result<Compiled, String> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("universal wgsl material"),
        source: wgpu::ShaderSource::Wgsl(info.code.clone().into()),
    });
    let max_group = info.bindings.iter().map(|b| b.0).max().unwrap_or(0);
    let mut layouts = Vec::new();
    let mut ubufs = Vec::new();
    let mut ubind = Vec::new();
    let mut resource_binds = Vec::new();

    // One uniform buffer per declared uniform variable, sized to at least what the packer writes.
    for (g, b, name, _ty, is_uniform) in &info.bindings {
        if *is_uniform {
            let size = 256u64; // generous: covers every generic export + control set we pack.
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            ubufs.push((name.clone(), buf));
            ubind.push((name.clone(), *g, *b));
        } else if is_sampler_ty(_ty) {
            resource_binds.push((*g, *b, "sampler"));
        } else if _ty.contains("texture_2d") {
            let kind = if is_fields_name(name) { "fields" } else { "backdrop" };
            resource_binds.push((*g, *b, kind));
        }
    }

    for g in 0..=max_group {
        let entries: Vec<wgpu::BindGroupLayoutEntry> = info
            .bindings
            .iter()
            .filter(|b| b.0 == g)
            .filter_map(|(_, b, _, ty, is_uniform)| {
                let t = if *is_uniform {
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
                } else if is_sampler_ty(ty) {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                } else if ty.contains("texture_2d") {
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    }
                } else {
                    return None;
                };
                Some(wgpu::BindGroupLayoutEntry { binding: *b, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: t, count: None })
            })
            .collect();
        layouts
            .push(device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("universal material group"), entries: &entries }));
    }
    let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("universal material"),
        bind_group_layouts: &layout_refs,
        immediate_size: 0,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        // Trilinear: a shader that samples the backdrop at a mip level gets the reduction *and* a
        // linear upsample, which is Figma's frost chain (downsample → upsample) in one call.
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });
    // Contract: run the first fragment entry as a full-coverage pass. Shaders needing several passes
    // express that with additional fragment entries each bound to their own inputs; the universal
    // host drives them by name ordering (see `draw_node`).
    let fragment = &info.fragments[0];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some(&info.vertex), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    Ok(Compiled { hash: info.hash, canvas, module, layout, layouts, sampler, pipeline, ubufs, ubind, resource_binds })
}

struct Out {
    w: u32,
    h: u32,
    padded: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// One in-flight asynchronous readback. A `COPY_DST | MAP_READ` buffer can be written by the GPU
/// *or* read by the CPU, never both at once (see the `Buffer` docs): once `map_async` is pending we
/// must hand the next frame a **different** buffer. A small ring of these is enough to turn the old
/// "submit, then block until the pixels are back" into "submit and return; collect a previous frame
/// later". Size is the region's padded row stride × height, so a small card never allocates a
/// full-canvas buffer.
struct ReadbackSlot {
    buffer: wgpu::Buffer,
    /// Canvas size this frame was encoded against — needed to splice the region back into `base`.
    w: u32,
    h: u32,
    /// The region's padded row stride and canvas-space box, recorded at submit so `poll_ready` can
    /// splice without re-deriving them from steps that have since been dropped.
    padded: u32,
    region: [u32; 4],
    /// The frame this slot currently belongs to, or `None` when free.
    frame_id: Option<u64>,
    /// Whether the slot's `buffer` is awaiting a map callback. Overwritten lazily: a completed
    /// `map_async` leaves the buffer *mapped*, and reusing it requires `unmap()` first.
    map_rx: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    /// True after `submit_async` fired the map; false once `poll_ready` has collected and unmapped.
    in_use: bool,
}

/// How many frames may be in flight before the oldest pending frame is reclaimed. Two is the
/// minimum for pipelining (frame N+1 encodes while frame N is still mapping); three leaves a spare
/// for a late frame without growing the ring unboundedly. Each slot holds only its region's bytes.
const ASYNC_SLOTS: usize = 3;

/// The node's backdrop: **its window plus a full mip chain**.
///
/// Level 0 is copied from *the composite so far* (not from the raw CPU scene) — that is what makes
/// a material respect the layers underneath it, including another material's finished result. Every
/// other level is a 2×2 box reduction **on the GPU**. Sampling level `L` with a linear filter is
/// exactly Figma's frost chain (downsample → bilinear upsample); generating it on the GPU is also
/// what removes the largest per-frame CPU cost of the earlier implementation, which rebuilt the
/// whole chain in Rust for every pass, every frame.
///
/// The texture is the window's size, so the shader's `0..1` uv maps onto exactly the window it was
/// given, and it is kept per node: rebuilding it (with its pyramid) on every zoom frame was the
/// other half of that cost. `levels` holds a single-level view of each level so it can be used as a
/// render target.
struct Backdrop {
    w: u32,
    h: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: Vec<(u32, u32, wgpu::TextureView)>,
}

/// One node's `fields` texture, kept across frames. Its `key` covers everything the bytes were
/// derived from (device size, sub-pixel window origin, radii, scale), so a pane that did not move or
/// resize skips **both** the SDF rebuild and the upload. Rebuilding a 4 MP signed-distance field
/// every frame was several milliseconds of pure CPU that no layer change could explain.
struct FieldTex {
    key: u64,
    w: u32,
    h: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// How many mip levels a `w` × `h` texture has: level `L` is the backdrop reduced by `2^L`.
fn mip_levels(w: u32, h: u32) -> u32 {
    (w.max(h).max(1) as f32).log2().floor() as u32 + 1
}

/// The largest frost radius any shader can ask for, in **document** units — `glass.wgsl` clamps its
/// `frost` control to this and `@fx param u.frost` declares the same ceiling.
const MAX_FROST_DOC: f64 = 100.0;

/// How many backdrop mip levels a frame at `dpr` device pixels per document unit can reach.
///
/// The blur radius is a **document** length, so the shader asks for `log2(frost × dpr)`: the level
/// budget follows the zoom, not the texture. Past the level that is one texel wide the blur is
/// already "the average of the whole window", so the texture's own chain is the hard ceiling.
fn backdrop_levels(w: u32, h: u32, dpr: f32) -> u32 {
    let radius_px = (MAX_FROST_DOC * f64::from(dpr.max(1e-6))).max(1.0);
    let need = radius_px.log2().ceil() as u32 + 1;
    mip_levels(w, h).min(need.max(1))
}

/// The 2×2 box reduction that builds one backdrop mip level. A `textureLoad` block average (rather
/// than a bilinear tap) keeps every level exactly aligned to the one above it: destination pixel
/// `i` always reads source pixels `2i` and `2i+1`, so the blur's centre cannot drift by half a pixel
/// frame to frame — the half-pixel drift of the host-side chain was visible as shimmer while a pane
/// was being zoomed.
const DOWNSAMPLE_WGSL: &str = r#"
@group(0) @binding(0) var src : texture_2d<f32>;
@group(0) @binding(1) var samp : sampler;

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<i32>(textureDimensions(src));
    let hi = max(dim - vec2<i32>(1, 1), vec2<i32>(0, 0));
    let s = vec2<i32>(i32(pos.x), i32(pos.y)) * 2;
    let a = textureLoad(src, clamp(s,                     vec2<i32>(0, 0), hi), 0);
    let b = textureLoad(src, clamp(s + vec2<i32>(1, 0),   vec2<i32>(0, 0), hi), 0);
    let c = textureLoad(src, clamp(s + vec2<i32>(0, 1),   vec2<i32>(0, 0), hi), 0);
    let d = textureLoad(src, clamp(s + vec2<i32>(1, 1),   vec2<i32>(0, 0), hi), 0);
    return (a + b + c + d) * 0.25;
}
"#;

/// The pipeline that puts a pass's own children back **on top of that pass**, at the right place in
/// the z order. Children are drawn above their material, never through it.
const OVER_WGSL: &str = r#"
@group(0) @binding(0) var over : texture_2d<f32>;
@group(0) @binding(1) var samp : sampler;

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(over));
    return textureSampleLevel(over, samp, pos.xy / dim, 0.0);
}
"#;

/// The staging texture a pass's children are uploaded into before being blended back on top.
struct OverlayTex {
    w: u32,
    h: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One step of a composite, in z order.
pub enum Step<'a> {
    /// A material pass. Its backdrop is the composite **so far**.
    Pass(&'a RunPass),
    /// A pass's own children, straight RGBA, drawn right above that pass.
    ///
    /// `pixels` is exactly `rect`'s box (row stride `rect[2]`), not the whole canvas: the layer is
    /// only rendered where those children can ink. The compositor used to index it with the canvas
    /// stride and bail out when the length did not match a full canvas — which silently dropped
    /// every container's contents.
    Over { pixels: &'a [u8], rect: [u32; 4] },
}

/// The canvas box the materials actually touch: the union of every pass rect and every children
/// layer, clipped to the canvas.
///
/// Nothing is written outside it (each step has its own scissor/viewport), and the backdrop a pass
/// reads is its own window-sized texture, so this box is the only part of the canvas that has to
/// travel to the GPU and back.
fn composite_region(steps: &[Step<'_>], w: u32, h: u32) -> Option<[u32; 4]> {
    let mut r: Option<[u32; 4]> = None;
    let mut add = |x0: u32, y0: u32, x1: u32, y1: u32| {
        r = Some(match r {
            None => [x0, y0, x1, y1],
            Some(a) => [a[0].min(x0), a[1].min(y0), a[2].max(x1), a[3].max(y1)],
        });
    };
    for s in steps {
        let rect = match s {
            Step::Pass(p) => p.rect_px,
            Step::Over { rect, .. } => *rect,
        };
        if let Some([x, y, rw, rh]) = clip_rect(rect, w, h) {
            add(x, y, x + rw, y + rh);
        }
    }
    r.map(|[x0, y0, x1, y1]| [x0, y0, x1 - x0, y1 - y0])
}

/// Trim a pass rectangle to the canvas. A node that hangs over the viewport edge must not produce a
/// rect the copies and viewports would run past the texture's end.
fn clip_rect([rx, ry, rw, rh]: [u32; 4], w: u32, h: u32) -> Option<[u32; 4]> {
    let x0 = rx.min(w);
    let y0 = ry.min(h);
    let x1 = rx.saturating_add(rw).min(w);
    let y1 = ry.saturating_add(rh).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// What a `fields` texture's bytes were derived from. Equal key = the bytes are unchanged, so a
/// still pane neither rebuilds the SDF nor uploads it.
fn field_key(p: &RunPass, rw: u32, rh: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (rw, rh).hash(&mut h);
    for v in [p.node_px.0, p.node_px.1, p.origin_px.0, p.origin_px.1] {
        v.to_bits().hash(&mut h);
    }
    for r in p.radii_px {
        r.to_bits().hash(&mut h);
    }
    h.finish()
}

/// The universal offscreen compositor. One device/queue and a cache of compiled pipelines reused
/// across frames.
pub struct WgslRunner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    cache: HashMap<(String, u64), Compiled>,
    out: Option<Out>,
    /// Per-node backdrops, reused while their window keeps its size.
    backdrop: HashMap<u64, Backdrop>,
    /// Per-node `fields` textures, reused while their key holds.
    fields: HashMap<u64, FieldTex>,
    /// One staging texture per `Over` step. Shared, the uploads would all run before the draw and
    /// the last layer would overwrite every earlier one — every child layer would show the same
    /// pixels. `queue.write_texture` is enqueued ahead of the whole command buffer, so the textures
    /// have to be distinct, not the writes ordered.
    overlays: Vec<OverlayTex>,
    /// The frost pyramid's box-reduction pipeline (built once).
    mips: Downsampler,
    /// The "children above their material" pipeline (built once).
    over: Overlay,
    over_sampler: wgpu::Sampler,
    /// When the shader hot-reload sweep last ran.
    stale_at: Option<std::time::Instant>,
    adapter_info: wgpu::AdapterInfo,
    /// Ring of buffers for the non-blocking `submit_async` / `poll_ready` path. Empty until the
    /// first async frame; the synchronous `composite_steps` path never touches it.
    async_slots: Vec<ReadbackSlot>,
    /// Monotonic id handed to `submit_async`, echoed by `poll_ready`, so a frame can pick up its
    /// *own* pixels even when several are in flight and they complete out of order.
    next_async_id: u64,
}

/// Built once: the 2×2 box reduction used to generate the backdrop's mip chain on the GPU.
struct Downsampler {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}
/// Built once: the pipeline that draws a pass's children above its finished material.
struct Overlay {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

fn make_fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    code: &str,
    layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(code.into()) });
    let gl =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&gl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl WgslRunner {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("figcraft-universal-wgsl"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("universal filter"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let tex_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fullscreen pair"), entries: &[tex_entry, samp_entry] });
        let mips = Downsampler {
            pipeline: make_fullscreen_pipeline(&device, "universal downsample", DOWNSAMPLE_WGSL, &layout, Some(wgpu::BlendState::REPLACE), FORMAT),
            layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fullscreen pair"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            }),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("universal nearest"),
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            }),
        };
        let over = Overlay {
            layout: device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("overlay pair"), entries: &[tex_entry, samp_entry] }),
            pipeline: make_fullscreen_pipeline(&device, "universal overlay", OVER_WGSL, &layout, Some(wgpu::BlendState::ALPHA_BLENDING), FORMAT),
        };
        Some(Self {
            device,
            queue,
            cache: HashMap::new(),
            out: None,
            backdrop: HashMap::new(),
            fields: HashMap::new(),
            overlays: Vec::new(),
            mips,
            over,
            over_sampler: sampler,
            stale_at: None,
            adapter_info,
            async_slots: Vec::new(),
            next_async_id: 0,
        })
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    fn ensure_out(&mut self, w: u32, h: u32) {
        if self.out.as_ref().is_some_and(|o| o.w == w && o.h == h) {
            return;
        }
        let padded = (w * 4).div_ceil(COPY_ALIGN) * COPY_ALIGN;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("universal output canvas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("universal readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        self.out = Some(Out { w, h, padded, texture, view, readback });
    }

    /// Make sure `n` staging textures exist at the canvas size — one per `Over` step in this frame.
    fn ensure_overlays(&mut self, n: usize, w: u32, h: u32) {
        if self.overlays.len() > n {
            self.overlays.truncate(n);
        }
        while self.overlays.len() < n {
            let t = self.make_overlay(w, h);
            self.overlays.push(t);
        }
        for i in 0..self.overlays.len() {
            if self.overlays[i].w == w && self.overlays[i].h == h {
                continue;
            }
            let t = self.make_overlay(w, h);
            self.overlays[i] = t;
        }
    }

    fn make_overlay(&self, w: u32, h: u32) -> OverlayTex {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("universal overlay"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OVERLAY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OverlayTex { w, h, texture, view }
    }

    /// One node's backdrop: its window plus the mip chain its frost can actually reach, kept while
    /// the window keeps its size.
    fn ensure_backdrop(&mut self, node: u64, w: u32, h: u32, dpr: f32) {
        let count = backdrop_levels(w, h, dpr);
        if self.backdrop.get(&node).is_some_and(|t| t.w == w && t.h == h && t.levels.len() as u32 == count) { return; }
        // The chain has to cover `log2(frost × dpr)`, and `dpr` is the zoom: a budget fixed at eight
        // levels covers every radius only while `dpr ≈ 1`. Above that the shader's `frost` silently
        // clamped to level 7 (a 128-px blur) no matter how far you zoomed in — the pane's interior
        // stopped being frosted and became a 1:1 copy of the backdrop, which reads as "the glass
        // vanished" as soon as the pane's own edge was off-screen. Deeper levels are 4× cheaper per
        // level than the one above, so following `dpr` to the texture's own chain costs almost
        // nothing.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("universal backdrop"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut levels = Vec::with_capacity(count as usize);
        for l in 0..count {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let v = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("backdrop level"),
                base_mip_level: l,
                mip_level_count: Some(1),
                ..Default::default()
            });
            levels.push((lw, lh, v));
        }
        self.backdrop.insert(node, Backdrop { w, h, texture, view, levels });
    }

    /// Make sure the node's `fields` texture exists at `w` × `h`. Returns `true` when the bytes behind
    /// it changed (or the texture was just created), i.e. when it has to be uploaded again.
    fn ensure_field(&mut self, node: u64, w: u32, h: u32, key: u64) -> bool {
        if let Some(t) = self.fields.get_mut(&node)
            && t.w == w
            && t.h == h
        {
            let changed = t.key != key;
            t.key = key;
            return changed;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("universal fields"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FIELDS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.fields.insert(node, FieldTex { key, w, h, texture, view });
        true
    }

    /// Compile what is missing, and (at most twice a second) notice a shader edited on disk.
    fn refresh_pipelines(&mut self, w: u32, h: u32, steps: &[Step<'_>]) {
        let mut needed: Vec<(String, u64, ShaderInfo)> = Vec::new();
        for step in steps {
            let Step::Pass(p) = step else { continue };
            if self.cache.get(&(p.effect.clone(), p.node.0)).is_none_or(|c| c.hash == 0)
                && !needed.iter().any(|(e, n, _)| *e == p.effect && *n == p.node.0)
                && let Some(info) = shader_info(&p.effect)
            {
                needed.push((p.effect.clone(), p.node.0, info));
            }
        }
        for (effect, node, info) in needed {
            match compile(&self.device, &info, (w, h), FORMAT) {
                Ok(c) => {
                    self.cache.insert((effect, node), c);
                }
                // A shader that fails to compile silently leaves the CPU image on screen, which
                // looks exactly like "the material does nothing". Say so once per pass.
                Err(e) => eprintln!("universal material '{effect}' failed to compile: {e}"),
            }
        }
        // Hot reload means reading and hashing every shader; on a per-pass, per-frame path that was
        // pure overhead. Twice a second is still instant to a person editing the file.
        if self.stale_at.is_some_and(|t| t.elapsed().as_millis() < 500) {
            return;
        }
        self.stale_at = Some(std::time::Instant::now());
        let stale: Vec<(String, u64)> =
            self.cache.iter().filter(|(k, c)| shader_info(&k.0).map(|s| s.hash != c.hash).unwrap_or(true)).map(|(k, _)| k.clone()).collect();
        for k in stale {
            self.cache.remove(&k);
        }
    }

    /// Composite materials over the CPU canvas. `canvas` is straight BGRA, top-down; the result is
    /// straight BGRA of the same size. `None` on any failure (the caller keeps the CPU image).
    pub fn composite(&mut self, canvas: &[u8], w: u32, h: u32, passes: &[RunPass]) -> Option<Vec<u8>> {
        let steps: Vec<Step<'_>> = passes.iter().map(Step::Pass).collect();
        self.composite_steps(canvas, w, h, &steps, None)
    }

    /// One composite, in z order. A `Step::Pass` reads **the composite so far** as its backdrop, so a
    /// material stacked on another material sees the finished lower one instead of the raw canvas —
    /// the earlier implementation cropped every pass out of the same CPU image, which is why a second
    /// glass pane ignored the first. `Step::Over` puts that pass's children back on top of it, so the
    /// z order survives nesting instead of every child floating above the whole stack.
    ///
    /// Everything is encoded into **one** command buffer and submitted once: the old path submitted
    /// per pass (and allocated a command encoder, bind groups and a texture per pass, per frame).
    /// The synchronous path: encode + submit, then block until the region's pixels are mapped and
    /// spliced back into `canvas`. Kept byte-for-byte identical for the drag patch and overview
    /// paths (which must show this frame's material *now*). The refined path uses
    /// [`Self::submit_async`] + [`Self::poll_ready`] instead.
    pub fn composite_steps(&mut self, canvas: &[u8], w: u32, h: u32, steps: &[Step<'_>], shade: Option<[u32; 4]>) -> Option<Vec<u8>> {
        let (region, padded) = self.encode_submit(canvas, w, h, steps, None, shade)?;
        self.map_region(w, h, region, padded, canvas)
    }

    /// Encode the whole stack and submit it **once**, copying the touched region into `readback`.
    /// When `readback` is `None` this uses `self.out.readback` (the synchronous path); otherwise it
    /// uses the caller's async slot buffer. Returns the region and its padded row stride. It does
    /// **not** map anything — the caller decides to block ([`Self::composite_steps`]) or defer
    /// ([`Self::submit_async`]).
    ///
    /// All shared resources (the output texture, per-node backdrops/fields, the overlay staging
    /// textures) live on one queue and are written by `queue.write_texture`, which the WebGPU queue
    /// orders after every earlier `submit`. So frame N+1 may re-encode into the same textures as
    /// soon as frame N is submitted: the GPU consumes the queue in order and never reads a texture
    /// that a later frame has overwritten. The **readback** is the one resource that cannot be
    /// reused until its `map_async` completes and it is unmapped — hence the per-frame slot.
    fn encode_submit(&mut self, canvas: &[u8], w: u32, h: u32, steps: &[Step<'_>], readback: Option<&wgpu::Buffer>, shade: Option<[u32; 4]>) -> Option<([u32; 4], u32)> {
        let expected = (w as usize) * (h as usize) * 4;
        if steps.is_empty() || w == 0 || h == 0 || canvas.len() < expected {
            return None;
        }
        self.ensure_out(w, h);
        let over_count = steps.iter().filter(|s| matches!(s, Step::Over { .. })).count();
        self.ensure_overlays(over_count, w, h);

        let t_up = std::time::Instant::now();
        // 1. Seed the canvas from the CPU image — **only the box the materials touch**. Everything
        //    outside `region` is copied straight through, so pushing the whole canvas up and pulling
        //    it back down was pure waste, and the pull is a synchronous wait: it stalled the render
        //    thread for as long as a full-screen effect even when the pane was a small card.
        let Some([gx, gy, gw, gh]) = composite_region(steps, w, h) else { return None };
        {
            let out = self.out.as_ref()?;
            let row = (gw * 4) as usize;
            let mut seed = vec![0u8; row * gh as usize];
            for y in 0..gh as usize {
                let src = ((gy as usize + y) * (w as usize) + gx as usize) * 4;
                seed[y * row..(y + 1) * row].copy_from_slice(&canvas[src..src + row]);
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &out.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: gx, y: gy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &seed,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(gw * 4), rows_per_image: Some(gh) },
                wgpu::Extent3d { width: gw, height: gh, depth_or_array_layers: 1 },
            );
        }

        let up_ms = t_up.elapsed().as_secs_f64() * 1000.0;
        let t_enc = std::time::Instant::now();

        // 2. Pipelines.
        self.refresh_pipelines(w, h, steps);

        // 3. Per-pass resources that need `&mut self`: backdrop textures and `fields` textures. The
        //    SDF bytes themselves are only *uploaded* when their key moved — a still frame with a
        //    steady rect uploads nothing at all.
        for step in steps {
            let Step::Pass(p) = step else { continue };
            let Some([_, _, rw, rh]) = clip_rect(p.rect_px, w, h) else { continue };
            self.ensure_backdrop(p.node.0, rw, rh, p.dpr);
            let key = field_key(p, rw, rh);
            let changed = self.ensure_field(p.node.0, rw, rh, key);
            // A rect's field is analytic and its key is exact, so an unchanged rect can skip. A real
            // outline's bytes come from the node's geometry: always upload those, they are cheap to
            // send and a stale one would be a wrong shape.
            let upload = changed || p.field_bytes.is_some();
            if !upload {
                continue;
            }
            let bytes = p.field_bytes.clone().unwrap_or_else(|| fields_for_rect_banded(rw, rh, p.node_px, p.origin_px, p.radii_px, p.field_band));
            if let Some(fd) = self.fields.get(&p.node.0) {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &fd.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rw * 8), rows_per_image: Some(rh) },
                    wgpu::Extent3d { width: rw, height: rh, depth_or_array_layers: 1 },
                );
            }
        }

        // 4. One command buffer for the whole stack, submitted once.
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("universal materials") });
        let mut over_at = 0usize;
        for step in steps {
            match step {
                Step::Over { pixels, rect } => {
                    let slot = over_at;
                    over_at += 1;
                    let [rx, ry, rw, rh] = *rect;
                    if rw == 0 || rh == 0 || pixels.len() < (rw as usize) * (rh as usize) * 4 {
                        continue;
                    }
                    let Some([ox, oy, ow, oh]) = clip_rect(*rect, w, h) else { continue };
                    let ovl = self.overlays.get(slot)?;
                    let out = self.out.as_ref()?;
                    {
                        // Crop to the canvas in the layer's **own** stride (`rw`), then upload.
                        let row = (ow * 4) as usize;
                        let mut crop = vec![0u8; row * (oh as usize)];
                        for y in 0..oh as usize {
                            let src = (((oy - ry) as usize + y) * (rw as usize) + (ox - rx) as usize) * 4;
                            crop[y * row..(y + 1) * row].copy_from_slice(&pixels[src..src + row]);
                        }
                        // Written at the layer's **canvas** position, not at 0,0: the overlay
                        // shader samples it at `pos.xy / size`, so the texture has to line up with
                        // the canvas or the children land at the wrong place.
                        self.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &ovl.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: ox, y: oy, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &crop,
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(ow * 4), rows_per_image: Some(oh) },
                            wgpu::Extent3d { width: ow, height: oh, depth_or_array_layers: 1 },
                        );
                    }
                    let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("universal overlay"),
                        layout: &self.over.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&ovl.view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.over_sampler) },
                        ],
                    });
                    let att = [Some(wgpu::RenderPassColorAttachment {
                        view: &out.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })];
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("universal overlay pass"),
                        color_attachments: &att,
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    rp.set_viewport(ox as f32, oy as f32, ow as f32, oh as f32, 0.0, 1.0);
                    rp.set_scissor_rect(ox, oy, ow, oh);
                    rp.set_pipeline(&self.over.pipeline);
                    rp.set_bind_group(0, &bg, &[]);
                    rp.draw(0..3, 0..1);
                }
                Step::Pass(p) => {
                    let Some([rx, ry, rw, rh]) = clip_rect(p.rect_px, w, h) else { continue };
                    let key = (p.effect.clone(), p.node.0);
                    let Some(ci) = self.cache.get(&key) else { continue };
                    let Some(bd) = self.backdrop.get(&p.node.0) else { continue };
                    let Some(fd) = self.fields.get(&p.node.0) else { continue };
                    let out = self.out.as_ref()?;

                    // Backdrop: copy the composited-so-far canvas into level 0, then build the rest of
                    // the pyramid on the GPU. This copy is what makes layered materials correct.
                    enc.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &out.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x: rx, y: ry, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &bd.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d { width: rw, height: rh, depth_or_array_layers: 1 },
                    );
                    for l in 1..bd.levels.len() {
                        let (sw, sh, sv) = &bd.levels[l - 1];
                        let (dw, dh, dv) = &bd.levels[l];
                        // Only the part level 0 actually holds — the mip texture is sized to the
                        // window, so this is the whole level unless the window shrank by an odd pixel.
                        let vw = (*dw).min(sw.div_ceil(2).max(1));
                        let vh = (*dh).min(sh.div_ceil(2).max(1));
                        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("universal downsample"),
                            layout: &self.mips.layout,
                            entries: &[
                                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(sv) },
                                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.mips.sampler) },
                            ],
                        });
                        // The viewport covers everything this level holds, so there is nothing to
                        // load: clearing is cheaper than pulling the old contents back in.
                        let att = [Some(wgpu::RenderPassColorAttachment {
                            view: dv,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                        })];
                        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("universal downsample pass"),
                            color_attachments: &att,
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_viewport(0.0, 0.0, vw as f32, vh as f32, 0.0, 1.0);
                        rp.set_scissor_rect(0, 0, vw, vh);
                        rp.set_pipeline(&self.mips.pipeline);
                        rp.set_bind_group(0, &bg, &[]);
                        rp.draw(0..3, 0..1);
                    }

                    for (var, bytes) in &p.uniforms {
                        if let Some((_, buf)) = ci.ubufs.iter().find(|(n, _)| n == var) {
                            self.queue.write_buffer(buf, 0, bytes);
                        }
                    }

                    // One bind group per declared group, assigning each binding by its contract kind.
                    let mut built: Vec<(u32, wgpu::BindGroup)> = Vec::new();
                    for g in 0..ci.layouts.len() as u32 {
                        let entries: Vec<wgpu::BindGroupEntry> = ci
                            .resource_binds
                            .iter()
                            .filter(|(gr, _, _)| *gr == g)
                            .filter_map(|(_, b, kind)| {
                                let resource = match *kind {
                                    "sampler" => wgpu::BindingResource::Sampler(&ci.sampler),
                                    "fields" => wgpu::BindingResource::TextureView(&fd.view),
                                    _ => wgpu::BindingResource::TextureView(&bd.view),
                                };
                                Some(wgpu::BindGroupEntry { binding: *b, resource })
                            })
                            .chain(ci.ubind.iter().filter(|(_, gr, _)| *gr == g).filter_map(|(name, _, b)| {
                                let (_, buf) = ci.ubufs.iter().find(|(n, _)| n == name)?;
                                Some(wgpu::BindGroupEntry { binding: *b, resource: buf.as_entire_binding() })
                            }))
                            .collect();
                        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("universal bind group"),
                            layout: &ci.layouts[g as usize],
                            entries: &entries,
                        });
                        built.push((g, bg));
                    }

                    let att = [Some(wgpu::RenderPassColorAttachment {
                        view: &out.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })];
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("universal glass pass"),
                        color_attachments: &att,
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    rp.set_viewport(rx as f32, ry as f32, rw as f32, rh as f32, 0.0, 1.0);
                    // 底图仍覆盖整个 pass 区域（视口外那一圈要给透镜采样），但**着色**只做可见视口：
                    // 那一圈不在屏幕上，逐像素跑 shader 只会把一帧拖到 250ms。
                    let scissor = shade.and_then(|s| {
                        let x0 = s[0].max(rx);
                        let y0 = s[1].max(ry);
                        let x1 = (s[0] + s[2]).min(rx + rw);
                        let y1 = (s[1] + s[3]).min(ry + rh);
                        (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
                    });
                    match scissor {
                        Some((sx, sy, sw, sh)) => rp.set_scissor_rect(sx, sy, sw, sh),
                        None => rp.set_scissor_rect(rx, ry, rw, rh),
                    }
                    rp.set_pipeline(&ci.pipeline);
                    for (g, bg) in &built {
                        rp.set_bind_group(*g, bg, &[]);
                    }
                    rp.draw(0..3, 0..1);
                }
            }
        }

        // 5. Copy the region out — into the caller's async slot, or the shared sync readback.
        let padded = (gw * 4).div_ceil(256) * 256;
        {
            let out = self.out.as_ref()?;
            let target = readback.unwrap_or(&out.readback);
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &out.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: gx, y: gy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: target,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(gh) },
                },
                wgpu::Extent3d { width: gw, height: gh, depth_or_array_layers: 1 },
            );
        }
        let enc_ms = t_enc.elapsed().as_secs_f64() * 1000.0;
        self.queue.submit(Some(enc.finish()));
        if std::env::var_os("FIGCRAFT_RENDER_LOG").is_some() {
            eprintln!(
                "[gpu] {w}x{h} region {gw}x{gh} upload {up_ms:.1}ms encode {enc_ms:.1}ms submit{}",
                if readback.is_some() { " (async)" } else { "" }
            );
        }
        Some(([gx, gy, gw, gh], padded))
    }

    /// Count how many material passes in `steps` actually have a compiled pipeline after the last
    /// [`Self::composite_steps`] (which calls `refresh_pipelines` to fill `cache`).
    ///
    /// `composite_steps` returns a well-formed buffer even when a shader failed to load or compile:
    /// it skips that pass's draw, still draws the `Over` children, and reads back a full-size image.
    /// Callers that treat "some pixels came back" as success would then show the container's children
    /// with the glass itself missing. Comparing `(drawn, total)` catches that silent partial failure
    /// so the host can fall back to the CPU renderer instead.
    pub fn steps_drawn(&self, steps: &[Step<'_>]) -> (usize, usize) {
        let mut total = 0usize;
        let mut drawn = 0usize;
        for step in steps {
            let Step::Pass(p) = step else { continue };
            total += 1;
            if self.cache.get(&(p.effect.clone(), p.node.0)).is_some_and(|c| c.hash != 0) {
                drawn += 1;
            }
        }
        (drawn, total)
    }

    /// Compile any missing pipelines for `steps` **before** an async submit, then report
    /// `(drawn, total)`. The async host must refuse to enqueue a frame whose stack is incomplete:
    /// an asynchronous submit that silently skipped a glass pass would hand back a container with
    /// its contents but no material, and by the time the pixels come back the frame has moved on.
    /// Doing the (synchronous, cached) compile up front lets the host fall back to the synchronous
    /// path — whose own `encode_submit` compiles too — without ever occupying a readback slot.
    ///
    /// After this returns `(n, n)`, the immediately following [`Self::submit_async`] re-enters
    /// `refresh_pipelines` but finds every cache entry warm (plus the 500 ms stale check), so the
    /// second pass is essentially free.
    pub fn ensure_pipelines(&mut self, w: u32, h: u32, steps: &[Step<'_>]) -> (usize, usize) {
        self.refresh_pipelines(w, h, steps);
        self.steps_drawn(steps)
    }

    /// Synchronous map: block until the shared `out.readback` is ready, splice the region in,
    /// unmap, return the full image. Used by the drag patch / overview paths only.
    fn map_region(&self, w: u32, h: u32, region: [u32; 4], padded: u32, base: &[u8]) -> Option<Vec<u8>> {
        let out = self.out.as_ref()?;
        let (tx, rx) = std::sync::mpsc::channel();
        out.readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let _ = rx.recv();
        self.splice_mapped(&out.readback, w, h, region, padded, base)
    }

    /// Take an already-mapped `buffer`, splice `region` into a copy of `base`, and unmap it.
    /// Shared by the synchronous path (after its blocking wait) and the async path (after
    /// `poll_ready` observed the map callback). Outside the region nothing was written, so the input
    /// *is* the output there — copying it is both correct and the cheapest way to preserve the rest
    /// of the canvas.
    fn splice_mapped(&self, buffer: &wgpu::Buffer, w: u32, h: u32, region: [u32; 4], padded: u32, base: &[u8]) -> Option<Vec<u8>> {
        let [gx, gy, gw, gh] = region;
        let row = gw * 4;
        let slice = buffer.slice(..);
        let mapped = slice.get_mapped_range().ok()?;
        let mut data = base[..((w as usize) * (h as usize) * 4)].to_vec();
        for y in 0..gh as usize {
            let src = y * padded as usize;
            let dst = ((gy as usize + y) * (w as usize) + gx as usize) * 4;
            data[dst..dst + row as usize].copy_from_slice(&mapped[src..src + row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Some(data)
    }

    // ---- Non-blocking readback pipeline ----------------------------------------------------
    // The encode + submit half is [`Self::encode_submit`] above (passing `Some(&slot.buffer)`).
    // These methods own the slot ring, the map callbacks and the frame-id bookkeeping. Nothing here
    // blocks: `submit_async` returns as soon as the command buffer is on the queue, and `poll_ready`
    // only collects frames whose map callback has already fired.

    /// Create the ring lazily (no GPU buffers until the first async frame). Slots grow to fit their
    /// region and are reused, so a steady-size pane allocates nothing after the first few frames.
    fn ensure_slots(&mut self) {
        while self.async_slots.len() < ASYNC_SLOTS {
            // A 256-byte placeholder; `grow_slot` reallocates to the real region size on first use.
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("universal async readback"),
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.async_slots.push(ReadbackSlot { buffer, w: 0, h: 0, padded: 0, region: [0; 4], frame_id: None, map_rx: None, in_use: false });
        }
    }

    /// Reserve a free slot whose buffer is at least `padded × h` bytes. Reuses a slot only after
    /// `poll_ready` has unmapped it; a slot still awaiting a map callback is never recycled, which
    /// is exactly why the ring must be sized for the maximum in-flight frames. Returns `None` when
    /// every slot is busy — the caller then keeps its previous picture (back-pressure by dropping
    /// the newest frame, never by blocking).
    fn acquire_slot(&mut self, padded: u32, h: u32) -> Option<usize> {
        self.ensure_slots();
        let need = (padded.max(256) as u64) * (h.max(1) as u64);
        let i = self.async_slots.iter().position(|s| !s.in_use)?;
        if self.async_slots[i].buffer.size() < need {
            self.async_slots[i].buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("universal async readback (grown)"),
                size: need,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        }
        Some(i)
    }

    /// Submit one composite **without waiting for its pixels**. Encodes into a free readback slot,
    /// submits, fires `map_async` on that slot, and returns a frame id. Collect the pixels later
    /// with [`Self::poll_ready`], passing the CPU canvas snapshot for *this* frame: region rows come
    /// from the GPU, everything else is preserved from that snapshot. It cannot be re-derived at
    /// collect time from a later frame whose view has moved, so the caller must hold on to it.
    ///
    /// Returns `None` if there is no free slot (all ASYNC_SLOTS frames still in flight), or on the
    /// same encode conditions `composite_steps` refuses. In both cases the canvas is untouched.
    pub fn submit_async(&mut self, canvas: &[u8], w: u32, h: u32, steps: &[Step<'_>], shade: Option<[u32; 4]>) -> Option<u64> {
        let expected = (w as usize) * (h as usize) * 4;
        if steps.is_empty() || w == 0 || h == 0 || canvas.len() < expected {
            return None;
        }
        let [gx, gy, gw, gh] = composite_region(steps, w, h)?;
        let padded = (gw * 4).div_ceil(256) * 256;
        let i = self.acquire_slot(padded, gh)?;

        let id = self.next_async_id;
        self.next_async_id += 1;

        // Clone the buffer handle out: `encode_submit` needs `&mut self`, which cannot coexist with
        // a borrow into `self.async_slots`. Both handles point at the same GPU allocation.
        let buffer = self.async_slots[i].buffer.clone();
        let (region, padded) = match self.encode_submit(canvas, w, h, steps, Some(&buffer), shade) {
            Some(v) => v,
            None => {
                self.async_slots[i].in_use = false;
                return None;
            }
        };

        // One channel per frame. `tx` goes to the map callback (which only signals); `rx` is stored
        // on the slot so `poll_ready(id, ..)` can find exactly this frame's completion.
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let s = &mut self.async_slots[i];
            s.in_use = true;
            s.w = w;
            s.h = h;
            s.padded = padded;
            s.region = region;
            s.frame_id = Some(id);
            s.map_rx = Some(rx);
        }
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        Some(id)
    }

    /// Advance all pending `map_async` callbacks **without blocking**. Call once per frame even when
    /// no new work was submitted — a device with no submit still needs a poll to fire its callbacks.
    ///
    /// Why `Wait { timeout: 0 }` and not `Poll`: in wgpu-core, `PollType::Poll` skips `raw().wait()`
    /// entirely and only reads the cached fence value (device/resource.rs:865-894). With several
    /// submissions queued and no `wait` to flush it, that cached value can stay stale, so
    /// `triage_submissions` never moves the finished buffers into `ready_to_map` and no map callback
    /// fires (confirmed: three back-to-back submits never completed under `Poll`). A zero-timeout
    /// `Wait` calls `raw().wait(fence, last_submission, Some(0))` — `vkWaitForFences(..., 0)` on
    /// Vulkan / the equivalent on DX12 & Metal — which flushes the fence value **without blocking**,
    /// then triages every completed submission. A frame that is still on the GPU simply yields
    /// `Timeout`, which we ignore; its callback fires on a later pump.
    pub fn pump(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::ZERO) });
    }

    /// Collect the pixels of frame `id` if they are ready, or `None` if it is still in flight.
    /// Non-blocking: one `poll` advances callbacks; a completed one is spliced into its own `base`,
    /// unmapped and returned to the slot pool. `None` is also returned for an unknown or already
    /// collected id. `base` must be the exact snapshot given to `submit_async`.
    pub fn poll_ready(&mut self, id: u64, base: &[u8]) -> Option<Vec<u8>> {
        self.pump();
        let i = self.async_slots.iter().position(|s| s.in_use && s.frame_id == Some(id))?;
        let ready = match self.async_slots[i].map_rx.as_ref() {
            Some(rx) => rx.try_recv(),
            None => return None,
        };
        match ready {
            Ok(Ok(())) => {
                let (region, padded, w, h) = {
                    let s = &self.async_slots[i];
                    (s.region, s.padded, s.w, s.h)
                };
                let out = self.splice_mapped(&self.async_slots[i].buffer.clone(), w, h, region, padded, base);
                if out.is_none() {
                    // `splice_mapped` bailed before its own `unmap()` (e.g. `get_mapped_range`
                    // failed). A reused mapped buffer is a validation error on the next
                    // `copy_texture_to_buffer`; `unmap` is a no-op when already unmapped.
                    self.async_slots[i].buffer.unmap();
                }
                let s = &mut self.async_slots[i];
                s.in_use = false;
                s.frame_id = None;
                s.map_rx = None;
                out
            }
            // Map failed, or the sender was dropped: hand the slot back but report nothing, so the
            // caller keeps its last good picture. A failed map leaves the buffer unmapped.
            Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let s = &mut self.async_slots[i];
                s.in_use = false;
                s.frame_id = None;
                s.map_rx = None;
                None
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        }
    }

    /// How many async frames are currently in flight (owned by a slot, pixels not yet collected).
    pub fn in_flight(&self) -> usize {
        self.async_slots.iter().filter(|s| s.in_use).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read one half-float of a `fields` buffer.
    fn f16_at(bytes: &[u8], x: usize, y: usize, w: usize) -> [f32; 4] {
        let i = (y * w + x) * 8;
        let h = |o: usize| -> f32 {
            let bits = u16::from_le_bytes([bytes[i + o], bytes[i + o + 1]]);
            let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
            let exp = ((bits >> 10) & 0x1f) as i32;
            let man = f32::from(bits & 0x3ff);
            if exp == 0 {
                sign * man * 2f32.powi(-24)
            } else if exp == 31 {
                if man == 0.0 { sign * f32::INFINITY } else { f32::NAN }
            } else {
                sign * (1.0 + man / 1024.0) * 2f32.powi(exp - 15)
            }
        };
        [h(0), h(2), h(4), h(6)]
    }

    #[test]
    fn fields_center_is_deep_inside_and_edge_has_coverage() {
        let (w, h) = (64, 64);
        let f = fields_for_rect(w, h, (64.0, 64.0), (0.0, 0.0), [8.0, 8.0, 8.0, 8.0]);
        assert_eq!(f.len(), (w * h * 8) as usize);
        let at = |x: usize, y: usize| f16_at(&f, x, y, w as usize);
        let centre = at(32, 32);
        assert!(centre[0] < -25.0, "centre is ~32 px inside (negative SDF): {centre:?}");
        assert!((centre[3] - 1.0).abs() < 1e-3, "centre fully covered: {centre:?}");
        let corner_out = at(1, 1);
        assert!(corner_out[3] < 0.05, "rounded corner outside is uncovered: {corner_out:?}");
        let edge_in = at(32, 2);
        assert!(edge_in[3] > 0.8, "just inside the top edge is covered: {edge_in:?}");
    }

    /// A window that is only the visible part of a bigger node must describe the *node's* edge,
    /// not the window's: this is the "a pane zoomed full screen grows rounded corners in the
    /// viewport's corners" bug. The window here is the top-left 40x40 of a 200x200 node.
    #[test]
    fn a_clipped_window_describes_the_node_edge_not_the_window_edge() {
        let (w, h) = (40u32, 40u32);
        let f = fields_for_rect(w, h, (200.0, 200.0), (0.0, 0.0), [12.0; 4]);
        let at = |x: usize, y: usize| f16_at(&f, x, y, w as usize);
        let inside = at(20, 20);
        assert!((inside[0] - -20.5).abs() < 0.2, "20,20 sits 20.5 px inside a 200 px node: {inside:?}");
        assert!((inside[3] - 1.0).abs() < 1e-3, "still covered: {inside:?}");
        // The node's top-left corner is at the window's origin: rounded, uncovered.
        assert!(at(1, 1)[3] < 0.05, "the node's own rounded corner is outside: {:?}", at(1, 1));
        // The window's far corner is deep inside the node (no fake corner there).
        assert!(at(39, 39)[3] > 0.99, "the clipped edge is not a shape edge: {:?}", at(39, 39));
        // A window offset into the node reports the rest of the node's depth, not a fresh edge.
        let offset = fields_for_rect(40, 40, (200.0, 200.0), (80.0, 80.0), [12.0; 4]);
        let deep = f16_at(&offset, 20, 20, 40);
        assert!((deep[0] - -99.5).abs() < 0.3, "the window centre is on the node's centre line: {deep:?}");
        assert!(f16_at(&offset, 0, 0, 40)[0] < -80.0, "the window's own edge is 80 px inside the node");
    }

    #[test]
    fn diagonal_normal_is_encoded_inward() {
        let (w, h) = (64, 64);
        let f = fields_for_rect(w, h, (64.0, 64.0), (0.0, 0.0), [0.0, 0.0, 0.0, 0.0]);
        let at = |x: usize, y: usize| {
            let p = f16_at(&f, x, y, w as usize);
            (p[1] * 2.0 - 1.0, p[2] * 2.0 - 1.0)
        };
        // Edge midpoints (not the corner diagonal, where kx==ky makes the branch degenerate).
        let (lx, ly) = at(2, 32); // just inside the LEFT edge -> inward normal points right (+1, 0)
        assert!(lx > 0.9 && ly.abs() < 0.2, "left edge inward normal is +x: ({lx}, {ly})");
        let (tx, ty) = at(32, 2); // just inside the TOP edge -> inward normal points down (0, +1)
        assert!(ty > 0.9 && tx.abs() < 0.2, "top edge inward normal is +y: ({tx}, {ty})");
    }

    /// The shipped original glass.wgsl must parse under the universal contract: its 7 @fx params,
    /// its `u` uniform struct (with resolution as a generic export), and its backdrop/fields/sampler
    /// bindings. No shader-specific host code — only the declared names matter.
    #[test]
    fn original_glass_shader_parses_under_universal_contract() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        assert!(path.exists(), "original glass.wgsl ships: {}", path.display());
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        // Keep the registry lock in its own scope: `shader_info` below re-locks fx::shared(), and a
        // non-reentrant Mutex held across that call would deadlock the test thread.
        {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            let scan = reg.scan(&eid).expect("glass.wgsl scans");
            let params = reg.params(&eid);
            assert_eq!(params.len(), 7, "the seven physical controls, no dead F/T fields: {params:?}");
            assert!(params.iter().all(|p| p.field.starts_with("u.")), "all controls on the u uniform");
            assert!(scan.uniforms.contains_key("u"), "the u uniform struct is discovered");
            assert!(scan.uniforms["u"].fields.iter().any(|f| f.name == "resolution"), "resolution field present");
            let names: Vec<&str> = scan.bindings.iter().map(|b| b.name.as_str()).collect();
            for required in ["backdrop", "samp", "fields", "u"] {
                assert!(names.contains(&required), "glass binding '{required}' declared: {names:?}");
            }
            // The universal runner keys resources by binding NUMBER; guard that the scanner keeps
            // each declared binding distinct (a past bug mapped every uniform to its group number).
            let slots: Vec<(u32, u32)> = scan.bindings.iter().map(|b| (b.group, b.binding)).collect();
            let unique: std::collections::BTreeSet<(u32, u32)> = slots.iter().copied().collect();
            assert_eq!(unique.len(), slots.len(), "no two resources share a (group,binding): {slots:?}");
            assert!(scan.bindings.iter().any(|b| b.name == "u" && b.uniform && b.binding == 3), "u uniform at binding 3: {slots:?}");
        } // lock released
        let info = shader_info(&eid).expect("shader info");
        assert_eq!(info.fragments, vec!["fs".to_string()], "single fragment entry -> universal 1 pass");
    }

    /// A node with a `figcraft.wgslMaterial` binding must yield exactly one pass, with the node's
    /// device-pixel rect — this is the step between "the panel wrote plugin_data" and "the GPU
    /// composites"; an empty list here means a bound material can never reach the screen.
    #[test]
    fn passes_for_finds_a_bound_frame() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let f = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 10.0, 20.0, 100.0, 80.0).expect("a frame");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(f).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());

        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 };
        let passes = passes_for(&doc, page, &view, 0.0);
        assert_eq!(passes.len(), 1, "one bound node → one pass");
        assert_eq!(passes[0].node, f);
        assert_eq!(passes[0].rect_px, [10, 20, 100, 80], "the frame's device-pixel rect");
        assert!(!passes[0].uniforms.is_empty(), "the shader's uniforms are packed");

        // Unbinding leaves nothing to composite.
        doc.get_mut(f).expect("the frame").plugin_data.remove(crate::wgsl_material::BINDING_KEY);
        assert!(passes_for(&doc, page, &view, 0.0).is_empty(), "an unbound node yields no pass");
    }

    /// Depth must be the CPU renderer's Depth, and it must be scale-invariant: the glass profile is
    /// defined in document units, so zooming the canvas must not widen or narrow the bend band
    /// *relative to the node*. The shader converts depth -> device px with `dpr` (= view.scale),
    /// exactly like `depth = g.depth * scale` and `bevel = BEVEL * scale` on the CPU path — no other
    /// coefficient in between.
    #[test]
    fn glass_depth_is_scale_invariant() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let f = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 0.0, 0.0, 220.0, 140.0).expect("a frame");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(f).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());

        // uniform layout: resolution(vec2 @0), light_angle@8, light_int@12, refraction@16,
        // depth@20, dispersion@24, frost@28, splay@32, dpr@36, nodeSizePx@40, regionOriginPx@48.
        let f32at = |b: &[u8], off: usize| f32::from_le_bytes(b[off..off + 4].try_into().unwrap());
        let probe = |zoom: f64| -> (f32, f32, f32, f32, f32) {
            let view = figcraft_render::View { origin: (0.0, 0.0), scale: zoom, width: 2000, height: 1600 };
            let passes = passes_for(&doc, page, &view, 0.0);
            let p = passes.into_iter().next().expect("one bound frame yields one pass");
            let (_, u) = p.uniforms.into_iter().find(|(n, _)| n == "u").expect("the u uniform");
            let res_x = f32at(&u, 0);
            let depth = f32at(&u, 20);
            let dpr = f32at(&u, 36);
            // Exactly the shader's conversion, which is the CPU renderer's: Depth is a length in
            // document units, so it is multiplied by dpr and by nothing else; the rim band is the
            // renderer's constant one-unit BEVEL. (The render wasm copies bevelSize into the
            // uniform block at 179 sites with zero `f32.mul` — no coefficient exists to copy.)
            let rd_px = depth * dpr;
            let bevel_px = 1.0 * dpr;
            (res_x, depth, dpr, rd_px, bevel_px)
        };
        let (res1, depth1, dpr1, rd1, bev1) = probe(1.0);
        let (res2, depth2, dpr2, rd2, bev2) = probe(2.0);

        assert!((dpr1 - 1.0).abs() < 1e-4, "dpr must be packed as view.scale at 1x: {dpr1}");
        assert!((dpr2 - 2.0).abs() < 1e-4, "dpr must be packed as view.scale at 2x: {dpr2}");
        assert!(res2 > res1 * 1.9, "the node's device-pixel size grows with zoom: {res1} -> {res2}");
        assert_eq!(depth1, depth2, "panel Depth is a constant, not a zoom-dependent value");
        assert_eq!(depth1, 20.0, "the shader's default Depth is the glass default (20)");
        // The bend band, expressed per device pixel, scales with zoom; but divided by the device
        // resolution (i.e. as a fraction of the node) it must be identical — that is what the eye
        // sees, and what makes zooming leave the glass unchanged.
        assert!((rd1 / res1 - rd2 / res2).abs() < 1e-5, "depth px / node width must not depend on zoom: {} vs {}", rd1 / res1, rd2 / res2);
        assert!((bev1 / res1 - bev2 / res2).abs() < 1e-5, "bevel px / node width must not depend on zoom: {} vs {}", bev1 / res1, bev2 / res2);
        assert!(
            (rd1 - 20.0).abs() < 1e-3 && (bev1 - 1.0).abs() < 1e-3,
            "at 1x, Depth 20 -> a 20 px reach and a 1 px band, with no fudge factor: {rd1}, {bev1}"
        );
    }

    /// The bug this fixes: the panel lets Depth go to 100, but a 100-document-unit reach on a small
    /// node folds the two opposite bend bands into each other and samples past the node — the
    /// "Depth = 100 looks broken" failure. `passes_for` must clamp the packed `u.depth` to a share of
    /// the node short side through the same `Glass::depth_reach_doc` the CPU path uses, while
    /// keeping the value in **document units** (so it stays zoom-invariant) and low notches that
    /// already fit byte-identical.
    #[test]
    fn high_depth_on_small_node_is_clamped_in_the_packed_uniform() {
        let f32at = |b: &[u8], off: usize| f32::from_le_bytes(b[off..off + 4].try_into().unwrap());
        let probe = |node_w: f64, node_h: f64, notch: f64, zoom: f64| -> (f32, f32) {
            let mut doc = figcraft_doc::Document::default();
            let page = doc.pages[0];
            let f = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 0.0, 0.0, node_w, node_h).expect("a frame");
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
            let params = [("u.depth".to_string(), notch)].into_iter().collect();
            let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy()).with_params(params);
            doc.get_mut(f).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());
            let view = figcraft_render::View { origin: (0.0, 0.0), scale: zoom, width: 2000, height: 1600 };
            let passes = passes_for(&doc, page, &view, 0.0);
            let p = passes.into_iter().next().expect("one bound frame yields one pass");
            let (_, u) = p.uniforms.into_iter().find(|(n, _)| n == "u").expect("the u uniform");
            // uniform layout: depth@20, dpr@36 (see `glass_depth_is_scale_invariant`).
            (f32at(&u, 20), f32at(&u, 36))
        };

        // Small node, Depth 100: packed depth must be the node-short-side cap, not 100.
        let (depth, _) = probe(80.0, 60.0, 100.0, 1.0);
        let cap = 60.0_f64 * figcraft_doc::Glass::DEPTH_REACH_FRAC;
        assert!((f64::from(depth) - cap).abs() < 1e-3, "Depth 100 on a 60px-short node -> packed u.depth {cap}, got {depth}");
        assert!(depth < 100.0, "a 60px node must never pack a 100px reach");

        // And it stays in document units: same node at 2x packs the same u.depth (dpr carries zoom).
        let (d1, _) = probe(80.0, 60.0, 100.0, 1.0);
        let (d2, _) = probe(80.0, 60.0, 100.0, 2.0);
        assert_eq!(d1, d2, "clamped u.depth must be zoom-invariant (document units)");

        // A big node still fits the whole notch: high Depth is untouched there.
        let (depth_big, _) = probe(600.0, 500.0, 100.0, 1.0);
        assert_eq!(depth_big, 100.0, "on a big node Depth 100 reaches 100 document units unchanged");
    }

    /// The shared conversion `Glass::depth_reach_doc` is the single numeric contract between the CPU
    /// glass and the external WGSL material. These are the properties the source materials actually
    /// establish (monotonic in Depth; a lens cannot outgrow its node; zoom-independent because it
    /// works purely in document units), plus the "low notches untouched" guarantee that keeps the
    /// fix from changing tuned looks. `figcraft-doc`'s own lib-test target is broken at HEAD, so these
    /// live here where both render paths' other tests also run.
    #[test]
    fn depth_reach_is_monotonic_capped_and_leaves_low_notches_untouched() {
        use figcraft_doc::Glass;

        // Every notch that physically fits passes through byte-identically.
        for notch in [1.0, 4.0, 20.0, 40.0] {
            assert_eq!(Glass::depth_reach_doc(notch, 400.0), notch, "notch {notch} must be untouched when it fits");
        }
        assert_eq!(Glass::depth_reach_doc(Glass::default().depth, 400.0), 20.0);

        // The bug case: a high notch on a small node is capped to a share of the node, never 100 px.
        let node = 100.0_f64;
        let cap = node * Glass::DEPTH_REACH_FRAC;
        assert!((Glass::depth_reach_doc(100.0, node) - cap).abs() < 1e-9);
        assert!(Glass::depth_reach_doc(100.0, node) < 100.0);
        assert_eq!(Glass::depth_reach_doc(100.0, 0.0), 1.0, "a degenerate node still keeps a 1-unit reach");

        // Monotonic in the notch, clamped to [1,100], monotonic in node size.
        let mut prev = 0.0;
        for notch in 1..=100 {
            let r = Glass::depth_reach_doc(f64::from(notch), 120.0);
            assert!(r >= prev, "reach must not fall as Depth grows at notch {notch}");
            assert!((1.0..=100.0).contains(&r));
            prev = r;
        }
        let mut prev = 0.0;
        for nd in [20.0, 40.0, 80.0, 160.0, 320.0] {
            let r = Glass::depth_reach_doc(100.0, nd);
            assert!(r >= prev, "bigger nodes allow at least as much reach");
            prev = r;
        }
        assert_eq!(Glass::depth_reach_doc(0.0, 400.0), 1.0);
        assert_eq!(Glass::depth_reach_doc(500.0, 400.0), 100.0);

        // Zoom-invariance is structural: inputs and output are document units only, and the capped
        // reach/node fraction is a constant regardless of the node's pixel size.
        for nd in [40.0_f64, 80.0, 160.0, 400.0] {
            let r = Glass::depth_reach_doc(100.0, nd);
            assert!((r / nd - Glass::DEPTH_REACH_FRAC).abs() < 1e-9, "clamped reach/node is a constant, node={nd}");
        }
    }

    /// A node hanging over the canvas edge must be clamped, not cropped past the texture's end:
    /// an unclamped `copy_texture_to_texture` is a wgpu validation error that panics the render
    /// thread, which is exactly how a bound material can silently never appear.
    #[test]
    fn a_pass_hanging_over_the_canvas_edge_is_clamped() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        // A 100×100 frame at (10,20) rendered into a 64×64 view hangs past both edges.
        let f = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 10.0, 20.0, 100.0, 100.0).expect("a frame");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(f).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());

        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 64, height: 64 };
        let passes = passes_for(&doc, page, &view, 0.0);
        assert_eq!(passes.len(), 1);
        let [x, y, rw, rh] = passes[0].rect_px;
        assert!(x + rw <= 64 && y + rh <= 64, "the pass rect stays inside the canvas: {:?}", passes[0].rect_px);

        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let out = runner.composite(&vec![128u8; 64 * 64 * 4], 64, 64, &passes);
        assert!(out.is_some(), "compositing a clamped pass succeeds instead of panicking");
    }

    /// Real GPU run of the shipped original glass.wgsl through the universal runner: build the
    /// pipeline, feed backdrop + fields + packed uniforms by declared name, draw into the node rect,
    /// and assert pixels change ONLY inside that rect. This is the end-to-end proof that dropping a
    /// `.wgsl` into the folder is enough — the host never mentions "glass".
    #[test]
    fn original_glass_shader_composites_on_real_adapter() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut runner = match WgslRunner::new() {
            Some(r) => r,
            None => {
                eprintln!("no GPU adapter — skipping universal glass pixel test");
                return;
            }
        };
        let (w, h) = (64u32, 64u32);
        // Straight RGBA scene: left half red, right half blue, fully opaque.
        let mut scene = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let c = if x < 32 { [200, 40, 40, 255] } else { [40, 40, 200, 255] };
                scene[i..i + 4].copy_from_slice(&c);
            }
        }
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        // Pack u with the universal exports (resolution = node device-pixel size) + declared params.
        let mut ex = fx::EngineExports::default();
        ex.set("resolution", vec![32.0, 32.0]);
        ex.set("nodeSize", vec![32.0, 32.0]);
        ex.set("nodeSizePx", vec![32.0, 32.0]);
        ex.set("regionOriginPx", vec![0.0, 0.0]);
        ex.set("dpr", vec![1.0]);
        ex.set("time", vec![0.0]);
        let u_bytes = {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            reg.uniform_bytes_for(&eid, "u", &ex).expect("pack u uniform")
        };
        // Node rect = [16,16]..[48,48]; glass refracts/tints only inside.
        let pass = RunPass {
            effect: eid,
            node: NodeId(77),
            rect_px: [16, 16, 32, 32],
            radii_px: [8.0, 8.0, 8.0, 8.0],
            node_px: (32.0, 32.0),
            origin_px: (0.0, 0.0),
            field_bytes: None,
            field_band: f32::INFINITY,
            uniforms: vec![("u".into(), u_bytes)],
            dpr: 1.0,
        };
        let out = runner.composite(&scene, w, h, &[pass]).expect("universal composite on a real adapter");
        assert_eq!(out.len(), scene.len());
        let px = |buf: &[u8], x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let outside = px(&out, 4, 4);
        assert_eq!(outside, [200, 40, 40, 255], "outside the node rect is byte-identical: {outside:?}");
        // The corrected glass (per lizard / Figma `gen_rect_glass`) has bend == 0 across the flat
        // middle, so the exact centre over a uniform patch legitimately stays its backdrop colour.
        // Assert what must hold: the effect visibly changes a meaningful number of pixels inside
        // the node (frost + rim light + dispersion + edge refraction) and changes nothing outside.
        let mut changed = 0usize;
        for y in 16..48 {
            for x in 16..48 {
                if px(&out, x, y) != px(&scene, x, y) {
                    changed += 1;
                }
            }
        }
        assert!(changed > 64, "glass must visibly change the node's pixels (frost/rim/dispersion/refraction): {changed} px changed");
        let mut outside_changed = 0usize;
        for y in 0..h {
            for x in 0..w {
                if (16..48).contains(&x) && (16..48).contains(&y) {
                    continue;
                }
                if px(&out, x, y) != px(&scene, x, y) {
                    outside_changed += 1;
                }
            }
        }
        assert_eq!(outside_changed, 0, "the pass must not touch pixels outside the node rect");
    }

    /// The `fields` input is shape-agnostic: a plain rectangle keeps the analytic field (so its
    /// glass look is byte-for-byte unchanged), while a non-rectangular node — a vector path here,
    /// and by the same path text / ellipse / polygon / boolean — gets a field built from its REAL
    /// outline. Both land in the identical R/GB/A encoding the shader already reads; the maths is
    /// untouched, only the shape input widens.
    #[test]
    fn arbitrary_outlines_get_real_shape_fields() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::VectorNetwork;
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];

        // A triangle vector path, bound to the same material a rectangle would use.
        let net = VectorNetwork::from_points(&[Point::new(20.0, 10.0), Point::new(110.0, 130.0), Point::new(10.0, 130.0)], true);
        let v = doc.create(page, NodeKind::Vector { network: net }, "V", 0.0, 0.0, 120.0, 140.0).expect("a vector");
        let rect = doc.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).expect("a rect");

        // Rectangle → no prebuilt bytes (analytic path, historic behaviour preserved).
        assert!(prepare_fields(&doc, rect, 100, 100, 1.0, (0.0, 0.0)).is_none(), "rectangle keeps the analytic field");

        // Vector → real outline field, correctly sized and with a covered interior + empty corner.
        let (rw, rh) = (120u32, 140u32);
        let bytes = prepare_fields(&doc, v, rw, rh, 1.0, (0.0, 0.0)).expect("a vector gets a real outline field");
        assert_eq!(bytes.len(), (rw * rh * 8) as usize, "field matches the node's device-pixel window");
        let edge_colour = |x: usize, y: usize| f16_at(&bytes, x, y, rw as usize);
        assert!(edge_colour(60, 110)[3] > 0.8, "inside the triangle is covered");
        assert!(edge_colour(6, 6)[3] < 0.05, "outside the triangle (corner) is empty");
        assert!(edge_colour(60, 110)[0] < -5.0, "the interior reports a real negative distance");
    }

    /// The generic field builder describes an ARBITRARY outline in the exact R/GB/A encoding the
    /// material shader already reads — here a triangle, with a covered interior, an empty corner,
    /// rising edge distance and a neutral normal at the flat centre. No shader maths involved.
    #[test]
    fn outline_fields_encode_any_shape() {
        use figcraft_geom::kurbo::BezPath;
        use figcraft_render::{FillRule, shadow::Outline};
        let mut tri = BezPath::new();
        tri.move_to((32.0, 4.0));
        tri.line_to((60.0, 60.0));
        tri.line_to((4.0, 60.0));
        tri.close_path();
        let outline: Outline = (tri, FillRule::Winding);
        let (w, h) = (64u32, 64u32);
        let bytes = figcraft_render::glass::outline_fields_half(&outline, Affine::IDENTITY, w, h, 2).expect("a triangle field");
        assert_eq!(bytes.len(), (w * h * 8) as usize);
        let px = |x: usize, y: usize| f16_at(&bytes, x, y, w as usize);
        let centre = px(32, 40);
        let near_edge = px(32, 10);
        assert!(centre[3] > 0.9, "triangle centre covered: {centre:?}");
        assert!(px(4, 4)[3] < 0.05, "outside the triangle is empty");
        assert!(near_edge[0] > centre[0], "the signed distance rises towards the border: centre R={} edge R={}", centre[0], near_edge[0]);
        assert!(centre[0] < -3.0, "inside is negative: {centre:?}");
        // GB holds an encoded unit normal (0.5*(n+1)): any value is legal, and an interior point
        // must carry a real direction, not the (0.5, 0.5) sentinel used outside the shape.
        let normal = (centre[1] - 0.5, centre[2] - 0.5);
        assert!(normal.0.abs() + normal.1.abs() > 0.1, "interior carries an inward normal: {centre:?}");
    }

    /// **Same numbers, same pane.** The GPU material must look like the engine's own glass (the
    /// built-in effect the reference tool was matched to), because the panel's Depth / Refraction /
    /// Dispersion / Light are that effect's fields. This renders one rectangle twice — once through
    /// `figcraft_render::glass::glass` on the CPU, once through the shipped `glass.wgsl` on the GPU —
    /// over the same striped backdrop, and compares the composited pixels.
    ///
    /// This is the regression test for the whole class of "the panel says 20 but the material does
    /// something else" bugs: a wrong Depth coefficient, a reversed `en` easing, a `pow(6)` highlight
    /// or a missing sheen term all move these numbers well outside the tolerance.
    #[test]
    fn the_material_looks_like_the_engine_glass() {
        use figcraft_geom::kurbo::BezPath;
        use figcraft_render::backdrop::EffectFrame;
        use figcraft_render::{FillRule, Pixmap};
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping the CPU/GPU glass comparison");
            return;
        };
        let (w, h) = (96u32, 96u32);
        let pad = 16u32;

        // A striped backdrop, so a bend shows up as a shifted stripe instead of a flat wash.
        let mut back = Pixmap::new(w + 2 * pad, h + 2 * pad).expect("backdrop pixmap");
        {
            let (bw, bh) = (back.width() as usize, back.height() as usize);
            let d = back.data_mut();
            for y in 0..bh {
                for x in 0..bw {
                    let i = (y * bw + x) * 4;
                    let c = if (x / 6) % 2 == 0 { [230, 60, 60, 255] } else { [40, 90, 220, 255] };
                    d[i..i + 4].copy_from_slice(&c);
                }
            }
        }
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((f64::from(w), 0.0));
        path.line_to((f64::from(w), f64::from(h)));
        path.line_to((0.0, f64::from(h)));
        path.close_path();
        let outline = (path, FillRule::Winding);
        // The same numbers in both: Depth 20 document px is a 20 px reach and the renderer's fixed
        // 1 px band — the built-in effect and the material must agree on what a value *means*.
        let g =
            figcraft_doc::Glass { light_angle: 135.0, light_intensity: 0.0, refraction: 0.4, depth: 20.0, dispersion: 0.25, frost: 8.0, splay: 0.0 };
        let frame = EffectFrame { shape: &outline, back: &back, pad, size: (w, h), xf: Affine::IDENTITY, scale: 1.0 };
        let cpu = figcraft_render::glass::glass(&g, frame, None).expect("the CPU glass renders");

        // The GPU sees the same backdrop without the padding, as the runner's crop does.
        let mut scene = vec![0u8; (w * h * 4) as usize];
        {
            let src = back.data();
            let bw = back.width() as usize;
            let p = pad as usize;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let si = ((y + p) * bw + x + p) * 4;
                    let di = (y * w as usize + x) * 4;
                    scene[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
        let path_on_disk = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path_on_disk.to_string_lossy());
        let u_bytes = {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            // Override on a private clone, never the shared registry — otherwise these params leak
            // into every later test (see the note in `glass_pass`).
            let mut local = reg.clone();
            for (field, value) in [
                ("u.light_angle", 135.0),
                ("u.light_int", 0.0),
                ("u.refraction", 0.4),
                ("u.depth", 20.0),
                ("u.dispersion", 0.25),
                ("u.frost", 8.0),
                ("u.splay", 0.0),
            ] {
                let _ = local.set_param(&eid, field, value);
            }
            let mut ex = fx::EngineExports::default();
            ex.set("resolution", vec![f64::from(w), f64::from(h)]);
            ex.set("dpr", vec![1.0]);
            ex.set("nodeSizePx", vec![f64::from(w), f64::from(h)]);
            ex.set("regionOriginPx", vec![0.0, 0.0]);
            local.uniform_bytes_for(&eid, "u", &ex).expect("pack the u uniform")
        };
        let pass = RunPass {
            effect: eid,
            node: NodeId(7),
            rect_px: [0, 0, w, h],
            radii_px: [0.0; 4],
            node_px: (w as f32, h as f32),
            origin_px: (0.0, 0.0),
            field_bytes: None,
            field_band: f32::INFINITY,
            uniforms: vec![("u".into(), u_bytes)],
            dpr: 1.0,
        };
        let gpu = runner.composite(&scene, w, h, &[pass]).expect("the GPU material composites");

        // Compare like with like: the CPU pixmap is premultiplied, the runner composites straight
        // alpha over the scene.
        let cpu_px = |x: usize, y: usize| -> [f32; 4] {
            let i = (y * w as usize + x) * 4;
            let d = cpu.data();
            [f32::from(d[i]), f32::from(d[i + 1]), f32::from(d[i + 2]), f32::from(d[i + 3])]
        };
        let scene_px = |x: usize, y: usize| -> [f32; 4] {
            let i = (y * w as usize + x) * 4;
            std::array::from_fn(|k| f32::from(scene[i + k]))
        };
        let gpu_px = |x: usize, y: usize| -> [f32; 4] {
            let i = (y * w as usize + x) * 4;
            std::array::from_fn(|k| f32::from(gpu[i + k]))
        };
        let mut diffs: Vec<f32> = Vec::new();
        let mut inner: Vec<f32> = Vec::new();
        let mut band: Vec<f32> = Vec::new();
        let mut worst = (0usize, 0usize, 0.0f32);
        // Skip a 2 px frame (antialiasing models differ at the silhouette) and compare the interior.
        let fields = fields_for_rect(w, h, (w as f32, h as f32), (0.0, 0.0), [0.0; 4]);
        for y in 2..h as usize - 2 {
            for x in 2..w as usize - 2 {
                let c = cpu_px(x, y);
                let s = scene_px(x, y);
                let a = c[3] / 255.0;
                let want: Vec<f32> = (0..3).map(|k| c[k] + s[k] * (1.0 - a)).collect();
                let got = gpu_px(x, y);
                let d = (0..3).map(|k| (got[k] - want[k]).abs()).fold(0.0f32, f32::max);
                if d > worst.2 {
                    worst = (x, y, d);
                }
                // Past the lens curve nothing bends and no light falls: those pixels must match.
                // Here depth=20, bevel=1 at dpr=1, so the curve ends ~21 px in; 31 is a deliberately
                // conservative bound (well past the curve) so these are unquestionably flat-interior.
                let inside = -f16_at(&fields, x, y, w as usize)[0];
                let near_corner = (x < 32 || x + 32 > w as usize) && (y < 32 || y + 32 > h as usize);
                if inside > 31.0 {
                    inner.push(d);
                } else if !near_corner {
                    band.push(d);
                }
                diffs.push(d);
            }
        }
        assert!(!diffs.is_empty());
        let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
        let imean = inner.iter().sum::<f32>() / inner.len().max(1) as f32;
        let imax = inner.iter().copied().fold(0.0f32, f32::max);
        let bmean = band.iter().sum::<f32>() / band.len().max(1) as f32;
        let bmax = band.iter().copied().fold(0.0f32, f32::max);
        let mut sorted = diffs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = sorted[sorted.len() * 95 / 100];
        eprintln!(
            "CPU vs GPU glass: all mean {mean:.2}, p95 {p95:.2}, worst {:.2} at ({}, {}); flat interior mean {imean:.2} max {imax:.2} ({} px)",
            worst.2,
            worst.0,
            worst.1,
            inner.len()
        );
        eprintln!("  straight-edge band (corners excluded): mean {bmean:.2} max {bmax:.2} ({} px)", band.len());
        // The flat interior (past the curve) has to match: nothing there bends or lights, so the
        // only content is the frosted backdrop. Both sides run Figma's downsample → upsample chain
        // (the shader through the backdrop's mip levels, the CPU through `frosted_backdrop`), and
        // both dither by 2/255 — but their dither hashes are indexed from different origins, so a
        // level or two of per-pixel difference is the dither, not the blur.
        assert!(imean < 2.0, "the flat interior matches the engine glass: {imean:.2} (max {imax:.2})");
        // Inside the curve, on the straight edges (where a corner's core direction plays no part),
        // the bend, the light and the dispersion agree to well under one level on average. The
        // corner pixels are the only ones left out: the CPU points its bend at the nearest pixel of
        // the shape's core (a diagonal near a corner), the shader at the distance field's gradient;
        // a 1 px difference in where the stripes land is the whole of that residual.
        assert!(bmean < 4.0, "straight-edge band within 4 levels: {bmean:.2} (max {bmax:.2}, {} px)", band.len());
        assert!(p95 < 60.0, "95% of pixels are within 60 levels: {p95:.2} (worst {:.2} at {}, {})", worst.2, worst.0, worst.1);
        let _ = mean;
    }

    /// Frost is Figma's **downsample → upsample chain**, not a blur kernel: a radius of `2^L` px is
    /// level `L` of the reduction. Three things follow and are checked here — a radius of 0 leaves
    /// the backdrop untouched, a bigger radius spreads a 1 px feature monotonically wider (a
    /// Gaussian would too, but it is not what Figma runs), and the composite dithers by `2/255`, so
    /// a flat backdrop picks up ±1 level instead of staying dead flat. That dither is the part the
    /// earlier stand-in was missing entirely.
    #[test]
    fn frost_reduces_by_halving_and_dithers_like_figma() {
        use figcraft_render::Pixmap;
        let (w, h) = (64u32, 64u32);
        // A white field with one 1 px black line: how far that line spreads *is* the blur radius.
        let mut p = Pixmap::new(w, h).expect("pixmap");
        {
            let d = p.data_mut();
            d.fill(255);
            for y in 0..h as usize {
                let i = (y * w as usize + 32) * 4;
                d[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        // How dark the line still is on the middle row (255 = fully spread out, 0 = untouched).
        let core = |q: &Pixmap| -> f64 {
            let d = q.data();
            f64::from(d[(32 * w as usize + 32) * 4])
        };
        assert!(core(&p) < 2.0, "the line starts black");
        let none = figcraft_render::glass::frosted_backdrop(&p, 0.0);
        assert_eq!(none.data(), p.data(), "a radius of 0 leaves the backdrop alone");
        let small = figcraft_render::glass::frosted_backdrop(&p, 4.0);
        let big = figcraft_render::glass::frosted_backdrop(&p, 16.0);
        assert!(core(&small) > 40.0, "radius 4 spreads the line: {}", core(&small));
        assert!(core(&big) > core(&small) + 30.0, "radius 16 spreads it further: {} vs {}", core(&big), core(&small));

        // The dither: a flat backdrop must not come out flat — Figma adds ±2/255 to break the
        // banding the reduction leaves, and the amplitude must be that small, not visible noise.
        let mut flat = Pixmap::new(32, 32).expect("flat");
        flat.data_mut().fill(128);
        let dithered = figcraft_render::glass::frosted_backdrop(&flat, 8.0);
        let levels: std::collections::BTreeSet<u8> = dithered.data().chunks(4).map(|p| p[0]).collect();
        assert!(levels.len() > 1, "the dither must vary the flat field: {levels:?}");
        assert!(levels.iter().all(|v| v.abs_diff(128) <= 2), "and only by 2/255: {levels:?}");
    }

    /// The seven controls must be **Figma's own Glass effect fields**, because that is the only
    /// thing the two panes can legitimately share. The authority is the Plugin API
    /// (`Effect → GlassEffect`): `lightIntensity`, `refraction` and `dispersion` are ratios 0–1,
    /// `depth` is "must be >= 1" (Figma's panel stops at 100), `lightAngle` is degrees, and frost is
    /// a `radius`, not a 0–1 amount. Figma's own help example uses 66°, 60 %, Refraction 76,
    /// Depth 98, Dispersion 100, Frost 0, Splay 100 — which only type-checks under this contract.
    ///
    /// Getting it wrong is not cosmetic: it decides whether 20 means "20 px of lens" or "a 0–1
    /// slider's 20th step", and the two look nothing alike.
    #[test]
    fn glass_panel_matches_the_figma_slider_table() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        let reg = fx::shared();
        let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
        let params = reg.params(&eid);
        assert_eq!(params.len(), 7, "the panel has seven sliders: {params:?}");
        let row = |field: &str| params.iter().find(|p| p.field == field).unwrap_or_else(|| panic!("missing {field}"));
        // (field, min, max, step, default) — the engine's `Glass` model, which *is* Figma's API
        // shape (`Glass::BOUND_FIELDS` lists the API names: lightAngle, lightIntensity, refraction,
        // depth, dispersion, radius, splay). Depth and frost are lengths in document px.
        for (field, min, max, step, default) in [
            ("u.light_angle", -180.0, 180.0, 1.0, -45.0),
            ("u.light_int", 0.0, 1.0, 0.01, 0.8),
            ("u.refraction", 0.0, 1.0, 0.01, 0.8),
            ("u.depth", 1.0, 100.0, 1.0, 20.0),
            ("u.dispersion", 0.0, 1.0, 0.01, 0.5),
            ("u.frost", 0.0, 100.0, 1.0, 4.0),
            ("u.splay", 0.0, 1.0, 0.01, 0.0),
        ] {
            let p = row(field);
            assert!((p.min - min).abs() < 1e-9, "{field} min {} != {min}", p.min);
            assert!((p.max - max).abs() < 1e-9, "{field} max {} != {max}", p.max);
            assert!((p.step - step).abs() < 1e-9, "{field} step {} != {step}", p.step);
            assert!((p.default - default).abs() < 1e-9, "{field} default {} != {default}", p.default);
            assert!(!p.auto, "{field} must come from an @fx param declaration, not auto-discovery");
        }
    }

    /// End to end through `passes_for`: a bound non-rectangular node reaches the compositor with
    /// its real-outline field bytes already prepared, while a rectangle carries none (analytic).
    /// This is the seam between "the document has a text/path/ellipse with glass" and "the GPU
    /// gets geometry it can actually shade".
    #[test]
    fn passes_for_carries_real_outline_fields_for_shapes() {
        use figcraft_doc::NodeKind;
        use figcraft_doc::VectorNetwork;
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let bind = |d: &mut figcraft_doc::Document, id| {
            let b = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
            d.get_mut(id).expect("node").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), b.to_json());
        };
        let net = VectorNetwork::from_points(&[Point::new(20.0, 10.0), Point::new(110.0, 130.0), Point::new(10.0, 130.0)], true);
        let v = doc.create(page, NodeKind::Vector { network: net }, "V", 0.0, 0.0, 120.0, 140.0).expect("a vector");
        let rect = doc.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).expect("a rect");
        bind(&mut doc, v);
        bind(&mut doc, rect);

        let view = figcraft_render::View { origin: (-20.0, -20.0), scale: 2.0, width: 640, height: 480 };
        let passes = passes_for(&doc, page, &view, 0.0);
        assert_eq!(passes.len(), 2, "both bound nodes yield a pass");
        for p in &passes {
            if p.node == v {
                let bytes = p.field_bytes.as_ref().expect("a vector carries real outline fields");
                assert_eq!(bytes.len(), (p.rect_px[2] * p.rect_px[3] * 8) as usize, "field sized to the device-pixel rect");
            } else if p.node == rect {
                assert!(p.field_bytes.is_none(), "a rectangle keeps the analytic field (zero regression)");
            }
        }
    }
    /// The window must cover **whole** pixels. Truncating it (`as u32`) left the texture up to a
    /// pixel narrower than the node while the shader normalised its uv to that texture, so the
    /// backdrop was rescaled by up to 1/width — a different factor every frame of a zoom. That is the
    /// shimmer seen inside a container while scaling it. `floor`/`ceil` makes uv and screen pixels a
    /// strict 1:1 map again.
    #[test]
    fn the_window_covers_whole_pixels_at_any_zoom() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let _f = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 10.0, 20.0, 100.0, 100.0).expect("a frame");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let binding = crate::wgsl_material::WgslMaterialBinding::new(path.to_string_lossy());
        doc.get_mut(_f).expect("the frame").plugin_data.insert(crate::wgsl_material::BINDING_KEY.into(), binding.to_json());

        // A 100-unit node at 1.37× starts at screen 13.7 and ends at 150.7: the window must be
        // [13, 151), i.e. 138 px — not [13, 150) = 137 px.
        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.37, width: 400, height: 400 };
        let passes = passes_for(&doc, page, &view, 0.0);
        assert_eq!(passes.len(), 1);
        let [x, y, w, h] = passes[0].rect_px;
        assert_eq!((x, y), (13, 27), "the window starts on the whole pixel below the node");
        assert_eq!(w, 138, "the window covers whole pixels: ceil(150.7) - floor(13.7)");
        assert_eq!(h, 138);
        // The sub-pixel part lives on in `origin_px`, so the distance field stays continuous.
        assert!((passes[0].origin_px.0 - (13.0 - 13.7)).abs() < 1e-3);
        assert!((passes[0].node_px.0 - 137.0).abs() < 1e-2);
    }

    /// A material stacked on another must see the *finished* lower one. The old compositor cropped
    /// every pass out of the same raw CPU image, so a second pane ignored the first — and a pane's
    /// own children were pasted over the whole stack at the very end, ignoring z order entirely.
    ///
    /// Here the canvas is black and a red `Over` step paints the pass's window before the pass runs:
    /// if the glass reads the composite so far, its interior is red; if it read the raw canvas, the
    /// interior would stay black.
    #[test]
    fn a_material_reads_the_layers_already_composited_below_it() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping layered composite test");
            return;
        };
        let (w, h) = (32u32, 32u32);
        let canvas = vec![0u8; (w * h * 4) as usize];
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        let mut ex = fx::EngineExports::default();
        ex.set("resolution", vec![32.0, 32.0]);
        ex.set("nodeSize", vec![32.0, 32.0]);
        ex.set("nodeSizePx", vec![32.0, 32.0]);
        ex.set("regionOriginPx", vec![0.0, 0.0]);
        ex.set("dpr", vec![1.0]);
        ex.set("time", vec![0.0]);
        let uniforms = {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            vec![("u".to_string(), reg.uniform_bytes_for(&eid, "u", &ex).expect("pack u"))]
        };
        // Frost 0 so the interior is the backdrop itself, not a blur of it.
        let pass = RunPass {
            effect: eid, node: NodeId(7), rect_px: [0, 0, 32, 32],
            radii_px: [0.0; 4], node_px: (32.0, 32.0), origin_px: (0.0, 0.0),
            field_bytes: None, field_band: f32::INFINITY, uniforms,
            dpr: 1.0,
        };
        // Straight RGBA (the overlay's format): opaque red everywhere.
        let red = vec![255u8, 0, 0, 255].repeat((w * h) as usize);
        let steps = [Step::Over { pixels: &red, rect: [0, 0, w, h] }, Step::Pass(&pass)];
        let out = runner.composite_steps(&canvas, w, h, &steps, None).expect("composite");
        // Read back as BGRA: red = channel 2.
        let i = ((16 * w + 16) * 4) as usize;
        assert!(out[i + 2] > 100, "the glass interior shows the layer below it: {:?}", &out[i..i + 4]);
    }

    /// Per-frame cost of the composite path, printed so a regression has a number next to it.
    /// `cargo test -p figcraft-ui-gpui --lib --features gpu,test-support the_composite_path -- --nocapture`
    #[test]
    fn the_composite_path_stays_cheap() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping timing");
            return;
        };
        let (w, h) = (1920u32, 1080u32);
        let mut scene = vec![0u8; (w * h * 4) as usize];
        for (i, px) in scene.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i % 251) as u8, (i % 97) as u8, (i % 53) as u8, 255]);
        }
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        let mut ex = fx::EngineExports::default();
        ex.set("resolution", vec![f64::from(w), f64::from(h)]);
        ex.set("nodeSize", vec![f64::from(w), f64::from(h)]);
        ex.set("nodeSizePx", vec![f64::from(w), f64::from(h)]);
        ex.set("regionOriginPx", vec![0.0, 0.0]);
        ex.set("dpr", vec![1.0]);
        ex.set("time", vec![0.0]);
        let uniforms = {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            vec![("u".to_string(), reg.uniform_bytes_for(&eid, "u", &ex).expect("pack u"))]
        };
        let pass = RunPass {
            effect: eid, node: NodeId(9), rect_px: [0, 0, w, h],
            radii_px: [24.0; 4], node_px: (w as f32, h as f32), origin_px: (0.0, 0.0),
            field_bytes: None, field_band: f32::INFINITY, uniforms,
            dpr: 1.0,
        };
        let steps = [Step::Pass(&pass)];
        // How long the *host* side of one 1080p field costs: this is the part a zoom re-pays every
        // frame, because the window origin moves continuously.
        let t_sdf = std::time::Instant::now();
        let _ = fields_for_rect(w, h, (w as f32, h as f32), (0.0, 0.0), [24.0; 4]);
        let exact = t_sdf.elapsed().as_secs_f64() * 1000.0;
        let t_band = std::time::Instant::now();
        let _ = fields_for_rect_banded(w, h, (w as f32, h as f32), (0.0, 0.0), [24.0; 4], 30.0);
        let banded = t_band.elapsed().as_secs_f64() * 1000.0;
        eprintln!("1080p fields: exact {exact:.2} ms, banded(30px) {banded:.2} ms");
        // Warm up: upload, compile, allocate.
        let _ = runner.composite_steps(&scene, w, h, &steps, None);
        let n = 12;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let _ = runner.composite_steps(&scene, w, h, &steps, None).expect("composite");
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        eprintln!("1920x1080 full-window glass composite: {per:.2} ms/frame (upload + SDF(free) + skyline + readback)");
        assert!(per < 200.0, "a full-window pass must not cost hundreds of milliseconds: {per} ms");
    }

    /// Two child layers in one frame must stay distinct. `queue.write_texture` is enqueued ahead of
    /// the whole command buffer, so a shared staging texture made the last upload win: every layer
    /// drew the same pixels. Each step gets its own texture instead.
    #[test]
    fn each_child_layer_keeps_its_own_pixels() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping overlay staging test");
            return;
        };
        let (w, h) = (16u32, 8u32);
        let canvas = vec![0u8; (w * h * 4) as usize];
        // Each layer is its own box (8×8, stride 8) — the `Step::Over` contract.
        let red: Vec<u8> = [255u8, 0, 0, 255].repeat(8 * 8);
        let green: Vec<u8> = [0u8, 255, 0, 255].repeat(8 * 8);
        let steps = [Step::Over { pixels: &red, rect: [0, 0, 8, h] }, Step::Over { pixels: &green, rect: [8, 0, 8, h] }];
        let out = runner.composite_steps(&canvas, w, h, &steps, None).expect("composite");
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y * w + x) * 4) as usize;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        let l = at(3, 4);
        let r = at(12, 4);
        assert!(l[2] > 200 && l[1] < 40, "left layer is red (BGRA): {l:?}");
        assert!(r[1] > 200 && r[2] < 40, "right layer is green (BGRA): {r:?}");
    }

    /// The banded field must be the exact field wherever a material can still see it. Inside the
    /// band the numbers are identical; outside, the only lie told is "this is far", which is what
    /// every shader's own early-out already assumes (`d >= depth + bevel`).
    #[test]
    fn the_banded_field_is_exact_within_reach() {
        let (w, h) = (120u32, 120u32);
        let (node, origin, radii) = ((120.0f32, 120.0f32), (0.0f32, 0.0f32), [24.0f32; 4]);
        let exact = fields_for_rect(w, h, node, origin, radii);
        let banded = fields_for_rect_banded(w, h, node, origin, radii, 30.0);
        assert_eq!(exact.len(), banded.len());
        let mut checked = 0;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let a = f16_at(&exact, x, y, w as usize);
                let b = f16_at(&banded, x, y, w as usize);
                if a[0].abs() <= 28.0 {
                    // Within the band the two must agree on distance, normal and coverage.
                    for c in 0..4 {
                        assert!((a[c] - b[c]).abs() < 2e-3, "banded differs inside the band at ({x},{y}) channel {c}: {:?} vs {:?}", a, b);
                    }
                    checked += 1;
                } else if a[0] > 31.0 {
                    assert_eq!(b[3], 0.0, "far outside must have no coverage at ({x},{y})");
                } else if a[0] < -31.0 {
                    assert_eq!(b[3], 1.0, "deep inside must be fully covered at ({x},{y})");
                }
            }
        }
        assert!(checked > 2000, "the band really was exercised: {checked} px");
    }

    /// A children layer arrives as **its own box**, not as a canvas-sized buffer. The compositor
    /// used to require a full-canvas length and `continue`d when it did not match — so every
    /// container's contents were silently dropped, with no error anywhere. This pins the contract:
    /// an 8×8 layer with an 8-pixel stride must land at its rect, and nowhere else.
    #[test]
    fn a_children_layer_lands_where_its_rect_says() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut runner) = WgslRunner::new() else {
            eprintln!("no GPU adapter — skipping overlay stride test");
            return;
        };
        let (w, h) = (64u32, 64u32);
        let canvas = vec![0u8; (w * h * 4) as usize];
        let block: Vec<u8> = [255u8, 0, 0, 255].repeat(8 * 8);
        let steps = [Step::Over { pixels: &block, rect: [20, 10, 8, 8] }];
        let out = runner.composite_steps(&canvas, w, h, &steps, None).expect("composite");
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y * w + x) * 4) as usize;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert!(at(24, 14)[2] > 200, "the layer is drawn at its rect: {:?}", at(24, 14));
        assert!(at(10, 10)[2] < 40, "and not at the canvas origin: {:?}", at(10, 10));
        assert!(at(30, 14)[3] == 0 || at(30, 14)[2] < 40, "and not smeared to the right: {:?}", at(30, 14));
    }

    /// A real glass pass over a striped scene, shared by the async-readback tests. Packs the same
    /// `u` uniform the shipped `glass.wgsl` declares, so these tests exercise the actual material
    /// path (not just a bare copy).
    fn glass_pass(w: u32, h: u32) -> RunPass {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/wgsl/glass.wgsl");
        let eid = crate::wgsl_material::ensure_loaded(&path.to_string_lossy());
        let u_bytes = {
            let reg = fx::shared();
            let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
            // Clone before overriding: `set_param` writes the **current** value into the shared
            // registry, and `passes_for` later reads that current value as the material's depth
            // reach. Writing `u.depth = 16` in place here leaked into every later test and made
            // `glass_depth_is_scale_invariant` read 16 instead of the shader's default 20 when the
            // whole crate ran single-threaded. Mirror `passes_for` (line ~165) and override on a
            // private clone so this helper's params never escape its own scope.
            let mut local = reg.clone();
            for (field, value) in [
                ("u.light_angle", 135.0),
                ("u.light_int", 0.0),
                ("u.refraction", 0.4),
                ("u.depth", 16.0),
                ("u.dispersion", 0.25),
                ("u.frost", 6.0),
                ("u.splay", 0.0),
            ] {
                let _ = local.set_param(&eid, field, value);
            }
            let mut ex = fx::EngineExports::default();
            ex.set("resolution", vec![f64::from(w), f64::from(h)]);
            ex.set("dpr", vec![1.0]);
            ex.set("nodeSizePx", vec![f64::from(w), f64::from(h)]);
            ex.set("regionOriginPx", vec![0.0, 0.0]);
            local.uniform_bytes_for(&eid, "u", &ex).expect("pack u uniform")
        };
        RunPass {
            effect: eid,
            node: NodeId(7),
            rect_px: [8, 8, w - 16, h - 16],
            radii_px: [10.0; 4],
            node_px: ((w - 16) as f32, (h - 16) as f32),
            origin_px: (0.0, 0.0),
            field_bytes: None,
            field_band: f32::INFINITY,
            uniforms: vec![("u".into(), u_bytes)],
            dpr: 1.0,
        }
    }

    fn striped_scene(w: u32, h: u32) -> Vec<u8> {
        let mut scene = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let c = if (x / 6) % 2 == 0 { [210, 55, 55, 255] } else { [45, 80, 210, 255] };
                scene[i..i + 4].copy_from_slice(&c);
            }
        }
        scene
    }

    /// The async two-phase path must be **byte-for-byte identical** to the synchronous one for the
    /// same (canvas, steps). This is the load-bearing correctness proof: the only thing that changed
    /// is *when* the pixels are collected, never what is drawn or how the region is spliced.
    #[test]
    fn async_readback_is_byte_identical_to_sync() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut runner = match WgslRunner::new() {
            Some(r) => r,
            None => {
                eprintln!("no GPU adapter — skipping async/sync equivalence");
                return;
            }
        };
        let (w, h) = (64u32, 64u32);
        let canvas = striped_scene(w, h);
        let pass = glass_pass(w, h);
        let steps = [Step::Pass(&pass)];

        let sync = runner.composite_steps(&canvas, w, h, &steps, None).expect("sync composite");
        let id = runner.submit_async(&canvas, w, h, &steps, None).expect("async submit");
        // Collect with a bounded pump loop — never spin forever if a broken driver never fires.
        let mut async_out = None;
        for _ in 0..2000 {
            if let Some(out) = runner.poll_ready(id, &canvas) {
                async_out = Some(out);
                break;
            }
            std::thread::yield_now();
        }
        let async_out = async_out.expect("async map callback fires and pixels are collected");
        assert_eq!(async_out.len(), sync.len(), "same image size");
        let mut diff = 0usize;
        for (a, b) in sync.iter().zip(async_out.iter()) {
            if a != b {
                diff += 1;
            }
        }
        assert_eq!(diff, 0, "async path must be pixel-for-pixel identical to sync: {diff} bytes differ");
        assert_eq!(runner.in_flight(), 0, "collected frame releases its slot");
    }

    /// `submit_async` must return immediately (no poll/Wait, no recv) and hold exactly one frame in
    /// flight; `poll_ready` on a frame that has not completed returns `None` rather than blocking;
    /// collecting twice yields `None` the second time.
    #[test]
    fn async_submit_does_not_block_and_poll_is_idempotent() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut runner = match WgslRunner::new() {
            Some(r) => r,
            None => {
                eprintln!("no GPU adapter — skipping non-blocking test");
                return;
            }
        };
        let (w, h) = (48u32, 48u32);
        let canvas = striped_scene(w, h);
        let pass = glass_pass(w, h);
        let steps = [Step::Pass(&pass)];

        let t = std::time::Instant::now();
        let id = runner.submit_async(&canvas, w, h, &steps, None).expect("async submit");
        let submit_ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(submit_ms < 50.0, "submit_async returns without waiting for the GPU: {submit_ms:.1}ms");
        assert_eq!(runner.in_flight(), 1, "the submitted frame owns one slot");

        let mut collected = None;
        for _ in 0..2000 {
            if let Some(out) = runner.poll_ready(id, &canvas) {
                collected = Some(out);
                break;
            }
            std::thread::yield_now();
        }
        assert!(collected.is_some(), "the frame is eventually collected");
        assert!(runner.poll_ready(id, &canvas).is_none(), "collecting the same frame twice yields None");
        assert_eq!(runner.in_flight(), 0);
    }

    /// With ASYNC_SLOTS frames in flight, the next submit is refused (back-pressure by dropping the
    /// newest frame, never blocking). Each collected frame keeps **its own** base for the pixels
    /// outside the region, so two frames with different canvases cannot bleed into each other.
    #[test]
    fn async_ring_backpressures_and_does_not_cross_frames() {
        let _gpu = GPU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut runner = match WgslRunner::new() {
            Some(r) => r,
            None => {
                eprintln!("no GPU adapter — skipping async ring test");
                return;
            }
        };
        let (w, h) = (32u32, 32u32);
        // A tiny Over layer so the region is small and several frames fit comfortably; the point is
        // slot bookkeeping, not the glass math (covered by the equivalence test above).
        let block: Vec<u8> = [255u8, 0, 0, 255].repeat(4 * 4);
        let steps = [Step::Over { pixels: &block, rect: [4, 4, 4, 4] }];

        let mut ids = Vec::new();
        let mut bases: Vec<Vec<u8>> = Vec::new();
        for k in 0..ASYNC_SLOTS {
            let mut base = vec![0u8; (w * h * 4) as usize];
            // A distinct flat colour per frame, visible everywhere outside the Over region.
            for px in base.chunks_exact_mut(4) {
                px.copy_from_slice(&[k as u8 * 40, 10, 200, 255]);
            }
            let id = runner.submit_async(&base, w, h, &steps, None).expect("a free slot exists");
            ids.push(id);
            bases.push(base);
        }
        assert_eq!(runner.in_flight(), ASYNC_SLOTS, "all slots occupied");
        let extra = vec![7u8; (w * h * 4) as usize];
        assert!(runner.submit_async(&extra, w, h, &steps, None).is_none(), "full ring refuses a new frame instead of blocking or evicting");

        // Collect every frame and confirm its outside-region pixels come from *its* base.
        for (id, base) in ids.into_iter().zip(bases.into_iter()) {
            let mut out = None;
            for _ in 0..2000 {
                if let Some(o) = runner.poll_ready(id, &base) {
                    out = Some(o);
                    break;
                }
                std::thread::yield_now();
            }
            let out = out.expect("frame collected");
            let want = [base[0], base[1], base[2], base[3]];
            let i = ((h - 1) * w + (w - 1)) * 4; // far from the [4,4,4,4] region
            assert_eq!(
                [out[i as usize], out[i as usize + 1], out[i as usize + 2], out[i as usize + 3]],
                want,
                "frame {id} keeps its own base outside the region"
            );
        }
        assert_eq!(runner.in_flight(), 0, "draining the ring frees every slot");
    }
}
