//! SK1: owned per-instance deformation. The ordinary mesh is the only geometry
//! consumed by materials, visibility, shadows and ray tracing.
use super::*;
use crate::asset::skin::Influence;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// One independently posed mesh primitive. Reuse its `mesh()` for multiple placements
/// sharing a pose; create another instance for an independent pose.
pub struct SkinnedMeshInstance {
    mesh: Mesh,
}
impl SkinnedMeshInstance {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
    /// Uploads the supplied mesh-local joint matrices on the next admitted render.
    /// AnimationPose::joint_palette supplies these from a cooked glTF clip.
    pub fn set_joint_palette(&mut self, palette: &[Mat4]) -> Result<(), Error> {
        let skin = self.mesh.skin.as_mut().unwrap();
        if palette.len() != skin.palette.len()
            || palette
                .iter()
                .any(|m| !m.is_finite() || !crate::normal::NormalTransform::new(*m).valid())
        {
            return Err(Error::InvalidInput(
                "skin palette must contain finite nonsingular affine joint transforms",
            ));
        }
        if skin.palette == palette {
            return Ok(());
        }
        let positions = deform_positions(&skin.source, &skin.influences, palette);
        let bounds = Aabb::from_positions(positions.iter().copied())
            .ok_or(Error::InvalidInput("skinned bounds are nonfinite"))?;
        if positions.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::InvalidInput("skinned position overflow"));
        }
        skin.positions = positions;
        skin.palette.copy_from_slice(palette);
        skin.revision = skin.revision.wrapping_add(1);
        self.mesh.bounds = bounds;
        Ok(())
    }
}

pub(crate) struct SkinGpu {
    pub previous: Arc<Owned<wgpu::Buffer>>,
    pub revision: u64,
    palette: Vec<Mat4>,
    source: Vec<Vertex>,
    influences: Vec<Influence>,
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    palette_buffer: Owned<wgpu::Buffer>,
    params: Owned<wgpu::Buffer>,
    vertices: Owned<wgpu::ComputePipeline>,
    clusters: Owned<wgpu::ComputePipeline>,
    vertex_bind: Owned<wgpu::BindGroup>,
    cluster_bind: Owned<wgpu::BindGroup>,
    mappings: u32,
    tangents: bool,
    first: AtomicBool,
    software_revision: AtomicU64,
    _inputs: Vec<Owned<wgpu::Buffer>>,
}

#[cfg(test)]
impl SkinGpu {
    /// Eight warmed component samples, including the duplicated cluster-position pass.
    pub(super) fn test_profile(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<f64> {
        let q = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("SK1 component timing"),
            ty: wgpu::QueryType::Timestamp,
            count: 32,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SK1 timing resolve"),
            size: 256,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SK1 timing readback"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut e = device.create_command_encoder(&Default::default());
        for repeat in 0..8u32 {
            for (kernel, bind, groups, offset) in [
                (
                    &self.vertices,
                    &self.vertex_bind,
                    (self.source.len() as u32).div_ceil(64),
                    0,
                ),
                (
                    &self.clusters,
                    &self.cluster_bind,
                    self.mappings.div_ceil(64),
                    2,
                ),
            ] {
                let mut p = e.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("SK1 timed deformation"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: &q,
                        beginning_of_pass_write_index: Some(repeat * 4 + offset),
                        end_of_pass_write_index: Some(repeat * 4 + offset + 1),
                    }),
                });
                p.set_pipeline(kernel);
                p.set_bind_group(0, &**bind, &[]);
                p.dispatch_workgroups(groups, 1, 1);
            }
        }
        e.resolve_query_set(&q, 0..32, &resolve, 0);
        e.copy_buffer_to_buffer(&resolve, 0, &read, 0, 256);
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        e.map_buffer_on_submit(&read, wgpu::MapMode::Read, .., move |r| {
            callback.set(r).unwrap();
        });
        queue.submit([e.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        done.get().unwrap().as_ref().unwrap();
        let view = read.get_mapped_range(..).unwrap();
        let times: Vec<_> = view
            .chunks_exact(8)
            .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        let result = times
            .chunks_exact(4)
            .map(|t| {
                ((t[1] - t[0]) + (t[3] - t[2])) as f64
                    * f64::from(queue.get_timestamp_period())
                    * 1e-6
            })
            .collect();
        drop(view);
        read.unmap();
        result
    }
}

