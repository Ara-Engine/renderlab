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
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/ara-backend")
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
