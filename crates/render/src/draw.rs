//! Recursive scene drawing.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_color::{BlendMode, Paint, PaintKind};
use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind, StrokeAlign};
use figcraft_geom::{Affine, BezPath, Rect, Shape};
use tiny_skia::{FillRule, FilterQuality, IntRect, Mask, MaskType, Pixmap, PixmapPaint, Transform};

use crate::backdrop::{self, Under};
use crate::boolean::{PathCache, locked};
use crate::paint::{Prepared, composite, prepare_at, ts_blend};
use crate::{blur, grain, lod, shadow};

/// Shared state for one render.
#[derive(Clone, Copy)]
/// `'d`: the document's (layers looked up for the render are kept that long), `'a`: the rest.
pub struct Ctx<'a, 'd> {
    pub doc: &'d Document,
    /// Device pixels per document unit (for effect radii).
    pub scale: f64,
    pub outline_mode: bool,
    /// What outline mode draws besides shapes, and its line colour.
    pub outline: crate::Outline,
    /// Layers drawn in outline mode with their contents (outlines for the selection).
    pub outlined: &'a [NodeId],
    /// Nodes not to draw (e.g. text being edited draws through the editor).
    pub skip: &'a [NodeId],
    /// Draw **only** these layers, walking down from the page as usual so their ancestors still
    /// clip, fade and isolate them. Empty: draw the page. (The canvas draws a layer being dragged
    /// itself; this is how it gets that layer alone, clipped the way it sits.)
    pub only: &'a [NodeId],
    /// Approximate what's too small to see in detail (canvas previews; see [`crate::lod`]).
    pub lod: bool,
    /// A canvas preview at any zoom ([`crate::Options::lod`]): large images shown smaller than
    /// their pixels draw from smaller copies (see [`crate::images::shown`]).
    pub preview: bool,
    pub magnified_image_pixels: bool,
    /// The surfaces under the one being drawn into, when it is an isolated layer (for backdrops).
    pub under: Option<&'a Under<'a>>,
    /// Seconds into animated GIFs (0: their first frame; only the presenter plays them).
    pub time: f64,
    /// Paint bounds already worked out in this render.
    pub bounds: &'a Bounds<'d>,
    /// 画布提示带（padding / spacing）：铺在所属图层的填充之后、子层之前（见 [`crate::Band`]）。
    pub bands: &'a [crate::Band],
    /// 外置 WGSL 材质板（`Some` 时，绑了材质的图层那层的底图由 GPU 产出，见 [`crate::shade`]）。
    pub shade: Option<&'a crate::shade::ShadeBoard>,
    /// 这次渲染**真正负责**的设备像素行范围（分带渲染时是带自己那段；`None` 是整幅）。
    ///
    /// 每带会多渲染一圈 overlap 好让模糊/材质读到自己那层之外的底图，这圈像素最后会被裁掉。
    /// 材质节点本身由「装得下它」的那一条带整块算一次（见 `crate::band_spans`），所以按行范围
    /// 归属就不必在相邻带的 overlap 里再算一遍——否则同一节点会白跑 N 次 GPU pass。
    pub rows: Option<(f64, f64)>,
    /// 页面 → 设备像素（提示带是页面坐标，直接用它画）。
    pub page_xf: Affine,
}

/// What one render works out about layers with children, which a walk down their subtree finds:
/// their local paint bounds by greeking scale, and whether they can be splatted (see
/// [`lod::splattable`]). Culling asks for a node's and then for each of its children's, so without
/// them every layer's subtree was walked again at each level above it. Each answer keeps how many
/// levels its walk went down.
/// It also keeps the layers it looked up: the walks for culling, splatting and drawing go down the
/// same layers, and in a large document a lookup is a good part of a layer's cost.
#[derive(Default)]
pub struct Bounds<'a> {
    paint: std::cell::RefCell<IdMap<BoundsKey, (Rect, usize)>>,
    pub(crate) splat: std::cell::RefCell<IdMap<NodeId, (bool, usize)>>,
    nodes: std::cell::RefCell<IdMap<NodeId, Option<&'a Node>>>,
}

impl<'a> Bounds<'a> {
    /// Node `id` of `doc`, looked up once for the render.
    pub(crate) fn node(&self, doc: &'a Document, id: NodeId) -> Option<&'a Node> {
        if let Some(n) = self.nodes.borrow().get(&id) {
            return *n;
        }
        let n = doc.node(id);
        self.nodes.borrow_mut().insert(id, n);
        n
    }
}

/// A hash map keyed by node ids, which need no SipHash: they are counters (or a peer's range of
/// them), and SipHash took a good part of a render.
type IdMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<IdHash>>;

/// Mixes each word in with a multiply (Fibonacci hashing), so ids that differ in low bits spread.
#[derive(Default)]
pub(crate) struct IdHash(u64);

impl Hasher for IdHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(u64::from(*b));
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
}

/// A layer, and the scale its text is greeked at (none: drawn in full).
type BoundsKey = (NodeId, Option<u64>);

/// Deepest nesting the paint bounds walk follows.
const MAX_BOUNDS_DEPTH: usize = 64;

pub fn ts_transform(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

/// kurbo path → tiny-skia path (None when empty or degenerate).
pub fn ts_path(p: &BezPath) -> Option<tiny_skia::Path> {
    use figcraft_geom::kurbo::PathEl;
    let mut b = tiny_skia::PathBuilder::new();
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(c, p) => b.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(c1, c2, p) => b.cubic_to(c1.x as f32, c1.y as f32, c2.x as f32, c2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

const IMAGE_PIXELS_SHOWN_FROM_MAGNIFICATION: f64 = 2.0;

fn pixels_when_magnified(prep: Prepared, device_scale: f64) -> Prepared {
    let Prepared::Pattern { pixmap, mode, quality, opacity, transform } = prep else { return prep };
    let image_scale = f64::from(transform.sx * transform.sy - transform.kx * transform.ky).abs().sqrt();
    let magnification = device_scale * image_scale;
    let quality = if magnification >= IMAGE_PIXELS_SHOWN_FROM_MAGNIFICATION { FilterQuality::Nearest } else { quality };
    Prepared::Pattern { pixmap, mode, quality, opacity, transform }
}

fn fill_with(pix: &mut Pixmap, path: &tiny_skia::Path, paints: &[Paint], w: f64, h: f64, xf: Affine, rule: FillRule, ctx: &Ctx, mask: Option<&Mask>) {
    let scale = xf.determinant().abs().sqrt();
    for p in paints {
        let Some(prep) = prepare_at(p, w, h, scale, ctx.doc, ctx.time, ctx.preview) else { continue };
        let prep = if ctx.magnified_image_pixels && matches!(p.kind, PaintKind::Image(_)) { pixels_when_magnified(prep, scale) } else { prep };
        let paint = tiny_skia::Paint { shader: prep.shader(), anti_alias: true, blend_mode: ts_blend(p.blend), ..Default::default() };
        if p.blend == BlendMode::PlusDarker {
            // No tiny-skia mode: the paint alone over its pixels, then blended in.
            let area = path
                .bounds()
                .transform(ts_transform(xf))
                .and_then(|b| int_rect(Rect::new(b.left().into(), b.top().into(), b.right().into(), b.bottom().into()), (pix.width(), pix.height())));
            let Some(mut alone) = area.and_then(|r| Pixmap::new(r.width(), r.height())) else { continue };
            let Some(r) = area else { continue };
            let t = ts_transform(Affine::translate((-f64::from(r.x()), -f64::from(r.y()))) * xf);
            alone.fill_path(path, &tiny_skia::Paint { blend_mode: tiny_skia::BlendMode::SourceOver, ..paint }, rule, t, None);
            crate::paint::composite(pix, (r.x(), r.y()), &alone, 1.0, p.blend);
            continue;
        }
        pix.fill_path(path, &paint, rule, ts_transform(xf), mask);
    }
}

/// Stroke outline cache: (geometry hash) → outline path.
fn stroke_cache() -> &'static Mutex<PathCache> {
    static C: OnceLock<Mutex<PathCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(PathCache::new(4096)))
}

pub(crate) fn hash_path(p: &BezPath, h: &mut impl Hasher) {
    for el in p.elements() {
        use figcraft_geom::kurbo::PathEl::*;
        match *el {
            MoveTo(a) | LineTo(a) => (0u8, a.x.to_bits(), a.y.to_bits()).hash(h),
            QuadTo(a, b) => (1u8, a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits()).hash(h),
            CurveTo(a, b, c) => (2u8, a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits(), c.x.to_bits(), c.y.to_bits()).hash(h),
            ClosePath => 3u8.hash(h),
        }
    }
}

/// Per-side strokes on rectangles and frames (a ring under non-zero winding too), if `n` has them.
fn side_strokes(n: &Node) -> Option<Arc<BezPath>> {
    let props = &n.stroke;
    let sides = props.sides?;
    if !matches!(n.kind, NodeKind::Rectangle | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. }) {
        return None;
    }
    let c = &n.corners;
    Some(Arc::new(if c.uniform() == Some(0.0) {
        figcraft_pathops::rect_side_strokes(n.width, n.height, sides, props.align)
    } else {
        figcraft_pathops::rounded_side_strokes(n.width, n.height, c.radii, c.smoothing, sides, props.align)
    }))
}

