// FigCraft — SOLID PROBE material.
//
// This is a diagnostic, not a real effect. It paints the whole node interior one flat, unmistakable
// colour (magenta) wherever the SDF coverage says "inside the shape". It carries NO edge-only
// design, NO backdrop refraction, NO blur — so if the shaded raster ever goes missing at some zoom,
// the cause is purely the render pipeline (ShadeRequest geometry, banding, placement, the scale
// handed to the GPU, the alpha blend), never the shader's own content.
//
// @fx id solid
// @fx label Solid Probe
// @fx param u.frost float 0 100 1 4 "Frost (ignored by this probe)"
//
struct Uniforms {
    resolution : vec2<f32>,
    light_angle: f32,
    light_int  : f32,
    refraction : f32,
    depth      : f32,
    dispersion : f32,
    frost      : f32,
    splay      : f32,
    dpr        : f32,
    nodeSizePx : vec2<f32>,
    regionOriginPx: vec2<f32>,
};

@group(0) @binding(0) var backdrop : texture_2d<f32>;
@group(0) @binding(1) var samp : sampler;
@group(0) @binding(2) var fields : texture_2d<f32>;
@group(0) @binding(3) var<uniform> u : Uniforms;

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
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

// Flat magenta, opaque only inside the node's coverage mask.
@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let f = textureSampleLevel(fields, samp, in.uv, 0.0);
    let coverage = f.a;
    let col = vec3<f32>(1.0, 0.0, 1.0);
    return vec4<f32>(col, coverage);
}
