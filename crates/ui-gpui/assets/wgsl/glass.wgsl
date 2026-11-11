// FigCraft — liquid glass / rounded-rect glass.
//
// The shading maths is a **line-for-line port of the engine's own CPU glass**
// (`figcraft_render::glass`), the implementation the built-in Glass effect draws with, so two panes
// with the same numbers must look the same whether the GPU material channel or the CPU renderer
// draws them. Every constant below comes from that file, not from a re-derivation:
//
//   bend_at()      mix(core, rim, u) — the thick-lens profile, `pow(t, 3.5)`, `RIM_BEND = 0.2`
//   rim_light_at() smoothstep(bevel .. 0.2·bevel)
//   catches()      smoothstep(0.35, 1, facing) + 0.6 · smoothstep(0.35, 1, -facing)
//   sheen_at()     the broad gradient, `BROAD = 0.05`
//   dispersed()    the Figma decomposition into red / green / blue taps
//
// ---------------------------------------------------------------------------------------------
// THE SEVEN CONTROLS — values and ranges, and where each comes from
// ---------------------------------------------------------------------------------------------
// The panel is Figma's Glass effect, so its fields are Figma's `GlassEffect` (Plugin API,
// developers.figma.com/docs/plugins/api/Effect → GlassEffect) and the numbers the Figma help
// article "Illustrate a flower vase … and the glass effect" itself uses in its example
// (66°, 60%, Refraction 76, Depth 98, Dispersion 100, Frost 0, Splay 100).
//
//   control           field             range        step   default  unit
//   Light Angle       lightAngle        -180 … 180   1      -45      degrees (y-up, 0 = up)
//   Light Intensity   lightIntensity    0 … 1        0.01   0.8      ratio (panel shows 0–100 %)
//   Refraction        refraction        0 … 1        0.01   0.8      ratio (panel shows 0–100)
//   Depth             depth             1 … 100      1      20       document px — the API says only
//                                                                     "must be >= 1"; 100 is where
//                                                                     Figma's own panel stops
//   Dispersion        dispersion        0 … 1        0.01   0.5      ratio (panel shows 0–100)
//   Frost             radius            0 … 100      1      4        blur radius in document px —
//                                                                     the API calls this `radius`,
//                                                                     not a 0–1 amount
//   Splay             (UI only)         0 … 1        0.01   0       ratio
//
// `depth` and `frost` are **lengths in document units**, not 0–1 sliders and not dimensionless
// counts. There is no coefficient between them and the shader: the render wasm copies
// `bevelSize`/`refractionDistance` into the uniform block with 179 write sites and **0** `f32.mul`
// (figma_depth_coefficient.html §0/§1), and it has no `depth` uniform at all. So the panel's Depth
// IS the pixel thickness, converted once by device-pixels-per-unit (`dpr`) and by nothing else. An
// earlier version of this file multiplied Depth by 1.3 and 0.25 — coefficients copied from a
// superseded demo page that the source material itself lists as a failed delivery. They are gone.
//
// ---------------------------------------------------------------------------------------------
// FROST — the original algorithm
// ---------------------------------------------------------------------------------------------
// Figma's frost is **not** computed inside the glass shader. It is a separate background pass:
// the backdrop is run through a downsample → upsample blur chain, and `gen_backdrop_composite.wgsl`
// only samples that finished texture, dithers it by 2/255 (its `d = 0.007843`) and composites it
// under the mask (figma_effects_solidified.html §②, "已坐实 · 真实源码").
//
// "Downsample to level L, sample it bilinearly" is exactly that chain: a 2×2 box reduction per
// level is the downsample, the sampler's linear filter is the upsample. The host uploads the whole
// mip chain of the backdrop, so this shader picks a level — `log2(radius)` levels for a radius of
// `2^L` pixels — instead of running a fixed ring of taps, which was a stand-in that does not exist
// anywhere in Figma's pipeline. The 2/255 dither then removes the banding the reduction creates,
// with the same hash and the same amplitude as Figma's.
//
// @fx id glass
// @fx label Glass
// NOTE: @fx param tokens must be separated by exactly one space. `parse_param` uses
// `splitn(6, char::is_whitespace)` *before* filtering empty tokens, so alignment padding
// silently shifts columns and drops the whole declaration into `extra_fields` (defaults 0).
// @fx param u.light_angle float -180 180 1 -45 "Light Angle (deg)"
// @fx param u.light_int float 0 1 0.01 0.8 "Light Intensity"
// @fx param u.refraction float 0 1 0.01 0.8 "Refraction"
// @fx param u.depth float 1 100 1 20 "Depth"
// @fx param u.dispersion float 0 1 0.01 0.5 "Dispersion"
// @fx param u.frost float 0 100 1 4 "Frost"
// @fx param u.splay float 0 1 0.01 0 "Splay"
//
// The host uploads, per node region:
//   backdrop    : the layer's **backdrop** (the already-rendered canvas under the node, with this
//                 node itself excluded), straight RGBA, sized to the region, **with its full mip
//                 chain** — that chain is the frost blur;
//   fields      : RGBA16F per pixel —
//                   R  = signed distance to the shape's edge in device pixels, negative inside
//                        (the reference lens reads exactly this: `eh = 0` on the border);
//                   GB = inward unit normal, encoded as 0.5 * (n + 1);
//                   A  = coverage (0 outside the shape .. 1 inside);
//   u           : the seven controls above + the generic exports below.
//
// The distance must be a float, not an 8-bit normalised edge factor: a 2000 px pane would quantise
// its distance into steps of several pixels, and a 1 px bevel would vanish into a staircase.