/// The stroke area of a node in local coordinates.
pub fn stroke_area(n: &Node, path: &BezPath, closed: bool) -> Arc<BezPath> {
    let props = &n.stroke;
    if let Some(p) = side_strokes(n) {
        return p;
    }
    // Rect/ellipse inside/outside strokes without booleans: an even-odd ring of two outlines.
    if props.dashes.is_empty() && props.profile().is_none() && props.brush.is_none() && closed && props.align != StrokeAlign::Center {
        let w = props.weight;
        let ring = |inset: f64| -> Option<BezPath> {
            let (x0, y0, ww, hh) = (inset, inset, n.width - 2.0 * inset, n.height - 2.0 * inset);
            if ww <= 0.0 || hh <= 0.0 {
                return None;
            }
            match n.kind {
                NodeKind::Rectangle
                | NodeKind::Frame
                | NodeKind::Component { .. }
                | NodeKind::ComponentSet { .. }
                | NodeKind::Instance { .. }
                | NodeKind::Icon { .. }
                | NodeKind::Section { .. }
                    if n.corners.smoothing == 0.0 =>
                {
                    let radii = n.corners.radii.map(|r| if r > 0.0 { (r - inset).max(0.0) } else { 0.0 });
                    let mut p = figcraft_geom::rounded_rect(ww, hh, radii, 0.0);
                    p.apply_affine(Affine::translate((x0, y0)));
                    Some(p)
                }
                NodeKind::Ellipse { sweep, inner, .. } if sweep.abs() >= std::f64::consts::TAU - 1e-9 && inner == 0.0 => {
                    Some(figcraft_geom::kurbo::Ellipse::from_rect(Rect::new(x0, y0, x0 + ww, y0 + hh)).to_path(0.05))
                }
                _ => None,
            }
        };
        let (outer, inner) = match props.align {
            StrokeAlign::Inside => (ring(0.0), ring(w)),
            _ => (ring(-w), ring(0.0)),
        };
        if let Some(mut o) = outer {
            if let Some(i) = inner {
                o.extend(i.elements().iter().copied());
            }
            return Arc::new(o);
        }
    }
    cached_stroke_outline(n, path, closed)
}

/// The stroke a boolean operand contributes: like [`stroke_area`], but never an even-odd ring
/// (boolean folding uses non-zero winding).
fn operand_stroke(n: &Node, path: &BezPath, closed: bool) -> Arc<BezPath> {
    side_strokes(n).unwrap_or_else(|| cached_stroke_outline(n, path, closed))
}

/// A non-zero stroke outline (cached), shared by painting and boolean operands.
fn cached_stroke_outline(n: &Node, path: &BezPath, closed: bool) -> Arc<BezPath> {
    let props = &n.stroke;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_path(path, &mut h);
    (props.weight.to_bits(), props.align as u8, props.cap_start as u8, props.cap_end as u8, props.join as u8, props.miter_limit.to_bits(), closed)
        .hash(&mut h);
    for d in &props.dashes {
        d.to_bits().hash(&mut h);
    }
    props.dashes.len().hash(&mut h);
    // Points' own caps and joins and the fill rule Inside/Outside clip by (vector networks).
    let net = match &n.kind {
        NodeKind::Vector { network } => Some(network),
        _ => None,
    };
    net.is_some_and(|n| n.regions.iter().any(|r| r.winding == figcraft_doc::WindingRule::EvenOdd)).hash(&mut h);
    for v in net.map(|n| n.vertices.as_slice()).unwrap_or_default() {
        (v.cap.map(|c| c as u8), v.join.map(|j| j as u8)).hash(&mut h);
    }
    for p in props.profile().map(|p| p.points.as_slice()).unwrap_or_default() {
        (p.t.to_bits(), p.scale.to_bits(), p.bias.to_bits()).hash(&mut h);
    }
    if let Some(b) = &props.brush {
        (&b.preset, b.spacing.to_bits(), b.size_jitter.to_bits(), b.rotation_jitter.to_bits(), b.scatter.to_bits(), b.seed).hash(&mut h);
    }
    let key = h.finish();
    if let Some(p) = locked(stroke_cache()).get(key) {
        return p;
    }
    let out = Arc::new(figcraft_pathops::stroke_outline_of(path, props, closed, net));
    locked(stroke_cache()).insert(key, out.clone());
    out
}

/// A node's painted stroke area in local coordinates with the rule to fill it by (`None` without a
/// visible stroke or geometry). Text strokes follow its glyphs and aren't covered.
pub fn stroke_fill(doc: &Document, n: &Node) -> Option<(Arc<BezPath>, FillRule)> {
    if !n.has_visible_stroke() {
        return None;
    }
    let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. });
    let geom = match &n.kind {
        NodeKind::BooleanOp { .. } => node_outline(doc, n).map(|(p, _)| p),
        NodeKind::Text(_) | NodeKind::Group | NodeKind::Page { .. } => None,
        _ => figcraft_doc::geometry::stroke_geometry(n),
    }?;
    Some((stroke_area(n, &geom, closed), stroke_rule(n, closed)))
}

/// Even-odd ring paths need the even-odd rule; everything else non-zero.
pub(crate) fn stroke_rule(n: &Node, closed: bool) -> FillRule {
    if n.stroke.sides.is_some() || (n.stroke.dashes.is_empty() && closed && n.stroke.align != StrokeAlign::Center) {
        FillRule::EvenOdd
    } else {
        FillRule::Winding
    }
}

/// The geometry a node fills, in local space: shapes, booleans (computed from children) and text
/// (glyph outlines). Groups and pages have none.
pub fn node_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    node_outline_at(doc, n, 0)
}

/// [`node_outline`] `depth` levels into a boolean or group nesting (one budget for both).
fn node_outline_at(doc: &Document, n: &Node, depth: usize) -> Option<(BezPath, FillRule)> {
    match &n.kind {
        NodeKind::BooleanOp { op } => {
            let operands: Vec<(BezPath, figcraft_doc::BoolOp)> = n
                .children
                .iter()
                .filter_map(|c| doc.node(*c))
                .filter(|c| c.visible)
                .filter_map(|c| {
                    let (mut p, _) = subtree_outline_at(doc, c, depth + 1, true)?;
                    p.apply_affine(c.transform);
                    Some((p, c.bool_op.unwrap_or(*op)))
                })
                .collect();
            Some((crate::boolean::fold_cached(&operands)?, FillRule::EvenOdd))
        }
        NodeKind::Text(_) => {
            let (l, dy) = figcraft_text::layout_node(n)?;
            let mut p = l.to_path();
            p.apply_affine(Affine::translate((0.0, dy)));
            Some((p, FillRule::Winding))
        }
        NodeKind::Vector { network } => {
            let rule =
                if network.regions.iter().any(|r| r.winding == figcraft_doc::WindingRule::EvenOdd) { FillRule::EvenOdd } else { FillRule::Winding };
            Some((network.fill_path(), rule))
        }
        _ => figcraft_doc::geometry::shape_path(n).map(|p| (p, FillRule::Winding)),
    }
}

