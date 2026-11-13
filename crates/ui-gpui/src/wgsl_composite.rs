//! Offscreen WGSL material compositor for the GPUI canvas (`--features gpu` only).
//!
//! GPUI's main canvas path is CPU tiny-skia: `render_worker::run` produces a full-page
//! `Arc<RenderImage>` in **straight BGRA** bytes. This module is the GPUI counterpart of the
//! egui line's `wgsl_channel.rs`: it takes that CPU-rendered canvas as `sceneTex` and runs each
//! bound node's WGSL material chain offscreen, then returns composited straight BGRA bytes.
//!
//! # Pipeline (per bound node, mirroring the HTML demo / egui `WgslPass`)
//!
//! ```text
//! whole CPU canvas ──upload──▶ scene_tex (Rgba8Unorm, shared sceneTex)
//!   scene_tex ──copy──▶ out_tex (the composited canvas, seeded with the scene)
//!   fs_tile  (T over scene)  ──▶ tile  (TILE_PX²)
//!   fs_blurH (blurIn = tile)  ──▶ tmp   (TILE_PX²)
//!   fs_blurV (blurIn = tmp)   ──▶ frost (TILE_PX²)
//!   fs_glass (sceneTex = frost, T group = scene) ──▶ out_tex, viewport+scissor = node rect
//! ```
//!
//! The glass pass is drawn into `out_tex` with a viewport + scissor set to the node's on-screen
//! rect — exactly what `egui_wgpu` does for a `Shape::Callback` — and standard source-over alpha
//! blending. The result is read back as straight RGBA, then converted to straight BGRA for
//! `wrap_bgra`. One shared `scene_tex` means a single scene upload per frame; only U/F/T uniform
//! buffers are per-node (the shared registry would otherwise bleed params between nodes).
//!
//! # Why offscreen + readback
//!
//! `gpui 0.2` does not expose its wgpu device / a `Shape::Callback` equivalent to app code, so a
//! zero-readback material callback is impossible without patching gpui. This is the only viable
//! route: CPU scene → GPU material → readback → the existing `Arc<RenderImage>` display path.
//! It is gated behind `--features gpu` and degrades to the untouched CPU image on any failure
//! (no adapter, unparseable shader, validation error) — the canvas never shows half-broken pixels.
#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use figcraft_doc::{Document, NodeId};
use figcraft_render::wgsl_effect as fx;

/// Fullscreen-triangle backdrop coverage, in multiples of the node box (egui `TILE_M`).
const TILE_M: f64 = 1.5;
const TILE_PX: u32 = 512;
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BEVEL_SIZE: f64 = 1.0;
const TILE_EXPANSION_SCALE: f64 = 1.0;
const MAX_BLUR_PX: f64 = 24.0;
const MAX_BLUR_TAPS: f64 = 64.0;

/// One component's material pass, handed to [`WgslComposite::composite`] by the render worker.
///
/// Geometry is in **document units** (zoom-independent); `rect_px` is the on-screen device-pixel
/// rectangle used for the glass pass viewport + scissor. `u` / `f` / `t` are the packed uniform
/// buffers (`U` from the scanned controls + engine exports, `F` / `T` the demo's fixed uniforms).
#[derive(Clone, Debug)]
pub struct MaterialPass {
    /// Registry id (`effect_id`) this node's shader is loaded under.
    pub effect: String,
    /// The selected node (for the per-(effect,node) GPU cache key).
    pub node: NodeId,
    /// U uniform bytes, packed with this node's own params + exports.
    pub u: Vec<u8>,
    /// F uniform: `[quad.xyzw, blurDelta.xy, count, 0]` as f32 bytes.
    pub f: Vec<u8>,
    /// T uniform: tile offset.xy / scale.xy over the whole canvas, as f32 bytes.
    pub t: Vec<u8>,
    /// Glass render viewport/scissor in device pixels: `[x, y, w, h]`.
    pub rect_px: [u32; 4],
}

