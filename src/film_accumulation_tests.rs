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
//! Manifest: the quarry fields of `load_quarry`, `output`, `t1_cameras` (a cameras file
//! whose frames are the scored poses), `poses`, and for the temporal test `frames`,
//! `checkpoints`, `repeats` and `cut` ([from, to] pose indices).
use super::*;
use primary_ray_gpu::film_accumulation;

#[path = "t1a_integration_tests.rs"]
mod integration_tests;

#[path = "contact_fold_tests.rs"]
mod contact_fold_tests;

#[path = "motion_bench_tests.rs"]
mod motion_bench_tests;

#[path = "virtual_shadow_tests.rs"]
pub(crate) mod virtual_shadow_tests;

/// Optional `virtual` manifest object overriding `VirtualShadowSettings` fields.
/// An omitted lookup follows the library's Surface default.
pub(crate) fn virtual_settings(m: &serde_json::Value) -> Option<crate::VirtualShadowSettings> {
    let v = m.get("virtual")?;
    let mut s = crate::VirtualShadowSettings::default();
    let f = |key: &str, value: &mut f32| {
        if let Some(x) = v[key].as_f64() {
            *value = x as f32;
        }
    };
    f("near", &mut s.near);
    f("texels_per_pixel_log2", &mut s.texels_per_pixel_log2);
    f("finest_texel", &mut s.finest_texel);
    f("caster_error_texels", &mut s.caster_error_texels);
    let u = |key: &str, value: &mut u32| {
        if let Some(x) = v[key].as_u64() {
            *value = x as u32;
        }
    };
    u("pages", &mut s.pages);
    u("march_samples", &mut s.march_samples);
    u("bias_ulps", &mut s.bias_ulps);
    u("first_block", &mut s.first_block);
    u("directions", &mut s.directions);
    u("walk_cap", &mut s.walk_cap);
    u("walk_steps", &mut s.walk_steps);
    u("refresh", &mut s.refresh);
    f("sun_hysteresis", &mut s.sun_hysteresis);
    if let Some(ring) = v["ring"].as_bool() {
        s.ring = ring;
    }
    if let Some(peel_own) = v["peel_own"].as_bool() {
        s.peel_own = peel_own;
    }
    if let Some(truncated) = v["truncated_walks_block"].as_bool() {
        s.truncated_walks_block = truncated;
    }
    s.walk_records = match v["walk_records"].as_str() {
        Some("off") => crate::WalkRecords::Off,
        Some("source") => crate::WalkRecords::Source,
        Some("all") => crate::WalkRecords::All,
        _ => s.walk_records,
    };
    s.lookup = match v["lookup"].as_str() {
        Some("exact") => crate::VirtualLookup::Exact,
        Some("map") => crate::VirtualLookup::Map,
        Some("hybrid") => crate::VirtualLookup::Hybrid,
        Some("surface") => crate::VirtualLookup::Surface,
        Some("disk") => crate::VirtualLookup::Disk,
        _ => s.lookup,
    };
    Some(s)
}

/// Pre-N8p reference benches retain Exact unless the manifest explicitly selects a lookup.
/// Keep this separate from the settings parser so default-path gates exercise Surface.
pub(crate) fn exact_virtual_settings(m: &serde_json::Value) -> crate::VirtualShadowSettings {
    let mut settings = virtual_settings(m).unwrap_or_default();
    if m["virtual"]["lookup"].is_null() {
        settings.lookup = crate::VirtualLookup::Exact;
    }
    settings
}