struct Uniforms {
    resolution : vec2<f32>,   // window size in device pixels (the texture size; generic export)
    light_angle: f32,         // degrees, y-up, 0 = up (panel -180..180)
    light_int  : f32,         // 0..1
    refraction : f32,         // 0..1
    depth      : f32,         // document px, 1..100 — the lens's reach, not a slider fraction
    dispersion : f32,         // 0..1
    frost      : f32,         // document px, 0..100 — the backdrop blur *radius*
    splay      : f32,         // 0..1
    dpr        : f32,         // device px per document unit (generic export)
    nodeSizePx : vec2<f32>,   // the node's own size in device pixels (generic export)
    regionOriginPx: vec2<f32>,// where this window's top-left sits inside the node (generic export)
};

@group(0) @binding(0) var backdrop : texture_2d<f32>;
@group(0) @binding(1) var samp : sampler;
@group(0) @binding(2) var fields : texture_2d<f32>;
@group(0) @binding(3) var<uniform> u : Uniforms;

/// The rim band's own depth, in document units — the CPU renderer's `BEVEL`. It is constant: the
/// panel's Depth moves the *reach*, not the band (that is what `bevelSize = Depth * 0.25` got
/// wrong; no such mapping exists in the reference).
const BEVEL_DOC: f32 = 1.0;
/// `PROFILE_EXP` — how sharply the bend falls off going in.
const PROFILE_EXP: f32 = 3.5;
/// The furthest the backdrop moves, as a share of `depth` (the reference's `0.999`).
const MAX_BEND: f32 = 0.999;
/// The bend on the bevel itself, as a share of `MAX_BEND * depth * refraction` (the reference's `cs`).
const RIM_BEND: f32 = 0.2;
/// The least an edge faces the light and still catches some.
const HL_LO: f32 = 0.35;
/// How much of the light the side opposite it catches.
const HL_SECONDARY: f32 = 0.6;
/// The gradient across the curve at its strongest, a share of the light's intensity.
const BROAD: f32 = 0.05;
/// How far `dispersion` splits the colour channels, as a share of the bend.
const DISPERSION_SPREAD: f32 = 0.16;
/// The share of the reach the bend-direction difference is taken over.
const KSTEP: f32 = 0.5;
/// Figma's dither amplitude: `0.007843 = 2/255` (`gen_backdrop_composite.wgsl`'s `d`).
const DITHER: f32 = 0.007843;

struct VSOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> VSOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out : VSOut;
    let xy = corners[vi];
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // UV in 0..1, y down, to match the texture's row order.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

fn safe_normalize(v : vec2<f32>) -> vec2<f32> {
    let l = length(v);
    return select(vec2<f32>(0.0, 0.0), v / l, l > 1e-6);
}

fn decode_normal(enc : vec2<f32>) -> vec2<f32> {
    return enc * 2.0 - vec2<f32>(1.0, 1.0);
}

/// `x` from `a` to `b` as 0 to 1, clamped (a span under 1e-5 counts as 1e-5) — the CPU path's `lin`.
fn lin(a : f32, b : f32, x : f32) -> f32 {
    return clamp((x - a) / max(b - a, 1e-5), 0.0, 1.0);
}

fn texel_uv(uv : vec2<f32>) -> vec2<f32> {
    let t = 0.5 / max(u.resolution, vec2<f32>(1.0, 1.0));
    return clamp(uv, t, vec2<f32>(1.0, 1.0) - t);
}

/// The backdrop at `uv`, sampled at mip level `lod`. Level `L` is the backdrop reduced by `2^L`
/// (a 2×2 box each level) and the sampler's linear filter is the upsample, so this one call is
/// Figma's downsample → upsample blur chain; `lod = 0` is the untouched backdrop.
fn sample_backdrop(uv : vec2<f32>, lod : f32) -> vec4<f32> {
    // The host only builds as many levels as a frost radius can reach, so ask the texture instead
    // of assuming twelve: sampling a level that does not exist would be undefined.
    let max_lod = f32(max(textureNumLevels(backdrop), 1u) - 1u);
    return textureSampleLevel(backdrop, samp, texel_uv(uv), clamp(lod, 0.0, max_lod));
}