/// Extracted shader description: entry point names, `@group/@binding` declarations and uniform
/// sizes. Host-agnostic — read straight from [`fx::Scan`] so the GPU code never parses WGSL text.
#[derive(Clone, Debug)]
struct Bundle {
    hash: u64,
    code: String,
    vertex: String,
    groups: Vec<(u32, Vec<fx::Binding>)>,
    uniforms: Vec<(String, usize)>,
    tile: Option<String>,
    blur_h: Option<String>,
    blur_v: Option<String>,
    glass: String,
}

fn snapshot(id: &str) -> Option<Bundle> {
    use std::hash::{Hash, Hasher};
    let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
    let scan = reg.scan(id)?;
    let code = reg.source_code(id)?.to_string();
    if scan.entries.is_empty() {
        return None;
    }
    let vertex = scan.entries.iter().find(|e| e.stage == "vertex").map(|e| e.name.clone()).or_else(|| scan.generic_entries.first().cloned())?;
    let find = |name: &str| scan.entries.iter().find(|e| e.name == name).map(|e| e.name.clone());
    let glass = find("fs_glass").or_else(|| scan.extra_entries.first().cloned())?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    let mut groups: std::collections::BTreeMap<u32, Vec<fx::Binding>> = std::collections::BTreeMap::new();
    for b in &scan.bindings {
        groups.entry(b.group).or_default().push(b.clone());
    }
    let uniforms = scan.uniforms.iter().map(|(k, v)| (k.clone(), v.size.max(16))).collect();
    Some(Bundle {
        hash: hasher.finish(),
        code,
        vertex,
        groups: groups.into_iter().collect(),
        uniforms,
        tile: find("fs_tile"),
        blur_h: find("fs_blurH"),
        blur_v: find("fs_blurV"),
        glass,
    })
}

/// GPU resources for one (effect, node): pipelines, uniform buffers, tile/blur textures and bind
/// groups. The whole-canvas `sceneTex` is shared (owned by [`WgslComposite`]); per-node state is
/// only the U/F/T buffers and their bind groups.
struct Gpu {
    hash: u64,
    canvas_size: (u32, u32),
    u_buf: wgpu::Buffer,
    f_buf: wgpu::Buffer,
    t_buf: wgpu::Buffer,
    tile_view: wgpu::TextureView,
    tmp_view: wgpu::TextureView,
    frost_view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
    _layouts: Vec<wgpu::BindGroupLayout>,
    _textures: Vec<wgpu::Texture>,
    _layout: wgpu::PipelineLayout,
    bg_tile: wgpu::BindGroup,
    bg_blur_h: Option<wgpu::BindGroup>,
    bg_blur_v: Option<wgpu::BindGroup>,
    bg_g1_frost: Option<wgpu::BindGroup>,
    bg_glass: wgpu::BindGroup,
    bg_t: Option<wgpu::BindGroup>,
    p_tile: Option<wgpu::RenderPipeline>,
    p_blur_h: Option<wgpu::RenderPipeline>,
    p_blur_v: Option<wgpu::RenderPipeline>,
    p_glass: wgpu::RenderPipeline,
}

fn tile_texture(device: &wgpu::Device, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: TILE_PX, height: TILE_PX, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TILE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn bind_entries<'a>(
    bs: &[fx::Binding],
    view: &'a wgpu::TextureView,
    u_buf: &'a wgpu::Buffer,
    f_buf: &'a wgpu::Buffer,
    t_buf: &'a wgpu::Buffer,
    sampler: &'a wgpu::Sampler,
) -> Vec<wgpu::BindGroupEntry<'a>> {
    bs.iter()
        .filter_map(|binding| {
            let resource = if binding.uniform {
                let buf = match binding.name.as_str() {
                    "U" => Some(u_buf),
                    "F" => Some(f_buf),
                    "T" => Some(t_buf),
                    _ => None,
                }?;
                buf.as_entire_binding()
            } else if binding.ty.contains("texture_2d") {
                wgpu::BindingResource::TextureView(view)
            } else if binding.ty.trim() == "sampler" {
                wgpu::BindingResource::Sampler(sampler)
            } else {
                return None;
            };
            Some(wgpu::BindGroupEntry { binding: binding.binding, resource })
        })
        .collect()
}

