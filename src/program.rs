//! Shading programs: user WGSL `surface` and `shade` functions composed with the engine
//! prelude, the program API and a generated entry point into one module per stage and
//! visibility variant. naga checks each program before the device compiles it, so
//! diagnostics name program lines, and `struct Parameters` is reflected from naga IR.
//! See docs/programmable-shading.md.
use crate::{Error, observe::Owned};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

/// The built-in photoreal program every material uses unless it names another.
pub const STANDARD_PROGRAM: &str = include_str!("../shaders/programs/standard.wgsl");
const SPECULAR_API: &str = include_str!("../shaders/specular_filter.wgsl");
const COMMON_API: &str = include_str!("../shaders/program_api.wgsl");
// DC1: resolved decal inputs are available to every user program on every stage.
const DECAL_API: &str = include_str!("../shaders/decals.wgsl");
const RASTER_API: &str = include_str!("../shaders/program_raster.wgsl");
const RASTER_ENTRY: &str = include_str!("../shaders/program_raster_entry.wgsl");
const TRACED_API: &str = include_str!("../shaders/program_traced.wgsl");
const TRACED_ENTRY: &str = include_str!("../shaders/program_traced_entry.wgsl");
/// The traced `sample_*_grad` helpers of frames that stream textures, which also record
/// residency requests (texture_residency.rs). They replace the traced API's own.
const TRACED_RESIDENCY: &str = include_str!("../shaders/program_traced_residency.wgsl");
/// Raster materials bind their block as a uniform beside the fixed material ABI.
const RASTER_PARAMETERS: &str = "@group(2) @binding(7) var<uniform> params:Parameters;\n";
/// Traced shading reads each draw's block from a binding array and copies the hit's.
const TRACED_PARAMETERS: &str = "struct RlParameters { value:Parameters };\n@group(2) @binding(17) var<storage,read> rl_parameters:binding_array<RlParameters,PRIMARY_BATCHES>;\nvar<private> params:Parameters;\nfn rl_load_parameters(slot:u32) { params=rl_parameters[slot].value; }\n";
const TRACED_NO_PARAMETERS: &str = "fn rl_load_parameters(slot:u32) {}\n";
/// M7 indirect-light inputs (world radiance cache, then the occluded environment specular
/// of M7-ES1): composed only for programs that call them, so every other program's modules
/// are unchanged (shaders/gi_api.wgsl, shaders/environment_lobes.wgsl).
const INDIRECT_API: [&str; 3] = [
    include_str!("../shaders/gi_cells.wgsl"),
    include_str!("../shaders/gi_api.wgsl"),
    include_str!("../shaders/environment_lobes.wgsl"),
];
/// The world radiance cache's cell shading entry (M7 G2): a program's `surface` plus its
/// optional `scatter`/`emission` at each cell's query point (shaders/gi_shade_entry.wgsl).
const CELL_ENTRY: &str = include_str!("../shaders/gi_shade_entry.wgsl");
/// The M7-ES1 inputs that read the receiver's sky visibility for specular. The generated
/// entry prepares the receiver's one cache lookup before `surface` for every program that
/// calls an indirect input (`rl_indirect_prepare`: a lookup at each call site cost 0.4 ms of
/// shading on the quarry), and the specular visibility from it for programs that call one
/// of these.
const SPECULAR_INPUTS: [&str; 4] = [
    "indirect_radiance",
    "environment_lobe_count",
    "environment_lobe",
    "environment_lobe_reflection",
];
/// The engine inputs `INDIRECT_API` defines.
const INDIRECT_INPUTS: [&str; 5] = [
    "indirect_irradiance",
    "indirect_radiance",
    "environment_lobe_count",
    "environment_lobe",
    "environment_lobe_reflection",
];

// ---- M7-R1 reflections (docs/m7-r1.md) ----
/// Traced reflections for programs that declare the optional
/// `fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe` and read
/// `specular_incident(input) -> vec4<f32>` (shaders/reflections.wgsl). `RL_SCENE` names the
/// head's shadow TLAS.
const REFLECTION_API: &str = include_str!("../shaders/reflections.wgsl");
/// Every other composition of such a program: raster, cell and non-RT traced stages, and
/// programs that use only one half of the pair. Its untraced estimate stays the answer.
const REFLECTION_STUB: &str = "struct SpecularLobe { normal:vec3<f32>, alpha:f32, weight:f32 };\nfn specular_incident(input:SurfaceInput)->vec4<f32> { return vec4<f32>(0.0); }\n";
/// The traced entry traces the lobe after the receiver's `surface`, before `shade` reads it.
/// The receiver's indirect inputs are prepared after the trace, so hit shading never
/// overwrites them and the cache's prepared state is not live across the ray loop (a restore
/// after the loop cost +0.62 ms of quarry shading with the cache on). `surface` and the hits
/// read the open environment state (`rl_indirect_open`).
const REFLECTION_ENTRY_SEAM: &str =
    "    rl_indirect_prepare(surface_input);\n    let surface_output=surface(surface_input);\n";
/// U-RTVIS: in the reflection resolve's pre-pass (`rl_prepass`, RENDERLAB_RL_RESOLVE_PASS=1) the
/// entry stops after the receiver's reflection, before its indirect inputs and `shade`.
const REFLECTION_ENTRY: &str = "    rl_indirect_open();\n    let surface_output=surface(surface_input);\n    rl_reflection_prepare(surface_output,surface_input,hit.batch,id,size);\n    if rl_prepass() {return;}\n    rl_indirect_prepare(surface_input);\n";
/// Reflection hits trace their own light visibility: the frame's belongs to its receivers.
const REFLECTION_VISIBILITY_SEAM: &str =
    "    return hard_shadow(lights[s.index],input.receiver,s.direction);\n";
