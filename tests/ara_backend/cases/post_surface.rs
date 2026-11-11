//! Raster depth reconstruction and camera motion at one and four samples.
use super::*;
use ara_gpu::{commands::Attachment, texture::SampleCount};
use renderlab::backend_ara::post_surface::{PostCameraFrame, RasterDepth, RasterPostSurface};

pub(super) fn depth_at(x: u32, y: u32) -> f32 {
    if x < 8 {
        0.6
    } else if y < 5 {
        0.25
    } else if y < 10 {
        0.8
    } else {
        0.
    }
}
fn reference(
    camera: renderlab::Camera,
    previous: renderlab::Mat4,
    history: bool,
    size: [u32; 2],
) -> Vec<[f32; 4]> {
    let current = camera.matrix(size[0], size[1]);
    let inverse = current.inverse();
    let view = camera.view();
    (0..size[0] * size[1])
        .map(|i| {
            let x = i % size[0];
            let y = i / size[0];
            let uv = [
                (x as f32 + 0.5) / size[0] as f32,
                (y as f32 + 0.5) / size[1] as f32,
            ];
            let ndc = glam::Vec2::new(uv[0] * 2. - 1., 1. - uv[1] * 2.);
            let z = depth_at(x, y);
            let h = inverse * glam::Vec4::new(ndc.x, ndc.y, z, 1.);
            let mut point = (h.truncate() / h.w).extend(1.);
            let mut depth = (-(view * point).z).max(0.001);
            if z == 0. {
                let near = inverse * glam::Vec4::new(ndc.x, ndc.y, 1., 1.);
                point = (point.truncate() - near.truncate() / near.w)
                    .normalize()
                    .extend(0.);
                depth = 1e8;
            }
            let a = current * point;
            let b = previous * point;
            let mut velocity = glam::Vec2::ZERO;
            if b.w > 0. && history {
                velocity = (a.truncate().truncate() / a.w - b.truncate().truncate() / b.w)
                    * glam::Vec2::new(0.5, -0.5)
                    * glam::Vec2::new(size[0] as f32, size[1] as f32)
                    * 0.25;
                velocity *= (4. / velocity.length().max(1e-5)).min(1.);
            }
            [velocity.x, velocity.y, depth, 1.]
        })
        .collect()
}
#[test]
fn raster_depth_velocity_and_camera_cuts_native_replay() -> Result<()> {
    let size = [17, 11];
    let path = root("post-surface");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = RasterPostSurface::new(&mut gpu, &cache)?;
    let camera = renderlab::Camera::look_at(
        renderlab::Vec3::new(1., 2., 6.),
        renderlab::Vec3::new(0., 1., 0.),
        renderlab::Vec3::Y,
        1.1,
        0.2,
        1000.,
    )?;
    let previous_camera = renderlab::Camera::look_at(
        renderlab::Vec3::new(-1., 2., 6.),
        renderlab::Vec3::new(0.8, 1., -2.),
        renderlab::Vec3::Y,
        1.1,
        0.2,
        1000.,
    )?;
    let previous = previous_camera.matrix(size[0], size[1]);
    let mut graph = Graph::default();
    for samples in [SampleCount::One, SampleCount::Four] {
        let depth = RasterDepth::new(&mut gpu, size, samples)?;
        let output = Texture::new(&mut gpu, size, Format::Rgba32Float, true, false)?;
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
        // The following compute list consumes raster depth on the same queue.
        for history in [true, false] {
            let frame = PostCameraFrame::new(
                &mut gpu,
                camera,
                if history { Some(previous) } else { None },
                size,
                180.,
                4.,
            )?;
            graph.import(Resource::Buffer(frame.record.buffer), Access::HostWrite);
            let mut rec = gpu.begin(Queue::Graphics);
            passes.record(&mut graph, &mut rec, &depth, &frame, &output)?;
            let read = renderlab::backend_ara::transfer::readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                output.image,
                size[0] as u64 * size[1] as u64 * 16,
            )?;
            let point = gpu.submit(rec.finish(), &[])?;
            gpu.wait(point)?;
            let actual: Vec<_> = gpu
                .read(read)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| f32::from_le_bytes(*v))
                .collect();
            let expected = reference(camera, previous, history, size);
            let mut max = 0f32;
            for (a, b) in actual.iter().zip(expected.iter().flatten()) {
                ensure!(a.is_finite(), "nonfinite raster surface");
                max = max.max((a - b).abs());
            }
            println!("raster surface {samples:?}, history={history}: max error {max}");
            ensure!(max <= 0.0003, "raster surface differs by {max}");
            if !history {
                ensure!(
                    actual
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|v| v[0] == 0. && v[1] == 0.),
                    "camera cut retained motion"
                );
            }
            graph.forget(Resource::Buffer(frame.record.buffer));
            frame.record.retire(&mut gpu)?;
            graph.forget(Resource::Buffer(read));
            gpu.retire(Resource::Buffer(read))?;
        }
        depth.retire(&mut gpu, &mut graph)?;
        graph.forget(Resource::Image(output.image));
        gpu.retire(Resource::Image(output.image))?;
        gpu.collect()?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