fn target(format: wgpu::TextureFormat, blend: bool) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState { format, blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL }
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    vertex: &str,
    fragment: &str,
    format: wgpu::TextureFormat,
    blend: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(layout),
        vertex: wgpu::VertexState { module, entry_point: Some(vertex), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(target(format, blend))],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn build(device: &wgpu::Device, b: &Bundle, canvas_view: &wgpu::TextureView, canvas: (u32, u32)) -> Result<Gpu, String> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figcraft wgsl material"),
        source: wgpu::ShaderSource::Wgsl(b.code.clone().into()),
    });
    let tile = tile_texture(device, "wgsl tile");
    let tmp = tile_texture(device, "wgsl tmp");
    let frost = tile_texture(device, "wgsl frost");
    let tile_view = tile.create_view(&wgpu::TextureViewDescriptor::default());
    let tmp_view = tmp.create_view(&wgpu::TextureViewDescriptor::default());
    let frost_view = frost.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    // The host always writes U/F/T with fixed arities (U = shader layout ≥16, F = 8 f32, T = 4 f32).
    // Size each buffer to the max of the shader's declared layout and what the host writes, so a
    // shader declaring a smaller F/T can't make `write_buffer` go out of bounds.
    let size_of = |var: &str, floor: u64| b.uniforms.iter().find(|(v, _)| v == var).map(|(_, s)| (*s as u64).max(floor)).unwrap_or(floor);
    let buffer = |name: &str, size: u64| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(name),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let u_buf = buffer("U", size_of("U", 16));
    let f_buf = buffer("F", size_of("F", 32));
    let t_buf = buffer("T", size_of("T", 16));

    let max_group = b.groups.iter().map(|(g, _)| *g).max().unwrap_or(0);
    let mut layouts = Vec::new();
    for g in 0..=max_group {
        let entries: Vec<wgpu::BindGroupLayoutEntry> = b
            .groups
            .iter()
            .find(|(k, _)| *k == g)
            .map(|(_, bs)| {
                bs.iter()
                    .filter_map(|binding| {
                        let ty = if binding.uniform {
                            wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
                        } else if binding.ty.contains("texture_2d") {
                            wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            }
                        } else if binding.ty.trim() == "sampler" {
                            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                        } else {
                            return None;
                        };
                        Some(wgpu::BindGroupLayoutEntry {
                            binding: binding.binding,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty,
                            count: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        layouts.push(device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("wgsl material group"), entries: &entries }));
    }
    let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wgsl material"),
        bind_group_layouts: &layout_refs,
        immediate_size: 0,
    });

    let g0: &[fx::Binding] = b.groups.iter().find(|(k, _)| *k == 0).map(|(_, bs)| bs.as_slice()).unwrap_or(&[]);
    let g1: &[fx::Binding] = b.groups.iter().find(|(k, _)| *k == 1).map(|(_, bs)| bs.as_slice()).unwrap_or(&[]);
    let g2: &[fx::Binding] = b.groups.iter().find(|(k, _)| *k == 2).map(|(_, bs)| bs.as_slice()).unwrap_or(&[]);
    let bg = |g: u32, entries: &[wgpu::BindGroupEntry<'_>]| -> Option<wgpu::BindGroup> {
        let l = layouts.get(g as usize)?;
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("wgsl material bind group"), layout: l, entries }))
    };
    let bg_tile = bg(0, &bind_entries(g0, canvas_view, &u_buf, &f_buf, &t_buf, &sampler)).ok_or("no group 0")?;
    let bg_glass = bg(0, &bind_entries(g0, &frost_view, &u_buf, &f_buf, &t_buf, &sampler)).ok_or("no group 0")?;
    let bg_blur_h = bg(1, &bind_entries(g1, &tile_view, &u_buf, &f_buf, &t_buf, &sampler));
    let bg_blur_v = bg(1, &bind_entries(g1, &tmp_view, &u_buf, &f_buf, &t_buf, &sampler));
    let bg_g1_frost = bg(1, &bind_entries(g1, &frost_view, &u_buf, &f_buf, &t_buf, &sampler));
    let bg_t = bg(2, &bind_entries(g2, canvas_view, &u_buf, &f_buf, &t_buf, &sampler));

    let p_tile = b.tile.as_deref().map(|f| pipeline(device, &module, &layout, &b.vertex, f, TILE_FORMAT, false));
    let p_blur_h = b.blur_h.as_deref().map(|f| pipeline(device, &module, &layout, &b.vertex, f, TILE_FORMAT, false));
    let p_blur_v = b.blur_v.as_deref().map(|f| pipeline(device, &module, &layout, &b.vertex, f, TILE_FORMAT, false));
    let p_glass = pipeline(device, &module, &layout, &b.vertex, &b.glass.clone(), TILE_FORMAT, true);

    Ok(Gpu {
        hash: b.hash,
        canvas_size: canvas,
        u_buf,
        f_buf,
        t_buf,
        tile_view,
        tmp_view,
        frost_view,
        _sampler: sampler,
        _layouts: layouts,
        _textures: vec![tile, tmp, frost],
        _layout: layout,
        bg_tile,
        bg_blur_h,
        bg_blur_v,
        bg_g1_frost,
        bg_glass,
        bg_t,
        p_tile,
        p_blur_h,
        p_blur_v,
        p_glass,
    })
}