const REFLECTION_VISIBILITY: &str = "    if rl_reflection_hit {return rl_hit_shadow(lights[s.index],input.receiver,s.direction);}\n";
/// A shadowed local light's visibility in `light_visibility`, where reflection hits from a
/// lobe at least `LocalShadows::unshadowed_hits_from` rough skip it (the cluster frame's
/// far.z; `rl_hist_rough` is the reflecting lobe's perceptual roughness, set before its hits).
const LOCAL_VISIBILITY_SEAM: &str = "    if lights[s.index].kind.w!=0u { return rl_local_shadow(";
const HIT_LOCAL_UNSHADOWED: &str = "    if rl_reflection_hit && lights[s.index].kind.w!=0u && rl_light_frame.far.z!=0.0 && rl_hist_rough+1.0>=rl_light_frame.far.z { return 1.0; }\n";
/// U-RTVIS: a reflection hit's local-light visibility takes its share of the estimate's
/// samples. A glossy pixel averages `rl_hit_strata` hits; stratum k of them evaluates every
/// count-th disk ray (RT, local_light_ray.wgsl) or PCF tap (non-RT PCSS, light_cluster_api.wgsl)
/// from an offset turning with the frame, so the pixel's strata together cover each sample about
/// once instead of every stratum repeating all of them. One stratum (mirrors) and camera
/// receivers take every sample, as before. The blocker search, which sets the penumbra's
/// width, stays whole. `RENDERLAB_RL_SPLIT=0` keeps every sample at every hit.
const RL_SPLIT: [(&str, &str); 4] = [
    (
        "    for(var i=0u;i<samples;i+=1u) {\n",
        "    let rl_step=select(1u,min(rl_hit_strata,samples),rl_reflection_hit);\n    var visited=0u;\n    for(var i=select(0u,(rl_hit_stratum+rl_light_frame.info.z)%rl_step,rl_reflection_hit);i<samples;i+=rl_step) {\n        visited+=1u;\n",
    ),
    (
        "    return lit/f32(samples);\n",
        "    return lit/f32(visited);\n",
    ),
    (
        "        for(var k=0u;k<24u;k+=1u) {\n",
        "        let rl_step=select(1u,min(rl_hit_strata,24u),rl_reflection_hit);\n        var visited=0u;\n        for(var k=select(0u,(rl_hit_stratum+rl_light_frame.info.z)%rl_step,rl_reflection_hit);k<24u;k+=rl_step) {\n            visited+=1u;\n",
    ),
    (
        "        return soft/24.0;\n",
        "        return soft/f32(visited);\n",
    ),
];
/// U-RTVIS reflection history (shaders/reflections_history.wgsl): the wgpu renderer's primary
/// shading modules bind it (group 0, bindings 54 and 55), so only they replace the stubs.
/// Off by default (unset: today's inline full set every frame). `RENDERLAB_RL_HISTORY=1` turns on
/// the candidate, with the resolve pass (`rl_resolve_pass_on`): glossy receivers trace four strata
/// a frame into the history and rl_resolve (docs/rtvis.md).
const REFLECTION_HISTORY: &str = include_str!("../shaders/reflections_history.wgsl");
const RL_HISTORY_STUBS: [&str; 2] = ["// ---- rl history stubs ----\n", "// ---- end of rl history stubs ----\n"];
pub(crate) fn rl_history_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_HISTORY").is_ok_and(|v| v == "1"))
}
/// U-RTVIS measurement: `RENDERLAB_RL_HISTORY_TUNE=strata=S,cap=C,moving=M,sigma=K` replaces
/// reflections.wgsl's RL_HISTORY_STRATA, _CAP, _MOVING and _SIGMA.
fn rl_history_tune(mut source: String) -> String {
    static LIST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| std::env::var("RENDERLAB_RL_HISTORY_TUNE").unwrap_or_default());
    for (key, value) in list.split(',').filter_map(|s| s.trim().split_once('=')) {
        let (name, ty, text) = match key {
            "strata" => ("RL_HISTORY_STRATA", "u32", format!("{value}u")),
            "rough_strata" => ("RL_HISTORY_ROUGH_STRATA", "u32", format!("{value}u")),
            "cap" => ("RL_HISTORY_CAP", "f32", format!("{:?}", value.parse::<f32>().unwrap())),
            "moving" => ("RL_HISTORY_MOVING", "f32", format!("{:?}", value.parse::<f32>().unwrap())),
            "sigma" => ("RL_HISTORY_SIGMA", "f32", format!("{:?}", value.parse::<f32>().unwrap())),
            _ => continue,
        };
        let prefix = format!("const {name}:{ty}=");
        let at = source.find(&prefix).expect("RENDERLAB_RL_HISTORY_TUNE seam");
        let end = at + source[at..].find(';').unwrap();
        source.replace_range(at..end, &format!("{prefix}{text}"));
    }
    source
}
/// U-RTVIS: `RENDERLAB_RL_RESOLVE=1` reads the history under a moving camera too, through the
/// spatial resolve (reflections_history.wgsl RL_RESOLVE). Off by default.
pub(crate) fn rl_resolve_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| rl_history_on() && std::env::var("RENDERLAB_RL_RESOLVE").is_ok_and(|v| v == "1"))
        || rl_resolve_pass_on()
}
/// U-RTVIS: with the history on, the frame's shading shows rl_resolve's output (a pre-pass, the
/// resolve, then the shading pass; shaders/reflections_resolve.wgsl) and reads history under
/// motion too. On whenever the history is; `RENDERLAB_RL_RESOLVE_PASS=0` keeps the history to
/// still frames.
pub(crate) fn rl_resolve_pass_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| rl_history_on() && std::env::var("RENDERLAB_RL_RESOLVE_PASS").map_or(true, |v| v != "0"))
}
/// U-RTVIS reflection pass (src/reflection_pass.rs): `RENDERLAB_RL_PASS=1` writes glossy
/// receivers' lobes from primary shading (shaders/reflections_lobe.wgsl, group 0 bindings 56-58)
/// and runs `rl_trace` after it. Off by default.
const REFLECTION_LOBE: &str = include_str!("../shaders/reflections_lobe.wgsl");
const RL_PASS_STUBS: [&str; 2] = ["// ---- rl pass stubs ----\n", "// ---- end of rl pass stubs ----\n"];
const REFLECTION_TRACE: &str = include_str!("../shaders/reflections_trace.wgsl");
pub(crate) fn rl_pass_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_PASS").is_ok_and(|v| v == "1"))
}
/// `rl_trace`'s module on a traced shading `head`: the standard program's traced composition
/// (the reflection block's helpers, with its stubs) and the trace entries.
pub(crate) fn rl_trace_source(head: &str) -> String {
    let composed = compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false);
    format!("{}\n{REFLECTION_TRACE}", composed.source)
}
/// `composed` with the block between `markers` replaced by `text`, moving the program's range.
fn replace_block(mut composed: Composed, markers: [&str; 2], text: &str) -> Composed {
    let Some(begin) = composed.source.find(markers[0]) else {
        return composed;
    };
    let end = composed.source[begin..].find(markers[1]).unwrap() + begin + markers[1].len();
    assert!(end <= composed.program.start, "reflection stubs precede the program");
    composed.source.replace_range(begin..end, text);
    let shift = text.len() as isize - (end - begin) as isize;
    let moved = |at: usize| (at as isize + shift) as usize;
    composed.program = moved(composed.program.start)..moved(composed.program.end);
    composed
}
/// `composed` with the reflection history and the reflection pass's lobe in place of their
/// stubs, each while its switch is on (wgpu primary shading modules only).
pub(crate) fn with_reflection_history(composed: Composed) -> Composed {
    let composed = if rl_pass_on() {
        replace_block(composed, RL_PASS_STUBS, REFLECTION_LOBE)
    } else {
        composed
    };
    with_history_block(composed)
}
fn with_history_block(mut composed: Composed) -> Composed {
    if !rl_history_on() {
        return composed;
    }
    let Some(begin) = composed.source.find(RL_HISTORY_STUBS[0]) else {
        return composed;
    };
    let end = composed.source[begin..].find(RL_HISTORY_STUBS[1]).unwrap() + begin + RL_HISTORY_STUBS[1].len();
    assert!(end <= composed.program.start, "the history stubs precede the program");
    let history = if rl_resolve_pass_on() {
        REFLECTION_HISTORY.replacen("const RL_RESOLVE_PASS:bool=false;", "const RL_RESOLVE_PASS:bool=true;", 1)
    } else if rl_resolve_on() {
        REFLECTION_HISTORY.replacen("const RL_RESOLVE:bool=false;", "const RL_RESOLVE:bool=true;", 1)
    } else {
        REFLECTION_HISTORY.to_owned()
    };
    composed.source.replace_range(begin..end, &history);
    let shift = history.len() as isize - (end - begin) as isize;
    let moved = |at: usize| (at as isize + shift) as usize;
    composed.program = moved(composed.program.start)..moved(composed.program.end);
    composed
}
fn rl_split(mut source: String) -> String {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("RENDERLAB_RL_SPLIT").map_or(true, |v| v != "0")) {
        return source;
    }
    // Ray heads compose the disk rays, map heads the PCSS; each pair appears once or not at all.
    for pair in RL_SPLIT.chunks(2) {
        if source.matches(pair[0].0).count() == 1 && source.matches(pair[1].0).count() == 1 {
            for (from, to) in pair {
                source = source.replacen(from, to, 1);
            }
        }
    }
    source
}
/// U-RTVIS measurement switches (`RENDERLAB_RL_HIT`, a comma list), reflection hits only:
/// `local` leaves shadowed local lights unshadowed, `sun` skips the hit's sun shadow ray,
/// `nolights` gives every light zero incidence, `black` skips the hit's `surface` and `shade`,
/// `strata=N` caps a glossy pixel's strata at N (RL_GLOSSY_MAX, 16).
/// Two act on every pixel, camera receivers included, to split lights and shadows between
/// primary pixels and reflection hits: `alllights` (every light zero incidence) and
/// `allshadows` (every light fully visible). They change the picture; unset, nothing changes.
fn rl_hit_switches(source: String) -> String {
    static LIST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| std::env::var("RENDERLAB_RL_HIT").unwrap_or_default());
    let mut source = source;
    for switch in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(n) = switch.strip_prefix("strata=") {
            let from = "const RL_GLOSSY_MAX:u32=16u;";
            assert_eq!(source.matches(from).count(), 1, "RENDERLAB_RL_HIT strata");
            source = source.replacen(from, &format!("const RL_GLOSSY_MAX:u32={n}u;"), 1);
            continue;
        }
        let (from, to) = match switch {
            "local" => (
                "    if lights[s.index].kind.w!=0u { return rl_local_shadow(",
                "    if rl_reflection_hit && lights[s.index].kind.w!=0u { return 1.0; }\n    if lights[s.index].kind.w!=0u { return rl_local_shadow(",
            ),
            "sun" => (
                "    if rl_reflection_hit {return rl_hit_shadow(lights[s.index],input.receiver,s.direction);}\n",
                "    if rl_reflection_hit {return 1.0;}\n",
            ),
            "nolights" => (
                "    return LightSample(index,l,attenuation,",
                "    return LightSample(index,l,select(attenuation,0.0,rl_reflection_hit),",
            ),
            "alllights" => (
                "    return LightSample(index,l,attenuation,",
                "    return LightSample(index,l,0.0,",
            ),
            "allshadows" => (
                "fn light_visibility(input:SurfaceInput, s:LightSample) -> f32 {",
                "fn light_visibility(input:SurfaceInput, s:LightSample) -> f32 { return 1.0; }\nfn rl_unused_visibility(input:SurfaceInput, s:LightSample) -> f32 {",
            ),
            "black" => (
                "                let hs=surface(h.input);\n                radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);\n",
                "                radiance=vec3<f32>(0.0);\n",
            ),
            _ => continue,
        };
        assert_eq!(
            source.matches(from).count(),
            1,
            "RENDERLAB_RL_HIT seam {switch}"
        );
        source = source.replacen(from, to, 1);
    }
    source
}

/// The reflection API text for `program` on `head` at `stage`, and whether it traces: empty
/// for programs that name neither hook nor input, so their modules are unchanged.
/// `hits` composes the traced block whatever the program declares, for the hit-shading
/// module (`compose_hits`): any program's draw can be hit.
fn reflection_api(head: &str, program: &str, stage: Stage, hits: bool) -> (String, bool) {
    let names = tokens(program);
    let lobe = hits || names.windows(2).any(|w| w == ["fn", "reflection_lobe"]);
    let reads = hits || names.contains(&"specular_incident");
    if !lobe && !reads {
        return (String::new(), false);
    }
    // Hardware heads declare their shadow TLAS at binding 2; non-RT heads have none.
    let scene = ["hardware_scene", "fresh_scene"]
        .into_iter()
        .find(|name| head.contains(&format!("var {name}:acceleration_structure;")));
    let traced = lobe && reads && matches!(stage, Stage::Traced | Stage::TracedStreamed);
    let api = match scene.filter(|_| traced) {
        Some(scene) => REFLECTION_API.replace("RL_SCENE", scene),
        // M7-R1n: non-RT heads that declare the software geometry trace it instead.
        None if traced && head.contains(SOFTWARE_REFLECTION_GEOMETRY) => software_reflection_api(),
        None => return (REFLECTION_STUB.to_owned(), false),
    };
    let api = match full_strata_weight() {
        Some(weight) => {
            let line = format!("const RL_FULL_STRATA:f32={weight:?};");
            api.replace(FULL_STRATA, &line)
        }
        None => api,
    };
    (api, true)
}
/// The glossy strata's full-count weight as shaders/reflections.wgsl declares it.
const FULL_STRATA: &str = "const RL_FULL_STRATA:f32=0.5;";
/// `RENDERLAB_RL_FULL_STRATA` overrides the reflectance weight from which a glossy lobe traces
/// all its strata (shaders/reflections.wgsl `RL_FULL_STRATA`); 0 traces every lobe's full
/// count, as before the weight. Read once per process; for comparisons, not a setting.
fn full_strata_weight() -> Option<f32> {
    static WEIGHT: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *WEIGHT.get_or_init(|| {
        std::env::var("RENDERLAB_RL_FULL_STRATA")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|w| *w >= 0.0)
    })
}
// ---- end of M7-R1 reflections ----

