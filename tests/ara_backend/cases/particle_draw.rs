//! Sorted original particle raster, depth rejection, soft fading and alpha blend.
use super::*;
use ara_gpu::sampler::SamplerDesc;
use renderlab::backend_ara::{
    atmosphere::AtmosphereParameters,
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    particle_billboard::{BillboardDraw, ParticleBillboard},
    shader::InputKey,
};
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
#[test]
fn particle_billboard_native_replay_against_cpu() -> Result<()> {
    let path = root("particle-draw");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = ParticleBillboard::new(&mut gpu, &cache, Format::Rgba16Float)?;
    for textured in [false, true] {
        let f = particle_draw_reference::fixture(textured);
        let buffers = [
            (
                key(0, 30),
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.frame))?,
            ),
            (
                key(0, 40),
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.particles))?,
            ),
            (
                key(0, 41),
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.sorted))?,
            ),
            (
                key(0, 42),
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.emitter))?,
            ),
            (
                key(3, 0),
                StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.lighting))?,
            ),
        ];
        let camera = renderlab::Camera::look_at(
            renderlab::Vec3::ZERO,
            renderlab::Vec3::NEG_Z,
            renderlab::Vec3::Y,
            1.,
            0.1,
            1000.,
        )?;
        let atmosphere = AtmosphereParameters::for_camera(
            &mut gpu,
            renderlab::Atmosphere::off(),
            renderlab::Vec3::Y,
            camera,
            f.viewport,
            [1, 1],
        )?;
        let depth = Texture::new(&mut gpu, f.viewport, Format::R32Float, false, false)?;
        let sprite = Texture::new(&mut gpu, f.sprite.size, Format::Rgba16Float, false, false)?;
        let trans = Texture::new(&mut gpu, [256, 65], Format::Rgba16Float, false, false)?;
        let volume = Texture::volume_shape(&mut gpu, [1, 1, 96], Format::Rgba16Float, false)?;
        let output = Texture::new(&mut gpu, f.viewport, Format::Rgba16Float, false, true)?;
        let mut values: Vec<_> = buffers
            .iter()
            .map(|(key, b)| (*key, Input::Buffer(b)))
            .collect();
        values.extend([
            (key(0, 32), Input::Image(&depth)),
            (key(0, 33), Input::Sampler(SamplerDesc::default())),
            (key(0, 36), Input::Image(&sprite)),
            (key(3, 3), Input::Sampler(SamplerDesc::default())),
            (key(3, 50), Input::Buffer(&atmosphere.record)),
            (key(3, 51), Input::Image(&trans)),
            (key(3, 53), Input::Image(&volume)),
        ]);
        ensure!(
            renderlab::backend_ara::shader::inputs(Pass::ParticleBillboard)?.len() == 12,
            "billboard interface changed"
        );
        let inputs = HeapBindings::new(&mut gpu, Pass::ParticleBillboard, &values)?;
        let mut graph = Graph::default();
        for (_, b) in &buffers {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
        graph.import(
            Resource::Buffer(atmosphere.record.buffer),
            Access::HostWrite,
        );
        inputs.import(&mut graph);
        let mut rec = gpu.begin(Queue::Graphics);
        let staging = [
            upload(&mut gpu, &mut graph, &mut rec, depth.image, &f.depth_bytes)?,
            upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                sprite.image,
                &f.sprite_bytes,
            )?,
            upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                trans.image,
                &half_bytes(&vec![[1., 1., 1., 0.]; 256 * 65]),
            )?,
            upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                volume.image,
                &half_bytes(&[[0., 0., 0., 1.]; 96]),
            )?,
        ];
        pass.record(
            &mut graph,
            &mut rec,
            &[BillboardDraw {
                inputs: &inputs,
                instances: f.draw_args[1],
            }],
            &output,
            Some(f.background),
        )?;
        let read = renderlab::backend_ara::transfer::readback(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(f.viewport[0] * f.viewport[1] * 8),
        )?;
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| f16::from_bits(u16::from_le_bytes(*v)).to_f32())
            .collect();
        let mut max = 0f32;
        let mut coverage = 0;
        for (index, (a, b)) in actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&f.expected.pixels)
            .enumerate()
        {
            if (*a != f.background) != (*b != f.background) {
                coverage += 1;
                for (channel, (x, y)) in a.iter().zip(b).enumerate() {
                    let clear = f16::from_f32(f.background[channel]);
                    let next = f16::from_bits(clear.to_bits() + 1).to_f32();
                    ensure!(
                        (x - y).abs() <= next - f.background[channel],
                        "clear boundary exceeds one half ULP at pixel{index}"
                    );
                }
            }
            for (x, y) in a.iter().zip(b) {
                ensure!(x.is_finite(), "nonfinite billboard");
                max = max.max((x - y).abs());
            }
        }
        println!(
            "particle billboards textured={textured}: max error {max}, clear-boundary pixels within one half ULP {coverage}"
        );
        ensure!(max <= 0.003, "particle billboard differs");
        inputs.retire(&mut gpu, &mut graph)?;
        atmosphere.retire(&mut gpu, &mut graph)?;
        for (_, b) in buffers {
            graph.forget(Resource::Buffer(b.buffer));
            b.retire(&mut gpu)?;
        }
        for t in [depth, sprite, trans, volume, output] {
            gpu.retire(Resource::Image(t.image))?;
        }
        for b in staging.into_iter().chain([read]) {
            gpu.retire(Resource::Buffer(b))?;
        }
    }
    pass.retire(&mut gpu)?;
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
