//! Original fresh soft-shadow ray queries against scalar triangle intersections.
use super::fresh_visibility_reference as cpu;
use super::*;
use ara_gpu::acceleration::{AccelerationDesc, AccelerationInstance};
use renderlab::backend_ara::{
    acceleration::Scene,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    cache::emit_spirv_program,
    fresh_visibility::{Mode, Visibility},
    shader::InputKey,
    transfer::readback,
};
#[test]
fn fresh_shadow_visibility_native_replay_against_cpu() -> Result<()> {
    let path = root("fresh-visibility");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    ensure!(gpu.info().ray_tracing, "ray queries unavailable");
    let cache = ShaderCache::new(path.join("shaders"))?;
    let modes = [Mode::Exact, Mode::Interactive];
    let programs = modes
        .map(Mode::program)
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let export = path.join("interactive-spirv");
    for stage in emit_spirv_program(&programs[1], &export)? {
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
                "interactive fresh SPIR-V: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let passes = modes
        .into_iter()
        .map(|m| Visibility::new(&mut gpu, &cache, m))
        .collect::<Result<Vec<_>>>()?;
    let fixtures = [
        cpu::fixture(0., true),
        cpu::fixture(0.25, true),
        cpu::fixture(0.25, false),
    ];
    let first = &fixtures[0];
    let usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
    let vertices =
        StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&first.vertices), usage)?;
    let indices = StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&first.indices), usage)?;
    let blas = gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
        vertices: vertices.buffer,
        vertex_count: first.vertices.len() as u32,
        vertex_stride: 12,
        indices: indices.buffer,
        index_count: first.indices.len() as u32,
    })?;
    let instance = |x| AccelerationInstance {
        geometry: blas,
        transform: [1., 0., 0., x, 0., 1., 0., 0., 0., 0., 1., 0.],
        custom_index: 7,
        mask: 255,
    };
    let decoy = Scene::new(&mut gpu, vec![instance(4096.)])?;
    let scene = Scene::new(&mut gpu, vec![instance(0.)])?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(vertices.buffer), Access::HostWrite);
    graph.import(Resource::Buffer(indices.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    graph.node(
        &mut rec,
        &[
            (Resource::Buffer(vertices.buffer), Access::AsInput),
            (Resource::Buffer(indices.buffer), Access::AsInput),
            (Resource::AccelerationStructure(blas), Access::AsBuild),
        ],
    )?;
    rec.push(Command::BuildAccelerationStructure(blas));
    for tlas in [&decoy, &scene] {
        graph.node(
            &mut rec,
            &[
                (Resource::AccelerationStructure(blas), Access::AsInput),
                (
                    Resource::AccelerationStructure(tlas.acceleration),
                    Access::AsBuild,
                ),
            ],
        )?;
        rec.push(Command::BuildAccelerationStructure(tlas.acceleration));
    }
    let world = Texture::new(&mut gpu, first.size, Format::Rgba32Float, false, false)?;
    let normal = Texture::new(&mut gpu, first.size, Format::Rgba32Float, false, false)?;
    let output = Texture::new(&mut gpu, first.size, Format::R32Uint, true, false)?;
    let mut staging = Vec::new();
    let mut records = Vec::new();
    let mut snapshots = Vec::new();
    let mut last = None;
    for (frame, f) in fixtures.iter().enumerate() {
        ensure!(
            f.vertices == first.vertices && f.indices == first.indices && f.size == first.size,
            "fixture scene changed"
        );
        for (image, bytes) in [
            (world.image, bytemuck::cast_slice::<[f32; 4], u8>(&f.world)),
            (
                normal.image,
                bytemuck::cast_slice::<[f32; 4], u8>(&f.normal),
            ),
        ] {
            staging.push(upload(&mut gpu, &mut graph, &mut rec, image, bytes)?);
        }
        let lighting = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lighting))?;
        let light = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.light))?;
        let view = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.view))?;
        for b in [&lighting, &light, &view] {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
        for (index, (pass, program)) in passes.iter().zip(&programs).enumerate() {
            let directions = if index == 0 { 256 } else { 8 };
            let packet = HeapBindings::for_program(
                &mut gpu,
                program,
                &[
                    (
                        InputKey {
                            group: 0,
                            binding: 2,
                        },
                        Input::Scene(&scene),
                    ),
                    (
                        InputKey {
                            group: 0,
                            binding: 3,
                        },
                        Input::Image(&world),
                    ),
                    (
                        InputKey {
                            group: 0,
                            binding: 4,
                        },
                        Input::Image(&normal),
                    ),
                    (
                        InputKey {
                            group: 0,
                            binding: 5,
                        },
                        Input::Image(&output),
                    ),
                    (
                        InputKey {
                            group: 3,
                            binding: 0,
                        },
                        Input::Buffer(&lighting),
                    ),
                    (
                        InputKey {
                            group: 3,
                            binding: 1,
                        },
                        Input::Buffer(&light),
                    ),
                    (
                        InputKey {
                            group: 3,
                            binding: 5,
                        },
                        Input::Buffer(&view),
                    ),
                ],
            )?;
            packet.import(&mut graph);
            let sentinel = vec![0xdeadbeefu32; (f.size[0] * f.size[1]) as usize];
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                output.image,
                bytemuck::cast_slice(&sentinel),
            )?);
            pass.record(&mut graph, &mut rec, &packet)?;
            let read = readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                output.image,
                u64::from(f.size[0] * f.size[1] * 4),
            )?;
            last = Some(gpu.submit(rec.finish(), &[])?);
            packet.retire(&mut gpu, &mut graph)?;
            snapshots.push((frame, directions, read, cpu::visibility(f, directions)));
            rec = gpu.begin(Queue::Graphics);
        }
        records.extend([lighting, light, view]);
    }
    gpu.wait(last.expect("six queued fresh query variants"))?;
    for (frame, directions, read, expected) in snapshots {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        ensure!(
            actual == expected,
            "fresh visibility frame{frame} directions{directions}: {actual:?} != {expected:?}"
        );
        let partial = actual.iter().filter(|&&x| x > 0 && x < 256).count();
        println!(
            "fresh visibility frame{frame} directions{directions}:{} exact counts, {partial} penumbra receivers",
            actual.len()
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    for p in passes {
        p.retire(&mut gpu)?;
    }
    for b in records {
        b.retire(&mut gpu)?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    for t in [world, normal, output] {
        gpu.retire(Resource::Image(t.image))?;
    }
    scene.retire(&mut gpu)?;
    decoy.retire(&mut gpu)?;
    gpu.retire(Resource::AccelerationStructure(blas))?;
    vertices.retire(&mut gpu)?;
    indices.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
