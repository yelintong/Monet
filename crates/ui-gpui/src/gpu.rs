//! GPU 渲染后端 —— `docs/migration-egui-to-gpui.md` §18 的第 1、2 步。
//!
//! 目标（原文 §18）：场景每帧在 GPU 直接画，面板与画布天然同帧，去掉
//! 「CPU 栅格 → 图像 → 纹理上传」这一跳。
//!
//! 这一版只做**最小可用面**：把所有纯色/圆角矩形收集成一个 [`Scene`]，用**一次实例化绘制**
//! 交给 GPU（片元里用有符号距离场做抗锯齿），离屏渲染到 `Rgba8Unorm` 纹理再读回字节。
//! 这就是后面接渐变/图片/文字/阴影的落点——它们各自加一个管线或一种实例即可。
//!
//! 明确**没做**（§18 第 3 步起）：渐变、图片、文字 atlas、阴影/模糊、混合模式、蒙版。
//! 所以本模块**不参与主渲染路径**：`figcraft_render::Retained`（tiny-skia，CPU）仍然是
//! 画布真正的出图者。开 `--features gpu` 才有此模块，`cargo test -p figcraft-ui-gpui
//! --features gpu` 会跑通「离屏出图 + 像素断言 + 与 CPU 的吞吐对比」。
//!
//! 适配器不可用（无显示/无 GPU 的 CI）时 [`Gpu::new`] 返回 `None`，测试跳过而不是失败——
//! 这与 §18「保留 CPU 路径做对照与降级」是同一条原则。

use bytemuck::{Pod, Zeroable};

/// 一个纯色圆角矩形（设备像素，y 向下，`rect = [x, y, w, h]`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolidRect {
    pub rect: [f32; 4],
    /// 非预乘 sRGB + alpha（0–1），写进着色器前由它自己乘 alpha。
    pub color: [f32; 4],
    pub radius: f32,
}

/// 一帧要画的纯色矩形。
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// 底色（清屏色），非预乘。
    pub clear: [f32; 4],
    pub rects: Vec<SolidRect>,
}

impl Scene {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, clear: [0.0, 0.0, 0.0, 0.0], rects: Vec::new() }
    }

    pub fn push(&mut self, rect: [f32; 4], color: [f32; 4], radius: f32) {
        self.rects.push(SolidRect { rect, color, radius });
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    rect: [f32; 4],
    color: [f32; 4],
    /// x = 圆角半径（用 vec4 而不是 `f32 + vec3`：后者的对齐会把结构体撑到 64 字节，
    /// 与 Rust 这边算出来的大小对不上，wgpu 会直接报 validation error）。
    extra: [f32; 4],
}

const SHADER: &str = r#"
struct Globals { size: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;

struct Inst {
  rect: vec4<f32>,     // x, y, w, h（设备像素，左上角为原点、y 向下）
  color: vec4<f32>,    // 非预乘 sRGB + alpha
  extra: vec4<f32>,    // x = 圆角半径
};
@group(1) @binding(0) var<storage, read> insts: array<Inst>;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,   // 矩形内坐标（像素，左上角 0,0）
  @location(1) half: vec2<f32>,
  @location(2) @interpolate(flat) color: vec4<f32>,
  @location(3) @interpolate(flat) radius: f32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
  var corners = array<vec2<f32>, 6>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
  let c = corners[vi];
  let inst = insts[ii];
  let p = inst.rect.xy + c * inst.rect.zw;
  var o: VsOut;
  // 像素 → NDC（y 翻转：设备坐标 y 向下）。
  o.pos = vec4(p / globals.size * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
  o.local = c * inst.rect.zw;
  o.half = inst.rect.zw * 0.5;
  o.color = inst.color;
  o.radius = inst.extra.x;
  return o;
}

// 圆角矩形的有符号距离（负 = 内部）。
fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
  let rr = min(r, min(b.x, b.y));
  let q = abs(p) - b + vec2(rr);
  return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - rr;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let p = in.local - in.half;
  let d = sd_round_box(p, in.half, in.radius);
  // 半像素覆盖：d = 0 时 0.5，向内 0.5px 后 1。
  let a = clamp(0.5 - d, 0.0, 1.0) * in.color.a;
  // 预乘输出（混合用 One / OneMinusSrcAlpha）。
  return vec4(in.color.rgb * a, a);
}
"#;

