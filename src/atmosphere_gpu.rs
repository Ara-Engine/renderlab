//! AT1's GPU state (docs/m7-at1.md): the transmittance, multiple-scattering and sky-view
//! LUTs, the camera volume (froxel fog slices, then aerial perspective slices), their
//! kernels and the raster sky. Lit frames with any atmosphere part on prepare it before the
//! lighting group binds it and encode its kernels before shading; every other frame skips
//! it entirely. The shading side is shaders/atmosphere_api.wgsl.
// PC1: only the device type changes; pipeline creation uses the shared cache.
use super::*;
use crate::atmosphere::{Atmosphere, AtmosphereMedium};
use crate::pipeline_compilation::PipelineDevice;

const COMMON: &str = include_str!("../shaders/atmosphere_common.wgsl");
const KERNELS: &str = include_str!("../shaders/atmosphere.wgsl");
const SKY: &str = include_str!("../shaders/atmosphere_sky.wgsl");
// Matches shaders/atmosphere_common.wgsl.
const TRANSMITTANCE: [u32; 2] = [256, 64];
const MULTISCATTER: u32 = 32;
const SKYVIEW: [u32; 2] = [192, 108];
const FROXEL_SLICES: u32 = 64;
const VOLUME_SLICES: u32 = 96;
/// Screen pixels per camera-volume column.
pub(super) const FROXEL_TILE: u32 = 8;
const FROXEL_SHADOW_SAMPLES: u32 = 2;
pub(super) const UNIFORM_BYTES: u64 = 432;
/// The procedural environment's specular chain (shaders/atmosphere.wgsl must match): the
/// smallest level stays 8 texels (7 levels from 64 down to 1 texel cost 0.47 ms more quarry
/// shading than a captured map; rough lookups there are mostly cross-face corner taps).
/// Roughness steps 0.25 between levels.
const ENVIRONMENT_SIZE: u32 = 128;
const ENVIRONMENT_LEVELS: u32 = 5;
const FLAG_SKY: u32 = 1;
const FLAG_SUN_DISK: u32 = 2;
const FLAG_AERIAL: u32 = 4;
const FLAG_HEIGHT_FOG: u32 = 8;
const FLAG_VOLUMETRIC: u32 = 16;

/// Hardware renderers shadow the froxels' sun with one ray each against the scene TLAS.
const HARDWARE_VISIBILITY: &str = "@group(0) @binding(20) var at_scene:acceleration_structure;
fn at_sun_visible(p:vec3<f32>) -> f32 {
    var query:ray_query;
    rayQueryInitialize(&query,at_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,0.02,1e5,p,atmosphere.sun.xyz));
    while rayQueryProceed(&query) {}
    return select(0.0,1.0,rayQueryGetCommittedIntersection(&query).kind==0u);
}
";
const MAP_VISIBILITY: &str = include_str!("../shaders/atmosphere_shadow_map.wgsl");

#[derive(PartialEq)]
struct VisibilityKey {
    virtual_map: bool,
    generation: u64,
    buffers: Vec<wgpu::Buffer>,
}

#[derive(PartialEq)]
struct HistoryKey {
    sun: Option<(Vec3, [f32; 3])>,
    exposure: f32,
    environment: u64,
    shadows: Shadows,
    primary: PrimaryVisibility,
    soft: SoftShadows,
}

pub(super) struct Lut {
    _texture: Owned<wgpu::Texture>,
    pub view: Owned<wgpu::TextureView>,
    _allocation: Allocation,
}
fn lut(device: &PipelineDevice, ledger: &Ledger, label: &str, size: [u32; 3], d3: bool) -> Lut {
    let texture = ledger.own(
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: size[2],
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: if d3 {
                wgpu::TextureDimension::D3
            } else {
                wgpu::TextureDimension::D2
            },
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }),
        ResourceKind::Texture,
    );
    let view = ledger.own(
        texture.create_view(&Default::default()),
        ResourceKind::TextureView,
    );
    Lut {
        _texture: texture,
        view,
        _allocation: ledger.allocate(
            AllocationKind::Lut,
            u64::from(size[0]) * u64::from(size[1]) * u64::from(size[2]) * 8,
        ),
    }
}

