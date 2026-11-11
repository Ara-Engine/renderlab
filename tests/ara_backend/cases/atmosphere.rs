//! Native atmosphere replay through the public backend encoder.
use super::*;
#[test]
fn atmosphere_lut_chain_native_replay_against_cpu() -> Result<()> {
    use renderlab::backend_ara::atmosphere::{
        AtmosphereParameters, AtmospherePasses, AtmosphereTargets,
    };
    let model = atmosphere_reference::Model {
        medium: renderlab::AtmosphereMedium::default(),
        sun: renderlab::Vec3::new(0.4, 0.8, 0.2).normalize(),
        altitude: 1.0,
    };
    let trans = model.transmittance();
    let multi = model.multiscatter(&trans);
    let sky = model.sky(&trans, &multi);
    let ambient = model.ambient(&sky);
    let path = root("atmosphere");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = AtmospherePasses::new(&mut gpu, &cache)?;
    let targets = AtmosphereTargets::new(&mut gpu)?;
    let params = AtmosphereParameters::new(&mut gpu, model.medium, model.sun, model.altitude)?;
    let invalid = Texture::new(&mut gpu, [256, 65], Format::Rgba16Float, false, false)?;
    let invalid_targets = AtmosphereTargets {
        transmittance: invalid,
        multiscatter: targets.multiscatter,
        sky: targets.sky,
    };
    ensure!(
        passes
            .record(
                &mut Graph::default(),
                &mut gpu.begin(Queue::Graphics),
                Pass::AtmosphereTransmittance,
                &params,
                &invalid_targets
            )
            .is_err(),
        "missing LUT storage view was accepted"
    );
    gpu.retire(Resource::Image(invalid.image))?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(params.record.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = upload(
        &mut gpu,
        &mut graph,
        &mut rec,
        targets.transmittance.image,
        &vec![0; 256 * 65 * 8],
    )?;
    let mut reads = Vec::new();
    for (pass, target) in [
        (Pass::AtmosphereTransmittance, &targets.transmittance),
        (Pass::AtmosphereMultiscatter, &targets.multiscatter),
        (Pass::AtmosphereSky, &targets.sky),
        (Pass::AtmosphereAmbient, &targets.transmittance),
    ] {
        passes.record(&mut graph, &mut rec, pass, &params, &targets)?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            target.image,
            target.size[0] as u64 * target.size[1] as u64 * 8,
        )?;
        reads.push(read);
    }
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    gpu.retire(Resource::Buffer(staging))?;
    for (i, (read, reference)) in reads[..3].iter().zip([trans, multi, sky]).enumerate() {
        let bytes = gpu.read(*read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
            .collect();
        ensure!(
            actual.len() == reference.pixels.len() * 4,
            "atmosphere read length"
        );
        let mut max = 0f32;
        let mut max_at = 0;
        for (j, (a, b)) in actual
            .iter()
            .zip(reference.pixels.iter().flatten())
            .enumerate()
        {
            ensure!(a.is_finite(), "nonfinite atmosphere");
            if (a - b).abs() > max {
                max = (a - b).abs();
                max_at = j;
            }
        }
        println!(
            "atmosphere stage {i}: max error {max}, index {max_at}, actual {}, expected {}",
            actual[max_at],
            reference.pixels[max_at / 4][max_at % 4]
        );
        ensure!(
            max <= [0.001, 0.0005, 0.001][i],
            "atmosphere stage {i} differs by {max}"
        );
    }
    let bytes = gpu.read(reads[3])?;
    let actual: Vec<_> = bytes[256 * 64 * 8..][..8]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
        .collect();
    let mut max = 0f32;
    for (a, b) in actual.iter().zip(ambient) {
        ensure!(a.is_finite(), "nonfinite ambient");
        max = max.max((a - b).abs());
    }
    println!("atmosphere ambient: max error {max}");
    ensure!(max <= 0.0005, "atmosphere ambient differs by {max}");
    params.record.retire(&mut gpu)?;
    for t in [targets.transmittance, targets.multiscatter, targets.sky] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for read in reads {
        gpu.retire(Resource::Buffer(read))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
