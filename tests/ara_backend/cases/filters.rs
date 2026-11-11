//! Native filters replay through the public backend encoder.
use super::*;
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
    for t in [hdr, auxiliary, tiles, motion, dof] {
        gpu.retire(Resource::Image(t.image))?;
    }
    gpu.retire(Resource::Buffer(settings.buffer))?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
