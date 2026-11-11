//! Independent AT1 LUT passes. Shadowed fog/history are deliberately separate ports.
use super::{
    buffer::StorageBuffer, cache::ShaderCache, graph::Graph, image::Texture, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
};
/// Physical sun illuminance and camera exposure for exposed sky/volume application.
#[derive(Clone, Copy)]
pub struct AtmosphereSun {
    /// World direction toward the sun, normalized when packed.
    pub toward_sun: crate::Vec3,
    /// Nonnegative, unexposed RGB illuminance outside the atmosphere.
    pub illuminance: [f32; 3],
    /// Nonnegative camera exposure; illuminance multiplication uses the original f64 clamp.
    pub exposure: f32,
}
/// What volumetric fog's history reads from the frames before (`atmosphere_uniform`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FogHistory {
    /// The previous lit frame's view-projection and eye, when its fog may be reused.
    pub previous: Option<(crate::Mat4, crate::Vec3)>,
    /// Frames the atmosphere has encoded: the froxel jitter's sequence index.
    pub frame: u32,
}
/// Static medium/sun/camera-altitude parameters in the existing 432-byte ABI.
pub struct AtmosphereParameters {
    /// Storage record to declare and retire.
    pub record: StorageBuffer,
    pub(crate) grid: Option<[u32; 2]>,
    pub(crate) viewport: Option<[u32; 2]>,
}
impl AtmosphereParameters {
    /// Validate physical settings; the LUTs are per unit extraterrestrial sun illuminance.
    pub fn new(
        gpu: &mut GpuDevice,
        medium: crate::AtmosphereMedium,
        toward_sun: crate::Vec3,
        altitude_km: f32,
    ) -> Result<Self> {
        let values = Self::medium_values(medium, toward_sun, altitude_km)?;
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&values))?,
            grid: None,
            viewport: None,
        })
    }
    /// Camera-volume record with the original fog settings and unjittered matrices.
    /// LUTs remain per unit extraterrestrial illuminance; no exposure is applied here.
    pub fn for_camera(
        gpu: &mut GpuDevice,
        settings: crate::Atmosphere,
        toward_sun: crate::Vec3,
        camera: crate::Camera,
        viewport: [u32; 2],
        grid: [u32; 2],
    ) -> Result<Self> {
        Self::for_camera_with_sun(
            gpu,
            settings,
            AtmosphereSun {
                toward_sun,
                illuminance: [1.; 3],
                exposure: 1.,
            },
            camera,
            viewport,
            grid,
        )
    }
    /// Pack actual sun illuminance and exposure using Renderlab's original ABI.
    pub fn for_camera_with_sun(
        gpu: &mut GpuDevice,
        settings: crate::Atmosphere,
        sun: AtmosphereSun,
        camera: crate::Camera,
        viewport: [u32; 2],
        grid: [u32; 2],
    ) -> Result<Self> {
        Self::for_frame(
            gpu,
            settings,
            sun,
            camera,
            viewport,
            grid,
            FogHistory::default(),
        )
    }
    /// [`for_camera_with_sun`](Self::for_camera_with_sun) with fog's history: the
    /// previous camera (valid flag set) and the frame index, and the hardware sun
    /// visibility source, as the wgpu renderer's hardware path packs them.
    pub fn for_frame(
        gpu: &mut GpuDevice,
        settings: crate::Atmosphere,
        sun: AtmosphereSun,
        camera: crate::Camera,
        viewport: [u32; 2],
        grid: [u32; 2],
        history: FogHistory,
    ) -> Result<Self> {
        ensure!(
            sun.illuminance
                .into_iter()
                .chain([sun.exposure])
                .all(|v| v.is_finite() && v >= 0.),
            "invalid atmosphere illuminance/exposure"
        );
        settings.validate()?;
        ensure!(
            viewport.into_iter().chain(grid).all(|n| n > 0),
            "empty atmosphere viewport/grid"
        );
        let eye = camera.view().inverse().w_axis.truncate();
        let altitude = (eye.y - settings.medium.ground_height) * 1e-3;
        let mut values = Self::medium_values(settings.medium, sun.toward_sun, altitude)?;
        values[28..31].copy_from_slice(
            &sun.illuminance
                .map(|v| (f64::from(v) * f64::from(sun.exposure)).min(f64::from(f32::MAX)) as f32),
        );
        values[31] = sun.exposure;
        values[32..35].copy_from_slice(&eye.to_array());
        let flags = u32::from(settings.sky)
            | (u32::from(settings.sky && settings.sun_disk) * 2)
            | (u32::from(settings.aerial_perspective) * 4)
            | (u32::from(settings.height_fog.is_some()) * 8)
            | (u32::from(settings.volumetric_fog.is_some()) * 16);
        values[27] = f32::from_bits(flags);
        let fog = settings.height_fog.unwrap_or(crate::HeightFog {
            density: 0.,
            ..Default::default()
        });
        values[36..40].copy_from_slice(&[
            fog.density,
            fog.falloff,
            fog.base_height,
            fog.start_distance,
        ]);
        values[40..43].copy_from_slice(&fog.albedo);
        values[43] = fog.anisotropy;
        let volume = settings.volumetric_fog.unwrap_or_default();
        values[44..48].copy_from_slice(&[
            volume.distance,
            volume.extra_density,
            volume.history_weight,
            2.,
        ]);
        let matrix = camera.matrix(viewport[0], viewport[1]);
        let (previous, previous_eye, valid) = match history.previous {
            Some((matrix, eye)) => (matrix, eye, 1.),
            None => (matrix, eye, 0.),
        };
        values[48..51].copy_from_slice(&previous_eye.to_array());
        values[51] = valid;
        values[52..68].copy_from_slice(&matrix.to_cols_array());
        values[68..84].copy_from_slice(&matrix.inverse().to_cols_array());
        values[84..100].copy_from_slice(&previous.to_cols_array());
        values[100] = f32::from_bits(grid[0]);
        values[101] = f32::from_bits(grid[1]);
        values[102] = f32::from_bits(history.frame);
        // The sun-visibility source: the scene's rays on a hardware renderer.
        values[103] = f32::from_bits(1);
        values[104] = viewport[0] as f32;
        values[105] = viewport[1] as f32;
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&values))?,
            grid: Some(grid),
            viewport: Some(viewport),
        })
    }
    /// Retire the record and its scheduled graph state after the last submission.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Buffer(self.record.buffer));
        self.record.retire(gpu)
    }
    fn medium_values(
        medium: crate::AtmosphereMedium,
        toward_sun: crate::Vec3,
        altitude_km: f32,
    ) -> Result<[f32; 108]> {
        crate::Atmosphere {
            medium,
            ..Default::default()
        }
        .validate()?;
        ensure!(
            toward_sun.is_finite()
                && toward_sun.length_squared() > 1e-12
                && altitude_km.is_finite(),
            "invalid atmosphere sun/altitude"
        );
        let sun = toward_sun.normalize();
        let mut values = [0f32; 108];
        let mut row = |i: usize, rgb: [f32; 3], w: f32| {
            values[i * 4..i * 4 + 3].copy_from_slice(&rgb);
            values[i * 4 + 3] = w;
        };
        row(
            0,
            medium.rayleigh_scattering,
            1. / medium.rayleigh_scale_height_km,
        );
        row(1, medium.mie_scattering, 1. / medium.mie_scale_height_km);
        row(2, medium.mie_absorption, medium.mie_g);
        row(3, medium.ozone_absorption, medium.ozone_peak_km);
        row(
            4,
            [
                medium.ground_radius_km,
                medium.top_radius_km,
                medium.ground_height,
            ],
            medium.ozone_half_width_km,
        );
        row(5, medium.ground_albedo, medium.sun_angular_radius);
        row(6, sun.to_array(), 0.);
        row(7, [1.; 3], 1.);
        row(
            8,
            [0.; 3],
            altitude_km.clamp(1e-3, medium.top_radius_km - medium.ground_radius_km - 1e-3),
        );
        Ok(values)
    }
}
/// Resource set shared by the four dependent LUT nodes.
pub struct AtmosphereTargets {
    /// 256×65: transmittance plus the ambient row.
    pub transmittance: Texture,
    /// 32×32 multiple-scattering transfer.
    pub multiscatter: Texture,
    /// Camera sky-view table; original default is 192×108.
    pub sky: Texture,
}
impl AtmosphereTargets {
    /// Allocate persistent LUTs. Initialize transmittance bytes once before the first
    /// chain if reading its unused ambient-row texels (only texel 0 is written there).
    pub fn new(gpu: &mut GpuDevice) -> Result<Self> {
        Ok(Self {
            transmittance: Texture::new(gpu, [256, 65], Format::Rgba16Float, true, false)?,
            multiscatter: Texture::new(gpu, [32, 32], Format::Rgba16Float, true, false)?,
            sky: Texture::new(gpu, [192, 108], Format::Rgba16Float, true, false)?,
        })
    }
}
/// Persistent transmittance, multiscatter, sky and ambient pipelines.
pub struct AtmospherePasses {
    pipelines: [Pipeline; 4],
    threads: [[u32; 3]; 4],
    sampler: u32,
}
impl AtmospherePasses {
    /// Compile only LUT entries; no stubbed sun-visibility or history path is included.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let stages = [
            Pass::AtmosphereTransmittance,
            Pass::AtmosphereMultiscatter,
            Pass::AtmosphereSky,
            Pass::AtmosphereAmbient,
        ];
        let threads = stages
            .map(Pass::threads)
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("four LUT entries");
        let mut pipelines = Vec::new();
        for stage in stages {
            match cache.pipeline(gpu, stage, RenderState::default()) {
                Ok(p) => pipelines.push(p),
                Err(e) => {
                    return Err(e);
                }
            }
        }
        let sampler = gpu
            .sampler(&ara_gpu::sampler::SamplerDesc::default())?
            .index;
        Ok(Self {
            pipelines: pipelines.try_into().expect("four LUT pipelines"),
            threads,
            sampler,
        })
    }
    /// Record one LUT with its producer/consumer edges, on the graphics timeline.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        stage: Pass,
        params: &AtmosphereParameters,
        targets: &AtmosphereTargets,
    ) -> Result<()> {
        let (i, output, size) = match stage {
            Pass::AtmosphereTransmittance => (0, &targets.transmittance, [256, 64]),
            Pass::AtmosphereMultiscatter => (1, &targets.multiscatter, [32, 32]),
            Pass::AtmosphereSky => (2, &targets.sky, targets.sky.size),
            Pass::AtmosphereAmbient => (3, &targets.transmittance, [1, 1]),
            _ => anyhow::bail!("not an atmosphere LUT stage"),
        };
        for target in [&targets.transmittance, &targets.multiscatter, &targets.sky] {
            ensure!(
                target.format == Format::Rgba16Float
                    && target.shape.dimension == ara_gpu::texture::TextureDimension::D2
                    && target.shape.layers == 1
                    && target.shape.samples == ara_gpu::texture::SampleCount::One
                    && target.shape.mip_levels == 1,
                "LUTs require single-level2D RGBA16Float views"
            );
        }
        ensure!(
            targets.transmittance.size == [256, 65] && targets.multiscatter.size == [32, 32],
            "LUT extent does not match the original shader ABI"
        );
        let storage = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("LUT output lacks a storage view"))?;
        let mut uses = vec![
            (Resource::Buffer(params.record.buffer), Access::ShaderRead),
            (Resource::Image(output.image), Access::Storage),
        ];
        if i == 1 || i == 2 {
            uses.push((
                Resource::Image(targets.transmittance.image),
                Access::ShaderRead,
            ));
        }
        if i == 2 {
            uses.push((
                Resource::Image(targets.multiscatter.image),
                Access::ShaderRead,
            ));
        }
        if i == 3 {
            uses.push((Resource::Image(targets.sky.image), Access::ShaderRead));
        }
        graph.node(rec, &uses)?;
        rec.push(Command::BeginPass(format!("ara::{stage:?}")));
        rec.push(Command::Pipeline(self.pipelines[i]));
        rec.push(Command::Constants(RootConstants {
            words: [
                params.record.slot,
                targets.transmittance.sampled,
                targets.multiscatter.sampled,
                targets.sky.sampled,
                storage,
                self.sampler,
                0,
                0,
            ],
        }));
        let groups = if i == 1 || i == 3 {
            [size[0], size[1], 1]
        } else {
            [
                size[0].div_ceil(self.threads[i][0]),
                size[1].div_ceil(self.threads[i][1]),
                1,
            ]
        };
        rec.push(Command::Dispatch(groups));
        rec.push(Command::EndPass);
        Ok(())
    }
}
