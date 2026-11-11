//! Two-phase LOD/frustum/HZB culling, real indirect dispatch and working-state reset.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    hzb::Hzb,
    shader::InputKey,
    transfer::readback_buffer,
    visibility_cull::VisibilityCull,
};
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
fn snapshot(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    buffers: &[&StorageBuffer],
) -> Result<Vec<ara_gpu::Buffer>> {
    buffers
        .iter()
        .map(|b| readback_buffer(gpu, graph, rec, b.buffer, 0, b.len))
        .collect()
}
fn words(gpu: &GpuDevice, buffer: ara_gpu::Buffer) -> Result<Vec<u32>> {
    Ok(gpu
        .read(buffer)?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(*v))
        .collect())
}
#[test]
fn two_phase_cluster_cull_finish_reset_native_replay() -> Result<()> {
    let f = visibility_cull_reference::fixture();
    let path = root("visibility-cull");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = VisibilityCull::new(&mut gpu, &cache)?;
    let sources = [
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.clusters))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.groups))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.models))?,
    ];
    let configs = [
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.configs[0]))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.configs[1]))?,
    ];
    let lists = [
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lists[0]))?,
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lists[1]))?,
    ];
    let args = [
        StorageBuffer::with_usage(
            &mut gpu,
            bytemuck::cast_slice(&f.args[0]),
            vk::BufferUsageFlags::INDIRECT_BUFFER,
        )?,
        StorageBuffer::with_usage(
            &mut gpu,
            bytemuck::cast_slice(&f.args[1]),
            vk::BufferUsageFlags::INDIRECT_BUFFER,
        )?,
    ];
    let rejects = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.rejects))?;
    let pixels = (f.viewport[0] * f.viewport[1]) as usize;
    let old = vec![[0u32, f.hzb_depths[0].to_bits()]; pixels];
    let visibility = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&old))?;
    let hzb = Hzb::new(&mut gpu, &cache, f.viewport)?;
    ensure!(
        hzb.levels().len() == f.hzb_levels as usize,
        "wrong HZB levels"
    );
    let hzb_inputs = hzb.inputs(&mut gpu, &visibility)?;
    let mut packets = Vec::new();
    for i in 0..2 {
        let cull = HeapBindings::new(
            &mut gpu,
            Pass::VisibilityCull,
            &[
                (key(0, 0), Input::Buffer(&configs[i])),
                (key(0, 1), Input::Buffer(&sources[0])),
                (key(0, 2), Input::Buffer(&sources[1])),
                (key(0, 3), Input::Buffer(&lists[i])),
                (key(0, 5), Input::Buffer(&args[i])),
                (key(0, 6), Input::Buffer(&rejects)),
                (key(0, 7), Input::Image(hzb.texture())),
                (key(1, 0), Input::Buffer(&sources[2])),
            ],
        )?;
        let finish = HeapBindings::new(
            &mut gpu,
            Pass::VisibilityFinish,
            &[
                (key(0, 0), Input::Buffer(&configs[i])),
                (key(0, 5), Input::Buffer(&args[i])),
                (key(0, 6), Input::Buffer(&rejects)),
                (key(0, 7), Input::Image(hzb.texture())),
            ],
        )?;
        let reset = HeapBindings::new(
            &mut gpu,
            Pass::VisibilityReset,
            &[
                (key(0, 0), Input::Buffer(&configs[i])),
                (key(0, 5), Input::Buffer(&args[i])),
                (key(0, 6), Input::Buffer(&rejects)),
            ],
        )?;
        packets.push((cull, finish, reset));
    }
    let mut graph = Graph::default();
    for b in sources
        .iter()
        .chain(&configs)
        .chain(&lists)
        .chain(&args)
        .chain([&rejects, &visibility])
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    hzb.import(&mut graph);
    hzb_inputs.import(&mut graph);
    for (cull, finish, reset) in &packets {
        cull.import(&mut graph);
        finish.import(&mut graph);
        reset.import(&mut graph);
    }
    let mut rec = gpu.begin(Queue::Graphics);
    let mut checks = Vec::new();
    let mut final_point = None;
    for i in 0..2 {
        if i == 1 {
            let current = vec![[0u32, f.hzb_depths[1].to_bits()]; pixels];
            graph.node(
                &mut rec,
                &[(Resource::Buffer(visibility.buffer), Access::CopyDestination)],
            )?;
            rec.push(Command::UpdateBuffer {
                destination: visibility.buffer,
                offset: 0,
                bytes: bytemuck::cast_slice(&current).to_vec(),
            });
        }
        hzb.record(&mut graph, &mut rec, &hzb_inputs)?;
        if i == 0 {
            pass.first(&mut graph, &mut rec, &packets[i].0, f.clusters.len() as u32)?;
        } else {
            pass.second(&mut graph, &mut rec, &packets[i].0, &args[0])?;
        }
        checks.push((
            i,
            false,
            snapshot(
                &mut gpu,
                &mut graph,
                &mut rec,
                &[&lists[i], &args[i], &rejects],
            )?,
        ));
        pass.finish(&mut graph, &mut rec, &packets[i].1)?;
        checks.push((
            i,
            true,
            snapshot(
                &mut gpu,
                &mut graph,
                &mut rec,
                &[&lists[i], &args[i], &rejects],
            )?,
        ));
        final_point = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    let mut resets = Vec::new();
    for (ordinal, i) in [1, 0].into_iter().enumerate() {
        pass.reset(&mut graph, &mut rec, &packets[i].2)?;
        resets.push((
            ordinal,
            snapshot(&mut gpu, &mut graph, &mut rec, &[&args[i], &rejects])?,
        ));
    }
    let reset_point = gpu.submit(rec.finish(), &[])?;
    ensure!(final_point.is_some(), "cull submissions missing");
    gpu.wait(reset_point)?;
    cache.retire(&mut gpu)?;
    gpu.assert_clean()?;
    for (i, finished, reads) in checks {
        f.assert_pass(
            i,
            finished,
            &words(&gpu, reads[0])?,
            &words(&gpu, reads[1])?,
            &words(&gpu, reads[2])?,
        );
        println!(
            "cluster cull pass{i} finish={finished}: exact lists/counters/header; SW{:?} HW{:?} rejects{:?}",
            f.expected.passes[i].software,
            f.expected.passes[i].hardware,
            f.expected.passes[i].rejects
        );
        for b in reads {
            gpu.retire(Resource::Buffer(b))?;
        }
    }
    for (ordinal, reads) in resets {
        assert_eq!(
            f.expected.resets[ordinal].pass,
            if ordinal == 0 { 1 } else { 0 }
        );
        f.assert_reset(ordinal, &words(&gpu, reads[0])?, &words(&gpu, reads[1])?);
        println!("cluster reset{ordinal}: exact working/history/header words");
        for b in reads {
            gpu.retire(Resource::Buffer(b))?;
        }
    }
    for (cull, finish, reset) in packets {
        cull.retire(&mut gpu, &mut graph)?;
        finish.retire(&mut gpu, &mut graph)?;
        reset.retire(&mut gpu, &mut graph)?;
    }
    hzb_inputs.retire(&mut gpu, &mut graph)?;
    hzb.retire(&mut gpu, &mut graph)?;
    for b in sources
        .into_iter()
        .chain(configs)
        .chain(lists)
        .chain(args)
        .chain([rejects, visibility])
    {
        b.retire(&mut gpu)?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