/// Shared whole-canvas `sceneTex`, rebuilt only when the size changes.
struct Scene {
    w: u32,
    h: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Full-canvas output texture + readback buffer, reused across frames (size-keyed rebuild).
struct Out {
    w: u32,
    h: u32,
    padded: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// The offscreen compositor. Owns one wgpu device/queue, a shared `sceneTex`, and a cache of
/// per-(effect,node) pipelines (keyed by shader hash + canvas size) reused across frames.
pub struct WgslComposite {
    device: wgpu::Device,
    queue: wgpu::Queue,
    cache: HashMap<(String, u64), Gpu>,
    scene: Option<Scene>,
    out: Option<Out>,
    adapter_info: wgpu::AdapterInfo,
}

fn f32_bytes(values: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&(*v as f32).to_le_bytes());
    }
    out
}

impl WgslComposite {
    /// Create the device (allows software fallback). `None` = no adapter; caller stays on CPU.
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
            label: Some("figcraft-wgsl-composite"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;
        Some(Self { device, queue, cache: HashMap::new(), scene: None, out: None, adapter_info })
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    fn ensure_scene(&mut self, w: u32, h: u32) {
        if self.scene.as_ref().is_some_and(|s| s.w == w && s.h == h) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgsl sceneTex"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.scene = Some(Scene { w, h, texture, view });
    }

    fn ensure_out(&mut self, w: u32, h: u32) {
        if self.out.as_ref().is_some_and(|o| o.w == w && o.h == h) {
            return;
        }
        let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgsl output canvas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wgsl output readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        self.out = Some(Out { w, h, padded, texture, view, readback });
    }

    /// Composite all material passes over the CPU canvas. `scene_rgba` is straight RGBA
    /// (`w*h*4`, row-major top-down). Returns composited straight RGBA on success, `None` on
    /// any failure (caller keeps the CPU image).
    pub fn composite(&mut self, scene_rgba: &[u8], w: u32, h: u32, passes: &[MaterialPass]) -> Option<Vec<u8>> {
        if passes.is_empty() || w == 0 || h == 0 || scene_rgba.len() < (w as usize) * (h as usize) * 4 {
            return None;
        }
        let expected = (w as usize) * (h as usize) * 4;
        self.ensure_scene(w, h);
        self.ensure_out(w, h);

        // 1. Upload the CPU scene once into the shared sceneTex, then copy it into out_tex so the
        //    glass pass composites over the already-rendered canvas.
        {
            let scene_view = self.scene.as_ref()?;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &scene_view.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &scene_rgba[..expected],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            let out = self.out.as_ref()?;
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("wgsl seed canvas") });
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &scene_view.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo { texture: &out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            self.queue.submit(Some(enc.finish()));
        }

        // 2. Build / refresh every (effect,node) pipeline against the shared sceneTex view.
        //    Collect clones of the immutable handles first, then mutate the cache — no borrow fight.
        {
            let scene_view = self.scene.as_ref()?.view.clone();
            let bundles: Vec<(String, u64, Bundle)> = passes
                .iter()
                .filter_map(|p| {
                    let key = (p.effect.clone(), p.node.0);
                    let stale = self.cache.get(&key).is_none_or(|g| g.canvas_size != (w, h));
                    if stale { snapshot(&p.effect).map(|b| (p.effect.clone(), p.node.0, b)) } else { None }
                })
                .collect();
            for (effect, node, bundle) in bundles {
                let gpu = build(&self.device, &bundle, &scene_view, (w, h)).ok()?;
                self.cache.insert((effect, node), gpu);
            }
            // Drop GPU bundles whose shader changed on disk (hash mismatch) — rebuild next frame.
            let mut stale = Vec::new();
            for (key, g) in &self.cache {
                let hash = snapshot(&key.0).map(|b| b.hash);
                if hash.is_none_or(|hsh| hsh != g.hash) {
                    stale.push(key.clone());
                }
            }
            for key in stale {
                self.cache.remove(&key);
            }
        }

        // 3. Per node: upload U/F/T, run tile → blurH → blurV, then the glass pass into out_tex
        //    clipped to the node rect. One encoder + submit per node keeps buffer writes ordered.
        for p in passes {
            let key = (p.effect.clone(), p.node.0);
            let Some(g) = self.cache.get(&key) else { continue };
            if !p.u.is_empty() {
                self.queue.write_buffer(&g.u_buf, 0, &p.u);
            }
            if !p.f.is_empty() {
                self.queue.write_buffer(&g.f_buf, 0, &p.f);
            }
            if !p.t.is_empty() {
                self.queue.write_buffer(&g.t_buf, 0, &p.t);
            }

            let out = self.out.as_ref()?;
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("wgsl material pass") });
            if let (Some(pipe), Some(g1)) = (&g.p_tile, &g.bg_g1_frost) {
                let mut groups = vec![(0u32, &g.bg_tile), (1, g1)];
                if let Some(t) = &g.bg_t {
                    groups.push((2, t));
                }
                draw_pass(&mut enc, pipe, &g.tile_view, &groups);
            }
            if let (Some(pipe), Some(g1)) = (&g.p_blur_h, &g.bg_blur_h) {
                let mut groups = vec![(0u32, &g.bg_tile), (1, g1)];
                if let Some(t) = &g.bg_t {
                    groups.push((2, t));
                }
                draw_pass(&mut enc, pipe, &g.tmp_view, &groups);
            }
            if let (Some(pipe), Some(g1)) = (&g.p_blur_v, &g.bg_blur_v) {
                let mut groups = vec![(0u32, &g.bg_tile), (1, g1)];
                if let Some(t) = &g.bg_t {
                    groups.push((2, t));
                }
                draw_pass(&mut enc, pipe, &g.frost_view, &groups);
            }
            // Glass: viewport + scissor = node rect, blending over the seeded canvas.
            {
                let [rx, ry, rw, rh] = p.rect_px;
                let x = rx.min(w.saturating_sub(1));
                let y = ry.min(h.saturating_sub(1));
                let vw = rw.min(w.saturating_sub(x)).max(1);
                let vh = rh.min(h.saturating_sub(y)).max(1);
                let mut groups = vec![(0u32, &g.bg_glass)];
                if let Some(g1) = &g.bg_g1_frost {
                    groups.push((1, g1));
                }
                if let Some(g2) = &g.bg_t {
                    groups.push((2, g2));
                }
                let att = [Some(wgpu::RenderPassColorAttachment {
                    view: &out.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })];
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("wgsl glass pass"),
                    color_attachments: &att,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_viewport(x as f32, y as f32, vw as f32, vh as f32, 0.0, 1.0);
                rp.set_scissor_rect(x, y, vw, vh);
                rp.set_pipeline(&g.p_glass);
                for (index, group) in groups {
                    rp.set_bind_group(index, group, &[]);
                }
                rp.draw(0..3, 0..1);
            }
            self.queue.submit(Some(enc.finish()));
        }

        self.readback(w, h)
    }

    fn readback(&self, w: u32, h: u32) -> Option<Vec<u8>> {
        let out = self.out.as_ref()?;
        let unpadded = w * 4;
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("wgsl readback") });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &out.readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(out.padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(enc.finish()));
        let slice = out.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let _ = rx.recv();
        let mapped = slice.get_mapped_range().ok()?;
        let mut data = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let start = (row * out.padded) as usize;
            data.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        out.readback.unmap();
        Some(data)
    }
}

