//! Original programmable tiled and listed shading, with independent CPU pixels.
use super::*;
use ara_gpu::{sampler::SamplerDesc, texture::ViewDimension};
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    primary_shade::Entry,
    shader::{InputKey, InputKind, SampleType},
    transfer::readback,
};
use std::collections::BTreeMap;
fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
struct Batch {
    buffers: [StorageBuffer; 5],
    images: [Texture; 3],
    samplers: [SamplerDesc; 3],
}
#[test]
fn programmable_primary_tiles_and_lists_native_replay_against_cpu() -> Result<()> {
    let f = primary_shade_reference::fixture();
    ensure!(
        f.region == [0, 0, 16, 8]
            && f.tiles_expected == *f.list_expected.last().expect("two lists"),
        "CPU list partition mismatch"
    );
    let path = root("primary-shade");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    ensure!(
        Kernel::new(&mut gpu, &cache, Pass::PrimaryShadeTiles).is_err(),
        "conformance material silently used at runtime"
    );
    ensure!(
        HeapBindings::new(&mut gpu, Pass::PrimaryShadeList, &[])
            .err()
            .is_some_and(|e| e.to_string().contains("caller material WGSL")),
        "program-only packet accepted"
    );
    // Caller-authored text, not the Pass family's built-in replay material.
    let program = Entry::Tiles.program(primary_shade_reference::PROGRAM)?;
    let list_program = Entry::List.program(primary_shade_reference::PROGRAM)?;
    let kernel = Kernel::from_program(&mut gpu, &cache, &program)?;
    let list_kernel = Kernel::from_program(&mut gpu, &cache, &list_program)?;
    println!(
        "primary shade: {} tiled / {} listed inputs through 32-byte roots",
        program.inputs().len(),
        list_program.inputs().len()
    );
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();
    let mut buffers = BTreeMap::new();
    let mut images = BTreeMap::new();
    let draws: Vec<_> = f
        .batches
        .iter()
        .map(|b| (b.index, b.indices.len(), false))
        .collect();
    for (at, words) in &f.buffers {
        // The frame record gains its zones and the placements their batch words.
        let words = if *at == key(0, 10) {
            with_zones(words)
        } else if *at == key(1, 5) {
            placement_words(words, &draws)
        } else {
            words.clone()
        };
        buffers.insert(
            *at,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&words))?,
        );
    }
    buffers.insert(
        key(0, 39),
        StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&f.list_words))?,
    );
    let list_records: Vec<_> = f
        .lists
        .iter()
        .map(|l| StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&l.record)))
        .collect::<Result<_>>()?;
    let mut batches = BTreeMap::new();
    for b in &f.batches {
        let buffers = [
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.vertices))?,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.indices))?,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.tangents))?,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.material))?,
            StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&b.parameters))?,
        ];
        let mut sampled = Vec::new();
        for data in [&b.base, &b.mr, &b.normal] {
            let texture = if data.layers == 1 {
                Texture::new(&mut gpu, [2, 2], Format::Rgba16Float, false, false)?
            } else {
                Texture::array(
                    &mut gpu,
                    [2, 2],
                    data.layers,
                    Format::Rgba16Float,
                    false,
                    false,
                )?
            };
            staging.push(upload(
                &mut gpu,
                &mut graph,
                &mut rec,
                texture.image,
                &half_bytes(&data.pixels),
            )?);
            sampled.push(texture);
        }
        batches.insert(
            b.index,
            Batch {
                buffers,
                images: sampled
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("three fixture textures"))?,
                samplers: [b.base.sampler, b.mr.sampler, b.normal.sampler],
            },
        );
    }
    for spec in list_program.inputs() {
        if images.contains_key(&spec.key) {
            continue;
        }
        match spec.kind {
            InputKind::AccelerationStructure => anyhow::bail!("unexpected AS in non-RT shading"),
            InputKind::Image { dimension, sample } => {
                let format = match sample {
                    SampleType::Float => Format::Rgba16Float,
                    SampleType::Uint => {
                        if spec.key == key(0, 7) {
                            Format::Rgba32Uint
                        } else {
                            Format::R32Uint
                        }
                    }
                    SampleType::Depth => Format::Depth32Float,
                    SampleType::UintMultisampled => {
                        anyhow::bail!("unexpected multisampled image in non-RT shading")
                    }
                };
                let (size, layers) = if spec.key == key(0, 7) {
                    (f.size, 1)
                } else if spec.key == key(0, 21) {
                    (f.coverage_size, 1)
                } else {
                    (
                        [1, 1],
                        if matches!(dimension, ViewDimension::D2Array | ViewDimension::CubeArray) {
                            6
                        } else {
                            1
                        },
                    )
                };
                let t = match dimension {
                    ViewDimension::D2 => Texture::new(&mut gpu, size, format, false, false)?,
                    ViewDimension::D2Array => {
                        Texture::array(&mut gpu, size, layers, format, false, false)?
                    }
                    ViewDimension::D3 => Texture::volume(&mut gpu, 1, format)?,
                    ViewDimension::CubeArray => Texture::cubes(&mut gpu, 1, 1, format)?,
                };
                let bytes = if spec.key == key(0, 7) {
                    bytemuck::cast_slice(&f.hit).to_vec()
                } else if spec.key == key(0, 21) {
                    bytemuck::cast_slice(&f.coverage).to_vec()
                } else {
                    vec![
                        0;
                        size[0] as usize
                            * size[1] as usize
                            * layers as usize
                            * if sample == SampleType::Float { 8 } else { 4 }
                    ]
                };
                staging.push(upload(&mut gpu, &mut graph, &mut rec, t.image, &bytes)?);
                images.insert(spec.key, t);
            }
            InputKind::StorageImage { .. } => {
                ensure!(spec.key == key(0, 18), "unexpected shading storage image");
                let t = Texture::new(&mut gpu, f.size, Format::Rgba16Float, true, false)?;
                staging.push(upload(
                    &mut gpu,
                    &mut graph,
                    &mut rec,
                    t.image,
                    &half_bytes(&f.initial),
                )?);
                images.insert(spec.key, t);
            }
            InputKind::Buffer { minimum, .. } => {
                if spec.key == key(0, 38) {
                    continue;
                }
                if !buffers.contains_key(&spec.key) {
                    anyhow::bail!(
                        "missing scalar fixture {} {:?} min{minimum}",
                        spec.name,
                        spec.key
                    );
                }
            }
            _ => {}
        }
    }
    let dummy = StorageBuffer::new(&mut gpu, &[0; 64])?;
    let arrays: Vec<Vec<_>> = (0..5)
        .map(|which| {
            (0..renderlab::TRACED_DRAWS)
                .map(|i| batches.get(&i).map_or(&dummy, |b| &b.buffers[which]))
                .collect()
        })
        .collect();
    let fallback = &batches[&3];
    let image_arrays: Vec<Vec<_>> = (0..3)
        .map(|which| {
            (0..renderlab::TRACED_DRAWS)
                .map(|i| &batches.get(&i).unwrap_or(fallback).images[which])
                .collect()
        })
        .collect();
    let sampler_arrays: Vec<Vec<_>> = (0..3)
        .map(|which| {
            (0..renderlab::TRACED_DRAWS)
                .map(|i| batches.get(&i).unwrap_or(fallback).samplers[which])
                .collect()
        })
        .collect();
    let values = |program: &renderlab::backend_ara::shader::Program,
                  list: Option<usize>|
     -> Result<Vec<_>> {
        program
            .inputs()
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match spec.kind {
                        InputKind::AccelerationStructure => {
                            anyhow::bail!("unexpected AS in non-RT shading")
                        }
                        InputKind::Buffer { .. } => Input::Buffer(if spec.key == key(0, 38) {
                            &list_records[list.expect("list entry")]
                        } else {
                            &buffers[&spec.key]
                        }),
                        InputKind::BufferArray { .. } => Input::Buffers(
                            &arrays[match spec.key {
                                InputKey {
                                    group: 1,
                                    binding: 2,
                                } => 0,
                                InputKey {
                                    group: 1,
                                    binding: 3,
                                } => 1,
                                InputKey {
                                    group: 1,
                                    binding: 4,
                                } => 2,
                                InputKey {
                                    group: 2,
                                    binding: 10,
                                } => 3,
                                InputKey {
                                    group: 2,
                                    binding: 17,
                                } => 4,
                                _ => anyhow::bail!("unexpected array {}", spec.name),
                            }],
                        ),
                        InputKind::ImageArray { .. } => Input::Images(
                            &image_arrays[match spec.key.binding {
                                11 => 0,
                                13 => 1,
                                15 => 2,
                                _ => anyhow::bail!("unexpected image array"),
                            }],
                        ),
                        InputKind::SamplerArray { .. } => Input::Samplers(
                            &sampler_arrays[match spec.key.binding {
                                12 => 0,
                                14 => 1,
                                16 => 2,
                                _ => anyhow::bail!("unexpected sampler array"),
                            }],
                        ),
                        InputKind::Image { .. } | InputKind::StorageImage { .. } => {
                            Input::Image(&images[&spec.key])
                        }
                        InputKind::Sampler { comparison } => {
                            ensure!(!comparison, "unexpected scalar comparison sampler");
                            Input::Sampler(SamplerDesc::default())
                        }
                    },
                ))
            })
            .collect()
    };
    let packet = HeapBindings::for_program(&mut gpu, &program, &values(&program, None)?)?;
    let lists: Vec<_> = (0..f.lists.len())
        .map(|i| {
            HeapBindings::for_program(&mut gpu, &list_program, &values(&list_program, Some(i))?)
        })
        .collect::<Result<_>>()?;
    for b in buffers
        .values()
        .chain(batches.values().flat_map(|b| &b.buffers))
        .chain(list_records.iter())
        .chain([&dummy])
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    let args_words: Vec<u32> = f.lists.iter().flat_map(|l| l.dispatch).collect();
    let args_source = StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&args_words))?;
    let args = StorageBuffer::with_usage(
        &mut gpu,
        &vec![0; args_words.len() * 4],
        vk::BufferUsageFlags::INDIRECT_BUFFER,
    )?;
    for b in [&args_source, &args] {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    graph.node(
        &mut rec,
        &[
            (Resource::Buffer(args_source.buffer), Access::CopySource),
            (Resource::Buffer(args.buffer), Access::CopyDestination),
        ],
    )?;
    rec.push(Command::CopyBufferRange {
        source: args_source.buffer,
        source_offset: 0,
        destination: args.buffer,
        destination_offset: 0,
        size: args.len,
    });
    packet.import(&mut graph);
    let threads = program.threads()?;
    let extent = std::array::from_fn(|i| f.tiles_dispatch[i] * threads[i]);
    kernel.record(&mut graph, &mut rec, &packet, extent)?;
    let hdr = images[&key(0, 18)];
    let bytes = u64::from(f.size[0]) * u64::from(f.size[1]) * 8;
    let mut snapshots = vec![readback(&mut gpu, &mut graph, &mut rec, hdr.image, bytes)?];
    gpu.submit(rec.finish(), &[])?;
    rec = gpu.begin(Queue::Graphics);
    staging.push(upload(
        &mut gpu,
        &mut graph,
        &mut rec,
        hdr.image,
        &half_bytes(&f.initial),
    )?);
    let mut last = None;
    for (i, packet) in lists.iter().enumerate() {
        packet.import(&mut graph);
        list_kernel.record_indirect(&mut graph, &mut rec, packet, &args, i as u64 * 12)?;
        snapshots.push(readback(&mut gpu, &mut graph, &mut rec, hdr.image, bytes)?);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two shading lists"))?;
    for (i, read) in snapshots.into_iter().enumerate() {
        let data = gpu.read(read)?;
        let actual: Vec<[f32; 4]> = data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
                })
            })
            .collect();
        let expected = if i == 0 {
            &f.tiles_expected
        } else {
            &f.list_expected[i - 1]
        };
        primary_shade_reference::assert_matches(&actual, expected);
        let error = actual
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        println!(
            "primary programmable shade{i}:171pixels maxerror{error}; exact list-skip sentinels"
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    packet.retire(&mut gpu, &mut graph)?;
    for p in lists {
        p.retire(&mut gpu, &mut graph)?;
    }
    for b in buffers.into_values() {
        b.retire(&mut gpu)?;
    }
    // Array owners are retired once, after every packet borrowing them.
    for batch in batches.into_values() {
        for b in batch.buffers {
            b.retire(&mut gpu)?;
        }
        for t in batch.images {
            gpu.retire(Resource::Image(t.image))?;
        }
    }
    for b in list_records.into_iter().chain([dummy, args_source, args]) {
        b.retire(&mut gpu)?;
    }
    for t in images.into_values() {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
