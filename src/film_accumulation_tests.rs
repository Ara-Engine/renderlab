//! T1 GPU tests (docs/m6-temporal-primaries.md), ignored by default and run under an
//! owned GPU lease with RENDERLAB_PRIMARY_MANIFEST:
//!
//! - `gpu_film_t1_reference`: the Level F reference generator. 1,024 jittered primary
//!   samples per pixel in two independent stratified halves, each with 32 independently
//!   salted production shadow rays, accumulating full radiance (R_L), sun-only radiance
//!   (R_D) and centre-surface visibility (R_F). Also the production Exact (seeds 0-7) and
//!   Interactive (seeds 0-3) frames the gate scores against the same reference.
//! - `gpu_film_t1_temporal`: T1a acceptance data. T-0 identity against Interactive,
//!   static accumulation checkpoints with the standard, sun-only and visibility programs,
//!   cuts, and in-process static repeats.
//!
//! - `gpu_film_t2_static`: T2a's static regression (docs/m6-motion-aa.md section 7): Q-static
//!   with sample records against T1a, byte for byte.
//! - `gpu_film_t2_motion`: T2a camera-motion data: a registered sequence from cold with each
//!   scored program, record diagnostics, T-0 on pixels without records, and replays.
//! - `gpu_film_t2_batch`: T2's follow-up checks in one process: per-record probes, the scatter
//!   variant's identity with the gathers, and paired gather and scatter timing.
//!
//! Manifest: the quarry fields of `load_quarry`, `output`, `t1_cameras` (a cameras file
//! whose frames are the scored poses), `poses`, and for the temporal test `frames`,
//! `checkpoints`, `repeats` and `cut` ([from, to] pose indices). The T2a tests read
//! `records` (4 or 8); the motion test reads `sequence` (a cameras file), `scored`,
//! `t0_frames` and `replays`.
use super::*;
use primary_ray_gpu::{film_accumulation, film_records};

/// Two independent stratified halves of 512 samples (32 x 16 strata each), randomized in
/// each stratum, so each half is an unbiased footprint estimate on its own.
const T1_JITTER: &str = "\nfn film_jitter(i:u32)->vec2<f32> {\n let h=fresh_hash(i^0x6a09e667u);\n let q=vec2<f32>(f32(h>>8u),f32(fresh_hash(h)>>8u))*(1.0/16777216.0);\n let j=i%512u;\n return (vec2<f32>(f32(j%32u),f32(j/32u))+q)/vec2<f32>(32.0,16.0);\n}\n";
const SAMPLES: u32 = 1024;
const RAYS: f64 = 32.0;
const PIXEL_BYTES: u64 = 64;

/// The standard program with the environment term reset before its light loop: sun only.
fn direct_program() -> String {
    edit(
        STANDARD,
        (
            "    for(var i=0u;i<light_count();i+=1u) {",
            "    exposed=vec3<f32>(0.0);\n    for(var i=0u;i<light_count();i+=1u) {",
        ),
    )
}

/// Misses shade black instead of the lit clear colour (sun-only and visibility images).
/// Soft modules name the clear colour at three stores; temporal modules once, in
/// `film_background`, and motion shading modules once, in `film_clear_colour`.
fn black_clear(source: &str) -> String {
    let sites = source.matches("primary_frame.clear").count();
    let temporal =
        source.contains("fn film_background()") || source.contains("fn film_clear_colour()");
    assert_eq!(sites, if temporal { 1 } else { 3 });
    source.replace("primary_frame.clear", "vec4<f32>(0.0,0.0,0.0,1.0)")
}

/// The scored programs: name, source and whether misses are black.
fn programs() -> [(&'static str, String, bool); 3] {
    [
        ("full", STANDARD.to_owned(), false),
        ("sun", direct_program(), true),
        (
            "visibility",
            crate::program::standard_returning(VISIBILITY),
            true,
        ),
    ]
}

/// Production Hard/Soft shading of `program`.
fn centre_shading(r: &Renderer, program: &str, black: bool) -> [Owned<wgpu::ComputePipeline>; 2] {
    let sources = primary_ray_gpu::shading_sources(SOURCE, program).unwrap();
    shading_pipelines(r, &sources.map(|s| if black { black_clear(&s) } else { s }))
}

/// Production temporal shading of `program`.
fn temporal_shading(r: &Renderer, program: &str, black: bool) -> Owned<wgpu::ComputePipeline> {
    let head = film_accumulation::shading_head(&primary_ray_gpu::shading_heads(SOURCE).unwrap()[1])
        .unwrap();
    let mut source = film_edit(film_accumulation::compose_shading(&head, program, false).source);
    if black {
        source = black_clear(&source);
    }
    let scopes = OperationScopes::new(&r.device);
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test temporal shading"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = r
        .primary
        .as_ref()
        .unwrap()
        .film_shade_pipeline(&r.device, &r.ledger, &module, None);
    assert!(
        scopes.finish().is_none(),
        "temporal shading failed to compile"
    );
    pipeline
}

/// Diagnostic manifest edit (`film_edit`: [find, replace]) of the temporal sequence text,
/// applied to every temporal module a test builds; registered runs omit it.
fn film_edit(source: String) -> String {
    let m = manifest();
    match m["film_edit"].as_array() {
        Some(pair) => edit(
            &source,
            (pair[0].as_str().unwrap(), pair[1].as_str().unwrap()),
        ),
        None => source,
    }
}

/// Temporal shading of `program` that stores each pixel's current sample instead of the
/// film (no history writes), for test passes over a temporal frame's own samples.
fn per_sample_shading(r: &Renderer, program: &str) -> Owned<wgpu::ComputePipeline> {
    let head = film_accumulation::shading_head(&primary_ray_gpu::shading_heads(SOURCE).unwrap()[1])
        .unwrap();
    let source = black_clear(&film_edit(
        film_accumulation::compose_shading(&head, program, false).source,
    ));
    let signature = "fn film_store(pixel:vec2<i32>,sample:vec3<f32>,placement:u32,distance:f32) {";
    let source = format!(
        "{}\n{signature}textureStore(primary_hdr,pixel,vec4<f32>(sample,1.0));}}\n",
        edit(
            &source,
            (
                signature,
                &signature.replace("film_store", "film_store_history")
            )
        )
    );
    let scopes = OperationScopes::new(&r.device);
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test per-sample temporal shading"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = r
        .primary
        .as_ref()
        .unwrap()
        .film_shade_pipeline(&r.device, &r.ledger, &module, None);
    assert!(
        scopes.finish().is_none(),
        "per-sample shading failed to compile"
    );
    pipeline
}

/// Visibility of a temporal sequence's own samples on each pixel's centre surface
/// (support/film_t1_visibility.wgsl): the reference's R_F estimand, accumulated with the
/// cumulative weights static T1 uses.
/// Two result halves (accumulated by frame parity) estimate its noise.
struct ConditionedVisibility {
    shading: Owned<wgpu::ComputePipeline>,
    accumulator: wgpu::ComputePipeline,
    bind: [wgpu::BindGroup; 2],
    result: [wgpu::Buffer; 2],
    size: [u32; 2],
}
impl ConditionedVisibility {
    /// `centre` is (placement + 1, depth along `forward`) per pixel of a centre frame.
    fn new(r: &Renderer, centre: &[u8], forward: Vec3) -> Self {
        let program = crate::program::standard_returning(&format!(
            "rl_conditioned(hard_shadow(lights[0],input.receiver,-lights[0].travel_outer.xyz),dot(input.receiver.world-input.eye,vec3<f32>({:?},{:?},{:?})))",
            forward.x, forward.y, forward.z
        )) + "
// The f16 target keeps the depth as a high part and its residual, biased by high/1024
// because rl_finite clears negatives (|residual| <= high/2048).
fn rl_conditioned(v:f32,depth:f32)->vec3<f32> { let high=quantizeToF16(depth); return vec3<f32>(v,high,depth-high+high/1024.0); }
";
        let buffer = |label, size, usage| {
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let centre_buffer = buffer(
            "T1 conditioned centre",
            centre.len() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        r.queue.write_buffer(&centre_buffer, 0, centre);
        let result = [0, 1].map(|_| {
            buffer(
                "T1 conditioned visibility",
                centre.len() as u64,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            )
        });
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("T1 conditioned visibility"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../support/film_t1_visibility.wgsl").into(),
            ),
        });
        let accumulator = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("T1 conditioned visibility"),
                layout: None,
                module: &module,
                entry_point: Some("accumulate"),
                compilation_options: Default::default(),
                cache: None,
            });
        let primary = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let size = [primary.hit.texture.width(), primary.hit.texture.height()];
        let views = [
            &primary.hit.texture,
            &r.lighting.hdr.as_ref().unwrap().resolved.texture,
            &primary.tiles.texture,
        ]
        .map(|t| t.create_view(&Default::default()));
        let mut entries: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: centre_buffer.as_entire_binding(),
        });
        let bind = result.each_ref().map(|result| {
            let mut entries = entries.clone();
            entries.push(wgpu::BindGroupEntry {
                binding: 4,
                resource: result.as_entire_binding(),
            });
            r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("T1 conditioned visibility"),
                layout: &accumulator.get_bind_group_layout(0),
                entries: &entries,
            })
        });
        Self {
            shading: per_sample_shading(r, &program),
            accumulator,
            bind,
            result,
            size,
        }
    }
    /// Re-shades the last temporal frame's samples and accumulates their visibility into
    /// `half`. The frame's HDR image is overwritten by the per-sample output.
    fn accumulate(&self, r: &Renderer, half: usize) {
        let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
        let mut e = r.device.create_command_encoder(&Default::default());
        r.encode_shade_group(
            &mut e,
            &self.shading,
            film.shade_group(),
            primary_ray_gpu::SHADE_TILE,
            None,
        );
        {
            let mut pass = e.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.accumulator);
            pass.set_bind_group(0, &self.bind[half], &[]);
            pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(4), 1);
        }
        r.queue.submit([e.finish()]);
    }
}