fn deform_positions(
    vertices: &[Vertex],
    influences: &[Influence],
    palette: &[Mat4],
) -> Vec<[f32; 3]> {
    vertices
        .iter()
        .zip(influences)
        .map(|(v, w)| {
            let m = (0..4)
                .map(|k| palette[w.joints[k] as usize] * w.weights[k])
                .reduce(|a, b| a + b)
                .unwrap();
            m.transform_point3(Vec3::from(v.position)).to_array()
        })
        .collect()
}

impl Renderer {
    /// Skins are enabled without a renderer toggle on both hardware and non-RT
    /// devices. Full-detail source clusters are used; attaching cluster LOD is rejected.
    pub fn try_upload_skinned_mesh(
        &mut self,
        primitive: &asset::Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<Option<SkinnedMeshInstance>, Error> {
        if influences.len() != primitive.vertices.len()
            || joint_count == 0
            || joint_count > 65536
            || influences.iter().any(|v| {
                v.joints.iter().any(|&j| u32::from(j) >= joint_count)
                    || v.weights.iter().any(|w| !w.is_finite() || *w < 0.)
                    || (v.weights.iter().sum::<f32>() - 1.).abs() > 1e-4
            })
        {
            return Err(Error::InvalidInput(
                "invalid skin vertex influences or joint count",
            ));
        }
        let limits = self.device.limits();
        let data = MeshData {
            vertices: &primitive.vertices,
            indices: &primitive.indices,
        };
        let sizes = data.validate(self.limits)?;
        if sizes.0 + 16 > limits.max_storage_buffer_binding_size
            || u64::from(joint_count) * 64 > limits.max_storage_buffer_binding_size
            || limits.max_storage_buffers_per_shader_stage < 7
            || primitive.vertices.len().div_ceil(64)
                > limits.max_compute_workgroups_per_dimension as usize
        {
            return Err(Error::InvalidInput(
                "skinning buffers or dispatch exceed device limits",
            ));
        }
        let source_clusters = visibility_buffer::source_clusters(&data);
        let mapping_bytes = source_clusters.source_map.len() as u64 * 8;
        let cluster_bytes = source_clusters.words.len() as u64 * 4;
        let additional = sizes.0 * 2
            + influences.len() as u64 * 32
            + u64::from(joint_count) * 64
            + mapping_bytes
            + cluster_bytes
            + 48
            + primitive
                .tangents
                .as_ref()
                .map_or(0, |t| t.values.len() as u64 * 16);
        if additional
            .checked_add(sizes.0 + sizes.1)
            .is_none_or(|n| n > self.limits.max_mesh_bytes)
            || mapping_bytes > limits.max_storage_buffer_binding_size
            || cluster_bytes > limits.max_storage_buffer_binding_size
            || source_clusters.source_map.len().div_ceil(64)
                > limits.max_compute_workgroups_per_dimension as usize
        {
            return Err(Error::InvalidInput(
                "skinning exceeds mesh or storage limits",
            ));
        }
        let Some(mut mesh) =
            self.try_upload_mesh_prepared(&data, primitive.tangents.as_ref(), None, true)?
        else {
            return Ok(None);
        };
        let scopes = OperationScopes::new(&self.device);
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
        let storage = wgpu::BufferUsages::STORAGE;
        let source = buffer("SK1 bind-pose vertices", sizes.0, storage);
        let previous = Arc::new(buffer(
            "SK1 previous submitted positions and attributes",
            // One padding vec4 distinguishes a previous-pose stream from a static
            // alias by its runtime length, without changing placement/material flags.
            sizes.0 + 16,
            storage | wgpu::BufferUsages::COPY_SRC,
        ));
        let weights = buffer(
            "SK1 vertex influences",
            influences.len() as u64 * 32,
            storage,
        );
        let palette_buffer = buffer("SK1 joint palette", u64::from(joint_count) * 64, storage);
        let params = buffer("SK1 dispatch parameters", 16, wgpu::BufferUsages::UNIFORM);
        let tangent_source = buffer(
            "SK1 bind-pose tangents",
            primitive
                .tangents
                .as_ref()
                .map_or(16, |t| t.values.len() as u64 * 16),
            storage,
        );
        let tangent_absent = buffer("SK1 absent tangent output", 16, storage);
        let mappings = buffer(
            "SK1 cluster position mapping",
            mapping_bytes.max(8),
            storage,
        );
        if mesh.clusters.is_none() {
            mesh.clusters = Some(self.upload_clusters(
                source_clusters.clusters,
                &source_clusters.words,
                vec![[0., 0., 0., 0., f32::MAX]],
            ));
        }
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("SK1 skinning"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/skinning.wgsl").into()),
            });
        let kernel = |entry| {
            self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                ResourceKind::ComputePipeline,
            )
        };
        let vertices = kernel("skin_vertices");
        let clusters = kernel("skin_clusters");
        let bind = |pipeline: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]| {
            self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("SK1 deformation inputs and outputs"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries
                        .iter()
                        .map(|(binding, b)| wgpu::BindGroupEntry {
                            binding: *binding,
                            resource: b.as_entire_binding(),
                        })
                        .collect::<Vec<_>>(),
                }),
                ResourceKind::BindGroup,
            )
        };
        let vertex_bind = bind(
            &vertices,
            &[
                (0, &source),
                (1, &weights),
                (2, &palette_buffer),
                (3, &mesh.vertex),
                (4, &previous),
                (5, &params),
                (6, &tangent_source),
                (7, mesh.tangent.as_deref().unwrap_or(&tangent_absent)),
            ],
        );
        let cluster_bind = bind(
            &clusters,
            &[
                (3, &mesh.vertex),
                (5, &params),
                (8, &mappings),
                (9, &mesh.clusters.as_ref().unwrap().data),
            ],
        );
        if let Some(e) = scopes.finish() {
            return Err(self.fail(e.to_string()));
        }
        let source_bytes: Vec<_> = primitive
            .vertices
            .iter()
            .flat_map(|v| {
                v.position
                    .into_iter()
                    .chain(v.normal)
                    .chain(v.uv)
                    .chain(v.color)
            })
            .flat_map(f32::to_ne_bytes)
            .collect();
        self.queue.write_buffer(&source, 0, &source_bytes);
        self.queue.write_buffer(&previous, 0, &source_bytes);
        let influence_bytes: Vec<_> = influences
            .iter()
            .flat_map(|v| {
                v.joints
                    .map(u32::from)
                    .map(u32::to_ne_bytes)
                    .into_iter()
                    .chain(v.weights.map(f32::to_ne_bytes))
            })
            .flatten()
            .collect();
        self.queue.write_buffer(&weights, 0, &influence_bytes);
        if let Some(t) = &primitive.tangents {
            let bytes: Vec<_> = t
                .values
                .iter()
                .flatten()
                .copied()
                .flat_map(f32::to_ne_bytes)
                .collect();
            self.queue.write_buffer(&tangent_source, 0, &bytes);
        }
        let bytes: Vec<_> = source_clusters
            .source_map
            .iter()
            .flatten()
            .copied()
            .flat_map(u32::to_ne_bytes)
            .collect();
        if !bytes.is_empty() {
            self.queue.write_buffer(&mappings, 0, &bytes);
        }
        mesh.skin = Some(SkinGpu {
            previous,
            revision: 1,
            palette: vec![Mat4::IDENTITY; joint_count as usize],
            positions: primitive.vertices.iter().map(|v| v.position).collect(),
            source: primitive.vertices.clone(),
            influences: influences.to_vec(),
            indices: primitive.indices.clone(),
            palette_buffer,
            params,
            vertices,
            clusters,
            vertex_bind,
            cluster_bind,
            mappings: source_clusters.source_map.len() as u32,
            tangents: primitive.tangents.is_some(),
            first: AtomicBool::new(true),
            software_revision: AtomicU64::new(0),
            _inputs: vec![source, weights, tangent_source, tangent_absent, mappings],
        });
        self.check_fault()?;
        Ok(Some(SkinnedMeshInstance { mesh }))
    }
    pub(super) fn prepare_skinning(&mut self, draws: &[Draw<'_>]) -> Result<UploadBytes, Error> {
        let mut uploads = UploadBytes::default();
        let mut seen = std::collections::HashSet::new();
        for d in draws {
            let Some(s) = &d.mesh.skin else {
                continue;
            };
            if !seen.insert(d.mesh.vertex.as_ref() as *const _) {
                continue;
            }
            let bytes: Vec<_> = s.palette.iter().flat_map(|m| matrix_bytes(*m)).collect();
            self.queue.write_buffer(&s.palette_buffer, 0, &bytes);
            let params = [
                s.source.len() as u32,
                s.mappings,
                u32::from(s.tangents),
                u32::from(s.first.load(Ordering::Relaxed)),
            ];
            self.queue.write_buffer(
                &s.params,
                0,
                &params
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
                    .collect::<Vec<_>>(),
            );
            uploads.skinning += bytes.len() as u64 + 16;
            uploads.write_calls += 2;
            if s.software_revision.load(Ordering::Relaxed) != s.revision
                && self.update_skinned_indirect(d.mesh, &s.positions, &s.indices)?
            {
                s.software_revision.store(s.revision, Ordering::Relaxed);
            }
        }
        Ok(uploads)
    }
    pub(super) fn encode_skinning(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[PreparedDraw<'_>],
        build_acceleration: bool,
    ) {
        let mut seen = std::collections::HashSet::new();
        for d in draws {
            let Some(s) = &d.mesh.skin else {
                continue;
            };
            if !seen.insert(d.mesh.vertex.as_ref() as *const _) {
                continue;
            }
            {
                let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("SK1 positions normals and history"),
                    timestamp_writes: None,
                });
                p.set_pipeline(&s.vertices);
                p.set_bind_group(0, &*s.vertex_bind, &[]);
                p.dispatch_workgroups((s.source.len() as u32).div_ceil(64), 1, 1);
            }
            {
                let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("SK1 full-detail cluster positions"),
                    timestamp_writes: None,
                });
                p.set_pipeline(&s.clusters);
                p.set_bind_group(0, &*s.cluster_bind, &[]);
                p.dispatch_workgroups(s.mappings.div_ceil(64), 1, 1);
            }
            s.first.store(false, Ordering::Relaxed);
            if build_acceleration && let Some(acceleration) = &d.mesh.acceleration {
                acceleration.encode(encoder);
            }
        }
    }
}

