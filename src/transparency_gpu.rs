//! TP1 transparent pass on the GPU (docs/m7-tp1.md, shaders/transparency.wgsl). Lit frames
//! with transparent draws or live emitters run, after opaque shading and temporal
//! reconstruction and before presentation, into the resolved HDR target:
//! 1. opaque view depth: from the hit records on traced and visibility-buffer frames, from
//!    the raster depth on 1x raster frames;
//! 2. the transparent receivers' sun shadow map, rendered only when the sun, the receivers'
//!    footprint or the opaque casters change;
//! 3. particle simulation and a one-workgroup bitonic sort per emitter;
//! 4. transparent draws and emitters back to front by object. A transparent draw copies
//!    the HDR colour behind it first, then composites its program's `shade` over the
//!    refracted copy.
//!
//! Transparent draws compose the material's own program (`surface`, `shade` and the
//! optional `transmittance` hook). Their sun visibility is the renderer's hardware ray on
//! RT renderers and the shadow map otherwise; particles use the map on both.
use super::*;
use crate::transparency::{Emitter, EmitterDescriptor, EmitterShared, TransparencyStats};
use glam::Vec4;

const SHADER: &str = include_str!("../shaders/transparency.wgsl");
/// Square sun shadow map of the transparent receivers' footprint.
const SHADOW_SIZE: u32 = 2048;
/// Transparent draws per frame: one 256-byte dynamic slot each.
const MAX_TRANSPARENT_DRAWS: usize = 64;
const SLOT: u64 = 256;
/// The camera block at 0, then `TpFrame` at 256.
const FRAME_OFFSET: u64 = 256;
const FRAME_BYTES: u64 = 336;
const EMITTER_BYTES: u64 = 160;
const PARTICLE_BYTES: u64 = 48;

/// One `// ---- NAME ----` section of the shader.
fn section(name: &str) -> &'static str {
    let start = SHADER.find(&format!("// ---- {name} ----")).unwrap();
    let end = SHADER.find(&format!("// ---- end {name} ----")).unwrap();
    &SHADER[start..end]
}

/// `source` with its `fn name(` function replaced by `replacement`.
fn replace_function(source: &str, name: &str, replacement: &str) -> String {
    let start = source.find(&format!("fn {name}(")).unwrap();
    let brace = start + source[start..].find('{').unwrap();
    let mut depth = 1;
    let mut end = brace + 1;
    let bytes = source.as_bytes();
    while depth != 0 {
        depth += i32::from(bytes[end] == b'{') - i32::from(bytes[end] == b'}');
        end += 1;
    }
    format!("{}{replacement}{}", &source[..start], &source[end..])
}

/// The non-RT transparent head's sun visibility: the cached map, for the frame's sun.
const MAP_SHADOW: &str = "fn hard_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    if light.kind.x!=0u || dot(l,tp.sun.xyz)<0.9999 { return 1.0; }
    return tp_sun_visibility(receiver.world);
}";

/// A transparent material's module: its program composed on `head` (the hardware prelude
/// on RT renderers; otherwise the prelude with the map's sun visibility), then the pass's
/// entry with the program's composite.
fn glass_source(head: &str, rt: bool, program: &str, parameters: bool) -> String {
    let head = if rt {
        head.to_owned()
    } else {
        replace_function(head, "hard_shadow", MAP_SHADOW)
    };
    let composed =
        crate::program::compose(&head, program, crate::program::Stage::Raster, parameters);
    // The surface's own light and the per-channel share of the light behind it.
    let (surface, transmit) = if crate::program::declares_transmittance(program) {
        ("lit", "transmittance(surface_output,surface_input)")
    } else {
        ("lit*coverage", "vec3<f32>(1.0-coverage)")
    };
    format!(
        "{}\n{}{}{}",
        composed.source,
        section("TP_FRAME"),
        section("TP_SUN"),
        section("TP_GLASS")
            .replace("TP_SURFACE", surface)
            .replace("TP_TRANSMIT", transmit)
    )
}
/// The particle simulation and draw modules, after `prelude` for its environment helpers
/// and AT1's camera fog and aerial perspective.
fn particle_sources(prelude: &str) -> [String; 2] {
    let common = format!(
        "{prelude}\n{}{}{}{}",
        crate::atmosphere::api_source(),
        section("TP_FRAME"),
        section("TP_SUN"),
        section("TP_PARTICLES")
    );
    ["TP_PARTICLES_SIM", "TP_PARTICLES_DRAW"].map(|s| format!("{common}{}", section(s)))
}
fn hit_depth_source() -> String {
    format!(
        "enable wgpu_binding_array;\nconst PRIMARY_BATCHES:u32={}u;\n{}{}",
        crate::TRACED_DRAWS,
        section("TP_FRAME"),
        section("TP_DEPTH")
    )
}

#[cfg(test)]
#[path = "transparency_tests.rs"]
mod tp1_tests;

#[derive(Default)]
pub(super) struct TransparencyState {
    pub stats: TransparencyStats,
    /// Particle simulation step per lit frame, seconds.
    pub step: f32,
    /// The frame's camera before any reconstruction jitter.
    pub camera: Option<Camera>,
    emitters: Vec<EmitterSlot>,
    next_emitter: u64,
    gpu: Option<Gpu>,
    glass: Vec<GlassPipelines>,
    targets: Option<Targets>,
    shadow_key: Option<[u8; 32]>,
}

