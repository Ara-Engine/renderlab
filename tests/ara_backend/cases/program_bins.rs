//! Atomic append lists are checked semantically, independently of scheduling order.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    program_bins::{BinDispatch, ProgramBins},
    shader::InputKey,
    transfer::readback_buffer,
};
fn key(binding: u32) -> InputKey {
    InputKey { group: 0, binding }
}
#[test]
fn program_tile_bins_native_replay_against_cpu() -> Result<()> {
    let f = program_bins_reference::fixture();
    let path = root("program-bins");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = ProgramBins::new(&mut gpu, &cache)?;
    let hit = Texture::new(&mut gpu, f.extent, Format::Rgba32Uint, false, false)?;
    let coverage = Texture::new(&mut gpu, f.coverage_extent, Format::R32Uint, false, false)?;
    let buffers = [
        (
            key(2),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.region))?,
        ),
        (
            key(3),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.placements))?,
        ),
        (
            key(4),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.list))?,
        ),
        (
            key(5),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.entries))?,
        ),
        (
            key(6),
            StorageBuffer::with_usage(
                &mut gpu,
                bytemuck::cast_slice(&f.args),
                vk::BufferUsageFlags::INDIRECT_BUFFER,
            )?,
        ),
    ];
    let mut values: Vec<_> = buffers
        .iter()
        .map(|(k, b)| (*k, Input::Buffer(b)))
        .collect();
    values.extend([
        (key(0), Input::Image(&hit)),
        (key(1), Input::Image(&coverage)),
    ]);
    let inputs = HeapBindings::new(&mut gpu, Pass::ProgramBins, &values)?;
    let mut graph = Graph::default();
    for (_, b) in &buffers {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    inputs.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = [
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            hit.image,
            bytemuck::cast_slice(&f.hit),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            coverage.image,
            bytemuck::cast_slice(&f.coverage),
        )?,
    ];
    let mut snapshots = Vec::new();
    let mut final_point = None;
    for frame in 0..2 {
        pass.record(
            &mut graph,
            &mut rec,
            &inputs,
            &BinDispatch {
                slots: f.slots,
                capacity: f.list[2],
                groups: f.groups,
            },
        )?;
        let entries = readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            buffers[3].1.buffer,
            0,
            buffers[3].1.len,
        )?;
        let args = readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            buffers[4].1.buffer,
            0,
            buffers[4].1.len,
        )?;
        snapshots.push((frame, entries, args));
        final_point = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(final_point.expect("two submissions"))?;
    for (frame, entries, args) in snapshots {
        let read_words = |gpu: &GpuDevice, buffer| -> Result<Vec<u32>> {
            Ok(gpu
                .read(buffer)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| u32::from_le_bytes(*v))
                .collect())
        };
        let entries_words = read_words(&gpu, entries)?;
        let args_words = read_words(&gpu, args)?;
        f.assert_matches(&entries_words, args_words.as_chunks::<4>().0);
        println!(
            "program bins frame{frame}: exact counts, tile sets, indirect args and unused tails {:?}",
            f.expected.counts
        );
        gpu.retire(Resource::Buffer(entries))?;
        gpu.retire(Resource::Buffer(args))?;
    }
    inputs.retire(&mut gpu, &mut graph)?;
    for (_, b) in buffers {
        b.retire(&mut gpu)?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    for t in [hit, coverage] {
        gpu.retire(Resource::Image(t.image))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
