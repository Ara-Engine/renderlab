//! Renderlab's forward lit frame, whole, on ara-gpu: clustered light lists, then the
//! lit mesh pass with the standard program into the exposed HDR target.
//!
//! This is the first frame assembled from the ported passes rather than one pass
//! replayed alone. It covers what `Renderer::try_render_view(.., ViewMode::Lit)`
//! encodes on an untraced frame with the atmosphere, sun and local shadows, GI,
//! decals and transmission off: every such input is bound with the same "off"
//! contents the wgpu renderer binds. [`render`] is one-shot; it owns nothing across
//! frames.
use super::{
    atmosphere::AtmosphereParameters,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    light_cluster::{ClusterInputs, LightClusters},
    mesh::MeshGeometry,
    post_surface::RasterDepth,
    shader::{InputKey, InputKind, Pass, Program, SampleType},
    toon::{MeshAttachments, ToonDraw, ToonPass},
    transfer,
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

/// Renders `scene` once and returns the exposed linear HDR target as tightly packed
/// RGBA16F rows, the bytes `Renderer::try_capture_hdr` yields for the same frame.
pub fn render(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    scene: &ForwardScene<'_>,
) -> Result<Vec<u8>> {
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
    let toward_sun = scene
        .lighting
        .lights
        .iter()
        .find_map(|l| match l {
            crate::Light::Directional { travel, .. } => Some(-*travel),
            _ => None,
        })
        .unwrap_or(crate::Vec3::Y);
    let atmosphere = AtmosphereParameters::for_camera(
        gpu,
        crate::Atmosphere::off(),
        toward_sun,
        scene.camera,
        size,
        [1, 1],
    )?;
    for buffer in [&camera, &lighting, &atmosphere.record] {
        graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
    }

    // Every other reflected input is a feature that is off in this frame. The wgpu
    // renderer binds zero-filled buffers and zero-initialised textures there.
    let mut buffers = BTreeMap::new();
    let mut images = BTreeMap::new();
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
                if spec.key == key(3, 24) {
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
                images.insert(spec.key, image);
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
                InputKind::Image { .. } => Input::Image(&images[&spec.key]),
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
    // `Renderer::lit_clear`: the uniform environment, exposed, in f64.
    let clear = packed
        .environment
        .map(|v| (f64::from(v) * f64::from(packed.exposure)).min(65504.0) as f32);
    pass.record(
        &mut graph,
        &mut rec,
        &draws,
        &MeshAttachments {
            color: &output,
            depth: &depth,
            clear_color: Some([clear[0], clear[1], clear[2], 1.0]),
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
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let pixels = gpu.read(read).context("HDR readback")?;

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
    atmosphere.retire(gpu, &mut graph)?;
    cluster_inputs.retire(gpu)?;
    depth.retire(gpu, &mut graph)?;
    for image in images.into_values().chain([lists, output]) {
        gpu.retire(Resource::Image(image.image))?;
    }
    for buffer in staging.into_iter().chain([read]) {
        gpu.retire(Resource::Buffer(buffer))?;
    }
    pass.retire(gpu)?;
    clusters.retire(gpu)?;
    gpu.collect()?;
    Ok(pixels)
}
