//! Shader reload latency on the quarry's renderer (hardware shadows, traced capable, 1x):
//! where an edit's compile time goes, per module and pipeline, and edit-to-image from a
//! file save through the first completed frame. Nested under primary_ray_gpu_tests for
//! its quarry and renderer helpers.
use super::*;
use naga::back::spv;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, ms(start.elapsed()))
}

/// wgpu-hal 30's SPIR-V options on this Vulkan 1.3 device without debug instance flags
/// (vulkan/adapter.rs), so CPU timings and sizes match what the driver receives.
fn spv_options() -> spv::Options<'static> {
    spv::Options {
        lang_version: (1, 6),
        flags: spv::WriterFlags::LABEL_VARYINGS | spv::WriterFlags::FORCE_POINT_SIZE,
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: naga::proc::BoundsCheckPolicy::Restrict,
            buffer: naga::proc::BoundsCheckPolicy::Unchecked,
            image_load: naga::proc::BoundsCheckPolicy::Unchecked,
            binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
        },
        force_loop_bounding: true,
        ray_query_initialization_tracking: true,
        ..Default::default()
    }
}

/// Instruction counts of one SPIR-V binary by opcode name (the ones that track inlining
/// and by-value struct traffic), plus its word count.
fn spv_stats(words: &[u32]) -> serde_json::Value {
    let names = [
        (54, "function"),
        (57, "call"),
        (59, "variable"),
        (61, "load"),
        (62, "store"),
        (80, "construct"),
        (81, "extract"),
        (247, "branch_conditional"),
        (95, "image_fetch"),
        (88, "image_sample_explicit"),
    ];
    let mut counts = std::collections::BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let (op, count) = (words[at] & 0xffff, (words[at] >> 16) as usize);
        if let Some((_, name)) = names.iter().find(|(o, _)| *o == op) {
            *counts.entry(*name).or_insert(0u32) += 1;
        }
        at += count.max(1);
    }
    serde_json::json!({"words": words.len(), "ops": counts})
}

/// naga's CPU share of one module as wgpu performs it: parse and validate (wgpu-core),
/// then SPIR-V for the whole module, or per entry point after override processing when
/// the module declares overrides (wgpu-hal `compile_stage`). Also the per-entry SPIR-V
/// of every listed entry, the code the driver compiles for it.
fn naga_cost(source: &str, entries: &[(&str, naga::ShaderStage)]) -> serde_json::Value {
    let (module, parse) = timed(|| naga::front::wgsl::parse_str(source).unwrap());
    let (info, validate) = timed(|| {
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap()
    });
    let options = spv_options();
    let entry = |name: &str, stage| {
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            Some((stage, name)),
            &Default::default(),
        )
        .unwrap();
        spv::write_vec(
            &module,
            &info,
            &options,
            Some(&spv::PipelineOptions {
                shader_stage: stage,
                entry_point: name.into(),
            }),
        )
        .unwrap()
    };
    let overrides = !module.overrides.is_empty();
    let (whole, write) = if overrides {
        (None, 0.0)
    } else {
        let (words, write) = timed(|| spv::write_vec(&module, &info, &options, None).unwrap());
        (Some(words.len()), write)
    };
    let mut per_entry = serde_json::Map::new();
    let mut entry_write = 0.0;
    for &(name, stage) in entries {
        let (words, write) = timed(|| entry(name, stage));
        entry_write += write;
        per_entry.insert(name.into(), spv_stats(&words));
    }
    serde_json::json!({"parse_ms": parse, "validate_ms": validate,
        "overrides": overrides, "module_spv_ms": write, "module_words": whole,
        // What wgpu-hal writes at pipeline creation for override modules.
        "entry_spv_ms": entry_write, "entries": per_entry})
}

/// Program composition variants on the production ABIs, as `compile_program` creates them.
#[derive(Clone, Copy, Debug)]
enum Variant {
    /// Raster lit pair on the plain lighting layout.
    Lit,
    /// Hardware-shadow lit pair.
    Hardware,
    /// Fresh-count lit pair (raster soft frames).
    Fresh,
    /// Traced Hard and Soft shading.
    Traced,
}
const VARIANTS: [Variant; 4] = [
    Variant::Lit,
    Variant::Hardware,
    Variant::Fresh,
    Variant::Traced,
];

/// The heads `compile_program` composes a program against.
struct Heads {
    hardware: String,
    fresh: String,
    traced: [String; 2],
}
impl Heads {
    fn new(prelude: &str) -> Self {
        Self {
            hardware: crate::hardware_shadows::shader_source(prelude, SampleCount::One).unwrap(),
            fresh: fresh_shadow_gpu::shader_source(prelude).unwrap(),
            traced: primary_ray_gpu::shading_heads(prelude).unwrap(),
        }
    }
    /// The composed module sources of `variant`: one raster module, or Hard and Soft.
    fn compose(&self, prelude: &str, program: &str, variant: Variant) -> Vec<String> {
        let raster = |head: &str| {
            vec![
                crate::program::compose(head, program, crate::program::Stage::Raster, false).source,
            ]
        };
        match variant {
            Variant::Lit => raster(prelude),
            Variant::Hardware => raster(&self.hardware),
            Variant::Fresh => raster(&self.fresh),
            Variant::Traced => self
                .traced
                .iter()
                .map(|h| {
                    crate::program::compose(h, program, crate::program::Stage::Traced, false).source
                })
                .collect(),
        }
    }
}

