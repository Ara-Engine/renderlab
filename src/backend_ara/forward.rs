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
//! the mesh pass (volumetric fog is not assembled). With post settings it continues through
//! `PostGpu::encode` and the tone stage: motion surface, tile maximum, motion blur,
//! bloom, tone and the display transfer. [`render`] is one-shot; it owns nothing
//! across frames, so the frame is always the first after a cut.
use super::{
    atmosphere::{AtmosphereParameters, AtmospherePasses, AtmosphereSun, AtmosphereTargets},
    bindings::{HeapBindings, Input},
    bloom::Bloom,
    buffer::StorageBuffer,
    cache::ShaderCache,
    environment::EnvironmentPasses,
    graph::Graph,
    image::Texture,
    light_cluster::{ClusterInputs, LightClusters},
    mesh::MeshGeometry,
    post::{FilterInputs, PostPasses, SettingsRecord, ToneInputs},
    post_surface::{PostCameraFrame, RasterDepth, RasterPostSurface},
    shader::{InputKey, InputKind, Pass, Program, SampleType},
    sky::SkyPass,
    toon::{MeshAttachments, ToonDraw, ToonPass},
    transfer,
    volumes::VolumePasses,
};
use crate::{light_cluster as lc, lighting::PackedLighting};
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::{
    Access, Device, GpuDevice, Queue, Resource,
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
    /// `None` stops at the exposed HDR target. Depth of field is not assembled yet.
    pub post: Option<crate::PostSettings>,
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

/// The standard program composed exactly as the wgpu renderer's lit raster variant.
pub fn standard_program() -> Result<Program> {
    let source = crate::program::STANDARD_PROGRAM;
    Program::new(
        Pass::MeshToon,
        crate::program::compose(
            include_str!("../../shaders/scene.wgsl"),
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
    let program = standard_program()?;
    let pass = ToonPass::program(gpu, cache, &program, Format::Rgba16Float)?;
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
    // The atmosphere's products where it ran; everything else falls through below.
    let sky_image = |key: InputKey| -> Option<&Texture> {
        let sky = sky.as_ref()?;
        match (key.group, key.binding) {
            (3, 51) => Some(&sky.luts.transmittance),
            (3, 52) => Some(&sky.luts.sky),
            (3, 53) => Some(&sky.volume),
            (3, 6) => sky.environment.as_ref().map(|e| &e[0]),
            (3, 7) => sky.environment.as_ref().map(|e| &e[1]),
            _ => None,
        }
    };

    // Every other reflected input is a feature that is off in this frame. The wgpu
    // renderer binds zero-filled buffers and zero-initialised textures there.
    let mut buffers = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    for spec in program.inputs() {
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
                {
                    continue;
                }
                let buffer = StorageBuffer::new(gpu, &vec![0; (minimum as usize).max(64)])?;
                graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                buffers.insert(spec.key, buffer);
            }
            InputKind::Image { dimension, sample } => {
                if spec.key == key(3, 24) || sky_image(spec.key).is_some() {
                    continue;
                }
                let (format, texel) = match sample {
                    SampleType::Float => (Format::Rgba16Float, 8),
                    SampleType::Uint => (Format::R32Uint, 4),
                    SampleType::Depth => (Format::Depth32Float, 4),
                };
                // The defaults `Renderer::new_configured` creates, and the prepared DFG.
                let (format, extent, bytes) = match (spec.key.group, spec.key.binding) {
                    (2, 1) => (Format::Rgba8Srgb, [1, 1, 1], vec![255; 4]),
                    (2, 3) => (Format::Rgba8, [1, 1, 1], vec![255; 4]),
                    (2, 5) => (Format::Rgba8, [1, 1, 1], vec![128, 128, 255, 255]),
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
            _ => bail!("forward frame cannot supply {}", spec.name),
        }
    }

    let mut geometries = Vec::new();
    let mut materials = Vec::new();
    for object in scene.objects {
        let geometry = MeshGeometry::new(
            gpu,
            object.vertices,
            object.indices,
            &[0],
            &[object.transform],
        )?;
        geometry.import(&mut graph);
        let material = StorageBuffer::new(gpu, &object.material.bytes())?;
        graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
        geometries.push(geometry);
        materials.push(material);
    }
    let mut packets = Vec::new();
    for (geometry, material) in geometries.iter().zip(&materials) {
        let mut values = Vec::new();
        for spec in program.inputs() {
            let input = match spec.kind {
                InputKind::Buffer { .. } => {
                    Input::Buffer(match (spec.key.group, spec.key.binding) {
                        (0, 0) => &camera,
                        (1, 0) => geometry.models(),
                        (1, 1) => geometry.normals(),
                        (2, 0) => material,
                        (3, 0) => &lighting,
                        (3, 1) => &cluster_inputs.lights,
                        (3, 23) => &cluster_inputs.frame,
                        (3, 50) => &atmosphere.record,
                        _ => &buffers[&spec.key],
                    })
                }
                InputKind::Image { .. } if spec.key == key(3, 24) => Input::Image(&lists),
                InputKind::Image { .. } => {
                    Input::Image(sky_image(spec.key).unwrap_or_else(|| &inputs[&spec.key]))
                }
                InputKind::Sampler { .. } => Input::Sampler(SamplerDesc::default()),
                _ => bail!("forward frame cannot supply {}", spec.name),
            };
            values.push((spec.key, input));
        }
        let packet = HeapBindings::for_program(gpu, &program, &values)?;
        packet.import(&mut graph);
        packets.push(packet);
    }

    let output = Texture::new(gpu, size, Format::Rgba16Float, false, true)?;
    let depth = RasterDepth::new(gpu, size, SampleCount::One)?;
    let draws: Vec<_> = geometries
        .iter()
        .zip(&packets)
        .map(|(geometry, bindings)| ToonDraw { geometry, bindings })
        .collect();
    // `Renderer::lit_clear`: the uniform environment, exposed, in f64. The sky
    // backdrop replaces it on every pixel when the atmosphere draws one.
    let clear = packed
        .environment
        .map(|v| (f64::from(v) * f64::from(packed.exposure)).min(65504.0) as f32);
    let backdrop = match sky.as_ref().filter(|_| settings.sky) {
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
    for packet in packets {
        packet.retire(gpu, &mut graph)?;
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
    pass.retire(gpu)?;
    clusters.retire(gpu)?;
    gpu.collect()?;
    Ok(images)
}
