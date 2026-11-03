use super::*;
use crate::shadow_plan::{self as plan, Bounds, LocalRange, ShadowView};
use crate::shadows::VIEW_BYTES;
use glam::DMat4;

pub(super) struct ShadowDraw<'a> {
    pub mesh: &'a Mesh,
    pub instances: &'a Instances,
    pub material: Option<&'a Material>,
    pub indices: std::ops::Range<u32>,
    pub slot: u32,
}
pub(super) struct ShadowFrame {
    pub views: Vec<ShadowView>,
    pub light_ranges: Vec<[u32; 2]>,
    bases: Vec<Option<DMat4>>,
    records: Vec<u8>,
}
struct ShadowResources {
    _texture: Owned<wgpu::Texture>,
    sample_view: Owned<wgpu::TextureView>,
    layers: Vec<Owned<wgpu::TextureView>>,
    records: Owned<wgpu::Buffer>,
    uniforms: Owned<wgpu::Buffer>,
    binds: Vec<Owned<wgpu::BindGroup>>,
    _allocation: Allocation,
    resolution: u32,
}
pub(super) struct ShadowState {
    pub settings: Shadows,
    pub limits: ShadowLimits,
    pub bindings: Owned<wgpu::BindGroupLayout>,
    pub layout: Owned<wgpu::PipelineLayout>,
    active: ShadowResources,
    retired: Vec<(u64, ShadowResources)>,
    bases: Vec<Option<DMat4>>,
    pub stats: ShadowStats,
    pub published_hash: [u8; 32],
    ranges_published: bool,
}
impl ShadowState {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        instances: &wgpu::BindGroupLayout,
        materials: &wgpu::BindGroupLayout,
    ) -> Self {
        let d = device.limits();
        let limits = ShadowLimits {
            max_resolution: d.max_texture_dimension_2d,
            max_views: d.max_texture_array_layers.min(
                (d.max_storage_buffer_binding_size / VIEW_BYTES).min(u64::from(u32::MAX)) as u32,
            ),
            max_buffer_bytes: d.max_buffer_size,
            uniform_alignment: d.min_uniform_buffer_offset_alignment,
        };
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("one authoritative shadow matrix per view"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(VIEW_BYTES),
                    },
                    count: None,
                }],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow depth ABI"),
                bind_group_layouts: &[Some(&bindings), Some(instances), Some(materials)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        // Start with a tiny fallback. After use, disabling retains the last full generation;
        // disabled frames perform no shadow raster, matrix uploads or sampling.
        let active = Self::resources(device, ledger, &bindings, limits, 1, 1);
        Self {
            settings: Shadows::default(),
            limits,
            bindings,
            layout,
            active,
            retired: vec![],
            bases: vec![],
            stats: ShadowStats::default(),
            published_hash: [0; 32],
            ranges_published: false,
        }
    }
    fn resources(
        device: &wgpu::Device,
        ledger: &Ledger,
        bindings: &wgpu::BindGroupLayout,
        limits: ShadowLimits,
        resolution: u32,
        views: u32,
    ) -> ShadowResources {
        let texture = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("redrawn hard shadow depth array"),
                size: wgpu::Extent3d {
                    width: resolution,
                    height: resolution,
                    depth_or_array_layers: views,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let sample_view = ledger.own(
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            ResourceKind::TextureView,
        );
        let layers = (0..views)
            .map(|layer| {
                ledger.own(
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: layer,
                        array_layer_count: Some(1),
                        ..Default::default()
                    }),
                    ResourceKind::TextureView,
                )
            })
            .collect();
        let stride = VIEW_BYTES.div_ceil(u64::from(limits.uniform_alignment))
            * u64::from(limits.uniform_alignment);
        let buffer = |label, size, usage| {
            ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        let records = buffer(
            "shadow sampling records",
            u64::from(views) * VIEW_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let uniforms = buffer(
            "disjoint shadow raster matrices",
            u64::from(views) * stride,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let binds = (0..views)
            .map(|i| {
                ledger.own(
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("one shadow view"),
                        layout: bindings,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &uniforms,
                                offset: u64::from(i) * stride,
                                size: wgpu::BufferSize::new(VIEW_BYTES),
                            }),
                        }],
                    }),
                    ResourceKind::BindGroup,
                )
            })
            .collect();
        ShadowResources {
            _texture: texture,
            sample_view,
            layers,
            records,
            uniforms,
            binds,
            _allocation: ledger.allocate(
                AllocationKind::Shadow,
                u64::from(views) * (4 * u64::from(resolution).pow(2) + stride + VIEW_BYTES),
            ),
            resolution,
        }
    }
    pub fn sample_view(&self) -> &wgpu::TextureView {
        &self.active.sample_view
    }
    pub fn records(&self) -> &wgpu::Buffer {
        &self.active.records
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
    pub fn enabled(&self, mode: ViewMode) -> bool {
        mode == ViewMode::Lit && self.settings.enabled()
    }
    pub fn plan(
        &self,
        camera: &Camera,
        viewport: [u32; 2],
        lights: &[Light],
        bounds: &[Bounds],
    ) -> Result<ShadowFrame, Error> {
        let mut views = vec![];
        let mut light_ranges = vec![[0, 0]; lights.len()];
        let mut bases = vec![None; lights.len()];
        for (index, (light, request)) in lights.iter().zip(&self.settings.requests).enumerate() {
            let first = views.len() as u32;
            match (light, request) {
                (_, ShadowRequest::None) => {}
                (Light::Directional { travel, .. }, ShadowRequest::Directional { split_ends }) => {
                    let basis = plan::transported_orientation(
                        *travel,
                        self.bases.get(index).copied().flatten(),
                    )?;
                    views.extend(plan::directional_oriented(
                        camera,
                        viewport,
                        basis,
                        split_ends,
                        self.settings.resolution,
                        bounds,
                    )?);
                    bases[index] = Some(basis);
                }
                (
                    Light::Spot {
                        position,
                        travel,
                        outer,
                        ..
                    },
                    ShadowRequest::Spot { near, far },
                ) => {
                    validate_local_casters(*position, *near, bounds)?;
                    let basis = plan::transported_orientation(
                        *travel,
                        self.bases.get(index).copied().flatten(),
                    )?;
                    views.push(plan::spot_oriented(
                        *position,
                        basis,
                        *outer,
                        LocalRange {
                            near: *near,
                            far: *far,
                        },
                        self.settings.resolution,
                        bounds,
                    )?);
                    bases[index] = Some(basis);
                }
                (Light::Point { position, .. }, ShadowRequest::Point { near, far }) => {
                    validate_local_casters(*position, *near, bounds)?;
                    views.extend(plan::point(
                        *position,
                        LocalRange {
                            near: *near,
                            far: *far,
                        },
                        self.settings.resolution,
                        bounds,
                    )?);
                }
                _ => unreachable!("validated whole shadow/light set"),
            }
            light_ranges[index] = [first, views.len() as u32 - first];
        }
        let mut records = Vec::with_capacity(views.len() * VIEW_BYTES as usize);
        // The final row is camera-forward depth, shared by raster metadata and sampling.
        let camera_depth = -camera.view().transpose().z_axis;
        for view in &views {
            debug_assert_eq!(view.resolution, self.settings.resolution);
            debug_assert!(view.world_to_view.is_finite() && view.view_to_clip.is_finite());
            debug_assert!(view.depth[1] > view.depth[0]);
            records.extend_from_slice(&matrix_bytes(view.world_to_clip));
            let (near, far) = match view.kind {
                plan::ViewKind::Cascade { near, far } => (near, far),
                _ => (0.0, 0.0),
            };
            for value in [
                near,
                far,
                self.settings.depth_bias,
                self.settings.normal_bias,
            ]
            .into_iter()
            .chain(camera_depth.to_array())
            .chain([view.world_units_per_texel.unwrap_or(0.0), 0.0, 0.0, 0.0])
            {
                records.extend_from_slice(&value.to_ne_bytes());
            }
        }
        for (request, &[first, count]) in self.settings.requests.iter().zip(&light_ranges) {
            if let ShadowRequest::Point { near, far } | ShadowRequest::Spot { near, far } = request
            {
                for layer in first..first + count {
                    let offset = layer as usize * VIEW_BYTES as usize + 64;
                    records[offset..offset + 4].copy_from_slice(&near.to_ne_bytes());
                    records[offset + 4..offset + 8].copy_from_slice(&far.to_ne_bytes());
                }
            }
        }
        Ok(ShadowFrame {
            views,
            light_ranges,
            bases,
            records,
        })
    }
}

