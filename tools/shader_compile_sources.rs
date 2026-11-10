//! Final WGSL module catalogue for the CPU-only structure census.
//!
//! This calls the same source composers as production. Entries are the entry points
//! production actually selects, not every incidental entry retained in a shared prelude.
//! Runtime values (sun, lights, maps, scene size) do not change these module texts.
use crate::renderer::{
    atmosphere_gpu, fresh_shadow_gpu, gi_cache, post_gpu, primary_ray_gpu, transparency_gpu,
    virtual_shadow_gpu, visibility_buffer,
};
use crate::{SampleCount, program};

pub(super) struct Source {
    pub name: String,
    pub path: String,
    pub source: String,
    /// Actual production module-creation API, not an assertion that no user code is present.
    pub engine_owned: bool,
    pub entry_points: Vec<&'static str>,
    pub constants: Vec<(String, f64)>,
}

fn add(
    sources: &mut Vec<Source>,
    path: &str,
    name: &str,
    source: String,
    engine_owned: bool,
    entries: &[&'static str],
) {
    sources.push(Source {
        name: name.to_owned(),
        path: path.to_owned(),
        source,
        engine_owned,
        entry_points: entries.to_vec(),
        constants: Vec::new(),
    });
}

fn overrides(sources: &mut [Source], constants: &[(&str, f64)]) {
    sources.last_mut().unwrap().constants = constants
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
}

/// All production module families at the shipped prelude/default structural settings.
///
/// Conditional paths are named explicitly: raster MSAA, streamed materials, mixed-program
/// lists, temporal film, virtual-map modes, glass, particles, procedural sky and post.
/// GI cell modules are absent for programs without `scatter` or `emission` (production's own gate).
/// Arbitrary custom preludes, test-only shader replacement environment variables,
/// diagnostic experiment files and the CPU path-traced reference are outside this census.
/// Non-default GI direction counts/custom specialization constants are not an exhaustive
/// Cartesian product; explicit override variants below are actual production pipelines.
/// Callers must reject source-override environment variables before collecting this list.
pub(super) fn sources(program_source: &str) -> Result<Vec<Source>, String> {
    for variable in [
        "RENDERLAB_GI_SHADERS",
        "RENDERLAB_VSM_SHADER",
        "RENDERLAB_VMAP_SHADER",
        "RENDERLAB_VSC_SHADER",
        "RENDERLAB_TP2_PROFILE",
    ] {
        if std::env::var_os(variable).is_some_and(|value| !value.is_empty()) {
            return Err(format!(
                "unset {variable}: the census requires production source composers"
            ));
        }
    }
    let prelude = include_str!("../shaders/scene.wgsl");
    let parameters = program::declares_parameters(program_source);
    let checked_as_engine = program_source == program::STANDARD_PROGRAM;
    let mut out = Vec::new();
    let err = |e: crate::Error| e.to_string();

    // The renderer's diagnostic, shadow-depth and presentation pipelines contain no
    // authored material program. Both device paths build these from the same prelude.
    add(
        &mut out,
        "shared",
        "engine-prelude",
        prelude.to_owned(),
        true,
        &[
            "vs",
            "vs_tangent",
            "fs",
            "vs_shadow",
            "fs_shadow",
            "vs_present",
            "fs_present",
        ],
    );
    add(
        &mut out,
        "shared",
        "skinning",
        include_str!("../shaders/skinning.wgsl").to_owned(),
        true,
        &["skin_vertices", "skin_clusters"],
    );
    add(
        &mut out,
        "shared",
        "light-clusters",
        format!(
            "{}{}",
            crate::light_cluster::constants(),
            crate::light_cluster::KERNEL
        ),
        true,
        &["lc_build"],
    );
    add(
        &mut out,
        "shared",
        "local-shadow-clear",
        include_str!("../shaders/local_light_clear.wgsl").to_owned(),
        true,
        &["vertex", "fragment"],
    );
    add(
        &mut out,
        "shared",
        "program-bins",
        include_str!("../shaders/program_bins.wgsl").to_owned(),
        true,
        &["program_bins"],
    );

    let raster_entries = ["vs", "vs_tangent", "fs_lit"];
    add(
        &mut out,
        "shared",
        "program-raster-map",
        program::compose(prelude, program_source, program::Stage::Raster, parameters).source,
        checked_as_engine,
        &raster_entries,
    );
    for samples in [SampleCount::One, SampleCount::Four] {
        let head = crate::hardware_shadows::shader_source(prelude, samples).map_err(err)?;
        add(
            &mut out,
            "rt",
            if samples == SampleCount::One {
                "program-raster-hardware-1x"
            } else {
                "program-raster-hardware-4x"
            },
            program::compose(&head, program_source, program::Stage::Raster, parameters).source,
            checked_as_engine,
            &raster_entries,
        );
    }
    let fresh = fresh_shadow_gpu::shader_source(prelude).map_err(err)?;
    add(
        &mut out,
        "rt",
        "fresh-receivers",
        fresh.clone(),
        true,
        &["vs", "fs_receiver", "fresh_compute"],
    );
    add(
        &mut out,
        "rt",
        "program-raster-fresh",
        program::compose(&fresh, program_source, program::Stage::Raster, parameters).source,
        checked_as_engine,
        &raster_entries,
    );

    let rt_kernels = primary_ray_gpu::shader_sources(prelude).map_err(err)?;
    add(
        &mut out,
        "rt",
        "primary-hard",
        rt_kernels[0].clone(),
        true,
        &["primary_trace", "primary_trace_receivers"],
    );
    add(
        &mut out,
        "rt",
        "primary-soft-fresh",
        rt_kernels[1].clone(),
        true,
        &["fresh_compute"],
    );
    let modern = rt_kernels[1].contains("fn primary_retry_interactive(");
    let deferred = modern || rt_kernels[1].contains("fn primary_trace_interactive_deferred(");
    let mut interactive = vec![if deferred {
        "primary_trace_interactive_deferred"
    } else {
        "primary_trace_interactive"
    }];
    if deferred {
        interactive.push("primary_trace_interactive");
    }
    if modern {
        interactive.push("primary_retry_interactive");
    } else if deferred {
        interactive.push("primary_repair_interactive");
    }
    add(
        &mut out,
        "rt",
        "primary-soft-interactive",
        rt_kernels[1].clone(),
        true,
        &interactive,
    );
    overrides(&mut out, &[("FRESH_DIRECTIONS", 8.0)]);
    if modern {
        add(
            &mut out,
            "rt",
            "primary-edge-check",
            primary_ray_gpu::edge_check_source(&rt_kernels[1]),
            true,
            &["primary_check_interactive"],
        );
    }
    let tr1_trace =
        primary_ray_gpu::compile_structure_reconstruction_trace(&rt_kernels[1]).map_err(err)?;
    add(
        &mut out,
        "rt",
        "tr1-interactive",
        tr1_trace,
        true,
        &interactive,
    );
    overrides(&mut out, &[("FRESH_DIRECTIONS", 8.0)]);
    let film = primary_ray_gpu::film_accumulation::kernel_source(&rt_kernels[1]).map_err(err)?;
    add(
        &mut out,
        "rt",
        "film-kernels",
        film,
        true,
        &["primary_trace_interactive", "film_init"],
    );
    overrides(&mut out, &[("FRESH_DIRECTIONS", 8.0)]);

    let rt_heads = primary_ray_gpu::shading_heads(prelude).map_err(err)?;
    let (nonrt_stub_kernels, nonrt_heads) =
        primary_ray_gpu::non_rt_sources(prelude).map_err(err)?;
    for (index, source) in nonrt_stub_kernels.into_iter().enumerate() {
        let entries: &[&str] = if index == 0 {
            &["primary_trace", "primary_trace_receivers"]
        } else {
            &["fresh_compute", "primary_trace_interactive"]
        };
        add(
            &mut out,
            "nonrt",
            &format!("primary-stubs-{index}"),
            source,
            true,
            entries,
        );
    }
    for (path, heads) in [("rt", rt_heads), ("nonrt", nonrt_heads)] {
        for (index, head) in heads.iter().enumerate() {
            let band = if index == 0 { "hard" } else { "soft" };
            for (stage_name, stage) in [
                ("resident", program::Stage::Traced),
                ("streamed", program::Stage::TracedStreamed),
            ] {
                add(
                    &mut out,
                    path,
                    &format!("program-{band}-{stage_name}"),
                    program::compose(head, program_source, stage, parameters).source,
                    checked_as_engine,
                    &["primary_shade_tiles"],
                );
                add(
                    &mut out,
                    path,
                    &format!("program-list-{band}-{stage_name}"),
                    program::compose_list(head, program_source, stage, parameters).source,
                    checked_as_engine,
                    &["primary_shade_list"],
                );
            }
        }
        let film_head = primary_ray_gpu::film_accumulation::shading_head(&heads[1]).map_err(err)?;
        add(
            &mut out,
            path,
            "program-film",
            primary_ray_gpu::film_accumulation::compose_shading(
                &film_head,
                program_source,
                parameters,
            )
            .source,
            checked_as_engine,
            &["primary_shade_tiles"],
        );
        if program::reflects(program_source) {
            for mixed in [false, true] {
                add(
                    &mut out,
                    path,
                    if mixed { "gi-cell-mixed" } else { "gi-cell" },
                    gi_cache::compile_structure_cell_source(&heads[1], program_source, 64, mixed),
                    true,
                    &["gi_shade_cells"],
                );
            }
        }
        let mut gi_entries = vec![
            "gi_probe",
            "gi_claim",
            "gi_resolve",
            "gi_resolve_last",
            "gi_scan",
            "gi_setup",
            "gi_rays",
            "gi_finish",
            "gi_integrate",
            "gi_light",
            "gi_filter",
        ];
        gi_entries.push(if path == "nonrt" {
            "gi_gather_batched"
        } else {
            "gi_gather"
        });
        add(
            &mut out,
            path,
            "gi-kernels",
            gi_cache::kernel_source(path == "nonrt", 64),
            true,
            &gi_entries,
        );
        add(
            &mut out,
            path,
            "tr1-resolve",
            primary_ray_gpu::compile_structure_reconstruction_resolve(prelude).map_err(err)?,
            true,
            &["tr1_resolve"],
        );
        for (name, source) in atmosphere_gpu::compile_structure_sources(path == "rt") {
            let entries: &[&str] = match name {
                "kernels" => &[
                    "at_transmittance_lut",
                    "at_multiscatter_lut",
                    "at_skyview_lut",
                    "at_ambient_mean",
                    "at_aerial_volume",
                    "at_fog_inject",
                    "at_fog_integrate",
                    "at_environment_specular",
                    "at_environment_diffuse",
                    "at_fog_temporal",
                ],
                "sky" => &["vs_sky", "fs_sky"],
                "virtual-injection" => &["at_fog_inject"],
                _ => unreachable!(),
            };
            add(
                &mut out,
                path,
                &format!("atmosphere-{name}"),
                source,
                true,
                entries,
            );
        }
        for (name, source) in
            transparency_gpu::compile_structure_sources(prelude, program_source, path == "rt")
                .map_err(err)?
        {
            let entries: &[&str] = match name {
                "glass" => &["vs", "vs_tangent", "fs_transparent", "fs_tp_lobe"],
                "particles-sim" => &["tp_particles_update", "tp_particles_sort"],
                "particles-draw" => &["tp_particle_vs", "tp_particle_fs"],
                "hit-depth" => &["tp_depth_hits"],
                "raster-depth" => &["tp_depth_raster"],
                "shadow" => &["tp_shadow_vs"],
                "aa-seed" => &["tp_seed_vs", "tp_seed_fs"],
                "transmission" => &["tx_vs", "tx_fs"],
                "reflection" => &["tp_reflect"],
                _ => unreachable!(),
            };
            add(
                &mut out,
                path,
                &format!("transparency-{name}"),
                source,
                if name == "glass" {
                    checked_as_engine
                } else {
                    true
                },
                entries,
            );
        }
    }

    for (name, stage, entries) in [
        (
            "cull",
            include_str!("../shaders/visibility_buffer_cull.wgsl"),
            vec!["cull", "finish"],
        ),
        (
            "reset",
            include_str!("../shaders/visibility_buffer_reset.wgsl"),
            vec!["reset"],
        ),
        (
            "software-raster",
            include_str!("../shaders/visibility_buffer_sw.wgsl"),
            vec!["raster"],
        ),
        (
            "hardware-raster",
            include_str!("../shaders/visibility_buffer_raster.wgsl"),
            vec!["vs", "fs"],
        ),
    ] {
        add(
            &mut out,
            "nonrt",
            &format!("visibility-{name}"),
            visibility_buffer::compile_structure_stage_source(stage),
            true,
            &entries,
        );
    }
    add(
        &mut out,
        "nonrt",
        "visibility-hzb",
        include_str!("../shaders/visibility_buffer_hzb.wgsl").to_owned(),
        true,
        &["base", "reduce"],
    );
    let visibility_resolve = visibility_buffer::resolve_source(prelude).map_err(err)?;
    for receivers in [0.0, 1.0] {
        add(
            &mut out,
            "nonrt",
            &format!("visibility-resolve-receivers-{receivers}"),
            visibility_resolve.clone(),
            true,
            &["resolve"],
        );
        overrides(
            &mut out,
            &[
                ("WRITE_RECEIVERS", receivers),
                ("LOD_LIFT", f64::from(visibility_buffer::DEFAULT_LOD_LIFT)),
            ],
        );
    }
    for (name, source) in virtual_shadow_gpu::compile_structure_sources() {
        let entries: &[&str] = match name {
            "virtual" => &[
                "vsm_mark",
                "vsm_compact",
                "vsm_clear",
                "vsm_cull",
                "vsm_finish",
                "vsm_mip",
                "vsm_lookup",
                "vsm_horizon",
                "vsm_order_count",
                "vsm_order_place",
            ],
            "records" => &["vsm_record"],
            "hybrid" => &["vsm_lookup_queued"],
            "map" => &[
                "vmap_resolve",
                "vmap_mip_dispatch",
                "vmap_minmip",
                "vmap_queue_args",
                "vmap_lookup",
            ],
            "surface" => &[
                "vsc_probe",
                "vsc_args",
                "vsc_sort",
                "vsc_missed",
                "vsc_claim",
                "vsc_resolve",
            ],
            "s4" => &["s4_missed"],
            _ => unreachable!(),
        };
        if name == "virtual" {
            for peel in [0.0, 1.0] {
                add(
                    &mut out,
                    "nonrt",
                    &format!("virtual-shadow-raster-peel-{peel}"),
                    source.clone(),
                    true,
                    &["vsm_raster", "vs_vsm_cluster", "fs_vsm_cluster"],
                );
                overrides(&mut out, &[("VSM_PEEL_PASS", peel)]);
            }
        }
        add(
            &mut out,
            "nonrt",
            &format!("virtual-shadow-{name}"),
            source,
            true,
            entries,
        );
    }
    for (name, source) in post_gpu::compile_structure_sources().map_err(err)? {
        let entries: &[&str] = match name {
            "filters" => &[
                "downsample",
                "upsample",
                "tile_max",
                "motion_blur",
                "depth_of_field",
            ],
            "tone" => &["vs", "fs"],
            "primary-surface" => &["post_primary_surface"],
            "raster-surface-1x" | "raster-surface-4x" => &["surface_raster"],
            _ => unreachable!(),
        };
        add(
            &mut out,
            "shared",
            &format!("post-{name}"),
            source,
            true,
            entries,
        );
    }
    Ok(out)
}
