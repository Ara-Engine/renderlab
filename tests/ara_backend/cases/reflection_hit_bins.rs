//! Reflection hit binning (shaders/reflection_hit_bins.wgsl) on synthetic H records: each
//! program slot's bin holds exactly its hits' record indices, in any order, with indirect
//! arguments covering them in 64-lane groups.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    reflection_hits::{HitBinDispatch, HitBins},
    shader::InputKey,
    transfer::readback_buffer,
};
fn key(binding: u32) -> InputKey {
    InputKey { group: 0, binding }
}
#[test]
fn hit_records_bin_by_their_programs_slot() -> Result<()> {
    // 1,000 records over 10 placements; placement p is draw p, and draw d is program d % 3.
    const RECORDS: u32 = 1000;
    const SLOTS: u32 = 3;
    let placements: Vec<u32> = (0..10).collect();
    let mut h = vec![RECORDS, 0, 0, 0];
    let mut expected = vec![Vec::new(); SLOTS as usize];
    for i in 0..RECORDS {
        let placement = (i * 7 + i / 13) % 10;
        expected[(placement % SLOTS) as usize].push(i);
        // Word 3 is the hit placement plus one; the other words are not read by binning.
        h.extend([i, i, 0, placement + 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    let mut bins = vec![RECORDS + 1, 0, 0, 0];
    bins.extend((0..renderlab::TRACED_DRAWS).map(|d| d % SLOTS));
    let stride = RECORDS + 1;
    let path = root("reflection-hit-bins");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = HitBins::new(&mut gpu, &cache)?;
    let buffers = [
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&h))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&placements))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&bins))?,
        StorageBuffer::new(&mut gpu, &vec![0xffu8; (stride * SLOTS * 4) as usize])?,
        StorageBuffer::with_usage(
            &mut gpu,
            &vec![0xffu8; (SLOTS * 16) as usize],
            vk::BufferUsageFlags::INDIRECT_BUFFER,
        )?,
    ];
    let values: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| (key(i as u32), Input::Buffer(b)))
        .collect();
    let inputs = HeapBindings::new(&mut gpu, Pass::ReflectionHitBins, &values)?;
    let mut graph = Graph::default();
    for b in &buffers {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    inputs.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    pass.record(
        &mut graph,
        &mut rec,
        &inputs,
        &HitBinDispatch {
            slots: SLOTS,
            stride,
            groups: [RECORDS.div_ceil(64), 1, 1],
        },
    )?;
    let order = readback_buffer(
        &mut gpu,
        &mut graph,
        &mut rec,
        buffers[3].buffer,
        0,
        buffers[3].len,
    )?;
    let args = readback_buffer(
        &mut gpu,
        &mut graph,
        &mut rec,
        buffers[4].buffer,
        0,
        buffers[4].len,
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let words = |gpu: &GpuDevice, buffer| -> Result<Vec<u32>> {
        Ok(gpu
            .read(buffer)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v))
            .collect())
    };
    let (order_words, args_words) = (words(&gpu, order)?, words(&gpu, args)?);
    for slot in 0..SLOTS as usize {
        let base = slot * stride as usize;
        let n = order_words[base] as usize;
        let mut got = order_words[base + 1..base + 1 + n].to_vec();
        got.sort_unstable();
        assert_eq!(got, expected[slot], "slot {slot}'s bin");
        let groups = (n as u32).div_ceil(64);
        assert_eq!(
            &args_words[slot * 4..slot * 4 + 3],
            [groups, 1, 1],
            "slot {slot}'s arguments"
        );
    }
    println!(
        "reflection hit bins: {:?} records per slot, exact sets and arguments",
        expected.iter().map(Vec::len).collect::<Vec<_>>()
    );
    gpu.retire(Resource::Buffer(order))?;
    gpu.retire(Resource::Buffer(args))?;
    inputs.retire(&mut gpu, &mut graph)?;
    for b in buffers {
        b.retire(&mut gpu)?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