/// One placement's pose: its skinned mesh (by vertex buffer) and that mesh's pose revision.
/// None for a rigid placement.
pub(crate) type PlacementPose = Option<(usize, u64)>;

/// Per placement, in draw order then instance slot: its `PlacementPose`.
pub(crate) fn placement_poses(draws: &[PreparedDraw<'_>]) -> Vec<PlacementPose> {
    draws
        .iter()
        .flat_map(|d| {
            let pose = d
                .mesh
                .skin
                .as_ref()
                .map(|s| (Arc::as_ptr(&d.mesh.vertex) as usize, s.revision));
            std::iter::repeat_n(pose, d.instances.len())
        })
        .collect()
}

/// P-SHOW: whether placement `i` deformed since the previous frame's `placement_poses`. A
/// skinned placement deforms when its mesh or pose revision differs from the previous
/// frame's at the same placement, or when the placement list changed length. An unchanged
/// pose leaves the mesh's positions unchanged (`set_joint_palette` bumps the revision only
/// on a new palette), so caches keyed on it stay valid.
pub(crate) fn deformed(previous: &[PlacementPose], current: &[PlacementPose], i: usize) -> bool {
    let now = current.get(i).copied().flatten();
    now.is_some() && (previous.len() != current.len() || previous.get(i).copied().flatten() != now)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skinning_shader_validates_and_blended_positions_preserve_translation() {
        let source = include_str!("../shaders/skinning.wgsl");
        let m = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
        let v = Vertex {
            position: [1., 2., 3.],
            normal: [0., 1., 0.],
            uv: [0.; 2],
            color: [1.; 4],
        };
        let p = deform_positions(
            &[v],
            &[Influence {
                joints: [0, 1, 0, 0],
                weights: [0.25, 0.75, 0., 0.],
            }],
            &[Mat4::IDENTITY, Mat4::from_translation(Vec3::X * 4.)],
        );
        assert_eq!(p, [[4., 2., 3.]]);
    }

    #[test]
    fn placements_deform_only_on_a_new_pose() {
        let rigid = None;
        let posed = Some((7usize, 3u64));
        let previous = [rigid, posed];
        // Same mesh and revision: no deformation; rigid placements never deform here.
        assert!(!deformed(&previous, &[rigid, posed], 0));
        assert!(!deformed(&previous, &[rigid, posed], 1));
        // A new revision, or another mesh at the placement, deforms.
        assert!(deformed(&previous, &[rigid, Some((7, 4))], 1));
        assert!(deformed(&previous, &[rigid, Some((8, 3))], 1));
        // A changed placement list counts every skinned placement as deforming.
        assert!(deformed(&previous, &[rigid, posed, rigid], 1));
        assert!(deformed(&[], &[posed], 0));
    }
}