fn draw_pass<'a>(enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, view: &'a wgpu::TextureView, groups: &[(u32, &'a wgpu::BindGroup)]) {
    let att = [Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
    })];
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("wgsl tile/blur pass"),
        color_attachments: &att,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipe);
    for (index, group) in groups {
        rp.set_bind_group(*index, *group, &[]);
    }
    rp.draw(0..3, 0..1);
}

fn map(m: figcraft_geom::Affine, x: f64, y: f64) -> (f64, f64) {
    let p = m * figcraft_geom::Point::new(x, y);
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

/// Compute the per-node material passes for a rendered page: enumerate bound + visible nodes in
/// `page`, derive document-unit geometry and screen rects, and pack each node's isolated `U` plus
/// the demo's `F` / `T` uniforms. Pure host-side math — no GPU — so it is unit-testable.
pub fn passes_for(doc: &Document, page: NodeId, view: &figcraft_render::View, time: f64) -> Vec<MaterialPass> {
    use figcraft_doc::NodeKind;
    let (cw, ch) = (f64::from(view.width), f64::from(view.height));
    if cw < 1.0 || ch < 1.0 {
        return Vec::new();
    }
    let (ox, oy, sc) = (view.origin.0, view.origin.1, view.scale.max(1e-6));
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

        let mut ex = fx::EngineExports::default();
        ex.set("nodeSize", vec![w, h]);
        ex.set("quad", vec![w / 2.0, h / 2.0, cx, cy]);
        ex.set("canvasSize", vec![canvas_w, canvas_h]);
        ex.set("resolution", vec![canvas_w, canvas_h]);
        ex.set("canvasToNode", vec![canvas_w / w.max(1e-6)]);
        ex.set("nodeOrigin", vec![r.x0, r.y0]);
        ex.set("nodeSpaceCenterPoint", vec![cx, cy]);
        ex.set("cornerRadii", radii.to_vec());
        ex.set("cornerSmoothings", vec![smoothing; 4]);
        ex.set("tileExpansionScale", vec![TILE_EXPANSION_SCALE]);
        ex.set("maskTextureSize", vec![1.0]);
        ex.set("hasUniformCorners", vec![if uniform_corners { 1.0 } else { 0.0 }]);
        ex.set("bevelSize", vec![BEVEL_SIZE]);
        ex.set("directComposite", vec![1.0]);
        ex.set("textureMaskDirectComposite", vec![0.0]);
        ex.set("frameRotation", vec![0.0]);
        ex.set("scale", vec![1.0]);
        ex.set("dpr", vec![sc]);
        ex.set("time", vec![time]);

        let (effect, u, frost) = crate::wgsl_material::pack_node_uniforms(&binding, &ex);

        // T: tile box over the whole canvas texture, normalised 0..1.
        let tx = r.x0 + w / 2.0 - w * TILE_M / 2.0;
        let ty = r.y0 + h / 2.0 - h * TILE_M / 2.0;
        let t = [((tx - ox) * sc) / cw, ((ty - oy) * sc) / ch, (w * TILE_M * sc) / cw, (h * TILE_M * sc) / ch];
        // F: quad.xyzw, separable-blur delta.xy, tap count, dummy.
        let radius = (frost / 100.0) * MAX_BLUR_PX;
        let count = radius.ceil().clamp(1.0, MAX_BLUR_TAPS);
        let (tile_w, tile_h) = (w * TILE_M, h * TILE_M);
        let f = [w / 2.0, h / 2.0, cx, cy, radius / count / tile_w, radius / count / tile_h, count, 0.0];

        // Screen rect in device pixels (same transform the CPU renderer used).
        let (x0, y0) = map(xf_view, r.x0, r.y0);
        let (x1, y1) = map(xf_view, r.x1, r.y1);
        let (sx0, sy0) = (x0.min(x1).max(0.0), y0.min(y1).max(0.0));
        let (sx1, sy1) = (x0.max(x1), y0.max(y1));
        if sx1 - sx0 <= 1.0 || sy1 - sy0 <= 1.0 {
            continue;
        }
        let rect_px = [sx0 as u32, sy0 as u32, (sx1 - sx0) as u32, (sy1 - sy0) as u32];

        out.push(MaterialPass { effect, node: id, u, f: f32_bytes(&f), t: f32_bytes(&t), rect_px });
    }
    out
}

