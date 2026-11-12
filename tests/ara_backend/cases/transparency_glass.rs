//! Original programmable pane shading over retained four-sample HDR color.
use super::transparency_glass_reference as cpu;
use super::*;
use ara_gpu::sampler::{Filter, SamplerDesc};
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    mesh::MeshGeometry,
    shader::{InputKey, InputKind},
    transfer::readback,
    transparency_glass::{self, Glass},
    transparency_seed::{MsaaColor, Seed},
};
#[test]
fn programmable_transparent_glass_native_replay_against_cpu() -> Result<()> {
    assert_eq!(cpu::SAMPLES, 4);
    let path = root("transparent-glass");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let program = transparency_glass::program(cpu::PROGRAM)?;
    ensure!(
        program.threads().is_err(),
        "raster advertised a compute size"
    );
    let export = path.join("authored-spirv");
    let stages = renderlab::backend_ara::cache::emit_spirv_program(&program, &export)?;
    for stage in stages {
        let bytes = std::fs::read(export.join(&stage.spirv))?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        assert_nonuniform_resources(&words)?;
        if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
            let validator = std::path::PathBuf::from(sdk).join(if cfg!(windows) {
                "Bin/spirv-val.exe"
            } else {
                "bin/spirv-val"
            });
            let output = std::process::Command::new(validator)
                .args(["--target-env", "vulkan1.3"])
                .arg(export.join(stage.spirv))
                .output()?;
            ensure!(
                output.status.success(),
                "authored glass SPIR-V: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let glass = Glass::new(&mut gpu, &cache, &program)?;
    let seed = Seed::new(&mut gpu, &cache)?;
    let sampler = SamplerDesc {
        min: Filter::Nearest,
        mag: Filter::Nearest,
        mip: Filter::Nearest,
        ..Default::default()
    };
    for case in [
        cpu::Case::Ordered,
        cpu::Case::BackFacingMirroredPane,
        cpu::Case::ZeroOpacity,
    ] {
        let fixture = cpu::fixture(case);
        let mut graph = Graph::default();
        let mut rec = gpu.begin(Queue::Graphics);
        let background = Texture::new(&mut gpu, fixture.size, Format::Rgba16Float, false, false)?;
        let behind = Texture::new(&mut gpu, fixture.size, Format::Rgba16Float, false, true)?;
        let depth = Texture::new(&mut gpu, fixture.size, Format::R32Float, false, false)?;
        let normal = Texture::array(&mut gpu, [1, 1], 1, Format::Rgba16Float, false, false)?;
        let mr = Texture::new(&mut gpu, [1, 1], Format::Rgba16Float, false, false)?;
        let transmittance = Texture::new(&mut gpu, [1, 1], Format::Rgba16Float, false, false)?;
        let aerial = Texture::volume(&mut gpu, 1, Format::Rgba16Float)?;
        // The light cluster's lists, unread: no cluster is prepared.
        let cluster_lists = Texture::new(&mut gpu, [1, 1], Format::R32Uint, false, false)?;
        let aa = MsaaColor::new(&mut gpu, fixture.size)?;
        let mut staging = Vec::new();
        for (texture, bytes) in [
            (&background, half_bytes(&fixture.background.pixels)),
            (
                &depth,
                bytemuck::cast_slice::<f32, u8>(&fixture.opaque_depth).to_vec(),
            ),
            (&normal, half_bytes(&fixture.normal_map.pixels)),
            (&mr, half_bytes(&fixture.metallic_roughness.pixels)),
            (&transmittance, half_bytes(&[[1.; 4]])),
            (&aerial, half_bytes(&[[0., 0., 0., 1.]])),
            (&cluster_lists, vec![0; 4]),
        ] {
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                texture.image,
                &bytes,
            )?);
        }
        let common: Vec<_> = [
            &fixture.camera[..],
            &fixture.frame[..],
            &fixture.lighting[..],
            &fixture.atmosphere[..],
        ]
        .into_iter()
        .map(|w| StorageBuffer::new(&mut gpu, bytemuck::cast_slice(w)))
        .collect::<Result<_>>()?;
        // The light cluster's frame, all zero: no cluster is prepared (`info.y` is 0), so
        // each fragment walks every light, as the scalar reference does.
        let cluster_bytes = program
            .inputs()
            .iter()
            .find_map(|s| match (s.key.group, s.key.binding, s.kind) {
                (3, 23, InputKind::Buffer { minimum, .. }) => Some(minimum as usize),
                _ => None,
            })
            .unwrap_or(16);
        let cluster_frame = StorageBuffer::new(&mut gpu, &vec![0; cluster_bytes])?;
        for b in common.iter().chain([&cluster_frame]) {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
        let seed_packet = HeapBindings::new(
            &mut gpu,
            Pass::TransparencySeed,
            &[(
                InputKey {
                    group: 0,
                    binding: 31,
                },
                Input::Image(&background),
            )],
        )?;
        seed_packet.import(&mut graph);
        seed.record(&mut graph, &mut rec, &seed_packet, &aa)?;
        let models: Vec<_> = fixture
            .models
            .iter()
            .map(|m| renderlab::Mat4::from_cols_array(&m.map(f32::from_bits)))
            .collect();
        let mut geometries = Vec::new();
        let mut records = Vec::new();
        let mut bases = Vec::new();
        let mut snapshots = Vec::new();
        for pane in &fixture.panes {
            let geometry = MeshGeometry::new(
                &mut gpu,
                &pane.vertices,
                &pane.indices,
                &[pane.slot],
                &models,
            )?;
            ensure!(
                gpu.read(geometry.models().buffer)?
                    == bytemuck::cast_slice::<[u32; 16], u8>(&fixture.models),
                "model ABI mismatch"
            );
            ensure!(
                gpu.read(geometry.normals().buffer)?
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .eq(fixture
                        .normals
                        .iter()
                        .flatten()
                        .copied()
                        .map(f32::from_bits)),
                "normal ABI mismatch"
            );
            geometry.import(&mut graph);
            let material = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&pane.material))?;
            let params = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&pane.parameters))?;
            let draw = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&pane.draw))?;
            for b in [&material, &params, &draw] {
                graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
            }
            let base = Texture::new(&mut gpu, pane.base.size, Format::Rgba16Float, false, false)?;
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                base.image,
                &half_bytes(&pane.base.pixels),
            )?);
            let values: Vec<_> = program
                .inputs()
                .iter()
                .map(|s| {
                    let input = match (s.key.group, s.key.binding) {
                        (0, 0) => Input::Buffer(&common[0]),
                        (0, 30) => Input::Buffer(&common[1]),
                        (0, 31) => Input::Image(&behind),
                        (0, 32) => Input::Image(&depth),
                        (0, 37) => Input::Buffer(&draw),
                        (1, 0) => Input::Buffer(geometry.models()),
                        (1, 1) => Input::Buffer(geometry.normals()),
                        (2, 0) => Input::Buffer(&material),
                        (2, 1) => Input::Image(&base),
                        (2, 3) => Input::Image(&mr),
                        (2, 5) => Input::Image(&normal),
                        (2, 7) => Input::Buffer(&params),
                        (3, 0) => Input::Buffer(&common[2]),
                        (3, 23) => Input::Buffer(&cluster_frame),
                        (3, 24) => Input::Image(&cluster_lists),
                        (3, 50) => Input::Buffer(&common[3]),
                        (3, 51) => Input::Image(&transmittance),
                        (3, 53) => Input::Image(&aerial),
                        _ if matches!(s.kind, InputKind::Sampler { comparison: false }) => {
                            Input::Sampler(sampler)
                        }
                        _ => anyhow::bail!("unexpected glass input {s:?}"),
                    };
                    Ok((s.key, input))
                })
                .collect::<Result<_>>()?;
            let packet = HeapBindings::for_program(&mut gpu, &program, &values)?;
            packet.import(&mut graph);
            aa.resolve_to(&mut graph, &mut rec, &behind)?;
            glass.record(&mut graph, &mut rec, &packet, &geometry, &aa)?;
            aa.resolve_to(&mut graph, &mut rec, &behind)?;
            snapshots.push(readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                behind.image,
                u64::from(fixture.size[0] * fixture.size[1] * 8),
            )?);
            // Packet ownership ends after this list has been submitted, not after encoding.
            let point = gpu.submit(rec.finish(), &[])?;
            packet.retire(&mut gpu, &mut graph)?;
            rec = gpu.begin(Queue::Graphics);
            records.extend([material, params, draw]);
            geometries.push(geometry);
            bases.push(base);
            if snapshots.len() == fixture.panes.len() {
                gpu.wait(point)?;
            }
        }
        let actual: Vec<_> = snapshots
            .iter()
            .map(|&b| {
                let bytes = gpu.read(b)?;
                Ok(reference::Image {
                    size: fixture.size,
                    pixels: bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|p| {
                            std::array::from_fn(|j| {
                                f16::from_bits(u16::from_le_bytes([p[j * 2], p[j * 2 + 1]]))
                                    .to_f32()
                            })
                        })
                        .collect(),
                })
            })
            .collect::<Result<_>>()?;
        fixture.assert_matches(&actual);
        println!("transparent glass {case:?}: two ordered 4x resolves match scalar reference");
        seed_packet.retire(&mut gpu, &mut graph)?;
        aa.retire(&mut gpu, &mut graph)?;
        for geometry in geometries {
            geometry.retire(&mut gpu, &mut graph)?;
        }
        for b in common.into_iter().chain([cluster_frame]).chain(records) {
            b.retire(&mut gpu)?;
        }
        for t in [
            background,
            behind,
            depth,
            normal,
            mr,
            transmittance,
            aerial,
            cluster_lists,
        ]
        .into_iter()
        .chain(bases)
        {
            gpu.retire(Resource::Image(t.image))?;
        }
        for b in staging.into_iter().chain(snapshots) {
            gpu.retire(Resource::Buffer(b))?;
        }
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// The standard program as glass on a ray-traced renderer's head. Its fragment stage
/// traces the pane's own sun ray against the frame's scene, so its interface names the
/// scene and no sun map, and it is still one vertex and one fragment stage whose SPIR-V
/// validates and whose pipeline compiles natively.
#[test]
fn traced_glass_reflects_validates_and_compiles() -> Result<()> {
    let path = root("traced-glass");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let program = transparency_glass::traced_program(renderlab::STANDARD_PROGRAM)?;
    let key = |binding| InputKey { group: 0, binding };
    let kind = |binding| {
        program
            .inputs()
            .iter()
            .find(|i| i.key == key(binding))
            .map(|i| i.kind)
    };
    ensure!(
        matches!(kind(2), Some(InputKind::AccelerationStructure))
            && kind(34).is_none()
            && kind(35).is_none()
            && matches!(kind(30), Some(InputKind::Buffer { minimum: 336, .. }))
            && program.inputs().len() == 35,
        "unexpected traced glass interface: {} inputs",
        program.inputs().len()
    );
    let export = path.join("spirv");
    let stages = renderlab::backend_ara::cache::emit_spirv_program(&program, &export)?;
    ensure!(stages.len() == 2, "traced glass is not two stages");
    let mut queries = 0;
    for stage in &stages {
        let bytes = std::fs::read(export.join(&stage.spirv))?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        assert_nonuniform_resources(&words)?;
        // OpRayQueryInitializeKHR, in the fragment stage alone.
        let mut at = 5;
        while at < words.len() {
            queries += usize::from(words[at] & 0xffff == 4473);
            at += (words[at] >> 16) as usize;
        }
        if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
            let validator = std::path::PathBuf::from(sdk).join(if cfg!(windows) {
                "Bin/spirv-val.exe"
            } else {
                "bin/spirv-val"
            });
            if validator.is_file() {
                let output = std::process::Command::new(validator)
                    .args(["--target-env", "vulkan1.3"])
                    .arg(export.join(&stage.spirv))
                    .output()?;
                ensure!(
                    output.status.success(),
                    "traced glass SPIR-V: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    ensure!(queries > 0, "the traced glass stages trace nothing");
    // The pipeline compiles.
    transparency_glass::Glass::new(&mut gpu, &cache, &program)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