fn module(r: &Renderer, source: &str) -> wgpu::ShaderModule {
    r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("reload profile"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// Creates `variant`'s pipelines from its modules, timing each creation the API exposes
/// separately: single raster pipelines, fresh and traced pairs.
fn variant_pipelines(r: &Renderer, variant: Variant, modules: &[wgpu::ShaderModule]) -> Vec<f64> {
    let lit = |layout: &wgpu::PipelineLayout| {
        [false, true]
            .map(|tangent| {
                timed(|| {
                    mesh_pipeline(
                        &r.device,
                        &r.ledger,
                        &modules[0],
                        layout,
                        wgpu::TextureFormat::Rgba16Float,
                        "fs_lit",
                        tangent,
                        r.options.samples,
                        None,
                    )
                })
                .1
            })
            .to_vec()
    };
    match variant {
        Variant::Lit => lit(&r.lighting.layout),
        Variant::Hardware => lit(&r.hardware_shadows.as_ref().unwrap().layout),
        Variant::Fresh => vec![
            timed(|| {
                r.view
                    .fresh
                    .as_ref()
                    .unwrap()
                    .shade_pipelines(&r.device, &r.ledger, &modules[0])
            })
            .1,
        ],
        Variant::Traced => vec![
            timed(|| {
                r.view.primary.as_ref().unwrap().shade_pipelines(
                    &r.device,
                    &r.ledger,
                    [&modules[0], &modules[1]],
                )
            })
            .1,
        ],
    }
}

/// Modules then pipelines of one variant, in one error scope.
fn compile_variant(r: &Renderer, variant: Variant, sources: &[String]) -> serde_json::Value {
    let scopes = OperationScopes::new(&r.device);
    let (modules, module_ms): (Vec<_>, Vec<_>) =
        sources.iter().map(|s| timed(|| module(r, s))).unzip();
    let pipelines = variant_pipelines(r, variant, &modules);
    assert!(scopes.finish().is_none(), "{variant:?} failed to compile");
    serde_json::json!({"variant": format!("{variant:?}"), "module_ms": module_ms,
        "pipeline_ms": pipelines,
        "total_ms": module_ms.iter().sum::<f64>() + pipelines.iter().sum::<f64>()})
}

/// The engine's prelude pipelines on `prelude`: the prelude module (diagnostic pair,
/// shadow depth, present), the fresh receiver and compute, and the traced kernels, as
/// `Renderer::compile` creates them, timed per group.
fn compile_engine(r: &Renderer, prelude: &str) -> serde_json::Value {
    let scopes = OperationScopes::new(&r.device);
    let (_, base) = timed(|| {
        Renderer::compile(
            &r.device,
            &r.ledger,
            &r.layout,
            &r.lighting,
            &r.view.shadows,
            None,
            None,
            r.format,
            r.options.samples,
            prelude,
        )
    });
    let fresh_source = fresh_shadow_gpu::shader_source(prelude).unwrap();
    let (_, fresh) = timed(|| {
        r.view
            .fresh
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &fresh_source)
    });
    let kernels = primary_ray_gpu::shader_sources(prelude).unwrap();
    let (_, traced) = timed(|| {
        r.view
            .primary
            .as_ref()
            .unwrap()
            .compile(&r.device, &r.ledger, &kernels)
    });
    assert!(scopes.finish().is_none(), "engine failed to compile");
    serde_json::json!({"prelude_ms": base, "fresh_kernels_ms": fresh, "traced_kernels_ms": traced,
        "total_ms": base + fresh + traced})
}

/// This process's random offset for `unique`: the driver's on-disk shader cache serves
/// a source an earlier process compiled, so fixed sequences would be cache hits on reruns.
fn salt() -> u32 {
    static SALT: OnceLock<u32> = OnceLock::new();
    *SALT.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        now.subsec_nanos() ^ (now.as_secs() as u32).rotate_left(16) ^ std::process::id()
    })
}
/// A distinct f32 in (`low`, 2 * `low`) for sample `n` of this process, so no driver
/// cache can serve an edit: `low` + k * `low` / 2^23 with k in 1..2^22.
fn unique_from(low: f64, n: u32) -> String {
    let k = 1 + salt().wrapping_add(n) % ((1 << 22) - 1);
    format!("{:.9}", low * (1.0 + f64::from(k) / 8_388_608.0))
}
/// A unique decimal just above 1 for sample `n`.
fn unique(n: u32) -> String {
    unique_from(1.0, n)
}
fn program_edit(n: u32) -> String {
    edit(
        STANDARD,
        (
            "*sampled.rgb*TINT",
            &format!("*sampled.rgb*TINT*{}", unique(n)),
        ),
    )
}
/// The tint edit `gpu_program_edit_to_image` makes, uniquified.
fn prelude_edit(n: u32) -> String {
    edit(
        SOURCE,
        (
            "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);",
            &format!(
                "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, {});",
                unique(n)
            ),
        ),
    )
}
/// A prelude edit every module's code reads (`unit_normal`), so no engine pipeline is
/// served from the driver cache.
fn prelude_edit_all(n: u32) -> String {
    edit(
        SOURCE,
        (
            "    let scaled=n/max(scale,1e-30);",
            &format!("    let scaled=n/max(scale,{}e-30);", unique(n)),
        ),
    )
}

