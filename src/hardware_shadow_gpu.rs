use super::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RENDERER_LABEL: AtomicU64 = AtomicU64::new(1);

/// Source buffers are retained with the BLAS through builds and every TLAS generation.
/// SK1 deforms them in queue order; ordinary meshes remain immutable. Safe wgpu owns
/// submitted backend lifetimes.
pub(crate) struct MeshAcceleration {
    // SK1: dynamic structures are updated in-place in queue order, never compacted.
    dynamic: bool,
    blas: Mutex<Arc<BlasAllocation>>,
    label: String,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    vertex: Arc<Owned<wgpu::Buffer>>,
    index: Arc<Owned<wgpu::Buffer>>,
    _receiver_adjacency: Option<Allocation>,
    /// The same triangles with non-opaque geometry, made the first time the mesh is drawn
    /// see-through: a ray that culls with SEE_THROUGH_MASK gets its triangles as candidates to
    /// pass by their transmission, while every opaque BLAS still commits in hardware.
    clear: Mutex<Option<Arc<BlasAllocation>>>,
    /// `size` with non-opaque geometry, for building `clear`.
    size_clear: wgpu::BlasTriangleGeometrySizeDescriptor,
}
/// A TLAS retains the exact BLAS generation it was built against. Compaction may
/// publish a replacement while earlier submissions still reference the original.
struct BlasAllocation {
    blas: Owned<wgpu::Blas>,
    _estimated: Allocation,
    _observed: Option<Allocation>,
}
impl MeshAcceleration {
    pub(super) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        Self::encode_all(encoder, &[self]);
    }
    /// Builds every mesh's BLAS from its current vertices in one call (one barrier batch).
    pub(super) fn encode_all(encoder: &mut wgpu::CommandEncoder, meshes: &[&Self]) {
        let guards: Vec<_> = meshes.iter().map(|m| m.blas.lock().unwrap()).collect();
        let entries: Vec<_> = meshes
            .iter()
            .zip(&guards)
            .map(|(mesh, blas)| wgpu::BlasBuildEntry {
                blas: &blas.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size: &mesh.size,
                        vertex_buffer: &mesh.vertex,
                        first_vertex: 0,
                        vertex_stride: 48,
                        index_buffer: Some(&mesh.index),
                        first_index: Some(0),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();
        encoder.build_acceleration_structures(&entries, std::iter::empty());
    }
}

/// TLAS instance masks. Every placement carries `VISIBLE_MASK`, which camera, reflection and
/// bounce rays cull with, and those that block light (`Instances::set_casts_shadows`) also
/// `SHADOW_CASTER_MASK`, which every shadow ray culls with.
const MASK_SHADOW: u8 = crate::shadows::SHADOW_CASTER_MASK as u8;
const MASK_VISIBLE: u8 = crate::shadows::VISIBLE_MASK as u8;
const MASK_SEE_THROUGH: u8 = crate::shadows::SEE_THROUGH_MASK as u8;
/// The see-through table (`HardwareState::see_through_table`): four header words (the first
/// see-through placement's index, their count, two spare), then per see-through placement four
/// words: its base colour's red and green, then blue and coverage (each `pack2x16float`), its
/// index of refraction (f32 bits) and a spare word.
pub(crate) const SEE_THROUGH_HEADER_WORDS: usize = 4;
pub(crate) const SEE_THROUGH_WORDS: usize = 4;

struct Placement {
    mesh: Arc<MeshAcceleration>,
    blas: Arc<BlasAllocation>,
    model: [u32; 16],
    mask: u8,
}
struct SceneAcceleration {
    tlas: Owned<wgpu::Tlas>,
    /// Built with ALLOW_UPDATE for skinned placements.
    dynamic: bool,
    bind: Owned<wgpu::BindGroup>,
    placements: Vec<Placement>,
    _estimated: Allocation,
    _observed: Option<Allocation>,
}
struct PendingBuild {
    _mesh: Option<Arc<MeshAcceleration>>,
    _scratch: Allocation,
}

pub(super) struct HardwareState {
    pub options: HardwareShadowOptions,
    /// RT1d's opt-in current-frame visibility gather; read through the frame ABI.
    pub visibility_filter: bool,
    /// Every AS this renderer builds allows hit-vertex return (traced visibility).
    flags: wgpu::AccelerationStructureFlags,
    bindings: Owned<wgpu::BindGroupLayout>,
    pub layout: Owned<wgpu::PipelineLayout>,
    active: Option<SceneAcceleration>,
    retired: Vec<(u64, SceneAcceleration)>,
    pending: Vec<(u64, PendingBuild)>,
    renderer_label: u64,
    next_label: u64,
    /// Counts TLAS objects (not builds): bind groups over the scene key on it.
    generation: u64,
    accounting_available: bool,
    pub stats: HardwareShadowStats,
    /// frame-cpu: the draw list the active TLAS was made from (`draw_keys`); an equal list
    /// skips rebuilding and comparing every placement.
    last_keys: Option<(Vec<DrawKey>, bool)>,
    /// The see-through table (SEE_THROUGH_HEADER_WORDS), written when its words change.
    see_through: Option<(Owned<wgpu::Buffer>, Vec<u32>)>,
    /// Non-opaque BLASes made this frame, built with the TLAS (`encode_scene`).
    pending_clear: Mutex<Vec<(Arc<MeshAcceleration>, Arc<BlasAllocation>)>>,
}

/// A draw's identity for the TLAS: its mesh, that mesh's current BLAS generation, the
/// placements' collection and edit count (`InstanceState::revision`, which also counts
/// shadow-casting changes), the placement count and whether its material is double-sided
/// (0 single, 1 double, 2 none: `placements` rejects single-sided casters).
type DrawKey = (usize, usize, (u64, u64), usize, u8);
/// frame-cpu: `RENDERLAB_TLAS_KEY=0` rebuilds and compares every placement each frame.
fn tlas_keys() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_TLAS_KEY").map_or(true, |v| v != "0"))
}