// ---- MQ2 view dependence (docs/m7-mq2.md) ----
/// Programs may declare `fn view_dependence(s: Surface, input: SurfaceInput) ->
/// ViewDependence`: the share (0-1) of `shade`'s output that moves with the view direction
/// like a GGX reflection of `alpha` (any style declares its own; a view-independent style
/// declares nothing). TR1 shortens such pixels' history while their view direction turns,
/// so reflections do not trail across their surfaces. The struct is composed on every stage
/// of a declaring program; only traced shading entries call the hook.
pub(crate) const VIEW_API: &str = "struct ViewDependence { alpha:f32, share:f32 };\n";
const VIEW_HINT: &str = include_str!("../shaders/temporal_view.wgsl");
/// The traced entry's hint for programs that declare nothing: view-independent.
const VIEW_STUB: &str = "fn tr1_view_hint(s:Surface,input:SurfaceInput,pixel:vec2<u32>,size:vec2<u32>)->f32 { return 1.0; }\n";
// ---- end of MQ2 view dependence ----

// ---- M7-R1n software reflections (docs/m7-r1n.md) ----
/// The non-RT shading heads' software reflection geometry (src/reflection_software.rs).
pub(crate) const SOFTWARE_REFLECTION_GEOMETRY: &str =
    "@group(0) @binding(36) var<storage,read> rl_geometry:array<vec4<u32>>;\n";
const SOFTWARE_REFLECTION_API: &str = include_str!("../shaders/reflections_software.wgsl");
/// R1's two ray queries and their software replacements: the hit's light visibility, and
/// the reflection ray itself, which yields the same hit record. The queries' declarations
/// (`SOFTWARE_REFLECTION_QUERY`, wherever R1 declares them) are removed. The software hit keeps
/// rs_trace's result (`rs`, U-RTVIS's seam): the resolve and cache-terminated hits read its
/// distance `rs.t` where the hardware path reads `hit.t`.
const SOFTWARE_REFLECTION_SEAMS: [(&str, &str); 5] = [
    // The resolve's hit distance: the software traversal's own (`rs`, the loop seam below; the
    // cache-terminated hits can read `rs.t` the same way).
    ("            rl_dist+=hit.t;rl_dist_n+=1.0;\n", "            rl_dist+=rs.t;rl_dist_n+=1.0;\n"),
    (
        "    rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,tmin,RL_FAR,origin,d));\n    while rayQueryProceed(&query) {}\n    let hit=rayQueryGetCommittedIntersection(&query);\n    if hit.kind==0u {return RlCast(vec4<u32>(0u),-1.0);}\n    return RlCast(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),hit.t);\n",
        "    let hit=rs_trace(origin,d,tmin,RL_FAR,false);\n    return RlCast(hit.record,select(-1.0,hit.t,hit.record.x!=0u));\n",
    ),
    (
        "    rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT|RAY_FLAG_FORCE_OPAQUE,255u,receiver.epsilon,tmax,origin,l));\n    while rayQueryProceed(&query) {}\n    return select(1.0,0.0,rayQueryGetCommittedIntersection(&query).kind!=0u);\n",
        "    return select(1.0,0.0,rs_blocked(origin,l,receiver.epsilon,tmax));\n",
    ),
    (
        "        rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,receiver.epsilon,RL_FAR,origin,d));\n        while rayQueryProceed(&query) {}\n        let hit=rayQueryGetCommittedIntersection(&query);\n        // Only a miss looks up the sky: the compiler does not sink a lookup the hit\n        // branch overwrites (P-SKY, docs/p-sky.md).\n        var radiance=vec3<f32>(0.0);\n        if hit.kind!=0u {\n            let record=vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics));\n",
        "        let rs=rs_trace(origin,d,receiver.epsilon,RL_FAR,false);\n        let hit=rs.record;\n        // Only a miss looks up the sky: the compiler does not sink a lookup the hit\n        // branch overwrites (P-SKY, docs/p-sky.md).\n        var radiance=vec3<f32>(0.0);\n        if hit.x!=0u {\n            let record=hit;\n",
    ),
    (
        "            let cached=rl_cache_hit(record,origin,d,hit.t,width+spread*hit.t,rough);\n",
        "            let cached=rl_cache_hit(record,origin,d,rs.t,width+spread*rs.t,rough);\n",
    ),
];
/// Cache-terminated reflection hits (shaders/reflection_cache_hits.wgsl): a rough lobe's hit
/// whose cone covers the radiance cache's cell there reads the cell's outgoing radiance
/// instead of shading the hit. Off unless `RENDERLAB_RL_CACHE_HITS` names the minimum
/// perceptual roughness (0-1); off, every module keeps the stub and its hits shade as before.
pub(crate) const CACHE_HITS: &str = include_str!("../shaders/reflection_cache_hits.wgsl");
pub(crate) const CACHE_HITS_STUB: &str = "fn rl_cache_hit(record:vec4<u32>,origin:vec3<f32>,d:vec3<f32>,t:f32,cone:f32,rough:f32)->vec4<f32> { return vec4<f32>(0.0); }\n";
/// The `RENDERLAB_RL_CACHE_HITS` minimum roughness, read once per process.
pub(crate) fn cache_hits_rough() -> Option<f32> {
    static ROUGH: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *ROUGH.get_or_init(|| {
        std::env::var("RENDERLAB_RL_CACHE_HITS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|r| (0.0..=1.0).contains(r))
    })
}
/// R1's ray-query declaration line.
const SOFTWARE_REFLECTION_QUERY: &str = "var query:ray_query;\n";
/// R1's reflection block traced in software: the same lobe, strata, cone footprints, hit
/// reconstruction, decals and program shading, with no ray queries.
fn software_reflection_api() -> String {
    let mut api = format!("{SOFTWARE_REFLECTION_API}{REFLECTION_API}");
    assert!(
        api.contains(SOFTWARE_REFLECTION_QUERY),
        "R1 reflection seam moved"
    );
    api = api.replace(SOFTWARE_REFLECTION_QUERY, "");
    for (from, to) in SOFTWARE_REFLECTION_SEAMS {
        assert_eq!(api.matches(from).count(), 1, "R1 reflection seam moved");
        api = api.replacen(from, to, 1);
    }
    assert!(!api.contains("RL_SCENE") && !api.contains("ray_query"));
    api
}
// ---- end of M7-R1n software reflections ----

/// A world radiance cache shader's source (`name` under shaders/). Test builds read the
/// same-named file from RENDERLAB_GI_SHADERS when it exists, so a lease can screen
/// shader variants on one binary, one process per variant (docs/m7-g1b.md).
pub(crate) fn gi_shader(name: &str, builtin: &'static str) -> std::borrow::Cow<'static, str> {
    #[cfg(test)]
    if let Some(text) = std::env::var_os("RENDERLAB_GI_SHADERS")
        .and_then(|dir| std::fs::read_to_string(std::path::Path::new(&dir).join(name)).ok())
    {
        return text.into();
    }
    let _ = name;
    builtin.into()
}
fn indirect_api() -> String {
    let mut source = gi_shader("gi_cells.wgsl", INDIRECT_API[0]).into_owned();
    // `RENDERLAB_GI_KEEP=0` samples the receiver's level again for the rest term instead of
    // restoring its kept sums (gi_api.wgsl GI_KEEP_LEVEL); the same picture either way.
    static KEEP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let keep = *KEEP.get_or_init(|| std::env::var("RENDERLAB_GI_KEEP").map_or(true, |v| v != "0"));
    let api = gi_shader("gi_api.wgsl", INDIRECT_API[1]);
    if keep {
        source.push_str(&api);
    } else {
        let from = "const GI_KEEP_LEVEL:bool=true;";
        assert_eq!(api.matches(from).count(), 1, "RENDERLAB_GI_KEEP seam");
        source.push_str(&api.replacen(from, "const GI_KEEP_LEVEL:bool=false;", 1));
    }
    source.push_str(&gi_shader("environment_lobes.wgsl", INDIRECT_API[2]));
    source
}

/// Source of a shading program: WGSL without bindings that defines
/// `fn surface(input: SurfaceInput) -> Surface` and
/// `fn shade(s: Surface, input: SurfaceInput) -> vec3<f32>` (exposed linear HDR), and may
/// declare `struct Parameters` of f32, vec2/3/4<f32>, u32, i32 and fixed arrays of
/// these types, read as `params`. Arrays must satisfy WGSL uniform layout (e.g.
/// `array<vec4<f32>, 2>`). Optional `const DEFAULT_PARAMETERS: Parameters = ...`
/// supplies omitted material values. The API is shaders/program_api.wgsl plus its
/// raster and traced stages.
#[derive(Clone, Copy, Debug)]
pub struct ProgramDescriptor<'a> {
    pub label: &'a str,
    pub source: &'a str,
}

/// Owned handle to one renderer's shading program. Materials retain their program; the
/// renderer owns its pipelines and releases them once every handle has dropped.
#[derive(Clone)]
pub struct Program(pub(crate) Arc<ProgramShared>);
pub(crate) struct ProgramShared {
    pub id: u64,
    pub renderer: Arc<()>,
    pub label: String,
}
impl Program {
    pub fn label(&self) -> &str {
        &self.0.label
    }
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }
}
impl PartialEq for Program {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Program {}
impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Program({:?})", self.0.label)
    }
}