/// Where reload time goes, per path and edit kind: naga (CPU), module creation (wgpu
/// parse, validation and whole-module SPIR-V) and pipeline creation (per-entry SPIR-V for
/// override modules, then the driver compile), per variant, for uniquified program and
/// prelude edits; the same sources again (driver cache); variants in parallel threads;
/// and ablations of the standard program locating the driver's cost.
#[test]
#[ignore = "shared GPU slot, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_reload_profile() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let reps = manifest["reps"].as_u64().unwrap_or(3) as u32;
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut n = 0;
    let mut next = || {
        n += 1;
        n
    };
    let mut rows = vec![];
    let mut emit = |row: serde_json::Value| {
        eprintln!("reload_profile {row}");
        rows.push(row);
    };
    // CPU: naga per module of each variant, current sources.
    let heads = Heads::new(SOURCE);
    let vertex = naga::ShaderStage::Vertex;
    let fragment = naga::ShaderStage::Fragment;
    let compute = naga::ShaderStage::Compute;
    for variant in VARIANTS {
        let sources = heads.compose(SOURCE, STANDARD, variant);
        for (i, source) in sources.iter().enumerate() {
            let entries: &[(&str, naga::ShaderStage)] = match variant {
                Variant::Traced => &[("primary_shade_tiles", compute)],
                _ => &[("fs_lit", fragment), ("vs", vertex), ("vs_tangent", vertex)],
            };
            let (_, compose) = timed(|| heads.compose(SOURCE, STANDARD, variant));
            let analyse = (i == 0 && matches!(variant, Variant::Lit)).then(|| {
                let composed =
                    crate::program::compose(SOURCE, STANDARD, crate::program::Stage::Raster, false);
                timed(|| crate::program::analyse("standard", STANDARD, &composed).unwrap()).1
            });
            emit(
                serde_json::json!({"kind": "naga", "variant": format!("{variant:?}"), "module": i,
                "lines": source.lines().count(), "compose_ms": compose, "analyse_ms": analyse,
                "naga": naga_cost(source, entries)}),
            );
        }
    }
    let kernels = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    for (i, (source, entries)) in kernels
        .iter()
        .zip([
            &[
                ("primary_trace", compute),
                ("primary_trace_receivers", compute),
            ][..],
            &[
                ("fresh_compute", compute),
                ("primary_trace_interactive", compute),
            ][..],
        ])
        .enumerate()
    {
        emit(
            serde_json::json!({"kind": "naga", "variant": "kernels", "module": i,
            "lines": source.lines().count(), "naga": naga_cost(source, entries)}),
        );
    }
    emit(
        serde_json::json!({"kind": "naga", "variant": "prelude", "module": 0,
        "naga": naga_cost(SOURCE, &[("fs", fragment), ("vs", vertex), ("fs_present", fragment)])}),
    );
    // The frozen pre-program compositions, for the program modules' compile difference.
    if let Some(frozen) = manifest["frozen"].as_str() {
        let frozen = PathBuf::from(frozen);
        for (name, entry, stage) in [
            ("raster.wgsl", "fs_lit", fragment),
            ("hardware-1x.wgsl", "fs_lit", fragment),
            ("fresh.wgsl", "fs_lit", fragment),
            ("traced-hard.wgsl", "primary_shade_tiles", compute),
            ("traced-soft.wgsl", "primary_shade_tiles", compute),
        ] {
            let source = fs::read_to_string(frozen.join(name)).unwrap();
            emit(serde_json::json!({"kind": "naga_frozen", "file": name,
                "naga": naga_cost(&source, &[(entry, stage)])}));
        }
    }
    for rep in 0..reps {
        // Sequential, as a reload compiles today, per edit kind; then the same sources
        // again, which the driver's shader cache serves.
        for kind in ["program", "prelude", "prelude_all"] {
            let count = next();
            let (prelude, program) = match kind {
                "program" => (SOURCE.to_owned(), program_edit(count)),
                "prelude" => (prelude_edit(count), STANDARD.to_owned()),
                _ => (prelude_edit_all(count), STANDARD.to_owned()),
            };
            let heads = Heads::new(&prelude);
            for cached in [false, true] {
                if kind != "program" {
                    let engine = compile_engine(&r, &prelude);
                    emit(
                        serde_json::json!({"kind": kind, "rep": rep, "cached": cached,
                        "variant": "engine", "engine": engine}),
                    );
                }
                for variant in VARIANTS {
                    let sources = heads.compose(&prelude, &program, variant);
                    let row = compile_variant(&r, variant, &sources);
                    emit(
                        serde_json::json!({"kind": kind, "rep": rep, "cached": cached, "compile": row}),
                    );
                }
            }
        }
        // Parallel: every program pipeline group of one uniquified program edit at once,
        // one thread per creation call; then the raster lit pair alone, in parallel.
        let program = program_edit(next());
        let sources: Vec<_> = VARIANTS
            .iter()
            .map(|&v| (v, heads.compose(SOURCE, &program, v)))
            .collect();
        let start = Instant::now();
        let parallel: Vec<serde_json::Value> = std::thread::scope(|s| {
            let r = &r;
            let jobs: Vec<_> = sources
                .iter()
                .flat_map(|(variant, sources)| {
                    let split = matches!(variant, Variant::Lit | Variant::Hardware);
                    let parts = if split { vec![Some(false), Some(true)] } else { vec![None] };
                    parts.into_iter().map(move |tangent| {
                        s.spawn(move || {
                            let begin = ms(start.elapsed());
                            let scopes = OperationScopes::new(&r.device);
                            let modules: Vec<_> = sources.iter().map(|x| module(r, x)).collect();
                            let pipelines = match (variant, tangent) {
                                (Variant::Lit | Variant::Hardware, Some(tangent)) => {
                                    let layout = if matches!(variant, Variant::Lit) {
                                        &*r.lighting.layout
                                    } else {
                                        &*r.hardware_shadows.as_ref().unwrap().layout
                                    };
                                    drop(mesh_pipeline(
                                        &r.device,
                                        &r.ledger,
                                        &modules[0],
                                        layout,
                                        wgpu::TextureFormat::Rgba16Float,
                                        "fs_lit",
                                        tangent,
                                        r.options.samples,
                                        None,
                                    ));
                                    1
                                }
                                _ => variant_pipelines(r, *variant, &modules).len(),
                            };
                            assert!(scopes.finish().is_none());
                            serde_json::json!({"variant": format!("{variant:?}"), "tangent": tangent,
                                "calls": pipelines, "begin_ms": begin, "end_ms": ms(start.elapsed())})
                        })
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        emit(serde_json::json!({"kind": "parallel_program", "rep": rep,
            "wall_ms": ms(start.elapsed()), "jobs": parallel}));
        let source = &heads.compose(SOURCE, &program_edit(next()), Variant::Lit)[0];
        let shared = module(&r, source);
        let start = Instant::now();
        let ends: Vec<f64> = std::thread::scope(|s| {
            let (r, shared) = (&r, &shared);
            let jobs: Vec<_> = [false, true]
                .map(|tangent| {
                    s.spawn(move || {
                        let scopes = OperationScopes::new(&r.device);
                        drop(mesh_pipeline(
                            &r.device,
                            &r.ledger,
                            shared,
                            &r.lighting.layout,
                            wgpu::TextureFormat::Rgba16Float,
                            "fs_lit",
                            tangent,
                            r.options.samples,
                            None,
                        ));
                        assert!(scopes.finish().is_none());
                        ms(start.elapsed())
                    })
                })
                .into();
            jobs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        emit(serde_json::json!({"kind": "parallel_lit_pair", "rep": rep, "end_ms": ends}));
        // Ablations: where the driver spends a lit and a traced compile.
        let flat = "struct Surface { c:vec3<f32> };\nfn surface(input:SurfaceInput)->Surface { return Surface(input.material.base_color.rgb*TINT); }\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return s.c; }\n";
        for (name, program) in [
            ("standard", program_edit(next())),
            (
                "no_environment",
                edit(
                    &program_edit(next()),
                    ("if environment_enabled() {", "if false {"),
                ),
            ),
            (
                "no_lights",
                edit(
                    &program_edit(next()),
                    (
                        "for(var i=0u;i<light_count();i+=1u) {",
                        "for(var i=0u;i<0u;i+=1u) {",
                    ),
                ),
            ),
            (
                "no_environment_no_lights",
                edit(
                    &edit(
                        &program_edit(next()),
                        ("if environment_enabled() {", "if false {"),
                    ),
                    (
                        "for(var i=0u;i<light_count();i+=1u) {",
                        "for(var i=0u;i<0u;i+=1u) {",
                    ),
                ),
            ),
            (
                "flat",
                flat.replace("*TINT", &format!("*TINT*{}", unique(next()))),
            ),
        ] {
            for variant in [Variant::Lit, Variant::Traced] {
                let sources = heads.compose(SOURCE, &program, variant);
                let row = compile_variant(&r, variant, &sources);
                emit(
                    serde_json::json!({"kind": "ablation", "rep": rep, "program": name, "compile": row}),
                );
            }
        }
    }
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
    fs::write(
        out.join("reload-profile.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

use crate::program_watch as shader_watch;

/// One watched file's edit: kind, path, the edited source for a tint value, the original.
type FileEdit<'a> = (&'a str, &'a PathBuf, fn(&str) -> String, &'a str);

/// Edit-to-image from a file save, through the viewer's own watcher, on the quarry,
/// raster then traced: a tint edit and its revert in `shaders/programs/standard.wgsl`
/// (a program edit) and in `scene.wgsl` (a prelude edit), written into a scratch copy.
/// Each sample separates save to detection (the poll wait: the poll runs `poll_ms`
/// apart, at a phase spread over the interval) from detection to the first completed
/// frame (reload, then one frame rendered and completed on the GPU; presentation
/// excluded). Reverts are byte-identical to the first image; invalid program and prelude
/// edits are rejected through the watcher with that image kept.
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_reload_edit_to_image() {
    let manifest = manifest();
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    let reps = manifest["reps"].as_u64().unwrap_or(5) as usize;
    let poll = Duration::from_millis(manifest["poll_ms"].as_u64().unwrap_or(50));
    let files = out.join("watch");
    fs::create_dir_all(files.join("programs")).unwrap();
    let prelude_path = files.join("scene.wgsl");
    let program_path = shader_watch::ShaderWatch::standard_beside(&prelude_path);
    fs::write(&prelude_path, SOURCE).unwrap();
    fs::write(&program_path, STANDARD).unwrap();
    // The viewer enables the pipeline cache when the adapter has it; so does this test.
    let mut r = renderer_with_features(validate, wgpu::Features::PIPELINE_CACHE);
    let mut watch =
        shader_watch::ShaderWatch::new(prelude_path.clone(), SOURCE.into(), program_path.clone());
    assert!(
        watch.poll(&mut r).is_empty(),
        "unchanged files reload nothing"
    );
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    // Each edit sample tints by its own value in (0.5, 1), unique to this process.
    let edits: [FileEdit<'_>; 2] = [
        (
            "program",
            &program_path,
            |v| {
                edit(
                    STANDARD,
                    ("*sampled.rgb*TINT", &format!("*sampled.rgb*TINT*{v}")),
                )
            },
            STANDARD,
        ),
        (
            "prelude",
            &prelude_path,
            |v| {
                edit(
                    SOURCE,
                    (
                        "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);",
                        &format!("const TINT: vec3<f32> = vec3<f32>({v}, {v}, {v});"),
                    ),
                )
            },
            SOURCE,
        ),
    ];
    // Saves `text` to `path`, waits out a poll phase, then polls until the watcher reloads.
    let save_and_reload = |r: &mut Renderer,
                           watch: &mut shader_watch::ShaderWatch,
                           path: &PathBuf,
                           text: &str,
                           phase: f64| {
        r.test_drain("edit admission");
        fs::write(path, text).unwrap();
        let saved = Instant::now();
        std::thread::sleep(poll.mul_f64(phase));
        loop {
            let mut reloads = watch.poll(r);
            if !reloads.is_empty() {
                assert_eq!(reloads.len(), 1, "one file changed");
                return (saved, reloads.remove(0));
            }
            std::thread::sleep(poll);
        }
    };
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut rows = vec![];
    let mut sample = 0;
    for traced in [false, true] {
        r.set_primary_visibility(if traced {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::Raster
        })
        .unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, traced);
        let accepted = capture(&mut r);
        for (kind, path, edited, original) in &edits {
            for rep in 0..reps {
                let edited = edited(&unique_from(0.5, sample as u32));
                for (step, text) in [("edit", edited.as_str()), ("revert", *original)] {
                    // Phases spread evenly over the poll interval, in a scrambled order.
                    let phase = f64::from((sample * 7 % 20) as u32) / 20.0;
                    sample += 1;
                    let created = r.stats().pipelines.created;
                    let (saved, reload) = save_and_reload(&mut r, &mut watch, path, text, phase);
                    assert_eq!(&reload.path, *path);
                    assert_eq!(
                        reload.kind == shader_watch::Kind::Prelude,
                        *kind == "prelude"
                    );
                    assert!(!reload.retry);
                    let report = reload.result.unwrap();
                    let reloaded = Instant::now();
                    traced_frame(&mut r, &color, &camera, &mut asset, traced);
                    r.test_drain("edit image");
                    let image = Instant::now();
                    assert_eq!(
                        capture(&mut r) == accepted,
                        step == "revert",
                        "{kind} {step}"
                    );
                    rows.push(
                        serde_json::json!({"path": if traced {"traced"} else {"raster"},
                        "kind": kind, "step": step, "rep": rep, "phase": phase,
                        "save_to_detect_ms": ms(reload.detected - saved),
                        "file_age_ms": ms(reload.file_age),
                        "detect_to_image_ms": ms(image - reload.detected),
                        "compile_ms": ms(report.compile),
                        "detect_to_reloaded_ms": ms(reloaded - reload.detected),
                        "frame_ms": ms(image - reloaded),
                        "save_to_image_ms": ms(image - saved),
                        "pipelines": r.stats().pipelines.created - created}),
                    );
                    eprintln!("edit_to_image {}", rows.last().unwrap());
                }
            }
        }
        // Invalid saves are rejected through the watcher and keep the accepted image.
        for (kind, path, invalid, original) in [
            (
                "program",
                &program_path,
                STANDARD.replace(
                    "return Surface(n,decal_base.rgb,decal_mr.y,decal_mr.x,alpha);",
                    "return Surface(n,base,metal);",
                ),
                STANDARD,
            ),
            (
                "prelude",
                &prelude_path,
                SOURCE.replace(
                    "fn unit_normal(n:vec3<f32>) -> vec3<f32> {",
                    "fn unit_normal(n:vec3<f32>) -> vec3<f32> {\n    let broken=;",
                ),
                SOURCE,
            ),
        ] {
            let (_, reload) = save_and_reload(&mut r, &mut watch, path, &invalid, 0.0);
            let Err(ReloadError::Rejected {
                diagnostic,
                compile,
            }) = reload.result
            else {
                panic!("{kind}: invalid edit was not rejected");
            };
            assert!(watch.rejected());
            traced_frame(&mut r, &color, &camera, &mut asset, traced);
            assert!(
                capture(&mut r) == accepted,
                "{kind} rejection kept the image"
            );
            let (_, restored) = save_and_reload(&mut r, &mut watch, path, original, 0.0);
            restored.result.unwrap();
            assert!(!watch.rejected());
            rows.push(serde_json::json!({"path": if traced {"traced"} else {"raster"},
                "kind": kind, "step": "rejected", "compile_ms": ms(compile), "diagnostic": diagnostic}));
            eprintln!("edit_to_image {}", rows.last().unwrap());
        }
    }
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
    fs::write(
        out.join("edit-to-image.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"validation": validate, "reps": reps,
            "poll_ms": ms(poll), "samples": rows}))
        .unwrap(),
    )
    .unwrap();
}

/// One raster lit pipeline (no tangents) on the plain lighting layout, as `mesh_pipeline`
/// creates it, through an optional pipeline cache.
fn cached_lit_pipeline(
    r: &Renderer,
    module: &wgpu::ShaderModule,
    cache: Option<&wgpu::PipelineCache>,
) -> wgpu::RenderPipeline {
    r.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cache probe"),
            layout: Some(&r.lighting.layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 48,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 4,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &MODEL_ATTRIBUTES,
                    }),
                    None,
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_lit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache,
        })
}

/// Paired mechanism screens for reload latency, alternating order per rep:
/// - whole-module SPIR-V (production for modules without overrides) against per-entry
///   SPIR-V, forced by an unused override, which wgpu-hal compacts per entry point;
/// - `wgpu::PipelineCache`: two independent lit pipelines created concurrently, then
///   again from fresh modules (a revert), with no cache, one shared cache or one each.
#[test]
#[ignore = "shared GPU slot, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_reload_mechanisms() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let reps = manifest["reps"].as_u64().unwrap_or(4) as u32;
    let mut r = renderer_with_features(false, wgpu::Features::PIPELINE_CACHE);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let heads = Heads::new(SOURCE);
    let mut rows = vec![];
    let mut n = 1000;
    // "spirv": false skips the per-entry SPIR-V screen.
    let spirv = manifest["spirv"].as_bool().unwrap_or(true);
    for rep in 0..reps {
        for variant in [Variant::Lit, Variant::Hardware, Variant::Traced]
            .into_iter()
            .filter(|_| spirv)
        {
            let modes = if rep % 2 == 0 {
                ["whole", "entry"]
            } else {
                ["entry", "whole"]
            };
            for mode in modes {
                n += 1;
                let sources: Vec<String> = heads
                    .compose(SOURCE, &program_edit(n), variant)
                    .into_iter()
                    .map(|s| {
                        if mode == "entry" {
                            s + "\noverride RL_ENTRY_SPLIT: u32 = 0u;\n"
                        } else {
                            s
                        }
                    })
                    .collect();
                let row = compile_variant(&r, variant, &sources);
                eprintln!("mechanism {mode} {row}");
                rows.push(serde_json::json!({"screen": "spirv", "mode": mode, "rep": rep, "compile": row}));
            }
        }
        // Two independent lit pipelines (distinct unique sources) created concurrently, then
        // again from fresh modules (a revert), with no cache, one shared cache, or one cache
        // each: whether a shared cache serializes concurrent compiles, and which mode makes
        // the repeat a hit.
        let create_cache = || {
            // SAFETY: no initial data is supplied, so nothing unvalidated reaches the driver.
            unsafe {
                r.device
                    .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                        label: Some("reload probe"),
                        data: None,
                        fallback: true,
                    })
            }
        };
        let mut times = serde_json::Map::new();
        let modes = [
            ["none", "shared", "separate"],
            ["shared", "separate", "none"],
            ["separate", "none", "shared"],
        ][rep as usize % 3];
        for mode in modes {
            let caches: [Option<wgpu::PipelineCache>; 2] = match mode {
                "none" => [None, None],
                "shared" => [Some(create_cache()), None],
                _ => [Some(create_cache()), Some(create_cache())],
            };
            let cache = |i: usize| match mode {
                "shared" => caches[0].as_ref(),
                _ => caches[i].as_ref(),
            };
            let sources = [0, 1].map(|_| {
                n += 1;
                heads
                    .compose(SOURCE, &program_edit(n), Variant::Lit)
                    .remove(0)
            });
            for step in ["first", "repeat"] {
                let modules = sources.each_ref().map(|s| module(&r, s));
                let start = Instant::now();
                let ends: Vec<f64> = std::thread::scope(|s| {
                    let jobs: Vec<_> = [0, 1]
                        .map(|i| {
                            let (r, module, cache) = (&r, &modules[i], cache(i));
                            s.spawn(move || {
                                let scopes = OperationScopes::new(&r.device);
                                drop(cached_lit_pipeline(r, module, cache));
                                assert!(scopes.finish().is_none());
                                ms(start.elapsed())
                            })
                        })
                        .into();
                    jobs.into_iter().map(|j| j.join().unwrap()).collect()
                });
                times.insert(
                    format!("{step}_{mode}_ms"),
                    ends.iter().copied().fold(0.0, f64::max).into(),
                );
                times.insert(format!("{step}_{mode}_ends"), ends.into());
            }
        }
        eprintln!("mechanism cache {times:?}");
        rows.push(serde_json::json!({"screen": "cache", "rep": rep, "times": times}));
    }
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
    fs::write(
        out.join("reload-mechanisms.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

/// The program pipeline contract on a traced-capable hardware renderer, with validation:
/// a frame creates the sets it draws with (raster Lit, traced Hard/Soft) and only those;
/// a program reload recreates only the set the last frame used, and a frame on another
/// path creates its set then; a prelude reload recreates the engine pipelines and the hot
/// set; an edit valid on raster but invalid on traced is rejected with the image kept
/// while traced is selected, because every path's module is created in the transaction.
#[test]
#[ignore = "shared GPU slot, experimental acknowledgment; offscreen"]
fn gpu_reload_on_demand() {
    let _validation = validation::begin(true);
    let mut r = renderer_with_features(true, wgpu::Features::PIPELINE_CACHE);
    let quad = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]].map(|[x, y]| Vertex {
        position: [x, y, 0.],
        normal: [0., 0., 1.],
        uv: [0.5; 2],
        color: [1.; 4],
    });
    let mesh = loop {
        if let Some(mesh) = r
            .try_upload_mesh(&MeshData {
                vertices: &quad,
                indices: &[0, 1, 2, 0, 2, 3],
            })
            .unwrap()
        {
            break mesh;
        }
        r.test_drain("on-demand mesh");
    };
    let mut instances = loop {
        if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
            break i;
        }
        r.test_drain("on-demand instances");
    };
    r.set_lighting(&Lighting {
        lights: vec![],
        environment: [0.5, 0.25, 0.125],
        ..Default::default()
    })
    .unwrap();
    let color = target(&r, 64, 64);
    let camera =
        Camera::look_at(Vec3::new(0., 0., 3.), Vec3::ZERO, Vec3::Y, 0.8, 0.1, 10.).unwrap();
    let created = |r: &Renderer| r.stats().pipelines.created;
    let mut frame = |r: &mut Renderer, traced: bool| {
        r.primary_mode = if traced {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::Raster
        };
        let before = created(r);
        render(r, &color, &camera, &mut [Draw::new(&mesh, &mut instances)]);
        (created(r) - before, capture(r))
    };
    // The first raster frame creates the standard program's Lit pair, then nothing.
    assert_eq!(frame(&mut r, false).0, 2);
    let (none, raster) = frame(&mut r, false);
    assert_eq!(none, 0);
    let tinted = edit(STANDARD, ("*sampled.rgb*TINT", "*sampled.rgb*TINT*0.5"));
    let before = created(&r);
    r.test_reload_standard(&tinted).unwrap();
    assert_eq!(
        created(&r) - before,
        2,
        "a program edit recreates the hot Lit pair"
    );
    let (none, edited) = frame(&mut r, false);
    assert_eq!(none, 0);
    assert_ne!(edited, raster);
    r.test_reload_standard(STANDARD).unwrap();
    assert_eq!(
        frame(&mut r, false),
        (0, raster.clone()),
        "the revert restores the image"
    );
    // Traced selection creates the kernels and every program's traced modules; the
    // first traced frame then creates the traced pair.
    let before = created(&r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    assert_eq!(
        created(&r) - before,
        4,
        "traced selection creates only the kernels"
    );
    let (new, traced) = frame(&mut r, true);
    assert_eq!(new, 2, "the first traced frame creates the traced pair");
    let before = created(&r);
    r.test_reload_standard(&tinted).unwrap();
    assert_eq!(
        created(&r) - before,
        2,
        "a traced program edit recreates only that pair"
    );
    assert_eq!(frame(&mut r, true).0, 0);
    // The reload dropped the old Lit pair: the next raster frame creates the edited one.
    let (new, edited_raster) = frame(&mut r, false);
    assert_eq!(new, 2);
    assert_eq!(
        edited_raster, edited,
        "on-demand Lit pair renders the edited program"
    );
    r.test_reload_standard(STANDARD).unwrap();
    assert_eq!(frame(&mut r, false), (0, raster.clone()));
    assert_eq!(frame(&mut r, true), (2, traced.clone()));
    // A prelude reload: the engine pipelines and the hot traced pair.
    let before = created(&r);
    let engine = r.pipeline.count();
    r.reload(SOURCE).unwrap();
    assert_eq!(created(&r) - before, engine + 2);
    assert_eq!(frame(&mut r, true), (0, traced.clone()));
    // Implicit sampling is valid raster WGSL but forbidden in traced shading. The traced
    // module is created in the transaction, so the edit is rejected at its program line
    // and every path keeps its last working pipelines.
    let implicit = edit(
        STANDARD,
        (
            "let sampled=sample_base_color(input);",
            "let sampled=textureSample(base_color,base_sampler,input.uv);",
        ),
    );
    match r.test_reload_standard(&implicit) {
        Err(ReloadError::Rejected { diagnostic, .. }) => {
            assert!(
                diagnostic.contains("program `standard` line 8:"),
                "{diagnostic}"
            )
        }
        other => panic!("expected rejection, got {other:?}"),
    }
    assert_eq!(frame(&mut r, true), (0, traced));
    assert_eq!(frame(&mut r, false), (2, raster));
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
}

/// The prelude's specular environment with `environment_level` (three bilinear filters)
/// inlined twice instead of five times: each path's levels and weights are chosen first,
/// with the same arithmetic per path. A probe of the driver-compile limiter only; the
/// production prelude is unchanged.
fn environment_two_levels(prelude: &str) -> String {
    let start = prelude.find("fn environment_legacy_lod(").unwrap();
    let end = prelude.find("fn environment_product(").unwrap();
    let replacement = "fn environment_radiance(d:vec3<f32>, roughness:f32) -> vec3<f32> {
    let r=clamp(roughness,0.0,1.0);
    var lo=64;
    var hi=64;
    var t=0.0;
    var single=false;
    if lighting.map.w>0.0 {
        let x=clamp(r*(lighting.map.w-1.0),0.0,lighting.map.w-1.0);
        lo=i32(floor(x));
        hi=min(lo+1,i32(lighting.map.w)-1);
        t=fract(x);
    } else if r>=1.0 {
        single=true;
    } else {
        lo=min(i32(floor(64.0*sqrt(r))),63);
        hi=lo+1;
        let a=f32(lo)/64.0;
        let b=f32(lo+1)/64.0;
        let a2=a*a; let b2=b*b;
        let a4=a2*a2; let b4=b2*b2;
        let a8=a4*a4; let b8=b4*b4;
        let r2=r*r;
        t=clamp((r2*r2-a8)/(b8-a8),0.0,1.0);
    }
    let first=environment_level(d,lo);
    if single { return first; }
    return first*(1.0-t)+environment_level(d,hi)*t;
}
";
    format!("{}{replacement}{}", &prelude[..start], &prelude[end..])
}

