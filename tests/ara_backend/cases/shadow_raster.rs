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