/// One `Parameters` member's value, checked against the member's name and type.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    F32(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    U32(u32),
    I32(i32),
    /// A fixed array, checked for element type and exact length. For scalar
    /// thresholds, pack four per vec4 to satisfy WGSL's uniform array stride.
    Array(Vec<Value>),
}
impl Value {
    fn matches(&self, kind: &Kind) -> bool {
        match (self, kind) {
            (Self::F32(_), Kind::F32)
            | (Self::Vec2(_), Kind::Vec2)
            | (Self::Vec3(_), Kind::Vec3)
            | (Self::Vec4(_), Kind::Vec4)
            | (Self::U32(_), Kind::U32)
            | (Self::I32(_), Kind::I32) => true,
            (Self::Array(values), Kind::Array { element, len, .. }) => {
                values.len() == *len as usize && values.iter().all(|v| v.matches(element))
            }
            _ => false,
        }
    }
    fn write(&self, kind: &Kind, out: &mut [u8]) {
        if let (
            Self::Array(values),
            Kind::Array {
                element, stride, ..
            },
        ) = (self, kind)
        {
            for (value, dst) in values.iter().zip(out.chunks_mut(*stride as usize)) {
                value.write(element, dst);
            }
            return;
        }
        let words: Vec<[u8; 4]> = match self {
            Self::F32(v) => vec![v.to_ne_bytes()],
            Self::Vec2(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::Vec3(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::Vec4(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::U32(v) => vec![v.to_ne_bytes()],
            Self::I32(v) => vec![v.to_ne_bytes()],
            Self::Array(_) => unreachable!("array type checked before packing"),
        };
        for (dst, word) in out.chunks_exact_mut(4).zip(words) {
            dst.copy_from_slice(&word);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    F32,
    Vec2,
    Vec3,
    Vec4,
    U32,
    I32,
    Array {
        element: Box<Kind>,
        len: u32,
        stride: u32,
    },
}
impl Kind {
    fn size(&self) -> usize {
        match self {
            Self::F32 | Self::U32 | Self::I32 => 4,
            Self::Vec2 => 8,
            Self::Vec3 => 12,
            Self::Vec4 => 16,
            Self::Array { len, stride, .. } => (*len * *stride) as usize,
        }
    }
}

/// Bytes of every material's parameter block; `Parameters` may use at most this many.
pub(crate) const PARAMETER_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub name: String,
    pub kind: Kind,
    pub offset: u32,
}

/// A program's reflected `struct Parameters`, in member order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParameterLayout {
    pub fields: Vec<Field>,
    pub defaults: [u8; PARAMETER_BYTES],
}
impl ParameterLayout {
    /// Every named value must name a member of the same type.
    pub fn check<'a>(
        layout: Option<&Self>,
        values: impl IntoIterator<Item = (&'a str, Value)>,
    ) -> Result<(), Error> {
        for (name, value) in values {
            let field = layout
                .and_then(|l| l.fields.iter().find(|f| f.name == name))
                .ok_or(Error::InvalidInput(
                    "material parameter is not a member of its program's Parameters",
                ))?;
            if !value.matches(&field.kind) {
                return Err(Error::InvalidInput(
                    "material parameter type differs from its Parameters member",
                ));
            }
        }
        Ok(())
    }
    /// Program defaults, then explicit overrides at naga-reflected offsets/strides.
    /// Values naming no member of the same type are skipped.
    pub fn pack(layout: Option<&Self>, values: &[(String, Value)]) -> [u8; PARAMETER_BYTES] {
        let mut block = layout.map_or([0; PARAMETER_BYTES], |l| l.defaults);
        for (name, value) in values {
            if let Some(f) = layout
                .and_then(|l| l.fields.iter().find(|f| f.name == *name))
                .filter(|f| value.matches(&f.kind))
            {
                let offset = f.offset as usize;
                // Explicit arrays also clear any default padding bytes.
                block[offset..offset + f.kind.size()].fill(0);
                value.write(&f.kind, &mut block[offset..offset + f.kind.size()]);
            }
        }
        block
    }
    /// New or retyped members without a nonzero program default.
    pub fn changed(previous: Option<&Self>, next: Option<&Self>) -> Vec<String> {
        let Some(next) = next else {
            return vec![];
        };
        next.fields
            .iter()
            .filter(|f| {
                next.defaults[f.offset as usize..f.offset as usize + f.kind.size()]
                    .iter()
                    .all(|&b| b == 0)
                    && !previous.is_some_and(|p| {
                        p.fields
                            .iter()
                            .any(|o| o.name == f.name && o.kind == f.kind)
                    })
            })
            .map(|f| f.name.clone())
            .collect()
    }
}

/// One material's parameter values and GPU block, repacked by name when its program
/// reloads with a different layout.
pub(crate) struct MaterialParameters {
    pub buffer: Owned<wgpu::Buffer>,
    pub values: Mutex<Vec<(String, Value)>>,
}

/// The stage a composition targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Raster,
    Traced,
    /// Traced, with the residency-recording sampling helpers (`TRACED_RESIDENCY`).
    TracedStreamed,
    /// The world radiance cache's cell shading (M7 G2): the traced material ABI and the
    /// cache definitions with `CELL_ENTRY`, for programs that `reflects`.
    Cell,
}

/// The traced API with its `sample_*_grad` helpers, from the base-colour one to the DFG
/// helper that follows them, replaced by `TRACED_RESIDENCY`. `Stage::Traced` composes
/// the traced API unchanged, so default shading contains no residency code.
fn traced_streamed_api() -> String {
    let start = TRACED_API.find("fn sample_base_color_grad(").unwrap();
    let end = TRACED_API.find("// The DFG table has one level").unwrap();
    format!(
        "{}{TRACED_RESIDENCY}{}",
        &TRACED_API[..start],
        &TRACED_API[end..]
    )
}

/// The traced entry's hit-record helpers (`PrimaryHit`, `primary_decode`, `primary_input`),
/// everything before its `primary_vertex`. A traced head with RT1d's shadow visibility filter
/// calls them, so a cell module composed on that head defines them too; cell shading itself
/// never runs the filter (its lights come from the cache's light pass).
fn traced_hit_helpers() -> &'static str {
    &TRACED_ENTRY[..TRACED_ENTRY.find("fn primary_vertex(").unwrap()]
}

/// A composed module and the bytes its program occupies.
pub(crate) struct Composed {
    pub source: String,
    pub program: Range<usize>,
}

/// One module: `head` (the prelude with its visibility variant and, for traced stages,
/// the ray helpers), the shared API, the stage API, the parameter binding when
/// `parameters`, the program, then the generated entry point.
pub(crate) fn compose(head: &str, program: &str, stage: Stage, parameters: bool) -> Composed {
    compose_stage(head, program, stage, parameters, false)
}
/// `compose`, or with `hits` the hit-shading module (`compose_hits`).
fn compose_stage(
    head: &str,
    program: &str,
    stage: Stage,
    parameters: bool,
    hits: bool,
) -> Composed {
    let streamed;
    let cell_entry;
    let (api, binding, entry) = match stage {
        Stage::Raster => (
            RASTER_API,
            if parameters { RASTER_PARAMETERS } else { "" },
            RASTER_ENTRY,
        ),
        Stage::Traced | Stage::TracedStreamed | Stage::Cell => (
            if stage == Stage::TracedStreamed {
                streamed = traced_streamed_api();
                &streamed
            } else {
                TRACED_API
            },
            if parameters {
                TRACED_PARAMETERS
            } else {
                TRACED_NO_PARAMETERS
            },
            if stage == Stage::Cell {
                cell_entry = gi_shader("gi_shade_entry.wgsl", CELL_ENTRY);
                &*cell_entry
            } else {
                TRACED_ENTRY
            },
        ),
    };
    let indirect_calls = uses_indirect(program);
    // Cell shading reads the cache's own definitions whatever the program calls.
    let mut indirect = if indirect_calls || stage == Stage::Cell {
        indirect_api()
    } else {
        String::new()
    };
    // Each entry calls this once per receiver before `surface`: the receiver's one cache
    // lookup for programs that call an indirect input, and the specular visibility from it
    // for those that read the specular inputs.
    let specular = tokens(program).iter().any(|t| SPECULAR_INPUTS.contains(t));
    indirect.push_str(match (indirect_calls, specular) {
        (true, true) => {
            "fn rl_indirect_prepare(input:SurfaceInput) { gi_prepare(input); es_prepare(input); }\n"
        }
        (true, false) => "fn rl_indirect_prepare(input:SurfaceInput) { gi_prepare(input); }\n",
        _ => "fn rl_indirect_prepare(input:SurfaceInput) {}\n",
    });
    // LC1: the light loop's clusters and local shadows, traced where the head binds rays.
    // Cell shading never calls light_visibility; it composes the map variant.
    let lights = crate::light_cluster::shading_api(
        if stage == Stage::Cell { "" } else { head },
        matches!(stage, Stage::Traced | Stage::TracedStreamed),
    );
    // AT1: camera-pixel entries apply the sky and fog (shaders/atmosphere_api.wgsl); cell
    // shading's bounce light carries none.
    let atmosphere = if stage == Stage::Cell {
        String::new()
    } else {
        crate::atmosphere::api_source()
    };
    let (mut reflection, traces) = reflection_api(head, program, stage, hits);
    // The hit module of a program that traces no lobe: its own pixels never trace, but the
    // reflection block names the hook.
    if hits && !declares(program, "fn", "reflection_lobe") {
        reflection.push_str(REFLECTION_LOBE_STUB);
    }
    if traces {
        // M7-R1: the indirect inputs' cache-off state, which reads the open environment.
        reflection.push_str(match (indirect_calls, specular) {
            (true, true) => {
                "fn rl_indirect_open() { gi_rx_valid=false; es_visibility=vec3<f32>(1.0); }\n"
            }
            (true, false) => "fn rl_indirect_open() { gi_rx_valid=false; }\n",
            _ => "fn rl_indirect_open() {}\n",
        });
        // Hardware and software traces alike (the software seam passes rs_trace's distance).
        reflection.push_str(&match cache_hits_rough().filter(|_| indirect_calls) {
            Some(rough) => CACHE_HITS.replace("RL_CACHE_ROUGH_MIN", &format!("{rough:?}")),
            None => CACHE_HITS_STUB.to_owned(),
        });
    }
    let view = declares(program, "fn", "view_dependence");
    let view_api = if view { VIEW_API } else { "" };
    // TP2: light_sample filters incident sun RGB through transparent caster layers.
    let transmission = super::renderer::transparency_gpu::transmission_shadow::api_source();
    let mut source = format!(
        "{head}\n{COMMON_API}{transmission}{lights}{DECAL_API}{SPECULAR_API}{api}{indirect}{reflection}{atmosphere}{binding}{view_api}// ---- program ----\n"
    );
    if traces {
        // M7-R1: hits trace their own light visibility.
        assert_eq!(source.matches(REFLECTION_VISIBILITY_SEAM).count(), 1);
        source = source.replacen(
            REFLECTION_VISIBILITY_SEAM,
            &format!("{REFLECTION_VISIBILITY}{REFLECTION_VISIBILITY_SEAM}"),
            1,
        );
        // `LocalShadows::unshadowed_hits_from`: rough enough hits light local lights unshadowed.
        assert_eq!(source.matches(LOCAL_VISIBILITY_SEAM).count(), 1);
        source = source.replacen(
            LOCAL_VISIBILITY_SEAM,
            &format!("{HIT_LOCAL_UNSHADOWED}{LOCAL_VISIBILITY_SEAM}"),
            1,
        );
        source = rl_history_tune(rl_split(rl_hit_switches(source)));
    }
    let start = source.len();
    source.push_str(program);
    let program_range = start..source.len();
    source.push_str("\n// ---- end of program ----\n");
    // MQ2: the traced entry stores the program's view dependence for TR1.
    if matches!(stage, Stage::Traced | Stage::TracedStreamed) {
        source.push_str(if view { VIEW_HINT } else { VIEW_STUB });
    }
    if stage == Stage::Cell {
        source.push_str(&reflection_wrappers(program));
        if head.contains("primary_decode(") {
            source.push_str(traced_hit_helpers());
        }
    }
    if hits {
        // The stage's entry as written (its decode helpers are the hits' too; its own entry
        // point goes unused), then the hit entry.
        source.push_str(entry);
        source.push_str(crate::renderer::reflection_hits::ENTRY);
    } else if traces {
        // M7-R1: the lobe is traced between the receiver's `surface` and `shade`.
        assert_eq!(entry.matches(REFLECTION_ENTRY_SEAM).count(), 1);
        source.push_str(&entry.replacen(REFLECTION_ENTRY_SEAM, REFLECTION_ENTRY, 1));
        if rl_deferred() && declares(program, "const", "RL_DEFERRED") {
            return deferred_variant(Composed {
                source,
                program: program_range,
            });
        }
    } else {
        source.push_str(entry);
    }
    Composed {
        source,
        program: program_range,
    }
}

// ---- Deferred reflections (the program side; U-RTVIS's composite reads them) ----
/// Whether traced shading writes the deferred reflection composite's inputs:
/// `RENDERLAB_RL_DEFERRED=1`, off by default. Read once per process.
pub(crate) fn rl_deferred() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_DEFERRED").is_ok_and(|v| v == "1"))
}
/// The program API's report, which deferred modules replace with `DEFERRED_REPORT`.
const DEFERRED_REPORT_STUB: &str =
    "fn reflection_report(reflectance:vec3<f32>, untraced:vec3<f32>) {}";
