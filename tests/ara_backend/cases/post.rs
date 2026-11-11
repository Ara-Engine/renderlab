//! Native post replay through the public backend encoder.
use super::*;
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
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| f32::from_le_bytes(*v))
                .collect()
        } else if bpp == 4 {
            bytes.iter().map(|v| *v as f32 / 255.).collect()
        } else {
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
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
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