/// WGSL's `pack2x16float` for values in [0, 3] (`transparency::f16_bits`): `a` low, `b` high.
fn pack2x16float(a: f32, b: f32) -> u32 {
    u32::from(crate::transparency::f16_bits(a)) | (u32::from(crate::transparency::f16_bits(b)) << 16)
}
fn row_transform(model: Mat4) -> [f32; 12] {
    let m = model.to_cols_array_2d();
    std::array::from_fn(|i| m[i % 4][i / 4])
}
pub(super) fn mesh_estimate(triangles: u64) -> Result<(u64, u64), Error> {
    Ok((
        triangles
            .checked_mul(256)
            .ok_or(Error::InvalidInput("BLAS estimate overflow"))?,
        triangles
            .checked_mul(128)
            .ok_or(Error::InvalidInput("BLAS scratch estimate overflow"))?,
    ))
}
fn scene_estimate(instances: usize) -> Result<(u64, u64), Error> {
    let n = u64::try_from(instances).map_err(|_| Error::InvalidInput("TLAS count overflow"))?;
    let bytes = n
        .checked_mul(256)
        .and_then(|n| n.checked_add(4096))
        .ok_or(Error::InvalidInput("TLAS estimate overflow"))?;
    Ok((bytes, bytes))
}

impl HardwareState {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        groups: [&wgpu::BindGroupLayout; 3],
        lighting: &wgpu::BindGroupLayout,
        options: HardwareShadowOptions,
        vertex_return: bool,
    ) -> Self {
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("optional hardware shadow camera ABI"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(80),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::AccelerationStructure {
                            vertex_return: false,
                        },
                        count: None,
                    },
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("optional hardware shadow mesh ABI"),
                bind_group_layouts: &[
                    Some(&bindings),
                    Some(groups[1]),
                    Some(groups[2]),
                    Some(lighting),
                ],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        Self {
            options,
            visibility_filter: false,
            flags: if vertex_return {
                wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
                    | wgpu::AccelerationStructureFlags::ALLOW_RAY_HIT_VERTEX_RETURN
            } else {
                wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
            },
            bindings,
            layout,
            active: None,
            retired: vec![],
            pending: vec![],
            renderer_label: NEXT_RENDERER_LABEL.fetch_add(1, Ordering::Relaxed),
            next_label: 0,
            generation: 0,
            accounting_available: device.generate_allocator_report().is_some(),
            stats: HardwareShadowStats::default(),
            last_keys: None,
            see_through: None,
            pending_clear: Mutex::new(Vec::new()),
        }
    }
    fn label(&mut self, kind: &str) -> String {
        self.next_label += 1;
        format!(
            "renderlab hardware {} {kind} {}",
            self.renderer_label, self.next_label
        )
    }
    fn observed(
        &mut self,
        device: &wgpu::Device,
        ledger: &Ledger,
        label: &str,
    ) -> Option<Allocation> {
        let bytes = device.generate_allocator_report().and_then(|report| {
            let mut sizes = report
                .allocations
                .iter()
                .filter(|a| a.name == label)
                .map(|a| a.size)
                .peekable();
            sizes.peek()?;
            sizes.try_fold(0u64, u64::checked_add).filter(|n| *n > 0)
        });
        if bytes.is_none() {
            self.accounting_available = false;
        }
        bytes.map(|bytes| ledger.allocate(AllocationKind::HardwareAsObserved, bytes))
    }
    fn admit(&self, ledger: &Ledger, additional: u64) -> Result<(), Error> {
        let owned = ledger.snapshot();
        let owned = owned
            .hardware_as_estimated_bytes
            .live
            .checked_add(owned.hardware_scratch_estimated_bytes.live)
            .ok_or(Error::InvalidInput("hardware memory estimate overflow"))?;
        self.options.admit(owned, additional)
    }
    pub fn validate_mesh(
        &self,
        ledger: &Ledger,
        limits: &wgpu::Limits,
        vertices: u64,
        indices: u64,
    ) -> Result<(), Error> {
        if vertices / 48 > u64::from(u32::MAX)
            || indices / 12 > u64::from(limits.max_blas_primitive_count)
        {
            return Err(Error::InvalidInput(
                "mesh exceeds negotiated BLAS device limits",
            ));
        }
        let (as_bytes, scratch) = mesh_estimate(indices / 12)?;
        self.admit(
            ledger,
            as_bytes
                .checked_add(scratch)
                .ok_or(Error::InvalidInput("BLAS estimate overflow"))?,
        )
    }
    pub fn create_mesh(
        &mut self,
        device: &wgpu::Device,
        ledger: &Ledger,
        vertex: &Arc<Owned<wgpu::Buffer>>,
        index: &Arc<Owned<wgpu::Buffer>>,
        index_count: u32,
        adjacency_bytes: u64,
        dynamic: bool,
    ) -> Arc<MeshAcceleration> {
        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: (vertex.size() / 48) as u32,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(index_count),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };
        let size_for_clear = size.clone();
        let label = self.label("BLAS");
        let blas = ledger.own(
            device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some(&label),
                    flags: if dynamic {
                        (self.flags - wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE)
                            | wgpu::AccelerationStructureFlags::ALLOW_UPDATE
                            | wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD
                    } else {
                        self.flags | wgpu::AccelerationStructureFlags::ALLOW_COMPACTION
                    },
                    update_mode: if dynamic {
                        wgpu::AccelerationStructureUpdateMode::PreferUpdate
                    } else {
                        wgpu::AccelerationStructureUpdateMode::Build
                    },
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            ),
            ResourceKind::Blas,
        );
        let observed = self.observed(device, ledger, &label);
        Arc::new(MeshAcceleration {
            dynamic,
            blas: Mutex::new(Arc::new(BlasAllocation {
                blas,
                _estimated: ledger.allocate(
                    AllocationKind::HardwareAsEstimate,
                    mesh_estimate(u64::from(index_count) / 3).unwrap().0,
                ),
                _observed: observed,
            })),
            label,
            size,
            vertex: vertex.clone(),
            index: index.clone(),
            _receiver_adjacency: (adjacency_bytes != 0)
                .then(|| ledger.allocate(AllocationKind::Shadow, adjacency_bytes)),
            clear: Mutex::new(None),
            size_clear: wgpu::BlasTriangleGeometrySizeDescriptor {
                flags: wgpu::AccelerationStructureGeometryFlags::empty(),
                ..size_for_clear
            },
        })
    }
    /// `mesh`'s non-opaque BLAS (`MeshAcceleration::clear`), made on first use and queued for
    /// building with the next TLAS (`encode_scene`). Skinned meshes have none (None).
    fn clear_blas(&mut self, device: &wgpu::Device, ledger: &Ledger, mesh: &Arc<MeshAcceleration>) -> Option<Arc<BlasAllocation>> {
        if mesh.dynamic {
            return None;
        }
        let mut slot = mesh.clear.lock().unwrap();
        if let Some(blas) = &*slot {
            return Some(blas.clone());
        }
        let size = mesh.size_clear.clone();
        let label = self.label("see-through BLAS");
        let blas = ledger.own(
            device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some(&label),
                    flags: self.flags,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size] },
            ),
            ResourceKind::Blas,
        );
        let estimate = mesh_estimate(u64::from(mesh.size.index_count.unwrap()) / 3).unwrap().0;
        let blas = Arc::new(BlasAllocation {
            blas,
            _estimated: ledger.allocate(AllocationKind::HardwareAsEstimate, estimate),
            _observed: self.observed(device, ledger, &label),
        });
        *slot = Some(blas.clone());
        self.pending_clear.lock().unwrap().push((mesh.clone(), blas.clone()));
        Some(blas)
    }
    pub fn mesh_submitted(
        &mut self,
        ledger: &Ledger,
        mesh: Arc<MeshAcceleration>,
        submission: u64,
    ) {
        if !mesh.dynamic {
            mesh.blas
                .lock()
                .unwrap()
                .blas
                .prepare_compaction_async(|_| {});
        }
        let scratch = mesh_estimate(u64::from(mesh.size.index_count.unwrap()) / 3)
            .unwrap()
            .1;
        self.pending.push((
            submission,
            PendingBuild {
                _mesh: Some(mesh),
                _scratch: ledger.allocate(AllocationKind::HardwareScratchEstimate, scratch),
            },
        ));
        self.stats.blas_builds += 1;
    }
    // SK1: retain and account each pose build's scratch through its submission.
    pub fn dynamic_submitted(
        &mut self,
        ledger: &Ledger,
        mesh: Arc<MeshAcceleration>,
        submission: u64,
    ) {
        self.mesh_submitted(ledger, mesh, submission);
        self.stats.blas_builds -= 1;
        self.stats.blas_updates += 1;
    }
    /// `casters`: the placements also cast hardware shadows, which need double-sided
    /// opaque materials. Traced primary visibility alone culls single-sided back faces.
    /// The see-through draws' placements follow the opaque ones, under SEE_THROUGH_MASK alone.
    fn placements(
        draws: &[Draw<'_>],
        see_through: &[Draw<'_>],
        max_instances: u32,
        casters: bool,
    ) -> Result<Vec<Placement>, Error> {
        let count = draws
            .iter()
            .chain(see_through)
            .try_fold(0usize, |n, d| n.checked_add(d.instances.len()))
            .ok_or(Error::InvalidInput("hardware instance count overflow"))?;
        if count > max_instances as usize || count > 0x1000000 {
            return Err(Error::InvalidInput(
                "hardware instance count exceeds negotiated TLAS limit",
            ));
        }
        let mut placements = Vec::with_capacity(count);
        for (i, draw) in draws.iter().chain(see_through).enumerate() {
            let clear = i >= draws.len();
            if casters && !clear && draw.material.is_some_and(|m| !m.double_sided) {
                return Err(Error::InvalidInput(
                    "hardware shadows currently require opaque double-sided materials",
                ));
            }
            // A see-through draw without an acceleration structure is left out (and out of the
            // see-through table, `write_see_through`): it then passes light as before.
            if clear && draw.mesh.acceleration.as_ref().is_none_or(|m| m.dynamic) {
                continue;
            }
            let mesh = draw.mesh.acceleration.as_ref().ok_or(Error::InvalidInput(
                "hardware shadow draw has no prepared mesh acceleration",
            ))?;
            if draw.instances.state.invalid_normals > 0 {
                return Err(Error::InvalidInput(
                    "hardware shadows require nonsingular affine instance transforms",
                ));
            }
            let state = &draw.instances.state;
            for (slot, model) in state.models.iter().enumerate() {
                let shadow = if state.casts_shadows(slot) {
                    MASK_SHADOW
                } else {
                    0
                };
                placements.push(Placement {
                    blas: if clear {
                        mesh.clear.lock().unwrap().clone().expect("see-through BLAS made before placements")
                    } else {
                        mesh.blas.lock().unwrap().clone()
                    },
                    mesh: mesh.clone(),
                    model: model.to_cols_array().map(f32::to_bits),
                    mask: if clear { MASK_SEE_THROUGH } else { MASK_VISIBLE | shadow },
                });
            }
        }
        Ok(placements)
    }
    /// Compaction queues a copy, flushed by the next ordinary submission. Existing
    /// TLAS generations retain the original allocation until their flights retire.
    fn compact_mesh(
        &mut self,
        mesh: &MeshAcceleration,
        gpu: (&wgpu::Device, &wgpu::Queue),
        ledger: &Ledger,
        reserve: u64,
    ) {
        if mesh.dynamic {
            return;
        }
        let mut current = mesh.blas.lock().unwrap();
        if !current.blas.ready_for_compaction() {
            return;
        }
        let estimate = mesh_estimate(u64::from(mesh.size.index_count.unwrap()) / 3)
            .unwrap()
            .0;
        // Optional under a caller's peak-memory policy: skip instead of stalling
        // or rejecting an otherwise valid upload/frame.
        if self.admit(ledger, estimate + reserve).is_err() {
            return;
        }
        let (device, queue) = gpu;
        let blas = ledger.own(queue.compact_blas(&current.blas), ResourceKind::Blas);
        let observed = self.observed(device, ledger, &format!("{} (compacted)", mesh.label));
        *current = Arc::new(BlasAllocation {
            blas,
            _estimated: ledger.allocate(AllocationKind::HardwareAsEstimate, estimate),
            _observed: observed,
        });
        self.stats.blas_compactions += 1;
    }
    /// Runs after flight admission, before frame writes. Completed BLAS builds compact
    /// without waiting; a changed BLAS generation or placement replaces the TLAS.
    pub fn prepare_scene(
        &mut self,
        gpu: (&wgpu::Device, &wgpu::Queue),
        ledger: &Ledger,
        camera: &wgpu::Buffer,
        draws: &[Draw<'_>],
        see_through: &[Draw<'_>],
        submission: u64,
        casters: bool,
    ) -> Result<bool, Error> {
        let (device, _) = gpu;
        self.write_see_through(gpu, ledger, draws, see_through);
        for draw in see_through {
            if let Some(mesh) = &draw.mesh.acceleration {
                self.clear_blas(device, ledger, mesh);
            }
        }
        // frame-cpu: an unchanged draw list keeps the active TLAS's placements. Compaction
        // still advances per mesh first, so a compacted BLAS changes the key.
        let keys = if tlas_keys() && self.active.is_some() {
            let count: usize = draws.iter().chain(see_through).map(|d| d.instances.len()).sum();
            let (scene, scratch) = scene_estimate(count)?;
            let mut meshes = std::collections::HashSet::new();
            for draw in draws.iter().chain(see_through) {
                if let Some(mesh) = &draw.mesh.acceleration
                    && meshes.insert(Arc::as_ptr(mesh))
                {
                    self.compact_mesh(mesh, gpu, ledger, scene + scratch);
                }
            }
            let keys = Self::draw_keys(draws, see_through);
            if self.last_keys.as_ref() == Some(&(keys.clone(), casters)) {
                let active = self.active.as_ref().unwrap();
                if !active.dynamic {
                    return Ok(false);
                }
                let dynamic_scratch = Self::dynamic_scratch(&active.placements)?;
                self.admit(
                    ledger,
                    scratch
                        .checked_add(dynamic_scratch)
                        .ok_or(Error::InvalidInput("TLAS estimate overflow"))?,
                )?;
                return Ok(true);
            }
            Some(keys)
        } else {
            None
        };
        let mut placements =
            Self::placements(draws, see_through, device.limits().max_tlas_instance_count, casters)?;
        let (scene, scratch) = scene_estimate(placements.len())?;
        // Each mesh once; placements take a compacted BLAS only when one was published.
        let compactions = self.stats.blas_compactions;
        let mut meshes = std::collections::HashSet::new();
        for placement in &placements {
            if meshes.insert(Arc::as_ptr(&placement.mesh)) {
                self.compact_mesh(&placement.mesh, gpu, ledger, scene + scratch);
            }
        }
        if self.stats.blas_compactions != compactions {
            for placement in &mut placements {
                placement.blas = placement.mesh.blas.lock().unwrap().clone();
            }
        }
        let dynamic_scene = placements.iter().any(|p| p.mesh.dynamic);
        // U-FIXED (audit 3.11): the same meshes and BLAS generations rebuild into the
        // active TLAS object; only moved placements rewrite their instance. Queue order
        // keeps earlier flights' reads before this build, as for the skinned BLASes.
        let reusable = self.active.as_ref().is_some_and(|active| {
            active.dynamic == dynamic_scene
                && active.placements.len() == placements.len()
                && active.placements.iter().zip(&placements).all(|(a, b)| {
                    Arc::ptr_eq(&a.mesh, &b.mesh) && Arc::ptr_eq(&a.blas, &b.blas)
                })
        });
        if reusable {
            let active = self.active.as_ref().unwrap();
            let moved: Vec<usize> = (0..placements.len())
                .filter(|&i| {
                    active.placements[i].model != placements[i].model
                        || active.placements[i].mask != placements[i].mask
                })
                .collect();
            if moved.is_empty() && !dynamic_scene {
                self.last_keys = Some((keys.unwrap_or_else(|| Self::draw_keys(draws, see_through)), casters));
                return Ok(false);
            }
            let dynamic_scratch = Self::dynamic_scratch(&placements)?;
            self.admit(
                ledger,
                scratch
                    .checked_add(dynamic_scratch)
                    .ok_or(Error::InvalidInput("TLAS estimate overflow"))?,
            )?;
            let active = self.active.as_mut().unwrap();
            for &i in &moved {
                let p = &placements[i];
                let model = Mat4::from_cols_array(&p.model.map(f32::from_bits));
                active.tlas[i] = Some(wgpu::TlasInstance::new(
                    &p.blas.blas,
                    row_transform(model),
                    i as u32,
                    p.mask,
                ));
            }
            active.placements = placements;
            self.last_keys = Some((keys.unwrap_or_else(|| Self::draw_keys(draws, see_through)), casters));
            return Ok(true);
        }
        let (estimate, scratch) = scene_estimate(placements.len())?;
        let dynamic_scratch = Self::dynamic_scratch(&placements)?;
        self.admit(
            ledger,
            estimate
                .checked_add(scratch)
                .and_then(|n| n.checked_add(dynamic_scratch))
                .ok_or(Error::InvalidInput("TLAS estimate overflow"))?,
        )?;
        let label = self.label("TLAS");
        // wgpu 30 implements TLAS rebuild, not refit. A changed placement count, mesh or
        // BLAS generation needs a new object; immutable BLAS geometry is always reused.
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some(&label),
            max_instances: placements.len().max(1) as u32,
            flags: if placements.iter().any(|p| p.mesh.dynamic) {
                self.flags | wgpu::AccelerationStructureFlags::ALLOW_UPDATE
            } else {
                self.flags
            },
            update_mode: if placements.iter().any(|p| p.mesh.dynamic) {
                wgpu::AccelerationStructureUpdateMode::PreferUpdate
            } else {
                wgpu::AccelerationStructureUpdateMode::Build
            },
        });
        for (i, p) in placements.iter().enumerate() {
            let model = Mat4::from_cols_array(&p.model.map(f32::from_bits));
            tlas[i] = Some(wgpu::TlasInstance::new(
                &p.blas.blas,
                row_transform(model),
                i as u32,
                p.mask,
            ));
        }
        let bind = ledger.own(
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera and current hardware shadow scene"),
                layout: &self.bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: tlas.as_binding(),
                    },
                ],
            }),
            ResourceKind::BindGroup,
        );
        let observed = self.observed(device, ledger, &label);
        self.generation += 1;
        let next = SceneAcceleration {
            tlas: ledger.own(tlas, ResourceKind::Tlas),
            dynamic: dynamic_scene,
            bind,
            placements,
            _estimated: ledger.allocate(AllocationKind::HardwareAsEstimate, estimate),
            _observed: observed,
        };
        if let Some(old) = self.active.replace(next) {
            self.retired.push((submission, old));
        }
        self.last_keys = Some((keys.unwrap_or_else(|| Self::draw_keys(draws, see_through)), casters));
        Ok(true)
    }
    /// One key per draw (`DrawKey`), read after this frame's compaction; the see-through draws'
    /// follow the opaque ones', marked by a material word of 3.
    fn draw_keys(draws: &[Draw<'_>], see_through: &[Draw<'_>]) -> Vec<DrawKey> {
        let opaque = draws.len();
        draws
            .iter()
            .chain(see_through)
            .enumerate()
            .map(|(i, d)| (i >= opaque, d))
            .map(|(clear, d)| {
                let mesh = d.mesh.acceleration.as_ref();
                (
                    mesh.map_or(0, |m| Arc::as_ptr(m) as usize),
                    mesh.map_or(0, |m| Arc::as_ptr(&*m.blas.lock().unwrap()) as usize),
                    d.instances.state.revision(),
                    d.instances.len(),
                    if clear { 3 } else { d.material.map_or(2, |m| u8::from(m.double_sided)) },
                )
            })
            .collect()
    }
    /// The see-through table's words for this frame's draws (SEE_THROUGH_HEADER_WORDS), uploaded
    /// when they change. A material's coverage takes its texture's mean alpha once the probe has
    /// read it back (`Material::see_through_coverage`), so the words change then too.
    fn write_see_through(
        &mut self,
        gpu: (&wgpu::Device, &wgpu::Queue),
        ledger: &Ledger,
        draws: &[Draw<'_>],
        see_through: &[Draw<'_>],
    ) {
        let (device, queue) = gpu;
        let first: usize = draws.iter().map(|d| d.instances.len()).sum();
        let listed = || see_through.iter().filter(|d| d.mesh.acceleration.as_ref().is_some_and(|m| !m.dynamic));
        let count: usize = listed().map(|d| d.instances.len()).sum();
        let mut words = vec![first as u32, count as u32, 0, 0];
        for draw in listed() {
            let (rgb, coverage, ior) = draw.material.map_or(([1.0; 3], 1.0, 1.0), |m| {
                (
                    [m.base_color[0], m.base_color[1], m.base_color[2]],
                    m.see_through_coverage().unwrap_or(1.0),
                    m.transparency.map_or(1.0, |t| t.ior),
                )
            });
            let record = [
                pack2x16float(rgb[0], rgb[1]),
                pack2x16float(rgb[2], coverage),
                f32::to_bits(ior),
                0,
            ];
            for _ in 0..draw.instances.len() {
                words.extend_from_slice(&record);
            }
        }
        let bytes = (words.len() * 4) as u64;
        let fits = self.see_through.as_ref().is_some_and(|(b, _)| b.size() >= bytes);
        if !fits {
            let buffer = ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("see-through table"),
                    size: bytes.next_power_of_two().max(256),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            self.see_through = Some((buffer, Vec::new()));
        }
        let (buffer, last) = self.see_through.as_mut().unwrap();
        if *last != words {
            let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            queue.write_buffer(buffer, 0, &raw);
            *last = words;
        }
    }
    /// The see-through table (SEE_THROUGH_HEADER_WORDS) for rays that pass see-through
    /// placements by their transmission; None before the first scene.
    pub fn see_through_table(&self) -> Option<&wgpu::Buffer> {
        self.see_through.as_ref().map(|(b, _)| &**b)
    }
    pub fn encode_scene(&self, encoder: &mut wgpu::CommandEncoder) {
        // This frame's new non-opaque BLASes first, in the same build as the TLAS that uses them.
        let pending = std::mem::take(&mut *self.pending_clear.lock().unwrap());
        let entries: Vec<_> = pending
            .iter()
            .map(|(mesh, blas)| wgpu::BlasBuildEntry {
                blas: &blas.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size: &mesh.size_clear,
                    vertex_buffer: &mesh.vertex,
                    first_vertex: 0,
                    vertex_stride: 48,
                    index_buffer: Some(&mesh.index),
                    first_index: Some(0),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .collect();
        encoder.build_acceleration_structures(
            entries.iter(),
            [&*self.active.as_ref().unwrap().tlas],
        );
    }
    pub fn scene_submitted(&mut self, ledger: &Ledger, submission: u64) {
        let scratch = scene_estimate(self.active.as_ref().unwrap().placements.len())
            .unwrap()
            .1;
        self.pending.push((
            submission,
            PendingBuild {
                _mesh: None,
                _scratch: ledger.allocate(AllocationKind::HardwareScratchEstimate, scratch),
            },
        ));
        self.stats.tlas_builds += 1;
    }
    /// Scratch for the skinned BLASes this scene rebuilds, each mesh once.
    fn dynamic_scratch(placements: &[Placement]) -> Result<u64, Error> {
        let mut dynamic = std::collections::HashSet::new();
        placements
            .iter()
            .filter(|p| p.mesh.dynamic && dynamic.insert(Arc::as_ptr(&p.mesh)))
            .try_fold(0u64, |n, p| {
                n.checked_add(mesh_estimate(u64::from(p.mesh.size.index_count.unwrap()) / 3)?.1)
                    .ok_or(Error::InvalidInput("dynamic BLAS scratch overflow"))
            })
    }
    /// The active TLAS object's generation, counting one prepared this frame: the key for
    /// bind groups over the scene. Rebuilds into the same object keep it.
    /// The active TLAS's instance count (measurement logs).
    pub fn active_placements(&self) -> Option<usize> {
        self.active.as_ref().map(|a| a.placements.len())
    }
    pub fn scene_generation(&self) -> u64 {
        self.generation
    }
    pub fn scene(&self) -> &wgpu::Tlas {
        &self.active.as_ref().unwrap().tlas
    }
    /// TP1: the current TLAS, if one is built (transparent receivers' sun rays).
    pub fn active_scene(&self) -> Option<&wgpu::Tlas> {
        self.active.as_ref().map(|a| &*a.tlas)
    }
    pub fn bind(&self) -> &wgpu::BindGroup {
        &self.active.as_ref().unwrap().bind
    }
    pub fn collect(&mut self, completed: u64, gpu: (&wgpu::Device, &wgpu::Queue), ledger: &Ledger) {
        let meshes: Vec<_> = self
            .pending
            .iter()
            .filter(|(last, _)| *last <= completed)
            .filter_map(|(_, p)| p._mesh.clone())
            .collect();
        self.retired.retain(|(last, _)| *last > completed);
        self.pending.retain(|(last, _)| *last > completed);
        // Compact completed uploads incrementally, rather than overlapping every
        // original BLAS with every compacted copy at the first rendered frame.
        for mesh in meshes {
            self.compact_mesh(&mesh, gpu, ledger, 0);
        }
    }
    pub fn release_scene(&mut self, last_submission: u64) {
        if let Some(active) = self.active.take() {
            self.retired.push((last_submission, active));
        }
    }
    /// The allocator report behind `observed_build_scratch_bytes` walks every allocation, so it
    /// runs here, when a caller reads the stats, not on every build (U-FIXED).
    pub fn stats(&self, ledger: &Ledger, device: &wgpu::Device) -> HardwareShadowStats {
        let owned = ledger.snapshot();
        HardwareShadowStats {
            active_instances: self
                .active
                .as_ref()
                .map_or(0, |s| s.placements.len() as u32),
            retained_generations: self.retired.len(),
            pending_builds: self.pending.len(),
            estimated_as_bytes: owned.hardware_as_estimated_bytes.live,
            estimated_pending_scratch_bytes: owned.hardware_scratch_estimated_bytes.live,
            observed_as_bytes: self
                .accounting_available
                .then_some(owned.hardware_as_observed_bytes.live),
            observed_build_scratch_bytes: device.generate_allocator_report().and_then(|r| {
                r.allocations
                    .iter()
                    .filter(|a| a.name == "(wgpu) scratch buffer")
                    .try_fold(0u64, |sum, a| sum.checked_add(a.size))
                    .filter(|n| *n > 0)
            }),
            ..self.stats
        }
    }
}

impl Renderer {
    /// None means the ordinary featureless/raster renderer was selected at creation.
    pub fn hardware_shadow_stats(&self) -> Option<HardwareShadowStats> {
        self.hardware_shadows
            .as_ref()
            .map(|s| s.stats(&self.ledger, &self.device))
    }
    pub(crate) fn reject_virtual_hardware_shadows(&self, mode: ViewMode) -> Result<(), Error> {
        if self.hardware_shadows.is_some() && self.view.shadows.enabled(mode) {
            return Err(Error::InvalidInput(
                "hardware shadows do not yet support virtual geometry; use the raster renderer",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
impl Renderer {
    pub(crate) fn test_hardware_busy(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        mesh: &Mesh,
        instances: &mut Instances,
    ) {
        self.test_drain("hardware busy admission baseline");
        for slot in &mut self.slots {
            slot.flight = Some(Flight {
                done: Arc::new(OnceLock::new()),
                mapped: None,
                lit: false,
                shadowed: false,
                fresh: false,
                submission: self.submissions,
                frame: None,
                view: None,
                revision: self.revision,
                submitted_at: Instant::now(),
            });
        }
        instances
            .set(0, Mat4::from_translation(Vec3::new(0.1, 0., 0.)))
            .unwrap();
        let before = self.stats();
        let builds = self.hardware_shadow_stats().unwrap().tlas_builds;
        assert!(
            self.try_render_view(
                color,
                camera,
                &mut [Draw::new(mesh, instances)],
                ViewMode::Lit
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(self.stats().uploads, before.uploads);
        assert_eq!(self.stats().ownership, before.ownership);
        assert_eq!(self.hardware_shadow_stats().unwrap().tlas_builds, builds);
        for slot in &self.slots {
            slot.flight.as_ref().unwrap().done.set(()).unwrap();
        }
        self.collect().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_instance_rows_preserve_affine_draw_transform() {
        let model = Mat4::from_cols_array(&[
            0., 3., 0., 0., -2., 0., 0., 0., 0., 0., 1., 0., 3., 4., 5., 1.,
        ]);
        assert_eq!(
            row_transform(model),
            [0., -2., 0., 3., 3., 0., 0., 4., 0., 0., 1., 5.]
        );
        assert_eq!(
            mesh_estimate(13_034_903).unwrap(),
            (3_336_935_168, 1_668_467_584)
        );
        assert!(mesh_estimate(u64::MAX).is_err());
    }
}
