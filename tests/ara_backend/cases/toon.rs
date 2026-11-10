//! Original lit toon source with a complete heap packet and an unshadowed scene.
use super::*;
use ara_gpu::{
    sampler::SamplerDesc,
    texture::{SampleCount, ViewDimension},
};
use renderlab::backend_ara::{
    atmosphere::AtmosphereParameters,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    mesh::{DiagnosticCamera, DiagnosticMaterial, MeshGeometry},
    post_surface::RasterDepth,
    shader::{InputKey, InputKind, SampleType},
    toon::{MeshAttachments, ToonDraw, ToonPass},
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
#[test]
fn lit_toon_original_program_native_replay_against_cpu() -> Result<()> {
    let mut fixture = mesh_reference::fixture();
    let path = root("toon");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = ToonPass::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let layout = renderlab::backend_ara::shader::inputs(Pass::MeshToon)?;
    ensure!(
        layout.len() == 22,
        "unexpected original toon interface: {layout:?}"
    );
    let geometry = MeshGeometry::new(
        &mut gpu,
        &fixture.vertices,
        &fixture.indices,
        &fixture.slots,
        &fixture.models,
    )?;
    let camera = DiagnosticCamera::new(&mut gpu, fixture.view_projection, 0)?;
    let real_camera = renderlab::Camera::look_at(
        renderlab::Vec3::ZERO,
        renderlab::Vec3::NEG_Z,
        renderlab::Vec3::Y,
        1.,
        0.1,
        1000.,
    )?;
    let atmosphere = AtmosphereParameters::for_camera(
        &mut gpu,
        renderlab::Atmosphere::off(),
        renderlab::Vec3::Y,
        real_camera,
        fixture.viewport,
        [1, 1],
    )?;
    let lighting = toon_reference::Lighting::default();
    let mut buffers = BTreeMap::new();
    let mut images = BTreeMap::new();
    let mut image_data = BTreeMap::new();
    for spec in &layout {
        match spec.kind {
            InputKind::AccelerationStructure
            | InputKind::BufferArray { .. }
            | InputKind::ImageArray { .. }
            | InputKind::SamplerArray { .. } => anyhow::bail!("unexpected toon binding array"),
            InputKind::Buffer { minimum, .. } => {
                if [
                    key(0, 0),
                    key(1, 0),
                    key(1, 1),
                    key(2, 0),
                    key(2, 7),
                    key(3, 50),
                ]
                .contains(&spec.key)
                {
                    continue;
                }
                let bytes = match spec.key {
                    InputKey {
                        group: 3,
                        binding: 0,
                    } => bytemuck::cast_slice::<u32, u8>(&lighting.uniform_words()).to_vec(),
                    InputKey {
                        group: 3,
                        binding: 1,
                    } => bytemuck::cast_slice::<u32, u8>(&lighting.light_words()).to_vec(),
                    InputKey {
                        group: 3,
                        binding: 5 | 23 | 26,
                    } => vec![0; minimum as usize],
                    InputKey {
                        group: 3,
                        binding: 21,
                    } => vec![0; 64],
                    _ => anyhow::bail!("unknown original toon buffer {}", spec.name),
                };
                buffers.insert(spec.key, StorageBuffer::new(&mut gpu, &bytes)?);
            }
            InputKind::Image { dimension, sample } => {
                let (size, layers, format, bytes) = match spec.key {
                    InputKey {
                        group: 2,
                        binding: 1,
                    } => (
                        fixture.base.size,
                        1,
                        Format::Rgba16Float,
                        half_bytes(&fixture.base.pixels),
                    ),
                    InputKey {
                        group: 3,
                        binding: 4,
                    } => ([1, 1], 1, Format::Depth32Float, vec![0; 4]),
                    InputKey {
                        group: 3,
                        binding: 6,
                    } => (
                        [1, 1],
                        6,
                        Format::Rgba16Float,
                        half_bytes(&[[0., 0., 0., 1.]; 6]),
                    ),
                    InputKey {
                        group: 3,
                        binding: 24,
                    } => ([1, 1], 1, Format::R32Uint, vec![0; 4]),
                    InputKey {
                        group: 3,
                        binding: 25,
                    } => ([1, 1], 6, Format::Depth32Float, vec![0; 6 * 4]),
                    InputKey {
                        group: 3,
                        binding: 27,
                    } => (
                        [1, 1],
                        1,
                        Format::Rgba16Float,
                        half_bytes(&[[1., 1., 1., 0.]]),
                    ),
                    InputKey {
                        group: 3,
                        binding: 51,
                    } => {
                        let mut pixels = vec![[1.; 4]; 256 * 65];
                        pixels[256 * 64] = [0., 0., 0., 1.];
                        ([256, 65], 1, Format::Rgba16Float, half_bytes(&pixels))
                    }
                    InputKey {
                        group: 3,
                        binding: 53,
                    } => (
                        [1, 1],
                        96,
                        Format::Rgba16Float,
                        half_bytes(&[[0., 0., 0., 1.]; 96]),
                    ),
                    _ => anyhow::bail!("unknown original toon image {}", spec.name),
                };
                let image = match dimension {
                    ViewDimension::D2 => Texture::new(&mut gpu, size, format, false, false)?,
                    ViewDimension::D2Array => Texture::array(
                        &mut gpu,
                        size,
                        layers,
                        format,
                        false,
                        sample == SampleType::Depth,
                    )?,
                    ViewDimension::D3 => {
                        Texture::volume_shape(&mut gpu, [size[0], size[1], layers], format, false)?
                    }
                };
                images.insert(spec.key, image);
                image_data.insert(spec.key, bytes);
            }
            InputKind::StorageImage { .. } => anyhow::bail!("unexpected toon storage image"),
            InputKind::Sampler { comparison } => {
                ensure!(!comparison, "unexpected comparison sampler")
            }
        }
    }
    let materials = [
        DiagnosticMaterial::new(
            &mut gpu,
            fixture.material.base_color,
            fixture.material.metallic,
            fixture.material.roughness,
            false,
        )?,
        DiagnosticMaterial::new(
            &mut gpu,
            fixture.material.base_color,
            fixture.material.metallic,
            fixture.material.roughness,
            true,
        )?,
    ];
    let parameters = [
        toon_reference::Parameters::default(),
        toon_reference::Parameters {
            flat_shaded: 1,
            ..Default::default()
        },
    ];
    let params = [
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&parameters[0].words()))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&parameters[1].words()))?,
    ];
    let output = Texture::new(&mut gpu, fixture.viewport, Format::Rgba32Float, false, true)?;
    let depth = RasterDepth::new(&mut gpu, fixture.viewport, SampleCount::One)?;
    let mut graph = Graph::default();
    geometry.import(&mut graph);
    for b in buffers.values().chain([
        &camera.record,
        &atmosphere.record,
        &materials[0].record,
        &materials[1].record,
        &params[0],
        &params[1],
    ]) {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();
    for (k, image) in &images {
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            image.image,
            &image_data[k],
        )?);
    }
    let mut packets = Vec::new();
    let mut snapshots = Vec::new();
    for (sided, material) in materials.iter().enumerate() {
        fixture.material.double_sided = sided == 1;
        for (flat, param) in params.iter().enumerate() {
            let mut values = Vec::new();
            for spec in &layout {
                let input = match spec.kind {
                    InputKind::AccelerationStructure
                    | InputKind::BufferArray { .. }
                    | InputKind::ImageArray { .. }
                    | InputKind::SamplerArray { .. } => {
                        anyhow::bail!("unexpected toon binding array")
                    }
                    InputKind::Buffer { .. } => Input::Buffer(match spec.key {
                        InputKey {
                            group: 0,
                            binding: 0,
                        } => &camera.record,
                        InputKey {
                            group: 1,
                            binding: 0,
                        } => geometry.models(),
                        InputKey {
                            group: 1,
                            binding: 1,
                        } => geometry.normals(),
                        InputKey {
                            group: 2,
                            binding: 0,
                        } => &material.record,
                        InputKey {
                            group: 2,
                            binding: 7,
                        } => param,
                        InputKey {
                            group: 3,
                            binding: 50,
                        } => &atmosphere.record,
                        _ => &buffers[&spec.key],
                    }),
                    InputKind::Image { .. } => Input::Image(&images[&spec.key]),
                    InputKind::Sampler { .. } => Input::Sampler(SamplerDesc::default()),
                    InputKind::StorageImage { .. } => {
                        anyhow::bail!("unexpected toon storage image")
                    }
                };
                values.push((spec.key, input));
            }
            if sided == 0 && flat == 0 {
                // Reuse the real selected interface to check rejection before publication.
                for (target, replacement, message) in [
                    (key(0, 0), Input::Buffer(param), "too short"),
                    (
                        key(3, 4),
                        Input::Image(&images[&key(3, 24)]),
                        "view dimension",
                    ),
                    (key(2, 1), Input::Image(&images[&key(3, 24)]), "sample type"),
                    (
                        key(2, 2),
                        Input::Sampler(SamplerDesc {
                            compare: Some(ara_gpu::sampler::CompareOp::LessEqual),
                            ..Default::default()
                        }),
                        "wrong heap",
                    ),
                ] {
                    let mut bad = values.clone();
                    bad.iter_mut()
                        .find(|(k, _)| *k == target)
                        .expect("selected key")
                        .1 = replacement;
                    let result = HeapBindings::new(&mut gpu, Pass::MeshToon, &bad);
                    ensure!(
                        result
                            .err()
                            .is_some_and(|e| e.to_string().contains(message)),
                        "wrong rejection for {target:?}"
                    );
                }
                let mut duplicate = values.clone();
                duplicate[1].0 = duplicate[0].0;
                ensure!(
                    HeapBindings::new(&mut gpu, Pass::MeshToon, &duplicate)
                        .err()
                        .is_some_and(|e| e.to_string().contains("duplicate")),
                    "duplicate packet key accepted"
                );
            }
            let packet = HeapBindings::new(&mut gpu, Pass::MeshToon, &values)?;
            packet.import(&mut graph);
            pass.record(
                &mut graph,
                &mut rec,
                &[ToonDraw {
                    geometry: &geometry,
                    bindings: &packet,
                }],
                &MeshAttachments {
                    color: &output,
                    depth: &depth,
                    clear_color: Some(fixture.background),
                    clear_depth: Some(0.),
                },
            )?;
            let read = renderlab::backend_ara::transfer::readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                output.image,
                u64::from(fixture.viewport[0] * fixture.viewport[1] * 16),
            )?;
            snapshots.push((
                sided,
                flat,
                read,
                toon_reference::render(&fixture, parameters[flat], lighting),
            ));
            packets.push(packet);
        }
    }
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    for (sided, flat, read, expected) in snapshots {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        let mut max = 0f32;
        let mut coverage = 0;
        for (a, b) in actual.as_chunks::<4>().0.iter().zip(&expected.pixels) {
            if (*a != fixture.background) != (*b != fixture.background) {
                coverage += 1;
            }
            for (a, b) in a.iter().zip(b) {
                ensure!(a.is_finite(), "nonfinite toon");
                max = max.max((a - b).abs());
            }
        }
        println!("toon sided={sided} flat={flat}: max error {max}, coverage mismatches {coverage}");
        ensure!(coverage == 0 && max <= 0.003, "toon differs");
        staging.push(read);
    }
    for p in packets {
        p.retire(&mut gpu, &mut graph)?;
    }
    for m in materials {
        m.retire(&mut gpu, &mut graph)?;
    }
    for p in params {
        p.retire(&mut gpu)?;
    }
    camera.retire(&mut gpu, &mut graph)?;
    atmosphere.retire(&mut gpu, &mut graph)?;
    geometry.retire(&mut gpu, &mut graph)?;
    depth.retire(&mut gpu, &mut graph)?;
    for b in buffers.into_values() {
        b.retire(&mut gpu)?;
    }
    for t in images.into_values().chain([output]) {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    pass.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