/// (placement + 1, depth along the camera forward) per pixel of an Exact centre frame.
fn centre_surface(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    camera: &Camera,
) -> (Vec<u8>, Vec3) {
    r.set_soft_shadows(crate::SoftShadows::Exact);
    traced_frame(r, color, camera, asset, true);
    r.test_drain("T1 centre surface");
    let [world, _, _] = receivers(r);
    let hit = hit_record(r);
    let inverse = camera.view().inverse();
    let (eye, forward) = (inverse.w_axis.truncate(), -inverse.z_axis.truncate());
    let centre = world
        .chunks_exact(16)
        .zip(hit.chunks_exact(16))
        .flat_map(|(w, h)| {
            let p = Vec3::from_array(std::array::from_fn(|i| {
                f32::from_le_bytes(w[i * 4..i * 4 + 4].try_into().unwrap())
            }));
            h[..4]
                .iter()
                .copied()
                .chain((p - eye).dot(forward).to_le_bytes())
                .collect::<Vec<_>>()
        })
        .collect();
    (centre, forward)
}

fn t1_cameras(m: &serde_json::Value) -> Vec<Camera> {
    let cameras =
        cameras::Cameras::load(&PathBuf::from(m["t1_cameras"].as_str().unwrap())).unwrap();
    cameras.frames.iter().map(|f| f.camera().unwrap()).collect()
}

fn poses(m: &serde_json::Value) -> Vec<usize> {
    m["poses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as usize)
        .collect()
}

/// Production Exact frames with seeds 0..exact and Interactive frames with seeds
/// 0..interactive at `camera`: full and sun-only HDR, counts and hit records.
fn centre_frames(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    camera: &Camera,
    out: &std::path::Path,
    [exact, interactive]: [u32; 2],
) {
    let module = seeded_module(r);
    let mut sun = centre_shading(r, &direct_program(), true);
    for (mode, seeds) in [("exact", exact), ("interactive", interactive)] {
        for seed in 0..seeds {
            let primary = r.primary.as_ref().unwrap();
            let mut pipeline = if mode == "exact" {
                primary.fresh_pipeline(
                    &r.device,
                    &r.ledger,
                    &module,
                    "fresh_compute",
                    &[("FRESH_SEED", f64::from(seed))],
                )
            } else {
                primary.interactive_pipeline(
                    &r.device,
                    &r.ledger,
                    &module,
                    &[("FRESH_SEED", f64::from(seed)), ("FRESH_DIRECTIONS", 8.0)],
                )
            };
            r.set_soft_shadows(if mode == "exact" {
                crate::SoftShadows::Exact
            } else {
                crate::SoftShadows::Interactive
            });
            // The seeded estimator replaces the production one for this seed's frames.
            let swap = |r: &mut Renderer, pipeline: &mut Owned<wgpu::ComputePipeline>| {
                let p = r.pipeline.primary.as_mut().unwrap();
                let slot = if mode == "exact" {
                    &mut p.fresh
                } else {
                    &mut p.interactive
                };
                std::mem::swap(slot, pipeline);
            };
            swap(r, &mut pipeline);
            let prefix = out.join(format!("{mode}-seed-{seed}"));
            traced_frame(r, color, camera, asset, true);
            fs::write(prefix.with_extension("rgba16f"), capture(r)).unwrap();
            let counts = if mode == "exact" {
                receivers(r)[2].clone()
            } else {
                interactive_outputs(r)[1].clone()
            };
            fs::write(prefix.with_extension("u32"), counts).unwrap();
            std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut sun);
            traced_frame(r, color, camera, asset, true);
            fs::write(prefix.with_extension("sun.rgba16f"), capture(r)).unwrap();
            std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut sun);
            swap(r, &mut pipeline);
        }
    }
    r.set_soft_shadows(crate::SoftShadows::Exact);
}

/// The 1,024 x 32 film reference at `camera`, summed over `replicas` independent
/// replicas (each offsets the sample index, so its jitter and shadow-ray salts are new).
/// `visibility_only` skips the sun-only radiance. A production Exact centre frame precedes
/// it; its receivers and hit records define the centre surface visibility conditions on.
#[allow(clippy::too_many_arguments)]
fn integrate_t1(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    camera: &Camera,
    out: &std::path::Path,
    pose: usize,
    replicas: u32,
    visibility_only: bool,
) {
    r.set_soft_shadows(crate::SoftShadows::Exact);
    traced_frame(r, color, camera, asset, true);
    r.test_drain("T1 reference centre");
    let start = Instant::now();
    let [world, normal, counts] = receivers(r);
    let hit = hit_record(r);
    let prefix = out.join(format!("pose-{pose}"));
    for (suffix, data) in [
        ("world.f32", &world),
        ("normal.f32", &normal),
        ("exact.u32", &counts),
        ("hit.u32", &hit),
    ] {
        fs::write(prefix.with_extension(suffix), data).unwrap();
    }
    fs::write(prefix.with_extension("exact.rgba16f"), capture(r)).unwrap();
    let centre: Vec<u8> = world
        .chunks_exact(16)
        .zip(hit.chunks_exact(16))
        .flat_map(|(w, h)| w[..12].iter().chain(h[..4].iter()).copied())
        .collect();
    let make = |label, size, usage| {
        r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
    let pixels = (world.len() / 16) as u64;
    let centre_buffer = make(
        "T1 centre surface",
        centre.len() as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    r.queue.write_buffer(&centre_buffer, 0, &centre);
    let sums = [
        make("T1 film sums", pixels * PIXEL_BYTES, storage),
        make("T1 direct sums", pixels * PIXEL_BYTES, storage),
    ];
    let params = make(
        "T1 reference parameters",
        48,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("T1 reference accumulator"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../support/film_reference_t1.wgsl").into()),
    });
    let accumulator = r
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("T1 reference accumulator"),
            layout: None,
            module: &module,
            entry_point: Some("accumulate"),
            compilation_options: Default::default(),
            cache: None,
        });
    let primary = r.primary.as_ref().unwrap();
    let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
    let pt = primary.targets.as_ref().unwrap();
    let views = [
        &pt.hit.texture,
        &t.world.texture,
        &t.visibility.texture,
        &r.lighting.hdr.as_ref().unwrap().resolved.texture,
        &pt.tiles.texture,
    ]
    .map(|t| t.create_view(&Default::default()));
    let binds = sums.each_ref().map(|sum| {
        let mut entries: Vec<_> = views[..4]
            .iter()
            .enumerate()
            .map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 4,
                resource: centre_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: sum.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&views[4]),
            },
        ]);
        r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("T1 reference accumulator"),
            layout: &accumulator.get_bind_group_layout(0),
            entries: &entries,
        })
    });
    let sources = film_sources_with(T1_JITTER);
    let pipelines = primary.test_film_pipelines(&r.device, &sources.kernels, &sources.shading);
    let kernels = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("T1 reference 32-ray estimator"),
        source: wgpu::ShaderSource::Wgsl(sources.kernels.as_str().into()),
    });
    let fresh = primary.fresh_pipeline(
        &r.device,
        &r.ledger,
        &kernels,
        "fresh_compute",
        &[("FRESH_DIRECTIONS", RAYS)],
    );
    let direct = primary.test_film_pipelines(
        &r.device,
        &sources.kernels,
        &black_clear(&film_shading_with(&direct_program(), T1_JITTER)),
    );
    let [w, h] = [pt.hit.texture.width(), pt.hit.texture.height()];
    let inverse = camera.view().inverse();
    let eye = inverse.w_axis;
    let forward = -inverse.z_axis;
    let mut values = Vec::new();
    for v in eye.to_array().into_iter().chain(forward.to_array()) {
        values.extend(v.to_le_bytes());
    }
    values.resize(48, 0);
    for global in 0..replicas * SAMPLES {
        let sample = global % SAMPLES;
        r.queue
            .write_buffer(&r.uniform, 68, &(global + 1).to_le_bytes());
        values[32..36].copy_from_slice(&sample.to_le_bytes());
        r.queue.write_buffer(&params, 0, &values);
        let mut e = r.device.create_command_encoder(&Default::default());
        r.encode_primary_trace(
            &mut e,
            &pipelines[0],
            primary_ray_gpu::TRACE_TILE,
            true,
            None,
        );
        r.encode_primary_fresh(&mut e, &fresh, None);
        let passes = [(&pipelines[2], &binds[0]), (&direct[2], &binds[1])];
        for &(shading, bind) in &passes[..if visibility_only { 1 } else { 2 }] {
            r.encode_primary_shade(
                &mut e,
                shading,
                primary_ray_gpu::SHADE_TILE,
                primary_ray_gpu::Shading::Soft,
                None,
            );
            let mut pass = e.begin_compute_pass(&Default::default());
            pass.set_pipeline(&accumulator);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(4), 1);
        }
        r.queue.submit([e.finish()]);
        // Bound queue residence and the OS watchdog; reference throughput only.
        if global % 32 == 31 {
            r.test_drain("T1 reference batch");
        }
    }
    r.queue.write_buffer(&r.uniform, 68, &0u32.to_le_bytes());
    for (kind, sum) in ["film", "direct"]
        .into_iter()
        .zip(&sums)
        .take(if visibility_only { 1 } else { 2 })
    {
        fs::write(
            prefix.with_extension(format!("{kind}.bin")),
            read_buffer(r, sum, pixels * PIXEL_BYTES),
        )
        .unwrap();
    }
    fs::write(prefix.with_extension("json"),serde_json::to_vec_pretty(&serde_json::json!({"pose":pose,"size":[w,h],"samples":SAMPLES,"replicas":replicas,"rays_per_sample":RAYS,"halves":"index/512, each 32x16 strata randomized within stratum","seconds":start.elapsed().as_secs_f64(),"camera_inverse":camera.matrix(w,h).inverse().to_cols_array(),"eye":eye.to_array(),"forward":forward.to_array(),"receiver_policy":"Production (viewed side, max(10um,16ulp)), epsilon t_min","layout":"counts[vis0,vis1,acc0,acc1] u32, moments[vis0^2,vis1^2,-,-]/65536, radiance0 rgba, radiance1 rgba"})).unwrap()).unwrap();
}

