//! Production geometric queue checks, including exact canonicalization and fallbacks.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    primary_check::PrimaryCheck,
    shader::{InputKey, inputs},
    transfer::readback_buffer,
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
#[test]
fn primary_geometry_check_retry_queue_native_replay() -> Result<()> {
    let f = primary_check_reference::fixture();
    let path = root("primary-check");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = PrimaryCheck::new(&mut gpu, &cache)?;
    let mut buffers = BTreeMap::new();
    for (key, bytes) in [
        (key(0, 10), bytemuck::cast_slice(f.frame.as_slice())),
        (key(0, 24), bytemuck::cast_slice(f.repairs.as_slice())),
        (key(0, 28), bytemuck::cast_slice(f.retries.as_slice())),
        (key(1, 0), bytemuck::cast_slice(f.models.as_slice())),
        (key(1, 1), bytemuck::cast_slice(f.normals.as_slice())),
        (key(1, 5), bytemuck::cast_slice(f.placements.as_slice())),
    ] {
        buffers.insert(key, StorageBuffer::new(&mut gpu, bytes)?);
    }
    let mut batches = BTreeMap::new();
    for b in &f.batches {
        batches.insert(
            b.index,
            [
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.vertices))?,
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.indices))?,
            ],
        );
    }
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let arrays: Vec<Vec<_>> = (0..2)
        .map(|which| {
            (0..64)
                .map(|i| batches.get(&i).map_or(&dummy, |b| &b[which]))
                .collect()
        })
        .collect();
    let layout = inputs(Pass::PrimaryCheck)?;
    ensure!(layout.len() == 8, "unexpected geometry-check interface");
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
                    k => Input::Buffer(&buffers[&k]),
                },
            )
        })
        .collect();
    let packet = HeapBindings::new(&mut gpu, Pass::PrimaryCheck, &values)?;
    let dispatch = [f.repairs[3].div_ceil(32), 1, 1];
    let argument_source = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&dispatch))?;
    let arguments =
        StorageBuffer::with_usage(&mut gpu, &[0; 12], vk::BufferUsageFlags::INDIRECT_BUFFER)?;
    let mut graph = Graph::default();
    for b in buffers.values().chain(batches.values().flatten()).chain([
        &dummy,
        &argument_source,
        &arguments,
    ]) {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    packet.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    graph.node(
        &mut rec,
        &[
            (Resource::Buffer(argument_source.buffer), Access::CopySource),
            (Resource::Buffer(arguments.buffer), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::CopyBufferRange {
        source: argument_source.buffer,
        source_offset: 0,
        destination: arguments.buffer,
        destination_offset: 0,
        size: 12,
    });
    let repairs = &buffers[&key(0, 24)];
    let retries = &buffers[&key(0, 28)];
    let mut snapshots = Vec::new();
    let mut last = None;
    for frame in 0..2 {
        if frame != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(repairs.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: repairs.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(&f.repairs).to_vec(),
            });
        }
        if frame == 0 {
            pass.record(&mut graph, &mut rec, &packet, f.dispatch)?;
        } else {
            pass.record_indirect(&mut graph, &mut rec, &packet, &arguments)?;
        }
        let a = readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            repairs.buffer,
            0,
            repairs.len,
        )?;
        let b = readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            retries.buffer,
            0,
            retries.len,
        )?;
        snapshots.push((a, b));
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two queued geometric checks"))?;
    for (frame, (a, b)) in snapshots.into_iter().enumerate() {
        let words = |buffer| -> Result<Vec<u32>> {
            Ok(gpu
                .read(buffer)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| u32::from_le_bytes(*v))
                .collect())
        };
        f.assert_matches(&words(a)?, &words(b)?);
        println!(
            "primary geometry check{frame}: exact canonical primitive/barycentric, fallback flags, retry header/set and canaries"
        );
        gpu.retire(Resource::Buffer(a))?;
        gpu.retire(Resource::Buffer(b))?;
    }
    packet.retire(&mut gpu, &mut graph)?;
    pass.retire(&mut gpu)?;
    for b in buffers
        .into_values()
        .chain(batches.into_values().flatten())
        .chain([dummy, argument_source, arguments])
    {
        b.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
