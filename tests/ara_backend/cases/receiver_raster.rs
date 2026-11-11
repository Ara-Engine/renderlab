//! Original two-MRT receiver raster against a world-translated scalar mesh oracle.
use super::receiver_raster_reference as cpu;
use super::*;
use ara_gpu::texture::SampleCount;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    mesh::{DiagnosticCamera, MeshGeometry},
    post_surface::RasterDepth,
    receiver_raster::{ReceiverDraw, ReceiverTargets, Receivers},
    shader::InputKey,
    transfer::readback,
};
#[test]
fn depth_winning_receiver_raster_native_replay_against_cpu() -> Result<()> {
    let path = root("receiver-raster");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = Receivers::new(&mut gpu, &cache)?;
    let mut fixture = cpu::fixture();
    let geometries: Vec<_> = fixture
        .slots
        .iter()
        .map(|slot| {
            MeshGeometry::new(
                &mut gpu,
                &fixture.vertices,
                &fixture.indices,
                &[*slot],
                &fixture.models,
            )
        })
        .collect::<Result<_>>()?;
    let geometry = &geometries[0];
    let camera = DiagnosticCamera::new(&mut gpu, fixture.view_projection, 0)?;
    let world = Texture::new(&mut gpu, fixture.viewport, Format::Rgba32Float, false, true)?;
    let normal = Texture::new(&mut gpu, fixture.viewport, Format::Rgba32Float, false, true)?;
    let depth = RasterDepth::new(&mut gpu, fixture.viewport, SampleCount::One)?;
    let mut graph = Graph::default();
    for geometry in &geometries {
        geometry.import(&mut graph);
    }
    graph.import(Resource::Buffer(camera.record.buffer), Access::HostWrite);
    let wrong_program = renderlab::backend_ara::shader::Program::new(
        Pass::ReceiverRaster,
        r#"
        @vertex fn vs() -> @builtin(position) vec4<f32> {return vec4<f32>(0.,0.,0.,1.);}
        @fragment fn fs_receiver() -> @location(0) vec4<f32> {return vec4<f32>(0.);}
    "#,
    )?;
    let wrong_packet = HeapBindings::for_program(&mut gpu, &wrong_program, &[])?;
    wrong_packet.import(&mut graph);
    let mut rejected = gpu.begin(Queue::Graphics);
    let error = pass
        .record(
            &mut graph,
            &mut rejected,
            &[ReceiverDraw {
                geometry,
                bindings: &wrong_packet,
            }],
            ReceiverTargets {
                world: &world,
                normal: &normal,
                depth: &depth,
            },
        )
        .expect_err("same-pass incompatible packet accepted");
    ensure!(
        error.to_string().contains("contract"),
        "wrong packet rejection: {error}"
    );
    ensure!(
        graph
            .require_initialized(Resource::Image(world.image))
            .is_err(),
        "rejected packet mutated attachment state"
    );
    wrong_packet.retire(&mut gpu, &mut graph)?;
    let mut snapshots = Vec::new();
    let mut materials = Vec::new();
    let mut last = None;
    for double_sided in [false, true] {
        fixture.material.double_sided = double_sided;
        for thin in [false, true] {
            let mut words = fixture.material.words();
            words[7] = u32::from(thin);
            let material = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&words))?;
            graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
            let inputs = HeapBindings::new(
                &mut gpu,
                Pass::ReceiverRaster,
                &[
                    (
                        InputKey {
                            group: 0,
                            binding: 0,
                        },
                        Input::Buffer(&camera.record),
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
            inputs.import(&mut graph);
            let mut rec = gpu.begin(Queue::Graphics);
            pass.record(
                &mut graph,
                &mut rec,
                &geometries
                    .iter()
                    .map(|geometry| ReceiverDraw {
                        geometry,
                        bindings: &inputs,
                    })
                    .collect::<Vec<_>>(),
                ReceiverTargets {
                    world: &world,
                    normal: &normal,
                    depth: &depth,
                },
            )?;
            let mut reads = Vec::new();
            for (image, bytes) in [(world.image, 16), (normal.image, 16), (depth.image(), 4)] {
                reads.push(readback(
                    &mut gpu,
                    &mut graph,
                    &mut rec,
                    image,
                    u64::from(fixture.viewport[0] * fixture.viewport[1] * bytes),
                )?);
            }
            last = Some(gpu.submit(rec.finish(), &[])?);
            inputs.retire(&mut gpu, &mut graph)?;
            snapshots.push((double_sided, thin, reads, cpu::replay(&fixture, thin)));
            materials.push(material);
        }
    }
    gpu.wait(last.expect("four queued receiver variants"))?;
    for (sided, thin, reads, expected) in snapshots {
        let mut outputs = Vec::new();
        for &read in &reads {
            outputs.push(
                gpu.read(read)?
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<_>>(),
            );
        }
        let mut covered = 0;
        let mut errors = [0f32; 3];
        for (p, &z) in expected.depth.iter().enumerate() {
            if z == 0. {
                ensure!(
                    outputs[2][p] == 0.
                        && outputs[0][p * 4..p * 4 + 4] == [0.; 4]
                        && outputs[1][p * 4..p * 4 + 4] == [0.; 4],
                    "receiver miss changed at {p}"
                );
                continue;
            }
            covered += 1;
            for c in 0..3 {
                errors[0] =
                    errors[0].max((outputs[0][p * 4 + c] - expected.world.pixels[p][c]).abs());
                errors[1] =
                    errors[1].max((outputs[1][p * 4 + c] - expected.normal.pixels[p][c]).abs());
            }
            errors[2] = errors[2].max((outputs[2][p] - z).abs());
            ensure!(
                outputs[0][p * 4 + 3] == cpu::EPSILON
                    && outputs[1][p * 4 + 3] == if thin { -1. } else { 1. },
                "receiver epsilon/coverage differs at {p}"
            );
        }
        // Four float32 ULPs at the translated X=384; normals/depth stay near unity.
        ensure!(
            outputs.iter().flatten().all(|v| v.is_finite())
                && errors[0] <= 1. / 8192.
                && errors[1] <= 2e-5
                && errors[2] <= 2e-6,
            "receiver error {errors:?}"
        );
        println!(
            "receiver sided{sided} thin{thin}: {covered} covered, exact epsilon/thin/misses, errors {errors:?}"
        );
        for read in reads {
            gpu.retire(Resource::Buffer(read))?;
        }
    }
    for b in materials {
        b.retire(&mut gpu)?;
    }
    for geometry in geometries {
        geometry.retire(&mut gpu, &mut graph)?;
    }
    camera.retire(&mut gpu, &mut graph)?;
    depth.retire(&mut gpu, &mut graph)?;
    for t in [world, normal] {
        gpu.retire(Resource::Image(t.image))?;
    }
    pass.retire(&mut gpu)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
