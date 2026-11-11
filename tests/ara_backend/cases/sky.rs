//! Raster backdrop replay across all sun/height/volume flag combinations.
use super::*;
use renderlab::backend_ara::{
    atmosphere::{AtmosphereParameters, AtmosphereSun, AtmosphereTargets},
    sky::SkyPass,
};
#[test]
fn raster_sky_sun_and_fog_native_replay_against_cpu() -> Result<()> {
    let viewport = [17, 11];
    let grid = [3, 2];
    let sun = renderlab::Vec3::new(0.4, 0.2, -1.).normalize();
    let eye = renderlab::Vec3::new(0., 10., 0.);
    let camera = renderlab::Camera::look_at(
        eye,
        eye + sun + renderlab::Vec3::new(0.004, 0., 0.),
        renderlab::Vec3::Y,
        0.4,
        0.1,
        1e5,
    )?;
    let model = atmosphere_reference::Model {
        medium: Default::default(),
        sun,
        altitude: 0.01,
    };
    let mut trans = model.transmittance();
    trans.size[1] = 65;
    trans.pixels.resize(256 * 65, [0.; 4]);
    trans.pixels[256 * 64] = [0.03125, 0.0625, 0.125, 1.];
    let sky = environment_reference::sky();
    let volume = reference::Image {
        size: [grid[0], grid[1] * 96],
        pixels: (0..grid[0] * grid[1] * 96)
            .map(|i| {
                let z = (i / (grid[0] * grid[1])) as f32 / 95.;
                let x = (i % grid[0]) as f32;
                reference::half([
                    z * 0.04 + x * 0.002,
                    z * 0.06,
                    z * 0.08,
                    (1. - 0.7 * z).max(0.1),
                ])
            })
            .collect(),
    };
    let path = root("sky");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = SkyPass::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let luts = AtmosphereTargets {
        transmittance: Texture::new(&mut gpu, trans.size, Format::Rgba16Float, false, false)?,
        multiscatter: Texture::new(&mut gpu, [32, 32], Format::Rgba16Float, false, false)?,
        sky: Texture::new(&mut gpu, sky.size, Format::Rgba16Float, false, false)?,
    };
    let packed =
        Texture::volume_shape(&mut gpu, [grid[0], grid[1], 96], Format::Rgba16Float, false)?;
    let output = Texture::new(&mut gpu, viewport, Format::Rgba32Float, false, true)?;
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();
    for (image, pixels) in [
        (luts.transmittance.image, &trans.pixels),
        (luts.sky.image, &sky.pixels),
        (packed.image, &volume.pixels),
    ] {
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            image,
            &half_bytes(pixels),
        )?);
    }
    let mut records = Vec::new();
    let mut snapshots = Vec::new();
    for settings in sky_reference::settings_cases(renderlab::Atmosphere::default()) {
        let params = AtmosphereParameters::for_camera_with_sun(
            &mut gpu,
            settings,
            AtmosphereSun {
                toward_sun: sun,
                illuminance: [2., 1., 0.5],
                exposure: 0.75,
            },
            camera,
            viewport,
            grid,
        )?;
        graph.import(Resource::Buffer(params.record.buffer), Access::HostWrite);
        pass.record(&mut graph, &mut rec, &params, &luts, &packed, &output)?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(viewport[0] * viewport[1] * 16),
        )?;
        snapshots.push((
            read,
            sky_reference::render(
                &model,
                &trans,
                &sky,
                &volume,
                settings,
                camera,
                viewport,
                [1.5, 0.75, 0.375],
            ),
        ));
        records.push(params);
    }
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    for (i, (read, expected)) in snapshots.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        let mut max = 0f32;
        let mut relative = 0f32;
        for (a, b) in actual.iter().zip(expected.pixels.iter().flatten()) {
            ensure!(a.is_finite(), "nonfinite sky");
            max = max.max((a - b).abs());
            relative = relative.max((a - b).abs() / b.abs().max(1.));
        }
        println!("sky flags {i}: max error {max}, normalized {relative}");
        ensure!(relative <= 0.0005, "sky flags {i} differs by {relative}");
        staging.push(read);
    }
    for p in records {
        p.retire(&mut gpu, &mut graph)?;
    }
    for t in [
        luts.transmittance,
        luts.multiscatter,
        luts.sky,
        packed,
        output,
    ] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
