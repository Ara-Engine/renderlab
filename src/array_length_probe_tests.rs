//! `arrayLength` of one element of a storage-buffer binding array, where the invocations of
//! a workgroup index different elements, and the engine's replacement for it.
//!
//! Naga 30 decorates the loads through `heap[i]` `NonUniform` but emits `OpArrayLength`
//! through an undecorated element pointer, so a Vulkan driver may take the index as the same
//! for the whole workgroup. RADV (Radeon 760M, Mesa 26.0.8) returned one element's length for
//! every invocation; the RTX 4070 SUPER answers per invocation. The renderer once told an
//! index buffer with adjacency from one without, and a skinned draw from a static one, by
//! such lengths, per pixel. It now reads `primary_batch_info` (primary_ray::batch_word).
use std::{
    num::NonZeroU32,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use crate::primary_ray::{
    BATCH_ADJACENT, BATCH_SKINNED, batch_info_source, batch_word, placement_words,
};

const LENGTH_SHADER: &str = "enable wgpu_binding_array;
struct Words { data: array<u32> };
@group(0) @binding(0) var<storage,read> reads: binding_array<Words,4>;
@group(0) @binding(1) var<storage,read_write> output: array<u32>;
@compute @workgroup_size(4)
fn probe(@builtin(local_invocation_index) i: u32) {
    output[i * 2u] = reads[i].data[0];
    output[i * 2u + 1u] = arrayLength(&reads[i].data);
}
";

/// Runs `shader`'s `probe` entry once, one workgroup of four invocations, and returns the
/// eight words it wrote. Binding 0 is `reads` as a four-element binding array, or the single
/// buffer `reads[0]`; binding 1 is the output.
fn run_probe(shader: &str, reads: &[Vec<u32>]) -> Vec<u32> {
    let array = reads.len() > 1;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("[array-length] adapter={:?}", adapter.get_info());
    // The renderer's own binding-array features (`VISIBILITY_FEATURES`' buffer part).
    let features = if array {
        wgpu::Features::BUFFER_BINDING_ARRAY
            | wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
    } else {
        wgpu::Features::empty()
    };
    assert!(
        adapter.features().contains(features),
        "the adapter has no storage-buffer binding arrays: nothing was measured"
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: wgpu::Limits {
            max_binding_array_elements_per_shader_stage: if array { 4 } else { 0 },
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let storage = |words: &[u32], usage| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("array-length probe"),
            size: words.len() as u64 * 4,
            usage,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        queue.write_buffer(&buffer, 0, &bytes);
        buffer
    };
    let reads: Vec<_> = reads
        .iter()
        .map(|words| {
            storage(
                words,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            )
        })
        .collect();
    let output = storage(
        &[u32::MAX; 8],
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    );
    let entries: Vec<_> = [
        (true, array.then(|| NonZeroU32::new(4).unwrap())),
        (false, None),
    ]
    .into_iter()
    .enumerate()
    .map(|(binding, (read_only, count))| wgpu::BindGroupLayoutEntry {
        binding: binding as u32,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count,
    })
    .collect();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("array-length probe"),
        entries: &entries,
    });
    let elements: Vec<_> = reads.iter().map(|b| b.as_entire_buffer_binding()).collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("array-length probe"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: if array {
                    wgpu::BindingResource::BufferArray(&elements)
                } else {
                    reads[0].as_entire_binding()
                },
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("array-length probe"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("array-length probe"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("array-length probe"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("probe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("array-length readback"),
        size: 32,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 32);
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
        callback.set(result).unwrap();
    });
    queue.submit([encoder.finish()]);
    let start = Instant::now();
    while done.get().is_none() {
        device.poll(wgpu::PollType::Poll).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "array-length readback timeout"
        );
        std::thread::yield_now();
    }
    done.get().unwrap().as_ref().unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    mapped
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_ne_bytes(*b))
        .collect()
}

/// The driver question itself, kept as the reproduction: four buffers of 3, 13, 6 and 22
/// words (two index buffers without adjacency, two with), one per invocation of a single
/// workgroup. Each invocation writes its element's first word, which names the buffer, and
/// its element's length. Fails on RADV until naga decorates the length; nothing in the
/// renderer depends on it any more.
#[test]
#[ignore = "GPU; cargo test -p renderlab --lib array_length_of_a_binding_array_element -- --ignored --nocapture"]
fn array_length_of_a_binding_array_element_is_that_elements() {
    const LENGTHS: [u32; 4] = [3, 13, 6, 22];
    let reads: Vec<Vec<u32>> = LENGTHS
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let mut words = vec![0u32; n as usize];
            words[0] = 100 + i as u32;
            words
        })
        .collect();
    let pairs = run_probe(LENGTH_SHADER, &reads);
    let (names, lengths): (Vec<u32>, Vec<u32>) = pairs
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (p[0], p[1]))
        .unzip();
    eprintln!("[array-length] first words {names:?}, lengths {lengths:?}, bound {LENGTHS:?}");
    // The loads are decorated: each invocation reads its own element.
    assert_eq!(
        names,
        [100, 101, 102, 103],
        "an invocation read another element's data"
    );
    assert_eq!(
        lengths, LENGTHS,
        "an invocation got another element's length on this device"
    );
}

/// The renderer's replacement, through its own shader text and packer: one workgroup whose
/// four invocations read four draws that differ in every fact (static and skinned, with and
/// without adjacency, different triangle counts). Each must get its own draw's word.
#[test]
#[ignore = "GPU; cargo test -p renderlab --lib batch_info_is_per_invocation -- --ignored --nocapture"]
fn batch_info_is_per_invocation_for_mixed_skinned_and_static_draws() {
    // Index words per draw: 4 triangles plain, 2 with adjacency, 1 plain, 3 with adjacency.
    let batches = [
        batch_word(12, false),
        batch_word(13, false),
        batch_word(3, true),
        batch_word(19, true),
    ];
    assert_eq!(
        batches,
        [
            4,
            2 | BATCH_ADJACENT,
            1 | BATCH_SKINNED,
            3 | BATCH_ADJACENT | BATCH_SKINNED
        ]
    );
    // Seven placements, so the batch words do not start at a round offset.
    let table = placement_words(&[0, 0, 1, 2, 2, 2, 3], &batches);
    let shader = format!(
        "@group(0) @binding(0) var<storage,read> primary_placements:array<u32>;
@group(0) @binding(1) var<storage,read_write> output:array<u32>;
{}
@compute @workgroup_size(4)
fn probe(@builtin(local_invocation_index) i:u32) {{
    let info=primary_batch_info(i);
    output[i*2u]=info;
    output[i*2u+1u]=select(0u,batch_triangles(info)*3u,batch_adjacent(info))+select(0u,100u,batch_skinned(info));
}}
",
        batch_info_source(include_str!("../shaders/primary_rays.wgsl"))
    );
    let pairs = run_probe(&shader, &[table]);
    eprintln!("[array-length] batch words, then adjacency offset plus 100 if skinned: {pairs:?}");
    assert_eq!(
        pairs,
        [
            batches[0], 0, batches[1], 6, batches[2], 100, batches[3], 109
        ],
        "an invocation read another draw's batch word"
    );
}
