//! Dense shared-edge samples, independent of raster coverage or a film average.
use super::*;

fn source() -> String {
    let helpers = SOURCE.split_once("struct Receiver {").unwrap().1;
    let helpers = helpers
        .split_once("// renderlab: affine vertices begin")
        .unwrap()
        .0;
    let fold = include_str!("../shaders/primary_trace.wgsl")
        .split_once("// renderlab: fold inset begin")
        .unwrap()
        .1
        .split_once("// renderlab: fold inset end")
        .unwrap()
        .0;
    format!(
        "enable wgpu_per_vertex;\nstruct TestLighting {{eye:vec3<f32>}};\nvar<private> lighting:TestLighting;\nstruct Receiver {{{helpers}\n{fold}\n{}",
        include_str!("../support/receiver_fold.wgsl")
    )
}

#[test]
fn receiver_fold_source_compiles() {
    validate_wgsl(&source());
    let kernels = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    validate_wgsl(&primary_ray_gpu::edge_check_source(&kernels[1]));
    if let Some(path) = std::env::var_os("RENDERLAB_RECEIVER_CHECK_EXPORT") {
        fs::write(path, primary_ray_gpu::edge_check_source(&kernels[1])).unwrap();
    }
}

#[test]
fn receiver_primary_spirv() {
    let Some(directory) = std::env::var_os("RENDERLAB_RECEIVER_SPIRV_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    fs::create_dir_all(&directory).unwrap();
    for (index, source) in primary_ray_gpu::shader_sources(SOURCE)
        .unwrap()
        .iter()
        .enumerate()
    {
        let (module, info) = validate_wgsl(source);
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            None,
            &Default::default(),
        )
        .unwrap();
        for debug in [false, true] {
            let mut flags = naga::back::spv::WriterFlags::LABEL_VARYINGS
                | naga::back::spv::WriterFlags::FORCE_POINT_SIZE;
            flags.set(naga::back::spv::WriterFlags::DEBUG, debug);
            let words = naga::back::spv::write_vec(
                &module,
                &info,
                &naga::back::spv::Options {
                    lang_version: (1, 3),
                    flags,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
            fs::write(
                directory.join(format!("primary-{index}-debug-{debug}.spv")),
                words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
    }
}

#[test]
#[ignore = "offscreen primary traversal and neighbour acquisition on millimetre folds"]
pub(super) fn gpu_receiver_fold_traced() {
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut meshes = vec![];
    let mut instances = vec![];
    for case in (0..18u32).step_by(2) {
        r.test_drain("millimetre fold uploads");
        r.collect().unwrap();
        let slope = (2u32 << (case / 6)) as f32;
        let facet = 0.002 * (1u32 << ((case / 2) % 3)) as f32;
        let extent = facet / (1. + slope * slope).sqrt();
        let vertices = [
            [0., -facet, 0.],
            [extent, -facet, slope * extent],
            [0., facet, 0.],
            [-extent, -facet, slope * extent],
        ]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        });
        meshes.push(
            r.try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
            })
            .unwrap()
            .unwrap(),
        );
        let models =
            [case, case + 1].map(|i| Mat4::from_translation(Vec3::new(0.05 * i as f32, 0., 0.)));
        instances.push(r.try_upload_instances(&models).unwrap().unwrap());
    }
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::new(0., 0., -1.),
                lux: [1000.; 3],
            }],
            ..Default::default()
        },
        &Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![200.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let camera = Camera::look_at(
        Vec3::new(0.45, 0., 100.),
        Vec3::new(0.45, 0., 0.),
        Vec3::Y,
        55f32.to_radians(),
        0.01,
        1000.,
    )
    .unwrap();
    let color = target(&r, 8192, 18);
    let mut draws: Vec<_> = meshes
        .iter()
        .zip(instances.iter_mut())
        .map(|(mesh, instances)| Draw::new(mesh, instances))
        .collect();
    render(&mut r, &color, &camera, &mut draws);
    let mut sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let start = sources[0].find("fn primary_ray(").unwrap();
    let end = start
        + sources[0][start..]
            .find("// World-space plane normal")
            .unwrap();
    sources[0].replace_range(start..end, "");
    sources[0] = sources[0].replace("fn primary_trace(", "fn unused_primary_trace(");
    sources[0].push_str(include_str!("../support/receiver_fold_traced.wgsl"));
    validate_wgsl(&sources[0]);
    let pipelines = r
        .primary
        .as_ref()
        .unwrap()
        .compile(&r.device, &r.ledger, &sources);
    let mut first = None;
    let repeats = manifest()["repeats"].as_u64().unwrap_or(8) as usize;
    for repeat in 0..repeats {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        r.encode_primary_trace(
            &mut encoder,
            &pipelines.trace[0],
            primary_ray_gpu::TRACE_TILE,
            false,
            None,
        );
        r.queue.submit([encoder.finish()]);
        let bytes = read_texture(
            &r,
            &r.primary
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .hit
                .texture,
        );
        if let Some(expected) = &first {
            assert_eq!(&bytes, expected, "traced fold repeat {repeat}");
        } else {
            first = Some(bytes);
        }
    }
    let bytes = first.unwrap();
    let mut rows = vec![];
    for (case, data) in bytes.chunks_exact(8192 * 16).enumerate() {
        let mut counts = [0u32; 3];
        let mut failures = vec![];
        for (x, sample) in data.chunks_exact(16).enumerate() {
            for i in 0..3 {
                let value = u32::from_le_bytes(sample[i * 4..i * 4 + 4].try_into().unwrap());
                counts[i] += u32::from(if i == 1 { value == 1 } else { value != 0 });
            }
            let result = u32::from_le_bytes(sample[4..8].try_into().unwrap());
            if result != 0 {
                failures.push(serde_json::json!({"x":x,"result":result,"u":f32::from_le_bytes(sample[12..16].try_into().unwrap())}));
            }
        }
        rows.push(serde_json::json!({"case":case,"old_false_shadows":counts[0],"new_false_shadows":counts[1],"real_occlusion_retained":counts[2],"failures":failures}));
    }
    let out = PathBuf::from(manifest()["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("traced-fold.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"repeats":repeats,"byte_identical":true,"cases":rows}),
        )
        .unwrap(),
    )
    .unwrap();
    for row in rows {
        // u=1 is the far outer vertex, outside the shared-edge strip. Rounded
        // instance translation may move that ray outside the triangle. A miss
        // there is coverage, not a shadow; interior misses remain failures.
        for failure in row["failures"].as_array().unwrap() {
            assert_eq!(failure["result"], 8, "{row}");
            assert_eq!(failure["x"], 8191, "{row}");
        }
        assert!(row["old_false_shadows"].as_u64().unwrap() > 0);
        assert_eq!(row["new_false_shadows"], 0, "{row}");
        assert!(row["real_occlusion_retained"].as_u64().unwrap() > 0);
    }

    // Exercise nonempty correction writes through the real Interactive worklist.
    // Quarry pixel centres can have no changed hits despite dense subpixel bands.
    let mut shadows = r.shadows.settings.clone();
    shadows.directional_disk_angle = 0.001;
    r.set_shadows(&shadows).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    render(&mut r, &color, &camera, &mut draws);
    let mut sources = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let start = sources[1].find("fn primary_ray(").unwrap();
    let end = start
        + sources[1][start..]
            .find("// World-space plane normal")
            .unwrap();
    sources[1].replace_range(start..end, "");
    sources[1] = sources[1].replace(
        "if in_bounds && all(id.xy>=scene.xy) && all(id.xy<scene.zw)",
        "if in_bounds",
    );
    sources[1].push_str(
        include_str!("../support/receiver_fold_traced.wgsl")
            .split_once("fn fold_neighbour(")
            .unwrap()
            .0,
    );
    validate_wgsl(&sources[1]);
    let mut control = sources.clone();
    control[1] = control[1].replace(
        "fn primary_retry_interactive(",
        "fn unused_primary_retry_interactive(",
    );
    control[1].push_str("\n@compute @workgroup_size(32) fn primary_retry_interactive() {}\n");
    r.pipeline.primary = Some(
        r.primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &control),
    );
    let dispatch = |r: &mut Renderer| {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        r.encode_primary_interactive(
            &mut encoder,
            &r.pipeline.primary.as_ref().unwrap().interactive,
            primary_ray_gpu::TRACE_TILE,
            None,
        );
        r.queue.submit([encoder.finish()]);
        [
            read_texture(
                r,
                &r.primary
                    .as_ref()
                    .unwrap()
                    .targets
                    .as_ref()
                    .unwrap()
                    .hit
                    .texture,
            ),
            read_texture(
                r,
                &r.fresh
                    .as_ref()
                    .unwrap()
                    .targets
                    .as_ref()
                    .unwrap()
                    .visibility
                    .texture,
            ),
        ]
    };
    let hot = dispatch(&mut r);
    let blocked = hot[1]
        .chunks_exact(4)
        .filter(|b| u32::from_le_bytes((*b).try_into().unwrap()) < 256)
        .count();
    assert!(
        blocked > 0,
        "the raw hot control must reproduce false shadows"
    );
    r.pipeline.primary = Some(
        r.primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &sources),
    );
    let mut first = None;
    let repeats = manifest()["repeats"].as_u64().unwrap_or(8) as usize;
    for repeat in 0..repeats {
        let actual = dispatch(&mut r);
        assert!(
            actual[1]
                .chunks_exact(4)
                .all(|b| u32::from_le_bytes(b.try_into().unwrap()) == 256),
            "deferred fold false shadow at repeat {repeat}"
        );
        if let Some(expected) = &first {
            assert_eq!(&actual, expected, "deferred fold repeat {repeat}");
        } else {
            first = Some(actual);
        }
    }
    let corrected = first.unwrap();
    let changed = hot[0]
        .chunks_exact(16)
        .zip(corrected[0].chunks_exact(16))
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 0);
    fs::write(out.join("deferred-fold.json"),serde_json::to_vec_pretty(&serde_json::json!({"repeats":repeats,"byte_identical":true,"hot_blocked_pixels":blocked,"changed_hit_records":changed,"corrected_blocked_pixels":0})).unwrap()).unwrap();
}

