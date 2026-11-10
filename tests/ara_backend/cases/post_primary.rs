//! Primary-hit post motion, with three buffer arrays and queued parameter updates.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::{InputKey, inputs},
    transfer::readback,
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
struct Batch {
    vertices: StorageBuffer,
    indices: StorageBuffer,
    previous: Option<StorageBuffer>,
}
#[test]
fn hit_based_post_velocity_depth_and_history_native_replay() -> Result<()> {
    let f = post_primary_reference::fixture();
    let path = root("post-primary");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let kernel = Kernel::new(&mut gpu, &cache, Pass::PostPrimarySurface)?;
    let mut buffers = BTreeMap::new();
    for (key, bytes) in [
        (key(0, 10), bytemuck::cast_slice(f.primary_frame.as_slice())),
        (
            key(0, 40),
            bytemuck::cast_slice(f.post_frames[0].as_slice()),
        ),
        (key(1, 0), bytemuck::cast_slice(f.models.as_slice())),
        (key(1, 5), bytemuck::cast_slice(f.placements.as_slice())),
    ] {
        buffers.insert(key, StorageBuffer::new(&mut gpu, bytes)?);
    }
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let mut batches = BTreeMap::new();
    for b in &f.batches {
        batches.insert(
            b.index,
            Batch {
                vertices: StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.vertices))?,
                indices: StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.indices))?,
                previous: if b.vertices == b.previous_vertices {
                    None
                } else {
                    Some(StorageBuffer::new(
                        &mut gpu,
                        bytemuck::cast_slice(&b.previous_vertices),
                    )?)
                },
            },
        );
    }
    let arrays: Vec<Vec<_>> = (0..3)
        .map(|which| {
            (0..renderlab::TRACED_DRAWS)
                .map(|index| {
                    batches.get(&index).map_or(&dummy, |b| match which {
                        0 => &b.vertices,
                        1 => &b.indices,
                        _ => b.previous.as_ref().unwrap_or(&b.vertices),
                    })
                })
                .collect()
        })
        .collect();
    let mut images = BTreeMap::new();
    for (binding, size, format, storage) in [
        (7, f.internal, Format::Rgba32Uint, false),
        (21, f.coverage_extent, Format::R32Uint, false),
        (41, f.output, Format::Rgba32Float, true),
    ] {
        images.insert(
            key(0, binding),
            Texture::new(&mut gpu, size, format, storage, false)?,
        );
    }
    let layout = inputs(Pass::PostPrimarySurface)?;
    ensure!(layout.len() == 10, "unexpected post-primary interface");
    let values: Vec<_> = layout
        .iter()
        .map(|s| {
            (
                s.key,
                match s.key {
                    InputKey {
                        group: 1,
                        binding: 2,
                    } => Input::Buffers(&arrays[0]),
                    InputKey {
                        group: 1,
                        binding: 3,
                    } => Input::Buffers(&arrays[1]),
                    InputKey {
                        group: 1,
                        binding: 6,
                    } => Input::Buffers(&arrays[2]),
                    k if images.contains_key(&k) => Input::Image(&images[&k]),
                    k => Input::Buffer(&buffers[&k]),
                },
            )
        })
        .collect();
    let packet = HeapBindings::new(&mut gpu, Pass::PostPrimarySurface, &values)?;
    let mut graph = Graph::default();
    for b in buffers.values().chain(std::iter::once(&dummy)) {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    for b in batches.values() {
        for record in [&b.vertices, &b.indices]
            .into_iter()
            .chain(b.previous.as_ref())
        {
            graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
        }
    }
    packet.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();
    for (binding, bytes) in [
        (7, bytemuck::cast_slice(f.hit.as_slice())),
        (21, bytemuck::cast_slice(f.coverage.as_slice())),
    ] {
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            images[&key(0, binding)].image,
            bytes,
        )?);
    }
    let output = &images[&key(0, 41)];
    let post = &buffers[&key(0, 40)];
    let mut snapshots = Vec::new();
    let mut last = None;
    ensure!(
        f.dispatch == [f.output[0].div_ceil(8), f.output[1].div_ceil(8), 1],
        "fixture dispatch differs"
    );
    for (frame, params) in f.post_frames.iter().enumerate() {
        if frame != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(post.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: post.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(params.as_slice()).to_vec(),
            });
        }
        kernel.record(&mut graph, &mut rec, &packet, [f.output[0], f.output[1], 1])?;
        snapshots.push(readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(f.output[0]) * u64::from(f.output[1]) * 16,
        )?);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("four queued post histories"))?;
    for (frame, read) in snapshots.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<[f32; 4]> = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|bytes| bytemuck::pod_read_unaligned(bytes))
            .collect();
        let expected = &f.expected[frame];
        ensure!(actual.len() == expected.len(), "post image length differs");
        let mut error = 0f32;
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            for c in 0..4 {
                ensure!(actual[c].is_finite(), "nonfinite post output");
                let diff = (actual[c] - expected[c]).abs();
                error = error.max(diff);
                ensure!(
                    diff <= 2e-5,
                    "post {} pixel{i} component{c}: {} != {}",
                    f.frame_names[frame],
                    actual[c],
                    expected[c]
                );
            }
            assert_eq!(actual[3], 1.);
            if expected[2] == 1e8 {
                assert_eq!(actual[2], expected[2]);
            }
            if frame >= 2 {
                assert_eq!(&actual[..2], &[0., 0.]);
            }
        }
        println!("post primary {}: max error {error}", f.frame_names[frame]);
        gpu.retire(Resource::Buffer(read))?;
    }
    packet.retire(&mut gpu, &mut graph)?;
    kernel.retire(&mut gpu)?;
    for b in buffers.into_values().chain(std::iter::once(dummy)) {
        b.retire(&mut gpu)?;
    }
    for b in batches.into_values() {
        b.vertices.retire(&mut gpu)?;
        b.indices.retire(&mut gpu)?;
        if let Some(previous) = b.previous {
            previous.retire(&mut gpu)?;
        }
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
