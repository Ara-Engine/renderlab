//! Native clusters replay through the public backend encoder.
use super::*;
#[test]
fn packed_light_clusters_native_replay_against_cpu() -> Result<()> {
    use renderlab::backend_ara::light_cluster::{ClusterInputs, LightClusters};
    use renderlab::{Camera, Light, Lighting, Vec3};
    let path = root("clusters");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = LightClusters::new(&mut gpu, &cache)?;
    let target = LightClusters::target(&mut gpu)?;
    let camera = Camera::look_at(
        Vec3::new(1., 2., 3.),
        Vec3::new(0., 1., -4.),
        Vec3::Y,
        1.1,
        0.2,
        80.,
    )?;
    let mixed = vec![
        Light::Directional {
            travel: -Vec3::Y,
            lux: [1.; 3],
        },
        Light::Point {
            position: Vec3::new(0., 0., -6.),
            candela: [1.; 3],
            range: Some(3.),
            source_radius: 0.,
        },
        Light::Point {
            position: Vec3::new(100., 100., 100.),
            candela: [1.; 3],
            range: Some(1.),
            source_radius: 0.,
        },
        Light::Spot {
            position: Vec3::new(1., 2., 0.),
            travel: Vec3::new(0., -0.2, -1.),
            candela: [1.; 3],
            range: Some(12.),
            inner: 0.1,
            outer: 0.3,
            source_radius: 0.,
        },
        Light::Spot {
            position: Vec3::new(-3., 1., -4.),
            travel: -Vec3::Z,
            candela: [1.; 3],
            range: Some(8.),
            inner: 0.3,
            outer: 1.0,
            source_radius: 0.,
        },
        Light::Point {
            position: Vec3::ZERO,
            candela: [1.; 3],
            range: None,
            source_radius: 0.,
        },
    ];
    let overflow = (0..133)
        .map(|_| Light::Directional {
            travel: -Vec3::Y,
            lux: [1.; 3],
        })
        .collect();
    let mut graph = Graph::default();
    for (scenario, lights) in [mixed, overflow, Vec::new()].into_iter().enumerate() {
        let lighting = Lighting {
            lights,
            environment: [0.; 3],
            exposure_stops: 0.,
        };
        let inputs = ClusterInputs::new(&mut gpu, &lighting, camera, [320, 180])?;
        inputs.import(&mut graph);
        let mut rec = gpu.begin(Queue::Graphics);
        pass.record(&mut graph, &mut rec, &inputs, &target)?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            target.image,
            target.size[0] as u64 * target.size[1] as u64 * 4,
        )?;
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
        let bytes = gpu.read(read)?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        for c in 0..clusters::COUNT {
            let expected = clusters::expected(&lighting.lights, camera, [320, 180], c);
            let at = ((c / clusters::ROW) * target.size[0] + (c % clusters::ROW) * clusters::STRIDE)
                as usize;
            ensure!(
                words[at]
                    == expected.len().min(128) as u32 | ((expected.len().min(65535) as u32) << 16),
                "scenario {scenario} cluster {c} count differs"
            );
            for (index, &light) in expected.iter().take(128).enumerate() {
                let actual = (words[at + 1 + index / 2] >> (16 * (index % 2))) & 65535;
                ensure!(
                    actual == light,
                    "cluster {c} slot {index}: {actual} != {light}"
                );
            }
        }
        println!(
            "light clusters scenario {scenario}: {} exact cluster lists",
            clusters::COUNT
        );
        graph.forget(Resource::Buffer(inputs.lights.buffer));
        graph.forget(Resource::Buffer(inputs.frame.buffer));
        inputs.retire(&mut gpu)?;
        graph.forget(Resource::Buffer(read));
        gpu.retire(Resource::Buffer(read))?;
        gpu.collect()?;
    }
    pass.retire(&mut gpu)?;
    gpu.retire(Resource::Image(target.image))?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
