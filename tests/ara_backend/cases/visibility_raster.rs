//! Packed cluster-ID raster replays against independent integer coverage.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    post_surface::RasterDepth,
    shader::{InputKey, inputs},
    transfer::readback_buffer,
    visibility_raster::{HardwareClusters, VisibilityHardware},
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
#[test]
fn software_and_hardware_cluster_raster_native_replay() -> Result<()> {
    let f = visibility_sw_reference::fixture();
    let path = root("visibility-raster");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let software = Kernel::new(&mut gpu, &cache, Pass::VisibilitySoftware)?;
    let hardware = VisibilityHardware::new(&mut gpu, &cache)?;
    let depth = RasterDepth::new(&mut gpu, f.viewport, ara_gpu::texture::SampleCount::One)?;
    let mut buffers = BTreeMap::new();
    for (key, bytes) in [
        (key(0, 0), bytemuck::cast_slice(f.config.as_slice())),
        (key(0, 1), bytemuck::cast_slice(f.clusters.as_slice())),
        (key(0, 3), bytemuck::cast_slice(f.lists.as_slice())),
        (key(0, 5), bytemuck::cast_slice(f.args.as_slice())),
        (key(0, 8), bytemuck::cast_slice(f.visibility.as_slice())),
        (key(1, 0), bytemuck::cast_slice(f.models.as_slice())),
        (key(1, 1), bytemuck::cast_slice(f.normals.as_slice())),
        (key(1, 5), bytemuck::cast_slice(f.placements.as_slice())),
    ] {
        buffers.insert(
            key,
            if key == self::key(0, 5) {
                StorageBuffer::with_usage(
                    &mut gpu,
                    &[0; 128],
                    vk::BufferUsageFlags::INDIRECT_BUFFER,
                )?
            } else {
                StorageBuffer::new(&mut gpu, bytes)?
            },
        );
    }
    let argument_source = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.args))?;
    let dummy = StorageBuffer::new(&mut gpu, &[0; 16])?;
    let mut batches = BTreeMap::new();
    let mut gaps = Vec::new();
    for batch in &f.batches {
        batches.insert(
            batch.index,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&batch.words))?,
        );
        gaps.push(StorageBuffer::new(&mut gpu, &[0; 16])?);
    }
    let array: Vec<_> = (0..renderlab::TRACED_DRAWS)
        .map(|i| batches.get(&i).unwrap_or(&dummy))
        .collect();
    let mut packets = Vec::new();
    for pass in [Pass::VisibilitySoftware, Pass::VisibilityHardware] {
        let values: Vec<_> = inputs(pass)?
            .iter()
            .map(|spec| {
                (
                    spec.key,
                    if spec.key == key(2, 8) {
                        Input::Buffers(&array)
                    } else {
                        Input::Buffer(&buffers[&spec.key])
                    },
                )
            })
            .collect();
        packets.push(HeapBindings::new(&mut gpu, pass, &values)?);
    }
    let output = &buffers[&key(0, 8)];
    let mut graph = Graph::default();
    for b in buffers
        .values()
        .chain(batches.values())
        .chain(std::iter::once(&dummy))
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    for packet in &packets {
        packet.import(&mut graph);
    }
    graph.import(Resource::Buffer(argument_source.buffer), Access::HostWrite);
    let arguments = &buffers[&key(0, 5)];
    let mut copy = gpu.begin(Queue::Graphics);
    graph.node(
        &mut copy,
        &[
            (Resource::Buffer(argument_source.buffer), Access::CopySource),
            (Resource::Buffer(arguments.buffer), Access::CopyDestination),
        ],
    )?;
    copy.push(Command::CopyBufferRange {
        source: argument_source.buffer,
        source_offset: 0,
        destination: arguments.buffer,
        destination_offset: 0,
        size: 128,
    });
    gpu.submit(copy.finish(), &[])?;
    let scene = HardwareClusters {
        inputs: &packets[1],
        count: f.clusters.len() as u32,
    };
    let mut rejected = gpu.begin(Queue::Graphics);
    ensure!(
        hardware
            .record_indirect(&mut graph, &mut rejected, &scene, &depth, arguments, false)
            .is_err(),
        "uninitialized depth load accepted"
    );
    let mut snapshots = Vec::new();
    let mut last = None;
    for index in 0..4 {
        let packet = &packets[index % 2];
        let mut rec = gpu.begin(Queue::Graphics);
        if index != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(output.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: output.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(&f.visibility).to_vec(),
            });
        }
        match index {
            0 => software.record(
                &mut graph,
                &mut rec,
                packet,
                [f.dispatch[0] * 128, f.dispatch[1], f.dispatch[2]],
            )?,
            1 => hardware.record(
                &mut graph,
                &mut rec,
                &scene,
                &depth,
                f.hardware_args[1],
                true,
            )?,
            2 => software.record_indirect(&mut graph, &mut rec, packet, arguments, 0)?,
            _ => {
                hardware.record_indirect(&mut graph, &mut rec, &scene, &depth, arguments, false)?
            }
        }
        snapshots.push(readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.buffer,
            0,
            output.len,
        )?);
        last = Some(gpu.submit(rec.finish(), &[])?);
    }
    gpu.wait(last.expect("four queued rasters"))?;
    for (index, read) in snapshots.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|v| u64::from_le_bytes(*v))
            .collect();
        f.assert_matches(&actual, index % 2 != 0);
        println!(
            "visibility raster{index}: exact packed depths/IDs and guard words over {} pixels",
            f.viewport[0] * f.viewport[1]
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    argument_source.retire(&mut gpu)?;
    for p in packets {
        p.retire(&mut gpu, &mut graph)?;
    }
    depth.retire(&mut gpu, &mut graph)?;
    for b in buffers
        .into_values()
        .chain(batches.into_values())
        .chain(gaps)
        .chain(std::iter::once(dummy))
    {
        b.retire(&mut gpu)?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
