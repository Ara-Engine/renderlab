//! One guarded process: serial/parallel upload identity and nine receiver micro-grids.
use super::super::film_accumulation_tests as t1;
use super::*;

struct GpuIdentity {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    counts: wgpu::Buffer,
}

impl GpuIdentity {
    fn new(r: &Renderer, soft: bool) -> Self {
        let primary = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let visibility = if soft {
            &r.fresh
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .visibility
                .texture
        } else {
            &primary.tiles.texture
        };
        let textures: [&wgpu::Texture; 4] = [
            &r.lighting.hdr.as_ref().unwrap().resolved.texture,
            &primary.hit.texture,
            visibility,
            &primary.tiles.texture,
        ];
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let copies = textures.map(|texture| {
            let copy = r.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("adjacency repeat reference"),
                size: texture.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            encoder.copy_texture_to_texture(
                texture.as_image_copy(),
                copy.as_image_copy(),
                texture.size(),
            );
            copy
        });
        r.queue.submit([encoder.finish()]);
        let counts = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("adjacency mismatch counts"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        r.queue
            .write_buffer(&counts, 12, &u32::from(soft).to_le_bytes());
        let [x, y] = primary_ray_gpu::TRACE_TILE;
        let source = include_str!("../support/adjacency_identity.wgsl")
            .replace("TRACE_X", &x.to_string())
            .replace("TRACE_Y", &y.to_string());
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("adjacency exact repeat comparison"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("compare"),
                compilation_options: Default::default(),
                cache: None,
            });
        let views: Vec<_> = textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .chain(copies.iter().map(|t| t.create_view(&Default::default())))
            .collect();
        let mut entries: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, view)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: counts.as_entire_binding(),
        });
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        Self {
            pipeline,
            bind,
            counts,
        }
    }

    fn check(&self, r: &mut Renderer) {
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&self.counts, 0, Some(12));
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.dispatch_workgroups(1920u32.div_ceil(8), 1080u32.div_ceil(8), 1);
        }
        r.queue.submit([encoder.finish()]);
        assert_eq!(read_buffer(r, &self.counts, 12), [0; 12]);
    }
}

fn hashes(r: &mut Renderer, soft: bool) -> Vec<String> {
    let mut bytes = vec![capture(r), hit_record(r)];
    if soft {
        bytes.push(interactive_outputs(r)[1].clone());
    }
    bytes
        .iter()
        .map(|b| format!("{:x}", Sha256::digest(b)))
        .collect()
}

#[test]
fn adjacency_repeat_comparison_source_compiles() {
    let [x, y] = primary_ray_gpu::TRACE_TILE;
    validate_wgsl(
        &include_str!("../support/adjacency_identity.wgsl")
            .replace("TRACE_X", &x.to_string())
            .replace("TRACE_Y", &y.to_string()),
    );
}

#[test]
#[ignore = "owned GPU lease; runtime adjacency identity and startup"]
fn gpu_parallel_adjacency_batch() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let _validation = validation::begin(true);
    let cameras = t1::t1_cameras(&m);
    let repeats = m["repeats"].as_u64().unwrap_or(8) as usize;
    assert_eq!(repeats, 8);
    let mut reference = vec![];
    let mut identity = vec![];
    let mut timings = vec![];
    for parallel in [false, true] {
        let start = Instant::now();
        let mut r = renderer(true);
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        let load_start = Instant::now();
        let mut config = m.clone();
        crate::receiver_adjacency::LEGACY.with(|v| v.set(!parallel));
        let (mut asset, _) = load_quarry(&config, &mut r);
        crate::receiver_adjacency::LEGACY.with(|v| v.set(false));
        let load_ms = load_start.elapsed().as_secs_f64() * 1000.;
        let color = target(&r, 1920, 1080);
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        traced_frame(&mut r, &color, &cameras[0], &mut asset, true);
        r.test_drain("startup to first completed offscreen image");
        timings.push(serde_json::json!({"parallel":parallel,"load_ms":load_ms,
            "startup_to_first_image_ms":start.elapsed().as_secs_f64()*1000.}));
        let soft = r.shadows.settings.clone();
        let mut hard = soft.clone();
        hard.directional_disk_angle = 0.;
        for (pose, camera) in cameras.iter().enumerate() {
            for mode in 0..3 {
                r.set_shadows(if mode == 0 { &hard } else { &soft })
                    .unwrap();
                r.set_soft_shadows(if mode == 2 {
                    crate::SoftShadows::Interactive
                } else {
                    crate::SoftShadows::Exact
                });
                traced_frame(&mut r, &color, camera, &mut asset, true);
                let digest = hashes(&mut r, mode != 0);
                if parallel && mode == 2 {
                    let file =
                        fs::File::create(out.join(format!("pose-{pose}-interactive.png"))).unwrap();
                    let mut encoder = png::Encoder::new(file, 1920, 1080);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&read_texture(&r, &color))
                        .unwrap();
                }
                if parallel {
                    assert_eq!(
                        digest,
                        reference[pose * 3 + mode],
                        "pose {pose} mode {mode}"
                    );
                } else {
                    reference.push(digest.clone());
                }
                let repeat_reference = GpuIdentity::new(&r, mode != 0);
                for _ in 1..repeats {
                    traced_frame(&mut r, &color, camera, &mut asset, true);
                    repeat_reference.check(&mut r);
                }
                identity.push(serde_json::json!({"parallel":parallel,"pose":pose,"mode":(["hard","exact","interactive"][mode]),
                "repeats":repeats,"sha256_hdr_hit_counts":digest,"byte_identical":true}));
            }
        }
        if parallel {
            config["output"] = serde_json::json!(out.join("micro"));
            t1::footprint_with(&config, &mut r, &mut asset, &color, &cameras);
            let proof: serde_json::Value =
                serde_json::from_slice(&fs::read(out.join("micro/footprint.json")).unwrap())
                    .unwrap();
            for rows in proof["poses"].as_object().unwrap().values() {
                for row in rows.as_array().unwrap() {
                    assert_eq!(row["opposite_samples"], 0);
                    assert_eq!(row["corner"], 0);
                }
            }
        }
        r.test_drain("adjacency batch asset release");
    }
    fs::write(
        out.join("identity.json"),
        serde_json::to_vec_pretty(&identity).unwrap(),
    )
    .unwrap();
    fs::write(
        out.join("startup.json"),
        serde_json::to_vec_pretty(&timings).unwrap(),
    )
    .unwrap();
}
