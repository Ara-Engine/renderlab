//! Full packed camera-volume replay, including untouched region preservation.
use super::*;
use renderlab::backend_ara::{
    atmosphere::{AtmosphereParameters, AtmosphereTargets},
    volumes::VolumePasses,
};
#[test]
fn aerial_and_fog_volume_native_replay_against_cpu() -> Result<()> {
    let grid = [3, 2];
    let viewport = [37, 23];
    let settings = renderlab::Atmosphere::default();
    let camera = renderlab::Camera::look_at(
        renderlab::Vec3::new(0., 1000., 0.),
        renderlab::Vec3::new(3., 1000.5, -10.),
        renderlab::Vec3::Y,
        1.1,
        0.1,
        1e5,
    )?;
    let model = atmosphere_reference::Model {
        medium: settings.medium,
        sun: renderlab::Vec3::new(0.4, 0.8, 0.2).normalize(),
        altitude: 1.,
    };
    let trans = model.transmittance();
    let multi = model.multiscatter(&trans);
    let ambient = [0.03125, 0.0625, 0.125];
    let scatter_cpu = volume_reference::scatter_fixture(grid);
    let aerial_cpu = volume_reference::aerial(&model, &trans, &multi, camera, viewport, grid);
    let fog_cpu = volume_reference::fog(
        &model,
        &trans,
        ambient,
        &scatter_cpu,
        settings,
        camera,
        viewport,
        grid,
    );
    let path = root("volumes");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = VolumePasses::new(&mut gpu, &cache)?;
    let params =
        AtmosphereParameters::for_camera(&mut gpu, settings, model.sun, camera, viewport, grid)?;
    let luts = AtmosphereTargets::new(&mut gpu)?;
    let scatter =
        Texture::volume_shape(&mut gpu, [grid[0], grid[1], 64], Format::Rgba16Float, false)?;
    let output =
        Texture::volume_shape(&mut gpu, [grid[0], grid[1], 96], Format::Rgba16Float, true)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(params.record.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let mut trans_pixels = trans.pixels.clone();
    trans_pixels.resize(256 * 65, [0.; 4]);
    trans_pixels[256 * 64] = [ambient[0], ambient[1], ambient[2], 1.];
    let mut staging = Vec::new();
    for (image, bytes) in [
        (luts.transmittance.image, half_bytes(&trans_pixels)),
        (luts.multiscatter.image, half_bytes(&multi.pixels)),
        (scatter.image, half_bytes(&scatter_cpu.pixels)),
        (output.image, vec![0; (grid[0] * grid[1] * 96 * 8) as usize]),
    ] {
        staging.push(upload(&mut gpu, &mut graph, &mut rec, image, &bytes)?);
    }
    passes.record(
        &mut graph,
        &mut rec,
        Pass::AtmosphereAerial,
        &params,
        &luts,
        &scatter,
        &output,
    )?;
    let bytes = u64::from(grid[0] * grid[1] * 96 * 8);
    let a = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        output.image,
        bytes,
    )?;
    passes.record(
        &mut graph,
        &mut rec,
        Pass::AtmosphereFogIntegrate,
        &params,
        &luts,
        &scatter,
        &output,
    )?;
    let b = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        output.image,
        bytes,
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let split = (grid[0] * grid[1] * 64) as usize;
    for (i, read) in [a, b].into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
            .collect();
        let mut max = 0f32;
        for (j, a) in actual.iter().enumerate() {
            let p = j / 4;
            let c = j % 4;
            let expected = if p >= split || i == 0 {
                aerial_cpu.pixels[p][c]
            } else {
                fog_cpu.pixels[p][c]
            };
            ensure!(a.is_finite(), "nonfinite camera volume");
            max = max.max((a - expected).abs());
            if i == 0 && p < split {
                ensure!(*a == 0., "aerial touched fog region");
            }
        }
        println!("camera volume stage {i}: max error {max}");
        ensure!(max <= 0.002, "camera volume stage {i} differs by {max}");
    }
    params.record.retire(&mut gpu)?;
    for t in [
        luts.transmittance,
        luts.multiscatter,
        luts.sky,
        scatter,
        output,
    ] {
        gpu.retire(Resource::Image(t.image))?;
    }
    staging.extend([a, b]);
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
