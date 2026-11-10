//! Native Metal/Vulkan replays. All generated files stay in ignored target directories.
#![cfg(feature = "backend-ara")]
#[path = "ara_backend/reference.rs"]
mod reference;
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Device, GpuDevice, Queue, Resource,
    commands::{Command, Recorder},
    descriptors::Format,
    vk,
};
use half::f16;
use renderlab::backend_ara::{
    cache::{ShaderCache, emit_spirv},
    graph::Graph,
    image::Texture,
    post::{FilterInputs, PostPasses, SettingsRecord, ToneInputs},
    shader::Pass,
};
fn root(label: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("renderlab-ara")
        .join(format!("{label}-{}", std::process::id()))
}
fn half_bytes(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flatten()
        .flat_map(|v| f16::from_f32(*v).to_bits().to_le_bytes())
        .collect()
}
fn upload(
    gpu: &mut GpuDevice,
    graph: &mut Graph,
    rec: &mut Recorder,
    image: ara_gpu::Image,
    bytes: &[u8],
) -> Result<ara_gpu::Buffer> {
    let buffer = gpu.create_buffer(bytes.len() as u64, vk::BufferUsageFlags::TRANSFER_SRC, true)?;
    gpu.upload(buffer, bytes)?;
    graph.import(Resource::Buffer(buffer), Access::HostWrite);
    graph.node(
        rec,
        &[
            (Resource::Buffer(buffer), Access::CopySource),
            (Resource::Image(image), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::UploadImage {
        source: buffer,
        destination: image,
    });
    Ok(buffer)
}
#[test]
fn all_ported_stages_emit_bindless_spirv() -> Result<()> {
    let path = root("spirv");
    for pass in [
        Pass::Downsample,
        Pass::Upsample,
        Pass::Tone,
        Pass::Display,
        Pass::TileMax,
        Pass::MotionBlur,
        Pass::DepthOfField,
        Pass::LightCluster,
        Pass::AtmosphereTransmittance,
        Pass::AtmosphereMultiscatter,
        Pass::AtmosphereSky,
        Pass::AtmosphereAmbient,
        Pass::EnvironmentSpecular,
        Pass::EnvironmentDiffuse,
    ] {
        emit_spirv(pass, &path)?;
    }
    println!("SPIR-V output: {}", path.display());
    Ok(())
}
#[test]
fn bloom_and_tone_native_replay_against_cpu() -> Result<()> {
    let path = root("native");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = PostPasses::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let f = reference::fixture();
    let hdr = Texture::new(&mut gpu, f.hdr.size, Format::Rgba16Float, false, false)?;
    let down0 = Texture::new(&mut gpu, [9, 6], Format::Rgba16Float, true, false)?;
    let down1 = Texture::new(&mut gpu, [5, 3], Format::Rgba16Float, true, false)?;
    let up1 = Texture::new(&mut gpu, [5, 3], Format::Rgba16Float, true, false)?;
    let up0 = Texture::new(&mut gpu, [9, 6], Format::Rgba16Float, true, false)?;
    let encoded = Texture::new(&mut gpu, f.hdr.size, Format::Rgba8, false, true)?;
    let output = Texture::new(&mut gpu, f.hdr.size, Format::Rgba32Float, false, true)?;
    let lut = Texture::volume(&mut gpu, f.lut_size, Format::Rgba16Float)?;
    let settings = SettingsRecord::new(&mut gpu, &f.settings)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(settings.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = [
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            hdr.image,
            &half_bytes(&f.hdr.pixels),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            lut.image,
            &half_bytes(&f.lut),
        )?,
    ];
    for (pass, input, second, target) in [
        (Pass::Downsample, &hdr, &hdr, &down0),
        (Pass::Downsample, &down0, &hdr, &down1),
        (Pass::Upsample, &down1, &down1, &up1),
        (Pass::Upsample, &down0, &up1, &up0),
    ] {
        passes.filter(
            &mut graph,
            &mut rec,
            pass,
            &FilterInputs {
                colour: input,
                second,
                auxiliary: &hdr,
                velocity_tiles: &hdr,
                settings: &settings,
            },
            target,
        )?;
    }
    passes.tone(
        &mut graph,
        &mut rec,
        &ToneInputs {
            colour: &hdr,
            bloom: &up0,
            lut: &lut,
            settings: &settings,
        },
        &output,
    )?;
    passes.display(&mut graph, &mut rec, &output, &encoded)?;
    let d0 = reference::downsample(&f.hdr, [9, 6]);
    let d1 = reference::downsample(&d0, [5, 3]);
    let u1 = reference::upsample(&d1, &d1);
    let u0 = reference::upsample(&d0, &u1);
    let tone = reference::tone(&f.hdr, &u0, &f);
    let display = reference::display(&tone);
    let mut reads = Vec::new();
    for (texture, reference) in [down0, down1, up1, up0, output, encoded]
        .iter()
        .zip([d0, d1, u1, u0, tone, display])
    {
        let bpp = if texture.format == Format::Rgba32Float {
            16
        } else if texture.format == Format::Rgba8 {
            4
        } else {
            8
        };
        let read = gpu.create_readback_buffer(reference.pixels.len() as u64 * bpp)?;
        graph.node(
            &mut rec,
            &[
                (Resource::Image(texture.image), Access::CopySource),
                (Resource::Buffer(read), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::ReadImage {
            source: texture.image,
            destination: read,
        });
        graph.node(&mut rec, &[(Resource::Buffer(read), Access::HostRead)])?;
        reads.push((read, reference, bpp));
    }
    let point = gpu.submit(rec.finish(), &[])?;
    for buffer in staging {
        gpu.retire(Resource::Buffer(buffer))?;
    }
    gpu.wait(point)?;
    for (index, (read, reference, bpp)) in reads.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<f32> = if bpp == 16 {
            bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().expect("four bytes")))
                .collect()
        } else if bpp == 4 {
            bytes.iter().map(|v| *v as f32 / 255.).collect()
        } else {
            bytes
                .chunks_exact(2)
                .map(|v| {
                    f16::from_bits(u16::from_le_bytes(v.try_into().expect("two bytes"))).to_f32()
                })
                .collect()
        };
        ensure!(
            actual.len() == reference.pixels.len() * 4,
            "readback length"
        );
        let mut maximum = 0f32;
        for (a, b) in actual.iter().zip(reference.pixels.iter().flatten()) {
            ensure!(a.is_finite(), "nonfinite output");
            maximum = maximum.max((a - b).abs());
        }
        println!(
            "post stage {index}, {:?}, max error {maximum}",
            reference.size
        );
        ensure!(maximum <= 0.003, "post stage {index} differs by {maximum}");
        gpu.retire(Resource::Buffer(read))?;
    }
    passes.retire(&mut gpu)?;
    for texture in [hdr, down0, down1, up1, up0, output, encoded, lut] {
        gpu.retire(Resource::Image(texture.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

#[test]
fn velocity_motion_and_lens_native_replays_against_cpu() -> Result<()> {
    let path = root("filters");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = PostPasses::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let f = reference::fixture();
    let aux = reference::Image {
        size: f.hdr.size,
        pixels: (0..17 * 11)
            .map(|i| {
                let x = i % 17;
                let y = i / 17;
                if x < 7 {
                    [3., 1., 2., 1.]
                } else if y < 4 {
                    [-2., 2., 12., 1.]
                } else {
                    [0., 0., 5., 1.]
                }
            })
            .collect(),
    };
    let mut values = f.settings;
    values[2] = 6.;
    values[4] = 5.;
    values[5] = 6.;
    values[6] = 3.;
    values[7] = 1.;
    let c_tiles = reference::tile_max(&aux);
    let c_motion = reference::motion_blur(&f.hdr, &aux, &c_tiles, &values);
    let c_dof = reference::depth_of_field(&c_motion, &aux, &values);
    let hdr = Texture::new(&mut gpu, f.hdr.size, Format::Rgba16Float, false, false)?;
    let auxiliary = Texture::new(&mut gpu, aux.size, Format::Rgba32Float, false, false)?;
    let tiles = Texture::new(&mut gpu, c_tiles.size, Format::Rgba16Float, true, false)?;
    let motion = Texture::new(&mut gpu, f.hdr.size, Format::Rgba16Float, true, false)?;
    let dof = Texture::new(&mut gpu, f.hdr.size, Format::Rgba16Float, true, false)?;
    let settings = SettingsRecord::new(&mut gpu, &values)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(settings.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let raw_aux: Vec<_> = aux
        .pixels
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let staging = [
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            hdr.image,
            &half_bytes(&f.hdr.pixels),
        )?,
        upload(&mut gpu, &mut graph, &mut rec, auxiliary.image, &raw_aux)?,
    ];
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
        graph.forget(Resource::Buffer(b));
    }
    // Reuse both descriptors and scheduled states across eight submitted frames.
    for frame in 0..8 {
        let mut rec = gpu.begin(Queue::Graphics);
        for (pass, input, target) in [
            (Pass::TileMax, &hdr, &tiles),
            (Pass::MotionBlur, &hdr, &motion),
            (Pass::DepthOfField, &motion, &dof),
        ] {
            // The tile kernel has no tile read: supply initialized HDR in that unused slot.
            passes.filter(
                &mut graph,
                &mut rec,
                pass,
                &FilterInputs {
                    colour: input,
                    second: &hdr,
                    auxiliary: &auxiliary,
                    velocity_tiles: if matches!(pass, Pass::TileMax) {
                        &hdr
                    } else {
                        &tiles
                    },
                    settings: &settings,
                },
                target,
            )?;
        }
        let mut reads = Vec::new();
        for t in [tiles, motion, dof] {
            let read = renderlab::backend_ara::transfer::readback(
                &mut gpu,
                &mut graph,
                &mut rec,
                t.image,
                t.size[0] as u64 * t.size[1] as u64 * 8,
            )?;
            reads.push(read);
        }
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
        for (index, (read, reference)) in reads
            .into_iter()
            .zip([&c_tiles, &c_motion, &c_dof])
            .enumerate()
        {
            let bytes = gpu.read(read)?;
            let actual: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
                .collect();
            ensure!(
                actual.len() == reference.pixels.len() * 4,
                "filter readback length"
            );
            let mut maximum = 0f32;
            for (a, b) in actual.iter().zip(reference.pixels.iter().flatten()) {
                ensure!(a.is_finite(), "nonfinite filter output");
                maximum = maximum.max((a - b).abs());
            }
            if frame == 0 {
                println!("filter {index}, {:?}, max error {maximum}", reference.size);
            }
            ensure!(
                maximum <= 0.004,
                "filter {index} frame {frame}: error {maximum}"
            );
            gpu.retire(Resource::Buffer(read))?;
            graph.forget(Resource::Buffer(read));
        }
        gpu.collect()?;
    }
    passes.retire(&mut gpu)?;
    for t in [hdr, auxiliary, tiles, motion, dof] {
        gpu.retire(Resource::Image(t.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

#[path = "ara_backend/clusters.rs"]
mod clusters;
#[test]
fn packed_light_clusters_native_replay_against_cpu() -> Result<()> {
    use renderlab::backend_ara::light_cluster::{ClusterInputs, LightClusters};
    use renderlab::{Camera, Light, Lighting, Vec3};
    let path = root("clusters");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = LightClusters::new(&mut gpu, &cache)?;
    let target = LightClusters::target(&mut gpu)?;
    let camera = Camera::look_at(
        Vec3::new(1., 2., 3.),
        Vec3::new(0., 1., -4.),
        Vec3::Y,
        1.1,
        0.2,
        80.,
    )?;
    let mixed = vec![
        Light::Directional {
            travel: -Vec3::Y,
            lux: [1.; 3],
        },
        Light::Point {
            position: Vec3::new(0., 0., -6.),
            candela: [1.; 3],
            range: Some(3.),
            source_radius: 0.,
        },
        Light::Point {
            position: Vec3::new(100., 100., 100.),
            candela: [1.; 3],
            range: Some(1.),
            source_radius: 0.,
        },
        Light::Spot {
            position: Vec3::new(1., 2., 0.),
            travel: Vec3::new(0., -0.2, -1.),
            candela: [1.; 3],
            range: Some(12.),
            inner: 0.1,
            outer: 0.3,
            source_radius: 0.,
        },
        Light::Spot {
            position: Vec3::new(-3., 1., -4.),
            travel: -Vec3::Z,
            candela: [1.; 3],
            range: Some(8.),
            inner: 0.3,
            outer: 1.0,
            source_radius: 0.,
        },
        Light::Point {
            position: Vec3::ZERO,
            candela: [1.; 3],
            range: None,
            source_radius: 0.,
        },
    ];
    let overflow = (0..133)
        .map(|_| Light::Directional {
            travel: -Vec3::Y,
            lux: [1.; 3],
        })
        .collect();
    let mut graph = Graph::default();
    for (scenario, lights) in [mixed, overflow, Vec::new()].into_iter().enumerate() {
        let lighting = Lighting {
            lights,
            environment: [0.; 3],
            exposure_stops: 0.,
        };
        let inputs = ClusterInputs::new(&mut gpu, &lighting, camera, [320, 180])?;
        inputs.import(&mut graph);
        let mut rec = gpu.begin(Queue::Graphics);
        pass.record(&mut graph, &mut rec, &inputs, &target)?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            target.image,
            target.size[0] as u64 * target.size[1] as u64 * 4,
        )?;
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
        let bytes = gpu.read(read)?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        for c in 0..clusters::COUNT {
            let expected = clusters::expected(&lighting.lights, camera, [320, 180], c);
            let at = ((c / clusters::ROW) * target.size[0] + (c % clusters::ROW) * clusters::STRIDE)
                as usize;
            ensure!(
                words[at]
                    == expected.len().min(128) as u32 | ((expected.len().min(65535) as u32) << 16),
                "scenario {scenario} cluster {c} count differs"
            );
            for (index, &light) in expected.iter().take(128).enumerate() {
                let actual = (words[at + 1 + index / 2] >> (16 * (index % 2))) & 65535;
                ensure!(
                    actual == light,
                    "cluster {c} slot {index}: {actual} != {light}"
                );
            }
        }
        println!(
            "light clusters scenario {scenario}: {} exact cluster lists",
            clusters::COUNT
        );
        graph.forget(Resource::Buffer(inputs.lights.buffer));
        graph.forget(Resource::Buffer(inputs.frame.buffer));
        inputs.retire(&mut gpu)?;
        graph.forget(Resource::Buffer(read));
        gpu.retire(Resource::Buffer(read))?;
        gpu.collect()?;
    }
    pass.retire(&mut gpu)?;
    gpu.retire(Resource::Image(target.image))?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

#[path = "ara_backend/atmosphere.rs"]
mod atmosphere_reference;
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
    passes.retire(&mut gpu)?;
    params.record.retire(&mut gpu)?;
    for t in [targets.transmittance, targets.multiscatter, targets.sky] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for read in reads {
        gpu.retire(Resource::Buffer(read))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

#[path = "ara_backend/environment.rs"]
mod environment_reference;
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
    passes.retire(&mut gpu)?;
    params.record.retire(&mut gpu)?;
    for t in [input, specular, diffuse] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in [staging, read_spec, read_diff] {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
