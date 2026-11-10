//! Renderlab's forward lit frame, whole, on ara-gpu: clustered light lists, then the
//! lit mesh pass with the standard program into the exposed HDR target.
//!
//! This is the first frame assembled from the ported passes rather than one pass
//! replayed alone. It covers what `Renderer::try_render_view(.., ViewMode::Lit)`
//! encodes on an untraced frame with the atmosphere, sun and local shadows, GI,
//! decals and transmission off: every such input is bound with the same "off"
//! contents the wgpu renderer binds. With an active atmosphere it adds
//! `encode_atmosphere`: the four LUTs, the procedural environment cube, the aerial
//! volume, sun transmittance on the directional records and the sky backdrop under
//! the mesh pass (volumetric fog is not assembled). With sun shadows it is the frame
//! `Renderer::new_with_hardware_shadows` draws for a sun with a disk: the scene's
//! acceleration structures, a receiver prepass, 256-ray fresh visibility and the lit
//! pass composed on the fresh prelude. With post settings it continues through
//! `PostGpu::encode` and the tone stage: motion surface, tile maximum, motion blur,
//! bloom, tone and the display transfer. [`render`] is one-shot; it owns nothing
//! across frames, so the frame is always the first after a cut.
use super::{
    acceleration::Scene,
    atmosphere::{AtmosphereParameters, AtmospherePasses, AtmosphereSun, AtmosphereTargets},
    bindings::{HeapBindings, Input},
    bloom::Bloom,
    buffer::StorageBuffer,
    cache::ShaderCache,
    environment::EnvironmentPasses,
    fresh_visibility::{self, Mode, Visibility},
    graph::Graph,
    image::Texture,
    kernel::Kernel,
    light_cluster::{ClusterInputs, LightClusters},
    mesh::MeshGeometry,
    post::{FilterInputs, PostPasses, SettingsRecord, ToneInputs},
    post_surface::{PostCameraFrame, RasterDepth, RasterPostSurface},
    primary_shade, primary_trace,
    receiver_raster::{ReceiverDraw, ReceiverTargets, Receivers},
    shader::{self, InputKey, InputKind, Pass, Program, SampleType, ShaderInput},
    sky::SkyPass,
    toon::{MeshAttachments, ToonDraw, ToonPass},
    transfer,
    volumes::VolumePasses,
};
use crate::{light_cluster as lc, lighting::PackedLighting, renderer::shadow_gpu};
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::{
    AccelerationStructure, Access, Device, GpuDevice, Queue, Resource,
    acceleration::{AccelerationDesc, AccelerationInstance},
    commands::Command,
    descriptors::Format,
    sampler::SamplerDesc,
    texture::{SampleCount, ViewDimension},
};
use std::collections::BTreeMap;

/// One opaque object: its own mesh, one instance and untextured material factors.
pub struct ForwardObject<'a> {
    /// Renderlab's 48-byte vertices.
    pub vertices: &'a [crate::Vertex],
    /// Triangle list.
    pub indices: &'a [u32],
    /// Object-to-world transform.
    pub transform: crate::Mat4,
    /// Factors and sidedness; texture slots must be empty (the 1x1 defaults bind).
    pub material: crate::MaterialDescriptor<'a>,
}

/// A scene for [`render`], in the terms the wgpu renderer takes it.
pub struct ForwardScene<'a> {
    /// Draw order; equal depths resolve to the earlier object, as in the mesh pass.
    pub objects: &'a [ForwardObject<'a>],
    /// Lights, uniform environment radiance and exposure.
    pub lighting: &'a crate::Lighting,
    /// View and projection; the aspect follows `size`.
    pub camera: crate::Camera,
    /// Target extent in pixels.
    pub size: [u32; 2],
    /// `Atmosphere::off()` leaves the uniform environment. Volumetric fog must be off.
    pub atmosphere: crate::Atmosphere,
    /// Ray-traced soft shadows for the first directional light; needs a ray-query device.
    pub sun_shadows: Option<&'a SunShadows>,
    /// How primary visibility is resolved. `Traced` needs `sun_shadows` and no post yet.
    pub primary: Primary,
    /// `None` stops at the exposed HDR target. Depth of field is not assembled yet.
    pub post: Option<crate::PostSettings>,
}

/// Which of the renderer's frame families draws the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// The mesh pass: what an untraced `ViewMode::Lit` frame encodes.
    Raster,
    /// Camera rays and deferred shading, as `HardwareRt` at `Quality::Reference` with
    /// GI, temporal reconstruction and reflections off: the camera trace with
    /// receivers, the gated fresh estimator, then shading on the hardware soft head.
    Traced,
}

/// The sun's shadow request, as `Shadows` carries it for a directional light.
#[derive(Clone, Debug)]
pub struct SunShadows {
    /// Positive camera-forward distances; the last one is the coverage limit.
    pub split_ends: Vec<f32>,
    /// The sun disk's half-angle in radians, in (0, 0.1].
    pub disk_angle: f32,
}

/// Readbacks of one frame, each tightly packed in row order.
pub struct ForwardImages {
    /// The exposed linear HDR target as RGBA16F: `Renderer::try_capture_hdr`'s bytes.
    pub hdr: Vec<u8>,
    /// Output-space shutter velocity and linear depth as RGBA32F, when motion blur ran.
    pub surface: Option<Vec<u8>>,
    /// The displayed image as RGBA8 with the display transfer applied, when post ran.
    pub display: Option<Vec<u8>>,
}

/// The post `Settings` record of the first frame after a cut at this output height,
/// with the identity LUT the renderer starts with.
pub fn post_record(settings: &crate::PostSettings, height: u32) -> [f32; 20] {
    settings.record(1, height, LUT_SIDE, false)
}
const LUT_SIDE: u32 = 2;
/// `atmosphere_gpu::FROXEL_TILE`: output pixels per camera-volume cell.
const FROXEL_TILE: u32 = 8;

