//! SK1 on the common heap; ten resources are selected through one 40-byte record.
use super::{
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    shader::{Pass, SkinResource as R},
};
use crate::{Mat4, Vertex, asset::skin::Influence};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    vk,
};

/// Prepared source data; mappings are (destination u32 word, source vertex).
pub struct SkinningDesc<'a> {
    /// Original 48-byte source vertex records.
    pub vertices: &'a [Vertex],
    /// Four-joint weights per source vertex.
    pub influences: &'a [Influence],
    /// Optional original tangent vec4 stream.
    pub tangents: Option<&'a [[f32; 4]]>,
    /// Duplicate cluster-position destinations; unchanged topology remains in words.
    pub mappings: &'a [[u32; 2]],
    /// Retained source-cluster words to update in place.
    pub cluster_words: &'a [u32],
    /// Optional AS-input/device-address roles for an RT-enabled device.
    pub acceleration_usage: vk::BufferUsageFlags,
}
/// Persistent source/output streams and one completion-safe heap-index record.
pub struct SkinningBuffers {
    resources: [StorageBuffer; 10],
    table: StorageBuffer,
    vertices: u32,
    mappings: u32,
    joints: usize,
    tangents: bool,
}
impl SkinningBuffers {
    /// Validate and publish a frozen mesh/palette. Initial history matches the first pose.
    pub fn new(gpu: &mut GpuDevice, desc: SkinningDesc<'_>, palette: &[Mat4]) -> Result<Self> {
        let count = u32::try_from(desc.vertices.len())?;
        let mapping_count = u32::try_from(desc.mappings.len())?;
        ensure!(
            count > 0 && !palette.is_empty() && palette.len() <= 65536,
            "invalid skin vertex/joint count"
        );
        ensure!(
            desc.influences.len() == desc.vertices.len(),
            "influence count mismatch"
        );
        validate_palette(palette, palette.len())?;
        for influence in desc.influences {
            ensure!(
                influence
                    .joints
                    .iter()
                    .all(|&j| usize::from(j) < palette.len())
                    && influence.weights.iter().all(|v| v.is_finite() && *v >= 0.)
                    && (influence.weights.iter().sum::<f32>() - 1.).abs() <= 1e-4,
                "invalid skin influence"
            );
        }
        let vertex_values: Vec<f32> = desc
            .vertices
            .iter()
            .flat_map(|v| {
                v.position
                    .into_iter()
                    .chain(v.normal)
                    .chain(v.uv)
                    .chain(v.color)
            })
            .collect();
        ensure!(
            vertex_values.iter().all(|v| v.is_finite()),
            "nonfinite source vertex"
        );
        if let Some(tangents) = desc.tangents {
            ensure!(
                tangents.len() == desc.vertices.len()
                    && tangents.iter().flatten().all(|v| v.is_finite()),
                "invalid tangent stream"
            );
        }
        for mapping in desc.mappings {
            ensure!(
                mapping[1] < count
                    && (mapping[0] as usize)
                        .checked_add(3)
                        .is_some_and(|n| n <= desc.cluster_words.len()),
                "cluster position mapping out of range"
            );
        }
        let vertices = bytemuck::cast_slice(&vertex_values).to_vec();
        let mut influences = Vec::new();
        for influence in desc.influences {
            for j in influence.joints {
                influences.extend(u32::from(j).to_le_bytes());
            }
            for w in influence.weights {
                influences.extend(w.to_le_bytes());
            }
        }
        let mut previous = vertices.clone();
        previous.extend([0; 16]);
        let tangent_values = desc
            .tangents
            .map_or_else(|| vec![[0f32; 4]], |v| v.to_vec());
        let tangent_bytes = bytemuck::cast_slice(&tangent_values).to_vec();
        let params =
            bytemuck::cast_slice(&[count, mapping_count, u32::from(desc.tangents.is_some()), 1])
                .to_vec();
        let mappings = if desc.mappings.is_empty() {
            vec![0u8; 8]
        } else {
            bytemuck::cast_slice(desc.mappings).to_vec()
        };
        let words = if desc.cluster_words.is_empty() {
            vec![0u8; 4]
        } else {
            bytemuck::cast_slice(desc.cluster_words).to_vec()
        };
        let mut bytes = [
            vertices.clone(),
            influences,
            palette_bytes(palette),
            vertices,
            previous,
            params,
            tangent_bytes.clone(),
            tangent_bytes,
            mappings,
            words,
        ]
        .into_iter();
        let mut resources = Vec::new();
        for resource in R::ALL {
            let extra = if matches!(resource, R::Vertices) {
                vk::BufferUsageFlags::VERTEX_BUFFER | desc.acceleration_usage
            } else {
                vk::BufferUsageFlags::empty()
            };
            match StorageBuffer::with_usage(
                gpu,
                &bytes.next().expect("ten skin resource payloads"),
                extra,
            ) {
                Ok(buffer) => resources.push(buffer),
                Err(e) => {
                    for buffer in resources {
                        buffer.retire(gpu)?;
                    }
                    return Err(e);
                }
            }
        }
        // Nonzero prefix proves the offset convention, leaving room for future
        // record tags without spending more root words.
        let mut indices = vec![0xfeedfaceu32; 4];
        indices.extend(resources.iter().map(|r| r.slot));
        let table = match StorageBuffer::new(gpu, bytemuck::cast_slice(&indices)) {
            Ok(table) => table,
            Err(e) => {
                for r in resources {
                    r.retire(gpu)?;
                }
                return Err(e);
            }
        };
        Ok(Self {
            resources: resources.try_into().expect("ten skin resources"),
            table,
            vertices: count,
            mappings: mapping_count,
            joints: palette.len(),
            tangents: desc.tangents.is_some(),
        })
    }
    /// Declare initial coherent writes before using the newly created packet.
    pub fn import(&self, graph: &mut Graph) {
        for r in self.resources.iter().chain([&self.table]) {
            graph.import(Resource::Buffer(r.buffer), Access::HostWrite);
        }
    }
    /// Current deformed stream, allocated for both storage and ordinary vertex fetch.
    pub fn vertices(&self) -> Buffer {
        self.resources[R::Vertices as usize].buffer
    }
    /// Previous submitted pose; the final padding vec4 preserves Renderlab's stream tag.
    pub fn previous(&self) -> Buffer {
        self.resources[R::Previous as usize].buffer
    }
    /// Deformed tangent frames (one dummy row when no tangents were supplied).
    pub fn tangents(&self) -> Buffer {
        self.resources[R::Tangents as usize].buffer
    }
    /// Source-cluster data with only mapped position triples changed.
    pub fn clusters(&self) -> Buffer {
        self.resources[R::ClusterData as usize].buffer
    }
    /// Queue palette/first-pose flags on the GPU. No host overwrite or GPU wait, even
    /// if an earlier frame still reads the palette. Reset history on cuts explicitly.
    pub fn update_palette(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        palette: &[Mat4],
        reset_history: bool,
    ) -> Result<()> {
        validate_palette(palette, self.joints)?;
        let palette_buffer = self.resources[R::Palette as usize].buffer;
        let params = self.resources[R::Params as usize].buffer;
        graph.node(
            rec,
            &[
                (Resource::Buffer(palette_buffer), Access::CopyDestination),
                (Resource::Buffer(params), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::UpdateBuffer {
            destination: palette_buffer,
            offset: 0,
            bytes: palette_bytes(palette),
        });
        rec.push(Command::UpdateBuffer {
            destination: params,
            offset: 0,
            bytes: bytemuck::cast_slice(&[
                self.vertices,
                self.mappings,
                u32::from(self.tangents),
                u32::from(reset_history),
            ])
            .to_vec(),
        });
        Ok(())
    }
    /// Forget and retire the packet after all its recorded lists have been submitted.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        for r in self.resources.into_iter().chain([self.table]) {
            graph.forget(Resource::Buffer(r.buffer));
            r.retire(gpu)?;
        }
        Ok(())
    }
}
/// The unchanged vertex deformation and cluster-copy pipelines.
pub struct SkinningPasses {
    pipelines: [Pipeline; 2],
    threads: [[u32; 3]; 2],
}
impl SkinningPasses {
    /// Load both entry programs once, preserving the last caller-owned set on errors.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let stages = [Pass::SkinVertices, Pass::SkinClusters];
        let threads = stages
            .map(Pass::threads)
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("two skin entries");
        let first = cache.pipeline(gpu, stages[0], RenderState::default())?;
        let second = match cache.pipeline(gpu, stages[1], RenderState::default()) {
            Ok(p) => p,
            Err(e) => {
                gpu.retire_pipeline(first)?;
                return Err(e);
            }
        };
        Ok(Self {
            pipelines: [first, second],
            threads,
        })
    }
    /// Record one kernel. Cluster copy reads the preceding current-vertex writes;
    /// a later consumer declares Vertex/Geometry or AsInput with the same identity.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        stage: Pass,
        buffers: &SkinningBuffers,
    ) -> Result<()> {
        let (index, count, resources) = match stage {
            Pass::SkinVertices => (
                0,
                buffers.vertices,
                &[
                    R::Source,
                    R::Influences,
                    R::Palette,
                    R::Vertices,
                    R::Previous,
                    R::Params,
                    R::TangentSource,
                    R::Tangents,
                ][..],
            ),
            Pass::SkinClusters => (
                1,
                buffers.mappings,
                &[R::Vertices, R::Mappings, R::Params, R::ClusterData][..],
            ),
            _ => anyhow::bail!("not a skinning pass"),
        };
        if count == 0 {
            return Ok(());
        }
        ensure!(
            self.threads[index][1..] == [1, 1],
            "skinning requires a one-dimensional workgroup"
        );
        let mut uses = vec![(Resource::Buffer(buffers.table.buffer), Access::ShaderRead)];
        for &r in resources {
            let write = if index == 0 {
                matches!(r, R::Vertices | R::Previous | R::Tangents)
            } else {
                matches!(r, R::ClusterData)
            };
            uses.push((
                Resource::Buffer(buffers.resources[r as usize].buffer),
                if write {
                    Access::Storage
                } else {
                    Access::ShaderRead
                },
            ));
        }
        graph.node(rec, &uses)?;
        rec.push(Command::BeginPass(format!("ara::{stage:?}")));
        rec.push(Command::Pipeline(self.pipelines[index]));
        rec.push(Command::Constants(RootConstants {
            words: [buffers.table.slot, 16, 0, 0, 0, 0, 0, 0],
        }));
        rec.push(Command::Dispatch([
            count.div_ceil(self.threads[index][0]),
            1,
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire pipelines after submitting all referencing lists.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for p in self.pipelines {
            gpu.retire_pipeline(p)?;
        }
        Ok(())
    }
}
fn validate_palette(palette: &[Mat4], count: usize) -> Result<()> {
    ensure!(
        palette.len() == count
            && palette
                .iter()
                .all(|m| m.is_finite() && crate::normal::NormalTransform::new(*m).valid()),
        "palette needs finite nonsingular affine transforms and unchanged joint count"
    );
    Ok(())
}
fn palette_bytes(palette: &[Mat4]) -> Vec<u8> {
    palette
        .iter()
        .flat_map(|m| m.to_cols_array())
        .flat_map(f32::to_le_bytes)
        .collect()
}