/// 离屏 GPU 渲染器。构造失败（无适配器/无设备）时 `new` 返回 `None`。
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    bind0: wgpu::BindGroup,
    layout1: wgpu::BindGroupLayout,
    adapter_info: wgpu::AdapterInfo,
    /// 实例缓冲按容量复用（每帧新建会把这框架的耗时翻几倍，见 `gpu_throughput_…`）。
    inst: Option<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    /// 离屏目标与读回缓冲按尺寸复用。
    target: Option<Target>,
}

struct Target {
    w: u32,
    h: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl Gpu {
    /// 建实例、取适配器（允许软件回退）、建设备与管线。
    ///
    /// 「允许软件回退」是有意的：没有独显的 CI / 远程桌面也能跑（DX12 的 WARP、Vulkan 的
    /// lavapipe），这样这条路径的测试在无 GPU 机器上仍然有效，只是慢一些。
    pub fn new() -> Option<Self> {
        // `Instance::default()` 用 wgpu 的默认后端集合（dx12 / vulkan / metal / gles）。
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
            label: Some("figcraft-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figcraft-gpu-pipeline-layout"),
            bind_group_layouts: &[Some(&bind0_layout), Some(&layout1)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("figcraft-gpu-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // 预乘 alpha 的 source-over。
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bind"),
            layout: &bind0_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });

        Some(Self { device, queue, pipeline, globals_buf, bind0, layout1, adapter_info, inst: None, target: None })
    }

    /// 适配器信息（测试与 `--control` 的 `ui.inspect` 用它说明「跑在哪张卡上」）。
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// 把一帧场景渲染成 **预乘 RGBA8** 像素（长度 = w × h × 4，行序自上而下）。
    pub fn render(&mut self, scene: &Scene) -> Vec<u8> {
        self.draw(scene, true)
    }

    /// 只画、不读回：这是真正的「每帧画一次」成本（真正接进画布时像素留在显存里，
    /// 直接当纹理用，不会有这条 `copy_texture_to_buffer` + `map_async` 的往返）。
    pub fn draw_only(&mut self, scene: &Scene) {
        let _ = self.draw(scene, false);
    }

    fn draw(&mut self, scene: &Scene, read_back: bool) -> Vec<u8> {
        let (w, h) = (scene.width.max(1), scene.height.max(1));
        let globals = Globals { size: [w as f32, h as f32], _pad: [0.0; 2] };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        // 实例缓冲：至少留一个（空场景也建缓冲，省一条分支）。
        let instances: Vec<Instance> =
            scene.rects.iter().map(|r| Instance { rect: r.rect, color: r.color, extra: [r.radius, 0.0, 0.0, 0.0] }).collect();
        let count = instances.len().max(1) as u64;
        let bytes = if instances.is_empty() { vec![0u8; std::mem::size_of::<Instance>()] } else { bytemuck::cast_slice(&instances).to_vec() };
        // 容量不够才重建（只增不减），否则直接覆写。
        let need = bytes.len() as u64;
        if self.inst.as_ref().is_none_or(|(_, _, cap)| *cap < need) {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: need.next_power_of_two().max(256),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instances-bind"),
                layout: &self.layout1,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            self.inst = Some((buf, bind, need.next_power_of_two().max(256)));
        }
        let (inst_buf, bind1, _) = self.inst.as_ref().expect("instances");
        self.queue.write_buffer(inst_buf, 0, &bytes);

        let unpadded = w * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if self.target.as_ref().is_none_or(|t| t.w != w || t.h != h) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("target"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (padded * h) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target { w, h, texture, view, readback });
        }
        let t = self.target.as_ref().expect("target");
        let (view, readback) = (&t.view, &t.readback);

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let c = scene.clear;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // 非预乘的底色 → 预乘后清屏（GPU 先清成预乘值，混合才自洽）。
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(c[0] * c[3]),
                            g: f64::from(c[1] * c[3]),
                            b: f64::from(c[2] * c[3]),
                            a: f64::from(c[3]),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !scene.rects.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind0, &[]);
                pass.set_bind_group(1, bind1, &[]);
                pass.draw(0..6, 0..count as u32);
            }
        }
        if read_back {
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        self.queue.submit(Some(enc.finish()));
        if !read_back {
            return Vec::new();
        }

        // 映射并读：`map_async` 的回调在设备轮询时才会跑。
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let _ = rx.recv();

        let Ok(mapped) = slice.get_mapped_range() else { return Vec::new() };
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            out.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        readback.unmap();
        out
    }
}

// ───────────────────── 文档 → 场景（接进画布的那一半） ─────────────────────