/// F64 affine corners plus outward f32 rounding retain the full submitted geometry.
pub(super) fn world_bounds(bounds: Aabb, model: Mat4) -> Result<Bounds, Error> {
    let mut min = glam::DVec3::splat(f64::INFINITY);
    let mut max = glam::DVec3::splat(f64::NEG_INFINITY);
    for i in 0..8 {
        let p = glam::DVec3::from_array(std::array::from_fn(|axis| {
            f64::from(if i & (1 << axis) == 0 {
                bounds.min[axis]
            } else {
                bounds.max[axis]
            })
        }));
        let rows = model.as_dmat4().transpose();
        let magnitude = p.abs().extend(1.0);
        let guard = glam::DVec3::new(
            rows.x_axis.abs().dot(magnitude),
            rows.y_axis.abs().dot(magnitude),
            rows.z_axis.abs().dot(magnitude),
        ) * (8.0 * f64::from(f32::EPSILON));
        let p = model.as_dmat4().transform_point3(p);
        min = min.min(p - guard);
        max = max.max(p + guard);
    }
    let lower = min.to_array().map(|v| (v as f32).next_down());
    let upper = max.to_array().map(|v| (v as f32).next_up());
    let result = Bounds {
        min: Vec3::from_array(lower),
        max: Vec3::from_array(upper),
    };
    if !result.min.is_finite() || !result.max.is_finite() {
        return Err(Error::InvalidInput(
            "submitted shadow caster world bounds overflow",
        ));
    }
    Ok(result)
}
fn validate_local_casters(position: Vec3, near: f32, bounds: &[Bounds]) -> Result<(), Error> {
    let p = position.as_dvec3();
    if bounds.iter().any(|b| {
        p.distance_squared(p.clamp(b.min.as_dvec3(), b.max.as_dvec3())) <= f64::from(near).powi(2)
    }) {
        return Err(Error::InvalidInput(
            "local shadow near sphere intersects submitted caster bounds; move the light or reduce near (no near-domain clipping is hidden)",
        ));
    }
    Ok(())
}