// ───────────────────────────────────────────────────────────────────────────────────────────────
// GPU 像素测试：`--features gpu` 且本机有适配器时，真跑一遍 WGSL 多 pass + readback，断言
// 材质只改绑定节点的矩形区域、其余画布逐像素不变。shader 文本为本测试自写，不取自任何产品。
// ───────────────────────────────────────────────────────────────────────────────────────────────
#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use super::*;

    /// 一个遵循通用契约的最小玻璃 shader：fullscreen 三角形；fs_tile 把 sceneTex 按 T 裁成
    /// 平铺背景，blurH/blurV 原样拷贝，fs_glass 在 U.tint 指定的颜色上输出覆盖度。
    const SHADER: &str = r#"
// @fx id testglass
// @fx label Test Glass
// @fx param U.tint float 0 100 1 100 "Tint" v/100
struct Params {
  tint : f32,
  pad0 : f32,
  pad1 : f32,
  pad2 : f32,
};
struct FT { a : vec4<f32>, b : vec4<f32>, }; // F: 32 bytes (quad.xyzw + blur/count)
struct TT { v : vec4<f32>, };                 // T: 16 bytes (tile offset/scale)
@group(0) @binding(0) var<uniform> U : Params;
@group(0) @binding(1) var sceneTex : texture_2d<f32>;
@group(0) @binding(2) var sceneSamp : sampler;
@group(1) @binding(0) var blurIn : texture_2d<f32>;
@group(1) @binding(1) var blurSamp : sampler;
@group(2) @binding(0) var<uniform> T : TT;
@group(2) @binding(1) var<uniform> F : FT;
struct VSOut { @builtin(position) pos : vec4<f32>, @location(0) uv : vec2<f32>, };
@vertex
fn vs_full(@builtin(vertex_index) vi : u32) -> VSOut {
  var c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let p = c[vi];
  var o : VSOut;
  o.pos = vec4<f32>(p, 0.0, 1.0);
  o.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
  return o;
}
@fragment
fn fs_tile(i : VSOut) -> @location(0) vec4<f32> {
  let uv = T.v.xy + i.uv * T.v.zw;
  return textureSampleLevel(sceneTex, sceneSamp, uv, 0.0);
}
@fragment
fn fs_blurH(i : VSOut) -> @location(0) vec4<f32> { return textureSampleLevel(blurIn, blurSamp, i.uv, 0.0); }
@fragment
fn fs_blurV(i : VSOut) -> @location(0) vec4<f32> { return textureSampleLevel(blurIn, blurSamp, i.uv, 0.0); }
@fragment
fn fs_glass(i : VSOut) -> @location(0) vec4<f32> {
  let a = clamp(U.tint, 0.0, 1.0);
  return vec4<f32>(a, 0.0, 0.0, a); // 半透明红，覆盖度 = tint
}
"#;

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * w + x) * 4) as usize;
        [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]
    }

    #[test]
    fn material_composites_only_over_the_bound_node_rect() {
        let mut comp = match WgslComposite::new() {
            Some(c) => c,
            None => {
                eprintln!("no GPU adapter — skipping the WGSL material pixel test");
                return;
            }
        };
        // Load the synthetic shader into the shared registry under a stable id.
        let id = "testglass";
        {
            let mut reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
            reg.define(id, fx::Source::Inline(SHADER.into()));
            reg.load(id, fx::Source::Inline(SHADER.into())).expect("test shader parses");
        }

        // A 64×64 flat grey canvas (straight RGBA), all pixels [128,128,128,255].
        let (w, h) = (64u32, 64u32);
        // Flat grey canvas with **opaque** alpha: straight RGBA [128,128,128,255] per pixel.
        let mut scene = vec![0u8; (w * h * 4) as usize];
        for px in scene.chunks_exact_mut(4) {
            px.copy_from_slice(&[128, 128, 128, 255]);
        }
        // Node rect: [16,16]..[48,48]. U.tint = 1.0 → opaque red glass over that rect.
        let pass = MaterialPass {
            effect: id.to_string(),
            node: NodeId(1),
            u: f32_bytes(&[1.0, 0.0, 0.0, 0.0]), // U.tint = 1.0 → opaque red glass
            f: f32_bytes(&[32.0, 32.0, 32.0, 32.0, 0.0, 0.0, 1.0, 0.0]),
            t: f32_bytes(&[0.25, 0.25, 0.5, 0.5]),
            rect_px: [16, 16, 32, 32],
        };
        let out = comp.composite(&scene, w, h, &[pass]).expect("composite succeeds on a real adapter");
        assert_eq!(out.len(), scene.len());

        let inside = px(&out, w, 32, 32); // rect centre
        let outside = px(&out, w, 4, 4); // well outside the rect
        let edge = px(&out, w, 8, 32); // outside on the same row
        assert!(inside[0] > 200 && inside[1] < 40 && inside[2] < 40, "glass is opaque red inside the node rect: {inside:?}");
        assert_eq!(outside, [128, 128, 128, 255], "pixels outside the rect are byte-identical to the scene: {outside:?}");
        assert_eq!(edge, [128, 128, 128, 255], "pixels left of the rect are untouched: {edge:?}");
    }

    /// Even when a canvas is already in `last_image` (a `Work::Reused` frame), the material must be
    /// applied to the *display* image exactly once — guarded in `refine` by keeping `last_image`
    /// raw. Here we assert two composites of the same pass don't stack (idempotent on raw input).
    #[test]
    fn recompositing_from_the_raw_scene_does_not_stack() {
        let mut comp = match WgslComposite::new() {
            Some(c) => c,
            None => {
                eprintln!("no GPU adapter — skipping");
                return;
            }
        };
        let id = "testglass";
        {
            let mut reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
            if reg.scan(id).is_none() {
                reg.define(id, fx::Source::Inline(SHADER.into()));
                let _ = reg.load(id, fx::Source::Inline(SHADER.into()));
            }
        }
        let (w, h) = (32u32, 32u32);
        // Flat grey canvas with **opaque** alpha: straight RGBA [128,128,128,255] per pixel.
        let mut scene = vec![0u8; (w * h * 4) as usize];
        for px in scene.chunks_exact_mut(4) {
            px.copy_from_slice(&[128, 128, 128, 255]);
        }
        let half = MaterialPass {
            effect: id.to_string(),
            node: NodeId(2),
            u: f32_bytes(&[0.5, 0.0, 0.0, 0.0]), // U.tint = 0.5
            f: f32_bytes(&[16.0, 16.0, 16.0, 16.0, 0.0, 0.0, 1.0, 0.0]),
            t: f32_bytes(&[0.25, 0.25, 0.5, 0.5]),
            rect_px: [8, 8, 16, 16],
        };
        let a = comp.composite(&scene, w, h, &[half.clone()]).expect("first composite");
        let b = comp.composite(&scene, w, h, &[half]).expect("second composite from the SAME raw scene");
        assert_eq!(a, b, "compositing from the raw scene twice yields identical pixels (no stacking)");
    }
}