const DEFERRED_API: &str = include_str!("../shaders/reflections_deferred.wgsl");
/// The traced entry's pixel, where a deferred module first clears its outputs.
const DEFERRED_CLEAR_SEAM: &str =
    "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);";
/// A traced shading module (`compose` output) of a program that declares `const
/// RL_DEFERRED`, made to write the deferred composite's inputs: the report keeps the
/// program's reflectance and untraced specular, and the entry writes them through the pixel's
/// fog and aerial perspective, with the traced fraction (binding 62: reflectance rgb and w;
/// 63: untraced specular). Every pixel the entry visits starts cleared, so misses and culled
/// faces leave zero. The program's range moves with the removed report stub, which precedes
/// it (temporal shading splits the module at the program's end).
pub(crate) fn deferred_variant(composed: Composed) -> Composed {
    let Composed { source, program } = composed;
    for seam in [DEFERRED_REPORT_STUB, DEFERRED_CLEAR_SEAM, VIEW_STORE_SEAM] {
        assert_eq!(source.matches(seam).count(), 1, "deferred seam {seam}");
    }
    assert!(
        source.find(DEFERRED_REPORT_STUB).unwrap() < program.start,
        "the report stub precedes the program"
    );
    let shift = DEFERRED_REPORT_STUB.len();
    let mut source = source
        .replacen(DEFERRED_REPORT_STUB, "", 1)
        .replacen(
            DEFERRED_CLEAR_SEAM,
            &format!("{DEFERRED_CLEAR_SEAM}\n    rl_deferred_clear(pixel);"),
            1,
        )
        .replacen(
            VIEW_STORE_SEAM,
            &format!("{VIEW_STORE_SEAM}\n    rl_deferred_store(pixel,input.world);"),
            1,
        );
    source.push_str(DEFERRED_API);
    Composed {
        source,
        program: program.start - shift..program.end - shift,
    }
}

// ---- View modes (docs/design-id-output.md) ----
/// The traced entry's lit store, which a view-mode module replaces.
const VIEW_STORE_SEAM: &str = "textureStore(primary_hdr,pixel,vec4<f32>(atmosphere_surface(rl_finite(shade(surface_output,surface_input)),input.world),tr1_view_hint(surface_output,surface_input,id,size)));";
const VIEW_STORE: &str = "textureStore(primary_hdr,pixel,rl_view_store(surface_output,surface_input));";
/// Data modes show misses as zero; the lit miss is kept under another name.
const VIEW_MISS_SEAM: &str = "fn primary_miss(id:vec2<u32>,size:vec2<u32>,clear:vec4<f32>)->vec4<f32> {";
const VIEW_MISS: &str = include_str!("../shaders/program_view.wgsl");

/// A traced or list shading module of `program` (`compose` or `compose_list` output) that
/// stores `mode`'s value instead of the lit colour: the shading-stage switch of a view mode.
/// Everything before the store (the receiver, `surface`, the engine's prepared inputs) is
/// the lit module's. `LightingOnly` keeps the lit miss; data modes store zero there.
pub(crate) fn view_variant(
    composed: &str,
    program: &str,
    mode: crate::ViewMode,
) -> Result<String, Error> {
    let seams = [VIEW_STORE_SEAM, VIEW_MISS_SEAM].map(|s| composed.matches(s).count());
    if seams != [1, 1] {
        return Err(Error::Shader(format!(
            "view mode {mode:?}: the traced entry's store or miss changed ({seams:?} matches)"
        )));
    }
    let data = if declares(program, "fn", "view_data") {
        "return view_data(s,input);".to_owned()
    } else {
        let emission = if declares(program, "fn", "emission") {
            "emission(s,input)"
        } else {
            "vec3<f32>(0.0)"
        };
        format!(
            "let m=input.material;let base=sample_base_color(input);let mr=sample_metallic_roughness(input);\n    return ViewData(clamp(m.base_color.rgb*input.color.rgb*base.rgb,vec3<f32>(0.0),vec3<f32>(1.0)),material_normal(input),clamp(m.roughness*mr.g,0.0,1.0),clamp(m.metallic*mr.b,0.0,1.0),{emission},1.0);"
        )
    };
    // Programs without the indirect inputs light from the open environment.
    let ambient = if uses_indirect(program) {
        "indirect_irradiance(input,n)"
    } else {
        "ambient(n)"
    };
    let mut source = composed
        .replacen(VIEW_STORE_SEAM, VIEW_STORE, 1)
        .replacen(VIEW_MISS_SEAM, &VIEW_MISS_SEAM.replace("primary_miss", "rl_lit_miss"), 1);
    source.push_str(&format!(
        "\nconst RL_VIEW_MODE:u32={}u;\nfn rl_view_data(s:Surface,input:SurfaceInput)->ViewData {{\n    {data}\n}}\nfn rl_view_ambient(n:vec3<f32>,input:SurfaceInput)->vec3<f32> {{ return {ambient}; }}\n{VIEW_MISS}",
        mode as u32
    ));
    Ok(source)
}
// ---- end of view modes ----

// ---- Reflection hit shading (docs/design-reflection-pass.md) ----
/// `reflection_lobe` for a program that declares none: no lobe.
const REFLECTION_LOBE_STUB: &str = "fn reflection_lobe(s:Surface,input:SurfaceInput)->SpecularLobe { return SpecularLobe(vec3<f32>(0.0,0.0,1.0),0.0,0.0); }\n";
/// The hit-shading module of `program` on a traced stage: `compose`'s text with the reflection
/// block composed whatever the program declares, and the `reflection_hits` entry
/// (shaders/reflection_hits.wgsl) after the stage's entry, whose decode helpers it shares.
/// Hits run the same hit code as the inline path: `rl_hit_input`, the program's `surface`
/// and `shade`, its lights.
/// With `lists` (a mixed-program frame, shaded per program on MP1's list ABI) the module
/// also declares MP1's slot uniform and keeps only the hits on its own slot's draws.
pub(crate) fn compose_hits(
    head: &str,
    program: &str,
    stage: Stage,
    parameters: bool,
    lists: bool,
) -> Composed {
    debug_assert!(matches!(stage, Stage::Traced | Stage::TracedStreamed));
    let mut composed = compose_stage(head, program, stage, parameters, true);
    if lists {
        assert_eq!(
            composed.source.matches(HITS_MINE).count(),
            1,
            "reflection_hits seam"
        );
        composed.source = composed.source.replacen(HITS_MINE, HITS_MINE_LIST, 1);
    }
    composed
}
/// shaders/reflection_hits.wgsl's per-program filter, and its mixed-frame replacement: the
/// hit's draw's frame slot against this dispatch's (program_traced_list.wgsl RlList, at
/// binding 38 with a dynamic offset per slot).
const HITS_MINE: &str = "fn rl_hits_mine(placement:u32)->bool { return true; }\n";
const HITS_MINE_LIST: &str = "struct RlList { slot:u32, base:u32, capacity:u32, flags:u32, programs:array<vec4<u32>,PRIMARY_BATCHES/4> };\n@group(0) @binding(38) var<uniform> rl_list:RlList;\nfn rl_hits_mine(placement:u32)->bool { let batch=primary_placements[placement]&0x1fffffffu; return rl_list.programs[batch>>2u][batch&3u]==rl_list.slot; }\n";
// ---- end of reflection hit shading ----

