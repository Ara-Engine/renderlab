//! Fixed handle-array packet conformance, separate from original renderer entries.
use super::*;
use ara_gpu::sampler::{AddressMode, CompareOp, Filter, SamplerDesc};
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::{InputKey, Program},
    transfer::readback_buffer,
};
fn key(binding: u32) -> InputKey {
    InputKey { group: 0, binding }
}
fn sample(data: &[[f32; 4]], uv: [f32; 2], sampler: SamplerDesc) -> [f32; 4] {
    let texel = |x: i32, y: i32| {
        let wrap = |i: i32, mode: AddressMode| -> usize {
            match mode {
                AddressMode::ClampToEdge => i.clamp(0, 1) as usize,
                AddressMode::Repeat => i.rem_euclid(2) as usize,
                AddressMode::MirrorRepeat => {
                    let i = i.rem_euclid(4);
                    (if i >= 2 { 3 - i } else { i }) as usize
                }
            }
        };
        data[wrap(y, sampler.address[1]) * 2 + wrap(x, sampler.address[0])]
    };
    if sampler.min == Filter::Nearest {
        return texel((uv[0] * 2.).floor() as i32, (uv[1] * 2.).floor() as i32);
    }
    let q = uv.map(|u| u * 2. - 0.5);
    let xy = q.map(|v| v.floor() as i32);
    let f = [q[0] - xy[0] as f32, q[1] - xy[1] as f32];
    let a = texel(xy[0], xy[1]);
    let b = texel(xy[0] + 1, xy[1]);
    let c = texel(xy[0], xy[1] + 1);
    let d = texel(xy[0] + 1, xy[1] + 1);
    std::array::from_fn(|k| {
        (a[k] * (1. - f[0]) + b[k] * f[0]) * (1. - f[1]) + (c[k] * (1. - f[0]) + d[k] * f[0]) * f[1]
    })
}
#[test]
fn scattered_image_sampler_and_comparison_arrays_native_replay() -> Result<()> {
    let unsupported = Program::new(
        Pass::HandleArrayProbe,
        r#"
        enable wgpu_binding_array;
        @group(0) @binding(0) var images: binding_array<texture_storage_2d<rgba16float,write>,4>;
        @compute @workgroup_size(1) fn probe() { textureStore(images[0],vec2(0),vec4(1.0)); }
    "#,
    );
    ensure!(unsupported.is_err(), "storage-image binding array accepted");
    let path = root("handle-arrays");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let kernel = Kernel::new(&mut gpu, &cache, Pass::HandleArrayProbe)?;
    let authored = Program::new(
        Pass::HandleArrayProbe,
        include_str!("../../../src/backend_ara/shaders/handle_array_probe.wgsl")
            .replace("@workgroup_size(4)", "@workgroup_size(2)"),
    )?;
    ensure!(
        authored.threads()? == [2, 1, 1],
        "authored workgroup size ignored"
    );
    let authored_kernel = Kernel::from_program(&mut gpu, &cache, &authored)?;
    let export = path.join("authored-spirv");
    let stages = renderlab::backend_ara::cache::emit_spirv_program(&authored, &export)?;
    for stage in &stages {
        let bytes = std::fs::read(export.join(&stage.spirv))?;
        let words: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        assert_nonuniform_resources(&words)?;
        let mut at = 5;
        let mut local_size = None;
        while at < words.len() {
            let n = (words[at] >> 16) as usize;
            if words[at] & 0xffff == 16 && n == 6 && words[at + 2] == 17 {
                local_size = Some(&words[at + 3..at + 6]);
            }
            at += n;
        }
        ensure!(
            local_size == Some(&[2, 1, 1][..]),
            "authored SPIR-V lost workgroup size"
        );
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let validator =
            std::path::Path::new(&sdk)
                .join("bin")
                .join(if cfg!(target_os = "windows") {
                    "spirv-val.exe"
                } else {
                    "spirv-val"
                });
        if validator.is_file() {
            for stage in stages {
                let output = std::process::Command::new(&validator)
                    .args(["--target-env", "vulkan1.3"])
                    .arg(export.join(stage.spirv))
                    .output()?;
                ensure!(
                    output.status.success(),
                    "authored SPIR-V: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    let mut textures = Vec::new();
    let mut query_images = Vec::new();
    let query_sizes = [[1, 1], [2, 1], [1, 2], [2, 2]];
    let query_levels = [1, 2, 1, 2];
    let mut depths = Vec::new();
    let mut staging = Vec::new();
    let pixels: Vec<Vec<[f32; 4]>> = (0..4)
        .map(|i| {
            (0..4)
                .map(|j| std::array::from_fn(|k| (i * 16 + j * 4 + k) as f32 / 16.))
                .collect()
        })
        .collect();
    let depth_values = [0.25f32, 0.75, 0.5, 1.];
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    for i in 0..4 {
        let (query, _) = Texture::mip_chain(
            &mut gpu,
            query_sizes[i],
            query_levels[i],
            Format::Rgba16Float,
        )?;
        let bytes = (0..query_levels[i])
            .map(|m| (query_sizes[i][0] >> m).max(1) * (query_sizes[i][1] >> m).max(1) * 8)
            .sum::<u32>();
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            query.image,
            &vec![0; bytes as usize],
        )?);
        query_images.push(query);
        let t = Texture::new(&mut gpu, [2, 2], Format::Rgba16Float, false, false)?;
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            t.image,
            &half_bytes(&pixels[i]),
        )?);
        textures.push(t);
        let d = Texture::new(&mut gpu, [2, 2], Format::Depth32Float, false, false)?;
        staging.push(upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            d.image,
            bytemuck::cast_slice(&[depth_values[i]; 4]),
        )?);
        depths.push(d);
    }
    let filters = std::array::from_fn::<_, 4, _>(|i| SamplerDesc {
        min: if i % 2 == 0 {
            Filter::Nearest
        } else {
            Filter::Linear
        },
        mag: if i % 2 == 0 {
            Filter::Nearest
        } else {
            Filter::Linear
        },
        mip: Filter::Nearest,
        address: [match i {
            0 => AddressMode::ClampToEdge,
            1 => AddressMode::Repeat,
            _ => AddressMode::MirrorRepeat,
        }; 3],
        ..Default::default()
    });
    let comparisons = [
        CompareOp::Less,
        CompareOp::Greater,
        CompareOp::LessEqual,
        CompareOp::Never,
    ]
    .map(|compare| SamplerDesc {
        min: Filter::Nearest,
        mag: Filter::Nearest,
        mip: Filter::Nearest,
        compare: Some(compare),
        ..Default::default()
    });
    let output = StorageBuffer::new(&mut gpu, &[0xa5; 320])?;
    graph.import(Resource::Buffer(output.buffer), Access::HostWrite);
    let image_maps = [[2, 0, 3, 2], [3, 2, 0, 3]];
    let depth_maps = [[1, 3, 0, 1], [2, 0, 3, 2]];
    let image_refs = image_maps.map(|m| m.map(|i| &textures[i]));
    let query_refs = image_maps.map(|m| m.map(|i| &query_images[i]));
    let depth_refs = depth_maps.map(|m| m.map(|i| &depths[i]));
    let sampler_sets = [filters, [filters[3], filters[1], filters[0], filters[3]]];
    let comparison_sets = [
        comparisons,
        [
            comparisons[2],
            comparisons[0],
            comparisons[1],
            comparisons[2],
        ],
    ];
    let values = |frame: usize| {
        [
            (key(0), Input::Images(&image_refs[frame])),
            (key(1), Input::Samplers(&sampler_sets[frame])),
            (key(2), Input::Images(&depth_refs[frame])),
            (key(3), Input::Samplers(&comparison_sets[frame])),
            (key(4), Input::Buffer(&output)),
            (key(5), Input::Images(&query_refs[frame])),
        ]
    };
    let mut bad = values(0);
    bad[0].1 = Input::Images(&image_refs[0][..3]);
    ensure!(
        HeapBindings::new(&mut gpu, Pass::HandleArrayProbe, &bad).is_err(),
        "short image array accepted"
    );
    bad = values(0);
    bad[0].1 = Input::Images(&depth_refs[0]);
    ensure!(
        HeapBindings::new(&mut gpu, Pass::HandleArrayProbe, &bad).is_err(),
        "depth as colour array accepted"
    );
    bad = values(0);
    bad[3].1 = Input::Samplers(&filters);
    ensure!(
        HeapBindings::new(&mut gpu, Pass::HandleArrayProbe, &bad).is_err(),
        "wrong comparison heap accepted"
    );
    let packets = [
        HeapBindings::for_program(&mut gpu, &authored, &values(0))?,
        HeapBindings::new(&mut gpu, Pass::HandleArrayProbe, &values(1))?,
    ];
    let other_program = Program::new(
        Pass::HandleArrayProbe,
        include_str!("../../../src/backend_ara/shaders/handle_array_probe.wgsl")
            .replace("@binding(4)", "@binding(9)"),
    )?;
    let mut other_values = values(0);
    other_values[4].0 = key(9);
    let incompatible = HeapBindings::for_program(&mut gpu, &other_program, &other_values)?;
    drop(authored);
    drop(other_program);
    ensure!(
        authored_kernel
            .record(&mut graph, &mut rec, &incompatible, [4, 1, 1])
            .is_err(),
        "incompatible packet accepted"
    );
    ensure!(
        kernel
            .record(&mut graph, &mut rec, &incompatible, [4, 1, 1])
            .is_err(),
        "incompatible default packet accepted"
    );
    incompatible.retire(&mut gpu, &mut graph)?;
    let mut reads = Vec::new();
    let mut last = None;
    for (frame, packet) in packets.iter().enumerate() {
        packet.import(&mut graph);
        let active = if frame == 0 {
            &kernel
        } else {
            &authored_kernel
        };
        active.record(&mut graph, &mut rec, packet, [4, 1, 1])?;
        reads.push(readback_buffer(
            &mut gpu,
            &mut graph,
            &mut rec,
            output.buffer,
            0,
            output.len,
        )?);
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    gpu.wait(last.expect("two handle-array frames"))?;
    for (frame, read) in reads.into_iter().enumerate() {
        let data = gpu.read(read)?;
        ensure!(
            data[256..].iter().all(|b| *b == 0xa5),
            "handle-array tail overwritten"
        );
        let actual: Vec<f32> = data[..256]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| f32::from_le_bytes(*w))
            .collect();
        let mut expected = Vec::new();
        for i in 0..4 {
            let uv = [i as f32 * 0.5 - 0.125, 0.375];
            expected.extend(sample(
                &pixels[image_maps[frame][i]],
                uv,
                sampler_sets[frame][(i + 1) % 4],
            ));
            expected.extend(sample(
                &pixels[image_maps[frame][2]],
                [0.5, 0.5],
                sampler_sets[frame][0],
            ));
            let z = depth_values[depth_maps[frame][i]];
            let pass = match comparison_sets[frame][i]
                .compare
                .expect("comparison fixture")
            {
                CompareOp::Less => 0.5 < z,
                CompareOp::Greater => 0.5 > z,
                CompareOp::LessEqual => 0.5 <= z,
                CompareOp::Never => false,
                _ => unreachable!("fixture comparator"),
            };
            expected.extend([u8::from(pass) as f32; 4]);
            let q = image_maps[frame][i];
            expected.extend([
                query_sizes[q][0] as f32,
                query_sizes[q][1] as f32,
                query_levels[q] as f32,
                0.,
            ]);
        }
        let error = actual
            .iter()
            .zip(expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        ensure!(
            actual.iter().all(|v| v.is_finite()) && error < 1e-6,
            "handle arrays frame{frame} error{error}: {actual:?}"
        );
        println!(
            "handle arrays{frame}: exact scattered/duplicate textures, regular/comparison samplers and dynamic/constant indices"
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    for packet in packets {
        packet.retire(&mut gpu, &mut graph)?;
    }
    kernel.retire(&mut gpu)?;
    authored_kernel.retire(&mut gpu)?;
    output.retire(&mut gpu)?;
    for t in textures.into_iter().chain(depths).chain(query_images) {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in staging {
        gpu.retire(Resource::Buffer(b))?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
