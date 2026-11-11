//! U-BC (docs/texture-compression.md): the GPU's block decode against the CPU decoder,
//! the fixture through every renderer path and the engine's two, streamed residency of
//! compressed chains, and texture memory. Nested under texture_residency_tests for its
//! renderers and scene.
use super::*;
use crate::asset::{TextureFormat, TextureRole};
use crate::texture_bc::{self, Storage, fixture};

const BC: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_BC;

fn out_dir() -> PathBuf {
    let out = PathBuf::from(
        std::env::var("RENDERLAB_BC_OUT")
            .unwrap_or_else(|_| "D:/Coding/renderlab-work/unit-bc/out".into()),
    );
    fs::create_dir_all(&out).unwrap();
    out
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

/// How two RGBA8 images differ: the largest channel difference, and the channels that
/// differ at all and by more than one code.
fn codes(a: &[u8], b: &[u8]) -> serde_json::Value {
    assert_eq!(a.len(), b.len());
    let differences = || a.iter().zip(b).map(|(a, b)| a.abs_diff(*b));
    serde_json::json!({
        "worst": differences().max().unwrap_or(0),
        "differing": differences().filter(|&d| d != 0).count(),
        "over_one": differences().filter(|&d| d > 1).count(),
        "channels": a.len(),
    })
}

const LOAD_SHADER: &str = r#"
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var decoded:texture_storage_2d<rgba8unorm,write>;
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(source);
    if id.x<size.x && id.y<size.y { textureStore(decoded,id.xy,textureLoad(source,id.xy,0)); }
}"#;

/// Level 0 of `data` as the GPU decodes it: every texel loaded and stored as RGBA8.
fn gpu_decode(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    data: &asset::Texture,
) -> Vec<u8> {
    let mip = &data.mips[0];
    let size = wgpu::Extent3d {
        width: mip.width,
        height: mip.height,
        depth_or_array_layers: 1,
    };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("U-BC decode"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let source = texture(
        texture_bc::gpu_format(data, Storage::Source),
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    texture_bc::write_level(queue, &source, 0, data, mip, Storage::Source);
    let decoded = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("U-BC decode"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &source.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &decoded.create_view(&Default::default()),
                ),
            },
        ],
    });
    let row = mip.width * 4;
    assert!(row.is_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT));
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("U-BC decode readback"),
        size: u64::from(row * mip.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = device.create_command_encoder(&Default::default());
    {
        let mut pass = e.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(mip.width.div_ceil(8), mip.height.div_ceil(8), 1);
    }
    e.copy_texture_to_buffer(
        decoded.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(mip.height),
            },
        },
        size,
    );
    e.map_buffer_on_submit(&read, wgpu::MapMode::Read, .., |result| result.unwrap());
    queue.submit([e.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(60)),
        })
        .unwrap();
    let bytes = read.get_mapped_range(..).unwrap().to_vec();
    read.unmap();
    bytes
}