// ---- MP1 per-program dispatch (docs/m7-mp1.md) ----
/// The tile-list shading entry of mixed-program frames (`primary_shade_list`).
const TRACED_LIST_ENTRY: &str = include_str!("../shaders/program_traced_list.wgsl");
/// R1's hit shading loads the hit's parameter block, which belongs to the hit's program.
// TP2 integration: match the call independently of LF/CRLF checkout endings.
const LIST_HIT_PARAMETERS_SEAM: &str = "rl_load_parameters(h.batch);";
/// In a mixed frame a reflection hit on another program's draw is shaded by this module's
/// program with the hit's material inputs and the receiver's own parameter block: the
/// recorded fallback (docs/m7-gaps.md, Programs), since another program's code is not in
/// this module.
const LIST_HIT_PARAMETERS: &str =
    "rl_load_parameters(select(batch,h.batch,rl_slot_of(h.batch)==rl_list.slot));";

/// `compose` for the traced stages plus the tile-list entry of mixed-program frames. The
/// single-program modules are composed by `compose` alone, so their text (and code) is
/// unchanged by MP1.
pub(crate) fn compose_list(head: &str, program: &str, stage: Stage, parameters: bool) -> Composed {
    debug_assert!(matches!(stage, Stage::Traced | Stage::TracedStreamed));
    let Composed {
        mut source,
        program: range,
    } = compose(head, program, stage, parameters);
    // The reflection block precedes the program; shift the program's range past the edit.
    let (engine, rest) = source.split_at(range.start);
    let edited = engine.replacen(LIST_HIT_PARAMETERS_SEAM, LIST_HIT_PARAMETERS, 1);
    let shift = edited.len() - engine.len();
    source = format!("{edited}{rest}{TRACED_LIST_ENTRY}");
    Composed {
        source,
        program: range.start + shift..range.end + shift,
    }
}
// ---- end of MP1 per-program dispatch ----

/// Whether `program` declares the optional bounce-light hooks (M7 G2): `fn scatter(s:
/// Surface, input: SurfaceInput, wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32>`, the radiance it
/// sends toward `wo` per unit radiance arriving from `wi` (its BRDF times the cosine at `wi`),
/// and `fn emission(s: Surface, input: SurfaceInput) -> vec3<f32>`, its own radiance. A program
/// that declares neither still blocks light in the world radiance cache but reflects none.
pub(crate) fn reflects(program: &str) -> bool {
    declares(program, "fn", "scatter") || declares(program, "fn", "emission")
}
/// Cell shading's calls into the hooks, zero for an absent one.
fn reflection_wrappers(program: &str) -> String {
    let scatter = if declares(program, "fn", "scatter") {
        "return scatter(s,input,wi,wo);"
    } else {
        "return vec3<f32>(0.0);"
    };
    let emission = if declares(program, "fn", "emission") {
        "return emission(s,input);"
    } else {
        "return vec3<f32>(0.0);"
    };
    format!(
        "fn rl_scatter(s:Surface,input:SurfaceInput,wi:vec3<f32>,wo:vec3<f32>)->vec3<f32> {{ {scatter} }}\nfn rl_emission(s:Surface,input:SurfaceInput)->vec3<f32> {{ {emission} }}\n"
    )
}

/// The program's identifiers in order, without comments (WGSL block comments nest).
fn tokens<'a>(source: &'a str) -> Vec<&'a str> {
    let bytes = source.as_bytes();
    let (mut out, mut i, mut depth, mut start) = (vec![], 0, 0usize, None);
    let flush = |out: &mut Vec<&'a str>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            out.push(&source[s..end]);
        }
    };
    while i < bytes.len() {
        let pair = &bytes[i..bytes.len().min(i + 2)];
        if pair == b"/*" {
            flush(&mut out, &mut start, i);
            depth += 1;
            i += 2;
        } else if depth > 0 && pair == b"*/" {
            depth -= 1;
            i += 2;
        } else if depth > 0 {
            i += 1;
        } else if pair == b"//" {
            flush(&mut out, &mut start, i);
            i += source[i..].find('\n').unwrap_or(source.len() - i);
        } else if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            start.get_or_insert(i);
            i += 1;
        } else {
            flush(&mut out, &mut start, i);
            i += 1;
        }
    }
    flush(&mut out, &mut start, bytes.len());
    out
}
fn declares(source: &str, keyword: &str, name: &str) -> bool {
    tokens(source).windows(2).any(|w| w == [keyword, name])
}
/// Whether `program` names an indirect-light engine input outside comments; those
/// programs get the cache inputs and make frames run the cache passes.
pub(crate) fn uses_indirect(program: &str) -> bool {
    tokens(program)
        .into_iter()
        .any(|t| INDIRECT_INPUTS.contains(&t))
}
/// Whether `program` declares `struct Parameters`, which composition binds as `params`.
/// TP1: whether `program` declares the transparency hook `fn transmittance(s: Surface,
/// input: SurfaceInput) -> vec3<f32>` (docs/m7-tp1.md).
pub(crate) fn declares_transmittance(program: &str) -> bool {
    declares(program, "fn", "transmittance")
}
/// Whether `program` declares `const FILMS`: for a transparent material with an index of
/// refraction of one and no thickness (a film: Ara's glTF BLEND), its `transmittance` is
/// exactly `1 - coverage` in every channel and its `shade` is already scaled by coverage, so
/// the engine draws such materials with hardware blending over what is behind them instead
/// of reading a copy of it (docs/api.md). Programs without it keep the copy path.
pub(crate) fn declares_films(program: &str) -> bool {
    static COPY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // `RENDERLAB_FILMS=copy` draws films on the copy path, as before (A/B).
    let copy = *COPY.get_or_init(|| std::env::var("RENDERLAB_FILMS").is_ok_and(|v| v == "copy"));
    !copy && declares(program, "const", "FILMS") && declares_transmittance(program)
}
// TP2: the forward glass pass supplies the declared lobe to R1 separately.
pub(crate) fn declares_reflection(program: &str) -> bool {
    declares(program, "fn", "reflection_lobe") && tokens(program).contains(&"specular_incident")
}
// TP2 traces a separately rasterized glass lobe. Opaque hits still run their own
// program, even when that program never asks for reflections of its own.
pub(crate) fn glass_hit_program(program: &str) -> String {
    let mut source = program.to_owned();
    if !declares(program, "fn", "reflection_lobe") {
        source.push_str("\nfn reflection_lobe(s:Surface,input:SurfaceInput)->SpecularLobe {return SpecularLobe(input.geometric,0.0,0.0);}\n");
    }
    source.push_str("\nfn rl_glass_requires_incident(input:SurfaceInput)->vec4<f32> {return specular_incident(input);}\n");
    source
}
pub(crate) fn declares_parameters(program: &str) -> bool {
    declares(program, "struct", "Parameters")
}

/// A diagnostic naming the program line when `offset` lies inside it.
fn diagnostic(
    label: &str,
    composed: &Composed,
    location: Option<naga::SourceLocation>,
    message: &str,
) -> Error {
    let first = composed.source[..composed.program.start]
        .matches('\n')
        .count() as u32
        + 1;
    Error::Shader(match location {
        Some(l) if composed.program.contains(&(l.offset as usize)) => format!(
            "program `{label}` line {}:{}: {message}",
            l.line_number - first + 1,
            l.line_position
        ),
        Some(l) => format!(
            "program `{label}`, engine composition line {}: {message}",
            l.line_number
        ),
        None => format!("program `{label}`: {message}"),
    })
}
fn in_program(composed: &Composed, span: naga::Span) -> bool {
    span.to_range()
        .is_some_and(|r| composed.program.contains(&r.start))
}

/// What the engine reads from a valid program: its `Parameters` layout, if declared.
#[derive(Debug)]
pub(crate) struct Analysis {
    pub parameters: Option<ParameterLayout>,
}

