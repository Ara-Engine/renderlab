//! Persistent six-level bloom and settings updated with eight frames in flight.
use super::*;
use renderlab::backend_ara::bloom::Bloom;
#[test]
fn full_bloom_and_queued_tone_settings_native_replay() -> Result<()> {
    let mut fixture = reference::fixture();
    let size = [65, 33];
    fixture.hdr = reference::Image {
        size,
        pixels: (0..size[0] * size[1])
            .map(|i| {
                let x = (i % size[0]) as f32;
                let y = (i / size[0]) as f32;
                reference::half([
                    0.05 + x * 0.03,
                    0.08 + y * 0.04,
                    0.1 + (x + y) * 0.01 + if i == 433 { 8. } else { 0. },
                    1.,
                ])
            })
            .collect(),
    };
    let mut levels = Vec::new();
    let mut current = fixture.hdr.clone();
    for _ in 0..6 {
        let next = reference::downsample(&current, current.size.map(|n| n.div_ceil(2)));
        levels.push(next.clone());
        current = next;
        if current.size == [1, 1] {
            break;
        }
    }
    let mut expected_bloom = reference::upsample(levels.last().unwrap(), levels.last().unwrap());
    for input in levels[..levels.len() - 1].iter().rev() {
        expected_bloom = reference::upsample(input, &expected_bloom);
    }
    let path = root("post-runtime");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = PostPasses::new(&mut gpu, &cache, Format::Rgba32Float)?;
    let hdr = Texture::new(&mut gpu, size, Format::Rgba16Float, false, false)?;
    let lut = Texture::volume(&mut gpu, fixture.lut_size, Format::Rgba16Float)?;
    let output = Texture::new(&mut gpu, size, Format::Rgba32Float, false, true)?;
    let settings = SettingsRecord::new(&mut gpu, &fixture.settings)?;
    let bloom = Bloom::new(&mut gpu, size)?;
    let mut graph = Graph::default();
    graph.import(Resource::Buffer(settings.buffer), Access::HostWrite);
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = [
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            hdr.image,
            &half_bytes(&fixture.hdr.pixels),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            lut.image,
            &half_bytes(&fixture.lut),
        )?,
    ];
    bloom.record(&passes, &mut graph, &mut rec, &hdr, &settings)?;
    let bloom_read = renderlab::backend_ara::transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        bloom.output().image,
        (expected_bloom.pixels.len() * 8) as u64,
    )?;
    let mut snapshots = Vec::new();
    let mut last = None;
    for mode in 0..8u32 {
        fixture.settings[15] = (mode & 1) as f32;
        fixture.settings[17] = if mode & 2 == 0 { 0. } else { 0.5 };
        fixture.settings[16] = if mode & 4 == 0 { 0. } else { 1.75 };
        fixture.settings[11] = if mode & 4 == 0 { 0. } else { 0.08 };
        fixture.settings[3] = (mode * 7 + 1) as f32;
        settings.update(&mut graph, &mut rec, &fixture.settings)?;
        passes.tone(
            &mut graph,
            &mut rec,
            &ToneInputs {
                colour: &hdr,
                bloom: bloom.output(),
                lut: &lut,
                settings: &settings,
            },
            &output,
        )?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(size[0] * size[1] * 16),
        )?;
        snapshots.push((
            read,
            reference::tone(&fixture.hdr, &expected_bloom, &fixture),
        ));
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    // No host wait or host mutation between the eight settings/read submissions.
    gpu.wait(last.expect("eight tone submissions"))?;
    let bytes = gpu.read(bloom_read)?;
    let actual: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
        .collect();
    let max = actual
        .iter()
        .zip(expected_bloom.pixels.iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    println!("six-level persistent bloom: max error {max}");
    ensure!(max <= 0.004, "full bloom differs by{max}");
    let mut reads = vec![bloom_read];
    for (i, (read, expected)) in snapshots.into_iter().enumerate() {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        let mut max = 0f32;
        for (a, b) in actual.iter().zip(expected.pixels.iter().flatten()) {
            ensure!(a.is_finite(), "nonfinite tone");
            max = max.max((a - b).abs());
        }
        println!("queued tone mode{i}: max error {max}");
        ensure!(max <= 0.003, "queued tone differs by{max}");
        reads.push(read);
    }
    bloom.retire(&mut gpu)?;
    for t in [hdr, lut, output] {
        gpu.retire(Resource::Image(t.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    reads.extend(staging);
    for b in reads {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