/// The footprint verifier's module: the temporal kernel with per-sample salted shadow rays.
fn footprint_source() -> String {
    let kernels = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    format!(
        "{}\n{}",
        edit(
            &film_accumulation::kernel_source(&kernels[1]).unwrap(),
            (
                SEED_EDIT.0,
                "    var seed=fresh_hash(pixel.x+pixel.y*width);\n    seed=fresh_hash(seed^(film_footprint_seed*0x9e3779b9u));",
            ),
        ),
        include_str!("../support/film_t1_footprint.wgsl")
    )
}

#[test]
fn t1_reference_sources_compile() {
    let sources = film_sources_with(T1_JITTER);
    for source in [
        sources.kernels,
        sources.shading,
        black_clear(&film_shading_with(&direct_program(), T1_JITTER)),
    ] {
        validate_wgsl(&source);
    }
    validate_wgsl(include_str!("../support/film_reference_t1.wgsl"));
    validate_wgsl(&footprint_source());
}

#[test]
#[ignore = "owned GPU lease; T1 film references and centre frames, offscreen"]
fn gpu_film_t1_reference() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let cameras = t1_cameras(&m);
    let root = PathBuf::from(m["output"].as_str().unwrap());
    for pose in poses(&m) {
        let out = root.join(format!("ref-p{pose}"));
        // Completed poses are kept, so an interrupted run resumes at the next one.
        if out.join("complete").exists() {
            continue;
        }
        fs::create_dir_all(&out).unwrap();
        let camera = &cameras[pose];
        let replicas = m["replicas"].as_u64().unwrap_or(1) as u32;
        let visibility_only = m["visibility_only"].as_bool().unwrap_or(false);
        integrate_t1(
            &mut r,
            &mut asset,
            &color,
            camera,
            &out,
            pose,
            replicas,
            visibility_only,
        );
        if visibility_only {
            fs::write(out.join("complete"), b"").unwrap();
            continue;
        }
        centre_frames(&mut r, &mut asset, &color, camera, &out, [8, 4]);
        fs::write(out.join("complete"), b"").unwrap();
        eprintln!("T1 reference pose={pose} done");
    }
}

/// The HDR image of the last frame once every frame in flight has completed.
fn settled(r: &mut Renderer) -> Vec<u8> {
    r.test_drain("T1 settled capture");
    capture(r)
}

/// SHA-256 of a capture, for repeat comparisons without retaining the bytes.
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Both history sides of the current film state, packed.
fn history(r: &Renderer) -> Vec<u8> {
    let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
    let written = film.parity;
    let [colour, anchor, tiles] = film.history(written);
    [
        read_texture(r, colour),
        read_texture(r, anchor),
        read_texture(r, tiles),
    ]
    .concat()
}

/// Discards the film history: the next temporal frame resets every pixel.
fn reset_film(r: &mut Renderer) {
    r.set_soft_shadows(crate::SoftShadows::Exact);
    r.set_soft_shadows(crate::SoftShadows::Temporal);
}

#[test]
#[ignore = "owned GPU lease; T1a static accumulation acceptance data, offscreen"]
fn gpu_film_t1_temporal() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let cameras = t1_cameras(&m);
    let root = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&root).unwrap();
    let frames = m["frames"].as_u64().unwrap_or(128) as u32;
    let checkpoints: Vec<u32> = m["checkpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u32)
        .collect();
    // Compile the temporal pipelines once, then swap scored programs into them.
    r.set_soft_shadows(crate::SoftShadows::Temporal);
    traced_frame(&mut r, &color, &cameras[0], &mut asset, true);
    let report = traced_frame(&mut r, &color, &cameras[0], &mut asset, true);
    assert_eq!(report.soft_shadows, crate::SoftShadows::Temporal);
    if m["film_edit"].is_array() {
        let source = film_edit(
            film_accumulation::kernel_source(&primary_ray_gpu::shader_sources(SOURCE).unwrap()[1])
                .unwrap(),
        );
        let kernel = r
            .primary
            .as_ref()
            .unwrap()
            .film_kernel(&r.device, &r.ledger, &source);
        r.pipeline.primary.as_mut().unwrap().film = Some(kernel);
    }
    let mut results = serde_json::Map::new();
    let mut identity = vec![];
    for pose in poses(&m) {
        let camera = &cameras[pose];
        let out = root.join(format!("t1-p{pose}"));
        fs::create_dir_all(&out).unwrap();
        let (surface, forward) = centre_surface(&mut r, &mut asset, &color, camera);
        let conditioned = ConditionedVisibility::new(&r, &surface, forward);
        let watch: Vec<[u32; 2]> = m["watch"][pose.to_string()]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| [0, 1].map(|i| p[i].as_u64().unwrap() as u32))
                    .collect()
            })
            .unwrap_or_default();
        let mut watched = vec![];
        for (name, program, black) in programs() {
            let mut centre = centre_shading(&r, &program, black);
            let mut temporal = temporal_shading(&r, &program, black);
            // T-0: a reset frame equals Interactive's frame, byte for byte. Reset frames shade
            // with the program's traced pipelines, so the variant stays installed throughout.
            r.set_soft_shadows(crate::SoftShadows::Interactive);
            std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut centre);
            traced_frame(&mut r, &color, camera, &mut asset, true);
            let interactive = [settled(&mut r), interactive_outputs(&r).concat()];
            reset_film(&mut r);
            std::mem::swap(r.test_standard().temporal.as_mut().unwrap(), &mut temporal);
            let mut digests = vec![];
            for n in 1..=frames {
                traced_frame(&mut r, &color, camera, &mut asset, true);
                if name == "full" && !watch.is_empty() {
                    r.test_drain("T1 watch");
                    let [hit, counts] = interactive_outputs(&r);
                    for &[x, y] in &watch {
                        let i = (x + y * 1920) as usize;
                        let word = |b: &[u8], j: usize| {
                            u32::from_le_bytes(b[j * 4..j * 4 + 4].try_into().unwrap())
                        };
                        watched.push(serde_json::json!({"pixel":[x,y],"frame":n,
                            "record":[word(&hit, i * 4), word(&hit, i * 4 + 1), word(&hit, i * 4 + 2), word(&hit, i * 4 + 3)],
                            "barycentrics":[f32::from_bits(word(&hit, i * 4 + 2)), f32::from_bits(word(&hit, i * 4 + 3))],
                            "count":word(&counts, i)}));
                    }
                }
                if n == 1 {
                    let first = [settled(&mut r), interactive_outputs(&r).concat()];
                    let same = [first[0] == interactive[0], first[1] == interactive[1]];
                    identity.push(serde_json::json!({"pose":pose,"program":name,"hdr_identical":same[0],"hit_and_counts_identical":same[1]}));
                    assert!(same[0] && same[1], "T-0 pose={pose} program={name}");
                }
                if checkpoints.contains(&n) {
                    let hdr = settled(&mut r);
                    digests.push(serde_json::json!({"n":n,"sha256":digest(&hdr)}));
                    fs::write(out.join(format!("{name}-n{n:03}.rgba16f")), hdr).unwrap();
                }
                if name == "visibility" {
                    // After the film capture: this pass overwrites the frame's HDR image.
                    conditioned.accumulate(&r, 0);
                    if checkpoints.contains(&n) {
                        let size = conditioned.result[0].size();
                        fs::write(
                            out.join(format!("conditioned-n{n:03}.u32")),
                            read_buffer(&mut r, &conditioned.result[0], size),
                        )
                        .unwrap();
                    }
                }
            }
            std::mem::swap(r.test_standard().temporal.as_mut().unwrap(), &mut temporal);
            std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut centre);
            results.insert(format!("p{pose}/{name}"), serde_json::json!(digests));
            if !watched.is_empty() {
                fs::write(
                    out.join("watch.json"),
                    serde_json::to_vec_pretty(&watched).unwrap(),
                )
                .unwrap();
            }
            eprintln!("T1 temporal pose={pose} program={name} done");
        }
    }
    // Cuts: converge at `from`, cut to `to`, then back. A cut is a global reset with no
    // history reads, so every cut frame must equal the cold-start frame of its pose.
    let [from, to] = [0, 1].map(|i| m["cut"][i].as_u64().unwrap() as usize);
    let mut cuts = vec![];
    reset_film(&mut r);
    for n in 1..=64 {
        traced_frame(&mut r, &color, &cameras[from], &mut asset, true);
        if n == 64 {
            cuts.push(
                serde_json::json!({"segment":"converged","n":n,"sha256":digest(&settled(&mut r))}),
            );
        }
    }
    for (segment, pose) in [("cut", to), ("back", from)] {
        for n in 1..=32 {
            traced_frame(&mut r, &color, &cameras[pose], &mut asset, true);
            if checkpoints.contains(&n) {
                cuts.push(serde_json::json!({"segment":segment,"pose":pose,"n":n,"sha256":digest(&settled(&mut r))}));
            }
        }
    }
    // F8: in-process static repeats of the full program's sequence at the first pose.
    let pose = poses(&m)[0];
    let repeats = m["repeats"].as_u64().unwrap_or(32) as usize;
    let mut first: Option<(Vec<String>, Vec<u8>)> = None;
    let mut identical = 0;
    let mut differing = vec![];
    for repeat in 0..repeats {
        reset_film(&mut r);
        let mut run = vec![];
        for n in 1..=frames {
            traced_frame(&mut r, &color, &cameras[pose], &mut asset, true);
            if [1, 2, 8, 32, frames].contains(&n) {
                run.push(digest(&settled(&mut r)));
            }
        }
        let state = history(&r);
        run.push(digest(&state));
        match &first {
            None => first = Some((run, state)),
            Some((expected, _)) if expected == &run => identical += 1,
            Some((expected, base)) => {
                // Keep both histories and classify the differing words before failing.
                fn word(b: &[u8], i: usize) -> &[u8] {
                    &b[i * 4..i * 4 + 4]
                }
                let pixels = 1920 * 1080;
                let mut channels = [0usize; 6];
                for i in 0..pixels {
                    for (c, count) in channels.iter_mut().enumerate() {
                        let w = if c < 4 {
                            i * 4 + c
                        } else {
                            pixels * 4 + i * 2 + c - 4
                        };
                        *count += usize::from(word(base, w) != word(&state, w));
                    }
                }
                fs::write(root.join(format!("repeat-{repeat}-history.bin")), &state).unwrap();
                fs::write(root.join("repeat-first-history.bin"), base).unwrap();
                differing.push(serde_json::json!({"repeat":repeat,
                    "checkpoints_identical":expected[..expected.len() - 1] == run[..run.len() - 1],
                    "differing_words":{"mean_r":channels[0],"mean_g":channels[1],"mean_b":channels[2],
                        "weight":channels[3],"anchor_x":channels[4],"anchor_y":channels[5]}}));
            }
        }
    }
    // History kept only in tiles a ray has hit must give the images of history kept in every
    // tile, frame by frame, and the same history wherever it is kept.
    let mut runs = vec![];
    for materialize in [false, true] {
        r.primary.as_mut().unwrap().test_film_materialize = materialize;
        reset_film(&mut r);
        let mut images = vec![];
        for _ in 1..=frames {
            traced_frame(&mut r, &color, &cameras[pose], &mut asset, true);
            images.push(digest(&settled(&mut r)));
        }
        let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
        let [colour, anchor, tiles] = film.history(film.parity).map(|t| read_texture(&r, t));
        runs.push((images, colour, anchor, tiles));
    }
    r.primary.as_mut().unwrap().test_film_materialize = false;
    let (kept, all) = (&runs[0], &runs[1]);
    let tile_width = 1920u32.div_ceil(8) as usize;
    let flag = |tiles: &[u8], i: usize| tiles[i * 4..i * 4 + 4] != [0; 4];
    let (mut kept_pixels, mut history_differs) = (0usize, 0usize);
    for i in 0..1920 * 1080 {
        let tile = (i % 1920) / 8 + (i / 1920) / 4 * tile_width;
        if flag(&kept.3, tile) {
            kept_pixels += 1;
            history_differs += usize::from(
                kept.1[i * 16..i * 16 + 16] != all.1[i * 16..i * 16 + 16]
                    || kept.2[i * 8..i * 8 + 8] != all.2[i * 8..i * 8 + 8],
            );
        }
    }
    let tiles = kept.3.len() / 4;
    let skip = serde_json::json!({"pose":pose,"frames":frames,
        "frames_identical":kept.0.iter().zip(&all.0).filter(|(a, b)| a == b).count(),
        "kept_tiles":(0..tiles).filter(|&t| flag(&kept.3, t)).count(),"tiles":tiles,
        "materialized_tiles":(0..tiles).filter(|&t| flag(&all.3, t)).count(),
        "kept_pixels":kept_pixels,"kept_history_differing_pixels":history_differs});
    fs::write(
        root.join("temporal.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":frames,"checkpoints":checkpoints,
            "identity":identity,"digests":results,"cuts":cuts,
            "repeats":{"pose":pose,"runs":repeats,"identical_to_first":identical,"differing":differing,
                "digests":first.as_ref().map(|f| &f.0)},"background_history":skip}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(differing.is_empty(), "static repeats differ: {differing:?}");
    assert!(
        kept.0 == all.0 && history_differs == 0,
        "history kept only where rays hit differs from history everywhere: {skip}"
    );
}