/// Checks a program's raster composition with naga before any device compile: both
/// functions, syntax, types and validation with program-line diagnostics, no program
/// bindings, and the reflected `Parameters` block.
pub(crate) fn analyse(label: &str, program: &str, composed: &Composed) -> Result<Analysis, Error> {
    for name in ["surface", "shade"] {
        if !declares(program, "fn", name) {
            return Err(Error::Shader(format!(
                "program `{label}` must define fn {name}; see shaders/programs/standard.wgsl"
            )));
        }
    }
    let source = &composed.source;
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| diagnostic(label, composed, e.location(source), e.message()))?;
    for (handle, global) in module.global_variables.iter() {
        let span = module.global_variables.get_span(handle);
        if global.binding.is_some() && in_program(composed, span) {
            return Err(diagnostic(
                label,
                composed,
                Some(span.location(source)),
                "programs cannot declare bindings; declare struct Parameters and use the engine services",
            ));
        }
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| {
        let mut message = e.as_inner().to_string();
        let mut cause = std::error::Error::source(e.as_inner());
        while let Some(next) = cause {
            message = format!("{message}: {next}");
            cause = next.source();
        }
        let location = e
            .spans()
            .find(|(s, _)| in_program(composed, *s))
            .map(|(s, _)| s.location(source))
            .or_else(|| match e.as_inner() {
                naga::valid::ValidationError::GlobalVariable { handle, .. } => {
                    let span = module.types.get_span(module.global_variables[*handle].ty);
                    in_program(composed, span).then(|| span.location(source))
                }
                _ => None,
            })
            .or_else(|| e.location(source));
        diagnostic(label, composed, location, &message)
    })?;
    // The optional bounce hooks must match cell shading's calls (M7 G2).
    if reflects(program) {
        let checked = format!("{source}\n{}", reflection_wrappers(program));
        let valid = naga::front::wgsl::parse_str(&checked)
            .map_err(|e| e.message().to_string())
            .and_then(|m| {
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&m)
                .map(|_| ())
                .map_err(|e| e.as_inner().to_string())
            });
        if let Err(message) = valid {
            return Err(Error::Shader(format!(
                "program `{label}`: bounce light calls fn scatter(s: Surface, input: SurfaceInput, wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32> and fn emission(s: Surface, input: SurfaceInput) -> vec3<f32>: {message}"
            )));
        }
    }
    let parameters = module
        .types
        .iter()
        .find(|(h, ty)| {
            ty.name.as_deref() == Some("Parameters")
                && in_program(composed, module.types.get_span(*h))
        })
        .map(|(handle, ty)| {
            reflect(&module, handle, ty).map_err(|message| {
                diagnostic(
                    label,
                    composed,
                    Some(module.types.get_span(handle).location(source)),
                    message,
                )
            })
        })
        .transpose()?;
    Ok(Analysis { parameters })
}

fn reflect(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
    ty: &naga::Type,
) -> Result<ParameterLayout, &'static str> {
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        return Err("Parameters must be a struct");
    };
    if *span as usize > PARAMETER_BYTES {
        return Err("Parameters may use at most 256 bytes");
    }
    let fields = members
        .iter()
        .map(|m| {
            let kind = reflect_kind(module, m.ty)?;
            Ok(Field {
                name: m.name.clone().unwrap_or_default(),
                kind,
                offset: m.offset,
            })
        })
        .collect::<Result<_, _>>()?;
    let mut defaults = [0; PARAMETER_BYTES];
    if let Some((_, constant)) = module
        .constants
        .iter()
        .find(|(_, c)| c.name.as_deref() == Some("DEFAULT_PARAMETERS"))
    {
        if constant.ty != handle {
            return Err("DEFAULT_PARAMETERS must be a const of type Parameters");
        }
        write_constant(module, constant.init, &mut defaults)?;
    }
    Ok(ParameterLayout { fields, defaults })
}

fn reflect_kind(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Result<Kind, &'static str> {
    Ok(match module.types[ty].inner {
        naga::TypeInner::Scalar(s) if s == naga::Scalar::F32 => Kind::F32,
        naga::TypeInner::Scalar(s) if s == naga::Scalar::U32 => Kind::U32,
        naga::TypeInner::Scalar(s) if s == naga::Scalar::I32 => Kind::I32,
        naga::TypeInner::Vector { size, scalar } if scalar == naga::Scalar::F32 => match size {
            naga::VectorSize::Bi => Kind::Vec2,
            naga::VectorSize::Tri => Kind::Vec3,
            naga::VectorSize::Quad => Kind::Vec4,
        },
        naga::TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(len),
            stride,
        } => {
            let element = reflect_kind(module, base)?;
            if matches!(element, Kind::Array { .. }) {
                return Err("Parameters supports one-dimensional fixed arrays only");
            }
            Kind::Array {
                element: Box::new(element),
                len: len.get(),
                stride,
            }
        }
        _ => {
            return Err(
                "Parameters members must be f32, vec2/3/4<f32>, u32, i32 or fixed arrays of these",
            );
        }
    })
}

/// WGSL's frontend evaluates const expressions; pack that IR, never parse WGSL values
/// or reimplement its expression evaluator. Struct offsets and array strides come
/// from the same naga types that validate both GPU bindings.
fn write_constant(
    module: &naga::Module,
    init: naga::Handle<naga::Expression>,
    out: &mut [u8],
) -> Result<usize, &'static str> {
    use naga::{Expression as E, Literal as L, TypeInner as T};
    Ok(match &module.global_expressions[init] {
        E::Constant(c) => write_constant(module, module.constants[*c].init, out)?,
        E::ZeroValue(ty) => {
            let size = module.types[*ty].inner.size(module.to_ctx()) as usize;
            out[..size].fill(0);
            size
        }
        E::Literal(l) => {
            out[..4].copy_from_slice(&match l {
                L::F32(v) => v.to_ne_bytes(),
                L::U32(v) => v.to_ne_bytes(),
                L::I32(v) => v.to_ne_bytes(),
                _ => return Err("unsupported DEFAULT_PARAMETERS scalar"),
            });
            4
        }
        E::Splat { size, value } => {
            for dst in out.chunks_exact_mut(4).take(*size as usize) {
                write_constant(module, *value, dst)?;
            }
            *size as usize * 4
        }
        E::Compose { ty, components } => {
            match &module.types[*ty].inner {
                T::Struct { members, .. } => {
                    for (member, value) in members.iter().zip(components) {
                        write_constant(module, *value, &mut out[member.offset as usize..])?;
                    }
                }
                T::Array { stride, .. } => {
                    for (value, dst) in components.iter().zip(out.chunks_mut(*stride as usize)) {
                        write_constant(module, *value, dst)?;
                    }
                }
                T::Vector { .. } => {
                    let mut offset = 0;
                    for value in components {
                        offset += write_constant(module, *value, &mut out[offset..])?;
                    }
                }
                _ => return Err("unsupported DEFAULT_PARAMETERS composition"),
            }
            module.types[*ty].inner.size(module.to_ctx()) as usize
        }
        _ => return Err("DEFAULT_PARAMETERS must be a WGSL const expression"),
    })
}

/// Names the program location of a traced-stage rejection: implicit-derivative sampling
/// and dpdx/dpdy, which compute stages lack. None when the program has neither.
pub(crate) fn traced_stage_error(label: &str, composed: &Composed) -> Option<Error> {
    let module = naga::front::wgsl::parse_str(&composed.source).ok()?;
    module.functions.iter().find_map(|(_, f)| {
        f.expressions.iter().find_map(|(h, e)| {
            let implicit = matches!(
                e,
                naga::Expression::ImageSample {
                    level: naga::SampleLevel::Auto | naga::SampleLevel::Bias(_),
                    ..
                } | naga::Expression::Derivative { .. }
            );
            let span = f.expressions.get_span(h);
            (implicit && in_program(composed, span)).then(|| {
                diagnostic(
                    label,
                    composed,
                    Some(span.location(&composed.source)),
                    "traced shading has no implicit derivatives: use the sample_* helpers or sample_*_grad with input.uv_dx/uv_dy, not textureSample or dpdx/dpdy",
                )
            })
        })
    })
}