/// The union outline of a node and its descendants (for booleans of groups/frames).
pub fn subtree_outline(doc: &Document, n: &Node, depth: usize) -> Option<(BezPath, FillRule)> {
    subtree_outline_at(doc, n, depth, false)
}

/// Boolean operands with only a stroke contribute the painted band, not their interior.
fn subtree_outline_at(doc: &Document, n: &Node, depth: usize, stroke_only: bool) -> Option<(BezPath, FillRule)> {
    if depth > 64 {
        return None;
    }
    match &n.kind {
        NodeKind::Group | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. } if !n.children.is_empty() => {
            let mut out = BezPath::new();
            if n.kind.is_frame_like()
                && let Some(p) = figcraft_doc::geometry::shape_path(n)
            {
                if n.fills.iter().any(|f| f.visible) {
                    out.extend(p.elements().iter().copied());
                } else if stroke_only && n.has_visible_stroke() {
                    out.extend(operand_stroke(n, &p, true).elements().iter().copied());
                }
            }
            for c in n.children.iter().filter_map(|c| doc.node(*c)) {
                if !c.visible {
                    continue;
                }
                if let Some((mut p, _)) = subtree_outline_at(doc, c, depth + 1, stroke_only) {
                    p.apply_affine(c.transform);
                    out.extend(p.elements().iter().copied());
                }
            }
            Some((out, FillRule::Winding))
        }
        _ if stroke_only && !n.fills.iter().any(|p| p.visible) && n.has_visible_stroke() => {
            let (path, _) = node_outline_at(doc, n, depth)?;
            let path = if matches!(n.kind, NodeKind::Text(_)) { path } else { figcraft_doc::geometry::stroke_geometry(n).unwrap_or(path) };
            let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. });
            Some(((*operand_stroke(n, &path, closed)).clone(), FillRule::Winding))
        }
        _ => node_outline_at(doc, n, depth),
    }
}

fn device_rect(xf: Affine, r: Rect) -> Rect {
    figcraft_geom::transform_rect_bbox(xf, r)
}

/// The area text may paint without its glyphs being looked at: its box, generously grown.
fn text_box_bounds(n: &Node) -> Rect {
    n.local_rect().inflate(n.height.max(10.0), n.height.max(10.0) * 0.5)
}

/// Local bounds a node paints into (its box grown by stroke/effect reach, plus unclipped children).
/// Text that overflows its box counts where its glyphs are, so it isn't culled while visible;
/// text greeked at `greek_scale` (see [`lod::greeked`]) paints only inside [`text_box_bounds`].
pub(crate) fn local_paint_bounds(doc: &Document, n: &Node, depth: usize, greek_scale: Option<f64>) -> Rect {
    bounds_within(doc, n, depth, greek_scale, None).0
}

/// [`local_paint_bounds`], kept in and read from `memo` when there is one. Also returns how many
/// levels the walk went down, or `None` when the nesting cap cut it short: such bounds depend on
/// the depth they were asked at and aren't kept, and kept bounds are read only where their levels
/// still fit under the cap.
pub(crate) fn bounds_within<'a>(
    doc: &'a Document,
    n: &Node,
    depth: usize,
    greek_scale: Option<f64>,
    memo: Option<&Bounds<'a>>,
) -> (Rect, Option<usize>) {
    // A layer without children is quicker to work out again than to look up.
    let memo = memo.filter(|_| !n.children.is_empty());
    let key = (n.id, greek_scale.map(f64::to_bits));
    if let Some((r, levels)) = memo.and_then(|m| m.paint.borrow().get(&key).copied())
        && depth + levels <= MAX_BOUNDS_DEPTH
    {
        return (r, Some(levels));
    }
    let reach = n.paint_reach();
    let mut r = n.local_rect().inflate(reach, reach);
    let mut levels = Some(0);
    let clips = n.clip && n.kind.is_frame_like();
    if (!clips || n.kind.bounds_follow_children()) && !n.hides_contents() {
        if depth < MAX_BOUNDS_DEPTH {
            let mut below = Some(0);
            for c in n.children.iter().filter_map(|c| memo.map_or_else(|| doc.node(*c), |m| m.node(doc, *c))) {
                if !c.visible {
                    continue;
                }
                let (cb, c_levels) = bounds_within(doc, c, depth + 1, greek_scale, memo);
                below = below.zip(c_levels).map(|(b, l)| b.max(l));
                r = r.union(figcraft_geom::transform_rect_bbox(c.transform, cb));
            }
            levels = below.map(|b| b + 1);
        } else if !n.children.is_empty() {
            levels = None;
        }
    }
    // Text can overflow its box.
    if let NodeKind::Text(_) = n.kind {
        r = r.union(text_box_bounds(n).inflate(reach, reach));
        if !greek_scale.is_some_and(|s| lod::greeked(n, s))
            && let Some((l, dy)) = figcraft_text::layout_node(n)
            && let Some(ink) = l.ink_bounds()
        {
            r = r.union(ink + figcraft_geom::kurbo::Vec2::new(0.0, dy));
        }
    }
    if let (Some(m), Some(l)) = (memo, levels) {
        m.paint.borrow_mut().insert(key, (r, l));
    }
    (r, levels)
}

/// How far (document units) effects that read outside a node's paint bounds reach: blurs, glass,
/// drop shadows and texture. Culling and partial renders keep this much margin.
pub fn effect_margin(n: &Node) -> f64 {
    n.effects
        .iter()
        .filter(|e| e.visible)
        .map(|e| match &e.kind {
            EffectKind::LayerBlur(b) => b.max_radius() * 2.0,
            EffectKind::BackgroundBlur { .. } | EffectKind::Glass(_) => e.backdrop_radius() * 2.0,
            EffectKind::DropShadow(s) => s.radius * 2.0 + s.spread.abs() * figcraft_doc::SPREAD_MITER_LIMIT + s.offset.0.abs() + s.offset.1.abs(),
            EffectKind::Texture(t) => t.radius.abs(),
            // A group's inner shadows are cast from around its pixels: a layer cut off at the
            // view would cast one from the cut.
            EffectKind::InnerShadow(s) if matches!(n.kind, NodeKind::Group) => s.radius * 2.0 + s.spread.abs() + s.offset.0.abs() + s.offset.1.abs(),
            _ => 0.0,
        })
        .filter(|m| m.is_finite())
        .fold(0.0, f64::max)
}

/// Everything node `id` can paint, in page coordinates: its paint bounds (glyphs of overflowing
/// text included) grown by its effects' margin. What a change to the node can affect on screen.
pub fn paint_bounds(doc: &Document, id: NodeId) -> Option<Rect> {
    let n = doc.node(id)?;
    let m = effect_margin(n).abs();
    let local = local_paint_bounds(doc, n, 0, None);
    Some(figcraft_geom::transform_rect_bbox(doc.world_transform(id), local).inflate(m, m))
}

fn needs_layer(n: &Node) -> bool {
    n.opacity < 0.999
        || !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough)
        || n.effects.iter().any(|e| e.visible)
        || n.isolates_children()
}

/// `v` on the whole pixel that only float noise keeps it off: an edge on a pixel boundary comes out
/// a hair either side of it depending on the view's origin, and rounding outwards would then
/// take in a whole extra row or column in one render and not in another.
fn snap_px(v: f64) -> f64 {
    let r = v.round();
    if (v - r).abs() < 1e-6 { r } else { v }
}

