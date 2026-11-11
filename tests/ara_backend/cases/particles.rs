//! Independent total-order reference for the original depth/slot keys.
use super::*;
use renderlab::backend_ara::particles::{ParticleSort, ParticleSortBuffers};
#[test]
fn particle_depth_sort_native_replay_against_cpu() -> Result<()> {
    let path = root("particles");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let first = ParticleSort::new(&mut gpu, &cache)?;
    let artifact = std::fs::read_dir(path.join("shaders"))?
        .next()
        .expect("one cached shader")?
        .path()
        .join("pipeline.json");
    let before = std::fs::metadata(&artifact)?.modified()?;
    let pass = ParticleSort::new(&mut gpu, &cache)?;
    ensure!(
        std::fs::metadata(&artifact)?.modified()? == before,
        "cache hit replaced the artifact"
    );
    first.retire(&mut gpu)?;
    let eye = renderlab::Vec3::new(2., 3., 4.);
    let target = renderlab::Vec3::new(-1., 2., -8.);
    let camera = renderlab::Camera::look_at(eye, target, renderlab::Vec3::Y, 1., 0.1, 1e4)?;
    // Build forward independently of the GPU packet's inverse-view extraction.
    let forward = (target - eye).normalize();
    // Camera stores a view matrix; its reconstructed eye is the packet input.
    let eye = camera.view().inverse().w_axis.truncate();
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut retained = Vec::new();
    for capacity in [1u32, 9, 2048] {
        let particles: Vec<[f32; 12]> = (0..capacity)
            .map(|i| {
                let depth: f32 = match i % 7 {
                    0 => -1.,
                    1 => 0.,
                    2 => 8.,
                    3 => 8.0001,
                    _ => ((i * 17) % 101) as f32 * 0.25,
                };
                // The key keeps a depth's bits above the low eleven. Round numbers have
                // those bits zero, exactly on a bucket boundary, where the last ulp of
                // either device's dot product decides the bucket. Keep every positive
                // depth at least 768 ulps inside its bucket; distinct depths that shared
                // a bucket (8 and 8.0001) stay distinct and still share it.
                let depth = if depth > 0. {
                    let bits = depth.to_bits();
                    f32::from_bits((bits & !0x7ff) | (0x300 + (bits & 0x7ff) % 0x200))
                } else {
                    depth
                };
                let p = eye + forward * depth;
                [
                    p.x,
                    p.y,
                    p.z,
                    if i % 5 == 0 { 2. } else { 0.25 },
                    0.,
                    0.,
                    0.,
                    1.,
                    0.1,
                    0.2,
                    0.3,
                    0.,
                ]
            })
            .collect();
        let buffers = ParticleSortBuffers::new(&mut gpu, camera, &particles)?;
        buffers.import(&mut graph);
        let mut expected: Vec<u32> = (0..buffers.sorted_len)
            .map(|i| {
                let mut key = 0u32;
                if i < capacity {
                    let p = particles[i as usize];
                    let depth = (renderlab::Vec3::new(p[0], p[1], p[2]) - eye).dot(forward);
                    if p[3] < p[7] && depth > 0. {
                        key = ((depth.to_bits() >> 11).max(1)) << 11;
                    }
                }
                key | i
            })
            .collect();
        expected.sort_unstable_by(|a, b| b.cmp(a));
        pass.record(&mut graph, &mut rec, &buffers)?;
        let read = renderlab::backend_ara::transfer::readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            buffers.keys().buffer,
            0,
            u64::from(buffers.sorted_len) * 4,
        )?;
        retained.push((buffers, read, expected));
    }
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    for (buffers, read, expected) in retained {
        let bytes = gpu.read(read)?;
        let actual: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v))
            .collect();
        if actual != expected {
            println!("actual {actual:?} expected {expected:?}");
        }
        ensure!(
            actual == expected,
            "particle depth/slot sort differs for{}slots",
            buffers.capacity
        );
        println!(
            "particle sort {} slots / {} keys: exact",
            buffers.capacity, buffers.sorted_len
        );
        buffers.retire(&mut gpu, &mut graph)?;
        gpu.retire(Resource::Buffer(read))?;
    }
    pass.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