/// The standard program returning `value`, a vec3 expression, in place of its lit sum:
/// how tests probe one term (visibility, base colour) through the full program path.
#[cfg(test)]
pub(crate) fn standard_returning(value: &str) -> String {
    assert_eq!(
        STANDARD_PROGRAM
            .matches(
                "    return exposed;
"
            )
            .count(),
        1
    );
    STANDARD_PROGRAM.replace(
        "    return exposed;
",
        &format!(
            "    return {value};
"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");
    const TOON: &str = "struct Parameters { shadow_tint: vec3<f32>, bands: f32, rim_color: vec3<f32>, rim_power: f32, fill: f32, flat_shaded: u32 };\nstruct Surface { albedo: vec3<f32>, n: vec3<f32> };\nfn surface(input: SurfaceInput) -> Surface {\n    var n=input.geometric;\n    if params.flat_shaded==0u { n=normalize(input.normal); }\n    return Surface(input.material.base_color.rgb*sample_base_color(input).rgb,n);\n}\nfn shade(s: Surface, input: SurfaceInput) -> vec3<f32> {\n    var lit=ambient(s.n)*params.fill;\n    for(var i=0u;i<light_count();i+=1u) {\n        let light=light_sample(i,input);\n        let incident=max(dot(s.n,light.direction),0.0)*light_visibility(input,light);\n        let band=floor(incident*params.bands)/params.bands;\n        lit+=light.intensity*light.attenuation*band;\n    }\n    return expose(s.albedo*lit/PI);\n}\n";

    /// `RENDERLAB_RL_FULL_STRATA` rewrites one constant, which the strata count reads on
    /// both the hardware and the software trace (the software API is the hardware one with
    /// its queries swapped).
    #[test]
    fn the_glossy_strata_follow_the_lobe_weight_on_both_traces() {
        let rule = "if RL_FULL_STRATA>0.0 && !mirror {count=max(1u,u32(ceil(f32(count)*min(lobe.weight/RL_FULL_STRATA,1.0))));}";
        for api in [REFLECTION_API.to_owned(), software_reflection_api()] {
            assert_eq!(api.matches(FULL_STRATA).count(), 1);
            assert_eq!(api.matches(rule).count(), 1);
        }
    }

    fn raster(program: &str) -> Composed {
        compose(
            PRELUDE,
            program,
            Stage::Raster,
            declares_parameters(program),
        )
    }
    fn rejection(label: &str, program: &str) -> String {
        match analyse(label, program, &raster(program)) {
            Err(Error::Shader(message)) => message,
            other => panic!("expected a shader rejection, got {other:?}"),
        }
    }

    #[test]
    fn default_traced_shading_has_no_residency_code() {
        let head = format!(
            "const PRIMARY_BATCHES:u32={}u;\n{PRELUDE}",
            crate::TRACED_DRAWS
        );
        let plain = compose(&head, STANDARD_PROGRAM, Stage::Traced, false).source;
        let streamed = compose(&head, STANDARD_PROGRAM, Stage::TracedStreamed, false).source;
        assert!(plain.contains(TRACED_API));
        for name in ["primary_residency", "primary_level_probe", "rl_record"] {
            assert!(!plain.contains(name) && streamed.contains(name), "{name}");
        }
        // Exactly the three `_grad` helpers are replaced, once each.
        for helper in ["base_color", "metallic_roughness", "normal_map"] {
            let definition = format!("fn sample_{helper}_grad(");
            assert_eq!(plain.matches(&definition).count(), 1);
            assert_eq!(streamed.matches(&definition).count(), 1);
        }
        assert!(
            streamed.contains("fn environment_dfg(") && streamed.contains("fn sample_base_color(")
        );
    }

    #[test]
    fn standard_program_composes_without_parameters() {
        let composed = raster(STANDARD_PROGRAM);
        assert!(!declares_parameters(STANDARD_PROGRAM));
        assert_eq!(&composed.source[composed.program.clone()], STANDARD_PROGRAM);
        assert!(
            analyse("standard", STANDARD_PROGRAM, &composed)
                .unwrap()
                .parameters
                .is_none()
        );
    }

    #[test]
    fn specular_filter_helpers_and_custom_geometric_inputs_compose() {
        analyse("specular AA", STANDARD_PROGRAM, &raster(STANDARD_PROGRAM)).unwrap();
        // A program can consume the geometric inputs without adopting GGX.
        let custom = "struct Surface { variance:f32 }; fn surface(input:SurfaceInput)->Surface { return Surface(geometric_normal_variance(input)); } fn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return vec3<f32>(s.variance); }";
        analyse("custom variance", custom, &raster(custom)).unwrap();
    }

    #[test]
    fn parameters_are_reflected_checked_and_packed() {
        let layout = analyse("toon", TOON, &raster(TOON))
            .unwrap()
            .parameters
            .unwrap();
        let offsets: Vec<_> = layout
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.offset))
            .collect();
        assert_eq!(
            offsets,
            [
                ("shadow_tint", 0),
                ("bands", 12),
                ("rim_color", 16),
                ("rim_power", 28),
                ("fill", 32),
                ("flat_shaded", 36)
            ]
        );
        let l = Some(&layout);
        assert!(ParameterLayout::check(l, [("bands", Value::F32(3.0))]).is_ok());
        assert!(ParameterLayout::check(l, [("bands", Value::U32(3))]).is_err());
        assert!(ParameterLayout::check(l, [("missing", Value::F32(1.0))]).is_err());
        assert!(ParameterLayout::check(None, [("bands", Value::F32(1.0))]).is_err());
        let block = ParameterLayout::pack(
            l,
            &[
                ("bands".into(), Value::F32(3.0)),
                ("flat_shaded".into(), Value::U32(1)),
            ],
        );
        assert_eq!(block[12..16], 3f32.to_ne_bytes());
        assert_eq!(block[36..40], 1u32.to_ne_bytes());
        assert!(
            block[..12]
                .iter()
                .chain(&block[16..36])
                .chain(&block[40..])
                .all(|&b| b == 0)
        );
        let retyped = TOON.replace("rim_power: f32", "rim_power: u32");
        let next = analyse("toon", &retyped, &raster(&retyped))
            .unwrap()
            .parameters;
        assert_eq!(ParameterLayout::changed(l, next.as_ref()), ["rim_power"]);
        assert!(ParameterLayout::changed(l, l).is_empty());
    }

    #[test]
    fn program_defaults_and_fixed_arrays_use_naga_layout() {
        let source = format!(
            "{}\n{STANDARD_PROGRAM}",
            r#"
struct Parameters { gain:f32, thresholds:array<vec3<f32>,2>, count:u32, signed:i32, tail:vec2<f32> };
const BASE:vec3<f32> = vec3<f32>(0.125,0.25,0.5);
const DEFAULT_PARAMETERS:Parameters = Parameters(
    2.0*0.25, array<vec3<f32>,2>(BASE,vec3<f32>(0.75)), 3u, -2, vec2<f32>(1.0,2.0));
"#
        );
        let layout = analyse("arrays", &source, &raster(&source))
            .unwrap()
            .parameters
            .unwrap();
        assert_eq!(
            layout.fields.iter().map(|f| f.offset).collect::<Vec<_>>(),
            [0, 16, 48, 52, 56]
        );
        let block = ParameterLayout::pack(Some(&layout), &[]);
        let word = |offset| f32::from_ne_bytes(block[offset..offset + 4].try_into().unwrap());
        assert_eq!(
            [word(0), word(16), word(20), word(24), word(32)],
            [0.5, 0.125, 0.25, 0.5, 0.75]
        );
        assert_eq!(&block[48..52], &3u32.to_ne_bytes());
        assert_eq!(&block[52..56], &(-2i32).to_ne_bytes());
        assert_eq!([word(56), word(60)], [1.0, 2.0]);
        assert!(block[28..32].iter().chain(&block[44..48]).all(|&v| v == 0));
        let array = Value::Array(vec![Value::Vec3([1., 2., 3.]), Value::Vec3([4., 5., 6.])]);
        ParameterLayout::check(Some(&layout), [("thresholds", array.clone())]).unwrap();
        for invalid in [
            Value::Array(vec![]),
            Value::Array(vec![Value::Vec3([1.; 3])]),
            Value::Array(vec![Value::Vec4([1.; 4]); 2]),
            Value::Vec3([1.; 3]),
        ] {
            assert!(ParameterLayout::check(Some(&layout), [("thresholds", invalid)]).is_err());
        }
        let overridden = ParameterLayout::pack(
            Some(&layout),
            &[
                ("thresholds".into(), array),
                ("gain".into(), Value::F32(0.)),
            ],
        );
        assert_eq!(&overridden[..4], &[0; 4]);
        assert_eq!(&overridden[32..36], &4f32.to_ne_bytes());
        assert_eq!(&overridden[48..64], &block[48..64]);
        let next_source = source.replace("2.0*0.25", "0.75");
        let next = analyse("arrays", &next_source, &raster(&next_source))
            .unwrap()
            .parameters
            .unwrap();
        assert_ne!(layout, next, "default edits trigger material repacking");
        assert!(ParameterLayout::changed(Some(&layout), Some(&next)).is_empty());
        assert!(
            rejection(
                "arrays",
                &format!(
                    "struct Parameters {{ oversized:array<vec4<f32>,17> }};\n{STANDARD_PROGRAM}"
                )
            )
            .contains("Parameters")
        );
        let scalar_array = format!("struct Parameters {{ a:array<f32,2> }};\n{STANDARD_PROGRAM}");
        let error = rejection("arrays", &scalar_array);
        assert!(error.contains("program `arrays` line 1:"), "{error}");
        let wrong = source.replace("const DEFAULT_PARAMETERS:Parameters = Parameters(\n    2.0*0.25, array<vec3<f32>,2>(BASE,vec3<f32>(0.75)), 3u, -2, vec2<f32>(1.0,2.0));", "const DEFAULT_PARAMETERS:f32 = 1.0;");
        assert!(rejection("arrays", &wrong).contains("DEFAULT_PARAMETERS"));
    }

    #[test]
    fn toon_defaults_equal_the_original_application_values() {
        let source = include_str!("../shaders/programs/toon.wgsl");
        let layout = analyse("toon", source, &raster(source))
            .unwrap()
            .parameters
            .unwrap();
        let explicit = [
            ("shadow_tint".into(), Value::Vec3([0.12, 0.08, 0.24])),
            ("bands".into(), Value::F32(4.)),
            ("rim_color".into(), Value::Vec3([0.12, 0.2, 0.3])),
            ("rim_power".into(), Value::F32(4.)),
            ("thresholds".into(), Value::Vec2([0.12, 0.9])),
            ("fill".into(), Value::F32(0.25)),
            ("flat_shaded".into(), Value::U32(0)),
        ];
        assert_eq!(
            ParameterLayout::pack(Some(&layout), &[]),
            ParameterLayout::pack(Some(&layout), &explicit)
        );
    }

    #[test]
    fn rejections_name_program_lines() {
        let line = |text: &str| {
            STANDARD_PROGRAM
                .lines()
                .position(|l| l.contains(text))
                .unwrap()
                + 1
        };
        let syntax = STANDARD_PROGRAM.replace(
            "let mr=sample_metallic_roughness(input);",
            "let mr=sample_metallic_roughness(input;",
        );
        let message = rejection("standard", &syntax);
        assert!(
            message.starts_with(&format!("program `standard` line {}:", line("let mr="))),
            "{message}"
        );
        let binding =
            format!("@group(2) @binding(9) var<uniform> mine:vec4<f32>;\n{STANDARD_PROGRAM}");
        let message = rejection("standard", &binding);
        assert!(
            message.starts_with("program `standard` line 1:") && message.contains("bindings"),
            "{message}"
        );
        let redefinition = format!("{STANDARD_PROGRAM}fn light_count() -> u32 {{ return 0u; }}\n");
        let message = rejection("standard", &redefinition);
        assert!(message.contains("light_count"), "{message}");
        let missing = STANDARD_PROGRAM.replace("fn shade(", "fn shaded(");
        assert!(rejection("standard", &missing).contains("must define fn shade"));
        // A commented-out definition does not count.
        let commented = STANDARD_PROGRAM.replace("fn surface(", "// fn surface(\nfn surfaced(");
        assert!(rejection("standard", &commented).contains("must define fn surface"));
        let oversized =
            format!("struct Parameters {{ a: array<vec4<f32>, 17> }};\n{STANDARD_PROGRAM}");
        assert!(rejection("standard", &oversized).contains("Parameters"));
    }
}