/// What sun shadows add: the scene to query, receivers to query from and the counts.
struct Traced {
    /// Vertex and index build inputs with their bottom-level structure, per object.
    meshes: Vec<(StorageBuffer, StorageBuffer, AccelerationStructure)>,
    scene: Scene,
    world: Texture,
    normal: Texture,
    /// Visible rays of 256 per pixel, `R32Uint`.
    visibility: Texture,
    /// The shadow view records the estimator and the lit pass read.
    records: StorageBuffer,
    receivers: Receivers,
    kernel: Visibility,
    program: Program,
}

/// What a traced frame adds to the scene and receivers: `Renderer::prepare_primary`'s
/// records and targets, and the three kernels.
struct Deferred {
    trace: Kernel,
    trace_program: Program,
    fresh: Kernel,
    fresh_program: Program,
    shade: Kernel,
    /// Hit records, one tile flag per trace group, and receiver precision.
    hit: Texture,
    tiles: Texture,
    epsilon: Texture,
    frame: StorageBuffer,
    region: StorageBuffer,
    placements: StorageBuffer,
    /// Current then previous model matrices, and the normal transforms, per placement.
    models: StorageBuffer,
    normals: StorageBuffer,
    /// Pads the per-draw arrays, as the renderer's 48-byte absent buffer does.
    absent: StorageBuffer,
    /// Dispatch groups of the trace, the estimator and the shading.
    groups: [[u32; 2]; 3],
}

/// The per-draw arrays of a traced frame, padded to `TRACED_DRAWS`.
#[derive(Default)]
struct Arrays<'a> {
    vertices: Vec<&'a StorageBuffer>,
    indices: Vec<&'a StorageBuffer>,
    absent: Vec<&'a StorageBuffer>,
    materials: Vec<&'a StorageBuffer>,
    samplers: Vec<SamplerDesc>,
    textures: BTreeMap<InputKey, Vec<&'a Texture>>,
}

/// Everything a packet of this frame can name, borrowed for the packets' lifetime.
struct Shared<'a> {
    output: &'a Texture,
    deferred: Option<&'a Deferred>,
    arrays: &'a Arrays<'a>,
    camera: &'a StorageBuffer,
    lighting: &'a StorageBuffer,
    clusters: &'a ClusterInputs,
    lists: &'a Texture,
    atmosphere: &'a AtmosphereParameters,
    sky: Option<&'a Sky>,
    traced: Option<&'a Traced>,
    buffers: &'a BTreeMap<InputKey, StorageBuffer>,
    images: &'a BTreeMap<InputKey, Texture>,
}
impl<'a> Shared<'a> {
    /// One reflected input of any of the frame's programs, for one object or none.
    fn input(
        &self,
        spec: &ShaderInput,
        object: Option<(&'a MeshGeometry, &'a StorageBuffer)>,
    ) -> Result<Input<'a>> {
        let per_object = || object.with_context(|| format!("{} needs an object", spec.name));
        let deferred = || self.deferred.context("not a traced frame");
        Ok(match (spec.kind, spec.key.group, spec.key.binding) {
            (InputKind::AccelerationStructure, ..) => {
                Input::Scene(&self.traced.context("no scene in this frame")?.scene)
            }
            (InputKind::Buffer { .. }, group, binding) => Input::Buffer(match (group, binding) {
                (0, 0) => self.camera,
                (0, 10) => &deferred()?.frame,
                (0, 22) => &deferred()?.region,
                (1, 0) => match object {
                    Some((geometry, _)) => geometry.models(),
                    None => &deferred()?.models,
                },
                (1, 1) => match object {
                    Some((geometry, _)) => geometry.normals(),
                    None => &deferred()?.normals,
                },
                (1, 5) => &deferred()?.placements,
                (2, 0) => per_object()?.1,
                (3, 0) => self.lighting,
                (3, 1) => &self.clusters.lights,
                (3, 23) => &self.clusters.frame,
                (3, 50) => &self.atmosphere.record,
                (3, 5) if self.traced.is_some() => &self.traced.context("checked")?.records,
                _ => &self.buffers[&spec.key],
            }),
            (InputKind::BufferArray { .. }, group, binding) => {
                Input::Buffers(match (group, binding) {
                    (1, 2) => &self.arrays.vertices,
                    (1, 3) => &self.arrays.indices,
                    (2, 10) => &self.arrays.materials,
                    _ => &self.arrays.absent,
                })
            }
            (InputKind::ImageArray { .. }, ..) => Input::Images(
                self.arrays
                    .textures
                    .get(&spec.key)
                    .with_context(|| format!("forward frame cannot supply {}", spec.name))?,
            ),
            (InputKind::SamplerArray { .. }, ..) => Input::Samplers(&self.arrays.samplers),
            (InputKind::Image { .. } | InputKind::StorageImage { .. }, group, binding) => {
                Input::Image(match (group, binding) {
                    (3, 24) => self.lists,
                    (0, 18) => self.output,
                    _ => provided(self.sky, self.traced, self.deferred, spec.key)
                        .or_else(|| self.images.get(&spec.key))
                        .with_context(|| format!("forward frame cannot supply {}", spec.name))?,
                })
            }
            (InputKind::Sampler { .. }, ..) => Input::Sampler(SamplerDesc::default()),
        })
    }
}
/// The atmosphere's and the shadow passes' images where they ran.
fn provided<'a>(
    sky: Option<&'a Sky>,
    traced: Option<&'a Traced>,
    deferred: Option<&'a Deferred>,
    key: InputKey,
) -> Option<&'a Texture> {
    match (key.group, key.binding) {
        (0, 13) | (0, 7) => deferred.map(|d| &d.hit),
        (0, 20) | (0, 21) => deferred.map(|d| &d.tiles),
        (0, 23) => deferred.map(|d| &d.epsilon),
        (0, 3) | (0, 16) => traced.map(|t| &t.world),
        (0, 4) | (0, 17) => traced.map(|t| &t.normal),
        (0, 5) | (0, 6) => traced.map(|t| &t.visibility),
        (3, 51) => sky.map(|s| &s.luts.transmittance),
        (3, 52) => sky.map(|s| &s.luts.sky),
        (3, 53) => sky.map(|s| &s.volume),
        (3, 6) => sky?.environment.as_ref().map(|e| &e[0]),
        (3, 7) => sky?.environment.as_ref().map(|e| &e[1]),
        _ => None,
    }
}