#[test]
#[ignore = "offscreen analytic fold; owned GPU lease and manifest"]
pub(super) fn gpu_receiver_fold() {
    let _validation = validation::begin(true);
    let r = renderer(true);
    let output = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dense fold samples"),
        size: wgpu::Extent3d {
            width: 8192,
            height: 36,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production fold receiver helpers"),
        source: wgpu::ShaderSource::Wgsl(source().into()),
    });
    let pipeline = r
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fold"),
            layout: None,
            module: &module,
            entry_point: Some("fold"),
            compilation_options: Default::default(),
            cache: None,
        });
    let group = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fold output"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&output.create_view(&Default::default())),
        }],
    });
    let mut first = None;
    let repeats = manifest()["repeats"].as_u64().unwrap_or(8) as usize;
    for repeat in 0..repeats {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(128, 36, 1);
        }
        r.queue.submit([encoder.finish()]);
        let bytes = read_texture(&r, &output);
        if let Some(first) = &first {
            assert_eq!(&bytes, first, "fold repeat {repeat}");
        } else {
            first = Some(bytes);
        }
    }
    let bytes = first.unwrap();
    let mut rows = vec![];
    for (case, data) in bytes.chunks_exact(8192 * 16).enumerate() {
        let mut old = 0;
        let mut new = 0;
        let mut real = 0;
        let mut min = 1f32;
        let mut max = 0f32;
        for words in data.chunks_exact(16) {
            let words: [u32; 4] = std::array::from_fn(|i| {
                u32::from_le_bytes(words[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if words[0] != 0 {
                old += 1;
                let w = f32::from_bits(words[3]);
                min = min.min(w);
                max = max.max(w);
            }
            new += u32::from(words[1] != 0);
            real += u32::from(words[2] != 0);
        }
        rows.push(serde_json::json!({"case":case,"slope":2u32.pow((case%18/6+1) as u32),"facet_mm":2u32.pow((case%6/2+1) as u32),"grazing":case%2!=0,"thin":case>=18,"samples":8192,"old_false_shadows":old,"new_false_shadows":new,"real_occlusion_retained":real,"old_band_barycentric":[min,max]}));
        assert!(
            old > 0,
            "old normal offset must reproduce the band, case {case}"
        );
        assert_eq!(new, 0, "false fold band, case {case}");
        assert!(real > 0, "must retain genuine occlusion, case {case}");
    }
    let out = PathBuf::from(manifest()["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("fold.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"repeats":repeats,"byte_identical":true,"cases":rows}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(out.join("fold.u32"), bytes).unwrap();
}