/// The camera volume, raw samples and alternating visibility/extinction histories.
pub(super) struct Volume {
    grid: [u32; 2],
    pub camera: Lut,
    scatter: Option<[Lut; 2]>,
    raw: Option<Lut>,
    aerial: Option<Owned<wgpu::BindGroup>>,
    integrate: Option<[Owned<wgpu::BindGroup>; 2]>,
    temporal: Option<[Owned<wgpu::BindGroup>; 2]>,
    /// Injection groups, keyed by their read-only shadow resources.
    inject: Option<([Owned<wgpu::BindGroup>; 2], VisibilityKey)>,
}
impl Volume {
    fn new(device: &PipelineDevice, ledger: &Ledger, grid: [u32; 2], volumetric: bool) -> Self {
        Self {
            grid,
            camera: lut(
                device,
                ledger,
                "AT1 camera volume",
                [grid[0], grid[1], VOLUME_SLICES],
                true,
            ),
            scatter: volumetric.then(|| {
                [0, 1].map(|_| {
                    lut(
                        device,
                        ledger,
                        "VF2 visibility/extinction history",
                        [grid[0], grid[1], FROXEL_SLICES],
                        true,
                    )
                })
            }),
            raw: volumetric.then(|| {
                lut(
                    device,
                    ledger,
                    "VF2 raw fog samples",
                    [grid[0], grid[1], FROXEL_SLICES],
                    true,
                )
            }),
            aerial: None,
            integrate: None,
            temporal: None,
            inject: None,
        }
    }
}

struct Kernels {
    transmittance: Owned<wgpu::ComputePipeline>,
    multiscatter: Owned<wgpu::ComputePipeline>,
    skyview: Owned<wgpu::ComputePipeline>,
    ambient: Owned<wgpu::ComputePipeline>,
    aerial: Owned<wgpu::ComputePipeline>,
    inject: Owned<wgpu::ComputePipeline>,
    inject_virtual: Option<Owned<wgpu::ComputePipeline>>,
    temporal: Owned<wgpu::ComputePipeline>,
    integrate: Owned<wgpu::ComputePipeline>,
    specular: Owned<wgpu::ComputePipeline>,
    diffuse: Owned<wgpu::ComputePipeline>,
    /// Groups over the fixed LUTs: transmittance, multiscatter, skyview, ambient.
    fixed: [Owned<wgpu::BindGroup>; 4],
}

/// The procedural environment (`Atmosphere::environment`): the inputs it was generated
/// from, the map, its kernel groups (the specular levels, then diffuse), and whether a
/// frame has written it yet.
struct Procedural {
    key: ProceduralKey,
    map: crate::environment::EnvironmentMap,
    groups: Vec<Owned<wgpu::BindGroup>>,
    written: bool,
}

#[derive(Debug, PartialEq)]
struct ProceduralKey {
    sun: Option<(Vec3, [f32; 3])>,
    medium: AtmosphereMedium,
    altitude: f32,
}

