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
        let data = MeshData {
            vertices: &data.vertices,
            indices: &data.indices,
        };
        let sizes = data.validate(self.limits)?;
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
        let staging = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("charged virtual page staging"),
                size: sizes.0 + sizes.1,
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
        }
        staging.unmap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&staging, 0, &vertex, 0, sizes.0);
        encoder.copy_buffer_to_buffer(&staging, sizes.0, &index, 0, sizes.1);
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
                submission: self.submissions,
                frame: None,
                revision: self.revision,
                submitted_at,
            },
        );
        self.uploads.geometry += sizes.0 + sizes.1;
        self.check_fault()?;
        Ok(Some(crate::virtual_geometry::GpuPage {
            mesh: Mesh {
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
        if !scene.ready(asset) {
            return Ok(None);
        }
        let preparation_start = Instant::now();
        let matrix = camera.matrix(color.width(), color.height());
        if !matrix.is_finite() {
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
        if self.camera_shadow != Some(camera_bytes) {
            let start = Instant::now();
            self.queue.write_buffer(&self.uniform, 0, &camera_bytes);
            uploads.camera = 64;
            uploads.write_calls += 1;
            upload += start.elapsed();
        }
        if self.view_mode != view_mode {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(&(view_mode as u32).to_ne_bytes());
            let start = Instant::now();
            self.queue.write_buffer(&self.uniform, 64, &bytes);
            uploads.view_settings = 16;
            uploads.write_calls += 1;
            upload += start.elapsed();
        }
        let instances = scene.instances.as_mut().unwrap();
        self.upload_instance_edits(instances, &mut uploads, &mut packing, &mut upload);
        let mut prepared =
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
            for cluster in &selection.cut.clusters {
                let cluster = &mesh.clusters[cluster.index()];
                let crate::virtual_geometry::PageState::Resident(page) =
                    &asset.states[cluster.page.index()]
                else {
                    return Err(Error::InvalidInput("virtual cut lost its published page"));
                };
                prepared.push(PreparedDraw {
                    mesh: &page.mesh,
                    instances,
                    material: Some(&asset.materials[cluster.material as usize]),
                    indices: cluster.indices.offset..cluster.indices.offset + cluster.indices.count,
                    slots: placement as u32..placement as u32 + 1,
                    #[cfg(test)]
                    reference_ids: None,
                });
                counts.draws += 1;
                counts.triangles += u64::from(cluster.indices.count / 3);
                counts.submitted_instances += 1;
            }
        }
        let result = self.submit_prepared(
            color,
            camera_bytes,
            view_mode,
            &prepared,
            slot,
            uploads,
            counts,
            CpuFrame {
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
/// Explicit asynchronous RGBA8 evidence readback. Separate from ordinary frames and geometry pools.
pub struct Capture {
    buffer: Owned<wgpu::Buffer>,
    mapped: MapResult,
    width: u32,
    height: u32,
    row: u32,
    consumed: bool,
}
impl Capture {
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
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in view.chunks_exact(self.row as usize) {
            rgba.extend_from_slice(&row[..self.width as usize * 4]);
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
        self.collect()?;
        let Some(slot) = self.free_slot() else {
            return Ok(None);
        };
        let row = (color.width() * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
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
        }))
    }
}