/// Quarry frames, raster then traced: the production prelude against the two-level
/// environment probe, as prelude reloads that recreate the hot program set. Records
/// each reload's compile time and whether the probe's HDR image is byte-identical.
#[test]
#[ignore = "shared GPU slot, 8 GiB RAM, experimental acknowledgment, RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_reload_environment_probe() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let reps = manifest["reps"].as_u64().unwrap_or(3) as u32;
    let mut r = renderer(false);
    let (mut asset, cameras) = load_quarry(&manifest, &mut r);
    let color = target(&r, 1920, 1080);
    let camera = cameras.frames[0].camera().unwrap();
    let probe = environment_two_levels(SOURCE);
    let mut rows = vec![];
    let mut n = 2000;
    for traced in [false, true] {
        r.set_primary_visibility(if traced {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::Raster
        })
        .unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, traced);
        let accepted = capture(&mut r);
        for rep in 0..reps {
            let order = if rep % 2 == 0 {
                ["production", "two_levels"]
            } else {
                ["two_levels", "production"]
            };
            for kind in order {
                n += 1;
                // Each reload is uniquified, so no compile is served from a cache.
                let base = if kind == "production" { SOURCE } else { &probe };
                let source = base.replace(
                    "    let scaled=n/max(scale,1e-30);",
                    &format!("    let scaled=n/max(scale,{}e-30);", unique(n)),
                );
                r.test_drain("environment probe");
                let report = r.reload(&source).unwrap();
                traced_frame(&mut r, &color, &camera, &mut asset, traced);
                let image = capture(&mut r);
                rows.push(
                    serde_json::json!({"path": if traced {"traced"} else {"raster"},
                    "kind": kind, "rep": rep, "compile_ms": ms(report.compile),
                    "identical": image == accepted}),
                );
                eprintln!("environment_probe {}", rows.last().unwrap());
            }
        }
        r.reload(SOURCE).unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, traced);
        assert!(
            capture(&mut r) == accepted,
            "production prelude restores the image"
        );
    }
    r.set_primary_visibility(PrimaryVisibility::Raster).unwrap();
    fs::write(
        out.join("environment-probe.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