/// Paired complete-frame timing on one renderer: production Interactive and Temporal frames
/// alternate (order rotating per pair) at the first manifest pose, or along `sequence` (one
/// camera per pair, cycling), after warmup pairs. Temporal frames keep accumulating: the
/// steady static state, or with `records` (4 or 8) steady motion, each sample marked moving
/// or static. Validation is off.
#[test]
#[ignore = "owned GPU lease; T1 paired frame timing, offscreen"]
fn gpu_film_t1_timing() {
    let m = manifest();
    let _validation = validation::begin(false);
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    r.set_film_motion(film_motion(&m));
    let path: Vec<Camera> = match m["sequence"].as_str() {
        Some(file) => cameras::Cameras::load(&PathBuf::from(file))
            .unwrap()
            .frames
            .iter()
            .map(|f| f.camera().unwrap())
            .collect(),
        None => vec![t1_cameras(&m)[poses(&m)[0]]],
    };
    let warmup = m["warmup_pairs"].as_u64().unwrap_or(24) as usize;
    let pairs = m["pairs"].as_u64().unwrap_or(48) as usize;
    // Cost attribution: `record_edits` ([find, replace] pairs) recompile the record kernels.
    if let Some(edits) = m["record_edits"].as_array() {
        r.soft_mode = crate::SoftShadows::Temporal;
        traced_frame(&mut r, &color, &path[0], &mut asset, true);
        r.test_record_kernel_edits(&string_pairs(&serde_json::json!(edits)));
    }
    // Scatter pricing: `scatter` "only" or "alternate", `scatter_edits` its module edits.
    let scatter = m["scatter"].as_str();
    if scatter.is_some() {
        r.soft_mode = crate::SoftShadows::Temporal;
        traced_frame(&mut r, &color, &path[0], &mut asset, true);
        r.test_record_scatter(&string_pairs(&m["scatter_edits"]));
    }
    let samples = paired_timing(&mut r, &color, &mut asset, &path, [warmup, pairs], scatter);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("paired-timing.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"warmup_pairs":warmup,"pairs":pairs,
            "sequence":m["sequence"],"records":r.film_motion().records(),
            "record_edits":m["record_edits"],"samples":samples}))
        .unwrap(),
    )
    .unwrap();
}

/// `[[find, replace], ...]` as owned pairs (absent: none).
fn string_pairs(v: &serde_json::Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|e| [0, 1].map(|i| e[i].as_str().unwrap().to_owned()).into())
                .collect()
        })
        .unwrap_or_default()
}

/// Paired Interactive and Temporal frames along `path`: `warmup` then `pairs` pairs, the order
/// within a pair alternating, with each measured frame's GPU pass times. Temporal frames keep
/// their history between pairs. `scatter` "only" runs every Temporal frame on the scatter variant
/// (`Renderer::test_record_scatter`), "alternate" every other one, so gather and scatter moving
/// frames share the process, the path and the Interactive pairing.
fn paired_timing(
    r: &mut Renderer,
    color: &wgpu::Texture,
    asset: &mut prepared::Prepared,
    path: &[Camera],
    [warmup, pairs]: [usize; 2],
    scatter: Option<&str>,
) -> Vec<serde_json::Value> {
    assert!(matches!(scatter, None | Some("only" | "alternate")), "scatter {scatter:?}");
    let modes = [
        crate::SoftShadows::Interactive,
        crate::SoftShadows::Temporal,
    ];
    type Pending = std::collections::HashMap<u64, (usize, usize, bool, &'static str)>;
    fn collect(r: &mut Renderer, pending: &mut Pending, samples: &mut Vec<serde_json::Value>) {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        for g in r.poll().unwrap().gpu {
            if let Some((pair, mode, moving, variant)) = pending.remove(&g.frame.0) {
                samples.push(serde_json::json!({"pair":pair,
                    "mode":if mode == 0 {"Interactive"} else {"Temporal"},
                    "moving":moving,"variant":variant,
                    "host_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64(),
                    "trace_ms":g.receiver_pass.map(ms),"compute_ms":g.visibility_compute.map(ms),"shading_ms":ms(g.mesh_pass),
                    "tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
            }
        }
    }
    let (mut pending, mut samples) = (Pending::new(), vec![]);
    let mut temporal_frames = 0usize;
    for pair in 0..warmup + pairs {
        let camera = path[pair % path.len()];
        for j in 0..2 {
            let mode = (pair + j) % 2;
            // set_soft_shadows would release the film history when leaving Temporal; the
            // field keeps it, so Temporal frames stay in their steady state.
            r.soft_mode = modes[mode];
            let scattered = mode == 1
                && match scatter {
                    Some("only") => true,
                    Some(_) => temporal_frames % 2 == 1,
                    None => false,
                };
            temporal_frames += mode;
            r.primary.as_mut().unwrap().test_scatter_active = scattered;
            let report = traced_frame(r, color, &camera, asset, true);
            assert_eq!(report.soft_shadows, modes[mode]);
            let moving = mode == 1 && r.primary.as_ref().unwrap().film.as_ref().unwrap().moving;
            let variant = match (mode, scattered) {
                (0, _) => "interactive",
                (_, true) => "scatter",
                _ => "gather",
            };
            if pair >= warmup {
                pending.insert(report.frame.0, (pair - warmup, mode, moving, variant));
            }
            collect(r, &mut pending, &mut samples);
        }
    }
    r.test_drain("T1 paired timing");
    collect(r, &mut pending, &mut samples);
    r.primary.as_mut().unwrap().test_scatter_active = false;
    assert!(
        pending.is_empty(),
        "{} frames without GPU samples; timing {:?}",
        pending.len(),
        r.timing_status
    );
    samples
}

/// G2-style local verification of reference-solid pixels where T1 deviates: manifest
/// `footprint` maps a pose to `[x, y, expected]` triples (expected: 1 lit, 0 umbra) and
/// `grid` sets the G x G stratified sub-pixel samples per pixel, 32 salted rays each.
#[test]
#[ignore = "owned GPU lease; T1 footprint verification of solid reference pixels, offscreen"]
fn gpu_film_t1_footprint() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let cameras = t1_cameras(&m);
    let grid = m["grid"].as_u64().unwrap_or(1024) as u32;
    let source = footprint_source();
    let mut results = serde_json::Map::new();
    let start = Instant::now();
    for (pose, pixels) in m["footprint"].as_object().unwrap() {
        let camera = &cameras[pose.parse::<usize>().unwrap()];
        let (surface, forward) = centre_surface(&mut r, &mut asset, &color, camera);
        let eye = camera.view().inverse().w_axis;
        // [x, y, expected] verifies the footprint; [x, y, expected, cx, cy, spacing] samples a
        // micro grid of that spacing around (cx, cy) inside the pixel.
        let micro: Vec<[f32; 3]> = pixels
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                std::array::from_fn(|i| p.get(3 + i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32)
            })
            .collect();
        let pixels: Vec<[u32; 3]> = pixels
            .as_array()
            .unwrap()
            .iter()
            .map(|p| std::array::from_fn(|i| p[i].as_u64().unwrap() as u32))
            .collect();
        let mut targets = vec![];
        for (&[x, y, expected], &[cx, cy, spacing]) in pixels.iter().zip(&micro) {
            let i = (x + y * 1920) as usize * 8;
            targets.extend([x, y, 1920, 1080].iter().flat_map(|v| v.to_le_bytes()));
            targets.extend_from_slice(&surface[i..i + 8]);
            targets.extend([expected, 0].iter().flat_map(|v| v.to_le_bytes()));
            targets.extend(eye.to_array().iter().flat_map(|v| v.to_le_bytes()));
            targets.extend(
                forward
                    .extend(0.0)
                    .to_array()
                    .iter()
                    .flat_map(|v| v.to_le_bytes()),
            );
            targets.extend([cx, cy, spacing, 0.0].iter().flat_map(|v| v.to_le_bytes()));
        }
        let sums = r.test_film_footprint(&source, &targets, grid);
        let rows: Vec<_> = pixels
            .iter()
            .zip(sums.chunks_exact(32))
            .enumerate()
            .map(|(index, (&[x, y, expected], s))| {
                let w: Vec<u32> = s
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                serde_json::json!({"pixel":[x,y],"expected":expected,"samples":grid*grid,"micro":micro[index],
                    "accepted":w[0],"visible_rays":w[1],"rays":32*u64::from(w[0]),
                    "visibility":f64::from(w[1])/(32.0*f64::from(w[0]).max(1.0)),
                    "opposite_samples":w[2],"corner":w[3],"edge":w[4],"interior":w[5],
                    "nearest_to_corner_px":if w[2]>0 {f64::from(w[6])*1e-6} else {-1.0},"rejected":w[7]})
            })
            .collect();
        results.insert(pose.clone(), serde_json::json!(rows));
    }
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("footprint.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"grid":grid,"rays_per_sample":32,
            "seconds":start.elapsed().as_secs_f64(),"poses":results}))
        .unwrap(),
    )
    .unwrap();
}