pub(super) struct AtmosphereGpu {
    pub settings: Atmosphere,
    uniform: Owned<wgpu::Buffer>,
    _uniform_allocation: Allocation,
    sampler: Owned<wgpu::Sampler>,
    pub transmittance: Lut,
    multiscatter: Lut,
    pub skyview: Lut,
    pub volume: Volume,
    retired: Vec<(u64, Volume)>,
    kernels: Option<Kernels>,
    procedural: Option<Procedural>,
    sky: Option<Owned<wgpu::RenderPipeline>>,
    /// The medium the transmittance and multiple-scattering LUTs hold.
    luts: Option<AtmosphereMedium>,
    published: Option<Vec<u8>>,
    /// The last encoded frame's view-projection and eye, for froxel reprojection.
    previous: Option<(Mat4, Vec3)>,
    history_key: Option<HistoryKey>,
    frame: u32,
    #[cfg(test)]
    test_fog_quality: Option<(u32, u32)>,
    /// Whether this frame encodes the kernels (set by `prepare_atmosphere`).
    active: bool,
    /// Bumped when the camera volume is replaced, so lighting rebinds it.
    pub generation: u64,
}
impl AtmosphereGpu {
    pub fn new(device: &PipelineDevice, ledger: &Ledger) -> Self {
        let uniform = ledger.own(
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("AT1 atmosphere uniform"),
                size: UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let sampler = ledger.own(
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("AT1 LUT bilinear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        Self {
            // Lib tests predate AT1 and score shading against atmosphere-free references;
            // atmosphere tests opt in with set_atmosphere. Every other build gets the default.
            settings: if cfg!(test) {
                Atmosphere::off()
            } else {
                Atmosphere::default()
            },
            uniform,
            _uniform_allocation: ledger.allocate(AllocationKind::Lut, UNIFORM_BYTES),
            sampler,
            transmittance: lut(
                device,
                ledger,
                "AT1 transmittance LUT (+ fog ambient row)",
                [TRANSMITTANCE[0], TRANSMITTANCE[1] + 1, 1],
                false,
            ),
            multiscatter: lut(
                device,
                ledger,
                "AT1 multiple-scattering LUT",
                [MULTISCATTER, MULTISCATTER, 1],
                false,
            ),
            skyview: lut(
                device,
                ledger,
                "AT1 sky-view LUT",
                [SKYVIEW[0], SKYVIEW[1], 1],
                false,
            ),
            volume: Volume::new(device, ledger, [1, 1], false),
            retired: vec![],
            kernels: None,
            procedural: None,
            sky: None,
            luts: None,
            published: None,
            previous: None,
            history_key: None,
            frame: 0,
            #[cfg(test)]
            test_fog_quality: None,
            active: false,
            generation: 0,
        }
    }
    pub fn uniform(&self) -> &wgpu::Buffer {
        &self.uniform
    }
    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }
    /// The raster sky draw for this lit frame, when the sky is on.
    pub fn raster_sky(&self) -> Option<&wgpu::RenderPipeline> {
        self.sky
            .as_deref()
            .filter(|_| self.active && self.settings.sky)
    }
}

/// One kernel pipeline with its derived layout.
fn kernel(
    device: &PipelineDevice,
    ledger: &Ledger,
    module: &wgpu::ShaderModule,
    entry: &str,
) -> Owned<wgpu::ComputePipeline> {
    ledger.own(
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        }),
        ResourceKind::ComputePipeline,
    )
}
fn group(
    device: &PipelineDevice,
    ledger: &Ledger,
    pipeline: &wgpu::ComputePipeline,
    entries: &[wgpu::BindGroupEntry<'_>],
) -> Owned<wgpu::BindGroup> {
    ledger.own(
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("AT1 kernel"),
            layout: &pipeline.get_bind_group_layout(0),
            entries,
        }),
        ResourceKind::BindGroup,
    )
}
/// An environment kernel's group: the uniform, the sampler, the sky view and its target level.
fn environment_entries<'a>(
    a: &'a AtmosphereGpu,
    binding: u32,
    target: &'a wgpu::TextureView,
) -> [wgpu::BindGroupEntry<'a>; 4] {
    [
        wgpu::BindGroupEntry {
            binding: 0,
            resource: a.uniform.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(&a.sampler),
        },
        view(4, &a.skyview.view),
        view(binding, target),
    ]
}
fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

impl Renderer {
    pub(crate) fn atmosphere_shadow_source() -> String {
        virtual_shadow_gpu::atmosphere_shadow::atmosphere_source()
    }
    /// Replaces every atmosphere setting at once; the next lit frame uses them.
    pub fn set_atmosphere(&mut self, atmosphere: Atmosphere) -> Result<(), Error> {
        self.check_fault()?;
        atmosphere.validate()?;
        if self.atmosphere.settings != atmosphere {
            self.atmosphere.previous = None;
            // TR1 otherwise keeps pixels integrated under the old fog density or
            // medium even after the froxels have restarted.
            self.cut_reconstruction();
        }
        self.atmosphere.settings = atmosphere;
        Ok(())
    }
    pub fn atmosphere(&self) -> Atmosphere {
        self.atmosphere.settings
    }
    /// Restarts volumetric fog history and its sample sequence on the next lit frame.
    /// Call alongside reconstruction/GI resets after a camera cut or teleport.
    pub fn reset_atmosphere_history(&mut self) {
        self.atmosphere.previous = None;
        self.atmosphere.frame = 0;
    }
    #[cfg(test)]
    pub(crate) fn test_fog_quality(&mut self, tile: u32, samples: u32) {
        assert!([8, 16].contains(&tile) && (1..=2).contains(&samples));
        self.atmosphere.test_fog_quality = Some((tile, samples));
        self.atmosphere.previous = None;
        self.cut_reconstruction();
    }
    #[cfg(test)]
    pub(crate) fn test_atmosphere_environment(&self) -> (u64, u64, usize) {
        (
            self.environment.generation,
            self.environment.content_revision,
            self.environment
                .selected
                .as_ref()
                .map_or(0, |m| Arc::as_ptr(&m.0) as usize),
        )
    }
    /// Whether primary misses show the per-pixel sky rather than one clear colour; TR1
    /// reconstructs them as a visible sky then.
    pub(super) fn atmosphere_sky_visible(&self) -> bool {
        self.atmosphere.settings.sky
    }
    /// The first directional light: its unit direction toward the sun and its lux.
    fn atmosphere_sun(&self) -> Option<(Vec3, [f32; 3])> {
        self.lighting
            .source_lights
            .iter()
            .find_map(|light| match *light {
                Light::Directional { travel, lux } => Some((-travel.normalize(), lux)),
                _ => None,
            })
    }
    /// The factor directional lux takes this frame (`Atmosphere::sun_transmittance`), or
    /// None to upload the lights as given.
    pub(super) fn sun_scale(&self, eye: Vec3) -> Option<[f32; 3]> {
        let s = &self.atmosphere.settings;
        if !s.sun_transmittance || !s.active() || self.captured_environment() {
            return None;
        }
        let (sun, _) = self.atmosphere_sun()?;
        Some(s.transmittance(eye.y - s.medium.ground_height, sun))
    }

