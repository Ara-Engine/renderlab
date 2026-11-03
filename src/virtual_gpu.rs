use super::*;
// Renderer-private integration: virtual geometry uses the same pass as ordinary Draw.
impl Renderer {
    pub(crate) fn virtual_admit_scene(&self, count: usize) -> Result<(), Error> {
        if count > self.limits.max_instances as usize {
            return Err(Error::InvalidInput(
                "virtual scene exceeds renderer max_instances",
            ));
        }
        Ok(())
    }
    /// Completion watermark independent of the bounded, lossy observation queue.
    pub fn completed_submission(&self) -> u64 {
        self.completed_submission
    }
    pub(crate) fn virtual_collect(&mut self) -> Result<(), Error> {
        self.collect().map(|_| ())
    }
    pub(crate) fn virtual_identity(&self, identity: &mut Option<Arc<()>>) -> Result<(), Error> {
        if identity
            .as_ref()
            .is_some_and(|id| !Arc::ptr_eq(id, &self.identity))
        {
            return Err(Error::InvalidInput(
                "virtual asset belongs to another renderer",
            ));
        }
        *identity = Some(self.identity.clone());
        Ok(())
    }
    pub(crate) fn virtual_resources(
        &mut self,
        asset: &mut crate::virtual_geometry::VirtualAsset,
    ) -> Result<(), Error> {
        if let Some(data) = asset.package.resources.textures.get(asset.textures.len()) {
            if let Some(texture) = self.try_upload_texture(data)? {
                asset.textures.push(texture);
            }
        } else if let Some(data) = asset.package.resources.materials.get(asset.materials.len()) {
            let descriptor = MaterialDescriptor {
                base_color: data.base_color,
                metallic: data.metallic,
                roughness: data.roughness,
                double_sided: data.double_sided,
                normal_scale: data.normal_texture.map_or(1., |b| b.scale),
                normal_texture: data.normal_texture.map(|b| TextureBinding {
                    texture: &asset.textures[b.texture as usize],
                    sampler: b.sampler,
                }),
                base_color_texture: data.base_color_texture.map(|b| TextureBinding {
                    texture: &asset.textures[b.texture as usize],
                    sampler: b.sampler,
                }),
                metallic_roughness_texture: data.metallic_roughness_texture.map(|b| {
                    TextureBinding {
                        texture: &asset.textures[b.texture as usize],
                        sampler: b.sampler,
                    }
                }),
            };
            if let Some(material) = self.try_create_material(&descriptor)? {
                asset.materials.push(material);
            }
        }
        Ok(())
    }
    /// Caller reserves a full page slot plus staging before entering this allocator.
    pub(crate) fn try_upload_virtual_page(
        &mut self,
        data: &asset::virtual_mesh::PageGeometry,
    ) -> Result<Option<crate::virtual_geometry::GpuPage>, Error> {
        self.check_fault()?;
        let tangents = data.tangents.as_ref();
        let data = MeshData {
            vertices: &data.vertices,
            indices: &data.indices,
        };
        let sizes = data.validate(self.limits)?;
        let tangent_bytes = self.validate_mesh_tangents(&data, tangents, sizes)?;
        self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let scopes = OperationScopes::new(&self.device);
        let buffer = |label, size, usage| {
            self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        let vertex = buffer(
            "virtual page vertices",
            sizes.0,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let index = buffer(
            "virtual page indices",
            sizes.1,
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        );
        let tangent = tangents.map(|_| {
            buffer(
                "virtual page tangents",
                tangent_bytes,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            )
        });
        let staging = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("charged virtual page staging"),
                size: sizes.0 + sizes.1 + tangent_bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            }),
            ResourceKind::Buffer,
        );
        {
            let mut view = staging
                .get_mapped_range_mut(..)
                .map_err(|e| self.fail(e.to_string()))?;
            let mut offset = 0;
            for v in data.vertices {
                let normal = glam::DVec3::from_array(v.normal.map(f64::from))
                    .normalize()
                    .as_vec3()
                    .to_array();
                for value in v
                    .position
                    .into_iter()
                    .chain(normal)
                    .chain(v.uv)
                    .chain(v.color)
                {
                    view.slice(offset..offset + 4)
                        .copy_from_slice(&value.to_ne_bytes());
                    offset += 4;
                }
            }
            for value in data.indices {
                view.slice(offset..offset + 4)
                    .copy_from_slice(&value.to_ne_bytes());
                offset += 4;
            }
            if let Some(t) = tangents {
                for value in t.values.iter().flatten() {
                    view.slice(offset..offset + 4)
                        .copy_from_slice(&value.to_ne_bytes());
                    offset += 4;
                }
            }
        }
        staging.unmap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&staging, 0, &vertex, 0, sizes.0);
        encoder.copy_buffer_to_buffer(&staging, sizes.0, &index, 0, sizes.1);
        if let Some(t) = &tangent {
            encoder.copy_buffer_to_buffer(&staging, sizes.0 + sizes.1, t, 0, tangent_bytes);
        }
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        let commands = encoder.finish();
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        let submitted_at = Instant::now();
        self.queue.submit([commands]);
        self.submissions += 1;
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.uploads.geometry += sizes.0 + sizes.1 + tangent_bytes;
        self.check_fault()?;
        Ok(Some(crate::virtual_geometry::GpuPage {
            mesh: Mesh {
                tangent,
                renderer: self.identity.clone(),
                vertex,
                index,
                index_count: data.indices.len() as u32,
                bounds: Aabb::from_positions(data.vertices.iter().map(|v| v.position)).unwrap(),
            },
            staging: Some(staging),
            upload: self.submissions,
            last_use: self.submissions,
        }))
    }
    /// One bounded streaming wake followed by the ordinary material/depth/MSAA/timestamp pass.
    /// None is nonblocking startup or submission backpressure; retain the previously presented image.
    pub fn try_render_virtual(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        asset: &mut crate::virtual_geometry::VirtualAsset,
        scene: &mut crate::virtual_geometry::VirtualScene,
        view_mode: ViewMode,
    ) -> Result<Option<FrameReport>, Error> {
        self.validate_color(color)?;
        if !Arc::ptr_eq(&asset.identity, &scene.asset) {
            return Err(Error::InvalidInput(
                "virtual scene belongs to another asset",
            ));
        }
        asset.advance(self, scene, camera, color.height())?;
        self.render_virtual_cover(color, camera, asset, scene, view_mode)
    }
    /// Draw the current view after `VirtualAsset::prepare_view` has settled. Does not advance
    /// streaming a second time. Degraded preparation draws its legal fallback; inspect the status
    /// before calling to label that outcome. After first presentation use `try_render_virtual`.
    pub fn try_render_prepared_virtual(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        asset: &mut crate::virtual_geometry::VirtualAsset,
        scene: &mut crate::virtual_geometry::VirtualScene,
        view_mode: ViewMode,
    ) -> Result<Option<FrameReport>, Error> {
        self.validate_color(color)?;
        self.virtual_identity(&mut asset.renderer)?;
        use crate::virtual_geometry::PreparationState;
        match scene.preparation(asset, camera, color.height())?.state {
            PreparationState::Pending => return Ok(None),
            PreparationState::Failed(_) => {
                return Err(Error::InvalidInput(
                    "virtual cover page failed; inspect asset.failure and explicitly retry after repair",
                ));
            }
            PreparationState::Settled | PreparationState::Degraded => {}
        }
        self.render_virtual_cover(color, camera, asset, scene, view_mode)
    }
    fn render_virtual_cover(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        asset: &mut crate::virtual_geometry::VirtualAsset,
        scene: &mut crate::virtual_geometry::VirtualScene,
        view_mode: ViewMode,
    ) -> Result<Option<FrameReport>, Error> {
        if !scene.ready(asset) {
            return Ok(None);
        }
        let preparation_start = Instant::now();
        let matrix = camera.matrix(color.width(), color.height());
        if !matrix.is_finite()
            || (view_mode == ViewMode::Lit && !camera.view().inverse().w_axis.is_finite())
        {
            return Err(Error::InvalidInput("virtual camera projection overflow"));
        }
        let preparation = preparation_start.elapsed();
        let poll = self.collect()?;
        let Some(slot) = self.free_slot() else {
            self.busy_frames += 1;
            return Ok(None);
        };
        let camera_bytes = matrix_bytes(matrix);
        let mut uploads = UploadBytes::default();
        let mut packing = Duration::ZERO;
        let mut upload = Duration::ZERO;
        let instances = scene.instances.as_mut().unwrap();
        if view_mode == ViewMode::Lit && instances.state.invalid_normals > 0 {
            return Err(Error::InvalidInput(
                "virtual lit/shadow geometry requires valid affine normal transforms",
            ));
        }
        let shadow_start = Instant::now();
        // Hierarchy bounds cover every LOD of the submitted placement. The caster
        // range below is exactly the same published cut used by the camera pass.
        let placement_bounds = if self.shadows.enabled(view_mode) {
            scene
                .meshes
                .iter()
                .zip(&instances.state.models)
                .map(|(&mesh, &model)| {
                    let mesh = &asset.package.meshes[mesh as usize];
                    let mut min = Vec3::splat(f32::INFINITY);
                    let mut max = Vec3::splat(f32::NEG_INFINITY);
                    for root in mesh.terminal_groups() {
                        let bounds = mesh.groups[root.index()].simplified;
                        let center = Vec3::from_array(bounds.center);
                        min = min.min(Vec3::from_array(std::array::from_fn(|axis| {
                            ((f64::from(center[axis]) - f64::from(bounds.radius)) as f32)
                                .next_down()
                        })));
                        max = max.max(Vec3::from_array(std::array::from_fn(|axis| {
                            ((f64::from(center[axis]) + f64::from(bounds.radius)) as f32).next_up()
                        })));
                    }
                    shadow_gpu::world_bounds(
                        Aabb {
                            min: min.to_array(),
                            max: max.to_array(),
                        },
                        model,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![]
        };
        let mut shadow_frame = if self.shadows.enabled(view_mode) {
            Some(self.shadows.plan(
                camera,
                [color.width(), color.height()],
                &self.lighting.source_lights,
                &placement_bounds,
            )?)
        } else {
            None
        };
        let mut shadow_preparation = if shadow_frame.is_some() {
            shadow_start.elapsed()
        } else {
            Duration::ZERO
        };
        self.upload_instance_edits(instances, &mut uploads, &mut packing, &mut upload);
        let mut prepared: Vec<PreparedDraw<'_>> =
            Vec::with_capacity(scene.selections.iter().map(|s| s.cut.clusters.len()).sum());
        let mut counts = SceneCounts {
            collections: 1,
            total_instances: instances.len() as u64,
            visible_instances: instances.len() as u64,
            normal_gpu_bytes: instances.normals.size(),
            normal_cache_bytes: (instances.state.normals.capacity()
                * std::mem::size_of::<crate::normal::NormalTransform>())
                as u64,
            ..Default::default()
        };
        for (placement, selection) in scene.selections.iter().enumerate() {
            let mesh = &asset.package.meshes[scene.meshes[placement] as usize];
            let slots = placement as u32..placement as u32 + 1;
            for cluster in &selection.cut.clusters {
                let cluster = &mesh.clusters[cluster.index()];
                let crate::virtual_geometry::PageState::Resident(page) =
                    &asset.states[cluster.page.index()]
                else {
                    return Err(Error::InvalidInput("virtual cut lost its published page"));
                };
                let material = &asset.materials[cluster.material as usize];
                let indices =
                    cluster.indices.offset..cluster.indices.offset + cluster.indices.count;
                counts.triangles += u64::from(cluster.indices.count / 3);
                // The cooker packs a group's clusters contiguously, so consecutive cut clusters
                // often continue the previous draw. Extending it keeps primitive order intact.
                if let Some(last) = prepared.last_mut()
                    && std::ptr::eq(last.mesh, &page.mesh)
                    && last.material.is_some_and(|m| std::ptr::eq(m, material))
                    && last.slots == slots
                    && last.indices.end == indices.start
                {
                    last.indices.end = indices.end;
                    continue;
                }
                prepared.push(PreparedDraw {
                    mesh: &page.mesh,
                    instances,
                    material: Some(material),
                    indices,
                    slots: slots.clone(),
                    #[cfg(test)]
                    reference_ids: None,
                });
                counts.draws += 1;
                counts.submitted_instances += 1;
            }
        }
        let shadow_start = Instant::now();
        if let Some(frame) = &mut shadow_frame {
            let mut ranges = vec![vec![]; instances.len()];
            for (index, draw) in prepared.iter().enumerate() {
                ranges[draw.slots.start as usize].push(index);
            }
            for view in &mut frame.views {
                view.caster_indices = view
                    .caster_indices
                    .iter()
                    .flat_map(|&placement| ranges[placement].iter().copied())
                    .collect();
            }
        }
        let shadow_draws = if shadow_frame.is_some() {
            prepared
                .iter()
                .map(|draw| ShadowDraw {
                    mesh: draw.mesh,
                    instances: draw.instances,
                    material: draw.material,
                    indices: draw.indices.clone(),
                    slot: draw.slots.start,
                })
                .collect()
        } else {
            Vec::new()
        };
        if shadow_frame.is_some() {
            shadow_preparation += shadow_start.elapsed();
        }
        let result = self.submit_prepared(
            color,
            camera_bytes,
            camera,
            view_mode,
            &prepared,
            shadow_frame,
            &shadow_draws,
            slot,
            uploads,
            counts,
            CpuFrame {
                shadow_preparation,
                preparation,
                packing,
                upload,
                poll,
                ..Default::default()
            },
        )?;
        if result.is_some() {
            for selection in &scene.selections {
                for page in &selection.cut.pages {
                    if let crate::virtual_geometry::PageState::Resident(gpu) =
                        &mut asset.states[page.index()]
                    {
                        gpu.last_use = self.submissions;
                    }
                }
            }
        }
        Ok(result)
    }
}
/// Identity of the stored image when the copy was enqueued. HDR is exposed linear
/// RGBA16F (little endian), after MSAA resolve. Saturated resolved texels do not
/// detect every saturated sample at 4x; use captures below saturation for radiance parity.
#[derive(Clone, Copy, Debug)]
pub struct CaptureInfo {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub samples: SampleCount,
    pub pre_exposure: Option<f32>,
    pub frame: FrameId,
    pub revision: Revision,
    pub shader_sha256: [u8; 32],
    pub lighting_sha256: Option<[u8; 32]>,
}
/// Explicit asynchronous image evidence readback. Separate from ordinary frames and geometry pools.
pub struct Capture {
    buffer: Owned<wgpu::Buffer>,
    mapped: MapResult,
    width: u32,
    height: u32,
    row: u32,
    consumed: bool,
    bytes_per_texel: u32,
    info: CaptureInfo,
}
impl Capture {
    pub fn info(&self) -> CaptureInfo {
        self.info
    }
    pub fn try_read(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if self.consumed {
            return Err(Error::InvalidInput("capture already consumed"));
        }
        let Some(mapped) = self.mapped.get() else {
            return Ok(None);
        };
        mapped
            .as_ref()
            .map_err(|_| Error::InvalidInput("capture mapping failed; inspect GPU diagnostics"))?;
        let view = self
            .buffer
            .get_mapped_range(..)
            .map_err(|e| Error::Shader(e.to_string()))?;
        let mut rgba = Vec::with_capacity(
            self.width as usize * self.height as usize * self.bytes_per_texel as usize,
        );
        for row in view.chunks_exact(self.row as usize) {
            rgba.extend_from_slice(&row[..self.width as usize * self.bytes_per_texel as usize]);
        }
        drop(view);
        self.buffer.unmap();
        self.consumed = true;
        Ok(Some(rgba))
    }
}
impl Renderer {
    pub fn try_capture(&mut self, color: &wgpu::Texture) -> Result<Option<Capture>, Error> {
        self.check_fault()?;
        self.validate_color(color)?;
        if !color.usage().contains(wgpu::TextureUsages::COPY_SRC)
            || !matches!(
                color.format(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
            )
        {
            return Err(Error::InvalidInput("capture requires RGBA8 COPY_SRC color"));
        }
        let info = CaptureInfo {
            width: color.width(),
            height: color.height(),
            format: color.format(),
            samples: self.options.samples,
            pre_exposure: None,
            frame: FrameId(self.frames),
            revision: self.revision,
            shader_sha256: self.shader_hash,
            lighting_sha256: None,
        };
        self.capture_texture(color, info)
    }
    /// Copy the last admitted Lit image without a window or surface. A subsequent
    /// diagnostic frame does not overwrite it; metadata keeps its original identity.
    pub fn try_capture_hdr(&mut self) -> Result<Option<Capture>, Error> {
        self.check_fault()?;
        let hdr = self
            .lighting
            .hdr
            .as_ref()
            .filter(|h| h.frame.0 > 0)
            .ok_or(Error::InvalidInput("no admitted Lit image to capture"))?;
        let color = hdr.resolved.texture.clone();
        let info = CaptureInfo {
            width: color.width(),
            height: color.height(),
            format: color.format(),
            samples: self.options.samples,
            pre_exposure: Some(hdr.exposure),
            frame: hdr.frame,
            revision: hdr.revision,
            shader_sha256: hdr.shader_hash,
            lighting_sha256: Some(hdr.lighting_hash),
        };
        self.capture_texture(&color, info)
    }
    fn capture_texture(
        &mut self,
        color: &wgpu::Texture,
        info: CaptureInfo,
    ) -> Result<Option<Capture>, Error> {
        self.collect()?;
        let Some(slot) = self.free_slot() else {
            return Ok(None);
        };
        let bytes_per_texel = color.format().block_copy_size(None).unwrap();
        let row = (color.width() * bytes_per_texel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("explicit image evidence readback"),
                size: u64::from(row) * u64::from(color.height()),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(color.height()),
                },
            },
            color.size(),
        );
        let mapped = Arc::new(OnceLock::new());
        let callback = mapped.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let _ = callback.set(result);
        });
        let done = Arc::new(OnceLock::new());
        let callback = done.clone();
        encoder.on_submitted_work_done(move || {
            let _ = callback.set(());
        });
        let submitted_at = Instant::now();
        self.queue.submit([encoder.finish()]);
        self.submissions += 1;
        self.record_flight(
            slot,
            Flight {
                done,
                mapped: None,
                lit: false,
                shadowed: false,
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        Ok(Some(Capture {
            buffer,
            mapped,
            width: color.width(),
            height: color.height(),
            row,
            consumed: false,
            bytes_per_texel,
            info,
        }))
    }
}