/// T1 static accumulation on the R1 fixtures (manifest `cases`: name, workload and, for flat
/// and convex, distance and incidence), `frames` frames each. Writes T1's own samples'
/// centre-surface visibility in R1's film-sum layout (halves by frame parity) with the centre
/// receivers and fixture geometry, for scripts/m6-temporal-fixtures.py's oracles.
#[test]
#[ignore = "owned GPU lease; T1 on the thin, stress and acne fixtures, offscreen"]
fn gpu_film_t1_fixtures() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let frames = m["frames"].as_u64().unwrap_or(256) as u32;
    let root = PathBuf::from(m["output"].as_str().unwrap());
    let color = target(&r, 256, 192);
    for case in m["cases"].as_array().unwrap() {
        let out = root.join(case["name"].as_str().unwrap());
        fs::create_dir_all(&out).unwrap();
        let workload = case["workload"].as_str().unwrap();
        let rect = matches!(workload, "thin" | "stress");
        let (mesh, mut ground, mut blockers, camera, geometry) = if rect {
            let (mesh, ground, blockers, camera, rects) = rect_scene(&mut r, workload == "stress");
            fs::write(
                out.join("rectangles.json"),
                serde_json::to_vec_pretty(&rects).unwrap(),
            )
            .unwrap();
            (mesh, ground, Some(blockers), camera, None)
        } else {
            let (mesh, instances, camera, geometry) =
                receiver_policy_tests::analytic_scene(&mut r, case);
            (mesh, instances, None, camera, Some(geometry))
        };
        if let Some(geometry) = geometry {
            fs::write(
                out.join("geometry.json"),
                serde_json::to_vec_pretty(&geometry).unwrap(),
            )
            .unwrap();
        }
        let mut frame = |r: &mut Renderer| match &mut blockers {
            Some(blockers) => render(
                r,
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut ground), Draw::new(&mesh, blockers)],
            ),
            None => render(r, &color, &camera, &mut [Draw::new(&mesh, &mut ground)]),
        };
        // The centre frame defines the centre surface, as for the R1 references.
        r.set_soft_shadows(crate::SoftShadows::Exact);
        frame(&mut r);
        r.test_drain("T1 fixture centre");
        let [world, normal, counts] = receivers(&r);
        let hit = hit_record(&r);
        for (suffix, data) in [
            ("world.f32", &world),
            ("normal.f32", &normal),
            ("exact.u32", &counts),
            ("hit.u32", &hit),
        ] {
            fs::write(out.join(format!("pose-0.{suffix}")), data).unwrap();
        }
        let inverse = camera.view().inverse();
        let (eye, forward) = (inverse.w_axis.truncate(), -inverse.z_axis.truncate());
        let centre: Vec<u8> = world
            .chunks_exact(16)
            .zip(hit.chunks_exact(16))
            .flat_map(|(w, h)| {
                let p = Vec3::from_array(std::array::from_fn(|i| {
                    f32::from_le_bytes(w[i * 4..i * 4 + 4].try_into().unwrap())
                }));
                h[..4]
                    .iter()
                    .copied()
                    .chain((p - eye).dot(forward).to_le_bytes())
                    .collect::<Vec<_>>()
            })
            .collect();
        let conditioned = ConditionedVisibility::new(&r, &centre, forward);
        r.set_soft_shadows(crate::SoftShadows::Temporal);
        for n in 1..=frames {
            frame(&mut r);
            conditioned.accumulate(&r, (n % 2) as usize);
        }
        r.test_drain("T1 fixture frames");
        let halves = conditioned
            .result
            .each_ref()
            .map(|b| read_buffer(&mut r, b, b.size()));
        // R1's film layout: counts [visible 0, visible 1, accepted 0, accepted 1], then zero
        // moments and radiance.
        let mut film = Vec::with_capacity(halves[0].len() / 8 * 48);
        for (a, b) in halves[0].chunks_exact(8).zip(halves[1].chunks_exact(8)) {
            for word in [&a[..4], &b[..4], &a[4..], &b[4..]] {
                film.extend_from_slice(word);
            }
            film.extend([0u8; 32]);
        }
        fs::write(out.join("pose-0.film.bin"), film).unwrap();
        fs::write(out.join("pose-0.json"), serde_json::to_vec_pretty(&serde_json::json!({"pose":0,
            "size":[256,192],"samples":frames,"frames":frames,"rays_per_sample":8,
            "estimator":"T1 temporal film accumulation, its own samples on the centre surface; halves by frame parity",
            "eye":eye.extend(1.0).to_array(),"forward":forward.extend(0.0).to_array(),
            "receiver_policy":"Production"})).unwrap()).unwrap();
        r.set_soft_shadows(crate::SoftShadows::Exact);
        eprintln!("T1 fixture {} done", case["name"]);
    }
}

/// The manifest's `records` (4 or 8; absent: no records).
fn film_motion(m: &serde_json::Value) -> crate::FilmMotion {
    match m["records"].as_u64() {
        None => crate::FilmMotion::Restart,
        Some(4) => crate::FilmMotion::Records4,
        Some(8) => crate::FilmMotion::Records8,
        Some(k) => panic!("records must be 4 or 8, not {k}"),
    }
}

/// Motion shading of `program` with K records (film_records.rs), misses black when `black`.
fn record_shading(
    r: &Renderer,
    program: &str,
    black: bool,
    count: u32,
) -> Owned<wgpu::ComputePipeline> {
    let head = film_accumulation::shading_head(&primary_ray_gpu::shading_heads(SOURCE).unwrap()[1])
        .unwrap();
    let mut source = film_records::compose_shading(&head, program, false).source;
    if black {
        source = black_clear(&source);
    }
    let scopes = OperationScopes::new(&r.device);
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test motion shading"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = r
        .primary
        .as_ref()
        .unwrap()
        .record_shade_pipeline(&r.device, &r.ledger, &module, count);
    assert!(
        scopes.finish().is_none(),
        "motion shading failed to compile"
    );
    pipeline
}

/// Every layer of `texture`, tightly packed.
fn read_layers(r: &mut Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    read_window(r, texture, [0, 0, size.width, size.height])
}

/// The `[x0, y0, width, height]` window of every layer of `texture`, tightly packed (layer,
/// row, column).
fn read_window(r: &mut Renderer, texture: &wgpu::Texture, [x0, y0, w, h]: [u32; 4]) -> Vec<u8> {
    let layers = texture.size().depth_or_array_layers;
    let tight = w * texture.format().block_copy_size(None).unwrap();
    let row = tight.div_ceil(256) * 256;
    let bytes = u64::from(row) * u64::from(h) * u64::from(layers);
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("T2 record readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: layers,
        },
    );
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
        callback.set(result).unwrap();
    });
    r.queue.submit([encoder.finish()]);
    r.test_drain("T2 record readback");
    done.get().unwrap().as_ref().unwrap();
    let view = buffer.get_mapped_range(..).unwrap();
    let out = view
        .chunks_exact(row as usize)
        .flat_map(|r| r[..tight as usize].iter().copied())
        .collect();
    drop(view);
    buffer.unmap();
    out
}

