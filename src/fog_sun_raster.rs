//! A fog-only sun depth cache. No receiver pages, triangle walks or surface shading.
use super::*;
use glam::{Quat, Vec2, Vec4};
use std::ops::Range;

const SIZE: u32 = 2048;
const LEVELS: usize = 2;
pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RENDERLAB_FOG_SUN_RASTER").as_deref() != Ok("0"))
}
// Timing control: exercise the update path on warm frames, even with static casters.
fn force_refresh() -> bool {
    static REFRESH: OnceLock<bool> = OnceLock::new();
    *REFRESH.get_or_init(|| std::env::var("RENDERLAB_FOG_SUN_REFRESH").as_deref() == Ok("1"))
}

#[derive(PartialEq)]
struct CasterKey {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    instances: (u64, u64),
    pose: u64,
    indices: Range<u32>,
}
#[derive(PartialEq)]
struct Key {
    sun: Vec3,
    distance: f32,
    casters: Vec<CasterKey>,
}
pub(super) struct FogSun {
    _texture: Owned<wgpu::Texture>,
    pub(super) sample: Owned<wgpu::TextureView>,
    layers: [Owned<wgpu::TextureView>; LEVELS],
    pub(super) uniform: Owned<wgpu::Buffer>,
    raster_uniforms: [Owned<wgpu::Buffer>; LEVELS],
    groups: [Owned<wgpu::BindGroup>; LEVELS],
    pipeline: Owned<wgpu::RenderPipeline>,
    _allocation: Allocation,
    key: Option<Key>,
    centers: [Vec2; LEVELS],
    radii: [f32; LEVELS],
    depth: [f32; 2],
    casters: [Vec<(usize, Range<u32>)>; LEVELS],
    dirty: bool,
}
impl FogSun {
    fn new(device: &PipelineDevice, ledger: &Ledger, instances: &wgpu::BindGroupLayout) -> Self {
        let texture = ledger.own(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fog sun depth cache"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: LEVELS as u32 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }), ResourceKind::Texture);
        let sample = ledger.own(texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        }), ResourceKind::TextureView);
        let layers = std::array::from_fn(|i| ledger.own(texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i as u32,
            array_layer_count: Some(1), ..Default::default()
        }), ResourceKind::TextureView));
        let buffer = |size| ledger.own(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog sun matrices"), size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }), ResourceKind::Buffer);
        let uniform = buffer(144);
        let raster_uniforms = std::array::from_fn(|_| buffer(64));
        let bindings = ledger.own(device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog sun raster matrix"), entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        }), ResourceKind::BindGroupLayout);
        let groups = std::array::from_fn(|i| ledger.own(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog sun raster matrix"), layout: &bindings,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: raster_uniforms[i].as_entire_binding() }],
        }), ResourceKind::BindGroup));
        let module = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog sun depth"), source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fog_sun_depth.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fog sun depth"), bind_group_layouts: &[Some(&bindings), Some(instances)], immediate_size: 0,
        });
        let pipeline = ledger.own(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fog sun depth"), layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module, entry_point: Some("fog_sun_vertex"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 48, step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            fragment: None, primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default(),
            }),
            multisample: Default::default(), multiview_mask: None, cache: None,
        }), ResourceKind::RenderPipeline);
        Self {
            _texture: texture, sample, layers, uniform, raster_uniforms, groups, pipeline,
            _allocation: ledger.allocate(AllocationKind::Shadow, u64::from(SIZE).pow(2) * LEVELS as u64 * 4 + 272),
            key: None, centers: [Vec2::ZERO; LEVELS], radii: [0.0; LEVELS], depth: [0.0; 2],
            casters: std::array::from_fn(|_| Vec::new()), dirty: false,
        }
    }

    fn prepare(&mut self, queue: &wgpu::Queue, draws: &[PreparedDraw<'_>], eye: Vec3, sun: Vec3, distance: f32) -> Result<(), Error> {
        let key = Key { sun, distance, casters: draws.iter().map(|d| CasterKey {
            vertex: (**d.mesh.vertex).clone(), index: (**d.mesh.index).clone(),
            instances: d.instances.state.revision(), pose: d.mesh.skin.as_ref().map_or(0, |s| s.revision),
            indices: d.indices.clone(),
        }).collect() };
        let rotation = Quat::from_rotation_arc(Vec3::Z, sun);
        let x = rotation * Vec3::X;
        let y = rotation * Vec3::Y;
        let eye_xy = Vec2::new(x.dot(eye), y.dot(eye));
        let eye_z = sun.dot(eye);
        // The outer square contains every point in the radial fog volume. The
        // guard band lets ordinary camera motion retain the exact world grid.
        let inside = (eye_xy - self.centers[1]).abs().max_element() + distance < self.radii[1]
            && eye_z - distance > self.depth[0] && eye_z + distance < self.depth[1];
        let near_inside = (eye_xy - self.centers[0]).abs().max_element() + distance.min(32.0) < self.radii[0];
        if !force_refresh() && self.key.as_ref() == Some(&key) && inside && near_inside { return Ok(()); }
        self.radii = [distance.min(32.0).max(1.0) * 1.25, distance.max(1.0) * 1.25];
        self.centers = self.radii.map(|r| (eye_xy / (r / 8.0)).round() * (r / 8.0));
        self.depth = [eye_z - self.radii[1], eye_z + self.radii[1]];
        self.casters.iter_mut().for_each(Vec::clear);
        for (draw_index, draw) in draws.iter().enumerate() {
            let bounds = draw.instances.state.world_bounds(draw.mesh.bounds)?;
            for (slot, b) in bounds.iter().enumerate() {
                if !draw.instances.state.casts_shadows(slot) { continue; }
                let center = (b.min + b.max) * 0.5;
                let extent = (b.max - b.min) * 0.5;
                let xy = Vec2::new(x.dot(center), y.dot(center));
                let half = Vec2::new(x.abs().dot(extent), y.abs().dot(extent));
                let z = sun.dot(center);
                let dz = sun.abs().dot(extent);
                self.depth[0] = self.depth[0].min(z - dz - 1.0);
                self.depth[1] = self.depth[1].max(z + dz + 1.0);
                for i in 0..LEVELS {
                    if ((xy - self.centers[i]).abs() - half).max_element() > self.radii[i] { continue; }
                    let slot = slot as u32;
                    match self.casters[i].last_mut() {
                        Some((d, range)) if *d == draw_index && range.end == slot => range.end += 1,
                        _ => self.casters[i].push((draw_index, slot..slot + 1)),
                    }
                }
            }
        }
        let depth_range = (self.depth[1] - self.depth[0]).max(1.0);
        let clips = std::array::from_fn::<_, LEVELS, _>(|i| {
            let r = self.radii[i];
            Mat4::from_cols(
                Vec4::new(x.x/r,y.x/r,sun.x/depth_range,0.0),
                Vec4::new(x.y/r,y.y/r,sun.y/depth_range,0.0),
                Vec4::new(x.z/r,y.z/r,sun.z/depth_range,0.0),
                Vec4::new(-self.centers[i].x/r,-self.centers[i].y/r,-self.depth[0]/depth_range,1.0),
            )
        });
        let mut bytes = Vec::with_capacity(144);
        for (i, clip) in clips.iter().enumerate() {
            let matrix = matrix_bytes(*clip);
            queue.write_buffer(&self.raster_uniforms[i], 0, &matrix);
            bytes.extend(matrix);
        }
        // The old fog ray begins 2 cm towards the sun. No normal/slope bias is
        // appropriate for a volume sample, which has no receiver surface.
        bytes.extend([0.02/depth_range, 0.0, 0.0, 0.0].into_iter().flat_map(f32::to_ne_bytes));
        queue.write_buffer(&self.uniform, 0, &bytes);
        self.key = Some(key);
        self.dirty = true;
        Ok(())
    }
}