/// 把一页文档压成 [`Scene`]（纯色/圆角矩形），**只接受能忠实表达的子集**。
///
/// 返回 `None` 表示这一帧有 GPU 画不对的东西（渐变、图片、文字、描边、阴影、混合模式、
/// 裁剪、旋转、透明度），调用方必须回退到 CPU 的 `Retained`。这条规则是刻意的：
/// 宁可慢，也不让画布上出现「看起来差不多、其实错」的像素。
///
/// 目前接受的：Page / Frame / Group / Section / Component / Instance / Rectangle，
/// 单个纯色填充、无描边、无可见效果、不裁剪、不旋转、不透明、正常混合。
pub fn scene_of(doc: &figcraft_doc::Document, page: figcraft_doc::NodeId, view: &figcraft_render::View) -> Option<Scene> {
    use figcraft_doc::NodeKind;
    let mut s = Scene::new(view.width, view.height);
    let pn = doc.node(page)?;
    if let NodeKind::Page { background, .. } = &pn.kind {
        s.clear = [background.r, background.g, background.b, background.a];
    }
    for &c in &pn.children {
        walk(doc, c, view, &mut s)?;
    }
    Some(s)
}

fn walk(doc: &figcraft_doc::Document, id: figcraft_doc::NodeId, view: &figcraft_render::View, out: &mut Scene) -> Option<()> {
    use figcraft_doc::NodeKind;
    let n = doc.node(id)?;
    if !n.visible {
        return Some(());
    }
    if n.opacity < 0.999 || n.blend != figcraft_color::BlendMode::Normal || n.clip || n.effects.iter().any(|e| e.visible) {
        return None;
    }
    if !n.strokes.is_empty() {
        return None;
    }
    if n.fills.len() > 1 {
        return None;
    }
    let xf = doc.world_transform(id);
    if figcraft_geom::rotation_degrees(xf).abs() > 0.01 {
        return None;
    }
    match &n.kind {
        NodeKind::Page { .. } | NodeKind::Frame | NodeKind::Group | NodeKind::Section { .. } | NodeKind::Rectangle => {}
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } => {}
        // 椭圆/多边形/星形/矢量/文字/图片：这条路径画不了，整帧回退。
        _ => return None,
    }
    let m = view.transform() * xf;
    let (w, h) = (n.width, n.height);
    if w <= 0.0 || h <= 0.0 {
        return Some(());
    }
    // 没有旋转时四个角映射成一个轴对齐矩形。
    let (x0, y0) = map(m, 0.0, 0.0);
    let (x1, y1) = map(m, w, h);
    let (rx0, ry0, rx1, ry1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    if let Some(paint) = n.fills.first() {
        let c = paint.solid_color()?;
        // 圆角按设备像素换算（四角取最大，够用；四角不同半径是后续再加的字段）。
        let unit = if w > 0.0 { (rx1 - rx0) / w } else { 1.0 };
        let radius = n.corners.radii.iter().copied().fold(0.0_f64, f64::max) * unit;
        out.push([rx0 as f32, ry0 as f32, (rx1 - rx0) as f32, (ry1 - ry0) as f32], [c.r, c.g, c.b, c.a], radius as f32);
    }
    for &c in &n.children {
        walk(doc, c, view, out)?;
    }
    Some(())
}

