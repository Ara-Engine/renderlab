//! Original shadow depth at selected atlas mip/layer and native model indices.
use super::*;
use ara_gpu::texture::ImageRegion;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    mesh::MeshGeometry,
    shader::InputKey,
    shadow_raster::{ShadowDraw, ShadowRaster},
    transfer::readback,
};
fn snapshot(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    view: &Texture,
) -> Result<ara_gpu::Buffer> {
    let buffer = gpu.create_readback_buffer(u64::from(view.size[0] * view.size[1] * 4))?;
    graph.node_uses(
        rec,
        &[
            view.usage(Access::CopySource),
            (Resource::Buffer(buffer), Access::CopyDestination).into(),
        ],
    )?;
    rec.push(Command::ReadImageRegion {
        source: view.image,
        destination: buffer,
        buffer_offset: 0,
        region: ImageRegion {
            mip: view.view.base_mip,
            origin: [0; 3],
            extent: [view.size[0], view.size[1], 1],
            base_layer: view.view.base_layer,
            layer_count: 1,
        },
    });
    graph.node(rec, &[(Resource::Buffer(buffer), Access::HostRead)])?;
    Ok(buffer)
}
#[test]
fn shadow_depth_atlas_native_instances_and_subresources_against_cpu() -> Result<()> {
    let path = root("shadow-raster");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = ShadowRaster::new(&mut gpu, &cache)?;
    let mut fixture = mesh_reference::fixture();
    fixture.slots = vec![1, 2, 0]; // Native range1..3, then row0; not a slot stream.
    let geometry = MeshGeometry::new(
        &mut gpu,
        &fixture.vertices,
        &fixture.indices,
        &fixture.slots,
        &fixture.models,
    )?;
    let base_projection = fixture.view_projection;
    let [width, height] = fixture.viewport;
    let n = (width * height) as usize;
    let (atlas, views) = Texture::depth_atlas(&mut gpu, [width * 2, height * 2], 3, 2)?;
    let mut graph = Graph::default();
    geometry.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    let sentinel = vec![0.125f32; n * 15];
    let staging = upload(
        &mut gpu,
        &mut graph,
        &mut rec,
        atlas.image,
        bytemuck::cast_slice(&sentinel),
    )?;
    let mut records = Vec::new();
    let mut snapshots = Vec::new();
    let mut last = None;
    for sided in [false, true] {
        fixture.material.double_sided = sided;
        let material =
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&fixture.material.words()))?;
        graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
        for layer in 0..2 {
            fixture.view_projection = renderlab::Mat4::from_scale(renderlab::Vec3::new(
                1.,
                1.,
                if layer == 0 { 1. } else { 0.75 },
            )) * base_projection;
            let mut words = [0u32; 28];
            words[..16].copy_from_slice(&fixture.view_projection.to_cols_array().map(f32::to_bits));
            let view = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&words))?;
            graph.import(Resource::Buffer(view.buffer), Access::HostWrite);
            let packet = HeapBindings::new(
                &mut gpu,
                Pass::ShadowRaster,
                &[
                    (
                        InputKey {
                            group: 0,
                            binding: 0,
                        },
                        Input::Buffer(&view),
                    ),
                    (
                        InputKey {
                            group: 1,
                            binding: 0,
                        },
                        Input::Buffer(geometry.models()),
                    ),
                    (
                        InputKey {
                            group: 1,
                            binding: 1,
                        },
                        Input::Buffer(geometry.normals()),
                    ),
                    (
                        InputKey {
                            group: 2,
                            binding: 0,
                        },
                        Input::Buffer(&material),
                    ),
                ],
            )?;
            packet.import(&mut graph);
            let target = &views[3 + layer];
            if snapshots.is_empty() {
                let bad = ShadowDraw {
                    geometry: &geometry,
                    bindings: &packet,
                    first_instance: fixture.models.len() as u32,
                    instance_count: 1,
                };
                let error = pass
                    .record(&mut graph, &mut rec, &[bad], target)
                    .expect_err("out-of-table native instance accepted");
                ensure!(
                    error.to_string().contains("range exceeds"),
                    "wrong shadow rejection: {error}"
                );
            }
            let draws: Vec<_> = [(1, 2), (0, 1)]
                .into_iter()
                .map(|(first_instance, instance_count)| ShadowDraw {
                    geometry: &geometry,
                    bindings: &packet,
                    first_instance,
                    instance_count,
                })
                .collect();
            pass.record(&mut graph, &mut rec, &draws, target)?;
            snapshots.push((
                sided,
                layer,
                snapshot(&mut gpu, &mut graph, &mut rec, target)?,
                mesh_reference::rasterize_with(&fixture, |_| [0.; 4]).depth,
            ));
            last = Some(gpu.submit(rec.finish(), &[])?);
            packet.retire(&mut gpu, &mut graph)?;
            records.push(view);
            rec = gpu.begin(Queue::Graphics);
        }
        records.push(material);
    }
    let all = readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        atlas.image,
        (sentinel.len() * 4) as u64,
    )?;
    last = Some(gpu.submit(rec.finish(), &[last.expect("shadow draws submitted")])?);
    gpu.wait(last.expect("atlas snapshot submitted"))?;
    for (sided, layer, read, expected) in snapshots {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let mut error = 0f32;
        let mut covered = 0;
        for (p, (&a, &e)) in actual.iter().zip(&expected).enumerate() {
            ensure!(a.is_finite(), "nonfinite shadow depth");
            if e == 0. {
                ensure!(a == 0., "shadow miss differs at {p}");
            } else {
                covered += 1;
                error = error.max((a - e).abs());
            }
        }
        ensure!(
            error <= 2e-6,
            "shadow sided{sided} layer{layer} error{error}"
        );
        println!(
            "shadow mip1 layer{layer} sided{sided}: {covered} covered, exact misses, max error{error}"
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    let bytes = gpu.read(all)?;
    let actual: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    ensure!(
        actual.len() == sentinel.len()
            && actual[..n * 12]
                .iter()
                .chain(&actual[n * 14..])
                .all(|&x| x == 0.125),
        "shadow atlas touched another mip/layer"
    );
    gpu.retire(Resource::Buffer(all))?;
    gpu.retire(Resource::Buffer(staging))?;
    for b in records {
        b.retire(&mut gpu)?;
    }
    geometry.retire(&mut gpu, &mut graph)?;
    gpu.retire(Resource::Image(atlas.image))?;
    pass.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

#[test]
fn vertex_only_transparency_sun_shadow_native_replay_against_cpu() -> Result<()> {
    let path = root("tp-sun-shadow");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let entries = renderlab::backend_ara::shader::msl(Pass::TransparencySunShadow)?.entries;
    ensure!(
        matches!(
            entries,
            renderlab::backend_ara::shader::MetalEntries::Raster { fragment: None, .. }
        ),
        "vertex-only source gained a fragment shader"
    );
    let pass = ShadowRaster::transparency_sun(&mut gpu, &cache)?;
    let size = [9, 7];
    let vertices: Vec<_> = [[-0.75, -0.75], [0.75, -0.75], [0.75, 0.75], [-0.75, 0.75]]
        .into_iter()
        .map(|p| renderlab::Vertex {
            position: [p[0], p[1], 0.25],
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        })
        .collect();
    let models = [
        renderlab::Mat4::IDENTITY,
        renderlab::Mat4::from_translation(renderlab::Vec3::new(0., 0., -0.0625))
            * renderlab::Mat4::from_scale(renderlab::Vec3::new(-1., 1., 1.)),
    ];
    let geometry = MeshGeometry::new(&mut gpu, &vertices, &[0, 1, 2, 0, 2, 3], &[1, 0], &models)?;
    let (atlas, views) = Texture::depth_atlas(&mut gpu, size, 1, 1)?;
    let mut frame = [0u32; 84];
    frame[32..48].copy_from_slice(&renderlab::Mat4::IDENTITY.to_cols_array().map(f32::to_bits));
    let frame = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&frame))?;
    let packet = HeapBindings::new(
        &mut gpu,
        Pass::TransparencySunShadow,
        &[
            (
                InputKey {
                    group: 0,
                    binding: 30,
                },
                Input::Buffer(&frame),
            ),
            (
                InputKey {
                    group: 1,
                    binding: 0,
                },
                Input::Buffer(geometry.models()),
            ),
        ],
    )?;
    let mut graph = Graph::default();
    geometry.import(&mut graph);
    packet.import(&mut graph);
    graph.import(Resource::Buffer(frame.buffer), Access::HostWrite);
    let mut reads = Vec::new();
    let mut last = None;
    for model in 0..2 {
        let mut rec = gpu.begin(Queue::Graphics);
        pass.record(
            &mut graph,
            &mut rec,
            &[ShadowDraw {
                geometry: &geometry,
                bindings: &packet,
                first_instance: model,
                instance_count: 1,
            }],
            &views[0],
        )?;
        reads.push(snapshot(&mut gpu, &mut graph, &mut rec, &views[0])?);
        last = Some(gpu.submit(rec.finish(), &[])?);
    }
    gpu.wait(last.expect("two queued vertex-only shadows"))?;
    for (model, read) in reads.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let expected = if model == 0 { 0.25 } else { 0.1875 };
        let mut covered = 0;
        let mut maximum_bias = 0f32;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let p = actual[(y * size[0] + x) as usize];
                let ndc = [
                    (x as f64 + 0.5) * 2. / size[0] as f64 - 1.,
                    1. - (y as f64 + 0.5) * 2. / size[1] as f64,
                ];
                if ndc.iter().all(|v| v.abs() < 0.75) {
                    // Flat planes have zero slope. Positive constant bias is native
                    // depth precision dependent; bound it by2e-6 and require it present.
                    ensure!(
                        p.is_finite() && p > expected && p - expected <= 2e-6,
                        "sun shadow model{model} pixel{x}/{y}: {p} != {expected}+native bias"
                    );
                    maximum_bias = maximum_bias.max(p - expected);
                    covered += 1;
                } else {
                    ensure!(p == 1., "sun shadow changed uncovered pixel");
                }
            }
        }
        println!(
            "TP sun shadow model{model}: {covered} covered, exact misses, positive bias<= {maximum_bias}"
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    // Native metadata must carry exactly the source's one stage, without a shim fragment.
    for key in std::fs::read_dir(path.join("shaders"))? {
        let key = key?;
        let metadata: ara_gpu::cooked::CookedPipeline =
            serde_json::from_slice(&std::fs::read(key.path().join("pipeline.json"))?)?;
        ensure!(
            metadata.stages.len() == 1
                && metadata.stages[0].stage == ara_gpu::cooked::Stage::Vertex,
            "sun pipeline artifact is not vertex-only"
        );
    }
    packet.retire(&mut gpu, &mut graph)?;
    frame.retire(&mut gpu)?;
    geometry.retire(&mut gpu, &mut graph)?;
    gpu.retire(Resource::Image(atlas.image))?;
    pass.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
