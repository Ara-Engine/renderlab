//! Original update kernel: wrapping births, integration and unexposed incident light.
use super::*;
use ara_gpu::sampler::{CompareOp, SamplerDesc};
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::InputKey,
    transfer::readback_buffer,
};
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
#[test]
fn particle_update_native_replay_against_cpu() -> Result<()> {
    let f = particle_update_reference::fixture();
    let path = root("particle-update");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = Kernel::new(&mut gpu, &cache, Pass::ParticleUpdate)?;
    let buffers = [
        (
            key(0, 30),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.frame))?,
        ),
        (
            key(0, 40),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.particles))?,
        ),
        (
            key(0, 42),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.emitter))?,
        ),
        (
            key(3, 0),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lighting))?,
        ),
        (
            key(3, 1),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lights))?,
        ),
        (
            key(3, 26),
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.transmission))?,
        ),
    ];
    let shadow = Texture::new(&mut gpu, [1, 1], Format::Depth32Float, false, false)?;
    let diffuse = Texture::cubes(&mut gpu, 1, 1, Format::Rgba16Float)?;
    let transmission = Texture::array(&mut gpu, [1, 1], 1, Format::Rgba16Float, false, false)?;
    let sampler = SamplerDesc {
        compare: Some(CompareOp::LessEqual),
        ..Default::default()
    };
    let mut values: Vec<_> = buffers
        .iter()
        .map(|(key, b)| (*key, Input::Buffer(b)))
        .collect();
    values.extend([
        (key(0, 34), Input::Image(&shadow)),
        (key(0, 35), Input::Sampler(sampler)),
        (key(3, 6), Input::Image(&diffuse)),
        (key(3, 11), Input::Sampler(SamplerDesc::default())),
        (key(3, 27), Input::Image(&transmission)),
    ]);
    ensure!(
        renderlab::backend_ara::shader::inputs(Pass::ParticleUpdate)?.len() == 11,
        "update interface changed"
    );
    let inputs = HeapBindings::new(&mut gpu, Pass::ParticleUpdate, &values)?;
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
            shadow.image,
            bytemuck::bytes_of(&0f32),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            diffuse.image,
            &half_bytes(&[[0., 0., 0., 1.]; 6]),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            transmission.image,
            &half_bytes(&[[1., 1., 1., 0.]]),
        )?,
    ];
    pass.record(&mut graph, &mut rec, &inputs, [f.emitter[32], 1, 1])?;
    let particles = &buffers[1].1;
    let read = readback_buffer(
        &mut gpu,
        &mut graph,
        &mut rec,
        particles.buffer,
        0,
        particles.len,
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let bytes = gpu.read(read)?;
    let actual: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes(*v))
        .collect();
    let mut max = 0f32;
    for (i, (a, b)) in actual
        .as_chunks::<12>()
        .0
        .iter()
        .zip(&f.expected)
        .enumerate()
    {
        for (j, (x, y)) in a.iter().zip(b).enumerate() {
            ensure!(x.is_finite(), "nonfinite particle {i}:{j}");
            max = max.max((x - y).abs());
            let preserved =
                [6, 7, 11, 14, 17, 18, 19].contains(&i) || ([5, 8, 9].contains(&i) && j >= 8);
            if preserved {
                ensure!(
                    x.to_bits() == y.to_bits(),
                    "preserved particle word differs {i}:{j}"
                );
            }
        }
    }
    println!("particle update: max error {max}; dead/tail and dying light words exact");
    ensure!(max <= 0.00002, "particle update differs");
    inputs.retire(&mut gpu, &mut graph)?;
    for (_, b) in buffers {
        graph.forget(Resource::Buffer(b.buffer));
        b.retire(&mut gpu)?;
    }
    for t in [shadow, diffuse, transmission] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging.into_iter().chain([read]) {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
