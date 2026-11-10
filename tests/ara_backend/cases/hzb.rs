//! Exact conservative min-depth pyramid, padding, ranges and queued rebuilds.
use super::*;
use renderlab::backend_ara::{buffer::StorageBuffer, hzb::Hzb};
fn visibility(size: [u32; 2], frame: u32) -> Vec<[u32; 2]> {
    (0..size[0] * size[1])
        .map(|i| {
            let z = if (i + frame * 3) % 29 == 0 {
                0.
            } else {
                0.05 + ((i * 17 + frame * 11) % 97) as f32 * 0.009
            };
            [0xdead0000 | i, z.to_bits()]
        })
        .collect()
}
fn reference(size: [u32; 2], source: &[[u32; 2]]) -> Vec<f32> {
    let mut extent = size.map(|n| n.div_ceil(2).next_power_of_two());
    let mut level = Vec::new();
    for y in 0..extent[1] {
        for x in 0..extent[0] {
            let mut min = 1f32;
            for j in 0..2 {
                for i in 0..2 {
                    let p = [x * 2 + i, y * 2 + j];
                    let z = if p[0] < size[0] && p[1] < size[1] {
                        f32::from_bits(source[(p[1] * size[0] + p[0]) as usize][1])
                    } else {
                        0.
                    };
                    min = min.min(z);
                }
            }
            level.push(min);
        }
    }
    let mut all = level.clone();
    while extent != [1, 1] {
        let next = extent.map(|n| (n / 2).max(1));
        let mut values = Vec::new();
        for y in 0..next[1] {
            for x in 0..next[0] {
                let mut min = 1f32;
                for j in 0..2 {
                    for i in 0..2 {
                        min = min.min(
                            level[((y * 2 + j).min(extent[1] - 1) * extent[0]
                                + (x * 2 + i).min(extent[0] - 1))
                                as usize],
                        );
                    }
                }
                values.push(min);
            }
        }
        all.extend_from_slice(&values);
        level = values;
        extent = next;
    }
    all
}
#[test]
fn hzb_mip_chain_native_replay_against_cpu() -> Result<()> {
    let path = root("hzb");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let size = [17, 11];
    let first = visibility(size, 0);
    let source = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&first))?;
    let hzb = Hzb::new(&mut gpu, &cache, size)?;
    let inputs = hzb.inputs(&mut gpu, &source)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(source.buffer), Access::HostWrite);
    hzb.import(&mut graph);
    inputs.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    ensure!(
        graph
            .node_uses(&mut rec, &[hzb.levels()[0].usage(Access::ShaderRead)])
            .is_err(),
        "uninitialized mip read accepted"
    );
    ensure!(
        graph
            .require_initialized(Resource::Image(hzb.texture().image))
            .is_err(),
        "failed node mutated initialization"
    );
    ensure!(
        graph
            .node_uses(
                &mut rec,
                &[
                    hzb.levels()[0].usage(Access::Storage),
                    hzb.levels()[0].usage(Access::ShaderRead)
                ]
            )
            .is_err(),
        "overlapping mip alias accepted"
    );
    let mut invalid = hzb.levels()[0];
    invalid.view.base_mip = hzb.texture().allocation.mips;
    ensure!(
        graph
            .node_uses(&mut rec, &[invalid.usage(Access::Storage)])
            .is_err(),
        "out of range mip accepted"
    );
    let bytes = hzb
        .levels()
        .iter()
        .map(|v| u64::from(v.size[0]) * u64::from(v.size[1]) * 4)
        .sum();
    let mut snapshots = Vec::new();
    let mut final_point = None;
    for frame in 0..2 {
        let data = visibility(size, frame);
        if frame != 0 {
            graph.node(
                &mut rec,
                &[(Resource::Buffer(source.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: source.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(&data).to_vec(),
            });
        }
        hzb.record(&mut graph, &mut rec, &inputs)?;
        graph.require_initialized(Resource::Image(hzb.texture().image))?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            hzb.texture().image,
            bytes,
        )?;
        snapshots.push((frame, read, reference(size, &data)));
        final_point = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(final_point.expect("two submissions"))?;
    for (frame, read, expected) in snapshots {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        ensure!(actual == expected, "HZB frame{frame} differs");
        println!(
            "HZB frame{frame}:{} exact texels across{} mips",
            actual.len(),
            hzb.levels().len()
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    inputs.retire(&mut gpu, &mut graph)?;
    hzb.retire(&mut gpu, &mut graph)?;
    source.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