struct Gpu {
    frame: Owned<wgpu::Buffer>,
    draws: Owned<wgpu::Buffer>,
    sampler: Owned<wgpu::Sampler>,
    compare: Owned<wgpu::Sampler>,
    shadow: Depth,
    white: Owned<wgpu::TextureView>,
    _white: Owned<wgpu::Texture>,
    /// Indexed by RT (hardware sun rays) or not.
    /// Indexed as above; the RT entry exists only on hardware-shadow renderers.
    glass_bindings: Vec<Owned<wgpu::BindGroupLayout>>,
    glass_layouts: Vec<Owned<wgpu::PipelineLayout>>,
    shadow_group: Owned<wgpu::BindGroup>,
    shadow_pipeline: Owned<wgpu::RenderPipeline>,
    raster_depth_bindings: Owned<wgpu::BindGroupLayout>,
    raster_depth: Owned<wgpu::ComputePipeline>,
    /// Created with the first traced or visibility-buffer frame.
    hit_depth: Option<(Owned<wgpu::BindGroupLayout>, Owned<wgpu::ComputePipeline>)>,
    sim_bindings: Owned<wgpu::BindGroupLayout>,
    draw_bindings: Owned<wgpu::BindGroupLayout>,
    /// Built from the prelude they were composed with.
    particles: Option<ParticlePipelines>,
    empty: Owned<wgpu::BindGroup>,
    empty_bindings: Owned<wgpu::BindGroupLayout>,
}
struct ParticlePipelines {
    prelude: [u8; 32],
    update: Owned<wgpu::ComputePipeline>,
    sort: Owned<wgpu::ComputePipeline>,
    draw: Owned<wgpu::RenderPipeline>,
}
struct Targets {
    color: Depth,
    depth: Depth,
    generation: u64,
}
struct GlassPipelines {
    program: u64,
    key: [u8; 32],
    /// By tangent stream.
    pipelines: [Owned<wgpu::RenderPipeline>; 2],
}
struct EmitterSlot {
    id: u64,
    handle: std::sync::Weak<EmitterShared>,
    params: EmitterParams,
    sprite: Option<Texture>,
    uniform: Owned<wgpu::Buffer>,
    sim: Owned<wgpu::BindGroup>,
    particles: Owned<wgpu::Buffer>,
    sorted: Owned<wgpu::Buffer>,
    spawned: u64,
    carry: f64,
    _allocation: Allocation,
}
/// An emitter descriptor without its texture borrow.
#[derive(Clone, Copy)]
struct EmitterParams {
    position: Vec3,
    spawn_radius: f32,
    spawn_rate: f32,
    lifetime: [f32; 2],
    velocity: Vec3,
    velocity_spread: f32,
    acceleration: Vec3,
    drag: f32,
    size: [f32; 2],
    color: [[f32; 4]; 2],
    emission: [[f32; 3]; 2],
    fade: [f32; 2],
    soft_distance: f32,
    capacity: u32,
    seed: u32,
}
impl EmitterParams {
    fn new(d: &EmitterDescriptor<'_>) -> Self {
        Self {
            position: d.position,
            spawn_radius: d.spawn_radius,
            spawn_rate: d.spawn_rate,
            lifetime: d.lifetime,
            velocity: d.velocity,
            velocity_spread: d.velocity_spread,
            acceleration: d.acceleration,
            drag: d.drag,
            size: d.size,
            color: d.color,
            emission: d.emission,
            fade: d.fade,
            soft_distance: d.soft_distance,
            capacity: d.capacity,
            seed: d.seed,
        }
    }
    /// A world box every live particle stays inside.
    fn bounds(&self) -> (Vec3, Vec3) {
        let life = self.lifetime[1];
        let reach = self.spawn_radius
            + (self.velocity.length() + self.velocity_spread) * life
            + 0.5 * self.acceleration.length() * life * life
            + 0.5 * self.size[0].max(self.size[1]);
        (self.position - Vec3::splat(reach), self.position + Vec3::splat(reach))
    }
    fn uniform(&self, first: u32, spawned: u32, textured: bool) -> [u8; EMITTER_BYTES as usize] {
        let mut f = [0f32; 40];
        let v4 = |f: &mut [f32; 40], at: usize, v: [f32; 4]| f[at..at + 4].copy_from_slice(&v);
        v4(&mut f, 0, self.position.extend(self.spawn_radius).to_array());
        v4(&mut f, 4, self.velocity.extend(self.velocity_spread).to_array());
        v4(&mut f, 8, self.acceleration.extend(self.drag).to_array());
        v4(&mut f, 12, self.color[0]);
        v4(&mut f, 16, self.color[1]);
        let [e0, e1] = self.emission;
        v4(&mut f, 20, [e0[0], e0[1], e0[2], self.size[0]]);
        v4(&mut f, 24, [e1[0], e1[1], e1[2], self.size[1]]);
        v4(&mut f, 28, [self.lifetime[0], self.lifetime[1], self.fade[0], self.fade[1]]);
        let sorted = self.capacity.next_power_of_two() as f32;
        v4(&mut f, 36, [self.soft_distance, sorted, f32::from(u8::from(textured)), 0.0]);
        let mut bytes = [0u8; EMITTER_BYTES as usize];
        for (dst, v) in bytes.chunks_exact_mut(4).zip(f) {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        for (k, v) in [self.capacity, first, spawned, self.seed].into_iter().enumerate() {
            bytes[128 + 4 * k..132 + 4 * k].copy_from_slice(&v.to_ne_bytes());
        }
        bytes
    }
}

fn entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}
fn uniform(dynamic: bool, size: u64) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: dynamic,
        min_binding_size: wgpu::BufferSize::new(size),
    }
}
fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}
fn texture(sample_type: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}
const FLOAT: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };
const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const FRAGMENT: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
const COMPUTE: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;