/// What an active atmosphere produces for the sky backdrop and the lit pass.
struct Sky {
    luts: AtmosphereTargets,
    /// Camera volume: fog in slices 0..64 (zero here), aerial perspective in 64..96.
    volume: Texture,
    scatter: Texture,
    /// The procedural diffuse and specular cubes, when the environment is on.
    environment: Option<[Texture; 2]>,
    kernels: (AtmospherePasses, VolumePasses, Option<EnvironmentPasses>),
}

fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}

/// The standard program composed exactly as the wgpu renderer's lit raster variant, or
/// its fresh variant, whose `hard_shadow` reads the receiver pixel's visible-ray count.
pub fn standard_program(fresh: bool) -> Result<Program> {
    let source = crate::program::STANDARD_PROGRAM;
    let scene = include_str!("../../shaders/scene.wgsl");
    let head = if fresh {
        crate::renderer::fresh_shadow_gpu::shader_source(scene)?
    } else {
        scene.to_owned()
    };
    Program::new(
        Pass::MeshToon,
        crate::program::compose(
            &head,
            source,
            crate::program::Stage::Raster,
            crate::program::declares_parameters(source),
        )
        .source,
    )
}

/// Renders `scene` once and reads back what it produced.
pub fn render(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    scene: &ForwardScene<'_>,
) -> Result<ForwardImages> {
    ensure!(!scene.objects.is_empty(), "forward frame needs an object");
    ensure!(
        scene.objects.iter().all(|o| {
            o.material.base_color_texture.is_none()
                && o.material.metallic_roughness_texture.is_none()
                && o.material.normal_texture.is_none()
                && o.material.program.is_none()
        }),
        "forward frame binds the default textures and the standard program only"
    );
    let size = scene.size;
    let deferred_path = scene.primary == Primary::Traced;
    // Post's motion surface on a traced frame comes from hit records (entry 35), which
    // this frame does not chain yet; bloom and tone need nothing from the frame family.
    ensure!(
        !deferred_path
            || (scene.sun_shadows.is_some()
                && scene.post.is_none_or(|p| !p.enabled || !p.motion_blur)),
        "a traced frame needs sun shadows, and its post runs without motion blur for now"
    );
    let program = if deferred_path {
        primary_shade::hardware_soft_program(crate::program::STANDARD_PROGRAM)?
    } else {
        standard_program(scene.sun_shadows.is_some())?
    };
    let pass = if deferred_path {
        None
    } else {
        Some(ToonPass::program(
            gpu,
            cache,
            &program,
            Format::Rgba16Float,
        )?)
    };
    let mut bounds = Vec::new();
    for object in scene.objects {
        let local =
            crate::visibility::Aabb::from_positions(object.vertices.iter().map(|v| v.position))
                .context("empty mesh")?;
        bounds.push(shadow_gpu::world_bounds(local, object.transform)?);
    }
    let clusters = LightClusters::new(gpu, cache)?;
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();

    // Lighting group 3: records, the eye/count/environment/exposure uniform and the
    // cluster frame, packed by Renderlab's own writers.
    let packed = PackedLighting::new(scene.lighting, lc::MAX_CLUSTERED as u32)?;
    let eye = scene.camera.view().inverse().w_axis.truncate();
    let mut uniform = [0u8; 48];
    uniform[..32].copy_from_slice(&packed.uniform(eye));
    let mut records = packed.records.clone();
    records.resize(records.len().max(crate::lighting::LIGHT_BYTES), 0);
    // `Renderer::render_view` with hardware shadows: plan the sun's views, convert their
    // biases to ray distances and publish each light's view range in its record.
    let shadow_frame = match scene.sun_shadows {
        Some(sun_shadows) => {
            let mut first = true;
            let shadows = crate::Shadows {
                requests: scene
                    .lighting
                    .lights
                    .iter()
                    .map(|l| match l {
                        crate::Light::Directional { .. } if std::mem::take(&mut first) => {
                            crate::ShadowRequest::Directional {
                                split_ends: sun_shadows.split_ends.clone(),
                            }
                        }
                        _ => crate::ShadowRequest::None,
                    })
                    .collect(),
                directional_disk_angle: sun_shadows.disk_angle,
                ..Default::default()
            };
            ensure!(
                !first && sun_shadows.disk_angle > 0. && sun_shadows.disk_angle <= 0.1,
                "sun shadows need a directional light and a disk angle in (0, 0.1]"
            );
            // `hardware_shadow_gpu`'s admission rule for casters.
            ensure!(
                scene.objects.iter().all(|o| o.material.double_sided),
                "hardware shadows currently require opaque double-sided materials"
            );
            let mut frame = shadow_gpu::plan_frame(
                &shadows,
                &[],
                &scene.camera,
                size,
                &scene.lighting.lights,
                &bounds,
            )?;
            frame.prepare_hardware_bias(&records)?;
            for (record, range) in records
                .as_chunks_mut::<{ crate::lighting::LIGHT_BYTES }>()
                .0
                .iter_mut()
                .zip(&frame.light_ranges)
            {
                record[52..56].copy_from_slice(&range[0].to_ne_bytes());
                record[56..60].copy_from_slice(&range[1].to_ne_bytes());
            }
            Some(frame)
        }
        None => None,
    };
    // `Renderer::atmosphere_sun` and `sun_scale`: the first directional light.
    let settings = scene.atmosphere;
    settings.validate()?;
    let sun = scene.lighting.lights.iter().find_map(|l| match *l {
        crate::Light::Directional { travel, lux } => Some((-travel.normalize(), lux)),
        _ => None,
    });
    if settings.active() {
        ensure!(
            settings.volumetric_fog.is_none(),
            "forward frame does not assemble volumetric fog yet"
        );
        if let (true, Some((toward, _))) = (settings.sun_transmittance, sun) {
            records = crate::atmosphere::scale_directional(
                &records,
                settings.transmittance(eye.y - settings.medium.ground_height, toward),
            );
        }
        if settings.environment {
            // `EnvironmentState::uniform` for the unrotated five-level procedural chain.
            uniform[32..].copy_from_slice(bytemuck::cast_slice(&[1f32, 0., 1., 5.]));
        }
    }
    let mut frame = lc::header(&scene.camera, size, packed.count, true, &[]);
    frame[100..104].copy_from_slice(&crate::LocalShadows::default().near.to_ne_bytes());
    let cluster_inputs = ClusterInputs {
        lights: StorageBuffer::new(gpu, &records)?,
        frame: StorageBuffer::new(gpu, &frame)?,
    };
    cluster_inputs.import(&mut graph);
    let lists = LightClusters::target(gpu)?;
    clusters.record(&mut graph, &mut rec, &cluster_inputs, &lists)?;

    let mut camera = [0u8; 80];
    camera[..64].copy_from_slice(&crate::mesh::matrix_bytes(
        scene.camera.matrix(size[0], size[1]),
    ));
    camera[64..68].copy_from_slice(&(crate::ViewMode::Lit as u32).to_ne_bytes());
    let camera = StorageBuffer::new(gpu, &camera)?;
    let lighting = StorageBuffer::new(gpu, &uniform)?;
    let grid = if settings.active() {
        size.map(|n| n.div_ceil(FROXEL_TILE))
    } else {
        [1, 1]
    };
    let (toward_sun, illuminance) = sun.unwrap_or((crate::Vec3::Y, [0.; 3]));
    let atmosphere = AtmosphereParameters::for_camera_with_sun(
        gpu,
        settings,
        AtmosphereSun {
            toward_sun,
            illuminance,
            exposure: packed.exposure,
        },
        scene.camera,
        size,
        grid,
    )?;
    for buffer in [&camera, &lighting, &atmosphere.record] {
        graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
    }

    // `Renderer::encode_atmosphere`, in its order. Stale LUTs: this is a first frame.
    let sky = if settings.active() {
        let luts = AtmosphereTargets::new(gpu)?;
        let volume = Texture::volume_shape(gpu, [grid[0], grid[1], 96], Format::Rgba16Float, true)?;
        let scatter =
            Texture::volume_shape(gpu, [grid[0], grid[1], 64], Format::Rgba16Float, false)?;
        // wgpu zero-initialises these; the kernels write only part of each.
        for (image, texels) in [
            (luts.transmittance.image, 256 * 65),
            (volume.image, grid[0] * grid[1] * 96),
            (scatter.image, grid[0] * grid[1] * 64),
        ] {
            staging.push(transfer::upload(
                gpu,
                &mut graph,
                &mut rec,
                image,
                &vec![0; texels as usize * 8],
            )?);
        }
        let lut_kernels = AtmospherePasses::new(gpu, cache)?;
        for stage in [
            Pass::AtmosphereTransmittance,
            Pass::AtmosphereMultiscatter,
            Pass::AtmosphereSky,
            Pass::AtmosphereAmbient,
        ] {
            lut_kernels.record(&mut graph, &mut rec, stage, &atmosphere, &luts)?;
        }
        let mut environment = None;
        let mut environment_kernels = None;
        if settings.environment {
            let kernels = EnvironmentPasses::new(gpu, cache)?;
            let (specular, levels) = Texture::cube_chain(gpu, 128, 5)?;
            for level in &levels {
                kernels.record(
                    &mut graph,
                    &mut rec,
                    Pass::EnvironmentSpecular,
                    &atmosphere,
                    &luts.sky,
                    level,
                )?;
            }
            let (diffuse, levels) = Texture::cube_chain(gpu, 16, 1)?;
            kernels.record(
                &mut graph,
                &mut rec,
                Pass::EnvironmentDiffuse,
                &atmosphere,
                &luts.sky,
                &levels[0],
            )?;
            environment = Some([diffuse, specular]);
            environment_kernels = Some(kernels);
        }
        let volume_kernels = VolumePasses::new(gpu, cache)?;
        volume_kernels.record(
            &mut graph,
            &mut rec,
            Pass::AtmosphereAerial,
            &atmosphere,
            &luts,
            &scatter,
            &volume,
        )?;
        Some(Sky {
            luts,
            volume,
            scatter,
            environment,
            kernels: (lut_kernels, volume_kernels, environment_kernels),
        })
    } else {
        None
    };
    // The scene and the fresh visibility's targets and kernels.
    let traced = match &shadow_frame {
        Some(frame) => {
            let usage =
                ara_gpu::vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
                    | ara_gpu::vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
            let mut meshes = Vec::new();
            let mut instances = Vec::new();
            for (index, object) in scene.objects.iter().enumerate() {
                let vertex_bytes: Vec<u8> = object
                    .vertices
                    .iter()
                    .flat_map(|v| {
                        v.position
                            .into_iter()
                            .chain(v.normal)
                            .chain(v.uv)
                            .chain(v.color)
                    })
                    .flat_map(f32::to_le_bytes)
                    .collect();
                let vertices = StorageBuffer::with_usage(gpu, &vertex_bytes, usage)?;
                let indices =
                    StorageBuffer::with_usage(gpu, bytemuck::cast_slice(object.indices), usage)?;
                let blas = gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
                    vertices: vertices.buffer,
                    vertex_count: object.vertices.len() as u32,
                    vertex_stride: 48,
                    indices: indices.buffer,
                    index_count: object.indices.len() as u32,
                })?;
                for buffer in [&vertices, &indices] {
                    graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                }
                graph.node(
                    &mut rec,
                    &[
                        (Resource::Buffer(vertices.buffer), Access::AsInput),
                        (Resource::Buffer(indices.buffer), Access::AsInput),
                        (Resource::AccelerationStructure(blas), Access::AsBuild),
                    ],
                )?;
                rec.push(Command::BuildAccelerationStructure(blas));
                // `hardware_shadow_gpu`: one instance per placement, in draw order.
                let m = object.transform.to_cols_array_2d();
                instances.push(AccelerationInstance {
                    geometry: blas,
                    transform: std::array::from_fn(|i| m[i % 4][i / 4]),
                    custom_index: index as u32,
                    mask: 255,
                });
                meshes.push((vertices, indices, blas));
            }
            let tlas = Scene::new(gpu, instances)?;
            let mut uses: Vec<_> = meshes
                .iter()
                .map(|m| (Resource::AccelerationStructure(m.2), Access::AsInput))
                .collect();
            uses.push((
                Resource::AccelerationStructure(tlas.acceleration),
                Access::AsBuild,
            ));
            graph.node(&mut rec, &uses)?;
            rec.push(Command::BuildAccelerationStructure(tlas.acceleration));
            let mut bytes = frame.records().to_vec();
            bytes.resize(bytes.len().max(crate::shadows::VIEW_BYTES as usize), 0);
            let records = StorageBuffer::new(gpu, &bytes)?;
            graph.import(Resource::Buffer(records.buffer), Access::HostWrite);
            Some(Traced {
                meshes,
                scene: tlas,
                world: Texture::new(gpu, size, Format::Rgba32Float, deferred_path, true)?,
                normal: Texture::new(gpu, size, Format::Rgba32Float, deferred_path, true)?,
                visibility: Texture::new(gpu, size, Format::R32Uint, true, false)?,
                records,
                receivers: Receivers::new(gpu, cache)?,
                kernel: Visibility::new(gpu, cache, Mode::Exact)?,
                program: Mode::Exact.program()?,
            })
        }
        None => None,
    };

    // Every other reflected input, of any program this frame runs, is a feature that is
    // off. The wgpu renderer binds zero-filled buffers and zero-initialised textures there.
    // `Renderer::lit_clear`: the uniform environment, exposed, in f64. The sky
    // backdrop replaces it on every pixel when the atmosphere draws one.
    let clear = packed
        .environment
        .map(|v| (f64::from(v) * f64::from(packed.exposure)).min(65504.0) as f32);
    // `Renderer::prepare_primary`: the frame and region records, the placement table,
    // the placement transforms and the targets of a traced frame.
    let deferred = if deferred_path {
        let trace_program = primary_trace::Entry::Receivers.program()?;
        let fresh_program = fresh_visibility::traced_program()?;
        let matrix = scene.camera.matrix(size[0], size[1]);
        let inverse = matrix.inverse();
        ensure!(inverse.is_finite(), "traced camera inverse is nonfinite");
        let rect = crate::renderer::primary_ray_gpu::scene_pixels(&bounds, matrix, size);
        let lo = [rect[0] / 8 * 8, rect[1] / 4 * 4];
        let region = [
            lo[0],
            lo[1],
            lo[0] + (rect[2] - lo[0]).div_ceil(8) * 8,
            lo[1] + (rect[3] - lo[1]).div_ceil(4) * 4,
        ];
        let placements = crate::primary_ray::placement_table(
            scene
                .objects
                .iter()
                .map(|o| (1, false, !o.material.double_sided, o.material.thin)),
        )?;
        let mut frame = [0u8; 144];
        frame[..64].copy_from_slice(&crate::mesh::matrix_bytes(inverse));
        frame[64..80].copy_from_slice(bytemuck::cast_slice(&[clear[0], clear[1], clear[2], 1.]));
        let sided = placements.iter().any(|p| p & (1 << 30) != 0);
        frame[80..84].copy_from_slice(&u32::from(sided).to_ne_bytes());
        frame[96..112].copy_from_slice(bytemuck::cast_slice(&rect));
        frame[112..116].copy_from_slice(&8u32.to_ne_bytes());
        frame[116..120].copy_from_slice(&8u32.to_ne_bytes());
        frame[124..128].copy_from_slice(&1f32.to_bits().to_ne_bytes());
        for (dst, value) in frame[128..144]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(crate::ReflectionSettings::off().frame_words())
        {
            *dst = value.to_ne_bytes();
        }
        let transforms: Vec<_> = scene.objects.iter().map(|o| o.transform).collect();
        let (matrices, normal_bytes) = super::mesh::pack_models(&transforms)?;
        let frame = StorageBuffer::new(gpu, &frame)?;
        let region_buffer = StorageBuffer::new(gpu, bytemuck::cast_slice(&region))?;
        let placement_buffer = StorageBuffer::new(gpu, bytemuck::cast_slice(&placements))?;
        // No history on a first frame: the previous matrices are the current ones.
        let models = StorageBuffer::new(gpu, &[matrices.as_slice(), &matrices].concat())?;
        let normals = StorageBuffer::new(gpu, &normal_bytes)?;
        let absent = StorageBuffer::new(gpu, &[0; 48])?;
        for buffer in [
            &frame,
            &region_buffer,
            &placement_buffer,
            &models,
            &normals,
            &absent,
        ] {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
        }
        let trace_groups = [size[0].div_ceil(8), size[1].div_ceil(4)];
        Some(Deferred {
            trace: Kernel::from_program(gpu, cache, &trace_program)?,
            fresh: Kernel::from_program(gpu, cache, &fresh_program)?,
            shade: Kernel::from_program(gpu, cache, &program)?,
            trace_program,
            fresh_program,
            hit: Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
            tiles: Texture::new(gpu, trace_groups, Format::R32Uint, true, false)?,
            epsilon: Texture::new(gpu, size, Format::R32Float, true, false)?,
            frame,
            region: region_buffer,
            placements: placement_buffer,
            models,
            normals,
            absent,
            groups: [
                trace_groups,
                [size[0].div_ceil(2), size[1].div_ceil(2)],
                [
                    (region[2] - region[0])
                        .div_ceil(8)
                        .max(size[0].div_ceil(16)),
                    (region[3] - region[1]).div_ceil(4) + size[1].div_ceil(16),
                ],
            ],
        })
    } else {
        None
    };

    let mut specs = program.inputs().to_vec();
    if let Some(traced) = &traced {
        specs.extend(shader::inputs(Pass::ReceiverRaster)?);
        specs.extend(traced.program.inputs().iter().cloned());
    }
    if let Some(deferred) = &deferred {
        specs.extend(deferred.trace_program.inputs().iter().cloned());
        specs.extend(deferred.fresh_program.inputs().iter().cloned());
    }
    let mut buffers = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    for spec in &specs {
        match spec.kind {
            InputKind::Buffer { minimum, .. } => {
                if [
                    key(0, 0),
                    key(1, 0),
                    key(1, 1),
                    key(2, 0),
                    key(3, 0),
                    key(3, 1),
                    key(3, 23),
                    key(3, 50),
                ]
                .contains(&spec.key)
                    || (spec.key == key(3, 5) && traced.is_some())
                    || (deferred.is_some()
                        && [key(0, 10), key(0, 22), key(1, 5)].contains(&spec.key))
                    || buffers.contains_key(&spec.key)
                {
                    continue;
                }
                let buffer = StorageBuffer::new(gpu, &vec![0; (minimum as usize).max(64)])?;
                graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                buffers.insert(spec.key, buffer);
            }
            InputKind::Image { dimension, sample }
            | InputKind::ImageArray {
                dimension, sample, ..
            } => {
                if spec.key == key(3, 24)
                    || provided(sky.as_ref(), traced.as_ref(), deferred.as_ref(), spec.key)
                        .is_some()
                    || inputs.contains_key(&spec.key)
                {
                    continue;
                }
                let (format, texel) = match sample {
                    SampleType::Float => (Format::Rgba16Float, 8),
                    SampleType::Uint => (Format::R32Uint, 4),
                    SampleType::Depth => (Format::Depth32Float, 4),
                };
                // The defaults `Renderer::new_configured` creates, and the prepared DFG.
                let (format, extent, bytes) = match (spec.key.group, spec.key.binding) {
                    (2, 1 | 11) => (Format::Rgba8Srgb, [1, 1, 1], vec![255; 4]),
                    (2, 3 | 13) => (Format::Rgba8, [1, 1, 1], vec![255; 4]),
                    (2, 5 | 15) => (Format::Rgba8, [1, 1, 1], vec![128, 128, 255, 255]),
                    (3, 2) => (
                        Format::Rg16Float,
                        [crate::lighting::DFG_SIZE, crate::lighting::DFG_SIZE, 1],
                        crate::lighting::DFG.to_vec(),
                    ),
                    _ => {
                        let layers = if dimension == ViewDimension::D2 { 1 } else { 6 };
                        (format, [1, 1, layers], vec![0; texel * layers as usize])
                    }
                };
                let flat = [extent[0], extent[1]];
                let image = match dimension {
                    ViewDimension::D2 => Texture::new(gpu, flat, format, false, false)?,
                    ViewDimension::D2Array => Texture::array(
                        gpu,
                        flat,
                        extent[2],
                        format,
                        false,
                        sample == SampleType::Depth,
                    )?,
                    ViewDimension::D3 => Texture::volume_shape(gpu, extent, format, false)?,
                };
                staging.push(transfer::upload(
                    gpu,
                    &mut graph,
                    &mut rec,
                    image.image,
                    &bytes,
                )?);
                inputs.insert(spec.key, image);
            }
            InputKind::Sampler { comparison } => {
                ensure!(!comparison, "unexpected comparison sampler {}", spec.name)
            }
            InputKind::AccelerationStructure | InputKind::StorageImage { .. }
                if traced.is_some() => {}
            InputKind::BufferArray { .. } | InputKind::SamplerArray { .. }
                if deferred.is_some() => {}
            _ => bail!("forward frame cannot supply {}", spec.name),
        }
    }

    let mut geometries = Vec::new();
    let mut materials = Vec::new();
    for object in scene.objects {
        if !deferred_path {
            let geometry = MeshGeometry::new(
                gpu,
                object.vertices,
                object.indices,
                &[0],
                &[object.transform],
            )?;
            geometry.import(&mut graph);
            geometries.push(geometry);
        }
        let material = StorageBuffer::new(gpu, &object.material.bytes())?;
        graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
        materials.push(material);
    }
    let output = Texture::new(gpu, size, Format::Rgba16Float, deferred_path, true)?;
    let depth = RasterDepth::new(gpu, size, SampleCount::One)?;
    let mut arrays = Arrays::default();
    if let (Some(deferred), Some(traced)) = (&deferred, &traced) {
        let draws = crate::TRACED_DRAWS as usize;
        ensure!(
            materials.len() <= draws,
            "traced frames hold TRACED_DRAWS draws"
        );
        arrays.vertices = (0..draws)
            .map(|i| traced.meshes.get(i).map_or(&deferred.absent, |m| &m.0))
            .collect();
        arrays.indices = (0..draws)
            .map(|i| traced.meshes.get(i).map_or(&deferred.absent, |m| &m.1))
            .collect();
        arrays.absent = vec![&deferred.absent; draws];
        // The renderer pads with its default material; padded slots are never read.
        arrays.materials = (0..draws)
            .map(|i| materials.get(i).unwrap_or(&materials[0]))
            .collect();
        arrays.samplers = vec![SamplerDesc::default(); draws];
        for spec in &specs {
            if let InputKind::ImageArray { .. } = spec.kind {
                arrays
                    .textures
                    .insert(spec.key, vec![&inputs[&spec.key]; draws]);
            }
        }
    }
    let shared = Shared {
        output: &output,
        deferred: deferred.as_ref(),
        arrays: &arrays,
        camera: &camera,
        lighting: &lighting,
        clusters: &cluster_inputs,
        lists: &lists,
        atmosphere: &atmosphere,
        sky: sky.as_ref(),
        traced: traced.as_ref(),
        buffers: &buffers,
        images: &inputs,
    };
    // One lit packet per object on the raster path (none when traced: no geometries).
    let mut packets = Vec::new();
    for object in geometries.iter().zip(&materials) {
        let values = program
            .inputs()
            .iter()
            .map(|spec| Ok((spec.key, shared.input(spec, Some(object))?)))
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::for_program(gpu, &program, &values)?;
        packet.import(&mut graph);
        packets.push(packet);
    }
    // The receiver prepass draws every object; the estimator runs once over the frame.
    let mut shadow_packets = Vec::new();
    if let Some(traced) = traced.as_ref().filter(|_| !deferred_path) {
        let layout = shader::inputs(Pass::ReceiverRaster)?;
        for object in geometries.iter().zip(&materials) {
            let values = layout
                .iter()
                .map(|spec| Ok((spec.key, shared.input(spec, Some(object))?)))
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::new(gpu, Pass::ReceiverRaster, &values)?;
            packet.import(&mut graph);
            shadow_packets.push(packet);
        }
        let values = traced
            .program
            .inputs()
            .iter()
            .map(|spec| Ok((spec.key, shared.input(spec, None)?)))
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::for_program(gpu, &traced.program, &values)?;
        packet.import(&mut graph);
        shadow_packets.push(packet);
    }

    // The traced frame's three packets: none of them belongs to one object.
    let mut deferred_packets = Vec::new();
    if let Some(deferred) = &deferred {
        for stage in [&deferred.trace_program, &deferred.fresh_program, &program] {
            let values = stage
                .inputs()
                .iter()
                .map(|spec| Ok((spec.key, shared.input(spec, None)?)))
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, stage, &values)?;
            packet.import(&mut graph);
            deferred_packets.push(packet);
        }
        // `encode_primary_visibility`, `encode_primary_fresh`, `encode_primary_shading`.
        let threads = [
            deferred.trace_program.threads()?,
            deferred.fresh_program.threads()?,
            program.threads()?,
        ];
        for (((kernel, packet), groups), threads) in
            [&deferred.trace, &deferred.fresh, &deferred.shade]
                .into_iter()
                .zip(&deferred_packets)
                .zip(deferred.groups)
                .zip(threads)
        {
            kernel.record(
                &mut graph,
                &mut rec,
                packet,
                [groups[0] * threads[0], groups[1] * threads[1], 1],
            )?;
        }
    }
    let draws: Vec<_> = geometries
        .iter()
        .zip(&packets)
        .map(|(geometry, bindings)| ToonDraw { geometry, bindings })
        .collect();
    // `Renderer::encode_fresh`: receivers with the frame's depth, then visibility.
    if let (Some(traced), Some((estimate, receivers))) = (&traced, shadow_packets.split_last()) {
        let draws: Vec<_> = geometries
            .iter()
            .zip(receivers)
            .map(|(geometry, bindings)| ReceiverDraw { geometry, bindings })
            .collect();
        traced.receivers.record(
            &mut graph,
            &mut rec,
            &draws,
            ReceiverTargets {
                world: &traced.world,
                normal: &traced.normal,
                depth: &depth,
            },
        )?;
        traced.kernel.record(&mut graph, &mut rec, estimate)?;
    }
    let backdrop = match sky.as_ref().filter(|_| settings.sky && !deferred_path) {
        Some(sky) => {
            let backdrop = SkyPass::new(gpu, cache, Format::Rgba16Float)?;
            backdrop.record(
                &mut graph,
                &mut rec,
                &atmosphere,
                &sky.luts,
                &sky.volume,
                &output,
            )?;
            Some(backdrop)
        }
        None => None,
    };
    if let Some(pass) = &pass {
        pass.record(
            &mut graph,
            &mut rec,
            &draws,
            &MeshAttachments {
                color: &output,
                depth: &depth,
                clear_color: backdrop
                    .is_none()
                    .then_some([clear[0], clear[1], clear[2], 1.0]),
                clear_depth: Some(0.0),
            },
        )?;
    }
    drop(draws);
    let read = transfer::readback(
        gpu,
        &mut graph,
        &mut rec,
        output.image,
        u64::from(size[0]) * u64::from(size[1]) * 8,
    )?;

    // The post tail, in `PostGpu::encode` order. Each stage reads the one before it.
    let mut post_images = Vec::new();
    let mut post_buffers = Vec::new();
    let mut reads = [None, None];
    let mut tail = None;
    if let Some(settings) = scene.post.filter(|s| s.enabled) {
        settings.validate()?;
        ensure!(
            settings.depth_of_field.is_none(),
            "forward frame does not assemble depth of field yet"
        );
        let passes = PostPasses::new(gpu, cache, Format::Rgba32Float)?;
        let record = SettingsRecord::new(gpu, &post_record(&settings, size[1]))?;
        graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
        let lut = Texture::volume(gpu, LUT_SIDE, Format::Rgba16Float)?;
        let identity: Vec<u8> = (0..LUT_SIDE.pow(3))
            .flat_map(|i| {
                [i & 1, (i >> 1) & 1, i >> 2, 1].map(|v| half::f16::from_f32(v as f32).to_bits())
            })
            .flat_map(u16::to_le_bytes)
            .collect();
        post_buffers.push(transfer::upload(
            gpu, &mut graph, &mut rec, lut.image, &identity,
        )?);
        let mut surface = None;
        if settings.motion_blur {
            let kernel = RasterPostSurface::new(gpu, cache)?;
            let frame = PostCameraFrame::new(
                gpu,
                scene.camera,
                None,
                size,
                settings.shutter_angle_degrees,
                settings.max_motion_radius_px,
            )?;
            graph.import(Resource::Buffer(frame.record.buffer), Access::HostWrite);
            let auxiliary = Texture::new(gpu, size, Format::Rgba32Float, true, false)?;
            kernel.record(&mut graph, &mut rec, &depth, &frame, &auxiliary)?;
            let tiles = Texture::new(
                gpu,
                size.map(|n| n.div_ceil(16)),
                Format::Rgba16Float,
                true,
                false,
            )?;
            passes.filter(
                &mut graph,
                &mut rec,
                Pass::TileMax,
                &FilterInputs {
                    colour: &output,
                    second: &output,
                    auxiliary: &auxiliary,
                    velocity_tiles: &output,
                    settings: &record,
                },
                &tiles,
            )?;
            let blurred = Texture::new(gpu, size, Format::Rgba16Float, true, false)?;
            passes.filter(
                &mut graph,
                &mut rec,
                Pass::MotionBlur,
                &FilterInputs {
                    colour: &output,
                    second: &output,
                    auxiliary: &auxiliary,
                    velocity_tiles: &tiles,
                    settings: &record,
                },
                &blurred,
            )?;
            reads[0] = Some(transfer::readback(
                gpu,
                &mut graph,
                &mut rec,
                auxiliary.image,
                u64::from(size[0]) * u64::from(size[1]) * 16,
            )?);
            surface = Some((kernel, frame, blurred));
            post_images.extend([auxiliary, tiles]);
        }
        let colour = surface.as_ref().map_or(&output, |(_, _, blurred)| blurred);
        let bloom = if settings.bloom > 0.0 {
            let bloom = Bloom::new(gpu, size)?;
            bloom.record(&passes, &mut graph, &mut rec, colour, &record)?;
            Some(bloom)
        } else {
            None
        };
        let linear = Texture::new(gpu, size, Format::Rgba32Float, false, true)?;
        passes.tone(
            &mut graph,
            &mut rec,
            &ToneInputs {
                colour,
                bloom: bloom.as_ref().map_or(colour, Bloom::output),
                lut: &lut,
                settings: &record,
            },
            &linear,
        )?;
        let display = Texture::new(gpu, size, Format::Rgba8, false, true)?;
        passes.display(&mut graph, &mut rec, &linear, &display)?;
        reads[1] = Some(transfer::readback(
            gpu,
            &mut graph,
            &mut rec,
            display.image,
            u64::from(size[0]) * u64::from(size[1]) * 4,
        )?);
        post_images.extend([lut, linear, display]);
        post_buffers.push(record.buffer);
        tail = Some((passes, bloom, surface));
    }

    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let images = ForwardImages {
        hdr: gpu.read(read).context("HDR readback")?,
        surface: reads[0].map(|b| gpu.read(b)).transpose()?,
        display: reads[1].map(|b| gpu.read(b)).transpose()?,
    };

    if let Some((passes, bloom, surface)) = tail {
        if let Some(bloom) = bloom {
            bloom.retire(gpu)?;
        }
        if let Some((kernel, frame, blurred)) = surface {
            kernel.retire(gpu)?;
            frame.record.retire(gpu)?;
            post_images.push(blurred);
        }
        passes.retire(gpu)?;
    }
    for packet in packets
        .into_iter()
        .chain(shadow_packets)
        .chain(deferred_packets)
    {
        packet.retire(gpu, &mut graph)?;
    }
    if let Some(deferred) = deferred {
        for kernel in [deferred.trace, deferred.fresh, deferred.shade] {
            kernel.retire(gpu)?;
        }
        for buffer in [
            deferred.frame,
            deferred.region,
            deferred.placements,
            deferred.models,
            deferred.normals,
            deferred.absent,
        ] {
            buffer.retire(gpu)?;
        }
        post_images.extend([deferred.hit, deferred.tiles, deferred.epsilon]);
    }
    if let Some(traced) = traced {
        traced.receivers.retire(gpu)?;
        traced.kernel.retire(gpu)?;
        traced.scene.retire(gpu)?;
        for (vertices, indices, blas) in traced.meshes {
            gpu.retire(Resource::AccelerationStructure(blas))?;
            vertices.retire(gpu)?;
            indices.retire(gpu)?;
        }
        traced.records.retire(gpu)?;
        post_images.extend([traced.world, traced.normal, traced.visibility]);
    }
    for geometry in geometries {
        geometry.retire(gpu, &mut graph)?;
    }
    for buffer in materials
        .into_iter()
        .chain(buffers.into_values())
        .chain([camera, lighting])
    {
        buffer.retire(gpu)?;
    }
    if let Some(backdrop) = backdrop {
        backdrop.retire(gpu)?;
    }
    if let Some(sky) = sky {
        let (luts, volumes, environment) = sky.kernels;
        luts.retire(gpu)?;
        volumes.retire(gpu)?;
        if let Some(environment) = environment {
            environment.retire(gpu)?;
        }
        post_images.extend(
            [
                sky.luts.transmittance,
                sky.luts.multiscatter,
                sky.luts.sky,
                sky.volume,
                sky.scatter,
            ]
            .into_iter()
            .chain(sky.environment.into_iter().flatten()),
        );
    }
    atmosphere.retire(gpu, &mut graph)?;
    cluster_inputs.retire(gpu)?;
    depth.retire(gpu, &mut graph)?;
    for image in inputs
        .into_values()
        .chain(post_images)
        .chain([lists, output])
    {
        gpu.retire(Resource::Image(image.image))?;
    }
    for buffer in staging
        .into_iter()
        .chain(post_buffers)
        .chain(reads.into_iter().flatten())
        .chain([read])
    {
        gpu.retire(Resource::Buffer(buffer))?;
    }
    if let Some(pass) = pass {
        pass.retire(gpu)?;
    }
    clusters.retire(gpu)?;
    gpu.collect()?;
    Ok(images)
}
