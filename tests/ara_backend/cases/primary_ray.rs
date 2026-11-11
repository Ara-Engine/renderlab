//! Native heap-bound ray queries against known transformed geometry.
use super::*;
use ara_gpu::acceleration::{AccelerationDesc, AccelerationInstance};
use renderlab::backend_ara::{
    acceleration::Scene,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::Program,
    transfer::readback,
};
#[test]
fn heap_bound_primary_ray_query_native_replay_against_cpu() -> Result<()> {
    let fixtures = [
        primary_ray_reference::fixture(false),
        primary_ray_reference::fixture(true),
    ];
    let f = &fixtures[0];
    let path = root("primary-ray");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    ensure!(
        gpu.info().ray_tracing,
        "native ray-query device unavailable"
    );
    let cache = ShaderCache::new(path.join("shaders"))?;
    let source = include_str!("../../../shaders/primary_ray_probe.wgsl");
    let programs = [
        Program::new(Pass::PrimaryRayProbe, source)?,
        Program::new(
            Pass::PrimaryRayProbe,
            source.replace(
                "override WRITE_WORLD:bool=false;",
                "override WRITE_WORLD:bool=true;",
            ),
        )?,
    ];
    let export = path.join("world-spirv");
    let stages = renderlab::backend_ara::cache::emit_spirv_program(&programs[1], &export)?;
    for stage in stages {
        let bytes = std::fs::read(export.join(&stage.spirv))?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        assert_nonuniform_resources(&words)?;
        if let Ok(sdk) = std::env::var("VULKAN_SDK") {
            let validator = std::path::Path::new(&sdk).join(if cfg!(windows) {
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
                "ray query SPIR-V: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let kernels = [
        Kernel::from_program(&mut gpu, &cache, &programs[0])?,
        Kernel::from_program(&mut gpu, &cache, &programs[1])?,
    ];
    let usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
    let vertices = StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&f.vertices), usage)?;
    let indices = StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&f.indices), usage)?;
    let blas = gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
        vertices: vertices.buffer,
        vertex_count: f.vertices.len() as u32,
        vertex_stride: 12,
        indices: indices.buffer,
        index_count: f.indices.len() as u32,
    })?;
    // Keep a built, empty-in-view TLAS in the first heap slot. A backend that
    // accidentally retains the constant-zero SPIR-V marker must fail this probe.
    let decoy = Scene::new(
        &mut gpu,
        vec![AccelerationInstance {
            geometry: blas,
            transform: [1., 0., 0., 100., 0., 1., 0., 0., 0., 0., 1., 0.],
            custom_index: 99,
            mask: 255,
        }],
    )?;
    let instances = f
        .instances
        .iter()
        .map(|i| AccelerationInstance {
            geometry: blas,
            transform: i.transform,
            custom_index: i.custom_index,
            mask: i.mask,
        })
        .collect();
    let scene = Scene::new(&mut gpu, instances)?;
    let camera = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.camera))?;
    let hits = Texture::new(&mut gpu, f.size, Format::Rgba32Uint, true, false)?;
    let depths = Texture::new(&mut gpu, f.size, Format::R32Float, true, false)?;
    let worlds = Texture::new(&mut gpu, f.size, Format::Rgba32Float, true, false)?;
    let packets: Vec<_> = programs
        .iter()
        .map(|program| {
            let values: Vec<_> = program
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match spec.key.binding {
                            0 => Input::Buffer(&camera),
                            1 => Input::Scene(&scene),
                            2 => Input::Image(&hits),
                            3 => Input::Image(&depths),
                            4 => Input::Image(&worlds),
                            _ => anyhow::bail!("unknown ray probe binding"),
                        },
                    ))
                })
                .collect::<Result<_>>()?;
            HeapBindings::for_program(&mut gpu, program, &values)
        })
        .collect::<Result<_>>()?;
    let mut graph = Graph::default();
    for b in [&vertices, &indices, &camera] {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
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
    graph.node(
        &mut rec,
        &[
            (Resource::AccelerationStructure(blas), Access::AsInput),
            (
                Resource::AccelerationStructure(decoy.acceleration),
                Access::AsBuild,
            ),
        ],
    )?;
    rec.push(Command::BuildAccelerationStructure(decoy.acceleration));
    graph.node(
        &mut rec,
        &[
            (Resource::AccelerationStructure(blas), Access::AsInput),
            (
                Resource::AccelerationStructure(scene.acceleration),
                Access::AsBuild,
            ),
        ],
    )?;
    rec.push(Command::BuildAccelerationStructure(scene.acceleration));
    let n = u64::from(f.size[0]) * u64::from(f.size[1]);
    let sentinel = vec![0xa5; n as usize * 16];
    let staging = upload(&mut gpu, &mut graph, &mut rec, worlds.image, &sentinel)?;
    let mut snapshots = Vec::new();
    let mut last = None;
    for (i, packet) in packets.iter().enumerate() {
        let f = &fixtures[i];
        ensure!(
            programs[i].threads()? == [8, 4, 1]
                && f.write_world == (i == 1)
                && f.dispatch() == [3, 3, 1],
            "query config differs"
        );
        packet.import(&mut graph);
        kernels[i].record(&mut graph, &mut rec, packet, [f.size[0], f.size[1], 1])?;
        let a = readback(&mut gpu, &mut graph, &mut rec, hits.image, n * 16)?;
        let b = readback(&mut gpu, &mut graph, &mut rec, depths.image, n * 4)?;
        let c = readback(&mut gpu, &mut graph, &mut rec, worlds.image, n * 16)?;
        snapshots.push([a, b, c]);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two ray-query snapshots"))?;
    for (i, [a, b, c]) in snapshots.into_iter().enumerate() {
        let bytes = gpu.read(a)?;
        let hits: Vec<[u32; 4]> = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|j| {
                    u32::from_le_bytes(p[j * 4..j * 4 + 4].try_into().expect("word"))
                })
            })
            .collect();
        let depths: Vec<f32> = gpu
            .read(b)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| f32::from_le_bytes(*w))
            .collect();
        let bytes = gpu.read(c)?;
        let worlds: Vec<[f32; 4]> = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|j| {
                    f32::from_le_bytes(p[j * 4..j * 4 + 4].try_into().expect("word"))
                })
            })
            .collect();
        if i == 0 {
            ensure!(bytes == sentinel, "disabled world output was written");
        }
        fixtures[i].assert_matches(&hits, &depths, if i == 0 { None } else { Some(&worlds) });
        println!(
            "primary ray{i}:187 exact hit identities,123 exact misses; transformed closest hits/barycentrics/depth/world within2e-5"
        );
        for b in [a, b, c] {
            gpu.retire(Resource::Buffer(b))?;
        }
    }
    for p in packets {
        p.retire(&mut gpu, &mut graph)?;
    }
    for k in kernels {
        k.retire(&mut gpu)?;
    }
    scene.retire(&mut gpu)?;
    decoy.retire(&mut gpu)?;
    gpu.retire(Resource::AccelerationStructure(blas))?;
    for b in [vertices, indices, camera] {
        b.retire(&mut gpu)?;
    }
    for t in [hits, depths, worlds] {
        gpu.retire(Resource::Image(t.image))?;
    }
    gpu.retire(Resource::Buffer(staging))?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