impl Renderer {
    fn tp_layout(&self, label: &str, entries: &[wgpu::BindGroupLayoutEntry]) -> Owned<wgpu::BindGroupLayout> {
        self.ledger.own(
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            }),
            ResourceKind::BindGroupLayout,
        )
    }
    fn tp_pipeline_layout(&self, label: &str, groups: &[&wgpu::BindGroupLayout]) -> Owned<wgpu::PipelineLayout> {
        let groups: Vec<_> = groups.iter().map(|g| Some(*g)).collect();
        self.ledger.own(
            self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        )
    }
    fn tp_module(&self, label: &str, source: &str) -> Owned<wgpu::ShaderModule> {
        self.ledger.own(
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        )
    }
    fn tp_compute(&self, label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str) -> Owned<wgpu::ComputePipeline> {
        self.ledger.own(
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        )
    }

    /// The pass's fixed resources and kernels, created with its first frame.
    fn tp_gpu(&self) -> Gpu {
        let buffer = |label, size, usage| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        let frame = buffer("TP1 frame", FRAME_OFFSET + FRAME_BYTES, wgpu::BufferUsages::UNIFORM);
        let draws = buffer("TP1 draws", SLOT * MAX_TRANSPARENT_DRAWS as u64, wgpu::BufferUsages::UNIFORM);
        let sampler = self.ledger.own(
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("TP1 linear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let compare = self.ledger.own(
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("TP1 shadow compare"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        let shadow = self.tp_target("TP1 sun shadow map", wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, 4);
        let white_texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("TP1 white sprite"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        self.queue.write_texture(white_texture.as_image_copy(), &[255; 4],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
        let white = self.ledger.own(white_texture.create_view(&Default::default()), ResourceKind::TextureView);
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let all = vf | COMPUTE;
        let glass_entries = |rt: bool| {
            let mut e = vec![
                entry(0, vf, uniform(false, 80)),
                entry(30, all, uniform(false, FRAME_BYTES)),
                entry(31, FRAGMENT, texture(FLOAT)),
                entry(32, FRAGMENT, texture(UNFILTERED)),
                entry(33, FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(34, FRAGMENT, wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                }),
                entry(35, FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                entry(37, FRAGMENT, uniform(true, 16)),
            ];
            if rt {
                e.push(entry(2, FRAGMENT, wgpu::BindingType::AccelerationStructure { vertex_return: false }));
            }
            e
        };
        let paths: &[bool] = if self.hardware_shadows.is_some() { &[false, true] } else { &[false] };
        let glass_bindings: Vec<_> = paths.iter().map(|&rt| self.tp_layout("TP1 transparent group 0", &glass_entries(rt))).collect();
        let glass_layouts: Vec<_> = glass_bindings.iter().map(|bindings| {
            self.tp_pipeline_layout("TP1 transparent", &[bindings, &self.instance_bindings, &self.material_bindings, &self.lighting.bindings])
        }).collect();
        // Shadow map: depth only, every opaque instance.
        let shadow_bindings = self.tp_layout("TP1 shadow group 0", &[entry(30, all, uniform(false, FRAME_BYTES))]);
        let shadow_group = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 shadow"),
                layout: &shadow_bindings,
                entries: &[wgpu::BindGroupEntry {
                    binding: 30,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &frame,
                        offset: FRAME_OFFSET,
                        size: wgpu::BufferSize::new(FRAME_BYTES),
                    }),
                }],
            }),
            ResourceKind::BindGroup,
        );
        let shadow_layout = self.tp_pipeline_layout("TP1 shadow", &[&shadow_bindings, &self.instance_bindings]);
        let module = self.tp_module("TP1 shadow", &format!("{}{}", section("TP_FRAME"), section("TP_SHADOW")));
        let shadow_pipeline = self.ledger.own(
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("TP1 shadow"),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("tp_shadow_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 48,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    })],
                },
                fragment: None,
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        let raster_depth_bindings = self.tp_layout("TP1 raster depth", &[
            entry(30, all, uniform(false, FRAME_BYTES)),
            entry(52, COMPUTE, wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::R32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            }),
            entry(53, COMPUTE, wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            }),
        ]);
        let layout = self.tp_pipeline_layout("TP1 raster depth", &[&raster_depth_bindings]);
        let module = self.tp_module("TP1 raster depth", &format!("{}{}", section("TP_FRAME"), section("TP_DEPTH_RASTER")));
        let raster_depth = self.tp_compute("TP1 raster depth", &layout, &module, "tp_depth_raster");
        let depth_entry = entry(34, all, wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        });
        let compare_entry = entry(35, all, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison));
        let sim_bindings = self.tp_layout("TP1 particle simulation", &[
            entry(30, all, uniform(false, FRAME_BYTES)),
            depth_entry,
            compare_entry,
            entry(40, COMPUTE, storage(false)),
            entry(41, COMPUTE, storage(false)),
            entry(42, all, uniform(false, EMITTER_BYTES)),
        ]);
        let draw_bindings = self.tp_layout("TP1 particle draw", &[
            entry(30, all, uniform(false, FRAME_BYTES)),
            entry(32, FRAGMENT, texture(UNFILTERED)),
            entry(33, FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            entry(36, FRAGMENT, texture(FLOAT)),
            entry(40, wgpu::ShaderStages::VERTEX, storage(true)),
            entry(41, wgpu::ShaderStages::VERTEX, storage(true)),
            entry(42, all, uniform(false, EMITTER_BYTES)),
        ]);
        let empty_bindings = self.tp_layout("TP1 empty", &[]);
        let empty = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 empty"),
                layout: &empty_bindings,
                entries: &[],
            }),
            ResourceKind::BindGroup,
        );
        Gpu {
            frame,
            draws,
            sampler,
            compare,
            shadow,
            white,
            _white: white_texture,
            glass_bindings,
            glass_layouts,
            shadow_group,
            shadow_pipeline,
            raster_depth_bindings,
            raster_depth,
            hit_depth: None,
            sim_bindings,
            draw_bindings,
            particles: None,
            empty,
            empty_bindings,
        }
    }

    fn tp_target(&self, label: &str, size: wgpu::Extent3d, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, texel: u64) -> Depth {
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(texture.create_view(&Default::default()), ResourceKind::TextureView);
        Depth {
            texture,
            view,
            _allocation: self.ledger.allocate(AllocationKind::Target, u64::from(size.width) * u64::from(size.height) * texel),
        }
    }

    /// Particle kernels, composed after the current prelude for its environment helpers.
    fn tp_particle_pipelines(&self) -> ParticlePipelines {
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let [sim, draw] = particle_sources(&self.source);
        let sim = self.tp_module("TP1 particle simulation", &sim);
        let layout = self.tp_pipeline_layout("TP1 particle simulation", &[&gpu.sim_bindings, &gpu.empty_bindings, &gpu.empty_bindings, &self.lighting.bindings]);
        let update = self.tp_compute("TP1 particle update", &layout, &sim, "tp_particles_update");
        let sort = self.tp_compute("TP1 particle sort", &layout, &sim, "tp_particles_sort");
        let module = self.tp_module("TP1 particle draw", &draw);
        let layout = self.tp_pipeline_layout("TP1 particle draw", &[&gpu.draw_bindings, &gpu.empty_bindings, &gpu.empty_bindings, &self.lighting.bindings]);
        let draw = self.ledger.own(
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("TP1 particle billboards"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("tp_particle_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("tp_particle_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        ParticlePipelines {
            prelude: Sha256::digest(self.source.as_bytes()).into(),
            update,
            sort,
            draw,
        }
    }

    /// The module source of `program`'s transparent pipelines on this renderer's
    /// sun-visibility path, and its identity.
    fn tp_glass_source(&self, program: u64, rt: bool) -> (String, [u8; 32]) {
        let slot = self.program_slot(program);
        let head = if rt { self.hardware_source.as_deref().unwrap() } else { &self.source };
        let source = glass_source(head, rt, &slot.source, slot.layout.is_some());
        let key = Sha256::digest(source.as_bytes()).into();
        (source, key)
    }
    /// The transparent pipelines of `program` from its module source.
    fn tp_glass_pipelines(&self, program: u64, rt: bool, source: &str, key: [u8; 32]) -> Result<GlassPipelines, Error> {
        let slot = self.program_slot(program);
        let scopes = OperationScopes::new(&self.device);
        let module = self.tp_module(&format!("TP1 transparent {}", slot.label), source);
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let layout = &gpu.glass_layouts[usize::from(rt)];
        let pipelines = [false, true].map(|tangent| {
            self.ledger.own(
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("TP1 transparent"),
                    layout: Some(layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(if tangent { "vs_tangent" } else { "vs" }),
                        compilation_options: Default::default(),
                        buffers: &[
                            Some(wgpu::VertexBufferLayout {
                                array_stride: 48,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &VERTEX_ATTRIBUTES,
                            }),
                            Some(wgpu::VertexBufferLayout {
                                array_stride: 4,
                                step_mode: wgpu::VertexStepMode::Instance,
                                attributes: &MODEL_ATTRIBUTES,
                            }),
                            tangent.then_some(wgpu::VertexBufferLayout {
                                array_stride: 16,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &TANGENT_ATTRIBUTES,
                            }),
                        ],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_transparent"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                    }),
                    primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
                ResourceKind::RenderPipeline,
            )
        });
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(format!("program `{}`, transparent pipelines: {error}", slot.label)),
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        Ok(GlassPipelines { program, key, pipelines })
    }

    /// Creates a particle emitter. It simulates and draws in every lit frame until the
    /// returned handle drops.
    pub fn try_create_emitter(&mut self, d: &EmitterDescriptor<'_>) -> Result<Emitter, Error> {
        self.check_fault()?;
        d.validate()?;
        if d.texture.is_some_and(|t| !Arc::ptr_eq(&self.identity, &t.0.renderer)) {
            return Err(Error::InvalidInput("emitter texture belongs to another renderer"));
        }
        if self.transparency.gpu.is_none() {
            self.transparency.gpu = Some(self.tp_gpu());
        }
        if self.transparency.step == 0.0 {
            self.transparency.step = 1.0 / 60.0;
        }
        let scopes = OperationScopes::new(&self.device);
        let sorted_len = u64::from(d.capacity.next_power_of_two());
        let buffer = |label, size, usage| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        // wgpu zero-initializes buffers: every slot starts dead (lifetime 0).
        let particles = buffer("TP1 particles", u64::from(d.capacity) * PARTICLE_BYTES, wgpu::BufferUsages::STORAGE);
        let sorted = buffer("TP1 particle order", sorted_len * 4, wgpu::BufferUsages::STORAGE);
        let uniform = buffer("TP1 emitter", EMITTER_BYTES, wgpu::BufferUsages::UNIFORM);
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let sim = self.ledger.own(
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 particle simulation"),
                layout: &gpu.sim_bindings,
                entries: &[
                    frame_entry(&gpu.frame),
                    wgpu::BindGroupEntry { binding: 34, resource: wgpu::BindingResource::TextureView(&gpu.shadow.view) },
                    wgpu::BindGroupEntry { binding: 35, resource: wgpu::BindingResource::Sampler(&gpu.compare) },
                    wgpu::BindGroupEntry { binding: 40, resource: particles.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 41, resource: sorted.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 42, resource: uniform.as_entire_binding() },
                ],
            }),
            ResourceKind::BindGroup,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        let id = self.transparency.next_emitter;
        self.transparency.next_emitter += 1;
        let shared = Arc::new(EmitterShared { id, renderer: self.identity.clone() });
        self.transparency.emitters.push(EmitterSlot {
            id,
            handle: Arc::downgrade(&shared),
            params: EmitterParams::new(d),
            sprite: d.texture.cloned(),
            uniform,
            sim,
            particles,
            sorted,
            spawned: 0,
            carry: 0.0,
            _allocation: self.ledger.allocate(AllocationKind::Target, u64::from(d.capacity) * PARTICLE_BYTES + sorted_len * 4 + EMITTER_BYTES),
        });
        Ok(Emitter(shared))
    }

    /// Changes a live emitter's parameters; its particles continue. The capacity and
    /// texture presence must stay the same.
    pub fn try_update_emitter(&mut self, emitter: &Emitter, d: &EmitterDescriptor<'_>) -> Result<(), Error> {
        d.validate()?;
        if !Arc::ptr_eq(&self.identity, &emitter.0.renderer) {
            return Err(Error::InvalidInput("emitter belongs to another renderer"));
        }
        let slot = self.transparency.emitters.iter_mut().find(|s| s.id == emitter.0.id).unwrap();
        if slot.params.capacity != d.capacity || slot.sprite.is_some() != d.texture.is_some() {
            return Err(Error::InvalidInput("emitter updates keep capacity and texture presence"));
        }
        slot.params = EmitterParams::new(d);
        if let Some(texture) = d.texture {
            slot.sprite = Some(texture.clone());
        }
        Ok(())
    }

    /// Particle simulation time per lit frame, seconds (default 1/60). Zero freezes
    /// particles in place.
    pub fn set_particle_step(&mut self, seconds: f32) -> Result<(), Error> {
        if !(seconds.is_finite() && (0.0..=1.0).contains(&seconds)) {
            return Err(Error::InvalidInput("particle step must be in [0, 1] s"));
        }
        // Zero is stored as a tiny step so the default is not reapplied; spawns need dt>0.
        self.transparency.step = if seconds == 0.0 { f32::MIN_POSITIVE } else { seconds };
        Ok(())
    }

    pub fn transparency_stats(&self) -> TransparencyStats {
        self.transparency.stats
    }

    /// Visibility and ID publication for transparent draws, as opaque draws get them.
    pub(super) fn tp_prepare_draws(&mut self, matrix: Mat4, transparent: &mut [Draw<'_>], uploads: &mut UploadBytes) -> Result<(), Error> {
        if transparent.iter().any(|d| {
            !Arc::ptr_eq(&self.identity, &d.instances.renderer) || !Arc::ptr_eq(&self.identity, &d.mesh.renderer)
        }) {
            return Err(Error::InvalidInput("draw resource belongs to another renderer"));
        }
        if transparent.iter().any(|d| d.material.is_some_and(|m| m.normal_mapped) && d.mesh.tangent.is_none()) {
            return Err(Error::InvalidInput("normal-mapped draw requires tangent stream"));
        }
        let (mut packing, mut upload) = (Duration::ZERO, Duration::ZERO);
        for draw in transparent.iter_mut() {
            let update = draw.instances.state.visibility(matrix, draw.mesh.bounds, draw.visibility);
            if let Some(span) = update.changed_id_span {
                self.scratch.clear();
                for &id in &draw.instances.state.ids[span.clone()] {
                    self.scratch.extend_from_slice(&id.to_ne_bytes());
                }
                self.queue.write_buffer(&draw.instances.id_buffer, span.start as u64 * 4, &self.scratch);
                uploads.visible_ids += self.scratch.len() as u64;
                uploads.visible_id_writes += 1;
                uploads.write_calls += 1;
            }
            self.upload_instance_edits(draw.instances, uploads, &mut packing, &mut upload);
        }
        Ok(())
    }

    /// Encodes the transparent pass into `encoder` after opaque shading and before
    /// presentation. `opaque` are the frame's opaque draws (shadow-map casters).
    pub(super) fn encode_transparency(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        opaque: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        traced: bool,
    ) -> Result<(), Error> {
        self.transparency.emitters.retain(|e| e.handle.strong_count() > 0);
        let transparent: Vec<&Draw<'_>> = transparent.iter().filter(|d| !d.instances.state.ids.is_empty()).collect();
        if transparent.is_empty() && self.transparency.emitters.is_empty() {
            return Ok(());
        }
        // 4x raster frames have no single-sample depth to read (docs/m7-tp1.md gaps).
        if !traced && self.options.samples != SampleCount::One {
            return Ok(());
        }
        let Some(size) = self.lighting.hdr.as_ref().map(|h| h.resolved.texture.size()) else {
            return Ok(());
        };
        if self.transparency.gpu.is_none() {
            self.transparency.gpu = Some(self.tp_gpu());
        }
        if self.transparency.step == 0.0 {
            self.transparency.step = 1.0 / 60.0;
        }
        let scopes = OperationScopes::new(&self.device);
        // Targets at the HDR size: the colour behind a transparent draw and opaque depth.
        if self.transparency.targets.as_ref().is_none_or(|t| t.color.texture.size() != size) {
            let generation = self.transparency.targets.as_ref().map_or(0, |t| t.generation + 1);
            let color = self.tp_target("TP1 colour behind", size, wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, 8);
            let depth = self.tp_target("TP1 opaque view depth", size, wgpu::TextureFormat::R32Float,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC, 4);
            self.transparency.targets = Some(Targets { color, depth, generation });
        }
        // The prelude-composed particle kernels follow prelude reloads.
        let prelude: [u8; 32] = Sha256::digest(self.source.as_bytes()).into();
        if !self.transparency.emitters.is_empty()
            && self.transparency.gpu.as_ref().unwrap().particles.as_ref().is_none_or(|p| p.prelude != prelude)
        {
            let pipelines = self.tp_particle_pipelines();
            self.transparency.gpu.as_mut().unwrap().particles = Some(pipelines);
        }
        if traced && self.transparency.gpu.as_ref().unwrap().hit_depth.is_none() {
            let primary = self.primary.as_ref().unwrap();
            let bindings = self.tp_layout("TP1 hit depth", &[
                entry(30, COMPUTE, uniform(false, FRAME_BYTES)),
                entry(50, COMPUTE, texture(wgpu::TextureSampleType::Uint)),
                entry(51, COMPUTE, texture(wgpu::TextureSampleType::Uint)),
                entry(52, COMPUTE, wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::R32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                }),
            ]);
            let layout = self.tp_pipeline_layout("TP1 hit depth", &[&bindings, primary.array_layout()]);
            let module = self.tp_module("TP1 hit depth", &hit_depth_source());
            let pipeline = self.tp_compute("TP1 hit depth", &layout, &module, "tp_depth_hits");
            self.transparency.gpu.as_mut().unwrap().hit_depth = Some((bindings, pipeline));
        }
        // Transparent programs' pipelines, by program and sun-visibility path.
        let rt = self.hardware_shadows.as_ref().is_some_and(|h| h.active_scene().is_some());
        let default_program = self.standard.id();
        // Sources are composed once per program and frame; pipelines only when they change
        // (a program or prelude reload, or the TLAS appearing).
        let mut programs: Vec<u64> = transparent
            .iter()
            .map(|d| d.material.map_or(default_program, |m| m.program.id()))
            .collect();
        programs.sort_unstable();
        programs.dedup();
        for program in programs {
            let (source, key) = self.tp_glass_source(program, rt);
            if self.transparency.glass.iter().any(|g| g.program == program && g.key == key) {
                continue;
            }
            let pipelines = match self.tp_glass_pipelines(program, rt, &source, key) {
                Err(Error::Fault(Fault(message))) => return Err(self.fail(message)),
                other => other?,
            };
            self.transparency.glass.retain(|g| g.program != program);
            self.transparency.glass.push(pipelines);
        }
        // ---- CPU: the frame block, the sun map fit and the draw order ----
        let camera = self.transparency.camera.unwrap();
        let matrix = camera.matrix(size.width, size.height);
        let inverse_view = camera.view().inverse();
        let eye = inverse_view.w_axis.truncate();
        let forward = -inverse_view.z_axis.truncate().normalize();
        let right = inverse_view.x_axis.truncate().normalize();
        let up = inverse_view.y_axis.truncate().normalize();
        let sun = self.lighting.source_lights.iter().find_map(|l| match l {
            crate::Light::Directional { travel, lux } => Some((travel.normalize(), *lux)),
            _ => None,
        });
        let mut receivers: Vec<(Vec3, Vec3)> = Vec::new();
        for draw in &transparent {
            for &model in &draw.instances.state.models {
                let b = shadow_gpu::world_bounds(draw.mesh.bounds, model)?;
                receivers.push((b.min, b.max));
            }
        }
        receivers.extend(self.transparency.emitters.iter().map(|e| e.params.bounds()));
        let fit = sun.and_then(|(travel, _)| shadow_fit(travel, &receivers, opaque));
        let (shadow, bias, texel) = fit.map_or((Mat4::IDENTITY, 0.0, 0.0), |f| (f.matrix, f.bias, f.texel));
        let mut key = Sha256::new();
        key.update(bytes_of(&shadow.to_cols_array()));
        for draw in opaque {
            key.update((Arc::as_ptr(&draw.mesh.vertex) as usize).to_ne_bytes());
            for model in &draw.instances.state.models {
                key.update(bytes_of(&model.to_cols_array()));
            }
        }
        let key: [u8; 32] = key.finalize().into();
        let render_shadow = fit.is_some() && self.transparency.shadow_key != Some(key);
        let mut frame = [0f32; (FRAME_BYTES / 4) as usize];
        frame[0..16].copy_from_slice(&matrix.to_cols_array());
        frame[16..32].copy_from_slice(&matrix.inverse().to_cols_array());
        frame[32..48].copy_from_slice(&shadow.to_cols_array());
        frame[48..52].copy_from_slice(&eye.extend(1.0).to_array());
        frame[52..56].copy_from_slice(&forward.extend(0.0).to_array());
        frame[56..60].copy_from_slice(&right.extend(0.0).to_array());
        frame[60..64].copy_from_slice(&up.extend(0.0).to_array());
        frame[64..68].copy_from_slice(&[size.width as f32, size.height as f32, 1.0 / size.width as f32, 1.0 / size.height as f32]);
        let (toward, lux) = sun.map_or((Vec3::Z, [0.0; 3]), |(t, l)| (-t, l));
        frame[68..72].copy_from_slice(&toward.extend(f32::from(u8::from(fit.is_some()))).to_array());
        frame[72..76].copy_from_slice(&[lux[0], lux[1], lux[2], texel]);
        frame[80..84].copy_from_slice(&[self.transparency.step, bias, SHADOW_SIZE as f32, 0.0]);
        let mut bytes = vec![0u8; FRAME_BYTES as usize];
        for (dst, v) in bytes.chunks_exact_mut(4).zip(frame) {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        let scene = if traced { self.primary.as_ref().unwrap().scene } else { [0; 4] };
        for (k, v) in scene.into_iter().enumerate() {
            bytes[304 + 4 * k..308 + 4 * k].copy_from_slice(&v.to_ne_bytes());
        }
        let gpu = self.transparency.gpu.as_ref().unwrap();
        let mut camera_block = [0u8; 80];
        camera_block[..64].copy_from_slice(&matrix_bytes(matrix));
        camera_block[64..68].copy_from_slice(&(ViewMode::Lit as u32).to_ne_bytes());
        self.queue.write_buffer(&gpu.frame, 0, &camera_block);
        self.queue.write_buffer(&gpu.frame, FRAME_OFFSET, &bytes);
        // Items far to near: transparent draws by their visible instances' mean bounds
        // centre, emitters by their spawn centre.
        enum Item {
            Draw(usize),
            Emitter(usize),
        }
        let mut items: Vec<(f32, Item)> = Vec::new();
        for (i, draw) in transparent.iter().enumerate() {
            let mut centre = Vec3::ZERO;
            for &id in &draw.instances.state.ids {
                let b = shadow_gpu::world_bounds(draw.mesh.bounds, draw.instances.state.models[id as usize])?;
                centre += (b.min + b.max) * 0.5;
            }
            centre /= draw.instances.state.ids.len() as f32;
            items.push(((centre - eye).dot(forward), Item::Draw(i)));
        }
        for (i, e) in self.transparency.emitters.iter().enumerate() {
            items.push(((e.params.position - eye).dot(forward), Item::Emitter(i)));
        }
        items.sort_by(|a, b| b.0.total_cmp(&a.0));
        if transparent.len() > MAX_TRANSPARENT_DRAWS {
            return Err(Error::InvalidInput("more than 64 transparent draws in one frame"));
        }
        let mut draw_slots = vec![0u8; SLOT as usize * transparent.len().max(1)];
        for (i, draw) in transparent.iter().enumerate() {
            let thickness = draw.material.and_then(|m| m.transparency).map_or(0.0, |t| t.thickness);
            draw_slots[i * SLOT as usize..i * SLOT as usize + 4].copy_from_slice(&thickness.to_ne_bytes());
        }
        self.queue.write_buffer(&gpu.draws, 0, &draw_slots);
        // Emitters' spawns for this step.
        let step = f64::from(self.transparency.step);
        for e in &mut self.transparency.emitters {
            e.carry += f64::from(e.params.spawn_rate) * step;
            let spawned = (e.carry.floor() as u64).min(u64::from(e.params.capacity));
            e.carry -= e.carry.floor();
            let first = e.spawned as u32;
            e.spawned += spawned;
            let textured = e.sprite.is_some();
            self.queue.write_buffer(&e.uniform, 0, &e.params.uniform(first, spawned as u32, textured));
        }
        // ---- GPU ----
        let hdr = self.lighting.hdr.as_ref().unwrap();
        let targets = self.transparency.targets.as_ref().unwrap();
        // 1. Opaque view depth.
        {
            let group = if traced {
                let primary = self.primary.as_ref().unwrap();
                let t = primary.targets.as_ref().unwrap();
                let (bindings, _) = gpu.hit_depth.as_ref().unwrap();
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 hit depth"),
                    layout: bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry { binding: 50, resource: wgpu::BindingResource::TextureView(&t.hit.view) },
                        wgpu::BindGroupEntry { binding: 51, resource: wgpu::BindingResource::TextureView(&t.tiles.view) },
                        wgpu::BindGroupEntry { binding: 52, resource: wgpu::BindingResource::TextureView(&targets.depth.view) },
                    ],
                });
                Some(group)
            } else {
                let depth_view = self.depth.as_ref().unwrap().texture.create_view(&Default::default());
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 raster depth"),
                    layout: &gpu.raster_depth_bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry { binding: 52, resource: wgpu::BindingResource::TextureView(&targets.depth.view) },
                        wgpu::BindGroupEntry { binding: 53, resource: wgpu::BindingResource::TextureView(&depth_view) },
                    ],
                }))
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("TP1 opaque depth"), timestamp_writes: None });
            if traced {
                let arrays = self.primary.as_ref().unwrap().virtual_geometry().unwrap().1;
                pass.set_pipeline(&gpu.hit_depth.as_ref().unwrap().1);
                pass.set_bind_group(1, arrays, &[]);
            } else {
                pass.set_pipeline(&gpu.raster_depth);
            }
            pass.set_bind_group(0, group.as_ref().unwrap(), &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
        // 2. The transparent receivers' sun shadow map, when its inputs changed.
        if render_shadow {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TP1 sun shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.shadow.view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&gpu.shadow_pipeline);
            pass.set_bind_group(0, &*gpu.shadow_group, &[]);
            for draw in opaque {
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
                pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..draw.instances.len() as u32);
            }
        }
        // 3. Particles: simulate, light and sort.
        if let Some(p) = &gpu.particles {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("TP1 particles"), timestamp_writes: None });
            pass.set_bind_group(1, &*gpu.empty, &[]);
            pass.set_bind_group(2, &*gpu.empty, &[]);
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            for e in &self.transparency.emitters {
                pass.set_bind_group(0, &*e.sim, &[]);
                pass.set_pipeline(&p.update);
                pass.dispatch_workgroups(e.params.capacity.div_ceil(64), 1, 1);
                pass.set_pipeline(&p.sort);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        // Per-frame groups: the TLAS, targets and shadow map they bind change between frames.
        let rt_scene = self.hardware_shadows.as_ref().and_then(|h| h.active_scene()).filter(|_| rt);
        let mut glass_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &gpu.frame, offset: 0, size: wgpu::BufferSize::new(80) }),
            },
            frame_entry(&gpu.frame),
            wgpu::BindGroupEntry { binding: 31, resource: wgpu::BindingResource::TextureView(&targets.color.view) },
            wgpu::BindGroupEntry { binding: 32, resource: wgpu::BindingResource::TextureView(&targets.depth.view) },
            wgpu::BindGroupEntry { binding: 33, resource: wgpu::BindingResource::Sampler(&gpu.sampler) },
            wgpu::BindGroupEntry { binding: 34, resource: wgpu::BindingResource::TextureView(&gpu.shadow.view) },
            wgpu::BindGroupEntry { binding: 35, resource: wgpu::BindingResource::Sampler(&gpu.compare) },
            wgpu::BindGroupEntry {
                binding: 37,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &gpu.draws, offset: 0, size: wgpu::BufferSize::new(16) }),
            },
        ];
        if let Some(scene) = rt_scene {
            glass_entries.push(wgpu::BindGroupEntry { binding: 2, resource: scene.as_binding() });
        }
        let glass_group = (!transparent.is_empty()).then(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TP1 transparent group 0"),
                layout: &gpu.glass_bindings[usize::from(rt)],
                entries: &glass_entries,
            })
        });
        let draw_groups: Vec<wgpu::BindGroup> = self
            .transparency
            .emitters
            .iter()
            .map(|e| {
                let sprite = e.sprite.as_ref().map_or(&*gpu.white, |t| self.texture_view(t));
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP1 particle draw"),
                    layout: &gpu.draw_bindings,
                    entries: &[
                        frame_entry(&gpu.frame),
                        wgpu::BindGroupEntry { binding: 32, resource: wgpu::BindingResource::TextureView(&targets.depth.view) },
                        wgpu::BindGroupEntry { binding: 33, resource: wgpu::BindingResource::Sampler(&gpu.sampler) },
                        wgpu::BindGroupEntry { binding: 36, resource: wgpu::BindingResource::TextureView(sprite) },
                        wgpu::BindGroupEntry { binding: 40, resource: e.particles.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 41, resource: e.sorted.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 42, resource: e.uniform.as_entire_binding() },
                    ],
                })
            })
            .collect();
        // 4. Back to front. Each transparent draw first copies the colour behind it.
        let hdr_view = &hdr.resolved.view;
        let mut index = 0;
        while index < items.len() {
            if let Item::Draw(_) = items[index].1 {
                encoder.copy_texture_to_texture(hdr.resolved.texture.as_image_copy(), targets.color.texture.as_image_copy(), size);
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TP1 transparent"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_bind_group(3, self.lighting.bind(), &[]);
            loop {
                match items[index].1 {
                    Item::Draw(i) => {
                        let draw = transparent[i];
                        let program = draw.material.map_or(default_program, |m| m.program.id());
                        let set = self.transparency.glass.iter().find(|g| g.program == program).unwrap();
                        let material = draw.material.unwrap_or(self.default_material.as_ref().unwrap());
                        pass.set_pipeline(&set.pipelines[usize::from(draw.mesh.tangent.is_some())]);
                        pass.set_bind_group(0, glass_group.as_ref().unwrap(), &[(i as u64 * SLOT) as u32]);
                        pass.set_bind_group(1, &*draw.instances.bind, &[]);
                        pass.set_bind_group(2, &*material.bind, &[]);
                        pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                        pass.set_vertex_buffer(1, draw.instances.id_buffer.slice(..));
                        if let Some(t) = &draw.mesh.tangent {
                            pass.set_vertex_buffer(2, t.slice(..));
                        }
                        pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..draw.mesh.index_count, 0, 0..draw.instances.state.ids.len() as u32);
                    }
                    Item::Emitter(i) => {
                        let e = &self.transparency.emitters[i];
                        pass.set_pipeline(&gpu.particles.as_ref().unwrap().draw);
                        pass.set_bind_group(0, &draw_groups[i], &[]);
                        pass.set_bind_group(1, &*gpu.empty, &[]);
                        pass.set_bind_group(2, &*gpu.empty, &[]);
                        pass.draw(0..6, 0..e.params.capacity);
                    }
                }
                index += 1;
                // A transparent draw next needs the colour this pass has written so far.
                if index == items.len() || matches!(items[index].1, Item::Draw(_)) {
                    break;
                }
            }
        }
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        if render_shadow {
            self.transparency.shadow_key = Some(key);
            self.transparency.stats.shadow_map_renders += 1;
        }
        let stats = &mut self.transparency.stats;
        stats.transparent_draws = transparent.len() as u32;
        stats.emitters = self.transparency.emitters.len() as u32;
        stats.particle_capacity = self.transparency.emitters.iter().map(|e| e.params.capacity).sum();
        stats.frames += 1;
        Ok(())
    }
}