fn int_rect(r: Rect, limit: (u32, u32)) -> Option<IntRect> {
    let x0 = snap_px(r.x0).floor().max(0.0);
    let y0 = snap_px(r.y0).floor().max(0.0);
    let x1 = snap_px(r.x1).ceil().min(f64::from(limit.0));
    let y1 = snap_px(r.y1).ceil().min(f64::from(limit.1));
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// The most a layer reaches past the view for its effects, in device pixels.
const MAX_EFFECT_MARGIN_PX: f64 = 2048.0;

thread_local! {
    /// How deep the layer being drawn on this thread is. A pattern fill draws its source inside the
    /// layer's paint, which has no depth of its own: it goes on from here, so one depth cap bounds
    /// the whole render, patterns in patterns included (#1082).
    static LAYER_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The depth of the layer being drawn on this thread (0 outside a render), or the one an
/// exporter gave [`at_layer_depth`].
pub fn layer_depth() -> usize {
    LAYER_DEPTH.with(std::cell::Cell::get)
}

/// Run `f` as if a layer `depth` levels deep were drawing: what it renders (a layer drawn alone,
/// a pattern cell) goes on from there. Exporters, which nest layers themselves, call the renderer
/// through this so one depth cap bounds their nesting and the render's together (#1082).
pub fn at_layer_depth<T>(depth: usize, f: impl FnOnce() -> T) -> T {
    let _depth = LayerDepth::enter(depth);
    f()
}

/// [`LAYER_DEPTH`] while a layer draws; the one around it again after (on unwinding too).
struct LayerDepth(usize);

impl LayerDepth {
    fn enter(depth: usize) -> Self {
        Self(LAYER_DEPTH.with(|d| d.replace(depth)))
    }
}

impl Drop for LayerDepth {
    fn drop(&mut self) {
        LAYER_DEPTH.with(|d| d.set(self.0));
    }
}

/// Draw node `id` onto `pix`, where `parent_xf` maps the parent's space to `pix` pixels.
pub fn draw_node(ctx: &Ctx, pix: &mut Pixmap, id: NodeId, parent_xf: Affine, depth: usize) {
    if let Some(n) = ctx.doc.node(id) {
        draw_layer(ctx, pix, n, parent_xf, depth);
    }
}

/// [`draw_node`] for a node already looked up.
fn draw_layer(ctx: &Ctx, pix: &mut Pixmap, n: &Node, parent_xf: Affine, depth: usize) {
    let id = n.id;
    if depth > crate::MAX_EXPORT_DEPTH || ctx.skip.contains(&id) {
        return;
    }
    // `Options::only`: the listed layers **and everything inside them** are drawn (a container's
    // children come with it); a layer on the way down to one is walked but paints nothing, so the
    // layers under a clipping frame still come out with the frame's clip.
    if !ctx.only.is_empty()
        && !ctx.only.contains(&id)
        && !ctx.only.iter().any(|o| ctx.doc.is_ancestor(id, *o))
        && !ctx.only.iter().any(|o| ctx.doc.is_ancestor(*o, id))
    {
        return;
    }
    let _depth = LayerDepth::enter(depth);
    let outline = ctx.outline_mode || ctx.outlined.contains(&id);
    if n.is_mask || n.opacity <= 0.0 && n.visible {
        return;
    }
    if !n.visible {
        // Outline mode can show hidden layers too (nothing of them is culled: a hidden layer
        // isn't counted in its parent's bounds).
        if outline && ctx.outline.hidden {
            draw_outline(&Ctx { outline_mode: true, ..*ctx }, pix, n, parent_xf * n.transform, depth);
        }
        return;
    }
    let xf = parent_xf * n.transform;
    let (pw, ph) = (pix.width(), pix.height());
    let local = bounds_within(ctx.doc, n, 0, ctx.lod.then_some(ctx.scale), Some(ctx.bounds)).0;
    let dev = device_rect(xf, local);
    // Effects that read outside (blur) need a margin when culling. Absurd effects (a spread or
    // blur of millions of pixels) keep a sane layer: what lies further off reads as empty.
    let blur_margin = (effect_margin(n) * ctx.scale).min(MAX_EFFECT_MARGIN_PX);
    let view = Rect::new(0.0, 0.0, f64::from(pw), f64::from(ph)).inflate(blur_margin, blur_margin);
    if dev.intersect(view).is_zero_area() && !(dev.width() == 0.0 || dev.height() == 0.0) {
        return;
    }
    if outline {
        draw_outline(&Ctx { outline_mode: true, ..*ctx }, pix, n, xf, depth);
        return;
    }
    if ctx.lod && dev.width().max(dev.height()) < lod::TINY_PX && lod::splattable_within(ctx.doc, n, 0, Some(ctx.bounds)).0 {
        lod::splat_subtree(ctx, pix, n, xf, None, 1.0, depth);
        return;
    }
    if !needs_layer(n) && ctx.shade.and_then(|b| b.spec_for(id)).is_none() {
        draw_content(ctx, pix, n, xf, depth, None);
        return;
    }
    // Isolated layer covering the node's paint bounds, clipped to the visible area + margin. Its
    // origin can be negative (inside the margin); the size counts from there, or a layer starting
    // above or left of the pixmap came out short by the margin (a band missing at its far edge).
    let vis = dev.intersect(view);
    let (x0, y0) = (vis.x0.floor(), vis.y0.floor());
    let (w, h) = (vis.x1.ceil() - x0, vis.y1.ceil() - y0);
    if !(x0.is_finite() && y0.is_finite() && w >= 1.0 && h >= 1.0) {
        return;
    }
    let (lx, ly) = (x0 as i32, y0 as i32);
    let lw = w.min(16_384.0) as u32;
    let lh = h.min(16_384.0) as u32;
    let Some(mut layer) = Pixmap::new(lw, lh) else { return };
    let lxf = Affine::translate((-f64::from(lx), -f64::from(ly))) * xf;
    let shape = node_outline(ctx.doc, n);
    // 外置 WGSL 材质：这一层的「底图」由 GPU 现算，落点与内置 Glass 完全相同（内容之下的
    // 那一层像素），子树仍由普通递归画在它上面。有材质时内置 backdrop 效果让位——两条路径
    // 同时画会在同一层里叠两次玻璃。
    //
    // 每条跨到这个节点的带都按**自己窗口**算玻璃：缓存键含窗口尺寸 / origin / 底图，所以相邻带
    // 不会互相覆盖（早期「单一 per-node 槽」才会让所有带画成同一张、出现重复亮带）；带的
    // overlap 又按材质透镜 reach 给了模糊足够上下文，拼接处连续。绝不能只让「一条带」认领——
    // 那样比视口高的窗格在其余带里整块不画玻璃，放大到 235% 时整块消失。
    let shader = ctx.shade.and_then(|b| b.spec_for(id));
    let shaded = shader.as_ref().and_then(|spec| shader_layer(ctx, pix, n, spec, (lx, ly), (lw, lh), lxf));
    // Background blur and glass: the whole backdrop under the layer (every surface below this
    // one, composited), through the shape, under the content.
    let back_shape = if shader.is_none() && backdrop::has_effects(n) { backdrop::outline(ctx.doc, n) } else { None };
    let read_backdrop = |pix: &Pixmap| back_shape.as_ref().and_then(|sh| backdrop_layer(ctx, pix, n, sh, (lx, ly), (lw, lh), lxf));
    // With drop shadows too, the layer casts them from what it paints, not from the backdrop, and
    // the backdrop is read again once they lie under it (a shadow showing behind is seen through
    // it). Until then, the layer's content sees the backdrop read first under it (#255).
    let casts = back_shape.is_some() && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::DropShadow(_)));
    // Kept apart, the first read is only what backdrop effects inside the content see: without
    // any, the backdrop (blur or glass, the costly part) is computed once, after the shadows.
    let nested = || ctx.doc.subtree(id).iter().skip(1).any(|c| ctx.doc.node(*c).is_some_and(backdrop::has_effects));
    let first_back = if !casts || nested() { read_backdrop(pix) } else { None };
    if !casts && let Some(b) = &first_back {
        layer.draw_pixmap(0, 0, b.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    // 材质产出的那一层像素：与上面一样，画在内容之下。
    if let Some(s) = &shaded {
        layer.draw_pixmap(0, 0, s.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    let op = n.opacity.clamp(0.0, 1.0);
    {
        // Inner shadows paint over the fills, under the children and strokes.
        let inner = |l: &mut Pixmap| {
            for e in n.effects.iter().filter(|e| e.visible) {
                if let EffectKind::InnerShadow(s) = &e.kind
                    && let Some(sh) = &shape
                    && let Some(inner) = shadow::inner_shadow(sh, lw, lh, lxf, ctx.scale, s)
                {
                    l.draw_pixmap(0, 0, inner.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
        };
        let has_inner = shape.is_some() && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::InnerShadow(_)));
        // Backdrops further down see this layer composited onto `pix` (over its backdrop, when
        // that is kept apart for now).
        let under = Under { opacity: op, blend: n.blend, ..Under::new(pix, (lx, ly), ctx.under) };
        let over_back = first_back.as_ref().filter(|_| casts).map(|b| Under::new(b, (0, 0), Some(&under)));
        let content_under = over_back.as_ref().unwrap_or(&under);
        let inner_ctx = Ctx { under: Some(content_under), ..*ctx };
        // 绑了外置材质：材质就是节点那层漆。整页合成通道把节点整个挖空、再把材质贴回，逐层
        // 通道要给出同一张图，就只能画子树——照常画填充会把玻璃整个盖掉（不透明填充尤其）。
        if shader.is_some() {
            draw_children_only(&inner_ctx, &mut layer, n, lxf, depth);
        } else {
            draw_content(&inner_ctx, &mut layer, n, lxf, depth, has_inner.then_some(&inner as &dyn Fn(&mut Pixmap)));
        }
    }
    // A group is no outline of its own: its inner shadows are cast from around what its layers
    // draw together, over them.
    if matches!(n.kind, NodeKind::Group) && n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::InnerShadow(_))) {
        let cover = layer.clone();
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::InnerShadow(s) = &e.kind
                && let Some(sh) = shadow::inner_shadow_of(&cover, ctx.scale, s)
            {
                layer.draw_pixmap(0, 0, sh.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        }
    }
    // Layer blur, noise and texture, in list order, over everything else.
    if !casts {
        layer_effects(ctx, &mut layer, n, id, lxf);
    }
    // Drop shadows go under the layer, in effect-list order.
    {
        // Kept apart, the backdrop isn't in the layer yet: the shadows are cast from the content
        // with the layer effects it will get (a copy only when it gets some).
        let layer_fx =
            n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::LayerBlur(_) | EffectKind::Noise(_) | EffectKind::Texture(_)));
        let content = (casts && layer_fx).then(|| {
            let mut c = layer.clone();
            layer_effects(ctx, &mut c, n, id, lxf);
            c
        });
        let caster = shadow::Caster::new(ctx.doc, n, content.as_ref().unwrap_or(&layer), lxf, ctx.scale);
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::DropShadow(s) = &e.kind
                && let Some(sh) = shadow::drop_shadow(&caster, s)
            {
                composite(pix, (lx, ly), &sh, op, s.blend);
            }
        }
    }
    if casts {
        // The backdrop with the shadows under it, the content over it, then the layer effects.
        if let Some(mut b) = read_backdrop(pix) {
            b.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            layer = b;
        }
        layer_effects(ctx, &mut layer, n, id, lxf);
    }
    composite(pix, (lx, ly), &layer, op, n.blend);
}

/// 外置 WGSL 材质产出的那一层像素：这一层内容之下的底图，由 GPU 换成 shader 的结果。
///
/// 命中（同一 layer、同一底图、同一参数）直接给出可画的图；未命中则**登记请求**并返回 `None`
/// ——本帧这一层就没有玻璃，内容照常画，下一帧真图上台。渲染线程不阻塞在 GPU 上，
/// 这是 [`crate::shade`] 的契约。
fn shader_layer(
    ctx: &Ctx,
    pix: &Pixmap,
    n: &Node,
    spec: &Arc<crate::shade::ShaderSpec>,
    at: (i32, i32),
    (lw, lh): (u32, u32),
    lxf: Affine,
) -> Option<Pixmap> {
    let board = ctx.shade?;
    if lw == 0 || lh == 0 {
        return None;
    }
    // 窗口左上角在**节点像素**空间里的位置：层可能只是节点的可见那一块。
    let origin = {
        let p = lxf * figcraft_geom::Point::new(0.0, 0.0);
        (-p.x as f32, -p.y as f32)
    };
    let bounds = ctx.doc.bounds_of(&[n.id]);
    let node_px = bounds.map_or((lw as f32, lh as f32), |r| {
        ((r.width() * ctx.scale) as f32, (r.height() * ctx.scale) as f32)
    });
    let bounds = bounds.map_or([0.0; 4], |r| [r.x0, r.y0, r.x1, r.y1]);
    let radii_px = n.corners.radii.map(|v| (v * ctx.scale).max(0.0) as f32);
    // 这一层背后（不含层自己）的像素：内置 Glass 读的就是这份底图，材质读同一份。
    let back = backdrop::backdrop(pix, ctx.under, at.0, at.1, lw, lh)?;
    let straight = crate::shade::straight(&back);
    let key = crate::shade::shade_key(spec, (lw, lh), origin, radii_px, ctx.scale, &straight);
    if let Some(s) = board.ready(n.id, key) {
        return Some(s.pixmap.clone());
    }
    board.request(crate::shade::ShadeRequest {
        node: n.id,
        key,
        spec: spec.clone(),
        size: (lw, lh),
        node_px,
        origin_px: origin,
        radii_px,
        scale: ctx.scale,
        bounds,
        smoothing: n.corners.smoothing,
        backdrop: Arc::new(straight),
    });
    // 没算出来就用这一层上一张：拖动材质本身时键每帧都换，画空白会闪。第一次见到这个节点
    // 时它本来就没有上一张，那一帧仍然是空的。
    board.latest(n.id, (lw, lh), origin).map(|s| s.pixmap.clone())
}

/// Node `n`'s backdrop effects through its backdrop outline `sh`, for its `size` layer at `at` on
/// `pix` (local → layer pixels through `lxf`): what `pix` holds there now, over what lies under it.
fn backdrop_layer(ctx: &Ctx, pix: &Pixmap, n: &Node, sh: &shadow::Outline, at: (i32, i32), size: (u32, u32), lxf: Affine) -> Option<Pixmap> {
    let ((lx, ly), (lw, lh)) = (at, size);
    let pad = backdrop::pad(n, ctx.scale);
    let p = i32::try_from(pad).unwrap_or(0);
    let back = backdrop::backdrop(pix, ctx.under, lx.saturating_sub(p), ly.saturating_sub(p), lw + pad * 2, lh + pad * 2)?;
    backdrop::effects(n, backdrop::EffectFrame { shape: sh, back: &back, pad, size: (lw, lh), xf: lxf, scale: ctx.scale })
}

/// Node `n`'s layer blur, noise and texture on its `layer`, in list order.
fn layer_effects(ctx: &Ctx, layer: &mut Pixmap, n: &Node, id: NodeId, lxf: Affine) {
    for e in n.effects.iter().filter(|e| e.visible) {
        match &e.kind {
            EffectKind::LayerBlur(b) => blur::apply(layer, b, (n.width, n.height), lxf, ctx.scale),
            EffectKind::Noise(nz) => grain::noise(layer, nz, lxf, id.0 as u32),
            EffectKind::Texture(t) => grain::texture(layer, t, lxf, ctx.scale, id.0 as u32),
            _ => {}
        }
    }
}

fn draw_outline(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    let color = if ctx.outline.light { tiny_skia::Color::WHITE } else { tiny_skia::Color::BLACK };
    let paint = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(color), anti_alias: true, ..Default::default() };
    let stroke = tiny_skia::Stroke { width: 1.0 / ctx.scale.max(1e-6) as f32, ..Default::default() };
    if let Some((path, _)) = node_outline(ctx.doc, n)
        && !matches!(n.kind, NodeKind::Group)
        && let Some(tp) = ts_path(&path)
    {
        pix.stroke_path(&tp, &paint, &stroke, ts_transform(xf), None);
    }
    // Object bounds: the box of every layer whose outline isn't its box already.
    let boxed = matches!(n.kind, NodeKind::Rectangle) && n.corners.uniform() == Some(0.0) || n.kind.is_frame_like();
    if ctx.outline.bounds
        && !boxed
        && let Some(tp) = ts_path(&figcraft_geom::kurbo::Shape::to_path(&n.local_rect(), 0.1))
    {
        pix.stroke_path(&tp, &paint, &stroke, ts_transform(xf), None);
    }
    if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.hides_contents() {
        for c in n.paint_order() {
            draw_node(ctx, pix, c, xf, depth + 1);
        }
    }
}

