//! Hit-derived opaque distances for transparency, including exact miss/negative values.
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
#[test]
fn transparency_hit_depth_native_replay_against_cpu() -> Result<()> {
    let f = transparency_hit_depth_reference::fixture();
    let source = &f.source;
    let path = root("transparency-hit-depth");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let kernel = Kernel::new(&mut gpu, &cache, Pass::TransparencyHitDepth)?;
    let mut buffers = BTreeMap::new();
    for (key, bytes) in [
        (key(0, 30), bytemuck::cast_slice(f.frames[0].as_slice())),
        (key(1, 0), bytemuck::cast_slice(source.models.as_slice())),
        (
            key(1, 5),
            bytemuck::cast_slice(source.placements.as_slice()),
        ),
    ] {
        buffers.insert(key, StorageBuffer::new(&mut gpu, bytes)?);
    }
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let mut batches = BTreeMap::new();
    for b in &source.batches {
        batches.insert(
            b.index,
            [
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.vertices))?,
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.indices))?,
            ],
        );
    }
    let arrays: Vec<Vec<_>> = (0..2)
        .map(|i| {
            (0..renderlab::TRACED_DRAWS)
                .map(|index| batches.get(&index).map_or(&dummy, |b| &b[i]))
                .collect()
        })
        .collect();
    let mut images = BTreeMap::new();
    for (binding, size, format, storage) in [
        (50, source.internal, Format::Rgba32Uint, false),
        (51, source.coverage_extent, Format::R32Uint, false),
        (52, source.internal, Format::R32Float, true),
    ] {
        images.insert(
            key(0, binding),
            Texture::new(&mut gpu, size, format, storage, false)?,
        );
    }
    let layout = inputs(Pass::TransparencyHitDepth)?;
    ensure!(
        layout.len() == 8,
        "unexpected transparency hit-depth interface"
    );
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
                    k if images.contains_key(&k) => Input::Image(&images[&k]),
                    k => Input::Buffer(&buffers[&k]),
                },
            )
        })
        .collect();
    let packet = HeapBindings::new(&mut gpu, Pass::TransparencyHitDepth, &values)?;
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
        (50, bytemuck::cast_slice(source.hit.as_slice())),
        (51, bytemuck::cast_slice(source.coverage.as_slice())),
    ] {
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            images[&key(0, binding)].image,
            bytes,
        )?);
    }
    let frame = &buffers[&key(0, 30)];
    let output = &images[&key(0, 52)];
    let mut snapshots = Vec::new();
    let mut last = None;
    for (i, params) in f.frames.iter().enumerate() {
        if i != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(frame.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: frame.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(params.as_slice()).to_vec(),
            });
        }
        kernel.record(
            &mut graph,
            &mut rec,
            &packet,
            [source.internal[0], source.internal[1], 1],
        )?;
        snapshots.push(readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(source.internal[0]) * u64::from(source.internal[1]) * 4,
        )?);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("four queued depth variants"))?;
    for (frame, read) in snapshots.into_iter().enumerate() {
        let data = gpu.read(read)?;
        let actual: Vec<_> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        f.assert_matches(frame, &actual);
        let error = actual
            .iter()
            .zip(&f.expected[frame])
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        println!(
            "transparency hit depth{frame}: max error {error}, exact misses {}",
            actual.iter().filter(|v| **v == 1e30).count()
        );
        gpu.retire(Resource::Buffer(read))?;
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
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