/// SHA-256 of the latest records: fresh and moved records and the fresh planes.
fn record_digest(r: &mut Renderer) -> String {
    let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
    let records = film.records.as_ref().unwrap();
    let (fresh, moved, _) = records.test_resources(records.side);
    let (fresh, moved) = (fresh.clone(), moved.clone());
    let planes = records.test_planes().clone();
    let mut hash = Sha256::new();
    hash.update(read_layers(r, &fresh));
    hash.update(read_layers(r, &moved));
    let size = planes.size();
    hash.update(read_buffer(r, &planes, size));
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The last moving frame's per-pixel record diagnostics (film_records_kernel.wgsl).
fn record_diagnostics(r: &mut Renderer) -> Vec<[u32; 2]> {
    let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
    let records = film.records.as_ref().unwrap();
    let buffer = records.test_resources(0).2.clone();
    // The record probes follow the per-pixel words.
    let size = buffer.size() - film_records::PROBE_BYTES;
    read_buffer(r, &buffer, size)
        .chunks_exact(8)
        .map(|w| [0, 1].map(|i| u32::from_le_bytes(w[i * 4..i * 4 + 4].try_into().unwrap())))
        .collect()
}

/// Installs a scored program's production Soft, T1a temporal and record motion shading, and
/// black misses for the record passes when `black`. Calling it again with the same pipelines
/// restores the previous ones.
struct ScoredProgram {
    centre: [Owned<wgpu::ComputePipeline>; 2],
    temporal: Owned<wgpu::ComputePipeline>,
    motion: Option<Owned<wgpu::ComputePipeline>>,
    black: bool,
}
impl ScoredProgram {
    fn new(r: &Renderer, program: &str, black: bool, count: u32) -> Self {
        Self {
            centre: centre_shading(r, program, black),
            temporal: temporal_shading(r, program, black),
            motion: Some(record_shading(r, program, black, count)),
            black,
        }
    }
    fn swap(&mut self, r: &mut Renderer) {
        std::mem::swap(r.test_standard().traced.as_mut().unwrap(), &mut self.centre);
        std::mem::swap(
            r.test_standard().temporal.as_mut().unwrap(),
            &mut self.temporal,
        );
        let state = r.primary.as_mut().unwrap();
        std::mem::swap(&mut state.test_record_shading, &mut self.motion);
        state.test_film_background = match state.test_film_background {
            None if self.black => Some([0.0; 3]),
            _ => None,
        };
    }
}

/// T2a's static regression: Q-static (`frames` frames at each manifest pose from cold) for each
/// scored program (or those named in `programs`), once per entry of `modes` ("restart": T1a;
/// "records": `records`; default both). Every run's HDR images and final T1a history must equal
/// the first run's byte for byte. With `detail`, each frame's hit records and counts are also
/// compared, and a differing image is compared pixel by pixel (the first run's images are kept
/// on disk under `output` meanwhile).
#[test]
#[ignore = "owned GPU lease; T2a static regression against T1a, offscreen"]
fn gpu_film_t2_static() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let cameras = t1_cameras(&m);
    let frames = m["frames"].as_u64().unwrap_or(128) as u32;
    let motion = film_motion(&m);
    let count = motion.records();
    assert!(count > 0, "the static regression compares records with T1a");
    let modes: Vec<(String, crate::FilmMotion)> = m["modes"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_else(|| vec!["restart".into(), "records".into()])
        .into_iter()
        .map(|name| {
            let mode = match name.as_str() {
                "restart" => crate::FilmMotion::Restart,
                "records" => motion,
                other => panic!("unknown mode {other}"),
            };
            (name, mode)
        })
        .collect();
    let only: Option<Vec<String>> = m["programs"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect());
    let detail = m["detail"].as_bool().unwrap_or(false);
    let baseline = m["baseline"].as_u64().unwrap_or(0) as usize;
    assert!(baseline < modes.len());
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    // Compile the temporal and record pipelines once, then swap scored programs into them.
    r.set_soft_shadows(crate::SoftShadows::Temporal);
    r.set_film_motion(motion);
    for camera in [&cameras[poses(&m)[0]], &cameras[poses(&m)[0]]] {
        traced_frame(&mut r, &color, camera, &mut asset, true);
    }
    let mut results = vec![];
    for pose in poses(&m) {
        let camera = &cameras[pose];
        for (name, program, black) in programs() {
            if only.as_ref().is_some_and(|o| !o.iter().any(|p| p == name)) {
                continue;
            }
            let mut scored = ScoredProgram::new(&r, &program, black, count);
            scored.swap(&mut r);
            let mut runs: Vec<(Vec<String>, Vec<String>, String)> = vec![];
            let mut mismatches = vec![];
            for (run, (mode_name, mode)) in modes.iter().enumerate() {
                r.set_film_motion(*mode);
                reset_film(&mut r);
                let (mut images, mut traces) = (vec![], vec![]);
                for n in 1..=frames {
                    traced_frame(&mut r, &color, camera, &mut asset, true);
                    let hdr = settled(&mut r);
                    images.push(digest(&hdr));
                    if detail {
                        traces.push(digest(&interactive_outputs(&r).concat()));
                        let kept = out.join(format!("first-p{pose}-{name}-n{n:03}.rgba16f"));
                        if run == 0 {
                            fs::write(&kept, &hdr).unwrap();
                        } else if images.last() != runs[0].0.get(n as usize - 1) {
                            // Pixel-level comparison with the first run's image of this frame.
                            let first = fs::read(&kept).unwrap();
                            let (mut pixels, mut largest) = (0usize, 0f32);
                            for (a, b) in first.chunks_exact(8).zip(hdr.chunks_exact(8)) {
                                if a != b {
                                    pixels += 1;
                                    for c in 0..3 {
                                        let v = |x: &[u8]| {
                                            decode_f16(u16::from_le_bytes([x[c * 2], x[c * 2 + 1]]))
                                        };
                                        largest = largest.max((v(a) - v(b)).abs());
                                    }
                                }
                            }
                            mismatches.push(
                                serde_json::json!({"run":run,"mode":mode_name,"frame":n,
                                "trace_identical":traces.last() == runs[0].1.get(n as usize - 1),
                                "differing_pixels":pixels,"largest_difference":largest}),
                            );
                        }
                    }
                }
                r.test_drain("T2 static history");
                runs.push((images, traces, digest(&history(&r))));
            }
            scored.swap(&mut r);
            if detail {
                for n in 1..=frames {
                    let _ =
                        fs::remove_file(out.join(format!("first-p{pose}-{name}-n{n:03}.rgba16f")));
                }
            }
            // Every run against the `baseline` run (default the first): a later baseline compares
            // records with T1a after both pipelines' first use. `mismatches` are against the first.
            let base = &runs[baseline];
            for (run, (images, _, history)) in
                runs.iter().enumerate().filter(|(i, _)| *i != baseline)
            {
                let identical = base.0.iter().zip(images).filter(|(a, b)| a == b).count();
                let row = serde_json::json!({"pose":pose,"program":name,"frames":frames,
                    "compared":[modes[baseline].0, modes[run].0],"baseline":baseline,"run":run,
                    "frames_identical":identical,"history_identical":base.2 == *history,
                    "first_differing_frame":base.0.iter().zip(images).position(|(a, b)| a != b).map(|i| i + 1),
                    "mismatches_against_first_run":mismatches.iter().filter(|x| x["run"] == run).take(20).collect::<Vec<_>>()});
                eprintln!("T2 static {row}");
                results.push(row);
            }
        }
    }
    fs::write(
        out.join("static.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"records":count,"modes":modes.iter().map(|m| &m.0).collect::<Vec<_>>(),
            "results":results}))
        .unwrap(),
    )
    .unwrap();
    for row in &results {
        assert!(
            row["frames_identical"] == row["frames"] && row["history_identical"] == true,
            "static output differs between runs: {row}"
        );
    }
}

