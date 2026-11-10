//! Original receiver resolve with stale regions and source-triangle geometry.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::{InputKey, inputs},
    transfer::{readback, readback_buffer},
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
fn read_words<T: bytemuck::Pod>(gpu: &GpuDevice, buffer: ara_gpu::Buffer) -> Result<Vec<T>> {
    Ok(gpu
        .read(buffer)?
        .chunks_exact(std::mem::size_of::<T>())
        .map(bytemuck::pod_read_unaligned)
        .collect())
}
#[test]
fn source_triangle_receiver_resolve_native_replay_against_cpu() -> Result<()> {
    let f = visibility_resolve_reference::fixture();
    let path = root("visibility-resolve");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let kernel = Kernel::new(&mut gpu, &cache, Pass::VisibilityResolve)?;
    let mut buffers = BTreeMap::new();
    for (key, bytes) in [
        (key(0, 10), bytemuck::cast_slice(f.primary_frame.as_slice())),
        (key(0, 29), bytemuck::cast_slice(f.visibility.as_slice())),
        (key(0, 30), bytemuck::cast_slice(f.repair_scan.as_slice())),
        (key(0, 31), bytemuck::cast_slice(f.clusters.as_slice())),
        (key(0, 33), bytemuck::cast_slice(f.groups.as_slice())),
        (key(1, 0), bytemuck::cast_slice(f.models.as_slice())),
        (key(1, 1), bytemuck::cast_slice(f.normals.as_slice())),
        (key(1, 5), bytemuck::cast_slice(f.placements.as_slice())),
    ] {
        buffers.insert(key, StorageBuffer::new(&mut gpu, bytes)?);
    }
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let mut batches = BTreeMap::new();
    for batch in &f.batches {
        batches.insert(
            batch.index,
            [
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&batch.vertices))?,
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&batch.indices))?,
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&batch.words))?,
            ],
        );
    }
    let arrays: Vec<Vec<_>> = (0..3)
        .map(|which| {
            (0..64)
                .map(|i| batches.get(&i).map_or(&dummy, |b| &b[which]))
                .collect()
        })
        .collect();
    let mut images = BTreeMap::new();
    for (binding, size, format) in [
        (13, f.viewport, Format::Rgba32Uint),
        (16, f.viewport, Format::Rgba32Float),
        (17, f.viewport, Format::Rgba32Float),
        (20, f.tile_size, Format::R32Uint),
        (23, f.viewport, Format::R32Float),
    ] {
        images.insert(
            key(0, binding),
            Texture::new(&mut gpu, size, format, true, false)?,
        );
    }
    let layout = inputs(Pass::VisibilityResolve)?;
    let values: Vec<_> = layout
        .iter()
        .map(|spec| {
            let value = match spec.key {
                InputKey {
                    group: 1,
                    binding: 2,
                } => Input::Buffers(&arrays[0]),
                InputKey {
                    group: 1,
                    binding: 3,
                } => Input::Buffers(&arrays[1]),
                InputKey {
                    group: 2,
                    binding: 8,
                } => Input::Buffers(&arrays[2]),
                k if images.contains_key(&k) => Input::Image(&images[&k]),
                k => Input::Buffer(&buffers[&k]),
            };
            (spec.key, value)
        })
        .collect();
    let packet = HeapBindings::new(&mut gpu, Pass::VisibilityResolve, &values)?;
    let mut graph = Graph::default();
    for b in buffers
        .values()
        .chain(batches.values().flatten())
        .chain(std::iter::once(&dummy))
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    packet.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();
    for (binding, bytes) in [
        (13, bytemuck::cast_slice(f.initial.hit.as_slice())),
        (16, bytemuck::cast_slice(f.initial.world.as_slice())),
        (17, bytemuck::cast_slice(f.initial.normal.as_slice())),
        (20, bytemuck::cast_slice(f.initial.tiles.as_slice())),
        (23, bytemuck::cast_slice(f.initial.epsilon.as_slice())),
    ] {
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            images[&key(0, binding)].image,
            bytes,
        )?);
    }
    let visibility = &buffers[&key(0, 29)];
    let mut snapshots = Vec::new();
    let mut last = None;
    for frame in 0..2 {
        if frame != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(visibility.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: visibility.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(&f.visibility).to_vec(),
            });
        }
        kernel.record(
            &mut graph,
            &mut rec,
            &packet,
            [f.viewport[0], f.viewport[1], 1],
        )?;
        let mut reads = Vec::new();
        for (binding, stride) in [(13, 16), (16, 16), (17, 16), (23, 4), (20, 4)] {
            let image = &images[&key(0, binding)];
            reads.push(readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                image.image,
                u64::from(image.size[0]) * u64::from(image.size[1]) * stride,
            )?);
        }
        reads.push(readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            visibility.buffer,
            0,
            visibility.len,
        )?);
        snapshots.push(reads);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two queued resolves"))?;
    for (frame, reads) in snapshots.into_iter().enumerate() {
        let actual = visibility_resolve_reference::Outputs {
            hit: read_words(&gpu, reads[0])?,
            world: read_words(&gpu, reads[1])?,
            normal: read_words(&gpu, reads[2])?,
            epsilon: read_words(&gpu, reads[3])?,
            tiles: read_words(&gpu, reads[4])?,
        };
        f.assert_matches(&actual, &read_words::<u64>(&gpu, reads[5])?);
        let mut error = [0f32; 3];
        for (i, written) in f.expected.receiver_written.iter().enumerate() {
            if !written {
                continue;
            }
            for c in 2..4 {
                error[0] = error[0].max(
                    (f32::from_bits(actual.hit[i][c])
                        - f32::from_bits(f.expected.outputs.hit[i][c]))
                    .abs(),
                );
            }
            for c in 0..3 {
                error[1] =
                    error[1].max((actual.world[i][c] - f.expected.outputs.world[i][c]).abs());
                error[2] =
                    error[2].max((actual.normal[i][c] - f.expected.outputs.normal[i][c]).abs());
            }
        }
        println!(
            "visibility resolve{frame}: bary/world/normal max errors {error:?}; exact IDs, epsilon, tiles {:?} and stale regions",
            actual.tiles
        );
        for read in reads {
            gpu.retire(Resource::Buffer(read))?;
        }
    }
    packet.retire(&mut gpu, &mut graph)?;
    kernel.retire(&mut gpu)?;
    for b in buffers
        .into_values()
        .chain(batches.into_values().flatten())
        .chain(std::iter::once(dummy))
    {
        b.retire(&mut gpu)?;
    }
    for t in images.into_values() {
        graph.forget(Resource::Image(t.image));
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