fn frame_entry(frame: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding: 30,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: frame,
            offset: FRAME_OFFSET,
            size: wgpu::BufferSize::new(FRAME_BYTES),
        }),
    }
}

fn bytes_of(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

#[derive(Clone, Copy)]
struct ShadowFit {
    matrix: Mat4,
    bias: f32,
    texel: f32,
}
/// An orthographic sun map over the receivers' footprint, deep enough toward the sun to
/// hold every opaque caster: square texels, depth 0 at the most sunward caster.
fn shadow_fit(travel: Vec3, receivers: &[(Vec3, Vec3)], opaque: &[PreparedDraw<'_>]) -> Option<ShadowFit> {
    if receivers.is_empty() || !travel.is_finite() {
        return None;
    }
    let w = travel;
    let helper = if w.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = helper.cross(w).normalize();
    let v = w.cross(u);
    let corners = |(min, max): (Vec3, Vec3)| {
        (0..8).map(move |i| Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        ))
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for &r in receivers {
        for p in corners(r) {
            let q = Vec3::new(p.dot(u), p.dot(v), p.dot(w));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    let mut near = lo.z;
    for draw in opaque {
        for &model in &draw.instances.state.models {
            if let Ok(b) = shadow_gpu::world_bounds(draw.mesh.bounds, model) {
                for p in corners((b.min, b.max)) {
                    near = near.min(p.dot(w));
                }
            }
        }
    }
    let margin = 0.25;
    let extent = (hi.x - lo.x).max(hi.y - lo.y) + 2.0 * margin;
    let centre = [(lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5];
    let near = near.max(lo.z - 2000.0) - margin;
    let far = hi.z + margin;
    let depth = (far - near).max(1e-3);
    let half = extent * 0.5;
    let rows = [
        (u / half).extend(-centre[0] / half),
        (v / half).extend(-centre[1] / half),
        (w / depth).extend(-near / depth),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    ];
    let texel = extent / SHADOW_SIZE as f32;
    Some(ShadowFit {
        matrix: Mat4::from_cols(rows[0], rows[1], rows[2], rows[3]).transpose(),
        bias: 1.5 * texel / depth,
        texel,
    })
}

/// Moves transparent draws after opaque ones, keeping each group's order. Returns the
/// opaque count and, when anything moved, each position's original index.
pub(super) fn split(draws: &mut [Draw<'_>]) -> (usize, Option<Vec<usize>>) {
    let transparent = |d: &Draw<'_>| d.material.is_some_and(|m| m.transparency.is_some());
    let opaque = draws.iter().filter(|d| !transparent(d)).count();
    if draws[..opaque].iter().all(|d| !transparent(d)) {
        return (opaque, None);
    }
    let mut order: Vec<usize> = (0..draws.len()).collect();
    order.sort_by_key(|&i| transparent(&draws[i]));
    apply(draws, &order);
    (opaque, Some(order))
}
/// Restores the caller's order after `split`.
pub(super) fn restore(draws: &mut [Draw<'_>], order: &[usize]) {
    let mut inverse = vec![0; order.len()];
    for (position, &original) in order.iter().enumerate() {
        inverse[original] = position;
    }
    apply(draws, &inverse);
}
/// Reorders in place so position k holds the element at `order[k]`.
fn apply(draws: &mut [Draw<'_>], order: &[usize]) {
    let mut done = vec![false; order.len()];
    for start in 0..order.len() {
        if done[start] {
            continue;
        }
        let mut k = start;
        loop {
            done[k] = true;
            let next = order[k];
            if next == start {
                break;
            }
            draws.swap(k, next);
            // After the swap, position `next` holds what position k held before.
            k = next;
        }
    }
}