    /// Whether the selected environment is a captured map (not the procedural one).
    pub(super) fn captured_environment(&self) -> bool {
        match (&self.environment.selected, &self.atmosphere.procedural) {
            (None, _) => false,
            (Some(a), Some(p)) => !Arc::ptr_eq(&a.0, &p.map.0),
            (Some(_), None) => true,
        }
    }

    /// Rewrites a procedural sky in place when its lighting changes. Only a storage/yaw
    /// change replaces the map; content revisions use GI's existing progressive relighting.
    fn update_procedural_environment(&mut self, eye: Vec3) {
        let s = self.atmosphere.settings;
        let captured = self.captured_environment();
        let mine = self.environment.selected.is_some() && !captured;
        if !s.environment || !s.active() || captured {
            if mine {
                self.environment.selected = None;
                self.environment.generation += 1;
            }
            self.atmosphere.procedural = None;
            return;
        }
        let key = ProceduralKey {
            sun: self.atmosphere_sun(),
            medium: s.medium,
            altitude: ((eye.y - s.medium.ground_height) * 1e-3).clamp(
                1e-3,
                s.medium.top_radius_km - s.medium.ground_radius_km - 1e-3,
            ),
        };
        if let Some(p) = self
            .atmosphere
            .procedural
            .as_mut()
            .filter(|p| mine && p.map.0.yaw == -self.environment.parameters.rotation_y_radians)
        {
            if p.key != key {
                p.key = key;
                p.written = false;
                self.environment.content_revision += 1;
            }
            return;
        }
        let (device, ledger) = (&self.device, &self.ledger);
        let cube = |label, size: u32, levels: u32| {
            let texture = ledger.own(
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: levels,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = ledger.own(
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                }),
                ResourceKind::TextureView,
            );
            let bytes = (0..levels)
                .map(|l| u64::from((size >> l).max(1)).pow(2) * 6 * 16)
                .sum();
            let levels: Vec<_> = (0..levels)
                .map(|l| {
                    ledger.own(
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            base_mip_level: l,
                            mip_level_count: Some(1),
                            ..Default::default()
                        }),
                        ResourceKind::TextureView,
                    )
                })
                .collect();
            (
                crate::environment::EnvironmentCube {
                    view,
                    _texture: texture,
                    _allocation: ledger.allocate(AllocationKind::Environment, bytes),
                },
                levels,
            )
        };
        let (specular, specular_levels) = cube(
            "AT1 procedural specular",
            ENVIRONMENT_SIZE,
            ENVIRONMENT_LEVELS,
        );
        let (diffuse, diffuse_levels) = cube("AT1 procedural diffuse", 16, 1);
        let a = &self.atmosphere;
        let k = a.kernels.as_ref().unwrap();
        let mut groups: Vec<_> = specular_levels
            .iter()
            .map(|v| group(device, ledger, &k.specular, &environment_entries(a, 16, v)))
            .collect();
        groups.push(group(
            device,
            ledger,
            &k.diffuse,
            &environment_entries(a, 17, &diffuse_levels[0]),
        ));
        let map =
            crate::environment::EnvironmentMap(Arc::new(crate::environment::EnvironmentInner {
                renderer: self.identity.clone(),
                diffuse,
                specular,
                specular_second: None,
                lobes: None,
                independent: false,
                levels: ENVIRONMENT_LEVELS,
                // World axes: cancel the caller's yaw.
                yaw: -self.environment.parameters.rotation_y_radians,
                hash: Sha256::digest(format!("{key:?}").as_bytes()).into(),
            }));
        self.environment.selected = Some(map.clone());
        self.environment.generation += 1;
        self.atmosphere.procedural = Some(Procedural {
            key,
            map,
            groups,
            written: false,
        });
    }

    /// Lit frames, before lighting binds: sizes the camera volume to the target, creates the
    /// kernels on first use and publishes this frame's uniform. Frames with every part off
    /// do nothing here or in `encode_atmosphere`.
    pub(super) fn prepare_atmosphere(
        &mut self,
        camera: &Camera,
        size: [u32; 2],
        uploads: &mut UploadBytes,
    ) -> Result<(), Error> {
        let s = self.atmosphere.settings;
        self.atmosphere.active = s.active();
        let eye = camera.view().inverse().w_axis.truncate();
        if !self.atmosphere.active {
            self.update_procedural_environment(eye);
            self.atmosphere.previous = None;
            return Ok(());
        }
        self.ensure_atmosphere_kernels()?;
        let tile = FROXEL_TILE;
        #[cfg(test)]
        let tile = self.atmosphere.test_fog_quality.map_or(tile, |q| q.0);
        let grid = size.map(|n| n.div_ceil(tile));
        let volumetric = s.volumetric_fog.is_some();
        if self.atmosphere.volume.grid != grid
            || self.atmosphere.volume.scatter.is_some() != volumetric
        {
            let volume = Volume::new(&self.device, &self.ledger, grid, volumetric);
            let old = std::mem::replace(&mut self.atmosphere.volume, volume);
            self.atmosphere.retired.push((self.submissions, old));
            self.atmosphere.generation += 1;
            self.atmosphere.previous = None;
        }
        self.atmosphere_volume_groups(false, 0);
        self.update_procedural_environment(eye);
        if uploads.geometry > 0
            || uploads.models > 0
            || uploads.instances > 0
            || uploads.skinning > 0
        {
            self.atmosphere.previous = None;
        }
        // Conservatively invalidate visibility reuse for shadow, medium and lighting
        // changes. Camera motion uses world reprojection; phase is always current.
        let key = HistoryKey {
            sun: self.atmosphere_sun(),
            exposure: self.lighting.pending.exposure,
            environment: self.environment.generation,
            shadows: self.shadows.settings.clone(),
            primary: self.primary_mode,
            soft: self.soft_mode,
        };
        if self.atmosphere.history_key.as_ref() != Some(&key) {
            self.atmosphere.previous = None;
            self.atmosphere.history_key = Some(key);
        }
        let bytes = self.atmosphere_uniform(camera, size);
        if self.atmosphere.published.as_deref() != Some(&bytes[..]) {
            self.queue.write_buffer(&self.atmosphere.uniform, 0, &bytes);
            uploads.lighting += bytes.len() as u64;
            uploads.lighting_writes += 1;
            uploads.write_calls += 1;
            self.atmosphere.published = Some(bytes);
        }
        Ok(())
    }

    fn atmosphere_uniform(&self, camera: &Camera, size: [u32; 2]) -> Vec<u8> {
        let a = &self.atmosphere;
        let s = &a.settings;
        let m = &s.medium;
        let (sun, lux) = self.atmosphere_sun().unwrap_or((Vec3::Y, [0.0; 3]));
        let exposure = f64::from(self.lighting.pending.exposure);
        let illuminance = lux.map(|l| (f64::from(l) * exposure).min(f64::from(f32::MAX)) as f32);
        let eye = camera.view().inverse().w_axis.truncate();
        let altitude = ((eye.y - m.ground_height) * 1e-3)
            .clamp(1e-3, m.top_radius_km - m.ground_radius_km - 1e-3);
        let flags = u32::from(s.sky) * FLAG_SKY
            | u32::from(s.sky && s.sun_disk) * FLAG_SUN_DISK
            | u32::from(s.aerial_perspective) * FLAG_AERIAL
            | u32::from(s.height_fog.is_some()) * FLAG_HEIGHT_FOG
            | u32::from(s.volumetric_fog.is_some()) * FLAG_VOLUMETRIC;
        let fog = s.height_fog.unwrap_or(crate::atmosphere::HeightFog {
            density: 0.0,
            ..Default::default()
        });
        let volume = s.volumetric_fog.unwrap_or_default();
        let samples = FROXEL_SHADOW_SAMPLES;
        #[cfg(test)]
        let samples = a.test_fog_quality.map_or(samples, |q| q.1);
        let matrix = camera.matrix(size[0], size[1]);
        let (previous, previous_eye, valid) = match a.previous {
            Some((m, e)) => (m, e, 1.0),
            None => (matrix, eye, 0.0),
        };
        let mut f: Vec<f32> = Vec::with_capacity(108);
        f.extend(m.rayleigh_scattering);
        f.push(1.0 / m.rayleigh_scale_height_km);
        f.extend(m.mie_scattering);
        f.push(1.0 / m.mie_scale_height_km);
        f.extend(m.mie_absorption);
        f.push(m.mie_g);
        f.extend(m.ozone_absorption);
        f.push(m.ozone_peak_km);
        f.extend([
            m.ground_radius_km,
            m.top_radius_km,
            m.ground_height,
            m.ozone_half_width_km,
        ]);
        f.extend(m.ground_albedo);
        f.push(m.sun_angular_radius);
        f.extend(sun.to_array());
        f.push(f32::from_bits(flags));
        f.extend(illuminance);
        f.push(exposure as f32);
        f.extend(eye.to_array());
        f.push(altitude);
        f.extend([
            fog.density,
            fog.falloff,
            fog.base_height,
            fog.start_distance,
        ]);
        f.extend(fog.albedo);
        f.push(fog.anisotropy);
        f.extend([
            volume.distance,
            volume.extra_density,
            volume.history_weight,
            samples as f32,
        ]);
        f.extend(previous_eye.to_array());
        f.push(valid);
        f.extend(matrix.to_cols_array());
        f.extend(matrix.inverse().to_cols_array());
        f.extend(previous.to_cols_array());
        let mut bytes: Vec<u8> = f.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let source = u32::from(self.hardware_shadows.is_some());
        for v in [a.volume.grid[0], a.volume.grid[1], a.frame, source] {
            bytes.extend(v.to_ne_bytes());
        }
        let range = self
            .lighting
            .source_lights
            .iter()
            .position(|l| matches!(l, Light::Directional { .. }))
            .and_then(|i| self.lighting.pending.records.chunks_exact(64).nth(i))
            .map_or([0, 0], |r| {
                [52, 56].map(|i| u32::from_ne_bytes(r[i..i + 4].try_into().unwrap()))
            });
        for v in [
            size[0] as f32,
            size[1] as f32,
            range[0] as f32,
            range[1] as f32,
        ] {
            bytes.extend(v.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len() as u64, UNIFORM_BYTES);
        bytes
    }

    fn ensure_atmosphere_kernels(&mut self) -> Result<(), Error> {
        if self.atmosphere.kernels.is_some() {
            return Ok(());
        }
        let hardware = self.hardware_shadows.is_some();
        let source = format!(
            "{}{COMMON}{KERNELS}{}",
            if hardware {
                "enable wgpu_ray_query;\n"
            } else {
                ""
            },
            if hardware {
                HARDWARE_VISIBILITY
            } else {
                MAP_VISIBILITY
            }
        );
        let scopes = OperationScopes::new(&self.device);
        let (device, ledger) = (&self.device, &self.ledger);
        let module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("AT1 atmosphere kernels"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            }),
            ResourceKind::ShaderModule,
        );
        let k = |entry| kernel(device, ledger, &module, entry);
        let (transmittance, multiscatter, skyview, ambient, aerial, inject, integrate) = (
            k("at_transmittance_lut"),
            k("at_multiscatter_lut"),
            k("at_skyview_lut"),
            k("at_ambient_mean"),
            k("at_aerial_volume"),
            k("at_fog_inject"),
            k("at_fog_integrate"),
        );
        let (specular, diffuse) = (k("at_environment_specular"), k("at_environment_diffuse"));
        let temporal = k("at_fog_temporal");
        let inject_virtual = (!hardware).then(|| {
            let module = device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("VF1 Virtual fog sampling"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{COMMON}{KERNELS}{}", Self::atmosphere_shadow_source()).into(),
                ),
            });
            kernel(device, ledger, &module, "at_fog_inject")
        });
        let a = &self.atmosphere;
        let uniform = wgpu::BindGroupEntry {
            binding: 0,
            resource: a.uniform.as_entire_binding(),
        };
        let sampler = wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(&a.sampler),
        };
        let fixed = [
            group(
                device,
                ledger,
                &transmittance,
                &[uniform.clone(), view(10, &a.transmittance.view)],
            ),
            group(
                device,
                ledger,
                &multiscatter,
                &[
                    uniform.clone(),
                    view(1, &a.transmittance.view),
                    sampler.clone(),
                    view(11, &a.multiscatter.view),
                ],
            ),
            group(
                device,
                ledger,
                &skyview,
                &[
                    uniform.clone(),
                    view(1, &a.transmittance.view),
                    view(2, &a.multiscatter.view),
                    sampler.clone(),
                    view(12, &a.skyview.view),
                ],
            ),
            group(
                device,
                ledger,
                &ambient,
                &[
                    uniform.clone(),
                    sampler.clone(),
                    view(4, &a.skyview.view),
                    view(10, &a.transmittance.view),
                ],
            ),
        ];
        let sky_module = ledger.own(
            device.create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("AT1 raster sky"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}{SKY}", crate::atmosphere::api_source()).into(),
                ),
            }),
            ResourceKind::ShaderModule,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("AT1 raster sky"),
                bind_group_layouts: &[None, None, None, Some(&self.lighting.bindings)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        let sky = ledger.own(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("AT1 raster sky"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &sky_module,
                    entry_point: Some("vs_sky"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &sky_module,
                    entry_point: Some("fs_sky"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                }),
                primitive: Default::default(),
                // Drawn first in the pass, on every sample, without depth: the surfaces that
                // follow cover it through their own depth test.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: self.options.samples as u32,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            }),
            ResourceKind::RenderPipeline,
        );
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.atmosphere.kernels = Some(Kernels {
            transmittance,
            multiscatter,
            skyview,
            ambient,
            aerial,
            inject,
            inject_virtual,
            temporal,
            integrate,
            specular,
            diffuse,
            fixed,
        });
        self.atmosphere.sky = Some(sky);
        Ok(())
    }

    /// Creates the camera volume's kernel groups that are missing (the injection's again
    /// when the scene TLAS changed). `scene` is the frame's TLAS generation
    /// (`tlas_builds + hardware_build`), the key the other TLAS consumers use.
    fn atmosphere_volume_groups(&mut self, injection: bool, scene: u64) {
        let (device, ledger) = (&self.device, &self.ledger);
        let a = &mut self.atmosphere;
        let k = a.kernels.as_ref().unwrap();
        let v = &mut a.volume;
        let uniform = wgpu::BindGroupEntry {
            binding: 0,
            resource: a.uniform.as_entire_binding(),
        };
        let sampler = wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(&a.sampler),
        };
        if v.aerial.is_none() {
            v.aerial = Some(group(
                device,
                ledger,
                &k.aerial,
                &[
                    uniform.clone(),
                    view(1, &a.transmittance.view),
                    view(2, &a.multiscatter.view),
                    sampler.clone(),
                    view(13, &v.camera.view),
                ],
            ));
        }
        let Some(scatter) = &v.scatter else {
            return;
        };
        let raw = v.raw.as_ref().unwrap();
        if v.temporal.is_none() {
            v.temporal = Some([0, 1].map(|i| {
                group(
                    device,
                    ledger,
                    &k.temporal,
                    &[
                        uniform.clone(),
                        sampler.clone(),
                        view(5, &scatter[1 - i].view),
                        view(6, &raw.view),
                        view(15, &scatter[i].view),
                    ],
                )
            }));
        }
        // Aerial perspective alone needs no shadow resources.
        let tlas = self.hardware_shadows.as_ref().map(|h| h.scene());
        let virtual_buffers = (self.hardware_shadows.is_none()
            && self.primary_mode == PrimaryVisibility::VisibilityBuffer
            && self.soft_mode == SoftShadows::Virtual
            && self.shadows.settings.enabled()
            && self.shadows.settings.directional_disk_angle > 0.0)
            .then(|| self.virtual_shadows.as_ref()?.atmosphere_buffers())
            .flatten();
        let key = VisibilityKey {
            virtual_map: virtual_buffers.is_some(),
            // API1b fix: the TLAS's address is not its identity. A new scene generation
            // replaces it in place (`HardwareState::active`), so an address key kept binding
            // the previous TLAS: stale fog shadows, and a validation fault once a skinned
            // BLAS was refit after that TLAS's build. Key on the scene generation instead.
            generation: tlas.map_or(self.shadows.stats.resource_generation, |_| scene),
            buffers: virtual_buffers.map_or_else(Vec::new, |b| b.map(Clone::clone).to_vec()),
        };
        if v.integrate.is_none() {
            v.integrate = Some([0, 1].map(|i| {
                group(
                    device,
                    ledger,
                    &k.integrate,
                    &[
                        uniform.clone(),
                        view(1, &a.transmittance.view),
                        sampler.clone(),
                        view(6, &scatter[i].view),
                        view(13, &v.camera.view),
                    ],
                )
            }));
        }
        if injection && v.inject.as_ref().is_none_or(|(_, old)| *old != key) {
            let groups = [0, 1].map(|_| {
                let mut entries = vec![uniform.clone(), view(15, &raw.view)];
                if let Some(tlas) = tlas {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 20,
                        resource: wgpu::BindingResource::AccelerationStructure(tlas),
                    });
                } else if let Some(buffers) = virtual_buffers {
                    entries.extend(buffers.into_iter().enumerate().map(|(i, b)| {
                        wgpu::BindGroupEntry {
                            binding: 21 + i as u32,
                            resource: b.as_entire_binding(),
                        }
                    }));
                } else {
                    entries.push(view(21, self.shadows.sample_view()));
                    entries.push(wgpu::BindGroupEntry {
                        binding: 22,
                        resource: self.shadows.records().as_entire_binding(),
                    });
                }
                group(
                    device,
                    ledger,
                    if key.virtual_map {
                        k.inject_virtual.as_ref().unwrap()
                    } else {
                        &k.inject
                    },
                    &entries,
                )
            });
            v.inject = Some((groups, key));
        }
    }

    /// The atmosphere kernels for this frame, after the TLAS build and before shading: the
    /// fixed LUTs when the medium changed, the sky view and its mean, the aerial
    /// perspective, then volumetric fog.
    pub(super) fn encode_atmosphere(&mut self, encoder: &mut wgpu::CommandEncoder, scene: u64) {
        if !self.atmosphere.active {
            return;
        }
        // Virtual's resources exist after prepare_virtual, and its pages have been
        // encoded before this call. Raster depth and the TLAS are ready here too.
        self.atmosphere_volume_groups(true, scene);
        let a = &mut self.atmosphere;
        let medium = a.settings.medium;
        let stale = a.luts != Some(medium);
        a.luts = Some(medium);
        let k = a.kernels.as_ref().unwrap();
        let v = &a.volume;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("AT1 atmosphere"),
            timestamp_writes: None,
        });
        if stale {
            pass.set_pipeline(&k.transmittance);
            pass.set_bind_group(0, &*k.fixed[0], &[]);
            pass.dispatch_workgroups(TRANSMITTANCE[0] / 8, TRANSMITTANCE[1] / 8, 1);
            pass.set_pipeline(&k.multiscatter);
            pass.set_bind_group(0, &*k.fixed[1], &[]);
            pass.dispatch_workgroups(MULTISCATTER, MULTISCATTER, 1);
        }
        pass.set_pipeline(&k.skyview);
        pass.set_bind_group(0, &*k.fixed[2], &[]);
        pass.dispatch_workgroups(SKYVIEW[0].div_ceil(8), SKYVIEW[1].div_ceil(8), 1);
        pass.set_pipeline(&k.ambient);
        pass.set_bind_group(0, &*k.fixed[3], &[]);
        pass.dispatch_workgroups(1, 1, 1);
        if let Some(p) = a.procedural.as_mut().filter(|p| !p.written) {
            pass.set_pipeline(&k.specular);
            for (level, group) in p.groups[..ENVIRONMENT_LEVELS as usize].iter().enumerate() {
                pass.set_bind_group(0, &**group, &[]);
                let n = (ENVIRONMENT_SIZE >> level).div_ceil(8);
                pass.dispatch_workgroups(n, n, 6);
            }
            pass.set_pipeline(&k.diffuse);
            pass.set_bind_group(0, &*p.groups[ENVIRONMENT_LEVELS as usize], &[]);
            pass.dispatch_workgroups(2, 2, 6);
            p.written = true;
        }
        pass.set_pipeline(&k.aerial);
        pass.set_bind_group(0, &**v.aerial.as_ref().unwrap(), &[]);
        pass.dispatch_workgroups(v.grid[0].div_ceil(8), v.grid[1].div_ceil(8), 1);
        if let (Some((inject, _)), Some(integrate)) = (&v.inject, &v.integrate) {
            let i = (a.frame % 2) as usize;
            pass.set_pipeline(if v.inject.as_ref().unwrap().1.virtual_map {
                k.inject_virtual.as_ref().unwrap()
            } else {
                &k.inject
            });
            pass.set_bind_group(0, &*inject[i], &[]);
            pass.dispatch_workgroups(
                v.grid[0].div_ceil(4),
                v.grid[1].div_ceil(4),
                FROXEL_SLICES / 4,
            );
            pass.set_pipeline(&k.temporal);
            pass.set_bind_group(0, &*v.temporal.as_ref().unwrap()[i], &[]);
            pass.dispatch_workgroups(
                v.grid[0].div_ceil(4),
                v.grid[1].div_ceil(4),
                FROXEL_SLICES / 4,
            );
            pass.set_pipeline(&k.integrate);
            pass.set_bind_group(0, &*integrate[i], &[]);
            pass.dispatch_workgroups(v.grid[0].div_ceil(8), v.grid[1].div_ceil(8), 1);
        }
        drop(pass);
        a.frame = a.frame.wrapping_add(1);
    }

    /// After a lit frame encodes: its camera becomes the froxels' reprojection source.
    pub(super) fn atmosphere_submitted(&mut self, camera: &Camera, size: [u32; 2]) {
        if self.atmosphere.active {
            self.atmosphere.previous = Some((
                camera.matrix(size[0], size[1]),
                camera.view().inverse().w_axis.truncate(),
            ));
        }
    }
}
