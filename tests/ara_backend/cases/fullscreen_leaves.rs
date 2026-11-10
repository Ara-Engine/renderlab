//! Exact multisample background copies and original fallback tone mapping.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    fallback_present::FallbackPresent,
    shader::InputKey,
    transfer::readback,
    transparency_seed::{MsaaColor, Seed},
};
#[test]
fn transparency_msaa_seed_and_original_fallback_present_native_replay() -> Result<()> {
    let path = root("fullscreen-leaves");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let size = [7, 5];
    let source = Texture::new(&mut gpu, size, Format::Rgba16Float, false, false)?;
    let multisample = MsaaColor::new(&mut gpu, size)?;
    let fresh = MsaaColor::new(&mut gpu, size)?;
    let resolved = Texture::new(&mut gpu, size, Format::Rgba16Float, false, true)?;
    let repeated = Texture::new(&mut gpu, size, Format::Rgba16Float, false, true)?;
    let output = Texture::new(&mut gpu, size, Format::Rgba16Float, false, true)?;
    let wrong = Texture::new(&mut gpu, [3, 2], Format::Rgba16Float, false, true)?;
    let seed = Seed::new(&mut gpu, &cache)?;
    let present = FallbackPresent::new(&mut gpu, &cache, Format::Rgba16Float)?;
    let seed_packet = HeapBindings::new(
        &mut gpu,
        Pass::TransparencySeed,
        &[(
            InputKey {
                group: 0,
                binding: 31,
            },
            Input::Image(&source),
        )],
    )?;
    let present_packet = HeapBindings::new(
        &mut gpu,
        Pass::FallbackPresent,
        &[(
            InputKey {
                group: 0,
                binding: 1,
            },
            Input::Image(&resolved),
        )],
    )?;
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    ensure!(
        fresh.resolve_to(&mut graph, &mut rec, &resolved).is_err(),
        "uninitialized MSAA resolved"
    );
    ensure!(
        multisample
            .resolve_to(&mut graph, &mut rec, &wrong)
            .is_err(),
        "wrong resolve size accepted"
    );
    ensure!(
        present
            .record(&mut graph, &mut rec, &seed_packet, &output)
            .is_err(),
        "wrong fullscreen packet accepted"
    );
    seed_packet.import(&mut graph);
    present_packet.import(&mut graph);
    let mut staging = Vec::new();
    let mut snapshots = Vec::new();
    let mut expected = Vec::new();
    let mut last = None;
    for frame in 0..2 {
        let pixels: Vec<[f32; 4]> = (0..size[0] * size[1])
            .map(|i| {
                std::array::from_fn(|c| {
                    let value = if c == 3 {
                        (i % 5) as f32 * 0.25
                    } else {
                        ((i * (c as u32 + 1) + frame * 11) % 97) as f32 * 0.125
                    };
                    f16::from_f32(value).to_f32()
                })
            })
            .collect();
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            source.image,
            &half_bytes(&pixels),
        )?);
        seed.record(&mut graph, &mut rec, &seed_packet, &multisample)?;
        multisample.resolve_to(&mut graph, &mut rec, &resolved)?;
        multisample.resolve_to(&mut graph, &mut rec, &repeated)?;
        let a = readback(&mut gpu, &mut graph, &mut rec, resolved.image, 35 * 8)?;
        let b = readback(&mut gpu, &mut graph, &mut rec, repeated.image, 35 * 8)?;
        present.record(&mut graph, &mut rec, &present_packet, &output)?;
        let c = readback(&mut gpu, &mut graph, &mut rec, output.image, 35 * 8)?;
        snapshots.push([a, b, c]);
        expected.push(pixels);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two queued fullscreen frames"))?;
    for (frame, ([a, b, c], expected)) in snapshots.into_iter().zip(expected).enumerate() {
        let exact = half_bytes(&expected);
        ensure!(
            gpu.read(a)? == exact && gpu.read(b)? == exact,
            "4x seed/retained second resolve differs frame{frame}"
        );
        let bytes = gpu.read(c)?;
        let actual: Vec<[f32; 4]> = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|j| {
                    f16::from_bits(u16::from_le_bytes([p[j * 2], p[j * 2 + 1]])).to_f32()
                })
            })
            .collect();
        let mut error = 0f32;
        for (actual, input) in actual.iter().zip(expected) {
            for j in 0..3 {
                let reference = f16::from_f32(input[j] / (1. + input[j])).to_f32();
                error = error.max((actual[j] - reference).abs());
            }
            ensure!(actual[3] == 1., "present alpha differs");
        }
        ensure!(
            actual.iter().flatten().all(|v| v.is_finite()) && error <= 0.00048828125,
            "fallback present maxerror{error}"
        );
        println!(
            "fullscreen leaves{frame}:35 exact MSAA-seed texels and retained repeated resolve; fallback maxerror{error}"
        );
        for read in [a, b, c] {
            gpu.retire(Resource::Buffer(read))?;
        }
    }
    seed_packet.retire(&mut gpu, &mut graph)?;
    present_packet.retire(&mut gpu, &mut graph)?;
    seed.retire(&mut gpu)?;
    present.retire(&mut gpu)?;
    multisample.retire(&mut gpu, &mut graph)?;
    fresh.retire(&mut gpu, &mut graph)?;
    for t in [source, resolved, repeated, output, wrong] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