/// T2a camera-motion data (docs/m6-motion-aa.md section 7): the registered `sequence` from
/// cold with `records` per pixel and each scored program, the film captured at the `scored`
/// frames. The full program's first run also records per frame the record diagnostics, writes
/// each scored frame's ages (frames since the pixel last kept no record; static frames age
/// every pixel) and flags, and checks T-0 at the `t0_frames`: every pixel without a kept record
/// against a production Interactive frame at the same camera, by HDR, hit record and counts.
/// `replays` reruns the full program for F8, the first with diagnostics and the rest without,
/// each compared with the first run's scored images and final records.
#[test]
#[ignore = "owned GPU lease; T2a camera motion data, offscreen"]
fn gpu_film_t2_motion() {
    let m = manifest();
    let _validation = validation::begin(m["validate"].as_bool().unwrap_or(true));
    let mut r = renderer(m["validate"].as_bool().unwrap_or(true));
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let [w, h] = [1920u32, 1080];
    let color = target(&r, w, h);
    let path: Vec<Camera> = cameras::Cameras::load(&PathBuf::from(m["sequence"].as_str().unwrap()))
        .unwrap()
        .frames
        .iter()
        .map(|f| f.camera().unwrap())
        .collect();
    let list = |key: &str| -> Vec<usize> {
        m[key]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_u64().unwrap() as usize).collect())
            .unwrap_or_default()
    };
    let scored = list("scored");
    let t0_frames = list("t0_frames");
    let replays = m["replays"].as_u64().unwrap_or(1) as usize;
    let motion = film_motion(&m);
    let count = motion.records();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Temporal);
    r.set_film_motion(motion);
    for camera in &path[..2] {
        traced_frame(&mut r, &color, camera, &mut asset, true);
    }
    let pixels = (w * h) as usize;
    let mut frames = vec![];
    let mut identity = vec![];
    let mut empty_identity = vec![];
    let mut digests = serde_json::Map::new();
    let start = Instant::now();
    for (name, program, black) in programs() {
        let mut program = ScoredProgram::new(&r, &program, black, count);
        program.swap(&mut r);
        r.set_soft_shadows(crate::SoftShadows::Interactive);
        traced_frame(&mut r, &color, &path[0], &mut asset, true);
        let interactive = settled(&mut r);
        r.set_soft_shadows(crate::SoftShadows::Temporal);
        let runs = if name == "full" { 1 + replays } else { 1 };
        let mut replay: Vec<Vec<String>> = vec![];
        for run in 0..runs {
            let record = name == "full" && run == 0;
            r.primary.as_mut().unwrap().test_record_diagnostics = run <= 1 && name == "full";
            reset_film(&mut r);
            let mut ages = vec![0u8; pixels];
            let mut run_digests = vec![];
            for (f, camera) in path.iter().enumerate() {
                traced_frame(&mut r, &color, camera, &mut asset, true);
                let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
                let (reset, moving) = (film.reset, film.moving);
                let seed = film.records.as_ref().unwrap().seed;
                assert_eq!(
                    reset,
                    f == 0,
                    "frame {f} of {name}: only the first frame resets"
                );
                let t0 = record && moving && t0_frames.contains(&f);
                // The frame's image, read before any T-0 reference frame overwrites it.
                let hdr = (f == 0 || t0 || scored.contains(&f)).then(|| settled(&mut r));
                if f == 0 {
                    let same = hdr.as_ref() == Some(&interactive);
                    identity
                        .push(serde_json::json!({"program":name,"run":run,"hdr_identical":same}));
                    assert!(same, "T-0 {name} run {run}");
                }
                let mut flags = vec![];
                if record {
                    let diagnostics = if moving {
                        record_diagnostics(&mut r)
                    } else {
                        vec![]
                    };
                    let mut stats = [0u64; 12];
                    let mut kept_histogram = [0u64; 9];
                    for (i, age) in ages.iter_mut().enumerate() {
                        let Some(&[x, y]) = diagnostics.get(i) else {
                            *age = if f == 0 { 0 } else { age.saturating_add(1) };
                            continue;
                        };
                        let kept = x & 0xf;
                        let hidden = (x >> 16) & 0xff;
                        *age = if kept == 0 { 0 } else { age.saturating_add(1) };
                        kept_histogram[kept as usize] += 1;
                        for (s, v) in stats.iter_mut().zip([
                            u64::from((x >> 6) & 1),
                            u64::from(kept == 0),
                            u64::from((x >> 4) & 1),
                            u64::from((x >> 5) & 1),
                            u64::from((x >> 8) & 0xff),
                            u64::from(kept),
                            u64::from(hidden),
                            u64::from(hidden > 0),
                            u64::from(x >> 24),
                            u64::from((x >> 7) & 1),
                            u64::from(y & 1),
                            u64::from(y >> 8),
                        ]) {
                            *s += v;
                        }
                    }
                    if moving {
                        flags = diagnostics
                            .iter()
                            .map(|&[x, y]| {
                                ((x & 0xff)
                                    | ((y & 1) << 8)
                                    | (u32::from((x >> 16) & 0xff > 0) << 9))
                                    as u16
                            })
                            .collect();
                    }
                    let keys = [
                        "explicit_pixels",
                        "empty_pixels",
                        "cold_pixels",
                        "discontinuity_pixels",
                        "landed",
                        "kept",
                        "hidden_records",
                        "hidden_pixels",
                        "validation_rays",
                        "kept_band_pixels",
                        "own_band_pixels",
                        "kept_band_records",
                    ];
                    let mut row = serde_json::json!({"frame":f,"reset":reset,"moving":moving,"seed":seed,
                        "kept_histogram":kept_histogram,"matrix":camera.matrix(w, h).to_cols_array(),
                        "eye":camera.view().inverse().w_axis.truncate().to_array()});
                    for (k, v) in keys.iter().zip(stats) {
                        row[*k] = serde_json::json!(v);
                    }
                    // T-0: pixels without a kept record traced their sample at the centre with
                    // Interactive's rays; their image, hit record and counts are Interactive's.
                    if t0 {
                        let hdr = hdr.as_ref().unwrap();
                        let outputs = interactive_outputs(&r);
                        r.soft_mode = crate::SoftShadows::Interactive;
                        traced_frame(&mut r, &color, camera, &mut asset, true);
                        let reference = settled(&mut r);
                        let reference_outputs = interactive_outputs(&r);
                        r.soft_mode = crate::SoftShadows::Temporal;
                        let mut differ = [0usize; 3];
                        let mut empty = 0usize;
                        for (i, &[x, _]) in diagnostics.iter().enumerate() {
                            if x & 0xf != 0 {
                                continue;
                            }
                            empty += 1;
                            differ[0] +=
                                usize::from(hdr[i * 8..i * 8 + 8] != reference[i * 8..i * 8 + 8]);
                            differ[1] += usize::from(
                                outputs[0][i * 16..i * 16 + 16]
                                    != reference_outputs[0][i * 16..i * 16 + 16],
                            );
                            differ[2] += usize::from(
                                outputs[1][i * 4..i * 4 + 4]
                                    != reference_outputs[1][i * 4..i * 4 + 4],
                            );
                        }
                        let check = serde_json::json!({"frame":f,"empty_pixels":empty,
                            "hdr_differing":differ[0],"hit_differing":differ[1],"counts_differing":differ[2]});
                        eprintln!("T2 T-0 {check}");
                        empty_identity.push(check);
                    }
                    frames.push(row);
                }
                if scored.contains(&f) {
                    let hdr = hdr.unwrap();
                    run_digests.push(digest(&hdr));
                    if run == 0 {
                        fs::write(out.join(format!("{name}-f{f:03}.rgba16f")), hdr).unwrap();
                        if record {
                            fs::write(out.join(format!("age-f{f:03}.u8")), &ages).unwrap();
                            if !flags.is_empty() {
                                let bytes: Vec<u8> =
                                    flags.iter().flat_map(|v| v.to_le_bytes()).collect();
                                fs::write(out.join(format!("flags-f{f:03}.u16")), bytes).unwrap();
                            }
                        }
                    }
                }
            }
            run_digests.push(record_digest(&mut r));
            if name == "full" {
                eprintln!(
                    "T2 motion run {run} of {runs} {:.1} s",
                    start.elapsed().as_secs_f64()
                );
            }
            replay.push(run_digests);
        }
        r.primary.as_mut().unwrap().test_record_diagnostics = false;
        program.swap(&mut r);
        digests.insert(name.to_owned(), serde_json::json!(replay));
        eprintln!(
            "T2 motion program={name} done {:.1} s",
            start.elapsed().as_secs_f64()
        );
    }
    let replay = digests["full"].as_array().unwrap();
    let identical = replay
        .iter()
        .skip(1)
        .filter(|run| *run == &replay[0])
        .count();
    // The first run's freshly created pipelines can round differently from later runs
    // (docs/m6-motion-aa-gpu.md section 3), so replays are also compared with the first replay,
    // and each run lists the digest indices (scored frames, then records) where it differs.
    let identical_to_replay = replay
        .iter()
        .skip(2)
        .filter(|run| replay.get(1) == Some(run))
        .count();
    let differing: Vec<_> = replay
        .iter()
        .enumerate()
        .skip(1)
        .map(|(run, digests)| {
            let at: Vec<usize> = digests
                .as_array()
                .unwrap()
                .iter()
                .zip(replay[0].as_array().unwrap())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, _)| i)
                .collect();
            serde_json::json!({"run":run,"differs_from_first_at":at})
        })
        .collect();
    let t0_passed = empty_identity
        .iter()
        .all(|c| c["hdr_differing"] == 0 && c["hit_differing"] == 0 && c["counts_differing"] == 0);
    fs::write(
        out.join("motion.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"sequence":m["sequence"],"records":count,
            "scored":scored,"frames":frames,"identity":identity,"empty_identity":empty_identity,
            "t0_empty_passed":t0_passed,"digests":digests,"replays":replays,
            "replays_identical":identical,"replays_identical_to_first_replay":identical_to_replay,
            "replay_differences":differing,"seconds":start.elapsed().as_secs_f64()}),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(identical, replays, "T2 replays differ");
}

/// The probed validation kernel (support/film_t2_probe.wgsl): the production record kernels
/// with every landed record's acceptance-test values written at the probe pixels.
const PROBE_EDITS: [(&str, &str); 4] = [
    (
        "    let discontinuity=(geometry>0u && background>0u) || far-near>4.0*film_motion.eye.w*near;\n",
        "    let discontinuity=(geometry>0u && background>0u) || far-near>4.0*film_motion.eye.w*near;\n    let probe=film_probe_slot(p);\n    if probe>=0 {film_probe_header(u32(probe),p,&planes,geometry,background,near,far,discontinuity);}\n",
    ),
    (
        "                if !valid {continue;}\n",
        "                if probe>=0 {film_probe_entry(u32(probe),s,slot,listed,r,moved,valid,&planes,discontinuity,v.count);}\n                if !valid {continue;}\n",
    ),
    (
        "    for(var k=0u;k<v.count;k+=1u) {geometric=geometric || film_placement(v.kept[k])!=0u;}\n",
        "    if probe>=0 {film_probe_final(u32(probe),&v);}\n    for(var k=0u;k<v.count;k+=1u) {geometric=geometric || film_placement(v.kept[k])!=0u;}\n",
    ),
    (
        "// ---- end of the T2 kernel passes ----",
        concat!(
            include_str!("../support/film_t2_probe.wgsl"),
            "// ---- end of the T2 kernel passes ----"
        ),
    ),
];

#[test]
fn t2_probe_and_scatter_kernels_compile() {
    let kernels = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let kernel = film_records::kernel_source(&kernels[1]).unwrap();
    let mut probed = kernel.clone();
    for e in PROBE_EDITS {
        probed = edit(&probed, e);
    }
    validate_wgsl(&probed);
    validate_wgsl(&format!(
        "{kernel}\n{}",
        include_str!("../support/film_t2_scatter.wgsl")
    ));
}

/// A registered cameras file's frames.
fn sequence(file: &str) -> Vec<Camera> {
    cameras::Cameras::load(&PathBuf::from(file))
        .unwrap()
        .frames
        .iter()
        .map(|f| f.camera().unwrap())
        .collect()
}

