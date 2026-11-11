//! Native environment replay through the public backend encoder.
use super::*;
#[test]
fn environment_array_mips_native_replay_against_cpu() -> Result<()> {
    use renderlab::backend_ara::{
        atmosphere::AtmosphereParameters, environment::EnvironmentPasses,
    };
    let model = atmosphere_reference::Model {
        medium: renderlab::AtmosphereMedium::default(),
        sun: renderlab::Vec3::new(0.4, 0.8, 0.2).normalize(),
        altitude: 1.,
    };
    let sky = environment_reference::sky();
    let path = root("environment");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = EnvironmentPasses::new(&mut gpu, &cache)?;
    let params = AtmosphereParameters::new(&mut gpu, model.medium, model.sun, model.altitude)?;
    let input = Texture::new(&mut gpu, sky.size, Format::Rgba16Float, false, false)?;
    let (specular, mips) = Texture::cube_chain(&mut gpu, 128, 5)?;
    let (diffuse, view) = Texture::cube_chain(&mut gpu, 16, 1)?;
    let invalid = Texture::new(&mut gpu, [16, 16], Format::Rgba32Float, true, false)?;
    ensure!(
        passes
            .record(
                &mut Graph::default(),
                &mut gpu.begin(Queue::Graphics),
                Pass::EnvironmentDiffuse,
                &params,
                &input,
                &invalid
            )
            .is_err(),
        "non-array environment view was accepted"
    );
    gpu.retire(Resource::Image(invalid.image))?;
    let (invalid, mip) = Texture::cube_chain(&mut gpu, 4, 1)?;
    ensure!(
        passes
            .record(
                &mut Graph::default(),
                &mut gpu.begin(Queue::Graphics),
                Pass::EnvironmentSpecular,
                &params,
                &input,
                &mip[0]
            )
            .is_err(),
        "wrong specular roughness extent was accepted"
    );
    gpu.retire(Resource::Image(invalid.image))?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(params.record.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = upload(
        &mut gpu,
        &mut graph,
        &mut rec,
        input.image,
        &half_bytes(&sky.pixels),
    )?;
    for view in &mips {
        passes.record(
            &mut graph,
            &mut rec,
            Pass::EnvironmentSpecular,
            &params,
            &input,
            view,
        )?;
    }
    passes.record(
        &mut graph,
        &mut rec,
        Pass::EnvironmentDiffuse,
        &params,
        &input,
        &view[0],
    )?;
    let mut references = Vec::new();
    let mut sizes = Vec::new();
    for side in [128, 64, 32, 16, 8] {
        let r = environment_reference::cube(&model, &sky, side, false);
        sizes.push(r.pixels.len());
        references.extend(r.pixels);
    }
    let read_spec = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        specular.image,
        references.len() as u64 * 16,
    )?;
    let ref_diff = environment_reference::cube(&model, &sky, 16, true);
    let read_diff = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        diffuse.image,
        ref_diff.pixels.len() as u64 * 16,
    )?;
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    for (label, read, pixels) in [
        ("specular", read_spec, references),
        ("diffuse", read_diff, ref_diff.pixels),
    ] {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        ensure!(actual.len() == pixels.len() * 4, "cube readback length");
        let mut max = 0f32;
        let mut max_at = 0;
        for (j, (a, b)) in actual.iter().zip(pixels.iter().flatten()).enumerate() {
            ensure!(a.is_finite(), "nonfinite environment");
            if (a - b).abs() > max {
                max = (a - b).abs();
                max_at = j;
            }
        }
        println!(
            "environment {label}: {} texels, max error {max}",
            pixels.len()
        );
        if label == "specular" && max_at / 4 < 128 * 128 * 6 {
            println!(
                "specular max texel {}, channel {}, ideal {}, actual {}, quantized probe {}",
                max_at / 4,
                max_at % 4,
                pixels[max_at / 4][max_at % 4],
                actual[max_at],
                environment_reference::base_probe(&model, &sky, max_at / 4)[max_at % 4]
            );
        }
        if label == "specular" {
            let mut offset = 0;
            for (level, texels) in sizes.iter().enumerate() {
                let count = texels * 4;
                let error = actual[offset..offset + count]
                    .iter()
                    .zip(pixels.iter().flatten().skip(offset).take(count))
                    .map(|(a, b)| (a - b).abs())
                    .fold(0f32, f32::max);
                println!("specular mip {level}: max error {error}");
                // A single unfiltered HDR lookup exposes fixed-point interpolation
                // fractions and half-texture filter rounding. The rougher levels
                // integrate 64 samples and retain the tighter aggregate bound.
                ensure!(
                    error <= if level == 0 { 0.01 } else { 0.004 },
                    "specular mip {level} differs by {error}"
                );
                offset += count;
            }
        } else {
            ensure!(max <= 0.004, "diffuse environment differs by {max}");
        }
    }
    params.record.retire(&mut gpu)?;
    for t in [input, specular, diffuse] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in [staging, read_spec, read_diff] {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