#[test]
fn virtual_manifest_lookup_defaults_and_overrides() {
    assert_eq!(crate::VirtualLookup::default(), crate::VirtualLookup::Disk);
    assert_eq!(
        virtual_settings(&serde_json::json!({"virtual": {}}))
            .unwrap()
            .lookup,
        crate::VirtualLookup::Disk
    );
    assert_eq!(
        exact_virtual_settings(&serde_json::json!({})).lookup,
        crate::VirtualLookup::Exact
    );
    for (name, lookup) in [
        ("exact", crate::VirtualLookup::Exact),
        ("surface", crate::VirtualLookup::Surface),
        ("disk", crate::VirtualLookup::Disk),
    ] {
        let manifest = serde_json::json!({"virtual": {"lookup": name}});
        assert_eq!(virtual_settings(&manifest).unwrap().lookup, lookup);
        assert_eq!(exact_virtual_settings(&manifest).lookup, lookup);
    }
}

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
/// `film_background`.
fn black_clear(source: &str) -> String {
    let sites = source.matches("primary_frame.clear").count();
    let temporal = source.contains("fn film_background()");
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

pub(super) fn t1_cameras(m: &serde_json::Value) -> Vec<Camera> {
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
            r.test_standard().swap_traced(&mut sun);
            traced_frame(r, color, camera, asset, true);
            fs::write(prefix.with_extension("sun.rgba16f"), capture(r)).unwrap();
            r.test_standard().swap_traced(&mut sun);
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
    temporal_acceptance(&mut r, &mut asset, &color, &m);
}

/// Reuses an uploaded quarry for the final integration batch.
fn temporal_acceptance(
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    m: &serde_json::Value,
) {
    let cameras = t1_cameras(m);
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
    traced_frame(r, color, &cameras[0], asset, true);
    let report = traced_frame(r, color, &cameras[0], asset, true);
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
    for pose in poses(m) {
        let camera = &cameras[pose];
        let out = root.join(format!("t1-p{pose}"));
        fs::create_dir_all(&out).unwrap();
        let (surface, forward) = centre_surface(r, asset, color, camera);
        let conditioned = ConditionedVisibility::new(r, &surface, forward);
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
            let mut centre = centre_shading(r, &program, black);
            let mut temporal = temporal_shading(r, &program, black);
            // T-0: a reset frame equals Interactive's frame, byte for byte. Reset frames shade
            // with the program's traced pipelines, so the variant stays installed throughout.
            r.set_soft_shadows(crate::SoftShadows::Interactive);
            r.test_standard().swap_traced(&mut centre);
            traced_frame(r, color, camera, asset, true);
            let interactive = [settled(r), interactive_outputs(r).concat()];
            reset_film(r);
            std::mem::swap(r.test_standard().temporal.as_mut().unwrap(), &mut temporal);
            let mut digests = vec![];
            for n in 1..=frames {
                traced_frame(r, color, camera, asset, true);
                if name == "full" && !watch.is_empty() {
                    r.test_drain("T1 watch");
                    let [hit, counts] = interactive_outputs(r);
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
                    let first = [settled(r), interactive_outputs(r).concat()];
                    let same = [first[0] == interactive[0], first[1] == interactive[1]];
                    identity.push(serde_json::json!({"pose":pose,"program":name,"hdr_identical":same[0],"hit_and_counts_identical":same[1]}));
                    assert!(same[0] && same[1], "T-0 pose={pose} program={name}");
                }
                if checkpoints.contains(&n) {
                    let hdr = settled(r);
                    digests.push(serde_json::json!({"n":n,"sha256":digest(&hdr)}));
                    fs::write(out.join(format!("{name}-n{n:03}.rgba16f")), hdr).unwrap();
                }
                if name == "visibility" {
                    // After the film capture: this pass overwrites the frame's HDR image.
                    conditioned.accumulate(r, 0);
                    if checkpoints.contains(&n) {
                        let size = conditioned.result[0].size();
                        fs::write(
                            out.join(format!("conditioned-n{n:03}.u32")),
                            read_buffer(r, &conditioned.result[0], size),
                        )
                        .unwrap();
                    }
                }
            }
            std::mem::swap(r.test_standard().temporal.as_mut().unwrap(), &mut temporal);
            r.test_standard().swap_traced(&mut centre);
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
    reset_film(r);
    for n in 1..=64 {
        traced_frame(r, color, &cameras[from], asset, true);
        if n == 64 {
            cuts.push(
                serde_json::json!({"segment":"converged","n":n,"sha256":digest(&settled(r))}),
            );
        }
    }
    for (segment, pose) in [("cut", to), ("back", from)] {
        for n in 1..=32 {
            traced_frame(r, color, &cameras[pose], asset, true);
            if checkpoints.contains(&n) {
                cuts.push(serde_json::json!({"segment":segment,"pose":pose,"n":n,"sha256":digest(&settled(r))}));
            }
        }
    }
    // F8: in-process static repeats of the full program's sequence at the first pose.
    let pose = poses(m)[0];
    let repeats = m["repeats"].as_u64().unwrap_or(32) as usize;
    let mut first: Option<(Vec<String>, Vec<u8>)> = None;
    let mut identical = 0;
    let mut differing = vec![];
    for repeat in 0..repeats {
        reset_film(r);
        let mut run = vec![];
        for n in 1..=frames {
            traced_frame(r, color, &cameras[pose], asset, true);
            if [1, 2, 8, 32, frames].contains(&n) {
                run.push(digest(&settled(r)));
            }
        }
        let state = history(r);
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
        reset_film(r);
        let mut images = vec![];
        for _ in 1..=frames {
            traced_frame(r, color, &cameras[pose], asset, true);
            images.push(digest(&settled(r)));
        }
        let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
        let [colour, anchor, tiles] = film.history(film.parity).map(|t| read_texture(r, t));
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
/// alternate (order rotating per pair) at the first manifest pose, after warmup pairs.
/// Temporal frames keep accumulating, the steady static state. Validation is off.
#[test]
#[ignore = "owned GPU lease; T1 paired frame timing, offscreen"]
fn gpu_film_t1_timing() {
    let m = manifest();
    let _validation = validation::begin(false);
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let camera = t1_cameras(&m)[poses(&m)[0]];
    let warmup = m["warmup_pairs"].as_u64().unwrap_or(24) as usize;
    let pairs = m["pairs"].as_u64().unwrap_or(48) as usize;
    let modes = [
        crate::SoftShadows::Interactive,
        crate::SoftShadows::Temporal,
    ];
    let mut pending = std::collections::HashMap::new();
    let mut samples = vec![];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut collect =
        |r: &mut Renderer, pending: &mut std::collections::HashMap<u64, (usize, usize)>| {
            for g in r.poll().unwrap().gpu {
                if let Some((pair, mode)) = pending.remove(&g.frame.0) {
                    samples.push(serde_json::json!({"pair":pair,"mode":format!("{:?}",modes[mode]),
                    "host_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64(),
                    "trace_ms":g.receiver_pass.map(ms),"shading_ms":ms(g.mesh_pass),
                    "tone_ms":g.present_pass.map(ms),"span_ms":ms(g.render_span)}));
                }
            }
        };
    for pair in 0..warmup + pairs {
        for j in 0..2 {
            let mode = (pair + j) % 2;
            // set_soft_shadows would release the film history when leaving Temporal; the
            // field keeps it, so Temporal frames stay in their steady static state.
            r.soft_mode = modes[mode];
            let report = traced_frame(&mut r, &color, &camera, &mut asset, true);
            assert_eq!(report.soft_shadows, modes[mode]);
            if pair >= warmup {
                pending.insert(report.frame.0, (pair - warmup, mode));
            }
            collect(&mut r, &mut pending);
        }
    }
    r.test_drain("T1 paired timing");
    collect(&mut r, &mut pending);
    assert!(
        pending.is_empty(),
        "{} frames without GPU samples; timing {:?}",
        pending.len(),
        r.timing_status
    );
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("paired-timing.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"warmup_pairs":warmup,"pairs":pairs,"samples":samples}),
        )
        .unwrap(),
    )
    .unwrap();
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
    footprint_with(&m, &mut r, &mut asset, &color, &cameras);
}

pub(super) fn footprint_with(
    m: &serde_json::Value,
    r: &mut Renderer,
    asset: &mut prepared::Prepared,
    color: &wgpu::Texture,
    cameras: &[Camera],
) {
    let grid = m["grid"].as_u64().unwrap_or(1024) as u32;
    let source = footprint_source();
    let mut results = serde_json::Map::new();
    let start = Instant::now();
    for (pose, pixels) in m["footprint"].as_object().unwrap() {
        let camera = &cameras[pose.parse::<usize>().unwrap()];
        let (surface, forward) = centre_surface(r, asset, color, camera);
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