/// Figma's 2/255 dither, its own hash (`fract(sin(dot(p, (12.9898, 78.233))) * 43758.547)`), applied
/// to a straight-alpha colour. It removes the banding the blur chain leaves in flat areas.
fn ditherize(c : vec3<f32>, frag : vec2<f32>) -> vec3<f32> {
    let h = fract(sin(dot(frag, vec2<f32>(12.9898, 78.233))) * 43758.547);
    let o = (h - 0.5) * DITHER;
    return min(abs(c + vec3<f32>(o, o, o)), vec3<f32>(1.0, 1.0, 1.0));
}

/// The blur level a frost radius (device px) asks for: a level halves the backdrop's resolution, so
/// a radius of `2^L` px is level `L`. Radius 1 (or less) is the backdrop itself.
fn frost_lod(radius_px : f32) -> f32 {
    return log2(max(radius_px, 1.0));
}

/// The lens profile: how far the backdrop moves `d` pixels in from the edge, for a `depth` and a
/// `bevel` (pixels) and `refraction` (0–1). Port of the CPU `bend_at`: the strongest a bevel in
/// from the edge, falling off as the 3.5th power of the way out of the curve to nothing `depth`
/// further in, and a fifth as much on the bevel itself, eased between the two over its outer 70%.
fn bend_at(d : f32, depth : f32, bevel : f32, refraction : f32) -> f32 {
    if (d >= max(depth, 1e-4) + bevel) {
        return 0.0;
    }
    let t = lin(-depth - bevel, -bevel, -d);
    let most = MAX_BEND * depth;
    let core = clamp(pow(t, PROFILE_EXP) * depth * refraction, -most, most);
    let rim = refraction * RIM_BEND * most;
    let e = lin(-bevel - 1e-4, -0.3 * bevel, -d);
    return core + (rim - core) * e;
}

/// How much of the rim light falls `d` pixels in from the edge: all of it within a fifth of the
/// bevel, none a bevel in.
fn rim_light_at(d : f32, bevel : f32) -> f32 {
    return 1.0 - smoothstep(0.2 * bevel, bevel, d);
}

/// How much light an edge catches, `facing` being the cosine between its outward normal and the
/// light, plus 60% as much on the side facing away.
fn catches(facing : f32) -> f32 {
    return smoothstep(HL_LO, 1.0, facing) + HL_SECONDARY * smoothstep(HL_LO, 1.0, -facing);
}

/// How strongly the gradient across the curve shows `d` pixels in from the edge: fully on the
/// bevel, falling as the square of the way in to none at `depth`.
fn sheen_at(d : f32, depth : f32, bevel : f32) -> f32 {
    let t = lin(-depth, -bevel + 1e-4, -d);
    return t * t;
}

/// The direction the pane bends towards: towards the **deepest pixel within the lens's reach**, not
/// along the edge normal.
///
/// The CPU renderer does exactly this (`bend_directions`): every inside pixel points at the nearest
/// pixel of the shape's core — the set of pixels at least `reach` in from the edge — and the result
/// is smoothed. On a straight edge the two agree, but near a corner the core is *diagonally* inside,
/// so the bend there pulls towards the middle of the pane instead of straight in from the edge. A
/// plain edge normal gets that wrong and tears the corner.
///
/// Generic: it reads only the shape's distance, so rounded rects, text, vectors, ellipses and
/// booleans all bend the same way, and the host still uploads geometry only.
fn core_dir(uv : vec2<f32>, reach : f32, texel : vec2<f32>, sd0 : f32, n_in : vec2<f32>) -> vec2<f32> {
    // Central difference across the lens's own reach: exact on a straight edge (the outside of the
    // shape does not bias it, as it does a one-sided tap), and turning through the diagonal only
    // near a corner — the rounding the CPU renderer gets by smoothing its normals over `reach`.
    let s = reach * KSTEP * texel;
    let gx = textureSampleLevel(fields, samp, texel_uv(uv + vec2<f32>(s.x, 0.0)), 0.0).r
           - textureSampleLevel(fields, samp, texel_uv(uv - vec2<f32>(s.x, 0.0)), 0.0).r;
    let gy = textureSampleLevel(fields, samp, texel_uv(uv + vec2<f32>(0.0, s.y)), 0.0).r
           - textureSampleLevel(fields, samp, texel_uv(uv - vec2<f32>(0.0, s.y)), 0.0).r;
    // `sd` falls going inwards, so the inward direction is minus its gradient.
    let inward = vec2<f32>(-gx, -gy);
    if (length(inward) < reach * 0.05) {
        return n_in;
    }
    return safe_normalize(inward);
}

