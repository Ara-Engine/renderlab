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
    for pass in [Pass::Downsample, Pass::Upsample, Pass::Tone] {
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
    let d0 = reference::downsample(&f.hdr, [9, 6]);
    let d1 = reference::downsample(&d0, [5, 3]);
    let u1 = reference::upsample(&d1, &d1);
    let u0 = reference::upsample(&d0, &u1);
    let tone = reference::tone(&f.hdr, &u0, &f);
    let mut reads = Vec::new();
    for (texture, reference) in [down0, down1, up1, up0, output]
        .iter()
        .zip([d0, d1, u1, u0, tone])
    {
        let bpp = if texture.format == Format::Rgba32Float {
            16
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
    for texture in [hdr, down0, down1, up1, up0, output, lut] {
        gpu.retire(Resource::Image(texture.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