/// The CPU decoder against the GPU's own decode: BC7 blocks in every mode and seeded BC5
/// blocks, and the two encoded fixture images. BC7's interpolation is exact integer
/// arithmetic, so it must agree bit for bit; BC5's runs at more than eight bits on the
/// GPU and is rounded to eight here and by the decoder, so it may differ by one code.
#[test]
#[ignore = "U-BC owned GPU lease: the GPU's BC7 and BC5 decode against the CPU decoder"]
fn gpu_bc_decode_matches_cpu() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging().with_env(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("U-BC decode adapter {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: BC,
        ..Default::default()
    }))
    .unwrap();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("U-BC decode"),
        source: wgpu::ShaderSource::Wgsl(LOAD_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("U-BC decode"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    // 256 x 256: 4,096 blocks, 512 of each BC7 mode.
    let side = 256;
    let top = |role, format, texels| fixture::chain(role, format, side, texels).mips.remove(0);
    let linear = TextureRole::MetallicRoughnessLinear;
    let normal = TextureRole::NormalLinear;
    let cases = [
        (
            "bc7-every-mode",
            linear,
            TextureFormat::Bc7,
            fixture::bc7_all_modes(11, side, side),
        ),
        (
            "bc5-seeded",
            normal,
            TextureFormat::Bc5,
            fixture::bc5_seeded(12, side, side),
        ),
        (
            "bc7-fixture",
            linear,
            TextureFormat::Bc7,
            top(linear, TextureFormat::Bc7, fixture::colour(side)),
        ),
        (
            "bc5-fixture",
            normal,
            TextureFormat::Bc5,
            top(normal, TextureFormat::Bc5, fixture::normals(side)),
        ),
    ];
    let mut report = vec![];
    for (name, role, format, mip) in cases {
        let data = asset::Texture {
            role,
            format,
            mips: vec![mip],
        };
        let gpu = gpu_decode(&device, &queue, &pipeline, &data);
        let cpu = texture_bc::decode_texture(&data).mips.remove(0).rgba8;
        // BC5 holds two channels: the GPU returns (X, Y, 0, 1), the decoder rebuilds Z.
        let kept = if format == TextureFormat::Bc5 { 2 } else { 4 };
        let pick = |bytes: &[u8]| -> Vec<u8> {
            bytes
                .chunks_exact(4)
                .flat_map(|t| t[..kept].to_vec())
                .collect()
        };
        let difference = codes(&pick(&gpu), &pick(&cpu));
        eprintln!("U-BC decode {name}: {difference}");
        let allowed = u64::from(format == TextureFormat::Bc5);
        assert!(
            difference["worst"].as_u64().unwrap() <= allowed,
            "{name}: {difference}"
        );
        report.push(serde_json::json!({"case": name, "gpu_vs_cpu": difference}));
    }
    fs::write(
        out_dir().join("decode.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

/// The fixture's three maps as blocks: colour (BC7, sRGB), metallic-roughness (BC7,
/// roughness in G and metallic in B) and normals (BC5), each a full chain.
fn fixture_maps(size: u32) -> [asset::Texture; 3] {
    [
        fixture::chain(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            size,
            fixture::colour(size),
        ),
        fixture::chain(
            TextureRole::MetallicRoughnessLinear,
            TextureFormat::Bc7,
            size,
            fixture::colour(size),
        ),
        fixture::chain(
            TextureRole::NormalLinear,
            TextureFormat::Bc5,
            size,
            fixture::normals(size),
        ),
    ]
}

fn upload(r: &mut Renderer, data: &asset::Texture) -> Texture {
    let texture = loop {
        if let Some(t) = r.try_upload_texture(data).unwrap() {
            break t;
        }
        r.collect().unwrap();
    };
    r.test_drain("U-BC texture");
    texture
}

/// A quad with uv (0,0), (1,0), (1,1), (0,1) at its corners and the tangent frame of
/// that mapping, which normal-mapped materials need.
fn tangent_quad(r: &mut Renderer, corners: [[f32; 3]; 4]) -> (Mesh, Instances) {
    let [a, b, c, d] = corners.map(Vec3::from_array);
    let normal = (c - a).cross(b - a).normalize();
    let tangent = (b - a).normalize();
    let handedness = normal.cross(tangent).dot(d - a).signum();
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let vertices: Vec<_> = corners
        .iter()
        .zip(uv)
        .map(|(&position, uv)| Vertex {
            position,
            normal: normal.to_array(),
            uv,
            color: [1.; 4],
        })
        .collect();
    let tangents = asset::TangentStream {
        basis: asset::TangentBasis::Authored,
        values: vec![[tangent.x, tangent.y, tangent.z, handedness]; 4],
    };
    let data = MeshData {
        vertices: &vertices,
        indices: &[0, 2, 1, 0, 3, 2],
    };
    let mesh = r
        .try_upload_mesh_with_tangents(&data, Some(&tangents))
        .unwrap()
        .unwrap();
    r.test_drain("U-BC quad");
    let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
    r.test_drain("U-BC quad placement");
    (mesh, instances)
}

/// The presented frame of a ground and a wall under one material of `maps` (None binds
/// the renderer's defaults), with trilinear 16x anisotropic sampling.
fn fixture_frame(
    r: &mut Renderer,
    quads: &mut [(Mesh, Instances); 2],
    maps: [Option<&asset::Texture>; 3],
) -> Vec<u8> {
    let textures = maps.map(|m| m.map(|data| upload(r, data)));
    let binding = |texture: &Option<Texture>| {
        texture.as_ref().map(|texture| TextureBinding {
            texture,
            sampler: asset::Sampler::default(),
        })
    };
    let material = loop {
        let descriptor = MaterialDescriptor {
            base_color_texture: binding(&textures[0]),
            metallic_roughness_texture: binding(&textures[1]),
            normal_texture: binding(&textures[2]),
            double_sided: true,
            ..Default::default()
        };
        if let Some(m) = r.try_create_material(&descriptor).unwrap() {
            break m;
        }
        r.collect().unwrap();
    };
    r.test_drain("U-BC material");
    let color = target(r, 640, 360);
    let camera = Camera::look_at(
        Vec3::new(0., 2.5, 5.),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.01,
        12.,
    )
    .unwrap();
    let [(ground, ground_at), (wall, wall_at)] = quads;
    for _ in 0..4 {
        render(
            r,
            &color,
            &camera,
            &mut [
                Draw::new(ground, ground_at).with_material(&material),
                Draw::new(wall, wall_at).with_material(&material),
            ],
        );
    }
    r.test_drain("U-BC fixture frame");
    r.test_capture_color(&color)
}

/// A renderer for `path` with single frames (no temporal history), lit, and the
/// fixture's ground and wall.
fn fixture_scene(path: &str, extra: wgpu::Features) -> (Renderer, [(Mesh, Instances); 2]) {
    let mut r = residency_renderer(path, extra);
    r.set_reconstruction(crate::Reconstruction::off()).unwrap();
    light(&mut r);
    let ground = tangent_quad(
        &mut r,
        [[-2., 0., -2.], [2., 0., -2.], [2., 0., 2.], [-2., 0., 2.]],
    );
    let wall = tangent_quad(
        &mut r,
        [
            [-1., 0., -1.5],
            [1., 0., -1.5],
            [1., 1.5, -1.5],
            [-1., 1.5, -1.5],
        ],
    );
    (r, [ground, wall])
}

/// U-BC acceptance: on every renderer path the fixture in BC7 and BC5 renders as the
/// same blocks decoded to RGBA8 and uploaded uncompressed. BC7 decodes exactly, so those
/// frames are identical. A BC5 normal keeps more than eight bits and rebuilds Z after
/// filtering where the RGBA8 map filters a rounded Z, so its frames may differ by a code.
/// A device without BC sampling decodes at upload and shades the BC5 map the same way.
#[test]
#[ignore = "U-BC owned GPU lease: BC fixture against its RGBA8 decode, three paths"]
fn gpu_bc_fixture_matches_decoded_rgba8() {
    let out = out_dir();
    let maps = fixture_maps(256);
    let decoded = maps.each_ref().map(texture_bc::decode_texture);
    let [colour, mr, normal] = maps.each_ref().map(Some);
    let [plain_colour, plain_mr, plain_normal] = decoded.each_ref().map(Some);
    let mut report = vec![];
    let mut failures = vec![];
    for path in ["nort", "rt", "traced-virtual"] {
        let (mut r, mut quads) = fixture_scene(path, BC);
        assert!(r.device().features().contains(BC));
        let mut frame = |maps| fixture_frame(&mut r, &mut quads, maps);
        // Colour and metallic-roughness alone, then the normal map alone, then all three.
        let bc7 = frame([colour, mr, None]);
        let bc7_plain = frame([plain_colour, plain_mr, None]);
        let bc5 = frame([plain_colour, plain_mr, normal]);
        let bc5_plain = frame([plain_colour, plain_mr, plain_normal]);
        let all = frame([colour, mr, normal]);
        let all_plain = frame([plain_colour, plain_mr, plain_normal]);
        let unmapped = frame([plain_colour, plain_mr, None]);
        drop(quads);
        drop(r);
        // The same blocks on a device that cannot sample them.
        let (mut r, mut quads) = fixture_scene(path, wgpu::Features::empty());
        assert!(!r.device().features().contains(BC));
        let fallback = fixture_frame(&mut r, &mut quads, [colour, mr, normal]);
        write_png(&out.join(format!("fixture-{path}-bc.png")), 640, 360, &all);
        write_png(
            &out.join(format!("fixture-{path}-rgba8.png")),
            640,
            360,
            &all_plain,
        );
        write_png(
            &out.join(format!("fixture-{path}-fallback.png")),
            640,
            360,
            &fallback,
        );
        let row = serde_json::json!({
            "path": path,
            "bc7_vs_rgba8": codes(&bc7, &bc7_plain),
            "bc5_vs_rgba8": codes(&bc5, &bc5_plain),
            "all_vs_rgba8": codes(&all, &all_plain),
            "fallback_vs_blocks": codes(&fallback, &all),
            // How much the normal map changes the frame at all: the test is not vacuous.
            "normal_map_effect": codes(&bc5, &unmapped),
        });
        eprintln!("U-BC fixture {row}");
        let worst = |key: &str| row[key]["worst"].as_u64().unwrap();
        if worst("bc7_vs_rgba8") != 0 {
            failures.push(format!("{path}: BC7 frames differ from their decode"));
        }
        for key in ["bc5_vs_rgba8", "all_vs_rgba8", "fallback_vs_blocks"] {
            if worst(key) > 1 {
                failures.push(format!("{path}: {key} worst {}", worst(key)));
            }
        }
        if worst("normal_map_effect") < 8 {
            failures.push(format!("{path}: the normal map barely shows"));
        }
        report.push(row);
    }
    fs::write(
        out.join("fixture.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(failures.is_empty(), "{failures:?}");
}

/// Streamed residency with compressed chains: the M3 camera cut (levels dropped to the
/// tail, then a cut that needs level 0) with BC7 blocks in every mode, as blocks on the
/// GPU. Verified frames must equal full residency's, as for RGBA8 chains: the tail and
/// margin rules hold for sampled blocks.
#[test]
#[ignore = "U-BC owned GPU lease: camera cut onto non-resident BC7 levels, two paths"]
fn gpu_bc_residency_camera_cut() {
    let mut report = vec![];
    let mut failures = vec![];
    // 1024 and 512 chains of whole blocks, one block for each level under 4 texels.
    let chain_bytes = |size: u32| -> u64 {
        (0..=size.trailing_zeros())
            .map(|level| u64::from((size >> level).div_ceil(4).pow(2)) * 16)
            .sum()
    };
    for path in ["nort", "rt"] {
        let (full, full_stats) = cut_run(path, TextureResidency::Full, TextureFormat::Bc7);
        let (streamed, stats) = cut_run(path, TextureResidency::Exact, TextureFormat::Bc7);
        let tops =
            |frames: &[CutFrame], label: &str| frames.iter().find(|f| f.0 == label).unwrap().3;
        // Blocks are resident as blocks: a quarter of the RGBA8 chains' bytes.
        assert_eq!(
            stats.full_bytes,
            chain_bytes(1024) + chain_bytes(512),
            "{path}"
        );
        assert_eq!(tops(&streamed, "away"), [6, 6], "{path}");
        assert_eq!(tops(&streamed, "close0"), [6, 6], "{path}");
        let after = tops(&streamed, "after");
        assert!(after[0] == 0 && after[1] < 6, "{path}: {after:?}");
        assert_eq!(tops(&full, "close0"), [0, 0], "{path}");
        assert!(stats.reshaded_frames >= 2, "{path}: {stats:?}");
        assert_eq!(stats.misses, 0, "{path}: {stats:?}");
        assert_eq!(full_stats.verified_frames, 0, "{path}");
        let frames: Vec<_> = streamed
            .iter()
            .zip(&full)
            .map(|(a, b)| {
                assert_eq!(a.0, b.0);
                if a.1 != b.1 || a.2 != b.2 {
                    failures.push(format!("{path} {}", a.0));
                }
                serde_json::json!({
                    "frame": a.0, "hdr_identical": a.1 == b.1, "presented_identical": a.2 == b.2,
                    "streamed_tops": a.3, "full_tops": b.3,
                })
            })
            .collect();
        report.push(serde_json::json!({
            "path": path, "frames": frames,
            "resident_bytes": stats.resident_bytes, "full_bytes": stats.full_bytes,
            "transitions": stats.transitions, "uploaded_bytes": stats.uploaded_bytes,
            "reshaded_frames": stats.reshaded_frames, "serviced_misses": stats.serviced_misses,
        }));
    }
    fs::write(
        out_dir().join("residency-cut.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(failures.is_empty(), "streamed frames differ: {failures:?}");
}

/// GPU texture memory of three 4096 x 4096 material maps with full chains (colour and
/// metallic-roughness in BC7, normals in BC5): as blocks, and decoded to RGBA8 as they
/// reached the renderer before U-BC. From the allocator report and the renderer's ledger.
#[test]
#[ignore = "U-BC owned GPU lease: texture memory of 4096 maps, blocks against RGBA8"]
fn gpu_bc_memory_4096() {
    let maps = [
        fixture::seeded_chain(TextureRole::BaseColorSrgb, TextureFormat::Bc7, 4096, 21),
        fixture::seeded_chain(
            TextureRole::MetallicRoughnessLinear,
            TextureFormat::Bc7,
            4096,
            41,
        ),
        fixture::seeded_chain(TextureRole::NormalLinear, TextureFormat::Bc5, 4096, 61),
    ];
    let measure = |label: &str, maps: &[asset::Texture]| {
        let mut r = residency_renderer("nort", BC);
        let allocated = |r: &Renderer| {
            r.device()
                .generate_allocator_report()
                .map_or(0, |report| report.total_allocated_bytes)
        };
        let before = (allocated(&r), r.stats().ownership.texture_bytes.live);
        let textures: Vec<_> = maps.iter().map(|data| upload(&mut r, data)).collect();
        let after = (allocated(&r), r.stats().ownership.texture_bytes.live);
        let named: u64 = r
            .device()
            .generate_allocator_report()
            .iter()
            .flat_map(|report| &report.allocations)
            .filter(|a| a.name == "immutable material texture")
            .map(|a| a.size)
            .sum();
        drop(textures);
        let row = serde_json::json!({
            "label": label,
            "source_bytes": maps.iter().flat_map(|t| &t.mips).map(|m| m.rgba8.len() as u64).sum::<u64>(),
            "allocator_allocated_delta": after.0 - before.0,
            "allocator_material_texture_bytes": named,
            "ledger_texture_delta": after.1 - before.1,
        });
        eprintln!("U-BC memory {row}");
        row
    };
    let blocks = measure("blocks", &maps);
    let rgba8 = measure("rgba8", &maps.each_ref().map(texture_bc::decode_texture));
    let ratio = |key: &str| rgba8[key].as_f64().unwrap() / blocks[key].as_f64().unwrap();
    let ratios = serde_json::json!({
        "allocator_allocated_delta": ratio("allocator_allocated_delta"),
        "allocator_material_texture_bytes": ratio("allocator_material_texture_bytes"),
        "ledger_texture_delta": ratio("ledger_texture_delta"),
    });
    eprintln!("U-BC memory rgba8 / blocks {ratios}");
    fs::write(
        out_dir().join("memory-4096.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"blocks": blocks, "rgba8": rgba8, "rgba8_over_blocks": ratios}),
        )
        .unwrap(),
    )
    .unwrap();
    // 8 bits a texel against 32, chains included: the renderer holds the blocks as given.
    assert_eq!(blocks["ledger_texture_delta"], blocks["source_bytes"]);
    let held = ratio("allocator_material_texture_bytes");
    assert!((3.9..4.1).contains(&held), "{ratios}");
}

/// One engine's settled frame of the fixture: a ground and a wall under `maps`, sunlit.
fn engine_frame(path: crate::engine::RenderPath, maps: &[asset::Texture; 3]) -> (Vec<u8>, bool) {
    use crate::engine::*;
    let adapter = headless_adapter().unwrap();
    // SAFETY: the lease supplies the renderer's explicit experimental-feature
    // acknowledgement, checked by the caller.
    let experimental_ray_query =
        (path == RenderPath::HardwareRt).then(|| unsafe { ExperimentalRayQuery::acknowledge() });
    let mut engine = Engine::new(
        &adapter,
        EngineDescriptor {
            path,
            experimental_ray_query,
            pipeline_cache: Some(out_dir().join("pipeline-cache")),
            texture_anisotropy: 16,
            settings: Settings {
                exposure: Exposure::Manual { stops: -14.5 },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    engine.resize(640, 360).unwrap();
    let sampled = engine.device().features().contains(BC);
    let [colour, mr, normal] = maps.each_ref().map(|data| TextureRef {
        texture: engine.create_texture(data.clone()).unwrap(),
        sampler: Sampler::default(),
    });
    let material = engine
        .create_material(&MaterialDesc {
            base_color_texture: Some(colour),
            metallic_roughness_texture: Some(mr),
            normal_texture: Some(normal),
            metallic: 1.0,
            roughness: 1.0,
            ..Default::default()
        })
        .unwrap();
    for corners in [
        [[-2., 0., -2.], [2., 0., -2.], [2., 0., 2.], [-2., 0., 2.]],
        [
            [-1., 0., -1.5],
            [1., 0., -1.5],
            [1., 1.5, -1.5],
            [-1., 1.5, -1.5],
        ],
    ] {
        let [a, b, c, d] = corners.map(Vec3::from_array);
        let normal = (c - a).cross(b - a).normalize();
        let tangent = (b - a).normalize();
        let handedness = normal.cross(tangent).dot(d - a).signum();
        let vertices: Vec<_> = corners
            .iter()
            .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
            .map(|(&position, uv)| Vertex {
                position,
                normal: normal.to_array(),
                uv,
                color: [1.; 4],
            })
            .collect();
        let mesh = engine
            .create_mesh_with_tangents(
                &MeshData {
                    vertices: &vertices,
                    indices: &[0, 2, 1, 0, 3, 2],
                },
                &TangentStream {
                    basis: asset::TangentBasis::Authored,
                    values: vec![[tangent.x, tangent.y, tangent.z, handedness]; 4],
                },
            )
            .unwrap();
        engine.spawn(mesh, material, Mat4::IDENTITY).unwrap();
    }
    engine
        .set_sun(Some(Sun {
            travel: Vec3::new(-0.4, -1.0, -0.3).normalize(),
            ..Sun::default()
        }))
        .unwrap();
    let camera = Camera::look_at(
        Vec3::new(0., 2.5, 5.),
        Vec3::ZERO,
        Vec3::Y,
        45f32.to_radians(),
        0.05,
        200.,
    )
    .unwrap();
    // Temporal reconstruction and the GI cache settle over the first frames.
    for _ in 0..48 {
        engine.render(&camera).unwrap();
    }
    (engine.capture().unwrap().rgba8, sampled)
}

/// The fixture through the engine on both paths, everything at its defaults: blocks
/// against the same blocks decoded to RGBA8, as a caller sent them before U-BC. The
/// engine requests BC sampling where the adapter has it.
#[test]
#[ignore = "U-BC owned GPU lease: the engine's two paths, BC fixture against its RGBA8 decode"]
fn gpu_bc_engine_fixture() {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    );
    let out = out_dir();
    let maps = fixture_maps(256);
    let decoded = maps.each_ref().map(texture_bc::decode_texture);
    let mut report = vec![];
    for (name, path) in [
        ("nonrt", crate::engine::RenderPath::NonRt),
        ("rt", crate::engine::RenderPath::HardwareRt),
    ] {
        // One engine alive at a time: each holds a device and its compiled pipelines.
        let (blocks, sampled) = engine_frame(path, &maps);
        assert!(
            sampled,
            "{name}: the adapter has BC sampling and the engine requests it"
        );
        let (plain, _) = engine_frame(path, &decoded);
        let (again, _) = engine_frame(path, &maps);
        write_png(
            &out.join(format!("engine-{name}-bc.png")),
            640,
            360,
            &blocks,
        );
        write_png(
            &out.join(format!("engine-{name}-rgba8.png")),
            640,
            360,
            &plain,
        );
        let row = serde_json::json!({
            "path": name,
            "blocks_vs_rgba8": codes(&blocks, &plain),
            // Two engines given the same blocks: the frame-to-frame floor of this scene.
            "blocks_vs_blocks_again": codes(&blocks, &again),
        });
        eprintln!("U-BC engine {row}");
        report.push(row);
    }
    fs::write(
        out.join("engine.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for row in &report {
        assert!(
            row["blocks_vs_rgba8"]["worst"].as_u64().unwrap() <= 1,
            "{row}"
        );
    }
}