/// The backdrop seen at `uv` moved by `off`, its channels split by `k` (dispersion, 0-1): red moves
/// up to `k` further out, blue up to `k` less, each smeared along the move by a triangular window of
/// seven steps. The CPU renderer's `dispersion_window`, same steps and weights — the shader keeps
/// exactly the same channel separation, so a striped backdrop lands on the same pixels. Every tap
/// reads the frosted level, because the CPU path blurs the backdrop once, before it samples it.
fn dispersed(uv : vec2<f32>, off : vec2<f32>, k : f32, lod : f32) -> vec3<f32> {
    if (k < 0.001) {
        return sample_backdrop(uv + off, lod).rgb;
    }
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    var wsum = 0.0;
    for (var i = 1; i < 7; i = i + 1) {
        let t = f32(i) / 7.0 - 0.5;
        let w = 1.0 - abs(2.0 * t);
        let r = sample_backdrop(uv + off * (1.0 + (t + 0.5) * k), lod).r;
        let g = sample_backdrop(uv + off * (1.0 + t * k), lod).g;
        let b = sample_backdrop(uv + off * (1.0 + (t - 0.5) * k), lod).b;
        acc = acc + vec3<f32>(r, g, b) * w;
        wsum = wsum + w;
    }
    return acc / max(wsum, 1e-4);
}

@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let fld = textureSampleLevel(fields, samp, in.uv, 0.0);
    let coverage = fld.a;
    if (coverage <= 0.001) {
        discard;
    }

    // `R` is the reference SDF: 0 on the border, negative inside.
    let eh = fld.r;
    let d = -eh;                                   // pixels in from the edge, positive inside
    let n_in = safe_normalize(decode_normal(fld.gb));

    let scale = max(u.dpr, 1e-4);
    // Document units → device pixels, once. Depth is a length (1..100 doc px), the bevel is the
    // renderer's fixed one-unit band, and Frost is a radius (0..100 doc px). No other coefficient.
    let depth = clamp(u.depth, 1.0, 100.0) * scale;
    let bevel = max(BEVEL_DOC * scale, 1e-4);
    let texel = 1.0 / max(u.resolution, vec2<f32>(1.0, 1.0));
    let lod = frost_lod(clamp(u.frost, 0.0, 100.0) * scale);

    let frag_px = u.regionOriginPx + in.uv * max(u.resolution, vec2<f32>(1.0, 1.0));

    // Past the curve nothing bends and no light falls: the frosted backdrop, dithered.
    if (d >= depth + bevel) {
        let flat = ditherize(sample_backdrop(in.uv, lod).rgb, frag_px);
        return vec4<f32>(flat, coverage);
    }

    // The curve's outward direction: away from its core, turned by `splay` towards the way away
    // from the layer's centre. Not renormalised (the CPU path does not): where the two part, the
    // bend is less.
    let toward = core_dir(in.uv, depth + bevel, texel, eh, n_in);
    let centre = 0.5 * u.nodeSizePx;
    let rel = frag_px - centre;
    let away = safe_normalize(vec2<f32>(rel.x, rel.y));
    var out_dir = -toward;
    if (u.splay > 0.0) {
        out_dir = -toward * (1.0 - u.splay) + away * u.splay;
    }

    let bend = bend_at(d, depth, bevel, clamp(u.refraction, 0.0, 1.0));
    // The backdrop is sampled **inwards** from where the pixel is: the lens pulls the outside in.
    let shift_px = -out_dir * bend;                // device px
    let off = shift_px * texel;

    // Dispersion: red samples a little further out, blue a little less, smeared along the move by
    // the CPU path's own seven-step triangular window (`DISPERSION_SPREAD` is the same constant).
    let k = select(0.0, clamp(u.dispersion, 0.0, 1.0) * DISPERSION_SPREAD, bend > 0.0);
    let col = ditherize(dispersed(in.uv, off, k, lod), frag_px);

    // Light: `light` is the CPU path's (sin, -cos) of the angle, 0 pointing up.
    let ang = u.light_angle * 0.0174532925;
    let light = vec2<f32>(sin(ang), -cos(ang));
    let facing = -(n_in.x * light.x + n_in.y * light.y);
    let rim = rim_light_at(d, bevel) * catches(facing);
    let sheen = -(out_dir.x * light.x + out_dir.y * light.y) * sheen_at(d, depth, bevel) * BROAD;
    let add = clamp(u.light_int, 0.0, 1.0) * (rim + sheen);

    // Straight (non-premultiplied) output: the runner composites with ALPHA_BLENDING.
    return vec4<f32>(col + vec3<f32>(add), coverage);
}
