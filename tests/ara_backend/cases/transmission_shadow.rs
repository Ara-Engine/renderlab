//! Independent per-object transmission layers, original Fresnel and packed coverage.
use super::transmission_shadow_reference as cpu;
use super::*;
use ara_gpu::{
    sampler::{Filter, SamplerDesc},
    texture::ImageRegion,
};
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    mesh::MeshGeometry,
    shader::InputKey,
    shadow_raster::ShadowDraw,
    transfer::readback,
    transmission_shadow::Transmission,
};
fn read_view(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    view: &Texture,
) -> Result<ara_gpu::Buffer> {
    let bytes =
        u64::from(view.size[0]) * u64::from(view.size[1]) * u64::from(view.format.texel_bytes());
    let output = gpu.create_readback_buffer(bytes)?;
    graph.node_uses(
        rec,
        &[
            view.usage(Access::CopySource),
            (Resource::Buffer(output), Access::CopyDestination).into(),
        ],
    )?;
    rec.push(Command::ReadImageRegion {
        source: view.image,
        destination: output,
        buffer_offset: 0,
        region: ImageRegion {
            mip: view.view.base_mip,
            origin: [0; 3],
            extent: [view.size[0], view.size[1], 1],
            base_layer: view.view.base_layer,
            layer_count: 1,
        },
    });
    graph.node(rec, &[(Resource::Buffer(output), Access::HostRead)])?;
    Ok(output)
}
#[test]
fn colored_transmission_shadow_layers_native_replay_against_cpu() -> Result<()> {
    let path = root("transmission-shadow");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = Transmission::new(&mut gpu, &cache)?;
    let sampler = SamplerDesc {
        min: Filter::Nearest,
        mag: Filter::Nearest,
        mip: Filter::Nearest,
        ..Default::default()
    };
    for case in [
        cpu::Case::Ordered,
        cpu::Case::Reversed,
        cpu::Case::BackFacing,
        cpu::Case::ZeroOpacity,
    ] {
        let f = cpu::fixture(case);
        let size = f.scene.size;
        let mut graph = Graph::default();
        let mut rec = gpu.begin(Queue::Graphics);
        let output = Texture::array(&mut gpu, size, 2, Format::Rgba16Float, false, true)?;
        let views = [
            output.view_2d(&mut gpu, 0, 0)?,
            output.view_2d(&mut gpu, 0, 1)?,
        ];
        ensure!(
            output.view_2d(&mut gpu, 1, 0).is_err() && views[0].view_2d(&mut gpu, 0, 1).is_err(),
            "view escaped its parent interval"
        );
        let (depth, depth_views) = Texture::depth_atlas(&mut gpu, size, 1, 1)?;
        let mut frame = [0u32; 24];
        frame[..16].copy_from_slice(&renderlab::Mat4::IDENTITY.to_cols_array().map(f32::to_bits));
        frame[16..19].copy_from_slice(&f.sun.map(f32::to_bits));
        let frame = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&frame))?;
        graph.import(Resource::Buffer(frame.buffer), Access::HostWrite);
        let models: Vec<_> = f
            .scene
            .models
            .iter()
            .map(|m| renderlab::Mat4::from_cols_array(&m.map(f32::from_bits)))
            .collect();
        let mut geometries = Vec::new();
        let mut materials = Vec::new();
        let mut bases = Vec::new();
        let mut staging = Vec::new();
        let mut snapshots = Vec::new();
        let mut last = None;
        for (layer, &index) in f.order.iter().enumerate() {
            let pane = &f.scene.panes[index];
            let geometry = MeshGeometry::new(
                &mut gpu,
                &pane.vertices,
                &pane.indices,
                &[pane.slot],
                &models,
            )?;
            geometry.import(&mut graph);
            let material = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&pane.material))?;
            graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
            let base = Texture::new(&mut gpu, [1, 1], Format::Rgba16Float, false, false)?;
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                base.image,
                &half_bytes(&pane.base.pixels),
            )?);
            let packet = HeapBindings::new(
                &mut gpu,
                Pass::TransmissionShadow,
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
                    (
                        InputKey {
                            group: 2,
                            binding: 0,
                        },
                        Input::Buffer(&material),
                    ),
                    (
                        InputKey {
                            group: 2,
                            binding: 1,
                        },
                        Input::Image(&base),
                    ),
                    (
                        InputKey {
                            group: 2,
                            binding: 2,
                        },
                        Input::Sampler(sampler),
                    ),
                ],
            )?;
            packet.import(&mut graph);
            pass.record_layer(
                &mut graph,
                &mut rec,
                &ShadowDraw {
                    geometry: &geometry,
                    bindings: &packet,
                    first_instance: pane.slot,
                    instance_count: 1,
                },
                &views[layer],
                &depth_views[0],
            )?;
            snapshots.push((
                read_view(&mut gpu, &mut graph, &mut rec, &views[layer])?,
                read_view(&mut gpu, &mut graph, &mut rec, &depth_views[0])?,
            ));
            last = Some(gpu.submit(rec.finish(), &[])?);
            packet.retire(&mut gpu, &mut graph)?;
            rec = gpu.begin(Queue::Graphics);
            geometries.push(geometry);
            materials.push(material);
            bases.push(base);
        }
        let all = readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(size[0] * size[1] * 2 * 8),
        )?;
        let point = gpu.submit(
            rec.finish(),
            &[last.expect("transmission layers submitted")],
        )?;
        gpu.wait(point)?;
        let mut images = Vec::new();
        let mut depths = Vec::new();
        let mut layer_bytes = Vec::new();
        for &(color, z) in &snapshots {
            let bytes = gpu.read(color)?;
            layer_bytes.extend_from_slice(&bytes);
            images.push(reference::Image {
                size,
                pixels: bytes
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|p| {
                        std::array::from_fn(|c| {
                            f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                        })
                    })
                    .collect(),
            });
            depths.push(
                gpu.read(z)?
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<_>>(),
            );
        }
        f.assert_matches(&images, &depths);
        ensure!(
            gpu.read(all)? == layer_bytes,
            "later object changed another transmission layer"
        );
        println!(
            "transmission {case:?}: two independent layers, RGB<=2halfULPs, exact clears/coverage/alpha"
        );
        for (a, b) in snapshots {
            gpu.retire(Resource::Buffer(a))?;
            gpu.retire(Resource::Buffer(b))?;
        }
        gpu.retire(Resource::Buffer(all))?;
        for b in staging {
            gpu.retire(Resource::Buffer(b))?;
        }
        for g in geometries {
            g.retire(&mut gpu, &mut graph)?;
        }
        for b in materials {
            b.retire(&mut gpu)?;
        }
        frame.retire(&mut gpu)?;
        for t in bases.into_iter().chain([output, depth]) {
            gpu.retire(Resource::Image(t.image))?;
        }
    }
    pass.retire(&mut gpu)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
