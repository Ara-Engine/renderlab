//! Original unlit/diagnostic indexed mesh path; lit programs are a separate port.
use super::{
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    post_surface::RasterDepth,
    shader::{MeshResource as R, Pass},
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    raster::{RasterState, VertexAttribute, VertexBinding, VertexFormat, VertexLayout, VertexStep},
    sampler::{CompareOp, SamplerDesc},
    texture::SampleCount,
    vk,
};

/// Persistent indexed geometry, instance slots and affine model/normal records.
pub struct MeshGeometry {
    buffers: [StorageBuffer; 5],
    index_count: u32,
    instances: u32,
}
impl MeshGeometry {
    /// Validate the same finite geometry/normal domain as Renderlab; policy limits
    /// remain the caller's responsibility. Slots index the supplied model table.
    pub fn new(
        gpu: &mut GpuDevice,
        vertices: &[crate::Vertex],
        indices: &[u32],
        slots: &[u32],
        models: &[crate::Mat4],
    ) -> Result<Self> {
        crate::MeshData { vertices, indices }.validate(crate::Limits {
            max_mesh_bytes: u64::MAX,
            ..Default::default()
        })?;
        ensure!(
            !slots.is_empty()
                && slots.len() <= u32::MAX as usize
                && slots.iter().all(|s| (*s as usize) < models.len()),
            "invalid mesh instance slots"
        );
        let (matrices, normals) = pack_models(models)?;
        let vertex_bytes: Vec<u8> = vertices
            .iter()
            .flat_map(|v| {
                v.position
                    .into_iter()
                    .chain(v.normal)
                    .chain(v.uv)
                    .chain(v.color)
            })
            .flat_map(f32::to_le_bytes)
            .collect();
        let bytes: [&[u8]; 5] = [
            &vertex_bytes,
            bytemuck::cast_slice(indices),
            bytemuck::cast_slice(slots),
            &matrices,
            &normals,
        ];
        let usage = [
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::BufferUsageFlags::INDEX_BUFFER,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::BufferUsageFlags::empty(),
            vk::BufferUsageFlags::empty(),
        ];
        let mut buffers = Vec::new();
        for (bytes, flags) in bytes.into_iter().zip(usage) {
            match StorageBuffer::with_usage(gpu, bytes, flags) {
                Ok(b) => buffers.push(b),
                Err(e) => {
                    for b in buffers {
                        b.retire(gpu)?;
                    }
                    return Err(e);
                }
            }
        }
        Ok(Self {
            buffers: buffers.try_into().expect("five mesh allocations"),
            index_count: indices.len() as u32,
            instances: slots.len() as u32,
        })
    }
    /// Import coherent initialization before the first draw.
    pub fn import(&self, graph: &mut Graph) {
        for b in &self.buffers {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
    }
    /// Model storage for programmable shader input packets.
    /// The 48-byte vertex records, readable as storage (`[position, normal.x]`,
    /// `[normal.yz, uv]`, `colour` per vertex).
    pub fn vertices(&self) -> &StorageBuffer {
        &self.buffers[0]
    }
    /// The triangle list's indices, readable as storage.
    pub fn indices(&self) -> &StorageBuffer {
        &self.buffers[1]
    }
    pub fn models(&self) -> &StorageBuffer {
        &self.buffers[3]
    }
    /// Affine normal/winding storage for programmable shader input packets.
    pub fn normals(&self) -> &StorageBuffer {
        &self.buffers[4]
    }
    pub(crate) fn raster_uses(&self) -> [(Resource, Access); 3] {
        [
            (Resource::Buffer(self.buffers[0].buffer), Access::Vertex),
            (Resource::Buffer(self.buffers[1].buffer), Access::Index),
            (Resource::Buffer(self.buffers[2].buffer), Access::Vertex),
        ]
    }
    pub(crate) fn draw(&self, rec: &mut Recorder) {
        rec.push(Command::DrawVertexBuffers {
            vertices: vec![
                VertexBinding {
                    buffer: self.buffers[0].buffer,
                    offset: 0,
                },
                VertexBinding {
                    buffer: self.buffers[2].buffer,
                    offset: 0,
                },
            ],
            indices: self.buffers[1].buffer,
            first_index: 0,
            index_count: self.index_count,
            vertex_offset: 0,
            instances: self.instances,
            first_instance: 0,
        });
    }
    // Shadow source uses builtin(instance_index), not the location4 slot stream.
    pub(crate) fn native_instance_uses(&self) -> [(Resource, Access); 2] {
        [
            (Resource::Buffer(self.buffers[0].buffer), Access::Vertex),
            (Resource::Buffer(self.buffers[1].buffer), Access::Index),
        ]
    }
    pub(crate) fn draw_native_instances(&self, rec: &mut Recorder, first: u32, count: u32) {
        rec.push(Command::DrawVertexBuffers {
            vertices: vec![VertexBinding {
                buffer: self.buffers[0].buffer,
                offset: 0,
            }],
            indices: self.buffers[1].buffer,
            first_index: 0,
            index_count: self.index_count,
            vertex_offset: 0,
            instances: count,
            first_instance: first,
        });
    }
    /// Queue a bounded model/normal-table range after prior draws. Geometry,
    /// slots and heap identities remain stable while earlier frames are in flight.
    pub fn update_models(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        first: u32,
        models: &[crate::Mat4],
    ) -> Result<()> {
        ensure!(
            !models.is_empty()
                && u64::from(first)
                    .checked_add(models.len() as u64)
                    .is_some_and(|end| end <= self.buffers[3].len / 64),
            "model update range is outside the table"
        );
        let (matrices, normals) = pack_models(models)?;
        graph.node(
            rec,
            &[
                (
                    Resource::Buffer(self.buffers[3].buffer),
                    Access::CopyDestination,
                ),
                (
                    Resource::Buffer(self.buffers[4].buffer),
                    Access::CopyDestination,
                ),
            ],
        )?;
        for (buffer, bytes) in [
            (self.buffers[3].buffer, matrices),
            (self.buffers[4].buffer, normals),
        ] {
            rec.push(Command::UpdateBuffer {
                destination: buffer,
                offset: u64::from(first) * 64,
                bytes,
            });
        }
        Ok(())
    }
    /// Retire after submitting the last referenced draw.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        for b in self.buffers {
            graph.forget(Resource::Buffer(b.buffer));
            b.retire(gpu)?;
        }
        Ok(())
    }
}
/// Original 80-byte camera ABI: matrix and diagnostic view mode 0..3.
pub struct DiagnosticCamera {
    /// Storage record to import and retire.
    pub record: StorageBuffer,
}
impl DiagnosticCamera {
    /// Publish a finite matrix and one of color, normal, metallic or roughness modes.
    pub fn new(gpu: &mut GpuDevice, matrix: crate::Mat4, mode: u32) -> Result<Self> {
        ensure!(matrix.is_finite() && mode <= 3, "invalid diagnostic camera");
        let mut words = [0u32; 20];
        words[..16].copy_from_slice(&matrix.to_cols_array().map(f32::to_bits));
        words[16] = mode;
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&words))?,
        })
    }
    /// Retire the camera record and its graph state after the last submission.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Buffer(self.record.buffer));
        self.record.retire(gpu)
    }
}
/// Original material factor record; sampled textures are explicit draw inputs.
pub struct DiagnosticMaterial {
    /// Storage record to import and retire.
    pub record: StorageBuffer,
}
impl DiagnosticMaterial {
    /// Reuse Renderlab's material writer, without constructing wgpu resources.
    pub fn new(
        gpu: &mut GpuDevice,
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        double_sided: bool,
    ) -> Result<Self> {
        ensure!(
            base_color
                .into_iter()
                .chain([metallic, roughness])
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
            "invalid diagnostic material factors"
        );
        let desc = crate::MaterialDescriptor {
            base_color,
            metallic,
            roughness,
            double_sided,
            ..Default::default()
        };
        Ok(Self {
            record: StorageBuffer::new(gpu, &desc.bytes())?,
        })
    }
    /// Retire the material record and its graph state after the last submission.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Buffer(self.record.buffer));
        self.record.retire(gpu)
    }
}
/// One complete diagnostic draw's owned inputs, borrowed for recording.
pub struct DiagnosticInputs<'a> {
    /// Vertex/index/model/instance records.
    pub geometry: &'a MeshGeometry,
    /// Camera matrix and view mode.
    pub camera: &'a DiagnosticCamera,
    /// Tint/metallic/roughness/winding factors.
    pub material: &'a DiagnosticMaterial,
    /// Linear shader-view base-color image (sRGB decoding is a texture-view concern).
    pub base: &'a Texture,
    /// Linear metallic/roughness image.
    pub metallic_roughness: &'a Texture,
}
/// Persistent original vs/fs pipeline and two immutable material samplers.
pub struct DiagnosticMesh {
    pipeline: Pipeline,
    samplers: [u32; 2],
    format: Format,
}
impl DiagnosticMesh {
    /// Preserve explicit 48-byte vertices, 4-byte per-instance slots and reversed depth.
    pub fn new(
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        format: Format,
        samplers: [SamplerDesc; 2],
    ) -> Result<Self> {
        ensure!(
            samplers.iter().all(|s| s.compare.is_none()),
            "material samplers must use the regular table"
        );
        let samplers = [
            gpu.sampler(&samplers[0])?.index,
            gpu.sampler(&samplers[1])?.index,
        ];
        let state = mesh_state(format);
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::MeshDiagnostic, state)?,
            samplers,
            format,
        })
    }
    /// Clear color/depth and draw every supplied instance. No material hardware cull;
    /// the unchanged fragment rejects source backfaces using the normal-table sign.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &DiagnosticInputs<'_>,
        output: &Texture,
        depth: &RasterDepth,
        clear: [f32; 4],
    ) -> Result<()> {
        ensure!(
            output.format == self.format
                && output.size == depth.size()
                && output.shape.samples == SampleCount::One
                && depth.samples() == SampleCount::One,
            "diagnostic attachment mismatch"
        );
        let g = inputs.geometry;
        let b = &g.buffers;
        graph.node(
            rec,
            &[
                (Resource::Buffer(b[0].buffer), Access::Vertex),
                (Resource::Buffer(b[1].buffer), Access::Index),
                (Resource::Buffer(b[2].buffer), Access::Vertex),
                (Resource::Buffer(b[3].buffer), Access::ShaderRead),
                (Resource::Buffer(b[4].buffer), Access::ShaderRead),
                (
                    Resource::Buffer(inputs.camera.record.buffer),
                    Access::ShaderRead,
                ),
                (
                    Resource::Buffer(inputs.material.record.buffer),
                    Access::ShaderRead,
                ),
                (Resource::Image(inputs.base.image), Access::ShaderRead),
                (
                    Resource::Image(inputs.metallic_roughness.image),
                    Access::ShaderRead,
                ),
                (Resource::Image(output.image), Access::Color),
                (Resource::Image(depth.image()), Access::Depth),
            ],
        )?;
        let mut words = [0; 8];
        for (r, slot) in [
            (R::Camera, inputs.camera.record.slot),
            (R::Models, b[3].slot),
            (R::Normals, b[4].slot),
            (R::Material, inputs.material.record.slot),
            (R::Base, inputs.base.sampled),
            (R::MetallicRoughness, inputs.metallic_roughness.sampled),
            (R::BaseSampler, self.samplers[0]),
            (R::MrSampler, self.samplers[1]),
        ] {
            words[r as usize] = slot;
        }
        rec.push(Command::BeginPass("ara::DiagnosticMesh".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: output.image,
                clear: Some(clear),
            }],
            depth: Some(Attachment {
                image: depth.image(),
                clear: Some([0.; 4]),
            }),
            size: output.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(RootConstants { words }));
        rec.push(Command::DrawVertexBuffers {
            vertices: vec![
                VertexBinding {
                    buffer: b[0].buffer,
                    offset: 0,
                },
                VertexBinding {
                    buffer: b[2].buffer,
                    offset: 0,
                },
            ],
            indices: b[1].buffer,
            first_index: 0,
            index_count: g.index_count,
            vertex_offset: 0,
            instances: g.instances,
            first_instance: 0,
        });
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}