/// What paints over a node's fills, under its children and strokes (its inner shadows).
type OverFills<'a> = Option<&'a dyn Fn(&mut Pixmap)>;

/// What a node's children paint, through its clip, **without** its own fill, stroke or inner shadow.
/// A layer that paints nothing of its own (see `paints_nothing`) and one whose paint an external
/// material replaces both come out this way.
fn draw_children_only(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    if n.children.is_empty() || n.hides_contents() {
        return;
    }
    if n.clip && n.kind.is_frame_like() {
        let outline = node_outline(ctx.doc, n);
        draw_children_clipped(ctx, pix, n, xf, depth, outline.as_ref().map(|(p, _)| p));
    } else {
        draw_children(ctx, pix, n, xf, depth);
    }
}

/// Fills, `over_fills`, children (clipped), strokes and text of a node, without opacity/effects.
fn draw_content(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize, over_fills: OverFills) {
    let doc = ctx.doc;
    // A layer on the way down to `Options::only` paints nothing of its own; its children still go
    // through its clip, so a dragged layer is cut off by its frame exactly as it sits. A layer
    // **inside** a listed one (a container's child) paints as usual.
    let in_only = ctx.only.is_empty() || ctx.only.iter().any(|o| doc.is_ancestor(*o, n.id));
    if !in_only {
        draw_children_only(ctx, pix, n, xf, depth);
        return;
    }
    let (w, h) = (n.width, n.height);
    match &n.kind {
        NodeKind::Page { .. } => {
            draw_children(ctx, pix, n, xf, depth);
        }
        NodeKind::Group => draw_children(ctx, pix, n, xf, depth),
        NodeKind::Text(_) if ctx.lod && lod::greeked(n, ctx.scale) => {
            lod::greek_text(pix, n, xf, text_box_bounds(n));
            if let Some(f) = over_fills {
                f(pix);
            }
        }
        NodeKind::Text(t) => {
            let Some((layout, dy)) = figcraft_text::layout_node(n) else { return };
            let gxf = xf * Affine::translate((0.0, dy));
            // Colour glyphs (emoji) carry their own ink, which draws instead of their outline: a
            // bitmap the font ships, or COLR layers with a colour each.
            let gs = gxf.determinant().abs().sqrt();
            let mut ink: Vec<(usize, Ink)> = Vec::new();
            // Group glyphs by style so runs with their own fills paint with them.
            let mut by_style: HashMap<usize, BezPath> = HashMap::new();
            for g in &layout.glyphs {
                match g.color(gs) {
                    Some(figcraft_text::ColorInk::Bitmap(bitmap, place)) => ink.push((g.style, Ink::Bitmap(bitmap, place))),
                    Some(figcraft_text::ColorInk::Layers(layers)) => {
                        for layer in layers.iter() {
                            ink.push((g.style, Ink::Layer(g.layer_path(layer.gid), layer.rgba)));
                        }
                    }
                    None => by_style.entry(g.style).or_default().extend(g.path().elements().iter().copied()),
                }
            }
            for d in &layout.decorations {
                by_style.entry(d.style).or_default().extend(d.path().elements().iter().copied());
            }
            // A run can have ink and no outline at all (bitmap-only fonts).
            let mut keys: Vec<usize> = by_style.keys().copied().chain(ink.iter().map(|(style, _)| *style)).collect();
            keys.sort_unstable();
            keys.dedup();
            // Every run's fills (its ink included), then what paints over the fills, then the
            // strokes on top.
            for k in &keys {
                let fills = layout.styles.get(*k).and_then(|s| s.fills.as_ref()).unwrap_or(&n.fills);
                if let Some(tp) = by_style.get(k).and_then(ts_path) {
                    fill_with(pix, &tp, fills, w, h, gxf, FillRule::Winding, ctx, None);
                }
                for (_, item) in ink.iter().filter(|(style, _)| style == k) {
                    match item {
                        Ink::Bitmap(bitmap, place) => blit(pix, bitmap, ts_transform(gxf * *place)),
                        Ink::Layer(path, rgba) => {
                            if let Some(tp) = ts_path(path) {
                                let mut paint = tiny_skia::Paint::default();
                                paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
                                paint.anti_alias = true;
                                pix.fill_path(&tp, &paint, FillRule::Winding, ts_transform(gxf), None);
                            }
                        }
                    }
                }
            }
            if let Some(f) = over_fills {
                f(pix);
            }
            if n.has_visible_stroke() {
                // Glyph outlines are closed: inside and outside strokes clip to them (#615).
                for path in keys.iter().filter_map(|k| by_style.get(k)) {
                    let s = stroke_area(n, path, true);
                    if let Some(ts) = ts_path(&s) {
                        fill_with(pix, &ts, &n.strokes, w, h, gxf, stroke_rule(n, true), ctx, None);
                    }
                }
            }
            let _ = t;
        }
        _ => {
            let closed = figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. });
            let splatted = ctx.lod && closed && lod::splat_fills(pix, n, xf);
            let clips = n.clip && n.kind.is_frame_like() && !n.children.is_empty();
            // The outline fills, clips children to rounded corners and strokes booleans.
            let outline = if splatted && !(clips && n.corners.uniform() != Some(0.0)) { None } else { node_outline(doc, n) };
            if !splatted
                && let Some((path, rule)) = &outline
                && closed
                && let Some(tp) = ts_path(path)
            {
                fill_with(pix, &tp, &n.fills, w, h, xf, *rule, ctx, None);
            }
            if let Some(f) = over_fills {
                f(pix);
            }
            // 画布提示带：在子层之前铺，子层的抗锯齿边缘与带子混色（而不是与画框底色混色）。
            paint_bands(ctx, pix, n.id);
            if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.children.is_empty() && !n.hides_contents() {
                if clips {
                    draw_children_clipped(ctx, pix, n, xf, depth, outline.as_ref().map(|(p, _)| p));
                } else {
                    draw_children(ctx, pix, n, xf, depth);
                }
            }
            if n.has_visible_stroke() {
                let geom = match &n.kind {
                    NodeKind::BooleanOp { .. } => outline.as_ref().map(|(p, _)| p.clone()),
                    _ => figcraft_doc::geometry::stroke_geometry(n),
                };
                if let Some(g) = geom {
                    let area = stroke_area(n, &g, closed);
                    if let Some(tp) = ts_path(&area) {
                        fill_with(pix, &tp, &n.strokes, w, h, xf, stroke_rule(n, closed), ctx, None);
                    }
                }
            }
        }
    }
}

