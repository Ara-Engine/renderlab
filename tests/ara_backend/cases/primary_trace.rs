//! Original production camera traces, with portable mesh-position fetches.
use super::*;
use ara_gpu::acceleration::{AccelerationDesc, AccelerationInstance};
use primary_trace_reference::{Entry, Mode, Outputs};
use renderlab::backend_ara::{
    acceleration::Scene,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    primary_trace::Entry as TraceEntry,
    shader::{InputKey, inputs},
    transfer::readback,
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
struct Mesh {
    vertices: StorageBuffer,
    indices: StorageBuffer,
    blas: ara_gpu::AccelerationStructure,
}
#[test]
fn production_primary_trace_and_receivers_native_replay_against_cpu() -> Result<()> {
    let fixtures = [
        primary_trace_reference::fixture(Mode::ForcedOpaque),
        primary_trace_reference::fixture(Mode::ConfirmSingleSided),
    ];
    let f = &fixtures[0];
    let path = root("primary-trace");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let programs = [
        TraceEntry::Trace.program()?,
        TraceEntry::Receivers.program()?,
    ];
    let kernels = [
        Kernel::from_program(&mut gpu, &cache, &programs[0])?,
        Kernel::from_program(&mut gpu, &cache, &programs[1])?,
    ];
    for (program, pass) in programs
        .iter()
        .zip([Pass::PrimaryTrace, Pass::PrimaryTraceReceivers])
    {
        ensure!(
            program.inputs().len() == inputs(pass)?.len(),
            "original trace interface drift"
        );
    }
    let usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
    let mut meshes = BTreeMap::new();
    for b in &f.batches {
        let vertices =
            StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&b.vertices), usage)?;
        let indices = StorageBuffer::with_usage(&mut gpu, bytemuck::cast_slice(&b.indices), usage)?;
        let blas = gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
            vertices: vertices.buffer,
            vertex_count: (b.vertices.len() / 3) as u32,
            vertex_stride: 48,
            indices: indices.buffer,
            index_count: b.indices.len() as u32,
        })?;
        meshes.insert(
            b.index,
            Mesh {
                vertices,
                indices,
                blas,
            },
        );
    }
    let decoy = Scene::new(
        &mut gpu,
        vec![AccelerationInstance {
            geometry: meshes[&3].blas,
            transform: [1., 0., 0., 4096., 0., 1., 0., 0., 0., 0., 1., 0.],
            custom_index: 99,
            mask: 255,
        }],
    )?;
    let scene = Scene::new(
        &mut gpu,
        f.instances
            .iter()
            .map(|i| AccelerationInstance {
                geometry: meshes[&i.owner].blas,
                transform: i.transform,
                custom_index: i.custom_index,
                mask: i.mask,
            })
            .collect(),
    )?;
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let vertex_array: Vec<_> = (0..renderlab::TRACED_DRAWS)
        .map(|i| meshes.get(&i).map_or(&dummy, |m| &m.vertices))
        .collect();
    let index_array: Vec<_> = (0..renderlab::TRACED_DRAWS)
        .map(|i| meshes.get(&i).map_or(&dummy, |m| &m.indices))
        .collect();
    let models = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.models))?;
    let normals = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.normals))?;
    let frames: Vec<_> = fixtures
        .iter()
        .map(|f| StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&with_zones(&f.frame))))
        .collect::<Result<_>>()?;
    let placements: Vec<_> = fixtures
        .iter()
        .map(|f| {
            let draws: Vec<_> = f
                .batches
                .iter()
                .map(|b| (b.index, b.indices.len(), false))
                .collect();
            StorageBuffer::new(
                &mut gpu,
                bytemuck::cast_slice(&placement_words(&f.placements, &draws)),
            )
        })
        .collect::<Result<_>>()?;
    let mut images = BTreeMap::new();
    for (binding, format, size) in [
        (13, Format::Rgba32Uint, f.size),
        (16, Format::Rgba32Float, f.size),
        (17, Format::Rgba32Float, f.size),
        (20, Format::R32Uint, f.dispatch()[..2].try_into()?),
        (23, Format::R32Float, f.size),
    ] {
        images.insert(
            key(0, binding),
            Texture::new(&mut gpu, size, format, true, false)?,
        );
    }
    let jobs = [
        (0usize, 0usize, Entry::Trace),
        (0, 1, Entry::Receivers),
        (1, 0, Entry::Trace),
        (1, 1, Entry::Receivers),
    ];
    let mut packets = Vec::new();
    for (mode, which, _) in jobs {
        let program = &programs[which];
        let values: Vec<_> = program
            .inputs()
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match spec.key {
                        InputKey {
                            group: 0,
                            binding: 10,
                        } => Input::Buffer(&frames[mode]),
                        InputKey {
                            group: 0,
                            binding: 19,
                        } => Input::Scene(&scene),
                        InputKey { group: 0, .. } => Input::Image(&images[&spec.key]),
                        InputKey {
                            group: 1,
                            binding: 0,
                        } => Input::Buffer(&models),
                        InputKey {
                            group: 1,
                            binding: 1,
                        } => Input::Buffer(&normals),
                        InputKey {
                            group: 1,
                            binding: 2,
                        } => Input::Buffers(&vertex_array),
                        InputKey {
                            group: 1,
                            binding: 3,
                        } => Input::Buffers(&index_array),
                        InputKey {
                            group: 1,
                            binding: 5,
                        } => Input::Buffer(&placements[mode]),
                        _ => anyhow::bail!("unknown trace input {}", spec.name),
                    },
                ))
            })
            .collect::<Result<_>>()?;
        packets.push(HeapBindings::for_program(&mut gpu, program, &values)?);
    }
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    for b in frames
        .iter()
        .chain(placements.iter())
        .chain([&models, &normals, &dummy])
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    for mesh in meshes.values() {
        for b in [&mesh.vertices, &mesh.indices] {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
        graph.node(
            &mut rec,
            &[
                (Resource::Buffer(mesh.vertices.buffer), Access::AsInput),
                (Resource::Buffer(mesh.indices.buffer), Access::AsInput),
                (Resource::AccelerationStructure(mesh.blas), Access::AsBuild),
            ],
        )?;
        rec.push(Command::BuildAccelerationStructure(mesh.blas));
    }
    for scene in [&decoy, &scene] {
        let mut uses: Vec<_> = meshes
            .values()
            .map(|m| (Resource::AccelerationStructure(m.blas), Access::AsInput))
            .collect();
        uses.push((
            Resource::AccelerationStructure(scene.acceleration),
            Access::AsBuild,
        ));
        graph.node(&mut rec, &uses)?;
        rec.push(Command::BuildAccelerationStructure(scene.acceleration));
    }
    let mut staging = Vec::new();
    let mut snapshots = Vec::new();
    let mut last = None;
    for (job, (mode, which, entry)) in jobs.into_iter().enumerate() {
        let f = &fixtures[mode];
        let initial = &f.initial;
        for (binding, bytes) in [
            (13, bytemuck::cast_slice(initial.hit.as_slice())),
            (16, bytemuck::cast_slice(initial.world.as_slice())),
            (17, bytemuck::cast_slice(initial.normal.as_slice())),
            (20, bytemuck::cast_slice(initial.coverage.as_slice())),
            (23, bytemuck::cast_slice(initial.epsilon.as_slice())),
        ] {
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                images[&key(0, binding)].image,
                bytes,
            )?);
        }
        packets[job].import(&mut graph);
        kernels[which].record(
            &mut graph,
            &mut rec,
            &packets[job],
            [f.size[0], f.size[1], 1],
        )?;
        let n = u64::from(f.size[0]) * u64::from(f.size[1]);
        let mut reads = Vec::new();
        for (binding, bytes) in [
            (13, n * 16),
            (23, n * 4),
            (16, n * 16),
            (17, n * 16),
            (
                20,
                u64::from(f.dispatch()[0]) * u64::from(f.dispatch()[1]) * 4,
            ),
        ] {
            reads.push(readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                images[&key(0, binding)].image,
                bytes,
            )?);
        }
        snapshots.push((mode, entry, reads));
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("four queued camera traces"))?;
    for (mode, entry, reads) in snapshots {
        let words = |read| -> Result<Vec<u32>> {
            Ok(gpu
                .read(read)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes(*w))
                .collect())
        };
        let hit = words(reads[0])?.as_chunks::<4>().0.to_vec();
        let epsilon = words(reads[1])?.into_iter().map(f32::from_bits).collect();
        let rgba = |read| -> Result<Vec<[f32; 4]>> {
            Ok(words(read)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p.map(f32::from_bits))
                .collect())
        };
        let actual = Outputs {
            hit,
            epsilon,
            world: rgba(reads[2])?,
            normal: rgba(reads[3])?,
            coverage: words(reads[4])?,
        };
        fixtures[mode].assert_matches(entry, &actual);
        println!(
            "primary trace mode{mode} {entry:?}: exact identities/coverage/crop/untouched outputs; mesh-derived positions, normal/thin and nonfloor epsilon match CPU"
        );
        for read in reads {
            gpu.retire(Resource::Buffer(read))?;
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
    for mesh in meshes.into_values() {
        gpu.retire(Resource::AccelerationStructure(mesh.blas))?;
        mesh.vertices.retire(&mut gpu)?;
        mesh.indices.retire(&mut gpu)?;
    }
    for b in frames
        .into_iter()
        .chain(placements)
        .chain([models, normals, dummy])
    {
        b.retire(&mut gpu)?;
    }
    for t in images.into_values() {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