impl Renderer {
    #[cfg(test)]
    pub(crate) fn test_shadow_admission(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        mesh: &Mesh,
        instances: &mut Instances,
    ) {
        self.test_drain("shadow admission setup");
        for slot in &mut self.slots {
            slot.flight = Some(Flight {
                done: Arc::new(OnceLock::new()),
                mapped: None,
                lit: false,
                shadowed: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at: Instant::now(),
            });
        }
        let before = self.stats();
        let generation = self.shadow_stats().generation;
        for resolution in [128, 256] {
            let shadows = Shadows {
                requests: vec![ShadowRequest::Directional {
                    split_ends: vec![10.],
                }],
                resolution,
                ..Default::default()
            };
            self.set_shadows(&shadows).unwrap();
        }
        instances
            .set(0, Mat4::from_translation(Vec3::new(0.1, 0., 0.)))
            .unwrap();
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
        assert_eq!(before.uploads, self.stats().uploads);
        assert_eq!(before.ownership, self.stats().ownership);
        assert_eq!(generation, self.shadow_stats().generation);
        for slot in &self.slots {
            slot.flight.as_ref().unwrap().done.set(()).unwrap();
        }
        self.collect().unwrap();
        let report = self
            .try_render_view(
                color,
                camera,
                &mut [Draw::new(mesh, instances)],
                ViewMode::Lit,
            )
            .unwrap()
            .unwrap();
        assert_eq!(self.shadow_stats().generation, generation + 1);
        assert_eq!(self.shadow_stats().resolution, 256);
        assert_eq!(report.uploads.models, 64);
        assert!(report.uploads.shadows > 0);
        assert_eq!(report.uploads.geometry, 0);
        self.test_drain("shadow admission cleanup");
    }
    pub fn set_shadows(&mut self, shadows: &Shadows) -> Result<(), Error> {
        self.check_fault()?;
        shadows.validate(&self.lighting.source_lights, self.shadows.limits)?;
        self.shadows.settings.clone_from(shadows);
        Ok(())
    }
    /// Replace lights and their corresponding shadow requests as one CPU transaction.
    pub fn set_lighting_and_shadows(
        &mut self,
        lighting: &Lighting,
        shadows: &Shadows,
    ) -> Result<(), Error> {
        self.check_fault()?;
        let exposure = crate::lighting::PackedLighting::settings(lighting)?;
        let packed = if self.lighting.source_lights == lighting.lights {
            None
        } else {
            Some(crate::lighting::PackedLighting::new(
                lighting,
                self.max_lights(),
            )?)
        };
        shadows.validate(&lighting.lights, self.shadows.limits)?;
        if let Some(packed) = packed {
            self.lighting.pending = packed;
            self.lighting.source_lights.clone_from(&lighting.lights);
        } else {
            self.lighting.pending.environment = lighting.environment;
            self.lighting.pending.exposure = exposure;
        }
        self.shadows.settings.clone_from(shadows);
        Ok(())
    }
    pub fn shadow_limits(&self) -> ShadowLimits {
        self.shadows.limits
    }
    pub fn shadow_stats(&self) -> ShadowStats {
        ShadowStats {
            retained_generations: self.shadows.retired.len(),
            ..self.shadows.stats
        }
    }
    pub(super) fn upload_shadows(
        &mut self,
        frame: Option<&ShadowFrame>,
        uploads: &mut UploadBytes,
    ) -> Result<(), Error> {
        let state = &mut self.shadows;
        if frame.is_none() && !state.ranges_published {
            return Ok(());
        }
        // Patch only the private shadow range in each light record, after admission.
        for (index, record) in self
            .lighting
            .pending
            .records
            .chunks_exact_mut(64)
            .enumerate()
        {
            let range = frame.map_or([0, 0], |f| f.light_ranges[index]);
            record[52..56].copy_from_slice(&range[0].to_ne_bytes());
            record[56..60].copy_from_slice(&range[1].to_ne_bytes());
        }
        state.ranges_published = frame.is_some();
        let Some(frame) = frame else {
            return Ok(());
        };
        if state.active.resolution != state.settings.resolution
            || state.active.layers.len() != frame.views.len()
        {
            let scopes = OperationScopes::new(&self.device);
            let next = ShadowState::resources(
                &self.device,
                &self.ledger,
                &state.bindings,
                state.limits,
                state.settings.resolution,
                frame.views.len() as u32,
            );
            if let Some(error) = scopes.finish() {
                return Err(self.fail(error.to_string()));
            }
            let old = std::mem::replace(&mut state.active, next);
            state.retired.push((self.submissions, old));
            state.stats.resource_generation += 1;
        }
        self.queue
            .write_buffer(&state.active.records, 0, &frame.records);
        let stride = VIEW_BYTES.div_ceil(u64::from(state.limits.uniform_alignment))
            * u64::from(state.limits.uniform_alignment);
        let mut uniforms = vec![0; frame.views.len() * stride as usize];
        for (dst, src) in uniforms
            .chunks_exact_mut(stride as usize)
            .zip(frame.records.chunks_exact(VIEW_BYTES as usize))
        {
            dst[..VIEW_BYTES as usize].copy_from_slice(src);
        }
        self.queue
            .write_buffer(&state.active.uniforms, 0, &uniforms);
        uploads.shadows += (frame.records.len() + uniforms.len()) as u64;
        uploads.write_calls += 2;
        Ok(())
    }
    pub(super) fn encode_shadows(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &ShadowFrame,
        draws: &[ShadowDraw<'_>],
        slot: usize,
        timed: bool,
    ) {
        for (index, view) in frame.views.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow generation"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadows.active.layers[index],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self
                    .timing
                    .as_ref()
                    .filter(|_| timed && (index == 0 || index + 1 == frame.views.len()))
                    .map(|t| wgpu::RenderPassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: (index == 0).then_some(slot as u32 * 6 + 4),
                        end_of_pass_write_index: (index + 1 == frame.views.len())
                            .then_some(slot as u32 * 6 + 5),
                    }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline.shadow);
            pass.set_bind_group(0, &*self.shadows.active.binds[index], &[]);
            let (mut instances, mut material, mut mesh) = (None, None, None);
            for &caster in &view.caster_indices {
                let draw = &draws[caster];
                let bind: &wgpu::BindGroup = &draw.instances.bind;
                if changed(&mut instances, bind) {
                    pass.set_bind_group(1, bind, &[]);
                }
                let bind: &wgpu::BindGroup = &draw
                    .material
                    .unwrap_or(self.default_material.as_ref().unwrap())
                    .bind;
                if changed(&mut material, bind) {
                    pass.set_bind_group(2, bind, &[]);
                }
                if changed(&mut mesh, draw.mesh) {
                    pass.set_vertex_buffer(0, draw.mesh.vertex.slice(..));
                    pass.set_index_buffer(draw.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                }
                // Builtin instance_index addresses the original transform slot directly.
                pass.draw_indexed(draw.indices.clone(), 0, draw.slot..draw.slot + 1);
            }
        }
    }
    pub(super) fn stamp_shadows(&mut self, frame: ShadowFrame, draws: &[ShadowDraw<'_>]) {
        let mut count = 0;
        let mut triangles = 0;
        for view in &frame.views {
            for &caster in &view.caster_indices {
                count += 1;
                triangles +=
                    u64::from((draws[caster].indices.end - draws[caster].indices.start) / 3);
            }
        }
        let mut hash = Sha256::new();
        hash.update(&frame.records);
        hash.update(self.shadows.settings.resolution.to_ne_bytes());
        self.shadows.published_hash = hash.finalize().into();
        self.shadows.bases = frame.bases;
        let stats = &mut self.shadows.stats;
        stats.generation += 1;
        stats.views = frame.views.len() as u32;
        stats.resolution = self.shadows.settings.resolution;
        stats.bytes = self
            .shadows
            .settings
            .bytes(stats.views, self.shadows.limits)
            .expect("validated shadow bytes");
        stats.caster_draws = count;
        stats.caster_triangles = triangles;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadow_near_domain_and_mirrored_bounds() {
        let b = world_bounds(
            Aabb {
                min: [-1.; 3],
                max: [1.; 3],
            },
            Mat4::from_scale_rotation_translation(
                Vec3::new(-2., 1., 0.5),
                glam::Quat::from_rotation_y(0.3),
                Vec3::new(5., 0., 0.),
            ),
        )
        .unwrap();
        assert!(validate_local_casters(Vec3::new(5., 0., 0.), 0.001, &[b]).is_err());
        assert!(validate_local_casters(Vec3::ZERO, 0.001, &[b]).is_ok());
        assert!(b.min.x < 4. && b.max.x > 6.);
    }

    #[test]
    fn shadow_bounds_enclose_actual_f32_affine_corners() {
        let local = Aabb {
            min: [-1.3, -0.02, -7.],
            max: [2.7, 0.03, 5.],
        };
        for offset in [Vec3::ZERO, Vec3::new(2_000_000., -1_000_000., 3_000_000.)] {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(-2., 0.3, 7.),
                glam::Quat::from_rotation_y(0.6),
                offset,
            );
            let bounds = world_bounds(local, model).unwrap();
            for i in 0..8 {
                let point = Vec3::from_array(std::array::from_fn(|axis| {
                    if i & (1 << axis) == 0 {
                        local.min[axis]
                    } else {
                        local.max[axis]
                    }
                }));
                let actual = (model * point.extend(1.)).truncate();
                assert!(actual.cmpge(bounds.min).all() && actual.cmple(bounds.max).all());
            }
        }
    }
}
