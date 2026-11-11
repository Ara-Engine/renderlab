//! Camera-space distance from stored raster depth, including original miss sentinel.
use super::*;
use ara_gpu::{commands::Attachment, texture::SampleCount};
use renderlab::backend_ara::{
    post_surface::RasterDepth,
    transparency_depth::{RasterDepthConversion, RasterDepthFrame},
};
#[test]
fn transparency_raster_depth_native_replay_against_cpu() -> Result<()> {
    let size = [17, 11];
    let near = 0.2;
    let far = 1000.;
    let path = root("transparency-depth");
    let camera = renderlab::Camera::look_at(
        renderlab::Vec3::new(1., 2., 6.),
        renderlab::Vec3::new(-0.5, 1.5, -2.),
        renderlab::Vec3::Y,
        1.1,
        near,
        far,
    )?;
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = RasterDepthConversion::new(&mut gpu, &cache)?;
    let frame = RasterDepthFrame::new(&mut gpu, camera, size)?;
    let depth = RasterDepth::new(&mut gpu, size, SampleCount::One)?;
    let output = Texture::new(&mut gpu, size, Format::R32Float, true, false)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(frame.record.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    graph.node(&mut rec, &[(Resource::Image(depth.image()), Access::Depth)])?;
    rec.push(Command::BeginRendering {
        colors: vec![],
        depth: Some(Attachment {
            image: depth.image(),
            clear: Some([0.; 4]),
        }),
        size,
    });
    for (origin, extent, z) in [
        ([0, 0], [8, 11], 0.6),
        ([8, 0], [9, 5], 0.25),
        ([8, 5], [9, 5], 0.8),
    ] {
        rec.push(Command::ClearDepth {
            origin,
            size: extent,
            depth: z,
        });
    }
    rec.push(Command::EndRendering);
    gpu.submit(rec.finish(), &[])?;
    let mut rec = gpu.begin(Queue::Graphics);
    pass.record(&mut graph, &mut rec, &frame, &depth, &output)?;
    let read = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        output.image,
        u64::from(size[0] * size[1] * 4),
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let bytes = gpu.read(read)?;
    let mut max = 0f32;
    let mut misses = 0;
    for (i, a) in bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes(*v))
        .enumerate()
    {
        let z = super::post_surface_cases::depth_at(i as u32 % size[0], i as u32 / size[0]);
        if z == 0. {
            ensure!(
                a.to_bits() == 1e30f32.to_bits(),
                "transparency miss sentinel changed"
            );
            misses += 1;
        } else {
            let expected = 1. / (z * (1. / near - 1. / far) + 1. / far);
            ensure!(a.is_finite(), "nonfinite view distance");
            max = max.max((a - expected).abs());
        }
    }
    println!("transparency depth: max error {max}, {misses} exact misses");
    ensure!(max <= 1e-5 && misses == 9, "transparency depth differs");
    pass.retire(&mut gpu)?;
    frame.record.retire(&mut gpu)?;
    depth.retire(&mut gpu, &mut graph)?;
    gpu.retire(Resource::Image(output.image))?;
    gpu.retire(Resource::Buffer(read))?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