/// Draw what `n` holds (a page's children for [`crate::render_view`], a frame's otherwise), with
/// rectangles of one plain colour that share an edge filled as one path.
pub fn draw_children_of(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    draw_children(ctx, pix, n, xf, depth);
}

/// One thing a container paints, in paint order.
struct Step {
    id: NodeId,
    /// Maps the step's *parent* space to pixels.
    xf: Affine,
    depth: usize,
    /// Set on a mask: the layers it clips, drawn through its alpha.
    mask_content: Option<Vec<NodeId>>,
}

fn draw_children(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize) {
    let mut steps = Vec::new();
    steps_of(ctx, n, xf, depth + 1, &mut steps);
    let merges = merge_plan(ctx, &steps);
    for (i, s) in steps.iter().enumerate() {
        if let Some(content) = &s.mask_content {
            // Everything above the mask (until the next mask) is clipped by its alpha.
            draw_masked(ctx, pix, s.id, content, s.xf, s.depth);
            continue;
        }
        if let Some(c) = merges.plan[i] {
            // The first of a cluster paints all of it, in paint order: nothing that paints lies
            // between them (see `merge_plan`), so drawing them at its turn changes no pixel.
            if merges.clusters[c].first() == Some(&i)
                && let Some(node) = ctx.bounds.node(ctx.doc, s.id)
                && let Some(fills) = plain_rect(ctx, node)
            {
                let members: Vec<(NodeId, Affine)> = merges.clusters[c].iter().filter_map(|k| steps.get(*k).map(|s| (s.id, s.xf))).collect();
                fill_rects(ctx, pix, &members, fills);
            }
            continue;
        }
        if let Some(node) = ctx.bounds.node(ctx.doc, s.id) {
            draw_layer(ctx, pix, node, s.xf, s.depth);
        }
    }
}