impl Renderer {
    pub(in crate::renderer) fn prepare_fog_sun_map(&mut self, draws: &[PreparedDraw<'_>], camera: &Camera, uploads: &mut UploadBytes) -> Result<(), Error> {
        if !enabled() || self.hardware_shadows.is_none() || !self.atmosphere.active || self.atmosphere.settings.volumetric_fog.is_none() { return Ok(()); }
        if self.atmosphere.fog_sun.is_none() {
            let scopes = OperationScopes::new(&self.device);
            let map = FogSun::new(&self.device, &self.ledger, &self.instance_bindings);
            if let Some(error) = scopes.finish() { return Err(self.fail(error.to_string())); }
            self.atmosphere.fog_sun = Some(map);
            self.pipelines.created += 1;
        }
        let eye = camera.view().inverse().w_axis.truncate();
        let sun = self.atmosphere_sun().map_or(Vec3::Y, |s| s.0);
        let distance = self.atmosphere.settings.volumetric_fog.unwrap().distance;
        let map = self.atmosphere.fog_sun.as_mut().unwrap();
        map.prepare(&self.queue, draws, eye, sun, distance)?;
        if map.dirty { uploads.shadows += 272; uploads.write_calls += 3; }
        Ok(())
    }

    pub(in crate::renderer) fn encode_fog_sun_map(&mut self, encoder: &mut wgpu::CommandEncoder, draws: &[PreparedDraw<'_>], mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str)) {
        if !self.atmosphere.active || self.atmosphere.settings.volumetric_fog.is_none() { return; }
        let Some(map) = self.atmosphere.fog_sun.as_mut().filter(|m| m.dirty) else { return; };
        mark(encoder, "fog_sun_map");
        for i in 0..LEVELS {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fog sun current depth"), color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &map.layers[i], depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None,
                }), ..Default::default()
            });
            pass.set_pipeline(&map.pipeline);
            pass.set_bind_group(0, &*map.groups[i], &[]);
            for (draw_index, slots) in &map.casters[i] {
                let draw = &draws[*draw_index];
                pass.set_bind_group(1, &*draw.instances.bind, &[]);
                pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(draw.indices.clone(), 0, slots.clone());
            }
        }
        map.dirty = false;
        mark(encoder, "after_fog_sun_map");
    }
}
