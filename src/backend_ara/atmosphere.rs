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
/// Static medium/sun/camera-altitude parameters in the existing 432-byte ABI.
pub struct AtmosphereParameters {
    /// Storage record to declare and retire.
    pub record: StorageBuffer,
}
impl AtmosphereParameters {
    /// Validate physical settings; the LUTs are per unit extraterrestrial sun illuminance.
    pub fn new(
        gpu: &mut GpuDevice,
        medium: crate::AtmosphereMedium,
        toward_sun: crate::Vec3,
        altitude_km: f32,
    ) -> Result<Self> {
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
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&values))?,
        })
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
                    for p in pipelines {
                        gpu.retire_pipeline(p)?;
                    }
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
                output.storage.expect("owned LUT target is storage"),
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
    /// Retire native pipelines after submitting their lists.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for p in self.pipelines {
            gpu.retire_pipeline(p)?;
        }
        Ok(())
    }
}