fn map(m: figcraft_geom::Affine, x: f64, y: f64) -> (f64, f64) {
    let p = m * figcraft_geom::Point::new(x, y);
    (p.x, p.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GPU 场景构建：只含纯色矩形的文档能压成场景，且像素位置对得上。
    #[test]
    fn a_solid_rectangle_document_becomes_a_scene() {
        let mut doc = figcraft_doc::Document::default();
        let page = doc.pages[0];
        let rect = doc.create(page, figcraft_doc::NodeKind::Rectangle, "Rect", 10.0, 20.0, 100.0, 50.0).expect("a rectangle");
        if let Ok(n) = doc.get_mut(rect) {
            n.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
        }
        let scene = scene_of(&doc, page, &figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 120 })
            .expect("a solid rectangle document is expressible");
        assert_eq!((scene.width, scene.height), (200, 120));
        assert_eq!(scene.rects.len(), 1, "one rectangle, one instance");
        let r = scene.rects[0].rect;
        assert!((r[2] - 100.0).abs() < 0.5 && (r[3] - 50.0).abs() < 0.5, "size in device pixels: {r:?}");
        let _ = rect;
    }

    /// 文档里有这条路径画不了的东西（文字）时，必须明确回退，不能画出半对不对的图。
    #[test]
    fn a_document_with_text_falls_back() {
        let doc = crate::app::sample_doc();
        let page = doc.pages[0];
        let scene = scene_of(&doc, page, &figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 800, height: 600 });
        assert!(scene.is_none(), "the sample document has text and gradients: the GPU scene must decline it");
    }

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn a_solid_rect_lands_where_it_says() {
        let Some(mut gpu) = Gpu::new() else {
            eprintln!("no GPU adapter available — skipping (the CPU path stays the default)");
            return;
        };
        let mut s = Scene::new(64, 64);
        s.push([16.0, 16.0, 32.0, 32.0], [1.0, 0.0, 0.0, 1.0], 0.0);
        let out = gpu.render(&s);
        assert_eq!(out.len(), 64 * 64 * 4);
        let inside = px(&out, 64, 32, 32);
        assert_eq!(&inside[..3], &[255, 0, 0], "inside the rect is red");
        let outside = px(&out, 64, 4, 4);
        assert_eq!(outside[3], 0, "outside stays cleared");
    }

    #[test]
    fn the_corner_radius_is_cut_out() {
        let Some(mut gpu) = Gpu::new() else {
            eprintln!("no GPU adapter available — skipping");
            return;
        };
        let mut s = Scene::new(64, 64);
        // 半径 30 的方块：四角被切掉。
        s.push([16.0, 16.0, 32.0, 32.0], [0.0, 1.0, 0.0, 1.0], 30.0);
        let out = gpu.render(&s);
        assert_eq!(px(&out, 64, 32, 32)[3], 255, "the centre is opaque");
        assert!(px(&out, 64, 17, 17)[3] < 40, "the corner falls outside the rounded box");
    }

    /// 吞吐对照：同一份「大量纯色矩形」的工作量，GPU（只画 / 画完读回）与 CPU（tiny-skia）各跑一遍。
    ///
    /// 这不是性能断言（机器差异太大），而是**保留 §18 说的「CPU 路径做对照」**：
    /// 跑 `cargo test -p figcraft-ui-gpui --features gpu -- --nocapture gpu_throughput`
    /// 就能看到三个数。`draw_only` 才是真正接进画布后的成本（像素留在显存直接当纹理），
    /// `render` 多了一条 `copy_texture_to_buffer` + `map_async` 的往返，仅供测试读取。
    #[test]
    fn gpu_throughput_against_the_cpu_path() {
        let Some(mut gpu) = Gpu::new() else {
            eprintln!("no GPU adapter available — skipping");
            return;
        };
        let mut s = Scene::new(1024, 1024);
        for i in 0..2000 {
            let x = (i % 50) as f32 * 20.0;
            let y = (i / 50) as f32 * 20.0;
            s.push([x, y, 18.0, 18.0], [0.2, 0.4, 0.8, 1.0], 4.0);
        }

        // 预热：第一帧要建缓冲/纹理/管线缓存。
        gpu.draw_only(&s);
        let _ = gpu.render(&s);

        const N: usize = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..N {
            gpu.draw_only(&s);
        }
        let draw_ms = t0.elapsed().as_secs_f64() * 1000.0 / N as f64;

        let t1 = std::time::Instant::now();
        let out = gpu.render(&s);
        let readback_ms = t1.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(out.len(), 1024 * 1024 * 4);

        // CPU 对照：同样 2000 个圆角矩形，用 tiny-skia 画到 Pixmap。
        let t1 = std::time::Instant::now();
        let mut pix = tiny_skia::Pixmap::new(1024, 1024).unwrap();
        for r in &s.rects {
            let mut pb = tiny_skia::PathBuilder::new();
            let (x, y, w, h, rad) = (r.rect[0], r.rect[1], r.rect[2], r.rect[3], r.radius);
            let (x0, y0, x1, y1) = (x, y, x + w, y + h);
            pb.move_to(x0 + rad, y0);
            pb.line_to(x1 - rad, y0);
            pb.quad_to(x1, y0, x1, y0 + rad);
            pb.line_to(x1, y1 - rad);
            pb.quad_to(x1, y1, x1 - rad, y1);
            pb.line_to(x0 + rad, y1);
            pb.quad_to(x0, y1, x0, y1 - rad);
            pb.line_to(x0, y0 + rad);
            pb.quad_to(x0, y0, x0 + rad, y0);
            pb.close();
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8((r.color[0] * 255.0) as u8, (r.color[1] * 255.0) as u8, (r.color[2] * 255.0) as u8, (r.color[3] * 255.0) as u8);
                paint.anti_alias = true;
                pix.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
        }
        let cpu_ms = t1.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "2000 rounded rects @1024²: gpu draw {draw_ms:.2} ms, +readback {readback_ms:.2} ms, cpu {cpu_ms:.2} ms ({})",
            gpu.adapter_info().name
        );
    }
}