/// The steps of container `n`, with layers that paint nothing of their own inlined: the children of
/// such a layer land on the same pixels as the layers painted around it, so a rectangle inside one
/// can share an edge with one outside it (and be filled together with it).
fn steps_of(ctx: &Ctx, n: &Node, xf: Affine, depth: usize, out: &mut Vec<Step>) {
    let kids = &n.children;
    // Each child looked up once: in a large document a lookup is a good part of a layer's cost.
    let nodes: Vec<Option<&Node>> = kids.iter().map(|c| ctx.bounds.node(ctx.doc, *c)).collect();
    let runs = Document::paint_runs_of(&nodes, n.layout.as_ref().is_some_and(|l| l.reverse_z));
    for run in runs {
        let (Some(&c), Some(Some(cn))) = (kids.get(run.start), nodes.get(run.start)) else { continue };
        if cn.is_mask && cn.visible {
            out.push(Step { id: c, xf, depth, mask_content: Some(kids.get(run.start + 1..run.end).unwrap_or(&[]).to_vec()) });
            continue;
        }
        // `only` and the preview LOD walk the tree their own way: leave their nesting alone.
        if run.len() == 1 && ctx.only.is_empty() && !ctx.lod && paints_nothing(ctx, cn) {
            steps_of(ctx, cn, xf * cn.transform, depth + 1, out);
            continue;
        }
        out.push(Step { id: c, xf, depth, mask_content: None });
    }
}

/// A layer that paints nothing of its own and doesn't clip: drawing it inline changes no pixel.
fn paints_nothing(ctx: &Ctx, n: &Node) -> bool {
    // Outline mode draws the layer instead of its contents.
    if ctx.outline_mode || ctx.outlined.contains(&n.id) {
        return false;
    }
    if !n.visible || n.is_mask || n.opacity < 0.999 || n.clip || n.hides_contents() || n.isolates_children() {
        return false;
    }
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) {
        return false;
    }
    if n.has_visible_stroke() || n.effects.iter().any(|e| e.visible) {
        return false;
    }
    // Drawing it inline would paint its children past the shader bound to it.
    if ctx.shade.and_then(|b| b.spec_for(n.id)).is_some() {
        return false;
    }
    match n.kind {
        NodeKind::Group => true,
        // A frame with a fill lays a backdrop where it sits in the paint order; an empty one only
        // holds its children.
        _ => n.kind.is_frame_like() && n.fills.is_empty(),
    }
}

/// Which steps a container fills as one path.
struct Merges {
    /// The cluster each step belongs to, if any.
    plan: Vec<Option<usize>>,
    /// Each cluster's steps, in paint order.
    clusters: Vec<Vec<usize>>,
}

/// Rectangles of one plain colour that meet along an edge, filled in one path.
///
/// Two shapes each carrying anti-aliasing cover a shared edge that falls between pixels with `a`
/// and `1-a`; two SourceOver passes leave `c*(a+1-a-a(1-a)) + bg*a(1-a)` — a hair of the backdrop
/// showing through, the 1px line at some zoom levels. In one path the shared edge is interior and
/// covered once. (Skia's GPU stencil-then-cover does that by construction; filling shape by shape
/// on the CPU is what needs the merge.)
///
/// Rectangles are merged across the layers painted between them only when none of those layers can
/// paint over them: then painting them at the first one's turn changes no pixel and the paint order
/// still holds. Anything in between that can paint blocks the merge.
fn merge_plan(ctx: &Ctx, steps: &[Step]) -> Merges {
    let mut cands: Vec<(usize, &[Paint], Rect)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if s.mask_content.is_some() || s.depth > crate::MAX_EXPORT_DEPTH {
            continue;
        }
        let Some(n) = ctx.bounds.node(ctx.doc, s.id) else { continue };
        let (Some(fills), Some(r)) = (plain_rect(ctx, n), world_box(ctx.doc, n)) else { continue };
        cands.push((i, fills, r));
    }
    let n = cands.len();
    let mut root: Vec<usize> = (0..n).collect();
    // 按填充分组后再两两比较：一个容器里通常只有一两种颜色，分组把 O(n²) 压到 O(Σk²)。
    let mut by_fill: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        match by_fill.iter_mut().find(|g| cands[g[0]].1 == cands[i].1) {
            Some(g) => g.push(i),
            None => by_fill.push(vec![i]),
        }
    }
    for group in &by_fill {
        for (x, &a) in group.iter().enumerate() {
            for &b in &group[x + 1..] {
                if shares_an_edge(cands[a].2, cands[b].2) {
                    let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                    if ra != rb {
                        root[ra] = rb;
                    }
                }
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut root, i);
        by_root.entry(r).or_default().push(i);
    }
    let mut plan: Vec<Option<usize>> = vec![None; steps.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for members in by_root.into_values() {
        if members.len() < 2 {
            continue;
        }
        let mut union = cands[members[0]].2;
        for m in &members {
            union = union.union(cands[*m].2);
        }
        let lo = members.iter().map(|m| cands[*m].0).min().unwrap_or(0);
        let hi = members.iter().map(|m| cands[*m].0).max().unwrap_or(0);
        let blocked = (lo..=hi).any(|k| {
            if members.iter().any(|m| cands[*m].0 == k) {
                return false;
            }
            match &steps[k].mask_content {
                Some(_) => true,
                None => occludes(ctx, steps[k].id, union),
            }
        });
        if blocked {
            continue;
        }
        let mut list: Vec<usize> = members.iter().map(|m| cands[*m].0).collect();
        list.sort_unstable();
        let at = clusters.len();
        for s in &list {
            plan[*s] = Some(at);
        }
        clusters.push(list);
    }
    Merges { plan, clusters }
}

fn find(root: &mut [usize], mut i: usize) -> usize {
    while root[i] != i {
        root[i] = root[root[i]];
        i = root[i];
    }
    i
}

/// Whether layer `id` can paint over `area` (both in page coordinates). A hidden or skipped layer
/// paints nothing and blocks nothing; one that can't be measured blocks to be safe.
fn occludes(ctx: &Ctx, id: NodeId, area: Rect) -> bool {
    let Some(n) = ctx.doc.node(id) else { return false };
    if !n.visible || ctx.skip.contains(&id) {
        return false;
    }
    let Some(b) = paint_bounds(ctx.doc, id) else { return true };
    let hit = b.intersect(area);
    hit.width() > 0.0 && hit.height() > 0.0
}

/// Two boxes meet along an edge: they touch, and their overlap along it is longer than a hair.
/// Only then does either side's anti-aliasing leave a seam.
fn shares_an_edge(a: Rect, b: Rect) -> bool {
    const EPS: f64 = 1e-6;
    let side_by_side = (a.x1 - b.x0).abs() < EPS || (b.x1 - a.x0).abs() < EPS;
    let stacked = (a.y1 - b.y0).abs() < EPS || (b.y1 - a.y0).abs() < EPS;
    let along_y = a.y1.min(b.y1) - a.y0.max(b.y0);
    let along_x = a.x1.min(b.x1) - a.x0.max(b.x0);
    (side_by_side && along_y > EPS) || (stacked && along_x > EPS)
}

/// The node's box in page coordinates, if it is still an axis-aligned rectangle there: a rotated
/// one has no edge to share.
fn world_box(doc: &Document, n: &Node) -> Option<Rect> {
    let w = doc.world_transform(n.id);
    let c = w.as_coeffs();
    if c[1].abs() > 1e-9 || c[2].abs() > 1e-9 {
        return None;
    }
    Some(figcraft_geom::transform_rect_bbox(w, n.local_rect()))
}

/// The fills of a layer that is nothing but a plain, square, unrotated rectangle: the only kind
/// whose edge can be shared with a neighbour without either of them showing through. Anything with
/// corners, a stroke, an effect, its own opacity or blend, or a gradient/image fill is drawn on its
/// own (merging it would change what it paints — those paints are laid out in the shape's own box).
fn plain_rect<'d>(ctx: &Ctx<'_, 'd>, n: &'d Node) -> Option<&'d [Paint]> {
    if !n.visible || n.is_mask || n.clip || !n.children.is_empty() {
        return None;
    }
    // A layer bound to a shader is painted by `draw_layer`, which evaluates the material on the
    // layer's own backdrop: filling it along with its neighbours would skip that.
    if ctx.shade.and_then(|b| b.spec_for(n.id)).is_some() {
        return None;
    }
    // Outline mode paints the outline instead of the fill; a skipped node isn't drawn at all.
    if ctx.outline_mode || ctx.outlined.contains(&n.id) || ctx.skip.contains(&n.id) {
        return None;
    }
    // Only these layers are being drawn: a neighbour that isn't one of them would be filled by the
    // merged path all the same.
    if !ctx.only.is_empty() {
        return None;
    }
    if !matches!(n.kind, NodeKind::Rectangle) || n.corners.uniform() != Some(0.0) {
        return None;
    }
    if n.has_visible_stroke() || n.effects.iter().any(|e| e.visible) {
        return None;
    }
    if n.opacity != 1.0 || n.blend != BlendMode::Normal {
        return None;
    }
    // Opaque only: two translucent copies overlapping would darken where they meet when drawn
    // apart, and one path paints that place once — a merge would change what the document shows.
    let mut solid = !n.fills.is_empty();
    for p in &n.fills {
        if p.blend != BlendMode::Normal || p.opacity < 1.0 {
            solid = false;
        }
        match &p.kind {
            PaintKind::Solid { color } if color.a >= 1.0 => {}
            _ => solid = false,
        }
    }
    solid.then_some(n.fills.as_slice())
}

