//! TP2 coloured sun transmission. This module owns the map and its preparation;
//! lighting and frame-order hooks are the only changes outside transparency.
use super::*;
use std::hash::{Hash, Hasher};

const SIZE: u32 = 1024;
const BYTES: u64 = 96;
const API: &str = include_str!("../shaders/transparency_shadow.wgsl");
const DRAW: &str = r#"
struct TpTransmission { matrix:mat4x4<f32>, sun:vec4<f32>, info:vec4<f32> };
@group(0) @binding(30) var<uniform> tx:TpTransmission;
struct TxVertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, @location(1) color:vec4<f32>, @location(2) world:vec3<f32> };
@vertex fn tx_vs(@location(0) position:vec3<f32>, @location(2) uv:vec2<f32>, @location(3) color:vec4<f32>, @builtin(instance_index) slot:u32) -> TxVertex {
    let world=world_position(position,slot);
    return TxVertex(tx.matrix*world,uv,color,world.xyz);
}
@fragment fn tx_fs(input:TxVertex) -> @location(0) vec4<f32> {
    let sampled=textureSample(base_color,base_sampler,input.uv)*material.base_color*input.color;
    let packed=unpack2x16float(material.pad);
    let coverage=clamp(packed.x*sampled.a,0.0,1.0);
    let ior=max(packed.y,1.0);
    let f0=(ior-1.0)/(ior+1.0);
    let normal=normalize(cross(dpdx(input.world),dpdy(input.world)));
    let fresnel=f0*f0+(1.0-f0*f0)*pow(1.0-clamp(abs(dot(normal,tx.sun.xyz)),0.0,1.0),5.0);
    return vec4<f32>((1.0-coverage)*(1.0-fresnel)*clamp(sampled.rgb,vec3<f32>(0.0),vec3<f32>(1.0)),input.position.z);
}
"#;

