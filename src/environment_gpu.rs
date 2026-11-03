use super::*;
use crate::environment::{EnvironmentCube, EnvironmentInner, pack_level, upload_bytes};
use renderlab_asset::environment::PreparedEnvironment;

fn cube(
    device: &wgpu::Device,
    ledger: &Ledger,
    size: u32,
    levels: u32,
    format: wgpu::TextureFormat,
    bytes: u64,
) -> EnvironmentCube {
    let texture = ledger.own(
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("immutable environment cube"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = ledger.own(
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        }),
        ResourceKind::TextureView,
    );
    EnvironmentCube {
        view,
        _texture: texture,
        _allocation: ledger.allocate(AllocationKind::Environment, bytes),
    }
}

pub(super) fn dummy_cube(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ledger: &Ledger,
) -> EnvironmentCube {
    // This filterable zero cube keeps the constant/direct ABI available without optional features.
    let cube = cube(device, ledger, 1, 1, wgpu::TextureFormat::Rgba16Float, 48);
    queue.write_texture(
        cube._texture.as_image_copy(),
        &[0; 48],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        cube._texture.size(),
    );
    cube
}

impl Renderer {
    #[cfg(test)]
    pub(crate) fn test_environment_busy(
        &mut self,
        data: &PreparedEnvironment,
        color: &wgpu::Texture,
        camera: &Camera,
        mesh: &Mesh,
        instances: &mut Instances,
    ) {
        self.test_drain("environment busy setup");
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
        assert!(self.try_upload_environment(data).unwrap().is_none());
        self.set_environment(
            self.environment.selected.clone().as_ref(),
            EnvironmentParameters {
                rotation_y_radians: 0.7,
                intensity: 0.5,
            },
        )
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
        let after = self.stats();
        assert_eq!(before.uploads, after.uploads);
        assert_eq!(before.ownership, after.ownership);
        assert_eq!(before.submissions, after.submissions);
        for slot in &self.slots {
            slot.flight.as_ref().unwrap().done.set(()).unwrap();
        }
        self.collect().unwrap();
    }

    pub fn environment_supported(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    }
    pub fn environment_limits(&self) -> EnvironmentLimits {
        EnvironmentLimits {
            max_dimension: self
                .environment
                .limits
                .max_dimension
                .min(self.device.limits().max_texture_dimension_2d),
            ..self.environment.limits
        }
    }
    pub fn set_environment_limits(&mut self, limits: EnvironmentLimits) -> Result<(), Error> {
        self.check_fault()?;
        if limits.max_dimension == 0 || limits.max_map_bytes == 0 || limits.max_owned_bytes == 0 {
            return Err(Error::InvalidInput(
                "environment policy limits must be positive",
            ));
        }
        self.environment.limits = limits;
        Ok(())
    }
    /// Bounded immediate upload. Busy performs no packing, allocation or queue write.
    /// Requires FLOAT32_FILTERABLE on the caller-created device; no range-losing fallback.
    pub fn try_upload_environment(
        &mut self,
        data: &PreparedEnvironment,
    ) -> Result<Option<EnvironmentMap>, Error> {
        self.check_fault()?;
        if !self.environment_supported() {
            return Err(Error::InvalidInput(
                "mapped environments require device FLOAT32_FILTERABLE; constant lighting remains available",
            ));
        }
        let started = Instant::now();
        let bytes = upload_bytes(
            data,
            self.environment.limits,
            self.device.limits().max_texture_dimension_2d,
        )?;
        let preparation = started.elapsed();
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        if self
            .ledger
            .snapshot()
            .environment_bytes
            .live
            .checked_add(bytes)
            .is_none_or(|n| n > self.environment.limits.max_owned_bytes)
        {
            return Err(Error::InvalidInput(
                "environment live GPU byte budget exceeded, including retained generations",
            ));
        }
        let slot = self.free_slot().unwrap();
        let started = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let diffuse_bytes = data.diffuse_irradiance().texels().len() as u64 * 16;
        let diffuse = cube(
            &self.device,
            &self.ledger,
            data.diffuse_irradiance().size(),
            1,
            wgpu::TextureFormat::Rgba32Float,
            diffuse_bytes,
        );
        let levels = data.specular_radiance().len() as u32;
        let specular = cube(
            &self.device,
            &self.ledger,
            data.specular_radiance()[0].size(),
            levels,
            wgpu::TextureFormat::Rgba32Float,
            bytes - diffuse_bytes,
        );
        let mut hash = Sha256::new();
        for (target, mips) in [
            (&diffuse, std::slice::from_ref(data.diffuse_irradiance())),
            (&specular, data.specular_radiance()),
        ] {
            for (mip, level) in mips.iter().enumerate() {
                let packed = pack_level(level);
                hash.update(&packed);
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        mip_level: mip as u32,
                        ..target._texture.as_image_copy()
                    },
                    &packed,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(level.size() * 16),
                        rows_per_image: Some(level.size()),
                    },
                    wgpu::Extent3d {
                        width: level.size(),
                        height: level.size(),
                        depth_or_array_layers: 6,
                    },
                );
                self.uploads.texture_writes += 1;
                self.uploads.write_calls += 1;
            }
        }
        hash.update(data.diffuse_irradiance().size().to_ne_bytes());
        hash.update(data.specular_radiance()[0].size().to_ne_bytes());
        let map = EnvironmentMap(Arc::new(EnvironmentInner {
            renderer: self.identity.clone(),
            diffuse,
            specular,
            levels,
            yaw: data.metadata().rotation_y_radians,
            hash: hash.finalize().into(),
        }));
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.uploads.environment += bytes;
        self.submit_upload(slot);
        self.environment
            .uploading
            .push((self.submissions, map.clone()));
        self.upload_cpu = CpuFrame {
            preparation,
            poll,
            upload: started.elapsed(),
            ..Default::default()
        };
        self.check_fault()?;
        Ok(Some(map))
    }
    /// Atomic CPU selection. None restores Lighting::environment. Admitted frames publish changes.
    /// The renderer retains the selected map; parameter edits never clone or reupload its pixels.
    pub fn set_environment(
        &mut self,
        map: Option<&EnvironmentMap>,
        parameters: EnvironmentParameters,
    ) -> Result<(), Error> {
        self.check_fault()?;
        parameters.validate()?;
        if map.is_some_and(|m| !Arc::ptr_eq(&self.identity, &m.0.renderer)) {
            return Err(Error::InvalidInput(
                "environment belongs to another renderer",
            ));
        }
        let same = match (&self.environment.selected, map) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(&a.0, &b.0),
            _ => false,
        };
        if !same {
            self.environment.selected = map.cloned();
            self.environment.generation += 1;
        }
        self.environment.parameters = parameters;
        Ok(())
    }
}