/// Fill the rectangles of one cluster as one path so their shared edges are covered once. Each
/// member's transform goes into the path with the transform it is drawn through (they can come from
/// different inlined layers); the path is then filled as it is.
fn fill_rects(ctx: &Ctx, pix: &mut Pixmap, members: &[(NodeId, Affine)], fills: &[Paint]) {
    let mut path = BezPath::new();
    for (id, xf) in members {
        let Some(n) = ctx.doc.node(*id) else { continue };
        let Some(mut p) = figcraft_doc::geometry::shape_path(n) else { continue };
        p.apply_affine(*xf * n.transform);
        path.extend(p.elements().iter().copied());
    }
    let Some(tp) = ts_path(&path) else { return };
    // Winding: every rectangle is a closed subpath of the same orientation, so the union fills and
    // the shared edges (interior, not outline) produce no partial coverage.
    fill_with(pix, &tp, fills, 0.0, 0.0, Affine::IDENTITY, FillRule::Winding, ctx, None);
}

/// 铺这个图层名下的提示带（没有就什么都不做）。
fn paint_bands(ctx: &Ctx, pix: &mut Pixmap, owner: NodeId) {
    if ctx.bands.is_empty() {
        return;
    }
    for b in ctx.bands.iter().filter(|b| b.owner == owner) {
        crate::band::paint(ctx.page_xf, pix, b);
    }
}

fn draw_masked(ctx: &Ctx, pix: &mut Pixmap, mask_id: NodeId, content: &[NodeId], xf: Affine, depth: usize) {
    let (w, h) = (pix.width(), pix.height());
    let Some(mut mask_px) = Pixmap::new(w, h) else { return };
    let mask_type = ctx.doc.node(mask_id).map(|m| m.mask_type).unwrap_or_default();
    // Draw the mask node as if it weren't a mask (a vector mask: its outline, fully opaque).
    if let Some(m) = ctx.doc.node(mask_id) {
        let mut m2 = m.clone();
        m2.is_mask = false;
        let mxf = xf * m2.transform;
        if mask_type == figcraft_doc::MaskType::Vector {
            if let Some((outline, rule)) = subtree_outline(ctx.doc, &m2, depth + 1)
                && let Some(path) = ts_path(&outline)
            {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(255, 255, 255, 255);
                paint.anti_alias = true;
                mask_px.fill_path(&path, &paint, rule, ts_transform(mxf), None);
            }
        } else if needs_layer(&m2) {
            let paint = PixmapPaint { opacity: m2.opacity, ..Default::default() };
            let mut tmp = Pixmap::new(w, h);
            if let Some(tmp) = tmp.as_mut() {
                draw_content(ctx, tmp, &m2, mxf, depth + 1, None);
                mask_px.draw_pixmap(0, 0, tmp.as_ref(), &paint, Transform::identity(), None);
            }
        } else {
            draw_content(ctx, &mut mask_px, &m2, mxf, depth + 1, None);
        }
    }
    let Some(mut layer) = Pixmap::new(w, h) else { return };
    let kind = if mask_type == figcraft_doc::MaskType::Luminance { MaskType::Luminance } else { MaskType::Alpha };
    let mask = Mask::from_pixmap(mask_px.as_ref(), kind);
    {
        let under = Under { clip: Some(&mask), ..Under::new(pix, (0, 0), ctx.under) };
        let sub = Ctx { under: Some(&under), ..*ctx };
        for c in content {
            draw_node(&sub, &mut layer, *c, xf, depth + 1);
        }
    }
    pix.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
}

fn draw_children_clipped(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, depth: usize, clip: Option<&BezPath>) {
    let (pw, ph) = (pix.width(), pix.height());
    let dev = device_rect(xf, n.local_rect());
    let Some(ir) = int_rect(dev, (pw, ph)) else { return };
    let (lx, ly) = (ir.x(), ir.y());
    // The layer starts as what lies under it and goes back in its place through the clip:
    // clipping cuts the children off without isolating them, so their blend modes reach the
    // layers behind the frame.
    let Some(mut layer) = pix.clone_rect(ir) else { return };
    let lxf = Affine::translate((-f64::from(lx), -f64::from(ly))) * xf;
    let square_aligned = n.corners.uniform() == Some(0.0) && is_axis_aligned(xf);
    let rect_clip = square_aligned && on_whole_pixels(dev);
    // The clip (a rectangular one on whole pixels is the layer's own box) is known before the
    // children draw, so their backdrops are clipped the way the layer will be. An axis-aligned
    // square frame with fractional edges gets an anti-aliased mask of its box so its edge pixels
    // are only partly covered.
    let mask = if rect_clip {
        None
    } else {
        let own_box = (square_aligned && clip.is_none()).then(|| n.local_rect().to_path(0.1));
        clip.or(own_box.as_ref()).and_then(ts_path).and_then(|cp| {
            let mut m = Mask::new(ir.width(), ir.height())?;
            m.fill_path(&cp, FillRule::Winding, true, ts_transform(lxf));
            Some(m)
        })
    };
    if !rect_clip && mask.is_none() {
        return;
    }
    {
        let under = Under { clip: mask.as_ref(), replaces: true, ..Under::new(pix, (lx, ly), ctx.under) };
        draw_children(&Ctx { under: Some(&under), ..*ctx }, &mut layer, n, lxf, depth);
    }
    backdrop::replace(pix, (lx, ly), &layer, mask.as_ref().map(|m| (m, (0, 0))));
}

/// Colour ink of a text run, in the order the run paints it. Strokes follow the outlines only: a
/// bitmap-strike glyph has no outline to stroke.
enum Ink {
    /// An embedded bitmap and the transform from its pixels into layout coordinates.
    Bitmap(Arc<figcraft_text::ColorBitmap>, Affine),
    /// One COLR layer: its outline in layout coordinates and the colour to fill it with.
    Layer(BezPath, [u8; 4]),
}

/// Draw a colour bitmap (premultiplied RGBA) under a transform, sampled smoothly when the
/// transform scales it.
fn blit(pix: &mut Pixmap, bitmap: &figcraft_text::ColorBitmap, transform: Transform) {
    let Some(image) = tiny_skia::PixmapRef::from_bytes(&bitmap.rgba, bitmap.width, bitmap.height) else { return };
    let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    pix.draw_pixmap(0, 0, image, &paint, transform, None);
}

fn is_axis_aligned(xf: Affine) -> bool {
    let c = xf.as_coeffs();
    c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && fract_ok(c[4]) && fract_ok(c[5])
}

fn fract_ok(_v: f64) -> bool {
    true
}

/// Whether all four edges of the device rectangle `r` lie on whole pixels, up to the float noise
/// `snap_px` forgives.
fn on_whole_pixels(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].into_iter().all(|v| (snap_px(v) - v.round()).abs() < 1e-9)
}