pub(crate) fn pack_models(models: &[crate::Mat4]) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut normals = Vec::new();
    let mut matrices = Vec::new();
    for m in models {
        let n = crate::normal::NormalTransform::new(*m);
        ensure!(
            m.is_finite() && n.valid(),
            "unsupported affine mesh transform"
        );
        normals.extend(n.bytes());
        matrices.extend(bytemuck::cast_slice::<f32, u8>(&m.to_cols_array()));
    }
    Ok((matrices, normals))
}

pub(crate) fn mesh_state(format: Format) -> RenderState {
    RenderState {
        color_formats: vec![format.vk().as_raw()],
        depth_format: Some(Format::Depth32Float.vk().as_raw()),
        depth_test: true,
        depth_write: true,
        blend: false,
        raster: RasterState {
            depth_compare: CompareOp::Greater,
            vertex_layouts: vec![
                VertexLayout {
                    stride: 48,
                    step: VertexStep::Vertex,
                    attributes: vec![
                        VertexAttribute {
                            location: 0,
                            format: VertexFormat::Float32x3,
                            offset: 0,
                        },
                        VertexAttribute {
                            location: 1,
                            format: VertexFormat::Float32x3,
                            offset: 12,
                        },
                        VertexAttribute {
                            location: 2,
                            format: VertexFormat::Float32x2,
                            offset: 24,
                        },
                        VertexAttribute {
                            location: 3,
                            format: VertexFormat::Float32x4,
                            offset: 32,
                        },
                    ],
                },
                VertexLayout {
                    stride: 4,
                    step: VertexStep::Instance,
                    attributes: vec![VertexAttribute {
                        location: 4,
                        format: VertexFormat::Uint32,
                        offset: 0,
                    }],
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    }
}