/// T2a's per-record probe (support/film_t2_probe.wgsl) for one `spec`: `path` from cold with
/// `records` per pixel and the visibility program (or `program`), test diagnostics on and
/// PROBE_EDITS' validation kernel, restored to production afterwards. Under `output`:
/// - at each frame of `probes` ({"frame": [[x, y], ...]}, at most three pixels),
///   `probe-f<frame>.bin`, the three probe slots;
/// - at every frame of each `windows` entry ([x, y, first, last]), `f<frame>-w<index>.bin`, the
///   `size` x `size` window around (x, y) of the arrays `probe.json` lists, row-major (moved
///   records layer-major).
fn probe_records(
    r: &mut Renderer,
    color: &wgpu::Texture,
    asset: &mut prepared::Prepared,
    path: &[Camera],
    spec: &serde_json::Value,
) {
    let [w, h] = [1920u32, 1080];
    let motion = film_motion(spec);
    let count = motion.records();
    let out = PathBuf::from(spec["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let numbers = |v: &serde_json::Value| -> Vec<u32> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as u32)
            .collect()
    };
    let probes: std::collections::BTreeMap<usize, [u32; 3]> = spec["probes"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(f, pixels)| {
                    let mut words = [0u32; 3];
                    for (word, p) in words.iter_mut().zip(pixels.as_array().unwrap()) {
                        let p = numbers(p);
                        *word = (p[0] + 1) | (p[1] << 16);
                    }
                    (f.parse().unwrap(), words)
                })
                .collect()
        })
        .unwrap_or_default();
    let windows: Vec<Vec<u32>> = spec["windows"]
        .as_array()
        .map(|a| a.iter().map(numbers).collect())
        .unwrap_or_default();
    let size = spec["size"].as_u64().unwrap_or(48) as u32;
    let last = probes
        .keys()
        .copied()
        .chain(windows.iter().map(|w| w[3] as usize))
        .max()
        .unwrap();
    let program_name = spec["program"].as_str().unwrap_or("visibility");
    let (_, program, black) = programs()
        .into_iter()
        .find(|p| p.0 == program_name)
        .unwrap();
    r.set_soft_shadows(crate::SoftShadows::Temporal);
    r.set_film_motion(motion);
    for camera in &path[..2] {
        traced_frame(r, color, camera, asset, true);
    }
    let edits: Vec<(String, String)> = PROBE_EDITS
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    r.test_record_kernel_edits(&edits);
    let mut scored = ScoredProgram::new(r, &program, black, count);
    scored.swap(r);
    r.primary.as_mut().unwrap().test_record_diagnostics = true;
    reset_film(r);
    let pixels = (w * h) as usize;
    let crop = |data: &[u8], bytes: usize, [x0, y0, cw, ch]: [u32; 4]| -> Vec<u8> {
        (y0..y0 + ch)
            .flat_map(|y| {
                let start = (y * w + x0) as usize * bytes;
                data[start..start + cw as usize * bytes].to_vec()
            })
            .collect()
    };
    let mut rows = vec![];
    for (f, camera) in path.iter().enumerate().take(last + 1) {
        r.primary.as_mut().unwrap().test_record_probe = probes.get(&f).copied().unwrap_or([0; 3]);
        traced_frame(r, color, camera, asset, true);
        let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
        let (reset, moving, parity) = (film.reset, film.moving, film.parity);
        let records = film.records.as_ref().unwrap();
        let seed = records.seed;
        let (fresh, moved, diagnostics) = records.test_resources(records.side);
        let (fresh, moved, diagnostics) = (fresh.clone(), moved.clone(), diagnostics.clone());
        let planes = records.test_planes().clone();
        let [colour, anchor, _] = film.history(parity).map(|t| t.clone());
        let active: Vec<usize> = (0..windows.len())
            .filter(|&i| (windows[i][2] as usize..=windows[i][3] as usize).contains(&f))
            .collect();
        if !active.is_empty() || probes.contains_key(&f) {
            let hdr = settled(r);
            let diagnostics = read_buffer(r, &diagnostics, diagnostics.size());
            if probes.contains_key(&f) {
                fs::write(
                    out.join(format!("probe-f{f:03}.bin")),
                    &diagnostics[8 * pixels..],
                )
                .unwrap();
            }
            let primary = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
            let (hit, tiles) = (primary.hit.texture.clone(), primary.tiles.texture.clone());
            let fresh_targets = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
            let visibility = fresh_targets.visibility.texture.clone();
            let planes = read_buffer(r, &planes, planes.size());
            let tiles = read_texture(r, &tiles);
            for i in active {
                let [x, y] = [windows[i][0], windows[i][1]];
                let x0 = x.saturating_sub(size / 2).min(w - size);
                let y0 = y.saturating_sub(size / 2).min(h - size);
                let window = [x0, y0, size, size];
                // Per-pixel coverage of the 8 x 4 trace tiles.
                let covered: Vec<u8> = (y0..y0 + size)
                    .flat_map(|py| {
                        let tiles = &tiles;
                        (x0..x0 + size).flat_map(move |px| {
                            let t = ((py / 4) * w.div_ceil(8) + px / 8) as usize * 4;
                            tiles[t..t + 4].to_vec()
                        })
                    })
                    .collect();
                let mut bytes = crop(&hdr, 8, window);
                bytes.extend(read_window(r, &hit, window));
                bytes.extend(read_window(r, &visibility, window));
                bytes.extend(covered);
                bytes.extend(crop(&planes, 16, window));
                bytes.extend(crop(&diagnostics[..8 * pixels], 8, window));
                bytes.extend(read_window(r, &fresh, window));
                bytes.extend(read_window(r, &moved, window));
                bytes.extend(read_window(r, &colour, window));
                bytes.extend(read_window(r, &anchor, window));
                fs::write(out.join(format!("f{f:03}-w{i}.bin")), bytes).unwrap();
            }
        }
        rows.push(serde_json::json!({"frame":f,"reset":reset,"moving":moving,"seed":seed,
            "matrix":camera.matrix(w, h).to_cols_array(),
            "infinite":camera.infinite_matrix(w, h).to_cols_array(),
            "eye":camera.view().inverse().w_axis.truncate().to_array()}));
    }
    scored.swap(r);
    let state = r.primary.as_mut().unwrap();
    state.test_record_diagnostics = false;
    state.test_record_probe = [0; 3];
    r.test_record_kernel_edits(&[]);
    fs::write(
        out.join("probe.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"records":count,
            "program":program_name,"probes":spec["probes"],"windows":windows,"size":size,
            "window_arrays":[["hdr","rgba16f",8],["hit","rgba32uint",16],["counts","r32uint",4],
                ["covered","r32uint",4],["planes","4xu32",16],["diagnostics","2xu32",8],
                ["fresh","rgba32uint",16],["moved","rgba32uint",16*(count-1)],
                ["colour","rgba32float",16],["anchor","rg32uint",8]],
            "frames":rows}))
        .unwrap(),
    )
    .unwrap();
}

/// The scatter variant's identity check (support/film_t2_scatter.wgsl) at the current K: the
/// first `frames` of `path` from cold with the full program, run with the gathers, the
/// scatter, the scatter and the gathers. Every frame's HDR image and the final records (fresh,
/// moved and planes) are compared by hash with the first run's.
fn scatter_identity(
    r: &mut Renderer,
    color: &wgpu::Texture,
    asset: &mut prepared::Prepared,
    path: &[Camera],
    frames: usize,
) -> Vec<serde_json::Value> {
    r.test_record_scatter(&[]);
    let mut runs: Vec<(bool, Vec<String>, String)> = vec![];
    for scattered in [false, true, true, false] {
        r.primary.as_mut().unwrap().test_scatter_active = scattered;
        reset_film(r);
        let images = path
            .iter()
            .take(frames)
            .map(|camera| {
                traced_frame(r, color, camera, asset, true);
                digest(&settled(r))
            })
            .collect();
        runs.push((scattered, images, record_digest(r)));
    }
    r.primary.as_mut().unwrap().test_scatter_active = false;
    let first = &runs[0];
    runs.iter()
        .map(|(scattered, images, records)| {
            serde_json::json!({"scatter":scattered,"frames":images.len(),
                "frames_identical":first.1.iter().zip(images).filter(|(a, b)| a == b).count(),
                "first_differing_frame":first.1.iter().zip(images).position(|(a, b)| a != b),
                "records_identical":first.2 == *records})
        })
        .collect()
}

/// T2's follow-up checks in one process, the quarry loaded once (AGENTS.md: batched checks):
/// - `probes`: probe_records per spec (each with its own `records` and `output`);
/// - `identity`: {"records": [K...], "frames": n}, scatter_identity per K;
/// - `timing`: {"warmup", "pairs", "runs": [{"label", "records", "scatter", "scatter_edits"}]},
///   paired_timing per run along `sequence`.
///
/// Validation is off. Writes `batch.json` (identity rows and timing samples) under `output`,
/// then asserts every identity run matched.
#[test]
#[ignore = "owned GPU lease; T2 batched probes, scatter identity and timing, offscreen"]
fn gpu_film_t2_batch() {
    let m = manifest();
    let _validation = validation::begin(false);
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let path = sequence(m["sequence"].as_str().unwrap());
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let start = Instant::now();
    let stage = |name: &str| eprintln!("T2 batch {name} {:.1} s", start.elapsed().as_secs_f64());
    for spec in m["probes"].as_array().into_iter().flatten() {
        probe_records(&mut r, &color, &mut asset, &path, spec);
        stage(&format!("probe K={}", spec["records"]));
    }
    // Compiles K's production record pipelines (two frames: reset, then moving).
    let use_records = |r: &mut Renderer, asset: &mut prepared::Prepared, k: &serde_json::Value| {
        r.set_soft_shadows(crate::SoftShadows::Temporal);
        r.set_film_motion(film_motion(&serde_json::json!({"records":k})));
        for camera in &path[..2] {
            traced_frame(r, &color, camera, asset, true);
        }
    };
    let mut identity = vec![];
    let frames = m["identity"]["frames"].as_u64().unwrap_or(64) as usize;
    for k in m["identity"]["records"].as_array().into_iter().flatten() {
        use_records(&mut r, &mut asset, k);
        let runs = scatter_identity(&mut r, &color, &mut asset, &path, frames);
        eprintln!("T2 batch identity K={k} {}", serde_json::json!(runs));
        identity.push(serde_json::json!({"records":k,"runs":runs}));
        stage(&format!("identity K={k}"));
    }
    let mut timing = vec![];
    let t = &m["timing"];
    let counts = [
        t["warmup"].as_u64().unwrap_or(8) as usize,
        t["pairs"].as_u64().unwrap_or(48) as usize,
    ];
    for run in t["runs"].as_array().into_iter().flatten() {
        use_records(&mut r, &mut asset, &run["records"]);
        let scatter = run["scatter"].as_str();
        if scatter.is_some() {
            r.test_record_scatter(&string_pairs(&run["scatter_edits"]));
        }
        let samples = paired_timing(&mut r, &color, &mut asset, &path, counts, scatter);
        timing.push(serde_json::json!({"label":run["label"],"records":run["records"],
            "scatter":scatter,"scatter_edits":run["scatter_edits"],"warmup_pairs":counts[0],
            "pairs":counts[1],"sequence":m["sequence"],"samples":samples}));
        stage(&format!("timing {}", run["label"]));
    }
    fs::write(
        out.join("batch.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"sequence":m["sequence"],
            "identity":identity,"timing":timing,"seconds":start.elapsed().as_secs_f64()}))
        .unwrap(),
    )
    .unwrap();
    for row in &identity {
        for run in row["runs"].as_array().unwrap() {
            assert!(
                run["frames_identical"] == run["frames"] && run["records_identical"] == true,
                "scatter identity failed: {row}"
            );
        }
    }
}