pub(crate) fn api_source() -> &'static str {
    API
}
pub(crate) fn draw_source(head: &str) -> String {
    format!("{head}{DRAW}")
}
pub(crate) fn binding_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        entry(26, FRAGMENT | COMPUTE, uniform(false, BYTES)),
        entry(
            27,
            FRAGMENT | COMPUTE,
            wgpu::BindingType::Texture {
                sample_type: UNFILTERED,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
        ),
    ]
}
pub(in crate::renderer) struct TransmissionShadow {
    pub uniform: Owned<wgpu::Buffer>,
    pub map: Depth,
    pub generation: u64,
    pub hash: [u8; 32],
    layers: Vec<Owned<wgpu::TextureView>>,
    depth: Option<Depth>,
    pipeline: Option<Owned<wgpu::RenderPipeline>>,
    group: Option<Owned<wgpu::BindGroup>>,
    key: Option<[u8; 32]>,
    render: bool,
}
impl TransmissionShadow {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let uniform = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TP2 transmission"),
                size: BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let map = map(device, ledger, 1, 1);
        Self {
            uniform,
            map,
            generation: 0,
            hash: [0; 32],
            layers: vec![],
            depth: None,
            pipeline: None,
            group: None,
            key: None,
            render: false,
        }
    }
}
fn map(device: &wgpu::Device, ledger: &Ledger, size: u32, layers: u32) -> Depth {
    let texture = ledger.own(
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("TP2 coloured transmission"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = ledger.own(
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        }),
        ResourceKind::TextureView,
    );
    Depth {
        _allocation: ledger.allocate(
            AllocationKind::Target,
            u64::from(size) * u64::from(size) * u64::from(layers) * 8,
        ),
        texture,
        view,
    }
}
impl Renderer {
    pub(crate) fn prepare_transmission(&mut self, transparent: &[Draw<'_>]) -> Result<(), Error> {
        let sun = self.lighting.source_lights.iter().find_map(|l| match l {
            Light::Directional { travel, .. } => Some(travel.normalize()),
            _ => None,
        });
        #[cfg(test)]
        let sun = if std::env::var("RENDERLAB_TP2_PROFILE").is_ok_and(|s| !s.is_empty()) {
            None
        } else {
            sun
        };
        let mut receivers = vec![];
        for draw in transparent {
            for &model in &draw.instances.state.models {
                let b = shadow_gpu::world_bounds(draw.mesh.bounds, model)?;
                receivers.push((b.min, b.max));
            }
        }
        let fit = sun.and_then(|s| shadow_fit(s, &receivers, &[]));
        let count = if fit.is_some() { transparent.len() } else { 0 };
        if count > MAX_TRANSPARENT_DRAWS
            || count as u32 > self.device.limits().max_texture_array_layers
        {
            return Err(Error::InvalidInput(
                "more than TRACED_DRAWS transparent shadow draws",
            ));
        }
        let matrix = fit.map_or(Mat4::IDENTITY, |f| f.matrix);
        let mut data = bytes_of(&matrix.to_cols_array());
        let toward = -sun.unwrap_or(Vec3::Z);
        data.extend(bytes_of(&toward.extend(count as f32).to_array()));
        data.extend(bytes_of(&[
            fit.map_or(0.0, |f| f.bias) * 2.0,
            0.0,
            0.0,
            0.0,
        ]));
        let mut key = Sha256::new();
        key.update(&data);
        key.update(self.source.as_bytes());
        for d in transparent {
            let mut identity = std::collections::hash_map::DefaultHasher::new();
            (**d.mesh.vertex).hash(&mut identity);
            (**d.mesh.index).hash(&mut identity);
            if let Some(material) = d.material {
                (*material._uniform).hash(&mut identity);
            }
            key.update(identity.finish().to_ne_bytes());
            for m in &d.instances.state.models {
                key.update(bytes_of(&m.to_cols_array()));
            }
        }
        let key: [u8; 32] = key.finalize().into();
        if self.lighting.transmission.hash != key {
            self.cut_reconstruction();
        }
        self.lighting.transmission.hash = key;
        self.queue
            .write_buffer(&self.lighting.transmission.uniform, 0, &data);
        self.lighting.transmission.render = false;
        if count == 0 || self.lighting.transmission.key == Some(key) {
            return Ok(());
        }
        if self.lighting.transmission.layers.len() != count {
            let map = map(&self.device, &self.ledger, SIZE, count as u32);
            let layers = (0..count)
                .map(|i| {
                    self.ledger.own(
                        map.texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_array_layer: i as u32,
                            array_layer_count: Some(1),
                            ..Default::default()
                        }),
                        ResourceKind::TextureView,
                    )
                })
                .collect();
            let depth = self.tp_target(
                "TP2 transmission depth",
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                4,
            );
            let state = &mut self.lighting.transmission;
            state.map = map;
            state.layers = layers;
            state.depth = Some(depth);
            state.generation += 1;
        }
        if self.lighting.transmission.pipeline.is_none() {
            let bindings = self.tp_layout(
                "TP2 transmission",
                &[entry(
                    30,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    uniform(false, BYTES),
                )],
            );
            let layout = self.tp_pipeline_layout(
                "TP2 transmission",
                &[&bindings, &self.instance_bindings, &self.material_bindings],
            );
            let module = self.tp_module("TP2 transmission", &draw_source(&self.source));
            let pipeline = self.ledger.own(
                self.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("TP2 transmission"),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("tx_vs"),
                            compilation_options: Default::default(),
                            buffers: &[Some(wgpu::VertexBufferLayout {
                                array_stride: 48,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &VERTEX_ATTRIBUTES,
                            })],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("tx_fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: None,
                            ..Default::default()
                        },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: Some(true),
                            depth_compare: Some(wgpu::CompareFunction::Less),
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        multiview_mask: None,
                        cache: None,
                    }),
                ResourceKind::RenderPipeline,
            );
            let group = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("TP2 transmission"),
                    layout: &bindings,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 30,
                        resource: self.lighting.transmission.uniform.as_entire_binding(),
                    }],
                }),
                ResourceKind::BindGroup,
            );
            let state = &mut self.lighting.transmission;
            state.pipeline = Some(pipeline);
            state.group = Some(group);
        }
        let state = &mut self.lighting.transmission;
        state.key = Some(key);
        state.render = true;
        Ok(())
    }
    pub(crate) fn encode_transmission(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[Draw<'_>],
    ) {
        let state = &self.lighting.transmission;
        if !state.render {
            return;
        }
        for (i, draw) in draws.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TP2 coloured sun transmission"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &state.layers[i],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.depth.as_ref().unwrap().view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(state.pipeline.as_ref().unwrap());
            pass.set_bind_group(0, &**state.group.as_ref().unwrap(), &[]);
            pass.set_bind_group(1, &*draw.instances.bind, &[]);
            pass.set_bind_group(2, &*draw.material.unwrap().bind, &[]);
            pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
            pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            // Draw every instance, including off-camera casters. instance_index is used
            // directly instead of the camera's compact visible ID vertex stream.
            pass.draw_indexed(0..draw.mesh.index_count, 0, 0..draw.instances.len() as u32);
        }
    }
}
